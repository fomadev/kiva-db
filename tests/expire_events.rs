// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot E of docs/architecture/PROJECTS_AND_COLLECTIONS.md (section 6, decision D6):
//! a key whose TTL ran out gives a change `expire` when the engine sees it
//! gone, with `expires_at`, once per key and expiry, and after a stop that
//! kept it from the journal, at the next opening.

mod common;

use common::{instance, ok, TempDir};
use kivadb::changes::{ChangeKind, Settings};
use kivadb::engine::{ChangesFrom, Engine, EngineOptions};
use kivadb::session::Session;
use kivadb::value::Value;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn engine(path: &Path) -> Arc<Engine> {
    Engine::open(path.join("store.kiva"), EngineOptions { background: false, ..Default::default() }).unwrap()
}

/// `(op, key)` of every change, with `expires_at` for the expiries.
fn summary(e: &Engine) -> Vec<(String, String, i64)> {
    let r = e.read_changes(ChangesFrom::Beginning, 1000, None, None).unwrap();
    r.changes
        .iter()
        .map(|c| {
            let op = match c.kind {
                ChangeKind::Set => "set",
                ChangeKind::Del => "delete",
                ChangeKind::Reset => "reset",
                ChangeKind::Expire => "expire",
            };
            (op.to_string(), c.key.clone(), if c.kind == ChangeKind::Expire { c.expires_at } else { 0 })
        })
        .collect()
}

fn expiries(e: &Engine) -> Vec<(String, i64)> {
    summary(e).into_iter().filter(|(op, _, _)| op == "expire").map(|(_, k, at)| (k, at)).collect()
}

fn s(v: &str) -> Value {
    Value::Str(v.into())
}

/// A deadline already past, in milliseconds.
fn past() -> i64 {
    kivadb::util::now_ms() - 1
}

#[test]
fn an_expiry_seen_by_a_read_is_a_change_with_its_deadline() {
    let dir = TempDir::new("expire_read");
    let e = engine(dir.path());
    e.enable_changes(Settings::default()).unwrap();
    let at = kivadb::util::now_ms() + 150;
    e.put("session:1", &s("abc"), at, false).unwrap();
    e.put("keep", &s("v"), 0, false).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert!(e.get("session:1").unwrap().is_none());
    // Read again: seen once, journaled once.
    assert!(e.get("session:1").unwrap().is_none());
    assert_eq!(expiries(&e), [("session:1".to_string(), at)]);
    let ops: Vec<String> = summary(&e).into_iter().map(|(op, k, _)| format!("{op} {k}")).collect();
    assert_eq!(ops, ["set session:1", "set keep", "expire session:1"]);
    // As a reader gets it: `op` expire, its deadline, no value.
    let r = e.read_changes(ChangesFrom::Beginning, 10, Some("session:*"), None).unwrap();
    let last = r.changes.last().unwrap();
    let j = kivadb::changes::to_json(last, "");
    assert_eq!(j["op"], "expire");
    assert_eq!(j["key"], "session:1");
    assert!(j["value"].is_null() && j["type"].is_null());
    let deadline = chrono::DateTime::from_timestamp_millis(at).unwrap();
    assert_eq!(j["expires_at"], json!(kivadb::time::iso(deadline)));
}

#[test]
fn the_sweep_journals_expiries_nobody_reads() {
    let dir = TempDir::new("expire_sweep");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "changes enable");
    ok(&mut s, "set token:1 1 ttl 1");
    ok(&mut s, "set token:2 2 ttl 1");
    // The sweep runs every second: wait for both without reading them.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while expiries(&s.engine).len() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let seen: Vec<String> = expiries(&s.engine).into_iter().map(|(k, _)| k).collect();
    assert_eq!(seen, ["token:1", "token:2"]);
    // The shell names it.
    let read = ok(&mut s, "changes read");
    assert!(read.contains("expire token:1 (its TTL ran out at "), "{read}");
}

#[test]
fn an_expiry_is_journaled_once_across_restarts_and_after_a_stop_that_lost_it() {
    let dir = TempDir::new("expire_restart");
    let journaled;
    {
        let e = engine(dir.path());
        e.enable_changes(Settings::default()).unwrap();
        let at = kivadb::util::now_ms() + 100;
        e.put("a", &s("1"), at, false).unwrap();
        e.put("b", &s("2"), 0, false).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(e.get("a").unwrap().is_none());
        journaled = expiries(&e);
        assert_eq!(journaled.len(), 1);
        e.close();
    }
    // The key is still in the data file: the opening finds it expired, and
    // knows the journal holds it already.
    {
        let e = engine(dir.path());
        assert_eq!(expiries(&e), journaled, "not journaled twice");
        e.close();
    }
    // An expiry no read and no sweep saw before the stop: journaled at the opening.
    let late;
    {
        let e = engine(dir.path());
        late = kivadb::util::now_ms() + 100;
        e.put("c", &s("3"), late, false).unwrap();
        e.close();
    }
    std::thread::sleep(Duration::from_millis(200));
    let e = engine(dir.path());
    let all = expiries(&e);
    assert_eq!(all, [journaled[0].clone(), ("c".to_string(), late)]);
    drop(e);
    let e = engine(dir.path());
    assert_eq!(expiries(&e), all, "and once only");
}

#[test]
fn without_a_journal_nothing_is_written_and_a_new_expiry_of_the_same_key_counts() {
    let dir = TempDir::new("expire_again");
    let e = engine(dir.path());
    e.put("x", &s("1"), past(), false).unwrap();
    assert!(e.get("x").unwrap().is_none());
    e.enable_changes(Settings::default()).unwrap();
    assert!(expiries(&e).is_empty(), "the journal starts empty");
    // The same key, written again and expired again: a second expiry.
    for _ in 0..2 {
        let at = kivadb::util::now_ms() + 50;
        e.put("x", &s("1"), at, false).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(e.get("x").unwrap().is_none());
    }
    assert_eq!(expiries(&e).len(), 2);
}

#[test]
fn an_expiry_is_outside_transactions_and_answered_by_the_api() {
    let dir = TempDir::new("expire_txn");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "changes enable");
    ok(&mut s, "set t 1 ttl 1");
    std::thread::sleep(Duration::from_millis(1100));
    ok(&mut s, "begin");
    ok(&mut s, "set other 1");
    let _ = common::run(&mut s, "get t");
    ok(&mut s, "commit");
    let r = s.engine.read_changes(ChangesFrom::Beginning, 100, None, None).unwrap();
    let expire = r.changes.iter().find(|c| c.kind == ChangeKind::Expire).expect("an expiry");
    assert_eq!(expire.txn, None, "{expire:?}");
    assert!(!expire.txn_end);
    // Structured requests answer it with the others.
    let api = kivadb::api::handle(&mut s, &json!({"op": "changes", "cursor": "beginning"})).unwrap();
    let ops: Vec<&str> = api["changes"].as_array().unwrap().iter().map(|c| c["op"].as_str().unwrap()).collect();
    assert!(ops.contains(&"expire"), "{ops:?}");
}

#[test]
fn a_replica_journals_the_expiries_it_sees_itself() {
    let dir = TempDir::new("expire_replica");
    let replica = engine(&dir.path().join("r"));
    replica.enable_changes(Settings::default()).unwrap();
    let at = kivadb::util::now_ms() + 100;
    // What a master sends: the value with its deadline, no expiry.
    replica.put_raw("s", kivadb::value::Type::String, b"1", kivadb::value::DocEnc::Json, 0, at).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(replica.get("s").unwrap().is_none());
    assert_eq!(expiries(&replica), [("s".to_string(), at)]);
}
