// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! A home written by the published KivaDB 2.0.0-rc.3 (tests/fixtures/v2.0.0-rc.3,
//! made by tests/fixtures/v2.0.0-rc.3.kva), opened by this version: the typed
//! documents of storage format 3, the exact decimal key, int and float keys,
//! their indexes and schema, the change journal and the accounts. And,
//! with the real rc.3 executable (`KIVADB_RC3_BIN`), the other way: rc.3
//! still reads what this version writes, random values included, since the
//! storage formats did not change after it.

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

/// What rc.3 wrote in its fixture, read back.
fn the_values_of_rc_3_are_all_there(s: &mut Session) {
    assert_eq!(ok(s, "get greeting"), "greeting: hello again [string]");
    assert_eq!(ok(s, "get n:count"), "n:count: 41 [int]");
    assert_eq!(ok(s, "get ratio"), "ratio: 0.5 [float]");
    assert_eq!(ok(s, "get price"), "price: 12.50 [decimal]");
    assert_eq!(ok(s, "get born"), "born: 2004-05-09 [date]");
    assert!(ok(s, "typeof created_at").ends_with("is a [datetime]"));
    assert_eq!(ok(s, "get acc:2.balance"), "acc:2.balance: 100.00 [decimal]", "json incr of rc.3");
    assert!(ok(s, "typeof acc:1").contains("[object]"));
    assert_eq!(ok(s, "get acc:1.limits"), r#"acc:1.limits: [decimal "500", decimal "2500.00"] [array]"#);
    assert_eq!(ok(s, "sum acc:*.balance"), "Sum: 1350.60 (3 matches)", "exact, as decimals");
    assert_eq!(ok(s, "get item:2.qty"), "item:2.qty: 3 [int]");
}

#[test]
fn a_home_written_by_2_0_0_rc_3_opens_with_everything_in_it() {
    let original = snapshot(&fixture("v2.0.0-rc.3"));
    let dir = TempDir::new("compat_rc3");
    let home = home_from("v2.0.0-rc.3", &dir);
    let store = home.join("data").join("store.kiva");
    assert_eq!(format_of(&store), 3, "rc.3 wrote typed documents: format 3");
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        the_values_of_rc_3_are_all_there(&mut s);

        // Its indexes answer, exactly, and are used.
        assert_eq!(ok(&mut s, r#"count where acc:*.balance > decimal "50""#), "Count: 2");
        assert!(ok(&mut s, r#"explain get where acc:*.balance > decimal "50""#).contains("INDEX SCAN"));
        assert!(fails(&mut s, r#"set acc:9 {"name": "ANA", "balance": decimal "1"}"#).contains("unique index"));
        // Its schema still checks and fills in.
        assert!(fails(&mut s, r#"set item:3 {"qty": int 2}"#).contains("price: required"));
        ok(&mut s, r#"set item:3 {"price": decimal "1.50"}"#);
        assert_eq!(ok(&mut s, "get item:3.qty"), "item:3.qty: 1 [int]", "its default");
        // The expiry, years away.
        let scan = ok(&mut s, "scan session:*");
        let left: u64 = scan.split("TTL: ").nth(1).and_then(|r| r.split('s').next()).unwrap().parse().unwrap();
        assert!(left > 300_000_000, "{scan}");

        // The change journal of rc.3 goes on with the writes of this version.
        let read = ok(&mut s, "changes read from beginning limit 100");
        assert!(read.contains("#18 ") && read.contains("insert item:2") && !read.contains("#20 "), "{read}");
        ok(&mut s, "set after:rc3 uuid_v7()");
        let next = ok(&mut s, "changes read from beginning limit 100");
        assert!(next.contains("insert after:rc3"), "{next}");

        // The random functions of this version, next to the values of rc.3.
        ok(
            &mut s,
            r#"set draw:1 {"id": uuid(), "order": shuffle([1, 2, 3]), "pick": sample([decimal "1.5", decimal "2.5"])}"#,
        );
        let drawn = ok(&mut s, "get where acc:* order by random() limit 2");
        assert!(drawn.contains("(2 matches)"), "{drawn}");
        let two = ok(&mut s, r#"get where acc:*.balance > decimal "50" order by random() limit 10"#);
        assert!(two.contains("acc:1") && two.contains("acc:2") && !two.contains("acc:3"), "{two}");

        // The project's workspace and the account of rc.3.
        let mut app = Session::new(inst.clone(), false, None);
        ok(&mut app, "auth app app-pw");
        assert!(ok(&mut app, "whoami").contains("travel.places"), "its default workspace");
        assert!(ok(&mut app, "get place:1.fee").contains(r#"50.00 [decimal]"#));
        drop(app);

        assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));
        ok(&mut s, "compact");
        drop(s);
        inst.shutdown();
    }
    let inst = open(&home);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_rc_3_are_all_there(&mut s);
    assert!(ok(&mut s, "get draw:1.pick").contains("[decimal]"));
    drop(s);
    inst.shutdown();
    assert_eq!(format_of(&store), 3, "the format stays");

    let (code, text) = doctor(&home);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("workspace 'default': storage format 3"), "{text}");
    assert_eq!(snapshot(&fixture("v2.0.0-rc.3")), original, "the fixture must not change");
}

#[test]
fn the_doctor_reads_a_home_of_rc_3_without_changing_it() {
    let dir = TempDir::new("compat_rc3_doctor");
    let home = home_from("v2.0.0-rc.3", &dir);
    let before = snapshot(&home);
    let (code, text) = doctor(&home);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("every checksum matches"), "{text}");
    assert!(text.contains("workspace 'default': storage format 3"), "{text}");
    assert_eq!(snapshot(&home), before, "the doctor must not write");
}

// ---------------------------------------------------------------------------
// Back to rc.3, with its real executable (CI, Linux)
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
fn rc_3_reads_what_this_version_writes() {
    let Some(rc3) = old_bin("KIVADB_RC3_BIN") else {
        return;
    };
    let dir = TempDir::new("compat_rc3_back");
    let home = home_from("v2.0.0-rc.3", &dir);
    let this: &Path = env!("CARGO_BIN_EXE_kivadb").as_ref();
    let text = run(
        this,
        &home,
        &[
            "set id uuid_v7()",
            r#"set q:1 {"id": uuid(), "score": random_int(7, 7), "order": shuffle(["a"]), "w": random()}"#,
            "json set acc:3 .code random_string(8)",
            "get id",
        ],
    );
    let id = text.lines().find_map(|l| l.strip_prefix("id: ")).unwrap().split(' ').next().unwrap().to_string();
    assert_eq!(id.len(), 36, "{text}");

    // rc.3 opens the home, finds the values drawn, and its own values.
    let text = run(&rc3, &home, &["get id", "get q:1.score", "get q:1.order", "get acc:3.code", "sum acc:*.balance"]);
    assert!(text.contains(&format!("id: {id} [string]")), "{text}");
    assert!(text.contains("q:1.score: 7 [int]"), "{text}");
    assert!(text.contains(r#"q:1.order: ["a"] [array]"#), "{text}");
    assert!(text.contains("acc:3.code: ") && text.contains("Sum: 1350.60 (3 matches)"), "{text}");
}
