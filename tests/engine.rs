mod common;

use common::TempDir;
use kivadb::compress::Compression;
use kivadb::engine::{BatchOp, CmpOp, Cond, Engine, EngineOptions};
use kivadb::error::KivaError;
use kivadb::util::now_ms;
use kivadb::value::Value;
use serde_json::json;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn quiet() -> EngineOptions {
    EngineOptions { background: false, auto_compact: false, ..Default::default() }
}

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

fn get_str(e: &Engine, key: &str) -> Option<String> {
    e.get(key).unwrap().map(|(v, _)| v.display())
}

/// Copies the data file (and optionally its hint) as a crashed process would
/// have left them: no clean shutdown, whatever is on disk right now.
fn crash_copy(from: &Path, to: &Path, with_hint: bool) {
    fs::create_dir_all(to.parent().unwrap()).unwrap();
    fs::copy(from, to).unwrap();
    if with_hint {
        let hint = |p: &Path| format!("{}.hint", p.display());
        fs::copy(hint(from), hint(to)).unwrap();
    }
}

#[test]
fn every_type_round_trips() {
    let dir = TempDir::new("types");
    let e = Engine::open(dir.path().join("store.kiva"), quiet()).unwrap();
    let values = vec![
        ("str", s("héllo wörld")),
        ("num", Value::Num("-3.14".into())),
        ("bool", Value::Bool(true)),
        ("null", Value::Null),
        ("arr", Value::Doc(json!([1, "two", [3.5], null]))),
        ("obj", Value::Doc(json!({"name": "Fordi", "tags": ["a", "b"], "n": {"x": 1}}))),
        ("bytes", Value::Bytes(vec![0, 255, 1, 2, 3])),
    ];
    for (k, v) in &values {
        assert!(e.put(k, v, 0, false).unwrap());
    }
    e.put("obj_mp", &values[5].1, 0, true).unwrap();
    for (k, v) in &values {
        let (got, entry) = e.get(k).unwrap().unwrap();
        assert_eq!(&got, v);
        assert_eq!(entry.ty, v.ty());
    }
    let (mp, entry) = e.get("obj_mp").unwrap().unwrap();
    assert_eq!(mp, values[5].1);
    assert!(entry.is_msgpack());
    assert!(e.get("missing").unwrap().is_none());
}

#[test]
fn reopen_loads_from_hint_and_replays_the_tail() {
    let dir = TempDir::new("hint");
    let path = dir.path().join("store.kiva");
    {
        let e = Engine::open(&path, quiet()).unwrap();
        for i in 0..500 {
            e.put(&format!("k{i:04}"), &s(&format!("value-{i}")), 0, false).unwrap();
        }
        e.delete("k0007").unwrap();
    }
    // Clean shutdown: the hint covers everything.
    {
        let e = Engine::open(&path, quiet()).unwrap();
        assert_eq!(e.stats().load_mode, "hint");
        assert_eq!(e.len(), 499);
        assert_eq!(get_str(&e, "k0123").as_deref(), Some("value-123"));
        assert!(get_str(&e, "k0007").is_none());

        // Writes after the snapshot, then a "crash".
        e.snapshot();
        e.put("late", &s("after-snapshot"), 0, false).unwrap();
        e.put("k0001", &s("overwritten"), 0, false).unwrap();
        e.delete("k0002").unwrap();
        crash_copy(&path, &dir.path().join("crash/store.kiva"), true);
    }
    let e = Engine::open(dir.path().join("crash/store.kiva"), quiet()).unwrap();
    assert_eq!(e.stats().load_mode, "hint");
    assert_eq!(get_str(&e, "late").as_deref(), Some("after-snapshot"));
    assert_eq!(get_str(&e, "k0001").as_deref(), Some("overwritten"));
    assert!(get_str(&e, "k0002").is_none());
    assert_eq!(e.len(), 499);
}

#[test]
fn a_corrupted_hint_falls_back_to_a_full_scan() {
    let dir = TempDir::new("badhint");
    let path = dir.path().join("store.kiva");
    {
        let e = Engine::open(&path, quiet()).unwrap();
        for i in 0..50 {
            e.put(&format!("k{i}"), &s("v"), 0, false).unwrap();
        }
    }
    let hint = dir.path().join("store.kiva.hint");
    let mut bytes = fs::read(&hint).unwrap();
    let last = bytes.len() - 5;
    bytes[last] ^= 0xff;
    fs::write(&hint, bytes).unwrap();

    let e = Engine::open(&path, quiet()).unwrap();
    assert_eq!(e.stats().load_mode, "scan");
    assert_eq!(e.len(), 50);
    assert!(e.hint_info().status.starts_with("INVALID"));
}

