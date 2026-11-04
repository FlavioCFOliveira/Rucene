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

//! Port of `org.apache.lucene.codecs.memory.DirectPostingsFormat`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use rucene::codecs::{
    available_postings_formats, postings_for_name, FieldsConsumer, FieldsProducer, PostingsFormat,
    SegmentReadState, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{
    feature_requested, Fields, ImpactsEnum, IndexOptions, OrdTermState, PostingsEnum, SeekStatus,
    SlowImpactsEnum, TermState, Terms, TermsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_POSITIONS,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::Context;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{AutomatonType, CompiledAutomaton, Transition};
use rucene::util::java_collections::java_compare_to;
use rucene::util::{Accountable, ArrayUtil, BytesRef, RamUsageEstimator};

/// Java's `assert cond;` (`assert cond : message;`) under `-ea`: an
/// `AssertionError`, raised in builds with `debug_assertions` (see
/// [`LuceneError::AssertionError`]).
macro_rules! java_assert {
    ($cond:expr) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(LuceneError::AssertionError(String::new()));
        }
    };
    ($cond:expr, $($msg:tt)+) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(LuceneError::AssertionError(format!($($msg)+)));
        }
    };
}

const DEFAULT_MIN_SKIP_COUNT: i32 = 8;
const DEFAULT_LOW_FREQ_CUTOFF: i32 = 32;

/// Wraps `Lucene104PostingsFormat` format for on-disk storage, but then at
/// read time loads and stores all terms and postings directly in RAM as
/// `byte[]`, `int[]`.
///
/// **WARNING**: This is exceptionally RAM intensive: it makes no effort to
/// compress the postings data, storing terms as separate `byte[]` and
/// postings as separate `int[]`, but as a result it gives substantial
/// increase in search performance.
///
/// This postings format supports [`TermsEnum::ord`] and
/// [`TermsEnum::seek_ord`].
///
/// Because this holds all term bytes as a single `byte[]`, you cannot have
/// more than 2.1GB worth of term bytes in a single segment.
///
/// Port of `org.apache.lucene.codecs.memory.DirectPostingsFormat`, named
/// `Direct`. Java does not override `toString()`, so the format prints as
/// every format does, `PostingsFormat(name=Direct)`.
#[derive(Debug, Clone, Copy)]
pub struct DirectPostingsFormat {
    min_skip_count: i32,
    low_freq_cutoff: i32,
}

impl Default for DirectPostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectPostingsFormat {
    /// Port of `DirectPostingsFormat()`: `minSkipCount = 8`,
    /// `lowFreqCutoff = 32`.
    pub fn new() -> Self {
        Self::with_params(DEFAULT_MIN_SKIP_COUNT, DEFAULT_LOW_FREQ_CUTOFF)
    }

    /// Port of `DirectPostingsFormat(int minSkipCount, int lowFreqCutoff)`.
    ///
    /// `min_skip_count` is how many terms in a row must have the same prefix
    /// before we put a skip pointer down. Terms with `docFreq <=
    /// low_freq_cutoff` will use a single `int[]` to hold all docs, freqs,
    /// position and offsets; terms with higher `docFreq` will use separate
    /// arrays.
    pub fn with_params(min_skip_count: i32, low_freq_cutoff: i32) -> Self {
        Self {
            min_skip_count,
            low_freq_cutoff,
        }
    }
}

/// `PostingsFormat.forName(name)`: the format the global registry resolves,
/// or Java's `IllegalArgumentException` (`NamedSPILoader.lookup`,
/// `NamedSPILoader.java:102-114`).
pub(crate) fn postings_format_for_name(name: &str) -> Result<Arc<dyn PostingsFormat>> {
    postings_for_name(name).ok_or_else(|| {
        LuceneError::IllegalArgument(format!(
            "An SPI class of type org.apache.lucene.codecs.PostingsFormat with name '{name}' \
             does not exist.  You need to add the corresponding JAR file supporting this SPI to \
             your classpath.  The current classpath supports the following names: [{}]",
            available_postings_formats().join(", ")
        ))
    })
}

impl PostingsFormat for DirectPostingsFormat {
    fn name(&self) -> &str {
        "Direct"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        postings_format_for_name("Lucene104")?.fields_consumer(state)
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        let mut postings = postings_format_for_name("Lucene104")?.fields_producer(state)?;
        if state.context.context() != Context::Merge {
            let loaded = postings.check_integrity().and_then(|()| {
                DirectFields::new(
                    state,
                    postings.as_ref(),
                    self.min_skip_count,
                    self.low_freq_cutoff,
                )
            });
            // `finally { postings.close(); }`: an exception of `close`
            // replaces the result, whichever it was.
            postings.close()?;
            Ok(Box::new(loaded?))
        } else {
            // Don't load postings for merge:
            Ok(postings)
        }
    }
}

/// Java's `ArrayIndexOutOfBoundsException` for `array[index]`.
fn out_of_bounds(index: i64, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// `array[index]`, or Java's `ArrayIndexOutOfBoundsException`.
fn at<T: Copy>(array: &[T], index: i32) -> Result<T> {
    usize::try_from(index)
        .ok()
        .and_then(|i| array.get(i).copied())
        .ok_or_else(|| out_of_bounds(i64::from(index), array.len()))
}

/// Java's `NullPointerException`.
fn null_pointer(message: &str) -> LuceneError {
    LuceneError::unchecked("java.lang.NullPointerException", message.to_string())
}

/// Port of the private inner class `DirectFields`.
///
/// Java's `TreeMap<String, DirectField>` is a map plus the names in
/// `String.compareTo` order.
struct DirectFields {
    fields: HashMap<String, Arc<DirectField>>,
    field_names: Vec<String>,
}

impl DirectFields {
    fn new(
        state: &SegmentReadState<'_>,
        fields: &dyn Fields,
        min_skip_count: i32,
        low_freq_cutoff: i32,
    ) -> Result<Self> {
        let mut map = HashMap::new();
        for field in fields.iterator() {
            let terms = fields.terms(&field)?;
            let direct = DirectField::new(
                state,
                &field,
                terms.as_deref(),
                min_skip_count,
                low_freq_cutoff,
            )?;
            map.insert(field, Arc::new(direct));
        }
        let mut field_names: Vec<String> = map.keys().cloned().collect();
        field_names.sort_by(|a, b| java_compare_to(a, b));
        Ok(Self {
            fields: map,
            field_names,
        })
    }
}

impl std::fmt::Display for DirectFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DirectFields(fields={})", self.fields.len())
    }
}

impl Fields for DirectFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.field_names.iter().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self
            .fields
            .get(field)
            .map(|t| Box::new(DirectTerms(Arc::clone(t))) as Box<dyn Terms>))
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

impl FieldsProducer for DirectFields {
    fn check_integrity(&self) -> Result<()> {
        // if we read entirely into ram, we already validated.
        // otherwise returned the raw postings reader
        Ok(())
    }

