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

//! A map of synonyms, keys and values are phrases, ported from
//! `org.apache.lucene.analysis.synonym.SynonymMap`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use rucene::analysis::{
    Analyzer, CharReader, CharTermAttribute, PositionIncrementAttribute, StringCharReader,
};
use rucene::error::{LuceneError, Result};
use rucene::util::byte_block_pool::{ByteBlockPool, DirectAllocator};
use rucene::util::chars_ref::CharsRef;
use rucene::util::fst::{Builder as FSTCompilerBuilder, ByteSequenceOutputs, InputType, Util, FST};
use rucene::util::refs::{CharsRefBuilder, IntsRefBuilder};
use rucene::util::{BytesRef, BytesRefBuilder, BytesRefHash};

use crate::jdk::character::chars_to_utf16;

/// The output words of a [`SynonymMap`]: Java's `BytesRefHash`, which owns
/// its `ByteBlockPool`; `rucene`'s [`BytesRefHash`] keeps the pool apart, so
/// the two travel together here.
#[derive(Debug)]
pub struct SynonymWords {
    hash: BytesRefHash,
    pool: ByteBlockPool,
}

impl SynonymWords {
    /// Java's `new BytesRefHash()`: an empty table over a pool of direct
    /// blocks.
    fn new() -> Self {
        Self {
            hash: BytesRefHash::new(Arc::new(AtomicI64::new(0))),
            pool: ByteBlockPool::with_allocator(Box::new(DirectAllocator::new())),
        }
    }

    /// Equivalent to `BytesRefHash.add(BytesRef)`: the new ord, or `-(ord +
    /// 1)` when the bytes are already in the table.
    ///
    /// # Errors
    ///
    /// What [`BytesRefHash::add`] raises for bytes longer than the pool
    /// holds.
    pub fn add(&mut self, bytes: &[u8]) -> Result<i32> {
        self.hash.add(&mut self.pool, bytes)
    }

    /// Equivalent to `BytesRefHash.get(int, BytesRef)`: the bytes of `ord`.
    pub fn get(&self, ord: i32) -> &[u8] {
        self.hash.get(&self.pool, ord)
    }

    /// Equivalent to `BytesRefHash.find(BytesRef)`: the ord of `bytes`, or
    /// `-1`.
    pub fn find(&self, bytes: &[u8]) -> i32 {
        self.hash.find(&self.pool, bytes)
    }

    /// Equivalent to `BytesRefHash.size()`.
    pub fn size(&self) -> i32 {
        self.hash.size()
    }
}

/// A map of synonyms, keys and values are phrases.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymMap`
/// (`@lucene.experimental`).
///
/// **Divergence from Lucene 10.5.0.** Java's map and the [`Builder`] that
/// built it hold the same `BytesRefHash` object, so words added to the
/// builder afterwards show in the map; Rust cannot share a mutable value
/// without a lock, so [`words`](Self::words) is the builder's table behind
/// an `Arc<RwLock<..>>`, which keeps that sharing. `fst` is `None` where
/// Java's is `null` (no rule was added).
#[derive(Debug)]
pub struct SynonymMap {
    /// map&lt;input word, list&lt;ord&gt;&gt;
    pub fst: Option<FST<ByteSequenceOutputs>>,
    /// map&lt;ord, outputword&gt;
    pub words: Arc<RwLock<SynonymWords>>,
    /// maxHorizontalContext: maximum context we need on the tokenstream
    pub max_horizontal_context: i32,
}

/// Locks `words` for reading; a poisoned lock still holds the table.
pub(crate) fn read_words(words: &RwLock<SynonymWords>) -> RwLockReadGuard<'_, SynonymWords> {
    words
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Locks `words` for writing; a poisoned lock still holds the table.
fn write_words(words: &RwLock<SynonymWords>) -> RwLockWriteGuard<'_, SynonymWords> {
    words
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl SynonymMap {
    /// for multiword support, you must separate words with this separator
    ///
    /// Equivalent to `SynonymMap.WORD_SEPARATOR`, the `char` 0.
    pub const WORD_SEPARATOR: u16 = 0;

    /// Equivalent to `SynonymMap(FST<BytesRef>, BytesRefHash, int)`.
    pub fn new(
        fst: Option<FST<ByteSequenceOutputs>>,
        words: Arc<RwLock<SynonymWords>>,
        max_horizontal_context: i32,
    ) -> Self {
        Self {
            fst,
            words,
            max_horizontal_context,
        }
    }
}

