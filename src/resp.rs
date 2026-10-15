// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Redis-compatible listener (RESP2 protocol, port 6379 by default).
//!
//! Existing Redis clients and libraries can use KivaDB without code changes
//! for the common commands: strings and counters, expiry, hashes (stored as
//! KivaDB objects), lists (stored as KivaDB arrays), key listing and
//! MULTI/EXEC transactions. Every command goes through a normal session, so
//! users, roles, workspaces and replication rules apply exactly as elsewhere.
//!
//! How Redis types map to KivaDB types:
//!
//! | Redis  | KivaDB                                   |
//! |--------|------------------------------------------|
//! | string | string (UTF-8) or bytes; INCR -> number  |
//! | hash   | object, field values as strings          |
//! | list   | array of strings                         |

use crate::engine::BatchOp;
use crate::error::{KivaError, Result};
use crate::instance::{Instance, VERSION};
use crate::session::{Meta, Need, Session};
use crate::util::{glob_match, now_secs};
use crate::value::{fmt_f64, Type, Value};
use serde_json::{Map, Value as Json};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ARGS: usize = 1024 * 1024;
const MAX_BULK: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Simple(String),
    Error(String),
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Reply>>),
}

impl Reply {
    fn ok() -> Reply {
        Reply::Simple("OK".into())
    }

    fn nil() -> Reply {
        Reply::Bulk(None)
    }

    fn bulk(b: impl Into<Vec<u8>>) -> Reply {
        Reply::Bulk(Some(b.into()))
    }

    fn err(msg: impl Into<String>) -> Reply {
        Reply::Error(format!("ERR {}", msg.into()))
    }

    fn wrong_type() -> Reply {
        Reply::Error("WRONGTYPE Operation against a key holding the wrong kind of value".into())
    }

