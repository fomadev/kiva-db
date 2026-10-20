// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! REST API, Prometheus metrics, interactive documentation and WebSocket
//! notifications, on a small dependency-free HTTP/1.1 server.

use crate::error::{KivaError, Result};
use crate::exec::validate_key_name;
use crate::instance::{Instance, VERSION};
use crate::session::{Need, Session};
use crate::util::{b64_decode, b64_encode, now_secs, url_decode};
use crate::value::Value;
use serde_json::{json, Value as Json};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 64 * 1024 * 1024;

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    extra: Vec<(String, String)>,
}

impl Response {
    fn json(status: u16, j: Json) -> Response {
        Response { status, content_type: "application/json", body: j.to_string().into_bytes(), extra: Vec::new() }
    }

    fn ok(mut j: Json) -> Response {
        j["status"] = json!("ok");
        j["timestamp"] = json!(now_secs());
        Response::json(200, j)
    }

    fn error(status: u16, msg: impl Into<String>) -> Response {
        Response::json(status, json!({ "status": "error", "error": msg.into(), "timestamp": now_secs() }))
    }

    fn text(content_type: &'static str, body: String) -> Response {
        Response { status: 200, content_type, body: body.into_bytes(), extra: Vec::new() }
    }
}

impl From<KivaError> for Response {
    fn from(e: KivaError) -> Response {
        let status = match &e {
            KivaError::Denied(_) => 403,
            KivaError::NotFound(_) => 404,
            KivaError::Exists(_) | KivaError::Conflict(_) | KivaError::ReadOnly(_) => 409,
            KivaError::Invalid(_) | KivaError::Type(_) | KivaError::Script(_) => 400,
            _ => 500,
        };
        Response::error(status, e.to_string())
    }
}

type RateTable = Arc<Mutex<HashMap<IpAddr, (u64, u64)>>>;

/// Starts the HTTP listener and returns the bound port.
pub fn start_http(inst: &Arc<Instance>, bind: &str, port: u16) -> Result<u16> {
    let listener = TcpListener::bind((bind, port))
        .map_err(|e| KivaError::Other(format!("cannot listen on {bind}:{port}: {e}")))?;
    let port = listener.local_addr()?.port();
    let inst = inst.clone();
    let rates: RateTable = Arc::new(Mutex::new(HashMap::new()));
    std::thread::Builder::new().name("kiva-http".into()).spawn(move || {
        for conn in listener.incoming() {
            if inst.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = conn else { continue };
            let (inst, rates) = (inst.clone(), rates.clone());
            let _ = std::thread::Builder::new().name("kiva-http-conn".into()).spawn(move || {
                let refusal = |_: u8| {
                    let body = "This server requires HTTPS from other machines.";
                    format!(
                        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .into_bytes()
                };
                if let Some((stream, peer)) = crate::tls::accept(&inst, stream, &refusal) {
                    handle(&inst, stream, peer, &rates);
                }
            });
        }
    })?;
    Ok(port)
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADER {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_ascii_uppercase();
    let target = first.next()?.to_string();
    let mut headers = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if len > MAX_BODY {
        return None;
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < len {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(len);
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    let mut query = HashMap::new();
    for pair in qs.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(url_decode(k), url_decode(v));
    }
    Some(Request { method, path: path.to_string(), query, headers, body })
}

fn write_response(stream: &mut TcpStream, resp: &Response, cors: bool) {
    let reason = match resp.status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    };
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nServer: KivaDB/{VERSION}\r\n",
        resp.status,
        resp.content_type,
        resp.body.len()
    );
    if cors {
        head.push_str("Access-Control-Allow-Origin: *\r\n");
        head.push_str("Access-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS\r\n");
        head.push_str("Access-Control-Allow-Headers: Authorization, Content-Type\r\n");
    }
    for (k, v) in &resp.extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&resp.body);
    let _ = stream.flush();
}

