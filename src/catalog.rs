// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Logical hierarchy metadata, independent of collection engines and paths.

use crate::auth::valid_name_part;
use crate::engine::{BatchOp, Engine};
use crate::error::{KivaError, Result};
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct WorkspaceId(u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProjectId(u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CollectionId(u64);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry<Id, Parent> {
    id: Id,
    parent: Parent,
    name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    next_id: u64,
    workspaces: Vec<Entry<WorkspaceId, ()>>,
    projects: Vec<Entry<ProjectId, WorkspaceId>>,
    collections: Vec<Entry<CollectionId, ProjectId>>,
}

#[derive(Clone, Debug)]
pub struct Catalog(Snapshot);

impl Default for Catalog {
    fn default() -> Self {
        Self(Snapshot {
            version: VERSION,
            next_id: 2,
            workspaces: vec![Entry { id: WorkspaceId(1), parent: (), name: "default".into() }],
            projects: vec![],
            collections: vec![],
        })
    }
}

fn lookup<Id: Copy, Parent: PartialEq>(entries: &[Entry<Id, Parent>], parent: Parent, name: &str) -> Option<Id> {
    entries.iter().find(|entry| entry.parent == parent && entry.name.eq_ignore_ascii_case(name)).map(|entry| entry.id)
}

fn valid_name(name: &str) -> bool {
    valid_name_part(name) && !name.eq_ignore_ascii_case("system")
}

fn validate_entries<Id: Copy, Parent: Copy + Ord>(
    entries: &[Entry<Id, Parent>],
    mut valid_id: impl FnMut(Id) -> bool,
    valid_parent: impl Fn(Parent) -> bool,
) -> Result<()> {
    let mut names = BTreeSet::new();
    for entry in entries {
        if !valid_id(entry.id)
            || !valid_parent(entry.parent)
            || !valid_name(&entry.name)
            || !names.insert((entry.parent, entry.name.to_ascii_lowercase()))
        {
            return Err(KivaError::Corrupted(format!("invalid hierarchy catalog entry '{}'", entry.name)));
        }
    }
    Ok(())
}

fn register<Id: Copy, Parent: Copy + PartialEq>(
    entries: &mut Vec<Entry<Id, Parent>>,
    next_id: &mut u64,
    id: impl FnOnce(u64) -> Id,
    parent: Parent,
    name: &str,
) -> Result<Id> {
    if !valid_name(name) {
        return Err(KivaError::Invalid(format!("invalid resource name '{name}'")));
    }
    if lookup(entries, parent, name).is_some() {
        return Err(KivaError::Exists(format!("'{name}' already exists in this parent")));
    }
    let allocated = id(*next_id);
    *next_id = next_id.checked_add(1).ok_or_else(|| KivaError::Invalid("hierarchy catalog ID limit reached".into()))?;
    entries.push(Entry { id: allocated, parent, name: name.into() });
    Ok(allocated)
}

fn rename<Id: Copy + PartialEq, Parent: Copy + PartialEq>(
    entries: &mut [Entry<Id, Parent>],
    id: Id,
    name: &str,
) -> Result<()> {
    if !valid_name(name) {
        return Err(KivaError::Invalid(format!("invalid resource name '{name}'")));
    }
    let position = entries
        .iter()
        .position(|entry| entry.id == id)
        .ok_or_else(|| KivaError::NotFound("resource does not exist".into()))?;
    if lookup(entries, entries[position].parent, name).is_some_and(|existing| existing != id) {
        return Err(KivaError::Exists(format!("'{name}' already exists in this parent")));
    }
    entries[position].name = name.into();
    Ok(())
}

impl Catalog {
    pub fn decode(value: serde_json::Value) -> Result<Self> {
        let snapshot: Snapshot = serde_json::from_value(value)
            .map_err(|error| KivaError::Corrupted(format!("invalid hierarchy catalog: {error}")))?;
        if snapshot.version != VERSION {
            return Err(KivaError::Format(format!("unsupported hierarchy catalog version {}", snapshot.version)));
        }
        let mut ids = BTreeSet::new();
        let mut valid_id = |id| id > 0 && id < snapshot.next_id && ids.insert(id);
        validate_entries(&snapshot.workspaces, |id| valid_id(id.0), |()| true)?;
        let workspaces: BTreeSet<_> = snapshot.workspaces.iter().map(|entry| entry.id).collect();
        validate_entries(&snapshot.projects, |id| valid_id(id.0), |parent| workspaces.contains(&parent))?;
        let projects: BTreeSet<_> = snapshot.projects.iter().map(|entry| entry.id).collect();
        validate_entries(&snapshot.collections, |id| valid_id(id.0), |parent| projects.contains(&parent))?;

        if !snapshot.workspaces.iter().any(|entry| entry.id == WorkspaceId(1) && entry.name == "default") {
            return Err(KivaError::Corrupted("missing default workspace".into()));
        }
        Ok(Self(snapshot))
    }

    pub fn encode(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(&self.0)?)
    }

    pub fn workspace(&self, name: &str) -> Option<WorkspaceId> {
        lookup(&self.0.workspaces, (), name)
    }

    pub fn project(&self, workspace: WorkspaceId, name: &str) -> Option<ProjectId> {
        lookup(&self.0.projects, workspace, name)
    }

    pub fn collection(&self, project: ProjectId, name: &str) -> Option<CollectionId> {
        lookup(&self.0.collections, project, name)
    }

    pub fn create_workspace(&mut self, name: &str) -> Result<WorkspaceId> {
        register(&mut self.0.workspaces, &mut self.0.next_id, WorkspaceId, (), name)
    }

    pub fn create_project(&mut self, workspace: WorkspaceId, name: &str) -> Result<ProjectId> {
        if !self.0.workspaces.iter().any(|entry| entry.id == workspace) {
            return Err(KivaError::NotFound("workspace does not exist".into()));
        }
        register(&mut self.0.projects, &mut self.0.next_id, ProjectId, workspace, name)
    }

    pub fn create_collection(&mut self, project: ProjectId, name: &str) -> Result<CollectionId> {
        if !self.0.projects.iter().any(|entry| entry.id == project) {
            return Err(KivaError::NotFound("project does not exist".into()));
        }
        register(&mut self.0.collections, &mut self.0.next_id, CollectionId, project, name)
    }

    pub fn rename_workspace(&mut self, id: WorkspaceId, name: &str) -> Result<()> {
        if id == WorkspaceId(1) {
            return Err(KivaError::Invalid("the default workspace cannot be renamed".into()));
        }
        rename(&mut self.0.workspaces, id, name)
    }

    pub fn rename_project(&mut self, id: ProjectId, name: &str) -> Result<()> {
        rename(&mut self.0.projects, id, name)
    }

    pub fn rename_collection(&mut self, id: CollectionId, name: &str) -> Result<()> {
        rename(&mut self.0.collections, id, name)
    }

    /// Unregister a workspace and return the affected collections for the
    /// caller to detach its engines. This only changes catalog metadata.
    pub fn drop_workspace(&mut self, id: WorkspaceId, cascade: bool) -> Result<Vec<CollectionId>> {
        if id == WorkspaceId(1) {
            return Err(KivaError::Invalid("the default workspace cannot be dropped".into()));
        }
        if !self.0.workspaces.iter().any(|entry| entry.id == id) {
            return Err(KivaError::NotFound("workspace does not exist".into()));
        }
        let projects: BTreeSet<_> =
            self.0.projects.iter().filter(|entry| entry.parent == id).map(|entry| entry.id).collect();
        if !cascade && !projects.is_empty() {
            return Err(KivaError::Invalid("workspace contains projects; use cascade".into()));
        }
        let mut collections = Vec::new();
        self.0.collections.retain(|entry| {
            if projects.contains(&entry.parent) {
                collections.push(entry.id);
                false
            } else {
                true
            }
        });
        self.0.projects.retain(|entry| entry.parent != id);
        self.0.workspaces.retain(|entry| entry.id != id);
        Ok(collections)
    }
}

/// A proposed translation of one existing storage name. The collection keeps
/// its historical physical path when the plan is activated.
#[derive(Clone, Debug)]
pub struct LegacyCollection {
    pub name: String,
    pub qualified_name: String,
    pub id: CollectionId,
}

/// Build the catalog and routing table without writing metadata or opening
/// stores. The caller must validate and back up the source before activation.
pub fn plan_legacy_catalog(
    mut projects: Vec<String>,
    mut stores: Vec<String>,
) -> Result<(Catalog, Vec<LegacyCollection>)> {
    projects.sort_unstable();
    stores.sort_unstable();
    let mut catalog = Catalog::default();
    let default = WorkspaceId(1);
    for project in projects {
        catalog.create_project(default, &project)?;
    }

    let legacy = if stores.iter().any(|name| !name.contains('.')) {
        let project = std::iter::once("legacy".to_string())
            .chain((2..).map(|number| format!("legacy_{number}")))
            .find(|name| catalog.project(default, name).is_none())
            .expect("the legacy name space cannot be exhausted");
        let id = catalog.create_project(default, &project)?;
        Some((project, id))
    } else {
        None
    };
    let mut aliases = Vec::with_capacity(stores.len());
    for name in stores {
        let (project, parent, collection) = match name.split_once('.') {
            Some((project, collection)) => {
                let parent = catalog
                    .0
                    .projects
                    .iter()
                    .find(|entry| entry.parent == default && entry.name == project)
                    .ok_or_else(|| {
                    KivaError::Corrupted(format!("legacy collection '{name}' has no registered project"))
                })?;
                (project, parent.id, collection)
            }
            None => {
                let (project, id) = legacy.as_ref().expect("source has an unqualified store");
                (project.as_str(), *id, name.as_str())
            }
        };
        let id = catalog.create_collection(parent, collection)?;
        let qualified_name = format!("default.{project}.{collection}");
        aliases.push(LegacyCollection { name, qualified_name, id });
    }
    Ok((catalog, aliases))
}

const SYSTEM_KEY: &str = "catalog:workspace-hierarchy";

/// A catalog stored as one versioned record in the system engine.
///
/// The instance must finish legacy migration and validate physical mappings
/// before creating this record. Missing metadata is distinct from corruption.
pub struct CatalogStore<'a> {
    system: &'a Engine,
}

