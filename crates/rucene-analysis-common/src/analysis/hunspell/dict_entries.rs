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

//! Homonym dictionary entries, ported from
//! `org.apache.lucene.analysis.hunspell.DictEntries`.

use super::dict_entry::DictEntry;

/// An object representing homonym dictionary entries. Note that the order of
/// entries here may differ from the order in the *.dic file!
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.DictEntries`, a
/// `List<DictEntry>`: [`size`](Self::size) and [`get`](Self::get) are the
/// list methods the interface declares.
pub trait DictEntries {
    /// A positive number of dictionary entries with the same word. Most
    /// often it's 1 (unless there are homonyms). Entries are indexed from 0
    /// to `size() - 1` and these indices can be passed into other methods of
    /// this class.
    ///
    /// Equivalent to `size()`.
    fn size(&self) -> usize;

    /// Equivalent to `get(int)`.
    fn get(&self, entry_index: usize) -> DictEntry;

    /// Same as `get(entryIndex).getMorphologicalData()`.
    ///
    /// Equivalent to `getMorphologicalData(int)`.
    fn get_morphological_data(&self, entry_index: usize) -> String {
        self.get(entry_index).get_morphological_data()
    }

    /// Same as `get(entryIndex).getMorphologicalValues(key)`.
    ///
    /// Equivalent to `getMorphologicalValues(int, String)`.
    fn get_morphological_values(&self, entry_index: usize, key: &str) -> Vec<String> {
        self.get(entry_index).get_morphological_values(key)
    }

    /// The entries in order, as iterating the list does.
    fn to_vec(&self) -> Vec<DictEntry> {
        (0..self.size()).map(|i| self.get(i)).collect()
    }
}
