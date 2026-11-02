// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The changes of [`crate::jsonpath`], made on a document of typed values
//! ([`Doc`]): the same paths, the same rules, and every value the change
//! does not touch keeps its type.

use crate::doc::{self, Doc};
use crate::jsonpath::{out_of_range, position, Seg};

pub fn get<'a>(doc: &'a Doc, path: &[Seg]) -> Option<&'a Doc> {
    let mut cur = doc;
    for seg in path {
        cur = match cur {
            Doc::Object(m) => match seg {
                Seg::Key(k) => m.get(k)?,
                Seg::Idx(_) => return None,
            },
            Doc::Array(a) => &a[position(seg, a.len(), false)?],
            _ => return None,
        };
    }
    Some(cur)
}

/// The value at `path`, for changing it in place. Creates nothing.
pub fn get_mut<'a>(doc: &'a mut Doc, path: &[Seg]) -> Option<&'a mut Doc> {
    let mut cur = doc;
    for seg in path {
        cur = match cur {
            Doc::Object(m) => match seg {
                Seg::Key(k) => m.get_mut(k)?,
                Seg::Idx(_) => return None,
            },
            Doc::Array(a) => {
                let i = position(seg, a.len(), false)?;
                &mut a[i]
            }
            _ => return None,
        };
    }
    Some(cur)
}

/// The parent of the last segment, creating missing objects on the way.
fn parent_mut<'a>(doc: &'a mut Doc, parents: &[Seg]) -> Result<&'a mut Doc, String> {
    let mut cur = doc;
    for seg in parents {
        cur = match cur {
            Doc::Object(m) => match seg {
                Seg::Key(k) => m.entry(k.clone()).or_insert_with(|| Doc::Object(Default::default())),
                Seg::Idx(_) => return Err("cannot index an object with [n]".into()),
            },
            Doc::Array(a) => {
                let len = a.len();
                let i = position(seg, len, false).ok_or_else(|| out_of_range(seg, len))?;
                &mut a[i]
            }
            _ => return Err("path traverses a scalar value".into()),
        };
    }
    Ok(cur)
}

