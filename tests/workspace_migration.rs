// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

mod common;

use common::{instance, ok, TempDir};
use kivadb::migration;
use kivadb::session::Session;
use kivadb::KivaError;
use std::path::Path;

fn prepare(dir: &TempDir) {
    let inst = instance(dir);
    let mut session = Session::new(inst.clone(), true, None);
    ok(&mut session, "set before 42");
    drop(session);
    inst.shutdown();
}

#[test]
fn offline_apply_preserves_legacy_bytes_and_rollback_keeps_new_writes() {
    let dir = TempDir::new("migration_offline");
    prepare(&dir);
    let home = dir.path();
    let store = home.join("data/store.kiva");
    let original = std::fs::read(&store).unwrap();
    let system = std::fs::read(home.join("system/system.kiva")).unwrap();
    assert!(migration::run(home, "plan").unwrap().contains("default.legacy.default"));
    assert_eq!(migration::run(home, "status").unwrap(), "NOT STARTED");
    assert!(migration::run(home, "apply").unwrap().starts_with("STAGED"));
    assert!(migration::run(home, "verify").unwrap().starts_with("VERIFIED"));
    assert!(matches!(migration::run(home, "apply"), Err(KivaError::Exists(_))));
    assert_eq!(std::fs::read(&store).unwrap(), original);
    assert_eq!(std::fs::read(home.join("system/system.kiva")).unwrap(), system);

    {
        let inst = instance(&dir);
        assert!(inst.workspace_catalog().unwrap().unwrap().resolve_collection("default", "legacy", "default").is_ok());
        let mut session = Session::new(inst.clone(), true, None);
        assert_eq!(ok(&mut session, "get before"), "before: 42 [int]");
        ok(&mut session, "set after 43");
        assert!(matches!(migration::run(home, "rollback"), Err(KivaError::Locked(_))));
        drop(session);
        inst.shutdown();
    }
    let updated = std::fs::read(&store).unwrap();
    assert_ne!(updated, original);
    assert!(migration::run(home, "rollback").unwrap().starts_with("ROLLED BACK"));
    assert!(migration::run(home, "rollback").unwrap().starts_with("ROLLED BACK"));
    assert_eq!(std::fs::read(&store).unwrap(), updated);
    let inst = instance(&dir);
    assert!(inst.workspace_catalog().unwrap().is_none());
    let mut session = Session::new(inst, true, None);
    assert_eq!(ok(&mut session, "get after"), "after: 43 [int]");
}

fn backup_dir(home: &Path) -> std::path::PathBuf {
    std::fs::read_dir(home.join("workspace-migration-backups")).unwrap().next().unwrap().unwrap().path()
}

#[test]
fn failed_verification_preserves_state_and_never_restores_any_files() {
    let dir = TempDir::new("migration_backup_integrity");
    prepare(&dir);
    let home = dir.path();
    migration::run(home, "apply").unwrap();
    let backup = backup_dir(home);
    let source = std::fs::read(home.join("data/store.kiva")).unwrap();
    std::fs::write(backup.join("00000000"), b"damaged").unwrap();
    assert!(matches!(migration::run(home, "verify"), Err(KivaError::Corrupted(_))));
    assert!(matches!(migration::run(home, "rollback"), Err(KivaError::Corrupted(_))));
    assert_eq!(migration::run(home, "status").unwrap(), "STAGED");
    assert_eq!(std::fs::read(home.join("data/store.kiva")).unwrap(), source);
}

#[cfg(feature = "crash-points")]
#[test]
fn interruption_at_each_publication_boundary_leaves_an_atomic_state() {
    use std::process::Command;

    for (action, point, expected) in [
        ("apply", "migration.before_publish", "NOT STARTED"),
        ("apply", "migration.after_publish", "STAGED"),
        ("rollback", "migration.before_rollback", "STAGED"),
        ("rollback", "migration.after_rollback", "ROLLED BACK"),
    ] {
        let dir = TempDir::new("migration_crash");
        prepare(&dir);
        if action == "rollback" {
            migration::run(dir.path(), "apply").unwrap();
        }
        let out = Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .args(["migrate", action, "--home"])
            .arg(dir.path())
            .env("KIVADB_CRASH_ARMED", "1")
            .env("KIVADB_CRASH_POINT", point)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{point}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("KIVADB-CRASH-POINT"), "{out:?}");
        assert_eq!(migration::run(dir.path(), "status").unwrap(), expected);
        if expected == "NOT STARTED" {
            migration::run(dir.path(), "apply").unwrap();
            assert_eq!(std::fs::read_dir(dir.path().join("workspace-migration-backups")).unwrap().count(), 2);
        } else {
            migration::run(dir.path(), "verify").unwrap();
        }
        migration::run(dir.path(), "rollback").unwrap();
        let inst = instance(&dir);
        let mut session = Session::new(inst, true, None);
        assert_eq!(ok(&mut session, "get before"), "before: 42 [int]");
    }
}

