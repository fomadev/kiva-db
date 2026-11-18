// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot C7 of docs/architecture/NATIVE_DOCUMENTS.md (section 25):
//! `convert <pattern>.<field> to date|datetime|time|decimal [check]`, all or
//! nothing, and `kivadb doctor` naming the storage format of each workspace.

mod common;

use common::{fails, ok, TempDir};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use std::sync::Arc;

fn open(dir: &TempDir) -> Arc<Instance> {
    Instance::open(dir.path(), Options::default()).unwrap()
}

fn format_of(dir: &TempDir) -> u32 {
    let h = std::fs::read(dir.path().join("data").join("store.kiva")).unwrap();
    u32::from_le_bytes([h[4], h[5], h[6], h[7]])
}

fn orders(s: &mut Session) {
    ok(s, r#"set order:1 {"at": "2026-10-31", "total": "1250.50", "profile": {"born": "2004-05-09"}}"#);
    ok(s, r#"set order:2 {"at": "2026-11-02", "total": "0.10", "profile": {"born": "1999-12-31"}}"#);
    ok(s, r#"set order:3 {"at": date "2026-11-05", "total": decimal "7"}"#);
    ok(s, r#"set order:4 {"note": "no date"}"#);
    ok(s, r#"set order:5 {"at": null}"#);
}

#[test]
fn texts_become_native_values_all_or_nothing() {
    let dir = TempDir::new("conv_date");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, r#"set order:1 {"at": "2026-10-31", "total": "1250.50", "profile": {"born": "2004-05-09"}}"#);
    ok(&mut s, r#"set order:2 {"at": "2026-11-02", "total": "0.10", "profile": {"born": "1999-12-31"}}"#);
    ok(&mut s, r#"set order:4 {"note": "no date"}"#);
    ok(&mut s, r#"set order:5 {"at": null}"#);
    ok(&mut s, "create index on order:*.at");
    assert!(format_of(&dir) < 3, "no native value yet");

    // The preview writes nothing.
    assert_eq!(
        ok(&mut s, "convert order:*.at to date check"),
        "Check: 2 document(s) of order:* to convert to date (0 already date, 2 without at); nothing written."
    );
    assert_eq!(ok(&mut s, "json type order:1 .at"), "order:1.at: [string]");
    assert!(format_of(&dir) < 3, "check writes nothing");

    assert_eq!(
        ok(&mut s, "convert order:*.at to date"),
        "Converted: 2 document(s) of order:* to date (0 already date, 2 without at)."
    );
    assert_eq!(ok(&mut s, "json type order:1 .at"), "order:1.at: [date]");
    assert_eq!(ok(&mut s, "json type order:1 .total"), "order:1.total: [string]", "other fields are left");
    assert_eq!(format_of(&dir), 3);
    // Queries answer as before, with the index.
    assert_eq!(ok(&mut s, r#"count where order:*.at >= "2026-11-01""#), "Count: 1");
    assert_eq!(ok(&mut s, r#"count where order:*.at = date "2026-10-31""#), "Count: 1");
    // Again: nothing left to convert.
    assert_eq!(
        ok(&mut s, "convert order:*.at to date"),
        "Converted: 0 document(s) of order:* to date (2 already date, 2 without at)."
    );

    // A nested field, and decimals.
    ok(&mut s, "convert order:*.profile.born to date");
    assert_eq!(ok(&mut s, "json type order:2 .profile.born"), "order:2.profile.born: [date]");
    ok(&mut s, "convert order:*.total to decimal");
    assert_eq!(ok(&mut s, "json get order:1 .total"), "order:1.total: 1250.50 [decimal]");
    assert_eq!(ok(&mut s, r#"count where order:*.total = decimal "0.1""#), "Count: 1");
}

#[test]
fn one_value_that_does_not_read_stops_the_whole_conversion() {
    let dir = TempDir::new("conv_refuse");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    orders(&mut s);
    ok(&mut s, r#"set order:9 {"at": "31/10/2026", "total": 12}"#);

    let e = fails(&mut s, "convert order:*.at to date");
    assert!(e.contains("Nothing converted: 1 document(s) of order:* hold a at that does not read as a date"), "{e}");
    assert!(e.contains(r#"order:9 ("31/10/2026""#), "{e}");
    assert_eq!(ok(&mut s, "json type order:1 .at"), "order:1.at: [string]", "nothing was written");
    let e = fails(&mut s, "convert order:*.at to date check");
    assert!(e.contains("Check: 1 document(s)"), "{e}");

    // A number is not a text: it is named, not guessed.
    let e = fails(&mut s, "convert order:*.total to decimal");
    assert!(e.contains("order:9 (12: a int, not a text)"), "{e}");

    // A datetime names its offset.
    ok(&mut s, r#"set ev:1 {"at": "2026-10-31T15:18:00"}"#);
    ok(&mut s, r#"set ev:2 {"at": "2026-10-31T15:18:00Z"}"#);
    let e = fails(&mut s, "convert ev:*.at to datetime");
    assert!(e.contains("ev:1") && e.contains("has no offset"), "{e}");
    ok(&mut s, r#"update ev:1 {"at": "2026-10-31T16:18:00+01:00"}"#);
    assert_eq!(
        ok(&mut s, "convert ev:*.at to datetime"),
        "Converted: 2 document(s) of ev:* to datetime (0 already datetime, 0 without at)."
    );
    assert_eq!(ok(&mut s, "count where ev:*.at = datetime \"2026-10-31T15:18:00Z\""), "Count: 2");

    ok(&mut s, r#"set alarm:1 {"t": "08:30"}"#);
    ok(&mut s, "convert alarm:*.t to time");
    assert_eq!(ok(&mut s, "json type alarm:1 .t"), "alarm:1.t: [time]");

    assert!(fails(&mut s, "convert order:*.at to int").contains("Usage: convert"));
    assert!(fails(&mut s, "convert order:*.at to date now").contains("Usage: convert"));
    assert!(fails(&mut s, "convert order:* to date").contains("<pattern>.<field>"));
}

#[test]
fn a_conversion_inside_a_transaction_is_taken_back_with_it() {
    let dir = TempDir::new("conv_txn");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    orders(&mut s);
    ok(&mut s, "begin");
    ok(&mut s, "convert order:*.at to date");
    ok(&mut s, "rollback");
    assert_eq!(ok(&mut s, "json type order:1 .at"), "order:1.at: [string]");
}

#[test]
fn the_doctor_names_the_storage_format_of_each_workspace() {
    let dir = TempDir::new("conv_doctor");
    {
        let inst = open(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "set plain 1");
        ok(&mut s, "workspace create dated");
        ok(&mut s, "workspace switch dated");
        ok(&mut s, r#"set date day "2026-10-31""#);
        ok(&mut s, "workspace create native");
        ok(&mut s, "workspace switch native");
        ok(&mut s, r#"set doc {"price": decimal "9.90"}"#);
        drop(s);
        inst.shutdown();
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .args(["doctor", "--home"])
        .arg(dir.path())
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("workspace 'default': storage format 1 (opened by KivaDB 1.x and 2.x)"), "{text}");
    assert!(text.contains("workspace 'dated': storage format 2, dates and times"), "{text}");
    assert!(
        text.contains("workspace 'native': storage format 3, native values (opened by KivaDB 2.0.0 and later"),
        "{text}"
    );
}

/// What `convert ... check` says: (texts to convert, already converted).
fn converted_state(s: &mut Session) -> (usize, usize) {
    let text = ok(s, "convert order:*.created_at to datetime check");
    let n = |after: &str| -> usize {
        let rest = text.split(after).nth(1).unwrap_or_else(|| panic!("{text}"));
        rest.trim_start().split(' ').next().unwrap().parse().unwrap()
    };
    (n("Check:"), n("to datetime ("))
}

#[test]
fn a_crash_in_the_middle_of_a_conversion_converts_everything_or_nothing() {
    const N: usize = 200;
    let dir = TempDir::new("conv_crash");
    let store = dir.path().join("data").join("store.kiva");
    let hint = dir.path().join("data").join("store.kiva.hint");
    {
        let inst = open(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "set greeting \"hello\"");
        for i in 0..N {
            ok(&mut s, &format!(r#"set order:{i} {{"created_at": "2026-10-{:02}T09:00:00Z", "n": {i}}}"#, i % 28 + 1));
        }
        drop(s);
        inst.shutdown();
    }
    let before = std::fs::read(&store).unwrap();
    assert_eq!(format_of(&dir), 1);
    {
        let inst = open(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "convert order:*.created_at to datetime");
        drop(s);
        inst.shutdown();
    }
    let after = std::fs::read(&store).unwrap();
    assert_eq!(format_of(&dir), 3);
    // The conversion only appended, behind a header raised to 3.
    assert_eq!(&after[8..before.len()], &before[8..], "appended, not rewritten");
    let written = after.len() - before.len();

    // The file cut at 64 places of the transaction, and at its last byte.
    let mut cuts: Vec<usize> = (0..64).map(|k| before.len() + written * k / 64).collect();
    cuts.push(after.len() - 1);
    for cut in cuts {
        std::fs::write(&store, &after[..cut]).unwrap();
        let _ = std::fs::remove_file(&hint);
        let inst = open(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        assert_eq!(converted_state(&mut s), (N, 0), "cut at {cut} of {}: nothing converted", after.len());
        assert_eq!(ok(&mut s, "get greeting"), "greeting: hello [string]");
        assert_eq!(ok(&mut s, "count order:*"), format!("Count: {N}"));
        assert_eq!(ok(&mut s, "get order:7.created_at").rsplit(' ').next().unwrap(), "[string]");
        assert!(ok(&mut s, "check").contains("CORRUPTED: 0"), "cut at {cut}");
        // A header raised before the crash stays raised, with no typed record.
        assert_eq!(format_of(&dir), 3);
        drop(s);
        inst.shutdown();
    }

    // The whole file: everything converted.
    std::fs::write(&store, &after).unwrap();
    let _ = std::fs::remove_file(&hint);
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    assert_eq!(converted_state(&mut s), (0, N));
    assert_eq!(ok(&mut s, "get order:7.created_at").rsplit(' ').next().unwrap(), "[datetime]");
}
