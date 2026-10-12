// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See LICENSE file in the project root for full license information.

//! Dotted paths into JSON documents: `.name`, `.address.city`, `.tags[0]`, `.tags.0`.

use serde_json::Value as Json;

#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Key(String),
    Idx(usize),
}

pub fn parse_path(path: &str) -> Result<Vec<Seg>, String> {
    let p = path.trim();
    let p = p.strip_prefix('.').unwrap_or(p);
    let mut segs = Vec::new();
    if p.is_empty() {
        return Ok(segs);
    }
    for part in p.split('.') {
        let (name, mut rest) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (part, ""),
        };
        if name.is_empty() && rest.is_empty() {
            return Err(format!("invalid path '{path}': empty segment"));
        }
        if !name.is_empty() {
            segs.push(Seg::Key(name.to_string()));
        }
        while !rest.is_empty() {
            let close = rest.find(']').ok_or_else(|| format!("invalid path '{path}': missing ']'"))?;
            if !rest.starts_with('[') {
                return Err(format!("invalid path '{path}'"));
            }
            let idx: usize = rest[1..close]
                .trim()
                .parse()
                .map_err(|_| format!("invalid path '{path}': array index must be a non-negative integer"))?;
            segs.push(Seg::Idx(idx));
            rest = &rest[close + 1..];
        }
    }
    Ok(segs)
}

fn as_index(seg: &Seg) -> Option<usize> {
    match seg {
        Seg::Idx(i) => Some(*i),
        Seg::Key(k) => k.parse().ok(),
    }
}

pub fn get<'a>(doc: &'a Json, path: &[Seg]) -> Option<&'a Json> {
    let mut cur = doc;
    for seg in path {
        cur = match cur {
            Json::Object(m) => match seg {
                Seg::Key(k) => m.get(k)?,
                Seg::Idx(_) => return None,
            },
            Json::Array(a) => a.get(as_index(seg)?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Sets the value at `path`, creating intermediate objects as needed.
pub fn set(doc: &mut Json, path: &[Seg], value: Json) -> Result<(), String> {
    let Some((last, parents)) = path.split_last() else {
        *doc = value;
        return Ok(());
    };
    let mut cur = doc;
    for seg in parents {
        cur = match cur {
            Json::Object(m) => match seg {
                Seg::Key(k) => m.entry(k.clone()).or_insert_with(|| Json::Object(Default::default())),
                Seg::Idx(_) => return Err("cannot index an object with [n]".into()),
            },
            Json::Array(a) => {
                let i = as_index(seg).ok_or("array elements are addressed by index")?;
                a.get_mut(i).ok_or_else(|| format!("array index {i} is out of range"))?
            }
            _ => return Err("path traverses a scalar value".into()),
        };
    }
    match cur {
        Json::Object(m) => match last {
            Seg::Key(k) => {
                m.insert(k.clone(), value);
                Ok(())
            }
            Seg::Idx(_) => Err("cannot index an object with [n]".into()),
        },
        Json::Array(a) => {
            let i = as_index(last).ok_or("array elements are addressed by index")?;
            if i < a.len() {
                a[i] = value;
                Ok(())
            } else if i == a.len() {
                a.push(value);
                Ok(())
            } else {
                Err(format!("array index {i} is out of range (length {})", a.len()))
            }
        }
        _ => Err("path traverses a scalar value".into()),
    }
}

/// Removes the value at `path`. Returns whether something was removed.
pub fn delete(doc: &mut Json, path: &[Seg]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    let mut cur = doc;
    for seg in parents {
        cur = match cur {
            Json::Object(m) => match seg {
                Seg::Key(k) => match m.get_mut(k) {
                    Some(v) => v,
                    None => return false,
                },
                Seg::Idx(_) => return false,
            },
            Json::Array(a) => match as_index(seg).and_then(|i| a.get_mut(i)) {
                Some(v) => v,
                None => return false,
            },
            _ => return false,
        };
    }
    match cur {
        Json::Object(m) => match last {
            Seg::Key(k) => m.shift_remove(k).is_some(),
            Seg::Idx(_) => false,
        },
        Json::Array(a) => match as_index(last) {
            Some(i) if i < a.len() => {
                a.remove(i);
                true
            }
            _ => false,
        },
        _ => false,
    }
}
