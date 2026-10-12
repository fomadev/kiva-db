// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Administrative commands: users, configuration, workspaces, maintenance,
//! replication, scripting and notifications.

use crate::auth::{self, Priv, Role};
use crate::compress::Compression;
use crate::exec::{key_for_read, uint, word, Cur, Out, R};
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
                out.say(format!("{} ({role}) @ workspace '{}'", self.user_name(), self.ws));
                Ok(())
            }
            "users" => self.cmd_users(out),
            "create" => self.cmd_create(a, out),
            "drop" => self.cmd_drop(a, out),
            "grant" => self.cmd_grant(a, out, true),
            "revoke" => self.cmd_grant(a, out, false),
            "modify" => self.cmd_modify(a, out),
            "audit" => self.cmd_audit(a, out),
            "config" => self.cmd_config(a, out),
            "workspace" => self.cmd_workspace(a, out),
            "hint" => self.cmd_hint(a, out),
            "autocompact" => self.cmd_autocompact(a, out),
            "check" => self.cmd_check(a, out),
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
                if list.is_empty() {
                    out.say("No secondary indexes in this workspace.");
                }
                for (p, f, n) in list {
                    out.say(format!("{p} ({f}) - {n} documents indexed"));
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
        out.say(format!(" {:<20} | {:<7} | {:<19} | WORKSPACE GRANTS", "USER", "ROLE", "CREATED"));
        out.say("-".repeat(78));
        for u in auth::list_users(&self.inst.system)? {
            let grants = if u.role == Role::Admin {
                "all (admin)".to_string()
            } else {
                let mut g = vec![format!("{}=write", u.home_workspace())];
                g.extend(u.grants.iter().map(|(w, p)| format!("{}={}", if w == "*" { "all" } else { w }, p.name())));
                g.join(", ")
            };
            out.say(format!(" {:<20} | {:<7} | {:<19} | {grants}", u.name, u.role.name(), fmt_time(u.created)));
        }
        Ok(())
    }

    fn cmd_create(&mut self, a: &[Token], out: &mut Out) -> R {
        let mut c = Cur::new(a);
        if c.eat("user") {
            const USAGE: &str = "create user \"<name>\" identified by \"<password>\" [role admin|editor|viewer]";
            let name = word(c.next(), "a user name")?;
            if !(c.eat("identified") && c.eat("by")) {
                return Err(format!("Usage: {USAGE}"));
            }
            let password = word(c.next(), "a password")?;
            let role = if c.eat("role") {
                Role::from_name(&word(c.next(), "a role")?).ok_or("role must be admin, editor or viewer")?
            } else {
                Role::Editor
            };
            c.finish(USAGE)?;
            let user = auth::create_user(&self.inst.system, &name, &password, role)?;
            self.inst.ensure_workspace(&user.home_workspace())?;
            out.say(format!("User '{name}' created with role [{}].", role.name()));
            Ok(())
        } else if c.eat("index") {
            let (pattern, field) = parse_index_spec(&mut c, "create index on <pattern> (<field>)")?;
            let n = self.engine.create_index(&pattern, &field)?;
            out.say(format!("Index created on {pattern} ({field}): {n} documents indexed."));
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
            Err("Usage: create user ... | create index on <pattern> (<field>) | create trigger ...".into())
        }
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
        } else if c.eat("index") {
            let (pattern, field) = parse_index_spec(&mut c, "drop index on <pattern> (<field>)")?;
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

    /// `grant <priv> on all|workspace "<w>" to "<user>"` and the matching `revoke ... from`.
    fn cmd_grant(&mut self, a: &[Token], out: &mut Out, grant: bool) -> R {
        let usage = if grant {
            "grant read|write|admin on all|workspace \"<w>\" to \"<user>\""
        } else {
            "revoke read|write|admin [on all|workspace \"<w>\"] from \"<user>\""
        };
        let mut c = Cur::new(a);
        c.eat("privileges");
        let privilege = word(c.next(), "a privilege").map_err(|e| format!("{e}\nUsage: {usage}"))?.to_ascii_lowercase();
        // None = no scope given, Some("*") = all workspaces.
        let mut scope: Option<String> = None;
        if c.eat("on") {
            if c.eat("all") {
                scope = Some("*".into());
            } else if c.eat("workspace") {
                let w = word(c.next(), "a workspace name")?;
                if self.inst.cfg().workspace_dir(&w).is_none() {
                    return Err(format!("workspace '{w}' does not exist."));
                }
                scope = Some(w);
            } else {
                return Err(format!("Usage: {usage}"));
            }
        }
        if !c.eat(if grant { "to" } else { "from" }) {
            return Err(format!("Usage: {usage}"));
        }
        let name = word(c.next(), "a user name")?;
        c.finish(usage)?;
        if grant && scope.is_none() {
            return Err(format!("a grant needs a scope.\nUsage: {usage}"));
        }
        let mut user = auth::get_user(&self.inst.system, &name)?.ok_or(format!("user '{name}' does not exist."))?;
        if name == auth::DEFAULT_ADMIN {
            return Err("the privileges of the built-in kiva account cannot be changed.".into());
        }
        let where_ = match scope.as_deref() {
            Some("*") | None => "all workspaces".to_string(),
            Some(w) => format!("workspace '{w}'"),
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
                user.grants.insert(scope.clone().unwrap(), p);
                // Write access is meaningless for a read-only role.
                if p == Priv::Write && user.role == Role::Viewer {
                    user.role = Role::Editor;
                }
                out.say(format!("Granted: {privilege} on {where_} to '{name}'."));
            }
            ("write", false) => {
                match &scope {
                    Some(s) => {
                        if let Some(p) = user.grants.get_mut(s) {
                            *p = Priv::Read;
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
                match &scope {
                    Some(s) => {
                        user.grants.remove(s);
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
        const USAGE: &str = "Usage: modify user \"<name>\" password \"<new>\" | modify user \"<name>\" role \"<role>\"";
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
            user.set_password(&password);
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
            self.inst.ensure_workspace(&user.home_workspace())?;
            out.say(format!("Role of '{name}' set to [{}].", role.name()));
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
            if s == "storage" && k == "default_workspace" {
                std::fs::create_dir_all(&value).map_err(|e| format!("cannot use '{value}': {e}"))?;
            }
            self.inst.cfg_mut().set(&s, &k, &value)?;
            out.say(format!("OK: {s}.{k} = {}", mask(&k, value)));
            match (s.as_str(), k.as_str()) {
                ("storage", "auto_compact" | "compact_threshold" | "compression") => self.inst.apply_storage_config(),
                ("security", _) | ("http", "rate_limit" | "cors") | ("scripting", _) => {}
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

    fn cmd_workspace(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: workspace list | current | switch <name> | create <name> [path] | remove <name>";
        let mut c = Cur::new(a);
        if c.eat("list") {
            let me = match (&self.user, self.role()) {
                (Some(u), Some(r)) if r != Role::Admin => auth::get_user(&self.inst.system, &u.name)?,
                _ => None,
            };
            for (name, path) in self.inst.workspaces() {
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
            out.say(format!("{} ({})", self.ws, self.engine.path().display()));
            Ok(())
        } else if c.eat("switch") || c.eat("use") {
            let name = word(c.next(), "a workspace name")?;
            c.finish(USAGE)?;
            self.switch_workspace(&name)?;
            out.say(format!("Switched to workspace '{name}' ({} keys).", self.engine.len()));
            Ok(())
        } else if c.eat("create") {
            let name = word(c.next(), "a workspace name")?;
            let path = c.next().map(|t| t.text.clone());
            c.finish(USAGE)?;
            let dir = self.inst.create_workspace(&name, path.as_deref())?;
            out.say(format!("Workspace '{name}' created at {}.", dir.display()));
            Ok(())
        } else if c.eat("remove") {
            let name = word(c.next(), "a workspace name")?;
            c.finish(USAGE)?;
            if name == "default" {
                return Err("the default workspace cannot be removed.".into());
            }
            if name == self.ws {
                return Err("you cannot remove the workspace you are in.".into());
            }
            if !self.inst.cfg_mut().remove("workspaces", &name)? {
                return Err(format!("workspace '{name}' does not exist."));
            }
            out.say(format!("Workspace '{name}' unregistered. Its data files were left on disk."));
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
        let [to, target] = a else {
            return Err("Usage: failover to <replica id or address>".into());
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
            let source = std::fs::read_to_string(&file).map_err(|e| format!("cannot read '{file}': {e}"))?;
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
fn parse_index_spec(c: &mut Cur, usage: &str) -> Result<(String, String), String> {
    if !c.eat("on") {
        return Err(format!("Usage: {usage}"));
    }
    let mut pattern = word(c.next(), "a key pattern")?;
    if !c.next().is_some_and(|t| t.kind == Kind::LParen) {
        return Err(format!("Usage: {usage}"));
    }
    let field = word(c.next(), "a field name")?;
    if !c.next().is_some_and(|t| t.kind == Kind::RParen) {
        return Err(format!("Usage: {usage}"));
    }
    c.finish(usage)?;
    if !pattern.contains(['*', '?']) {
        pattern.push('*');
    }
    Ok((pattern, field.trim_start_matches('.').to_string()))
}
