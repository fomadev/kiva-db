// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Vector similarity search, for embeddings produced by AI models.
//!
//! A vector is a JSON array of numbers stored in a document field, e.g.
//! `{"text": "...", "embedding": [0.12, -0.4, ...]}`. A search compares a
//! query vector with every stored vector of the documents matching a key
//! pattern and returns the closest ones. Without an index, or with an exact
//! index, the search compares every candidate. An exact vector index keeps
//! the vectors in memory as `f32` so that the comparison runs without
//! reading documents from disk; an HNSW index ([`crate::hnsw`]) also links
//! them in a graph, so that a search compares only a few of them.

use crate::hnsw::{Hnsw, Params};
use crate::jsonpath::{self, Seg};
use crate::util::glob_match;
use serde_json::Value as Json;
use std::collections::HashMap;

/// Longest vector accepted (OpenAI's largest embeddings have 3072 dimensions).
pub const MAX_DIMS: usize = 16_384;

/// The answer to text sent where a vector is expected.
pub const NOT_EMBEDDINGS: &str = "KivaDB does not compute embeddings: it stores and compares the vectors      your application sends. Compute the vector of this text with your embedding model (OpenAI, Mistral,      Ollama, sentence-transformers...) and send it as an array of numbers, such as [0.12, -0.4, ...].";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Metric {
    /// Cosine similarity, from -1 to 1; higher is closer.
    Cosine,
    /// Dot product; higher is closer.
    Dot,
    /// Euclidean distance; lower is closer.
    L2,
}

impl Metric {
    pub fn parse(s: &str) -> Option<Metric> {
        Some(match s.to_ascii_lowercase().as_str() {
            "cosine" | "cos" => Metric::Cosine,
            "dot" | "inner" | "ip" => Metric::Dot,
            "l2" | "euclidean" | "euclid" => Metric::L2,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Metric::Cosine => "cosine",
            Metric::Dot => "dot",
            Metric::L2 => "l2",
        }
    }
}

pub(crate) fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Reads a vector from a JSON value: a non-empty array of finite numbers.
pub fn from_json(j: &Json) -> Option<Vec<f32>> {
    let a = j.as_array()?;
    if a.is_empty() || a.len() > MAX_DIMS {
        return None;
    }
    let mut v = Vec::with_capacity(a.len());
    for x in a {
        let f = x.as_f64()? as f32;
        if !f.is_finite() {
            return None;
        }
        v.push(f);
    }
    Some(v)
}

pub fn extract(doc: &Json, path: &[Seg]) -> Option<Vec<f32>> {
    jsonpath::get(doc, path).and_then(from_json)
}

/// A query vector with what the comparisons need, computed once.
pub struct Query {
    pub vector: Vec<f32>,
    pub metric: Metric,
    norm: f32,
}

impl Query {
    pub fn new(vector: Vec<f32>, metric: Metric) -> Query {
        let norm = norm(&vector);
        Query { vector, metric, norm }
    }

    pub fn dims(&self) -> usize {
        self.vector.len()
    }

    /// Score of `v` (whose length must equal the query's); `v_norm` is its norm.
    pub fn score(&self, v: &[f32], v_norm: f32) -> f32 {
        match self.metric {
            Metric::Dot => dot(&self.vector, v),
            Metric::Cosine => {
                let d = self.norm * v_norm;
                if d == 0.0 {
                    0.0
                } else {
                    dot(&self.vector, v) / d
                }
            }
            Metric::L2 => l2_squared(&self.vector, v).sqrt(),
        }
    }

    pub fn score_vec(&self, v: &[f32]) -> f32 {
        self.score(v, norm(v))
    }