    /// `FieldsProducer.getMergeInstance()`, which `DirectFields` inherits:
    /// `this`, here a second handle on the same loaded fields.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            fields: self.fields.clone(),
            field_names: self.field_names.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Port of `DirectField.LowFreqTerm`: the postings of a term with `docFreq
/// <= lowFreqCutoff`, packed into a single `int[]`.
#[derive(Debug)]
struct LowFreqTerm {
    postings: Vec<i32>,
    payloads: Option<Vec<u8>>,
    doc_freq: i32,
    total_term_freq: i32,
}

impl Accountable for LowFreqTerm {
    fn ram_bytes_used(&self) -> i64 {
        // Java's `BASE_RAM_BYTES_USED` of `LowFreqTerm` measures
        // `HighFreqTerm.class`.
        RamUsageEstimator::shallow_size_of_instance::<HighFreqTerm>()
            + RamUsageEstimator::shallow_size_of_array::<i32>(self.postings.len())
            + self.payloads.as_ref().map_or(0, |p| {
                RamUsageEstimator::shallow_size_of_array::<u8>(p.len())
            })
    }
}

/// Port of `DirectField.HighFreqTerm`: the postings of a term with `docFreq
/// > lowFreqCutoff`, in separate arrays.
#[derive(Debug)]
struct HighFreqTerm {
    total_term_freq: i64,
    doc_ids: Vec<i32>,
    freqs: Option<Vec<i32>>,
    positions: Option<Vec<Vec<i32>>>,
    payloads: Option<Vec<Vec<Option<Vec<u8>>>>>,
}

impl Accountable for HighFreqTerm {
    fn ram_bytes_used(&self) -> i64 {
        let mut size_in_bytes = RamUsageEstimator::shallow_size_of_instance::<HighFreqTerm>();
        size_in_bytes += RamUsageEstimator::shallow_size_of_array::<i32>(self.doc_ids.len());
        size_in_bytes += self.freqs.as_ref().map_or(0, |f| {
            RamUsageEstimator::shallow_size_of_array::<i32>(f.len())
        });
        if let Some(positions) = &self.positions {
            size_in_bytes += RamUsageEstimator::shallow_size_of(positions);
            for position in positions {
                size_in_bytes += RamUsageEstimator::shallow_size_of_array::<i32>(position.len());
            }
        }
        if let Some(payloads) = &self.payloads {
            size_in_bytes += RamUsageEstimator::shallow_size_of(payloads);
            for payload in payloads {
                size_in_bytes += RamUsageEstimator::shallow_size_of(payload);
                for pload in payload.iter().flatten() {
                    size_in_bytes += RamUsageEstimator::shallow_size_of_array::<u8>(pload.len());
                }
            }
        }
        size_in_bytes
    }
}

/// Port of the abstract `DirectField.TermAndSkip`, without its `skips`,
/// which only exist while the field loads (see `DirectFieldBuilder`).
#[derive(Debug, Clone)]
enum TermAndSkip {
    Low(Arc<LowFreqTerm>),
    High(Arc<HighFreqTerm>),
}

impl TermAndSkip {
    fn doc_freq(&self) -> i32 {
        match self {
            TermAndSkip::Low(t) => t.doc_freq,
            TermAndSkip::High(t) => t.doc_ids.len() as i32,
        }
    }

    fn total_term_freq(&self) -> i64 {
        match self {
            TermAndSkip::Low(t) => i64::from(t.total_term_freq),
            TermAndSkip::High(t) => t.total_term_freq,
        }
    }

    fn ram_bytes_used(&self) -> i64 {
        match self {
            TermAndSkip::Low(t) => t.ram_bytes_used(),
            TermAndSkip::High(t) => t.ram_bytes_used(),
        }
    }
}

/// Port of the private inner class `DirectField`: one field's terms and
/// postings, all in RAM.
#[derive(Debug)]
struct DirectField {
    term_bytes: Vec<u8>,
    term_offsets: Vec<i32>,

    skips: Vec<i32>,
    skip_offsets: Vec<i32>,

    terms: Vec<TermAndSkip>,
    has_freq: bool,
    has_pos: bool,
    has_offsets: bool,
    has_payloads: bool,
    sum_total_term_freq: i64,
    doc_count: i32,
    sum_doc_freq: i64,
    /// The length of Java's `sameCounts` once the field is loaded, which
    /// `ramBytesUsed` charges.
    same_counts_len: usize,
    min_skip_count: i32,
}

/// Port of `DirectField.IntArrayWriter`.
struct IntArrayWriter {
    ints: Vec<i32>,
}

impl IntArrayWriter {
    fn add(&mut self, value: i32) {
        self.ints.push(value);
    }

    fn get(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.ints)
    }
}

/// The state of `DirectField`'s constructor that Java keeps in the field
/// itself ("TODO: maybe make a separate builder? These are only used during
/// load"): `count`, `sameCounts`, `skipCount` and each term's `skips`.
struct DirectFieldBuilder {
    term_offsets: Vec<i32>,
    skips: Vec<Option<Vec<i32>>>,
    count: i32,
    same_counts: Vec<i32>,
    skip_count: i32,
    min_skip_count: i32,
}

impl DirectFieldBuilder {
    /// Port of `setSkips(int termOrd, byte[] termBytes)`.
    fn set_skips(&mut self, term_ord: usize, term_bytes: &[u8]) {
        let term_length = (self.term_offsets[term_ord + 1] - self.term_offsets[term_ord]) as usize;

        if self.same_counts.len() < term_length {
            // `ArrayUtil.grow(sameCounts, termLength)`
            self.same_counts.resize(
                ArrayUtil::oversize(term_length, std::mem::size_of::<i32>()),
                0,
            );
        }

        // Update skip pointers:
        if term_ord > 0 {
            let last_term_length =
                (self.term_offsets[term_ord] - self.term_offsets[term_ord - 1]) as usize;
            let limit = term_length.min(last_term_length);

            let mut last_term_offset = self.term_offsets[term_ord - 1] as usize;
            let mut term_offset = self.term_offsets[term_ord] as usize;

            let mut i = 0;
            while i < limit {
                let same = term_bytes[last_term_offset] == term_bytes[term_offset];
                last_term_offset += 1;
                term_offset += 1;
                if same {
                    self.same_counts[i] += 1;
                } else {
                    while i < limit {
                        if self.same_counts[i] >= self.min_skip_count {
                            // Go back and add a skip pointer:
                            self.save_skip(term_ord as i32, self.same_counts[i]);
                        }
                        self.same_counts[i] = 1;
                        i += 1;
                    }
                    break;
                }
                i += 1;
            }

            while i < last_term_length {
                if self.same_counts[i] >= self.min_skip_count {
                    // Go back and add a skip pointer:
                    self.save_skip(term_ord as i32, self.same_counts[i]);
                }
                self.same_counts[i] = 0;
                i += 1;
            }
            for j in limit..term_length {
                self.same_counts[j] = 1;
            }
        } else {
            for i in 0..term_length {
                self.same_counts[i] += 1;
            }
        }
    }

    /// Port of `finishSkips()`.
    fn finish_skips(&mut self) -> Result<()> {
        java_assert!(self.count as usize == self.skips.len());
        let last_term_offset = at(&self.term_offsets, self.count - 1)?;
        let last_term_length = self.term_offsets[self.count as usize] - last_term_offset;

        for i in 0..last_term_length as usize {
            if self.same_counts[i] >= self.min_skip_count {
                // Go back and add a skip pointer:
                self.save_skip(self.count, self.same_counts[i]);
            }
        }

        // Reverse the skip pointers so they are "nested":
        for skips in self.skips.iter_mut().flatten() {
            if skips.len() > 1 {
                skips.reverse();
            }
        }
        Ok(())
    }

    /// Port of `saveSkip(int ord, int backCount)`.
    fn save_skip(&mut self, ord: i32, back_count: i32) {
        let term = &mut self.skips[(ord - back_count) as usize];
        self.skip_count += 1;
        // Normally we'd grow at a slight exponential... but
        // given that the skips themselves are already log(N)
        // we can grow by only 1 and still have amortized
        // linear time:
        term.get_or_insert_with(Vec::new).push(ord);
    }
}

