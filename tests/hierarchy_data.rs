// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::session::Session;

fn provision(s: &mut Session) {
    ok(s, "hierarchy data");
    ok(s, "workspace create Fordima Trading");
    for workspace in ["Fordima", "Trading"] {
        ok(s, &format!("switch {workspace}"));
        ok(s, "project create app");
        ok(s, "use app");
        ok(s, "collection create users");
    }
}

#[test]
fn data_lives_in_the_selected_collection_and_survives_restart() {
    let dir = TempDir::new("managed_data");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "set legacy 99");
        provision(&mut s);
        assert!(fails(&mut s, "set key 1").contains("No collection selected"));
        ok(&mut s, "use collection users");
        ok(&mut s, "set key 42");
        ok(&mut s, r#"set doc:1 {"city":"Paris","embedding":[1,0]}"#);
        ok(&mut s, "set date born '2004-05-09'");
        ok(&mut s, "create index on doc:* (city)");
        ok(&mut s, "create vector index on doc:* (embedding)");
        ok(&mut s, r#"schema set doc: {"city":{"type":"string"}}"#);
        ok(&mut s, "changes enable");
        ok(&mut s, "set transient 'key' ttl 1");
        ok(&mut s, "set logged 7");
        assert!(ok(&mut s, "changes read from beginning").contains("logged"));
        assert!(ok(&mut s, "get similar doc:*.embedding to [1,0] limit 1").contains("doc:1"));
        assert!(ok(&mut s, r#"explain get where doc:*.city = "Paris""#).contains("INDEX SCAN"));
        assert_eq!(ok(&mut s, "get born"), "born: 2004-05-09 [date]");
        assert!(ok(&mut s, "get legacy").contains("(nil)"));
        ok(&mut s, "use collection Fordima.app.users");
        ok(&mut s, "set key 17");
        assert!(ok(&mut s, "indexes").contains("No secondary indexes"));
        assert!(ok(&mut s, "schemas").contains("No schemas"));
        assert!(ok(&mut s, "changes status").contains("keeps no change journal"));
        ok(&mut s, "use collection Trading.app.users");
        assert_eq!(ok(&mut s, "get key"), "key: 42 [int]");
        drop(s);
        inst.shutdown();
    }
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    assert!(ok(&mut s, "get key").contains("(nil)"));
    assert_eq!(ok(&mut s, "get legacy"), "legacy: 99 [int]");
    ok(&mut s, "hierarchy data");
    ok(&mut s, "use collection Trading.app.users");
    assert_eq!(ok(&mut s, "get key"), "key: 42 [int]");
    assert!(ok(&mut s, "indexes").contains("doc:*"));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(ok(&mut s, "get transient").contains("(nil)"));
    assert!(ok(&mut s, "check").contains("CORRUPTED: 0"));
    assert!(ok(&mut s, "compact").contains("compacted successfully"));
    ok(&mut s, "use collection Fordima.app.users");
    assert_eq!(ok(&mut s, "get key"), "key: 17 [int]");
}

#[test]
fn permission_checks_cover_ddl_revocation_and_transaction_commit() {
    let dir = TempDir::new("managed_data_grants");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "create user editor identified by test-pass role editor");
    provision(&mut admin);
    ok(&mut admin, "use collection users");
    ok(&mut admin, "set before 42");
    ok(&mut admin, "hierarchy grant editor manage collection Trading.app.users");
    let mut editor = Session::new(inst.clone(), true, None);
    editor.login("editor", "test-pass").unwrap();
    ok(&mut editor, "hierarchy data");
    ok(&mut editor, "use collection Trading.app.users");
    assert!(fails(&mut editor, "get before").contains("insufficient permission"));
    assert!(fails(&mut editor, "use collection Fordima.app.users").contains("unavailable"));
    ok(&mut admin, "hierarchy grant editor read collection Trading.app.users");
    assert_eq!(ok(&mut editor, "get before"), "before: 42 [int]");
    assert!(fails(&mut editor, "set before 17").contains("insufficient permission"));
    // The historical plural command dispatches to the same schema handler.
    assert!(fails(&mut editor, r#"schemas set doc: {"name":{"type":"string"}}"#).contains("insufficient permission"));
    assert!(fails(&mut editor, "changes read as cursor").contains("insufficient permission"));
    ok(&mut admin, "hierarchy grant editor write collection Trading.app.users");
    ok(&mut editor, "begin");
    ok(&mut editor, "set unpublished 17");
    assert!(fails(&mut editor, "switch Fordima").contains("during a transaction"));
    assert!(fails(&mut admin, "collection rename users to members").contains("transaction is active"));
    assert!(fails(&mut admin, "collection drop users").contains("transaction is active"));
    ok(&mut admin, "hierarchy revoke editor write collection Trading.app.users");
    assert!(fails(&mut editor, "commit").contains("insufficient permission"));
    assert!(editor.txn.is_none());
    assert!(ok(&mut admin, "get unpublished").contains("(nil)"));
    ok(&mut admin, "collection rename users to members");
    assert_eq!(ok(&mut editor, "get before"), "before: 42 [int]");
    ok(&mut admin, "collection drop members");
    ok(&mut admin, "collection create members");
    assert!(fails(&mut editor, "get before").contains("unavailable"));
    assert!(fails(&mut editor, "use collection Trading.app.members").contains("unavailable"));
    ok(&mut admin, "use collection members");
    assert!(ok(&mut admin, "get before").contains("(nil)"));
}

#[test]
fn pending_transactions_are_isolated_and_rollback_releases_the_engine() {
    let dir = TempDir::new("managed_data_transactions");
    let inst = instance(&dir);
    let mut first = Session::new(inst.clone(), true, None);
    let mut second = Session::new(inst, true, None);
    provision(&mut first);
    ok(&mut first, "use collection users");
    ok(&mut second, "hierarchy data");
    ok(&mut second, "use collection Trading.app.users");
    ok(&mut first, "begin");
    ok(&mut first, "set key 1");
    assert!(ok(&mut second, "get key").contains("(nil)"));
    ok(&mut first, "commit");
    assert_eq!(ok(&mut second, "get key"), "key: 1 [int]");
    ok(&mut second, "begin");
    ok(&mut second, "update key 2");
    ok(&mut second, "rollback");
    assert_eq!(ok(&mut first, "get key"), "key: 1 [int]");
    ok(&mut first, "collection rename users to moved");
    assert_eq!(ok(&mut second, "get key"), "key: 1 [int]");
}

#[test]
fn native_and_structured_data_routes_stay_closed() {
    let dir = TempDir::new("managed_data_transports");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    provision(&mut s);
    ok(&mut s, "use collection users");
    assert!(kivadb::api::handle(&mut s, &serde_json::json!({"op":"scan"})).is_err());
    assert!(s.kv_get("something").is_err());
    assert!(fails(&mut s, "eval 1").contains("metadata commands"));
    assert!(fails(&mut s, "backup to exported").contains("metadata commands"));
    ok(&mut s, "hierarchy off");
    ok(&mut s, "modify user kiva password admin-pw");
    ok(&mut s, "config set auth_required true");
    let port = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let mut remote = kivadb::client::Client::connect(("127.0.0.1", port)).unwrap();
    remote.auth("kiva", "admin-pw").unwrap();
    assert!(remote.exec("hierarchy data").unwrap_err().to_string().contains("local shell"));
}

#[test]
fn concurrent_readers_and_writers_observe_revocation_after_confirmation() {
    let dir = TempDir::new("managed_data_concurrent");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "create user writer identified by test-pass role editor");
    provision(&mut admin);
    ok(&mut admin, "use collection users");
    ok(&mut admin, "set key 1");
    ok(&mut admin, "hierarchy grant writer write collection Trading.app.users");

    let mut writer = Session::new(inst.clone(), true, None);
    writer.login("writer", "test-pass").unwrap();
    ok(&mut writer, "hierarchy data");
    ok(&mut writer, "use collection Trading.app.users");
    let mut reader = Session::new(inst, true, None);
    ok(&mut reader, "hierarchy data");
    ok(&mut reader, "use collection Trading.app.users");
    let barrier = std::sync::Barrier::new(3);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            let _ = writer.execute("set racing 2");
        });
        scope.spawn(|| {
            barrier.wait();
            assert_eq!(ok(&mut reader, "get key"), "key: 1 [int]");
        });
        barrier.wait();
        ok(&mut admin, "hierarchy revoke writer write collection Trading.app.users");
    });
    assert!(fails(&mut writer, "set after 3").contains("insufficient permission"));
    assert!(ok(&mut admin, "get after").contains("(nil)"));
}

