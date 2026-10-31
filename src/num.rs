// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Numbers compared and carried without losing precision.
//!
//! A JSON number is an `i64`, a `u64` or an `f64`. Two of them compare by
//! their mathematical value, never by turning an integer into an `f64`:
//! `9007199254740993` and `9007199254740992` are two values, though they are
//! the same `f64`. `1` and `1.0` are one value.
//!
//! An integer written beyond the 64-bit integers would be read as an `f64`,
//! rounded without a word: [`check_integers`] refuses it before it is parsed.

use serde_json::{Number, Value as Json};
use std::cmp::Ordering;

/// 2^63 and 2^64, exactly, as floats.
const TWO_63: f64 = 9_223_372_036_854_775_808.0;
const TWO_64: f64 = 18_446_744_073_709_551_616.0;

enum Exact {
    Int(i128),
    Float(f64),
}

fn exact(n: &Number) -> Exact {
    if let Some(i) = n.as_i64() {
        Exact::Int(i as i128)
    } else if let Some(u) = n.as_u64() {
        Exact::Int(u as i128)
    } else {
        Exact::Float(n.as_f64().unwrap_or(0.0))
    }
}

/// An integer against a finite float, exactly.
fn int_float(i: i128, f: f64) -> Ordering {
    if f >= TWO_64 {
        return Ordering::Less;
    }
    if f < -TWO_63 {
        return Ordering::Greater;
    }
    let whole = f.trunc();
    // Within (-2^63, 2^64), the whole part of a float is an exact integer.
    i.cmp(&(whole as i128)).then_with(|| 0.0f64.partial_cmp(&(f - whole)).unwrap_or(Ordering::Equal))
}

/// The order of two numbers by their value.
pub fn cmp(a: &Number, b: &Number) -> Ordering {
    match (exact(a), exact(b)) {
        (Exact::Int(x), Exact::Int(y)) => x.cmp(&y),
        (Exact::Int(x), Exact::Float(y)) => int_float(x, y),
        (Exact::Float(x), Exact::Int(y)) => int_float(y, x).reverse(),
        // JSON holds no NaN; -0.0 and 0.0 are equal.
        (Exact::Float(x), Exact::Float(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
    }
}

/// A number as an index holds it: equal, ordered and hashed by its value.
#[derive(Clone, Debug)]
pub struct NumKey(pub Number);

impl PartialEq for NumKey {
    fn eq(&self, o: &Self) -> bool {
        cmp(&self.0, &o.0).is_eq()
    }
}
impl Eq for NumKey {}
impl PartialOrd for NumKey {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for NumKey {
    fn cmp(&self, o: &Self) -> Ordering {
        cmp(&self.0, &o.0)
    }
}
// Equal values hash alike: a float that holds an integer of the 64-bit
// range hashes as that integer, and -0.0 as 0.
impl std::hash::Hash for NumKey {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match exact(&self.0) {
            Exact::Int(i) => (0u8, i).hash(h),
            Exact::Float(f) if f.fract() == 0.0 && (-TWO_63..TWO_64).contains(&f) => (0u8, f as i128).hash(h),
            Exact::Float(f) => (1u8, f.to_bits()).hash(h),
        }
    }
}

/// A JSON number of an integer within 64 bits.
pub fn int_json(i: i128) -> Option<Json> {
    if let Ok(v) = i64::try_from(i) {
        Some(Json::from(v))
    } else {
        u64::try_from(i).ok().map(Json::from)
    }
}

/// The sum of two numbers: exact while both are integers (an overflow of
/// the 64-bit integers is refused), else in floating point.
pub fn add(a: &Number, b: &Number) -> Result<Number, String> {
    match (exact(a), exact(b)) {
        (Exact::Int(x), Exact::Int(y)) => match int_json(x + y) {
            Some(Json::Number(n)) => Ok(n),
            _ => Err("the result would be beyond the 64-bit integers".into()),
        },
        _ => Number::from_f64(a.as_f64().unwrap_or(0.0) + b.as_f64().unwrap_or(0.0))
            .ok_or_else(|| "the result would be NaN or infinite".into()),
    }
}

/// A running sum, exact over integers.
#[derive(Clone, Debug, Default)]
pub struct Sum {
    int: i128,
    float: f64,
    /// Whether a value with a fraction was added: the sum is then a float.
    fractional: bool,
}

impl Sum {
    pub fn push(&mut self, n: &Number) {
        match exact(n) {
            // 2^64 additions of 2^64 stay far within an i128.
            Exact::Int(i) => self.int += i,
            // `1.0` adds exactly, as `1` does.
            Exact::Float(f) if f.fract() == 0.0 && (-TWO_63..TWO_64).contains(&f) => self.int += f as i128,
            Exact::Float(f) => {
                self.fractional = true;
                self.float += f;
            }
        }
    }

    pub fn total(&self) -> f64 {
        self.int as f64 + self.float
    }

    /// The sum as JSON: an exact integer when only integers were added.
    pub fn json(&self) -> Result<Json, String> {
        if self.fractional {
            return Ok(crate::query::num(self.total()));
        }
        int_json(self.int).ok_or_else(|| "the sum is beyond the 64-bit integers".into())
    }
}

/// The digits of a decimal number, without leading or trailing zeros, and
/// the power of ten of the first one: `1250.50` is `("12505", 3)`, `0.07`
/// is `("7", -2)`; zero is `("", 0)`. The sign is left out.
fn digits(text: &str) -> Option<(String, i64)> {
    let s = text.trim_start_matches(['-', '+']);
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], s[i + 1..].parse::<i64>().ok()?),
        None => (s, 0),
    };
    let (whole, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all: String = format!("{whole}{frac}");
    let lead = all.len() - all.trim_start_matches('0').len();
    let kept = all.trim_matches('0');
    if kept.is_empty() {
        return Some((String::new(), 0));
    }
    Some((kept.to_string(), exp + whole.len() as i64 - lead as i64 - 1))
}

