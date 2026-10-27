// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The change journal of a workspace (change data capture): every committed
//! write, numbered, kept on disk for a retention period, read again from a
//! cursor after a disconnection or a restart.
//!
//! The data file stays the source of truth. The journal is written under the
//! engine lock right after the data, and each change records where its data
//! record ends. When KivaDB opens a workspace after a crash, the changes the
//! journal lacks are rebuilt from the data file past the last one it holds,
//! and the changes whose data did not reach the disk are dropped: data and
//! changes always agree. A change is handed to a reader only once its data
//! and itself are on disk (the read syncs both first).
//!
//! On disk: `<data>.changes/`, segment files named by their first sequence
//! number. Segment header (24 bytes): `KCDC` | version u32 | journal id u64 |
//! first sequence u64. Record: length u32 | crc32 u32 of the body | body,
//! encrypted with the workspace's key when it has one. Body:
//! seq u64 | data file id u64 | data end u64 | txn u64 | kind u8 | flags u8 |
//! type u8 | timestamp i64 | expires_at i64 | key length u16 | key | value.

use crate::crypt::{Domain, Keys};
use crate::error::{KivaError, Result};
use crate::util::glob_match;
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"KCDC";
const VERSION: u32 = 1;
const HEADER_LEN: u64 = 24;
const BODY_FIXED: usize = 8 * 4 + 3 + 8 * 2 + 2;
/// A record longer than this is damage, not a change (values are at most 256 MB).
const MAX_RECORD: usize = 300 * 1024 * 1024;
/// One read returns at most this many changes, and looks at most at this
/// many records (those a pattern filters out included).
pub const MAX_READ: usize = 1000;
const MAX_SCAN: usize = 20_000;
/// A sparse index of each segment: every this many records, where it starts.
const INDEX_EVERY: u64 = 256;

const KIND_MARK: u8 = 0;
const KIND_SET: u8 = 1;
const KIND_DEL: u8 = 2;
const KIND_RESET: u8 = 3;

const F_TXN_END: u8 = 1;
const F_MSGPACK: u8 = 2;
const F_VALUE: u8 = 4;
const F_CREATED: u8 = 8;
/// Rebuilt from the data file after a crash: whether the key was created is not known.
const F_REBUILT: u8 = 16;

/// How long and how much the journal keeps, and whether it keeps the values.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub values: bool,
    pub retain_secs: u64,
    pub max_bytes: u64,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { values: true, retain_secs: 24 * 3600, max_bytes: 256 << 20 }
    }
}

impl Settings {
    pub fn to_json(&self) -> Json {
        json!({ "values": self.values, "retain_secs": self.retain_secs, "max_bytes": self.max_bytes })
    }

    pub fn from_json(j: &Json) -> Settings {
        let d = Settings::default();
        Settings {
            values: j["values"].as_bool().unwrap_or(d.values),
            retain_secs: j["retain_secs"].as_u64().unwrap_or(d.retain_secs),
            max_bytes: j["max_bytes"].as_u64().unwrap_or(d.max_bytes),
        }
    }

    fn segment_bytes(&self) -> u64 {
        (self.max_bytes / 8).clamp(1 << 20, 64 << 20)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Set,
    Del,
    Reset,
}

/// A change about to be journaled: where its data record ends, and what it did.
pub struct NewChange<'a> {
    pub kind: ChangeKind,
    pub key: &'a str,
    pub ty: u8,
    pub raw: &'a [u8],
    pub msgpack: bool,
    pub timestamp: i64,
    pub expires_at: i64,
    /// `None` when not known (rebuilt after a crash).
    pub created: Option<bool>,
    pub data_end: u64,
}

/// A change read back.
#[derive(Clone, Debug)]
pub struct Change {
    pub seq: u64,
    pub kind: ChangeKind,
    pub key: String,
    pub ty: u8,
    /// The value as stored (canonical bytes), when the journal keeps values.
    pub value: Option<Vec<u8>>,
    pub msgpack: bool,
    pub timestamp: i64,
    pub expires_at: i64,
    pub created: Option<bool>,
    /// The sequence number of the first change of its transaction, if it was one.
    pub txn: Option<u64>,
    pub txn_end: bool,
}

