// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Blocking commands: `BLPOP`, `BRPOP`, `BRPOPLPUSH`, `BLMOVE`, `BLMPOP`,
//! `BZPOPMIN`, `BZPOPMAX`, `BZMPOP`, and `WAIT`.
//!
//! Each one tries its non-blocking counterpart (`LPOP`, `ZPOPMIN`, `LMOVE`...)
//! and answers at once when there is something to take. Otherwise the
//! connection waits, without holding the turn that commands take one at a
//! time, until a write lands in any workspace (from Redis clients, the
//! shell, the REST API, a script or replication), then tries again, until
//! its timeout. Waiting clients are woken together: the first to get the
//! turn takes the value, not necessarily the one that waited longest.
//!
//! Inside `MULTI` and in scripts, a blocking command does not wait: it
//! answers like its counterpart, or with a null reply, as in Redis.

use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::Condvar;
use std::time::{Duration, Instant};

/// Commands that may wait for data.
pub(super) fn is_blocking(name: &str) -> bool {
    matches!(
        name,
        "BLPOP"
            | "BRPOP"
            | "BRPOPLPUSH"
            | "BLMOVE"
            | "BLMPOP"
            | "BZPOPMIN"
            | "BZPOPMAX"
            | "BZMPOP"
            | "XREAD"
            | "XREADGROUP"
    )
}

/// Wakes the connections waiting in a blocking command.
#[derive(Default)]
pub struct Waiters {
    /// Bumped by every write, so that a connection can tell one happened.
    writes: Mutex<u64>,
    woken: Condvar,
    waiting: AtomicUsize,
}

impl Waiters {
    /// Called for every committed write: wakes the waiting connections, if any.
    pub fn wake(&self) {
        if self.waiting.load(Ordering::SeqCst) == 0 {
            return;
        }
        *self.writes.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.woken.notify_all();
    }

    fn seen(&self) -> u64 {
        *self.writes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Waits for a write after `seen`, at most `max`.
    fn wait(&self, seen: u64, max: Duration) {
        let guard = self.writes.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.woken.wait_timeout_while(guard, max, |w| *w == seen);
    }
}

/// Counts a connection as waiting while it lives.
struct Waiting<'a>(&'a Waiters);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A timeout in seconds, possibly fractional; 0 waits forever.
pub(super) fn timeout_secs(arg: &[u8]) -> std::result::Result<Option<Duration>, Reply> {
    let t = std::str::from_utf8(arg)
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|t| t.is_finite())
        .ok_or_else(|| Reply::err("timeout is not a float or out of range"))?;
    if t < 0.0 {
        return Err(Reply::err("timeout is negative"));
    }
    Ok((t > 0.0).then(|| Duration::from_secs_f64(t.min(1e9))))
}

fn args_of(parts: &[&[u8]]) -> Vec<Vec<u8>> {
    parts.iter().map(|p| p.to_vec()).collect()
}

impl Conn {
    /// Runs a blocking command outside `MULTI`: tries, then waits for
    /// writes until it succeeds or its timeout passes.
    pub(super) fn block(&mut self, name: &str, args: &[Vec<u8>]) -> Reply {
        let timeout = match self.blocking_timeout(name, args) {
            Ok(t) => t,
            Err(e) => return e,
        };
        let deadline = timeout.map(|t| Instant::now() + t);
        let inst = self.sess.inst.clone();
        let waiters = &inst.resp.waiters;
        waiters.waiting.fetch_add(1, Ordering::SeqCst);
        let _waiting = Waiting(waiters);
        loop {
            // Read before trying: a write landing after the try then counts.
            let seen = waiters.seen();
            {
                let _turn = inst.resp.turn.lock().unwrap_or_else(|e| e.into_inner());
                match self.try_blocking(name, args) {
                    Ok(Some(r)) => return r,
                    Ok(None) => {}
                    Err(e) => return e,
                }
            }
            let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            if left.is_some_and(|l| l.is_zero()) {
                return self.timed_out(name);
            }
            // A client that left stops waiting: what arrives is not taken for it.
            if self.client_gone() {
                return Reply::nil();
            }
            waiters.wait(seen, left.unwrap_or(Duration::MAX).min(Duration::from_millis(200)));
        }
    }

    /// What a blocking command answers when nothing came.
    pub(super) fn timed_out(&self, name: &str) -> Reply {
        match name {
            "BRPOPLPUSH" | "BLMOVE" => Reply::nil(),
            _ => Reply::Array(None),
        }
    }

    /// Whether the client closed its connection.
    fn client_gone(&self) -> bool {
        let Some(out) = &self.outbox else { return false };
        let s = &out.stream;
        if s.set_read_timeout(Some(Duration::from_millis(1))).is_err() {
            return false;
        }
        let gone = matches!(s.peek(&mut [0u8; 1]), Ok(0));
        let _ = s.set_read_timeout(None);
        gone
    }

    /// The timeout of a blocking command, after checking its arguments.
    fn blocking_timeout(&self, name: &str, args: &[Vec<u8>]) -> std::result::Result<Option<Duration>, Reply> {
        match name {
            "BLMPOP" | "BZMPOP" => {
                arity(args, 5)?;
                timeout_secs(&args[1])
            }
            "XREAD" | "XREADGROUP" => Ok(self.stream_block(args)?.flatten()),
            _ => {
                arity(args, 3)?;
                timeout_secs(&args[args.len() - 1])
            }
        }
    }

