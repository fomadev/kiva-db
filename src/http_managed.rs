// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use super::{write_transport_response, Response};
use crate::auth::{Role, User};
use crate::error::{KivaError, Result};
use crate::http_sessions;
use crate::instance::Instance;
use crate::managed_transport as transport;
use crate::native_admission::Class;
use crate::native_timing::{span, Stage};
use crate::server::Deadline;
use crate::session::Session;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader};
use std::net::TcpStream;
use std::sync::{atomic::Ordering, Arc};

/// Start the explicit managed HTTP adapter on a hierarchical instance.
pub fn start_hierarchical_http(inst: &Arc<Instance>, bind: &str, port: u16) -> Result<u16> {
    transport::start(
        inst,
        bind,
        port,
        "http",
        b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        serve,
    )
}

const HEADER_MAX: usize = 16 * 1024;
const REQUESTS_PER_CONNECTION: usize = 128;

struct Request {
    method: String,
    path: Vec<String>,
    authorization: Option<String>,
    body: Vec<u8>,
    keep_alive: bool,
}

fn invalid(message: &str) -> KivaError {
    KivaError::Invalid(message.into())
}

fn error(status: u16, code: &str, message: &str) -> Response {
    Response::json(status, json!({"error": code, "message": message}))
}

fn unauthorized() -> Response {
    let mut response = error(401, "authentication_required", "valid HTTP credentials are required");
    response.extra.push(("WWW-Authenticate".into(), "Basic realm=\"KivaDB hierarchy\"".into()));
    response.extra.push(("WWW-Authenticate".into(), "Bearer realm=\"KivaDB hierarchy\"".into()));
    response
}

fn public_error(err: KivaError) -> Response {
    match err {
        KivaError::Denied(m) if m == http_sessions::INVALID => unauthorized(),
        KivaError::Other(m) if m.starts_with("ERR_SESSION_LIMIT:") => {
            error(429, "session_capacity", "HTTP session capacity reached")
        }
        KivaError::Denied(m) if m == "hierarchy resource or operation is unavailable" => missing(),
        KivaError::NotFound(_) => missing(),
        KivaError::Denied(_) => error(403, "forbidden", "operation is forbidden"),
        KivaError::Other(m) | KivaError::Invalid(m) if m.starts_with("ERR_ADAPTER_LIMIT:") => {
            error(422, "result_too_large", "operation exceeds the adapter value or result limit")
        }
        KivaError::Other(m) if m.starts_with("ERR_SERVER_BUSY:") => {
            error(503, "server_busy", "request was refused before execution")
        }
        KivaError::Exists(_) | KivaError::Conflict(_) | KivaError::Unique(_) | KivaError::ReadOnly(_) => {
            error(409, "conflict", "operation conflicts with the current state")
        }
        KivaError::Invalid(_) | KivaError::Type(_) => error(400, "invalid_request", "invalid operation or arguments"),
        KivaError::Schema(_) => error(422, "schema", "value does not satisfy the collection schema"),
        _ => error(500, "outcome_unknown", "operation could not be confirmed"),
    }
}

fn missing() -> Response {
    error(404, "not_found", "resource is unavailable")
}