impl DirectField {
    /// Port of `DirectField(SegmentReadState, String, Terms, int, int)`.
    fn new(
        state: &SegmentReadState<'_>,
        field: &str,
        terms_in: Option<&dyn Terms>,
        min_skip_count: i32,
        low_freq_cutoff: i32,
    ) -> Result<Self> {
        let field_info = state.field_infos.field_info(field);

        let terms_in = terms_in.ok_or_else(|| {
            null_pointer(
                "Cannot invoke \"org.apache.lucene.index.Terms.getSumTotalTermFreq()\" because \
                 \"termsIn\" is null",
            )
        })?;
        let sum_total_term_freq = terms_in.sum_total_term_freq()?;
        let sum_doc_freq = terms_in.sum_doc_freq()?;
        let doc_count = terms_in.doc_count()?;

        let num_terms = terms_in.size()? as i32;
        if num_terms == -1 {
            return Err(LuceneError::IllegalArgument(
                "codec does not provide Terms.size()".to_string(),
            ));
        }
        let num_terms = num_terms as usize;
        let mut terms: Vec<TermAndSkip> = Vec::with_capacity(num_terms);
        let mut builder = DirectFieldBuilder {
            term_offsets: vec![0; 1 + num_terms],
            skips: Vec::with_capacity(num_terms),
            count: 0,
            same_counts: vec![0; 10],
            skip_count: 0,
            min_skip_count,
        };

        let mut term_bytes: Vec<u8> = Vec::with_capacity(1024);

        let field_info = field_info.ok_or_else(|| {
            null_pointer(
                "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" because \
                 \"fieldInfo\" is null",
            )
        })?;
        let index_options = field_info.get_index_options();
        let has_freq = index_options.subsumes(IndexOptions::DOCS_AND_FREQS);
        let has_pos = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        let has_offsets =
            index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        let has_payloads = field_info.has_payloads();

        let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;
        let mut docs_and_positions_enum: Option<Box<dyn PostingsEnum>> = None;
        let mut terms_enum = terms_in.iterator()?;
        let mut term_offset = 0i32;

        let mut scratch = IntArrayWriter { ints: Vec::new() };

        // Used for payloads, if any:
        let mut ros: Vec<u8> = Vec::new();

        while let Some(term) = terms_enum.next()? {
            let doc_freq = terms_enum.doc_freq()?;
            let total_term_freq = terms_enum.total_term_freq()?;

            let count = builder.count as usize;
            builder.term_offsets[count] = term_offset;

            term_bytes.extend_from_slice(term.slice());
            term_offset += term.slice().len() as i32;
            builder.term_offsets[count + 1] = term_offset;

            let postings_enum2: &mut Box<dyn PostingsEnum> = if has_pos {
                let reuse = docs_and_positions_enum.take();
                docs_and_positions_enum.insert(terms_enum.postings(reuse, POSTINGS_ENUM_ALL)?)
            } else {
                let reuse = postings_enum.take();
                postings_enum.insert(terms_enum.postings(reuse, POSTINGS_ENUM_FREQS)?)
            };

            let ent = if doc_freq <= low_freq_cutoff {
                ros.clear();

                // Pack postings for low-freq terms into a single int[]:
                loop {
                    let doc_id = postings_enum2.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    scratch.add(doc_id);
                    if has_freq {
                        let freq = postings_enum2.freq()?;
                        scratch.add(freq);
                        if has_pos {
                            for _ in 0..freq {
                                scratch.add(postings_enum2.next_position()?);
                                if has_offsets {
                                    scratch.add(postings_enum2.start_offset());
                                    scratch.add(postings_enum2.end_offset());
                                }
                                if has_payloads {
                                    match postings_enum2.get_payload()? {
                                        Some(payload) => {
                                            scratch.add(payload.len() as i32);
                                            ros.extend_from_slice(payload);
                                        }
                                        None => scratch.add(0),
                                    }
                                }
                            }
                        }
                    }
                }

                let payloads = has_payloads.then(|| ros.clone());
                let postings = scratch.get();

                TermAndSkip::Low(Arc::new(LowFreqTerm {
                    postings,
                    payloads,
                    doc_freq,
                    total_term_freq: total_term_freq as i32,
                }))
            } else {
                let df = doc_freq as usize;
                let mut docs = vec![0i32; df];
                let mut freqs: Option<Vec<i32>> = None;
                let mut positions: Option<Vec<Vec<i32>>> = None;
                let mut payloads: Option<Vec<Vec<Option<Vec<u8>>>>> = None;
                if has_freq {
                    freqs = Some(vec![0; df]);
                    if has_pos {
                        positions = Some(vec![Vec::new(); df]);
                        if has_payloads {
                            payloads = Some(vec![Vec::new(); df]);
                        }
                    }
                }

                // Use separate int[] for the postings for high-freq
                // terms:
                let mut upto = 0usize;
                loop {
                    let doc_id = postings_enum2.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    *docs
                        .get_mut(upto)
                        .ok_or_else(|| out_of_bounds(upto as i64, df))? = doc_id;
                    if let Some(freqs) = freqs.as_mut() {
                        let freq = postings_enum2.freq()?;
                        freqs[upto] = freq;
                        if let Some(positions) = positions.as_mut() {
                            let mult = if has_offsets { 3 } else { 1 };
                            if let Some(payloads) = payloads.as_mut() {
                                payloads[upto] = vec![None; freq as usize];
                            }
                            let mut doc_positions = vec![0i32; mult * freq as usize];
                            let mut pos_upto = 0usize;
                            for pos in 0..freq as usize {
                                doc_positions[pos_upto] = postings_enum2.next_position()?;
                                if let Some(payloads) = payloads.as_mut() {
                                    if let Some(payload) = postings_enum2.get_payload()? {
                                        payloads[upto][pos] = Some(payload.to_vec());
                                    }
                                }
                                pos_upto += 1;
                                if has_offsets {
                                    doc_positions[pos_upto] = postings_enum2.start_offset();
                                    pos_upto += 1;
                                    doc_positions[pos_upto] = postings_enum2.end_offset();
                                    pos_upto += 1;
                                }
                            }
                            positions[upto] = doc_positions;
                        }
                    }

                    upto += 1;
                }
                java_assert!(upto == df);
                TermAndSkip::High(Arc::new(HighFreqTerm {
                    total_term_freq,
                    doc_ids: docs,
                    freqs,
                    positions,
                    payloads,
                }))
            };

            terms.push(ent);
            builder.skips.push(None);
            builder.set_skips(count, &term_bytes);
            builder.count += 1;
        }

        // End sentinel:
        builder.term_offsets[builder.count as usize] = term_offset;

        builder.finish_skips()?;

        // Pack skips:
        let mut skips = Vec::with_capacity(builder.skip_count as usize);
        let mut skip_offsets = vec![0i32; 1 + num_terms];

        let mut skip_offset = 0i32;
        for (i, term_skips) in builder.skips.iter().enumerate() {
            skip_offsets[i] = skip_offset;
            if let Some(term_skips) = term_skips {
                skips.extend_from_slice(term_skips);
                skip_offset += term_skips.len() as i32;
            }
        }
        skip_offsets[num_terms] = skip_offset;
        java_assert!(skip_offset == builder.skip_count);

        Ok(Self {
            term_bytes,
            term_offsets: builder.term_offsets,
            skips,
            skip_offsets,
            terms,
            has_freq,
            has_pos,
            has_offsets,
            has_payloads,
            sum_total_term_freq,
            doc_count,
            sum_doc_freq,
            same_counts_len: builder.same_counts.len(),
            min_skip_count,
        })
    }

    /// Compares in unicode (UTF8) order.
    ///
    /// Port of `compare(int ord, BytesRef other)`.
    fn compare(&self, ord: usize, other: &[u8]) -> i32 {
        let mut upto = self.term_offsets[ord] as usize;
        let term_len = self.term_offsets[1 + ord] as usize - upto;
        let mut other_upto = 0usize;

        let stop = upto + term_len.min(other.len());
        while upto < stop {
            let diff = i32::from(self.term_bytes[upto]) - i32::from(other[other_upto]);
            upto += 1;
            other_upto += 1;
            if diff != 0 {
                return diff;
            }
        }

        // One is a prefix of the other, or, they are equal:
        term_len as i32 - other.len() as i32
    }

    /// The bytes of the term at `ord`.
    fn term_slice(&self, ord: usize) -> &[u8] {
        &self.term_bytes[self.term_offsets[ord] as usize..self.term_offsets[ord + 1] as usize]
    }

    /// `terms[termOrd]`, or Java's `ArrayIndexOutOfBoundsException`.
    fn term(&self, term_ord: i32) -> Result<&TermAndSkip> {
        usize::try_from(term_ord)
            .ok()
            .and_then(|i| self.terms.get(i))
            .ok_or_else(|| out_of_bounds(i64::from(term_ord), self.terms.len()))
    }
}

impl Accountable for DirectField {
    fn ram_bytes_used(&self) -> i64 {
        let mut size_in_bytes = RamUsageEstimator::shallow_size_of_instance::<DirectField>();
        size_in_bytes += RamUsageEstimator::shallow_size_of_array::<u8>(self.term_bytes.len());
        size_in_bytes += RamUsageEstimator::shallow_size_of_array::<i32>(self.term_offsets.len());
        size_in_bytes += RamUsageEstimator::shallow_size_of_array::<i32>(self.skips.len());
        size_in_bytes += RamUsageEstimator::shallow_size_of_array::<i32>(self.skip_offsets.len());
        size_in_bytes += RamUsageEstimator::shallow_size_of_array::<i32>(self.same_counts_len);
        size_in_bytes += RamUsageEstimator::shallow_size_of(&self.terms);
        for term_and_skip in &self.terms {
            size_in_bytes += term_and_skip.ram_bytes_used();
        }
        size_in_bytes
    }
}

impl std::fmt::Display for DirectField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DirectTerms(terms={},postings={},positions={},docs={})",
            self.terms.len(),
            self.sum_doc_freq,
            self.sum_total_term_freq,
            self.doc_count
        )
    }
}

/// The `Terms` the fields hand out: the `DirectField` itself.
struct DirectTerms(Arc<DirectField>);

impl std::fmt::Display for DirectTerms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&*self.0, f)
    }
}

