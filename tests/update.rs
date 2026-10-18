// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! `kivadb update` against a local stand-in for the GitHub releases API.

mod common;

use common::TempDir;
use kivadb::update::{asset_name, sha256_hex};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

/// Serves fixed bodies by path, like the GitHub API and its downloads.
fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes = Arc::new(routes);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap_or(0);
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                // Skip the headers.
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let mut stream = stream;
                match routes.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ =
                            write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
        }
    });
    base
}

/// The release archive for this system, holding `binary` under its usual name.
fn archive(name: &str, binary: &[u8]) -> Vec<u8> {
    if name.ends_with(".zip") {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file("kivadb.exe", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(binary).unwrap();
        zip.finish().unwrap();
        buf.into_inner()
    } else {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(binary.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "kivadb", binary).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }
}

struct Setup {
    _dir: TempDir,
    exe: PathBuf,
    home: PathBuf,
    url: String,
}

/// A copy of kivadb to update, with a marker appended so that its
/// replacement is visible, and a fake release `tag`.
fn setup(tag: &str, good_checksum: bool) -> Option<Setup> {
    let asset = asset_name(std::env::consts::OS, std::env::consts::ARCH)?;
    let dir = TempDir::new("update");
    let original = std::fs::read(env!("CARGO_BIN_EXE_kivadb")).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "kivadb.exe" } else { "kivadb" });
    let mut marked = original.clone();
    marked.extend_from_slice(b"old-version-marker");
    std::fs::write(&exe, &marked).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let packed = archive(asset, &original);
    let hash = if good_checksum { sha256_hex(&packed) } else { "0".repeat(64) };
    // The JSON names the download addresses: start that server first.
    let files = serve(vec![
        (format!("/download/{asset}"), packed),
        ("/download/SHA256SUMS.txt".into(), format!("{hash}  {asset}\n").into_bytes()),
    ]);
    let release = serde_json::json!({
        "tag_name": tag,
        "assets": [
            {"name": asset, "browser_download_url": format!("{files}/download/{asset}")},
            {"name": "SHA256SUMS.txt", "browser_download_url": format!("{files}/download/SHA256SUMS.txt")},
        ],
    });
    let api = serve(vec![("/latest".into(), release.to_string().into_bytes())]);
    let home = dir.path().join("home");
    Some(Setup { exe, home, url: format!("{api}/latest"), _dir: dir })
}

fn run(s: &Setup, args: &[&str]) -> Output {
    Command::new(&s.exe).args(args).arg("--home").arg(&s.home).env("KIVADB_RELEASES_URL", &s.url).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).unwrap().len()
}

#[test]
fn update_installs_a_newer_release() {
    let Some(s) = setup("v99.0.0", true) else { return };
    let before = size(&s.exe);
    let out = run(&s, &["update"]);
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("New version available: 99.0.0"), "{t}");
    assert!(t.contains("Checksum verified"), "{t}");
    assert!(t.contains("KivaDB successfully updated to version"), "{t}");
    assert_eq!(size(&s.exe), before - b"old-version-marker".len() as u64, "the executable was not replaced");
    // The cache remembers the latest version for the discreet check.
    let cache = std::fs::read_to_string(s.home.join("version_check.json")).unwrap();
    assert!(cache.contains("99.0.0"), "{cache}");

    // The replaced executable works, and cleans up what the update left.
    let again = Command::new(&s.exe).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&again.stdout).starts_with("KivaDB "));
    let old = s.exe.with_file_name(format!("{}.old", s.exe.file_name().unwrap().to_string_lossy()));
    assert!(!old.exists(), "the previous executable was left behind");
}

#[test]
fn a_bad_checksum_changes_nothing() {
    let Some(s) = setup("v99.0.0", false) else { return };
    let before = std::fs::read(&s.exe).unwrap();
    let out = run(&s, &["update"]);
    let t = text(&out);
    assert!(!out.status.success(), "{t}");
    assert!(t.contains("checksum") && t.contains("nothing was changed"), "{t}");
    assert_eq!(std::fs::read(&s.exe).unwrap(), before);
}

#[test]
fn check_only_tells_without_installing() {
    let Some(s) = setup("v99.0.0", true) else { return };
    let before = size(&s.exe);
    let out = run(&s, &["update", "--check"]);
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("New version available: 99.0.0") && t.contains("Run 'kivadb update'"), "{t}");
    assert_eq!(size(&s.exe), before);
}

#[test]
fn an_older_release_is_not_installed() {
    let Some(s) = setup("v0.9.0", true) else { return };
    let before = size(&s.exe);
    let out = run(&s, &["update"]);
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("is up to date"), "{t}");
    assert_eq!(size(&s.exe), before);
}

#[test]
fn an_unreachable_server_is_explained() {
    let Some(mut s) = setup("v99.0.0", true) else { return };
    // Nothing listens on port 1.
    s.url = "http://127.0.0.1:1/latest".into();
    let out = run(&s, &["update"]);
    let t = text(&out);
    assert!(!out.status.success());
    assert!(t.contains("cannot reach GitHub"), "{t}");
}