struct Segment {
    path: PathBuf,
    first_seq: u64,
    len: u64,
    /// Timestamp of its newest change (seconds), for the retention by age.
    last_ts: i64,
    /// (sequence, offset) every `INDEX_EVERY` records.
    index: Vec<(u64, u64)>,
    records: u64,
}

pub struct Journal {
    dir: PathBuf,
    pub id: u64,
    pub settings: Settings,
    keys: Option<Arc<Keys>>,
    segments: Vec<Segment>,
    file: File,
    /// The last sequence number given (0: none yet).
    pub last_seq: u64,
    /// Changes not yet synced to disk.
    dirty: bool,
    /// Where the data file stood after the last record: what to rebuild from.
    pub last_pos: Option<(u64, u64)>,
    consumers: BTreeMap<String, (u64, i64)>,
    /// Set when a write to the journal failed: reads are refused until a restart rebuilds it.
    pub broken: Option<String>,
    /// Changes written since the journal was opened.
    pub written: u64,
    /// `written` when a read last synced the data and the journal.
    pub read_synced: u64,
}

#[derive(Debug)]
pub enum ReadError {
    /// The cursor points before the oldest change kept: those were removed.
    Expired {
        oldest: u64,
    },
    Invalid(String),
    Other(KivaError),
}

impl From<ReadError> for KivaError {
    fn from(e: ReadError) -> KivaError {
        match e {
            ReadError::Expired { oldest } => KivaError::Invalid(format!(
                "CURSOR_EXPIRED: the changes after this cursor were removed by the retention (the oldest kept is \
                 number {oldest}); read again from the beginning, and resynchronise what you hold"
            )),
            ReadError::Invalid(m) => KivaError::Invalid(format!("CURSOR_INVALID: {m}")),
            ReadError::Other(e) => e,
        }
    }
}

/// What one read gave.
pub struct Batch {
    pub changes: Vec<Change>,
    /// The position after the last change looked at: the next read starts there.
    pub next: u64,
    /// Whether more changes are waiting past `next`.
    pub more: bool,
}

pub fn dir_of(data: &Path) -> PathBuf {
    let mut p = data.as_os_str().to_owned();
    p.push(".changes");
    PathBuf::from(p)
}

fn seg_name(first: u64) -> String {
    format!("{first:020}.log")
}

