// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Projects and collections in the shell before 2.0.0: `project current` and
//! `collection current`, `collection create` in the current project (several
//! at once, all or none), `drop` that unregisters and keeps the files,
//! `project drop ... cascade`, and `workspace` as in 1.x. The context is the
//! session's own.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::session::Session;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;

fn names(inst: &kivadb::instance::Instance) -> Vec<String> {
    let mut n = inst.workspace_names();
    n.sort();
    n
}

#[test]
fn qualified_navigation_uses_the_existing_collection_and_permissions() {
    let dir = TempDir::new("ctx_qualified");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "project create shop collections (orders)");
    ok(&mut admin, "project create other collections (orders)");
    ok(&mut admin, "use shop.orders");
    ok(&mut admin, "set order 42");
    ok(&mut admin, "use collection other.orders");
    for command in ["use collection shop.orders", "use default.shop.orders", "use collection default.shop.orders"] {
        ok(&mut admin, command);
        assert!(ok(&mut admin, "get order").contains("42"));
        assert_eq!(ok(&mut admin, "collection current"), "Current collection: shop.orders");
    }
    assert!(fails(&mut admin, "use collection unknown.shop.orders").contains("invalid collection path"));
    ok(&mut admin, r#"create user "dev" identified by "pw" role editor grant write on shop.*"#);
    ok(&mut admin, "config set auth_required true");
    let mut dev = Session::new(inst, false, None);
    ok(&mut dev, "auth dev pw");
    ok(&mut dev, "use collection default.shop.orders");
    assert!(fails(&mut dev, "use collection default.other.orders").contains("access"));
    assert_eq!(ok(&mut dev, "collection current"), "Current collection: shop.orders");
}

#[test]
fn current_says_where_the_session_is() {
    let dir = TempDir::new("ctx_current");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    assert_eq!(ok(&mut s, "project current"), "Current project: none");
    assert_eq!(ok(&mut s, "collection current"), "Current collection: default");
    ok(&mut s, "project create fordima");
    ok(&mut s, "project create shop");
    ok(&mut s, "use fordima");
    assert_eq!(ok(&mut s, "project current"), "Current project: fordima");
    assert_eq!(ok(&mut s, "collection current"), "Current collection: none");
    ok(&mut s, "collection create users");
    ok(&mut s, "use fordima.users");
    assert_eq!(ok(&mut s, "project current"), "Current project: fordima");
    assert_eq!(ok(&mut s, "collection current"), "Current collection: fordima.users");
    // Another collection, another project.
    ok(&mut s, "collection create orders");
    ok(&mut s, "use fordima.orders");
    assert_eq!(ok(&mut s, "collection current"), "Current collection: fordima.orders");
    ok(&mut s, "use shop");
    assert_eq!(ok(&mut s, "project current"), "Current project: shop");
    assert_eq!(ok(&mut s, "collection current"), "Current collection: none");
    // `workspace current` keeps the answer of 1.x.
    ok(&mut s, "use fordima.users");
    assert!(ok(&mut s, "workspace current").starts_with("fordima.users ("));
    ok(&mut s, "use default");
    assert!(ok(&mut s, "workspace current").starts_with("default ("));
}

#[test]
fn a_short_name_is_created_in_the_current_project_only() {
    let dir = TempDir::new("ctx_create");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    // No project: refused, with the ways out; nothing created.
    let e = fails(&mut s, "collection create users");
    assert!(e.contains("no current project") && e.contains("workspace create users"), "{e}");
    assert!(!inst.workspace_names().contains(&"users".to_string()));
    // A full name needs its project: none is made on the way.
    assert!(fails(&mut s, "collection create nope.users").contains("project 'nope' does not exist"));
    assert!(!inst.project_exists("nope"));

    ok(&mut s, "project create fordima");
    // The full name works from anywhere.
    assert!(ok(&mut s, "collection create fordima.places").starts_with("Collection 'fordima.places' created at "));
    ok(&mut s, "use fordima");
    let made = ok(&mut s, "collection create users");
    assert!(made.starts_with("Collection 'fordima.users' created at "), "{made}");
    let dir_of = inst.cfg().workspace_dir("fordima.users").unwrap();
    assert!(dir_of.ends_with(Path::new("projects").join("fordima").join("users")), "{}", dir_of.display());
    assert!(!inst.workspace_names().contains(&"users".to_string()), "never a collection outside the project");
    // Inside a collection of the project, the same.
    ok(&mut s, "use fordima.users");
    assert!(ok(&mut s, "collection create logs").starts_with("Collection 'fordima.logs'"));
}

