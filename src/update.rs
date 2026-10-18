// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Keeping KivaDB up to date: a discreet check for new releases, and
//! `kivadb update`, which downloads the executable for this system from the
//! GitHub releases, verifies it and puts it in place of the running one.

use crate::instance::VERSION;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where the latest release is described. `KIVADB_RELEASES_URL` replaces it
/// (a mirror, or a test server).
pub const RELEASES_URL: &str = "https://api.github.com/repos/fomadev/kiva-db/releases/latest";

/// How long a version check stays valid.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

/// Largest archive accepted: a KivaDB executable is a few megabytes.
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

fn releases_url() -> String {
    std::env::var("KIVADB_RELEASES_URL").ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| RELEASES_URL.into())
}

// ---------------------------------------------------------------- versions

/// `v1.2.3`, `1.2.3` or `1.2.3-beta` -> (1, 2, 3). Pre-release suffixes are ignored.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(v)
}

/// Whether `candidate` is a later version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

// ---------------------------------------------------------------- platform

/// Name of the release archive for this system, if one is published.
pub fn asset_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("kivadb_win_x64.zip"),
        ("linux", "x86_64") => Some("kivadb_linux_x64.tar.gz"),
        ("macos", "aarch64") => Some("kivadb_mac_arm64.tar.gz"),
        _ => None,
    }
}

/// Inside a container, the image is what gets updated, not the executable.
pub fn in_container() -> bool {
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

// ---------------------------------------------------------------- GitHub

pub struct Release {
    pub version: String,
    pub assets: Vec<(String, String)>,
}

impl Release {
    fn asset_url(&self, name: &str) -> Option<&str> {
        self.assets.iter().find(|(n, _)| n == name).map(|(_, u)| u.as_str())
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(timeout.min(Duration::from_secs(10)))
        .timeout(timeout)
        .user_agent(&format!("kivadb/{VERSION}"))
        .build()
}

/// Turns a failed request into a sentence that says what to do.
fn explain(err: ureq::Error, what: &str) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let remaining = resp.header("x-ratelimit-remaining").map(str::to_owned);
            let reset = resp.header("x-ratelimit-reset").and_then(|r| r.parse::<i64>().ok());
            if (code == 403 || code == 429) && remaining.as_deref() == Some("0") {
                let at = reset
                    .and_then(|r| chrono::DateTime::from_timestamp(r, 0))
                    .map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
                    .unwrap_or_else(|| "a few minutes".into());
                format!(
                    "GitHub limits anonymous requests and the limit is reached for this network. Try again after {at}."
                )
            } else if code == 404 {
                format!("{what}: not found (HTTP 404). No release may have been published yet.")
            } else {
                format!("{what}: the server answered HTTP {code}.")
            }
        }
        ureq::Error::Transport(t) => {
            format!("cannot reach GitHub ({what}). Check the Internet connection or the proxy. Details: {t}")
        }
    }
}

