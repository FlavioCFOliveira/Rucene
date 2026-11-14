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

//! The deduplicating store of flag sets, ported from
//! `org.apache.lucene.analysis.hunspell.FlagEnumerator`.

use std::collections::HashMap;

use rucene::error::{LuceneError, Result};

use super::dictionary::FLAG_UNSET;

/// A structure similar to `BytesRefHash`, but specialized for sorted char
/// sequences used for Hunspell flags. It deduplicates flag sequences, gives
/// them unique ids, stores the sequences in a contiguous `char[]` (via
/// [`finish`](Self::finish)) and allows to query presence of the flags later
/// via [`Lookup::has_flag`].
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.FlagEnumerator`.
#[derive(Debug)]
pub(crate) struct FlagEnumerator {
    builder: Vec<u16>,
    indices: HashMap<Vec<u16>, i32>,
}

impl FlagEnumerator {
    /// Equivalent to `FlagEnumerator()`: no flags have ord 0.
    pub(crate) fn new() -> Self {
        let mut enumerator = Self {
            builder: Vec::new(),
            indices: HashMap::new(),
        };
        enumerator
            .add(&mut [])
            .expect("INVARIANT: an empty flag set fits");
        enumerator
    }

    /// Sorts `chars` in place and returns the id of that flag set, adding it
    /// when new.
    ///
    /// Equivalent to `add(char[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Too many flags: <flags>")` for more than
    /// 65535 flags.
    pub(crate) fn add(&mut self, chars: &mut [u16]) -> Result<i32> {
        chars.sort_unstable();
        let key = chars.to_vec();
        if key.len() > usize::from(u16::MAX) {
            return Err(LuceneError::illegal_argument(format!(
                "Too many flags: {}",
                String::from_utf16_lossy(&key)
            )));
        }
        if let Some(&existing) = self.indices.get(&key) {
            return Ok(existing);
        }
        let result = self.builder.len() as i32;
        self.builder.push(key.len() as u16);
        self.builder.extend_from_slice(&key);
        self.indices.insert(key, result);
        Ok(result)
    }

    /// Equivalent to `finish()`.
    pub(crate) fn finish(&self) -> Lookup {
        Lookup::new(self.builder.clone())
    }

    /// Whether the sorted `array[start..start + length]` holds `flag`.
    ///
    /// Equivalent to `hasFlagInSortedArray(char, char[], int, int)`.
    pub(crate) fn has_flag_in_sorted_array(
        flag: u16,
        array: &[u16],
        start: usize,
        length: usize,
    ) -> bool {
        if flag == FLAG_UNSET {
            return false;
        }
        for &c in &array[start..start + length] {
            if c == flag {
                return true;
            }
            if c > flag {
                return false;
            }
        }
        false
    }
}

/// The flag sets of a [`FlagEnumerator`], by id.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.FlagEnumerator$Lookup`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct Lookup {
    data: Vec<u16>,
}

impl Lookup {
    /// Equivalent to `Lookup(char[])`.
    fn new(data: Vec<u16>) -> Self {
        Self { data }
    }

    /// Equivalent to `hasFlag(int, char)`.
    pub(crate) fn has_flag(&self, entry_id: i32, flag: u16) -> bool {
        entry_id >= 0 && {
            let entry_id = entry_id as usize;
            FlagEnumerator::has_flag_in_sorted_array(
                flag,
                &self.data,
                entry_id + 1,
                usize::from(self.data[entry_id]),
            )
        }
    }

    /// Equivalent to `hasAnyFlag(int, char[])`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn has_any_flag(&self, entry_id: i32, sorted_flags: &[u16]) -> bool {
        let entry_id = entry_id as usize;
        let length = usize::from(self.data[entry_id]);
        if length == 0 {
            return false;
        }
        let mut pos1 = entry_id + 1;
        let limit1 = entry_id + 1 + length;
        let mut pos2 = 0;
        let limit2 = sorted_flags.len();
        let mut c1 = self.data[pos1];
        let mut c2 = sorted_flags[pos2];
        loop {
            if c1 == c2 {
                return true;
            }
            if c1 < c2 {
                pos1 += 1;
                if pos1 >= limit1 {
                    return false;
                }
                c1 = self.data[pos1];
            } else {
                pos2 += 1;
                if pos2 >= limit2 {
                    return false;
                }
                c2 = sorted_flags[pos2];
            }
        }
    }

    /// Equivalent to `getFlags(int)`.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub fn get_flags(&self, entry_id: i32) -> Vec<u16> {
        let entry_id = entry_id as usize;
        self.data[entry_id + 1..entry_id + 1 + usize::from(self.data[entry_id])].to_vec()
    }
}
