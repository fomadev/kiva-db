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
use crate::util::{b64_decode, b64_encode, deadline_in, secs_left};
use crate::value::{is_valid_number, Type, Value};
use serde_json::{json, Value as Json};
use std::sync::atomic::Ordering;

fn invalid(msg: impl Into<String>) -> KivaError {
    KivaError::Invalid(msg.into())
}

/// The filter of a vector search: `{"field": value, ...}` for equalities,
/// or `[{"field": f, "cmp": ">=", "value": v}, ...]`; every condition must hold.
/// The query of a structured request:
/// - `"target": "user:*", "where": {...}` (see [`crate::query::Expr::from_json`]);
/// - `"target": "user:*.age", "cmp": ">", "value": 18`: one comparison;
/// - for an aggregate, `"target": "user:*"` alone: every document.
fn request_query(req: &Json, aggregate: bool) -> Result<crate::query::Query> {
    use crate::query::{Expr, Query};
    let target = str_field(req, "target")?;
    let single = !req["cmp"].is_null() || !req["value"].is_null();
    if !req["where"].is_null() {
        if single {
            return Err(invalid("give either \"where\" or \"cmp\" and \"value\", not both"));
        }
        if crate::exec::split_target(target).is_ok() {
            return Err(invalid(format!(
                "with \"where\", \"target\" is the key pattern alone ('{target}' names a field after it): \
                 the fields go in the conditions"
            )));
        }
        let expr = Expr::from_json(&req["where"], target).map_err(KivaError::Invalid)?;
        return Ok(Query { pattern: target.to_string(), expr: Some(expr) });
    }
    if aggregate && !single {
        if crate::exec::split_target(target).is_ok() {
            return Err(invalid(format!(
                "\"target\" is the key pattern alone ('{target}' names a field after it): the fields \
                 an aggregate reads go in \"compute\", e.g. {{\"avg\": \"age\"}}"
            )));
        }
        return Ok(Query { pattern: target.to_string(), expr: None });
    }
    let (pattern, field) = crate::exec::split_target(target).map_err(KivaError::Invalid)?;
    let cmp = crate::engine::CmpOp::parse(req["cmp"].as_str().unwrap_or("="))
        .ok_or_else(|| invalid(format!("cmp must be one of {}", crate::engine::CmpOp::LIST)))?;
    let cond = crate::engine::Cond::new(pattern, field, cmp, req["value"].clone());
    Query::single(cond).map_err(KivaError::Invalid)
}

/// `"order_by": "created_at desc"`, `"after"`, `"reverse"` and `"limit"` of a query.
fn read_opts(req: &Json, q: &crate::query::Query, limit: usize) -> Result<crate::engine::ReadOpts> {
    let order = match &req["order_by"] {
        Json::Null => None,
        Json::String(s) => {
            let mut words = s.split_whitespace();
            let field = words.next().ok_or_else(|| invalid("\"order_by\" names a field: \"created_at desc\""))?;
            let desc = match words.next().map(str::to_ascii_lowercase).as_deref() {
                None | Some("asc") => false,
                Some("desc") => true,
                Some(w) => return Err(invalid(format!("\"order_by\": '{w}' is neither asc nor desc"))),
            };
            if words.next().is_some() {
                return Err(invalid("\"order_by\" is one field, then asc or desc: KivaDB sorts by one field"));
            }
            let field = field.strip_prefix(&q.pattern).and_then(|f| f.strip_prefix('.')).unwrap_or(field);
            Some(crate::engine::Order::new(field, desc).map_err(KivaError::Invalid)?)
        }
        _ => return Err(invalid("\"order_by\" is a text: \"created_at\" or \"created_at desc\"")),
    };
    let after = match &req["after"] {
        Json::Null => None,
        a => Some(a.as_str().ok_or_else(|| invalid("\"after\" is the key to go on after"))?.to_string()),
    };
    let reverse = req["reverse"].as_bool().unwrap_or(false);
    if order.is_some() && reverse {
        return Err(invalid("with \"order_by\", write \"created_at desc\" rather than \"reverse\""));
    }
    Ok(crate::engine::ReadOpts { limit, order, after, reverse })
}

