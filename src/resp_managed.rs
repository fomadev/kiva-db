// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use super::{scalar_need, text, Reply, Res, Scalar};
use crate::error::{KivaError, Result};
use crate::instance::Instance;
use crate::managed_transport as transport;
use crate::native_admission::Class;
use crate::server::Deadline;
use crate::session::{Need, Session};
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::{atomic::Ordering, Arc};

const MAX_ARGS: usize = 128;
const MAX_QUEUED: usize = 64;

/// Starts the opt-in, authenticated RESP adapter for managed collections.
pub fn start_hierarchical_resp(inst: &Arc<Instance>, bind: &str, port: u16) -> Result<u16> {
    transport::start(
        inst,
        bind,
        port,
        "resp",
        b"-ERR this server requires TLS from other machines (use rediss://)\r\n",
        b"-BUSYERR managed client limit reached\r\n",
        serve,
    )
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "malformed or excessive managed RESP request")
}

fn number(header: &[u8], prefix: u8) -> io::Result<usize> {
    let rest = header
        .strip_prefix(&[prefix])
        .filter(|s| !s.is_empty() && s.iter().all(u8::is_ascii_digit))
        .ok_or_else(invalid)?;
    std::str::from_utf8(rest).ok().and_then(|n| n.parse().ok()).ok_or_else(invalid)
}

fn read_command(reader: &mut impl BufRead) -> io::Result<Option<Vec<Vec<u8>>>> {
    let Some(header) = transport::line(reader, 32)? else { return Ok(None) };
    let count = number(&header, b'*')?;
    if !(1..=MAX_ARGS).contains(&count) {
        return Err(invalid());
    }
    let mut remaining = transport::REQUEST_MAX - header.len() - 2;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let header = transport::line(reader, 32)?.ok_or_else(invalid)?;
        let len = number(&header, b'$')?;
        let consumed = (header.len() + 4).checked_add(len).filter(|n| *n <= remaining).ok_or_else(invalid)?;
        remaining -= consumed;
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes)?;
        let mut crlf = [0; 2];
        reader.read_exact(&mut crlf)?;
        if crlf != *b"\r\n" {
            return Err(invalid());
        }
        args.push(bytes);
    }
    Ok(Some(args))
}

fn serve(inst: &Arc<Instance>, mut stream: TcpStream) {
    let Ok(mut writer) = stream.try_clone() else { return };
    let (mut read_timeout, mut write_timeout) = (None, None);
    let mut reader = BufReader::new(Deadline::new(&mut stream, &mut read_timeout));
    let mut conn = Connection { sess: transport::session(inst), queued: None, resp3: false };
    while !inst.shutting_down.load(Ordering::Acquire) {
        reader.get_mut().rearm();
        let (reply, quit) = match read_command(&mut reader) {
            Ok(Some(args)) => {
                let quit = args[0].eq_ignore_ascii_case(b"QUIT");
                (conn.handle(args).unwrap_or_else(|e| e), quit)
            }
            Ok(None) => break,
            Err(_) => (Reply::err("protocol error"), true),
        };
        let mut output = Vec::new();
        reply.encode(&mut output, conn.resp3);
        if output.len() > transport::REPLY_MAX {
            output = b"-ERR_OUTCOME_UNKNOWN managed reply exceeded the limit\r\n".to_vec();
        }
        if Deadline::new(&mut writer, &mut write_timeout).write_all(&output).is_err() || quit {
            break;
        }
    }
}

struct Queue {
    items: Vec<Vec<Vec<u8>>>,
    bytes: usize,
    aborted: bool,
}

struct Connection {
    sess: Session,
    queued: Option<Queue>,
    resp3: bool,
}

pub(super) fn error(e: KivaError) -> Reply {
    match e {
        KivaError::Denied(_) | KivaError::NotFound(_) => {
            Reply::Error("NOPERM resource or operation unavailable".into())
        }
        KivaError::Other(m) | KivaError::Invalid(m) if m.starts_with("ERR_ADAPTER_LIMIT:") => limit(),
        KivaError::Other(m) if m.starts_with("ERR_SERVER_BUSY:") => {
            Reply::Error("BUSYERR request refused before execution".into())
        }
        KivaError::Conflict(_) => Reply::Error("TRYAGAIN transaction conflict".into()),
        KivaError::Io(_)
        | KivaError::Corrupted(_)
        | KivaError::Locked(_)
        | KivaError::Format(_)
        | KivaError::Other(_) => Reply::Error("ERR_OUTCOME_UNKNOWN managed operation could not be confirmed".into()),
        e => Reply::from(e),
    }
}

fn limit() -> Reply {
    Reply::Error("ERR_ADAPTER_LIMIT response or transaction budget exceeded".into())
}

