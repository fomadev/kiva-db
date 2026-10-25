// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! The rest of the classic Redis commands on strings, keys, hashes and
//! lists: the ones applications and libraries use less often than `GET`,
//! `HSET` or `LPUSH`, but expect to find (`LMOVE` for reliable queues,
//! `LTRIM` for capped lists, `HINCRBYFLOAT`, `COPY`...).
//!
//! They work on the same values as the commands of `resp.rs`: strings are
//! KivaDB strings, hashes KivaDB objects, lists KivaDB arrays of strings.
//! Like there, a change keeps the key's expiry.

use super::*;

fn upper(arg: &[u8]) -> String {
    String::from_utf8_lossy(arg).to_ascii_uppercase()
}

fn syntax() -> Reply {
    Reply::err("syntax error")
}

/// `LEFT` (true) or `RIGHT` (false).
fn side(arg: &[u8]) -> std::result::Result<bool, Reply> {
    match upper(arg).as_str() {
        "LEFT" => Ok(true),
        "RIGHT" => Ok(false),
        _ => Err(syntax()),
    }
}

/// A random index below `n`.
fn pick(n: usize) -> usize {
    (crate::util::random_u64() % n as u64) as usize
}

/// Positions `start..=stop` of a sequence of `len` items, negative ones
/// counted from the end, clamped as Redis does; `None` when empty.
fn span(start: i64, stop: i64, len: usize) -> Option<(usize, usize)> {
    let len = len as i64;
    let norm = |i: i64| if i < 0 { len + i } else { i };
    let (start, stop) = (norm(start).max(0), norm(stop).min(len - 1));
    (start <= stop && start < len).then_some((start as usize, stop as usize))
}

/// Largest string `SETRANGE` may build, as in Redis.
const MAX_STRING: i64 = 512 * 1024 * 1024;

impl Conn {
    /// The list at `k` and its expiry; `None` when there is no key.
    fn list(&mut self, k: &str) -> std::result::Result<Option<(Vec<Json>, i64)>, Reply> {
        Ok(match self.doc(k, Type::Array)? {
            Some((Json::Array(a), meta)) => Some((a, meta.expires_at)),
            _ => None,
        })
    }

    /// Saves a list; like Redis, an empty list disappears.
    fn store_list(&mut self, k: &str, list: Vec<Json>, expires_at: i64) -> std::result::Result<(), Reply> {
        if list.is_empty() {
            self.sess.kv_del(k)?;
            return Ok(());
        }
        self.check_size(k, "list", list.len())?;
        self.put(k, Value::Doc(Json::Array(list)), expires_at)
    }

    /// The hash at `k` and its expiry; an empty one when there is no key.
    fn hash(&mut self, k: &str) -> std::result::Result<(Map<String, Json>, i64), Reply> {
        Ok(match self.doc(k, Type::Object)? {
            Some((Json::Object(m), meta)) => (m, meta.expires_at),
            _ => (Map::new(), 0),
        })
    }

    /// The string at `k` as bytes, and its expiry.
    fn string(&mut self, k: &str) -> std::result::Result<Option<(Vec<u8>, i64)>, Reply> {
        match self.get(k)? {
            None => Ok(None),
            Some((v, m)) => Ok(Some((bytes_of(&v).ok_or_else(Reply::wrong_type)?, m.expires_at))),
        }
    }

    /// Runs one of the commands of this module; `None` when `name` is not one.
    pub(super) fn run_classic(&mut self, name: &str, args: &[Vec<u8>]) -> Option<Res> {
        Some(match name {
            "GETRANGE" | "SUBSTR" | "SETRANGE" | "MSETNX" => self.string_command(name, args),
            "RENAMENX" | "COPY" | "TOUCH" => self.key_command(name, args),
            "HINCRBYFLOAT" | "HSTRLEN" | "HSCAN" | "HRANDFIELD" => self.hash_command(name, args),
            "LPUSHX" | "RPUSHX" | "LSET" | "LREM" | "LTRIM" | "LINSERT" | "LPOS" | "LMOVE" | "RPOPLPUSH" | "LMPOP" => {
                self.list_command(name, args)
            }
            _ => return None,
        })
    }

