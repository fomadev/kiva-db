// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The change journal (change data capture): every committed write, in
//! order, read again from a cursor after a disconnection, a restart or a
//! crash, within its retention.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::changes::{ChangeKind, Settings};
use kivadb::engine::{ChangesFrom, Engine, EngineOptions};
use kivadb::instance::Instance;
use kivadb::session::Session;
use kivadb::value::{Type, Value};
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

fn engine(path: &Path) -> Arc<Engine> {
    Engine::open(path.join("store.kiva"), EngineOptions { background: false, ..Default::default() }).unwrap()
}

/// `(seq, op, key)` of every change after `from`.
fn summary(e: &Engine, from: ChangesFrom) -> Vec<(u64, String, String)> {
    let r = e.read_changes(from, 1000, None, None).unwrap();
    r.changes
        .iter()
        .map(|c| {
            let op = match c.kind {
                ChangeKind::Set if c.created == Some(true) => "insert",
                ChangeKind::Set if c.created == Some(false) => "update",
                ChangeKind::Set => "set",
                ChangeKind::Del => "delete",
                ChangeKind::Reset => "reset",
            };
            (c.seq, op.to_string(), c.key.clone())
        })
        .collect()
}

fn s(v: &str) -> Value {
    Value::Str(v.into())
}

fn row(seq: u64, op: &str, key: &str) -> (u64, String, String) {
    (seq, op.to_string(), key.to_string())
}

