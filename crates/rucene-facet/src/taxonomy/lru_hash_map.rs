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

//! Port of `LRUHashMap.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// A simple LRU cache: a map of limited size which, when full, discards the
/// least recently used entry. Access order is that of Java's
/// `LinkedHashMap(16, 0.75f, true)`: `get` and `put` of a present key move it
/// to the most recent end; a `put` that leaves more than `max_size` entries
/// removes the eldest.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.LRUHashMap`.
#[derive(Debug, Clone)]
pub struct LRUHashMap<K: Eq + Hash + Clone, V> {
    max_size: usize,
    entries: HashMap<K, (V, u64)>,
    order: BTreeMap<u64, K>,
    next_stamp: u64,
}

impl<K: Eq + Hash + Clone, V> LRUHashMap<K, V> {
    /// Create a new hash map with a bounded size and with least recently used
    /// entries removed.
    ///
    /// Equivalent to `LRUHashMap(int)`.
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            entries: HashMap::with_capacity(16),
            order: BTreeMap::new(),
            next_stamp: 0,
        }
    }

    /// Return the max size.
    ///
    /// Equivalent to `getMaxSize()`.
    pub fn get_max_size(&self) -> usize {
        self.max_size
    }

    /// Allows changing the map's maximal number of elements which was defined
    /// at construction time. The change takes effect on the next `put`.
    ///
    /// Equivalent to `setMaxSize(int)`.
    pub fn set_max_size(&mut self, max_size: usize) {
        self.max_size = max_size;
    }

    fn touch(&mut self, key: &K) {
        let stamp = self.next_stamp;
        self.next_stamp += 1;
        if let Some(entry) = self.entries.get_mut(key) {
            self.order.remove(&entry.1);
            entry.1 = stamp;
            self.order.insert(stamp, key.clone());
        }
    }

    /// `LinkedHashMap.get(Object)` in access order.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        if !self.entries.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.entries.get(key).map(|(v, _)| v)
    }

    /// `LinkedHashMap.put(K, V)` followed by `removeEldestEntry`.
    pub fn put(&mut self, key: K, value: V) -> Option<V> {
        if let Some(entry) = self.entries.get_mut(&key) {
            let old = std::mem::replace(&mut entry.0, value);
            self.touch(&key);
            return Some(old);
        }
        let stamp = self.next_stamp;
        self.next_stamp += 1;
        self.entries.insert(key.clone(), (value, stamp));
        self.order.insert(stamp, key);
        // `removeEldestEntry(eldest)`: `size() > maxSize`.
        if self.entries.len() > self.max_size {
            self.remove_eldest();
        }
        None
    }

    fn remove_eldest(&mut self) -> Option<(K, V)> {
        let (&stamp, _) = self.order.iter().next()?;
        let key = self.order.remove(&stamp)?;
        self.entries.remove(&key).map(|(v, _)| (key, v))
    }

    /// `HashMap.remove(Object)`.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (value, stamp) = self.entries.remove(key)?;
        self.order.remove(&stamp);
        Some(value)
    }

    /// `HashMap.size()`.
    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// `HashMap.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `HashMap.clear()`.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// `HashMap.containsKey(Object)`, which does not count as an access.
    pub fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// The keys from eldest to most recently used, as `keySet()` iterates.
    pub fn keys(&self) -> impl Iterator<Item = &K> + '_ {
        self.order.values()
    }

    /// Removes up to `n` of the eldest entries, as an iterator over `keySet()`
    /// removing as it goes does.
    pub fn remove_eldest_n(&mut self, n: usize) {
        for _ in 0..n {
            if self.remove_eldest().is_none() {
                break;
            }
        }
    }
}