// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The storage engine: an append-only, checksummed log on disk and an ordered
//! in-memory index (`BTreeMap`) mapping each key to its latest record.

use crate::cache::ValueCache;
use crate::compress::{self, Compression};
use crate::crypt::{self, Keys};
use crate::error::{KivaError, Result};
use crate::format::*;
use crate::hint;
use crate::hnsw::{self, Hnsw};
use crate::jsonpath;
use crate::util::{glob_match, glob_prefix, now_secs, random_u64};
use crate::value::{Type, Value};
pub use crate::vector::Kind as VectorIndexKind;
use crate::vector::{self, Metric, VecIndex};
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
    /// Memory budget of the value cache in bytes; 0 = no cache.
    pub cache_bytes: usize,
    /// Memory bound of this workspace in bytes; 0 = no bound. See [`Engine::check_room`].
    pub max_memory_bytes: usize,
    /// Encryption at rest: new records are encrypted with the current key.
    pub keys: Option<Arc<Keys>>,
    /// Refuse unencrypted records (and an unencrypted index snapshot).
    pub require_encryption: bool,
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
            cache_bytes: 0,
            max_memory_bytes: 0,
            keys: None,
            require_encryption: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineStats {
    pub cache_limit: u64,
    pub cache_bytes: u64,
    pub cache_entries: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub vector_indexes: usize,
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
    /// Why the last automatic compaction failed, if it did.
    pub compact_error: Option<String>,
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
// Equal under `total_cmp` means the same bits, so hashing the bits agrees with `eq`.
impl std::hash::Hash for OrdF64 {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.0.to_bits().hash(h);
    }
}

/// Sortable form of a scalar JSON value. Variant order groups values by kind.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// How a secondary index keeps the values of its field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IndexKind {
    /// Values in order (a B-tree): answers `=`, `>`, `>=`, `<` and `<=`.
    #[default]
    BTree,
    /// A hash table: answers `=` only, in constant time whatever the number
    /// of distinct values.
    Hash,
}

impl IndexKind {
    pub fn name(self) -> &'static str {
        match self {
            IndexKind::BTree => "btree",
            IndexKind::Hash => "hash",
        }
    }

    pub fn parse(s: &str) -> Option<IndexKind> {
        match s.to_ascii_lowercase().as_str() {
            "btree" | "b-tree" => Some(IndexKind::BTree),
            "hash" => Some(IndexKind::Hash),
            _ => None,
        }
    }

    fn serves(self, op: CmpOp) -> bool {
        match self {
            IndexKind::BTree => op != CmpOp::Ne,
            IndexKind::Hash => op == CmpOp::Eq,
        }
    }
}

/// Field value -> keys of the documents holding it.
enum Postings {
    Tree(BTreeMap<SKey, BTreeSet<String>>),
    Hash(HashMap<SKey, BTreeSet<String>>),
}

impl Postings {
    fn new(kind: IndexKind) -> Postings {
        match kind {
            IndexKind::BTree => Postings::Tree(BTreeMap::new()),
            IndexKind::Hash => Postings::Hash(HashMap::new()),
        }
    }

    fn get(&self, sk: &SKey) -> Option<&BTreeSet<String>> {
        match self {
            Postings::Tree(m) => m.get(sk),
            Postings::Hash(m) => m.get(sk),
        }
    }

    fn add(&mut self, sk: SKey, key: &str) {
        match self {
            Postings::Tree(m) => m.entry(sk).or_default(),
            Postings::Hash(m) => m.entry(sk).or_default(),
        }
        .insert(key.to_string());
    }

    fn take(&mut self, sk: &SKey, key: &str) {
        let set = match self {
            Postings::Tree(m) => m.get_mut(sk),
            Postings::Hash(m) => m.get_mut(sk),
        };
        let emptied = set.is_some_and(|set| set.remove(key) && set.is_empty());
        if emptied {
            match self {
                Postings::Tree(m) => m.remove(sk),
                Postings::Hash(m) => m.remove(sk),
            };
        }
    }

    fn distinct(&self) -> usize {
        match self {
            Postings::Tree(m) => m.len(),
            Postings::Hash(m) => m.len(),
        }
    }
}

struct SecIndex {
    pattern: String,
    field: String,
    path: Vec<jsonpath::Seg>,
    kind: IndexKind,
    map: Postings,
    rev: HashMap<String, SKey>,
}

impl SecIndex {
    fn new(pattern: &str, field: &str, path: Vec<jsonpath::Seg>, kind: IndexKind) -> SecIndex {
        SecIndex {
            pattern: pattern.to_string(),
            field: field.to_string(),
            path,
            kind,
            map: Postings::new(kind),
            rev: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.map = Postings::new(self.kind);
        self.rev.clear();
    }

    fn remove(&mut self, key: &str) {
        if let Some(sk) = self.rev.remove(key) {
            self.map.take(&sk, key);
        }
    }

    fn update(&mut self, key: &str, doc: Option<&Json>) {
        if !glob_match(&self.pattern, key) {
            return;
        }
        let sk = doc.and_then(|d| jsonpath::get(d, &self.path)).and_then(SKey::from_json);
        // A write that leaves the indexed field as it was costs nothing more.
        if self.rev.get(key) == sk.as_ref() {
            return;
        }
        self.remove(key);
        if let Some(sk) = sk {
            self.map.add(sk.clone(), key);
            self.rev.insert(key.to_string(), sk);
        }
    }
}

/// Memory taken by one key in the in-memory index: its bytes, the entry and
/// the B-tree node overhead.
fn key_cost(key: &str) -> usize {
    key.len() + std::mem::size_of::<String>() + std::mem::size_of::<Entry>() + 48
}

/// Where this workspace's memory goes, in bytes (estimates).
#[derive(Clone, Debug, Default)]
pub struct MemoryReport {
    /// Every key and the position of its value: always in memory.
    pub keys: usize,
    pub key_count: usize,
    /// Secondary indexes.
    pub secondary: usize,
    /// Vector indexes and HNSW graphs.
    pub vectors: usize,
    /// Values kept in memory by `memory pin`.
    pub pinned: usize,
    pub pinned_values: usize,
    pub pinned_patterns: Vec<String>,
    /// Values of the value cache: given back first when memory runs short.
    pub cache: usize,
    /// `storage.max_memory_mb` in bytes; 0 = no bound.
    pub limit: usize,
}

impl MemoryReport {
    /// What cannot be given back without deleting data or indexes.
    pub fn resident(&self) -> usize {
        self.keys + self.secondary + self.vectors + self.pinned
    }

