// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Test F: a transfer between two accounts, the process stopped at each
//! crash point of the commit and of the recovery, then started again on the
//! same files (docs/testing/transaction-crash-matrix.md).
//!
//!     cargo test --features crash-points --test crash
//!
//! Each case runs `kivadb` in a child process, in a folder of its own, with
//! one crash point armed. The case passes only if the child stopped at that
//! very point (its marker on stderr, an abnormal exit), and if a start
//! without crash point then finds one of the states the protocol allows,
//! every checksum sound, the same at every restart.
#![cfg(feature = "crash-points")]

mod common;

use common::TempDir;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BEFORE: (i64, i64) = (100_000, 100_000);
const AFTER: (i64, i64) = (99_999, 100_001);

fn kivadb(home: &Path, crash: Option<(&str, u64)>, cmds: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kivadb"));
    c.arg("--home").arg(home).arg("--no-listen");
    for cmd in cmds {
        c.arg("-e").arg(cmd);
    }
    c.env("KIVADB_NO_UPDATE_CHECK", "1")
        .env_remove("KIVADB_ENCRYPTION_KEY")
        .env_remove("KIVADB_CRASH_ARMED")
        .env_remove("KIVADB_CRASH_POINT")
        .env_remove("KIVADB_CRASH_HIT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some((point, hit)) = crash {
        c.env("KIVADB_CRASH_ARMED", "1").env("KIVADB_CRASH_POINT", point).env("KIVADB_CRASH_HIT", hit.to_string());
    }
    // A child that hangs fails the case instead of the whole run.
    let mut child = c.spawn().unwrap();
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("kivadb {cmds:?} did not end within 60 s (crash point {crash:?})");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Runs `cmds` with `point` armed and checks that the process stopped
/// there, and nowhere else.
fn crash_at(home: &Path, point: &str, hit: u64, cmds: &[&str]) -> Output {
    let o = kivadb(home, Some((point, hit)), cmds);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "{point}: the process was not stopped:\n{}", text(&o));
    let marks: Vec<&str> = stderr.lines().filter(|l| l.starts_with("KIVADB-CRASH-POINT ")).collect();
    assert_eq!(marks, [format!("KIVADB-CRASH-POINT {point}")], "{point}: stopped elsewhere:\n{}", text(&o));
    o
}

fn value(out: &str, key: &str) -> i64 {
    let line = out.lines().find(|l| l.starts_with(&format!("{key}: "))).unwrap_or_else(|| panic!("no {key}:\n{out}"));
    line[key.len() + 2..].split_whitespace().next().unwrap().parse().unwrap_or_else(|_| panic!("{line}"))
}

/// The two balances after a start without crash point, with the checks of
/// every record; twice, so that recovery is seen to change nothing more.
fn recovered(home: &Path) -> (i64, i64) {
    let mut seen = Vec::new();
    for _ in 0..2 {
        let o = kivadb(home, None, &["get account:alpha", "get account:beta", "check"]);
        let out = text(&o);
        assert!(o.status.success(), "{out}");
        assert!(out.contains("CORRUPTED: 0"), "a record does not match its checksum:\n{out}");
        seen.push((value(&out, "account:alpha"), value(&out, "account:beta")));
    }
    assert_eq!(seen[0], seen[1], "a second start found something else");
    let (a, b) = seen[0];
    assert_eq!(a + b, 200_000, "the total changed: {seen:?}");
    seen[0]
}

/// A home with both accounts at 100 000, `extra` set first.
fn accounts(tag: &str, extra: &[&str]) -> TempDir {
    let dir = TempDir::new(tag);
    let mut cmds: Vec<&str> = extra.to_vec();
    cmds.extend(["set account:alpha 100000", "set account:beta 100000"]);
    let o = kivadb(dir.path(), None, &cmds);
    assert!(o.status.success(), "{}", text(&o));
    dir
}

const TRANSFER: [&str; 4] = ["begin", "update account:alpha 99999", "update account:beta 100001", "commit"];

/// One case of the matrix: the transfer, stopped at `point`, must come back
/// as `expected`.
fn case(point: &str, extra: &[&str], expected: (i64, i64)) {
    let dir = accounts(&format!("crash_{}", point.replace('.', "_")), extra);
    let o = crash_at(dir.path(), point, 1, &TRANSFER);
    assert!(!text(&o).contains("Transaction committed"), "{point}: the client was answered:\n{}", text(&o));
    assert_eq!(recovered(dir.path()), expected, "{point}");
}

#[test]
fn before_the_records_are_written_nothing_happened() {
    case("batch.before_append", &[], BEFORE);
}

