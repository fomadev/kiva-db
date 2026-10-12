mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::proto::*;
use kivadb::session::Session;
use kivadb::util::b64_encode;
use kivadb::{http, server};
use serde_json::{json, Value as Json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn connect(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s
}

fn call(s: &mut TcpStream, cmd: u16, payload: &[u8]) -> (u16, String) {
    write_frame(s, cmd, payload).unwrap();
    let (code, body) = read_frame(s).unwrap();
    (code, String::from_utf8_lossy(&body).into_owned())
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn text_and_binary_protocols() {
    let dir = TempDir::new("tcp");
    let inst = instance(&dir);
    let port = server::start_tcp(&inst, "127.0.0.1", 0).unwrap();

    // Plain text, as typed in telnet.
    let mut text = connect(port);
    let mut lines = BufReader::new(text.try_clone().unwrap());
    let mut ask = |cmd: &str| {
        text.write_all(format!("{cmd}\n").as_bytes()).unwrap();
        let mut line = String::new();
        lines.read_line(&mut line).unwrap();
        line.trim_end().to_string()
    };
    assert_eq!(ask(r#"set user:1 "Alice""#), "OK: user:1 saved.");
    assert_eq!(ask("get user:1"), "user:1: Alice [string]");
    assert_eq!(ask("nonsense"), "Unknown command: 'nonsense'. Try 'help'.");

    // Binary frames.
    let mut bin = connect(port);
    let (code, hello) = call(&mut bin, CMD_HELLO, &[]);
    assert_eq!(code, ST_OK);
    assert!(hello.starts_with("KivaDB 1.0.0") && hello.contains("auth=none"));
    assert_eq!(call(&mut bin, CMD_PING, &[]), (ST_OK, "PONG".into()));
    assert_eq!(call(&mut bin, CMD_EXEC, b"get user:1"), (ST_OK, "+user:1: Alice [string]".into()));
    let (code, body) = call(&mut bin, CMD_EXEC, b"update user:1 5");
    assert_eq!(code, ST_ERR);
    assert!(body.starts_with("-Error: Cannot update [string]"));

    // Typed SET / GET / DEL without going through the command language.
    let mut set = vec![6u8]; // object
    set.extend_from_slice(&0u32.to_be_bytes());
    set.extend_from_slice(&(6u16).to_be_bytes());
    set.extend_from_slice(b"user:2");
    set.extend_from_slice(br#"{"name":"Bob"}"#);
    assert_eq!(call(&mut bin, CMD_SET, &set).0, ST_OK);
    write_frame(&mut bin, CMD_GET, b"user:2").unwrap();
    let (code, body) = read_frame(&mut bin).unwrap();
    assert_eq!((code, body[0], &body[1..]), (ST_OK, 6, &br#"{"name":"Bob"}"#[..]));
    assert_eq!(call(&mut bin, CMD_GET, b"ghost").0, ST_NOT_FOUND);
    assert_eq!(call(&mut bin, CMD_DEL, b"user:2"), (ST_OK, "1".into()));
    assert_eq!(call(&mut bin, CMD_DEL, b"user:2").0, ST_NOT_FOUND);
    let mut bad = vec![2u8]; // number
    bad.extend_from_slice(&0u32.to_be_bytes());
    bad.extend_from_slice(&(1u16).to_be_bytes());
    bad.extend_from_slice(b"nnot-a-number");
    assert_eq!(call(&mut bin, CMD_SET, &bad).0, ST_ERR);
    assert_eq!(call(&mut bin, 999, &[]).0, ST_ERR);

    // Notifications are pushed to the subscriber while another client writes.
    assert_eq!(call(&mut bin, CMD_EXEC, b"subscribe order:*").0, ST_OK);
    assert_eq!(ask("set order:7 99"), "OK: order:7 saved.");
    let (code, ev) = read_frame(&mut bin).unwrap();
    assert_eq!((code, String::from_utf8_lossy(&ev).as_ref()), (ST_EVENT, "set order:7 (number)"));

    // Both transports share the same data, and `exit` closes the connection.
    assert_eq!(call(&mut bin, CMD_EXEC, b"get order:7"), (ST_OK, "+order:7: 99 [number]".into()));
    assert_eq!(call(&mut bin, CMD_EXEC, b"exit").0, ST_BYE);
    assert!(read_frame(&mut bin).is_err());
}

#[test]
fn network_logins() {
    let dir = TempDir::new("tcpauth");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"modify user "root" password "pw""#);
    ok(&mut admin, r#"create user "eve" identified by "evepw" role viewer"#);
    ok(&mut admin, "config set auth_required true");
    ok(&mut admin, r#"set secret "42""#);
    let port = server::start_tcp(&inst, "127.0.0.1", 0).unwrap();

    let mut c = connect(port);
    assert!(call(&mut c, CMD_HELLO, &[]).1.contains("auth=required"));
    let (code, body) = call(&mut c, CMD_EXEC, b"get secret");
    assert_eq!(code, ST_ERR);
    assert!(body.contains("authentication required"));
    assert_eq!(call(&mut c, CMD_GET, b"secret").0, ST_ERR);
    assert_eq!(call(&mut c, CMD_AUTH, b"root\0wrong").0, ST_ERR);
    assert_eq!(call(&mut c, CMD_AUTH, b"root\0pw"), (ST_OK, "Welcome root (admin)".into()));
    assert_eq!(call(&mut c, CMD_EXEC, b"get secret"), (ST_OK, "+secret: 42 [string]".into()));

    // A viewer cannot start a replication stream.
    let mut v = connect(port);
    assert_eq!(call(&mut v, CMD_AUTH, b"eve\0evepw").0, ST_OK);
    let (code, body) = call(&mut v, CMD_REPL_SYNC, b"");
    assert_eq!(code, ST_ERR);
    assert!(body.contains("admin role"));

    // Text mode logs in with the `auth` command.
    let mut t = connect(port);
    let mut lines = BufReader::new(t.try_clone().unwrap());
    let mut line = String::new();
    t.write_all(b"get secret\n").unwrap();
    lines.read_line(&mut line).unwrap();
    assert!(line.contains("authentication required"));
    line.clear();
    t.write_all(b"auth root pw\n").unwrap();
    lines.read_line(&mut line).unwrap();
    assert_eq!(line.trim_end(), "Welcome root (admin)");
}

struct HttpReply {
    status: u16,
    head: String,
    body: String,
}

impl HttpReply {
    fn json(&self) -> Json {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.body))
    }
}

fn http_req(port: u16, method: &str, path: &str, body: Option<&str>, auth: Option<&str>) -> HttpReply {
    let mut s = connect(port);
    let body = body.unwrap_or("");
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n", body.len());
    if let Some(a) = auth {
        req.push_str(&format!("Authorization: Basic {}\r\n", b64_encode(a.as_bytes())));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    HttpReply { status, head: head.to_string(), body: body.to_string() }
}

#[test]
fn rest_api() {
    let dir = TempDir::new("http");
    let inst = instance(&dir);
    let port = http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let get = |p: &str| http_req(port, "GET", p, None, None);
    let post = |p: &str, b: &str| http_req(port, "POST", p, Some(b), None);

    let r = post("/v1/key/user:1", r#"{"value": {"name": "Alice", "age": 30}, "ttl": 3600}"#);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["type"], "object");
    assert!(r.head.contains("Access-Control-Allow-Origin: *"));
    post("/v1/key/user:2", r#"{"name": "Bob", "age": 41}"#);
    post("/v1/key/greeting", r#""hello""#);
    post("/v1/key/count?ttl=60", "12");
    post("/v1/key/plain", "just text, not JSON");
    post("/set", r#"{"key": "legacy", "value": true}"#);
    assert_eq!(post("/v1/key/123", "1").status, 400);

    let r = get("/v1/key/user:1");
    let j = r.json();
    assert_eq!(
        (r.status, &j["status"], &j["type"], &j["data"]["name"]),
        (200, &json!("ok"), &json!("object"), &json!("Alice"))
    );
    assert!(j["ttl"].as_i64().unwrap() > 3500 && j["timestamp"].as_i64().unwrap() > 0);
    assert_eq!(get("/v1/key/user:1?path=.age").json()["data"], 30);
    assert_eq!(get("/v1/key/greeting").json()["data"], "hello");
    assert_eq!(get("/v1/key/count").json()["data"], 12);
    assert_eq!(get("/v1/key/plain").json()["data"], "just text, not JSON");
    assert_eq!(get("/get/legacy").json()["data"], true);
    let r = get("/v1/key/ghost");
    assert_eq!((r.status, r.json()["status"].clone()), (404, json!("error")));

    assert_eq!(get("/v1/keys?from=user:&to=user:9").json()["data"], json!(["user:1", "user:2"]));
    assert_eq!(get("/v1/keys?limit=1&reverse=true").json()["data"], json!(["user:2"]));
    assert_eq!(get("/v1/scan?pattern=user:*").json()["count"], 2);
    let range = get("/v1/range?from=user:1&to=user:2").json();
    assert_eq!(range["data"][1]["data"]["name"], "Bob");
    let stats = get("/v1/stats").json();
    assert_eq!((&stats["data"]["keys"], &stats["data"]["role"]), (&json!(6), &json!("master")));

    let r = post("/v1/exec", r#"{"command": "count where user:*.age > 35"}"#).json();
    assert_eq!((&r["ok"], &r["output"]), (&json!(true), &json!(["Count: 1"])));
    let r = post("/v1/exec", r#"{"command": "update count \"x\""}"#).json();
    assert_eq!(r["ok"], false);

    assert_eq!(http_req(port, "DELETE", "/v1/key/user:2", None, None).json()["deleted"], true);
    assert_eq!(http_req(port, "DELETE", "/v1/key/user:2", None, None).status, 404);
    assert_eq!(http_req(port, "OPTIONS", "/v1/key/x", None, None).status, 204);
    assert_eq!(get("/nowhere").status, 404);
    assert_eq!(get("/health").json()["version"], "1.0.0");
    assert!(get("/docs").body.contains("KivaDB REST API"));
    assert!(get("/openapi.json").json()["paths"]["/v1/key/{key}"].is_object());
    let m = get("/metrics").body;
    for name in [
        "kiva_requests_total{type=\"get\"}",
        "kiva_request_duration_seconds_sum",
        "kiva_storage_fragmentation_ratio",
        "kiva_keys 5",
    ] {
        assert!(m.contains(name), "metrics lack {name}:\n{m}");
    }

    // Basic authentication uses the same accounts as the shell.
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"create user "fordi" identified by "mon_pass" role viewer"#);
    ok(&mut admin, r#"grant read on workspace "default" to "fordi""#);
    ok(&mut admin, "config set auth_required true");
    let r = get("/v1/key/greeting");
    assert_eq!(r.status, 401);
    assert!(r.head.contains("WWW-Authenticate: Basic"));
    assert_eq!(http_req(port, "GET", "/v1/key/greeting", None, Some("fordi:nope")).status, 401);
    let auth = Some("fordi:mon_pass");
    assert_eq!(http_req(port, "GET", "/v1/key/greeting?workspace=default", None, auth).json()["data"], "hello");
    assert_eq!(http_req(port, "POST", "/v1/key/x?workspace=default", Some("1"), auth).status, 403);
    assert_eq!(get("/health").status, 200);
    ok(&mut admin, "config set auth_required false");

    // Rate limiting is per client address and per minute.
    ok(&mut admin, "config set rate_limit 3");
    let codes: Vec<u16> = (0..5).map(|_| get("/health").status).collect();
    assert!(codes.ends_with(&[429]), "{codes:?}");
    assert!(codes.contains(&200) || codes.iter().all(|c| *c == 429));
}

#[test]
fn websocket_notifications() {
    let dir = TempDir::new("ws");
    let inst = instance(&dir);
    let port = http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let mut s = connect(port);
    s.write_all(
        b"GET /ws?pattern=user:* HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
          Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
    )
    .unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 101"));
    // The accept value for this key is fixed by RFC 6455.
    assert!(head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));

    let frame = |s: &mut TcpStream| -> Json {
        let mut h = [0u8; 2];
        s.read_exact(&mut h).unwrap();
        assert_eq!(h[0], 0x81);
        let mut payload = vec![0u8; (h[1] & 0x7f) as usize];
        s.read_exact(&mut payload).unwrap();
        serde_json::from_slice(&payload).unwrap()
    };
    assert_eq!(frame(&mut s)["event"], "subscribed");
    let mut writer = Session::new(inst.clone(), true, None);
    ok(&mut writer, r#"set order:1 1 and user:5 {"n": 1}"#);
    let ev = frame(&mut s);
    assert_eq!((&ev["event"], &ev["key"], &ev["type"]), (&json!("set"), &json!("user:5"), &json!("object")));
    ok(&mut writer, "del user:5");
    assert_eq!(frame(&mut s)["event"], "del");
    // A masked close frame from the client ends the stream cleanly.
    s.write_all(&[0x88, 0x80, 1, 2, 3, 4]).unwrap();
    let mut rest = Vec::new();
    let _ = s.read_to_end(&mut rest);
    assert!(rest.is_empty() || rest[0] == 0x88);
}

#[test]
fn replication_and_failover() {
    let (dir_m, dir_r) = (TempDir::new("master"), TempDir::new("replica"));
    let (master, replica) = (instance(&dir_m), instance(&dir_r));
    let port_m = server::start_tcp(&master, "127.0.0.1", 0).unwrap();
    let port_r = server::start_tcp(&replica, "127.0.0.1", 0).unwrap();
    let mut m = Session::new(master.clone(), true, None);
    let mut r = Session::new(replica.clone(), true, None);

    for i in 0..200 {
        ok(&mut m, &format!(r#"set seed:{i:03} {{"n": {i}, "pad": "{}"}}"#, "x".repeat(100)));
    }
    ok(&mut m, r#"set temp "x" ttl 3600"#);
    ok(&mut r, r#"set stale "will be replaced by the snapshot""#);
    assert!(fails(&mut r, &format!("replica of 127.0.0.1 {port_r}")).contains("cannot replicate from itself"));
    assert!(ok(&mut r, &format!("replica of 127.0.0.1 {port_m}")).contains("Now replicating"));
    wait_for("the initial snapshot", || replica.repl.link().status == "connected");
    let rep = replica.engine("default").unwrap();
    assert_eq!(rep.len(), 201);
    assert!(!rep.contains("stale"));
    assert_eq!(ok(&mut r, "get seed:042.n"), "seed:042.n: 42 [number]");
    assert!(rep.meta("temp").unwrap().expires_at > 0);

    // Live stream: sets, updates, deletes, transactions.
    ok(&mut m, r#"set live "one""#);
    ok(&mut m, r#"update live "two""#);
    ok(&mut m, "del seed:000");
    ok(&mut m, "begin");
    ok(&mut m, "set t1 1 and t2 2");
    ok(&mut m, "commit");
    assert_eq!(ok(&mut m, "wait 1"), "OK: 1 replica(s) acknowledged.");
    assert_eq!(
        ok(&mut r, "get live and seed:000 and t1 and t2"),
        "live: two [string]\nseed:000: (nil)\nt1: 1 [number]\nt2: 2 [number]"
    );
    assert!(fails(&mut m, "wait 2 300").contains("only 1 of 2"));

    // A replica refuses writes but serves reads.
    assert!(fails(&mut r, "set nope 1").contains("READONLY"));
    assert!(fails(&mut r, "del all keys").contains("READONLY"));
    let status = ok(&mut m, "replica status");
    assert!(
        status.contains("Role: master") && status.contains("Connected replicas: 1") && status.contains("lag: 0 ops")
    );
    let status = ok(&mut r, "replica status");
    assert!(status.contains("Role: replica") && status.contains("Link: connected"));
    assert!(ok(&mut r, "stats").contains(&format!("replica of 127.0.0.1:{port_m} (connected)")));

    // Synchronous mode waits for the acknowledgement before returning.
    ok(&mut m, "config set sync_mode sync");
    ok(&mut m, r#"set synced "yes""#);
    assert_eq!(ok(&mut r, "get synced"), "synced: yes [string]");
    ok(&mut m, "config set sync_mode async");

    // A dropped link reconnects and resynchronises by itself.
    ok(&mut r, "replica stop");
    ok(&mut m, r#"set while_down "missed""#);
    assert_eq!(ok(&mut r, "get while_down"), "while_down: (nil)");
    ok(&mut r, "replica start");
    wait_for("the resync", || replica.repl.link().status == "connected" && rep.contains("while_down"));

    // Failover: the replica becomes the master, the old master follows it.
    wait_for("the old link to be forgotten", || master.repl.replicas().len() == 1);
    let id = master.repl.replicas()[0].id;
    assert!(ok(&mut m, &format!("failover to {id}")).contains("is the new master"));
    wait_for("the promotion", || !replica.repl.is_replica());
    wait_for("the old master to follow", || master.repl.is_replica() && master.repl.link().status == "connected");
    assert!(fails(&mut m, "set nope 1").contains("READONLY"));
    ok(&mut r, r#"set after_failover "written on the new master""#);
    let old = master.engine("default").unwrap();
    wait_for("replication in the new direction", || old.contains("after_failover"));
    assert_eq!(old.len(), rep.len());
    assert_eq!(ok(&mut m, "get live"), "live: two [string]");

    // And back to a standalone master.
    assert_eq!(ok(&mut m, "replica promote"), "This node is now a master.");
    ok(&mut m, "set independent 1");
}

fn kivadb(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kivadb"));
    c.arg("--home").arg(home).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    c
}

#[test]
fn command_line_and_shared_terminals() {
    let dir = TempDir::new("cli");
    let out = kivadb(dir.path()).args(["-e", r#"set a 4 and string be 'f'"#, "-e", "get a and be"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).replace('\r', ""),
        "OK: a saved.\nOK: be saved.\na: 4 [number]\nbe: f [string]\n"
    );
    let out = kivadb(dir.path()).args(["-e", "update a \"x\""]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Cannot update [number]"));
    let out = Command::new(env!("CARGO_BIN_EXE_kivadb")).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "KivaDB 1.0.0");

    // Terminal 1 keeps a shell open...
    let mut first = kivadb(dir.path()).spawn().unwrap();
    let mut stdin = first.stdin.take().unwrap();
    let mut stdout = BufReader::new(first.stdout.take().unwrap());
    let mut send = |cmd: &str| {
        stdin.write_all(format!("{cmd}\n").as_bytes()).unwrap();
        stdin.flush().unwrap();
    };
    let mut recv = || {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        line.trim_end().to_string()
    };
    send("get a");
    assert_eq!(recv(), "a: 4 [number]");
    send("subscribe from_*");
    assert_eq!(recv(), "Subscribed to 'from_*' in workspace 'default'. Changes will be announced as they happen.");

    // ...and terminal 2 works on the very same data, live.
    let out = kivadb(dir.path()).args(["-e", "set from_two 2", "-e", "update a 5", "-e", "whoami"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("root (admin) @ workspace 'default'"));
    assert_eq!(recv(), "[event] set from_two (number)");
    send("get a and from_two");
    assert_eq!(recv(), "a: 5 [number]");
    assert_eq!(recv(), "from_two: 2 [number]");
    drop(send);
    drop(stdin);
    assert!(first.wait().unwrap().success());

    // After both are gone the data is still there.
    let out = kivadb(dir.path()).args(["-e", "get a and from_two"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).replace('\r', ""), "a: 5 [number]\nfrom_two: 2 [number]\n");
}

#[test]
fn a_killed_process_loses_nothing() {
    let dir = TempDir::new("kill");
    let mut child = kivadb(dir.path()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    for i in 0..300 {
        stdin.write_all(format!("set k{i:03} \"value {i}\"\n").as_bytes()).unwrap();
    }
    stdin.write_all(b"begin\nset in_txn 1\nping\n").unwrap();
    stdin.flush().unwrap();
    while line.trim_end() != "PONG" {
        line.clear();
        assert!(stdout.read_line(&mut line).unwrap() > 0);
    }
    // No clean shutdown: no final snapshot, an open transaction.
    child.kill().unwrap();
    child.wait().unwrap();

    let out = kivadb(dir.path())
        .args(["-e", "get k000 and k299 and in_txn", "-e", "keys limit 1", "-e", "check"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    assert!(text.starts_with("k000: value 0 [string]\nk299: value 299 [string]\nin_txn: (nil)\n"), "{text}");
    assert!(text.contains("Checking 300 entries...") && text.contains("CORRUPTED: 0"));
}

#[test]
fn rust_client() {
    use kivadb::client::Client;
    use kivadb::Value;
    let dir = TempDir::new("client");
    let inst = instance(&dir);
    let port = server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let mut db = Client::connect(("127.0.0.1", port)).unwrap();
    assert!(db.hello().unwrap().contains("auth=none"));
    db.ping().unwrap();
    db.set("greeting", &Value::Str("hello".into()), 0).unwrap();
    db.set("user:1", &Value::Doc(json!({"name": "Alice", "role": "admin"})), 60).unwrap();
    db.set("blob", &Value::Bytes(vec![0, 1, 2, 255]), 0).unwrap();
    assert_eq!(db.get("greeting").unwrap(), Some(Value::Str("hello".into())));
    assert_eq!(db.get("user:1").unwrap(), Some(Value::Doc(json!({"name": "Alice", "role": "admin"}))));
    assert_eq!(db.get("blob").unwrap(), Some(Value::Bytes(vec![0, 1, 2, 255])));
    assert_eq!(db.get("ghost").unwrap(), None);
    assert_eq!(db.exec(r#"count where user:*.role = "admin""#).unwrap(), "Count: 1");
    assert!(db.exec("update greeting 5").unwrap_err().to_string().contains("Cannot update [string]"));
    assert!(db.set("123", &Value::Null, 0).is_err());
    db.exec("subscribe user:*").unwrap();
    assert!(db.del("user:1").unwrap());
    assert!(!db.del("user:1").unwrap());
    // Notifications travel on their own: keep talking until it shows up.
    wait_for("the notification", || {
        db.ping().unwrap();
        !db.events.is_empty()
    });
    assert_eq!(db.events, ["del user:1"]);
    assert!(db.auth("root", "wrong").is_err());
}

#[test]
fn start_status_stop() {
    let dir = TempDir::new("daemon");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
    let run = |args: &[&str]| {
        let out = kivadb(dir.path()).args(args).stdin(Stdio::null()).output().unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).replace('\r', "") + &String::from_utf8_lossy(&out.stderr),
        )
    };
    let (code, text) = run(&["status"]);
    assert_eq!(code, Some(1));
    assert!(text.contains("not running"));
    assert!(run(&["stop"]).1.contains("not running"));

    let (code, text) = run(&["start", "--port", &port, "--no-http"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("KivaDB started") && text.contains(&format!("TCP port : {port}")));
    assert!(run(&["start", "--port", &port, "--no-http"]).1.contains("already running"));
    let (code, text) = run(&["status"]);
    assert_eq!(code, Some(0));
    assert!(text.contains("KivaDB is running") && text.contains(&port));

    // The started database serves programs and shells alike.
    let mut db = kivadb::client::Client::connect(format!("127.0.0.1:{port}")).unwrap();
    db.set("from_code", &kivadb::Value::Num("7".into()), 0).unwrap();
    let (_, text) = run(&["-e", "get from_code", "-e", "set from_shell 1"]);
    assert_eq!(text, "from_code: 7 [number]\nOK: from_shell saved.\n");
    assert_eq!(db.get("from_shell").unwrap(), Some(kivadb::Value::Num("1".into())));
    assert!(run(&["serve"]).1.contains("already running"));

    let (code, text) = run(&["stop"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("KivaDB stopped."));
    assert!(db.ping().is_err());
    assert_eq!(run(&["status"]).0, Some(1));
    // Data written while it ran is there after a stop.
    assert_eq!(run(&["-e", "get from_code and from_shell"]).1, "from_code: 7 [number]\nfrom_shell: 1 [number]\n");
    assert!(std::fs::read_to_string(dir.path().join("server.log")).unwrap().contains("TCP protocol"));
}
