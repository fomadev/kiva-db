// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Redis sets and sorted sets, stored as KivaDB objects so that they are
//! saved, replicated and encrypted like every other value, and readable from
//! the shell:
//!
//! ```text
//! set:  {"redis": "set",  "members": ["a", "b"]}
//! zset: {"redis": "zset", "members": {"alice": 12.5, "bob": 3}}
//! ```
//!
//! Members keep their insertion order on disk; a sorted set is ordered by
//! score, then member, when it is read. Each command reads and rewrites the
//! whole value: fine for sets of a few thousand members (rate-limit windows,
//! tags, leaderboards of a game or a shop), not for millions.

use super::*;

/// The field that marks a set or a sorted set.
const MARK: &str = "redis";

/// Members of a set (value `null`) or of a sorted set (value: the score), in
/// insertion order with constant-time lookups.
type Members = Map<String, Json>;

/// `"set"` or `"zset"` when `j` is one of these, `None` for a plain object.
pub(super) fn marked(j: &Json) -> Option<&'static str> {
    match j.get(MARK)?.as_str()? {
        "set" if j["members"].is_array() => Some("set"),
        "zset" if j["members"].is_object() => Some("zset"),
        _ => None,
    }
}

fn score_of(v: &Json) -> f64 {
    match v {
        Json::Number(n) => n.as_f64().unwrap_or(0.0),
        Json::String(s) if s == "inf" => f64::INFINITY,
        Json::String(s) if s == "-inf" => f64::NEG_INFINITY,
        _ => 0.0,
    }
}

/// A score as JSON; infinities, which JSON has no number for, as text.
fn score_json(f: f64) -> Json {
    // Whole scores read better as integers: 120, not 120.0.
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        return Json::from(f as i64);
    }
    match serde_json::Number::from_f64(f) {
        Some(n) => Json::Number(n),
        None if f > 0.0 => Json::String("inf".into()),
        None => Json::String("-inf".into()),
    }
}

/// A score argument: a number, or `inf`, `+inf`, `-inf`.
fn score(arg: &[u8]) -> std::result::Result<f64, Reply> {
    let s = std::str::from_utf8(arg).map_err(|_| Reply::err("value is not a valid float"))?;
    match s.to_ascii_lowercase().as_str() {
        "inf" | "+inf" => Ok(f64::INFINITY),
        "-inf" => Ok(f64::NEG_INFINITY),
        t => t.parse::<f64>().ok().filter(|f| f.is_finite()).ok_or_else(|| Reply::err("value is not a valid float")),
    }
}

#[derive(Clone, Copy)]
struct ScoreBound {
    value: f64,
    exclusive: bool,
}

fn score_bound(arg: &[u8]) -> std::result::Result<ScoreBound, Reply> {
    let bad = || Reply::err("min or max is not a float");
    match arg.strip_prefix(b"(") {
        Some(rest) => Ok(ScoreBound { value: score(rest).map_err(|_| bad())?, exclusive: true }),
        None => Ok(ScoreBound { value: score(arg).map_err(|_| bad())?, exclusive: false }),
    }
}

#[derive(Clone)]
enum LexBound {
    Lowest,
    Highest,
    Inclusive(String),
    Exclusive(String),
}

fn lex_bound(arg: &[u8]) -> std::result::Result<LexBound, Reply> {
    let rest = || text(&arg[1..]);
    match arg.first() {
        Some(b'-') if arg.len() == 1 => Ok(LexBound::Lowest),
        Some(b'+') if arg.len() == 1 => Ok(LexBound::Highest),
        Some(b'[') => Ok(LexBound::Inclusive(rest()?)),
        Some(b'(') => Ok(LexBound::Exclusive(rest()?)),
        _ => Err(Reply::err("min or max not valid string range item")),
    }
}

fn above_min(m: &str, b: &LexBound) -> bool {
    match b {
        LexBound::Lowest => true,
        LexBound::Highest => false,
        LexBound::Inclusive(s) => m >= s.as_str(),
        LexBound::Exclusive(s) => m > s.as_str(),
    }
}

fn below_max(m: &str, b: &LexBound) -> bool {
    match b {
        LexBound::Lowest => false,
        LexBound::Highest => true,
        LexBound::Inclusive(s) => m <= s.as_str(),
        LexBound::Exclusive(s) => m < s.as_str(),
    }
}

/// Which members a range command picks.
enum Range {
    /// Positions, negative ones counted from the end.
    Rank(i64, i64),
    Score {
        min: ScoreBound,
        max: ScoreBound,
    },
    Lex {
        min: LexBound,
        max: LexBound,
    },
}

/// Members ordered by score, then member.
fn sorted(members: &Members) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = members.iter().map(|(m, s)| (m.clone(), score_of(s))).collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// Clamps Redis start/stop positions (negative from the end) to `len`.
fn rank_span(start: i64, stop: i64, len: usize) -> Option<(usize, usize)> {
    let len = len as i64;
    let norm = |i: i64| if i < 0 { len + i } else { i };
    let (start, stop) = (norm(start).max(0), norm(stop).min(len - 1));
    (start <= stop && start < len).then_some((start as usize, stop as usize))
}