/// The phrase-to-ords entry of the working set.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymMap$Builder$MapEntry`;
/// its hppc `IntArrayList` is a `Vec<i32>`.
#[derive(Debug, Default)]
pub(crate) struct MapEntry {
    include_orig: bool,
    // we could sort for better sharing ultimately, but it could confuse people
    ords: Vec<i32>,
}

/// Builds an FSTSynonymMap.
///
/// Call add() until you have added all the mappings, then call build() to
/// get an FSTSynonymMap
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymMap$Builder`
/// (`@lucene.experimental`).
#[derive(Debug)]
pub struct Builder {
    working_set: HashMap<CharsRef, MapEntry>,
    words: Arc<RwLock<SynonymWords>>,
    utf8_scratch: BytesRefBuilder,
    max_horizontal_context: i32,
    dedup: bool,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// `DataOutput.writeVInt(int)` into `out`.
fn write_vint(out: &mut Vec<u8>, mut i: i32) {
    while (i & !0x7F) != 0 {
        out.push(((i & 0x7F) | 0x80) as u8);
        i = ((i as u32) >> 7) as i32;
    }
    out.push(i as u8);
}

impl Builder {
    /// Default constructor, passes `dedup=true`.
    ///
    /// Equivalent to `Builder()`.
    pub fn new() -> Self {
        Self::with_dedup(true)
    }

    /// If dedup is true then identical rules (same input, same output) will
    /// be added only once.
    ///
    /// Equivalent to `Builder(boolean)`.
    pub fn with_dedup(dedup: bool) -> Self {
        Self {
            working_set: HashMap::new(),
            words: Arc::new(RwLock::new(SynonymWords::new())),
            utf8_scratch: BytesRefBuilder::new(),
            max_horizontal_context: 0,
            dedup,
        }
    }

    /// Sugar: just joins the provided terms with
    /// [`SynonymMap::WORD_SEPARATOR`]. reuse and its chars must not be null.
    ///
    /// Equivalent to `Builder.join(String[], CharsRefBuilder)`; the words
    /// are read as their UTF-16 units.
    pub fn join(words: &[&str], reuse: &mut CharsRefBuilder) -> CharsRef {
        let mut upto = 0;
        for word in words {
            let word: Vec<u16> = word.encode_utf16().collect();
            let word_len = word.len();
            // Add 1 for WORD_SEPARATOR
            let needed = if upto == 0 {
                word_len
            } else {
                1 + upto + word_len
            };
            if needed > reuse.chars().len() {
                reuse.grow(needed);
            }
            if upto > 0 {
                reuse.set_char_at(upto, SynonymMap::WORD_SEPARATOR);
                upto += 1;
            }
            for (i, &unit) in word.iter().enumerate() {
                reuse.set_char_at(upto + i, unit);
            }
            upto += word_len;
        }
        reuse.set_length(upto);
        reuse.get()
    }

    /// only used for asserting!
    ///
    /// Equivalent to `hasHoles(CharsRef)`.
    fn has_holes(chars: &CharsRef) -> bool {
        let end = chars.offset + chars.length;
        for idx in chars.offset + 1..end {
            if chars.chars[idx] == SynonymMap::WORD_SEPARATOR
                && chars.chars[idx - 1] == SynonymMap::WORD_SEPARATOR
            {
                return true;
            }
        }
        if chars.chars[chars.offset] == 0 {
            return true;
        }
        if chars.chars[chars.offset + chars.length - 1] == 0 {
            return true;
        }
        false
    }

    /// Equivalent to the private `add(CharsRef, int, CharsRef, int,
    /// boolean)`.
    ///
    /// NOTE: while it's tempting to make this public, since caller's parser
    /// likely knows the numInput/numOutputWords, sneaky exceptions, much
    /// later on, will result if these values are wrong; so we always
    /// recompute ourselves to be safe.
    fn add_words(
        &mut self,
        input: &CharsRef,
        num_input_words: i32,
        output: &CharsRef,
        num_output_words: i32,
        include_orig: bool,
    ) -> Result<()> {
        // first convert to UTF-8
        if num_input_words <= 0 {
            return Err(LuceneError::illegal_argument(format!(
                "numInputWords must be > 0 (got {num_input_words})"
            )));
        }
        if input.length == 0 {
            return Err(LuceneError::illegal_argument(format!(
                "input.length must be > 0 (got {})",
                input.length
            )));
        }
        if num_output_words <= 0 {
            return Err(LuceneError::illegal_argument(format!(
                "numOutputWords must be > 0 (got {num_output_words})"
            )));
        }
        if output.length == 0 {
            return Err(LuceneError::illegal_argument(format!(
                "output.length must be > 0 (got {})",
                output.length
            )));
        }

        debug_assert!(!Self::has_holes(input), "input has holes: {input}");
        debug_assert!(!Self::has_holes(output), "output has holes: {output}");

        self.utf8_scratch
            .copy_chars_utf16(&output.chars, output.offset, output.length);
        // lookup in hash
        let mut ord = write_words(&self.words).add(self.utf8_scratch.get().slice())?;
        if ord < 0 {
            // already exists in our hash
            ord = (-ord) - 1;
        }

        // make a copy, since we will keep around in our map
        let e = self
            .working_set
            .entry(CharsRef::deep_copy_of(input))
            .or_default();

        e.ords.push(ord);
        e.include_orig |= include_orig;
        self.max_horizontal_context = self.max_horizontal_context.max(num_input_words);
        self.max_horizontal_context = self.max_horizontal_context.max(num_output_words);
        Ok(())
    }

    /// Equivalent to `countWords(CharsRef)`.
    fn count_words(chars: &CharsRef) -> i32 {
        let mut word_count = 1;
        let limit = chars.offset + chars.length;
        for &unit in &chars.chars[chars.offset..limit] {
            if unit == SynonymMap::WORD_SEPARATOR {
                word_count += 1;
            }
        }
        word_count
    }

    /// Add a phrase-&gt;phrase synonym mapping. Phrases are character
    /// sequences where words are separated with character zero (U+0000).
    /// Empty words (two U+0000s in a row) are not allowed in the input nor
    /// the output!
    ///
    /// `input` input phrase, `output` output phrase, `include_orig` true if
    /// the original should be included.
    ///
    /// Equivalent to `Builder.add(CharsRef, CharsRef, boolean)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty input or output, and what
    /// adding the output to the words table raises.
    pub fn add(&mut self, input: &CharsRef, output: &CharsRef, include_orig: bool) -> Result<()> {
        self.add_words(
            input,
            Self::count_words(input),
            output,
            Self::count_words(output),
            include_orig,
        )
    }

    /// Builds an [`SynonymMap`] and returns it.
    ///
    /// Equivalent to `Builder.build()`: one FST arc path per input phrase
    /// (its code points, `FST.INPUT_TYPE.BYTE4`), whose output is the vInt
    /// `count << 1 | (includeOrig ? 0 : 1)` followed by the vInt ord of
    /// each output, the inputs added in `CharsRef.getUTF16SortedAsUTF8Comparator()`
    /// order.
    ///
    /// # Errors
    ///
    /// What compiling the FST raises.
    pub fn build(&mut self) -> Result<SynonymMap> {
        let outputs = ByteSequenceOutputs::get_singleton();
        // TODO: are we using the best sharing options?
        let mut fst_compiler = FSTCompilerBuilder::new(InputType::Byte4, outputs).build()?;

        let mut dedup_set: Option<HashSet<i32>> = if self.dedup {
            Some(HashSet::new())
        } else {
            None
        };

        let mut sorted_keys: Vec<&CharsRef> = self.working_set.keys().collect();
        let comparator = CharsRef::get_utf16_sorted_as_utf8_comparator();
        sorted_keys.sort_by(|a, b| comparator(a, b));

        let mut scratch_ints_ref = IntsRefBuilder::new();

        for input in sorted_keys {
            let output = &self.working_set[input];

            let num_entries = output.ords.len();
            // output size, assume the worst case
            let estimated_size = 5 + num_entries * 5; // numEntries + one ord for each entry
            let mut scratch: Vec<u8> = Vec::with_capacity(estimated_size);

            // now write our output data:
            let mut count = 0;
            for i in 0..num_entries {
                if let Some(dedup_set) = dedup_set.as_mut() {
                    let ent = output.ords[i];
                    if dedup_set.contains(&ent) {
                        continue;
                    }
                    dedup_set.insert(ent);
                }
                write_vint(&mut scratch, output.ords[i]);
                count += 1;
            }

            let pos = scratch.len();
            write_vint(
                &mut scratch,
                (count << 1) | if output.include_orig { 0 } else { 1 },
            );
            let v_int_len = scratch.len() - pos;

            // Move the count + includeOrig to the front of the byte[]:
            scratch.rotate_right(v_int_len);

            if let Some(dedup_set) = dedup_set.as_mut() {
                dedup_set.clear();
            }

            fst_compiler.add(
                &Util::to_utf32_units(
                    &input.chars,
                    input.offset,
                    input.length,
                    &mut scratch_ints_ref,
                ),
                BytesRef::new(scratch),
            )?;
        }

        let metadata = fst_compiler.compile()?;
        let fst = FST::from_fst_reader(metadata, fst_compiler.into_fst_reader()?);
        Ok(SynonymMap::new(
            fst,
            Arc::clone(&self.words),
            self.max_horizontal_context,
        ))
    }
}

