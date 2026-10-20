// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! TCP server. Each connection gets its own thread and session; the number
//! of commands executing at once is bounded by `[server] threads`.

use crate::error::{KivaError, Result};
use crate::exec::{validate_key_name, Action};
use crate::instance::{Instance, VERSION};
use crate::proto::*;
use crate::repl;
use crate::session::{Need, Session};
use crate::util::now_secs;
use crate::value::{Type, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

/// Starts listening and returns the bound port (useful with port 0).
pub fn start_tcp(inst: &Arc<Instance>, bind: &str, port: u16) -> Result<u16> {
    let listener = TcpListener::bind((bind, port))
        .map_err(|e| KivaError::Other(format!("cannot listen on {bind}:{port}: {e}")))?;
    let port = listener.local_addr()?.port();
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
            let inst = inst.clone();
            let _ = std::thread::Builder::new().name("kiva-conn".into()).spawn(move || {
                let max = inst.cfg().get_u64("server", "max_clients") as usize;
                if inst.clients.fetch_add(1, Ordering::SeqCst) >= max {
                    let mut s = stream;
                    let _ = s.write_all(b"Error: max clients reached\n");
                } else if let Some((stream, peer)) = crate::tls::accept(&inst, stream, &refusal) {
                    handle_conn(&inst, stream, peer);
                }
                inst.clients.fetch_sub(1, Ordering::SeqCst);
            });
        }
    })?;
    Ok(port)
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
    let mut first = [0u8; 1];
    match stream.peek(&mut first) {
        Ok(1) => {}
        _ => return,
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
                let res = serde_json::from_slice::<serde_json::Value>(&payload)
                    .map_err(KivaError::from)
                    .and_then(|req| crate::api::handle(&mut sess, &req));
                match res {
                    Ok(j) => reply(ST_OK, j.to_string().as_bytes()),
                    Err(e) => reply(ST_ERR, e.to_string().as_bytes()),
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
    let mut r = Reader::new(payload);
    let ty = Type::from_u8(r.u8()?).ok_or_else(|| KivaError::Invalid("unknown type tag".into()))?;
    let ttl = r.u32()? as i64;
    let key = r.str16()?;
    validate_key_name(&key).map_err(KivaError::Invalid)?;
    let value = Value::decode(ty, r.rest(), false)
        .map_err(|e| KivaError::Invalid(format!("value does not match its type: {e}")))?;
    let expires_at = if ttl > 0 { now_secs() + ttl } else { 0 };
    sess.kv_put(&key, value, expires_at, false)
}
