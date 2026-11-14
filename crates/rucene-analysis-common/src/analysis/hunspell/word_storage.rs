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

//! The compact store of the dictionary's words, ported from
//! `org.apache.lucene.analysis.hunspell.WordStorage`.

use rucene::error::{LuceneError, Result};
use rucene::store::{ByteArrayDataInput, DataInput, DataOutput, VecDataOutput};
use rucene::util::fst::{IntSequenceOutputs, Outputs};
use rucene::util::{CharsRef, IntsRef, IntsRefBuilder};

use super::dictionary::{Dictionary, HIDDEN_FLAG};
use super::flag_enumerator::FlagEnumerator;
use super::flyweight_entry::{FlyweightEntry, LowerCaseRoot};
use super::generating_suggester::GeneratingSuggester;
use super::word_case::WordCase;
use crate::jdk::character::is_upper_case;
use crate::jdk::string::{hash_code, to_rust_string};

const OFFSET_BITS: u32 = 25;
const OFFSET_MASK: i32 = (1 << OFFSET_BITS) - 1;
const COLLISION_MASK: i32 = 0x40;
const SUGGESTIBLE_MASK: i32 = 0x20;
const MAX_STORED_LENGTH: i32 = SUGGESTIBLE_MASK - 1;

/// `new java.lang.RuntimeException(message)`.
fn runtime_exception(message: impl Into<String>) -> LuceneError {
    LuceneError::unchecked("java.lang.RuntimeException", message)
}

/// `entryCode >>> OFFSET_BITS`.
fn mask_of(entry_code: i32) -> i32 {
    ((entry_code as u32) >> OFFSET_BITS) as i32
}

/// Reads a VInt the builder or the storage wrote.
fn read_vint(input: &mut ByteArrayDataInput<'_>) -> i32 {
    input
        .read_v_int()
        .expect("INVARIANT: the word data holds a complete VInt here")
}

/// A data structure for memory-efficient word storage and fast
/// lookup/enumeration. Each dictionary entry is stored as:
///
/// 1. the last character
/// 2. pointer to a similar entry for the prefix (all characters except the
///    last one)
/// 3. value data: a list of ints representing word flags and morphological
///    data, and a pointer to hash collisions, if any
///
/// There's only one entry for each prefix, so it's like a trie/`FST`, but a
/// reversed one: each node points to a single previous node instead of
/// several following ones. For example, "abc" and "abd" point to the same
/// prefix entry "ab" which points to "a" which points to 0.
///
/// The entries are stored in a contiguous byte array, identified by their
/// offsets, using `DataOutput.writeVInt` format for compression.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordStorage`, an
/// abstract class whose only subclass, in `Dictionary`, implements
/// `caseFold(char)` as `Dictionary.caseFold`; this port keeps the flag that
/// method reads, `Dictionary.alternateCasing`, which is final once the words
/// are stored.
#[derive(Debug)]
pub(crate) struct WordStorage {
    max_entry_length: usize,
    has_custom_morph_data: bool,
    /// A map from word's hash (modulo array's length) into an int
    /// containing: the lower `OFFSET_BITS` the offset in `word_data` of the
    /// last entry with this hash; the remaining highest bits
    /// COLLISION+SUGGESTIBLE+LENGTH info for that entry.
    hash_table: Vec<i32>,
    /// An array of word entries: VINT the word's last character, VINT a
    /// delta pointer to the entry for the same word without the last
    /// character, for hash-colliding entries a BYTE of COLLISION+SUGGESTIBLE+
    /// LENGTH info and a VINT (delta) pointer to the previous entry, and for
    /// non-leaf entries the VINT+ word form data preceded by its length.
    word_data: Vec<u8>,
    /// `Dictionary.alternateCasing`, which `caseFold` reads.
    alternate_casing: bool,
}

impl WordStorage {
    /// Equivalent to `WordStorage(Builder)` in `Dictionary`'s anonymous
    /// subclass, whose `caseFold` folds with `alternate_casing`.
    ///
    /// # Errors
    ///
    /// What flushing the builder's last group raises.
    pub(crate) fn new(mut builder: Builder<'_>, alternate_casing: bool) -> Result<Self> {
        if !builder.hash_table.is_empty() {
            debug_assert!(
                !builder.group.is_empty(),
                "WordStorage builder should be only used once"
            );
            builder.flush_group()?;
        }
        Ok(Self {
            max_entry_length: builder.max_entry_length,
            has_custom_morph_data: builder.has_custom_morph_data,
            hash_table: if builder.hash_table.is_empty() {
                vec![0]
            } else {
                builder.hash_table
            },
            word_data: builder.data_writer.into_inner(),
            alternate_casing,
        })
    }

