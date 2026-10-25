// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! A running KivaDB instance: configuration, the system database, every open
//! workspace, and the services shared by all sessions.

use crate::auth;
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
    pub http_requests: AtomicU64,
    pub failed_logins: AtomicU64,
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
    pub config: RwLock<Config>,
    /// Hidden database holding users, stored scripts and triggers.
    pub system: Arc<Engine>,
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
        fs::create_dir_all(home)?;
        let lock = OpenOptions::new().create(true).write(true).truncate(false).open(home.join("instance.lock"))?;
        if !crate::util::lock_exclusive(&lock) {
            return Err(KivaError::Locked(home.display().to_string()));
        }
        let _ = fs::remove_file(home.join("instance.port"));
        let _ = fs::remove_file(home.join("instance.pid"));
        let config = Config::load(home)?;
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
        auth::ensure_admin(&system)?;
        let threads = config.get_u64("server", "threads") as usize;

        let inst = Arc::new_cyclic(|weak| Instance {
            home: home.to_path_buf(),
            config: RwLock::new(config),
            system,
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

    pub fn create_workspace(&self, name: &str, dir: Option<&str>) -> Result<PathBuf> {
        let valid = !name.is_empty()
            && name.len() <= 64
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
        if !valid {
            return Err(KivaError::Invalid("workspace names may only contain letters, digits, '_' and '-'".into()));
        }
        if self.cfg().workspace_dir(name).is_some() {
            return Err(KivaError::Exists(format!("workspace '{name}' already exists")));
        }
        let dir = match dir {
            Some(d) => PathBuf::from(d),
            None => self.home.join("workspaces").join(name),
        };
        fs::create_dir_all(&dir)?;
        self.cfg_mut().set("workspaces", name, &dir.display().to_string())?;
        Ok(dir)
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

    /// Makes sure a user's personal workspace is registered.
    pub fn ensure_workspace(&self, name: &str) -> Result<()> {
        if self.cfg().workspace_dir(name).is_none() {
            self.create_workspace(name, None)?;
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
            e.set_sync_writes(o.sync_writes);
        }
    }

    fn on_op(&self, workspace: &str, op: &Op) {
        self.pubsub.publish(workspace, op);
        // Cached answers built from data that just changed are stale.
        match op {
            Op::Set { key, .. } | Op::Del { key } => self.semantic.invalidate(workspace, key),
            Op::Reset => {
                self.semantic.clear(workspace, None);
            }
        }
        if workspace == "default" {
            self.repl.broadcast(op);
        }
        if IN_TRIGGER.with(|c| c.get()) {
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
        self.system.close();
        let _ = fs::remove_file(self.home.join("instance.port"));
        let _ = fs::remove_file(self.home.join("instance.pid"));
    }
}
