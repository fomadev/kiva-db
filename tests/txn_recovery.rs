// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Test F, recovery and isolation without crash points: transactions read
//! back from a damaged or cut log apply whole or not at all, and concurrent
//! transfers lose no update (docs/testing/transaction-crash-matrix.md).

mod common;

use common::{instance, ok, TempDir};
use kivadb::session::Session;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn data_file(dir: &TempDir) -> PathBuf {
    dir.path().join("data").join("store.kiva")
}

/// The two accounts, then the transfer, then a stop that leaves no hint:
/// the next start reads the whole log, as after a crash.
fn transferred(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "set account:alpha 100000");
    ok(&mut s, "set account:beta 100000");
    ok(&mut s, "begin");
    ok(&mut s, "update account:alpha 99999");
    ok(&mut s, "update account:beta 100001");
    ok(&mut s, "commit");
    drop(s);
    inst.shutdown();
    drop(inst);
    let _ = std::fs::remove_file(dir.path().join("data").join("store.kiva.hint"));
    dir
}

/// Flips one bit in the key of the last record of `key`.
fn damage_last(file: &Path, key: &str) {
    let mut b = std::fs::read(file).unwrap();
    let at = b.windows(key.len()).rposition(|w| w == key.as_bytes()).unwrap();
    b[at + key.len() - 1] ^= 0x01;
    std::fs::write(file, b).unwrap();
}

fn balances(dir: &TempDir) -> (String, String) {
    let inst = instance(dir);
    let mut s = Session::new(inst.clone(), true, None);
    let r = (ok(&mut s, "get account:alpha"), ok(&mut s, "get account:beta"));
    drop(s);
    inst.shutdown();
    let _ = std::fs::remove_file(dir.path().join("data").join("store.kiva.hint"));
    r
}

fn before() -> (String, String) {
    ("account:alpha: 100000 [int]".into(), "account:beta: 100000 [int]".into())
}

#[test]
fn a_damaged_record_takes_its_whole_transaction_away() {
    // The first record of the transfer: before, the second one was applied
    // alone (alpha 100000, beta 100001).
    let dir = transferred("txn_damaged_first");
    damage_last(&data_file(&dir), "account:alpha");
    assert_eq!(balances(&dir), before());
    // Every start reads the same.
    assert_eq!(balances(&dir), before());
}

#[test]
fn a_damaged_commit_record_followed_by_writes() {
    let dir = transferred("txn_damaged_commit");
    // Another write after the transfer: its commit record is no longer the
    // end of the file, so it is not a torn write.
    {
        let inst = instance(&dir);
        ok(&mut Session::new(inst.clone(), true, None), "set other 1");
        inst.shutdown();
    }
    let _ = std::fs::remove_file(dir.path().join("data").join("store.kiva.hint"));
    damage_last(&data_file(&dir), "account:beta");
    assert_eq!(balances(&dir), before());
    let inst = instance(&dir);
    assert_eq!(ok(&mut Session::new(inst.clone(), true, None), "get other"), "other: 1 [int]");
}

#[test]
fn a_transaction_after_a_broken_one_at_the_end_is_kept() {
    // A damaged record of a batch whose commit never came, then a new
    // transfer: the new one must not be taken for the end of the broken one.
    let dir = transferred("txn_broken_tail");
    let file = data_file(&dir);
    damage_last(&file, "account:alpha");
    // Cut the commit record of the transfer away: a broken batch the file
    // ends in.
    let len = std::fs::metadata(&file).unwrap().len();
    let b = std::fs::read(&file).unwrap();
    let beta = b.windows(12).rposition(|w| w == b"account:beta").unwrap();
    let start_of_beta = beta - 28;
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_len(start_of_beta as u64).unwrap();
    assert!((start_of_beta as u64) < len);
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "begin");
        ok(&mut s, "update account:alpha 99990");
        ok(&mut s, "update account:beta 100010");
        ok(&mut s, "commit");
        drop(s);
        inst.shutdown();
    }
    let _ = std::fs::remove_file(dir.path().join("data").join("store.kiva.hint"));
    assert_eq!(balances(&dir), ("account:alpha: 99990 [int]".to_string(), "account:beta: 100010 [int]".to_string()));
}

#[test]
fn a_log_cut_in_the_middle_of_a_transaction() {
    let dir = transferred("txn_cut");
    let file = data_file(&dir);
    let len = std::fs::metadata(&file).unwrap().len();
    // Cut inside the last record (the commit of the transfer).
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_len(len - 3).unwrap();
    assert_eq!(balances(&dir), before());
    assert_eq!(balances(&dir), before());
}

#[test]
fn concurrent_transfers_lose_no_update() {
    let dir = TempDir::new("txn_concurrent");
    let inst = instance(&dir);
    {
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "set account:alpha 100000");
        ok(&mut s, "set account:beta 100000");
    }
    // Four clients, 100 transfers of 1 each from alpha to beta, every one
    // reading the balances inside its transaction: a conflict is retried.
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let inst = Arc::clone(&inst);
            std::thread::spawn(move || {
                let mut s = Session::new(inst, true, None);
                let mut conflicts = 0;
                for _ in 0..100 {
                    loop {
                        ok(&mut s, "begin");
                        let a: i64 = num(&ok(&mut s, "get account:alpha"));
                        let b: i64 = num(&ok(&mut s, "get account:beta"));
                        ok(&mut s, &format!("update account:alpha {}", a - 1));
                        ok(&mut s, &format!("update account:beta {}", b + 1));
                        match s.execute("commit").text() {
                            t if t.contains("Transaction committed") => break,
                            t => {
                                assert!(t.contains("modified by another session"), "{t}");
                                conflicts += 1;
                            }
                        }
                    }
                }
                conflicts
            })
        })
        .collect();
    let conflicts: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
    let mut s = Session::new(inst.clone(), true, None);
    assert_eq!(ok(&mut s, "get account:alpha"), "account:alpha: 99600 [int]", "{conflicts} conflicts");
    assert_eq!(ok(&mut s, "get account:beta"), "account:beta: 100400 [int]");
    // A rolled-back transaction leaves nothing behind.
    ok(&mut s, "begin");
    ok(&mut s, "update account:alpha 1");
    s.execute("rollback");
    assert_eq!(ok(&mut s, "get account:alpha"), "account:alpha: 99600 [int]");
}

fn num(line: &str) -> i64 {
    line.split(": ").nth(1).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok()).unwrap()
}
