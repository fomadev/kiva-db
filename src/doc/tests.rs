// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use super::*;
use serde_json::json;

fn dec(s: &str) -> Decimal {
    parse_decimal(s).unwrap()
}

/// A small deterministic generator, so that a failure can be replayed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A number of any of the three kinds, near the edges more often than not.
    fn num(&mut self) -> Doc {
        const EDGES: [i64; 10] = [
            0,
            1,
            -1,
            9_007_199_254_740_992,
            9_007_199_254_740_993,
            -9_007_199_254_740_993,
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            1_000_000_007,
        ];
        let base =
            if self.below(2) == 0 { EDGES[self.below(10) as usize] } else { self.next() as i64 >> self.below(60) };
        match self.below(6) {
            0 | 1 => Doc::Int(base),
            2 => Doc::Float(base as f64),
            3 => Doc::Float(base as f64 / (1u64 << self.below(12)) as f64),
            4 => Doc::Decimal(Decimal::from(base)),
            _ => Doc::Decimal(Decimal::from_i128_with_scale(base as i128, self.below(29) as u32)),
        }
    }

    fn text(&mut self) -> String {
        const PIECES: [&str; 8] = ["date", "int", "$x", "é", "\"", "\\", "日本", "2026-10-31"];
        (0..self.below(4)).map(|_| PIECES[self.below(8) as usize]).collect()
    }

    fn doc(&mut self, depth: usize) -> Doc {
        let pick = if depth >= 4 { self.below(9) } else { self.below(11) };
        match pick {
            0 => Doc::Null,
            1 => Doc::Bool(self.below(2) == 1),
            2..=4 => self.num(),
            5 => Doc::Str(self.text()),
            6 => Doc::Date(epoch_day() + Duration::days(self.below(3_000_000) as i64 - 1_000_000)),
            7 => Doc::DateTime(DateTime::from_timestamp_millis(self.next() as i64 >> 20).unwrap()),
            8 => Doc::Time(
                NaiveTime::from_num_seconds_from_midnight_opt(
                    self.below(86_400) as u32,
                    self.below(1000) as u32 * 1_000_000,
                )
                .unwrap(),
            ),
            9 => Doc::Array((0..self.below(4)).map(|_| self.doc(depth + 1)).collect()),
            _ => {
                Doc::Object((0..self.below(4)).map(|i| (format!("{}{i}", self.text()), self.doc(depth + 1))).collect())
            }
        }
    }
}

#[test]
fn decimals_are_read_as_kivadb_numbers_and_never_rounded() {
    assert_eq!(dec("1250.50").to_string(), "1250.50", "the scale is kept");
    assert_eq!(dec("-0.07").to_string(), "-0.07");
    assert_eq!(dec("79228162514264337593543950335").scale(), 0);
    assert_eq!(dec("0.0000000000000000000000000001").scale(), 28);
    for bad in ["+5", ".5", "5.", "1_000", "1e3", "", "-", "1.2.3", "abc"] {
        assert!(parse_decimal(bad).unwrap_err().contains("is not a decimal"), "{bad}");
    }
    for too_much in
        ["79228162514264337593543950336", "0.00000000000000000000000000001", "123456789012345678901234567890"]
    {
        assert!(parse_decimal(too_much).unwrap_err().contains("does not fit"), "{too_much}");
    }
}

