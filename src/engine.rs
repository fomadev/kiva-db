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
use crate::query::{Expr, Query};
use crate::util::{glob_match, glob_prefix, now_ms, now_secs, random_u64};
use crate::value::{Type, Value};
pub use crate::vector::AutoTune;
pub use crate::vector::Kind as VectorIndexKind;
use crate::vector::{self, Metric, VecIndex};
use serde_json::Value as Json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// Unix time in milliseconds after which the key is gone; 0 means it
    /// never expires.
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

/// A key watched by Redis `WATCH`, with the engine sequence at that time.
#[derive(Clone, Debug)]
pub struct WatchedKey {
    pub key: String,
    pub since: u64,
    /// It held a live value: expiring counts as a change.
    pub existed: bool,
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
    /// Memory bound of the vector indexes in bytes; 0 = none of their own.
    pub max_vector_bytes: usize,
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
            max_vector_bytes: 0,
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
    /// Whether every write is on disk before it is acknowledged.
    pub sync_writes: bool,
    /// Writes that waited to be on disk, and the fsyncs that served them:
    /// with many writers at once, one fsync serves several writes.
    pub durable_writes: u64,
    pub disk_syncs: u64,
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
    /// The values of the fields of an index on several fields.
    Tuple(Vec<SKey>),
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