#[test]
fn a_torn_final_write_is_discarded() {
    let dir = TempDir::new("torn");
    let path = dir.path().join("store.kiva");
    let crashed = dir.path().join("crash/store.kiva");
    {
        let e = Engine::open(&path, quiet()).unwrap();
        e.put("first", &s("safe"), 0, false).unwrap();
        e.put("second", &s("also safe"), 0, false).unwrap();
        e.put("third", &s("this record will be cut in half"), 0, false).unwrap();
        crash_copy(&path, &crashed, false);
    }
    let len = fs::metadata(&crashed).unwrap().len();
    fs::OpenOptions::new().write(true).open(&crashed).unwrap().set_len(len - 10).unwrap();

    let e = Engine::open(&crashed, quiet()).unwrap();
    assert_eq!(get_str(&e, "first").as_deref(), Some("safe"));
    assert_eq!(get_str(&e, "second").as_deref(), Some("also safe"));
    assert!(get_str(&e, "third").is_none());
    assert!(e.stats().recovered_bytes > 0);
    // The file is usable again.
    e.put("third", &s("rewritten"), 0, false).unwrap();
    drop(e);
    let e = Engine::open(&crashed, quiet()).unwrap();
    assert_eq!(get_str(&e, "third").as_deref(), Some("rewritten"));
}

#[test]
fn a_batch_without_its_commit_record_never_happened() {
    let dir = TempDir::new("batch");
    let path = dir.path().join("store.kiva");
    let crashed = dir.path().join("crash/store.kiva");
    let before;
    {
        let e = Engine::open(&path, quiet()).unwrap();
        e.put("base", &s("committed"), 0, false).unwrap();
        before = e.stats().file_bytes;
        e.apply_batch(
            vec![
                BatchOp::Put { key: "a".into(), value: s("1"), expires_at: 0, msgpack: false },
                BatchOp::Put { key: "b".into(), value: s("2"), expires_at: 0, msgpack: false },
                BatchOp::Del { key: "base".into() },
            ],
            None,
        )
        .unwrap();
        assert!(get_str(&e, "base").is_none());
        crash_copy(&path, &crashed, false);
    }
    // Cut inside the last record of the batch: its first records are intact.
    let len = fs::metadata(&crashed).unwrap().len();
    fs::OpenOptions::new().write(true).open(&crashed).unwrap().set_len(len - 4).unwrap();

    let e = Engine::open(&crashed, quiet()).unwrap();
    assert_eq!(get_str(&e, "base").as_deref(), Some("committed"));
    assert!(get_str(&e, "a").is_none());
    assert!(get_str(&e, "b").is_none());
    assert_eq!(e.stats().file_bytes, before);
}

fn flip_byte(path: &Path, offset: u64) {
    let mut f = fs::OpenOptions::new().read(true).write(true).open(path).unwrap();
    let mut b = [0u8; 1];
    f.seek(SeekFrom::Start(offset)).unwrap();
    f.read_exact(&mut b).unwrap();
    b[0] ^= 0x55;
    f.seek(SeekFrom::Start(offset)).unwrap();
    f.write_all(&b).unwrap();
}

#[test]
fn bit_rot_is_detected_and_repaired() {
    let dir = TempDir::new("rot");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet()).unwrap();
    e.put("good1", &s("fine"), 0, false).unwrap();
    e.put("victim", &s("this value will be damaged on disk"), 0, false).unwrap();
    e.put("good2", &s("fine too"), 0, false).unwrap();
    let entry = e.meta("victim").unwrap();
    flip_byte(&path, entry.offset + entry.rec_len() - 3);

    assert!(matches!(e.get("victim"), Err(KivaError::Corrupted(_))));
    assert_eq!(get_str(&e, "good1").as_deref(), Some("fine"));

    let report = e.check(false).unwrap();
    assert_eq!((report.total, report.ok, report.corrupted.len()), (3, 2, 1));
    assert_eq!(report.corrupted[0], "victim");
    assert!(e.contains("victim"));

    let report = e.check(true).unwrap();
    assert_eq!(report.repaired, 1);
    assert!(report.backup.as_ref().unwrap().exists());
    assert!(!e.contains("victim"));
    assert_eq!(e.check(false).unwrap().corrupted.len(), 0);
    assert!(e.stats().corrupted_detected >= 1);
}

