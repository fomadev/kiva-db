// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! A session: one authenticated user working in one workspace, possibly
//! inside a transaction. Every front end (shell, TCP, HTTP, Lua) goes
//! through the typed operations defined here.

use crate::auth::{self, Priv, Role, User};
use crate::engine::{BatchOp, Engine};
use crate::error::{KivaError, Result};
use crate::instance::Instance;
use crate::value::{Type, Value};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct UserCtx {
    pub name: String,
    pub role: Role,
}

#[derive(Clone, Debug)]
pub enum TxOp {
    Put { value: Value, expires_at: i64, msgpack: bool },
    Del,
}

/// Buffered writes of an open transaction. Nothing reaches the engine until
/// `commit`, which applies everything as one atomic batch.
pub struct Txn {
    pub id: u64,
    pub begin_seq: u64,
    pub writes: BTreeMap<String, TxOp>,
    pub savepoints: Vec<(String, BTreeMap<String, TxOp>)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Need {
    None,
    Read,
    Write,
    /// An administrator anywhere, or a user who can write in every
    /// workspace of the current one's project (maintenance of its data).
    Owner,
    Admin,
    /// Logged in, in a workspace or not (`workspace list`, `whoami`).
    Login,
}

/// What a read returns besides the value itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub expires_at: i64,
    pub msgpack: bool,
    pub timestamp: i64,
}

pub struct Session {
    pub inst: Arc<Instance>,
    pub id: u64,
    pub user: Option<UserCtx>,
    pub ws: String,
    pub engine: Arc<Engine>,
    /// Access to the current workspace; `None` when the session is in none
    /// (`ws` is then empty, and `engine` the empty `nowhere` store).
    pub ws_priv: Option<Priv>,
    pub txn: Option<Txn>,
    /// True for the local shell; remote sessions cannot log in with an empty password.
    pub local: bool,
    /// Where key-change notifications for this session are delivered.
    pub events: Option<Sender<String>>,
    pub subs: Vec<String>,
    /// Set when the session wrote (every workspace is replicated).
    pub wrote: bool,
    /// Keys watched by Redis `WATCH`, with the engine they belong to.
    pub watch: Option<(Arc<Engine>, Vec<crate::engine::WatchedKey>)>,
}

impl Session {
    pub fn new(inst: Arc<Instance>, local: bool, events: Option<Sender<String>>) -> Session {
        let engine = inst.engine("default").expect("default workspace is open");
        let mut s = Session {
            id: inst.next_id(),
            inst,
            user: None,
            ws: "default".into(),
            engine,
            ws_priv: Some(Priv::Read),
            txn: None,
            local,
            events,
            subs: Vec::new(),
            wrote: false,
            watch: None,
        };
        if !s.inst.auth_required() {
            s.become_root();
        }
        s
    }

    /// An internal, fully privileged session (triggers, replication tooling).
    pub fn system(inst: Arc<Instance>) -> Session {
        let mut s = Session::new(inst, true, None);
        s.become_root();
        s
    }

    fn become_root(&mut self) {
        self.user = Some(UserCtx { name: auth::DEFAULT_ADMIN.into(), role: Role::Admin });
        self.ws_priv = Some(Priv::Write);
    }

    pub fn user_name(&self) -> &str {
        self.user.as_ref().map(|u| u.name.as_str()).unwrap_or("-")
    }

    pub fn role(&self) -> Option<Role> {
        self.user.as_ref().map(|u| u.role)
    }

    /// Verifies credentials and enters the workspace the user arrives in:
    /// their default one, or the only one they can reach, or none. A name
    /// `app@travel.places` logs `app` in, straight into `travel.places`: clients
    /// send it again at every reconnection, in a URL like any user name.
    pub fn login(&mut self, name: &str, password: &str) -> Result<User> {
        let Some((user, workspace)) = name.split_once('@') else { return self.login_user(name, password) };
        let u = self.login_user(user, password)?;
        if let Err(e) = self.switch_workspace(workspace) {
            self.leave();
            self.user = None;
            return Err(e);
        }
        Ok(u)
    }

