// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Line editing for the interactive shell: arrow keys, a history kept
//! between sessions, and Tab completion of the command language.

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Config, Context, Editor, ExternalPrinter, Helper, Highlighter, Hinter, Validator};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const PROMPT: &str = "kiva> ";

/// First words of every command.
const COMMANDS: &[&str] = &[
    "set",
    "get",
    "update",
    "del",
    "has",
    "typeof",
    "change",
    "bump",
    "print",
    "scan",
    "keys",
    "range",
    "count",
    "sum",
    "explain",
    "json",
    "convert",
    "begin",
    "commit",
    "rollback",
    "savepoint",
    "transaction",
    "stats",
    "compact",
    "autocompact",
    "check",
    "hint",
    "compress",
    "workspace",
    "config",
    "create",
    "drop",
    "grant",
    "revoke",
    "modify",
    "users",
    "whoami",
    "auth",
    "audit",
    "eval",
    "script",
    "triggers",
    "indexes",
    "bench",
    "replica",
    "wait",
    "failover",
    "subscribe",
    "unsubscribe",
    "ping",
    "version",
    "help",
    "clear",
    "exit",
    "shutdown",
];

/// Words that may follow a given first word.
fn followers(first: &str) -> &'static [&'static str] {
    match first {
        "get" => &["where", "range", "similar", "string", "number", "boolean", "null", "array", "object", "bytes"],
        "set" | "update" | "del" | "has" => {
            &["string", "number", "boolean", "null", "array", "object", "bytes", "ttl", "and"]
        }
        "count" | "sum" => &["where"],
        "explain" => &["get", "count", "sum", "range"],
        "json" => &["get", "set", "delete"],
        "transaction" => &["status"],
        "rollback" => &["to"],
        "hint" => &["info"],
        "check" => &["repair"],
        "compress" => &["lz4", "zlib", "none"],
        "autocompact" => &["on", "off", "status", "threshold"],
        "workspace" => &["list", "current", "switch", "create", "remove"],
        "config" => &["list", "get", "set"],
        "create" => &["user", "index", "vector", "trigger"],
        "drop" => &["user", "index", "vector", "trigger"],
        "modify" => &["user"],
        "grant" | "revoke" => &["read", "write", "admin"],
        "audit" => &["last", "since", "failed-logins"],
        "script" => &["load", "run", "list", "show", "delete"],
        "bench" => &["script", "write", "read"],
        "replica" => &["status", "start", "stop", "sync", "promote", "of"],
        "failover" => &["to"],
        _ => &[],
    }
}

#[derive(Helper, Hinter, Highlighter, Validator)]
struct KivaHelper;

impl Completer for KivaHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let before = &line[..pos];
        let start = before.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        let word = &before[start..];
        let earlier: Vec<&str> = before[..start].split_whitespace().collect();
        let pool: &[&str] = match earlier.first() {
            None => COMMANDS,
            Some(first) => followers(&first.to_ascii_lowercase()),
        };
        let lower = word.to_ascii_lowercase();
        let pairs = pool
            .iter()
            .filter(|c| c.starts_with(&lower))
            .map(|c| Pair { display: c.to_string(), replacement: format!("{c} ") })
            .collect();
        Ok((start, pairs))
    }
}

/// Where asynchronous messages (change notifications) are written.
pub type Sink = Arc<Mutex<Box<dyn FnMut(&str) + Send>>>;

fn plain_sink() -> Sink {
    let interactive = io::stdin().is_terminal();
    Arc::new(Mutex::new(Box::new(move |msg: &str| {
        if interactive {
            print!("\n{msg}\n{PROMPT}");
        } else {
            println!("{msg}");
        }
        let _ = io::stdout().flush();
    })))
}

/// Commands that carry a secret are never written to the history file.
fn sensitive(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("identified by") || l.contains("password") || l.starts_with("auth ") || l.starts_with("login ")
}

/// Reads command lines: with a line editor on a terminal, plainly otherwise.
pub struct Prompt {
    editor: Option<Editor<KivaHelper, DefaultHistory>>,
    history: Option<PathBuf>,
}

impl Prompt {
    pub fn new(history: Option<PathBuf>) -> (Prompt, Sink) {
        if !io::stdin().is_terminal() {
            return (Prompt { editor: None, history: None }, plain_sink());
        }
        let config = Config::builder().max_history_size(1000).map(|b| b.auto_add_history(false).build());
        let editor = config.ok().and_then(|c| Editor::<KivaHelper, DefaultHistory>::with_config(c).ok());
        let Some(mut editor) = editor else {
            return (Prompt { editor: None, history: None }, plain_sink());
        };
        editor.set_helper(Some(KivaHelper));
        if let Some(path) = &history {
            let _ = editor.load_history(path);
        }
        // Notifications are printed above the line being typed.
        let sink: Sink = match editor.create_external_printer() {
            Ok(mut printer) => Arc::new(Mutex::new(Box::new(move |msg: &str| {
                let _ = printer.print(msg.to_string());
            }))),
            Err(_) => plain_sink(),
        };
        (Prompt { editor: Some(editor), history }, sink)
    }

    /// The next line, or `None` at the end of input (Ctrl+D).
    pub fn read(&mut self) -> Option<String> {
        let Some(editor) = self.editor.as_mut() else {
            let mut line = String::new();
            return match io::stdin().lock().read_line(&mut line) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
            };
        };
        match editor.readline(PROMPT) {
            Ok(line) => {
                if !line.trim().is_empty() && !sensitive(&line) {
                    let _ = editor.add_history_entry(line.as_str());
                }
                Some(line)
            }
            // Ctrl+C abandons the line being typed, as in most shells.
            Err(ReadlineError::Interrupted) => Some(String::new()),
            Err(_) => None,
        }
    }

    pub fn save(&mut self) {
        if let (Some(editor), Some(path)) = (self.editor.as_mut(), &self.history) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = editor.save_history(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyline::history::DefaultHistory;

    fn complete(line: &str) -> Vec<String> {
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        KivaHelper.complete(line, line.len(), &ctx).unwrap().1.into_iter().map(|p| p.display).collect()
    }

    #[test]
    fn completion() {
        assert_eq!(complete("tra"), ["transaction"]);
        assert!(complete("s").contains(&"stats".to_string()));
        assert_eq!(complete("get sim"), ["similar"]);
        assert_eq!(complete("workspace sw"), ["switch"]);
        assert!(complete("compact x").is_empty());
    }

    #[test]
    fn secrets_stay_out_of_history() {
        assert!(sensitive(r#"create user "a" identified by "b""#));
        assert!(sensitive(r#"modify user "a" password "b""#));
        assert!(sensitive("auth kiva pw"));
        assert!(!sensitive("get user:1"));
    }
}