#[test]
fn a_damaged_record_in_the_middle_is_skipped_at_startup() {
    let dir = TempDir::new("midrot");
    let path = dir.path().join("store.kiva");
    let crashed = dir.path().join("crash/store.kiva");
    let entry;
    {
        let e = Engine::open(&path, quiet()).unwrap();
        e.put("a", &s("alpha"), 0, false).unwrap();
        e.put("b", &s("bravo bravo bravo"), 0, false).unwrap();
        e.put("c", &s("charlie"), 0, false).unwrap();
        entry = e.meta("b").unwrap();
        crash_copy(&path, &crashed, false);
    }
    flip_byte(&crashed, entry.offset + entry.rec_len() - 2);
    let e = Engine::open(&crashed, quiet()).unwrap();
    assert_eq!(get_str(&e, "a").as_deref(), Some("alpha"));
    assert!(get_str(&e, "b").is_none());
    assert_eq!(get_str(&e, "c").as_deref(), Some("charlie"));
    assert_eq!(e.stats().corrupted_detected, 1);
}

#[test]
fn compaction_reclaims_space_and_keeps_every_live_key() {
    let dir = TempDir::new("compact");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet()).unwrap();
    for round in 0..20 {
        for i in 0..100 {
            e.put(&format!("k{i:03}"), &s(&format!("round-{round}-value-{i}")), 0, false).unwrap();
        }
    }
    for i in 0..50 {
        e.delete(&format!("k{i:03}")).unwrap();
    }
    let before = e.stats();
    assert!(before.fragmentation > 0.9);
    let report = e.compact().unwrap();
    assert_eq!(report.records, 50);
    let after = e.stats();
    assert!(after.file_bytes < before.file_bytes / 10);
    assert_eq!(after.fragmentation, 0.0);
    assert_eq!(after.file_bytes, fs::metadata(&path).unwrap().len());
    for i in 50..100 {
        assert_eq!(get_str(&e, &format!("k{i:03}")), Some(format!("round-19-value-{i}")));
    }
    assert!(get_str(&e, "k010").is_none());
    assert_eq!(e.hint_info().status, "VALID");

    // Still writable, and everything survives a restart.
    e.put("after", &s("compaction"), 0, false).unwrap();
    drop(e);
    let e = Engine::open(&path, quiet()).unwrap();
    assert_eq!(e.len(), 51);
    assert_eq!(get_str(&e, "after").as_deref(), Some("compaction"));
    assert_eq!(get_str(&e, "k099").as_deref(), Some("round-19-value-99"));
}

#[test]
fn compaction_does_not_lose_concurrent_writes() {
    let dir = TempDir::new("concurrent");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet()).unwrap();
    for i in 0..2000 {
        e.put(&format!("seed{i:05}"), &s(&"x".repeat(200)), 0, false).unwrap();
    }
    let writers: Vec<_> = (0..4)
        .map(|w| {
            let e: Arc<Engine> = e.clone();
            std::thread::spawn(move || {
                for i in 0..1500 {
                    e.put(&format!("w{w}:{i:05}"), &Value::Num(i.to_string()), 0, false).unwrap();
                    if i % 3 == 0 {
                        e.put(&format!("seed{:05}", (i * 7 + w) % 2000), &s("rewritten"), 0, false).unwrap();
                    }
                    assert!(e.get(&format!("w{w}:{i:05}")).unwrap().is_some());
                    if i % 10 == 0 {
                        e.delete(&format!("w{w}:{:05}", i / 2)).unwrap();
                    }
                }
            })
        })
        .collect();
    let mut compactions = 0;
    while writers.iter().any(|w| !w.is_finished()) {
        if e.compact().is_ok() {
            compactions += 1;
        }
    }
    for w in writers {
        w.join().unwrap();
    }
    assert!(compactions > 0);
    e.compact().unwrap();

    let verify = |e: &Engine| {
        assert_eq!(e.check(false).unwrap().corrupted.len(), 0);
        for w in 0..4 {
            for i in 0..1500 {
                let deleted = (0..1500).step_by(10).any(|j| j / 2 == i && j >= i);
                let got = e.get(&format!("w{w}:{i:05}")).unwrap().map(|(v, _)| v.display());
                // A key deleted after it was written must stay deleted;
                // every other key must hold exactly its value.
                let expected = if deleted { None } else { Some(i.to_string()) };
                assert_eq!(got, expected, "w{w}:{i:05}");
            }
        }
        assert_eq!(e.range(Some("seed"), Some("seed99999"), false, 0).len(), 2000);
    };
    verify(&e);
    let keys = e.len();
    drop(e);
    let e = Engine::open(&path, quiet()).unwrap();
    assert_eq!(e.len(), keys);
    verify(&e);
}

