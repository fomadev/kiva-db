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

/// Every record of a data file has a type of 1.x (codes 1 to 7, or 0 for a deletion).
fn only_types_of_1_x(file: &Path) -> bool {
    let data = std::fs::read(file).unwrap();
    let mut pos = kivadb::format::FILE_HEADER_LEN as usize;
    while data.len() - pos >= kivadb::format::REC_HEADER_LEN {
        let h = kivadb::format::parse_header(&data[pos..]);
        if h.ty > 7 {
            return false;
        }
        pos += h.total_len() as usize;
    }
    true
}

/// The KivaDB 1.7.1 executable, when `KIVADB_V1_BIN` names one: the scenarios
/// that go back to 1.x then run it for real.
fn v1_bin() -> Option<PathBuf> {
    std::env::var_os("KIVADB_V1_BIN").map(PathBuf::from).filter(|p| p.is_file())
}

fn run_v1(bin: &Path, home: &Path, commands: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("--home").arg(home).arg("--no-listen");
    for c in commands {
        cmd.arg("-e").arg(c);
    }
    cmd.output().unwrap()
}

#[test]
fn a_home_used_by_2_0_without_its_new_types_goes_back_to_1_x() {
    let dir = TempDir::new("compat_back_to_1");
    let home = home_from("v1.7.1", &dir);
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, r#"set user:4 {"name": "Dan", "role": "dev"}"#);
        ok(&mut s, "del greeting");
        ok(&mut s, "compact");
        ok(&mut s, "set late 7");
        drop(s);
        inst.shutdown();
    }
    let store = home.join("data").join("store.kiva");
    assert_eq!(format_of(&store), 1);
    assert!(only_types_of_1_x(&store));
    if let Some(bin) = v1_bin() {
        let out =
            run_v1(&bin, &home, &["get user:4.name", "get late", "get greeting", "count where user:*.role = \"dev\""]);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            ["user:4.name: Dan [string]", "late: 7 [number]", "greeting: (nil)", "Count: 3"]
        );
    }
}

#[test]
fn a_home_holding_dates_is_refused_by_1_x_and_left_as_it_is() {
    let dir = TempDir::new("compat_refused_by_1");
    let home = home_from("v1.7.1", &dir);
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "set datetime seen '2026-10-27T14:30:00Z'");
        drop(s);
        inst.shutdown();
    }
    let store = home.join("data").join("store.kiva");
    assert_eq!(format_of(&store), 2);
    if let Some(bin) = v1_bin() {
        let before = std::fs::read(&store).unwrap();
        let out = run_v1(&bin, &home, &["get seen"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("unsupported storage format 2"));
        assert_eq!(std::fs::read(&store).unwrap(), before, "1.x must not touch it");
    }
    let mut s = Session::new(open(&home), true, None);
    ok(&mut s, "timezone UTC");
    assert_eq!(ok(&mut s, "get seen"), "seen: 2026-10-27 14:30:00 +00:00 [datetime]");
}

#[test]
fn a_crash_while_the_first_date_is_written_loses_nothing_else() {
    // The header is raised and synced before the record is written. A crash
    // in between leaves either the old header, or the new one over records
    // of 1.x only, or the new one and a cut date: each opens with every
    // earlier value.
    let dir = TempDir::new("compat_crash_format");
    let home = home_from("v1.7.1", &dir);
    let store = home.join("data").join("store.kiva");
    let original = std::fs::read(&store).unwrap();

    // The new header, no record yet.
    let mut raised = original.clone();
    raised[4..8].copy_from_slice(&2u32.to_le_bytes());
    std::fs::write(&store, &raised).unwrap();
    let _ = std::fs::remove_file(home.join("data").join("store.kiva.hint"));
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        the_values_of_1_7_1_are_all_there(&mut s);
        ok(&mut s, "set date born '2004-05-09'");
        drop(s);
        inst.shutdown();
    }

    // The new header and a date cut in the middle of its bytes.
    let full = std::fs::read(&store).unwrap();
    let date_len = full.len() - raised.len();
    std::fs::write(&store, &full[..raised.len() + date_len / 2]).unwrap();
    let _ = std::fs::remove_file(home.join("data").join("store.kiva.hint"));
    let inst = open(&home);
    let mut s = Session::new(inst.clone(), true, None);
    the_values_of_1_7_1_are_all_there(&mut s);
    assert_eq!(ok(&mut s, "get born"), "born: (nil)");
    assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));
    // And the next date is written whole.
    ok(&mut s, "set date born '2004-05-09'");
    assert_eq!(ok(&mut s, "get born"), "born: 2004-05-09 [date]");
    assert_eq!(format_of(&store), 2);
}

