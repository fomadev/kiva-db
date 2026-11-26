// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

mod common;
use common::{fails, instance, ok, TempDir};
use kivadb::session::Session;
use serde_json::json;

fn provision(admin: &mut Session) {
    ok(admin, "create user operator identified by test-pass role editor");
    ok(admin, "create user auditor identified by test-pass role viewer");
    ok(admin, "hierarchy storage");
    ok(admin, "workspace create Fordima Malanda");
    for workspace in ["Fordima", "Malanda"] {
        ok(admin, &format!("switch {workspace}"));
        ok(admin, "project create Efoma Fomapay");
        for project in ["Efoma", "Fomapay"] {
            ok(admin, &format!("use {project}"));
            ok(admin, "collection create users products");
        }
    }
}

fn login(inst: std::sync::Arc<kivadb::instance::Instance>, name: &str) -> Session {
    let mut user = Session::new(inst, true, None);
    user.login(name, "test-pass").unwrap();
    ok(&mut user, "hierarchy storage");
    user
}

#[test]
fn direct_inherited_and_administrative_grants_are_distinct() {
    let dir = TempDir::new("grants_scopes");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    provision(&mut admin);
    let mut user = login(inst.clone(), "operator");
    assert!(ok(&mut user, "workspace list").is_empty());
    let hidden = fails(&mut user, "use collection Fordima.Efoma.users");
    assert_eq!(hidden, fails(&mut user, "use collection Missing.Unknown.users"));

    ok(&mut admin, "hierarchy grant operator read collection Fordima.Efoma.users");
    assert_eq!(ok(&mut user, "workspace list"), "Fordima");
    ok(&mut user, "switch Fordima");
    assert_eq!(ok(&mut user, "project list"), "Efoma");
    ok(&mut user, "use Efoma");
    assert_eq!(ok(&mut user, "collection list"), "users");
    ok(&mut user, "use collection users");
    assert_eq!(ok(&mut user, "hierarchy permissions"), "read=true write=false manage=false");
    assert_eq!(fails(&mut user, "use collection Fordima.Efoma.products"), hidden);
    assert_eq!(fails(&mut user, "use collection Fordima.Fomapay.users"), hidden);
    assert_eq!(fails(&mut user, "use collection Malanda.Efoma.users"), hidden);
    assert!(fails(&mut user, "workspace list --details").contains("unavailable"));
    assert!(fails(&mut user, "collection drop users").contains("unavailable"));
    assert!(fails(&mut user, "hierarchy grant operator manage instance").contains("unavailable"));

    ok(&mut admin, "hierarchy grant operator write project Fordima.Efoma");
    ok(&mut user, "use collection products");
    assert_eq!(ok(&mut user, "hierarchy permissions"), "read=true write=true manage=false");
    assert!(fails(&mut user, "set secret 1").contains("metadata commands"));
    assert!(kivadb::api::handle(&mut user, &json!({"op":"scan"})).is_err());
    assert!(fails(&mut user, "collection create rejected").contains("unavailable"));
    ok(&mut admin, "hierarchy grant operator manage project Fordima.Efoma");
    ok(&mut user, "collection create allowed");
    assert!(fails(&mut user, "project create denied").contains("unavailable"));

    ok(&mut admin, "hierarchy grant auditor write workspace Fordima");
    let mut viewer = login(inst, "auditor");
    ok(&mut viewer, "use collection Fordima.Fomapay.users");
    assert_eq!(ok(&mut viewer, "hierarchy permissions"), "read=true write=false manage=false");
    ok(&mut admin, "hierarchy grant auditor read instance");
    assert_eq!(ok(&mut viewer, "workspace list"), "Fordima\nMalanda\ndefault");
}

#[test]
fn rename_revocation_retirement_and_account_reuse_do_not_rebind_access() {
    let dir = TempDir::new("grants_revoke");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    provision(&mut admin);
    ok(&mut admin, "hierarchy grant operator read collection Fordima.Efoma.users");
    let mut first = login(inst.clone(), "operator");
    let mut second = login(inst.clone(), "operator");
    ok(&mut first, "use collection Fordima.Efoma.users");
    ok(&mut second, "use collection Fordima.Efoma.users");

    ok(&mut admin, "switch Fordima");
    ok(&mut admin, "use Efoma");
    ok(&mut admin, "collection rename users to members");
    assert_eq!(ok(&mut first, "collection current"), "members");
    ok(&mut admin, "hierarchy revoke operator read collection Fordima.Efoma.members");
    assert_eq!(ok(&mut first, "collection current"), "none");
    let state = kivadb::api::handle(&mut second, &json!({"op":"whoami"})).unwrap();
    assert!(state["workspace"].is_null());
    assert!(state["collection"].is_null());

    ok(&mut admin, "hierarchy grant operator read collection Fordima.Efoma.members");
    ok(&mut admin, "collection drop members");
    ok(&mut admin, "collection create members");
    assert!(fails(&mut first, "use collection Fordima.Efoma.members").contains("unavailable"));
    ok(&mut admin, "hierarchy grant operator read collection Fordima.Efoma.members");
    ok(&mut admin, "hierarchy off");
    ok(&mut admin, "drop user operator");
    ok(&mut admin, "create user operator identified by test-pass role editor");
    assert!(fails(&mut first, "workspace list").contains("unavailable"));
    let mut replacement = login(inst, "operator");
    assert!(ok(&mut replacement, "workspace list").is_empty());
}

