// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot H of docs/architecture/PROJECTS_AND_COLLECTIONS.md (sections 9 and 10):
//! Prometheus counts commands by kind with their durations, expiries,
//! transaction conflicts, query strategies, the semantic cache and failed
//! backups; `stats` shows the same counts; and every error of the shell
//! starts by saying what went wrong.

mod common;

use common::{instance, ok, run, TempDir};
use kivadb::engine::BatchOp;
use kivadb::http;
use kivadb::session::Session;
use kivadb::value::Value;
use std::io::{Read, Write};
use std::net::TcpStream;

fn metrics(port: u16) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    raw.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
}

/// The value of the series `name` (with its labels), as printed.
fn value(m: &str, name: &str) -> f64 {
    m.lines()
        .find_map(|l| l.strip_prefix(name).and_then(|rest| rest.strip_prefix(' ')))
        .unwrap_or_else(|| panic!("no {name} in:\n{m}"))
        .parse()
        .unwrap()
}

#[test]
fn prometheus_counts_commands_by_kind_and_what_the_engine_sees() {
    let dir = TempDir::new("metrics_kinds");
    let inst = instance(&dir);
    let port = http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set user:1 {"city": "Goma", "n": 1}"#);
    ok(&mut s, r#"set user:2 {"city": "Bukavu", "n": 2}"#);
    ok(&mut s, "get user:1");
    ok(&mut s, r#"get where user:*.city = "Goma""#);
    ok(&mut s, r#"get where user:1.city = "Goma""#);
    ok(&mut s, "count where user:*.n > 0");
    ok(&mut s, "use default");
    ok(&mut s, "begin");
    ok(&mut s, "rollback");
    let _ = run(&mut s, "frobnicate");
    // A failed backup: the archive would replace a folder.
    let taken = dir.path().join("taken.tar.gz");
    std::fs::create_dir_all(&taken).unwrap();
    let failed = run(&mut s, &format!("backup to \"{}\"", taken.display()));
    assert!(failed.starts_with("Error: "), "{failed}");
    // An expiry, seen by a read.
    s.engine.put("t", &Value::Str("x".into()), kivadb::util::now_ms() - 1, false).unwrap();
    let _ = run(&mut s, "get t");
    // A conflict: a watched key written by someone else before the batch.
    let watched = s.engine.watch(&["w".to_string()]);
    s.engine.put("w", &Value::Str("1".into()), 0, false).unwrap();
    let op = BatchOp::Put { key: "w".into(), value: Value::Str("2".into()), expires_at: 0, msgpack: false };
    assert!(s.engine.apply_batch_watched(vec![op], None, Some(&watched)).is_err());

    let m = metrics(port);
    assert!(m.contains("# TYPE kiva_commands_total counter"), "{m}");
    assert!(value(&m, "kiva_commands_total{command=\"set\"}") >= 2.0);
    assert!(value(&m, "kiva_commands_total{command=\"get\"}") >= 2.0);
    assert!(value(&m, "kiva_commands_total{command=\"query\"}") >= 2.0);
    assert!(value(&m, "kiva_commands_total{command=\"aggregate\"}") >= 1.0);
    assert!(value(&m, "kiva_commands_total{command=\"transaction\"}") >= 2.0);
    assert!(value(&m, "kiva_commands_total{command=\"collection\"}") >= 1.0);
    assert!(value(&m, "kiva_commands_total{command=\"other\"}") >= 1.0);
    assert!(value(&m, "kiva_commands_total{command=\"admin\"}") >= 1.0);
    // A histogram per kind, for histogram_quantile().
    assert!(m.contains("# TYPE kiva_command_duration_seconds histogram"));
    let all = value(&m, "kiva_command_duration_seconds_bucket{command=\"query\",le=\"+Inf\"}");
    assert_eq!(all, value(&m, "kiva_command_duration_seconds_count{command=\"query\"}"));
    // The old series stay as they were, for the dashboards that read them.
    assert!(m.contains("kiva_requests_total{type=\"get\"}"));
    assert!(m.contains("kiva_request_duration_seconds_bucket{le=\"+Inf\"}"));
    // What the engine saw.
    assert_eq!(value(&m, "kiva_expired_keys_total"), 1.0);
    assert_eq!(value(&m, "kiva_transaction_conflicts_total"), 1.0);
    assert!(value(&m, "kiva_index_scans_total{strategy=\"prefix_scan\"}") >= 2.0, "{m}");
    assert!(value(&m, "kiva_index_scans_total{strategy=\"key_lookup\"}") >= 1.0);
    assert_eq!(value(&m, "kiva_vector_index_bytes"), 0.0);
    assert_eq!(value(&m, "kiva_semantic_cache_hits_total"), 0.0);
    assert_eq!(value(&m, "kiva_semantic_cache_misses_total"), 0.0);
    assert_eq!(value(&m, "kiva_semantic_cache_invalidations_total"), 0.0);
    assert_eq!(value(&m, "kiva_backup_failures_total"), 1.0);
    // No key and no user ever becomes a label.
    assert!(!m.contains("user:1") && !m.contains("kiva\"}"));

    // `stats` shows the same counts.
    let stats = ok(&mut s, "stats");
    assert!(stats.contains(" > Expired Keys         : 1 since opening"), "{stats}");
    assert!(stats.contains(" > Txn Conflicts        : 1"), "{stats}");
}

#[test]
fn every_error_starts_by_saying_what_went_wrong() {
    let dir = TempDir::new("errors_form");
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    // A command that gave back its usage alone.
    let out = run(&mut s, "collection frobnicate");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "Error: 'collection' is incomplete, or has a word it does not know.", "{out}");
    assert!(lines[1].starts_with("Usage: collection list"), "{out}");
    assert_eq!(lines.len(), 2, "{out}");
    // The others keep their own first line.
    let out = run(&mut s, "get k extra words");
    assert!(out.starts_with("Error: ") && !out.starts_with("Error: Usage"), "{out}");
    for cmd in ["workspace", "backup", "project frob", "changes frob", "use", "export", "import from"] {
        let out = run(&mut s, cmd);
        assert!(out.starts_with("Error: ") && !out.starts_with("Error: Usage"), "{cmd}: {out}");
        assert!(out.matches("Usage:").count() <= 1, "{cmd}: {out}");
    }
}
