// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::engine::Engine;
use kivadb::session::Session;
use kivadb::Value;
use std::path::Path;

fn directory(home: &Path, w: &str, p: &str, c: &str) -> std::path::PathBuf {
    home.join("workspaces").join(w).join("projects").join(p).join("collections").join(c)
}

fn create(s: &mut Session) {
    ok(s, "hierarchy storage");
    ok(s, "workspace create Trading");
    ok(s, "switch Trading");
    ok(s, "project create MaplaTrade");
    ok(s, "use MaplaTrade");
    ok(s, "collection create agents positions transactions");
}

#[test]
fn managed_store_rename_retire_and_recreate_preserve_identity_and_data() {
    let dir = TempDir::new("managed_lifecycle");
    let home = dir.path();
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "set old 10");
    create(&mut s);
    let agents = directory(home, "Trading", "MaplaTrade", "agents");
    let store = agents.join("store.kiva");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&agents).unwrap().permissions().mode() & 0o077, 0);
    }
    let identity = std::fs::read(agents.join(".managed-identity")).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&identity).unwrap();
    let id = parsed["id"].as_u64().unwrap();
    assert!(directory(home, "Trading", "MaplaTrade", "positions").join("store.kiva").is_file());
    assert!(directory(home, "Trading", "MaplaTrade", "transactions").join("store.kiva").is_file());
    assert!(!home.join("workspaces/Trading/store.kiva").exists());
    assert!(!home.join("workspaces/Trading/projects/MaplaTrade/store.kiva").exists());
    {
        // Exercise existing storage independently of session routing.
        let engine = Engine::open(&store, Default::default()).unwrap();
        engine.put("preserved", &Value::Str("value".into()), 0, false).unwrap();
        engine.sync().unwrap();
    }
    let data = std::fs::read(&store).unwrap();
    assert!(fails(&mut s, "set hello 1").contains("metadata commands"));
    assert!(fails(&mut s, "collection rename agents to Positions").contains("already exists"));
    ok(&mut s, "workspace rename Trading to Finance");
    ok(&mut s, "project rename MaplaTrade to TradingEngine");
    ok(&mut s, "collection rename agents to workers");
    let renamed = directory(home, "Finance", "TradingEngine", "workers");
    assert_eq!(std::fs::read(renamed.join(".managed-identity")).unwrap(), identity);
    assert_eq!(std::fs::read(renamed.join("store.kiva")).unwrap(), data);
    assert!(!store.exists());

    ok(&mut s, "collection drop workers");
    let retired = home.join("retired").join(format!("{id:016x}")).join("store.kiva");
    assert_eq!(std::fs::read(&retired).unwrap(), data);
    ok(&mut s, "collection create workers");
    assert_ne!(std::fs::read(renamed.join(".managed-identity")).unwrap(), identity);
    let fresh = Engine::open(renamed.join("store.kiva"), Default::default()).unwrap();
    assert!(fresh.get("preserved").unwrap().is_none());
    drop(fresh);
    assert!(fails(&mut s, "project drop TradingEngine").contains("cascade"));
    ok(&mut s, "project drop TradingEngine cascade");
    ok(&mut s, "project create Last");
    ok(&mut s, "use Last");
    ok(&mut s, "collection create records");
    let workspace_marker = std::fs::read(home.join("workspaces/Finance/.managed-identity")).unwrap();
    let workspace_id = serde_json::from_slice::<serde_json::Value>(&workspace_marker).unwrap()["id"].as_u64().unwrap();
    let records = directory(home, "Finance", "Last", "records").join("store.kiva");
    let records_bytes = std::fs::read(records).unwrap();
    assert!(fails(&mut s, "workspace drop Finance").contains("cascade"));
    ok(&mut s, "workspace drop Finance cascade");
    assert_eq!(
        std::fs::read(
            home.join("retired")
                .join(format!("{workspace_id:016x}"))
                .join("projects/Last/collections/records/store.kiva")
        )
        .unwrap(),
        records_bytes
    );
    assert_eq!(std::fs::read(retired).unwrap(), data);
    drop(s);
    inst.shutdown();
    drop(inst);

    let reopened = instance(&dir);
    let mut s = Session::new(reopened, true, None);
    assert_eq!(ok(&mut s, "get old"), "old: 10 [int]");
    assert!(fails(&mut s, "workspace create legacy").contains("frozen"));
    ok(&mut s, "hierarchy storage");
    assert_eq!(ok(&mut s, "workspace list"), "default");
}

