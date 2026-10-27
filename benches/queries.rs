// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Microbenchmarks of the query planner: `cargo bench --bench queries`.
//!
//! 50 000 documents, one engine, the value cache on (its default size), so
//! that what is timed is the plan and the reading, not the disk. Each case
//! runs 5 times to warm up, then 40 times; the median and the 90th
//! percentile are printed. The cases that sort ask for the first 20 rows:
//! [`first_in_order`] is the only part that needs `order by`, so that the
//! same file measures a version without it (reading every match and sorting
//! them in the program) by replacing that one function.

use kivadb::engine::{Engine, EngineOptions, IndexKind};
use kivadb::query::{Expr, Query};
use kivadb::value::Value;
use serde_json::{json, Value as Json};
use std::time::{Duration, Instant};

const DOCS: u64 = 50_000;
const OFFSETS: [&str; 4] = ["Z", "+01:00", "-05:00", "+05:30"];

fn query(w: Json) -> Query {
    Query { pattern: "k:*".into(), expr: Some(Expr::from_json(&w, "k:*").unwrap()) }
}

/// The first `limit` documents of `q` by `field`, descending or not.
fn first_in_order(e: &Engine, q: &Query, field: &str, desc: bool, limit: usize) -> usize {
    let opts = kivadb::engine::ReadOpts {
        limit,
        order: Some(kivadb::engine::Order::new(field, desc).unwrap()),
        ..Default::default()
    };
    let mut n = 0;
    e.read_query(q, &opts, |_, _| {
        n += 1;
        Ok(())
    })
    .unwrap();
    n
}

fn time(name: &str, docs: &str, mut f: impl FnMut() -> usize) {
    let mut rows = 0;
    for _ in 0..5 {
        rows = f();
    }
    let mut runs: Vec<Duration> = (0..40)
        .map(|_| {
            let t = Instant::now();
            rows = f();
            t.elapsed()
        })
        .collect();
    runs.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let median = ms(runs[runs.len() / 2]);
    println!(
        "| {name} | {docs} | {rows} | {median:.3} | {:.3} | {:.0} |",
        ms(runs[runs.len() * 9 / 10]),
        1000.0 / median
    );
}

fn main() {
    let dir = std::env::temp_dir().join(format!("kivadb_bench_queries_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let opts = EngineOptions { background: false, auto_compact: false, ..Default::default() };
    let e = Engine::open(dir.join("bench.kiva"), opts).unwrap();
    for i in 0..DOCS {
        let off = OFFSETS[(i % 4) as usize];
        let doc = json!({
            "num": i, "grp": i % 100, "x": i % 3,
            "t": format!("2026-{:02}-{:02}T{:02}:{:02}:00{off}", 1 + i % 12, 1 + i % 28, i % 24, i % 60),
        });
        e.put(&format!("k:{i:06}"), &Value::Doc(doc), 0, false).unwrap();
    }
    e.create_index("k:*", "num").unwrap();
    e.create_index("k:*", "t").unwrap();
    e.create_index_with("k:*", "grp", IndexKind::Hash).unwrap();
    e.create_index_on("k:*", &["grp", "num"], IndexKind::BTree, true).unwrap();

    println!("| Case | Query | Rows | Median (ms) | p90 (ms) | Queries/s |");
    println!("| :--- | :--- | ---: | ---: | ---: | ---: |");
    let q = query(json!({"x": 1}));
    time("limit, no index", "x = 1 limit 10", || e.query_expr(&q, 10).unwrap().0.len());
    let q = query(json!([{"grp": 7}, {"field": "num", "cmp": "<", "value": 25_000}]));
    time("and, two indexes", "grp = 7 and num < 25000", || e.query_expr(&q, 0).unwrap().0.len());
    let q = query(json!({"grp": 7, "num": 707}));
    time("and, unique index on both", "grp = 7 and num = 707", || e.query_expr(&q, 0).unwrap().0.len());
    let q = query(json!({"field": "num", "cmp": ">=", "value": 0}));
    time("order, broad filter", "num >= 0 order by t desc limit 20", || first_in_order(&e, &q, "t", true, 20));
    let q = query(json!({"grp": 7}));
    time("order, selective filter", "grp = 7 order by t limit 20", || first_in_order(&e, &q, "t", false, 20));
    let q = query(json!({"field": "num", "cmp": ">=", "value": 49_000}));
    time("order by the filtered field", "num >= 49000 order by num desc limit 20", || {
        first_in_order(&e, &q, "num", true, 20)
    });
    // Unchanged plans, to see what the new code costs them.
    let q = query(json!({"num": 12_345}));
    time("one index, one key", "num = 12345", || e.query_expr(&q, 0).unwrap().0.len());
    let q = query(json!({"x": 1}));
    time("no index, every match", "x = 1", || e.query_expr(&q, 0).unwrap().0.len());
    let q = query(json!({"field": "num", "cmp": "<", "value": 5_000}));
    time("one index, a range", "num < 5000", || e.query_expr(&q, 0).unwrap().0.len());

    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
}
