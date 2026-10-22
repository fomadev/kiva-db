// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Encryption at rest: what reaches the disk, keys, migration and rotation.

mod common;

use common::{ok, TempDir};
use kivadb::crypt::Keys;
use kivadb::engine::{Engine, EngineOptions, VectorIndexKind};
use kivadb::format::{parse_header, FLAG_ENCRYPTED, REC_HEADER_LEN};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use kivadb::value::Value;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

fn keys(seed: u8) -> Option<Arc<Keys>> {
    Some(Arc::new(Keys::new(&[seed; 32], &[])))
}

fn opts(keys: Option<Arc<Keys>>) -> EngineOptions {
    EngineOptions { background: false, auto_compact: false, keys, ..Default::default() }
}

fn contains(path: &Path, needle: &str) -> bool {
    std::fs::read(path).unwrap().windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn sidecar(path: &Path, ext: &str) -> PathBuf {
    PathBuf::from(format!("{}.{ext}", path.display()))
}

fn fill(e: &Engine) {
    e.put(
        "patient:1",
        &Value::Doc(json!({ "name": "Mbuyi", "diagnosis": "malaria", "embedding": [1.0, 0.0] })),
        0,
        false,
    )
    .unwrap();
    e.put("patient:2", &Value::Doc(json!({ "name": "Kabila", "diagnosis": "flu", "embedding": [0.0, 1.0] })), 0, false)
        .unwrap();
    e.put("iban", &Value::Str("CD08-0001-SECRET".into()), 0, false).unwrap();
    e.put("draft", &Value::Str("to delete".into()), 0, false).unwrap();
    e.delete("draft").unwrap();
}

#[test]
fn nothing_readable_reaches_the_disk_and_the_key_reads_it_back() {
    let dir = TempDir::new("enc_disk");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, opts(keys(1))).unwrap();
    fill(&e);
    e.create_vector_index_with("patient:*", "embedding", VectorIndexKind::Hnsw(Default::default())).unwrap();
    e.close();
    drop(e);

    for f in [path.clone(), sidecar(&path, "hint"), sidecar(&path, "graphs")] {
        assert!(f.exists(), "{} was not written", f.display());
        for secret in ["patient:1", "Mbuyi", "malaria", "CD08-0001-SECRET", "draft"] {
            assert!(!contains(&f, secret), "'{secret}' is readable in {}", f.display());
        }
    }

    // Same key: everything is back, from the hint and from a full scan.
    for scan in [false, true] {
        if scan {
            std::fs::remove_file(sidecar(&path, "hint")).unwrap();
        }
        let e = Engine::open(&path, opts(keys(1))).unwrap();
        assert_eq!(e.get("iban").unwrap().unwrap().0, Value::Str("CD08-0001-SECRET".into()));
        assert_eq!(e.get("patient:1").unwrap().unwrap().0.to_json()["diagnosis"], "malaria");
        assert!(e.get("draft").unwrap().is_none());
        assert_eq!(e.len(), 3);
        let r = e.encryption_report().unwrap();
        assert_eq!((r.plain, r.other_key), (0, 0));
        assert!(r.records >= 5);
        e.close();
    }

    // No key, or another key: refused with a clear reason, nothing is changed.
    let no_key = Engine::open(&path, opts(None)).err().unwrap().to_string();
    assert!(no_key.contains("KIVADB_ENCRYPTION_KEY"), "{no_key}");
    let wrong = Engine::open(&path, opts(keys(2))).err().unwrap().to_string();
    assert!(wrong.contains("encrypted with key"), "{wrong}");
    assert!(Engine::open(&path, opts(keys(1))).is_ok());
}

#[test]
fn a_changed_byte_is_detected_even_with_a_valid_checksum() {
    let dir = TempDir::new("enc_tamper");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, opts(keys(1))).unwrap();
    e.put("balance", &Value::Str("100".into()), 0, false).unwrap();
    e.put("other", &Value::Str("x".into()), 0, false).unwrap();
    e.close();
    drop(e);

    // Flip a byte of the first record's value and fix its CRC, as an attacker would.
    let mut data = std::fs::read(&path).unwrap();
    let start = 16;
    let h = parse_header(&data[start..]);
    assert!(h.flags & FLAG_ENCRYPTED != 0);
    let end = start + h.total_len() as usize;
    data[start + REC_HEADER_LEN + h.k_size as usize] ^= 0x55;
    let crc = crc32fast::hash(&data[start + 4..end]);
    data[start..start + 4].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&path, &data).unwrap();
    std::fs::remove_file(sidecar(&path, "hint")).unwrap();

    let e = Engine::open(&path, opts(keys(1))).unwrap();
    assert!(e.get("balance").unwrap().is_none(), "the altered record must not be accepted");
    assert_eq!(e.get("other").unwrap().unwrap().0, Value::Str("x".into()));
    assert_eq!(e.stats().corrupted_detected, 1);
}

