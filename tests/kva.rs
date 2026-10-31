// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! `.kva` files and commands written over several lines.

mod common;

use common::TempDir;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn kivadb(home: &std::path::Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .arg("--home")
        .arg(home)
        .args(args)
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

const EXAMPLE: &str = r#"# strings
set nameA "Fordi Malanda"
set nameB "Nathan Malanda"

# numbers
set number ageM 24

set user:1 {
  "nom": "Fordi",
  "age": 22
}

set classe:1 [
  "1er",
  "2eme",
  "3eme"
]

--- and so on
get user:1.nom and classe:1[2]
"#;

#[test]
fn a_kva_file_runs_command_by_command() {
    let dir = TempDir::new("kva_run");
    let file = dir.path().join("example.kva");
    std::fs::write(&file, EXAMPLE).unwrap();
    let home = dir.path().join("home");
    let o = kivadb(&home, &["run", file.to_str().unwrap()], None);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    assert!(text.contains("OK: user:1 saved.") && text.contains("OK: classe:1 saved."), "{text}");
    assert!(text.contains("user:1.nom: Fordi [string]") && text.contains("classe:1[2]: 3eme [string]"), "{text}");
    let check = kivadb(&home, &["-e", "get user:1.age and ageM"], None);
    assert_eq!(out(&check), "user:1.age: 22 [number]\nageM: 24 [number]\n");
}

#[test]
fn files_saved_by_windows_powershell_run_as_well() {
    let dir = TempDir::new("kva_bom");
    let home = dir.path().join("home");
    let text = "set ville \"Élisabethville\"\r\nset bad oops\r\n";
    // UTF-8 with a byte order mark, and UTF-16 (Windows PowerShell 5.1 with `>`).
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend(text.as_bytes());
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    for (name, bytes) in [("bom.kva", bom), ("utf16.kva", utf16)] {
        let file = dir.path().join(name);
        std::fs::write(&file, bytes).unwrap();
        let o = kivadb(&home, &["run", file.to_str().unwrap(), "--keep-going"], None);
        assert!(out(&o).contains("OK: ville saved."), "{name}: {}", out(&o));
        // The first line is a command, and the lines keep their numbers.
        assert!(!err(&o).contains("Unknown command"), "{name}: {}", err(&o));
        assert!(err(&o).contains(&format!("{name}:2: Error: String values like 'oops'")), "{name}: {}", err(&o));
        assert_eq!(out(&kivadb(&home, &["-e", "get ville"], None)), "ville: Élisabethville [string]\n");
        kivadb(&home, &["-e", "del ville"], None);
    }
    // Bytes that are not text are refused before anything runs.
    let file = dir.path().join("binary.kva");
    std::fs::write(&file, [0x73, 0x65, 0x74, 0x20, 0xC3, 0x28]).unwrap();
    let o = kivadb(&home, &["run", file.to_str().unwrap()], None);
    assert!(err(&o).contains("is not UTF-8 text"), "{}", err(&o));
}

#[test]
fn a_failing_command_is_reported_with_its_line_and_stops_the_file() {
    let dir = TempDir::new("kva_error");
    let file = dir.path().join("broken.kva");
    std::fs::write(&file, "set ok 1\n\nset bad oops\nset after 2\n").unwrap();
    let home = dir.path().join("home");
    let o = kivadb(&home, &["run", file.to_str().unwrap()], None);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("broken.kva:3: Error: String values like 'oops' must be quoted"), "{}", err(&o));
    assert!(out(&kivadb(&home, &["-e", "has after"], None)).contains("does not exist"));

    // --keep-going runs the rest and still reports the failure.
    let o = kivadb(&home, &["run", file.to_str().unwrap(), "--keep-going"], None);
    assert_eq!(o.status.code(), Some(1));
    assert!(out(&o).contains("OK: after saved."));

    // A document never closed is refused before anything runs.
    std::fs::write(&file, "set a 1\nset doc {\n  \"x\": 1\n").unwrap();
    let o = kivadb(&home, &["run", file.to_str().unwrap()], None);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("broken.kva:2: this command opens a { or [ that is never closed"), "{}", err(&o));
    assert!(out(&kivadb(&home, &["-e", "has a"], None)).contains("does not exist"));
}