/// Sets the value at `path`, creating intermediate objects as needed.
pub fn set(doc: &mut Doc, path: &[Seg], value: Doc) -> Result<(), String> {
    let Some((last, parents)) = path.split_last() else {
        *doc = value;
        return Ok(());
    };
    match parent_mut(doc, parents)? {
        Doc::Object(m) => match last {
            Seg::Key(k) => {
                m.insert(k.clone(), value);
                Ok(())
            }
            Seg::Idx(_) => Err("cannot index an object with [n]".into()),
        },
        Doc::Array(a) => {
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
pub fn delete(doc: &mut Doc, path: &[Seg]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    match get_mut(doc, parents) {
        Some(Doc::Object(m)) => match last {
            Seg::Key(k) => m.shift_remove(k).is_some(),
            Seg::Idx(_) => false,
        },
        Some(Doc::Array(a)) => match position(last, a.len(), false) {
            Some(i) => {
                a.remove(i);
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Adds `values` at the end of the array at `path`, creating the array if
/// the field is missing. Returns the new length.
pub fn append(doc: &mut Doc, path: &[Seg], values: Vec<Doc>) -> Result<usize, String> {
    if get(doc, path).is_none() {
        set(doc, path, Doc::Array(Vec::new()))?;
    }
    match get_mut(doc, path) {
        Some(Doc::Array(a)) => {
            a.extend(values);
            Ok(a.len())
        }
        _ => Err("the value at this path is not an array".into()),
    }
}

/// Inserts `value` before the element `path` designates. Returns the new length.
pub fn insert(doc: &mut Doc, path: &[Seg], value: Doc) -> Result<usize, String> {
    let Some((last, parents)) = path.split_last() else {
        return Err("insert needs a path ending with an array position, such as .tags[0]".into());
    };
    match get_mut(doc, parents) {
        Some(Doc::Array(a)) => {
            let i = position(last, a.len(), true).ok_or_else(|| out_of_range(last, a.len()))?;
            a.insert(i, value);
            Ok(a.len())
        }
        Some(_) => Err("insert works on arrays; the parent of this path is not an array".into()),
        None => Err("the array at this path does not exist".into()),
    }
}

/// Adds `by` to the number at `path` (a missing field counts as 0) and
/// returns the result, by the rules of [`doc::add`]: an `int` stays an
/// `int`, a `decimal` a `decimal`, and a decimal is never mixed with a float.
pub fn incr(doc: &mut Doc, path: &[Seg], by: &Doc) -> Result<Doc, String> {
    let next = match get(doc, path) {
        None => by.clone(),
        Some(cur @ (Doc::Int(_) | Doc::Float(_) | Doc::Decimal(_))) => {
            doc::add(cur, by).map_err(|e| format!("the increment cannot be made: {e}"))?
        }
        Some(_) => return Err("the value at this path is not a number".into()),
    };
    set(doc, path, next.clone())?;
    Ok(next)
}

/// The number an increment written as `text` (the JSON number `n`) brings
/// to `target`: read from its text as a decimal when the value it changes is
/// a decimal (`-100.25` is that decimal, not the nearest float), else as
/// the JSON number it is.
pub fn increment(target: Option<&Doc>, n: &serde_json::Number, text: &str) -> Doc {
    match (target, doc::parse_decimal(text.trim())) {
        (Some(Doc::Decimal(_)), Ok(d)) => Doc::Decimal(d),
        _ => doc::from_json(&serde_json::Value::Number(n.clone())),
    }
}

/// Multiplies the number at `path` by `by`, by the rules of [`doc::mul`].
pub fn mul(doc: &mut Doc, path: &[Seg], by: &Doc) -> Result<Doc, String> {
    let next = match get(doc, path) {
        Some(cur @ (Doc::Int(_) | Doc::Float(_) | Doc::Decimal(_))) => {
            doc::mul(cur, by).map_err(|e| format!("the multiplication cannot be made: {e}"))?
        }
        Some(_) => return Err("the value at this path is not a number".into()),
        None => return Err("nothing at this path".into()),
    };
    set(doc, path, next.clone())?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonpath::parse_path;
    use serde_json::json;

    fn p(s: &str) -> Vec<Seg> {
        parse_path(s).unwrap()
    }

    fn d(j: serde_json::Value) -> Doc {
        doc::from_extended_json(&j).unwrap()
    }

    fn ext(x: &Doc) -> serde_json::Value {
        doc::to_extended_json(x)
    }

    #[test]
    fn changing_one_field_keeps_the_types_of_the_others() {
        let mut x = d(json!({
            "name": "Fordi",
            "balance": {"$decimal": "1250.50"},
            "created_at": {"$datetime": "2026-10-31T16:18:55.000Z"},
            "tags": [{"$date": "2026-01-01"}, 1, 1.5]
        }));
        set(&mut x, &p(".name"), Doc::Str("Fordima".into())).unwrap();
        append(&mut x, &p(".tags"), vec![d(json!([{"$time": "08:00:00"}]))]).unwrap();
        insert(&mut x, &p(".tags[0]"), Doc::Int(0)).unwrap();
        assert!(delete(&mut x, &p(".tags[-1]")));
        assert_eq!(
            ext(&x),
            json!({
                "name": "Fordima",
                "balance": {"$decimal": "1250.50"},
                "created_at": {"$datetime": "2026-10-31T16:18:55.000Z"},
                "tags": [0, {"$date": "2026-01-01"}, 1, 1.5]
            })
        );
    }

    #[test]
    fn numbers_change_exactly_or_not_at_all() {
        let mut x =
            d(json!({"n": 1, "f": 0.5, "m": {"$decimal": "1250.50"}, "big": {"$decimal": "18446744073709551615"}}));
        assert_eq!(ext(&incr(&mut x, &p(".n"), &Doc::Int(2)).unwrap()), json!(3));
        let quarter = d(json!({"$decimal": "0.25"}));
        assert_eq!(ext(&incr(&mut x, &p(".m"), &quarter).unwrap()), json!({"$decimal": "1250.75"}));
        assert_eq!(ext(&incr(&mut x, &p(".m"), &Doc::Int(1)).unwrap()), json!({"$decimal": "1251.75"}));
        assert!(incr(&mut x, &p(".m"), &Doc::Float(0.1)).unwrap_err().contains("do not mix"));
        assert_eq!(ext(&incr(&mut x, &p(".f"), &Doc::Int(1)).unwrap()), json!(1.5));
        // The u64 integers of 2.0 stay integers: beyond them is refused.
        let lower = incr(&mut x, &p(".big"), &Doc::Int(-1)).unwrap();
        assert_eq!(doc::to_plain_json(&lower), json!(18446744073709551614u64));
        assert!(incr(&mut x, &p(".big"), &Doc::Int(2)).is_err());
        // Products of decimals: exact, or refused.
        assert_eq!(ext(&mul(&mut x, &p(".m"), &Doc::Int(2)).unwrap()), json!({"$decimal": "2503.50"}));
        let tiny = d(json!({"$decimal": "0.000000000000000000000000001"}));
        assert!(mul(&mut x, &p(".m"), &tiny).is_err(), "30 decimals would be rounded");
        assert_eq!(ext(get(&x, &p(".m")).unwrap()), json!({"$decimal": "2503.50"}), "a refusal changes nothing");
        assert!(mul(&mut x, &p(".n"), &Doc::Int(i64::MAX)).is_err());
    }
}