impl Journal {
    /// Opens the journal in `dir`, or starts one. Damaged or cut records at
    /// the end of the newest segment are cut off.
    pub fn open(dir: &Path, keys: Option<Arc<Keys>>, settings: Settings) -> Result<Journal> {
        fs::create_dir_all(dir)?;
        let mut names: Vec<(u64, PathBuf)> = fs::read_dir(dir)?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let first = name.strip_suffix(".log")?.parse::<u64>().ok()?;
                Some((first, e.path()))
            })
            .collect();
        names.sort();
        let mut id = None;
        let mut segments = Vec::new();
        let mut last_seq = 0;
        let mut last_pos = None;
        let count = names.len();
        for (n, (first, path)) in names.into_iter().enumerate() {
            let newest = n + 1 == count;
            let data = fs::read(&path)?;
            let Some(seg_id) = parse_header(&data, first) else {
                if newest {
                    // Cut before its header was whole: nothing in it.
                    let _ = fs::remove_file(&path);
                    continue;
                }
                return Err(KivaError::Corrupted(format!("{} is not a change journal segment", path.display())));
            };
            if *id.get_or_insert(seg_id) != seg_id {
                return Err(KivaError::Corrupted(format!("{} belongs to another change journal", path.display())));
            }
            let mut seg = Segment {
                path: path.clone(),
                first_seq: first,
                len: HEADER_LEN,
                last_ts: 0,
                index: Vec::new(),
                records: 0,
            };
            let mut pos = HEADER_LEN as usize;
            while pos < data.len() {
                let Some((body, end)) = frame(&data, pos) else { break };
                let Ok(body) = open_body(keys.as_deref(), seg_id, body) else { break };
                let Some(r) = parse_body(&body) else { break };
                if r.kind != KIND_MARK {
                    if seg.records.is_multiple_of(INDEX_EVERY) {
                        seg.index.push((r.seq, pos as u64));
                    }
                    seg.records += 1;
                    last_seq = r.seq;
                    seg.last_ts = r.timestamp;
                }
                last_pos = Some((r.file_id, r.data_end));
                pos = end;
                seg.len = pos as u64;
            }
            if (seg.len as usize) < data.len() {
                if !newest {
                    return Err(KivaError::Corrupted(format!(
                        "{} is damaged at byte {}: remove the folder {} to start a new change journal",
                        path.display(),
                        seg.len,
                        dir.display()
                    )));
                }
                OpenOptions::new().write(true).open(&path)?.set_len(seg.len)?;
            }
            segments.push(seg);
        }
        let id = id.unwrap_or_else(crate::util::random_u64);
        if segments.is_empty() {
            segments.push(new_segment(dir, id, last_seq + 1)?);
        }
        let active = segments.last().expect("one segment");
        let file = OpenOptions::new().read(true).write(true).open(&active.path)?;
        let consumers = read_consumers(dir);
        Ok(Journal {
            dir: dir.to_path_buf(),
            id,
            settings,
            keys,
            segments,
            file,
            last_seq,
            dirty: false,
            last_pos,
            consumers,
            broken: None,
            written: 0,
            read_synced: 0,
        })
    }

    /// The oldest sequence number a reader can still get.
    pub fn first_seq(&self) -> u64 {
        self.segments.first().map(|s| s.first_seq).unwrap_or(1)
    }

    pub fn bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.len).sum()
    }

    pub fn retained(&self) -> u64 {
        self.segments.iter().map(|s| s.records).sum()
    }

    /// Appends the changes of one write, or of one transaction (`txn`: they
    /// are numbered together, the last one closing it), in one write.
    pub fn append(&mut self, file_id: u64, changes: &[NewChange], txn: bool) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let first = self.last_seq + 1;
        let mut buf = Vec::new();
        let mut offsets = Vec::with_capacity(changes.len());
        for (i, c) in changes.iter().enumerate() {
            let seq = first + i as u64;
            let mut flags = 0;
            if c.msgpack {
                flags |= F_MSGPACK;
            }
            match c.created {
                Some(true) => flags |= F_CREATED,
                Some(false) => {}
                None => flags |= F_REBUILT,
            }
            if txn && i + 1 == changes.len() {
                flags |= F_TXN_END;
            }
            let keep_value = self.settings.values && c.kind == ChangeKind::Set;
            if keep_value {
                flags |= F_VALUE;
            }
            let kind = match c.kind {
                ChangeKind::Set => KIND_SET,
                ChangeKind::Del => KIND_DEL,
                ChangeKind::Reset => KIND_RESET,
            };
            let body = body(Body {
                seq,
                file_id,
                data_end: c.data_end,
                txn: if txn { first } else { 0 },
                kind,
                flags,
                ty: c.ty,
                timestamp: c.timestamp,
                expires_at: c.expires_at,
                key: c.key,
                value: if keep_value { c.raw } else { &[] },
            });
            offsets.push((seq, buf.len() as u64, c.timestamp));
            self.push_record(&mut buf, body);
        }
        let last = changes.last().expect("not empty");
        self.write(&buf, &offsets)?;
        self.last_seq = first + changes.len() as u64 - 1;
        self.last_pos = Some((file_id, last.data_end));
        self.written += changes.len() as u64;
        Ok(())
    }

    /// Records where the data file stands without a change: after a
    /// compaction (a new data file) or when the journal starts.
    pub fn mark(&mut self, file_id: u64, data_end: u64) -> Result<()> {
        let b = body(Body {
            seq: self.last_seq,
            file_id,
            data_end,
            txn: 0,
            kind: KIND_MARK,
            flags: 0,
            ty: 0,
            timestamp: 0,
            expires_at: 0,
            key: "",
            value: &[],
        });
        let mut buf = Vec::new();
        self.push_record(&mut buf, b);
        self.write(&buf, &[])?;
        self.last_pos = Some((file_id, data_end));
        Ok(())
    }

    fn push_record(&self, buf: &mut Vec<u8>, mut body: Vec<u8>) {
        if let Some(k) = &self.keys {
            k.seal(Domain::Changes, &self.id.to_le_bytes(), &mut body);
        }
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
        buf.extend_from_slice(&body);
    }

    fn write(&mut self, buf: &[u8], offsets: &[(u64, u64, i64)]) -> Result<()> {
        if self.segments.last().is_some_and(|s| s.len >= self.settings.segment_bytes() && s.records > 0) {
            self.rotate()?;
        }
        let seg = self.segments.last_mut().expect("one segment");
        let at = seg.len;
        let res = self.file.seek(SeekFrom::Start(at)).and_then(|_| self.file.write_all(buf));
        if let Err(e) = res {
            let _ = self.file.set_len(at);
            self.broken = Some(e.to_string());
            return Err(e.into());
        }
        seg.len += buf.len() as u64;
        for &(seq, off, ts) in offsets {
            if seg.records.is_multiple_of(INDEX_EVERY) {
                seg.index.push((seq, at + off));
            }
            seg.records += 1;
            seg.last_ts = ts;
        }
        self.dirty = true;
        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        self.sync()?;
        let seg = new_segment(&self.dir, self.id, self.last_seq + 1)?;
        self.file = OpenOptions::new().read(true).write(true).open(&seg.path)?;
        self.segments.push(seg);
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        if self.dirty {
            self.file.sync_data()?;
            self.dirty = false;
        }
        Ok(())
    }

    /// Removes the oldest segments past the retention (age or size), never
    /// the one being written. Returns how many changes went.
    pub fn trim(&mut self, now_secs: i64) -> u64 {
        let mut removed = 0;
        while self.segments.len() > 1 {
            let old = &self.segments[0];
            let too_old = old.last_ts > 0 && old.last_ts < now_secs - self.settings.retain_secs as i64;
            let too_big = self.bytes() > self.settings.max_bytes;
            if !(too_old || too_big) {
                break;
            }
            let seg = self.segments.remove(0);
            removed += seg.records;
            let _ = fs::remove_file(&seg.path);
        }
        removed
    }

    pub fn cursor(&self, seq: u64) -> String {
        cursor(self.id, seq)
    }

    /// The sequence number a cursor stands for. A cursor of another journal
    /// (another node, a journal started again) is refused.
    pub fn parse_cursor(&self, cursor: &str) -> std::result::Result<u64, ReadError> {
        let c = cursor.trim();
        let bad = || ReadError::Invalid(format!("'{cursor}' is not a cursor of this change journal"));
        if c.len() != 32 || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let id = u64::from_str_radix(&c[..16], 16).map_err(|_| bad())?;
        let seq = u64::from_str_radix(&c[16..], 16).map_err(|_| bad())?;
        if id != self.id {
            return Err(ReadError::Invalid(
                "this cursor comes from another change journal (another node, or a journal started again after \
                 a restore or a reset of its files): read again from the beginning, and resynchronise"
                    .into(),
            ));
        }
        if seq > self.last_seq {
            return Err(ReadError::Invalid(format!(
                "this cursor is ahead of the journal (number {seq}, the last is {})",
                self.last_seq
            )));
        }
        Ok(seq)
    }

    /// The changes after `after`, up to `limit`, whose key matches `pattern`.
    pub fn read(&mut self, after: u64, limit: usize, pattern: Option<&str>) -> std::result::Result<Batch, ReadError> {
        if let Some(why) = &self.broken {
            return Err(ReadError::Other(KivaError::Other(format!(
                "the change journal could not be written ({why}): restart KivaDB, which rebuilds it from the data"
            ))));
        }
        let oldest = self.first_seq();
        if after + 1 < oldest {
            return Err(ReadError::Expired { oldest });
        }
        let limit = limit.clamp(1, MAX_READ);
        let mut changes = Vec::new();
        let mut next = after;
        let mut scanned = 0;
        let start = self.segments.iter().rposition(|s| s.first_seq <= after + 1).unwrap_or(0);
        'segments: for seg in &self.segments[start..] {
            let from = seg.index.iter().rev().find(|(s, _)| *s <= after + 1).map(|(_, o)| *o).unwrap_or(HEADER_LEN);
            if from >= seg.len {
                continue;
            }
            let mut f = File::open(&seg.path).map_err(|e| ReadError::Other(e.into()))?;
            f.seek(SeekFrom::Start(from)).map_err(|e| ReadError::Other(e.into()))?;
            let mut data = vec![0u8; (seg.len - from) as usize];
            f.read_exact(&mut data).map_err(|e| ReadError::Other(e.into()))?;
            let mut pos = 0;
            while pos < data.len() {
                let Some((b, end)) = frame(&data, pos) else { break };
                pos = end;
                let body = open_body(self.keys.as_deref(), self.id, b).map_err(ReadError::Other)?;
                let Some(r) = parse_body(&body) else { break };
                if r.kind == KIND_MARK || r.seq <= after {
                    continue;
                }
                scanned += 1;
                next = r.seq;
                if pattern.is_none_or(|p| glob_match(p, &r.key) || r.kind == KIND_RESET) {
                    changes.push(r.into_change());
                }
                if changes.len() >= limit || scanned >= MAX_SCAN {
                    break 'segments;
                }
            }
        }
        Ok(Batch { changes, next, more: next < self.last_seq })
    }

    /// Remembers that `name` has handled everything up to `seq`.
    pub fn ack(&mut self, name: &str, seq: u64, now_secs: i64) -> Result<()> {
        if self.consumers.get(name).map(|(s, _)| *s) == Some(seq) {
            return Ok(());
        }
        self.consumers.insert(name.to_string(), (seq, now_secs));
        let list: serde_json::Map<String, Json> =
            self.consumers.iter().map(|(n, (s, at))| (n.clone(), json!({ "seq": s, "at": at }))).collect();
        let tmp = self.dir.join("consumers.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&Json::Object(list))?)?;
        fs::rename(&tmp, self.dir.join("consumers.json"))?;
        Ok(())
    }

    pub fn consumer(&self, name: &str) -> Option<u64> {
        self.consumers.get(name).map(|(s, _)| *s)
    }

    /// Each named consumer: the last change it handled, and when it said so.
    pub fn consumers(&self) -> Vec<(String, u64, i64)> {
        self.consumers.iter().map(|(n, (s, at))| (n.clone(), *s, *at)).collect()
    }

    pub fn remove_consumer(&mut self, name: &str) -> Result<bool> {
        if self.consumers.remove(name).is_none() {
            return Ok(false);
        }
        let list: serde_json::Map<String, Json> =
            self.consumers.iter().map(|(n, (s, at))| (n.clone(), json!({ "seq": s, "at": at }))).collect();
        fs::write(self.dir.join("consumers.json"), serde_json::to_vec_pretty(&Json::Object(list))?)?;
        Ok(true)
    }

    /// Drops the changes whose data is not in the data file (`file_id`,
    /// `len`): written to the journal, lost from the data by a crash. They
    /// were never handed to a reader, which only gets changes synced.
    pub fn drop_after(&mut self, file_id: u64, len: u64) -> Result<u64> {
        let mut dropped = 0;
        loop {
            let seg = self.segments.last().expect("one segment");
            let data = fs::read(&seg.path)?;
            let mut pos = HEADER_LEN as usize;
            let mut keep = HEADER_LEN;
            let mut kept_seq = seg.first_seq.saturating_sub(1);
            let mut kept_pos = None;
            let mut cut = false;
            while pos < data.len() {
                let Some((b, end)) = frame(&data, pos) else { break };
                let body = open_body(self.keys.as_deref(), self.id, b)?;
                let Some(r) = parse_body(&body) else { break };
                if r.file_id == file_id && r.data_end > len {
                    cut = true;
                    break;
                }
                if r.kind != KIND_MARK {
                    kept_seq = r.seq;
                }
                kept_pos = Some((r.file_id, r.data_end));
                pos = end;
                keep = pos as u64;
            }
            if !cut {
                return Ok(dropped);
            }
            let before = self.last_seq;
            let seg = self.segments.last_mut().expect("one segment");
            OpenOptions::new().write(true).open(&seg.path)?.set_len(keep)?;
            seg.len = keep;
            seg.index.retain(|(s, _)| *s <= kept_seq);
            seg.records = kept_seq + 1 - seg.first_seq;
            dropped += before - kept_seq;
            self.last_seq = kept_seq;
            self.last_pos = kept_pos.or(self.last_pos);
            if keep == HEADER_LEN && self.segments.len() > 1 {
                let seg = self.segments.pop().expect("checked");
                let _ = fs::remove_file(&seg.path);
                let active = self.segments.last().expect("one segment");
                self.file = OpenOptions::new().read(true).write(true).open(&active.path)?;
            } else {
                return Ok(dropped);
            }
        }
    }

    /// Where the data file `file_id` stood after the last record that names
    /// it: what to rebuild the missing changes from. `None` when no record
    /// names it (the journal describes another data file).
    pub fn position_for(&self, file_id: u64) -> Result<Option<u64>> {
        for seg in self.segments.iter().rev() {
            let data = fs::read(&seg.path)?;
            let mut pos = HEADER_LEN as usize;
            let mut found = None;
            while pos < data.len() {
                let Some((b, end)) = frame(&data, pos) else { break };
                let body = open_body(self.keys.as_deref(), self.id, b)?;
                let Some(r) = parse_body(&body) else { break };
                if r.file_id == file_id {
                    found = Some(r.data_end);
                }
                pos = end;
            }
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }

    /// Removes the journal and its folder.
    pub fn destroy(self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A change as programs receive it: the value decoded (bytes in base64,
/// dates as ISO 8601 in UTC), the moments as ISO 8601.
pub fn to_json(c: &Change, cursor: &str) -> Json {
    let op = match c.kind {
        ChangeKind::Set => "set",
        ChangeKind::Del => "delete",
        ChangeKind::Reset => "reset",
    };
    let ty = crate::value::Type::from_u8(c.ty).filter(|_| c.kind == ChangeKind::Set);
    let value = match (&c.value, ty) {
        (Some(raw), Some(ty)) => match crate::value::Value::decode(ty, raw, c.msgpack) {
            Ok(crate::value::Value::Bytes(b)) => Json::String(crate::util::b64_encode(&b)),
            Ok(v) => v.to_json(),
            Err(_) => Json::Null,
        },
        _ => Json::Null,
    };
    let at = |ms: i64| {
        chrono::DateTime::from_timestamp_millis(ms).map(|d| Json::String(crate::time::iso(d))).unwrap_or(Json::Null)
    };
    json!({
        "seq": c.seq,
        "cursor": cursor,
        "op": op,
        "key": if c.kind == ChangeKind::Reset { Json::Null } else { Json::String(c.key.clone()) },
        "type": ty.map(|t| t.name()),
        "created": c.created,
        "value": value,
        "timestamp": at(c.timestamp * 1000),
        "expires_at": if c.expires_at > 0 { at(c.expires_at) } else { Json::Null },
        "txn": c.txn,
        "txn_end": c.txn_end,
    })
}

/// The cursor of change `seq` in journal `id`: opaque to readers.
pub fn cursor(id: u64, seq: u64) -> String {
    format!("{id:016x}{seq:016x}")
}

fn new_segment(dir: &Path, id: u64, first: u64) -> Result<Segment> {
    let path = dir.join(seg_name(first));
    let mut h = Vec::with_capacity(HEADER_LEN as usize);
    h.extend_from_slice(MAGIC);
    h.extend_from_slice(&VERSION.to_le_bytes());
    h.extend_from_slice(&id.to_le_bytes());
    h.extend_from_slice(&first.to_le_bytes());
    let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(&path)?;
    f.write_all(&h)?;
    f.sync_data()?;
    Ok(Segment { path, first_seq: first, len: HEADER_LEN, last_ts: 0, index: Vec::new(), records: 0 })
}

fn parse_header(data: &[u8], first: u64) -> Option<u64> {
    if data.len() < HEADER_LEN as usize || &data[..4] != MAGIC || u32_at(data, 4) != VERSION {
        return None;
    }
    (u64_at(data, 16) == first).then(|| u64_at(data, 8))
}

fn read_consumers(dir: &Path) -> BTreeMap<String, (u64, i64)> {
    let Some(j) = fs::read(dir.join("consumers.json")).ok().and_then(|b| serde_json::from_slice::<Json>(&b).ok())
    else {
        return BTreeMap::new();
    };
    j.as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(n, v)| Some((n.clone(), (v["seq"].as_u64()?, v["at"].as_i64().unwrap_or(0)))))
                .collect()
        })
        .unwrap_or_default()
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"))
}

/// The body of the record at `pos`, and where the next one starts; `None`
/// when it is cut or its checksum does not match.
fn frame(data: &[u8], pos: usize) -> Option<(&[u8], usize)> {
    if data.len() - pos < 8 {
        return None;
    }
    let len = u32_at(data, pos) as usize;
    let end = pos.checked_add(8)?.checked_add(len)?;
    if len > MAX_RECORD || end > data.len() {
        return None;
    }
    let body = &data[pos + 8..end];
    (crc32fast::hash(body) == u32_at(data, pos + 4)).then_some((body, end))
}

fn open_body(keys: Option<&Keys>, id: u64, sealed: &[u8]) -> Result<Vec<u8>> {
    match keys {
        Some(k) => k.open(Domain::Changes, &id.to_le_bytes(), sealed).map_err(|e| e.into_error("a change")),
        None => Ok(sealed.to_vec()),
    }
}

struct Body<'a> {
    seq: u64,
    file_id: u64,
    data_end: u64,
    txn: u64,
    kind: u8,
    flags: u8,
    ty: u8,
    timestamp: i64,
    expires_at: i64,
    key: &'a str,
    value: &'a [u8],
}

