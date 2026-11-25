// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use kivadb::catalog::Catalog;
use kivadb::error::KivaError;

#[test]
fn hierarchy_round_trips_with_distinct_ids_for_homonyms() {
    let mut catalog = Catalog::default();
    let default = catalog.workspace("default").unwrap();
    assert!(catalog.project(default, "legacy").is_none());
    let fordima = catalog.create_workspace("Fordima").unwrap();
    let malanda = catalog.create_workspace("malanda").unwrap();
    assert!(matches!(catalog.create_workspace("fordima"), Err(KivaError::Exists(_))));
    assert!(catalog.create_workspace("../elsewhere").is_err());
    let first = catalog.create_project(fordima, "shop").unwrap();
    let second = catalog.create_project(malanda, "shop").unwrap();
    let archive = catalog.create_project(fordima, "archive").unwrap();
    let orders = catalog.create_collection(first, "orders").unwrap();
    let other_orders = catalog.create_collection(second, "orders").unwrap();
    let archived_orders = catalog.create_collection(archive, "orders").unwrap();
    assert_ne!(orders, other_orders);
    assert_ne!(orders, archived_orders);
    assert!(matches!(catalog.create_collection(first, "Orders"), Err(KivaError::Exists(_))));

    let snapshot = catalog.encode().unwrap();
    let decoded = Catalog::decode(snapshot.clone()).unwrap();
    assert_eq!(decoded.workspace("FORDIMA"), Some(fordima));
    assert_eq!(decoded.project(malanda, "shop"), Some(second));
    assert_eq!(decoded.collection(first, "orders"), Some(orders));
    assert_eq!(decoded.collection(second, "orders"), Some(other_orders));
    assert_eq!(decoded.encode().unwrap(), snapshot);
}

#[test]
fn missing_parents_are_rejected_without_allocating_ids() {
    let mut other = Catalog::default();
    let workspace = other.create_workspace("other").unwrap();
    let project = other.create_project(workspace, "shop").unwrap();

    let mut catalog = Catalog::default();
    let before = catalog.encode().unwrap();
    assert!(matches!(catalog.create_project(workspace, "shop"), Err(KivaError::NotFound(_))));
    assert!(matches!(catalog.create_collection(project, "orders"), Err(KivaError::NotFound(_))));
    assert_eq!(catalog.encode().unwrap(), before);
}

#[test]
fn malformed_snapshots_are_rejected() {
    let mut catalog = Catalog::default();
    let default = catalog.workspace("default").unwrap();
    catalog.create_project(default, "shop").unwrap();
    let original = catalog.encode().unwrap();

    let mut orphan = original.clone();
    orphan["projects"][0]["parent"] = 900.into();
    assert!(matches!(Catalog::decode(orphan), Err(KivaError::Corrupted(_))));

    let mut duplicate = original.clone();
    duplicate["projects"][0]["id"] = duplicate["workspaces"][0]["id"].clone();
    assert!(matches!(Catalog::decode(duplicate), Err(KivaError::Corrupted(_))));

    let mut missing_default = original.clone();
    missing_default["workspaces"][0]["name"] = "other".into();
    assert!(matches!(Catalog::decode(missing_default), Err(KivaError::Corrupted(_))));

    let mut unsupported = original;
    unsupported["version"] = 2.into();
    assert!(matches!(Catalog::decode(unsupported), Err(KivaError::Format(_))));
}

#[test]
fn migration_plan_keeps_legacy_names_distinct_and_refuses_orphans() {
    use kivadb::catalog::plan_legacy_catalog;

    let (catalog, aliases) =
        plan_legacy_catalog(vec!["legacy".into()], vec!["legacy.orders".into(), "default".into(), "orders".into()])
            .unwrap();
    let default = catalog.workspace("default").unwrap();
    let legacy = catalog.project(default, "legacy").unwrap();
    let legacy_import = catalog.project(default, "legacy_2").unwrap();
    assert_eq!(
        aliases.iter().map(|entry| (entry.name.as_str(), entry.qualified_name.as_str())).collect::<Vec<_>>(),
        [
            ("default", "default.legacy_2.default"),
            ("legacy.orders", "default.legacy.orders"),
            ("orders", "default.legacy_2.orders")
        ]
    );
    assert_eq!(catalog.collection(legacy, "orders"), Some(aliases[1].id));
    assert_eq!(catalog.collection(legacy_import, "orders"), Some(aliases[2].id));
    assert_ne!(aliases[1].id, aliases[2].id);
    assert!(plan_legacy_catalog(vec![], vec!["missing.orders".into()]).is_err());
    assert!(plan_legacy_catalog(vec!["shop".into()], vec!["shop.orders".into(), "shop.Orders".into()]).is_err());
}