    fn array(items: Vec<Reply>) -> Reply {
        Reply::Array(Some(items))
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Reply::Simple(s) => {
                out.push(b'+');
                out.extend_from_slice(s.replace(['\r', '\n'], " ").as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            Reply::Error(s) => {
                out.push(b'-');
                out.extend_from_slice(s.replace(['\r', '\n'], " ").as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            Reply::Int(i) => out.extend_from_slice(format!(":{i}\r\n").as_bytes()),
            Reply::Bulk(None) => out.extend_from_slice(b"$-1\r\n"),
            Reply::Bulk(Some(b)) => {
                out.extend_from_slice(format!("${}\r\n", b.len()).as_bytes());
                out.extend_from_slice(b);
                out.extend_from_slice(b"\r\n");
            }
            Reply::Array(None) => out.extend_from_slice(b"*-1\r\n"),
            Reply::Array(Some(items)) => {
                out.extend_from_slice(format!("*{}\r\n", items.len()).as_bytes());
                for i in items {
                    i.encode(out);
                }
            }
        }
    }
}

impl From<KivaError> for Reply {
    fn from(e: KivaError) -> Reply {
        match e {
            KivaError::Denied(m) if m.starts_with("authentication required") => {
                Reply::Error("NOAUTH Authentication required.".into())
            }
            KivaError::Denied(m) => Reply::Error(format!("NOPERM {m}")),
            KivaError::ReadOnly(m) => Reply::Error(format!("READONLY {m}")),
            KivaError::Conflict(m) => Reply::Error(format!("EXECABORT {m}")),
            other => Reply::err(other.to_string()),
        }
    }
}

type Res = std::result::Result<Reply, Reply>;

/// Starts the Redis-compatible listener and returns the bound port.
pub fn start_resp(inst: &Arc<Instance>, bind: &str, port: u16) -> Result<u16> {
    let listener = TcpListener::bind((bind, port))
        .map_err(|e| KivaError::Other(format!("cannot listen on {bind}:{port}: {e}")))?;
    let port = listener.local_addr()?.port();
    let inst = inst.clone();
    std::thread::Builder::new().name("kiva-resp".into()).spawn(move || {
        for conn in listener.incoming() {
            if inst.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = conn else { continue };
            let inst = inst.clone();
            let _ = std::thread::Builder::new().name("kiva-resp-conn".into()).spawn(move || {
                let max = inst.cfg().get_u64("server", "max_clients") as usize;
                if inst.clients.fetch_add(1, Ordering::SeqCst) >= max {
                    let mut s = stream;
                    let _ = s.write_all(b"-ERR max number of clients reached\r\n");
                } else {
                    serve(&inst, stream);
                }
                inst.clients.fetch_sub(1, Ordering::SeqCst);
            });
        }
    })?;
    Ok(port)
}

fn read_line(r: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    if r.read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    while line.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        line.pop();
    }
    Ok(Some(line))
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Reads one command: a RESP array of bulk strings, or an inline command
/// line (`PING`, as typed in telnet). `None` at end of stream.
pub fn read_command(r: &mut impl BufRead) -> io::Result<Option<Vec<Vec<u8>>>> {
    loop {
        let first = match r.fill_buf()? {
            [] => return Ok(None),
            b => b[0],
        };
        if first != b'*' {
            let Some(line) = read_line(r)? else { return Ok(None) };
            let args: Vec<Vec<u8>> =
                line.split(|b| b.is_ascii_whitespace()).filter(|a| !a.is_empty()).map(|a| a.to_vec()).collect();
            if args.is_empty() {
                continue;
            }
            return Ok(Some(args));
        }
        let header = read_line(r)?.ok_or_else(|| bad("truncated command"))?;
        let n: usize =
            std::str::from_utf8(&header[1..]).ok().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad array"))?;
        if n > MAX_ARGS {
            return Err(bad("too many arguments"));
        }
        let mut args = Vec::with_capacity(n);
        for _ in 0..n {
            let h = read_line(r)?.ok_or_else(|| bad("truncated command"))?;
            if h.first() != Some(&b'$') {
                return Err(bad("expected a bulk string"));
            }
            let len: usize =
                std::str::from_utf8(&h[1..]).ok().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad length"))?;
            if len > MAX_BULK {
                return Err(bad("argument too large"));
            }
            let mut buf = vec![0u8; len + 2];
            r.read_exact(&mut buf)?;
            buf.truncate(len);
            args.push(buf);
        }
        if args.is_empty() {
            continue;
        }
        return Ok(Some(args));
    }
}

fn serve(inst: &Arc<Instance>, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let local = stream.peer_addr().map(|p| p.ip().is_loopback()).unwrap_or(false);
    let Ok(write_half) = stream.try_clone() else { return };
    let mut writer = io::BufWriter::new(write_half);
    let mut reader = BufReader::new(stream);
    let mut conn = Conn { sess: Session::new(inst.clone(), local, None), queued: None };
    let mut out = Vec::new();
    loop {
        let args = match read_command(&mut reader) {
            Ok(Some(a)) => a,
            Ok(None) => break,
            Err(e) => {
                let _ = writer.write_all(format!("-ERR Protocol error: {e}\r\n").as_bytes());
                let _ = writer.flush();
                break;
            }
        };
        let quit = args[0].eq_ignore_ascii_case(b"QUIT");
        out.clear();
        conn.handle(&args).encode(&mut out);
        if writer.write_all(&out).is_err() {
            break;
        }
        // Pipelined commands are answered together.
        if reader.buffer().is_empty() && writer.flush().is_err() {
            break;
        }
        if quit {
            let _ = writer.flush();
            break;
        }
    }
}

pub struct Conn {
    pub sess: Session,
    /// Commands collected between MULTI and EXEC.
    queued: Option<Vec<Vec<Vec<u8>>>>,
}

fn text(arg: &[u8]) -> std::result::Result<String, Reply> {
    String::from_utf8(arg.to_vec()).map_err(|_| Reply::err("arguments must be valid UTF-8 text"))
}

fn key(arg: &[u8]) -> std::result::Result<String, Reply> {
    let k = text(arg)?;
    if k.is_empty() || k.len() > crate::format::MAX_KEY_LEN || k.chars().any(|c| c.is_control()) {
        return Err(Reply::err("invalid key: KivaDB keys are 1 to 1024 bytes of text without control characters"));
    }
    Ok(k)
}

fn int(arg: &[u8]) -> std::result::Result<i64, Reply> {
    std::str::from_utf8(arg)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Reply::err("value is not an integer or out of range"))
}

fn float(arg: &[u8]) -> std::result::Result<f64, Reply> {
    std::str::from_utf8(arg)
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|f| f.is_finite())
        .ok_or_else(|| Reply::err("value is not a valid float"))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A Redis string as a KivaDB value: text when it is UTF-8, bytes otherwise.
fn value_of(bytes: &[u8]) -> Value {
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => Value::Str(s),
        Err(e) => Value::Bytes(e.into_bytes()),
    }
}

/// A KivaDB value as a Redis string; documents are hashes or lists.
fn bytes_of(v: &Value) -> Option<Vec<u8>> {
    Some(match v {
        Value::Str(s) | Value::Num(s) => s.as_bytes().to_vec(),
        Value::Bool(b) => b.to_string().into_bytes(),
        Value::Null => Vec::new(),
        Value::Bytes(b) => b.clone(),
        Value::Doc(_) => return None,
    })
}

fn json_text(j: &Json) -> Vec<u8> {
    match j {
        Json::String(s) => s.as_bytes().to_vec(),
        other => other.to_string().into_bytes(),
    }
}

fn arity(args: &[Vec<u8>], min: usize) -> std::result::Result<(), Reply> {
    if args.len() < min {
        let name = String::from_utf8_lossy(&args[0]).to_ascii_lowercase();
        return Err(Reply::err(format!("wrong number of arguments for '{name}' command")));
    }
    Ok(())
}

impl Conn {
    pub fn new(sess: Session) -> Conn {
        Conn { sess, queued: None }
    }

