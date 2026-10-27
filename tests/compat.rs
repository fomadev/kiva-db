// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Homes written by KivaDB 1.7.1 (tests/fixtures, made by tests/fixtures/v1.7.1.kva),
//! opened by this version: every value, index, expiry, vector, workspace and
//! account must still be there, and the fixtures themselves are never changed.

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

fn the_values_of_1_7_1_are_all_there(s: &mut Session) {
    assert_eq!(ok(s, "get greeting"), "greeting: hello [string]");
    assert_eq!(ok(s, "get n:count"), "n:count: 41 [number]");
    assert_eq!(ok(s, "get flag and nothing"), "flag: true [boolean]\nnothing: null [null]");
    assert_eq!(ok(s, "get list"), "list: [1,2,3] [array]");
    assert_eq!(ok(s, "get blob"), "blob: 0x00010203 [bytes]");
    assert!(ok(s, "get long").starts_with("long: kivadb kivadb"), "a value compressed by 1.7.1");
    assert_eq!(ok(s, "get after:compress"), "after:compress: yes [string]");
    assert_eq!(ok(s, "get user:2.name"), "user:2.name: Ben [string]");
}

#[test]
fn a_home_written_by_1_7_1_opens_with_everything_in_it() {
    let original = snapshot(&fixture("v1.7.1"));
    let dir = TempDir::new("compat_1_7_1");
    let home = home_from("v1.7.1", &dir);
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        the_values_of_1_7_1_are_all_there(&mut s);

        // Its indexes answer, and are used.
        assert_eq!(ok(&mut s, "count where user:*.role = \"dev\""), "Count: 2");
        assert!(ok(&mut s, "explain get where user:*.role = \"dev\"").contains("INDEX SCAN"));
        assert!(ok(&mut s, "explain get where user:*.city = \"Goma\"").contains("INDEX SCAN"));
        assert_eq!(ok(&mut s, "count where user:*.city = \"Goma\""), "Count: 2");
        // The unique index still refuses a second "ana", whatever the case.
        assert!(fails(&mut s, r#"set user:9 {"name": "ANA", "role": "x"}"#).contains("unique index"));

        // The expiry is kept: years away, still counted.
        let scan = ok(&mut s, "scan session:*");
        let left: u64 = scan.split("TTL: ").nth(1).and_then(|r| r.split("s").next()).unwrap().parse().unwrap();
        assert!(left > 300_000_000, "{scan}");

        // The HNSW graph saved by 1.7.1 is read back.
        let similar = ok(&mut s, "get similar doc:*.embedding to [1, 0.1, 0] limit 1");
        assert!(similar.contains("doc:1"), "{similar}");

        // Other workspaces, and integrity.
        ok(&mut s, "workspace switch shop");
        assert_eq!(ok(&mut s, "get item:1.label"), "item:1.label: pen [string]");
        ok(&mut s, "workspace switch default");
        assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));

        // Accounts of 1.x keep their personal workspace (migrated to a grant).
        let mut ana = Session::new(inst.clone(), false, None);
        ok(&mut ana, "auth ana ana-pw");
        assert_eq!(ok(&mut ana, "whoami"), "ana (editor) @ workspace 'u_ana'");
        ok(&mut ana, "set mine 1");
        let mut guest = Session::new(inst.clone(), false, None);
        ok(&mut guest, "auth guest guest-pw");
        assert!(ok(&mut guest, "whoami").starts_with("guest (viewer)"));

        // New values of 2.0 sit next to the old ones, and compaction keeps both.
        ok(&mut s, "set date born '2004-05-09'");
        ok(&mut s, "compact");
        drop((s, ana, guest));
        inst.shutdown();
    }
    let inst = open(&home);
    let mut s = Session::new(inst, true, None);
    the_values_of_1_7_1_are_all_there(&mut s);
    assert_eq!(ok(&mut s, "get born"), "born: 2004-05-09 [date]");
    assert!(ok(&mut s, "explain get where user:*.role = \"dev\"").contains("INDEX SCAN"));
    assert_eq!(snapshot(&fixture("v1.7.1")), original, "the fixture must not change");
}