fn decode_component(input: &str) -> Option<String> {
    fn hex(ch: u8) -> Option<u8> {
        (ch as char).to_digit(16).map(|n| n as u8)
    }

    let mut rest = input.as_bytes();
    let mut bytes = Vec::with_capacity(rest.len());
    while let Some((&first, remaining)) = rest.split_first() {
        let byte = if first == b'%' {
            let [a, b, tail @ ..] = remaining else { return None };
            rest = tail;
            hex(*a)? * 16 + hex(*b)?
        } else {
            rest = remaining;
            first
        };
        if byte == b'/' || byte == b'\\' || byte.is_ascii_control() {
            return None;
        }
        bytes.push(byte);
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.is_empty() || matches!(text.as_str(), "." | "..") {
        return None;
    }
    Some(text)
}

fn read_request(reader: &mut impl BufRead) -> std::result::Result<Option<Request>, u16> {
    let get_line = |reader: &mut _, limit| {
        transport::line(reader, limit).map_err(|e| if e.kind() == io::ErrorKind::InvalidInput { 431u16 } else { 400 })
    };
    let Some(first) = get_line(reader, HEADER_MAX)? else { return Ok(None) };
    let headers_timing = span(Stage::HttpHeaders);
    let mut remaining = HEADER_MAX - first.len() - 2;
    let first = std::str::from_utf8(&first).map_err(|_| 400u16)?;
    let mut words = first.split(' ');
    let (Some(method), Some(target), Some(version), None) = (words.next(), words.next(), words.next(), words.next())
    else {
        return Err(400);
    };
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || method.is_empty()
        || !method.bytes().all(|b| b.is_ascii_uppercase())
    {
        return Err(400);
    }
    let target = target.strip_prefix('/').filter(|p| !p.contains(['?', '#'])).ok_or(400u16)?;
    let path = target.split('/').map(decode_component).collect::<Option<Vec<_>>>().ok_or(400u16)?;
    let mut headers = HashMap::new();
    loop {
        if remaining < 2 {
            return Err(431);
        }
        let line = get_line(reader, remaining)?.ok_or(400u16)?;
        remaining -= line.len() + 2;
        if line.is_empty() {
            break;
        }
        if headers.len() >= 64 {
            return Err(431);
        }
        let line = std::str::from_utf8(&line).map_err(|_| 400u16)?;
        let (name, value) = line.split_once(':').ok_or(400u16)?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.bytes().any(|b| b.is_ascii_control() && b != b'\t')
            || headers.insert(name.to_ascii_lowercase(), value.trim().to_owned()).is_some()
        {
            return Err(400);
        }
    }
    if headers.contains_key("transfer-encoding")
        || headers.contains_key("expect")
        || (version == "HTTP/1.1" && headers.get("host").is_none_or(String::is_empty))
    {
        return Err(400);
    }
    let len: usize = match headers.get("content-length") {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => n.parse().map_err(|_| 413u16)?,
        Some(_) => return Err(400),
        None => 0,
    };
    if len > transport::REQUEST_MAX {
        return Err(413);
    }
    drop(headers_timing);
    let mut body = vec![0; len];
    reader.read_exact(&mut body).map_err(|_| 400u16)?;
    let keep_alive = version == "HTTP/1.1"
        && !headers
            .get("connection")
            .is_some_and(|v| v.split(',').any(|token| token.trim().eq_ignore_ascii_case("close")));
    Ok(Some(Request { method: method.into(), path, authorization: headers.remove("authorization"), body, keep_alive }))
}

fn serve(inst: &Arc<Instance>, mut stream: TcpStream) {
    let Ok(mut writer) = stream.try_clone() else { return };
    let (mut read_timeout, mut write_timeout) = (None, None);
    let mut reader = BufReader::new(Deadline::new(&mut stream, &mut read_timeout));
    for n in 0..REQUESTS_PER_CONNECTION {
        if inst.shutting_down.load(Ordering::Acquire) {
            break;
        }
        reader.get_mut().rearm();
        if let Some(metrics) = &inst.native_metrics {
            metrics.begin();
        }
        let request = {
            let _read = span(Stage::FrameRead);
            read_request(&mut reader)
        };
        let bearer = inst.native_metrics.is_some()
            && request.as_ref().ok().and_then(|req| req.as_ref()).is_some_and(|req| bearer_token(req).is_some());
        let (mut response, keep_alive) = match request {
            Ok(Some(req)) => (dispatch(inst, &req), req.keep_alive && n + 1 < REQUESTS_PER_CONNECTION),
            Ok(None) => {
                if let Some(metrics) = &inst.native_metrics {
                    metrics.discard();
                }
                break;
            }
            Err(status) => (error(status, "invalid_http", "invalid or excessive HTTP request"), false),
        };
        if response.body.len() > transport::REPLY_MAX {
            response = error(500, "outcome_unknown", "response exceeded the output limit");
        }
        response.extra.push(("Cache-Control".into(), "no-store".into()));
        let written = {
            let _write = span(Stage::FrameWrite);
            write_transport_response(&mut Deadline::new(&mut writer, &mut write_timeout), &response, false, keep_alive)
        };
        if let Some(metrics) = &inst.native_metrics {
            metrics.finish_http(bearer);
        }
        if written.is_err() || !keep_alive {
            break;
        }
    }
}

fn credentials(header: Option<&str>) -> Option<(String, String)> {
    let _decode = span(Stage::BasicDecode);
    let (scheme, token) = header?.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") || token.is_empty() || token.len() > 4096 {
        return None;
    }
    let text = String::from_utf8(crate::util::b64_decode(token)?).ok()?;
    let (user, password) = text.split_once(':')?;
    if user.is_empty() || password.is_empty() || user.contains('\0') || password.contains('\0') {
        return None;
    }
    Some((user.into(), password.into()))
}

