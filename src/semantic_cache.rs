// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Semantic cache: answers kept for questions, found again for questions
//! that mean the same thing.
//!
//! An application that calls a language model (or any slow computation)
//! first asks the cache with the embedding of the question. If a question
//! stored earlier is close enough (cosine similarity at least the threshold,
//! 0.98 by default), its answer comes back at once; otherwise the application
//! computes the answer and stores it. A question repeated word for word is
//! also found by its text alone, without an embedding.
//!
//! KivaDB does not compute embeddings: the application gives the vector its
//! model produced. Each cache lives in memory, per workspace, with an HNSW
//! graph ([`crate::hnsw`]) over its questions. Entries expire after their
//! TTL, and an entry that `depends` on a key pattern is dropped as soon as a
//! key matching it is written or deleted. The cache is not saved: after a
//! restart it fills again.

use crate::hnsw::{Hnsw, Params};
use crate::util::{glob_match, now_secs};
use crate::vector::{Metric, Query};
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::sync::Mutex;

/// Settings of `[semantic]`.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Cache the results of `get similar` by themselves.
    pub enabled: bool,
    pub threshold: f32,
    /// Seconds an entry lives; 0 = until invalidated or evicted.
    pub ttl: i64,
    /// Entries per cache; the oldest go first beyond it.
    pub max_entries: usize,
}

impl Settings {
    pub fn of(cfg: &crate::config::Config) -> Settings {
        Settings {
            enabled: cfg.get_bool("semantic", "cache.enabled"),
            threshold: cfg.get("semantic", "cache.threshold").parse().ok().filter(|t: &f32| *t > 0.0).unwrap_or(0.98),
            ttl: cfg.get_u64("semantic", "cache.ttl") as i64,
            max_entries: (cfg.get_u64("semantic", "cache.max_entries") as usize).max(1),
        }
    }
}

/// Text compared for exact repeats: trimmed, lower case, spaces collapsed.
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

struct Entry {
    answer: Json,
    text: Option<String>,
    has_vector: bool,
    created: i64,
    /// Unix time after which it is gone; 0 = never.
    expires_at: i64,
    depends: Vec<String>,
    hits: u64,
}

impl Entry {
    fn alive(&self, now: i64) -> bool {
        self.expires_at == 0 || now < self.expires_at
    }
}

/// One named cache of one workspace.
struct Space {
    graph: Hnsw,
    dims: Option<usize>,
    entries: HashMap<String, Entry>,
    /// Normalized question text -> entry id.
    texts: HashMap<String, String>,
    next: u64,
    hits: u64,
    misses: u64,
}