    // -- strings ------------------------------------------------------------

    fn string_command(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "GETRANGE" | "SUBSTR" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let (start, stop) = (int(&args[2])?, int(&args[3])?);
                let s = self.string(&k)?.map(|(s, _)| s).unwrap_or_default();
                Ok(Reply::bulk(match span(start, stop, s.len()) {
                    Some((a, b)) => s[a..=b].to_vec(),
                    None => Vec::new(),
                }))
            }
            "SETRANGE" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let offset = int(&args[2])?;
                if offset < 0 {
                    return Err(Reply::err("offset is out of range"));
                }
                let piece = &args[3];
                let (mut s, exp) = self.string(&k)?.unwrap_or_default();
                if piece.is_empty() {
                    return Ok(Reply::Int(s.len() as i64));
                }
                if offset + piece.len() as i64 > MAX_STRING {
                    return Err(Reply::err("string exceeds maximum allowed size (proto-max-bulk-len)"));
                }
                let end = offset as usize + piece.len();
                if s.len() < end {
                    s.resize(end, 0);
                }
                s[offset as usize..end].copy_from_slice(piece);
                let len = s.len();
                self.put(&k, value_of(&s), exp)?;
                Ok(Reply::Int(len as i64))
            }
            _ => {
                // MSETNX: all the keys or none, and none if one exists.
                if args.len() < 3 || args.len().is_multiple_of(2) {
                    return Err(Reply::err("wrong number of arguments for 'msetnx' command"));
                }
                let mut ops = Vec::new();
                for pair in args[1..].chunks(2) {
                    let k = key(&pair[0])?;
                    if self.sess.kv_exists(&k)? {
                        return Ok(Reply::Int(0));
                    }
                    ops.push(BatchOp::Put { key: k, value: value_of(&pair[1]), expires_at: 0, msgpack: false });
                }
                self.sess.apply(ops)?;
                Ok(Reply::Int(1))
            }
        }
    }

    // -- keys ---------------------------------------------------------------

    fn key_command(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "RENAMENX" => {
                arity(args, 3)?;
                let (from, to) = (key(&args[1])?, key(&args[2])?);
                let Some((v, m)) = self.get(&from)? else {
                    return Err(Reply::err("no such key"));
                };
                if from == to || self.sess.kv_exists(&to)? {
                    return Ok(Reply::Int(0));
                }
                self.sess.apply(vec![
                    BatchOp::Put { key: to, value: v, expires_at: m.expires_at, msgpack: m.msgpack },
                    BatchOp::Del { key: from },
                ])?;
                Ok(Reply::Int(1))
            }
            "COPY" => {
                arity(args, 3)?;
                let (from, to) = (key(&args[1])?, key(&args[2])?);
                let mut replace = false;
                for opt in &args[3..] {
                    match upper(opt).as_str() {
                        "REPLACE" => replace = true,
                        // Databases are workspaces, and one command changes one.
                        "DB" => return Err(Reply::err("COPY ... DB is not supported: SELECT the database, then copy")),
                        _ => return Err(syntax()),
                    }
                }
                let Some((v, m)) = self.get(&from)? else { return Ok(Reply::Int(0)) };
                if from == to || (!replace && self.sess.kv_exists(&to)?) {
                    return Ok(Reply::Int(0));
                }
                self.sess.kv_put(&to, v, m.expires_at, m.msgpack)?;
                Ok(Reply::Int(1))
            }
            _ => {
                // TOUCH: KivaDB keeps no access time, so it only counts.
                arity(args, 2)?;
                let mut n = 0;
                for a in &args[1..] {
                    n += self.sess.kv_exists(&key(a)?)? as i64;
                }
                Ok(Reply::Int(n))
            }
        }
    }

    // -- hashes -------------------------------------------------------------

    fn hash_command(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "HINCRBYFLOAT" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let f = text(&args[2])?;
                let step = float(&args[3])?;
                let (mut obj, exp) = self.hash(&k)?;
                let cur = match obj.get(&f) {
                    None => 0.0,
                    Some(Json::Number(n)) => n.as_f64().unwrap_or(0.0),
                    Some(Json::String(s)) => {
                        float(s.as_bytes()).map_err(|_| Reply::err("hash value is not a float"))?
                    }
                    Some(_) => return Err(Reply::err("hash value is not a float")),
                };
                let next = cur + step;
                if !next.is_finite() {
                    return Err(Reply::err("increment would produce NaN or Infinity"));
                }
                let text = fmt_f64(next);
                obj.insert(f, Json::String(text.clone()));
                self.put(&k, Value::Doc(Json::Object(obj)), exp)?;
                Ok(Reply::bulk(text))
            }
            "HSTRLEN" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let f = text(&args[2])?;
                let (obj, _) = self.hash(&k)?;
                Ok(Reply::Int(obj.get(&f).map_or(0, |v| json_text(v).len()) as i64))
            }
            "HSCAN" => {
                // Everything at once, cursor 0, as SSCAN and ZSCAN do.
                arity(args, 3)?;
                let k = key(&args[1])?;
                let mut opts: Vec<Vec<u8>> = args[3..].to_vec();
                let no_values = opts.iter().any(|o| upper(o) == "NOVALUES");
                opts.retain(|o| upper(o) != "NOVALUES");
                let pattern = sets::scan_pattern(&opts)?;
                let (obj, _) = self.hash(&k)?;
                let items = obj
                    .iter()
                    .filter(|(f, _)| crate::pubsub::glob(pattern.as_bytes(), f.as_bytes()))
                    .flat_map(|(f, v)| {
                        let field = Reply::bulk(f.clone());
                        if no_values {
                            vec![field]
                        } else {
                            vec![field, Reply::bulk(json_text(v))]
                        }
                    })
                    .collect();
                Ok(Reply::array(vec![Reply::bulk("0"), Reply::array(items)]))
            }
            _ => {
                // HRANDFIELD key [count [WITHVALUES]]
                arity(args, 2)?;
                let k = key(&args[1])?;
                let (obj, _) = self.hash(&k)?;
                let fields: Vec<(&String, &Json)> = obj.iter().collect();
                let Some(count) = args.get(2) else {
                    return Ok(match fields.len() {
                        0 => Reply::nil(),
                        n => Reply::bulk(fields[pick(n)].0.clone()),
                    });
                };
                let count = int(count)?;
                let with_values = match args.get(3) {
                    None => false,
                    Some(o) if upper(o) == "WITHVALUES" && args.len() == 4 => true,
                    _ => return Err(syntax()),
                };
                let chosen: Vec<usize> = if fields.is_empty() {
                    Vec::new()
                } else if count >= 0 {
                    // Distinct fields, in a random order.
                    let mut all: Vec<usize> = (0..fields.len()).collect();
                    for i in (1..all.len()).rev() {
                        all.swap(i, pick(i + 1));
                    }
                    all.truncate(count as usize);
                    all
                } else {
                    // A negative count may repeat fields.
                    (0..count.unsigned_abs()).map(|_| pick(fields.len())).collect()
                };
                let field = |i: usize| Reply::bulk(fields[i].0.clone());
                let value = |i: usize| Reply::bulk(json_text(fields[i].1));
                Ok(Reply::array(if !with_values {
                    chosen.into_iter().map(field).collect()
                } else if self.resp3 {
                    chosen.into_iter().map(|i| Reply::array(vec![field(i), value(i)])).collect()
                } else {
                    chosen.into_iter().flat_map(|i| [field(i), value(i)]).collect()
                }))
            }
        }
    }

    // -- lists --------------------------------------------------------------

    fn list_command(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "LPUSHX" | "RPUSHX" => {
                arity(args, 3)?;
                let k = key(&args[1])?;
                let Some((mut list, exp)) = self.list(&k)? else { return Ok(Reply::Int(0)) };
                for v in &args[2..] {
                    let item = Json::String(text(v)?);
                    if name == "LPUSHX" {
                        list.insert(0, item);
                    } else {
                        list.push(item);
                    }
                }
                let len = list.len();
                self.store_list(&k, list, exp)?;
                Ok(Reply::Int(len as i64))
            }
            "LSET" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let Some((mut list, exp)) = self.list(&k)? else {
                    return Err(Reply::err("no such key"));
                };
                let i = int(&args[2])?;
                let i = if i < 0 { list.len() as i64 + i } else { i };
                if !(0..list.len() as i64).contains(&i) {
                    return Err(Reply::err("index out of range"));
                }
                list[i as usize] = Json::String(text(&args[3])?);
                self.store_list(&k, list, exp)?;
                Ok(Reply::ok())
            }
            "LREM" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let count = int(&args[2])?;
                let target = &args[3];
                let Some((list, exp)) = self.list(&k)? else { return Ok(Reply::Int(0)) };
                let limit = if count == 0 { usize::MAX } else { count.unsigned_abs() as usize };
                let mut removed = 0;
                let mut keep = |v: &Json| {
                    if removed < limit && json_text(v) == *target {
                        removed += 1;
                        false
                    } else {
                        true
                    }
                };
                // A negative count removes from the tail.
                let kept: Vec<Json> = if count < 0 {
                    let mut kept: Vec<Json> = list.into_iter().rev().filter(|v| keep(v)).collect();
                    kept.reverse();
                    kept
                } else {
                    list.into_iter().filter(|v| keep(v)).collect()
                };
                if removed > 0 {
                    self.store_list(&k, kept, exp)?;
                }
                Ok(Reply::Int(removed as i64))
            }
            "LTRIM" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let (start, stop) = (int(&args[2])?, int(&args[3])?);
                let Some((list, exp)) = self.list(&k)? else { return Ok(Reply::ok()) };
                let kept = match span(start, stop, list.len()) {
                    Some((a, b)) if a == 0 && b + 1 == list.len() => return Ok(Reply::ok()),
                    Some((a, b)) => list[a..=b].to_vec(),
                    None => Vec::new(),
                };
                self.store_list(&k, kept, exp)?;
                Ok(Reply::ok())
            }
            "LINSERT" => {
                arity(args, 5)?;
                let k = key(&args[1])?;
                let before = match upper(&args[2]).as_str() {
                    "BEFORE" => true,
                    "AFTER" => false,
                    _ => return Err(syntax()),
                };
                let Some((mut list, exp)) = self.list(&k)? else { return Ok(Reply::Int(0)) };
                let Some(at) = list.iter().position(|v| json_text(v) == args[3]) else {
                    return Ok(Reply::Int(-1));
                };
                list.insert(if before { at } else { at + 1 }, Json::String(text(&args[4])?));
                let len = list.len();
                self.store_list(&k, list, exp)?;
                Ok(Reply::Int(len as i64))
            }
            "LPOS" => self.lpos(args),
            "LMOVE" | "RPOPLPUSH" => {
                let (from_left, to_left) = if name == "LMOVE" {
                    arity(args, 5)?;
                    (side(&args[3])?, side(&args[4])?)
                } else {
                    arity(args, 3)?;
                    (false, true)
                };
                let (src, dst) = (key(&args[1])?, key(&args[2])?);
                let Some((mut list, exp)) = self.list(&src)?.filter(|(l, _)| !l.is_empty()) else {
                    return Ok(Reply::nil());
                };
                // A destination of another type is refused before anything moves.
                let target = if src == dst { None } else { Some(self.list(&dst)?.unwrap_or_default()) };
                let item = if from_left { list.remove(0) } else { list.pop().unwrap_or(Json::Null) };
                let moved = json_text(&item);
                let insert = |l: &mut Vec<Json>, v: Json| if to_left { l.insert(0, v) } else { l.push(v) };
                match target {
                    // The same list: a rotation.
                    None => {
                        insert(&mut list, item);
                        self.store_list(&src, list, exp)?;
                    }
                    Some((mut other, other_exp)) => {
                        insert(&mut other, item);
                        self.check_size(&dst, "list", other.len())?;
                        self.store_list(&src, list, exp)?;
                        self.store_list(&dst, other, other_exp)?;
                    }
                }
                Ok(Reply::bulk(moved))
            }
            _ => {
                // LMPOP numkeys key [key ...] LEFT|RIGHT [COUNT n]
                arity(args, 4)?;
                let n = int(&args[1])?;
                if n <= 0 {
                    return Err(Reply::err("numkeys should be greater than 0"));
                }
                let n = n as usize;
                if args.len() < 3 + n {
                    return Err(syntax());
                }
                let left = side(&args[2 + n])?;
                let count = match &args[3 + n..] {
                    [] => 1,
                    [c, v] if upper(c) == "COUNT" => match int(v)? {
                        c if c > 0 => c as usize,
                        _ => return Err(Reply::err("count should be greater than 0")),
                    },
                    _ => return Err(syntax()),
                };
                for a in &args[2..2 + n] {
                    let k = key(a)?;
                    let Some((mut list, exp)) = self.list(&k)? else { continue };
                    let take = count.min(list.len());
                    let popped: Vec<Json> = if left {
                        list.drain(..take).collect()
                    } else {
                        let at = list.len() - take;
                        list.drain(at..).rev().collect()
                    };
                    self.store_list(&k, list, exp)?;
                    return Ok(Reply::array(vec![
                        Reply::bulk(k),
                        Reply::array(popped.iter().map(|v| Reply::bulk(json_text(v))).collect()),
                    ]));
                }
                Ok(Reply::Array(None))
            }
        }
    }

    /// `LPOS key element [RANK r] [COUNT n] [MAXLEN m]`.
    fn lpos(&mut self, args: &[Vec<u8>]) -> Res {
        arity(args, 3)?;
        let k = key(&args[1])?;
        let (mut rank, mut count, mut maxlen) = (1i64, None, 0usize);
        for pair in args[3..].chunks(2) {
            let [opt, v] = pair else { return Err(syntax()) };
            let v = int(v)?;
            match upper(opt).as_str() {
                "RANK" if v == 0 => {
                    return Err(Reply::err(
                        "RANK can't be zero: use 1 to start from the first match, 2 from the second ... \
                         or use negative to start from the end of the list",
                    ))
                }
                "RANK" if v == i64::MIN => return Err(Reply::err("value is out of range")),
                "RANK" => rank = v,
                "COUNT" if v < 0 => return Err(Reply::err("COUNT can't be negative")),
                "COUNT" => count = Some(v as usize),
                "MAXLEN" if v < 0 => return Err(Reply::err("MAXLEN can't be negative")),
                "MAXLEN" => maxlen = v as usize,
                _ => return Err(syntax()),
            }
        }
        let list = self.list(&k)?.map(|(l, _)| l).unwrap_or_default();
        let len = list.len();
        let scanned = if maxlen == 0 { len } else { maxlen.min(len) };
        // Positions in the order of the search: from the head, or from the tail.
        let order: Box<dyn Iterator<Item = usize>> =
            if rank > 0 { Box::new(0..scanned) } else { Box::new((len - scanned..len).rev()) };
        let wanted = match count {
            Some(0) => usize::MAX,
            Some(c) => c,
            None => 1,
        };
        let found: Vec<usize> = order
            .filter(|&i| json_text(&list[i]) == args[2])
            .skip(rank.unsigned_abs() as usize - 1)
            .take(wanted)
            .collect();
        Ok(match count {
            None => found.first().map_or(Reply::nil(), |&i| Reply::Int(i as i64)),
            Some(_) => Reply::array(found.into_iter().map(|i| Reply::Int(i as i64)).collect()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::span;

    #[test]
    fn spans_follow_redis() {
        assert_eq!(span(0, -1, 5), Some((0, 4)));
        assert_eq!(span(-2, 100, 5), Some((3, 4)));
        assert_eq!(span(3, 1, 5), None);
        assert_eq!(span(5, 9, 5), None);
        assert_eq!(span(-100, 0, 5), Some((0, 0)));
        assert_eq!(span(0, -1, 0), None);
    }
}
