// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Master -> replica replication of the default workspace.
//!
//! A replica connects to the master's TCP port, receives a full snapshot and
//! then the live stream of operations, acknowledging each one. Operations are
//! absolute (`set key = value`, `del key`), so replaying one twice is harmless.

use crate::engine::Op;
use crate::error::{KivaError, Result};
use crate::instance::Instance;
use crate::proto::*;
use crate::util::now_secs;
use crate::value::Type;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

enum ReplMsg {
    Op(u64, Op),
    Promote,
}

struct ReplicaConn {
    id: u64,
    addr: String,
    listen: String,
    tx: Sender<ReplMsg>,
    acked: Arc<AtomicU64>,
    connected_at: i64,
}

#[derive(Clone, Debug)]
pub struct ReplicaInfo {
    pub id: u64,
    pub addr: String,
    /// Address the replica itself listens on (used by `failover`).
    pub listen: String,
    pub acked: u64,
    pub connected_at: i64,
}

/// State of this node's link to its master (replica side).
#[derive(Clone, Debug, Default)]
pub struct Link {
    pub master: String,
    pub status: String,
    pub applied_seq: u64,
    pub applied_ops: u64,
    pub last_sync: Option<i64>,
    pub running: bool,
}

pub struct ReplState {
    seq: AtomicU64,
    replicas: Mutex<Vec<ReplicaConn>>,
    ack_lock: Mutex<()>,
    ack_cv: Condvar,
    replica_mode: AtomicBool,
    link: Mutex<Link>,
    generation: AtomicU64,
}

impl Default for ReplState {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplState {
    pub fn new() -> ReplState {
        ReplState {
            seq: AtomicU64::new(0),
            replicas: Mutex::new(Vec::new()),
            ack_lock: Mutex::new(()),
            ack_cv: Condvar::new(),
            replica_mode: AtomicBool::new(false),
            link: Mutex::new(Link::default()),
            generation: AtomicU64::new(0),
        }
    }

    pub fn is_replica(&self) -> bool {
        self.replica_mode.load(Ordering::SeqCst)
    }

    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }

    pub fn link(&self) -> Link {
        self.link.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_status(&self, status: impl Into<String>) {
        self.link.lock().unwrap_or_else(|e| e.into_inner()).status = status.into();
    }

    /// Called by the engine listener, in commit order.
    pub fn broadcast(&self, op: &Op) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut replicas = self.replicas.lock().unwrap_or_else(|e| e.into_inner());
        if replicas.is_empty() {
            return;
        }
        replicas.retain(|r| r.tx.send(ReplMsg::Op(seq, op.clone())).is_ok());
    }

    pub fn replicas(&self) -> Vec<ReplicaInfo> {
        self.replicas
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|r| ReplicaInfo {
                id: r.id,
                addr: r.addr.clone(),
                listen: r.listen.clone(),
                acked: r.acked.load(Ordering::SeqCst),
                connected_at: r.connected_at,
            })
            .collect()
    }

    fn unregister(&self, id: u64) {
        self.replicas.lock().unwrap_or_else(|e| e.into_inner()).retain(|r| r.id != id);
        self.ack_cv.notify_all();
    }

    pub fn drop_replicas(&self) {
        self.replicas.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Blocks until `n` replicas have acknowledged everything written so far.
    /// Returns how many actually did before the timeout.
    pub fn wait(&self, n: usize, timeout: Duration) -> usize {
        let target = self.seq();
        let deadline = Instant::now() + timeout;
        let count = |s: &ReplState| {
            s.replicas
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|r| r.acked.load(Ordering::SeqCst) >= target)
                .count()
        };
        let mut guard = self.ack_lock.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let c = count(self);
            let now = Instant::now();
            if c >= n || now >= deadline {
                return c;
            }
            guard = self
                .ack_cv
                .wait_timeout(guard, (deadline - now).min(Duration::from_millis(100)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Asks the given replica (by id or address) to become a master.
    /// Returns the address it listens on.
    pub fn send_promote(&self, target: &str) -> Result<(u64, String)> {
        let replicas = self.replicas.lock().unwrap_or_else(|e| e.into_inner());
        let r = replicas
            .iter()
            .find(|r| r.id.to_string() == target || r.addr == target || r.listen == target)
            .ok_or_else(|| KivaError::NotFound(format!("no connected replica matches '{target}'")))?;
        r.tx.send(ReplMsg::Promote).map_err(|_| KivaError::Other("replica connection is gone".into()))?;
        Ok((r.id, r.listen.clone()))
    }

    pub fn has_replica(&self, id: u64) -> bool {
        self.replicas.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|r| r.id == id)
    }

    /// Stops following a master. The node stays read-only until promoted.
    pub fn stop_replica(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut link = self.link.lock().unwrap_or_else(|e| e.into_inner());
        if link.running {
            link.running = false;
            link.status = "stopped".into();
        }
    }

    /// Turns this node back into a writable master.
    pub fn promote(&self) {
        self.stop_replica();
        self.replica_mode.store(false, Ordering::SeqCst);
        self.set_status("promoted to master");
    }
}

fn set_payload(seq: u64, key: &str, ty: Type, msgpack: bool, timestamp: i64, expires_at: i64, raw: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(28 + key.len() + raw.len());
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&timestamp.to_be_bytes());
    p.extend_from_slice(&expires_at.to_be_bytes());
    p.push(ty as u8);
    p.push(msgpack as u8);
    p.extend_from_slice(&(key.len() as u16).to_be_bytes());
    p.extend_from_slice(key.as_bytes());
    p.extend_from_slice(raw);
    p
}

