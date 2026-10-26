// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Redis streams: `XADD`, `XLEN`, `XRANGE`, `XREVRANGE`, `XDEL`, `XTRIM`,
//! `XSETID`, `XREAD`, consumer groups (`XGROUP`, `XREADGROUP`, `XACK`,
//! `XPENDING`, `XCLAIM`, `XAUTOCLAIM`) and `XINFO`.
//!
//! A stream is a KivaDB object, like a set: `{"redis": "stream", "last":
//! "<id>", "entries": [["<id>", ["field", "value", ...]], ...], "groups":
//! {...}}`. It is written to disk, replicated, encrypted and readable from
//! the shell like any document. Each command reads and rewrites the whole
//! object, so a stream holds at most `resp.max_collection_members` entries:
//! trim it (`XADD ... MAXLEN ~ 10000`, `XTRIM`) as Redis applications do.
//!
//! `XREAD BLOCK` and `XREADGROUP BLOCK` wait as the blocking list commands
//! do ([`super::blocking`]).

use super::*;
use std::collections::BTreeMap;
use std::time::Duration;

/// A stream entry id, `<milliseconds>-<sequence>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Id {
    ms: u64,
    seq: u64,
}

const MAX_ID: Id = Id { ms: u64::MAX, seq: u64::MAX };

impl Id {
    fn parse(s: &str) -> Option<Id> {
        match s.split_once('-') {
            Some((ms, seq)) => Some(Id { ms: ms.parse().ok()?, seq: seq.parse().ok()? }),
            None => Some(Id { ms: s.parse().ok()?, seq: 0 }),
        }
    }

    fn text(self) -> String {
        format!("{}-{}", self.ms, self.seq)
    }

    fn next(self) -> Option<Id> {
        if self.seq < u64::MAX {
            Some(Id { ms: self.ms, seq: self.seq + 1 })
        } else {
            self.ms.checked_add(1).map(|ms| Id { ms, seq: 0 })
        }
    }

    fn prev(self) -> Option<Id> {
        if self.seq > 0 {
            Some(Id { ms: self.ms, seq: self.seq - 1 })
        } else {
            self.ms.checked_sub(1).map(|ms| Id { ms, seq: u64::MAX })
        }
    }
}

fn invalid_id() -> Reply {
    Reply::err("Invalid stream ID specified as stream command argument")
}

/// A bound of `XRANGE`: `-`, `+`, an id, a time alone (`1700000000000`),
/// or one of those after `(` to exclude it.
fn bound(arg: &[u8], start: bool) -> std::result::Result<Option<Id>, Reply> {
    let s = text(arg)?;
    let (exclusive, s) = match s.strip_prefix('(') {
        Some(rest) => (true, rest),
        None => (false, s.as_str()),
    };
    let id = match s {
        "-" if !exclusive => Id::default(),
        "+" if !exclusive => MAX_ID,
        _ => {
            let full = s.contains('-');
            let mut id = Id::parse(s).ok_or_else(invalid_id)?;
            // A time alone covers all of its sequence numbers.
            if !full && !start {
                id.seq = u64::MAX;
            }
            id
        }
    };
    Ok(if exclusive {
        if start {
            id.next()
        } else {
            id.prev()
        }
    } else {
        Some(id)
    })
}

/// A pending entry of a consumer group.
#[derive(Clone, Debug)]
struct Pending {
    consumer: String,
    /// When it was last delivered, in milliseconds.
    delivered: i64,
    count: u64,
}

#[derive(Clone, Debug, Default)]
struct Consumer {
    /// Last time it read or was given entries.
    seen: i64,
    /// Last time it was given entries.
    active: i64,
}

#[derive(Clone, Debug, Default)]
struct Group {
    last: Id,
    /// Entries read by the group, when known.
    read: Option<u64>,
    pending: BTreeMap<Id, Pending>,
    consumers: BTreeMap<String, Consumer>,
}

#[derive(Clone, Debug, Default)]
struct Stream {
    last: Id,
    entries: BTreeMap<Id, Vec<String>>,
    /// Entries ever added.
    added: u64,
    max_deleted: Id,
    groups: BTreeMap<String, Group>,
}

fn id_of(j: &Json) -> Id {
    j.as_str().and_then(Id::parse).unwrap_or_default()
}