    fn to_json(&self) -> Json {
        match self {
            SKey::Null => Json::Null,
            SKey::Bool(b) => Json::Bool(*b),
            SKey::Num(n) => crate::query::num(n.0),
            SKey::Str(s) => Json::String(s.clone()),
            SKey::Tuple(parts) => Json::Array(parts.iter().map(SKey::to_json).collect()),
        }
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
            // `like` tests each distinct value: no order needed.
            IndexKind::Hash => matches!(op, CmpOp::Eq | CmpOp::In | CmpOp::Like | CmpOp::ILike),
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

    fn for_each(&self, mut f: impl FnMut(&SKey, &BTreeSet<String>)) {
        match self {
            Postings::Tree(m) => m.iter().for_each(|(k, v)| f(k, v)),
            Postings::Hash(m) => m.iter().for_each(|(k, v)| f(k, v)),
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
    /// The field, or the fields joined by `, ` for an index on several.
    field: String,
    paths: Vec<Vec<jsonpath::Seg>>,
    kind: IndexKind,
    /// Two documents of the pattern may not hold the same value.
    unique: bool,
    map: Postings,
    rev: HashMap<String, SKey>,
}

impl SecIndex {
    fn new(pattern: &str, field: &str, paths: Vec<Vec<jsonpath::Seg>>, kind: IndexKind, unique: bool) -> SecIndex {
        SecIndex {
            pattern: pattern.to_string(),
            field: field.to_string(),
            paths,
            kind,
            unique,
            map: Postings::new(kind),
            rev: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.map = Postings::new(self.kind);
        self.rev.clear();
    }

    /// The indexed value of `doc`. On several fields, a document lacking
    /// one of them, or holding an array or an object there, is not indexed.
    fn value(&self, doc: &Json) -> Option<SKey> {
        let one = |path: &Vec<jsonpath::Seg>| jsonpath::get(doc, path).and_then(SKey::from_json);
        match self.paths.as_slice() {
            [path] => one(path),
            paths => paths.iter().map(one).collect::<Option<Vec<SKey>>>().map(SKey::Tuple),
        }
    }

    /// The keys other than `key` that hold `sk`, when the index is unique.
    /// A null is a value like any other; a document lacking a field holds
    /// nothing.
    fn holders<'a>(&'a self, key: &'a str, sk: &SKey) -> impl Iterator<Item = &'a String> + 'a {
        let set = if self.unique { self.map.get(sk) } else { None };
        set.into_iter().flatten().filter(move |k| k.as_str() != key)
    }

    /// `user:* (email)`, as errors and plans name the index.
    fn describe(&self) -> String {
        format!("{} ({})", self.pattern, self.field)
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
        let sk = doc.and_then(|d| self.value(d));
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

/// Expired keys removed per pass of the maintenance thread.
const PURGE_BATCH: usize = 10_000;

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
    /// `vector.max_ram_mb` in bytes; 0 = no bound of their own.
    pub vector_limit: usize,
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
    /// The field, or the fields joined by `, ` for a unique index on several.
    pub field: String,
    pub kind: IndexKind,
    pub unique: bool,
    /// Documents that have the field.
    pub documents: usize,
    /// Distinct values of the field.
    pub values: usize,
}

/// Fields one unique index may cover.
pub const MAX_UNIQUE_FIELDS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    /// One of the values of a list: `city in ["Goma", "Bukavu"]`.
    In,
    /// A text pattern: `%` stands for any run of characters, `_` for one.
    Like,
    /// `like`, capitals and small letters being the same.
    ILike,
}

/// Values one `in` list may hold.
pub const MAX_IN_VALUES: usize = 1024;

impl CmpOp {
    /// The operators, as error messages list them.
    pub const LIST: &'static str = "= != > >= < <= in like ilike";

    pub fn parse(s: &str) -> Option<CmpOp> {
        Some(match s {
            "=" | "==" => CmpOp::Eq,
            "!=" | "<>" => CmpOp::Ne,
            ">" => CmpOp::Gt,
            ">=" => CmpOp::Ge,
            "<" => CmpOp::Lt,
            "<=" => CmpOp::Le,
            _ if s.eq_ignore_ascii_case("in") => CmpOp::In,
            _ if s.eq_ignore_ascii_case("like") => CmpOp::Like,
            _ if s.eq_ignore_ascii_case("ilike") => CmpOp::ILike,
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
            CmpOp::In => "in",
            CmpOp::Like => "like",
            CmpOp::ILike => "ilike",
        }
    }

    /// Whether the operator is a word (`in`, `like`), written between spaces.
    pub fn is_word(self) -> bool {
        matches!(self, CmpOp::In | CmpOp::Like | CmpOp::ILike)
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
    match (op, a, b) {
        (CmpOp::In, _, Json::Array(list)) => return list.iter().any(|v| compare_json(a, CmpOp::Eq, v)),
        (CmpOp::Like | CmpOp::ILike, Json::String(text), Json::String(pattern)) => {
            return like_match(text, pattern, op == CmpOp::ILike)
        }
        (CmpOp::In | CmpOp::Like | CmpOp::ILike, _, _) => return false,
        _ => {}
    }
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
        (Some(_), CmpOp::In | CmpOp::Like | CmpOp::ILike) => unreachable!("answered above"),
    }
}

/// One element of a `like` pattern.
enum LikePart {
    /// `%`: any run of characters, none included.
    Any,
    /// `_`: one character.
    One,
    Char(char),
}

fn like_parts(pattern: &str) -> Vec<LikePart> {
    let mut parts = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        parts.push(match c {
            '%' => LikePart::Any,
            '_' => LikePart::One,
            // `\%`, `\_` and `\\` stand for the character itself.
            '\\' => LikePart::Char(chars.next().unwrap_or('\\')),
            c => LikePart::Char(c),
        });
    }
    parts
}

/// Whether `text` matches the `like` pattern. With `fold`, capitals and
/// small letters are the same, in every alphabet (`É` matches `é`).
pub fn like_match(text: &str, pattern: &str, fold: bool) -> bool {
    let (text, pattern) =
        if fold { (text.to_lowercase(), pattern.to_lowercase()) } else { (text.into(), pattern.into()) };
    let t: Vec<char> = text.chars().collect();
    let p = like_parts(&pattern);
    // Greedy, going back to the last `%` on a mismatch: at most
    // len(text) * len(pattern) steps, usually about len(text).
    let (mut ti, mut pi) = (0, 0);
    let mut last_any: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some(LikePart::Any) => {
                last_any = Some((pi, ti));
                pi += 1;
            }
            Some(LikePart::One) => {
                ti += 1;
                pi += 1;
            }
            Some(LikePart::Char(c)) if *c == t[ti] => {
                ti += 1;
                pi += 1;
            }
            _ => match last_any {
                // The last `%` takes one more character.
                Some((ap, at)) => {
                    pi = ap + 1;
                    ti = at + 1;
                    last_any = Some((ap, at + 1));
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|x| matches!(x, LikePart::Any))
}

/// What every text matching the `like` pattern starts with: its characters
/// before the first wildcard.
pub fn like_prefix(pattern: &str) -> String {
    like_parts(pattern)
        .into_iter()
        .map_while(|x| match x {
            LikePart::Char(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// A graph search pre-filters through a secondary index only when the
/// candidates are at most this fraction of the graph (1/10).
const PREFILTER_RATIO: usize = 10;

/// A quantized graph is asked for this many times the results wanted, which
/// the documents' full vectors then put in their true order.
const RERANK: usize = 2;

fn as_doc(v: &Value) -> Option<&Json> {
    match v {
        Value::Doc(j) => Some(j),
        _ => None,
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
    /// A handle of the data file `file_id` for its fsyncs (group commit).
    sync_file: Option<(u64, Arc<File>)>,
    index: BTreeMap<String, Entry>,
    /// File header plus the records the index currently points to.
    live_bytes: u64,
    /// Keys with a TTL, by deadline: the expired ones are found without
    /// looking at the others.
    expiry: BTreeSet<(i64, String)>,
    /// Told the key of each document dropped because its TTL elapsed.
    on_expire: Vec<ExpiryListener>,
    seq: u64,
    compression: Compression,
    auto_compact: bool,
    threshold: u8,
    sec: Vec<SecIndex>,
    vecs: Vec<VecIndex>,
    cache: ValueCache,
    /// Sum of `key_cost` over the index, kept up to date on every change.
    key_bytes: usize,
    max_memory: usize,
    /// `vector.max_ram_mb` in bytes; 0 = no bound of their own.
    max_vectors: usize,
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

/// A write put off: its engine, and where it ended in the data file.
type PutOff = (Arc<Engine>, (u64, u64));

thread_local! {
    /// The durable writes of this thread whose disk sync is put off, while
    /// [`put_off_syncs`] runs.
    static PUT_OFF: RefCell<Option<Vec<PutOff>>> = const { RefCell::new(None) };
}

/// Runs `f`, putting off the disk syncs that its writes would wait for
/// under `storage.sync_writes`. The caller makes them with
/// [`PendingSyncs::wait`] before it answers.
///
/// For a caller that runs its commands one at a time under a lock of its
/// own (the Redis listener): waiting for the disk while holding that lock
/// would make every write wait for its own fsync, one after the other. With
/// the wait after the lock, the writes of all the clients share their
/// fsyncs (group commit), and each is still on disk before it is answered.
pub fn put_off_syncs<R>(f: impl FnOnce() -> R) -> (R, PendingSyncs) {
    // Inside another `put_off_syncs`, the outer one makes the syncs.
    if PUT_OFF.with(|p| p.borrow().is_some()) {
        return (f(), PendingSyncs(Vec::new()));
    }
    /// Stops putting syncs off, even when `f` panics.
    struct Collect;
    impl Drop for Collect {
        fn drop(&mut self) {
            PUT_OFF.with(|p| p.borrow_mut().take());
        }
    }
    PUT_OFF.with(|p| *p.borrow_mut() = Some(Vec::new()));
    let collect = Collect;
    let r = f();
    let syncs = PUT_OFF.with(|p| p.borrow_mut().take()).unwrap_or_default();
    drop(collect);
    (r, PendingSyncs(syncs))
}

/// Disk syncs put off by [`put_off_syncs`].
#[must_use = "the writes are not on disk until wait() returns"]
pub struct PendingSyncs(Vec<PutOff>);

impl PendingSyncs {
    /// Whether no write is waiting for the disk.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Waits until every write made in `put_off_syncs` is on disk.
    pub fn wait(self) -> Result<()> {
        // Within an engine, the last write ends after the others: syncing
        // up to it covers them.
        let mut last: Vec<PutOff> = Vec::new();
        for (engine, mark) in self.0 {
            match last.iter_mut().find(|(e, _)| Arc::ptr_eq(e, &engine)) {
                Some(seen) => seen.1 = mark,
                None => last.push((engine, mark)),
            }
        }
        for (engine, mark) in last {
            engine.sync_to(mark)?;
        }
        Ok(())
    }
}
type ExpiryListener = Box<dyn Fn(&str) + Send + Sync>;

pub struct Engine {
    /// The engine itself, for the disk syncs a thread puts off
    /// ([`put_off_syncs`]).
    me: Weak<Engine>,
    path: PathBuf,
    inner: Mutex<Inner>,
    listeners: RwLock<Vec<Listener>>,
    signal: Arc<Signal>,
    compacting: AtomicBool,
    opts: EngineOptions,
    /// `storage.sync_writes`: every write is on disk before it returns.
    sync_writes: AtomicBool,
    /// How far the data file is known to be on disk, and whether a writer
    /// is syncing it. Never held during an fsync, nor while taking `inner`.
    durable: Mutex<Durable>,
    /// Wakes the writers waiting for an fsync when it ends.
    synced: Condvar,
    /// Durable writes that have set off, and those that have appended their
    /// records: the difference is on its way to the data file.
    setting_off: AtomicU64,
    appended: AtomicU64,
    durable_writes: AtomicU64,
    disk_syncs: AtomicU64,
    /// An HNSW index is being tuned again in the background (`auto`).
    tuning: AtomicBool,
    /// Threads waiting for `inner` (see [`Engine::let_waiters_in`]).
    lock_waiters: AtomicU64,
    _lock: File,
}

/// How long an fsync waits at most for the writes on their way to join it.
const MAX_GATHER: Duration = Duration::from_millis(1);

/// A durable write between the moment it sets off and the moment its
/// records are appended (or it gave up).
struct Arriving<'a>(&'a Engine);

impl Drop for Arriving<'_> {
    fn drop(&mut self) {
        self.0.appended.fetch_add(1, Ordering::AcqRel);
    }
}

/// Fsyncs held off by [`Engine::hold_syncs`].
struct SyncsHeld<'a>(&'a Engine);

impl Drop for SyncsHeld<'_> {
    fn drop(&mut self) {
        self.0.durable.lock().unwrap_or_else(|e| e.into_inner()).syncing = false;
        self.0.synced.notify_all();
    }
}

/// The part of the data file `file_id` that an fsync has put on disk.
#[derive(Default)]
struct Durable {
    file_id: u64,
    len: u64,
    /// A writer is running an fsync for the others.
    syncing: bool,
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
        let mut index = std::mem::replace(&mut self.vecs[i], VecIndex::new("", "", Vec::new()));
        self.build_vec(&mut index, usize::MAX);
        self.vecs[i] = index;
    }

    /// Fills `index` from the documents on disk, stopping once it holds
    /// more than `budget` bytes. Returns whether every document was read.
    fn build_vec(&mut self, index: &mut VecIndex, budget: usize) -> bool {
        index.clear();
        let pattern = index.pattern.clone();
        let prefix = glob_prefix(&pattern).to_string();
        let now = now_ms();
        let keys: Vec<(String, Entry)> = self
            .index
            .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, e)| e.ty.is_doc() && !e.is_expired(now) && glob_match(&pattern, k))
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect();
        for (k, e) in keys {
            if index.memory_bytes() > budget {
                return false;
            }
            if let Ok(Value::Doc(doc)) = self.read_value(&k, &e) {
                index.update(&k, Some(&doc));
            }
        }
        true
    }

    /// Brings a graph read from disk up to date: it reflected the data up to
    /// byte `since` of the log, so only the keys written after that, and the
    /// keys deleted since, need looking at.
    fn catch_up_vec(&mut self, i: usize, since: u64) {
        let now = now_ms();
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
        let expires_at = e.expires_at;
        match self.index.insert(key.to_string(), e) {
            Some(old) => {
                self.live_bytes -= old.rec_len();
                if old.expires_at > 0 {
                    self.expiry.remove(&(old.expires_at, key.to_string()));
                }
            }
            None => self.key_bytes += key_cost(key),
        }
        if expires_at > 0 {
            self.expiry.insert((expires_at, key.to_string()));
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
            self.expiry.remove(&(old.expires_at, key.to_string()));
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
            self.expired(key);
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
        self.expiry =
            self.index.iter().filter(|(_, e)| e.expires_at > 0).map(|(k, e)| (e.expires_at, k.clone())).collect();
        self.key_bytes = self.index.keys().map(|k| key_cost(k)).sum();
    }

    /// Memory of the vector indexes; each one keeps its own count.
    fn vec_bytes(&self) -> usize {
        self.vecs.iter().map(|v| v.memory_bytes()).sum()
    }

    /// Memory the vector indexes would gain from writing `doc` under `key`.
    fn vector_growth(&self, key: &str, doc: Option<&Json>) -> usize {
        self.vecs.iter().map(|v| v.growth(key, doc)).sum()
    }

    /// Bytes the vector indexes may still take under both bounds.
    fn vector_room_left(&self) -> usize {
        let mut left = usize::MAX;
        if self.max_vectors > 0 {
            left = self.max_vectors.saturating_sub(self.vec_bytes());
        }
        if self.max_memory > 0 {
            left = left.min(self.max_memory.saturating_sub(self.memory().resident()));
        }
        left
    }

    /// Refuses writes that would give two documents the same value in a
    /// unique index. `writes` holds each key written with the document it
    /// will hold (`None`: deleted, or no longer a document); keys written
    /// together are judged by their new values, never their old ones.
    fn check_unique(&self, writes: &[(&str, Option<&Json>)]) -> Result<()> {
        let now = now_ms();
        for s in self.sec.iter().filter(|s| s.unique) {
            let mut taken: HashMap<SKey, &str> = HashMap::new();
            for &(key, doc) in writes {
                if !glob_match(&s.pattern, key) {
                    continue;
                }
                let Some(sk) = doc.and_then(|d| s.value(d)) else {
                    continue;
                };
                let other = taken.get(&sk).copied().or_else(|| {
                    s.holders(key, &sk)
                        .find(|k| {
                            !writes.iter().any(|(w, _)| w == k)
                                && self.index.get(k.as_str()).is_some_and(|e| !e.is_expired(now))
                        })
                        .map(String::as_str)
                });
                if let Some(other) = other {
                    return Err(KivaError::Exists(format!(
                        "'{key}' not written: '{other}' already holds {} in the unique index on {}",
                        sk.to_json(),
                        s.describe()
                    )));
                }
                taken.insert(sk, key);
            }
        }
        Ok(())
    }

    /// Refuses a write that would take the vector indexes past their bound,
    /// or past the workspace's memory bound. Vector indexes live in memory
    /// only, so this is what stops them from exhausting the machine's RAM.
    fn check_vector_room(&self, added: usize) -> Result<()> {
        if added == 0 {
            return Ok(());
        }
        let size = |n: usize| crate::util::human_size(n as u64);
        let used = self.vec_bytes();
        if self.max_vectors > 0 && used + added > self.max_vectors {
            return Err(KivaError::Other(format!(
                "OOM: the vector indexes of this workspace reached their memory bound ({} of {} used), \
                 so new vectors are refused. Delete documents, drop a vector index, or raise vector.max_ram_mb.",
                size(used),
                size(self.max_vectors)
            )));
        }
        if self.max_memory > 0 {
            let m = self.memory();
            if m.resident() + added > self.max_memory {
                return Err(KivaError::Other(format!(
                    "OOM: this workspace reached its memory bound ({} of {} used), so new vectors are refused. \
                     Delete keys or expired data, drop an index, or raise storage.max_memory_mb.",
                    size(m.resident()),
                    size(self.max_memory)
                )));
            }
        }
        Ok(())
    }

    fn memory(&self) -> MemoryReport {
        let n = self.index.len();
        let avg_key = self.key_bytes.checked_div(n).map_or(0, |b| b - key_cost(""));
        // Per indexed document: the key twice (reverse map and value set),
        // the value, and the nodes holding them.
        let per_doc = 2 * (avg_key + std::mem::size_of::<String>()) + std::mem::size_of::<SKey>() + 64;
        MemoryReport {
            // The deadlines of keys with a TTL are kept in a second tree.
            keys: self.key_bytes + self.expiry.len() * (avg_key + 2 * std::mem::size_of::<String>()),
            key_count: n,
            secondary: self.sec.iter().map(|s| s.rev.len() * per_doc).sum(),
            vectors: self.vec_bytes(),
            pinned: self.cache.pinned_bytes(),
            pinned_values: self.cache.pinned_len(),
            pinned_patterns: self.cache.patterns().to_vec(),
            cache: self.cache.bytes(),
            limit: self.max_memory,
            vector_limit: self.max_vectors,
        }
    }

    /// Reads into memory the values of the live keys matching `pattern`.
    fn warm_pinned(&mut self, pattern: &str) {
        let prefix = glob_prefix(pattern).to_string();
        let now = now_ms();
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

    /// Removes keys whose TTL has elapsed, earliest deadline first, at most
    /// `PURGE_BATCH` per call so that the lock is never held for long. Keys
    /// that have not expired are not looked at.
    fn purge_expired(&mut self, now: i64) -> usize {
        let dead: Vec<String> =
            self.expiry.iter().take_while(|(t, _)| *t <= now).take(PURGE_BATCH).map(|(_, k)| k.clone()).collect();
        for k in &dead {
            self.idx_remove(k);
            self.expired(k);
        }
        dead.len()
    }

    fn expired(&self, key: &str) {
        for l in &self.on_expire {
            l(key);
        }
    }

    fn purge_all_expired(&mut self, now: i64) {
        while self.purge_expired(now) == PURGE_BATCH {}
    }

    fn rebuild_sec(&mut self, i: usize) {
        self.sec[i].clear();
        let pattern = self.sec[i].pattern.clone();
        let prefix = glob_prefix(&pattern).to_string();
        let now = now_ms();
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

        let now = now_ms();
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
            sync_file: None,
            index,
            live_bytes: 0,
            expiry: BTreeSet::new(),
            on_expire: Vec::new(),
            seq,
            compression: opts.compression,
            auto_compact: opts.auto_compact,
            threshold: opts.compact_threshold,
            sec: Vec::new(),
            vecs: Vec::new(),
            cache: ValueCache::new(opts.cache_bytes),
            key_bytes: 0,
            max_memory: opts.max_memory_bytes,
            max_vectors: opts.max_vector_bytes,
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
                            inner.vecs[i].ef_search = def["ef_search"].as_u64().unwrap_or(0) as usize;
                            inner.vecs[i].auto_tune = AutoTune::from_json(&def["auto_tune"]);
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
                        // An index on several fields lists them in "fields".
                        let paths: std::result::Result<Vec<_>, String> = match def["fields"].as_array() {
                            Some(fields) => {
                                fields.iter().map(|f| jsonpath::parse_path(f.as_str().unwrap_or_default())).collect()
                            }
                            None => jsonpath::parse_path(f).map(|segs| vec![segs]),
                        };
                        if let Ok(paths) = paths {
                            let kind = def["kind"].as_str().and_then(IndexKind::parse).unwrap_or_default();
                            let unique = def["unique"].as_bool().unwrap_or(false);
                            inner.sec.push(SecIndex::new(p, f, paths, kind, unique));
                            let i = inner.sec.len() - 1;
                            inner.rebuild_sec(i);
                        }
                    }
                }
            }
        }
        inner.load.ms = started.elapsed().as_secs_f64() * 1000.0;

        let signal = Arc::new(Signal { wake: Mutex::new(false), cv: Condvar::new(), stop: AtomicBool::new(false) });
        let engine = Arc::new_cyclic(|me| Engine {
            me: me.clone(),
            path,
            inner: Mutex::new(inner),
            listeners: RwLock::new(Vec::new()),
            signal: signal.clone(),
            compacting: AtomicBool::new(false),
            opts: opts.clone(),
            sync_writes: AtomicBool::new(opts.sync_writes),
            durable: Mutex::new(Durable::default()),
            synced: Condvar::new(),
            setting_off: AtomicU64::new(0),
            appended: AtomicU64::new(0),
            durable_writes: AtomicU64::new(0),
            disk_syncs: AtomicU64::new(0),
            tuning: AtomicBool::new(false),
            lock_waiters: AtomicU64::new(0),
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
        match self.inner.try_lock() {
            Ok(inner) => inner,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.lock_waiters.fetch_add(1, Ordering::AcqRel);
                let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                self.lock_waiters.fetch_sub(1, Ordering::AcqRel);
                inner
            }
        }
    }

    /// For a long task that takes the lock again and again (tuning): lets
    /// the threads waiting for it go first. The lock is not fair, and the
    /// thread that just released it would otherwise often take it back
    /// before them, for seconds.
    fn let_waiters_in(&self) {
        let until = Instant::now() + Duration::from_millis(20);
        while self.lock_waiters.load(Ordering::Acquire) > 0 && Instant::now() < until {
            std::thread::yield_now();
        }
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

    /// Registers a callback told the key of every document dropped because
    /// its TTL elapsed (no `Op` is emitted for those: replicas and the log
    /// expire them by themselves). Same rules as [`Engine::add_listener`].
    pub fn add_expiry_listener(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        self.lock().on_expire.push(Box::new(f));
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
        self.retune_if_due(inner);
    }

    /// Where the last write ended, when it must reach the disk before the
    /// caller is answered. Taken while still holding the lock.
    fn durable_mark(&self, inner: &Inner) -> Option<(u64, u64)> {
        self.sync_writes.load(Ordering::Relaxed).then_some((inner.file_id, inner.file_len))
    }

    /// Group commit: waits until the data file is on disk up to `mark`.
    ///
    /// One writer at a time runs the fsync, outside the engine lock, so the
    /// others keep appending meanwhile. They wait for it to end: the ones it
    /// covered return all together, and one of the others runs the next
    /// fsync, for everything written so far at once. With many clients
    /// writing, one fsync serves many writes, and each write is still on
    /// disk before it is acknowledged.
    fn make_durable(&self, mark: Option<(u64, u64)>, arriving: Option<Arriving>) -> Result<()> {
        drop(arriving);
        let Some(mark) = mark else { return Ok(()) };
        self.durable_writes.fetch_add(1, Ordering::Relaxed);
        if let Some(me) = self.me.upgrade() {
            if PUT_OFF.with(|p| p.borrow_mut().as_mut().map(|syncs| syncs.push((me, mark))).is_some()) {
                return Ok(());
            }
        }
        self.sync_to(mark)
    }

    fn sync_to(&self, (file_id, end): (u64, u64)) -> Result<()> {
        let covered = |d: &Durable| d.file_id == file_id && d.len >= end;
        let mut d = self.durable.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if covered(&d) {
                return Ok(());
            }
            if !d.syncing {
                break;
            }
            d = self.synced.wait(d).unwrap_or_else(|e| e.into_inner());
        }
        d.syncing = true;
        drop(d);
        // Whatever happens, the next writer can take over.
        let result = self.run_sync(file_id);
        let mut d = self.durable.lock().unwrap_or_else(|e| e.into_inner());
        d.syncing = false;
        if let Ok(Some(len)) = result {
            if d.file_id != file_id || d.len < len {
                (d.file_id, d.len) = (file_id, len);
            }
        }
        drop(d);
        self.synced.notify_all();
        result.map(|_| ())
    }

    /// Syncs the data file `file_id` as far as it is written; returns that
    /// length, or `None` when the file was replaced meanwhile.
    fn run_sync(&self, file_id: u64) -> Result<Option<u64>> {
        // The writes that set off before this fsync (often held up by the
        // last one) finish appending first, so that this fsync covers them
        // too. Not for long: a write that is slow to come waits for the next.
        let target = self.setting_off.load(Ordering::Acquire);
        let gather = Instant::now();
        std::thread::yield_now();
        while self.appended.load(Ordering::Acquire) < target && gather.elapsed() < MAX_GATHER {
            std::thread::yield_now();
        }
        let (file, len) = {
            let mut inner = self.lock();
            // A compaction copied the record into a new file and synced it, or
            // a reset wiped it: nothing left to do.
            if inner.file_id != file_id {
                return Ok(None);
            }
            // The fsync goes through a handle of its own. Through the handle
            // that writes (or a clone of it), Windows holds the writes back
            // until the fsync ends, and with them the Redis clients, which
            // write one at a time: none could join the next fsync.
            let file = match &inner.sync_file {
                Some((id, f)) if *id == file_id => f.clone(),
                _ => {
                    let f = Arc::new(OpenOptions::new().write(true).open(&self.path)?);
                    inner.sync_file = Some((file_id, f.clone()));
                    f
                }
            };
            (file, inner.file_len)
        };
        file.sync_data()?;
        self.disk_syncs.fetch_add(1, Ordering::Relaxed);
        Ok(Some(len))
    }

    /// Waits for a running fsync to end and holds off the next ones, until
    /// the guard is dropped.
    fn hold_syncs(&self) -> SyncsHeld<'_> {
        let mut d = self.durable.lock().unwrap_or_else(|e| e.into_inner());
        while d.syncing {
            d = self.synced.wait(d).unwrap_or_else(|e| e.into_inner());
        }
        d.syncing = true;
        SyncsHeld(self)
    }

    /// A write sets off: with `storage.sync_writes`, the next fsync waits
    /// for it to append its records, until it is dropped.
    fn arriving(&self) -> Option<Arriving<'_>> {
        self.sync_writes.load(Ordering::Relaxed).then(|| {
            self.setting_off.fetch_add(1, Ordering::AcqRel);
            Arriving(self)
        })
    }