/// The members `range` picks, in order (reversed with `rev`), then `limit`.
fn select(mut items: Vec<(String, f64)>, range: &Range, rev: bool, limit: Option<(i64, i64)>) -> Vec<(String, f64)> {
    match range {
        Range::Rank(start, stop) => {
            if rev {
                items.reverse();
            }
            match rank_span(*start, *stop, items.len()) {
                Some((a, b)) => items.drain(a..=b).collect(),
                None => Vec::new(),
            }
        }
        Range::Score { min, max } => {
            items.retain(|(_, s)| {
                (if min.exclusive { *s > min.value } else { *s >= min.value })
                    && (if max.exclusive { *s < max.value } else { *s <= max.value })
            });
            limited(items, rev, limit)
        }
        Range::Lex { min, max } => {
            items.retain(|(m, _)| above_min(m, min) && below_max(m, max));
            limited(items, rev, limit)
        }
    }
}

fn limited(mut items: Vec<(String, f64)>, rev: bool, limit: Option<(i64, i64)>) -> Vec<(String, f64)> {
    if rev {
        items.reverse();
    }
    match limit {
        None => items,
        Some((offset, _)) if offset < 0 => Vec::new(),
        Some((offset, count)) => {
            let it = items.into_iter().skip(offset as usize);
            if count < 0 {
                it.collect()
            } else {
                it.take(count as usize).collect()
            }
        }
    }
}

fn upper(arg: &[u8]) -> String {
    String::from_utf8_lossy(arg).to_ascii_uppercase()
}

fn syntax() -> Reply {
    Reply::err("syntax error")
}

/// A random index below `n`.
fn pick(n: usize) -> usize {
    (crate::util::random_u64() % n as u64) as usize
}

/// How `ZUNIONSTORE` and friends combine the scores of one member.
#[derive(Clone, Copy)]
enum Aggregate {
    Sum,
    Min,
    Max,
}

impl Conn {
    /// The members of the set or sorted set at `k` (`kind`), and its expiry.
    /// A key holding anything else answers `WRONGTYPE`.
    fn load(&mut self, k: &str, kind: &str) -> std::result::Result<Option<(Members, i64)>, Reply> {
        match self.get(k)? {
            None => Ok(None),
            Some((Value::Doc(j), meta)) if marked(&j) == Some(kind) => {
                let members = match (kind, &j["members"]) {
                    ("set", Json::Array(a)) => {
                        a.iter().filter_map(|v| v.as_str()).map(|m| (m.to_string(), Json::Null)).collect()
                    }
                    (_, Json::Object(o)) => o.clone(),
                    _ => Map::new(),
                };
                Ok(Some((members, meta.expires_at)))
            }
            Some(_) => Err(Reply::wrong_type()),
        }
    }

    fn members(&mut self, k: &str, kind: &str) -> std::result::Result<Members, Reply> {
        Ok(self.load(k, kind)?.map(|(m, _)| m).unwrap_or_default())
    }

    /// Saves members; like Redis, an empty set or sorted set disappears.
    fn store(&mut self, k: &str, kind: &str, members: Members, expires_at: i64) -> std::result::Result<(), Reply> {
        if members.is_empty() {
            self.sess.kv_del(k)?;
            return Ok(());
        }
        self.check_size(k, if kind == "set" { "set" } else { "sorted set" }, members.len())?;
        let body = if kind == "set" {
            Json::Array(members.into_iter().map(|(m, _)| Json::String(m)).collect())
        } else {
            Json::Object(members)
        };
        self.put(k, Value::Doc(serde_json::json!({ MARK: kind, "members": body })), expires_at)
    }

    /// Members with their scores, as Redis answers `WITHSCORES`: a flat list
    /// in RESP2, pairs in RESP3.
    fn scored(&self, items: &[(String, f64)], with_scores: bool) -> Reply {
        if !with_scores {
            return Reply::array(items.iter().map(|(m, _)| Reply::bulk(m.clone())).collect());
        }
        if self.resp3 {
            Reply::array(
                items.iter().map(|(m, s)| Reply::array(vec![Reply::bulk(m.clone()), Reply::Double(*s)])).collect(),
            )
        } else {
            Reply::array(items.iter().flat_map(|(m, s)| [Reply::bulk(m.clone()), Reply::Double(*s)]).collect())
        }
    }

    /// The scores of a sorted set, or of a set (each member scoring 1), for
    /// `ZUNIONSTORE` and friends.
    fn weighted(&mut self, k: &str) -> std::result::Result<Members, Reply> {
        match self.get(k)? {
            None => Ok(Map::new()),
            Some((Value::Doc(j), _)) if marked(&j) == Some("zset") => Ok(self.members(k, "zset")?),
            Some((Value::Doc(j), _)) if marked(&j) == Some("set") => {
                Ok(self.members(k, "set")?.into_iter().map(|(m, _)| m).map(|m| (m, Json::from(1))).collect())
            }
            Some(_) => Err(Reply::wrong_type()),
        }
    }

