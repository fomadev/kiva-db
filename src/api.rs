// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Structured (JSON) operations for graphical clients.
//!
//! The command language answers with text meant for people. Tools such as
//! Kiva Studio need data they can rely on, so the binary protocol carries a
//! second kind of request: a JSON object with an `op` field, answered by a
//! JSON object. Every operation goes through the same session, with the same
//! permissions, as the equivalent shell command.

use crate::auth::{self, Role};
use crate::error::{KivaError, Result};
use crate::exec::validate_key_name;
use crate::instance::VERSION;
use crate::session::{Need, Session};
use crate::util::{b64_decode, b64_encode, now_secs};
use crate::value::{is_valid_number, Type, Value};
use serde_json::{json, Value as Json};
use std::sync::atomic::Ordering;

fn invalid(msg: impl Into<String>) -> KivaError {
    KivaError::Invalid(msg.into())
}

fn str_field<'a>(req: &'a Json, name: &str) -> Result<&'a str> {
    req[name].as_str().ok_or_else(|| invalid(format!("missing field \"{name}\"")))
}

/// Value as JSON for display: bytes travel as base64.
fn value_out(v: &Value) -> Json {
    match v {
        Value::Bytes(b) => Json::String(b64_encode(b)),
        other => other.to_json(),
    }
}

/// Builds a value of the requested type from its JSON form.
fn value_in(ty: Type, j: &Json) -> Result<Value> {
    let mismatch = || invalid(format!("value is not a valid {}", ty.name()));
    Ok(match ty {
        Type::String => Value::Str(j.as_str().ok_or_else(mismatch)?.to_string()),
        Type::Number => {
            let text = match j {
                Json::Number(n) => n.to_string(),
                Json::String(s) => s.trim().to_string(),
                _ => return Err(mismatch()),
            };
            if !is_valid_number(&text) {
                return Err(mismatch());
            }
            Value::Num(text)
        }
        Type::Boolean => Value::Bool(j.as_bool().ok_or_else(mismatch)?),
        Type::Null => Value::Null,
        Type::Array if j.is_array() => Value::Doc(j.clone()),
        Type::Object if j.is_object() => Value::Doc(j.clone()),
        Type::Array | Type::Object => return Err(mismatch()),
        Type::Bytes => Value::Bytes(j.as_str().and_then(b64_decode).ok_or_else(mismatch)?),
    })
}

fn ttl_of(expires_at: i64) -> Json {
    if expires_at > 0 {
        json!((expires_at - now_secs()).max(0))
    } else {
        Json::Null
    }
}