pub(super) fn charge(remaining: &mut usize, reply: &Reply) -> std::result::Result<(), Reply> {
    *remaining = remaining.checked_sub(32).ok_or_else(limit)?;
    match reply {
        Reply::Bulk(Some(bytes)) => *remaining = remaining.checked_sub(bytes.len()).ok_or_else(limit)?,
        Reply::Simple(msg) | Reply::Error(msg) => {
            *remaining = remaining.checked_sub(msg.len()).ok_or_else(limit)?;
        }
        Reply::Array(Some(items)) => {
            for item in items {
                charge(remaining, item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_arity(name: &str, n: usize) -> Res {
    let valid = match name {
        "GET" | "TYPE" | "KEYS" | "TTL" | "PTTL" | "EXPIRETIME" | "PEXPIRETIME" | "PERSIST" | "INCR" | "DECR" => n == 2,
        "SET" | "EXPIRE" | "PEXPIRE" | "EXPIREAT" | "PEXPIREAT" => n >= 3,
        "INCRBY" | "DECRBY" => n == 3,
        "MSET" => n >= 3 && !n.is_multiple_of(2),
        "MGET" | "EXISTS" | "DEL" | "UNLINK" => n >= 2,
        "SCAN" => n >= 2 && n.is_multiple_of(2),
        _ => false,
    };
    if valid {
        Ok(Reply::ok())
    } else {
        Err(Reply::err("wrong number of arguments"))
    }
}

impl Connection {
    fn handle(&mut self, args: Vec<Vec<u8>>) -> Res {
        if args[0].len() > 32 {
            return Err(Reply::err("invalid command name"));
        }
        let name = text(&args[0])?.to_ascii_uppercase();
        let inst = self.sess.inst.clone();
        match (name.as_str(), args.as_slice()) {
            ("QUIT", [_]) => return Ok(Reply::ok()),
            ("PING", [_]) => return Ok(Reply::Simple("PONG".into())),
            ("PING", [_, value]) | ("ECHO", [_, value]) => return Ok(Reply::bulk(value.clone())),
            ("AUTH" | "HELLO" | "KIVA.USE", _) if self.queued.is_some() => {
                return Err(Reply::err("authentication or selection is unavailable during MULTI"));
            }
            ("AUTH", [_, password]) => return self.auth(crate::auth::DEFAULT_ADMIN, password),
            ("AUTH", [_, user, password]) => return self.auth(&text(user)?, password),
            ("HELLO", [_, version, verb, user, password]) if verb.eq_ignore_ascii_case(b"AUTH") => {
                let resp3 = protocol(version)?;
                self.auth(&text(user)?, password)?;
                self.resp3 = resp3;
                return Ok(self.hello());
            }
            ("HELLO", [_, version]) => {
                if self.sess.user.is_none() {
                    return Err(Reply::Error("NOAUTH Authentication required".into()));
                }
                self.resp3 = protocol(version)?;
                return Ok(self.hello());
            }
            ("HELLO", [_]) if self.sess.user.is_some() => return Ok(self.hello()),
            _ => {}
        }
        if self.sess.user.is_none() {
            return Err(Reply::Error("NOAUTH Authentication required".into()));
        }
        self.sess.use_zone();
        match (name.as_str(), args.as_slice()) {
            ("KIVA.USE", [_, path]) => {
                let _permit = inst.native_admission.enter_bounded(Class::Control, transport::WAIT).map_err(error)?;
                transport::select(&mut self.sess, &text(path)?).map_err(error)?;
                return Ok(Reply::ok());
            }
            ("DISCARD", [_]) => {
                return self.queued.take().map(|_| Reply::ok()).ok_or_else(|| Reply::err("DISCARD without MULTI"))
            }
            ("MULTI", [_]) => {
                if self.queued.is_some() {
                    return Err(Reply::err("MULTI calls can not be nested"));
                }
                let _permit = inst.native_admission.enter_bounded(Class::Control, transport::WAIT).map_err(error)?;
                if self.sess.hierarchy_location().map_err(error)?.is_none_or(|location| location.names[2].is_empty()) {
                    return Err(Reply::err("select an authorized collection with KIVA.USE"));
                }
                self.queued = Some(Queue { items: Vec::new(), bytes: 0, aborted: false });
                return Ok(Reply::ok());
            }
            ("EXEC", [_]) => return self.exec(),
            ("SELECT", _) => {
                return Err(Reply::err("SELECT is unavailable; use KIVA.USE workspace.project.collection"))
            }
            ("CLIENT", [_, sub, _, _]) if sub.eq_ignore_ascii_case(b"SETINFO") => return Ok(Reply::ok()),
            ("KIVA.API", [_, payload]) if self.queued.is_none() => {
                let _permit = inst.native_admission.enter_bounded(Class::Data, transport::WAIT).map_err(error)?;
                let req = crate::num::from_slice(payload).map_err(|_| Reply::err("invalid JSON request"))?;
                if matches!(req["op"].as_str(), Some("begin" | "commit" | "rollback" | "whoami")) {
                    return Err(Reply::err("operation is unavailable through KIVA.API"));
                }
                let answer = crate::api::handle_native_hierarchy(&mut self.sess, &req).map_err(error)?;
                return Ok(Reply::bulk(answer.to_string()));
            }
            _ => {}
        }
        let need = scalar_need(&name);
        if let Some(queue) = self.queued.as_mut() {
            if need.is_none() || matches!(name.as_str(), "SCAN" | "KEYS") || check_arity(&name, args.len()).is_err() {
                queue.aborted = true;
                return Err(Reply::err("unsupported or invalid command in MULTI"));
            }
            let bytes = args.iter().map(|part| part.len() + 32).sum::<usize>();
            if queue.items.len() >= MAX_QUEUED || queue.bytes + bytes > transport::REQUEST_MAX {
                queue.aborted = true;
                return Err(Reply::Error("BUSYERR transaction queue limit reached".into()));
            }
            queue.bytes += bytes;
            queue.items.push(args);
            return Ok(Reply::Simple("QUEUED".into()));
        }
        let need = need.ok_or_else(|| Reply::err("command unavailable in hierarchical RESP"))?;
        check_arity(&name, args.len())?;
        let lane = if name == "GET" { Class::Read } else { Class::Data };
        let _permit = inst.native_admission.enter_bounded(lane, transport::WAIT).map_err(error)?;
        self.sess
            .with_managed_key("resp", need, |sess| {
                if need == Need::Write {
                    sess.begin()?;
                }
                let result = Scalar { sess }.run(&name, &args);
                if need == Need::Write {
                    if result.is_ok() {
                        sess.commit()?;
                    } else {
                        sess.rollback();
                    }
                }
                Ok(result)
            })
            .map_err(error)?
    }

    fn auth(&mut self, user: &str, password: &[u8]) -> Res {
        let pass = text(password)?;
        if user.len() + pass.len() > 4096 {
            return Err(Reply::Error("WRONGPASS invalid credentials".into()));
        }
        match transport::login(&mut self.sess, user, &pass) {
            Ok(()) => Ok(Reply::ok()),
            Err(KivaError::Denied(_) | KivaError::NotFound(_)) => {
                Err(Reply::Error("WRONGPASS invalid credentials".into()))
            }
            Err(e) => Err(error(e)),
        }
    }

    fn hello(&self) -> Reply {
        Reply::Map(vec![
            (Reply::bulk("server"), Reply::bulk("kivadb")),
            (Reply::bulk("version"), Reply::bulk(crate::instance::VERSION)),
            (Reply::bulk("proto"), Reply::Int(if self.resp3 { 3 } else { 2 })),
            (Reply::bulk("id"), Reply::Int(self.sess.id as i64)),
            (Reply::bulk("mode"), Reply::bulk("standalone")),
            (Reply::bulk("role"), Reply::bulk("master")),
            (Reply::bulk("modules"), Reply::array(vec![])),
        ])
    }

    fn exec(&mut self) -> Res {
        let inst = self.sess.inst.clone();
        let _permit = inst.native_admission.enter_bounded(Class::Data, transport::WAIT).map_err(error)?;
        let queue = self.queued.take().ok_or_else(|| Reply::err("EXEC without MULTI"))?;
        if queue.aborted {
            return Err(Reply::Error("EXECABORT transaction discarded because of previous errors".into()));
        }
        self.sess
            .with_managed_key("resp-exec", Need::Write, |sess| -> Result<Res> {
                sess.begin()?;
                let mut budget = transport::REPLY_MAX - 32;
                let mut results = Vec::with_capacity(queue.items.len());
                for args in queue.items {
                    let name = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
                    let result = Scalar { sess }.run(&name, &args).unwrap_or_else(|e| e);
                    if matches!(&result, Reply::Error(m) if m.starts_with("ERR_OUTCOME_UNKNOWN")) {
                        sess.rollback();
                        return Ok(Err(Reply::Error("EXECABORT transaction discarded after a storage error".into())));
                    }
                    if matches!(&result, Reply::Error(m) if m.starts_with("ERR_ADAPTER_LIMIT"))
                        || charge(&mut budget, &result).is_err()
                    {
                        sess.rollback();
                        return Ok(Err(limit()));
                    }
                    results.push(result);
                }
                sess.commit()?;
                Ok(Ok(Reply::array(results)))
            })
            .map_err(error)?
    }
}

fn protocol(bytes: &[u8]) -> std::result::Result<bool, Reply> {
    match bytes {
        b"2" => Ok(false),
        b"3" => Ok(true),
        _ => Err(Reply::Error("NOPROTO unsupported protocol version".into())),
    }
}
