// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Offline staging. A single atomic record publishes the migration metadata.
//! Backup copies are never restored by this module: rollback only withdraws
//! the staged catalog, so legacy writes cannot be overwritten.

use crate::backup::{check_log, sha256_file};
use crate::catalog::{plan_legacy_catalog, Catalog, CatalogStore, LegacyCollection};
use crate::config::Config;
use crate::engine::{Engine, EngineOptions};
use crate::error::{KivaError, Result};
use crate::hierarchy_layout as layout;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

const RECORD: &str = "workspace-migration.json";
const BACKUPS: &str = "workspace-migration-backups";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Route {
    alias: LegacyCollection,
    directory: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CopyEntry {
    source: PathBuf,
    copy: String,
    sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Migration {
    version: u32,
    rolled_back: bool,
    backup: String,
    files: Vec<CopyEntry>,
    catalog: serde_json::Value,
    routes: Vec<Route>,
}

fn read_record(home: &Path) -> Result<Option<Migration>> {
    let bytes = match fs::read(home.join(RECORD)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let record: Migration = serde_json::from_slice(&bytes)
        .map_err(|e| KivaError::Corrupted(format!("invalid workspace migration record: {e}")))?;
    if record.version != 1 {
        return Err(KivaError::Format(format!("unsupported workspace migration version {}", record.version)));
    }
    let catalog = Catalog::decode(record.catalog.clone())?;
    let mut ids = BTreeSet::new();
    let mut historical = BTreeSet::new();
    for route in &record.routes {
        let parts: Vec<_> = route.alias.qualified_name.split('.').collect();
        let ["default", project, collection] = parts.as_slice() else {
            return Err(KivaError::Corrupted("invalid staged collection path".into()));
        };
        if catalog.resolve_collection("default", project, collection)? != route.alias.id
            || !ids.insert(route.alias.id)
            || !historical.insert(&route.alias.name)
        {
            return Err(KivaError::Corrupted("invalid or duplicate staged collection identity".into()));
        }
    }
    Ok(Some(record))
}

pub(crate) fn staged_catalog(home: &Path) -> Result<Option<Catalog>> {
    read_record(home)?.filter(|record| !record.rolled_back).map(|record| Catalog::decode(record.catalog)).transpose()
}

fn private_directory(path: &Path) -> Result<()> {
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

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn regular_path(path: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(KivaError::Invalid(format!("expected a regular file: {}", path.display())));
    }
    fs::canonicalize(path).map_err(Into::into)
}

fn collect_files(path: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        for entry in fs::read_dir(path)? {
            collect_files(&entry?.path(), files)?;
        }
    } else if path.extension().is_none_or(|extension| extension != "lock") {
        files.insert(regular_path(path)?);
    }
    Ok(())
}

fn verify_log(path: &Path) -> Result<()> {
    check_log(path).map_err(|error| KivaError::Corrupted(format!("{}: {error}", path.display())))?;
    Ok(())
}

fn inspect_copy(source: &Path, copy: &Path, options: EngineOptions) -> Result<std::sync::Arc<Engine>> {
    verify_log(source)?;
    fs::copy(source, copy)?;
    let engine = Engine::open(copy, options)?;
    if sha256_file(copy)? != sha256_file(source)? {
        return Err(KivaError::Corrupted(format!("legacy log needs recovery before migration: {}", source.display())));
    }
    if !engine.check(false)?.corrupted.is_empty() {
        return Err(KivaError::Corrupted(format!(
            "legacy log failed authentication or checksum validation: {}",
            source.display()
        )));
    }
    Ok(engine)
}

struct SourcePlan {
    catalog: Catalog,
    routes: Vec<Route>,
    files: BTreeSet<PathBuf>,
    _locks: Vec<File>,
}

fn lock_source(path: &Path) -> Result<File> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path)?;
    if !crate::util::lock_exclusive(&lock) {
        return Err(KivaError::Locked(path.display().to_string()));
    }
    Ok(lock)
}