    /// An empty storage, which `Dictionary` holds until its words are read.
    pub(crate) fn empty() -> Self {
        Self {
            max_entry_length: 0,
            has_custom_morph_data: false,
            hash_table: vec![0],
            word_data: vec![0],
            alternate_casing: false,
        }
    }

    /// The forms of the word `word[offset..offset + length]`, or `None`.
    ///
    /// Equivalent to `lookupWord(char[], int, int)`.
    pub(crate) fn lookup_word(
        &self,
        word: &[u16],
        offset: usize,
        length: usize,
    ) -> Option<IntsRef> {
        debug_assert!(length > 0);
        let table_length = self.hash_table.len() as i32;
        let hash = (CharsRef::string_hash_code(word, offset, length) % table_length).unsigned_abs()
            as usize;
        let entry_code = self.hash_table[hash];
        if entry_code == 0 {
            return None;
        }
        let mut pos = entry_code & OFFSET_MASK;
        let mut mask = mask_of(entry_code);
        let last_char = word[offset + length - 1];
        let mut input = ByteArrayDataInput::new(&self.word_data);
        loop {
            input.set_position(pos as usize);
            let c = read_vint(&mut input) as u16;
            let prev_pos = pos - read_vint(&mut input);
            let last = !has_collision(mask);
            let might_match = c == last_char && has_length(mask, length as i32);
            if !last {
                mask = i32::from(input.read_byte().expect("INVARIANT: a mask byte") as i8);
                pos -= read_vint(&mut input);
            }
            if might_match {
                let before_forms = input.position();
                if is_same_string(word, offset, length as i32 - 1, prev_pos, &mut input) {
                    input.set_position(before_forms);
                    let form_length = read_vint(&mut input) as usize;
                    let mut forms = IntsRef::new(vec![0; form_length]);
                    read_forms(&mut forms, &mut input, form_length);
                    return Some(forms);
                }
            }
            if last {
                return None;
            }
        }
    }

    /// Calls the processor for every dictionary entry with length between
    /// `min_length` and `max_length`, both ends inclusive, and at least one
    /// suggestible alternative (without NOSUGGEST, FORBIDDENWORD or
    /// ONLYINCOMPOUND flags). Note that the callback arguments (word and
    /// forms) are reused, so they can be modified in any way, but may not be
    /// saved for later by the processor.
    ///
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
        self.process_all_words(min_length, max_length, true, processor)
    }

    /// Equivalent to `processAllWords(int, int, boolean, Consumer)`.
    ///
    /// # Errors
    ///
    /// What the processor raises, which stops the enumeration.
    pub(crate) fn process_all_words(
        &self,
        min_length: i32,
        max_length: i32,
        suggestible_only: bool,
        processor: &mut dyn FnMut(&mut dyn FlyweightEntry) -> Result<()>,
    ) -> Result<()> {
        debug_assert!(min_length <= max_length);
        let max_length = (self.max_entry_length as i64).min(i64::from(max_length)) as i32;
        let mut entry = MyFlyweightEntry {
            chars: vec![0; max_length.max(0) as usize],
            offset: 0,
            length: 0,
            word_data: &self.word_data,
            data_pos: 0,
            forms: Vec::new(),
            has_custom_morph_data: self.has_custom_morph_data,
            alternate_casing: self.alternate_casing,
        };
        let mut input = ByteArrayDataInput::new(&self.word_data);
        for &entry_code in &self.hash_table {
            let mut pos = entry_code & OFFSET_MASK;
            let mut mask = mask_of(entry_code);
            while pos != 0 {
                let mut word_start = max_length as i64 - 1;
                input.set_position(pos as usize);
                entry.chars[word_start as usize] = read_vint(&mut input) as u16;
                let mut prev_pos = pos - read_vint(&mut input);
                let last = !has_collision(mask);
                let might_match = (!suggestible_only || has_suggestible_entries(mask))
                    && has_length_in_range(mask, min_length, max_length);
                if !last {
                    mask = i32::from(input.read_byte().expect("INVARIANT: a mask byte") as i8);
                    pos -= read_vint(&mut input);
                }
                if might_match {
                    entry.data_pos = input.position();
                    while prev_pos != 0 && word_start > 0 {
                        input.set_position(prev_pos as usize);
                        word_start -= 1;
                        entry.chars[word_start as usize] = read_vint(&mut input) as u16;
                        prev_pos -= read_vint(&mut input);
                    }
                    if prev_pos == 0 {
                        entry.offset = word_start as usize;
                        entry.length = (max_length as i64 - word_start) as usize;
                        processor(&mut entry)?;
                    }
                }
                if last {
                    break;
                }
            }
        }
        Ok(())
    }

    /// The folded unit of `c`.
    ///
    /// Equivalent to `caseFold(char)`, which the subclass in `Dictionary`
    /// implements as `Dictionary.caseFold`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn case_fold(&self, c: u16) -> u16 {
        Dictionary::case_fold_with(self.alternate_casing, c)
    }
}

