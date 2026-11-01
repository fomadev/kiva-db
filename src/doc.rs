// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Documents with typed values: the `Doc` tree, its binary encoding `KDOC`,
//! the exact order of its values, and its plain and extended JSON forms.
//! See `docs/architecture/NATIVE_DOCUMENTS.md`.
//!
//! A `Doc` holds what JSON holds, plus `int`, `float`, `decimal`, `date`,
//! `time` and `datetime` leaves, which keep their type after a write and a
//! read. Nothing in the engine uses it yet (lot C1): it is built and tested
//! on its own before documents move to it.
//!
//! Numbers of the three kinds compare by their exact value: `int 1`,
//! `float 1.0` and `decimal "1.00"` are one value, and no comparison, order
//! or hash turns one into another with a loss.

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Timelike, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Value as Json;
use std::cmp::Ordering;

/// The members of an object, in the order they were written.
pub type Members = IndexMap<String, Doc>;

/// A document value.
#[derive(Clone, Debug)]
pub enum Doc {
    Null,
    Bool(bool),
    Int(i64),
    /// Finite only.
    Float(f64),
    Decimal(Decimal),
    Str(String),
    Date(NaiveDate),
    /// An instant, kept in UTC to the millisecond.
    DateTime(DateTime<Utc>),
    /// A time of day, to the millisecond.
    Time(NaiveTime),
    Array(Vec<Doc>),
    Object(Members),
}

/// The deepest nesting read or written, as `serde_json` allows.
pub const MAX_DEPTH: usize = 128;

impl Doc {
    /// The name of its type, as `typeof` and `json type` say it.
    pub fn type_name(&self) -> &'static str {
        match self {
            Doc::Null => "null",
            Doc::Bool(_) => "boolean",
            Doc::Int(_) => "int",
            Doc::Float(_) => "float",
            Doc::Decimal(_) => "decimal",
            Doc::Str(_) => "string",
            Doc::Date(_) => "date",
            Doc::DateTime(_) => "datetime",
            Doc::Time(_) => "time",
            Doc::Array(_) => "array",
            Doc::Object(_) => "object",
        }
    }

    /// Whether JSON can hold this document without losing a type: it has
    /// no `date`, `time` or `datetime`, and its decimals are only integers
    /// above the 64-bit signed range and within the unsigned one, which JSON
    /// reads back as decimals (NATIVE_DOCUMENTS.md 15.4).
    pub fn json_encodable(&self) -> bool {
        match self {
            Doc::Decimal(d) => u64_beyond_i64(d).is_some(),
            Doc::Date(_) | Doc::DateTime(_) | Doc::Time(_) => false,
            Doc::Array(items) => items.iter().all(Doc::json_encodable),
            Doc::Object(m) => m.values().all(Doc::json_encodable),
            _ => true,
        }
    }
}

/// A decimal with scale 0 above `i64::MAX` and within `u64::MAX`: what a
/// JSON document of 2.0 held as a `u64`.
fn u64_beyond_i64(d: &Decimal) -> Option<u64> {
    if d.normalize().scale() != 0 {
        return None;
    }
    u64::try_from(d.normalize().mantissa()).ok().filter(|u| *u > i64::MAX as u64)
}

/// Reads a decimal written as a KivaDB number: an optional `-`, digits, and
/// optionally a point and digits (`1250.50`); no `+`, exponent, `.5`, `5.`
/// or `_`. Refused when a decimal cannot hold it exactly (more than 28
/// decimals, or beyond about 7.9 × 10^28), never rounded.
pub fn parse_decimal(text: &str) -> Result<Decimal, String> {
    let s = text.trim();
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let (whole, frac) = match unsigned.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (unsigned, None),
    };
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit());
    if !digits(whole) || frac.is_some_and(|f| !digits(f)) {
        return Err(format!(
            "'{text}' is not a decimal: write it as 1250.50 (digits, and optionally a point and digits)"
        ));
    }
    Decimal::from_str_exact(s).map_err(|_| {
        format!("'{text}' does not fit a decimal: at most 28 digits after the point, and about 28 significant digits")
    })
}