#[test]
fn an_encrypted_home_of_1_7_1_opens_with_its_key() {
    let dir = TempDir::new("compat_1_7_1_encrypted");
    let home = home_from("v1.7.1-encrypted", &dir);
    // The test key the fixture was made with (KIVADB_ENCRYPTION_KEY = 63 zeros then 1).
    let key = dir.path().join("test.key");
    std::fs::write(&key, format!("{}1", "0".repeat(63))).unwrap();
    {
        let mut cfg = kivadb::config::Config::load(&home).unwrap();
        cfg.set("encryption", "key_file", &key.display().to_string()).unwrap();
    }
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        assert_eq!(ok(&mut s, "get secret.pin"), "secret.pin: 4321 [string]");
        assert!(ok(&mut s, "explain get where secret*.pin = \"4321\"").contains("INDEX SCAN"));
        assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));
        drop(s);
        inst.shutdown();
    }

    // Without the key, nothing opens.
    let mut cfg = kivadb::config::Config::load(&home).unwrap();
    cfg.set("encryption", "key_file", "").unwrap();
    drop(cfg);
    let err = Instance::open(&home, Options::default()).err().expect("no key, no opening").to_string();
    assert!(err.contains("encrypted"), "{err}");
}

#[test]
fn the_doctor_reads_a_home_of_1_7_1_without_changing_it() {
    let dir = TempDir::new("compat_doctor");
    let home = home_from("v1.7.1", &dir);
    let before = snapshot(&home);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .args(["doctor", "--json", "--home"])
        .arg(&home)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{report:#}");
    assert_eq!(report["summary"]["problems"], 0, "{report:#}");
    let data = report["findings"].as_array().unwrap().iter().find(|f| f["topic"] == "Data").unwrap();
    assert!(data["message"].as_str().unwrap().contains("every checksum matches"), "{data}");
    assert_eq!(snapshot(&home), before, "the doctor must not write");

    // A damaged record is a problem, with the way out.
    let store = home.join("data").join("store.kiva");
    let mut bytes = std::fs::read(&store).unwrap();
    bytes[40] ^= 0xff;
    std::fs::write(&store, bytes).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .args(["doctor", "--home"])
        .arg(&home)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FAIL  Data        workspace 'default': checksum mismatch"), "{text}");
    assert!(text.contains("check repair"), "{text}");
}

fn format_of(file: &Path) -> u32 {
    let h = std::fs::read(file).unwrap();
    u32::from_le_bytes([h[4], h[5], h[6], h[7]])
}

#[test]
fn a_file_holding_the_types_of_2_0_is_refused_by_1_x_instead_of_cut() {
    let dir = TempDir::new("compat_format");
    let home = home_from("v1.7.1", &dir);
    let store = home.join("data").join("store.kiva");
    let shop = home.join("workspaces").join("shop").join("store.kiva");
    assert_eq!(format_of(&store), 1);
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        // Old types only: the file stays readable by 1.x.
        ok(&mut s, "set more 1");
        assert_eq!(format_of(&store), 1);
        // The first date raises it, before the record is written.
        ok(&mut s, "set date born '2004-05-09'");
        assert_eq!(format_of(&store), 2);
        assert_eq!(format_of(&shop), 1, "only the workspace that holds a date");
        // A compaction keeps it.
        ok(&mut s, "compact");
        assert_eq!(format_of(&store), 2);
        ok(&mut s, "set after 2");
        drop(s);
        inst.shutdown();
    }
    let inst = open(&home);
    let mut s = Session::new(inst, true, None);
    assert_eq!(ok(&mut s, "get born and after"), "born: 2004-05-09 [date]\nafter: 2 [number]");
    the_values_of_1_7_1_are_all_there(&mut s);
}

#[test]
fn a_format_from_a_newer_kivadb_is_refused() {
    let dir = TempDir::new("compat_newer");
    let home = home_from("v1.7.1", &dir);
    let store = home.join("data").join("store.kiva");
    let mut bytes = std::fs::read(&store).unwrap();
    bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
    std::fs::write(&store, &bytes).unwrap();
    let err = Instance::open(&home, Options::default()).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("unsupported storage format 3"), "{err}");
    assert_eq!(std::fs::read(&store).unwrap(), bytes, "a refused file is left as it is");
}
