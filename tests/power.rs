//! Value cache, vector search, Redis compatibility and Server-Sent Events.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::engine::{AutoTune, Engine, EngineOptions, VectorIndexKind, VectorQuery};
use kivadb::hnsw::Params as HnswParams;
use kivadb::session::Session;
use kivadb::value::Value;
use kivadb::vector::Metric;
use kivadb::{http, resp, server};
use serde_json::{json, Value as Json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn quiet(cache_bytes: usize) -> EngineOptions {
    EngineOptions { background: false, auto_compact: false, cache_bytes, ..Default::default() }
}

// ---------------------------------------------------------------------------
// Value cache
// ---------------------------------------------------------------------------

#[test]
fn the_value_cache_serves_hot_keys_and_never_returns_stale_data() {
    let dir = TempDir::new("cache");
    let e = Engine::open(dir.path().join("store.kiva"), quiet(1024 * 1024)).unwrap();
    e.put("hot", &Value::Str("v1".into()), 0, false).unwrap();
    for _ in 0..10 {
        assert_eq!(e.get("hot").unwrap().unwrap().0, Value::Str("v1".into()));
    }
    let s = e.stats();
    assert_eq!((s.cache_misses, s.cache_hits, s.cache_entries), (1, 9, 1));

    // Writes, deletes, transactions and resets invalidate the copy.
    e.put("hot", &Value::Str("v2".into()), 0, false).unwrap();
    assert_eq!(e.get("hot").unwrap().unwrap().0, Value::Str("v2".into()));
    e.apply_batch(
        vec![kivadb::engine::BatchOp::Put {
            key: "hot".into(),
            value: Value::Str("v3".into()),
            expires_at: 0,
            msgpack: false,
        }],
        None,
    )
    .unwrap();
    assert_eq!(e.get("hot").unwrap().unwrap().0, Value::Str("v3".into()));
    e.delete("hot").unwrap();
    assert!(e.get("hot").unwrap().is_none());
    e.put("hot", &Value::Str("v4".into()), 0, false).unwrap();
    e.get("hot").unwrap();
    e.reset().unwrap();
    assert!(e.get("hot").unwrap().is_none());
    assert_eq!(e.stats().cache_entries, 0);

    // Compaction moves records but keeps cached values valid.
    for i in 0..50 {
        e.put(&format!("k{i}"), &Value::Num(i.to_string()), 0, false).unwrap();
        e.put(&format!("k{i}"), &Value::Num((i * 2).to_string()), 0, false).unwrap();
        e.get(&format!("k{i}")).unwrap();
    }
    e.compact().unwrap();
    for i in 0..50 {
        assert_eq!(e.get(&format!("k{i}")).unwrap().unwrap().0, Value::Num((i * 2).to_string()));
    }
}

#[test]
fn the_value_cache_stays_within_its_budget() {
    let dir = TempDir::new("cache_budget");
    let e = Engine::open(dir.path().join("store.kiva"), quiet(64 * 1024)).unwrap();
    let big = Value::Str("x".repeat(1000));
    for i in 0..500 {
        e.put(&format!("k{i:03}"), &big, 0, false).unwrap();
        e.get(&format!("k{i:03}")).unwrap();
    }
    let s = e.stats();
    assert!(s.cache_bytes <= 64 * 1024, "cache uses {} bytes", s.cache_bytes);
    assert!(s.cache_entries > 10 && s.cache_entries < 500);
    // Everything is still readable, cached or not.
    for i in 0..500 {
        assert_eq!(e.get(&format!("k{i:03}")).unwrap().unwrap().0, big);
    }
    e.set_cache_limit(0);
    assert_eq!(e.stats().cache_entries, 0);
}

#[test]
fn the_cache_is_configured_from_the_shell() {
    let dir = TempDir::new("cache_cfg");
    let mut s = Session::new(instance(&dir), true, None);
    assert!(ok(&mut s, "stats").contains("Value Cache          : off"));
    ok(&mut s, "config set cache_values true");
    ok(&mut s, "config set cache_size_mb 8");
    ok(&mut s, r#"set k "v""#);
    ok(&mut s, "get k and k and k");
    let stats = ok(&mut s, "stats");
    assert!(stats.contains("of 8.00 MB used, 1 values, 66.7% hits"), "{stats}");
}

// ---------------------------------------------------------------------------
// Vector search
// ---------------------------------------------------------------------------

fn embed(e: &Engine, key: &str, v: &[f32], extra: Json) {
    let mut doc = json!({ "embedding": v });
    for (k, x) in extra.as_object().unwrap() {
        doc[k] = x.clone();
    }
    e.put(key, &Value::Doc(doc), 0, false).unwrap();
}

fn query(pattern: &str, v: Vec<f32>, metric: Metric, limit: usize) -> VectorQuery {
    VectorQuery::new(pattern, "embedding", v, metric, limit)
}

#[test]
fn similarity_search_with_and_without_an_index() {
    let dir = TempDir::new("vectors");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet(0)).unwrap();
    embed(&e, "doc:cat", &[1.0, 0.1, 0.0], json!({"text": "cat"}));
    embed(&e, "doc:lion", &[0.9, 0.3, 0.0], json!({"text": "lion"}));
    embed(&e, "doc:car", &[0.0, 0.1, 1.0], json!({"text": "car"}));
    embed(&e, "doc:odd", &[1.0, 2.0], json!({"text": "wrong length"}));
    embed(&e, "other:cat", &[1.0, 0.1, 0.0], json!({}));
    e.put("doc:text", &Value::Str("not a document".into()), 0, false).unwrap();

    let keys = |r: &kivadb::engine::VectorResult| r.rows.iter().map(|(k, _, _)| k.clone()).collect::<Vec<_>>();
    let r = e.similar(&query("doc:*", vec![1.0, 0.0, 0.0], Metric::Cosine, 2)).unwrap();
    assert_eq!(keys(&r), ["doc:cat", "doc:lion"]);
    assert!(!r.indexed);
    assert_eq!(r.compared, 3, "the 2-dimension vector is skipped");
    assert!(r.rows[0].1 > 0.99 && r.rows[0].1 <= 1.0);
    assert_eq!(r.rows[0].2["text"], "cat");

    let far = e.similar(&query("doc:*", vec![0.0, 0.0, 1.0], Metric::L2, 1)).unwrap();
    assert_eq!(keys(&far), ["doc:car"]);
    let dot = e.similar(&query("doc:*", vec![0.0, 1.0, 0.0], Metric::Dot, 1)).unwrap();
    assert_eq!(keys(&dot), ["doc:lion"]);

    // The index gives the same answers, from memory, and follows every change.
    assert_eq!(e.create_vector_index("doc:*", "embedding").unwrap(), 3);
    let r = e.similar(&query("doc:*", vec![1.0, 0.0, 0.0], Metric::Cosine, 2)).unwrap();
    assert!(r.indexed);
    assert_eq!(keys(&r), ["doc:cat", "doc:lion"]);
    let info = &e.list_vector_indexes()[0];
    assert_eq!(
        (info.pattern.as_str(), info.field.as_str(), info.vectors, info.dims, info.rejected),
        ("doc:*", "embedding", 3, Some(3), 1)
    );
    assert!(e.similar(&query("doc:*", vec![1.0, 0.0], Metric::Cosine, 2)).is_err());

    e.delete("doc:cat").unwrap();
    embed(&e, "doc:tiger", &[0.95, 0.2, 0.0], json!({"text": "tiger"}));
    embed(&e, "doc:lion", &[0.0, 0.0, 1.0], json!({"text": "lion, moved"}));
    let r = e.similar(&query("doc:*", vec![1.0, 0.0, 0.0], Metric::Cosine, 1)).unwrap();
    assert_eq!(keys(&r), ["doc:tiger"]);

    // Index definitions survive a restart and are rebuilt.
    drop(e);
    let e = Engine::open(&path, quiet(0)).unwrap();
    let r = e.similar(&query("doc:*", vec![1.0, 0.0, 0.0], Metric::Cosine, 3)).unwrap();
    assert!(r.indexed);
    assert_eq!(keys(&r)[0], "doc:tiger");
    assert!(e.drop_vector_index("doc:*", "embedding").unwrap());
    assert!(!e.similar(&query("doc:*", vec![1.0, 0.0, 0.0], Metric::Cosine, 3)).unwrap().indexed);
}

#[test]
fn vector_search_scales_to_thousands_of_embeddings() {
    let dir = TempDir::new("vectors_many");
    let e = Engine::open(dir.path().join("store.kiva"), quiet(0)).unwrap();
    let dims = 384;
    // Deterministic pseudo-random vectors.
    let mut seed = 42u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f32 / u32::MAX as f32) - 0.25
    };
    let mut target = Vec::new();
    for i in 0..3000 {
        let v: Vec<f32> = (0..dims).map(|_| next()).collect();
        if i == 1234 {
            target = v.clone();
        }
        embed(&e, &format!("chunk:{i:05}"), &v, json!({"n": i}));
    }
    e.create_vector_index("chunk:*", "embedding").unwrap();
    let start = std::time::Instant::now();
    let r = e.similar(&query("chunk:*", target, Metric::Cosine, 5)).unwrap();
    let took = start.elapsed();
    assert_eq!(r.rows[0].0, "chunk:01234");
    assert!((r.rows[0].1 - 1.0).abs() < 1e-4);
    assert_eq!(r.compared, 3000);
    assert!(took < Duration::from_secs(2), "search took {took:?}");
}

/// Deterministic pseudo-random vectors.
fn random_vectors(n: usize, dims: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            (0..dims)
                .map(|_| {
                    s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    ((s >> 33) as f32 / u32::MAX as f32) - 0.25
                })
                .collect()
        })
        .collect()
}

fn hnsw(metric: Metric) -> VectorIndexKind {
    VectorIndexKind::Hnsw(HnswParams { metric, ..Default::default() })
}

/// Share of the exact top-k that the approximate search found.
fn recall(e: &Engine, queries: &[Vec<f32>], k: usize, ef: usize) -> f64 {
    let mut hits = 0;
    for q in queries {
        let mut exact = query("chunk:*", q.clone(), Metric::Cosine, k);
        exact.exact = true;
        let truth: Vec<String> = e.similar(&exact).unwrap().rows.into_iter().map(|r| r.0).collect();
        let mut vq = query("chunk:*", q.clone(), Metric::Cosine, k);
        vq.ef = ef;
        let r = e.similar(&vq).unwrap();
        assert!(r.approximate);
        hits += r.rows.iter().filter(|row| truth.contains(&row.0)).count();
    }
    hits as f64 / (queries.len() * k) as f64
}

#[test]
fn hnsw_index_finds_nearly_the_same_neighbours_comparing_far_fewer_vectors() {
    let dir = TempDir::new("hnsw");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet(0)).unwrap();
    let n = 3000;
    let vectors = random_vectors(n, 32, 11);
    for (i, v) in vectors.iter().enumerate() {
        embed(&e, &format!("chunk:{i:05}"), v, json!({ "n": i }));
    }
    assert_eq!(e.create_vector_index_with("chunk:*", "embedding", hnsw(Metric::Cosine)).unwrap(), n);
    let info = &e.list_vector_indexes()[0];
    assert_eq!((info.vectors, info.dims, info.kind), (n, Some(32), hnsw(Metric::Cosine)));

    // Accuracy and work, against the exact search on the same index.
    let queries = random_vectors(30, 32, 77);
    let r = recall(&e, &queries, 10, 0);
    assert!(r >= 0.95, "recall@10 is {r}");
    let approx = e.similar(&query("chunk:*", queries[0].clone(), Metric::Cosine, 10)).unwrap();
    let mut full = query("chunk:*", queries[0].clone(), Metric::Cosine, 10);
    full.exact = true;
    let exact = e.similar(&full).unwrap();
    assert!((approx.indexed, approx.approximate, exact.approximate) == (true, true, false));
    assert_eq!(exact.compared, n);
    // On so few vectors the gain is modest; it grows with the size (see hnsw_speed).
    assert!(approx.compared < n / 2, "HNSW compared {} of {n} vectors", approx.compared);
    // A stored vector is its own nearest neighbour.
    let me = e.similar(&query("chunk:*", vectors[1234].clone(), Metric::Cosine, 1)).unwrap();
    assert_eq!(me.rows[0].0, "chunk:01234");

    // Another metric, or a narrower pattern, compares every vector exactly.
    let l2 = e.similar(&query("chunk:*", queries[0].clone(), Metric::L2, 5)).unwrap();
    assert!(l2.indexed && !l2.approximate);
    let narrow = e.similar(&query("chunk:001*", queries[0].clone(), Metric::Cosine, 5)).unwrap();
    assert!(!narrow.approximate && narrow.rows.iter().all(|r| r.0.starts_with("chunk:001")));

    // Writes, replacements and deletions show at once.
    e.delete("chunk:01234").unwrap();
    embed(&e, "chunk:00007", &vectors[1234], json!({ "moved": true }));
    let r = e.similar(&query("chunk:*", vectors[1234].clone(), Metric::Cosine, 2)).unwrap();
    assert_eq!(r.rows[0].0, "chunk:00007");
    assert!(r.rows.iter().all(|row| row.0 != "chunk:01234"));
    e.put("chunk:expired", &Value::Doc(json!({ "embedding": vectors[1234] })), 1, false).unwrap();
    assert!(e
        .similar(&query("chunk:*", vectors[1234].clone(), Metric::Cosine, 3))
        .unwrap()
        .rows
        .iter()
        .all(|r| r.0 != "chunk:expired"));

    // The graph is saved with the index; writes made after that are replayed
    // on the next start. A copy taken now has a graph older than its data.
    assert!(dir.path().join("store.kiva.graphs").exists());
    e.close();
    drop(e);
    let e = Engine::open(&path, quiet(0)).unwrap();
    let copy = dir.path().join("copy");
    std::fs::create_dir_all(&copy).unwrap();
    for ext in ["", ".meta", ".graphs"] {
        std::fs::copy(dir.path().join(format!("store.kiva{ext}")), copy.join(format!("store.kiva{ext}"))).unwrap();
    }
    e.delete("chunk:00007").unwrap();
    embed(&e, "chunk:new", &vectors[99], json!({}));
    e.close();
    drop(e);
    std::fs::copy(&path, copy.join("store.kiva")).unwrap();
    let e = Engine::open(copy.join("store.kiva"), quiet(0)).unwrap();
    let r = e.similar(&query("chunk:*", vectors[99].clone(), Metric::Cosine, 2)).unwrap();
    assert!(r.approximate);
    let keys: Vec<&str> = r.rows.iter().map(|r| r.0.as_str()).collect();
    assert!(keys.contains(&"chunk:new") && keys.contains(&"chunk:00099"), "{keys:?}");
    let r = e.similar(&query("chunk:*", vectors[1234].clone(), Metric::Cosine, 3)).unwrap();
    assert!(r.rows.iter().all(|row| row.0 != "chunk:00007" && row.0 != "chunk:01234"));
    assert_eq!(e.list_vector_indexes()[0].vectors, n - 1);

    // A damaged graphs file is ignored: the graph is built again.
    e.close();
    drop(e);
    std::fs::write(copy.join("store.kiva.graphs"), b"garbage").unwrap();
    let e = Engine::open(copy.join("store.kiva"), quiet(0)).unwrap();
    assert!(recall(&e, &queries[..5], 10, 0) >= 0.9);
    assert!(e.drop_vector_index("chunk:*", "embedding").unwrap());
    assert!(!copy.join("store.kiva.graphs").exists());
}

/// Speed on a larger set. Run with:
/// cargo test --release --test power hnsw_speed -- --ignored --nocapture
#[test]
fn tuning_an_hnsw_index_keeps_the_smallest_ef_search_that_reaches_the_recall() {
    let dir = TempDir::new("hnsw_tune");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet(0)).unwrap();
    // Evenly spread vectors, the hardest case for a graph: a small ef misses.
    for (i, v) in random_vectors(3000, 16, 33).iter().enumerate() {
        embed(&e, &format!("chunk:{i:05}"), v, json!({}));
    }
    let small = VectorIndexKind::Hnsw(HnswParams { m: 4, ef_construction: 20, ..Default::default() });
    e.create_vector_index_with("chunk:*", "embedding", small).unwrap();
    assert_eq!(e.list_vector_indexes()[0].ef_search, 100, "the default");

    let r = e.tune_vector_index("chunk:*", "embedding", 0.9, 10, 40, None).unwrap();
    assert_eq!((r.queries, r.vectors, r.from_data), (40, 3000, true));
    assert!(r.steps.len() >= 2, "{:?}", r.steps);
    assert_eq!(r.steps[0].ef, 10);
    assert!(r.reached && r.steps.last().unwrap().recall >= 0.9, "{:?}", r.steps);
    // Smaller values were tried first and missed too much.
    assert!(r.steps[..r.steps.len() - 1].iter().all(|s| s.recall < 0.9));
    assert!(r.steps.first().unwrap().compared < r.steps.last().unwrap().compared);
    assert_eq!(e.list_vector_indexes()[0].ef_search, r.chosen);

    // Searches without ef use the tuned value; an ef of their own wins.
    let q = random_vectors(1, 16, 77).remove(0);
    let compared = |ef: usize| {
        let mut vq = query("chunk:*", q.clone(), Metric::Cosine, 10);
        vq.ef = ef;
        e.similar(&vq).unwrap().compared
    };
    assert_eq!(compared(0), compared(r.chosen));
    assert!(compared(10) < compared(0));

    // The value is saved with the index, and the graph read back from disk.
    drop(e);
    let e = Engine::open(&path, quiet(0)).unwrap();
    assert_eq!(e.list_vector_indexes()[0].ef_search, r.chosen);
    e.set_vector_ef_search("chunk:*", "embedding", 0).unwrap();
    assert_eq!(e.list_vector_indexes()[0].ef_search, 100);

    // An unreachable recall keeps the largest value tried, and says so.
    let r = e.tune_vector_index("chunk:*", "embedding", 1.0, 10, 5, None).unwrap();
    assert_eq!(r.reached, r.steps.last().unwrap().recall >= 1.0);
    assert!(r.reached || r.chosen == r.steps.iter().map(|s| s.ef).max().unwrap());

    // Nothing to tune on an exact index, nor without an index.
    e.create_vector_index("other:*", "embedding").unwrap();
    let err = e.tune_vector_index("other:*", "embedding", 0.9, 10, 10, None).unwrap_err().to_string();
    assert!(err.contains("nothing to tune"), "{err}");
    assert!(e.set_vector_ef_search("none:*", "embedding", 50).is_err());
}