    fn login_user(&mut self, name: &str, password: &str) -> Result<User> {
        let denied = |inst: &Instance, why: &str| {
            inst.metrics.failed_logins.fetch_add(1, Ordering::Relaxed);
            inst.audit.log(name, &format!("login ({why})"), false);
            KivaError::Denied("invalid username or password".into())
        };
        let Some(user) = auth::get_user(&self.inst.system, name)? else {
            return Err(denied(&self.inst, "unknown user"));
        };
        if !user.verify(password) {
            return Err(denied(&self.inst, "wrong password"));
        }
        if password.is_empty() && !self.local {
            self.inst.audit.log(name, "login (empty password over the network)", false);
            return Err(KivaError::Denied(
                "accounts with an empty password can only log in from the local shell".into(),
            ));
        }
        if self.txn.is_some() {
            self.rollback();
        }
        self.user = Some(UserCtx { name: user.name.clone(), role: user.role });
        match user.arrival(&self.inst.workspace_names()) {
            Some(w) => {
                let level = user.access(&w).unwrap_or(Priv::Read);
                self.enter(&w, level)?;
            }
            None => self.leave(),
        }
        self.inst.audit.log(name, "login", true);
        Ok(user)
    }

    fn enter(&mut self, workspace: &str, level: Priv) -> Result<()> {
        let engine = self.inst.engine(workspace)?;
        self.drop_subscriptions();
        self.engine = engine;
        self.ws = workspace.to_string();
        self.ws_priv = Some(level);
        Ok(())
    }

    /// Leaves the current workspace for none.
    pub fn leave(&mut self) {
        self.drop_subscriptions();
        self.engine = self.inst.nowhere.clone();
        self.ws.clear();
        self.ws_priv = None;
    }

    // Subscriptions are per workspace: they do not follow the session.
    fn drop_subscriptions(&mut self) {
        if !self.subs.is_empty() {
            self.inst.pubsub.unsubscribe(self.id, None);
            self.subs.clear();
        }
    }

    pub fn switch_workspace(&mut self, workspace: &str) -> Result<()> {
        if self.txn.is_some() {
            return Err(KivaError::Invalid("cannot switch workspace inside a transaction".into()));
        }
        let ctx = self.user.clone().ok_or_else(|| KivaError::Denied("authentication required".into()))?;
        if self.inst.cfg().workspace_dir(workspace).is_none() {
            return Err(KivaError::NotFound(format!("workspace '{workspace}' does not exist")));
        }
        let level = if ctx.role == Role::Admin {
            Priv::Write
        } else {
            // Grants may have changed since login: always read the stored account.
            auth::get_user(&self.inst.system, &ctx.name)?
                .and_then(|u| u.access(workspace))
                .ok_or_else(|| KivaError::Denied(format!("no access to workspace '{workspace}'")))?
        };
        self.enter(workspace, level)
    }

    pub fn check(&self, need: Need) -> Result<()> {
        if need == Need::None {
            return Ok(());
        }
        let Some(u) = &self.user else {
            return Err(KivaError::Denied("authentication required. Use: auth <user> <password>".into()));
        };
        if matches!(need, Need::Read | Need::Write | Need::Owner) && self.ws_priv.is_none() {
            return Err(KivaError::Denied(
                "you are in no workspace: 'workspace list' shows the ones you can use, 'workspace switch <name>' \
                 enters one"
                    .into(),
            ));
        }
        match need {
            Need::Admin if u.role != Role::Admin => {
                Err(KivaError::Denied("this command requires the admin role".into()))
            }
            Need::Owner if u.role != Role::Admin => {
                if u.role == Role::Viewer {
                    return Err(KivaError::Denied("the viewer role is read-only".into()));
                }
                let manages = auth::get_user(&self.inst.system, &u.name)?.is_some_and(|u| u.manages(&self.ws));
                if manages {
                    return Ok(());
                }
                Err(KivaError::Denied(match auth::project_of(&self.ws) {
                    Some(p) => format!(
                        "this command requires the admin role, or write access to every workspace of project '{p}' \
                         (grant write on {p}.*)"
                    ),
                    None => "this command requires the admin role".to_string(),
                }))
            }
            Need::Write if u.role == Role::Viewer => Err(KivaError::Denied("the viewer role is read-only".into())),
            Need::Write if self.ws_priv < Some(Priv::Write) => {
                Err(KivaError::Denied(format!("you only have read access to workspace '{}'", self.ws)))
            }
            _ => Ok(()),
        }
    }