/// Reads the description of the latest release.
pub fn latest_release(timeout: Duration) -> Result<Release, String> {
    let body = agent(timeout)
        .get(&releases_url())
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| explain(e, "reading the latest release"))?
        .into_string()
        .map_err(|e| format!("reading the latest release: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
    let tag = json["tag_name"].as_str().ok_or("unexpected answer from GitHub: no tag_name")?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if parse_version(&version).is_none() {
        return Err(format!("the latest release has an unexpected version name: {tag}"));
    }
    let assets = json["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some((a["name"].as_str()?.to_string(), a["browser_download_url"].as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release { version, assets })
}

fn download(url: &str, what: &str) -> Result<Vec<u8>, String> {
    let resp = agent(Duration::from_secs(300)).get(url).call().map_err(|e| explain(e, what))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(MAX_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("downloading {what}: {e}"))?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err(format!("{what} is larger than expected; download refused."));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------- verification

/// The expected SHA-256 of `file` in a `SHA256SUMS.txt` (`<hash>  <name>` lines).
pub fn expected_sha256(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == file && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The `kivadb` executable inside a release archive.
pub fn extract_binary(archive: &[u8], archive_name: &str) -> Result<Vec<u8>, String> {
    let wanted = if archive_name.ends_with(".zip") { "kivadb.exe" } else { "kivadb" };
    let mut out = Vec::new();
    if archive_name.ends_with(".zip") {
        let mut zip =
            zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| format!("damaged archive: {e}"))?;
        let mut file = zip.by_name(wanted).map_err(|_| format!("{wanted} is missing from the archive"))?;
        file.read_to_end(&mut out).map_err(|e| format!("damaged archive: {e}"))?;
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        let entries = tar.entries().map_err(|e| format!("damaged archive: {e}"))?;
        for entry in entries {
            let mut entry = entry.map_err(|e| format!("damaged archive: {e}"))?;
            let path = entry.path().map_err(|e| format!("damaged archive: {e}"))?.into_owned();
            if path.file_name().is_some_and(|n| n == wanted) {
                entry.read_to_end(&mut out).map_err(|e| format!("damaged archive: {e}"))?;
                break;
            }
        }
        if out.is_empty() {
            return Err(format!("{wanted} is missing from the archive"));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- replacement

/// Puts `new_exe` (a file next to `target`) in place of `target`.
fn replace(target: &Path, new_exe: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // A running executable cannot be overwritten on Windows, but it can
        // be renamed: move it aside, then move the new one in. The old file
        // is deleted by the next run of KivaDB.
        let old = old_path(target);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(target, &old)?;
        if let Err(e) = std::fs::rename(new_exe, target) {
            let _ = std::fs::rename(&old, target);
            return Err(e);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        // A rename within one directory is atomic: the file is either the old
        // or the new one, and running processes keep the old one.
        std::fs::rename(new_exe, target)
    }
}

fn old_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    target.with_file_name(name)
}

/// Deletes what a previous update left on Windows. Called at every start.
pub fn clean_previous() {
    if cfg!(windows) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::fs::remove_file(old_path(&exe));
        }
    }
}

fn permission_hint(target: &Path) -> String {
    if cfg!(windows) {
        format!("cannot replace {}: permission denied. Run the terminal as administrator, or install KivaDB in a folder you own.", target.display())
    } else {
        format!("cannot replace {}: permission denied. Run: sudo kivadb update", target.display())
    }
}

// ---------------------------------------------------------------- the command

/// `kivadb update [--check]`. Returns the process exit code.
pub fn run(check_only: bool, home: &Path) -> i32 {
    match update(check_only, home) {
        Ok(()) => 0,
        Err(msg) => {
            eprintln!("Error: {msg}");
            1
        }
    }
}

fn update(check_only: bool, home: &Path) -> Result<(), String> {
    if in_container() {
        return Err("KivaDB runs in a container: update the image instead, e.g. docker pull fomadev0/kiva-db:latest, then recreate the container.".into());
    }
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    println!("Checking for a new version of KivaDB...");
    let release = latest_release(Duration::from_secs(20))?;
    save_check(home, &release.version);
    if !is_newer(&release.version, VERSION) {
        println!("KivaDB {VERSION} is up to date (latest release: {}).", release.version);
        return Ok(());
    }
    println!("New version available: {} (current: {VERSION}).", release.version);
    if check_only {
        println!("Run 'kivadb update' to install it.");
        return Ok(());
    }

    let asset = asset_name(os, arch).ok_or_else(|| {
        format!("no ready-made executable is published for {os}/{arch}. Build KivaDB from the source code instead (see DOCUMENTATION.md, section 1.3).")
    })?;
    let url =
        release.asset_url(asset).ok_or_else(|| format!("the release {} has no file {asset}.", release.version))?;
    let target = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("cannot locate the kivadb executable: {e}"))?;
    let dir = target.parent().ok_or("cannot locate the folder of the kivadb executable")?.to_path_buf();

    // Before downloading anything, make sure the executable can be replaced.
    // Windows only runs files ending in .exe.
    let staged = dir.join(if cfg!(windows) { ".kivadb-new.exe" } else { ".kivadb-new" });
    std::fs::write(&staged, b"").map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => permission_hint(&target),
        _ => format!("cannot write in {}: {e}", dir.display()),
    })?;

    let result = (|| {
        println!("Downloading {asset}...");
        let archive = download(url, asset)?;
        match release.asset_url("SHA256SUMS.txt") {
            Some(sums_url) => {
                let sums = String::from_utf8_lossy(&download(sums_url, "SHA256SUMS.txt")?).into_owned();
                let expected = expected_sha256(&sums, asset)
                    .ok_or_else(|| format!("SHA256SUMS.txt has no entry for {asset}; update refused."))?;
                let actual = sha256_hex(&archive);
                if actual != expected {
                    return Err(format!("the checksum of {asset} does not match SHA256SUMS.txt (expected {expected}, got {actual}). The download was damaged or altered; nothing was changed."));
                }
                println!("Checksum verified (SHA-256).");
            }
            None => println!("[WARN] this release publishes no SHA256SUMS.txt: the download cannot be verified."),
        }
        let binary = extract_binary(&archive, asset)?;
        std::fs::write(&staged, &binary).map_err(|e| format!("cannot write the new executable: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("cannot make the new executable runnable: {e}"))?;
        }
        // The new file must really be KivaDB before it replaces anything.
        let out = std::process::Command::new(&staged)
            .arg("--version")
            .env("KIVADB_NO_UPDATE_CHECK", "1")
            .output()
            .map_err(|e| format!("the downloaded executable does not run on this system: {e}"))?;
        let reported = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || !reported.starts_with("KivaDB ") {
            return Err(format!(
                "the downloaded executable did not answer as KivaDB ({reported:?}); nothing was changed."
            ));
        }
        replace(&target, &staged).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => permission_hint(&target),
            _ => format!("cannot replace {}: {e}", target.display()),
        })?;
        Ok(reported.trim_start_matches("KivaDB ").to_string())
    })();
    let _ = std::fs::remove_file(&staged);
    let installed = result?;

    println!("KivaDB successfully updated to version {installed}!");
    if home.join("instance.port").exists() {
        println!("KivaDB is running with the previous version: restart it with 'kivadb stop' then 'kivadb start'.");
    }
    Ok(())
}

