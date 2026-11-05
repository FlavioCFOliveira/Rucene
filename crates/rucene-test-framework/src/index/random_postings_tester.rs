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

//! Port of `org.apache.lucene.tests.index.RandomPostingsTester`: "Helper class
//! extracted from BasePostingsFormatTestCase to exercise a postings format."
//!
//! **Declared divergences**, each a Rust limit:
//!
//! * `testFields` checks that `Fields.iterator()` refuses `remove()` and throws
//!   `NoSuchElementException` past its end; a Rust [`Iterator`] has no
//!   `remove` and answers `None` past its end, which is what is checked.
//! * With [`TestOption::Threads`], Java's threads shuffle and read the one
//!   shared `allTerms` list concurrently (a data race). Here each thread
//!   shuffles it under a lock and iterates the order it left.
//! * `SeedTermsEnum.postings` returns `null` for flags the field cannot honour;
//!   [`TermsEnum::postings`] returns a postings enum or an error, so it fails
//!   with `IllegalStateException` instead. The writers ask only for what the
//!   field indexes, so neither is reached by `buildIndex`.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Bound;
use std::sync::{Arc, Mutex, PoisonError};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::postings::FieldsProducer;
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::Codec;
use rucene::error::{LuceneError, Result};
use rucene::index::doc_values::{DocValuesIterator, NumericDocValues};
use rucene::index::documents_writer::BufferedUpdates;
use rucene::index::postings_enum::{
    feature_requested, DocAndFloatFeatureBuffer, FreqAndNormBuffer, ImpactsEnum, PostingsEnum,
    POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS,
    POSTINGS_ENUM_POSITIONS,
};
use rucene::index::terms::{Fields, SeekStatus, TermState, Terms, TermsEnum};
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, FieldInfos, IndexOptions, SegmentInfo,
    VectorEncoding, VectorSimilarityFunction,
};
use rucene::internal::tests_hooks::{IndexPackageAccess, TestSecrets};
use rucene::search::{DocIdSetIterator, Sort, NO_MORE_DOCS};
use rucene::store::java_nio_file::Path;
use rucene::store::{flush_io_context, Directory, FlushInfo, DEFAULT_IO_CONTEXT};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::compiled_automaton::{AutomatonType, CompiledAutomaton};
use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::automaton::run_automaton::ByteRunnable;
use rucene::util::string_helper::StringHelper;
use rucene::util::unicode_util::UnicodeUtil;
use rucene::util::{BytesRef, FixedBitSet, NoOutputInfoStream, Version};

use rucene::codecs::term_state::Impact;

use crate::util::automaton::automaton_test_util::{AutomatonTestUtil, RandomAcceptedStrings};
use crate::util::lucene_test_case::{LuceneTestCase, RANDOM_MULTIPLIER, TEST_NIGHTLY, VERBOSE};
use crate::util::TestUtil;

/// `INDEX_PACKAGE_ACCESS = TestSecrets.getIndexPackageAccess()`.
fn index_package_access() -> Arc<dyn IndexPackageAccess> {
    TestSecrets::get_index_package_access()
        .expect("the index package registers its IndexPackageAccess with TestSecrets")
}

/// `DOC_TO_NORM = doc -> 1 + (doc & 0x0f)`.
fn doc_to_norm(doc: i32) -> i64 {
    1 + i64::from(doc & 0x0f)
}

/// `IndexOptions.values()`.
const INDEX_OPTIONS_VALUES: [IndexOptions; 6] = [
    IndexOptions::NONE,
    IndexOptions::DOCS,
    IndexOptions::DOCS_AND_FREQS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
    IndexOptions::DOCS_AND_CUSTOM_FREQS,
];

/// `Arrays.asList(IndexOptions.values()).indexOf(options)`.
fn index_option_ordinal(options: IndexOptions) -> i32 {
    INDEX_OPTIONS_VALUES
        .iter()
        .position(|&o| o == options)
        .expect("INVARIANT: every IndexOptions is a value") as i32
}

/// The nested enum `RandomPostingsTester.Option`: "Which features to test."
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TestOption {
    /// Sometimes use .advance():
    Skipping,
    /// Sometimes reuse the PostingsEnum across terms:
    ReuseEnums,
    /// Sometimes pass non-null live docs:
    LiveDocs,
    /// Sometimes seek to term using previously saved TermState:
    TermState,
    /// Sometimes don't fully consume docs from the enum
    PartialDocConsume,
    /// Sometimes don't fully consume positions at each doc
    PartialPosConsume,
    /// Check DocIdSetIterator#intoBitSet
    IntoBitSet,
    /// Check PostingsEnum#nextDocsAndFreqs
    NextDocsAndFreqs,
    /// Sometimes check payloads
    Payloads,
    /// Test w/ multiple threads
    Threads,
}

impl TestOption {
    /// `Option.values()`.
    pub const VALUES: [TestOption; 10] = [
        TestOption::Skipping,
        TestOption::ReuseEnums,
        TestOption::LiveDocs,
        TestOption::TermState,
        TestOption::PartialDocConsume,
        TestOption::PartialPosConsume,
        TestOption::IntoBitSet,
        TestOption::NextDocsAndFreqs,
        TestOption::Payloads,
        TestOption::Threads,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            TestOption::Skipping => "SKIPPING",
            TestOption::ReuseEnums => "REUSE_ENUMS",
            TestOption::LiveDocs => "LIVE_DOCS",
            TestOption::TermState => "TERM_STATE",
            TestOption::PartialDocConsume => "PARTIAL_DOC_CONSUME",
            TestOption::PartialPosConsume => "PARTIAL_POS_CONSUME",
            TestOption::IntoBitSet => "INTO_BIT_SET",
            TestOption::NextDocsAndFreqs => "NEXT_DOCS_AND_FREQS",
            TestOption::Payloads => "PAYLOADS",
            TestOption::Threads => "THREADS",
        }
    }
}

/// `EnumSet<Option>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct TestOptions(u16);

impl TestOptions {
    /// `EnumSet.noneOf(Option.class)`.
    pub fn none_of() -> Self {
        TestOptions(0)
    }

    /// `EnumSet.allOf(Option.class)`.
    pub fn all_of() -> Self {
        TestOptions((1 << TestOption::VALUES.len()) - 1)
    }

    /// `EnumSet.of(options...)`.
    pub fn of(options: &[TestOption]) -> Self {
        let mut set = Self::none_of();
        for &o in options {
            set.add(o);
        }
        set
    }

    /// `EnumSet.complementOf(set)`.
    pub fn complement_of(set: TestOptions) -> Self {
        TestOptions(!set.0 & Self::all_of().0)
    }

    /// `add(option)`.
    pub fn add(&mut self, option: TestOption) {
        self.0 |= 1 << option as u16;
    }

    /// `remove(option)`.
    pub fn remove(&mut self, option: TestOption) {
        self.0 &= !(1 << option as u16);
    }

    /// `contains(option)`.
    pub fn contains(&self, option: TestOption) -> bool {
        self.0 & (1 << option as u16) != 0
    }
}

impl fmt::Display for TestOptions {
    /// `AbstractCollection.toString()`: `[A, B]` in declaration order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = TestOption::VALUES
            .iter()
            .filter(|&&o| self.contains(o))
            .map(|o| o.name())
            .collect();
        write!(f, "[{}]", names.join(", "))
    }
}

/// `RandomPostingsTester.SeedAndOrd`.
#[derive(Debug, Clone, Copy)]
struct SeedAndOrd {
    seed: i64,
    ord: i64,
}

/// `Map<String, SortedMap<BytesRef, SeedAndOrd>>`: a `TreeMap` of field names
/// (simple strings, so UTF-16 and UTF-8 order agree) to `TreeMap`s of terms.
type PostingsMap = BTreeMap<String, Arc<BTreeMap<BytesRef, SeedAndOrd>>>;

/// The nested class `RandomPostingsTester.FieldAndTerm`: "Holds one field,
/// term and ord."
#[derive(Debug, Clone)]
pub struct FieldAndTerm {
    field: String,
    term: BytesRef,
    ord: i64,
}

impl FieldAndTerm {
    /// `new FieldAndTerm(field, term, ord)`: the term is deep-copied.
    pub fn new(field: &str, term: &BytesRef, ord: i64) -> Self {
        FieldAndTerm {
            field: field.to_string(),
            term: BytesRef::deep_copy_of(term),
            ord,
        }
    }

    /// `field`.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// `term`.
    pub fn term(&self) -> &BytesRef {
        &self.term
    }

    /// `ord`.
    pub fn ord(&self) -> i64 {
        self.ord
    }
}