/// Equivalent to `hasCollision(int)`.
fn has_collision(mask: i32) -> bool {
    (mask & COLLISION_MASK) != 0
}

/// Equivalent to `hasSuggestibleEntries(int)`.
fn has_suggestible_entries(mask: i32) -> bool {
    (mask & SUGGESTIBLE_MASK) != 0
}

/// Equivalent to `hasLength(int, int)`.
fn has_length(mask: i32, length: i32) -> bool {
    let len_code = mask & MAX_STORED_LENGTH;
    if len_code == MAX_STORED_LENGTH {
        length >= MAX_STORED_LENGTH
    } else {
        len_code == length
    }
}

/// Equivalent to `hasLengthInRange(int, int, int)`.
fn has_length_in_range(mask: i32, min_length: i32, max_length: i32) -> bool {
    let len_code = mask & MAX_STORED_LENGTH;
    if len_code == MAX_STORED_LENGTH {
        return max_length >= MAX_STORED_LENGTH;
    }
    len_code >= min_length && len_code <= max_length
}

/// Equivalent to `isSameString(char[], int, int, int, ByteArrayDataInput)`.
fn is_same_string(
    word: &[u16],
    offset: usize,
    length: i32,
    data_pos: i32,
    input: &mut ByteArrayDataInput<'_>,
) -> bool {
    let mut data_pos = data_pos;
    let mut i = length - 1;
    while i >= 0 {
        input.set_position(data_pos as usize);
        let c = read_vint(input) as u16;
        if c != word[i as usize + offset] {
            return false;
        }
        data_pos -= read_vint(input);
        if data_pos == 0 {
            return i == 0;
        }
        i -= 1;
    }
    length == 0 && data_pos == 0
}

/// Equivalent to `readForms(IntsRef, ByteArrayDataInput, int)`.
fn read_forms(forms: &mut IntsRef, input: &mut ByteArrayDataInput<'_>, length: usize) {
    for i in 0..length {
        forms.ints[i] = read_vint(input);
    }
    forms.length = length;
}

/// Builds a [`WordStorage`] from entries added in order.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordStorage$Builder`.
/// Java's `ByteArrayDataOutput` subclass that grows its array on demand is a
/// growable [`VecDataOutput`]: the bytes written are the same.
#[derive(Debug)]
pub(crate) struct Builder<'a> {
    has_custom_morph_data: bool,
    hash_table: Vec<i32>,
    no_suggest_flags: Vec<u16>,
    chain_lengths: Vec<i32>,
    current_ords: IntsRefBuilder,
    group: Vec<Vec<u16>>,
    morph_data_ids: Vec<i32>,
    current_entry: Option<Vec<u16>>,
    word_count: i32,
    hash_factor: f64,
    flag_enumerator: &'a mut FlagEnumerator,
    data_writer: VecDataOutput,
    common_prefix_length: usize,
    common_prefix_pos: i32,
    actual_words: i32,
    max_entry_length: usize,
}

