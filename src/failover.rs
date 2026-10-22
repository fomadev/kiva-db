// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Automatic failover (`failover.auto`).
//!
//! **Detection.** The master sends a heartbeat to each replica every
//! `failover.heartbeat_ms`, with the list of nodes of the group, so that every
//! replica knows the others. A replica that hears nothing for
//! `failover.down_after_ms` suspects the master.
//!
//! **Agreement.** The suspecting replica first tries to reach the master
//! itself, then asks every other replica whether it sees the master too. A new
//! master is only chosen when a quorum of the group's nodes (by default a
//! majority, the master included in the count) says the master is gone: a
//! replica cut off from the others alone can never promote itself, so two
//! masters cannot appear on both sides of a network split.
//!
//! **Election.** Among the replicas that agree, the one that applied the most
//! of the master's stream wins (then the highest `failover.priority`, then the
//! smallest address). It asks the others for their vote in a new *epoch*; a
//! node grants one vote per epoch, only to a candidate at least as up to date
//! as itself. With a quorum of votes the candidate becomes master, and tells
//! every node, which then resynchronise from it.
//!
//! **Return of the old master.** A master with no replica connected asks the
//! nodes it knew whether a newer epoch exists; if so it was replaced, and it
//! becomes a replica of the new master. Writes it accepted while cut off are
//! replaced by the new master's data.

use crate::error::{KivaError, Result};
use crate::instance::Instance;
use crate::proto::*;
use serde_json::{json, Value as Json};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Settings of `[failover]`, read at every check so that changes apply at once.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub auto: bool,
    pub heartbeat: Duration,
    pub timeout: Duration,
    /// Nodes that must see the master down; 0 = a majority of the group.
    pub quorum: usize,
    /// 0: this node never becomes master by itself.
    pub priority: u64,
}

impl Settings {
    pub fn of(inst: &Instance) -> Settings {
        let c = inst.cfg();
        Settings {
            auto: c.get_bool("failover", "auto"),
            heartbeat: Duration::from_millis(c.get_u64("failover", "heartbeat_ms").max(50)),
            timeout: Duration::from_millis(c.get_u64("failover", "down_after_ms").max(200)),
            quorum: c.get_u64("failover", "quorum") as usize,
            priority: c.get_u64("failover", "priority"),
        }
    }

    /// Nodes needed to replace the master of a group of `nodes` nodes.
    pub fn needed(&self, nodes: usize) -> usize {
        needed(nodes, self.quorum)
    }
}

/// A majority of `nodes`, unless `configured` says otherwise.
pub fn needed(nodes: usize, configured: usize) -> usize {
    if configured > 0 {
        configured
    } else {
        nodes / 2 + 1
    }
}

/// A replica that could become the master.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub addr: String,
    /// Position in the master's stream it has applied.
    pub seq: u64,
    pub priority: u64,
}

/// The replica that should take over: the most up to date, then the highest
/// priority, then the smallest address, so that every node picks the same one.
pub fn best(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates
        .iter()
        .filter(|c| c.priority > 0)
        .max_by(|a, b| a.seq.cmp(&b.seq).then(a.priority.cmp(&b.priority)).then(b.addr.cmp(&a.addr)))
}

/// Whether a node in epoch `epoch` that already voted in `voted` grants its
/// vote for `request_epoch` to a candidate at `candidate_seq`.
pub fn grants(epoch: u64, voted: u64, request_epoch: u64, master_down: bool, own_seq: u64, candidate_seq: u64) -> bool {
    request_epoch > epoch && request_epoch > voted && master_down && candidate_seq >= own_seq
}

fn host_port(addr: &str) -> Option<(String, u16)> {
    let (h, p) = addr.rsplit_once(':')?;
    Some((h.trim_matches(['[', ']']).to_string(), p.parse().ok().filter(|p: &u16| *p != 0)?))
}

/// Sends a failover request to the node at `addr` and returns its answer.
/// A node that does not answer within `timeout` (twice, to connect then to
/// answer) counts as unreachable.
pub fn call(inst: &Arc<Instance>, addr: &str, req: &Json, timeout: Duration) -> Result<Json> {
    let deadline = Instant::now() + timeout * 2;
    let in_time = || Instant::now() < deadline;
    let mut stream = crate::repl::connect_node(inst, addr, timeout, &in_time)?;
    write_frame(&mut stream, CMD_FAILOVER, req.to_string().as_bytes())?;
    let (st, payload) = read_frame_with(&mut stream, &in_time)?;
    if st != ST_OK {
        return Err(KivaError::Other(String::from_utf8_lossy(&payload).into_owned()));
    }
    Ok(serde_json::from_slice(&payload)?)
}