    /// Turns `storage.sync_writes` on or off; it applies to the next write.
    pub fn set_sync_writes(&self, on: bool) {
        self.sync_writes.store(on, Ordering::Relaxed);
    }

    fn wake(&self) {
        *self.signal.wake.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.signal.cv.notify_all();
    }

    // -- reads --------------------------------------------------------------

    pub fn get(&self, key: &str) -> Result<Option<(Value, Entry)>> {
        let mut inner = self.lock();
        let Some(e) = inner.live(key, now_ms()) else {
            return Ok(None);
        };
        let v = inner.value_of(key, &e)?;
        Ok(Some((v, e)))
    }

    /// Canonical payload bytes of `key` (used by replication).
    pub fn get_raw(&self, key: &str) -> Result<Option<(Vec<u8>, Entry)>> {
        let mut inner = self.lock();
        let Some(e) = inner.live(key, now_ms()) else {
            return Ok(None);
        };
        let raw = inner.read_raw(key, &e)?;
        Ok(Some((raw, e)))
    }

    pub fn meta(&self, key: &str) -> Option<Entry> {
        self.lock().live(key, now_ms())
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
        let now = now_ms();
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
        let now = now_ms();
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
        self.put_at(key, value, expires_at, msgpack, now_secs(), true)
    }

