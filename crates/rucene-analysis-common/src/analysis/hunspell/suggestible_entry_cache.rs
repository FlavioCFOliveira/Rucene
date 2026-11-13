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

//! A flat cache of the suggestible dictionary entries, ported from
//! `org.apache.lucene.analysis.hunspell.SuggestibleEntryCache`.

use std::collections::HashMap;

use rucene::error::{LuceneError, Result};

use super::flyweight_entry::{CharSequence, FlyweightEntry, LowerCaseRoot};
use super::word_case::WordCase;
use super::word_storage::WordStorage;

const LOWER_CASE: i16 = WordCase::Lower.ordinal();
const NEUTRAL_CASE: i16 = WordCase::Neutral.ordinal();
const TITLE_CASE: i16 = WordCase::Title.ordinal();

/// A cache allowing for CPU-cache-friendlier iteration over [`WordStorage`]
/// entries that can be used for suggestions. The words and the form data are
/// stored in plain contiguous arrays with no compression.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.SuggestibleEntryCache`.
#[derive(Debug)]
pub(crate) struct SuggestibleEntryCache {
    sections: Vec<Option<Section>>,
}

/// `new UnsupportedOperationException("Too long dictionary entry, ...")`.
fn too_long_entry() -> LuceneError {
    LuceneError::unsupported_operation(
        "Too long dictionary entry, please report this to dev@lucene.apache.org",
    )
}

impl SuggestibleEntryCache {
    /// Equivalent to `SuggestibleEntryCache(IntObjectHashMap, int)`.
    fn new(builders: HashMap<usize, SectionBuilder>, max_length: usize) -> Self {
        let mut builders = builders;
        let sections = (0..=max_length)
            .map(|i| builders.remove(&i).map(|builder| builder.build(i)))
            .collect();
        Self { sections }
    }

    /// Equivalent to `buildCache(WordStorage)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for an entry longer than 32767 units.
    pub(crate) fn build_cache(storage: &WordStorage) -> Result<Self> {
        let mut builders: HashMap<usize, SectionBuilder> = HashMap::new();
        let mut max_length = 0;
        storage.process_suggestible_words(1, i32::MAX, &mut |entry: &mut dyn FlyweightEntry| {
            let root_length = entry.root().len();
            if root_length > i16::MAX as usize {
                return Err(too_long_entry());
            } else if root_length > max_length {
                max_length = root_length;
            }
            builders
                .entry(root_length)
                .or_insert_with(SectionBuilder::new)
                .add(entry)
        })?;
        Ok(Self::new(builders, max_length))
    }

    /// Equivalent to `processSuggestibleWords(int, int, Consumer)`.
    ///
    /// # Errors
    ///
    /// What the processor raises, which stops the enumeration.
    pub(crate) fn process_suggestible_words(
        &self,
        min_length: i32,
        max_length: i32,
        processor: &mut dyn FnMut(&mut dyn FlyweightEntry) -> Result<()>,
    ) -> Result<()> {
        let max_length = i64::from(max_length).min(self.sections.len() as i64 - 1);
        let mut i = i64::from(min_length).min(self.sections.len() as i64);
        while i <= max_length {
            if let Some(section) = &self.sections[i as usize] {
                section.process_words(processor)?;
            }
            i += 1;
        }
        Ok(())
    }
}

/// Collects the entries of one root length.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.SuggestibleEntryCache$SectionBuilder`.
#[derive(Debug)]
pub(crate) struct SectionBuilder {
    roots: Vec<u16>,
    low_roots: Vec<u16>,
    meta: Vec<i16>,
    form_data: Vec<i32>,
}

impl SectionBuilder {
    fn new() -> Self {
        Self {
            roots: Vec::new(),
            low_roots: Vec::new(),
            meta: Vec::with_capacity(10),
            form_data: Vec::with_capacity(10),
        }
    }

    /// Equivalent to `add(FlyweightEntry)`.
    fn add(&mut self, entry: &mut dyn FlyweightEntry) -> Result<()> {
        if entry.root().len() > i16::MAX as usize {
            return Err(too_long_entry());
        }
        let forms = entry.forms().to_vec();
        let root = entry.root();
        let root_case = WordCase::case_of(root).ordinal();
        self.meta.push(forms.len() as i16);
        self.meta.push(root_case);
        let lower = entry.lower_case_root();
        self.low_roots
            .extend((0..lower.length()).map(|i| lower.char_at(i)));
        if has_upper_case(root_case) {
            self.roots.extend_from_slice(entry.root());
        }
        self.form_data.extend_from_slice(&forms);
        Ok(())
    }

    /// Equivalent to `build(int)`.
    fn build(self, root_length: usize) -> Section {
        Section {
            root_length,
            meta: self.meta,
            roots: self.roots,
            low_roots: self.low_roots,
            form_data: self.form_data,
        }
    }
}

/// Equivalent to `hasUpperCase(short)`.
fn has_upper_case(root_case: i16) -> bool {
    root_case != LOWER_CASE && root_case != NEUTRAL_CASE
}

/// The entries of one root length: `meta` holds the lengths of the entry
/// sub-arrays in `form_data` plus the case information, `roots` the original
/// roots if they're not all-lowercase.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.SuggestibleEntryCache$Section`.
#[derive(Debug)]
pub(crate) struct Section {
    root_length: usize,
    meta: Vec<i16>,
    roots: Vec<u16>,
    low_roots: Vec<u16>,
    form_data: Vec<i32>,
}

/// The flyweight entry of `Section.processWords`: the views of the section's
/// arrays the Java `CharsRef`s and `IntsRef` hold, by offset.
struct CacheEntry<'a> {
    section: &'a Section,
    chars_offset: usize,
    chars_length: usize,
    lower_offset: usize,
    forms_offset: usize,
    forms_length: usize,
    word_case: i16,
}

impl FlyweightEntry for CacheEntry<'_> {
    fn root(&self) -> &[u16] {
        if has_upper_case(self.word_case) {
            &self.section.roots[self.chars_offset..self.chars_offset + self.chars_length]
        } else {
            self.lower()
        }
    }

    fn has_title_case(&self) -> bool {
        self.word_case == TITLE_CASE
    }

    fn lower_case_root(&self) -> LowerCaseRoot<'_> {
        LowerCaseRoot::Plain(self.lower())
    }

    fn forms(&mut self) -> &[i32] {
        &self.section.form_data[self.forms_offset..self.forms_offset + self.forms_length]
    }
}

impl CacheEntry<'_> {
    fn lower(&self) -> &[u16] {
        &self.section.low_roots[self.lower_offset..self.lower_offset + self.section.root_length]
    }
}

impl Section {
    /// Equivalent to `processWords(Consumer)`.
    fn process_words(
        &self,
        processor: &mut dyn FnMut(&mut dyn FlyweightEntry) -> Result<()>,
    ) -> Result<()> {
        let mut entry = CacheEntry {
            section: self,
            chars_offset: 0,
            chars_length: self.root_length.min(self.roots.len()),
            lower_offset: 0,
            forms_offset: 0,
            forms_length: 0,
            word_case: 0,
        };
        for i in (0..self.meta.len()).step_by(2) {
            let form_data_length = self.meta[i];
            let word_case = self.meta[i + 1];
            entry.forms_length = form_data_length as usize;
            entry.word_case = word_case;
            processor(&mut entry)?;
            entry.lower_offset += self.root_length;
            if has_upper_case(word_case) {
                entry.chars_offset += self.root_length;
            }
            entry.forms_offset += form_data_length as usize;
        }
        Ok(())
    }
}
