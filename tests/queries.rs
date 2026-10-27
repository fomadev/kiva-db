// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Queries: `order by`, pages (`after`), the plans the optimiser chooses and
//! what `explain analyze` counts. Every query run through the indexes is
//! compared with a reading of every document, filtered and sorted here,
//! independently of the engine.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::engine::{Engine, EngineOptions, IndexKind, Measured, Order, Plan, ReadOpts};
use kivadb::instance::Instance;
use kivadb::query::{Expr, Query};
use kivadb::session::Session;
use kivadb::value::Value;
use serde_json::{json, Value as Json};
use std::cmp::Ordering;
use std::path::Path;
use std::sync::Arc;

fn quiet() -> EngineOptions {
    EngineOptions { background: false, auto_compact: false, ..Default::default() }
}

/// xorshift: the same documents at every run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

const OFFSETS: [&str; 6] = ["Z", "+01:00", "-05:00", "+14:00", "-12:00", "+05:30"];

/// A document holding every kind of value an order has to place: missing
/// fields, nulls, texts among numbers, datetimes with different offsets and
/// precisions, dates, wall clocks, times with and without seconds, arrays.
fn person(r: &mut Rng, i: usize) -> Json {
    let mut d = serde_json::Map::new();
    d.insert("n".into(), json!(i));
    d.insert("team".into(), json!(["red", "blue", "green"][r.below(3) as usize]));
    match r.below(10) {
        0 => {}
        1 => _ = d.insert("age".into(), Json::Null),
        2 => _ = d.insert("age".into(), json!("n/a")),
        _ => _ = d.insert("age".into(), json!(r.below(90))),
    }
    let day = 25 + r.below(4);
    match r.below(12) {
        0 => {}
        1 => _ = d.insert("at".into(), Json::Null),
        2 => _ = d.insert("at".into(), json!(format!("2026-10-{day}"))),
        3 => _ = d.insert("at".into(), json!(format!("2026-10-{day}T{:02}:{:02}:00", r.below(24), r.below(60)))),
        _ => {
            let frac = if r.below(3) == 0 { format!(".{}", r.below(1000)) } else { String::new() };
            let (h, m, s) = (r.below(24), r.below(60), r.below(60));
            let off = OFFSETS[r.below(6) as usize];
            d.insert("at".into(), json!(format!("2026-10-{day}T{h:02}:{m:02}:{s:02}{frac}{off}")));
        }
    }
    match r.below(6) {
        0 => {}
        1 => _ = d.insert("opens".into(), json!(format!("{:02}:{:02}", r.below(24), r.below(60)))),
        _ => _ = d.insert("opens".into(), json!(format!("{:02}:{:02}:{:02}", r.below(24), r.below(60), r.below(60)))),
    }
    let score = if r.below(8) == 0 { json!([1, 2]) } else { json!(r.below(1000) as f64 / 10.0) };
    d.insert("score".into(), score);
    Json::Object(d)
}

/// The order, written again here: kinds (booleans, numbers, texts), then
/// values; a datetime with an offset by its instant, a time with seconds.
#[derive(PartialEq, PartialOrd, Debug)]
enum Key {
    Bool(bool),
    Num(f64),
    Text(String),
}

fn reference_key(v: Option<&Json>) -> Option<Key> {
    match v? {
        Json::Bool(b) => Some(Key::Bool(*b)),
        Json::Number(n) => Some(Key::Num(n.as_f64()?)),
        Json::String(s) => {
            let time = chrono::NaiveTime::parse_from_str(s, "%H:%M:%S%.f")
                .or_else(|_| chrono::NaiveTime::parse_from_str(s, "%H:%M"))
                .map(|t| t.format("%H:%M:%S%.f").to_string());
            let instant = chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string());
            Some(Key::Text(time.or(instant).unwrap_or_else(|_| s.clone())))
        }
        _ => None,
    }
}