// ---------------------------------------------------------------------------
// Exact comparison of numbers
// ---------------------------------------------------------------------------

/// A non-negative integer of any size, little-endian 32-bit limbs.
#[derive(Clone, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn from_u128(mut n: u128) -> Big {
        let mut v = Vec::new();
        while n > 0 {
            v.push(n as u32);
            n >>= 32;
        }
        Big(v)
    }

    fn trim(mut self) -> Big {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
        self
    }

    fn mul_small(mut self, m: u32) -> Big {
        let mut carry = 0u64;
        for limb in &mut self.0 {
            let p = *limb as u64 * m as u64 + carry;
            *limb = p as u32;
            carry = p >> 32;
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
        self.trim()
    }

    fn mul_pow10(mut self, n: u32) -> Big {
        for _ in 0..n {
            self = self.mul_small(10);
        }
        self
    }

    fn shl(self, bits: u32) -> Big {
        let (words, rest) = ((bits / 32) as usize, bits % 32);
        let mut v = vec![0u32; words];
        let mut carry = 0u32;
        for limb in self.0 {
            v.push((limb << rest) | carry);
            carry = if rest == 0 { 0 } else { limb >> (32 - rest) };
        }
        if carry > 0 {
            v.push(carry);
        }
        Big(v).trim()
    }

    fn cmp(&self, o: &Big) -> Ordering {
        let (a, b) = (&self.clone().trim().0, &o.clone().trim().0);
        a.len().cmp(&b.len()).then_with(|| a.iter().rev().cmp(b.iter().rev()))
    }
}

/// A number as an exact fraction of non-negative integers, with its sign:
/// `negative`, numerator, and the denominator as a power of 10 and of 2.
struct Ratio {
    negative: bool,
    num: Big,
    pow10: u32,
    pow2: u32,
}

impl Ratio {
    fn of_int(i: i128) -> Ratio {
        Ratio { negative: i < 0, num: Big::from_u128(i.unsigned_abs()), pow10: 0, pow2: 0 }
    }

    fn of_decimal(d: &Decimal) -> Ratio {
        let m = d.mantissa();
        Ratio { negative: m < 0, num: Big::from_u128(m.unsigned_abs()), pow10: d.scale(), pow2: 0 }
    }

    /// The exact value of a finite float: mantissa × 2^exponent.
    fn of_float(f: f64) -> Ratio {
        let bits = f.to_bits();
        let negative = bits >> 63 == 1 && f != 0.0;
        let exp_bits = ((bits >> 52) & 0x7ff) as i32;
        let frac = bits & ((1u64 << 52) - 1);
        let (m, e) = if exp_bits == 0 { (frac, -1074) } else { (frac | (1u64 << 52), exp_bits - 1075) };
        if e >= 0 {
            Ratio { negative, num: Big::from_u128(m as u128).shl(e as u32), pow10: 0, pow2: 0 }
        } else {
            Ratio { negative, num: Big::from_u128(m as u128), pow10: 0, pow2: (-e) as u32 }
        }
    }

    fn is_zero(&self) -> bool {
        self.num.0.iter().all(|l| *l == 0)
    }

    /// `|self|` against `|o|`: a/(10^p·2^q) against b/(10^r·2^s), by cross multiplication.
    fn cmp_magnitude(&self, o: &Ratio) -> Ordering {
        let left = self.num.clone().mul_pow10(o.pow10).shl(o.pow2);
        let right = o.num.clone().mul_pow10(self.pow10).shl(self.pow2);
        left.cmp(&right)
    }

    fn cmp(&self, o: &Ratio) -> Ordering {
        let sign = |r: &Ratio| {
            if r.is_zero() {
                0
            } else if r.negative {
                -1
            } else {
                1
            }
        };
        match sign(self).cmp(&sign(o)) {
            Ordering::Equal if sign(self) == 0 => Ordering::Equal,
            Ordering::Equal if sign(self) > 0 => self.cmp_magnitude(o),
            Ordering::Equal => o.cmp_magnitude(self),
            other => other,
        }
    }
}

