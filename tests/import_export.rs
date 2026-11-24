// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot G of docs/architecture/PROJECTS_AND_COLLECTIONS.md (section 8, decision
//! D10): `export` writes a collection's keys as NDJSON in extended JSON,
//! `import` reads them back with their types, checks the whole file before
//! writing, writes nothing with `dry run`, and gives the same keys when run
//! twice.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::session::Session;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn kivadb(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .arg("--home")
        .arg(home)
        .args(args)
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .env_remove("KIVADB_ENCRYPTION_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

/// Every type, with a TTL on one key.
fn fill(s: &mut Session) {
    for cmd in [
        r#"set name "Ana""#,
        "set count 42",
        "set float ratio 0.5",
        r#"set decimal balance "1250.50""#,
        "set flag true",
        r#"set bytes note "Hello""#,
        r#"set date born "2004-05-09""#,
        r#"set datetime at "2026-10-27 14:30:00 +01:00""#,
        r#"set time opens "08:30:00""#,
        r#"set tags ["a", "b"]"#,
        r#"set acc:1 {"name": "Ana", "balance": decimal "1250.50", "opened": date "2026-01-15", "n": 3}"#,
        r#"set plain:1 {"city": "Goma", "n": 1.5}"#,
        r#"set session:1 "abc" ttl 3600"#,
    ] {
        ok(s, cmd);
    }
    // A number of 1.x, neither int nor float: no command writes one any more.
    s.engine.put("big", &kivadb::value::Value::Num("123456789012345678901234567890".into()), 0, false).unwrap();
}

#[test]
fn a_collection_goes_out_and_back_with_its_types() {
    let dir = TempDir::new("ndjson_roundtrip");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    fill(&mut s);
    let file = dir.path().join("out.ndjson");
    let said = ok(&mut s, &format!("export to \"{}\"", file.display()));
    assert_eq!(said, format!("Exported 14 key(s) of 'default' to {}.", file.display()));

    // One JSON object per line, extended JSON for the values.
    let text = std::fs::read_to_string(&file).unwrap();
    let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 14);
    let line = |k: &str| lines.iter().find(|l| l["key"] == k).unwrap_or_else(|| panic!("{k}")).clone();
    assert_eq!(line("balance")["type"], "decimal");
    assert_eq!(line("balance")["value"], serde_json::json!({"$decimal": "1250.50"}));
    assert_eq!(line("count")["type"], "int");
    assert_eq!(line("big")["type"], "number");
    assert_eq!(line("big")["value"], "123456789012345678901234567890");
    assert_eq!(line("born")["value"], serde_json::json!({"$date": "2004-05-09"}));
    assert_eq!(line("acc:1")["value"]["balance"], serde_json::json!({"$decimal": "1250.50"}));
    assert!(line("session:1")["expires_at"].is_string());
    assert!(line("name")["expires_at"].is_null());

    // Into another collection: the same values, with their types and TTL.
    ok(&mut s, "workspace create copy");
    ok(&mut s, "use copy");
    let imported = ok(&mut s, &format!("import from \"{}\"", file.display()));
    assert_eq!(imported, "Imported into 'copy': 14 key(s) written (14 new, 0 replaced).");
    for key in
        ["name", "count", "ratio", "balance", "big", "flag", "note", "born", "at", "opens", "tags", "acc:1", "plain:1"]
    {
        ok(&mut s, "use default");
        let original = ok(&mut s, &format!("get {key}"));
        ok(&mut s, "use copy");
        assert_eq!(ok(&mut s, &format!("get {key}")), original, "{key}");
    }
    // The deadline itself, to the millisecond.
    let copied = s.kv_get("session:1").unwrap().unwrap().1.expires_at;
    ok(&mut s, "use default");
    let original = s.kv_get("session:1").unwrap().unwrap().1.expires_at;
    ok(&mut s, "use copy");
    assert!(copied > 0 && copied == original, "{copied} {original}");

    // A second run gives the same keys: those present are kept.
    let again = ok(&mut s, &format!("import from \"{}\"", file.display()));
    assert_eq!(
        again,
        "Imported into 'copy': 0 key(s) written (0 new, 0 replaced), 14 kept as they were (already present; \
         --replace overwrites them)."
    );
    ok(&mut s, r#"update name "Changed""#);
    ok(&mut s, &format!("import from \"{}\" replace", file.display()));
    assert_eq!(ok(&mut s, "get name"), "name: Ana [string]");
    // A pattern exports part of it.
    let part = dir.path().join("part.ndjson");
    ok(&mut s, &format!("export to \"{}\" keys acc:*", part.display()));
    assert_eq!(std::fs::read_to_string(&part).unwrap().lines().count(), 1);
}

#[test]
fn a_wrong_line_refuses_the_file_and_a_dry_run_writes_nothing() {
    let dir = TempDir::new("ndjson_checks");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let file = dir.path().join("in.ndjson");
    let good = [
        r#"{"key": "a", "type": "string", "value": "x"}"#,
        r#"{"key": "b", "type": "decimal", "value": {"$decimal": "0.10"}, "expires_at": null}"#,
        r#"{"key": "c", "type": "int", "value": 7, "expires_at": "2000-01-01T00:00:00Z"}"#,
        "",
    ];
    std::fs::write(&file, good.join("\n")).unwrap();
    let dry = ok(&mut s, &format!("import from \"{}\" dry run", file.display()));
    assert_eq!(
        dry,
        "Dry run: 3 line(s) checked; 2 key(s) would be written to 'default' (2 new, 0 replaced), 1 skipped \
         (their expires_at is past). Nothing was written."
    );
    assert!(!s.engine.contains("a"), "a dry run writes nothing");

    // Wrong lines: listed with their numbers, and nothing written.
    let bad = [
        r#"{"key": "a", "type": "string", "value": "x"}"#,
        "not json",
        r#"{"key": "d", "type": "int", "value": "seven"}"#,
        r#"{"key": "e", "type": "colour", "value": 1}"#,
        r#"{"type": "string", "value": "x"}"#,
        r#"{"key": "f", "type": "date", "value": "2026-13-45"}"#,
    ];
    std::fs::write(&file, bad.join("\n")).unwrap();
    let err = fails(&mut s, &format!("import from \"{}\"", file.display()));
    assert!(err.contains("5 line(s) are wrong, and nothing was written"), "{err}");
    for n in 2..=6 {
        assert!(err.contains(&format!("line {n}: ")), "line {n}: {err}");
    }
    assert!(err.contains("unknown type 'colour'") && err.contains("\"key\" is missing"), "{err}");
    assert!(!s.engine.contains("a"));

    // A schema is checked before anything is written, in a dry run too.
    ok(&mut s, r#"schema set user: {"age": {"type": "int", "required": true}}"#);
    let users = [
        r#"{"key": "user:1", "type": "object", "value": {"age": 3}}"#,
        r#"{"key": "user:2", "type": "object", "value": {"name": "x"}}"#,
    ];
    std::fs::write(&file, users.join("\n")).unwrap();
    let err = fails(&mut s, &format!("import from \"{}\" dry run", file.display()));
    assert!(err.contains("line 2: ") && err.contains("schema"), "{err}");
    assert!(!s.engine.contains("user:1"));
    assert!(fails(&mut s, "import from").contains("Usage: import from"));
    assert!(fails(&mut s, "export").contains("Usage: export to"));
}

#[test]
fn export_and_import_need_the_admin_role_and_an_open_collection() {
    let dir = TempDir::new("ndjson_grants");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"create user "app" identified by "pw" grant write on default"#);
    ok(&mut admin, "config set auth_required true");
    let mut s = Session::new(inst.clone(), false, None);
    ok(&mut s, "auth app pw");
    let file = dir.path().join("x.ndjson");
    assert!(fails(&mut s, &format!("export to \"{}\"", file.display())).contains("admin role"));
    assert!(fails(&mut s, &format!("import from \"{}\"", file.display())).contains("admin role"));
    assert!(!file.exists());
}

#[test]
fn the_command_line_exports_and_imports_a_collection() {
    let dir = TempDir::new("ndjson_cli");
    let home = dir.path().join("home");
    {
        let inst = kivadb::instance::Instance::open(&home, Default::default()).unwrap();
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "project create baak collections (places, copy)");
        ok(&mut s, "use baak.places");
        ok(&mut s, r#"set city:1 {"name": "Goma", "pop": decimal "670000"}"#);
        ok(&mut s, r#"set city:2 {"name": "Bukavu"}"#);
        inst.shutdown();
    }
    let file = dir.path().join("places.ndjson");
    let o = kivadb(&home, &["export", "baak.places", "-o", file.to_str().unwrap(), "--keys", "city:*"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("Exported 2 key(s) of 'baak.places'"));
    let o = kivadb(&home, &["import", "baak.copy", file.to_str().unwrap(), "--dry-run"]);
    let said = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(o.status.success() && said.contains("Dry run: 2 line(s) checked"), "{said}");
    let o = kivadb(&home, &["import", "baak.copy", file.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("2 key(s) written"));
    let o = kivadb(&home, &["-w", "baak.copy", "-e", "get city:1"]);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains(r#""pop":decimal "670000""#)
            || String::from_utf8_lossy(&o.stdout).contains("670000")
    );
    let o = kivadb(&home, &["import", "nowhere", file.to_str().unwrap()]);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("does not exist"));
}
