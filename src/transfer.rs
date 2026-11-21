// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Export and import of a collection's keys as NDJSON (lot G of
//! docs/architecture/RC5_ARCHITECTURE.md, decision D10): one line per key,
//! `{"key": ..., "type": ..., "value": ..., "expires_at": ...}`, the value in
//! extended JSON so that decimals, dates and typed documents keep their
//! types. It is a logical copy, readable by other tools and filtered by a
//! pattern; `kivadb backup` stays the physical copy.
//!
//! Both go through a session: grants, schemas, unique indexes, the change
//! journal and replication apply as to any other write.

use crate::api::{type_on_wire, value_in_as, value_to_wire, Types};
use crate::engine::BatchOp;
use crate::error::{KivaError, Result};
use crate::session::Session;
use crate::value::{Type, Value};
use serde_json::{json, Value as Json};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// Keys written by one batch: each batch is atomic.
const BATCH: usize = 500;
/// Lines whose errors are listed; the others are counted.
const SHOWN_ERRORS: usize = 20;

/// One key as a line of the file.
pub fn line_of(key: &str, value: &Value, expires_at: i64) -> Json {
    let value_json = match value {
        // A number neither `int` nor `float` (1.x) keeps its exact text.
        Value::Num(text) if crate::value::num_kind(text) == "number" => Json::String(text.clone()),
        v => value_to_wire(v, Types::Extended),
    };
    let expires = match chrono::DateTime::from_timestamp_millis(expires_at).filter(|_| expires_at > 0) {
        Some(d) => Json::String(crate::time::iso(d)),
        None => Json::Null,
    };
    json!({ "key": key, "type": type_on_wire(value, Types::Extended), "value": value_json, "expires_at": expires })
}