pub fn measured_json(m: &crate::engine::Measured) -> Json {
    let ms = |d: std::time::Duration| (d.as_secs_f64() * 1_000_000.0).round() / 1000.0;
    json!({
        "index_entries": m.index_entries, "keys_examined": m.keys_examined, "documents_read": m.docs_read,
        "from_cache": m.from_cache, "from_disk": m.docs_read - m.from_cache, "filtered_out": m.filtered_out,
        "sorted": m.sorted, "returned": m.returned,
        "time_ms": { "total": ms(m.total_time), "plan": ms(m.plan_time), "read": ms(m.read_time), "sort": ms(m.sort_time) },
    })
}

fn vector_filter(j: &Json, pattern: &str) -> Result<Vec<crate::engine::Cond>> {
    let cond = |field: &str, cmp: &str, value: &Json| -> Result<crate::engine::Cond> {
        let field = field.strip_prefix(pattern).and_then(|f| f.strip_prefix('.')).unwrap_or(field);
        crate::query::check_field(field).map_err(KivaError::Invalid)?;
        let op = crate::engine::CmpOp::parse(cmp)
            .ok_or_else(|| invalid(format!("cmp must be one of {}", crate::engine::CmpOp::LIST)))?;
        crate::query::check_value(op, value).map_err(KivaError::Invalid)?;
        Ok(crate::engine::Cond::new(pattern, field, op, value.clone()))
    };
    match j {
        Json::Null => Ok(Vec::new()),
        Json::Object(m) => m.iter().map(|(f, v)| cond(f, if v.is_array() { "in" } else { "=" }, v)).collect(),
        Json::Array(a) => a
            .iter()
            .map(|c| {
                let field = c["field"].as_str().ok_or_else(|| invalid("each filter condition needs a \"field\""))?;
                cond(field, c["cmp"].as_str().unwrap_or("="), &c["value"])
            })
            .collect(),
        _ => Err(invalid("\"filter\" is an object of equalities or an array of conditions")),
    }
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
        // Text, read in the zone of the session when it has no offset; a datetime may also be
        // milliseconds since 1970.
        Type::DateTime => Value::DateTime(match j {
            Json::Number(n) => n.as_i64().and_then(chrono::DateTime::from_timestamp_millis).ok_or_else(mismatch)?,
            _ => {
                crate::time::parse_datetime(j.as_str().ok_or_else(mismatch)?, &crate::time::zone()).map_err(invalid)?
            }
        }),
        Type::Date => Value::Date(crate::time::parse_date(j.as_str().ok_or_else(mismatch)?).map_err(invalid)?),
        Type::Time => Value::Time(crate::time::parse_time(j.as_str().ok_or_else(mismatch)?).map_err(invalid)?),
    })
}

fn ttl_of(expires_at: i64) -> Json {
    if expires_at > 0 {
        json!(secs_left(expires_at))
    } else {
        Json::Null
    }
}

