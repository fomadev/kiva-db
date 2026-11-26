// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! TCP server. Each connection gets its own thread and session.
//! The hierarchical listener uses bounded admission lanes before dispatch.

use crate::error::{KivaError, Result};
use crate::exec::{validate_key_name, Action};
use crate::instance::{Instance, VERSION};
use crate::native_admission::Class;
pub use crate::native_timing::NativeTiming;
use crate::native_timing::{span, Stage};
use crate::proto::*;
use crate::repl;
use crate::session::{Need, Session};
use crate::value::Type;
use std::io::{BufRead, BufReader, IoSlice, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Starts listening and returns the bound port (useful with port 0).
pub fn start_tcp(inst: &Arc<Instance>, bind: &str, port: u16) -> Result<u16> {
    let listener = TcpListener::bind((bind, port))
        .map_err(|e| KivaError::Other(format!("cannot listen on {bind}:{port}: {e}")))?;
    let address = listener.local_addr()?;
    if inst.hierarchical_tcp
        && !address.ip().is_loopback()
        && (!inst.cfg().get_bool("tls", "required") || inst.tls().is_none())
    {
        return Err(KivaError::Denied("hierarchical TCP requires TLS on non-loopback bindings".into()));
    }
    let port = address.port();
    inst.tcp_port.store(port as u64, Ordering::SeqCst);
    let _ = std::fs::write(inst.home.join("instance.port"), port.to_string());
    let _ = std::fs::write(inst.home.join("instance.pid"), std::process::id().to_string());
    let inst = inst.clone();
    std::thread::Builder::new().name("kiva-tcp".into()).spawn(move || {
        for conn in listener.incoming() {
            if inst.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = conn else { continue };
            let max = inst.cfg().get_u64("server", "max_clients") as usize;
            let max = if inst.hierarchical_tcp { max.min(128) } else { max };
            let slot = ClientSlot(inst.clone());
            if inst.clients.fetch_add(1, Ordering::SeqCst) >= max {
                let mut stream = stream;
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = stream.write_all(b"Error: max clients reached\n");
                continue;
            }
            let _ = std::thread::Builder::new().name("kiva-conn".into()).spawn(move || {
                if let Some((stream, peer)) = crate::tls::accept(&slot.0, stream, &refusal) {
                    handle_conn(&slot.0, stream, peer);
                }
            });
        }
    })?;
    Ok(port)
}

struct ClientSlot(Arc<Instance>);

impl Drop for ClientSlot {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::SeqCst);
    }
}

/// "Encryption required", in the protocol the client started to speak.
fn refusal(first: u8) -> Vec<u8> {
    const MSG: &str = "this server requires an encrypted connection (TLS) from other machines";
    if first == 0 {
        let mut frame = Vec::new();
        let _ = crate::proto::write_frame(&mut frame, crate::proto::ST_ERR, MSG.as_bytes());
        frame
    } else {
        format!("Error: {MSG}. Connect with: kivadb --connect <host:port> --tls\n").into_bytes()
    }
}

/// Serves one connection; `peer` is the client's address (the stream itself
/// may be the clear end of a TLS relay, on the loopback interface).
fn handle_conn(inst: &Arc<Instance>, stream: TcpStream, peer: std::net::SocketAddr) {
    let _ = stream.set_nodelay(true);
    if inst.hierarchical_tcp {
        let _ = stream.set_read_timeout(Some(NATIVE_TIMEOUT));
        let _ = stream.set_write_timeout(Some(NATIVE_TIMEOUT));
    }
    let mut first = [0u8; 1];
    match stream.peek(&mut first) {
        Ok(1) => {}
        _ => return,
    }
    if inst.hierarchical_tcp {
        if first[0] == 0 {
            hierarchy_loop(inst, stream);
        } else {
            let mut stream = stream;
            let _ = stream.write_all(b"Error: hierarchical TCP requires the negotiated binary protocol\n");
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            let _ = stream.read(&mut [0; 1024]);
        }
        return;
    }
    let (tx, rx) = mpsc::channel::<String>();
    let sess = Session::new(inst.clone(), peer.ip().is_loopback(), Some(tx));
    let Ok(write_half) = stream.try_clone() else { return };
    let writer = Arc::new(Mutex::new(write_half));
    // A leading zero byte is the high byte of a frame length: binary protocol.
    if first[0] == 0 {
        binary_loop(inst, sess, stream, writer, rx, peer.to_string());
    } else {
        text_loop(sess, stream, writer, rx);
    }
}

