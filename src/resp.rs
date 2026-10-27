// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Redis-compatible listener (RESP2 and RESP3 protocols, port 6379 by default).
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
//! | set    | object `{"redis": "set", "members": [..]}` |
//! | zset   | object `{"redis": "zset", "members": {..}}` |
//!
//! Like Redis, the connections of an instance execute one command at a time,
//! so read-modify-write commands (INCR, HINCRBY, LPUSH...), MULTI/EXEC and
//! Lua scripts (EVAL) are atomic with respect to each other. Numbered
//! databases map to workspaces: 0 is the user's own, `SELECT n` (1 to 15)
//! uses the workspace `db<n>`.
//!
//! Publish/subscribe (`PUBLISH`, `SUBSCRIBE`, `PSUBSCRIBE`, `PUBSUB`) is
//! described in [`crate::pubsub`].

use crate::auth::Role;
use crate::engine::BatchOp;
use crate::error::{KivaError, Result};
use crate::instance::{Instance, VERSION};
use crate::pubsub::{Frame, Held, Outbox, OUTBOX_FRAMES};
use crate::session::{Meta, Need, Session};
use crate::util::{glob_match, now_ms};
use crate::value::{fmt_f64, Type, Value};
use mlua::{Lua, MultiValue, Table, Value as LVal};
use serde_json::{json, Map, Value as Json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

#[path = "resp_blocking.rs"]
mod blocking;
#[path = "resp_classic.rs"]
mod classic;
#[path = "resp_commands.rs"]
mod commands;
#[path = "resp_json.rs"]
mod json;
#[path = "resp_sets.rs"]
mod sets;
#[path = "resp_streams.rs"]
mod streams;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_ARGS: usize = 1024 * 1024;
const MAX_BULK: usize = 512 * 1024 * 1024;

/// Numbered databases accepted by SELECT, as in a default Redis.
const DATABASES: i64 = 16;

/// Commands a Lua script may not run through `redis.call`.
const NOT_IN_SCRIPTS: &[&str] = &[
    "AUTH",
    "HELLO",
    "QUIT",
    "MULTI",
    "EXEC",
    "DISCARD",
    "WATCH",
    "UNWATCH",
    "EVAL",
    "EVALSHA",
    "EVAL_RO",
    "EVALSHA_RO",
    "SCRIPT",
    "SELECT",
    "SUBSCRIBE",
    "PSUBSCRIBE",
    "UNSUBSCRIBE",
    "PUNSUBSCRIBE",
];

/// Commands a client in RESP2 may send while it listens to channels.
const WHILE_SUBSCRIBED: &[&str] = &["SUBSCRIBE", "PSUBSCRIBE", "UNSUBSCRIBE", "PUNSUBSCRIBE", "PING", "QUIT", "RESET"];

/// State shared by the Redis connections of one instance.
#[derive(Default)]
pub struct Shared {
    /// Held while a command runs: Redis executes one command at a time, and
    /// clients count on it (INCR from two connections never loses a step).
    turn: Mutex<()>,
    /// Lua scripts by SHA-1, for EVALSHA.
    scripts: Mutex<HashMap<String, String>>,
    /// Connections listening to channels.
    pub hub: crate::pubsub::Hub,
    /// Connections waiting in a blocking command.
    waiters: blocking::Waiters,
}

impl Shared {
    /// A write was committed somewhere: connections waiting for data look again.
    pub fn wake(&self) {
        self.waiters.wake();
    }

    fn script(&self, sha: &str) -> Option<String> {
        self.scripts.lock().unwrap_or_else(|e| e.into_inner()).get(&sha.to_ascii_lowercase()).cloned()
    }

    fn remember(&self, source: &str) -> String {
        let sha = sha1_hex(source.as_bytes());
        self.scripts.lock().unwrap_or_else(|e| e.into_inner()).insert(sha.clone(), source.to_string());
        sha
    }
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Simple(String),
    Error(String),
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Reply>>),
    /// Field-value pairs: a map in RESP3, a flat array in RESP2.
    Map(Vec<(Reply, Reply)>),
    /// A pub/sub message or notice: a push in RESP3, an array in RESP2.
    Push(Vec<Reply>),
    /// A floating-point number (scores): a double in RESP3, text in RESP2.
    Double(f64),
    /// Members of a set: a set in RESP3, an array in RESP2.
    Set(Vec<Reply>),
    /// Nothing to write: the command already queued its replies (SUBSCRIBE).
    Sent,
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

    /// Writes the reply in RESP2, or in RESP3 when `resp3` (after `HELLO 3`).
    pub fn encode(&self, out: &mut Vec<u8>, resp3: bool) {
        match self {
            Reply::Bulk(None) | Reply::Array(None) if resp3 => out.extend_from_slice(b"_\r\n"),
            Reply::Map(pairs) if resp3 => {
                out.extend_from_slice(format!("%{}\r\n", pairs.len()).as_bytes());
                for (k, v) in pairs {
                    k.encode(out, resp3);
                    v.encode(out, resp3);
                }
            }
            Reply::Push(items) => {
                let mark = if resp3 { '>' } else { '*' };
                out.extend_from_slice(format!("{mark}{}\r\n", items.len()).as_bytes());
                for i in items {
                    i.encode(out, resp3);
                }
            }
            Reply::Sent => {}
            Reply::Double(f) if resp3 => {
                let text = if f.is_nan() { "nan".to_string() } else { fmt_f64(*f) };
                out.extend_from_slice(format!(",{text}\r\n").as_bytes());
            }
            Reply::Double(f) => Reply::bulk(fmt_f64(*f)).encode(out, resp3),
            Reply::Set(items) => {
                let mark = if resp3 { '~' } else { '*' };
                out.extend_from_slice(format!("{mark}{}\r\n", items.len()).as_bytes());
                for i in items {
                    i.encode(out, resp3);
                }
            }
            Reply::Map(pairs) => {
                out.extend_from_slice(format!("*{}\r\n", pairs.len() * 2).as_bytes());
                for (k, v) in pairs {
                    k.encode(out, resp3);
                    v.encode(out, resp3);
                }
            }
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
                    i.encode(out, resp3);
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
            KivaError::Unique(m) => Reply::Error(format!("UNIQUE {m}")),
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
                    let refusal =
                        |_: u8| b"-ERR this server requires TLS from other machines (use rediss://)\r\n".to_vec();
                    if let Some((stream, peer)) = crate::tls::accept(&inst, stream, &refusal) {
                        serve(&inst, stream, peer.ip().is_loopback());
                    }
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

fn serve(inst: &Arc<Instance>, stream: TcpStream, local: bool) {
    let _ = stream.set_nodelay(true);
    let (Ok(write_half), Ok(control)) = (stream.try_clone(), stream.try_clone()) else { return };
    // Replies and published messages leave in order through the outbox.
    let (tx, rx) = mpsc::sync_channel(OUTBOX_FRAMES);
    let resp3 = Arc::new(AtomicBool::new(false));
    let shared = resp3.clone();
    let writer =
        std::thread::Builder::new().name("kiva-resp-out".into()).spawn(move || write_frames(write_half, rx, &shared));
    let Ok(writer) = writer else { return };
    let mut reader = BufReader::new(stream);
    let mut conn = Conn::new(Session::new(inst.clone(), local, None));
    conn.outbox = Some(Outbox { tx: tx.clone(), stream: control });
    loop {
        let args = match read_command(&mut reader) {
            Ok(Some(a)) => a,
            Ok(None) => break,
            Err(e) => {
                let _ = tx.send(Frame::Bytes(format!("-ERR Protocol error: {e}\r\n").into_bytes()));
                break;
            }
        };
        let quit = args[0].eq_ignore_ascii_case(b"QUIT");
        let mut out = Vec::new();
        conn.handle(&args).encode(&mut out, conn.resp3);
        resp3.store(conn.resp3, Ordering::SeqCst);
        if (!out.is_empty() && tx.send(Frame::Bytes(out)).is_err()) || quit {
            break;
        }
    }
    // Stop listening, then let the writer send what is left.
    drop(conn);
    drop(tx);
    let _ = writer.join();
}

/// Writes a connection's replies and messages, flushing when none is waiting
/// (pipelined commands are answered together).
fn write_frames(stream: TcpStream, rx: Receiver<Frame>, resp3: &AtomicBool) {
    let control = stream.try_clone();
    let mut w = io::BufWriter::new(stream);
    while let Ok(mut frame) = rx.recv() {
        loop {
            let bytes = match frame {
                Frame::Bytes(b) => b,
                Frame::Push(r) => {
                    let mut b = Vec::new();
                    r.encode(&mut b, resp3.load(Ordering::SeqCst));
                    b
                }
            };
            if w.write_all(&bytes).is_err() {
                // The client is gone: wake the reader up too.
                if let Ok(s) = &control {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
                return;
            }
            match rx.try_recv() {
                Ok(f) => frame = f,
                Err(_) => break,
            }
        }
        if w.flush().is_err() {
            return;
        }
    }
}

pub struct Conn {
    pub sess: Session,
    /// Commands collected between MULTI and EXEC.
    queued: Option<Vec<Vec<Vec<u8>>>>,
    /// A command was refused while queuing: EXEC will discard the transaction.
    aborted: bool,
    /// The workspace of database 0: where the user arrived. Known once SELECT is used.
    home: Option<String>,
    /// Whether the client switched to RESP3 with `HELLO 3`.
    pub resp3: bool,
    /// Where pub/sub messages go; `None` outside a network connection.
    pub outbox: Option<Outbox>,
    /// How many channels and patterns this connection listens to.
    subs: i64,
    /// Messages published inside MULTI or a script, sent when it commits.
    held: Option<Vec<Held>>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        if self.outbox.is_some() {
            self.sess.inst.resp.hub.leave(self.sess.id);
        }
    }
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

/// The deadline of a Redis expiry option, in milliseconds: `EX` seconds or
/// `PX` milliseconds from now, or `EXAT` / `PXAT` a Unix time.
fn deadline(option: &str, n: i64) -> Option<i64> {
    Some(match option {
        "EX" | "EXPIRE" | "SETEX" => now_ms().saturating_add(n.saturating_mul(1000)),
        "PX" | "PEXPIRE" | "PSETEX" => now_ms().saturating_add(n),
        "EXAT" | "EXPIREAT" => n.saturating_mul(1000),
        "PXAT" | "PEXPIREAT" => n,
        _ => return None,
    })
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

fn no_outbox() -> Reply {
    Reply::err("publish/subscribe needs its own Redis connection")
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
        Conn { sess, queued: None, aborted: false, home: None, resp3: false, outbox: None, subs: 0, held: None }
    }

    pub fn handle(&mut self, args: &[Vec<u8>]) -> Reply {
        let name = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
        // In RESP2, a listening connection only receives messages.
        if self.subs > 0 && !self.resp3 && !WHILE_SUBSCRIBED.contains(&name.as_str()) {
            return Reply::err(format!(
                "Can't execute '{}': only (P|S)SUBSCRIBE / (P)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context",
                name.to_ascii_lowercase()
            ));
        }
        if self.sess.user.is_none() && !matches!(name.as_str(), "AUTH" | "HELLO" | "PING" | "QUIT" | "ECHO") {
            return Reply::Error("NOAUTH Authentication required.".into());
        }
        // Commands KivaDB does not run are refused here, before they are
        // queued, so that a MULTI fails as a whole (EXECABORT), as in Redis.
        if let Some(refused) = commands::refuse(&name, args) {
            if self.queued.is_some() {
                self.aborted = true;
            }
            return refused;
        }
        // Inside MULTI, commands are queued until EXEC.
        if let Some(queue) = self.queued.as_mut() {
            if !matches!(name.as_str(), "EXEC" | "DISCARD" | "MULTI" | "QUIT" | "WATCH") {
                queue.push(args.to_vec());
                return Reply::Simple("QUEUED".into());
            }
        }
        // Commands that wait do it without the turn, so that the others run.
        if self.queued.is_none() {
            if name == "WAIT" {
                return self.wait_replicas(args).unwrap_or_else(|e| e);
            }
            let waits = match name.as_str() {
                "XREAD" | "XREADGROUP" => matches!(self.stream_block(args), Ok(Some(_))),
                n => blocking::is_blocking(n),
            };
            if waits {
                let args = {
                    let inst = self.sess.inst.clone();
                    let _turn = inst.resp.turn.lock().unwrap_or_else(|e| e.into_inner());
                    match self.resolve_last_ids(args) {
                        Ok(a) => a,
                        Err(e) => return e,
                    }
                };
                return self.block(&name, &args);
            }
        }
        self.with_turn(|c| c.run(&name, args).unwrap_or_else(|e| e)).unwrap_or_else(|e| e)
    }

    /// Runs `f` with the turn, then waits, without it, until the writes `f`
    /// made are on disk when `storage.sync_writes` is on. Waiting outside
    /// the turn lets the writes of every client share their fsyncs instead
    /// of waiting for them one after the other; each write is still on disk
    /// before its client is answered. `Err` when the disk sync failed.
    pub(super) fn with_turn<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> std::result::Result<R, Reply> {
        let inst = self.sess.inst.clone();
        let (r, syncs) = {
            let _turn = inst.resp.turn.lock().unwrap_or_else(|e| e.into_inner());
            crate::engine::put_off_syncs(|| f(self))
        };
        match syncs.wait() {
            Ok(()) => Ok(r),
            Err(e) => Err(Reply::err(format!("the write was made but could not be confirmed on disk: {e}"))),
        }
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
            // A set or sorted set is an object too, but not a hash.
            Some((Value::Doc(j), m)) if Value::Doc(j.clone()).ty() == want && sets::marked(&j).is_none() => {
                Ok(Some((j, m)))
            }
            Some(_) => Err(Reply::wrong_type()),
        }
    }

    /// Refuses to let the set, sorted set or list at `k` grow to `len`
    /// members beyond `resp.max_collection_members`. Each command reads and
    /// rewrites the whole value, so a huge one would make every command on it
    /// slow and costly. A collection already above the limit (the limit was
    /// lowered) may still shrink.
    fn check_size(&mut self, k: &str, kind: &str, len: usize) -> std::result::Result<(), Reply> {
        let limit = self.sess.inst.cfg().get_u64("resp", "max_collection_members") as usize;
        if limit == 0 || len <= limit {
            return Ok(());
        }
        let now = match self.get(k)? {
            Some((Value::Doc(Json::Array(a)), _)) => a.len(),
            Some((Value::Doc(j), _)) => match &j["members"] {
                Json::Array(a) => a.len(),
                Json::Object(o) => o.len(),
                _ => 0,
            },
            _ => 0,
        };
        if len <= now {
            return Ok(());
        }
        Err(Reply::err(format!(
            "the {kind} '{k}' would hold {len} members, more than resp.max_collection_members ({limit}). \
             Every command reads and rewrites the whole value: split it over several keys, or raise the limit"
        )))
    }

    /// The Redis type of a key: `hash` and `list` from the KivaDB type,
    /// `set` and `zset` from the object's marker.
    fn redis_type(&mut self, k: &str, ty: Type) -> std::result::Result<&'static str, Reply> {
        Ok(match ty {
            Type::Object => match self.get(k)? {
                Some((Value::Doc(j), _)) => sets::marked(&j).unwrap_or("hash"),
                _ => "hash",
            },
            Type::Array => "list",
            _ => "string",
        })
    }

    /// `KIVA.SIMILAR <pattern>.<field> <vector as JSON> [FILTER <field> <op> <value>]... [LIMIT n]
    /// [METRIC cosine|dot|l2] [EF n] [EXACT]`: the closest documents, as
    /// `[key, score, document as JSON]` triples. Every FILTER must hold.
    fn kiva_similar(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 3)?;
        let text = |a: &Vec<u8>| String::from_utf8_lossy(a).to_string();
        let vector: Json = serde_json::from_slice(&args[2]).map_err(|_| {
            // Plain text where numbers were expected: the embedding was not computed.
            if args[2].first().is_some_and(|b| *b != b'[') {
                Reply::err(crate::vector::NOT_EMBEDDINGS)
            } else {
                Reply::err("the vector must be a JSON array of numbers, such as [0.1,0.2]")
            }
        })?;
        let mut req = json!({ "op": "similar", "target": text(&args[1]), "vector": vector, "filter": [] });
        let mut rest = args[3..].iter();
        while let Some(opt) = rest.next() {
            let opt = text(opt).to_ascii_uppercase();
            let mut value = || rest.next().map(text).ok_or_else(|| Reply::err(format!("{opt} needs a value")));
            match opt.as_str() {
                "LIMIT" | "EF" => {
                    let n =
                        value()?.parse::<u64>().map_err(|_| Reply::err("value is not an integer or out of range"))?;
                    req[opt.to_ascii_lowercase()] = n.into();
                }
                "METRIC" => req["metric"] = value()?.to_ascii_lowercase().into(),
                "EXACT" => req["exact"] = true.into(),
                "FILTER" => {
                    let (field, cmp, raw) = (value()?, value()?, value()?);
                    // A number, true, false or null as such; anything else is a string.
                    let v = serde_json::from_str::<Json>(&raw)
                        .ok()
                        .filter(|j| !j.is_object() && !j.is_array())
                        .unwrap_or(Json::String(raw));
                    req["filter"]
                        .as_array_mut()
                        .expect("array")
                        .push(json!({ "field": field, "cmp": cmp, "value": v }));
                }
                _ => return Err(Reply::err("syntax error")),
            }
        }
        let answer = crate::api::handle(&mut self.sess, &req)?;
        let rows = answer["results"].as_array().cloned().unwrap_or_default();
        Ok(Reply::array(
            rows.iter()
                .map(|r| {
                    Reply::array(vec![
                        Reply::bulk(r["key"].as_str().unwrap_or_default()),
                        // Scores are f32: send their shortest form, not 16 digits of noise.
                        Reply::Double((r["score"].as_f64().unwrap_or(0.0) as f32).to_string().parse().unwrap_or(0.0)),
                        Reply::bulk(r["value"].to_string()),
                    ])
                })
                .collect(),
        ))
    }

    fn run(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "PING" if self.subs > 0 && !self.resp3 => {
                Ok(Reply::array(vec![Reply::bulk("pong"), Reply::bulk(args.get(1).cloned().unwrap_or_default())]))
            }
            "PING" => Ok(match args.get(1) {
                Some(m) => Reply::bulk(m.clone()),
                None => Reply::Simple("PONG".into()),
            }),
            "ECHO" => {
                arity(args, 2)?;
                Ok(Reply::bulk(args[1].clone()))
            }
            "KIVA.SIMILAR" => self.kiva_similar(args),
            "KIVA.API" => {
                if args.len() != 2 {
                    return Err(Reply::err("wrong number of arguments for 'kiva.api' command"));
                }
                let req: Json = serde_json::from_slice(&args[1])
                    .map_err(|e| Reply::err(format!("the request must be a JSON object: {e}")))?;
                Ok(Reply::bulk(crate::api::handle(&mut self.sess, &req)?.to_string()))
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
                // The login lands in the user's own workspace: that is database 0 again.
                self.home = None;
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
                // `SELECT travel.places` names a workspace; a number is a Redis database.
                let name = text(&args[1])?;
                if name.parse::<i64>().is_err() {
                    self.select_workspace(&name)?;
                    return Ok(Reply::ok());
                }
                let n = int(&args[1])?;
                if !(0..DATABASES).contains(&n) {
                    return Err(Reply::err("DB index is out of range"));
                }
                self.select(n)?;
                Ok(Reply::ok())
            }
            "TIME" => {
                let us = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros()).unwrap_or(0);
                Ok(Reply::array(vec![
                    Reply::bulk((us / 1_000_000).to_string()),
                    Reply::bulk((us % 1_000_000).to_string()),
                ]))
            }
            "EVAL" | "EVAL_RO" | "EVALSHA" | "EVALSHA_RO" => {
                arity(args, 3)?;
                self.sess.check(Need::Read)?;
                let source = if name.starts_with("EVALSHA") {
                    let sha = text(&args[1])?;
                    self.sess
                        .inst
                        .resp
                        .script(&sha)
                        .ok_or_else(|| Reply::Error("NOSCRIPT No matching script. Please use EVAL.".into()))?
                } else {
                    let source = text(&args[1])?;
                    self.sess.inst.resp.remember(&source);
                    source
                };
                let numkeys = int(&args[2])?;
                if numkeys < 0 {
                    return Err(Reply::err("Number of keys can't be negative"));
                }
                if numkeys as usize > args.len() - 3 {
                    return Err(Reply::err("Number of keys can't be greater than number of args"));
                }
                let split = 3 + numkeys as usize;
                self.eval(&source, &args[3..split], &args[split..])
            }
            "SCRIPT" => {
                arity(args, 2)?;
                match String::from_utf8_lossy(&args[1]).to_ascii_uppercase().as_str() {
                    "LOAD" => {
                        arity(args, 3)?;
                        let source = text(&args[2])?;
                        Lua::new().load(&source).into_function().map_err(|e| {
                            Reply::err(format!("Error compiling script: {}", first_line(&e.to_string())))
                        })?;
                        Ok(Reply::bulk(self.sess.inst.resp.remember(&source)))
                    }
                    "EXISTS" => {
                        let mut found = Vec::new();
                        for sha in &args[2..] {
                            found.push(Reply::Int(self.sess.inst.resp.script(&text(sha)?).is_some() as i64));
                        }
                        Ok(Reply::array(found))
                    }
                    "FLUSH" => {
                        self.sess.inst.resp.scripts.lock().unwrap_or_else(|e| e.into_inner()).clear();
                        Ok(Reply::ok())
                    }
                    other => {
                        Err(Reply::err(format!("unknown subcommand '{}' for 'script'", other.to_ascii_lowercase())))
                    }
                }
            }
            "COMMAND" => Ok(match args.get(1).map(|a| String::from_utf8_lossy(a).to_ascii_uppercase()) {
                Some(s) if s == "COUNT" => Reply::Int(commands::SUPPORTED.len() as i64),
                Some(s) if s == "LIST" => {
                    Reply::array(commands::SUPPORTED.iter().map(|c| Reply::bulk(c.to_ascii_lowercase())).collect())
                }
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
                self.aborted = false;
                Ok(Reply::ok())
            }
            "DISCARD" => match self.queued.take() {
                Some(_) => {
                    self.sess.unwatch();
                    Ok(Reply::ok())
                }
                None => Err(Reply::err("DISCARD without MULTI")),
            },
            "EXEC" => {
                let r = self.exec();
                self.sess.unwatch();
                r
            }
            // EXEC answers nil when one of these keys changes before it.
            "WATCH" => {
                arity(args, 2)?;
                if self.queued.is_some() {
                    return Err(Reply::err("WATCH inside MULTI is not allowed"));
                }
                self.sess.check(Need::Read)?;
                let keys = args[1..].iter().map(|a| key(a)).collect::<std::result::Result<Vec<_>, _>>()?;
                self.sess.watch(&keys);
                Ok(Reply::ok())
            }
            // Inside MULTI or a script: one try, no wait.
            "BLPOP" | "BRPOP" | "BRPOPLPUSH" | "BLMOVE" | "BLMPOP" | "BZPOPMIN" | "BZPOPMAX" | "BZMPOP" => {
                Ok(self.try_blocking(name, args)?.unwrap_or_else(|| self.timed_out(name)))
            }
            "XREAD" | "XREADGROUP" => Ok(self.stream_read(name, args)?.unwrap_or(Reply::Array(None))),
            "WAIT" => {
                arity(args, 3)?;
                let n = usize::try_from(int(&args[1])?).map_err(|_| Reply::err("value is out of range"))?;
                Ok(Reply::Int(self.sess.inst.repl.wait(n, Duration::ZERO) as i64))
            }
            "UNWATCH" => {
                self.sess.unwatch();
                Ok(Reply::ok())
            }
            "SUBSCRIBE" | "PSUBSCRIBE" => {
                arity(args, 2)?;
                let outbox = self.outbox.as_ref().ok_or_else(no_outbox)?;
                let scope = self.scope();
                let n =
                    self.sess.inst.resp.hub.subscribe(self.sess.id, outbox, &scope, &args[1..], name == "PSUBSCRIBE");
                self.subs = n;
                Ok(Reply::Sent)
            }
            "UNSUBSCRIBE" | "PUNSUBSCRIBE" => {
                let outbox = self.outbox.as_ref().ok_or_else(no_outbox)?;
                let n = self.sess.inst.resp.hub.unsubscribe(self.sess.id, outbox, &args[1..], name == "PUNSUBSCRIBE");
                self.subs = n;
                Ok(Reply::Sent)
            }
            "PUBLISH" => {
                if args.len() != 3 {
                    return Err(Reply::err("wrong number of arguments for 'publish' command"));
                }
                let scope = self.scope();
                let inst = self.sess.inst.clone();
                Ok(Reply::Int(match self.held.as_mut() {
                    // Inside MULTI or a script: sent once it commits.
                    Some(held) => {
                        held.push(Held { scope: scope.clone(), channel: args[1].clone(), message: args[2].clone() });
                        inst.resp.hub.publish(&scope, &args[1], &args[2], true)
                    }
                    None => inst.resp.hub.publish(&scope, &args[1], &args[2], false),
                }))
            }
            "PUBSUB" => {
                arity(args, 2)?;
                let scope = self.scope();
                let hub = &self.sess.inst.resp.hub;
                match String::from_utf8_lossy(&args[1]).to_ascii_uppercase().as_str() {
                    "CHANNELS" => Ok(Reply::array(
                        hub.channels(&scope, args.get(2).map(|p| p.as_slice())).into_iter().map(Reply::bulk).collect(),
                    )),
                    "NUMSUB" => Ok(Reply::Map(
                        args[2..].iter().map(|c| (Reply::bulk(c.clone()), Reply::Int(hub.numsub(&scope, c)))).collect(),
                    )),
                    "NUMPAT" => Ok(Reply::Int(hub.numpat(&scope))),
                    other => {
                        Err(Reply::err(format!("unknown subcommand '{}' for 'pubsub'", other.to_ascii_lowercase())))
                    }
                }
            }

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
                let exp = deadline(name, n).expect("an expiry command");
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
                let k = key(&args[1])?;
                let t = match self.sess.kv_type(&k)? {
                    None => "none",
                    Some(ty) => self.redis_type(&k, ty)?,
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
                let (mut nx, mut xx, mut gt, mut lt) = (false, false, false, false);
                for opt in &args[3..] {
                    match String::from_utf8_lossy(opt).to_ascii_uppercase().as_str() {
                        "NX" => nx = true,
                        "XX" => xx = true,
                        "GT" => gt = true,
                        "LT" => lt = true,
                        other => return Err(Reply::err(format!("Unsupported option {other}"))),
                    }
                }
                if (nx && (xx || gt || lt)) || (gt && lt) {
                    return Err(Reply::err("NX and XX, GT or LT options at the same time are not compatible"));
                }
                let exp = deadline(name, n).expect("an expiry command");
                let Some((v, m)) = self.get(&k)? else { return Ok(Reply::Int(0)) };
                // 0 means "no expiry", which counts as infinitely far for GT and LT.
                let cur = m.expires_at;
                let allowed = if nx {
                    cur == 0
                } else {
                    (!xx || cur != 0) && (!gt || (cur != 0 && exp > cur)) && (!lt || cur == 0 || exp < cur)
                };
                if !allowed {
                    return Ok(Reply::Int(0));
                }
                if exp <= now_ms() {
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
            "EXPIRETIME" | "PEXPIRETIME" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                Ok(Reply::Int(match self.get(&k)? {
                    None => -2,
                    Some((_, m)) if m.expires_at == 0 => -1,
                    Some((_, m)) if name == "EXPIRETIME" => m.expires_at / 1000,
                    Some((_, m)) => m.expires_at,
                }))
            }
            "GETEX" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let Some((v, m)) = self.get(&k)? else { return Ok(Reply::nil()) };
                let bytes = bytes_of(&v).ok_or_else(Reply::wrong_type)?;
                let exp = match args.get(2).map(|o| String::from_utf8_lossy(o).to_ascii_uppercase()) {
                    None => None,
                    Some(o) if o == "PERSIST" && args.len() == 3 => Some(0),
                    Some(o) if args.len() == 4 => {
                        let n = int(&args[3])?;
                        if n <= 0 {
                            return Err(Reply::err("invalid expire time in 'getex' command"));
                        }
                        Some(deadline(&o, n).ok_or_else(|| Reply::err("syntax error"))?)
                    }
                    _ => return Err(Reply::err("syntax error")),
                };
                if let Some(e) = exp.filter(|e| *e != m.expires_at) {
                    self.sess.kv_put(&k, v, e, m.msgpack)?;
                }
                Ok(Reply::bulk(bytes))
            }
            "TTL" | "PTTL" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                Ok(Reply::Int(match self.get(&k)? {
                    None => -2,
                    Some((_, m)) if m.expires_at == 0 => -1,
                    Some((_, m)) => {
                        let ms = (m.expires_at - now_ms()).max(0);
                        // Rounded to the nearest second, as Redis does.
                        if name == "TTL" {
                            (ms + 500) / 1000
                        } else {
                            ms
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
                    _ => Reply::Map(
                        obj.iter().map(|(f, v)| (Reply::bulk(f.clone()), Reply::bulk(json_text(v)))).collect(),
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
                self.check_size(&k, "list", len)?;
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
                if let Some(r) = self.run_sets(name, args) {
                    return r;
                }
                if let Some(r) = self.run_classic(name, args) {
                    return r;
                }
                if let Some(r) = self.run_json(name, args) {
                    return r;
                }
                if let Some(r) = self.run_streams(name, args) {
                    return r;
                }
                Err(commands::unknown(args))
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
        let mut proto = if self.resp3 { 3 } else { 2 };
        if let Some(v) = args.get(1) {
            proto = int(v)?;
            if proto != 2 && proto != 3 {
                return Err(Reply::Error("NOPROTO unsupported protocol version".into()));
            }
            i = 2;
        }
        while i < args.len() {
            match String::from_utf8_lossy(&args[i]).to_ascii_uppercase().as_str() {
                "AUTH" if i + 2 < args.len() => {
                    let (u, p) = (text(&args[i + 1])?, text(&args[i + 2])?);
                    self.login(&u, &p)?;
                    self.home = None;
                    i += 3;
                }
                "SETNAME" if i + 1 < args.len() => i += 2,
                _ => return Err(Reply::err("syntax error in HELLO")),
            }
        }
        if self.sess.user.is_none() {
            return Err(Reply::Error("NOAUTH HELLO must be called with the client already authenticated".into()));
        }
        self.resp3 = proto == 3;
        let pairs: Vec<(&str, Reply)> = vec![
            ("server", Reply::bulk("kivadb")),
            ("version", Reply::bulk(VERSION)),
            ("proto", Reply::Int(proto)),
            ("id", Reply::Int(self.sess.id as i64)),
            ("mode", Reply::bulk("standalone")),
            ("role", Reply::bulk(if self.sess.inst.repl.is_replica() { "replica" } else { "master" })),
            ("modules", Reply::array(Vec::new())),
        ];
        Ok(Reply::Map(pairs.into_iter().map(|(k, v)| (Reply::bulk(k), v)).collect()))
    }

    /// `SELECT n`: database 0 is the workspace the user arrived in (their default
    /// one), database n
    /// the workspace `db<n>`, created on first use when the user is an administrator.
    fn select(&mut self, n: i64) -> std::result::Result<(), Reply> {
        let arrived = self.sess.ws.clone();
        let home = self.home.get_or_insert(arrived).clone();
        let target = if n == 0 { home } else { format!("db{n}") };
        if target == self.sess.ws {
            return Ok(());
        }
        // A user who arrived in no workspace is back in none.
        if target.is_empty() {
            self.sess.leave();
            return Ok(());
        }
        let (user, admin) = match &self.sess.user {
            Some(u) => (u.name.clone(), u.role == Role::Admin),
            None => return Err(Reply::Error("NOAUTH Authentication required.".into())),
        };
        if n > 0 && admin {
            self.sess.inst.ensure_workspace(&target)?;
        }
        self.sess.switch_workspace(&target).map_err(|e| match e {
            KivaError::Denied(_) | KivaError::NotFound(_) => Reply::Error(format!(
                "NOPERM no access to database {n} (KivaDB workspace '{target}'). An administrator can create it and give access with: workspace create {target}, then grant write on workspace \"{target}\" to \"{user}\""
            )),
            other => other.into(),
        })
    }

    /// `SELECT <workspace>`: a workspace by its name, such as `travel.places`.
    /// Clients that resend `SELECT` on reconnection keep it (ioredis with
    /// `db: "travel.places"`, node-redis with `database: "travel.places"`).
    fn select_workspace(&mut self, name: &str) -> std::result::Result<(), Reply> {
        let Some(user) = self.sess.user.as_ref().map(|u| u.name.clone()) else {
            return Err(Reply::Error("NOAUTH Authentication required.".into()));
        };
        if name == self.sess.ws && self.sess.ws_priv.is_some() {
            return Ok(());
        }
        self.home.get_or_insert_with(|| self.sess.ws.clone());
        self.sess.switch_workspace(name).map_err(|e| match e {
            KivaError::Denied(_) | KivaError::NotFound(_) => Reply::Error(format!(
                "NOPERM no access to workspace '{name}' for '{user}', or it does not exist. An administrator gives \
                 it with: grant write on {name} to \"{user}\""
            )),
            other => other.into(),
        })
    }

    /// Runs a Lua script, all or nothing: its writes are committed together
    /// when it ends without error, and discarded otherwise.
    fn eval(&mut self, source: &str, keys: &[Vec<u8>], argv: &[Vec<u8>]) -> Res {
        let lua = crate::script::sandbox(&self.sess, None)?;
        // A read-only user (or replica) runs without a transaction: any write it
        // attempts is refused on its own.
        let implicit = self.sess.txn.is_none() && self.sess.writable().is_ok();
        if implicit {
            self.sess.begin()?;
            self.held = Some(Vec::new());
        }
        let result = self.run_script(&lua, source, keys, argv);
        lua.remove_hook();
        if implicit {
            let held = self.held.take();
            match &result {
                Ok(_) => {
                    self.sess.commit()?;
                    self.release(held);
                }
                Err(_) => {
                    self.sess.rollback();
                }
            }
        }
        result
    }

    fn run_script(&mut self, lua: &Lua, source: &str, keys: &[Vec<u8>], argv: &[Vec<u8>]) -> Res {
        let cell = RefCell::new(&mut *self);
        let out = lua.scope(|scope| {
            let globals = lua.globals();
            let sequence = |items: &[Vec<u8>]| -> mlua::Result<Table> {
                let t = lua.create_table()?;
                for (i, a) in items.iter().enumerate() {
                    t.raw_set(i + 1, lua.create_string(a)?)?;
                }
                Ok(t)
            };
            globals.set("KEYS", sequence(keys)?)?;
            globals.set("ARGV", sequence(argv)?)?;

            let redis = lua.create_table()?;
            for (fname, raise) in [("call", true), ("pcall", false)] {
                let cell = &cell;
                let f = scope.create_function(move |lua, values: MultiValue| {
                    let args = values.iter().map(lua_arg).collect::<mlua::Result<Vec<_>>>()?;
                    let Some(first) = args.first() else {
                        return Err(mlua::Error::RuntimeError(
                            "ERR Please specify at least one argument for this redis lib call".into(),
                        ));
                    };
                    let name = String::from_utf8_lossy(first).to_ascii_uppercase();
                    let reply = if NOT_IN_SCRIPTS.contains(&name.as_str()) {
                        Reply::err("This Redis command is not allowed from script")
                    } else if let Some(refused) = commands::refuse(&name, &args) {
                        refused
                    } else {
                        cell.borrow_mut().run(&name, &args).unwrap_or_else(|e| e)
                    };
                    match reply {
                        Reply::Error(e) if raise => Err(mlua::Error::RuntimeError(e)),
                        other => reply_to_lua(lua, &other),
                    }
                })?;
                redis.set(fname, f)?;
            }
            redis.set(
                "error_reply",
                lua.create_function(|lua, msg: String| {
                    let t = lua.create_table()?;
                    t.set("err", msg)?;
                    Ok(t)
                })?,
            )?;
            redis.set(
                "status_reply",
                lua.create_function(|lua, msg: String| {
                    let t = lua.create_table()?;
                    t.set("ok", msg)?;
                    Ok(t)
                })?,
            )?;
            redis.set("sha1hex", lua.create_function(|_, s: mlua::String| Ok(sha1_hex(&s.as_bytes())))?)?;
            redis.set("log", lua.create_function(|_, _: MultiValue| Ok(()))?)?;
            for (i, level) in ["LOG_DEBUG", "LOG_VERBOSE", "LOG_NOTICE", "LOG_WARNING"].iter().enumerate() {
                redis.set(*level, i as i64)?;
            }
            globals.set("redis", redis)?;

            let value = lua.load(source).set_name("user_script").eval::<LVal>()?;
            Ok(lua_to_reply(&value, 0))
        });
        out.map_err(script_error)
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
                    exp = deadline(&opt, n);
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
            if !glob_match(&pattern, k) {
                continue;
            }
            // Telling a set from a hash means reading the value: only with TYPE.
            let wanted = match &kind {
                None => true,
                Some(w) => self.redis_type(k, e.ty)? == w.as_str(),
            };
            if wanted {
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
        if std::mem::take(&mut self.aborted) {
            return Err(Reply::Error("EXECABORT Transaction discarded because of previous errors.".into()));
        }
        if self.sess.txn.is_some() {
            return Err(Reply::err("a transaction is already open on this session"));
        }
        // A watched key changed already: nothing runs. (The commit checks
        // again, under the same lock as its writes.)
        if let Some((e, w)) = &self.sess.watch {
            if e.watch_conflict(w).is_some() {
                return Ok(Reply::Array(None));
            }
        }
        let begun = self.sess.begin();
        let in_txn = begun.is_ok();
        if in_txn {
            self.held = Some(Vec::new());
        }
        let mut replies = Vec::with_capacity(queued.len());
        for args in &queued {
            let name = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
            let r = if matches!(
                name.as_str(),
                "FLUSHDB" | "FLUSHALL" | "MULTI" | "EXEC" | "SUBSCRIBE" | "PSUBSCRIBE" | "UNSUBSCRIBE" | "PUNSUBSCRIBE"
            ) {
                Reply::err(format!("{name} is not allowed inside MULTI"))
            } else {
                self.run(&name, args).unwrap_or_else(|e| e)
            };
            replies.push(r);
        }
        let held = self.held.take();
        if in_txn {
            if let Err(e) = self.sess.commit() {
                return Ok(match e {
                    KivaError::Conflict(_) => Reply::Array(None),
                    other => Reply::from(other),
                });
            }
        }
        self.release(held);
        Ok(Reply::array(replies))
    }

    /// Sends the messages held during a transaction that committed.
    fn release(&self, held: Option<Vec<Held>>) {
        for m in held.unwrap_or_default() {
            self.sess.inst.resp.hub.publish(&m.scope, &m.channel, &m.message, false);
        }
    }

    /// The workspace whose channels this connection uses: its database 0.
    /// A user who arrived in no workspace has channels of their own.
    fn scope(&self) -> String {
        let ws = self.home.clone().unwrap_or_else(|| self.sess.ws.clone());
        if ws.is_empty() {
            format!("@{}", self.sess.user_name())
        } else {
            ws
        }
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
        if inst.repl.is_replica() {
            // Clients that follow a failover read the master here, as with Redis.
            let master = inst.repl.link().master;
            if let Some((host, port)) = master.rsplit_once(':') {
                t.push_str(&format!("master_host:{host}\r\nmaster_port:{port}\r\n"));
            }
        }
        t.push_str(&format!("failover_epoch:{}\r\n", inst.repl.epoch()));
        t.push_str(&format!("\r\n# Keyspace\r\ndb0:keys={},expires={},avg_ttl=0\r\n", s.keys, s.ttl_keys));
        t.into_bytes()
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

/// A `redis.call` argument: Lua strings and numbers become Redis strings.
fn lua_arg(v: &LVal) -> mlua::Result<Vec<u8>> {
    match v {
        LVal::String(s) => Ok(s.as_bytes().to_vec()),
        LVal::Integer(i) => Ok(i.to_string().into_bytes()),
        LVal::Number(f) => Ok(fmt_f64(*f).into_bytes()),
        _ => Err(mlua::Error::RuntimeError("ERR Lua redis lib command arguments must be strings or integers".into())),
    }
}

/// A command's reply as Redis gives it to Lua.
fn reply_to_lua(lua: &Lua, r: &Reply) -> mlua::Result<LVal> {
    Ok(match r {
        Reply::Int(i) => LVal::Integer(*i),
        Reply::Bulk(Some(b)) => LVal::String(lua.create_string(b)?),
        Reply::Bulk(None) | Reply::Array(None) => LVal::Boolean(false),
        Reply::Simple(s) => {
            let t = lua.create_table()?;
            t.set("ok", s.as_str())?;
            LVal::Table(t)
        }
        Reply::Error(e) => {
            let t = lua.create_table()?;
            t.set("err", e.as_str())?;
            LVal::Table(t)
        }
        Reply::Sent => LVal::Boolean(false),
        // Scripts see Redis 2 replies: a score is text.
        Reply::Double(f) => LVal::String(lua.create_string(fmt_f64(*f))?),
        Reply::Array(Some(items)) | Reply::Push(items) | Reply::Set(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.raw_set(i + 1, reply_to_lua(lua, item)?)?;
            }
            LVal::Table(t)
        }
        Reply::Map(pairs) => {
            let t = lua.create_table()?;
            for (i, (k, v)) in pairs.iter().enumerate() {
                t.raw_set(2 * i + 1, reply_to_lua(lua, k)?)?;
                t.raw_set(2 * i + 2, reply_to_lua(lua, v)?)?;
            }
            LVal::Table(t)
        }
    })
}

/// A script's return value as Redis sends it to the client.
fn lua_to_reply(v: &LVal, depth: usize) -> Reply {
    match v {
        LVal::Boolean(true) => Reply::Int(1),
        LVal::Integer(i) => Reply::Int(*i),
        // Redis truncates numbers to integers.
        LVal::Number(f) => Reply::Int(*f as i64),
        LVal::String(s) => Reply::bulk(s.as_bytes().to_vec()),
        LVal::Table(t) if depth < 64 => {
            if let Ok(LVal::String(e)) = t.raw_get::<LVal>("err") {
                return Reply::Error(e.to_string_lossy().to_string());
            }
            if let Ok(LVal::String(s)) = t.raw_get::<LVal>("ok") {
                return Reply::Simple(s.to_string_lossy().to_string());
            }
            let mut items = Vec::new();
            for i in 1.. {
                match t.raw_get::<LVal>(i) {
                    Ok(LVal::Nil) | Err(_) => break,
                    Ok(item) => items.push(lua_to_reply(&item, depth + 1)),
                }
            }
            Reply::array(items)
        }
        _ => Reply::nil(),
    }
}

/// A script failure as a Redis error: the error of a failed `redis.call`
/// is passed through, anything else is reported as a script error.
fn script_error(e: mlua::Error) -> Reply {
    let mut cause = &e;
    while let mlua::Error::CallbackError { cause: inner, .. } = cause {
        cause = inner;
    }
    let text = match cause {
        mlua::Error::RuntimeError(m) => m.clone(),
        other => other.to_string(),
    };
    let text = text.split("\nstack traceback").next().unwrap_or(&text).trim().to_string();
    let coded =
        text.split_whitespace().next().is_some_and(|w| w.len() > 1 && w.chars().all(|c| c.is_ascii_uppercase()));
    if coded {
        Reply::Error(text)
    } else {
        Reply::Error(format!("ERR Error running script: {text}"))
    }
}