/// The float a number text means, if that float holds the same value as
/// the text (`0.1`, `1250.50`, `9007199254740992`); `None` when the float
/// would round it (`9007199254740993`, a 23-digit integer, 20 decimals).
pub fn exact_f64(text: &str) -> Option<f64> {
    let f = text.parse::<f64>().ok().filter(|f| f.is_finite())?;
    let written = digits(text)?;
    // `{:e}` writes the shortest digits that read back as the same float.
    if written == digits(&format!("{f:e}"))? {
        return Some(f);
    }
    // A whole float may hold more digits than its shortest form shows:
    // 18446744073709551616.0 is 2^64 exactly.
    let whole = f.fract() == 0.0 && f.abs() < 1.0e38;
    (whole && written == digits(&(f.abs() as u128).to_string())?).then_some(f)
}

/// A number text as JSON without loss: an `i64`, a `u64`, or a float that
/// holds its value; `None` when none does.
pub fn exact_json(text: &str) -> Option<Json> {
    if let Ok(i) = text.parse::<i64>() {
        return Some(Json::from(i));
    }
    if let Ok(u) = text.parse::<u64>() {
        return Some(Json::from(u));
    }
    exact_f64(text).and_then(Number::from_f64).map(Json::Number)
}

/// Refuses JSON text holding an integer beyond the 64-bit integers, which
/// `serde_json` would read as a rounded float. Numbers with a fraction or
/// an exponent are floats, and are left to it.
pub fn check_integers(text: &[u8]) -> Result<(), String> {
    let mut i = 0;
    while i < text.len() {
        match text[i] {
            b'"' => {
                i += 1;
                while i < text.len() && text[i] != b'"' {
                    i += if text[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                i += 1;
                while i < text.len() && matches!(text[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                    i += 1;
                }
                let token = std::str::from_utf8(&text[start..i]).unwrap_or("");
                let integer = !token.contains(['.', 'e', 'E']);
                if integer && token.parse::<i64>().is_err() && token.parse::<u64>().is_err() && token != "-" {
                    return Err(format!(
                        "the integer {token} is beyond the 64-bit integers (-9223372036854775808 to \
                         18446744073709551615) and would be rounded: write it as a text (\"{token}\"), or with \
                         a fraction (.0) to store it as a rounded float"
                    ));
                }
            }
            _ => i += 1,
        }
    }
    Ok(())
}

/// Reads JSON sent by a program, refusing an integer that would be rounded.
pub fn from_slice(bytes: &[u8]) -> Result<Json, String> {
    let j = serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON: {e}"))?;
    check_integers(bytes)?;
    Ok(j)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(j: Json) -> Number {
        match j {
            Json::Number(n) => n,
            _ => unreachable!(),
        }
    }

    #[test]
    fn numbers_compare_by_value() {
        let big = n(json!(9007199254740993u64));
        assert_eq!(cmp(&big, &n(json!(9007199254740992u64))), Ordering::Greater);
        assert_eq!(cmp(&big, &n(json!(9007199254740992.0))), Ordering::Greater);
        assert_eq!(cmp(&n(json!(1)), &n(json!(1.0))), Ordering::Equal);
        assert_eq!(cmp(&n(json!(-0.0)), &n(json!(0))), Ordering::Equal);
        assert_eq!(cmp(&n(json!(i64::MIN)), &n(json!(-9223372036854775808.0))), Ordering::Equal);
        assert_eq!(cmp(&n(json!(i64::MIN)), &n(json!(-9223372036854777856.0))), Ordering::Greater);
        assert_eq!(cmp(&n(json!(u64::MAX)), &n(json!(18446744073709551616.0))), Ordering::Less);
        assert_eq!(cmp(&n(json!(u64::MAX)), &n(json!(i64::MAX))), Ordering::Greater);
        assert_eq!(cmp(&n(json!(-3)), &n(json!(-2.5))), Ordering::Less);
        assert_eq!(cmp(&n(json!(2)), &n(json!(2.5))), Ordering::Less);
        assert_eq!(cmp(&n(json!(-2)), &n(json!(-2.5))), Ordering::Greater);
        use std::collections::HashSet;
        let set: HashSet<NumKey> =
            [json!(1), json!(1.0), json!(-0.0), json!(0)].into_iter().map(|j| NumKey(n(j))).collect();
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn sums_stay_exact_over_integers() {
        let mut s = Sum::default();
        s.push(&n(json!(9007199254740993u64)));
        s.push(&n(json!(2)));
        s.push(&n(json!(1.0)));
        assert_eq!(s.json().unwrap(), json!(9007199254740996u64));
        s.push(&n(json!(u64::MAX)));
        assert!(s.json().is_err());
        assert!(add(&n(json!(i64::MAX)), &n(json!(1))).is_ok(), "goes on as a u64");
        assert!(add(&n(json!(u64::MAX)), &n(json!(1))).is_err());
        assert_eq!(add(&n(json!(0.5)), &n(json!(1))).unwrap(), n(json!(1.5)));
    }

    #[test]
    fn texts_are_read_without_loss() {
        assert_eq!(exact_json("9007199254740993"), Some(json!(9007199254740993u64)));
        assert_eq!(exact_json("1250.50"), Some(json!(1250.5)));
        assert_eq!(exact_json("0.1"), Some(json!(0.1)));
        assert_eq!(exact_json("-0"), Some(json!(0)));
        assert_eq!(exact_json("2.5E-3"), Some(json!(0.0025)));
        assert_eq!(exact_json("18446744073709551616.0"), Some(json!(18446744073709551616.0)));
        assert_eq!(exact_json("9007199254740993.0"), None);
        assert_eq!(exact_json("12345678901234567890123"), None);
        assert_eq!(exact_json("3.14159265358979323846"), None);
        assert_eq!(exact_json("1e400"), None);
        assert!(
            check_integers(br#"{"a": 18446744073709551615, "b": -9223372036854775808, "c": 1e30, "d": 2.5}"#).is_ok()
        );
        assert!(check_integers(br#"{"a": "12345678901234567890123", "b\"": [1]}"#).is_ok());
        assert!(check_integers(br#"{"a": [1, 12345678901234567890123]}"#)
            .unwrap_err()
            .contains("12345678901234567890123"));
        assert!(check_integers(b"-9223372036854775809").is_err());
    }
}
