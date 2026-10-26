// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Approximate nearest-neighbour search with HNSW (Hierarchical Navigable
//! Small World graphs, Malkov and Yashunin, 2016).
//!
//! Every vector is a node of a graph built in layers: the bottom layer holds
//! all of them, each upper layer a random, ever smaller sample. A node keeps
//! links to a few close nodes on each of its layers. A search enters at the
//! top, walks greedily towards the query on each layer, and explores the
//! bottom layer with a list of `ef` best candidates. It compares a small
//! fraction of the vectors, so it stays fast with millions of them, at the
//! cost of sometimes missing one of the true nearest neighbours.
//!
//! A deleted or replaced vector stays in the graph as a tombstone, still used
//! to navigate but never returned. When tombstones reach a third of the nodes,
//! the graph is rebuilt from the live vectors.
//!
//! **Scalar quantization** ([`Quant::Sq8`]): each vector is kept as one byte
//! per dimension, `lo + step * code` with `lo` and `step` from the vector's
//! own minimum and maximum, a quarter of the memory of 32-bit numbers. The
//! distances are computed on the codes directly; they are close to the true
//! ones, not equal, so the engine re-ranks what the graph finds with the full
//! vectors of the documents, on disk.
//!
//! **Filtered search** ([`Hnsw::search_filtered`]): a condition on the
//! documents is tested while the graph is explored. Nodes that fail it are
//! still walked through, so the graph stays connected, but only those that
//! pass fill the list of best candidates; the search ends with `ef` of them,
//! however rare they are, instead of filtering a list found without them.

use crate::vector::{dot, l2_squared, norm, Metric};
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};

/// Links per node on the upper layers (twice as many on the bottom one).
pub const DEFAULT_M: usize = 16;
/// Candidates kept while inserting: higher builds a better graph, slower.
pub const DEFAULT_EF_CONSTRUCTION: usize = 200;
/// Candidates kept while searching, unless the query asks for another value.
pub const DEFAULT_EF_SEARCH: usize = 100;

const MAX_LEVEL: usize = 16;
const NONE: u32 = u32::MAX;

/// How the graph keeps its vectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quant {
    /// 32-bit numbers, as given.
    #[default]
    None,
    /// One byte per dimension: a quarter of the memory.
    Sq8,
}

impl Quant {
    pub fn parse(s: &str) -> Option<Quant> {
        match s.to_ascii_lowercase().as_str() {
            "none" | "f32" => Some(Quant::None),
            "sq8" | "int8" => Some(Quant::Sq8),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Quant::None => "none",
            Quant::Sq8 => "sq8",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    /// The metric the graph is built for; searches with it use the graph.
    pub metric: Metric,
    pub m: usize,
    pub ef_construction: usize,
    pub quant: Quant,
}

impl Default for Params {
    fn default() -> Self {
        Params { metric: Metric::Cosine, m: DEFAULT_M, ef_construction: DEFAULT_EF_CONSTRUCTION, quant: Quant::None }
    }
}

/// A vector quantized to one byte per dimension: `x[i] ~ lo + step * codes[i]`.
#[derive(Clone, Debug)]
struct Sq8 {
    codes: Vec<u8>,
    lo: f32,
    step: f32,
    /// Sum of the codes, for the dot products.
    sum: u32,
}

impl Sq8 {
    fn encode(v: &[f32]) -> Sq8 {
        let lo = v.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let step = if hi > lo { (hi - lo) / 255.0 } else { 0.0 };
        let codes: Vec<u8> =
            v.iter().map(|x| if step > 0.0 { ((x - lo) / step).round().clamp(0.0, 255.0) as u8 } else { 0 }).collect();
        Sq8::from_codes(codes, lo, step)
    }

    fn from_codes(codes: Vec<u8>, lo: f32, step: f32) -> Sq8 {
        let sum = codes.iter().map(|&c| c as u32).sum();
        Sq8 { codes, lo, step, sum }
    }

    fn decode(&self) -> Vec<f32> {
        self.codes.iter().map(|&c| self.lo + self.step * c as f32).collect()
    }

    /// Dot product with a full vector whose sum is `qsum`.
    fn dot_full(&self, q: &[f32], qsum: f32) -> f32 {
        let qc: f32 = q.iter().zip(&self.codes).map(|(x, &c)| x * c as f32).sum();
        self.lo * qsum + self.step * qc
    }

    /// Dot product with another quantized vector, from the integer dot
    /// product of the codes.
    fn dot(&self, o: &Sq8) -> f32 {
        let cc: u32 = self.codes.iter().zip(&o.codes).map(|(&a, &b)| a as u32 * b as u32).sum();
        let n = self.codes.len() as f32;
        n * self.lo * o.lo
            + self.lo * o.step * o.sum as f32
            + o.lo * self.step * self.sum as f32
            + self.step * o.step * cc as f32
    }
}

#[derive(Clone, Debug)]
enum Stored {
    Full(Vec<f32>),
    Sq8(Sq8),
}

impl Stored {
    fn new(v: Vec<f32>, quant: Quant) -> Stored {
        match quant {
            Quant::None => Stored::Full(v),
            Quant::Sq8 => Stored::Sq8(Sq8::encode(&v)),
        }
    }

    fn dims(&self) -> usize {
        match self {
            Stored::Full(v) => v.len(),
            Stored::Sq8(s) => s.codes.len(),
        }
    }

    fn vector(&self) -> Cow<'_, [f32]> {
        match self {
            Stored::Full(v) => Cow::Borrowed(v),
            Stored::Sq8(s) => Cow::Owned(s.decode()),
        }
    }
}

/// A graph distance from a dot product and the two norms.
fn from_dot(metric: Metric, dot: f32, an: f32, bn: f32) -> f32 {
    match metric {
        Metric::Cosine => {
            let d = an * bn;
            if d == 0.0 {
                1.0
            } else {
                1.0 - dot / d
            }
        }
        Metric::Dot => -dot,
        Metric::L2 => (an * an + bn * bn - 2.0 * dot).max(0.0),
    }
}

/// What a search compares the nodes with.
struct Probe<'a> {
    q: &'a [f32],
    norm: f32,
    sum: f32,
}

