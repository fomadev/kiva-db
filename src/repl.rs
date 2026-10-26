// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Master -> replica replication of every workspace.
//!
//! A replica connects to the master's TCP port, receives a full snapshot and
//! then the live stream of operations, acknowledging each one. Operations are
//! absolute (`set key = value`, `del key`), so replaying one twice is harmless.
//!
//! The stream carries every workspace: the snapshot starts with the list of
//! the master's workspaces (`R_WORKSPACES`), then each one's keys after an
//! `R_USE <name>`; live operations name their workspace the same way when
//! it changes. Operations of all workspaces share one sequence, numbered in
//! the order they are queued, so an acknowledgement covers everything before
//! it whatever the workspace. A replica says it understands this with
//! `repl=2` in its hello; the master refuses one that does not, since it
//! would apply every workspace's operations to its default one.
//!
//! The system database (users, stored scripts, triggers) travels the same
//! way, under the name [`SYSTEM`], so that a replica promoted by a
//! failover has the master's accounts, scripts and triggers.
//!
//! While the stream is idle, the master sends a heartbeat every
//! `failover.heartbeat_ms` listing the nodes of the group; the automatic
//! failover ([`crate::failover`]) builds on it.

use crate::engine::Op;
use crate::error::{KivaError, Result};
use crate::instance::Instance;
use crate::proto::*;
use crate::util::now_secs;
use crate::value::Type;
use serde_json::{json, Value as Json};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

enum ReplMsg {
    /// Sequence, workspace, operation.
    Op(u64, String, Op),
    /// A workspace was created (`false`) or unregistered (`true`).
    Workspace(String, bool),
    Promote,
}

/// What a replica puts in its hello to receive every workspace.
const ALL_WORKSPACES: &str = "repl=2";

/// The name the system database goes by in the stream; no workspace can
/// have it, since workspace names hold letters, digits, `_` and `-` only.
pub const SYSTEM: &str = "@system";

struct ReplicaConn {
    id: u64,
    addr: String,
    listen: String,
    /// The replica's node id, to tell it apart in the group.
    node: String,
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
    group: Mutex<Group>,
}

