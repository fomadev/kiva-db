// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Administrative storage lifecycle. No managed store is connected to a session.

use crate::catalog::Catalog;
use crate::engine::{Engine, EngineOptions};
use crate::error::{KivaError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

const JOURNAL: &str = "system/managed-layout.json";
const IDENTITY: &str = ".managed-identity";
const RETIRED: &str = "retired";

pub(crate) struct Resource {
    pub parent: Option<u64>,
    pub path: PathBuf,
    pub collection: bool,
}

type Resources = BTreeMap<u64, Resource>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    catalog: Option<serde_json::Value>,
    pending: Option<serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: u64,
    file_id: Option<u64>,
}

fn checked_meta(path: &Path, directory: bool) -> Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(KivaError::Invalid(format!("managed path uses a reparse point: {}", path.display())));
        }
    }
    if meta.is_symlink() || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(KivaError::Invalid(format!("unsafe managed path: {}", path.display())));
    }
    Ok(true)
}

fn require(path: &Path, directory: bool) -> Result<()> {
    if !checked_meta(path, directory)? {
        return Err(KivaError::Corrupted(format!("missing managed path: {}", path.display())));
    }
    Ok(())
}

fn checked_path(home: &Path, relative: &Path) -> Result<PathBuf> {
    let mut path = home.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(KivaError::Invalid("invalid managed path component".into()));
        };
        path.push(name);
        checked_meta(&path, true)?;
    }
    Ok(path)
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn create_private(path: &Path) -> Result<()> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path)?;
    Ok(())
}

fn ensure_parents(home: &Path, relative: &Path) -> Result<PathBuf> {
    let path = checked_path(home, relative)?;
    for ancestor in relative.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let directory = home.join(ancestor);
        if !checked_meta(&directory, true)? {
            create_private(&directory)?;
            sync_directory(directory.parent().expect("managed directory parent"))?;
        }
    }
    Ok(path)
}

