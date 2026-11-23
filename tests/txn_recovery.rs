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

/// The start refused, with the error it gave, and the data file left
/// exactly as it was, with no copy of a cut beside it.
fn refused(dir: &TempDir) -> String {
    let file = data_file(dir);
    let bytes = std::fs::read(&file).unwrap();
    let err = match kivadb::instance::Instance::open(dir.path(), Default::default()) {
        Ok(_) => panic!("a damaged data file was opened"),
        Err(e) => e.to_string(),
    };
    assert!(err.starts_with("ERR_DATA_CORRUPTED: "), "{err}");
    assert!(err.contains("KivaDB did not open it and changed nothing in it"), "{err}");
    assert_eq!(std::fs::read(&file).unwrap(), bytes, "a refused file must not change");
    assert!(copies(dir, "cut").is_empty(), "nothing is cut from a refused file");
    err
}

/// The copies `store.kiva.<kind>-<ms>` kept next to the data file, oldest first.
fn copies(dir: &TempDir, kind: &str) -> Vec<PathBuf> {
    let prefix = format!("store.kiva.{kind}-");
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir.path().join("data"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(&prefix))
        .collect();
    found.sort();
    found
}

/// Byte where the last record of `key` starts.
fn record_of(file: &Path, key: &str) -> usize {
    let b = std::fs::read(file).unwrap();
    b.windows(key.len()).rposition(|w| w == key.as_bytes()).unwrap() - 28
}

#[test]
fn a_damaged_record_of_a_transaction_stops_the_start() {
    // R1. The first record of the transfer. Before test F, the second one
    // was applied alone (alpha 100000, beta 100001); then the whole
    // transfer was left out, with a warning. Now the start refuses.
    let dir = transferred("txn_damaged_first");
    let at = record_of(&data_file(&dir), "account:alpha");
    damage_last(&data_file(&dir), "account:alpha");
    let err = refused(&dir);
    assert!(err.contains(&format!("damaged at byte {at}: a whole record whose checksum does not match")), "{err}");
    // Every start says the same.
    assert_eq!(refused(&dir), err);
}

#[test]
fn a_damaged_commit_record_followed_by_writes_stops_the_start() {
    // R2.
    let dir = transferred("txn_damaged_commit");
    {
        let inst = instance(&dir);
        ok(&mut Session::new(inst.clone(), true, None), "set other 1");
        inst.shutdown();
    }
    let _ = std::fs::remove_file(dir.path().join("data").join("store.kiva.hint"));
    let at = record_of(&data_file(&dir), "account:beta");
    damage_last(&data_file(&dir), "account:beta");
    let err = refused(&dir);
    assert!(err.contains(&format!("damaged at byte {at}")), "{err}");
}

#[test]
fn a_damaged_commit_record_at_the_end_is_not_taken_for_a_cut_write() {
    // P1. The last record of the file, whole, its checksum wrong: no crash
    // of the process writes that. Before, the acknowledged transfer was
    // left out and its bytes cut away, the start saying only "incomplete
    // log discarded".
    let dir = transferred("txn_damaged_last");
    let at = record_of(&data_file(&dir), "account:beta");
    damage_last(&data_file(&dir), "account:beta");
    let err = refused(&dir);
    assert!(err.contains(&format!("damaged at byte {at}: a whole record whose checksum does not match")), "{err}");
}

#[test]
fn a_damaged_header_in_the_middle_stops_the_start_instead_of_cutting_the_rest() {
    // P2. One bit of the size of the very first record: before, the start
    // took it for a write cut short and cut the whole file away, every key
    // lost, with no copy.
    let dir = transferred("txn_damaged_header");
    let file = data_file(&dir);
    let mut b = std::fs::read(&file).unwrap();
    b[16 + 6 + 3] ^= 0x10;
    std::fs::write(&file, &b).unwrap();
    let err = refused(&dir);
    assert!(err.contains("damaged at byte 16: a record header"), "{err}");
    // The doctor says the same: damage, not a write cut short.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .args(["doctor", "--home"])
        .arg(dir.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FAIL  Data        workspace 'default': damaged at byte 16: a record header"), "{text}");
    assert!(text.contains("restore it from a backup"), "{text}");
    assert!(!text.contains("a write was interrupted"), "{text}");

    // A smaller change of the size, a header that could be a record's: the
    // valid record after it tells it from a cut write.
    let dir = transferred("txn_damaged_size");
    let file = data_file(&dir);
    let mut b = std::fs::read(&file).unwrap();
    b[16 + 6 + 1] ^= 0x10;
    std::fs::write(&file, &b).unwrap();
    let err = refused(&dir);
    assert!(err.contains("damaged at byte 16: a record header that does not frame its record"), "{err}");
}

#[test]
fn a_broken_batch_at_the_end_stops_the_start() {
    // R3. A damaged record of a batch whose commit record was cut away.
    let dir = transferred("txn_broken_tail");
    let file = data_file(&dir);
    let at = record_of(&file, "account:alpha");
    damage_last(&file, "account:alpha");
    let beta = record_of(&file, "account:beta");
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_len(beta as u64).unwrap();
    let err = refused(&dir);
    assert!(err.contains(&format!("damaged at byte {at}")), "{err}");
}

#[test]
fn a_log_cut_in_the_middle_of_a_transaction_keeps_what_it_cuts() {
    // R4 and P3. A cut inside the commit record: a crash. The transfer is
    // left out, and the bytes cut away are kept, in a copy of their own.
    let dir = transferred("txn_cut");
    let file = data_file(&dir);
    let whole = std::fs::read(&file).unwrap();
    let alpha = record_of(&file, "account:alpha");
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_len(whole.len() as u64 - 3).unwrap();
    assert_eq!(balances(&dir), before());
    let kept = copies(&dir, "cut");
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(&kept[0]).unwrap(), whole[alpha..whole.len() - 3]);
    assert_eq!(std::fs::metadata(&file).unwrap().len(), alpha as u64);
    assert_eq!(balances(&dir), before());
    assert_eq!(copies(&dir, "cut").len(), 1, "nothing more to cut");

    // A second cut write: a second copy, the first one untouched.
    let first = std::fs::read(&kept[0]).unwrap();
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "begin");
        ok(&mut s, "update account:alpha 1");
        ok(&mut s, "update account:beta 2");
        ok(&mut s, "commit");
        drop(s);
        inst.shutdown();
    }
    let _ = std::fs::remove_file(dir.path().join("data").join("store.kiva.hint"));
    let len = std::fs::metadata(&file).unwrap().len();
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_len(len - 5).unwrap();
    assert_eq!(balances(&dir), before());
    let kept = copies(&dir, "cut");
    assert_eq!(kept.len(), 2);
    assert_eq!(std::fs::read(&kept[0]).unwrap(), first);
}