/// Forwards key-change notifications to the client as they happen.
fn forward_events(rx: Receiver<String>, writer: Arc<Mutex<TcpStream>>, binary: bool) {
    let _ = std::thread::Builder::new().name("kiva-events".into()).spawn(move || {
        for ev in rx {
            let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
            let res = if binary {
                write_frame(&mut *w, ST_EVENT, ev.as_bytes())
            } else {
                w.write_all(format!("[event] {ev}\n").as_bytes())
            };
            if res.is_err() {
                break;
            }
        }
    });
}

fn text_loop(mut sess: Session, stream: TcpStream, writer: Arc<Mutex<TcpStream>>, rx: Receiver<String>) {
    forward_events(rx, writer.clone(), false);
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let out = sess.execute(&line);
        let mut text = out.text();
        if !text.is_empty() {
            text.push('\n');
        }
        let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
        if w.write_all(text.as_bytes()).is_err() || out.action == Action::Exit {
            break;
        }
    }
}

fn binary_loop(
    inst: &Arc<Instance>,
    mut sess: Session,
    mut stream: TcpStream,
    writer: Arc<Mutex<TcpStream>>,
    rx: Receiver<String>,
    peer: String,
) {
    forward_events(rx, writer.clone(), true);
    let reply = |code: u16, payload: &[u8]| {
        let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
        write_frame(&mut *w, code, payload)
    };
    while let Ok((cmd, payload)) = read_frame(&mut stream) {
        let res: std::io::Result<()> = match cmd {
            CMD_HELLO => {
                let info = format!(
                    "KivaDB {VERSION}\nauth={}\nuser={}",
                    if sess.user.is_none() { "required" } else { "none" },
                    sess.user_name()
                );
                reply(ST_OK, info.as_bytes())
            }
            CMD_PING => reply(ST_OK, b"PONG"),
            CMD_NEGOTIATE => reply(ST_ERR, b"hierarchical mode is not enabled on this server"),
            CMD_AUTH => {
                let mut parts = payload.splitn(2, |b| *b == 0);
                let user = String::from_utf8_lossy(parts.next().unwrap_or(&[])).into_owned();
                let pass = String::from_utf8_lossy(parts.next().unwrap_or(&[])).into_owned();
                match sess.login(&user, &pass) {
                    Ok(u) => {
                        let mut msg = format!("Welcome {} ({})", u.name, u.role.name());
                        if u.has_empty_password() {
                            msg.push_str(&format!(
                                "\n[WARN] {0} password is empty. Please run: modify user \"{0}\" password \"...\"",
                                u.name
                            ));
                        }
                        reply(ST_OK, msg.as_bytes())
                    }
                    Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
                }
            }
            CMD_EXEC => {
                let line = String::from_utf8_lossy(&payload);
                let out = sess.execute(&line);
                // Each line is tagged so the client can tell errors apart.
                let body = out
                    .lines
                    .iter()
                    .map(|(err, l)| format!("{}{l}", if *err { '-' } else { '+' }))
                    .collect::<Vec<_>>()
                    .join("\n");
                let code = match out.action {
                    Action::Exit => ST_BYE,
                    Action::Clear => ST_CLEAR,
                    Action::None if out.has_err() => ST_ERR,
                    Action::None => ST_OK,
                };
                let r = reply(code, body.as_bytes());
                if out.action == Action::Exit {
                    break;
                }
                r
            }
            CMD_GET => {
                let key = String::from_utf8_lossy(&payload);
                match sess.kv_get(&key) {
                    Ok(Some((v, _))) => match v.encode(false) {
                        Ok(raw) => {
                            let mut p = vec![v.ty() as u8];
                            p.extend_from_slice(&raw);
                            reply(ST_OK, &p)
                        }
                        Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
                    },
                    Ok(None) => reply(ST_NOT_FOUND, &[]),
                    Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
                }
            }
            CMD_SET => match binary_set(&mut sess, &payload) {
                Ok(()) => reply(ST_OK, b"OK"),
                Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
            },
            CMD_DEL => {
                let key = String::from_utf8_lossy(&payload);
                match sess.kv_del(&key) {
                    Ok(true) => reply(ST_OK, b"1"),
                    Ok(false) => reply(ST_NOT_FOUND, b"0"),
                    Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
                }
            }
            CMD_API => {
                let res = crate::num::from_slice(&payload)
                    .map_err(KivaError::Invalid)
                    .and_then(|req| crate::api::handle(&mut sess, &req));
                match res {
                    Ok(j) => reply(ST_OK, j.to_string().as_bytes()),
                    Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
                }
            }
            CMD_FAILOVER => {
                // Only the nodes of the group (logged in as administrators) take part.
                if let Err(e) = sess.check(Need::Admin) {
                    reply(ST_ERR, e.to_string().as_bytes())
                } else {
                    match serde_json::from_slice::<serde_json::Value>(&payload) {
                        Ok(req) => reply(ST_OK, crate::failover::handle(inst, &req).to_string().as_bytes()),
                        Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
                    }
                }
            }
            CMD_REPL_SYNC => {
                if let Err(e) = sess.check(Need::Admin) {
                    let _ = reply(ST_ERR, e.to_string().as_bytes());
                    break;
                }
                let listen = String::from_utf8_lossy(&payload).into_owned();
                inst.audit.log(sess.user_name(), &format!("replica connected from {peer}"), true);
                // From here on the connection belongs to the replication stream.
                drop(sess);
                repl::serve_replica(inst, stream, peer, listen);
                return;
            }
            _ => reply(ST_ERR, b"unknown command id"),
        };
        if res.is_err() {
            break;
        }
    }
}

