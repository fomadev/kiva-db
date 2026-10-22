// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Redis publish/subscribe (`PUBLISH`, `SUBSCRIBE`, `PSUBSCRIBE`...).
//!
//! Each Redis connection owns an outbox, a bounded queue drained by its own
//! writer thread: command replies and published messages leave through it in
//! order. A publisher only queues messages, so a slow subscriber never slows
//! it down; a subscriber whose outbox overflows is disconnected, as Redis does
//! when a client's output buffer passes its limit.
//!
//! Channels are separate for each database 0 workspace (the workspace a user
//! arrives in): applications logged in as different users never see each
//! other's messages, and, as in Redis, `SELECT` does not change the channels.

use crate::resp::Reply;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Mutex;

/// Messages and replies a connection may have waiting before it counts as
/// stuck and is disconnected.
pub const OUTBOX_FRAMES: usize = 65_536;

/// What a connection's writer thread sends.
pub enum Frame {
    /// A reply, already encoded.
    Bytes(Vec<u8>),
    /// A pub/sub message, encoded in the protocol the client speaks then.
    Push(Reply),
}

/// Where a connection's messages go.
pub struct Outbox {
    pub tx: SyncSender<Frame>,
    /// The connection itself, to close it when its outbox overflows.
    pub stream: TcpStream,
}

/// A channel or pattern name, within its workspace.
type Name = (String, Vec<u8>);

struct Listener {
    outbox: Outbox,
    channels: HashSet<Name>,
    patterns: HashSet<Name>,
}

impl Listener {
    fn count(&self) -> i64 {
        (self.channels.len() + self.patterns.len()) as i64
    }
}

/// A message published inside MULTI or a script, sent once it commits.
pub struct Held {
    pub scope: String,
    pub channel: Vec<u8>,
    pub message: Vec<u8>,
}

/// The listening connections of an instance, by connection id.
#[derive(Default)]
pub struct Hub {
    listeners: Mutex<HashMap<u64, Listener>>,
}

fn notice(kind: &str, name: Option<&[u8]>, count: i64) -> Reply {
    Reply::Push(vec![
        Reply::Bulk(Some(kind.as_bytes().to_vec())),
        Reply::Bulk(name.map(|n| n.to_vec())),
        Reply::Int(count),
    ])
}