    pub fn handle(&mut self, args: &[Vec<u8>]) -> Reply {
        let name = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
        // Inside MULTI, commands are queued until EXEC.
        if let Some(queue) = self.queued.as_mut() {
            if !matches!(name.as_str(), "EXEC" | "DISCARD" | "MULTI" | "QUIT") {
                queue.push(args.to_vec());
                return Reply::Simple("QUEUED".into());
            }
        }
        if self.sess.user.is_none() && !matches!(name.as_str(), "AUTH" | "HELLO" | "PING" | "QUIT" | "ECHO") {
            return Reply::Error("NOAUTH Authentication required.".into());
        }
        self.run(&name, args).unwrap_or_else(|e| e)
    }

    fn get(&mut self, k: &str) -> std::result::Result<Option<(Value, Meta)>, Reply> {
        Ok(self.sess.kv_get(k)?)
    }

    fn put(&mut self, k: &str, v: Value, expires_at: i64) -> std::result::Result<(), Reply> {
        Ok(self.sess.kv_put(k, v, expires_at, false)?)
    }

    fn doc(&mut self, k: &str, want: Type) -> std::result::Result<Option<(Json, Meta)>, Reply> {
        match self.get(k)? {
            None => Ok(None),
            Some((Value::Doc(j), m)) if Value::Doc(j.clone()).ty() == want => Ok(Some((j, m))),
            Some(_) => Err(Reply::wrong_type()),
        }
    }