impl Space {
    fn new() -> Space {
        let params = Params { metric: Metric::Cosine, m: 16, ef_construction: 100 };
        Space {
            graph: Hnsw::new(params),
            dims: None,
            entries: HashMap::new(),
            texts: HashMap::new(),
            next: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn remove(&mut self, id: &str) {
        if let Some(e) = self.entries.remove(id) {
            if let Some(t) = e.text {
                if self.texts.get(&t).is_some_and(|x| x == id) {
                    self.texts.remove(&t);
                }
            }
            if e.has_vector {
                self.graph.remove(id);
            }
        }
        if self.graph.needs_rebuild() {
            self.graph.rebuild();
        }
    }

    fn purge(&mut self, now: i64) {
        let dead: Vec<String> = self.entries.iter().filter(|(_, e)| !e.alive(now)).map(|(k, _)| k.clone()).collect();
        for id in dead {
            self.remove(&id);
        }
    }

    /// The closest live question to `vector`, with its similarity.
    fn nearest(&self, vector: &[f32], now: i64) -> Option<(String, f32)> {
        let found = self.graph.search(vector, 1, 64, |id| self.entries.get(id).is_some_and(|e| e.alive(now)));
        let id = found.keys.into_iter().next()?;
        let (v, n) = self.graph.get(&id)?;
        Some((id, Query::new(vector.to_vec(), Metric::Cosine).score(v, n)))
    }
}

/// What a lookup found.
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    Hit {
        id: String,
        similarity: f32,
        answer: Json,
    },
    /// Nothing close enough; `best` is the closest similarity seen, if any.
    Miss {
        best: Option<f32>,
    },
}

/// A question: its text, its embedding, or both.
#[derive(Debug, Clone, Default)]
pub struct Question {
    pub text: Option<String>,
    pub vector: Option<Vec<f32>>,
}

/// Every semantic cache of an instance.
#[derive(Default)]
pub struct SemanticCache {
    spaces: Mutex<HashMap<(String, String), Space>>,
}

fn invalid(msg: impl Into<String>) -> crate::error::KivaError {
    crate::error::KivaError::Invalid(msg.into())
}

impl SemanticCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Space>> {
        self.spaces.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Looks for the answer of a question in cache `name` of `workspace`.
    pub fn lookup(&self, workspace: &str, name: &str, q: &Question, threshold: f32) -> crate::error::Result<Lookup> {
        if q.text.is_none() && q.vector.is_none() {
            return Err(invalid("give the question's text, its vector, or both"));
        }
        let now = now_secs();
        let mut spaces = self.lock();
        let Some(space) = spaces.get_mut(&(workspace.to_string(), name.to_string())) else {
            return Ok(Lookup::Miss { best: None });
        };
        // A question asked again word for word needs no vector.
        let exact = q.text.as_deref().map(normalize).and_then(|t| space.texts.get(&t).cloned());
        let mut found = exact.filter(|id| space.entries.get(id).is_some_and(|e| e.alive(now))).map(|id| (id, 1.0f32));
        let mut best = found.as_ref().map(|(_, s)| *s);
        if found.is_none() {
            if let Some(v) = &q.vector {
                if let Some(d) = space.dims.filter(|d| *d != v.len()) {
                    return Err(invalid(format!(
                        "this cache holds vectors of {d} dimensions, the question has {}",
                        v.len()
                    )));
                }
                if let Some((id, s)) = space.nearest(v, now) {
                    best = Some(s);
                    if s >= threshold {
                        found = Some((id, s));
                    }
                }
            }
        }
        let Some((id, similarity)) = found else {
            space.misses += 1;
            return Ok(Lookup::Miss { best });
        };
        space.hits += 1;
        let entry = space.entries.get_mut(&id).expect("found among the entries");
        entry.hits += 1;
        Ok(Lookup::Hit { id: format!("{name}:{id}"), similarity, answer: entry.answer.clone() })
    }

    /// Stores the answer of a question; returns the entry's id. A question
    /// already cached (same text, or a vector at similarity 1) is replaced.
    #[allow(clippy::too_many_arguments)]
    pub fn store(
        &self,
        workspace: &str,
        name: &str,
        q: &Question,
        answer: Json,
        ttl: i64,
        depends: Vec<String>,
        max_entries: usize,
    ) -> crate::error::Result<String> {
        if q.text.is_none() && q.vector.is_none() {
            return Err(invalid("give the question's text, its vector, or both"));
        }
        if let Some(v) = &q.vector {
            if v.is_empty() || v.len() > crate::vector::MAX_DIMS || v.iter().all(|x| *x == 0.0) {
                return Err(invalid("the question's vector must have numbers, not all zero"));
            }
        }
        let now = now_secs();
        let mut spaces = self.lock();
        let space = spaces.entry((workspace.to_string(), name.to_string())).or_insert_with(Space::new);
        if let (Some(v), Some(d)) = (&q.vector, space.dims) {
            if v.len() != d {
                return Err(invalid(format!(
                    "this cache holds vectors of {d} dimensions, the question has {}",
                    v.len()
                )));
            }
        }
        space.purge(now);
        let text = q.text.as_deref().map(normalize);
        // The same question again replaces its entry.
        let same = text.as_ref().and_then(|t| space.texts.get(t).cloned()).or_else(|| {
            q.vector.as_ref().and_then(|v| space.nearest(v, now)).filter(|(_, s)| *s >= 0.9999).map(|x| x.0)
        });
        if let Some(old) = same {
            space.remove(&old);
        }
        while space.entries.len() >= max_entries {
            let oldest = space.entries.iter().min_by_key(|(_, e)| e.created).map(|(k, _)| k.clone());
            match oldest {
                Some(id) => space.remove(&id),
                None => break,
            }
        }
        space.next += 1;
        let id = space.next.to_string();
        if let Some(v) = &q.vector {
            space.dims = Some(v.len());
            space.graph.insert(&id, v.clone());
        }
        if let Some(t) = &text {
            space.texts.insert(t.clone(), id.clone());
        }
        space.entries.insert(
            id.clone(),
            Entry {
                answer,
                text,
                has_vector: q.vector.is_some(),
                created: now,
                expires_at: if ttl > 0 { now + ttl } else { 0 },
                depends,
                hits: 0,
            },
        );
        Ok(format!("{name}:{id}"))
    }

