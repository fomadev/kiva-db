// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The examples of DOCUMENTATION.md, run (NATIVE_DOCUMENTS.md 27.5).
//!
//! Each `text` block of the chapters below whose lines start with `kiva>` is
//! run in a shell session; the blocks of one chapter run one after the other
//! in the same new home, in the time zone of the examples (Africa/Kinshasa).
//! The answer of a command is compared with the lines under it:
//!
//! * `...` in a line stands for any text (a time, an identifier, a size);
//! * a line that is only `...` stands for any number of lines;
//! * a command with no line under it is run, and its answer not compared;
//! * a time of the clock without an offset (`2026-10-12 09:44:54`, as `scan`,
//!   `users` and `audit` show them) stands for any such time.
//!
//! What an example needs and the manual says in words (the documents of the
//! previous section, an account) is written in a comment just before its
//! block, one command a line, run without showing its answers:
//!
//! ```text
//! <!-- setup
//! set user:1 {"name": "Alice"}
//! -->
//! ```
//!
//! A block that cannot run here (a server, a second machine, a file outside
//! the home, the network) is preceded by `<!-- not run: <reason> -->`. The
//! number of blocks left out is checked, so that it only changes on purpose.
//! The folder of the test's home is shown as the manual's, the default home
//! of Windows.

mod common;

use common::TempDir;
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use regex::Regex;
use std::sync::LazyLock;

/// The chapters whose examples are run.
const CHAPTERS: &[u32] = &[5, 6, 9, 10, 11, 13, 14, 16, 26];

/// The blocks of those chapters marked `<!-- not run -->`.
const NOT_RUN: usize = 8;

/// The home the manual shows.
const HOME: &str = r"C:\Users\you\AppData\Roaming\KivaDB";

struct Command {
    line: usize,
    input: String,
    expected: Vec<String>,
}

struct Block {
    chapter: u32,
    line: usize,
    setup: Vec<String>,
    commands: Vec<Command>,
}

/// The blocks of the manual: (blocks to run, blocks left out).
fn blocks(manual: &str) -> (Vec<Block>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0;
    let mut chapter = 0;
    let lines: Vec<&str> = manual.lines().collect();
    let mut i = 0;
    let mut setup = Vec::new();
    while i < lines.len() {
        let l = lines[i];
        if let Some(rest) = l.strip_prefix("## Chapter ") {
            chapter = rest.split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap_or(0);
        }
        if l.trim_end() == "<!-- setup" {
            let end = (i + 1..lines.len()).find(|&j| lines[j].trim_end() == "-->").expect("a closed setup");
            setup.extend(lines[i + 1..end].iter().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()));
            i = end + 1;
            continue;
        }
        if l.trim_end() != "```text" || !CHAPTERS.contains(&chapter) {
            i += 1;
            continue;
        }
        let start = i;
        let end = (i + 1..lines.len()).find(|&j| lines[j].trim_end() == "```").expect("a closed block");
        let body = &lines[i + 1..end];
        i = end + 1;
        if !body.iter().any(|l| l.starts_with("kiva> ")) {
            continue;
        }
        let before = lines[..start].iter().rev().find(|l| !l.trim().is_empty()).copied().unwrap_or("");
        if before.trim_start().starts_with("<!-- not run") {
            skipped += 1;
            setup.clear();
            continue;
        }
        let mut commands: Vec<Command> = Vec::new();
        for (k, l) in body.iter().enumerate() {
            if let Some(cmd) = l.strip_prefix("kiva> ") {
                commands.push(Command { line: start + 2 + k, input: cmd.to_string(), expected: Vec::new() });
            } else if let (Some(c), Some(more)) = (commands.last_mut(), l.trim_start().strip_prefix("...")) {
                if c.expected.is_empty() && l.starts_with("   ...") {
                    // A command typed on several lines.
                    c.input.push('\n');
                    c.input.push_str(more.trim_start());
                    continue;
                }
                c.expected.push(l.to_string());
            } else if let Some(c) = commands.last_mut() {
                c.expected.push(l.to_string());
            }
        }
        for c in &mut commands {
            while c.expected.last().is_some_and(|l| l.trim().is_empty()) {
                c.expected.pop();
            }
        }
        out.push(Block { chapter, line: start + 1, setup: std::mem::take(&mut setup), commands });
    }
    (out, skipped)
}

/// A time of the clock with no offset after it: when something happened.
static CLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4}-\d\d-\d\d \d\d:\d\d:\d\d(\.\d+)?( [|\]]|\]|$)").unwrap());

/// Does `got` match `want`, where `...` stands for any text?
fn line_matches(want: &str, got: &str) -> bool {
    let (want, got) = (CLOCK.replace_all(want, "<clock>$2"), CLOCK.replace_all(got, "<clock>$2"));
    let (want, got) = (want.as_ref(), got.as_ref());
    let parts: Vec<&str> = want.split("...").collect();
    if parts.len() == 1 {
        return want.trim_end() == got.trim_end();
    }
    let got = got.trim_end();
    let mut rest = got;
    for (n, p) in parts.iter().enumerate() {
        let p = if n == parts.len() - 1 { p.trim_end() } else { p };
        if n == 0 {
            let Some(r) = rest.strip_prefix(p) else { return false };
            rest = r;
        } else if n == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(at) = rest.find(p) else { return false };
            rest = &rest[at + p.len()..];
        }
    }
    true
}