/// Whether this replica has not heard from its master for `timeout`.
fn master_silent(inst: &Instance, timeout: Duration) -> bool {
    inst.repl.is_replica()
        && inst.repl.link().running
        && inst.repl.last_contact().is_none_or(|t| t.elapsed() >= timeout)
}

/// Answers another node's failover request (`CMD_FAILOVER`).
pub fn handle(inst: &Arc<Instance>, req: &Json) -> Json {
    let s = Settings::of(inst);
    let repl = &inst.repl;
    match req["op"].as_str() {
        Some("probe") => {
            let link = repl.link();
            json!({
                "node": repl.node_id(),
                "role": if repl.is_replica() { "replica" } else { "master" },
                "master": if repl.is_replica() { link.master } else { String::new() },
                "master_down": master_silent(inst, s.timeout),
                "seq": if repl.is_replica() { link.applied_seq } else { repl.seq() },
                "epoch": repl.epoch(),
                "priority": s.priority,
                "auto": s.auto,
            })
        }
        Some("vote") => {
            let request = req["epoch"].as_u64().unwrap_or(0);
            let seq = req["seq"].as_u64().unwrap_or(0);
            let granted = s.auto
                && grants(
                    repl.epoch(),
                    repl.voted(),
                    request,
                    master_silent(inst, s.timeout),
                    repl.link().applied_seq,
                    seq,
                );
            if granted {
                repl.set_voted(inst, request);
                repl.set_event(format!("voted for {} in epoch {request}", req["candidate"].as_str().unwrap_or("?")));
            }
            json!({ "granted": granted, "epoch": repl.epoch() })
        }
        Some("announce") => {
            let epoch = req["epoch"].as_u64().unwrap_or(0);
            let master = req["master"].as_str().unwrap_or("").to_string();
            if epoch >= repl.epoch() && !master.is_empty() {
                follow_new_master(inst, &master, epoch, "announced by the new master");
            }
            json!({ "ok": true, "epoch": repl.epoch() })
        }
        _ => json!({ "error": "unknown failover request" }),
    }
}

/// Makes this node a replica of `master` (from epoch `epoch` on).
fn follow_new_master(inst: &Arc<Instance>, master: &str, epoch: u64, why: &str) {
    let Some((host, port)) = host_port(master) else { return };
    inst.repl.set_epoch(inst, epoch);
    if inst.repl.is_replica() && inst.repl.link().master == master {
        return;
    }
    {
        let mut cfg = inst.cfg_mut();
        let _ = cfg.set("replication", "role", "replica");
        let _ = cfg.set("replication", "master_host", &host);
        let _ = cfg.set("replication", "master_port", &port.to_string());
    }
    inst.repl.drop_replicas();
    crate::repl::start_replica(inst, &host, port);
    let event = format!("following {master} (epoch {epoch}, {why})");
    inst.audit.log("system", &format!("failover: {event}"), true);
    inst.repl.set_event(event);
}

/// Starts the failure detector of a server. It does nothing while
/// `failover.auto` is off.
pub fn start(inst: &Arc<Instance>) {
    inst.repl.load_identity(inst);
    let inst = inst.clone();
    let _ = std::thread::Builder::new().name("kiva-failover".into()).spawn(move || {
        let mut next_try = Instant::now();
        let mut alone_since: Option<Instant> = Some(Instant::now() - Duration::from_secs(3600));
        while !inst.shutting_down.load(Ordering::SeqCst) {
            let s = Settings::of(&inst);
            std::thread::sleep(s.heartbeat.min(Duration::from_millis(500)));
            if !s.auto || Instant::now() < next_try {
                continue;
            }
            if inst.repl.is_replica() {
                alone_since = None;
                if master_silent(&inst, s.timeout) {
                    if let Err(e) = elect(&inst, &s) {
                        inst.repl.set_event(format!("no failover: {e}"));
                    }
                    // Spread the attempts of several replicas apart.
                    let jitter = crate::util::random_u64() % (s.timeout.as_millis() as u64).max(1);
                    next_try = Instant::now() + s.timeout + Duration::from_millis(jitter);
                }
            } else if inst.repl.replicas().is_empty() {
                // A master alone: was it replaced while it was away?
                let since = *alone_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= s.timeout {
                    check_replaced(&inst, &s);
                    alone_since = Some(Instant::now());
                }
            } else {
                alone_since = None;
            }
        }
    });
}