#[test]
fn an_end_never_written_is_a_cut_write() {
    // A power cut can leave the end of a file allotted but never written:
    // zeros. Cut away as a write cut short, and kept.
    let dir = transferred("txn_zeros");
    let file = data_file(&dir);
    let alpha = record_of(&file, "account:alpha");
    let mut b = std::fs::read(&file).unwrap();
    for x in &mut b[alpha..] {
        *x = 0;
    }
    std::fs::write(&file, &b).unwrap();
    assert_eq!(balances(&dir), before());
    assert_eq!(std::fs::read(&copies(&dir, "cut")[0]).unwrap(), b[alpha..]);
}

#[test]
fn a_damaged_last_header_that_looks_cut_is_cut_and_kept() {
    // The limit of the format: the size of the very last record made
    // larger, nothing valid after it. It cannot be told from a write cut
    // short. The transfer is left out, as after a crash, and its bytes are
    // kept: not lost, and named at the start.
    let dir = transferred("txn_last_size");
    let file = data_file(&dir);
    let beta = record_of(&file, "account:beta");
    let alpha = record_of(&file, "account:alpha");
    let mut b = std::fs::read(&file).unwrap();
    b[beta + 6] ^= 0x40;
    std::fs::write(&file, &b).unwrap();
    assert_eq!(balances(&dir), before());
    assert_eq!(std::fs::read(&copies(&dir, "cut")[0]).unwrap(), b[alpha..]);
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

#[test]
fn only_the_keys_written_are_checked_at_commit() {
    // The isolation of DOCUMENTATION.md 13.7: two transactions that each
    // read both keys and write one are both committed (write skew)...
    let dir = TempDir::new("txn_skew");
    let inst = instance(&dir);
    let mut a = Session::new(inst.clone(), true, None);
    let mut b = Session::new(inst.clone(), true, None);
    ok(&mut a, "set on_call:ann 1");
    ok(&mut a, "set on_call:bob 1");
    for s in [&mut a, &mut b] {
        ok(s, "begin");
        ok(s, "get on_call:ann");
        ok(s, "get on_call:bob");
    }
    ok(&mut a, "update on_call:ann 0");
    ok(&mut b, "update on_call:bob 0");
    assert!(ok(&mut a, "commit").contains("Transaction committed"));
    assert!(ok(&mut b, "commit").contains("Transaction committed"));
    assert_eq!(ok(&mut a, "get on_call:ann"), "on_call:ann: 0 [int]");
    assert_eq!(ok(&mut a, "get on_call:bob"), "on_call:bob: 0 [int]");

    // ...and writing the key the decision depends on, as the documentation
    // says to, makes the second one conflict.
    ok(&mut a, "update on_call:ann 1");
    ok(&mut a, "update on_call:bob 1");
    for s in [&mut a, &mut b] {
        ok(s, "begin");
        ok(s, "get on_call:ann");
        ok(s, "get on_call:bob");
    }
    ok(&mut a, "update on_call:ann 0");
    ok(&mut a, "update on_call:bob 1");
    ok(&mut b, "update on_call:ann 1");
    ok(&mut b, "update on_call:bob 0");
    assert!(ok(&mut a, "commit").contains("Transaction committed"));
    let t = b.execute("commit").text();
    assert!(t.contains("modified by another session"), "{t}");
}

#[test]
fn a_damaged_collection_does_not_stop_the_others() {
    let dir = TempDir::new("txn_damaged_collection");
    let store;
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "set here 1");
        ok(&mut s, "collection create bank");
        ok(&mut s, "use bank");
        ok(&mut s, "set account:alpha 100000");
        ok(&mut s, "set account:beta 100000");
        store = inst.cfg().workspace_dir("bank").unwrap().join("store.kiva");
        drop(s);
        inst.shutdown();
    }
    let _ = std::fs::remove_file(store.with_extension("kiva.hint"));
    damage_last(&store, "account:alpha");
    let bytes = std::fs::read(&store).unwrap();

    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    assert_eq!(ok(&mut s, "get here"), "here: 1 [int]");
    let t = s.execute("use bank").text();
    assert!(t.contains("ERR_DATA_CORRUPTED") && t.contains("is damaged at byte"), "{t}");
    assert_eq!(ok(&mut s, "get here"), "here: 1 [int]");
    drop(s);
    inst.shutdown();
    assert_eq!(std::fs::read(&store).unwrap(), bytes);
}