#[test]
fn staged_grants_use_exact_legacy_aliases_without_widening_scope() {
    let dir = TempDir::new("grants_legacy");
    {
        let inst = instance(&dir);
        let mut admin = Session::new(inst.clone(), true, None);
        ok(&mut admin, "project create Efoma collections (users, products)");
        ok(&mut admin, "project create Fomapay collections (users)");
        for name in ["Efoma.users", "Efoma.products", "Fomapay.users"] {
            inst.engine(name).unwrap();
        }
        ok(&mut admin, "workspace create lab");
        ok(&mut admin, "workspace switch lab");
        ok(&mut admin, "set staged 1");
        ok(&mut admin, "create user operator identified by test-pass role editor");
        ok(&mut admin, "grant read on Efoma.* to operator");
        ok(&mut admin, "grant write on lab to operator");
        let user = kivadb::auth::get_user(&inst.system, "operator").unwrap().unwrap();
        use kivadb::auth::Priv;
        assert_eq!(user.access("Efoma.users"), Some(Priv::Read));
        assert_eq!(user.access("Efoma.products"), Some(Priv::Read));
        assert_eq!(user.access("Fomapay.users"), None);
        assert_eq!(user.access("default"), None);
        assert_eq!(user.access("lab"), Some(Priv::Write));
        drop(admin);
        inst.shutdown();
    }
    kivadb::migration::run(dir.path(), "apply").unwrap();
    let inst = instance(&dir);
    let mut user = Session::new(inst, true, None);
    user.login("operator", "test-pass").unwrap();
    ok(&mut user, "hierarchy on");
    ok(&mut user, "use collection default.Efoma.users");
    assert_eq!(ok(&mut user, "hierarchy permissions"), "read=true write=false manage=false");
    ok(&mut user, "use collection default.Efoma.products");
    assert!(fails(&mut user, "use collection default.Fomapay.users").contains("unavailable"));
    assert!(fails(&mut user, "use collection default.legacy.default").contains("unavailable"));
    ok(&mut user, "use collection default.legacy.lab");
    assert_eq!(ok(&mut user, "hierarchy permissions"), "read=true write=true manage=false");
    assert!(fails(&mut user, "collection create denied").contains("read-only"));
}

#[test]
fn unsupported_or_corrupt_policies_prevent_reopen() {
    for policy in [json!({"version":2,"grants":[]}), json!({"version":1,"grants":[{"subject":"bad"}]})] {
        let dir = TempDir::new("grants_corrupt");
        {
            let inst = instance(&dir);
            inst.system.put("catalog:hierarchy-grants", &kivadb::Value::Doc(policy), 0, false).unwrap();
            inst.system.sync().unwrap();
            inst.shutdown();
        }
        let error =
            kivadb::instance::Instance::open(dir.path(), Default::default()).err().expect("invalid policy loaded");
        assert!(error.to_string().contains("hierarchy grants"));
    }
}

#[test]
fn revocation_serializes_with_concurrent_metadata_commands() {
    use std::sync::Barrier;

    let dir = TempDir::new("grants_parallel");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    provision(&mut admin);
    ok(&mut admin, "hierarchy grant operator manage project Fordima.Efoma");
    let mut editor = login(inst.clone(), "operator");
    ok(&mut editor, "switch Fordima");
    ok(&mut editor, "use Efoma");
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            // This can linearize before or after the revocation.
            let _ = editor.execute("collection create racing");
        });
        barrier.wait();
        ok(&mut admin, "hierarchy revoke operator manage project Fordima.Efoma");
    });
    assert!(fails(&mut editor, "collection create after").contains("unavailable"));
    assert!(ok(&mut editor, "collection list").is_empty());
    drop((admin, editor));
    inst.shutdown();
    drop(inst);
    let reopened = instance(&dir);
    let mut editor = login(reopened, "operator");
    assert!(ok(&mut editor, "workspace list").is_empty());
}

#[test]
fn cascading_retirement_does_not_transfer_direct_grants() {
    let dir = TempDir::new("grants_cascade");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    provision(&mut admin);
    ok(&mut admin, "hierarchy grant operator read project Fordima.Efoma");
    let mut editor = login(inst, "operator");
    ok(&mut editor, "use collection Fordima.Efoma.users");
    ok(&mut admin, "workspace drop Fordima cascade");
    ok(&mut admin, "workspace create Fordima");
    ok(&mut admin, "switch Fordima");
    ok(&mut admin, "project create Efoma");
    ok(&mut admin, "use Efoma");
    ok(&mut admin, "collection create users");
    assert!(ok(&mut editor, "workspace list").is_empty());
    assert!(fails(&mut editor, "use collection Fordima.Efoma.users").contains("unavailable"));
}

#[cfg(feature = "crash-points")]
#[test]
fn grant_recovery_distinguishes_precommit_and_committed_revocations() {
    use std::process::Command;
    for (point, visible) in [("hierarchy_grants.before_commit", true), ("hierarchy_grants.after_commit", false)] {
        let dir = TempDir::new("grants_crash");
        {
            let inst = instance(&dir);
            let mut admin = Session::new(inst.clone(), true, None);
            provision(&mut admin);
            ok(&mut admin, "hierarchy grant operator read collection Fordima.Efoma.users");
            drop(admin);
            inst.shutdown();
        }
        let out = Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .args(["--home"])
            .arg(dir.path())
            .args([
                "--no-listen",
                "-e",
                "hierarchy storage",
                "-e",
                "hierarchy revoke operator read collection Fordima.Efoma.users",
            ])
            .env("KIVADB_CRASH_ARMED", "1")
            .env("KIVADB_CRASH_POINT", point)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{point}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("KIVADB-CRASH-POINT"), "{point}: {out:?}");
        let mut editor = login(instance(&dir), "operator");
        assert_eq!(!editor.execute("use collection Fordima.Efoma.users").has_err(), visible, "{point}");
    }
}