/// A master that lost its replicas asks the nodes it knew for a newer epoch.
fn check_replaced(inst: &Arc<Instance>, s: &Settings) {
    let me = inst.repl.node_id();
    for addr in inst.repl.known_nodes() {
        let Ok(r) = call(inst, &addr, &json!({ "op": "probe" }), s.heartbeat.max(Duration::from_secs(1))) else {
            continue;
        };
        if r["node"].as_str() == Some(me.as_str()) {
            continue;
        }
        let epoch = r["epoch"].as_u64().unwrap_or(0);
        if epoch <= inst.repl.epoch() {
            continue;
        }
        let master = match r["role"].as_str() {
            Some("master") => addr.clone(),
            _ => r["master"].as_str().unwrap_or("").to_string(),
        };
        if !master.is_empty() {
            follow_new_master(inst, &master, epoch, "this node was replaced while it was away");
            return;
        }
    }
}

/// Tries to replace a silent master; `Err` says why it did not happen.
fn elect(inst: &Arc<Instance>, s: &Settings) -> std::result::Result<(), String> {
    let repl = &inst.repl;
    let master = repl.link().master;
    let probe = json!({ "op": "probe" });
    let wait = s.heartbeat.max(Duration::from_secs(1));

    // 1. Only our link may be broken: look for the master ourselves.
    if call(inst, &master, &probe, wait).is_ok() {
        return Err(format!("{master} still answers; only the replication link was down"));
    }
    // 2. Do the other replicas see it down too?
    let me = repl.advertised();
    let my_seq = repl.link().applied_seq;
    let peers: Vec<String> = repl.peers().into_iter().filter(|p| *p != me).collect();
    let nodes = peers.len() + 2;
    let needed = s.needed(nodes);
    let mut agree = vec![Candidate { addr: me.clone(), seq: my_seq, priority: s.priority }];
    let mut highest_epoch = repl.epoch();
    for p in &peers {
        let Ok(r) = call(inst, p, &probe, wait) else { continue };
        highest_epoch = highest_epoch.max(r["epoch"].as_u64().unwrap_or(0));
        if r["role"] == "master" && r["epoch"].as_u64().unwrap_or(0) >= repl.epoch() {
            // Someone already took over: follow it.
            follow_new_master(inst, p, r["epoch"].as_u64().unwrap_or(0), "found during an election");
            return Ok(());
        }
        if r["master_down"] == true && r["auto"] == true && r["master"].as_str() == Some(master.as_str()) {
            agree.push(Candidate {
                addr: p.clone(),
                seq: r["seq"].as_u64().unwrap_or(0),
                priority: r["priority"].as_u64().unwrap_or(0),
            });
        }
    }
    if agree.len() < needed {
        return Err(format!(
            "{master} is unreachable, but only {} of the {needed} nodes needed see it down (group of {nodes})",
            agree.len()
        ));
    }
    // 3. The best placed replica takes over; the others wait for it.
    let winner = best(&agree).ok_or("no replica may become master (failover.priority is 0 everywhere)")?;
    if winner.addr != me {
        return Err(format!("waiting for {} (more up to date) to take over", winner.addr));
    }
    // 4. Votes for a new epoch.
    let epoch = highest_epoch.max(repl.voted()) + 1;
    repl.set_voted(inst, epoch);
    let mut votes = 1;
    let ask = json!({ "op": "vote", "epoch": epoch, "candidate": me, "seq": my_seq });
    for c in agree.iter().filter(|c| c.addr != me) {
        if let Ok(r) = call(inst, &c.addr, &ask, wait) {
            if r["granted"] == true {
                votes += 1;
            }
        }
    }
    if votes < needed {
        return Err(format!("{votes} vote(s) of the {needed} needed in epoch {epoch}"));
    }
    // 5. Take over, and tell everybody.
    repl.promote();
    repl.set_epoch(inst, epoch);
    let _ = inst.cfg_mut().set("replication", "role", "master");
    let event = format!("promoted to master in epoch {epoch} ({votes} of {nodes} nodes agreed that {master} was down)");
    inst.audit.log("system", &format!("failover: {event}"), true);
    repl.set_event(event);
    let announce = json!({ "op": "announce", "epoch": epoch, "master": me });
    for p in peers.iter().chain(std::iter::once(&master)) {
        let _ = call(inst, p, &announce, wait);
    }
    Ok(())
}