#[test]
fn numbers_of_the_three_kinds_compare_by_their_exact_value() {
    use Ordering::*;
    fn n(d: &Doc) -> Num<'_> {
        d.as_num().unwrap()
    }
    let cases = [
        (Doc::Int(1), Doc::Float(1.0), Equal),
        (Doc::Int(1), Doc::Decimal(dec("1.00")), Equal),
        (Doc::Float(0.5), Doc::Decimal(dec("0.50")), Equal),
        (Doc::Float(-0.0), Doc::Int(0), Equal),
        (Doc::Decimal(dec("-0.00")), Doc::Float(0.0), Equal),
        (Doc::Int(9_007_199_254_740_993), Doc::Float(9_007_199_254_740_992.0), Greater),
        (Doc::Decimal(dec("9007199254740993")), Doc::Float(9_007_199_254_740_992.0), Greater),
        (Doc::Int(i64::MIN), Doc::Float(-9_223_372_036_854_775_808.0), Equal),
        (Doc::Int(i64::MAX), Doc::Float(9_223_372_036_854_775_808.0), Less),
        (Doc::Decimal(dec("18446744073709551615")), Doc::Int(i64::MAX), Greater),
        // 0.1 as a float is 0.1000000000000000055511151231257827...
        (Doc::Float(0.1), Doc::Decimal(dec("0.1")), Greater),
        (Doc::Float(0.1), Doc::Decimal(dec("0.1000000000000000055511151232")), Less),
        (Doc::Float(0.1), Doc::Decimal(dec("0.1000000000000000055511151231")), Greater),
        (Doc::Decimal(dec("79228162514264337593543950335")), Doc::Float(1e29), Less),
        (Doc::Decimal(dec("0.0000000000000000000000000001")), Doc::Float(1e-300), Greater),
        (Doc::Float(5e-324), Doc::Int(0), Greater),
        (Doc::Decimal(dec("-2.5")), Doc::Int(-2), Less),
        (Doc::Float(-2.5), Doc::Decimal(dec("-2.50")), Equal),
    ];
    for (a, b, want) in &cases {
        assert_eq!(cmp_num(n(a), n(b)), *want, "{a:?} against {b:?}");
        assert_eq!(cmp_num(n(b), n(a)), want.reverse(), "{b:?} against {a:?}");
        assert_eq!(num_class(n(a)) == num_class(n(b)), *want == Equal, "the hash of {a:?} and {b:?}");
    }
}

#[test]
fn the_order_of_numbers_is_total_and_agrees_with_their_hash() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let values: Vec<Doc> = (0..300).map(|_| rng.num()).collect();
    for a in &values {
        for b in &values {
            let (x, y) = (a.as_num().unwrap(), b.as_num().unwrap());
            let o = cmp_num(x, y);
            assert_eq!(o, cmp_num(y, x).reverse(), "antisymmetric: {a:?} {b:?}");
            assert_eq!(o == Ordering::Equal, num_class(x) == num_class(y), "hash agrees: {a:?} {b:?}");
        }
    }
    // Transitive: sorting gives an order that every pair respects.
    let mut sorted = values.clone();
    sorted.sort_by(|a, b| cmp_num(a.as_num().unwrap(), b.as_num().unwrap()));
    for w in sorted.windows(2) {
        assert_ne!(cmp_num(w[0].as_num().unwrap(), w[1].as_num().unwrap()), Ordering::Greater);
    }
    for (i, a) in sorted.iter().enumerate().step_by(7) {
        for b in &sorted[i..] {
            assert_ne!(cmp_num(a.as_num().unwrap(), b.as_num().unwrap()), Ordering::Greater, "{a:?} {b:?}");
        }
    }
}

#[test]
fn kinds_sort_in_the_documented_order() {
    let order = [
        Doc::Null,
        Doc::Bool(false),
        Doc::Bool(true),
        Doc::Int(-5),
        Doc::Decimal(dec("2.5")),
        Doc::Float(3.0),
        Doc::Date(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()),
        Doc::DateTime(DateTime::from_timestamp_millis(0).unwrap()),
        Doc::Time(NaiveTime::from_hms_opt(8, 0, 0).unwrap()),
        Doc::Str("2026-10-31".into()),
    ];
    for w in order.windows(2) {
        assert_eq!(cmp_scalar(&w[0], &w[1]), Some(Ordering::Less), "{:?} before {:?}", w[0], w[1]);
    }
    assert_eq!(cmp_scalar(&Doc::Array(vec![]), &Doc::Null), None);
    // A native date and a text that reads as one are two values.
    assert_ne!(Doc::Date(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()), Doc::Str("2026-10-31".into()));
}

