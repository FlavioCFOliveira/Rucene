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
//! Overrides stemming with a custom dictionary, ported from
//! `org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilter`.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::byte_block_pool::{ByteBlockPool, DirectAllocator};
use rucene::util::fst::{
    Arc as FSTArc, Builder as FSTCompilerBuilder, ByteSequenceOutputs, BytesReader, InputType,
    Outputs, FST,
};
use rucene::util::refs::IntsRefBuilder;
use rucene::util::{BytesRef, BytesRefBuilder, BytesRefHash};

/// The body of [`StemmerOverrideFilter`]: what Lucene's
/// `StemmerOverrideFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct StemmerOverrideFilterLogic {
    stemmer_override_map: Arc<StemmerOverrideMap>,
    scratch_arc: FSTArc<BytesRef>,
}

impl TokenFilterLogic for StemmerOverrideFilterLogic {
    /// Equivalent to `StemmerOverrideFilter.incrementToken()`.
    ///
    /// Java keeps the `fstReader` of its constructor; a reader borrows the
    /// FST here and cannot live in the filter next to the map it reads, so a
    /// reader is taken from the map for each lookup (its position is set by
    /// every arc lookup, so a fresh one reads the same bytes).
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        let Some(mut fst_reader) = self.stemmer_override_map.get_bytes_reader()? else {
            // No overrides
            return Ok(true);
        };
        let is_keyword = source
            .get_attribute_interface::<dyn KeywordAttribute>()
            .expect("INVARIANT: the constructor added the KeywordAttribute")
            .is_keyword();
        if !is_keyword {
            // don't muck with already-keyworded terms
            let term: Vec<char> = {
                let term_att = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                term_att.buffer()[..term_att.length()].to_vec()
            };
            let stem = self.stemmer_override_map.get(
                &term,
                term.len(),
                &mut self.scratch_arc,
                fst_reader.as_mut(),
            )?;
            if let Some(stem) = stem {
                let chars: Vec<char> = String::from_utf8_lossy(stem.slice()).chars().collect();
                source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute")
                    .copy_buffer(&chars, 0, chars.len());
                source
                    .get_attribute_interface_mut::<dyn KeywordAttribute>()
                    .expect("INVARIANT: the constructor added the KeywordAttribute")
                    .set_keyword(true);
            }
        }
        Ok(true)
    }
}

/// Provides the ability to override any `KeywordAttribute` aware stemmer with
/// custom dictionary-based stemming.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilter`.
pub type StemmerOverrideFilter = TokenFilter<StemmerOverrideFilterLogic>;

