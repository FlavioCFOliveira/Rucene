/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `NameIntCacheLRU.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.writercache`.

use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::lru_hash_map::LRUHashMap;

/// The key of a [`NameIntCacheLRU`]: Java's `Object key(FacetLabel)`, the
/// label itself or, for [`NameHashIntCacheLRU`](super::name_hash_int_cache_lru::NameHashIntCacheLRU),
/// its `longHashCode()`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CacheKey {
    Label(FacetLabel),
    Hash(i64),
}

/// An LRU cache of mapping from name to int. Used to cache Ordinals of
/// category paths.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.NameIntCacheLRU`.
/// Java's `key(...)` overrides are the [`hashed`](Self::hashed) flag that
/// `NameHashIntCacheLRU` sets.
#[derive(Debug)]
pub struct NameIntCacheLRU {
    /// Java uses an access-ordered `LinkedHashMap` below `Integer.MAX_VALUE`
    /// and a plain `HashMap` at it; the plain map is never trimmed, so the
    /// access order it would lack is never observed.
    cache: LRUHashMap<CacheKey, i32>,
    /// For debug.
    pub(crate) n_misses: i64,
    /// For debug.
    pub(crate) n_hits: i64,
    max_cache_size: i32,
    hashed: bool,
}

impl NameIntCacheLRU {
    /// Equivalent to `NameIntCacheLRU(int)`.
    pub(crate) fn new(max_cache_size: i32) -> Self {
        Self::with_keys(max_cache_size, false)
    }

    pub(crate) fn with_keys(max_cache_size: i32, hashed: bool) -> Self {
        Self {
            // `createCache(int)`; the map itself is never bounded by size, as
            // Java's `LinkedHashMap` is not (`removeEldestEntry` is not
            // overridden): trimming is `makeRoomLRU`'s job.
            cache: LRUHashMap::new(usize::MAX),
            n_misses: 0,
            n_hits: 0,
            max_cache_size,
            hashed,
        }
    }

    /// Maximum number of cache entries before eviction.
    ///
    /// Equivalent to `getMaxSize()`.
    pub(crate) fn get_max_size(&self) -> i32 {
        self.max_cache_size
    }

    /// Number of entries currently in the cache.
    ///
    /// Equivalent to `getSize()`.
    pub(crate) fn get_size(&self) -> i32 {
        self.cache.size() as i32
    }

    /// Equivalent to `get(FacetLabel)`.
    pub(crate) fn get(&mut self, name: &FacetLabel) -> Option<i32> {
        let key = self.key(name);
        let res = self.cache.get(&key).copied();
        if res.is_none() {
            self.n_misses += 1;
        } else {
            self.n_hits += 1;
        }
        res
    }

    /// Subclasses can override this to provide caching by e.g. hash of the
    /// string.
    ///
    /// Equivalent to `key(FacetLabel)`.
    pub(crate) fn key(&self, name: &FacetLabel) -> CacheKey {
        if self.hashed {
            CacheKey::Hash(name.long_hash_code())
        } else {
            CacheKey::Label(name.clone())
        }
    }

    /// Equivalent to `key(FacetLabel, int)`.
    // Public for the port of the facet tests (Java package-private).
    pub fn key_prefix(&self, name: &FacetLabel, prefix_len: usize) -> CacheKey {
        if self.hashed {
            CacheKey::Hash(name.subpath(prefix_len).long_hash_code())
        } else {
            CacheKey::Label(name.subpath(prefix_len))
        }
    }

    /// Add a new value to cache. Return true if cache became full and some
    /// room need to be made.
    ///
    /// Equivalent to `put(FacetLabel, Integer)`.
    pub(crate) fn put(&mut self, name: &FacetLabel, val: i32) -> bool {
        let key = self.key(name);
        self.cache.put(key, val);
        self.is_cache_full()
    }

    /// Equivalent to `put(FacetLabel, int, Integer)`.
    // Public for the port of the facet tests (Java package-private).
    pub fn put_prefix(&mut self, name: &FacetLabel, prefix_len: usize, val: i32) -> bool {
        let key = self.key_prefix(name, prefix_len);
        self.cache.put(key, val);
        self.is_cache_full()
    }

    /// Equivalent to `isCacheFull()`.
    fn is_cache_full(&self) -> bool {
        self.cache.size() as i64 > self.max_cache_size as i64
    }

    /// Equivalent to `clear()`.
    pub(crate) fn clear(&mut self) {
        self.cache.clear();
    }

    /// Equivalent to `stats()`.
    // Public for the port of the facet tests (Java package-private).
    pub fn stats(&self) -> String {
        format!("#miss={} #hit={}", self.n_misses, self.n_hits)
    }

    /// If cache is full remove least recently used entries from cache. Return
    /// true if anything was removed, false otherwise.
    ///
    /// Equivalent to `makeRoomLRU()`.
    pub(crate) fn make_room_lru(&mut self) -> bool {
        if !self.is_cache_full() {
            return false;
        }
        let n = self.cache.size() as i64 - ((2i64 * self.max_cache_size as i64) / 3) as i32 as i64;
        if n <= 0 {
            return false;
        }
        self.cache.remove_eldest_n(n as usize);
        true
    }
}