impl Terms for DirectTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(DirectTermsEnum::new(Arc::clone(&self.0))))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        if compiled.automaton_type != AutomatonType::Normal {
            return Err(LuceneError::IllegalArgument(
                "please use CompiledAutomaton.getTermsEnum instead".to_string(),
            ));
        }
        Ok(Box::new(DirectIntersectTermsEnum::new(
            Arc::clone(&self.0),
            compiled.clone(),
            start_term,
        )?))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.0.terms.len() as i64)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.0.sum_total_term_freq)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.0.sum_doc_freq)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.0.doc_count)
    }

    fn has_freqs(&self) -> bool {
        self.0.has_freq
    }

    fn has_offsets(&self) -> bool {
        self.0.has_offsets
    }

    fn has_positions(&self) -> bool {
        self.0.has_pos
    }

    fn has_payloads(&self) -> bool {
        self.0.has_payloads
    }
}

/// `reuse instanceof T`, then the reused enum.
fn reusable<T: 'static>(reuse: Option<Box<dyn PostingsEnum>>) -> Option<Box<dyn PostingsEnum>> {
    let mut reuse = reuse?;
    let ok = reuse
        .as_any_mut()
        .is_some_and(|any| any.downcast_mut::<T>().is_some());
    ok.then_some(reuse)
}

/// The concrete enum inside a box [`reusable`] accepted.
fn concrete<T: 'static>(e: &mut Box<dyn PostingsEnum>) -> &mut T {
    e.as_any_mut()
        .and_then(|any| any.downcast_mut::<T>())
        .expect("INVARIANT: `reusable` checked the enum's type")
}

/// `reuse instanceof T ? (T) reuse : new T()`, then `reset(...)`.
fn reuse_or_new<T: PostingsEnum + 'static>(
    reuse: Option<Box<dyn PostingsEnum>>,
    can_reuse: impl Fn(&T) -> bool,
    create: impl FnOnce() -> T,
    reset: impl FnOnce(&mut T),
) -> Box<dyn PostingsEnum> {
    let reused = reusable::<T>(reuse).and_then(|mut e| {
        let ok = can_reuse(concrete::<T>(&mut e));
        ok.then_some(e)
    });
    match reused {
        Some(mut e) => {
            reset(concrete::<T>(&mut e));
            e
        }
        None => {
            let mut e = create();
            reset(&mut e);
            Box::new(e)
        }
    }
}

/// Port of `DirectTermsEnum.postings(PostingsEnum, int)`.
fn direct_postings(
    field: &DirectField,
    term: &TermAndSkip,
    reuse: Option<Box<dyn PostingsEnum>>,
    flags: i32,
) -> Box<dyn PostingsEnum> {
    // TODO: implement reuse
    // it's hairy!

    // TODO: the logic of which enum impl to choose should be refactored to be simpler...
    if feature_requested(flags, POSTINGS_ENUM_POSITIONS) {
        match term {
            TermAndSkip::Low(term) => {
                if !field.has_freq {
                    return reuse_or_new(
                        reuse,
                        |_: &LowFreqDocsEnumNoTF| true,
                        LowFreqDocsEnumNoTF::new,
                        |e| e.reset(Arc::clone(term)),
                    );
                } else if !field.has_pos {
                    return reuse_or_new(
                        reuse,
                        |_: &LowFreqDocsEnumNoPos| true,
                        LowFreqDocsEnumNoPos::new,
                        |e| e.reset(Arc::clone(term)),
                    );
                }
                let mut e = LowFreqPostingsEnum::new(field.has_offsets, field.has_payloads);
                e.reset(Arc::clone(term));
                return Box::new(e);
            }
            TermAndSkip::High(term) => {
                if !field.has_pos {
                    let mut e = HighFreqDocsEnum::new();
                    e.reset(Arc::clone(term));
                    return Box::new(e);
                } else {
                    let mut e = HighFreqPostingsEnum::new(field.has_offsets);
                    e.reset(Arc::clone(term));
                    return Box::new(e);
                }
            }
        }
    }

    match term {
        TermAndSkip::Low(term) => {
            if field.has_freq {
                if field.has_pos {
                    let mut pos_len = if field.has_offsets { 3 } else { 1 };
                    if field.has_payloads {
                        pos_len += 1;
                    }
                    reuse_or_new(
                        reuse,
                        |e: &LowFreqDocsEnum| e.can_reuse(pos_len),
                        || LowFreqDocsEnum::new(pos_len),
                        |e| e.reset(Arc::clone(term)),
                    )
                } else {
                    reuse_or_new(
                        reuse,
                        |_: &LowFreqDocsEnumNoPos| true,
                        LowFreqDocsEnumNoPos::new,
                        |e| e.reset(Arc::clone(term)),
                    )
                }
            } else {
                reuse_or_new(
                    reuse,
                    |_: &LowFreqDocsEnumNoTF| true,
                    LowFreqDocsEnumNoTF::new,
                    |e| e.reset(Arc::clone(term)),
                )
            }
        }
        TermAndSkip::High(term) => reuse_or_new(
            reuse,
            |_: &HighFreqDocsEnum| true,
            HighFreqDocsEnum::new,
            |e| e.reset(Arc::clone(term)),
        ),
    }
}

/// Port of `DirectIntersectTermsEnum.postings(PostingsEnum, int)`, which
/// never reuses.
fn intersect_postings(
    field: &DirectField,
    term: &TermAndSkip,
    flags: i32,
) -> Box<dyn PostingsEnum> {
    // TODO: implement reuse
    // it's hairy!

    // TODO: the logic of which enum impl to choose should be refactored to be simpler...
    if field.has_pos && feature_requested(flags, POSTINGS_ENUM_POSITIONS) {
        return match term {
            TermAndSkip::Low(term) => {
                let mut e = LowFreqPostingsEnum::new(field.has_offsets, field.has_payloads);
                e.reset(Arc::clone(term));
                Box::new(e)
            }
            TermAndSkip::High(term) => {
                let mut e = HighFreqPostingsEnum::new(field.has_offsets);
                e.reset(Arc::clone(term));
                Box::new(e)
            }
        };
    }

    match term {
        TermAndSkip::Low(term) => {
            if field.has_freq {
                if field.has_pos {
                    let mut pos_len = if field.has_offsets { 3 } else { 1 };
                    if field.has_payloads {
                        pos_len += 1;
                    }
                    let mut e = LowFreqDocsEnum::new(pos_len);
                    e.reset(Arc::clone(term));
                    Box::new(e)
                } else {
                    let mut e = LowFreqDocsEnumNoPos::new();
                    e.reset(Arc::clone(term));
                    Box::new(e)
                }
            } else {
                let mut e = LowFreqDocsEnumNoTF::new();
                e.reset(Arc::clone(term));
                Box::new(e)
            }
        }
        TermAndSkip::High(term) => {
            let mut e = HighFreqDocsEnum::new();
            e.reset(Arc::clone(term));
            Box::new(e)
        }
    }
}

/// Java's `scratch` `BytesRef` of the two enums: `(offset, length)` into
/// `termBytes`, or `None` while it is still `new BytesRef()`.
type Scratch = Option<(usize, usize)>;

fn scratch_term(field: &DirectField, scratch: Scratch) -> BytesRef {
    match scratch {
        None => BytesRef::default(),
        Some((offset, length)) => BytesRef::new(field.term_bytes[offset..offset + length].to_vec()),
    }
}

/// `seekExact(term, state)`'s `(OrdTermState) state`, or Java's
/// `ClassCastException`.
fn ord_term_state(state: &dyn TermState) -> Result<i64> {
    state
        .as_any()
        .downcast_ref::<OrdTermState>()
        .map(|s| s.ord)
        .ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ClassCastException",
                "class cannot be cast to class org.apache.lucene.index.OrdTermState",
            )
        })
}

/// Port of the private inner class `DirectField.DirectTermsEnum`.
struct DirectTermsEnum {
    field: Arc<DirectField>,
    attributes: Option<AttributeSource>,
    scratch: Scratch,
    term_ord: i32,
}

impl std::fmt::Display for DirectTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.memory.DirectPostingsFormat$DirectField$DirectTermsEnum",
            self,
        ))
    }
}

impl DirectTermsEnum {
    fn new(field: Arc<DirectField>) -> Self {
        Self {
            field,
            attributes: None,
            scratch: None,
            term_ord: -1,
        }
    }

