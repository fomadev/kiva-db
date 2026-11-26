// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Release localhost comparison. Pass a new scratch directory and the
//! client counts to exercise, for example 1 8 32 128.

use kivadb::{client::Client, session::Session, Instance, Options, Value};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const STEPS: usize = 8;
const METRICS: [&str; 6] = ["auth", "selection", "get", "set", "indexed", "transaction"];

fn local(s: &mut Session, command: &str) {
    let out = s.execute(command);
    assert!(!out.has_err(), "{command}: {}", out.text());
}

fn select(managed: bool, store: usize) -> String {
    if managed {
        format!("use collection bench{store}.app.users")
    } else {
        format!("workspace switch bench{store}")
    }
}

fn provision(home: &Path, managed: bool, records: usize) {
    let inst = Instance::open(home, Options::default()).unwrap();
    let mut s = Session::new(inst.clone(), true, None);
    local(&mut s, "config set storage.sync_writes true");
    if std::env::var_os("KIVADB_BENCH_NATIVE_METRICS").is_some() {
        local(&mut s, "config set server.native_metrics true");
    }
    if let Ok(slots) = std::env::var("KIVADB_BENCH_SLOTS") {
        local(&mut s, &format!("config set server.threads {slots}"));
    }
    local(&mut s, "modify user kiva password bench-password");
    local(&mut s, "create user editor identified by bench-password role editor");
    local(&mut s, "config set auth_required true");
    s.login("kiva", "bench-password").unwrap();
    if managed {
        local(&mut s, "hierarchy data");
    }
    for store in 0..4 {
        local(&mut s, &format!("workspace create bench{store}"));
        if managed {
            local(&mut s, &format!("switch bench{store}"));
            local(&mut s, "project create app");
            local(&mut s, "use app");
            local(&mut s, "collection create users");
        }
        local(&mut s, &select(managed, store));
        local(&mut s, "set data 0");
        local(&mut s, r#"set doc:1 {"city":"Paris"}"#);
        local(&mut s, "create index on doc:* (city)");
        if records != 0 {
            local(&mut s, "begin");
            let value = "x".repeat(256);
            for n in 0..records {
                local(&mut s, &format!("set sample:{n} \"{value}\""));
            }
            local(&mut s, "commit");
        }
    }
    if managed {
        local(&mut s, "hierarchy grant editor write instance");
    } else {
        local(&mut s, "grant write on all to editor");
    }
    drop(s);
    inst.shutdown();
}

fn connect(port: u16, managed: bool, fingerprint: Option<&str>) -> Client {
    let mut client = match fingerprint {
        Some(fp) => Client::connect_tls(&format!("127.0.0.1:{port}"), Some(fp)),
        None => Client::connect(("127.0.0.1", port)),
    }
    .unwrap();
    if managed {
        client.negotiate_hierarchy().unwrap();
    }
    client
}

fn timed<T>(values: &mut Vec<Duration>, operation: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = operation();
    values.push(start.elapsed());
    result
}

fn report(label: &str, mut values: Vec<Duration>) {
    if values.is_empty() {
        return;
    }
    values.sort_unstable();
    let n = values.len();
    let percentile = |p: usize| values[(n * p).div_ceil(100) - 1].as_secs_f64() * 1e6;
    println!("{label} {n} {:.2} {:.2} {:.2}", percentile(50), percentile(95), percentile(99));
}

fn resources() {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        if let Some(rss) = status.lines().find(|row| row.starts_with("VmRSS:")) {
            println!("{rss}");
        }
    }
    if let Ok(descriptors) = std::fs::read_dir("/proc/self/fd") {
        println!("descriptors {}", descriptors.count());
    }
}