    pub fn total(&self) -> usize {
        self.resident() + self.cache
    }
}

/// What `indexes` shows about one secondary index.
#[derive(Clone, Debug)]
pub struct IndexInfo {
    pub pattern: String,
    pub field: String,
    pub kind: IndexKind,
    /// Documents that have the field.
    pub documents: usize,
    /// Distinct values of the field.
    pub values: usize,
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
    vecs: Vec<VecIndex>,
    cache: ValueCache,
    /// Sum of `key_cost` over the index, kept up to date on every change.
    key_bytes: usize,
    /// Memory of the vector indexes, refreshed by the maintenance thread
    /// (walking them on every write would cost too much).
    vec_bytes: usize,
    max_memory: usize,
    keys: Option<Arc<Keys>>,
    active_txns: usize,
    /// Versions of keys deleted while a transaction was open.
    tomb_versions: HashMap<String, u64>,
    reset_seq: u64,
    dirty_ops: u64,
    unsynced: bool,
    last_hint: Instant,
    /// When the HNSW graphs were last written to their file.
    last_graph_save: Instant,
    ticks: u64,
    load: LoadInfo,
    corrupted_detected: u64,
    last_check: Option<(i64, usize, usize)>,
    compactions: u64,
    last_compact: Option<i64>,
    compact_error: Option<String>,
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

/// Replays the log from `start`, applying records to `index`. Encrypted
/// records are decrypted with `keys`; one that needs a key this engine does
/// not have stops the scan with an error.
fn scan_log(
    file: &File,
    start: u64,
    file_len: u64,
    index: &mut BTreeMap<String, Entry>,
    seq: &mut u64,
    keys: Option<&Keys>,
    require_encryption: bool,
) -> Result<ScanOutcome> {
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
        let crc_ok = hasher.finalize() == h.crc;
        let tombstone = h.flags & FLAG_TOMBSTONE != 0;
        // The key and the stored value, decrypted if need be.
        let mut opened = None;
        if crc_ok {
            if h.flags & FLAG_ENCRYPTED != 0 {
                let mut rec = hdr.to_vec();
                rec.extend_from_slice(&body);
                match crypt::open_record(keys, &rec) {
                    Ok(kv) => opened = Some(kv),
                    Err(crypt::OpenError::Tampered) => {}
                    Err(e) => return Err(e.into_error("this workspace")),
                }
            } else if require_encryption {
                return Err(KivaError::Denied(format!(
                    "encryption.required is on, but the data file holds an unencrypted record at byte {pos}: \
                     turn it off, run 'compact' to encrypt the whole file, then turn it on again"
                )));
            } else {
                let k = h.k_size as usize;
                opened = Some((body[..k].to_vec(), body[k..].to_vec()));
            }
        }
        let key = opened.as_ref().and_then(|(k, _)| std::str::from_utf8(k).ok());
        let ty = Type::from_u8(h.ty);
        let valid = crc_ok && key.is_some() && (tombstone || ty.is_some());
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
            let stored = opened.as_ref().map(|(_, v)| v.as_slice()).unwrap_or_default();
            let raw_size = if comp && stored.len() >= 4 {
                u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]])
            } else {
                stored.len() as u32
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

    /// The value bytes of the record behind `e`, decrypted and decompressed.
    fn read_raw(&mut self, key: &str, e: &Entry) -> Result<Vec<u8>> {
        let rec = self.read_record(key, e)?;
        let (_, stored) = crypt::open_record(self.keys.as_deref(), &rec).map_err(|err| {
            if err == crypt::OpenError::Tampered {
                self.corrupted_detected += 1;
            }
            err.into_error(&format!("the record of '{key}'"))
        })?;
        compress::unpack(&stored, e.compression())
    }

    fn read_value(&mut self, key: &str, e: &Entry) -> Result<Value> {
        let raw = self.read_raw(key, e)?;
        Value::decode(e.ty, &raw, e.is_msgpack())
    }

    /// Encodes a record, encrypted when this engine has a key.
    fn encode(&self, key: &str, value: &[u8], ty: u8, flags: u8, timestamp: i64, expires_at: i64) -> Vec<u8> {
        crypt::encode_record(self.keys.as_deref(), key, value, ty, flags, timestamp, expires_at)
    }

    /// Like `read_value`, served from the value cache when possible.
    fn value_of(&mut self, key: &str, e: &Entry) -> Result<Value> {
        if let Some(v) = self.cache.get(key, e.version) {
            return Ok(v);
        }
        let v = self.read_value(key, e)?;
        self.cache.put(key, e.version, &v, e.raw_size as usize);
        Ok(v)
    }

    fn rebuild_vec(&mut self, i: usize) {
        self.vecs[i].clear();
        let pattern = self.vecs[i].pattern.clone();
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
                self.vecs[i].update(&k, Some(&doc));
            }
        }
    }

    /// Brings a graph read from disk up to date: it reflected the data up to
    /// byte `since` of the log, so only the keys written after that, and the
    /// keys deleted since, need looking at.
    fn catch_up_vec(&mut self, i: usize, since: u64) {
        let now = now_secs();
        let gone: Vec<String> = self.vecs[i]
            .keys()
            .into_iter()
            .filter(|k| self.index.get(k).is_none_or(|e| e.is_expired(now) || !e.ty.is_doc()))
            .collect();
        for k in &gone {
            self.vecs[i].remove(k);
        }
        let pattern = self.vecs[i].pattern.clone();
        let prefix = glob_prefix(&pattern).to_string();
        let written: Vec<(String, Entry)> = self
            .index
            .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, e)| e.offset >= since && !e.is_expired(now) && glob_match(&pattern, k))
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect();
        for (k, e) in written {
            match self.read_value(&k, &e) {
                Ok(Value::Doc(doc)) => self.vecs[i].update(&k, Some(&doc)),
                _ => self.vecs[i].update(&k, None),
            }
        }
    }

    /// Writes the HNSW graphs to `<data>.graphs`, so that a restart does not
    /// build them again. Written to a temporary file first, then renamed.
    fn save_graphs(&mut self, path: &Path) {
        let file = sidecar(path, "graphs");
        if !self.vecs.iter().any(|v| v.graph().is_some()) {
            let _ = fs::remove_file(&file);
            return;
        }
        let _ = self.file.sync_data();
        self.unsynced = false;
        let mut out = Vec::new();
        out.extend_from_slice(GRAPHS_MAGIC);
        out.extend_from_slice(&self.file_id.to_le_bytes());
        out.extend_from_slice(&self.file_len.to_le_bytes());
        let graphs: Vec<&VecIndex> = self.vecs.iter().filter(|v| v.graph().is_some()).collect();
        out.extend_from_slice(&(graphs.len() as u32).to_le_bytes());
        for v in graphs {
            hnsw::write_str(&mut out, &v.pattern);
            hnsw::write_str(&mut out, &v.field);
            out.extend_from_slice(&(v.dims.unwrap_or(0) as u32).to_le_bytes());
            out.extend_from_slice(&(v.rejected as u64).to_le_bytes());
            if let Some(g) = v.graph() {
                g.write(&mut out);
            }
        }
        let crc = crc32fast::hash(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        if let Some(keys) = &self.keys {
            keys.seal(crypt::Domain::Graphs, &self.file_id.to_le_bytes(), &mut out);
            out.splice(0..0, GRAPHS_SEALED.iter().copied());
        }
        let tmp = sidecar(path, "graphs.tmp");
        let written = File::create(&tmp).and_then(|mut f| {
            f.write_all(&out)?;
            f.sync_all()
        });
        if written.is_ok() && fs::rename(&tmp, &file).is_ok() {
            for v in &mut self.vecs {
                if let Some(g) = v.graph_mut() {
                    g.changes = 0;
                }
            }
            self.last_graph_save = Instant::now();
        } else {
            let _ = fs::remove_file(&tmp);
        }
    }

    fn graph_changes(&self) -> u64 {
        self.vecs.iter().filter_map(|v| v.graph()).map(|g| g.changes).sum()
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
        match self.index.insert(key.to_string(), e) {
            Some(old) => {
                self.live_bytes -= old.rec_len();
                if old.expires_at > 0 {
                    self.ttl_count -= 1;
                }
            }
            None => self.key_bytes += key_cost(key),
        }
        for s in &mut self.sec {
            s.update(key, doc);
        }
        for v in &mut self.vecs {
            v.update(key, doc);
        }
        self.cache.remove(key);
    }

    fn idx_remove(&mut self, key: &str) -> Option<Entry> {
        let old = self.index.remove(key)?;
        self.key_bytes -= key_cost(key);
        self.live_bytes -= old.rec_len();
        if old.expires_at > 0 {
            self.ttl_count -= 1;
        }
        for s in &mut self.sec {
            s.remove(key);
        }
        for v in &mut self.vecs {
            v.remove(key);
        }
        self.cache.remove(key);
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
        self.key_bytes = self.index.keys().map(|k| key_cost(k)).sum();
    }

    fn refresh_vec_bytes(&mut self) {
        self.vec_bytes = self.vecs.iter().map(|v| v.memory_bytes()).sum();
    }

    fn memory(&self) -> MemoryReport {
        let n = self.index.len();
        let avg_key = self.key_bytes.checked_div(n).map_or(0, |b| b - key_cost(""));
        // Per indexed document: the key twice (reverse map and value set),
        // the value, and the nodes holding them.
        let per_doc = 2 * (avg_key + std::mem::size_of::<String>()) + std::mem::size_of::<SKey>() + 64;
        MemoryReport {
            keys: self.key_bytes,
            key_count: n,
            secondary: self.sec.iter().map(|s| s.rev.len() * per_doc).sum(),
            vectors: self.vec_bytes,
            pinned: self.cache.pinned_bytes(),
            pinned_values: self.cache.pinned_len(),
            pinned_patterns: self.cache.patterns().to_vec(),
            cache: self.cache.bytes(),
            limit: self.max_memory,
        }
    }

    /// Reads into memory the values of the live keys matching `pattern`.
    fn warm_pinned(&mut self, pattern: &str) {
        let prefix = glob_prefix(pattern).to_string();
        let now = now_secs();
        let keys: Vec<(String, Entry)> = self
            .index
            .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, e)| !e.is_expired(now) && glob_match(pattern, k))
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect();
        for (k, e) in keys {
            if let Ok(v) = self.read_value(&k, &e) {
                self.cache.put(&k, e.version, &v, e.raw_size as usize);
            }
        }
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
        self.sec[i].clear();
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
        if hint::write_hint(&hp, self.file_id, self.file_len, &self.index, self.compression, self.keys.as_deref())
            .is_ok()
        {
            self.dirty_ops = 0;
            self.last_hint = Instant::now();
        }
    }
}