/// Master side: feeds one replica until it disconnects.
pub fn serve_replica(inst: &Arc<Instance>, mut stream: TcpStream, peer: String, listen: String) {
    let Ok(engine) = inst.engine("default") else { return };
    let (tx, rx) = mpsc::channel();
    let acked = Arc::new(AtomicU64::new(0));
    let id = inst.next_id();
    // Register before taking the snapshot so no operation can slip between
    // the two: anything committed from now on is queued for this replica.
    inst.repl.replicas.lock().unwrap_or_else(|e| e.into_inner()).push(ReplicaConn {
        id,
        addr: peer,
        listen,
        tx,
        acked: acked.clone(),
        connected_at: now_secs(),
    });

    let alive = Arc::new(AtomicBool::new(true));
    if let Ok(mut reader) = stream.try_clone() {
        let (alive, acked, inst) = (alive.clone(), acked.clone(), inst.clone());
        let _ = std::thread::Builder::new().name("kiva-repl-ack".into()).spawn(move || {
            while let Ok((code, payload)) = read_frame(&mut reader) {
                if code == CMD_REPL_ACK && payload.len() >= 8 {
                    if let Ok(seq) = Reader::new(&payload).u64() {
                        acked.fetch_max(seq, Ordering::SeqCst);
                        inst.repl.ack_cv.notify_all();
                    }
                }
            }
            alive.store(false, Ordering::SeqCst);
            // The replica is gone: forget it now. Dropping its queue also
            // wakes the sending side up.
            inst.repl.unregister(id);
        });
    }

    let run = |stream: &mut TcpStream| -> std::io::Result<()> {
        write_frame(stream, R_SNAP_BEGIN, &[])?;
        for (key, _) in engine.range(None, None, false, 0) {
            if let Ok(Some((raw, e))) = engine.get_raw(&key) {
                let p = set_payload(0, &key, e.ty, e.is_msgpack(), e.timestamp, e.expires_at, &raw);
                write_frame(stream, R_SET, &p)?;
            }
        }
        write_frame(stream, R_SNAP_END, &inst.repl.seq().to_be_bytes())?;
        loop {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(ReplMsg::Op(seq, op)) => match op {
                    Op::Set { key, ty, raw, msgpack, timestamp, expires_at, .. } => {
                        let p = set_payload(seq, &key, ty, msgpack, timestamp, expires_at, &raw);
                        write_frame(stream, R_SET, &p)?;
                    }
                    Op::Del { key } => {
                        let mut p = seq.to_be_bytes().to_vec();
                        p.extend_from_slice(key.as_bytes());
                        write_frame(stream, R_DEL, &p)?;
                    }
                    Op::Reset => write_frame(stream, R_RESET, &seq.to_be_bytes())?,
                },
                Ok(ReplMsg::Promote) => write_frame(stream, R_PROMOTE, &[])?,
                Err(RecvTimeoutError::Timeout) => {
                    if !alive.load(Ordering::SeqCst) || inst.shutting_down.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    };
    let _ = run(&mut stream);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    inst.repl.unregister(id);
}

/// Replica side: starts (or restarts) following `host:port`.
pub fn start_replica(inst: &Arc<Instance>, host: &str, port: u16) {
    let repl = &inst.repl;
    let my_gen = repl.generation.fetch_add(1, Ordering::SeqCst) + 1;
    repl.replica_mode.store(true, Ordering::SeqCst);
    {
        let mut link = repl.link.lock().unwrap_or_else(|e| e.into_inner());
        *link =
            Link { master: format!("{host}:{port}"), status: "connecting".into(), running: true, ..Default::default() };
    }
    let inst = inst.clone();
    let addr = format!("{host}:{port}");
    let _ = std::thread::Builder::new().name("kiva-replica".into()).spawn(move || {
        let active = |inst: &Instance| {
            inst.repl.generation.load(Ordering::SeqCst) == my_gen && !inst.shutting_down.load(Ordering::SeqCst)
        };
        while active(&inst) {
            if let Err(e) = follow(&inst, &addr, &|| active(&inst)) {
                if active(&inst) {
                    inst.repl.set_status(format!("disconnected ({e}); retrying"));
                }
            }
            for _ in 0..20 {
                if !active(&inst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
}

fn follow(inst: &Arc<Instance>, addr: &str, active: &dyn Fn() -> bool) -> Result<()> {
    let (master_tls, fingerprint) = {
        let c = inst.cfg();
        (c.get_bool("replication", "master_tls"), c.get("replication", "master_fingerprint"))
    };
    let mut stream = if master_tls {
        if fingerprint.trim().is_empty() {
            return Err(KivaError::Invalid(
                "replication.master_tls is on: set replication.master_fingerprint to the master's (kivadb tls)".into(),
            ));
        }
        crate::tls::connect(addr, Some(&fingerprint), Duration::from_secs(5))?.stream
    } else {
        let sock =
            addr.to_socket_addrs()?.next().ok_or_else(|| KivaError::Invalid(format!("cannot resolve '{addr}'")))?;
        TcpStream::connect_timeout(&sock, Duration::from_secs(5))?
    };
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let _ = stream.set_nodelay(true);

    let (user, password, listen) = {
        let c = inst.cfg();
        (
            c.get("replication", "master_user"),
            c.get("replication", "master_password"),
            format!("{}:{}", c.get("network", "bind"), inst.tcp_port.load(Ordering::SeqCst)),
        )
    };
    write_frame(&mut stream, CMD_HELLO, &[])?;
    let (_, hello) = read_frame_with(&mut stream, active)?;
    if String::from_utf8_lossy(&hello).contains("auth=required") {
        let mut p = user.into_bytes();
        p.push(0);
        p.extend_from_slice(password.as_bytes());
        write_frame(&mut stream, CMD_AUTH, &p)?;
        let (st, msg) = read_frame_with(&mut stream, active)?;
        if st != ST_OK {
            return Err(KivaError::Denied(format!("master refused login: {}", String::from_utf8_lossy(&msg))));
        }
    }
    write_frame(&mut stream, CMD_REPL_SYNC, listen.as_bytes())?;

    let engine = inst.engine("default")?;
    let applied = |seq: u64, stream: &mut TcpStream| -> std::io::Result<()> {
        if seq > 0 {
            let mut link = inst.repl.link.lock().unwrap_or_else(|e| e.into_inner());
            link.applied_seq = link.applied_seq.max(seq);
            link.applied_ops += 1;
            drop(link);
            write_frame(stream, CMD_REPL_ACK, &seq.to_be_bytes())?;
        }
        Ok(())
    };
    loop {
        let (code, payload) = read_frame_with(&mut stream, active)?;
        // `replica stop` takes effect immediately, even mid-stream.
        if !active() {
            return Ok(());
        }
        let mut r = Reader::new(&payload);
        match code {
            R_SNAP_BEGIN => {
                inst.repl.set_status("receiving snapshot");
                engine.reset()?;
            }
            R_SET => {
                let seq = r.u64()?;
                let timestamp = r.i64()?;
                let expires_at = r.i64()?;
                let ty = Type::from_u8(r.u8()?).ok_or_else(|| KivaError::Corrupted("bad type in stream".into()))?;
                let msgpack = r.u8()? != 0;
                let key = r.str16()?;
                engine.put_raw(&key, ty, r.rest(), msgpack, timestamp, expires_at)?;
                applied(seq, &mut stream)?;
            }
            R_DEL => {
                let seq = r.u64()?;
                let key = String::from_utf8_lossy(r.rest()).into_owned();
                engine.delete(&key)?;
                applied(seq, &mut stream)?;
            }
            R_RESET => {
                let seq = r.u64()?;
                engine.reset()?;
                applied(seq, &mut stream)?;
            }
            R_SNAP_END => {
                let seq = r.u64()?;
                {
                    let mut link = inst.repl.link.lock().unwrap_or_else(|e| e.into_inner());
                    link.status = "connected".into();
                    link.last_sync = Some(now_secs());
                    link.applied_seq = link.applied_seq.max(seq);
                }
                write_frame(&mut stream, CMD_REPL_ACK, &seq.to_be_bytes())?;
            }
            R_PROMOTE => {
                inst.repl.promote();
                let _ = inst.cfg_mut().set("replication", "role", "master");
                inst.audit.log("system", "promoted to master by failover", true);
                let _ = stream.flush();
                return Ok(());
            }
            ST_ERR => {
                return Err(KivaError::Other(String::from_utf8_lossy(&payload).into_owned()));
            }
            _ => {}
        }
    }
}
