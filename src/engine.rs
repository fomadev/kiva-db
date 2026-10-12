// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The storage engine: an append-only, checksummed log on disk and an ordered
//! in-memory index (`BTreeMap`) mapping each key to its latest record.

use crate::compress::{self, Compression};
use crate::error::{KivaError, Result};
use crate::format::*;
use crate::hint;
use crate::jsonpath;
use crate::util::{glob_match, glob_prefix, now_secs, random_u64};
use crate::value::{Type, Value};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant};

/// Where a key's latest record lives, plus the metadata needed to answer
/// `typeof`, `scan` and TTL checks without touching the disk.
#[derive(Clone, Debug)]
pub struct Entry {
    pub offset: u64,
    pub k_size: u16,
    /// Size of the value as stored (after compression).
    pub v_size: u32,
    /// Size of the value before compression.
    pub raw_size: u32,
    pub ty: Type,
    pub flags: u8,
    pub timestamp: i64,
    /// Unix time after which the key is gone; 0 means it never expires.
    pub expires_at: i64,
    /// Engine sequence number of the write that produced this entry.
    pub version: u64,
}

impl Entry {
    pub fn rec_len(&self) -> u64 {
        REC_HEADER_LEN as u64 + self.k_size as u64 + self.v_size as u64
    }

    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at > 0 && self.expires_at <= now
    }

    pub fn is_msgpack(&self) -> bool {
        self.flags & FLAG_MSGPACK != 0
    }

    pub fn compression(&self) -> Compression {
        Compression::from_flag(self.flags & FLAG_COMP_MASK)
    }
}

/// A committed mutation, delivered to listeners in commit order.
#[derive(Clone, Debug)]
pub enum Op {
    Set {
        key: String,
        ty: Type,
        /// Canonical (uncompressed) payload.
        raw: Vec<u8>,
        msgpack: bool,
        timestamp: i64,
        expires_at: i64,
        created: bool,
    },
    Del {
        key: String,
    },
    Reset,
}

pub enum BatchOp {
    Put { key: String, value: Value, expires_at: i64, msgpack: bool },
    Del { key: String },
}