fn insensitive_collision(path: &Path) -> Result<bool> {
    let parent = path.parent().expect("managed path parent");
    if !checked_meta(parent, true)? {
        return Ok(false);
    }
    let name = path.file_name().expect("managed resource name").to_string_lossy();
    for entry in fs::read_dir(parent)? {
        if entry?.file_name().to_string_lossy().eq_ignore_ascii_case(&name) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn file_id(path: &Path) -> Result<u64> {
    require(path, false)?;
    let mut header = [0; crate::format::FILE_HEADER_LEN as usize];
    File::open(path)?.read_exact(&mut header)?;
    crate::format::parse_file_header(&header).map(|(id, _)| id).map_err(KivaError::Corrupted)
}

fn validate_identity(path: &Path, id: u64, collection: bool) -> Result<()> {
    require(path, true)?;
    let marker = path.join(IDENTITY);
    require(&marker, false)?;
    let identity: Identity = serde_json::from_slice(&fs::read(&marker)?)?;
    if identity.id != id || identity.file_id.is_some() != collection {
        return Err(KivaError::Corrupted(format!("managed identity mismatch: {}", path.display())));
    }
    if let Some(expected) = identity.file_id {
        if file_id(&path.join("store.kiva"))? != expected {
            return Err(KivaError::Corrupted(format!("managed store header mismatch: {}", path.display())));
        }
    }
    Ok(())
}

enum Action {
    Create(u64, PathBuf, bool),
    Move(u64, PathBuf, PathBuf, bool),
}

fn actions(before: &Resources, after: &Resources) -> Vec<Action> {
    let mut result = Vec::new();
    for (&id, resource) in before {
        let next = after.get(&id);
        let changes = next.is_none_or(|node| node.path != resource.path);
        let parent_changes = resource.parent.is_some_and(|id| {
            let parent = &before[&id];
            after.get(&id).is_none_or(|node| node.path != parent.path)
        });
        if changes && !parent_changes {
            let destination =
                next.map_or_else(|| Path::new(RETIRED).join(format!("{id:016x}")), |node| node.path.clone());
            result.push(Action::Move(id, resource.path.clone(), destination, resource.collection));
        }
    }
    for (&id, resource) in after {
        if !before.contains_key(&id) {
            result.push(Action::Create(id, resource.path.clone(), resource.collection));
        }
    }
    result
}

fn validate_resources(home: &Path, resources: &Resources) -> Result<()> {
    let mut physical_stores = std::collections::HashSet::new();
    let legacy = home.join("data/store.kiva");
    if checked_meta(&legacy, false)? {
        physical_stores.insert(same_file::Handle::from_path(legacy)?);
    }
    for (&id, resource) in resources {
        let path = checked_path(home, &resource.path)?;
        validate_identity(&path, id, resource.collection)?;
        if !resource.collection {
            let container = if resource.parent.is_none() { "projects" } else { "collections" };
            for entry in fs::read_dir(&path)? {
                let name = entry?.file_name();
                if name != IDENTITY && name != container {
                    return Err(KivaError::Corrupted(format!(
                        "unregistered content in managed resource: {}",
                        path.display()
                    )));
                }
            }
            let children = path.join(container);
            if checked_meta(&children, true)? {
                let child_paths: std::collections::BTreeSet<_> = resources
                    .values()
                    .filter(|entry| entry.parent == Some(id))
                    .map(|entry| home.join(&entry.path))
                    .collect();
                for entry in fs::read_dir(&children)? {
                    let entry = entry?.path();
                    if !child_paths.contains(&entry) {
                        return Err(KivaError::Corrupted(format!(
                            "unregistered managed resource: {}",
                            entry.display()
                        )));
                    }
                }
            }
        }
        if resource.collection && !physical_stores.insert(same_file::Handle::from_path(path.join("store.kiva"))?) {
            return Err(KivaError::Corrupted(format!("managed stores share a file: {}", path.display())));
        }
    }
    Ok(())
}

fn check_movable(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let item = entry.path();
        if entry.file_type()?.is_dir() {
            require(&item, true)?;
            check_movable(&item)?;
        } else {
            require(&item, false)?;
            if entry.file_name() == "store.kiva" {
                let lock_path = path.join("store.kiva.lock");
                require(&lock_path, false)?;
                let lock = fs::OpenOptions::new().read(true).write(true).open(lock_path)?;
                if !crate::util::lock_exclusive(&lock) {
                    return Err(KivaError::Locked(item.display().to_string()));
                }
            }
        }
    }
    Ok(())
}

fn preflight(home: &Path, before: &Resources, actions: &[Action]) -> Result<()> {
    validate_resources(home, before)?;
    for action in actions {
        let destination = match action {
            Action::Create(_, path, _) | Action::Move(_, _, path, _) => checked_path(home, path)?,
        };
        if insensitive_collision(&destination)? {
            return Err(KivaError::Exists(format!("managed destination already exists: {}", destination.display())));
        }
        if let Action::Move(_, origin, _, _) = action {
            check_movable(&checked_path(home, origin)?)?;
        }
    }
    Ok(())
}

fn prepare_directory(home: &Path, id: u64, collection: bool, options: &EngineOptions) -> Result<PathBuf> {
    let scratch = ensure_parents(home, Path::new("system/managed-layout-staging"))?;
    let staging = scratch.join(format!("{id:016x}-{:016x}", crate::util::random_u64()?));
    crate::crash_point!("managed_layout.before_directory");
    create_private(&staging)?;
    crate::crash_point!("managed_layout.after_directory");
    let file_id = if collection {
        let mut options = options.clone();
        options.background = false;
        let store = staging.join("store.kiva");
        // This directory was allocated by this operation and has no store.
        crate::crash_point!("managed_layout.before_store");
        let engine = Engine::open(&store, options)?;
        crate::crash_point!("managed_layout.after_store");
        engine.sync()?;
        drop(engine);
        Some(file_id(&store)?)
    } else {
        None
    };
    crate::util::write_atomic(&staging.join(IDENTITY), &serde_json::to_vec(&Identity { id, file_id })?)?;
    sync_directory(&scratch)?;
    Ok(staging)
}

fn apply(home: &Path, action: Action, options: &EngineOptions) -> Result<()> {
    let (id, origin, destination, collection) = match action {
        Action::Create(id, destination, collection) => (id, None, destination, collection),
        Action::Move(id, origin, destination, collection) => (id, Some(origin), destination, collection),
    };
    let target = checked_path(home, &destination)?;
    let origin = origin.map(|path| checked_path(home, &path)).transpose()?;
    if checked_meta(&target, true)? {
        validate_identity(&target, id, collection)?;
        if let Some(origin) = origin {
            if checked_meta(&origin, true)? {
                return Err(KivaError::Corrupted("managed resource exists at both move endpoints".into()));
            }
            sync_directory(origin.parent().expect("managed source parent"))?;
        }
        return sync_directory(target.parent().expect("managed target parent"));
    }
    if insensitive_collision(&target)? {
        return Err(KivaError::Exists(format!("managed destination already exists: {}", target.display())));
    }
    let origin = match origin {
        Some(origin) => {
            validate_identity(&origin, id, collection)?;
            check_movable(&origin)?;
            origin
        }
        None => prepare_directory(home, id, collection, options)?,
    };
    let parent = ensure_parents(home, destination.parent().expect("managed target parent"))?;
    crate::crash_point!("managed_layout.before_move");
    fs::rename(&origin, &target)?;
    crate::crash_point!("managed_layout.after_move");
    sync_directory(&parent)?;
    sync_directory(origin.parent().expect("managed source parent"))?;
    Ok(())
}

pub(crate) fn validate_roots(home: &Path) -> Result<()> {
    for directory in ["system", "data", "workspaces", RETIRED] {
        checked_path(home, Path::new(directory))?;
    }
    Ok(())
}

pub(crate) fn exists(home: &Path) -> Result<bool> {
    checked_meta(&home.join(JOURNAL), false)
}

fn read_state(home: &Path) -> Result<Option<State>> {
    let file = home.join(JOURNAL);
    if !checked_meta(&file, false)? {
        return Ok(None);
    }
    checked_path(home, Path::new("system"))?;
    let state: State = serde_json::from_slice(&fs::read(file)?)?;
    if state.version != 1 || (state.catalog.is_none() && state.pending.is_none()) {
        return Err(KivaError::Corrupted("invalid managed layout state".into()));
    }
    Ok(Some(state))
}

fn finish(home: &Path, mut state: State, options: &EngineOptions) -> Result<Catalog> {
    let current = state.catalog.clone().map(Catalog::decode).transpose()?;
    let before = current.as_ref().map(Catalog::managed_entries).transpose()?.unwrap_or_default();
    let Some(next) = state.pending.take() else {
        validate_resources(home, &before)?;
        return current.ok_or_else(|| KivaError::Corrupted("missing managed catalog".into()));
    };
    let catalog = Catalog::decode(next.clone())?;
    let after = catalog.managed_entries()?;
    for action in actions(&before, &after) {
        apply(home, action, options)?;
    }
    validate_resources(home, &after)?;
    state.catalog = Some(next);
    crate::crash_point!("managed_layout.before_publish");
    crate::util::write_atomic(&home.join(JOURNAL), &serde_json::to_vec(&state)?)?;
    crate::crash_point!("managed_layout.after_publish");
    Ok(catalog)
}

pub(crate) fn open(home: &Path, options: &EngineOptions) -> Result<Option<Catalog>> {
    read_state(home)?.map(|state| finish(home, state, options)).transpose()
}

pub(crate) fn edit(
    home: &Path,
    options: &EngineOptions,
    mutation: impl FnOnce(&mut Catalog) -> Result<()>,
) -> Result<Catalog> {
    validate_roots(home)?;
    let current = open(home, options)?;
    let before = current.as_ref().map(Catalog::managed_entries).transpose()?.unwrap_or_default();
    let mut next = current.clone().unwrap_or_default();
    mutation(&mut next)?;
    let delta = actions(&before, &next.managed_entries()?);
    preflight(home, &before, &delta)?;
    let state = State {
        version: 1,
        catalog: current.as_ref().map(Catalog::encode).transpose()?,
        pending: Some(next.encode()?),
    };
    crate::util::write_atomic(&home.join(JOURNAL), &serde_json::to_vec(&state)?)?;
    crate::crash_point!("managed_layout.after_intent");
    finish(home, state, options)
}
