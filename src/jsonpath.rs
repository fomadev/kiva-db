// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Paths into JSON documents and the changes made through them.
//!
//! A path is a list of segments: `.name`, `.address.city`, `.tags[0]`,
//! `.tags.0`, `.tags[-1]` (the last element), `["a.b"]` (a field whose name
//! contains a dot, a space or a bracket). Every operation works on the parsed
//! tree in place: changing one field touches only that field, whatever the
//! size of the document.

use serde_json::Value as Json;

#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Key(String),
    /// An array position; negative counts from the end (`-1` is the last).
    Idx(i64),
}

pub fn parse_path(path: &str) -> Result<Vec<Seg>, String> {
    let p = path.trim();
    let bad = |why: &str| format!("invalid path '{path}': {why}");
    let chars: Vec<char> = p.chars().collect();
    let mut segs = Vec::new();
    let mut i = 0;
    let mut need_name = false;
    // A leading dot is optional: `.name` and `name` are the same path.
    if chars.first() == Some(&'.') {
        if chars.len() == 1 {
            return Ok(segs);
        }
        i = 1;
        need_name = true;
    }
    while i < chars.len() {
        match chars[i] {
            '[' => {
                let close = chars[i..].iter().position(|&c| c == ']').map(|n| n + i);
                if let Some(q @ ('"' | '\'')) = chars.get(i + 1).copied() {
                    let end = chars[i + 2..]
                        .iter()
                        .position(|&c| c == q)
                        .map(|n| n + i + 2)
                        .ok_or_else(|| bad("unclosed quote"))?;
                    if chars.get(end + 1) != Some(&']') {
                        return Err(bad("missing ']'"));
                    }
                    segs.push(Seg::Key(chars[i + 2..end].iter().collect()));
                    i = end + 2;
                } else {
                    let close = close.ok_or_else(|| bad("missing ']'"))?;
                    let text: String = chars[i + 1..close].iter().collect();
                    let n = text.trim().parse::<i64>().map_err(|_| bad("an array index must be an integer"))?;
                    segs.push(Seg::Idx(n));
                    i = close + 1;
                }
                need_name = false;
            }
            '.' => {
                if need_name || i + 1 == chars.len() {
                    return Err(bad("empty segment"));
                }
                need_name = true;
                i += 1;
            }
            ']' => return Err(bad("unexpected ']'")),
            _ => {
                if !segs.is_empty() && !need_name {
                    return Err(bad("expected '.' or '[' between segments"));
                }
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '[' && chars[i] != ']' {
                    i += 1;
                }
                segs.push(Seg::Key(chars[start..i].iter().collect()));
                need_name = false;
            }
        }
    }
    if need_name {
        return Err(bad("empty segment"));
    }
    Ok(segs)
}

/// The position `seg` designates in an array of `len` elements. With
/// `append`, the position just after the last one is accepted too.
fn position(seg: &Seg, len: usize, append: bool) -> Option<usize> {
    let n = match seg {
        Seg::Idx(i) => *i,
        Seg::Key(k) => k.parse().ok()?,
    };
    let len = len as i64;
    let at = if n < 0 { len + n } else { n };
    let max = if append && n >= 0 { len } else { len - 1 };
    (0..=max).contains(&at).then_some(at as usize)
}

pub fn get<'a>(doc: &'a Json, path: &[Seg]) -> Option<&'a Json> {
    let mut cur = doc;
    for seg in path {
        cur = match cur {
            Json::Object(m) => match seg {
                Seg::Key(k) => m.get(k)?,
                Seg::Idx(_) => return None,
            },
            Json::Array(a) => &a[position(seg, a.len(), false)?],
            _ => return None,
        };
    }
    Some(cur)
}