fn bearer_token(req: &Request) -> Option<&str> {
    let (scheme, token) = req.authorization.as_deref()?.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

fn authenticate(sess: &mut Session, req: &Request) -> Result<Option<User>> {
    if let Some(token) = bearer_token(req) {
        let lease = {
            let _token = span(Stage::Token);
            sess.inst.http_sessions.lookup(token)?
        };
        sess.resume_http(lease)?;
        return Ok(None);
    }
    let (user, pass) = credentials(req.authorization.as_deref()).ok_or_else(http_sessions::invalid)?;
    transport::login(sess, &user, &pass).map(Some).map_err(|err| match err {
        KivaError::Denied(_) | KivaError::NotFound(_) => http_sessions::invalid(),
        other => other,
    })
}

fn dispatch(inst: &Arc<Instance>, req: &Request) -> Response {
    if req.path == ["health"] && req.method == "GET" {
        return Response::json(200, json!({"status":"ok", "mode":"hierarchical"}));
    }
    dispatch_authenticated(inst, req).unwrap_or_else(public_error)
}

fn dispatch_authenticated(inst: &Arc<Instance>, req: &Request) -> Result<Response> {
    let mut sess = {
        let _session = span(Stage::Session);
        transport::session(inst)
    };
    let verified = authenticate(&mut sess, req)?;
    let path: Vec<_> = req.path.iter().map(String::as_str).collect();
    let class = match (req.method.as_str(), path.as_slice()) {
        ("GET", ["v2", "workspaces", _, "projects", _, "collections", _, "keys", _]) => Class::Read,
        (_, ["v2", "workspaces", _, "projects", _, "collections", _, "api" | "keys" | "batch", ..]) => Class::Data,
        _ => Class::Control,
    };
    let queued = span(Stage::DataQueue);
    let _permit = inst.native_admission.enter_bounded(class, transport::WAIT)?;
    drop(queued);
    if let ["v2", "auth", action] = path.as_slice() {
        return auth_route(&sess, verified.as_ref(), req, action);
    }
    route(&mut sess, req, &path)
}

fn auth_route(sess: &Session, verified: Option<&User>, req: &Request, action: &str) -> Result<Response> {
    let sessions = &sess.inst.http_sessions;
    if action != "revoke" && !req.body.is_empty() {
        return Err(invalid("this session operation does not take a body"));
    }
    let require_lease = || sess.http_lease.as_deref().ok_or_else(http_sessions::invalid);
    let require_admin = |user: &User| {
        if user.role != Role::Admin {
            return Err(KivaError::Denied("administrator required".into()));
        }
        Ok(())
    };
    match (req.method.as_str(), action) {
        ("POST", "login") => {
            let verified = verified.ok_or_else(http_sessions::invalid)?;
            let (token, lease) = sess.with_http_account(false, |user| {
                if user.credential_generation() != verified.credential_generation() {
                    return Err(http_sessions::invalid());
                }
                sessions.issue(user)
            })?;
            Ok(Response::json(201, json!({"access_token": token, "token_type": "Bearer", "session": lease.info()})))
        }
        ("GET", "session") => {
            let lease = require_lease()?;
            sess.with_http_account(false, |_| Ok(Response::json(200, lease.info())))
        }
        ("POST", "logout") => {
            let lease = require_lease()?;
            sess.with_http_account(true, |_| sessions.logout(lease))?;
            Ok(Response::json(200, json!({"revoked": true})))
        }
        ("POST", "revoke") => {
            let body = json_body(req)?;
            let account = body["account"]
                .as_str()
                .filter(|name| crate::auth::valid_username(name))
                .ok_or_else(|| invalid("valid account name is required"))?;
            let revoked = sess.with_http_account(true, |user| {
                require_admin(user)?;
                sessions.revoke_account(account)
            })?;
            Ok(Response::json(200, json!({"revoked": revoked})))
        }
        ("GET", "metrics") => sess.with_http_account(false, |user| {
            require_admin(user)?;
            let timings: Vec<_> = sess
                .inst
                .native_timings()
                .into_iter()
                .filter(|t| t.samples > 0)
                .map(|t| json!({"operation":t.operation,"stage":t.stage,"samples":t.samples,"nanos":t.nanos}))
                .collect();
            Ok(Response::json(200, json!({"sessions": sessions.metrics()?, "timings": timings})))
        }),
        (_, "login" | "session" | "logout" | "revoke" | "metrics") => {
            Ok(error(405, "method_not_allowed", "unsupported method on this resource"))
        }
        _ => Ok(missing()),
    }
}

fn json_body(req: &Request) -> Result<Json> {
    let body = crate::num::from_slice(&req.body).map_err(|_| invalid("invalid JSON"))?;
    if !body.is_object() {
        return Err(invalid("expected a JSON object"));
    }
    Ok(body)
}

fn route(sess: &mut Session, req: &Request, path: &[&str]) -> Result<Response> {
    use transport::{command, name, select};
    let (noun, target) = match path {
        ["v2", "workspaces"] => ("workspace", None),
        ["v2", "workspaces", w] => ("workspace", Some(name(w)?)),
        ["v2", "workspaces", w, "projects", rest @ ..] if rest.len() <= 1 => {
            command(sess, &format!("switch \"{}\"", name(w)?))?;
            ("project", rest.first().map(|p| name(p)).transpose()?)
        }
        ["v2", "workspaces", w, "projects", p, "collections", rest @ ..] if rest.len() <= 1 => {
            command(sess, &format!("switch \"{}\"", name(w)?))?;
            command(sess, &format!("use \"{}\"", name(p)?))?;
            ("collection", rest.first().map(|c| name(c)).transpose()?)
        }
        ["v2", "workspaces", w, "projects", p, "collections", c, action @ ..] if !action.is_empty() => {
            let address = [name(w)?, name(p)?, name(c)?].join(".");
            {
                let _path = span(Stage::HierarchyPath);
                select(sess, &address)?;
            }
            return data(sess, req, action);
        }
        _ => return Ok(missing()),
    };
    let (source, status) = match (req.method.as_str(), target) {
        ("GET", None) => (format!("{noun} list"), 200),
        ("GET", Some(target)) => (format!("{noun} info \"{target}\""), 200),
        ("POST", None) => {
            let body = json_body(req)?;
            let new = name(body["name"].as_str().ok_or_else(|| invalid("name is required"))?)?;
            (format!("{noun} create \"{new}\""), 201)
        }
        ("PUT", Some(target)) => {
            let body = json_body(req)?;
            let new = name(body["name"].as_str().ok_or_else(|| invalid("name is required"))?)?;
            (format!("{noun} rename \"{target}\" to \"{new}\""), 200)
        }
        ("DELETE", Some(target)) => {
            let cascade = if req.body.is_empty() {
                false
            } else {
                json_body(req)?["cascade"].as_bool().ok_or_else(|| invalid("cascade must be a boolean"))?
            };
            if cascade && noun == "collection" {
                return Err(invalid("a collection has no children"));
            }
            (format!("{noun} drop \"{target}\"{}", if cascade { " cascade" } else { "" }), 200)
        }
        _ => return Ok(error(405, "method_not_allowed", "unsupported method on this resource")),
    };
    let output = command(sess, &source)?.text();
    let body = if req.method == "GET" && target.is_none() {
        json!({"items": output.lines().filter(|s| !s.is_empty()).collect::<Vec<_>>()})
    } else {
        json!({"message":output})
    };
    Ok(Response::json(status, body))
}

fn data(sess: &mut Session, req: &Request, action: &[&str]) -> Result<Response> {
    let value = match (req.method.as_str(), action) {
        ("POST", ["batch"]) => crate::api::handle_hierarchy_batch(sess, &json_body(req)?)?,
        ("POST", ["api"]) => {
            let mut body = json_body(req)?;
            if matches!(body["op"].as_str(), Some("begin" | "commit" | "rollback")) {
                return Ok(error(400, "transaction_requires_batch", "use the atomic batch endpoint"));
            }
            if let Some("query" | "scan" | "similar") = body["op"].as_str() {
                let limit = match body.get("limit") {
                    Some(value) => value.as_u64().ok_or_else(|| invalid("limit must be an integer"))?,
                    None => 100,
                };
                if !(1..=1000).contains(&limit) {
                    return Err(invalid("limit is outside 1 to 1000"));
                }
                body["limit"] = json!(limit);
            }
            crate::api::handle_native_hierarchy(sess, &body)?
        }
        ("GET" | "PUT" | "DELETE", ["keys", key]) => {
            crate::exec::validate_key_name(key).map_err(KivaError::Invalid)?;
            let mut body = match req.method.as_str() {
                "PUT" => json_body(req)?,
                _ => json!({}),
            };
            body["key"] = json!(key);
            body["op"] = json!(match req.method.as_str() {
                "GET" => "get",
                "PUT" => "put",
                _ => "del",
            });
            if body["types"].is_null() {
                body["types"] = json!("extended");
            }
            let answer = crate::api::handle_native_hierarchy(sess, &body)?;
            if answer["found"] == false {
                return Ok(missing());
            }
            answer
        }
        ("POST", ["maintenance"]) => {
            let body = json_body(req)?;
            let source =
                body["command"].as_str().filter(|s| s.len() <= 65_536).ok_or_else(|| invalid("command is required"))?;
            let tokens = crate::lexer::tokenize(source).map_err(KivaError::Invalid)?;
            let verb = tokens.first().filter(|t| t.is_bare()).map(|t| t.text.to_ascii_lowercase()).unwrap_or_default();
            if !matches!(
                verb.as_str(),
                "create" | "drop" | "indexes" | "schema" | "schemas" | "changes" | "compact" | "check"
            ) || (verb == "changes" && tokens.get(1).is_some_and(|token| token.is_kw("read")))
                || crate::exec::managed_command(&verb, &tokens[1..]).is_none()
            {
                return Ok(error(400, "unsupported_operation", "operation is unavailable on this endpoint"));
            }
            let out = sess.execute_admitted(source);
            if out.has_err() {
                return Ok(if out.text().contains(http_sessions::INVALID) {
                    unauthorized()
                } else if out.text().contains("permission denied") {
                    error(403, "forbidden", "operation is forbidden")
                } else {
                    error(500, "outcome_unknown", "collection operation could not be confirmed")
                });
            }
            json!({"message": out.text()})
        }
        _ => return Ok(error(405, "method_not_allowed", "unsupported managed collection operation")),
    };
    let _encode = span(Stage::Encode);
    Ok(Response::json(200, value))
}