#[test]
fn hnsw_tuning_from_the_shell() {
    let dir = TempDir::new("hnsw_tune_shell");
    let mut s = Session::new(instance(&dir), true, None);
    for (i, v) in random_vectors(300, 8, 5).iter().enumerate() {
        ok(&mut s, &format!("set doc:{i:03} {}", json!({ "embedding": v })));
    }
    let out = ok(&mut s, "create vector index on doc: (embedding) using hnsw m 8 ef 50 ef_search 40");
    assert!(out.contains("HNSW vector index created"), "{out}");
    assert!(ok(&mut s, "indexes").contains("(cosine, m 8, ef 50, ef_search 40)"));

    let out = ok(&mut s, "tune vector index on doc: (embedding) recall 0.9 limit 5 sample 30");
    assert!(
        out.contains(
            "300 vectors: 30 queries taken from its vectors, each searched without its own node, 5 closest each"
        ),
        "{out}"
    );
    assert!(out.contains("exact search") && out.contains("ef_search 10"), "{out}");
    assert!(out.contains("Kept ef_search"), "{out}");
    let kept: usize =
        out.split("Kept ef_search ").nth(1).unwrap().split([' ', ',', ':']).next().unwrap().parse().unwrap();
    assert!(ok(&mut s, "indexes").contains(&format!("ef_search {kept})")));

    // Real queries kept by the application, in documents of their own.
    for (i, v) in random_vectors(12, 8, 6).iter().enumerate() {
        ok(&mut s, &format!("set query:{i:02} {}", json!({ "embedding": v })));
    }
    let out = ok(&mut s, r#"tune vector index on doc: (embedding) limit 5 queries "query:*""#);
    assert!(out.contains("12 queries read from query:*"), "{out}");
    ok(&mut s, r#"set query:bad {"embedding": [1, 2]}"#);
    assert!(
        fails(&mut s, r#"tune vector index on doc: (embedding) queries "nothing:*""#).contains("no document matching")
    );

    let out = ok(&mut s, "tune vector index on doc: (embedding) ef_search default");
    assert!(out.contains("the default, 100"), "{out}");
    ok(&mut s, "tune vector index on doc: (embedding) ef_search 250");
    assert!(ok(&mut s, "indexes").contains("ef_search 250)"));
    assert!(fails(&mut s, "tune vector index on doc: (embedding) ef_search 0").contains("between 1 and 100000"));
    assert!(fails(&mut s, "tune vector index on doc: (embedding) recall 2").contains("at most 1"));
    assert!(fails(&mut s, "tune vector index on ghost: (embedding)").contains("no vector index"));
    assert!(fails(&mut s, "tune index on doc: (embedding)").contains("Usage"));
}

#[test]
#[ignore]
fn hnsw_speed() {
    let n: usize = std::env::var("HNSW_N").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let dims: usize = std::env::var("HNSW_DIMS").ok().and_then(|v| v.parse().ok()).unwrap_or(128);
    let dir = TempDir::new("hnsw_speed");
    let e = Engine::open(dir.path().join("store.kiva"), quiet(0)).unwrap();
    // Embeddings gather around topics: points around 200 random centres.
    // HNSW_DATA=uniform spreads them evenly instead, the hardest case.
    let uniform = std::env::var("HNSW_DATA").is_ok_and(|d| d == "uniform");
    let centres = random_vectors(200, dims, 8);
    let noise = random_vectors(n + 100, dims, 5);
    let around = |i: usize| -> Vec<f32> {
        if uniform {
            return noise[i].clone();
        }
        centres[i % 200].iter().zip(&noise[i]).map(|(c, x)| 2.0 * c + (x - 0.25) * 0.8).collect()
    };
    let vectors: Vec<Vec<f32>> = (0..n).map(around).collect();
    for (i, v) in vectors.iter().enumerate() {
        embed(&e, &format!("chunk:{i:07}"), v, json!({}));
    }
    let start = std::time::Instant::now();
    e.create_vector_index_with("chunk:*", "embedding", hnsw(Metric::Cosine)).unwrap();
    println!("{n} vectors of {dims} dimensions: graph built in {:?}", start.elapsed());
    let queries: Vec<Vec<f32>> = (n..n + 100).map(around).collect();
    let time = |exact: bool| {
        let start = std::time::Instant::now();
        let mut compared = 0;
        for q in &queries {
            let mut vq = query("chunk:*", q.clone(), Metric::Cosine, 10);
            vq.exact = exact;
            compared += e.similar(&vq).unwrap().compared;
        }
        (start.elapsed() / queries.len() as u32, compared / queries.len())
    };
    let (exact_t, exact_c) = time(true);
    let (hnsw_t, hnsw_c) = time(false);
    println!("exact: {exact_t:?} per search, {exact_c} vectors compared");
    println!("HNSW:  {hnsw_t:?} per search, {hnsw_c} vectors compared");
    for ef in [50, 100, 200, 400, 800] {
        println!("recall@10 with ef {ef}: {:.3}", recall(&e, &queries[..30], 10, ef));
    }
    let start = std::time::Instant::now();
    let r = e.tune_vector_index("chunk:*", "embedding", 0.95, 10, 100, None).unwrap();
    println!("tune (recall 0.95) in {:?}: exact {:.0} µs per search", start.elapsed(), r.exact_micros);
    for s in &r.steps {
        println!("  ef_search {}: recall {:.3}, {} compared, {:.0} µs", s.ef, s.recall, s.compared, s.micros);
    }
    println!("kept ef_search {} (reached: {})", r.chosen, r.reached);
    // The same with the real queries, to see how close the estimate is.
    for (i, q) in queries.iter().enumerate() {
        embed(&e, &format!("query:{i:03}"), q, json!({}));
    }
    let r = e.tune_vector_index("chunk:*", "embedding", 0.95, 10, 100, Some("query:*")).unwrap();
    for s in &r.steps {
        println!("  real queries, ef_search {}: recall {:.3}, {} compared", s.ef, s.recall, s.compared);
    }
    println!("kept ef_search {} with the real queries", r.chosen);
    let start = std::time::Instant::now();
    e.close();
    drop(e);
    let e = Engine::open(dir.path().join("store.kiva"), quiet(0)).unwrap();
    println!("restart with the saved graph: {:?}", start.elapsed());
    assert_eq!(e.list_vector_indexes()[0].vectors, n);
}

/// The `limit` closest keys by cosine among the documents `keep` accepts.
fn brute_force(
    docs: &[(String, Vec<f32>, Json)],
    q: &[f32],
    keep: impl Fn(&Json) -> bool,
    limit: usize,
) -> Vec<String> {
    let cos = |v: &[f32]| {
        let d: f32 = v.iter().zip(q).map(|(a, b)| a * b).sum();
        d / (v.iter().map(|x| x * x).sum::<f32>().sqrt() * q.iter().map(|x| x * x).sum::<f32>().sqrt())
    };
    let mut scored: Vec<(f32, &String)> = docs.iter().filter(|d| keep(&d.2)).map(|d| (cos(&d.1), &d.0)).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
    scored.into_iter().take(limit).map(|(_, k)| k.clone()).collect()
}

#[test]
fn vector_search_filters_on_metadata_before_or_after_the_similarity() {
    use kivadb::engine::{CmpOp, Cond};
    let dir = TempDir::new("vector_filter");
    let e = Engine::open(dir.path().join("store.kiva"), quiet(0)).unwrap();
    let vectors = random_vectors(2000, 8, 21);
    let docs: Vec<(String, Vec<f32>, Json)> = vectors
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let meta = json!({
                "lang": if i % 10 == 0 { "fr" } else { "en" },
                "year": 2000 + (i % 25) as i64,
                "tag": if i % 400 == 0 { "rare" } else { "common" },
            });
            (format!("chunk:{i:05}"), v, meta)
        })
        .collect();
    for (k, v, meta) in &docs {
        embed(&e, k, v, meta.clone());
    }
    let cond = |field: &str, op: CmpOp, value: Json| Cond {
        pattern: "chunk:*".into(),
        field: field.into(),
        op,
        value,
        func: None,
    };
    let fr = || cond("lang", CmpOp::Eq, json!("fr"));
    let recent = || cond("year", CmpOp::Ge, json!(2020));
    let is_fr_recent = |d: &Json| d["lang"] == "fr" && d["year"].as_i64().unwrap() >= 2020;
    let q = vec![0.3, -0.1, 0.2, 0.0, 0.1, 0.25, -0.2, 0.05];
    let search = |filter: Vec<Cond>, exact: bool| {
        let mut vq = query("chunk:*", q.clone(), Metric::Cosine, 5);
        vq.filter = filter;
        vq.exact = exact;
        e.similar(&vq).unwrap()
    };
    let keys = |r: &kivadb::engine::VectorResult| r.rows.iter().map(|(k, _, _)| k.clone()).collect::<Vec<_>>();
    let truth = brute_force(&docs, &q, is_fr_recent, 5);
    assert_eq!(truth.len(), 5);

    // Without a vector index the filter is tested on every document read.
    let r = search(vec![fr(), recent()], false);
    assert_eq!(keys(&r), truth);
    assert!(r.filter.as_deref().unwrap().contains("every document"), "{:?}", r.filter);

    // An exact index: closest first, documents read only until 5 pass.
    e.create_vector_index("chunk:*", "embedding").unwrap();
    let r = search(vec![fr(), recent()], false);
    assert_eq!(keys(&r), truth);
    let how = r.filter.unwrap();
    assert!(how.contains("closest vectors first"), "{how}");
    let read: usize = how.split(", ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
    assert!(read < 2000 / 4, "{read} documents read for 5 results");

    // An HNSW graph: searched, then filtered, widening until enough pass.
    e.drop_vector_index("chunk:*", "embedding").unwrap();
    e.create_vector_index_with("chunk:*", "embedding", hnsw(Metric::Cosine)).unwrap();
    let r = search(vec![fr(), recent()], false);
    assert!(r.approximate);
    assert_eq!(r.rows.len(), 5);
    assert!(r.rows.iter().all(|(_, _, d)| is_fr_recent(d)));
    let found = keys(&r).iter().filter(|k| truth.contains(k)).count();
    assert!(found >= 4, "recall {found}/5");
    // A filter nothing passes ends with the whole graph explored.
    assert!(search(vec![cond("lang", CmpOp::Eq, json!("sw"))], false).rows.is_empty());

    // A secondary index serving a selective condition: pre-filtering, exact.
    e.create_index("chunk:*", "tag").unwrap();
    let rare = || cond("tag", CmpOp::Eq, json!("rare"));
    let r = search(vec![rare()], false);
    assert!(!r.approximate, "few candidates are compared one by one");
    assert_eq!(r.examined, 5);
    assert!(r.filter.as_deref().unwrap().contains("pre-filtered by the index on chunk:* (tag)"));
    assert_eq!(keys(&r), brute_force(&docs, &q, |d| d["tag"] == "rare", 5));
    // Not selective enough for a graph: the graph is searched instead.
    e.create_index("chunk:*", "lang").unwrap();
    let r = search(vec![cond("lang", CmpOp::Eq, json!("en"))], false);
    assert!(r.approximate);
    assert!(r.rows.iter().all(|(_, _, d)| d["lang"] == "en"));
    // Exact on request: the index's candidates, every one compared.
    let r = search(vec![fr(), recent()], true);
    assert_eq!(keys(&r), truth);
    assert!(r.filter.unwrap().contains("pre-filtered"));
}

#[test]
fn vector_indexes_stay_within_their_memory_bound() {
    let dir = TempDir::new("vector_ram");
    let path = dir.path().join("store.kiva");
    // 64 dimensions: about 330 bytes per vector in an exact index.
    let opts = || EngineOptions { max_vector_bytes: 16 * 1024, ..quiet(0) };
    let e = Engine::open(&path, opts()).unwrap();
    e.create_vector_index("doc:*", "embedding").unwrap();
    let v = |i: usize| (0..64).map(|d| ((i * 7 + d) % 13) as f32 / 13.0).collect::<Vec<f32>>();
    let put = |e: &Engine, i: usize| e.put(&format!("doc:{i:03}"), &Value::Doc(json!({ "embedding": v(i) })), 0, false);
    let mut stored = 0;
    let refused = loop {
        match put(&e, stored) {
            Ok(_) => stored += 1,
            Err(err) => break err.to_string(),
        }
        assert!(stored < 1000, "never refused");
    };
    assert!(refused.contains("OOM") && refused.contains("vector.max_ram_mb"), "{refused}");
    assert!((40..=50).contains(&stored), "{stored} vectors in 16 KB");
    let m = e.memory();
    assert!(m.vectors <= 16 * 1024 && m.vector_limit == 16 * 1024);
    assert_eq!(e.list_vector_indexes()[0].memory_bytes, m.vectors);
    // What adds no vector is never refused: a new vector for a key that has
    // one, a document without a vector, other keys, deletes.
    put(&e, 0).unwrap();
    e.put("doc:text", &Value::Doc(json!({ "text": "no vector" })), 0, false).unwrap();
    e.put("other:1", &Value::Doc(json!({ "embedding": v(1) })), 0, false).unwrap();
    e.delete("doc:001").unwrap();
    put(&e, stored).expect("a delete gives its room back");
    assert!(put(&e, stored + 1).is_err());

    // An index too large is refused before it fills the memory.
    for i in 0..200 {
        e.put(&format!("big:{i:03}"), &Value::Doc(json!({ "embedding": v(i) })), 0, false).unwrap();
    }
    let err = e.create_vector_index("big:*", "embedding").unwrap_err().to_string();
    assert!(err.contains("stopped building after") && err.contains("vector.max_ram_mb"), "{err}");
    assert_eq!(e.list_vector_indexes().len(), 1);

    // The bound is a setting: lifted, the index and the writes go through.
    e.set_vector_limit(0);
    assert_eq!(e.create_vector_index("big:*", "embedding").unwrap(), 200);
    put(&e, stored + 1).unwrap();
    // A restart never refuses what is already stored.
    drop(e);
    let e = Engine::open(&path, opts()).unwrap();
    assert_eq!(e.list_vector_indexes().iter().map(|v| v.vectors).sum::<usize>(), stored + 1 + 200);
    assert!(put(&e, 999).is_err());

    // The workspace bound counts the vectors too, even for an existing key.
    let e2 = Engine::open(dir.path().join("two.kiva"), quiet(0)).unwrap();
    e2.put("doc:1", &Value::Doc(json!({ "title": "no vector yet" })), 0, false).unwrap();
    e2.create_vector_index("doc:*", "embedding").unwrap();
    e2.set_memory_limit(e2.memory().resident() + 100);
    let err = e2.put("doc:1", &Value::Doc(json!({ "embedding": v(1) })), 0, false).unwrap_err().to_string();
    assert!(err.contains("storage.max_memory_mb"), "{err}");
}

#[test]
fn vector_filters_limits_and_embeddings_from_the_shell_and_the_api() {
    let dir = TempDir::new("vector_shell");
    let mut s = Session::new(instance(&dir), true, None);
    ok(&mut s, r#"set doc:1 {"title": "Kinshasa guide", "lang": "fr", "year": 2024, "embedding": [0.9, 0.1, 0.0]}"#);
    ok(&mut s, r#"set doc:2 {"title": "Goma guide", "lang": "en", "year": 2024, "embedding": [0.95, 0.05, 0.0]}"#);
    ok(&mut s, r#"set doc:3 {"title": "Old Kinshasa", "lang": "fr", "year": 1990, "embedding": [0.8, 0.2, 0.1]}"#);
    ok(&mut s, r#"set doc:4 {"title": "Text only", "embedding": "a guide to Kinshasa"}"#);
    ok(&mut s, "create vector index on doc:* (embedding)");

    let out = ok(&mut s, r#"get similar doc:*.embedding to [1, 0, 0] where lang = "fr" and year >= 2000 return title"#);
    assert!(out.contains("doc:1") && !out.contains("doc:2") && !out.contains("doc:3"), "{out}");
    assert!(out.contains("(1 results") && out.contains("filter tested on the closest vectors first"), "{out}");
    // The field may repeat the pattern of the search.
    let out = ok(&mut s, r#"get similar doc:*.embedding to [1, 0, 0] where doc:*.lang = "fr" limit 1 return title"#);
    assert!(out.contains("doc:1:"), "{out}");
    assert!(fails(&mut s, r#"get similar doc:*.embedding to [1, 0, 0] where lang = "fr" or lang = "en""#)
        .contains("'and' only"));
    assert!(fails(&mut s, r#"get similar doc:*.embedding to [1, 0, 0] where user:*.lang = "fr""#)
        .contains("fields of the documents of doc:*"));
    assert!(fails(&mut s, r#"get similar doc:*.embedding to [1, 0, 0] where lang in "fr""#).contains("list of values"));
    assert!(
        fails(&mut s, r#"get similar doc:*.embedding to [1, 0, 0] where year between 1 and 2"#).contains("operators")
    );

    // KivaDB receives vectors; text to embed is the application's job.
    assert!(fails(&mut s, r#"get similar doc:*.embedding to "a guide to Kinshasa""#)
        .contains("does not compute embeddings"));
    let indexes = ok(&mut s, "indexes");
    assert!(
        indexes.contains("1 skipped (not an array of numbers: embeddings are computed by the application)"),
        "{indexes}"
    );
    assert!(indexes.contains("in memory") && indexes.contains("Vector memory:"), "{indexes}");
    assert!(ok(&mut s, "memory").contains("Vector indexes    :"));

    // The bound is a setting, applied at once.
    ok(&mut s, "config set vector.max_ram_mb 0");
    assert!(ok(&mut s, "indexes").contains("no bound: vector.max_ram_mb = 0"));
    assert!(fails(&mut s, "config set vector.max_ram_mb lots").contains("invalid value"));
    ok(&mut s, "config set vector.max_ram_mb auto");

    // The structured API: equalities as an object, comparisons as a list.
    let api = |s: &mut Session, req: Json| kivadb::api::handle(s, &req);
    let r = api(
        &mut s,
        json!({ "op": "similar", "target": "doc:*.embedding", "vector": [1, 0, 0], "filter": { "lang": "fr" } }),
    )
    .unwrap();
    let keys: Vec<&str> = r["results"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["doc:1", "doc:3"]);
    assert!(r["filter"].as_str().unwrap().contains("closest vectors first"));
    let r = api(
        &mut s,
        json!({ "op": "similar", "target": "doc:*.embedding", "vector": [1, 0, 0],
                "filter": [{ "field": "year", "cmp": "<", "value": 2000 }] }),
    )
    .unwrap();
    assert_eq!(r["results"][0]["key"], "doc:3");
    assert_eq!(r["results"].as_array().unwrap().len(), 1);
    let err = api(&mut s, json!({ "op": "similar", "target": "doc:*.embedding", "vector": "Kinshasa" })).unwrap_err();
    assert!(err.to_string().contains("does not compute embeddings"));
    let err = api(&mut s, json!({ "op": "similar", "target": "doc:*.embedding", "vector": [1, 0, 0], "filter": 3 }))
        .unwrap_err();
    assert!(err.to_string().contains("\"filter\""));
}

#[test]
fn vector_commands() {
    let dir = TempDir::new("vector_cmds");
    let mut s = Session::new(instance(&dir), true, None);
    ok(&mut s, r#"set doc:1 {"title": "Kinshasa travel guide", "embedding": [0.9, 0.1, 0.0]}"#);
    ok(&mut s, r#"set doc:2 {"title": "Goma lake trips", "embedding": [0.8, 0.3, 0.1]}"#);
    ok(&mut s, r#"set doc:3 {"title": "Rust programming", "embedding": [0.0, 0.2, 0.95]}"#);

    let out = ok(&mut s, "get similar doc:*.embedding to [1, 0, 0] limit 2 return title");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("doc:1: 0.99"), "{out}");
    assert!(lines[0].ends_with(r#"{"title":"Kinshasa travel guide"}"#));
    assert!(lines[1].starts_with("doc:2: "));
    assert_eq!(lines[2], "(2 results, cosine metric, 3 vectors compared, documents read from disk)");

    // "More like this": the query vector can be another document's field.
    let out = ok(&mut s, "get similar doc:*.embedding to doc:3.embedding limit 1 metric l2 return title");
    assert!(out.starts_with(r#"doc:3: 0.0000 {"title":"Rust programming"}"#), "{out}");

    assert_eq!(
        ok(&mut s, "create vector index on doc: (embedding)"),
        "Vector index created on doc:* (embedding): 3 vectors in memory."
    );
    assert!(ok(&mut s, "get similar doc:*.embedding to [0, 0, 1] limit 1").contains("vector index"));
    assert!(ok(&mut s, "indexes").contains("doc:* (embedding) - vector index, 3 vectors, 3 dimensions"));
    assert!(ok(&mut s, "stats").contains("(+ 1 vector)"));
    assert!(fails(&mut s, "get similar doc:*.embedding to [1, 0] limit 1").contains("3 dimensions"));
    assert!(fails(&mut s, r#"get similar doc:*.embedding to ["a"]"#).contains("non-empty array of numbers"));
    assert!(fails(&mut s, "get similar doc:*.embedding to [1, 0, 0] metric manhattan").contains("cosine, dot or l2"));
    assert!(fails(&mut s, "get similar embedding to [1]").contains("<pattern>.<field>"));
    assert!(fails(&mut s, "get similar doc:*.embedding to ghost.embedding").contains("does not exist"));
    assert!(fails(&mut s, "create vector index on doc: (embedding)").contains("already exists"));
    assert_eq!(ok(&mut s, "drop vector index on doc: (embedding)"), "Vector index on doc:* (embedding) dropped.");

    // The approximate index, and the options of a search.
    assert_eq!(
        ok(&mut s, "create vector index on doc: (embedding) using hnsw m 8 ef 50"),
        "HNSW vector index created on doc:* (embedding): 3 vectors in a graph (cosine, m 8, ef 50)."
    );
    assert!(ok(&mut s, "indexes").contains(
        "doc:* (embedding) - HNSW vector index (cosine, m 8, ef 50, ef_search 100), 3 vectors, 3 dimensions"
    ));
    let out = ok(&mut s, "get similar doc:*.embedding to [0, 0, 1] limit 1 ef 20 return title");
    assert!(out.starts_with(r#"doc:3: 0.97"#) && out.contains("HNSW index, approximate"), "{out}");
    assert!(ok(&mut s, "get similar doc:*.embedding to [0, 0, 1] limit 1 exact")
        .contains("3 vectors compared, vector index)"));
    assert!(ok(&mut s, "get similar doc:*.embedding to [0, 0, 1] metric l2").contains("vector index)"));
    assert!(fails(&mut s, "get similar doc:*.embedding to [0, 0, 1] ef 0").contains("ef must be between"));
    assert!(fails(&mut s, "create vector index on x: (v) using annoy").contains("only approximate index is hnsw"));
    assert!(fails(&mut s, "create vector index on x: (v) using hnsw m 1").contains("m must be between"));
}

// ---------------------------------------------------------------------------
// Redis protocol
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum R {
    S(String),
    E(String),
    I(i64),
    B(Option<String>),
    A(Option<Vec<R>>),
    /// A RESP3 map, as its flattened key-value list.
    M(Vec<R>),
    /// A RESP3 push (pub/sub messages).
    P(Vec<R>),
    /// A RESP3 double, as its text.
    D(String),
    /// A RESP3 set.
    Set(Vec<R>),
}

fn read_reply(r: &mut impl BufRead) -> R {
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let line = line.trim_end_matches("\r\n");
    let (kind, rest) = line.split_at(1);
    match kind {
        "+" => R::S(rest.into()),
        "-" => R::E(rest.into()),
        ":" => R::I(rest.parse().unwrap()),
        "$" => {
            let n: i64 = rest.parse().unwrap();
            if n < 0 {
                return R::B(None);
            }
            let mut buf = vec![0u8; n as usize + 2];
            r.read_exact(&mut buf).unwrap();
            buf.truncate(n as usize);
            R::B(Some(String::from_utf8_lossy(&buf).into_owned()))
        }
        "*" => {
            let n: i64 = rest.parse().unwrap();
            if n < 0 {
                return R::A(None);
            }
            R::A(Some((0..n).map(|_| read_reply(r)).collect()))
        }
        // RESP3: null and maps.
        "_" => R::B(None),
        "," => R::D(rest.into()),
        "~" => {
            let n: usize = rest.parse().unwrap();
            R::Set((0..n).map(|_| read_reply(r)).collect())
        }
        "%" => {
            let n: usize = rest.parse().unwrap();
            R::M((0..n * 2).map(|_| read_reply(r)).collect())
        }
        ">" => {
            let n: usize = rest.parse().unwrap();
            R::P((0..n).map(|_| read_reply(r)).collect())
        }
        other => panic!("unexpected reply type {other:?} in {line:?}"),
    }
}

struct Redis {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Redis {
    fn connect(port: u16) -> Redis {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        Redis { stream, reader }
    }

    fn cmd(&mut self, args: &[&str]) -> R {
        let mut req = format!("*{}\r\n", args.len());
        for a in args {
            req.push_str(&format!("${}\r\n{a}\r\n", a.len()));
        }
        self.stream.write_all(req.as_bytes()).unwrap();
        read_reply(&mut self.reader)
    }

    /// The next reply or message, without sending anything.
    fn next(&mut self) -> R {
        read_reply(&mut self.reader)
    }
}

fn b(s: &str) -> R {
    R::B(Some(s.into()))
}

fn ok_reply() -> R {
    R::S("OK".into())
}

#[test]
fn redis_clients_can_use_kivadb() {
    let dir = TempDir::new("resp");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);

    assert_eq!(r.cmd(&["PING"]), R::S("PONG".into()));
    assert_eq!(r.cmd(&["ECHO", "mbote"]), b("mbote"));
    assert_eq!(r.cmd(&["SELECT", "0"]), ok_reply());
    assert!(matches!(r.cmd(&["SELECT", "16"]), R::E(e) if e.contains("out of range")));

    // Strings and counters.
    assert_eq!(r.cmd(&["SET", "greeting", "hello world"]), ok_reply());
    assert_eq!(r.cmd(&["GET", "greeting"]), b("hello world"));
    assert_eq!(r.cmd(&["GET", "ghost"]), R::B(None));
    assert_eq!(r.cmd(&["SET", "greeting", "x", "NX"]), R::B(None));
    assert_eq!(r.cmd(&["SET", "fresh", "x", "XX"]), R::B(None));
    assert_eq!(r.cmd(&["SET", "greeting", "bye", "GET"]), b("hello world"));
    assert_eq!(r.cmd(&["SETNX", "greeting", "no"]), R::I(0));
    assert_eq!(r.cmd(&["APPEND", "greeting", "!"]), R::I(4));
    assert_eq!(r.cmd(&["STRLEN", "greeting"]), R::I(4));
    assert_eq!(r.cmd(&["INCR", "visits"]), R::I(1));
    assert_eq!(r.cmd(&["INCRBY", "visits", "41"]), R::I(42));
    assert_eq!(r.cmd(&["DECR", "visits"]), R::I(41));
    assert_eq!(r.cmd(&["INCRBYFLOAT", "price", "2.5"]), b("2.5"));
    assert!(matches!(r.cmd(&["INCR", "greeting"]), R::E(e) if e.contains("not an integer")));
    assert_eq!(r.cmd(&["MSET", "a", "1", "b", "2"]), ok_reply());
    assert_eq!(r.cmd(&["MGET", "a", "b", "ghost"]), R::A(Some(vec![b("1"), b("2"), R::B(None)])));
    assert_eq!(r.cmd(&["EXISTS", "a", "b", "ghost"]), R::I(2));
    assert_eq!(r.cmd(&["GETDEL", "a"]), b("1"));
    assert_eq!(r.cmd(&["DEL", "a", "b", "ghost"]), R::I(1));

    // Expiry.
    assert_eq!(r.cmd(&["SET", "session", "abc", "EX", "100"]), ok_reply());
    assert!(matches!(r.cmd(&["TTL", "session"]), R::I(t) if (99..=100).contains(&t)));
    assert_eq!(r.cmd(&["TTL", "greeting"]), R::I(-1));
    assert_eq!(r.cmd(&["TTL", "ghost"]), R::I(-2));
    assert_eq!(r.cmd(&["PERSIST", "session"]), R::I(1));
    assert_eq!(r.cmd(&["EXPIRE", "session", "50"]), R::I(1));
    assert_eq!(r.cmd(&["SET", "session", "def", "KEEPTTL"]), ok_reply());
    assert!(matches!(r.cmd(&["TTL", "session"]), R::I(t) if t > 0));
    assert_eq!(r.cmd(&["SETEX", "tmp", "1", "v"]), ok_reply());
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(r.cmd(&["GET", "tmp"]), R::B(None));

    // Hashes are KivaDB objects, lists are arrays.
    assert_eq!(r.cmd(&["HSET", "user:1", "name", "Alice", "city", "Kinshasa"]), R::I(2));
    assert_eq!(r.cmd(&["HSET", "user:1", "city", "Goma"]), R::I(0));
    assert_eq!(r.cmd(&["HGET", "user:1", "city"]), b("Goma"));
    assert_eq!(r.cmd(&["HINCRBY", "user:1", "logins", "3"]), R::I(3));
    assert_eq!(r.cmd(&["HLEN", "user:1"]), R::I(3));
    assert_eq!(
        r.cmd(&["HGETALL", "user:1"]),
        R::A(Some(vec![b("name"), b("Alice"), b("city"), b("Goma"), b("logins"), b("3")]))
    );
    assert_eq!(r.cmd(&["HDEL", "user:1", "logins", "nope"]), R::I(1));
    assert_eq!(r.cmd(&["TYPE", "user:1"]), R::S("hash".into()));
    assert!(matches!(r.cmd(&["GET", "user:1"]), R::E(e) if e.starts_with("WRONGTYPE")));
    assert_eq!(r.cmd(&["RPUSH", "queue", "a", "b", "c"]), R::I(3));
    assert_eq!(r.cmd(&["LPUSH", "queue", "z"]), R::I(4));
    assert_eq!(r.cmd(&["LRANGE", "queue", "0", "-1"]), R::A(Some(vec![b("z"), b("a"), b("b"), b("c")])));
    assert_eq!(r.cmd(&["LPOP", "queue"]), b("z"));
    assert_eq!(r.cmd(&["RPOP", "queue"]), b("c"));
    assert_eq!(r.cmd(&["LLEN", "queue"]), R::I(2));
    assert_eq!(r.cmd(&["LINDEX", "queue", "-1"]), b("b"));
    assert_eq!(r.cmd(&["TYPE", "queue"]), R::S("list".into()));

    // The same data, seen from the KivaDB side.
    let mut s = Session::new(inst.clone(), true, None);
    assert_eq!(
        ok(&mut s, "get user:1.name and visits and queue"),
        "user:1.name: Alice [string]\nvisits: 41 [number]\nqueue: [\"a\",\"b\"] [array]"
    );
    ok(&mut s, r#"set from_kiva {"lang": "ln"}"#);
    assert_eq!(r.cmd(&["HGET", "from_kiva", "lang"]), b("ln"));

    // Keys, transactions, pipelining, inline commands.
    assert!(matches!(r.cmd(&["KEYS", "user:*"]), R::A(Some(v)) if v == vec![b("user:1")]));
    match r.cmd(&["SCAN", "0", "COUNT", "100"]) {
        R::A(Some(v)) => assert_eq!(v[0], b("0")),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.cmd(&["MULTI"]), ok_reply());
    assert_eq!(r.cmd(&["INCR", "visits"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["SET", "in_tx", "yes"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["EXEC"]), R::A(Some(vec![R::I(42), ok_reply()])));
    assert_eq!(r.cmd(&["MULTI"]), ok_reply());
    assert_eq!(r.cmd(&["SET", "never", "x"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["DISCARD"]), ok_reply());
    assert_eq!(r.cmd(&["EXISTS", "never"]), R::I(0));
    r.stream.write_all(b"*1\r\n$4\r\nPING\r\n*2\r\n$3\r\nGET\r\n$5\r\nin_tx\r\n").unwrap();
    assert_eq!(read_reply(&mut r.reader), R::S("PONG".into()));
    assert_eq!(read_reply(&mut r.reader), b("yes"));
    r.stream.write_all(b"PING\r\n").unwrap();
    assert_eq!(read_reply(&mut r.reader), R::S("PONG".into()));
    assert!(
        matches!(r.cmd(&["INFO"]), R::B(Some(t)) if t.contains("redis_version:") && t.contains(concat!("kivadb_version:", env!("CARGO_PKG_VERSION"))))
    );
    assert!(matches!(r.cmd(&["NOPE"]), R::E(e) if e.contains("unknown command")));
    assert_eq!(r.cmd(&["DBSIZE"]), R::I(8));
    assert_eq!(r.cmd(&["QUIT"]), ok_reply());
}

#[test]
fn redis_clients_must_log_in_when_kivadb_requires_it() {
    let dir = TempDir::new("resp_auth");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"modify user "kiva" password "pw""#);
    ok(&mut admin, r#"create user "eve" identified by "evepw" role viewer"#);
    ok(&mut admin, r#"grant read on workspace "default" to "eve""#);
    ok(&mut admin, r#"set shared "data""#);
    ok(&mut admin, "config set auth_required true");
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();

    let mut r = Redis::connect(port);
    assert!(matches!(r.cmd(&["GET", "shared"]), R::E(e) if e.starts_with("NOAUTH")));
    assert!(matches!(r.cmd(&["AUTH", "wrong"]), R::E(e) if e.starts_with("WRONGPASS")));
    // One argument means the built-in administrator, as Redis's "default" user.
    assert_eq!(r.cmd(&["AUTH", "pw"]), ok_reply());
    assert_eq!(r.cmd(&["GET", "shared"]), b("data"));

    let mut v = Redis::connect(port);
    assert!(matches!(v.cmd(&["HELLO", "4"]), R::E(e) if e.starts_with("NOPROTO")));
    assert!(matches!(v.cmd(&["HELLO", "2", "AUTH", "eve", "evepw"]), R::A(Some(_))));
    assert_eq!(v.cmd(&["SELECT", "0"]), ok_reply());
    assert!(matches!(v.cmd(&["SET", "x", "1"]), R::E(e) if e.starts_with("NOPERM")));
    assert!(matches!(v.cmd(&["FLUSHDB"]), R::E(e) if e.starts_with("NOPERM")));
}

#[test]
fn redis_counters_are_atomic_across_connections() {
    let dir = TempDir::new("resp_atomic");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    // Eight clients count login attempts at the same time: none may be lost.
    let workers: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(move || {
                let mut r = Redis::connect(port);
                for _ in 0..250 {
                    assert!(matches!(r.cmd(&["INCR", "rl:login:1.2.3.4"]), R::I(_)));
                    r.cmd(&["HINCRBY", "stats", "attempts", "1"]);
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let mut r = Redis::connect(port);
    assert_eq!(r.cmd(&["GET", "rl:login:1.2.3.4"]), b("2000"));
    assert_eq!(r.cmd(&["HGET", "stats", "attempts"]), b("2000"));
}

#[test]
fn redis_expiry_options_and_time() {
    let dir = TempDir::new("resp_expire");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);

    r.cmd(&["SET", "k", "v"]);
    // NX: only when there is no expiry yet. The rate-limit window must not slide.
    assert_eq!(r.cmd(&["EXPIRE", "k", "900", "NX"]), R::I(1));
    assert_eq!(r.cmd(&["EXPIRE", "k", "5", "NX"]), R::I(0));
    assert!(matches!(r.cmd(&["TTL", "k"]), R::I(t) if t > 890));
    // XX: only when there is one. GT / LT: only when it moves that way.
    assert_eq!(r.cmd(&["EXPIRE", "k", "100", "GT"]), R::I(0));
    assert_eq!(r.cmd(&["EXPIRE", "k", "100", "LT"]), R::I(1));
    assert!(matches!(r.cmd(&["TTL", "k"]), R::I(t) if (95..=100).contains(&t)));
    assert_eq!(r.cmd(&["EXPIRE", "k", "2000", "XX", "GT"]), R::I(1));
    r.cmd(&["SET", "p", "v"]);
    assert_eq!(r.cmd(&["EXPIRE", "p", "10", "XX"]), R::I(0));
    assert_eq!(r.cmd(&["EXPIRE", "p", "10", "GT"]), R::I(0), "no expiry counts as infinite");
    assert_eq!(r.cmd(&["EXPIRE", "p", "10", "LT"]), R::I(1));
    assert!(matches!(r.cmd(&["EXPIRE", "p", "10", "NX", "XX"]), R::E(e) if e.contains("not compatible")));
    assert!(matches!(r.cmd(&["EXPIRE", "p", "10", "SOON"]), R::E(e) if e.contains("Unsupported option")));
    assert_eq!(r.cmd(&["EXPIRE", "ghost", "10"]), R::I(0));

    // INCR keeps the expiry of the counter, like Redis.
    r.cmd(&["SET", "c", "1", "EX", "600"]);
    r.cmd(&["INCR", "c"]);
    assert!(matches!(r.cmd(&["TTL", "c"]), R::I(t) if t > 590));

    // EXPIRETIME, GETEX.
    let R::I(at) = r.cmd(&["EXPIRETIME", "c"]) else { panic!() };
    assert!(at > 1_700_000_000);
    assert!(matches!(r.cmd(&["PEXPIRETIME", "c"]), R::I(p) if p / 1000 == at));

    // Milliseconds are kept as given: PX 150 is gone after 150 ms, not a second.
    r.cmd(&["SET", "lock", "me", "PX", "150"]);
    assert!(matches!(r.cmd(&["PTTL", "lock"]), R::I(t) if (100..=150).contains(&t)));
    assert_eq!(r.cmd(&["TTL", "lock"]), R::I(0), "rounded to the nearest second");
    r.cmd(&["SET", "lease", "x"]);
    assert_eq!(r.cmd(&["PEXPIRE", "lease", "120"]), R::I(1));
    r.cmd(&["PSETEX", "short", "130", "y"]);
    std::thread::sleep(Duration::from_millis(250));
    for k in ["lock", "lease", "short"] {
        assert_eq!(r.cmd(&["GET", k]), R::B(None), "{k}");
    }
    r.cmd(&["SET", "at", "v", "PXAT", &(kivadb::util::now_ms() + 100).to_string()]);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(r.cmd(&["EXISTS", "at"]), R::I(0));
    assert_eq!(r.cmd(&["EXPIRETIME", "greeting-ghost"]), R::I(-2));
    r.cmd(&["SET", "s", "hello"]);
    assert_eq!(r.cmd(&["EXPIRETIME", "s"]), R::I(-1));
    assert_eq!(r.cmd(&["GETEX", "s", "EX", "60"]), b("hello"));
    assert!(matches!(r.cmd(&["TTL", "s"]), R::I(t) if t > 55));
    assert_eq!(r.cmd(&["GETEX", "s", "PERSIST"]), b("hello"));
    assert_eq!(r.cmd(&["TTL", "s"]), R::I(-1));
    assert_eq!(r.cmd(&["GETEX", "ghost"]), R::B(None));

    // TIME: seconds and microseconds.
    let R::A(Some(t)) = r.cmd(&["TIME"]) else { panic!() };
    assert!(matches!(&t[0], R::B(Some(s)) if s.parse::<i64>().unwrap() > 1_700_000_000));
}

#[test]
fn redis_numbered_databases_are_workspaces() {
    let dir = TempDir::new("resp_select");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"create user "pay" identified by "paypw""#);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();

    // The administrator gets db2 on first use, isolated from database 0.
    let mut r = Redis::connect(port);
    r.cmd(&["SET", "where", "zero"]);
    assert_eq!(r.cmd(&["SELECT", "2"]), ok_reply());
    assert_eq!(r.cmd(&["GET", "where"]), R::B(None));
    r.cmd(&["SET", "where", "two"]);
    assert_eq!(r.cmd(&["SELECT", "0"]), ok_reply());
    assert_eq!(r.cmd(&["GET", "where"]), b("zero"));
    assert_eq!(r.cmd(&["SELECT", "2"]), ok_reply());
    assert_eq!(r.cmd(&["GET", "where"]), b("two"));
    assert!(ok(&mut admin, "workspace list").contains("db2"));

    // A user needs to be given access, and the error says how.
    let mut p = Redis::connect(port);
    assert_eq!(p.cmd(&["AUTH", "pay", "paypw"]), ok_reply());
    assert!(
        matches!(p.cmd(&["SELECT", "3"]), R::E(e) if e.starts_with("NOPERM") && e.contains("workspace create db3"))
    );
    assert!(matches!(p.cmd(&["SELECT", "2"]), R::E(e) if e.contains(r#"grant write on workspace "db2" to "pay""#)));
    ok(&mut admin, r#"grant write on workspace "db2" to "pay""#);
    assert_eq!(p.cmd(&["SELECT", "2"]), ok_reply());
    assert_eq!(p.cmd(&["GET", "where"]), b("two"));
    // Database 0 is where the user arrived: no workspace for an account that had no grant then.
    assert_eq!(p.cmd(&["SELECT", "0"]), ok_reply());
    assert!(matches!(p.cmd(&["GET", "where"]), R::E(e) if e.contains("no workspace")));

    // A workspace by its name, in a project.
    ok(&mut admin, "project create school");
    ok(&mut admin, "workspace create school.places");
    assert!(
        matches!(p.cmd(&["SELECT", "school.places"]), R::E(e) if e.starts_with("NOPERM") && e.contains("grant write on school.places"))
    );
    ok(&mut admin, r#"grant write on school.* to "pay""#);
    assert_eq!(p.cmd(&["SELECT", "school.places"]), ok_reply());
    p.cmd(&["SET", "mine", "1"]);
    assert_eq!(ok(&mut admin, "workspace switch school.places"), "Switched to workspace 'school.places' (1 keys).");
    // With a default workspace, a new connection lands in it.
    ok(&mut admin, r#"modify user "pay" default workspace school.places"#);
    let mut q = Redis::connect(port);
    assert_eq!(q.cmd(&["AUTH", "pay", "paypw"]), ok_reply());
    assert_eq!(q.cmd(&["GET", "mine"]), b("1"));
}

#[test]
fn redis_clients_read_dates_as_iso_text() {
    let dir = TempDir::new("resp_dates");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "timezone +01:00");
    ok(&mut admin, r#"set datetime at "2026-10-27 14:30:00" and date born "2004-05-09" and time opens "08:00""#);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    assert_eq!(r.cmd(&["GET", "at"]), b("2026-10-27T13:30:00.000Z"));
    assert_eq!(r.cmd(&["GET", "born"]), b("2004-05-09"));
    assert_eq!(r.cmd(&["GET", "opens"]), b("08:00:00"));
    assert_eq!(r.cmd(&["TYPE", "at"]), R::S("string".into()));
}

#[test]
fn redis_writes_follow_the_schema() {
    let dir = TempDir::new("resp_schema");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"schema set acct: {"id": {"type": "string", "required": true}, "tier": {"default": "free"}}"#);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    match r.cmd(&["HSET", "acct:1", "name", "x"]) {
        R::E(e) => assert!(e.starts_with("SCHEMA ") && e.contains("id: required"), "{e}"),
        other => panic!("HSET without the id was accepted: {other:?}"),
    }
    match r.cmd(&["JSON.SET", "acct:2", "$", r#"{"name": "x"}"#]) {
        R::E(e) => assert!(e.starts_with("SCHEMA "), "{e}"),
        other => panic!("JSON.SET without the id was accepted: {other:?}"),
    }
    assert_eq!(r.cmd(&["HSET", "acct:1", "id", "a1"]), R::I(1));
    assert_eq!(r.cmd(&["HGET", "acct:1", "tier"]), b("free"));
}

#[test]
fn redis_lua_scripts_run_atomically() {
    let dir = TempDir::new("resp_eval");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);

    // Return values convert as in Redis.
    assert_eq!(r.cmd(&["EVAL", "return 42", "0"]), R::I(42));
    assert_eq!(r.cmd(&["EVAL", "return 3.9", "0"]), R::I(3));
    assert_eq!(r.cmd(&["EVAL", "return 'hi'", "0"]), b("hi"));
    assert_eq!(r.cmd(&["EVAL", "return nil", "0"]), R::B(None));
    assert_eq!(r.cmd(&["EVAL", "return true", "0"]), R::I(1));
    assert_eq!(
        r.cmd(&["EVAL", "return {1, 'two', {3}}", "0"]),
        R::A(Some(vec![R::I(1), b("two"), R::A(Some(vec![R::I(3)]))]))
    );
    assert_eq!(r.cmd(&["EVAL", "return redis.status_reply('FINE')", "0"]), R::S("FINE".into()));
    assert_eq!(r.cmd(&["EVAL", "return redis.error_reply('MYERR custom')", "0"]), R::E("MYERR custom".into()));
    assert_eq!(
        r.cmd(&["EVAL", "return {KEYS[1], ARGV[1], #KEYS, #ARGV}", "1", "k", "a", "b"]),
        R::A(Some(vec![b("k"), b("a"), R::I(1), R::I(2)]))
    );

    // The usual fixed-window rate limiter: count, start the window once, report.
    let limiter = "local n = redis.call('INCR', KEYS[1]) \
                   if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end \
                   return {n, redis.call('TTL', KEYS[1])}";
    assert_eq!(r.cmd(&["EVAL", limiter, "1", "rl:2fa:42", "900"]), R::A(Some(vec![R::I(1), R::I(900)])));
    let R::A(Some(second)) = r.cmd(&["EVAL", limiter, "1", "rl:2fa:42", "900"]) else { panic!() };
    assert_eq!(second[0], R::I(2));

    // EVALSHA: unknown until loaded, then served from the cache.
    let sha = "e0e1f9fabfc9d4800c877a703b823ac0578ff8db"; // sha1("return 1")
    assert!(matches!(r.cmd(&["EVALSHA", sha, "0"]), R::E(e) if e.starts_with("NOSCRIPT")));
    assert_eq!(r.cmd(&["SCRIPT", "LOAD", "return 1"]), b(sha));
    assert_eq!(r.cmd(&["EVALSHA", sha, "0"]), R::I(1));
    assert_eq!(r.cmd(&["EVALSHA", &sha.to_uppercase(), "0"]), R::I(1));
    assert_eq!(
        r.cmd(&["SCRIPT", "EXISTS", sha, "0000000000000000000000000000000000000000"]),
        R::A(Some(vec![R::I(1), R::I(0)]))
    );
    assert!(matches!(r.cmd(&["SCRIPT", "LOAD", "return +"]), R::E(e) if e.contains("Error compiling script")));
    assert_eq!(r.cmd(&["SCRIPT", "FLUSH"]), ok_reply());
    assert!(matches!(r.cmd(&["EVALSHA", sha, "0"]), R::E(e) if e.starts_with("NOSCRIPT")));

    // All or nothing: a failing script leaves no trace.
    let failing = "redis.call('SET', 'half', '1') return redis.call('INCR', 'text')";
    r.cmd(&["SET", "text", "abc"]);
    assert!(matches!(r.cmd(&["EVAL", failing, "0"]), R::E(e) if e.contains("not an integer")));
    assert_eq!(r.cmd(&["GET", "half"]), R::B(None));
    // pcall catches the error instead.
    assert_eq!(r.cmd(&["EVAL", "local e = redis.pcall('INCR', 'text') return e.err ~= nil", "0"]), R::I(1));
    assert!(
        matches!(r.cmd(&["EVAL", "error('boom')", "0"]), R::E(e) if e.starts_with("ERR Error running script") && e.contains("boom"))
    );
    assert!(
        matches!(r.cmd(&["EVAL", "return redis.call('MULTI')", "0"]), R::E(e) if e.contains("not allowed from script"))
    );
    assert!(
        matches!(r.cmd(&["EVAL", "return 1", "2", "only-one"]), R::E(e) if e.contains("greater than number of args"))
    );
    assert!(matches!(r.cmd(&["EVAL", "return os.time()", "0"]), R::E(_)), "the sandbox has no os library");

    // Scripts and MULTI/EXEC together.
    assert_eq!(r.cmd(&["MULTI"]), ok_reply());
    assert_eq!(r.cmd(&["EVAL", "return redis.call('INCR', KEYS[1])", "1", "m"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["INCR", "m"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["EXEC"]), R::A(Some(vec![R::I(1), R::I(2)])));
}

#[test]
fn redis_clients_can_speak_resp3() {
    let dir = TempDir::new("resp3");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    // RESP2 until the client asks for more.
    r.cmd(&["HSET", "user:1", "name", "Alice"]);
    assert_eq!(r.cmd(&["HGETALL", "user:1"]), R::A(Some(vec![b("name"), b("Alice")])));
    let R::M(hello) = r.cmd(&["HELLO", "3"]) else { panic!("HELLO 3 answers a map") };
    let proto = hello.chunks(2).find(|p| p[0] == b("proto")).map(|p| &p[1]);
    assert_eq!(proto, Some(&R::I(3)));
    assert_eq!(r.cmd(&["HGETALL", "user:1"]), R::M(vec![b("name"), b("Alice")]));
    assert_eq!(r.cmd(&["GET", "ghost"]), R::B(None));
    assert_eq!(r.cmd(&["SET", "k", "v"]), ok_reply());
    // Scripts keep seeing RESP2 shapes.
    assert_eq!(r.cmd(&["EVAL", "return #redis.call('HGETALL', 'user:1')", "0"]), R::I(2));
    assert!(matches!(r.cmd(&["HELLO", "2"]), R::A(Some(_))));
    assert_eq!(r.cmd(&["HGETALL", "user:1"]), R::A(Some(vec![b("name"), b("Alice")])));
}

#[test]
fn redis_scripts_obey_roles() {
    let dir = TempDir::new("resp_eval_roles");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"create user "eve" identified by "evepw" role viewer grant read on default"#);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut v = Redis::connect(port);
    assert_eq!(v.cmd(&["AUTH", "eve", "evepw"]), ok_reply());
    assert_eq!(v.cmd(&["EVAL", "return redis.call('GET', 'nothing')", "0"]), R::B(None));
    assert!(matches!(v.cmd(&["EVAL", "return redis.call('SET', 'x', '1')", "0"]), R::E(e) if e.starts_with("NOPERM")));
}

// ---------------------------------------------------------------------------
// HTTP: Server-Sent Events and structured requests
// ---------------------------------------------------------------------------

fn msg(parts: &[&str]) -> R {
    R::A(Some(parts.iter().map(|p| b(p)).collect()))
}

fn notice(kind: &str, name: &str, n: i64) -> R {
    R::A(Some(vec![b(kind), b(name), R::I(n)]))
}

#[test]
fn redis_publish_and_subscribe() {
    let dir = TempDir::new("resp_pubsub");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"create user "eve" identified by "evepw" role editor"#);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut sub = Redis::connect(port);
    let mut publ = Redis::connect(port);

    // One confirmation per channel, then the messages as they are published.
    assert_eq!(sub.cmd(&["SUBSCRIBE", "texts", "news"]), notice("subscribe", "texts", 1));
    assert_eq!(sub.next(), notice("subscribe", "news", 2));
    assert_eq!(publ.cmd(&["PUBLISH", "texts", "v2"]), R::I(1));
    assert_eq!(sub.next(), msg(&["message", "texts", "v2"]));
    assert_eq!(publ.cmd(&["PUBLISH", "nobody", "x"]), R::I(0));

    // Patterns, with Redis's glob syntax.
    assert_eq!(sub.cmd(&["PSUBSCRIBE", "app.[ab]*"]), notice("psubscribe", "app.[ab]*", 3));
    assert_eq!(publ.cmd(&["PUBLISH", "app.b1", "hi"]), R::I(1));
    assert_eq!(sub.next(), msg(&["pmessage", "app.[ab]*", "app.b1", "hi"]));
    assert_eq!(publ.cmd(&["PUBLISH", "app.c1", "hi"]), R::I(0));

    // In RESP2, a listening connection only accepts pub/sub commands and PING.
    assert!(matches!(sub.cmd(&["GET", "k"]), R::E(e) if e.contains("Can't execute 'get'")));
    assert_eq!(sub.cmd(&["PING"]), msg(&["pong", ""]));

    assert_eq!(publ.cmd(&["PUBSUB", "CHANNELS"]), msg(&["news", "texts"]));
    assert_eq!(publ.cmd(&["PUBSUB", "CHANNELS", "t*"]), msg(&["texts"]));
    assert_eq!(
        publ.cmd(&["PUBSUB", "NUMSUB", "texts", "ghost"]),
        R::A(Some(vec![b("texts"), R::I(1), b("ghost"), R::I(0)]))
    );
    assert_eq!(publ.cmd(&["PUBSUB", "NUMPAT"]), R::I(1));

    // SELECT does not change the channels, as in Redis.
    assert_eq!(publ.cmd(&["SELECT", "2"]), ok_reply());
    assert_eq!(publ.cmd(&["PUBLISH", "texts", "from db2"]), R::I(1));
    assert_eq!(sub.next(), msg(&["message", "texts", "from db2"]));
    // Another user has channels of their own.
    let mut eve = Redis::connect(port);
    assert_eq!(eve.cmd(&["AUTH", "eve", "evepw"]), ok_reply());
    assert_eq!(eve.cmd(&["PUBLISH", "texts", "spy"]), R::I(0));

    // Messages published in MULTI or a script leave when it commits, and
    // never when the script fails.
    publ.cmd(&["MULTI"]);
    publ.cmd(&["PUBLISH", "texts", "in multi"]);
    assert_eq!(publ.cmd(&["EXEC"]), R::A(Some(vec![R::I(1)])));
    assert_eq!(sub.next(), msg(&["message", "texts", "in multi"]));
    assert!(matches!(publ.cmd(&["EVAL", "redis.call('PUBLISH', 'texts', 'lost') error('boom')", "0"]), R::E(_)));
    assert_eq!(publ.cmd(&["EVAL", "return redis.call('PUBLISH', 'texts', 'lua')", "0"]), R::I(1));
    assert_eq!(sub.next(), msg(&["message", "texts", "lua"]));

    // Unsubscribing confirms each channel; then the connection is normal again.
    assert_eq!(sub.cmd(&["UNSUBSCRIBE"]), notice("unsubscribe", "news", 2));
    assert_eq!(sub.next(), notice("unsubscribe", "texts", 1));
    assert_eq!(sub.cmd(&["PUNSUBSCRIBE"]), notice("punsubscribe", "app.[ab]*", 0));
    assert_eq!(sub.cmd(&["GET", "k"]), R::B(None));
    assert_eq!(publ.cmd(&["PUBLISH", "texts", "x"]), R::I(0));

    // RESP3: messages are pushes, and other commands stay allowed.
    let mut s3 = Redis::connect(port);
    assert!(matches!(s3.cmd(&["HELLO", "3"]), R::M(_)));
    assert_eq!(s3.cmd(&["SUBSCRIBE", "texts"]), R::P(vec![b("subscribe"), b("texts"), R::I(1)]));
    assert_eq!(s3.cmd(&["SET", "k", "v"]), ok_reply());
    assert_eq!(publ.cmd(&["PUBLISH", "texts", "v3"]), R::I(1));
    assert_eq!(s3.next(), R::P(vec![b("message"), b("texts"), b("v3")]));

    // A closed connection stops counting as a subscriber.
    drop(s3);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while publ.cmd(&["PUBLISH", "texts", "gone"]) != R::I(0) {
        assert!(std::time::Instant::now() < deadline, "the closed subscriber is still listed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ints(v: &[i64]) -> R {
    R::A(Some(v.iter().map(|i| R::I(*i)).collect()))
}

fn bulks(v: &[&str]) -> R {
    R::A(Some(v.iter().map(|s| b(s)).collect()))
}

fn is_wrongtype(r: R) -> bool {
    matches!(r, R::E(e) if e.starts_with("WRONGTYPE"))
}

#[test]
fn redis_sets() {
    let dir = TempDir::new("resp_sets");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);

    assert_eq!(r.cmd(&["SADD", "tags", "a", "b", "c", "a"]), R::I(3));
    assert_eq!(r.cmd(&["SADD", "tags", "d"]), R::I(1));
    assert_eq!(r.cmd(&["SCARD", "tags"]), R::I(4));
    assert_eq!(r.cmd(&["SISMEMBER", "tags", "a"]), R::I(1));
    assert_eq!(r.cmd(&["SMISMEMBER", "tags", "a", "z"]), ints(&[1, 0]));
    assert_eq!(r.cmd(&["SREM", "tags", "a", "z"]), R::I(1));
    assert_eq!(r.cmd(&["SMEMBERS", "tags"]), bulks(&["b", "c", "d"]));
    assert_eq!(r.cmd(&["TYPE", "tags"]), R::S("set".into()));
    assert_eq!(r.cmd(&["SMEMBERS", "ghost"]), bulks(&[]));

    // A set is not a hash, a list or a string, and the other way round.
    assert!(is_wrongtype(r.cmd(&["HGET", "tags", "b"])));
    assert!(is_wrongtype(r.cmd(&["GET", "tags"])));
    assert!(is_wrongtype(r.cmd(&["LPUSH", "tags", "x"])));
    r.cmd(&["HSET", "user", "name", "Alice"]);
    assert!(is_wrongtype(r.cmd(&["SADD", "user", "x"])));
    assert!(is_wrongtype(r.cmd(&["ZADD", "tags", "1", "x"])));

    r.cmd(&["SADD", "other", "c", "d", "e"]);
    assert_eq!(r.cmd(&["SINTER", "tags", "other"]), bulks(&["c", "d"]));
    assert_eq!(r.cmd(&["SUNION", "tags", "other"]), bulks(&["b", "c", "d", "e"]));
    assert_eq!(r.cmd(&["SDIFF", "tags", "other"]), bulks(&["b"]));
    assert_eq!(r.cmd(&["SINTERCARD", "2", "tags", "other", "LIMIT", "1"]), R::I(1));
    assert_eq!(r.cmd(&["SUNIONSTORE", "all", "tags", "other"]), R::I(4));
    assert_eq!(r.cmd(&["SDIFFSTORE", "none", "tags", "tags"]), R::I(0));
    assert_eq!(r.cmd(&["EXISTS", "none"]), R::I(0));
    assert_eq!(r.cmd(&["SMOVE", "tags", "other", "b"]), R::I(1));
    assert_eq!(r.cmd(&["SISMEMBER", "other", "b"]), R::I(1));
    assert_eq!(r.cmd(&["SSCAN", "all", "0", "MATCH", "[cd]"]), R::A(Some(vec![b("0"), bulks(&["c", "d"])])));

    // Random members, with and without removal; an emptied set disappears.
    let R::A(Some(five)) = r.cmd(&["SRANDMEMBER", "all", "-5"]) else { panic!() };
    assert_eq!(five.len(), 5);
    let R::A(Some(two)) = r.cmd(&["SPOP", "tags", "5"]) else { panic!() };
    assert_eq!(two.len(), 2);
    assert_eq!(r.cmd(&["EXISTS", "tags"]), R::I(0));

    // The same set from the KivaDB side, saved like any document.
    let mut s = Session::new(inst.clone(), true, None);
    assert!(ok(&mut s, "get other.members").contains(r#"["c","d","e","b"]"#));
}

#[test]
fn redis_sorted_sets() {
    let dir = TempDir::new("resp_zsets");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);

    assert_eq!(r.cmd(&["ZADD", "lb", "10", "alice", "20", "bob", "15", "carol"]), R::I(3));
    assert_eq!(r.cmd(&["ZADD", "lb", "XX", "CH", "25", "bob", "5", "dave"]), R::I(1));
    assert_eq!(r.cmd(&["ZADD", "lb", "NX", "1", "alice"]), R::I(0));
    assert_eq!(r.cmd(&["ZADD", "lb", "GT", "5", "alice"]), R::I(0));
    assert_eq!(r.cmd(&["ZSCORE", "lb", "alice"]), b("10"));
    assert_eq!(r.cmd(&["ZADD", "lb", "LT", "CH", "5", "alice"]), R::I(1));
    assert_eq!(r.cmd(&["ZADD", "lb", "INCR", "2", "alice"]), b("7"));
    assert_eq!(r.cmd(&["ZADD", "lb", "NX", "INCR", "2", "alice"]), R::B(None));
    assert_eq!(r.cmd(&["ZINCRBY", "lb", "1.5", "carol"]), b("16.5"));
    assert_eq!(r.cmd(&["ZMSCORE", "lb", "bob", "nobody"]), R::A(Some(vec![b("25"), R::B(None)])));
    assert_eq!(r.cmd(&["ZCARD", "lb"]), R::I(3));
    assert_eq!(r.cmd(&["TYPE", "lb"]), R::S("zset".into()));

    assert_eq!(r.cmd(&["ZRANGE", "lb", "0", "-1", "WITHSCORES"]), bulks(&["alice", "7", "carol", "16.5", "bob", "25"]));
    assert_eq!(r.cmd(&["ZREVRANGE", "lb", "0", "0"]), bulks(&["bob"]));
    assert_eq!(r.cmd(&["ZRANK", "lb", "carol"]), R::I(1));
    assert_eq!(r.cmd(&["ZREVRANK", "lb", "alice"]), R::I(2));
    assert_eq!(r.cmd(&["ZRANK", "lb", "bob", "WITHSCORE"]), R::A(Some(vec![R::I(2), b("25")])));
    assert_eq!(r.cmd(&["ZRANK", "lb", "nobody"]), R::B(None));
    assert_eq!(r.cmd(&["ZRANGEBYSCORE", "lb", "(7", "+inf"]), bulks(&["carol", "bob"]));
    assert_eq!(r.cmd(&["ZREVRANGEBYSCORE", "lb", "+inf", "-inf", "LIMIT", "1", "1"]), bulks(&["carol"]));
    assert_eq!(r.cmd(&["ZRANGE", "lb", "+inf", "-inf", "BYSCORE", "REV", "LIMIT", "0", "1"]), bulks(&["bob"]));
    assert_eq!(r.cmd(&["ZCOUNT", "lb", "-inf", "20"]), R::I(2));
    assert!(matches!(r.cmd(&["ZRANGE", "lb", "0", "1", "LIMIT", "0", "1"]), R::E(e) if e.contains("LIMIT")));
    assert!(matches!(r.cmd(&["ZADD", "lb", "NX", "XX", "1", "a"]), R::E(e) if e.contains("not compatible")));
    assert!(matches!(r.cmd(&["ZADD", "lb", "abc", "x"]), R::E(e) if e.contains("not a valid float")));
    assert!(matches!(r.cmd(&["ZADD", "lb", "1"]), R::E(e) if e.contains("wrong number of arguments")));
    assert!(matches!(r.cmd(&["ZADD", "lb", "1", "a", "2"]), R::E(e) if e.contains("syntax error")));

    // Combining sorted sets (and plain sets, whose members score 1).
    r.cmd(&["ZADD", "other", "100", "bob", "3", "erin"]);
    r.cmd(&["SADD", "plain", "bob", "zed"]);
    assert_eq!(r.cmd(&["ZUNIONSTORE", "u", "2", "lb", "other", "AGGREGATE", "MAX"]), R::I(4));
    assert_eq!(r.cmd(&["ZRANGE", "u", "-1", "-1", "WITHSCORES"]), bulks(&["bob", "100"]));
    assert_eq!(r.cmd(&["ZINTER", "2", "lb", "other", "WEIGHTS", "2", "1", "WITHSCORES"]), bulks(&["bob", "150"]));
    assert_eq!(r.cmd(&["ZINTERSTORE", "i", "2", "lb", "plain"]), R::I(1));
    assert_eq!(r.cmd(&["ZSCORE", "i", "bob"]), b("26"));
    assert_eq!(r.cmd(&["ZDIFF", "2", "lb", "other"]), bulks(&["alice", "carol"]));
    assert_eq!(r.cmd(&["ZINTERCARD", "2", "lb", "other"]), R::I(1));
    assert_eq!(r.cmd(&["ZRANGESTORE", "top", "lb", "0", "1"]), R::I(2));

    // Lexicographic ranges, among members of equal score.
    r.cmd(&["ZADD", "names", "0", "a", "0", "b", "0", "c", "0", "d"]);
    assert_eq!(r.cmd(&["ZRANGEBYLEX", "names", "[b", "+"]), bulks(&["b", "c", "d"]));
    assert_eq!(r.cmd(&["ZREVRANGEBYLEX", "names", "(c", "-"]), bulks(&["b", "a"]));
    assert_eq!(r.cmd(&["ZLEXCOUNT", "names", "-", "+"]), R::I(4));
    assert_eq!(r.cmd(&["ZREMRANGEBYLEX", "names", "[a", "[b"]), R::I(2));

    // Popping and removing; an emptied sorted set disappears.
    assert_eq!(r.cmd(&["ZPOPMIN", "lb"]), bulks(&["alice", "7"]));
    assert_eq!(r.cmd(&["ZPOPMAX", "lb", "1"]), bulks(&["bob", "25"]));
    assert_eq!(r.cmd(&["ZREMRANGEBYRANK", "lb", "0", "-1"]), R::I(1));
    assert_eq!(r.cmd(&["EXISTS", "lb"]), R::I(0));

    // A sliding-window rate limiter, as libraries write it, inside MULTI.
    for (i, want) in [(1, 1), (2, 2), (3, 3)] {
        r.cmd(&["MULTI"]);
        r.cmd(&["ZREMRANGEBYSCORE", "rl:ip", "0", "1000"]);
        r.cmd(&["ZADD", "rl:ip", &(2000 + i).to_string(), &format!("req{i}")]);
        r.cmd(&["ZCARD", "rl:ip"]);
        r.cmd(&["PEXPIRE", "rl:ip", "60000"]);
        assert_eq!(r.cmd(&["EXEC"]), ints(&[0, 1, want, 1]));
    }
    assert!(matches!(r.cmd(&["TTL", "rl:ip"]), R::I(t) if t > 50));
    assert_eq!(r.cmd(&["EVAL", "return redis.call('ZSCORE', KEYS[1], 'req2')", "1", "rl:ip"]), b("2002"));

    // RESP3: scores are doubles, members of a set a set, WITHSCORES pairs.
    assert!(matches!(r.cmd(&["HELLO", "3"]), R::M(_)));
    r.cmd(&["SADD", "s3", "x"]);
    assert_eq!(r.cmd(&["SMEMBERS", "s3"]), R::Set(vec![b("x")]));
    assert_eq!(r.cmd(&["ZSCORE", "rl:ip", "req1"]), R::D("2001".into()));
    assert_eq!(
        r.cmd(&["ZRANGE", "rl:ip", "0", "1", "WITHSCORES"]),
        R::A(Some(vec![
            R::A(Some(vec![b("req1"), R::D("2001".into())])),
            R::A(Some(vec![b("req2"), R::D("2002".into())])),
        ]))
    );
    assert_eq!(r.cmd(&["ZPOPMAX", "rl:ip"]), R::A(Some(vec![b("req3"), R::D("2003".into())])));
    r.cmd(&["ZADD", "inf", "+inf", "top"]);
    assert_eq!(r.cmd(&["ZSCORE", "inf", "top"]), R::D("inf".into()));
}

#[test]
fn redis_classic_commands_on_strings_keys_hashes_and_lists() {
    let dir = TempDir::new("resp_classic");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);

    // Strings.
    assert_eq!(r.cmd(&["SET", "greet", "Hello World"]), ok_reply());
    assert_eq!(r.cmd(&["GETRANGE", "greet", "0", "4"]), b("Hello"));
    assert_eq!(r.cmd(&["GETRANGE", "greet", "-5", "-1"]), b("World"));
    assert_eq!(r.cmd(&["GETRANGE", "ghost", "0", "-1"]), b(""));
    assert_eq!(r.cmd(&["SETRANGE", "greet", "6", "Kivu!"]), R::I(11));
    assert_eq!(r.cmd(&["GET", "greet"]), b("Hello Kivu!"));
    assert_eq!(r.cmd(&["SETRANGE", "pad", "3", "x"]), R::I(4));
    assert_eq!(r.cmd(&["STRLEN", "pad"]), R::I(4));
    assert_eq!(r.cmd(&["MSETNX", "m1", "a", "m2", "b"]), R::I(1));
    assert_eq!(r.cmd(&["MSETNX", "m2", "z", "m3", "c"]), R::I(0));
    assert_eq!(r.cmd(&["EXISTS", "m3"]), R::I(0));

    // Keys.
    assert_eq!(r.cmd(&["SET", "session", "s1", "EX", "600"]), ok_reply());
    assert_eq!(r.cmd(&["COPY", "session", "session:copy"]), R::I(1));
    assert!(matches!(r.cmd(&["TTL", "session:copy"]), R::I(t) if t > 590));
    assert_eq!(r.cmd(&["COPY", "session", "m1"]), R::I(0));
    assert_eq!(r.cmd(&["COPY", "session", "m1", "REPLACE"]), R::I(1));
    assert_eq!(r.cmd(&["RENAMENX", "m1", "m2"]), R::I(0));
    assert_eq!(r.cmd(&["RENAMENX", "m1", "m9"]), R::I(1));
    assert!(matches!(r.cmd(&["RENAMENX", "ghost", "x"]), R::E(e) if e.contains("no such key")));
    assert_eq!(r.cmd(&["TOUCH", "m9", "m2", "ghost"]), R::I(2));

    // Hashes.
    assert_eq!(r.cmd(&["HSET", "item", "price", "10.5", "name", "maize"]), R::I(2));
    assert_eq!(r.cmd(&["HINCRBYFLOAT", "item", "price", "0.25"]), b("10.75"));
    assert_eq!(r.cmd(&["HINCRBYFLOAT", "item", "stock", "3"]), b("3"));
    assert!(matches!(r.cmd(&["HINCRBYFLOAT", "item", "name", "1"]), R::E(e) if e.contains("not a float")));
    assert_eq!(r.cmd(&["HSTRLEN", "item", "name"]), R::I(5));
    assert_eq!(r.cmd(&["HSTRLEN", "item", "none"]), R::I(0));
    assert_eq!(r.cmd(&["HSCAN", "item", "0", "MATCH", "n*"]), R::A(Some(vec![b("0"), bulks(&["name", "maize"])])));
    assert_eq!(
        r.cmd(&["HSCAN", "item", "0", "NOVALUES"]),
        R::A(Some(vec![b("0"), bulks(&["price", "name", "stock"])]))
    );
    assert!(
        matches!(r.cmd(&["HRANDFIELD", "item"]), R::B(Some(f)) if ["price", "name", "stock"].contains(&f.as_str()))
    );
    match r.cmd(&["HRANDFIELD", "item", "10"]) {
        R::A(Some(v)) => assert_eq!(v.len(), 3),
        other => panic!("{other:?}"),
    }
    match r.cmd(&["HRANDFIELD", "item", "-5", "WITHVALUES"]) {
        R::A(Some(v)) => assert_eq!(v.len(), 10),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.cmd(&["HRANDFIELD", "ghost"]), R::B(None));

    // Lists.
    assert_eq!(r.cmd(&["LPUSHX", "jobs", "a"]), R::I(0));
    assert_eq!(r.cmd(&["EXISTS", "jobs"]), R::I(0));
    assert_eq!(r.cmd(&["RPUSH", "jobs", "a", "b", "c", "b", "d", "b"]), R::I(6));
    assert_eq!(r.cmd(&["RPUSHX", "jobs", "e"]), R::I(7));
    assert_eq!(r.cmd(&["LPOS", "jobs", "b"]), R::I(1));
    assert_eq!(r.cmd(&["LPOS", "jobs", "b", "RANK", "2"]), R::I(3));
    assert_eq!(r.cmd(&["LPOS", "jobs", "b", "RANK", "-1"]), R::I(5));
    assert_eq!(r.cmd(&["LPOS", "jobs", "b", "COUNT", "0"]), ints(&[1, 3, 5]));
    assert_eq!(r.cmd(&["LPOS", "jobs", "b", "COUNT", "0", "MAXLEN", "4"]), ints(&[1, 3]));
    assert_eq!(r.cmd(&["LPOS", "jobs", "zz"]), R::B(None));
    assert!(matches!(r.cmd(&["LPOS", "jobs", "b", "RANK", "0"]), R::E(e) if e.contains("RANK can't be zero")));
    assert_eq!(r.cmd(&["LREM", "jobs", "-2", "b"]), R::I(2));
    assert_eq!(r.cmd(&["LRANGE", "jobs", "0", "-1"]), bulks(&["a", "b", "c", "d", "e"]));
    assert_eq!(r.cmd(&["LSET", "jobs", "-1", "E"]), ok_reply());
    assert!(matches!(r.cmd(&["LSET", "jobs", "9", "x"]), R::E(e) if e.contains("index out of range")));
    assert!(matches!(r.cmd(&["LSET", "ghost", "0", "x"]), R::E(e) if e.contains("no such key")));
    assert_eq!(r.cmd(&["LINSERT", "jobs", "BEFORE", "c", "b2"]), R::I(6));
    assert_eq!(r.cmd(&["LINSERT", "jobs", "AFTER", "zz", "x"]), R::I(-1));
    assert_eq!(r.cmd(&["LINSERT", "ghost", "AFTER", "a", "x"]), R::I(0));
    assert_eq!(r.cmd(&["LRANGE", "jobs", "0", "-1"]), bulks(&["a", "b", "b2", "c", "d", "E"]));
    // A capped list: keep the last 4.
    assert_eq!(r.cmd(&["LTRIM", "jobs", "-4", "-1"]), ok_reply());
    assert_eq!(r.cmd(&["LRANGE", "jobs", "0", "-1"]), bulks(&["b2", "c", "d", "E"]));

    // A reliable queue: move a job to a processing list, atomically.
    assert_eq!(r.cmd(&["LMOVE", "jobs", "processing", "LEFT", "RIGHT"]), b("b2"));
    assert_eq!(r.cmd(&["RPOPLPUSH", "jobs", "processing"]), b("E"));
    assert_eq!(r.cmd(&["LRANGE", "processing", "0", "-1"]), bulks(&["E", "b2"]));
    assert_eq!(r.cmd(&["LMOVE", "jobs", "jobs", "LEFT", "RIGHT"]), b("c"));
    assert_eq!(r.cmd(&["LRANGE", "jobs", "0", "-1"]), bulks(&["d", "c"]));
    assert_eq!(r.cmd(&["LMOVE", "ghost", "processing", "LEFT", "LEFT"]), R::B(None));
    assert_eq!(r.cmd(&["SET", "str", "x"]), ok_reply());
    assert!(matches!(r.cmd(&["LMOVE", "jobs", "str", "LEFT", "LEFT"]), R::E(e) if e.starts_with("WRONGTYPE")));
    assert_eq!(r.cmd(&["LLEN", "jobs"]), R::I(2));
    assert_eq!(
        r.cmd(&["LMPOP", "2", "ghost", "jobs", "RIGHT", "COUNT", "5"]),
        R::A(Some(vec![b("jobs"), bulks(&["c", "d"])]))
    );
    assert_eq!(r.cmd(&["EXISTS", "jobs"]), R::I(0));
    assert_eq!(r.cmd(&["LMPOP", "1", "jobs", "LEFT"]), R::A(None));

    // Inside MULTI and from a script, like any other command.
    assert_eq!(r.cmd(&["MULTI"]), ok_reply());
    assert_eq!(r.cmd(&["RPUSH", "q", "1", "2"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["LMOVE", "q", "q2", "LEFT", "LEFT"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["EXEC"]), R::A(Some(vec![R::I(2), b("1")])));
    assert_eq!(r.cmd(&["EVAL", "return redis.call('LPOS', KEYS[1], '2')", "1", "q"]), R::I(0));
}

#[test]
fn redis_refuses_unsupported_commands_clearly() {
    let dir = TempDir::new("resp_refused");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    let refused = |reply: R, needle: &str| match reply {
        R::E(e) => assert!(e.starts_with("ERR unknown command") && e.contains(needle), "{e}"),
        other => panic!("{other:?}"),
    };

    refused(r.cmd(&["CLUSTER", "INFO"]), "not a Redis Cluster");
    refused(r.cmd(&["FCALL", "f", "0"]), "EVAL");
    refused(r.cmd(&["REPLICAOF", "10.0.0.1", "6379"]), "replica start");
    // Other unknown commands get the plain Redis message.
    assert_eq!(
        r.cmd(&["GEOADD", "m", "1", "2", "x"]),
        R::E("ERR unknown command 'GEOADD', with args beginning with: 'm' '1' '2' 'x' ".into())
    );
    assert_eq!(r.cmd(&["UNWATCH"]), ok_reply());

    // Refused while queuing: the whole transaction is discarded, as in Redis.
    assert_eq!(r.cmd(&["MULTI"]), ok_reply());
    assert_eq!(r.cmd(&["SET", "a", "1"]), R::S("QUEUED".into()));
    refused(r.cmd(&["FCALL", "f", "0"]), "EVAL");
    assert!(matches!(r.cmd(&["EXEC"]), R::E(e) if e.starts_with("EXECABORT")));
    assert_eq!(r.cmd(&["EXISTS", "a"]), R::I(0));
    assert_eq!(r.cmd(&["MULTI"]), ok_reply());
    assert_eq!(r.cmd(&["SET", "a", "1"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["EXEC"]), R::A(Some(vec![ok_reply()])));

    // Scripts go through the same filter.
    refused(r.cmd(&["EVAL", "return redis.pcall('FCALL', 'f', '0')", "0"]), "EVAL or EVALSHA");

    // COMMAND tells clients what is there.
    match (r.cmd(&["COMMAND", "COUNT"]), r.cmd(&["COMMAND", "LIST"])) {
        (R::I(n), R::A(Some(names))) => {
            assert_eq!(n as usize, names.len());
            assert!(names.contains(&b("zadd")) && names.contains(&b("blpop")) && !names.contains(&b("fcall")));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn redis_collections_have_a_size_limit() {
    let dir = TempDir::new("resp_collection_limit");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "config set max_collection_members 3");
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    let too_big = |reply: R, limit: &str| {
        assert!(matches!(&reply, R::E(e) if e.contains(&format!("resp.max_collection_members ({limit})"))), "{reply:?}")
    };

    assert_eq!(r.cmd(&["SADD", "tags", "a", "b", "c"]), R::I(3));
    too_big(r.cmd(&["SADD", "tags", "d"]), "3");
    assert_eq!(r.cmd(&["SADD", "tags", "a"]), R::I(0));
    assert_eq!(r.cmd(&["SCARD", "tags"]), R::I(3));
    too_big(r.cmd(&["ZADD", "lb", "1", "a", "2", "b", "3", "c", "4", "d"]), "3");
    assert_eq!(r.cmd(&["EXISTS", "lb"]), R::I(0));
    too_big(r.cmd(&["RPUSH", "queue", "1", "2", "3", "4"]), "3");
    assert_eq!(r.cmd(&["SADD", "more", "z"]), R::I(1));
    too_big(r.cmd(&["SUNIONSTORE", "all", "tags", "more"]), "3");

    // Lowered below the size of a set: it may still shrink, never grow.
    ok(&mut admin, "config set max_collection_members 1");
    assert_eq!(r.cmd(&["SREM", "tags", "a"]), R::I(1));
    too_big(r.cmd(&["SADD", "tags", "x"]), "1");
    ok(&mut admin, "config set max_collection_members 0");
    assert_eq!(r.cmd(&["SADD", "tags", "x", "y", "z"]), R::I(3));
}

#[test]
fn redis_sorted_sets_survive_a_restart() {
    let dir = TempDir::new("resp_zsets_restart");
    let run = |inst: &std::sync::Arc<kivadb::instance::Instance>, args: &[&str]| {
        let mut c = resp::Conn::new(Session::new(inst.clone(), true, None));
        let mut out = Vec::new();
        c.handle(&args.iter().map(|a| a.as_bytes().to_vec()).collect::<Vec<_>>()).encode(&mut out, false);
        String::from_utf8(out).unwrap()
    };
    {
        let inst = instance(&dir);
        run(&inst, &["ZADD", "scores", "3", "c", "1", "a", "-inf", "low"]);
        run(&inst, &["SADD", "seen", "x", "y"]);
        run(&inst, &["EXPIRE", "seen", "3600"]);
    }
    let inst = instance(&dir);
    assert_eq!(
        run(&inst, &["ZRANGE", "scores", "0", "-1", "WITHSCORES"]),
        "*6\r\n$3\r\nlow\r\n$4\r\n-inf\r\n$1\r\na\r\n$1\r\n1\r\n$1\r\nc\r\n$1\r\n3\r\n"
    );
    assert_eq!(run(&inst, &["SCARD", "seen"]), ":2\r\n");
    let ttl: i64 = run(&inst, &["TTL", "seen"]).trim_start_matches(':').trim().parse().unwrap();
    assert!(ttl > 3500, "{ttl}");
}

#[test]
fn server_sent_events_stream_key_changes() {
    let dir = TempDir::new("sse");
    let inst = instance(&dir);
    let port = http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(b"GET /v1/stream?pattern=order:* HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    let mut reader = BufReader::new(s);
    let mut events = Vec::new();
    let mut head = String::new();
    // Read until the subscription is confirmed.
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        head.push_str(&line);
        if line.starts_with("data: ") {
            break;
        }
    }
    assert!(head.starts_with("HTTP/1.1 200 OK") && head.contains("Content-Type: text/event-stream"), "{head}");
    assert!(head.contains("event: subscribed"));

    let mut sess = Session::new(inst.clone(), true, None);
    ok(&mut sess, r#"set user:1 "ignored" and order:7 {"total": 12}"#);
    ok(&mut sess, "del order:7");
    while events.len() < 2 {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if let Some(data) = line.strip_prefix("data: ") {
            events.push(serde_json::from_str::<Json>(data.trim()).unwrap());
        }
    }
    assert_eq!(
        (&events[0]["event"], &events[0]["key"], &events[0]["type"]),
        (&json!("set"), &json!("order:7"), &json!("object"))
    );
    assert_eq!((&events[1]["event"], &events[1]["key"]), (&json!("del"), &json!("order:7")));
}

#[test]
fn structured_requests_over_http_and_tcp_include_vector_search() {
    let dir = TempDir::new("api_similar");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set doc:1 {"t": "a", "embedding": [1, 0]} and doc:2 {"t": "b", "embedding": [0, 1]}"#);

    let http_port = http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let body = r#"{"op": "similar", "target": "doc:*.embedding", "vector": [0.9, 0.1], "limit": 1}"#;
    let mut c = TcpStream::connect(("127.0.0.1", http_port)).unwrap();
    write!(c, "POST /v1/api HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut raw = String::new();
    c.read_to_string(&mut raw).unwrap();
    let j: Json = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        (&j["status"], &j["results"][0]["key"], &j["results"][0]["value"]["t"]),
        (&json!("ok"), &json!("doc:1"), &json!("a"))
    );

    let tcp_port = server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let mut db = kivadb::client::Client::connect(("127.0.0.1", tcp_port)).unwrap();
    let r = db.api(&json!({"op": "similar", "target": "doc:*.embedding", "vector": [0, 1], "metric": "l2"})).unwrap();
    assert_eq!(r["results"][0]["key"], "doc:2");
    assert_eq!(r["metric"], "l2");
    assert!(db.api(&json!({"op": "similar", "target": "doc:*.embedding", "vector": []})).is_err());
    let stats = db.api(&json!({"op": "stats"})).unwrap();
    assert_eq!(stats["cache_limit_bytes"], 0);
}

#[test]
fn structured_queries_and_plans() {
    let dir = TempDir::new("api_query");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    for i in 1..=6 {
        ok(
            &mut s,
            &format!(
                r#"set user:{i} {{"name": "u{i}", "age": {}, "role": "{}"}}"#,
                20 + i,
                if i % 2 == 0 { "admin" } else { "dev" }
            ),
        );
    }
    let port = server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let mut db = kivadb::client::Client::connect(("127.0.0.1", port)).unwrap();
    let r = db.api(&json!({"op": "query", "target": "user:*.age", "cmp": ">", "value": 23, "limit": 2})).unwrap();
    assert_eq!(r["rows"].as_array().unwrap().len(), 2);
    assert_eq!(r["rows"][0]["key"], "user:4");
    let plan = db.api(&json!({"op": "explain", "target": "user:*.role", "cmp": "=", "value": "admin"})).unwrap();
    assert_eq!(
        (&plan["plan"]["strategy"], &plan["suggestion"]),
        (&json!("PREFIX SCAN"), &json!("create index on user:* (role)"))
    );
    db.exec("create index on user: (role)").unwrap();
    let plan = db.api(&json!({"op": "explain", "target": "user:*.role", "value": "admin"})).unwrap();
    assert_eq!(
        (&plan["plan"]["strategy"], &plan["plan"]["candidates"], &plan["suggestion"]),
        (&json!("INDEX SCAN"), &json!(3), &Json::Null)
    );
    assert!(db.api(&json!({"op": "query", "target": "user:*.age", "cmp": "~", "value": 1})).is_err());
}

/// What one-condition queries cost, to compare across versions.
/// `cargo test --release --test power query_speed -- --ignored --nocapture`
#[test]
#[ignore]
fn query_speed() {
    use kivadb::engine::{CmpOp, Cond};
    let dir = TempDir::new("query_speed");
    let e = Engine::open(dir.path().join("store.kiva"), quiet(256 * 1024 * 1024)).unwrap();
    let n = 100_000;
    for i in 0..n {
        let role = ["dev", "ops", "admin", "sales"][i % 4];
        let doc = json!({"role": role, "age": 18 + i % 60, "city": i % 100});
        e.put(&format!("u:{i:06}"), &Value::Doc(doc), 0, false).unwrap();
    }
    e.create_index("u:*", "city").unwrap();
    let cond = |field: &str, op: CmpOp, value: Json| Cond {
        pattern: "u:*".into(),
        field: field.into(),
        op,
        value,
        func: None,
    };
    let cases = [
        ("index =, 1000 rows", cond("city", CmpOp::Eq, json!(7)), 200),
        ("scan =, 25000 rows", cond("role", CmpOp::Eq, json!("dev")), 10),
        ("scan >, 10 rows (limit)", cond("age", CmpOp::Gt, json!(70)), 200),
    ];
    for (name, c, runs) in cases {
        let limit = if name.contains("limit") { 10 } else { 0 };
        e.query(&c, limit).unwrap();
        let t = std::time::Instant::now();
        for _ in 0..runs {
            std::hint::black_box(e.query(&c, limit).unwrap());
        }
        println!("{name}: {:?} per query", t.elapsed() / runs);
    }
}

/// Data of the filter and quantization benchmarks: embeddings around 200
/// topics, `group` (one in five) and `rare` (one in a hundred) to filter on.
fn bench_corpus(dir: &TempDir) -> (std::sync::Arc<Engine>, Vec<Vec<f32>>) {
    let n: usize = std::env::var("HNSW_N").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let dims: usize = std::env::var("HNSW_DIMS").ok().and_then(|v| v.parse().ok()).unwrap_or(128);
    let e = Engine::open(dir.path().join("store.kiva"), quiet(0)).unwrap();
    let centres = random_vectors(200, dims, 8);
    let noise = random_vectors(n + 50, dims, 5);
    // HNSW_DATA=uniform spreads them evenly instead, the hardest case.
    let uniform = std::env::var("HNSW_DATA").is_ok_and(|d| d == "uniform");
    let around = |i: usize| -> Vec<f32> {
        if uniform {
            return noise[i].clone();
        }
        centres[i % 200].iter().zip(&noise[i]).map(|(c, x)| 2.0 * c + (x - 0.25) * 0.8).collect()
    };
    for i in 0..n {
        embed(&e, &format!("chunk:{i:07}"), &around(i), json!({ "group": i % 5, "rare": i % 100 == 7 }));
    }
    (e, (n..n + 50).map(around).collect())
}

/// Recall@10 against the exact filtered answer, time and documents read per search.
fn filtered_run(e: &Engine, queries: &[Vec<f32>], filter: &[kivadb::engine::Cond]) -> (f64, Duration, usize) {
    let (mut hits, mut read, mut time) = (0, 0, Duration::ZERO);
    for q in queries {
        let mut exact = query("chunk:*", q.clone(), Metric::Cosine, 10);
        exact.exact = true;
        exact.filter = filter.to_vec();
        let truth: Vec<String> = e.similar(&exact).unwrap().rows.into_iter().map(|r| r.0).collect();
        let mut vq = query("chunk:*", q.clone(), Metric::Cosine, 10);
        vq.filter = filter.to_vec();
        let t = std::time::Instant::now();
        let r = e.similar(&vq).unwrap();
        time += t.elapsed();
        let how = r.filter.unwrap_or_default();
        read += how.split(", ").find_map(|p| p.strip_suffix(" documents read")?.parse::<usize>().ok()).unwrap_or(0);
        hits += r.rows.iter().filter(|row| truth.contains(&row.0)).count();
    }
    let n = queries.len();
    (hits as f64 / (n * 10) as f64, time / n as u32, read / n)
}

/// Vector search with a filter on other fields, through an HNSW graph.
/// `cargo test --release --test power hnsw_filter_speed -- --ignored --nocapture`
#[test]
#[ignore]
fn hnsw_filter_speed() {
    use kivadb::engine::{CmpOp, Cond};
    let dir = TempDir::new("hnsw_filter_speed");
    let (e, queries) = bench_corpus(&dir);
    e.create_vector_index_with("chunk:*", "embedding", hnsw(Metric::Cosine)).unwrap();
    let cond = |field: &str, value: Json| Cond {
        pattern: "chunk:*".into(),
        field: field.into(),
        op: CmpOp::Eq,
        value,
        func: None,
    };
    let cases =
        [("group = 2 (1 in 5)", cond("group", json!(2))), ("rare = true (1 in 100)", cond("rare", json!(true)))];
    for (name, c) in &cases {
        let (recall, t, read) = filtered_run(&e, &queries, std::slice::from_ref(c));
        println!("{name}, no secondary index: recall@10 {recall:.3}, {t:?} per search, {read} documents read");
    }
    e.create_index("chunk:*", "group").unwrap();
    let (recall, t, read) = filtered_run(&e, &queries, std::slice::from_ref(&cases[0].1));
    println!("group = 2, with an index on group: recall@10 {recall:.3}, {t:?} per search, {read} documents read");
}

/// Memory and accuracy of a scalar-quantized HNSW graph.
/// `cargo test --release --test power hnsw_sq8_speed -- --ignored --nocapture`
#[test]
#[ignore]
fn hnsw_sq8_speed() {
    use kivadb::hnsw::Quant;
    let dir = TempDir::new("hnsw_sq8_speed");
    let (e, queries) = bench_corpus(&dir);
    let only_sq8 = std::env::var("HNSW_ONLY_SQ8").is_ok();
    for quant in [Quant::None, Quant::Sq8].into_iter().filter(|q| !only_sq8 || *q == Quant::Sq8) {
        let start = std::time::Instant::now();
        let kind = VectorIndexKind::Hnsw(HnswParams { quant, ..Default::default() });
        e.create_vector_index_with("chunk:*", "embedding", kind).unwrap();
        let built = start.elapsed();
        let memory = e.list_vector_indexes()[0].memory_bytes;
        let (recall, t, _) = filtered_run(&e, &queries, &[]);
        println!(
            "{}: graph built in {built:?}, {} MB, recall@10 {recall:.3}, {t:?} per search",
            quant.name(),
            memory / 1_000_000
        );
        e.drop_vector_index("chunk:*", "embedding").unwrap();
    }
}

#[test]
fn quantized_graphs_rank_with_the_full_vectors_and_filters_apply_while_walking() {
    use kivadb::engine::{CmpOp, Cond};
    use kivadb::hnsw::Quant;
    let dir = TempDir::new("hnsw_sq8");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet(0)).unwrap();
    let (n, dims) = (2000, 24);
    let vectors = random_vectors(n, dims, 17);
    for (i, v) in vectors.iter().enumerate() {
        embed(&e, &format!("chunk:{i:05}"), v, json!({ "group": i % 5, "rare": i % 100 == 3 }));
    }
    let sq8 = VectorIndexKind::Hnsw(HnswParams { quant: Quant::Sq8, ..Default::default() });
    e.create_vector_index_with("chunk:*", "embedding", hnsw(Metric::Cosine)).unwrap();
    let full_memory = e.list_vector_indexes()[0].memory_bytes;
    e.drop_vector_index("chunk:*", "embedding").unwrap();
    e.create_vector_index_with("chunk:*", "embedding", sq8).unwrap();
    // One byte per number instead of four, and 12 bytes of range per vector.
    assert_eq!(full_memory - e.list_vector_indexes()[0].memory_bytes, n * (dims * 3 - 12));

    // The scores are those of the documents' vectors: identical to an exact search.
    let exact_rows = |q: &VectorQuery| {
        let mut x = q.clone();
        x.exact = true;
        e.similar(&x).unwrap()
    };
    let q = query("chunk:*", vectors[42].clone(), Metric::Cosine, 5);
    let (approx, exact) = (e.similar(&q).unwrap(), exact_rows(&q));
    assert!(approx.approximate && !exact.indexed, "exact reads the documents, not the codes");
    assert_eq!(approx.rows[0].0, "chunk:00042");
    for row in &approx.rows {
        if let Some(t) = exact.rows.iter().find(|r| r.0 == row.0) {
            assert_eq!(t.1, row.1, "{}", row.0);
        }
    }
    let mut hits = 0;
    for qv in random_vectors(20, dims, 3) {
        let q = query("chunk:*", qv, Metric::Cosine, 10);
        let truth: Vec<String> = exact_rows(&q).rows.into_iter().map(|r| r.0).collect();
        hits += e.similar(&q).unwrap().rows.iter().filter(|r| truth.contains(&r.0)).count();
    }
    assert!(hits >= 180, "recall@10 {hits}/200");

    // Filters are tested while the graph is walked: enough results, all passing.
    let cond = |field: &str, value: Json| Cond {
        pattern: "chunk:*".into(),
        field: field.into(),
        op: CmpOp::Eq,
        value,
        func: None,
    };
    for c in [cond("group", json!(2)), cond("rare", json!(true))] {
        let mut q = query("chunk:*", vectors[7].clone(), Metric::Cosine, 10);
        q.filter = vec![c.clone()];
        let r = e.similar(&q).unwrap();
        assert_eq!(r.rows.len(), 10);
        let path = kivadb::jsonpath::parse_path(&c.field).unwrap();
        assert!(r.rows.iter().all(|row| kivadb::jsonpath::get(&row.2, &path) == Some(&c.value)));
        assert!(r.filter.as_deref().unwrap().starts_with("tested while walking the graph,"), "{:?}", r.filter);
        let truth: Vec<String> = exact_rows(&q).rows.into_iter().map(|r| r.0).collect();
        assert!(r.rows.iter().filter(|row| truth.contains(&row.0)).count() >= 8, "{c:?}");
    }
    // An index on the field gives a mask: no document read to test it.
    e.create_index("chunk:*", "group").unwrap();
    let mut q = query("chunk:*", vectors[7].clone(), Metric::Cosine, 10);
    q.filter = vec![cond("group", json!(2))];
    let how = e.similar(&q).unwrap().filter.unwrap();
    assert!(how.starts_with("tested while walking the graph and the secondary indexes"), "{how}");

    // The quantization is kept across a restart, graph file or not.
    e.close();
    drop(e);
    let e = Engine::open(&path, quiet(0)).unwrap();
    assert_eq!(e.list_vector_indexes()[0].kind, sq8);
    assert_eq!(e.similar(&query("chunk:*", vectors[42].clone(), Metric::Cosine, 1)).unwrap().rows[0].0, "chunk:00042");
}

#[test]
fn quantization_and_filtered_graph_search_from_the_shell() {
    let dir = TempDir::new("hnsw_sq8_shell");
    let mut s = Session::new(instance(&dir), true, None);
    for (i, v) in random_vectors(400, 8, 9).iter().enumerate() {
        let lang = ["fr", "en"][i % 2];
        ok(&mut s, &format!("set doc:{i:03} {}", json!({ "embedding": v, "lang": lang })));
    }
    let out = ok(&mut s, "create vector index on doc: (embedding) using hnsw quantize sq8");
    assert!(out.contains("sq8: one byte per number, re-ranked with the documents' vectors"), "{out}");
    assert!(ok(&mut s, "indexes").contains("ef_search 100, sq8)"));
    let out = ok(&mut s, r#"get similar doc:*.embedding to doc:004.embedding where lang = "fr" limit 3 return lang"#);
    assert!(out.starts_with("doc:004: 1.0000"), "{out}");
    assert_eq!(out.matches(r#"{"lang":"fr"}"#).count(), 3, "{out}");
    assert!(out.contains("filter tested while walking the graph, "), "{out}");
    assert!(fails(&mut s, "create vector index on doc: (other) using hnsw quantize pq").contains("sq8 or none"));
}

#[test]
fn redis_watch_makes_exec_fail_when_a_watched_key_changes() {
    let dir = TempDir::new("resp_watch");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let (mut a, mut other) = (Redis::connect(port), Redis::connect(port));
    let ok = || R::S("OK".into());
    a.cmd(&["SET", "balance", "100"]);

    // Changed by another client between WATCH and EXEC: nothing is applied.
    assert_eq!(a.cmd(&["WATCH", "balance"]), ok());
    assert_eq!(other.cmd(&["INCRBY", "balance", "5"]), R::I(105));
    assert_eq!(a.cmd(&["MULTI"]), ok());
    assert_eq!(a.cmd(&["SET", "balance", "0"]), R::S("QUEUED".into()));
    assert_eq!(a.cmd(&["SET", "audit", "x"]), R::S("QUEUED".into()));
    assert_eq!(a.cmd(&["EXEC"]), R::A(None));
    assert_eq!(a.cmd(&["GET", "balance"]), b("105"));
    assert_eq!(a.cmd(&["EXISTS", "audit"]), R::I(0));

    // Untouched: the transaction runs. The watch ends with EXEC.
    a.cmd(&["WATCH", "balance", "other"]);
    a.cmd(&["MULTI"]);
    a.cmd(&["DECRBY", "balance", "5"]);
    assert_eq!(a.cmd(&["EXEC"]), R::A(Some(vec![R::I(100)])));
    other.cmd(&["SET", "balance", "1"]);
    a.cmd(&["MULTI"]);
    a.cmd(&["GET", "balance"]);
    assert_eq!(a.cmd(&["EXEC"]), R::A(Some(vec![b("1")])), "no longer watched after EXEC");

    // A deleted, a created and an expired key all count as changed.
    for change in [&["DEL", "balance"][..], &["SET", "fresh", "1"][..]] {
        a.cmd(&["WATCH", "balance", "fresh"]);
        other.cmd(change);
        a.cmd(&["MULTI"]);
        a.cmd(&["PING"]);
        assert_eq!(a.cmd(&["EXEC"]), R::A(None), "{change:?}");
    }
    a.cmd(&["SET", "ephemeral", "1", "PX", "100"]);
    a.cmd(&["WATCH", "ephemeral"]);
    std::thread::sleep(Duration::from_millis(200));
    a.cmd(&["MULTI"]);
    a.cmd(&["PING"]);
    assert_eq!(a.cmd(&["EXEC"]), R::A(None), "expired");

    // UNWATCH and DISCARD forget the keys; WATCH inside MULTI is an error.
    a.cmd(&["WATCH", "k"]);
    assert_eq!(a.cmd(&["UNWATCH"]), ok());
    other.cmd(&["SET", "k", "1"]);
    a.cmd(&["MULTI"]);
    assert!(matches!(a.cmd(&["WATCH", "k"]), R::E(e) if e.contains("inside MULTI")));
    a.cmd(&["INCR", "k"]);
    assert_eq!(a.cmd(&["EXEC"]), R::A(Some(vec![R::I(2)])));
    a.cmd(&["WATCH", "k"]);
    a.cmd(&["MULTI"]);
    assert_eq!(a.cmd(&["DISCARD"]), ok());
    other.cmd(&["SET", "k", "9"]);
    a.cmd(&["MULTI"]);
    a.cmd(&["GET", "k"]);
    assert_eq!(a.cmd(&["EXEC"]), R::A(Some(vec![b("9")])));

    // A change from the KivaDB shell counts too.
    a.cmd(&["WATCH", "k"]);
    let mut s = Session::new(inst.clone(), true, None);
    ok_shell(&mut s, r#"update k "10""#);
    a.cmd(&["MULTI"]);
    a.cmd(&["INCR", "k"]);
    assert_eq!(a.cmd(&["EXEC"]), R::A(None));
    // WATCH, then SELECT another database: the key of the first is watched.
    a.cmd(&["WATCH", "k"]);
    a.cmd(&["SELECT", "1"]);
    other.cmd(&["SET", "k", "11"]);
    a.cmd(&["MULTI"]);
    a.cmd(&["SET", "elsewhere", "1"]);
    assert_eq!(a.cmd(&["EXEC"]), R::A(None));
}

fn ok_shell(s: &mut Session, cmd: &str) -> String {
    ok(s, cmd)
}

#[test]
fn redis_blocking_commands_wait_for_data() {
    let dir = TempDir::new("resp_blocking");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    let in_thread = |cmd: Vec<&'static str>| {
        std::thread::spawn(move || {
            let mut c = Redis::connect(port);
            let t = std::time::Instant::now();
            (c.cmd(&cmd), t.elapsed())
        })
    };

    // Data already there: no wait. Keys are tried in order.
    r.cmd(&["RPUSH", "low", "a"]);
    r.cmd(&["RPUSH", "high", "b"]);
    assert_eq!(r.cmd(&["BLPOP", "high", "low", "1"]), R::A(Some(vec![b("high"), b("b")])));
    assert_eq!(r.cmd(&["BRPOP", "high", "low", "1"]), R::A(Some(vec![b("low"), b("a")])));

    // Nothing: the client waits, others keep working, a push wakes it.
    let waiting = in_thread(vec!["BLPOP", "jobs", "5"]);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(r.cmd(&["PING"]), R::S("PONG".into()), "the turn is not held while waiting");
    r.cmd(&["RPUSH", "jobs", "job-1"]);
    let (got, took) = waiting.join().unwrap();
    assert_eq!(got, R::A(Some(vec![b("jobs"), b("job-1")])));
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert_eq!(r.cmd(&["EXISTS", "jobs"]), R::I(0));

    // A timeout answers nil; fractions of a second work.
    let t = std::time::Instant::now();
    assert_eq!(r.cmd(&["BLPOP", "empty", "0.2"]), R::A(None));
    assert!(t.elapsed() >= Duration::from_millis(190) && t.elapsed() < Duration::from_secs(2));
    assert!(matches!(r.cmd(&["BLPOP", "q", "-1"]), R::E(e) if e.contains("negative")));
    assert!(matches!(r.cmd(&["BLPOP", "q", "soon"]), R::E(e) if e.contains("not a float")));

    // A write from the KivaDB shell wakes it too.
    let waiting = in_thread(vec!["BRPOP", "shell-q", "5"]);
    std::thread::sleep(Duration::from_millis(150));
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set shell-q ["from the shell"]"#);
    assert_eq!(waiting.join().unwrap().0, R::A(Some(vec![b("shell-q"), b("from the shell")])));

    // BLMOVE, BRPOPLPUSH, BLMPOP, BZPOPMIN, BZPOPMAX, BZMPOP.
    let waiting = in_thread(vec!["BLMOVE", "src", "dst", "LEFT", "RIGHT", "5"]);
    std::thread::sleep(Duration::from_millis(100));
    r.cmd(&["RPUSH", "src", "x", "y"]);
    assert_eq!(waiting.join().unwrap().0, b("x"));
    assert_eq!(r.cmd(&["BRPOPLPUSH", "src", "dst", "1"]), b("y"));
    assert_eq!(r.cmd(&["LRANGE", "dst", "0", "-1"]), R::A(Some(vec![b("y"), b("x")])));
    assert_eq!(r.cmd(&["BRPOPLPUSH", "src", "dst", "0.1"]), R::B(None));
    assert_eq!(
        r.cmd(&["BLMPOP", "1", "2", "none", "dst", "RIGHT", "COUNT", "5"]),
        R::A(Some(vec![b("dst"), R::A(Some(vec![b("x"), b("y")]))]))
    );
    let waiting = in_thread(vec!["BZPOPMIN", "board", "5"]);
    std::thread::sleep(Duration::from_millis(100));
    r.cmd(&["ZADD", "board", "3", "c", "1", "a", "2", "b"]);
    assert_eq!(waiting.join().unwrap().0, R::A(Some(vec![b("board"), b("a"), b("1")])));
    assert_eq!(r.cmd(&["BZPOPMAX", "board", "1"]), R::A(Some(vec![b("board"), b("c"), b("3")])));
    assert_eq!(
        r.cmd(&["BZMPOP", "1", "1", "board", "MIN"]),
        R::A(Some(vec![b("board"), R::A(Some(vec![R::A(Some(vec![b("b"), b("2")]))]))]))
    );

    // Inside MULTI, a blocking command does not wait.
    r.cmd(&["MULTI"]);
    r.cmd(&["BLPOP", "nothing", "0"]);
    assert_eq!(r.cmd(&["EXEC"]), R::A(Some(vec![R::A(None)])));
    r.cmd(&["SET", "str", "v"]);
    assert!(matches!(r.cmd(&["BLPOP", "str", "1"]), R::E(e) if e.starts_with("WRONGTYPE")));

    // A client that left takes nothing: the value stays for the others.
    let gone = std::thread::spawn(move || {
        let mut c = Redis::connect(port);
        let mut req = String::new();
        for a in ["BLPOP", "orphan", "0"] {
            req.push_str(&format!("${}\r\n{a}\r\n", a.len()));
        }
        c.stream.write_all(format!("*3\r\n{req}").as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(c);
    });
    gone.join().unwrap();
    std::thread::sleep(Duration::from_millis(400));
    r.cmd(&["RPUSH", "orphan", "kept"]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(r.cmd(&["LRANGE", "orphan", "0", "-1"]), R::A(Some(vec![b("kept")])));

    // WAIT: no replica, nothing to wait for.
    assert_eq!(r.cmd(&["WAIT", "0", "100"]), R::I(0));
}

#[test]
fn redis_streams_and_consumer_groups() {
    let dir = TempDir::new("resp_streams");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    let id = |reply: R| match reply {
        R::B(Some(s)) => s,
        other => panic!("{other:?}"),
    };
    let entry = |i: &str, f: &[&str]| R::A(Some(vec![b(i), R::A(Some(f.iter().map(|x| b(x)).collect()))]));

    // XADD with explicit and generated ids.
    assert_eq!(r.cmd(&["XADD", "s", "1-1", "temp", "20"]), b("1-1"));
    assert_eq!(r.cmd(&["XADD", "s", "1-*", "temp", "21"]), b("1-2"));
    assert!(matches!(r.cmd(&["XADD", "s", "1-1", "x", "y"]), R::E(e) if e.contains("equal or smaller")));
    assert!(matches!(r.cmd(&["XADD", "t", "0-0", "x", "y"]), R::E(e) if e.contains("greater than 0-0")));
    assert!(matches!(r.cmd(&["XADD", "s", "*", "odd"]), R::E(e) if e.contains("wrong number")));
    let auto = id(r.cmd(&["XADD", "s", "*", "temp", "22", "unit", "C"]));
    assert!(auto.split('-').next().unwrap().parse::<i64>().unwrap() >= kivadb::util::now_ms() - 5000);
    assert_eq!(r.cmd(&["XLEN", "s"]), R::I(3));
    assert_eq!(r.cmd(&["TYPE", "s"]), R::S("stream".into()));
    assert!(matches!(r.cmd(&["HGET", "s", "x"]), R::E(e) if e.starts_with("WRONGTYPE")));
    assert_eq!(r.cmd(&["XADD", "absent", "NOMKSTREAM", "*", "a", "b"]), R::B(None));

    // Ranges, forwards and backwards, inclusive and exclusive, with COUNT.
    assert_eq!(
        r.cmd(&["XRANGE", "s", "-", "1"]),
        R::A(Some(vec![entry("1-1", &["temp", "20"]), entry("1-2", &["temp", "21"])]))
    );
    assert_eq!(r.cmd(&["XRANGE", "s", "(1-1", "+", "COUNT", "1"]), R::A(Some(vec![entry("1-2", &["temp", "21"])])));
    let R::A(Some(rev)) = r.cmd(&["XREVRANGE", "s", "+", "-", "COUNT", "2"]) else { panic!() };
    assert_eq!(rev[1], entry("1-2", &["temp", "21"]));

    // XDEL, XTRIM, MAXLEN on XADD.
    assert_eq!(r.cmd(&["XDEL", "s", "1-1", "9-9"]), R::I(1));
    for i in 0..10 {
        r.cmd(&["XADD", "log", "MAXLEN", "~", "5", "*", "n", &i.to_string()]);
    }
    assert_eq!(r.cmd(&["XLEN", "log"]), R::I(5));
    assert_eq!(r.cmd(&["XTRIM", "log", "MAXLEN", "2"]), R::I(3));
    let R::A(Some(left)) = r.cmd(&["XRANGE", "log", "-", "+"]) else { panic!() };
    assert_eq!(left.len(), 2);

    // XREAD after an id; BLOCK with $ waits for what comes next.
    let R::A(Some(read)) = r.cmd(&["XREAD", "COUNT", "10", "STREAMS", "s", "0"]) else { panic!() };
    assert_eq!(read.len(), 1);
    assert_eq!(r.cmd(&["XREAD", "STREAMS", "s", "$"]), R::A(None));
    let waiting = std::thread::spawn(move || {
        let mut c = Redis::connect(port);
        c.cmd(&["XREAD", "BLOCK", "5000", "STREAMS", "s", "$"])
    });
    std::thread::sleep(Duration::from_millis(200));
    let new = id(r.cmd(&["XADD", "s", "*", "temp", "23"]));
    let R::A(Some(got)) = waiting.join().unwrap() else { panic!() };
    assert_eq!(got, vec![R::A(Some(vec![b("s"), R::A(Some(vec![entry(&new, &["temp", "23"])]))]))]);
    let t = std::time::Instant::now();
    assert_eq!(r.cmd(&["XREAD", "BLOCK", "150", "STREAMS", "s", "$"]), R::A(None));
    assert!(t.elapsed() >= Duration::from_millis(140));

    // Consumer groups: each entry goes to one consumer, until acknowledged.
    assert_eq!(r.cmd(&["XGROUP", "CREATE", "jobs", "workers", "$", "MKSTREAM"]), R::S("OK".into()));
    assert!(matches!(r.cmd(&["XGROUP", "CREATE", "jobs", "workers", "$"]), R::E(e) if e.starts_with("BUSYGROUP")));
    for j in ["a", "b", "c"] {
        r.cmd(&["XADD", "jobs", "*", "job", j]);
    }
    let R::A(Some(first)) = r.cmd(&["XREADGROUP", "GROUP", "workers", "w1", "COUNT", "2", "STREAMS", "jobs", ">"])
    else {
        panic!()
    };
    let R::A(Some(ref stream)) = first[0] else { panic!() };
    let R::A(Some(ref items)) = stream[1] else { panic!() };
    assert_eq!(items.len(), 2);
    let R::A(Some(ref e0)) = items[0] else { panic!() };
    let R::B(Some(ref id_a)) = e0[0] else { panic!() };
    let R::A(Some(second)) = r.cmd(&["XREADGROUP", "GROUP", "workers", "w2", "STREAMS", "jobs", ">"]) else { panic!() };
    let R::A(Some(ref s2)) = second[0] else { panic!() };
    assert!(matches!(&s2[1], R::A(Some(v)) if v.len() == 1), "w2 gets only the third");
    assert_eq!(r.cmd(&["XREADGROUP", "GROUP", "workers", "w2", "STREAMS", "jobs", ">"]), R::A(None));
    let R::A(Some(summary)) = r.cmd(&["XPENDING", "jobs", "workers"]) else { panic!() };
    assert_eq!(summary[0], R::I(3));
    assert_eq!(r.cmd(&["XACK", "jobs", "workers", id_a]), R::I(1));
    assert_eq!(r.cmd(&["XACK", "jobs", "workers", id_a]), R::I(0));
    let R::A(Some(rows)) = r.cmd(&["XPENDING", "jobs", "workers", "-", "+", "10", "w1"]) else { panic!() };
    assert_eq!(rows.len(), 1);
    // The history of w1: what it was given and did not acknowledge.
    let R::A(Some(hist)) = r.cmd(&["XREADGROUP", "GROUP", "workers", "w1", "STREAMS", "jobs", "0"]) else { panic!() };
    let R::A(Some(ref h)) = hist[0] else { panic!() };
    assert!(matches!(&h[1], R::A(Some(v)) if v.len() == 1));
    // Another consumer takes over what was left idle.
    std::thread::sleep(Duration::from_millis(50));
    let R::A(Some(claimed)) = r.cmd(&["XAUTOCLAIM", "jobs", "workers", "w3", "10", "0", "COUNT", "10", "JUSTID"])
    else {
        panic!()
    };
    assert!(matches!(&claimed[1], R::A(Some(v)) if v.len() == 2), "{claimed:?}");
    let R::A(Some(rows)) = r.cmd(&["XPENDING", "jobs", "workers", "-", "+", "10", "w3"]) else { panic!() };
    let R::A(Some(ref row)) = rows[0] else { panic!() };
    let R::B(Some(ref pid)) = row[0] else { panic!() };
    assert_eq!(r.cmd(&["XCLAIM", "jobs", "workers", "w1", "0", pid, "JUSTID"]), R::A(Some(vec![b(pid)])));

    // A blocked group reader is woken by XADD.
    let waiting = std::thread::spawn(move || {
        let mut c = Redis::connect(port);
        c.cmd(&["XREADGROUP", "GROUP", "workers", "w4", "BLOCK", "5000", "STREAMS", "jobs", ">"])
    });
    std::thread::sleep(Duration::from_millis(200));
    r.cmd(&["XADD", "jobs", "*", "job", "d"]);
    assert!(matches!(waiting.join().unwrap(), R::A(Some(v)) if v.len() == 1));

    // XINFO.
    let R::A(Some(info)) = r.cmd(&["XINFO", "STREAM", "jobs"]) else { panic!() };
    assert_eq!(info[0], b("length"));
    assert_eq!(info[1], R::I(4));
    let R::A(Some(groups)) = r.cmd(&["XINFO", "GROUPS", "jobs"]) else { panic!() };
    assert_eq!(groups.len(), 1);
    let R::A(Some(consumers)) = r.cmd(&["XINFO", "CONSUMERS", "jobs", "workers"]) else { panic!() };
    assert_eq!(consumers.len(), 4);
    assert!(
        matches!(r.cmd(&["XREADGROUP", "GROUP", "nope", "c", "STREAMS", "jobs", ">"]), R::E(e) if e.starts_with("NOGROUP"))
    );
    assert_eq!(r.cmd(&["XGROUP", "DESTROY", "jobs", "workers"]), R::I(1));

    // A stream is a KivaDB document, readable from the shell.
    let mut s = Session::new(inst.clone(), true, None);
    assert!(ok(&mut s, "get jobs.redis").contains("stream"));
}

/// Waits for the background tuning of the first vector index to reach `size`.
fn tuned_at(e: &Engine, size: usize) -> kivadb::engine::AutoTune {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let a = e.list_vector_indexes()[0].auto_tune.clone().expect("kept tuned");
        if a.tuned_at == size || std::time::Instant::now() > until {
            return a;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn an_hnsw_index_kept_tuned_is_measured_again_as_it_grows_and_shrinks() {
    let dir = TempDir::new("hnsw_auto_tune");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, quiet(0)).unwrap();
    let vectors = random_vectors(1700, 16, 41);
    for (i, v) in vectors[..400].iter().enumerate() {
        embed(&e, &format!("chunk:{i:05}"), v, json!({}));
    }
    let small = VectorIndexKind::Hnsw(HnswParams { m: 4, ef_construction: 20, ..Default::default() });
    e.create_vector_index_with("chunk:*", "embedding", small).unwrap();
    let auto = AutoTune { recall: 0.95, limit: 10, sample: 30, queries: None, tuned_at: 0 };
    e.set_vector_auto_tune("chunk:*", "embedding", Some(auto.clone())).unwrap();
    // Due at once (never tuned): the next write starts it.
    embed(&e, "chunk:00400", &vectors[400], json!({}));
    assert_eq!(tuned_at(&e, 401).tuned_at, 401);

    // Not again before the size doubles.
    for (i, v) in vectors[401..801].iter().enumerate() {
        embed(&e, &format!("chunk:{:05}", 401 + i), v, json!({}));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(e.list_vector_indexes()[0].auto_tune.as_ref().unwrap().tuned_at, 401, "801 vectors: not yet");
    embed(&e, "chunk:00801", &vectors[801], json!({}));
    assert_eq!(tuned_at(&e, 802).tuned_at, 802);

    // Saved with the index.
    drop(e);
    let e = Engine::open(&path, quiet(0)).unwrap();
    assert_eq!(e.list_vector_indexes()[0].auto_tune, Some(AutoTune { tuned_at: 802, ..auto.clone() }));

    // Down to half: measured again.
    for i in 0..401 {
        e.delete(&format!("chunk:{i:05}")).unwrap();
    }
    assert_eq!(tuned_at(&e, 401).tuned_at, 401);

    // A value set by hand stops it.
    e.set_vector_ef_search("chunk:*", "embedding", 64).unwrap();
    assert_eq!(e.list_vector_indexes()[0].auto_tune, None);
    assert_eq!(e.list_vector_indexes()[0].ef_search, 64);
}

/// Tuning searches one query at a time: writes go on meanwhile.
#[test]
fn tuning_lets_the_workspace_write_meanwhile() {
    let dir = TempDir::new("hnsw_tune_writes");
    let e = Engine::open(dir.path().join("store.kiva"), quiet(0)).unwrap();
    for (i, v) in random_vectors(4000, 64, 9).iter().enumerate() {
        embed(&e, &format!("chunk:{i:05}"), v, json!({}));
    }
    let hnsw = VectorIndexKind::Hnsw(HnswParams { m: 8, ef_construction: 40, ..Default::default() });
    e.create_vector_index_with("chunk:*", "embedding", hnsw).unwrap();
    let tuner = {
        let e = e.clone();
        std::thread::spawn(move || e.tune_vector_index("chunk:*", "embedding", 1.0, 10, 300, None).unwrap())
    };
    // Counted in sections of the tuning (one search under the lock each)
    // that began while the write waited for the lock, not in milliseconds: a
    // loaded machine may pause any thread, for any time, but the tuner must
    // let a waiting write in before its next search.
    let (mut writes, mut slowest) = (0, 0);
    while !tuner.is_finished() {
        e.put(&format!("other:{writes}"), &Value::Str("x".into()), 0, false).unwrap();
        slowest = slowest.max(Engine::sections_waited());
        writes += 1;
    }
    let r = tuner.join().unwrap();
    let sections = e.tune_sections();
    assert!(sections >= 600, "{sections} sections ({} steps)", r.steps.len());
    assert!(slowest <= 1, "a write waited for {slowest} sections of the tuning");
    assert!(writes > 100, "{writes} writes during {sections} sections");
}

#[test]
fn an_index_kept_tuned_from_the_shell() {
    let dir = TempDir::new("hnsw_auto_shell");
    let mut s = Session::new(instance(&dir), true, None);
    ok(&mut s, "create vector index on doc: (embedding) using hnsw m 8 ef 50");
    // Empty for now: tuned once it can be.
    let out = ok(&mut s, "tune vector index on doc: (embedding) recall 0.9 limit 5 sample 20 auto");
    assert!(out.contains("it will be tuned for a recall of 90 % once it has more"), "{out}");
    assert!(ok(&mut s, "indexes").contains("tuned again as it grows (recall 90 %, once it has enough vectors)"));
    for (i, v) in random_vectors(40, 8, 5).iter().enumerate() {
        ok(&mut s, &format!("set doc:{i:03} {}", json!({ "embedding": v })));
    }
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while !ok(&mut s, "indexes").contains(", last at ") && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let shown = ok(&mut s, "indexes");
    assert!(shown.contains("tuned again as it grows (recall 90 %, last at "), "{shown}");

    let out = ok(&mut s, "tune vector index on doc: (embedding) limit 5 sample 20 auto");
    assert!(
        out.contains("Kept ef_search")
            && out.contains("measured again in the background when the index reaches 80 or falls to 20 vectors"),
        "{out}"
    );
    let out = ok(&mut s, "tune vector index on doc: (embedding) auto off");
    assert!(out.contains("keeps its ef_search as it grows"), "{out}");
    assert!(!ok(&mut s, "indexes").contains("tuned again"));
    assert!(fails(&mut s, "tune vector index on doc: (embedding) auto off now").contains("Unexpected"));
}
