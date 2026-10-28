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
    assert!(hello.starts_with(concat!("KivaDB ", env!("CARGO_PKG_VERSION"))) && hello.contains("auth=none"));
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
    ok(&mut admin, r#"modify user "kiva" password "pw""#);
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
    assert_eq!(call(&mut c, CMD_AUTH, b"kiva\0wrong").0, ST_ERR);
    assert_eq!(call(&mut c, CMD_AUTH, b"kiva\0pw"), (ST_OK, "Welcome kiva (admin)".into()));
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
    t.write_all(b"auth kiva pw\n").unwrap();
    lines.read_line(&mut line).unwrap();
    assert_eq!(line.trim_end(), "Welcome kiva (admin)");
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
    assert_eq!(get("/health").json()["version"], env!("CARGO_PKG_VERSION"));
    let ready = get("/ready").json();
    assert_eq!((&ready["ready"], &ready["role"]), (&json!(true), &json!("master")));
    assert!(get("/docs").body.contains("KivaDB REST API"));
    assert!(get("/openapi.json").json()["paths"]["/v1/key/{key}"].is_object());
    let m = get("/metrics").body;
    for name in [
        "kiva_requests_total{type=\"get\"}",
        "kiva_request_duration_seconds_sum",
        "# TYPE kiva_request_duration_seconds histogram",
        "kiva_request_duration_seconds_bucket{le=\"0.001\"}",
        "kiva_request_duration_seconds_bucket{le=\"+Inf\"}",
        "kiva_storage_fragmentation_ratio",
        "kiva_keys 5",
    ] {
        assert!(m.contains(name), "metrics lack {name}:\n{m}");
    }

    // A schema answers 422 over REST.
    ok(
        &mut Session::new(inst.clone(), true, None),
        r#"schema set acct: {"id": {"type": "integer", "required": true}}"#,
    );
    let r = post("/v1/key/acct:1", r#"{"name": "x"}"#);
    assert_eq!(r.status, 422, "{}", r.body);
    assert!(
        r.json()["error"].as_str().unwrap_or_default().starts_with("SCHEMA: 'acct:1' not written: id: required"),
        "{}",
        r.body
    );
    assert_eq!(post("/v1/key/acct:1", r#"{"id": 1}"#).status, 200);
    ok(&mut Session::new(inst.clone(), true, None), "del acct:1");
    ok(&mut Session::new(inst.clone(), true, None), "schema drop acct:");

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
    // The heartbeats carry the master's clock: one machine, so a few milliseconds at most.
    let started = Instant::now();
    let clock = loop {
        let stats = ok(&mut r, "stats");
        if let Some(line) = stats.lines().find(|l| l.contains("> Clock                :")) {
            break line.to_string();
        }
        assert!(started.elapsed() < Duration::from_secs(10), "no clock line in:\n{stats}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(clock.contains(" ms ") && clock.contains("the master") && !clock.contains("WARN"), "{clock}");

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
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), concat!("KivaDB ", env!("CARGO_PKG_VERSION")));

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
    assert!(String::from_utf8_lossy(&out.stdout).contains("kiva (admin) @ workspace 'default'"));
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
    assert!(db.auth("kiva", "wrong").is_err());
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

#[test]
fn structured_api() {
    use kivadb::client::Client;
    let dir = TempDir::new("api");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"create user "eve" identified by "evepw" role viewer"#);
    ok(&mut admin, r#"grant read on workspace "default" to "eve""#);
    let port = server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let mut db = Client::connect(("127.0.0.1", port)).unwrap();

    let me = db.api(&json!({"op": "whoami"})).unwrap();
    assert_eq!(
        (&me["user"], &me["role"], &me["workspace"], &me["can_write"]),
        (&json!("kiva"), &json!("admin"), &json!("default"), &json!(true))
    );
    assert_eq!(me["can_compact"], true);

    // Typed writes: no quoting rules to worry about.
    let put = |db: &mut Client, key: &str, ty: &str, value: Json, extra: Json| {
        let mut req = json!({"op": "put", "key": key, "type": ty, "value": value});
        for (k, v) in extra.as_object().unwrap() {
            req[k] = v.clone();
        }
        db.api(&req)
    };
    put(&mut db, "quote", "string", json!("both \" and ' quotes"), json!({"mode": "create"})).unwrap();
    put(&mut db, "user:1", "object", json!({"name": "Alice"}), json!({"ttl": 3600})).unwrap();
    put(&mut db, "n", "number", json!("3.14"), json!({})).unwrap();
    put(&mut db, "blob", "bytes", json!("AAEC/w=="), json!({})).unwrap();
    assert!(put(&mut db, "quote", "string", json!("x"), json!({"mode": "create"}))
        .unwrap_err()
        .to_string()
        .contains("already exists"));
    assert!(put(&mut db, "ghost", "string", json!("x"), json!({"mode": "update"}))
        .unwrap_err()
        .to_string()
        .contains("not found"));
    assert!(put(&mut db, "n2", "number", json!("abc"), json!({})).is_err());
    assert!(put(&mut db, "123", "null", Json::Null, json!({})).unwrap_err().to_string().contains("purely numeric"));

    let got = db.api(&json!({"op": "get", "key": "quote"})).unwrap();
    assert_eq!(
        (&got["found"], &got["type"], &got["value"]),
        (&json!(true), &json!("string"), &json!("both \" and ' quotes"))
    );
    let got = db.api(&json!({"op": "get", "key": "user:1"})).unwrap();
    assert_eq!(got["value"]["name"], "Alice");
    assert!(got["ttl"].as_i64().unwrap() > 3500);
    // Updating without a ttl keeps the expiry; ttl 0 removes it.
    put(&mut db, "user:1", "object", json!({"name": "Alicia"}), json!({})).unwrap();
    assert!(db.api(&json!({"op": "get", "key": "user:1"})).unwrap()["ttl"].as_i64().unwrap() > 3500);
    put(&mut db, "user:1", "object", json!({"name": "Alicia"}), json!({"ttl": 0})).unwrap();
    assert!(db.api(&json!({"op": "get", "key": "user:1"})).unwrap()["ttl"].is_null());
    assert_eq!(db.api(&json!({"op": "get", "key": "blob"})).unwrap()["value"], "AAEC/w==");
    assert_eq!(db.get("blob").unwrap(), Some(kivadb::Value::Bytes(vec![0, 1, 2, 255])));
    assert_eq!(db.api(&json!({"op": "get", "key": "ghost"})).unwrap()["found"], false);

    let scan = db.api(&json!({"op": "scan", "pattern": "*", "limit": 2})).unwrap();
    assert_eq!(
        (scan["keys"].as_array().unwrap().len(), &scan["truncated"], &scan["total"]),
        (2, &json!(true), &json!(4))
    );
    let scan = db.api(&json!({"op": "scan", "pattern": "user:*"})).unwrap();
    assert_eq!(
        (&scan["keys"][0]["key"], &scan["keys"][0]["type"], &scan["truncated"]),
        (&json!("user:1"), &json!("object"), &json!(false))
    );

    let stats = db.api(&json!({"op": "stats"})).unwrap();
    assert_eq!(
        (&stats["keys"], &stats["role"], &stats["version"]),
        (&json!(4), &json!("master"), &json!(env!("CARGO_PKG_VERSION")))
    );
    let users = db.api(&json!({"op": "users"})).unwrap();
    let names: Vec<&str> = users["users"].as_array().unwrap().iter().map(|u| u["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["eve", "kiva"]);
    assert_eq!((&users["users"][1]["builtin"], &users["users"][1]["empty_password"]), (&json!(true), &json!(true)));
    assert_eq!(users["users"][0]["grants"]["default"], "read");
    let ws = db.api(&json!({"op": "workspaces"})).unwrap();
    assert!(ws["workspaces"].as_array().unwrap().iter().any(|w| w["name"] == "default" && w["current"] == true));

    assert_eq!(db.api(&json!({"op": "del", "key": "n"})).unwrap()["deleted"], true);
    assert_eq!(db.api(&json!({"op": "del", "key": "n"})).unwrap()["deleted"], false);
    assert!(db.api(&json!({"op": "nonsense"})).unwrap_err().to_string().contains("unknown operation"));
    assert!(db.api(&json!({"nop": 1})).is_err());

    // The same permissions as the shell apply.
    db.auth("eve", "evepw").unwrap();
    db.exec("workspace switch default").unwrap();
    let me = db.api(&json!({"op": "whoami"})).unwrap();
    assert_eq!((&me["role"], &me["can_write"], &me["can_compact"]), (&json!("viewer"), &json!(false), &json!(false)));
    assert_eq!(db.api(&json!({"op": "get", "key": "quote"})).unwrap()["found"], true);
    assert!(put(&mut db, "x", "null", Json::Null, json!({})).unwrap_err().to_string().contains("read-only"));
    assert!(db.api(&json!({"op": "del", "key": "quote"})).unwrap_err().to_string().contains("read-only"));
    assert!(db.api(&json!({"op": "users"})).unwrap_err().to_string().contains("admin role"));
}

#[test]
fn legacy_root_account_is_renamed() {
    let dir = TempDir::new("legacy");
    {
        // A home created when the built-in administrator was called root.
        let sys = kivadb::Engine::open(dir.path().join("system").join("system.kiva"), Default::default()).unwrap();
        kivadb::auth::save_user(&sys, &kivadb::auth::User::new("root", "old-secret", kivadb::auth::Role::Admin))
            .unwrap();
    }
    let inst = instance(&dir);
    assert!(kivadb::auth::get_user(&inst.system, "root").unwrap().is_none());
    let admin = kivadb::auth::get_user(&inst.system, "kiva").unwrap().unwrap();
    assert!(admin.verify("old-secret") && admin.role == kivadb::auth::Role::Admin);
}

#[test]
fn environment_configures_a_hosted_server() {
    use kivadb::client::Client;
    let dir = TempDir::new("hosted");
    let free_port = || std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let start = |password: &str, port: u16| {
        let mut child = kivadb(dir.path())
            .args(["serve", "--no-http"])
            .env("KIVADB_ADMIN_PASSWORD", password)
            .env("KIVADB_PORT", port.to_string())
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        // "Press Ctrl+C to stop." is the last line printed once it is ready.
        let mut log = String::new();
        while !log.contains("Press Ctrl+C") {
            assert!(out.read_line(&mut log).unwrap() > 0, "server stopped early: {log}");
        }
        (child, log)
    };

    // First start: the password is installed and authentication turned on.
    let port = free_port();
    let (mut server, log) = start("s3cret-one", port);
    assert!(log.contains("administrator password set") && log.contains("Auth         : required"), "{log}");
    assert!(log.contains(&format!("127.0.0.1:{port}")));
    let mut db = Client::connect(("127.0.0.1", port)).unwrap();
    assert!(db.hello().unwrap().contains("auth=required"));
    assert!(db.exec("keys").is_err());
    assert!(db.auth("kiva", "").is_err());
    assert!(db.auth("kiva", "s3cret-one").unwrap().contains("Welcome kiva (admin)"));
    db.exec(r#"modify user "kiva" password "changed-by-hand""#).unwrap();
    db.exec("set hosted true").unwrap();
    db.exec("shutdown").unwrap();
    assert!(server.wait().unwrap().success());

    // Later starts never overwrite a password that is already set.
    let port = free_port();
    let (mut server, log) = start("s3cret-two", port);
    assert!(!log.contains("administrator password set"), "{log}");
    let mut db = Client::connect(("127.0.0.1", port)).unwrap();
    assert!(db.auth("kiva", "s3cret-two").is_err());
    db.auth("kiva", "changed-by-hand").unwrap();
    assert_eq!(db.exec("get hosted").unwrap(), "hosted: true [boolean]");
    db.exec("shutdown").unwrap();
    assert!(server.wait().unwrap().success());
}

#[test]
fn replication_covers_every_workspace() {
    let (dir_m, dir_r) = (TempDir::new("ws-master"), TempDir::new("ws-replica"));
    let (master, replica) = (instance(&dir_m), instance(&dir_r));
    let port_m = server::start_tcp(&master, "127.0.0.1", 0).unwrap();
    server::start_tcp(&replica, "127.0.0.1", 0).unwrap();
    let mut m = Session::new(master.clone(), true, None);
    let mut r = Session::new(replica.clone(), true, None);

    ok(&mut m, "workspace create sales");
    ok(&mut m, "workspace switch sales");
    for i in 0..50 {
        ok(&mut m, &format!(r#"set order:{i:02} {{"total": {i}}}"#));
    }
    ok(&mut m, "workspace switch default");
    ok(&mut m, "set config:mode \"live\"");
    // A workspace only the replica has is unregistered by the sync.
    ok(&mut r, "workspace create local_only");
    ok(&mut r, &format!("replica of 127.0.0.1 {port_m}"));
    wait_for("the snapshot of every workspace", || replica.repl.link().status == "connected");
    assert_eq!(replica.engine("sales").unwrap().len(), 50);
    assert!(replica.engine("default").unwrap().contains("config:mode"));
    let list = ok(&mut r, "workspace list");
    assert!(list.contains("sales") && !list.contains("local_only"), "{list}");
    assert!(ok(&mut r, "replica status").contains("Workspaces: 2 (default, sales)"));
    assert!(ok(&mut m, "replica status").contains("Workspaces: 2 (default, sales)"));

    // Live operations name their workspace; one sequence covers them all.
    ok(&mut m, "workspace switch sales");
    ok(&mut m, r#"set order:99 {"total": 99}"#);
    ok(&mut m, "del order:00");
    ok(&mut m, "workspace switch default");
    ok(&mut m, "update config:mode \"maintenance\"");
    ok(&mut m, "workspace switch sales");
    ok(&mut m, r#"json set order:01 .total 1000"#);
    assert_eq!(ok(&mut m, "wait 1"), "OK: 1 replica(s) acknowledged.");
    ok(&mut r, "workspace switch sales");
    assert_eq!(
        ok(&mut r, "get order:99.total and order:00 and order:01.total"),
        "order:99.total: 99 [number]\norder:00: (nil)\norder:01.total: 1000 [number]"
    );
    ok(&mut r, "workspace switch default");
    assert_eq!(ok(&mut r, "get config:mode"), "config:mode: maintenance [string]");

    // Every replicated workspace is read-only on the replica.
    ok(&mut r, "workspace switch sales");
    assert!(fails(&mut r, "set nope 1").contains("READONLY"));
    ok(&mut r, "workspace switch default");
    assert!(fails(&mut r, "set nope 1").contains("READONLY"));

    // Users, stored scripts and triggers follow; triggers fire on the master
    // only, their writes reaching the replica with the rest.
    let script = dir_m.path().join("count.lua");
    std::fs::write(&script, "kiva.incr('fired', 1)\n").unwrap();
    ok(&mut m, "workspace switch default");
    ok(&mut m, &format!("script load \"{}\" as \"count\"", script.display()));
    ok(&mut m, r#"create trigger on_evt on set "evt:*" run "count""#);
    ok(&mut m, r#"create user "amani" identified by "secret" role editor"#);
    ok(&mut m, "set evt:1 1");
    wait_for("the trigger on the master", || master.engine("default").unwrap().contains("fired"));
    assert_eq!(ok(&mut m, "wait 1"), "OK: 1 replica(s) acknowledged.");
    assert!(ok(&mut r, "users").contains("amani"));
    assert!(ok(&mut r, "triggers").contains("on_evt"));
    assert!(ok(&mut r, "script list").contains("count"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(ok(&mut r, "get fired"), "fired: 1 [number]", "not run again on the replica");

    // Workspaces created and removed on the master follow.
    ok(&mut m, "workspace switch default");
    ok(&mut m, "workspace create hr");
    wait_for("the new workspace", || ok(&mut r, "workspace list").contains("hr"));
    ok(&mut m, "workspace switch hr");
    ok(&mut m, "set staff:1 \"Amani\"");
    ok(&mut m, "workspace switch default");
    assert_eq!(ok(&mut m, "wait 1"), "OK: 1 replica(s) acknowledged.");
    assert!(replica.engine("hr").unwrap().contains("staff:1"));
    ok(&mut m, "workspace remove sales");
    wait_for("the removal", || !ok(&mut r, "workspace list").contains("sales"));

    // A reset of one workspace leaves the others alone.
    ok(&mut m, "workspace switch hr");
    ok(&mut m, "del all keys");
    ok(&mut m, "workspace switch default");
    assert_eq!(ok(&mut m, "wait 1"), "OK: 1 replica(s) acknowledged.");
    assert_eq!(replica.engine("hr").unwrap().len(), 0);
    assert!(replica.engine("default").unwrap().contains("config:mode"));

    // A replica that does not announce every workspace is refused.
    let mut old = connect(port_m);
    assert_eq!(call(&mut old, CMD_HELLO, &[]).0, ST_OK);
    let (code, msg) = call(&mut old, CMD_REPL_SYNC, b"127.0.0.1:1 node=old");
    assert_eq!(code, ST_ERR);
    assert!(msg.contains("replicates every workspace"), "{msg}");
}

#[test]
fn the_change_journal_over_rest_with_a_wait() {
    let dir = TempDir::new("http_changes");
    let inst = instance(&dir);
    let port = http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let mut admin = Session::new(inst.clone(), true, None);
    let get = |p: &str, auth: Option<&str>| http_req(port, "GET", p, None, auth);
    assert_eq!(get("/v1/changes", None).status, 404, "no journal yet");
    ok(&mut admin, "changes enable");
    ok(&mut admin, r#"set user:1 {"name": "Ana"}"#);
    ok(&mut admin, "del user:1");

    let r = get("/v1/changes?from=beginning", None).json();
    let ops: Vec<&str> = r["changes"].as_array().unwrap().iter().map(|c| c["op"].as_str().unwrap()).collect();
    assert_eq!(ops, ["set", "delete"]);
    assert_eq!((&r["changes"][0]["value"], &r["changes"][0]["created"]), (&json!({"name": "Ana"}), &json!(true)));
    let cursor = r["cursor"].as_str().unwrap().to_string();
    assert_eq!(r["changes"][1]["cursor"], cursor);

    // Nothing after the cursor yet: the request waits for the next change.
    let writer = {
        let inst = inst.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            ok(&mut Session::new(inst, true, None), "set late 1");
        })
    };
    let started = Instant::now();
    let r = get(&format!("/v1/changes?cursor={cursor}&wait=10000"), None).json();
    writer.join().unwrap();
    assert_eq!(r["changes"][0]["key"], "late", "{r}");
    assert!(started.elapsed() >= Duration::from_millis(250) && started.elapsed() < Duration::from_secs(8));
    // A wait with nothing coming ends at its time, with no change.
    let r = get(&format!("/v1/changes?cursor={}&wait=200", r["cursor"].as_str().unwrap()), None).json();
    assert_eq!(r["changes"], json!([]));
    let r = get("/v1/changes?cursor=00", None);
    assert_eq!(r.status, 400);
    assert!(r.body.contains("CURSOR_INVALID"), "{}", r.body);

    // With authentication: read access to the workspace, checked at every request.
    ok(&mut admin, r#"create user "feed" identified by "pw" grant read on default"#);
    ok(&mut admin, "config set auth_required true");
    let auth = Some("feed:pw");
    assert_eq!(get("/v1/changes?from=beginning&consumer=search", auth).status, 200);
    ok(&mut admin, r#"revoke read on default from "feed""#);
    assert_eq!(get("/v1/changes?from=beginning", auth).status, 403);
}

#[test]
fn a_killed_process_keeps_its_changes() {
    let dir = TempDir::new("kill_changes");
    let mut child = kivadb(dir.path()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdin.write_all(b"changes enable\n").unwrap();
    for i in 0..200 {
        stdin.write_all(format!("set k{i:03} \"value {i}\"\n").as_bytes()).unwrap();
    }
    stdin.write_all(b"begin\nset in_txn 1\nping\n").unwrap();
    stdin.flush().unwrap();
    while line.trim_end() != "PONG" {
        line.clear();
        assert!(stdout.read_line(&mut line).unwrap() > 0);
    }
    child.kill().unwrap();
    child.wait().unwrap();

    let out = kivadb(dir.path()).args(["-e", "changes read from beginning limit 1000"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    let changes: Vec<&str> = text.lines().filter(|l| l.starts_with('#')).collect();
    assert_eq!(changes.len(), 200, "{text}");
    assert!(changes[0].starts_with("#1 ") && changes[0].ends_with("insert k000 (string) \"value 0\""), "{text}");
    assert!(!text.contains("in_txn"), "the open transaction never happened");
}

#[test]
fn a_replica_journals_what_its_master_sends() {
    let (dir_m, dir_r) = (TempDir::new("cdc-master"), TempDir::new("cdc-replica"));
    let (master, replica) = (instance(&dir_m), instance(&dir_r));
    let port_m = server::start_tcp(&master, "127.0.0.1", 0).unwrap();
    server::start_tcp(&replica, "127.0.0.1", 0).unwrap();
    let mut m = Session::new(master.clone(), true, None);
    let mut r = Session::new(replica.clone(), true, None);
    ok(&mut m, "changes enable");
    ok(&mut m, "set before 1");
    ok(&mut r, "changes enable");
    ok(&mut r, &format!("replica of 127.0.0.1 {port_m}"));
    wait_for("the initial snapshot", || replica.repl.link().status == "connected");
    ok(&mut m, r#"set live "one""#);
    ok(&mut m, "begin");
    ok(&mut m, "set t1 1 and t2 2");
    ok(&mut m, "commit");
    ok(&mut m, "del before");
    wait_for("the live writes", || !replica.engine("default").unwrap().contains("before"));
    let on_replica = ok(&mut r, "changes read from beginning");
    for what in ["live", "t1", "t2", "delete before"] {
        assert!(on_replica.contains(what), "{what} missing on the replica:\n{on_replica}");
    }
    // Each node numbers its own journal: a cursor of the master means nothing on the replica.
    let cursor_m =
        ok(&mut m, "changes read from now").lines().last().unwrap().split_whitespace().nth(1).unwrap().to_string();
    assert!(fails(&mut r, &format!("changes read from {cursor_m}")).contains("another change journal"));
}