/// Runs one structured request.
pub fn handle(sess: &mut Session, req: &Json) -> Result<Json> {
    let op = str_field(req, "op")?;
    match op {
        "whoami" => {
            sess.check(Need::Read)?;
            let role = sess.role().unwrap_or(Role::Viewer);
            Ok(json!({
                "version": VERSION,
                "user": sess.user_name(),
                "role": role.name(),
                "workspace": sess.ws,
                "access": sess.ws_priv.name(),
                "can_write": sess.writable().is_ok(),
                "auth_required": sess.inst.auth_required(),
                "replica": sess.inst.repl.is_replica(),
            }))
        }
        "scan" => {
            sess.check(Need::Read)?;
            let pattern = req["pattern"].as_str().filter(|p| !p.is_empty()).unwrap_or("*");
            let limit = req["limit"].as_u64().unwrap_or(500).clamp(1, 10_000) as usize;
            // One extra row tells whether there is more than the limit.
            let mut rows = sess.engine.keys_matching(pattern, limit + 1);
            let truncated = rows.len() > limit;
            rows.truncate(limit);
            let keys: Vec<Json> = rows
                .iter()
                .map(|(k, e)| {
                    json!({
                        "key": k,
                        "type": e.ty.name(),
                        "size": e.raw_size,
                        "modified": e.timestamp,
                        "ttl": ttl_of(e.expires_at),
                        "compression": e.compression().name(),
                        "msgpack": e.is_msgpack(),
                    })
                })
                .collect();
            Ok(json!({ "keys": keys, "truncated": truncated, "total": sess.engine.len() }))
        }
        "get" => {
            let key = str_field(req, "key")?;
            match sess.kv_get(key)? {
                Some((v, meta)) => Ok(json!({
                    "found": true,
                    "key": key,
                    "type": v.ty().name(),
                    "value": value_out(&v),
                    "ttl": ttl_of(meta.expires_at),
                    "modified": meta.timestamp,
                    "msgpack": meta.msgpack,
                })),
                None => Ok(json!({ "found": false, "key": key })),
            }
        }
        "put" => {
            let key = str_field(req, "key")?;
            let ty = Type::from_name(str_field(req, "type")?).ok_or_else(|| invalid("unknown type"))?;
            let value = value_in(ty, &req["value"])?;
            let existing = sess.kv_get(key)?;
            match (req["mode"].as_str().unwrap_or("upsert"), &existing) {
                ("create", Some(_)) => {
                    return Err(KivaError::Exists(format!("Key '{key}' already exists.")));
                }
                ("update", None) => return Err(KivaError::NotFound(format!("Key '{key}' not found."))),
                ("create" | "update" | "upsert", _) => {}
                (other, _) => return Err(invalid(format!("unknown mode '{other}'"))),
            }
            if existing.is_none() {
                validate_key_name(key).map_err(KivaError::Invalid)?;
            }
            let meta = existing.map(|(_, m)| m).unwrap_or_default();
            // ttl: absent keeps the current expiry, 0 removes it, N sets it.
            let expires_at = match req["ttl"].as_i64() {
                None => meta.expires_at,
                Some(t) if t > 0 => now_secs() + t,
                Some(_) => 0,
            };
            sess.kv_put(key, value, expires_at, meta.msgpack)?;
            Ok(json!({ "key": key, "type": ty.name() }))
        }
        "del" => {
            let key = str_field(req, "key")?;
            let deleted = sess.kv_del(key)?;
            if deleted {
                sess.inst.audit.log(sess.user_name(), &format!("del {key}"), true);
            }
            Ok(json!({ "deleted": deleted }))
        }
        "stats" => {
            sess.check(Need::Read)?;
            let s = sess.engine.stats();
            let inst = &sess.inst;
            let repl = &inst.repl;
            let seq = repl.seq();
            let replicas = repl.replicas();
            let link = repl.link();
            Ok(json!({
                "version": VERSION,
                "workspace": sess.ws,
                "path": s.path,
                "keys": s.keys,
                "ttl_keys": s.ttl_keys,
                "storage_bytes": s.file_bytes,
                "live_bytes": s.live_bytes,
                "fragmentation": s.fragmentation,
                "memory_bytes": s.memory_bytes,
                "compression": s.compression,
                "raw_value_bytes": s.raw_value_bytes,
                "stored_value_bytes": s.stored_value_bytes,
                "compressed_records": s.compressed_records,
                "startup_ms": s.load_ms,
                "startup_mode": s.load_mode,
                "checksum_errors": s.corrupted_detected,
                "last_check": s.last_check.map(|(ts, ok, bad)| json!({ "at": ts, "ok": ok, "corrupted": bad })),
                "compactions": s.compactions,
                "last_compact": s.last_compact,
                "auto_compact": s.auto_compact,
                "compact_threshold": s.compact_threshold,
                "indexes": s.indexes,
                "clients": inst.clients.load(Ordering::SeqCst),
                "uptime_seconds": inst.started.elapsed().as_secs(),
                "role": if repl.is_replica() { "replica" } else { "master" },
                "replicas": replicas.len(),
                "replication_lag": replicas.iter().map(|r| seq.saturating_sub(r.acked)).max().unwrap_or(0),
                "master": if repl.is_replica() { json!(link.master) } else { Json::Null },
                "link_status": if repl.is_replica() { json!(link.status) } else { Json::Null },
            }))
        }
        "users" => {
            sess.check(Need::Admin)?;
            let users: Vec<Json> = auth::list_users(&sess.inst.system)?
                .iter()
                .map(|u| {
                    let grants: serde_json::Map<String, Json> =
                        u.grants.iter().map(|(w, p)| (w.clone(), json!(p.name()))).collect();
                    json!({
                        "name": u.name,
                        "role": u.role.name(),
                        "created": u.created,
                        "home": u.home_workspace(),
                        "grants": grants,
                        "empty_password": u.has_empty_password(),
                        "builtin": u.name == auth::DEFAULT_ADMIN,
                    })
                })
                .collect();
            Ok(json!({ "users": users }))
        }
        "workspaces" => {
            sess.check(Need::Read)?;
            let me = match sess.role() {
                Some(Role::Admin) | None => None,
                Some(_) => auth::get_user(&sess.inst.system, sess.user_name())?,
            };
            let mut list = Vec::new();
            for (name, path) in sess.inst.workspaces() {
                let access = match &me {
                    Some(u) => match u.access(&name) {
                        Some(p) => p.name(),
                        None => continue,
                    },
                    None => "write",
                };
                list.push(json!({ "name": name, "path": path, "access": access, "current": name == sess.ws }));
            }
            Ok(json!({ "workspaces": list }))
        }
        other => Err(invalid(format!("unknown operation '{other}'"))),
    }
}