/// Payload: `type u8 | ttl u32 | key_len u16 | key | value bytes`.
fn binary_set(sess: &mut Session, payload: &[u8]) -> Result<()> {
    let decode = span(Stage::Decode);
    let mut r = Reader::new(payload);
    let ty = Type::from_u8(r.u8()?).ok_or_else(|| KivaError::Invalid("unknown type tag".into()))?;
    let ttl = r.u32()? as i64;
    let key = r.str16()?;
    validate_key_name(&key).map_err(KivaError::Invalid)?;
    let value = decode_value(ty, r.rest(), sess.native_hierarchy)
        .map_err(|e| KivaError::Invalid(format!("value does not match its type: {e}")))?;
    let expires_at = crate::util::deadline_in(ttl);
    drop(decode);
    sess.kv_put(&key, value, expires_at, false)
}

const NATIVE_TIMEOUT: Duration = Duration::from_secs(30);
const NATIVE_REQUEST_MAX: usize = 2 * 1024 * 1024;
const NATIVE_REPLY_MAX: usize = 4 * 1024 * 1024;

/// Bound the whole frame. Cache the socket setting across ordinary
/// frames, then shorten it as the absolute deadline approaches.
struct Deadline<'a> {
    stream: &'a mut TcpStream,
    configured: &'a mut Option<Duration>,
    expires: Instant,
}

impl<'a> Deadline<'a> {
    fn new(stream: &'a mut TcpStream, configured: &'a mut Option<Duration>) -> Self {
        Self { stream, configured, expires: Instant::now() + NATIVE_TIMEOUT }
    }

