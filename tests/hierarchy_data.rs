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
    assert!(fails(&mut s, "compact").contains("only hierarchy metadata commands"));
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

#[cfg(feature = "crash-points")]
#[test]
fn encrypted_managed_transactions_recover_across_the_commit_boundary() {
    use std::process::Command;

    for (point, committed) in [("batch.before_append", false), ("batch.after_append", true)] {
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
            drop(s);
            inst.shutdown();
        }
        let out = Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .arg("--home")
            .arg(dir.path())
            .args([
                "--no-listen",
                "-e",
                "hierarchy data",
                "-e",
                "use collection Trading.app.users",
                "-e",
                "begin",
                "-e",
                r#"set secret "encrypted payload""#,
                "-e",
                "commit",
            ])
            .env("KIVADB_CRASH_ARMED", "1")
            .env("KIVADB_CRASH_POINT", point)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("KIVADB-CRASH-POINT"), "{out:?}");
        let inst = instance(&dir);
        let mut s = Session::new(inst, true, None);
        ok(&mut s, "hierarchy data");
        ok(&mut s, "use collection Trading.app.users");
        assert_eq!(ok(&mut s, "get existing"), "existing: 42 [int]");
        let value = ok(&mut s, "get secret");
        assert_eq!(value.contains("encrypted payload"), committed, "{point}: {value}");
        let path = dir.path().join("workspaces/Trading/projects/app/collections/users/store.kiva");
        assert!(!std::fs::read(path).unwrap().windows(17).any(|value| value == b"encrypted payload"));
    }
}