fn body(b: Body) -> Vec<u8> {
    let mut out = Vec::with_capacity(BODY_FIXED + b.key.len() + b.value.len());
    for n in [b.seq, b.file_id, b.data_end, b.txn] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&[b.kind, b.flags, b.ty]);
    out.extend_from_slice(&b.timestamp.to_le_bytes());
    out.extend_from_slice(&b.expires_at.to_le_bytes());
    out.extend_from_slice(&(b.key.len() as u16).to_le_bytes());
    out.extend_from_slice(b.key.as_bytes());
    out.extend_from_slice(b.value);
    out
}

struct Parsed {
    seq: u64,
    file_id: u64,
    data_end: u64,
    txn: u64,
    kind: u8,
    flags: u8,
    ty: u8,
    timestamp: i64,
    expires_at: i64,
    key: String,
    value: Vec<u8>,
}

fn parse_body(b: &[u8]) -> Option<Parsed> {
    if b.len() < BODY_FIXED {
        return None;
    }
    let klen = u16::from_le_bytes([b[BODY_FIXED - 2], b[BODY_FIXED - 1]]) as usize;
    if b.len() < BODY_FIXED + klen {
        return None;
    }
    Some(Parsed {
        seq: u64_at(b, 0),
        file_id: u64_at(b, 8),
        data_end: u64_at(b, 16),
        txn: u64_at(b, 24),
        kind: b[32],
        flags: b[33],
        ty: b[34],
        timestamp: u64_at(b, 35) as i64,
        expires_at: u64_at(b, 43) as i64,
        key: String::from_utf8_lossy(&b[BODY_FIXED..BODY_FIXED + klen]).into_owned(),
        value: b[BODY_FIXED + klen..].to_vec(),
    })
}

