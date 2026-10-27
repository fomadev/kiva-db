//! The order of `order by`: the value a document sorts by, how two rows
//! compare, the rows a limited query keeps while it reads, and how far a
//! walk of an index in order can be from the order of the values.
//!
//! Values sort by kind first (booleans, numbers, texts), then by value. A
//! datetime with an offset sorts by its instant (`15:30+01:00` before
//! `15:00Z`), a time written `HH:MM` as `HH:MM:SS`; other texts as texts,
//! dates (`2026-10-27`) included. A document without the field, or holding
//! `null`, an array or an object there, comes last in both directions.
//! Rows of the same value come in key order.

use super::SKey;
use chrono::{Datelike, Duration, NaiveDate};
use serde_json::Value as Json;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// The value a document sorts by, `None` to come last.
pub fn sort_key(v: Option<&Json>) -> Option<SKey> {
    match SKey::from_json(v?)? {
        SKey::Str(s) => Some(SKey::Str(canonical(&s).unwrap_or(s))),
        sk => sort_key_of(&sk),
    }
}

/// The sort key of a value held by an index: that of the field.
pub fn sort_key_of(v: &SKey) -> Option<SKey> {
    match v {
        SKey::Null | SKey::Tuple(_) => None,
        SKey::Str(s) => Some(SKey::Str(canonical(s).unwrap_or_else(|| s.clone()))),
        sk => Some(sk.clone()),
    }
}

/// The text a time or a datetime with an offset sorts as. The first three
/// characters of a time stay as they are, and a datetime moves by less than
/// a day: [`floor`] and [`ceiling`] count on both.
fn canonical(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() >= 5 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b':' {
        let t = crate::time::parse_time(s).ok()?;
        return Some(t.format("%H:%M:%S%.f").to_string());
    }
    let year = date_prefix(s)?.year();
    if !(1001..=9998).contains(&year) || !has_offset(s) {
        return None;
    }
    let at = crate::time::parse_datetime(s, &crate::time::Zone::Utc).ok()?;
    Some(at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
}

/// Whether a datetime text names its offset: without one it is a wall
/// clock, whose instant depends on the zone reading it, and sorts as text.
fn has_offset(s: &str) -> bool {
    let s = s.trim();
    if s.ends_with(['Z', 'z']) || s.ends_with(" UTC") {
        return true;
    }
    let b = s.as_bytes();
    let signed = |at: usize| b.len() > at && matches!(b[b.len() - at - 1], b'+' | b'-');
    let digits = |r: std::ops::Range<usize>| r.into_iter().all(|i| b[b.len() - 1 - i].is_ascii_digit());
    // +01:00, +0100.
    (signed(5) && b[b.len() - 3] == b':' && digits(0..2) && digits(3..5)) || (signed(4) && digits(0..4))
}

/// The date a text starts with, written `YYYY-MM-DD`.
fn date_prefix(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes().get(..10)?;
    let digits = |r: std::ops::Range<usize>| {
        b[r].iter().try_fold(0u32, |n, c| c.is_ascii_digit().then(|| n * 10 + u32::from(c - b'0')))
    };
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    NaiveDate::from_ymd_opt(digits(0..4)? as i32, digits(5..7)?, digits(8..10)?)
}

fn day_text(d: NaiveDate, days: i64) -> SKey {
    SKey::Str(crate::time::date_text(d + Duration::days(days)))
}

/// Texts that may be read as a time or a datetime, whose sort key can then
/// differ from the text.
fn may_move(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_digit() || c == '+' || c == '-')
}

/// In a walk of an index from its smallest value, at value `v`: a key no
/// value still to come sorts below. `None` when it cannot be told.
pub fn floor(v: &SKey) -> Option<SKey> {
    match v {
        SKey::Str(s) => match date_prefix(s) {
            Some(d) => Some(day_text(d, -1)),
            None if may_move(s) => None,
            None => Some(v.clone()),
        },
        SKey::Tuple(_) => None,
        _ => Some(v.clone()),
    }
}