impl BatchOp {
    pub fn key(&self) -> &str {
        match self {
            BatchOp::Put { key, .. } | BatchOp::Del { key } => key,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EngineOptions {
    pub compression: Compression,
    pub auto_compact: bool,
    /// Fragmentation percentage that triggers a background compaction.
    pub compact_threshold: u8,
    /// Auto-compaction never runs on files smaller than this.
    pub auto_compact_min_bytes: u64,
    /// The index snapshot (hint file) is refreshed after this many seconds...
    pub snapshot_interval_secs: u64,
    /// ...or after this many modifications, whichever comes first.
    pub snapshot_every_ops: u64,
    /// fsync after every write instead of once per second.
    pub sync_writes: bool,
    /// Run the maintenance thread (TTL sweep, auto-compaction, snapshots).
    pub background: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            compression: Compression::None,
            auto_compact: true,
            compact_threshold: 30,
            auto_compact_min_bytes: 64 * 1024,
            snapshot_interval_secs: 300,
            snapshot_every_ops: 1000,
            sync_writes: false,
            background: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineStats {
    pub path: String,
    pub keys: usize,
    pub ttl_keys: usize,
    pub file_bytes: u64,
    pub live_bytes: u64,
    pub fragmentation: f64,
    pub memory_bytes: u64,
    pub raw_value_bytes: u64,
    pub stored_value_bytes: u64,
    pub compressed_records: usize,
    pub compression: &'static str,
    pub load_mode: &'static str,
    pub load_ms: f64,
    pub recovered_bytes: u64,
    pub recovered_compaction: bool,
    pub corrupted_detected: u64,
    /// (timestamp, ok, corrupted) of the last `check`.
    pub last_check: Option<(i64, usize, usize)>,
    pub compactions: u64,
    pub last_compact: Option<i64>,
    pub auto_compact: bool,
    pub compact_threshold: u8,
    pub indexes: usize,
    pub seq: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CheckReport {
    pub total: usize,
    pub ok: usize,
    pub corrupted: Vec<String>,
    pub repaired: usize,
    pub backup: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct CompactReport {
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub records: usize,
    pub dropped_corrupted: usize,
    pub aborted: bool,
}

#[derive(Clone, Debug)]
pub struct HintInfo {
    pub path: PathBuf,
    pub status: String,
    pub entries: usize,
    pub size: u64,
    pub modified: Option<i64>,
    pub checksum_ok: bool,
    pub compression: &'static str,
}

// ---------------------------------------------------------------------------
// Secondary indexes and queries
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct OrdF64(pub f64);

impl PartialEq for OrdF64 {
    fn eq(&self, o: &Self) -> bool {
        self.0.total_cmp(&o.0).is_eq()
    }
}
impl Eq for OrdF64 {}
impl PartialOrd for OrdF64 {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for OrdF64 {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// Sortable form of a scalar JSON value. Variant order groups values by kind.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SKey {
    Null,
    Bool(bool),
    Num(OrdF64),
    Str(String),
}

impl SKey {
    pub fn from_json(j: &Json) -> Option<SKey> {
        match j {
            Json::Null => Some(SKey::Null),
            Json::Bool(b) => Some(SKey::Bool(*b)),
            Json::Number(n) => n.as_f64().map(|f| SKey::Num(OrdF64(f))),
            Json::String(s) => Some(SKey::Str(s.clone())),
            _ => None,
        }
    }

    fn same_kind(&self, o: &SKey) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(o)
    }
}

struct SecIndex {
    pattern: String,
    field: String,
    path: Vec<jsonpath::Seg>,
    map: BTreeMap<SKey, BTreeSet<String>>,
    rev: HashMap<String, SKey>,
}

impl SecIndex {
    fn remove(&mut self, key: &str) {
        if let Some(sk) = self.rev.remove(key) {
            if let Some(set) = self.map.get_mut(&sk) {
                set.remove(key);
                if set.is_empty() {
                    self.map.remove(&sk);
                }
            }
        }
    }

    fn update(&mut self, key: &str, doc: Option<&Json>) {
        if !glob_match(&self.pattern, key) {
            return;
        }
        self.remove(key);
        if let Some(sk) = doc.and_then(|d| jsonpath::get(d, &self.path)).and_then(SKey::from_json) {
            self.map.entry(sk.clone()).or_default().insert(key.to_string());
            self.rev.insert(key.to_string(), sk);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl CmpOp {
    pub fn parse(s: &str) -> Option<CmpOp> {
        Some(match s {
            "=" | "==" => CmpOp::Eq,
            "!=" | "<>" => CmpOp::Ne,
            ">" => CmpOp::Gt,
            ">=" => CmpOp::Ge,
            "<" => CmpOp::Lt,
            "<=" => CmpOp::Le,
            _ => return None,
        })
    }

    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
        }
    }
}

/// `<pattern>.<field> <op> <value>`, e.g. `user:*.role = "admin"`.
#[derive(Clone, Debug)]
pub struct Cond {
    pub pattern: String,
    pub field: String,
    pub op: CmpOp,
    pub value: Json,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub strategy: &'static str,
    pub index: Option<String>,
    /// Keys the engine expects to examine.
    pub candidates: usize,
    pub total_keys: usize,
}

pub fn compare_json(a: &Json, op: CmpOp, b: &Json) -> bool {
    use std::cmp::Ordering::*;
    let ord = match (a, b) {
        (Json::Number(x), Json::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x.partial_cmp(&y),
            _ => None,
        },
        (Json::String(x), Json::String(y)) => Some(x.cmp(y)),
        (Json::Bool(x), Json::Bool(y)) => {
            return match op {
                CmpOp::Eq => x == y,
                CmpOp::Ne => x != y,
                _ => false,
            }
        }
        (Json::Null, Json::Null) => Some(Equal),
        _ => None,
    };
    match (ord, op) {
        (None, CmpOp::Ne) => true,
        (None, _) => false,
        (Some(o), CmpOp::Eq) => o == Equal,
        (Some(o), CmpOp::Ne) => o != Equal,
        (Some(o), CmpOp::Gt) => o == Greater,
        (Some(o), CmpOp::Ge) => o != Less,
        (Some(o), CmpOp::Lt) => o == Less,
        (Some(o), CmpOp::Le) => o != Greater,
    }
}

fn pattern_covers(index_pattern: &str, query_pattern: &str) -> bool {
    if index_pattern == query_pattern || index_pattern == "*" {
        return true;
    }
    match index_pattern.strip_suffix('*') {
        Some(prefix) if !prefix.contains(['*', '?']) => query_pattern.starts_with(prefix),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

struct LoadInfo {
    mode: &'static str,
    ms: f64,
    recovered_bytes: u64,
    recovered_compaction: bool,
}

struct Inner {
    file: File,
    file_len: u64,
    file_id: u64,
    index: BTreeMap<String, Entry>,
    /// File header plus the records the index currently points to.
    live_bytes: u64,
    ttl_count: usize,
    seq: u64,
    compression: Compression,
    auto_compact: bool,
    threshold: u8,
    sec: Vec<SecIndex>,
    active_txns: usize,
    /// Versions of keys deleted while a transaction was open.
    tomb_versions: HashMap<String, u64>,
    reset_seq: u64,
    dirty_ops: u64,
    unsynced: bool,
    last_hint: Instant,
    ticks: u64,
    load: LoadInfo,
    corrupted_detected: u64,
    last_check: Option<(i64, usize, usize)>,
    compactions: u64,
    last_compact: Option<i64>,
    closed: bool,
}

struct Signal {
    wake: Mutex<bool>,
    cv: Condvar,
    stop: AtomicBool,
}

type Listener = Box<dyn Fn(&Op) + Send + Sync>;

pub struct Engine {
    path: PathBuf,
    inner: Mutex<Inner>,
    listeners: RwLock<Vec<Listener>>,
    signal: Arc<Signal>,
    compacting: AtomicBool,
    opts: EngineOptions,
    _lock: File,
}

fn sidecar(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(KivaError::Invalid("key cannot be empty".into()));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(KivaError::Invalid(format!("key is longer than {MAX_KEY_LEN} bytes")));
    }
    Ok(())
}

struct ScanOutcome {
    /// Length of the valid prefix of the log.
    end: u64,
    corrupted: u64,
}

/// Replays the log from `start`, applying records to `index`.
fn scan_log(
    file: &File,
    start: u64,
    file_len: u64,
    index: &mut BTreeMap<String, Entry>,
    seq: &mut u64,
) -> std::io::Result<ScanOutcome> {
    let mut r = BufReader::with_capacity(1 << 20, file);
    r.seek(SeekFrom::Start(start))?;
    let mut pos = start;
    let mut corrupted = 0u64;
    let mut hdr = [0u8; REC_HEADER_LEN];
    let mut body: Vec<u8> = Vec::new();
    // Records of an atomic batch wait here until its commit record shows up.
    let mut pending: Vec<(String, Option<Entry>)> = Vec::new();
    let mut pending_start = 0u64;

    let mut apply = |index: &mut BTreeMap<String, Entry>, key: String, entry: Option<Entry>| {
        *seq += 1;
        match entry {
            Some(mut e) => {
                e.version = *seq;
                index.insert(key, e);
            }
            None => {
                index.remove(&key);
            }
        }
    };

    while pos < file_len {
        if file_len - pos < REC_HEADER_LEN as u64 {
            break; // torn header
        }
        r.read_exact(&mut hdr)?;
        let h = parse_header(&hdr);
        let total = h.total_len();
        if h.k_size == 0 || pos + total > file_len {
            break; // torn or unframed record
        }
        body.resize(h.k_size as usize + h.v_size as usize, 0);
        r.read_exact(&mut body)?;

        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&hdr[4..]);
        hasher.update(&body);
        let tombstone = h.flags & FLAG_TOMBSTONE != 0;
        let key = std::str::from_utf8(&body[..h.k_size as usize]).ok();
        let ty = Type::from_u8(h.ty);
        let valid = hasher.finalize() == h.crc && key.is_some() && (tombstone || ty.is_some());
        if !valid {
            if pos + total == file_len {
                break; // last record: a torn write
            }
            corrupted += 1;
            pending.clear();
            pos += total;
            continue;
        }
        let key = key.unwrap().to_string();
        let entry = if tombstone {
            None
        } else {
            let comp = h.flags & FLAG_COMP_MASK != 0;
            let k = h.k_size as usize;
            let raw_size = if comp && h.v_size >= 4 {
                u32::from_le_bytes([body[k], body[k + 1], body[k + 2], body[k + 3]])
            } else {
                h.v_size
            };
            Some(Entry {
                offset: pos,
                k_size: h.k_size,
                v_size: h.v_size,
                raw_size,
                ty: ty.unwrap(),
                flags: h.flags,
                timestamp: h.timestamp,
                expires_at: h.expires_at,
                version: 0,
            })
        };
        if h.flags & FLAG_TXN != 0 {
            if pending.is_empty() {
                pending_start = pos;
            }
            pending.push((key, entry));
            if h.flags & FLAG_TXN_COMMIT != 0 {
                for (k, e) in pending.drain(..) {
                    apply(index, k, e);
                }
            }
        } else {
            pending.clear();
            apply(index, key, entry);
        }
        pos += total;
    }
    // A batch without its commit record never happened.
    let end = if pending.is_empty() { pos } else { pending_start };
    Ok(ScanOutcome { end, corrupted })
}

impl Inner {
    fn read_at(&mut self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Reads and checksums the record behind `e`.
    fn read_record(&mut self, key: &str, e: &Entry) -> Result<Vec<u8>> {
        let rec = match self.read_at(e.offset, e.rec_len() as usize) {
            Ok(r) => r,
            Err(_) => {
                self.corrupted_detected += 1;
                return Err(KivaError::Corrupted(format!("record of '{key}' cannot be read")));
            }
        };
        if !verify_record(&rec) {
            self.corrupted_detected += 1;
            return Err(KivaError::Corrupted(format!("checksum mismatch on key '{key}'")));
        }
        Ok(rec)
    }

    fn read_value(&mut self, key: &str, e: &Entry) -> Result<Value> {
        let rec = self.read_record(key, e)?;
        let raw = compress::unpack(&rec[REC_HEADER_LEN + e.k_size as usize..], e.compression())?;
        Value::decode(e.ty, &raw, e.is_msgpack())
    }

    fn append(&mut self, buf: &[u8], sync: bool) -> Result<u64> {
        if self.closed {
            return Err(KivaError::Other("database is closed".into()));
        }
        let offset = self.file_len;
        let res = self.file.seek(SeekFrom::Start(offset)).and_then(|_| self.file.write_all(buf)).and_then(|_| {
            if sync {
                self.file.sync_data()
            } else {
                Ok(())
            }
        });
        if let Err(e) = res {
            // Leave no half-written record behind.
            let _ = self.file.set_len(offset);
            return Err(e.into());
        }
        self.file_len += buf.len() as u64;
        self.unsynced = !sync;
        Ok(offset)
    }

    fn idx_insert(&mut self, key: &str, e: Entry, doc: Option<&Json>) {
        self.live_bytes += e.rec_len();
        if e.expires_at > 0 {
            self.ttl_count += 1;
        }
        if let Some(old) = self.index.insert(key.to_string(), e) {
            self.live_bytes -= old.rec_len();
            if old.expires_at > 0 {
                self.ttl_count -= 1;
            }
        }
        for s in &mut self.sec {
            s.update(key, doc);
        }
    }

    fn idx_remove(&mut self, key: &str) -> Option<Entry> {
        let old = self.index.remove(key)?;
        self.live_bytes -= old.rec_len();
        if old.expires_at > 0 {
            self.ttl_count -= 1;
        }
        for s in &mut self.sec {
            s.remove(key);
        }
        Some(old)
    }

    /// The entry for `key`, dropping it first if its TTL has elapsed.
    fn live(&mut self, key: &str, now: i64) -> Option<Entry> {
        let e = self.index.get(key)?;
        if e.is_expired(now) {
            self.idx_remove(key);
            return None;
        }
        Some(e.clone())
    }

    fn fragmentation(&self) -> f64 {
        if self.file_len > self.live_bytes && self.file_len > 0 {
            1.0 - self.live_bytes as f64 / self.file_len as f64
        } else {
            0.0
        }
    }

    fn recount(&mut self) {
        self.live_bytes = FILE_HEADER_LEN + self.index.values().map(|e| e.rec_len()).sum::<u64>();
        self.ttl_count = self.index.values().filter(|e| e.expires_at > 0).count();
    }

    fn purge_expired(&mut self, now: i64) -> usize {
        if self.ttl_count == 0 {
            return 0;
        }
        let dead: Vec<String> = self.index.iter().filter(|(_, e)| e.is_expired(now)).map(|(k, _)| k.clone()).collect();
        for k in &dead {
            self.idx_remove(k);
        }
        dead.len()
    }

    fn rebuild_sec(&mut self, i: usize) {
        self.sec[i].map.clear();
        self.sec[i].rev.clear();
        let pattern = self.sec[i].pattern.clone();
        let prefix = glob_prefix(&pattern).to_string();
        let now = now_secs();
        let keys: Vec<(String, Entry)> = self
            .index
            .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, e)| e.ty.is_doc() && !e.is_expired(now) && glob_match(&pattern, k))
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect();
        for (k, e) in keys {
            if let Ok(Value::Doc(doc)) = self.read_value(&k, &e) {
                self.sec[i].update(&k, Some(&doc));
            }
        }
    }

    fn write_hint(&mut self, path: &Path) {
        let _ = self.file.sync_data();
        self.unsynced = false;
        let hp = hint::hint_path(path);
        if hint::write_hint(&hp, self.file_id, self.file_len, &self.index, self.compression).is_ok() {
            self.dirty_ops = 0;
            self.last_hint = Instant::now();
        }
    }
}

/// Encodes a put as a full record plus the index entry (offset left at 0).
fn build_put(
    compression: Compression,
    key: &str,
    value: &Value,
    expires_at: i64,
    msgpack: bool,
    timestamp: i64,
    extra_flags: u8,
) -> Result<(Vec<u8>, Entry, Vec<u8>)> {
    validate_key(key)?;
    let ty = value.ty();
    let msgpack = msgpack && ty.is_doc();
    let raw = value.encode(msgpack)?;
    if raw.len() > MAX_VALUE_LEN {
        return Err(KivaError::Invalid("value is larger than 256 MB".into()));
    }
    let (stored, cflag) = compress::pack(&raw, compression);
    let mut flags = cflag | extra_flags;
    if msgpack {
        flags |= FLAG_MSGPACK;
    }
    let rec = encode_record(key, &stored, ty as u8, flags, timestamp, expires_at);
    let entry = Entry {
        offset: 0,
        k_size: key.len() as u16,
        v_size: stored.len() as u32,
        raw_size: raw.len() as u32,
        ty,
        flags,
        timestamp,
        expires_at,
        version: 0,
    };
    Ok((rec, entry, raw))
}

impl Engine {
    pub fn open(path: impl AsRef<Path>, opts: EngineOptions) -> Result<Arc<Engine>> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let lock_file = OpenOptions::new().create(true).write(true).truncate(false).open(sidecar(&path, "lock"))?;
        if lock_file.try_lock().is_err() {
            return Err(KivaError::Locked(path.display().to_string()));
        }

        let started = Instant::now();
        // A leftover temp file means a compaction was interrupted; the
        // original data file is still authoritative.
        let compact_tmp = sidecar(&path, "compact");
        let recovered_compaction = compact_tmp.exists();
        let _ = fs::remove_file(&compact_tmp);

        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        let mut file_len = file.metadata()?.len();
        let mut index = BTreeMap::new();
        let mut seq = 0u64;
        let mut corrupted = 0u64;
        let mut recovered_bytes = 0u64;
        let file_id;
        let mode;

        if file_len == 0 {
            file_id = random_u64();
            file.write_all(&file_header(file_id))?;
            file.sync_data()?;
            file_len = FILE_HEADER_LEN;
            mode = "new";
        } else {
            let mut h = [0u8; FILE_HEADER_LEN as usize];
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut h).map_err(|_| KivaError::Format("file header is truncated".into()))?;
            file_id = parse_file_header(&h).map_err(KivaError::Format)?;

            let mut scan_from = FILE_HEADER_LEN;
            let mut m = "scan";
            if let Ok(hd) = hint::read_hint(&hint::hint_path(&path)) {
                if hd.file_id == file_id && hd.data_len <= file_len && hd.data_len >= FILE_HEADER_LEN {
                    for (k, mut e) in hd.entries {
                        seq += 1;
                        e.version = seq;
                        index.insert(k, e);
                    }
                    scan_from = hd.data_len;
                    m = "hint";
                }
            }
            mode = m;
            let outcome = scan_log(&file, scan_from, file_len, &mut index, &mut seq)?;
            corrupted = outcome.corrupted;
            if outcome.end < file_len {
                recovered_bytes = file_len - outcome.end;
                // Losing a few bytes is a torn final write. Losing a lot means
                // the framing broke mid-file: keep a copy before cutting.
                if recovered_bytes > 4 * 1024 * 1024 {
                    let _ = fs::copy(&path, sidecar(&path, "bak"));
                }
                file.set_len(outcome.end)?;
                file.sync_data()?;
                file_len = outcome.end;
            }
        }

        let now = now_secs();
        index.retain(|_, e| !e.is_expired(now));

        let mut inner = Inner {
            file,
            file_len,
            file_id,
            index,
            live_bytes: 0,
            ttl_count: 0,
            seq,
            compression: opts.compression,
            auto_compact: opts.auto_compact,
            threshold: opts.compact_threshold,
            sec: Vec::new(),
            active_txns: 0,
            tomb_versions: HashMap::new(),
            reset_seq: 0,
            dirty_ops: 0,
            unsynced: false,
            last_hint: Instant::now(),
            ticks: 0,
            load: LoadInfo { mode, ms: 0.0, recovered_bytes, recovered_compaction },
            corrupted_detected: corrupted,
            last_check: None,
            compactions: 0,
            last_compact: None,
            closed: false,
        };
        inner.recount();

        // Secondary index definitions live in a small sidecar file.
        if let Ok(text) = fs::read_to_string(sidecar(&path, "meta")) {
            if let Ok(meta) = serde_json::from_str::<Json>(&text) {
                for def in meta["indexes"].as_array().cloned().unwrap_or_default() {
                    if let (Some(p), Some(f)) = (def["pattern"].as_str(), def["field"].as_str()) {
                        if let Ok(segs) = jsonpath::parse_path(f) {
                            inner.sec.push(SecIndex {
                                pattern: p.to_string(),
                                field: f.to_string(),
                                path: segs,
                                map: BTreeMap::new(),
                                rev: HashMap::new(),
                            });
                            let i = inner.sec.len() - 1;
                            inner.rebuild_sec(i);
                        }
                    }
                }
            }
        }
        inner.load.ms = started.elapsed().as_secs_f64() * 1000.0;

        let signal = Arc::new(Signal { wake: Mutex::new(false), cv: Condvar::new(), stop: AtomicBool::new(false) });
        let engine = Arc::new(Engine {
            path,
            inner: Mutex::new(inner),
            listeners: RwLock::new(Vec::new()),
            signal: signal.clone(),
            compacting: AtomicBool::new(false),
            opts: opts.clone(),
            _lock: lock_file,
        });
        if opts.background {
            let weak = Arc::downgrade(&engine);
            let _ =
                std::thread::Builder::new().name("kiva-maintenance".into()).spawn(move || maintenance(weak, signal));
        }
        Ok(engine)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Registers a callback invoked, in commit order, for every mutation.
    /// Callbacks run while the engine is locked: they must be quick and must
    /// not call back into the engine.
    pub fn add_listener(&self, f: impl Fn(&Op) + Send + Sync + 'static) {
        self.listeners.write().unwrap_or_else(|e| e.into_inner()).push(Box::new(f));
    }

    fn emit(&self, op: &Op) {
        for l in self.listeners.read().unwrap_or_else(|e| e.into_inner()).iter() {
            l(op);
        }
    }

    fn after_write(&self, inner: &Inner) {
        let frag_due = inner.auto_compact
            && inner.file_len >= self.opts.auto_compact_min_bytes
            && inner.fragmentation() * 100.0 >= inner.threshold as f64;
        if frag_due || inner.dirty_ops >= self.opts.snapshot_every_ops {
            self.wake();
        }
    }

    fn wake(&self) {
        *self.signal.wake.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.signal.cv.notify_all();
    }

    // -- reads --------------------------------------------------------------

    pub fn get(&self, key: &str) -> Result<Option<(Value, Entry)>> {
        let mut inner = self.lock();
        let Some(e) = inner.live(key, now_secs()) else {
            return Ok(None);
        };
        let v = inner.read_value(key, &e)?;
        Ok(Some((v, e)))
    }

    /// Canonical payload bytes of `key` (used by replication).
    pub fn get_raw(&self, key: &str) -> Result<Option<(Vec<u8>, Entry)>> {
        let mut inner = self.lock();
        let Some(e) = inner.live(key, now_secs()) else {
            return Ok(None);
        };
        let rec = inner.read_record(key, &e)?;
        let raw = compress::unpack(&rec[REC_HEADER_LEN + e.k_size as usize..], e.compression())?;
        Ok(Some((raw, e)))
    }

    pub fn meta(&self, key: &str) -> Option<Entry> {
        self.lock().live(key, now_secs())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.meta(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.lock().index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn seq(&self) -> u64 {
        self.lock().seq
    }

    /// Keys in `[from, to]` (inclusive, either bound optional), in key order
    /// or reversed. `limit == 0` means no limit.
    pub fn range(&self, from: Option<&str>, to: Option<&str>, reverse: bool, limit: usize) -> Vec<(String, Entry)> {
        let inner = self.lock();
        let now = now_secs();
        let (lo, hi) = match (from, to) {
            (Some(a), Some(b)) if a > b => (Some(b), Some(a)),
            other => other,
        };
        let lo = lo.map_or(Bound::Unbounded, Bound::Included);
        let hi = hi.map_or(Bound::Unbounded, Bound::Included);
        let it = inner.index.range::<str, _>((lo, hi)).filter(|(_, e)| !e.is_expired(now));
        let take = if limit == 0 { usize::MAX } else { limit };
        if reverse {
            it.rev().take(take).map(|(k, e)| (k.clone(), e.clone())).collect()
        } else {
            it.take(take).map(|(k, e)| (k.clone(), e.clone())).collect()
        }
    }

    /// Keys matching a glob pattern, in key order.
    pub fn keys_matching(&self, pattern: &str, limit: usize) -> Vec<(String, Entry)> {
        let inner = self.lock();
        let now = now_secs();
        let prefix = glob_prefix(pattern);
        let take = if limit == 0 { usize::MAX } else { limit };
        inner
            .index
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(prefix))
            .filter(|(k, e)| !e.is_expired(now) && glob_match(pattern, k))
            .take(take)
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect()
    }

    // -- writes -------------------------------------------------------------

    /// Stores `value` under `key`. Returns whether the key was created.
    pub fn put(&self, key: &str, value: &Value, expires_at: i64, msgpack: bool) -> Result<bool> {
        self.put_at(key, value, expires_at, msgpack, now_secs())
    }

    fn put_at(&self, key: &str, value: &Value, expires_at: i64, msgpack: bool, timestamp: i64) -> Result<bool> {
        let mut inner = self.lock();
        let (rec, mut entry, raw) = build_put(inner.compression, key, value, expires_at, msgpack, timestamp, 0)?;
        let created = inner.live(key, now_secs()).is_none();
        entry.offset = inner.append(&rec, self.opts.sync_writes)?;
        inner.seq += 1;
        entry.version = inner.seq;
        let msgpack = entry.is_msgpack();
        let doc = match value {
            Value::Doc(j) => Some(j),
            _ => None,
        };
        inner.idx_insert(key, entry, doc);
        inner.dirty_ops += 1;
        self.emit(&Op::Set { key: key.to_string(), ty: value.ty(), raw, msgpack, timestamp, expires_at, created });
        self.after_write(&inner);
        Ok(created)
    }

    /// Applies a replicated write exactly as the primary recorded it.
    pub fn put_raw(
        &self,
        key: &str,
        ty: Type,
        raw: &[u8],
        msgpack: bool,
        timestamp: i64,
        expires_at: i64,
    ) -> Result<()> {
        let value = Value::decode(ty, raw, msgpack)?;
        self.put_at(key, &value, expires_at, msgpack, timestamp).map(|_| ())
    }

    pub fn delete(&self, key: &str) -> Result<bool> {
        let mut inner = self.lock();
        if inner.live(key, now_secs()).is_none() {
            return Ok(false);
        }
        let rec = encode_record(key, &[], 0, FLAG_TOMBSTONE, now_secs(), 0);
        inner.append(&rec, self.opts.sync_writes)?;
        inner.seq += 1;
        inner.idx_remove(key);
        if inner.active_txns > 0 {
            let seq = inner.seq;
            inner.tomb_versions.insert(key.to_string(), seq);
        }
        inner.dirty_ops += 1;
        self.emit(&Op::Del { key: key.to_string() });
        self.after_write(&inner);
        Ok(true)
    }

    /// Applies several operations atomically: either every record of the
    /// batch is on disk and applied, or (after a crash) none is.
    ///
    /// With `begin_seq`, the batch is rejected if any of its keys was
    /// modified by someone else after that sequence number.
    pub fn apply_batch(&self, ops: Vec<BatchOp>, begin_seq: Option<u64>) -> Result<()> {
        let mut inner = self.lock();
        let now = now_secs();
        if let Some(begin) = begin_seq {
            if inner.reset_seq > begin {
                return Err(KivaError::Conflict("the database was reset during the transaction".into()));
            }
            for op in &ops {
                let key = op.key();
                let ver = inner
                    .index
                    .get(key)
                    .map(|e| e.version)
                    .or_else(|| inner.tomb_versions.get(key).copied())
                    .unwrap_or(0);
                if ver > begin {
                    return Err(KivaError::Conflict(format!("key '{key}' was modified by another session")));
                }
            }
        }

        enum Staged {
            Put { key: String, entry: Entry, raw: Vec<u8>, doc: Option<Json>, ty: Type, created: bool, rec_off: usize },
            Del { key: String },
        }
        let mut staged = Vec::with_capacity(ops.len());
        let mut recs: Vec<Vec<u8>> = Vec::with_capacity(ops.len());
        // Later operations in the batch must see earlier ones.
        let mut shadow: HashMap<String, bool> = HashMap::new();
        for op in ops {
            match op {
                BatchOp::Put { key, value, expires_at, msgpack } => {
                    let (rec, entry, raw) =
                        build_put(inner.compression, &key, &value, expires_at, msgpack, now, FLAG_TXN)?;
                    let exists = match shadow.get(&key) {
                        Some(e) => *e,
                        None => inner.live(&key, now).is_some(),
                    };
                    shadow.insert(key.clone(), true);
                    let doc = match value {
                        Value::Doc(j) => Some(j),
                        _ => None,
                    };
                    staged.push(Staged::Put { key, ty: entry.ty, entry, raw, doc, created: !exists, rec_off: 0 });
                    recs.push(rec);
                }
                BatchOp::Del { key } => {
                    let exists = match shadow.get(&key) {
                        Some(e) => *e,
                        None => inner.live(&key, now).is_some(),
                    };
                    if !exists {
                        continue;
                    }
                    shadow.insert(key.clone(), false);
                    recs.push(encode_record(&key, &[], 0, FLAG_TOMBSTONE | FLAG_TXN, now, 0));
                    staged.push(Staged::Del { key });
                }
            }
        }
        if recs.is_empty() {
            return Ok(());
        }
        // Mark the final record as the commit point (flags byte is covered by
        // the CRC, so re-stamp it).
        let last = recs.last_mut().unwrap();
        last[11] |= FLAG_TXN_COMMIT;
        let crc = crc32fast::hash(&last[4..]);
        last[..4].copy_from_slice(&crc.to_le_bytes());

        let mut buf = Vec::with_capacity(recs.iter().map(|r| r.len()).sum());
        let mut offs = Vec::with_capacity(recs.len());
        for r in &recs {
            offs.push(buf.len());
            buf.extend_from_slice(r);
        }
        let last_idx = staged.len() - 1;
        for (i, s) in staged.iter_mut().enumerate() {
            if let Staged::Put { entry, rec_off, .. } = s {
                *rec_off = offs[i];
                if i == last_idx {
                    entry.flags |= FLAG_TXN_COMMIT;
                }
            }
        }
        let base = inner.append(&buf, self.opts.sync_writes)?;

        for s in staged {
            inner.seq += 1;
            let seq = inner.seq;
            match s {
                Staged::Put { key, mut entry, raw, doc, ty, created, rec_off } => {
                    entry.offset = base + rec_off as u64;
                    entry.version = seq;
                    let (msgpack, timestamp, expires_at) = (entry.is_msgpack(), entry.timestamp, entry.expires_at);
                    inner.idx_insert(&key, entry, doc.as_ref());
                    self.emit(&Op::Set { key, ty, raw, msgpack, timestamp, expires_at, created });
                }
                Staged::Del { key } => {
                    inner.idx_remove(&key);
                    if inner.active_txns > 0 {
                        inner.tomb_versions.insert(key.clone(), seq);
                    }
                    self.emit(&Op::Del { key });
                }
            }
            inner.dirty_ops += 1;
        }
        self.after_write(&inner);
        Ok(())
    }

    /// Wipes every key. The data file is recreated from scratch.
    pub fn reset(&self) -> Result<()> {
        let mut inner = self.lock();
        if inner.closed {
            return Err(KivaError::Other("database is closed".into()));
        }
        let file_id = random_u64();
        inner.file.set_len(0)?;
        inner.file.seek(SeekFrom::Start(0))?;
        inner.file.write_all(&file_header(file_id))?;
        inner.file.sync_data()?;
        inner.file_id = file_id;
        inner.file_len = FILE_HEADER_LEN;
        inner.index.clear();
        for s in &mut inner.sec {
            s.map.clear();
            s.rev.clear();
        }
        inner.recount();
        inner.seq += 1;
        inner.reset_seq = inner.seq;
        inner.tomb_versions.clear();
        inner.dirty_ops = 0;
        let _ = fs::remove_file(hint::hint_path(&self.path));
        self.emit(&Op::Reset);
        Ok(())
    }

    // -- transactions -------------------------------------------------------

    /// Marks a transaction as open and returns the sequence number it starts from.
    pub fn txn_begin(&self) -> u64 {
        let mut inner = self.lock();
        inner.active_txns += 1;
        inner.seq
    }

    pub fn txn_end(&self) {
        let mut inner = self.lock();
        inner.active_txns = inner.active_txns.saturating_sub(1);
        if inner.active_txns == 0 {
            inner.tomb_versions.clear();
        }
    }

    // -- maintenance --------------------------------------------------------

    pub fn set_compression(&self, c: Compression) {
        self.lock().compression = c;
    }

    pub fn compression(&self) -> Compression {
        self.lock().compression
    }

    pub fn set_auto_compact(&self, on: bool, threshold: u8) {
        let mut inner = self.lock();
        inner.auto_compact = on;
        inner.threshold = threshold.clamp(1, 99);
    }

    pub fn auto_compact(&self) -> (bool, u8) {
        let inner = self.lock();
        (inner.auto_compact, inner.threshold)
    }

    pub fn is_compacting(&self) -> bool {
        self.compacting.load(Ordering::SeqCst)
    }

    pub fn compact(&self) -> Result<CompactReport> {
        self.compact_with(None)
    }

    /// Rewrites the data file keeping only live records. Reads and writes
    /// keep flowing while the bulk of the copy happens; with `recompress`,
    /// every value is re-encoded with the given algorithm.
    pub fn compact_with(&self, recompress: Option<Compression>) -> Result<CompactReport> {
        if self.compacting.swap(true, Ordering::SeqCst) {
            return Err(KivaError::Other("a compaction is already running".into()));
        }
        struct Clear<'a>(&'a AtomicBool);
        impl Drop for Clear<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _clear = Clear(&self.compacting);
        let tmp_path = sidecar(&self.path, "compact");
        let res = self.compact_impl(recompress, &tmp_path);
        if !matches!(&res, Ok(r) if !r.aborted) {
            let _ = fs::remove_file(&tmp_path);
        }
        res
    }

    fn compact_impl(&self, recompress: Option<Compression>, tmp_path: &Path) -> Result<CompactReport> {
        // Phase 1 (locked): freeze a view of the index.
        let (snapshot, snap_len, old_id) = {
            let mut inner = self.lock();
            if inner.closed {
                return Err(KivaError::Other("database is closed".into()));
            }
            inner.purge_expired(now_secs());
            if let Some(c) = recompress {
                inner.compression = c;
            }
            let _ = inner.file.sync_data();
            let snap: Vec<(String, Entry)> = inner.index.iter().map(|(k, e)| (k.clone(), e.clone())).collect();
            (snap, inner.file_len, inner.file_id)
        };

        // Phase 2 (unlocked): copy live records into a fresh file.
        let new_id = random_u64();
        let mut reader = File::open(&self.path)?;
        let mut out = BufWriter::with_capacity(1 << 20, File::create(tmp_path)?);
        out.write_all(&file_header(new_id))?;
        let mut pos = FILE_HEADER_LEN;
        let mut moved: HashMap<String, (u64, Entry)> = HashMap::with_capacity(snapshot.len());
        let mut dropped = 0usize;
        let mut buf = Vec::new();
        for (key, e) in &snapshot {
            buf.resize(e.rec_len() as usize, 0);
            let read_ok = reader.seek(SeekFrom::Start(e.offset)).and_then(|_| reader.read_exact(&mut buf)).is_ok();
            if !read_ok || !verify_record(&buf) {
                dropped += 1;
                continue;
            }
            let mut ne = e.clone();
            let has_txn_flags = e.flags & (FLAG_TXN | FLAG_TXN_COMMIT) != 0;
            let wrong_algo = recompress.is_some_and(|c| c != e.compression());
            if has_txn_flags || wrong_algo {
                let stored = &buf[REC_HEADER_LEN + e.k_size as usize..];
                let (stored2, cflag) = if wrong_algo {
                    match compress::unpack(stored, e.compression()) {
                        Ok(raw) => compress::pack(&raw, recompress.unwrap()),
                        Err(_) => {
                            dropped += 1;
                            continue;
                        }
                    }
                } else {
                    (stored.to_vec(), e.flags & FLAG_COMP_MASK)
                };
                ne.flags = (e.flags & FLAG_MSGPACK) | cflag;
                ne.v_size = stored2.len() as u32;
                let rec = encode_record(key, &stored2, e.ty as u8, ne.flags, e.timestamp, e.expires_at);
                out.write_all(&rec)?;
                ne.offset = pos;
                pos += rec.len() as u64;
            } else {
                out.write_all(&buf)?;
                ne.offset = pos;
                pos += buf.len() as u64;
            }
            moved.insert(key.clone(), (e.offset, ne));
        }
        out.flush()?;
        let mut tmp_file = out.into_inner().map_err(|e| KivaError::Io(e.into_error()))?;
        drop(reader);

        // Phase 3 (locked): bring over whatever was appended meanwhile, then swap.
        let mut inner = self.lock();
        let before_bytes = inner.file_len;
        if inner.file_id != old_id || inner.closed {
            return Ok(CompactReport { before_bytes, after_bytes: before_bytes, aborted: true, ..Default::default() });
        }
        let tail_start = pos;
        if inner.file_len > snap_len {
            let tail_len = (inner.file_len - snap_len) as usize;
            let tail = inner.read_at(snap_len, tail_len)?;
            tmp_file.write_all(&tail)?;
            pos += tail.len() as u64;
        }
        tmp_file.sync_all()?;
        drop(tmp_file);

        let mut new_index = BTreeMap::new();
        let mut lost: Vec<String> = Vec::new();
        for (k, cur) in &inner.index {
            if cur.offset >= snap_len {
                let mut e = cur.clone();
                e.offset = cur.offset - snap_len + tail_start;
                new_index.insert(k.clone(), e);
            } else if let Some((old_off, ne)) = moved.get(k).filter(|(o, _)| *o == cur.offset) {
                let _ = old_off;
                let mut e = ne.clone();
                e.version = cur.version;
                new_index.insert(k.clone(), e);
            } else {
                lost.push(k.clone());
            }
        }

        // Close our handle on the old file before replacing it (required on Windows).
        let new_handle = OpenOptions::new().read(true).write(true).open(tmp_path)?;
        let old_handle = std::mem::replace(&mut inner.file, new_handle);
        drop(old_handle);
        let mut renamed = fs::rename(tmp_path, &self.path);
        for _ in 0..20 {
            if renamed.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
            renamed = fs::rename(tmp_path, &self.path);
        }
        if let Err(e) = renamed {
            inner.file = OpenOptions::new().read(true).write(true).open(&self.path)?;
            return Err(e.into());
        }

        for k in &lost {
            inner.idx_remove(k);
        }
        let records = new_index.len();
        inner.index = new_index;
        inner.file_len = pos;
        inner.file_id = new_id;
        inner.recount();
        inner.compactions += 1;
        inner.last_compact = Some(now_secs());
        inner.corrupted_detected += dropped as u64;
        inner.write_hint(&self.path);
        Ok(CompactReport { before_bytes, after_bytes: pos, records, dropped_corrupted: dropped, aborted: false })
    }

    /// Verifies the checksum of every live record. With `repair`, a backup
    /// (`.bak`) is taken and corrupted entries are removed.
    pub fn check(&self, repair: bool) -> Result<CheckReport> {
        let mut inner = self.lock();
        inner.purge_expired(now_secs());
        let entries: Vec<(String, Entry)> = inner.index.iter().map(|(k, e)| (k.clone(), e.clone())).collect();
        let mut report = CheckReport { total: entries.len(), ..Default::default() };
        let before = inner.corrupted_detected;
        for (k, e) in &entries {
            match inner.read_value(k, e) {
                Ok(_) => report.ok += 1,
                Err(_) => report.corrupted.push(k.clone()),
            }
        }
        // `read_value` already counted these; count each bad record once.
        inner.corrupted_detected = before + report.corrupted.len() as u64;
        if repair && !report.corrupted.is_empty() {
            let _ = inner.file.sync_data();
            let bak = sidecar(&self.path, "bak");
            fs::copy(&self.path, &bak)?;
            report.backup = Some(bak);
            for k in &report.corrupted {
                let rec = encode_record(k, &[], 0, FLAG_TOMBSTONE, now_secs(), 0);
                inner.append(&rec, true)?;
                inner.seq += 1;
                inner.idx_remove(k);
                inner.dirty_ops += 1;
                self.emit(&Op::Del { key: k.clone() });
                report.repaired += 1;
            }
        }
        inner.last_check = Some((now_secs(), report.ok, report.corrupted.len()));
        Ok(report)
    }

    pub fn hint_info(&self) -> HintInfo {
        let inner = self.lock();
        let path = hint::hint_path(&self.path);
        let meta = fs::metadata(&path).ok();
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        if meta.is_none() {
            return HintInfo {
                path,
                status: "MISSING".into(),
                entries: 0,
                size: 0,
                modified: None,
                checksum_ok: false,
                compression: "none",
            };
        }
        match hint::read_hint(&path) {
            Ok(h) => {
                let status = if h.file_id != inner.file_id || h.data_len > inner.file_len {
                    "STALE (belongs to an older data file)".to_string()
                } else if h.data_len < inner.file_len {
                    format!("VALID (+{} bytes of log replayed at startup)", inner.file_len - h.data_len)
                } else {
                    "VALID".to_string()
                };
                HintInfo {
                    path,
                    status,
                    entries: h.entries.len(),
                    size,
                    modified,
                    checksum_ok: true,
                    compression: h.compression.name(),
                }
            }
            Err(_) => HintInfo {
                path,
                status: "INVALID".into(),
                entries: 0,
                size,
                modified,
                checksum_ok: false,
                compression: "none",
            },
        }
    }

    pub fn stats(&self) -> EngineStats {
        let inner = self.lock();
        let mut s = EngineStats {
            path: self.path.display().to_string(),
            keys: inner.index.len(),
            ttl_keys: inner.ttl_count,
            file_bytes: inner.file_len,
            live_bytes: inner.live_bytes,
            fragmentation: inner.fragmentation(),
            compression: inner.compression.name(),
            load_mode: inner.load.mode,
            load_ms: inner.load.ms,
            recovered_bytes: inner.load.recovered_bytes,
            recovered_compaction: inner.load.recovered_compaction,
            corrupted_detected: inner.corrupted_detected,
            last_check: inner.last_check,
            compactions: inner.compactions,
            last_compact: inner.last_compact,
            auto_compact: inner.auto_compact,
            compact_threshold: inner.threshold,
            indexes: inner.sec.len(),
            seq: inner.seq,
            ..Default::default()
        };
        let mut mem = std::mem::size_of::<Engine>() + std::mem::size_of::<Inner>();
        for (k, e) in &inner.index {
            // Allocated key bytes + the entry + B-tree node overhead.
            mem += k.capacity() + std::mem::size_of::<Entry>() + 48;
            s.raw_value_bytes += e.raw_size as u64;
            s.stored_value_bytes += e.v_size as u64;
            if e.flags & FLAG_COMP_MASK != 0 {
                s.compressed_records += 1;
            }
        }
        s.memory_bytes = mem as u64;
        s
    }

    /// Writes the index snapshot now.
    pub fn snapshot(&self) {
        let mut inner = self.lock();
        if !inner.closed {
            inner.write_hint(&self.path);
        }
    }

    /// Flushes everything and writes the index snapshot. Idempotent.
    pub fn close(&self) {
        self.signal.stop.store(true, Ordering::SeqCst);
        self.wake();
        let mut inner = self.lock();
        if inner.closed {
            return;
        }
        inner.purge_expired(now_secs());
        inner.write_hint(&self.path);
        inner.closed = true;
    }

    fn tick(&self) {
        let compact_due = {
            let mut inner = self.lock();
            if inner.closed {
                return;
            }
            inner.ticks += 1;
            if inner.ticks.is_multiple_of(5) {
                inner.purge_expired(now_secs());
            }
            if inner.unsynced {
                let _ = inner.file.sync_data();
                inner.unsynced = false;
            }
            let snapshot_due = inner.dirty_ops >= self.opts.snapshot_every_ops
                || (inner.dirty_ops > 0
                    && inner.last_hint.elapsed() >= Duration::from_secs(self.opts.snapshot_interval_secs));
            if snapshot_due {
                inner.write_hint(&self.path);
            }
            inner.auto_compact
                && inner.file_len >= self.opts.auto_compact_min_bytes
                && inner.fragmentation() * 100.0 >= inner.threshold as f64
        };
        if compact_due {
            let _ = self.compact();
        }
    }

    // -- secondary indexes --------------------------------------------------

    fn save_meta(&self, inner: &Inner) -> Result<()> {
        let defs: Vec<Json> =
            inner.sec.iter().map(|s| serde_json::json!({ "pattern": s.pattern, "field": s.field })).collect();
        let text = serde_json::to_string_pretty(&serde_json::json!({ "indexes": defs }))?;
        fs::write(sidecar(&self.path, "meta"), text)?;
        Ok(())
    }

    /// Creates an index on `field` of every document whose key matches
    /// `pattern`. Returns the number of documents indexed.
    pub fn create_index(&self, pattern: &str, field: &str) -> Result<usize> {
        let path = jsonpath::parse_path(field).map_err(KivaError::Invalid)?;
        if path.is_empty() {
            return Err(KivaError::Invalid("an index needs a field name".into()));
        }
        let mut inner = self.lock();
        if inner.sec.iter().any(|s| s.pattern == pattern && s.field == field) {
            return Err(KivaError::Exists(format!("index on {pattern} ({field}) already exists")));
        }
        inner.sec.push(SecIndex {
            pattern: pattern.to_string(),
            field: field.to_string(),
            path,
            map: BTreeMap::new(),
            rev: HashMap::new(),
        });
        let i = inner.sec.len() - 1;
        inner.rebuild_sec(i);
        let n = inner.sec[i].rev.len();
        self.save_meta(&inner)?;
        Ok(n)
    }

    pub fn drop_index(&self, pattern: &str, field: &str) -> Result<bool> {
        let mut inner = self.lock();
        let before = inner.sec.len();
        inner.sec.retain(|s| !(s.pattern == pattern && s.field == field));
        let removed = inner.sec.len() != before;
        if removed {
            self.save_meta(&inner)?;
        }
        Ok(removed)
    }

    /// `(pattern, field, indexed documents)` for every index.
    pub fn list_indexes(&self) -> Vec<(String, String, usize)> {
        self.lock().sec.iter().map(|s| (s.pattern.clone(), s.field.clone(), s.rev.len())).collect()
    }

    fn find_index(inner: &Inner, cond: &Cond) -> Option<usize> {
        if cond.op == CmpOp::Ne || SKey::from_json(&cond.value).is_none() {
            return None;
        }
        inner.sec.iter().position(|s| s.field == cond.field && pattern_covers(&s.pattern, &cond.pattern))
    }

    fn index_candidates(idx: &SecIndex, cond: &Cond) -> Vec<String> {
        let Some(sk) = SKey::from_json(&cond.value) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut push = |set: &BTreeSet<String>| out.extend(set.iter().cloned());
        match cond.op {
            CmpOp::Eq => {
                if let Some(set) = idx.map.get(&sk) {
                    push(set);
                }
            }
            CmpOp::Gt | CmpOp::Ge => {
                let lo = if cond.op == CmpOp::Gt { Bound::Excluded(&sk) } else { Bound::Included(&sk) };
                for (k, set) in idx.map.range((lo, Bound::Unbounded)) {
                    if !k.same_kind(&sk) {
                        break;
                    }
                    push(set);
                }
            }
            CmpOp::Lt | CmpOp::Le => {
                let hi = if cond.op == CmpOp::Lt { Bound::Excluded(&sk) } else { Bound::Included(&sk) };
                for (k, set) in idx.map.range((Bound::Unbounded, hi)).rev() {
                    if !k.same_kind(&sk) {
                        break;
                    }
                    push(set);
                }
            }
            CmpOp::Ne => {}
        }
        out.sort();
        out
    }

    /// How `cond` would be executed, without running it.
    pub fn explain(&self, cond: &Cond) -> Plan {
        let inner = self.lock();
        Self::plan(&inner, cond)
    }

    fn plan(inner: &Inner, cond: &Cond) -> Plan {
        let total_keys = inner.index.len();
        if let Some(i) = Self::find_index(inner, cond) {
            let idx = &inner.sec[i];
            return Plan {
                strategy: "INDEX SCAN",
                index: Some(format!("{} ({})", idx.pattern, idx.field)),
                candidates: Self::index_candidates(idx, cond).len(),
                total_keys,
            };
        }
        let prefix = glob_prefix(&cond.pattern);
        let candidates = inner
            .index
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(prefix))
            .count();
        Plan {
            strategy: if prefix.is_empty() { "FULL SCAN" } else { "PREFIX SCAN" },
            index: None,
            candidates,
            total_keys,
        }
    }

    /// Documents whose key matches `cond.pattern` and whose field satisfies
    /// the comparison. `limit == 0` means no limit.
    pub fn query(&self, cond: &Cond, limit: usize) -> Result<(Vec<(String, Json)>, Plan)> {
        let path = jsonpath::parse_path(&cond.field).map_err(KivaError::Invalid)?;
        let mut inner = self.lock();
        let plan = Self::plan(&inner, cond);
        let now = now_secs();
        let candidates: Vec<String> = match Self::find_index(&inner, cond) {
            Some(i) => Self::index_candidates(&inner.sec[i], cond),
            None => {
                let prefix = glob_prefix(&cond.pattern);
                inner
                    .index
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .take_while(|(k, _)| k.starts_with(prefix))
                    .filter(|(_, e)| e.ty.is_doc())
                    .map(|(k, _)| k.clone())
                    .collect()
            }
        };
        let mut rows = Vec::new();
        for key in candidates {
            if !glob_match(&cond.pattern, &key) {
                continue;
            }
            let Some(e) = inner.live(&key, now) else {
                continue;
            };
            if !e.ty.is_doc() {
                continue;
            }
            let Value::Doc(doc) = inner.read_value(&key, &e)? else {
                continue;
            };
            let hit = jsonpath::get(&doc, &path).is_some_and(|v| compare_json(v, cond.op, &cond.value));
            if hit {
                rows.push((key, doc));
                if limit != 0 && rows.len() >= limit {
                    break;
                }
            }
        }
        Ok((rows, plan))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.close();
    }
}

fn maintenance(weak: Weak<Engine>, signal: Arc<Signal>) {
    loop {
        {
            let mut woken = signal.wake.lock().unwrap_or_else(|e| e.into_inner());
            if !*woken {
                woken = signal.cv.wait_timeout(woken, Duration::from_secs(1)).unwrap_or_else(|e| e.into_inner()).0;
            }
            *woken = false;
        }
        if signal.stop.load(Ordering::SeqCst) {
            break;
        }
        match weak.upgrade() {
            Some(engine) => engine.tick(),
            None => break,
        }
    }
}