impl<'a> Probe<'a> {
    fn new(q: &'a [f32]) -> Probe<'a> {
        Probe { q, norm: norm(q), sum: q.iter().sum() }
    }
}

struct Node {
    key: String,
    vector: Stored,
    norm: f32,
    /// Neighbours on each layer the node belongs to, bottom first.
    links: Vec<Vec<u32>>,
    deleted: bool,
}

/// A node and its distance to what is being searched; lower is closer.
#[derive(Clone, Copy, Debug)]
struct Near {
    dist: f32,
    id: u32,
}

impl PartialEq for Near {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Near {}
impl PartialOrd for Near {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Near {
    fn cmp(&self, o: &Self) -> Ordering {
        self.dist.total_cmp(&o.dist).then(self.id.cmp(&o.id))
    }
}

/// Distance for the graph, lower is closer, whatever the metric.
fn distance(metric: Metric, a: &[f32], an: f32, b: &[f32], bn: f32) -> f32 {
    match metric {
        Metric::Cosine => {
            let d = an * bn;
            if d == 0.0 {
                1.0
            } else {
                1.0 - dot(a, b) / d
            }
        }
        Metric::Dot => -dot(a, b),
        Metric::L2 => l2_squared(a, b),
    }
}

/// Nodes already seen by the search in progress: a mark per node, valid
/// for one search, so that no set is allocated or cleared per search.
#[derive(Default)]
struct Visits {
    marks: Vec<u32>,
    epoch: u32,
}

impl Visits {
    fn start(&mut self, nodes: usize) {
        if self.marks.len() < nodes {
            self.marks.resize(nodes, 0);
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.marks.fill(0);
            self.epoch = 1;
        }
    }

    /// True the first time `id` is seen in this search.
    fn first(&mut self, id: u32) -> bool {
        let m = &mut self.marks[id as usize];
        let first = *m != self.epoch;
        *m = self.epoch;
        first
    }
}

pub struct Hnsw {
    pub params: Params,
    nodes: Vec<Node>,
    ids: HashMap<String, u32>,
    entry: u32,
    max_level: usize,
    deleted: usize,
    rng: u64,
    /// Insertions and removals since the graph was last saved.
    pub changes: u64,
    /// Memory held by the nodes and the key map, kept up to date so that a
    /// write can be checked against the vector memory bound in O(1).
    bytes: usize,
    visits: RefCell<Visits>,
}

/// What a search found: the keys, closest first, and how many vectors it compared.
pub struct Found {
    pub keys: Vec<String>,
    pub compared: usize,
}

impl Hnsw {
    pub fn new(params: Params) -> Hnsw {
        Hnsw {
            params,
            nodes: Vec::new(),
            ids: HashMap::new(),
            entry: NONE,
            max_level: 0,
            deleted: 0,
            rng: 0x9E37_79B9_7F4A_7C15,
            changes: 0,
            bytes: 0,
            visits: RefCell::default(),
        }
    }