#[test]
fn rename_preserves_ids_and_cascade_does_not_reuse_them() {
    let mut catalog = Catalog::default();
    let default = catalog.workspace("default").unwrap();
    assert!(catalog.drop_workspace(default, true).is_err());
    assert!(catalog.rename_workspace(default, "other").is_err());
    let workspace = catalog.create_workspace("fordima").unwrap();
    let project = catalog.create_project(workspace, "shop").unwrap();
    let collection = catalog.create_collection(project, "orders").unwrap();

    catalog.rename_workspace(workspace, "fordima_inc").unwrap();
    catalog.rename_project(project, "commerce").unwrap();
    catalog.rename_collection(collection, "sales").unwrap();
    let mut catalog = Catalog::decode(catalog.encode().unwrap()).unwrap();
    assert_eq!(catalog.workspace("fordima_inc"), Some(workspace));
    assert_eq!(catalog.project(workspace, "commerce"), Some(project));
    assert_eq!(catalog.collection(project, "sales"), Some(collection));
    let before = catalog.encode().unwrap();
    assert!(catalog.drop_workspace(workspace, false).is_err());
    assert_eq!(catalog.encode().unwrap(), before);
    assert_eq!(catalog.drop_workspace(workspace, true).unwrap(), vec![collection]);
    assert!(catalog.project(workspace, "commerce").is_none());
    assert!(catalog.collection(project, "sales").is_none());
    assert_ne!(catalog.create_workspace("fordima_inc").unwrap(), workspace);
}

mod common;

#[test]
fn catalog_updates_are_committed_as_one_record_and_reopen() {
    use common::TempDir;
    use kivadb::catalog::CatalogStore;
    use kivadb::engine::Engine;

    let dir = TempDir::new("hierarchy_store");
    let path = dir.path().join("system.kiva");
    let recorded_id = {
        let engine = Engine::open(&path, Default::default()).unwrap();
        let store = CatalogStore::new(&engine);
        assert!(store.load().unwrap().is_none());
        store.initialize(&Catalog::default()).unwrap();
        assert!(matches!(store.initialize(&Catalog::default()), Err(KivaError::Exists(_))));
        let before = store.load().unwrap().unwrap().encode().unwrap();
        let rejected = store.update(|catalog| {
            catalog.create_workspace("first")?;
            catalog.create_workspace("FIRST")
        });
        assert!(matches!(rejected, Err(KivaError::Exists(_))));
        assert_eq!(store.load().unwrap().unwrap().encode().unwrap(), before);

        let rejected = store.update(|catalog| {
            store.update(|other| other.create_workspace("winner"))?;
            catalog.create_workspace("stale")
        });
        assert!(matches!(rejected, Err(KivaError::Conflict(_))));
        let catalog = store.load().unwrap().unwrap();
        assert!(catalog.workspace("stale").is_none());
        catalog.workspace("winner").unwrap()
    };
    let engine = Engine::open(&path, Default::default()).unwrap();
    assert_eq!(CatalogStore::new(&engine).load().unwrap().unwrap().workspace("winner"), Some(recorded_id));
}

#[test]
fn startup_loads_staged_identities_without_switching_storage() {
    use common::{instance, ok, TempDir};
    use kivadb::catalog::{plan_legacy_catalog, CatalogStore};
    use kivadb::session::Session;

    let dir = TempDir::new("hierarchy_startup");
    let expected = {
        let inst = instance(&dir);
        assert!(inst.workspace_catalog().unwrap().is_none());
        let mut session = Session::new(inst.clone(), true, None);
        ok(&mut session, "set greeting 42");
        let (catalog, aliases) = plan_legacy_catalog(inst.projects(), inst.workspace_names()).unwrap();
        CatalogStore::new(&inst.system).initialize(&catalog).unwrap();
        drop(session);
        inst.shutdown();
        aliases[0].id
    };

    let inst = instance(&dir);
    let catalog = inst.workspace_catalog().unwrap().unwrap();
    assert_eq!(catalog.resolve_collection("default", "legacy", "default").unwrap(), expected);
    assert!(catalog.resolve_collection("default", "missing", "default").is_err());
    let mut session = Session::new(inst.clone(), true, None);
    assert_eq!(ok(&mut session, "get greeting"), "greeting: 42 [int]");
    assert_eq!(inst.workspace_names(), vec!["default"]);
}

#[test]
fn startup_refuses_invalid_staged_metadata_without_replacing_it() {
    use common::{instance, TempDir};
    use kivadb::engine::Engine;
    use kivadb::instance::{Instance, Options};
    use kivadb::value::Value;

    let dir = TempDir::new("hierarchy_startup_invalid");
    let key = "catalog:workspace-hierarchy";
    let invalid = serde_json::json!({"version":1,"next_id":2,"workspaces":[],"projects":[],"collections":[]});
    {
        let inst = instance(&dir);
        inst.system.put(key, &Value::Doc(invalid.clone()), 0, false).unwrap();
        inst.system.sync().unwrap();
        inst.shutdown();
    }
    assert!(matches!(Instance::open(dir.path(), Options::default()), Err(KivaError::Corrupted(_))));
    let system = Engine::open(dir.path().join("system/system.kiva"), Default::default()).unwrap();
    let Some((Value::Doc(document), _)) = system.get(key).unwrap() else {
        panic!("invalid record was removed or replaced");
    };
    assert_eq!(document, invalid);
}

#[test]
fn migration_preview_refuses_missing_and_shared_legacy_storage() {
    use common::{instance, TempDir};

    let dir = TempDir::new("hierarchy_preview_storage");
    let inst = instance(&dir);
    let missing = dir.path().join("missing");
    inst.cfg_mut().set("workspaces", "other", &missing.display().to_string()).unwrap();
    assert!(inst.plan_workspace_migration().unwrap_err().to_string().contains("legacy store 'other'"));

    let default = inst.cfg().workspace_dir("default").unwrap();
    inst.cfg_mut().set("workspaces", "other", &default.display().to_string()).unwrap();
    assert!(inst.plan_workspace_migration().unwrap_err().to_string().contains("share storage"));
    assert!(inst.workspace_catalog().unwrap().is_none());
}
