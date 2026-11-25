// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::client::Client;
use kivadb::session::Session;
use serde_json::json;

#[test]
fn navigation_uses_strict_relative_and_qualified_names() {
    let dir = TempDir::new("preview_navigation");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "set existing 42");
    ok(&mut s, "hierarchy on");
    assert_eq!(ok(&mut s, "workspace current"), "default");
    assert!(fails(&mut s, "project create missing.project").contains("invalid"));
    assert!(fails(&mut s, "collection create orphan").contains("select a project"));
    ok(&mut s, "workspace create fordima other");
    assert!(fails(&mut s, "workspace create another other").contains("already exists"));
    assert_eq!(ok(&mut s, "workspace exists another"), "false");
    ok(&mut s, "switch fordima");
    ok(&mut s, "project create efoma");
    ok(&mut s, "use efoma");
    ok(&mut s, "collection create users products orders");
    assert_eq!(ok(&mut s, "collection list"), "orders\nproducts\nusers");
    assert_eq!(ok(&mut s, "use collection users"), "Using fordima.efoma.users");
    assert_eq!(ok(&mut s, "use efoma.products"), "Using fordima.efoma.products");
    assert!(fails(&mut s, "use orders").contains("project 'orders'"));
    assert!(fails(&mut s, "use collection absent").contains("does not exist"));
    assert_eq!(ok(&mut s, "collection current"), "products");
    assert_eq!(ok(&mut s, "use collection fordima.efoma.orders"), "Using fordima.efoma.orders");
    ok(&mut s, "use efoma");
    assert_eq!(ok(&mut s, "collection current"), "none");
    ok(&mut s, "switch other");
    assert_eq!(ok(&mut s, "project current"), "none");
    ok(&mut s, "project create efoma");
    ok(&mut s, "use efoma");
    assert_eq!(ok(&mut s, "collection exists users"), "false");
    ok(&mut s, "collection create users");
    assert_eq!(ok(&mut s, "use collection users"), "Using other.efoma.users");
    assert!(fails(&mut s, "get existing").contains("metadata commands"));
    assert!(fails(&mut s, "set new 43").contains("metadata commands"));
    assert_eq!(inst.workspace_names(), ["default"]);
    ok(&mut s, "hierarchy off");
    assert_eq!(ok(&mut s, "get existing"), "existing: 42 [int]");
    ok(&mut s, "begin");
    assert!(fails(&mut s, "hierarchy on").contains("transaction"));
    ok(&mut s, "rollback");
}

#[test]
fn renames_and_drops_update_other_sessions_without_rebinding() {
    let dir = TempDir::new("preview_sessions");
    {
        let inst = instance(&dir);
        let mut a = Session::new(inst.clone(), true, None);
        let mut b = Session::new(inst.clone(), true, None);
        for s in [&mut a, &mut b] {
            ok(s, "hierarchy on");
        }
        assert!(fails(&mut a, "workspace rename default new").contains("default"));
        assert!(fails(&mut a, "workspace drop default cascade").contains("default"));
        ok(&mut a, "workspace create team");
        ok(&mut a, "switch team");
        ok(&mut a, "project create app");
        ok(&mut a, "use app");
        ok(&mut a, "collection create records");
        ok(&mut b, "use collection team.app.records");
        ok(&mut a, "workspace rename team group");
        ok(&mut a, "project rename app site");
        ok(&mut a, "collection rename records history");
        assert_eq!(ok(&mut b, "workspace current"), "group");
        assert_eq!(ok(&mut b, "project current"), "site");
        assert_eq!(ok(&mut b, "collection current"), "history");
        assert!(fails(&mut a, "workspace drop group").contains("cascade"));
        assert!(fails(&mut a, "project drop site").contains("cascade"));
        ok(&mut a, "collection drop history");
        ok(&mut a, "collection create history");
        assert_eq!(ok(&mut b, "collection current"), "none");
        ok(&mut a, "project drop site cascade");
        assert_eq!(ok(&mut b, "project current"), "none");
        ok(&mut a, "workspace drop group");
        assert_eq!(ok(&mut b, "workspace current"), "default");
        ok(&mut a, "workspace create saved");
        drop((a, b));
        inst.shutdown();
    }
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, "hierarchy on");
    assert_eq!(ok(&mut s, "workspace list"), "default\nsaved");
}