    /// One attempt: `Some(reply)` when it took something (or failed),
    /// `None` when there was nothing to take.
    pub(super) fn try_blocking(&mut self, name: &str, args: &[Vec<u8>]) -> std::result::Result<Option<Reply>, Reply> {
        match name {
            "BLPOP" | "BRPOP" | "BZPOPMIN" | "BZPOPMAX" => {
                arity(args, 3)?;
                timeout_secs(&args[args.len() - 1])?;
                let pop: &[u8] = match name {
                    "BLPOP" => b"LPOP",
                    "BRPOP" => b"RPOP",
                    "BZPOPMIN" => b"ZPOPMIN",
                    _ => b"ZPOPMAX",
                };
                for k in &args[1..args.len() - 1] {
                    let r = self.run(&String::from_utf8_lossy(pop), &args_of(&[pop, k]))?;
                    match r {
                        Reply::Bulk(Some(v)) => {
                            return Ok(Some(Reply::array(vec![Reply::bulk(k.clone()), Reply::Bulk(Some(v))])))
                        }
                        // ZPOPMIN without a count: one flat pair.
                        Reply::Array(Some(pair)) if pair.len() == 2 => {
                            let mut out = vec![Reply::bulk(k.clone())];
                            out.extend(pair);
                            return Ok(Some(Reply::array(out)));
                        }
                        _ => {}
                    }
                }
                Ok(None)
            }
            "BRPOPLPUSH" | "BLMOVE" => {
                let (from, to, timeout) = if name == "BRPOPLPUSH" {
                    arity(args, 4)?;
                    (b"RIGHT".to_vec(), b"LEFT".to_vec(), &args[3])
                } else {
                    arity(args, 6)?;
                    (args[3].clone(), args[4].clone(), &args[5])
                };
                timeout_secs(timeout)?;
                let r = self.run("LMOVE", &args_of(&[b"LMOVE", &args[1], &args[2], &from, &to]))?;
                Ok(matches!(r, Reply::Bulk(Some(_))).then_some(r))
            }
            "BLMPOP" => {
                arity(args, 5)?;
                timeout_secs(&args[1])?;
                let mut lmpop = vec![b"LMPOP".to_vec()];
                lmpop.extend_from_slice(&args[2..]);
                let r = self.run("LMPOP", &lmpop)?;
                Ok(matches!(r, Reply::Array(Some(_))).then_some(r))
            }
            "BZMPOP" => {
                arity(args, 5)?;
                timeout_secs(&args[1])?;
                let n = usize::try_from(int(&args[2])?)
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| Reply::err("numkeys should be greater than 0"))?;
                let keys = args.get(3..3 + n).ok_or_else(|| Reply::err("syntax error"))?;
                let rest = &args[3 + n..];
                let pop: &[u8] = match rest.first().map(|a| String::from_utf8_lossy(a).to_ascii_uppercase()) {
                    Some(d) if d == "MIN" => b"ZPOPMIN",
                    Some(d) if d == "MAX" => b"ZPOPMAX",
                    _ => return Err(Reply::err("syntax error")),
                };
                let count = match rest.get(1..) {
                    Some([c, n]) if c.eq_ignore_ascii_case(b"COUNT") => {
                        let n = int(n)?;
                        if n <= 0 {
                            return Err(Reply::err("count should be greater than 0"));
                        }
                        n.to_string().into_bytes()
                    }
                    Some([]) => b"1".to_vec(),
                    _ => return Err(Reply::err("syntax error")),
                };
                for k in keys {
                    let r = self.run(&String::from_utf8_lossy(pop), &args_of(&[pop, k, &count]))?;
                    let Reply::Array(Some(items)) = r else { continue };
                    if items.is_empty() {
                        continue;
                    }
                    // Member and score pairs, flat in RESP2: grouped as ZMPOP does.
                    let pairs = if items.iter().all(|i| matches!(i, Reply::Array(_))) {
                        items
                    } else {
                        items.chunks(2).map(|p| Reply::array(p.to_vec())).collect()
                    };
                    return Ok(Some(Reply::array(vec![Reply::bulk(k.clone()), Reply::array(pairs)])));
                }
                Ok(None)
            }
            "XREAD" | "XREADGROUP" => self.stream_read(name, args),
            _ => Err(commands::unknown(args)),
        }
    }

    /// `WAIT numreplicas timeout`: how many replicas have every write made
    /// so far, waiting up to `timeout` milliseconds (0: until enough do).
    pub(super) fn wait_replicas(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 3)?;
        let n = usize::try_from(int(&args[1])?).map_err(|_| Reply::err("value is out of range"))?;
        let ms = u64::try_from(int(&args[2])?).map_err(|_| Reply::err("timeout is negative"))?;
        let timeout = if ms == 0 { Duration::from_secs(365 * 24 * 3600) } else { Duration::from_millis(ms) };
        Ok(Reply::Int(self.sess.inst.repl.wait(n, timeout) as i64))
    }
}