    /// `get similar`, answered from the semantic cache when
    /// `semantic.cache.enabled` is on and a near-identical query was run
    /// recently on the same data. Returns the similarity of that query on a hit.
    pub fn similar_cached(
        &mut self,
        q: &crate::engine::VectorQuery,
    ) -> Result<(crate::engine::VectorResult, Option<f32>)> {
        use crate::semantic_cache::{Lookup, Question, Settings};
        let s = Settings::of(&self.inst.cfg());
        if !s.enabled || q.exact {
            return Ok((self.engine.similar(q)?, None));
        }
        let mut name = format!("similar {}.{} {} {} {}", q.pattern, q.field, q.metric.name(), q.limit, q.ef);
        // Another filter is another question, however close the vectors.
        for c in &q.filter {
            name.push_str(&format!(" where {} {} {}", c.field, c.op.symbol(), c.value));
        }
        let question = Question { text: None, vector: Some(q.vector.clone()) };
        let since = match self.inst.semantic.lookup(&self.ws, &name, &question, s.threshold)? {
            Lookup::Miss { since, .. } => since,
            Lookup::Hit { similarity, answer, .. } => {
                let rows = answer["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        let key = r["key"].as_str().unwrap_or("").to_string();
                        (key, r["score"].as_f64().unwrap_or(0.0) as f32, r["doc"].clone())
                    })
                    .collect();
                let r = crate::engine::VectorResult {
                    rows,
                    indexed: answer["indexed"].as_bool().unwrap_or(false),
                    approximate: answer["approximate"].as_bool().unwrap_or(false),
                    examined: 0,
                    compared: 0,
                    filter: None,
                };
                return Ok((r, Some(similarity)));
            }
        };
        let r = self.engine.similar(q)?;
        let rows: Vec<serde_json::Value> =
            r.rows.iter().map(|(k, score, doc)| serde_json::json!({ "key": k, "score": score, "doc": doc })).collect();
        let answer = serde_json::json!({ "rows": rows, "indexed": r.indexed, "approximate": r.approximate });
        // Any write under the searched pattern makes the answer stale, also
        // one made during the search: then it is not kept at all.
        let depends = vec![q.pattern.clone()];
        self.inst.semantic.store_since(
            &self.ws,
            &name,
            &question,
            answer,
            s.ttl,
            depends,
            s.max_entries,
            Some(since),
        )?;
        Ok((r, None))
    }

    /// Checks that this session may modify data right now.
    pub fn writable(&self) -> Result<()> {
        self.check(Need::Write)?;
        if self.inst.repl.is_replica() {
            // Name the master, so that clients know where to go.
            let master = self.inst.repl.link().master;
            return Err(KivaError::ReadOnly(format!(
                "this node is a replica of {master}; write to the master or run 'replica promote'"
            )));
        }
        Ok(())
    }

    // -- typed key/value operations -----------------------------------------

    pub fn kv_get(&mut self, key: &str) -> Result<Option<(Value, Meta)>> {
        self.check(Need::Read)?;
        self.inst.metrics.gets.fetch_add(1, Ordering::Relaxed);
        if let Some(op) = self.txn.as_ref().and_then(|t| t.writes.get(key)) {
            return Ok(match op {
                TxOp::Put { value, expires_at, msgpack } => {
                    Some((value.clone(), Meta { expires_at: *expires_at, msgpack: *msgpack, timestamp: 0 }))
                }
                TxOp::Del => None,
            });
        }
        Ok(self
            .engine
            .get(key)?
            .map(|(v, e)| (v, Meta { expires_at: e.expires_at, msgpack: e.is_msgpack(), timestamp: e.timestamp })))
    }

    /// The type of `key` without reading its value from disk.
    pub fn kv_type(&mut self, key: &str) -> Result<Option<Type>> {
        self.check(Need::Read)?;
        if let Some(op) = self.txn.as_ref().and_then(|t| t.writes.get(key)) {
            return Ok(match op {
                TxOp::Put { value, .. } => Some(value.ty()),
                TxOp::Del => None,
            });
        }
        Ok(self.engine.meta(key).map(|e| e.ty))
    }

    pub fn kv_exists(&mut self, key: &str) -> Result<bool> {
        Ok(self.kv_type(key)?.is_some())
    }

    /// Unconditional write (create or overwrite).
    pub fn kv_put(&mut self, key: &str, value: Value, expires_at: i64, msgpack: bool) -> Result<()> {
        self.apply(vec![BatchOp::Put { key: key.to_string(), value, expires_at, msgpack }])
    }

    /// Read-modify-write of one key, atomic with respect to other clients:
    /// see [`crate::engine::Engine::update_with`]. Inside a transaction the
    /// change is buffered like any other write. Returns `None` when the key
    /// does not exist.
    pub fn kv_modify<T>(
        &mut self,
        key: &str,
        f: impl FnOnce(Value) -> Result<(Option<Value>, T)>,
    ) -> Result<Option<T>> {
        if self.txn.is_some() {
            let Some((value, meta)) = self.kv_get(key)? else {
                return Ok(None);
            };
            let (new, out) = f(value)?;
            if let Some(new) = new {
                self.kv_put(key, new, meta.expires_at, meta.msgpack)?;
            }
            return Ok(Some(out));
        }
        self.writable()?;
        self.inst.metrics.gets.fetch_add(1, Ordering::Relaxed);
        let Some((out, wrote)) = self.engine.update_with(key, f)? else {
            return Ok(None);
        };
        if wrote {
            self.inst.metrics.sets.fetch_add(1, Ordering::Relaxed);
            self.wrote = true;
        }
        Ok(Some(out))
    }

    pub fn kv_del(&mut self, key: &str) -> Result<bool> {
        let existed = self.kv_exists(key)?;
        if existed {
            self.apply(vec![BatchOp::Del { key: key.to_string() }])?;
        }
        Ok(existed)
    }

    /// Applies operations atomically: buffered if a transaction is open,
    /// otherwise written to the engine as one batch.
    pub fn apply(&mut self, ops: Vec<BatchOp>) -> Result<()> {
        self.writable()?;
        for op in &ops {
            let counter = match op {
                BatchOp::Put { .. } => &self.inst.metrics.sets,
                BatchOp::Del { .. } => &self.inst.metrics.dels,
            };
            counter.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(txn) = self.txn.as_mut() {
            for op in ops {
                match op {
                    BatchOp::Put { key, value, expires_at, msgpack } => {
                        if key.is_empty() || key.len() > crate::format::MAX_KEY_LEN {
                            return Err(KivaError::Invalid("invalid key length".into()));
                        }
                        txn.writes.insert(key, TxOp::Put { value, expires_at, msgpack });
                    }
                    BatchOp::Del { key } => {
                        txn.writes.insert(key, TxOp::Del);
                    }
                }
            }
            return Ok(());
        }
        self.engine.check_room(put_keys(&ops))?;
        self.wrote = true;
        let mut ops = ops;
        if ops.len() == 1 {
            match ops.pop().unwrap() {
                BatchOp::Put { key, value, expires_at, msgpack } => {
                    self.engine.put(&key, &value, expires_at, msgpack)?;
                }
                BatchOp::Del { key } => {
                    self.engine.delete(&key)?;
                }
            }
            Ok(())
        } else {
            self.engine.apply_batch(ops, None)
        }
    }

    // -- transactions -------------------------------------------------------

    pub fn begin(&mut self) -> Result<u64> {
        self.writable()?;
        if self.txn.is_some() {
            return Err(KivaError::Invalid("a transaction is already active".into()));
        }
        let id = self.inst.next_id();
        let begin_seq = self.engine.txn_begin();
        self.txn = Some(Txn { id, begin_seq, writes: BTreeMap::new(), savepoints: Vec::new() });
        Ok(id)
    }

    /// Commits the open transaction. Returns the number of keys written.
    pub fn commit(&mut self) -> Result<usize> {
        let txn = self.txn.take().ok_or_else(|| KivaError::Invalid("no active transaction".into()))?;
        let n = txn.writes.len();
        let ops: Vec<BatchOp> = txn
            .writes
            .into_iter()
            .map(|(key, op)| match op {
                TxOp::Put { value, expires_at, msgpack } => BatchOp::Put { key, value, expires_at, msgpack },
                TxOp::Del => BatchOp::Del { key },
            })
            .collect();
        // Watched keys of the same workspace are checked with the writes;
        // those of another one (WATCH, then SELECT) just before.
        let (same, other) = match &self.watch {
            Some((e, w)) if Arc::ptr_eq(e, &self.engine) => (Some(w.as_slice()), None),
            Some((e, w)) => (None, e.watch_conflict(w)),
            None => (None, None),
        };
        let res = match self.writable().and_then(|_| self.engine.check_room(put_keys(&ops))) {
            Ok(()) if other.is_some() => {
                Err(KivaError::Conflict(format!("the watched key '{}' changed", other.unwrap_or_default())))
            }
            Ok(()) => self.engine.apply_batch_watched(ops, Some(txn.begin_seq), same),
            Err(e) => Err(e),
        };
        self.engine.txn_end();
        res?;
        self.wrote = n > 0;
        Ok(n)
    }

    /// Discards the open transaction, if any.
    pub fn rollback(&mut self) -> bool {
        match self.txn.take() {
            Some(_) => {
                self.engine.txn_end();
                true
            }
            None => false,
        }
    }

    pub fn savepoint(&mut self, name: &str) -> Result<()> {
        let txn = self.txn.as_mut().ok_or_else(|| KivaError::Invalid("no active transaction".into()))?;
        txn.savepoints.push((name.to_string(), txn.writes.clone()));
        Ok(())
    }

    pub fn rollback_to(&mut self, name: &str) -> Result<()> {
        let txn = self.txn.as_mut().ok_or_else(|| KivaError::Invalid("no active transaction".into()))?;
        let pos = txn
            .savepoints
            .iter()
            .rposition(|(n, _)| n == name)
            .ok_or_else(|| KivaError::NotFound(format!("savepoint '{name}' does not exist")))?;
        txn.writes = txn.savepoints[pos].1.clone();
        txn.savepoints.truncate(pos + 1);
        Ok(())
    }
}

