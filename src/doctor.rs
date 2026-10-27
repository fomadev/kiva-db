// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! `kivadb doctor`: what could stop KivaDB from starting, from being reached
//! or from keeping its data, found by reading the home directory. It never
//! writes there (apart from one test file it removes at once), works whether
//! KivaDB is running or not, and needs no password.

use crate::config::Config;
use crate::instance::VERSION;
use crate::util::is_loopback;
use serde_json::{json, Value as Json};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warning",
            Level::Fail => "problem",
        }
    }
}

#[derive(Debug)]
pub struct Finding {
    pub level: Level,
    pub topic: &'static str,
    pub message: String,
    /// What to do about it.
    pub advice: Option<String>,
}

/// The running KivaDB on this home, as `kivadb status` sees it: its TCP port and process id.
pub type Running = Option<(u16, String)>;

pub struct Report {
    pub home: PathBuf,
    pub findings: Vec<Finding>,
}

impl Report {
    fn add(&mut self, level: Level, topic: &'static str, message: impl Into<String>, advice: Option<String>) {
        self.findings.push(Finding { level, topic, message: message.into(), advice });
    }

    fn count(&self, level: Level) -> usize {
        self.findings.iter().filter(|f| f.level == level).count()
    }

    /// 0 when nothing is wrong (warnings included), 1 when a problem was found.
    pub fn exit_code(&self) -> i32 {
        i32::from(self.count(Level::Fail) > 0)
    }

    pub fn text(&self) -> String {
        let mut out = format!("KivaDB {VERSION} doctor, home {}\n\n", self.home.display());
        for f in &self.findings {
            let tag = match f.level {
                Level::Ok => "OK  ",
                Level::Warn => "WARN",
                Level::Fail => "FAIL",
            };
            out.push_str(&format!("  {tag}  {:<11} {}\n", f.topic, f.message));
            if let Some(a) = &f.advice {
                out.push_str(&format!("        {:<11} -> {a}\n", ""));
            }
        }
        let (w, p) = (self.count(Level::Warn), self.count(Level::Fail));
        out.push_str(&format!(
            "\n{} check(s) passed, {w} warning{}, {p} problem{}.\n",
            self.count(Level::Ok),
            if w == 1 { "" } else { "s" },
            if p == 1 { "" } else { "s" }
        ));
        out
    }

    pub fn json(&self) -> Json {
        json!({
            "version": VERSION,
            "home": self.home.display().to_string(),
            "findings": self.findings.iter().map(|f| json!({
                "level": f.level.name(),
                "topic": f.topic,
                "message": f.message,
                "advice": f.advice,
            })).collect::<Vec<_>>(),
            "summary": {
                "ok": self.count(Level::Ok),
                "warnings": self.count(Level::Warn),
                "problems": self.count(Level::Fail),
            },
        })
    }
}

/// Examines `home`. `running` says whether a KivaDB process holds it.
pub fn examine(home: &Path, running: Running) -> Report {
    let mut r = Report { home: home.to_path_buf(), findings: Vec::new() };
    if !home.join("kiva.conf").is_file() {
        // Loading the configuration would create it: a doctor does not.
        r.add(
            Level::Warn,
            "Home",
            format!("{} holds no KivaDB configuration (kiva.conf)", home.display()),
            Some("it is created the first time KivaDB starts here; check --home or KIVADB_HOME".into()),
        );
        return r;
    }
    home_writable(&mut r, home);
    disk_space(&mut r, home);
    let cfg = match Config::load(home) {
        Ok(c) => c,
        Err(e) => {
            r.add(Level::Fail, "Config", format!("kiva.conf cannot be read: {e}"), None);
            return r;
        }
    };
    server(&mut r, &cfg, &running);
    exposure(&mut r, &cfg);
    tls(&mut r, home, &cfg);
    encryption(&mut r, &cfg);
    time_zone(&mut r, &cfg);
    replication(&mut r, &cfg);
    data(&mut r, home, &cfg);
    server_log(&mut r, home);
    r
}

fn home_writable(r: &mut Report, home: &Path) {
    let probe = home.join(format!(".doctor-{:016x}", crate::util::random_u64()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            r.add(Level::Ok, "Home", format!("{} is writable", home.display()), None);
        }
        Err(e) => r.add(
            Level::Fail,
            "Home",
            format!("{} cannot be written: {e}", home.display()),
            Some("KivaDB must run as a user who owns this folder".into()),
        ),
    }
}

/// Below this, writes and compactions are at risk (a compaction rewrites the live data).
const LOW_DISK: u64 = 1 << 30;