#[test]
fn inserts_updates_deletes_and_transactions_in_commit_order() {
    let dir = TempDir::new("cdc_basics");
    let inst = instance(&dir);
    let mut sh = Session::new(inst.clone(), true, None);
    ok(&mut sh, "set before 1");
    ok(&mut sh, "changes enable");
    ok(&mut sh, r#"set user:1 {"name": "Ana"}"#);
    ok(&mut sh, r#"update user:1 {"name": "Ana", "age": 31}"#);
    ok(&mut sh, "json set user:1 .age 32");
    ok(&mut sh, "del before");
    ok(&mut sh, "begin");
    ok(&mut sh, "set a 1");
    ok(&mut sh, "set b 2");
    ok(&mut sh, "del user:1");
    ok(&mut sh, "commit");
    // Rolled back: nothing reached the data, nothing reaches the journal.
    ok(&mut sh, "begin");
    ok(&mut sh, "set never 1");
    ok(&mut sh, "rollback");
    ok(&mut sh, "set date born '2004-05-09'");

    let e = inst.engine("default").unwrap();
    assert_eq!(
        summary(&e, ChangesFrom::Beginning),
        [
            row(1, "insert", "user:1"),
            row(2, "update", "user:1"),
            row(3, "update", "user:1"),
            row(4, "delete", "before"),
            row(5, "insert", "a"),
            row(6, "insert", "b"),
            row(7, "delete", "user:1"),
            row(8, "insert", "born"),
        ],
        "the write made before the journal started is not in it"
    );
    let r = e.read_changes(ChangesFrom::Beginning, 1000, None, None).unwrap();
    let txn: Vec<(Option<u64>, bool)> = r.changes.iter().map(|c| (c.txn, c.txn_end)).collect();
    assert_eq!(txn[4..7], [(Some(5), false), (Some(5), false), (Some(5), true)], "one transaction, closed by its last");
    let j = kivadb::changes::to_json(&r.changes[2], "c");
    assert_eq!((&j["op"], &j["value"]), (&json!("set"), &json!({"name": "Ana", "age": 32})));
    let born = kivadb::changes::to_json(&r.changes[7], "c");
    assert_eq!((&born["type"], &born["value"]), (&json!("date"), &json!("2004-05-09")));
    // A pattern filters, and the cursor still moves past what it left out.
    let only = e.read_changes(ChangesFrom::Beginning, 1000, Some("user:*"), None).unwrap();
    assert_eq!(only.changes.len(), 4);
    assert_eq!(only.cursor, r.cursor);

    // The shell shows them, with the cursor to go on from.
    let out = ok(&mut sh, "changes read limit 2");
    assert!(out.contains("#1 ") && out.contains("insert user:1 (object)") && out.contains("(more after it)"), "{out}");
    let cursor = out.lines().last().unwrap().split_whitespace().nth(1).unwrap().to_string();
    let next = ok(&mut sh, &format!("changes read from {cursor} limit 1"));
    assert!(next.starts_with("#3 "), "{next}");
}

#[test]
fn keys_only_keeps_no_value() {
    let dir = TempDir::new("cdc_keys_only");
    let e = engine(dir.path());
    e.enable_changes(Settings { values: false, ..Default::default() }).unwrap();
    e.put("secret", &s("hunter2"), 0, false).unwrap();
    let r = e.read_changes(ChangesFrom::Beginning, 10, None, None).unwrap();
    assert_eq!((r.changes[0].key.as_str(), r.changes[0].value.as_ref()), ("secret", None));
    let bytes: Vec<u8> = std::fs::read_dir(dir.path().join("store.kiva.changes"))
        .unwrap()
        .flatten()
        .flat_map(|f| std::fs::read(f.path()).unwrap())
        .collect();
    assert!(!bytes.windows(7).any(|w| w == b"hunter2"));
}

#[test]
fn a_reader_resumes_from_its_cursor_after_a_restart() {
    let dir = TempDir::new("cdc_restart");
    let cursor;
    {
        let e = engine(dir.path());
        e.enable_changes(Settings::default()).unwrap();
        for i in 0..5 {
            e.put(&format!("k{i}"), &s("v"), 0, false).unwrap();
        }
        let r = e.read_changes(ChangesFrom::Beginning, 3, None, None).unwrap();
        cursor = r.cursor;
        e.close();
    }
    let e = engine(dir.path());
    e.put("k5", &s("v"), 0, false).unwrap();
    let rest = summary(&e, ChangesFrom::Cursor(cursor));
    assert_eq!(rest, [row(4, "insert", "k3"), row(5, "insert", "k4"), row(6, "insert", "k5")]);
}

#[test]
fn changes_a_crash_kept_from_the_journal_are_rebuilt_from_the_data() {
    let dir = TempDir::new("cdc_crash_journal");
    let journal = dir.path().join("store.kiva.changes");
    {
        let e = engine(dir.path());
        e.enable_changes(Settings::default()).unwrap();
        e.put("a", &s("1"), 0, false).unwrap();
        e.read_changes(ChangesFrom::Beginning, 10, None, None).unwrap(); // synced so far
        e.put("b", &s("2"), 0, false).unwrap();
        e.apply_batch(
            vec![
                kivadb::engine::BatchOp::Put { key: "c".into(), value: s("3"), expires_at: 0, msgpack: false },
                kivadb::engine::BatchOp::Del { key: "a".into() },
            ],
            None,
        )
        .unwrap();
        e.close();
    }
    // The crash: the journal lost what came after "a", the data kept it.
    let seg = std::fs::read_dir(&journal).unwrap().flatten().next().unwrap().path();
    let data = std::fs::read(&seg).unwrap();
    let mut cut = 24;
    for _ in 0..2 {
        // The mark written at the start, then "a".
        let len = u32::from_le_bytes(data[cut..cut + 4].try_into().unwrap()) as usize;
        cut += 8 + len;
    }
    std::fs::write(&seg, &data[..cut]).unwrap();
    let e = engine(dir.path());
    assert_eq!(
        summary(&e, ChangesFrom::Beginning),
        [row(1, "insert", "a"), row(2, "set", "b"), row(3, "set", "c"), row(4, "delete", "a")],
        "rebuilt, with the same numbers, the transaction whole"
    );
    let r = e.read_changes(ChangesFrom::Beginning, 10, None, None).unwrap();
    assert_eq!((r.changes[2].txn, r.changes[3].txn_end), (Some(3), true));
    // And nothing twice after another restart.
    e.close();
    drop(e);
    assert_eq!(summary(&engine(dir.path()), ChangesFrom::Beginning).len(), 4);
}

#[test]
fn changes_whose_data_a_crash_lost_are_dropped() {
    let dir = TempDir::new("cdc_crash_data");
    let store = dir.path().join("store.kiva");
    {
        let e = engine(dir.path());
        e.enable_changes(Settings::default()).unwrap();
        e.put("kept", &s("1"), 0, false).unwrap();
        e.put("lost", &s("2"), 0, false).unwrap();
        e.close();
    }
    // The data file lost its last record (never synced), the journal kept it.
    let len = std::fs::metadata(&store).unwrap().len();
    std::fs::OpenOptions::new().write(true).open(&store).unwrap().set_len(len - 5).unwrap();
    let _ = std::fs::remove_file(dir.path().join("store.kiva.hint"));
    let e = engine(dir.path());
    assert_eq!(summary(&e, ChangesFrom::Beginning), [row(1, "insert", "kept")]);
    e.put("next", &s("3"), 0, false).unwrap();
    assert_eq!(summary(&e, ChangesFrom::Beginning)[1], row(2, "insert", "next"), "numbered after the last kept");
}

#[test]
fn a_transaction_cut_by_a_crash_gives_no_change() {
    let dir = TempDir::new("cdc_crash_txn");
    let store = dir.path().join("store.kiva");
    {
        let e = engine(dir.path());
        e.enable_changes(Settings::default()).unwrap();
        e.put("a", &s("1"), 0, false).unwrap();
        e.read_changes(ChangesFrom::Beginning, 10, None, None).unwrap();
        e.apply_batch(
            vec![
                kivadb::engine::BatchOp::Put { key: "t1".into(), value: s("x"), expires_at: 0, msgpack: false },
                kivadb::engine::BatchOp::Put { key: "t2".into(), value: s("y"), expires_at: 0, msgpack: false },
            ],
            None,
        )
        .unwrap();
        e.close();
    }
    // The commit record of the transaction never reached the disk, nor its changes.
    let len = std::fs::metadata(&store).unwrap().len();
    std::fs::OpenOptions::new().write(true).open(&store).unwrap().set_len(len - 3).unwrap();
    let _ = std::fs::remove_file(dir.path().join("store.kiva.hint"));
    let e = engine(dir.path());
    assert_eq!(summary(&e, ChangesFrom::Beginning), [row(1, "insert", "a")]);
    assert!(!e.contains("t1"));
}

#[test]
fn compaction_keeps_the_journal_and_its_cursors() {
    let dir = TempDir::new("cdc_compact");
    let cursor;
    {
        let e = engine(dir.path());
        e.enable_changes(Settings::default()).unwrap();
        for i in 0..50 {
            e.put("hot", &s(&format!("v{i}")), 0, false).unwrap();
        }
        cursor = e.read_changes(ChangesFrom::Beginning, 10, None, None).unwrap().cursor;
        e.compact().unwrap();
        e.put("after", &s("1"), 0, false).unwrap();
        assert_eq!(summary(&e, ChangesFrom::Beginning).len(), 51, "compaction removes old values, not changes");
        e.close();
    }
    let e = engine(dir.path());
    let rest = summary(&e, ChangesFrom::Cursor(cursor));
    assert_eq!(rest.len(), 41);
    assert_eq!(rest.last().unwrap(), &row(51, "insert", "after"));
}

#[test]
fn retention_removes_old_changes_and_their_cursors_expire() {
    let dir = TempDir::new("cdc_retention");
    let e = engine(dir.path());
    e.enable_changes(Settings { values: true, retain_secs: 3600, max_bytes: 1 << 20 }).unwrap();
    let old = e.read_changes(ChangesFrom::Beginning, 1, None, None).unwrap().cursor;
    let big = "x".repeat(200_000);
    // A consumer that never reads holds nothing back: the writes go on.
    for i in 0..40 {
        e.put(&format!("k{i}"), &s(&big), 0, false).unwrap();
    }
    e.trim_changes();
    let status = e.changes_status().unwrap();
    assert!(status.first_seq > 1 && status.bytes <= 3 << 20, "{status:?}");
    let err = match e.read_changes(ChangesFrom::Cursor(old), 10, None, None) {
        Err(err) => err.to_string(),
        Ok(_) => panic!("an expired cursor was accepted"),
    };
    assert!(err.starts_with("CURSOR_EXPIRED:"), "{err}");
    assert!(!summary(&e, ChangesFrom::Beginning).is_empty(), "the newest changes are kept");
}

#[test]
fn invalid_cursors_are_refused() {
    let dir = TempDir::new("cdc_invalid");
    let e = engine(dir.path());
    e.enable_changes(Settings::default()).unwrap();
    e.put("a", &s("1"), 0, false).unwrap();
    for bad in ["nonsense", "ffffffffffffffff0000000000000001"] {
        let err = e.read_changes(ChangesFrom::Cursor(bad.into()), 10, None, None).err().unwrap().to_string();
        assert!(err.starts_with("CURSOR_INVALID:"), "{err}");
    }
    // Turned off and on again: a new journal, whose cursors are its own.
    let old = e.read_changes(ChangesFrom::Now, 1, None, None).unwrap().cursor;
    e.disable_changes().unwrap();
    assert!(e.read_changes(ChangesFrom::Beginning, 1, None, None).is_err());
    e.enable_changes(Settings::default()).unwrap();
    let err = e.read_changes(ChangesFrom::Cursor(old), 10, None, None).err().unwrap().to_string();
    assert!(err.contains("another change journal"), "{err}");
}

#[test]
fn named_consumers_resume_where_they_acknowledged() {
    let dir = TempDir::new("cdc_consumers");
    let e = engine(dir.path());
    e.enable_changes(Settings::default()).unwrap();
    for i in 0..10 {
        e.put(&format!("k{i}"), &s("v"), 0, false).unwrap();
    }
    let first = e.read_changes(ChangesFrom::Beginning, 4, None, Some("billing")).unwrap();
    // Handled: acknowledged by asking for what comes after.
    let second = e.read_changes(ChangesFrom::Cursor(first.cursor.clone()), 4, None, Some("billing")).unwrap();
    assert_eq!(second.changes[0].seq, 5);
    // A restart, without a cursor: from the last acknowledgement, so the batch
    // not acknowledged comes again (at least once).
    e.close();
    drop(e);
    let e = engine(dir.path());
    let again = e.read_changes(ChangesFrom::Consumer, 4, None, Some("billing")).unwrap();
    assert_eq!(again.changes[0].seq, 5);
    let other = e.read_changes(ChangesFrom::Beginning, 100, None, Some("search")).unwrap();
    assert_eq!(other.changes.len(), 10, "each consumer has its own position");
    let status = e.changes_status().unwrap();
    let lag: Vec<(String, u64)> = status.consumers.iter().map(|(n, s, _)| (n.clone(), status.last_seq - s)).collect();
    assert_eq!(lag, [("billing".to_string(), 6), ("search".to_string(), 10)]);
}

#[test]
fn concurrent_readers_see_every_change_once_and_in_order() {
    let dir = TempDir::new("cdc_concurrent");
    let e = engine(dir.path());
    e.enable_changes(Settings::default()).unwrap();
    let writer = {
        let e = e.clone();
        std::thread::spawn(move || {
            for i in 0..500 {
                e.put(&format!("k{}", i % 37), &s(&i.to_string()), 0, false).unwrap();
            }
        })
    };
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let e = e.clone();
            std::thread::spawn(move || {
                let (mut seen, mut from) = (Vec::new(), ChangesFrom::Beginning);
                while seen.len() < 500 {
                    let r = e.read_changes(from, 50, None, None).unwrap();
                    seen.extend(r.changes.iter().map(|c| c.seq));
                    from = ChangesFrom::Cursor(r.cursor);
                }
                seen
            })
        })
        .collect();
    writer.join().unwrap();
    for r in readers {
        assert_eq!(r.join().unwrap(), (1..=500).collect::<Vec<u64>>());
    }
}