    /// `check`: refuse a vector past the memory bounds (never for what a
    /// master sent: the replica must stay a copy).
    fn put_at(
        &self,
        key: &str,
        value: &Value,
        expires_at: i64,
        msgpack: bool,
        timestamp: i64,
        check: bool,
    ) -> Result<bool> {
        let arriving = self.arriving();
        let mut inner = self.lock();
        if check {
            inner.check_unique(&[(key, as_doc(value))])?;
            inner.check_vector_room(inner.vector_growth(key, as_doc(value)))?;
        }
        let created = self.put_locked(&mut inner, key, value, expires_at, msgpack, timestamp)?;
        self.after_write(&inner);
        let mark = self.durable_mark(&inner);
        drop(inner);
        self.make_durable(mark, arriving)?;
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
        let arriving = self.arriving();
        let mut inner = self.lock();
        let Some(e) = inner.live(key, now_ms()) else {
            return Ok(None);
        };
        let current = inner.value_of(key, &e)?;
        let (new, out) = f(current)?;
        let Some(new) = new else {
            return Ok(Some((out, false)));
        };
        inner.check_unique(&[(key, as_doc(&new))])?;
        inner.check_vector_room(inner.vector_growth(key, as_doc(&new)))?;
        self.put_locked(&mut inner, key, &new, e.expires_at, e.is_msgpack(), now_secs())?;
        self.after_write(&inner);
        let mark = self.durable_mark(&inner);
        drop(inner);
        self.make_durable(mark, arriving)?;
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
        let created = inner.live(key, now_ms()).is_none();
        entry.offset = inner.append(&rec, false)?;
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
        self.put_at(key, &value, expires_at, msgpack, timestamp, false).map(|_| ())
    }

    pub fn delete(&self, key: &str) -> Result<bool> {
        let arriving = self.arriving();
        let mut inner = self.lock();
        if inner.live(key, now_ms()).is_none() {
            return Ok(false);
        }
        let rec = inner.encode(key, &[], 0, FLAG_TOMBSTONE, now_secs(), 0);
        inner.append(&rec, false)?;
        inner.seq += 1;
        inner.idx_remove(key);
        if inner.active_txns > 0 {
            let seq = inner.seq;
            inner.tomb_versions.insert(key.to_string(), seq);
        }
        inner.dirty_ops += 1;
        self.emit(&Op::Del { key: key.to_string() });
        self.after_write(&inner);
        let mark = self.durable_mark(&inner);
        drop(inner);
        self.make_durable(mark, arriving)?;
        Ok(true)
    }

    /// Applies several operations atomically: either every record of the
    /// batch is on disk and applied, or (after a crash) none is.
    ///
    /// With `begin_seq`, the batch is rejected if any of its keys was
    /// modified by someone else after that sequence number.
    pub fn apply_batch(&self, ops: Vec<BatchOp>, begin_seq: Option<u64>) -> Result<()> {
        self.apply_batch_watched(ops, begin_seq, None)
    }

    /// Starts watching `keys` (Redis `WATCH`): a later
    /// [`Engine::apply_batch_watched`] given the result fails when one of
    /// them was written, deleted or expired meanwhile. Until
    /// [`Engine::unwatch`], deletions are remembered as for a transaction.
    pub fn watch(&self, keys: &[String]) -> Vec<WatchedKey> {
        let mut inner = self.lock();
        inner.active_txns += 1;
        let now = now_ms();
        let since = inner.seq;
        keys.iter().map(|k| WatchedKey { key: k.clone(), since, existed: inner.live(k, now).is_some() }).collect()
    }

    /// Ends one [`Engine::watch`].
    pub fn unwatch(&self) {
        self.txn_end();
    }

    /// The first watched key that changed since it was watched, if any.
    pub fn watch_conflict(&self, watched: &[WatchedKey]) -> Option<String> {
        let mut inner = self.lock();
        Self::changed(&mut inner, watched, now_ms())
    }

    fn changed(inner: &mut Inner, watched: &[WatchedKey], now: i64) -> Option<String> {
        watched
            .iter()
            .find(|w| {
                let ver = inner
                    .index
                    .get(&w.key)
                    .map(|e| e.version)
                    .or_else(|| inner.tomb_versions.get(&w.key).copied())
                    .unwrap_or(0);
                // Written, deleted, emptied with the database, or expired.
                ver > w.since || inner.reset_seq > w.since || (w.existed && inner.live(&w.key, now).is_none())
            })
            .map(|w| w.key.clone())
    }