fn compare_keys(a: (&str, &Option<Key>), b: (&str, &Option<Key>), desc: bool) -> Ordering {
    match (a.1, b.1) {
        (Some(x), Some(y)) => {
            let o = x.partial_cmp(y).unwrap();
            if desc { o.reverse() } else { o }.then_with(|| a.0.cmp(b.0))
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.0.cmp(b.0),
    }
}

fn reference_cmp(a: &(String, Json), b: &(String, Json), field: &str, desc: bool) -> Ordering {
    let (x, y) = (reference_key(a.1.get(field)), reference_key(b.1.get(field)));
    compare_keys((&a.0, &x), (&b.0, &y), desc)
}

/// What a query must return: every document read, filtered, put in order.
fn reference(docs: &[(String, Json)], q: &Query, o: &Opts) -> Vec<String> {
    let mut rows: Vec<(String, Json)> = docs.iter().filter(|(_, d)| q.matches(d)).cloned().collect();
    match &o.order {
        Some((field, desc)) => {
            let mut keyed: Vec<(String, Option<Key>)> =
                rows.drain(..).map(|(k, d)| (k, reference_key(d.get(*field)))).collect();
            keyed.sort_by(|a, b| compare_keys((&a.0, &a.1), (&b.0, &b.1), *desc));
            if let Some(after) = &o.after {
                let at = &docs.iter().find(|(k, _)| k == after).unwrap();
                let at_key = reference_key(at.1.get(*field));
                keyed.retain(|r| compare_keys((&r.0, &r.1), (&at.0, &at_key), *desc) == Ordering::Greater);
            }
            rows = keyed.into_iter().map(|(k, _)| (k, Json::Null)).collect();
        }
        None => {
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            if o.reverse {
                rows.reverse();
            }
            if let Some(after) = &o.after {
                rows.retain(|(k, _)| if o.reverse { k < after } else { k > after });
            }
        }
    }
    let keys = rows.into_iter().map(|(k, _)| k);
    if o.limit > 0 {
        keys.take(o.limit).collect()
    } else {
        keys.collect()
    }
}

#[derive(Clone, Debug, Default)]
struct Opts {
    limit: usize,
    order: Option<(&'static str, bool)>,
    after: Option<String>,
    reverse: bool,
}

impl Opts {
    fn read(&self) -> ReadOpts {
        ReadOpts {
            limit: self.limit,
            order: self.order.map(|(f, desc)| Order::new(f, desc).unwrap()),
            after: self.after.clone(),
            reverse: self.reverse,
        }
    }
}

fn query(w: Json) -> Query {
    Query { pattern: "p:*".into(), expr: Some(Expr::from_json(&w, "p:*").unwrap()) }
}

fn run(e: &Engine, q: &Query, o: &ReadOpts) -> (Vec<String>, Plan, Measured) {
    let mut keys = Vec::new();
    let (plan, m) = e
        .read_query(q, o, |k, _| {
            keys.push(k);
            Ok(())
        })
        .unwrap();
    (keys, plan, m)
}

fn conditions() -> Vec<Json> {
    let cmp = |f: &str, c: &str, v: Json| json!({"field": f, "cmp": c, "value": v});
    vec![
        cmp("age", ">=", json!(18)),
        json!({"team": "red"}),
        json!([{"team": "red"}, cmp("age", "<", json!(40))]),
        json!([{"team": ["red", "blue"]}, cmp("score", ">", json!(20))]),
        cmp("at", ">=", json!({"$datetime": "2026-10-26T12:00:00Z"})),
        cmp("at", "<", json!({"$datetime": "2026-10-27T03:00:00+05:00"})),
        cmp("n", ">=", json!(0)),
        json!({"not": {"team": "green"}}),
        json!([{"or": [{"team": "red"}, {"team": "blue"}]}, cmp("age", ">", json!(50))]),
        json!({"team": "blue", "n": 17}),
        cmp("age", ">", json!(1000)),
        cmp("opens", "<=", json!({"$time": "12:00"})),
        cmp("at", "like", json!("2026-10-26%")),
    ]
}

const ORDERS: [Option<(&str, bool)>; 9] = [
    None,
    Some(("age", false)),
    Some(("age", true)),
    Some(("at", false)),
    Some(("at", true)),
    Some(("opens", false)),
    Some(("opens", true)),
    Some(("score", true)),
    Some(("n", true)),
];

fn put(e: &Engine, docs: &mut Vec<(String, Json)>, key: &str, d: Json) {
    e.put(key, &Value::Doc(d.clone()), 0, false).unwrap();
    match docs.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = d,
        None => docs.push((key.to_string(), d)),
    }
}

fn indexed(e: &Engine) {
    e.create_index_with("p:*", "age", IndexKind::BTree).unwrap();
    e.create_index_with("p:*", "at", IndexKind::BTree).unwrap();
    e.create_index_with("p:*", "opens", IndexKind::BTree).unwrap();
    e.create_index_with("p:*", "team", IndexKind::Hash).unwrap();
    e.create_index_with("p:*", "score", IndexKind::BTree).unwrap();
    e.create_index_with("p:*", "n", IndexKind::BTree).unwrap();
    e.create_index_on("p:*", &["team", "n"], IndexKind::BTree, true).unwrap();
}

/// Every condition, order, limit and page, with and without indexes,
/// against the reference. Returns the strategies the indexed engine chose.
fn compare_all(with: &Engine, without: &Engine, docs: &[(String, Json)]) -> Vec<&'static str> {
    let mut strategies = Vec::new();
    for w in conditions() {
        let q = query(w.clone());
        for order in ORDERS {
            for limit in [0, 7] {
                for (after, reverse) in [(None, false), (Some("p:00037"), false), (None, true), (Some("p:00600"), true)]
                {
                    if reverse && order.is_some() {
                        continue;
                    }
                    let o = Opts { limit, order, after: after.map(String::from), reverse };
                    let want = reference(docs, &q, &o);
                    let (got, plan, m) = run(with, &q, &o.read());
                    assert_eq!(got, want, "{w} {o:?} with indexes ({})", plan.strategy);
                    assert_eq!(m.returned as usize, want.len());
                    let (plain, ..) = run(without, &q, &o.read());
                    assert_eq!(plain, want, "{w} {o:?} without indexes");
                    strategies.push(plan.strategy);
                }
            }
        }
    }
    strategies
}