#[test]
fn a_backup_of_a_home_holding_dates_restores_them() {
    let dir = TempDir::new("compat_backup_dates");
    let home = home_from("v1.7.1", &dir);
    let archive = dir.path().join("home.tar.gz");
    {
        let inst = open(&home);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "set date born '2004-05-09'");
        ok(&mut s, "set time opens '08:00'");
        kivadb::backup::backup(&inst, &kivadb::backup::Scope::Home, &archive).unwrap();
        drop(s);
        inst.shutdown();
    }
    let target = dir.path().join("restored");
    let opts = kivadb::backup::RestoreOptions { target: Some(target.clone()), ..Default::default() };
    kivadb::backup::restore(&archive, &home, &opts).unwrap();
    assert_eq!(format_of(&target.join("data").join("store.kiva")), 2);
    let mut s = Session::new(open(&target), true, None);
    assert_eq!(ok(&mut s, "get born and opens"), "born: 2004-05-09 [date]\nopens: 08:00:00 [time]");
    the_values_of_1_7_1_are_all_there(&mut s);
    assert!(ok(&mut s, "explain get where user:*.role = \"dev\"").contains("INDEX SCAN"));
}

/// A server of KivaDB 1.7.1 in its own process, stopped when dropped.
struct V1Server(std::process::Child);

impl V1Server {
    fn start(bin: &Path, home: &Path, port: u16) -> V1Server {
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
        let server = V1Server(child);
        wait_until("1.7.1 to listen", || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
        server
    }
}

impl Drop for V1Server {
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
fn replication_never_crosses_major_versions() {
    let dir = TempDir::new("compat_replication");
    let inst = open(&dir.path().join("v2"));
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "set datetime seen '2026-10-27T13:30:00Z'");
    let port = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    // The hello of a replica of 1.x is refused, before any snapshot.
    let (code, msg) = answer_to_replica(port, "127.0.0.1:1 node=old repl=2");
    assert_eq!(code, kivadb::proto::ST_ERR);
    assert!(msg.contains("a replica of KivaDB 1.x cannot follow it: upgrade the replica to KivaDB 2"), "{msg}");

    let Some(bin) = v1_bin() else { return };
    // A real 1.7.1 replica keeps its data and says why it does not follow.
    let old = dir.path().join("v1");
    assert!(run_v1(&bin, &old, &["set kept 1"]).status.success());
    let old_port = free_port();
    let v1 = V1Server::start(&bin, &old, old_port);
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
    wait_until("1.7.1 to be refused", || cli("replica status").contains("cannot follow it"));
    assert_eq!(cli("get kept"), "kept: 1 [number]\n");
    cli("replica stop");

    // And a 2.0 replica of a 1.7.1 master keeps its own.
    ok(&mut s, &format!("replica of 127.0.0.1 {old_port}"));
    wait_until("2.0 to refuse 1.7.1", || ok(&mut s, "replica status").contains("upgrade the master to KivaDB 2 first"));
    ok(&mut s, "timezone UTC");
    assert_eq!(ok(&mut s, "get seen"), "seen: 2026-10-27 13:30:00 +00:00 [datetime]");
    ok(&mut s, "replica stop");
    drop(v1);
    inst.shutdown();
}