fn plan(home: &Path) -> Result<SourcePlan> {
    let config_file = regular_path(&home.join("kiva.conf"))?;
    let config = Config::read_existing(home)?;
    let system_file = regular_path(&home.join("system/system.kiva"))?;
    let mut locks = vec![lock_source(&system_file)?];
    let scratch = std::env::temp_dir().join(format!("kivadb-migrate-{:016x}", crate::util::random_u64()?));
    private_directory(&scratch)?;
    let scratch = Scratch(scratch);
    let options = EngineOptions {
        background: false,
        keys: crate::crypt::Keys::load(&config)?,
        require_encryption: config.get_bool("encryption", "required"),
        ..Default::default()
    };
    let system = inspect_copy(&system_file, &scratch.0.join("system.kiva"), options.clone())?;
    if !system.keys_matching("pending:*", 1).is_empty() || !system.keys_matching("moving:*", 1).is_empty() {
        return Err(KivaError::Invalid("finish pending legacy administrative operations before migration".into()));
    }
    if CatalogStore::new(&system).load()?.is_some() {
        return Err(KivaError::Exists("this home already contains a staged system catalog".into()));
    }

    let projects = system.keys_matching("project:*", 0).into_iter().map(|(key, _)| key[8..].to_owned()).collect();
    let names = std::iter::once("default".to_owned()).chain(config.workspaces().into_iter().map(|(name, _)| name));
    let (catalog, aliases) = plan_legacy_catalog(projects, names.collect())?;
    let mut files = BTreeSet::from([config_file]);
    for name in ["system", "tls", "audit.log"] {
        let path = home.join(name);
        if path.exists() {
            collect_files(&path, &mut files)?;
        }
    }
    let mut routes = Vec::new();
    let mut directories = BTreeSet::from([fs::canonicalize(home.join("system"))?]);
    let mut file_ids = HashSet::from([same_file::Handle::from_path(&system_file)?]);
    for alias in aliases {
        let directory = config
            .workspace_dir(&alias.name)
            .ok_or_else(|| KivaError::Corrupted(format!("missing registration for '{}'", alias.name)))?;
        let meta = fs::symlink_metadata(&directory)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(KivaError::Invalid(format!("expected a legacy directory: {}", directory.display())));
        }
        let canonical_dir = fs::canonicalize(&directory)?;
        let backup_root = home.join(BACKUPS);
        if canonical_dir == home
            || canonical_dir.starts_with(&backup_root)
            || backup_root.starts_with(&canonical_dir)
            || directories.iter().any(|other| other.starts_with(&canonical_dir) || canonical_dir.starts_with(other))
        {
            return Err(KivaError::Invalid(format!("overlapping legacy directory: {}", directory.display())));
        }
        directories.insert(canonical_dir);
        let store = regular_path(&directory.join("store.kiva"))?;
        if !file_ids.insert(same_file::Handle::from_path(&store)?) {
            return Err(KivaError::Invalid(format!("hard-linked legacy store: {}", store.display())));
        }
        locks.push(lock_source(&store)?);
        let copy = scratch.0.join(format!("collection-{}.kiva", routes.len()));
        let engine = inspect_copy(&store, &copy, options.clone())?;
        drop(engine);
        // Engine sidecars are named after the opened path. Use a new path for
        // each source so no cached index from another store can be reused.
        fs::remove_file(copy)?;
        collect_files(&directory, &mut files)?;
        routes.push(Route { alias, directory });
    }
    Ok(SourcePlan { catalog, routes, files, _locks: locks })
}

fn verify_backup(home: &Path, migration: &Migration) -> Result<()> {
    let name = &migration.backup;
    if name.len() != 16 || !name.bytes().all(|b| b.is_ascii_hexdigit()) || migration.files.is_empty() {
        return Err(KivaError::Corrupted("invalid migration backup manifest".into()));
    }
    let dir = home.join(BACKUPS).join(name);
    for (i, entry) in migration.files.iter().enumerate() {
        if entry.copy != format!("{i:08}") || sha256_file(&dir.join(&entry.copy))? != entry.sha256 {
            return Err(KivaError::Corrupted(format!("migration backup does not match {}", entry.source.display())));
        }
    }
    Ok(())
}

fn backup(home: &Path, sources: BTreeSet<PathBuf>) -> Result<(String, Vec<CopyEntry>)> {
    let root = home.join(BACKUPS);
    match private_directory(&root) {
        Ok(()) => {}
        Err(KivaError::Io(e))
            if e.kind() == std::io::ErrorKind::AlreadyExists
                && fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) => {}
        Err(e) => return Err(e),
    }
    let id = format!("{:016x}", crate::util::random_u64()?);
    let dir = root.join(&id);
    private_directory(&dir)?;
    let mut files = Vec::new();
    for source in sources {
        let sha256 = sha256_file(&source)?;
        let copy = format!("{:08}", files.len());
        let mut target = OpenOptions::new().write(true).create_new(true).open(dir.join(&copy))?;
        std::io::copy(&mut File::open(&source)?, &mut target)?;
        target.sync_all()?;
        if sha256_file(&source)? != sha256 {
            return Err(KivaError::Conflict(format!("source was modified while copying {}", source.display())));
        }
        files.push(CopyEntry { source, copy, sha256 });
    }
    #[cfg(unix)]
    {
        File::open(&dir)?.sync_all()?;
        File::open(&root)?.sync_all()?;
    }
    Ok((id, files))
}