/// The keys a batch writes (not those it deletes).
fn put_keys(ops: &[BatchOp]) -> impl Iterator<Item = &str> {
    ops.iter().filter_map(|op| match op {
        BatchOp::Put { key, .. } => Some(key.as_str()),
        BatchOp::Del { .. } => None,
    })
}

impl Session {
    /// Watches `keys` of the current workspace (Redis `WATCH`); the next
    /// commit fails as a conflict if one of them changes meanwhile.
    pub fn watch(&mut self, keys: &[String]) {
        match &mut self.watch {
            Some((e, w)) if Arc::ptr_eq(e, &self.engine) => {
                // Already watching here: the engine holds one watch already.
                let more = self.engine.watch(keys);
                self.engine.unwatch();
                w.extend(more);
            }
            _ => {
                self.unwatch();
                self.watch = Some((self.engine.clone(), self.engine.watch(keys)));
            }
        }
    }

    /// Forgets the watched keys (`UNWATCH`, and after `EXEC` or `DISCARD`).
    pub fn unwatch(&mut self) {
        if let Some((e, _)) = self.watch.take() {
            e.unwatch();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.unwatch();
        self.rollback();
        if !self.subs.is_empty() {
            self.inst.pubsub.unsubscribe(self.id, None);
        }
    }
}