impl<'a> Builder<'a> {
    /// `word_count` is an approximate number of the words in the resulting
    /// dictionary, used to pre-size the hash table. This argument can be a
    /// bit larger than the actual word count, but not smaller.
    ///
    /// Equivalent to `Builder(int, double, boolean, FlagEnumerator, char[])`.
    ///
    /// # Errors
    ///
    /// Java's `NegativeArraySizeException` for a negative table size.
    pub(crate) fn new(
        word_count: i32,
        hash_factor: f64,
        has_custom_morph_data: bool,
        flag_enumerator: &'a mut FlagEnumerator,
        no_suggest_flags: Vec<u16>,
    ) -> Result<Self> {
        let table_size = (f64::from(word_count) * hash_factor) as i32;
        if table_size < 0 {
            return Err(LuceneError::unchecked(
                "java.lang.NegativeArraySizeException",
                table_size.to_string(),
            ));
        }
        let mut data_writer =
            VecDataOutput::with_capacity((i64::from(word_count) * 6).max(0) as usize);
        // zero index is root, contains nothing
        data_writer.write_byte(0)?;
        Ok(Self {
            has_custom_morph_data,
            hash_table: vec![0; table_size as usize],
            no_suggest_flags,
            chain_lengths: vec![0; table_size as usize],
            current_ords: IntsRefBuilder::new(),
            group: Vec::new(),
            morph_data_ids: Vec::new(),
            current_entry: None,
            word_count,
            hash_factor,
            flag_enumerator,
            data_writer,
            common_prefix_length: 0,
            common_prefix_pos: 0,
            actual_words: 0,
            max_entry_length: 0,
        })
    }

    /// Add a dictionary entry. This method should be called for entries
    /// sorted non-descending by `String.compareTo` rules.
    ///
    /// Equivalent to `add(String, char[], int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("out of order: ...")` for an entry sorted
    /// before the previous one, and what flushing a group raises.
    pub(crate) fn add(&mut self, entry: &[u16], flags: Vec<u16>, morph_data_id: i32) -> Result<()> {
        self.max_entry_length = self.max_entry_length.max(entry.len());
        if self.current_entry.as_deref() != Some(entry) {
            if let Some(current_entry) = self.current_entry.clone() {
                if entry < current_entry.as_slice() {
                    return Err(LuceneError::illegal_argument(format!(
                        "out of order: {} < {}",
                        to_rust_string(entry),
                        to_rust_string(&current_entry)
                    )));
                }
                let mut pos = self.flush_group()?;
                self.common_prefix_length =
                    GeneratingSuggester::common_prefix(&current_entry, entry);
                let mut input = ByteArrayDataInput::new(self.data_writer.as_inner());
                input.set_position(pos as usize);
                for i in (self.common_prefix_length..current_entry.len()).rev() {
                    let c = read_vint(&mut input) as u16;
                    debug_assert_eq!(c, current_entry[i]);
                    pos -= read_vint(&mut input);
                    input.set_position(pos as usize);
                }
                self.common_prefix_pos = pos;
            }
            self.current_entry = Some(entry.to_vec());
        }
        self.group.push(flags);
        if self.has_custom_morph_data {
            self.morph_data_ids.push(morph_data_id);
        }
        Ok(())
    }