#[test]
fn kdoc_gives_back_every_document_with_its_types() {
    let mut rng = Rng(42);
    for _ in 0..2000 {
        let doc = rng.doc(0);
        let bytes = encode(&doc);
        let back = decode(&bytes).unwrap_or_else(|e| panic!("{e}: {doc:?}"));
        assert_eq!(back, doc);
        assert_eq!(type_tree(&back), type_tree(&doc), "types kept");
        assert_eq!(encode(&back), bytes, "one encoding per document");
    }
    let d = Doc::Decimal(dec("1250.50"));
    assert_eq!(decode(&encode(&d)).unwrap().to_string_for_test(), "1250.50", "the scale is kept");
}

/// The types of a document, leaf by leaf.
fn type_tree(d: &Doc) -> String {
    match d {
        Doc::Array(items) => format!("[{}]", items.iter().map(type_tree).collect::<Vec<_>>().join(",")),
        Doc::Object(m) => {
            format!("{{{}}}", m.iter().map(|(k, v)| format!("{k}:{}", type_tree(v))).collect::<Vec<_>>().join(","))
        }
        other => other.type_name().to_string(),
    }
}

impl Doc {
    fn to_string_for_test(&self) -> String {
        match self {
            Doc::Decimal(d) => d.to_string(),
            other => format!("{other:?}"),
        }
    }
}

#[test]
fn kdoc_refuses_damaged_payloads_instead_of_reading_them() {
    let refused = |bytes: &[u8], why: &str| {
        let e = decode(bytes).unwrap_err();
        assert!(e.contains(why), "{bytes:?}: {e}");
    };
    refused(&[], "empty");
    refused(&[2, T_NULL], "version 2");
    refused(&[1, 0x7F], "unknown type");
    refused(&[1, T_NULL, T_NULL], "bytes after");
    refused(&[1, T_STRING, 2, 0xC3, 0x28], "not UTF-8");
    refused(&[1, T_STRING, 200, b'a'], "past the end");
    refused(&[1, T_ARRAY, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F], "past the end");
    refused(&[1, T_INT, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F], "too large");
    let mut nan = vec![1, T_FLOAT];
    nan.extend_from_slice(&f64::NAN.to_le_bytes());
    refused(&nan, "not finite");
    let mut scale = vec![1, T_DECIMAL];
    let mut raw = dec("1.5").serialize();
    raw[2] = 29;
    scale.extend_from_slice(&raw);
    refused(&scale, "wrong scale");
    let mut time = vec![1, T_TIME];
    time.extend_from_slice(&86_400_000u32.to_le_bytes());
    refused(&time, "time out of range");
    let mut twice = vec![1, T_OBJECT, 2];
    for _ in 0..2 {
        twice.extend_from_slice(&[1, b'a', T_NULL]);
    }
    refused(&twice, "given twice");
    // 128 levels are read, 129 are not.
    let nested = |levels: usize| {
        let mut b = vec![1];
        b.extend(std::iter::repeat_n([T_ARRAY, 1], levels - 1).flatten());
        b.push(T_NULL);
        b
    };
    assert!(decode(&nested(MAX_DEPTH)).is_ok());
    refused(&nested(MAX_DEPTH + 1), "deeper than 128");

    // Every prefix of a valid payload is refused, and no change of a byte panics.
    let mut rng = Rng(7);
    for _ in 0..200 {
        let bytes = encode(&rng.doc(0));
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "a prefix of {bytes:?} was read");
        }
        for _ in 0..20 {
            let mut changed = bytes.clone();
            let i = rng.below(changed.len() as u64) as usize;
            changed[i] ^= 1 << rng.below(8);
            let _ = decode(&changed);
        }
    }
}