/// In a walk of an index from its largest value, at value `v`: a key no
/// value still to come sorts above, and whether it may sort equal to it.
pub fn ceiling(v: &SKey) -> Option<(SKey, bool)> {
    match v {
        SKey::Str(s) => match date_prefix(s) {
            Some(d) => Some((day_text(d, 2), false)),
            None if may_move(s) => None,
            None => Some((v.clone(), true)),
        },
        SKey::Tuple(_) => None,
        _ => Some((v.clone(), true)),
    }
}

/// Where a walk may start to find the rows after a row of value `v`: in
/// ascending order, no smaller index value sorts after `v`; in descending
/// order, no larger one sorts before it. `None`: from the very end.
pub fn walk_start(v: &SKey, desc: bool) -> Option<SKey> {
    match v {
        SKey::Str(s) => match date_prefix(s) {
            Some(d) => Some(day_text(d, if desc { 3 } else { -3 })),
            None if may_move(s) => (!desc).then(|| SKey::Str(String::new())),
            None => Some(v.clone()),
        },
        SKey::Tuple(_) => None,
        _ => Some(v.clone()),
    }
}

/// How two rows compare: by their sort keys (`None` last), then by key.
pub fn compare(a: &Option<SKey>, a_key: &str, b: &Option<SKey>, b_key: &str, desc: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => {
            let o = x.cmp(y);
            if desc { o.reverse() } else { o }.then_with(|| a_key.cmp(b_key))
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a_key.cmp(b_key),
    }
}

pub struct Row {
    pub sk: Option<SKey>,
    pub key: String,
    pub doc: Json,
    desc: bool,
}

impl PartialEq for Row {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Row {}
impl PartialOrd for Row {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Row {
    fn cmp(&self, o: &Self) -> Ordering {
        compare(&self.sk, &self.key, &o.sk, &o.key, self.desc)
    }
}

/// The rows a query keeps while it reads: the first `limit` in order (a
/// heap whose top is the last one kept), or all of them without a limit.
pub struct TopK {
    limit: usize,
    desc: bool,
    heap: BinaryHeap<Row>,
    all: Vec<Row>,
    /// Rows offered.
    pub offered: u64,
}

impl TopK {
    pub fn new(limit: usize, desc: bool) -> TopK {
        TopK { limit, desc, heap: BinaryHeap::new(), all: Vec::new(), offered: 0 }
    }

    pub fn offer(&mut self, sk: Option<SKey>, key: String, doc: Json) {
        self.offered += 1;
        let row = Row { sk, key, doc, desc: self.desc };
        if self.limit == 0 {
            self.all.push(row);
        } else if self.heap.len() < self.limit {
            self.heap.push(row);
        } else if self.heap.peek().is_some_and(|last| row < *last) {
            self.heap.pop();
            self.heap.push(row);
        }
    }

    /// Whether a row placed at `sk` would be kept, before its document is
    /// read.
    pub fn would_keep(&self, sk: &Option<SKey>, key: &str) -> bool {
        self.last_kept().is_none_or(|last| compare(sk, key, &last.sk, &last.key, self.desc) == Ordering::Less)
    }

    pub fn len(&self) -> usize {
        self.heap.len() + self.all.len()
    }

    /// The last row kept, once `limit` rows are: a row sorting after it
    /// can no longer make it.
    fn last_kept(&self) -> Option<&Row> {
        (self.limit != 0 && self.heap.len() >= self.limit).then(|| self.heap.peek()).flatten()
    }

    /// Whether no row still to come in a walk of an index can be kept: in
    /// ascending order every one sorts at or above `floor`, in descending
    /// order at or below `ceiling`.
    pub fn full_before(&self, v: &SKey) -> bool {
        let Some(Row { sk: Some(last), .. }) = self.last_kept() else {
            return false;
        };
        if self.desc {
            match ceiling(v) {
                Some((c, true)) => c < *last,
                Some((c, false)) => c <= *last,
                None => false,
            }
        } else {
            floor(v).is_some_and(|f| f > *last)
        }
    }