    /// Port of `setTerm()`: Java assigns `scratch.offset` before it reads
    /// `termOffsets[termOrd + 1]`, so a failing read leaves the new offset.
    fn set_term(&mut self) -> Result<BytesRef> {
        let offsets = &self.field.term_offsets;
        let offset = at(offsets, self.term_ord)?;
        let length = self.scratch.map_or(0, |(_, l)| l);
        self.scratch = Some((offset as usize, length));
        let end = at(offsets, self.term_ord + 1)?;
        self.scratch = Some((offset as usize, (end - offset) as usize));
        Ok(scratch_term(&self.field, self.scratch))
    }

    /// If non-negative, exact match; else, `-ord-1`, where ord is where you
    /// would insert the term.
    ///
    /// Port of `findTerm(BytesRef)`.
    fn find_term(&self, term: &[u8]) -> i32 {
        // Just do binary search: should be (constant factor)
        // faster than using the skip list:
        let mut low = 0i32;
        let mut high = self.field.terms.len() as i32 - 1;

        while low <= high {
            let mid = ((low + high) as u32 >> 1) as i32;
            let cmp = self.field.compare(mid as usize, term);
            if cmp < 0 {
                low = mid + 1;
            } else if cmp > 0 {
                high = mid - 1;
            } else {
                return mid; // key found
            }
        }

        -(low + 1) // key not found.
    }
}

impl TermsEnum for DirectTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.attributes.get_or_insert_with(AttributeSource::new)
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.term_ord += 1;
        if (self.term_ord as usize) < self.field.terms.len() {
            Ok(Some(self.set_term()?))
        } else {
            Ok(None)
        }
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        let mut state = OrdTermState::new();
        state.ord = i64::from(self.term_ord);
        Ok(Box::new(state))
    }

    fn seek_ceil(&mut self, term: &BytesRef) -> Result<SeekStatus> {
        // TODO: we should use the skip pointers; should be
        // faster than bin search; we should also hold
        // & reuse current state so seeking forwards is
        // faster
        let ord = self.find_term(term.slice());
        if ord >= 0 {
            self.term_ord = ord;
            self.set_term()?;
            Ok(SeekStatus::FOUND)
        } else if ord == -(self.field.terms.len() as i32) - 1 {
            Ok(SeekStatus::END)
        } else {
            self.term_ord = -ord - 1;
            self.set_term()?;
            Ok(SeekStatus::NOT_FOUND)
        }
    }

    fn seek_exact(&mut self, term: &BytesRef) -> Result<bool> {
        // TODO: we should use the skip pointers; should be
        // faster than bin search; we should also hold
        // & reuse current state so seeking forwards is
        // faster
        let ord = self.find_term(term.slice());
        if ord >= 0 {
            self.term_ord = ord;
            self.set_term()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.term_ord = ord as i32;
        self.set_term()?;
        Ok(())
    }

    fn seek_term_state(&mut self, term: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.term_ord = ord_term_state(state)? as i32;
        let set = self.set_term()?;
        java_assert!(*term == set);
        Ok(())
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(scratch_term(&self.field, self.scratch))
    }

    fn ord(&self) -> Result<i64> {
        Ok(i64::from(self.term_ord))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(self.field.term(self.term_ord)?.doc_freq())
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Ok(self.field.term(self.term_ord)?.total_term_freq())
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let term = self.field.term(self.term_ord)?;
        Ok(direct_postings(&self.field, term, reuse, flags))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Ok(Box::new(SlowImpactsEnum::new(self.postings(None, flags)?)))
    }
}

/// Port of `DirectIntersectTermsEnum.State`.
#[derive(Clone, Default)]
struct State {
    change_ord: i32,
    state: i32,
    transition_upto: i32,
    transition_count: i32,
    transition_max: i32,
    transition_min: i32,
    transition: Transition,
}

/// Port of the private inner class `DirectField.DirectIntersectTermsEnum`.
struct DirectIntersectTermsEnum {
    field: Arc<DirectField>,
    attributes: Option<AttributeSource>,
    /// Holds Java's `byteRunnable`, `transitionAccessor` and
    /// `commonSuffixRef`, all of which are views of it.
    compiled: CompiledAutomaton,
    term_ord: i32,
    scratch: Scratch,
    states: Vec<State>,
    state_upto: i32,
}

impl std::fmt::Display for DirectIntersectTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.memory.DirectPostingsFormat$DirectField$\
             DirectIntersectTermsEnum",
            self,
        ))
    }
}

impl DirectIntersectTermsEnum {
    /// Port of `DirectIntersectTermsEnum(CompiledAutomaton, BytesRef)`.
    fn new(
        field: Arc<DirectField>,
        compiled: CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Self> {
        if compiled.get_byte_runnable().is_none() || compiled.get_transition_accessor().is_none() {
            return Err(null_pointer(
                "Cannot invoke \"org.apache.lucene.util.automaton.TransitionAccessor.\
                 getNumTransitions(int)\" because \"this.transitionAccessor\" is null",
            ));
        }
        let mut e = Self {
            field,
            attributes: None,
            compiled,
            term_ord: -1,
            scratch: None,
            states: vec![State::default()],
            state_upto: 0,
        };
        let ta = e
            .compiled
            .get_transition_accessor()
            .expect("INVARIANT: checked above");
        e.states[0].change_ord = e.field.terms.len() as i32;
        e.states[0].state = 0;
        e.states[0].transition_count = ta.get_num_transitions(e.states[0].state);
        ta.init_transition(e.states[0].state, &mut e.states[0].transition);
        e.states[0].transition_upto = -1;
        e.states[0].transition_max = -1;

        if let Some(start_term) = start_term {
            e.seek_start(start_term.slice())?;
        }
        Ok(e)
    }

    /// The `startTerm != null` block of the constructor, whose `return`s
    /// leave the constructor.
    fn seek_start(&mut self, start_term: &[u8]) -> Result<()> {
        let f = &*self.field;
        let ra = self
            .compiled
            .get_byte_runnable()
            .expect("INVARIANT: the constructor checked it");
        let ta = self
            .compiled
            .get_transition_accessor()
            .expect("INVARIANT: the constructor checked it");
        let num_terms = f.terms.len() as i32;
        let mut skip_upto = 0i32;
        if start_term.is_empty() {
            if num_terms > 0 && f.term_offsets[1] == 0 {
                self.term_ord = 0;
            }
        } else {
            self.term_ord += 1;

            'next_label: for i in 0..start_term.len() {
                let label = i32::from(start_term[i]);

                while label > self.states[i].transition_max {
                    let s = &mut self.states[i];
                    s.transition_upto += 1;
                    if s.transition_upto >= s.transition_count {
                        // All transitions compare less than the required label
                        break;
                    }
                    ta.get_next_transition(&mut s.transition);
                    s.transition_min = s.transition.min;
                    s.transition_max = s.transition.max;
                    java_assert!(s.transition_min >= 0);
                    java_assert!(s.transition_min <= 255);
                    java_assert!(s.transition_max >= 0);
                    java_assert!(s.transition_max <= 255);
                }

                // Skip forwards until we find a term matching
                // the label at this position:
                while self.term_ord < num_terms {
                    let term_ord = self.term_ord as usize;
                    let skip_offset = f.skip_offsets[term_ord];
                    let num_skips = f.skip_offsets[term_ord + 1] - skip_offset;
                    let term_offset = f.term_offsets[term_ord] as usize;
                    let term_length = f.term_offsets[1 + term_ord] as usize - term_offset;

                    if self.term_ord == self.states[self.state_upto as usize].change_ord {
                        self.state_upto -= 1;
                        self.term_ord -= 1;
                        return Ok(());
                    }

                    if term_length == i {
                        self.term_ord += 1;
                        skip_upto = 0;
                    } else if label < i32::from(f.term_bytes[term_offset + i]) {
                        self.term_ord -= 1;
                        self.state_upto -= skip_upto;
                        java_assert!(self.state_upto >= 0);
                        return Ok(());
                    } else if label == i32::from(f.term_bytes[term_offset + i]) {
                        if skip_upto < num_skips {
                            grow(&mut self.states, self.state_upto);

                            let next_state =
                                ra.step(self.states[self.state_upto as usize].state, label);

                            // Automaton is required to accept startTerm:
                            java_assert!(next_state != -1);

                            self.state_upto += 1;
                            let s = &mut self.states[self.state_upto as usize];
                            s.change_ord = f.skips[(skip_offset + skip_upto) as usize];
                            skip_upto += 1;
                            s.state = next_state;
                            s.transition_count = ta.get_num_transitions(next_state);
                            ta.init_transition(s.state, &mut s.transition);
                            s.transition_upto = -1;
                            s.transition_max = -1;

                            // Match next label at this same term:
                            continue 'next_label;
                        } else {
                            // Index exhausted: just scan now (the
                            // number of scans required will be less
                            // than the minSkipCount):
                            let start_term_ord = self.term_ord;
                            while self.term_ord < num_terms
                                && f.compare(self.term_ord as usize, start_term) <= 0
                            {
                                java_assert!(
                                    self.term_ord == start_term_ord
                                        || f.skip_offsets[self.term_ord as usize]
                                            == f.skip_offsets[self.term_ord as usize + 1]
                                );
                                self.term_ord += 1;
                            }
                            java_assert!(self.term_ord - start_term_ord < f.min_skip_count);
                            self.term_ord -= 1;
                            self.state_upto -= skip_upto;
                            return Ok(());
                        }
                    } else {
                        if skip_upto < num_skips {
                            self.term_ord = f.skips[(skip_offset + skip_upto) as usize];
                        } else {
                            self.term_ord += 1;
                        }
                        skip_upto = 0;
                    }
                }

                // startTerm is >= last term so enum will not
                // return any terms:
                self.term_ord -= 1;
                return Ok(());
            }
        }

        if self.term_ord >= 0 {
            let term = f.term_slice(self.term_ord as usize);
            if start_term != term {
                self.state_upto -= skip_upto;
                self.term_ord -= 1;
            }
        }
        Ok(())
    }