/// The nested class `RandomPostingsTester.SeedPostings` (`extends
/// PostingsEnum`): "Given the same random seed this always enumerates the same
/// random postings".
pub struct SeedPostings {
    /// Used only to generate docIDs; this way if you pull w/
    /// or w/o positions you get the same docID sequence:
    doc_random: JdkRandom,
    random: JdkRandom,
    /// `docFreq`.
    pub doc_freq: i32,
    max_doc_spacing: i32,
    payload_size: i32,
    fixed_payloads: bool,
    /// `payload.bytes`, `payloadSize` long.
    payload_bytes: Vec<u8>,
    /// `payload.length`.
    payload_length: usize,
    do_positions: bool,
    allow_payloads: bool,

    doc_id: i32,
    freq: i32,
    /// `upto`.
    pub upto: i32,

    pos: i32,
    offset: i32,
    start_offset: i32,
    end_offset: i32,
    pos_spacing: i32,
    pos_upto: i32,
}

impl SeedPostings {
    /// `new SeedPostings(seed, minDocFreq, maxDocFreq, options, allowPayloads)`.
    pub fn new(
        seed: i64,
        min_doc_freq: i32,
        max_doc_freq: i32,
        options: IndexOptions,
        allow_payloads: bool,
    ) -> Self {
        let random = JdkRandom::new(seed);
        let doc_random = JdkRandom::new(random.next_long());
        let doc_freq = TestUtil::next_int(&random, min_doc_freq, max_doc_freq);

        // TODO: more realistic to inversely tie this to numDocs:
        let max_doc_spacing = TestUtil::next_int(&random, 1, 100);

        let payload_size = if random.next_int_bound(10) == 7 {
            // 10% of the time create big payloads:
            1 + random.next_int_bound(3)
        } else {
            1 + random.next_int_bound(1)
        };

        let fixed_payloads = random.next_boolean();
        let payload_bytes = vec![0u8; payload_size as usize];
        SeedPostings {
            doc_random,
            random,
            doc_freq,
            max_doc_spacing,
            payload_size,
            fixed_payloads,
            payload_length: payload_bytes.len(),
            payload_bytes,
            do_positions: options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS),
            allow_payloads,
            doc_id: -1,
            freq: 0,
            upto: 0,
            pos: 0,
            offset: 0,
            start_offset: 0,
            end_offset: 0,
            pos_spacing: 0,
            pos_upto: 0,
        }
    }

    /// `_nextDoc()`.
    fn next_doc_inner(&mut self) -> i32 {
        if self.doc_id == -1 {
            self.doc_id = 0;
        }
        // Must consume random:
        while self.pos_upto < self.freq {
            self.next_position_inner();
        }

        if self.upto < self.doc_freq {
            if self.upto == 0 && self.doc_random.next_boolean() {
                // Sometimes index docID = 0
            } else if self.max_doc_spacing == 1 {
                self.doc_id += 1;
            } else {
                // TODO: sometimes have a biggish gap here!
                self.doc_id += TestUtil::next_int(&self.doc_random, 1, self.max_doc_spacing);
            }

            self.freq = if self.random.next_int_bound(200) == 17 {
                TestUtil::next_int(&self.random, 1, 1000)
            } else if self.random.next_int_bound(10) == 17 {
                TestUtil::next_int(&self.random, 1, 20)
            } else {
                TestUtil::next_int(&self.random, 1, 4)
            };

            self.pos = 0;
            self.offset = 0;
            self.pos_upto = 0;
            self.pos_spacing = TestUtil::next_int(&self.random, 1, 100);

            self.upto += 1;
            self.doc_id
        } else {
            self.doc_id = NO_MORE_DOCS;
            self.doc_id
        }
    }

    /// `nextPosition()`.
    fn next_position_inner(&mut self) -> i32 {
        if !self.do_positions {
            self.pos_upto = self.freq;
            return -1;
        }
        debug_assert!(self.pos_upto < self.freq);

        if self.pos_upto == 0 && self.random.next_boolean() {
            // Sometimes index pos = 0
        } else if self.pos_spacing == 1 {
            self.pos += 1;
        } else {
            self.pos += TestUtil::next_int(&self.random, 1, self.pos_spacing);
        }

        if self.payload_size != 0 {
            if self.fixed_payloads {
                self.payload_length = self.payload_size as usize;
                self.random.next_bytes(&mut self.payload_bytes);
            } else {
                let this_payload_size = self.random.next_int_bound(self.payload_size);
                if this_payload_size != 0 {
                    self.payload_length = self.payload_size as usize;
                    self.random.next_bytes(&mut self.payload_bytes);
                } else {
                    self.payload_length = 0;
                }
            }
        } else {
            self.payload_length = 0;
        }
        if !self.allow_payloads {
            self.payload_length = 0;
        }

        self.start_offset = self.offset + self.random.next_int_bound(5);
        self.end_offset = self.start_offset + self.random.next_int_bound(10);
        self.offset = self.end_offset;

        self.pos_upto += 1;
        self.pos
    }

    /// `getPayload()` as the bytes it holds: `null` when `length == 0`.
    pub fn payload(&self) -> Option<&[u8]> {
        if self.payload_length == 0 {
            None
        } else {
            Some(&self.payload_bytes[..self.payload_length])
        }
    }
}

impl fmt::Display for SeedPostings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SeedPostings")
    }
}

impl DocIdSetIterator for SeedPostings {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.next_doc_inner();
        Ok(self.doc_id)
    }

    /// `advance(target)`: `slowAdvance(target)`.
    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        i64::from(self.doc_freq)
    }
}

impl PostingsEnum for SeedPostings {
    fn freq(&self) -> Result<i32> {
        Ok(self.freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(self.next_position_inner())
    }

    fn start_offset(&self) -> i32 {
        self.start_offset
    }

    fn end_offset(&self) -> i32 {
        self.end_offset
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(self.payload())
    }
}

/// `RandomPostingsTester.SeedFields` (`extends Fields`).
struct SeedFields {
    fields: Arc<PostingsMap>,
    field_infos: FieldInfos,
    max_allowed: IndexOptions,
    allow_payloads: bool,
}

impl Fields for SeedFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.fields.keys().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        match self.fields.get(field) {
            None => Ok(None),
            Some(terms) => {
                let field_info = self.field_infos.field_info(field).ok_or_else(|| {
                    LuceneError::illegal_state(format!("no FieldInfo for field {field}"))
                })?;
                Ok(Some(Box::new(SeedTerms {
                    terms: Arc::clone(terms),
                    index_options: field_info.get_index_options(),
                    has_payloads: field_info.has_payloads(),
                    max_allowed: self.max_allowed,
                    allow_payloads: self.allow_payloads,
                })))
            }
        }
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

/// `RandomPostingsTester.SeedTerms` (`extends Terms`); `fieldInfo` is held as
/// the two properties its methods read.
struct SeedTerms {
    terms: Arc<BTreeMap<BytesRef, SeedAndOrd>>,
    index_options: IndexOptions,
    has_payloads: bool,
    max_allowed: IndexOptions,
    allow_payloads: bool,
}

fn unsupported() -> LuceneError {
    LuceneError::UnsupportedOperation(String::new())
}

impl Terms for SeedTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        // `termsEnum.reset()`: positioned before the first term.
        Ok(Box::new(SeedTermsEnum {
            terms: Arc::clone(&self.terms),
            max_allowed: self.max_allowed,
            allow_payloads: self.allow_payloads,
            iterator_after: Some(Bound::Unbounded),
            current: None,
            attributes: AttributeSource::new(),
        }))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.terms.len() as i64)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_count(&self) -> Result<i32> {
        Err(unsupported())
    }

    fn has_freqs(&self) -> bool {
        self.index_options.subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.allow_payloads && self.has_payloads
    }
}

/// `RandomPostingsTester.SeedTermsEnum` (`extends BaseTermsEnum`).
///
/// Java's `iterator` over a `tailMap` entry set is the bound after which its
/// next entry starts: `Unbounded` before the first `next()`, `Included(t)` at
/// a sought `t`, `Excluded(t)` once `t` was returned; `None` before `reset()`.
struct SeedTermsEnum {
    terms: Arc<BTreeMap<BytesRef, SeedAndOrd>>,
    max_allowed: IndexOptions,
    allow_payloads: bool,
    iterator_after: Option<Bound<BytesRef>>,
    current: Option<(BytesRef, SeedAndOrd)>,
    attributes: AttributeSource,
}

impl SeedTermsEnum {
    fn current(&self) -> Result<&(BytesRef, SeedAndOrd)> {
        // Java dereferences `current` (`NullPointerException` when unset).
        self.current
            .as_ref()
            .ok_or_else(|| LuceneError::illegal_state("java.lang.NullPointerException"))
    }
}

impl fmt::Display for SeedTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SeedTermsEnum")
    }
}

