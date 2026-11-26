// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! A running KivaDB instance: configuration, the system database, every open
//! workspace, and the services shared by all sessions.

use crate::auth;
use crate::catalog::{plan_legacy_catalog, Catalog, CatalogStore, LegacyCollection};
use crate::compress::Compression;
use crate::config::Config;
use crate::engine::{Engine, EngineOptions, Op};
use crate::error::{KivaError, Result};
use crate::repl::ReplState;
use crate::util::{fmt_time, glob_match, now_secs};
use crate::value::Value;
use std::cell::Cell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::time::Instant;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

thread_local! {
    /// Set on the trigger worker so that writes made by a trigger script do
    /// not fire triggers again.
    static IN_TRIGGER: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub skip_auth: bool,
    pub lua_debug: bool,
}

// ---------------------------------------------------------------------------

/// Append-only journal of sensitive actions.
pub struct Audit {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl Audit {
    fn new(path: PathBuf) -> Audit {
        let file = OpenOptions::new().create(true).append(true).open(&path).ok();
        Audit { path, file: Mutex::new(file) }
    }

    pub fn log(&self, user: &str, action: &str, success: bool) {
        let line = format!(
            "[{}] user={} action=\"{}\" status={}\n",
            fmt_time(now_secs()),
            user,
            action.replace('"', "'"),
            if success { "SUCCESS" } else { "FAILURE" }
        );
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    pub fn lines(&self) -> Vec<String> {
        fs::read_to_string(&self.path).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ---------------------------------------------------------------------------

struct Sub {
    session: u64,
    workspace: String,
    pattern: String,
    tx: Sender<String>,
}

/// Key-change notifications (`subscribe`, WebSocket).
#[derive(Default)]
pub struct PubSub {
    subs: Mutex<Vec<Sub>>,
    count: AtomicUsize,
}

impl PubSub {
    pub fn subscribe(&self, session: u64, workspace: &str, pattern: &str, tx: Sender<String>) {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| !(s.session == session && s.workspace == workspace && s.pattern == pattern));
        subs.push(Sub { session, workspace: workspace.to_string(), pattern: pattern.to_string(), tx });
        self.count.store(subs.len(), Ordering::SeqCst);
    }

    /// Removes one pattern, or every subscription of the session.
    pub fn unsubscribe(&self, session: u64, pattern: Option<&str>) -> usize {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        let before = subs.len();
        subs.retain(|s| !(s.session == session && pattern.is_none_or(|p| p == s.pattern)));
        self.count.store(subs.len(), Ordering::SeqCst);
        before - subs.len()
    }

    fn publish(&self, workspace: &str, op: &Op) {
        if self.count.load(Ordering::SeqCst) == 0 {
            return;
        }
        let (key, msg) = match op {
            Op::Set { key, ty, created, .. } => {
                (key.as_str(), format!("{} {} ({})", if *created { "set" } else { "update" }, key, ty.name()))
            }
            Op::Del { key } => (key.as_str(), format!("del {key}")),
            Op::Reset => ("", "reset (all keys deleted)".to_string()),
        };
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        let before = subs.len();
        subs.retain(|s| {
            if s.workspace != workspace || !(matches!(op, Op::Reset) || glob_match(&s.pattern, key)) {
                return true;
            }
            s.tx.send(msg.clone()).is_ok()
        });
        if subs.len() != before {
            self.count.store(subs.len(), Ordering::SeqCst);
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Metrics {
    pub gets: AtomicU64,
    pub sets: AtomicU64,
    pub dels: AtomicU64,
    pub others: AtomicU64,
    pub errors: AtomicU64,
    pub micros: AtomicU64,
    /// Commands by duration: `latency[i]` counts those up to `LATENCY_BUCKETS_US[i]`
    /// (and above the previous bound), the last one those beyond every bound.
    pub latency: [AtomicU64; LATENCY_BUCKETS_US.len() + 1],
    pub http_requests: AtomicU64,
    pub failed_logins: AtomicU64,
    /// Commands by kind (`COMMAND_KINDS`), with their durations.
    pub commands: [CommandStat; COMMAND_KINDS.len()],
    pub backup_failures: AtomicU64,
}

/// The kinds commands are counted by in the metrics: a fixed list, never a
/// key or a user, so that the number of series stays small.
pub const COMMAND_KINDS: [&str; 15] = [
    "get",
    "set",
    "update",
    "del",
    "query",
    "aggregate",
    "similar",
    "json",
    "transaction",
    "script",
    "changes",
    "collection",
    "admin",
    "auth",
    "other",
];

/// The count and durations of one kind of command.
#[derive(Default)]
pub struct CommandStat {
    pub count: AtomicU64,
    pub micros: AtomicU64,
    pub latency: [AtomicU64; LATENCY_BUCKETS_US.len() + 1],
}

/// The kind of a command for the metrics: its first word, and for `get` the
/// second (`get where` is a query, `get similar` a vector search).
pub fn command_kind(cmd: &str, second: Option<&str>) -> usize {
    let kind = match cmd {
        "get" => match second.map(str::to_ascii_lowercase).as_deref() {
            Some("where" | "range") => "query",
            Some("similar") => "similar",
            _ => "get",
        },
        "set" | "add" | "change" | "bump" | "convert" => "set",
        "update" => "update",
        "del" => "del",
        "scan" | "keys" | "range" | "explain" | "has" | "typeof" => "query",
        "count" | "sum" | "avg" | "min" | "max" => "aggregate",
        "semantic" => "similar",
        "json" => "json",
        "begin" | "commit" | "rollback" | "savepoint" | "transaction" => "transaction",
        "eval" | "script" | "wait" => "script",
        "changes" => "changes",
        "use" | "collection" | "collections" | "workspace" | "project" => "collection",
        "auth" | "login" | "whoami" => "auth",
        "create" | "drop" | "tune" | "grant" | "revoke" | "users" | "modify" | "config" | "backup" | "export"
        | "import" | "compact" | "autocompact" | "check" | "schema" | "schemas" | "encryption" | "replica"
        | "failover" | "audit" | "memory" | "hint" | "compress" | "shutdown" => "admin",
        _ => "other",
    };
    COMMAND_KINDS.iter().position(|k| *k == kind).unwrap_or(COMMAND_KINDS.len() - 1)
}

/// Upper bounds of the latency histogram, in microseconds: from a value read
/// from memory (well under a millisecond) to a long scan or script.
pub const LATENCY_BUCKETS_US: [u64; 10] =
    [100, 500, 1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000, 5_000_000];

impl Metrics {
    pub fn record_latency(&self, micros: u64) {
        self.micros.fetch_add(micros, Ordering::Relaxed);
        let i = LATENCY_BUCKETS_US.iter().position(|b| micros <= *b).unwrap_or(LATENCY_BUCKETS_US.len());
        self.latency[i].fetch_add(1, Ordering::Relaxed);
    }

    /// A command of `kind` (`command_kind`) that took `micros`.
    pub fn record_command(&self, kind: usize, micros: u64) {
        let c = &self.commands[kind];
        c.count.fetch_add(1, Ordering::Relaxed);
        c.micros.fetch_add(micros, Ordering::Relaxed);
        let i = LATENCY_BUCKETS_US.iter().position(|b| micros <= *b).unwrap_or(LATENCY_BUCKETS_US.len());
        c.latency[i].fetch_add(1, Ordering::Relaxed);
    }
}

pub struct Semaphore {
    permits: Mutex<usize>,
    cv: Condvar,
}

pub struct Permit<'a>(&'a Semaphore);

impl Semaphore {
    pub fn new(n: usize) -> Semaphore {
        Semaphore { permits: Mutex::new(n.max(1)), cv: Condvar::new() }
    }

    pub fn acquire(&self) -> Permit<'_> {
        let mut p = self.permits.lock().unwrap_or_else(|e| e.into_inner());
        while *p == 0 {
            p = self.cv.wait(p).unwrap_or_else(|e| e.into_inner());
        }
        *p -= 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.permits.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.0.cv.notify_one();
    }
}

// ---------------------------------------------------------------------------

/// `create trigger <name> on <event> "<pattern>" run "<script>"`
#[derive(Clone, Debug)]
pub struct Trigger {
    pub name: String,
    /// `set` (key created), `update` (key overwritten) or `del`.
    pub event: String,
    pub pattern: String,
    pub script: String,
}

struct TriggerJob {
    workspace: String,
    event: &'static str,
    key: String,
    scripts: Vec<String>,
}

pub struct Instance {
    pub home: PathBuf,
    staged_migration: bool,
    pub(crate) hierarchical_tcp: bool,
    pub(crate) managed_layout_lock: RwLock<()>,
    pub(crate) managed_engines: Mutex<crate::managed_layout::EngineRegistry>,
    pub(crate) managed_catalog: Mutex<Option<Arc<Catalog>>>,
    pub(crate) managed_policy: Mutex<crate::hierarchy_auth::CachedPolicy>,

    pub config: RwLock<Config>,
    /// Hidden database holding users, stored scripts and triggers.
    pub system: Arc<Engine>,
    /// An empty store, the engine of a session that is in no workspace:
    /// every command on data is refused before it, so it stays empty.
    pub nowhere: Arc<Engine>,
    /// Encryption at rest, for every workspace and the system database.
    pub keys: Option<Arc<crate::crypt::Keys>>,
    engines: Mutex<HashMap<String, Arc<Engine>>>,
    pub pubsub: PubSub,
    pub metrics: Metrics,
    pub audit: Audit,
    pub repl: ReplState,
    pub opts: Options,
    triggers: RwLock<Vec<Trigger>>,
    trigger_tx: Mutex<Option<Sender<TriggerJob>>>,
    pub next_id: AtomicU64,
    pub clients: AtomicUsize,
    pub started: Instant,
    pub exec_slots: Semaphore,
    pub(crate) native_metrics: Option<crate::native_timing::NativeMetrics>,
    pub shutting_down: AtomicBool,
    /// Set by the `shutdown` command; the process hosting the instance exits.
    pub stop_requested: AtomicBool,
    /// Port of the TCP listener, once one is running.
    pub tcp_port: AtomicU64,
    /// State shared by the Redis-compatible connections.
    pub resp: crate::resp::Shared,
    /// Answers cached by meaning (`semantic` commands, `get similar`).
    pub semantic: crate::semantic_cache::SemanticCache,
    /// The TLS certificate, loaded on first use (None when TLS is off or broken).
    tls_state: std::sync::OnceLock<Option<Arc<crate::tls::ServerTls>>>,
    weak: Weak<Instance>,
    _lock: File,
}

impl Instance {
    /// Opens (creating on first run) the instance rooted at `home`.
    /// Fails with `Locked` if another process already owns it.
    pub fn open(home: &Path, opts: Options) -> Result<Arc<Instance>> {
        Self::open_mode(home, opts, false)
    }

    /// Opens an initialized managed home for the authenticated native listener.
    /// Provision accounts and initialize the managed catalog before using this mode.
    pub fn open_hierarchical(home: &Path, opts: Options) -> Result<Arc<Instance>> {
        Self::open_mode(home, opts, true)
    }

    fn open_mode(home: &Path, opts: Options, hierarchical_tcp: bool) -> Result<Arc<Instance>> {
        if home.join(crate::hierarchy_layout::EXPORT_MARKER).try_exists()? {
            return Err(KivaError::Invalid("a hierarchy layout export is not a runnable database home".into()));
        }
        fs::create_dir_all(home)?;
        let lock = OpenOptions::new().create(true).write(true).truncate(false).open(home.join("instance.lock"))?;
        if !crate::util::lock_exclusive(&lock) {
            return Err(KivaError::Locked(home.display().to_string()));
        }
        let _ = fs::remove_file(home.join("instance.port"));
        let _ = fs::remove_file(home.join("instance.pid"));
        let config = Config::load(home)?;
        if hierarchical_tcp {
            if opts.skip_auth || !config.get_bool("security", "auth_required") {
                return Err(KivaError::Denied("hierarchical TCP requires account authentication".into()));
            }
            if matches!(config.get("replication", "role").as_str(), "replica" | "slave")
                || config.get_bool("failover", "auto")
            {
                return Err(KivaError::Invalid(
                    "hierarchical TCP requires replication and failover to be disabled".into(),
                ));
            }
            if !crate::managed_layout::exists(home)? {
                return Err(KivaError::Invalid(
                    "initialize the managed catalog from an offline local shell first".into(),
                ));
            }
        }
        let keys = crate::crypt::Keys::load(&config)?;
        let require_encryption = config.get_bool("encryption", "required");
        let system = Engine::open(
            home.join("system").join("system.kiva"),
            EngineOptions {
                auto_compact_min_bytes: 16 * 1024,
                keys: keys.clone(),
                require_encryption,
                ..Default::default()
            },
        )?;
        // Validate staged metadata before account migration and administrative recovery.
        // An absent record keeps a historical home on the existing storage layout.
        let embedded = CatalogStore::new(&system).load()?;
        crate::hierarchy_auth::Policy::load(&system)?;
        let staged_migration = crate::migration::staged_catalog(home)?.is_some();
        if staged_migration && embedded.is_some() {
            return Err(KivaError::Corrupted("multiple staged workspace catalogs".into()));
        }
        auth::ensure_admin(&system)?;
        auth::migrate_personal_workspaces(&system, |w| config.workspace_dir(w).is_some())?;
        for done in recover_moves(&system, &config)? {
            eprintln!("[WARN] {done}.");
        }
        let nowhere = Engine::open(
            home.join("system").join("nowhere.kiva"),
            EngineOptions { keys: keys.clone(), require_encryption, ..Default::default() },
        )?;
        let threads = config.get_u64("server", "threads") as usize;

        let inst = Arc::new_cyclic(|weak| Instance {
            home: home.to_path_buf(),
            staged_migration,
            hierarchical_tcp,
            managed_layout_lock: RwLock::new(()),
            managed_engines: Mutex::new(crate::managed_layout::EngineRegistry::new(
                config.get_u64("storage", "managed_open_engines").clamp(1, 1024) as usize,
            )),
            managed_catalog: Mutex::default(),
            managed_policy: Mutex::default(),

            native_metrics: (hierarchical_tcp && config.get_bool("server", "native_metrics")).then(Default::default),
            config: RwLock::new(config),
            system,
            nowhere,
            keys,
            engines: Mutex::new(HashMap::new()),
            pubsub: PubSub::default(),
            metrics: Metrics::default(),
            audit: Audit::new(home.join("audit.log")),
            repl: ReplState::new(),
            opts,
            triggers: RwLock::new(Vec::new()),
            trigger_tx: Mutex::new(None),
            next_id: AtomicU64::new(1),
            clients: AtomicUsize::new(0),
            started: Instant::now(),
            exec_slots: Semaphore::new(threads),
            shutting_down: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            tcp_port: AtomicU64::new(0),
            resp: Default::default(),
            semantic: Default::default(),
            tls_state: std::sync::OnceLock::new(),
            weak: weak.clone(),
            _lock: lock,
        });
        // Users, stored scripts and triggers are replicated like a workspace.
        let weak = Arc::downgrade(&inst);
        inst.system.add_listener(move |op| {
            if let Some(inst) = weak.upgrade() {
                inst.repl.broadcast(crate::repl::SYSTEM, op);
            }
        });
        if crate::managed_layout::exists(home)? {
            inst.check_managed_layout_home()?;
            let catalog = crate::managed_layout::open(home, &inst.engine_options())?.map(Arc::new);
            *inst.managed_catalog.lock().unwrap_or_else(|e| e.into_inner()) = catalog;
        }
        inst.recover_pending_projects()?;
        inst.reload_triggers();
        inst.start_trigger_worker();
        // Open the default workspace eagerly so problems surface at startup.
        inst.engine("default")?;
        Ok(inst)
    }

    pub fn arc(&self) -> Arc<Instance> {
        self.weak.upgrade().expect("instance is alive")
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn cfg(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn cfg_mut(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.config.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The server's time zone (`time.zone`); the machine's when it is not valid.
    pub fn zone(&self) -> crate::time::Zone {
        crate::time::Zone::parse(&self.cfg().get("time", "zone")).unwrap_or(crate::time::Zone::Local)
    }

    /// The server's format of a kind of date (`time.date_format`...); ISO
    /// (`None`) when it is not set or not valid.
    pub fn format(&self, kind: crate::time::FormatKind) -> Option<crate::time::Pattern> {
        crate::time::parse_format(kind, &self.cfg().get("time", kind.config_key())).ok().flatten()
    }

    /// Returns process-local aggregate timings when native metrics are enabled.
    pub fn native_timings(&self) -> Vec<crate::server::NativeTiming> {
        self.native_metrics.as_ref().map_or_else(Vec::new, |metrics| metrics.snapshot())
    }

    pub fn auth_required(&self) -> bool {
        !self.opts.skip_auth && self.cfg().get_bool("security", "auth_required")
    }

    pub fn engine_options(&self) -> EngineOptions {
        let c = self.cfg();
        EngineOptions {
            compression: Compression::from_name(&c.get("storage", "compression")).unwrap_or(Compression::None),
            auto_compact: c.get_bool("storage", "auto_compact"),
            compact_threshold: c.get_u64("storage", "compact_threshold").clamp(1, 99) as u8,
            snapshot_interval_secs: c.get_u64("storage", "snapshot_interval").max(1),
            snapshot_every_ops: c.get_u64("storage", "snapshot_every").max(1),
            sync_writes: c.get_bool("storage", "sync_writes"),
            cache_bytes: if c.get_bool("storage", "cache_values") {
                (c.get_u64("storage", "cache_size_mb") as usize).saturating_mul(1024 * 1024)
            } else {
                0
            },
            max_memory_bytes: (c.get_u64("storage", "max_memory_mb") as usize).saturating_mul(1024 * 1024),
            max_vector_bytes: match c.get("vector", "max_ram_mb").as_str() {
                // Half the RAM leaves room for the rest of the server and the
                // system; unknown RAM means no bound rather than a wrong one.
                "auto" => crate::util::physical_memory().map_or(0, |b| (b / 2) as usize),
                _ => (c.get_u64("vector", "max_ram_mb") as usize).saturating_mul(1024 * 1024),
            },
            // Keys are read once, at start: changing them needs a restart.
            keys: self.keys.clone(),
            require_encryption: self.keys.is_some() && c.get_bool("encryption", "required"),
            ..Default::default()
        }
    }

    /// The engine of a workspace, opened on first use.
    pub fn engine(&self, workspace: &str) -> Result<Arc<Engine>> {
        let mut map = self.engines.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = map.get(workspace) {
            return Ok(e.clone());
        }
        let dir = self
            .cfg()
            .workspace_dir(workspace)
            .ok_or_else(|| KivaError::NotFound(format!("workspace '{workspace}' does not exist")))?;
        let engine = Engine::open(dir.join("store.kiva"), self.engine_options())?;
        let weak = self.weak.clone();
        let name = workspace.to_string();
        engine.add_listener(move |op| {
            if let Some(inst) = weak.upgrade() {
                inst.on_op(&name, op);
            }
        });
        // A document gone with its TTL is a change too, for cached answers.
        let (weak, name) = (self.weak.clone(), workspace.to_string());
        engine.add_expiry_listener(move |key| {
            if let Some(inst) = weak.upgrade() {
                inst.semantic.invalidate(&name, key);
            }
        });
        map.insert(workspace.to_string(), engine.clone());
        Ok(engine)
    }

    pub fn open_engines(&self) -> Vec<(String, Arc<Engine>)> {
        let map = self.engines.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<_> = map.iter().map(|(k, e)| (k.clone(), e.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// `(name, directory)` of every known workspace, `default` first.
    pub fn workspaces(&self) -> Vec<(String, String)> {
        let c = self.cfg();
        let mut out = vec![("default".to_string(), c.get("storage", "default_workspace"))];
        out.extend(c.workspaces());
        out
    }

    /// Names of the workspaces, `default` first.
    pub fn workspace_names(&self) -> Vec<String> {
        self.workspaces().into_iter().map(|(n, _)| n).collect()
    }

    /// Registers a workspace. A name such as `travel.places` puts it in the
    /// project `travel`; the commands check that the project exists, not this
    /// (a replica registers the master's workspaces before their projects).
    ///
    /// Without a path, the folder is the one of the layout of rc.5
    /// (`collection_dir`): it must not hold files already, and the name must
    /// not differ from another only by capitals, which Windows and macOS do
    /// not tell apart. A path given by an administrator is used as it is.
    pub fn create_workspace(&self, name: &str, dir: Option<&str>) -> Result<PathBuf> {
        self.create_workspace_in(name, dir, false)
    }

    /// `create_workspace`; `reuse` lets a folder of the layout that already
    /// holds files be taken back (a replica, a Redis database number).
    fn create_workspace_in(&self, name: &str, dir: Option<&str>, reuse: bool) -> Result<PathBuf> {
        self.guard_legacy_catalog_mutation()?;
        check_workspace_name(name)?;
        if self.cfg().workspace_dir(name).is_some() {
            return Err(KivaError::Exists(format!("workspace '{name}' already exists")));
        }
        let dir = match dir {
            Some(d) => PathBuf::from(d),
            None => {
                let twin = self.workspace_names().into_iter().find(|w| w.eq_ignore_ascii_case(name));
                if let Some(twin) = twin {
                    return Err(KivaError::Exists(format!(
                        "'{name}' differs from the collection '{twin}' only by capitals, which Windows and macOS \
                         do not tell apart: choose another name"
                    )));
                }
                let dir = collection_dir(&self.home, name);
                if !reuse && fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some()) {
                    return Err(KivaError::Exists(format!(
                        "the folder {} already holds files: KivaDB does not take them over without being told. \
                         To use them, give the folder: collection create {name} \"{}\"",
                        dir.display(),
                        dir.display()
                    )));
                }
                dir
            }
        };
        fs::create_dir_all(&dir)?;
        self.cfg_mut().set("workspaces", name, &dir.display().to_string())?;
        self.repl.workspace_changed(name, false);
        Ok(dir)
    }

    /// Unregisters a workspace; its files stay on disk. Returns whether it existed.
    pub fn remove_workspace(&self, name: &str) -> Result<bool> {
        self.guard_legacy_catalog_mutation()?;
        if name == "default" {
            return Err(KivaError::Invalid("the default workspace cannot be removed".into()));
        }
        let removed = self.cfg_mut().remove("workspaces", name)?;
        if removed {
            self.repl.workspace_changed(name, true);
        }
        Ok(removed)
    }

    /// The server's TLS certificate, loading or creating it on first use.
    /// `None` when `tls.enabled` is false or the certificate cannot be used
    /// (the reason is printed once).
    pub fn tls(&self) -> Option<Arc<crate::tls::ServerTls>> {
        self.tls_state
            .get_or_init(|| {
                if !self.cfg().get_bool("tls", "enabled") {
                    return None;
                }
                let loaded = crate::tls::load_server(&self.home, &self.cfg());
                match loaded {
                    Ok(t) => Some(Arc::new(t)),
                    Err(e) => {
                        eprintln!("[WARN] encrypted connections are not available: {e}");
                        None
                    }
                }
            })
            .clone()
    }

    /// Gives a workspace another name, such as `places_travel` to `travel.places`
    /// (into a project). Its files stay where they are, and the grants that
    /// named it follow it. Refused while a session or a script is in it.
    pub fn rename_workspace(&self, from: &str, to: &str) -> Result<()> {
        self.guard_legacy_catalog_mutation()?;
        if from == "default" || to == "default" {
            return Err(KivaError::Invalid("the default workspace keeps its name".into()));
        }
        check_workspace_name(to)?;
        if self.cfg().workspace_dir(from).is_none() {
            return Err(KivaError::NotFound(format!("workspace '{from}' does not exist")));
        }
        if self.cfg().workspace_dir(to).is_some() {
            return Err(KivaError::Exists(format!("workspace '{to}' already exists")));
        }
        // In three steps, so that a stop between two of them never takes a
        // workspace away from its users, nor leaves them a grant on a name
        // that is not the workspace's. The move is recorded first, and the
        // grants on the old name are given on the new one too; both reach
        // the disk before kiva.conf changes. A start after a stop finishes
        // the move or takes it back (recover_moves).
        let mut doubled = Vec::new();
        for mut user in auth::list_users(&self.system)? {
            if let Some(p) = user.grants.get(from).copied() {
                user.grants.insert(to.to_string(), p);
                doubled.push(user);
            }
        }
        let names: Vec<String> = doubled.iter().map(|u| u.name.clone()).collect();
        let record = serde_json::json!({ "from": from, "to": to, "doubled": names });
        self.system.put(&move_record(from), &Value::Doc(record), 0, false)?;
        for user in &doubled {
            auth::save_user(&self.system, user)?;
        }
        self.system.sync()?;
        let renamed = {
            let mut map = self.engines.lock().unwrap_or_else(|e| e.into_inner());
            match map.get(from) {
                // The map holds one reference; any other is a session in it.
                Some(engine) if Arc::strong_count(engine) > 1 => Err(KivaError::Invalid(format!(
                    "workspace '{from}' is in use: try again when the sessions in it have left"
                ))),
                // Then the workspace takes its new name, in one write of
                // kiva.conf. Closed now: its listeners report changes under
                // the old name.
                _ => {
                    map.remove(from);
                    self.cfg_mut().rename("workspaces", from, to)
                }
            }
        };
        if let Err(e) = renamed {
            // Nothing moved: the grants given on the new name go away.
            undo_move(&self.system, from, to, &names)?;
            return Err(e);
        }
        // Last, the old name leaves the accounts.
        self.semantic.reset(from);
        finish_move(&self.system, from, to)?;
        // Replicas copy the workspaces again under their new names.
        self.repl.drop_replicas();
        Ok(())
    }

    /// The projects, in alphabetical order. They are stored in the system
    /// database, so replicas know them too.
    pub fn projects(&self) -> Vec<String> {
        self.system.keys_matching("project:*", 0).into_iter().map(|(k, _)| k["project:".len()..].to_string()).collect()
    }

    /// Read validated hierarchy metadata for internal migration tooling.
    /// Serving continues to resolve storage and authorization through the legacy configuration.
    pub fn workspace_catalog(&self) -> Result<Option<Catalog>> {
        match crate::migration::staged_catalog(&self.home)? {
            Some(catalog) => Ok(Some(catalog)),
            None => CatalogStore::new(&self.system).load(),
        }
    }

    /// Prepare an advisory migration plan from the current registrations.
    /// Activation must revalidate the source with catalog mutations quiesced.
    pub fn plan_workspace_migration(&self) -> Result<(Catalog, Vec<(LegacyCollection, PathBuf)>)> {
        let stores: HashMap<String, String> = self.workspaces().into_iter().collect();
        let (catalog, aliases) = plan_legacy_catalog(self.projects(), stores.keys().cloned().collect())?;
        let mut locations = HashMap::new();
        #[cfg(unix)]
        let mut files = HashMap::new();
        let mut plan = Vec::with_capacity(aliases.len());
        for alias in aliases {
            let directory = PathBuf::from(&stores[&alias.name]);
            let path = directory.join("store.kiva");
            let canonical = fs::canonicalize(&path)
                .map_err(|e| KivaError::Invalid(format!("legacy store '{}': {}: {e}", alias.name, path.display())))?;
            let meta = fs::metadata(&canonical)?;
            if !meta.is_file() {
                return Err(KivaError::Invalid(format!("legacy store '{}' is not a regular file", alias.name)));
            }
            if let Some(other) = locations.insert(canonical, alias.name.clone()) {
                return Err(KivaError::Invalid(format!("legacy stores '{other}' and '{}' share storage", alias.name)));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if let Some(other) = files.insert((meta.dev(), meta.ino()), alias.name.clone()) {
                    return Err(KivaError::Invalid(format!(
                        "legacy stores '{other}' and '{}' share storage",
                        alias.name
                    )));
                }
            }
            plan.push((alias, directory));
        }
        Ok((catalog, plan))
    }

    pub(crate) fn guard_legacy_catalog_mutation(&self) -> Result<()> {
        if crate::managed_layout::exists(&self.home)? {
            return Err(KivaError::ReadOnly("legacy topology is frozen while the managed layout exists".into()));
        }
        if self.staged_migration {
            return Err(KivaError::ReadOnly("legacy catalog is frozen while an offline migration is staged".into()));
        }
        Ok(())
    }

    pub(crate) fn check_managed_layout_home(&self) -> Result<()> {
        crate::managed_layout::validate_roots(&self.home)?;
        if self.workspace_catalog()?.is_some() {
            return Err(KivaError::ReadOnly("managed layout cannot be used with a staged migration".into()));
        }
        let config = self.cfg();
        let default = config.workspace_dir("default").expect("default path");
        let default_path = default.canonicalize().unwrap_or(default);
        let expected = self.home.join("data");
        let expected = expected.canonicalize().unwrap_or(expected);
        if self.staged_migration || !config.workspaces().is_empty() || default_path != expected {
            return Err(KivaError::ReadOnly(
                "managed layout requires an unstaged home with only the standard legacy data store".into(),
            ));
        }
        Ok(())
    }

    pub fn project_exists(&self, name: &str) -> bool {
        self.system.get(&format!("project:{name}")).ok().flatten().is_some()
    }

    pub fn create_project(&self, name: &str) -> Result<()> {
        self.guard_legacy_catalog_mutation()?;
        if !auth::valid_name_part(name) {
            return Err(KivaError::Invalid("project names may only contain letters, digits, '_' and '-'".into()));
        }
        if self.project_exists(name) {
            return Err(KivaError::Exists(format!("project '{name}' already exists")));
        }
        let doc = serde_json::json!({ "created": now_secs() });
        self.system.put(&format!("project:{name}"), &Value::Doc(doc), 0, false)?;
        Ok(())
    }

    /// `project create <name> collections (<c>, ...)`: the project and its
    /// collections, all new. Every name is checked first; then a record
    /// `pending:project:<name>` is written, so that a start after a crash
    /// finishes the creation (`recover_pending_projects`): nothing is lost by
    /// finishing, since everything is new and empty.
    pub fn create_project_with(&self, name: &str, collections: &[String]) -> Result<Vec<PathBuf>> {
        self.guard_legacy_catalog_mutation()?;
        if !auth::valid_name_part(name) {
            return Err(KivaError::Invalid("project names may only contain letters, digits, '_' and '-'".into()));
        }
        if self.project_exists(name) {
            return Err(KivaError::Exists(format!("project '{name}' already exists")));
        }
        let mut seen: Vec<String> = Vec::new();
        for c in collections {
            if !auth::valid_name_part(c) {
                return Err(KivaError::Invalid(format!("collection name '{c}': only letters, digits, '_' and '-'")));
            }
            // Windows and macOS do not tell `Places` from `places`.
            if seen.iter().any(|s| s.eq_ignore_ascii_case(c)) {
                return Err(KivaError::Invalid(format!("collection '{c}' is named twice (capitals do not count)")));
            }
            seen.push(c.clone());
            if self.cfg().workspace_dir(&format!("{name}.{c}")).is_some() {
                return Err(KivaError::Exists(format!("collection '{name}.{c}' already exists")));
            }
        }
        let record = pending_project(name);
        self.system.put(&record, &Value::Doc(serde_json::json!({ "collections": collections })), 0, false)?;
        self.system.sync()?;
        let dirs = self.finish_project(name, collections)?;
        self.system.delete(&record)?;
        self.system.sync()?;
        Ok(dirs)
    }

    /// Creates what is missing of a project and its collections.
    fn finish_project(&self, name: &str, collections: &[String]) -> Result<Vec<PathBuf>> {
        if !self.project_exists(name) {
            self.create_project(name)?;
        }
        let mut dirs = Vec::new();
        for c in collections {
            let full = format!("{name}.{c}");
            // The read lock of kiva.conf is let go before `create_workspace` writes it.
            let known = self.cfg().workspace_dir(&full);
            let dir = match known {
                Some(d) => d,
                None => self.create_workspace(&full, None)?,
            };
            dirs.push(dir);
        }
        Ok(dirs)
    }

    /// `collection create a b c`: several collections, given by their full
    /// names, all new. Every name is checked first (its form, its project,
    /// twice in the list, already there); nothing is created if one is
    /// wrong. Then a record `pending:collections:<id>` is written, so that a
    /// start after a stop in the middle creates the rest
    /// (`recover_pending_projects`): finishing loses nothing, since each one
    /// is new and empty.
    pub fn create_collections(&self, names: &[String]) -> Result<Vec<PathBuf>> {
        self.guard_legacy_catalog_mutation()?;
        let mut seen: Vec<&String> = Vec::new();
        for name in names {
            check_workspace_name(name)?;
            if let Some(p) = auth::project_of(name) {
                if !self.project_exists(p) {
                    return Err(KivaError::NotFound(format!(
                        "project '{p}' does not exist (create it with: project create {p})"
                    )));
                }
            }
            // Windows and macOS do not tell `Places` from `places`.
            if seen.iter().any(|s| s.eq_ignore_ascii_case(name)) {
                return Err(KivaError::Invalid(format!("collection '{name}' is named twice (capitals do not count)")));
            }
            seen.push(name);
            if self.cfg().workspace_dir(name).is_some() {
                return Err(KivaError::Exists(format!("collection '{name}' already exists")));
            }
        }
        if let [one] = names {
            return Ok(vec![self.create_workspace(one, None)?]);
        }
        let record = format!("pending:collections:{:016x}", crate::util::random_u64()?);
        self.system.put(&record, &Value::Doc(serde_json::json!({ "collections": names })), 0, false)?;
        self.system.sync()?;
        let dirs = self.finish_collections(names)?;
        self.system.delete(&record)?;
        self.system.sync()?;
        Ok(dirs)
    }

    /// Creates the collections of `names` that do not exist yet.
    fn finish_collections(&self, names: &[String]) -> Result<Vec<PathBuf>> {
        let mut dirs = Vec::new();
        for name in names {
            // The read lock of kiva.conf is let go before `create_workspace` writes it.
            let known = self.cfg().workspace_dir(name);
            dirs.push(match known {
                Some(d) => d,
                None => self.create_workspace(name, None)?,
            });
        }
        Ok(dirs)
    }

    /// Finishes the `project create ... collections` and the
    /// `collection create a b c` that a stop cut short.
    fn recover_pending_projects(&self) -> Result<()> {
        for (key, _) in self.system.keys_matching("pending:collections:*", 0) {
            let names: Vec<String> = match self.system.get(&key)? {
                Some((Value::Doc(j), _)) => j["collections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            };
            let projects_there = names.iter().all(|n| auth::project_of(n).is_none_or(|p| self.project_exists(p)));
            if names.iter().all(|n| auth::valid_workspace_name(n)) && projects_there {
                self.finish_collections(&names)?;
                eprintln!(
                    "[WARN] the creation of collections {}, stopped in the middle, was finished.",
                    names.join(", ")
                );
            }
            self.system.delete(&key)?;
        }
        for (key, _) in self.system.keys_matching("pending:project:*", 0) {
            let name = key["pending:project:".len()..].to_string();
            let collections: Vec<String> = match self.system.get(&key)? {
                Some((Value::Doc(j), _)) => j["collections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            };
            if auth::valid_name_part(&name) && collections.iter().all(|c| auth::valid_name_part(c)) {
                self.finish_project(&name, &collections)?;
                eprintln!("[WARN] the creation of project '{name}', stopped in the middle, was finished.");
            }
            self.system.delete(&key)?;
        }
        self.system.sync()
    }

    /// Removes an empty project. Returns whether it existed.
    pub fn remove_project(&self, name: &str) -> Result<bool> {
        self.guard_legacy_catalog_mutation()?;
        let inside: Vec<String> =
            self.workspace_names().into_iter().filter(|w| auth::project_of(w) == Some(name)).collect();
        if !inside.is_empty() {
            return Err(KivaError::Invalid(format!(
                "project '{name}' still holds {}: move or remove them first",
                inside.join(", ")
            )));
        }
        self.system.delete(&format!("project:{name}"))
    }

    /// Makes sure a workspace is registered.
    pub fn ensure_workspace(&self, name: &str) -> Result<()> {
        if self.cfg().workspace_dir(name).is_none() {
            self.create_workspace_in(name, None, true)?;
        }
        Ok(())
    }

    /// Pushes storage settings from the configuration to every open engine.
    pub fn apply_storage_config(&self) {
        let o = self.engine_options();
        for (_, e) in self.open_engines() {
            e.set_auto_compact(o.auto_compact, o.compact_threshold);
            e.set_compression(o.compression);
            e.set_cache_limit(o.cache_bytes);
            e.set_memory_limit(o.max_memory_bytes);
            e.set_vector_limit(o.max_vector_bytes);
            e.set_sync_writes(o.sync_writes);
        }
    }

    fn on_op(&self, workspace: &str, op: &Op) {
        self.pubsub.publish(workspace, op);
        // Cached answers built from data that just changed are stale.
        match op {
            Op::Set { key, .. } | Op::Del { key } => {
                self.semantic.invalidate(workspace, key);
            }
            Op::Reset => self.semantic.reset(workspace),
        }
        self.repl.broadcast(workspace, op);
        // Redis clients waiting for data (BLPOP, XREAD BLOCK...) look again.
        self.resp.wake();
        // On a replica, what the master's triggers wrote arrives with the
        // rest: running them again would apply their effects twice.
        if IN_TRIGGER.with(|c| c.get()) || self.repl.is_replica() {
            return;
        }
        let (event, key) = match op {
            Op::Set { key, created: true, .. } => ("set", key),
            Op::Set { key, .. } => ("update", key),
            Op::Del { key } => ("del", key),
            Op::Reset => return,
        };
        let scripts: Vec<String> = {
            let triggers = self.triggers.read().unwrap_or_else(|e| e.into_inner());
            if triggers.is_empty() {
                return;
            }
            triggers
                .iter()
                .filter(|t| t.event == event && glob_match(&t.pattern, key))
                .map(|t| t.script.clone())
                .collect()
        };
        if scripts.is_empty() {
            return;
        }
        if let Some(tx) = self.trigger_tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = tx.send(TriggerJob { workspace: workspace.to_string(), event, key: key.clone(), scripts });
        }
    }

    fn start_trigger_worker(&self) {
        let (tx, rx) = mpsc::channel::<TriggerJob>();
        *self.trigger_tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let weak = self.weak.clone();
        let _ = std::thread::Builder::new().name("kiva-triggers".into()).spawn(move || {
            IN_TRIGGER.with(|c| c.set(true));
            for job in rx {
                let Some(inst) = weak.upgrade() else { break };
                let mut sess = crate::session::Session::system(inst.clone());
                if sess.switch_workspace(&job.workspace).is_err() {
                    continue;
                }
                for script in &job.scripts {
                    let args = vec![job.event.to_string(), job.key.clone()];
                    if let Err(e) = crate::script::run_stored(&mut sess, script, &args) {
                        inst.audit.log("system", &format!("trigger script '{script}' failed: {e}"), false);
                    }
                }
            }
        });
    }

    pub fn triggers(&self) -> Vec<Trigger> {
        self.triggers.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Re-reads trigger definitions from the system database.
    pub fn reload_triggers(&self) {
        let mut list = Vec::new();
        for (key, _) in self.system.keys_matching("trigger:*", 0) {
            if let Ok(Some((Value::Doc(j), _))) = self.system.get(&key) {
                if let (Some(e), Some(p), Some(s)) = (j["event"].as_str(), j["pattern"].as_str(), j["script"].as_str())
                {
                    list.push(Trigger {
                        name: key["trigger:".len()..].to_string(),
                        event: e.to_string(),
                        pattern: p.to_string(),
                        script: s.to_string(),
                    });
                }
            }
        }
        *self.triggers.write().unwrap_or_else(|e| e.into_inner()) = list;
    }

    /// Flushes and snapshots every database. Safe to call more than once.
    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.repl.stop_replica();
        for (_, e) in self.open_engines() {
            e.close();
        }
        {
            let _layout = self.managed_layout_lock.write().unwrap_or_else(|e| e.into_inner());
            let mut engines = self.managed_engines.lock().unwrap_or_else(|e| e.into_inner());
            engines.close();
        }
        self.system.close();
        let _ = fs::remove_file(self.home.join("instance.port"));
        let _ = fs::remove_file(self.home.join("instance.pid"));
    }
}

/// The record of a workspace move under way, kept in the system database
/// until the move is done.
fn move_record(from: &str) -> String {
    format!("moving:{from}")
}

/// Takes back a move that did not happen: the grants it gave on the new name
/// go, and so does its record.
fn undo_move(system: &Engine, from: &str, to: &str, doubled: &[String]) -> Result<()> {
    for name in doubled {
        if let Some(mut user) = auth::get_user(system, name)? {
            if user.grants.remove(to).is_some() {
                auth::save_user(system, &user)?;
            }
        }
    }
    system.delete(&move_record(from))?;
    system.sync()
}

/// Ends a move once kiva.conf names the workspace `to`: the old name leaves
/// the grants and the default workspaces, then the record goes. Running it
/// again changes nothing.
fn finish_move(system: &Engine, from: &str, to: &str) -> Result<()> {
    for mut user in auth::list_users(system)? {
        let old = user.grants.remove(from);
        if let Some(p) = old {
            let kept = user.grants.entry(to.to_string()).or_insert(p);
            *kept = (*kept).max(p);
        }
        let default = user.default_workspace.as_deref() == Some(from);
        if default {
            user.default_workspace = Some(to.to_string());
        }
        if old.is_some() || default {
            auth::save_user(system, &user)?;
        }
    }
    system.delete(&move_record(from))?;
    system.sync()
}

/// Finishes or takes back the workspace moves that a stop interrupted, from
/// what kiva.conf says: the workspace under its new name, the move is
/// finished; under its old one, it is taken back. Returns what was done.
pub(crate) fn recover_moves(system: &Engine, cfg: &Config) -> Result<Vec<String>> {
    let mut done = Vec::new();
    for (key, _) in system.keys_matching("moving:*", 0) {
        let record = match system.get(&key)? {
            Some((Value::Doc(j), _)) => j,
            _ => continue,
        };
        let (Some(from), Some(to)) = (record["from"].as_str(), record["to"].as_str()) else {
            system.delete(&key)?;
            continue;
        };
        if cfg.workspace_dir(to).is_some() && cfg.workspace_dir(from).is_none() {
            finish_move(system, from, to)?;
            done.push(format!("the move of workspace '{from}' to '{to}', stopped in the middle, was finished"));
        } else {
            let doubled: Vec<String> = record["doubled"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str().map(String::from))
                .collect();
            undo_move(system, from, to, &doubled)?;
            done.push(format!("the move of workspace '{from}' to '{to}', stopped before it happened, was taken back"));
        }
    }
    Ok(done)
}

/// The folder a new collection gets when no path is given (rc.5, section 5.1
/// of docs/architecture/PROJECTS_AND_COLLECTIONS.md): `<home>/projects/<project>/<collection>`
/// in a project, `<home>/collections/<name>` outside one. The name has been
/// checked (`check_workspace_name`): its parts hold no `.`, `/` or `\`.
/// Collections made before rc.5 keep their folders, written in kiva.conf.
pub fn collection_dir(home: &Path, name: &str) -> PathBuf {
    match name.split_once('.') {
        Some((project, collection)) => home.join("projects").join(project).join(collection),
        None => home.join("collections").join(name),
    }
}

/// The record that marks a `project create ... collections` in progress.
fn pending_project(name: &str) -> String {
    format!("pending:project:{name}")
}

/// Refuses a name that is not `<workspace>` or `<project>.<workspace>`.
pub fn check_workspace_name(name: &str) -> Result<()> {
    if auth::valid_workspace_name(name) {
        return Ok(());
    }
    Err(KivaError::Invalid(
        "workspace names may only contain letters, digits, '_' and '-', with one '.' after a project name \
         (travel.places)"
            .into(),
    ))
}