fn load(port: u16, managed: bool, count: usize, spread: bool) {
    let start_gate = Arc::new(Barrier::new(count + 1));
    let ready_gate = Arc::new(Barrier::new(count + 1));
    let cohort = if spread { "spread" } else { "concentrated" };
    println!("{} {count} {cohort}", if managed { "hierarchy" } else { "legacy" });
    let jobs: Vec<_> = (0..count)
        .map(|id| {
            let ready = ready_gate.clone();
            let gate = start_gate.clone();
            std::thread::spawn(move || {
                let mut client = connect(port, managed, None);
                let mut samples: [Vec<Duration>; 6] = std::array::from_fn(|_| Vec::new());
                timed(&mut samples[0], || client.auth("editor", "bench-password").unwrap());
                let store = if spread { id % 4 } else { 0 };
                let command = select(managed, store);
                timed(&mut samples[1], || client.exec(&command).unwrap());
                ready.wait();
                gate.wait();
                let key = format!("worker:{id}");
                for step in 0..STEPS {
                    timed(&mut samples[2], || client.get("data").unwrap().unwrap());
                    let value = Value::Num(step.to_string());
                    timed(&mut samples[3], || client.set(&key, &value, 0).unwrap());
                    timed(&mut samples[4], || client.exec(r#"get where doc:*.city = "Paris""#).unwrap());
                    timed(&mut samples[5], || {
                        client.exec("begin").unwrap();
                        client.set(&key, &value, 0).unwrap();
                        client.exec("commit").unwrap();
                    });
                }
                samples
            })
        })
        .collect();
    ready_gate.wait();
    resources();
    let start = Instant::now();
    start_gate.wait();
    let mut combined: [Vec<Duration>; 6] = std::array::from_fn(|_| Vec::new());
    for job in jobs {
        for (out, samples) in combined.iter_mut().zip(job.join().unwrap()) {
            out.extend(samples);
        }
    }
    println!("throughput {:.2}", (count * STEPS * 4) as f64 / start.elapsed().as_secs_f64());
    for (metric, values) in METRICS.into_iter().zip(combined) {
        report(metric, values);
    }
}

fn maintenance(port: u16, managed: bool, fingerprint: Option<&str>) {
    let mut admin = connect(port, managed, fingerprint);
    admin.auth("kiva", "bench-password").unwrap();
    admin.exec(&select(managed, 0)).unwrap();
    let mut editor = connect(port, managed, fingerprint);
    editor.auth("editor", "bench-password").unwrap();
    editor.exec(&select(managed, 0)).unwrap();
    let (grant, revoke) = if managed {
        ("hierarchy grant editor write instance", "hierarchy revoke editor write instance")
    } else {
        ("grant write on all to editor", "revoke write on all from editor")
    };
    let mut samples = Vec::new();
    for _ in 0..16 {
        timed(&mut samples, || admin.exec(revoke).unwrap());
        if !managed {
            editor.auth("editor", "bench-password").unwrap();
            editor.exec(&select(managed, 0)).unwrap();
        }
        assert!(editor.set("denied", &Value::Null, 0).is_err());
        admin.exec(grant).unwrap();
        editor.auth("editor", "bench-password").unwrap();
        editor.exec(&select(managed, 0)).unwrap();
    }
    report("revoke", samples);
    for (store, label) in [(0, "same"), (1, "other")] {
        editor.exec(&select(managed, store)).unwrap();
        let gate = Barrier::new(2);
        let (compact, reads) = std::thread::scope(|scope| {
            let reads = scope.spawn(|| {
                let mut values = Vec::new();
                gate.wait();
                for _ in 0..128 {
                    timed(&mut values, || editor.get("data").unwrap().unwrap());
                }
                values
            });
            gate.wait();
            let mut compact = Vec::new();
            timed(&mut compact, || admin.exec("compact").unwrap());
            (compact, reads.join().unwrap())
        });
        report(&format!("compact-{label}"), compact);
        report(&format!("get-during-compact-{label}"), reads);
    }
}

struct Profile {
    name: String,
    records: usize,
    duration: Duration,
    fingerprint: Option<String>,
}

impl Profile {
    // Metric indices match the fixed-loop benchmark.
    fn operation(&self, client: &mut Client, key: &str, step: usize) -> (usize, kivadb::Result<()>) {
        let metric = match self.name.as_str() {
            "read" => 2,
            "write" => 3,
            "95read" => {
                if !step.is_multiple_of(20) {
                    2
                } else {
                    3
                }
            }
            "50read" => {
                if step.is_multiple_of(2) {
                    2
                } else {
                    3
                }
            }
            "index" => 4,
            "txn" => 5,
            "mixed" => step % 4 + 2,
            _ => panic!("unknown profile"),
        };
        let value = || Value::Str("y".repeat(256));
        let result = match metric {
            2 => client.get(&format!("sample:{}", step % self.records)).map(|value| {
                assert!(value.is_some());
            }),
            3 => client.set(key, &value(), 0),
            4 => client.exec(r#"get where doc:*.city = "Paris""#).map(|_| ()),
            5 => (|| {
                client.exec("begin")?;
                client.set(key, &value(), 0)?;
                client.exec("commit")?;
                Ok(())
            })(),
            _ => unreachable!(),
        };
        (metric, result)
    }
}

fn cpu_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}

fn profiled_load(port: u16, managed: bool, count: usize, spread: bool, profile: &Profile) {
    let ready = Barrier::new(count + 1);
    let gate = Barrier::new(count + 1);
    let cohort = if spread { "spread" } else { "concentrated" };
    println!("{} {count} {cohort} {}", if managed { "hierarchy" } else { "legacy" }, profile.name);
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..count)
            .map(|id| {
                let (ready, gate) = (&ready, &gate);
                scope.spawn(move || {
                    let mut client = connect(port, managed, profile.fingerprint.as_deref());
                    let mut samples: [Vec<Duration>; 6] = std::array::from_fn(|_| Vec::new());
                    timed(&mut samples[0], || client.auth("editor", "bench-password").unwrap());
                    let store = if spread { id % 4 } else { 0 };
                    timed(&mut samples[1], || client.exec(&select(managed, store)).unwrap());
                    let key = format!("worker:{id}");
                    let warmup = Instant::now() + Duration::from_millis(150);
                    let mut step = id % 20;
                    while Instant::now() < warmup {
                        profile.operation(&mut client, &key, step).1.unwrap();
                        step += 1;
                    }
                    ready.wait();
                    gate.wait();
                    let end = Instant::now() + profile.duration;
                    let mut errors = 0;
                    while Instant::now() < end {
                        let start = Instant::now();
                        let (metric, result) = profile.operation(&mut client, &key, step);
                        if let Err(error) = result {
                            errors += 1;
                            eprintln!("sample error: {error}");
                            break;
                        }
                        samples[metric].push(start.elapsed());
                        step += 1;
                    }
                    (samples, errors)
                })
            })
            .collect();
        ready.wait();
        resources();
        let cpu_start = cpu_ticks();
        let start = Instant::now();
        gate.wait();
        let mut combined: [Vec<Duration>; 6] = std::array::from_fn(|_| Vec::new());
        let mut errors = 0;
        for job in jobs {
            let (samples, failures) = job.join().unwrap();
            errors += failures;
            for (out, values) in combined.iter_mut().zip(samples) {
                out.extend(values);
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        let operations: usize = combined[2..].iter().map(Vec::len).sum();
        println!("throughput {:.2}", operations as f64 / elapsed);
        println!("completed {operations} errors {errors}");
        if let Some((before, after)) = cpu_start.zip(cpu_ticks()) {
            println!("cpu_ticks {} elapsed {elapsed:.3}", after - before);
        }
        for (metric, values) in METRICS.into_iter().zip(combined) {
            report(metric, values);
        }
    });
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("pass a new scratch directory and client counts");
    std::fs::create_dir(&dir).expect("benchmark root must be new");
    let counts: Vec<usize> = args.map(|s| s.parse().unwrap()).collect();
    assert!(!counts.is_empty() && counts.iter().all(|n| (1..=128).contains(n)));
    let profile = std::env::var("KIVADB_BENCH_PROFILE").ok();
    let records = if profile.is_some() { 512 } else { 0 };
    let duration =
        Duration::from_millis(std::env::var("KIVADB_BENCH_MS").ok().and_then(|n| n.parse().ok()).unwrap_or(800));
    for managed in [false, true] {
        let home = Path::new(&dir).join(if managed { "managed" } else { "legacy" });
        provision(&home, managed, records);
        let open = if managed { Instance::open_hierarchical } else { Instance::open };
        let instance = open(&home, Options::default()).unwrap();
        let port = kivadb::server::start_tcp(&instance, "127.0.0.1", 0).unwrap();
        let fingerprint = std::env::var_os("KIVADB_BENCH_TLS").map(|_| instance.tls().unwrap().fingerprint.clone());
        let profile = profile.as_ref().map(|name| Profile {
            name: name.clone(),
            records,
            duration,
            fingerprint: fingerprint.clone(),
        });
        for &count in &counts {
            for spread in [false, true] {
                if let Some(profile) = &profile {
                    profiled_load(port, managed, count, spread, profile);
                } else {
                    load(port, managed, count, spread);
                }
                // Just-closed workers may still hold a server-side slot.
                let deadline = Instant::now() + Duration::from_secs(5);
                while instance.clients.load(std::sync::atomic::Ordering::Acquire) != 0 {
                    assert!(Instant::now() < deadline);
                    std::thread::yield_now();
                }
            }
        }
        maintenance(port, managed, fingerprint.as_deref());
        for row in instance.native_timings().into_iter().filter(|row| row.samples != 0) {
            println!(
                "timing {} {} {} {:.2}",
                row.operation,
                row.stage,
                row.samples,
                row.nanos as f64 / row.samples as f64 / 1000.0
            );
        }
        instance.shutdown();
        let _ = TcpStream::connect(("127.0.0.1", port));
    }
}