/// A numeric leaf, borrowed.
#[derive(Clone, Copy, Debug)]
pub enum Num<'a> {
    Int(i64),
    Float(f64),
    Decimal(&'a Decimal),
}

impl Doc {
    pub fn as_num(&self) -> Option<Num<'_>> {
        match self {
            Doc::Int(i) => Some(Num::Int(*i)),
            Doc::Float(f) => Some(Num::Float(*f)),
            Doc::Decimal(d) => Some(Num::Decimal(d)),
            _ => None,
        }
    }
}

/// 2^63, exactly, as a float.
const TWO_63: f64 = 9_223_372_036_854_775_808.0;

/// The order of two numbers by their exact value, whatever their kinds.
pub fn cmp_num(a: Num, b: Num) -> Ordering {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => x.cmp(&y),
        (Num::Float(x), Num::Float(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        // The common case without big numbers: within (-2^63, 2^63), the
        // whole part of a float is an exact integer.
        (Num::Int(i), Num::Float(f)) if f.abs() < TWO_63 => {
            let whole = f.trunc();
            i.cmp(&(whole as i64)).then_with(|| 0.0f64.partial_cmp(&(f - whole)).unwrap_or(Ordering::Equal))
        }
        (Num::Float(_), Num::Int(_)) => cmp_num(b, a).reverse(),
        _ => ratio(a).cmp(&ratio(b)),
    }
}

fn ratio(n: Num) -> Ratio {
    match n {
        Num::Int(i) => Ratio::of_int(i as i128),
        Num::Float(f) => Ratio::of_float(f),
        Num::Decimal(d) => Ratio::of_decimal(d),
    }
}

/// What a number hashes as, equal for equal values whatever their kinds
/// (NATIVE_DOCUMENTS.md 15.1): an integer; else the decimal that holds it,
/// without trailing zeros; else, for a float no decimal equals, its bits.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum NumClass {
    Integer(i128),
    Decimal { mantissa: i128, scale: u32 },
    Float(u64),
}

pub fn num_class(n: Num) -> NumClass {
    match n {
        Num::Int(i) => NumClass::Integer(i as i128),
        Num::Decimal(d) => {
            let d = d.normalize();
            match d.scale() {
                0 => NumClass::Integer(d.mantissa()),
                scale => NumClass::Decimal { mantissa: d.mantissa(), scale },
            }
        }
        Num::Float(f) => {
            // Within the 128-bit integers, a whole float is that integer.
            if f.fract() == 0.0 && f.abs() < 1.0e38 {
                return NumClass::Integer(f as i128);
            }
            float_as_decimal(f).unwrap_or(NumClass::Float(f.to_bits()))
        }
    }
}

/// The normalized decimal a fractional float equals exactly, if one does.
fn float_as_decimal(f: f64) -> Option<NumClass> {
    let bits = f.to_bits();
    let exp_bits = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (mut m, mut e) = if exp_bits == 0 { (frac, -1074) } else { (frac | (1u64 << 52), exp_bits - 1075) };
    while m % 2 == 0 && m != 0 {
        m /= 2;
        e += 1;
    }
    // m odd, f = m / 2^k = m·5^k / 10^k; no trailing zero, since m is odd.
    let k = u32::try_from(-e).ok().filter(|k| (1..=Decimal::MAX_SCALE).contains(k))?;
    let mantissa = (m as u128).checked_mul(5u128.checked_pow(k)?)?;
    if mantissa >= 1u128 << 96 {
        return None;
    }
    let mantissa = mantissa as i128;
    Some(NumClass::Decimal { mantissa: if f < 0.0 { -mantissa } else { mantissa }, scale: k })
}

