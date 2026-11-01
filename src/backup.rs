// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! `kivadb backup` and `kivadb restore`: a `.tar.gz` archive of a whole home
//! or of one workspace, with a SHA-256 for every file and for the archive.
//!
//! Archive layout, everything under `kivadb-backup/`:
//!
//! ```text
//! kiva.conf                         (home backups)
//! system/system.kiva[.meta]         users, scripts, triggers (home backups)
//! workspaces/<name>/store.kiva      one folder per workspace
//! workspaces/<name>/store.kiva.meta index definitions
//! audit.log, tls/...                (home backups)
//! manifest.json                     last: what was saved, with the hashes
//! ```
//!
//! A data file is copied up to the length it had at one instant, taken under
//! the engine's lock: the log is append-only, so that prefix is a consistent
//! state of the workspace, and writes go on meanwhile. Compactions (which
//! replace the file) and resets (which truncate it) wait until the copy ends.
//! The index snapshot and HNSW graphs are not saved: they are rebuilt.
//! Encrypted records stay encrypted in the archive.

use crate::engine::Engine;
use crate::error::{KivaError, Result};
use crate::format::{
    parse_file_header, parse_header, FILE_HEADER_LEN, FLAG_ENCRYPTED, FLAG_KDOC, FLAG_TOMBSTONE, REC_HEADER_LEN,
};
use crate::instance::{Instance, VERSION};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const ROOT: &str = "kivadb-backup";
const FORMAT: u64 = 1;
/// The manifest format of an archive holding a data file of storage format 3:
/// an older KivaDB refuses it before restoring anything.
const FORMAT_NATIVE: u64 = 2;

/// What to save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Configuration, users and every workspace.
    Home,
    /// One workspace.
    Workspace(String),
}

#[derive(Debug)]
pub struct BackupReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub files: usize,
    pub workspaces: Vec<String>,
    /// Id of the encryption key the data was written with, if any.
    pub key_id: Option<String>,
}

impl BackupReport {
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!(
                "Backup written: {} ({}, {} file(s), workspace(s): {})",
                self.path.display(),
                crate::util::human_size(self.bytes),
                self.files,
                self.workspaces.join(", ")
            ),
            format!("SHA-256: {} (also in {})", self.sha256, checksum_path(&self.path).display()),
        ];
        if let Some(id) = &self.key_id {
            out.push(format!(
                "The data is encrypted at rest (key {id}): keep that key with care, the backup cannot be read without it."
            ));
        }
        out
    }
}

/// What to tell when `<archive>.sha256` is missing, `None` when it is there.
pub fn missing_checksum_note(archive: &Path) -> Option<String> {
    let sums = checksum_path(archive);
    (archive.is_file() && !sums.exists()).then(|| {
        format!(
            "Note: no {} next to the archive: checking the files inside it only.",
            sums.file_name().unwrap_or_default().to_string_lossy()
        )
    })
}

/// `<archive>.sha256`, in the format of `sha256sum`.
pub fn checksum_path(archive: &Path) -> PathBuf {
    let mut s = archive.as_os_str().to_owned();
    s.push(".sha256");
    PathBuf::from(s)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Passes bytes through, hashing them.
struct Hashing<T> {
    inner: T,
    hash: Sha256,
    count: u64,
    /// The first bytes read: a data file's header.
    head: Vec<u8>,
}

impl<T> Hashing<T> {
    fn new(inner: T) -> Self {
        Hashing { inner, hash: Sha256::new(), count: 0, head: Vec::new() }
    }
}

impl<T: Write> Write for Hashing<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hash.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: Read> Read for Hashing<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let want = (FILE_HEADER_LEN as usize).saturating_sub(self.head.len()).min(n);
        self.head.extend_from_slice(&buf[..want]);
        self.hash.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }
}

struct Writer<W: Write> {
    tar: tar::Builder<W>,
    files: Vec<Json>,
    /// The newest storage format among the data files saved.
    storage_format: u32,
}

impl<W: Write> Writer<W> {
    /// Saves a file; returns its first bytes.
    fn add(&mut self, name: &str, len: u64, data: impl Read) -> Result<Vec<u8>> {
        let mut header = tar::Header::new_gnu();
        header.set_size(len);
        header.set_mode(0o600);
        header.set_mtime(crate::util::now_secs().max(0) as u64);
        header.set_cksum();
        let mut reader = Hashing::new(data.take(len));
        self.tar.append_data(&mut header, format!("{ROOT}/{name}"), &mut reader)?;
        if reader.count != len {
            return Err(KivaError::Other(format!("{name} changed size while it was saved")));
        }
        self.files.push(json!({ "path": name, "size": len, "sha256": hex(&reader.hash.finalize()) }));
        Ok(reader.head)
    }