fn verify_manifest(home: &Path, record: &Migration) -> Result<()> {
    verify_backup(home, record)?;
    let manifest = home.join(BACKUPS).join(&record.backup).join("manifest.json");
    let mut expected = serde_json::to_value(record)?;
    expected["rolled_back"] = false.into();
    let archived: serde_json::Value = serde_json::from_slice(&fs::read(regular_path(&manifest)?)?)?;
    if archived != expected {
        return Err(KivaError::Corrupted("staged migration differs from the backup manifest".into()));
    }
    Ok(())
}

fn destination(route: &Route) -> Result<PathBuf> {
    let mut parts = route.alias.qualified_name.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(workspace), Some(project), Some(name), None) => {
            layout::collection(Path::new(""), workspace, project, name)
        }
        _ => Err(KivaError::Corrupted("invalid staged collection name".into())),
    }
}

fn export_target(home: &Path, target: &Path) -> Result<PathBuf> {
    let name = target.file_name().ok_or_else(|| KivaError::Invalid("provide a new target directory".into()))?;
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    // Check every existing component. Canonicalizing alone would silently
    // follow directory links and Windows junctions supplied as the target.
    for ancestor in parent.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let meta = fs::symlink_metadata(ancestor)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(KivaError::Invalid(format!("target uses a reparse point: {}", ancestor.display())));
            }
        }
        if !meta.is_dir() || meta.is_symlink() {
            return Err(KivaError::Invalid(format!("target uses a non-directory parent: {}", ancestor.display())));
        }
    }
    let target = fs::canonicalize(parent)?.join(name);
    if target.starts_with(home) || home.starts_with(&target) {
        return Err(KivaError::Invalid("choose a new target outside the database home".into()));
    }
    Ok(target)
}

fn export_layout(home: &Path, record: &Migration, target: &Path) -> Result<String> {
    let target = export_target(home, target)?;
    let catalog = Catalog::decode(record.catalog.clone())?;
    let mut routes = Vec::new();
    for route in &record.routes {
        let original = fs::canonicalize(&route.directory)?;
        if original.starts_with(&target) || target.starts_with(&original) {
            return Err(KivaError::Invalid(format!("target overlaps {}", original.display())));
        }
        let relative = destination(route)?;
        let mut files = Vec::new();
        for entry in &record.files {
            if let Ok(path) = entry.source.strip_prefix(&original) {
                if path.as_os_str().is_empty()
                    || !path.components().all(|part| matches!(part, std::path::Component::Normal(_)))
                {
                    return Err(KivaError::Corrupted("invalid backup file path".into()));
                }
                files.push((path, entry));
            }
        }
        if !files.iter().any(|(path, _)| *path == Path::new("store.kiva")) {
            return Err(KivaError::Corrupted(format!("backup does not contain storage for '{}'", route.alias.name)));
        }
        routes.push((route, relative, files));
    }

    private_directory(&target)?;
    let marker = target.join(layout::EXPORT_MARKER);
    crate::util::write_atomic(&marker, br#"{"version":1,"state":"COPYING"}"#)?;
    let backup = home.join(BACKUPS).join(&record.backup);
    let mut exported = Vec::new();
    let mut exported_files = Vec::new();
    for workspace in catalog.workspace_names() {
        fs::create_dir_all(layout::workspace(&target, workspace)?)?;
        let id = catalog.workspace(workspace).expect("catalog workspace");
        for project in catalog.project_names(id) {
            fs::create_dir_all(layout::project(&target, workspace, project)?)?;
        }
    }
    for (route, directory, files) in routes {
        let folder = target.join(&directory);
        fs::create_dir_all(folder.parent().expect("collection parent"))?;
        private_directory(&folder)?;
        for (relative, entry) in files {
            let path = folder.join(relative);
            fs::create_dir_all(path.parent().expect("file parent"))?;
            let mut output = OpenOptions::new().write(true).create_new(true).open(&path)?;
            let input = regular_path(&backup.join(&entry.copy))?;
            std::io::copy(&mut File::open(input)?, &mut output)?;
            output.sync_all()?;
            if sha256_file(&path)? != entry.sha256 {
                return Err(KivaError::Corrupted(format!("layout copy failed verification: {}", path.display())));
            }
            exported_files.push(serde_json::json!({"path": directory.join(relative), "sha256": entry.sha256}));
        }
        crate::crash_point!("migration.layout_after_collection");
        exported.push(serde_json::json!({"id": route.alias.id, "name": route.alias.qualified_name, "path": directory}));
    }
    #[cfg(unix)]
    {
        fn sync_tree(path: &Path) -> Result<()> {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    sync_tree(&entry.path())?;
                }
            }
            File::open(path)?.sync_all()?;
            Ok(())
        }
        sync_tree(&target)?;
        File::open(target.parent().expect("export parent"))?.sync_all()?;
    }
    crate::util::write_atomic(
        &marker,
        &serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1, "state": "COMPLETE", "backup": record.backup, "collections": exported, "files": exported_files,
        }))?,
    )?;
    Ok(format!("EXPORTED verified collection snapshot to {}", target.display()))
}