    /// [`Engine::apply_batch`], refused as a conflict when one of the
    /// `watched` keys changed: checked under the same lock as the writes.
    pub fn apply_batch_watched(
        &self,
        ops: Vec<BatchOp>,
        begin_seq: Option<u64>,
        watched: Option<&[WatchedKey]>,
    ) -> Result<()> {
        let arriving = self.arriving();
        let mut inner = self.lock();
        let now = now_ms();
        let ts = now / 1000;
        if let Some(key) = watched.and_then(|w| Self::changed(&mut inner, w, now)) {
            return Err(KivaError::Conflict(format!("the watched key '{key}' changed")));
        }
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

        if inner.sec.iter().any(|s| s.unique) {
            // What each key holds once the whole batch is applied.
            let mut last: Vec<(&str, Option<&Json>)> = Vec::new();
            let mut slot: HashMap<&str, usize> = HashMap::new();
            for op in &ops {
                let (key, doc) = match op {
                    BatchOp::Put { key, value, .. } => (key.as_str(), as_doc(value)),
                    BatchOp::Del { key } => (key.as_str(), None),
                };
                match slot.get(key) {
                    Some(&i) => last[i].1 = doc,
                    None => {
                        slot.insert(key, last.len());
                        last.push((key, doc));
                    }
                }
            }
            inner.check_unique(&last)?;
        }

        enum Staged {
            Put { key: String, entry: Entry, raw: Vec<u8>, doc: Option<Json>, ty: Type, created: bool, rec_off: usize },
            Del { key: String },
        }
        let mut staged = Vec::with_capacity(ops.len());
        let mut recs: Vec<Vec<u8>> = Vec::with_capacity(ops.len());
        // Later operations in the batch must see earlier ones.
        let mut shadow: HashMap<String, bool> = HashMap::new();
        let mut vector_growth = 0;
        for op in ops {
            match op {
                BatchOp::Put { key, value, expires_at, msgpack } => {
                    let (rec, entry, raw) = build_put(&inner, &key, &value, expires_at, msgpack, ts, FLAG_TXN)?;
                    let exists = match shadow.get(&key) {
                        Some(e) => *e,
                        None => inner.live(&key, now).is_some(),
                    };
                    shadow.insert(key.clone(), true);
                    vector_growth += inner.vector_growth(&key, as_doc(&value));
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
        inner.check_vector_room(vector_growth)?;
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
        let base = inner.append(&buf, false)?;

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
        let mark = self.durable_mark(&inner);
        drop(inner);
        self.make_durable(mark, arriving)
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
            inner.purge_all_expired(now_ms());
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
        // No fsync may hold a handle on the old file while it is replaced.
        let _held = self.hold_syncs();
        let mut inner = self.lock();
        inner.sync_file = None;
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
        inner.purge_all_expired(now_ms());
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
            ttl_keys: inner.expiry.len(),
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
            sync_writes: self.sync_writes.load(Ordering::Relaxed),
            durable_writes: self.durable_writes.load(Ordering::Relaxed),
            disk_syncs: self.disk_syncs.load(Ordering::Relaxed),
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
        inner.purge_all_expired(now_ms());
        inner.write_hint(&self.path);
        if inner.graph_changes() > 0 {
            inner.save_graphs(&self.path);
        }
        inner.sync_file = None;
        inner.closed = true;
    }

    fn tick(&self) {
        let compact_due = {
            let mut inner = self.lock();
            if inner.closed {
                return;
            }
            inner.ticks += 1;
            // Every second: cheap, since only expired keys are visited. A
            // full batch means more are waiting: come back at once.
            if inner.purge_expired(now_ms()) == PURGE_BATCH {
                self.wake();
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
                if s.unique {
                    def["unique"] = true.into();
                }
                if s.paths.len() > 1 {
                    def["fields"] = s.field.split(", ").collect::<Vec<_>>().into();
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
                    if p.quant != hnsw::Quant::None {
                        def["quantize"] = p.quant.name().into();
                    }
                    if v.ef_search > 0 {
                        def["ef_search"] = v.ef_search.into();
                    }
                    if let Some(a) = &v.auto_tune {
                        def["auto_tune"] = a.to_json();
                    }
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
        self.create_index_on(pattern, &[field], kind, false)
    }

    /// Creates an index on one field, or a unique index on one or several
    /// fields (`(type, parent, name)`: no two documents of the pattern hold
    /// the same three values). A unique index is refused while two
    /// documents hold the same value. Returns the number of documents
    /// indexed.
    pub fn create_index_on(&self, pattern: &str, fields: &[&str], kind: IndexKind, unique: bool) -> Result<usize> {
        if fields.is_empty() || fields.iter().any(|f| f.trim().is_empty()) {
            return Err(KivaError::Invalid("an index needs a field name".into()));
        }
        if fields.len() > 1 && !unique {
            return Err(KivaError::Invalid(
                "an index on several fields must be unique (create unique index ...): queries use \
                 indexes on one field, and combine them for 'and'"
                    .into(),
            ));
        }
        if fields.len() > MAX_UNIQUE_FIELDS {
            return Err(KivaError::Invalid(format!("a unique index has at most {MAX_UNIQUE_FIELDS} fields")));
        }
        let paths =
            fields.iter().map(|f| jsonpath::parse_path(f).map_err(KivaError::Invalid)).collect::<Result<Vec<_>>>()?;
        let field = fields.join(", ");
        let mut inner = self.lock();
        if let Some(s) = inner.sec.iter().find(|s| s.pattern == pattern && s.field == field) {
            return Err(KivaError::Exists(format!(
                "index on {pattern} ({field}) already exists ({}{}); drop it first to change it",
                if s.unique { "unique, " } else { "" },
                s.kind.name()
            )));
        }
        inner.sec.push(SecIndex::new(pattern, &field, paths, kind, unique));
        let i = inner.sec.len() - 1;
        inner.rebuild_sec(i);
        if unique {
            let s = &inner.sec[i];
            let mut twice = None;
            s.map.for_each(|sk, keys| {
                if twice.is_none() && keys.len() > 1 {
                    let mut k = keys.iter();
                    twice = Some((k.next().cloned(), k.next().cloned(), sk.to_json()));
                }
            });
            if let Some((a, b, value)) = twice {
                let what = s.describe();
                inner.sec.pop();
                return Err(KivaError::Exists(format!(
                    "cannot create the unique index on {what}: {} and {} both hold {value}. \
                     Change one of them, then create the index again.",
                    a.unwrap_or_default(),
                    b.unwrap_or_default()
                )));
            }
        }
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
                unique: s.unique,
                documents: s.rev.len(),
                values: s.map.distinct(),
            })
            .collect()
    }

    // -- memory -------------------------------------------------------------

    /// Where this workspace's memory goes.
    pub fn memory(&self) -> MemoryReport {
        self.lock().memory()
    }

    /// Sets the memory bound in bytes; 0 removes it.
    pub fn set_memory_limit(&self, bytes: usize) {
        self.lock().max_memory = bytes;
    }

    /// Sets the memory bound of the vector indexes in bytes; 0 removes it.
    pub fn set_vector_limit(&self, bytes: usize) {
        self.lock().max_vectors = bytes;
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
        let now = now_ms();
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
        // Built, then measured: the size of a vector is known once it is read.
        // The new index is not counted yet, so it is checked against the rest.
        // The build stops as soon as it passes the room left, so that an
        // index too large for the bounds never fills the memory first.
        let mut index = VecIndex::with_kind(pattern, field, path, kind);
        let room = inner.vector_room_left();
        let complete = inner.build_vec(&mut index, room);
        let (n, built) = (index.len(), index.memory_bytes());
        if let Err(e) = inner.check_vector_room(built) {
            let e = e.to_string();
            let what = if complete { "would hold" } else { "stopped building after" };
            return Err(KivaError::Other(format!(
                "the vector index on {pattern} ({field}) {what} {n} vectors in {}. {}",
                crate::util::human_size(built as u64),
                e.strip_prefix("Error: ").unwrap_or(&e)
            )));
        }
        inner.vecs.push(index);
        self.save_meta(&inner)?;
        if kind != VectorIndexKind::Exact {
            // Building the graph took a while: keep it for the next start.
            inner.save_graphs(&self.path);
        }
        Ok(n)
    }

    /// Sets the `ef_search` of an HNSW index; 0 goes back to the default.
    /// A value chosen this way is kept: the index is no longer tuned again
    /// as it grows (`auto`).
    pub fn set_vector_ef_search(&self, pattern: &str, field: &str, ef: usize) -> Result<()> {
        let mut inner = self.lock();
        let i = Self::graph_index(&inner, pattern, field)?;
        inner.vecs[i].ef_search = ef;
        inner.vecs[i].auto_tune = None;
        self.save_meta(&inner)
    }

    fn graph_index(inner: &Inner, pattern: &str, field: &str) -> Result<usize> {
        let i = inner
            .vecs
            .iter()
            .position(|v| v.pattern == pattern && v.field == field)
            .ok_or_else(|| KivaError::NotFound(format!("no vector index on {pattern} ({field})")))?;
        if inner.vecs[i].graph().is_none() {
            return Err(KivaError::Invalid(format!(
                "the vector index on {pattern} ({field}) is exact: every search compares every vector and \
                 always finds the true closest ones, so there is nothing to tune. An HNSW index \
                 ('create vector index ... using hnsw') trades a little accuracy for speed."
            )));
        }
        Ok(i)
    }

    /// Measures how many of the true `k` closest vectors the graph finds for
    /// each `ef` of [`TUNE_LADDER`], using up to `sample` of the indexed
    /// vectors as queries, and keeps the smallest `ef` that reaches
    /// `recall` (the largest tried when none does) as the index's
    /// `ef_search`. Each query is also answered exactly, comparing every
    /// vector; the engine is locked for one search at a time, so the
    /// workspace keeps working meanwhile.
    pub fn tune_vector_index(
        &self,
        pattern: &str,
        field: &str,
        recall: f64,
        k: usize,
        sample: usize,
        queries_from: Option<&str>,
    ) -> Result<TuneReport> {
        let sample = sample.max(1);
        let (queries, from_data, vectors) = {
            let mut inner = self.lock();
            let i = Self::tunable(&inner, pattern, field)?;
            // Real queries, when the application keeps some: documents
            // holding a vector in the same field.
            let mut given: Vec<Vec<f32>> = Vec::new();
            if let Some(qp) = queries_from {
                let path = jsonpath::parse_path(field).map_err(KivaError::Invalid)?;
                let prefix = glob_prefix(qp).to_string();
                let now = now_ms();
                let keys: Vec<(String, Entry)> = inner
                    .index
                    .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
                    .take_while(|(k, _)| k.starts_with(&prefix))
                    .filter(|(k, e)| e.ty.is_doc() && !e.is_expired(now) && glob_match(qp, k))
                    .map(|(k, e)| (k.clone(), e.clone()))
                    .collect();
                let dims = inner.vecs[i].dims;
                let step = (keys.len() / sample).max(1);
                for (k, e) in keys.into_iter().step_by(step).take(sample) {
                    if let Value::Doc(doc) = inner.value_of(&k, &e)? {
                        given.extend(vector::extract(&doc, &path).filter(|v| Some(v.len()) == dims));
                    }
                }
                if given.is_empty() {
                    return Err(KivaError::Invalid(format!(
                        "no document matching {qp} holds a vector in '{field}' with the index's number of dimensions"
                    )));
                }
            }
            let g = inner.vecs[i].graph().expect("graph index");
            let vectors = g.len();
            if vectors <= k {
                return Err(KivaError::Invalid(format!(
                    "tuning needs more than {k} vectors in the index (it has {vectors}): with so few, every search finds them all"
                )));
            }
            let metric = g.params.metric;
            let from_data = given.is_empty();
            // Without real queries, indexed vectors stand for them, each
            // searched as if its own node were not in the graph
            // (leave-one-out): landing on that node would lead straight to
            // its neighbours and make the graph look better than it is for
            // a new query.
            let queries: Vec<(Option<String>, vector::Query)> = if from_data {
                let step = (vectors / sample).max(1);
                g.iter()
                    .step_by(step)
                    .take(sample)
                    .map(|(key, v, _)| (Some(key.to_string()), vector::Query::new(v.to_vec(), metric)))
                    .collect()
            } else {
                given.into_iter().map(|v| (None, vector::Query::new(v, metric))).collect()
            };
            (queries, from_data, vectors)
        };
        // Only the time spent searching is counted, not the waits for the lock.
        let mut exact = Duration::ZERO;
        let mut truth: Vec<Vec<String>> = Vec::with_capacity(queries.len());
        for (own, q) in &queries {
            self.let_waiters_in();
            let inner = self.lock();
            let index = &inner.vecs[Self::tunable(&inner, pattern, field)?];
            let started = Instant::now();
            let scored = index.scores(&index.pattern, q, |key| Some(key) != own.as_deref());
            truth.push(vector::top_k(scored, k, q).into_iter().map(|(key, _)| key).collect());
            exact += started.elapsed();
        }
        let exact_micros = exact.as_secs_f64() * 1e6 / queries.len().max(1) as f64;
        let wanted: usize = truth.iter().map(|t| t.len()).sum();
        let mut steps = Vec::new();
        for &ef in TUNE_LADDER.iter().filter(|&&ef| ef >= k) {
            let (mut hits, mut compared, mut spent) = (0, 0, Duration::ZERO);
            for ((own, q), truth) in queries.iter().zip(&truth) {
                self.let_waiters_in();
                let inner = self.lock();
                let g = inner.vecs[Self::tunable(&inner, pattern, field)?].graph().expect("graph index");
                let started = Instant::now();
                let found = g.search_hiding(&q.vector, k, ef, |_| true, own.as_deref());
                spent += started.elapsed();
                compared += found.compared;
                hits += found.keys.iter().filter(|k2| truth.contains(k2)).count();
            }
            let n = queries.len();
            let step = TuneStep {
                ef,
                recall: hits as f64 / wanted.max(1) as f64,
                compared: compared / n,
                micros: spent.as_secs_f64() * 1e6 / n as f64,
            };
            let done = step.recall >= recall || ef >= vectors;
            steps.push(step);
            if done {
                break;
            }
        }
        let last = steps.last().expect("one step at least");
        let (chosen, reached) = (last.ef, last.recall >= recall);
        let mut inner = self.lock();
        let i = Self::tunable(&inner, pattern, field)?;
        inner.vecs[i].ef_search = chosen;
        if let Some(auto) = inner.vecs[i].auto_tune.as_mut() {
            auto.tuned_at = vectors;
        }
        self.save_meta(&inner)?;
        Ok(TuneReport { steps, chosen, reached, queries: truth.len(), from_data, vectors, exact_micros })
    }

    /// The HNSW index on `pattern (field)`, while the engine is open.
    fn tunable(inner: &Inner, pattern: &str, field: &str) -> Result<usize> {
        if inner.closed {
            return Err(KivaError::Other("database is closed".into()));
        }
        Self::graph_index(inner, pattern, field)
    }

    /// Keeps an HNSW index tuned as it grows or shrinks (`auto`), or stops
    /// (`None`). With \`auto\`, the index is tuned again in the background
    /// each time its number of vectors doubles or halves since the last
    /// tuning.
    pub fn set_vector_auto_tune(&self, pattern: &str, field: &str, auto: Option<AutoTune>) -> Result<()> {
        let mut inner = self.lock();
        let i = Self::graph_index(&inner, pattern, field)?;
        inner.vecs[i].auto_tune = auto;
        self.save_meta(&inner)
    }

    /// Starts tuning in the background the first index kept tuned whose
    /// size doubled or halved since its last tuning; one at a time.
    fn retune_if_due(&self, inner: &Inner) {
        let Some(v) = inner.vecs.iter().find(|v| v.auto_tune.as_ref().is_some_and(|a| a.due(v.len()))) else {
            return;
        };
        if self.tuning.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(me) = self.me.upgrade() else {
            self.tuning.store(false, Ordering::Release);
            return;
        };
        let (pattern, field) = (v.pattern.clone(), v.field.clone());
        let auto = v.auto_tune.clone().expect("due");
        let started = std::thread::Builder::new().name("kivadb-tune".into()).spawn(move || {
            let tuned =
                me.tune_vector_index(&pattern, &field, auto.recall, auto.limit, auto.sample, auto.queries.as_deref());
            if tuned.is_err() {
                // Too few vectors left, or no query document: tried again
                // when the size doubles or halves from here.
                let mut inner = me.lock();
                if let Some(v) = inner.vecs.iter_mut().find(|v| v.pattern == pattern && v.field == field) {
                    let len = v.len();
                    if let Some(a) = v.auto_tune.as_mut() {
                        a.tuned_at = len.max(1);
                    }
                }
            }
            me.tuning.store(false, Ordering::Release);
        });
        if started.is_err() {
            self.tuning.store(false, Ordering::Release);
        }
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
                not_vectors: v.not_vectors,
                tombstones: v.graph().map_or(0, |g| g.tombstones()),
                memory_bytes: v.memory_bytes(),
                ef_search: if v.ef_search == 0 { hnsw::DEFAULT_EF_SEARCH } else { v.ef_search },
                auto_tune: v.auto_tune.clone(),
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
        let mut filter = Vec::with_capacity(q.filter.len());
        for c in &q.filter {
            filter.push((jsonpath::parse_path(&c.field).map_err(KivaError::Invalid)?, c));
        }
        let passes = |doc: &Json| {
            filter.iter().all(|(p, c)| jsonpath::get(doc, p).is_some_and(|v| compare_json(v, c.op, &c.value)))
        };
        let query = vector::Query::new(q.vector.clone(), q.metric);
        let mut inner = self.lock();
        let now = now_ms();
        let limit = if q.limit == 0 { 10 } else { q.limit };

        let index = inner.vecs.iter().position(|v| v.field == q.field && pattern_covers(&v.pattern, &q.pattern));
        if let Some(d) = index.and_then(|i| inner.vecs[i].dims) {
            if d != query.dims() {
                return Err(KivaError::Invalid(format!(
                    "the query vector has {} dimensions, the indexed vectors have {d} dimensions",
                    query.dims()
                )));
            }
        }
        // The graph answers searches over its whole pattern, with the metric
        // it was built for. Anything else compares every vector.
        let graph = index.filter(|&i| {
            let v = &inner.vecs[i];
            v.graph().is_some_and(|g| !q.exact && g.params.metric == q.metric && q.pattern == v.pattern)
        });
        // A quantized graph only approximates its vectors: outside a graph
        // search, the documents' own vectors are compared.
        let index = index.filter(|&i| graph == Some(i) || !inner.vecs[i].quantized());
        // The conditions a secondary index answers, with the keys it gives.
        let mut indexed: Vec<(usize, Vec<String>)> = q
            .filter
            .iter()
            .filter_map(|c| Self::find_index(&inner, c).map(|i| (i, Self::index_candidates(&inner.sec[i], c))))
            .collect();
        // Pre-filtering: the condition a secondary index narrows the most.
        // Its candidates are compared one by one, which is exact; with a
        // graph, only when they are few enough to beat the graph search.
        let narrowest = (0..indexed.len()).min_by_key(|&j| indexed[j].1.len());
        let prefilter = narrowest
            .filter(|&j| match graph {
                Some(i) => indexed[j].1.len().saturating_mul(PREFILTER_RATIO) <= inner.vecs[i].len(),
                None => true,
            })
            .map(|j| indexed.swap_remove(j));

        // Documents read to test the filter, kept for the answer.
        let mut docs: HashMap<String, Json> = HashMap::new();
        let mut read = 0usize;
        let (chosen, examined, compared, approximate, how) = if let Some((s, candidates)) = prefilter {
            let examined = candidates.len();
            let mut scored = Vec::new();
            for key in candidates {
                if !glob_match(&q.pattern, &key) {
                    continue;
                }
                let Some(e) = inner.live(&key, now) else { continue };
                let Value::Doc(doc) = inner.value_of(&key, &e)? else { continue };
                read += 1;
                if !passes(&doc) {
                    continue;
                }
                let found = match index {
                    Some(i) if !inner.vecs[i].quantized() => inner.vecs[i].get(&key).map(|(v, n)| query.score(&v, n)),
                    _ => vector::extract(&doc, &path).filter(|v| v.len() == query.dims()).map(|v| query.score_vec(&v)),
                };
                if let Some(score) = found {
                    scored.push((key.clone(), score));
                    docs.insert(key, doc);
                }
            }
            let ix = &inner.sec[s];
            let how = format!("pre-filtered by the index on {} ({}), {read} documents read", ix.pattern, ix.field);
            let n = scored.len();
            (vector::top_k(scored, limit, &query), examined, n, false, Some(how))
        } else if let Some(i) = graph {
            let ef = match (q.ef, inner.vecs[i].ef_search) {
                (0, 0) => hnsw::DEFAULT_EF_SEARCH,
                (0, tuned) => tuned,
                (asked, _) => asked,
            };
            // A quantized graph orders its vectors nearly as the true ones:
            // it is asked for more, re-ranked with the documents' vectors.
            let quantized = inner.vecs[i].quantized();
            let want = if quantized { limit.saturating_mul(RERANK) } else { limit };
            let found = if filter.is_empty() {
                let live = |k: &str| inner.index.get(k).is_some_and(|e| !e.is_expired(now));
                let g = inner.vecs[i].graph().expect("graph index");
                g.search(&query.vector, want, ef.max(want), live)
            } else {
                // The filter is tested while the graph is walked. The keys
                // the secondary indexes give make a mask tested without
                // reading anything; the other conditions read the document,
                // once per node close enough to count.
                let mut mask: Option<HashSet<&str>> = None;
                for (_, keys) in &indexed {
                    let set: HashSet<&str> = keys.iter().map(String::as_str).collect();
                    mask = Some(match mask {
                        None => set,
                        Some(m) => m.intersection(&set).copied().collect(),
                    });
                }
                let rest: Vec<&(Vec<jsonpath::Seg>, &Cond)> =
                    filter.iter().filter(|(_, c)| Self::find_index(&inner, c).is_none()).collect();
                // The index is set aside while the documents are read, and
                // put back whatever happens.
                let vi = std::mem::replace(&mut inner.vecs[i], VecIndex::new("", "", Vec::new()));
                let state = std::cell::RefCell::new((&mut *inner, &mut docs, 0usize, None::<KivaError>));
                let accept = |key: &str| {
                    let mut s = state.borrow_mut();
                    let (inner, docs, read, err) = &mut *s;
                    if mask.as_ref().is_some_and(|m| !m.contains(key)) {
                        return false;
                    }
                    let Some(e) = inner.live(key, now) else { return false };
                    if rest.is_empty() {
                        return true;
                    }
                    *read += 1;
                    match inner.value_of(key, &e) {
                        Ok(Value::Doc(d)) => {
                            let ok = rest
                                .iter()
                                .all(|(p, c)| jsonpath::get(&d, p).is_some_and(|v| compare_json(v, c.op, &c.value)));
                            if ok {
                                docs.insert(key.to_string(), d);
                            }
                            ok
                        }
                        Ok(_) => false,
                        Err(x) => {
                            err.get_or_insert(x);
                            false
                        }
                    }
                };
                let found = vi.graph().expect("graph index").search_filtered(&query.vector, want, ef.max(want), accept);
                let (_, _, n, err) = state.into_inner();
                inner.vecs[i] = vi;
                if let Some(e) = err {
                    return Err(e);
                }
                read += n;
                found
            };
            let compared = found.compared;
            let mut scored = Vec::with_capacity(found.keys.len());
            for key in found.keys {
                // The document: to re-rank with its full vector, and to
                // check every condition once more on what is returned.
                if (quantized || !filter.is_empty()) && !docs.contains_key(&key) {
                    let Some(e) = inner.live(&key, now) else { continue };
                    let Value::Doc(d) = inner.value_of(&key, &e)? else { continue };
                    read += 1;
                    docs.insert(key.clone(), d);
                }
                if let Some(d) = docs.get(&key) {
                    if !passes(d) {
                        docs.remove(&key);
                        continue;
                    }
                }
                let full = quantized
                    .then(|| docs.get(&key).and_then(|d| vector::extract(d, &path)))
                    .flatten()
                    .filter(|v| v.len() == query.dims());
                let score = match full {
                    Some(v) => Some(query.score_vec(&v)),
                    None => inner.vecs[i].get(&key).map(|(v, n)| query.score(&v, n)),
                };
                if let Some(score) = score {
                    scored.push((key, score));
                }
            }
            let how = (!filter.is_empty()).then(|| {
                let mask = if indexed.is_empty() { String::new() } else { " and the secondary indexes".to_string() };
                format!("tested while walking the graph{mask}, {read} documents read")
            });
            (vector::top_k(scored, limit, &query), compared, compared, true, how)
        } else if let Some(i) = index {
            let scored = {
                let live = |k: &str| inner.index.get(k).is_some_and(|e| !e.is_expired(now));
                inner.vecs[i].scores(&q.pattern, &query, live)
            };
            let n = scored.len();
            if filter.is_empty() {
                (vector::top_k(scored, limit, &query), n, n, false, None)
            } else {
                // Post-filtering: closest first, until enough pass.
                let mut chosen = Vec::new();
                for (key, score) in vector::top_k(scored, n, &query) {
                    let Some(e) = inner.live(&key, now) else { continue };
                    let Value::Doc(doc) = inner.value_of(&key, &e)? else { continue };
                    read += 1;
                    if passes(&doc) {
                        docs.insert(key.clone(), doc);
                        chosen.push((key, score));
                        if chosen.len() >= limit {
                            break;
                        }
                    }
                }
                let how = format!("tested on the closest vectors first, {read} documents read");
                (chosen, n, n, false, Some(how))
            }
        } else {
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
                    if !passes(&doc) {
                        continue;
                    }
                    if let Some(v) = vector::extract(&doc, &path).filter(|v| v.len() == query.dims()) {
                        scored.push((k, query.score_vec(&v)));
                    }
                }
            }
            let n = scored.len();
            let how = (!filter.is_empty()).then(|| "tested on every document read".to_string());
            (vector::top_k(scored, limit, &query), examined, n, false, how)
        };
        let mut rows = Vec::new();
        for (key, score) in chosen {
            if let Some(doc) = docs.remove(&key) {
                rows.push((key, score, doc));
                continue;
            }
            let Some(e) = inner.live(&key, now) else { continue };
            if let Value::Doc(doc) = inner.value_of(&key, &e)? {
                rows.push((key, score, doc));
            }
        }
        Ok(VectorResult { rows, indexed: index.is_some(), approximate, examined, compared, filter: how })
    }

    /// The index that can answer `cond`: for `=` and `in`, a hash index is
    /// preferred; for `like` with text before its first wildcard, a B-tree.
    fn find_index(inner: &Inner, cond: &Cond) -> Option<usize> {
        if !Self::indexable(cond) {
            return None;
        }
        let usable = |s: &SecIndex| {
            s.paths.len() == 1
                && s.kind.serves(cond.op)
                && s.field == cond.field
                && pattern_covers(&s.pattern, &cond.pattern)
        };
        let prefixed = cond.op == CmpOp::Like && cond.value.as_str().is_some_and(|p| !like_prefix(p).is_empty());
        let preferred = if prefixed { IndexKind::BTree } else { IndexKind::Hash };
        let first = inner.sec.iter().position(|s| s.kind == preferred && usable(s));
        first.or_else(|| inner.sec.iter().position(usable))
    }

    /// The keys whose indexed value satisfies `cond`, sorted. Exact: the
    /// vector search takes them as the documents passing `cond`.
    fn index_candidates(idx: &SecIndex, cond: &Cond) -> Vec<String> {
        let mut out = Vec::new();
        let mut push = |set: &BTreeSet<String>| out.extend(set.iter().cloned());
        match cond.op {
            CmpOp::Eq => {
                if let Some(set) = SKey::from_json(&cond.value).and_then(|sk| idx.map.get(&sk)) {
                    push(set);
                }
                return out;
            }
            CmpOp::In => {
                for v in cond.value.as_array().into_iter().flatten() {
                    if let Some(set) = SKey::from_json(v).and_then(|sk| idx.map.get(&sk)) {
                        push(set);
                    }
                }
                out.sort();
                out.dedup();
                return out;
            }
            CmpOp::Like | CmpOp::ILike => {
                let Some(pattern) = cond.value.as_str() else {
                    return out;
                };
                let fold = cond.op == CmpOp::ILike;
                let matching = |sk: &SKey| matches!(sk, SKey::Str(t) if like_match(t, pattern, fold));
                let prefix = if fold { String::new() } else { like_prefix(pattern) };
                match &idx.map {
                    // The values that start with the pattern's text are
                    // next to each other in a B-tree.
                    Postings::Tree(map) if !prefix.is_empty() => {
                        let from = SKey::Str(prefix.clone());
                        for (sk, set) in map.range(from..) {
                            if !matches!(sk, SKey::Str(t) if t.starts_with(&prefix)) {
                                break;
                            }
                            if matching(sk) {
                                push(set);
                            }
                        }
                    }
                    // Otherwise every distinct value is tested: still
                    // without reading a document.
                    map => map.for_each(|sk, set| {
                        if matching(sk) {
                            push(set)
                        }
                    }),
                }
                out.sort();
                return out;
            }
            _ => {}
        }
        let Some(sk) = SKey::from_json(&cond.value) else {
            return out;
        };
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
            _ => {}
        }
        out.sort();
        out
    }

    /// Whether an index on the field of `cond` could answer it: false for
    /// `!=`, and for a value that cannot be looked up.
    pub fn indexable(cond: &Cond) -> bool {
        match cond.op {
            CmpOp::Ne => false,
            CmpOp::In => cond.value.as_array().is_some_and(|l| l.iter().all(|v| SKey::from_json(v).is_some())),
            CmpOp::Like | CmpOp::ILike => cond.value.is_string(),
            _ => SKey::from_json(&cond.value).is_some(),
        }
    }

    /// How `cond` would be executed, without running it.
    pub fn explain(&self, cond: &Cond) -> Plan {
        match Query::single(cond.clone()) {
            Ok(q) => self.explain_query(&q),
            Err(_) => Self::scan_plan(&self.lock(), &cond.pattern),
        }
    }

    /// How `q` would be executed, without running it.
    pub fn explain_query(&self, q: &Query) -> Plan {
        let inner = self.lock();
        match q.expr.as_ref().and_then(|e| Self::index_plan(&inner, e)) {
            Some(p) => p.plan(inner.index.len()),
            None => Self::scan_plan(&inner, &q.pattern),
        }
    }

    fn scan_plan(inner: &Inner, pattern: &str) -> Plan {
        let prefix = glob_prefix(pattern);
        let candidates = inner
            .index
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .take_while(|(k, _)| k.starts_with(prefix))
            .count();
        Plan {
            strategy: if prefix.is_empty() { "FULL SCAN" } else { "PREFIX SCAN" },
            index: None,
            candidates,
            total_keys: inner.index.len(),
        }
    }

    /// The keys the secondary indexes give for `e`, sorted, when they can:
    /// a comparison with an index, an `and` with at least one such part
    /// (their keys intersected), an `or` whose parts all have one (merged).
    /// `not` never: it selects what an index does not list.
    fn index_plan(inner: &Inner, e: &Expr) -> Option<IndexPlan> {
        match e {
            Expr::Test(t) => {
                let i = Self::find_index(inner, &t.cond)?;
                let idx = &inner.sec[i];
                let kind = if idx.kind == IndexKind::Hash { " using hash" } else { "" };
                let mut keys = Self::index_candidates(idx, &t.cond);
                keys.dedup();
                Some(IndexPlan { keys, used: format!("{} ({}){kind}", idx.pattern, idx.field), indexes: 1 })
            }
            Expr::And(parts) => {
                let mut plans: Vec<IndexPlan> = parts.iter().filter_map(|p| Self::index_plan(inner, p)).collect();
                // Smallest first: each intersection then walks fewer keys.
                plans.sort_by_key(|p| p.keys.len());
                let mut it = plans.into_iter();
                let mut acc = it.next()?;
                for p in it {
                    if acc.keys.is_empty() {
                        break;
                    }
                    acc.keys = intersect_sorted(&acc.keys, &p.keys);
                    acc.used = format!("{} and {}", acc.used, p.used);
                    acc.indexes += p.indexes;
                }
                Some(acc)
            }
            Expr::Or(parts) => {
                let plans: Option<Vec<IndexPlan>> = parts.iter().map(|p| Self::index_plan(inner, p)).collect();
                let mut it = plans?.into_iter();
                let mut acc = it.next()?;
                for p in it {
                    acc.keys = union_sorted(&acc.keys, &p.keys);
                    acc.used = format!("{} or {}", acc.used, p.used);
                    acc.indexes += p.indexes;
                }
                acc.used = format!("({})", acc.used);
                Some(acc)
            }
            Expr::Not(_) => None,
        }
    }

    /// Documents whose key matches `cond.pattern` and whose field satisfies
    /// the comparison. `limit == 0` means no limit.
    pub fn query(&self, cond: &Cond, limit: usize) -> Result<(Vec<(String, Json)>, Plan)> {
        let q = Query::single(cond.clone()).map_err(KivaError::Invalid)?;
        self.query_expr(&q, limit)
    }

    /// The documents `q` selects, in key order. `limit == 0` means no limit.
    pub fn query_expr(&self, q: &Query, limit: usize) -> Result<(Vec<(String, Json)>, Plan)> {
        let mut rows = Vec::new();
        let plan = self.scan_query(q, limit, |key, doc| {
            rows.push((key, doc));
            Ok(())
        })?;
        Ok((rows, plan))
    }

    /// Calls `f` with each document `q` selects, in key order, without
    /// keeping them (aggregates fold them as they come). `f` runs while
    /// the engine is locked. `limit == 0` means no limit.
    pub fn scan_query(
        &self,
        q: &Query,
        limit: usize,
        mut f: impl FnMut(String, Json) -> std::result::Result<(), String>,
    ) -> Result<Plan> {
        let mut inner = self.lock();
        let total_keys = inner.index.len();
        let (candidates, plan) = match q.expr.as_ref().and_then(|e| Self::index_plan(&inner, e)) {
            Some(p) => {
                let plan = p.plan(total_keys);
                (p.keys, plan)
            }
            None => {
                let plan = Self::scan_plan(&inner, &q.pattern);
                let prefix = glob_prefix(&q.pattern);
                let keys = inner
                    .index
                    .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                    .take_while(|(k, _)| k.starts_with(prefix))
                    .filter(|(_, e)| e.ty.is_doc())
                    .map(|(k, _)| k.clone())
                    .collect();
                (keys, plan)
            }
        };
        let now = now_ms();
        let mut found = 0;
        for key in candidates {
            if !glob_match(&q.pattern, &key) {
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
            // An index only narrows the reading: the whole condition decides.
            if q.matches(&doc) {
                f(key, doc).map_err(KivaError::Invalid)?;
                found += 1;
                if limit != 0 && found >= limit {
                    break;
                }
            }
        }
        Ok(plan)
    }
}

/// Keys given by secondary indexes for part of a condition.
struct IndexPlan {
    /// Sorted, without duplicates.
    keys: Vec<String>,
    /// The indexes used, as a formula: `user:* (role) and user:* (age)`.
    used: String,
    indexes: usize,
}

impl IndexPlan {
    fn plan(&self, total_keys: usize) -> Plan {
        Plan {
            strategy: if self.indexes > 1 { "INDEX MERGE" } else { "INDEX SCAN" },
            index: Some(self.used.clone()),
            candidates: self.keys.len(),
            total_keys,
        }
    }
}

fn intersect_sorted(a: &[String], b: &[String]) -> Vec<String> {
    let (mut i, mut j, mut out) = (0, 0, Vec::with_capacity(a.len().min(b.len())));
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i].clone());
                i += 1;
                j += 1;
            }
        }
    }
    out
}