impl TermsEnum for SeedTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let first = self
            .terms
            .range((Bound::Included(text.clone()), Bound::Unbounded))
            .next()
            .map(|(k, v)| (k.clone(), *v));
        match first {
            None => Ok(SeekStatus::END),
            Some((key, value)) => {
                self.iterator_after = Some(Bound::Excluded(key.clone()));
                let found = key == *text;
                self.current = Some((key, value));
                Ok(if found {
                    SeekStatus::FOUND
                } else {
                    SeekStatus::NOT_FOUND
                })
            }
        }
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        let after = self
            .iterator_after
            .clone()
            .ok_or_else(|| LuceneError::illegal_state("java.lang.NullPointerException"))?;
        let next = self
            .terms
            .range((after, Bound::Unbounded))
            .next()
            .map(|(k, v)| (k.clone(), *v));
        match next {
            Some((key, value)) => {
                self.iterator_after = Some(Bound::Excluded(key.clone()));
                self.current = Some((key.clone(), value));
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.current()?.0.clone())
    }

    fn ord(&self) -> Result<i64> {
        Ok(self.current()?.1.ord)
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Err(unsupported())
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Err(unsupported())
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let null = || LuceneError::illegal_state("SeedTermsEnum.postings returns null");
        if feature_requested(flags, POSTINGS_ENUM_POSITIONS) {
            if !self
                .max_allowed
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            {
                return Err(null());
            }
            if feature_requested(flags, POSTINGS_ENUM_OFFSETS)
                && !self
                    .max_allowed
                    .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            {
                return Err(null());
            }
            if feature_requested(flags, POSTINGS_ENUM_PAYLOADS) && !self.allow_payloads {
                return Err(null());
            }
        }
        if feature_requested(flags, POSTINGS_ENUM_FREQS)
            && !self.max_allowed.subsumes(IndexOptions::DOCS_AND_FREQS)
        {
            return Err(null());
        }
        let (term, seed_and_ord) = self.current()?;
        Ok(Box::new(RandomPostingsTester::get_seed_postings(
            &term.utf8_to_string()?,
            seed_and_ord.seed,
            self.max_allowed,
            self.allow_payloads,
        )))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(unsupported())
    }
}

/// The nested class `RandomPostingsTester.ThreadState`.
#[derive(Default)]
struct ThreadState {
    /// Only used with REUSE option:
    reuse_postings_enum: Option<Box<dyn PostingsEnum>>,
}

/// The anonymous `NormsProducer` of `buildIndex`.
#[derive(Debug, Clone)]
struct FakeNorms {
    /// `newFieldInfos.fieldInfo(number).hasNorms()`, by field number.
    has_norms: Vec<bool>,
    max_doc: i32,
}

impl NormsProducer for FakeNorms {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        if self
            .has_norms
            .get(field.number as usize)
            .copied()
            .unwrap_or(false)
        {
            Ok(Some(Box::new(FakeNormValues {
                doc: -1,
                max_doc: self.max_doc,
            })))
        } else {
            Ok(None)
        }
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The anonymous `NumericDocValues` of the fake norms: every document has
/// the norm `DOC_TO_NORM(doc)`.
struct FakeNormValues {
    doc: i32,
    max_doc: i32,
}

impl fmt::Display for FakeNormValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FakeNormValues")
    }
}

impl DocIdSetIterator for FakeNormValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc += 1;
        if self.doc == self.max_doc {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = if target >= self.max_doc {
            NO_MORE_DOCS
        } else {
            target
        };
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc)
    }
}

impl DocValuesIterator for FakeNormValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        Ok(true)
    }
}

impl NumericDocValues for FakeNormValues {
    fn long_value(&self) -> Result<i64> {
        Ok(doc_to_norm(self.doc))
    }
}

/// `Impact.toString()`: `"freq=" + freq + ",norm=" + norm`.
fn impact_to_string(impact: &Impact) -> String {
    format!("freq={},norm={}", impact.freq, impact.norm)
}

/// `List<Impact>.toString()`.
fn impacts_to_string(impacts: &[Impact]) -> String {
    let parts: Vec<String> = impacts.iter().map(impact_to_string).collect();
    format!("[{}]", parts.join(", "))
}

/// `copyOf(FreqAndNormBuffer)`.
fn copy_of(buffer: &FreqAndNormBuffer) -> Vec<Impact> {
    (0..buffer.size)
        .map(|i| Impact::new(buffer.freqs[i], buffer.norms[i]))
        .collect()
}

/// `Collections.binarySearch(impacts, key, Comparator.comparing(i ->
/// i.freq))`: Java's search, which picks the same index among equal keys.
fn binary_search_by_freq(impacts: &[Impact], freq: i32) -> i32 {
    let mut low: i32 = 0;
    let mut high: i32 = impacts.len() as i32 - 1;
    while low <= high {
        let mid = ((low + high) as u32 >> 1) as i32;
        let mid_freq = impacts[mid as usize].freq;
        match mid_freq.cmp(&freq) {
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Greater => high = mid - 1,
            std::cmp::Ordering::Equal => return mid,
        }
    }
    -(low + 1)
}

/// The impact check that ends both impact loops of `verifyEnum`.
fn assert_competitive_impact(impacts_copy: &[Impact], freq: i32, norm: i64) {
    let mut idx = binary_search_by_freq(impacts_copy, freq);
    if idx < 0 {
        idx = -1 - idx;
    }
    let ok = idx as usize <= impacts_copy.len() && {
        // `impactsCopy.get(idx)` throws when `idx == size()`.
        let impact = impacts_copy.get(idx as usize).unwrap_or_else(|| {
            panic!(
                "java.lang.IndexOutOfBoundsException: Index {idx} out of bounds for length {}",
                impacts_copy.len()
            )
        });
        impact.norm <= norm
    };
    Assert::assert_true_msg(
        &format!(
            "Got {} in postings, but no impact triggers equal or better scores in {}",
            impact_to_string(&Impact::new(freq, norm)),
            impacts_to_string(impacts_copy)
        ),
        ok,
    );
}

/// `assertEquals(message, expected, actual)` on `BytesRef`s, whose
/// `toString()` is the hex form.
fn assert_equals_bytes(message: Option<&str>, expected: &BytesRef, actual: &BytesRef) {
    if expected != actual {
        Assert::assert_equals_msg(message, expected.to_string(), actual.to_string());
    }
}

/// `payload == null ? "null" : payload.toString()`, for messages.
fn payload_to_string(payload: Option<&[u8]>) -> String {
    match payload {
        None => "null".to_string(),
        Some(bytes) => BytesRef::new(bytes.to_vec()).to_string(),
    }
}

/// Port of `org.apache.lucene.tests.index.RandomPostingsTester`.
pub struct RandomPostingsTester {
    total_postings: i64,
    total_payload_bytes: i64,

    /// Holds all postings:
    fields: Arc<PostingsMap>,

    field_infos: FieldInfos,

    all_terms: Mutex<Vec<FieldAndTerm>>,
    max_doc: i32,

    random: Arc<dyn Random>,

    current_field_infos: Option<FieldInfos>,
}

impl fmt::Debug for RandomPostingsTester {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RandomPostingsTester")
            .field("fields", &self.fields.len())
            .field("max_doc", &self.max_doc)
            .finish_non_exhaustive()
    }
}

/// `new FieldInfo(field, number, false, false, storePayloads, indexOptions,
/// DocValuesType.NONE, DocValuesSkipIndexType.NONE, -1, new HashMap<>(), 0,
/// 0, 0, 0, VectorEncoding.FLOAT32, VectorSimilarityFunction.EUCLIDEAN,
/// false, false)`.
fn postings_field_info(
    name: &str,
    number: i32,
    store_payloads: bool,
    index_options: IndexOptions,
) -> Result<FieldInfo> {
    FieldInfo::new_full(
        name,
        number,
        false,
        false,
        store_payloads,
        index_options,
        DocValuesType::NONE,
        DocValuesSkipIndexType::NONE,
        -1,
        Default::default(),
        0,
        0,
        0,
        0,
        VectorEncoding::FLOAT32,
        VectorSimilarityFunction::EUCLIDEAN,
        false,
        false,
    )
}