#[test]
fn preexisting_directories_are_not_adopted_including_case_collisions() {
    let dir = TempDir::new("managed_collisions");
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, "hierarchy storage");
    let existing = dir.path().join("workspaces/tRaDiNg");
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("unregistered"), b"untouched").unwrap();
    assert!(fails(&mut s, "workspace create Trading").contains("already exists"));
    assert_eq!(ok(&mut s, "workspace exists Trading"), "false");
    assert_eq!(std::fs::read(existing.join("unregistered")).unwrap(), b"untouched");
    assert!(fails(&mut s, "workspace create ../outside").contains("invalid"));
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_refused() {
    let dir = TempDir::new("managed_symlink");
    let unrelated = TempDir::new("managed_symlink_target");
    let inst = instance(&dir);
    std::os::unix::fs::symlink(unrelated.path(), dir.path().join("workspaces")).unwrap();
    let mut s = Session::new(inst, true, None);
    assert!(fails(&mut s, "hierarchy storage").contains("unsafe managed path"));
    assert_eq!(std::fs::read_dir(unrelated.path()).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn windows_junction_is_refused_without_touching_the_target() {
    let dir = TempDir::new("managed_junction");
    let unrelated = TempDir::new("managed_junction_target");
    let inst = instance(&dir);
    let link = dir.path().join("workspaces");
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(unrelated.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let mut s = Session::new(inst, true, None);
    assert!(fails(&mut s, "hierarchy storage").contains("reparse point"));
    assert_eq!(std::fs::read_dir(unrelated.path()).unwrap().count(), 0);
    std::fs::remove_dir(&link).unwrap();
}

#[cfg(feature = "crash-points")]
#[test]
fn interrupted_create_rename_and_retirement_recover_by_identity() {
    use std::process::Command;

    for (command, point, expected) in [
        ("collection create second", "managed_layout.after_intent", "second"),
        ("collection create second", "managed_layout.before_directory", "second"),
        ("collection create second", "managed_layout.after_directory", "second"),
        ("collection create second", "managed_layout.before_store", "second"),
        ("collection create second", "managed_layout.after_store", "second"),
        ("collection rename agents to people", "managed_layout.before_move", "people"),
        ("collection rename agents to people", "managed_layout.after_move", "people"),
        ("collection drop agents", "managed_layout.before_publish", "retired"),
        ("collection drop agents", "managed_layout.after_publish", "retired"),
    ] {
        let dir = TempDir::new("managed_crash");
        let home = dir.path();
        {
            let inst = instance(&dir);
            let mut s = Session::new(inst.clone(), true, None);
            create(&mut s);
            drop(s);
            inst.shutdown();
        }
        let agents = directory(home, "Trading", "MaplaTrade", "agents");
        let identity = std::fs::read(agents.join(".managed-identity")).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&identity).unwrap();
        let id = parsed["id"].as_u64().unwrap();
        {
            let engine = Engine::open(agents.join("store.kiva"), Default::default()).unwrap();
            engine.put("saved", &Value::Str("before crash".into()), 0, false).unwrap();
            engine.sync().unwrap();
        }
        let data = std::fs::read(agents.join("store.kiva")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .arg("--home")
            .arg(home)
            .args([
                "--no-listen",
                "-e",
                "hierarchy storage",
                "-e",
                "switch Trading",
                "-e",
                "use MaplaTrade",
                "-e",
                command,
            ])
            .env("KIVADB_CRASH_ARMED", "1")
            .env("KIVADB_CRASH_POINT", point)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{point}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("KIVADB-CRASH-POINT"), "{point}: {out:?}");

        let recovered = instance(&dir);
        let mut s = Session::new(recovered, true, None);
        ok(&mut s, "hierarchy storage");
        ok(&mut s, "switch Trading");
        ok(&mut s, "use MaplaTrade");
        if expected == "second" {
            assert_eq!(ok(&mut s, "collection exists second"), "true");
            assert_eq!(std::fs::read(agents.join("store.kiva")).unwrap(), data);
        } else {
            assert_eq!(ok(&mut s, "collection exists agents"), "false");
            let path = if expected == "retired" {
                home.join("retired").join(format!("{id:016x}"))
            } else {
                directory(home, "Trading", "MaplaTrade", expected)
            };
            assert_eq!(std::fs::read(path.join(".managed-identity")).unwrap(), identity);
            assert_eq!(std::fs::read(path.join("store.kiva")).unwrap(), data);
        }
    }
}

#[test]
fn managed_collection_initialization_reuses_the_home_encryption_options() {
    let dir = TempDir::new("managed_encryption");
    let keyfile = dir.path().join("test.key");
    std::fs::write(&keyfile, "a1".repeat(32)).unwrap();
    let mut config = kivadb::config::Config::load(dir.path()).unwrap();
    config.set("encryption", "key_file", &keyfile.display().to_string()).unwrap();
    config.set("encryption", "required", "true").unwrap();
    drop(config);
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    create(&mut s);
    let store = directory(dir.path(), "Trading", "MaplaTrade", "agents").join("store.kiva");
    let engine = Engine::open(&store, inst.engine_options()).unwrap();
    engine.put("confidential", &Value::Str("managed data".into()), 0, false).unwrap();
    engine.sync().unwrap();
    drop(engine);
    assert!(!std::fs::read(&store).unwrap().windows(12).any(|bytes| bytes == b"managed data"));
    assert!(Engine::open(&store, Default::default()).is_err());
    drop(s);
    inst.shutdown();
    drop(inst);
    let inst = instance(&dir);
    let engine = Engine::open(&store, inst.engine_options()).unwrap();
    assert_eq!(engine.get("confidential").unwrap().unwrap().0, Value::Str("managed data".into()));
}

#[test]
fn an_open_store_prevents_physical_rename_and_retirement() {
    let dir = TempDir::new("managed_store_busy");
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    create(&mut s);
    let store = directory(dir.path(), "Trading", "MaplaTrade", "agents").join("store.kiva");
    let engine = Engine::open(&store, Default::default()).unwrap();
    assert!(fails(&mut s, "workspace rename Trading to Finance").contains("locked"));
    assert!(fails(&mut s, "collection drop agents").contains("locked"));
    assert_eq!(ok(&mut s, "workspace current"), "Trading");
    assert_eq!(ok(&mut s, "collection exists agents"), "true");
    drop(engine);
    ok(&mut s, "workspace rename Trading to Finance");
}

#[test]
fn a_staged_legacy_migration_remains_protected() {
    let dir = TempDir::new("managed_staged");
    {
        let inst = instance(&dir);
        inst.shutdown();
    }
    kivadb::migration::run(dir.path(), "apply").unwrap();
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    assert!(fails(&mut s, "hierarchy storage").contains("staged migration"));
    assert!(!dir.path().join("system/managed-layout.json").exists());
}

#[test]
fn simultaneous_collection_creation_has_one_winner() {
    let dir = TempDir::new("managed_concurrent");
    let inst = instance(&dir);
    let mut first = Session::new(inst.clone(), true, None);
    create(&mut first);
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let run = || {
            let mut session = Session::new(inst.clone(), true, None);
            ok(&mut session, "hierarchy storage");
            ok(&mut session, "switch Trading");
            ok(&mut session, "use MaplaTrade");
            barrier.wait();
            session.execute("collection create shared").has_err()
        };
        let a = scope.spawn(run);
        let b = scope.spawn(run);
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_ne!(results.0, results.1);
    let shared = directory(dir.path(), "Trading", "MaplaTrade", "shared");
    assert!(shared.join("store.kiva").is_file());
    assert_eq!(ok(&mut first, "collection exists shared"), "true");
}

#[test]
fn startup_refuses_a_store_replaced_with_another_valid_store() {
    let dir = TempDir::new("managed_wrong_identity");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        create(&mut s);
        drop(s);
        inst.shutdown();
    }
    let source = directory(dir.path(), "Trading", "MaplaTrade", "positions").join("store.kiva");
    let target = directory(dir.path(), "Trading", "MaplaTrade", "agents").join("store.kiva");
    std::fs::copy(source, &target).unwrap();
    let bytes = std::fs::read(&target).unwrap();
    let reopened = kivadb::instance::Instance::open(dir.path(), Default::default());
    let error = reopened.err().expect("mismatched store was reopened");
    assert!(error.to_string().contains("managed store header mismatch"));
    assert_eq!(std::fs::read(target).unwrap(), bytes);
}