#[test]
fn preflight_refuses_busy_missing_and_physically_shared_stores() {
    use kivadb::engine::Engine;

    let dir = TempDir::new("migration_sources");
    prepare(&dir);
    {
        let inst = instance(&dir);
        let other = dir.path().join("custom");
        std::fs::create_dir(&other).unwrap();
        inst.cfg_mut().set("workspaces", "custom", &other.display().to_string()).unwrap();
        inst.shutdown();
    }
    let home = dir.path();
    assert!(migration::run(home, "apply").is_err());
    assert_eq!(migration::run(home, "status").unwrap(), "NOT STARTED");

    let store = home.join("custom/store.kiva");
    std::fs::hard_link(home.join("data/store.kiva"), &store).unwrap();
    assert!(migration::run(home, "plan").unwrap_err().to_string().contains("hard-linked legacy store"));
    std::fs::remove_file(&store).unwrap();

    let external = Engine::open(&store, Default::default()).unwrap();
    assert!(matches!(migration::run(home, "apply"), Err(KivaError::Locked(_))));
    drop(external);
    assert!(migration::run(home, "apply").unwrap().starts_with("STAGED"));
}

#[test]
fn backup_creation_failure_leaves_the_migration_unpublished() {
    let dir = TempDir::new("migration_backup_write_failure");
    prepare(&dir);
    let home = dir.path();
    std::fs::write(home.join("workspace-migration-backups"), b"existing file").unwrap();
    let original = std::fs::read(home.join("data/store.kiva")).unwrap();
    assert!(migration::run(home, "apply").is_err());
    assert_eq!(migration::run(home, "status").unwrap(), "NOT STARTED");
    assert_eq!(std::fs::read(home.join("workspace-migration-backups")).unwrap(), b"existing file");
    assert_eq!(std::fs::read(home.join("data/store.kiva")).unwrap(), original);
}

#[test]
fn physical_layout_export_preserves_sources_and_refuses_existing_destinations() {
    let dir = TempDir::new("migration_layout_source");
    let target = TempDir::new("migration_layout_target");
    let export = target.path().canonicalize().unwrap().join("layout");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "project create Shop collections (Orders)");
        ok(&mut s, "project create Empty");
        ok(&mut s, "use Shop.Orders");
        ok(&mut s, "set amount 42");
        drop(s);
        inst.shutdown();
    }
    let home = dir.path();
    let source = home.join("projects/Shop/Orders/store.kiva");
    let stored = std::fs::read(&source).unwrap();
    migration::run(home, "apply").unwrap();
    let plan = migration::run(home, "layout-plan").unwrap();
    let managed = Path::new("workspaces/default/projects/Shop/collections/Orders");
    assert!(plan.contains(&managed.display().to_string().replace('/', std::path::MAIN_SEPARATOR_STR)));
    assert!(migration::run_with_target(home, "copy-layout", Some(home)).is_err());
    migration::run_with_target(home, "copy-layout", Some(&export)).unwrap();
    let copied = export.join(managed).join("store.kiva");
    assert_eq!(std::fs::read(&copied).unwrap(), stored);
    assert_eq!(std::fs::read(&source).unwrap(), stored);
    assert!(!export.join("workspaces/default/store.kiva").exists());
    assert!(export.join("workspaces/default/projects/Empty").is_dir());
    assert!(!export.join("workspaces/default/projects/Empty/store.kiva").exists());
    assert!(migration::run_with_target(home, "copy-layout", Some(&export)).is_err());
    assert!(kivadb::instance::Instance::open(&export, Default::default()).is_err());
    migration::run(home, "rollback").unwrap();
    assert_eq!(std::fs::read(copied).unwrap(), stored);
}

#[cfg(unix)]
#[test]
fn layout_copy_refuses_a_destination_through_a_symlink() {
    let dir = TempDir::new("migration_layout_symlink_source");
    let target = TempDir::new("migration_layout_symlink_target");
    prepare(&dir);
    migration::run(dir.path(), "apply").unwrap();
    let link = target.path().canonicalize().unwrap().join("link");
    std::os::unix::fs::symlink(dir.path(), &link).unwrap();
    let result = migration::run_with_target(dir.path(), "copy-layout", Some(&link.join("layout")));
    assert!(result.unwrap_err().to_string().contains("non-directory parent"));
    assert!(!dir.path().join("layout").exists());
}

#[cfg(feature = "crash-points")]
#[test]
fn interrupted_layout_export_is_not_published_and_can_be_retried() {
    let dir = TempDir::new("migration_layout_interrupted");
    let parent = TempDir::new("migration_layout_retry");
    prepare(&dir);
    let original = std::fs::read(dir.path().join("data/store.kiva")).unwrap();
    migration::run(dir.path(), "apply").unwrap();
    let root = parent.path().canonicalize().unwrap();
    let export = root.join("layout");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .args(["migrate", "copy-layout", "--home"])
        .arg(dir.path())
        .arg("--target")
        .arg(&export)
        .env("KIVADB_CRASH_ARMED", "1")
        .env("KIVADB_CRASH_POINT", "migration.layout_after_collection")
        .output()
        .unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("KIVADB-CRASH-POINT"), "{out:?}");
    let marker = std::fs::read_to_string(export.join("workspace-layout-export.json")).unwrap();
    assert!(marker.contains("COPYING"));
    assert!(kivadb::instance::Instance::open(&export, Default::default()).is_err());
    assert!(migration::run_with_target(dir.path(), "copy-layout", Some(&export)).is_err());
    migration::run_with_target(dir.path(), "copy-layout", Some(&root.join("retry"))).unwrap();
    assert_eq!(std::fs::read(dir.path().join("data/store.kiva")).unwrap(), original);
}
