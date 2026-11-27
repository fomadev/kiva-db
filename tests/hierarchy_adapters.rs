// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.

mod common;

use common::{instance, ok, TempDir};
use kivadb::{client::Client, session::Session, util::b64_encode, Instance, Options, Value};
use serde_json::{json, Value as Json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

const A: &str = "/v2/workspaces/Fordima/projects/app/collections/users";
const B: &str = "/v2/workspaces/Trading/projects/app/collections/users";

fn provision(dir: &TempDir, max_clients: usize) {
    let inst = instance(dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "modify user kiva password test-pass");
    ok(&mut admin, "create user editor identified by test-pass role editor");
    ok(&mut admin, "create user viewer identified by test-pass role viewer");
    ok(&mut admin, &format!("config set server.max_clients {max_clients}"));
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
    inst.shutdown();
}

struct Server {
    inst: Arc<Instance>,
    tcp: u16,
    rest: u16,
    resp: u16,
}

impl Server {
    fn open(dir: &TempDir) -> Self {
        let inst = Instance::open_hierarchical(dir.path(), Options::default()).unwrap();
        let tcp = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
        let rest = kivadb::http::start_hierarchical_http(&inst, "127.0.0.1", 0).unwrap();
        let resp = kivadb::resp::start_hierarchical_resp(&inst, "127.0.0.1", 0).unwrap();
        Self { inst, tcp, rest, resp }
    }

    fn native(&self) -> Client {
        let mut client = Client::connect(("127.0.0.1", self.tcp)).unwrap();
        client.negotiate_hierarchy().unwrap();
        client.auth("kiva", "test-pass").unwrap();
        client
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.inst.shutdown();
        for port in [self.tcp, self.rest, self.resp] {
            let _ = TcpStream::connect(("127.0.0.1", port));
        }
    }
}

fn connect(port: u16) -> BufReader<TcpStream> {
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    BufReader::new(stream)
}

fn line(reader: &mut impl BufRead) -> String {
    let mut line = String::new();
    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
    line.trim_end_matches("\r\n").into()
}

fn http_reply(reader: &mut impl BufRead) -> (u16, Json) {
    let status = line(reader).split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut size = 0;
    loop {
        let header = line(reader);
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            size = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; size];
    reader.read_exact(&mut body).unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

fn http(reader: &mut BufReader<TcpStream>, user: &str, method: &str, path: &str, body: Json) -> (u16, Json) {
    let auth = if user.is_empty() {
        String::new()
    } else {
        format!("Authorization: Basic {}\r\n", b64_encode(format!("{user}:test-pass").as_bytes()))
    };
    let body = if body.is_null() { String::new() } else { body.to_string() };
    write!(
        reader.get_mut(),
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    http_reply(reader)
}

fn send_resp(stream: &mut impl Write, args: &[&str]) {
    let mut frame = format!("*{}\r\n", args.len());
    for arg in args {
        frame.push_str(&format!("${}\r\n{arg}\r\n", arg.len()));
    }
    stream.write_all(frame.as_bytes()).unwrap();
}

fn resp(reader: &mut BufReader<TcpStream>, args: &[&str]) -> String {
    send_resp(reader.get_mut(), args);
    line(reader)
}

#[test]
fn adapters_share_collection_data_permissions_and_stable_identities() {
    let dir = TempDir::new("hierarchy_adapters");
    provision(&dir, 128);
    let server = Server::open(&dir);
    let mut native = server.native();
    let mut rest = connect(server.rest);
    let mut redis = connect(server.resp);
    let key = format!("{A}/keys/shared");
    assert_eq!(http(&mut rest, "", "GET", "/health", Json::Null).0, 200);
    assert_eq!(http(&mut rest, "", "GET", &key, Json::Null).0, 401);
    assert!(resp(&mut redis, &["GET", "shared"]).starts_with("-NOAUTH"));
    assert_eq!(resp(&mut redis, &["AUTH", "editor", "test-pass"]), "+OK");
    assert!(resp(&mut redis, &["GET", "shared"]).starts_with('-'));
    assert!(resp(&mut redis, &["SELECT", "0"]).starts_with('-'));
    assert!(resp(&mut redis, &["FLUSHALL"]).starts_with('-'));
    assert_eq!(resp(&mut redis, &["KIVA.USE", "Fordima.app.users"]), "+OK");

    native.collection("Fordima", "app", "users").unwrap().set("shared", &Value::Str("native".into()), 0).unwrap();
    let (status, body) = http(&mut rest, "editor", "GET", &key, Json::Null);
    assert_eq!((status, &body["value"]), (200, &json!("native")));
    assert_eq!(http(&mut rest, "editor", "PUT", &key, json!({"type":"string","value":"rest"})).0, 200);
    assert_eq!(resp(&mut redis, &["GET", "shared"]), "$4");
    assert_eq!(line(&mut redis), "rest");
    assert_eq!(http(&mut rest, "kiva", "GET", &format!("{B}/keys/shared"), Json::Null).0, 404);
    let mut redis_other = connect(server.resp);
    assert_eq!(resp(&mut redis_other, &["AUTH", "kiva", "test-pass"]), "+OK");
    assert_eq!(resp(&mut redis_other, &["KIVA.USE", "Trading.app.users"]), "+OK");
    assert_eq!(resp(&mut redis_other, &["GET", "shared"]), "$-1");
    assert_eq!(resp(&mut redis_other, &["SET", "shared", "other"]), "+OK");
    assert_eq!(http(&mut rest, "kiva", "GET", &format!("{B}/keys/shared"), Json::Null).1["value"], "other");
    assert_eq!(
        http(&mut rest, "editor", "GET", B, Json::Null),
        http(&mut rest, "editor", "GET", &B.replace("Trading", "Missing"), Json::Null)
    );
    assert!(matches!(http(&mut rest, "viewer", "PUT", &key, json!({"type":"string","value":"bad"})).0, 403 | 404));

    send_resp(redis.get_mut(), &["SET", "shared", "resp", "EX", "60"]);
    send_resp(redis.get_mut(), &["TTL", "shared"]);
    assert_eq!(line(&mut redis), "+OK");
    let ttl = line(&mut redis);
    assert!((1..=60).contains(&ttl[1..].parse::<u64>().unwrap()));
    assert_eq!(native.get("shared").unwrap(), Some(Value::Str("resp".into())));
    assert_eq!(resp(&mut redis, &["MULTI"]), "+OK");
    assert_eq!(resp(&mut redis, &["SET", "revoked", "bad"]), "+QUEUED");
    native.exec("hierarchy revoke editor write collection Fordima.app.users").unwrap();
    assert!(resp(&mut redis, &["EXEC"]).starts_with("-NOPERM"));
    assert_eq!(http(&mut rest, "editor", "GET", &key, Json::Null).0, 404);
    assert_eq!(native.get("revoked").unwrap(), None);

    assert_eq!(resp(&mut redis, &["AUTH", "kiva", "test-pass"]), "+OK");
    assert_eq!(resp(&mut redis, &["KIVA.USE", "Fordima.app.users"]), "+OK");
    assert_eq!(http(&mut rest, "kiva", "PUT", A, json!({"name":"renamed"})).0, 200);
    assert_eq!(resp(&mut redis, &["GET", "shared"]), "$4");
    assert_eq!(line(&mut redis), "resp");
    let renamed = A.replace("/users", "/renamed");
    assert_eq!(http(&mut rest, "kiva", "DELETE", &renamed, Json::Null).0, 200);
    let parent = A.rsplit_once('/').unwrap().0;
    assert_eq!(http(&mut rest, "kiva", "POST", parent, json!({"name":"users"})).0, 201);
    assert!(resp(&mut redis, &["GET", "shared"]).starts_with("-NOPERM"));
    assert_eq!(http(&mut rest, "kiva", "GET", &key, Json::Null).0, 404);

    let parent = "/v2/workspaces/Trading";
    assert_eq!(http(&mut rest, "kiva", "DELETE", parent, Json::Null).0, 400);
    assert_eq!(http(&mut rest, "kiva", "DELETE", parent, json!({"cascade":true})).0, 200);
    assert!(resp(&mut redis_other, &["GET", "shared"]).starts_with("-NOPERM"));
    assert_eq!(http(&mut rest, "kiva", "DELETE", "/v2/workspaces/default", json!({"cascade":true})).0, 400);
}

#[test]
fn requests_and_transactions_are_bounded_and_fail_safely() {
    let dir = TempDir::new("hierarchy_adapter_limits");
    provision(&dir, 128);
    let server = Server::open(&dir);
    let mut rest = connect(server.rest);
    let mut redis = connect(server.resp);
    let batch = format!("{A}/batch");
    let key = format!("{A}/keys/staged");
    assert_eq!(
        http(
            &mut rest,
            "kiva",
            "POST",
            &batch,
            json!({"operations":[{"op":"put","key":"staged","type":"int","value":13},{"op":"put"}]})
        )
        .0,
        400
    );
    assert_eq!(http(&mut rest, "kiva", "GET", &key, Json::Null).0, 404);
    assert_eq!(
        http(
            &mut rest,
            "kiva",
            "POST",
            &batch,
            json!({"operations":[{"op":"put","key":"staged","type":"int","value":13}]})
        )
        .0,
        200
    );
    assert_eq!(http(&mut rest, "kiva", "GET", &key, Json::Null).1["value"], 13);
    assert_eq!(http(&mut rest, "kiva", "POST", &format!("{A}/api"), json!({"op":"begin"})).0, 400);
    assert_eq!(http(&mut connect(server.rest), "kiva", "GET", &format!("{A}/keys/%2f"), Json::Null).0, 400);
    let mut oversized = connect(server.rest);
    oversized.get_mut().write_all(b"PUT /health HTTP/1.1\r\nHost: local\r\nContent-Length: 3000000\r\n\r\n").unwrap();
    assert_eq!(http_reply(&mut oversized).0, 413);

    let mut ambiguous = connect(server.rest);
    ambiguous
        .get_mut()
        .write_all(b"POST /health HTTP/1.1\r\nHost: local\r\nContent-Length: 0\r\nContent-Length: 1\r\n\r\n")
        .unwrap();
    assert_eq!(http_reply(&mut ambiguous).0, 400);

    assert_eq!(resp(&mut redis, &["AUTH", "kiva", "test-pass"]), "+OK");
    assert_eq!(resp(&mut redis, &["KIVA.USE", "Fordima.app.users"]), "+OK");
    assert_eq!(resp(&mut redis, &["MULTI"]), "+OK");
    assert_eq!(resp(&mut redis, &["DEL", "staged"]), "+QUEUED");
    assert_eq!(resp(&mut redis, &["SET", "staged", "new"]), "+QUEUED");
    assert_eq!(resp(&mut redis, &["EXEC"]), "*2");
    assert_eq!(line(&mut redis), ":1");
    assert_eq!(line(&mut redis), "+OK");
    assert_eq!(resp(&mut redis, &["MULTI"]), "+OK");
    assert_eq!(resp(&mut redis, &["DEL", "staged"]), "+QUEUED");
    drop(redis);
    assert_eq!(http(&mut rest, "kiva", "GET", &key, Json::Null).1["value"], "new");

    let mut malformed = connect(server.resp);
    malformed.get_mut().write_all(b"*1\r\n$9999999\r\n").unwrap();
    assert!(line(&mut malformed).starts_with('-'));
}

#[test]
fn adapters_accept_pinned_tls_and_share_concurrent_durable_writes() {
    let dir = TempDir::new("hierarchy_adapter_tls");
    provision(&dir, 128);
    let server = Server::open(&dir);
    let fingerprint = server.inst.tls().unwrap().fingerprint.clone();
    let tls = |port| {
        let conn =
            kivadb::tls::connect(&format!("127.0.0.1:{port}"), Some(&fingerprint), Duration::from_secs(5)).unwrap();
        conn.stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        BufReader::new(conn.stream)
    };
    let mut rest = tls(server.rest);
    let mut redis = tls(server.resp);
    let mut native = server.native();
    assert_eq!(resp(&mut redis, &["AUTH", "kiva", "test-pass"]), "+OK");
    assert_eq!(resp(&mut redis, &["KIVA.USE", "Fordima.app.users"]), "+OK");
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|threads| {
        let write = threads.spawn(|| {
            barrier.wait();
            http(&mut rest, "kiva", "PUT", &format!("{A}/keys/binary"), json!({"type":"bytes","value":"AAH/"}))
        });
        barrier.wait();
        assert_eq!(resp(&mut redis, &["SET", "durable", "yes"]), "+OK");
        assert_eq!(write.join().unwrap().0, 200);
    });
    let mut collection = native.collection("Fordima", "app", "users").unwrap();
    assert_eq!(collection.get("binary").unwrap(), Some(Value::Bytes(vec![0, 1, 255])));
    assert_eq!(collection.get("durable").unwrap(), Some(Value::Str("yes".into())));
    drop((rest, redis, native));
    let instance = Arc::downgrade(&server.inst);
    drop(server);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while instance.strong_count() != 0 {
        assert!(std::time::Instant::now() < deadline, "listener failed to shut down");
        std::thread::sleep(Duration::from_millis(10));
    }
    let reopened = Server::open(&dir);
    let mut rest = connect(reopened.rest);
    assert_eq!(http(&mut rest, "kiva", "GET", &format!("{A}/keys/durable"), Json::Null).1["value"], "yes");
}

#[test]
fn adapters_share_the_connection_limit_with_native_tcp() {
    let dir = TempDir::new("hierarchy_adapter_saturation");
    provision(&dir, 2);
    let server = Server::open(&dir);
    let _native = server.native();
    let mut rest = connect(server.rest);
    assert_eq!(http(&mut rest, "", "GET", "/health", Json::Null).0, 200);
    let mut refused = connect(server.resp);
    assert!(line(&mut refused).starts_with("-BUSYERR "));
}