/// Abstraction for parsing synonym files.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymMap$Parser`
/// (`@lucene.experimental`), an abstract class extending `Builder`: a parser
/// holds its [`Builder`] and its analyzer (Java's `Parser(boolean dedup,
/// Analyzer analyzer)` is the parser's `Builder::with_dedup(dedup)` and that
/// analyzer), and supplies the abstract `parse`.
pub trait Parser: Send + Sync + std::fmt::Debug {
    /// The `Builder` this parser extends.
    fn builder(&self) -> &Builder;

    /// The `Builder` this parser extends, mutably.
    fn builder_mut(&mut self) -> &mut Builder;

    /// The analyzer of `Parser(boolean, Analyzer)`.
    fn analyzer(&self) -> &dyn Analyzer;

    /// Parse the given input, adding synonyms to the inherited [`Builder`].
    ///
    /// Equivalent to the abstract `parse(Reader)`.
    ///
    /// # Errors
    ///
    /// What reading the input raises, and Java's `java.text.ParseException`.
    fn parse(&mut self, input: &mut dyn CharReader) -> Result<()>;

    /// Sugar: analyzes the text with the analyzer and separates by
    /// [`SynonymMap::WORD_SEPARATOR`]. reuse and its chars must not be null.
    ///
    /// Equivalent to `Parser.analyze(String, CharsRefBuilder)`; `text` holds
    /// the UTF-16 units of Java's string.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a token of length zero, a token whose
    /// position increment is not 1, or a text the analyzer removes
    /// completely, and what the analysis raises.
    fn analyze(&self, text: &[u16], reuse: &mut CharsRefBuilder) -> Result<CharsRef> {
        analyze(self.analyzer(), text, reuse)
    }

