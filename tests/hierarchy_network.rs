// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

mod common;

use common::{instance, ok, TempDir};
use kivadb::client::Client;
use kivadb::proto::*;
use kivadb::session::Session;
use kivadb::{Instance, Options, Value};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Server {
    inst: Arc<Instance>,
    port: u16,
}

impl Server {
    fn open(dir: &TempDir) -> Self {
        let inst = Instance::open_hierarchical(dir.path(), Options::default()).unwrap();
        let port = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
        Self { inst, port }
    }

    fn connect(&self) -> Client {
        Client::connect(("127.0.0.1", self.port)).unwrap()
    }

    fn client(&self, user: &str) -> Client {
        let mut client = self.connect();
        client.negotiate_hierarchy().unwrap();
        client.auth(user, "test-pass").unwrap();
        client
    }

    fn raw(&self) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.inst.shutdown();
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

const A: &str = "use collection Fordima.app.users";
const B: &str = "use collection Trading.app.users";

fn provision(dir: &TempDir, capacity: usize) {
    let inst = instance(dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "set historical 7");
    ok(&mut admin, "modify user kiva password test-pass");
    ok(&mut admin, "create user editor identified by test-pass role editor");
    ok(&mut admin, "create user viewer identified by test-pass role viewer");
    ok(&mut admin, "create user manager identified by test-pass role editor");
    ok(&mut admin, &format!("config set storage.managed_open_engines {capacity}"));
    ok(&mut admin, "config set auth_required true");
    admin.login("kiva", "test-pass").unwrap();
    ok(&mut admin, "hierarchy data");
    ok(&mut admin, "workspace create Fordima Trading");
    for workspace in ["Fordima", "Trading"] {
        ok(&mut admin, &format!("switch {workspace}"));
        ok(&mut admin, "project create app");
        ok(&mut admin, "use app");
        ok(&mut admin, "collection create users");
    }
    ok(&mut admin, "hierarchy grant editor write collection Fordima.app.users");
    ok(&mut admin, "hierarchy grant viewer write collection Fordima.app.users");
    ok(&mut admin, "hierarchy grant manager manage collection Fordima.app.users");
    drop(admin);
    inst.shutdown();
}

fn request(stream: &mut TcpStream, code: u16, bytes: &[u8]) -> (u16, Vec<u8>) {
    write_frame(stream, code, bytes).unwrap();
    read_frame(stream).unwrap()
}

#[test]
fn negotiation_auth_and_adapter_boundaries_are_explicit() {
    let dir = TempDir::new("hierarchy_native_negotiation");
    assert!(Instance::open_hierarchical(dir.path(), Options::default()).is_err());
    provision(&dir, 32);
    assert!(Instance::open_hierarchical(dir.path(), Options { skip_auth: true, ..Options::default() }).is_err());
    let server = Server::open(&dir);
    assert!(kivadb::http::start_http(&server.inst, "127.0.0.1", 0).is_err());
    assert!(kivadb::resp::start_resp(&server.inst, "127.0.0.1", 0).is_err());
    assert!(kivadb::server::start_tcp(&server.inst, "0.0.0.0", 0).is_err());
    let mut old = server.connect();
    assert!(old.hello().unwrap().contains("capability=hierarchy/1"));
    assert!(old.auth("kiva", "test-pass").unwrap_err().to_string().contains("negotiate"));
    assert!(old.exec("get historical").unwrap_err().to_string().contains("negotiate"));
    old.negotiate_hierarchy().unwrap();
    assert!(old.get("historical").unwrap_err().to_string().contains("authentication required"));
    assert!(old.auth("kiva@default", "test-pass").is_err());
    assert!(old.auth("kiva", "").is_err());
    old.auth("kiva", "test-pass").unwrap();
    assert!(old.get("historical").unwrap_err().to_string().contains("No collection selected"));
    old.exec(A).unwrap();
    assert_eq!(old.get("historical").unwrap(), None);
    assert!(old.exec("hierarchy off").is_err());
    assert!(old.exec("subscribe *").is_err());
    assert!(old.exec("workspace list --details").is_err());
    assert!(!old.exec("stats").unwrap().contains("Database Path"));
    assert!(old.api(&json!({"op":"stats"})).is_err());
    assert!(old.api(&json!({"op":"semantic","action":"lookup"})).is_err());
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .args([
            "--connect",
            &format!("127.0.0.1:{}", server.port),
            "--hierarchical-client",
            "-u",
            "kiva",
            "-p",
            "test-pass",
            "-e",
            A,
            "-e",
            "whoami",
        ])
        .output()
        .unwrap();
    assert!(cli.status.success(), "{cli:?}");
    assert!(String::from_utf8_lossy(&cli.stdout).contains("Fordima.app.users"));
    let mut text = server.raw();
    text.write_all(b"get historical\n").unwrap();
    let mut reply = String::new();
    BufReader::new(text).read_line(&mut reply).unwrap();
    assert!(reply.contains("negotiated binary protocol"));
    let mut raw = server.raw();
    assert_eq!(request(&mut raw, CMD_NEGOTIATE, b"hierarchy/2").0, ST_ERR);
    assert_eq!(request(&mut raw, CMD_NEGOTIATE, HIERARCHY_V1.as_bytes()).0, ST_OK);
    assert_eq!(request(&mut raw, CMD_AUTH, b"kiva\0test-pass").0, ST_OK);
    assert_eq!(request(&mut raw, CMD_REPL_SYNC, b"").0, ST_ERR);
    assert_eq!(request(&mut raw, CMD_FAILOVER, b"{}").0, ST_ERR);
    assert_eq!(request(&mut raw, CMD_API, b"{bad").0, ST_ERR);
    assert_eq!(request(&mut raw, CMD_EXEC, &[0xff]).0, ST_ERR);
    assert_eq!(request(&mut raw, CMD_SET, &[255]).0, ST_ERR);
    raw.write_all(&[0, 48, 0, 0, 0, CMD_GET as u8]).unwrap();
    assert!(read_frame(&mut raw).is_err());
}

#[test]
fn native_binary_json_and_typed_contexts_share_authorization_and_data() {
    let dir = TempDir::new("hierarchy_native_routes");
    provision(&dir, 32);
    let server = Server::open(&dir);
    let mut a = server.client("kiva");
    let mut b = server.client("kiva");
    let mut editor = server.client("editor");
    let mut viewer = server.client("viewer");
    let mut manager = server.client("manager");
    a.exec(A).unwrap();
    b.exec(B).unwrap();
    a.set("binary", &Value::Bytes(vec![0, 1, 255]), 600).unwrap();
    b.set("binary", &Value::Str("isolated".into()), 0).unwrap();
    editor.exec("switch Fordima").unwrap();
    editor.exec("use app").unwrap();
    assert!(editor.exec("use users").is_err());
    editor.exec("use collection users").unwrap();
    let hidden = editor.exec(B).unwrap_err().to_string();
    assert_eq!(hidden, editor.exec("use collection Trading.app.missing").unwrap_err().to_string());
    assert_eq!(editor.get("binary").unwrap(), Some(Value::Bytes(vec![0, 1, 255])));
    assert_eq!(b.get("binary").unwrap(), Some(Value::Str("isolated".into())));
    assert_eq!(editor.api(&json!({"op":"whoami"})).unwrap()["hierarchy_path"], "Fordima.app.users");
    viewer.exec(A).unwrap();
    assert!(viewer.get("binary").unwrap().is_some());
    assert!(viewer.set("test", &Value::Str("no".into()), 0).is_err());
    assert!(viewer.api(&json!({"op":"del","key":"binary"})).is_err());
    manager.exec(A).unwrap();
    assert!(manager.get("binary").is_err());
    assert!(manager.api(&json!({"op":"scan"})).is_err());
    assert!(manager.exec("hierarchy grant manager read instance").is_err());
    a.exec(r#"set doc:1 {"city":"Paris","embedding":[1,0]}"#).unwrap();
    a.exec("create index on doc:* (city)").unwrap();
    a.exec("create vector index on doc:* (embedding)").unwrap();
    a.exec("changes enable").unwrap();
    editor.api(&json!({"op":"put","type":"date","key":"born","value":"2004-05-09"})).unwrap();
    assert_eq!(editor.api(&json!({"op":"get","key":"born","types":"extended"})).unwrap()["type"], "date");
    assert!(editor
        .api(&json!({"op":"explain","target":"doc:*.city","cmp":"=","value":"Paris","analyze":true}))
        .is_ok());
    assert!(editor.api(&json!({"op":"similar","target":"doc:*.embedding","vector":[1,0]})).is_ok());
    assert!(viewer.api(&json!({"op":"changes","wait":1})).is_err());
    assert!(viewer.api(&json!({"op":"changes","consumer":"reader"})).is_err());
    assert!(viewer.api(&json!({"op":"changes","from":"beginning"})).unwrap()["changes"].is_array());

    let mut workspace = editor.workspace("Fordima").unwrap();
    let mut project = workspace.project("app").unwrap();
    let mut selected = project.collection("users").unwrap();
    selected.begin().unwrap();
    selected.set("pending", &Value::Str("never".into()), 0).unwrap();
    a.exec("hierarchy revoke editor write collection Fordima.app.users").unwrap();
    assert!(selected.commit().is_err());
    assert_eq!(a.get("pending").unwrap(), None);
    assert!(selected.get("binary").is_err());
    a.exec("hierarchy grant editor write collection Fordima.app.users").unwrap();
    selected.begin().unwrap();
    selected.set("pending", &Value::Str("confirmed".into()), 0).unwrap();
    selected.commit().unwrap();
    assert_eq!(a.get("pending").unwrap(), Some(Value::Str("confirmed".into())));
    selected.begin().unwrap();
    selected.set("orphan", &Value::Str("never".into()), 0).unwrap();
    a.exec("drop user editor").unwrap();
    a.exec("create user editor identified by test-pass role editor").unwrap();
    assert!(selected.commit().is_err());
    assert_eq!(a.get("orphan").unwrap(), None);
    assert!(selected.get("binary").is_err());

    let mut selected = b.collection("Fordima", "app", "users").unwrap();
    a.exec("collection rename users to people").unwrap();
    assert!(selected.get("binary").unwrap().is_some());
    a.exec("collection drop people").unwrap();
    a.exec("collection create users").unwrap();
    assert!(selected.get("binary").is_err());
}

#[test]
fn tls_and_slow_clients_do_not_hold_admission_and_disconnect_releases_pins() {
    let dir = TempDir::new("hierarchy_native_resources");
    provision(&dir, 1);
    let server = Server::open(&dir);
    let tls = server.inst.tls().unwrap();
    let mut secure = Client::connect_tls(&format!("127.0.0.1:{}", server.port), Some(&tls.fingerprint)).unwrap();
    secure.negotiate_hierarchy().unwrap();
    secure.auth("kiva", "test-pass").unwrap();
    secure.exec(A).unwrap();
    secure.exec("begin").unwrap();
    secure.set("uncommitted", &Value::Str("never".into()), 0).unwrap();

    let mut slow = server.raw();
    slow.write_all(&[0, 0]).unwrap();
    let mut admin = server.client("kiva");
    admin.exec(B).unwrap();
    assert!(admin.get("uncommitted").unwrap_err().to_string().contains("limit reached"));
    admin.exec(A).unwrap();
    assert!(admin.exec("compact").unwrap_err().to_string().contains("collection transaction"));
    assert!(admin.exec("collection drop users").is_err());
    drop(secure);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match admin.exec("compact") {
            Ok(_) => break,
            Err(e) => {
                assert!(Instant::now() < deadline, "{e}");
                std::thread::yield_now();
            }
        }
    }
    assert_eq!(admin.get("uncommitted").unwrap(), None);
    admin.exec(B).unwrap();
    admin.set("persisted", &Value::Str("yes".into()), 0).unwrap();
    assert_eq!(admin.get("persisted").unwrap(), Some(Value::Str("yes".into())));
}

#[test]
fn legacy_clients_and_managed_restart_keep_separate_semantics() {
    let dir = TempDir::new("hierarchy_native_reopen");
    provision(&dir, 32);
    let server = Server::open(&dir);
    let weak = Arc::downgrade(&server.inst);
    let mut client = server.client("kiva");
    let mut collection = client.collection("Fordima", "app", "users").unwrap();
    collection.begin().unwrap();
    collection.set("committed", &Value::Str("yes".into()), 0).unwrap();
    collection.commit().unwrap();
    collection.begin().unwrap();
    collection.set("aborted", &Value::Str("no".into()), 0).unwrap();
    collection.rollback().unwrap();
    drop(client);
    drop(server);
    let deadline = Instant::now() + Duration::from_secs(5);
    while weak.strong_count() > 0 {
        assert!(Instant::now() < deadline, "native listener did not release the instance");
        std::thread::yield_now();
    }
    let server = Server::open(&dir);
    let mut client = server.client("kiva");
    assert!(client.get("committed").is_err());
    let mut collection = client.collection("Fordima", "app", "users").unwrap();
    assert_eq!(collection.get("committed").unwrap(), Some(Value::Str("yes".into())));
    assert_eq!(collection.get("aborted").unwrap(), None);

    let legacy_dir = TempDir::new("legacy_native_compat");
    let legacy = instance(&legacy_dir);
    let port = kivadb::server::start_tcp(&legacy, "127.0.0.1", 0).unwrap();
    let mut old = Client::connect(("127.0.0.1", port)).unwrap();
    assert!(old.negotiate_hierarchy().is_err());
    old.set("legacy", &Value::Str("unchanged".into()), 0).unwrap();
    assert_eq!(old.get("legacy").unwrap(), Some(Value::Str("unchanged".into())));
    drop(old);
    legacy.shutdown();
    let _ = TcpStream::connect(("127.0.0.1", port));
}

#[cfg(feature = "crash-points")]
#[test]
fn commit_before_lost_reply_survives_server_crash() {
    use std::process::{Child, Command, Stdio};
    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let dir = TempDir::new("hierarchy_commit_lost_reply");
    provision(&dir, 32);
    let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .arg("serve")
            .arg("--hierarchical-tcp")
            .arg("--home")
            .arg(dir.path())
            .args(["--bind", "127.0.0.1", "--port", &port.to_string()])
            .env("KIVADB_CRASH_ARMED", "1")
            .env("KIVADB_CRASH_POINT", "commit.before_reply")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut raw = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(e) => {
                assert!(Instant::now() < deadline && child.0.try_wait().unwrap().is_none(), "{e}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    raw.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_eq!(request(&mut raw, CMD_NEGOTIATE, HIERARCHY_V1.as_bytes()).0, ST_OK);
    assert_eq!(request(&mut raw, CMD_AUTH, b"kiva\0test-pass").0, ST_OK);
    assert_eq!(request(&mut raw, CMD_EXEC, A.as_bytes()).0, ST_OK);
    assert_eq!(request(&mut raw, CMD_EXEC, b"begin").0, ST_OK);
    assert_eq!(request(&mut raw, CMD_EXEC, b"set uncertain 42").0, ST_OK);
    write_frame(&mut raw, CMD_API, br#"{"op":"commit"}"#).unwrap();
    assert!(read_frame(&mut raw).is_err());
    assert!(!child.0.wait().unwrap().success());
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.0.stderr.take().unwrap(), &mut stderr).unwrap();
    assert!(stderr.contains("KIVADB-CRASH-POINT commit.before_reply"), "{stderr}");
    let server = Server::open(&dir);
    let mut client = server.client("kiva");
    let mut collection = client.collection("Fordima", "app", "users").unwrap();
    assert_eq!(collection.get("uncertain").unwrap(), Some(Value::Num("42".into())));
}

#[test]
fn concurrent_network_writes_survive_compaction_and_rename_across_workspaces() {
    let dir = TempDir::new("hierarchy_native_concurrency");
    provision(&dir, 32);
    let server = Server::open(&dir);
    let mut admin = server.client("kiva");
    admin.exec(A).unwrap();
    let clients: Vec<_> = (0..4)
        .map(|i| {
            let mut client = server.client("kiva");
            client.exec(if i < 2 { A } else { B }).unwrap();
            (i, client)
        })
        .collect();
    let gate = std::sync::Barrier::new(5);
    std::thread::scope(|scope| {
        for (index, mut client) in clients {
            let gate = &gate;
            scope.spawn(move || {
                gate.wait();
                for n in 0..8 {
                    let key = format!("worker{index}:{n}");
                    let value = Value::Num(index.to_string());
                    client.set(&key, &value, 0).unwrap();
                    assert_eq!(client.get(&key).unwrap(), Some(value));
                }
            });
        }
        gate.wait();
        admin.exec("compact").unwrap();
        admin.exec("collection rename users to people").unwrap();
    });
    assert_eq!(admin.get("worker0:7").unwrap(), Some(Value::Num("0".into())));
    assert_eq!(admin.get("worker2:7").unwrap(), None);
    admin.exec(B).unwrap();
    assert_eq!(admin.get("worker2:7").unwrap(), Some(Value::Num("2".into())));
    assert_eq!(admin.get("worker0:7").unwrap(), None);
}