/// `failover status`.
pub fn status(inst: &Arc<Instance>) -> Vec<String> {
    let s = Settings::of(inst);
    let repl = &inst.repl;
    let mut out = Vec::new();
    let nodes = repl.known_nodes();
    out.push(if s.auto {
        format!(
            "Automatic failover: on (heartbeat {} ms, master down after {} ms of silence, {} of {} nodes must agree)",
            s.heartbeat.as_millis(),
            s.timeout.as_millis(),
            s.needed(nodes.len().max(1) + 1),
            nodes.len().max(1) + 1
        )
    } else {
        "Automatic failover: off (config set failover.auto true on every node)".to_string()
    });
    out.push(format!("Epoch: {}", repl.epoch()));
    if repl.is_replica() {
        let l = repl.link();
        let heard = repl
            .last_contact()
            .map(|t| format!("{:.1} s ago", t.elapsed().as_secs_f64()))
            .unwrap_or_else(|| "never".into());
        out.push(format!(
            "This node ({}): replica of {} (applied sequence {}, last heard {heard})",
            repl.advertised(),
            l.master,
            l.applied_seq
        ));
    } else {
        out.push(format!("This node ({}): master, {} replica(s) connected", repl.advertised(), repl.replicas().len()));
    }
    if !nodes.is_empty() {
        out.push(format!("Known nodes: {}", nodes.join(", ")));
    }
    if s.priority == 0 {
        out.push("This node never becomes master by itself (failover.priority 0).".into());
    }
    if let Some(e) = repl.event() {
        out.push(format!("Last event: {e}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(addr: &str, seq: u64, priority: u64) -> Candidate {
        Candidate { addr: addr.into(), seq, priority }
    }

    #[test]
    fn a_majority_is_needed_unless_configured() {
        assert_eq!(needed(2, 0), 2, "master + 1 replica: the replica alone is not a majority");
        assert_eq!(needed(3, 0), 2);
        assert_eq!(needed(4, 0), 3);
        assert_eq!(needed(5, 0), 3);
        assert_eq!(needed(2, 1), 1);
    }

    #[test]
    fn the_most_up_to_date_replica_wins() {
        let all = [c("10.0.0.3:2911", 90, 100), c("10.0.0.2:2911", 100, 100), c("10.0.0.4:2911", 100, 10)];
        assert_eq!(best(&all).unwrap().addr, "10.0.0.2:2911");
        // Equal progress and priority: the smallest address, for everyone.
        let tie = [c("10.0.0.9:2911", 5, 100), c("10.0.0.1:2911", 5, 100)];
        assert_eq!(best(&tie).unwrap().addr, "10.0.0.1:2911");
        // Priority 0 never wins, even ahead.
        let never = [c("a:1", 99, 0), c("b:1", 1, 1)];
        assert_eq!(best(&never).unwrap().addr, "b:1");
        assert!(best(&[c("a:1", 1, 0)]).is_none());
    }

    #[test]
    fn one_vote_per_epoch_for_an_up_to_date_candidate() {
        assert!(grants(3, 3, 4, true, 100, 100));
        assert!(!grants(3, 4, 4, true, 100, 100), "already voted in epoch 4");
        assert!(!grants(4, 3, 4, true, 100, 100), "not a newer epoch");
        assert!(!grants(3, 3, 4, false, 100, 100), "the voter still sees the master");
        assert!(!grants(3, 3, 4, true, 100, 99), "the candidate is behind the voter");
    }

    #[test]
    fn addresses_split_into_host_and_port() {
        assert_eq!(host_port("10.0.0.2:2911"), Some(("10.0.0.2".into(), 2911)));
        assert_eq!(host_port("[::1]:2911"), Some(("::1".into(), 2911)));
        assert_eq!(host_port("nohost"), None);
        assert_eq!(host_port("h:0"), None);
    }
}