    /// The rows kept, in order.
    pub fn into_sorted(self) -> Vec<Row> {
        let mut rows = if self.limit == 0 { self.all } else { self.heap.into_vec() };
        rows.sort();
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sk(v: Json) -> Option<SKey> {
        sort_key(Some(&v))
    }

    #[test]
    fn datetimes_with_an_offset_sort_by_their_instant() {
        let a = sk(json!("2026-10-27T15:30:00+01:00")); // 14:30Z
        let b = sk(json!("2026-10-27T15:00:00Z"));
        assert!(a < b);
        // Fractions of a second, written or not.
        assert!(sk(json!("2026-10-27T14:30:00Z")) < sk(json!("2026-10-27T14:30:00.500Z")));
        // Without an offset: a text.
        assert_eq!(sk(json!("2026-10-27T15:30:00")), Some(SKey::Str("2026-10-27T15:30:00".into())));
        assert_eq!(sk(json!("2026-10-27")), Some(SKey::Str("2026-10-27".into())));
        assert_eq!(sk(json!("08:00")), sk(json!("08:00:00")));
        assert!(sk(json!("08:00")) < sk(json!("08:00:01")));
    }

    #[test]
    fn missing_null_and_objects_come_last_both_ways() {
        for desc in [false, true] {
            assert_eq!(compare(&Some(SKey::Bool(false)), "b", &None, "a", desc), Ordering::Less);
            assert_eq!(compare(&None, "a", &None, "b", desc), Ordering::Less);
        }
        assert_eq!(sk(json!(null)), None);
        assert_eq!(sk(json!([1])), None);
        assert_eq!(sk(json!({"$datetime": "2026-10-27T14:30:00Z"})), None);
    }

    #[test]
    fn the_bounds_hold_for_every_text_after_or_before() {
        let texts = [
            "2026-10-25T23:59:00-14:00",
            "2026-10-26",
            "2026-10-26T10:00:00+14:00",
            "2026-10-27T00:30:00+05:00",
            "2026-10-27T13:00:00Z",
            "2026-10-27",
            "2026-10-28T01:00:00+14:00",
            "2026-10-28T23:00:00-12:00",
            "08:00",
            "23:59:59.5",
            "1000",
            "abc",
            "Zeta",
            "+33 6 00",
            "-5",
        ];
        let mut raw: Vec<SKey> = texts.iter().map(|t| SKey::Str(t.to_string())).collect();
        raw.extend([SKey::Num(super::super::OrdF64(3.0)), SKey::Bool(true)]);
        raw.sort();
        let key = |v: &SKey| sort_key(Some(&v.to_json()));
        for (i, v) in raw.iter().enumerate() {
            if let Some(f) = floor(v) {
                for u in &raw[i..] {
                    assert!(key(u).unwrap() >= f, "floor of {v:?} above {u:?}");
                }
            }
            if let Some((c, equal)) = ceiling(v) {
                for u in &raw[..=i] {
                    let k = key(u).unwrap();
                    assert!(if equal { k <= c } else { k < c }, "ceiling of {v:?} below {u:?}");
                }
            }
            for desc in [false, true] {
                let Some(start) = walk_start(v, desc) else { continue };
                let target = key(v).unwrap();
                for u in &raw {
                    let k = key(u).unwrap();
                    let skipped = if desc { *u > start } else { *u < start };
                    let after = if desc { k < target } else { k > target };
                    assert!(!(skipped && after), "start {start:?} for {v:?} skips {u:?}");
                }
            }
        }
    }

    #[test]
    fn the_heap_keeps_the_first_rows() {
        let mut top = TopK::new(2, true);
        for (k, v) in [("a", 1), ("b", 5), ("c", 3), ("d", 5)] {
            top.offer(sk(json!(v)), k.into(), json!(v));
        }
        top.offer(None, "e".into(), json!(null));
        let keys: Vec<String> = top.into_sorted().into_iter().map(|r| r.key).collect();
        assert_eq!(keys, ["b", "d"]);
    }
}
