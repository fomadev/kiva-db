// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Typed literals of the shell (lot C4 of docs/architecture/NATIVE_DOCUMENTS.md,
//! section 19): values typed at any depth, exact conditions on decimals,
//! increments with a typed operand, and `json set` of a typed value.

mod common;

use common::{fails, ok, TempDir};
use kivadb::doc;
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use kivadb::value::{DocEnc, Value};
use serde_json::{json, Value as Json};
use std::path::Path;
use std::sync::Arc;

fn open(dir: &TempDir) -> Arc<Instance> {
    Instance::open(dir.path(), Options::default()).unwrap()
}

/// What a key holds, in extended JSON: every type told apart.
fn ext(inst: &Instance, key: &str) -> Json {
    match inst.engine("default").unwrap().get(key).unwrap().unwrap().0 {
        Value::Typed(d) => doc::to_extended_json(&d),
        Value::Doc(j) => j,
        other => panic!("not a document: {other:?}"),
    }
}

fn enc(inst: &Instance, key: &str) -> DocEnc {
    inst.engine("default").unwrap().get(key).unwrap().unwrap().1.doc_enc()
}

/// The highest storage format of the data files under `dir`.
fn formats(dir: &Path) -> u32 {
    let mut best = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            best = best.max(formats(&p));
        } else if p.extension().is_some_and(|x| x == "kiva") {
            let h = std::fs::read(&p).unwrap();
            if h.len() >= 8 {
                best = best.max(u32::from_le_bytes([h[4], h[5], h[6], h[7]]));
            }
        }
    }
    best
}

const ACCOUNT: &str = r#"set account:1 {
  "id": uuid "550E8400E29B41D4A716446655440000",
  "age": int 25,
  "balance": decimal "1250.50",
  "created_at": datetime "2026-10-31T16:18:55Z",
  "values": [int 8, float 0.5, decimal "0.10", date "2026-10-31"],
  "owner": {"since": date "2020-01-15", "limits": [decimal "500", decimal "2500.00"]}
}"#;

fn account() -> Json {
    json!({
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "age": 25,
        "balance": {"$decimal": "1250.50"},
        "created_at": {"$datetime": "2026-10-31T16:18:55.000Z"},
        "values": [8, 0.5, {"$decimal": "0.10"}, {"$date": "2026-10-31"}],
        "owner": {"since": {"$date": "2020-01-15"}, "limits": [{"$decimal": "500"}, {"$decimal": "2500.00"}]}
    })
}