#[test]
fn indexed_sorted_and_paged_queries_match_a_reading_of_every_document() {
    let dir = TempDir::new("queries_reference");
    let cached = || EngineOptions { cache_bytes: 64 << 20, ..quiet() };
    let with = Engine::open(dir.path().join("with.kiva"), cached()).unwrap();
    let without = Engine::open(dir.path().join("without.kiva"), cached()).unwrap();
    indexed(&with);
    let mut docs = Vec::new();
    let mut plain = Vec::new();
    let mut r = Rng(0x9e3779b97f4a7c15);
    for i in 0..800 {
        let d = person(&mut r, i);
        put(&with, &mut docs, &format!("p:{i:05}"), d.clone());
        put(&without, &mut plain, &format!("p:{i:05}"), d);
    }
    // Keys that are not documents of the pattern are never rows.
    with.put("p:zz-text", &Value::Str("x".into()), 0, false).unwrap();
    without.put("p:zz-text", &Value::Str("x".into()), 0, false).unwrap();
    let strategies = compare_all(&with, &without, &docs);
    for s in ["INDEX ORDER SCAN", "INDEX SCAN", "INDEX MERGE", "PREFIX SCAN"] {
        assert!(strategies.contains(&s), "{s} was never chosen: the test no longer covers it");
    }

    // Changed after the indexes were made: values moved, fields removed,
    // set to null or to another kind, documents deleted.
    for i in (0..800).step_by(7) {
        let key = format!("p:{i:05}");
        let mut d = docs.iter().find(|(k, _)| *k == key).unwrap().1.clone();
        match i % 4 {
            0 => _ = d.as_object_mut().unwrap().remove("at"),
            1 => d["age"] = json!(i % 90),
            2 => d["at"] = Json::Null,
            _ => d["opens"] = json!(7),
        }
        put(&with, &mut docs, &key, d.clone());
        put(&without, &mut plain, &key, d);
    }
    for i in (3..800).step_by(11) {
        let key = format!("p:{i:05}");
        if key == "p:00037" || key == "p:00600" {
            continue;
        }
        with.delete(&key).unwrap();
        without.delete(&key).unwrap();
        docs.retain(|(k, _)| *k != key);
    }
    compare_all(&with, &without, &docs);

    // After a compaction, and once the indexes are rebuilt by a restart.
    with.compact().unwrap();
    compare_all(&with, &without, &docs);
    drop(with);
    let with = Engine::open(dir.path().join("with.kiva"), cached()).unwrap();
    compare_all(&with, &without, &docs);
}

