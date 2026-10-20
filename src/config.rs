// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! `kiva.conf`: the central INI configuration file.

use crate::error::{KivaError, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// (section, key, default, description). `default_workspace` is resolved
/// against the home directory at load time.
pub const KNOWN: &[(&str, &str, &str, &str)] = &[
    ("storage", "default_workspace", "", "Directory of the default workspace"),
    ("storage", "auto_compact", "true", "Compact automatically when fragmentation is high"),
    ("storage", "compact_threshold", "30", "Fragmentation percentage that triggers auto-compaction"),
    ("storage", "compression", "none", "Value compression: none, lz4 or zlib"),
    ("storage", "sync_writes", "false", "fsync after every write (safer, slower)"),
    ("storage", "snapshot_interval", "300", "Seconds between index snapshots"),
    ("storage", "snapshot_every", "1000", "Modifications between index snapshots"),
    ("storage", "cache_values", "false", "Also keep recently read values in memory"),
    ("storage", "cache_size_mb", "256", "Memory budget of the value cache, per workspace"),
    ("network", "port", "2911", "TCP port of the server"),
    ("network", "bind", "127.0.0.1", "Address the server listens on"),
    ("security", "auth_required", "false", "Require a username and password"),
    ("server", "threads", "8", "Commands executed in parallel"),
    ("server", "max_clients", "1000", "Maximum simultaneous connections"),
    ("http", "enabled", "true", "Expose the REST API when serving"),
    ("http", "stream", "true", "Offer key changes as Server-Sent Events on /v1/stream"),
    ("http", "port", "8080", "HTTP port of the REST API"),
    ("http", "rate_limit", "600", "Requests per minute per IP (0 = unlimited)"),
    ("http", "cors", "true", "Send CORS headers"),
    ("resp", "enabled", "true", "Speak the Redis protocol (RESP) when serving"),
    ("resp", "port", "6379", "Port of the Redis-compatible listener"),
    ("tls", "enabled", "true", "Accept encrypted (TLS) connections on every port"),
    ("tls", "required", "false", "Refuse unencrypted connections from other machines"),
    ("tls", "cert", "", "Certificate file (PEM); empty = one created in <home>/tls"),
    ("tls", "key", "", "Private key file (PEM) of that certificate"),
    ("tls", "hosts", "", "Extra names or IP addresses for the created certificate"),
    ("replication", "role", "master", "master or replica"),
    ("replication", "master_host", "", "Host of the master (replica only)"),
    ("replication", "master_port", "2911", "Port of the master (replica only)"),
    ("replication", "master_user", "kiva", "User the replica authenticates as"),
    ("replication", "master_password", "", "Password the replica authenticates with"),
    ("replication", "master_tls", "false", "Connect to the master with TLS"),
    ("replication", "master_fingerprint", "", "SHA-256 fingerprint of the master's certificate"),
    ("replication", "sync_mode", "async", "async or sync"),
    ("replication", "min_slaves_ack", "1", "Replica acknowledgements required in sync mode"),
    ("scripting", "timeout_ms", "5000", "Maximum run time of a Lua script"),
    ("scripting", "memory_limit_mb", "64", "Maximum memory of a Lua script"),
];

pub struct Config {
    path: PathBuf,
    home: PathBuf,
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

/// The KivaDB home directory: `$KIVADB_HOME`, else `%APPDATA%\KivaDB` on
/// Windows and `~/.kivadb` elsewhere.
pub fn default_home() -> PathBuf {
    if let Some(h) = std::env::var_os("KIVADB_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    if cfg!(windows) {
        if let Some(a) = std::env::var_os("APPDATA") {
            return PathBuf::from(a).join("KivaDB");
        }
    }
    match std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        Some(h) => PathBuf::from(h).join(".kivadb"),
        None => PathBuf::from(".kivadb"),
    }
}

fn validate(key: &str, value: &str) -> std::result::Result<(), String> {
    let is_bool = || matches!(value, "true" | "false");
    let int_in = |lo: u64, hi: u64| value.parse::<u64>().map(|n| n >= lo && n <= hi).unwrap_or(false);
    let ok = match key {
        "auto_compact" | "sync_writes" | "auth_required" | "enabled" | "cors" | "cache_values" | "stream"
        | "required" | "master_tls" => is_bool(),
        "master_fingerprint" => value.is_empty() || crate::tls::normalize_fingerprint(value).is_some(),
        "cache_size_mb" => int_in(1, 1_048_576),
        "compact_threshold" => int_in(1, 99),
        "compression" => matches!(value, "none" | "lz4" | "zlib"),
        "port" | "master_port" => int_in(1, 65535),
        "threads" => int_in(1, 1024),
        "max_clients" => int_in(1, 1_000_000),
        "rate_limit" | "min_slaves_ack" => int_in(0, 1_000_000),
        "snapshot_interval" | "snapshot_every" | "timeout_ms" | "memory_limit_mb" => int_in(1, u32::MAX as u64),
        "role" => matches!(value, "master" | "replica" | "slave"),
        "sync_mode" => matches!(value, "async" | "sync"),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("invalid value '{value}' for '{key}'"))
    }
}