    fn add_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.add(name, bytes.len() as u64, bytes).map(|_| ())
    }

    /// The data file of an engine as it is now, and its index definitions.
    fn add_engine(&mut self, dir: &str, file: &str, engine: &Engine) -> Result<()> {
        let snap = engine.backup_snapshot()?;
        let head = self.add(&format!("{dir}/{file}"), snap.len, &snap.file)?;
        if let Ok((_, format)) = parse_file_header(&head) {
            self.storage_format = self.storage_format.max(format);
        }
        if let Some(meta) = &snap.meta {
            self.add_bytes(&format!("{dir}/{file}.meta"), meta)?;
        }
        Ok(())
    }
}

/// Writes a backup of `scope` to `output`, a new `.tar.gz` file, and its
/// checksum next to it. Works while the instance serves clients.
pub fn backup(inst: &Instance, scope: &Scope, output: &Path) -> Result<BackupReport> {
    if output.exists() {
        return Err(KivaError::Exists(format!("{} already exists: choose another file name", output.display())));
    }
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = output.as_os_str().to_owned();
    tmp.push(".partial");
    let tmp = PathBuf::from(tmp);
    let result = write_archive(inst, scope, &tmp);
    let (sha, workspaces, files) = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    fs::rename(&tmp, output)?;
    let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    fs::write(checksum_path(output), format!("{sha}  {name}\n"))?;
    Ok(BackupReport {
        path: output.to_path_buf(),
        bytes: fs::metadata(output)?.len(),
        sha256: sha,
        files,
        workspaces,
        key_id: inst.keys.as_ref().map(|k| k.current.id_hex()),
    })
}

fn write_archive(inst: &Instance, scope: &Scope, path: &Path) -> Result<(String, Vec<String>, usize)> {
    let file = Hashing::new(File::create(path)?);
    let gz = GzEncoder::new(file, flate2::Compression::default());
    let mut w = Writer { tar: tar::Builder::new(gz), files: Vec::new(), storage_format: 0 };
    let names: Vec<String> = match scope {
        Scope::Home => {
            let cfg = inst.cfg();
            std::iter::once("default".to_string()).chain(cfg.workspaces().into_iter().map(|(n, _)| n)).collect()
        }
        Scope::Workspace(n) => vec![n.clone()],
    };
    let mut listed = Vec::new();
    for name in &names {
        let dir = inst.cfg().workspace_dir(name).unwrap_or_default();
        let engine = inst.engine(name)?;
        w.add_engine(&format!("workspaces/{name}"), "store.kiva", &engine)?;
        listed.push(json!({ "name": name, "dir": dir.display().to_string() }));
    }
    if *scope == Scope::Home {
        let conf = fs::read(inst.cfg().path())?;
        w.add_bytes("kiva.conf", &conf)?;
        w.add_engine("system", "system.kiva", &inst.system)?;
        if let Ok(log) = fs::read(inst.home.join("audit.log")) {
            w.add_bytes("audit.log", &log)?;
        }
        if let Ok(entries) = fs::read_dir(inst.home.join("tls")) {
            for e in entries.flatten() {
                if e.path().is_file() {
                    let bytes = fs::read(e.path())?;
                    w.add_bytes(&format!("tls/{}", e.file_name().to_string_lossy()), &bytes)?;
                }
            }
        }
    }
    let count = w.files.len();
    let manifest = json!({
        "format": if w.storage_format >= 3 { FORMAT_NATIVE } else { FORMAT },
        "kivadb": VERSION,
        "created": crate::util::now_secs(),
        "scope": if *scope == Scope::Home { "home" } else { "workspace" },
        "workspaces": listed,
        "encryption_key": inst.keys.as_ref().map(|k| k.current.id_hex()),
        "files": w.files,
    });
    let text = serde_json::to_vec_pretty(&manifest)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(text.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(crate::util::now_secs().max(0) as u64);
    header.set_cksum();
    w.tar.append_data(&mut header, format!("{ROOT}/manifest.json"), &text[..])?;
    let gz = w.tar.into_inner()?;
    let mut file = gz.finish()?;
    file.flush()?;
    file.inner.sync_all()?;
    Ok((hex(&file.hash.finalize()), names, count))
}

// -- restore ------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct RestoreOptions {
    /// Home backup: the new home (default: the home of the command).
    /// Workspace backup: the folder of the restored workspace.
    pub target: Option<PathBuf>,
    /// Workspace backup: the name to restore it under (default: its own).
    pub rename: Option<String>,
    /// Only check the archive.
    pub check_only: bool,
}

