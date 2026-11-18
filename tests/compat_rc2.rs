// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! A home written by the published KivaDB 2.0.0-rc.2 (tests/fixtures,
//! made by tests/fixtures/v2.0.0-rc.2.kva), opened by this version: dates,
//! projects and grants, schemas, indexes, vectors and the change journal
//! must all be there, and the fixture itself is never changed
//! (NATIVE_DOCUMENTS.md 27.2). The path 1.7.1 -> rc.2 -> this version runs
//! against the real executables when the CI gives them.

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

/// What rc.2 wrote in its fixture, read back.
fn the_values_of_rc_2_are_all_there(s: &mut Session) {
    assert_eq!(ok(s, "get greeting"), "greeting: hello [string]");
    assert_eq!(ok(s, "get flag"), "flag: true [boolean]");
    assert_eq!(ok(s, "get born"), "born: 2004-05-09 [date]");
    assert!(ok(s, "typeof created_at").ends_with("is a [datetime]"));
    assert_eq!(ok(s, "get opens"), "opens: 08:00:00 [time]");
    assert_eq!(ok(s, "get n:count"), "n:count: (nil)", "deleted by rc.2");
    assert_eq!(ok(s, "get user:2.name"), "user:2.name: Ben [string]");
    assert_eq!(ok(s, "get order:1.total"), "order:1.total: 1250.50 [string]");
}

/// The accounts of rc.2: a grant on a project with its default workspace,
/// and a read grant.
fn the_accounts_of_rc_2_are_all_there(inst: &Arc<Instance>) {
    let mut app = Session::new(inst.clone(), false, None);
    ok(&mut app, "auth app app-pw");
    assert!(ok(&mut app, "whoami").contains("travel.places"), "its default workspace");
    assert!(ok(&mut app, "get place:1.name").contains("Virunga"));
    ok(&mut app, "workspace switch travel.cache");
    ok(&mut app, "incr hits:app");
    let mut guest = Session::new(inst.clone(), false, None);
    ok(&mut guest, "auth guest guest-pw");
    ok(&mut guest, "workspace switch travel.places");
    assert!(ok(&mut guest, "get place:2.kind").contains("hotel"));
    assert!(fails(&mut guest, "set place:9 {\"name\": \"x\"}").to_lowercase().contains("read"), "a read grant only");
}

#[test]
fn a_home_written_by_2_0_0_rc_2_opens_with_everything_in_it() {
    let original = snapshot(&fixture("v2.0.0-rc.2"));
    let dir = TempDir::new("compat_rc2");
    let home = home_from("v2.0.0-rc.2", &dir);
    let store = home.join("data").join("store.kiva");
    let places = home.join("workspaces").join("travel.places").join("store.kiva");
    assert_eq!(format_of(&store), 2, "rc.2 wrote dates: format 2");
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        the_values_of_rc_2_are_all_there(&mut s);

        // Its indexes answer, and are used.
        assert_eq!(ok(&mut s, "count where user:*.role = \"dev\""), "Count: 2");
        assert!(ok(&mut s, "explain get where user:*.role = \"dev\"").contains("INDEX SCAN"));
        assert!(ok(&mut s, "explain get where user:*.city = \"Goma\"").contains("INDEX SCAN"));
        assert!(fails(&mut s, r#"set user:9 {"name": "ANA", "role": "x"}"#).contains("unique index"));
        // The expiry, years away.
        let scan = ok(&mut s, "scan session:*");
        let left: u64 = scan.split("TTL: ").nth(1).and_then(|r| r.split('s').next()).unwrap().parse().unwrap();
        assert!(left > 300_000_000, "{scan}");
        // The HNSW graph saved by rc.2.
        assert!(ok(&mut s, "get similar doc:*.embedding to [1, 0.1, 0] limit 1").contains("doc:1"));

        // The change journal of rc.2, read from the place of its consumer:
        // every change is there, and goes on with the writes of this version.
        let read = ok(&mut s, "changes read as search limit 100");
        assert!(read.contains("#4 ") && read.contains("insert born (date) \"2004-05-09\""), "{read}");
        assert!(read.contains("insert created_at (datetime) \"2026-10-27T14:30:00.000Z\""), "{read}");
        assert!(read.contains("#13 ") && read.contains("delete n:count"), "{read}");
        assert!(read.contains("#16 ") && !read.contains("#17 "), "{read}");
        ok(&mut s, "set after:rc2 1");
        let next = ok(&mut s, "changes read as search limit 100");
        assert!(next.contains("#17 ") && next.contains("insert after:rc2"), "{next}");

        // The schema of the project's workspace still checks and fills in.
        ok(&mut s, "workspace switch travel.places");
        assert!(fails(&mut s, r#"set place:3 {"opens": "09:00"}"#).contains("name: required"));
        ok(&mut s, r#"set place:3 {"name": "Nyiragongo", "opens": time "06:00"}"#);
        assert_eq!(ok(&mut s, "get place:3.kind"), "place:3.kind: park [string]", "its default");
        assert_eq!(format_of(&places), 3, "a typed document raises its own workspace");
        ok(&mut s, "workspace switch default");
        the_accounts_of_rc_2_are_all_there(&inst);

        // New values of this version next to those of rc.2.
        ok(&mut s, "set decimal price \"19.99\"");
        assert_eq!(
            ok(&mut s, "convert order:*.total to decimal"),
            "Converted: 2 document(s) of order:* to decimal (0 already decimal, 0 without total)."
        );
        assert_eq!(format_of(&store), 3);
        assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));
        ok(&mut s, "compact");
        drop(s);
        inst.shutdown();
    }
    let inst = open(&home);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_rc_2_are_all_there_after_convert(&mut s);
    assert_eq!(ok(&mut s, "get price"), "price: 19.99 [decimal]");
    assert!(ok(&mut s, "explain get where user:*.role = \"dev\"").contains("INDEX SCAN"));
    the_accounts_of_rc_2_are_all_there(&inst);
    drop(s);
    inst.shutdown();

    let (code, text) = doctor(&home);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("workspace 'default': storage format 3"), "{text}");
    assert!(text.contains("workspace 'travel.cache': storage format 1"), "{text}");
    assert_eq!(snapshot(&fixture("v2.0.0-rc.2")), original, "the fixture must not change");
}

/// The values of rc.2 once its order totals are decimals.
fn the_values_of_rc_2_are_all_there_after_convert(s: &mut Session) {
    assert_eq!(ok(s, "get greeting"), "greeting: hello [string]");
    assert_eq!(ok(s, "get born"), "born: 2004-05-09 [date]");
    assert_eq!(ok(s, "get opens"), "opens: 08:00:00 [time]");
    assert_eq!(ok(s, "get user:2.name"), "user:2.name: Ben [string]");
    assert_eq!(ok(s, "get order:1.total"), "order:1.total: 1250.50 [decimal]");
    assert_eq!(ok(s, "sum order:*.total"), "Sum: 1330.75 (2 matches)");
}

#[test]
fn the_doctor_reads_a_home_of_rc_2_without_changing_it() {
    let dir = TempDir::new("compat_rc2_doctor");
    let home = home_from("v2.0.0-rc.2", &dir);
    let before = snapshot(&home);
    let (code, text) = doctor(&home);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("every checksum matches"), "{text}");
    assert!(text.contains("workspace 'default': storage format 2, dates and times"), "{text}");
    assert!(text.contains("workspace 'travel.places': storage format 1"), "{text}");
    assert_eq!(snapshot(&home), before, "the doctor must not write");
}