    /// The term at `termOrd`, set into `scratch`.
    fn set_scratch(field: &DirectField, scratch: &mut Scratch, term_ord: i32) -> BytesRef {
        let offset = field.term_offsets[term_ord as usize] as usize;
        let length = field.term_offsets[1 + term_ord as usize] as usize - offset;
        *scratch = Some((offset, length));
        scratch_term(field, *scratch)
    }

    /// Port of `next()`.
    fn do_next(&mut self) -> Result<Option<BytesRef>> {
        let field = Arc::clone(&self.field);
        let f = &*field;
        let ra = self
            .compiled
            .get_byte_runnable()
            .expect("INVARIANT: the constructor checked it");
        let ta = self
            .compiled
            .get_transition_accessor()
            .expect("INVARIANT: the constructor checked it");
        let num_terms = f.terms.len() as i32;

        self.term_ord += 1;
        let mut skip_upto = 0i32;

        if self.term_ord == 0 && f.term_offsets[1] == 0 {
            // Special-case empty string:
            java_assert!(self.state_upto == 0);
            if ra.is_accept(self.states[0].state) {
                self.scratch = Some((0, 0));
                return Ok(Some(BytesRef::default()));
            }
            self.term_ord += 1;
        }

        'next_term: loop {
            if self.term_ord == num_terms {
                return Ok(None);
            }

            let su = self.state_upto as usize;
            if self.term_ord == self.states[su].change_ord {
                // Pop:
                self.state_upto -= 1;
                continue;
            }

            let term_ord = self.term_ord as usize;
            let term_offset = f.term_offsets[term_ord] as usize;
            let term_length = f.term_offsets[term_ord + 1] as usize - term_offset;
            let skip_offset = f.skip_offsets[term_ord];
            let num_skips = f.skip_offsets[term_ord + 1] - skip_offset;

            java_assert!(self.term_ord < self.states[su].change_ord);

            java_assert!(
                su <= term_length,
                "term.length={term_length}; stateUpto={su}"
            );
            let label = i32::from(f.term_bytes[term_offset + su]);

            while label > self.states[su].transition_max {
                let state = &mut self.states[su];
                state.transition_upto += 1;
                if state.transition_upto == state.transition_count {
                    // We've exhausted transitions leaving this
                    // state; force pop+next/skip now:
                    if self.state_upto == 0 {
                        self.term_ord = num_terms;
                        return Ok(None);
                    } else {
                        java_assert!(state.change_ord > self.term_ord);
                        self.term_ord = state.change_ord;
                        skip_upto = 0;
                        self.state_upto -= 1;
                    }
                    continue 'next_term;
                }
                // `TransitionAccessor.getNextTransition` asserts that a
                // transition is left (`Automaton.java:488-490`); a state the
                // constructor already ran past its last transition reaches
                // here with `transitionUpto > transitionCount`. The port's
                // accessor cannot return Java's `AssertionError`, so the
                // check runs here, before the call.
                java_assert!(state.transition_upto < state.transition_count);
                ta.get_next_transition(&mut state.transition);
                java_assert!(
                    state.transition_upto < state.transition_count,
                    " state.transitionUpto={} vs {}",
                    state.transition_upto,
                    state.transition_count
                );
                state.transition_min = state.transition.min;
                state.transition_max = state.transition.max;
                java_assert!(state.transition_min >= 0);
                java_assert!(state.transition_min <= 255);
                java_assert!(state.transition_max >= 0);
                java_assert!(state.transition_max <= 255);
            }

            let target_label = self.states[su].transition_min;

            if i32::from(f.term_bytes[term_offset + su]) < target_label {
                let mut low = self.term_ord + 1;
                let mut high = self.states[su].change_ord - 1;
                loop {
                    if low > high {
                        // Label not found
                        self.term_ord = low;
                        skip_upto = 0;
                        continue 'next_term;
                    }
                    let mid = ((low + high) as u32 >> 1) as i32;
                    let cmp = i32::from(f.term_bytes[f.term_offsets[mid as usize] as usize + su])
                        - target_label;
                    if cmp < 0 {
                        low = mid + 1;
                    } else if cmp > 0 {
                        high = mid - 1;
                    } else {
                        // Label found; walk backwards to first
                        // occurrence:
                        let mut mid = mid;
                        while mid > self.term_ord
                            && i32::from(
                                f.term_bytes[f.term_offsets[(mid - 1) as usize] as usize + su],
                            ) == target_label
                        {
                            mid -= 1;
                        }
                        self.term_ord = mid;
                        skip_upto = 0;
                        continue 'next_term;
                    }
                }
            }

            let mut next_state = ra.step(self.states[su].state, label);

            if next_state == -1 {
                // Skip
                if skip_upto < num_skips {
                    self.term_ord = f.skips[(skip_offset + skip_upto) as usize];
                } else {
                    self.term_ord += 1;
                }
                skip_upto = 0;
            } else if skip_upto < num_skips {
                // Push:
                grow(&mut self.states, self.state_upto);
                self.state_upto += 1;
                let s = &mut self.states[self.state_upto as usize];
                s.state = next_state;
                s.change_ord = f.skips[(skip_offset + skip_upto) as usize];
                skip_upto += 1;
                s.transition_count = ta.get_num_transitions(next_state);
                ta.init_transition(next_state, &mut s.transition);
                s.transition_upto = -1;
                s.transition_max = -1;

                if self.state_upto as usize == term_length {
                    if ra.is_accept(next_state) {
                        return Ok(Some(Self::set_scratch(f, &mut self.scratch, self.term_ord)));
                    } else {
                        self.term_ord += 1;
                        skip_upto = 0;
                    }
                }
            } else {
                // Run the non-indexed tail of this term:

                // TODO: add assert that we don't inc too many times

                if let Some(common_suffix_ref) = &self.compiled.common_suffix_ref {
                    let suffix_bytes = common_suffix_ref.slice();
                    if term_length < suffix_bytes.len() {
                        self.term_ord += 1;
                        skip_upto = 0;
                        continue 'next_term;
                    }
                    let offset = term_offset + term_length - suffix_bytes.len();
                    for (suffix, &b) in suffix_bytes.iter().enumerate() {
                        if f.term_bytes[offset + suffix] != b {
                            self.term_ord += 1;
                            skip_upto = 0;
                            continue 'next_term;
                        }
                    }
                }

                let mut upto = su + 1;
                while upto < term_length {
                    next_state = ra.step(next_state, i32::from(f.term_bytes[term_offset + upto]));
                    if next_state == -1 {
                        self.term_ord += 1;
                        skip_upto = 0;
                        continue 'next_term;
                    }
                    upto += 1;
                }

                if ra.is_accept(next_state) {
                    return Ok(Some(Self::set_scratch(f, &mut self.scratch, self.term_ord)));
                } else {
                    self.term_ord += 1;
                    skip_upto = 0;
                }
            }
        }
    }
}