#[test]
fn auto_compaction_runs_in_the_background() {
    let dir = TempDir::new("auto");
    let opts = EngineOptions {
        auto_compact: true,
        compact_threshold: 30,
        auto_compact_min_bytes: 4096,
        background: true,
        ..Default::default()
    };
    let e = Engine::open(dir.path().join("store.kiva"), opts).unwrap();
    for i in 0..2000 {
        e.put("hot", &s(&format!("version {i} {}", "pad".repeat(20))), 0, false).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    // A compaction may run while the loop is still writing. Its leftover can be
    // a small file that is mostly dead weight but below the 4096-byte minimum,
    // which auto-compaction deliberately leaves alone: that is settled too.
    let settled = |e: &Engine| {
        let st = e.stats();
        st.compactions >= 1 && (st.fragmentation < 0.3 || st.file_bytes < 4096)
    };
    while !settled(&e) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let st = e.stats();
    assert!(st.compactions >= 1, "auto-compaction never ran");
    assert!(
        st.fragmentation < 0.3 || st.file_bytes < 4096,
        "fragmentation {:.2} on {} bytes, compactions {}, last error {:?}",
        st.fragmentation,
        st.file_bytes,
        st.compactions,
        st.compact_error
    );
    // 2000 versions of about 90 bytes were written: almost all of it is gone.
    assert!(st.file_bytes < 20_000, "the file still holds {} bytes", st.file_bytes);
    assert!(get_str(&e, "hot").unwrap().starts_with("version 1999"));
}

#[test]
fn expired_keys_disappear() {
    let dir = TempDir::new("ttl");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet()).unwrap();
    e.put("gone", &s("x"), now_ms() - 1000, false).unwrap();
    e.put("soon", &s("y"), now_ms() + 1000, false).unwrap();
    e.put("kept", &s("z"), now_ms() + 3_600_000, false).unwrap();
    assert!(e.get("gone").unwrap().is_none());
    assert!(e.get("soon").unwrap().is_some());
    std::thread::sleep(Duration::from_millis(2100));
    assert!(e.get("soon").unwrap().is_none());
    assert!(e.range(None, None, false, 0).iter().all(|(k, _)| k == "kept"));
    drop(e);
    let e = Engine::open(&path, quiet()).unwrap();
    assert_eq!(e.len(), 1);
    assert!(e.meta("kept").unwrap().expires_at > now_ms());
}

#[test]
fn the_background_sweep_removes_expired_keys_in_batches() {
    let dir = TempDir::new("ttl-sweep");
    let opts = EngineOptions { auto_compact: false, ..Default::default() };
    let e = Engine::open(dir.path().join("store.kiva"), opts).unwrap();
    // More than two batches of the sweep, plus keys that must stay. The
    // deadlines are far first, so that a slow machine cannot see them pass
    // before the counts are checked.
    let sessions = |expires_at: i64| -> Vec<BatchOp> {
        (0..25_000)
            .map(|i| BatchOp::Put { key: format!("session:{i:05}"), value: s("x"), expires_at, msgpack: false })
            .collect()
    };
    e.apply_batch(sessions(now_ms() + 3_600_000), None).unwrap();
    e.put("kept", &s("z"), now_ms() + 3_600_000, false).unwrap();
    e.put("plain", &s("p"), 0, false).unwrap();
    assert_eq!(e.stats().ttl_keys, 25_001);

    // Changing or removing a TTL moves the key's deadline, or drops it.
    e.put("session:00000", &s("x"), 0, false).unwrap();
    e.put("session:00001", &s("x"), now_ms() + 7_200_000, false).unwrap();
    e.delete("session:00002").unwrap();
    assert_eq!(e.stats().ttl_keys, 24_999);

    // Now the other sessions' time is up.
    let gone: Vec<BatchOp> = sessions(now_ms() - 1000).into_iter().skip(3).collect();
    e.apply_batch(gone, None).unwrap();

    // Removed by the maintenance thread, not only hidden from reads.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while e.len() > 4 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(e.len(), 4, "keys left: {}", e.len());
    assert_eq!(e.stats().ttl_keys, 2);
    assert!(e.get("session:00000").unwrap().is_some() && e.get("session:00001").unwrap().is_some());
}

