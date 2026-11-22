// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot B of docs/architecture/RC5_ARCHITECTURE.md (sections 2 and 3): a
//! collection is what a workspace is, `collection ...` its commands next to
//! `workspace ...`, `use` enters a collection or selects a project, and
//! `project create <p> collections (...)` makes them together, finished after
//! a crash.

mod common;

use common::{fails, instance, ok, run, TempDir};
use kivadb::session::Session;
use serde_json::json;

#[test]
fn collections_are_workspaces_under_their_new_name() {
    let dir = TempDir::new("collections_alias");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak");
    let made = ok(&mut s, "collection create baak.places");
    assert!(made.starts_with("Collection 'baak.places' created at "), "{made}");
    // The old form keeps its words.
    assert!(ok(&mut s, "workspace create baak.cache").starts_with("Workspace 'baak.cache' created at "));
    // One list, under both names; `collection` alone lists too.
    let list = ok(&mut s, "collection list baak");
    assert!(list.contains("baak.places") && list.contains("baak.cache") && !list.contains("default"), "{list}");
    assert_eq!(ok(&mut s, "workspace list baak"), list);
    assert!(ok(&mut s, "collection").contains("default"));

    assert!(ok(&mut s, "collection switch baak.places").contains("Switched to collection 'baak.places'"));
    ok(&mut s, r#"set city:1 {"name": "Goma"}"#);
    assert!(ok(&mut s, "workspace current").starts_with("baak.places ("));
    ok(&mut s, "use default");
    assert!(!run(&mut s, "get city:1").contains("Goma"), "each collection holds its own keys");

    // rename and move are one crash-safe mechanism; the data follows.
    let renamed = ok(&mut s, "collection rename baak.places to baak.towns");
    assert!(renamed.starts_with("Collection 'baak.places' is now 'baak.towns'"), "{renamed}");
    ok(&mut s, "project create dzoket");
    ok(&mut s, "workspace move baak.towns to dzoket.towns");
    ok(&mut s, "use dzoket.towns");
    assert!(ok(&mut s, "get city:1").contains("Goma"));
    ok(&mut s, "use default");
    assert!(ok(&mut s, "collection remove dzoket.towns").contains("Collection 'dzoket.towns' unregistered"));
    assert!(fails(&mut s, "collection remove dzoket.towns").contains("collection 'dzoket.towns' does not exist"));
    assert!(fails(&mut s, "collection remove default").contains("the default collection cannot be removed"));
    assert!(fails(&mut s, "collection frobnicate").contains("Usage: collection list"));

    // The structured API answers both names, with the same value.
    let who = kivadb::api::handle(&mut s, &json!({"op": "whoami"})).unwrap();
    assert_eq!(who["workspace"], "default");
    assert_eq!(who["collection"], "default");
    let all = kivadb::api::handle(&mut s, &json!({"op": "collections", "project": "baak"})).unwrap();
    let names: Vec<&str> = all["collections"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["baak.cache"]);
    let old = kivadb::api::handle(&mut s, &json!({"op": "workspaces"})).unwrap();
    assert!(old["workspaces"].as_array().unwrap().len() >= 2);
}

#[test]
fn use_enters_a_collection_or_selects_a_project() {
    let dir = TempDir::new("collections_use");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak");
    ok(&mut s, "collection create baak.places");
    ok(&mut s, "collection create baak.users");
    ok(&mut s, "collection create lab");

    // A project: no collection in use, data commands ask for one.
    let used = ok(&mut s, "use baak");
    assert!(
        used.contains("Using project 'baak' (2 collection(s))") && used.contains("use baak.<collection>"),
        "{used}"
    );
    let err = fails(&mut s, "get k");
    assert!(
        err.contains("you are in project 'baak' but in no collection") && err.contains("use baak.<collection>"),
        "{err}"
    );
    assert!(fails(&mut s, "set k 1").contains("in no collection"));
    assert_eq!(ok(&mut s, "whoami"), "kiva (admin) @ project 'baak', in no collection");
    assert_eq!(ok(&mut s, "collection current"), "(project 'baak', no collection)");
    // The list is the project's.
    let list = ok(&mut s, "collection list");
    assert!(list.contains("baak.places") && list.contains("baak.users") && !list.contains("lab"), "{list}");
    let who = kivadb::api::handle(&mut s, &json!({"op": "whoami"})).unwrap();
    assert_eq!(who["project"], "baak");
    assert!(who["collection"].is_null());

    // A collection: by its full name, or outside a project by its own.
    assert!(ok(&mut s, "use baak.places").starts_with("Using collection 'baak.places' (0 keys)"));
    ok(&mut s, "set k 1");
    let who = kivadb::api::handle(&mut s, &json!({"op": "whoami"})).unwrap();
    assert_eq!((who["project"].as_str(), who["collection"].as_str()), (Some("baak"), Some("baak.places")));
    ok(&mut s, "use lab");
    assert_eq!(kivadb::api::handle(&mut s, &json!({"op": "whoami"})).unwrap()["project"], json!(null));
    assert!(fails(&mut s, "use nowhere").contains("no collection or project named 'nowhere'"));
    assert!(fails(&mut s, "use baak.nothing").contains("does not exist"));
    assert!(fails(&mut s, "use").contains("Usage: use"));
    // Not inside a transaction.
    ok(&mut s, "begin");
    assert!(fails(&mut s, "use baak").contains("inside a transaction"));
    ok(&mut s, "rollback");
}

#[test]
fn use_respects_grants() {
    let dir = TempDir::new("collections_grants");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "project create baak");
    ok(&mut admin, "project create other");
    ok(&mut admin, "collection create baak.places");
    ok(&mut admin, "collection create other.secret");
    ok(&mut admin, r#"create user "analyst" identified by "pw" grant read on baak.*"#);
    ok(&mut admin, "config set auth_required true");

    let mut s = Session::new(inst.clone(), false, None);
    ok(&mut s, "auth analyst pw");
    assert!(ok(&mut s, "use baak").contains("Using project 'baak'"));
    assert!(fails(&mut s, "use other").contains("no access to project 'other'"));
    assert!(fails(&mut s, "use other.secret").contains("no access"));
    ok(&mut s, "use baak.places");
    assert!(fails(&mut s, "set k 1").contains("read access"));
    // Only administrators create a project, or rename and remove collections.
    assert!(fails(&mut s, "project create mine collections (a)").contains("admin role"));
    assert!(fails(&mut s, "collection rename baak.places to baak.p").contains("admin role"));
}

#[test]
fn a_project_is_created_with_its_collections_and_finished_after_a_crash() {
    let dir = TempDir::new("collections_project");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        let made = ok(&mut s, "project create baak collections (cache, places, users)");
        assert!(made.starts_with("Project 'baak' created with 3 collection(s):"), "{made}");
        for c in ["baak.cache", "baak.places", "baak.users"] {
            assert!(made.contains(c), "{made}");
        }
        assert!(ok(&mut s, "project list").contains("3 collection(s): cache, places, users"));
        ok(&mut s, "use baak.users");
        ok(&mut s, "set u:1 1");

        // Every name is checked before anything is made.
        for (bad, why) in [
            ("project create baak collections (x)", "already exists"),
            ("project create p2 collections (a, A)", "named twice"),
            ("project create p2 collections (a.b)", "only letters"),
            ("project create p2 collections ()", "Usage: project"),
            ("project create p2 collections (a b)", "Usage: project"),
            ("project create p2 collections a, b", "Usage: project"),
        ] {
            assert!(fails(&mut s, bad).contains(why), "{bad}: {}", fails(&mut s, bad));
        }
        assert!(!inst.project_exists("p2"), "nothing made by a refused command");

        // A crash after the record and the project, before the collections.
        inst.system
            .put(
                "pending:project:shop",
                &kivadb::value::Value::Doc(json!({ "collections": ["orders", "items"] })),
                0,
                false,
            )
            .unwrap();
        inst.create_project("shop").unwrap();
        inst.system.sync().unwrap();
    }
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let list = ok(&mut s, "collection list shop");
    assert!(list.contains("shop.orders") && list.contains("shop.items"), "{list}");
    assert!(inst.system.keys_matching("pending:*", 0).is_empty(), "the record is gone");
    // What existed is untouched.
    ok(&mut s, "use baak.users");
    assert!(ok(&mut s, "get u:1").contains('1'));
}