#[test]
fn typed_literals_keep_their_types_at_any_depth_through_reopening() {
    let dir = TempDir::new("tl_roundtrip");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "timezone UTC");
    ok(&mut s, ACCOUNT);
    assert_eq!(ext(&inst, "account:1"), account());
    assert_eq!(enc(&inst, "account:1"), DocEnc::Kdoc);
    assert_eq!(ok(&mut s, "json type account:1 .id"), "account:1.id: [string]");
    assert_eq!(ok(&mut s, "json type account:1 .age"), "account:1.age: [number]", "as in C3");
    assert_eq!(ok(&mut s, "json type account:1 .values[1]"), "account:1.values[1]: [number]");
    assert_eq!(ok(&mut s, "json type account:1 .values[2]"), "account:1.values[2]: [decimal]");
    assert_eq!(ok(&mut s, "json get account:1 .owner.since"), "account:1.owner.since: 2020-01-15 [date]");

    // Shown in the typed syntax, which can be typed back.
    let shown = ok(&mut s, "get account:1");
    assert!(shown.contains(r#""balance": decimal "1250.50""#), "{shown}");
    let literal = shown.strip_prefix("account:1: ").unwrap().strip_suffix(" [object]").unwrap();
    ok(&mut s, &format!("set account:2 {literal}"));
    assert_eq!(ext(&inst, "account:2"), account());

    // `update`, `add` and arrays at the top take them too.
    ok(&mut s, r#"update account:2 {"balance": decimal "1.00", "list": [time "08:30"]}"#);
    assert_eq!(ext(&inst, "account:2"), json!({"balance": {"$decimal": "1.00"}, "list": [{"$time": "08:30:00"}]}));
    let added = ok(&mut s, r#"add row: [int 8, string "fordi", datetime "2026-10-31 16:18:55"]"#);
    let key = added.strip_prefix("OK: ").unwrap().strip_suffix(" saved.").unwrap().to_string();
    assert_eq!(ext(&inst, &key), json!([8, "fordi", {"$datetime": "2026-10-31T16:18:55.000Z"}]));

    drop(s);
    inst.shutdown();
    drop(inst);
    let inst = open(&dir);
    assert_eq!(ext(&inst, "account:1"), account());
    assert_eq!(formats(dir.path()), 3);
}

#[test]
fn a_literal_of_json_types_only_stays_json_and_keeps_the_format() {
    let dir = TempDir::new("tl_json_only");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(
        &mut s,
        r#"set j:1 {"id": uuid "550e8400e29b41d4a716446655440000", "n": int 8, "f": float 8, "s": string "x", "b": bool true}"#,
    );
    assert_eq!(enc(&inst, "j:1"), DocEnc::Json);
    let j = ext(&inst, "j:1");
    assert_eq!(j, json!({"id": "550e8400-e29b-41d4-a716-446655440000", "n": 8, "f": 8.0, "s": "x", "b": true}));
    assert!(j["f"].is_f64(), "float 8 is the float 8.0");
    // Plain JSON is read as before, a type word in a text or a key is text.
    ok(&mut s, r#"set j:2 {"int 8": "decimal 1"}"#);
    assert_eq!(ext(&inst, "j:2"), json!({"int 8": "decimal 1"}));
    assert!(formats(dir.path()) < 3);
}

#[test]
fn refusals_name_the_line_the_column_and_the_path() {
    let dir = TempDir::new("tl_refusals");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    let cases = [
        (r#"set r:1 {"a": int 1.5}"#, "(.a)", "not an int"),
        (r#"set r:1 {"p": decimal "0.12345678901234567890123456789"}"#, "(.p)", "does not fit a decimal"),
        (r#"set r:1 {"d": [date "2026-02-31"]}"#, "(.d[0])", "date"),
        (r#"set r:1 {"u": uuid "550e8400"}"#, "(.u)", "not a uuid"),
        (r#"set r:1 [int 8, datetime created_at "2026-10-31"]"#, "([1])", ""),
        (r#"set r:1 {"a": int 1, "x": nope 1}"#, "(.x)", "not a type"),
    ];
    for (cmd, path, what) in cases {
        let e = fails(&mut s, cmd);
        assert!(e.contains("line 1, column") && e.contains(path) && e.contains(what), "{cmd}: {e}");
    }
    // A mistake in plain JSON keeps the message of before.
    assert!(fails(&mut s, r#"set r:1 {"a": hello}"#).contains("Invalid JSON literal"));
    assert!(fails(&mut s, r#"set r:1 {"x": nope 1}"#).contains("Invalid JSON literal"));
    // The session's wall clock: an hour skipped by the change to summer time.
    ok(&mut s, "timezone Europe/Paris");
    let e = fails(&mut s, r#"set r:1 {"at": datetime "2026-03-29 02:30:00"}"#);
    assert!(e.contains("(.at)"), "{e}");
}

#[test]
fn conditions_on_decimal_literals_give_the_same_keys_with_and_without_indexes() {
    let dir = TempDir::new("tl_conditions");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set p:1 {"price": decimal "0.1", "age": int 25}"#);
    ok(&mut s, r#"set p:2 {"price": decimal "1250.50", "age": 30}"#);
    ok(&mut s, r#"set p:3 {"price": 1250.5, "age": 25}"#);
    ok(&mut s, r#"set p:4 {"price": 0.1, "age": 40}"#);
    ok(&mut s, r#"set p:5 {"price": decimal "0.10", "id": uuid "550E8400-E29B-41D4-A716-446655440000"}"#);

    let keys = |s: &mut Session, cond: &str| -> Vec<String> {
        let out = ok(s, &format!("get where {cond}"));
        let mut k: Vec<String> = out.lines().filter_map(|l| l.split_once(": ").map(|(k, _)| k.to_string())).collect();
        k.sort();
        k
    };
    let check = |s: &mut Session| {
        assert_eq!(keys(s, r#"p:*.price = decimal "0.1""#), ["p:1", "p:5"], "0.1 = 0.10, not the float 0.1");
        assert_eq!(keys(s, r#"p:*.price = decimal "1250.50""#), ["p:2", "p:3"], "the float 1250.5 is exactly it");
        assert_eq!(keys(s, "p:*.price = 0.1"), ["p:4"], "the float 0.1 is never made a decimal");
        assert_eq!(keys(s, "p:*.price = float 0.1"), ["p:4"]);
        assert_eq!(keys(s, r#"p:*.price in [decimal "0.10", decimal "1250.50"]"#), ["p:1", "p:2", "p:3", "p:5"]);
        assert_eq!(keys(s, r#"p:*.price >= decimal "1000""#), ["p:2", "p:3"]);
        assert_eq!(keys(s, r#"p:*.price < decimal "0.1""#), Vec::<String>::new());
        assert_eq!(keys(s, r#"p:*.price >= decimal "1000" and age = int 25"#), ["p:3"]);
        assert_eq!(keys(s, r#"p:*.price = decimal "0.1" or age = int 40"#), ["p:1", "p:4", "p:5"]);
        assert_eq!(keys(s, r#"p:*.id = uuid "550e8400e29b41d4a716446655440000""#), ["p:5"]);
        assert_eq!(ok(s, r#"count where p:*.price != decimal "0.1""#), "Count: 3");
    };
    check(&mut s);
    ok(&mut s, "create index on p:* (price)");
    assert!(ok(&mut s, r#"explain get where p:*.price = decimal "0.1""#).contains("INDEX SCAN"));
    assert!(ok(&mut s, r#"explain get where p:*.price >= decimal "1000""#).contains("INDEX SCAN"));
    check(&mut s);
    ok(&mut s, "drop index on p:* (price)");
    ok(&mut s, "create index on p:* (price) using hash");
    assert!(ok(&mut s, r#"explain get where p:*.price = decimal "0.1""#).contains("INDEX SCAN"));
    check(&mut s);
    drop(s);
    inst.shutdown();
    drop(inst);
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    check(&mut s);
    // The condition reads back as it was written.
    assert!(ok(&mut s, r#"explain get where p:*.price = decimal "0.10""#).contains(r#"price = decimal "0.10""#));
}

#[test]
fn increments_take_a_typed_operand() {
    let dir = TempDir::new("tl_incr");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set a:1 {"balance": decimal "1000.00", "ratio": float 0.5, "count": int 1}"#);
    assert_eq!(ok(&mut s, r#"json incr a:1 .balance decimal "-100.25""#), "a:1.balance: 899.75 [decimal]");
    assert_eq!(ok(&mut s, "json incr a:1 .balance decimal 0.25"), "a:1.balance: 900.00 [decimal]");
    assert_eq!(ok(&mut s, "json incr a:1 .count int 3"), "a:1.count: 4 [number]");
    assert_eq!(ok(&mut s, "json incr a:1 .balance int 1"), "a:1.balance: 901.00 [decimal]");
    assert!(ok(&mut s, "json incr a:1 .ratio float 0.25").starts_with("a:1.ratio: 0.75"));
    let e = fails(&mut s, "json incr a:1 .balance float 0.1");
    assert!(e.contains("do not mix"), "{e}");
    let e = fails(&mut s, r#"json incr a:1 .ratio decimal "0.1""#);
    assert!(e.contains("do not mix"), "{e}");
    assert!(
        fails(&mut s, r#"json incr a:1 .count uuid "550e8400e29b41d4a716446655440000""#).contains("must be a number")
    );
    // The untyped rule of C3 is unchanged.
    assert_eq!(ok(&mut s, "json incr a:1 .balance -1.25"), "a:1.balance: 899.75 [decimal]");
    assert_eq!(ext(&inst, "a:1")["balance"], json!({"$decimal": "899.75"}));

    // On a JSON document, a decimal operand makes the field a decimal.
    ok(&mut s, r#"set j:1 {"n": 10, "s": "x"}"#);
    assert_eq!(enc(&inst, "j:1"), DocEnc::Json);
    assert_eq!(ok(&mut s, r#"json incr j:1 .n decimal "0.5""#), "j:1.n: 10.5 [decimal]");
    assert_eq!(enc(&inst, "j:1"), DocEnc::Kdoc);
    // An int operand on a JSON document keeps it JSON.
    ok(&mut s, r#"set j:2 {"n": 10}"#);
    assert_eq!(ok(&mut s, "json incr j:2 .n int 5"), "j:2.n: 15 [number]");
    assert_eq!(enc(&inst, "j:2"), DocEnc::Json);
}

#[test]
fn json_set_append_insert_store_native_values_even_in_json_documents() {
    let dir = TempDir::new("tl_json_set");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "timezone UTC");
    ok(&mut s, r#"set o:1 {"name": "a", "tags": ["x"]}"#);
    assert_eq!(enc(&inst, "o:1"), DocEnc::Json);
    let before = formats(dir.path());
    assert!(before < 3);
    // Before C4 this wrote the text "2026-10-31"; option A of 19.6 stores a date.
    ok(&mut s, r#"json set o:1 .day date "2026-10-31""#);
    assert_eq!(enc(&inst, "o:1"), DocEnc::Kdoc);
    assert_eq!(formats(dir.path()), 3);
    ok(&mut s, r#"json set o:1 .price decimal "9.90""#);
    ok(&mut s, r#"json set o:1 .id uuid "550E8400E29B41D4A716446655440000""#);
    ok(&mut s, r#"json set o:1 .n int 7"#);
    ok(&mut s, r#"json append o:1 .tags decimal "1.5", date "2027-01-01", "y", int 2"#);
    ok(&mut s, r#"json insert o:1 .tags[0] time "08:30""#);
    ok(&mut s, r#"json set o:1 .sub {"at": datetime "2026-10-31T10:00:00Z", "v": [decimal "0.1"]}"#);
    assert_eq!(
        ext(&inst, "o:1"),
        json!({
            "name": "a",
            "tags": [{"$time": "08:30:00"}, "x", {"$decimal": "1.5"}, {"$date": "2027-01-01"}, "y", 2],
            "day": {"$date": "2026-10-31"},
            "price": {"$decimal": "9.90"},
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "n": 7,
            "sub": {"at": {"$datetime": "2026-10-31T10:00:00.000Z"}, "v": [{"$decimal": "0.1"}]}
        })
    );
    assert_eq!(ok(&mut s, "json type o:1 .day"), "o:1.day: [date]");
    // Queries answer as before: a native date is judged as its ISO text.
    assert_eq!(ok(&mut s, r#"count where o:*.day = date "2026-10-31""#), "Count: 1");
    assert_eq!(ok(&mut s, r#"count where o:*.day = "2026-10-31""#), "Count: 1");
    assert_eq!(ok(&mut s, r#"count where o:*.price = decimal "9.9""#), "Count: 1");

    // A plain value on a JSON document keeps the code of 2.0.
    ok(&mut s, r#"set o:2 {"a": 1}"#);
    ok(&mut s, r#"json set o:2 .b "2026-10-31""#);
    ok(&mut s, r#"json set o:2 .list [1]"#);
    ok(&mut s, "json append o:2 .list 2, 3");
    assert_eq!(ext(&inst, "o:2"), json!({"a": 1, "b": "2026-10-31", "list": [1, 2, 3]}));
    assert_eq!(enc(&inst, "o:2"), DocEnc::Json);
}

#[test]
fn a_unique_index_takes_a_typed_decimal_and_its_json_number_for_one_value() {
    let dir = TempDir::new("tl_unique");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, "create unique index on c:* (code)");
    ok(&mut s, r#"set c:1 {"code": decimal "10.50"}"#);
    assert!(fails(&mut s, r#"set c:2 {"code": 10.5}"#).contains("unique index"));
    assert!(fails(&mut s, r#"set c:3 {"code": decimal "10.5000"}"#).contains("unique index"));
    ok(&mut s, r#"set c:4 {"code": decimal "10.51"}"#);
}
