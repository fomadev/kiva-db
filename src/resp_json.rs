// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! JSON documents from a Redis client, with the commands of RedisJSON
//! (`JSON.SET`, `JSON.GET`, `JSON.NUMINCRBY`...).
//!
//! A document written this way is an ordinary KivaDB document: the shell
//! reads and queries it (`get where`), secondary and vector indexes cover it,
//! and `KIVA.SIMILAR` finds it. The other way round, every document written
//! from the shell, the REST API or a driver can be read and changed here.
//!
//! Paths are those of RedisJSON that name one value: `$` or `.` for the
//! whole document, `$.address.city`, `.tags[0]`, `$.tags[-1]`. A path
//! starting with `$` answers a list of matches, as in RedisJSON 2; the
//! legacy form (`.a.b`) answers the value itself. Wildcards, `..` and
//! filters, which name several values, are refused.

use super::*;
use crate::jsonpath::{self, Seg};

/// A parsed path, and whether it was written in the `$` form.
struct Path {
    segs: Vec<Seg>,
    dollar: bool,
}

impl Path {
    fn root(&self) -> bool {
        self.segs.is_empty()
    }
}

fn path(arg: &[u8]) -> std::result::Result<Path, Reply> {
    let p = text(arg)?;
    let p = p.trim();
    let (dollar, rest) = match p.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, p),
    };
    if rest.contains('*') || rest.contains("..") || rest.contains("?(") || rest.contains(':') {
        return Err(Reply::err(format!(
            "unsupported JSON path '{p}': KivaDB follows paths that name one value ($.a.b, .tags[0], $.tags[-1]); \
             wildcards, '..', slices and filters are not supported"
        )));
    }
    if rest.is_empty() || rest == "." {
        return Ok(Path { segs: Vec::new(), dollar });
    }
    let segs = jsonpath::parse_path(rest).map_err(Reply::err)?;
    Ok(Path { segs, dollar })
}

/// The path argument at `i`, or the root when it is absent.
fn path_at(args: &[Vec<u8>], i: usize) -> std::result::Result<Path, Reply> {
    match args.get(i) {
        Some(a) => path(a),
        None => Ok(Path { segs: Vec::new(), dollar: false }),
    }
}

fn json_arg(arg: &[u8]) -> std::result::Result<Json, Reply> {
    serde_json::from_slice(arg).map_err(|e| Reply::err(format!("expected a JSON value: {e}")))
}

/// A KivaDB value as JSON; strings stay strings.
fn to_json(v: Value) -> Json {
    match v {
        Value::Doc(j) => j,
        Value::Str(s) => Json::String(s),
        Value::Num(n) => serde_json::from_str(&n).unwrap_or(Json::String(n)),
        Value::Bool(b) => Json::Bool(b),
        Value::Null => Json::Null,
        Value::Bytes(b) => Json::String(String::from_utf8_lossy(&b).into_owned()),
    }
}

/// A whole document as the KivaDB value stored under its key.
fn to_value(j: Json) -> Value {
    match j {
        Json::Object(_) | Json::Array(_) => Value::Doc(j),
        Json::String(s) => Value::Str(s),
        Json::Number(n) => Value::Num(n.to_string()),
        Json::Bool(b) => Value::Bool(b),
        Json::Null => Value::Null,
    }
}

/// A JSON number as a RESP3 number.
fn number(n: &Json) -> Reply {
    match n.as_i64() {
        Some(i) => Reply::Int(i),
        None => Reply::Double(n.as_f64().unwrap_or(0.0)),
    }
}

fn type_name(j: &Json) -> &'static str {
    match j {
        Json::Object(_) => "object",
        Json::Array(_) => "array",
        Json::String(_) => "string",
        Json::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Json::Number(_) => "number",
        Json::Bool(_) => "boolean",
        Json::Null => "null",
    }
}

fn get<'a>(doc: &'a Json, p: &Path) -> Option<&'a Json> {
    if p.root() {
        Some(doc)
    } else {
        jsonpath::get(doc, &p.segs)
    }
}

impl Conn {
    /// The document at `k` and its expiry.
    fn json_doc(&mut self, k: &str) -> std::result::Result<Option<(Json, i64)>, Reply> {
        Ok(self.get(k)?.map(|(v, m)| (to_json(v), m.expires_at)))
    }

    /// Answers one value per path: a list of the matches for a `$` path
    /// (empty when nothing is there), the value or nil for a legacy one.
    fn per_path(&self, p: &Path, found: Option<Reply>) -> Reply {
        match (p.dollar, found) {
            (true, Some(r)) => Reply::array(vec![r]),
            (true, None) => Reply::array(Vec::new()),
            (false, Some(r)) => r,
            (false, None) => Reply::nil(),
        }
    }

    /// Runs a `JSON.*` command; `None` when `name` is not one.
    pub(super) fn run_json(&mut self, name: &str, args: &[Vec<u8>]) -> Option<Res> {
        if !name.starts_with("JSON.") {
            return None;
        }
        Some(self.json_command(name, args))
    }

