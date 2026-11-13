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

//! A `*.dic` file entry, ported from
//! `org.apache.lucene.analysis.hunspell.DictEntry`.

use std::fmt;

use crate::jdk::string::{contains, split_unit, starts_with_str, to_rust_string, units};

/// An object representing *.dic file entry with its word, flags and
/// morphological data.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.DictEntry`, an abstract
/// class whose two anonymous subclasses (`Dictionary.dictEntry` and
/// `DictEntry.create`) compute the flags and the morphological data.
///
/// **Divergence.** `Dictionary.dictEntry` computes `getFlags()` and
/// `getMorphologicalData()` on each call from the dictionary, which never
/// changes after construction; this port computes them once, when the entry
/// is created, so an entry holds no reference to its dictionary. The values
/// are the same.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DictEntry {
    stem: Vec<u16>,
    flags: Vec<u16>,
    morphological_data: Vec<u16>,
}

impl DictEntry {
    /// An entry of `stem` with the presentable `flags` and
    /// `morphological_data`.
    pub(crate) fn new(stem: Vec<u16>, flags: Vec<u16>, morphological_data: Vec<u16>) -> Self {
        Self {
            stem,
            flags,
            morphological_data,
        }
    }

    /// Equivalent to `DictEntry.create(String, String)`: an entry with no
    /// morphological data.
    pub(crate) fn create(stem: Vec<u16>, flags: Vec<u16>) -> Self {
        Self::new(stem, flags, Vec::new())
    }

    /// The stem word in the dictionary.
    ///
    /// Equivalent to `getStem()`.
    pub fn get_stem(&self) -> String {
        to_rust_string(&self.stem)
    }

    /// The stem as UTF-16 units.
    pub(crate) fn stem(&self) -> &[u16] {
        &self.stem
    }

    /// The flags associated with the dictionary entry, encoded in the same
    /// format as in the *.dic file, but possibly in a different order.
    ///
    /// Equivalent to `getFlags()`.
    pub fn get_flags(&self) -> String {
        to_rust_string(&self.flags)
    }

    /// The flags as UTF-16 units.
    pub(crate) fn flags(&self) -> &[u16] {
        &self.flags
    }

    /// Morphological fields (of `kk:vvvvvv` form, sorted, space-separated,
    /// excluding `ph:`) associated with the homonym at the given entry index,
    /// or an empty string.
    ///
    /// Equivalent to `getMorphologicalData()`.
    pub fn get_morphological_data(&self) -> String {
        to_rust_string(&self.morphological_data)
    }

    /// The values (of `vvvvvv` form) of morphological fields with the given
    /// key (in the form `kk:`) associated with the homonym at the given entry
    /// index.
    ///
    /// Equivalent to `getMorphologicalValues(String)`. Java asserts the key's
    /// form; this port checks it in debug builds.
    pub fn get_morphological_values(&self, key: &str) -> Vec<String> {
        let key = units(key);
        debug_assert!(
            key.len() == 3 && key[2] == u16::from(b':'),
            "A morphological data key should consist of two letters followed by a semicolon, found: {}",
            to_rust_string(&key)
        );
        let data = &self.morphological_data;
        if data.is_empty() || !contains(data, &key) {
            return Vec::new();
        }
        split_unit(data, u16::from(b' '))
            .into_iter()
            .filter(|s| s.starts_with(&key))
            .map(|s| to_rust_string(&s[3..]))
            .collect()
    }
}

impl fmt::Display for DictEntry {
    /// Equivalent to `toString()`: the stem, then `/` and the flags when
    /// there are some, then a space and the morphological data when there is
    /// some.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_rust_string(&self.stem))?;
        if !self.flags.is_empty() {
            write!(f, "/{}", to_rust_string(&self.flags))?;
        }
        if !self.morphological_data.is_empty() {
            write!(f, " {}", to_rust_string(&self.morphological_data))?;
        }
        Ok(())
    }
}

/// Whether `s` starts with `ph:`, the phonetic morphological field.
pub(crate) fn is_phonetic_field(s: &[u16]) -> bool {
    starts_with_str(s, "ph:")
}
