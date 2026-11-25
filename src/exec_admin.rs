// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Administrative commands: users, configuration, workspaces, maintenance,
//! replication, scripting and notifications.

use crate::auth::{self, Priv, Role};
use crate::compress::Compression;
use crate::engine::{IndexKind, VectorIndexKind};
use crate::error::KivaError;
use crate::exec::{key_for_read, uint, usage_line, word, Cur, Out, R};
use crate::lexer::{Kind, Token};
use crate::repl;
use crate::script;
use crate::session::{Need, Session};
use crate::util::{fmt_time, human_size, parse_date};
use crate::value::Value;
use serde_json::json;
use std::time::{Duration, Instant};

impl Session {
    pub(crate) fn dispatch_admin(&mut self, cmd: &str, t: &[Token], out: &mut Out) -> R {
        let a = &t[1..];
        match cmd {
            "auth" | "login" => self.cmd_auth(a, out),
            "whoami" => {
                let role = self.role().map(|r| r.name()).unwrap_or("-");
                if let (None, Some(p)) = (self.ws_priv, &self.project) {
                    out.say(format!("{} ({role}) @ project '{p}', in no collection", self.user_name()));
                } else if self.ws_priv.is_none() {
                    out.say(format!("{} ({role}), in no workspace", self.user_name()));
                } else {
                    out.say(format!("{} ({role}) @ workspace '{}'", self.user_name(), self.ws));
                }
                Ok(())
            }
            "project" => self.cmd_project(a, out),
            "timezone" => self.cmd_timezone(a, out),
            "format" => self.cmd_format(a, out),
            "users" => self.cmd_users(out),
            "create" => self.cmd_create(a, out),
            "tune" => self.cmd_tune(a, out),
            "drop" => self.cmd_drop(a, out),
            "grant" => self.cmd_grant(a, out, true),
            "revoke" => self.cmd_grant(a, out, false),
            "modify" => self.cmd_modify(a, out),
            "audit" => self.cmd_audit(a, out),
            "config" => self.cmd_config(a, out),
            "workspace" => self.cmd_workspace(a, out, "workspace"),
            "collection" | "collections" => self.cmd_workspace(a, out, "collection"),
            "use" => self.cmd_use(a, out),
            "hint" => self.cmd_hint(a, out),
            "encryption" => self.cmd_encryption(a, out),
            "semantic" => self.cmd_semantic(a, out),
            "memory" => self.cmd_memory(a, out),
            "backup" => self.cmd_backup(a, out),
            "export" => self.cmd_export(a, out),
            "import" => self.cmd_import(a, out),
            "autocompact" => self.cmd_autocompact(a, out),
            "check" => self.cmd_check(a, out),
            "schema" => self.cmd_schema(a, out),
            "changes" => self.cmd_changes(a, out),
            "schemas" => self.cmd_schema(a, out),
            "compact" => {
                if !a.is_empty() {
                    return Err("'compact' command does not accept arguments.".into());
                }
                let r = self.engine.compact()?;
                if r.aborted {
                    out.say("Compaction skipped: the database was reset while it was running.");
                } else {
                    out.say(format!(
                        "Database storage compacted successfully ({} -> {}, {} records).",
                        human_size(r.before_bytes),
                        human_size(r.after_bytes),
                        r.records
                    ));
                    if r.dropped_corrupted > 0 {
                        out.err(format!("Warning: {} corrupted record(s) were dropped.", r.dropped_corrupted));
                    }
                }
                Ok(())
            }
            "compress" => self.cmd_compress(a, out),
            "replica" => self.cmd_replica(a, out),
            "wait" => self.cmd_wait(a, out),
            "failover" => self.cmd_failover(a, out),
            "eval" => self.cmd_eval(a, out),
            "script" => self.cmd_script(a, out),
            "triggers" => {
                let list = self.inst.triggers();
                if list.is_empty() {
                    out.say("No triggers defined.");
                }
                for tr in list {
                    out.say(format!("{:<16} on {:<6} \"{}\" run \"{}\"", tr.name, tr.event, tr.pattern, tr.script));
                }
                Ok(())
            }
            "indexes" => {
                let list = self.engine.list_indexes();
                let vectors = self.engine.list_vector_indexes();
                if list.is_empty() && vectors.is_empty() {
                    out.say("No secondary indexes in this workspace.");
                }
                let has_vectors = !vectors.is_empty();
                for i in list {
                    let mut how = String::new();
                    if i.unique {
                        how.push_str("unique, ");
                    }
                    if i.ignore_case {
                        how.push_str("ignore case, ");
                    }
                    if i.kind == IndexKind::Hash {
                        how.push_str("hash, ");
                    }
                    out.say(format!(
                        "{} ({}) - {how}{} documents indexed, {} distinct values",
                        i.pattern, i.field, i.documents, i.values
                    ));
                }
                for v in vectors {
                    let dims = v.dims.map(|d| format!("{d} dimensions")).unwrap_or_else(|| "empty".into());
                    let mut extra =
                        if v.rejected > 0 { format!(", {} skipped (other length)", v.rejected) } else { String::new() };
                    if v.not_vectors > 0 {
                        extra.push_str(&format!(
                            ", {} skipped (not an array of numbers: embeddings are computed by the application)",
                            v.not_vectors
                        ));
                    }
                    if v.tombstones > 0 {
                        extra.push_str(&format!(", {} replaced or deleted awaiting a rebuild", v.tombstones));
                    }
                    if let Some(a) = &v.auto_tune {
                        extra.push_str(&format!(
                            ", ef_search tuned again as it grows (recall {:.0} %",
                            a.recall * 100.0
                        ));
                        extra.push_str(&match a.tuned_at {
                            0 => ", once it has enough vectors)".to_string(),
                            n => format!(", last at {n} vectors)"),
                        });
                    }
                    let kind = match v.kind {
                        VectorIndexKind::Exact => "vector index".to_string(),
                        VectorIndexKind::Hnsw(p) => format!(
                            "HNSW vector index ({}, m {}, ef {}, ef_search {}{})",
                            p.metric.name(),
                            p.m,
                            p.ef_construction,
                            v.ef_search,
                            if p.quant == crate::hnsw::Quant::None {
                                String::new()
                            } else {
                                format!(", {}", p.quant.name())
                            }
                        ),
                    };
                    out.say(format!(
                        "{} ({}) - {kind}, {} vectors, {dims}, {} in memory{extra}",
                        v.pattern,
                        v.field,
                        v.vectors,
                        human_size(v.memory_bytes as u64)
                    ));
                }
                if has_vectors {
                    let m = self.engine.memory();
                    out.say(match m.vector_limit {
                        0 => {
                            format!("Vector memory: {} (no bound: vector.max_ram_mb = 0)", human_size(m.vectors as u64))
                        }
                        b => format!(
                            "Vector memory: {} of {} ({:.1}%, vector.max_ram_mb)",
                            human_size(m.vectors as u64),
                            human_size(b as u64),
                            m.vectors as f64 * 100.0 / b as f64
                        ),
                    });
                }
                Ok(())
            }
            "bench" => self.cmd_bench(a, out),
            "subscribe" => self.cmd_subscribe(a, out),
            "unsubscribe" => {
                let pattern = a.first().map(|t| t.text.clone());
                let n = self.inst.pubsub.unsubscribe(self.id, pattern.as_deref());
                self.subs.retain(|p| pattern.as_ref().is_some_and(|x| x != p));
                out.say(format!("Unsubscribed ({n} subscription(s) removed)."));
                Ok(())
            }
            _ => {
                out.say(format!("Unknown command: '{}'. Try 'help'.", t[0].text));
                Ok(())
            }
        }
    }

    fn cmd_auth(&mut self, a: &[Token], out: &mut Out) -> R {
        if a.is_empty() || a.len() > 2 {
            return Err("Usage: auth <user> <password>".into());
        }
        let name = word(a.first(), "a user name")?;
        let password = if a.len() == 2 { word(a.get(1), "a password")? } else { String::new() };
        let user = self.login(&name, &password)?;
        out.say(format!("Welcome {} ({})", user.name, user.role.name()));
        if user.has_empty_password() {
            out.err(format!(
                "[WARN] {0} password is empty. Please run: modify user \"{0}\" password \"...\"",
                user.name
            ));
        }
        Ok(())
    }

    fn cmd_users(&mut self, out: &mut Out) -> R {
        out.say(format!(" {:<20} | {:<7} | {:<19} | GRANTS", "USER", "ROLE", "CREATED"));
        out.say("-".repeat(78));
        for u in auth::list_users(&self.inst.system)? {
            let mut grants = if u.role == Role::Admin {
                "all (admin)".to_string()
            } else if u.grants.is_empty() {
                "none".to_string()
            } else {
                let g: Vec<String> = u
                    .grants
                    .iter()
                    .map(|(w, p)| format!("{}={}", if w == "*" { "*.*" } else { w }, p.name()))
                    .collect();
                g.join(", ")
            };
            if let Some(w) = &u.default_workspace {
                grants.push_str(&format!(" (default: {w})"));
            }
            out.say(format!(
                " {:<20} | {:<7} | {:<19} | {grants}",
                u.name,
                u.role.name(),
                crate::time::show_secs(u.created)
            ));
        }
        Ok(())
    }

