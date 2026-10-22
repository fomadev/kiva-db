// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! `kivadb backup` and `kivadb restore`.

mod common;

use common::{instance, ok, TempDir};
use kivadb::backup::{self, RestoreOptions, Scope};
use kivadb::session::Session;
use std::io::Read;
use std::path::{Path, PathBuf};
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

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// A home with two workspaces, a user, an index and a key with an expiry.
fn fill(home: &Path) {
    let o = kivadb(
        home,
        &[
            "-e",
            r#"set user:1 {"name": "Amani", "city": "Goma"}"#,
            "-e",
            "set session \"abc\" ttl 3600",
            "-e",
            "create index on user: (city)",
            "-e",
            r#"create user "baraka" identified by "pa55-word" role editor"#,
            "-e",
            "workspace create shop",
            "-e",
            "workspace switch shop",
            "-e",
            "set product:1 {\"price\": 12}",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn a_home_is_saved_and_restored_into_a_new_folder() {
    let dir = TempDir::new("backup_home");
    let home = dir.path().join("home");
    fill(&home);
    let archive = dir.path().join("saves").join("all.tar.gz");

    let o = kivadb(&home, &["backup", "--output", s(&archive)]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("Backup written:") && out(&o).contains("workspace(s): default, shop"), "{}", out(&o));
    let sums = std::fs::read_to_string(dir.path().join("saves").join("all.tar.gz.sha256")).unwrap();
    assert!(sums.ends_with("  all.tar.gz\n") && sums.len() == 64 + 2 + 10 + 1, "{sums}");
    // Never over an existing file.
    assert!(err(&kivadb(&home, &["backup", "-o", s(&archive)])).contains("already exists"));

    let o = kivadb(&home, &["restore", s(&archive), "--check"]);
    assert!(o.status.success() && out(&o).contains("The archive is sound"), "{}{}", out(&o), err(&o));

    let target = dir.path().join("restored");
    let o = kivadb(&home, &["restore", s(&archive), "--target", s(&target)]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("Archive SHA-256 matches") && out(&o).contains("Home restored to"), "{}", out(&o));

    // Everything is there, in the new home, which no longer points at the old one.
    let o = kivadb(&target, &["-e", "get user:1.name", "-e", "ttl session", "-e", "indexes", "-e", "users"]);
    let text = out(&o);
    assert!(text.contains("user:1.name: Amani"), "{text}");
    assert!(text.contains("user:* (city)"), "{text}");
    assert!(text.contains("baraka"), "{text}");
    let o = kivadb(&target, &["-w", "shop", "-e", "get product:1.price"]);
    assert!(out(&o).contains("product:1.price: 12"), "{}{}", out(&o), err(&o));
    let conf = std::fs::read_to_string(target.join("kiva.conf")).unwrap();
    assert!(!conf.contains(s(&home)), "the restored kiva.conf still points to the old home");

    // A home is never restored over existing data.
    let o = kivadb(&home, &["restore", s(&archive), "--target", s(&target)]);
    assert!(!o.status.success() && err(&o).contains("is not empty"), "{}", err(&o));
}

#[test]
fn a_workspace_is_restored_under_another_name() {
    let dir = TempDir::new("backup_ws");
    let home = dir.path().join("home");
    fill(&home);
    let archive = dir.path().join("shop.tar.gz");
    let o = kivadb(&home, &["backup", "-o", s(&archive), "-w", "shop"]);
    assert!(o.status.success() && out(&o).contains("workspace(s): shop"), "{}", err(&o));

    let o = kivadb(&home, &["restore", s(&archive)]);
    assert!(err(&o).contains("workspace 'shop' already exists") && err(&o).contains("--as"), "{}", err(&o));
    let o = kivadb(&home, &["restore", s(&archive), "--as", "shop_copy"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("Workspace 'shop' restored as 'shop_copy'"), "{}", out(&o));
    let o = kivadb(&home, &["-w", "shop_copy", "-e", "get product:1.price"]);
    assert!(out(&o).contains("product:1.price: 12"), "{}{}", out(&o), err(&o));
    assert!(err(&kivadb(&home, &["restore", s(&archive), "--as", "bad name!"])).contains("workspace names"));
}

/// Rewrites an archive with one file's content changed, and its manifest kept.
fn tamper(archive: &Path, to: &Path) {
    let mut input = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(archive).unwrap()));
    let mut output =
        tar::Builder::new(flate2::write::GzEncoder::new(std::fs::File::create(to).unwrap(), Default::default()));
    for entry in input.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if path.ends_with("store.kiva") {
            let last = data.len() - 1;
            data[last] ^= 0xff;
        }
        let mut header = entry.header().clone();
        output.append_data(&mut header, &path, &data[..]).unwrap();
    }
    output.into_inner().unwrap().finish().unwrap();
}

#[test]
fn a_damaged_archive_is_refused_before_anything_changes() {
    let dir = TempDir::new("backup_damaged");
    let home = dir.path().join("home");
    fill(&home);
    let archive = dir.path().join("a.tar.gz");
    assert!(kivadb(&home, &["backup", "-o", s(&archive)]).status.success());

    // One byte changed: the archive no longer matches its .sha256.
    let mut bytes = std::fs::read(&archive).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 1;
    let broken = dir.path().join("b.tar.gz");
    std::fs::write(&broken, &bytes).unwrap();
    std::fs::copy(dir.path().join("a.tar.gz.sha256"), dir.path().join("b.tar.gz.sha256")).unwrap();
    let target = dir.path().join("t1");
    let o = kivadb(&home, &["restore", s(&broken), "--target", s(&target)]);
    assert!(err(&o).contains("does not match"), "{}", err(&o));
    assert!(!target.exists(), "a refused restore leaves nothing behind");

    // Without a .sha256, the manifest still catches a changed file.
    let changed = dir.path().join("c.tar.gz");
    tamper(&archive, &changed);
    let o = kivadb(&home, &["restore", s(&changed), "--target", s(&dir.path().join("t2"))]);
    assert!(err(&o).contains("checking the files inside it only"), "{}", err(&o));
    assert!(err(&o).contains("store.kiva does not match its SHA-256"), "{}", err(&o));
    assert!(!dir.path().join("t2").join("kiva.conf").exists());

    // Not an archive at all.
    let junk = dir.path().join("junk.tar.gz");
    std::fs::write(&junk, b"not a backup").unwrap();
    assert!(err(&kivadb(&home, &["restore", s(&junk), "--check"])).contains("cannot be restored"));
    assert!(err(&kivadb(&home, &["restore", s(&dir.path().join("none.tar.gz"))])).contains("does not exist"));
}

#[test]
fn a_running_server_makes_the_backup_and_blocks_restores() {
    let dir = TempDir::new("backup_running");
    let home = dir.path().join("home");
    fill(&home);
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
    let o = kivadb(&home, &["start", "--port", &port, "--no-http", "--no-resp"]);
    assert!(o.status.success(), "{}", err(&o));
    let archive = dir.path().join("live.tar.gz");
    let o = kivadb(&home, &["backup", "-o", s(&archive)]);
    let ws_archive = dir.path().join("shop.tar.gz");
    let o2 = kivadb(&home, &["backup", "-o", s(&ws_archive), "-w", "shop"]);
    let busy = kivadb(&home, &["restore", s(&ws_archive), "--as", "other"]);
    let stopped = kivadb(&home, &["stop"]);
    assert!(o.status.success() && out(&o).contains("Backup written:"), "{}{}", out(&o), err(&o));
    assert!(o2.status.success(), "{}{}", out(&o2), err(&o2));
    assert!(err(&busy).contains("KivaDB is running"), "{}", err(&busy));
    assert!(stopped.status.success());
    let target = dir.path().join("restored");
    assert!(kivadb(&home, &["restore", s(&archive), "--target", s(&target)]).status.success());
    assert!(out(&kivadb(&target, &["-e", "get user:1.city"])).contains("Goma"));
}

#[test]
fn a_backup_taken_during_writes_is_a_consistent_state() {
    let dir = TempDir::new("backup_writes");
    let inst = instance(&dir);
    let writer = {
        let inst = inst.clone();
        std::thread::spawn(move || {
            let mut s = Session::new(inst, true, None);
            for i in 0..3000 {
                ok(&mut s, &format!("set n:{i:05} {i}"));
            }
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    let archive = dir.path().join("during.tar.gz");
    let report = backup::backup(&inst, &Scope::Workspace("default".into()), &archive).unwrap();
    writer.join().unwrap();
    assert_eq!(report.workspaces, ["default"]);

    // The copy holds the first keys written, every one of them whole.
    let home2 = dir.path().join("home2");
    let lines =
        backup::restore(&archive, &home2, &RestoreOptions { rename: Some("copy".into()), ..Default::default() })
            .unwrap();
    assert!(lines.iter().any(|l| l.contains("restored as 'copy'")), "{lines:?}");
    let inst2 = kivadb::instance::Instance::open(&home2, Default::default()).unwrap();
    let mut s2 = Session::new(inst2, true, None);
    ok(&mut s2, "workspace switch copy");
    let count = s2.engine.len();
    assert!(count > 0, "nothing was saved");
    for i in 0..count {
        let v = ok(&mut s2, &format!("get n:{i:05}"));
        assert_eq!(v, format!("n:{i:05}: {i} [number]"), "keys must be a prefix of the writes");
    }
}

#[test]
fn an_encrypted_home_stays_encrypted_in_its_backup() {
    let dir = TempDir::new("backup_encrypted");
    let home = dir.path().join("home");
    let key = "ab".repeat(32);
    let run = |home: &PathBuf, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .arg("--home")
            .arg(home)
            .args(args)
            .env("KIVADB_NO_UPDATE_CHECK", "1")
            .env("KIVADB_ENCRYPTION_KEY", &key)
            .output()
            .unwrap()
    };
    assert!(run(&home, &["-e", "set secret \"Kinshasa-0042\""]).status.success());
    let archive = dir.path().join("enc.tar.gz");
    let o = run(&home, &["backup", "-o", s(&archive)]);
    assert!(out(&o).contains("encrypted at rest (key "), "{}", out(&o));

    // The secret is nowhere in the unpacked archive.
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&archive).unwrap()).read_to_end(&mut raw).unwrap();
    assert!(!raw.windows(13).any(|w| w == b"Kinshasa-0042"));

    let target = dir.path().join("restored");
    let o = kivadb(&home, &["restore", s(&archive), "--target", s(&target)]);
    assert!(out(&o).contains("encrypted with key"), "{}", out(&o));
    assert!(err(&kivadb(&target, &["-e", "get secret"])).contains("encrypted"));
    assert!(out(&run(&target, &["-e", "get secret"])).contains("Kinshasa-0042"));
}