    /// Builds the [`SynonymMap`] of the rules parsed so far.
    ///
    /// Equivalent to the inherited `Builder.build()`.
    ///
    /// # Errors
    ///
    /// As [`Builder::build`].
    fn build(&mut self) -> Result<SynonymMap> {
        self.builder_mut().build()
    }
}

/// `Parser.analyze(String, CharsRefBuilder)` with `analyzer`.
fn analyze(analyzer: &dyn Analyzer, text: &[u16], reuse: &mut CharsRefBuilder) -> Result<CharsRef> {
    let text_string = || String::from_utf16_lossy(text);
    {
        // `try (TokenStream ts = analyzer.tokenStream("", text))`
        let ts =
            analyzer.token_stream("", Box::new(StringCharReader::from_utf16(text.to_vec())))?;
        let mut ts = ts.borrow_mut();
        let body = (|| -> Result<()> {
            ts.attribute_source_mut()
                .add_attribute_interface::<dyn CharTermAttribute>()?;
            ts.attribute_source_mut()
                .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
            ts.reset()?;
            reuse.clear();
            while ts.increment_token()? {
                let (term, pos_inc) = {
                    let source = ts.attribute_source();
                    let term_att = source
                        .get_attribute_interface::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the attribute was added above");
                    let pos_inc = source
                        .get_attribute_interface::<dyn PositionIncrementAttribute>()
                        .expect("INVARIANT: the attribute was added above")
                        .get_position_increment();
                    (term_att.buffer()[..term_att.length()].to_vec(), pos_inc)
                };
                let term = chars_to_utf16(&term);
                let length = term.len();
                if length == 0 {
                    return Err(LuceneError::illegal_argument(format!(
                        "term: {} analyzed to a zero-length token",
                        text_string()
                    )));
                }
                if pos_inc != 1 {
                    return Err(LuceneError::illegal_argument(format!(
                        "term: {} analyzed to a token ({}) with position increment != 1 (got: {})",
                        text_string(),
                        String::from_utf16_lossy(&term),
                        pos_inc
                    )));
                }
                reuse.grow(reuse.length() + length + 1); // current + word + separator
                if reuse.length() > 0 {
                    reuse.append(SynonymMap::WORD_SEPARATOR);
                }
                reuse.append_chars(&term, 0, length);
            }
            ts.end()
        })();
        let closed = ts.close();
        match (body, closed) {
            (Err(mut e), Err(c)) => {
                e.add_suppressed(c);
                return Err(e);
            }
            (Err(e), Ok(())) | (Ok(()), Err(e)) => return Err(e),
            (Ok(()), Ok(())) => {}
        }
    }
    if reuse.length() == 0 {
        return Err(LuceneError::illegal_argument(format!(
            "term: {} was completely eliminated by analyzer",
            text_string()
        )));
    }
    Ok(reuse.get())
}