#[test]
fn pages_follow_each_other_without_a_gap_or_a_repeat() {
    let dir = TempDir::new("queries_pages");
    let e = Engine::open(dir.path().join("p.kiva"), quiet()).unwrap();
    indexed(&e);
    let mut docs = Vec::new();
    let mut r = Rng(42);
    for i in 0..700 {
        put(&e, &mut docs, &format!("p:{i:05}"), person(&mut r, i));
    }
    let q = query(json!({"field": "n", "cmp": ">=", "value": 0}));
    for (order, reverse) in [(None, false), (None, true), (Some(("at", true)), false), (Some(("age", false)), false)] {
        let full = reference(&docs, &q, &Opts { order, reverse, ..Opts::default() });
        let mut paged: Vec<String> = Vec::new();
        loop {
            let o = Opts { limit: 9, order, after: paged.last().cloned(), reverse };
            let (page, ..) = run(&e, &q, &o.read());
            if page.is_empty() {
                break;
            }
            assert!(page.len() <= 9);
            paged.extend(page);
        }
        assert_eq!(paged, full, "{order:?} reverse {reverse}");
    }
    // The key a page ends on may be deleted before the next one: in key
    // order the next page still starts after it; in the order of a field its
    // place is no longer known, and that is said.
    let o = Opts { limit: 3, order: Some(("at", false)), after: Some("p:99999".into()), reverse: false };
    let err = e.read_query(&q, &o.read(), |_, _| Ok(())).unwrap_err().to_string();
    assert!(err.contains("no such document"), "{err}");
    let o = Opts { limit: 3, after: Some("p:00010x".into()), ..Opts::default() };
    assert_eq!(run(&e, &q, &o.read()).0, ["p:00011", "p:00012", "p:00013"]);
}

#[test]
fn large_sets_read_only_what_the_plan_needs() {
    let dir = TempDir::new("queries_large");
    let e = Engine::open(dir.path().join("l.kiva"), quiet()).unwrap();
    for i in 0..20_000u64 {
        let off = OFFSETS[(i % 6) as usize];
        let doc = json!({"num": i, "grp": i % 100, "x": i % 3,
            "t": format!("2026-{:02}-{:02}T{:02}:00:00{off}", 1 + i % 12, 1 + i % 28, i % 24)});
        e.put(&format!("k:{i:06}"), &Value::Doc(doc), 0, false).unwrap();
    }
    e.create_index("k:*", "num").unwrap();
    e.create_index("k:*", "t").unwrap();
    e.create_index_with("k:*", "grp", IndexKind::Hash).unwrap();
    let docs: Vec<(String, Json)> = (0..20_000u64)
        .map(|i| {
            let (v, _) = e.get(&format!("k:{i:06}")).unwrap().unwrap();
            let Value::Doc(d) = v else { panic!() };
            (format!("k:{i:06}"), d)
        })
        .collect();
    let q = |w: Json| Query { pattern: "k:*".into(), expr: Some(Expr::from_json(&w, "k:*").unwrap()) };
    let check = |w: Json, o: Opts| {
        let q = q(w);
        let (got, plan, m) = run(&e, &q, &o.read());
        assert_eq!(got, reference(&docs, &q, &o), "{o:?}");
        (plan, m)
    };

    // The ten largest: the index of the order is read from its end, ten
    // documents read.
    let (plan, m) = check(
        json!({"field": "num", "cmp": ">=", "value": 0}),
        Opts { limit: 10, order: Some(("num", true)), ..Opts::default() },
    );
    assert_eq!(plan.strategy, "INDEX ORDER SCAN");
    assert_eq!(m.docs_read, 10);

    // A selective filter: its 30 documents read and sorted.
    let (plan, m) = check(
        json!({"field": "num", "cmp": "<", "value": 30}),
        Opts { limit: 5, order: Some(("t", false)), ..Opts::default() },
    );
    assert_eq!(plan.strategy, "INDEX SCAN");
    assert_eq!((m.docs_read, m.sorted), (30, 30));

    // Less selective: the order index, each key checked against the index
    // of the filter before its document is read.
    let (plan, m) = check(json!({"grp": 7}), Opts { limit: 5, order: Some(("t", false)), ..Opts::default() });
    assert_eq!(plan.strategy, "INDEX ORDER SCAN");
    assert!(m.docs_read < 200, "{m:?}");
    let half: Vec<u64> = (0..50).collect();
    let (plan, m) = check(json!({"grp": half}), Opts { limit: 5, order: Some(("t", false)), ..Opts::default() });
    assert_eq!(plan.strategy, "INDEX ORDER SCAN");
    assert!(m.docs_read < 1000, "{m:?}");

    // `and`: the 200 keys of the smallest part checked against the other
    // index, not its 10 000 keys listed.
    let (plan, m) = check(json!([{"grp": 7}, {"field": "num", "cmp": "<", "value": 10_000}]), Opts::default());
    assert_eq!(plan.strategy, "INDEX MERGE");
    assert_eq!((m.index_entries, m.docs_read), (400, 100));

    // A limit without an index stops after a few keys, never copying the
    // 20 000 keys of the pattern.
    let (plan, m) = check(json!({"x": 1}), Opts { limit: 3, ..Opts::default() });
    assert_eq!(plan.strategy, "PREFIX SCAN");
    assert!(m.keys_examined <= 256, "{m:?}");
    assert_eq!(m.returned, 3);

    // A unique index on two fields answers both equalities in one lookup.
    e.create_index_on("k:*", &["grp", "num"], IndexKind::BTree, true).unwrap();
    let (plan, m) = check(json!({"grp": 7, "num": 707}), Opts::default());
    assert!(plan.index.as_deref().unwrap().starts_with("k:* (grp, num)"), "{plan:?}");
    assert_eq!((m.docs_read, m.returned), (1, 1));
    let (_, m) = check(json!({"grp": 7, "num": 708}), Opts::default());
    assert_eq!((m.docs_read, m.returned), (0, 0));
}