/// An archive unpacked into a temporary folder and checked.
pub struct Checked {
    pub manifest: Json,
    pub staging: PathBuf,
    pub lines: Vec<String>,
}

impl Drop for Checked {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.staging);
    }
}

fn bad(archive: &Path, why: impl std::fmt::Display) -> KivaError {
    KivaError::Corrupted(format!("{} cannot be restored: {why}", archive.display()))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut h = Hashing::new(io::sink());
    io::copy(&mut File::open(path)?, &mut h)?;
    Ok(hex(&h.hash.finalize()))
}

/// Checks the framing and the checksum of every record of a data file
/// (encrypted records too: their checksum covers the encrypted bytes).
pub fn check_log(path: &Path) -> std::result::Result<u64, String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    if data.len() < FILE_HEADER_LEN as usize {
        return Err("the data file is too short".into());
    }
    parse_file_header(&data[..FILE_HEADER_LEN as usize])?;
    let mut pos = FILE_HEADER_LEN as usize;
    let mut records = 0;
    while pos < data.len() {
        if data.len() - pos < REC_HEADER_LEN {
            return Err(format!("truncated record at byte {pos}"));
        }
        let h = parse_header(&data[pos..]);
        let end = pos + h.total_len() as usize;
        if h.k_size == 0 || end > data.len() {
            return Err(format!("truncated record at byte {pos}"));
        }
        if crc32fast::hash(&data[pos + 4..end]) != h.crc {
            return Err(format!("checksum mismatch in the record at byte {pos}"));
        }
        // A KDOC document is read whole (an encrypted one needs its key: its checksum is checked).
        if h.flags & FLAG_KDOC != 0 && h.flags & (FLAG_ENCRYPTED | FLAG_TOMBSTONE) == 0 {
            let stored = &data[pos + REC_HEADER_LEN + h.k_size as usize..end];
            crate::compress::unpack(stored, h.compression())
                .map_err(|e| e.to_string())
                .and_then(|raw| crate::doc::decode(&raw))
                .map_err(|e| format!("the document in the record at byte {pos} cannot be read: {e}"))?;
        }
        records += 1;
        pos = end;
    }
    Ok(records)
}

