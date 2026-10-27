// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Optional schemas: defaults, constraints and unique fields, on every write
//! path, with the documents stored before the schema left as they are.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::engine::{Engine, EngineOptions};
use kivadb::session::Session;
use kivadb::value::{Type, Value};
use serde_json::{json, Value as Json};
use std::sync::Arc;

const USERS: &str = r#"schema set user: {
    "name": {"type": "string", "required": true, "minLength": 1, "maxLength": 40},
    "email": {"type": "string", "required": true, "unique": true, "pattern": "^[^@ ]+@[^@ ]+$"},
    "age": {"type": "integer", "min": 0, "max": 150},
    "role": {"enum": ["dev", "ops"], "default": "dev"},
    "active": {"type": "boolean", "default": true},
    "created_at": {"type": "datetime", "default": "now()"}
}"#;

fn kiva(dir: &TempDir) -> Session {
    Session::new(instance(dir), true, None)
}

fn doc(s: &mut Session, key: &str) -> Json {
    let out = ok(s, &format!("get {key}"));
    let text = out.split_once(": ").unwrap().1.trim_end_matches(" [object]");
    serde_json::from_str(text).unwrap()
}

#[test]
fn defaults_are_filled_and_rules_enforced() {
    let dir = TempDir::new("schema_basics");
    let mut s = kiva(&dir);
    assert_eq!(ok(&mut s, USERS), "Schema set on user:* (6 fields).\nUnique index created on user:* (email).");

    ok(&mut s, r#"set user:1 {"name": "Ana", "email": "ana@example.org", "extra": [1, 2]}"#);
    let d = doc(&mut s, "user:1");
    assert_eq!((&d["role"], &d["active"], &d["extra"]), (&json!("dev"), &json!(true), &json!([1, 2])));
    assert!(kivadb::time::parse_datetime(d["created_at"].as_str().unwrap(), &kivadb::time::Zone::Utc).is_ok());
    // A value given wins over the default.
    ok(&mut s, r#"set user:2 {"name": "Ben", "email": "ben@example.org", "role": "ops", "active": false}"#);
    assert_eq!(doc(&mut s, "user:2")["role"], "ops");

    let refused = |s: &mut Session, value: &str| fails(s, &format!("set user:9 {value}"));
    assert_eq!(
        refused(&mut s, r#"{"email": "x@y.z"}"#),
        "Error: SCHEMA: 'user:9' not written: name: required (schema on user:*)"
    );
    assert!(
        refused(&mut s, r#"{"name": "X", "email": "x@y.z", "age": 200}"#).contains("age: 200 is above the maximum 150")
    );
    assert!(refused(&mut s, r#"{"name": "X", "email": "x@y.z", "age": "old"}"#).contains("age: expected an integer"));
    assert!(refused(&mut s, r#"{"name": "X", "email": "nope"}"#).contains("does not match the pattern"));
    assert!(
        refused(&mut s, r#"{"name": "X", "email": "x@y.z", "role": "boss"}"#).contains(r#"is not one of "dev", "ops""#)
    );
    assert!(refused(&mut s, r#"{"name": "", "email": "x@y.z"}"#).contains("at least 1 needed"));
    assert!(refused(&mut s, r#""a text""#).contains("a JSON object is expected, not a [string]"));
    // The unique field goes through the unique index.
    assert!(refused(&mut s, r#"{"name": "X", "email": "ana@example.org"}"#).starts_with("Error: UNIQUE:"));
    assert!(!ok(&mut s, "keys").contains("user:9"), "nothing written");

    // Keys outside the pattern are free.
    ok(&mut s, r#"set other:1 {"anything": true}"#);
    assert_eq!(ok(&mut s, "schemas"), "user:* - name, email, age, role, active, created_at (unique: email)");
    assert!(ok(&mut s, "schema show user:").starts_with("schema set user:* {"));
    assert_eq!(ok(&mut s, "schema check user:"), "user:*: 2 document(s), all follow the schema.");
}

#[test]
fn nested_objects_arrays_dates_and_nullable_fields() {
    let dir = TempDir::new("schema_nested");
    let mut s = kiva(&dir);
    ok(
        &mut s,
        r#"schema set order: {
            "items": {"type": "array", "minItems": 1, "maxItems": 3,
                      "items": {"type": "object", "properties": {"sku": {"type": "string", "required": true},
                                                                "qty": {"type": "integer", "min": 1}}}},
            "ship": {"type": "object", "properties": {"city": {"type": "string", "required": true},
                                                     "country": {"type": "string", "default": "CD"}}},
            "due": {"type": "date"},
            "opens": {"type": "time"},
            "paid_at": {"type": "datetime", "nullable": true}
        }"#,
    );
    ok(
        &mut s,
        r#"set order:1 {"items": [{"sku": "A", "qty": 2}], "ship": {"city": "Goma"}, "due": "2026-11-02", "opens": "08:00", "paid_at": null}"#,
    );
    assert_eq!(doc(&mut s, "order:1")["ship"]["country"], "CD");
    let bad = |s: &mut Session, v: &str| fails(s, &format!("set order:2 {v}"));
    assert!(bad(&mut s, r#"{"items": []}"#).contains("items: 0 items, at least 1 needed"));
    assert!(bad(&mut s, r#"{"items": [{"qty": 1}]}"#).contains("items[0].sku: required"));
    assert!(bad(&mut s, r#"{"items": [{"sku": "A", "qty": 0}]}"#).contains("items[0].qty: 0 is below the minimum 1"));
    assert!(bad(&mut s, r#"{"ship": {"country": "RW"}}"#).contains("ship.city: required"));
    assert!(bad(&mut s, r#"{"due": "2026-02-30"}"#).contains("due: expected a date"));
    assert!(bad(&mut s, r#"{"opens": "25:00"}"#).contains("opens: expected a time"));
    assert!(bad(&mut s, r#"{"paid_at": "2026-10-27 14:30"}"#).contains("with its offset"));
    ok(&mut s, r#"set order:3 {"paid_at": "2026-10-27T14:30:00+01:00"}"#);
}

#[test]
fn a_broken_schema_or_a_duplicate_changes_nothing() {
    let dir = TempDir::new("schema_refused");
    let mut s = kiva(&dir);
    assert!(fails(&mut s, r#"schema set user: {"name": {"tpye": "string"}}"#).contains("unknown rule 'tpye'"));
    assert!(fails(&mut s, r#"schema set user: {"n": {"type": "integer", "default": "x"}}"#).contains("the default"));
    // Two documents already share an email: no unique index, so no schema.
    ok(&mut s, r#"set user:1 {"name": "A", "email": "same@x.y"}"#);
    ok(&mut s, r#"set user:2 {"name": "B", "email": "same@x.y"}"#);
    assert!(fails(&mut s, USERS).contains("both hold \"same@x.y\""));
    assert_eq!(ok(&mut s, "schemas"), "No schemas in this workspace.");
    assert_eq!(ok(&mut s, "indexes"), "No secondary indexes in this workspace.");
    // A plain index on the field must be dropped first.
    ok(&mut s, "del user:2");
    ok(&mut s, "create index on user: (email)");
    assert!(fails(&mut s, USERS).contains("is not: drop it"));
    ok(&mut s, "drop index on user: (email)");
    ok(&mut s, USERS);
    // An existing unique index is used, and stays when the schema goes.
    ok(&mut s, "schema drop user:");
    ok(&mut s, "create unique index on user: (email)");
    assert_eq!(ok(&mut s, USERS), "Schema set on user:* (6 fields).\n1 document(s) already stored: all follow it.");
    assert_eq!(ok(&mut s, "schema drop user:"), "Schema dropped on user:*.");
    assert!(ok(&mut s, "indexes").contains("user:* (email) - unique"));
}

#[test]
fn documents_stored_before_the_schema_stay_as_they_are() {
    let dir = TempDir::new("schema_existing");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set user:old {"name": "Old"}"#);
    ok(&mut s, r#"set user:fine {"name": "Fine", "email": "fine@x.y"}"#);
    let out = ok(&mut s, USERS);
    assert!(out.ends_with("2 document(s) already stored, 1 of them do not follow it: they stay as they are, and must follow it at their next change (schema check user:*)."), "{out}");
    assert_eq!(doc(&mut s, "user:old"), json!({"name": "Old"}), "not changed, no default added");
    assert_eq!(doc(&mut s, "user:fine"), json!({"name": "Fine", "email": "fine@x.y"}));
    assert_eq!(
        ok(&mut s, "schema check user:"),
        "user:*: 2 document(s), 1 do not follow the schema:\n  - user:old: email: required"
    );
    // Reading, deleting and a TTL change work; a change must follow the schema.
    ok(&mut s, "bump user:old set 600");
    assert!(fails(&mut s, r#"json set user:old .name "Older""#).contains("email: required"));
    assert_eq!(doc(&mut s, "user:old"), json!({"name": "Old"}));
    ok(&mut s, r#"json set user:old .email "old@x.y""#);
    let d = doc(&mut s, "user:old");
    assert_eq!((&d["email"], &d["role"]), (&json!("old@x.y"), &json!("dev")), "defaults filled at the change");
    ok(&mut s, "del user:fine");
}

#[test]
fn a_partial_update_is_judged_by_the_whole_document() {
    let dir = TempDir::new("schema_update");
    let mut s = kiva(&dir);
    ok(&mut s, USERS);
    ok(&mut s, r#"set user:1 {"name": "Ana", "email": "ana@x.y", "age": 30}"#);
    let before = doc(&mut s, "user:1");
    assert!(fails(&mut s, "json set user:1 .age -4").contains("age: -4 is below the minimum 0"));
    assert!(fails(&mut s, "json del user:1 .name").contains("name: required"));
    assert!(fails(&mut s, r#"update user:1 {"name": "Ana"}"#).contains("email: required"));
    assert_eq!(doc(&mut s, "user:1"), before, "a refused change leaves the document whole");
    ok(&mut s, "json set user:1 .age 31");
    let after = doc(&mut s, "user:1");
    assert_eq!(after["age"], 31);
    assert_eq!(after["created_at"], before["created_at"], "a default is filled once, not at every change");
}

#[test]
fn a_transaction_is_judged_at_commit_and_refused_whole() {
    let dir = TempDir::new("schema_txn");
    let mut s = kiva(&dir);
    ok(&mut s, USERS);
    ok(&mut s, "begin");
    ok(&mut s, r#"set user:1 {"name": "Ana", "email": "ana@x.y"}"#);
    ok(&mut s, r#"set user:2 {"name": "Ben"}"#);
    ok(&mut s, "set plain 1");
    assert!(fails(&mut s, "commit").contains("'user:2' not written: email: required"));
    assert_eq!(ok(&mut s, "get user:1 and plain"), "user:1: (nil)\nplain: (nil)", "nothing of it written");

    // Every default of one transaction is the moment of its commit.
    ok(&mut s, "begin");
    for i in 1..=5 {
        ok(&mut s, &format!(r#"set user:{i} {{"name": "U{i}", "email": "u{i}@x.y"}}"#));
    }
    ok(&mut s, "commit");
    let first = doc(&mut s, "user:1")["created_at"].clone();
    for i in 2..=5 {
        assert_eq!(doc(&mut s, &format!("user:{i}"))["created_at"], first);
    }
    // Rolled back: nothing to judge.
    ok(&mut s, "begin");
    ok(&mut s, r#"set user:9 {"bad": true}"#);
    ok(&mut s, "rollback");
    assert_eq!(ok(&mut s, "get user:9"), "user:9: (nil)");
    // A unique email taken twice inside one transaction.
    ok(&mut s, "begin");
    ok(&mut s, r#"set user:7 {"name": "A", "email": "twice@x.y"}"#);
    ok(&mut s, r#"set user:8 {"name": "B", "email": "twice@x.y"}"#);
    assert!(fails(&mut s, "commit").starts_with("Error: UNIQUE:"));
}

#[test]
fn concurrent_writers_never_share_a_unique_field() {
    let dir = TempDir::new("schema_concurrent");
    let inst = instance(&dir);
    ok(&mut Session::new(inst.clone(), true, None), USERS);
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let inst = inst.clone();
            std::thread::spawn(move || {
                let mut s = Session::new(inst, true, None);
                let out = s.execute(&format!(r#"set user:{i} {{"name": "U{i}", "email": "same@x.y"}}"#));
                !out.has_err()
            })
        })
        .collect();
    let written = handles.into_iter().map(|h| h.join().unwrap()).filter(|w| *w).count();
    assert_eq!(written, 1);
    let mut s = Session::new(inst, true, None);
    assert_eq!(ok(&mut s, r#"count where user:*.email = "same@x.y""#), "Count: 1");
}

#[test]
fn schemas_survive_a_restart_and_a_compaction() {
    let dir = TempDir::new("schema_restart");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, USERS);
        ok(&mut s, r#"set user:1 {"name": "Ana", "email": "ana@x.y"}"#);
        ok(&mut s, "compact");
        drop(s);
        inst.shutdown();
    }
    let mut s = kiva(&dir);
    assert!(ok(&mut s, "schemas").starts_with("user:* - name"));
    assert!(fails(&mut s, r#"set user:2 {"name": "Ben"}"#).contains("email: required"));
    assert!(fails(&mut s, r#"set user:2 {"name": "Ben", "email": "ana@x.y"}"#).starts_with("Error: UNIQUE:"));
    ok(&mut s, "schema drop user:");
    ok(&mut s, r#"set user:2 {"name": "Ben"}"#);
}

#[test]
fn scripts_obey_the_schema_and_replicated_writes_are_kept_as_sent() {
    let dir = TempDir::new("schema_paths");
    let mut s = kiva(&dir);
    ok(&mut s, USERS);
    let out = fails(&mut s, r#"eval 'kiva.set("user:5", {name = "Lua"})'"#);
    assert!(out.contains("SCHEMA") && out.contains("email: required"), "{out}");

    // What a master sends is applied as it is: it was judged there.
    let engine_dir = TempDir::new("schema_replica");
    let e: Arc<Engine> = Engine::open(engine_dir.path().join("store.kiva"), EngineOptions::default()).unwrap();
    e.set_schema("user:*", &json!({"email": {"type": "string", "required": true}})).unwrap();
    assert!(e.put("user:1", &Value::Doc(json!({"name": "x"})), 0, false).is_err());
    e.put_raw("user:1", Type::Object, br#"{"name":"x"}"#, false, 0, 0).unwrap();
    assert_eq!(e.get("user:1").unwrap().unwrap().0, Value::Doc(json!({"name": "x"})));
}