#[test]
fn explain_reads_no_document_and_analyze_reads_them() {
    let dir = TempDir::new("queries_no_read");
    let e = Engine::open(dir.path().join("x.kiva"), EngineOptions { cache_bytes: 1 << 20, ..quiet() }).unwrap();
    for i in 0..50 {
        e.put(&format!("p:{i:02}"), &Value::Doc(json!({"n": i, "t": format!("2026-10-{:02}", 1 + i % 28)})), 0, false)
            .unwrap();
    }
    e.create_index("p:*", "n").unwrap();
    e.create_index("p:*", "t").unwrap();
    let lookups = |e: &Engine| {
        let s = e.stats();
        s.cache_hits + s.cache_misses
    };
    let before = lookups(&e);
    let cases = [
        (json!({"field": "n", "cmp": ">=", "value": 10}), Opts::default()),
        (
            json!({"field": "n", "cmp": ">=", "value": 0}),
            Opts { limit: 3, order: Some(("t", true)), ..Opts::default() },
        ),
        (json!({"t": "2026-10-03"}), Opts { limit: 3, order: Some(("n", false)), ..Opts::default() }),
        (json!({"not": {"n": 3}}), Opts { limit: 2, reverse: true, ..Opts::default() }),
    ];
    for (w, o) in &cases {
        e.explain_read(&query(w.clone()), &o.read());
    }
    assert_eq!(lookups(&e), before, "explain read a document");
    let mut read = 0;
    for (w, o) in &cases {
        read += run(&e, &query(w.clone()), &o.read()).2.docs_read;
    }
    assert_eq!(lookups(&e) - before, read, "every document counted was looked up once");
}

