// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Optional in-memory cache of decoded values.
//!
//! Normally only keys live in memory and every read goes to the data file.
//! With the cache on, recently read values are also kept in memory, up to a
//! byte budget, so that hot keys are served without touching the disk. The
//! file stays the source of truth: the cache only ever holds copies, keyed by
//! the version of the record they were read from.

use crate::value::Value;
use std::collections::HashMap;

struct Slot {
    value: Value,
    version: u64,
    size: usize,
    used: u64,
}

#[derive(Default)]
pub struct ValueCache {
    map: HashMap<String, Slot>,
    bytes: usize,
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

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Changes the budget; 0 turns the cache off and frees it.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        if limit == 0 {
            self.clear();
        } else {
            self.evict();
        }
    }

    /// The cached value of `key`, if it was read from this exact version.
    pub fn get(&mut self, key: &str, version: u64) -> Option<Value> {
        if !self.enabled() {
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
        // A single huge value would push everything else out: skip it.
        if !self.enabled() || size > self.limit / 8 {
            return;
        }
        self.clock += 1;
        if let Some(old) =
            self.map.insert(key.to_string(), Slot { value: value.clone(), version, size, used: self.clock })
        {
            self.bytes -= old.size;
        }
        self.bytes += size;
        self.evict();
    }

    pub fn remove(&mut self, key: &str) {
        if let Some(old) = self.map.remove(key) {
            self.bytes -= old.size;
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    /// Drops the least recently used values until a quarter of the budget is free.
    fn evict(&mut self) {
        if self.bytes <= self.limit {
            return;
        }
        let target = self.limit / 4 * 3;
        let mut by_age: Vec<(u64, String)> = self.map.iter().map(|(k, s)| (s.used, k.clone())).collect();
        by_age.sort_unstable();
        for (_, key) in by_age {
            if self.bytes <= target {
                break;
            }
            self.remove(&key);
        }
    }
}
