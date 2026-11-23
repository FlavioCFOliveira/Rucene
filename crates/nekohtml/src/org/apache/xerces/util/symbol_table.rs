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

//! Port of `SymbolTable.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::cell::RefCell;

use rucene::error::LuceneError;

use crate::java::{from_chars, string_hash_code, Exception, JResult};
use crate::org::apache::xerces::util::prime_number_sequence_generator::PrimeNumberSequenceGenerator;

/// This class is a symbol table implementation that guarantees that
/// strings used as identifiers are unique references.
///
/// Equivalent to `org.apache.xerces.util.SymbolTable`; a bucket's chain is
/// a `Vec` whose first element is Java's head entry.
#[derive(Debug)]
pub struct SymbolTable {
    state: RefCell<SymbolTableState>,
    /// `fCollisionThreshold`.
    collision_threshold: i32,
}

#[derive(Debug)]
struct SymbolTableState {
    buckets: Vec<Vec<Entry>>,
    table_size: i32,
    count: i32,
    threshold: i32,
    load_factor: f32,
    hash_multipliers: Option<Vec<i32>>,
}

/// Equivalent to the nested class `SymbolTable.Entry`.
#[derive(Debug, Clone)]
struct Entry {
    symbol: String,
    characters: Vec<u16>,
}