impl Stream {
    fn from_json(j: &Json) -> Stream {
        let mut s = Stream {
            last: id_of(&j["last"]),
            added: j["added"].as_u64().unwrap_or(0),
            max_deleted: id_of(&j["max_deleted"]),
            ..Default::default()
        };
        for e in j["entries"].as_array().into_iter().flatten() {
            let fields = e[1].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(String::from)).collect();
            s.entries.insert(id_of(&e[0]), fields);
        }
        for (name, g) in j["groups"].as_object().into_iter().flatten() {
            let mut group = Group { last: id_of(&g["last"]), read: g["read"].as_u64(), ..Default::default() };
            for p in g["pending"].as_array().into_iter().flatten() {
                let pending = Pending {
                    consumer: p[1].as_str().unwrap_or("").to_string(),
                    delivered: p[2].as_i64().unwrap_or(0),
                    count: p[3].as_u64().unwrap_or(1),
                };
                group.pending.insert(id_of(&p[0]), pending);
            }
            for (c, v) in g["consumers"].as_object().into_iter().flatten() {
                let consumer =
                    Consumer { seen: v["seen"].as_i64().unwrap_or(0), active: v["active"].as_i64().unwrap_or(0) };
                group.consumers.insert(c.clone(), consumer);
            }
            s.groups.insert(name.clone(), group);
        }
        s
    }

    fn to_json(&self) -> Json {
        let entries: Vec<Json> = self.entries.iter().map(|(id, f)| serde_json::json!([id.text(), f])).collect();
        let mut groups = serde_json::Map::new();
        for (name, g) in &self.groups {
            let pending: Vec<Json> = g
                .pending
                .iter()
                .map(|(id, p)| serde_json::json!([id.text(), p.consumer, p.delivered, p.count]))
                .collect();
            let consumers: serde_json::Map<String, Json> = g
                .consumers
                .iter()
                .map(|(c, v)| (c.clone(), serde_json::json!({ "seen": v.seen, "active": v.active })))
                .collect();
            groups.insert(
                name.clone(),
                serde_json::json!({ "last": g.last.text(), "read": g.read, "pending": pending, "consumers": consumers }),
            );
        }
        serde_json::json!({
            "redis": "stream",
            "last": self.last.text(),
            "added": self.added,
            "max_deleted": self.max_deleted.text(),
            "entries": entries,
            "groups": groups,
        })
    }

    fn first(&self) -> Option<Id> {
        self.entries.keys().next().copied()
    }

    /// Removes entries until `keep` remain, or those below `min_id`.
    fn trim(&mut self, max_len: Option<usize>, min_id: Option<Id>) -> usize {
        let mut removed = 0;
        while let Some(first) = self.first() {
            let too_many = max_len.is_some_and(|m| self.entries.len() > m);
            let too_old = min_id.is_some_and(|m| first < m);
            if !(too_many || too_old) {
                break;
            }
            self.entries.remove(&first);
            self.max_deleted = self.max_deleted.max(first);
            removed += 1;
        }
        removed
    }

    /// Entries a group has not read yet, for `XINFO GROUPS`.
    fn lag(&self, g: &Group) -> Option<u64> {
        if self.entries.is_empty() {
            return Some(0);
        }
        if g.last >= self.last {
            return Some(0);
        }
        // Exact only when no entry the group has not read was deleted.
        if self.max_deleted > g.last || self.first().is_some_and(|f| f > g.last && g.last != Id::default()) {
            return None;
        }
        g.read.map(|r| self.added.saturating_sub(r))
    }
}

/// `MAXLEN|MINID [=|~] <threshold> [LIMIT <n>]` at `args[i..]`; the
/// threshold and how many arguments it took.
type Trim = (Option<usize>, Option<Id>);

fn trim_args(args: &[Vec<u8>], mut i: usize) -> std::result::Result<Option<(Trim, usize)>, Reply> {
    let start = i;
    let Some(kind) = args.get(i).map(|a| String::from_utf8_lossy(a).to_ascii_uppercase()) else {
        return Ok(None);
    };
    if kind != "MAXLEN" && kind != "MINID" {
        return Ok(None);
    }
    i += 1;
    let mut approx = false;
    if let Some(op) = args.get(i).filter(|a| a.as_slice() == b"=" || a.as_slice() == b"~") {
        approx = op.as_slice() == b"~";
        i += 1;
    }
    let value = args.get(i).ok_or_else(|| Reply::err("syntax error"))?;
    i += 1;
    let trim = if kind == "MAXLEN" {
        let n = int(value)?;
        if n < 0 {
            return Err(Reply::err("The MAXLEN argument must be >= 0."));
        }
        (Some(n as usize), None)
    } else {
        (None, Some(Id::parse(&text(value)?).ok_or_else(invalid_id)?))
    };
    if args.get(i).is_some_and(|a| a.eq_ignore_ascii_case(b"LIMIT")) {
        if !approx {
            return Err(Reply::err("syntax error, LIMIT cannot be used without the special ~ option"));
        }
        int(args.get(i + 1).ok_or_else(|| Reply::err("syntax error"))?)?;
        i += 2;
    }
    Ok(Some((trim, i - start)))
}

fn entry_reply(id: Id, fields: &[String]) -> Reply {
    Reply::array(vec![Reply::bulk(id.text()), Reply::array(fields.iter().map(|f| Reply::bulk(f.clone())).collect())])
}

fn no_group(group: &str, key: &str) -> Reply {
    Reply::Error(format!("NOGROUP No such key '{key}' or consumer group '{group}'"))
}

fn upper(a: &[u8]) -> String {
    String::from_utf8_lossy(a).to_ascii_uppercase()
}

impl Conn {
    fn load_stream(&mut self, k: &str) -> std::result::Result<Option<(Stream, i64)>, Reply> {
        match self.get(k)? {
            None => Ok(None),
            Some((Value::Doc(j), m)) if sets::marked(&j) == Some("stream") => {
                Ok(Some((Stream::from_json(&j), m.expires_at)))
            }
            Some(_) => Err(Reply::wrong_type()),
        }
    }