#[test]
fn binary_sessions_have_independent_contexts_and_no_data_access_in_preview() {
    let dir = TempDir::new("preview_tcp");
    let inst = instance(&dir);
    let mut local = Session::new(inst.clone(), true, None);
    ok(&mut local, "set old 42");
    ok(&mut local, "modify user kiva password admin-pw");
    ok(&mut local, "create user viewer identified by viewer-pw role viewer");
    ok(&mut local, "config set auth_required true");

    let port = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let mut first = Client::connect(("127.0.0.1", port)).unwrap();
    let mut second = Client::connect(("127.0.0.1", port)).unwrap();
    assert!(first.exec("hierarchy on").is_err());
    first.auth("viewer", "viewer-pw").unwrap();
    assert!(first.exec("hierarchy on").is_err());
    first.auth("kiva", "admin-pw").unwrap();
    second.auth("kiva", "admin-pw").unwrap();
    first.exec("hierarchy on").unwrap();
    second.exec("hierarchy on").unwrap();
    first.exec("workspace create team").unwrap();
    first.exec("switch team").unwrap();
    let a = first.api(&json!({"op":"whoami"})).unwrap();
    let b = second.api(&json!({"op":"whoami"})).unwrap();
    assert_eq!(a["hierarchy_path"], "team");
    assert_eq!(b["hierarchy_path"], "default");
    assert!(first.get("old").is_err());
    assert!(first.set("old", &kivadb::Value::Str("changed".into()), 0).is_err());
    assert!(first.del("old").is_err());
    assert!(first.api(&json!({"op":"scan"})).is_err());
    first.exec("hierarchy off").unwrap();
    assert!(first.get("old").unwrap().is_some());
}

#[test]
fn staged_migration_freezes_legacy_topology_but_preserves_legacy_data_access() {
    let dir = TempDir::new("preview_migration");
    {
        let inst = instance(&dir);
        let mut session = Session::new(inst.clone(), true, None);
        ok(&mut session, "set old 42");
        ok(&mut session, "hierarchy on");
        ok(&mut session, "workspace create scratch");
        drop(session);
        inst.shutdown();
    }
    kivadb::migration::run(dir.path(), "apply").unwrap();
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        assert_eq!(ok(&mut s, "get old"), "old: 42 [int]");
        ok(&mut s, "set new 43");
        assert!(fails(&mut s, "workspace create no").contains("frozen"));
        assert!(fails(&mut s, "project create no").contains("frozen"));
        assert!(fails(&mut s, "config set default_workspace unused").contains("frozen"));
        ok(&mut s, "hierarchy on");
        assert_eq!(ok(&mut s, "workspace exists scratch"), "false");
        assert_eq!(ok(&mut s, "use collection default.legacy.default"), "Using default.legacy.default");
        assert!(fails(&mut s, "collection drop default").contains("read-only"));
        drop(s);
        inst.shutdown();
    }
    kivadb::migration::run(dir.path(), "rollback").unwrap();
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    assert_eq!(ok(&mut s, "get new"), "new: 43 [int]");
    ok(&mut s, "hierarchy on");
    assert_eq!(ok(&mut s, "workspace exists scratch"), "true");
}

#[test]
fn the_cli_flag_enters_preview_before_executing_commands() {
    let dir = TempDir::new("preview_cli_flag");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .arg("--home")
        .arg(dir.path())
        .args(["--no-listen", "--hierarchy-preview", "-e", "workspace current"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).lines().any(|line| line == "default"), "{output:?}");
}

#[test]
fn concurrent_rename_and_drop_do_not_both_commit_against_the_same_name() {
    use std::sync::Barrier;

    let dir = TempDir::new("preview_concurrent");
    let inst = instance(&dir);
    let mut observer = Session::new(inst.clone(), true, None);
    ok(&mut observer, "hierarchy on");
    ok(&mut observer, "workspace create conflicting");
    let start = Barrier::new(2);
    let (rename, drop) = std::thread::scope(|scope| {
        let change = |command| {
            let mut s = Session::new(inst.clone(), true, None);
            ok(&mut s, "hierarchy on");
            start.wait();
            !s.execute(command).has_err()
        };
        let rename = scope.spawn(move || change("workspace rename conflicting renamed"));
        let drop = scope.spawn(move || change("workspace drop conflicting"));
        (rename.join().unwrap(), drop.join().unwrap())
    });
    assert_ne!(rename, drop);
    assert_eq!(ok(&mut observer, "workspace exists conflicting"), "false");
    assert_eq!(ok(&mut observer, "workspace exists renamed"), rename.to_string());
}