#[test]
fn a_reset_is_a_change_and_the_journal_follows_the_new_file() {
    let dir = TempDir::new("cdc_reset");
    let e = engine(dir.path());
    e.enable_changes(Settings::default()).unwrap();
    e.put("a", &s("1"), 0, false).unwrap();
    e.reset().unwrap();
    e.put("b", &s("2"), 0, false).unwrap();
    e.close();
    drop(e);
    let e = engine(dir.path());
    assert_eq!(
        summary(&e, ChangesFrom::Beginning),
        [row(1, "insert", "a"), row(2, "reset", ""), row(3, "insert", "b")]
    );
}

#[test]
fn an_encrypted_workspace_keeps_an_encrypted_journal() {
    let dir = TempDir::new("cdc_encrypted");
    let keys = Arc::new(kivadb::crypt::Keys::new(&[7u8; 32], &[]));
    let opts = || EngineOptions { background: false, keys: Some(keys.clone()), ..Default::default() };
    {
        let e = Engine::open(dir.path().join("store.kiva"), opts()).unwrap();
        e.enable_changes(Settings::default()).unwrap();
        e.put("card", &s("4111-plaintext"), 0, false).unwrap();
        e.close();
    }
    let bytes: Vec<u8> = std::fs::read_dir(dir.path().join("store.kiva.changes"))
        .unwrap()
        .flatten()
        .flat_map(|f| std::fs::read(f.path()).unwrap())
        .collect();
    assert!(!bytes.windows(4).any(|w| w == b"card") && !bytes.windows(9).any(|w| w == b"plaintext"));
    let e = Engine::open(dir.path().join("store.kiva"), opts()).unwrap();
    let r = e.read_changes(ChangesFrom::Beginning, 10, None, None).unwrap();
    assert_eq!(r.changes[0].value.as_deref(), Some(&b"4111-plaintext"[..]));
}