    fn cmd_create(&mut self, a: &[Token], out: &mut Out) -> R {
        let mut c = Cur::new(a);
        if c.eat("user") {
            const USAGE: &str = "create user \"<name>\" identified by \"<password>\" [role admin|editor|viewer] \
                                 [grant read|write on <scope>[, <scope>...]] [default workspace <name>]";
            let name = word(c.next(), "a user name")?;
            if !(c.eat("identified") && c.eat("by")) {
                return Err(format!("Usage: {USAGE}"));
            }
            let password = word(c.next(), "a password")?;
            let mut role = Role::Editor;
            let mut grants: Vec<(Vec<String>, Priv)> = Vec::new();
            let mut default_ws = None;
            while c.peek().is_some() {
                if c.eat("role") {
                    role = Role::from_name(&word(c.next(), "a role")?).ok_or("role must be admin, editor or viewer")?;
                } else if c.eat("grant") {
                    let p = match word(c.next(), "read or write")?.to_ascii_lowercase().as_str() {
                        "read" => Priv::Read,
                        "write" => Priv::Write,
                        _ => return Err(format!("a grant is read or write.\nUsage: {USAGE}")),
                    };
                    if !c.eat("on") {
                        return Err(format!("Usage: {USAGE}"));
                    }
                    grants.push((self.parse_scopes(&mut c, true)?, p));
                } else if c.eat("default") {
                    if !c.eat("workspace") {
                        return Err(format!("Usage: {USAGE}"));
                    }
                    default_ws = Some(self.existing_workspace(c.next())?);
                } else {
                    c.finish(USAGE)?;
                }
            }
            let mut user = auth::create_user(&self.inst.system, &name, &password, role)?;
            for (scopes, p) in &grants {
                for s in scopes {
                    user.grants.insert(s.clone(), *p);
                }
            }
            user.default_workspace = default_ws;
            if !grants.is_empty() || user.default_workspace.is_some() {
                auth::save_user(&self.inst.system, &user)?;
            }
            let reach = if role == Role::Admin {
                "every workspace".to_string()
            } else if user.grants.is_empty() {
                "no workspace yet (give it some with grant)".to_string()
            } else {
                let g: Vec<String> = user
                    .grants
                    .iter()
                    .map(|(s, p)| format!("{} {}", p.name(), if s == "*" { "*.*" } else { s }))
                    .collect();
                g.join(", ")
            };
            out.say(format!("User '{name}' created with role [{}]: {reach}.", role.name()));
            Ok(())
        } else if c.eat("vector") {
            const USAGE: &str =
                "create vector index on <pattern> (<field>) [using hnsw [metric cosine|dot|l2] [m N] [ef N] [ef_search N] [quantize sq8|none]]";
            if !c.eat("index") {
                return Err(format!("Usage: {USAGE}"));
            }
            let (pattern, field) = parse_index_target(&mut c, USAGE)?;
            if field.contains(", ") {
                return Err(format!("a vector index covers one field.\nUsage: {USAGE}"));
            }
            let mut ef_search = 0;
            let kind = if c.eat("using") {
                if !c.eat("hnsw") {
                    return Err(format!("the only approximate index is hnsw.\nUsage: {USAGE}"));
                }
                let mut p = crate::hnsw::Params::default();
                loop {
                    if c.eat("ef_search") {
                        ef_search = parse_ef_search(c.next())?;
                        continue;
                    }
                    if c.eat("quantize") {
                        let q = word(c.next(), "sq8 or none")?;
                        p.quant = crate::hnsw::Quant::parse(&q).ok_or("quantize takes sq8 or none")?;
                    } else if c.eat("metric") {
                        let m = word(c.next(), "a metric")?;
                        p.metric = crate::vector::Metric::parse(&m).ok_or("metric must be cosine, dot or l2")?;
                    } else if c.eat("m") {
                        p.m = uint(c.next(), "m")? as usize;
                        if !(2..=100).contains(&p.m) {
                            return Err("m must be between 2 and 100".into());
                        }
                    } else if c.eat("ef") {
                        p.ef_construction = uint(c.next(), "ef")? as usize;
                        if !(10..=2000).contains(&p.ef_construction) {
                            return Err("ef must be between 10 and 2000".into());
                        }
                    } else {
                        break;
                    }
                }
                VectorIndexKind::Hnsw(p)
            } else {
                VectorIndexKind::Exact
            };
            c.finish(USAGE)?;
            let n = self.engine.create_vector_index_with(&pattern, &field, kind)?;
            if ef_search > 0 {
                self.engine.set_vector_ef_search(&pattern, &field, ef_search)?;
            }
            match kind {
                VectorIndexKind::Exact => {
                    out.say(format!("Vector index created on {pattern} ({field}): {n} vectors in memory."))
                }
                VectorIndexKind::Hnsw(p) => out.say(format!(
                    "HNSW vector index created on {pattern} ({field}): {n} vectors in a graph ({}, m {}, ef {}{}).",
                    p.metric.name(),
                    p.m,
                    p.ef_construction,
                    match p.quant {
                        crate::hnsw::Quant::None => String::new(),
                        q => format!(", {}: one byte per number, re-ranked with the documents' vectors", q.name()),
                    }
                )),
            }
            Ok(())
        } else if c.peek().is_some_and(|t| t.is_kw("index") || t.is_kw("unique")) {
            const USAGE: &str =
                "create [unique] index on <pattern> (<field>[, <field>...]) [using btree|hash] [ignore case]";
            let unique = c.eat("unique");
            if !c.eat("index") {
                return Err(format!("Usage: {USAGE}"));
            }
            let (pattern, field) = parse_index_target(&mut c, USAGE)?;
            let kind = if c.eat("using") {
                IndexKind::parse(&word(c.next(), "an index kind")?).ok_or("the index kind must be btree or hash")?
            } else {
                IndexKind::BTree
            };
            let ignore_case = c.eat("ignore");
            if ignore_case && !c.eat("case") {
                return Err(format!("Usage: {USAGE}"));
            }
            c.finish(USAGE)?;
            let fields: Vec<&str> = field.split(", ").collect();
            let n = self.engine.create_unique_index(&pattern, &fields, kind, unique, ignore_case)?;
            let how = if kind == IndexKind::Hash { " using hash" } else { "" };
            let case = if ignore_case { " ignore case" } else { "" };
            let what = if unique { "Unique index" } else { "Index" };
            out.say(format!("{what} created on {pattern} ({field}){how}{case}: {n} documents indexed."));
            Ok(())
        } else if c.eat("trigger") {
            const USAGE: &str = "create trigger [name] on set|update|del \"<pattern>\" run \"<script>\"";
            let name = if c.peek().is_some_and(|t| t.is_kw("on")) {
                format!("t{}", self.inst.next_id())
            } else {
                word(c.next(), "a trigger name")?
            };
            if !c.eat("on") {
                return Err(format!("Usage: {USAGE}"));
            }
            let event = word(c.next(), "an event")?.to_ascii_lowercase();
            if !matches!(event.as_str(), "set" | "update" | "del") {
                return Err("trigger events are: set (key created), update (key overwritten), del".into());
            }
            let pattern = word(c.next(), "a key pattern")?;
            if !c.eat("run") {
                return Err(format!("Usage: {USAGE}"));
            }
            let script_name = word(c.next(), "a script name")?;
            let script_name = script_name.strip_suffix(".lua").unwrap_or(&script_name).to_string();
            c.finish(USAGE)?;
            if !script::valid_script_name(&name) {
                return Err("trigger names may only contain letters, digits, '_', '-' and '.'".into());
            }
            if script::load_source(self, &script_name)?.is_none() {
                return Err(format!("script '{script_name}' does not exist. Load it first with: script load \"<file>\" as \"{script_name}\""));
            }
            let def = json!({ "event": event, "pattern": pattern, "script": script_name });
            self.inst.system.put(&format!("trigger:{name}"), &Value::Doc(def), 0, false)?;
            self.inst.reload_triggers();
            out.say(format!("Trigger '{name}' created: on {event} \"{pattern}\" run \"{script_name}\"."));
            Ok(())
        } else {
            Err("Usage: create user ... | create [unique] index on <pattern> (<field>) | create trigger ...".into())
        }
    }