fn union_sorted(a: &[String], b: &[String]) -> Vec<String> {
    let (mut i, mut j, mut out) = (0, 0, Vec::with_capacity(a.len() + b.len()));
    while i < a.len() || j < b.len() {
        let next = match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if x == y => {
                i += 1;
                j += 1;
                x
            }
            (Some(x), Some(y)) if x < y => {
                i += 1;
                x
            }
            (Some(x), None) => {
                i += 1;
                x
            }
            (_, Some(y)) => {
                j += 1;
                y
            }
            (None, None) => break,
        };
        out.push(next.clone());
    }
    out
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
    if let Some(q) = def["quantize"].as_str().and_then(hnsw::Quant::parse) {
        p.quant = q;
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
    /// Conditions on the documents' other fields, all of which must hold;
    /// their pattern is the query's.
    pub filter: Vec<Cond>,
}

impl VectorQuery {
    pub fn new(pattern: &str, field: &str, vector: Vec<f32>, metric: Metric, limit: usize) -> VectorQuery {
        VectorQuery {
            pattern: pattern.into(),
            field: field.into(),
            vector,
            metric,
            limit,
            ef: 0,
            exact: false,
            filter: Vec::new(),
        }
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
    /// Documents whose field held something else than numbers (text to embed).
    pub not_vectors: usize,
    /// HNSW: deleted or replaced vectors waiting for the graph to be rebuilt.
    pub tombstones: usize,
    pub memory_bytes: usize,
    /// HNSW: candidates a search keeps when it does not say.
    pub ef_search: usize,
    /// HNSW: tuned again as it grows (`tune ... auto`).
    pub auto_tune: Option<AutoTune>,
}

/// One `ef` tried by [`Engine::tune_vector_index`].
#[derive(Clone, Debug)]
pub struct TuneStep {
    pub ef: usize,
    /// Share of the exact closest vectors the graph found.
    pub recall: f64,
    /// Vectors compared per search, on average.
    pub compared: usize,
    pub micros: f64,
}

/// What tuning an HNSW index measured and chose.
#[derive(Clone, Debug)]
pub struct TuneReport {
    pub steps: Vec<TuneStep>,
    /// The `ef_search` now stored with the index.
    pub chosen: usize,
    /// Whether `chosen` reaches the recall asked for.
    pub reached: bool,
    pub queries: usize,
    /// Whether indexed vectors stood for the queries (each searched without
    /// its own node) rather than queries given by the application.
    pub from_data: bool,
    pub vectors: usize,
    /// An exact search over the same index, for comparison.
    pub exact_micros: f64,
}

/// The values of `ef` tuning tries, smallest first.
const TUNE_LADDER: &[usize] = &[10, 16, 25, 40, 64, 100, 160, 250, 400, 640, 1000, 1600, 2500, 4000];

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
    /// How the filter was applied, when there is one.
    pub filter: Option<String>,
}