    /// Runs a set or sorted-set command; `None` when `name` is not one.
    pub(super) fn run_sets(&mut self, name: &str, args: &[Vec<u8>]) -> Option<Res> {
        Some(match name {
            "SADD" | "SREM" | "SMEMBERS" | "SISMEMBER" | "SMISMEMBER" | "SCARD" | "SPOP" | "SRANDMEMBER" | "SMOVE"
            | "SINTER" | "SUNION" | "SDIFF" | "SINTERSTORE" | "SUNIONSTORE" | "SDIFFSTORE" | "SINTERCARD" | "SSCAN" => {
                self.set_command(name, args)
            }
            "ZADD" | "ZINCRBY" | "ZREM" | "ZSCORE" | "ZMSCORE" | "ZCARD" | "ZCOUNT" | "ZLEXCOUNT" | "ZRANK"
            | "ZREVRANK" | "ZRANGE" | "ZRANGESTORE" | "ZREVRANGE" | "ZRANGEBYSCORE" | "ZREVRANGEBYSCORE"
            | "ZRANGEBYLEX" | "ZREVRANGEBYLEX" | "ZPOPMIN" | "ZPOPMAX" | "ZREMRANGEBYRANK" | "ZREMRANGEBYSCORE"
            | "ZREMRANGEBYLEX" | "ZRANDMEMBER" | "ZSCAN" | "ZUNION" | "ZINTER" | "ZDIFF" | "ZUNIONSTORE"
            | "ZINTERSTORE" | "ZDIFFSTORE" | "ZINTERCARD" => self.zset_command(name, args),
            _ => return None,
        })
    }

    // -- sets ---------------------------------------------------------------