    /// `tune vector index on <pattern> (<field>) [recall R] [limit K] [sample N]`
    /// measures the HNSW index and keeps the smallest `ef_search` that finds
    /// that share of the true closest vectors; with `auto`, it is measured
    /// again each time the index doubles or halves. `... ef_search N |
    /// default` sets it by hand.
    fn cmd_tune(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "tune vector index on <pattern> (<field>) [recall 0.95] [limit 10] [sample 100] [queries \"<pattern>\"] [auto]
       tune vector index on <pattern> (<field>) auto off
       tune vector index on <pattern> (<field>) ef_search <N> | default";
        let mut c = Cur::new(a);
        if !(c.eat("vector") && c.eat("index")) {
            return Err(format!("Usage: {USAGE}"));
        }
        let (pattern, field) = parse_index_target(&mut c, USAGE)?;
        if c.eat("ef_search") {
            let ef = parse_ef_search(c.next())?;
            c.finish(USAGE)?;
            self.engine.set_vector_ef_search(&pattern, &field, ef)?;
            let shown =
                if ef == 0 { format!("the default, {}", crate::hnsw::DEFAULT_EF_SEARCH) } else { ef.to_string() };
            out.say(format!("Searches on {pattern} ({field}) now keep {shown} candidates (ef_search)."));
            return Ok(());
        }
        if matches!(c.rest(), [a, o] if a.is_kw("auto") && o.is_kw("off")) {
            self.engine.set_vector_auto_tune(&pattern, &field, None)?;
            out.say(format!(
                "{pattern} ({field}) keeps its ef_search as it grows; 'tune vector index ... auto' measures it again."
            ));
            return Ok(());
        }
        let (mut recall, mut k, mut sample) = (0.95, 10, 100);
        let (mut queries, mut auto) = (None, false);
        loop {
            if c.eat("auto") {
                auto = true;
                continue;
            }
            if c.eat("queries") {
                queries = Some(word(c.next(), "the key pattern of documents holding real queries")?);
                continue;
            }
            if c.eat("recall") {
                let t = c.next().map(|t| t.text.clone()).unwrap_or_default();
                recall = t
                    .parse::<f64>()
                    .ok()
                    .filter(|r| *r > 0.0 && *r <= 1.0)
                    .ok_or("recall is a share of the true closest vectors, above 0 and at most 1 (0.95 = 95 %)")?;
            } else if c.eat("limit") {
                k = uint(c.next(), "limit")? as usize;
                if !(1..=100).contains(&k) {
                    return Err("limit must be between 1 and 100".into());
                }
            } else if c.eat("sample") {
                sample = uint(c.next(), "sample")? as usize;
                if !(1..=1000).contains(&sample) {
                    return Err("sample must be between 1 and 1000".into());
                }
            } else {
                break;
            }
        }
        c.finish(USAGE)?;
        let keep = |tuned_at| crate::engine::AutoTune { recall, limit: k, sample, queries: queries.clone(), tuned_at };
        let r = match self.engine.tune_vector_index(&pattern, &field, recall, k, sample, queries.as_deref()) {
            Ok(r) => r,
            // An index still filling up is tuned as soon as it can be.
            Err(_) if auto && self.vector_count(&pattern, &field).is_some_and(|n| n <= k) => {
                self.engine.set_vector_auto_tune(&pattern, &field, Some(keep(0)))?;
                out.say(format!(
                    "{pattern} ({field}) has {k} vectors or fewer: it will be tuned for a recall of {:.0} % once it \
                     has more, then each time its number of vectors doubles or halves.",
                    recall * 100.0
                ));
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        if auto {
            self.engine.set_vector_auto_tune(&pattern, &field, Some(keep(r.vectors)))?;
        }
        let source = match &queries {
            Some(q) => format!("read from {q}"),
            None => "taken from its vectors, each searched without its own node".to_string(),
        };
        out.say(format!(
            "Tuning {pattern} ({field}), {} vectors: {} queries {source}, {k} closest each",
            r.vectors, r.queries
        ));
        out.say(format!("  exact search       : {:.0} µs per search, every vector compared", r.exact_micros));
        for s in &r.steps {
            out.say(format!(
                "  ef_search {:<8} : recall {:>5.1} %, {} vectors compared, {:.0} µs per search",
                s.ef,
                s.recall * 100.0,
                s.compared,
                s.micros
            ));
        }
        if r.reached {
            out.say(format!(
                "Kept ef_search {} for {pattern} ({field}): the smallest that finds {:.0} % of the true closest vectors.",
                r.chosen,
                recall * 100.0
            ));
        } else {
            out.say(format!(
                "Kept ef_search {}, the largest tried: no value reached {:.0} %. A graph with more links \
                 (create vector index ... using hnsw m 32 ef 400) finds more; 'get similar ... exact' finds them all.",
                r.chosen,
                recall * 100.0
            ));
        }
        if auto {
            out.say(format!(
                "It is measured again in the background when the index reaches {} or falls to {} vectors.",
                r.vectors * 2,
                r.vectors / 2
            ));
        }
        Ok(())
    }

    /// Vectors in the index on `pattern (field)`.
    fn vector_count(&self, pattern: &str, field: &str) -> Option<usize> {
        self.engine
            .list_vector_indexes()
            .into_iter()
            .find(|v| v.pattern == pattern && v.field == field)
            .map(|v| v.vectors)
    }

    fn cmd_drop(&mut self, a: &[Token], out: &mut Out) -> R {
        let mut c = Cur::new(a);
        if c.eat("user") {
            let name = word(c.next(), "a user name")?;
            c.finish("drop user \"<name>\"")?;
            if name == auth::DEFAULT_ADMIN {
                return Err("the built-in kiva account cannot be dropped.".into());
            }
            if name == self.user_name() {
                return Err("you cannot drop your own account.".into());
            }
            if !auth::delete_user(&self.inst.system, &name)? {
                return Err(format!("user '{name}' does not exist."));
            }
            out.say(format!("User '{name}' dropped."));
            Ok(())
        } else if c.eat("vector") {
            if !c.eat("index") {
                return Err("Usage: drop vector index on <pattern> (<field>)".into());
            }
            let (pattern, field) = parse_index_spec(&mut c, "drop vector index on <pattern> (<field>)")?;
            if !self.engine.drop_vector_index(&pattern, &field)? {
                return Err(format!("no vector index on {pattern} ({field})."));
            }
            out.say(format!("Vector index on {pattern} ({field}) dropped."));
            Ok(())
        } else if c.eat("index") {
            let (pattern, field) = parse_index_spec(&mut c, "drop index on <pattern> (<field>[, <field>...])")?;
            if !self.engine.drop_index(&pattern, &field)? {
                return Err(format!("no index on {pattern} ({field})."));
            }
            out.say(format!("Index on {pattern} ({field}) dropped."));
            Ok(())
        } else if c.eat("trigger") {
            let name = word(c.next(), "a trigger name")?;
            c.finish("drop trigger <name>")?;
            if !self.inst.system.delete(&format!("trigger:{name}"))? {
                return Err(format!("trigger '{name}' does not exist."));
            }
            self.inst.reload_triggers();
            out.say(format!("Trigger '{name}' dropped."));
            Ok(())
        } else {
            Err("Usage: drop user \"<name>\" | drop index on <pattern> (<field>) | drop trigger <name>".into())
        }
    }

    /// After `on`: one or more scopes separated by commas (`travel.*,
    /// school.places`), or the older forms `all` and `workspace "<w>"`. Each
    /// workspace named must exist, and each project named too.
    fn parse_scopes(&self, c: &mut Cur, must_exist: bool) -> Result<Vec<String>, String> {
        let mut scopes = Vec::new();
        loop {
            let scope = if c.eat("workspace") {
                auth::normalize_scope(&word(c.next(), "a workspace name")?)?
            } else if c.eat("project") {
                let p = word(c.next(), "a project name")?;
                auth::normalize_scope(&format!("{p}.*"))?
            } else {
                auth::normalize_scope(&word(c.next(), "a scope (travel.places, travel.*, *.*)")?)?
            };
            // A revoke may name what no longer exists.
            if must_exist {
                if !scope.contains('*') && self.inst.cfg().workspace_dir(&scope).is_none() {
                    return Err(format!("workspace '{scope}' does not exist."));
                }
                if let Some(p) = auth::project_of(&scope).filter(|p| *p != "*") {
                    if !self.inst.project_exists(p) {
                        return Err(format!("project '{p}' does not exist (create it with: project create {p})."));
                    }
                }
            }
            scopes.push(scope);
            if !c.peek().is_some_and(|t| t.kind == crate::lexer::Kind::Comma) {
                return Ok(scopes);
            }
            c.next();
        }
    }

    /// The name of a registered workspace, or an error naming it.
    fn existing_workspace(&self, t: Option<&Token>) -> Result<String, String> {
        let w = word(t, "a workspace name")?;
        if self.inst.cfg().workspace_dir(&w).is_none() {
            return Err(format!("workspace '{w}' does not exist."));
        }
        Ok(w)
    }

    /// `grant <priv> on <scope>[, <scope>...] to "<user>"` and the matching `revoke ... from`.
    fn cmd_grant(&mut self, a: &[Token], out: &mut Out, grant: bool) -> R {
        let usage = if grant {
            "grant read|write|admin on <scope>[, <scope>...] to \"<user>\"  (scopes: travel.places, travel.*, *.places, *.*)"
        } else {
            "revoke read|write|admin [on <scope>[, <scope>...]] from \"<user>\""
        };
        let mut c = Cur::new(a);
        c.eat("privileges");
        let privilege = word(c.next(), "a privilege").map_err(|e| format!("{e}\nUsage: {usage}"))?.to_ascii_lowercase();
        // None = no scope given.
        let scopes = if c.eat("on") { Some(self.parse_scopes(&mut c, grant)?) } else { None };
        if !c.eat(if grant { "to" } else { "from" }) {
            return Err(format!("Usage: {usage}"));
        }
        let name = word(c.next(), "a user name")?;
        c.finish(usage)?;
        if grant && scopes.is_none() && privilege != "admin" {
            return Err(format!("a grant needs a scope.\nUsage: {usage}"));
        }
        let mut user = auth::get_user(&self.inst.system, &name)?.ok_or(format!("user '{name}' does not exist."))?;
        if name == auth::DEFAULT_ADMIN {
            return Err("the privileges of the built-in kiva account cannot be changed.".into());
        }
        let where_ = match scopes.as_deref() {
            None => "all workspaces".to_string(),
            Some(s) if s == ["*"] => "all workspaces".to_string(),
            Some(s) => s.join(", "),
        };
        match (privilege.as_str(), grant) {
            ("admin", true) => {
                user.role = Role::Admin;
                out.say(format!("Granted: '{name}' is now an admin."));
            }
            ("admin", false) => {
                if user.role == Role::Admin {
                    user.role = Role::Editor;
                }
                out.say(format!("Revoked: '{name}' is no longer an admin."));
            }
            ("read" | "write", true) => {
                let p = if privilege == "write" { Priv::Write } else { Priv::Read };
                for s in scopes.iter().flatten() {
                    user.grants.insert(s.clone(), p);
                }
                // Write access is meaningless for a read-only role.
                if p == Priv::Write && user.role == Role::Viewer {
                    user.role = Role::Editor;
                }
                out.say(format!("Granted: {privilege} on {where_} to '{name}'."));
            }
            ("write", false) => {
                match &scopes {
                    Some(scopes) => {
                        for s in scopes {
                            if let Some(p) = user.grants.get_mut(s) {
                                *p = Priv::Read;
                            }
                        }
                    }
                    None => {
                        // No scope: the user loses the ability to write anywhere.
                        user.grants.values_mut().for_each(|p| *p = Priv::Read);
                        if user.role != Role::Viewer {
                            user.role = Role::Viewer;
                        }
                    }
                }
                out.say(format!("Revoked: write on {where_} from '{name}'."));
            }
            ("read", false) => {
                match &scopes {
                    Some(scopes) => {
                        for s in scopes {
                            user.grants.remove(s);
                        }
                    }
                    None => user.grants.clear(),
                }
                out.say(format!("Revoked: read on {where_} from '{name}'."));
            }
            _ => return Err(format!("unknown privilege '{privilege}'.\nUsage: {usage}")),
        }
        auth::save_user(&self.inst.system, &user)?;
        Ok(())
    }

    fn cmd_modify(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "Usage: modify user \"<name>\" password \"<new>\" | modify user \"<name>\" role \"<role>\" | \
                             modify user \"<name>\" default workspace <name>|none | modify user \"<name>\" timezone <zone>|none \
                             | modify user \"<name>\" date|time|datetime format \"<format>\"|iso|none";
        let mut c = Cur::new(a);
        if !c.eat("user") {
            return Err(USAGE.into());
        }
        let name = word(c.next(), "a user name")?;
        let is_admin = self.role() == Some(Role::Admin);
        let mut user = auth::get_user(&self.inst.system, &name)?.ok_or(format!("user '{name}' does not exist."))?;
        if c.eat("password") {
            let password = word(c.next(), "a password")?;
            c.finish(USAGE)?;
            if !is_admin && name != self.user_name() {
                return Err("permission denied: you can only change your own password.".into());
            }
            user.set_password(&password)?;
            auth::save_user(&self.inst.system, &user)?;
            out.say(format!("Password updated for '{name}'."));
            if password.is_empty() {
                out.err("[WARN] the new password is empty.");
            }
        } else if c.eat("role") {
            let role = Role::from_name(&word(c.next(), "a role")?).ok_or("role must be admin, editor or viewer")?;
            c.finish(USAGE)?;
            self.check(Need::Admin)?;
            if name == auth::DEFAULT_ADMIN {
                return Err("the role of the built-in kiva account cannot be changed.".into());
            }
            user.role = role;
            auth::save_user(&self.inst.system, &user)?;
            out.say(format!("Role of '{name}' set to [{}].", role.name()));
        } else if c.eat("timezone") {
            let zone = if c.peek().is_some_and(|t| t.is_kw("none")) {
                c.next();
                None
            } else {
                let t = word(c.next(), "a time zone (Africa/Kinshasa, UTC, +01:00)")?;
                Some(crate::time::Zone::parse(&t)?.name())
            };
            c.finish(USAGE)?;
            if !is_admin && name != self.user_name() {
                return Err("permission denied: you can only change your own time zone.".into());
            }
            out.say(match &zone {
                Some(z) => format!("Time zone of '{name}' set to {z}: it applies from the next login."),
                None => format!("'{name}' now uses the server's time zone, from the next login."),
            });
            user.timezone = zone;
            auth::save_user(&self.inst.system, &user)?;
        } else if let Some(kind) = c.peek().and_then(|t| crate::time::FormatKind::parse(&t.text)) {
            c.next();
            if !c.eat("format") {
                return Err(USAGE.into());
            }
            let t = word(c.next(), "a format such as \"DD-MM-YYYY\", iso or none")?;
            let format = if t.eq_ignore_ascii_case("none") {
                None
            } else {
                crate::time::parse_format(kind, &t)?;
                Some(if t.eq_ignore_ascii_case("iso") { "iso".to_string() } else { t })
            };
            c.finish(USAGE)?;
            if !is_admin && name != self.user_name() {
                return Err("permission denied: you can only change your own formats.".into());
            }
            let k = kind.name();
            out.say(match &format {
                Some(f) => format!("The {k} format of '{name}' set to {f}: it applies from the next login."),
                None => format!("'{name}' now uses the server's {k} format, from the next login."),
            });
            match format {
                Some(f) => user.formats.insert(k.to_string(), f),
                None => user.formats.remove(k),
            };
            auth::save_user(&self.inst.system, &user)?;
        } else if c.eat("default") {
            if !c.eat("workspace") {
                return Err(USAGE.into());
            }
            self.check(Need::Admin)?;
            let w = if c.peek().is_some_and(|t| t.is_kw("none")) {
                c.next();
                None
            } else {
                Some(self.existing_workspace(c.next())?)
            };
            c.finish(USAGE)?;
            out.say(match &w {
                Some(w) => format!("Default workspace of '{name}' set to '{w}'."),
                None => format!("'{name}' has no default workspace any more."),
            });
            user.default_workspace = w;
            auth::save_user(&self.inst.system, &user)?;
        } else {
            return Err(USAGE.into());
        }
        self.inst.audit.log(self.user_name(), &format!("modify user \"{name}\""), true);
        Ok(())
    }

    fn cmd_audit(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: audit last <N> | audit since \"<YYYY-MM-DD>\" | audit failed-logins";
        let lines = self.inst.audit.lines();
        let mut c = Cur::new(a);
        let selected: Vec<&String> = if c.eat("last") {
            let n = uint(c.next(), "N")? as usize;
            lines.iter().skip(lines.len().saturating_sub(n)).collect()
        } else if c.eat("since") {
            let date = word(c.next(), "a date")?;
            if parse_date(&date).is_none() {
                return Err("dates are written YYYY-MM-DD or \"YYYY-MM-DD HH:MM:SS\".".into());
            }
            lines.iter().filter(|l| l.get(1..20).is_some_and(|ts| ts >= date.as_str())).collect()
        } else if c.eat("failed-logins") {
            lines.iter().filter(|l| l.contains("action=\"login") && l.ends_with("FAILURE")).collect()
        } else {
            return Err(USAGE.into());
        };
        if selected.is_empty() {
            out.say("No audit entries.");
        }
        for l in selected {
            out.say(l.as_str());
        }
        Ok(())
    }

    fn cmd_config(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: config list | config get <key> | config set <key> <value>";
        let mask = |k: &str, v: String| if k.contains("password") && !v.is_empty() { "***".to_string() } else { v };
        let mut c = Cur::new(a);
        if c.eat("list") {
            let cfg = self.inst.cfg();
            out.say(format!("Configuration file: {}", cfg.path().display()));
            for (k, v) in cfg.list() {
                out.say(format!("  {:<32} = {}", k, mask(&k, v)));
            }
            Ok(())
        } else if c.eat("get") {
            let name = word(c.next(), "a configuration key")?;
            let cfg = self.inst.cfg();
            let (s, k) = cfg.resolve(&name)?;
            let v = if s == "workspaces" {
                cfg.workspace_dir(&k).map(|p| p.display().to_string()).unwrap_or_default()
            } else {
                cfg.get(&s, &k)
            };
            out.say(format!("{s}.{k} = {}", mask(&k, v)));
            Ok(())
        } else if c.eat("set") {
            let name = word(c.next(), "a configuration key")?;
            let value = word(c.next(), "a value")?;
            c.finish(USAGE)?;
            let (s, k) = self.inst.cfg().resolve(&name)?;
            if s == "workspaces" || (s == "storage" && k == "default_workspace") {
                self.inst.guard_legacy_catalog_mutation()?;
            }
            if s == "storage" && k == "default_workspace" {
                std::fs::create_dir_all(&value).map_err(|e| format!("cannot use '{value}': {e}"))?;
            }
            self.inst.cfg_mut().set(&s, &k, &value)?;
            out.say(format!("OK: {s}.{k} = {}", mask(&k, value)));
            match (s.as_str(), k.as_str()) {
                (
                    "storage",
                    "auto_compact" | "compact_threshold" | "compression" | "cache_values" | "cache_size_mb"
                    | "max_memory_mb" | "sync_writes",
                )
                | ("vector", "max_ram_mb") => self.inst.apply_storage_config(),
                // Read again at every use: nothing to restart.
                ("security", _)
                | ("http", "rate_limit" | "cors" | "stream")
                | ("scripting", _)
                | ("tls", "required")
                | ("server", "max_clients")
                | ("resp", "max_collection_members")
                | ("semantic", _)
                | ("failover", _)
                | ("time", _) => {}
                ("replication", "sync_mode" | "min_slaves_ack" | "master_user" | "master_password") => {}
                ("replication", _) => {
                    out.say("Note: use 'replica start' or 'replica promote' to change the replication role now.")
                }
                _ => out.say("Note: this setting takes effect the next time KivaDB starts."),
            }
            Ok(())
        } else {
            Err(USAGE.into())
        }
    }

    /// `workspace ...` and `collection ...`: the same commands, `noun` naming
    /// the thing in what they answer. `collection` is the name since rc.5;
    /// `workspace` stays until 3.0, its answers unchanged.
    fn cmd_workspace(&mut self, a: &[Token], out: &mut Out, noun: &str) -> R {
        let usage = if noun == "collection" {
            "Usage: collection list [<project>] | current | switch <name> | create <name> [\"<folder>\"] | \
             create <name> <name> ... | rename|move <name> to <new name> | drop|remove <name>"
                .to_string()
        } else {
            format!(
                "Usage: {noun} list [<project>] | current | switch <name> | create <name> [path] | \
                 rename|move <name> to <new name> | remove <name>"
            )
        };
        let usage = usage.as_str();
        let title = if noun == "collection" { "Collection" } else { "Workspace" };
        let a_name = format!("a {noun} name");
        let mut c = Cur::new(a);
        if c.eat("list") || (noun == "collection" && c.peek().is_none()) {
            let project = c.next().map(|t| t.text.clone()).or_else(|| self.project.clone());
            c.finish(usage)?;
            let me = match (&self.user, self.role()) {
                (Some(u), Some(r)) if r != Role::Admin => auth::get_user(&self.inst.system, &u.name)?,
                _ => None,
            };
            for (name, path) in self.inst.workspaces() {
                if project.is_some() && auth::project_of(&name) != project.as_deref() {
                    continue;
                }
                let access = match &me {
                    Some(u) => match u.access(&name) {
                        Some(p) => p.name(),
                        None => continue,
                    },
                    None => "write",
                };
                let mark = if name == self.ws { "*" } else { " " };
                out.say(format!(" {mark} {name:<20} {access:<5}  {path}"));
            }
            Ok(())
        } else if c.eat("current") {
            c.finish(usage)?;
            if noun == "collection" {
                let name = if self.ws_priv.is_some() { self.ws.as_str() } else { "none" };
                out.say(format!("Current collection: {name}"));
                return Ok(());
            }
            // The answer of 1.x, for the scripts that read it.
            if let (None, Some(p)) = (self.ws_priv, &self.project) {
                out.say(format!("(project '{p}', no {noun})"));
            } else if self.ws_priv.is_none() {
                out.say(format!("(no {noun})"));
            } else {
                out.say(format!("{} ({})", self.ws, self.engine.path().display()));
            }
            Ok(())
        } else if c.eat("switch") || c.eat("use") {
            let name = word(c.next(), &a_name)?;
            c.finish(usage)?;
            self.switch_workspace(&name)?;
            out.say(format!("Switched to {noun} '{name}' ({} keys).", self.engine.len()));
            Ok(())
        } else if noun == "collection" && c.eat("create") {
            // Bare words are names; a quoted text after a single name is its
            // folder, as before (`collection create users "D:\db"`).
            let (mut names, mut folder) = (Vec::new(), None);
            while let Some(t) = c.next() {
                match t.kind {
                    Kind::Bare => names.push(t.text.clone()),
                    Kind::DQuote | Kind::SQuote if folder.is_none() => folder = Some(t.text.clone()),
                    _ => return Err(usage.into()),
                }
            }
            if names.is_empty() {
                // `collection create "places"`: a quoted name, as before.
                names.extend(folder.take());
            }
            if names.is_empty() {
                return Err(usage.into());
            }
            if folder.is_some() && names.len() > 1 {
                return Err(
                    "a folder is given for one collection at a time: collection create <name> \"<folder>\"".into()
                );
            }
            let full = names.iter().map(|n| self.collection_named(n, true)).collect::<Result<Vec<_>, _>>()?;
            if let Some(bad) = full.iter().find(|n| !auth::valid_workspace_name(n)) {
                return Err(format!(
                    "collection name '{bad}': only letters, digits, '_' and '-', with one '.' after a project name                      (travel.places)"
                ));
            }
            let me = self.caller()?;
            for name in &full {
                self.may_create(&me, name, folder.as_deref(), noun)?;
            }
            let dirs = match &folder {
                Some(f) => vec![self.inst.create_workspace(&full[0], Some(f))?],
                None => self.inst.create_collections(&full)?,
            };
            for (name, dir) in full.iter().zip(&dirs) {
                out.say(format!("Collection '{name}' created at {}.", dir.display()));
            }
            Ok(())
        } else if c.eat("create") {
            let name = word(c.next(), &a_name)?;
            let path = c.next().map(|t| t.text.clone());
            c.finish(usage)?;
            let dir = self.create_collection(&name, path.as_deref(), noun)?;
            out.say(format!("{title} '{name}' created at {}.", dir.display()));
            Ok(())
        } else if c.eat("move") || c.eat("rename") {
            let form = format!("{noun} rename|move <name> to <new name>");
            let from = word(c.next(), &a_name)?;
            if !c.eat("to") {
                return Err(usage_line(&form));
            }
            let to = word(c.next(), "the new name")?;
            c.finish(&form)?;
            if from == self.ws {
                let verb =
                    if a.first().is_some_and(|t| t.text.eq_ignore_ascii_case("rename")) { "rename" } else { "move" };
                return Err(format!("you cannot {verb} the {noun} you are in: enter another one first (use default)."));
            }
            if let Some(p) = auth::project_of(&to) {
                if !self.inst.project_exists(p) {
                    return Err(format!("project '{p}' does not exist (create it with: project create {p})."));
                }
            }
            self.inst.rename_workspace(&from, &to)?;
            out.say(format!("{title} '{from}' is now '{to}'. Its files did not move, and its grants followed it."));
            Ok(())
        } else if c.eat("remove") || (noun == "collection" && c.eat("drop")) {
            let name = word(c.next(), &a_name)?;
            c.finish(usage)?;
            // In a project, a short name is a collection of it: never one of
            // the same name elsewhere.
            let name = if noun == "collection" { self.collection_named(&name, false)? } else { name };
            if name == "default" {
                return Err(format!("the default {noun} cannot be removed."));
            }
            if name == self.ws {
                return Err(format!("you cannot remove the {noun} you are in."));
            }
            if !self.inst.remove_workspace(&name)? {
                return Err(format!("{noun} '{name}' does not exist."));
            }
            out.say(format!("{title} '{name}' unregistered. Its data files were left on disk."));
            Ok(())
        } else {
            Err(usage.into())
        }
    }

    /// Creates one collection after the checks of the command: its name, its
    /// project, and who may (an administrator, or the project's managers,
    /// who do not choose the folder).
    fn create_collection(&mut self, name: &str, path: Option<&str>, noun: &str) -> Result<std::path::PathBuf, String> {
        let me = self.caller()?;
        self.may_create(&me, name, path, noun)?;
        Ok(self.inst.create_workspace(name, path)?)
    }

    /// The account of a caller who is not an administrator.
    fn caller(&self) -> Result<Option<auth::User>, String> {
        Ok(match &self.user {
            Some(u) if u.role != Role::Admin => auth::get_user(&self.inst.system, &u.name)?,
            _ => None,
        })
    }

    /// The full name of the collection `name` names in `collection create`
    /// (`create`) or `collection drop`: a name with a dot is full; a short
    /// one is a collection of the current project. Without a project, a
    /// short name is created nowhere, and dropped outside a project.
    fn collection_named(&self, name: &str, create: bool) -> Result<String, String> {
        if name.contains('.') {
            return Ok(name.to_string());
        }
        match self.current_project() {
            Some(p) => Ok(format!("{p}.{name}")),
            None if create => Err(format!(
                "no current project: use <project> first, or give the full name (collection create \
                 <project>.{name}). A collection outside a project is made with: workspace create {name}"
            )),
            None => Ok(name.to_string()),
        }
    }

    /// Whether the caller may create the collection `name` (an
    /// administrator, or the project's managers, who do not choose the
    /// folder).
    fn may_create(&self, me: &Option<auth::User>, name: &str, path: Option<&str>, noun: &str) -> Result<(), String> {
        crate::instance::check_workspace_name(name)?;
        if let Some(p) = auth::project_of(name) {
            if !self.inst.project_exists(p) {
                return Err(format!("project '{p}' does not exist (create it with: project create {p})."));
            }
        }
        if let Some(u) = &me {
            if !u.manages(name) {
                return Err(match auth::project_of(name) {
                    Some(p) => format!(
                        "creating a {noun} in project '{p}' requires the admin role, or write access to \
                         every {noun} of it (grant write on {p}.*)"
                    ),
                    None => format!("creating a {noun} outside a project requires the admin role"),
                });
            }
            if path.is_some() {
                return Err(format!("only an administrator chooses the folder of a {noun}"));
            }
        }
        Ok(())
    }

    /// `export to "<file>" [keys <pattern>]`: the keys of the collection in
    /// use, as NDJSON in extended JSON (`crate::transfer`).
    fn cmd_export(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "export to \"<file.ndjson>\" [keys <pattern>]";
        let mut c = Cur::new(a);
        if !c.eat("to") || c.peek().is_none() {
            return Err(usage_line(USAGE));
        }
        let file = word(c.next(), "a file")?;
        let pattern = if c.eat("keys") { word(c.next(), "a key pattern")? } else { "*".to_string() };
        c.finish(USAGE)?;
        self.check(Need::Read)?;
        let path = std::path::PathBuf::from(&file);
        let n = crate::transfer::export(self, &pattern, &path)?;
        out.say(format!("Exported {n} key(s) of '{}' to {}.", self.ws, path.display()));
        Ok(())
    }

    /// `import from "<file>" [dry run] [replace]`: the lines of an NDJSON
    /// file into the collection in use; a wrong line refuses the whole file.
    fn cmd_import(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "import from \"<file.ndjson>\" [dry run] [replace]";
        let mut c = Cur::new(a);
        if !c.eat("from") || c.peek().is_none() {
            return Err(usage_line(USAGE));
        }
        let file = word(c.next(), "a file")?;
        let mut opts = crate::transfer::ImportOptions::default();
        loop {
            if c.eat("dry") {
                if !c.eat("run") {
                    return Err(usage_line(USAGE));
                }
                opts.dry_run = true;
            } else if c.eat("replace") {
                opts.replace = true;
            } else {
                break;
            }
        }
        c.finish(USAGE)?;
        if self.txn.is_some() {
            return Err("import writes in batches of its own: run it outside a transaction.".into());
        }
        let report = crate::transfer::import(self, std::path::Path::new(&file), opts)?;
        out.say(report.text(&self.ws, opts.dry_run));
        Ok(())
    }

    /// `use <project>.<collection>` (or `use <collection>`) enters a
    /// collection; `use <project>` selects the project, in no collection.
    /// A name without a dot is, in this order: a collection outside a
    /// project, a project, then a collection of the current project
    /// (`use baak` then `use places` enters `baak.places`). The last comes
    /// last so that a name that worked before keeps its meaning.
    /// `use collection <name>` and `use project <name>` say which one is
    /// meant: the collection of the current project, or the project.
    fn cmd_use(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "use <project>.<collection> | <collection> | <project> | collection <name> | project <name>";
        if a.is_empty() {
            return Err(usage_line(USAGE));
        }
        // Two words only: `use collection` alone still enters a collection
        // named `collection`.
        if a.len() == 2 {
            let kind = a[0].text.to_ascii_lowercase();
            if kind == "collection" || kind == "project" {
                let name = word(a.get(1), "a name")?;
                return if kind == "collection" {
                    self.use_collection_here(&name, out)
                } else {
                    self.use_project_named(&name, out)
                };
            }
        }
        let mut c = Cur::new(a);
        let name = word(c.next(), "a collection or a project")?;
        c.finish(USAGE)?;
        if name.contains('.') {
            return self.use_collection_here(&name, out);
        }
        // The same name inside the current project, when it is a collection.
        let in_project = match self.current_project() {
            Some(p) if !name.contains('.') => {
                let q = format!("{p}.{name}");
                self.inst.cfg().workspace_dir(&q).is_some().then_some(q)
            }
            _ => None,
        };
        // Named in a note or a hint only to a user who may enter it, so that
        // `use` tells no one of a collection that `collection list` hides.
        let reachable = in_project.clone().filter(|q| self.may_enter(q));
        // A refusal of the name found first points to the project's collection.
        let hint = |e: KivaError| match (&e, &reachable) {
            (KivaError::Denied(_), Some(q)) => format!("{e}. {q} is in your project: use {q}"),
            _ => e.to_string(),
        };
        if self.inst.cfg().workspace_dir(&name).is_some() {
            self.switch_workspace(&name).map_err(hint)?;
            out.say(format!("Using collection '{name}' ({} keys).", self.engine.len()));
            if let Some(q) = &reachable {
                out.say(format!("{q} also exists: use {q}"));
            }
        } else if self.inst.project_exists(&name) {
            self.use_project(&name).map_err(hint)?;
            self.say_project(&name, out);
            if let Some(q) = &reachable {
                out.say(format!("{q} also exists: use {q}"));
            }
        } else if let Some(q) = in_project {
            self.switch_workspace(&q)?;
            out.say(format!("Using collection '{q}' ({} keys).", self.engine.len()));
        } else {
            let also = match self.current_project() {
                Some(p) if !name.contains('.') => format!(", nor a collection '{p}.{name}' in the project '{p}'"),
                _ => String::new(),
            };
            return Err(format!(
                "there is no collection or project named '{name}'{also} ('project list', 'collection list')."
            ));
        }
        Ok(())
    }

    fn say_project(&self, name: &str, out: &mut Out) {
        let inside = self.inst.workspace_names().into_iter().filter(|w| auth::project_of(w) == Some(name)).count();
        out.say(format!(
            "Using project '{name}' ({inside} collection(s)). Choose one: use {name}.<collection> \
             ('collection list' shows them)."
        ));
    }

    /// Select a collection by its local or qualified name. Existing project
    /// collections are addressed in the default workspace.
    fn use_collection_here(&mut self, name: &str, out: &mut Out) -> R {
        if self.txn.is_some() {
            return Err("cannot switch collection inside a transaction".into());
        }
        let qualified = match name.split('.').collect::<Vec<_>>().as_slice() {
            [collection] => {
                let Some(project) = self.current_project() else {
                    return Err("no current project: use <project> first, or use <project>.<collection>.".into());
                };
                let qualified = format!("{project}.{collection}");
                if self.inst.cfg().workspace_dir(&qualified).is_none() {
                    return Err(format!(
                        "collection '{qualified}' does not exist in project '{project}' ('collection list' shows them)."
                    ));
                }
                qualified
            }
            [project, collection] | ["default", project, collection]
                if auth::valid_name_part(project) && auth::valid_name_part(collection) =>
            {
                format!("{project}.{collection}")
            }
            _ => return Err(format!("invalid collection path '{name}'")),
        };
        self.switch_workspace(&qualified)?;
        out.say(format!("Using collection '{qualified}' ({} keys).", self.engine.len()));
        Ok(())
    }

    /// `use project <name>`: the project, even when a collection outside a
    /// project has its name.
    fn use_project_named(&mut self, name: &str, out: &mut Out) -> R {
        if let Some(p) = auth::project_of(name) {
            return Err(format!("a project name has no '.': use project {p}"));
        }
        self.use_project(name)?;
        self.say_project(name, out);
        Ok(())
    }

    /// `format` shows the formats dates are written and read in;
    /// `format date "DD-MM-YYYY"` (or `iso`) changes one for this session,
    /// `format date default` goes back to the user's, else the server's;
    /// `format iso` and `format default` do it for the three at once.
    fn cmd_format(&mut self, a: &[Token], out: &mut Out) -> R {
        use crate::time::FormatKind;
        const USAGE: &str = "Usage: format [iso | default | date|time|datetime \"<format>\"|iso|default]   \
                             (formats: DD-MM-YYYY, MM/DD/YYYY, HH:mm, DD/MM/YYYY HH:mm:ss XXX...)";
        let mut c = Cur::new(a);
        let account = || {
            let user = self.user.as_ref().map(|u| u.name.clone());
            user.and_then(|n| auth::get_user(&self.inst.system, &n).ok().flatten())
        };
        // The account's format of a kind, if it sets one.
        let of_account = |user: &Option<auth::User>, kind: FormatKind| {
            let text = user.as_ref()?.formats.get(kind.name())?;
            crate::time::parse_format(kind, text).ok().map(|p| (p, "the account"))
        };
        // All three when asked; after a change, the ones changed.
        let mut shown = FormatKind::ALL.to_vec();
        if let Some(t) = c.next() {
            let all = FormatKind::parse(&t.text).is_none();
            let kinds: Vec<FormatKind> = match FormatKind::parse(&t.text) {
                Some(kind) => vec![kind],
                None => {
                    c.finish(USAGE)?;
                    FormatKind::ALL.to_vec()
                }
            };
            let value = if kinds.len() == 1 { word(c.next(), "a format, iso or default")? } else { t.text.clone() };
            c.finish(USAGE)?;
            let user = account();
            shown = kinds.clone();
            for kind in kinds {
                if value.eq_ignore_ascii_case("default") {
                    match of_account(&user, kind) {
                        Some(p) => self.formats.insert(kind, p),
                        None => self.formats.remove(&kind),
                    };
                } else if value.eq_ignore_ascii_case("iso") {
                    self.formats.insert(kind, (None, "this session"));
                } else if all {
                    return Err(USAGE.into());
                } else {
                    self.formats.insert(kind, (crate::time::parse_format(kind, &value)?, "this session"));
                }
            }
            self.use_zone();
        }
        let now = crate::time::now();
        let examples = [
            crate::time::shown_date(crate::time::today()),
            crate::time::shown_time(crate::time::current_time()),
            crate::time::shown_instant(now),
        ];
        for (kind, example) in FormatKind::ALL.into_iter().zip(examples).filter(|(k, _)| shown.contains(k)) {
            let (text, from) = match self.formats.get(&kind) {
                Some((p, from)) => (p.as_ref().map_or("iso", |p| p.text()).to_string(), from.to_string()),
                None => (
                    self.inst.format(kind).map_or("iso".to_string(), |p| p.text().to_string()),
                    format!("the server's time.{}", kind.config_key()),
                ),
            };
            out.say(format!("{:<9} {text} (from {from}), now: {example}", format!("{}:", kind.name())));
        }
        Ok(())
    }

    /// `timezone` shows the zone dates are shown and read in; `timezone <name>`
    /// changes it for this session, `timezone default` goes back to the
    /// user's, else the server's.
    fn cmd_timezone(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: timezone [<name> | default]   (names: UTC, local, Africa/Kinshasa, +01:00...)";
        let mut c = Cur::new(a);
        if let Some(t) = c.next() {
            c.finish(USAGE)?;
            self.tz = if t.is_kw("default") {
                let user = self.user.as_ref().map(|u| u.name.clone());
                user.and_then(|n| auth::get_user(&self.inst.system, &n).ok().flatten())
                    .and_then(|u| u.timezone)
                    .and_then(|z| crate::time::Zone::parse(&z).ok())
            } else {
                Some(crate::time::Zone::parse(&t.text)?)
            };
            self.use_zone();
        }
        let zone = self.zone();
        let from = if self.tz.is_some() { "this session" } else { "the server's time.zone" };
        out.say(format!(
            "Time zone: {} (from {from}). It is {} here, {} UTC.",
            zone.name(),
            crate::time::show(crate::time::now()),
            crate::time::iso(crate::time::now())
        ));
        Ok(())
    }

    /// `project list | create <name> | remove <name>`: projects group
    /// workspaces (`travel.places`, `travel.cache`) under one name.
    fn cmd_project(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "Usage: project list | current | create <name> [collections (<name>, ...)] | drop|remove <name> [cascade]";
        let mut c = Cur::new(a);
        if c.eat("list") || c.peek().is_none() {
            c.finish(USAGE)?;
            let me = match &self.user {
                Some(u) if u.role != Role::Admin => auth::get_user(&self.inst.system, &u.name)?,
                _ => None,
            };
            let names = self.inst.workspace_names();
            for p in self.inst.projects() {
                let inside: Vec<&String> = names
                    .iter()
                    .filter(|w| auth::project_of(w) == Some(p.as_str()))
                    .filter(|w| me.as_ref().is_none_or(|u| u.access(w).is_some()))
                    .collect();
                // A user sees the projects where they can reach something.
                if me.as_ref().is_some_and(|u| inside.is_empty() && !u.manages_project(&p)) {
                    continue;
                }
                let list: Vec<&str> = inside.iter().map(|w| &w[p.len() + 1..]).collect();
                out.say(format!(" {p:<20} {} collection(s): {}", list.len(), list.join(", ")));
            }
            Ok(())
        } else if c.eat("create") {
            let name = word(c.next(), "a project name")?;
            if c.eat("collections") || c.eat("collection") {
                let inside = name_list(&mut c, USAGE)?;
                c.finish(USAGE)?;
                let dirs = self.inst.create_project_with(&name, &inside)?;
                out.say(format!("Project '{name}' created with {} collection(s):", dirs.len()));
                for (c, dir) in inside.iter().zip(&dirs) {
                    out.say(format!("  {name}.{c}  {}", dir.display()));
                }
                return Ok(());
            }
            c.finish(USAGE)?;
            self.inst.create_project(&name)?;
            out.say(format!(
                "Project '{name}' created. Its collections are named {name}.<name>: use {name}, then collection \
                 create places"
            ));
            Ok(())
        } else if c.eat("current") {
            c.finish(USAGE)?;
            out.say(format!("Current project: {}", self.current_project().unwrap_or("none")));
            Ok(())
        } else if c.eat("remove") || c.eat("drop") {
            let name = word(c.next(), "a project name")?;
            let cascade = c.eat("cascade");
            c.finish(USAGE)?;
            if !self.inst.project_exists(&name) {
                return Err(format!("project '{name}' does not exist."));
            }
            let inside: Vec<String> = self
                .inst
                .workspace_names()
                .into_iter()
                .filter(|w| auth::project_of(w) == Some(name.as_str()))
                .collect();
            if !inside.is_empty() && !cascade {
                return Err(format!(
                    "project '{name}' still holds {}: drop them first, or drop the project with them \
                     (project drop {name} cascade), which unregisters them and keeps their files.",
                    inside.join(", ")
                ));
            }
            if self.ws_priv.is_some() && inside.contains(&self.ws) {
                return Err(format!("you are in {}: enter another collection first (use default).", self.ws));
            }
            for w in &inside {
                self.inst.remove_workspace(w)?;
            }
            if !self.inst.remove_project(&name)? {
                return Err(format!("project '{name}' does not exist."));
            }
            if self.project.as_deref() == Some(name.as_str()) {
                self.project = None;
            }
            if inside.is_empty() {
                out.say(format!("Project '{name}' removed."));
            } else {
                out.say(format!(
                    "Project '{name}' removed, and {} collection(s) unregistered: {}. Their data files were left \
                     on disk.",
                    inside.len(),
                    inside.join(", ")
                ));
            }
            Ok(())
        } else {
            Err(USAGE.into())
        }
    }

    fn cmd_hint(&mut self, a: &[Token], out: &mut Out) -> R {
        if !(a.len() == 1 && a[0].is_kw("info")) {
            return Err("Usage: hint info".into());
        }
        // Report on a fresh snapshot rather than on whatever was last written.
        self.engine.snapshot();
        let h = self.engine.hint_info();
        out.say(format!("Hint File: {}", h.path.display()));
        out.say(format!("  Status: {}", h.status));
        out.say(format!("  Entries: {}", h.entries));
        out.say(format!("  Size on disk: {}", human_size(h.size)));
        out.say(format!("  Compression: {}", h.compression));
        out.say(format!("  Last updated: {}", h.modified.map(fmt_time).unwrap_or_else(|| "-".into())));
        out.say(format!("  Checksum: {}", if h.checksum_ok { "PASSED" } else { "FAILED" }));
        Ok(())
    }

    fn cmd_autocompact(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: autocompact on | off | status | threshold <1-99>";
        let mut c = Cur::new(a);
        if c.eat("status") {
            let (on, th) = self.engine.auto_compact();
            let s = self.engine.stats();
            out.say(format!("Auto-compaction: {}", if on { "ON" } else { "OFF" }));
            out.say(format!("  Threshold: {th}% fragmentation"));
            out.say(format!("  Current fragmentation: {:.1}%", s.fragmentation * 100.0));
            out.say(format!("  Compactions run: {}", s.compactions));
            return Ok(());
        }
        let (key, value) = if c.eat("on") {
            ("auto_compact", "true".to_string())
        } else if c.eat("off") {
            ("auto_compact", "false".to_string())
        } else if c.eat("threshold") {
            let n = uint(c.next(), "threshold")?;
            if !(1..=99).contains(&n) {
                return Err("ValueError: threshold must be between 1 and 99.".into());
            }
            ("compact_threshold", n.to_string())
        } else {
            return Err(USAGE.into());
        };
        c.finish(USAGE)?;
        self.inst.cfg_mut().set("storage", key, &value)?;
        self.inst.apply_storage_config();
        let (on, th) = self.engine.auto_compact();
        out.say(format!("Auto-compaction is {} (threshold {th}%).", if on { "ON" } else { "OFF" }));
        Ok(())
    }

    fn cmd_changes(&mut self, a: &[Token], out: &mut Out) -> R {
        use crate::engine::ChangesFrom;
        const USAGE: &str = "Usage: changes enable [keys only] [retain <24h>] [max <256mb>] | changes disable | \
                             changes status | changes read [from beginning|now|<cursor>] [as <consumer>] [limit N] \
                             [like <pattern>] | changes forget <consumer>";
        let mut c = Cur::new(a);
        let sub = word(c.next(), "a subcommand").map_err(|_| USAGE.to_string())?.to_ascii_lowercase();
        match sub.as_str() {
            "enable" => {
                let mut s = self.engine.changes_status().map(|s| s.settings).unwrap_or_default();
                while let Some(t) = c.next() {
                    if t.is_kw("keys") {
                        if !c.eat("only") {
                            return Err(USAGE.into());
                        }
                        s.values = false;
                    } else if t.is_kw("values") {
                        s.values = true;
                    } else if t.is_kw("retain") {
                        s.retain_secs = parse_span(&word(c.next(), "a duration such as 24h")?)?;
                    } else if t.is_kw("max") {
                        s.max_bytes = parse_bytes(&word(c.next(), "a size such as 256mb")?)?;
                    } else {
                        return Err(USAGE.into());
                    }
                }
                let what = if s.values { "keys and values" } else { "keys only" };
                let (retain, max) = (span_text(s.retain_secs), human_size(s.max_bytes));
                let started = self.engine.enable_changes(s)?;
                out.say(if started {
                    format!("Change journal started for workspace '{}': {what}, kept {retain}, at most {max}.", self.ws)
                } else {
                    format!("Change journal of workspace '{}' now keeps {what}, for {retain}, at most {max}.", self.ws)
                });
                Ok(())
            }
            "disable" => {
                c.finish(USAGE)?;
                if self.engine.disable_changes()? {
                    out.say(format!(
                        "Change journal of workspace '{}' removed; its cursors are no longer valid.",
                        self.ws
                    ));
                } else {
                    out.say(format!("Workspace '{}' keeps no change journal.", self.ws));
                }
                Ok(())
            }
            "forget" => {
                let name = word(c.next(), "a consumer name")?;
                c.finish(USAGE)?;
                if !self.engine.remove_consumer(&name)? {
                    return Err(format!("No consumer named '{name}'."));
                }
                out.say(format!("Consumer '{name}' forgotten."));
                Ok(())
            }
            "status" => {
                c.finish(USAGE)?;
                let Some(s) = self.engine.changes_status() else {
                    out.say(format!("Workspace '{}' keeps no change journal (changes enable starts one).", self.ws));
                    return Ok(());
                };
                if let Some(e) = &s.error {
                    out.say(format!("Change journal: unusable ({e}). Restart KivaDB to rebuild it from the data."));
                    return Ok(());
                }
                out.say(format!(
                    "Change journal: {}, kept {}, at most {}.",
                    if s.settings.values { "keys and values" } else { "keys only" },
                    span_text(s.settings.retain_secs),
                    human_size(s.settings.max_bytes)
                ));
                out.say(format!(
                    "  Changes kept: {} (numbers {} to {}), {} on disk.",
                    s.retained,
                    s.first_seq,
                    s.last_seq,
                    human_size(s.bytes)
                ));
                for (name, seq, at) in &s.consumers {
                    out.say(format!(
                        "  Consumer {name}: handled up to {seq}, {} behind, last seen {}.",
                        s.last_seq.saturating_sub(*seq),
                        fmt_time(*at)
                    ));
                }
                Ok(())
            }
            "read" => {
                let (mut from, mut consumer, mut limit, mut pattern) = (None, None, 20, None);
                while let Some(t) = c.next() {
                    if t.is_kw("from") {
                        let v = word(c.next(), "beginning, now or a cursor")?;
                        from = Some(match v.to_ascii_lowercase().as_str() {
                            "beginning" => ChangesFrom::Beginning,
                            "now" => ChangesFrom::Now,
                            _ => ChangesFrom::Cursor(v),
                        });
                    } else if t.is_kw("as") {
                        consumer = Some(word(c.next(), "a consumer name")?);
                    } else if t.is_kw("limit") {
                        limit = uint(c.next(), "limit")? as usize;
                    } else if t.is_kw("like") {
                        pattern = Some(word(c.next(), "a key pattern")?);
                    } else {
                        return Err(USAGE.into());
                    }
                }
                let from =
                    from.unwrap_or(if consumer.is_some() { ChangesFrom::Consumer } else { ChangesFrom::Beginning });
                let read = self.engine.read_changes(from, limit, pattern.as_deref(), consumer.as_deref())?;
                for ch in &read.changes {
                    let j = crate::changes::to_json(ch, "");
                    let what = match ch.kind {
                        crate::changes::ChangeKind::Set => format!(
                            "{} {} ({}){}",
                            if ch.created == Some(true) {
                                "insert"
                            } else if ch.created.is_none() {
                                "set"
                            } else {
                                "update"
                            },
                            ch.key,
                            j["type"].as_str().unwrap_or("?"),
                            if j["value"].is_null() { String::new() } else { format!(" {}", j["value"]) }
                        ),
                        crate::changes::ChangeKind::Del => format!("delete {}", ch.key),
                        crate::changes::ChangeKind::Reset => "reset (all keys deleted)".to_string(),
                        crate::changes::ChangeKind::Expire => format!(
                            "expire {} (its TTL ran out at {})",
                            ch.key,
                            j["expires_at"].as_str().unwrap_or("?")
                        ),
                    };
                    let txn = match (ch.txn, ch.txn_end) {
                        (Some(t), true) => format!(" [transaction {t}, end]"),
                        (Some(t), false) => format!(" [transaction {t}]"),
                        _ => String::new(),
                    };
                    out.say(format!("#{} {} {what}{txn}", ch.seq, j["timestamp"].as_str().unwrap_or("")));
                }
                if read.changes.is_empty() {
                    out.say("No change after this point.");
                }
                out.say(format!("Cursor: {}{}", read.cursor, if read.more { " (more after it)" } else { "" }));
                Ok(())
            }
            _ => Err(USAGE.into()),
        }
    }

    fn cmd_schema(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "Usage: schema set <pattern> {<field>: {<rules>}} | schema show <pattern> | schema check <pattern> | schema drop <pattern> | schemas";
        let mut c = Cur::new(a);
        // `schemas`, or `schema` alone, lists them.
        let sub = match c.next() {
            None => "list".to_string(),
            t => word(t, "set, show, check, drop or list").map_err(|_| USAGE.to_string())?.to_ascii_lowercase(),
        };
        if sub == "list" {
            c.finish(USAGE)?;
            let list = self.engine.schemas();
            if list.is_empty() {
                out.say("No schemas in this workspace.");
            }
            for (s, _) in list {
                let fields: Vec<String> = s.spec.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
                let unique = s.unique_fields();
                let unique =
                    if unique.is_empty() { String::new() } else { format!(" (unique: {})", unique.join(", ")) };
                out.say(format!("{} - {}{unique}", s.pattern, fields.join(", ")));
            }
            return Ok(());
        }
        let mut pattern = word(c.next(), "a key pattern").map_err(|_| USAGE.to_string())?;
        if !pattern.contains(['*', '?']) {
            pattern.push('*');
        }
        match sub.as_str() {
            "set" => {
                let spec = c.next().filter(|t| t.kind == Kind::Json).ok_or(USAGE)?;
                let spec: serde_json::Value = crate::exec::json_literal(&spec.text)?;
                c.finish(USAGE)?;
                let r = self.engine.set_schema(&pattern, &spec)?;
                let fields = spec.as_object().map(|m| m.len()).unwrap_or(0);
                let verb = if r.replaced { "replaced" } else { "set" };
                out.say(format!("Schema {verb} on {pattern} ({fields} field{}).", if fields == 1 { "" } else { "s" }));
                for f in &r.unique_indexes {
                    out.say(format!("Unique index created on {pattern} ({f})."));
                }
                match (r.checked, r.nonconforming) {
                    (0, _) => {}
                    (n, 0) => out.say(format!("{n} document(s) already stored: all follow it.")),
                    (n, bad) => out.say(format!(
                        "{n} document(s) already stored, {bad} of them do not follow it: they stay as they are, and must follow it at their next change (schema check {pattern})."
                    )),
                }
                Ok(())
            }
            "show" => {
                c.finish(USAGE)?;
                let list = self.engine.schemas();
                let (s, _) =
                    list.iter().find(|(s, _)| s.pattern == pattern).ok_or(format!("No schema on {pattern}."))?;
                out.say(format!(
                    "schema set {} {}",
                    s.pattern,
                    serde_json::to_string_pretty(&s.spec).unwrap_or_default()
                ));
                Ok(())
            }
            "check" => {
                c.finish(USAGE)?;
                let found = self.engine.check_schema(&pattern, 20)?;
                let (read, bad, list) = (found.read, found.failing, found.examples);
                if bad == 0 {
                    out.say(format!("{pattern}: {read} document(s), all follow the schema."));
                } else {
                    out.say(format!("{pattern}: {read} document(s), {bad} do not follow the schema:"));
                    for (k, why) in list {
                        out.say(format!("  - {k}: {why}"));
                    }
                    if bad > 20 {
                        out.say(format!("  ... and {} more", bad - 20));
                    }
                }
                Ok(())
            }
            "drop" => {
                c.finish(USAGE)?;
                let owned = self
                    .engine
                    .schemas()
                    .into_iter()
                    .find(|(s, _)| s.pattern == pattern)
                    .map(|(_, o)| o)
                    .unwrap_or_default();
                if !self.engine.drop_schema(&pattern)? {
                    return Err(format!("No schema on {pattern}."));
                }
                out.say(format!("Schema dropped on {pattern}."));
                for f in owned {
                    out.say(format!("Unique index on {pattern} ({f}) dropped with it."));
                }
                Ok(())
            }
            _ => Err(USAGE.into()),
        }
    }

    fn cmd_check(&mut self, a: &[Token], out: &mut Out) -> R {
        let repair = match a {
            [] => false,
            [r] if r.is_kw("repair") => true,
            _ => return Err("Usage: check | check repair".into()),
        };
        if repair {
            self.writable()?;
        }
        let r = self.engine.check(repair)?;
        out.say(format!("Checking {} entries...", r.total));
        out.say(format!("  OK: {}", r.ok));
        out.say(format!("  CORRUPTED: {}", r.corrupted.len()));
        for k in r.corrupted.iter().take(20) {
            out.say(format!("    - {k}"));
        }
        if repair && r.repaired > 0 {
            if let Some(b) = &r.backup {
                out.say(format!("  Backup saved to {}", b.display()));
            }
            out.say(format!("  Repairing... {} entries removed.", r.repaired));
        } else if !r.corrupted.is_empty() {
            out.say("  Run 'check repair' to remove the corrupted entries.");
        }
        out.say("Done.");
        Ok(())
    }

    fn cmd_compress(&mut self, a: &[Token], out: &mut Out) -> R {
        let algo = match a {
            [t] => Compression::from_name(&t.text),
            _ => None,
        }
        .ok_or("Usage: compress lz4 | zlib | none")?;
        self.writable()?;
        let before = self.engine.stats();
        let r = self.engine.compact_with(Some(algo))?;
        if r.aborted {
            return Err("compression was interrupted by a database reset.".into());
        }
        self.inst.cfg_mut().set("storage", "compression", algo.name())?;
        self.inst.apply_storage_config();
        let after = self.engine.stats();
        let ratio = if after.raw_value_bytes > 0 {
            (1.0 - after.stored_value_bytes as f64 / after.raw_value_bytes as f64) * 100.0
        } else {
            0.0
        };
        out.say(format!(
            "Database re-encoded with compression '{}': {} -> {} on disk ({} records, values {ratio:.1}% smaller).",
            algo.name(),
            human_size(before.file_bytes),
            human_size(after.file_bytes),
            r.records
        ));
        Ok(())
    }

    fn cmd_replica(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "Usage: replica status | start | stop | sync | promote | replica of <host> <port> | replica of none";
        let inst = self.inst.clone();
        let mut c = Cur::new(a);
        let follow = |host: &str, port: u16| -> R {
            if port == inst.tcp_port.load(std::sync::atomic::Ordering::SeqCst) as u16
                && matches!(host, "127.0.0.1" | "localhost")
            {
                return Err("a node cannot replicate from itself.".into());
            }
            let mut cfg = inst.cfg_mut();
            cfg.set("replication", "role", "replica")?;
            cfg.set("replication", "master_host", host)?;
            cfg.set("replication", "master_port", &port.to_string())?;
            drop(cfg);
            repl::start_replica(&inst, host, port);
            Ok(())
        };
        let configured = || -> Result<(String, u16), String> {
            let cfg = inst.cfg();
            let host = cfg.get("replication", "master_host");
            if host.is_empty() {
                return Err("no master configured. Use: replica of <host> <port>".into());
            }
            Ok((host, cfg.get_u64("replication", "master_port") as u16))
        };
        if c.eat("status") {
            if inst.repl.is_replica() {
                let l = inst.repl.link();
                out.say("Role: replica (read-only)");
                out.say(format!("  Master: {}", l.master));
                out.say(format!("  Link: {}", if l.running { l.status.as_str() } else { "stopped" }));
                out.say(format!("  Applied sequence: {} ({} operations since connect)", l.applied_seq, l.applied_ops));
                out.say(format!("  Last sync: {}", l.last_sync.map(fmt_time).unwrap_or_else(|| "never".into())));
                out.say(format!("  Workspaces: {}", workspace_names(&inst)));
            } else {
                let reps = inst.repl.replicas();
                let seq = inst.repl.seq();
                let cfg = inst.cfg();
                out.say("Role: master");
                out.say(format!(
                    "  Mode: {} (min acks: {})",
                    cfg.get("replication", "sync_mode"),
                    cfg.get("replication", "min_slaves_ack")
                ));
                out.say(format!("  Sequence: {seq}"));
                out.say(format!("  Workspaces: {}", workspace_names(&inst)));
                out.say(format!("  Connected replicas: {}", reps.len()));
                for r in reps {
                    out.say(format!(
                        "    #{} {} (listens on {}) lag: {} ops, connected {}",
                        r.id,
                        r.addr,
                        r.listen,
                        seq.saturating_sub(r.acked),
                        fmt_time(r.connected_at)
                    ));
                }
            }
            Ok(())
        } else if c.eat("of") {
            if c.eat("none") {
                inst.repl.promote();
                inst.repl.set_epoch(&inst, inst.repl.epoch() + 1);
                inst.cfg_mut().set("replication", "role", "master")?;
                out.say("This node is now a master.");
                return Ok(());
            }
            let host = word(c.next(), "a host")?;
            let port = uint(c.next(), "port")? as u16;
            c.finish(USAGE)?;
            follow(&host, port)?;
            out.say(format!("Now replicating from {host}:{port}. Local data is replaced by the master's."));
            Ok(())
        } else if c.eat("start") || c.eat("sync") {
            let (host, port) = configured()?;
            follow(&host, port)?;
            out.say(format!("Replication started: full synchronisation from {host}:{port}."));
            Ok(())
        } else if c.eat("stop") {
            inst.repl.stop_replica();
            out.say("Replication stopped. This node stays read-only until 'replica promote'.");
            Ok(())
        } else if c.eat("promote") {
            inst.repl.promote();
            // A new epoch: a former master that comes back steps down.
            inst.repl.set_epoch(&inst, inst.repl.epoch() + 1);
            inst.cfg_mut().set("replication", "role", "master")?;
            out.say("This node is now a master.");
            Ok(())
        } else {
            Err(USAGE.into())
        }
    }

    fn cmd_wait(&mut self, a: &[Token], out: &mut Out) -> R {
        if a.is_empty() || a.len() > 2 {
            return Err("Usage: wait <replicas> [timeout_ms]".into());
        }
        let n = uint(a.first(), "replica count")? as usize;
        let timeout = if a.len() == 2 { uint(a.get(1), "timeout")? } else { 5000 };
        let got = self.inst.repl.wait(n, Duration::from_millis(timeout));
        if got >= n {
            out.say(format!("OK: {got} replica(s) acknowledged."));
        } else {
            out.err(format!("Timeout: only {got} of {n} replica(s) acknowledged within {timeout} ms."));
        }
        Ok(())
    }

    fn cmd_failover(&mut self, a: &[Token], out: &mut Out) -> R {
        if a.len() == 1 && a[0].is_kw("status") {
            for line in crate::failover::status(&self.inst) {
                out.say(line);
            }
            return Ok(());
        }
        let [to, target] = a else {
            return Err("Usage: failover to <replica id or address> | failover status".into());
        };
        if !to.is_kw("to") {
            return Err("Usage: failover to <replica id or address>".into());
        }
        let inst = self.inst.clone();
        if inst.repl.is_replica() {
            return Err("this node is a replica; run 'replica promote' on it instead.".into());
        }
        let target = target.text.trim_start_matches('#');
        let info = inst
            .repl
            .replicas()
            .into_iter()
            .find(|r| r.id.to_string() == target || r.addr == target || r.listen == target)
            .ok_or(format!("no connected replica matches '{target}'. See 'replica status'."))?;
        let (host, port) = info
            .listen
            .rsplit_once(':')
            .and_then(|(h, p)| p.parse::<u16>().ok().map(|p| (h.to_string(), p)))
            .filter(|(_, p)| *p != 0)
            .ok_or("that replica does not accept connections, so it cannot become the master.")?;
        // Make sure it has everything before handing over.
        inst.repl.wait(1, Duration::from_secs(5));
        let (id, _) = inst.repl.send_promote(target)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while inst.repl.has_replica(id) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        inst.repl.drop_replicas();
        {
            let mut cfg = inst.cfg_mut();
            cfg.set("replication", "role", "replica")?;
            cfg.set("replication", "master_host", &host)?;
            cfg.set("replication", "master_port", &port.to_string())?;
        }
        repl::start_replica(&inst, &host, port);
        out.say(format!("Failover complete: {host}:{port} is the new master and this node now follows it."));
        out.say("Other replicas must be pointed at the new master with: replica of <host> <port>");
        Ok(())
    }

    fn cmd_eval(&mut self, a: &[Token], out: &mut Out) -> R {
        let Some(src) = a.first().filter(|t| t.is_quoted()) else {
            return Err("Usage: eval '<lua code>' [args...]".into());
        };
        let args: Vec<String> = a[1..].iter().filter(|t| t.kind != Kind::Comma).map(|t| t.text.clone()).collect();
        let r = script::eval(self, &src.text, &args)?;
        for l in r.printed {
            out.say(l);
        }
        out.say(r.result.unwrap_or_else(|| "OK".into()));
        Ok(())
    }

    fn cmd_script(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "Usage: script load \"<file>\" as \"<name>\" | script run \"<name>\" [args] | script list | script show \"<name>\" | script delete \"<name>\"";
        let mut c = Cur::new(a);
        if c.eat("load") {
            let file = word(c.next(), "a file path")?;
            if !c.eat("as") {
                return Err(USAGE.into());
            }
            let name = word(c.next(), "a script name")?;
            c.finish(USAGE)?;
            let bytes = std::fs::read(&file).map_err(|e| format!("cannot read '{file}': {e}"))?;
            // Saved by a Windows editor or PowerShell: UTF-8 with a byte order mark, or UTF-16.
            let source = crate::kva::decode(&bytes).map_err(|e| format!("cannot read '{file}': {e}"))?;
            script::store(self, &name, &source)?;
            out.say(format!("Script '{name}' stored ({} bytes).", source.len()));
            Ok(())
        } else if c.eat("run") {
            let name = word(c.next(), "a script name")?;
            let args: Vec<String> = c.rest().iter().filter(|t| t.kind != Kind::Comma).map(|t| t.text.clone()).collect();
            let r = script::run_stored(self, &name, &args)?;
            for l in r.printed {
                out.say(l);
            }
            out.say(r.result.unwrap_or_else(|| "OK".into()));
            Ok(())
        } else if c.eat("list") {
            let list = script::list(self);
            if list.is_empty() {
                out.say("No stored scripts.");
            }
            for (name, size) in list {
                out.say(format!("{name:<24} {size} bytes"));
            }
            Ok(())
        } else if c.eat("show") {
            let name = word(c.next(), "a script name")?;
            match script::load_source(self, &name)? {
                Some(src) => out.say(src),
                None => return Err(format!("script '{name}' does not exist.")),
            }
            Ok(())
        } else if c.eat("delete") {
            let name = word(c.next(), "a script name")?;
            c.finish(USAGE)?;
            if !script::remove(self, &name)? {
                return Err(format!("script '{name}' does not exist."));
            }
            out.say(format!("Script '{name}' deleted."));
            Ok(())
        } else {
            Err(USAGE.into())
        }
    }

    fn cmd_bench(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: bench script \"<name>\" iterations <N> [args] | bench write <N> | bench read <N>";
        let report = |out: &mut Out, label: &str, n: u64, elapsed: Duration| {
            let secs = elapsed.as_secs_f64().max(1e-9);
            out.say(format!(
                "{label}: {n} ops in {secs:.3}s -> {:.0} ops/sec (avg {:.3} ms)",
                n as f64 / secs,
                secs * 1000.0 / n.max(1) as f64
            ));
        };
        let mut c = Cur::new(a);
        if c.eat("script") {
            let name = word(c.next(), "a script name")?;
            if !c.eat("iterations") {
                return Err(USAGE.into());
            }
            let n = uint(c.next(), "iterations")?;
            if n == 0 || n > 10_000_000 {
                return Err("ValueError: iterations must be between 1 and 10000000.".into());
            }
            let args: Vec<String> = c.rest().iter().map(|t| t.text.clone()).collect();
            let source = script::load_source(self, &name)?.ok_or(format!("script '{name}' does not exist."))?;
            let start = Instant::now();
            for _ in 0..n {
                script::eval(self, &source, &args)?;
            }
            report(out, &format!("SCRIPT {name}"), n, start.elapsed());
            Ok(())
        } else if c.eat("write") || c.eat("read") {
            let reading = a[0].is_kw("read");
            let n = uint(c.next(), "N")?;
            if n == 0 || n > 10_000_000 {
                return Err("ValueError: N must be between 1 and 10000000.".into());
            }
            self.writable()?;
            let value = Value::Str("x".repeat(64));
            let key = |i: u64| format!("__bench:{i:010}");
            let start = Instant::now();
            for i in 0..n {
                self.engine.put(&key(i), &value, 0, false)?;
            }
            let write_time = start.elapsed();
            if reading {
                let start = Instant::now();
                for i in 0..n {
                    self.engine.get(&key(i))?;
                }
                report(out, "READ ", n, start.elapsed());
            } else {
                report(out, "WRITE", n, write_time);
            }
            for i in 0..n {
                self.engine.delete(&key(i))?;
            }
            Ok(())
        } else {
            Err(USAGE.into())
        }
    }

    fn cmd_subscribe(&mut self, a: &[Token], out: &mut Out) -> R {
        let [p] = a else {
            return Err("Usage: subscribe <pattern>   (example: subscribe user:*)".into());
        };
        let pattern = key_for_read(p).unwrap_or_else(|_| p.text.clone());
        let Some(tx) = self.events.clone() else {
            return Err(
                "this connection cannot receive notifications. Use the shell, a TCP connection or the /ws WebSocket."
                    .into(),
            );
        };
        self.inst.pubsub.subscribe(self.id, &self.ws, &pattern, tx);
        if !self.subs.contains(&pattern) {
            self.subs.push(pattern.clone());
        }
        out.say(format!(
            "Subscribed to '{pattern}' in workspace '{}'. Changes will be announced as they happen.",
            self.ws
        ));
        Ok(())
    }
}

/// `on <pattern> ( <field> )`. A pattern without wildcard is used as a prefix.
impl Session {
    /// `backup to "<file.tar.gz>" [workspace <name>]`: an archive of the whole
    /// home, or of one workspace, written by the server while it keeps serving.
    fn cmd_backup(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: backup to \"<file.tar.gz>\" [workspace <name>]";
        let mut c = Cur::new(a);
        if !c.eat("to") {
            return Err(USAGE.into());
        }
        let file = word(c.next(), "a file name").map_err(|e| format!("{e}\n{USAGE}"))?;
        let scope = if c.eat("workspace") {
            crate::backup::Scope::Workspace(word(c.next(), "a workspace name")?)
        } else {
            crate::backup::Scope::Home
        };
        c.finish(USAGE)?;
        let report = crate::backup::backup(&self.inst, &scope, std::path::Path::new(&file))?;
        for line in report.lines() {
            out.say(line);
        }
        Ok(())
    }

    /// `memory [status] | memory pin <pattern> | memory unpin <pattern>`.
    fn cmd_memory(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: memory [status] | memory pin \"<pattern>\" | memory unpin \"<pattern>\"";
        let mut c = Cur::new(a);
        if c.eat("pin") {
            let pattern = word(c.next(), "a key pattern")?;
            c.finish(USAGE)?;
            let (n, bytes) = self.engine.pin(&pattern)?;
            out.say(format!(
                "Pinned '{pattern}': {n} values ({}) kept in memory, now and after every write.",
                human_size(bytes as u64)
            ));
            return Ok(());
        }
        if c.eat("unpin") {
            let pattern = word(c.next(), "a key pattern")?;
            c.finish(USAGE)?;
            if !self.engine.unpin(&pattern)? {
                return Err(format!("'{pattern}' is not pinned."));
            }
            out.say(format!("Unpinned '{pattern}': its values are read from disk again."));
            return Ok(());
        }
        c.eat("status");
        c.finish(USAGE)?;
        let m = self.engine.memory();
        let size = |n: usize| human_size(n as u64);
        out.say(format!("Memory of workspace '{}' (estimates)", self.ws));
        out.say(format!("  Keys              : {} ({} keys)", size(m.keys), m.key_count));
        out.say(format!("  Secondary indexes : {}", size(m.secondary)));
        match m.vector_limit {
            0 => out.say(format!("  Vector indexes    : {} (no bound of their own)", size(m.vectors))),
            b => out.say(format!(
                "  Vector indexes    : {} of {} ({:.1}%, vector.max_ram_mb)",
                size(m.vectors),
                size(b),
                m.vectors as f64 * 100.0 / b as f64
            )),
        }
        out.say(format!("  Pinned values     : {} ({} values)", size(m.pinned), m.pinned_values));
        out.say(format!("  Value cache       : {} (given back first)", size(m.cache)));
        out.say(format!("  Total             : {}", size(m.total())));
        if m.limit == 0 {
            out.say("  Bound             : none (storage.max_memory_mb = 0)");
        } else {
            let pct = m.resident() as f64 * 100.0 / m.limit as f64;
            out.say(format!("  Bound             : {} ({pct:.1}% used)", size(m.limit)));
        }
        if !m.pinned_patterns.is_empty() {
            out.say(format!("  Pinned patterns   : {}", m.pinned_patterns.join(", ")));
        }
        Ok(())
    }

    /// `semantic lookup | store | stats | clear`: the semantic cache.
    fn cmd_semantic(&mut self, a: &[Token], out: &mut Out) -> R {
        use crate::semantic_cache::{Lookup, Question, Settings, Stored};
        const USAGE: &str = "Usage: semantic lookup <cache> [text \"<question>\"] [vector [..]] [threshold <0-1>]
       semantic store <cache> [text \"<question>\"] [vector [..]] answer <value> [ttl <sec>] [depends \"<pattern>\", ...] [since <token>]
       semantic stats | semantic clear [<cache>]";
        let settings = Settings::of(&self.inst.cfg());
        let mut c = Cur::new(a);
        let action = word(c.next(), "lookup, store, stats or clear").map_err(|_| USAGE.to_string())?;
        match action.to_ascii_lowercase().as_str() {
            "stats" => {
                c.finish(USAGE)?;
                let stats = self.inst.semantic.stats(&self.ws);
                if stats.is_empty() {
                    out.say("No semantic cache in this workspace yet.");
                }
                for s in stats {
                    let rate = (s.hits * 100).checked_div(s.hits + s.misses).unwrap_or(0);
                    out.say(format!(
                        "{}: {} entries, {} hits, {} misses ({rate}% hit rate), {} invalidated by writes, {} dependencies",
                        s.name, s.entries, s.hits, s.misses, s.invalidated, s.depends
                    ));
                }
                Ok(())
            }
            "clear" => {
                let name = c.next().map(|t| t.text.clone());
                c.finish(USAGE)?;
                let n = self.inst.semantic.clear(&self.ws, name.as_deref());
                out.say(format!("Cleared {n} cached answer(s)."));
                Ok(())
            }
            act @ ("lookup" | "store") => {
                let name = word(c.next(), "a cache name").map_err(|e| format!("{e}\n{USAGE}"))?;
                let mut q = Question::default();
                let (mut threshold, mut ttl, mut answer, mut depends, mut since) =
                    (settings.threshold, settings.ttl, None, Vec::new(), None);
                loop {
                    if c.eat("text") {
                        q.text = Some(
                            c.next().filter(|t| t.is_quoted()).ok_or("text needs a quoted question")?.text.clone(),
                        );
                    } else if c.eat("vector") {
                        let t =
                            c.next().filter(|t| t.kind == Kind::Json).ok_or("vector needs a JSON array of numbers")?;
                        let j: serde_json::Value = serde_json::from_str(&t.text).map_err(|e| e.to_string())?;
                        q.vector = Some(crate::vector::from_json(&j).ok_or("vector needs a JSON array of numbers")?);
                    } else if c.eat("threshold") {
                        let t = c.next().map(|t| t.text.clone()).unwrap_or_default();
                        threshold = t
                            .parse()
                            .ok()
                            .filter(|t: &f32| *t > 0.0 && *t <= 1.0)
                            .ok_or("threshold is a number between 0 and 1")?;
                    } else if c.eat("ttl") {
                        ttl = uint(c.next(), "ttl")? as i64;
                    } else if act == "store" && c.eat("since") {
                        since = Some(uint(c.next(), "since")?);
                    } else if c.eat("answer") {
                        let t = c.next().ok_or("answer needs a value")?;
                        answer = Some(match t.kind {
                            Kind::Json => crate::exec::json_literal(&t.text)?,
                            _ if t.is_quoted() => serde_json::Value::String(t.text.clone()),
                            _ => serde_json::from_str(&t.text).unwrap_or(serde_json::Value::String(t.text.clone())),
                        });
                    } else if c.eat("depends") {
                        loop {
                            depends.push(
                                c.next()
                                    .filter(|t| t.is_quoted())
                                    .ok_or("depends needs quoted key patterns")?
                                    .text
                                    .clone(),
                            );
                            if c.peek().is_some_and(|t| t.kind == Kind::Comma) {
                                c.next();
                            } else {
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                }
                c.finish(USAGE)?;
                if act == "lookup" {
                    match self.inst.semantic.lookup(&self.ws, &name, &q, threshold)? {
                        Lookup::Hit { id, similarity, answer } => {
                            out.say(format!("Hit (similarity {similarity:.4}, {id}): {answer}"))
                        }
                        Lookup::Miss { best: Some(b), since } => {
                            out.say(format!("Miss (closest similarity {b:.4}, threshold {threshold}; since {since})"))
                        }
                        Lookup::Miss { best: None, since } => {
                            out.say(format!("Miss (nothing cached for this question; since {since})"))
                        }
                    }
                    return Ok(());
                }
                let answer = answer.ok_or_else(|| format!("store needs an answer.\n{USAGE}"))?;
                let id = match self.inst.semantic.store_since(
                    &self.ws,
                    &name,
                    &q,
                    answer,
                    ttl,
                    depends,
                    settings.max_entries,
                    since,
                )? {
                    Stored::Kept(id) => id,
                    Stored::Stale(why) => {
                        out.say(format!(
                            "Not stored: {why} after the lookup, so this answer may be built from old data."
                        ));
                        return Ok(());
                    }
                };
                out.say(if ttl > 0 {
                    format!("Stored as {id} (expires in {ttl} s).")
                } else {
                    format!("Stored as {id} (no expiry).")
                });
                Ok(())
            }
            _ => Err(USAGE.into()),
        }
    }

    /// `encryption`: whether data is encrypted at rest, and what is left in clear.
    fn cmd_encryption(&mut self, a: &[Token], out: &mut Out) -> R {
        if !a.is_empty() {
            return Err("Usage: encryption".into());
        }
        let r = self.engine.encryption_report()?;
        match &r.key_id {
            Some(id) => out.say(format!("Encryption at rest: on (AES-256-GCM, key {id})")),
            None => out.say("Encryption at rest: off (no key: set KIVADB_ENCRYPTION_KEY or encryption.key_file)"),
        }
        let encrypted = r.records - r.plain;
        out.say(format!(
            "Workspace '{}': {} records in the data file, {encrypted} encrypted, {} in clear",
            self.ws, r.records, r.plain
        ));
        if r.key_id.is_some() {
            if r.other_key > 0 {
                out.say(format!(" - {} encrypted with an older key", r.other_key));
            }
            if r.plain > 0 || r.other_key > 0 {
                out.say(" - run 'compact' to rewrite them with the current key (older versions included)");
            } else {
                out.say(" - every record is encrypted with the current key");
            }
            out.say(format!(
                "Index snapshot (hint): {}",
                if r.hint_encrypted { "encrypted" } else { "not written yet with the key" }
            ));
        }
        Ok(())
    }
}

fn parse_index_spec(c: &mut Cur, usage: &str) -> Result<(String, String), String> {
    let target = parse_index_target(c, usage)?;
    c.finish(usage)?;
    Ok(target)
}

/// `on <pattern> (<field>)` or `on <pattern>.<field>`, leaving what follows
/// to the caller.
/// `ef_search`: a number of candidates, or `default`.
fn parse_ef_search(t: Option<&Token>) -> Result<usize, String> {
    if t.is_some_and(|t| t.is_kw("default")) {
        return Ok(0);
    }
    let ef = uint(t, "ef_search")? as usize;
    if !(1..=100_000).contains(&ef) {
        return Err("ef_search must be between 1 and 100000, or default".into());
    }
    Ok(ef)
}

/// `24h`, `7d`, `30m`, `90s`, or seconds.
fn parse_span(s: &str) -> Result<u64, String> {
    let s = s.trim().to_ascii_lowercase();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = n.parse().map_err(|_| format!("'{s}' is not a duration: write it as 24h, 7d, 30m or seconds"))?;
    let mult = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return Err(format!("'{s}' is not a duration: write it as 24h, 7d, 30m or seconds")),
    };
    n.checked_mul(mult).filter(|v| *v > 0).ok_or_else(|| format!("'{s}' is not a usable duration"))
}

fn span_text(secs: u64) -> String {
    match secs {
        s if s % 86400 == 0 => format!("{} day(s)", s / 86400),
        s if s % 3600 == 0 => format!("{} hour(s)", s / 3600),
        s if s % 60 == 0 => format!("{} minute(s)", s / 60),
        s => format!("{s} second(s)"),
    }
}

/// `256mb`, `1gb`, `64kb`, or bytes.
fn parse_bytes(s: &str) -> Result<u64, String> {
    let s = s.trim().to_ascii_lowercase();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = n.parse().map_err(|_| format!("'{s}' is not a size: write it as 256mb or 1gb"))?;
    let mult = match unit {
        "" | "b" => 1,
        "kb" | "k" => 1 << 10,
        "mb" | "m" => 1 << 20,
        "gb" | "g" => 1 << 30,
        _ => return Err(format!("'{s}' is not a size: write it as 256mb or 1gb")),
    };
    n.checked_mul(mult).filter(|v| *v >= 1 << 20).ok_or_else(|| "the journal needs at least 1mb".to_string())
}

fn parse_index_target(c: &mut Cur, usage: &str) -> Result<(String, String), String> {
    if !c.eat("on") {
        return Err(format!("Usage: {usage}"));
    }
    let target = word(c.next(), "a key pattern")?;
    let (mut pattern, field) = if c.peek().is_some_and(|t| t.kind == Kind::LParen) {
        c.next();
        // `(type, parent, name)`: the fields of a unique index, joined by `, `.
        let mut fields = vec![word(c.next(), "a field name")?.trim_start_matches('.').to_string()];
        loop {
            match c.next() {
                Some(t) if t.kind == Kind::RParen => break,
                Some(t) if t.kind == Kind::Comma => {
                    fields.push(word(c.next(), "a field name")?.trim_start_matches('.').to_string())
                }
                _ => return Err(format!("Usage: {usage}")),
            }
        }
        (target, fields.join(", "))
    } else {
        crate::exec::split_target(&target).map_err(|_| format!("Usage: {usage}"))?
    };
    if !pattern.contains(['*', '?']) {
        pattern.push('*');
    }
    Ok((pattern, field.trim_start_matches('.').to_string()))
}

/// `3 (default, hr, sales)`: the workspaces replication covers.
fn workspace_names(inst: &crate::instance::Instance) -> String {
    let names: Vec<String> = inst.workspaces().into_iter().map(|(n, _)| n).collect();
    format!("{} ({})", names.len(), names.join(", "))
}

/// `(cache, places, users)`: names between parentheses, separated by commas.
fn name_list(c: &mut Cur, usage: &str) -> Result<Vec<String>, String> {
    if !c.peek().is_some_and(|t| t.kind == Kind::LParen) {
        return Err(usage_line(usage));
    }
    c.next();
    let mut names = Vec::new();
    loop {
        let t = c.next().ok_or_else(|| usage_line(usage))?;
        match t.kind {
            Kind::RParen if !names.is_empty() => return Ok(names),
            Kind::Comma | Kind::LParen | Kind::RParen => return Err(usage_line(usage)),
            _ => names.push(t.text.clone()),
        }
        match c.next() {
            Some(t) if t.kind == Kind::Comma => {}
            Some(t) if t.kind == Kind::RParen => return Ok(names),
            _ => return Err(usage_line(usage)),
        }
    }
}
