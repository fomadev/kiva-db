//! Value cache, vector search, Redis compatibility and Server-Sent Events.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::engine::{Engine, EngineOptions, VectorQuery};
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
    VectorQuery { pattern: pattern.into(), field: "embedding".into(), vector: v, metric, limit }
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
    assert_eq!(e.list_vector_indexes(), [("doc:*".to_string(), "embedding".to_string(), 3, Some(3), 1)]);
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