#[test]
fn compression_is_transparent() {
    let dir = TempDir::new("comp");
    let path = dir.path().join("store.kiva");
    let text = "KivaDB compresses repetitive values very well. ".repeat(100);
    for algo in [Compression::Lz4, Compression::Zlib] {
        let p = dir.path().join(format!("{}.kiva", algo.name()));
        let e = Engine::open(&p, EngineOptions { compression: algo, ..quiet() }).unwrap();
        e.put("big", &s(&text), 0, false).unwrap();
        e.put("small", &s("tiny"), 0, false).unwrap();
        e.put("doc", &Value::Doc(json!({"text": text})), 0, true).unwrap();
        let big = e.meta("big").unwrap();
        assert_eq!(big.compression(), algo);
        assert!((big.v_size as usize) < text.len() / 5);
        assert_eq!(e.meta("small").unwrap().compression(), Compression::None);
        drop(e);
        let e = Engine::open(&p, quiet()).unwrap();
        assert_eq!(get_str(&e, "big").as_deref(), Some(text.as_str()));
        assert_eq!(e.get("doc").unwrap().unwrap().0, Value::Doc(json!({"text": text})));
    }

    // Re-encoding an existing database in both directions.
    let e = Engine::open(&path, quiet()).unwrap();
    for i in 0..20 {
        e.put(&format!("k{i}"), &s(&text), 0, false).unwrap();
    }
    let plain = e.stats().file_bytes;
    e.compact_with(Some(Compression::Zlib)).unwrap();
    let packed = e.stats();
    assert!(packed.file_bytes < plain / 5);
    assert_eq!(packed.compressed_records, 20);
    assert_eq!(get_str(&e, "k7").as_deref(), Some(text.as_str()));
    e.compact_with(Some(Compression::None)).unwrap();
    assert_eq!(e.stats().compressed_records, 0);
    assert_eq!(get_str(&e, "k7").as_deref(), Some(text.as_str()));
    assert_eq!(e.check(false).unwrap().corrupted.len(), 0);
}

#[test]
fn ranges_are_ordered_and_bounded() {
    let dir = TempDir::new("range");
    let e = Engine::open(dir.path().join("store.kiva"), quiet()).unwrap();
    for i in (1..=50).rev() {
        e.put(&format!("user_{i:03}"), &Value::Num(i.to_string()), 0, false).unwrap();
    }
    e.put("zzz", &s("other"), 0, false).unwrap();
    let keys = |from, to, rev, limit| -> Vec<String> {
        e.range(Some(from), Some(to), rev, limit).into_iter().map(|(k, _)| k).collect()
    };
    assert_eq!(keys("user_001", "user_003", false, 0), ["user_001", "user_002", "user_003"]);
    assert_eq!(keys("user_048", "user_999", true, 2), ["user_050", "user_049"]);
    // Bounds given high-to-low describe the same range.
    assert_eq!(keys("user_003", "user_001", true, 0), ["user_003", "user_002", "user_001"]);
    assert_eq!(e.range(None, None, false, 0).len(), 51);
    assert_eq!(e.keys_matching("user_01?", 0).len(), 10);
    assert_eq!(e.keys_matching("*", 5).len(), 5);
}

