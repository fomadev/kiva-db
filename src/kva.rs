// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Commands that span several lines, and `.kva` files.
//!
//! A command goes on while a JSON document it started (`{` or `[`) is not
//! closed, so documents can be written over several lines:
//!
//! ```text
//! set user:1 {
//!   "name": "Fordi",
//!   "age": 22
//! }
//! ```
//!
//! A `.kva` file is a list of such commands, with `#` or `--` comment lines
//! and blank lines between them. `kivadb run file.kva` runs it, and so does
//! `source "file.kva"` in the shell.

/// One command of a file, with the line where it starts (1-based).
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub line: usize,
    pub text: String,
}

/// Whether `text` opened a JSON document (`{` or `[`) that is not closed yet:
/// the command continues on the next line. Brackets inside quoted text do
/// not count; inside a document, only double-quoted JSON strings exist.
pub fn is_open(text: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut in_json_string = false;
    let mut escaped = false;
    let mut word_start = true;
    for c in text.chars() {
        if depth > 0 {
            if in_json_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_json_string = false;
                }
                continue;
            }
            match c {
                '"' => in_json_string = true,
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                word_start = false;
            }
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            // As in the lexer, a document starts a word: `a{` is a plain word.
            '{' | '[' if word_start => depth = 1,
            c if c.is_whitespace() || matches!(c, ',' | '(' | ')') => {
                word_start = true;
                continue;
            }
            _ => {}
        }
        word_start = false;
    }
    depth > 0
}

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#') || t.starts_with("--")
}

/// Splits a `.kva` text into commands. Comment and blank lines between
/// commands are skipped; a command that is still open at the end of the
/// text is an error, reported with the line where it started.
pub fn split(source: &str) -> Result<Vec<Statement>, (usize, String)> {
    let mut out = Vec::new();
    let mut pending: Option<Statement> = None;
    for (i, raw) in source.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        match pending.as_mut() {
            Some(p) => {
                p.text.push('\n');
                p.text.push_str(line);
            }
            None => {
                if line.trim().is_empty() || is_comment(line) {
                    continue;
                }
                pending = Some(Statement { line: i + 1, text: line.trim_start().to_string() });
            }
        }
        if let Some(p) = pending.as_ref() {
            if !is_open(&p.text) {
                out.push(pending.take().expect("pending"));
            }
        }
    }
    match pending {
        Some(p) => Err((p.line, "this command opens a { or [ that is never closed".into())),
        None => Ok(out),
    }
}

/// The text of a file: UTF-8, with or without a byte order mark, or UTF-16
/// with one (what Windows PowerShell 5.1 writes with `>` or `Out-File`).
pub fn decode(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let (pairs, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return Err("it starts as UTF-16 text but has an odd number of bytes".to_string());
        }
        let units: Vec<u16> = pairs.iter().map(|&p| unit(p)).collect();
        String::from_utf16(&units).map_err(|_| "it starts as UTF-16 text but is not valid UTF-16".to_string())
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => std::str::from_utf8(rest).map(String::from).map_err(|e| e.to_string()),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => std::str::from_utf8(bytes)
            .map(String::from)
            .map_err(|_| "it is not UTF-8 text (save it as UTF-8, or as UTF-16 with a byte order mark)".to_string()),
    }
}

/// Reads and splits a `.kva` file; errors are ready to show.
pub fn load(path: &std::path::Path) -> Result<Vec<Statement>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let text = decode(&bytes).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    split(&text).map_err(|(line, msg)| format!("{}:{line}: {msg}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_documents_continue_the_command() {
        assert!(is_open("set user:1 {"));
        assert!(is_open("set user:1 [\"s\","));
        assert!(is_open("set user:1 [") && is_open("set x {\"a\": [1, 2"));
        assert!(!is_open("set user:1 {\"a\": 1}"));
        assert!(!is_open("set x [1, [2]]"));
        // Brackets in text, in JSON strings, or inside a word do not count.
        assert!(!is_open("set note \"a { b\""));
        assert!(!is_open("set note 'x [ y'"));
        assert!(!is_open("set k {\"t\": \"} and {\"}"));
        assert!(is_open("set k {\"t\": \"escaped \\\" {\""));
        assert!(!is_open("get user:{id}"));
        assert!(!is_open("get a{"));
    }

    #[test]
    fn files_split_into_commands_with_their_lines() {
        let src = "# strings\nset nameA \"Fordi Malanda\"\n\n-- numbers\nset number ageM 24\n\nset user:1 {\n  \"nom\": \"Fordi\",\n  \"age\": 22\n}\nset classe:1 [\n  \"1er\",\n  \"2eme\"\n]\n--- end\n";
        let s = split(src).unwrap();
        assert_eq!(s.len(), 4);
        assert_eq!(s[0], Statement { line: 2, text: "set nameA \"Fordi Malanda\"".into() });
        assert_eq!((s[1].line, s[2].line, s[3].line), (5, 7, 11));
        assert_eq!(s[2].text, "set user:1 {\n  \"nom\": \"Fordi\",\n  \"age\": 22\n}");
        // Windows line endings work too.
        assert_eq!(split("set a 1\r\nset b {\r\n\"x\": 1\r\n}\r\n").unwrap().len(), 2);
    }

    #[test]
    fn files_written_by_windows_tools_are_decoded() {
        let text = "set nom \"Élisabethville\"\r\nset b {\"x\": 1}\r\n";
        let utf16 = |be: bool| -> Vec<u8> {
            let mut out = if be { vec![0xFE, 0xFF] } else { vec![0xFF, 0xFE] };
            for u in text.encode_utf16() {
                out.extend(if be { u.to_be_bytes() } else { u.to_le_bytes() });
            }
            out
        };
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend(text.as_bytes());
        for bytes in [text.as_bytes().to_vec(), bom, utf16(false), utf16(true)] {
            let decoded = decode(&bytes).unwrap();
            assert_eq!(decoded, text);
            assert_eq!(split(&decoded).unwrap()[0].text, "set nom \"Élisabethville\"");
        }
        assert!(decode(&[0xFF, 0xFE, 0x41]).unwrap_err().contains("odd number"));
        assert!(decode(&[0xC3, 0x28]).unwrap_err().contains("not UTF-8"));
    }

    #[test]
    fn an_unclosed_document_is_reported_where_it_starts() {
        assert_eq!(split("set a 1\nset b {\n  \"x\": 1\n").unwrap_err().0, 2);
    }
}
