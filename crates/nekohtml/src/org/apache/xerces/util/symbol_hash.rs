/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `SymbolHash.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use crate::java::{string_hash_code, Object};
use crate::org::apache::xerces::util::prime_number_sequence_generator::PrimeNumberSequenceGenerator;

/// This class is an unsynchronized hash table primary used for String to
/// Object mapping.
///
/// Equivalent to `org.apache.xerces.util.SymbolHash`, over `String` keys
/// (its only use in the ported closure); a bucket's chain is a `Vec` whose
/// first element is Java's head entry.
#[derive(Debug, Clone)]
pub struct SymbolHash {
    table_size: i32,
    buckets: Vec<Vec<Entry>>,
    num: i32,
    hash_multipliers: Option<Vec<i32>>,
}

/// Equivalent to the nested class `SymbolHash.Entry`.
#[derive(Debug, Clone)]
struct Entry {
    key: String,
    value: Option<Object>,
}

/// Default table size.
pub const TABLE_SIZE: i32 = 101;
/// Maximum hash collisions per bucket.
pub const MAX_HASH_COLLISIONS: i32 = 40;
/// `MULTIPLIERS_SIZE`.
pub const MULTIPLIERS_SIZE: i32 = 1 << 5;
/// `MULTIPLIERS_MASK`.
pub const MULTIPLIERS_MASK: i32 = MULTIPLIERS_SIZE - 1;

impl Default for SymbolHash {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolHash {
    /// Constructs a key table with the default size.
    ///
    /// Equivalent to `new SymbolHash()`.
    pub fn new() -> Self {
        Self::with_size(TABLE_SIZE)
    }

    /// Constructs a key table with a given size.
    ///
    /// Equivalent to `new SymbolHash(int)`.
    pub fn with_size(size: i32) -> Self {
        Self {
            table_size: size,
            buckets: vec![Vec::new(); size.max(0) as usize],
            num: 0,
            hash_multipliers: None,
        }
    }

    /// Adds the key/value mapping to the key table.
    ///
    /// Equivalent to `put(Object, Object)`.
    pub fn put(&mut self, key: &str, value: Option<Object>) {
        let mut collision_count = 0;
        let hash = self.hash(key);
        let mut bucket = (hash % self.table_size) as usize;
        for entry in self.buckets[bucket].iter_mut() {
            if entry.key == key {
                entry.value = value;
                return;
            }
            collision_count += 1;
        }
        if self.num >= self.table_size {
            self.rehash();
            bucket = (hash % self.table_size) as usize;
        } else if collision_count >= MAX_HASH_COLLISIONS {
            self.rebalance();
            bucket = (self.hash(key) % self.table_size) as usize;
        }
        self.buckets[bucket].insert(
            0,
            Entry {
                key: key.to_string(),
                value,
            },
        );
        self.num += 1;
    }

    /// Get the value associated with the given key.
    ///
    /// Equivalent to `get(Object)`.
    pub fn get(&self, key: &str) -> Option<Object> {
        let bucket = (self.hash(key) % self.table_size) as usize;
        self.buckets[bucket]
            .iter()
            .find(|e| e.key == key)
            .and_then(|e| e.value.clone())
    }

    /// Get the number of key/value pairs stored in this table.
    ///
    /// Equivalent to `getLength()`.
    pub fn get_length(&self) -> i32 {
        self.num
    }

    /// Add all values to the given array, starting at `from`.
    ///
    /// Equivalent to `getValues(Object[], int)`.
    pub fn get_values(&self, elements: &mut [Option<Object>], from: i32) -> i32 {
        let mut j = 0;
        for chain in &self.buckets {
            if j >= self.num {
                break;
            }
            for entry in chain {
                elements[(from + j) as usize] = entry.value.clone();
                j += 1;
            }
        }
        self.num
    }

    /// Return key/value pairs of all entries in the map.
    ///
    /// Equivalent to `getEntries()`.
    pub fn get_entries(&self) -> Vec<(String, Option<Object>)> {
        let mut entries = Vec::with_capacity(self.num as usize);
        for chain in &self.buckets {
            if entries.len() as i32 >= self.num {
                break;
            }
            for entry in chain {
                entries.push((entry.key.clone(), entry.value.clone()));
            }
        }
        entries
    }

    /// Make a clone of this object.
    ///
    /// Equivalent to `makeClone()`.
    pub fn make_clone(&self) -> SymbolHash {
        self.clone()
    }

    /// Remove all key/value association.
    ///
    /// Equivalent to `clear()`.
    pub fn clear(&mut self) {
        for chain in self.buckets.iter_mut() {
            chain.clear();
        }
        self.num = 0;
        self.hash_multipliers = None;
    }

    /// Returns a hashcode value for the specified key.
    ///
    /// Equivalent to the protected `hash(Object)`.
    fn hash(&self, key: &str) -> i32 {
        match &self.hash_multipliers {
            None => string_hash_code(key) & 0x7FFF_FFFF,
            Some(multipliers) => {
                let mut code = 0i32;
                for (i, c) in key.encode_utf16().enumerate() {
                    code = code
                        .wrapping_mul(multipliers[i & MULTIPLIERS_MASK as usize])
                        .wrapping_add(c as i32);
                }
                code & 0x7FFF_FFFF
            }
        }
    }

    /// Equivalent to the protected `rehash()`.
    fn rehash(&mut self) {
        let capacity = (self.buckets.len() << 1) + 1;
        self.rehash_common(capacity);
    }

    /// Equivalent to the protected `rebalance()`.
    fn rebalance(&mut self) {
        let multipliers = self
            .hash_multipliers
            .get_or_insert_with(|| vec![0; MULTIPLIERS_SIZE as usize]);
        PrimeNumberSequenceGenerator::generate_sequence(multipliers);
        let capacity = self.buckets.len();
        self.rehash_common(capacity);
    }

    fn rehash_common(&mut self, new_capacity: usize) {
        let old_table = std::mem::take(&mut self.buckets);
        self.buckets = vec![Vec::new(); new_capacity];
        self.table_size = new_capacity as i32;
        for chain in old_table.into_iter().rev() {
            for e in chain {
                let index = (self.hash(&e.key) % new_capacity as i32) as usize;
                self.buckets[index].insert(0, e);
            }
        }
    }
}