fn handle(inst: &Arc<Instance>, mut stream: TcpStream, peer: std::net::SocketAddr, rates: &RateTable) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Some(req) = read_request(&mut stream) else { return };
    inst.metrics.http_requests.fetch_add(1, Ordering::Relaxed);
    let (cors, limit) = {
        let c = inst.cfg();
        (c.get_bool("http", "cors"), c.get_u64("http", "rate_limit"))
    };
    if limit > 0 {
        let minute = now_secs() as u64 / 60;
        let mut table = rates.lock().unwrap_or_else(|e| e.into_inner());
        if table.len() > 10_000 {
            table.retain(|_, (m, _)| *m == minute);
        }
        let slot = table.entry(peer.ip()).or_insert((minute, 0));
        if slot.0 != minute {
            *slot = (minute, 0);
        }
        slot.1 += 1;
        if slot.1 > limit {
            drop(table);
            let mut resp = Response::error(429, format!("rate limit of {limit} requests per minute exceeded"));
            resp.extra.push(("Retry-After".into(), "60".into()));
            write_response(&mut stream, &resp, cors);
            return;
        }
    }
    if req.method == "OPTIONS" {
        let resp = Response { status: 204, content_type: "text/plain", body: Vec::new(), extra: Vec::new() };
        write_response(&mut stream, &resp, cors);
        return;
    }
    // Pages that need no credentials.
    let open = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/health") => Some(Response::ok(json!({ "version": VERSION }))),
        ("GET", "/docs") => Some(Response::text("text/html; charset=utf-8", DOCS_HTML.to_string())),
        ("GET", "/openapi.json") => Some(Response::json(200, openapi())),
        _ => None,
    };
    if let Some(resp) = open {
        write_response(&mut stream, &resp, cors);
        return;
    }

    let (tx, rx) = mpsc::channel::<String>();
    let mut sess = Session::new(inst.clone(), peer.ip().is_loopback(), Some(tx));
    if sess.user.is_none() {
        let creds = req
            .headers
            .get("authorization")
            .and_then(|h| h.strip_prefix("Basic "))
            .and_then(|b| b64_decode(b.trim()))
            .and_then(|raw| String::from_utf8(raw).ok())
            .and_then(|s| s.split_once(':').map(|(u, p)| (u.to_string(), p.to_string())));
        let logged = creds.is_some_and(|(u, p)| sess.login(&u, &p).is_ok());
        if !logged {
            let mut resp = Response::error(401, "authentication required (HTTP Basic)");
            resp.extra.push(("WWW-Authenticate".into(), "Basic realm=\"KivaDB\"".into()));
            write_response(&mut stream, &resp, cors);
            return;
        }
    }
    if let Some(ws) = req.query.get("workspace") {
        if let Err(e) = sess.switch_workspace(ws) {
            write_response(&mut stream, &Response::from(e), cors);
            return;
        }
    }
    if req.path == "/ws" {
        websocket(sess, rx, stream, &req);
        return;
    }
    if req.path == "/v1/stream" && req.method == "GET" {
        server_sent_events(sess, rx, stream, &req, cors);
        return;
    }
    let resp = route(&mut sess, &req).unwrap_or_else(Response::from);
    write_response(&mut stream, &resp, cors);
}

fn value_json(key: &str, v: &Value, expires_at: i64) -> Json {
    let ttl = if expires_at > 0 { json!((expires_at - now_secs()).max(0)) } else { Json::Null };
    let data = match v {
        Value::Bytes(b) => Json::String(b64_encode(b)),
        other => other.to_json(),
    };
    json!({ "key": key, "type": v.ty().name(), "data": data, "ttl": ttl })
}

fn put_key(sess: &mut Session, key: &str, req: &Request) -> Result<Response> {
    validate_key_name(key).map_err(KivaError::Invalid)?;
    let body: Json = match serde_json::from_slice(&req.body) {
        Ok(j) => j,
        // Anything that is not JSON is stored as a plain string.
        Err(_) => Json::String(String::from_utf8_lossy(&req.body).into_owned()),
    };
    // {"value": ..., "ttl": N} is an envelope; any other body is the value itself.
    let envelope = body
        .as_object()
        .is_some_and(|m| m.contains_key("value") && m.keys().all(|k| matches!(k.as_str(), "value" | "ttl" | "key")));
    let (value, mut ttl) =
        if envelope { (body["value"].clone(), body["ttl"].as_i64().unwrap_or(0)) } else { (body, 0) };
    if let Some(t) = req.query.get("ttl").and_then(|t| t.parse::<i64>().ok()) {
        ttl = t;
    }
    let value = Value::from_json(value);
    let ty = value.ty();
    let expires_at = if ttl > 0 { now_secs() + ttl } else { 0 };
    sess.kv_put(key, value, expires_at, false)?;
    Ok(Response::ok(json!({ "key": key, "type": ty.name() })))
}