fn disk_space(r: &mut Report, home: &Path) {
    let Some(free) = crate::util::free_disk_bytes(home) else {
        r.add(Level::Warn, "Disk", "the free space of this disk could not be measured", None);
        return;
    };
    let data = folder_size(&home.join("data")) + folder_size(&home.join("workspaces"));
    let text = format!("{} free, data takes {}", crate::util::human_size(free), crate::util::human_size(data));
    if free < LOW_DISK || free < data {
        r.add(
            Level::Warn,
            "Disk",
            text,
            Some("a compaction needs room for a copy of the live data: free some space".into()),
        );
    } else {
        r.add(Level::Ok, "Disk", text, None);
    }
}

fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
        })
        .sum()
}

fn ports(cfg: &Config) -> Vec<(&'static str, u16)> {
    let mut out = vec![("TCP", cfg.get_u64("network", "port") as u16)];
    if cfg.get_bool("http", "enabled") {
        out.push(("REST API", cfg.get_u64("http", "port") as u16));
    }
    if cfg.get_bool("resp", "enabled") {
        out.push(("Redis", cfg.get_u64("resp", "port") as u16));
    }
    out
}

fn server(r: &mut Report, cfg: &Config, running: &Running) {
    if let Some((port, pid)) = running {
        r.add(Level::Ok, "Server", format!("running (pid {pid}) on port {port}"), None);
        return;
    }
    r.add(Level::Ok, "Server", "not running", None);
    let bind = cfg.get("network", "bind");
    let bind = if bind.is_empty() { "127.0.0.1".to_string() } else { bind };
    for (name, port) in ports(cfg) {
        if let Err(e) = TcpListener::bind((bind.as_str(), port)) {
            r.add(
                Level::Warn,
                "Ports",
                format!("the {name} port {bind}:{port} is taken by another program ({e})"),
                Some(format!(
                    "stop that program, or choose another port (kivadb start --port, config set ...port); see {}",
                    cfg.path().display()
                )),
            );
            return;
        }
    }
    r.add(Level::Ok, "Ports", format!("{} free on {bind}", list_ports(cfg)), None);
}

fn list_ports(cfg: &Config) -> String {
    ports(cfg).iter().map(|(n, p)| format!("{n} {p}")).collect::<Vec<_>>().join(", ")
}

fn exposure(r: &mut Report, cfg: &Config) {
    let bind = cfg.get("network", "bind");
    let open = !bind.is_empty() && !is_loopback(&bind);
    let auth = cfg.get_bool("security", "auth_required");
    match (open, auth) {
        (true, false) => r.add(
            Level::Fail,
            "Access",
            format!("KivaDB listens on {bind}, open to other machines, without asking for a password"),
            Some("config set security.auth_required true, after giving kiva a password (modify user \"kiva\" password ...)".into()),
        ),
        (true, true) => r.add(Level::Ok, "Access", format!("open to other machines on {bind}, passwords required"), None),
        (false, true) => r.add(Level::Ok, "Access", "this machine only, passwords required", None),
        (false, false) => r.add(Level::Ok, "Access", "this machine only (no password asked)", None),
    }
}

fn tls(r: &mut Report, home: &Path, cfg: &Config) {
    let open = !is_loopback(&cfg.get("network", "bind"));
    if !cfg.get_bool("tls", "enabled") {
        let level = if open { Level::Warn } else { Level::Ok };
        let advice =
            open.then(|| "config set tls.enabled true: passwords and data cross the network in clear".to_string());
        r.add(level, "TLS", "off", advice);
        return;
    }
    let cert = cfg.get("tls", "cert");
    let (cert, key) = if cert.trim().is_empty() {
        (home.join("tls").join("cert.pem"), home.join("tls").join("key.pem"))
    } else {
        (PathBuf::from(cert.trim()), PathBuf::from(cfg.get("tls", "key").trim()))
    };
    if !cert.exists() && cfg.get("tls", "cert").trim().is_empty() {
        r.add(Level::Ok, "TLS", "on; the certificate is created at the first start", None);
        return;
    }
    for f in [&cert, &key] {
        if !f.is_file() {
            r.add(
                Level::Fail,
                "TLS",
                format!("{} is missing", f.display()),
                Some("fix tls.cert and tls.key, or empty them to let KivaDB create a certificate".into()),
            );
            return;
        }
    }
    match crate::tls::load_server(home, cfg) {
        Ok(t) => {
            let required = cfg.get_bool("tls", "required");
            let advice = (open && !required).then(|| {
                "config set tls.required true to refuse unencrypted connections from other machines".to_string()
            });
            r.add(
                if advice.is_some() { Level::Warn } else { Level::Ok },
                "TLS",
                format!("on, certificate {} (SHA-256 {})", t.cert_path.display(), t.fingerprint),
                advice,
            );
        }
        Err(e) => r.add(Level::Fail, "TLS", format!("the certificate cannot be used: {e}"), None),
    }
}