impl Config {
    /// Loads `kiva.conf` from `home`, creating it with defaults on first run.
    pub fn load(home: &Path) -> Result<Config> {
        fs::create_dir_all(home)?;
        let path = home.join("kiva.conf");
        let mut cfg = Config { path: path.clone(), home: home.to_path_buf(), sections: BTreeMap::new() };
        match fs::read_to_string(&path) {
            Ok(text) => cfg.parse(&text),
            Err(_) => {
                for (s, k, d, _) in KNOWN {
                    let value = cfg.default_of(k, d);
                    cfg.sections.entry(s.to_string()).or_default().insert(k.to_string(), value);
                }
                cfg.sections.entry("workspaces".into()).or_default();
                cfg.save()?;
            }
        }
        Ok(cfg)
    }

    fn default_of(&self, key: &str, default: &str) -> String {
        if key == "default_workspace" {
            self.home.join("data").display().to_string()
        } else {
            default.to_string()
        }
    }

    fn parse(&mut self, text: &str) {
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                self.sections.entry(section.clone()).or_default();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let (k, mut v) = (k.trim(), v.trim());
                // Strip an inline comment and surrounding quotes.
                if let Some(i) = v.find(" #") {
                    v = v[..i].trim();
                }
                let v = v.trim_matches('"');
                self.sections.entry(section.clone()).or_default().insert(k.to_string(), v.to_string());
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let mut out = String::from("# KivaDB configuration\n");
        let order = [
            "storage",
            "workspaces",
            "network",
            "security",
            "server",
            "http",
            "resp",
            "tls",
            "replication",
            "scripting",
        ];
        let mut names: Vec<&String> = self.sections.keys().collect();
        names.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(order.len()));
        for name in names {
            out.push_str(&format!("\n[{name}]\n"));
            for (k, v) in &self.sections[name] {
                out.push_str(&format!("{k} = {v}\n"));
            }
        }
        fs::write(&self.path, out)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn get(&self, section: &str, key: &str) -> String {
        if let Some(v) = self.sections.get(section).and_then(|s| s.get(key)) {
            return v.clone();
        }
        KNOWN
            .iter()
            .find(|(s, k, _, _)| *s == section && *k == key)
            .map(|(_, k, d, _)| self.default_of(k, d))
            .unwrap_or_default()
    }

    pub fn get_bool(&self, section: &str, key: &str) -> bool {
        self.get(section, key) == "true"
    }

    pub fn get_u64(&self, section: &str, key: &str) -> u64 {
        let v = self.get(section, key);
        v.parse().unwrap_or_else(|_| {
            KNOWN
                .iter()
                .find(|(s, k, _, _)| *s == section && *k == key)
                .and_then(|(_, _, d, _)| d.parse().ok())
                .unwrap_or(0)
        })
    }

    /// Resolves `key` or `section.key` to a known `(section, key)` pair.
    pub fn resolve(&self, name: &str) -> Result<(String, String)> {
        let name = name.trim();
        if let Some((s, k)) = name.split_once('.') {
            let s = s.to_ascii_lowercase();
            if s == "workspaces" || KNOWN.iter().any(|(ks, kk, _, _)| *ks == s && *kk == k) {
                return Ok((s, k.to_string()));
            }
            return Err(KivaError::NotFound(format!("unknown configuration key '{name}'")));
        }
        // Accept the alias used in older notes.
        let name = if name == "default_path" { "default_workspace" } else { name };
        let hits: Vec<_> = KNOWN.iter().filter(|(_, k, _, _)| *k == name).collect();
        match hits.len() {
            0 => Err(KivaError::NotFound(format!("unknown configuration key '{name}'"))),
            1 => Ok((hits[0].0.to_string(), name.to_string())),
            _ => Err(KivaError::Invalid(format!(
                "'{name}' exists in several sections; use one of: {}",
                hits.iter().map(|h| format!("{}.{}", h.0, h.1)).collect::<Vec<_>>().join(", ")
            ))),
        }
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) -> Result<()> {
        validate(key, value).map_err(KivaError::Invalid)?;
        self.sections.entry(section.to_string()).or_default().insert(key.to_string(), value.to_string());
        self.save()
    }

    pub fn remove(&mut self, section: &str, key: &str) -> Result<bool> {
        let removed = self.sections.get_mut(section).map(|s| s.remove(key).is_some()).unwrap_or(false);
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Every setting as `(section.key, value)`, defaults included.
    pub fn list(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> =
            KNOWN.iter().map(|(s, k, _, _)| (format!("{s}.{k}"), self.get(s, k))).collect();
        for (name, path) in self.workspaces() {
            out.push((format!("workspaces.{name}"), path));
        }
        out
    }

    /// Named workspaces from the `[workspaces]` section.
    pub fn workspaces(&self) -> Vec<(String, String)> {
        self.sections
            .get("workspaces")
            .map(|s| s.iter().filter(|(k, _)| k.as_str() != "default").map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    pub fn workspace_dir(&self, name: &str) -> Option<PathBuf> {
        if name == "default" {
            return Some(PathBuf::from(self.get("storage", "default_workspace")));
        }
        self.sections.get("workspaces").and_then(|s| s.get(name)).map(PathBuf::from)
    }
}
