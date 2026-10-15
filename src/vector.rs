// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Vector similarity search, for embeddings produced by AI models.
//!
//! A vector is a JSON array of numbers stored in a document field, e.g.
//! `{"text": "...", "embedding": [0.12, -0.4, ...]}`. A search compares a
//! query vector with every stored vector of the documents matching a key
//! pattern and returns the closest ones. The search is exact (every
//! candidate is compared). A vector index keeps the vectors in memory as
//! `f32` so that the comparison runs without reading documents from disk.

use crate::jsonpath::{self, Seg};
use crate::util::glob_match;
use serde_json::Value as Json;
use std::collections::HashMap;

/// Longest vector accepted (OpenAI's largest embeddings have 3072 dimensions).
pub const MAX_DIMS: usize = 16_384;

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

fn norm(v: &[f32]) -> f32 {
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
            Metric::L2 => self.vector.iter().zip(v).map(|(a, b)| (a - b) * (a - b)).sum::<f32>().sqrt(),
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

fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Four independent accumulators let the compiler vectorise the loop.
    let mut acc = [0f32; 4];
    let chunks = a.len() / 4;
    for i in 0..chunks {
        for j in 0..4 {
            acc[j] += a[i * 4 + j] * b[i * 4 + j];
        }
    }
    let mut sum = acc.iter().sum::<f32>();
    for i in chunks * 4..a.len() {
        sum += a[i] * b[i];
    }
    sum
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

/// In-memory copy of one vector field of the documents matching a pattern.
pub struct VecIndex {
    pub pattern: String,
    pub field: String,
    path: Vec<Seg>,
    /// Fixed by the first vector indexed.
    pub dims: Option<usize>,
    vectors: HashMap<String, (Vec<f32>, f32)>,
    /// Documents whose vector had another length than `dims`.
    pub rejected: usize,
}

impl VecIndex {
    pub fn new(pattern: &str, field: &str, path: Vec<Seg>) -> VecIndex {
        VecIndex {
            pattern: pattern.to_string(),
            field: field.to_string(),
            path,
            dims: None,
            vectors: HashMap::new(),
            rejected: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    pub fn memory_bytes(&self) -> usize {
        self.vectors.iter().map(|(k, (v, _))| k.len() + v.len() * 4 + 64).sum()
    }

    pub fn clear(&mut self) {
        self.vectors.clear();
        self.dims = None;
        self.rejected = 0;
    }

    pub fn remove(&mut self, key: &str) {
        self.vectors.remove(key);
    }

    pub fn update(&mut self, key: &str, doc: Option<&Json>) {
        if !glob_match(&self.pattern, key) {
            return;
        }
        self.vectors.remove(key);
        let Some(v) = doc.and_then(|d| extract(d, &self.path)) else { return };
        match self.dims {
            Some(d) if d != v.len() => self.rejected += 1,
            _ => {
                self.dims = Some(v.len());
                let n = norm(&v);
                self.vectors.insert(key.to_string(), (v, n));
            }
        }
    }

    /// Scores every indexed vector whose key matches `pattern` and passes `alive`.
    pub fn scores(&self, pattern: &str, query: &Query, alive: impl Fn(&str) -> bool) -> Vec<(String, f32)> {
        self.vectors
            .iter()
            .filter(|(k, (v, _))| v.len() == query.dims() && glob_match(pattern, k) && alive(k))
            .map(|(k, (v, n))| (k.clone(), query.score(v, *n)))
            .collect()
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