/// Unpacks `archive` into a temporary folder next to `near` and checks it:
/// the archive's own SHA-256 when `<archive>.sha256` exists, then every file
/// against the manifest, then every record of every data file.
pub fn check(archive: &Path, near: &Path) -> Result<Checked> {
    if !archive.is_file() {
        return Err(KivaError::NotFound(format!("{} does not exist", archive.display())));
    }
    let mut lines = Vec::new();
    let sums = checksum_path(archive);
    // Without it, the command line says so before starting (`missing_checksum_note`).
    if let Ok(text) = fs::read_to_string(&sums) {
        let want = text.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if sha256_file(archive)? != want {
            return Err(bad(
                archive,
                format!("its SHA-256 does not match {}: it was damaged or changed", sums.display()),
            ));
        }
        lines.push(format!("Archive SHA-256 matches {}.", sums.display()));
    }

    fs::create_dir_all(near)?;
    let staging = near.join(format!(".kivadb-restore-{:016x}", crate::util::random_u64()));
    fs::create_dir_all(&staging)?;
    let mut checked = Checked { manifest: Json::Null, staging: staging.clone(), lines };

    let mut tar = tar::Archive::new(GzDecoder::new(File::open(archive)?));
    let mut found: Vec<(String, u64, String)> = Vec::new();
    for entry in tar.entries().map_err(|e| bad(archive, e))? {
        let mut entry = entry.map_err(|e| bad(archive, e))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|e| bad(archive, e))?.into_owned();
        let mut parts = path.components();
        if parts.next() != Some(Component::Normal(ROOT.as_ref())) {
            return Err(bad(archive, format!("unexpected file {}", path.display())));
        }
        let rel: PathBuf = parts.clone().collect();
        if rel.as_os_str().is_empty() || !parts.all(|c| matches!(c, Component::Normal(_))) {
            return Err(bad(archive, format!("unsafe path {}", path.display())));
        }
        let dest = staging.join(&rel);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }
        let mut out = Hashing::new(File::create(&dest)?);
        io::copy(&mut entry, &mut out).map_err(|e| bad(archive, e))?;
        let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        found.push((name, out.count, hex(&out.hash.finalize())));
    }

    let manifest: Json = fs::read(staging.join("manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| bad(archive, "it has no readable manifest.json: is it a KivaDB backup?"))?;
    if !manifest["format"].as_u64().is_some_and(|f| (FORMAT..=FORMAT_NATIVE).contains(&f)) {
        return Err(bad(archive, format!("backup format {} is not supported by this KivaDB", manifest["format"])));
    }
    let listed = manifest["files"].as_array().cloned().unwrap_or_default();
    for f in &listed {
        let name = f["path"].as_str().unwrap_or("");
        let Some((_, size, sha)) = found.iter().find(|(n, _, _)| n == name) else {
            return Err(bad(archive, format!("{name} is missing")));
        };
        if Some(*size) != f["size"].as_u64() || Some(sha.as_str()) != f["sha256"].as_str() {
            return Err(bad(archive, format!("{name} does not match its SHA-256")));
        }
    }
    if found.len() != listed.len() + 1 {
        return Err(bad(archive, "it holds files its manifest does not list"));
    }
    let mut records = 0;
    for f in &listed {
        let name = f["path"].as_str().unwrap_or("");
        if name.ends_with(".kiva") {
            records += check_log(&staging.join(name)).map_err(|e| bad(archive, format!("{name}: {e}")))?;
        }
    }
    checked.lines.push(format!(
        "{} file(s) match their SHA-256; {records} record(s) checked. Made by KivaDB {} on {}.",
        listed.len(),
        manifest["kivadb"].as_str().unwrap_or("?"),
        crate::util::fmt_time(manifest["created"].as_i64().unwrap_or(0))
    ));
    checked.manifest = manifest;
    Ok(checked)
}

/// Holds `<home>/instance.lock` so that KivaDB cannot start on the home
/// being restored into; fails if it is running there.
fn lock_home(home: &Path) -> Result<File> {
    fs::create_dir_all(home)?;
    let lock = fs::OpenOptions::new().create(true).write(true).truncate(false).open(home.join("instance.lock"))?;
    if !crate::util::lock_exclusive(&lock) {
        return Err(KivaError::Locked(format!(
            "KivaDB is running on {}: stop it first (kivadb stop), then restore",
            home.display()
        )));
    }
    Ok(lock)
}

/// True when `dir` is missing or holds nothing but a lock and our unpacking folder.
fn is_empty_dir(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|d| {
            d.flatten().all(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n == "instance.lock" || n.starts_with(".kivadb-restore-")
            })
        })
        .unwrap_or(true)
}

/// Moves (or copies, across disks) a checked file into place.
fn install(from: &Path, to: &Path) -> Result<()> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p)?;
    }
    if fs::rename(from, to).is_err() {
        fs::copy(from, to)?;
    }
    Ok(())
}

fn install_workspace(staging: &Path, name: &str, dir: &Path) -> Result<()> {
    let src = staging.join("workspaces").join(name);
    for f in ["store.kiva", "store.kiva.meta"] {
        if src.join(f).exists() {
            install(&src.join(f), &dir.join(f))?;
        }
    }
    Ok(())
}