impl Parsed {
    fn into_change(self) -> Change {
        Change {
            seq: self.seq,
            kind: match self.kind {
                KIND_DEL => ChangeKind::Del,
                KIND_RESET => ChangeKind::Reset,
                _ => ChangeKind::Set,
            },
            key: self.key,
            ty: self.ty,
            value: (self.flags & F_VALUE != 0).then_some(self.value),
            msgpack: self.flags & F_MSGPACK != 0,
            timestamp: self.timestamp,
            expires_at: self.expires_at,
            created: if self.flags & F_REBUILT != 0 { None } else { Some(self.flags & F_CREATED != 0) },
            txn: (self.txn != 0).then_some(self.txn),
            txn_end: self.flags & F_TXN_END != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set<'a>(key: &'a str, raw: &'a [u8], end: u64) -> NewChange<'a> {
        NewChange {
            kind: ChangeKind::Set,
            key,
            ty: 1,
            raw,
            msgpack: false,
            timestamp: 100,
            expires_at: 0,
            created: Some(true),
            data_end: end,
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kivadb_changes_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn changes_are_read_back_from_a_cursor_after_reopening() {
        let dir = tmp("reopen");
        let mut j = Journal::open(&dir, None, Settings::default()).unwrap();
        j.mark(7, 16).unwrap();
        j.append(7, &[set("a", b"1", 40)], false).unwrap();
        j.append(7, &[set("b", b"2", 60), set("c", b"3", 80)], true).unwrap();
        j.sync().unwrap();
        let id = j.id;
        drop(j);
        let mut j = Journal::open(&dir, None, Settings::default()).unwrap();
        assert_eq!((j.id, j.last_seq, j.last_pos), (id, 3, Some((7, 80))));
        let b = j.read(0, 10, None).unwrap();
        let keys: Vec<&str> = b.changes.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "c"]);
        assert_eq!((b.changes[1].txn, b.changes[1].txn_end, b.changes[2].txn_end), (Some(2), false, true));
        assert_eq!(b.changes[0].value.as_deref(), Some(&b"1"[..]));
        let after = j.parse_cursor(&j.cursor(1)).ok().unwrap();
        assert_eq!(j.read(after, 1, None).ok().unwrap().changes[0].key, "b");
        assert!(matches!(j.parse_cursor(&format!("{:016x}{:016x}", id ^ 1, 1)), Err(ReadError::Invalid(_))));
        assert!(matches!(j.parse_cursor(&j.cursor(9)), Err(ReadError::Invalid(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cut_record_is_dropped_and_changes_past_the_data_are_removed() {
        let dir = tmp("cut");
        let mut j = Journal::open(&dir, None, Settings::default()).unwrap();
        j.append(7, &[set("a", b"1", 40), set("b", b"2", 60)], false).unwrap();
        j.append(7, &[set("c", b"3", 80)], false).unwrap();
        j.sync().unwrap();
        let seg = j.segments[0].path.clone();
        drop(j);
        let len = fs::metadata(&seg).unwrap().len();
        OpenOptions::new().write(true).open(&seg).unwrap().set_len(len - 3).unwrap();
        let mut j = Journal::open(&dir, None, Settings::default()).unwrap();
        assert_eq!(j.last_seq, 2, "the cut record is gone");
        // The data file ends at 50: "b" (ending at 60) never reached it.
        assert_eq!(j.drop_after(7, 50).unwrap(), 1);
        assert_eq!(j.last_seq, 1);
        j.append(7, &[set("d", b"4", 70)], false).unwrap();
        let keys: Vec<String> = j.read(0, 10, None).ok().unwrap().changes.into_iter().map(|c| c.key).collect();
        assert_eq!(keys, ["a", "d"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_removes_whole_old_segments_and_old_cursors_expire() {
        let dir = tmp("retention");
        let settings = Settings { values: true, retain_secs: 3600, max_bytes: 1 };
        let mut j = Journal::open(&dir, None, settings).unwrap();
        let big = vec![7u8; 600_000];
        for i in 0..6u64 {
            j.append(7, &[set("k", &big, 100 + i)], false).unwrap();
        }
        assert!(j.segments.len() > 1);
        j.trim(200);
        assert_eq!(j.segments.len(), 1, "only the segment being written stays");
        let oldest = j.first_seq();
        assert!(oldest > 1);
        assert!(matches!(j.read(0, 10, None), Err(ReadError::Expired { .. })));
        assert!(j.read(oldest - 1, 10, None).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