    fn set_command(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "SADD" | "SREM" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let (mut set, exp) = self.load(&k, "set")?.unwrap_or_default();
                let mut n = 0;
                for m in &args[2..] {
                    let m = text(m)?;
                    let changed = if name == "SADD" {
                        set.insert(m, Json::Null).is_none()
                    } else {
                        set.shift_remove(&m).is_some()
                    };
                    n += changed as i64;
                }
                if n > 0 {
                    self.store(&k, "set", set, exp)?;
                }
                Ok(Reply::Int(n))
            }
            "SMEMBERS" => {
                arity(args, 2)?;
                let set = self.members(&key(&args[1])?, "set")?;
                Ok(Reply::Set(set.into_iter().map(|(m, _)| m).map(Reply::bulk).collect()))
            }
            "SISMEMBER" => {
                arity(args, 3)?;
                let set = self.members(&key(&args[1])?, "set")?;
                Ok(Reply::Int(set.contains_key(&text(&args[2])?) as i64))
            }
            "SMISMEMBER" => {
                arity(args, 3)?;
                let set = self.members(&key(&args[1])?, "set")?;
                let mut out = Vec::new();
                for m in &args[2..] {
                    out.push(Reply::Int(set.contains_key(&text(m)?) as i64));
                }
                Ok(Reply::array(out))
            }
            "SCARD" => {
                arity(args, 2)?;
                Ok(Reply::Int(self.members(&key(&args[1])?, "set")?.len() as i64))
            }
            "SPOP" | "SRANDMEMBER" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let count = args.get(2).map(|c| int(c)).transpose()?;
                if name == "SPOP" && count.is_some_and(|c| c < 0) {
                    return Err(Reply::err("value is out of range, must be positive"));
                }
                let Some((mut set, exp)) = self.load(&k, "set")? else {
                    return Ok(if count.is_some() { Reply::array(Vec::new()) } else { Reply::nil() });
                };
                let mut chosen: Vec<String> = Vec::new();
                match count {
                    // SRANDMEMBER with a negative count may repeat members.
                    Some(c) if c < 0 => {
                        let all: Vec<&String> = set.keys().collect();
                        for _ in 0..c.unsigned_abs().min(1_000_000) {
                            chosen.push(all[pick(all.len())].clone());
                        }
                    }
                    _ => {
                        let mut pool: Vec<String> = set.keys().cloned().collect();
                        for _ in 0..count.unwrap_or(1).min(pool.len() as i64) {
                            let i = pick(pool.len());
                            chosen.push(pool.swap_remove(i));
                        }
                    }
                }
                if name == "SPOP" && !chosen.is_empty() {
                    for m in &chosen {
                        set.shift_remove(m);
                    }
                    self.store(&k, "set", set, exp)?;
                }
                Ok(match count {
                    None => Reply::Bulk(chosen.pop().map(String::into_bytes)),
                    Some(_) => Reply::array(chosen.into_iter().map(Reply::bulk).collect()),
                })
            }
            "SMOVE" => {
                arity(args, 4)?;
                let (src, dst, m) = (key(&args[1])?, key(&args[2])?, text(&args[3])?);
                let Some((mut from, exp)) = self.load(&src, "set")? else {
                    return Ok(Reply::Int(0));
                };
                let (mut to, to_exp) = self.load(&dst, "set")?.unwrap_or_default();
                if from.shift_remove(&m).is_none() {
                    return Ok(Reply::Int(0));
                }
                if src != dst {
                    to.insert(m, Json::Null);
                    self.store(&src, "set", from, exp)?;
                    self.store(&dst, "set", to, to_exp)?;
                }
                Ok(Reply::Int(1))
            }
            "SINTER" | "SUNION" | "SDIFF" | "SINTERSTORE" | "SUNIONSTORE" | "SDIFFSTORE" => {
                let store = name.ends_with("STORE");
                arity(args, if store { 3 } else { 2 })?;
                let first = if store { 2 } else { 1 };
                let mut sets = Vec::new();
                for k in &args[first..] {
                    sets.push(self.members(&key(k)?, "set")?);
                }
                let mut result: Members = Map::new();
                match name.trim_end_matches("STORE") {
                    "SINTER" => {
                        if let Some((head, rest)) = sets.split_first() {
                            for m in head.keys() {
                                if rest.iter().all(|s| s.contains_key(m)) {
                                    result.insert(m.clone(), Json::Null);
                                }
                            }
                        }
                    }
                    "SUNION" => {
                        for s in sets {
                            result.extend(s);
                        }
                    }
                    _ => {
                        if let Some((head, rest)) = sets.split_first() {
                            for m in head.keys() {
                                if !rest.iter().any(|s| s.contains_key(m)) {
                                    result.insert(m.clone(), Json::Null);
                                }
                            }
                        }
                    }
                }
                if store {
                    let n = result.len();
                    self.store(&key(&args[1])?, "set", result, 0)?;
                    Ok(Reply::Int(n as i64))
                } else {
                    Ok(Reply::Set(result.into_iter().map(|(m, _)| m).map(Reply::bulk).collect()))
                }
            }
            "SINTERCARD" => {
                arity(args, 3)?;
                let n = usize::try_from(int(&args[1])?)
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| Reply::err("numkeys should be greater than 0"))?;
                if args.len() < 2 + n {
                    return Err(Reply::err("Number of keys can't be greater than number of args"));
                }
                let limit = match &args[2 + n..] {
                    [] => 0,
                    [l, v] if upper(l) == "LIMIT" => {
                        usize::try_from(int(v)?).map_err(|_| Reply::err("LIMIT can't be negative"))?
                    }
                    _ => return Err(syntax()),
                };
                let mut sets = Vec::new();
                for k in &args[2..2 + n] {
                    sets.push(self.members(&key(k)?, "set")?);
                }
                let (head, rest) = sets.split_first().expect("numkeys > 0");
                let mut count = 0;
                for m in head.keys() {
                    if rest.iter().all(|s| s.contains_key(m)) {
                        count += 1;
                        if limit > 0 && count == limit {
                            break;
                        }
                    }
                }
                Ok(Reply::Int(count as i64))
            }
            _ => {
                // SSCAN: everything at once, cursor 0.
                arity(args, 3)?;
                let set = self.members(&key(&args[1])?, "set")?;
                let pattern = scan_pattern(&args[3..])?;
                let items = set
                    .into_iter()
                    .map(|(m, _)| m)
                    .filter(|m| crate::pubsub::glob(pattern.as_bytes(), m.as_bytes()))
                    .map(Reply::bulk)
                    .collect();
                Ok(Reply::array(vec![Reply::bulk("0"), Reply::array(items)]))
            }
        }
    }

    // -- sorted sets ----------------------------------------------------------

    fn zset_command(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "ZADD" => self.zadd(args),
            "ZINCRBY" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let step = score(&args[2])?;
                let m = text(&args[3])?;
                let (mut z, exp) = self.load(&k, "zset")?.unwrap_or_default();
                let next = z.get(&m).map(score_of).unwrap_or(0.0) + step;
                if next.is_nan() {
                    return Err(Reply::err("resulting score is not a number (NaN)"));
                }
                z.insert(m, score_json(next));
                self.store(&k, "zset", z, exp)?;
                Ok(Reply::Double(next))
            }
            "ZREM" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let Some((mut z, exp)) = self.load(&k, "zset")? else {
                    return Ok(Reply::Int(0));
                };
                let mut n = 0;
                for m in &args[2..] {
                    n += z.shift_remove(&text(m)?).is_some() as i64;
                }
                if n > 0 {
                    self.store(&k, "zset", z, exp)?;
                }
                Ok(Reply::Int(n))
            }
            "ZSCORE" => {
                arity(args, 3)?;
                let z = self.members(&key(&args[1])?, "zset")?;
                Ok(z.get(&text(&args[2])?).map(|s| Reply::Double(score_of(s))).unwrap_or_else(Reply::nil))
            }
            "ZMSCORE" => {
                arity(args, 3)?;
                let z = self.members(&key(&args[1])?, "zset")?;
                let mut out = Vec::new();
                for m in &args[2..] {
                    out.push(z.get(&text(m)?).map(|s| Reply::Double(score_of(s))).unwrap_or_else(Reply::nil));
                }
                Ok(Reply::array(out))
            }
            "ZCARD" => {
                arity(args, 2)?;
                Ok(Reply::Int(self.members(&key(&args[1])?, "zset")?.len() as i64))
            }
            "ZCOUNT" | "ZLEXCOUNT" => {
                arity(args, 4)?;
                let range = if name == "ZCOUNT" {
                    Range::Score { min: score_bound(&args[2])?, max: score_bound(&args[3])? }
                } else {
                    Range::Lex { min: lex_bound(&args[2])?, max: lex_bound(&args[3])? }
                };
                let z = self.members(&key(&args[1])?, "zset")?;
                Ok(Reply::Int(select(sorted(&z), &range, false, None).len() as i64))
            }
            "ZRANK" | "ZREVRANK" => {
                arity(args, 3)?;
                let with_score = match args.get(3) {
                    None => false,
                    Some(a) if upper(a) == "WITHSCORE" && args.len() == 4 => true,
                    _ => return Err(syntax()),
                };
                let z = self.members(&key(&args[1])?, "zset")?;
                let m = text(&args[2])?;
                let mut items = sorted(&z);
                if name == "ZREVRANK" {
                    items.reverse();
                }
                Ok(match items.iter().position(|(x, _)| *x == m) {
                    None if with_score => Reply::Array(None),
                    None => Reply::nil(),
                    Some(r) if with_score => Reply::array(vec![Reply::Int(r as i64), Reply::Double(items[r].1)]),
                    Some(r) => Reply::Int(r as i64),
                })
            }
            "ZRANGE" | "ZRANGESTORE" => {
                let store = name == "ZRANGESTORE";
                let first = if store { 2 } else { 1 };
                arity(args, first + 3)?;
                let (range, rev, limit, with_scores) = parse_zrange(&args[first + 1..])?;
                if store && with_scores {
                    return Err(syntax());
                }
                let z = self.members(&key(&args[first])?, "zset")?;
                let items = select(sorted(&z), &range, rev, limit);
                if store {
                    let n = items.len();
                    let out: Members = items.into_iter().map(|(m, s)| (m, score_json(s))).collect();
                    self.store(&key(&args[1])?, "zset", out, 0)?;
                    return Ok(Reply::Int(n as i64));
                }
                Ok(self.scored(&items, with_scores))
            }
            "ZREVRANGE" => {
                arity(args, 4)?;
                let with_scores = match args.get(4) {
                    None => false,
                    Some(a) if upper(a) == "WITHSCORES" && args.len() == 5 => true,
                    _ => return Err(syntax()),
                };
                let z = self.members(&key(&args[1])?, "zset")?;
                let items = select(sorted(&z), &Range::Rank(int(&args[2])?, int(&args[3])?), true, None);
                Ok(self.scored(&items, with_scores))
            }
            "ZRANGEBYSCORE" | "ZREVRANGEBYSCORE" | "ZRANGEBYLEX" | "ZREVRANGEBYLEX" => {
                arity(args, 4)?;
                let rev = name.starts_with("ZREV");
                let by_lex = name.ends_with("LEX");
                // The reversed forms take max before min.
                let (lo, hi) = if rev { (&args[3], &args[2]) } else { (&args[2], &args[3]) };
                let range = if by_lex {
                    Range::Lex { min: lex_bound(lo)?, max: lex_bound(hi)? }
                } else {
                    Range::Score { min: score_bound(lo)?, max: score_bound(hi)? }
                };
                let (mut with_scores, mut limit, mut i) = (false, None, 4);
                while i < args.len() {
                    match upper(&args[i]).as_str() {
                        "WITHSCORES" if !by_lex => {
                            with_scores = true;
                            i += 1;
                        }
                        "LIMIT" if i + 2 < args.len() => {
                            limit = Some((int(&args[i + 1])?, int(&args[i + 2])?));
                            i += 3;
                        }
                        _ => return Err(syntax()),
                    }
                }
                let z = self.members(&key(&args[1])?, "zset")?;
                let items = select(sorted(&z), &range, rev, limit);
                Ok(self.scored(&items, with_scores))
            }
            "ZPOPMIN" | "ZPOPMAX" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let count = match args.get(2) {
                    Some(c) => Some(
                        usize::try_from(int(c)?).map_err(|_| Reply::err("value is out of range, must be positive"))?,
                    ),
                    None => None,
                };
                let Some((mut z, exp)) = self.load(&k, "zset")? else {
                    return Ok(Reply::array(Vec::new()));
                };
                let mut items = sorted(&z);
                if name == "ZPOPMAX" {
                    items.reverse();
                }
                items.truncate(count.unwrap_or(1));
                for (m, _) in &items {
                    z.shift_remove(m);
                }
                self.store(&k, "zset", z, exp)?;
                // Without a count, one flat pair, whatever the protocol.
                Ok(match (count, items.first()) {
                    (None, Some((m, s))) => Reply::array(vec![Reply::bulk(m.clone()), Reply::Double(*s)]),
                    (None, None) => Reply::array(Vec::new()),
                    _ => self.scored(&items, true),
                })
            }
            "ZREMRANGEBYRANK" | "ZREMRANGEBYSCORE" | "ZREMRANGEBYLEX" => {
                arity(args, 4)?;
                let range = match name {
                    "ZREMRANGEBYRANK" => Range::Rank(int(&args[2])?, int(&args[3])?),
                    "ZREMRANGEBYSCORE" => Range::Score { min: score_bound(&args[2])?, max: score_bound(&args[3])? },
                    _ => Range::Lex { min: lex_bound(&args[2])?, max: lex_bound(&args[3])? },
                };
                let k = key(&args[1])?;
                let Some((mut z, exp)) = self.load(&k, "zset")? else {
                    return Ok(Reply::Int(0));
                };
                let gone = select(sorted(&z), &range, false, None);
                for (m, _) in &gone {
                    z.shift_remove(m);
                }
                if !gone.is_empty() {
                    self.store(&k, "zset", z, exp)?;
                }
                Ok(Reply::Int(gone.len() as i64))
            }
            "ZRANDMEMBER" => {
                arity(args, 2)?;
                let count = args.get(2).map(|c| int(c)).transpose()?;
                let with_scores = match args.get(3) {
                    None => false,
                    Some(a) if upper(a) == "WITHSCORES" && args.len() == 4 => true,
                    _ => return Err(syntax()),
                };
                let z = self.members(&key(&args[1])?, "zset")?;
                if z.is_empty() {
                    return Ok(if count.is_some() { Reply::array(Vec::new()) } else { Reply::nil() });
                }
                let all: Vec<(String, f64)> = z.iter().map(|(m, s)| (m.clone(), score_of(s))).collect();
                let mut chosen = Vec::new();
                match count {
                    None => return Ok(Reply::bulk(all[pick(all.len())].0.clone())),
                    Some(c) if c < 0 => {
                        for _ in 0..c.unsigned_abs().min(1_000_000) {
                            chosen.push(all[pick(all.len())].clone());
                        }
                    }
                    Some(c) => {
                        let mut pool = all;
                        for _ in 0..(c as usize).min(pool.len()) {
                            let i = pick(pool.len());
                            chosen.push(pool.swap_remove(i));
                        }
                    }
                }
                Ok(self.scored(&chosen, with_scores))
            }
            "ZSCAN" => {
                // Everything at once, cursor 0; scores as text, as Redis sends them.
                arity(args, 3)?;
                let z = self.members(&key(&args[1])?, "zset")?;
                let pattern = scan_pattern(&args[3..])?;
                let items = z
                    .iter()
                    .filter(|(m, _)| crate::pubsub::glob(pattern.as_bytes(), m.as_bytes()))
                    .flat_map(|(m, s)| [Reply::bulk(m.clone()), Reply::bulk(fmt_f64(score_of(s)))])
                    .collect();
                Ok(Reply::array(vec![Reply::bulk("0"), Reply::array(items)]))
            }
            "ZINTERCARD" => {
                arity(args, 3)?;
                let (keys, rest) = numkeys(&args[1..])?;
                let limit = match rest {
                    [] => 0,
                    [l, v] if upper(l) == "LIMIT" => {
                        usize::try_from(int(v)?).map_err(|_| Reply::err("LIMIT can't be negative"))?
                    }
                    _ => return Err(syntax()),
                };
                let mut sets = Vec::new();
                for k in keys {
                    sets.push(self.weighted(&key(k)?)?);
                }
                let (head, others) = sets.split_first().expect("numkeys > 0");
                let mut n = head.keys().filter(|m| others.iter().all(|s| s.contains_key(*m))).count();
                if limit > 0 {
                    n = n.min(limit);
                }
                Ok(Reply::Int(n as i64))
            }
            _ => self.zcombine(name, args),
        }
    }

    /// `ZADD key [NX|XX] [GT|LT] [CH] [INCR] score member ...`
    fn zadd(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 4)?;
        let k = key(&args[1])?;
        let (mut nx, mut xx, mut gt, mut lt, mut ch, mut incr) = (false, false, false, false, false, false);
        let mut i = 2;
        while i < args.len() {
            match upper(&args[i]).as_str() {
                "NX" => nx = true,
                "XX" => xx = true,
                "GT" => gt = true,
                "LT" => lt = true,
                "CH" => ch = true,
                "INCR" => incr = true,
                _ => break,
            }
            i += 1;
        }
        let pairs = &args[i..];
        if pairs.is_empty() || !pairs.len().is_multiple_of(2) {
            return Err(syntax());
        }
        if nx && xx {
            return Err(Reply::err("XX and NX options at the same time are not compatible"));
        }
        if (gt && lt) || (nx && (gt || lt)) {
            return Err(Reply::err("GT, LT, and/or NX options at the same time are not compatible"));
        }
        if incr && pairs.len() > 2 {
            return Err(Reply::err("INCR option supports a single increment-element pair"));
        }
        // Check every score before changing anything.
        let mut parsed = Vec::new();
        for p in pairs.chunks(2) {
            parsed.push((score(&p[0])?, text(&p[1])?));
        }
        let (mut z, exp) = self.load(&k, "zset")?.unwrap_or_default();
        let (mut added, mut changed) = (0, 0);
        let mut result = None;
        for (s, m) in parsed {
            let old = z.get(&m).map(score_of);
            if (nx && old.is_some()) || (xx && old.is_none()) {
                continue;
            }
            let new = if incr { old.unwrap_or(0.0) + s } else { s };
            if new.is_nan() {
                return Err(Reply::err("resulting score is not a number (NaN)"));
            }
            match old {
                Some(o) if (gt && new <= o) || (lt && new >= o) => continue,
                Some(o) => {
                    if new != o {
                        changed += 1;
                        z.insert(m, score_json(new));
                    }
                }
                None => {
                    added += 1;
                    changed += 1;
                    z.insert(m, score_json(new));
                }
            }
            result = Some(new);
        }
        if changed > 0 {
            self.store(&k, "zset", z, exp)?;
        }
        Ok(if incr {
            result.map(Reply::Double).unwrap_or_else(Reply::nil)
        } else {
            Reply::Int(if ch { changed } else { added })
        })
    }

    /// `ZUNION`, `ZINTER`, `ZDIFF` and their `STORE` forms.
    fn zcombine(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        let store = name.ends_with("STORE");
        let first = if store { 2 } else { 1 };
        arity(args, first + 2)?;
        let (keys, rest) = numkeys(&args[first..])?;
        let diff = name.starts_with("ZDIFF");
        let (mut weights, mut agg, mut with_scores) = (vec![1.0; keys.len()], Aggregate::Sum, false);
        let mut i = 0;
        while i < rest.len() {
            match upper(&rest[i]).as_str() {
                "WEIGHTS" if !diff && i + keys.len() < rest.len() => {
                    for (w, a) in weights.iter_mut().zip(&rest[i + 1..=i + keys.len()]) {
                        *w = score(a).map_err(|_| Reply::err("weight value is not a float"))?;
                    }
                    i += 1 + keys.len();
                }
                "AGGREGATE" if !diff && i + 1 < rest.len() => {
                    agg = match upper(&rest[i + 1]).as_str() {
                        "SUM" => Aggregate::Sum,
                        "MIN" => Aggregate::Min,
                        "MAX" => Aggregate::Max,
                        _ => return Err(syntax()),
                    };
                    i += 2;
                }
                "WITHSCORES" if !store => {
                    with_scores = true;
                    i += 1;
                }
                _ => return Err(syntax()),
            }
        }
        let mut inputs = Vec::new();
        for k in keys {
            inputs.push(self.weighted(&key(k)?)?);
        }
        let combine = |a: f64, b: f64| {
            let r = match agg {
                Aggregate::Sum => a + b,
                Aggregate::Min => a.min(b),
                Aggregate::Max => a.max(b),
            };
            // inf + -inf: Redis counts it as 0.
            if r.is_nan() {
                0.0
            } else {
                r
            }
        };
        let mut out: Map<String, Json> = Map::new();
        let (head, others) = inputs.split_first().expect("numkeys > 0");
        let mul = |s: f64, w: f64| if s * w == 0.0 || (s * w).is_nan() { 0.0 } else { s * w };
        if name.starts_with("ZUNION") {
            for (set, w) in inputs.iter().zip(&weights) {
                for (m, s) in set {
                    let v = mul(score_of(s), *w);
                    let next = out.get(m).map(|o| combine(score_of(o), v)).unwrap_or(v);
                    out.insert(m.clone(), score_json(next));
                }
            }
        } else if name.starts_with("ZINTER") {
            for (m, s) in head {
                if others.iter().all(|o| o.contains_key(m)) {
                    let mut acc = mul(score_of(s), weights[0]);
                    for (o, w) in others.iter().zip(&weights[1..]) {
                        acc = combine(acc, mul(score_of(&o[m]), *w));
                    }
                    out.insert(m.clone(), score_json(acc));
                }
            }
        } else {
            for (m, s) in head {
                if !others.iter().any(|o| o.contains_key(m)) {
                    out.insert(m.clone(), s.clone());
                }
            }
        }
        if store {
            let n = out.len();
            self.store(&key(&args[1])?, "zset", out, 0)?;
            return Ok(Reply::Int(n as i64));
        }
        Ok(self.scored(&sorted(&out), with_scores))
    }
}

