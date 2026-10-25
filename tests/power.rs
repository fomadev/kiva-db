//! Value cache, vector search, Redis compatibility and Server-Sent Events.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::engine::{Engine, EngineOptions, VectorIndexKind, VectorQuery};
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
    e.close();
    drop(e);
    let e = Engine::open(dir.path().join("store.kiva"), quiet(0)).unwrap();
    println!("restart with the saved graph: {:?}", start.elapsed());
    assert_eq!(e.list_vector_indexes()[0].vectors, n);
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
    assert!(ok(&mut s, "indexes")
        .contains("doc:* (embedding) - HNSW vector index (cosine, m 8, ef 50), 3 vectors, 3 dimensions"));
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
    assert_eq!(r.cmd(&["PEXPIRETIME", "c"]), R::I(at * 1000));
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
    // Database 0 is the user's own workspace.
    assert_eq!(p.cmd(&["SELECT", "0"]), ok_reply());
    assert_eq!(p.cmd(&["GET", "where"]), R::B(None));
    p.cmd(&["SET", "mine", "1"]);
    assert_eq!(ok(&mut admin, "workspace switch u_pay"), "Switched to workspace 'u_pay' (1 keys).");
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
    ok(&mut admin, r#"create user "eve" identified by "evepw" role viewer"#);
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
fn redis_refuses_unsupported_commands_clearly() {
    let dir = TempDir::new("resp_refused");
    let inst = instance(&dir);
    let port = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = Redis::connect(port);
    let refused = |reply: R, needle: &str| match reply {
        R::E(e) => assert!(e.starts_with("ERR unknown command") && e.contains(needle), "{e}"),
        other => panic!("{other:?}"),
    };

    refused(r.cmd(&["BLPOP", "jobs", "0"]), "blocking commands");
    refused(r.cmd(&["BZPOPMIN", "lb", "1"]), "blocking commands");
    refused(r.cmd(&["XADD", "events", "*", "type", "login"]), "streams");
    refused(r.cmd(&["XREADGROUP", "GROUP", "g", "c", "STREAMS", "events", ">"]), "streams");
    refused(r.cmd(&["WATCH", "balance"]), "EXEC answers nil");
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
    refused(r.cmd(&["BRPOP", "q", "0"]), "blocking");
    assert!(matches!(r.cmd(&["EXEC"]), R::E(e) if e.starts_with("EXECABORT")));
    assert_eq!(r.cmd(&["EXISTS", "a"]), R::I(0));
    assert_eq!(r.cmd(&["MULTI"]), ok_reply());
    assert_eq!(r.cmd(&["SET", "a", "1"]), R::S("QUEUED".into()));
    assert_eq!(r.cmd(&["EXEC"]), R::A(Some(vec![ok_reply()])));

    // Scripts go through the same filter.
    refused(r.cmd(&["EVAL", "return redis.pcall('XADD', KEYS[1], '*', 'f', 'v')", "1", "s"]), "streams");

    // COMMAND tells clients what is there.
    match (r.cmd(&["COMMAND", "COUNT"]), r.cmd(&["COMMAND", "LIST"])) {
        (R::I(n), R::A(Some(names))) => {
            assert_eq!(n as usize, names.len());
            assert!(names.contains(&b("zadd")) && !names.contains(&b("blpop")));
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