    fn json_command(&mut self, name: &str, args: &[Vec<u8>]) -> Res {
        match name {
            "JSON.SET" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let p = path(&args[2])?;
                let value = json_arg(&args[3])?;
                let (nx, xx) = match args.get(4).map(|a| text(a).map(|t| t.to_ascii_uppercase())).transpose()? {
                    None => (false, false),
                    Some(o) if o == "NX" && args.len() == 5 => (true, false),
                    Some(o) if o == "XX" && args.len() == 5 => (false, true),
                    _ => return Err(Reply::err("syntax error")),
                };
                let current = self.json_doc(&k)?;
                let exists = current.as_ref().is_some_and(|(d, _)| get(d, &p).is_some());
                if (nx && exists) || (xx && !exists) {
                    return Ok(Reply::nil());
                }
                if p.root() {
                    // A new document; an existing key keeps its expiry, as JSON.SET does in Redis.
                    let exp = current.map_or(0, |(_, e)| e);
                    self.put(&k, to_value(value), exp)?;
                    return Ok(Reply::ok());
                }
                let Some((mut doc, exp)) = current else {
                    return Err(Reply::err("new objects must be created at the root"));
                };
                jsonpath::set(&mut doc, &p.segs, value).map_err(Reply::err)?;
                self.put(&k, to_value(doc), exp)?;
                Ok(Reply::ok())
            }
            "JSON.GET" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                // INDENT, NEWLINE and SPACE only change the layout: the answer stays compact.
                let mut paths = Vec::new();
                let mut i = 2;
                while i < args.len() {
                    match text(&args[i])?.to_ascii_uppercase().as_str() {
                        "INDENT" | "NEWLINE" | "SPACE" => i += 2,
                        _ => {
                            paths.push((text(&args[i])?, path(&args[i])?));
                            i += 1;
                        }
                    }
                }
                let Some((doc, _)) = self.json_doc(&k)? else { return Ok(Reply::nil()) };
                let shown = |p: &Path| -> Option<Json> {
                    let v = get(&doc, p).cloned();
                    if p.dollar {
                        Some(Json::Array(v.into_iter().collect()))
                    } else {
                        v
                    }
                };
                Ok(match paths.as_slice() {
                    [] => Reply::bulk(doc.to_string()),
                    [(_, p)] => match shown(p) {
                        Some(v) => Reply::bulk(v.to_string()),
                        None if p.dollar => Reply::bulk("[]"),
                        None => return Err(Reply::err(format!("Path '{}' does not exist", text(&args[2])?))),
                    },
                    many => {
                        let mut out = Map::new();
                        for (raw, p) in many {
                            out.insert(raw.clone(), shown(p).unwrap_or(Json::Null));
                        }
                        Reply::bulk(Json::Object(out).to_string())
                    }
                })
            }
            "JSON.MGET" => {
                arity(args, 3)?;
                let p = path(&args[args.len() - 1])?;
                let mut items = Vec::new();
                for a in &args[1..args.len() - 1] {
                    let k = key(a)?;
                    let doc = self.json_doc(&k)?.map(|(d, _)| d);
                    items.push(match doc.as_ref().and_then(|d| get(d, &p)) {
                        Some(v) if p.dollar => Reply::bulk(Json::Array(vec![v.clone()]).to_string()),
                        Some(v) => Reply::bulk(v.to_string()),
                        None => Reply::nil(),
                    });
                }
                Ok(Reply::array(items))
            }
            "JSON.DEL" | "JSON.FORGET" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let p = path_at(args, 2)?;
                let Some((mut doc, exp)) = self.json_doc(&k)? else { return Ok(Reply::Int(0)) };
                if p.root() {
                    self.sess.kv_del(&k)?;
                    return Ok(Reply::Int(1));
                }
                if !jsonpath::delete(&mut doc, &p.segs) {
                    return Ok(Reply::Int(0));
                }
                self.put(&k, to_value(doc), exp)?;
                Ok(Reply::Int(1))
            }
            "JSON.TYPE" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let p = path_at(args, 2)?;
                let doc = self.json_doc(&k)?.map(|(d, _)| d);
                let found = doc.as_ref().and_then(|d| get(d, &p)).map(|v| Reply::Simple(type_name(v).into()));
                let reply = self.per_path(&p, found);
                // In RESP3, RedisJSON wraps the answer in one more array.
                Ok(if self.resp3 { Reply::array(vec![reply]) } else { reply })
            }
            "JSON.NUMINCRBY" | "JSON.NUMMULTBY" => {
                arity(args, 4)?;
                let k = key(&args[1])?;
                let p = path(&args[2])?;
                let by = match json_arg(&args[3])? {
                    Json::Number(n) => n,
                    _ => return Err(Reply::err("expected a number")),
                };
                let Some((mut doc, exp)) = self.json_doc(&k)? else {
                    return Err(Reply::err("could not perform this operation on a key that doesn't exist"));
                };
                let next = if name == "JSON.NUMINCRBY" {
                    if get(&doc, &p).is_none() {
                        return missing(&p);
                    }
                    jsonpath::incr(&mut doc, &p.segs, &by).map_err(Reply::err)?
                } else {
                    let slot = if p.root() { Some(&mut doc) } else { jsonpath::get_mut(&mut doc, &p.segs) };
                    let Some(slot) = slot else { return missing(&p) };
                    let (Some(a), Some(b)) = (slot.as_f64(), by.as_f64()) else {
                        return Err(Reply::err("the value at this path is not a number"));
                    };
                    let product = match (slot.as_i64(), by.as_i64()) {
                        (Some(x), Some(y)) => x.checked_mul(y).map(Json::from),
                        _ => None,
                    }
                    .or_else(|| serde_json::Number::from_f64(a * b).map(Json::Number))
                    .ok_or_else(|| Reply::err("result is not a finite number"))?;
                    *slot = product.clone();
                    product
                };
                self.put(&k, to_value(doc), exp)?;
                // RESP3 answers numbers; RESP2, their JSON text.
                Ok(match (self.resp3, p.dollar) {
                    (true, true) => Reply::array(vec![number(&next)]),
                    (true, false) => number(&next),
                    (false, true) => Reply::bulk(Json::Array(vec![next]).to_string()),
                    (false, false) => Reply::bulk(next.to_string()),
                })
            }
            "JSON.STRLEN" | "JSON.ARRLEN" | "JSON.OBJLEN" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let p = path_at(args, 2)?;
                let Some((doc, _)) = self.json_doc(&k)? else { return Ok(Reply::nil()) };
                let found = get(&doc, &p).map(|v| match (name, v) {
                    ("JSON.STRLEN", Json::String(s)) => Reply::Int(s.chars().count() as i64),
                    ("JSON.ARRLEN", Json::Array(a)) => Reply::Int(a.len() as i64),
                    ("JSON.OBJLEN", Json::Object(o)) => Reply::Int(o.len() as i64),
                    _ => Reply::nil(),
                });
                Ok(self.per_path(&p, found))
            }
            "JSON.OBJKEYS" => {
                arity(args, 2)?;
                let k = key(&args[1])?;
                let p = path_at(args, 2)?;
                let Some((doc, _)) = self.json_doc(&k)? else { return Ok(Reply::nil()) };
                let found = get(&doc, &p).map(|v| match v {
                    Json::Object(o) => Reply::array(o.keys().map(|f| Reply::bulk(f.clone())).collect()),
                    _ => Reply::nil(),
                });
                Ok(self.per_path(&p, found))
            }
            "JSON.ARRAPPEND" | "JSON.ARRINSERT" => {
                let insert = name == "JSON.ARRINSERT";
                arity(args, if insert { 5 } else { 4 })?;
                let k = key(&args[1])?;
                let p = path(&args[2])?;
                let first = if insert { 4 } else { 3 };
                let values = args[first..].iter().map(|a| json_arg(a)).collect::<std::result::Result<Vec<_>, _>>()?;
                let Some((mut doc, exp)) = self.json_doc(&k)? else {
                    return Err(Reply::err("could not perform this operation on a key that doesn't exist"));
                };
                let Some(Json::Array(arr)) =
                    (if p.root() { Some(&mut doc) } else { jsonpath::get_mut(&mut doc, &p.segs) })
                else {
                    return Ok(self.per_path(&p, Some(Reply::nil())));
                };
                if insert {
                    let at = int(&args[3])?;
                    let len = arr.len() as i64;
                    let at = if at < 0 { len + at } else { at };
                    if !(0..=len).contains(&at) {
                        return Err(Reply::err("index out of bounds"));
                    }
                    for (i, v) in values.into_iter().enumerate() {
                        arr.insert(at as usize + i, v);
                    }
                } else {
                    arr.extend(values);
                }
                let len = arr.len() as i64;
                self.put(&k, to_value(doc), exp)?;
                Ok(self.per_path(&p, Some(Reply::Int(len))))
            }
            other => Err(commands::unknown(&[other.as_bytes().to_vec()])),
        }
    }
}

/// A change at a path that matches nothing: an empty list for a `$` path,
/// an error for a legacy one, as in RedisJSON.
fn missing(p: &Path) -> Res {
    if p.dollar {
        Ok(Reply::array(Vec::new()))
    } else {
        Err(Reply::err("Path does not exist"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_of_redisjson() {
        for p in ["$", ".", ""] {
            assert!(path(p.as_bytes()).ok().unwrap().root(), "{p}");
        }
        let p = path(b"$.address.city").ok().unwrap();
        assert!(p.dollar);
        assert_eq!(p.segs, vec![Seg::Key("address".into()), Seg::Key("city".into())]);
        let p = path(b".tags[-1]").ok().unwrap();
        assert!(!p.dollar);
        assert_eq!(p.segs, vec![Seg::Key("tags".into()), Seg::Idx(-1)]);
        assert_eq!(path(b"name").ok().unwrap().segs, vec![Seg::Key("name".into())]);
        for bad in ["$..price", "$.items[*]", "$.a[?(@.x>1)]", "$.a[0:2]"] {
            assert!(path(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