#[test]
fn a_write_cut_short_is_dropped_whole() {
    let dir = accounts("crash_torn", &[]);
    let data = dir.path().join("data").join("store.kiva");
    let len = std::fs::metadata(&data).unwrap().len();
    crash_at(dir.path(), "batch.torn_append", 1, &TRANSFER);
    // The first record of the batch is on disk, and half of the second.
    assert!(std::fs::metadata(&data).unwrap().len() > len, "nothing was written before the crash");
    let o = kivadb(dir.path(), None, &["get account:alpha"]);
    assert!(text(&o).contains("incomplete log discarded"), "{}", text(&o));
    assert_eq!(recovered(dir.path()), BEFORE);
}

#[test]
fn once_written_the_batch_survives_a_crash_of_the_process() {
    case("batch.after_append", &[], AFTER);
}

#[test]
fn halfway_through_the_index_the_batch_comes_back_whole() {
    case("batch.mid_index", &[], AFTER);
}

#[test]
fn with_sync_writes_a_crash_before_the_fsync() {
    // The bytes reached the operating system: a crash of the process keeps
    // them. A power cut here could lose them; that is not tested.
    case("batch.before_sync", &["config set sync_writes true"], AFTER);
}

#[test]
fn with_sync_writes_a_crash_after_the_fsync() {
    case("batch.after_sync", &["config set sync_writes true"], AFTER);
}

#[test]
fn committed_but_not_answered() {
    // The client never learns it, yet the transfer is made: a client that
    // gets no answer must read the balances again before trying once more.
    case("commit.before_reply", &["config set sync_writes true"], AFTER);
}

#[test]
fn with_the_change_journal_on_the_data_and_the_journal_agree() {
    let dir = accounts("crash_changes", &["changes enable"]);
    crash_at(dir.path(), "batch.after_append", 1, &TRANSFER);
    assert_eq!(recovered(dir.path()), AFTER);
    // The journal was rebuilt from the data: it holds the transfer whole.
    let o = kivadb(dir.path(), None, &["changes read from beginning limit 100"]);
    let out = text(&o);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("99999") && out.contains("100001"), "the journal misses the transfer:\n{out}");
}

#[test]
fn a_crash_aims_at_one_transaction_among_several() {
    let dir = accounts("crash_second", &[]);
    let two = [
        "begin",
        "update account:alpha 99999",
        "update account:beta 100001",
        "commit",
        "begin",
        "update account:alpha 99998",
        "update account:beta 100002",
        "commit",
    ];
    // The first commit is made, the second stopped before its write.
    crash_at(dir.path(), "batch.before_append", 2, &two);
    assert_eq!(recovered(dir.path()), AFTER);
    // Then after the write of the fifth of five.
    let dir = accounts("crash_fifth", &[]);
    let mut many = Vec::new();
    let steps: Vec<(String, String)> = (1..=5)
        .map(|i| (format!("update account:alpha {}", 100_000 - i), format!("update account:beta {}", 100_000 + i)))
        .collect();
    for (a, b) in &steps {
        many.extend(["begin", a.as_str(), b.as_str(), "commit"]);
    }
    crash_at(dir.path(), "batch.after_append", 5, &many);
    assert_eq!(recovered(dir.path()), (99_995, 100_005));
}

#[test]
fn a_crash_during_recovery_leaves_it_to_the_next_start() {
    let dir = accounts("crash_recovery", &[]);
    crash_at(dir.path(), "batch.torn_append", 1, &TRANSFER);
    // Stopped twice while recovering: before cutting the torn bytes, then
    // just after.
    crash_at(dir.path(), "open.before_truncate", 1, &["get account:alpha"]);
    crash_at(dir.path(), "open.after_truncate", 1, &["get account:alpha"]);
    assert_eq!(recovered(dir.path()), BEFORE);
    // And the accounts take a new transfer as usual.
    let o = kivadb(dir.path(), None, &TRANSFER);
    assert!(text(&o).contains("Transaction committed"), "{}", text(&o));
    assert_eq!(recovered(dir.path()), AFTER);
}

#[test]
fn no_crash_point_fires_unless_armed() {
    // The point named, but not armed: the transfer goes through.
    let dir = accounts("crash_unarmed", &[]);
    let mut c = Command::new(env!("CARGO_BIN_EXE_kivadb"));
    c.arg("--home").arg(dir.path()).arg("--no-listen");
    for cmd in TRANSFER {
        c.arg("-e").arg(cmd);
    }
    let o = c
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .env_remove("KIVADB_CRASH_ARMED")
        .env("KIVADB_CRASH_POINT", "batch.before_append")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success() && text(&o).contains("Transaction committed"), "{}", text(&o));
    assert_eq!(recovered(dir.path()), AFTER);
}
