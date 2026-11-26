// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Local Release benchmark. Pass an empty scratch directory followed by collection
//! counts. Use --baseline as the last argument when linking against Phase 6.

use kivadb::{session::Session, Instance, Options};
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn run(session: &mut Session, command: &str) {
    let result = session.execute(command);
    assert!(!result.has_err(), "{command}: {:?}", result.lines);
}

fn report(label: &str, mut values: Vec<Duration>, elapsed: Duration) {
    values.sort_unstable();
    let n = values.len();
    let percentile = |rank: usize| values[(n * rank).div_ceil(100usize) - 1].as_secs_f64() * 1e6;
    println!(
        "{label} {n} {:.2} {:.2} {:.2} {:.2}",
        percentile(50usize),
        percentile(95),
        percentile(99),
        n as f64 / elapsed.as_secs_f64(),
    );
}

fn measure(label: &str, samples: usize, mut operation: impl FnMut(usize)) {
    let before = Instant::now();
    let values = (0..samples)
        .map(|i| {
            let start = Instant::now();
            operation(i);
            start.elapsed()
        })
        .collect();
    report(label, values, before.elapsed());
}

fn target(index: usize) -> String {
    format!("w{}.p{}.c{index}", index % 4, (index / 4) % 2)
}

fn select(session: &mut Session, index: usize) {
    run(session, &format!("use collection {}", target(index)));
}

fn editor(instance: Arc<Instance>, id: usize) -> Session {
    let mut session = Session::new(instance, true, None);
    session.login(&format!("editor{id}"), "bench-test").unwrap();
    run(&mut session, "hierarchy data");
    session
}

fn provision(home: &Path, count: usize) {
    let instance = Instance::open(home, Options::default()).unwrap();
    let mut admin = Session::new(instance.clone(), true, None);
    for i in 0..4 {
        run(&mut admin, &format!("create user editor{i} identified by bench-test role editor"));
    }
    run(&mut admin, "hierarchy data");
    run(&mut admin, "workspace create w0 w1 w2 w3");
    for w in 0..4 {
        run(&mut admin, &format!("switch w{w}"));
        run(&mut admin, "project create p0 p1");
        for p in 0..2 {
            let names = (w + p * 4..count).step_by(8).map(|i| format!("c{i}")).collect::<Vec<_>>().join(" ");
            if !names.is_empty() {
                run(&mut admin, &format!("use p{p}"));
                run(&mut admin, &format!("collection create {names}"));
            }
        }
    }
    for i in 0..4 {
        run(&mut admin, &format!("hierarchy grant editor{i} write instance"));
    }
    for i in 0..count.min(128) {
        run(&mut admin, &format!("hierarchy grant editor0 read collection {}", target(i)));
    }
    for collection in 0..count.min(4) {
        select(&mut admin, collection);
        for command in [
            "set key 0",
            r#"set doc:1 {"city":"Paris","embedding":[1,0]}"#,
            "create index on doc:* (city)",
            "create vector index on doc:* (embedding)",
        ] {
            run(&mut admin, command);
        }
        for worker in 0..4 {
            run(&mut admin, &format!("set worker{worker} 0"));
        }
    }
    drop(admin);
    instance.shutdown();
}

fn concurrent(instance: &Arc<Instance>, count: usize, distributed: bool) -> (Vec<Duration>, Duration) {
    let barrier = Arc::new(Barrier::new(5));
    let jobs = (0..4)
        .map(|worker| {
            let mut s = editor(instance.clone(), worker);
            select(&mut s, if distributed { worker % count } else { 0 });
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (0..64)
                    .map(|i| {
                        let start = Instant::now();
                        if worker < 2 {
                            run(&mut s, &format!("update worker{worker} {i}"));
                        } else {
                            run(&mut s, "get key");
                        }
                        start.elapsed()
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let start = Instant::now();
    barrier.wait();
    let values = jobs.into_iter().flat_map(|job| job.join().unwrap()).collect();
    (values, start.elapsed())
}

fn benchmark(home: &Path, count: usize, baseline: bool) {
    provision(home, count);
    println!("{count} collections, {} stored grants", count.min(128) + 4);
    measure("startup", 12, |_| {
        let instance = Instance::open(home, Options::default()).unwrap();
        instance.shutdown();
    });
    let instance = Instance::open(home, Options::default()).unwrap();
    let mut s = editor(instance.clone(), 0);
    measure("navigation", 128, |i| select(&mut s, i % count));
    measure("open-or-hit", count.min(128), |i| {
        select(&mut s, i);
        run(&mut s, "get key");
    });
    select(&mut s, 0);
    for (label, command) in [
        ("get", "get key"),
        ("index", r#"get where doc:*.city = "Paris""#),
        ("vector", "get similar doc:*.embedding to [1,0] limit 1"),
        ("permissions", "hierarchy permissions"),
    ] {
        measure(label, 256, |_| run(&mut s, command));
    }
    measure("set", 64, |i| run(&mut s, &format!("set sample{i} {i}")));
    measure("transaction", 64, |i| {
        run(&mut s, "begin");
        run(&mut s, &format!("update key {i}"));
        run(&mut s, "commit");
    });
    for (label, distributed) in [("concentrated", false), ("distributed", true)] {
        let (samples, elapsed) = concurrent(&instance, count, distributed);
        report(label, samples, elapsed);
    }
    let mut admin = Session::new(instance.clone(), true, None);
    run(&mut admin, "hierarchy data");
    measure("grant-revoke-refresh", 16, |_| {
        run(&mut admin, "hierarchy grant editor3 read instance");
        run(&mut admin, "hierarchy revoke editor3 read instance");
        run(&mut s, "get key");
    });
    if !baseline {
        select(&mut admin, 0);
        measure("compaction", 8, |_| run(&mut admin, "compact"));
        println!("{}", admin.execute("hierarchy engines").text());
    }
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        if let Some(rss) = status.lines().find(|line| line.starts_with("VmRSS:")) {
            println!("{rss}");
        }
    }
    if let Ok(files) = std::fs::read_dir("/proc/self/fd") {
        println!("open descriptors: {}", files.count());
    }
    drop(s);
    drop(admin);
    instance.shutdown();
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args.next().expect("pass a scratch directory followed by collection counts");
    let mut sizes: Vec<_> = args.collect();
    let baseline = sizes.last().is_some_and(|arg| arg == "--baseline");
    if baseline {
        sizes.pop();
    }
    for size in sizes {
        let count: usize = size.parse().expect("positive collection count");
        assert!(count > 0);
        let home = Path::new(&root).join(size);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(&home).expect("benchmark homes must be new");
        benchmark(&home, count, baseline);
    }
}