/// `numkeys key ...` at the start of `args`: the keys and what follows them.
/// Keys, then the arguments after them.
type Split<'a> = (&'a [Vec<u8>], &'a [Vec<u8>]);

fn numkeys(args: &[Vec<u8>]) -> std::result::Result<Split<'_>, Reply> {
    let n = usize::try_from(int(&args[0])?)
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| Reply::err("numkeys should be greater than 0"))?;
    if args.len() < 1 + n {
        return Err(Reply::err("Number of keys can't be greater than number of args"));
    }
    Ok((&args[1..1 + n], &args[1 + n..]))
}

/// `[MATCH pattern] [COUNT n]` of SSCAN and ZSCAN.
pub(super) fn scan_pattern(args: &[Vec<u8>]) -> std::result::Result<String, Reply> {
    let mut pattern = "*".to_string();
    for pair in args.chunks(2) {
        match (upper(&pair[0]).as_str(), pair.get(1)) {
            ("MATCH", Some(p)) => pattern = text(p)?,
            ("COUNT", Some(c)) => {
                int(c)?;
            }
            _ => return Err(syntax()),
        }
    }
    Ok(pattern)
}

/// `start stop [BYSCORE|BYLEX] [REV] [LIMIT offset count] [WITHSCORES]` of ZRANGE.
/// What ZRANGE asks: the range, reversed or not, the limit, with scores or not.
type ZRangeArgs = (Range, bool, Option<(i64, i64)>, bool);