    /// Equivalent to `flushGroup()`.
    fn flush_group(&mut self) -> Result<i32> {
        self.actual_words += 1;
        if self.actual_words > self.word_count {
            return Err(runtime_exception("Don't add more words than wordCount!"));
        }
        let Some(current_entry) = self.current_entry.clone() else {
            // `currentEntry.length()` on `null`
            return Err(LuceneError::null_pointer());
        };
        self.current_ords.clear();
        let mut has_non_hidden = false;
        let mut is_suggestible = false;
        for flags in &self.group {
            if !has_flag(flags, HIDDEN_FLAG) {
                has_non_hidden = true;
            }
            if !self.has_no_suggest_flag(flags) {
                is_suggestible = true;
            }
        }
        let group_size = self.group.len();
        for i in 0..group_size {
            if has_non_hidden && group_size > 1 && has_flag(&self.group[i], HIDDEN_FLAG) {
                continue;
            }
            let ord = self.flag_enumerator.add(&mut self.group[i])?;
            self.current_ords.append(ord);
            if self.has_custom_morph_data {
                self.current_ords.append(self.morph_data_ids[i]);
            }
        }

        // write the non-leaf entries for chars after the shared prefix, except the last one
        let mut last_pos = self.common_prefix_pos;
        for &c in current_entry
            .iter()
            .take(current_entry.len() - 1)
            .skip(self.common_prefix_length)
        {
            let pos = self.data_writer.len() as i32;
            self.data_writer.write_v_int(i32::from(c))?;
            self.data_writer.write_v_int(pos - last_pos)?;
            last_pos = pos;
        }

        let pos = self.data_writer.len() as i32;
        if pos >= 1 << OFFSET_BITS {
            return Err(runtime_exception(
                "Too much word data, please report this to dev@lucene.apache.org",
            ));
        }

        let table_length = self.hash_table.len() as i32;
        if table_length == 0 {
            // `hashCode() % hashTable.length` over an empty table
            return Err(LuceneError::from(
                rucene::error::LuceneErrorKind::Arithmetic("/ by zero".to_string()),
            ));
        }
        let hash = (hash_code(&current_entry) % table_length).unsigned_abs() as usize;
        let prev_code = self.hash_table[hash];
        let mask = (if prev_code == 0 { 0 } else { COLLISION_MASK })
            | (if is_suggestible { SUGGESTIBLE_MASK } else { 0 })
            | (current_entry.len() as i32).min(MAX_STORED_LENGTH);
        self.hash_table[hash] = (mask << OFFSET_BITS) | pos;

        self.chain_lengths[hash] += 1;
        if self.chain_lengths[hash] > 20 {
            return Err(runtime_exception(format!(
                "Too many collisions. Try a larger Dictionary#hashFactor (now {}). If this doesn't help, please report this to dev@lucene.apache.org",
                rucene::util::java_fmt::double_to_string(self.hash_factor)
            )));
        }

        // write the leaf entry for the last character
        self.data_writer
            .write_v_int(i32::from(current_entry[current_entry.len() - 1]))?;
        self.data_writer.write_v_int(pos - last_pos)?;
        if prev_code != 0 {
            self.data_writer.write_byte(mask_of(prev_code) as u8)?;
            self.data_writer
                .write_v_int(pos - (prev_code & OFFSET_MASK))?;
        }
        IntSequenceOutputs::get_singleton()
            .write(&self.current_ords.get(), &mut self.data_writer)?;

        self.group.clear();
        self.morph_data_ids.clear();
        Ok(pos)
    }

    /// Equivalent to `hasNoSuggestFlag(char[])`.
    fn has_no_suggest_flag(&self, flags: &[u16]) -> bool {
        flags
            .iter()
            .any(|&flag| has_flag(&self.no_suggest_flags, flag))
    }
}

/// Equivalent to `Builder.hasFlag(char[], char)`.
fn has_flag(flags: &[u16], flag: u16) -> bool {
    flags.contains(&flag)
}

/// The entry [`WordStorage::process_all_words`] passes to its processor.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordStorage$MyFlyweightEntry`:
/// `chars[offset..offset + length]` is the reused `CharsRef`, and the forms
/// are read from `word_data` at `data_pos`.
#[derive(Debug)]
pub(crate) struct MyFlyweightEntry<'a> {
    chars: Vec<u16>,
    offset: usize,
    length: usize,
    word_data: &'a [u8],
    data_pos: usize,
    forms: Vec<i32>,
    has_custom_morph_data: bool,
    alternate_casing: bool,
}

impl FlyweightEntry for MyFlyweightEntry<'_> {
    fn has_title_case(&self) -> bool {
        let root = self.root();
        is_upper_case(i32::from(root[0])) && WordCase::case_of(root) == WordCase::Title
    }

    fn root(&self) -> &[u16] {
        &self.chars[self.offset..self.offset + self.length]
    }

    fn lower_case_root(&self) -> LowerCaseRoot<'_> {
        LowerCaseRoot::Folded {
            chars: self.root(),
            alternate_casing: self.alternate_casing,
        }
    }

    fn forms(&mut self) -> &[i32] {
        let mut input = ByteArrayDataInput::new(self.word_data);
        input.set_position(self.data_pos);
        let entry_count =
            (read_vint(&mut input) / if self.has_custom_morph_data { 2 } else { 1 }) as usize;
        if self.forms.len() < entry_count {
            self.forms = vec![0; entry_count];
        }
        for i in 0..entry_count {
            self.forms[i] = read_vint(&mut input);
            if self.has_custom_morph_data {
                read_vint(&mut input);
            }
        }
        &self.forms[..entry_count]
    }
}