#[test]
fn several_collections_are_created_all_or_none() {
    let dir = TempDir::new("ctx_many");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create fordima");
    ok(&mut s, "use fordima");
    let made = ok(&mut s, "collection create users products orders payments");
    assert_eq!(made.lines().count(), 4, "{made}");
    for c in ["users", "products", "orders", "payments"] {
        assert!(made.contains(&format!("Collection 'fordima.{c}' created at ")), "{made}");
    }
    let before = names(&inst);
    // One wrong name, and none is made.
    assert!(fails(&mut s, "collection create a b users").contains("'fordima.users' already exists"));
    assert!(fails(&mut s, "collection create a b A").contains("named twice"));
    assert!(fails(&mut s, "collection create a b c.d.e").contains("collection name 'c.d.e'"));
    assert!(fails(&mut s, "collection create a nope.b").contains("project 'nope' does not exist"));
    assert_eq!(names(&inst), before);
    // A folder goes with one collection; quoted, it is a folder, bare, a name.
    assert!(fails(&mut s, r#"collection create a b "somewhere""#).contains("one collection at a time"));
    let folder = dir.path().join("elsewhere");
    let made = ok(&mut s, &format!("collection create a \"{}\"", folder.display()));
    assert!(made.starts_with("Collection 'fordima.a' created at "), "{made}");
    assert_eq!(inst.cfg().workspace_dir("fordima.a").unwrap(), folder);
    let made = ok(&mut s, &format!("collection create q '{}'", dir.path().join("q2").display()));
    assert!(made.starts_with("Collection 'fordima.q' created at "), "{made}");
    // A quoted name alone is a name, as before.
    assert!(ok(&mut s, r#"collection create "b""#).starts_with("Collection 'fordima.b'"));
}

#[test]
fn an_editor_creates_only_in_the_projects_they_manage_and_nothing_else() {
    let dir = TempDir::new("ctx_grants");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "project create fordima");
    ok(&mut admin, "project create other");
    ok(&mut admin, "collection create fordima.users");
    ok(&mut admin, r#"create user "dev" identified by "pw" role editor grant write on fordima.*"#);
    ok(&mut admin, "config set auth_required true");
    let mut s = Session::new(inst.clone(), false, None);
    ok(&mut s, "auth dev pw");
    ok(&mut s, "use fordima");
    let before = names(&inst);
    assert!(fails(&mut s, "collection create a other.b").contains("project 'other'"));
    assert_eq!(names(&inst), before, "one refused, none made");
    assert!(fails(&mut s, &format!("collection create c \"{}\"", dir.path().join("x").display()))
        .contains("only an administrator chooses the folder"));
    ok(&mut s, "collection create a b");
    assert!(inst.workspace_names().contains(&"fordima.b".to_string()));
    // Dropping stays with administrators.
    assert!(fails(&mut s, "collection drop a").contains("admin"));
    assert!(fails(&mut s, "project drop fordima cascade").contains("admin"));
}

#[test]
fn drop_unregisters_the_collection_of_the_project_and_keeps_its_files() {
    let dir = TempDir::new("ctx_drop");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create fordima");
    ok(&mut s, "project create baak");
    // A collection outside a project with the same short name.
    ok(&mut s, "workspace create places");
    ok(&mut s, "use places");
    ok(&mut s, "set outside 1");
    ok(&mut s, "collection create fordima.places baak.places");
    ok(&mut s, "use fordima.places");
    ok(&mut s, "set inside 1");
    let files = inst.cfg().workspace_dir("fordima.places").unwrap();

    ok(&mut s, "use fordima");
    let out = ok(&mut s, "collection drop places");
    assert_eq!(out, "Collection 'fordima.places' unregistered. Its data files were left on disk.");
    assert!(inst.workspace_names().contains(&"places".to_string()), "never the one outside the project");
    assert!(inst.workspace_names().contains(&"baak.places".to_string()), "never the one of another project");
    assert!(files.join("store.kiva").exists(), "the files stay");
    ok(&mut s, "use places");
    assert!(ok(&mut s, "get outside").contains('1'));
    // A full name from anywhere; a short one outside a project, as before.
    ok(&mut s, "use default");
    ok(&mut s, "collection drop baak.places");
    ok(&mut s, "collection drop places");
    assert!(!inst.workspace_names().iter().any(|w| w.ends_with("places")));
    // `remove` is the same command.
    ok(&mut s, "collection create fordima.x");
    ok(&mut s, "use fordima");
    ok(&mut s, "collection remove x");
    assert!(fails(&mut s, "collection drop x").contains("'fordima.x' does not exist"));
    // The collection in use is not dropped.
    ok(&mut s, "collection create y");
    ok(&mut s, "use fordima.y");
    assert!(fails(&mut s, "collection drop y").contains("you cannot remove the collection you are in"));
    // And the files of a dropped collection come back with its folder.
    ok(&mut s, "use default");
    ok(&mut s, &format!("collection create fordima.places \"{}\"", files.display()));
    ok(&mut s, "use fordima.places");
    assert!(ok(&mut s, "get inside").contains('1'));
}

#[test]
fn a_project_with_collections_is_dropped_only_with_cascade() {
    let dir = TempDir::new("ctx_project_drop");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak collections (places, users)");
    ok(&mut s, "use baak.places");
    ok(&mut s, "set k 1");
    let files = inst.cfg().workspace_dir("baak.places").unwrap();
    let e = fails(&mut s, "project drop baak");
    assert!(e.contains("still holds baak.places, baak.users") && e.contains("project drop baak cascade"), "{e}");
    // Not from inside one of them.
    assert!(fails(&mut s, "project drop baak cascade").contains("you are in baak.places"));
    ok(&mut s, "use baak");
    let out = ok(&mut s, "project drop baak cascade");
    assert!(out.contains("2 collection(s) unregistered: baak.places, baak.users"), "{out}");
    assert!(!inst.project_exists("baak"));
    assert!(files.join("store.kiva").exists(), "the files stay");
    // The project dropped was the current one: none now.
    assert_eq!(ok(&mut s, "project current"), "Current project: none");
    assert!(fails(&mut s, "project drop baak").contains("does not exist"));
    // An empty project needs no cascade; `remove` is the same command.
    ok(&mut s, "project create empty");
    assert_eq!(ok(&mut s, "project drop empty"), "Project 'empty' removed.");
    ok(&mut s, "project create empty2");
    ok(&mut s, "project remove empty2");
}

#[test]
fn workspace_keeps_the_commands_of_1x() {
    let dir = TempDir::new("ctx_workspace");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    // Outside a project, in the folder of rc.5.
    let made = ok(&mut s, "workspace create legacy");
    assert!(made.starts_with("Workspace 'legacy' created at "), "{made}");
    assert!(inst.cfg().workspace_dir("legacy").unwrap().ends_with(Path::new("collections").join("legacy")));
    // Its second word is still a folder, quoted or not.
    let folder = dir.path().join("w2");
    ok(&mut s, &format!("workspace create w2 \"{}\"", folder.display()));
    assert_eq!(inst.cfg().workspace_dir("w2").unwrap(), folder);
    ok(&mut s, "workspace switch legacy");
    assert!(ok(&mut s, "workspace current").starts_with("legacy ("));
    ok(&mut s, "use default");
    // In a project too, `workspace create` takes the name as written.
    ok(&mut s, "project create p");
    ok(&mut s, "use p");
    ok(&mut s, "workspace create plain");
    assert!(inst.workspace_names().contains(&"plain".to_string()));
    ok(&mut s, "workspace remove plain");
    ok(&mut s, "workspace remove legacy");
    ok(&mut s, "use default");
    assert!(fails(&mut s, "workspace drop w2").contains("Usage: workspace"));
}

#[test]
fn each_session_has_its_own_context() {
    let dir = TempDir::new("ctx_sessions");
    let inst = instance(&dir);
    let mut a = Session::new(inst.clone(), true, None);
    let mut b = Session::new(inst.clone(), true, None);
    ok(&mut a, "project create one");
    ok(&mut a, "project create two");
    ok(&mut a, "use one");
    ok(&mut b, "use two");
    ok(&mut a, "collection create c");
    ok(&mut b, "collection create c");
    assert_eq!(ok(&mut a, "project current"), "Current project: one");
    assert_eq!(ok(&mut b, "project current"), "Current project: two");
    assert!(inst.workspace_names().contains(&"one.c".to_string()));
    assert!(inst.workspace_names().contains(&"two.c".to_string()));
    // A new session starts in default, with no project: nothing is kept.
    let mut again = Session::new(inst.clone(), true, None);
    assert_eq!(ok(&mut again, "project current"), "Current project: none");
    assert_eq!(ok(&mut again, "collection current"), "Current collection: default");

    // Over TCP, two connections, two contexts.
    let port = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let ask = |conn: &mut (TcpStream, BufReader<TcpStream>), cmd: &str| -> String {
        conn.0.write_all(format!("{cmd}\n").as_bytes()).unwrap();
        let mut line = String::new();
        conn.1.read_line(&mut line).unwrap();
        line.trim_end().to_string()
    };
    let open = || {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let r = BufReader::new(s.try_clone().unwrap());
        (s, r)
    };
    let (mut x, mut y) = (open(), open());
    ask(&mut x, "use one.c");
    ask(&mut y, "use two");
    assert_eq!(ask(&mut x, "collection current"), "Current collection: one.c");
    assert_eq!(ask(&mut y, "project current"), "Current project: two");
    assert_eq!(ask(&mut y, "collection current"), "Current collection: none");
    assert_eq!(ask(&mut x, "project current"), "Current project: one");
}

#[test]
fn the_command_line_keeps_the_context_from_one_e_to_the_next() {
    let dir = TempDir::new("ctx_cli");
    let folder = dir.path().join("own folder");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .arg("--home")
        .arg(dir.path())
        .arg("--no-listen")
        .args(["-e", "project create fordima", "-e", "use fordima", "-e", "collection create users products"])
        .args(["-e", &format!("collection create logs \"{}\"", folder.display())])
        .args(["-e", "use fordima.users", "-e", "project current", "-e", "collection current"])
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("Collection 'fordima.users' created at "), "{text}");
    assert!(text.contains("Collection 'fordima.products' created at "), "{text}");
    assert!(text.contains(&format!("Collection 'fordima.logs' created at {}.", folder.display())), "{text}");
    assert!(text.ends_with("Current project: fordima\nCurrent collection: fordima.users\n"), "{text}");
}

#[test]
fn a_creation_of_several_stopped_in_the_middle_is_finished_at_the_next_start() {
    let dir = TempDir::new("ctx_pending");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "project create fordima");
        ok(&mut s, "collection create fordima.users");
        // What a stop after the first of three leaves: the record, one made.
        inst.system
            .put(
                "pending:collections:00000000000000aa",
                &kivadb::value::Value::Doc(serde_json::json!({
                    "collections": ["fordima.users", "fordima.products", "fordima.orders"]
                })),
                0,
                false,
            )
            .unwrap();
        drop(s);
        inst.shutdown();
    }
    let inst = instance(&dir);
    for c in ["fordima.users", "fordima.products", "fordima.orders"] {
        assert!(inst.workspace_names().contains(&c.to_string()), "{c}");
    }
    assert!(inst.system.keys_matching("pending:collections:*", 0).is_empty());
}