#[test]
fn explain_analyze_counts_what_it_read() {
    let dir = TempDir::new("queries_explain");
    let mut s = Session::new(instance(&dir), true, None);
    for i in 0..100 {
        ok(
            &mut s,
            &format!(
                r#"set user:{i:03} {{"age": {i}, "email": "u{i}@secret.example", "created_at": "2026-10-{:02}T10:00:00Z"}}"#,
                1 + i % 28
            ),
        );
    }
    ok(&mut s, "create index on user:* (age)");
    let plan = ok(&mut s, "explain get where user:*.age >= 70 order by created_at desc limit 5");
    assert!(plan.starts_with("Query Plan\n") && !plan.contains("Measured"), "{plan}");
    assert!(plan.contains("Order     : created_at desc, sorted in memory (the first 5 kept while reading)"), "{plan}");
    assert!(plan.contains("Tip       : create index on user:* (created_at)"), "{plan}");

    let a = ok(&mut s, "explain analyze get where user:*.age >= 70 order by created_at desc limit 5");
    assert!(a.starts_with("Query Plan (explain analyze: the query was run; its rows are not shown)"), "{a}");
    for line in [
        "Strategy  : INDEX SCAN",
        "Index     : user:* (age)",
        "Limit     : 5",
        "Estimated : 30 of 100 keys examined",
        "Index entries : 30",
        "Keys examined : 30",
        "Documents     : 30 read",
        "0 filtered out",
        "Sorted        : 30 rows",
        "Returned      : 5",
    ] {
        assert!(a.contains(line), "missing `{line}`:\n{a}");
    }
    // Plans show the query, never the values of the documents.
    assert!(!a.contains("secret.example"), "{a}");

    // Without an index: every key of the prefix, most of them filtered out.
    let a = ok(&mut s, "explain analyze get where user:*.email like \"u9%\"");
    for line in [
        "Strategy  : PREFIX SCAN",
        "Keys examined : 100",
        "Documents     : 100 read",
        "89 filtered out",
        "Returned      : 11",
    ] {
        assert!(a.contains(line), "missing `{line}`:\n{a}");
    }
    // With an index on the order, the reading stops after the limit.
    ok(&mut s, "create index on user:* (created_at)");
    let a = ok(&mut s, "explain analyze get where user:*.age >= 0 order by created_at desc limit 3");
    assert!(a.contains("Strategy  : INDEX ORDER SCAN") && a.contains("Returned      : 3"), "{a}");
    // Datetimes are read a day around the limit: their offsets may reorder them.
    let read: u64 = a.split("Documents     : ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
    assert!(read < 20, "{a}");
    // An aggregate: the groups it made.
    let a = ok(&mut s, "explain analyze count where user:*.age >= 50 group by created_at");
    assert!(a.contains("Groups        : 28 (50 documents folded)"), "{a}");
    // No result.
    let a = ok(&mut s, "explain analyze get where user:*.age > 1000");
    assert!(a.contains("Returned      : 0") && a.contains("Documents     : 0 read"), "{a}");

    // Writes are never run, analyzed or not.
    for w in [
        "explain analyze set x 1",
        "explain set x 1",
        "explain analyze del user:001",
        "explain analyze update user:001 {}",
    ] {
        assert!(fails(&mut s, w).contains("never runs a write"), "{w}");
    }
    assert_eq!(ok(&mut s, "get x"), "x: (nil)");
    assert!(ok(&mut s, "get user:001").contains("\"age\":1"));

    // Over REST: the plan, and the counts under "analyze".
    let r = kivadb::api::handle(
        &mut s,
        &json!({"op": "explain", "target": "user:*", "where": {"field": "age", "cmp": ">=", "value": 70},
                "order_by": "created_at desc", "limit": 4, "analyze": true}),
    )
    .unwrap();
    assert_eq!(r["plan"]["strategy"], "INDEX ORDER SCAN");
    assert_eq!(r["analyze"]["returned"], 4);
    assert!(r["analyze"]["time_ms"]["total"].as_f64().unwrap() >= 0.0);
    let r =
        kivadb::api::handle(&mut s, &json!({"op": "explain", "target": "user:*.age", "cmp": ">", "value": 1})).unwrap();
    assert!(r["analyze"].is_null(), "explain without analyze runs nothing: {r}");
    let r = kivadb::api::handle(
        &mut s,
        &json!({"op": "query", "target": "user:*", "where": {"field": "age", "cmp": "<", "value": 3}, "order_by": "age desc"}),
    )
    .unwrap();
    let keys: Vec<&str> = r["rows"].as_array().unwrap().iter().map(|x| x["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["user:002", "user:001", "user:000"]);
}

#[test]
fn explain_analyze_needs_read_access_and_reads_what_is_committed() {
    let dir = TempDir::new("queries_access");
    let inst: Arc<Instance> = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "workspace create shop");
    ok(&mut admin, "workspace create hr");
    ok(&mut admin, r#"create user "app" identified by "pw" grant read on shop"#);
    ok(&mut admin, "workspace switch hr");
    ok(&mut admin, r#"set salary:1 {"amount": 1000}"#);
    ok(&mut admin, "workspace switch shop");
    ok(&mut admin, r#"set item:1 {"price": 3}"#);

    let mut app = Session::new(inst.clone(), false, None);
    ok(&mut app, "auth app pw");
    ok(&mut app, "workspace switch shop");
    assert!(ok(&mut app, "explain analyze get where item:*.price > 0").contains("Returned      : 1"));
    assert!(fails(&mut app, "workspace switch hr").contains("no access"));
    // Revoked: a session checks it at its next read of a journal or its
    // next workspace, as for get where; a new one is refused at once.
    ok(&mut admin, r#"revoke read on shop from "app""#);
    assert!(app.check_reads_now().is_err());
    let mut again = Session::new(inst.clone(), false, None);
    ok(&mut again, "auth app pw");
    assert!(fails(&mut again, "workspace switch shop").contains("no access"));

    // A transaction's own writes are not read before its commit, as by get where.
    ok(&mut admin, "begin");
    ok(&mut admin, r#"set item:2 {"price": 5}"#);
    assert!(ok(&mut admin, "explain analyze get where item:*.price > 0").contains("Returned      : 1"));
    ok(&mut admin, "commit");
    assert!(ok(&mut admin, "explain analyze get where item:*.price > 0").contains("Returned      : 2"));
}

#[test]
fn queries_running_while_others_write_see_one_state_each() {
    let dir = TempDir::new("queries_concurrent");
    let e = Engine::open(dir.path().join("c.kiva"), quiet()).unwrap();
    e.create_index("c:*", "v").unwrap();
    e.create_index("c:*", "w").unwrap();
    let writer = {
        let e = e.clone();
        std::thread::spawn(move || {
            for i in 0..3000u64 {
                let doc = json!({"v": i % 500, "w": (i * 7919) % 1000});
                e.put(&format!("c:{:04}", i % 800), &Value::Doc(doc), 0, false).unwrap();
            }
        })
    };
    let q = Query {
        pattern: "c:*".into(),
        expr: Some(Expr::from_json(&json!({"field": "v", "cmp": "<", "value": 250}), "c:*").unwrap()),
    };
    let mut runs = 0;
    while !writer.is_finished() || runs < 20 {
        for (field, desc) in [("w", true), ("v", false)] {
            let opts = ReadOpts { limit: 25, order: Some(Order::new(field, desc).unwrap()), ..ReadOpts::default() };
            let mut rows = Vec::new();
            let (_, m) = e
                .read_query(&q, &opts, |k, d| {
                    rows.push((k, d));
                    Ok(())
                })
                .unwrap();
            assert_eq!(m.returned as usize, rows.len());
            for (_, d) in &rows {
                assert!(d["v"].as_u64().unwrap() < 250);
            }
            for pair in rows.windows(2) {
                assert_ne!(reference_cmp(&pair[0], &pair[1], field, desc), Ordering::Greater, "{pair:?}");
            }
        }
        runs += 1;
    }
    writer.join().unwrap();
}

#[test]
fn dates_times_and_datetimes_compare_as_moments_with_or_without_an_index() {
    let dir = TempDir::new("queries_time");
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    let rows = [
        ("e:1", r#"{"day": "2026-10-26", "at": "2026-10-27T12:00:00Z", "opens": "08:00"}"#),
        ("e:2", r#"{"day": "2026-10-27", "at": "2026-10-27T13:00:00+01:00", "opens": "08:00:00"}"#),
        ("e:3", r#"{"day": "2026-10-28", "at": "2026-10-27T11:59:59.999-00:00", "opens": "07:59:59"}"#),
        ("e:4", r#"{"day": "2026-10-27", "at": "2026-10-28T00:30:00+14:00", "opens": "23:00"}"#),
        ("e:5", r#"{"day": null, "at": null, "opens": null}"#),
        ("e:6", r#"{"name": "no dates"}"#),
    ];
    for (k, v) in rows {
        ok(&mut s, &format!("set {k} {v}"));
    }
    let cases = [
        // 12:00Z, 13:00+01:00 are the same instant; 11:59:59.999 is before.
        (r#"e:*.at = datetime "2026-10-27T12:00:00Z""#, "e:1 e:2"),
        (r#"e:*.at >= datetime "2026-10-27T12:00:00Z""#, "e:1 e:2"),
        (r#"e:*.at > datetime "2026-10-27T12:00:00Z""#, ""),
        (r#"e:*.at < datetime "2026-10-27T12:00:00Z""#, "e:3 e:4"),
        (r#"e:*.at <= datetime "2026-10-27T11:59:59.999Z""#, "e:3 e:4"),
        (r#"e:*.day >= "2026-10-27" and day < "2026-10-28""#, "e:2 e:4"),
        (r#"e:*.opens >= time "08:00""#, "e:1 e:2 e:4"),
        (r#"e:*.opens < time "08:00""#, "e:3"),
    ];
    let keys = |out: &str| {
        out.lines()
            .filter(|l| l.starts_with("e:"))
            .map(|l| l.split(':').take(2).collect::<Vec<_>>().join(":"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut before = Vec::new();
    for (cond, want) in cases {
        let got = keys(&ok(&mut s, &format!("get where {cond}")));
        assert_eq!(got, want, "{cond}");
        before.push(got);
    }
    let orders = [
        ("order by at", "e:4 e:3 e:1 e:2 e:5 e:6"),
        ("order by at desc", "e:1 e:2 e:3 e:4 e:5 e:6"),
        ("order by opens", "e:3 e:1 e:2 e:4 e:5 e:6"),
        ("order by opens desc", "e:4 e:1 e:2 e:3 e:5 e:6"),
        ("order by day desc limit 3", "e:3 e:2 e:4"),
    ];
    let all = r#"get where not e:*.name = "x" "#;
    let mut sorted = Vec::new();
    for (order, want) in orders {
        let got = keys(&ok(&mut s, &format!("{all}{order}")));
        assert_eq!(got, want, "{order}");
        sorted.push(got);
    }
    // The same answers through B-tree indexes, and a unique index on a room
    // and a datetime.
    for f in ["at", "day", "opens"] {
        ok(&mut s, &format!("create index on e:* ({f})"));
    }
    for ((cond, _), want) in cases.iter().zip(&before) {
        assert_eq!(keys(&ok(&mut s, &format!("get where {cond}"))), *want, "{cond} with an index");
    }
    for ((order, _), want) in orders.iter().zip(&sorted) {
        assert_eq!(keys(&ok(&mut s, &format!("{all}{order}"))), *want, "{order} with an index");
    }
    ok(&mut s, r#"set r:1 {"room": "A", "start": "2026-10-27T09:00:00+01:00"}"#);
    ok(&mut s, r#"set r:2 {"room": "A", "start": "2026-10-27T10:00:00+01:00"}"#);
    ok(&mut s, "create unique index on r:* (room, start)");
    let plan = ok(&mut s, r#"explain analyze get where r:*.room = "A" and start = "2026-10-27T10:00:00+01:00""#);
    assert!(plan.contains("r:* (room, start)") && plan.contains("Returned      : 1"), "{plan}");
    let moment = ok(&mut s, r#"get where r:*.room = "A" and start = datetime "2026-10-27T08:00:00Z""#);
    assert!(moment.starts_with("r:1:") && moment.ends_with("(1 matches)"), "{moment}");
}

#[test]
fn a_home_of_1_7_1_is_queried_sorted_and_explained_without_changing_its_format() {
    let dir = TempDir::new("queries_compat");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("v1.7.1");
    let home = dir.path().join("home");
    copy_dir(&fixture, &home);
    let conf = home.join("kiva.conf");
    let text = std::fs::read_to_string(&conf).unwrap();
    std::fs::write(&conf, text.replace("@HOME@", &home.display().to_string().replace('\\', "/"))).unwrap();
    let data = home.join("data").join("store.kiva");
    let original = std::fs::read(&data).unwrap();
    let inst = Instance::open(&home, Default::default()).unwrap();
    let mut s = Session::new(inst.clone(), true, None);
    let a = ok(&mut s, r#"explain analyze get where user:*.role = "dev" order by name desc"#);
    assert!(a.contains("Index     : user:* (role)") && a.contains("Returned      : 2"), "{a}");
    let rows = ok(&mut s, r#"get where user:*.role = "dev" order by name desc"#);
    let names: Vec<&str> =
        rows.lines().filter_map(|l| l.split("\"name\":\"").nth(1)).map(|n| n.split('"').next().unwrap()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.reverse();
    assert_eq!(names, sorted);
    drop(s);
    drop(inst);
    assert_eq!(std::fs::read(&data).unwrap(), original, "reading changed the 1.7.1 data file");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}