/// Port of `DirectIntersectTermsEnum.grow()`.
fn grow(states: &mut Vec<State>, state_upto: i32) {
    if states.len() == 1 + state_upto as usize {
        states.push(State::default());
    }
}

fn unsupported() -> LuceneError {
    LuceneError::UnsupportedOperation(String::new())
}

impl TermsEnum for DirectIntersectTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.attributes.get_or_insert_with(AttributeSource::new)
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.do_next()
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        let mut state = OrdTermState::new();
        state.ord = i64::from(self.term_ord);
        Ok(Box::new(state))
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(scratch_term(&self.field, self.scratch))
    }

    fn ord(&self) -> Result<i64> {
        Ok(i64::from(self.term_ord))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(self.field.term(self.term_ord)?.doc_freq())
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Ok(self.field.term(self.term_ord)?.total_term_freq())
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let term = self.field.term(self.term_ord)?;
        Ok(intersect_postings(&self.field, term, flags))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Ok(Box::new(SlowImpactsEnum::new(self.postings(None, flags)?)))
    }

    fn seek_ceil(&mut self, _term: &BytesRef) -> Result<SeekStatus> {
        Err(unsupported())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }
}

macro_rules! java_display {
    ($ty:ty, $class:literal) => {
        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&java_object_to_string(
                    concat!(
                        "org.apache.lucene.codecs.memory.DirectPostingsFormat$",
                        $class
                    ),
                    self,
                ))
            }
        }
    };
}

/// Docs only.
///
/// Port of the private class `LowFreqDocsEnumNoTF`.
struct LowFreqDocsEnumNoTF {
    term: Option<Arc<LowFreqTerm>>,
    upto: i32,
}

java_display!(LowFreqDocsEnumNoTF, "LowFreqDocsEnumNoTF");

impl LowFreqDocsEnumNoTF {
    fn new() -> Self {
        Self {
            term: None,
            upto: 0,
        }
    }

    fn reset(&mut self, term: Arc<LowFreqTerm>) {
        self.term = Some(term);
        self.upto = -1;
    }

    fn postings(&self) -> &[i32] {
        &self
            .term
            .as_ref()
            .expect("INVARIANT: `reset` runs before the enum is handed out")
            .postings
    }
}

impl DocIdSetIterator for LowFreqDocsEnumNoTF {
    fn doc_id(&self) -> i32 {
        if self.upto < 0 {
            -1
        } else if (self.upto as usize) < self.postings().len() {
            self.postings()[self.upto as usize]
        } else {
            NO_MORE_DOCS
        }
    }

    // TODO: can do this w/o setting members?
    fn next_doc(&mut self) -> Result<i32> {
        self.upto += 1;
        if (self.upto as usize) < self.postings().len() {
            return Ok(self.postings()[self.upto as usize]);
        }
        Ok(NO_MORE_DOCS)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        // Linear scan, but this is low-freq term so it won't
        // be costly:
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        self.postings().len() as i64
    }
}

