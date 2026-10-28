// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! KivaDB against KivaDB: the same work given to real servers of two
//! versions, through the binary protocol, from outside.
//!
//! ```text
//! KIVADB_BENCH_BINS="1.7.1=../kv171/target/release/kivadb.exe;2.0=target/release/kivadb.exe" \
//!     cargo bench --bench versions
//! ```
//!
//! `KIVADB_BENCH_N` (20000) sets the size of the data, `KIVADB_BENCH_ROUNDS`
//! (3) the rounds; in each round every version runs every case once, in a
//! fresh home, the versions taking turns. A Markdown table goes to standard
//! output and every measurement to `KIVADB_BENCH_OUT` (`bench-versions.json`).
//! The servers run with their default settings, except the case with
//! `sync_writes`. Every answer is checked, so that both did the same work.

use kivadb::client::Client;
use kivadb::value::Value;
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    bin: PathBuf,
    home: PathBuf,
    port: u16,
    child: Option<Child>,
}

impl Server {
    fn new(bin: &Path, tag: &str, sync_writes: bool) -> Server {
        let home = std::env::temp_dir().join(format!("kivadb_bench_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        if sync_writes {
            std::fs::write(home.join("kiva.conf"), "[storage]\nsync_writes = true\n").unwrap();
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut s = Server { bin: bin.to_path_buf(), home, port, child: None };
        s.start();
        s
    }

    /// Starts the server; returns how long it took to answer a ping.
    fn start(&mut self) -> Duration {
        let started = Instant::now();
        let child = Command::new(&self.bin)
            .args(["serve", "--home"])
            .arg(&self.home)
            .args(["--port", &self.port.to_string(), "--no-http", "--no-resp"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.child = Some(child);
        loop {
            if let Ok(mut c) = Client::connect(("127.0.0.1", self.port)) {
                if c.ping().is_ok() {
                    return started.elapsed();
                }
            }
            assert!(started.elapsed() < Duration::from_secs(120), "{} did not start", self.bin.display());
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn client(&self) -> Client {
        Client::connect(("127.0.0.1", self.port)).unwrap()
    }

    /// Stops it cleanly, as `shutdown` does.
    fn shutdown(&mut self) {
        let _ = self.client().exec("shutdown");
        if let Some(mut c) = self.child.take() {
            let _ = c.wait();
        }
    }

    /// Kills it, as a crash would.
    fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// Resident memory (peak and now, bytes) and CPU time of the server.
    fn usage(&self) -> (u64, u64, f64) {
        let pid = self.pid();
        if cfg!(windows) {
            let script = format!(
                "$p = Get-Process -Id {pid}; \"$($p.PeakWorkingSet64) $($p.WorkingSet64) $($p.TotalProcessorTime.TotalSeconds)\""
            );
            let out = Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            let v: Vec<&str> = text.split_whitespace().collect();
            (v[0].parse().unwrap_or(0), v[1].parse().unwrap_or(0), v[2].replace(',', ".").parse().unwrap_or(0.0))
        } else {
            let out = Command::new("ps").args(["-o", "rss=,cputime=", "-p", &pid.to_string()]).output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            let v: Vec<&str> = text.split_whitespace().collect();
            let rss = v[0].parse::<u64>().unwrap_or(0) * 1024;
            let secs = v[1].split(':').fold(0.0, |acc, p| acc * 60.0 + p.parse::<f64>().unwrap_or(0.0));
            (rss, rss, secs)
        }
    }

    fn data_bytes(&self) -> u64 {
        fn walk(p: &Path) -> u64 {
            std::fs::read_dir(p)
                .map(|d| {
                    d.flatten()
                        .map(|e| if e.path().is_dir() { walk(&e.path()) } else { e.metadata().map_or(0, |m| m.len()) })
                        .sum()
                })
                .unwrap_or(0)
        }
        walk(&self.home.join("data"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Latencies of one case, in microseconds.
#[derive(Default)]
struct Sample {
    us: Vec<f64>,
    /// Operations, and the wall time they took (for throughput).
    ops: u64,
    wall: Duration,
}

impl Sample {
    fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let r = f();
        self.us.push(t.elapsed().as_secs_f64() * 1e6);
        r
    }

    fn pct(&self, p: f64) -> f64 {
        let mut v = self.us.clone();
        v.sort_by(f64::total_cmp);
        if v.is_empty() {
            return 0.0;
        }
        v[((v.len() - 1) as f64 * p).round() as usize]
    }
}

fn s(len: usize, i: usize) -> Value {
    Value::Str(format!("{i:08}{}", "x".repeat(len - 8)))
}

fn doc(i: usize) -> Value {
    Value::Doc(json!({
        "name": format!("user {i}"), "city": format!("city{}", i % 100), "age": 18 + i % 60,
        "score": (i * 7919) % 1000, "tags": ["a", "b"], "bio": "x".repeat(120),
    }))
}

fn embedding(i: usize) -> Vec<f32> {
    let mut x = (i as u64).wrapping_mul(0x9e3779b97f4a7c15) | 1;
    (0..64)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 2000) as f32 / 1000.0 - 1.0
        })
        .collect()
}

type Results = BTreeMap<String, Sample>;
/// Measures taken once per run: memory, disk, times of compaction and start.
type Facts = BTreeMap<String, f64>;

/// Every case on one server of one version.
fn run(bin: &Path, label: &str, n: usize, round: usize) -> (Results, Facts) {
    let mut r: Results = BTreeMap::new();
    let mut facts: Facts = BTreeMap::new();
    let tag = format!("{label}_{round}").replace(['.', ' '], "_");
    let mut srv = Server::new(bin, &tag, false);
    let mut c = srv.client();

    let wall = |r: &mut Results, name: &str, ops: u64, t: Instant| {
        let e = r.get_mut(name).unwrap();
        e.ops = ops;
        e.wall = t.elapsed();
    };

    // Strings of 100 bytes.
    let t = Instant::now();
    let e = r.entry("write a string".into()).or_default();
    for i in 0..n {
        e.time(|| c.set(&format!("s:{i:07}"), &s(100, i), 0).unwrap());
    }
    wall(&mut r, "write a string", n as u64, t);
    let t = Instant::now();
    let e = r.entry("read a string".into()).or_default();
    for k in 0..n {
        let i = (k * 7919) % n;
        let v = e.time(|| c.get(&format!("s:{i:07}")).unwrap());
        assert!(v.is_some());
    }
    wall(&mut r, "read a string", n as u64, t);

    // Documents with two indexes, written after the indexes.
    c.exec("create index on doc: (city)").unwrap();
    c.exec("create index on doc: (age)").unwrap();
    let t = Instant::now();
    let e = r.entry("write an indexed document".into()).or_default();
    for i in 0..n {
        e.time(|| c.set(&format!("doc:{i:07}"), &doc(i), 0).unwrap());
    }
    wall(&mut r, "write an indexed document", n as u64, t);
    let t = Instant::now();
    let e = r.entry("query an index (100 rows)".into()).or_default();
    for k in 0..500 {
        let q = json!({"op": "query", "target": "doc:*", "where": {"city": format!("city{}", k % 100)}, "limit": 100});
        let a = e.time(|| c.api(&q).unwrap());
        assert_eq!(a["rows"].as_array().unwrap().len(), 100.min(n / 100));
    }
    wall(&mut r, "query an index (100 rows)", 500, t);
    let t = Instant::now();
    let e = r.entry("query without an index".into()).or_default();
    let want = (0..n).filter(|i| (i * 7919) % 1000 >= 990).count().min(10_000);
    for _ in 0..20 {
        let q = json!({"op": "query", "target": "doc:*", "where": {"field": "score", "cmp": ">=", "value": 990}, "limit": 10_000});
        let a = e.time(|| c.api(&q).unwrap());
        assert_eq!(a["rows"].as_array().unwrap().len(), want);
    }
    wall(&mut r, "query without an index", 20, t);
    let t = Instant::now();
    let e = r.entry("count with group by".into()).or_default();
    for _ in 0..20 {
        let out = e.time(|| c.exec("count where doc:*.age >= 30 group by city").unwrap());
        assert!(out.starts_with("Count by city (100 groups"), "{out}");
    }
    wall(&mut r, "count with group by", 20, t);

    // Transactions of 5 writes: begin, 5 sets, commit.
    let t = Instant::now();
    let e = r.entry("transaction of 5 writes".into()).or_default();
    let txns = (n / 10).max(100);
    for k in 0..txns {
        e.time(|| {
            c.exec("begin").unwrap();
            for j in 0..5 {
                c.exec(&format!("set acct:{k}:{j} {}", k + j)).unwrap();
            }
            c.exec("commit").unwrap();
        });
    }
    wall(&mut r, "transaction of 5 writes", txns as u64, t);

    // 8 clients at once: 80% reads, 20% writes.
    let clients = 8;
    let per = n / 2 / clients;
    let t = Instant::now();
    let parts: Vec<Vec<f64>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..clients)
            .map(|w| {
                let srv = &srv;
                sc.spawn(move || {
                    let mut c = srv.client();
                    let mut us = Vec::with_capacity(per);
                    for k in 0..per {
                        let i = (w * per + k) * 31 % n;
                        let t = Instant::now();
                        if k % 5 == 0 {
                            c.set(&format!("s:{i:07}"), &s(100, i + 1), 0).unwrap();
                        } else {
                            assert!(c.get(&format!("s:{i:07}")).unwrap().is_some());
                        }
                        us.push(t.elapsed().as_secs_f64() * 1e6);
                    }
                    us
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let name = "8 clients, 80% reads";
    r.insert(name.into(), Sample { us: parts.concat(), ops: (per * clients) as u64, wall: t.elapsed() });

    // Vector search: 64 dimensions, HNSW.
    let vectors = (n / 4).max(1000);
    for i in 0..vectors {
        c.set(&format!("vec:{i:06}"), &Value::Doc(json!({ "embedding": embedding(i) })), 0).unwrap();
    }
    c.exec("create vector index on vec: (embedding) using hnsw").unwrap();
    let t = Instant::now();
    let e = r.entry("vector search, k = 10".into()).or_default();
    for k in 0..300 {
        let v: Vec<String> = embedding(k * 13 + 7).iter().map(|x| format!("{x:.3}")).collect();
        let out = e.time(|| c.exec(&format!("get similar vec:*.embedding to [{}] limit 10", v.join(", "))).unwrap());
        assert!(out.contains("(10 results"), "{out}");
    }
    wall(&mut r, "vector search, k = 10", 300, t);

    let (peak, now, cpu) = srv.usage();
    facts.insert("server memory, peak (MB)".into(), peak as f64 / 1e6);
    facts.insert("server memory, at the end (MB)".into(), now as f64 / 1e6);
    facts.insert("server CPU time (s)".into(), cpu);
    facts.insert("data on disk (MB)".into(), srv.data_bytes() as f64 / 1e6);

    // Every string written again: half the file is dead, then compacted.
    for i in 0..n {
        c.set(&format!("s:{i:07}"), &s(100, i + 2), 0).unwrap();
    }
    let before = srv.data_bytes();
    let t = Instant::now();
    c.exec("compact").unwrap();
    facts.insert("compaction (ms)".into(), t.elapsed().as_secs_f64() * 1000.0);
    facts.insert("data on disk before compaction (MB)".into(), before as f64 / 1e6);
    facts.insert("data on disk after compaction (MB)".into(), srv.data_bytes() as f64 / 1e6);
    drop(c);

    // Starting again: after a clean stop (index snapshot), after a crash.
    srv.shutdown();
    facts.insert("start after a clean stop (ms)".into(), srv.start().as_secs_f64() * 1000.0);
    srv.client().set("last", &s(100, 0), 0).unwrap();
    srv.kill();
    facts.insert("start after a crash (ms)".into(), srv.start().as_secs_f64() * 1000.0);
    assert!(srv.client().get("last").unwrap().is_some(), "the last write survived the crash");
    drop(srv);

    // Writes made durable one by one (fsync after each).
    let srv = Server::new(bin, &format!("{tag}_sync"), true);
    let mut c = srv.client();
    let t = Instant::now();
    let e = r.entry("write with sync_writes (fsync)".into()).or_default();
    let durable = (n / 10).max(200);
    for i in 0..durable {
        e.time(|| c.set(&format!("d:{i:07}"), &s(100, i), 0).unwrap());
    }
    wall(&mut r, "write with sync_writes (fsync)", durable as u64, t);
    (r, facts)
}

fn main() {
    let bins: Vec<(String, PathBuf)> = std::env::var("KIVADB_BENCH_BINS")
        .unwrap_or_default()
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (l, p) = s.split_once('=').expect("label=path");
            (l.to_string(), PathBuf::from(p))
        })
        .collect();
    if bins.is_empty() {
        eprintln!("Set KIVADB_BENCH_BINS=\"label=path;label=path\" (see benches/versions.rs).");
        return;
    }
    let n: usize = std::env::var("KIVADB_BENCH_N").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let rounds: usize = std::env::var("KIVADB_BENCH_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let mut all: BTreeMap<String, Vec<(Results, Facts)>> = BTreeMap::new();
    for round in 0..rounds {
        // The versions take turns, the first one alternating.
        let mut order: Vec<&(String, PathBuf)> = bins.iter().collect();
        if round % 2 == 1 {
            order.reverse();
        }
        for (label, bin) in order {
            eprintln!("round {} of {rounds}: {label}", round + 1);
            all.entry(label.clone()).or_default().push(run(bin, label, n, round));
        }
    }

    // Median over the rounds of each statistic.
    // With an even number of rounds, the mean of the two middle ones.
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        let m = v.len() / 2;
        if v.len().is_multiple_of(2) {
            (v[m - 1] + v[m]) / 2.0
        } else {
            v[m]
        }
    };
    let labels: Vec<&String> = all.keys().collect();
    let cases: Vec<String> = all[labels[0]][0].0.keys().cloned().collect();
    let mut out = json!({ "n": n, "rounds": rounds, "versions": {} });
    println!("| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |");
    println!("| :--- | :--- | ---: | ---: | ---: | ---: |");
    for case in &cases {
        for l in &labels {
            let runs = &all[*l];
            let stat = |f: &dyn Fn(&Sample) -> f64| median(runs.iter().map(|(r, _)| f(&r[case])).collect());
            let (p50, p95, p99) = (stat(&|s| s.pct(0.5)), stat(&|s| s.pct(0.95)), stat(&|s| s.pct(0.99)));
            let rate = stat(&|s| s.ops as f64 / s.wall.as_secs_f64());
            println!("| {case} | {l} | {p50:.0} | {p95:.0} | {p99:.0} | {rate:.0} |");
            out["versions"][l.as_str()]["cases"][case] = json!({
                "p50_us": p50, "p95_us": p95, "p99_us": p99, "ops_per_s": rate,
                "rounds": runs.iter().map(|(r, _)| json!({
                    "p50_us": r[case].pct(0.5), "p95_us": r[case].pct(0.95), "p99_us": r[case].pct(0.99),
                    "ops_per_s": r[case].ops as f64 / r[case].wall.as_secs_f64(),
                })).collect::<Vec<Json>>(),
            });
        }
    }
    println!();
    println!("| Measure | {} |", labels.iter().map(|l| l.as_str()).collect::<Vec<_>>().join(" | "));
    println!("| :--- |{}", " ---: |".repeat(labels.len()));
    let facts: Vec<String> = all[labels[0]][0].1.keys().cloned().collect();
    for f in &facts {
        let vals: Vec<String> = labels
            .iter()
            .map(|l| {
                let m = median(all[*l].iter().map(|(_, x)| x[f]).collect());
                out["versions"][l.as_str()]["facts"][f] =
                    json!({ "median": m, "rounds": all[*l].iter().map(|(_, x)| x[f]).collect::<Vec<f64>>() });
                format!("{m:.1}")
            })
            .collect();
        println!("| {f} | {} |", vals.join(" | "));
    }
    let path = std::env::var("KIVADB_BENCH_OUT").unwrap_or_else(|_| "bench-versions.json".into());
    std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    eprintln!("every measurement: {path}");
}
