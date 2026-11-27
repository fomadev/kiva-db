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
    provision_with(dir, max_clients, &[]);
}

fn provision_with(dir: &TempDir, max_clients: usize, settings: &[&str]) {
    let inst = instance(dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "modify user kiva password test-pass");
    ok(&mut admin, "create user editor identified by test-pass role editor");
    ok(&mut admin, "create user viewer identified by test-pass role viewer");
    ok(&mut admin, &format!("config set server.max_clients {max_clients}"));
    for command in settings {
        ok(&mut admin, command);
    }
    ok(&mut admin, "config set semantic.cache.enabled true");
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
        format!("Basic {}", b64_encode(format!("{user}:test-pass").as_bytes()))
    };
    authenticated_http(reader, &auth, method, path, body)
}

fn authenticated_http(
    reader: &mut BufReader<TcpStream>,
    auth: &str,
    method: &str,
    path: &str,
    body: Json,
) -> (u16, Json) {
    let auth = if auth.is_empty() { String::new() } else { format!("Authorization: {auth}\r\n") };
    let body = if body.is_null() { String::new() } else { body.to_string() };
    write!(
        reader.get_mut(),
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    http_reply(reader)
}

fn login(reader: &mut BufReader<TcpStream>, user: &str) -> String {
    let (status, body) = http(reader, user, "POST", "/v2/auth/login", Json::Null);
    assert_eq!(status, 201, "{body}");
    let token = body["access_token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert_eq!(body["session"]["account"], user);
    format!("Bearer {token}")
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
    let vector = json!({"op":"similar","target":"doc:*.embedding","vector":[1,0]});
    let api = format!("{A}/api");
    native.exec(r#"set doc:1 {"embedding":[1,0]}"#).unwrap();
    native.exec("create vector index on doc:* (embedding)").unwrap();
    for _ in 0..2 {
        let (status, result) = http(&mut rest, "kiva", "POST", &api, vector.clone());
        assert_eq!(status, 200);
        assert_eq!(result["results"][0]["key"], "doc:1");
        assert_eq!(result["cached"], Json::Null);
    }
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
    assert_eq!(http(&mut rest, "kiva", "POST", &api, vector.clone()).1["results"], json!([]));
    let large = Value::Doc(json!({"embedding":[1,0], "payload":"x".repeat(1536 * 1024)}));
    let mut recreated = native.collection("Fordima", "app", "users").unwrap();
    for i in 0..3 {
        recreated.set(&format!("doc:{i}"), &large, 0).unwrap();
    }
    assert_eq!(http(&mut rest, "kiva", "POST", &api, vector).0, 422);

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
    let token = login(&mut rest, "kiva");
    assert_eq!(authenticated_http(&mut rest, &token, "GET", "/v2/auth/session", Json::Null).0, 200);
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
    assert_eq!(authenticated_http(&mut rest, &token, "GET", "/v2/auth/session", Json::Null).0, 401);
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

#[test]
fn http_sessions_follow_live_account_and_collection_identity() {
    let dir = TempDir::new("hierarchy_http_identity");
    provision(&dir, 128);
    let server = Server::open(&dir);
    let mut rest = connect(server.rest);
    let mut native = server.native();
    let key_a = format!("{A}/keys/test");
    let key_b = format!("{B}/keys/test");
    let bad = format!("Basic {}", b64_encode(b"editor:wrong-password"));
    assert_eq!(authenticated_http(&mut rest, &bad, "POST", "/v2/auth/login", Json::Null).0, 401);
    assert_eq!(authenticated_http(&mut rest, "Bearer malformed", "GET", &key_a, Json::Null).0, 401);

    let editor = login(&mut rest, "editor");
    assert_eq!(authenticated_http(&mut rest, &editor, "GET", "/v2/auth/session", Json::Null).1["account"], "editor");
    assert_eq!(authenticated_http(&mut rest, &editor, "GET", "/v2/auth/metrics", Json::Null).0, 403);
    assert_eq!(authenticated_http(&mut rest, &editor, "GET", &key_b, Json::Null).0, 404);
    assert_eq!(authenticated_http(&mut rest, &editor, "PUT", &key_a, json!({"type":"int","value":7})).0, 200);
    let viewer = login(&mut rest, "viewer");
    assert!(matches!(
        authenticated_http(&mut rest, &viewer, "PUT", &key_a, json!({"type":"int","value":8})).0,
        403 | 404
    ));
    native.exec("hierarchy revoke editor write collection Fordima.app.users").unwrap();
    assert_eq!(authenticated_http(&mut rest, &editor, "GET", &key_a, Json::Null).0, 404);
    assert_eq!(authenticated_http(&mut rest, &editor, "GET", "/v2/auth/session", Json::Null).0, 200);
    native.exec("hierarchy grant editor write collection Fordima.app.users").unwrap();

    // Replace the verifier persisted by account administration while the listener runs.
    let mut user = kivadb::auth::get_user(&server.inst.system, "editor").unwrap().unwrap();
    user.set_password("rotated-password").unwrap();
    kivadb::auth::save_user(&server.inst.system, &user).unwrap();
    assert_eq!(authenticated_http(&mut rest, &editor, "GET", &key_a, Json::Null).0, 401);
    let rotated = format!("Basic {}", b64_encode(b"editor:rotated-password"));
    let (status, body) = authenticated_http(&mut rest, &rotated, "POST", "/v2/auth/login", Json::Null);
    assert_eq!(status, 201);
    let renewed = format!("Bearer {}", body["access_token"].as_str().unwrap());
    native.exec("drop user editor").unwrap();
    native.exec("create user editor identified by rotated-password role editor").unwrap();
    assert_eq!(authenticated_http(&mut rest, &renewed, "GET", &key_a, Json::Null).0, 401);
    let (status, body) = authenticated_http(&mut rest, &rotated, "POST", "/v2/auth/login", Json::Null);
    assert_eq!(status, 201);
    let recreated = format!("Bearer {}", body["access_token"].as_str().unwrap());
    assert_eq!(authenticated_http(&mut rest, &recreated, "GET", &key_a, Json::Null).0, 404);

    native.exec("hierarchy grant viewer read collection Fordima.app.users").unwrap();
    assert_eq!(http(&mut rest, "kiva", "DELETE", A, Json::Null).0, 200);
    let parent = A.rsplit_once('/').unwrap().0;
    assert_eq!(http(&mut rest, "kiva", "POST", parent, json!({"name":"users"})).0, 201);
    assert_eq!(authenticated_http(&mut rest, &viewer, "GET", &key_a, Json::Null).0, 404);

    let audit = std::fs::read_to_string(dir.path().join("audit.log")).unwrap();
    for secret in [&editor, &viewer, &renewed, &recreated] {
        assert!(!audit.contains(secret.strip_prefix("Bearer ").unwrap()));
    }
    assert!(!audit.contains("rotated-password"));
}

#[test]
fn http_sessions_are_bounded_and_revoked_across_connections() {
    let dir = TempDir::new("hierarchy_http_revoke");
    provision_with(
        &dir,
        128,
        &[
            "config set http.session_capacity 2",
            "config set http.sessions_per_account 1",
            "config set server.native_metrics true",
        ],
    );
    let server = Server::open(&dir);
    let mut rest = connect(server.rest);
    let mut second = connect(server.rest);
    let admin = login(&mut rest, "kiva");
    assert_eq!(http(&mut rest, "kiva", "POST", "/v2/auth/login", Json::Null).0, 429);
    let editor = login(&mut rest, "editor");
    assert_eq!(http(&mut rest, "viewer", "POST", "/v2/auth/login", Json::Null).0, 429);
    let write = |stream: &mut BufReader<TcpStream>, path: &str, value| {
        authenticated_http(
            &mut *stream,
            &admin,
            "PUT",
            &format!("{path}/keys/scoped"),
            json!({"type":"int","value":value}),
        )
    };
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            write(&mut rest, A, 1)
        });
        barrier.wait();
        assert_eq!(write(&mut second, B, 2).0, 200);
        assert_eq!(first.join().unwrap().0, 200);
    });
    let mut native = server.native();
    assert_eq!(
        native.collection("Fordima", "app", "users").unwrap().get("scoped").unwrap(),
        Some(Value::Num("1".into()))
    );
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let server = &server;
            scope.spawn(move || {
                let mut client = connect(server.rest);
                let wrong = format!("Basic {}", b64_encode(b"viewer:wrong-attempt"));
                assert_eq!(authenticated_http(&mut client, &wrong, "POST", "/v2/auth/login", Json::Null).0, 401);
            });
        }
    });
    let stats = authenticated_http(&mut rest, &admin, "GET", "/v2/auth/metrics", Json::Null).1;
    let timings = stats["timings"].as_array().unwrap();
    assert!(timings.iter().any(|row| row["operation"] == "http_basic" && row["stage"] == "password"));
    assert!(timings.iter().any(|row| row["operation"] == "http_bearer" && row["stage"] == "token"));
    assert!(!timings.iter().any(|row| row["operation"] == "http_bearer" && row["stage"] == "password"));
    assert_eq!(stats["sessions"]["active"], 2);
    assert_eq!(stats["sessions"]["refused"], 2);

    assert_eq!(authenticated_http(&mut second, &editor, "POST", "/v2/auth/logout", Json::Null).0, 200);
    let editor_batch = format!("{A}/batch");
    assert_eq!(
        authenticated_http(
            &mut rest,
            &editor,
            "POST",
            &editor_batch,
            json!({"operations":[{"op":"put","key":"scoped","type":"int","value":3}]})
        )
        .0,
        401
    );
    assert_eq!(native.get("scoped").unwrap(), Some(Value::Num("1".into())));
    let replacement = login(&mut rest, "editor");
    assert_eq!(
        authenticated_http(&mut rest, &admin, "POST", "/v2/auth/revoke", json!({"account":"editor"})).1["revoked"],
        1
    );
    assert_eq!(authenticated_http(&mut second, &replacement, "GET", "/v2/auth/session", Json::Null).0, 401);
}
