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

//! Port of `org.apache.lucene.tests.index.BaseTermVectorsFormatTestCase`:
//! base class aiming at testing term vectors formats. To test a new format,
//! all you need is to register a new `Codec` which uses it and extend this
//! class and override `getCodec()`.
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BaseTermVectorsFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * [`RandomTokenStream`] / [`RandomDocument`]: Java's `RandomDocument`
//!   holds the token streams it indexes, and `toDocument()` puts them in its
//!   fields. A Rust token stream lives behind an `Rc` (its attribute source
//!   is single-threaded) while a `RandomDocument` crosses threads
//!   (`testClone`), so the document keeps each stream's random data
//!   ([`RandomTokenStreamData`], drawn once by the constructor, as Java's)
//!   and `toDocument()` builds a stream over it. A stream is reset before it
//!   is consumed, so what each indexing reads is what Java's reads.
//! * The `ThreadLocal<PostingsEnum> docsEnum` "to test reuse" is set and
//!   never read in Java; the port keeps it as a thread-local slot.
//! * `testPostingsEnumFreqs`: Java checks `docsOnly.freq()` after reusing
//!   `docsOnly` as `docsOnly2`; a reused Rust enum is moved into the call
//!   that reuses it, so the port reads the freq of `docsOnly2`, which is
//!   the same object whenever the format reuses it.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::OffsetAttribute;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStream, TokenStreamComponents,
};
use rucene::document::{
    Document, Field, FieldType, NumericDocValuesField, Store, StringField, TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::postings_enum::{
    PostingsEnum, POSTINGS_ENUM_ALL as ALL, POSTINGS_ENUM_FREQS as FREQS,
    POSTINGS_ENUM_NONE as NONE, POSTINGS_ENUM_OFFSETS as OFFSETS,
    POSTINGS_ENUM_PAYLOADS as PAYLOADS, POSTINGS_ENUM_POSITIONS as POSITIONS,
};
use rucene::index::terms::{Fields, SeekStatus, Term, Terms};
use rucene::index::{IndexReader, IndexableFieldType};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::NO_MORE_DOCS;
use rucene::util::attribute::{AttributeImpl, AttributeReflector, AttributeSource};
use rucene::util::BytesRef;

use super::base_index_file_format_test_case::{
    expect_throws_lucene, format_test, is_instance_of_java_class, rethrow,
    BaseIndexFileFormatTestCase,
};
use super::base_stored_fields_format_test_case::java_hash_map_key_order;
use crate::analysis::attributes::{char_term_id, payload_id, pos_inc_id, Att};
use crate::analysis::canned_token_stream::CannedTokenStream;
use crate::analysis::mock_tokenizer::MockTokenizer;
use crate::analysis::token::Token;
use crate::analysis::tokenizer_components;
use crate::index::random_index_writer::RandomIndexWriter;
use crate::util::lucene_test_case::{LuceneTestCase, TEST_NIGHTLY};
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BaseTermVectorsFormatTestCase`
/// (`public abstract class BaseTermVectorsFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseTermVectorsFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(term_vectors_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseTermVectorsFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testRareVectors", |c| c.test_rare_vectors()))
            .method(t("testHighFreqs", |c| c.test_high_freqs()))
            .method(t("testLotsOfFields", |c| c.test_lots_of_fields()))
            .method(t("testMixedOptions", |c| c.test_mixed_options()))
            .method(t("testRandom", |c| c.test_random()))
            .method(t("testMerge", |c| c.test_merge()))
            .method(t("testMergeWithDeletes", |c| c.test_merge_with_deletes()))
            .method(t("testMergeWithIndexSort", |c| {
                c.test_merge_with_index_sort()
            }))
            .method(t("testMergeWithIndexSortAndDeletes", |c| {
                c.test_merge_with_index_sort_and_deletes()
            }))
            .method(t("testClone", |c| c.test_clone()))
            .method(t("testPostingsEnumFreqs", |c| c.test_postings_enum_freqs()))
            .method(t("testPostingsEnumPositions", |c| {
                c.test_postings_enum_positions()
            }))
            .method(t("testPostingsEnumOffsets", |c| {
                c.test_postings_enum_offsets()
            }))
            .method(t("testPostingsEnumOffsetsWithoutPositions", |c| {
                c.test_postings_enum_offsets_without_positions()
            }))
            .method(t("testPostingsEnumPayloads", |c| {
                c.test_postings_enum_payloads()
            }))
            .method(t("testPostingsEnumAll", |c| c.test_postings_enum_all()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseTermVectorsFormatTestCase`.
fn term_vectors_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseTermVectorsFormatTestCase {
    c.as_term_vectors().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BaseTermVectorsFormatTestCase",
        )
        .throw()
    })
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// A null result Java would dereference: `NullPointerException`.
fn npe<T>(value: Option<T>) -> T {
    value.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// `assertNull(message, object)`: `<message> expected null, but was:<object>`.
fn assert_null_msg(message: Option<&str>, actual: Option<String>) {
    if let Some(actual) = actual {
        let prefix = message.map_or(String::new(), |m| format!("{m} "));
        Throwable::new(
            "java.lang.AssertionError",
            format!("{prefix}expected null, but was:<{actual}>"),
        )
        .throw()
    }
}

/// `assertNotNull(object)`.
fn assert_not_null(present: bool) {
    if !present {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
    }
}

/// A boxed value as JUnit prints and compares it: its `toString()` or
/// `null`.
#[derive(PartialEq)]
struct Boxed<T>(Option<T>);

impl<T: fmt::Display> fmt::Display for Boxed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            None => f.write_str("null"),
            Some(v) => write!(f, "{v}"),
        }
    }
}

thread_local! {
    /// `docsEnum`: "to test reuse" (set, never read).
    static DOCS_ENUM: RefCell<Option<Box<dyn PostingsEnum>>> = const { RefCell::new(None) };
}

// -----------------------------------------------------------------------------
// Options
// -----------------------------------------------------------------------------

/// Port of `BaseTermVectorsFormatTestCase.Options`: a combination of term
/// vectors options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Options {
    /// `NONE(false, false, false)`.
    None,
    /// `POSITIONS(true, false, false)`.
    Positions,
    /// `OFFSETS(false, true, false)`.
    Offsets,
    /// `POSITIONS_AND_OFFSETS(true, true, false)`.
    PositionsAndOffsets,
    /// `POSITIONS_AND_PAYLOADS(true, false, true)`.
    PositionsAndPayloads,
    /// `POSITIONS_AND_OFFSETS_AND_PAYLOADS(true, true, true)`.
    PositionsAndOffsetsAndPayloads,
}

impl Options {
    /// `Options.values()`, in declaration order.
    pub const VALUES: [Options; 6] = [
        Options::None,
        Options::Positions,
        Options::Offsets,
        Options::PositionsAndOffsets,
        Options::PositionsAndPayloads,
        Options::PositionsAndOffsetsAndPayloads,
    ];

    /// `positions`.
    pub fn positions(self) -> bool {
        matches!(
            self,
            Options::Positions
                | Options::PositionsAndOffsets
                | Options::PositionsAndPayloads
                | Options::PositionsAndOffsetsAndPayloads
        )
    }

    /// `offsets`.
    pub fn offsets(self) -> bool {
        matches!(
            self,
            Options::Offsets
                | Options::PositionsAndOffsets
                | Options::PositionsAndOffsetsAndPayloads
        )
    }

    /// `payloads`.
    pub fn payloads(self) -> bool {
        matches!(
            self,
            Options::PositionsAndPayloads | Options::PositionsAndOffsetsAndPayloads
        )
    }

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            Options::None => "NONE",
            Options::Positions => "POSITIONS",
            Options::Offsets => "OFFSETS",
            Options::PositionsAndOffsets => "POSITIONS_AND_OFFSETS",
            Options::PositionsAndPayloads => "POSITIONS_AND_PAYLOADS",
            Options::PositionsAndOffsetsAndPayloads => "POSITIONS_AND_OFFSETS_AND_PAYLOADS",
        }
    }
}

impl fmt::Display for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// -----------------------------------------------------------------------------
// PermissiveOffsetAttributeImpl
// -----------------------------------------------------------------------------

/// Port of `BaseTermVectorsFormatTestCase.PermissiveOffsetAttributeImpl`:
/// custom impl to test cases that are forbidden by the default
/// `OffsetAttribute` impl.
#[derive(Clone, Debug, Default)]
struct PermissiveOffsetAttributeImpl {
    start: i32,
    end: i32,
}

impl rucene::util::attribute::Attribute for PermissiveOffsetAttributeImpl {}

impl OffsetAttribute for PermissiveOffsetAttributeImpl {
    fn start_offset(&self) -> i32 {
        self.start
    }

    fn set_offset(&mut self, start_offset: i32, end_offset: i32) {
        // no check!
        self.start = start_offset;
        self.end = end_offset;
    }

    fn end_offset(&self) -> i32 {
        self.end
    }
}

impl AttributeImpl for PermissiveOffsetAttributeImpl {
    fn as_offset_attribute(&self) -> Option<&dyn OffsetAttribute> {
        Some(self)
    }

    fn as_offset_attribute_mut(&mut self) -> Option<&mut dyn OffsetAttribute> {
        Some(self)
    }

    fn clear(&mut self) {
        self.start = 0;
        self.end = 0;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        let t = target
            .as_offset_attribute_mut()
            .expect("INVARIANT: the target implements OffsetAttribute");
        t.set_offset(self.start, self.end);
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn OffsetAttribute>(),
            "OffsetAttribute",
            "startOffset",
            &self.start,
        );
        reflector.reflect(
            TypeId::of::<dyn OffsetAttribute>(),
            "OffsetAttribute",
            "endOffset",
            &self.end,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static IDS: std::sync::OnceLock<&'static [TypeId]> = std::sync::OnceLock::new();
        IDS.get_or_init(|| {
            Box::leak(
                vec![
                    TypeId::of::<PermissiveOffsetAttributeImpl>(),
                    TypeId::of::<dyn OffsetAttribute>(),
                ]
                .into_boxed_slice(),
            )
        })
    }

    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        if std::ptr::eq(
            self as *const Self as *const (),
            other.as_any() as *const dyn Any as *const (),
        ) {
            return true;
        }
        match other
            .as_any()
            .downcast_ref::<PermissiveOffsetAttributeImpl>()
        {
            Some(o) => o.start == self.start && o.end == self.end,
            None => false,
        }
    }

    fn hash_code(&self) -> i32 {
        self.start.wrapping_add(31i32.wrapping_mul(self.end))
    }
}

// -----------------------------------------------------------------------------
// RandomTokenStream
// -----------------------------------------------------------------------------

/// The random data of a [`RandomTokenStream`], drawn by its constructor
/// (see the module docs).
#[derive(Debug)]
pub struct RandomTokenStreamData {
    terms: Vec<String>,
    term_bytes: Vec<BytesRef>,
    positions_increments: Vec<i32>,
    positions: Vec<i32>,
    start_offsets: Vec<i32>,
    end_offsets: Vec<i32>,
    payloads: Vec<Option<BytesRef>>,
    freqs: HashMap<String, i32>,
    position_to_terms: HashMap<i32, HashSet<usize>>,
    start_offset_to_terms: HashMap<i32, HashSet<usize>>,
}

impl RandomTokenStreamData {
    /// The body of `RandomTokenStream(int len, String[] sampleTerms,
    /// BytesRef[] sampleTermBytes)`, but the attributes.
    pub fn new(len: i32, sample_terms: &[String], sample_term_bytes: &[BytesRef]) -> Self {
        Self::with_payloads(len, sample_terms, sample_term_bytes, &random_payload)
    }

    /// The constructor of a subclass that overrides `randomPayload()`
    /// (`protected`), which `random_payload` stands for.
    pub fn with_payloads(
        len: i32,
        sample_terms: &[String],
        sample_term_bytes: &[BytesRef],
        random_payload: &dyn Fn() -> Option<BytesRef>,
    ) -> Self {
        let len = len as usize;
        let mut terms = Vec::with_capacity(len);
        let mut term_bytes = Vec::with_capacity(len);
        let mut positions_increments = vec![0i32; len];
        let mut positions = vec![0i32; len];
        let mut start_offsets = vec![0i32; len];
        let mut end_offsets = vec![0i32; len];
        for i in 0..len {
            let o = random().next_int_bound(sample_terms.len() as i32) as usize;
            terms.push(sample_terms[o].clone());
            term_bytes.push(sample_term_bytes[o].clone());
            positions_increments[i] =
                TestUtil::next_int(&*random(), if i == 0 { 1 } else { 0 }, 10);
            if i == 0 {
                start_offsets[i] = TestUtil::next_int(&*random(), 0, 1 << 16);
            } else {
                let max = if LuceneTestCase::rarely() {
                    1 << 16
                } else {
                    20
                };
                start_offsets[i] = start_offsets[i - 1] + TestUtil::next_int(&*random(), 0, max);
            }
            let max = if LuceneTestCase::rarely() {
                1 << 10
            } else {
                20
            };
            end_offsets[i] = start_offsets[i] + TestUtil::next_int(&*random(), 0, max);
        }

        for i in 0..len {
            if i == 0 {
                positions[i] = positions_increments[i] - 1;
            } else {
                positions[i] = positions[i - 1] + positions_increments[i];
            }
        }
        let payloads: Vec<Option<BytesRef>> = if LuceneTestCase::rarely() {
            vec![random_payload(); len]
        } else {
            (0..len).map(|_| random_payload()).collect()
        };

        let mut position_to_terms: HashMap<i32, HashSet<usize>> = HashMap::with_capacity(len);
        let mut start_offset_to_terms: HashMap<i32, HashSet<usize>> = HashMap::with_capacity(len);
        for i in 0..len {
            position_to_terms.entry(positions[i]).or_default().insert(i);
            start_offset_to_terms
                .entry(start_offsets[i])
                .or_default()
                .insert(i);
        }

        let mut freqs: HashMap<String, i32> = HashMap::new();
        for term in &terms {
            *freqs.entry(term.clone()).or_insert(0) += 1;
        }

        RandomTokenStreamData {
            terms,
            term_bytes,
            positions_increments,
            positions,
            start_offsets,
            end_offsets,
            payloads,
            freqs,
            position_to_terms,
            start_offset_to_terms,
        }
    }

    /// `hasPayloads()`.
    pub fn has_payloads(&self) -> bool {
        self.payloads
            .iter()
            .any(|payload| payload.as_ref().is_some_and(|p| p.length > 0))
    }

    /// `getTerms()`.
    pub fn get_terms(&self) -> &[String] {
        &self.terms
    }

    /// `getTermBytes()`.
    pub fn get_term_bytes(&self) -> &[BytesRef] {
        &self.term_bytes
    }

    /// `getPositionsIncrements()`.
    pub fn get_positions_increments(&self) -> &[i32] {
        &self.positions_increments
    }

    /// `getStartOffsets()`.
    pub fn get_start_offsets(&self) -> &[i32] {
        &self.start_offsets
    }

    /// `getEndOffsets()`.
    pub fn get_end_offsets(&self) -> &[i32] {
        &self.end_offsets
    }

    /// `getPayloads()`.
    pub fn get_payloads(&self) -> &[Option<BytesRef>] {
        &self.payloads
    }
}

/// `randomPayload()` (`protected`; see
/// [`RandomTokenStreamData::with_payloads`] for an override).
pub fn random_payload() -> Option<BytesRef> {
    let len = random().next_int_bound(5) as usize;
    if len == 0 {
        return None;
    }
    let mut payload = BytesRef::new(vec![0u8; len]);
    random().next_bytes(&mut payload.bytes);
    payload.length = len;

    Some(LuceneTestCase::new_bytes_ref_from(&payload))
}

/// Port of `BaseTermVectorsFormatTestCase.RandomTokenStream`: produces a
/// random `TokenStream` based off of provided terms (over the data a
/// [`RandomTokenStreamData`] drew; see the module docs).
#[derive(Debug)]
pub struct RandomTokenStream {
    data: Arc<RandomTokenStreamData>,
    source: AttributeSource,
    term_att: Att,
    pi_att: Att,
    p_att: Att,
    i: usize,
}

impl RandomTokenStream {
    /// `RandomTokenStream(int len, String[] sampleTerms, BytesRef[]
    /// sampleTermBytes)`.
    pub fn new(len: i32, sample_terms: &[String], sample_term_bytes: &[BytesRef]) -> Self {
        Self::over(Arc::new(RandomTokenStreamData::new(
            len,
            sample_terms,
            sample_term_bytes,
        )))
    }

    /// A stream over data drawn before: the attributes of Java's
    /// constructor.
    pub fn over(data: Arc<RandomTokenStreamData>) -> Self {
        // `extends TokenStream`: the implicit `super()` builds the source
        // over `DEFAULT_TOKEN_ATTRIBUTE_FACTORY` (`TokenStream.java:86-89`).
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        source.add_attribute_impl_instance(Box::new(PermissiveOffsetAttributeImpl::default()));

        let term_att = Att::add(&mut source, char_term_id()).unwrap_or_else(|e| rethrow(e));
        let pi_att = Att::add(&mut source, pos_inc_id()).unwrap_or_else(|e| rethrow(e));
        source
            .add_attribute_interface::<dyn OffsetAttribute>()
            .unwrap_or_else(|e| rethrow(e));
        let p_att = Att::add(&mut source, payload_id()).unwrap_or_else(|e| rethrow(e));
        RandomTokenStream {
            data,
            source,
            term_att,
            pi_att,
            p_att,
            i: 0,
        }
    }

    /// The data the stream produces.
    pub fn data(&self) -> &Arc<RandomTokenStreamData> {
        &self.data
    }
}

impl TokenStream for RandomTokenStream {
    fn reset(&mut self) -> Result<()> {
        self.i = 0;
        Ok(())
    }

    fn increment_token(&mut self) -> Result<bool> {
        let i = self.i;
        if i < self.data.terms.len() {
            self.source.clear_attributes();
            {
                let mut term = self.term_att.char_term_mut();
                term.set_length(0);
                term.append_string(Some(&self.data.terms[i]));
            }
            self.pi_att
                .pos_inc_mut()
                .set_position_increment(self.data.positions_increments[i]);
            npe(self
                .source
                .get_attribute_interface_mut::<dyn OffsetAttribute>())
            .set_offset(self.data.start_offsets[i], self.data.end_offsets[i]);
            self.p_att
                .payload_mut()
                .set_payload(self.data.payloads[i].clone());
            self.i += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

// -----------------------------------------------------------------------------
// RandomDocument and RandomDocumentFactory
// -----------------------------------------------------------------------------

/// Port of `BaseTermVectorsFormatTestCase.RandomDocument`: randomly
/// generated document: call [`to_document`](Self::to_document) to index it.
#[derive(Debug, Clone)]
pub struct RandomDocument {
    field_names: Vec<String>,
    field_types: Vec<FieldType>,
    token_streams: Vec<Arc<RandomTokenStreamData>>,
}

impl RandomDocument {
    /// `RandomDocument(fieldCount, maxTermCount, options, fieldNames,
    /// sampleTerms, sampleTermBytes)`, with `field_type` the
    /// `fieldType(options)` of the enclosing test.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `field_count` exceeds the names.
    pub fn new(
        field_count: i32,
        max_term_count: i32,
        field_type: FieldType,
        field_names: &[String],
        sample_terms: &[String],
        sample_term_bytes: &[BytesRef],
    ) -> Result<Self> {
        if field_count as usize > field_names.len() {
            return Err(LuceneError::IllegalArgument(String::new()));
        }
        let field_count = field_count as usize;
        let field_types = vec![field_type; field_count];
        let mut names: Vec<String> = Vec::with_capacity(field_count);
        let mut token_streams = Vec::with_capacity(field_count);
        let mut used_file_names: HashSet<String> = HashSet::new();
        for _ in 0..field_count {
            let mut name;
            loop {
                name = RandomPicks::random_from(&*random(), field_names).clone();
                if !used_file_names.contains(&name) {
                    break;
                }
            }
            used_file_names.insert(name.clone());
            names.push(name);
            token_streams.push(Arc::new(RandomTokenStreamData::new(
                TestUtil::next_int(&*random(), 1, max_term_count),
                sample_terms,
                sample_term_bytes,
            )));
        }
        Ok(RandomDocument {
            field_names: names,
            field_types,
            token_streams,
        })
    }

    /// `toDocument()`.
    pub fn to_document(&self) -> Document {
        let mut doc = Document::new();
        for i in 0..self.field_names.len() {
            let stream: Rc<RefCell<dyn TokenStream>> = Rc::new(RefCell::new(
                RandomTokenStream::over(Arc::clone(&self.token_streams[i])),
            ));
            doc.add(Box::new(
                Field::new_with_token_stream(
                    &self.field_names[i],
                    stream,
                    self.field_types[i].clone(),
                )
                .unwrap_or_else(|e| rethrow(e)),
            ));
        }
        doc
    }
}

/// `java.util.HashSet<String>` built with `new HashSet<>()`, remembering
/// what its iteration order depends on.
struct JavaStringHashSet {
    set: HashSet<String>,
    inserted: Vec<String>,
    max_size: usize,
}

impl JavaStringHashSet {
    fn new() -> Self {
        JavaStringHashSet {
            set: HashSet::new(),
            inserted: Vec::new(),
            max_size: 0,
        }
    }

    fn add(&mut self, s: String) {
        if self.set.insert(s.clone()) {
            self.inserted.push(s);
            self.max_size = self.max_size.max(self.set.len());
        }
    }

    fn remove(&mut self, s: &str) {
        if self.set.remove(s) {
            self.inserted.retain(|k| k != s);
        }
    }

    /// `toArray(String[]::new)`: the elements in Java's iteration order.
    fn to_array(&self) -> Vec<String> {
        java_hash_map_key_order(&self.inserted, self.max_size)
    }
}

/// Port of `BaseTermVectorsFormatTestCase.RandomDocumentFactory`: factory
/// for generating random documents, call
/// [`new_document`](Self::new_document) to generate each one.
#[derive(Debug)]
pub struct RandomDocumentFactory {
    field_names: Vec<String>,
    terms: Vec<String>,
    term_bytes: Vec<BytesRef>,
}

impl RandomDocumentFactory {
    /// `RandomDocumentFactory(distinctFieldNames, disctinctTerms)`.
    pub fn new(distinct_field_names: i32, disctinct_terms: i32) -> Self {
        let mut field_names = JavaStringHashSet::new();
        while field_names.set.len() < distinct_field_names as usize {
            field_names.add(TestUtil::random_simple_string(&*random()));
            field_names.remove("id");
        }
        let field_names = field_names.to_array();
        let mut terms = Vec::with_capacity(disctinct_terms as usize);
        let mut term_bytes = Vec::with_capacity(disctinct_terms as usize);
        for _ in 0..disctinct_terms {
            let term = TestUtil::random_realistic_unicode_string(&*random());
            term_bytes.push(LuceneTestCase::new_bytes_ref_str(&term));
            terms.push(term);
        }
        RandomDocumentFactory {
            field_names,
            terms,
            term_bytes,
        }
    }

    /// `newDocument(fieldCount, maxTermCount, options)`, with `case` the
    /// enclosing test.
    pub fn new_document(
        &self,
        case: &(impl BaseTermVectorsFormatTestCase + ?Sized),
        field_count: i32,
        max_term_count: i32,
        options: Options,
    ) -> RandomDocument {
        RandomDocument::new(
            field_count,
            max_term_count,
            case.field_type(options),
            &self.field_names,
            &self.terms,
            &self.term_bytes,
        )
        .unwrap_or_else(|e| rethrow(e))
    }
}

// -----------------------------------------------------------------------------
// The assertions (free, for the threads of testClone)
// -----------------------------------------------------------------------------

/// `assertEquals(RandomDocument doc, Fields fields)`; `read_past` is
/// `getReadPastLastPositionExceptionClass()`.
fn assert_doc_equals(
    read_past: &str,
    doc: &RandomDocument,
    fields: Option<&dyn Fields>,
) -> Result<()> {
    // compare field names
    assert_not_null(fields.is_some());
    let fields = npe(fields);
    Assert::assert_equals(doc.field_names.len() as i64, i64::from(fields.size()));
    let fields1: HashSet<String> = doc.field_names.iter().cloned().collect();
    let fields2: HashSet<String> = fields.iterator().collect();
    Assert::assert_equals(
        super::base_index_file_format_test_case::JavaStringSet(fields1),
        super::base_index_file_format_test_case::JavaStringSet(fields2),
    );

    for i in 0..doc.field_names.len() {
        let terms = fields.terms(&doc.field_names[i])?;
        assert_tk_equals(
            read_past,
            &doc.token_streams[i],
            &doc.field_types[i],
            terms.as_deref(),
        )?;
    }
    Ok(())
}

/// `equals(Object o1, Object o2)` (`protected static`) over two payloads.
fn payload_equals(o1: Option<&BytesRef>, o2: Option<&[u8]>) -> bool {
    match o1 {
        None => o2.is_none(),
        Some(o1) => o2 == Some(o1.slice()),
    }
}

/// `assertEquals(RandomTokenStream tk, FieldType ft, Terms terms)`.
fn assert_tk_equals(
    read_past: &str,
    tk: &RandomTokenStreamData,
    ft: &FieldType,
    terms: Option<&dyn Terms>,
) -> Result<()> {
    let terms = npe(terms);
    Assert::assert_equals(1, terms.doc_count()?);
    let term_count = tk.terms.iter().collect::<HashSet<_>>().len() as i64;
    Assert::assert_equals(term_count, terms.size()?);
    Assert::assert_equals(term_count, terms.sum_doc_freq()?);
    Assert::assert_equals(ft.store_term_vector_positions(), terms.has_positions());
    Assert::assert_equals(ft.store_term_vector_offsets(), terms.has_offsets());
    Assert::assert_equals(
        ft.store_term_vector_payloads() && tk.has_payloads(),
        terms.has_payloads(),
    );
    let unique_terms: HashSet<BytesRef> = tk
        .freqs
        .keys()
        .map(|term| BytesRef::from_char_sequence(term))
        .collect();
    let mut sorted_terms: Vec<BytesRef> = unique_terms.into_iter().collect();
    sorted_terms.sort();
    let mut terms_enum = terms.iterator()?;
    let freq_of = |term: &BytesRef| -> Result<Boxed<i32>> {
        Ok(Boxed(tk.freqs.get(&term.utf8_to_string()?).copied()))
    };
    for sorted_term in &sorted_terms {
        let next_term = terms_enum.next()?;
        Assert::assert_equals(Boxed(Some(sorted_term.clone())), Boxed(next_term));
        Assert::assert_equals(sorted_term.clone(), terms_enum.term()?);
        Assert::assert_equals(1, terms_enum.doc_freq()?);

        let postings_enum = terms_enum.postings(None, FREQS)?;
        let reuse = if random().next_boolean() {
            None
        } else {
            Some(postings_enum)
        };
        let mut postings_enum = terms_enum.postings(reuse, FREQS)?;
        Assert::assert_equals(0, postings_enum.next_doc()?);
        Assert::assert_equals(0, postings_enum.doc_id());
        Assert::assert_equals(
            freq_of(&terms_enum.term()?)?,
            Boxed(Some(postings_enum.freq()?)),
        );
        Assert::assert_equals(NO_MORE_DOCS, postings_enum.next_doc()?);
        DOCS_ENUM.with(|slot| *slot.borrow_mut() = Some(postings_enum));

        let docs_and_positions_enum = terms_enum.postings(None, FREQS)?;
        let reuse = if random().next_boolean() {
            None
        } else {
            Some(docs_and_positions_enum)
        };
        let mut docs_and_positions_enum = terms_enum.postings(reuse, POSITIONS)?;
        if terms.has_positions() || terms.has_offsets() {
            Assert::assert_equals(0, docs_and_positions_enum.next_doc()?);
            let freq = docs_and_positions_enum.freq()?;
            Assert::assert_equals(freq_of(&terms_enum.term()?)?, Boxed(Some(freq)));
            let current = terms_enum.term()?;
            for _ in 0..freq {
                let position = docs_and_positions_enum.next_position()?;
                let indexes: &HashSet<usize> = if terms.has_positions() {
                    let indexes = tk.position_to_terms.get(&position);
                    assert_not_null(indexes.is_some());
                    npe(indexes)
                } else {
                    let indexes = tk
                        .start_offset_to_terms
                        .get(&docs_and_positions_enum.start_offset());
                    assert_not_null(indexes.is_some());
                    npe(indexes)
                };
                if terms.has_positions() {
                    let found_position = indexes.iter().any(|&index| {
                        tk.term_bytes[index] == current && tk.positions[index] == position
                    });
                    Assert::assert_true(found_position);
                }
                if terms.has_offsets() {
                    let found_offset = indexes.iter().any(|&index| {
                        tk.term_bytes[index] == current
                            && tk.start_offsets[index] == docs_and_positions_enum.start_offset()
                            && tk.end_offsets[index] == docs_and_positions_enum.end_offset()
                    });
                    Assert::assert_true(found_offset);
                }
                if terms.has_payloads() {
                    let payload = docs_and_positions_enum.get_payload()?;
                    let found_payload = indexes.iter().any(|&index| {
                        tk.term_bytes[index] == current
                            && payload_equals(tk.payloads[index].as_ref(), payload)
                    });
                    Assert::assert_true(found_payload);
                }
            }
            expect_throws_lucene(
                read_past,
                |e| is_instance_of_java_class(e, read_past),
                || docs_and_positions_enum.next_position(),
            );
            Assert::assert_equals(NO_MORE_DOCS, docs_and_positions_enum.next_doc()?);
        }
        DOCS_ENUM.with(|slot| *slot.borrow_mut() = Some(docs_and_positions_enum));
    }
    assert_null_msg(None, terms_enum.next()?.map(|t| t.to_string()));
    for _ in 0..5 {
        if random().next_boolean() {
            let term = RandomPicks::random_from(&*random(), &tk.term_bytes).clone();
            Assert::assert_true(terms_enum.seek_exact(&term)?);
        } else {
            let term = RandomPicks::random_from(&*random(), &tk.term_bytes).clone();
            Assert::assert_equals(
                SeekStatusDisplay(SeekStatus::FOUND),
                SeekStatusDisplay(terms_enum.seek_ceil(&term)?),
            );
        }
    }
    Ok(())
}

/// A `SeekStatus` as JUnit prints it (`FOUND`, `NOT_FOUND`, `END`).
#[derive(PartialEq)]
struct SeekStatusDisplay(SeekStatus);

impl fmt::Display for SeekStatusDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            SeekStatus::FOUND => "FOUND",
            SeekStatus::NOT_FOUND => "NOT_FOUND",
            SeekStatus::END => "END",
        })
    }
}

/// `docID(reader, id)`: the document whose `id` is `id`.
fn doc_id_of(reader: &Arc<dyn IndexReader>, id: &str) -> Result<i32> {
    let searcher = IndexSearcher::new(Arc::clone(reader))?;
    let top = searcher.search(
        Arc::new(TermQuery::new(Term::new(
            "id",
            BytesRef::from_char_sequence(id),
        ))),
        1,
    )?;
    Ok(top.score_docs[0].doc)
}

// -----------------------------------------------------------------------------
// The postings-enum checks
// -----------------------------------------------------------------------------

/// How a test checks an offset: `assertEquals(v, offset)` or
/// `assertTrue(offset == a || offset == b)`.
#[derive(Clone, Copy)]
enum OffsetIs {
    Eq(i32),
    Either(i32, i32),
}

impl OffsetIs {
    fn check(self, actual: i32) {
        match self {
            OffsetIs::Eq(v) => Assert::assert_equals(v, actual),
            OffsetIs::Either(a, b) => Assert::assert_true(actual == a || actual == b),
        }
    }
}

/// How a test checks a payload: `assertNull`, `assertTrue(payload == null
/// || newBytesRef(s).equals(payload))` (which builds the `BytesRef`, and so
/// draws, only for a payload) or `assertEquals(newBytesRef(s), payload)`.
#[derive(Clone, Copy)]
enum PayloadIs {
    Null,
    NullOr(&'static str),
    Eq(&'static str),
}

impl PayloadIs {
    fn check(self, actual: Option<&[u8]>) {
        match self {
            PayloadIs::Null => {
                assert_null_msg(None, actual.map(|p| BytesRef::new(p.to_vec()).to_string()))
            }
            PayloadIs::NullOr(s) => Assert::assert_true(
                actual.is_none() || Some(LuceneTestCase::new_bytes_ref_str(s).slice()) == actual,
            ),
            PayloadIs::Eq(s) => Assert::assert_equals(
                Boxed(Some(LuceneTestCase::new_bytes_ref_str(s))),
                Boxed(actual.map(|p| BytesRef::new(p.to_vec()))),
            ),
        }
    }
}

/// How a test checks `freq()`: `assertEquals`, `assertTrue(freq == 1 ||
/// freq == 2)`, or not at all.
#[derive(Clone, Copy)]
enum FreqIs {
    Eq(i32),
    OneOrTwo,
    Unchecked,
}

/// One position a test checks: `nextPosition()`, `startOffset()`,
/// `endOffset()`, `getPayload()`.
#[derive(Clone, Copy)]
struct PositionIs(i32, OffsetIs, OffsetIs, PayloadIs);

/// The block every `testPostingsEnum*` test repeats on an enum over the one
/// document: `assertEquals(-1, docID())`, `assertEquals(0, nextDoc())`, the
/// freq check, the position checks, `assertEquals(NO_MORE_DOCS,
/// nextDoc())`.
fn check_one_doc(p: &mut dyn PostingsEnum, freq: FreqIs, positions: &[PositionIs]) -> Result<()> {
    Assert::assert_equals(-1, p.doc_id());
    Assert::assert_equals(0, p.next_doc()?);
    match freq {
        FreqIs::Eq(f) => Assert::assert_equals(f, p.freq()?),
        FreqIs::OneOrTwo => {
            let f = p.freq()?;
            Assert::assert_true(f == 1 || f == 2);
        }
        FreqIs::Unchecked => {}
    }
    for &PositionIs(position, start, end, payload) in positions {
        Assert::assert_equals(position, p.next_position()?);
        start.check(p.start_offset());
        end.check(p.end_offset());
        payload.check(p.get_payload()?);
    }
    Assert::assert_equals(NO_MORE_DOCS, p.next_doc()?);
    Ok(())
}

/// The analyzer of the `testPostingsEnum*` tests: `new Analyzer() {
/// createComponents(f) { return new TokenStreamComponents(new
/// MockTokenizer()); } }`.
#[derive(Debug)]
struct MockTokenizerAnalyzer {
    state: AnalyzerState,
}

impl MockTokenizerAnalyzer {
    fn new() -> Self {
        MockTokenizerAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Analyzer for MockTokenizerAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.index.BaseTermVectorsFormatTestCase$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = MockTokenizer::whitespace_lower_case()?;
        Ok(tokenizer_components(Rc::new(RefCell::new(tokenizer))))
    }
}

/// `new IndexWriterConfig(new Analyzer() {...})` of the
/// `testPostingsEnum*` tests.
fn mock_tokenizer_config() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockTokenizerAnalyzer::new())))
}

/// The two `bar` tokens of `testPostingsEnumPayloads`/`All`, with payloads
/// `newBytesRef("pay1")` and `newBytesRef("pay2")`.
fn payload_tokens() -> Vec<Token> {
    use rucene::analysis::tokenattributes::PayloadAttribute;
    let mut token1 = Token::with_offsets("bar", 0, 3);
    token1.set_payload(Some(LuceneTestCase::new_bytes_ref_str("pay1")));
    let mut token2 = Token::with_offsets("bar", 4, 7);
    token2.set_payload(Some(LuceneTestCase::new_bytes_ref_str("pay2")));
    vec![token1, token2]
}

/// The term vector type of the `testPostingsEnum*` tests: `TYPE_NOT_STORED`
/// with term vectors and the given options.
fn tv_type(positions: bool, offsets: bool, payloads: bool) -> Result<FieldType> {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(true)?;
    if positions {
        ft.set_store_term_vector_positions(true)?;
    }
    if payloads {
        ft.set_store_term_vector_payloads(true)?;
    }
    if offsets {
        ft.set_store_term_vector_offsets(true)?;
    }
    Ok(ft)
}

/// The body of the `testPostingsEnum*` tests: indexes `field` alone with
/// the config `iwc` builds (after the directory, as Java's), then runs on the term vector of `foo` of document 0 the freqs and
/// docs-only checks, and, per flag of `POSITIONS`, `PAYLOADS`, `OFFSETS`,
/// `ALL`, the positions checks `positions` gives (a fresh enum, then the
/// enum reused); `flags_only` runs Java's `testPostingsEnumFreqs` loop over
/// the six flags instead.
fn postings_enum_test(
    iwc: impl FnOnce() -> IndexWriterConfig,
    field: impl FnOnce() -> Result<Field>,
    flags_only: bool,
    positions: &dyn Fn(i32) -> [PositionIs; 2],
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    {
        let iw = IndexWriter::new(dir.as_directory(), iwc())?;
        let mut doc = Document::new();
        doc.add(Box::new(field()?));
        iw.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*iw)?;

        let terms = npe(
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?
                .term_vectors()?
                .get_field(0, "foo")?,
        );
        let mut terms_enum = terms.iterator()?;
        let bar = LuceneTestCase::new_bytes_ref_str("bar");
        Assert::assert_equals(Boxed(Some(bar)), Boxed(terms_enum.next()?));

        // simple use (FREQS)
        let mut postings = terms_enum.postings(None, FREQS)?;
        check_one_doc(&mut *postings, FreqIs::Eq(2), &[])?;

        // termsenum reuse (FREQS)
        let mut postings2 = terms_enum.postings(Some(postings), FREQS)?;
        // and it had better work
        check_one_doc(&mut *postings2, FreqIs::Eq(2), &[])?;

        // asking for docs only: ok
        let mut docs_only = terms_enum.postings(None, NONE)?;
        // we don't define what it is, but if its something else, we should look into it?
        check_one_doc(&mut *docs_only, FreqIs::OneOrTwo, &[])?;
        // reuse that too
        let mut docs_only2 = terms_enum.postings(Some(docs_only), NONE)?;
        // and it had better work
        // we don't define what it is, but if its something else, we should look into it?
        check_one_doc(&mut *docs_only2, FreqIs::OneOrTwo, &[])?;

        if flags_only {
            // asking for any flags: ok
            for flag in [NONE, FREQS, POSITIONS, PAYLOADS, OFFSETS, ALL] {
                let freq = if flag != NONE {
                    FreqIs::Eq(2)
                } else {
                    FreqIs::Unchecked
                };
                let mut postings = terms_enum.postings(None, flag)?;
                check_one_doc(&mut *postings, freq, &[])?;
                // reuse that too
                let mut postings2 = terms_enum.postings(Some(postings), flag)?;
                // and it had better work
                check_one_doc(&mut *postings2, freq, &[])?;
            }
        } else {
            for flag in [POSITIONS, PAYLOADS, OFFSETS, ALL] {
                let mut docs_and_positions_enum = terms_enum.postings(None, flag)?;
                check_one_doc(
                    &mut *docs_and_positions_enum,
                    FreqIs::Eq(2),
                    &positions(flag),
                )?;
                // now reuse
                let mut docs_and_positions_enum2 =
                    terms_enum.postings(Some(docs_and_positions_enum), flag)?;
                check_one_doc(
                    &mut *docs_and_positions_enum2,
                    FreqIs::Eq(2),
                    &positions(flag),
                )?;
            }
        }

        iw.close()?;
        drop(iw);
        drop((terms_enum, terms, postings2, docs_only2));
        reader.close()?;
        drop(reader);
    }
    dir.close()
}

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `BaseTermVectorsFormatTestCase`.
pub trait BaseTermVectorsFormatTestCase: BaseIndexFileFormatTestCase {
    /// `validOptions()`.
    fn valid_options(&self) -> Vec<Options> {
        Options::VALUES.to_vec()
    }