#[test]
fn existing_data_is_encrypted_by_a_compaction_and_keys_can_rotate() {
    let dir = TempDir::new("enc_migrate");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, opts(None)).unwrap();
    fill(&e);
    e.close();
    drop(e);
    assert!(contains(&path, "malaria"));

    // With a key: the old records stay readable; new ones are encrypted.
    let e = Engine::open(&path, opts(keys(1))).unwrap();
    assert_eq!(e.get("iban").unwrap().unwrap().0, Value::Str("CD08-0001-SECRET".into()));
    e.put("new", &Value::Str("written with the key".into()), 0, false).unwrap();
    let r = e.encryption_report().unwrap();
    assert!(r.plain >= 5 && r.records == r.plain + 1, "{r:?}");
    // A compaction rewrites everything with the key, old versions gone.
    e.compact().unwrap();
    let r = e.encryption_report().unwrap();
    assert_eq!((r.plain, r.other_key, r.records), (0, 0, 4));
    e.close();
    drop(e);
    assert!(!contains(&path, "malaria") && !contains(&path, "patient:1"));

    // Required encryption accepts this file now, and refuses one in clear.
    let strict = |k| EngineOptions { require_encryption: true, ..opts(k) };
    assert!(Engine::open(&path, strict(keys(1))).is_ok());
    let plain = dir.path().join("plain.kiva");
    Engine::open(&plain, opts(None)).unwrap().put("a", &Value::Str("1".into()), 0, false).unwrap();
    let refused = Engine::open(&plain, strict(keys(1))).err().unwrap().to_string();
    assert!(refused.contains("unencrypted record"), "{refused}");

    // Rotation: the new key reads with the old one alongside, compaction
    // re-encrypts, and the old key is no longer needed.
    let rotated = Some(Arc::new(Keys::new(&[2; 32], &[[1; 32]])));
    let e = Engine::open(&path, opts(rotated)).unwrap();
    assert_eq!(e.encryption_report().unwrap().other_key, 4);
    e.compact().unwrap();
    assert_eq!(e.encryption_report().unwrap().other_key, 0);
    e.close();
    drop(e);
    let e = Engine::open(&path, opts(keys(2))).unwrap();
    assert_eq!(e.get("new").unwrap().unwrap().0, Value::Str("written with the key".into()));
}

#[test]
fn transactions_ttl_and_replays_work_encrypted() {
    let dir = TempDir::new("enc_txn");
    let path = dir.path().join("store.kiva");
    let e = Engine::open(&path, opts(keys(7))).unwrap();
    use kivadb::engine::BatchOp;
    e.apply_batch(
        vec![
            BatchOp::Put { key: "a".into(), value: Value::Str("1".into()), expires_at: 0, msgpack: false },
            BatchOp::Put { key: "b".into(), value: Value::Doc(json!({"x": 1})), expires_at: 0, msgpack: true },
            BatchOp::Del { key: "a".into() },
        ],
        None,
    )
    .unwrap();
    e.put("short", &Value::Str("gone soon".into()), kivadb::util::now_secs() + 3600, false).unwrap();
    // A write after the snapshot is replayed from the log.
    e.snapshot();
    e.put("after", &Value::Str("replayed".into()), 0, false).unwrap();
    drop(e);
    let e = Engine::open(&path, opts(keys(7))).unwrap();
    assert!(e.get("a").unwrap().is_none());
    assert_eq!(e.get("b").unwrap().unwrap().0.to_json(), json!({"x": 1}));
    assert!(e.get("short").unwrap().unwrap().1.expires_at > 0);
    assert_eq!(e.get("after").unwrap().unwrap().0, Value::Str("replayed".into()));
    assert!(e.check(false).unwrap().corrupted.is_empty());
}

fn kivadb(home: &Path, args: &[&str], key: Option<&str>, stdin: &str) -> Output {
    use std::io::Write;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kivadb"));
    cmd.arg("--home").arg(home).args(args).env("KIVADB_NO_UPDATE_CHECK", "1").env_remove("KIVADB_ENCRYPTION_KEY");
    if let Some(k) = key {
        cmd.env("KIVADB_ENCRYPTION_KEY", k);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn the_instance_encrypts_every_workspace_and_its_users() {
    let dir = TempDir::new("enc_instance");
    let home = dir.path();
    let key = String::from_utf8(kivadb(home, &["keygen"], None, "").stdout).unwrap().trim().to_string();
    assert_eq!(key.len(), 64);

    // A key file named in kiva.conf.
    let key_file = dir.path().join("secret.key");
    std::fs::write(&key_file, &key).unwrap();
    {
        let mut cfg = kivadb::config::Config::load(home).unwrap();
        cfg.set("encryption", "key_file", &key_file.display().to_string()).unwrap();
        cfg.set("encryption", "required", "true").unwrap();
    }
    let inst = Instance::open(home, Options::default()).unwrap();
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"create user "amani" identified by "pa55-word" role editor"#);
    ok(&mut s, r#"set card {"holder": "Amani", "number": "4970-1234"}"#);
    let report = ok(&mut s, "encryption");
    assert!(report.contains("Encryption at rest: on (AES-256-GCM, key "), "{report}");
    assert!(report.contains("0 in clear"), "{report}");
    drop(s);
    drop(inst);
    assert!(!contains(&home.join("data").join("store.kiva"), "4970-1234"));
    assert!(!contains(&home.join("system").join("system.kiva"), "amani"));

    // Without its key file, the instance does not start.
    {
        let mut cfg = kivadb::config::Config::load(home).unwrap();
        cfg.set("encryption", "key_file", "").unwrap();
    }
    let e = Instance::open(home, Options::default()).err().unwrap().to_string();
    assert!(e.contains("encryption.required is on but no key"), "{e}");

    // The same key through the environment, from the command line; the
    // shell history is not written in clear.
    let o = kivadb(home, &["-e", "get card.holder"], Some(&key), "");
    assert!(String::from_utf8_lossy(&o.stdout).contains("card.holder: Amani"), "{o:?}");
    let o = kivadb(home, &[], Some(&key), "set note \"typed in the shell\"\n");
    assert!(o.status.success(), "{o:?}");
    assert!(!home.join("history.txt").exists() || !contains(&home.join("history.txt"), "typed in the shell"));
    let o = kivadb(home, &["-e", "get card"], Some(&"0".repeat(64)), "");
    assert!(String::from_utf8_lossy(&o.stderr).contains("encrypted with key"), "{o:?}");
}