/// Queues a frame without waiting. A full outbox closes the connection.
fn offer(outbox: &Outbox, frame: Frame) -> bool {
    match outbox.tx.try_send(frame) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            let _ = outbox.stream.shutdown(Shutdown::Both);
            false
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

impl Hub {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Listener>> {
        self.listeners.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Subscribes connection `id` to channels (or patterns) and confirms each
    /// one. Returns how many subscriptions the connection has now.
    pub fn subscribe(&self, id: u64, outbox: &Outbox, scope: &str, names: &[Vec<u8>], pattern: bool) -> i64 {
        let mut all = self.lock();
        let l = match all.entry(id) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let Ok(outbox) = clone_outbox(outbox) else { return 0 };
                e.insert(Listener { outbox, channels: HashSet::new(), patterns: HashSet::new() })
            }
        };
        let kind = if pattern { "psubscribe" } else { "subscribe" };
        let mut overflow = false;
        for n in names {
            let set = if pattern { &mut l.patterns } else { &mut l.channels };
            set.insert((scope.to_string(), n.clone()));
            overflow |= !offer(&l.outbox, Frame::Push(notice(kind, Some(n), l.count())));
        }
        let count = l.count();
        if overflow {
            all.remove(&id);
        }
        count
    }

    /// Unsubscribes from the given channels (or patterns), or from all of them
    /// when `names` is empty, and confirms each one.
    pub fn unsubscribe(&self, id: u64, outbox: &Outbox, names: &[Vec<u8>], pattern: bool) -> i64 {
        let mut all = self.lock();
        let kind = if pattern { "punsubscribe" } else { "unsubscribe" };
        let Some(l) = all.get_mut(&id) else {
            // Not subscribed to anything: Redis still answers once per name.
            if names.is_empty() {
                offer(outbox, Frame::Push(notice(kind, None, 0)));
            }
            for n in names {
                offer(outbox, Frame::Push(notice(kind, Some(n), 0)));
            }
            return 0;
        };
        let set = if pattern { &mut l.patterns } else { &mut l.channels };
        let targets: Vec<Vec<u8>> = if names.is_empty() {
            let mut all_names: Vec<Vec<u8>> = set.iter().map(|(_, n)| n.clone()).collect();
            all_names.sort();
            all_names.dedup();
            all_names
        } else {
            names.to_vec()
        };
        if targets.is_empty() {
            offer(&l.outbox, Frame::Push(notice(kind, None, l.count())));
        }
        for n in &targets {
            let set = if pattern { &mut l.patterns } else { &mut l.channels };
            set.retain(|(_, name)| name != n);
            offer(&l.outbox, Frame::Push(notice(kind, Some(n), l.count())));
        }
        let count = l.count();
        if count == 0 {
            all.remove(&id);
        }
        count
    }

    /// Forgets a connection that closed.
    pub fn leave(&self, id: u64) {
        self.lock().remove(&id);
    }

    /// Sends a message to the subscribers of `channel` in `scope`, and to the
    /// matching patterns. Returns how many deliveries were made. With `dry`,
    /// only counts them (messages published inside a transaction).
    pub fn publish(&self, scope: &str, channel: &[u8], message: &[u8], dry: bool) -> i64 {
        let mut all = self.lock();
        let mut sent = 0;
        let mut stuck = Vec::new();
        let key = (scope.to_string(), channel.to_vec());
        for (id, l) in all.iter() {
            let mut frames = Vec::new();
            if l.channels.contains(&key) {
                frames.push(Reply::Push(vec![
                    Reply::Bulk(Some(b"message".to_vec())),
                    Reply::Bulk(Some(channel.to_vec())),
                    Reply::Bulk(Some(message.to_vec())),
                ]));
            }
            for (s, p) in &l.patterns {
                if s == scope && glob(p, channel) {
                    frames.push(Reply::Push(vec![
                        Reply::Bulk(Some(b"pmessage".to_vec())),
                        Reply::Bulk(Some(p.clone())),
                        Reply::Bulk(Some(channel.to_vec())),
                        Reply::Bulk(Some(message.to_vec())),
                    ]));
                }
            }
            for f in frames {
                if dry || offer(&l.outbox, Frame::Push(f)) {
                    sent += 1;
                } else {
                    stuck.push(*id);
                    break;
                }
            }
        }
        for id in stuck {
            all.remove(&id);
        }
        sent
    }

    /// `PUBSUB CHANNELS [pattern]`: the channels with subscribers in `scope`.
    pub fn channels(&self, scope: &str, pattern: Option<&[u8]>) -> Vec<Vec<u8>> {
        let all = self.lock();
        let mut out: Vec<Vec<u8>> = all
            .values()
            .flat_map(|l| l.channels.iter())
            .filter(|(s, n)| s == scope && pattern.is_none_or(|p| glob(p, n)))
            .map(|(_, n)| n.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// `PUBSUB NUMSUB`: how many connections listen to `channel` in `scope`.
    pub fn numsub(&self, scope: &str, channel: &[u8]) -> i64 {
        let key = (scope.to_string(), channel.to_vec());
        self.lock().values().filter(|l| l.channels.contains(&key)).count() as i64
    }

    /// `PUBSUB NUMPAT`: how many patterns are listened to in `scope`.
    pub fn numpat(&self, scope: &str) -> i64 {
        self.lock().values().flat_map(|l| l.patterns.iter()).filter(|(s, _)| s == scope).count() as i64
    }
}

fn clone_outbox(o: &Outbox) -> std::io::Result<Outbox> {
    Ok(Outbox { tx: o.tx.clone(), stream: o.stream.try_clone()? })
}

/// Redis glob matching on bytes: `*`, `?`, `[abc]`, `[a-z]`, `[^a]` and `\` to
/// escape the next character.
pub fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si) = (0usize, 0usize);
    // Where to resume after the last `*`: (pattern position, text position).
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() {
            match p[pi] {
                b'*' => {
                    star = Some((pi + 1, si));
                    pi += 1;
                    continue;
                }
                b'?' => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                b'[' => {
                    if let Some((hit, next)) = class(p, pi, s[si]) {
                        if hit {
                            pi = next;
                            si += 1;
                            continue;
                        }
                    }
                }
                b'\\' if pi + 1 < p.len() => {
                    if p[pi + 1] == s[si] {
                        pi += 2;
                        si += 1;
                        continue;
                    }
                }
                c if c == s[si] => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, ss)) => {
                pi = sp;
                si = ss + 1;
                star = Some((sp, ss + 1));
            }
            None => return false,
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// Matches `c` against the class starting at `p[at] == '['`. Returns whether
/// it matched and where the pattern goes on, or `None` for an unclosed class.
fn class(p: &[u8], at: usize, c: u8) -> Option<(bool, usize)> {
    let mut i = at + 1;
    let negate = p.get(i) == Some(&b'^');
    if negate {
        i += 1;
    }
    let mut hit = false;
    while i < p.len() && p[i] != b']' {
        let mut lo = p[i];
        if lo == b'\\' && i + 1 < p.len() {
            i += 1;
            lo = p[i];
        }
        if i + 2 < p.len() && p[i + 1] == b'-' && p[i + 2] != b']' {
            let hi = p[i + 2];
            let (a, b) = if lo <= hi { (lo, hi) } else { (hi, lo) };
            hit |= (a..=b).contains(&c);
            i += 3;
        } else {
            hit |= lo == c;
            i += 1;
        }
    }
    if i >= p.len() {
        return None;
    }
    Some((hit != negate, i + 1))
}

#[cfg(test)]
mod tests {
    use super::glob;

    #[test]
    fn patterns_match_like_redis() {
        let m = |p: &str, s: &str| glob(p.as_bytes(), s.as_bytes());
        assert!(m("*", ""));
        assert!(m("news.*", "news.sport"));
        assert!(!m("news.*", "weather"));
        assert!(m("h?llo", "hello") && !m("h?llo", "hllo"));
        assert!(m("h[ae]llo", "hallo") && !m("h[ae]llo", "hillo"));
        assert!(m("h[^e]llo", "hallo") && !m("h[^e]llo", "hello"));
        assert!(m("h[a-c]llo", "hbllo") && !m("h[a-c]llo", "hdllo"));
        assert!(m("a\\*b", "a*b") && !m("a\\*b", "axb"));
        assert!(m("*.texts.*", "app.texts.fr"));
        assert!(m("a*b*c", "aXXbYYc") && !m("a*b*c", "aXXbYY"));
    }
}