#[test]
fn secondary_indexes_follow_every_change() {
    let dir = TempDir::new("index");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet()).unwrap();
    for i in 0..100 {
        let doc = json!({"name": format!("u{i}"), "age": i, "role": if i % 10 == 0 { "admin" } else { "dev" }});
        e.put(&format!("user:{i:03}"), &Value::Doc(doc), 0, i % 2 == 0).unwrap();
    }
    e.put("user:str", &s("not a document"), 0, false).unwrap();
    let cond = |field: &str, op, value| Cond { pattern: "user:*".into(), field: field.into(), op, value };
    let admins = cond("role", CmpOp::Eq, json!("admin"));

    let (rows, plan) = e.query(&admins, 0).unwrap();
    assert_eq!(rows.len(), 10);
    assert_eq!(plan.strategy, "PREFIX SCAN");

    assert_eq!(e.create_index("user:*", "role").unwrap(), 100);
    assert_eq!(e.create_index("user:*", "age").unwrap(), 100);
    let (rows, plan) = e.query(&admins, 0).unwrap();
    assert_eq!(rows.len(), 10);
    assert_eq!(plan.strategy, "INDEX SCAN");
    assert_eq!(plan.candidates, 10);

    assert_eq!(e.query(&cond("age", CmpOp::Ge, json!(90)), 0).unwrap().0.len(), 10);
    assert_eq!(e.query(&cond("age", CmpOp::Lt, json!(5)), 0).unwrap().0.len(), 5);
    assert_eq!(e.query(&cond("age", CmpOp::Gt, json!(5)), 3).unwrap().0.len(), 3);
    assert_eq!(e.query(&cond("role", CmpOp::Ne, json!("admin")), 0).unwrap().0.len(), 90);

    // Updates, deletes and type changes keep the index exact.
    e.put("user:001", &Value::Doc(json!({"name": "u1", "age": 1, "role": "admin"})), 0, false).unwrap();
    e.delete("user:000").unwrap();
    e.put("user:010", &s("replaced by a string"), 0, false).unwrap();
    let (rows, _) = e.query(&admins, 0).unwrap();
    let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(rows.len(), 9);
    assert!(keys.contains(&"user:001") && !keys.contains(&"user:000") && !keys.contains(&"user:010"));

    // Index definitions persist and are rebuilt at startup, also after compaction.
    e.compact().unwrap();
    drop(e);
    let e = Engine::open(&path, quiet()).unwrap();
    assert_eq!(e.list_indexes().len(), 2);
    let (rows, plan) = e.query(&admins, 0).unwrap();
    assert_eq!((rows.len(), plan.strategy), (9, "INDEX SCAN"));
    assert!(e.drop_index("user:*", "role").unwrap());
    assert_eq!(e.query(&admins, 0).unwrap().1.strategy, "PREFIX SCAN");
}

#[test]
fn optimistic_conflicts_are_detected() {
    let dir = TempDir::new("conflict");
    let e = Engine::open(dir.path().join("store.kiva"), quiet()).unwrap();
    e.put("shared", &s("v0"), 0, false).unwrap();
    e.put("doomed", &s("v0"), 0, false).unwrap();
    let put = |k: &str, v: &str| BatchOp::Put { key: k.into(), value: s(v), expires_at: 0, msgpack: false };

    let begin = e.txn_begin();
    e.put("shared", &s("someone else"), 0, false).unwrap();
    let err = e.apply_batch(vec![put("shared", "mine"), put("other", "x")], Some(begin)).unwrap_err();
    assert!(matches!(err, KivaError::Conflict(_)));
    // Nothing of the rejected batch was applied.
    assert_eq!(get_str(&e, "shared").as_deref(), Some("someone else"));
    assert!(get_str(&e, "other").is_none());

    // A concurrent delete conflicts as well.
    e.delete("doomed").unwrap();
    assert!(e.apply_batch(vec![put("doomed", "back")], Some(begin)).is_err());
    e.txn_end();

    // Untouched keys commit fine.
    let begin = e.txn_begin();
    e.put("unrelated", &s("x"), 0, false).unwrap();
    e.apply_batch(vec![put("shared", "mine"), BatchOp::Del { key: "unrelated2".into() }], Some(begin)).unwrap();
    e.txn_end();
    assert_eq!(get_str(&e, "shared").as_deref(), Some("mine"));
}

#[test]
fn a_database_cannot_be_opened_twice() {
    let dir = TempDir::new("lock");
    let path = dir.path().join("store.kiva");
    let first = Engine::open(&path, quiet()).unwrap();
    assert!(matches!(Engine::open(&path, quiet()), Err(KivaError::Locked(_))));
    drop(first);
    Engine::open(&path, quiet()).unwrap();
}