impl RandomPostingsTester {
    /// `new RandomPostingsTester(random)`.
    ///
    /// # Errors
    ///
    /// What building the `FieldInfos` throws.
    pub fn new(random: Arc<dyn Random>) -> Result<Self> {
        let mut fields: PostingsMap = BTreeMap::new();

        let num_fields = TestUtil::next_int(&*random, 1, 5);
        if *VERBOSE {
            println!("TEST: {num_fields} fields");
        }
        let mut max_doc = 0;

        let mut field_info_array: Vec<FieldInfo> = Vec::with_capacity(num_fields as usize);
        let mut field_upto = 0;
        while field_upto < num_fields {
            let field = TestUtil::random_simple_string(&*random);
            if fields.contains_key(&field) {
                continue;
            }

            field_info_array.push(postings_field_info(
                &field,
                field_upto,
                true,
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            )?);
            field_upto += 1;

            let mut postings: BTreeMap<BytesRef, SeedAndOrd> = BTreeMap::new();
            let mut seen_terms = std::collections::HashSet::new();

            let num_terms = if random.next_int_bound(10) == 7 {
                LuceneTestCase::at_least_with(&*random, 50)
            } else {
                TestUtil::next_int(&*random, 2, 20)
            };

            while (postings.len() as i32) < num_terms {
                let term_upto = postings.len();
                // Cannot contain surrogates else default Java string sort order (by UTF16 code unit) is
                // different from Lucene:
                let mut term = TestUtil::random_simple_string(&*random);
                if seen_terms.contains(&term) {
                    continue;
                }
                seen_terms.insert(term.clone());

                if *TEST_NIGHTLY && term_upto == 0 && field_upto == 1 {
                    // Make 1 big term:
                    term = format!("big_{term}");
                } else if term_upto == 1 && field_upto == 1 {
                    // Make 1 medium term:
                    term = format!("medium_{term}");
                } else if random.next_boolean() {
                    // Low freq term:
                    term = format!("low_{term}");
                } else {
                    // Very low freq term (don't multiply by RANDOM_MULTIPLIER):
                    term = format!("verylow_{term}");
                }

                let term_seed = random.next_long();
                postings.insert(
                    BytesRef::from_char_sequence(&term),
                    SeedAndOrd {
                        seed: term_seed,
                        ord: 0,
                    },
                );

                // NOTE: sort of silly: we enum all the docs just to
                // get the maxDoc
                let mut postings_enum =
                    Self::get_seed_postings(&term, term_seed, IndexOptions::DOCS, true);
                let mut last_doc = 0;
                loop {
                    let doc = postings_enum.next_doc_inner();
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    last_doc = doc;
                }
                max_doc = last_doc.max(max_doc);
            }

            // assign ords
            for (ord, ent) in postings.values_mut().enumerate() {
                ent.ord = ord as i64;
            }
            fields.insert(field, Arc::new(postings));
        }

        let field_infos = FieldInfos::new(field_info_array)?;

        // It's the count, not the last docID:
        max_doc += 1;

        let mut all_terms = Vec::new();
        for (field, terms) in &fields {
            for (ord, term) in terms.keys().enumerate() {
                all_terms.push(FieldAndTerm::new(field, term, ord as i64));
            }
        }

        if *VERBOSE {
            println!(
                "TEST: done init postings; {} total terms, across {} fields",
                all_terms.len(),
                field_infos.size()
            );
        }

        Ok(RandomPostingsTester {
            total_postings: 0,
            total_payload_bytes: 0,
            fields: Arc::new(fields),
            field_infos,
            all_terms: Mutex::new(all_terms),
            max_doc,
            random,
            current_field_infos: None,
        })
    }

    /// `getSeedPostings(term, seed, options, allowPayloads)`.
    pub fn get_seed_postings(
        term: &str,
        seed: i64,
        options: IndexOptions,
        allow_payloads: bool,
    ) -> SeedPostings {
        let multiplier = *RANDOM_MULTIPLIER;
        let (min_doc_freq, max_doc_freq) = if term.starts_with("big_") {
            (multiplier * 50000, multiplier * 70000)
        } else if term.starts_with("medium_") {
            (multiplier * 3000, multiplier * 6000)
        } else if term.starts_with("low_") {
            (multiplier, multiplier * 40)
        } else {
            (1, 3)
        };

        SeedPostings::new(seed, min_doc_freq, max_doc_freq, options, allow_payloads)
    }

    /// The package-private `maxDoc`: one past the largest generated docID.
    pub fn max_doc(&self) -> i32 {
        self.max_doc
    }