    fn store_stream(&mut self, k: &str, s: &Stream, expires_at: i64) -> std::result::Result<(), Reply> {
        let limit = self.sess.inst.cfg().get_u64("resp", "max_collection_members") as usize;
        if limit > 0 && s.entries.len() > limit {
            return Err(Reply::err(format!(
                "the stream '{k}' would hold {} entries, more than resp.max_collection_members ({limit}). \
                 Every command reads and rewrites the whole stream: trim it (XADD ... MAXLEN ~ N, XTRIM), or raise the limit",
                s.entries.len()
            )));
        }
        self.put(k, Value::Doc(s.to_json()), expires_at)
    }

    /// The stream of a group command, and the group.
    fn group_of(&mut self, k: &str, g: &str) -> std::result::Result<(Stream, i64), Reply> {
        match self.load_stream(k)? {
            Some((s, exp)) if s.groups.contains_key(g) => Ok((s, exp)),
            _ => Err(no_group(g, k)),
        }
    }

    pub(super) fn run_streams(&mut self, name: &str, args: &[Vec<u8>]) -> Option<Res> {
        Some(match name {
            "XADD" => self.xadd(args),
            "XLEN" => (|| -> Res {
                arity(args, 2)?;
                let k = key(&args[1])?;
                Ok(Reply::Int(self.load_stream(&k)?.map(|(s, _)| s.entries.len() as i64).unwrap_or(0)))
            })(),
            "XRANGE" | "XREVRANGE" => self.xrange(name, args),
            "XDEL" => (|| -> Res {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let Some((mut s, exp)) = self.load_stream(&k)? else { return Ok(Reply::Int(0)) };
                let mut n = 0;
                for a in &args[2..] {
                    let id = Id::parse(&text(a)?).ok_or_else(invalid_id)?;
                    if s.entries.remove(&id).is_some() {
                        s.max_deleted = s.max_deleted.max(id);
                        n += 1;
                    }
                }
                if n > 0 {
                    self.store_stream(&k, &s, exp)?;
                }
                Ok(Reply::Int(n))
            })(),
            "XTRIM" => (|| -> Res {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let Some(((max_len, min_id), used)) = trim_args(args, 2)? else {
                    return Err(Reply::err("syntax error"));
                };
                if 2 + used != args.len() {
                    return Err(Reply::err("syntax error"));
                }
                let Some((mut s, exp)) = self.load_stream(&k)? else { return Ok(Reply::Int(0)) };
                let n = s.trim(max_len, min_id);
                if n > 0 {
                    self.store_stream(&k, &s, exp)?;
                }
                Ok(Reply::Int(n as i64))
            })(),
            "XSETID" => (|| -> Res {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let id = Id::parse(&text(&args[2])?).ok_or_else(invalid_id)?;
                let Some((mut s, exp)) = self.load_stream(&k)? else {
                    return Err(Reply::err("no such key"));
                };
                if s.entries.keys().next_back().is_some_and(|top| id < *top) {
                    return Err(Reply::err("The ID specified in XSETID is smaller than the target stream top item"));
                }
                s.last = id;
                self.store_stream(&k, &s, exp)?;
                Ok(Reply::ok())
            })(),
            "XGROUP" => self.xgroup(args),
            "XACK" => (|| -> Res {
                arity(args, 4)?;
                let (k, g) = (key(&args[1])?, text(&args[2])?);
                let Some((mut s, exp)) = self.load_stream(&k)? else { return Ok(Reply::Int(0)) };
                let Some(group) = s.groups.get_mut(&g) else { return Ok(Reply::Int(0)) };
                let mut n = 0;
                for a in &args[3..] {
                    let id = Id::parse(&text(a)?).ok_or_else(invalid_id)?;
                    if group.pending.remove(&id).is_some() {
                        n += 1;
                    }
                }
                if n > 0 {
                    self.store_stream(&k, &s, exp)?;
                }
                Ok(Reply::Int(n))
            })(),
            "XPENDING" => self.xpending(args),
            "XCLAIM" => self.xclaim(args),
            "XAUTOCLAIM" => self.xautoclaim(args),
            "XINFO" => self.xinfo(args),
            _ => return None,
        })
    }