/// The changes of the workspace's journal after `cursor` (`beginning`,
/// `now`, or a cursor; a named consumer's position without one), as JSON.
/// With `wait` (milliseconds, at most 30 000), waits for a change when
/// there is none yet; access is checked again at every look, so a grant
/// revoked meanwhile ends the wait. Shared by `GET /v1/changes` and the
/// `changes` request.
pub fn read_changes(
    sess: &mut Session,
    cursor: Option<&str>,
    consumer: Option<&str>,
    limit: usize,
    pattern: Option<&str>,
    wait: u64,
) -> Result<Json> {
    use crate::engine::ChangesFrom;
    let consumer = consumer.filter(|c| !c.is_empty());
    let mut from = match (cursor, consumer) {
        (Some("beginning"), _) => ChangesFrom::Beginning,
        (Some("now"), _) => ChangesFrom::Now,
        (Some(c), _) => ChangesFrom::Cursor(c.to_string()),
        (None, Some(_)) => ChangesFrom::Consumer,
        (None, None) => ChangesFrom::Beginning,
    };
    let until = std::time::Instant::now() + std::time::Duration::from_millis(wait.min(30_000));
    loop {
        sess.check_reads_now()?;
        let read = sess.engine.read_changes(from, limit, pattern, consumer)?;
        if !read.changes.is_empty() || read.more || std::time::Instant::now() >= until {
            let list: Vec<Json> =
                read.changes.iter().map(|c| crate::changes::to_json(c, &read.cursor_of(c.seq))).collect();
            return Ok(json!({
                "workspace": sess.ws,
                "cursor": read.cursor,
                "more": read.more,
                "first_seq": read.first_seq,
                "last_seq": read.last_seq,
                "changes": list,
            }));
        }
        // Nothing yet: wait for the journal to move, from where this read ended.
        let seen = read.last_seq;
        from = ChangesFrom::Cursor(read.cursor);
        while sess.engine.last_change() == Some(seen) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// Runs one structured request.
pub fn handle(sess: &mut Session, req: &Json) -> Result<Json> {
    sess.use_zone();
    let op = str_field(req, "op")?;
    match op {
        "whoami" => {
            sess.check(Need::Login)?;
            let role = sess.role().unwrap_or(Role::Viewer);
            Ok(json!({
                "version": VERSION,
                "user": sess.user_name(),
                "role": role.name(),
                "workspace": if sess.ws_priv.is_some() { json!(sess.ws) } else { Json::Null },
                "access": sess.ws_priv.map(|p| p.name()),
                "can_write": sess.writable().is_ok(),
                // The same check as the command, so that a tool offers it
                // exactly when the server would run it.
                "can_compact": sess.check(Need::Owner).is_ok(),
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
                Some(t) if t > 0 => deadline_in(t),
                Some(_) => 0,
            };
            sess.kv_put(key, value, expires_at, meta.msgpack)?;
            Ok(json!({ "key": key, "type": ty.name() }))
        }
        "add" => {
            // A new key: the prefix followed by an identifier KivaDB makes.
            let prefix = str_field(req, "prefix")?;
            let ty = match req["type"].as_str() {
                Some(t) => Type::from_name(t).ok_or_else(|| invalid("unknown type"))?,
                None if req["value"].is_array() => Type::Array,
                None if req["value"].is_object() => Type::Object,
                None => return Err(invalid("\"type\" is needed for a value that is not an array or an object")),
            };
            let value = value_in(ty, &req["value"])?;
            let expires_at = match req["ttl"].as_i64() {
                Some(t) if t > 0 => deadline_in(t),
                _ => 0,
            };
            let key = sess.new_key(prefix).map_err(KivaError::Invalid)?;
            sess.kv_put(&key, value, expires_at, false)?;
            Ok(json!({ "key": key, "id": &key[prefix.len()..], "type": ty.name() }))
        }
        "changes" => {
            let text = |name: &str| match &req[name] {
                Json::Null => Ok(None),
                Json::String(s) => Ok(Some(s.as_str())),
                _ => Err(invalid(format!("\"{name}\" is a text"))),
            };
            let cursor = match text("cursor")? {
                Some(c) => Some(c),
                None => text("from")?,
            };
            let limit = req["limit"].as_u64().unwrap_or(100) as usize;
            let wait = req["wait"].as_u64().unwrap_or(0);
            read_changes(sess, cursor, text("consumer")?, limit, text("pattern")?, wait)
        }
        "del" => {
            let key = str_field(req, "key")?;
            let deleted = sess.kv_del(key)?;
            if deleted {
                sess.inst.audit.log(sess.user_name(), &format!("del {key}"), true);
            }
            Ok(json!({ "deleted": deleted }))
        }
        "query" | "explain" | "aggregate" => {
            sess.check(Need::Read)?;
            // One pattern, a condition, no join or sorting: see crate::query.
            crate::query::check_request(req).map_err(KivaError::Invalid)?;
            let q = request_query(req, op == "aggregate")?;
            let plan_json = |p: &crate::engine::Plan| json!({ "strategy": p.strategy, "index": p.index, "candidates": p.candidates, "total_keys": p.total_keys, "order": p.order });
            if op == "aggregate" && !req["order_by"].is_null() {
                return Err(invalid(crate::query::Unsupported::Pipeline.message()));
            }
            if op == "explain" {
                let opts = read_opts(req, &q, req["limit"].as_u64().unwrap_or(0) as usize)?;
                // `"analyze": true` runs the query: its rows are counted, not returned.
                let measured = match req["analyze"].as_bool().unwrap_or(false) {
                    true => Some(sess.engine.read_query(&q, &opts, |_, _| Ok(()))?),
                    false => None,
                };
                let plan = match &measured {
                    Some((plan, _)) => plan.clone(),
                    None => sess.engine.explain_read(&q, &opts),
                };
                let suggestion = plan.index.is_none().then(|| {
                    let conds = q.expr.iter().flat_map(|e| e.positive_conditions());
                    let missing = conds
                        .into_iter()
                        .find(|c| crate::engine::Engine::indexable(c) && sess.engine.explain(c).index.is_none());
                    missing.map(|c| format!("create index on {} ({})", c.pattern, c.field))
                });
                let mut out = json!({ "plan": plan_json(&plan), "suggestion": suggestion.flatten() });
                if let Some((_, m)) = &measured {
                    out["analyze"] = measured_json(m);
                }
                return Ok(out);
            }
            if op == "aggregate" {
                use crate::query::{aggregated_json, Aggregation};
                let aggs = Aggregation::functions_from_json(&req["compute"]).map_err(KivaError::Invalid)?;
                let group_by = match &req["group_by"] {
                    Json::Null => None,
                    g => Some(g.as_str().ok_or_else(|| invalid("\"group_by\" is a field name"))?),
                };
                let group_by =
                    group_by.map(|g| g.strip_prefix(&q.pattern).and_then(|f| f.strip_prefix('.')).unwrap_or(g));
                let mut acc = Aggregation::new(aggs, group_by).map_err(KivaError::Invalid)?;
                let limit = req["limit"].as_u64().unwrap_or(0) as usize;
                let plan = sess.engine.scan_query(&q, limit, |_, doc| acc.add(&doc))?;
                let mut out = aggregated_json(&acc.finish().map_err(KivaError::Invalid)?, group_by.is_some());
                out["plan"] = plan_json(&plan);
                return Ok(out);
            }
            let limit = req["limit"].as_u64().unwrap_or(100).clamp(1, 10_000) as usize;
            let opts = read_opts(req, &q, limit)?;
            let mut rows = Vec::new();
            let (plan, _) = sess.engine.read_query(&q, &opts, |k, doc| {
                rows.push(json!({ "key": k, "value": doc }));
                Ok(())
            })?;
            Ok(json!({ "rows": rows, "plan": plan_json(&plan) }))
        }
        "similar" => {
            sess.check(Need::Read)?;
            let target = str_field(req, "target")?;
            let (pattern, field) = crate::exec::split_target(target).map_err(KivaError::Invalid)?;
            if req["vector"].is_string() {
                return Err(invalid(crate::vector::NOT_EMBEDDINGS));
            }
            let vector = crate::vector::from_json(&req["vector"])
                .ok_or_else(|| invalid("\"vector\" must be a non-empty array of numbers"))?;
            let metric = match req["metric"].as_str() {
                Some(m) => {
                    crate::vector::Metric::parse(m).ok_or_else(|| invalid("metric must be cosine, dot or l2"))?
                }
                None => crate::vector::Metric::Cosine,
            };
            let limit = req["limit"].as_u64().unwrap_or(10).clamp(1, 1000) as usize;
            let mut q = crate::engine::VectorQuery::new(&pattern, &field, vector, metric, limit);
            q.ef = req["ef"].as_u64().unwrap_or(0).min(100_000) as usize;
            q.exact = req["exact"].as_bool().unwrap_or(false);
            q.filter = vector_filter(&req["filter"], &pattern)?;
            let (r, cached) = sess.similar_cached(&q)?;
            let rows: Vec<Json> =
                r.rows.iter().map(|(k, score, doc)| json!({ "key": k, "score": score, "value": doc })).collect();
            Ok(json!({
                "results": rows,
                "metric": metric.name(),
                "compared": r.compared,
                "indexed": r.indexed,
                "approximate": r.approximate,
                "filter": r.filter,
                "cached": cached
            }))
        }
        "semantic" => {
            use crate::semantic_cache::{lookup_json, Question, Settings, Stored};
            let s = Settings::of(&sess.inst.cfg());
            let cache = str_field(req, "cache")?;
            let vector = match &req["vector"] {
                Json::Null => None,
                v => {
                    Some(crate::vector::from_json(v).ok_or_else(|| invalid("\"vector\" must be an array of numbers"))?)
                }
            };
            let q = Question { text: req["text"].as_str().map(String::from), vector };
            let ws = sess.ws.clone();
            match req["action"].as_str().unwrap_or("lookup") {
                "lookup" => {
                    sess.check(Need::Read)?;
                    let threshold = req["threshold"].as_f64().map(|t| t as f32).unwrap_or(s.threshold);
                    Ok(lookup_json(&sess.inst.semantic.lookup(&ws, cache, &q, threshold)?))
                }
                "store" => {
                    sess.check(Need::Write)?;
                    let ttl = req["ttl"].as_i64().unwrap_or(s.ttl);
                    let depends = req["depends"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect())
                        .unwrap_or_default();
                    let since = match &req["since"] {
                        Json::Null => None,
                        v => Some(v.as_u64().ok_or_else(|| invalid("\"since\" is the token a lookup answered"))?),
                    };
                    let answer = req["answer"].clone();
                    match sess.inst.semantic.store_since(&ws, cache, &q, answer, ttl, depends, s.max_entries, since)? {
                        Stored::Kept(id) => Ok(json!({ "id": id, "stored": true })),
                        Stored::Stale(why) => Ok(json!({ "id": null, "stored": false, "stale": why })),
                    }
                }
                other => Err(invalid(format!("unknown semantic action '{other}' (lookup or store)"))),
            }
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
                "compact_error": s.compact_error,
                "last_compact": s.last_compact,
                "auto_compact": s.auto_compact,
                "compact_threshold": s.compact_threshold,
                "indexes": s.indexes,
                "vector_indexes": s.vector_indexes,
                "cache_limit_bytes": s.cache_limit,
                "cache_bytes": s.cache_bytes,
                "cache_entries": s.cache_entries,
                "cache_hits": s.cache_hits,
                "cache_misses": s.cache_misses,
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
                        "default_workspace": u.default_workspace,
                        "grants": grants,
                        "empty_password": u.has_empty_password(),
                        "builtin": u.name == auth::DEFAULT_ADMIN,
                    })
                })
                .collect();
            Ok(json!({ "users": users }))
        }
        "workspaces" => {
            sess.check(Need::Login)?;
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
                let project = auth::project_of(&name).map(str::to_string);
                list.push(json!({ "name": name, "project": project, "path": path, "access": access, "current": name == sess.ws }));
            }
            Ok(json!({ "workspaces": list }))
        }
        other => Err(invalid(format!("unknown operation '{other}'"))),
    }
}