    /// The package-private `allTerms`, in its current order.
    pub fn all_terms(&self) -> Vec<FieldAndTerm> {
        self.all_terms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The generated fields and terms: each field with its terms in order and
    /// the seed of each term's postings.
    pub fn postings_seeds(&self) -> Vec<(String, Vec<(BytesRef, i64)>)> {
        self.fields
            .iter()
            .map(|(field, terms)| {
                (
                    field.clone(),
                    terms.iter().map(|(t, s)| (t.clone(), s.seed)).collect(),
                )
            })
            .collect()
    }

    /// The `FieldInfos` of the generated fields (`fieldInfos`).
    pub fn field_infos(&self) -> &FieldInfos {
        &self.field_infos
    }

    /// `buildIndex(codec, dir, maxAllowed, allowPayloads, alwaysTestMax)`:
    /// "maxAllowed = the "highest" we can index, but we will still randomly
    /// index at lower IndexOption".
    ///
    /// # Errors
    ///
    /// What writing or opening the postings throws.
    pub fn build_index(
        &mut self,
        codec: Arc<dyn Codec>,
        dir: Arc<dyn Directory>,
        max_allowed: IndexOptions,
        allow_payloads: bool,
        always_test_max: bool,
    ) -> Result<Box<dyn FieldsProducer>> {
        let segment_info = SegmentInfo::new(
            Arc::clone(&dir),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            self.max_doc,
            false,
            false,
            Arc::clone(&codec),
            std::collections::HashMap::new(),
            StringHelper::random_id(),
            std::collections::HashMap::new(),
            Sort::default(),
        )?;

        let max_index_option = index_option_ordinal(max_allowed);
        if *VERBOSE {
            println!("\nTEST: now build index");
        }

        // TODO use allowPayloads

        let mut new_field_info_array = Vec::with_capacity(self.fields.len());
        for field_upto in 0..self.fields.len() as i32 {
            let old_field_info = self
                .field_infos
                .field_info_by_number(field_upto)
                .ok_or_else(|| LuceneError::illegal_state("java.lang.NullPointerException"))?;

            // Randomly picked the IndexOptions to index this
            // field with:
            let index_options = INDEX_OPTIONS_VALUES[if always_test_max {
                max_index_option
            } else {
                TestUtil::next_int(&*self.random, 1, max_index_option)
            } as usize];
            let do_payloads = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
                && allow_payloads;

            new_field_info_array.push(postings_field_info(
                &old_field_info.name,
                field_upto,
                do_payloads,
                index_options,
            )?);
        }

        let new_field_infos = FieldInfos::new(new_field_info_array)?;

        // Estimate that flushed segment size will be 25% of
        // what we use in RAM:
        let bytes = self.total_postings * 8 + self.total_payload_bytes;

        let info_stream = NoOutputInfoStream;
        let seg_updates = BufferedUpdates::new("_0");
        let context = flush_io_context(FlushInfo::new(self.max_doc, bytes));
        let write_state = SegmentWriteState::new(
            &info_stream,
            Arc::clone(&dir),
            &segment_info,
            &new_field_infos,
            &seg_updates,
            &*context,
        );

        let seed_fields = SeedFields {
            fields: Arc::clone(&self.fields),
            field_infos: new_field_infos.clone(),
            max_allowed,
            allow_payloads,
        };

        let fake_norms = FakeNorms {
            has_norms: (0..new_field_infos.size() as i32)
                .map(|n| {
                    new_field_infos
                        .field_info_by_number(n)
                        .is_some_and(|fi| fi.has_norms())
                })
                .collect(),
            max_doc: segment_info.max_doc()?,
        };
        let mut consumer = codec.postings_format().fields_consumer(&write_state)?;
        match consumer.write(&seed_fields, Some(&fake_norms)) {
            Ok(()) => consumer.close()?,
            Err(e) => {
                let _ = consumer.close();
                return Err(e);
            }
        }
        drop(consumer);

        if *VERBOSE {
            println!("TEST: after indexing: files=");
            for file in dir.list_all()? {
                println!("  {file}: {} bytes", dir.file_length(&file)?);
            }
        }

        self.current_field_infos = Some(new_field_infos.clone());

        let read_state =
            SegmentReadState::new(&*dir, &segment_info, &new_field_infos, &*DEFAULT_IO_CONTEXT);

        codec.postings_format().fields_producer(&read_state)
    }

    #[allow(clippy::too_many_arguments)]
    fn verify_enum(
        &self,
        random: &dyn Random,
        thread_state: &mut ThreadState,
        field: &str,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        // Maximum options (docs/freqs/positions/offsets) to test:
        max_test_options: IndexOptions,
        max_index_options: IndexOptions,
        options: TestOptions,
        always_test_max: bool,
    ) -> Result<()> {
        if *VERBOSE {
            println!(
                "  verifyEnum: options={options} maxTestOptions={}",
                index_options_name(max_test_options)
            );
        }

        // Make sure TermsEnum really is positioned on the
        // expected term:
        assert_equals_bytes(None, term, &terms_enum.term()?);

        let current_field_infos = self
            .current_field_infos
            .as_ref()
            .ok_or_else(|| LuceneError::illegal_state("java.lang.NullPointerException"))?;
        let field_info = current_field_infos
            .field_info(field)
            .ok_or_else(|| LuceneError::illegal_state("java.lang.NullPointerException"))?;
        let field_index_options = field_info.get_index_options();

        // NOTE: can be empty list if we are using liveDocs:
        let seed = self
            .fields
            .get(field)
            .and_then(|terms| terms.get(term))
            .ok_or_else(|| LuceneError::illegal_state("java.lang.NullPointerException"))?
            .seed;
        let mut expected =
            Self::get_seed_postings(&term.utf8_to_string()?, seed, max_index_options, true);
        Assert::assert_equals(expected.doc_freq, terms_enum.doc_freq()?);

        let allow_freqs = field_index_options.subsumes(IndexOptions::DOCS_AND_FREQS)
            && max_test_options.subsumes(IndexOptions::DOCS_AND_FREQS);
        let do_check_freqs = allow_freqs && (always_test_max || random.next_int_bound(3) <= 2);

        let allow_positions = field_index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            && max_test_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        let do_check_positions =
            allow_positions && (always_test_max || random.next_int_bound(3) <= 2);

        let allow_offsets = field_index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            && max_test_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        let do_check_offsets = allow_offsets && (always_test_max || random.next_int_bound(3) <= 2);

        let do_check_payloads = options.contains(TestOption::Payloads)
            && allow_positions
            && field_info.has_payloads()
            && (always_test_max || random.next_int_bound(3) <= 2);

        let mut prev_postings_enum: Option<Box<dyn PostingsEnum>> = None;

        if !do_check_positions {
            if allow_positions && random.next_int_bound(10) == 7 {
                // 10% of the time, even though we will not check positions, pull a DocsAndPositions enum

                if options.contains(TestOption::ReuseEnums) && random.next_int_bound(10) < 9 {
                    prev_postings_enum = thread_state.reuse_postings_enum.take();
                }

                let mut flags = POSTINGS_ENUM_POSITIONS;
                if always_test_max || random.next_boolean() {
                    flags |= POSTINGS_ENUM_OFFSETS;
                }
                if always_test_max || random.next_boolean() {
                    flags |= POSTINGS_ENUM_PAYLOADS;
                }

                if *VERBOSE {
                    println!("  get DocsEnum (but we won't check positions) flags={flags}");
                }

                thread_state.reuse_postings_enum = None;
                let had_prev = prev_postings_enum.is_some();
                let postings_enum = terms_enum.postings(prev_postings_enum.take(), flags)?;
                return self.verify_postings(
                    random,
                    thread_state,
                    postings_enum,
                    had_prev,
                    &mut expected,
                    terms_enum,
                    field_info,
                    options,
                    always_test_max,
                    (
                        do_check_freqs,
                        do_check_positions,
                        do_check_offsets,
                        do_check_payloads,
                    ),
                );
            } else {
                if *VERBOSE {
                    println!("  get DocsEnum");
                }
                if options.contains(TestOption::ReuseEnums) && random.next_int_bound(10) < 9 {
                    prev_postings_enum = thread_state.reuse_postings_enum.take();
                }
                thread_state.reuse_postings_enum = None;
                let had_prev = prev_postings_enum.is_some();
                let postings_enum = terms_enum.postings(
                    prev_postings_enum.take(),
                    if do_check_freqs {
                        POSTINGS_ENUM_FREQS
                    } else {
                        POSTINGS_ENUM_NONE
                    },
                )?;
                return self.verify_postings(
                    random,
                    thread_state,
                    postings_enum,
                    had_prev,
                    &mut expected,
                    terms_enum,
                    field_info,
                    options,
                    always_test_max,
                    (
                        do_check_freqs,
                        do_check_positions,
                        do_check_offsets,
                        do_check_payloads,
                    ),
                );
            }
        }
        if options.contains(TestOption::ReuseEnums) && random.next_int_bound(10) < 9 {
            prev_postings_enum = thread_state.reuse_postings_enum.take();
        }

        let mut flags = POSTINGS_ENUM_POSITIONS;
        if always_test_max || do_check_offsets || random.next_int_bound(3) == 1 {
            flags |= POSTINGS_ENUM_OFFSETS;
        }
        if always_test_max || do_check_payloads || random.next_int_bound(3) == 1 {
            flags |= POSTINGS_ENUM_PAYLOADS;
        }

        if *VERBOSE {
            println!("  get DocsEnum flags={flags}");
        }

        thread_state.reuse_postings_enum = None;
        let had_prev = prev_postings_enum.is_some();
        let postings_enum = terms_enum.postings(prev_postings_enum.take(), flags)?;
        self.verify_postings(
            random,
            thread_state,
            postings_enum,
            had_prev,
            &mut expected,
            terms_enum,
            field_info,
            options,
            always_test_max,
            (
                do_check_freqs,
                do_check_positions,
                do_check_offsets,
                do_check_payloads,
            ),
        )
    }

    /// The rest of `verifyEnum`, from `assertNotNull("null DocsEnum",
    /// postingsEnum)` on, over the enum `threadState.reusePostingsEnum` holds
    /// (which it holds again on return).
    #[allow(clippy::too_many_arguments)]
    fn verify_postings(
        &self,
        random: &dyn Random,
        thread_state: &mut ThreadState,
        mut postings_enum: Box<dyn PostingsEnum>,
        had_prev: bool,
        expected: &mut SeedPostings,
        terms_enum: &mut dyn TermsEnum,
        field_info: &FieldInfo,
        options: TestOptions,
        always_test_max: bool,
        checks: (bool, bool, bool, bool),
    ) -> Result<()> {
        let (do_check_freqs, do_check_positions, do_check_offsets, do_check_payloads) = checks;
        let max_doc = self.max_doc;
        let field_index_options = field_info.get_index_options();

        let initial_doc_id = postings_enum.doc_id();
        Assert::assert_equals_msg(
            Some(&format!("initial docID should be -1: {postings_enum}")),
            -1,
            initial_doc_id,
        );

        if *VERBOSE {
            if !had_prev {
                println!("  got enum={postings_enum}");
            } else {
                println!("  got enum={postings_enum} (reuse of a previous enum)");
            }
        }

        // 10% of the time don't consume all docs:
        let stop_at = if !always_test_max
            && options.contains(TestOption::PartialDocConsume)
            && expected.doc_freq > 1
            && random.next_int_bound(10) == 7
        {
            let stop_at = random.next_int_bound(expected.doc_freq - 1);
            if *VERBOSE {
                println!(
                    "  will not consume all docs ({stop_at} vs {})",
                    expected.doc_freq
                );
            }
            stop_at
        } else {
            if *VERBOSE {
                println!("  consume all docs");
            }
            expected.doc_freq
        };

        let skip_chance = if always_test_max {
            0.5
        } else {
            random.next_double()
        };
        let num_skips = if expected.doc_freq < 3 {
            1
        } else {
            TestUtil::next_int(random, 1, 20.min(expected.doc_freq / 3))
        };
        let skip_inc = expected.doc_freq / num_skips;
        let skip_doc_inc = max_doc / num_skips;

        // Sometimes do 100% skipping:
        let do_all_skipping =
            options.contains(TestOption::Skipping) && random.next_int_bound(7) == 1;

        let freq_ask_chance = if always_test_max {
            1.0
        } else {
            random.next_double()
        };
        let payload_check_chance = if always_test_max {
            1.0
        } else {
            random.next_double()
        };
        let offset_check_chance = if always_test_max {
            1.0
        } else {
            random.next_double()
        };

        if *VERBOSE {
            if options.contains(TestOption::Skipping) {
                println!(
                    "  skipChance={} numSkips={num_skips}",
                    randomizedtesting::jdk::lang::double_to_string(skip_chance)
                );
            } else {
                println!("  no skipping");
            }
            if do_check_freqs {
                println!(
                    "  freqAskChance={}",
                    randomizedtesting::jdk::lang::double_to_string(freq_ask_chance)
                );
            }
            if do_check_payloads {
                println!(
                    "  payloadCheckChance={}",
                    randomizedtesting::jdk::lang::double_to_string(payload_check_chance)
                );
            }
            if do_check_offsets {
                println!(
                    "  offsetCheckChance={}",
                    randomizedtesting::jdk::lang::double_to_string(offset_check_chance)
                );
            }
        }

        while expected.upto <= stop_at {
            if expected.upto == stop_at {
                if stop_at == expected.doc_freq {
                    Assert::assert_equals_msg(
                        Some("DocsEnum should have ended but didn't"),
                        NO_MORE_DOCS,
                        postings_enum.next_doc()?,
                    );

                    // Common bug is to forget to set this.doc=NO_MORE_DOCS in the enum!:
                    Assert::assert_equals_msg(
                        Some("DocsEnum should have ended but didn't"),
                        NO_MORE_DOCS,
                        postings_enum.doc_id(),
                    );
                }
                break;
            }

            if options.contains(TestOption::Skipping)
                && (do_all_skipping || random.next_double() <= skip_chance)
            {
                let mut target_doc_id = -1;
                if expected.upto < stop_at && random.next_boolean() {
                    // Pick target we know exists:
                    let skip_count = TestUtil::next_int(random, 1, skip_inc);
                    for _ in 0..skip_count {
                        if expected.next_doc()? == NO_MORE_DOCS {
                            break;
                        }
                    }
                } else {
                    // Pick random target (might not exist):
                    let skip_doc_ids = TestUtil::next_int(random, 1, skip_doc_inc);
                    if skip_doc_ids > 0 {
                        target_doc_id = expected.doc_id() + skip_doc_ids;
                        expected.advance(target_doc_id)?;
                    }
                }

                if expected.upto >= stop_at {
                    let target = if random.next_boolean() {
                        max_doc
                    } else {
                        NO_MORE_DOCS
                    };
                    if *VERBOSE {
                        println!("  now advance to end (target={target})");
                    }
                    Assert::assert_equals_msg(
                        Some("DocsEnum should have ended but didn't"),
                        NO_MORE_DOCS,
                        postings_enum.advance(target)?,
                    );
                    break;
                } else {
                    if *VERBOSE {
                        if target_doc_id != -1 {
                            println!(
                                "  now advance to random target={target_doc_id} ({} of {stop_at}) current={}",
                                expected.upto,
                                postings_enum.doc_id()
                            );
                        } else {
                            println!(
                                "  now advance to known-exists target={} ({} of {stop_at}) current={}",
                                expected.doc_id(),
                                expected.upto,
                                postings_enum.doc_id()
                            );
                        }
                    }
                    let doc_id = postings_enum.advance(if target_doc_id != -1 {
                        target_doc_id
                    } else {
                        expected.doc_id()
                    })?;
                    Assert::assert_equals_msg(Some("docID is wrong"), expected.doc_id(), doc_id);
                }
            } else {
                expected.next_doc()?;
                if *VERBOSE {
                    println!(
                        "  now nextDoc to {} ({} of {stop_at})",
                        expected.doc_id(),
                        expected.upto
                    );
                }
                let doc_id = postings_enum.next_doc()?;
                Assert::assert_equals_msg(Some("docID is wrong"), expected.doc_id(), doc_id);
                if doc_id == NO_MORE_DOCS {
                    break;
                }
            }

            if do_check_freqs && random.next_double() <= freq_ask_chance {
                if *VERBOSE {
                    println!("    now freq()={}", expected.freq);
                }
                let freq = postings_enum.freq()?;
                Assert::assert_equals_msg(Some("freq is wrong"), expected.freq, freq);
            }

            if do_check_positions {
                let freq = postings_enum.freq()?;
                let num_pos_to_consume = if !always_test_max
                    && options.contains(TestOption::PartialPosConsume)
                    && random.next_int_bound(5) == 1
                {
                    random.next_int_bound(freq)
                } else {
                    freq
                };

                for _ in 0..num_pos_to_consume {
                    let pos = expected.next_position_inner();
                    if *VERBOSE {
                        println!("    now nextPosition to {pos}");
                    }
                    Assert::assert_equals_msg(
                        Some("position is wrong"),
                        pos,
                        postings_enum.next_position()?,
                    );

                    if do_check_payloads {
                        let expected_payload = expected.payload().map(<[u8]>::to_vec);
                        if random.next_double() <= payload_check_chance {
                            if *VERBOSE {
                                println!(
                                    "      now check expectedPayload length={}",
                                    expected_payload.as_ref().map_or(0, Vec::len)
                                );
                            }
                            match &expected_payload {
                                None => {
                                    let payload = postings_enum.get_payload()?;
                                    if payload.is_some() {
                                        Assert::fail(&format!(
                                            "should not have payload expected null, but was:<{}>",
                                            payload_to_string(payload)
                                        ));
                                    }
                                }
                                Some(expected_payload) => {
                                    let payload = postings_enum
                                        .get_payload()?
                                        .map(<[u8]>::to_vec)
                                        .unwrap_or_else(|| {
                                            Assert::fail("should have payload but doesn't")
                                        });

                                    Assert::assert_equals_msg(
                                        Some("payload length is wrong"),
                                        expected_payload.len(),
                                        payload.len(),
                                    );
                                    for (byte_upto, &b) in expected_payload.iter().enumerate() {
                                        Assert::assert_equals_msg(
                                            Some("payload bytes are wrong"),
                                            b as i8,
                                            payload[byte_upto] as i8,
                                        );
                                    }

                                    // make a deep copy
                                    let again = postings_enum.get_payload()?.map(<[u8]>::to_vec);
                                    if again.as_deref() != Some(payload.as_slice()) {
                                        Assert::assert_equals_msg(
                                            Some(
                                                "2nd call to getPayload returns something different!",
                                            ),
                                            payload_to_string(Some(&payload)),
                                            payload_to_string(again.as_deref()),
                                        );
                                    }
                                }
                            }
                        } else if *VERBOSE {
                            println!(
                                "      skip check payload length={}",
                                expected_payload.as_ref().map_or(0, Vec::len)
                            );
                        }
                    }

                    if do_check_offsets {
                        if random.next_double() <= offset_check_chance {
                            if *VERBOSE {
                                println!(
                                    "      now check offsets: startOff={} endOffset={}",
                                    expected.start_offset, expected.end_offset
                                );
                            }
                            Assert::assert_equals_msg(
                                Some("startOffset is wrong"),
                                expected.start_offset,
                                postings_enum.start_offset(),
                            );
                            Assert::assert_equals_msg(
                                Some("endOffset is wrong"),
                                expected.end_offset,
                                postings_enum.end_offset(),
                            );
                        } else if *VERBOSE {
                            println!("      skip check offsets");
                        }
                    } else if !field_index_options
                        .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
                    {
                        if *VERBOSE {
                            println!("      now check offsets are -1");
                        }
                        Assert::assert_equals_msg(
                            Some("startOffset isn't -1"),
                            -1,
                            postings_enum.start_offset(),
                        );
                        Assert::assert_equals_msg(
                            Some("endOffset isn't -1"),
                            -1,
                            postings_enum.end_offset(),
                        );
                    }
                }
            }
        }

        // `threadState.reusePostingsEnum` is the enum just verified.
        thread_state.reuse_postings_enum = Some(postings_enum);

        let mut flags_for_checks = POSTINGS_ENUM_FREQS;
        if do_check_positions {
            flags_for_checks |= POSTINGS_ENUM_POSITIONS;
            if do_check_offsets {
                flags_for_checks |= POSTINGS_ENUM_OFFSETS;
            }
            if do_check_payloads {
                flags_for_checks |= POSTINGS_ENUM_PAYLOADS;
            }
        }

        if options.contains(TestOption::Skipping) {
            self.check_impacts(
                random,
                terms_enum,
                field_info.has_norms(),
                flags_for_checks,
                do_check_offsets,
                do_check_payloads,
            )?;
        }

        if options.contains(TestOption::IntoBitSet) {
            let flags = flags_for_checks;
            let mut pe1 = terms_enum.postings(None, flags)?;
            if random.next_boolean() {
                pe1.advance(max_doc / 2)?;
                pe1 = terms_enum.postings(Some(pe1), flags)?;
            }
            let mut pe2 = terms_enum.postings(None, flags)?;
            let mut set1 = FixedBitSet::new(1024);
            let mut set2 = FixedBitSet::new(1024);

            loop {
                pe1.next_doc()?;
                pe2.next_doc()?;

                let length = set1.length() as i32;
                let offset = TestUtil::next_int(
                    random,
                    0.max(pe1.doc_id().wrapping_sub(length)),
                    pe1.doc_id(),
                );
                let up_to = offset.wrapping_add(random.next_int_bound(length));
                pe1.into_bit_set(up_to, &mut set1, offset)?;
                let mut d = pe2.doc_id();
                while d < up_to {
                    set2.set((d - offset) as usize);
                    d = pe2.next_doc()?;
                }

                Assert::assert_true_msg(
                    "expected:<FixedBitSet> but was:<FixedBitSet>",
                    set1 == set2,
                );
                Assert::assert_equals(pe1.doc_id(), pe2.doc_id());
                if pe1.doc_id() == NO_MORE_DOCS {
                    break;
                }
                set1.clear_all();
                set2.clear_all();
            }
        }

        if options.contains(TestOption::NextDocsAndFreqs) {
            let flags = flags_for_checks;

            for impacts in [false, true] {
                let mut pe1: Box<dyn PostingsEnum> = if impacts {
                    terms_enum.impacts(flags)?
                } else {
                    let mut pe1 = terms_enum.postings(None, flags)?;
                    if random.next_boolean() {
                        // test reuse
                        pe1.advance(max_doc / 2)?;
                        pe1 = terms_enum.postings(Some(pe1), flags)?;
                    }
                    pe1
                };
                let mut pe2 = terms_enum.postings(None, flags)?;

                pe1.next_doc()?;
                pe2.next_doc()?;
                let mut buffer = DocAndFloatFeatureBuffer::new();
                loop {
                    let cur_doc = pe1.doc_id();
                    let up_to = TestUtil::next_int(
                        random,
                        cur_doc,
                        (i64::from(i32::MAX)).min(i64::from(cur_doc) + 512) as i32,
                    );
                    pe1.next_postings(up_to, &mut buffer)?;
                    Assert::assert_equals(buffer.size == 0, cur_doc >= up_to);

                    for i in 0..buffer.size {
                        Assert::assert_true(buffer.docs[i] < up_to);
                        Assert::assert_equals(pe2.doc_id(), buffer.docs[i]);
                        // `assertEquals(0f, pe2.freq(), buffer.features[i])`:
                        // expected 0, actual the freq, delta the feature.
                        Assert::assert_equals_float_msg(
                            None,
                            0.0,
                            pe2.freq()? as f32,
                            buffer.features[i],
                        );
                        pe2.next_doc()?;
                    }

                    Assert::assert_equals(pe1.doc_id(), pe2.doc_id());
                    if pe1.doc_id() == NO_MORE_DOCS {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// The `Option.SKIPPING` block of `verifyEnum`: "First check impacts and
    /// block uptos", then "Now check advancing".
    fn check_impacts(
        &self,
        random: &dyn Random,
        terms_enum: &mut dyn TermsEnum,
        has_norms: bool,
        flags: i32,
        do_check_offsets: bool,
        do_check_payloads: bool,
    ) -> Result<()> {
        let doc_to_norm_fn: fn(i32) -> i64 = if has_norms { doc_to_norm } else { |_| 1 };
        let access = index_package_access();

        // First check impacts and block uptos
        let mut max = -1;
        let mut impacts_copy: Vec<Impact> = Vec::new();

        let mut impacts_enum = terms_enum.impacts(flags)?;
        let mut postings = terms_enum.postings(None, flags)?;
        let mut doc = impacts_enum.next_doc()?;
        loop {
            Assert::assert_equals(postings.next_doc()?, doc);
            if doc == NO_MORE_DOCS {
                break;
            }
            let freq = postings.freq()?;
            Assert::assert_equals_msg(Some("freq is wrong"), freq, impacts_enum.freq()?);
            for _ in 0..freq {
                let pos = postings.next_position()?;
                Assert::assert_equals_msg(
                    Some("position is wrong"),
                    pos,
                    impacts_enum.next_position()?,
                );
                if do_check_offsets {
                    Assert::assert_equals_msg(
                        Some("startOffset is wrong"),
                        postings.start_offset(),
                        impacts_enum.start_offset(),
                    );
                    Assert::assert_equals_msg(
                        Some("endOffset is wrong"),
                        postings.end_offset(),
                        impacts_enum.end_offset(),
                    );
                }
                if do_check_payloads {
                    let expected = postings.get_payload()?.map(<[u8]>::to_vec);
                    let actual = impacts_enum.get_payload()?.map(<[u8]>::to_vec);
                    if expected != actual {
                        Assert::assert_equals_msg(
                            Some("payload is wrong"),
                            payload_to_string(expected.as_deref()),
                            payload_to_string(actual.as_deref()),
                        );
                    }
                }
            }
            if doc > max {
                impacts_enum.advance_shallow(doc)?;
                let impacts = impacts_enum.get_impacts()?;
                access.check_impacts(&*impacts, doc)?;
                impacts_copy = copy_of(&impacts.get_impacts(0));
            }
            let freq = impacts_enum.freq()?;
            let norm = doc_to_norm_fn(doc);
            assert_competitive_impact(&impacts_copy, freq, norm);
            doc = impacts_enum.next_doc()?;
        }

        // Now check advancing
        impacts_enum = terms_enum.impacts(flags)?;
        postings = terms_enum.postings(Some(postings), flags)?;

        max = -1;
        loop {
            let mut doc = impacts_enum.doc_id();
            let advance;
            let target;
            if random.next_boolean() {
                advance = false;
                target = doc.wrapping_add(1);
            } else {
                advance = true;
                // Java's int arithmetic wraps: on the unpositioned enum (doc -1)
                // `NO_MORE_DOCS - doc` overflows to Integer.MIN_VALUE and the
                // target to NO_MORE_DOCS.
                let delta = (1 + random.next_int_bound(512)).min(NO_MORE_DOCS.wrapping_sub(doc));
                target = impacts_enum.doc_id().wrapping_add(delta);
            }

            if target > max && random.next_boolean() {
                let delta = random
                    .next_int_bound(512)
                    .min(NO_MORE_DOCS.wrapping_sub(target));
                max = target.wrapping_add(delta);

                impacts_enum.advance_shallow(target)?;
                let impacts = impacts_enum.get_impacts()?;
                access.check_impacts(&*impacts, target)?;
                impacts_copy = vec![Impact::new(i32::MAX, 1)];
                for level in 0..impacts.num_levels() {
                    if impacts.doc_id_up_to(level) >= max {
                        impacts_copy = copy_of(&impacts.get_impacts(level));
                        break;
                    }
                }
            }

            doc = if advance {
                impacts_enum.advance(target)?
            } else {
                impacts_enum.next_doc()?
            };

            Assert::assert_equals(postings.advance(target)?, doc);
            if doc == NO_MORE_DOCS {
                break;
            }
            let freq = postings.freq()?;
            Assert::assert_equals_msg(Some("freq is wrong"), freq, impacts_enum.freq()?);
            let mut i = 0;
            while i < postings.freq()? {
                let pos = postings.next_position()?;
                Assert::assert_equals_msg(
                    Some("position is wrong"),
                    pos,
                    impacts_enum.next_position()?,
                );
                if do_check_offsets {
                    Assert::assert_equals_msg(
                        Some("startOffset is wrong"),
                        postings.start_offset(),
                        impacts_enum.start_offset(),
                    );
                    Assert::assert_equals_msg(
                        Some("endOffset is wrong"),
                        postings.end_offset(),
                        impacts_enum.end_offset(),
                    );
                }
                if do_check_payloads {
                    let expected = postings.get_payload()?.map(<[u8]>::to_vec);
                    let actual = impacts_enum.get_payload()?.map(<[u8]>::to_vec);
                    if expected != actual {
                        Assert::assert_equals_msg(
                            Some("payload is wrong"),
                            payload_to_string(expected.as_deref()),
                            payload_to_string(actual.as_deref()),
                        );
                    }
                }
                i += 1;
            }

            if doc > max {
                let delta = (1 + random.next_int_bound(512)).min(NO_MORE_DOCS.wrapping_sub(doc));
                max = doc.wrapping_add(delta);
                let impacts = impacts_enum.get_impacts()?;
                access.check_impacts(&*impacts, doc)?;
                impacts_copy = vec![Impact::new(i32::MAX, 1)];
                for level in 0..impacts.num_levels() {
                    if impacts.doc_id_up_to(level) >= max {
                        impacts_copy = copy_of(&impacts.get_impacts(level));
                        break;
                    }
                }
            }

            let freq = impacts_enum.freq()?;
            let norm = doc_to_norm_fn(doc);
            assert_competitive_impact(&impacts_copy, freq, norm);
        }
        Ok(())
    }

    /// `testTerms(fieldsSource, options, maxTestOptions, maxIndexOptions,
    /// alwaysTestMax)`.
    ///
    /// # Errors
    ///
    /// What the postings under test throw.
    ///
    /// # Panics
    ///
    /// With the `AssertionError` of a failed check, and with a thread's
    /// failure (Java's `RuntimeException` wrapping it) under
    /// [`TestOption::Threads`].
    pub fn test_terms(
        &self,
        fields_source: &dyn Fields,
        options: TestOptions,
        max_test_options: IndexOptions,
        max_index_options: IndexOptions,
        always_test_max: bool,
    ) -> Result<()> {
        if options.contains(TestOption::Threads) {
            let num_threads = if *TEST_NIGHTLY {
                TestUtil::next_int(&*self.random, 2, 5)
            } else {
                2
            };
            let randoms: Vec<JdkRandom> = (0..num_threads)
                .map(|_| JdkRandom::new(self.random.next_long()))
                .collect();
            std::thread::scope(|scope| {
                let threads: Vec<_> = randoms
                    .iter()
                    .map(|random| {
                        scope.spawn(move || {
                            if let Err(t) = self.test_terms_one_thread(
                                random,
                                fields_source,
                                options,
                                max_test_options,
                                max_index_options,
                                always_test_max,
                            ) {
                                panic!("java.lang.RuntimeException: {t}");
                            }
                        })
                    })
                    .collect();
                // `Thread.join()`: a thread's uncaught exception does not
                // reach the joining thread in Java; the runner's uncaught
                // exception handler fails the test. Re-raising it here is
                // that failure.
                for thread in threads {
                    if let Err(payload) = thread.join() {
                        std::panic::resume_unwind(payload);
                    }
                }
            });
            Ok(())
        } else {
            self.test_terms_one_thread(
                &*self.random,
                fields_source,
                options,
                max_test_options,
                max_index_options,
                always_test_max,
            )
        }
    }

    fn test_terms_one_thread(
        &self,
        random: &dyn Random,
        fields_source: &dyn Fields,
        options: TestOptions,
        max_test_options: IndexOptions,
        max_index_options: IndexOptions,
        always_test_max: bool,
    ) -> Result<()> {
        let mut thread_state = ThreadState::default();

        // Test random terms/fields:
        let mut term_states: Vec<Box<dyn TermState>> = Vec::new();
        let mut term_state_terms: Vec<FieldAndTerm> = Vec::new();

        let mut supports_ords = true;

        let all_terms = {
            let mut all_terms = self
                .all_terms
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            shuffle(&mut all_terms, random);
            all_terms.clone()
        };
        let mut upto = 0;
        while upto < all_terms.len() {
            let mut use_term_state = !term_states.is_empty() && random.next_int_bound(5) == 1;
            let use_term_ord = supports_ords && !use_term_state && random.next_int_bound(5) == 1;

            let field_and_term;
            let mut term_state_idx = None;

            if !use_term_state {
                // Seek by random field+term:
                field_and_term = all_terms[upto].clone();
                upto += 1;
                if *VERBOSE {
                    if use_term_ord {
                        println!(
                            "\nTEST: seek to term={}:{} using ord={}",
                            field_and_term.field,
                            field_and_term.term.utf8_to_string()?,
                            field_and_term.ord
                        );
                    } else {
                        println!(
                            "\nTEST: seek to term={}:{}",
                            field_and_term.field,
                            field_and_term.term.utf8_to_string()?
                        );
                    }
                }
            } else {
                // Seek by previous saved TermState
                let idx = random.next_int_bound(term_states.len() as i32) as usize;
                field_and_term = term_state_terms[idx].clone();
                if *VERBOSE {
                    println!(
                        "\nTEST: seek using TermState to term={}:{}",
                        field_and_term.field,
                        field_and_term.term.utf8_to_string()?
                    );
                }
                term_state_idx = Some(idx);
            }

            let terms = fields_source.terms(&field_and_term.field)?;
            let terms = terms.unwrap_or_else(|| Assert::fail_without_message());
            let mut terms_enum = terms.iterator()?;

            if let Some(idx) = term_state_idx {
                terms_enum.seek_term_state(&field_and_term.term, &*term_states[idx])?;
            } else if use_term_ord {
                // Try seek by ord sometimes:
                match terms_enum.seek_ord(field_and_term.ord) {
                    Ok(()) => {}
                    Err(LuceneError::UnsupportedOperation(_)) => {
                        supports_ords = false;
                        Assert::assert_true(terms_enum.seek_exact(&field_and_term.term)?);
                    }
                    Err(e) => return Err(e),
                }
            } else {
                Assert::assert_true(terms_enum.seek_exact(&field_and_term.term)?);
            }

            // check we really seeked to the right place
            assert_equals_bytes(None, &field_and_term.term, &terms_enum.term()?);

            let term_ord = if supports_ords {
                match terms_enum.ord() {
                    Ok(ord) => ord,
                    Err(LuceneError::UnsupportedOperation(_)) => {
                        supports_ords = false;
                        -1
                    }
                    Err(e) => return Err(e),
                }
            } else {
                -1
            };

            if term_ord != -1 {
                // PostingsFormat supports ords
                Assert::assert_equals(field_and_term.ord, terms_enum.ord()?);
            }

            let mut saved_term_state = false;

            if options.contains(TestOption::TermState)
                && !use_term_state
                && random.next_int_bound(5) == 1
            {
                // Save away this TermState:
                term_states.push(terms_enum.term_state()?);
                term_state_terms.push(field_and_term.clone());
                saved_term_state = true;
            }

            self.verify_enum(
                random,
                &mut thread_state,
                &field_and_term.field,
                &field_and_term.term,
                &mut *terms_enum,
                max_test_options,
                max_index_options,
                options,
                always_test_max,
            )?;

            // Sometimes save term state after pulling the enum:
            if options.contains(TestOption::TermState)
                && !use_term_state
                && !saved_term_state
                && random.next_int_bound(5) == 1
            {
                // Save away this TermState:
                term_states.push(terms_enum.term_state()?);
                term_state_terms.push(field_and_term.clone());
                use_term_state = true;
            }
            let _ = use_term_state;

            // 10% of the time make sure you can pull another enum
            // from the same term:
            if always_test_max || random.next_int_bound(10) == 7 {
                // Try same term again
                if *VERBOSE {
                    println!("TEST: try enum again on same term");
                }

                self.verify_enum(
                    random,
                    &mut thread_state,
                    &field_and_term.field,
                    &field_and_term.term,
                    &mut *terms_enum,
                    max_test_options,
                    max_index_options,
                    options,
                    always_test_max,
                )?;
            }
        }

        // Test Terms.intersect:
        for field in self.fields.keys() {
            loop {
                let a = AutomatonTestUtil::random_automaton(random);
                let a = Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT)
                    .map_err(|e| LuceneError::illegal_state(e.to_string()))?;
                let ca = CompiledAutomaton::new(a.clone(), false, true, false)?;
                if ca.automaton_type != AutomatonType::Normal {
                    // Keep retrying until we get an A that will really "use" the PF's intersect code:
                    continue;
                }
                // System.out.println("A:\n" + a.toDot());

                let mut start_term: Option<BytesRef> = None;
                if random.next_boolean() {
                    let ras = RandomAcceptedStrings::new(&a);
                    for _ in 0..100 {
                        let code_points = ras.get_random_accepted_string(random);
                        if code_points.is_empty() {
                            continue;
                        }
                        start_term = Some(BytesRef::from_char_sequence(&UnicodeUtil::new_string(
                            &code_points,
                            0,
                            code_points.len(),
                        )?));
                        break;
                    }
                    // Don't allow empty string startTerm:
                    if start_term.is_none() {
                        continue;
                    }
                }
                let terms = fields_source
                    .terms(field)?
                    .ok_or_else(|| LuceneError::illegal_state("java.lang.NullPointerException"))?;
                let mut intersected = terms.intersect(&ca, start_term.as_ref())?;

                let mut intersected_terms = std::collections::HashSet::new();
                while let Some(term) = intersected.next()? {
                    if let Some(start_term) = &start_term {
                        // NOTE: not <=
                        Assert::assert_true(*start_term < term);
                    }
                    intersected_terms.insert(BytesRef::deep_copy_of(&term));
                    self.verify_enum(
                        random,
                        &mut thread_state,
                        field,
                        &term,
                        &mut *intersected,
                        max_test_options,
                        max_index_options,
                        options,
                        always_test_max,
                    )?;
                }

                match &ca.run_automaton {
                    None => Assert::assert_true(intersected_terms.is_empty()),
                    Some(run_automaton) => {
                        for term2 in self.fields[field].keys() {
                            let expected = match &start_term {
                                Some(start_term) if *start_term >= *term2 => false,
                                _ => run_automaton.run_range(
                                    &term2.bytes,
                                    term2.offset,
                                    term2.length,
                                ),
                            };
                            Assert::assert_equals_msg(
                                Some(&format!("term={term2}")),
                                expected,
                                intersected_terms.contains(term2),
                            );
                        }
                    }
                }

                break;
            }
        }
        Ok(())
    }

    /// `testFields(fields)`: iterating the field names to their end.
    ///
    /// # Panics
    ///
    /// With an `AssertionError` when the iterator yields past its end.
    pub fn test_fields(&self, fields: &dyn Fields) {
        let mut iterator = fields.iterator();
        for _ in iterator.by_ref() {}
        Assert::assert_false(iterator.next().is_some());
    }

    /// `testFull(codec, path, options, withPayloads)`: "Indexes all
    /// fields/terms at the specified IndexOptions, and fully tests at that
    /// IndexOptions."
    ///
    /// # Errors
    ///
    /// What writing, reading or closing the postings throws.
    pub fn test_full(
        &mut self,
        codec: Arc<dyn Codec>,
        path: &Path,
        options: IndexOptions,
        with_payloads: bool,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_fs_directory(path);

        // TODO test thread safety of buildIndex too
        let mut fields_producer =
            self.build_index(codec, dir.as_directory(), options, with_payloads, true)?;

        self.test_fields(&*fields_producer);

        let max_index_option = index_option_ordinal(options);

        for &test_options in INDEX_OPTIONS_VALUES
            .iter()
            .take(max_index_option as usize + 1)
        {
            self.test_terms(
                &*fields_producer,
                TestOptions::all_of(),
                test_options,
                options,
                true,
            )?;
            if with_payloads {
                // If we indexed w/ payloads, also test enums w/o accessing payloads:
                self.test_terms(
                    &*fields_producer,
                    TestOptions::complement_of(TestOptions::of(&[TestOption::Payloads])),
                    test_options,
                    options,
                    true,
                )?;
            }
        }

        fields_producer.close()?;
        drop(fields_producer);
        dir.close()
    }
}

/// `IndexOptions.toString()`.
fn index_options_name(options: IndexOptions) -> String {
    format!("{options:?}")
}