fn encryption(r: &mut Report, cfg: &Config) {
    match crate::crypt::Keys::load(cfg) {
        Ok(Some(k)) => r.add(Level::Ok, "Encryption", format!("at rest, key {}", k.id_hex()), None),
        Ok(None) => r.add(Level::Ok, "Encryption", "off (data files are not encrypted)", None),
        Err(e) => r.add(
            Level::Fail,
            "Encryption",
            e.to_string(),
            Some(format!("KivaDB will not start: give the key in {} or encryption.key_file", crate::crypt::KEY_ENV)),
        ),
    }
}

fn time_zone(r: &mut Report, cfg: &Config) {
    let name = cfg.get("time", "zone");
    match crate::time::Zone::parse(&name) {
        Ok(z) => {
            r.add(Level::Ok, "Time zone", format!("{}, now {}", z.name(), crate::time::iso(crate::time::now())), None)
        }
        Err(e) => r.add(
            Level::Fail,
            "Time zone",
            format!("time.zone = {name}: {e}"),
            Some("config set time.zone UTC (or a name such as Africa/Kinshasa)".into()),
        ),
    }
}

fn replication(r: &mut Report, cfg: &Config) {
    if cfg.get("replication", "role") != "replica" {
        let auto = if cfg.get_bool("failover", "auto") { ", automatic failover on" } else { "" };
        r.add(Level::Ok, "Replication", format!("master{auto}"), None);
        return;
    }
    let host = cfg.get("replication", "master_host");
    if host.is_empty() {
        r.add(
            Level::Fail,
            "Replication",
            "replica of no master (replication.master_host is empty)",
            Some("replica of <host> <port>, or replica of none".into()),
        );
        return;
    }
    let port = cfg.get("replication", "master_port");
    let open = !is_loopback(&host);
    let advice = (open && !cfg.get_bool("replication", "master_tls"))
        .then(|| "config set replication.master_tls true: the link carries every write".to_string());
    r.add(
        if advice.is_some() { Level::Warn } else { Level::Ok },
        "Replication",
        format!("replica of {host}:{port}"),
        advice,
    );
}

/// Every record of every workspace, read from the files with their checksums.
fn data(r: &mut Report, home: &Path, cfg: &Config) {
    let mut spaces = vec![("default".to_string(), PathBuf::from(cfg.get("storage", "default_workspace")))];
    spaces.extend(cfg.workspaces().into_iter().map(|(n, d)| (n, PathBuf::from(d))));
    spaces.push(("system".into(), home.join("system")));
    let (mut records, mut checked, mut empty, mut damaged) = (0u64, 0usize, 0usize, false);
    for (name, dir) in spaces {
        let file = if name == "system" { dir.join("system.kiva") } else { dir.join("store.kiva") };
        // A workspace nothing was written to has no file yet.
        if !file.exists() {
            empty += 1;
            continue;
        }
        match crate::backup::check_log(&file) {
            Ok(n) => {
                records += n;
                checked += 1;
            }
            // A last record cut short is a write interrupted by a crash: the
            // engine drops it when it opens the file, nothing else is lost.
            Err(e) if e.starts_with("truncated") => r.add(
                Level::Warn,
                "Data",
                format!("workspace '{name}': {e}"),
                Some("a write was interrupted (crash or power cut); KivaDB drops it at the next start".into()),
            ),
            Err(e) => {
                damaged = true;
                let how = if name == "system" {
                    "users and scripts live there: restore system/system.kiva from a backup".to_string()
                } else {
                    format!("open it (workspace switch {name}) and run check, then check repair")
                };
                r.add(
                    Level::Fail,
                    "Data",
                    format!("workspace '{name}': {e}"),
                    Some(format!("{how}; keep a copy of {} first", file.display())),
                );
            }
        }
    }
    if !damaged {
        let empty = if empty > 0 { format!(" ({empty} empty workspace(s))") } else { String::new() };
        r.add(
            Level::Ok,
            "Data",
            format!("{checked} file(s), {records} record(s), every checksum matches{empty}"),
            None,
        );
    }
}

/// Errors the server wrote in its log recently.
fn server_log(r: &mut Report, home: &Path) {
    let Ok(text) = std::fs::read_to_string(home.join("server.log")) else { return };
    let tail: Vec<&str> = text.lines().rev().take(500).collect();
    let errors: Vec<&&str> = tail.iter().filter(|l| l.contains("[ERROR]")).collect();
    match errors.first() {
        None => r.add(Level::Ok, "Server log", "no recent error", None),
        Some(last) => r.add(
            Level::Warn,
            "Server log",
            format!("{} error(s) in the last 500 lines; the latest: {}", errors.len(), last.trim()),
            Some(format!("read {}", home.join("server.log").display())),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_folder_is_not_a_home() {
        let dir = std::env::temp_dir().join(format!("kivadb_doctor_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let r = examine(&dir, None);
        assert!(!dir.join("kiva.conf").exists(), "the doctor must not create the configuration");
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.exit_code(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