    /// True when `a` is a better match than `b`.
    pub fn better(&self, a: f32, b: f32) -> bool {
        match self.metric {
            Metric::L2 => a < b,
            _ => a > b,
        }
    }
}

pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Eight independent accumulators over fixed-size chunks (no bounds checks)
    // let the compiler use SIMD instructions.
    let ((ca, ra), (cb, rb)) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    let tail: f32 = ra.iter().zip(rb).map(|(x, y)| x * y).sum();
    let mut acc = [0f32; 8];
    for (x, y) in ca.iter().zip(cb) {
        for j in 0..8 {
            acc[j] += x[j] * y[j];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// Squared Euclidean distance, vectorised like [`dot`].
pub(crate) fn l2_squared(a: &[f32], b: &[f32]) -> f32 {
    let ((ca, ra), (cb, rb)) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    let tail: f32 = ra.iter().zip(rb).map(|(x, y)| (x - y) * (x - y)).sum();
    let mut acc = [0f32; 8];
    for (x, y) in ca.iter().zip(cb) {
        for j in 0..8 {
            let d = x[j] - y[j];
            acc[j] += d * d;
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// Keeps the `k` best `(key, score)` pairs, best first.
pub fn top_k(mut scored: Vec<(String, f32)>, k: usize, query: &Query) -> Vec<(String, f32)> {
    let order = |a: &(String, f32), b: &(String, f32)| {
        let o = a.1.total_cmp(&b.1);
        let o = if query.metric == Metric::L2 { o } else { o.reverse() };
        o.then_with(|| a.0.cmp(&b.0))
    };
    if k < scored.len() {
        scored.select_nth_unstable_by(k, order);
        scored.truncate(k);
    }
    scored.sort_by(order);
    scored
}

/// How a vector index searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Every vector is compared: always the true closest ones.
    Exact,
    /// An HNSW graph: few vectors compared, very rarely a miss.
    Hnsw(Params),
}

enum Store {
    Exact(HashMap<String, (Vec<f32>, f32)>),
    Graph(Hnsw),
}

/// What one vector costs in an exact store.
fn exact_cost(key: &str, dims: usize) -> usize {
    key.len() + dims * 4 + 64
}

/// In-memory copy of one vector field of the documents matching a pattern.
pub struct VecIndex {
    pub pattern: String,
    pub field: String,
    path: Vec<Seg>,
    /// Fixed by the first vector indexed.
    pub dims: Option<usize>,
    store: Store,
    /// Documents whose vector had another length than `dims`.
    pub rejected: usize,
    /// Documents whose field holds something else than numbers, such as the
    /// text to embed: KivaDB stores vectors, it does not compute them.
    pub not_vectors: usize,
    /// Memory of an exact store, kept up to date (a graph counts its own).
    exact_bytes: usize,
}

impl VecIndex {
    pub fn new(pattern: &str, field: &str, path: Vec<Seg>) -> VecIndex {
        VecIndex::with_kind(pattern, field, path, Kind::Exact)
    }

    pub fn with_kind(pattern: &str, field: &str, path: Vec<Seg>, kind: Kind) -> VecIndex {
        VecIndex {
            pattern: pattern.to_string(),
            field: field.to_string(),
            path,
            dims: None,
            store: match kind {
                Kind::Exact => Store::Exact(HashMap::new()),
                Kind::Hnsw(p) => Store::Graph(Hnsw::new(p)),
            },
            rejected: 0,
            not_vectors: 0,
            exact_bytes: 0,
        }
    }

    pub fn kind(&self) -> Kind {
        match &self.store {
            Store::Exact(_) => Kind::Exact,
            Store::Graph(g) => Kind::Hnsw(g.params),
        }
    }

    pub fn graph(&self) -> Option<&Hnsw> {
        match &self.store {
            Store::Graph(g) => Some(g),
            Store::Exact(_) => None,
        }
    }

    pub fn graph_mut(&mut self) -> Option<&mut Hnsw> {
        match &mut self.store {
            Store::Graph(g) => Some(g),
            Store::Exact(_) => None,
        }
    }

    /// Installs a graph read back from disk, with what it knew of the vectors.
    pub fn restore(&mut self, graph: Hnsw, dims: Option<usize>, rejected: usize) {
        self.store = Store::Graph(graph);
        self.dims = dims;
        self.rejected = rejected;
    }

    pub fn len(&self) -> usize {
        match &self.store {
            Store::Exact(m) => m.len(),
            Store::Graph(g) => g.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn memory_bytes(&self) -> usize {
        match &self.store {
            Store::Exact(_) => self.exact_bytes,
            Store::Graph(g) => g.memory_bytes(),
        }
    }

    /// Memory that writing `doc` under `key` would add to the index: what
    /// a new vector costs, nothing for an exact store that replaces one.
    pub fn growth(&self, key: &str, doc: Option<&Json>) -> usize {
        if !glob_match(&self.pattern, key) {
            return 0;
        }
        let Some(v) = doc.and_then(|d| extract(d, &self.path)) else {
            return 0;
        };
        if self.dims.is_some_and(|d| d != v.len()) {
            return 0;
        }
        match &self.store {
            Store::Exact(m) if m.contains_key(key) => 0,
            Store::Exact(_) => exact_cost(key, v.len()),
            // A replaced vector leaves its node behind as a tombstone.
            Store::Graph(g) if g.get(key).is_some_and(|(old, _)| old == v.as_slice()) => 0,
            Store::Graph(g) => g.node_bytes(key.len(), v.len(), 1),
        }
    }

    /// The keys of every indexed vector.
    pub fn keys(&self) -> Vec<String> {
        match &self.store {
            Store::Exact(m) => m.keys().cloned().collect(),
            Store::Graph(g) => g.iter().map(|(k, _, _)| k.to_string()).collect(),
        }
    }

    /// The vector indexed for `key`, with its norm.
    pub fn get(&self, key: &str) -> Option<(&[f32], f32)> {
        match &self.store {
            Store::Exact(m) => m.get(key).map(|(v, n)| (v.as_slice(), *n)),
            Store::Graph(g) => g.get(key),
        }
    }

    pub fn clear(&mut self) {
        match &mut self.store {
            Store::Exact(m) => m.clear(),
            Store::Graph(g) => g.clear(),
        }
        self.dims = None;
        self.rejected = 0;
        self.not_vectors = 0;
        self.exact_bytes = 0;
    }

    pub fn remove(&mut self, key: &str) {
        match &mut self.store {
            Store::Exact(m) => {
                if let Some((v, _)) = m.remove(key) {
                    self.exact_bytes -= exact_cost(key, v.len());
                }
            }
            Store::Graph(g) => g.remove(key),
        }
    }

    pub fn update(&mut self, key: &str, doc: Option<&Json>) {
        if !glob_match(&self.pattern, key) {
            return;
        }
        let field = doc.and_then(|d| jsonpath::get(d, &self.path));
        let Some(v) = field.and_then(from_json) else {
            if field.is_some() {
                self.not_vectors += 1;
            }
            self.remove(key);
            return;
        };
        if self.dims.is_some_and(|d| d != v.len()) {
            self.remove(key);
            self.rejected += 1;
            return;
        }
        self.dims = Some(v.len());
        match &mut self.store {
            Store::Exact(m) => {
                let n = norm(&v);
                self.exact_bytes += exact_cost(key, v.len());
                if let Some((old, _)) = m.insert(key.to_string(), (v, n)) {
                    self.exact_bytes -= exact_cost(key, old.len());
                }
            }
            // A document changed elsewhere than in its vector keeps its node:
            // replacing it would only leave a tombstone behind.
            Store::Graph(g) => {
                if g.get(key).is_none_or(|(old, _)| old != v.as_slice()) {
                    g.insert(key, v);
                }
            }
        }
    }

    /// Scores every indexed vector whose key matches `pattern` and passes `alive`.
    pub fn scores(&self, pattern: &str, query: &Query, alive: impl Fn(&str) -> bool) -> Vec<(String, f32)> {
        let keep = |k: &str, v: &[f32]| v.len() == query.dims() && glob_match(pattern, k) && alive(k);
        match &self.store {
            Store::Exact(m) => {
                m.iter().filter(|(k, (v, _))| keep(k, v)).map(|(k, (v, n))| (k.clone(), query.score(v, *n))).collect()
            }
            Store::Graph(g) => {
                g.iter().filter(|(k, v, _)| keep(k, v)).map(|(k, v, n)| (k.to_string(), query.score(v, n))).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics() {
        let q = Query::new(vec![1.0, 0.0], Metric::Cosine);
        assert!((q.score_vec(&[2.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(q.score_vec(&[0.0, 3.0]).abs() < 1e-6);
        assert!((q.score_vec(&[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        let d = Query::new(vec![1.0, 2.0, 3.0, 4.0, 5.0], Metric::Dot);
        assert_eq!(d.score_vec(&[1.0, 1.0, 1.0, 1.0, 1.0]), 15.0);
        let l = Query::new(vec![0.0, 0.0], Metric::L2);
        assert_eq!(l.score_vec(&[3.0, 4.0]), 5.0);
        assert!(l.better(1.0, 2.0) && q.better(0.9, 0.1));
        assert_eq!(Query::new(vec![0.0, 0.0], Metric::Cosine).score_vec(&[1.0, 1.0]), 0.0);
    }

    #[test]
    fn best_first() {
        let q = Query::new(vec![1.0], Metric::L2);
        let scored = vec![("a".into(), 3.0), ("b".into(), 1.0), ("c".into(), 2.0), ("d".into(), 0.5)];
        let best: Vec<String> = top_k(scored, 2, &q).into_iter().map(|(k, _)| k).collect();
        assert_eq!(best, ["d", "b"]);
    }

    #[test]
    fn parsing() {
        assert_eq!(from_json(&serde_json::json!([1, 2.5, -3])), Some(vec![1.0, 2.5, -3.0]));
        assert_eq!(from_json(&serde_json::json!([])), None);
        assert_eq!(from_json(&serde_json::json!([1, "x"])), None);
        assert_eq!(from_json(&serde_json::json!({"a": 1})), None);
    }
}