    /// Drops the entries of `workspace` that depend on a pattern `key` matches.
    pub fn invalidate(&self, workspace: &str, key: &str) {
        let mut spaces = self.lock();
        for ((ws, _), space) in spaces.iter_mut() {
            if ws != workspace {
                continue;
            }
            let stale: Vec<String> = space
                .entries
                .iter()
                .filter(|(_, e)| e.depends.iter().any(|p| glob_match(p, key)))
                .map(|(k, _)| k.clone())
                .collect();
            for id in stale {
                space.remove(&id);
            }
        }
    }

    /// Empties one cache, or every cache of the workspace.
    pub fn clear(&self, workspace: &str, name: Option<&str>) -> usize {
        let mut spaces = self.lock();
        let keys: Vec<(String, String)> =
            spaces.keys().filter(|(ws, n)| ws == workspace && name.is_none_or(|x| x == n)).cloned().collect();
        let mut n = 0;
        for k in keys {
            n += spaces.remove(&k).map(|s| s.entries.len()).unwrap_or(0);
        }
        n
    }

    /// `(name, entries, hits, misses)` of every cache of the workspace.
    pub fn stats(&self, workspace: &str) -> Vec<(String, usize, u64, u64)> {
        let now = now_secs();
        let mut spaces = self.lock();
        let mut out: Vec<(String, usize, u64, u64)> = spaces
            .iter_mut()
            .filter(|((ws, _), _)| ws == workspace)
            .map(|((_, name), s)| {
                s.purge(now);
                (name.clone(), s.entries.len(), s.hits, s.misses)
            })
            .collect();
        out.sort();
        out
    }
}

/// The JSON a structured request answers for a lookup.
pub fn lookup_json(l: &Lookup) -> Json {
    match l {
        Lookup::Hit { id, similarity, answer } => {
            json!({ "hit": true, "id": id, "similarity": similarity, "answer": answer })
        }
        Lookup::Miss { best } => json!({ "hit": false, "best": best }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: Option<&str>, vector: Option<Vec<f32>>) -> Question {
        Question { text: text.map(String::from), vector }
    }

    #[test]
    fn close_questions_hit_and_far_ones_miss() {
        let c = SemanticCache::default();
        c.store("default", "faq", &q(None, Some(vec![1.0, 0.0, 0.0])), json!("Kinshasa"), 0, vec![], 100).unwrap();
        // cos(angle) = 0.995: the same question, worded slightly differently.
        let near = c.lookup("default", "faq", &q(None, Some(vec![1.0, 0.1, 0.0])), 0.98).unwrap();
        assert!(matches!(&near, Lookup::Hit { answer, similarity, .. } if answer == "Kinshasa" && *similarity > 0.99));
        let far = c.lookup("default", "faq", &q(None, Some(vec![0.6, 0.8, 0.0])), 0.98).unwrap();
        assert!(matches!(far, Lookup::Miss { best: Some(b) } if (b - 0.6).abs() < 1e-4));
        // Another workspace, another cache name: nothing.
        assert_eq!(
            c.lookup("other", "faq", &q(None, Some(vec![1.0, 0.0, 0.0])), 0.98).unwrap(),
            Lookup::Miss { best: None }
        );
        assert!(c.lookup("default", "faq", &q(None, Some(vec![1.0, 0.0])), 0.98).is_err());
        assert_eq!(c.stats("default"), [("faq".to_string(), 1, 1, 1)]);
    }

    #[test]
    fn exact_text_needs_no_vector() {
        let c = SemanticCache::default();
        c.store("w", "llm", &q(Some("What is KivaDB?"), None), json!("A database."), 0, vec![], 100).unwrap();
        let hit = c.lookup("w", "llm", &q(Some("  what is   KIVADB? "), None), 0.98).unwrap();
        assert!(matches!(hit, Lookup::Hit { similarity, .. } if similarity == 1.0));
        assert!(matches!(c.lookup("w", "llm", &q(Some("What is Redis?"), None), 0.98).unwrap(), Lookup::Miss { .. }));
        assert_eq!(normalize(" A  b\tC "), "a b c");
    }

    #[test]
    fn writes_invalidate_and_entries_expire_or_make_room() {
        let c = SemanticCache::default();
        let deps = vec!["doc:*".to_string()];
        c.store("w", "rag", &q(Some("q1"), None), json!(1), 0, deps, 100).unwrap();
        c.store("w", "rag", &q(Some("q2"), None), json!(2), 0, vec![], 100).unwrap();
        c.invalidate("w", "user:1");
        c.invalidate("other", "doc:1");
        assert_eq!(c.stats("w")[0].1, 2);
        c.invalidate("w", "doc:7");
        assert!(matches!(c.lookup("w", "rag", &q(Some("q1"), None), 0.98).unwrap(), Lookup::Miss { .. }));
        assert!(matches!(c.lookup("w", "rag", &q(Some("q2"), None), 0.98).unwrap(), Lookup::Hit { .. }));

        // An expired entry is gone; the oldest entries make room.
        c.store("w", "short", &q(Some("x"), None), json!(0), 0, vec![], 100).unwrap();
        {
            let mut s = c.lock();
            s.get_mut(&("w".into(), "short".into())).unwrap().entries.values_mut().for_each(|e| e.expires_at = 1);
        }
        assert!(matches!(c.lookup("w", "short", &q(Some("x"), None), 0.98).unwrap(), Lookup::Miss { .. }));
        for i in 0..5 {
            c.store("w", "small", &q(Some(&format!("q{i}")), None), json!(i), 0, vec![], 3).unwrap();
        }
        assert_eq!(c.stats("w").iter().find(|s| s.0 == "small").unwrap().1, 3);
        assert_eq!(c.clear("w", Some("small")), 3);
        assert_eq!(c.clear("w", None), 1, "q2 is left; the expired entry was purged");
    }

    #[test]
    fn the_same_question_replaces_its_answer() {
        let c = SemanticCache::default();
        c.store("w", "c", &q(Some("q"), Some(vec![0.0, 1.0])), json!("old"), 0, vec![], 100).unwrap();
        c.store("w", "c", &q(Some("Q"), Some(vec![0.0, 2.0])), json!("new"), 0, vec![], 100).unwrap();
        assert_eq!(c.stats("w")[0].1, 1);
        let hit = c.lookup("w", "c", &q(None, Some(vec![0.0, 1.0])), 0.98).unwrap();
        assert!(matches!(hit, Lookup::Hit { answer, .. } if answer == "new"));
    }
}