    /// Live vectors.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Deleted or replaced vectors still in the graph.
    pub fn tombstones(&self) -> usize {
        self.deleted
    }

    /// Memory of the graph, tombstones included, counting every layer of a
    /// node as full: an estimate that a growing graph does not outrun.
    pub fn memory_bytes(&self) -> usize {
        self.bytes
    }

    /// What a node of `levels` layers and its entry in the key map cost.
    pub fn node_bytes(&self, key: usize, dims: usize, levels: usize) -> usize {
        let links: usize = (0..levels).map(|l| self.max_links(l) * 4 + 24).sum();
        let vector = match self.params.quant {
            Quant::None => dims * 4,
            Quant::Sq8 => dims + 12,
        };
        key + vector + links + 80 + key + 40
    }

    /// Whether the vectors are kept quantized, so their distances are close
    /// to the true ones but not equal.
    pub fn quantized(&self) -> bool {
        self.params.quant != Quant::None
    }

    pub fn clear(&mut self) {
        *self = Hnsw { rng: self.rng, ..Hnsw::new(self.params) };
        self.changes = 1;
    }

    /// The vector stored for `key`, with its norm; with quantization, the
    /// vector the codes stand for.
    pub fn get(&self, key: &str) -> Option<(Cow<'_, [f32]>, f32)> {
        let n = &self.nodes[*self.ids.get(key)? as usize];
        Some((n.vector.vector(), n.norm))
    }

    /// Whether `key` holds what `vector` would be stored as: then writing
    /// it again changes nothing.
    pub fn same(&self, key: &str, vector: &[f32]) -> bool {
        let Some(&id) = self.ids.get(key) else { return false };
        match &self.nodes[id as usize].vector {
            Stored::Full(v) => v == vector,
            Stored::Sq8(s) => {
                let q = Sq8::encode(vector);
                q.codes == s.codes && q.lo == s.lo && q.step == s.step
            }
        }
    }

    /// Every live vector.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Cow<'_, [f32]>, f32)> {
        self.nodes.iter().filter(|n| !n.deleted).map(|n| (n.key.as_str(), n.vector.vector(), n.norm))
    }

    pub fn remove(&mut self, key: &str) {
        if let Some(id) = self.ids.remove(key) {
            // The node stays as a tombstone until the graph is rebuilt.
            self.bytes -= key.len() + 40;
            self.nodes[id as usize].deleted = true;
            self.deleted += 1;
            self.changes += 1;
        }
    }

    /// True when tombstones make up a third of the graph.
    pub fn needs_rebuild(&self) -> bool {
        self.deleted >= 64 && self.deleted * 3 >= self.nodes.len()
    }

    /// Builds the graph again from the live vectors only.
    pub fn rebuild(&mut self) {
        let live: Vec<(String, Vec<f32>)> = std::mem::take(&mut self.nodes)
            .into_iter()
            .filter(|n| !n.deleted)
            .map(|n| (n.key, n.vector.vector().into_owned()))
            .collect();
        *self = Hnsw { rng: self.rng, ..Hnsw::new(self.params) };
        for (k, v) in live {
            self.insert(&k, v);
        }
        self.changes = self.changes.max(1);
    }

    fn dist_to(&self, p: &Probe, id: u32) -> f32 {
        let n = &self.nodes[id as usize];
        match &n.vector {
            Stored::Full(v) => distance(self.params.metric, p.q, p.norm, v, n.norm),
            Stored::Sq8(s) => from_dot(self.params.metric, s.dot_full(p.q, p.sum), p.norm, n.norm),
        }
    }

    fn dist_nodes(&self, a: u32, b: u32) -> f32 {
        let (na, nb) = (&self.nodes[a as usize], &self.nodes[b as usize]);
        match (&na.vector, &nb.vector) {
            (Stored::Full(x), Stored::Full(y)) => distance(self.params.metric, x, na.norm, y, nb.norm),
            (Stored::Sq8(x), Stored::Sq8(y)) => from_dot(self.params.metric, x.dot(y), na.norm, nb.norm),
            (x, y) => distance(self.params.metric, &x.vector(), na.norm, &y.vector(), nb.norm),
        }
    }

