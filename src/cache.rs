// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! In-memory cache of decoded values.
//!
//! Normally only keys live in memory and every read goes to the data file.
//! With the cache on, recently read values are also kept in memory, up to a
//! byte budget, so that hot keys are served without touching the disk. The
//! file stays the source of truth: the cache only ever holds copies, keyed by
//! the version of the record they were read from.
//!
//! Keys matching a *pinned* pattern (`memory pin`) are kept apart: their
//! values stay in memory whether the cache is on or off, are never evicted,
//! and do not count against the cache budget.

use crate::util::glob_match;
use crate::value::Value;
use std::collections::HashMap;

struct Slot {
    value: Value,
    version: u64,
    size: usize,
    used: u64,
    pinned: bool,
}

#[derive(Default)]
pub struct ValueCache {
    map: HashMap<String, Slot>,
    /// Bytes of the evictable values only.
    bytes: usize,
    pinned_bytes: usize,
    pinned_count: usize,
    patterns: Vec<String>,
    limit: usize,
    clock: u64,
    pub hits: u64,
    pub misses: u64,
}

/// Rough memory cost of a cached value: payload, key and bookkeeping.
fn cost(key: &str, raw_size: usize) -> usize {
    raw_size + key.len() + 96
}

impl ValueCache {
    pub fn new(limit: usize) -> ValueCache {
        ValueCache { limit, ..Default::default() }
    }

    pub fn enabled(&self) -> bool {
        self.limit > 0
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Evictable values held.
    pub fn len(&self) -> usize {
        self.map.len() - self.pinned_count
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn pinned_bytes(&self) -> usize {
        self.pinned_bytes
    }

    pub fn pinned_len(&self) -> usize {
        self.pinned_count
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    pub fn is_pinned(&self, key: &str) -> bool {
        self.patterns.iter().any(|p| glob_match(p, key))
    }

    /// Adds a pinned pattern. Returns false if it was already there.
    pub fn pin(&mut self, pattern: &str) -> bool {
        if self.patterns.iter().any(|p| p == pattern) {
            return false;
        }
        self.patterns.push(pattern.to_string());
        true
    }

    /// Removes a pinned pattern and lets go of the values only it held.
    pub fn unpin(&mut self, pattern: &str) -> bool {
        let before = self.patterns.len();
        self.patterns.retain(|p| p != pattern);
        if self.patterns.len() == before {
            return false;
        }
        let released: Vec<String> =
            self.map.iter().filter(|(k, s)| s.pinned && !self.is_pinned(k)).map(|(k, _)| k.clone()).collect();
        for k in released {
            self.remove(&k);
        }
        true
    }

    /// Changes the budget; 0 turns the cache off and frees it. Pinned values stay.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        if limit == 0 {
            self.drop_evictable();
        } else {
            self.evict();
        }
    }

    /// The cached value of `key`, if it was read from this exact version.
    pub fn get(&mut self, key: &str, version: u64) -> Option<Value> {
        if !self.enabled() && self.pinned_count == 0 {
            return None;
        }
        self.clock += 1;
        match self.map.get_mut(key) {
            Some(slot) if slot.version == version => {
                slot.used = self.clock;
                self.hits += 1;
                Some(slot.value.clone())
            }
            Some(_) => {
                self.remove(key);
                self.misses += 1;
                None
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    pub fn put(&mut self, key: &str, version: u64, value: &Value, raw_size: usize) {
        let size = cost(key, raw_size);
        let pinned = self.is_pinned(key);
        // A single huge value would push everything else out: skip it.
        if !pinned && (!self.enabled() || size > self.limit / 8) {
            return;
        }
        self.clock += 1;
        self.remove(key);
        self.map.insert(key.to_string(), Slot { value: value.clone(), version, size, used: self.clock, pinned });
        if pinned {
            self.pinned_bytes += size;
            self.pinned_count += 1;
        } else {
            self.bytes += size;
            self.evict();
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Some(old) = self.map.remove(key) {
            if old.pinned {
                self.pinned_bytes -= old.size;
                self.pinned_count -= 1;
            } else {
                self.bytes -= old.size;
            }
        }
    }

    /// Drops every value, pinned ones included (the patterns stay).
    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
        self.pinned_bytes = 0;
        self.pinned_count = 0;
    }

    /// Drops the values that are not pinned, to give memory back.
    pub fn drop_evictable(&mut self) {
        self.map.retain(|_, s| s.pinned);
        self.bytes = 0;
    }

    /// Drops the least recently used values until a quarter of the budget is free.
    fn evict(&mut self) {
        if self.bytes <= self.limit {
            return;
        }
        let target = self.limit / 4 * 3;
        let mut by_age: Vec<(u64, String)> =
            self.map.iter().filter(|(_, s)| !s.pinned).map(|(k, s)| (s.used, k.clone())).collect();
        by_age.sort_unstable();
        for (_, key) in by_age {
            if self.bytes <= target {
                break;
            }
            self.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_values_stay_without_a_budget() {
        let mut c = ValueCache::new(0);
        let v = Value::Str("x".repeat(1000));
        c.put("a", 1, &v, 1000);
        assert!(c.get("a", 1).is_none());
        assert!(c.pin("conf:*"));
        assert!(!c.pin("conf:*"));
        c.put("conf:1", 1, &v, 1000);
        assert_eq!(c.get("conf:1", 1), Some(v.clone()));
        assert_eq!((c.len(), c.pinned_len(), c.bytes()), (0, 1, 0));
        // A newer version replaces the copy; an older read misses.
        c.put("conf:1", 2, &v, 1000);
        assert!(c.get("conf:1", 1).is_none());
        assert_eq!(c.pinned_len(), 0);
        c.put("conf:1", 2, &v, 1000);
        assert!(c.unpin("conf:*"));
        assert_eq!((c.pinned_len(), c.pinned_bytes()), (0, 0));
    }

    #[test]
    fn eviction_never_drops_pinned_values() {
        let mut c = ValueCache::new(10_000);
        c.pin("hot:*");
        let v = Value::Str("x".into());
        c.put("hot:1", 1, &v, 1000);
        for i in 0..100 {
            c.put(&format!("k{i}"), 1, &v, 1000);
        }
        assert!(c.bytes() <= 10_000);
        assert!(c.get("hot:1", 1).is_some());
        c.set_limit(0);
        assert!(c.get("hot:1", 1).is_some());
        assert_eq!(c.len(), 0);
    }
}