#[test]
fn foreign_files_are_rejected() {
    let dir = TempDir::new("foreign");
    let path = dir.path().join("store.kiva");
    fs::write(&path, b"this is definitely not a KivaDB data file").unwrap();
    assert!(matches!(Engine::open(&path, quiet()), Err(KivaError::Format(_))));
    // The file is left untouched.
    assert_eq!(fs::read(&path).unwrap(), b"this is definitely not a KivaDB data file");
}

#[test]
fn reset_empties_the_database() {
    let dir = TempDir::new("reset");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet()).unwrap();
    for i in 0..100 {
        e.put(&format!("k{i}"), &s("v"), 0, false).unwrap();
    }
    e.snapshot();
    e.reset().unwrap();
    assert_eq!(e.len(), 0);
    e.put("fresh", &s("start"), 0, false).unwrap();
    drop(e);
    let e = Engine::open(&path, quiet()).unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(get_str(&e, "fresh").as_deref(), Some("start"));
}

#[test]
fn synchronous_writes_share_their_disk_syncs() {
    let dir = TempDir::new("group-commit");
    let path = dir.path().join("store.kiva");
    let opts = EngineOptions { sync_writes: true, auto_compact: false, ..Default::default() };
    {
        let e = Engine::open(&path, opts.clone()).unwrap();
        let writers: Vec<_> = (0..8)
            .map(|t| {
                let e = e.clone();
                std::thread::spawn(move || {
                    for i in 0..100 {
                        e.put(&format!("w{t}:{i:03}"), &s("v"), 0, false).unwrap();
                        if i % 10 == 0 {
                            e.delete(&format!("w{t}:{i:03}")).unwrap();
                        }
                    }
                })
            })
            .collect();
        // A compaction in the middle must neither block the writers nor lose a write.
        let compactor = {
            let e = e.clone();
            std::thread::spawn(move || {
                for _ in 0..3 {
                    e.compact().unwrap();
                }
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        compactor.join().unwrap();
        e.apply_batch(vec![BatchOp::Put { key: "batch".into(), value: s("b"), expires_at: 0, msgpack: false }], None)
            .unwrap();
        let st = e.stats();
        assert!(st.sync_writes);
        assert_eq!(st.durable_writes, 8 * 100 + 8 * 10 + 1);
        assert!(st.disk_syncs >= 1 && st.disk_syncs <= st.durable_writes, "{} syncs", st.disk_syncs);

        // Off again: writes no longer wait for the disk.
        e.set_sync_writes(false);
        e.put("async", &s("a"), 0, false).unwrap();
        assert_eq!(e.stats().durable_writes, st.durable_writes);
    }
    let e = Engine::open(&path, opts).unwrap();
    assert_eq!(e.len(), 8 * 90 + 2);
    assert!(e.get("w7:099").unwrap().is_some() && e.get("w7:090").unwrap().is_none());
}

/// Durable writes from many threads: how many share each fsync.
/// `cargo test --release --test engine group_commit_speed -- --ignored --nocapture`
#[test]
#[ignore]
fn group_commit_speed() {
    let dir = TempDir::new("group-commit-speed");
    let opts = EngineOptions { sync_writes: true, auto_compact: false, ..Default::default() };
    let e = Engine::open(dir.path().join("store.kiva"), opts).unwrap();
    for threads in [1, 4, 16, 64] {
        let before = e.stats();
        let t = std::time::Instant::now();
        let writers: Vec<_> = (0..threads)
            .map(|n| {
                let e = e.clone();
                std::thread::spawn(move || {
                    while t.elapsed() < std::time::Duration::from_secs(2) {
                        e.put(&format!("k{n}"), &s(&"x".repeat(100)), 0, false).unwrap();
                    }
                })
            })
            .collect();
        writers.into_iter().for_each(|w| w.join().unwrap());
        let st = e.stats();
        let (writes, syncs) = (st.durable_writes - before.durable_writes, st.disk_syncs - before.disk_syncs);
        println!(
            "{threads:>2} threads: {:>6.0} writes/s, {:.1} writes per fsync",
            writes as f64 / t.elapsed().as_secs_f64(),
            writes as f64 / syncs as f64
        );
    }
}