/// Do the lines of an answer match those of the manual?
fn lines_match(want: &[String], got: &[&str]) -> bool {
    match want.split_first() {
        None => got.is_empty(),
        Some((w, rest)) if w.trim() == "..." => (0..=got.len()).any(|k| lines_match(rest, &got[k..])),
        Some((w, rest)) => got.split_first().is_some_and(|(g, more)| line_matches(w, g) && lines_match(rest, more)),
    }
}

/// An answer with the folder of the test's home shown as the manual's.
fn as_in_manual(text: &str, home: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(home) {
        out.push_str(&rest[..at]);
        out.push_str(HOME);
        rest = &rest[at + home.len()..];
        let end = rest.find(|c: char| c.is_whitespace() || matches!(c, ',' | ')' | '\'' | '"')).unwrap_or(rest.len());
        let path = rest[..end].strip_suffix('.').unwrap_or(&rest[..end]);
        out.push_str(&path.replace('/', "\\"));
        rest = &rest[path.len()..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_examples_of_the_manual_give_the_answers_it_shows() {
    let manual = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/DOCUMENTATION.md")).unwrap();
    let (blocks, skipped) = blocks(&manual);
    let mut failures = Vec::new();
    let mut run = 0;
    for &chapter in CHAPTERS {
        let dir = TempDir::new(&format!("manual_{chapter}"));
        let inst = Instance::open(dir.path(), Options::default()).unwrap();
        let mut s = Session::new(inst.clone(), true, None);
        s.execute("config set time.zone Africa/Kinshasa");
        let home = dir.path().display().to_string();
        for b in blocks.iter().filter(|b| b.chapter == chapter) {
            for c in &b.setup {
                let out = s.execute(c);
                if out.has_err() {
                    failures.push(format!("setup of the block at DOCUMENTATION.md:{}: {c}\n  {}", b.line, out.text()));
                }
            }
            for c in &b.commands {
                run += 1;
                let text = as_in_manual(&s.execute(&c.input).text(), &home);
                if c.expected.is_empty() {
                    continue;
                }
                let got: Vec<&str> = text.lines().collect();
                if !lines_match(&c.expected, &got) {
                    failures.push(format!(
                        "DOCUMENTATION.md:{} (chapter {chapter}, block at {}): kiva> {}\n  manual:\n    {}\n  got:\n    {}",
                        c.line,
                        b.line,
                        c.input,
                        c.expected.join("\n    "),
                        got.join("\n    ")
                    ));
                }
            }
        }
        drop(s);
        inst.shutdown();
    }
    assert!(run > 200, "only {run} commands found: has the manual's layout changed?");
    assert!(failures.is_empty(), "{} of {run} examples differ:\n\n{}", failures.len(), failures.join("\n\n"));
    assert_eq!(skipped, NOT_RUN, "blocks marked <!-- not run -->");
}

#[test]
fn the_reader_of_the_manual_reads_commands_answers_and_wildcards() {
    let text = "## Chapter 5 — x\n\n<!-- setup\nset b 2\n-->\n```text\nkiva> set a 1\nOK: a saved.\nkiva> schema set u: {\"a\": 1,\n   ...   \"b\": 2}\nSchema set\n...\nkiva> get a\n```\n\n<!-- not run: a server -->\n```text\nkiva> serve\n```\n";
    let (b, skipped) = blocks(text);
    assert_eq!((b.len(), skipped), (1, 1));
    assert_eq!(b[0].setup, ["set b 2"]);
    let c = &b[0].commands;
    assert_eq!(c.len(), 3);
    assert_eq!(c[1].input, "schema set u: {\"a\": 1,\n\"b\": 2}");
    assert_eq!(c[1].expected, ["Schema set", "..."]);
    assert!(c[2].expected.is_empty());
    assert!(line_matches("Took ... ms.", "Took 12 ms."));
    assert!(!line_matches("Took ... ms.", "Took 12 s."));
    assert!(line_matches("| 2026-10-12 09:44:54 |", "| 2026-11-18 11:54:22 |"));
    assert!(!line_matches("at: 2026-10-27 14:30:00 +01:00", "at: 2026-10-27 15:30:00 +01:00"));
    assert!(lines_match(&["a".into(), "...".into(), "z".into()], &["a", "b", "c", "z"]));
    assert!(!lines_match(&["a".into()], &["a", "b"]));
    assert_eq!(
        as_in_manual("created at /tmp/h/workspaces/shop.", "/tmp/h"),
        r"created at C:\Users\you\AppData\Roaming\KivaDB\workspaces\shop."
    );
}