/// Default table size.
pub const TABLE_SIZE: i32 = 101;
/// Maximum hash collisions per bucket for a table with load factor == 1.
pub const MAX_HASH_COLLISIONS: i32 = 40;
/// `MULTIPLIERS_SIZE`.
pub const MULTIPLIERS_SIZE: i32 = 1 << 5;
/// `MULTIPLIERS_MASK`.
pub const MULTIPLIERS_MASK: i32 = MULTIPLIERS_SIZE - 1;

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    /// Equivalent to `new SymbolTable(int, float)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a negative capacity or a load factor
    /// that is not positive.
    pub fn with_capacity_and_load_factor(initial_capacity: i32, load_factor: f32) -> JResult<Self> {
        if initial_capacity < 0 {
            return Err(Exception::runtime(LuceneError::illegal_argument(format!(
                "Illegal Capacity: {initial_capacity}"
            ))));
        }
        if load_factor <= 0.0 || load_factor.is_nan() {
            return Err(Exception::runtime(LuceneError::illegal_argument(format!(
                "Illegal Load: {}",
                rucene::util::java_fmt::float_to_string(load_factor)
            ))));
        }
        let initial_capacity = if initial_capacity == 0 {
            1
        } else {
            initial_capacity
        };
        Ok(Self {
            state: RefCell::new(SymbolTableState {
                buckets: vec![Vec::new(); initial_capacity as usize],
                table_size: initial_capacity,
                count: 0,
                threshold: (initial_capacity as f32 * load_factor) as i32,
                load_factor,
                hash_multipliers: None,
            }),
            collision_threshold: (MAX_HASH_COLLISIONS as f32 * load_factor) as i32,
        })
    }

    /// Equivalent to `new SymbolTable(int)`.
    ///
    /// # Errors
    ///
    /// As [`with_capacity_and_load_factor`](Self::with_capacity_and_load_factor).
    pub fn with_capacity(initial_capacity: i32) -> JResult<Self> {
        Self::with_capacity_and_load_factor(initial_capacity, 0.75)
    }

    /// Equivalent to `new SymbolTable()`.
    pub fn new() -> Self {
        Self::with_capacity_and_load_factor(TABLE_SIZE, 0.75).expect("INVARIANT: valid defaults")
    }

    /// Adds the specified symbol to the symbol table and returns a reference
    /// to the unique symbol.
    ///
    /// Equivalent to `addSymbol(String)`.
    pub fn add_symbol(&self, symbol: &str) -> String {
        let chars: Vec<u16> = symbol.encode_utf16().collect();
        self.add_symbol_chars(&chars, 0, chars.len() as i32)
    }

    /// Adds the specified symbol to the symbol table and returns a reference
    /// to the unique symbol.
    ///
    /// Equivalent to `addSymbol(char[], int, int)` (and, over the string's
    /// UTF-16 units, `addSymbol(String)`).
    pub fn add_symbol_chars(&self, buffer: &[u16], offset: i32, length: i32) -> String {
        let chars = &buffer[offset as usize..(offset + length) as usize];
        let mut s = self.state.borrow_mut();
        let mut collision_count = 0;
        let mut bucket = (Self::hash_with(&s, chars) % s.table_size) as usize;
        for entry in &s.buckets[bucket] {
            if entry.characters == chars {
                return entry.symbol.clone();
            }
            collision_count += 1;
        }
        if s.count >= s.threshold {
            Self::rehash(&mut s);
            bucket = (Self::hash_with(&s, chars) % s.table_size) as usize;
        } else if collision_count >= self.collision_threshold {
            Self::rebalance(&mut s);
            bucket = (Self::hash_with(&s, chars) % s.table_size) as usize;
        }
        let entry = Entry {
            symbol: from_chars(chars),
            characters: chars.to_vec(),
        };
        let symbol = entry.symbol.clone();
        s.buckets[bucket].insert(0, entry);
        s.count += 1;
        symbol
    }

    fn hash_with(s: &SymbolTableState, chars: &[u16]) -> i32 {
        match &s.hash_multipliers {
            None => {
                let mut code = 0i32;
                for &c in chars {
                    code = code.wrapping_mul(31).wrapping_add(c as i32);
                }
                code & 0x7FFF_FFFF
            }
            Some(multipliers) => {
                let mut code = 0i32;
                for (i, &c) in chars.iter().enumerate() {
                    code = code
                        .wrapping_mul(multipliers[i & MULTIPLIERS_MASK as usize])
                        .wrapping_add(c as i32);
                }
                code & 0x7FFF_FFFF
            }
        }
    }

    /// Returns a hashcode value for the specified symbol.
    ///
    /// Equivalent to `hash(String)`.
    pub fn hash(&self, symbol: &str) -> i32 {
        let s = self.state.borrow();
        if s.hash_multipliers.is_none() {
            return string_hash_code(symbol) & 0x7FFF_FFFF;
        }
        let chars: Vec<u16> = symbol.encode_utf16().collect();
        Self::hash_with(&s, &chars)
    }

    /// Returns a hashcode value for the specified symbol information.
    ///
    /// Equivalent to `hash(char[], int, int)`.
    pub fn hash_chars(&self, buffer: &[u16], offset: i32, length: i32) -> i32 {
        Self::hash_with(
            &self.state.borrow(),
            &buffer[offset as usize..(offset + length) as usize],
        )
    }

    /// Equivalent to the protected `rehash()`.
    fn rehash(s: &mut SymbolTableState) {
        let new_capacity = s.buckets.len() * 2 + 1;
        Self::rehash_common(s, new_capacity);
    }

    /// Equivalent to the protected `rebalance()`.
    fn rebalance(s: &mut SymbolTableState) {
        let multipliers = s
            .hash_multipliers
            .get_or_insert_with(|| vec![0; MULTIPLIERS_SIZE as usize]);
        PrimeNumberSequenceGenerator::generate_sequence(multipliers);
        let capacity = s.buckets.len();
        Self::rehash_common(s, capacity);
    }

    fn rehash_common(s: &mut SymbolTableState, new_capacity: usize) {
        let old_table = std::mem::take(&mut s.buckets);
        let mut new_table: Vec<Vec<Entry>> = vec![Vec::new(); new_capacity];
        s.threshold = (new_capacity as f32 * s.load_factor) as i32;
        s.table_size = new_capacity as i32;
        for chain in old_table.into_iter().rev() {
            for e in chain {
                let index = (Self::hash_with(s, &e.characters) % new_capacity as i32) as usize;
                new_table[index].insert(0, e);
            }
        }
        s.buckets = new_table;
    }

    /// Returns true if the symbol table already contains the specified
    /// symbol.
    ///
    /// Equivalent to `containsSymbol(String)`.
    pub fn contains_symbol(&self, symbol: &str) -> bool {
        let chars: Vec<u16> = symbol.encode_utf16().collect();
        self.contains_symbol_chars(&chars, 0, chars.len() as i32)
    }

    /// Returns true if the symbol table already contains the specified
    /// symbol.
    ///
    /// Equivalent to `containsSymbol(char[], int, int)`.
    pub fn contains_symbol_chars(&self, buffer: &[u16], offset: i32, length: i32) -> bool {
        let chars = &buffer[offset as usize..(offset + length) as usize];
        let s = self.state.borrow();
        let bucket = (Self::hash_with(&s, chars) % s.table_size) as usize;
        s.buckets[bucket].iter().any(|e| e.characters == chars)
    }
}