#[test]
fn concurrent_retirement_blocks_old_sessions_from_the_replacement() {
    let dir = TempDir::new("managed_data_retirement");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    provision(&mut admin);
    ok(&mut admin, "use collection users");
    let mut writer = Session::new(inst, true, None);
    ok(&mut writer, "hierarchy data");
    ok(&mut writer, "use collection Trading.app.users");
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            let result = writer.execute("set racing 2");
            if result.has_err() {
                assert!(result.lines.iter().any(|(_, line)| line.contains("unavailable")));
            }
        });
        barrier.wait();
        ok(&mut admin, "collection drop users");
    });
    ok(&mut admin, "collection create users");
    assert!(fails(&mut writer, "set stale 3").contains("unavailable"));
    ok(&mut admin, "use collection users");
    assert!(ok(&mut admin, "get racing").contains("(nil)"));
    assert!(ok(&mut admin, "get stale").contains("(nil)"));
}

#[cfg(feature = "crash-points")]
#[test]
fn encrypted_managed_writes_recover_across_implicit_and_explicit_commits() {
    use std::process::Command;

    for (point, committed, transaction) in [
        ("batch.before_append", false, true),
        ("batch.after_append", true, true),
        ("batch.torn_append", false, false),
        ("batch.after_append", true, false),
    ] {
        let dir = TempDir::new("managed_data_crash");
        let keyfile = dir.path().join("test.key");
        std::fs::write(&keyfile, "b3".repeat(32)).unwrap();
        let mut config = kivadb::config::Config::load(dir.path()).unwrap();
        config.set("encryption", "key_file", &keyfile.display().to_string()).unwrap();
        config.set("encryption", "required", "true").unwrap();
        drop(config);
        {
            let inst = instance(&dir);
            let mut s = Session::new(inst.clone(), true, None);
            provision(&mut s);
            ok(&mut s, "use collection users");
            ok(&mut s, "set existing 42");
            if !transaction {
                ok(&mut s, r#"set source "encrypted payload""#);
            }
            drop(s);
            inst.shutdown();
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_kivadb"));
        child.arg("--home").arg(dir.path()).args([
            "--no-listen",
            "-e",
            "hierarchy data",
            "-e",
            "use collection Trading.app.users",
        ]);
        if transaction {
            child.args(["-e", "begin", "-e", r#"set secret "encrypted payload""#, "-e", "commit"]);
        } else {
            child.args(["-e", "change source to secret"]);
        }
        let out = child.env("KIVADB_CRASH_ARMED", "1").env("KIVADB_CRASH_POINT", point).output().unwrap();
        assert!(!out.status.success(), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("KIVADB-CRASH-POINT"), "{out:?}");
        let inst = instance(&dir);
        let mut s = Session::new(inst, true, None);
        ok(&mut s, "hierarchy data");
        ok(&mut s, "use collection Trading.app.users");
        assert_eq!(ok(&mut s, "get existing"), "existing: 42 [int]");
        let value = ok(&mut s, "get secret");
        assert_eq!(value.contains("encrypted payload"), committed, "{point}: {value}");
        if !transaction {
            assert_eq!(ok(&mut s, "get source").contains("encrypted payload"), !committed);
        }
        let path = dir.path().join("workspaces/Trading/projects/app/collections/users/store.kiva");
        assert!(!std::fs::read(path).unwrap().windows(17).any(|value| value == b"encrypted payload"));
    }
}

#[test]
fn registry_evicts_the_least_recent_idle_store_and_reports_saturation() {
    let dir = TempDir::new("registry_lru");
    let mut cfg = kivadb::config::Config::load(dir.path()).unwrap();
    cfg.set("storage", "managed_open_engines", "2").unwrap();
    drop(cfg);
    let inst = instance(&dir);
    let mut a = Session::new(inst.clone(), true, None);
    provision(&mut a);
    ok(&mut a, "collection create positions");
    ok(&mut a, "use collection Trading.app.users");
    ok(&mut a, "set pinned 10");
    ok(&mut a, "begin");

    let mut b = Session::new(inst.clone(), true, None);
    ok(&mut b, "hierarchy data");
    ok(&mut b, "use collection Fordima.app.users");
    ok(&mut b, "set pinned 20");
    ok(&mut b, "begin");

    let mut c = Session::new(inst, true, None);
    ok(&mut c, "hierarchy data");
    ok(&mut c, "use collection Trading.app.positions");
    assert!(fails(&mut c, "get pinned").contains("all open stores are in use"));
    ok(&mut b, "rollback");
    ok(&mut c, "set position 30");
    assert!(ok(&mut c, "hierarchy engines").contains("3 opens, 1 evictions, 1 saturated"));
    ok(&mut a, "commit");
    // Refresh Trading.users, leaving Trading.positions the least recent.
    ok(&mut a, "get pinned");
    assert_eq!(ok(&mut b, "get pinned"), "pinned: 20 [int]");
    assert!(ok(&mut b, "hierarchy engines").contains("4 opens, 2 evictions"));
    assert_eq!(ok(&mut c, "get position"), "position: 30 [int]");
    assert!(ok(&mut c, "hierarchy engines").contains("5 opens, 3 evictions"));
}

#[test]
fn compaction_requires_write_and_manage_and_respects_transaction_pins() {
    let dir = TempDir::new("managed_compact_gate");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "create user editor identified by test-pass role editor");
    provision(&mut admin);
    ok(&mut admin, "use collection users");
    ok(&mut admin, "set data 1");
    ok(&mut admin, "hierarchy grant editor write collection Trading.app.users");
    let mut editor = Session::new(inst, true, None);
    editor.login("editor", "test-pass").unwrap();
    ok(&mut editor, "hierarchy data");
    ok(&mut editor, "use collection Trading.app.users");
    assert!(fails(&mut editor, "compact").contains("insufficient permission"));
    ok(&mut admin, "hierarchy grant editor manage collection Trading.app.users");
    ok(&mut admin, "begin");
    ok(&mut admin, "update data 2");
    assert!(fails(&mut editor, "compact").contains("collection transaction"));
    ok(&mut admin, "commit");
    assert!(ok(&mut editor, "compact").contains("compacted successfully"));
    assert_eq!(ok(&mut editor, "get data"), "data: 2 [int]");
    ok(&mut admin, "collection rename users to renamed");
    assert_eq!(ok(&mut editor, "get data"), "data: 2 [int]");
}

#[cfg(feature = "crash-points")]
#[test]
fn compaction_recovers_values_and_identity_at_each_durable_boundary() {
    use std::process::Command;

    let points = [
        ("before_prepare", false),
        ("during_copy", false),
        ("after_sync", false),
        ("before_intent_install", false),
        ("before_replace", false),
        ("after_replace", true),
        ("before_publish", true),
        ("before_marker_install", true),
        ("after_publish", true),
        ("during_cleanup", true),
    ];
    for (encrypted, (point, installed)) in [false, true].into_iter().flat_map(|e| points.map(|p| (e, p))) {
        let dir = TempDir::new("managed_compact_crash");
        if encrypted {
            let keyfile = dir.path().join("test.key");
            std::fs::write(&keyfile, "b3".repeat(32)).unwrap();
            let mut config = kivadb::config::Config::load(dir.path()).unwrap();
            config.set("encryption", "key_file", &keyfile.display().to_string()).unwrap();
            config.set("encryption", "required", "true").unwrap();
        }
        {
            let inst = instance(&dir);
            let mut s = Session::new(inst.clone(), true, None);
            provision(&mut s);
            ok(&mut s, "use collection users");
            ok(&mut s, r#"set doc:1 {"city":"Paris","embedding":[1,0]}"#);
            ok(&mut s, "create index on doc:* (city)");
            ok(&mut s, "create vector index on doc:* (embedding)");
            ok(&mut s, r#"schema set doc: {"city":{"type":"string"}}"#);
            ok(&mut s, "changes enable");
            ok(&mut s, "set temporary 1 ttl 3600");
            ok(&mut s, "set dead 1");
            ok(&mut s, "del dead");
            ok(&mut s, "begin");
            ok(&mut s, "set date native '2004-05-09'");
            ok(&mut s, r#"set secret "managed recovery payload""#);
            ok(&mut s, "commit");
            drop(s);
            inst.shutdown();
        }
        let store = dir.path().join("workspaces/Trading/projects/app/collections/users");
        let snapshot = std::fs::read(store.join(".managed-identity")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .arg("--home")
            .arg(dir.path())
            .args(["--no-listen", "-e", "hierarchy data", "-e", "use collection Trading.app.users", "-e", "compact"])
            .env("KIVADB_CRASH_ARMED", "1")
            .env("KIVADB_CRASH_POINT", format!("managed_compact.{point}"))
            .output()
            .unwrap();
        assert!(!result.status.success(), "{point}: {result:?}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("KIVADB-CRASH-POINT"), "{point}: {result:?}");
        let inst = instance(&dir);
        let mut s = Session::new(inst, true, None);
        ok(&mut s, "hierarchy data");
        ok(&mut s, "use collection Trading.app.users");
        assert_eq!(ok(&mut s, "get native"), "native: 2004-05-09 [date]");
        assert_eq!(ok(&mut s, "get secret"), "secret: managed recovery payload [string]");
        assert_eq!(ok(&mut s, "get temporary"), "temporary: 1 [int]");
        assert!(ok(&mut s, "get dead").contains("(nil)"));
        assert!(ok(&mut s, r#"explain get where doc:*.city = "Paris""#).contains("INDEX SCAN"));
        assert!(ok(&mut s, "get similar doc:*.embedding to [1,0] limit 1").contains("doc:1"));
        assert!(ok(&mut s, "schemas").contains("doc:"));
        assert!(ok(&mut s, "changes read from beginning").contains("secret"));
        let identity = std::fs::read(store.join(".managed-identity")).unwrap();
        assert_eq!(identity != snapshot, installed, "{point}");
        assert!(!store.join(".managed-compaction").exists());
        assert!(!store.join(".managed-compaction.pending").exists());
        assert!(!store.join(".managed-identity.pending").exists());
        assert!(!store.join("store.kiva.compact").exists());
    }
}

#[test]
fn recreated_account_cannot_commit_or_reuse_prior_managed_permissions() {
    let dir = TempDir::new("managed_account_recreation");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "create user editor identified by test-pass role editor");
    provision(&mut admin);
    ok(&mut admin, "use collection users");
    ok(&mut admin, "hierarchy grant editor write collection Trading.app.users");
    let mut writer = Session::new(inst.clone(), true, None);
    writer.login("editor", "test-pass").unwrap();
    ok(&mut writer, "hierarchy data");
    ok(&mut writer, "use collection Trading.app.users");
    ok(&mut writer, "begin");
    ok(&mut writer, "set uncommitted 1");
    let mut accounts = Session::new(inst, true, None);
    ok(&mut accounts, "drop user editor");
    ok(&mut accounts, "create user editor identified by test-pass role editor");
    assert!(fails(&mut writer, "commit").contains("unavailable"));
    assert!(ok(&mut admin, "get uncommitted").contains("(nil)"));
    writer.login("editor", "test-pass").unwrap();
    ok(&mut writer, "hierarchy data");
    assert!(fails(&mut writer, "use collection Trading.app.users").contains("unavailable"));
}

#[test]
fn an_unrelated_compaction_intent_is_rejected_without_altering_data() {
    let dir = TempDir::new("managed_compact_substitution");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        provision(&mut s);
        ok(&mut s, "use collection users");
        ok(&mut s, "set untouched 42");
        drop(s);
        inst.shutdown();
    }
    let collection = dir.path().join("workspaces/Trading/projects/app/collections/users");
    let store = collection.join("store.kiva");
    let before = std::fs::read(&store).unwrap();
    let journal = collection.join(".managed-compaction");
    std::fs::write(&journal, r#"{"collection":0,"before":1,"after":2}"#).unwrap();
    let error = kivadb::Instance::open(dir.path(), kivadb::Options::default()).err().expect("invalid intent accepted");
    assert!(error.to_string().contains("managed compaction identity mismatch"));
    assert_eq!(std::fs::read(&store).unwrap(), before);
    std::fs::remove_file(journal).unwrap();
    let mut s = Session::new(instance(&dir), true, None);
    ok(&mut s, "hierarchy data");
    ok(&mut s, "use collection Trading.app.users");
    assert_eq!(ok(&mut s, "get untouched"), "untouched: 42 [int]");
}