/// Restores `archive`. A home backup becomes a new home (`target`, or
/// `home` when empty); a workspace backup becomes a workspace of `home`.
/// Nothing is changed unless the whole archive checks out.
pub fn restore(archive: &Path, home: &Path, opts: &RestoreOptions) -> Result<Vec<String>> {
    if opts.check_only {
        let tmp = std::env::temp_dir();
        let checked = check(archive, &tmp)?;
        let mut lines = checked.lines.clone();
        lines.push(format!("The archive is sound; nothing was restored ({} backup).", checked.manifest["scope"]));
        return Ok(lines);
    }
    // Unpack inside the destination: always writable there (a container's
    // /data, say), and moving files into place is then a rename.
    let near = opts.target.clone().unwrap_or_else(|| home.to_path_buf());
    let created = !near.exists();
    let checked = check(archive, &near).inspect_err(|_| {
        // A refused archive leaves no empty folder behind.
        if created {
            let _ = fs::remove_dir(&near);
        }
    })?;
    let mut lines = checked.lines.clone();
    let staging = checked.staging.clone();
    let names: Vec<String> = checked.manifest["workspaces"]
        .as_array()
        .map(|a| a.iter().filter_map(|w| w["name"].as_str().map(String::from)).collect())
        .unwrap_or_default();

    if checked.manifest["scope"] == "home" {
        if opts.rename.is_some() {
            return Err(KivaError::Invalid("--as renames a workspace backup, not a home backup".into()));
        }
        let target = opts.target.clone().unwrap_or_else(|| home.to_path_buf());
        let _lock = lock_home(&target)?;
        if !is_empty_dir(&target) {
            return Err(KivaError::Exists(format!(
                "{} is not empty: restore into a new folder with --target, or move this one away first",
                target.display()
            )));
        }
        install(&staging.join("kiva.conf"), &target.join("kiva.conf"))?;
        for f in ["system.kiva", "system.kiva.meta"] {
            if staging.join("system").join(f).exists() {
                install(&staging.join("system").join(f), &target.join("system").join(f))?;
            }
        }
        if staging.join("audit.log").exists() {
            install(&staging.join("audit.log"), &target.join("audit.log"))?;
        }
        if let Ok(entries) = fs::read_dir(staging.join("tls")) {
            for e in entries.flatten() {
                install(&e.path(), &target.join("tls").join(e.file_name()))?;
            }
        }
        // Workspaces go inside the new home, wherever they were before.
        let mut cfg = crate::config::Config::load(&target)?;
        for name in &names {
            let dir = if name == "default" { target.join("data") } else { target.join("workspaces").join(name) };
            install_workspace(&staging, name, &dir)?;
            if name == "default" {
                cfg.set("storage", "default_workspace", &dir.display().to_string())?;
            } else {
                cfg.set("workspaces", name, &dir.display().to_string())?;
            }
        }
        lines.push(format!("Home restored to {} (workspaces: {}).", target.display(), names.join(", ")));
        lines.push(format!("Start it with: kivadb --home \"{}\" start", target.display()));
    } else {
        let original = names.first().cloned().ok_or_else(|| bad(archive, "it names no workspace"))?;
        let name = opts.rename.clone().unwrap_or_else(|| original.clone());
        let _lock = lock_home(home)?;
        let mut cfg = crate::config::Config::load(home)?;
        if cfg.workspace_dir(&name).is_some() {
            return Err(KivaError::Exists(format!(
                "workspace '{name}' already exists in {}: restore it under another name with --as <name>",
                home.display()
            )));
        }
        let valid = !name.is_empty()
            && name.len() <= 64
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
        if !valid {
            return Err(KivaError::Invalid("workspace names may only contain letters, digits, '_' and '-'".into()));
        }
        let dir = opts.target.clone().unwrap_or_else(|| home.join("workspaces").join(&name));
        if !is_empty_dir(&dir) {
            return Err(KivaError::Exists(format!(
                "{} is not empty: choose another folder with --target",
                dir.display()
            )));
        }
        install_workspace(&staging, &original, &dir)?;
        cfg.set("workspaces", &name, &dir.display().to_string())?;
        lines.push(format!("Workspace '{original}' restored as '{name}' in {}.", dir.display()));
    }
    if let Some(id) = checked.manifest["encryption_key"].as_str() {
        lines.push(format!("This data is encrypted with key {id}: start KivaDB with that key."));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_is_checked_record_by_record() {
        let dir = std::env::temp_dir().join(format!("kivadb-checklog-{:x}", crate::util::random_u64()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.kiva");
        let mut data = crate::format::file_header(7, crate::format::FORMAT_V1).to_vec();
        data.extend(crate::format::encode_record("a", b"1", 1, 0, 0, 0));
        data.extend(crate::format::encode_record("b", b"2", 1, 0, 0, 0));
        fs::write(&path, &data).unwrap();
        assert_eq!(check_log(&path), Ok(2));
        let last = data.len() - 1;
        data[last] ^= 1;
        fs::write(&path, &data).unwrap();
        assert!(check_log(&path).unwrap_err().contains("checksum mismatch"));
        fs::write(&path, &data[..data.len() - 3]).unwrap();
        assert!(check_log(&path).unwrap_err().contains("truncated"));
        let _ = fs::remove_dir_all(&dir);
    }
}