impl<'a> CatalogStore<'a> {
    pub fn new(system: &'a Engine) -> Self {
        Self { system }
    }

    pub fn load(&self) -> Result<Option<Catalog>> {
        match self.system.get(SYSTEM_KEY)? {
            None => Ok(None),
            Some((Value::Doc(document), _)) => Catalog::decode(document).map(Some),
            Some(_) => Err(KivaError::Corrupted("hierarchy catalog is not a document".into())),
        }
    }

    pub fn initialize(&self, catalog: &Catalog) -> Result<()> {
        let transaction = CatalogTransaction::begin(self.system);
        if self.load()?.is_some() {
            return Err(KivaError::Exists("hierarchy catalog already exists".into()));
        }
        transaction.write(catalog)
    }

    /// Publish a catalog-only change. Concurrent writers conflict, so an
    /// aborted attempt cannot overwrite a newer snapshot.
    pub fn update<T>(&self, edit: impl FnOnce(&mut Catalog) -> Result<T>) -> Result<T> {
        let transaction = CatalogTransaction::begin(self.system);
        let mut catalog = self.load()?.ok_or_else(|| KivaError::NotFound("hierarchy catalog is absent".into()))?;
        let result = edit(&mut catalog)?;
        transaction.write(&catalog)?;
        Ok(result)
    }
}

struct CatalogTransaction<'a> {
    system: &'a Engine,
    start: u64,
}

impl<'a> CatalogTransaction<'a> {
    fn begin(system: &'a Engine) -> Self {
        Self { system, start: system.txn_begin() }
    }

    fn write(&self, catalog: &Catalog) -> Result<()> {
        let record = BatchOp::Put {
            key: SYSTEM_KEY.into(),
            value: Value::Doc(catalog.encode()?),
            expires_at: 0,
            msgpack: false,
        };
        self.system.apply_batch(vec![record], Some(self.start))?;
        self.system.sync()
    }
}

impl Drop for CatalogTransaction<'_> {
    fn drop(&mut self) {
        self.system.txn_end();
    }
}