/// What this node knows of its replication group, for the failover.
#[derive(Clone, Debug, Default)]
struct Group {
    /// Random id of this node, kept in `failover.node_id`.
    node: String,
    /// This node's address as the others reach it (told by the master).
    advertised: String,
    epoch: u64,
    voted: u64,
    /// The other replicas of the current master.
    peers: Vec<String>,
    /// Every node ever seen in the group, kept in `failover.nodes`.
    known: Vec<String>,
    last_contact: Option<Instant>,
    event: Option<String>,
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
            group: Mutex::new(Group::default()),
        }
    }

    fn group(&self) -> std::sync::MutexGuard<'_, Group> {
        self.group.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads (or creates) this node's identity and group state from the configuration.
    pub fn load_identity(&self, inst: &Instance) {
        let mut cfg = inst.cfg_mut();
        let mut node = cfg.get("failover", "node_id");
        if node.is_empty() {
            node = format!("{:016x}", crate::util::random_u64());
            let _ = cfg.set("failover", "node_id", &node);
        }
        let mut g = self.group();
        g.node = node;
        g.epoch = g.epoch.max(cfg.get_u64("failover", "epoch"));
        g.voted = g.voted.max(cfg.get_u64("failover", "voted"));
        g.known = cfg.get("failover", "nodes").split(',').filter(|s| !s.is_empty()).map(String::from).collect();
    }

    pub fn node_id(&self) -> String {
        self.group().node.clone()
    }

    pub fn epoch(&self) -> u64 {
        self.group().epoch
    }

    /// Moves to a newer epoch (never back), and remembers it.
    pub fn set_epoch(&self, inst: &Instance, epoch: u64) {
        let changed = {
            let mut g = self.group();
            let changed = epoch > g.epoch;
            g.epoch = g.epoch.max(epoch);
            changed
        };
        if changed {
            let _ = inst.cfg_mut().set("failover", "epoch", &epoch.to_string());
        }
    }

    pub fn voted(&self) -> u64 {
        self.group().voted
    }

    pub fn set_voted(&self, inst: &Instance, epoch: u64) {
        self.group().voted = epoch;
        let _ = inst.cfg_mut().set("failover", "voted", &epoch.to_string());
    }

    /// When this replica last heard from its master.
    pub fn last_contact(&self) -> Option<Instant> {
        self.group().last_contact
    }

    fn touch(&self) {
        self.group().last_contact = Some(Instant::now());
    }

    /// This node's address as the group knows it (`bind:port` until told).
    pub fn advertised(&self) -> String {
        self.group().advertised.clone()
    }

    /// The other replicas of the current master.
    pub fn peers(&self) -> Vec<String> {
        self.group().peers.clone()
    }

    /// Every other node seen in the group.
    pub fn known_nodes(&self) -> Vec<String> {
        let g = self.group();
        g.known.iter().filter(|n| **n != g.advertised).cloned().collect()
    }

    pub fn event(&self) -> Option<String> {
        self.group().event.clone()
    }

    pub fn set_event(&self, event: impl Into<String>) {
        self.group().event = Some(format!("{} ({})", event.into(), crate::util::fmt_time(now_secs())));
    }

    /// Adds nodes to the ones remembered in `failover.nodes`.
    fn remember(&self, inst: &Instance, nodes: &[String]) {
        let known = {
            let mut g = self.group();
            let before = g.known.len();
            for n in nodes {
                if !n.is_empty() && !g.known.contains(n) {
                    g.known.push(n.clone());
                }
            }
            (g.known.len() != before).then(|| g.known.join(","))
        };
        if let Some(list) = known {
            let _ = inst.cfg_mut().set("failover", "nodes", &list);
        }
    }

    /// The heartbeat a master sends: its epoch and the replicas of the group.
    fn heartbeat(&self, inst: &Instance) -> Vec<u8> {
        let replicas = self.replicas.lock().unwrap_or_else(|e| e.into_inner());
        let nodes: Vec<Json> = replicas
            .iter()
            .map(|r| json!({ "addr": r.listen, "node": r.node, "acked": r.acked.load(Ordering::SeqCst) }))
            .collect();
        let addrs: Vec<String> = replicas.iter().map(|r| r.listen.clone()).collect();
        drop(replicas);
        self.remember(inst, &addrs);
        json!({ "epoch": self.epoch(), "seq": self.seq(), "nodes": nodes }).to_string().into_bytes()
    }

    /// A replica learns the group from its master's heartbeat.
    fn learn(&self, inst: &Instance, master: &str, payload: &[u8]) {
        let Ok(j) = serde_json::from_slice::<Json>(payload) else { return };
        self.set_epoch(inst, j["epoch"].as_u64().unwrap_or(0));
        let me = self.node_id();
        let mut peers = Vec::new();
        for n in j["nodes"].as_array().into_iter().flatten() {
            let addr = n["addr"].as_str().unwrap_or("").to_string();
            if n["node"].as_str() == Some(me.as_str()) {
                self.group().advertised = addr;
            } else if !addr.is_empty() {
                peers.push(addr);
            }
        }
        let mut all = peers.clone();
        all.push(master.to_string());
        self.group().peers = peers;
        self.remember(inst, &all);
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

    /// Called by the engine listeners, in commit order within a workspace.
    pub fn broadcast(&self, workspace: &str, op: &Op) {
        // Numbered under the lock, so that every replica receives the
        // operations in sequence order, whichever workspaces they come from.
        let mut replicas = self.replicas.lock().unwrap_or_else(|e| e.into_inner());
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        if replicas.is_empty() {
            return;
        }
        replicas.retain(|r| r.tx.send(ReplMsg::Op(seq, workspace.to_string(), op.clone())).is_ok());
    }

    /// A workspace was created or unregistered on this node.
    pub fn workspace_changed(&self, name: &str, removed: bool) {
        let mut replicas = self.replicas.lock().unwrap_or_else(|e| e.into_inner());
        replicas.retain(|r| r.tx.send(ReplMsg::Workspace(name.to_string(), removed)).is_ok());
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

/// An address others can reach: a wildcard host (`0.0.0.0`) is replaced by
/// the address the connection came from.
fn reachable(listen: &str, peer: &str) -> String {
    match (listen.rsplit_once(':'), peer.rsplit_once(':')) {
        (Some(("" | "0.0.0.0" | "::" | "[::]", port)), Some((peer_host, _))) => {
            format!("{peer_host}:{port}")
        }
        _ => listen.to_string(),
    }
}

/// Master side: feeds one replica until it disconnects. `hello` is what the
/// replica sent: `<listen address> node=<id>`.
pub fn serve_replica(inst: &Arc<Instance>, mut stream: TcpStream, peer: String, hello: String) {
    let mut words = hello.split_whitespace();
    let listen = words.next().unwrap_or_default().to_string();
    let (mut node, mut all) = (String::new(), false);
    for w in words {
        match w.strip_prefix("node=") {
            Some(n) => node = n.to_string(),
            None => all |= w == ALL_WORKSPACES,
        }
    }
    if !all {
        let msg = format!(
            "this master replicates every workspace: upgrade the replica to KivaDB {} or later",
            crate::instance::VERSION
        );
        let _ = write_frame(&mut stream, ST_ERR, msg.as_bytes());
        return;
    }
    let listen = reachable(&listen, &peer);
    let (tx, rx) = mpsc::channel();
    let acked = Arc::new(AtomicU64::new(0));
    let id = inst.next_id();
    // Register before taking the snapshot so no operation can slip between
    // the two: anything committed from now on is queued for this replica.
    inst.repl.replicas.lock().unwrap_or_else(|e| e.into_inner()).push(ReplicaConn {
        id,
        addr: peer,
        listen,
        node,
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
        let names: Vec<String> = inst.workspaces().into_iter().map(|(n, _)| n).collect();
        write_frame(stream, R_WORKSPACES, names.join("\n").as_bytes())?;
        for name in &names {
            let Ok(engine) = inst.engine(name) else { continue };
            write_frame(stream, R_USE, name.as_bytes())?;
            write_frame(stream, R_RESET, &0u64.to_be_bytes())?;
            for (key, _) in engine.range(None, None, false, 0) {
                if let Ok(Some((raw, e))) = engine.get_raw(&key) {
                    let p = set_payload(0, &key, e.ty, e.is_msgpack(), e.timestamp, e.expires_at, &raw);
                    write_frame(stream, R_SET, &p)?;
                }
            }
        }
        write_frame(stream, R_USE, SYSTEM.as_bytes())?;
        write_frame(stream, R_RESET, &0u64.to_be_bytes())?;
        for (key, _) in inst.system.range(None, None, false, 0) {
            if let Ok(Some((raw, e))) = inst.system.get_raw(&key) {
                let p = set_payload(0, &key, e.ty, e.is_msgpack(), e.timestamp, e.expires_at, &raw);
                write_frame(stream, R_SET, &p)?;
            }
        }
        write_frame(stream, R_SNAP_END, &inst.repl.seq().to_be_bytes())?;
        // The workspace the replica applies operations to.
        let mut current = SYSTEM.to_string();
        let mut last_beat = Instant::now() - Duration::from_secs(3600);
        loop {
            let beat = crate::failover::Settings::of(inst).heartbeat;
            if last_beat.elapsed() >= beat {
                write_frame(stream, R_PING, &inst.repl.heartbeat(inst))?;
                last_beat = Instant::now();
            }
            match rx.recv_timeout(beat.min(Duration::from_secs(1))) {
                Ok(ReplMsg::Op(seq, ws, op)) => {
                    if ws != current {
                        write_frame(stream, R_USE, ws.as_bytes())?;
                        current = ws;
                    }
                    match op {
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
                    }
                }
                // A new workspace exists on the replica as soon as it is named.
                Ok(ReplMsg::Workspace(ws, false)) => {
                    write_frame(stream, R_USE, ws.as_bytes())?;
                    current = ws;
                }
                Ok(ReplMsg::Workspace(ws, true)) => write_frame(stream, R_WS_REMOVE, ws.as_bytes())?,
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

/// Connects to another node of the group and logs in with the replication
/// account (`replication.master_user` / `master_password`), over TLS when
/// `replication.master_tls` is on.
pub fn connect_node(
    inst: &Arc<Instance>,
    addr: &str,
    timeout: Duration,
    active: &dyn Fn() -> bool,
) -> Result<TcpStream> {
    let (master_tls, fingerprint, user, password) = {
        let c = inst.cfg();
        (
            c.get_bool("replication", "master_tls"),
            c.get("replication", "master_fingerprint"),
            c.get("replication", "master_user"),
            c.get("replication", "master_password"),
        )
    };
    let mut stream = if master_tls {
        if fingerprint.trim().is_empty() {
            return Err(KivaError::Invalid(
                "replication.master_tls is on: set replication.master_fingerprint to the master's (kivadb tls)".into(),
            ));
        }
        crate::tls::connect(addr, Some(&fingerprint), timeout)?.stream
    } else {
        let sock =
            addr.to_socket_addrs()?.next().ok_or_else(|| KivaError::Invalid(format!("cannot resolve '{addr}'")))?;
        TcpStream::connect_timeout(&sock, timeout)?
    };
    stream.set_read_timeout(Some(Duration::from_millis(200).min(timeout)))?;
    let _ = stream.set_nodelay(true);
    write_frame(&mut stream, CMD_HELLO, &[])?;
    let (_, hello) = read_frame_with(&mut stream, active)?;
    if String::from_utf8_lossy(&hello).contains("auth=required") {
        let mut p = user.into_bytes();
        p.push(0);
        p.extend_from_slice(password.as_bytes());
        write_frame(&mut stream, CMD_AUTH, &p)?;
        let (st, msg) = read_frame_with(&mut stream, active)?;
        if st != ST_OK {
            return Err(KivaError::Denied(format!("{addr} refused login: {}", String::from_utf8_lossy(&msg))));
        }
    }
    Ok(stream)
}

fn follow(inst: &Arc<Instance>, addr: &str, active: &dyn Fn() -> bool) -> Result<()> {
    let mut stream = connect_node(inst, addr, Duration::from_secs(5), active)?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    let listen = {
        let c = inst.cfg();
        format!("{}:{}", c.get("network", "bind"), inst.tcp_port.load(Ordering::SeqCst))
    };
    if inst.repl.advertised().is_empty() {
        inst.repl.group().advertised = listen.clone();
    }
    let hello = format!("{listen} node={} {ALL_WORKSPACES}", inst.repl.node_id());
    write_frame(&mut stream, CMD_REPL_SYNC, hello.as_bytes())?;
    inst.repl.touch();

    // A master older than multi-workspace replication sends the default
    // workspace only, without R_USE: everything then goes there.
    let mut engine = inst.engine("default")?;
    let mut on_system = false;
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
        inst.repl.touch();
        let mut r = Reader::new(&payload);
        match code {
            R_SNAP_BEGIN => {
                inst.repl.set_status("receiving snapshot");
                engine = inst.engine("default")?;
                on_system = false;
                engine.reset()?;
            }
            R_WORKSPACES => {
                let names: Vec<String> =
                    String::from_utf8_lossy(&payload).lines().map(str::to_string).filter(|n| !n.is_empty()).collect();
                for name in &names {
                    inst.ensure_workspace(name)?;
                }
                // The replica mirrors the master: its other workspaces are
                // unregistered, their files left on disk.
                for (name, _) in inst.workspaces() {
                    if name != "default" && !names.contains(&name) {
                        inst.remove_workspace(&name)?;
                    }
                }
            }
            R_USE => {
                let name = String::from_utf8_lossy(&payload).into_owned();
                on_system = name == SYSTEM;
                engine = if on_system {
                    inst.system.clone()
                } else {
                    inst.ensure_workspace(&name)?;
                    inst.engine(&name)?
                };
            }
            R_WS_REMOVE => {
                let name = String::from_utf8_lossy(&payload).into_owned();
                if name != "default" {
                    inst.remove_workspace(&name)?;
                }
            }
            R_SET => {
                let seq = r.u64()?;
                let timestamp = r.i64()?;
                let expires_at = r.i64()?;
                let ty = Type::from_u8(r.u8()?).ok_or_else(|| KivaError::Corrupted("bad type in stream".into()))?;
                let msgpack = r.u8()? != 0;
                let key = r.str16()?;
                engine.put_raw(&key, ty, r.rest(), msgpack, timestamp, expires_at)?;
                if on_system && key.starts_with("trigger:") {
                    inst.reload_triggers();
                }
                applied(seq, &mut stream)?;
            }
            R_DEL => {
                let seq = r.u64()?;
                let key = String::from_utf8_lossy(r.rest()).into_owned();
                engine.delete(&key)?;
                if on_system && key.starts_with("trigger:") {
                    inst.reload_triggers();
                }
                applied(seq, &mut stream)?;
            }
            R_RESET => {
                let seq = r.u64()?;
                engine.reset()?;
                if on_system {
                    inst.reload_triggers();
                }
                applied(seq, &mut stream)?;
            }
            R_SNAP_END => {
                let seq = r.u64()?;
                inst.reload_triggers();
                {
                    let mut link = inst.repl.link.lock().unwrap_or_else(|e| e.into_inner());
                    link.status = "connected".into();
                    link.last_sync = Some(now_secs());
                    link.applied_seq = link.applied_seq.max(seq);
                }
                write_frame(&mut stream, CMD_REPL_ACK, &seq.to_be_bytes())?;
            }
            R_PING => inst.repl.learn(inst, addr, &payload),
            R_PROMOTE => {
                inst.repl.promote();
                // A planned handover opens a new epoch too: the old master
                // learns it from the new master's heartbeats.
                inst.repl.set_epoch(inst, inst.repl.epoch() + 1);
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