impl PostingsEnum for LowFreqDocsEnumNoTF {
    fn freq(&self) -> Result<i32> {
        Ok(1)
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(-1)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Docs + freqs.
///
/// Port of the private class `LowFreqDocsEnumNoPos`.
struct LowFreqDocsEnumNoPos {
    term: Option<Arc<LowFreqTerm>>,
    upto: i32,
}

java_display!(LowFreqDocsEnumNoPos, "LowFreqDocsEnumNoPos");

impl LowFreqDocsEnumNoPos {
    fn new() -> Self {
        Self {
            term: None,
            upto: 0,
        }
    }

    fn reset(&mut self, term: Arc<LowFreqTerm>) {
        self.term = Some(term);
        self.upto = -2;
    }

    fn postings(&self) -> &[i32] {
        &self
            .term
            .as_ref()
            .expect("INVARIANT: `reset` runs before the enum is handed out")
            .postings
    }
}

impl DocIdSetIterator for LowFreqDocsEnumNoPos {
    fn doc_id(&self) -> i32 {
        if self.upto < 0 {
            -1
        } else if (self.upto as usize) < self.postings().len() {
            self.postings()[self.upto as usize]
        } else {
            NO_MORE_DOCS
        }
    }

    // TODO: can do this w/o setting members?
    fn next_doc(&mut self) -> Result<i32> {
        self.upto += 2;
        if (self.upto as usize) < self.postings().len() {
            return Ok(self.postings()[self.upto as usize]);
        }
        Ok(NO_MORE_DOCS)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        // Linear scan, but this is low-freq term so it won't
        // be costly:
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        (self.postings().len() / 2) as i64
    }
}

impl PostingsEnum for LowFreqDocsEnumNoPos {
    fn freq(&self) -> Result<i32> {
        at(self.postings(), self.upto + 1)
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(-1)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Docs + freqs + positions/offsets.
///
/// Port of the private class `LowFreqDocsEnum`.
struct LowFreqDocsEnum {
    term: Option<Arc<LowFreqTerm>>,
    pos_mult: i32,
    upto: i32,
    freq: i32,
}

java_display!(LowFreqDocsEnum, "LowFreqDocsEnum");

impl LowFreqDocsEnum {
    fn new(pos_mult: i32) -> Self {
        Self {
            term: None,
            pos_mult,
            upto: 0,
            freq: 0,
        }
    }

    fn can_reuse(&self, pos_mult: i32) -> bool {
        self.pos_mult == pos_mult
    }

    fn reset(&mut self, term: Arc<LowFreqTerm>) {
        self.term = Some(term);
        self.upto = -2;
        self.freq = 0;
    }

    fn postings(&self) -> &[i32] {
        &self
            .term
            .as_ref()
            .expect("INVARIANT: `reset` runs before the enum is handed out")
            .postings
    }
}

impl DocIdSetIterator for LowFreqDocsEnum {
    fn doc_id(&self) -> i32 {
        // TODO: store docID member?
        if self.upto < 0 {
            -1
        } else if (self.upto as usize) < self.postings().len() {
            self.postings()[self.upto as usize]
        } else {
            NO_MORE_DOCS
        }
    }

    // TODO: can do this w/o setting members?
    fn next_doc(&mut self) -> Result<i32> {
        self.upto += 2 + self.freq * self.pos_mult;
        if (self.upto as usize) < self.postings().len() {
            self.freq = self.postings()[self.upto as usize + 1];
            java_assert!(self.freq > 0);
            return Ok(self.postings()[self.upto as usize]);
        }
        Ok(NO_MORE_DOCS)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        // Linear scan, but this is low-freq term so it won't
        // be costly:
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        // TODO: could do a better estimate
        (self.postings().len() / 2) as i64
    }
}

impl PostingsEnum for LowFreqDocsEnum {
    fn freq(&self) -> Result<i32> {
        // TODO: can I do postings[upto+1]?
        Ok(self.freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(-1)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Port of the private class `LowFreqPostingsEnum`.
struct LowFreqPostingsEnum {
    term: Option<Arc<LowFreqTerm>>,
    pos_mult: i32,
    has_offsets: bool,
    has_payloads: bool,
    upto: i32,
    doc_id: i32,
    freq: i32,
    skip_positions: i32,
    pos: i32,
    start_offset: i32,
    end_offset: i32,
    last_payload_offset: i32,
    payload_offset: i32,
    payload_length: i32,
}

java_display!(LowFreqPostingsEnum, "LowFreqPostingsEnum");

impl LowFreqPostingsEnum {
    fn new(has_offsets: bool, has_payloads: bool) -> Self {
        let pos_mult = if has_offsets {
            if has_payloads {
                4
            } else {
                3
            }
        } else if has_payloads {
            2
        } else {
            1
        };
        Self {
            term: None,
            pos_mult,
            has_offsets,
            has_payloads,
            upto: 0,
            doc_id: 0,
            freq: 0,
            skip_positions: 0,
            pos: 0,
            start_offset: 0,
            end_offset: 0,
            last_payload_offset: 0,
            payload_offset: 0,
            payload_length: 0,
        }
    }

    fn reset(&mut self, term: Arc<LowFreqTerm>) {
        self.term = Some(term);
        self.upto = 0;
        self.skip_positions = 0;
        self.pos = -1;
        self.start_offset = -1;
        self.end_offset = -1;
        self.doc_id = -1;
        self.payload_length = 0;
    }

    fn term(&self) -> &LowFreqTerm {
        self.term
            .as_ref()
            .expect("INVARIANT: `reset` runs before the enum is handed out")
    }

    /// `postings[upto++]`.
    fn read(&mut self) -> Result<i32> {
        let v = at(&self.term().postings, self.upto)?;
        self.upto += 1;
        Ok(v)
    }
}

impl DocIdSetIterator for LowFreqPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.pos = -1;
        if self.has_payloads {
            for _ in 0..self.skip_positions {
                self.upto += 1;
                if self.has_offsets {
                    self.upto += 2;
                }
                self.payload_offset += self.read()?;
            }
        } else {
            self.upto += self.pos_mult * self.skip_positions;
        }

        if (self.upto as usize) < self.term().postings.len() {
            self.doc_id = self.read()?;
            self.freq = self.read()?;
            self.skip_positions = self.freq;
            return Ok(self.doc_id);
        }

        self.doc_id = NO_MORE_DOCS;
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        // TODO: could do a better estimate
        (self.term().postings.len() / 2) as i64
    }
}

impl PostingsEnum for LowFreqPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        java_assert!(self.skip_positions > 0);
        self.skip_positions -= 1;
        self.pos = self.read()?;
        if self.has_offsets {
            self.start_offset = self.read()?;
            self.end_offset = self.read()?;
        }
        if self.has_payloads {
            self.payload_length = self.read()?;
            self.last_payload_offset = self.payload_offset;
            self.payload_offset += self.payload_length;
        }
        Ok(self.pos)
    }

    fn start_offset(&self) -> i32 {
        self.start_offset
    }

    fn end_offset(&self) -> i32 {
        self.end_offset
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        if self.payload_length > 0 {
            let bytes = self.term().payloads.as_deref().unwrap_or_default();
            let start = self.last_payload_offset as usize;
            Ok(Some(&bytes[start..start + self.payload_length as usize]))
        } else {
            Ok(None)
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Port of the `advance(int)` both high-freq enums share: "grow" outwards,
/// since most advances are to nearby docs, then binary search; returns the
/// new `upto`.
fn high_freq_advance(doc_ids: &[i32], upto: i32, target: i32) -> i32 {
    let len = doc_ids.len() as i32;
    // First "grow" outwards, since most advances are to
    // nearby docs:
    let mut inc = 10;
    let mut next_upto = upto + 10;
    let mut low;
    let mut high;
    loop {
        if next_upto >= len {
            low = next_upto - inc;
            high = len - 1;
            break;
        }

        if target <= doc_ids[next_upto as usize] {
            low = next_upto - inc;
            high = next_upto;
            break;
        }
        inc *= 2;
        next_upto += inc;
    }

    // Now do normal binary search
    loop {
        if low > high {
            // Not exactly found
            return low;
        }

        let mid = ((low + high) as u32 >> 1) as i32;
        let cmp = doc_ids[mid as usize] - target;

        if cmp < 0 {
            low = mid + 1;
        } else if cmp > 0 {
            high = mid - 1;
        } else {
            // Found target
            return mid;
        }
    }
}

/// Docs + freqs.
///
/// Port of the private class `HighFreqDocsEnum`.
struct HighFreqDocsEnum {
    term: Option<Arc<HighFreqTerm>>,
    upto: i32,
    doc_id: i32,
}

java_display!(HighFreqDocsEnum, "HighFreqDocsEnum");

impl HighFreqDocsEnum {
    fn new() -> Self {
        Self {
            term: None,
            upto: 0,
            doc_id: -1,
        }
    }

    fn reset(&mut self, term: Arc<HighFreqTerm>) {
        self.term = Some(term);
        self.upto = -1;
        self.doc_id = -1;
    }

    fn doc_ids(&self) -> &[i32] {
        &self
            .term
            .as_ref()
            .expect("INVARIANT: `reset` runs before the enum is handed out")
            .doc_ids
    }
}

impl DocIdSetIterator for HighFreqDocsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.upto += 1;
        // `try { return docID = docIDs[upto]; } catch (ArrayIndexOutOfBoundsException e) {}`
        self.doc_id = at(self.doc_ids(), self.upto).unwrap_or(NO_MORE_DOCS);
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.upto += 1;
        let len = self.doc_ids().len() as i32;
        if self.upto == len {
            self.doc_id = NO_MORE_DOCS;
            return Ok(self.doc_id);
        }

        self.upto = high_freq_advance(self.doc_ids(), self.upto, target);

        if self.upto == len {
            self.doc_id = NO_MORE_DOCS;
        } else {
            self.doc_id = at(self.doc_ids(), self.upto)?;
        }
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.doc_ids().len() as i64
    }
}

impl PostingsEnum for HighFreqDocsEnum {
    fn freq(&self) -> Result<i32> {
        let term = self
            .term
            .as_ref()
            .expect("INVARIANT: `reset` runs before the enum is handed out");
        match &term.freqs {
            None => Ok(1),
            Some(freqs) => at(freqs, self.upto),
        }
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(-1)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Port of the private class `HighFreqPostingsEnum` ("TODO: specialize
/// offsets and not").
struct HighFreqPostingsEnum {
    term: Option<Arc<HighFreqTerm>>,
    has_offsets: bool,
    pos_jump: i32,
    upto: i32,
    doc_id: i32,
    pos_upto: i32,
}

java_display!(HighFreqPostingsEnum, "HighFreqPostingsEnum");

impl HighFreqPostingsEnum {
    fn new(has_offsets: bool) -> Self {
        Self {
            term: None,
            has_offsets,
            pos_jump: if has_offsets { 3 } else { 1 },
            upto: 0,
            doc_id: -1,
            pos_upto: 0,
        }
    }

    fn reset(&mut self, term: Arc<HighFreqTerm>) {
        self.term = Some(term);
        self.upto = -1;
    }

    fn term(&self) -> &HighFreqTerm {
        self.term
            .as_ref()
            .expect("INVARIANT: `reset` runs before the enum is handed out")
    }

    /// `curPositions`: `positions[upto]` of the document `nextDoc` or
    /// `advance` last moved to.
    fn cur_positions(&self) -> &[i32] {
        self.term()
            .positions
            .as_ref()
            .and_then(|p| p.get(self.upto as usize))
            .map_or(&[], Vec::as_slice)
    }
}

impl DocIdSetIterator for HighFreqPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.upto += 1;
        if (self.upto as usize) < self.term().doc_ids.len() {
            self.pos_upto = -self.pos_jump;
            self.doc_id = self.term().doc_ids[self.upto as usize];
            return Ok(self.doc_id);
        }

        self.doc_id = NO_MORE_DOCS;
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.upto += 1;
        let len = self.term().doc_ids.len() as i32;
        if self.upto == len {
            self.doc_id = NO_MORE_DOCS;
            return Ok(self.doc_id);
        }

        self.upto = high_freq_advance(&self.term().doc_ids, self.upto, target);

        if self.upto == len {
            self.doc_id = NO_MORE_DOCS;
        } else {
            self.pos_upto = -self.pos_jump;
            self.doc_id = at(&self.term().doc_ids, self.upto)?;
        }
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.term().doc_ids.len() as i64
    }
}

impl PostingsEnum for HighFreqPostingsEnum {
    fn freq(&self) -> Result<i32> {
        match &self.term().freqs {
            Some(freqs) => at(freqs, self.upto),
            None => Err(null_pointer(
                "Cannot load from int array because \"this.freqs\" is null",
            )),
        }
    }

    fn next_position(&mut self) -> Result<i32> {
        self.pos_upto += self.pos_jump;
        java_assert!((self.pos_upto as usize) < self.cur_positions().len());
        at(self.cur_positions(), self.pos_upto)
    }

    fn start_offset(&self) -> i32 {
        if self.has_offsets {
            self.cur_positions()[(self.pos_upto + 1) as usize]
        } else {
            -1
        }
    }

    fn end_offset(&self) -> i32 {
        if self.has_offsets {
            self.cur_positions()[(self.pos_upto + 2) as usize]
        } else {
            -1
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        match &self.term().payloads {
            None => Ok(None),
            Some(payloads) => {
                let doc = usize::try_from(self.upto)
                    .ok()
                    .and_then(|u| payloads.get(u))
                    .ok_or_else(|| out_of_bounds(i64::from(self.upto), payloads.len()))?;
                let index = self.pos_upto / if self.has_offsets { 3 } else { 1 };
                let payload_bytes = usize::try_from(index)
                    .ok()
                    .and_then(|i| doc.get(i))
                    .ok_or_else(|| out_of_bounds(i64::from(index), doc.len()))?;
                Ok(payload_bytes.as_deref())
            }
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
