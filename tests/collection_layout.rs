// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot C of docs/architecture/PROJECTS_AND_COLLECTIONS.md (section 5): new
//! collections go in `<home>/projects/<project>/<collection>` or
//! `<home>/collections/<name>`, their folders still written in kiva.conf;
//! folders of the layout holding files are not taken over, names differing
//! only by capitals are refused, and collections made before keep their
//! folders.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::backup::{self, RestoreOptions, Scope};
use kivadb::session::Session;

#[test]
fn new_collections_go_in_the_layout_of_projects() {
    let dir = TempDir::new("layout_new");
    let home = dir.path().to_path_buf();
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak");
    ok(&mut s, "collection create baak.places");
    ok(&mut s, "workspace create lab");
    ok(&mut s, "project create dzoket collections (orders)");
    let places = home.join("projects").join("baak").join("places");
    let lab = home.join("collections").join("lab");
    let orders = home.join("projects").join("dzoket").join("orders");
    for (name, folder) in [("baak.places", &places), ("lab", &lab), ("dzoket.orders", &orders)] {
        assert_eq!(inst.cfg().workspace_dir(name).as_deref(), Some(folder.as_path()), "{name}");
        ok(&mut s, &format!("use {name}"));
        ok(&mut s, "set k 1");
        assert!(folder.join("store.kiva").exists(), "{name}: {}", folder.display());
    }
    // A folder given by an administrator is used as it is.
    let mine = dir.path().join("elsewhere").join("db");
    ok(&mut s, &format!("collection create baak.mine \"{}\"", mine.display()));
    assert_eq!(inst.cfg().workspace_dir("baak.mine").as_deref(), Some(mine.as_path()));

    // A rename keeps the folder: names and folders are two things.
    ok(&mut s, "use default");
    ok(&mut s, "collection rename baak.places to baak.towns");
    assert_eq!(inst.cfg().workspace_dir("baak.towns").as_deref(), Some(places.as_path()));
}

#[test]
fn folders_holding_files_and_names_differing_by_capitals_are_refused() {
    let dir = TempDir::new("layout_checks");
    let home = dir.path().to_path_buf();
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak");
    ok(&mut s, "collection create baak.places");
    let err = fails(&mut s, "collection create baak.Places");
    assert!(err.contains("only by capitals"), "{err}");
    assert!(fails(&mut s, "collection create BAAK.places").contains("project 'BAAK' does not exist"));

    // Unregistered, its files stay: creating it again does not take them over.
    ok(&mut s, "use baak.places");
    ok(&mut s, "set secret 1");
    ok(&mut s, "use default");
    ok(&mut s, "collection remove baak.places");
    let err = fails(&mut s, "collection create baak.places");
    assert!(err.contains("already holds files") && err.contains("collection create baak.places \""), "{err}");
    assert!(inst.cfg().workspace_dir("baak.places").is_none());
    // Told to, with the folder, it does.
    let folder = home.join("projects").join("baak").join("places");
    ok(&mut s, &format!("collection create baak.places \"{}\"", folder.display()));
    ok(&mut s, "use baak.places");
    assert!(ok(&mut s, "get secret").contains('1'));

    // An empty folder of the layout is fine (a creation cut by a crash).
    std::fs::create_dir_all(home.join("collections").join("empty")).unwrap();
    ok(&mut s, "workspace create empty");
}

#[test]
fn collections_made_before_keep_their_folders() {
    let dir = TempDir::new("layout_old");
    let home = dir.path().to_path_buf();
    // A home of rc.4: its collections in <home>/workspaces/<name>.
    {
        let inst = instance(&dir);
        let old = home.join("workspaces").join("shop");
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, &format!("workspace create shop \"{}\"", old.display()));
        ok(&mut s, "use shop");
        ok(&mut s, "set p:1 1");
    }
    let inst = instance(&dir);
    assert_eq!(inst.cfg().workspace_dir("shop"), Some(home.join("workspaces").join("shop")));
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "use shop");
    assert!(ok(&mut s, "get p:1").contains('1'));
    assert!(!home.join("collections").join("shop").exists(), "nothing moved");
}

#[test]
fn a_restore_puts_collections_in_the_layout() {
    let dir = TempDir::new("layout_restore");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "project create baak");
    ok(&mut s, "collection create baak.places");
    ok(&mut s, "use baak.places");
    ok(&mut s, "set city:1 1");
    let archive = dir.path().join("home.tar.gz");
    backup::backup(&inst, &Scope::Home, &archive).unwrap();
    let archive_lab = dir.path().join("places.tar.gz");
    backup::backup(&inst, &Scope::Workspace("baak.places".into()), &archive_lab).unwrap();
    drop(s);
    drop(inst);

    // A whole home, into a new one.
    let target = dir.path().join("restored");
    backup::restore(&archive, dir.path(), &RestoreOptions { target: Some(target.clone()), ..Default::default() })
        .unwrap();
    let folder = target.join("projects").join("baak").join("places");
    assert!(folder.join("store.kiva").exists(), "{}", folder.display());
    // One collection, under a name of its own outside a project.
    let home2 = dir.path().join("home2");
    backup::restore(&archive_lab, &home2, &RestoreOptions { rename: Some("copy".into()), ..Default::default() })
        .unwrap();
    assert!(home2.join("collections").join("copy").join("store.kiva").exists());
}
