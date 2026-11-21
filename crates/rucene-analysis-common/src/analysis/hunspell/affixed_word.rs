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

//! The analysis of a simple word, ported from
//! `org.apache.lucene.analysis.hunspell.AffixedWord`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::dict_entry::DictEntry;
use super::dictionary::{Dictionary, AFFIX_FLAG};
use crate::jdk::string::to_rust_string;

/// An object representing the analysis result of a simple (non-compound)
/// word.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.AffixedWord`. The
/// dictionary entry is shared between the words derived from it, as Java
/// shares the object.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AffixedWord {
    word: Vec<u16>,
    entry: Arc<DictEntry>,
    prefixes: Vec<Affix>,
    suffixes: Vec<Affix>,
}

impl AffixedWord {
    /// Equivalent to `AffixedWord(String, DictEntry, List<Affix>,
    /// List<Affix>)`.
    pub(crate) fn new(
        word: Vec<u16>,
        entry: Arc<DictEntry>,
        prefixes: Vec<Affix>,
        suffixes: Vec<Affix>,
    ) -> Self {
        Self {
            word,
            entry,
            prefixes,
            suffixes,
        }
    }

    /// The word being analyzed.
    ///
    /// Equivalent to `getWord()`.
    pub fn get_word(&self) -> String {
        to_rust_string(&self.word)
    }

    /// The word as UTF-16 units.
    pub(crate) fn word(&self) -> &[u16] {
        &self.word
    }

    /// The dictionary entry for the stem in this analysis.
    ///
    /// Equivalent to `getDictEntry()`.
    pub fn get_dict_entry(&self) -> &DictEntry {
        &self.entry
    }

    /// The shared dictionary entry.
    pub(crate) fn entry(&self) -> &Arc<DictEntry> {
        &self.entry
    }

    /// The list of prefixes applied to the stem, at most two, outermost
    /// first.
    ///
    /// Equivalent to `getPrefixes()`.
    pub fn get_prefixes(&self) -> &[Affix] {
        &self.prefixes
    }

    /// The list of suffixes applied to the stem, at most two, outermost
    /// first.
    ///
    /// Equivalent to `getSuffixes()`.
    pub fn get_suffixes(&self) -> &[Affix] {
        &self.suffixes
    }
}

/// Java's `List.toString()`: `[a, b]`.
fn list_to_string<T: fmt::Display>(items: &[T]) -> String {
    let items: Vec<String> = items.iter().map(ToString::to_string).collect();
    format!("[{}]", items.join(", "))
}

impl fmt::Display for AffixedWord {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AffixedWord[word={}, entry={}, prefixes={}, suffixes={}]",
            to_rust_string(&self.word),
            self.entry,
            list_to_string(&self.prefixes),
            list_to_string(&self.suffixes)
        )
    }
}

/// An object representing a prefix or a suffix applied to a word stem.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.AffixedWord$Affix`: two
/// affixes are equal when their ids are.
#[derive(Debug, Clone)]
pub struct Affix {
    pub(crate) affix_id: i32,
    presentable_flag: Vec<u16>,
}

impl Affix {
    /// Equivalent to `Affix(Dictionary, int)`.
    pub(crate) fn new(dictionary: &Dictionary, affix_id: i32) -> Self {
        let encoded_flag = dictionary.affix_data(affix_id, AFFIX_FLAG);
        let presentable_flag = dictionary.flag_parsing_strategy.print_flag(encoded_flag);
        Self {
            affix_id,
            presentable_flag,
        }
    }

    /// The corresponding affix flag as it appears in the *.aff file.
    /// Depending on the format, it could be a Unicode character, two ASCII
    /// characters, or an integer in decimal form.
    ///
    /// Equivalent to `getFlag()`.
    pub fn get_flag(&self) -> String {
        to_rust_string(&self.presentable_flag)
    }
}

impl PartialEq for Affix {
    fn eq(&self, other: &Self) -> bool {
        self.affix_id == other.affix_id
    }
}

impl Eq for Affix {}

impl Hash for Affix {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.affix_id.hash(state);
    }
}

impl fmt::Display for Affix {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}(id={})",
            to_rust_string(&self.presentable_flag),
            self.affix_id
        )
    }
}