/// The rank of a value's kind in the order of documents: null, booleans,
/// numbers, dates, datetimes, times, texts; arrays and objects last.
fn rank(d: &Doc) -> u8 {
    match d {
        Doc::Null => 0,
        Doc::Bool(_) => 1,
        Doc::Int(_) | Doc::Float(_) | Doc::Decimal(_) => 2,
        Doc::Date(_) => 3,
        Doc::DateTime(_) => 4,
        Doc::Time(_) => 5,
        Doc::Str(_) => 6,
        Doc::Array(_) | Doc::Object(_) => 7,
    }
}

/// The order of two scalar values: by kind, then by value. `None` for
/// arrays and objects, which are not ordered.
pub fn cmp_scalar(a: &Doc, b: &Doc) -> Option<Ordering> {
    if rank(a) == 7 || rank(b) == 7 {
        return None;
    }
    Some(rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
        (Doc::Bool(x), Doc::Bool(y)) => x.cmp(y),
        (Doc::Date(x), Doc::Date(y)) => x.cmp(y),
        (Doc::DateTime(x), Doc::DateTime(y)) => x.cmp(y),
        (Doc::Time(x), Doc::Time(y)) => x.cmp(y),
        (Doc::Str(x), Doc::Str(y)) => x.cmp(y),
        _ => match (a.as_num(), b.as_num()) {
            (Some(x), Some(y)) => cmp_num(x, y),
            _ => Ordering::Equal,
        },
    }))
}

/// Equality of documents: numbers by value, objects member by member
/// whatever their order, arrays element by element.
impl PartialEq for Doc {
    fn eq(&self, o: &Doc) -> bool {
        match (self, o) {
            (Doc::Array(x), Doc::Array(y)) => x == y,
            (Doc::Object(x), Doc::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k) == Some(v)),
            _ => cmp_scalar(self, o) == Some(Ordering::Equal),
        }
    }
}

// ---------------------------------------------------------------------------
// The binary encoding KDOC
// ---------------------------------------------------------------------------

/// The version of `KDOC` this build writes and reads.
pub const KDOC_VERSION: u8 = 1;

const T_NULL: u8 = 0x00;
const T_FALSE: u8 = 0x01;
const T_TRUE: u8 = 0x02;
const T_INT: u8 = 0x03;
const T_FLOAT: u8 = 0x04;
const T_DECIMAL: u8 = 0x05;
const T_STRING: u8 = 0x06;
const T_DATE: u8 = 0x07;
const T_DATETIME: u8 = 0x08;
const T_TIME: u8 = 0x09;
const T_ARRAY: u8 = 0x0A;
const T_OBJECT: u8 = 0x0B;

fn epoch_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).expect("1970-01-01")
}

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// A document in `KDOC`: the version byte, then the value.
pub fn encode(doc: &Doc) -> Vec<u8> {
    let mut out = vec![KDOC_VERSION];
    encode_value(doc, &mut out);
    out
}