    fn run(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "PING" => Ok(match args.get(1) {
                Some(m) => Reply::bulk(m.clone()),
                None => Reply::Simple("PONG".into()),
            }),
            "ECHO" => {
                arity(args, 2)?;
                Ok(Reply::bulk(args[1].clone()))
            }
            "QUIT" => Ok(Reply::ok()),
            "AUTH" => {
                arity(args, 2)?;
                let (user, pass) = if args.len() >= 3 {
                    (text(&args[1])?, text(&args[2])?)
                } else {
                    (crate::auth::DEFAULT_ADMIN.to_string(), text(&args[1])?)
                };
                self.login(&user, &pass)?;
                Ok(Reply::ok())
            }
            "HELLO" => self.hello(args),
            "CLIENT" => {
                arity(args, 2)?;
                match String::from_utf8_lossy(&args[1]).to_ascii_uppercase().as_str() {
                    "GETNAME" => Ok(Reply::nil()),
                    "ID" => Ok(Reply::Int(self.sess.id as i64)),
                    _ => Ok(Reply::ok()),
                }
            }
            "SELECT" => {
                arity(args, 2)?;
                if int(&args[1])? == 0 {
                    Ok(Reply::ok())
                } else {
                    Err(Reply::err(
                        "KivaDB uses named workspaces; only database 0 (your current workspace) exists here",
                    ))
                }
            }
            "COMMAND" => Ok(match args.get(1).map(|a| String::from_utf8_lossy(a).to_ascii_uppercase()) {
                Some(s) if s == "COUNT" => Reply::Int(0),
                _ => Reply::array(Vec::new()),
            }),
            "INFO" => Ok(Reply::bulk(self.info())),
            "DBSIZE" => {
                self.sess.check(Need::Read)?;
                Ok(Reply::Int(self.sess.engine.len() as i64))
            }
            "FLUSHDB" | "FLUSHALL" => {
                self.sess.check(Need::Admin)?;
                self.sess.writable()?;
                self.sess.engine.reset()?;
                Ok(Reply::ok())
            }
            "MULTI" => {
                if self.queued.is_some() {
                    return Err(Reply::err("MULTI calls can not be nested"));
                }
                self.queued = Some(Vec::new());
                Ok(Reply::ok())
            }
            "DISCARD" => match self.queued.take() {
                Some(_) => Ok(Reply::ok()),
                None => Err(Reply::err("DISCARD without MULTI")),
            },
            "EXEC" => self.exec(),
            "WATCH" => Err(Reply::err("WATCH is not supported: KivaDB checks conflicts at EXEC time instead")),
            "UNWATCH" => Ok(Reply::ok()),
            "SUBSCRIBE" | "PSUBSCRIBE" | "PUBLISH" => Err(Reply::err(
                "publish/subscribe is not supported on this port; use 'subscribe' on port 2911, /v1/stream or /ws",
            )),

            // -- strings ----------------------------------------------------
            "GET" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                match self.get(&k)? {
                    None => Ok(Reply::nil()),
                    Some((v, _)) => bytes_of(&v).map(Reply::bulk).ok_or_else(Reply::wrong_type),
                }
            }
            "SET" => self.set(args),
            "SETNX" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                if self.sess.kv_exists(&k)? {
                    return Ok(Reply::Int(0));
                }
                self.put(&k, value_of(&args[2]), 0)?;
                Ok(Reply::Int(1))
            }
            "SETEX" | "PSETEX" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let n = int(&args[2])?;
                if n <= 0 {
                    return Err(Reply::err(format!("invalid expire time in '{}' command", name.to_ascii_lowercase())));
                }
                let exp = if name == "SETEX" { now_secs() + n } else { (now_ms() + n + 999) / 1000 };
                self.put(&k, value_of(&args[3]), exp)?;
                Ok(Reply::ok())
            }
            "GETSET" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let old = self.get(&k)?;
                let old = match old {
                    None => None,
                    Some((v, _)) => Some(bytes_of(&v).ok_or_else(Reply::wrong_type)?),
                };
                self.put(&k, value_of(&args[2]), 0)?;
                Ok(Reply::Bulk(old))
            }
            "GETDEL" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                match self.get(&k)? {
                    None => Ok(Reply::nil()),
                    Some((v, _)) => {
                        let b = bytes_of(&v).ok_or_else(Reply::wrong_type)?;
                        self.sess.kv_del(&k)?;
                        Ok(Reply::bulk(b))
                    }
                }
            }
            "MGET" => {
                arity(args, 2)?;
                let mut items = Vec::new();
                for a in &args[1..] {
                    let k = key(a)?;
                    items.push(match self.get(&k)? {
                        Some((v, _)) => Reply::Bulk(bytes_of(&v)),
                        None => Reply::nil(),
                    });
                }
                Ok(Reply::array(items))
            }
            "MSET" => {
                if args.len() < 3 || args.len().is_multiple_of(2) {
                    return Err(Reply::err("wrong number of arguments for 'mset' command"));
                }
                let mut ops = Vec::new();
                for pair in args[1..].chunks(2) {
                    ops.push(BatchOp::Put {
                        key: key(&pair[0])?,
                        value: value_of(&pair[1]),
                        expires_at: 0,
                        msgpack: false,
                    });
                }
                self.sess.apply(ops)?;
                Ok(Reply::ok())
            }
            "APPEND" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let (mut cur, exp) = match self.get(&k)? {
                    None => (Vec::new(), 0),
                    Some((v, m)) => (bytes_of(&v).ok_or_else(Reply::wrong_type)?, m.expires_at),
                };
                cur.extend_from_slice(&args[2]);
                let len = cur.len();
                self.put(&k, value_of(&cur), exp)?;
                Ok(Reply::Int(len as i64))
            }
            "STRLEN" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                match self.get(&k)? {
                    None => Ok(Reply::Int(0)),
                    Some((v, _)) => Ok(Reply::Int(bytes_of(&v).ok_or_else(Reply::wrong_type)?.len() as i64)),
                }
            }
            "INCR" | "DECR" | "INCRBY" | "DECRBY" => {
                arity(args, if name.ends_with("BY") { 3 } else { 2 })?;
                let k = key(&args[1])?;
                let step = match name {
                    "INCR" => 1,
                    "DECR" => -1,
                    "INCRBY" => int(&args[2])?,
                    _ => int(&args[2])?.checked_neg().ok_or_else(|| Reply::err("decrement would overflow"))?,
                };
                let (cur, exp) = self.counter(&k)?;
                let next = cur.checked_add(step).ok_or_else(|| Reply::err("increment or decrement would overflow"))?;
                self.put(&k, Value::Num(next.to_string()), exp)?;
                Ok(Reply::Int(next))
            }
            "INCRBYFLOAT" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let step = float(&args[2])?;
                let (cur, exp) = match self.get(&k)? {
                    None => (0.0, 0),
                    Some((v, m)) => {
                        let b = bytes_of(&v).ok_or_else(Reply::wrong_type)?;
                        (float(&b).map_err(|_| Reply::err("value is not a valid float"))?, m.expires_at)
                    }
                };
                let next = cur + step;
                if !next.is_finite() {
                    return Err(Reply::err("increment would produce NaN or Infinity"));
                }
                let text = fmt_f64(next);
                self.put(&k, Value::Num(text.clone()), exp)?;
                Ok(Reply::bulk(text))
            }

            // -- keys -------------------------------------------------------
            "DEL" | "UNLINK" => {
                arity(args, 2)?;
                let mut n = 0;
                for a in &args[1..] {
                    if self.sess.kv_del(&key(a)?)? {
                        n += 1;
                    }
                }
                Ok(Reply::Int(n))
            }
            "EXISTS" => {
                arity(args, 2)?;
                let mut n = 0;
                for a in &args[1..] {
                    if self.sess.kv_exists(&key(a)?)? {
                        n += 1;
                    }
                }
                Ok(Reply::Int(n))
            }
            "TYPE" => {
                arity(args, 2)?;
                let t = match self.sess.kv_type(&key(&args[1])?)? {
                    None => "none",
                    Some(Type::Object) => "hash",
                    Some(Type::Array) => "list",
                    Some(_) => "string",
                };
                Ok(Reply::Simple(t.into()))
            }
            "KEYS" => {
                arity(args, 2)?;
                self.sess.check(Need::Read)?;
                let pattern = text(&args[1])?;
                let keys = self.sess.engine.keys_matching(&pattern, 0);
                Ok(Reply::array(keys.into_iter().map(|(k, _)| Reply::bulk(k)).collect()))
            }
            "SCAN" => self.scan(args),
            "RENAME" => {
                arity(args, 3)?;
                let (from, to) = (key(&args[1])?, key(&args[2])?);
                let Some((v, m)) = self.get(&from)? else {
                    return Err(Reply::err("no such key"));
                };
                if from != to {
                    self.sess.apply(vec![
                        BatchOp::Put { key: to, value: v, expires_at: m.expires_at, msgpack: m.msgpack },
                        BatchOp::Del { key: from },
                    ])?;
                }
                Ok(Reply::ok())
            }
            "EXPIRE" | "PEXPIRE" | "EXPIREAT" | "PEXPIREAT" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let n = int(&args[2])?;
                let exp = match name {
                    "EXPIRE" => now_secs().saturating_add(n),
                    "PEXPIRE" => (now_ms().saturating_add(n) + 999) / 1000,
                    "EXPIREAT" => n,
                    _ => (n + 999) / 1000,
                };
                let Some((v, m)) = self.get(&k)? else { return Ok(Reply::Int(0)) };
                if exp <= now_secs() {
                    self.sess.kv_del(&k)?;
                } else {
                    self.sess.kv_put(&k, v, exp, m.msgpack)?;
                }
                Ok(Reply::Int(1))
            }
            "PERSIST" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                match self.get(&k)? {
                    Some((v, m)) if m.expires_at > 0 => {
                        self.sess.kv_put(&k, v, 0, m.msgpack)?;
                        Ok(Reply::Int(1))
                    }
                    _ => Ok(Reply::Int(0)),
                }
            }
            "TTL" | "PTTL" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                Ok(Reply::Int(match self.get(&k)? {
                    None => -2,
                    Some((_, m)) if m.expires_at == 0 => -1,
                    Some((_, m)) => {
                        let secs = (m.expires_at - now_secs()).max(0);
                        if name == "TTL" {
                            secs
                        } else {
                            (m.expires_at * 1000 - now_ms()).max(0)
                        }
                    }
                }))
            }

            // -- hashes (KivaDB objects) ------------------------------------
            "HSET" | "HMSET" => {
                if args.len() < 4 || !args.len().is_multiple_of(2) {
                    return Err(Reply::err(format!(
                        "wrong number of arguments for '{}' command",
                        name.to_ascii_lowercase()
                    )));
                }
                let k = key(&args[1])?;
                let (mut obj, exp) = match self.doc(&k, Type::Object)? {
                    Some((Json::Object(m), meta)) => (m, meta.expires_at),
                    _ => (Map::new(), 0),
                };
                let mut added = 0;
                for pair in args[2..].chunks(2) {
                    if obj.insert(text(&pair[0])?, Json::String(text(&pair[1])?)).is_none() {
                        added += 1;
                    }
                }
                self.put(&k, Value::Doc(Json::Object(obj)), exp)?;
                Ok(if name == "HSET" { Reply::Int(added) } else { Reply::ok() })
            }
            "HSETNX" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let (mut obj, exp) = match self.doc(&k, Type::Object)? {
                    Some((Json::Object(m), meta)) => (m, meta.expires_at),
                    _ => (Map::new(), 0),
                };
                let f = text(&args[2])?;
                if obj.contains_key(&f) {
                    return Ok(Reply::Int(0));
                }
                obj.insert(f, Json::String(text(&args[3])?));
                self.put(&k, Value::Doc(Json::Object(obj)), exp)?;
                Ok(Reply::Int(1))
            }
            "HGET" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let f = text(&args[2])?;
                Ok(match self.doc(&k, Type::Object)? {
                    Some((j, _)) => Reply::Bulk(j.get(&f).map(json_text)),
                    None => Reply::nil(),
                })
            }
            "HMGET" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let obj = self.doc(&k, Type::Object)?.map(|(j, _)| j);
                let mut items = Vec::new();
                for f in &args[2..] {
                    let f = text(f)?;
                    items.push(Reply::Bulk(obj.as_ref().and_then(|j| j.get(&f)).map(json_text)));
                }
                Ok(Reply::array(items))
            }
            "HGETALL" | "HKEYS" | "HVALS" | "HLEN" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let obj = match self.doc(&k, Type::Object)? {
                    Some((Json::Object(m), _)) => m,
                    _ => Map::new(),
                };
                Ok(match name {
                    "HLEN" => Reply::Int(obj.len() as i64),
                    "HKEYS" => Reply::array(obj.keys().map(|f| Reply::bulk(f.clone())).collect()),
                    "HVALS" => Reply::array(obj.values().map(|v| Reply::bulk(json_text(v))).collect()),
                    _ => Reply::array(
                        obj.iter().flat_map(|(f, v)| [Reply::bulk(f.clone()), Reply::bulk(json_text(v))]).collect(),
                    ),
                })
            }
            "HEXISTS" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let f = text(&args[2])?;
                let has = self.doc(&k, Type::Object)?.is_some_and(|(j, _)| j.get(&f).is_some());
                Ok(Reply::Int(has as i64))
            }
            "HDEL" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let Some((Json::Object(mut obj), meta)) = self.doc(&k, Type::Object)? else {
                    return Ok(Reply::Int(0));
                };
                let mut n = 0;
                for f in &args[2..] {
                    if obj.shift_remove(&text(f)?).is_some() {
                        n += 1;
                    }
                }
                // Like Redis, an empty hash disappears.
                if obj.is_empty() {
                    self.sess.kv_del(&k)?;
                } else if n > 0 {
                    self.put(&k, Value::Doc(Json::Object(obj)), meta.expires_at)?;
                }
                Ok(Reply::Int(n))
            }
            "HINCRBY" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let f = text(&args[2])?;
                let step = int(&args[3])?;
                let (mut obj, exp) = match self.doc(&k, Type::Object)? {
                    Some((Json::Object(m), meta)) => (m, meta.expires_at),
                    _ => (Map::new(), 0),
                };
                let cur = match obj.get(&f) {
                    None => 0,
                    Some(v) => match v {
                        Json::Number(n) => n.as_i64(),
                        Json::String(s) => s.parse().ok(),
                        _ => None,
                    }
                    .ok_or_else(|| Reply::err("hash value is not an integer"))?,
                };
                let next = cur.checked_add(step).ok_or_else(|| Reply::err("increment or decrement would overflow"))?;
                obj.insert(f, Json::String(next.to_string()));
                self.put(&k, Value::Doc(Json::Object(obj)), exp)?;
                Ok(Reply::Int(next))
            }

            // -- lists (KivaDB arrays) --------------------------------------
            "LPUSH" | "RPUSH" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let (mut list, exp) = match self.doc(&k, Type::Array)? {
                    Some((Json::Array(a), meta)) => (a, meta.expires_at),
                    _ => (Vec::new(), 0),
                };
                for v in &args[2..] {
                    let item = Json::String(text(v)?);
                    if name == "LPUSH" {
                        list.insert(0, item);
                    } else {
                        list.push(item);
                    }
                }
                let len = list.len();
                self.put(&k, Value::Doc(Json::Array(list)), exp)?;
                Ok(Reply::Int(len as i64))
            }
            "LPOP" | "RPOP" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let count = match args.get(2) {
                    Some(c) => Some(int(c)?.max(0) as usize),
                    None => None,
                };
                let Some((Json::Array(mut list), meta)) = self.doc(&k, Type::Array)? else {
                    return Ok(if count.is_some() { Reply::Array(None) } else { Reply::nil() });
                };
                let take = count.unwrap_or(1).min(list.len());
                let popped: Vec<Json> = if name == "LPOP" {
                    list.drain(..take).collect()
                } else {
                    let at = list.len() - take;
                    list.drain(at..).rev().collect()
                };
                if list.is_empty() {
                    self.sess.kv_del(&k)?;
                } else {
                    self.put(&k, Value::Doc(Json::Array(list)), meta.expires_at)?;
                }
                Ok(match count {
                    None => Reply::Bulk(popped.first().map(json_text)),
                    Some(_) => Reply::array(popped.iter().map(|v| Reply::bulk(json_text(v))).collect()),
                })
            }
            "LLEN" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                Ok(Reply::Int(match self.doc(&k, Type::Array)? {
                    Some((Json::Array(a), _)) => a.len() as i64,
                    _ => 0,
                }))
            }
            "LRANGE" | "LINDEX" => {
                arity(args, if name == "LRANGE" { 4 } else { 3 })?;
                let k = key(&args[1])?;
                let list = match self.doc(&k, Type::Array)? {
                    Some((Json::Array(a), _)) => a,
                    _ => Vec::new(),
                };
                let len = list.len() as i64;
                let norm = |i: i64| if i < 0 { len + i } else { i };
                if name == "LINDEX" {
                    let i = norm(int(&args[2])?);
                    return Ok(Reply::Bulk(if (0..len).contains(&i) {
                        Some(json_text(&list[i as usize]))
                    } else {
                        None
                    }));
                }
                let start = norm(int(&args[2])?).max(0);
                let stop = norm(int(&args[3])?).min(len - 1);
                if start > stop || start >= len {
                    return Ok(Reply::array(Vec::new()));
                }
                Ok(Reply::array(
                    list[start as usize..=stop as usize].iter().map(|v| Reply::bulk(json_text(v))).collect(),
                ))
            }
            _ => {
                let first = args.get(1).map(|a| format!("'{}' ", String::from_utf8_lossy(a))).unwrap_or_default();
                Err(Reply::err(format!(
                    "unknown command '{}', with args beginning with: {first}",
                    String::from_utf8_lossy(&args[0])
                )))
            }
        }
    }

    fn login(&mut self, user: &str, pass: &str) -> std::result::Result<(), Reply> {
        // Redis calls its built-in account "default".
        let user = if user == "default" { crate::auth::DEFAULT_ADMIN } else { user };
        self.sess
            .login(user, pass)
            .map(|_| ())
            .map_err(|_| Reply::Error("WRONGPASS invalid username-password pair or user is disabled.".into()))
    }

    fn hello(&mut self, args: &[Vec<u8>]) -> Res {
        let mut i = 1;
        if let Some(v) = args.get(1) {
            if int(v)? != 2 {
                return Err(Reply::Error("NOPROTO this server speaks RESP2 only".into()));
            }
            i = 2;
        }
        while i < args.len() {
            match String::from_utf8_lossy(&args[i]).to_ascii_uppercase().as_str() {
                "AUTH" if i + 2 < args.len() => {
                    let (u, p) = (text(&args[i + 1])?, text(&args[i + 2])?);
                    self.login(&u, &p)?;
                    i += 3;
                }
                "SETNAME" if i + 1 < args.len() => i += 2,
                _ => return Err(Reply::err("syntax error in HELLO")),
            }
        }
        if self.sess.user.is_none() {
            return Err(Reply::Error("NOAUTH HELLO must be called with the client already authenticated".into()));
        }
        let pairs: Vec<(&str, Reply)> = vec![
            ("server", Reply::bulk("kivadb")),
            ("version", Reply::bulk(VERSION)),
            ("proto", Reply::Int(2)),
            ("id", Reply::Int(self.sess.id as i64)),
            ("mode", Reply::bulk("standalone")),
            ("role", Reply::bulk(if self.sess.inst.repl.is_replica() { "replica" } else { "master" })),
            ("modules", Reply::array(Vec::new())),
        ];
        Ok(Reply::array(pairs.into_iter().flat_map(|(k, v)| [Reply::bulk(k), v]).collect()))
    }

    /// The integer held by `key` (0 when absent) and its expiry.
    fn counter(&mut self, k: &str) -> std::result::Result<(i64, i64), Reply> {
        match self.get(k)? {
            None => Ok((0, 0)),
            Some((v, m)) => {
                let b = bytes_of(&v).ok_or_else(Reply::wrong_type)?;
                let s = std::str::from_utf8(&b).map_err(|_| Reply::err("value is not an integer or out of range"))?;
                let n = s.parse::<i64>().map_err(|_| Reply::err("value is not an integer or out of range"))?;
                Ok((n, m.expires_at))
            }
        }
    }

    fn set(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 3)?;
        let k = key(&args[1])?;
        let value = value_of(&args[2]);
        let (mut nx, mut xx, mut keep, mut get) = (false, false, false, false);
        let mut exp: Option<i64> = None;
        let mut i = 3;
        while i < args.len() {
            let opt = String::from_utf8_lossy(&args[i]).to_ascii_uppercase();
            let next = |i: usize| args.get(i + 1).ok_or_else(|| Reply::err("syntax error"));
            match opt.as_str() {
                "NX" => nx = true,
                "XX" => xx = true,
                "KEEPTTL" => keep = true,
                "GET" => get = true,
                "EX" | "PX" | "EXAT" | "PXAT" => {
                    let n = int(next(i)?)?;
                    if n <= 0 {
                        return Err(Reply::err("invalid expire time in 'set' command"));
                    }
                    exp = Some(match opt.as_str() {
                        "EX" => now_secs() + n,
                        "PX" => (now_ms() + n + 999) / 1000,
                        "EXAT" => n,
                        _ => (n + 999) / 1000,
                    });
                    i += 1;
                }
                _ => return Err(Reply::err("syntax error")),
            }
            i += 1;
        }
        if (nx && xx) || (keep && exp.is_some()) {
            return Err(Reply::err("syntax error"));
        }
        let old = self.get(&k)?;
        let old_bytes = match (&old, get) {
            (Some((v, _)), true) => Some(bytes_of(v).ok_or_else(Reply::wrong_type)?),
            _ => None,
        };
        let skip = (nx && old.is_some()) || (xx && old.is_none());
        if !skip {
            let expires_at = match (exp, keep, &old) {
                (Some(e), _, _) => e,
                (None, true, Some((_, m))) => m.expires_at,
                _ => 0,
            };
            self.put(&k, value, expires_at)?;
        }
        Ok(match (get, skip) {
            (true, _) => Reply::Bulk(old_bytes),
            (false, true) => Reply::nil(),
            (false, false) => Reply::ok(),
        })
    }

    fn scan(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 2)?;
        self.sess.check(Need::Read)?;
        let cursor = usize::try_from(int(&args[1])?).map_err(|_| Reply::err("invalid cursor"))?;
        let (mut pattern, mut count, mut kind) = ("*".to_string(), 10usize, None::<String>);
        let mut i = 2;
        while i + 1 < args.len() {
            match String::from_utf8_lossy(&args[i]).to_ascii_uppercase().as_str() {
                "MATCH" => pattern = text(&args[i + 1])?,
                "COUNT" => count = int(&args[i + 1])?.clamp(1, 100_000) as usize,
                "TYPE" => kind = Some(text(&args[i + 1])?.to_ascii_lowercase()),
                _ => return Err(Reply::err("syntax error")),
            }
            i += 2;
        }
        // The cursor is a position in the sorted key list.
        let all = self.sess.engine.range(None, None, false, 0);
        let end = (cursor + count).min(all.len());
        let mut keys = Vec::new();
        for (k, e) in all.get(cursor..end).unwrap_or(&[]) {
            let t = match e.ty {
                Type::Object => "hash",
                Type::Array => "list",
                _ => "string",
            };
            if glob_match(&pattern, k) && kind.as_deref().is_none_or(|w| w == t) {
                keys.push(Reply::bulk(k.clone()));
            }
        }
        let next = if end >= all.len() { 0 } else { end };
        Ok(Reply::array(vec![Reply::bulk(next.to_string()), Reply::array(keys)]))
    }

    fn exec(&mut self) -> Res {
        let Some(queued) = self.queued.take() else {
            return Err(Reply::err("EXEC without MULTI"));
        };
        if self.sess.txn.is_some() {
            return Err(Reply::err("a transaction is already open on this session"));
        }
        let begun = self.sess.begin();
        let in_txn = begun.is_ok();
        let mut replies = Vec::with_capacity(queued.len());
        for args in &queued {
            let name = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
            let r = if matches!(name.as_str(), "FLUSHDB" | "FLUSHALL" | "MULTI" | "EXEC") {
                Reply::err(format!("{name} is not allowed inside MULTI"))
            } else {
                self.run(&name, args).unwrap_or_else(|e| e)
            };
            replies.push(r);
        }
        if in_txn {
            if let Err(e) = self.sess.commit() {
                return Ok(match e {
                    KivaError::Conflict(_) => Reply::Array(None),
                    other => Reply::from(other),
                });
            }
        }
        Ok(Reply::array(replies))
    }

    fn info(&self) -> Vec<u8> {
        let s = self.sess.engine.stats();
        let inst = &self.sess.inst;
        let role = if inst.repl.is_replica() { "slave" } else { "master" };
        let mut t = String::new();
        t.push_str("# Server\r\n");
        // Libraries read this field to decide which commands they may use.
        t.push_str("redis_version:7.2.0\r\n");
        t.push_str(&format!("kivadb_version:{VERSION}\r\nredis_mode:standalone\r\n"));
        t.push_str(&format!("uptime_in_seconds:{}\r\n", inst.started.elapsed().as_secs()));
        t.push_str(&format!("\r\n# Clients\r\nconnected_clients:{}\r\n", inst.clients.load(Ordering::SeqCst)));
        t.push_str(&format!("\r\n# Memory\r\nused_memory:{}\r\n", s.memory_bytes + s.cache_bytes));
        t.push_str(&format!("\r\n# Replication\r\nrole:{role}\r\nconnected_slaves:{}\r\n", inst.repl.replicas().len()));
        t.push_str(&format!("\r\n# Keyspace\r\ndb0:keys={},expires={},avg_ttl=0\r\n", s.keys, s.ttl_keys));
        t.into_bytes()
    }
}