#[test]
fn replicated_writes_are_journaled_on_the_replica_with_its_own_cursors() {
    let dir = TempDir::new("cdc_replica");
    let master = engine(&dir.path().join("m"));
    let replica = engine(&dir.path().join("r"));
    master.enable_changes(Settings::default()).unwrap();
    replica.enable_changes(Settings::default()).unwrap();
    master.put("a", &s("1"), 0, false).unwrap();
    replica.put_raw("a", Type::String, b"1", false, 0, 0).unwrap();
    assert_eq!(summary(&replica, ChangesFrom::Beginning), [row(1, "insert", "a")]);
    let cursor = master.read_changes(ChangesFrom::Now, 1, None, None).unwrap().cursor;
    let err = replica.read_changes(ChangesFrom::Cursor(cursor), 10, None, None).err().unwrap().to_string();
    assert!(err.contains("another change journal"), "a cursor of the master is refused on a replica: {err}");
}

#[test]
fn reading_needs_access_and_switching_on_needs_admin() {
    let dir = TempDir::new("cdc_access");
    let inst: Arc<Instance> = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "workspace create shop");
    ok(&mut admin, "workspace create hr");
    ok(&mut admin, r#"create user "app" identified by "pw" grant read on shop"#);
    ok(&mut admin, "workspace switch shop");
    ok(&mut admin, "changes enable");
    ok(&mut admin, "set item:1 1");
    ok(&mut admin, "workspace switch hr");
    ok(&mut admin, "changes enable");
    ok(&mut admin, r#"set salary:1 {"amount": 1000}"#);

    let mut app = Session::new(inst.clone(), false, None);
    ok(&mut app, "auth app pw");
    ok(&mut app, "workspace switch shop");
    assert!(ok(&mut app, "changes read").contains("insert item:1"));
    assert!(fails(&mut app, "changes disable").contains("admin"));
    assert!(fails(&mut app, "workspace switch hr").contains("no access"));
    // Revoked: the next read is refused.
    ok(&mut admin, r#"revoke read on shop from "app""#);
    assert!(app.check_reads_now().is_err());
}

#[test]
fn a_home_of_1_7_1_keeps_its_format_with_a_journal() {
    let dir = TempDir::new("cdc_compat");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("v1.7.1");
    let home = dir.path().join("home");
    copy_dir(&fixture, &home);
    let conf = home.join("kiva.conf");
    let text = std::fs::read_to_string(&conf).unwrap();
    std::fs::write(&conf, text.replace("@HOME@", &home.display().to_string().replace('\\', "/"))).unwrap();
    let inst = Instance::open(&home, Default::default()).unwrap();
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "changes enable");
    ok(&mut s, r#"set user:4 {"name": "Dan", "role": "dev"}"#);
    assert!(ok(&mut s, "changes read").contains("insert user:4"));
    let header = std::fs::read(home.join("data").join("store.kiva")).unwrap();
    assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 1, "the data file stays readable by 1.x");
}

#[test]
fn schema_defaults_are_in_the_change() {
    let dir = TempDir::new("cdc_schema");
    let mut sh = Session::new(instance(&dir), true, None);
    ok(&mut sh, "changes enable");
    ok(&mut sh, r#"schema set user: {"active": {"type": "boolean", "default": true}}"#);
    ok(&mut sh, r#"set user:1 {"name": "Ana"}"#);
    assert!(ok(&mut sh, "changes read").contains(r#"{"name":"Ana","active":true}"#));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[test]
fn the_structured_request_reads_the_journal_like_rest() {
    let dir = TempDir::new("cdc_api");
    let mut s = Session::new(instance(&dir), true, None);
    ok(&mut s, "changes enable");
    ok(&mut s, "set a 1");
    ok(&mut s, "set b 2");
    let r = kivadb::api::handle(&mut s, &json!({"op": "changes", "cursor": "beginning", "limit": 1})).unwrap();
    assert_eq!(r["changes"][0]["key"], "a");
    assert_eq!(r["more"], true);
    let next = kivadb::api::handle(&mut s, &json!({"op": "changes", "cursor": r["cursor"]})).unwrap();
    assert_eq!(next["changes"][0]["key"], "b");
    // A named consumer resumes after what it acknowledged.
    kivadb::api::handle(&mut s, &json!({"op": "changes", "cursor": r["cursor"], "consumer": "app"})).unwrap();
    let again = kivadb::api::handle(&mut s, &json!({"op": "changes", "consumer": "app"})).unwrap();
    assert_eq!(again["changes"][0]["key"], "b");
    let err = kivadb::api::handle(&mut s, &json!({"op": "changes", "cursor": 3})).unwrap_err().to_string();
    assert!(err.contains("is a text"), "{err}");
    let err = kivadb::api::handle(&mut s, &json!({"op": "changes", "cursor": "0".repeat(32)})).unwrap_err().to_string();
    assert!(err.starts_with("CURSOR_INVALID"), "{err}");
}