/// Encodes a put as a full record plus the index entry (offset left at 0).
/// `inner` gives the compression and the encryption key.
fn build_put(
    inner: &Inner,
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
    let (stored, cflag) = compress::pack(&raw, inner.compression);
    let mut flags = cflag | extra_flags;
    if msgpack {
        flags |= FLAG_MSGPACK;
    }
    let rec = inner.encode(key, &stored, ty as u8, flags, timestamp, expires_at);
    let h = parse_header(&rec);
    let entry = Entry {
        offset: 0,
        k_size: h.k_size,
        v_size: h.v_size,
        raw_size: raw.len() as u32,
        ty,
        flags: h.flags,
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
        if !crate::util::lock_exclusive(&lock_file) {
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
            if let Ok(hd) = hint::read_hint(&hint::hint_path(&path), opts.keys.as_deref()) {
                // With encryption required, a snapshot written in clear (or
                // listing records in clear) is not trusted: scan the log.
                let trusted = !opts.require_encryption
                    || (hd.encrypted && hd.entries.iter().all(|(_, e)| e.flags & FLAG_ENCRYPTED != 0));
                if trusted && hd.file_id == file_id && hd.data_len <= file_len && hd.data_len >= FILE_HEADER_LEN {
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
            let outcome = scan_log(
                &file,
                scan_from,
                file_len,
                &mut index,
                &mut seq,
                opts.keys.as_deref(),
                opts.require_encryption,
            )?;
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
        // A missing or wrong key shows now, not at the first read.
        if let Some((k, e)) = index.iter().find(|(_, e)| e.flags & FLAG_ENCRYPTED != 0) {
            let mut rec = vec![0u8; e.rec_len() as usize];
            file.seek(SeekFrom::Start(e.offset))?;
            file.read_exact(&mut rec)?;
            match crypt::open_record(opts.keys.as_deref(), &rec) {
                Ok(_) | Err(crypt::OpenError::Tampered) => {}
                Err(err) => return Err(err.into_error(&format!("this workspace (record '{k}')"))),
            }
        }

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
            vecs: Vec::new(),
            cache: ValueCache::new(opts.cache_bytes),
            key_bytes: 0,
            vec_bytes: 0,
            max_memory: opts.max_memory_bytes,
            keys: opts.keys.clone(),
            active_txns: 0,
            tomb_versions: HashMap::new(),
            reset_seq: 0,
            dirty_ops: 0,
            unsynced: false,
            last_hint: Instant::now(),
            last_graph_save: Instant::now(),
            ticks: 0,
            load: LoadInfo { mode, ms: 0.0, recovered_bytes, recovered_compaction },
            corrupted_detected: corrupted,
            last_check: None,
            compactions: 0,
            last_compact: None,
            compact_error: None,
            closed: false,
        };
        inner.recount();

        // Secondary index definitions live in a small sidecar file.
        if let Ok(text) = fs::read_to_string(sidecar(&path, "meta")) {
            if let Ok(meta) = serde_json::from_str::<Json>(&text) {
                let mut saved =
                    read_graphs(&sidecar(&path, "graphs"), inner.file_id, inner.file_len, inner.keys.as_deref());
                for def in meta["vector_indexes"].as_array().cloned().unwrap_or_default() {
                    if let (Some(p), Some(f)) = (def["pattern"].as_str(), def["field"].as_str()) {
                        if let Ok(segs) = jsonpath::parse_path(f) {
                            let kind = kind_from_json(&def);
                            inner.vecs.push(VecIndex::with_kind(p, f, segs, kind));
                            let i = inner.vecs.len() - 1;
                            let restored = saved.as_mut().and_then(|(since, list)| {
                                let at = list.iter().position(|s| {
                                    s.pattern == p && s.field == f && VectorIndexKind::Hnsw(s.graph.params) == kind
                                })?;
                                Some((*since, list.swap_remove(at)))
                            });
                            match restored {
                                Some((since, s)) => {
                                    inner.vecs[i].restore(s.graph, s.dims, s.rejected);
                                    inner.catch_up_vec(i, since);
                                }
                                None => inner.rebuild_vec(i),
                            }
                        }
                    }
                }
                for p in meta["pinned"].as_array().cloned().unwrap_or_default() {
                    if let Some(p) = p.as_str() {
                        inner.cache.pin(p);
                        inner.warm_pinned(p);
                    }
                }
                for def in meta["indexes"].as_array().cloned().unwrap_or_default() {
                    if let (Some(p), Some(f)) = (def["pattern"].as_str(), def["field"].as_str()) {
                        if let Ok(segs) = jsonpath::parse_path(f) {
                            let kind = def["kind"].as_str().and_then(IndexKind::parse).unwrap_or_default();
                            inner.sec.push(SecIndex::new(p, f, segs, kind));
                            let i = inner.sec.len() - 1;
                            inner.rebuild_sec(i);
                        }
                    }
                }
            }
        }
        inner.refresh_vec_bytes();
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
        let v = inner.value_of(key, &e)?;
        Ok(Some((v, e)))
    }

    /// Canonical payload bytes of `key` (used by replication).
    pub fn get_raw(&self, key: &str) -> Result<Option<(Vec<u8>, Entry)>> {
        let mut inner = self.lock();
        let Some(e) = inner.live(key, now_secs()) else {
            return Ok(None);
        };
        let raw = inner.read_raw(key, &e)?;
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
        let created = self.put_locked(&mut inner, key, value, expires_at, msgpack, timestamp)?;
        self.after_write(&inner);
        Ok(created)
    }

    /// Reads `key`, lets `f` compute its new value and writes it, all while
    /// holding the engine lock: no other write can come in between, so two
    /// clients changing different fields of one document never undo each
    /// other's change. `f` returns the new value (`None` leaves the key as
    /// it is) and a result for the caller. The key keeps its TTL and its
    /// encoding. Returns `None` when the key does not exist, otherwise the
    /// result of `f` and whether something was written. `f` must not call
    /// back into the engine.
    pub fn update_with<T>(
        &self,
        key: &str,
        f: impl FnOnce(Value) -> Result<(Option<Value>, T)>,
    ) -> Result<Option<(T, bool)>> {
        let mut inner = self.lock();
        let Some(e) = inner.live(key, now_secs()) else {
            return Ok(None);
        };
        let current = inner.value_of(key, &e)?;
        let (new, out) = f(current)?;
        let Some(new) = new else {
            return Ok(Some((out, false)));
        };
        self.put_locked(&mut inner, key, &new, e.expires_at, e.is_msgpack(), now_secs())?;
        self.after_write(&inner);
        Ok(Some((out, true)))
    }

    fn put_locked(
        &self,
        inner: &mut Inner,
        key: &str,
        value: &Value,
        expires_at: i64,
        msgpack: bool,
        timestamp: i64,
    ) -> Result<bool> {
        let (rec, mut entry, raw) = build_put(inner, key, value, expires_at, msgpack, timestamp, 0)?;
        let created = inner.live(key, now_secs()).is_none();
        entry.offset = inner.append(&rec, self.opts.sync_writes)?;
        inner.seq += 1;
        entry.version = inner.seq;
        let msgpack = entry.is_msgpack();
        let doc = match value {
            Value::Doc(j) => Some(j),
            _ => None,
        };
        let version = entry.version;
        inner.idx_insert(key, entry, doc);
        if inner.cache.is_pinned(key) {
            inner.cache.put(key, version, value, raw.len());
        }
        inner.dirty_ops += 1;
        self.emit(&Op::Set { key: key.to_string(), ty: value.ty(), raw, msgpack, timestamp, expires_at, created });
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
        let rec = inner.encode(key, &[], 0, FLAG_TOMBSTONE, now_secs(), 0);
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
                    let (rec, entry, raw) = build_put(&inner, &key, &value, expires_at, msgpack, now, FLAG_TXN)?;
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
                    recs.push(inner.encode(&key, &[], 0, FLAG_TOMBSTONE | FLAG_TXN, now, 0));
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
                    if inner.cache.is_pinned(&key) {
                        if let Ok(v) = Value::decode(ty, &raw, msgpack) {
                            inner.cache.put(&key, seq, &v, raw.len());
                        }
                    }
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
        // Truncating the file under a running backup would cut its copy short.
        if self.compacting.load(Ordering::SeqCst) {
            return Err(KivaError::Other("a compaction or a backup is running: try again when it ends".into()));
        }
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
            s.clear();
        }
        for v in &mut inner.vecs {
            v.clear();
        }
        inner.cache.clear();
        inner.recount();
        inner.seq += 1;
        inner.reset_seq = inner.seq;
        inner.tomb_versions.clear();
        inner.dirty_ops = 0;
        let _ = fs::remove_file(hint::hint_path(&self.path));
        inner.save_graphs(&self.path);
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
            return Err(KivaError::Other("a compaction or a backup is already running".into()));
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
        let keys = self.opts.keys.clone();

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
            // Encrypted with the current key, or in clear when there is none.
            let wrong_key = crypt::record_key_id(&buf) != keys.as_deref().map(|k| k.current.id);
            if has_txn_flags || wrong_algo || wrong_key {
                let Ok((_, stored)) = crypt::open_record(keys.as_deref(), &buf) else {
                    dropped += 1;
                    continue;
                };
                let (stored2, cflag) = if wrong_algo {
                    match compress::unpack(&stored, e.compression()) {
                        Ok(raw) => compress::pack(&raw, recompress.unwrap()),
                        Err(_) => {
                            dropped += 1;
                            continue;
                        }
                    }
                } else {
                    (stored, e.flags & FLAG_COMP_MASK)
                };
                let flags = (e.flags & FLAG_MSGPACK) | cflag;
                let rec =
                    crypt::encode_record(keys.as_deref(), key, &stored2, e.ty as u8, flags, e.timestamp, e.expires_at);
                let h = parse_header(&rec);
                ne.flags = h.flags;
                ne.v_size = h.v_size;
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
        if inner.vecs.iter().any(|v| v.graph().is_some()) {
            inner.save_graphs(&self.path);
        }
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
                let rec = inner.encode(k, &[], 0, FLAG_TOMBSTONE, now_secs(), 0);
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
        match hint::read_hint(&path, inner.keys.as_deref()) {
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
            compact_error: inner.compact_error.clone(),
            auto_compact: inner.auto_compact,
            compact_threshold: inner.threshold,
            indexes: inner.sec.len(),
            vector_indexes: inner.vecs.len(),
            cache_limit: inner.cache.limit() as u64,
            cache_bytes: inner.cache.bytes() as u64,
            cache_entries: inner.cache.len(),
            cache_hits: inner.cache.hits,
            cache_misses: inner.cache.misses,
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
        mem += inner.vecs.iter().map(|v| v.memory_bytes()).sum::<usize>();
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
        if inner.graph_changes() > 0 {
            inner.save_graphs(&self.path);
        }
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
            for v in &mut inner.vecs {
                if let Some(g) = v.graph_mut().filter(|g| g.needs_rebuild()) {
                    g.rebuild();
                }
            }
            if inner.ticks.is_multiple_of(5) {
                inner.refresh_vec_bytes();
            }
            // Writing a large graph costs more than replaying the few writes it
            // misses at the next start: save it after many changes, not often.
            let changes = inner.graph_changes();
            let size: usize = inner.vecs.iter().filter_map(|v| v.graph()).map(|g| g.len()).sum();
            if changes >= (size as u64 / 10).max(1000)
                && inner.last_graph_save.elapsed() >= Duration::from_secs(self.opts.snapshot_interval_secs)
            {
                inner.save_graphs(&self.path);
            }
            inner.auto_compact
                && inner.file_len >= self.opts.auto_compact_min_bytes
                && inner.fragmentation() * 100.0 >= inner.threshold as f64
        };
        if compact_due {
            let result = self.compact();
            self.lock().compact_error = match result {
                Ok(_) => None,
                Err(e) => Some(e.to_string()),
            };
        }
    }

    /// What the data file holds, encrypted or not, record by record
    /// (current versions and the older ones a compaction has not removed yet).
    pub fn encryption_report(&self) -> Result<EncryptionReport> {
        let inner = self.lock();
        let _ = inner.file.sync_data();
        let current = inner.keys.as_ref().map(|k| k.current.id);
        let mut report =
            EncryptionReport { key_id: inner.keys.as_ref().map(|k| k.current.id_hex()), ..Default::default() };
        let len = inner.file_len;
        let mut r = BufReader::with_capacity(1 << 20, &inner.file);
        let mut pos = FILE_HEADER_LEN;
        let mut hdr = [0u8; REC_HEADER_LEN];
        while pos + REC_HEADER_LEN as u64 <= len {
            r.seek(SeekFrom::Start(pos))?;
            r.read_exact(&mut hdr)?;
            let h = parse_header(&hdr);
            let total = h.total_len();
            if h.k_size == 0 || pos + total > len {
                break;
            }
            report.records += 1;
            if h.flags & FLAG_ENCRYPTED == 0 {
                report.plain += 1;
            } else {
                let mut id = [0u8; 4];
                r.seek(SeekFrom::Start(pos + total - 4))?;
                r.read_exact(&mut id)?;
                if Some(id) != current {
                    report.other_key += 1;
                }
            }
            pos += total;
        }
        drop(r);
        report.hint_encrypted =
            hint::read_hint(&hint::hint_path(&self.path), inner.keys.as_deref()).map(|h| h.encrypted).unwrap_or(false);
        Ok(report)
    }

    /// The data file as it is now, for a backup: a handle on it, the length
    /// that holds whole records up to this instant, and the index definitions.
    /// The log only grows, so that prefix stays valid while writes go on; until
    /// the snapshot is dropped, compactions and resets wait.
    pub fn backup_snapshot(&self) -> Result<BackupSnapshot<'_>> {
        let deadline = Instant::now() + Duration::from_secs(600);
        while self.compacting.swap(true, Ordering::SeqCst) {
            if Instant::now() > deadline {
                return Err(KivaError::Other(
                    "a compaction has been running for 10 minutes: try the backup later".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // From here on, dropping the snapshot (or an error) releases the guard.
        let guard = BackupGuard(&self.compacting);
        let inner = self.lock();
        if inner.closed {
            return Err(KivaError::Other("database is closed".into()));
        }
        inner.file.sync_data()?;
        let len = inner.file_len;
        let file = File::open(&self.path)?;
        let meta = fs::read(sidecar(&self.path, "meta")).ok();
        Ok(BackupSnapshot { file, len, meta, _guard: guard })
    }

    // -- secondary indexes --------------------------------------------------

    fn save_meta(&self, inner: &Inner) -> Result<()> {
        let defs: Vec<Json> = inner
            .sec
            .iter()
            .map(|s| {
                let mut def = serde_json::json!({ "pattern": s.pattern, "field": s.field });
                // B-tree indexes keep the format older versions read.
                if s.kind != IndexKind::BTree {
                    def["kind"] = s.kind.name().into();
                }
                def
            })
            .collect();
        let vecs: Vec<Json> = inner
            .vecs
            .iter()
            .map(|v| {
                let mut def = serde_json::json!({ "pattern": v.pattern, "field": v.field });
                if let VectorIndexKind::Hnsw(p) = v.kind() {
                    def["kind"] = "hnsw".into();
                    def["metric"] = p.metric.name().into();
                    def["m"] = p.m.into();
                    def["ef_construction"] = p.ef_construction.into();
                }
                def
            })
            .collect();
        let mut meta = serde_json::json!({ "indexes": defs, "vector_indexes": vecs });
        if !inner.cache.patterns().is_empty() {
            meta["pinned"] = inner.cache.patterns().into();
        }
        let text = serde_json::to_string_pretty(&meta)?;
        fs::write(sidecar(&self.path, "meta"), text)?;
        Ok(())
    }

    /// Creates an index on `field` of every document whose key matches
    /// `pattern`. Returns the number of documents indexed.
    pub fn create_index(&self, pattern: &str, field: &str) -> Result<usize> {
        self.create_index_with(pattern, field, IndexKind::BTree)
    }

    /// Like `create_index`, choosing how the values are kept.
    pub fn create_index_with(&self, pattern: &str, field: &str, kind: IndexKind) -> Result<usize> {
        let path = jsonpath::parse_path(field).map_err(KivaError::Invalid)?;
        if path.is_empty() {
            return Err(KivaError::Invalid("an index needs a field name".into()));
        }
        let mut inner = self.lock();
        if let Some(s) = inner.sec.iter().find(|s| s.pattern == pattern && s.field == field) {
            return Err(KivaError::Exists(format!(
                "index on {pattern} ({field}) already exists ({}); drop it first to change its kind",
                s.kind.name()
            )));
        }
        inner.sec.push(SecIndex::new(pattern, field, path, kind));
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

    pub fn list_indexes(&self) -> Vec<IndexInfo> {
        self.lock()
            .sec
            .iter()
            .map(|s| IndexInfo {
                pattern: s.pattern.clone(),
                field: s.field.clone(),
                kind: s.kind,
                documents: s.rev.len(),
                values: s.map.distinct(),
            })
            .collect()
    }

    // -- memory -------------------------------------------------------------

    /// Where this workspace's memory goes.
    pub fn memory(&self) -> MemoryReport {
        let mut inner = self.lock();
        inner.refresh_vec_bytes();
        inner.memory()
    }

    /// Sets the memory bound in bytes; 0 removes it.
    pub fn set_memory_limit(&self, bytes: usize) {
        self.lock().max_memory = bytes;
    }

    /// Checks that writing `keys` fits in the memory bound. Changing a key
    /// that exists is always allowed, and so is deleting: only new keys are
    /// refused once the bound is reached, after the value cache has given
    /// its memory back. Replicas never refuse what their master sent.
    pub fn check_room<'a>(&self, keys: impl IntoIterator<Item = &'a str>) -> Result<()> {
        let mut inner = self.lock();
        if inner.max_memory == 0 {
            return Ok(());
        }
        let now = now_secs();
        let new: Vec<&str> =
            keys.into_iter().filter(|k| !inner.index.get(*k).is_some_and(|e| !e.is_expired(now))).collect();
        if new.is_empty() {
            return Ok(());
        }
        let needed: usize = new.iter().map(|k| key_cost(k)).sum();
        let mut m = inner.memory();
        if m.total() + needed > inner.max_memory && m.cache > 0 {
            inner.cache.drop_evictable();
            m = inner.memory();
        }
        if m.resident() + needed > inner.max_memory {
            return Err(KivaError::Other(format!(
                "OOM: this workspace reached its memory bound ({} of {} used), so new keys are refused. \
                 Delete keys or expired data, drop an index, or raise storage.max_memory_mb.",
                crate::util::human_size(m.resident() as u64),
                crate::util::human_size(inner.max_memory as u64)
            )));
        }
        Ok(())
    }

    /// Keeps in memory the values of the keys matching `pattern`, now and
    /// after every write, whether the value cache is on or not. Returns the
    /// number of values and their size.
    pub fn pin(&self, pattern: &str) -> Result<(usize, usize)> {
        let mut inner = self.lock();
        if inner.cache.patterns().iter().any(|p| p == pattern) {
            return Err(KivaError::Exists(format!("'{pattern}' is already pinned")));
        }
        if inner.max_memory > 0 {
            // Values to read, at their stored size: refuse before loading them.
            let prefix = glob_prefix(pattern).to_string();
            let wanted: usize = inner
                .index
                .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
                .take_while(|(k, _)| k.starts_with(&prefix))
                .filter(|(k, _)| glob_match(pattern, k) && !inner.cache.is_pinned(k))
                .map(|(k, e)| e.raw_size as usize + k.len() + 96)
                .sum();
            let m = inner.memory();
            if m.resident() + wanted > inner.max_memory {
                return Err(KivaError::Other(format!(
                    "OOM: pinning '{pattern}' needs about {} and would pass the memory bound ({} of {} used).",
                    crate::util::human_size(wanted as u64),
                    crate::util::human_size(m.resident() as u64),
                    crate::util::human_size(inner.max_memory as u64)
                )));
            }
        }
        let before = (inner.cache.pinned_len(), inner.cache.pinned_bytes());
        inner.cache.pin(pattern);
        inner.warm_pinned(pattern);
        self.save_meta(&inner)?;
        Ok((inner.cache.pinned_len() - before.0, inner.cache.pinned_bytes() - before.1))
    }

    pub fn unpin(&self, pattern: &str) -> Result<bool> {
        let mut inner = self.lock();
        let removed = inner.cache.unpin(pattern);
        if removed {
            self.save_meta(&inner)?;
        }
        Ok(removed)
    }

    // -- value cache --------------------------------------------------------

    /// Sets the memory budget of the value cache; 0 turns it off.
    pub fn set_cache_limit(&self, bytes: usize) {
        self.lock().cache.set_limit(bytes);
    }

    // -- vector search ------------------------------------------------------

    /// Keeps in memory the vectors stored in `field` of the documents whose
    /// key matches `pattern`. Returns the number of vectors indexed.
    pub fn create_vector_index(&self, pattern: &str, field: &str) -> Result<usize> {
        self.create_vector_index_with(pattern, field, VectorIndexKind::Exact)
    }

    /// Like [`Engine::create_vector_index`], exact or HNSW.
    pub fn create_vector_index_with(&self, pattern: &str, field: &str, kind: VectorIndexKind) -> Result<usize> {
        let path = jsonpath::parse_path(field).map_err(KivaError::Invalid)?;
        if path.is_empty() {
            return Err(KivaError::Invalid("a vector index needs a field name".into()));
        }
        let mut inner = self.lock();
        if inner.vecs.iter().any(|v| v.pattern == pattern && v.field == field) {
            return Err(KivaError::Exists(format!("vector index on {pattern} ({field}) already exists")));
        }
        inner.vecs.push(VecIndex::with_kind(pattern, field, path, kind));
        let i = inner.vecs.len() - 1;
        inner.rebuild_vec(i);
        let n = inner.vecs[i].len();
        self.save_meta(&inner)?;
        if kind != VectorIndexKind::Exact {
            // Building the graph took a while: keep it for the next start.
            inner.save_graphs(&self.path);
        }
        Ok(n)
    }

    pub fn drop_vector_index(&self, pattern: &str, field: &str) -> Result<bool> {
        let mut inner = self.lock();
        let before = inner.vecs.len();
        let had_graph = inner.vecs.iter().any(|v| v.graph().is_some());
        inner.vecs.retain(|v| !(v.pattern == pattern && v.field == field));
        let removed = inner.vecs.len() != before;
        if removed {
            self.save_meta(&inner)?;
            if had_graph {
                inner.save_graphs(&self.path);
            }
        }
        Ok(removed)
    }

    /// Every vector index, with its size.
    pub fn list_vector_indexes(&self) -> Vec<VectorIndexInfo> {
        self.lock()
            .vecs
            .iter()
            .map(|v| VectorIndexInfo {
                pattern: v.pattern.clone(),
                field: v.field.clone(),
                kind: v.kind(),
                vectors: v.len(),
                dims: v.dims,
                rejected: v.rejected,
                tombstones: v.graph().map_or(0, |g| g.tombstones()),
            })
            .collect()
    }

    /// The `limit` documents whose vector in `field` is closest to `vector`.
    pub fn similar(&self, q: &VectorQuery) -> Result<VectorResult> {
        if q.vector.is_empty() || q.vector.len() > vector::MAX_DIMS {
            return Err(KivaError::Invalid(format!("a query vector has 1 to {} numbers", vector::MAX_DIMS)));
        }
        if q.vector.iter().any(|x| !x.is_finite()) {
            return Err(KivaError::Invalid("a query vector contains only finite numbers".into()));
        }
        let path = jsonpath::parse_path(&q.field).map_err(KivaError::Invalid)?;
        let query = vector::Query::new(q.vector.clone(), q.metric);
        let mut inner = self.lock();
        let now = now_secs();
        let limit = if q.limit == 0 { 10 } else { q.limit };

        let index = inner.vecs.iter().position(|v| v.field == q.field && pattern_covers(&v.pattern, &q.pattern));
        let (scored, examined, indexed, approximate) = match index {
            Some(i) => {
                if let Some(d) = inner.vecs[i].dims {
                    if d != query.dims() {
                        return Err(KivaError::Invalid(format!(
                            "the query vector has {} dimensions, the indexed vectors have {d} dimensions",
                            query.dims()
                        )));
                    }
                }
                let index_ref = &inner.vecs[i];
                let live = |k: &str| inner.index.get(k).is_some_and(|e| !e.is_expired(now));
                // The graph answers searches over its whole pattern, with the
                // metric it was built for. Anything else compares every vector.
                let graph = index_ref
                    .graph()
                    .filter(|g| !q.exact && g.params.metric == q.metric && q.pattern == index_ref.pattern);
                match graph {
                    Some(g) => {
                        let ef = if q.ef == 0 { hnsw::DEFAULT_EF_SEARCH } else { q.ef };
                        let found = g.search(&query.vector, limit, ef, live);
                        let scored: Vec<(String, f32)> = found
                            .keys
                            .into_iter()
                            .filter_map(|k| index_ref.get(&k).map(|(v, n)| (k.clone(), query.score(v, n))))
                            .collect();
                        (scored, found.compared, true, true)
                    }
                    None => {
                        let scored = index_ref.scores(&q.pattern, &query, live);
                        let n = scored.len();
                        (scored, n, true, false)
                    }
                }
            }
            None => {
                let prefix = glob_prefix(&q.pattern).to_string();
                let candidates: Vec<(String, Entry)> = inner
                    .index
                    .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
                    .take_while(|(k, _)| k.starts_with(&prefix))
                    .filter(|(k, e)| e.ty.is_doc() && !e.is_expired(now) && glob_match(&q.pattern, k))
                    .map(|(k, e)| (k.clone(), e.clone()))
                    .collect();
                let examined = candidates.len();
                let mut scored = Vec::new();
                for (k, e) in candidates {
                    if let Value::Doc(doc) = inner.value_of(&k, &e)? {
                        if let Some(v) = vector::extract(&doc, &path).filter(|v| v.len() == query.dims()) {
                            scored.push((k, query.score_vec(&v)));
                        }
                    }
                }
                (scored, examined, false, false)
            }
        };
        let compared = if approximate { examined } else { scored.len() };
        let mut rows = Vec::new();
        for (key, score) in vector::top_k(scored, limit, &query) {
            let Some(e) = inner.live(&key, now) else { continue };
            if let Value::Doc(doc) = inner.value_of(&key, &e)? {
                rows.push((key, score, doc));
            }
        }
        Ok(VectorResult { rows, indexed, approximate, examined, compared })
    }

    /// The index that can answer `cond`; for `=`, a hash index is preferred.
    fn find_index(inner: &Inner, cond: &Cond) -> Option<usize> {
        // Only a scalar value can be looked up.
        SKey::from_json(&cond.value)?;
        let usable =
            |s: &SecIndex| s.kind.serves(cond.op) && s.field == cond.field && pattern_covers(&s.pattern, &cond.pattern);
        let hash = inner.sec.iter().position(|s| s.kind == IndexKind::Hash && usable(s));
        hash.or_else(|| inner.sec.iter().position(usable))
    }

    fn index_candidates(idx: &SecIndex, cond: &Cond) -> Vec<String> {
        let Some(sk) = SKey::from_json(&cond.value) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut push = |set: &BTreeSet<String>| out.extend(set.iter().cloned());
        if cond.op == CmpOp::Eq {
            if let Some(set) = idx.map.get(&sk) {
                push(set);
            }
            return out;
        }
        // Only a B-tree is ever chosen for a range.
        let Postings::Tree(map) = &idx.map else {
            return out;
        };
        match cond.op {
            CmpOp::Gt | CmpOp::Ge => {
                let lo = if cond.op == CmpOp::Gt { Bound::Excluded(&sk) } else { Bound::Included(&sk) };
                for (k, set) in map.range((lo, Bound::Unbounded)) {
                    if !k.same_kind(&sk) {
                        break;
                    }
                    push(set);
                }
            }
            CmpOp::Lt | CmpOp::Le => {
                let hi = if cond.op == CmpOp::Lt { Bound::Excluded(&sk) } else { Bound::Included(&sk) };
                for (k, set) in map.range((Bound::Unbounded, hi)).rev() {
                    if !k.same_kind(&sk) {
                        break;
                    }
                    push(set);
                }
            }
            CmpOp::Eq | CmpOp::Ne => {}
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
            let kind = if idx.kind == IndexKind::Hash { " using hash" } else { "" };
            return Plan {
                strategy: "INDEX SCAN",
                index: Some(format!("{} ({}){kind}", idx.pattern, idx.field)),
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
            let Value::Doc(doc) = inner.value_of(&key, &e)? else {
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

const GRAPHS_MAGIC: &[u8; 8] = b"KVGRAPH1";
/// An encrypted graphs file starts with this, then the sealed plain file.
const GRAPHS_SEALED: &[u8; 8] = b"KVGRAPE1";

/// An HNSW graph read back from the graphs file.
struct SavedGraph {
    pattern: String,
    field: String,
    dims: Option<usize>,
    rejected: usize,
    graph: Hnsw,
}

/// Reads the graphs file when it belongs to this data file: the same log
/// identity, and written at a point the log still reaches. Returns that
/// point and the graphs; `None` when the graphs must be built again.
fn read_graphs(file: &Path, file_id: u64, file_len: u64, keys: Option<&Keys>) -> Option<(u64, Vec<SavedGraph>)> {
    let mut buf = fs::read(file).ok()?;
    if buf.starts_with(GRAPHS_SEALED) {
        buf =
            crypt::open_with(keys, crypt::Domain::Graphs, &file_id.to_le_bytes(), &buf[GRAPHS_SEALED.len()..]).ok()?;
    }
    let (body, crc) = buf.split_at(buf.len().checked_sub(4)?);
    if crc32fast::hash(body).to_le_bytes() != crc {
        return None;
    }
    let mut r = hnsw::Reader::new(body);
    if r.take(8)? != GRAPHS_MAGIC || r.u64()? != file_id {
        return None;
    }
    let since = r.u64()?;
    if since > file_len {
        return None;
    }
    let count = r.u32()?;
    let mut list = Vec::new();
    for _ in 0..count {
        let pattern = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let field = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let dims = match r.u32()? {
            0 => None,
            d => Some(d as usize),
        };
        let rejected = r.u64()? as usize;
        let graph = Hnsw::read(&mut r)?;
        list.push(SavedGraph { pattern, field, dims, rejected, graph });
    }
    Some((since, list))
}

/// The kind of a vector index from its saved definition.
fn kind_from_json(def: &Json) -> VectorIndexKind {
    if def["kind"].as_str() != Some("hnsw") {
        return VectorIndexKind::Exact;
    }
    let mut p = hnsw::Params::default();
    if let Some(m) = def["metric"].as_str().and_then(Metric::parse) {
        p.metric = m;
    }
    if let Some(m) = def["m"].as_u64() {
        p.m = m as usize;
    }
    if let Some(ef) = def["ef_construction"].as_u64() {
        p.ef_construction = ef as usize;
    }
    VectorIndexKind::Hnsw(p)
}

/// See [`Engine::backup_snapshot`].
pub struct BackupSnapshot<'a> {
    pub file: File,
    /// Bytes of `file` to copy.
    pub len: u64,
    /// The index definitions (`.meta`), if any.
    pub meta: Option<Vec<u8>>,
    _guard: BackupGuard<'a>,
}

struct BackupGuard<'a>(&'a AtomicBool);

impl Drop for BackupGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// See [`Engine::encryption_report`].
#[derive(Clone, Debug, Default)]
pub struct EncryptionReport {
    /// Id of the key new records are encrypted with; `None` when encryption is off.
    pub key_id: Option<String>,
    /// Records in the data file, current or superseded.
    pub records: u64,
    /// Of them, records in clear.
    pub plain: u64,
    /// Of them, records encrypted with another key than the current one.
    pub other_key: u64,
    pub hint_encrypted: bool,
}

/// A similarity search: `<pattern>.<field>` closest to `vector`.
#[derive(Clone, Debug)]
pub struct VectorQuery {
    pub pattern: String,
    pub field: String,
    pub vector: Vec<f32>,
    pub metric: Metric,
    /// Number of results wanted; 0 means 10.
    pub limit: usize,
    /// Candidates an HNSW search keeps; 0 means the default (100).
    pub ef: usize,
    /// Compare every vector even when an HNSW index exists.
    pub exact: bool,
}

impl VectorQuery {
    pub fn new(pattern: &str, field: &str, vector: Vec<f32>, metric: Metric, limit: usize) -> VectorQuery {
        VectorQuery { pattern: pattern.into(), field: field.into(), vector, metric, limit, ef: 0, exact: false }
    }
}

/// A vector index as `indexes` shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorIndexInfo {
    pub pattern: String,
    pub field: String,
    pub kind: VectorIndexKind,
    pub vectors: usize,
    pub dims: Option<usize>,
    /// Documents skipped because their vector had another length.
    pub rejected: usize,
    /// HNSW: deleted or replaced vectors waiting for the graph to be rebuilt.
    pub tombstones: usize,
}

#[derive(Debug)]
pub struct VectorResult {
    /// `(key, score, document)`, best match first.
    pub rows: Vec<(String, f32, Json)>,
    /// Whether an in-memory vector index was used.
    pub indexed: bool,
    /// Whether an HNSW graph answered: fast, very rarely missing a match.
    pub approximate: bool,
    /// Keys matching the pattern that were looked at.
    pub examined: usize,
    /// Vectors actually compared (right field, right number of dimensions).
    pub compared: usize,
}