#[test]
fn a_file_with_if_clauses_runs_again_without_errors() {
    let dir = TempDir::new("kva_again");
    let file = dir.path().join("seed.kva");
    std::fs::write(
        &file,
        "del old if exists\nset name \"Fordi\" if not exists\nset user:1 {\n  \"age\": 22\n} if not exists\n",
    )
    .unwrap();
    let home = dir.path().join("home");
    let first = kivadb(&home, &["run", file.to_str().unwrap()], None);
    assert!(first.status.success(), "{}", err(&first));
    assert_eq!(out(&first), "OK: old skipped (does not exist)\nOK: name saved.\nOK: user:1 saved.\n");

    let again = kivadb(&home, &["run", file.to_str().unwrap()], None);
    assert!(again.status.success(), "{}", err(&again));
    assert_eq!(
        out(&again),
        "OK: old skipped (does not exist)\nOK: name skipped (already exists)\nOK: user:1 skipped (already exists)\n"
    );
}

#[test]
fn commands_go_on_over_several_lines_until_their_document_closes() {
    let dir = TempDir::new("kva_lines");
    let home = dir.path().join("home");
    let o = kivadb(
        &home,
        &[],
        Some("set user:1 {\n  \"name\": \"Alice\",\n  \"tags\": [\"a\",\n           \"b\"]\n}\nget user:1.tags[1]\n"),
    );
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "OK: user:1 saved.\nuser:1.tags[1]: b [string]\n");

    // `source` runs a file from the shell; an error in piped commands sets the exit code.
    let file = dir.path().join("more.kva");
    std::fs::write(&file, "set from_file [\n  1,\n  2\n]\n").unwrap();
    let o = kivadb(&home, &[], Some(&format!("source \"{}\"\nget from_file[1]\nset broken oops\n", file.display())));
    assert_eq!(o.status.code(), Some(1));
    assert!(out(&o).contains("OK: from_file saved.\nfrom_file[1]: 2 [number]"), "{}", out(&o));
}

#[test]
fn a_kva_file_reads_and_shows_dates_in_iso_whatever_the_formats_of_its_session() {
    let dir = TempDir::new("kva_formats");
    let home = dir.path().join("home");
    let o = kivadb(&home, &["-e", r#"config set time.date_format "DD-MM-YYYY""#], None);
    assert!(o.status.success(), "{}", err(&o));
    let o = kivadb(&home, &["-e", r#"set date shell "25-12-2000""#, "-e", "get shell"], None);
    assert_eq!(out(&o), "OK: shell saved.\nshell: 25-12-2000 [date]\n", "{}", err(&o));

    // The same file gives the same result for everyone: ISO, unless it sets a format itself.
    let file = dir.path().join("dates.kva");
    std::fs::write(
        &file,
        "set date d \"2026-10-31\"\nget d\nformat date \"MM/DD/YYYY\"\nset date e \"12/24/2026\"\nget e\n",
    )
    .unwrap();
    let o = kivadb(&home, &["run", file.to_str().unwrap()], None);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    assert!(text.starts_with("OK: d saved.\nd: 2026-10-31 [date]\ndate:     MM/DD/YYYY (from this session)"), "{text}");
    assert!(text.ends_with("OK: e saved.\ne: 12/24/2026 [date]\n"), "{text}");
    assert_eq!(text.lines().count(), 5, "only the format changed is shown: {text}");

    // `source` in the shell, then the session's formats come back.
    let o = kivadb(&home, &[], Some(&format!("source \"{}\"\nget shell\n", file.display())));
    assert!(out(&o).ends_with("shell: 25-12-2000 [date]\n"), "{}", out(&o));
}