    fn xadd(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 5)?;
        let k = key(&args[1])?;
        let mut i = 2;
        let mut nomkstream = false;
        let mut trim = None;
        loop {
            match args.get(i).map(|a| upper(a)).as_deref() {
                Some("NOMKSTREAM") => {
                    nomkstream = true;
                    i += 1;
                }
                Some("MAXLEN" | "MINID") => {
                    let (t, used) = trim_args(args, i)?.expect("MAXLEN or MINID");
                    trim = Some(t);
                    i += used;
                }
                _ => break,
            }
        }
        let id_arg = text(args.get(i).ok_or_else(|| Reply::err("syntax error"))?)?;
        let fields = &args[i + 1..];
        if fields.is_empty() || !fields.len().is_multiple_of(2) {
            return Err(Reply::err("wrong number of arguments for 'xadd' command"));
        }
        let (mut s, exp) = match self.load_stream(&k)? {
            Some(x) => x,
            None if nomkstream => return Ok(Reply::nil()),
            None => (Stream::default(), 0),
        };
        let top = s.last;
        let too_small = || Reply::err("The ID specified in XADD is equal or smaller than the target stream top item");
        // The next id: greater than every id the stream ever gave.
        let id = if id_arg == "*" {
            let ms = now_ms().max(0) as u64;
            if ms > top.ms {
                Id { ms, seq: 0 }
            } else {
                top.next().ok_or_else(|| {
                    Reply::err("The stream has exhausted the last possible ID, unable to add more items")
                })?
            }
        } else if let Some(ms) = id_arg.strip_suffix("-*") {
            let ms: u64 = ms.parse().map_err(|_| invalid_id())?;
            match ms.cmp(&top.ms) {
                std::cmp::Ordering::Greater => Id { ms, seq: 0 },
                std::cmp::Ordering::Equal => top.next().filter(|n| n.ms == ms).ok_or_else(too_small)?,
                std::cmp::Ordering::Less => return Err(too_small()),
            }
        } else {
            let id = Id::parse(&id_arg).ok_or_else(invalid_id)?;
            if id == Id::default() {
                return Err(Reply::err("The ID specified in XADD must be greater than 0-0"));
            }
            if id <= top {
                return Err(too_small());
            }
            id
        };
        let fields = fields.iter().map(|f| text(f)).collect::<std::result::Result<Vec<_>, _>>()?;
        s.entries.insert(id, fields);
        s.last = id;
        s.added += 1;
        if let Some((max_len, min_id)) = trim {
            s.trim(max_len, min_id);
        }
        self.store_stream(&k, &s, exp)?;
        Ok(Reply::bulk(id.text()))
    }

    fn xrange(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        arity(args, 4)?;
        let k = key(&args[1])?;
        let rev = name == "XREVRANGE";
        let (lo, hi) = if rev { (&args[3], &args[2]) } else { (&args[2], &args[3]) };
        let (lo, hi) = (bound(lo, true)?, bound(hi, false)?);
        let count = match args.get(4..) {
            Some([c, n]) if c.eq_ignore_ascii_case(b"COUNT") => Some(int(n)?.max(0) as usize),
            Some([]) => None,
            _ => return Err(Reply::err("syntax error")),
        };
        let Some((s, _)) = self.load_stream(&k)? else { return Ok(Reply::array(Vec::new())) };
        let (Some(lo), Some(hi)) = (lo, hi) else { return Ok(Reply::array(Vec::new())) };
        if lo > hi || count == Some(0) {
            return Ok(Reply::array(Vec::new()));
        }
        let range = s.entries.range(lo..=hi);
        let items: Vec<Reply> = if rev {
            range.rev().take(count.unwrap_or(usize::MAX)).map(|(id, f)| entry_reply(*id, f)).collect()
        } else {
            range.take(count.unwrap_or(usize::MAX)).map(|(id, f)| entry_reply(*id, f)).collect()
        };
        Ok(Reply::array(items))
    }

    fn xgroup(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 2)?;
        let sub = upper(&args[1]);
        if sub == "HELP" {
            return Ok(Reply::array(
                ["CREATE", "DESTROY", "CREATECONSUMER", "DELCONSUMER", "SETID"]
                    .iter()
                    .map(|s| Reply::bulk(*s))
                    .collect(),
            ));
        }
        arity(args, 4)?;
        let (k, g) = (key(&args[2])?, text(&args[3])?);
        match sub.as_str() {
            "CREATE" => {
                arity(args, 5)?;
                let mut mkstream = false;
                let mut read = None;
                let mut i = 5;
                while i < args.len() {
                    match upper(&args[i]).as_str() {
                        "MKSTREAM" => mkstream = true,
                        "ENTRIESREAD" => {
                            i += 1;
                            read = Some(int(args.get(i).ok_or_else(|| Reply::err("syntax error"))?)?.max(0) as u64);
                        }
                        _ => return Err(Reply::err("syntax error")),
                    }
                    i += 1;
                }
                let (mut s, exp) = match self.load_stream(&k)? {
                    Some(x) => x,
                    None if mkstream => (Stream::default(), 0),
                    None => {
                        return Err(Reply::err(
                            "The XGROUP subcommand requires the key to exist. Note that for CREATE you may want to use the MKSTREAM option to create an empty stream automatically.",
                        ))
                    }
                };
                if s.groups.contains_key(&g) {
                    return Err(Reply::Error("BUSYGROUP Consumer Group name already exists".into()));
                }
                let id_arg = text(&args[4])?;
                let last = if id_arg == "$" { s.last } else { Id::parse(&id_arg).ok_or_else(invalid_id)? };
                let read =
                    read.or_else(|| (last >= s.last).then_some(s.added)).or((last == Id::default()).then_some(0));
                s.groups.insert(g, Group { last, read, ..Default::default() });
                self.store_stream(&k, &s, exp)?;
                Ok(Reply::ok())
            }
            "SETID" => {
                arity(args, 5)?;
                let (mut s, exp) = self.group_of(&k, &g)?;
                let id_arg = text(&args[4])?;
                let last = if id_arg == "$" { s.last } else { Id::parse(&id_arg).ok_or_else(invalid_id)? };
                let read = match args.get(5..) {
                    Some([o, n]) if o.eq_ignore_ascii_case(b"ENTRIESREAD") => Some(int(n)?.max(0) as u64),
                    Some([]) => (last >= s.last).then_some(s.added),
                    _ => return Err(Reply::err("syntax error")),
                };
                let group = s.groups.get_mut(&g).expect("checked");
                group.last = last;
                group.read = read;
                self.store_stream(&k, &s, exp)?;
                Ok(Reply::ok())
            }
            "DESTROY" => {
                let Some((mut s, exp)) = self.load_stream(&k)? else { return Ok(Reply::Int(0)) };
                let gone = s.groups.remove(&g).is_some();
                if gone {
                    self.store_stream(&k, &s, exp)?;
                }
                Ok(Reply::Int(gone as i64))
            }
            "CREATECONSUMER" => {
                arity(args, 5)?;
                let c = text(&args[4])?;
                let (mut s, exp) = self.group_of(&k, &g)?;
                let group = s.groups.get_mut(&g).expect("checked");
                if group.consumers.contains_key(&c) {
                    return Ok(Reply::Int(0));
                }
                group.consumers.insert(c, Consumer { seen: now_ms(), active: 0 });
                self.store_stream(&k, &s, exp)?;
                Ok(Reply::Int(1))
            }
            "DELCONSUMER" => {
                arity(args, 5)?;
                let c = text(&args[4])?;
                let (mut s, exp) = self.group_of(&k, &g)?;
                let group = s.groups.get_mut(&g).expect("checked");
                // Its pending entries are dropped with it, as in Redis.
                let before = group.pending.len();
                group.pending.retain(|_, p| p.consumer != c);
                let dropped = before - group.pending.len();
                group.consumers.remove(&c);
                self.store_stream(&k, &s, exp)?;
                Ok(Reply::Int(dropped as i64))
            }
            _ => {
                Err(Reply::err(format!("unknown subcommand '{}'. Try XGROUP HELP.", String::from_utf8_lossy(&args[1]))))
            }
        }
    }

    /// `BLOCK <ms>` of `XREAD` / `XREADGROUP`: `None` without it,
    /// `Some(None)` to wait forever (`BLOCK 0`).
    pub(super) fn stream_block(&self, args: &[Vec<u8>]) -> std::result::Result<Option<Option<Duration>>, Reply> {
        let mut i = 1;
        while i < args.len() {
            match upper(&args[i]).as_str() {
                "STREAMS" => return Ok(None),
                "BLOCK" => {
                    let ms = int(args.get(i + 1).ok_or_else(|| Reply::err("syntax error"))?)?;
                    if ms < 0 {
                        return Err(Reply::err("timeout is negative"));
                    }
                    return Ok(Some((ms > 0).then(|| Duration::from_millis(ms as u64))));
                }
                _ => i += 1,
            }
        }
        Ok(None)
    }

    /// `XREAD` / `XREADGROUP` with `$` replaced by each stream's last id
    /// now, so that a blocked read waits for entries added after it began.
    pub(super) fn resolve_last_ids(&mut self, args: &[Vec<u8>]) -> std::result::Result<Vec<Vec<u8>>, Reply> {
        let Some(at) = args.iter().position(|a| a.eq_ignore_ascii_case(b"STREAMS")) else {
            return Ok(args.to_vec());
        };
        let rest = &args[at + 1..];
        let n = rest.len() / 2;
        let mut out = args.to_vec();
        for j in 0..n {
            if rest[n + j].as_slice() == b"$" {
                let k = key(&rest[j])?;
                let last = self.load_stream(&k)?.map(|(s, _)| s.last).unwrap_or_default();
                out[at + 1 + n + j] = last.text().into_bytes();
            }
        }
        Ok(out)
    }

    /// One attempt of `XREAD` / `XREADGROUP`: `None` when nothing is there.
    pub(super) fn stream_read(&mut self, name: &str, args: &[Vec<u8>]) -> std::result::Result<Option<Reply>, Reply> {
        let group = name == "XREADGROUP";
        let mut i = 1;
        let (mut count, mut noack, mut who) = (None, false, None);
        if group {
            if !args.get(1).is_some_and(|a| a.eq_ignore_ascii_case(b"GROUP")) || args.len() < 4 {
                return Err(Reply::err("syntax error"));
            }
            who = Some((text(&args[2])?, text(&args[3])?));
            i = 4;
        }
        let at = loop {
            match args.get(i).map(|a| upper(a)).as_deref() {
                Some("COUNT") => {
                    count = Some(int(args.get(i + 1).ok_or_else(|| Reply::err("syntax error"))?)?.max(0) as usize);
                    i += 2;
                }
                Some("BLOCK") => i += 2,
                Some("NOACK") if group => {
                    noack = true;
                    i += 1;
                }
                Some("STREAMS") => break i + 1,
                _ => return Err(Reply::err("syntax error")),
            }
        };
        let rest = &args[at..];
        if rest.is_empty() || !rest.len().is_multiple_of(2) {
            return Err(Reply::err(
                "Unbalanced 'xread' list of streams: for each stream key an ID or '$' must be specified.",
            ));
        }
        let n = rest.len() / 2;
        let limit = count.filter(|c| *c > 0).unwrap_or(usize::MAX);
        let mut found: Vec<(Vec<u8>, Reply)> = Vec::new();
        for j in 0..n {
            let k = key(&rest[j])?;
            let id_arg = text(&rest[n + j])?;
            let Some((g, c)) = &who else {
                let Some((s, _)) = self.load_stream(&k)? else { continue };
                let after = match id_arg.as_str() {
                    "$" => s.last,
                    "+" => s.entries.keys().next_back().and_then(|t| t.prev()).unwrap_or_default(),
                    _ => Id::parse(&id_arg).ok_or_else(invalid_id)?,
                };
                let Some(from) = after.next() else { continue };
                let items: Vec<Reply> =
                    s.entries.range(from..).take(limit).map(|(id, f)| entry_reply(*id, f)).collect();
                if !items.is_empty() {
                    found.push((rest[j].clone(), Reply::array(items)));
                }
                continue;
            };
            let (mut s, exp) = self.group_of(&k, g)?;
            let now = now_ms();
            let mut items = Vec::new();
            // Written back only when something changed: a blocked reader that
            // wrote on every try would wake the others, which would write...
            let mut changed;
            {
                let s_entries = &s.entries;
                let group = s.groups.get_mut(g).expect("checked");
                changed = !group.consumers.contains_key(c);
                let consumer = group.consumers.entry(c.clone()).or_default();
                consumer.seen = now;
                if id_arg == ">" {
                    let from = group.last.next();
                    let new: Vec<(Id, Vec<String>)> = match from {
                        Some(f) => s_entries.range(f..).take(limit).map(|(id, f)| (*id, f.clone())).collect(),
                        None => Vec::new(),
                    };
                    if let Some((last, _)) = new.last() {
                        group.last = *last;
                        group.read = group.read.map(|r| r + new.len() as u64);
                        consumer.active = now;
                    }
                    changed |= !new.is_empty();
                    for (id, f) in new {
                        if !noack {
                            group.pending.insert(id, Pending { consumer: c.clone(), delivered: now, count: 1 });
                        }
                        items.push(entry_reply(id, &f));
                    }
                } else {
                    // The consumer's own pending entries after the id, again.
                    let after = Id::parse(&id_arg).ok_or_else(invalid_id)?;
                    let from = after.next().unwrap_or(MAX_ID);
                    let mine: Vec<Id> = group
                        .pending
                        .range(from..)
                        .filter(|(_, p)| &p.consumer == c)
                        .map(|(id, _)| *id)
                        .take(limit)
                        .collect();
                    changed |= !mine.is_empty();
                    for id in mine {
                        let p = group.pending.get_mut(&id).expect("listed");
                        p.delivered = now;
                        p.count += 1;
                        // A deleted entry comes back as its id with no fields.
                        items.push(match s_entries.get(&id) {
                            Some(f) => entry_reply(id, f),
                            None => Reply::array(vec![Reply::bulk(id.text()), Reply::Array(None)]),
                        });
                    }
                }
            }
            // History reads answer even when empty; new entries only when there are some.
            let history = id_arg != ">";
            if changed {
                self.store_stream(&k, &s, exp)?;
            }
            if history || !items.is_empty() {
                found.push((rest[j].clone(), Reply::array(items)));
            }
        }
        if found.is_empty() {
            return Ok(None);
        }
        Ok(Some(if self.resp3 {
            Reply::Map(found.into_iter().map(|(k, v)| (Reply::bulk(k), v)).collect())
        } else {
            Reply::array(found.into_iter().map(|(k, v)| Reply::array(vec![Reply::bulk(k), v])).collect())
        }))
    }

    fn xpending(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 3)?;
        let (k, g) = (key(&args[1])?, text(&args[2])?);
        let (s, _) = self.group_of(&k, &g)?;
        let group = &s.groups[&g];
        if args.len() == 3 {
            // Summary: count, smallest and largest id, and per consumer.
            let mut per: BTreeMap<&str, u64> = BTreeMap::new();
            for p in group.pending.values() {
                *per.entry(p.consumer.as_str()).or_default() += 1;
            }
            let ends = |id: Option<&Id>| id.map(|i| Reply::bulk(i.text())).unwrap_or(Reply::nil());
            let consumers = if per.is_empty() {
                Reply::Array(None)
            } else {
                Reply::array(
                    per.iter().map(|(c, n)| Reply::array(vec![Reply::bulk(*c), Reply::bulk(n.to_string())])).collect(),
                )
            };
            return Ok(Reply::array(vec![
                Reply::Int(group.pending.len() as i64),
                ends(group.pending.keys().next()),
                ends(group.pending.keys().next_back()),
                consumers,
            ]));
        }
        let mut i = 3;
        let mut idle = 0i64;
        if args[i].eq_ignore_ascii_case(b"IDLE") {
            idle = int(args.get(i + 1).ok_or_else(|| Reply::err("syntax error"))?)?;
            i += 2;
        }
        let (Some(lo), Some(hi), Some(n)) = (args.get(i), args.get(i + 1), args.get(i + 2)) else {
            return Err(Reply::err("syntax error"));
        };
        let (lo, hi) = (bound(lo, true)?, bound(hi, false)?);
        let n = int(n)?.max(0) as usize;
        let consumer = args.get(i + 3).map(|c| text(c)).transpose()?;
        let now = now_ms();
        let (Some(lo), Some(hi)) = (lo, hi) else { return Ok(Reply::array(Vec::new())) };
        if lo > hi {
            return Ok(Reply::array(Vec::new()));
        }
        let rows = group
            .pending
            .range(lo..=hi)
            .filter(|(_, p)| consumer.as_ref().is_none_or(|c| &p.consumer == c) && now - p.delivered >= idle)
            .take(n)
            .map(|(id, p)| {
                Reply::array(vec![
                    Reply::bulk(id.text()),
                    Reply::bulk(p.consumer.clone()),
                    Reply::Int(now - p.delivered),
                    Reply::Int(p.count as i64),
                ])
            })
            .collect();
        Ok(Reply::array(rows))
    }

    fn xclaim(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 6)?;
        let (k, g, c) = (key(&args[1])?, text(&args[2])?, text(&args[3])?);
        let min_idle = int(&args[4])?.max(0);
        let mut ids = Vec::new();
        let mut i = 5;
        while let Some(id) = args.get(i).and_then(|a| std::str::from_utf8(a).ok()).and_then(Id::parse) {
            ids.push(id);
            i += 1;
        }
        let now = now_ms();
        let (mut delivered, mut retry, mut force, mut justid) = (None, None, false, false);
        while i < args.len() {
            let opt = upper(&args[i]);
            let val = || args.get(i + 1).ok_or_else(|| Reply::err("syntax error"));
            match opt.as_str() {
                "IDLE" => {
                    delivered = Some(now - int(val()?)?);
                    i += 1;
                }
                "TIME" => {
                    delivered = Some(int(val()?)?);
                    i += 1;
                }
                "RETRYCOUNT" => {
                    retry = Some(int(val()?)?.max(0) as u64);
                    i += 1;
                }
                "LASTID" => {
                    Id::parse(&text(val()?)?).ok_or_else(invalid_id)?;
                    i += 1;
                }
                "FORCE" => force = true,
                "JUSTID" => justid = true,
                _ => return Err(Reply::err("syntax error")),
            }
            i += 1;
        }
        let (mut s, exp) = self.group_of(&k, &g)?;
        let entries = &s.entries;
        let group = s.groups.get_mut(&g).expect("checked");
        let mut out = Vec::new();
        for id in ids {
            if let std::collections::btree_map::Entry::Vacant(slot) = group.pending.entry(id) {
                if !force || !entries.contains_key(&id) {
                    continue;
                }
                slot.insert(Pending { consumer: c.clone(), delivered: now, count: 0 });
            }
            if !entries.contains_key(&id) {
                // Deleted meanwhile: no longer pending.
                group.pending.remove(&id);
                continue;
            }
            let p = group.pending.get_mut(&id).expect("pending");
            if now - p.delivered < min_idle {
                continue;
            }
            p.consumer = c.clone();
            p.delivered = delivered.unwrap_or(now);
            if let Some(r) = retry {
                p.count = r;
            } else if !justid {
                p.count += 1;
            }
            out.push(if justid { Reply::bulk(id.text()) } else { entry_reply(id, &entries[&id]) });
        }
        let consumer = group.consumers.entry(c).or_default();
        consumer.seen = now;
        if !out.is_empty() {
            consumer.active = now;
        }
        self.store_stream(&k, &s, exp)?;
        Ok(Reply::array(out))
    }

    fn xautoclaim(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 6)?;
        let (k, g, c) = (key(&args[1])?, text(&args[2])?, text(&args[3])?);
        let min_idle = int(&args[4])?.max(0);
        let start = bound(&args[5], true)?.unwrap_or(MAX_ID);
        let (mut count, mut justid) = (100usize, false);
        let mut i = 6;
        while i < args.len() {
            match upper(&args[i]).as_str() {
                "COUNT" => {
                    count = int(args.get(i + 1).ok_or_else(|| Reply::err("syntax error"))?)?.max(1) as usize;
                    i += 1;
                }
                "JUSTID" => justid = true,
                _ => return Err(Reply::err("syntax error")),
            }
            i += 1;
        }
        let now = now_ms();
        let (mut s, exp) = self.group_of(&k, &g)?;
        let entries = &s.entries;
        let group = s.groups.get_mut(&g).expect("checked");
        let scanned: Vec<Id> = group.pending.range(start..).map(|(id, _)| *id).take(count * 10).collect();
        let (mut claimed, mut deleted, mut next) = (Vec::new(), Vec::new(), Id::default());
        let mut taken = 0;
        for (n, id) in scanned.iter().enumerate() {
            if taken >= count {
                next = *id;
                break;
            }
            if n + 1 == scanned.len() && scanned.len() == count * 10 {
                next = id.next().unwrap_or_default();
            }
            if !entries.contains_key(id) {
                group.pending.remove(id);
                deleted.push(Reply::bulk(id.text()));
                continue;
            }
            let p = group.pending.get_mut(id).expect("listed");
            if now - p.delivered < min_idle {
                continue;
            }
            p.consumer = c.clone();
            p.delivered = now;
            if !justid {
                p.count += 1;
            }
            taken += 1;
            claimed.push(if justid { Reply::bulk(id.text()) } else { entry_reply(*id, &entries[id]) });
        }
        let consumer = group.consumers.entry(c).or_default();
        consumer.seen = now;
        if taken > 0 {
            consumer.active = now;
        }
        self.store_stream(&k, &s, exp)?;
        Ok(Reply::array(vec![Reply::bulk(next.text()), Reply::array(claimed), Reply::array(deleted)]))
    }

    fn xinfo(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 2)?;
        let sub = upper(&args[1]);
        if sub == "HELP" {
            return Ok(Reply::array(["CONSUMERS", "GROUPS", "STREAM"].iter().map(|s| Reply::bulk(*s)).collect()));
        }
        arity(args, 3)?;
        let k = key(&args[2])?;
        let Some((s, _)) = self.load_stream(&k)? else { return Err(Reply::err("no such key")) };
        let now = now_ms();
        let pair = |name: &str, v: Reply| (Reply::bulk(name.to_string()), v);
        let entry = |e: Option<(&Id, &Vec<String>)>| e.map(|(id, f)| entry_reply(*id, f)).unwrap_or(Reply::nil());
        match sub.as_str() {
            "STREAM" => {
                let mut info = vec![
                    pair("length", Reply::Int(s.entries.len() as i64)),
                    pair("last-generated-id", Reply::bulk(s.last.text())),
                    pair("max-deleted-entry-id", Reply::bulk(s.max_deleted.text())),
                    pair("entries-added", Reply::Int(s.added as i64)),
                    pair("recorded-first-entry-id", Reply::bulk(s.first().unwrap_or_default().text())),
                    pair("groups", Reply::Int(s.groups.len() as i64)),
                ];
                if args.get(3).is_some_and(|a| a.eq_ignore_ascii_case(b"FULL")) {
                    let entries = s.entries.iter().map(|(id, f)| entry_reply(*id, f)).collect();
                    info.push(pair("entries", Reply::array(entries)));
                } else {
                    info.push(pair("first-entry", entry(s.entries.iter().next())));
                    info.push(pair("last-entry", entry(s.entries.iter().next_back())));
                }
                Ok(Reply::Map(info))
            }
            "GROUPS" => Ok(Reply::array(
                s.groups
                    .iter()
                    .map(|(name, g)| {
                        Reply::Map(vec![
                            pair("name", Reply::bulk(name.clone())),
                            pair("consumers", Reply::Int(g.consumers.len() as i64)),
                            pair("pending", Reply::Int(g.pending.len() as i64)),
                            pair("last-delivered-id", Reply::bulk(g.last.text())),
                            pair("entries-read", g.read.map(|r| Reply::Int(r as i64)).unwrap_or(Reply::nil())),
                            pair("lag", s.lag(g).map(|l| Reply::Int(l as i64)).unwrap_or(Reply::nil())),
                        ])
                    })
                    .collect(),
            )),
            "CONSUMERS" => {
                arity(args, 4)?;
                let g = text(&args[3])?;
                let group = s.groups.get(&g).ok_or_else(|| no_group(&g, &k))?;
                Ok(Reply::array(
                    group
                        .consumers
                        .iter()
                        .map(|(c, v)| {
                            let pending = group.pending.values().filter(|p| &p.consumer == c).count();
                            Reply::Map(vec![
                                pair("name", Reply::bulk(c.clone())),
                                pair("pending", Reply::Int(pending as i64)),
                                pair("idle", Reply::Int(now - v.seen)),
                                pair("inactive", Reply::Int(if v.active > 0 { now - v.active } else { -1 })),
                            ])
                        })
                        .collect(),
                ))
            }
            _ => {
                Err(Reply::err(format!("unknown subcommand '{}'. Try XINFO HELP.", String::from_utf8_lossy(&args[1]))))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_bounds() {
        assert_eq!(Id::parse("5-3"), Some(Id { ms: 5, seq: 3 }));
        assert_eq!(Id::parse("5"), Some(Id { ms: 5, seq: 0 }));
        assert_eq!(Id::parse("x"), None);
        assert_eq!(Id { ms: 5, seq: u64::MAX }.next(), Some(Id { ms: 6, seq: 0 }));
        assert_eq!(Id { ms: 5, seq: 0 }.prev(), Some(Id { ms: 4, seq: u64::MAX }));
        assert_eq!(bound(b"-", true).unwrap(), Some(Id::default()));
        assert_eq!(bound(b"7", false).unwrap(), Some(Id { ms: 7, seq: u64::MAX }));
        assert_eq!(bound(b"(7-1", true).unwrap(), Some(Id { ms: 7, seq: 2 }));
        assert!(bound(b"(-", true).is_err());
    }

    #[test]
    fn a_stream_survives_its_json_and_trims() {
        let mut s = Stream::default();
        for i in 1..=5 {
            s.entries.insert(Id { ms: i, seq: 0 }, vec!["f".into(), i.to_string()]);
            s.last = Id { ms: i, seq: 0 };
            s.added += 1;
        }
        s.groups.insert("g".into(), Group { last: Id { ms: 2, seq: 0 }, read: Some(2), ..Default::default() });
        let back = Stream::from_json(&s.to_json());
        assert_eq!((back.entries.len(), back.last, back.added), (5, s.last, 5));
        assert_eq!(back.lag(&back.groups["g"]), Some(3));
        let mut t = back.clone();
        assert_eq!(t.trim(Some(2), None), 3);
        assert_eq!(t.first(), Some(Id { ms: 4, seq: 0 }));
        assert_eq!(t.lag(&t.groups["g"]), None, "entries it had not read were deleted");
        assert_eq!(t.trim(None, Some(Id { ms: 5, seq: 0 })), 1);
    }
}