pub fn run(home: &Path, action: &str) -> Result<String> {
    run_with_target(home, action, None)
}

pub fn run_with_target(home: &Path, action: &str, target: Option<&Path>) -> Result<String> {
    let home = fs::canonicalize(home)?;
    let lock =
        OpenOptions::new().read(true).write(true).create(true).truncate(false).open(home.join("instance.lock"))?;
    if !crate::util::lock_exclusive(&lock) {
        return Err(KivaError::Locked(home.display().to_string()));
    }
    let existing = read_record(&home)?;
    match action {
        "status" => Ok(match existing {
            None => "NOT STARTED",
            Some(record) if record.rolled_back => "ROLLED BACK",
            Some(_) => "STAGED",
        }
        .into()),
        "plan" => {
            let source = plan(&home)?;
            Ok(source
                .routes
                .iter()
                .map(|r| format!("{} -> {} ({})", r.alias.name, r.alias.qualified_name, r.directory.display()))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "apply" => {
            if existing.is_some() {
                return Err(KivaError::Exists("migration history exists; use status or verify".into()));
            }
            let source = plan(&home)?;
            let (backup, files) = backup(&home, source.files)?;
            let record = Migration {
                version: 1,
                rolled_back: false,
                backup,
                files,
                catalog: source.catalog.encode()?,
                routes: source.routes,
            };
            verify_backup(&home, &record)?;
            // Keep a manifest with the backup, including if publication is interrupted.
            crate::util::write_atomic(
                &home.join(BACKUPS).join(&record.backup).join("manifest.json"),
                &serde_json::to_vec_pretty(&record)?,
            )?;
            crate::crash_point!("migration.before_publish");
            crate::util::write_atomic(&home.join(RECORD), &serde_json::to_vec_pretty(&record)?)?;
            crate::crash_point!("migration.after_publish");
            Ok(format!("STAGED; verified backup {}", home.join(BACKUPS).join(record.backup).display()))
        }
        "layout-plan" | "copy-layout" => {
            let record = existing
                .filter(|record| !record.rolled_back)
                .ok_or_else(|| KivaError::NotFound("stage a workspace migration first".into()))?;
            verify_manifest(&home, &record)?;
            if action == "copy-layout" {
                return export_layout(
                    &home,
                    &record,
                    target.ok_or_else(|| {
                        KivaError::Invalid("copy-layout requires --target with a new directory".into())
                    })?,
                );
            }
            let mut lines = Vec::new();
            for route in &record.routes {
                lines.push(format!("{} -> {}", route.directory.display(), destination(route)?.display()));
            }
            Ok(lines.join("\n"))
        }
        "verify" | "rollback" => {
            let mut record =
                existing.ok_or_else(|| KivaError::NotFound("workspace migration has not been staged".into()))?;
            verify_manifest(&home, &record)?;
            if action == "verify" {
                return Ok("VERIFIED; legacy routing remains in effect".into());
            }
            if !record.rolled_back {
                record.rolled_back = true;
                crate::crash_point!("migration.before_rollback");
                crate::util::write_atomic(&home.join(RECORD), &serde_json::to_vec_pretty(&record)?)?;
                crate::crash_point!("migration.after_rollback");
            }
            Ok("ROLLED BACK; backup and legacy files retained".into())
        }
        _ => Err(KivaError::Invalid(
            "expected migrate plan, apply, status, verify, rollback, layout-plan or copy-layout".into(),
        )),
    }
}