fn route(sess: &mut Session, req: &Request) -> Result<Response> {
    let method = req.method.as_str();
    let path = req.path.trim_end_matches('/');
    let q = |name: &str| req.query.get(name).map(|s| s.as_str());
    let limit = q("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(0);
    let reverse = matches!(q("reverse"), Some("true" | "1"));

    let key_of = |prefix: &str| path.strip_prefix(prefix).filter(|k| !k.is_empty()).map(url_decode);
    if let Some(key) = key_of("/v1/key/").or_else(|| key_of("/get/")).or_else(|| key_of("/del/")) {
        return match method {
            "GET" => {
                let full = match q("path") {
                    Some(p) => format!("{key}.{}", p.trim_start_matches('.')),
                    None => key.clone(),
                };
                match sess.lookup(&full).map_err(KivaError::Invalid)? {
                    Some((v, source)) => {
                        let exp = sess.kv_get(&source)?.map(|(_, m)| m.expires_at).unwrap_or(0);
                        Ok(Response::ok(value_json(&full, &v, exp)))
                    }
                    None => Ok(Response::error(404, format!("key '{full}' not found"))),
                }
            }
            "POST" | "PUT" => put_key(sess, &key, req),
            "DELETE" => {
                if sess.kv_del(&key)? {
                    Ok(Response::ok(json!({ "key": key, "deleted": true })))
                } else {
                    Ok(Response::error(404, format!("key '{key}' not found")))
                }
            }
            _ => Ok(Response::error(405, "method not allowed")),
        };
    }

    match (method, path) {
        ("POST", "/set") => {
            let body: Json = serde_json::from_slice(&req.body)?;
            let key = body["key"].as_str().ok_or_else(|| KivaError::Invalid("body needs a \"key\"".into()))?;
            put_key(sess, key, req)
        }
        ("GET", "/v1/scan" | "/scan") => {
            sess.check(Need::Read)?;
            let now = now_secs();
            let rows: Vec<Json> = sess
                .engine
                .keys_matching(q("pattern").unwrap_or("*"), limit)
                .into_iter()
                .map(|(k, e)| {
                    json!({
                        "key": k,
                        "type": e.ty.name(),
                        "size": e.raw_size,
                        "modified": e.timestamp,
                        "ttl": if e.expires_at > 0 { json!((e.expires_at - now).max(0)) } else { Json::Null },
                    })
                })
                .collect();
            Ok(Response::ok(json!({ "count": rows.len(), "data": rows })))
        }
        ("GET", "/v1/keys") => {
            sess.check(Need::Read)?;
            let keys: Vec<String> =
                sess.engine.range(q("from"), q("to"), reverse, limit).into_iter().map(|(k, _)| k).collect();
            Ok(Response::ok(json!({ "count": keys.len(), "data": keys })))
        }
        ("GET", "/v1/range") => {
            sess.check(Need::Read)?;
            let mut rows = Vec::new();
            for (k, e) in sess.engine.range(q("from"), q("to"), reverse, limit) {
                if let Some((v, _)) = sess.engine.get(&k)? {
                    rows.push(value_json(&k, &v, e.expires_at));
                }
            }
            Ok(Response::ok(json!({ "count": rows.len(), "data": rows })))
        }
        ("GET", "/v1/stats" | "/stats") => {
            sess.check(Need::Read)?;
            let s = sess.engine.stats();
            let repl = &sess.inst.repl;
            Ok(Response::ok(json!({ "data": {
                "version": VERSION,
                "workspace": sess.ws,
                "keys": s.keys,
                "ttl_keys": s.ttl_keys,
                "storage_bytes": s.file_bytes,
                "live_bytes": s.live_bytes,
                "fragmentation_ratio": s.fragmentation,
                "memory_bytes": s.memory_bytes,
                "compression": s.compression,
                "raw_value_bytes": s.raw_value_bytes,
                "stored_value_bytes": s.stored_value_bytes,
                "startup_ms": s.load_ms,
                "startup_mode": s.load_mode,
                "checksum_errors": s.corrupted_detected,
                "compactions": s.compactions,
                "indexes": s.indexes,
                "role": if repl.is_replica() { "replica" } else { "master" },
                "replicas": repl.replicas().len(),
                "clients": sess.inst.clients.load(Ordering::SeqCst),
                "uptime_seconds": sess.inst.started.elapsed().as_secs(),
            }})))
        }
        ("POST", "/v1/api") => {
            let body: Json = serde_json::from_slice(&req.body)?;
            let mut answer = crate::api::handle(sess, &body)?;
            answer["status"] = json!("ok");
            Ok(Response::json(200, answer))
        }
        ("POST", "/v1/exec") => {
            let body: Json = serde_json::from_slice(&req.body)?;
            let cmd = body["command"].as_str().ok_or_else(|| KivaError::Invalid("body needs a \"command\"".into()))?;
            let out = sess.execute(cmd);
            let lines: Vec<&str> = out.lines.iter().map(|(_, l)| l.as_str()).collect();
            Ok(Response::ok(json!({ "ok": !out.has_err(), "output": lines })))
        }
        ("GET", "/metrics") => {
            sess.check(Need::Read)?;
            Ok(Response::text("text/plain; version=0.0.4", metrics(sess)))
        }
        _ => Ok(Response::error(404, format!("no route for {method} {}", req.path))),
    }
}

/// Prometheus text exposition.
fn metrics(sess: &Session) -> String {
    let inst = &sess.inst;
    let m = &inst.metrics;
    let s = sess.engine.stats();
    let repl = &inst.repl;
    let seq = repl.seq();
    let reps = repl.replicas();
    let lag = reps.iter().map(|r| seq.saturating_sub(r.acked)).max().unwrap_or(0);
    let load = |a: &std::sync::atomic::AtomicU64| a.load(Ordering::Relaxed);
    let total = load(&m.gets) + load(&m.sets) + load(&m.dels) + load(&m.others);
    let mut o = String::new();
    let mut metric = |name: &str, kind: &str, help: &str, lines: &[String]| {
        o.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n"));
        for l in lines {
            o.push_str(l);
            o.push('\n');
        }
    };
    metric(
        "kiva_requests_total",
        "counter",
        "Operations processed, by type.",
        &[
            format!("kiva_requests_total{{type=\"get\"}} {}", load(&m.gets)),
            format!("kiva_requests_total{{type=\"set\"}} {}", load(&m.sets)),
            format!("kiva_requests_total{{type=\"del\"}} {}", load(&m.dels)),
            format!("kiva_requests_total{{type=\"other\"}} {}", load(&m.others)),
        ],
    );
    metric(
        "kiva_request_duration_seconds",
        "summary",
        "Time spent executing commands.",
        &[
            format!("kiva_request_duration_seconds_sum {}", load(&m.micros) as f64 / 1e6),
            format!("kiva_request_duration_seconds_count {total}"),
        ],
    );
    metric(
        "kiva_errors_total",
        "counter",
        "Commands that ended in an error.",
        &[format!("kiva_errors_total {}", load(&m.errors))],
    );
    metric(
        "kiva_http_requests_total",
        "counter",
        "HTTP requests received.",
        &[format!("kiva_http_requests_total {}", load(&m.http_requests))],
    );
    metric(
        "kiva_failed_logins_total",
        "counter",
        "Rejected login attempts.",
        &[format!("kiva_failed_logins_total {}", load(&m.failed_logins))],
    );
    metric("kiva_keys", "gauge", "Live keys in the workspace.", &[format!("kiva_keys {}", s.keys)]);
    metric("kiva_storage_bytes", "gauge", "Size of the data file.", &[format!("kiva_storage_bytes {}", s.file_bytes)]);
    metric(
        "kiva_storage_live_bytes",
        "gauge",
        "Bytes occupied by live records.",
        &[format!("kiva_storage_live_bytes {}", s.live_bytes)],
    );
    metric(
        "kiva_storage_fragmentation_ratio",
        "gauge",
        "Share of the data file that is dead weight.",
        &[format!("kiva_storage_fragmentation_ratio {:.6}", s.fragmentation)],
    );
    metric(
        "kiva_memory_bytes",
        "gauge",
        "Estimated memory used by the index.",
        &[format!("kiva_memory_bytes {}", s.memory_bytes)],
    );
    metric(
        "kiva_checksum_errors_total",
        "counter",
        "Records that failed checksum verification.",
        &[format!("kiva_checksum_errors_total {}", s.corrupted_detected)],
    );
    metric(
        "kiva_compactions_total",
        "counter",
        "Compactions completed.",
        &[format!("kiva_compactions_total {}", s.compactions)],
    );
    metric(
        "kiva_connected_clients",
        "gauge",
        "Open TCP connections.",
        &[format!("kiva_connected_clients {}", inst.clients.load(Ordering::SeqCst))],
    );
    metric(
        "kiva_replicas_connected",
        "gauge",
        "Replicas attached to this node.",
        &[format!("kiva_replicas_connected {}", reps.len())],
    );
    metric(
        "kiva_replication_lag_ops",
        "gauge",
        "Operations the slowest replica is behind.",
        &[format!("kiva_replication_lag_ops {lag}")],
    );
    metric(
        "kiva_uptime_seconds",
        "gauge",
        "Seconds since the instance started.",
        &[format!("kiva_uptime_seconds {}", inst.started.elapsed().as_secs())],
    );
    o
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

fn ws_send(stream: &Mutex<TcpStream>, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => frame.push(n as u8),
        n if n < 65536 => {
            frame.push(126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            frame.push(127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    let mut s = stream.lock().unwrap_or_else(|e| e.into_inner());
    s.write_all(&frame)?;
    s.flush()
}

/// Reads one client frame: `(opcode, payload)`.
fn ws_read(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    stream.read_exact(&mut h)?;
    let opcode = h[0] & 0x0f;
    let masked = h[1] & 0x80 != 0;
    let mut len = (h[1] & 0x7f) as u64;
    if len == 126 {
        let mut b = [0u8; 2];
        stream.read_exact(&mut b)?;
        len = u16::from_be_bytes(b) as u64;
    } else if len == 127 {
        let mut b = [0u8; 8];
        stream.read_exact(&mut b)?;
        len = u64::from_be_bytes(b);
    }
    if len > 1 << 20 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut mask = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload)?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok((opcode, payload))
}

/// Upgrades the connection and streams key-change notifications as JSON.
/// Turns a notification (`set user:1 (object)`) into its JSON form.
fn event_json(msg: &str) -> Json {
    let mut parts = msg.splitn(3, ' ');
    let event = parts.next().unwrap_or("");
    let key = parts.next().unwrap_or("");
    let ty = parts.next().map(|t| t.trim_matches(|c| c == '(' || c == ')'));
    json!({ "event": event, "key": key, "type": ty, "timestamp": now_secs() })
}

/// `GET /v1/stream?pattern=user:*`: key changes as Server-Sent Events.
/// One-way and plain HTTP, so a browser's `EventSource` or `curl -N` is enough.
fn server_sent_events(mut sess: Session, rx: mpsc::Receiver<String>, mut stream: TcpStream, req: &Request, cors: bool) {
    if !sess.inst.cfg().get_bool("http", "stream") {
        write_response(&mut stream, &Response::error(404, "the event stream is turned off (http.stream)"), cors);
        return;
    }
    let pattern = req.query.get("pattern").cloned().unwrap_or_else(|| "*".into());
    let out = sess.execute(&format!("subscribe \"{}\"", pattern.replace('"', "")));
    if out.has_err() {
        write_response(&mut stream, &Response::error(400, out.text()), cors);
        return;
    }
    let mut head = String::from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n",
    );
    if cors {
        head.push_str("Access-Control-Allow-Origin: *\r\n");
    }
    head.push_str("\r\n");
    let hello = json!({ "event": "subscribed", "pattern": pattern, "workspace": sess.ws });
    let first = format!("{head}retry: 2000\n\nevent: subscribed\ndata: {hello}\n\n");
    if stream.write_all(first.as_bytes()).and_then(|_| stream.flush()).is_err() {
        return;
    }
    let mut id = 0u64;
    while !sess.inst.shutting_down.load(Ordering::SeqCst) {
        let chunk = match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(msg) => {
                id += 1;
                let j = event_json(&msg);
                format!("id: {id}\nevent: {}\ndata: {j}\n\n", j["event"].as_str().unwrap_or("change"))
            }
            // A comment line keeps proxies from closing an idle stream and
            // tells us when the client has gone.
            Err(mpsc::RecvTimeoutError::Timeout) => ": keep-alive\n\n".to_string(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if stream.write_all(chunk.as_bytes()).and_then(|_| stream.flush()).is_err() {
            break;
        }
    }
}

fn websocket(mut sess: Session, rx: mpsc::Receiver<String>, mut stream: TcpStream, req: &Request) {
    let Some(key) = req.headers.get("sec-websocket-key") else {
        write_response(&mut stream, &Response::error(400, "this endpoint expects a WebSocket upgrade"), false);
        return;
    };
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept = b64_encode(&hasher.finalize());
    let head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(None);
    let pattern = req.query.get("pattern").cloned().unwrap_or_else(|| "*".into());
    let out = sess.execute(&format!("subscribe \"{}\"", pattern.replace('"', "")));
    if out.has_err() {
        return;
    }
    let Ok(read_half) = stream.try_clone() else { return };
    let writer = Arc::new(Mutex::new(stream));
    let closed = Arc::new(AtomicBool::new(false));
    {
        let (writer, closed) = (writer.clone(), closed.clone());
        let mut read_half = read_half;
        let _ = std::thread::Builder::new().name("kiva-ws-read".into()).spawn(move || {
            loop {
                match ws_read(&mut read_half) {
                    Ok((0x8, _)) | Err(_) => break,
                    Ok((0x9, payload)) => {
                        if ws_send(&writer, 0xA, &payload).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                }
            }
            closed.store(true, Ordering::SeqCst);
        });
    }
    let hello = json!({ "event": "subscribed", "pattern": pattern, "workspace": sess.ws });
    if ws_send(&writer, 0x1, hello.to_string().as_bytes()).is_err() {
        return;
    }
    while !closed.load(Ordering::SeqCst) && !sess.inst.shutting_down.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(msg) => {
                let mut parts = msg.splitn(3, ' ');
                let event = parts.next().unwrap_or("");
                let key = parts.next().unwrap_or("");
                let ty = parts.next().map(|t| t.trim_matches(|c| c == '(' || c == ')'));
                let j = json!({ "event": event, "key": key, "type": ty, "timestamp": now_secs() });
                if ws_send(&writer, 0x1, j.to_string().as_bytes()).is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = ws_send(&writer, 0x8, &[]);
    let _ = writer.lock().unwrap_or_else(|e| e.into_inner()).shutdown(std::net::Shutdown::Both);
}

// ---------------------------------------------------------------------------
// Documentation
// ---------------------------------------------------------------------------

fn openapi() -> Json {
    let key_param = json!([{ "name": "key", "in": "path", "required": true, "schema": { "type": "string" } }]);
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "KivaDB REST API",
            "version": VERSION,
            "description": "Every response is JSON: {\"status\": \"ok\"|\"error\", ...}. When authentication is enabled, send HTTP Basic credentials."
        },
        "paths": {
            "/v1/key/{key}": {
                "get": { "summary": "Read a key. Optional ?path=.field reads inside a document.", "parameters": key_param },
                "post": {
                    "summary": "Create or replace a key. Body: any JSON value, or {\"value\": ..., \"ttl\": seconds}.",
                    "parameters": key_param,
                    "requestBody": { "content": { "application/json": { "example": { "value": { "name": "Fordi" }, "ttl": 3600 } } } }
                },
                "delete": { "summary": "Delete a key.", "parameters": key_param }
            },
            "/v1/scan": { "get": { "summary": "List keys with metadata. Query: pattern, limit." } },
            "/v1/keys": { "get": { "summary": "List key names in order. Query: from, to, limit, reverse." } },
            "/v1/range": { "get": { "summary": "Read a range of keys with their values. Query: from, to, limit, reverse." } },
            "/v1/stats": { "get": { "summary": "Engine statistics." } },
            "/v1/exec": {
                "post": {
                    "summary": "Run any shell command.",
                    "requestBody": { "content": { "application/json": { "example": { "command": "count where user:*.role = \"admin\"" } } } }
                }
            },
            "/metrics": { "get": { "summary": "Prometheus metrics (text format)." } },
            "/health": { "get": { "summary": "Liveness probe. No authentication." } },
            "/v1/api": {
                "post": {
                    "summary": "Structured request: whoami, scan, get, put, del, similar, stats, users, workspaces.",
                    "requestBody": { "content": { "application/json": { "example": { "op": "similar", "target": "doc:*.embedding", "vector": [0.1, 0.2, 0.3], "limit": 5 } } } }
                }
            },
            "/v1/stream": { "get": { "summary": "Server-Sent Events stream of key changes. Query: pattern (default *)." } },
            "/ws": { "get": { "summary": "WebSocket stream of key changes. Query: pattern (default *)." } }
        }
    })
}

const DOCS_HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>KivaDB API</title>
<style>
:root{--bg:#f7f7f5;--fg:#1c1c1a;--mut:#6b6b66;--card:#fff;--line:#deded8;--acc:#0b6e4f}
@media(prefers-color-scheme:dark){:root{--bg:#161614;--fg:#ecece6;--mut:#9a9a92;--card:#1f1f1c;--line:#34342f;--acc:#4fc39a}}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,sans-serif}
main{max-width:860px;margin:0 auto;padding:24px 16px}
h1{margin:0 0 4px;font-size:24px}p.sub{margin:0 0 20px;color:var(--mut)}
.ep{background:var(--card);border:1px solid var(--line);border-radius:8px;padding:12px 14px;margin:10px 0}
.m{display:inline-block;min-width:58px;font:600 12px ui-monospace,monospace;color:var(--acc)}
code,input,textarea,pre{font:13px ui-monospace,monospace}
.row{display:flex;gap:8px;flex-wrap:wrap;margin-top:8px}
input,textarea{flex:1;min-width:180px;padding:6px 8px;border:1px solid var(--line);border-radius:6px;background:var(--bg);color:var(--fg)}
button{padding:6px 14px;border:0;border-radius:6px;background:var(--acc);color:var(--bg);font-weight:600;cursor:pointer}
pre{margin:8px 0 0;padding:8px;border-radius:6px;background:var(--bg);overflow:auto;max-height:260px;white-space:pre-wrap}
.d{color:var(--mut);margin:2px 0 0}
</style></head><body><main>
<h1>KivaDB REST API</h1><p class="sub" id="sub">Loading...</p><div id="eps"></div>
<script>
fetch('/openapi.json').then(r=>r.json()).then(spec=>{
  document.getElementById('sub').textContent='Version '+spec.info.version+' - '+spec.info.description;
  const root=document.getElementById('eps');
  for(const [path,ops] of Object.entries(spec.paths))for(const [method,op] of Object.entries(ops)){
    const el=document.createElement('div');el.className='ep';
    const body=op.requestBody?JSON.stringify(Object.values(op.requestBody.content)[0].example):'';
    el.innerHTML='<div><span class="m"></span><code></code></div><p class="d"></p><div class="row"><input><button>Try it</button></div>'+(body?'<div class="row"><textarea rows="2"></textarea></div>':'')+'<pre hidden></pre>';
    el.querySelector('.m').textContent=method.toUpperCase();el.querySelector('code').textContent=path;
    el.querySelector('.d').textContent=op.summary;
    const url=el.querySelector('input');url.value=path.replace('{key}','user:1');
    const ta=el.querySelector('textarea');if(ta)ta.value=body;
    const pre=el.querySelector('pre');
    el.querySelector('button').onclick=async()=>{
      pre.hidden=false;pre.textContent='...';
      try{const r=await fetch(url.value,{method:method.toUpperCase(),headers:ta?{'Content-Type':'application/json'}:{},body:ta?ta.value:undefined});
        const t=await r.text();try{pre.textContent=r.status+'\n'+JSON.stringify(JSON.parse(t),null,2)}catch(e){pre.textContent=r.status+'\n'+t}
      }catch(e){pre.textContent=String(e)}};
    root.appendChild(el);
  }
});
</script></main></body></html>"##;