#[test]
fn json_documents_of_2_0_read_as_they_were() {
    let j = json!({"a": 1, "b": 8.0, "c": -0.0, "d": 18446744073709551615u64, "e": "2026-10-31", "f": [1, "x", null, true]});
    let doc = from_json(&j);
    let Doc::Object(m) = &doc else { panic!() };
    assert!(matches!(m["a"], Doc::Int(1)));
    assert!(matches!(m["b"], Doc::Float(f) if f == 8.0));
    assert!(matches!(m["d"], Doc::Decimal(_)), "a u64 beyond i64 is an exact decimal");
    assert!(matches!(m["e"], Doc::Str(_)), "a text stays a text");
    // Plain JSON gives back exactly what was read, and it can be stored as JSON.
    assert_eq!(to_plain_json(&doc).to_string(), j.to_string());
    assert!(doc.json_encodable());
}

#[test]
fn json_encodable_documents_are_those_json_gives_back_with_their_types() {
    let date = Doc::Date(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
    assert!(!date.json_encodable());
    assert!(!Doc::Array(vec![Doc::Int(1), date]).json_encodable());
    assert!(!Doc::Decimal(dec("1250.50")).json_encodable());
    assert!(!Doc::Decimal(dec("5")).json_encodable(), "JSON would read it back as an int");
    assert!(Doc::Decimal(dec("18446744073709551615")).json_encodable());
    assert!(!Doc::Decimal(dec("18446744073709551616")).json_encodable());
    let mut rng = Rng(99);
    for _ in 0..1000 {
        let doc = rng.doc(0);
        if doc.json_encodable() {
            let back = from_json(&serde_json::from_str(&to_plain_json(&doc).to_string()).unwrap());
            assert_eq!(type_tree(&back), type_tree(&doc), "{doc:?}");
            assert_eq!(back, doc);
        }
    }
}

#[test]
fn plain_json_keeps_values_and_extended_json_keeps_types() {
    let doc = from_extended_json(&json!({
        "balance": {"$decimal": "1250.50"},
        "born": {"$date": "2000-12-25"},
        "opens": {"$time": "08:30:15.1234"},
        "at": {"$datetime": "2026-10-31T16:18:55+01:00"},
        "$$ref": 1,
        "n": 8.0
    }))
    .unwrap();
    assert_eq!(
        to_plain_json(&doc),
        json!({"balance": "1250.50", "born": "2000-12-25", "opens": "08:30:15.123", "at": "2026-10-31T15:18:55.000Z", "$ref": 1, "n": 8.0})
    );
    let ext = to_extended_json(&doc);
    assert_eq!(ext["balance"], json!({"$decimal": "1250.50"}));
    assert_eq!(ext["$$ref"], json!(1), "a key of the data starting with $ is escaped");
    assert!(ext.to_string().contains("\"n\":8.0"), "a float keeps its fraction");
    assert_eq!(from_extended_json(&ext).unwrap(), doc);

    let mut rng = Rng(2026);
    for _ in 0..2000 {
        let doc = rng.doc(0);
        let text = to_extended_json(&doc).to_string();
        let back = from_extended_json(&serde_json::from_str(&text).unwrap()).unwrap_or_else(|e| panic!("{e}: {text}"));
        assert_eq!(back, doc, "{text}");
        assert_eq!(type_tree(&back), type_tree(&doc), "{text}");
    }

    let refused = |j: Json, why: &str| {
        let e = from_extended_json(&j).unwrap_err();
        assert!(e.contains(why), "{j}: {e}");
    };
    refused(json!({"$money": "5"}), "is not a tag");
    refused(json!({"$decimal": 5}), "takes a text");
    refused(json!({"$decimal": "1e3"}), "is not a decimal");
    refused(json!({"$date": "31-10-2026"}), "is not a date");
    refused(json!({"$datetime": "2026-10-31 16:18:55"}), "has no offset");
    refused(json!(18446744073709551615u64), "does not fit an int");
    // A tag beside other keys is not a typed value, and a $ key is refused there too.
    refused(json!({"$date": "2026-10-31", "x": 1}), "is not a tag");
}
