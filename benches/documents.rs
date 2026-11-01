// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Documents in JSON, MessagePack and `KDOC`: `cargo bench --bench documents`.
//!
//! The same orders written three ways: their decimals, dates and times as
//! texts in JSON and in MessagePack, and as native values in `KDOC`. Measured:
//! the bytes of a payload, encoding and decoding it, and an engine writing,
//! reading, reopening (indexes rebuilt) and compacting 20 000 of them
//! (`KIVADB_BENCH_N`). Each timing is the median of 7 runs after 2 to warm
//! up. The numbers hold for the machine they ran on only.

use kivadb::doc::{self, Doc};
use kivadb::engine::{Engine, EngineOptions};
use kivadb::value::{DocEnc, Type, Value};
use serde_json::json;
use std::time::{Duration, Instant};

/// An order with a decimal total, dates, a datetime, a time and lines.
fn order(i: u64) -> Doc {
    let day = 1 + i % 28;
    let city = ["Goma", "Bukavu", "Kinshasa", "Lubumbashi"][(i % 4) as usize];
    doc::from_extended_json(&json!({
        "id": i,
        "customer": format!("customer-{}", i % 997),
        "city": city,
        "total": {"$decimal": format!("{}.{:02}", 100 + i % 9000, i % 100)},
        "day": {"$date": format!("2026-10-{day:02}")},
        "at": {"$datetime": format!("2026-10-{day:02}T{:02}:{:02}:00.000Z", i % 24, i % 60)},
        "opens": {"$time": "08:30:00"},
        "paid": i.is_multiple_of(3),
        "lines": (0..3).map(|n| json!({
            "sku": format!("sku-{}", (i + n) % 501),
            "qty": 1 + n,
            "price": {"$decimal": format!("{}.{:02}", 1 + (i + n) % 90, (i * 7 + n) % 100)},
        })).collect::<Vec<_>>(),
    }))
    .unwrap()
}

/// The three ways to store it.
fn ways(d: &Doc) -> [(&'static str, Value, bool); 3] {
    let texts = Value::Doc(doc::to_plain_json(d));
    [("JSON", texts.clone(), false), ("MessagePack", texts, true), ("KDOC", Value::typed(d.clone()).unwrap(), false)]
}

fn median(mut f: impl FnMut()) -> Duration {
    for _ in 0..2 {
        f();
    }
    let mut runs: Vec<Duration> = (0..7)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    runs.sort();
    runs[3]
}

fn per_doc(d: Duration, n: usize) -> String {
    format!("{:.2} µs", d.as_secs_f64() * 1e6 / n as f64)
}

fn main() {
    let n: usize = std::env::var("KIVADB_BENCH_N").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let docs: Vec<Doc> = (0..n as u64).map(order).collect();

    println!("## Payloads ({n} orders)\n");
    println!("| Encoding | Bytes per order | Encode | Decode |");
    println!("| :--- | ---: | ---: | ---: |");
    for (w, (name, _, msgpack)) in ways(&docs[0]).iter().enumerate() {
        let values: Vec<Value> = docs.iter().map(|d| ways(d)[w].1.clone()).collect();
        let encoded: Vec<(Vec<u8>, DocEnc)> = values.iter().map(|v| v.encode_stored(*msgpack).unwrap()).collect();
        let bytes: usize = encoded.iter().map(|(b, _)| b.len()).sum();
        let enc = median(|| {
            for v in &values {
                std::hint::black_box(v.encode_stored(*msgpack).unwrap());
            }
        });
        let dec = median(|| {
            for (b, e) in &encoded {
                std::hint::black_box(Value::decode(Type::Object, b, *e).unwrap());
            }
        });
        println!("| {name} | {:.0} | {} | {} |", bytes as f64 / n as f64, per_doc(enc, n), per_doc(dec, n));
    }

    println!("\n## Engine ({n} orders, one index on `city`)\n");
    println!("| Encoding | Write all | Read all | Reopen | Compact | File after compaction |");
    println!("| :--- | ---: | ---: | ---: | ---: | ---: |");
    for (w, (name, _, msgpack)) in ways(&docs[0]).iter().enumerate() {
        let values: Vec<Value> = docs.iter().map(|d| ways(d)[w].1.clone()).collect();
        let dir = std::env::temp_dir().join(format!("kivadb_bench_docs_{}_{w}", std::process::id()));
        let path = dir.join("store.kiva");
        let fresh = || {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
        };
        let write = median(|| {
            fresh();
            let e = Engine::open(&path, EngineOptions::default()).unwrap();
            for (i, v) in values.iter().enumerate() {
                e.put(&format!("o:{i}"), v, 0, *msgpack).unwrap();
            }
        });
        let e = Engine::open(&path, EngineOptions::default()).unwrap();
        e.set_cache_limit(0);
        e.create_index("o:*", "city").unwrap();
        let read = median(|| {
            for i in 0..n {
                std::hint::black_box(e.get(&format!("o:{i}")).unwrap());
            }
        });
        drop(e);
        let reopen = median(|| {
            let _ = std::fs::remove_file(dir.join("store.kiva.hint"));
            drop(Engine::open(&path, EngineOptions::default()).unwrap());
        });
        let e = Engine::open(&path, EngineOptions::default()).unwrap();
        let compact = median(|| {
            e.compact().unwrap();
        });
        drop(e);
        let size = std::fs::metadata(&path).unwrap().len();
        println!(
            "| {name} | {:.0} ms | {:.0} ms | {:.0} ms | {:.0} ms | {:.1} MB |",
            write.as_secs_f64() * 1e3,
            read.as_secs_f64() * 1e3,
            reopen.as_secs_f64() * 1e3,
            compact.as_secs_f64() * 1e3,
            size as f64 / 1e6
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