fn parse_zrange(args: &[Vec<u8>]) -> std::result::Result<ZRangeArgs, Reply> {
    let (start, stop) = (&args[0], &args[1]);
    let (mut by, mut rev, mut limit, mut with_scores) = ("RANK", false, None, false);
    let mut i = 2;
    while i < args.len() {
        match upper(&args[i]).as_str() {
            "BYSCORE" => by = "SCORE",
            "BYLEX" => by = "LEX",
            "REV" => rev = true,
            "WITHSCORES" => with_scores = true,
            "LIMIT" if i + 2 < args.len() => {
                limit = Some((int(&args[i + 1])?, int(&args[i + 2])?));
                i += 2;
            }
            _ => return Err(syntax()),
        }
        i += 1;
    }
    if limit.is_some() && by == "RANK" {
        return Err(Reply::err("syntax error, LIMIT is only supported in combination with either BYSCORE or BYLEX"));
    }
    if with_scores && by == "LEX" {
        return Err(Reply::err("syntax error, WITHSCORES not supported in combination with BYLEX"));
    }
    // With REV, the range is given from the high end.
    let (lo, hi) = if rev { (stop, start) } else { (start, stop) };
    let range = match by {
        "SCORE" => Range::Score { min: score_bound(lo)?, max: score_bound(hi)? },
        "LEX" => Range::Lex { min: lex_bound(lo)?, max: lex_bound(hi)? },
        _ => Range::Rank(int(start)?, int(stop)?),
    };
    Ok((range, rev, limit, with_scores))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(pairs: &[(&str, f64)]) -> Members {
        pairs.iter().map(|(m, s)| (m.to_string(), score_json(*s))).collect()
    }

    fn names(items: &[(String, f64)]) -> Vec<&str> {
        items.iter().map(|(m, _)| m.as_str()).collect()
    }

    #[test]
    fn ranges_by_rank_score_and_lex() {
        let m = z(&[("c", 2.0), ("a", 1.0), ("b", 1.0), ("d", f64::INFINITY)]);
        let all = sorted(&m);
        assert_eq!(names(&all), ["a", "b", "c", "d"]);
        assert_eq!(names(&select(all.clone(), &Range::Rank(1, -2), false, None)), ["b", "c"]);
        assert_eq!(names(&select(all.clone(), &Range::Rank(0, 0), true, None)), ["d"]);
        assert!(select(all.clone(), &Range::Rank(5, 9), false, None).is_empty());
        let s = |a: &[u8], b: &[u8]| Range::Score { min: score_bound(a).unwrap(), max: score_bound(b).unwrap() };
        assert_eq!(names(&select(all.clone(), &s(b"(1", b"+inf"), false, None)), ["c", "d"]);
        assert_eq!(names(&select(all.clone(), &s(b"-inf", b"2"), true, Some((1, 1)))), ["b"]);
        let l = |a: &[u8], b: &[u8]| Range::Lex { min: lex_bound(a).unwrap(), max: lex_bound(b).unwrap() };
        assert_eq!(names(&select(all.clone(), &l(b"[b", b"(d"), false, None)), ["b", "c"]);
        assert_eq!(names(&select(all, &l(b"-", b"+"), false, Some((0, -1)))), ["a", "b", "c", "d"]);
        assert!(lex_bound(b"b").is_err() && score_bound(b"x").is_err());
    }

    #[test]
    fn scores_survive_json() {
        for f in [1.5, -3.0, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            assert_eq!(score_of(&score_json(f)), f);
        }
        assert!(score(b"nan").is_err());
        assert_eq!(score(b"+inf").unwrap(), f64::INFINITY);
    }
}