// ---------------------------------------------------------------- discreet check

fn cache_path(home: &Path) -> PathBuf {
    home.join("version_check.json")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn save_check(home: &Path, latest: &str) {
    let json = serde_json::json!({"checked_at": now(), "latest": latest});
    let _ = std::fs::create_dir_all(home);
    let _ = std::fs::write(cache_path(home), json.to_string());
}

/// The last check: (when, latest version).
fn load_check(home: &Path) -> Option<(u64, String)> {
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(cache_path(home)).ok()?).ok()?;
    Some((json["checked_at"].as_u64()?, json["latest"].as_str()?.to_string()))
}

/// The "new version available" notice. [`Notice::begin`] starts the check in
/// the background (at most once a day); [`Notice::finish`] prints the result
/// if it is known by then, without ever waiting for the network.
pub struct Notice {
    latest: Arc<Mutex<Option<String>>>,
}

impl Notice {
    /// Shown only to a person at a terminal: scripts, services and tests never
    /// see it and never contact GitHub. `KIVADB_NO_UPDATE_CHECK=1` turns it off.
    pub fn begin(home: &Path) -> Option<Notice> {
        use std::io::IsTerminal;
        let disabled = std::env::var("KIVADB_NO_UPDATE_CHECK").is_ok_and(|v| !v.is_empty() && v != "0");
        if disabled || !std::io::stderr().is_terminal() || in_container() {
            return None;
        }
        let cached = load_check(home);
        let latest = Arc::new(Mutex::new(cached.as_ref().map(|(_, v)| v.clone())));
        let fresh = cached.is_some_and(|(at, _)| now().saturating_sub(at) < CHECK_EVERY.as_secs());
        if !fresh {
            let (slot, home) = (latest.clone(), home.to_path_buf());
            std::thread::spawn(move || {
                if let Ok(r) = latest_release(Duration::from_secs(4)) {
                    save_check(&home, &r.version);
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r.version);
                } else if let Some((_, v)) = load_check(&home) {
                    // Offline: try again tomorrow rather than at every command.
                    save_check(&home, &v);
                }
            });
        }
        Some(Notice { latest })
    }

    pub fn finish(&self) {
        let latest = self.latest.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(v) = latest.filter(|v| is_newer(v, VERSION)) {
            eprintln!();
            eprintln!("A new version of KivaDB is available: {v} (current: {VERSION})");
            eprintln!("Run 'kivadb update' to install the latest release.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.10.0-beta.1"), Some((1, 10, 0)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
        assert_eq!(parse_version("1.2.x"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(is_newer("v2.0.0", "1.99.99"));
        assert!(!is_newer("1.0.1", "1.0.1"));
        assert!(!is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("garbage", "1.0.1"));
    }

    #[test]
    fn each_published_system_has_its_archive() {
        assert_eq!(asset_name("windows", "x86_64"), Some("kivadb_win_x64.zip"));
        assert_eq!(asset_name("linux", "x86_64"), Some("kivadb_linux_x64.tar.gz"));
        assert_eq!(asset_name("macos", "aarch64"), Some("kivadb_mac_arm64.tar.gz"));
        assert_eq!(asset_name("macos", "x86_64"), None);
        assert_eq!(asset_name("linux", "aarch64"), None);
    }

    #[test]
    fn checksums_are_read_from_sha256sums() {
        let sums = format!("{}  kivadb_win_x64.zip\n{} *kivadb_linux_x64.tar.gz\n", "a".repeat(64), "B".repeat(64));
        assert_eq!(expected_sha256(&sums, "kivadb_win_x64.zip"), Some("a".repeat(64)));
        assert_eq!(expected_sha256(&sums, "kivadb_linux_x64.tar.gz"), Some("b".repeat(64)));
        assert_eq!(expected_sha256(&sums, "kivadb_mac_arm64.tar.gz"), None);
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