/// The parent of the last segment, creating missing objects on the way.
fn parent_mut<'a>(doc: &'a mut Json, parents: &[Seg]) -> Result<&'a mut Json, String> {
    let mut cur = doc;
    for seg in parents {
        cur = match cur {
            Json::Object(m) => match seg {
                Seg::Key(k) => m.entry(k.clone()).or_insert_with(|| Json::Object(Default::default())),
                Seg::Idx(_) => return Err("cannot index an object with [n]".into()),
            },
            Json::Array(a) => {
                let len = a.len();
                let i = position(seg, len, false).ok_or_else(|| out_of_range(seg, len))?;
                &mut a[i]
            }
            _ => return Err("path traverses a scalar value".into()),
        };
    }
    Ok(cur)
}

fn out_of_range(seg: &Seg, len: usize) -> String {
    match seg {
        Seg::Idx(i) => format!("array index {i} is out of range (length {len})"),
        Seg::Key(k) => format!("array elements are addressed by index, not by '{k}'"),
    }
}

/// Sets the value at `path`, creating intermediate objects as needed.
pub fn set(doc: &mut Json, path: &[Seg], value: Json) -> Result<(), String> {
    let Some((last, parents)) = path.split_last() else {
        *doc = value;
        return Ok(());
    };
    match parent_mut(doc, parents)? {
        Json::Object(m) => match last {
            Seg::Key(k) => {
                m.insert(k.clone(), value);
                Ok(())
            }
            Seg::Idx(_) => Err("cannot index an object with [n]".into()),
        },
        Json::Array(a) => {
            let i = position(last, a.len(), true).ok_or_else(|| out_of_range(last, a.len()))?;
            if i == a.len() {
                a.push(value);
            } else {
                a[i] = value;
            }
            Ok(())
        }
        _ => Err("path traverses a scalar value".into()),
    }
}

