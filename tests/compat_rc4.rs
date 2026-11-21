// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! A home written by the published KivaDB 2.0.0-rc.4 (tests/fixtures/v2.0.0-rc.4,
//! made by tests/fixtures/v2.0.0-rc.4.kva), opened by this version: its
//! values, random values included, its workspaces, now collections, left
//! where rc.4 put them, the expiry it did not see, and its accounts. And,
//! with the real rc.4 executable (`KIVADB_RC4_BIN`), the other way: rc.4
//! still reads what this version writes, a collection in the new layout
//! and an expire event (a delete for rc.4) included.

mod common;

use common::{fails, ok, TempDir};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
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

/// A copy of a fixture home, its paths pointing at the copy.
fn home_from(name: &str, dir: &TempDir) -> PathBuf {
    let home = dir.path().join("home");
    copy_dir(&fixture(name), &home);
    let conf = home.join("kiva.conf");
    let text = std::fs::read_to_string(&conf).unwrap();
    std::fs::write(&conf, text.replace("@HOME@", &home.display().to_string().replace('\\', "/"))).unwrap();
    home
}

fn open(home: &Path) -> Arc<Instance> {
    Instance::open(home, Options::default()).unwrap()
}

/// Every file of a directory with its content, to prove nothing was written.
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

fn doctor(home: &Path) -> (Option<i32>, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .args(["doctor", "--home"])
        .arg(home)
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What rc.4 wrote in its fixture, read back.
fn the_values_of_rc_4_are_all_there(s: &mut Session) {
    assert_eq!(ok(s, "get greeting"), "greeting: hello again [string]");
    assert_eq!(ok(s, "get n:count"), "n:count: 41 [int]");
    assert_eq!(ok(s, "get price"), "price: 12.50 [decimal]");
    assert_eq!(ok(s, "get born"), "born: 2004-05-09 [date]");
    assert_eq!(ok(s, "get id:fixed"), "id:fixed: 0192f4a3-7b1c-7def-8123-456789abcdef [string]");
    let drawn = ok(s, "get id:drawn");
    let id = drawn.strip_prefix("id:drawn: ").unwrap().split(' ').next().unwrap();
    assert!(id.len() == 36 && id.as_bytes()[14] == b'7', "a UUID v7 drawn by rc.4: {drawn}");
    assert_eq!(ok(s, "get draw:1.score"), "draw:1.score: 7 [int]");
    assert!(ok(s, "get draw:1.pick").ends_with("[decimal]"));
    let order = ok(s, "get draw:1.order");
    for n in ["1", "2", "3"] {
        assert!(order.contains(n), "{order}");
    }
    assert_eq!(ok(s, "sum acc:*.balance"), "Sum: 1330.85 (3 matches)");
    assert_eq!(ok(s, "get item:1.qty"), "item:1.qty: 1 [int]");
    assert_eq!(ok(s, "get temp:gone"), "temp:gone: (nil)", "its expiry passed long ago");
}

#[test]
fn a_home_written_by_2_0_0_rc_4_opens_with_everything_in_it() {
    let original = snapshot(&fixture("v2.0.0-rc.4"));
    let dir = TempDir::new("compat_rc4");
    let home = home_from("v2.0.0-rc.4", &dir);
    let store = home.join("data").join("store.kiva");
    assert_eq!(format_of(&store), 3, "rc.4 wrote typed documents: format 3");
    let places = snapshot(&home.join("workspaces").join("travel.places"));
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        the_values_of_rc_4_are_all_there(&mut s);

        // The expiry rc.4 did not see is in the journal, once, after its
        // 15 changes, with the time it ran out.
        let read = ok(&mut s, "changes read from beginning limit 100");
        assert!(read.contains("#15 ") && read.contains("insert greeting (string) \"hello again\""), "{read}");
        assert!(read.contains("#16 ") && read.contains("expire temp:gone (its TTL ran out at "), "{read}");
        assert_eq!(read.matches("expire ").count(), 1, "{read}");

        // Its index, unique index and schema.
        assert_eq!(ok(&mut s, r#"count where acc:*.balance > decimal "50""#), "Count: 2");
        assert!(ok(&mut s, r#"explain get where acc:*.balance > decimal "50""#).contains("INDEX SCAN"));
        assert!(fails(&mut s, r#"set acc:9 {"name": "ANA", "balance": decimal "1"}"#).contains("unique index"));
        assert!(fails(&mut s, r#"set item:3 {"qty": int 2}"#).contains("price: required"));

        // Its workspaces are the collections of this version, where rc.4 put them.
        let list = ok(&mut s, "collection list");
        for name in ["default", "lab", "travel.cache", "travel.places"] {
            assert!(list.contains(name), "{list}");
        }
        assert!(list.contains("workspaces/travel.places"), "nothing moved: {list}");
        assert_eq!(ok(&mut s, "workspace list"), list, "workspace is the former name");
        ok(&mut s, "use travel.places");
        assert_eq!(ok(&mut s, "get place:1.fee"), "place:1.fee: 50.00 [decimal]");
        ok(&mut s, "use lab");
        assert_eq!(ok(&mut s, "get note:1"), "note:1: outside a project [string]");

        // A new collection of its project goes to the layout of this version.
        ok(&mut s, "collection create travel.photos");
        ok(&mut s, "use travel.photos");
        ok(&mut s, r#"set pic:1 "photo""#);
        assert!(home.join("projects").join("travel").join("photos").join("store.kiva").is_file());
        assert!(!home.join("projects").join("travel").join("places").exists(), "nothing moved");
        ok(&mut s, "use default");

        // The accounts of rc.4: their default collection, their grants.
        let mut app = Session::new(inst.clone(), false, None);
        ok(&mut app, "auth app app-pw");
        let who = ok(&mut app, "whoami");
        assert!(who.contains("travel.places"), "its default collection: {who}");
        ok(&mut app, "use travel.photos");
        ok(&mut app, r#"set pic:2 "granted on travel.*""#);
        let mut guest = Session::new(inst.clone(), false, None);
        ok(&mut guest, "auth guest guest-pw");
        ok(&mut guest, "use travel.places");
        assert!(ok(&mut guest, "get place:1").contains("Virunga"));
        assert!(fails(&mut guest, r#"set place:2 "no""#).contains("ermission"));
        assert!(fails(&mut guest, "use travel.cache").contains("ermission"));
        drop((app, guest));

        assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));
        ok(&mut s, "compact");
        drop(s);
        inst.shutdown();
    }
    let inst = open(&home);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_rc_4_are_all_there(&mut s);
    let read = ok(&mut s, "changes read from beginning limit 100");
    assert_eq!(read.matches("expire ").count(), 1, "not twice after a restart: {read}");
    ok(&mut s, "use travel.photos");
    let pics = ok(&mut s, "scan pic:*");
    assert!(pics.contains("Total: 2 keys."), "{pics}");
    drop(s);
    inst.shutdown();
    assert_eq!(format_of(&store), 3, "the format stays");
    let mut after = snapshot(&home.join("workspaces").join("travel.places"));
    after.retain(|(p, _)| p.extension().is_none_or(|e| e != "lock"));
    assert_eq!(after, places, "rc.4's folders stay where they are, with what they held");

    let (code, text) = doctor(&home);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("'default': storage format 3"), "{text}");
    assert!(text.contains("'travel.photos': storage format 1"), "{text}");
    assert_eq!(snapshot(&fixture("v2.0.0-rc.4")), original, "the fixture must not change");
}

#[test]
fn the_doctor_reads_a_home_of_rc_4_without_changing_it() {
    let dir = TempDir::new("compat_rc4_doctor");
    let home = home_from("v2.0.0-rc.4", &dir);
    let before = snapshot(&home);
    let (code, text) = doctor(&home);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("every checksum matches"), "{text}");
    assert!(text.contains("'default': storage format 3"), "{text}");
    assert!(text.contains("'travel.places': storage format 3"), "{text}");
    assert_eq!(snapshot(&home), before, "the doctor must not write");
}

// ---------------------------------------------------------------------------
// Back to rc.4, with its real executable (CI, Linux)
// ---------------------------------------------------------------------------

fn old_bin(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_file())
}

fn run(bin: &Path, home: &Path, commands: &[&str]) -> String {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("--home").arg(home).arg("--no-listen").env("KIVADB_NO_UPDATE_CHECK", "1");
    for c in commands {
        cmd.arg("-e").arg(c);
    }
    let out = cmd.output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{} {commands:?}: {text}", bin.display());
    text
}

#[test]
fn rc_4_reads_what_this_version_writes() {
    let Some(rc4) = old_bin("KIVADB_RC4_BIN") else {
        return;
    };
    let dir = TempDir::new("compat_rc4_back");
    let home = home_from("v2.0.0-rc.4", &dir);
    let this: &Path = env!("CARGO_BIN_EXE_kivadb").as_ref();
    // This version journals the expiry at open, creates a collection in
    // projects/travel/photos, and writes in it and in a collection of rc.4.
    let text = run(
        this,
        &home,
        &[
            "changes read from beginning limit 100",
            "collection create travel.photos",
            "use travel.photos",
            r#"set pic:1 {"id": uuid_v7(), "fee": decimal "5.00"}"#,
            "use travel.places",
            r#"set place:2 {"name": "Nyiragongo"}"#,
            "use default",
            r#"set after:rc4 "written by rc.5""#,
        ],
    );
    assert!(text.contains("expire temp:gone"), "{text}");
    assert!(home.join("projects").join("travel").join("photos").join("store.kiva").is_file());

    // rc.4 opens the home: the expire event is a delete for it, and it finds
    // the new collection where kiva.conf says it is.
    let text = run(
        &rc4,
        &home,
        &[
            "changes read from beginning limit 100",
            "get after:rc4",
            "get greeting",
            "workspace switch travel.photos",
            "get pic:1.fee",
            "workspace switch travel.places",
            "count place:*",
            "check",
        ],
    );
    assert!(text.contains("#16 ") && text.contains("delete temp:gone"), "{text}");
    assert!(text.contains("insert after:rc4"), "{text}");
    assert!(text.contains(r#"after:rc4: written by rc.5 [string]"#), "{text}");
    assert!(text.contains("greeting: hello again [string]"), "{text}");
    assert!(text.contains("pic:1.fee: 5.00 [decimal]"), "{text}");
    assert!(text.contains("Count: 2") && text.contains("CORRUPTED: 0"), "{text}");

    // And its doctor finds nothing wrong.
    let out = std::process::Command::new(&rc4)
        .args(["doctor", "--home"])
        .arg(&home)
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{report}");
    assert!(report.contains("0 problems") && report.contains("'travel.photos'"), "{report}");
}