    fn remaining(&self) -> std::io::Result<Duration> {
        let delay = self.expires.saturating_duration_since(Instant::now());
        if delay.is_zero() {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "native frame timed out"));
        }
        Ok(delay)
    }

    fn transfer(
        &mut self,
        configure: impl Fn(&TcpStream, Option<Duration>) -> std::io::Result<()>,
        mut operation: impl FnMut(&mut TcpStream) -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        loop {
            let delay = self.remaining()?.min(Duration::from_secs(1));
            if *self.configured != Some(delay) {
                configure(self.stream, Some(delay))?;
                *self.configured = Some(delay);
            }
            match operation(self.stream) {
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Ok(n) => {
                    self.remaining()?;
                    return Ok(n);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Read for Deadline<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.transfer(TcpStream::set_read_timeout, |stream| stream.read(bytes))
    }
}

impl Write for Deadline<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.transfer(TcpStream::set_write_timeout, |stream| stream.write(bytes))
    }

    fn write_vectored(&mut self, bytes: &[IoSlice<'_>]) -> std::io::Result<usize> {
        self.transfer(TcpStream::set_write_timeout, |stream| stream.write_vectored(bytes))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

fn hierarchy_loop(inst: &Arc<Instance>, mut stream: TcpStream) {
    let mut session = Session::new_native(inst.clone(), false, None);
    let mut negotiated = false;
    let mut payload = Vec::new();
    let (mut read_timeout, mut write_timeout) = (None, None);
    while !inst.shutting_down.load(Ordering::Acquire) {
        if let Some(metrics) = &inst.native_metrics {
            metrics.begin();
        }
        let input = {
            let _read = span(Stage::FrameRead);
            read_frame_into(
                &mut Deadline::new(&mut stream, &mut read_timeout),
                NATIVE_REQUEST_MAX,
                &|| false,
                &mut payload,
            )
        };
        let Ok(cmd) = input else { break };
        let response = hierarchy_request(&mut session, &mut negotiated, cmd, &payload);
        if payload.capacity() > 64 * 1024 {
            payload = Vec::new();
        }
        let (status, bytes) = match response {
            Ok(reply) => reply,
            Err(error) => {
                let error = match error {
                    KivaError::Io(_) | KivaError::Corrupted(_) | KivaError::Locked(_) | KivaError::Format(_) => {
                        "managed storage operation failed".to_owned()
                    }
                    error => error.to_string(),
                };
                let body = if cmd == CMD_EXEC { format!("-Error: {error}") } else { error };
                (ST_ERR, body.into_bytes())
            }
        };
        let (status, body) = if bytes.len() + 2 > NATIVE_REPLY_MAX {
            (
                ST_ERR,
                if cmd == CMD_EXEC {
                    &b"-Error: response exceeds native limit"[..]
                } else {
                    &b"response exceeds native limit"[..]
                },
            )
        } else {
            (status, bytes.as_slice())
        };
        // Session dispatch has released all admission and storage guards.
        let output = {
            let _write = span(Stage::FrameWrite);
            write_native_frame(&mut Deadline::new(&mut stream, &mut write_timeout), status, body)
        };
        if let Some(metrics) = &inst.native_metrics {
            metrics.finish(cmd);
        }
        if output.is_err() || status == ST_BYE {
            break;
        }
    }
}

fn hierarchy_request(sess: &mut Session, negotiated: &mut bool, cmd: u16, payload: &[u8]) -> Result<(u16, Vec<u8>)> {
    let ok = |body: Vec<u8>| Ok((ST_OK, body));
    match cmd {
        CMD_HELLO if payload.is_empty() => {
            return ok(format!("KivaDB {VERSION}\nauth=required\ncapability={HIERARCHY_V1}").into_bytes());
        }
        CMD_PING if payload.is_empty() => return ok(b"PONG".to_vec()),
        CMD_NEGOTIATE => {
            if *negotiated || payload != HIERARCHY_V1.as_bytes() {
                return Err(KivaError::Invalid("expected a single hierarchy/1 negotiation".into()));
            }
            *negotiated = true;
            return ok(payload.to_vec());
        }
        _ => {}
    }
    if !*negotiated {
        return Err(KivaError::Denied("negotiate hierarchy/1 before authentication or data access".into()));
    }
    if cmd == CMD_AUTH {
        let inst = sess.inst.clone();
        let _login = inst.native_admission.enter(Class::Authentication)?;
        let _authentication = span(Stage::Authentication);
        let credentials = native_text(payload, 4096)?;
        let (user, password) = credentials
            .split_once('\0')
            .filter(|(_, password)| !password.contains('\0'))
            .ok_or_else(|| KivaError::Invalid("invalid authentication payload".into()))?;
        let user = sess.login(user, password)?;
        return ok(format!("Welcome {} ({})", user.name, user.role.name()).into_bytes());
    }
    if sess.user.is_none() || !sess.hierarchy_data {
        return Err(KivaError::Denied("authentication required".into()));
    }
    if cmd == CMD_EXEC {
        let line = native_text(payload, 65_536)?;
        let out = sess.execute(line);
        let _encode = span(Stage::Encode);
        let status = match out.action {
            Action::Exit => ST_BYE,
            _ if out.has_err() => ST_ERR,
            _ => ST_OK,
        };
        let body = out
            .lines
            .iter()
            .map(|(error, line)| format!("{}{line}", if *error { '-' } else { '+' }))
            .collect::<Vec<_>>()
            .join("\n");
        return Ok((status, body.into_bytes()));
    }
    let inst = sess.inst.clone();
    let response = match cmd {
        CMD_GET => {
            let decode = span(Stage::Decode);
            let key = native_text(payload, crate::format::MAX_KEY_LEN)?;
            drop(decode);
            let _permit = inst.native_admission.enter(Class::Read)?;
            let value =
                sess.with_managed_key("get", Need::Read, |sess| Ok(sess.kv_get(key)?.map(|(value, _)| value)))?;
            let _encode = span(Stage::Encode);
            match value {
                Some(value) => {
                    let tag = value.ty() as u8;
                    let mut bytes = match value {
                        crate::Value::Str(text) | crate::Value::Num(text) => text.into_bytes(),
                        crate::Value::Bytes(bytes) => bytes,
                        value => value.encode_stored(false)?.0,
                    };
                    bytes.insert(0, tag);
                    ok(bytes)
                }
                None => Ok((ST_NOT_FOUND, Vec::new())),
            }
        }
        CMD_SET => {
            let _permit = inst.native_admission.enter(Class::Data)?;
            sess.with_managed_key("set", Need::Write, |sess| {
                binary_set(sess, payload)?;
                ok(b"OK".to_vec())
            })
        }
        CMD_DEL => {
            let key = native_text(payload, crate::format::MAX_KEY_LEN)?;
            let _permit = inst.native_admission.enter(Class::Data)?;
            sess.with_managed_key("del", Need::Write, |sess| {
                Ok(if sess.kv_del(key)? { (ST_OK, b"1".to_vec()) } else { (ST_NOT_FOUND, b"0".to_vec()) })
            })
        }
        CMD_API => {
            // Parsing JSON can allocate substantially more than its frame.
            // Keep decoding, dispatch and serialization under data admission.
            let _permit = inst.native_admission.enter(Class::Data)?;
            let decode = span(Stage::Decode);
            let request = crate::num::from_slice(payload).map_err(KivaError::Invalid)?;
            drop(decode);
            let result = crate::api::handle_native_hierarchy(sess, &request)?;
            let _encode = span(Stage::Encode);
            ok(result.to_string().into_bytes())
        }
        _ => Err(KivaError::Denied("command unavailable in hierarchical TCP mode".into())),
    };
    if response.is_err() && matches!(cmd, CMD_SET | CMD_DEL) {
        sess.rollback();
    }
    response
}

fn native_text(payload: &[u8], max: usize) -> Result<&str> {
    if payload.len() > max {
        return Err(KivaError::Invalid("native payload is too large".into()));
    }
    std::str::from_utf8(payload).map_err(|_| KivaError::Invalid("native payload must be valid UTF-8".into()))
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use crate::client::Client;
    use crate::{Options, Value};

    #[test]
    fn network_data_backpressure_preserves_reads_controls_and_transactions() {
        let home =
            std::env::temp_dir().join(format!("kiva-native-admission-{:016x}", crate::util::random_u64().unwrap()));
        {
            let inst = Instance::open(&home, Options::default()).unwrap();
            let mut session = Session::new(inst.clone(), true, None);
            for command in [
                "modify user kiva password test-pass",
                "create user reader identified by reader-pass role editor",
                "config set server.threads 1",
                "config set server.native_queue 0",
                "config set server.native_metrics true",
                "config set auth_required true",
            ] {
                assert!(!session.execute(command).has_err(), "{command}");
            }
            session.login("kiva", "test-pass").unwrap();
            for command in [
                "hierarchy data",
                "project create app",
                "use app",
                "collection create items",
                "use collection items",
                "set sample 42",
                "hierarchy grant reader read instance",
            ] {
                let result = session.execute(command);
                assert!(!result.has_err(), "{command}: {}", result.text());
            }
            drop(session);
            inst.shutdown();
        }

        let inst = Instance::open_hierarchical(&home, Options::default()).unwrap();
        let port = start_tcp(&inst, "127.0.0.1", 0).unwrap();
        let connect = |user, password| {
            let mut client = Client::connect(("127.0.0.1", port)).unwrap();
            client.negotiate_hierarchy().unwrap();
            client.auth(user, password).unwrap();
            client.exec("use collection default.app.items").unwrap();
            client
        };
        let mut admin = connect("kiva", "test-pass");
        let mut reader = connect("reader", "reader-pass");
        admin.exec("begin").unwrap();
        admin.set("pending", &Value::Num("7".into()), 0).unwrap();
        let held = inst.native_admission.enter(Class::Data).unwrap();
        let refused = admin.set("refused", &Value::Num("9".into()), 0).unwrap_err();
        assert!(refused.to_string().starts_with("ERR_SERVER_BUSY"));
        assert_eq!(reader.get("sample").unwrap(), Some(Value::Num("42".into())));
        assert!(reader.exec("hierarchy timings").is_err());
        assert!(admin.exec("rollback").is_ok());
        // Model an operation already admitted by the shared guard. The
        // remote revoke takes the turnstile before waiting for that operation.
        let admitted = inst.managed_layout_lock.read().unwrap();
        std::thread::scope(|scope| {
            let revoke = scope.spawn(|| admin.exec("hierarchy revoke reader read instance"));
            let until = Instant::now() + Duration::from_secs(5);
            while inst.managed_layout_turnstile.try_lock().is_ok() {
                assert!(Instant::now() < until, "revocation did not reach the turnstile");
                std::thread::yield_now();
            }
            let late_read = scope.spawn(|| reader.get("sample"));
            drop(admitted);
            revoke.join().unwrap().unwrap();
            assert!(late_read.join().unwrap().is_err());
        });
        assert!(reader.get("sample").is_err());
        drop(held);
        assert_eq!(admin.get("refused").unwrap(), None);
        assert_eq!(admin.get("pending").unwrap(), None);
        assert!(admin.exec("hierarchy timings").unwrap().contains("get queue"));
        drop(reader);
        drop(admin);
        inst.shutdown();
        let _ = TcpStream::connect(("127.0.0.1", port));
        let _ = std::fs::remove_dir_all(home);
    }
}