#[test]
fn use_finds_a_collection_of_the_current_project_last() {
    let dir = TempDir::new("collections_use_short");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak collections (cache, places, users, lab, other)");
    ok(&mut s, "collection create lab");
    ok(&mut s, "project create other");
    ok(&mut s, "use baak.places");
    ok(&mut s, r#"set city:1 "Kinshasa""#);

    // After `use baak`, a collection of the project by its short name.
    ok(&mut s, "use baak");
    assert!(ok(&mut s, "use cache").starts_with("Using collection 'baak.cache' (0 keys)."));
    // From a collection of the project too.
    assert!(ok(&mut s, "use places").starts_with("Using collection 'baak.places' (1 keys)."));
    assert_eq!(ok(&mut s, "get city:1"), "city:1: Kinshasa [string]");
    assert_eq!(kivadb::api::handle(&mut s, &json!({"op": "whoami"})).unwrap()["collection"], "baak.places");
    assert!(ok(&mut s, "use users").starts_with("Using collection 'baak.users'"));

    // What worked before keeps its meaning: a collection outside a project
    // first, then a project, each saying that the project has one too.
    let lab = ok(&mut s, "use lab");
    assert!(
        lab.starts_with("Using collection 'lab' (0 keys).") && lab.contains("baak.lab also exists: use baak.lab"),
        "{lab}"
    );
    ok(&mut s, "use baak");
    let other = ok(&mut s, "use other");
    assert!(other.starts_with("Using project 'other'") && other.contains("baak.other also exists"), "{other}");
    ok(&mut s, "use baak");
    assert!(ok(&mut s, "use default").starts_with("Using collection 'default'"));
    assert!(ok(&mut s, "use baak.lab").starts_with("Using collection 'baak.lab'"));

    // Out of a project, nothing new; inside, the error names both.
    ok(&mut s, "use default");
    assert!(fails(&mut s, "use places").contains("no collection or project named 'places' ("));
    ok(&mut s, "use baak");
    let err = fails(&mut s, "use nothing");
    assert!(err.contains("named 'nothing', nor a collection 'baak.nothing' in the project 'baak'"), "{err}");
    // A full name never looks into the project; neither does `collection switch`.
    assert!(fails(&mut s, "use baak.baak.places").contains("does not exist"));
    assert!(fails(&mut s, "collection switch places").contains("does not exist"));
    assert!(fails(&mut s, "workspace switch places").contains("does not exist"));
    // Not inside a transaction.
    ok(&mut s, "use baak.cache");
    ok(&mut s, "begin");
    assert!(fails(&mut s, "use places").contains("inside a transaction"));
    ok(&mut s, "rollback");
}

#[test]
fn use_by_a_short_name_asks_the_same_grants() {
    let dir = TempDir::new("collections_use_short_grants");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "project create baak collections (cache, places)");
    ok(&mut admin, r#"create user "analyst" identified by "pw" grant read on baak.places"#);
    ok(&mut admin, "config set auth_required true");

    let mut s = Session::new(inst.clone(), false, None);
    ok(&mut s, "auth analyst pw");
    ok(&mut s, "use baak");
    assert!(ok(&mut s, "use places").starts_with("Using collection 'baak.places'"));
    assert!(fails(&mut s, "set k 1").contains("read access"));
    let short = fails(&mut s, "use cache");
    let full = fails(&mut s, "use baak.cache");
    assert_eq!(short, full, "the same refusal as the full name");
    assert!(short.contains("no access"), "{short}");
}

#[test]
fn a_refused_short_name_points_to_the_project_and_hides_the_rest() {
    let dir = TempDir::new("collections_use_hint");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "project create baak collections (cache, places, users)");
    // A project and a collection outside a project, named like collections of baak.
    ok(&mut admin, "project create places collections (demo)");
    ok(&mut admin, "project create cache collections (demo)");
    ok(&mut admin, "collection create users");
    ok(&mut admin, r#"create user "analyst" identified by "pw" grant read on baak.places"#);
    ok(&mut admin, r#"create user "u3" identified by "pw" grant read on users"#);
    ok(&mut admin, "grant read on baak.places to u3");
    ok(&mut admin, "config set auth_required true");

    let mut s = Session::new(inst.clone(), false, None);
    ok(&mut s, "auth analyst pw");
    ok(&mut s, "use baak");
    // The order stays: the project `places` comes first and is refused, with
    // the way to the collection of the project the user may enter.
    let err = fails(&mut s, "use places");
    assert!(err.contains("no access to project 'places'"), "{err}");
    assert!(err.contains("baak.places is in your project: use baak.places"), "{err}");
    // The refusal changed nothing.
    assert!(ok(&mut s, "whoami").contains("@ project 'baak', in no collection"));
    // No hint toward a collection the user may not enter.
    let err = fails(&mut s, "use cache");
    assert!(err.contains("no access to project 'cache'") && !err.contains("baak.cache"), "{err}");
    assert!(ok(&mut s, "use baak.places").starts_with("Using collection 'baak.places'"));

    // The note names the project's collection only to who may enter it.
    let mut u = Session::new(inst.clone(), false, None);
    ok(&mut u, "auth u3 pw");
    ok(&mut u, "use baak.places");
    let used = ok(&mut u, "use users");
    assert!(used.starts_with("Using collection 'users'") && !used.contains("baak.users"), "{used}");
    ok(&mut admin, "use baak.places");
    assert!(ok(&mut admin, "use users").contains("baak.users also exists: use baak.users"));
}

#[test]
fn a_transaction_keeps_its_collection_whatever_the_name() {
    let dir = TempDir::new("collections_use_txn");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak collections (cache, places)");
    ok(&mut s, "use baak.places");
    ok(&mut s, "begin");
    ok(&mut s, "set t 1");
    for cmd in
        ["use cache", "use baak.cache", "use baak", "collection switch baak.cache", "workspace switch baak.cache"]
    {
        assert!(fails(&mut s, cmd).contains("inside a transaction"), "{cmd}");
    }
    ok(&mut s, "set t2 2");
    ok(&mut s, "commit");
    assert!(ok(&mut s, "whoami").contains("baak.places"));
    assert_eq!(ok(&mut s, "get t2"), "t2: 2 [int]");
    ok(&mut s, "use cache");
    assert!(ok(&mut s, "get t").contains("(nil)") && ok(&mut s, "get t2").contains("(nil)"));
    ok(&mut s, "begin");
    ok(&mut s, "set r 1");
    ok(&mut s, "rollback");
    assert!(ok(&mut s, "get r").contains("(nil)"));
}

#[test]
fn renaming_the_collection_in_use_says_rename() {
    let dir = TempDir::new("collections_rename_in_use");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak collections (places)");
    ok(&mut s, "use baak.places");
    ok(&mut s, "set k 1");
    let err = fails(&mut s, "collection rename baak.places to baak.locations");
    assert!(err.contains("you cannot rename the collection you are in"), "{err}");
    assert!(fails(&mut s, "workspace move baak.places to baak.locations").contains("cannot move the workspace"));
    // Nothing changed.
    assert!(ok(&mut s, "collection list").contains("baak.places"));
    ok(&mut s, "use default");
    ok(&mut s, "collection rename baak.places to baak.locations");
    ok(&mut s, "use baak.locations");
    assert_eq!(ok(&mut s, "get k"), "k: 1 [int]");
}