fn encode_value(doc: &Doc, out: &mut Vec<u8>) {
    match doc {
        Doc::Null => out.push(T_NULL),
        Doc::Bool(false) => out.push(T_FALSE),
        Doc::Bool(true) => out.push(T_TRUE),
        Doc::Int(i) => {
            out.push(T_INT);
            put_varint(out, ((i << 1) ^ (i >> 63)) as u64);
        }
        Doc::Float(f) => {
            out.push(T_FLOAT);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Doc::Decimal(d) => {
            out.push(T_DECIMAL);
            out.extend_from_slice(&d.serialize());
        }
        Doc::Str(s) => {
            out.push(T_STRING);
            put_varint(out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        Doc::Date(d) => {
            out.push(T_DATE);
            let days = d.signed_duration_since(epoch_day()).num_days() as i32;
            out.extend_from_slice(&days.to_le_bytes());
        }
        Doc::DateTime(t) => {
            out.push(T_DATETIME);
            out.extend_from_slice(&t.timestamp_millis().to_le_bytes());
        }
        Doc::Time(t) => {
            out.push(T_TIME);
            let ms = t.num_seconds_from_midnight() * 1000 + t.nanosecond() / 1_000_000;
            out.extend_from_slice(&ms.to_le_bytes());
        }
        Doc::Array(items) => {
            out.push(T_ARRAY);
            put_varint(out, items.len() as u64);
            for v in items {
                encode_value(v, out);
            }
        }
        Doc::Object(m) => {
            out.push(T_OBJECT);
            put_varint(out, m.len() as u64);
            for (k, v) in m {
                put_varint(out, k.len() as u64);
                out.extend_from_slice(k.as_bytes());
                encode_value(v, out);
            }
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn bad(&self, why: &str) -> String {
        format!("{why} at byte {}", self.pos)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.b.len()).ok_or_else(|| self.bad("cut short"))?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }

    fn varint(&mut self) -> Result<u64, String> {
        let mut n = 0u64;
        for i in 0..10 {
            let c = self.byte()?;
            if i == 9 && c > 1 {
                return Err(self.bad("a number too large"));
            }
            n |= u64::from(c & 0x7f) << (7 * i);
            if c & 0x80 == 0 {
                return Ok(n);
            }
        }
        Err(self.bad("a number too large"))
    }

    /// A length or a count, never more than the bytes left: each element
    /// takes at least one, so nothing is allocated beyond the payload.
    fn count(&mut self) -> Result<usize, String> {
        let n = self.varint()?;
        usize::try_from(n)
            .ok()
            .filter(|n| *n <= self.b.len() - self.pos)
            .ok_or_else(|| self.bad("a length past the end"))
    }

    fn text(&mut self) -> Result<String, String> {
        let n = self.count()?;
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| self.bad("a text that is not UTF-8"))
    }

    fn value(&mut self, depth: usize) -> Result<Doc, String> {
        if depth > MAX_DEPTH {
            return Err(self.bad(&format!("nesting deeper than {MAX_DEPTH}")));
        }
        Ok(match self.byte()? {
            T_NULL => Doc::Null,
            T_FALSE => Doc::Bool(false),
            T_TRUE => Doc::Bool(true),
            T_INT => {
                let z = self.varint()?;
                Doc::Int(((z >> 1) as i64) ^ -((z & 1) as i64))
            }
            T_FLOAT => {
                let f = f64::from_le_bytes(self.array()?);
                if !f.is_finite() {
                    return Err(self.bad("a float that is not finite"));
                }
                Doc::Float(f)
            }
            T_DECIMAL => {
                let raw: [u8; 16] = self.array()?;
                let flags = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
                // `Decimal::deserialize` masks what it does not use: check it here.
                if flags & !0x801F_0000 != 0 || (flags >> 16) & 0xFF > Decimal::MAX_SCALE {
                    return Err(self.bad("a decimal with a wrong scale"));
                }
                Doc::Decimal(Decimal::deserialize(raw))
            }
            T_STRING => Doc::Str(self.text()?),
            T_DATE => {
                let days = i32::from_le_bytes(self.array()?);
                epoch_day()
                    .checked_add_signed(Duration::days(days as i64))
                    .map(Doc::Date)
                    .ok_or_else(|| self.bad("a date out of range"))?
            }
            T_DATETIME => {
                let ms = i64::from_le_bytes(self.array()?);
                DateTime::from_timestamp_millis(ms)
                    .map(Doc::DateTime)
                    .ok_or_else(|| self.bad("a datetime out of range"))?
            }
            T_TIME => {
                let ms = u32::from_le_bytes(self.array()?);
                NaiveTime::from_num_seconds_from_midnight_opt(ms / 1000, (ms % 1000) * 1_000_000)
                    .filter(|_| ms < 86_400_000)
                    .map(Doc::Time)
                    .ok_or_else(|| self.bad("a time out of range"))?
            }
            T_ARRAY => {
                let n = self.count()?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.value(depth + 1)?);
                }
                Doc::Array(items)
            }
            T_OBJECT => {
                let n = self.count()?;
                let mut m = Members::with_capacity(n);
                for _ in 0..n {
                    let k = self.text()?;
                    let v = self.value(depth + 1)?;
                    if m.insert(k, v).is_some() {
                        return Err(self.bad("a member given twice"));
                    }
                }
                Doc::Object(m)
            }
            _ => return Err(self.bad("an unknown type")),
        })
    }
}

/// Reads a `KDOC` payload, refusing anything that is not exactly one
/// well-formed value of a known version: a damaged record is reported, never
/// taken for data.
pub fn decode(bytes: &[u8]) -> Result<Doc, String> {
    let mut r = Reader { b: bytes, pos: 0 };
    let version = r.byte().map_err(|_| "an empty document".to_string())?;
    if version != KDOC_VERSION {
        return Err(format!(
            "a document of encoding version {version}, which this KivaDB does not read: it was written by a newer one"
        ));
    }
    let doc = r.value(1)?;
    if r.pos != bytes.len() {
        return Err(r.bad("bytes after the document"));
    }
    Ok(doc)
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// A JSON document as KivaDB 2.0 stored it: integers within `i64` become
/// `int`, those up to `u64::MAX` an exact `decimal`, others `float`; texts
/// stay texts, whatever they look like.
pub fn from_json(j: &Json) -> Doc {
    match j {
        Json::Null => Doc::Null,
        Json::Bool(b) => Doc::Bool(*b),
        Json::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => Doc::Int(i),
            (None, Some(u)) => Doc::Decimal(Decimal::from(u)),
            _ => Doc::Float(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Doc::Str(s.clone()),
        Json::Array(items) => Doc::Array(items.iter().map(from_json).collect()),
        Json::Object(m) => Doc::Object(m.iter().map(|(k, v)| (k.clone(), from_json(v))).collect()),
    }
}

fn float_json(f: f64) -> Json {
    serde_json::Number::from_f64(f).map(Json::Number).unwrap_or(Json::Null)
}

/// Plain JSON, what programs receive by default: every type of 2.0 exactly
/// as 2.0 wrote it; a `decimal` as its text (a JSON number would be rounded
/// by most parsers), except the integers 2.0 sent as JSON numbers; dates,
/// times and datetimes as their ISO texts. A type may be lost, never a value.
pub fn to_plain_json(doc: &Doc) -> Json {
    match doc {
        Doc::Null => Json::Null,
        Doc::Bool(b) => Json::Bool(*b),
        Doc::Int(i) => Json::from(*i),
        Doc::Float(f) => float_json(*f),
        Doc::Decimal(d) => match u64_beyond_i64(d) {
            Some(u) => Json::from(u),
            None => Json::String(d.to_string()),
        },
        Doc::Str(s) => Json::String(s.clone()),
        Doc::Date(d) => Json::String(crate::time::date_text(*d)),
        Doc::DateTime(t) => Json::String(crate::time::iso(*t)),
        Doc::Time(t) => Json::String(crate::time::time_text(*t)),
        Doc::Array(items) => Json::Array(items.iter().map(to_plain_json).collect()),
        Doc::Object(m) => Json::Object(m.iter().map(|(k, v)| (k.clone(), to_plain_json(v))).collect()),
    }
}

/// The tags of extended JSON.
const TAGS: [&str; 4] = ["$decimal", "$date", "$time", "$datetime"];

fn tagged(tag: &str, text: String) -> Json {
    let mut m = serde_json::Map::new();
    m.insert(tag.to_string(), Json::String(text));
    Json::Object(m)
}

/// Extended JSON, asked for by a program: every type kept without
/// ambiguity. `decimal`, `date`, `time` and `datetime` are written
/// `{"$decimal": "1250.50"}`...; a float always has a fraction or an
/// exponent (`8.0`); a key of the data that starts with `$` is written with
/// the `$` doubled, so that it is never taken for a tag.
pub fn to_extended_json(doc: &Doc) -> Json {
    match doc {
        Doc::Decimal(d) => tagged("$decimal", d.to_string()),
        Doc::Date(d) => tagged("$date", crate::time::date_text(*d)),
        Doc::DateTime(t) => tagged("$datetime", crate::time::iso(*t)),
        Doc::Time(t) => tagged("$time", crate::time::time_text(*t)),
        Doc::Array(items) => Json::Array(items.iter().map(to_extended_json).collect()),
        Doc::Object(m) => Json::Object(
            m.iter()
                .map(|(k, v)| (if k.starts_with('$') { format!("${k}") } else { k.clone() }, to_extended_json(v)))
                .collect(),
        ),
        other => to_plain_json(other),
    }
}

/// Reads extended JSON (see [`to_extended_json`]). An object whose only key
/// is a tag is a typed value; a key starting with `$$` is a key of the data
/// starting with `$`; any other key starting with `$` is refused, since it
/// can only be a mistaken tag. An integer must fit an `int`: a larger one is
/// written `{"$decimal": "..."}`.
pub fn from_extended_json(j: &Json) -> Result<Doc, String> {
    from_extended(j, 1)
}

fn from_extended(j: &Json, depth: usize) -> Result<Doc, String> {
    if depth > MAX_DEPTH {
        return Err(format!("a document nested deeper than {MAX_DEPTH}"));
    }
    Ok(match j {
        Json::Number(n) => match n.as_i64() {
            Some(i) => Doc::Int(i),
            None if n.is_f64() => Doc::Float(n.as_f64().unwrap_or(0.0)),
            None => return Err(format!("{n} does not fit an int: write it as {{\"$decimal\": \"{n}\"}}")),
        },
        Json::Array(items) => Doc::Array(items.iter().map(|v| from_extended(v, depth + 1)).collect::<Result<_, _>>()?),
        Json::Object(m) => {
            if let (1, Some((tag, value))) = (m.len(), m.iter().next()) {
                if TAGS.contains(&tag.as_str()) {
                    let text = value.as_str().ok_or_else(|| format!("{tag} takes a text, not {value}"))?;
                    return typed_leaf(tag, text);
                }
            }
            let mut out = Members::with_capacity(m.len());
            for (k, v) in m {
                let key = match k.strip_prefix("$$") {
                    Some(rest) => format!("${rest}"),
                    None if k.starts_with('$') => {
                        return Err(format!(
                            "'{k}' is not a tag ({}); a key of the data that starts with $ is written ${k}",
                            TAGS.join(", ")
                        ))
                    }
                    None => k.clone(),
                };
                out.insert(key, from_extended(v, depth + 1)?);
            }
            Doc::Object(out)
        }
        plain => from_json(plain),
    })
}

/// The value of a tag of extended JSON, read in ISO only: programs never
/// use the formats of a session. Times and datetimes are kept to the
/// millisecond, as everywhere; a datetime names its offset, since a program
/// has no session zone to read a wall clock in.
fn typed_leaf(tag: &str, text: &str) -> Result<Doc, String> {
    use crate::time::{parse_datetime, Zone};
    Ok(match tag {
        "$decimal" => Doc::Decimal(parse_decimal(text)?),
        "$date" => Doc::Date(crate::time::parse_date(text)?),
        "$time" => {
            let t = crate::time::parse_time(text)?;
            Doc::Time(t.with_nanosecond(t.nanosecond() / 1_000_000 * 1_000_000).unwrap_or(t))
        }
        _ => {
            let at = parse_datetime(text, &Zone::Utc)?;
            // Read in two zones, a text without an offset gives two instants.
            let elsewhere = Zone::parse("+01:00").map(|z| parse_datetime(text, &z)).ok();
            if elsewhere.is_some_and(|e| e != Ok(at)) {
                return Err(format!("'{text}' has no offset: write it as 2026-10-31T15:18:00Z or with +01:00"));
            }
            Doc::DateTime(DateTime::from_timestamp_millis(at.timestamp_millis()).unwrap_or(at))
        }
    })
}

#[cfg(test)]
mod tests;