// ---------------------------------------------------------------------------
// The path of an upgrade, with the real executables (CI, Linux)
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
fn a_home_goes_from_1_7_1_through_rc_2_to_this_version() {
    let (Some(v1), Some(rc2)) = (old_bin("KIVADB_V1_BIN"), old_bin("KIVADB_RC2_BIN")) else {
        return;
    };
    let dir = TempDir::new("compat_path");
    let home = dir.path().join("home");
    let store = home.join("data").join("store.kiva");

    // Written by 1.7.1.
    run(
        &v1,
        &home,
        &[
            "set greeting \"hello\"",
            r#"set user:1 {"name": "Ana", "role": "dev", "total": "10.10"}"#,
            r#"set user:2 {"name": "Ben", "role": "ops", "total": "2.05"}"#,
            "create index on user: (role)",
            "create user \"ana\" identified by \"ana-pw\"",
        ],
    );
    assert_eq!(format_of(&store), 1);

    // Opened and written by rc.2: its accounts migrated, a date raises format 2.
    let text =
        run(&rc2, &home, &["get user:1.name", "set date born \"2004-05-09\"", "count where user:*.role = \"dev\""]);
    assert!(text.contains("Ana") && text.contains("Count: 1"), "{text}");
    assert_eq!(format_of(&store), 2);

    // Opened by this version: everything is there, and typed values follow.
    let text = run(
        env!("CARGO_BIN_EXE_kivadb").as_ref(),
        &home,
        &[
            "get greeting",
            "get born",
            "count where user:*.role = \"dev\"",
            "convert user:*.total to decimal",
            "sum user:*.total",
        ],
    );
    assert!(text.contains("greeting: hello [string]"), "{text}");
    assert!(text.contains("born: 2004-05-09 [date]"), "{text}");
    assert!(text.contains("Count: 1"), "{text}");
    assert!(text.contains("Sum: 12.15 (2 matches)"), "{text}");
    assert_eq!(format_of(&store), 3);
    let inst = open(&home);
    let mut ana = Session::new(inst.clone(), false, None);
    ok(&mut ana, "auth ana ana-pw");
    assert!(ok(&mut ana, "whoami").starts_with("ana (editor)"), "the account of 1.7.1, migrated by rc.2");
    drop(ana);
    inst.shutdown();
    let (code, text) = doctor(&home);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("workspace 'default': storage format 3"), "{text}");

    // And rc.2 now refuses it, leaving it as it is.
    let out = std::process::Command::new(&rc2)
        .arg("--home")
        .arg(&home)
        .arg("--no-listen")
        .args(["-e", "get greeting"])
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(format_of(&store), 3);
}
