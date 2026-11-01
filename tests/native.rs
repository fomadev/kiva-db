// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Documents holding `decimal`, `date`, `time` and `datetime` values, stored
//! in `KDOC` (storage format 3): they keep their types through every path,
//! a file changes format only when one is written, and older versions refuse
//! such a file without touching it (docs/architecture/NATIVE_DOCUMENTS.md).

mod common;

use common::{fails, ok, TempDir};
use kivadb::doc::{self, Doc};
use kivadb::engine::{BatchOp, Engine, EngineOptions};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use kivadb::value::{DocEnc, Value};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
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

/// A copy of a home written by KivaDB 1.7.1, its paths pointing at the copy.
fn home_of_1_7_1(dir: &TempDir) -> PathBuf {
    let home = dir.path().join("home");
    copy_dir(&fixture("v1.7.1"), &home);
    let conf = home.join("kiva.conf");
    let text = std::fs::read_to_string(&conf).unwrap();
    std::fs::write(&conf, text.replace("@HOME@", &home.display().to_string().replace('\\', "/"))).unwrap();
    home
}

fn open(home: &Path) -> Arc<Instance> {
    Instance::open(home, Options::default()).unwrap()
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.file_type().unwrap().is_dir() {
            out.extend(snapshot(&e.path()));
        } else {
            out.push((e.path(), std::fs::read(e.path()).unwrap()));
        }
    }
    out.sort();
    out
}

fn format_of(file: &Path) -> u32 {
    let h = std::fs::read(file).unwrap();
    u32::from_le_bytes([h[4], h[5], h[6], h[7]])
}

