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

//! A dictionary root with its entry id, ported from
//! `org.apache.lucene.analysis.hunspell.Root`.

use std::cmp::Ordering;

/// A root word and the id of its dictionary entry.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Root<T extends
/// CharSequence>`, a record: `T` is held as its UTF-16 units. Equality
/// compares both components, as the record's `equals`; the order compares
/// the words only (`CharSequence.compare`), as `compareTo`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Root {
    /// The record component `word`.
    pub(crate) word: Vec<u16>,
    /// The record component `entryId`.
    pub(crate) entry_id: i32,
}

impl Root {
    /// Equivalent to the canonical constructor `Root(T, int)`.
    pub(crate) fn new(word: Vec<u16>, entry_id: i32) -> Self {
        Self { word, entry_id }
    }

    /// Equivalent to `compareTo(Root)`: `CharSequence.compare(word, o.word)`.
    pub(crate) fn compare_to(&self, o: &Root) -> Ordering {
        self.word.cmp(&o.word)
    }
}

impl std::fmt::Display for Root {
    /// Equivalent to `toString()`, the word.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from_utf16_lossy(&self.word))
    }
}