    /// `randomOptions()`.
    fn random_options(&self) -> Options {
        *RandomPicks::random_from(&*random(), &self.valid_options())
    }

    /// `fieldType(options)`.
    fn field_type(&self, options: Options) -> FieldType {
        let ft = FieldType::new_from(TextField::type_not_stored());
        let set = (|| -> Result<()> {
            ft.set_store_term_vectors(true)?;
            ft.set_store_term_vector_positions(options.positions())?;
            ft.set_store_term_vector_offsets(options.offsets())?;
            ft.set_store_term_vector_payloads(options.payloads())
        })();
        set.unwrap_or_else(|e| rethrow(e));
        ft.freeze();
        ft
    }

    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn term_vectors_add_random_fields(&self, doc: &mut Document) {
        for opts in self.valid_options() {
            let ft = self.field_type(opts);
            let num_fields = random().next_int_bound(5);
            for _ in 0..num_fields {
                doc.add(Box::new(
                    Field::new(
                        &format!("f_{opts}"),
                        TestUtil::random_simple_string_max(&*random(), 2),
                        ft.clone(),
                    )
                    .unwrap_or_else(|e| rethrow(e)),
                ));
            }
        }
    }

    /// `getReadPastLastPositionExceptionClass()`: the Java class name.
    fn get_read_past_last_position_exception_class(&self) -> &'static str {
        "java.lang.IllegalStateException"
    }

    /// `assertEquals(RandomDocument doc, Fields fields)`.
    fn assert_equals_doc(&self, doc: &RandomDocument, fields: Option<&dyn Fields>) -> Result<()> {
        assert_doc_equals(
            self.get_read_past_last_position_exception_class(),
            doc,
            fields,
        )
    }

    /// `assertEquals(RandomTokenStream tk, FieldType ft, Terms terms)`.
    fn assert_equals_tk(
        &self,
        tk: &RandomTokenStreamData,
        ft: &FieldType,
        terms: Option<&dyn Terms>,
    ) -> Result<()> {
        assert_tk_equals(
            self.get_read_past_last_position_exception_class(),
            tk,
            ft,
            terms,
        )
    }

    /// `addId(doc, id)`.
    fn add_id(&self, mut doc: Document, id: &str) -> Document {
        doc.add(Box::new(
            StringField::new("id", id.to_string(), Store::NO).unwrap_or_else(|e| rethrow(e)),
        ));
        doc
    }

    /// `docID(reader, id)`.
    fn doc_id(&self, reader: &Arc<dyn IndexReader>, id: &str) -> Result<i32> {
        doc_id_of(reader, id)
    }

    /// `testRareVectors()`: only one doc with vectors.
    fn test_rare_vectors(&self) -> Result<()> {
        let doc_factory = RandomDocumentFactory::new(10, 20);
        for options in self.valid_options() {
            let num_docs = LuceneTestCase::at_least(200);
            let doc_with_vectors = random().next_int_bound(num_docs);
            let empty_doc = Document::new();
            let mut dir = LuceneTestCase::new_directory();
            {
                let writer =
                    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
                let doc = doc_factory.new_document(
                    self,
                    TestUtil::next_int(&*random(), 1, 3),
                    20,
                    options,
                );
                for i in 0..num_docs {
                    if i == doc_with_vectors {
                        writer.add_document(&self.add_id(doc.to_document(), "42"))?;
                    } else {
                        writer.add_document(&empty_doc)?;
                    }
                }
                let reader: Arc<dyn IndexReader> = writer.get_reader()?;
                let doc_with_vectors_id = self.doc_id(&reader, "42")?;
                let term_vectors = reader.term_vectors()?;
                for _ in 0..10 {
                    let doc_id = random().next_int_bound(num_docs);
                    let fields = term_vectors.get(doc_id)?;
                    if doc_id == doc_with_vectors_id {
                        self.assert_equals_doc(&doc, fields.as_deref())?;
                    } else {
                        assert_null_msg(None, fields.map(|_| "Fields".to_string()));
                    }
                }
                let fields = term_vectors.get(doc_with_vectors_id)?;
                self.assert_equals_doc(&doc, fields.as_deref())?;
                drop(term_vectors);
                reader.close()?;
                drop(reader);
                writer.close()?;
                drop(writer);
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `testHighFreqs()`.
    fn test_high_freqs(&self) -> Result<()> {
        let doc_factory = RandomDocumentFactory::new(3, 5);
        for options in self.valid_options() {
            if options == Options::None {
                continue;
            }
            let mut dir = LuceneTestCase::new_directory();
            {
                let writer =
                    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
                let field_count = TestUtil::next_int(&*random(), 1, 2);
                let doc = doc_factory.new_document(
                    self,
                    field_count,
                    LuceneTestCase::at_least(2000),
                    options,
                );
                writer.add_document(&doc.to_document())?;
                let reader: Arc<dyn IndexReader> = writer.get_reader()?;
                self.assert_equals_doc(&doc, reader.term_vectors()?.get(0)?.as_deref())?;
                reader.close()?;
                drop(reader);
                writer.close()?;
                drop(writer);
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `testLotsOfFields()`.
    fn test_lots_of_fields(&self) -> Result<()> {
        let field_count = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(100)
        } else {
            LuceneTestCase::at_least(10)
        };
        let doc_factory = RandomDocumentFactory::new(field_count, 10);
        for options in self.valid_options() {
            let mut dir = LuceneTestCase::new_directory();
            {
                let writer =
                    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
                let count = TestUtil::next_int(&*random(), 5, field_count);
                let doc = doc_factory.new_document(self, count, 5, options);
                writer.add_document(&doc.to_document())?;
                let reader: Arc<dyn IndexReader> = writer.get_reader()?;
                self.assert_equals_doc(&doc, reader.term_vectors()?.get(0)?.as_deref())?;
                reader.close()?;
                drop(reader);
                writer.close()?;
                drop(writer);
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `testMixedOptions()`: different options for the same field.
    fn test_mixed_options(&self) -> Result<()> {
        let num_fields = TestUtil::next_int(&*random(), 1, 3);
        let doc_factory = RandomDocumentFactory::new(num_fields, 10);
        for options1 in self.valid_options() {
            for options2 in self.valid_options() {
                if options1 == options2 {
                    continue;
                }
                let mut dir = LuceneTestCase::new_directory();
                {
                    let writer =
                        RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
                    let doc1 = doc_factory.new_document(self, num_fields, 20, options1);
                    let doc2 = doc_factory.new_document(self, num_fields, 20, options2);
                    writer.add_document(&self.add_id(doc1.to_document(), "1"))?;
                    writer.add_document(&self.add_id(doc2.to_document(), "2"))?;
                    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
                    let doc1_id = self.doc_id(&reader, "1")?;
                    self.assert_equals_doc(&doc1, reader.term_vectors()?.get(doc1_id)?.as_deref())?;
                    let doc2_id = self.doc_id(&reader, "2")?;
                    self.assert_equals_doc(&doc2, reader.term_vectors()?.get(doc2_id)?.as_deref())?;
                    reader.close()?;
                    drop(reader);
                    writer.close()?;
                    drop(writer);
                }
                dir.close()?;
            }
        }
        Ok(())
    }

    /// `testRandom()`.
    fn test_random(&self) -> Result<()> {
        let doc_factory = RandomDocumentFactory::new(5, 20);
        let num_docs = LuceneTestCase::at_least(50);
        let mut docs = Vec::with_capacity(num_docs as usize);
        for _ in 0..num_docs {
            let field_count = TestUtil::next_int(&*random(), 1, 3);
            let max_term_count = TestUtil::next_int(&*random(), 10, 50);
            let options = self.random_options();
            docs.push(doc_factory.new_document(self, field_count, max_term_count, options));
        }
        let mut dir = LuceneTestCase::new_directory();
        {
            let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
            for (i, doc) in docs.iter().enumerate() {
                writer.add_document(&self.add_id(doc.to_document(), &i.to_string()))?;
            }
            let reader: Arc<dyn IndexReader> = writer.get_reader()?;
            let term_vectors = reader.term_vectors()?;
            for (i, doc) in docs.iter().enumerate() {
                let doc_id = self.doc_id(&reader, &i.to_string())?;
                self.assert_equals_doc(doc, term_vectors.get(doc_id)?.as_deref())?;
            }
            drop(term_vectors);
            reader.close()?;
            drop(reader);
            writer.close()?;
            drop(writer);
        }
        dir.close()
    }

    /// `doTestMerge(indexSort, allowDeletes)` (`private`).
    fn do_test_merge(&self, index_sort: Option<Sort>, allow_deletes: bool) -> Result<()> {
        let doc_factory = RandomDocumentFactory::new(5, 20);
        let num_docs = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(100)
        } else {
            LuceneTestCase::at_least(10)
        };
        for options in self.valid_options() {
            let mut docs: HashMap<String, RandomDocument> = HashMap::new();
            let mut inserted: Vec<String> = Vec::new();
            for i in 0..num_docs {
                let field_count = TestUtil::next_int(&*random(), 1, 3);
                let doc = doc_factory.new_document(
                    self,
                    field_count,
                    LuceneTestCase::at_least(10),
                    options,
                );
                docs.insert(i.to_string(), doc);
                inserted.push(i.to_string());
            }
            let mut dir = LuceneTestCase::new_directory();
            {
                let mut iwc = LuceneTestCase::new_index_writer_config();
                if let Some(index_sort) = &index_sort {
                    iwc.set_index_sort(index_sort.clone())?;
                }
                let writer =
                    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
                let mut live_doc_ids: Vec<String> = Vec::new();
                let mut ids: Vec<String> = java_hash_map_key_order(&inserted, docs.len());
                randomizedtesting::jdk::util::shuffle(&mut ids, &*random());
                let verify_term_vectors = |live_doc_ids: &[String]| -> Result<()> {
                    let reader = self.maybe_wrap_with_merging_reader(writer.get_reader()?)?;
                    let checked = (|| -> Result<()> {
                        let reader = Arc::clone(&reader) as Arc<dyn IndexReader>;
                        let term_vectors = reader.term_vectors()?;
                        for id in live_doc_ids {
                            let doc_id = self.doc_id(&reader, id)?;
                            self.assert_equals_doc(
                                &docs[id],
                                term_vectors.get(doc_id)?.as_deref(),
                            )?;
                        }
                        Ok(())
                    })();
                    let closed = reader.close();
                    checked?;
                    closed
                };
                for id in &ids {
                    let mut doc = self.add_id(docs[id].to_document(), id);
                    if let Some(index_sort) = &index_sort {
                        for sort_field in index_sort.fields() {
                            doc.add(Box::new(NumericDocValuesField::new(
                                npe(sort_field.field()),
                                i64::from(TestUtil::next_int(&*random(), 0, 1024)),
                            )));
                        }
                    }
                    if random().next_int_bound(100) < 5 {
                        // add via foreign writer
                        let mut other_iwc = LuceneTestCase::new_index_writer_config();
                        if let Some(index_sort) = &index_sort {
                            other_iwc.set_index_sort(index_sort.clone())?;
                        }
                        let mut other_dir = LuceneTestCase::new_directory();
                        let other_iw = RandomIndexWriter::new_with_config(
                            &*random(),
                            other_dir.as_directory(),
                            other_iwc,
                        )?;
                        let added = (|| -> Result<()> {
                            other_iw.add_document(&doc)?;
                            let other_reader = other_iw.get_reader()?;
                            let slowly = TestUtil::add_indexes_slowly(
                                writer.w(),
                                std::slice::from_ref(&other_reader),
                            );
                            let closed = other_reader.close();
                            slowly?;
                            closed
                        })();
                        let closed_iw = other_iw.close();
                        drop(other_iw);
                        let closed_dir = other_dir.close();
                        added?;
                        closed_iw?;
                        closed_dir?;
                    } else {
                        writer.add_document(&doc)?;
                    }
                    live_doc_ids.push(id.clone());
                    if allow_deletes && random().next_int_bound(100) < 20 {
                        let index = random().next_int_bound(live_doc_ids.len() as i32) as usize;
                        let delete_id = live_doc_ids.remove(index);
                        writer.delete_documents(Term::new(
                            "id",
                            BytesRef::from_char_sequence(&delete_id),
                        ))?;
                    }
                    if LuceneTestCase::rarely() {
                        writer.commit()?;
                        verify_term_vectors(&live_doc_ids)?;
                    }
                    if LuceneTestCase::rarely() {
                        writer.force_merge(1)?;
                        verify_term_vectors(&live_doc_ids)?;
                    }
                }
                verify_term_vectors(&live_doc_ids)?;
                writer.force_merge(1)?;
                verify_term_vectors(&live_doc_ids)?;
                // `IOUtils.close(writer, dir)`
                writer.close()?;
                drop(writer);
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `testMerge()`.
    fn test_merge(&self) -> Result<()> {
        self.do_test_merge(None, false)
    }

    /// `testMergeWithDeletes()`.
    fn test_merge_with_deletes(&self) -> Result<()> {
        self.do_test_merge(None, true)
    }

    /// `testMergeWithIndexSort()`.
    fn test_merge_with_index_sort(&self) -> Result<()> {
        let sort = random_long_sort()?;
        self.do_test_merge(Some(sort), false)
    }

    /// `testMergeWithIndexSortAndDeletes()`.
    fn test_merge_with_index_sort_and_deletes(&self) -> Result<()> {
        let sort = random_long_sort()?;
        self.do_test_merge(Some(sort), true)
    }

    /// `testClone()`: run random tests from different threads to make sure
    /// the per-thread clones don't share mutable data.
    fn test_clone(&self) -> Result<()> {
        let doc_factory = RandomDocumentFactory::new(5, 20);
        let num_docs = LuceneTestCase::at_least(50);
        for options in self.valid_options() {
            let mut docs = Vec::with_capacity(num_docs as usize);
            for _ in 0..num_docs {
                let field_count = TestUtil::next_int(&*random(), 1, 3);
                docs.push(doc_factory.new_document(
                    self,
                    field_count,
                    LuceneTestCase::at_least(10),
                    options,
                ));
            }
            let docs = Arc::new(docs);
            let mut dir = LuceneTestCase::new_directory();
            let thrown = {
                let writer =
                    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
                for (i, doc) in docs.iter().enumerate() {
                    writer.add_document(&self.add_id(doc.to_document(), &i.to_string()))?;
                }
                let reader: Arc<dyn IndexReader> = writer.get_reader()?;
                let term_vectors = reader.term_vectors()?;
                for (i, doc) in docs.iter().enumerate() {
                    let doc_id = self.doc_id(&reader, &i.to_string())?;
                    self.assert_equals_doc(doc, term_vectors.get(doc_id)?.as_deref())?;
                }
                drop(term_vectors);

                let exception: Arc<Mutex<Option<Throwable>>> = Arc::new(Mutex::new(None));
                let read_past = self.get_read_past_last_position_exception_class();
                let mut threads: Vec<Thread> = Vec::with_capacity(2);
                for _ in 0..2 {
                    let (reader, docs, exception) = (
                        Arc::clone(&reader),
                        Arc::clone(&docs),
                        Arc::clone(&exception),
                    );
                    threads.push(Thread::unnamed(move || {
                    let run = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
                        let term_vectors = reader.term_vectors()?;
                        let mut i = 0;
                        while i < LuceneTestCase::at_least(100) {
                            let idx = random().next_int_bound(num_docs) as usize;
                            let doc_id = doc_id_of(&reader, &idx.to_string())?;
                            assert_doc_equals(read_past, &docs[idx], term_vectors.get(doc_id)?.as_deref())?;
                            i += 1;
                        }
                        Ok(())
                    }));
                    let thrown = match run {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => {
                            let (class, message) =
                                crate::analysis::base_token_stream_test_case::java_class_and_message(&e);
                            Some(Throwable::new(class, message))
                        }
                        Err(payload) => Some(Throwable::from_panic(payload)),
                    };
                    if let Some(t) = thrown {
                        *exception.lock().unwrap_or_else(PoisonError::into_inner) = Some(t);
                    }
                }));
                }
                for thread in &threads {
                    thread.start();
                }
                for thread in &threads {
                    if let Err(payload) = thread.join() {
                        std::panic::resume_unwind(payload);
                    }
                }
                reader.close()?;
                drop(reader);
                writer.close()?;
                drop(writer);
                let thrown = exception
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                thrown
            };
            dir.close()?;
            assert_null_msg(
                Some("One thread threw an exception"),
                thrown.map(|t| t.to_string()),
            );
        }
        Ok(())
    }

    /// `testPostingsEnumFreqs()`.
    fn test_postings_enum_freqs(&self) -> Result<()> {
        postings_enum_test(
            mock_tokenizer_config,
            || Field::new("foo", "bar bar".to_string(), tv_type(false, false, false)?),
            true,
            &|_| unreachable!("no position checks"),
        )
    }

    /// `testPostingsEnumPositions()`.
    fn test_postings_enum_positions(&self) -> Result<()> {
        use OffsetIs::Eq as E;
        postings_enum_test(
            mock_tokenizer_config,
            || Field::new("foo", "bar bar".to_string(), tv_type(true, false, false)?),
            false,
            &|_| {
                [
                    PositionIs(0, E(-1), E(-1), PayloadIs::Null),
                    PositionIs(1, E(-1), E(-1), PayloadIs::Null),
                ]
            },
        )
    }

    /// `testPostingsEnumOffsets()`.
    fn test_postings_enum_offsets(&self) -> Result<()> {
        use OffsetIs::{Either, Eq as E};
        postings_enum_test(
            mock_tokenizer_config,
            || Field::new("foo", "bar bar".to_string(), tv_type(true, true, false)?),
            false,
            &|flag| {
                if flag == OFFSETS || flag == ALL {
                    [
                        PositionIs(0, E(0), E(3), PayloadIs::Null),
                        PositionIs(1, E(4), E(7), PayloadIs::Null),
                    ]
                } else {
                    // we don't define what it is, but if its something else, we should look into it?
                    [
                        PositionIs(0, Either(-1, 0), Either(-1, 3), PayloadIs::Null),
                        PositionIs(1, Either(-1, 4), Either(-1, 7), PayloadIs::Null),
                    ]
                }
            },
        )
    }

    /// `testPostingsEnumOffsetsWithoutPositions()`.
    fn test_postings_enum_offsets_without_positions(&self) -> Result<()> {
        use OffsetIs::{Either, Eq as E};
        postings_enum_test(
            mock_tokenizer_config,
            || Field::new("foo", "bar bar".to_string(), tv_type(false, true, false)?),
            false,
            &|flag| {
                if flag == OFFSETS || flag == ALL {
                    [
                        PositionIs(-1, E(0), E(3), PayloadIs::Null),
                        PositionIs(-1, E(4), E(7), PayloadIs::Null),
                    ]
                } else {
                    // we don't define what it is, but if its something else, we should look into it?
                    [
                        PositionIs(-1, Either(-1, 0), Either(-1, 3), PayloadIs::Null),
                        PositionIs(-1, Either(-1, 4), Either(-1, 7), PayloadIs::Null),
                    ]
                }
            },
        )
    }

    /// `testPostingsEnumPayloads()`.
    fn test_postings_enum_payloads(&self) -> Result<()> {
        use OffsetIs::Eq as E;
        postings_enum_test(
            || IndexWriterConfig::with_analyzer(None),
            || {
                let tokens = payload_tokens();
                let ft = tv_type(true, false, true)?;
                let stream: Rc<RefCell<dyn TokenStream>> =
                    Rc::new(RefCell::new(CannedTokenStream::new(tokens)));
                Field::new_with_token_stream("foo", stream, ft)
            },
            false,
            &|flag| {
                if flag == PAYLOADS || flag == ALL {
                    [
                        PositionIs(0, E(-1), E(-1), PayloadIs::Eq("pay1")),
                        PositionIs(1, E(-1), E(-1), PayloadIs::Eq("pay2")),
                    ]
                } else {
                    // we don't define what it is, but if its something else, we should look into it?
                    [
                        PositionIs(0, E(-1), E(-1), PayloadIs::NullOr("pay1")),
                        PositionIs(1, E(-1), E(-1), PayloadIs::NullOr("pay2")),
                    ]
                }
            },
        )
    }

    /// `testPostingsEnumAll()`.
    fn test_postings_enum_all(&self) -> Result<()> {
        use OffsetIs::{Either, Eq as E};
        postings_enum_test(
            || IndexWriterConfig::with_analyzer(None),
            || {
                let tokens = payload_tokens();
                let ft = tv_type(true, true, true)?;
                let stream: Rc<RefCell<dyn TokenStream>> =
                    Rc::new(RefCell::new(CannedTokenStream::new(tokens)));
                Field::new_with_token_stream("foo", stream, ft)
            },
            false,
            &|flag| {
                let payload = |s| {
                    if flag == PAYLOADS || flag == ALL {
                        PayloadIs::Eq(s)
                    } else {
                        // we don't define what it is, but if its something else, we should look into it?
                        PayloadIs::NullOr(s)
                    }
                };
                if flag == OFFSETS || flag == ALL {
                    [
                        PositionIs(0, E(0), E(3), payload("pay1")),
                        PositionIs(1, E(4), E(7), payload("pay2")),
                    ]
                } else {
                    // we don't define what it is, but if its something else, we should look into it?
                    [
                        PositionIs(0, Either(-1, 0), Either(-1, 3), payload("pay1")),
                        PositionIs(1, Either(-1, 4), Either(-1, 7), payload("pay2")),
                    ]
                }
            },
        )
    }
}

/// The index sort of `testMergeWithIndexSort*`: one or two `LONG` sort
/// fields `sort_field_<i>`.
fn random_long_sort() -> Result<Sort> {
    let count = TestUtil::next_int(&*random(), 1, 2);
    let mut sort_fields = Vec::with_capacity(count as usize);
    for i in 0..count {
        sort_fields.push(SortField::new(
            Some(format!("sort_field_{i}")),
            SortFieldType::Long,
        )?);
    }
    Sort::new_fields(sort_fields)
}