/// What 1.7.1 wrote in its fixture, read back, with its indexes, workspaces and accounts.
fn the_values_of_1_7_1_are_all_there(s: &mut Session) {
    assert_eq!(ok(s, "get greeting"), "greeting: hello [string]");
    assert_eq!(ok(s, "get n:count"), "n:count: 41 [number]");
    assert_eq!(ok(s, "get list"), "list: [1,2,3] [array]");
    assert_eq!(ok(s, "get blob"), "blob: 0x00010203 [bytes]");
    assert!(ok(s, "get long").starts_with("long: kivadb kivadb"));
    assert_eq!(ok(s, "get user:2.name"), "user:2.name: Ben [string]");
    assert_eq!(ok(s, "count where user:*.role = \"dev\""), "Count: 2");
    assert!(ok(s, "explain get where user:*.role = \"dev\"").contains("INDEX SCAN"));
    assert!(fails(s, r#"set user:9 {"name": "ANA", "role": "x"}"#).contains("unique index"));
    ok(s, "workspace switch shop");
    assert_eq!(ok(s, "get item:1.label"), "item:1.label: pen [string]");
    ok(s, "workspace switch default");
}

/// The accounts of the fixture sign in with their roles.
fn the_accounts_of_1_7_1_are_all_there(inst: &Arc<Instance>) {
    let mut ana = Session::new(inst.clone(), false, None);
    ok(&mut ana, "auth ana ana-pw");
    assert_eq!(ok(&mut ana, "whoami"), "ana (editor) @ workspace 'u_ana'");
    let mut guest = Session::new(inst.clone(), false, None);
    ok(&mut guest, "auth guest guest-pw");
    assert!(ok(&mut guest, "whoami").starts_with("guest (viewer)"));
}

/// A document with every native type, at every depth, next to the JSON types.
fn order() -> Doc {
    doc::from_extended_json(&json!({
        "id": 7,
        "ratio": 0.5,
        "eight": 8.0,
        "total": {"$decimal": "1250.50"},
        "day": {"$date": "2026-10-31"},
        "at": {"$datetime": "2026-10-31T15:18:00.250Z"},
        "opens": {"$time": "08:30:00"},
        "note": "2026-10-31",
        "big": {"$decimal": "18446744073709551615"},
        "lines": [
            {"qty": 2, "price": {"$decimal": "9.99"}},
            {"$date": "2027-01-01"},
            [{"$time": "23:59:59.999"}, {"$decimal": "-0.000000000000000000000000001"}, null, true]
        ],
        "empty": {}
    }))
    .unwrap()
}

/// Same types and same values, leaf by leaf: extended JSON tells them apart.
fn assert_same(a: &Doc, b: &Doc) {
    assert_eq!(doc::to_extended_json(a), doc::to_extended_json(b));
}

fn typed(v: &Value) -> &Doc {
    match v {
        Value::Typed(d) => d,
        other => panic!("not a typed document: {other:?}"),
    }
}

fn engine(path: &Path) -> Arc<Engine> {
    Engine::open(path, EngineOptions::default()).unwrap()
}

#[test]
fn native_values_survive_writing_reading_and_reopening() {
    let dir = TempDir::new("native_roundtrip");
    let path = dir.path().join("store.kiva");
    let original = order();
    {
        let e = engine(&path);
        e.put("o:1", &Value::typed(original.clone()).unwrap(), 0, false).unwrap();
        // An array at the top too.
        let list = doc::from_extended_json(&json!([{"$date": "2026-01-02"}, 1, 1.5])).unwrap();
        e.put("days", &Value::typed(list).unwrap(), 0, false).unwrap();
        let (v, entry) = e.get("o:1").unwrap().unwrap();
        assert_eq!(entry.doc_enc(), DocEnc::Kdoc);
        assert_same(typed(&v), &original);
        drop(e);
    }
    assert_eq!(format_of(&path), 3);
    let e = engine(&path);
    let (v, _) = e.get("o:1").unwrap().unwrap();
    assert_same(typed(&v), &original);
    let (days, _) = e.get("days").unwrap().unwrap();
    assert_eq!(days.ty(), kivadb::value::Type::Array);
    assert_eq!(doc::to_extended_json(typed(&days)), json!([{"$date": "2026-01-02"}, 1, 1.5]));
    // Programs read plain JSON by default: the types of 2.0 as they were,
    // decimals, dates and times as texts.
    assert_eq!(v.to_json()["total"], json!("1250.50"));
    assert_eq!(v.to_json()["at"], json!("2026-10-31T15:18:00.250Z"));
    assert_eq!(v.to_json()["eight"], json!(8.0));
    assert_eq!(v.to_json()["big"], json!(18446744073709551615u64));
}

#[test]
fn a_document_json_can_hold_is_written_as_json_and_leaves_the_format_alone() {
    let dir = TempDir::new("native_json_stays");
    let path = dir.path().join("store.kiva");
    let e = engine(&path);
    // Built as a tree, but holding JSON types only, the u64 integer of 2.0 included.
    let plain =
        doc::from_extended_json(&json!({"n": 1, "f": 1.0, "big": {"$decimal": "18446744073709551615"}, "s": "x"}))
            .unwrap();
    let v = Value::typed(plain).unwrap();
    assert!(matches!(v, Value::Doc(_)));
    e.put("a", &v, 0, false).unwrap();
    e.put("b", &v, 0, true).unwrap();
    assert_eq!(e.get("a").unwrap().unwrap().1.doc_enc(), DocEnc::Json);
    assert_eq!(e.get("b").unwrap().unwrap().1.doc_enc(), DocEnc::MsgPack);
    // Even made by hand, a `Typed` JSON can hold is written as JSON.
    let by_hand = doc::from_extended_json(&json!({"n": 1})).unwrap();
    e.put("c", &Value::Typed(by_hand), 0, false).unwrap();
    assert_eq!(e.get("c").unwrap().unwrap().1.doc_enc(), DocEnc::Json);
    // A scalar is not a document.
    assert!(Value::typed(Doc::Int(1)).is_err());
    assert!(e.put("d", &Value::Typed(Doc::Int(1)), 0, false).is_err());
    drop(e);
    assert_eq!(format_of(&path), 1);
}

#[test]
fn what_could_not_be_read_back_is_refused_before_it_is_written() {
    let dir = TempDir::new("native_refused");
    let e = engine(&dir.path().join("store.kiva"));
    let mut deep = Doc::Array(vec![Doc::Date(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())]);
    for _ in 0..doc::MAX_DEPTH {
        deep = Doc::Array(vec![deep]);
    }
    let fine_time = chrono::NaiveTime::from_hms_micro_opt(8, 30, 0, 1).unwrap();
    let nan = Doc::Object([("x".to_string(), Doc::Float(f64::NAN))].into_iter().collect());
    let micro = Doc::Array(vec![Doc::Time(fine_time)]);
    for bad in [deep, nan, micro] {
        let err = Value::typed(bad.clone()).map(|_| ()).unwrap_err().to_string();
        assert!(!err.is_empty());
        assert!(e.put("k", &Value::Typed(bad), 0, false).is_err());
    }
    assert!(e.get("k").unwrap().is_none());
}

#[test]
fn a_home_of_1_7_1_is_raised_to_format_3_only_by_a_typed_document() {
    let dir = TempDir::new("native_raise");
    let home = home_of_1_7_1(&dir);
    let store = home.join("data").join("store.kiva");
    let shop = home.join("workspaces").join("shop").join("store.kiva");
    let system = home.join("system.kiva");
    let system_before = std::fs::read(&system).ok();
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        // JSON and MessagePack documents, read, changed and written back:
        // nothing raises the file.
        ok(&mut s, r#"set native:1 {"n": 18446744073709551615, "tags": ["a"]}"#);
        ok(&mut s, "json set native:1 .other 2");
        ok(&mut s, "convert native:1 to msgpack");
        ok(&mut s, "json set native:1 .other 3");
        assert_eq!(ok(&mut s, "get native:1.n"), "native:1.n: 18446744073709551615 [number]");
        assert_eq!(format_of(&store), 1);
        // The first typed document raises it, and only its own workspace.
        inst.engine("default").unwrap().put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
        assert_eq!(format_of(&store), 3);
        assert_eq!(format_of(&shop), 1);
        drop(s);
        inst.shutdown();
    }
    let inst = open(&home);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_1_7_1_are_all_there(&mut s);
    the_accounts_of_1_7_1_are_all_there(&inst);
    let (v, _) = inst.engine("default").unwrap().get("o:1").unwrap().unwrap();
    assert_same(typed(&v), &order());
    if let Some(before) = system_before {
        assert_eq!(format_of(&system), u32::from_le_bytes([before[4], before[5], before[6], before[7]]));
    }
}

#[test]
fn json_messagepack_and_kdoc_documents_answer_the_same_queries() {
    let dir = TempDir::new("native_mixed");
    let inst = open(dir.path());
    let mut s = Session::new(inst.clone(), true, None);
    let e = inst.engine("default").unwrap();
    ok(&mut s, r#"set o:1 {"id": 1, "email": "a@x", "city": "Goma", "day": "2026-10-30"}"#);
    e.put("o:2", &Value::from_json(json!({"id": 2, "email": "b@x", "city": "Goma"})), 0, true).unwrap();
    let third = doc::from_extended_json(&json!({
        "id": 3, "email": "c@x", "city": "Goma", "day": {"$date": "2026-10-31"}, "total": {"$decimal": "10.50"}
    }))
    .unwrap();
    e.put("o:3", &Value::typed(third).unwrap(), 0, false).unwrap();
    let encs: Vec<DocEnc> = ["o:1", "o:2", "o:3"].iter().map(|k| e.get(k).unwrap().unwrap().1.doc_enc()).collect();
    assert_eq!(encs, [DocEnc::Json, DocEnc::MsgPack, DocEnc::Kdoc]);

    assert_eq!(ok(&mut s, "count where o:*.city = \"Goma\""), "Count: 3");
    assert_eq!(ok(&mut s, "count where o:*.id >= 2"), "Count: 2");
    ok(&mut s, "create index on o:*.id");
    assert_eq!(ok(&mut s, "count where o:*.id >= 2"), "Count: 2");
    // A date reads as its ISO text until queries know native values (lot C3).
    assert!(ok(&mut s, "get where o:*.day = \"2026-10-31\"").contains("o:3"));
    assert!(ok(&mut s, "get o:3").contains(r#""total":"10.50""#));

    // One unique index over the three encodings.
    ok(&mut s, "create unique index on o:*.email");
    let dup = doc::from_extended_json(&json!({"id": 4, "email": "a@x", "at": {"$time": "10:00:00"}})).unwrap();
    assert!(e.put("o:4", &Value::typed(dup).unwrap(), 0, false).is_err());
    fails(&mut s, r#"set o:5 {"id": 5, "email": "c@x"}"#);
    drop((s, e));
    inst.shutdown();
    drop(inst);

    // The indexes are rebuilt from the three encodings when the home opens.
    let inst = open(dir.path());
    let mut s = Session::new(inst, true, None);
    assert!(ok(&mut s, "explain get where o:*.id = 3").contains("INDEX SCAN"));
    assert!(ok(&mut s, "get where o:*.id = 3").contains("o:3"));
    fails(&mut s, r#"set o:6 {"id": 6, "email": "c@x"}"#);
}

#[test]
fn typed_documents_of_a_transaction_are_written_together_or_not_at_all() {
    let dir = TempDir::new("native_txn");
    let path = dir.path().join("store.kiva");
    let e = engine(&path);
    e.put("keep", &Value::Str("x".into()), 0, false).unwrap();
    drop(e);
    let before = std::fs::metadata(&path).unwrap().len();
    let e = engine(&path);
    let put =
        |k: &str| BatchOp::Put { key: k.into(), value: Value::typed(order()).unwrap(), expires_at: 0, msgpack: false };
    e.apply_batch(vec![put("t:1"), put("t:2"), BatchOp::Del { key: "keep".into() }], None).unwrap();
    drop(e);
    let after = std::fs::read(&path).unwrap();

    let e = engine(&path);
    for k in ["t:1", "t:2"] {
        assert_same(typed(&e.get(k).unwrap().unwrap().0), &order());
    }
    assert!(e.get("keep").unwrap().is_none());
    drop(e);

    // The same batch cut before its last record: none of it took effect.
    std::fs::write(&path, &after[..after.len() - 1]).unwrap();
    let _ = std::fs::remove_file(dir.path().join("store.kiva.hint"));
    let e = engine(&path);
    assert!(e.get("t:1").unwrap().is_none());
    assert!(e.get("t:2").unwrap().is_none());
    assert_eq!(e.get("keep").unwrap().unwrap().0, Value::Str("x".into()));
    assert!(std::fs::metadata(&path).unwrap().len() >= before);
}

#[test]
fn a_crash_while_the_first_typed_document_is_written_loses_nothing_else() {
    let dir = TempDir::new("native_crash");
    let home = home_of_1_7_1(&dir);
    let store = home.join("data").join("store.kiva");
    let hint = home.join("data").join("store.kiva.hint");
    // The header raised to 3, no record yet: everything of 1.7.1 opens.
    let original = std::fs::read(&store).unwrap();
    let mut raised = original.clone();
    raised[4..8].copy_from_slice(&3u32.to_le_bytes());
    std::fs::write(&store, &raised).unwrap();
    let _ = std::fs::remove_file(&hint);
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        the_values_of_1_7_1_are_all_there(&mut s);
        inst.engine("default").unwrap().put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
        drop(s);
        inst.shutdown();
    }
    // The typed document cut in the middle of its bytes.
    let full = std::fs::read(&store).unwrap();
    let len = full.len() - raised.len();
    std::fs::write(&store, &full[..raised.len() + len / 2]).unwrap();
    let _ = std::fs::remove_file(&hint);
    let inst = open(&home);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_1_7_1_are_all_there(&mut s);
    assert_eq!(ok(&mut s, "get o:1"), "o:1: (nil)");
    assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));
    inst.engine("default").unwrap().put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
    assert_same(typed(&inst.engine("default").unwrap().get("o:1").unwrap().unwrap().0), &order());
}

#[test]
fn compaction_keeps_typed_documents_whether_it_copies_or_recompresses_them() {
    let dir = TempDir::new("native_compact");
    let path = dir.path().join("store.kiva");
    let e = engine(&path);
    let big = {
        let mut d = order();
        if let Doc::Object(m) = &mut d {
            m.insert("text".into(), Doc::Str("kivadb ".repeat(200)));
        }
        d
    };
    for i in 0..3 {
        e.put("o:1", &Value::typed(big.clone()).unwrap(), 0, false).unwrap();
        e.put(&format!("j:{i}"), &Value::from_json(json!({"i": i})), 0, false).unwrap();
    }
    e.compact().unwrap();
    assert_eq!(format_of(&path), 3);
    assert_same(typed(&e.get("o:1").unwrap().unwrap().0), &big);
    // Recompressing rewrites each record: the document keeps its encoding.
    e.compact_with(Some(kivadb::compress::Compression::Lz4)).unwrap();
    let (v, entry) = e.get("o:1").unwrap().unwrap();
    assert_eq!(entry.doc_enc(), DocEnc::Kdoc);
    assert_ne!(entry.compression(), kivadb::compress::Compression::None);
    assert_same(typed(&v), &big);
    drop(e);
    let e = engine(&path);
    assert_same(typed(&e.get("o:1").unwrap().unwrap().0), &big);
    assert_eq!(format_of(&path), 3);
    // Emptied, the file goes back to format 1.
    e.reset().unwrap();
    assert_eq!(format_of(&path), 1);
}

/// The offset of the payload of `key`'s last record in a data file.
fn payload_of(data: &[u8], key: &str) -> (usize, usize) {
    use kivadb::format::{parse_header, FILE_HEADER_LEN, REC_HEADER_LEN};
    let mut pos = FILE_HEADER_LEN as usize;
    let mut found = None;
    while data.len() - pos >= REC_HEADER_LEN {
        let h = parse_header(&data[pos..]);
        let k = &data[pos + REC_HEADER_LEN..pos + REC_HEADER_LEN + h.k_size as usize];
        if k == key.as_bytes() {
            found = Some((pos, pos + REC_HEADER_LEN + h.k_size as usize));
        }
        pos += h.total_len() as usize;
    }
    found.unwrap()
}

#[test]
fn a_damaged_kdoc_document_is_refused_with_its_key() {
    let dir = TempDir::new("native_damaged");
    let path = dir.path().join("store.kiva");
    let e = engine(&path);
    e.put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
    e.put("other", &Value::Str("fine".into()), 0, false).unwrap();
    drop(e);
    // An unknown type tag, its record's checksum made right again: only the
    // reading of the document can tell.
    let mut data = std::fs::read(&path).unwrap();
    let (rec, payload) = payload_of(&data, "o:1");
    assert_eq!(data[payload], doc::KDOC_VERSION);
    data[payload + 1] = 0x7F;
    let end = rec + kivadb::format::parse_header(&data[rec..]).total_len() as usize;
    let crc = crc32fast::hash(&data[rec + 4..end]);
    data[rec..rec + 4].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&path, &data).unwrap();

    let e = engine(&path);
    let err = e.get("o:1").unwrap_err().to_string();
    assert!(err.contains("o:1") && err.contains("KDOC document"), "{err}");
    assert_eq!(e.get("other").unwrap().unwrap().0, Value::Str("fine".into()));
    // And a backup check reads every document.
    let err = kivadb::backup::check_log(&path).unwrap_err();
    assert!(err.contains("cannot be read"), "{err}");
}

#[test]
fn a_backup_of_typed_documents_restores_them_and_says_its_format() {
    let dir = TempDir::new("native_backup");
    let home = home_of_1_7_1(&dir);
    let plain = dir.path().join("plain.tar.gz");
    let native = dir.path().join("native.tar.gz");
    {
        let inst = open(&home);
        kivadb::backup::backup(&inst, &kivadb::backup::Scope::Home, &plain).unwrap();
        inst.engine("default").unwrap().put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
        kivadb::backup::backup(&inst, &kivadb::backup::Scope::Home, &native).unwrap();
        inst.shutdown();
    }
    let checked = kivadb::backup::check(&plain, dir.path()).unwrap();
    assert_eq!(checked.manifest["format"], json!(1), "no format-3 file: older versions restore it");
    let checked = kivadb::backup::check(&native, dir.path()).unwrap();
    assert_eq!(checked.manifest["format"], json!(2));

    let target = dir.path().join("restored");
    let opts = kivadb::backup::RestoreOptions { target: Some(target.clone()), ..Default::default() };
    kivadb::backup::restore(&native, &home, &opts).unwrap();
    assert_eq!(format_of(&target.join("data").join("store.kiva")), 3);
    let inst = open(&target);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_1_7_1_are_all_there(&mut s);
    assert_same(typed(&inst.engine("default").unwrap().get("o:1").unwrap().unwrap().0), &order());
}

#[test]
fn commands_that_would_turn_typed_values_into_texts_refuse_them() {
    let dir = TempDir::new("native_commands");
    let inst = open(dir.path());
    let mut s = Session::new(inst.clone(), true, None);
    let e = inst.engine("default").unwrap();
    e.put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
    for cmd in ["json set o:1 .id 8", "json incr o:1 .id 1", "json del o:1 .note", "json append o:1 .lines 1"] {
        let err = fails(&mut s, cmd);
        assert!(err.contains("holds decimal, date, time or datetime values"), "{cmd}: {err}");
    }
    assert_same(typed(&e.get("o:1").unwrap().unwrap().0), &order());
    // Reading works, in plain JSON.
    assert_eq!(ok(&mut s, "json get o:1 .total"), "o:1.total: 1250.50 [string]");
    assert!(ok(&mut s, "convert o:1 to msgpack").contains("stays in KDOC"));
    // A whole new document replaces it, here one JSON holds.
    ok(&mut s, r#"update o:1 {"id": 8}"#);
    assert_eq!(e.get("o:1").unwrap().unwrap().1.doc_enc(), DocEnc::Json);
}

#[test]
fn the_change_journal_carries_typed_documents() {
    let dir = TempDir::new("native_changes");
    let e = engine(&dir.path().join("store.kiva"));
    e.enable_changes(kivadb::changes::Settings::default()).unwrap();
    e.put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
    let r = e.read_changes(kivadb::engine::ChangesFrom::Beginning, 10, None, None).unwrap();
    assert_eq!(r.changes.len(), 1);
    assert_eq!(r.changes[0].enc, DocEnc::Kdoc);
    let j = kivadb::changes::to_json(&r.changes[0], "c");
    assert_eq!(j["value"]["total"], json!("1250.50"));
    assert_eq!(j["value"]["day"], json!("2026-10-31"));
}

#[test]
fn a_replica_applies_typed_documents_as_the_master_wrote_them() {
    // What the replication stream carries for a record: its type, encoding and bytes.
    let dir = TempDir::new("native_replica_apply");
    let master = engine(&dir.path().join("master.kiva"));
    master.put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
    let (_, entry) = master.get("o:1").unwrap().unwrap();
    let raw = doc::encode(&order());
    let replica = engine(&dir.path().join("replica.kiva"));
    replica.put_raw("o:1", entry.ty, &raw, entry.doc_enc(), entry.timestamp, 0).unwrap();
    assert_same(typed(&replica.get("o:1").unwrap().unwrap().0), &order());
    drop(replica);
    assert_eq!(format_of(&dir.path().join("replica.kiva")), 3);
}

// ---------------------------------------------------------------------------
// Older versions, run for real when CI gives their executables
// ---------------------------------------------------------------------------

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// What a master answers to a replica that says `hello`.
fn answer_to_replica(port: u16, hello: &str) -> (u16, String) {
    use kivadb::proto::*;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write_frame(&mut s, CMD_HELLO, &[]).unwrap();
    read_frame(&mut s).unwrap();
    write_frame(&mut s, CMD_REPL_SYNC, hello.as_bytes()).unwrap();
    let (code, payload) = read_frame(&mut s).unwrap();
    (code, String::from_utf8_lossy(&payload).into_owned())
}

#[test]
fn a_replica_that_cannot_read_kdoc_is_refused_before_any_data() {
    let dir = TempDir::new("native_replica_refused");
    let inst = open(dir.path());
    inst.engine("default").unwrap().put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
    let port = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    // The hello of 2.0.0-rc.2.
    let (code, msg) = answer_to_replica(port, "127.0.0.1:1 node=rc2 repl=3");
    assert_eq!(code, kivadb::proto::ST_ERR);
    assert!(msg.contains("upgrade the replica to KivaDB"), "{msg}");
    // This version's is accepted.
    let (code, _) = answer_to_replica(port, "127.0.0.1:1 node=new repl=3 native");
    assert_ne!(code, kivadb::proto::ST_ERR);
    inst.shutdown();
}

/// The executable of an older KivaDB named by `var`, when CI gives one.
fn old_bin(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_file())
}

fn run_old(bin: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let out = std::process::Command::new(bin)
        .arg("--home")
        .arg(home)
        .arg("--no-listen")
        .args(args)
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn older_versions_refuse_a_home_holding_typed_documents_and_leave_it_as_it_is() {
    let bins: Vec<(&str, PathBuf)> = [("rc.2", "KIVADB_RC2_BIN"), ("1.7.1", "KIVADB_V1_BIN")]
        .into_iter()
        .filter_map(|(name, var)| old_bin(var).map(|b| (name, b)))
        .collect();
    let dir = TempDir::new("native_old_versions");
    let home = home_of_1_7_1(&dir);
    let archive = dir.path().join("native.tar.gz");
    {
        let inst = open(&home);
        inst.engine("default").unwrap().put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
        kivadb::backup::backup(&inst, &kivadb::backup::Scope::Home, &archive).unwrap();
        inst.shutdown();
    }
    let before = snapshot(&home);
    for (name, bin) in &bins {
        let (success, text) = run_old(bin, &home, &["-e", "get greeting"]);
        assert!(!success, "{name}: {text}");
        assert!(text.contains("unsupported storage format 3"), "{name}: {text}");
        assert_eq!(snapshot(&home), before, "{name} must not touch the home");

        // Its archive is refused before anything is restored.
        let target = dir.path().join(format!("restored-{name}"));
        let elsewhere = dir.path().join(format!("home-{name}"));
        let (success, text) = run_old(
            bin,
            &elsewhere,
            &["restore", &archive.display().to_string(), "--target", &target.display().to_string()],
        );
        assert!(!success, "{name}: {text}");
        assert!(text.contains("is not supported") || text.contains("unsupported storage format 3"), "{name}: {text}");
        assert!(!target.join("data").join("store.kiva").exists(), "{name} restored a file it cannot open");
    }
    // And this version still reads it all.
    let inst = open(&home);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_1_7_1_are_all_there(&mut s);
    assert_same(typed(&inst.engine("default").unwrap().get("o:1").unwrap().unwrap().0), &order());
}

/// A server of an older KivaDB in its own process, stopped when dropped.
struct OldServer(std::process::Child);

impl OldServer {
    fn start(bin: &Path, home: &Path, port: u16) -> OldServer {
        let child = std::process::Command::new(bin)
            .arg("--home")
            .arg(home)
            .args(["serve", "--port", &port.to_string(), "--no-http", "--no-resp"])
            .env("KIVADB_NO_UPDATE_CHECK", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let server = OldServer(child);
        wait_until("the older KivaDB to listen", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
        server
    }
}

impl Drop for OldServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
fn a_real_replica_of_2_0_0_rc_2_keeps_its_data_and_says_why_it_does_not_follow() {
    let Some(bin) = old_bin("KIVADB_RC2_BIN") else { return };
    let dir = TempDir::new("native_rc2_replica");
    let inst = open(&dir.path().join("master"));
    inst.engine("default").unwrap().put("o:1", &Value::typed(order()).unwrap(), 0, false).unwrap();
    let port = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();

    let old = dir.path().join("rc2");
    assert!(run_old(&bin, &old, &["-e", "set kept 1"]).0);
    let old_port = free_port();
    let server = OldServer::start(&bin, &old, old_port);
    let cli = |cmd: &str| {
        let o = std::process::Command::new(&bin)
            .arg("--home")
            .arg(&old)
            .args(["--port", &old_port.to_string(), "-e", cmd])
            .env("KIVADB_NO_UPDATE_CHECK", "1")
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    assert!(cli(&format!("replica of 127.0.0.1 {port}")).contains("Now replicating"));
    wait_until("rc.2 to be refused", || cli("replica status").contains("upgrade the replica to KivaDB"));
    assert_eq!(cli("get kept"), "kept: 1 [number]\n");
    assert_eq!(cli("get o:1"), "o:1: (nil)\n");
    cli("replica stop");
    drop(server);
    inst.shutdown();
}

#[test]
fn a_home_without_typed_documents_still_opens_in_2_0_0_rc_2() {
    let Some(bin) = old_bin("KIVADB_RC2_BIN") else { return };
    let dir = TempDir::new("native_back_to_rc2");
    let home = home_of_1_7_1(&dir);
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, r#"set native:1 {"n": 1, "f": 2.5}"#);
        ok(&mut s, "set date born '2004-05-09'");
        ok(&mut s, "json set native:1 .n 2");
        drop(s);
        inst.shutdown();
    }
    assert_eq!(format_of(&home.join("data").join("store.kiva")), 2);
    let (success, text) = run_old(&bin, &home, &["-e", "get native:1.n", "-e", "get born", "-e", "get greeting"]);
    assert!(success, "{text}");
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        ["native:1.n: 2 [number]", "born: 2004-05-09 [date]", "greeting: hello [string]"]
    );
}