/// Writes the keys of the session's collection matching `pattern` to
/// `path`. Returns how many were written.
pub fn export(sess: &mut Session, pattern: &str, path: &Path) -> Result<u64> {
    let keys: Vec<String> = sess.engine.keys_matching(pattern, 0).into_iter().map(|(k, _)| k).collect();
    let file = File::create(path)?;
    let mut out = BufWriter::new(file);
    let mut n = 0;
    for key in keys {
        // Gone since the listing (deleted, expired): not exported.
        let Some((value, meta)) = sess.kv_get(&key)? else { continue };
        let line = serde_json::to_string(&line_of(&key, &value, meta.expires_at))
            .map_err(|e| KivaError::Invalid(format!("'{key}' could not be written: {e}")))?;
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
        n += 1;
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok(n)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ImportOptions {
    /// Check every line, write nothing.
    pub dry_run: bool,
    /// Overwrite the keys the collection holds already; otherwise they are kept.
    pub replace: bool,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub lines: u64,
    /// Keys written (or, in a dry run, that would be): new ones, and replaced ones.
    pub created: u64,
    pub replaced: u64,
    /// Keys the collection held already, left as they were (without `replace`).
    pub kept: u64,
    /// Lines whose `expires_at` is past: not written.
    pub expired: u64,
}

impl ImportReport {
    pub fn text(&self, collection: &str, dry_run: bool) -> String {
        let written = self.created + self.replaced;
        let mut s = if dry_run {
            format!(
                "Dry run: {} line(s) checked; {written} key(s) would be written to '{collection}' ({} new, {} replaced)",
                self.lines, self.created, self.replaced
            )
        } else {
            format!(
                "Imported into '{collection}': {written} key(s) written ({} new, {} replaced)",
                self.created, self.replaced
            )
        };
        if self.kept > 0 {
            s.push_str(&format!(", {} kept as they were (already present; --replace overwrites them)", self.kept));
        }
        if self.expired > 0 {
            s.push_str(&format!(", {} skipped (their expires_at is past)", self.expired));
        }
        s.push('.');
        if dry_run {
            s.push_str(" Nothing was written.");
        }
        s
    }
}

/// What the first pass found for a line.
enum Outcome {
    Expired,
    Kept,
    Replaced,
    New,
    /// A key named again further in the file: the last line wins.
    Again,
}

/// One line read and checked.
struct Item {
    key: String,
    value: Value,
    expires_at: i64,
}

fn parse_line(text: &str) -> std::result::Result<Item, String> {
    let j: Json = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let obj = j.as_object().ok_or("a line is a JSON object {\"key\": ..., \"type\": ..., \"value\": ...}")?;
    let key = obj.get("key").and_then(Json::as_str).ok_or("\"key\" is missing or not a text")?.to_string();
    let ty_name = obj.get("type").and_then(Json::as_str).ok_or("\"type\" is missing or not a text")?;
    let ty = Type::from_name(ty_name).ok_or_else(|| format!("unknown type '{ty_name}'"))?;
    let raw = obj.get("value").unwrap_or(&Json::Null);
    let value = value_in_as(ty, raw, Types::Extended).map_err(|e| e.to_string())?;
    let expires_at = match obj.get("expires_at").unwrap_or(&Json::Null) {
        Json::Null => 0,
        Json::Number(n) => n.as_i64().filter(|ms| *ms > 0).ok_or("\"expires_at\" is not a time")?,
        Json::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map_err(|_| format!("\"expires_at\" '{s}' is not an ISO 8601 time"))?
            .timestamp_millis(),
        _ => return Err("\"expires_at\" is null, an ISO 8601 time or milliseconds".into()),
    };
    Ok(Item { key, value, expires_at })
}

/// Reads the lines of `path`, checks them all, and when none is wrong
/// writes them (unless `dry_run`). A wrong line refuses the whole file
/// before anything is written: its errors say where.
pub fn import(sess: &mut Session, path: &Path, opts: ImportOptions) -> Result<ImportReport> {
    sess.writable()?;
    let mut report = ImportReport::default();
    let mut errors: Vec<String> = Vec::new();
    let mut wrong = 0u64;
    let now = crate::util::now_ms();
    // First pass: every line checked; only the keys are kept, to count each once.
    let mut seen = std::collections::HashSet::new();
    for (n, item) in read_items(path)? {
        report.lines += 1;
        let checked = item.and_then(|it| {
            if it.expires_at > 0 && it.expires_at <= now {
                return Ok(Outcome::Expired);
            }
            let exists = sess.engine.contains(&it.key);
            if !exists {
                crate::exec::validate_key_name(&it.key)?;
            }
            if exists && !opts.replace {
                return Ok(Outcome::Kept);
            }
            sess.engine.check_against_schemas(&it.key, &it.value).map_err(|e| e.to_string())?;
            let again = !seen.insert(it.key);
            Ok(match (again, exists) {
                (true, _) => Outcome::Again,
                (false, true) => Outcome::Replaced,
                (false, false) => Outcome::New,
            })
        });
        match checked {
            Ok(Outcome::Expired) => report.expired += 1,
            Ok(Outcome::Kept) => report.kept += 1,
            Ok(Outcome::Replaced) => report.replaced += 1,
            Ok(Outcome::New) => report.created += 1,
            Ok(Outcome::Again) => {}
            Err(e) => {
                wrong += 1;
                if errors.len() < SHOWN_ERRORS {
                    errors.push(format!("  line {n}: {e}"));
                }
            }
        }
    }
    if wrong > 0 {
        let more = if wrong as usize > errors.len() {
            format!("\n  ... and {} more", wrong as usize - errors.len())
        } else {
            String::new()
        };
        return Err(KivaError::Invalid(format!(
            "{} not imported: {wrong} line(s) are wrong, and nothing was written:\n{}{more}",
            path.display(),
            errors.join("\n")
        )));
    }
    if opts.dry_run {
        return Ok(report);
    }
    // Second pass: write, in atomic batches.
    let mut batch: Vec<BatchOp> = Vec::with_capacity(BATCH);
    let mut first_line = 0;
    let mut written = 0u64;
    for (n, item) in read_items(path)? {
        let Ok(it) = item else { continue };
        if it.expires_at > 0 && it.expires_at <= crate::util::now_ms() {
            continue;
        }
        if !opts.replace && sess.engine.contains(&it.key) {
            continue;
        }
        if batch.is_empty() {
            first_line = n;
        }
        batch.push(BatchOp::Put { key: it.key, value: it.value, expires_at: it.expires_at, msgpack: false });
        if batch.len() == BATCH {
            written += flush(sess, &mut batch, first_line, written)?;
        }
    }
    if !batch.is_empty() {
        flush(sess, &mut batch, first_line, written)?;
    }
    Ok(report)
}

fn flush(sess: &mut Session, batch: &mut Vec<BatchOp>, first_line: u64, written: u64) -> Result<u64> {
    let n = batch.len() as u64;
    sess.apply(std::mem::take(batch)).map_err(|e| {
        KivaError::Invalid(format!(
            "the import stopped in the batch starting at line {first_line}: {e}. {written} key(s) were written \
             before it; importing the file again keeps them (or replaces them with --replace)"
        ))
    })?;
    Ok(n)
}

/// The lines of the file, numbered from 1, blank ones left out, each read
/// and checked on its own.
fn read_items(path: &Path) -> Result<impl Iterator<Item = (u64, std::result::Result<Item, String>)>> {
    let file = File::open(path).map_err(|e| KivaError::Invalid(format!("{} cannot be read: {e}", path.display())))?;
    Ok(BufReader::new(file).lines().enumerate().filter_map(|(i, line)| {
        let n = i as u64 + 1;
        match line {
            Ok(text) if text.trim().is_empty() => None,
            Ok(text) => Some((n, parse_line(text.trim_start_matches('\u{feff}')))),
            Err(e) => Some((n, Err(format!("cannot be read: {e}")))),
        }
    }))
}