    /// A layer drawn so that each one holds about 1/M of the one below.
    fn random_level(&mut self) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let u = ((self.rng >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
        let ml = 1.0 / (self.params.m.max(2) as f64).ln();
        ((-u.ln() * ml) as usize).min(MAX_LEVEL)
    }

    fn max_links(&self, level: usize) -> usize {
        if level == 0 {
            self.params.m * 2
        } else {
            self.params.m
        }
    }

    /// Moves to the closest neighbour on `level` until none is closer.
    fn greedy(&self, mut cur: Near, level: usize, dist: &impl Fn(u32) -> f32, count: &mut usize) -> Near {
        loop {
            let mut moved = false;
            for &n in &self.nodes[cur.id as usize].links[level] {
                let d = dist(n);
                *count += 1;
                if d < cur.dist {
                    cur = Near { dist: d, id: n };
                    moved = true;
                }
            }
            if !moved {
                return cur;
            }
        }
    }

    /// The `ef` closest nodes reachable on `level` from `entry`, closest first.
    fn search_layer(
        &self,
        entry: &[Near],
        ef: usize,
        level: usize,
        dist: &impl Fn(u32) -> f32,
        count: &mut usize,
    ) -> Vec<Near> {
        self.search_layer_with(entry, ef, level, dist, None, count)
    }

    /// [`Hnsw::search_layer`], keeping among the best only the nodes
    /// `allow` lets through; the others are still walked through. `allow`
    /// is asked once per node, and only for a node close enough to count.
    fn search_layer_with(
        &self,
        entry: &[Near],
        ef: usize,
        level: usize,
        dist: &impl Fn(u32) -> f32,
        allow: Option<&dyn Fn(u32) -> bool>,
        count: &mut usize,
    ) -> Vec<Near> {
        let allowed = |id: u32| allow.is_none_or(|f| f(id));
        let mut visits = self.visits.borrow_mut();
        visits.start(self.nodes.len());
        for n in entry {
            visits.first(n.id);
        }
        let mut candidates: BinaryHeap<Reverse<Near>> = entry.iter().map(|n| Reverse(*n)).collect();
        let mut best: BinaryHeap<Near> = entry.iter().copied().filter(|n| allowed(n.id)).collect();
        while let Some(Reverse(c)) = candidates.pop() {
            if best.len() >= ef && best.peek().is_some_and(|w| c.dist > w.dist) {
                break;
            }
            for &n in &self.nodes[c.id as usize].links[level] {
                if !visits.first(n) {
                    continue;
                }
                let d = dist(n);
                // A hidden node is neither a result nor a way through.
                if d == f32::INFINITY {
                    continue;
                }
                *count += 1;
                if best.len() < ef || best.peek().is_some_and(|w| d < w.dist) {
                    let near = Near { dist: d, id: n };
                    candidates.push(Reverse(near));
                    if allowed(n) {
                        best.push(near);
                        if best.len() > ef {
                            best.pop();
                        }
                    }
                }
            }
        }
        best.into_sorted_vec()
    }

    /// Keeps up to `m` of `candidates` (closest first), preferring ones that
    /// are not closer to an already kept neighbour than to the base node:
    /// links then point in varied directions and the graph stays navigable.
    fn select(&self, candidates: &[Near], m: usize) -> Vec<u32> {
        let mut kept: Vec<Near> = Vec::with_capacity(m);
        let mut skipped = Vec::new();
        for &c in candidates {
            if kept.len() >= m {
                break;
            }
            if kept.iter().all(|k| self.dist_nodes(c.id, k.id) > c.dist) {
                kept.push(c);
            } else {
                skipped.push(c);
            }
        }
        for c in skipped {
            if kept.len() >= m {
                break;
            }
            kept.push(c);
        }
        kept.into_iter().map(|n| n.id).collect()
    }

    /// Adds a link from `from` to `to` on `level`, pruning when full.
    fn link(&mut self, from: u32, to: u32, level: usize) {
        let max = self.max_links(level);
        let links = &mut self.nodes[from as usize].links[level];
        if links.contains(&to) {
            return;
        }
        links.push(to);
        if links.len() <= max {
            return;
        }
        let mut near: Vec<Near> = self.nodes[from as usize].links[level]
            .iter()
            .map(|&n| Near { dist: self.dist_nodes(from, n), id: n })
            .collect();
        near.sort();
        let kept = self.select(&near, max);
        self.nodes[from as usize].links[level] = kept;
    }

    /// Adds or replaces the vector of `key`.
    pub fn insert(&mut self, key: &str, vector: Vec<f32>) {
        self.remove(key);
        let level = self.random_level();
        let id = self.nodes.len() as u32;
        self.bytes += self.node_bytes(key.len(), vector.len(), level + 1);
        let vector = Stored::new(vector, self.params.quant);
        let vn = norm(&vector.vector());
        self.nodes.push(Node {
            key: key.to_string(),
            vector,
            norm: vn,
            links: vec![Vec::new(); level + 1],
            deleted: false,
        });
        self.ids.insert(key.to_string(), id);
        self.changes += 1;
        if self.entry == NONE {
            self.entry = id;
            self.max_level = level;
            return;
        }
        let mut count = 0;
        let mut cur = Near { dist: self.dist_nodes(id, self.entry), id: self.entry };
        for l in (level + 1..=self.max_level).rev() {
            cur = self.greedy(cur, l, &|o| self.dist_nodes(id, o), &mut count);
        }
        let mut entry = vec![cur];
        for l in (0..=level.min(self.max_level)).rev() {
            let found =
                self.search_layer(&entry, self.params.ef_construction, l, &|o| self.dist_nodes(id, o), &mut count);
            let neighbours = self.select(&found, self.params.m);
            for &n in &neighbours {
                self.link(n, id, l);
            }
            self.nodes[id as usize].links[l] = neighbours;
            entry = found;
        }
        if level > self.max_level {
            self.max_level = level;
            self.entry = id;
        }
    }

    /// The `k` keys closest to `query` among those `accept` lets through.
    /// `ef` (at least `k`) trades speed for accuracy; when too few accepted
    /// keys are found, the search widens until the whole graph is explored.
    pub fn search(&self, query: &[f32], k: usize, ef: usize, accept: impl Fn(&str) -> bool) -> Found {
        self.search_hiding(query, k, ef, accept, None)
    }

    /// Like [`Hnsw::search`], as if the node of `hidden` were not in the
    /// graph: the search never steps on it. Searching with an indexed vector
    /// while hiding its own node tells how the graph serves a new query that
    /// resembles the data, which is how tuning measures the graph.
    pub fn search_hiding(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
        accept: impl Fn(&str) -> bool,
        hidden: Option<&str>,
    ) -> Found {
        if self.entry == NONE || k == 0 {
            return Found { keys: Vec::new(), compared: 0 };
        }
        let probe = Probe::new(query);
        let hidden = hidden.and_then(|k| self.ids.get(k).copied()).filter(|&h| h != self.entry);
        let dist = |o: u32| if Some(o) == hidden { f32::INFINITY } else { self.dist_to(&probe, o) };
        let mut compared = 1;
        let mut cur = Near { dist: dist(self.entry), id: self.entry };
        for l in (1..=self.max_level).rev() {
            cur = self.greedy(cur, l, &dist, &mut compared);
        }
        let mut ef = ef.max(k);
        loop {
            let found = self.search_layer(&[cur], ef, 0, &dist, &mut compared);
            let keys: Vec<String> = found
                .iter()
                .filter(|n| Some(n.id) != hidden)
                .map(|n| &self.nodes[n.id as usize])
                .filter(|n| !n.deleted && accept(&n.key))
                .take(k)
                .map(|n| n.key.clone())
                .collect();
            if keys.len() >= k || ef >= self.nodes.len() {
                return Found { keys, compared };
            }
            ef = (ef * 2).min(self.nodes.len());
        }
    }

    /// The `k` closest keys among those `accept` lets through, tested while
    /// the bottom layer is explored: the search ends with `ef` (at least
    /// `k`) accepted candidates, or once every node it can reach was seen.
    /// `accept` is asked at most once per node, only for nodes close enough
    /// to be among the best; a rare condition makes the search walk further.
    pub fn search_filtered(&self, query: &[f32], k: usize, ef: usize, accept: impl Fn(&str) -> bool) -> Found {
        if self.entry == NONE || k == 0 {
            return Found { keys: Vec::new(), compared: 0 };
        }
        let probe = Probe::new(query);
        let dist = |o: u32| self.dist_to(&probe, o);
        let mut compared = 1;
        let mut cur = Near { dist: dist(self.entry), id: self.entry };
        for l in (1..=self.max_level).rev() {
            cur = self.greedy(cur, l, &dist, &mut compared);
        }
        let allow = |id: u32| {
            let n = &self.nodes[id as usize];
            !n.deleted && accept(&n.key)
        };
        let found = self.search_layer_with(&[cur], ef.max(k), 0, &dist, Some(&allow), &mut compared);
        let keys = found.iter().take(k).map(|n| self.nodes[n.id as usize].key.clone()).collect();
        Found { keys, compared }
    }

    // -- persistence --------------------------------------------------------

    /// Appends the graph to `out` (little-endian, read back by [`Hnsw::read`]).
    pub fn write(&self, out: &mut Vec<u8>) {
        let metric = match self.params.metric {
            Metric::Cosine => 0u8,
            Metric::Dot => 1,
            Metric::L2 => 2,
        };
        // The quantization in the high bits: 0 in files written before it.
        let quant = match self.params.quant {
            Quant::None => 0u8,
            Quant::Sq8 => 1,
        };
        out.push(metric | (quant << 4));
        put_u32(out, self.params.m as u32);
        put_u32(out, self.params.ef_construction as u32);
        put_u32(out, self.entry);
        put_u32(out, self.max_level as u32);
        out.extend_from_slice(&self.rng.to_le_bytes());
        put_u32(out, self.nodes.len() as u32);
        for n in &self.nodes {
            put_bytes(out, n.key.as_bytes());
            out.push(n.deleted as u8);
            put_u32(out, n.vector.dims() as u32);
            match &n.vector {
                Stored::Full(v) => {
                    for x in v {
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                }
                Stored::Sq8(s) => {
                    out.extend_from_slice(&s.lo.to_le_bytes());
                    out.extend_from_slice(&s.step.to_le_bytes());
                    out.extend_from_slice(&s.codes);
                }
            }
            out.push(n.links.len() as u8);
            for l in &n.links {
                put_u32(out, l.len() as u32);
                for id in l {
                    put_u32(out, *id);
                }
            }
        }
    }

    /// Reads a graph written by [`Hnsw::write`]; `None` if it is malformed.
    pub fn read(r: &mut Reader) -> Option<Hnsw> {
        let flags = r.u8()?;
        let metric = match flags & 0x0f {
            0 => Metric::Cosine,
            1 => Metric::Dot,
            2 => Metric::L2,
            _ => return None,
        };
        let quant = match flags >> 4 {
            0 => Quant::None,
            1 => Quant::Sq8,
            _ => return None,
        };
        let params = Params { metric, m: r.u32()? as usize, ef_construction: r.u32()? as usize, quant };
        let entry = r.u32()?;
        let max_level = r.u32()? as usize;
        let rng = r.u64()?;
        let count = r.u32()? as usize;
        let mut g = Hnsw { rng, entry, max_level, ..Hnsw::new(params) };
        g.nodes.reserve(count);
        for _ in 0..count {
            let key = String::from_utf8(r.bytes()?.to_vec()).ok()?;
            let deleted = r.u8()? != 0;
            let dims = r.u32()? as usize;
            let f32_at = |r: &mut Reader| r.take(4).and_then(|b| b.try_into().ok()).map(f32::from_le_bytes);
            let vector = match quant {
                Quant::None => {
                    let mut v = Vec::with_capacity(dims.min(crate::vector::MAX_DIMS));
                    for _ in 0..dims {
                        v.push(f32_at(r)?);
                    }
                    Stored::Full(v)
                }
                Quant::Sq8 => {
                    let (lo, step) = (f32_at(r)?, f32_at(r)?);
                    Stored::Sq8(Sq8::from_codes(r.take(dims)?.to_vec(), lo, step))
                }
            };
            let levels = r.u8()? as usize;
            let mut links = Vec::with_capacity(levels);
            for _ in 0..levels {
                let n = r.u32()? as usize;
                let mut l = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    let id = r.u32()?;
                    if id as usize >= count {
                        return None;
                    }
                    l.push(id);
                }
                links.push(l);
            }
            let id = g.nodes.len() as u32;
            g.bytes += g.node_bytes(key.len(), vector.dims(), links.len());
            if deleted {
                g.bytes -= key.len() + 40;
                g.deleted += 1;
            } else {
                g.ids.insert(key.clone(), id);
            }
            g.nodes.push(Node { norm: norm(&vector.vector()), key, vector, links, deleted });
        }
        let valid_entry = if count == 0 { entry == NONE } else { (entry as usize) < count };
        if !valid_entry || (count > 0 && g.nodes[entry as usize].links.len() != max_level + 1) {
            return None;
        }
        // A link on a layer must lead to a node that has that layer, or a
        // search would index past its links.
        let consistent = g.nodes.iter().all(|n| {
            n.vector.dims() == g.nodes[0].vector.dims()
                && n.links.iter().enumerate().all(|(l, ids)| ids.iter().all(|&id| g.nodes[id as usize].links.len() > l))
        });
        consistent.then_some(g)
    }
}

fn put_u32(out: &mut Vec<u8>, x: u32) {
    out.extend_from_slice(&x.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

/// Reads the values written by [`Hnsw::write`] and the engine's graph file.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.buf.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }

    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
}

/// Writes a length-prefixed string, as [`Reader::bytes`] reads it.
pub fn write_str(out: &mut Vec<u8>, s: &str) {
    put_bytes(out, s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_vectors(n: usize, dims: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                (0..dims)
                    .map(|_| {
                        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        ((s >> 33) as f32 / u32::MAX as f32) - 0.25
                    })
                    .collect()
            })
            .collect()
    }

    fn exact(vectors: &[Vec<f32>], q: &[f32], k: usize, metric: Metric) -> Vec<String> {
        let qn = norm(q);
        let mut all: Vec<(f32, usize)> =
            vectors.iter().enumerate().map(|(i, v)| (distance(metric, q, qn, v, norm(v)), i)).collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        all.into_iter().take(k).map(|(_, i)| format!("v{i}")).collect()
    }

    fn recall(metric: Metric) -> f64 {
        let vectors = random_vectors(2000, 16, 7);
        let mut g = Hnsw::new(Params { metric, ..Params::default() });
        for (i, v) in vectors.iter().enumerate() {
            g.insert(&format!("v{i}"), v.clone());
        }
        let queries = random_vectors(50, 16, 99);
        let mut hits = 0;
        for q in &queries {
            let truth = exact(&vectors, q, 10, metric);
            let found = g.search(q, 10, DEFAULT_EF_SEARCH, |_| true);
            hits += found.keys.iter().filter(|k| truth.contains(k)).count();
        }
        hits as f64 / (queries.len() * 10) as f64
    }

    #[test]
    fn finds_the_nearest_neighbours_for_every_metric() {
        for metric in [Metric::Cosine, Metric::L2, Metric::Dot] {
            let r = recall(metric);
            assert!(r >= 0.9, "recall@10 with {metric:?} is {r}");
        }
    }

    #[test]
    fn deletions_replacements_and_rebuilds() {
        let vectors = random_vectors(500, 8, 3);
        let mut g = Hnsw::new(Params::default());
        for (i, v) in vectors.iter().enumerate() {
            g.insert(&format!("v{i}"), v.clone());
        }
        // The vector itself is its nearest neighbour.
        assert_eq!(g.search(&vectors[42], 1, 50, |_| true).keys, ["v42"]);
        g.remove("v42");
        assert!(!g.search(&vectors[42], 5, 50, |_| true).keys.contains(&"v42".to_string()));
        g.insert("v43", vectors[42].clone());
        assert_eq!(g.search(&vectors[42], 1, 50, |_| true).keys, ["v43"]);
        assert_eq!(g.len(), 499);
        assert_eq!(g.tombstones(), 2);
        // A filter that rejects the best answers widens the search.
        let only_odd = |k: &str| k[1..].parse::<usize>().unwrap() % 2 == 1;
        let found = g.search(&vectors[10], 3, 10, only_odd);
        assert_eq!(found.keys.len(), 3);
        assert!(found.keys.iter().all(|k| only_odd(k)));

        for i in 0..300 {
            g.remove(&format!("v{i}"));
        }
        assert!(g.needs_rebuild());
        g.rebuild();
        assert_eq!((g.len(), g.tombstones()), (200, 0));
        assert_eq!(g.search(&vectors[400], 1, 50, |_| true).keys, ["v400"]);
    }

    #[test]
    fn survives_a_round_trip_to_bytes() {
        let vectors = random_vectors(300, 8, 5);
        let mut g = Hnsw::new(Params { metric: Metric::L2, m: 8, ef_construction: 64, ..Params::default() });
        for (i, v) in vectors.iter().enumerate() {
            g.insert(&format!("v{i}"), v.clone());
        }
        g.remove("v7");
        let mut buf = Vec::new();
        g.write(&mut buf);
        let back = Hnsw::read(&mut Reader::new(&buf)).unwrap();
        assert_eq!(back.params, g.params);
        assert_eq!((back.len(), back.tombstones()), (299, 1));
        for q in &vectors[..20] {
            assert_eq!(back.search(q, 5, 40, |_| true).keys, g.search(q, 5, 40, |_| true).keys);
        }
        assert!(Hnsw::read(&mut Reader::new(&buf[..buf.len() / 2])).is_none());
        assert!(Hnsw::read(&mut Reader::new(&[])).is_none());
    }

    #[test]
    fn scalar_quantization_keeps_distances_close_with_a_byte_per_number() {
        let v = vec![-1.0, 0.0, 0.5, 2.0];
        let q = Sq8::encode(&v);
        assert_eq!((q.codes[0], q.codes[3]), (0, 255));
        for (a, b) in q.decode().iter().zip(&v) {
            assert!((a - b).abs() <= 3.0 / 255.0 / 2.0 + 1e-6, "{a} vs {b}");
        }
        // Dot products on the codes match those of the decoded vectors.
        let w = Sq8::encode(&[0.3, -0.7, 0.1, 0.9]);
        let (dv, dw) = (q.decode(), w.decode());
        assert!((q.dot(&w) - dot(&dv, &dw)).abs() < 1e-4);
        let probe = [0.2f32, 0.4, -0.1, 0.8];
        assert!((q.dot_full(&probe, probe.iter().sum()) - dot(&probe, &dv)).abs() < 1e-4);
        // A constant vector has no spread: every code is 0.
        assert_eq!(Sq8::encode(&[0.5, 0.5]).decode(), [0.5, 0.5]);
        // Decoding then encoding again gives the same codes.
        let again = Sq8::encode(&q.decode());
        assert_eq!((again.codes, again.lo, again.step), (q.codes.clone(), q.lo, q.step));

        let vectors = random_vectors(2000, 32, 11);
        for metric in [Metric::Cosine, Metric::L2, Metric::Dot] {
            let full = Params { metric, ..Params::default() };
            let (mut f, mut s) = (Hnsw::new(full), Hnsw::new(Params { quant: Quant::Sq8, ..full }));
            for (i, v) in vectors.iter().enumerate() {
                f.insert(&format!("v{i}"), v.clone());
                s.insert(&format!("v{i}"), v.clone());
            }
            // Each vector takes one byte per number, and 12 for its range.
            assert_eq!(f.memory_bytes() - s.memory_bytes(), vectors.len() * (32 * 3 - 12));
            // Among 40 candidates, the true 10 closest are nearly all there.
            let queries = random_vectors(30, 32, 5);
            let mut hits = 0;
            for q in &queries {
                let truth = exact(&vectors, q, 10, metric);
                let found = s.search(q, 40, DEFAULT_EF_SEARCH, |_| true);
                hits += truth.iter().filter(|k| found.keys.contains(k)).count();
            }
            let r = hits as f64 / 300.0;
            assert!(r >= 0.9, "{metric:?}: {r}");
            assert!(s.same("v3", &vectors[3]) && !s.same("v3", &vectors[4]));
            let mut buf = Vec::new();
            s.write(&mut buf);
            let back = Hnsw::read(&mut Reader::new(&buf)).unwrap();
            assert_eq!(back.params.quant, Quant::Sq8);
            assert_eq!(back.memory_bytes(), s.memory_bytes());
            assert_eq!(back.search(&queries[0], 5, 40, |_| true).keys, s.search(&queries[0], 5, 40, |_| true).keys);
        }
    }

    #[test]
    fn a_filtered_search_fills_its_list_with_nodes_that_pass() {
        let vectors = random_vectors(3000, 16, 21);
        let mut g = Hnsw::new(Params::default());
        for (i, v) in vectors.iter().enumerate() {
            g.insert(&format!("v{i}"), v.clone());
        }
        // One vector in fifty passes.
        let rare = |k: &str| k[1..].parse::<usize>().unwrap() % 50 == 0;
        let kept: Vec<Vec<f32>> = vectors.iter().step_by(50).cloned().collect();
        let asked = std::cell::Cell::new(0);
        let mut hits = 0;
        for q in &random_vectors(20, 16, 8) {
            // The truth, named like the graph's keys.
            let truth: Vec<String> = exact(&kept, q, 10, Metric::Cosine)
                .iter()
                .map(|k| format!("v{}", k[1..].parse::<usize>().unwrap() * 50))
                .collect();
            let found = g.search_filtered(q, 10, 64, |k| {
                asked.set(asked.get() + 1);
                rare(k)
            });
            assert!(found.keys.iter().all(|k| rare(k)));
            assert_eq!(found.keys.len(), 10);
            hits += truth.iter().filter(|k| found.keys.contains(k)).count();
        }
        assert!(hits as f64 / 200.0 >= 0.9, "recall {}", hits as f64 / 200.0);
        // The condition is asked once per node at most.
        assert!(asked.get() <= 20 * 3000);
        // Deleted nodes never come back, and an impossible filter finds nothing.
        g.remove("v0");
        assert!(!g.search_filtered(&vectors[0], 5, 32, rare).keys.contains(&"v0".to_string()));
        assert!(g.search_filtered(&vectors[0], 5, 32, |_| false).keys.is_empty());
    }
}