/// Create a new StemmerOverrideFilter, performing dictionary-based stemming
/// with the provided dictionary (`stemmer_override_map`).
///
/// Any dictionary-stemmed terms will be marked with `KeywordAttribute` so
/// that they will not be stemmed with stemmers down the chain.
///
/// Equivalent to `StemmerOverrideFilter(TokenStream, StemmerOverrideMap)`,
/// which adds the `CharTermAttribute` and the `KeywordAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_stemmer_override_filter(
    input: Box<dyn TokenStream>,
    stemmer_override_map: Arc<StemmerOverrideMap>,
) -> Result<StemmerOverrideFilter> {
    let mut filter = TokenFilter::new(
        input,
        StemmerOverrideFilterLogic {
            stemmer_override_map,
            scratch_arc: FSTArc::default(),
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}

/// A read-only 4-byte FST backed map that allows fast case-insensitive key
/// value lookups for [`StemmerOverrideFilter`].
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilter$StemmerOverrideMap`.
#[derive(Debug)]
pub struct StemmerOverrideMap {
    fst: Option<FST<ByteSequenceOutputs>>,
    ignore_case: bool,
}

impl StemmerOverrideMap {
    /// Creates a new StemmerOverrideMap: `fst` the fst to lookup the
    /// overrides (`None` for Java's `null`), `ignore_case` if the keys case
    /// should be ingored.
    ///
    /// Equivalent to `StemmerOverrideMap(FST<BytesRef>, boolean)`.
    pub fn new(fst: Option<FST<ByteSequenceOutputs>>, ignore_case: bool) -> Self {
        Self { fst, ignore_case }
    }

    /// Returns a `BytesReader` to pass to the [`get`](Self::get) method, or
    /// `None` when there is no FST.
    ///
    /// Equivalent to `StemmerOverrideMap.getBytesReader()`.
    ///
    /// # Errors
    ///
    /// What the FST's reader raises.
    pub fn get_bytes_reader(&self) -> Result<Option<Box<dyn BytesReader + '_>>> {
        match &self.fst {
            None => Ok(None),
            Some(fst) => Ok(Some(fst.get_bytes_reader()?)),
        }
    }

    /// Returns the value mapped to the given key or `None` if the key is not
    /// in the FST dictionary.
    ///
    /// Equivalent to `StemmerOverrideMap.get(char[], int, Arc, BytesReader)`;
    /// `buffer` holds `char`s, one per code point, which Java's loop reads
    /// with `Character.codePointAt` and advances by `Character.charCount`.
    ///
    /// # Errors
    ///
    /// What reading the FST raises.
    ///
    /// # Panics
    ///
    /// With the `NullPointerException` Java throws when there is no FST.
    pub fn get(
        &self,
        buffer: &[char],
        buffer_len: usize,
        scratch_arc: &mut FSTArc<BytesRef>,
        fst_reader: &mut dyn BytesReader,
    ) -> Result<Option<BytesRef>> {
        let Some(fst) = &self.fst else {
            rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None)
        };
        let outputs = fst.outputs();
        let mut pending_output = outputs.no_output();
        let mut match_output = None;
        let mut buf_upto = 0;
        fst.get_first_arc(scratch_arc);
        while buf_upto < buffer_len {
            let code_point = buffer[buf_upto] as i32;
            let label = if self.ignore_case {
                rucene::util::automaton::case_folding::to_lower_case(code_point)
            } else {
                code_point
            };
            if !fst.find_target_arc_in_place(label, scratch_arc, fst_reader)? {
                return Ok(None);
            }
            pending_output = outputs.add(&pending_output, scratch_arc.output());
            buf_upto += 1;
        }
        if scratch_arc.is_final() {
            match_output = Some(outputs.add(&pending_output, scratch_arc.next_final_output()));
        }
        Ok(match_output)
    }
}

/// This builder builds an FST for the [`StemmerOverrideFilter`].
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.StemmerOverrideFilter$Builder`.
#[derive(Debug)]
pub struct Builder {
    /// Java's `new BytesRefHash()`, which owns a `ByteBlockPool` over a
    /// `DirectAllocator`; the pool is a separate value here.
    hash: BytesRefHash,
    pool: ByteBlockPool,
    spare: BytesRefBuilder,
    output_values: Vec<String>,
    ignore_case: bool,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// Creates a new [`Builder`] with ignoreCase set to `false`.
    ///
    /// Equivalent to `Builder()`, `this(false)`.
    pub fn new() -> Self {
        Self::with_ignore_case(false)
    }

    /// Creates a new [`Builder`]; `ignore_case` if the input case should be
    /// ignored.
    ///
    /// Equivalent to `Builder(boolean)`.
    pub fn with_ignore_case(ignore_case: bool) -> Self {
        Self {
            hash: BytesRefHash::new(Arc::new(AtomicI64::new(0))),
            pool: ByteBlockPool::with_allocator(Box::new(DirectAllocator::new())),
            spare: BytesRefBuilder::new(),
            output_values: Vec::new(),
            ignore_case,
        }
    }

    /// Adds an input string and its stemmer override output to this builder.
    /// Returns `false` iff the input has already been added to this builder
    /// otherwise `true`.
    ///
    /// Equivalent to `Builder.add(CharSequence, CharSequence)`: with
    /// `ignoreCase`, each code point is lower-cased with
    /// `Character.toLowerCase(int)`.
    ///
    /// # Errors
    ///
    /// What [`BytesRefHash::add`] raises for a term longer than the pool
    /// holds.
    pub fn add(&mut self, input: &str, output: &str) -> Result<bool> {
        if self.ignore_case {
            // convert on the fly to lowercase
            let lower: String = input
                .chars()
                .map(|c| {
                    crate::jdk::character::to_term_char(
                        rucene::util::automaton::case_folding::to_lower_case(c as i32),
                    )
                })
                .collect();
            self.spare.copy_chars(&lower);
        } else {
            self.spare.copy_chars(input);
        }
        let bytes = self.spare.get();
        if self.hash.add(&mut self.pool, bytes.slice())? >= 0 {
            self.output_values.push(output.to_string());
            return Ok(true);
        }
        Ok(false)
    }

    /// Returns a [`StemmerOverrideMap`] to be used with the
    /// [`StemmerOverrideFilter`].
    ///
    /// Equivalent to `Builder.build()`: an FST over 4-byte labels (the code
    /// points of the inputs) with `ByteSequenceOutputs`, the inputs added in
    /// sorted order.
    ///
    /// # Errors
    ///
    /// What compiling the FST raises.
    pub fn build(&mut self) -> Result<StemmerOverrideMap> {
        let outputs = ByteSequenceOutputs::get_singleton();
        let mut fst_compiler = FSTCompilerBuilder::new(InputType::Byte4, outputs).build()?;
        let sort: Vec<i32> = self.hash.sort(&self.pool).to_vec();
        let mut ints_spare = IntsRefBuilder::new();
        let size = self.hash.size() as usize;
        for &id in &sort[..size] {
            let bytes_ref = BytesRef::new(self.hash.get(&self.pool, id).to_vec());
            ints_spare.copy_utf8_bytes(&bytes_ref)?;
            fst_compiler.add(
                &ints_spare.get(),
                BytesRef::new(self.output_values[id as usize].as_bytes().to_vec()),
            )?;
        }
        let metadata = fst_compiler.compile()?;
        let fst = FST::from_fst_reader(metadata, fst_compiler.into_fst_reader()?);
        Ok(StemmerOverrideMap::new(fst, self.ignore_case))
    }
}