/// Removes the value at `path`. Returns whether something was removed.
pub fn delete(doc: &mut Json, path: &[Seg]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    let Some(parent) = get_mut(doc, parents) else {
        return false;
    };
    match parent {
        Json::Object(m) => match last {
            Seg::Key(k) => m.shift_remove(k).is_some(),
            Seg::Idx(_) => false,
        },
        Json::Array(a) => match position(last, a.len(), false) {
            Some(i) => {
                a.remove(i);
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// The value at `path`, for changing it in place. Creates nothing.
pub fn get_mut<'a>(doc: &'a mut Json, path: &[Seg]) -> Option<&'a mut Json> {
    let mut cur = doc;
    for seg in path {
        cur = match cur {
            Json::Object(m) => match seg {
                Seg::Key(k) => m.get_mut(k)?,
                Seg::Idx(_) => return None,
            },
            Json::Array(a) => {
                let i = position(seg, a.len(), false)?;
                &mut a[i]
            }
            _ => return None,
        };
    }
    Some(cur)
}

/// Adds `values` at the end of the array at `path`, creating the array if
/// the field is missing. Returns the new length.
pub fn append(doc: &mut Json, path: &[Seg], values: Vec<Json>) -> Result<usize, String> {
    if get(doc, path).is_none() {
        set(doc, path, Json::Array(Vec::new()))?;
    }
    match get_mut(doc, path) {
        Some(Json::Array(a)) => {
            a.extend(values);
            Ok(a.len())
        }
        _ => Err("the value at this path is not an array".into()),
    }
}

/// Inserts `value` before the element `path` designates (`[0]` first,
/// `[-1]` before the last, the array's length to add at the end). Returns
/// the new length.
pub fn insert(doc: &mut Json, path: &[Seg], value: Json) -> Result<usize, String> {
    let Some((last, parents)) = path.split_last() else {
        return Err("insert needs a path ending with an array position, such as .tags[0]".into());
    };
    match get_mut(doc, parents) {
        Some(Json::Array(a)) => {
            let i = position(last, a.len(), true).ok_or_else(|| out_of_range(last, a.len()))?;
            a.insert(i, value);
            Ok(a.len())
        }
        Some(_) => Err("insert works on arrays; the parent of this path is not an array".into()),
        None => Err("the array at this path does not exist".into()),
    }
}

/// Adds `by` to the number at `path` (a missing field counts as 0) and
/// returns the result. Integers stay integers while they fit in 64 bits.
pub fn incr(doc: &mut Json, path: &[Seg], by: &serde_json::Number) -> Result<Json, String> {
    let next = match get(doc, path) {
        None => Json::Number(by.clone()),
        Some(Json::Number(cur)) => match (cur.as_i64(), by.as_i64()) {
            (Some(a), Some(b)) => Json::from(a.checked_add(b).ok_or("the increment would overflow")?),
            _ => {
                let f = cur.as_f64().unwrap_or(0.0) + by.as_f64().unwrap_or(0.0);
                Json::Number(serde_json::Number::from_f64(f).ok_or("the increment would produce NaN or Infinity")?)
            }
        },
        Some(_) => return Err("the value at this path is not a number".into()),
    };
    set(doc, path, next.clone())?;
    Ok(next)
}

/// Number of elements of an array, fields of an object, or characters of a string.
pub fn len(v: &Json) -> Option<usize> {
    match v {
        Json::Array(a) => Some(a.len()),
        Json::Object(m) => Some(m.len()),
        Json::String(s) => Some(s.chars().count()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(s: &str) -> Vec<Seg> {
        parse_path(s).unwrap()
    }

    #[test]
    fn parses_every_form() {
        assert_eq!(p(".a.b"), vec![Seg::Key("a".into()), Seg::Key("b".into())]);
        assert_eq!(p("tags[0]"), vec![Seg::Key("tags".into()), Seg::Idx(0)]);
        assert_eq!(p(".m[1][-1]"), vec![Seg::Key("m".into()), Seg::Idx(1), Seg::Idx(-1)]);
        assert_eq!(p(r#"["a.b"].c"#), vec![Seg::Key("a.b".into()), Seg::Key("c".into())]);
        assert_eq!(p(".x['with space']"), vec![Seg::Key("x".into()), Seg::Key("with space".into())]);
        assert!(p(".").is_empty());
        for bad in ["a..b", "..a", "a.", "a[1", "a[x]", "a]", "[\"a]", "a[0]b"] {
            assert!(parse_path(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn negative_positions_count_from_the_end() {
        let mut d = json!({"t": [1, 2, 3]});
        assert_eq!(get(&d, &p(".t[-1]")), Some(&json!(3)));
        assert_eq!(get(&d, &p(".t[-4]")), None);
        set(&mut d, &p(".t[-2]"), json!(20)).unwrap();
        assert!(set(&mut d, &p(".t[-4]"), json!(0)).is_err());
        assert!(delete(&mut d, &p(".t[-1]")));
        assert_eq!(d, json!({"t": [1, 20]}));
    }

    #[test]
    fn array_and_number_changes() {
        let mut d = json!({"n": 1});
        assert_eq!(append(&mut d, &p(".tags"), vec![json!("a"), json!("b")]).unwrap(), 2);
        assert_eq!(insert(&mut d, &p(".tags[0]"), json!("z")).unwrap(), 3);
        assert_eq!(insert(&mut d, &p(".tags[3]"), json!("end")).unwrap(), 4);
        assert_eq!(d["tags"], json!(["z", "a", "b", "end"]));
        assert!(append(&mut d, &p(".n"), vec![json!(1)]).is_err());
        assert_eq!(incr(&mut d, &p(".n"), &5.into()).unwrap(), json!(6));
        assert_eq!(incr(&mut d, &p(".stats.views"), &1.into()).unwrap(), json!(1));
        let half = serde_json::Number::from_f64(0.5).unwrap();
        assert_eq!(incr(&mut d, &p(".n"), &half).unwrap(), json!(6.5));
        assert!(incr(&mut d, &p(".tags"), &1.into()).is_err());
        let mut big = json!({"n": i64::MAX});
        assert!(incr(&mut big, &p(".n"), &1.into()).is_err());
    }
}
