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
    /// An administrator anywhere, or a user who can write, in their own
    /// personal workspace (maintenance of one's own data).
    Owner,
    Admin,
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
    pub ws_priv: Priv,
    pub txn: Option<Txn>,
    /// True for the local shell; remote sessions cannot log in with an empty password.
    pub local: bool,
    /// Where key-change notifications for this session are delivered.
    pub events: Option<Sender<String>>,
    pub subs: Vec<String>,
    /// Set when the session wrote to the replicated workspace.
    pub wrote: bool,
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
            ws_priv: Priv::Read,
            txn: None,
            local,
            events,
            subs: Vec::new(),
            wrote: false,
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
        self.ws_priv = Priv::Write;
    }

    pub fn user_name(&self) -> &str {
        self.user.as_ref().map(|u| u.name.as_str()).unwrap_or("-")
    }

    pub fn role(&self) -> Option<Role> {
        self.user.as_ref().map(|u| u.role)
    }

    /// Verifies credentials and enters the user's home workspace.
    pub fn login(&mut self, name: &str, password: &str) -> Result<User> {
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
        let home = user.home_workspace();
        self.inst.ensure_workspace(&home)?;
        self.enter(&home, Priv::Write)?;
        self.inst.audit.log(name, "login", true);
        Ok(user)
    }

    fn enter(&mut self, workspace: &str, level: Priv) -> Result<()> {
        let engine = self.inst.engine(workspace)?;
        // Subscriptions are per workspace: they do not follow the session.
        if !self.subs.is_empty() {
            self.inst.pubsub.unsubscribe(self.id, None);
            self.subs.clear();
        }
        self.engine = engine;
        self.ws = workspace.to_string();
        self.ws_priv = level;
        Ok(())
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
        match need {
            Need::Admin if u.role != Role::Admin => {
                Err(KivaError::Denied("this command requires the admin role".into()))
            }
            Need::Owner if u.role != Role::Admin => {
                let home = crate::auth::personal_workspace(&u.name);
                if u.role == Role::Viewer {
                    Err(KivaError::Denied("the viewer role is read-only".into()))
                } else if self.ws != home {
                    Err(KivaError::Denied(format!(
                        "this command requires the admin role, except in your personal workspace '{home}'"
                    )))
                } else {
                    Ok(())
                }
            }
            Need::Write if u.role == Role::Viewer => Err(KivaError::Denied("the viewer role is read-only".into())),
            Need::Write if self.ws_priv < Priv::Write => {
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
        let name = format!("similar {}.{} {} {} {}", q.pattern, q.field, q.metric.name(), q.limit, q.ef);
        let question = Question { text: None, vector: Some(q.vector.clone()) };
        if let Lookup::Hit { similarity, answer, .. } =
            self.inst.semantic.lookup(&self.ws, &name, &question, s.threshold)?
        {
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
            };
            return Ok((r, Some(similarity)));
        }
        let r = self.engine.similar(q)?;
        let rows: Vec<serde_json::Value> =
            r.rows.iter().map(|(k, score, doc)| serde_json::json!({ "key": k, "score": score, "doc": doc })).collect();
        let answer = serde_json::json!({ "rows": rows, "indexed": r.indexed, "approximate": r.approximate });
        // Any write under the searched pattern makes the answer stale.
        self.inst.semantic.store(&self.ws, &name, &question, answer, s.ttl, vec![q.pattern.clone()], s.max_entries)?;
        Ok((r, None))
    }

    /// Checks that this session may modify data right now.
    pub fn writable(&self) -> Result<()> {
        self.check(Need::Write)?;
        if self.ws == "default" && self.inst.repl.is_replica() {
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
            self.wrote = self.ws == "default";
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
        self.wrote = self.ws == "default";
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
        let res = match self.writable() {
            Ok(()) => self.engine.apply_batch(ops, Some(txn.begin_seq)),
            Err(e) => Err(e),
        };
        self.engine.txn_end();
        res?;
        self.wrote = self.ws == "default" && n > 0;
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

impl Drop for Session {
    fn drop(&mut self) {
        self.rollback();
        if !self.subs.is_empty() {
            self.inst.pubsub.unsubscribe(self.id, None);
        }
    }
}
