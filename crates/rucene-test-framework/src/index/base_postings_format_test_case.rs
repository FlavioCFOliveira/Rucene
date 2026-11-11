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

//! Port of `org.apache.lucene.tests.index.BasePostingsFormatTestCase`:
//! abstract class to do basic tests for a postings format. NOTE: this test
//! focuses on the postings (docs/freqs/positions/payloads/offsets) impl, not
//! the terms dict. The [stretch] goal is for this test to be so thorough in
//! testing a new PostingsFormat that if this test passes, then all Lucene
//! tests should also pass. Ie, if there is some bug in a given PostingsFormat
//! that this test fails to catch then this test needs to be improved!
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BasePostingsFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `PostingsEnum` reuse (`checkReuse`, `assertReused` and the tests that
//!   keep using an enum after offering it for reuse): `TermsEnum::postings`
//!   takes the enum to reuse by value, so `assertSame(p1, p2)` compares the
//!   address of the enum offered with the one returned (equal exactly when it
//!   was reused); after the call Java keeps using `p1`, which is `p2` when it
//!   was reused, and the port uses the returned enum. A format that does not
//!   reuse (Java reads the old enum) is the only case the two differ.
//! * `testInvertedWrite`: Java's anonymous `FilterCodec` builds its
//!   `PostingsFormat` on every `postingsFormat()` call and captures the
//!   calling thread as `mainThread`; the port's `FilterCodec` holds one
//!   format, built on the test thread, so the assertion that a flush runs on
//!   `mainThread` names the test thread. `termFreqs` is a
//!   `ConcurrentHashMap` whose key iteration order decides the order of the
//!   random draws of the seek pass; the port keeps the keys in the order
//!   JDK 25's `ConcurrentHashMap` iterates them ([`ConcurrentHashMapKeys`]).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::postings::{FieldsConsumer, FieldsProducer, PostingsFormat};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{Codec, FilterCodec};
use rucene::document::{Document, Field, FieldType, Store, StringField, TextField};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::{
    PostingsEnum, POSTINGS_ENUM_ALL as ALL, POSTINGS_ENUM_FREQS as FREQS,
    POSTINGS_ENUM_NONE as NONE, POSTINGS_ENUM_OFFSETS as OFFSETS,
    POSTINGS_ENUM_PAYLOADS as PAYLOADS, POSTINGS_ENUM_POSITIONS as POSITIONS,
};
use rucene::index::terms::{Fields, SeekStatus, Term, TermsEnum};
use rucene::index::{IndexOptions, IndexReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::term_query::TermQuery;
use rucene::search::NO_MORE_DOCS;
use rucene::store::Context;
use rucene::util::BytesRef;

use super::base_index_file_format_test_case::{format_test, rethrow, BaseIndexFileFormatTestCase};
use crate::analysis::canned_token_stream::CannedTokenStream;
use crate::analysis::mock_tokenizer::MockTokenizer;
use crate::analysis::token::Token;
use crate::analysis::{tokenizer_components, MockAnalyzer};
use crate::index::random_index_writer::RandomIndexWriter;
use crate::index::random_postings_tester::{RandomPostingsTester, TestOptions};
use crate::index::MismatchedCodecReader;
use crate::util::line_file_docs::LineFileDocs;
use crate::util::lucene_test_case::{LuceneTestCase, Nightly};
use crate::util::ram_usage_tester::{
    aligned_shallow_instance_size, JavaObject, RamObject, RamUsageTester,
};
use crate::util::TestUtil;

/// `static RandomPostingsTester postingsTester`.
static POSTINGS_TESTER: Mutex<Option<Arc<Mutex<RandomPostingsTester>>>> = Mutex::new(None);

/// `postingsTester`, which `@BeforeClass createPostings()` built.
///
/// # Panics
///
/// `NullPointerException` outside the class, as Java's `null` field.
pub fn postings_tester() -> Arc<Mutex<RandomPostingsTester>> {
    POSTINGS_TESTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        })
}

/// A null result Java would dereference: `NullPointerException`.
fn npe<T>(value: Option<T>) -> T {
    value.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// The declaration of `org.apache.lucene.tests.index.BasePostingsFormatTestCase`
/// (`public abstract class BasePostingsFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its class hooks and its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BasePostingsFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(postings_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BasePostingsFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(
                Method::new("createPostings")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.io.IOException"])
                    .annotate(randomizedtesting::junit::BeforeClass)
                    .static_body(|| {
                        let tester = RandomPostingsTester::new(LuceneTestCase::random())
                            .unwrap_or_else(|e| rethrow(e));
                        *POSTINGS_TESTER
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner) =
                            Some(Arc::new(Mutex::new(tester)));
                    }),
            )
            .method(
                Method::new("afterClass")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(randomizedtesting::junit::AfterClass)
                    .static_body(|| {
                        *POSTINGS_TESTER
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner) = None;
                    }),
            )
            .method(t("testDocsOnly", |c| c.test_docs_only()))
            .method(t("testDocsAndFreqs", |c| c.test_docs_and_freqs()))
            .method(t("testDocsAndFreqsAndPositions", |c| {
                c.test_docs_and_freqs_and_positions()
            }))
            .method(t("testDocsAndFreqsAndPositionsAndPayloads", |c| {
                c.test_docs_and_freqs_and_positions_and_payloads()
            }))
            .method(t("testDocsAndFreqsAndPositionsAndOffsets", |c| {
                c.test_docs_and_freqs_and_positions_and_offsets()
            }))
            .method(t(
                "testDocsAndFreqsAndPositionsAndOffsetsAndPayloads",
                |c| c.test_docs_and_freqs_and_positions_and_offsets_and_payloads(),
            ))
            .method(t("testRandom", |c| c.test_random()))
            .method(t("testPostingsEnumReuse", |c| c.test_postings_enum_reuse()))
            .method(t("testJustEmptyField", |c| c.test_just_empty_field()))
            .method(t("testEmptyFieldAndEmptyTerm", |c| {
                c.test_empty_field_and_empty_term()
            }))
            .method(t("testDidntWantFreqsButAskedAnyway", |c| {
                c.test_didnt_want_freqs_but_asked_anyway()
            }))
            .method(t("testAskForPositionsWhenNotThere", |c| {
                c.test_ask_for_positions_when_not_there()
            }))
            .method(t("testGhosts", |c| c.test_ghosts()))
            .method(t("testDisorder", |c| c.test_disorder()))
            .method(t("testBinarySearchTermLeaf", |c| {
                c.test_binary_search_term_leaf()
            }))
            .method(t("testLevel2Ghosts", |c| c.test_level2_ghosts()))
            .method(t("testInvertedWrite", |c| c.test_inverted_write()))
            .method(t("testPostingsEnumDocsOnly", |c| {
                c.test_postings_enum_docs_only()
            }))
            .method(t("testPostingsEnumFreqs", |c| c.test_postings_enum_freqs()))
            .method(t("testPostingsEnumPositions", |c| {
                c.test_postings_enum_positions()
            }))
            .method(t("testPostingsEnumOffsets", |c| {
                c.test_postings_enum_offsets()
            }))
            .method(t("testPostingsEnumPayloads", |c| {
                c.test_postings_enum_payloads()
            }))
            .method(t("testPostingsEnumAll", |c| c.test_postings_enum_all()))
            .method(
                t("testLineFileDocs", |c| c.test_line_file_docs())
                    // this test takes a few seconds
                    .annotate(Nightly),
            )
            .method(t("testMismatchedFields", |c| c.test_mismatched_fields()))
            .method(t("testDocIDRunEnd", |c| c.test_doc_id_run_end()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BasePostingsFormatTestCase`.
fn postings_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BasePostingsFormatTestCase {
    c.as_postings().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BasePostingsFormatTestCase",
        )
        .throw()
    })
}

// -----------------------------------------------------------------------------
// Assertion helpers
// -----------------------------------------------------------------------------

/// The address of a postings enum, its identity for `assertSame`.
fn identity(p: &dyn PostingsEnum) -> *const () {
    p as *const dyn PostingsEnum as *const ()
}

/// The runtime class name a reuse message names (`getClass().getName()`).
fn class_name_of(p: &dyn PostingsEnum) -> String {
    p.to_string()
}

/// A payload as JUnit prints it: `BytesRef.toString()`, or `null`.
#[derive(PartialEq)]
struct JavaPayload(Option<Vec<u8>>);

impl fmt::Display for JavaPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            None => f.write_str("null"),
            Some(bytes) => write!(f, "{}", BytesRef::new(bytes.clone())),
        }
    }
}

/// The payload of the current position.
fn payload_of(p: &dyn PostingsEnum) -> Result<JavaPayload> {
    Ok(JavaPayload(p.get_payload()?.map(<[u8]>::to_vec)))
}

/// `assertNull(object)`: `expected null, but was:<object>`.
fn assert_null(actual: &JavaPayload) {
    if actual.0.is_some() {
        Throwable::new(
            "java.lang.AssertionError",
            format!("expected null, but was:<{actual}>"),
        )
        .throw()
    }
}

/// `assertSame(message, expected, actual)` /
/// `assertNotSame(message, unexpected, actual)` on two enum identities.
fn assert_same(message: &str, same: bool, expected: *const (), actual: *const ()) {
    if same && expected != actual {
        Throwable::new(
            "java.lang.AssertionError",
            format!("{message} expected same:<{expected:?}> was not:<{actual:?}>"),
        )
        .throw()
    }
    if !same && expected == actual {
        Throwable::new(
            "java.lang.AssertionError",
            format!("{message} expected not same"),
        )
        .throw()
    }
}

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
/// || new BytesRef(s).equals(payload))` or `assertEquals(new BytesRef(s),
/// payload)`.
#[derive(Clone, Copy)]
enum PayloadIs {
    Null,
    NullOr(&'static str),
    Eq(&'static str),
}

impl PayloadIs {
    fn check(self, actual: JavaPayload) {
        match self {
            PayloadIs::Null => assert_null(&actual),
            PayloadIs::NullOr(s) => {
                Assert::assert_true(actual.0.is_none() || actual.0.as_deref() == Some(s.as_bytes()))
            }
            PayloadIs::Eq(s) => {
                Assert::assert_equals(JavaPayload(Some(s.as_bytes().to_vec())), actual)
            }
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
        payload.check(payload_of(p)?);
    }
    Assert::assert_equals(NO_MORE_DOCS, p.next_doc()?);
    Ok(())
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::new(field, BytesRef::from_char_sequence(text))
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
        "org.apache.lucene.tests.index.BasePostingsFormatTestCase$2"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = MockTokenizer::whitespace_lower_case()?;
        Ok(tokenizer_components(std::sync::Arc::new(
            rucene::util::atomic_ref_cell::AtomicRefCell::new(tokenizer),
        )))
    }
}

/// The two `bar` tokens of `testPostingsEnumPayloads`/`All`, with payloads
/// `pay1` and `pay2`.
fn payload_tokens() -> Vec<Token> {
    use rucene::analysis::tokenattributes::PayloadAttribute;
    let mut token1 = Token::with_offsets("bar", 0, 3);
    token1.set_payload(Some(BytesRef::from_char_sequence("pay1")));
    let mut token2 = Token::with_offsets("bar", 4, 7);
    token2.set_payload(Some(BytesRef::from_char_sequence("pay2")));
    vec![token1, token2]
}

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `BasePostingsFormatTestCase`.
pub trait BasePostingsFormatTestCase: BaseIndexFileFormatTestCase {
    /// `testDocsOnly()`.
    fn test_docs_only(&self) -> Result<()> {
        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        tester.test_full(
            self.get_codec(),
            &LuceneTestCase::create_temp_dir_prefix("testPostingsFormat.testExact"),
            IndexOptions::DOCS,
            false,
        )
    }

    /// `testDocsAndFreqs()`.
    fn test_docs_and_freqs(&self) -> Result<()> {
        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        tester.test_full(
            self.get_codec(),
            &LuceneTestCase::create_temp_dir_prefix("testPostingsFormat.testExact"),
            IndexOptions::DOCS_AND_FREQS,
            false,
        )
    }

    /// `testDocsAndFreqsAndPositions()`.
    fn test_docs_and_freqs_and_positions(&self) -> Result<()> {
        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        tester.test_full(
            self.get_codec(),
            &LuceneTestCase::create_temp_dir_prefix("testPostingsFormat.testExact"),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            false,
        )
    }

    /// `testDocsAndFreqsAndPositionsAndPayloads()`.
    fn test_docs_and_freqs_and_positions_and_payloads(&self) -> Result<()> {
        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        tester.test_full(
            self.get_codec(),
            &LuceneTestCase::create_temp_dir_prefix("testPostingsFormat.testExact"),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
        )
    }

    /// `testDocsAndFreqsAndPositionsAndOffsets()`.
    fn test_docs_and_freqs_and_positions_and_offsets(&self) -> Result<()> {
        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        tester.test_full(
            self.get_codec(),
            &LuceneTestCase::create_temp_dir_prefix("testPostingsFormat.testExact"),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            false,
        )
    }

    /// `testDocsAndFreqsAndPositionsAndOffsetsAndPayloads()`.
    fn test_docs_and_freqs_and_positions_and_offsets_and_payloads(&self) -> Result<()> {
        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        tester.test_full(
            self.get_codec(),
            &LuceneTestCase::create_temp_dir_prefix("testPostingsFormat.testExact"),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            true,
        )
    }

    /// `testRandom()`.
    fn test_random(&self) -> Result<()> {
        let iters = 5;

        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        for _ in 0..iters {
            let path = LuceneTestCase::create_temp_dir_prefix("testPostingsFormat");
            let mut dir = LuceneTestCase::new_fs_directory(&path);

            let index_payloads = LuceneTestCase::random().next_boolean();
            // TODO test thread safety of buildIndex too
            let mut fields_producer = tester.build_index(
                self.get_codec(),
                dir.as_directory(),
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
                index_payloads,
                false,
            )?;

            tester.test_fields(&*fields_producer);

            // NOTE: you can also test "weaker" index options than
            // you indexed with:
            tester.test_terms(
                &*fields_producer,
                TestOptions::all_of(),
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
                false,
            )?;

            fields_producer.close()?;
            drop(fields_producer);

            dir.close()?;
        }
        Ok(())
    }

    /// `isPostingsEnumReuseImplemented()`.
    fn is_postings_enum_reuse_implemented(&self) -> bool {
        true
    }

    /// `testPostingsEnumReuse()`.
    fn test_postings_enum_reuse(&self) -> Result<()> {
        let path = LuceneTestCase::create_temp_dir_prefix("testPostingsEnumReuse");
        let mut dir = LuceneTestCase::new_fs_directory(&path);

        let tester = postings_tester();
        let mut tester = tester.lock().unwrap_or_else(PoisonError::into_inner);
        let mut fields_producer = tester.build_index(
            self.get_codec(),
            dir.as_directory(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            LuceneTestCase::random().next_boolean(),
            true,
        )?;
        tester.shuffle_all_terms(&*LuceneTestCase::random());
        let field_and_term = tester.all_terms()[0].clone();

        let terms = npe(fields_producer.terms(field_and_term.field())?);
        let mut te = terms.iterator()?;

        te.seek_exact(field_and_term.term())?;
        check_reuse(&mut *te, FREQS, ALL, false)?;
        if self.is_postings_enum_reuse_implemented() {
            check_reuse(&mut *te, ALL, ALL, true)?;
        }

        drop(te);
        drop(terms);
        fields_producer.close()?;
        dir.close()?;
        Ok(())
    }

    /// `testJustEmptyField()`.
    fn test_just_empty_field(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
        iwc.set_codec(self.get_codec())?;
        let iw = RandomIndexWriter::new_with_config(
            &*LuceneTestCase::random(),
            dir.as_directory(),
            iwc,
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "",
            "something",
            Store::NO,
        )));
        iw.add_document(&doc)?;
        let ir = iw.get_reader()?;
        let ar = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as _)?;
        Assert::assert_equals(1, ar.get_field_infos().size());
        let terms = ar.terms("")?;
        Assert::assert_true(terms.is_some());
        let mut terms_enum = npe(terms).iterator()?;
        Assert::assert_true(terms_enum.next()?.is_some());
        Assert::assert_equals(
            terms_enum.term()?,
            BytesRef::from_char_sequence("something"),
        );
        Assert::assert_true(terms_enum.next()?.is_none());
        drop((terms_enum, ar));
        ir.close()?;
        drop(ir);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testEmptyFieldAndEmptyTerm()`.
    fn test_empty_field_and_empty_term(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
        iwc.set_codec(self.get_codec())?;
        let iw = RandomIndexWriter::new_with_config(
            &*LuceneTestCase::random(),
            dir.as_directory(),
            iwc,
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "",
            "",
            Store::NO,
        )));
        iw.add_document(&doc)?;
        let ir = iw.get_reader()?;
        let ar = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as _)?;
        Assert::assert_equals(1, ar.get_field_infos().size());
        let terms = ar.terms("")?;
        Assert::assert_true(terms.is_some());
        let mut terms_enum = npe(terms).iterator()?;
        Assert::assert_true(terms_enum.next()?.is_some());
        Assert::assert_equals(terms_enum.term()?, BytesRef::from_char_sequence(""));
        Assert::assert_true(terms_enum.next()?.is_none());
        drop((terms_enum, ar));
        ir.close()?;
        drop(ir);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testDidntWantFreqsButAskedAnyway()`.
    fn test_didnt_want_freqs_but_asked_anyway(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*LuceneTestCase::random()),
        )));
        iwc.set_codec(self.get_codec())?;
        let iw = RandomIndexWriter::new_with_config(
            &*LuceneTestCase::random(),
            dir.as_directory(),
            iwc,
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            "value",
            Store::NO,
        )));
        iw.add_document(&doc)?;
        iw.add_document(&doc)?;
        let ir = iw.get_reader()?;
        let ar = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as _)?;
        let mut terms_enum = npe(ar.terms("field")?).iterator()?;
        Assert::assert_true(terms_enum.seek_exact(&BytesRef::from_char_sequence("value"))?);
        let mut docs_enum = terms_enum.postings(None, NONE)?;
        Assert::assert_equals(0, docs_enum.next_doc()?);
        Assert::assert_equals(1, docs_enum.freq()?);
        Assert::assert_equals(1, docs_enum.next_doc()?);
        Assert::assert_equals(1, docs_enum.freq()?);
        drop((docs_enum, terms_enum, ar));
        ir.close()?;
        drop(ir);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testAskForPositionsWhenNotThere()`.
    fn test_ask_for_positions_when_not_there(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*LuceneTestCase::random()),
        )));
        iwc.set_codec(self.get_codec())?;
        let iw = RandomIndexWriter::new_with_config(
            &*LuceneTestCase::random(),
            dir.as_directory(),
            iwc,
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            "value",
            Store::NO,
        )));
        iw.add_document(&doc)?;
        iw.add_document(&doc)?;
        let ir = iw.get_reader()?;
        let ar = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as _)?;
        let mut terms_enum = npe(ar.terms("field")?).iterator()?;
        Assert::assert_true(terms_enum.seek_exact(&BytesRef::from_char_sequence("value"))?);
        let mut docs_enum = terms_enum.postings(None, POSITIONS)?;
        Assert::assert_equals(0, docs_enum.next_doc()?);
        Assert::assert_equals(1, docs_enum.freq()?);
        Assert::assert_equals(1, docs_enum.next_doc()?);
        Assert::assert_equals(1, docs_enum.freq()?);
        drop((docs_enum, terms_enum, ar));
        ir.close()?;
        drop(ir);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testGhosts()`: tests that ghost fields still work.
    // TODO: can this be improved?
    fn test_ghosts(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
        iwc.set_codec(self.get_codec())?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let iw = RandomIndexWriter::new_with_config(
            &*LuceneTestCase::random(),
            dir.as_directory(),
            iwc,
        )?;
        let mut doc = Document::new();
        iw.add_document(&doc)?;
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "ghostField",
            "something",
            Store::NO,
        )));
        iw.add_document(&doc)?;
        iw.force_merge(1)?;
        iw.delete_documents(term("ghostField", "something"))?; // delete the only term for the field
        iw.force_merge(1)?;
        let ir = iw.get_reader()?;
        let ar = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as _)?;
        // Ghost busting terms dict impls will have
        // fields.size() == 0; all others must be == 1:
        Assert::assert_true(ar.get_field_infos().size() <= 1);
        if let Some(terms) = ar.terms("ghostField")? {
            let mut terms_enum = terms.iterator()?;
            if terms_enum.next()?.is_some() {
                let mut postings_enum = terms_enum.postings(None, FREQS)?;
                Assert::assert_true(postings_enum.next_doc()? == NO_MORE_DOCS);
            }
        }
        drop(ar);
        ir.close()?;
        drop(ir);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testDisorder()`: test seek in disorder.
    fn test_disorder(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();

        let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
        iwc.set_codec(self.get_codec())?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_tiered_merge_policy())))?;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;

        for i in 0..10000 {
            let mut document = Document::new();
            document.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
            iw.add_document(&document)?;
        }
        iw.commit()?;
        iw.force_merge(1)?;

        let reader = directory_reader::open_with_writer(&*iw)?;
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
        let mut terms_enum = npe(leaf.terms("id")?).iterator()?;

        for _ in 0..20000 {
            let n = LuceneTestCase::random().next_int_range(0, 10000);
            let target = BytesRef::from_char_sequence(&n.to_string());
            // seekExact.
            Assert::assert_true(terms_enum.seek_exact(&target)?);
            Assert::assert_equals(terms_enum.term()?, target.clone());
            // seekCeil.
            Assert::assert_equals(
                JavaSeekStatus(SeekStatus::FOUND),
                JavaSeekStatus(terms_enum.seek_ceil(&target)?),
            );
            Assert::assert_equals(terms_enum.term()?, target);
        }

        drop((terms_enum, leaf));
        reader.close()?;
        drop(reader);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `subCheckBinarySearch(termsEnum)`.
    fn sub_check_binary_search(&self, _terms_enum: &mut dyn TermsEnum) -> Result<()> {
        Ok(())
    }

    /// `testBinarySearchTermLeaf()`.
    fn test_binary_search_term_leaf(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();

        let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
        iwc.set_codec(self.get_codec())?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_tiered_merge_policy())))?;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;

        for i in 100000..=100400 {
            // only add odd number
            if i % 2 == 1 {
                let mut document = Document::new();
                document.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                iw.add_document(&document)?;
            }
        }
        iw.commit()?;
        iw.force_merge(1)?;

        let reader = directory_reader::open_with_writer(&*iw)?;
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
        let mut terms_enum = npe(leaf.terms("id")?).iterator()?;
        // test seekExact
        for i in 100000..=100400 {
            let target = BytesRef::from_char_sequence(&i.to_string());
            if i % 2 == 1 {
                Assert::assert_true(terms_enum.seek_exact(&target)?);
                Assert::assert_equals(terms_enum.term()?, target);
            } else {
                Assert::assert_false(terms_enum.seek_exact(&target)?);
            }
        }

        self.sub_check_binary_search(&mut *terms_enum)?;
        // test seekCeil
        for i in 100000..100400 {
            let target = BytesRef::from_char_sequence(&i.to_string());
            if i % 2 == 1 {
                Assert::assert_equals(
                    JavaSeekStatus(SeekStatus::FOUND),
                    JavaSeekStatus(terms_enum.seek_ceil(&target)?),
                );
                Assert::assert_equals(terms_enum.term()?, target);
                if i <= 100397 {
                    Assert::assert_equals(
                        JavaBytesRef(Some(BytesRef::from_char_sequence(&(i + 2).to_string()))),
                        JavaBytesRef(terms_enum.next()?),
                    );
                }
            } else {
                Assert::assert_equals(
                    JavaSeekStatus(SeekStatus::NOT_FOUND),
                    JavaSeekStatus(terms_enum.seek_ceil(&target)?),
                );
                Assert::assert_equals(
                    BytesRef::from_char_sequence(&(i + 1).to_string()),
                    terms_enum.term()?,
                );
            }
        }
        Assert::assert_equals(
            JavaSeekStatus(SeekStatus::END),
            JavaSeekStatus(terms_enum.seek_ceil(&BytesRef::from_char_sequence("100400"))?),
        );
        drop((terms_enum, leaf));
        reader.close()?;
        drop(reader);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testLevel2Ghosts()`: tests that level 2 ghost fields still work.
    fn test_level2_ghosts(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();

        let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
        iwc.set_codec(self.get_codec())?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;

        let mut document = Document::new();
        document.add(Box::new(StringField::new(
            "id",
            "0".to_string(),
            Store::NO,
        )?));
        document.add(Box::new(StringField::new(
            "suggest_field",
            "apples".to_string(),
            Store::NO,
        )?));
        iw.add_document(&document)?;
        // need another document so whole segment isn't deleted
        iw.add_document(&Document::new())?;
        iw.commit()?;

        let mut document = Document::new();
        document.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::NO,
        )?));
        document.add(Box::new(StringField::new(
            "suggest_field2",
            "apples".to_string(),
            Store::NO,
        )?));
        iw.add_document(&document)?;
        iw.commit()?;

        iw.delete_documents_by_terms(vec![term("id", "0")])?;
        // first force merge creates a level 1 ghost field
        iw.force_merge(1)?;

        // second force merge creates a level 2 ghost field, causing MultiFields to include
        // "suggest_field" in its iteration, yet a null Terms is returned (no documents have
        // this field anymore)
        iw.add_document(&Document::new())?;
        iw.force_merge(1)?;

        let reader = directory_reader::open_with_writer(&*iw)?;
        let index_searcher = IndexSearcher::new(Arc::clone(&reader) as _)?;

        Assert::assert_equals(
            1,
            index_searcher.count(Arc::new(TermQuery::new(term("id", "1"))))?,
        );

        drop(index_searcher);
        reader.close()?;
        drop(reader);
        iw.close()?;
        drop(iw);
        dir.close()?;
        Ok(())
    }

    /// `testInvertedWrite()` (LUCENE-5123): make sure we can visit postings
    /// twice during flush/merge.
    fn test_inverted_write(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let analyzer = MockAnalyzer::new(&*LuceneTestCase::random());
        analyzer.set_max_token_length(TestUtil::next_int(
            &*LuceneTestCase::random(),
            1,
            rucene::index::index_writer::MAX_TERM_LENGTH as i32,
        ));
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));

        // Must be concurrent because thread(s) can be merging
        // while up to one thread flushes, and each of those
        // threads iterates over the map while the flushing
        // thread might be adding to it:
        let state = Arc::new(InvertedWriteState::default());

        // TODO: would be better to use / delegate to the current
        // Codec returned by getCodec()

        let codec = self.get_codec();
        let filter = FilterCodec::new(codec.name().to_string(), Arc::clone(&codec))
            .with_postings_format(InvertedWritePostingsFormat {
                codec: Arc::clone(&codec),
                main_thread: std::thread::current().id(),
                state: Arc::clone(&state),
            });
        iwc.set_codec(Arc::new(filter))?;

        let w = RandomIndexWriter::new_with_config(
            &*LuceneTestCase::random(),
            dir.as_directory(),
            iwc,
        )?;

        let docs = LineFileDocs::with_default_file(&*LuceneTestCase::random())
            .map_err(LuceneError::from)?;
        let bytes_to_index = i64::from(LuceneTestCase::at_least(100)) * 1024;
        let mut bytes_indexed: i64 = 0;
        while bytes_indexed < bytes_to_index {
            let doc = docs.next_doc()?;
            let body = npe(doc
                .borrow()
                .get_fields()
                .iter()
                .find(|field| field.name() == "body")
                .cloned());
            let body_value = body.string_value().unwrap_or_default();
            let mut just_body_doc = Document::new();
            just_body_doc.add_shared(body);
            w.add_document(&just_body_doc)?;
            bytes_indexed += RamUsageTester::ram_used(&JustBodyDoc::new(&body_value));
        }

        let r = w.get_reader()?;
        w.close()?;

        let r: Arc<dyn IndexReader> = r;
        let terms = npe(MultiTerms::get_terms(&r, "body")?);
        Assert::assert_equals(
            state.sum_doc_freq.load(Ordering::SeqCst),
            terms.sum_doc_freq()?,
        );
        Assert::assert_equals(
            state.sum_total_term_freq.load(Ordering::SeqCst),
            terms.sum_total_term_freq()?,
        );

        let term_freqs = state
            .term_freqs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut terms_enum = terms.iterator()?;
        let mut term_count: i64 = 0;
        let mut supports_ords = true;
        while terms_enum.next()?.is_some() {
            let term = terms_enum.term()?;
            let tf = npe(term_freqs.get(&term.utf8_to_string()?));
            Assert::assert_equals(tf.doc_freq, terms_enum.doc_freq()?);
            Assert::assert_equals(tf.total_term_freq, terms_enum.total_term_freq()?);
            if supports_ords {
                let ord = match terms_enum.ord() {
                    Ok(ord) => ord,
                    Err(LuceneError {
                        kind: LuceneErrorKind::UnsupportedOperation { .. },
                        ..
                    }) => {
                        supports_ords = false;
                        -1
                    }
                    Err(e) => return Err(e),
                };
                if ord != -1 {
                    Assert::assert_equals(term_count, ord);
                }
            }
            term_count += 1;
        }
        Assert::assert_equals(term_freqs.len() as i64, term_count);
        drop(term_freqs);

        drop((terms_enum, terms));
        r.close()?;
        drop(r);
        drop(w);
        docs.close();
        dir.close()?;
        Ok(())
    }

    /// `assertReused(field, p1, p2)`.
    fn assert_reused(&self, field: &str, p1: *const (), p2: *const ()) {
        // if its not DirectPF, we should always reuse. This one has trouble.
        if TestUtil::get_postings_format(field) != "Direct" {
            assert_same("", true, p1, p2);
        }
    }

    /// `testPostingsEnumDocsOnly()`.
    fn test_postings_enum_docs_only(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = IndexWriterConfig::with_analyzer(None);
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "foo",
            "bar".to_string(),
            Store::NO,
        )?));
        iw.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;

        // sugar method (FREQS)
        let mut postings = npe(leaf.postings_default(&term("foo", "bar"))?);
        check_one_doc(&mut *postings, FreqIs::Eq(1), &[])?;

        // termsenum reuse (FREQS)
        let mut terms_enum = npe(leaf.terms("foo")?).iterator()?;
        terms_enum.seek_exact(&BytesRef::from_char_sequence("bar"))?;
        let p1 = identity(&*postings);
        let mut postings2 = terms_enum.postings(Some(postings), FREQS)?;
        self.assert_reused("foo", p1, identity(&*postings2));
        // and it had better work
        check_one_doc(&mut *postings2, FreqIs::Eq(1), &[])?;

        // asking for any flags: ok
        for flag in [NONE, FREQS, POSITIONS, PAYLOADS, OFFSETS, ALL] {
            let mut postings = terms_enum.postings(None, flag)?;
            check_one_doc(&mut *postings, FreqIs::Eq(1), &[])?;
            // reuse that too
            let p1 = identity(&*postings);
            let mut postings2 = terms_enum.postings(Some(postings), flag)?;
            self.assert_reused("foo", p1, identity(&*postings2));
            // and it had better work
            check_one_doc(&mut *postings2, FreqIs::Eq(1), &[])?;
        }

        iw.close()?;
        drop(iw);
        drop((terms_enum, postings2, leaf));
        reader.close()?;
        drop(reader);
        dir.close()?;
        Ok(())
    }

    /// `testPostingsEnumFreqs()`.
    fn test_postings_enum_freqs(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockTokenizerAnalyzer::new())));
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        let ft = FieldType::new_from(TextField::type_not_stored());
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
        doc.add(Box::new(Field::new("foo", "bar bar".to_string(), ft)?));
        iw.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;

        // sugar method (FREQS)
        let mut postings = npe(leaf.postings_default(&term("foo", "bar"))?);
        check_one_doc(&mut *postings, FreqIs::Eq(2), &[])?;

        // termsenum reuse (FREQS)
        let mut terms_enum = npe(leaf.terms("foo")?).iterator()?;
        terms_enum.seek_exact(&BytesRef::from_char_sequence("bar"))?;
        let p1 = identity(&*postings);
        let mut postings2 = terms_enum.postings(Some(postings), FREQS)?;
        self.assert_reused("foo", p1, identity(&*postings2));
        // and it had better work
        check_one_doc(&mut *postings2, FreqIs::Eq(2), &[])?;

        // asking for docs only: ok
        let mut docs_only = terms_enum.postings(None, NONE)?;
        // we don't define what it is, but if its something else, we should look into it?
        check_one_doc(&mut *docs_only, FreqIs::OneOrTwo, &[])?;
        // reuse that too
        let d1 = identity(&*docs_only);
        let mut docs_only2 = terms_enum.postings(Some(docs_only), NONE)?;
        self.assert_reused("foo", d1, identity(&*docs_only2));
        // and it had better work
        // we don't define what it is, but if its something else, we should look into it?
        check_one_doc(&mut *docs_only2, FreqIs::OneOrTwo, &[])?;

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
            let p1 = identity(&*postings);
            let mut postings2 = terms_enum.postings(Some(postings), flag)?;
            self.assert_reused("foo", p1, identity(&*postings2));
            // and it had better work
            check_one_doc(&mut *postings2, freq, &[])?;
        }

        iw.close()?;
        drop(iw);
        drop((terms_enum, postings2, docs_only2, leaf));
        reader.close()?;
        drop(reader);
        dir.close()?;
        Ok(())
    }

    /// `testPostingsEnumPositions()`.
    fn test_postings_enum_positions(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockTokenizerAnalyzer::new())));
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new(
            "foo",
            "bar bar".to_string(),
            Store::NO,
        )?));
        iw.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*iw)?;

        use OffsetIs::Eq as E;
        let no_offsets = [
            PositionIs(0, E(-1), E(-1), PayloadIs::Null),
            PositionIs(1, E(-1), E(-1), PayloadIs::Null),
        ];
        check_postings_enum_blocks(
            self,
            &reader,
            &no_offsets,
            &no_offsets,
            &no_offsets,
            &no_offsets,
        )?;

        iw.close()?;
        drop(iw);
        reader.close()?;
        drop(reader);
        dir.close()?;
        Ok(())
    }

    /// `testPostingsEnumOffsets()`.
    fn test_postings_enum_offsets(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockTokenizerAnalyzer::new())));
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        let ft = FieldType::new_from(TextField::type_not_stored());
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
        doc.add(Box::new(Field::new("foo", "bar bar".to_string(), ft)?));
        iw.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*iw)?;

        use OffsetIs::{Either as O, Eq as E};
        let maybe_offsets = [
            PositionIs(0, O(-1, 0), O(-1, 3), PayloadIs::Null),
            PositionIs(1, O(-1, 4), O(-1, 7), PayloadIs::Null),
        ];
        let offsets = [
            PositionIs(0, E(0), E(3), PayloadIs::Null),
            PositionIs(1, E(4), E(7), PayloadIs::Null),
        ];
        check_postings_enum_blocks(
            self,
            &reader,
            &maybe_offsets,
            &maybe_offsets,
            &offsets,
            &offsets,
        )?;

        iw.close()?;
        drop(iw);
        reader.close()?;
        drop(reader);
        dir.close()?;
        Ok(())
    }

    /// `testPostingsEnumPayloads()`.
    fn test_postings_enum_payloads(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = IndexWriterConfig::with_analyzer(None);
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        let stream: Arc<AtomicRefCell<dyn rucene::analysis::TokenStream>> =
            Arc::new(AtomicRefCell::new(CannedTokenStream::new(payload_tokens())));
        doc.add(Box::new(TextField::new_with_token_stream("foo", stream)?));
        iw.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*iw)?;

        use OffsetIs::Eq as E;
        let maybe_payloads = [
            PositionIs(0, E(-1), E(-1), PayloadIs::NullOr("pay1")),
            PositionIs(1, E(-1), E(-1), PayloadIs::NullOr("pay2")),
        ];
        let payloads = [
            PositionIs(0, E(-1), E(-1), PayloadIs::Eq("pay1")),
            PositionIs(1, E(-1), E(-1), PayloadIs::Eq("pay2")),
        ];
        check_postings_enum_blocks(
            self,
            &reader,
            &maybe_payloads,
            &payloads,
            &maybe_payloads,
            &payloads,
        )?;

        iw.close()?;
        drop(iw);
        reader.close()?;
        drop(reader);
        dir.close()?;
        Ok(())
    }

    /// `testPostingsEnumAll()`.
    fn test_postings_enum_all(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iwc = IndexWriterConfig::with_analyzer(None);
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        let ft = FieldType::new_from(TextField::type_not_stored());
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
        let stream: Arc<AtomicRefCell<dyn rucene::analysis::TokenStream>> =
            Arc::new(AtomicRefCell::new(CannedTokenStream::new(payload_tokens())));
        doc.add(Box::new(Field::new_with_token_stream("foo", stream, ft)?));
        iw.add_document(&doc)?;
        let reader = directory_reader::open_with_writer(&*iw)?;

        use OffsetIs::{Either as O, Eq as E};
        let positions = [
            PositionIs(0, O(-1, 0), O(-1, 3), PayloadIs::NullOr("pay1")),
            PositionIs(1, O(-1, 4), O(-1, 7), PayloadIs::NullOr("pay2")),
        ];
        let payloads = [
            PositionIs(0, O(-1, 0), O(-1, 3), PayloadIs::Eq("pay1")),
            PositionIs(1, O(-1, 4), O(-1, 7), PayloadIs::Eq("pay2")),
        ];
        let offsets = [
            PositionIs(0, E(0), E(3), PayloadIs::NullOr("pay1")),
            PositionIs(1, E(4), E(7), PayloadIs::NullOr("pay2")),
        ];
        let all = [
            PositionIs(0, E(0), E(3), PayloadIs::Eq("pay1")),
            PositionIs(1, E(4), E(7), PayloadIs::Eq("pay2")),
        ];
        check_postings_enum_blocks(self, &reader, &positions, &payloads, &offsets, &all)?;

        iw.close()?;
        drop(iw);
        reader.close()?;
        drop(reader);
        dir.close()?;
        Ok(())
    }

    /// `testLineFileDocs()` (`@Nightly`, this test takes a few seconds):
    /// test realistic data, which is often better at uncovering real bugs.
    fn test_line_file_docs(&self) -> Result<()> {
        // Use a FS dir and a non-randomized IWC to not slow down indexing
        let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
        {
            let docs = LineFileDocs::with_default_file(&*LuceneTestCase::random())
                .map_err(LuceneError::from)?;
            let w = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
            let num_docs = LuceneTestCase::at_least(10_000);
            for _ in 0..num_docs {
                // Only keep the body field, and don't index term vectors on it, we only care
                // about postings
                let doc = docs.next_doc()?;
                let doc = doc.borrow();
                let body = doc.get_field("body");
                Assert::assert_true(body.is_some());
                let body = npe(body);
                Assert::assert_true(body.string_value().is_some());
                Assert::assert_true(body.field_type().index_options() != IndexOptions::NONE);
                let body = TextField::new("body", npe(body.string_value()), Store::NO)?;
                let mut single = Document::new();
                single.add(Box::new(body));
                w.add_document(&single)?;
            }
            w.force_merge(1)?;
            docs.close();
            w.close()?;
        }
        TestUtil::check_index(dir.as_directory())?;
        dir.close()?;
        Ok(())
    }

    /// `testMismatchedFields()`.
    fn test_mismatched_fields(&self) -> Result<()> {
        let mut dir1 = LuceneTestCase::new_directory();
        let w1 = IndexWriter::new(
            dir1.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("f", "a".to_string(), Store::NO)?));
        doc.add(Box::new(StringField::new("g", "b".to_string(), Store::NO)?));
        w1.add_document(&doc)?;

        let mut dir2 = LuceneTestCase::new_directory();
        let config2 = LuceneTestCase::new_index_writer_config();
        config2.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
        let w2 = IndexWriter::new(dir2.as_directory(), config2)?;
        w2.add_document(&doc)?;
        w2.commit()?;

        let reader = directory_reader::open_with_writer(&*w1)?;
        w1.close()?;
        drop(w1);
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
        let codec_reader = Arc::clone(&leaf).as_codec_reader().unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "the leaf reader is not a CodecReader",
            )
            .throw()
        });
        let mismatched = MismatchedCodecReader::new(codec_reader, &*LuceneTestCase::random())?;
        w2.add_indexes_codec_readers(&[Arc::new(mismatched)])?;
        drop(leaf);
        reader.close()?;
        drop(reader);
        w2.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*w2)?;
        w2.close()?;
        drop(w2);

        let leaf_reader = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;

        let mut te = npe(leaf_reader.terms("f")?).iterator()?;
        Assert::assert_equals_str(None, Some("a"), Some(&npe(te.next()?).utf8_to_string()?));
        Assert::assert_equals(2, te.doc_freq()?);
        Assert::assert_true(te.next()?.is_none());

        te = npe(leaf_reader.terms("g")?).iterator()?;
        Assert::assert_equals_str(None, Some("b"), Some(&npe(te.next()?).utf8_to_string()?));
        Assert::assert_equals(2, te.doc_freq()?);
        Assert::assert_true(te.next()?.is_none());

        drop((te, leaf_reader));
        // `IOUtils.close(reader, w2, dir1, dir2)`
        reader.close()?;
        drop(reader);
        dir1.close()?;
        dir2.close()?;
        Ok(())
    }

    /// `testDocIDRunEnd()`.
    fn test_doc_id_run_end(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        for _ in 0..100 {
            let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
            iwc.set_open_mode(OpenMode::CREATE)?;
            iwc.set_codec(self.get_codec())?;
            // Prevent randomization from slowing down indexing too much
            if iwc.max_buffered_docs() < 1_000 {
                iwc.set_max_buffered_docs(1_000)?;
            }
            let iw = IndexWriter::new(dir.as_directory(), iwc)?;
            let empty_doc = Document::new();
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "",
                "something".to_string(),
                Store::NO,
            )?));
            let num_empty_docs = TestUtil::next_int(&*LuceneTestCase::random(), 0, 5_000);
            let num_docs = TestUtil::next_int(&*LuceneTestCase::random(), 4096, 20_000);
            for _ in 0..num_empty_docs {
                iw.add_document(&empty_doc)?;
            }
            for _ in 0..num_docs {
                iw.add_document(&doc)?;
            }
            iw.force_merge(1)?;
            let ir = directory_reader::open_with_writer(&*iw)?;
            let ar = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as _)?;
            TestUtil::check_leaf_reader(
                ar,
                rucene::index::check_index::Level::MIN_LEVEL_FOR_SLOW_CHECKS,
            )?;
            ir.close()?;
            drop(ir);
            iw.close()?;
        }
        dir.close()?;
        Ok(())
    }

    /// `addRandomFields(Document)`, the override every postings suite
    /// inherits; a subclass's [`BaseIndexFileFormatTestCase::add_random_fields`]
    /// delegates here.
    fn postings_add_random_fields(&self, doc: &mut Document) {
        for opts in [
            IndexOptions::NONE,
            IndexOptions::DOCS,
            IndexOptions::DOCS_AND_FREQS,
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            IndexOptions::DOCS_AND_CUSTOM_FREQS,
        ] {
            if opts == IndexOptions::NONE || opts == IndexOptions::DOCS_AND_CUSTOM_FREQS {
                continue;
            }
            let ft = FieldType::new();
            ft.set_index_options(opts).unwrap_or_else(|e| rethrow(e));
            ft.freeze();
            let num_fields = LuceneTestCase::random().next_int_bound(5);
            for _ in 0..num_fields {
                let value = TestUtil::random_simple_string_max(&*LuceneTestCase::random(), 2);
                doc.add(Box::new(
                    Field::new(&format!("f_{opts:?}"), value, ft.clone())
                        .unwrap_or_else(|e| rethrow(e)),
                ));
            }
        }
    }
}

/// The body `testPostingsEnumPositions`, `Offsets`, `Payloads` and `All`
/// share after indexing their one document: the `FREQS` sugar enum, its
/// reuse, the docs-only enum and its reuse, then for `POSITIONS`,
/// `PAYLOADS`, `OFFSETS` and `ALL` an enum from the reader and its reuse
/// from the terms enum, each checked with the positions the test expects
/// for that flag.
fn check_postings_enum_blocks(
    case: &(impl BasePostingsFormatTestCase + ?Sized),
    reader: &Arc<dyn DirectoryReader>,
    positions: &[PositionIs],
    payloads: &[PositionIs],
    offsets: &[PositionIs],
    all: &[PositionIs],
) -> Result<()> {
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(reader) as _)?;

    // sugar method (FREQS)
    let mut postings = npe(leaf.postings_default(&term("foo", "bar"))?);
    check_one_doc(&mut *postings, FreqIs::Eq(2), &[])?;

    // termsenum reuse (FREQS)
    let mut terms_enum = npe(leaf.terms("foo")?).iterator()?;
    terms_enum.seek_exact(&BytesRef::from_char_sequence("bar"))?;
    let p1 = identity(&*postings);
    let mut postings2 = terms_enum.postings(Some(postings), FREQS)?;
    case.assert_reused("foo", p1, identity(&*postings2));
    // and it had better work
    check_one_doc(&mut *postings2, FreqIs::Eq(2), &[])?;

    // asking for docs only: ok
    let mut docs_only = terms_enum.postings(None, NONE)?;
    // we don't define what it is, but if its something else, we should look into it?
    check_one_doc(&mut *docs_only, FreqIs::OneOrTwo, &[])?;
    // reuse that too
    let d1 = identity(&*docs_only);
    let mut docs_only2 = terms_enum.postings(Some(docs_only), NONE)?;
    case.assert_reused("foo", d1, identity(&*docs_only2));
    // and it had better work
    // we don't define what it is, but if its something else, we should look into it?
    check_one_doc(&mut *docs_only2, FreqIs::OneOrTwo, &[])?;

    // asking for positions, ok; payloads, offsets, etc don't cause an error if they
    // aren't there, but make sure they work; and reuse each
    for (flag, expected) in [
        (POSITIONS, positions),
        (PAYLOADS, payloads),
        (OFFSETS, offsets),
        (ALL, all),
    ] {
        let mut docs_and_positions_enum = npe(leaf.postings(&term("foo", "bar"), flag)?);
        check_one_doc(&mut *docs_and_positions_enum, FreqIs::Eq(2), expected)?;
        // now reuse the positions
        let e1 = identity(&*docs_and_positions_enum);
        let mut docs_and_positions_enum2 =
            terms_enum.postings(Some(docs_and_positions_enum), flag)?;
        case.assert_reused("foo", e1, identity(&*docs_and_positions_enum2));
        check_one_doc(&mut *docs_and_positions_enum2, FreqIs::Eq(2), expected)?;
    }
    Ok(())
}

/// `checkReuse(termsEnum, firstFlags, secondFlags, shouldReuse)`.
///
/// # Errors
///
/// The errors of the terms enum.
pub fn check_reuse(
    terms_enum: &mut dyn TermsEnum,
    first_flags: i32,
    second_flags: i32,
    should_reuse: bool,
) -> Result<()> {
    let postings1 = terms_enum.postings(None, first_flags)?;
    let p1 = identity(&*postings1);
    let class = class_name_of(&*postings1);
    let postings2 = terms_enum.postings(Some(postings1), second_flags)?;
    if should_reuse {
        assert_same(
            &format!("Expected PostingsEnum {class} to be reused"),
            true,
            p1,
            identity(&*postings2),
        );
    } else {
        assert_same(
            &format!("Expected PostingsEnum {class} to not be reused"),
            false,
            p1,
            identity(&*postings2),
        );
    }
    Ok(())
}

/// A `SeekStatus` as JUnit prints it (the enum constant's name).
#[derive(PartialEq)]
struct JavaSeekStatus(SeekStatus);

impl fmt::Display for JavaSeekStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            SeekStatus::END => "END",
            SeekStatus::FOUND => "FOUND",
            SeekStatus::NOT_FOUND => "NOT_FOUND",
        })
    }
}

/// A nullable `BytesRef` as JUnit prints it.
#[derive(PartialEq)]
struct JavaBytesRef(Option<BytesRef>);

impl fmt::Display for JavaBytesRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            None => f.write_str("null"),
            Some(b) => write!(f, "{b}"),
        }
    }
}

// -----------------------------------------------------------------------------
// testInvertedWrite's anonymous classes
// -----------------------------------------------------------------------------

/// `TermFreqs` (`private static class`).
#[derive(Debug, Default, Clone, Copy)]
struct TermFreqs {
    total_term_freq: i64,
    doc_freq: i32,
}

/// The state `testInvertedWrite` shares with its postings format: `termFreqs`,
/// `sumDocFreq` and `sumTotalTermFreq`.
#[derive(Debug, Default)]
struct InvertedWriteState {
    term_freqs: Mutex<ConcurrentHashMapKeys<TermFreqs>>,
    sum_doc_freq: AtomicI64,
    sum_total_term_freq: AtomicI64,
}

/// The anonymous `PostingsFormat` of `testInvertedWrite`: counts up some
/// stats and then in the end we verify the stats match what the final
/// IndexReader says, just to exercise the new freedom of iterating the
/// postings more than once at flush/merge.
#[derive(Debug)]
struct InvertedWritePostingsFormat {
    codec: Arc<dyn Codec>,
    main_thread: std::thread::ThreadId,
    state: Arc<InvertedWriteState>,
}

impl PostingsFormat for InvertedWritePostingsFormat {
    fn name(&self) -> &str {
        self.codec.postings_format().name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let fields_consumer = self.codec.postings_format().fields_consumer(state)?;
        Ok(Box::new(InvertedWriteConsumer {
            fields_consumer,
            is_merge: state.context.context() == Context::Merge,
            main_thread: self.main_thread,
            state: Arc::clone(&self.state),
        }))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        self.codec.postings_format().fields_producer(state)
    }
}

/// The anonymous `FieldsConsumer` of `testInvertedWrite`.
struct InvertedWriteConsumer<'a> {
    fields_consumer: Box<dyn FieldsConsumer + 'a>,
    is_merge: bool,
    main_thread: std::thread::ThreadId,
    state: Arc<InvertedWriteState>,
}

impl FieldsConsumer for InvertedWriteConsumer<'_> {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        self.fields_consumer.write(fields, norms)?;

        let is_merge = self.is_merge;

        // We only use one thread for flushing
        // in this test:
        debug_assert!(is_merge || std::thread::current().id() == self.main_thread);

        // We iterate the provided TermsEnum
        // twice, so we excercise this new freedom
        // with the inverted API; if
        // addOnSecondPass is true, we add up
        // term stats on the 2nd iteration:
        let add_on_second_pass = LuceneTestCase::random().next_boolean();

        // Gather our own stats:
        let terms = fields.terms("body")?;
        debug_assert!(terms.is_some());
        let terms = npe(terms);

        let mut term_freqs = self
            .state
            .term_freqs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut terms_enum = terms.iterator()?;
        let mut docs: Option<Box<dyn PostingsEnum>> = None;
        while terms_enum.next()?.is_some() {
            let term = terms_enum.term()?;
            // TODO: also sometimes ask for payloads/offsets?
            let no_positions = LuceneTestCase::random().next_boolean();
            let mut d = if no_positions {
                terms_enum.postings(docs.take(), FREQS)?
            } else {
                terms_enum.postings(None, POSITIONS)?
            };
            let mut doc_freq = 0;
            let mut total_term_freq: i64 = 0;
            while d.next_doc()? != NO_MORE_DOCS {
                doc_freq += 1;
                total_term_freq += i64::from(d.freq()?);
                let limit = TestUtil::next_int(&*LuceneTestCase::random(), 1, d.freq()?);
                if !no_positions {
                    for _ in 0..limit {
                        d.next_position()?;
                    }
                }
            }
            docs = Some(d);

            let term_string = term.utf8_to_string()?;

            // During merge we should only see terms
            // we had already seen during a
            // previous flush:
            Assert::assert_true(!is_merge || term_freqs.contains_key(&term_string));

            if !is_merge {
                if !add_on_second_pass {
                    let tf = term_freqs.get_or_insert(&term_string);
                    tf.doc_freq += doc_freq;
                    tf.total_term_freq += total_term_freq;
                    self.state
                        .sum_doc_freq
                        .fetch_add(i64::from(doc_freq), Ordering::SeqCst);
                    self.state
                        .sum_total_term_freq
                        .fetch_add(total_term_freq, Ordering::SeqCst);
                } else if !term_freqs.contains_key(&term_string) {
                    // Add placeholder (2nd pass will
                    // set its counts):
                    term_freqs.get_or_insert(&term_string);
                }
            }
        }

        // Also test seeking the TermsEnum:
        for term in term_freqs.keys() {
            if terms_enum.seek_exact(&BytesRef::from_char_sequence(&term))? {
                // TODO: also sometimes ask for payloads/offsets?
                let no_positions = LuceneTestCase::random().next_boolean();
                let mut d = if no_positions {
                    terms_enum.postings(docs.take(), FREQS)?
                } else {
                    terms_enum.postings(None, POSITIONS)?
                };

                let mut doc_freq = 0;
                let mut total_term_freq: i64 = 0;
                while d.next_doc()? != NO_MORE_DOCS {
                    doc_freq += 1;
                    total_term_freq += i64::from(d.freq()?);
                    let limit = TestUtil::next_int(&*LuceneTestCase::random(), 1, d.freq()?);
                    if !no_positions {
                        for _ in 0..limit {
                            d.next_position()?;
                        }
                    }
                }
                docs = Some(d);

                if !is_merge && add_on_second_pass {
                    let tf = term_freqs.get_mut(&term);
                    debug_assert!(tf.is_some());
                    let tf = npe(tf);
                    tf.doc_freq += doc_freq;
                    tf.total_term_freq += total_term_freq;
                    self.state
                        .sum_doc_freq
                        .fetch_add(i64::from(doc_freq), Ordering::SeqCst);
                    self.state
                        .sum_total_term_freq
                        .fetch_add(total_term_freq, Ordering::SeqCst);
                }

                let tf = npe(term_freqs.get(&term));
                Assert::assert_true(doc_freq <= tf.doc_freq);
                Assert::assert_true(total_term_freq <= tf.total_term_freq);
            }
        }

        // Also test seekCeil
        for _ in 0..10 {
            let term = BytesRef::from_char_sequence(&TestUtil::random_realistic_unicode_string(
                &*LuceneTestCase::random(),
            ));
            let status = terms_enum.seek_ceil(&term)?;
            if status == SeekStatus::NOT_FOUND {
                Assert::assert_true(term < terms_enum.term()?);
            }
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.fields_consumer.close()
    }
}

// -----------------------------------------------------------------------------
// ConcurrentHashMapKeys
// -----------------------------------------------------------------------------

/// A `ConcurrentHashMap<String, V>` that only grows, single-threaded, with
/// the key iteration order of JDK 25's `ConcurrentHashMap`: `putVal` appends
/// a new key to its bin (`(n - 1) & spread(hashCode())`, `spread(h) = (h ^
/// h >>> 16) & HASH_BITS`); `addCount` doubles the table while the size has
/// reached `sizeCtl` (three quarters of the table); `transfer` splits each
/// bin at its last run of nodes bound for the same half, which keeps its
/// order, and prepends the nodes before it, which reverses theirs; a bin of
/// eight keys (`TREEIFY_THRESHOLD`) presizes a table under 64 bins
/// (`tryPresize(n << 1)`) or becomes a `TreeBin`, which prepends new keys to
/// its `first` list and splits in order (back to a list at six keys or
/// fewer, `UNTREEIFY_THRESHOLD`); the traverser walks the bins in index
/// order.
#[derive(Debug, Default)]
pub struct ConcurrentHashMapKeys<V> {
    /// The bins, each a list of indices into `entries`, and whether it is a
    /// `TreeBin`.
    table: Vec<(Vec<usize>, bool)>,
    /// `sizeCtl` once the table exists.
    size_ctl: usize,
    /// The keys, their spread hashes and values, in insertion order.
    entries: Vec<(String, i32, V)>,
    /// Key to index in `entries`.
    index: HashMap<String, usize>,
}

const HASH_BITS: i32 = 0x7fff_ffff;
const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;

impl<V: Default> ConcurrentHashMapKeys<V> {
    /// `containsKey(key)`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    /// `get(key)`.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.index.get(key).map(|&i| &self.entries[i].2)
    }

    /// `get(key)` for an update in place.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.index.get(key).map(|&i| &mut self.entries[i].2)
    }

    /// `size()`.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The value of `key`, `put(key, new V())` first when absent.
    pub fn get_or_insert(&mut self, key: &str) -> &mut V {
        if !self.index.contains_key(key) {
            self.put_new(key);
        }
        let i = self.index[key];
        &mut self.entries[i].2
    }

    /// `keySet()` in iteration order.
    pub fn keys(&self) -> Vec<String> {
        self.table
            .iter()
            .flat_map(|(bin, _)| bin.iter().map(|&i| self.entries[i].0.clone()))
            .collect()
    }

    fn put_new(&mut self, key: &str) {
        if self.table.is_empty() {
            // `initTable()`: DEFAULT_CAPACITY bins, sizeCtl = n - (n >>> 2).
            self.table = vec![(Vec::new(), false); 16];
            self.size_ctl = 12;
        }
        let h = randomizedtesting::jdk::lang::string_hash_code(key);
        let spread = (h ^ ((h as u32) >> 16) as i32) & HASH_BITS;
        let e = self.entries.len();
        self.entries.push((key.to_string(), spread, V::default()));
        self.index.insert(key.to_string(), e);
        let n = self.table.len();
        let i = (n - 1) & spread as usize;
        let (bin, tree) = &mut self.table[i];
        let bin_count = if *tree {
            // `putTreeVal`: the new node becomes `first`.
            bin.insert(0, e);
            2
        } else {
            bin.push(e);
            bin.len()
        };
        if bin_count >= TREEIFY_THRESHOLD && !self.table[i].1 {
            // `treeifyBin(tab, i)`.
            if n < MIN_TREEIFY_CAPACITY {
                self.try_presize(n << 1);
            } else {
                self.table[i].1 = true;
            }
        }
        // `addCount(1L, binCount)`.
        while self.entries.len() >= self.size_ctl && self.table.len() < (1 << 30) {
            self.transfer();
        }
    }

    /// `tryPresize(size)` with an existing table.
    fn try_presize(&mut self, size: usize) {
        let c = table_size_for(size + (size >> 1) + 1);
        while c > self.size_ctl && self.table.len() < (1 << 30) {
            self.transfer();
        }
    }

    /// `transfer(tab, null)`: doubles the table.
    fn transfer(&mut self) {
        let n = self.table.len();
        let mut next: Vec<(Vec<usize>, bool)> = vec![(Vec::new(), false); n << 1];
        for (i, (bin, tree)) in std::mem::take(&mut self.table).into_iter().enumerate() {
            if bin.is_empty() {
                continue;
            }
            let bit = |e: usize| self.entries[e].1 as usize & n;
            if tree {
                let lo: Vec<usize> = bin.iter().copied().filter(|&e| bit(e) == 0).collect();
                let hi: Vec<usize> = bin.iter().copied().filter(|&e| bit(e) != 0).collect();
                let lo_tree = lo.len() > UNTREEIFY_THRESHOLD;
                let hi_tree = hi.len() > UNTREEIFY_THRESHOLD;
                next[i] = (lo, lo_tree);
                next[i + n] = (hi, hi_tree);
            } else {
                // `lastRun`: the start of the final run of nodes with the same bit.
                let mut last_run = 0;
                let mut run_bit = bit(bin[0]);
                for (k, &e) in bin.iter().enumerate().skip(1) {
                    let b = bit(e);
                    if b != run_bit {
                        run_bit = b;
                        last_run = k;
                    }
                }
                let mut ln: Vec<usize> = Vec::new();
                let mut hn: Vec<usize> = Vec::new();
                if run_bit == 0 {
                    ln.extend_from_slice(&bin[last_run..]);
                } else {
                    hn.extend_from_slice(&bin[last_run..]);
                }
                for &e in &bin[..last_run] {
                    if bit(e) == 0 {
                        ln.insert(0, e);
                    } else {
                        hn.insert(0, e);
                    }
                }
                next[i] = (ln, false);
                next[i + n] = (hn, false);
            }
        }
        self.table = next;
        // `sizeCtl = (n << 1) - (n >>> 1)`.
        self.size_ctl = (n << 1) - (n >> 1);
    }
}

/// `ConcurrentHashMap.tableSizeFor(c)`.
fn table_size_for(c: usize) -> usize {
    let mut n: usize = 1;
    while n < c {
        n <<= 1;
    }
    n.min(1 << 30)
}

// -----------------------------------------------------------------------------
// RamUsageTester.ramUsed(justBodyDoc)
// -----------------------------------------------------------------------------

/// The Java object graph of `testInvertedWrite`'s `justBodyDoc`, as
/// `RamUsageTester.ramUsed` walks it on JDK 25: the `Document`, its
/// `ArrayList` of fields, the `body` `Field` of `LineFileDocs` (`type`,
/// `name`, `fieldsData`) and its `FieldType` (eight `boolean`s, four `int`s,
/// five enum references and a `null` attribute map), each of the five enum
/// constants at its own shallow size.
struct JustBodyDoc {
    fields: JustBodyFields,
}

impl JustBodyDoc {
    fn new(body: &str) -> Self {
        JustBodyDoc {
            fields: JustBodyFields {
                body: BodyField {
                    value: body.to_string(),
                },
            },
        }
    }
}

/// The `ArrayList<IndexableField>` of a [`JustBodyDoc`].
struct JustBodyFields {
    body: BodyField,
}

/// The `body` `Field` of a [`JustBodyDoc`].
struct BodyField {
    value: String,
}

/// The `FieldType` of [`BodyField`].
struct BodyFieldType;

/// An enum constant of [`BodyFieldType`]: its class, and how many `int`
/// fields the enum declares beyond `java.lang.Enum`'s.
struct EnumConstant(&'static str, usize);

static BODY_NAME: LazyLock<String> = LazyLock::new(|| "body".to_string());

impl RamObject for JustBodyDoc {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.Document"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[4]),
            fields: vec![("fields", Some(&self.fields as &dyn RamObject))],
        }
    }
}

impl RamObject for JustBodyFields {
    fn java_class_name(&self) -> &'static str {
        "java.util.ArrayList"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::JavaIterable {
            // object header, `modCount`, `size`, `elementData`
            shallow_size: aligned_shallow_instance_size(&[4, 4, 4]),
            elements: vec![Some(&self.body as &dyn RamObject)],
        }
    }
}

impl RamObject for BodyField {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.Field"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[4, 4, 4]),
            fields: vec![
                ("type", Some(&BodyFieldType as &dyn RamObject)),
                ("name", Some(&*BODY_NAME as &dyn RamObject)),
                ("fieldsData", Some(&self.value as &dyn RamObject)),
            ],
        }
    }
}

impl RamObject for BodyFieldType {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.FieldType"
    }

    fn java_object(&self) -> JavaObject<'_> {
        static INDEX_OPTIONS: EnumConstant =
            EnumConstant("org.apache.lucene.index.IndexOptions", 0);
        static DOC_VALUES_TYPE: EnumConstant =
            EnumConstant("org.apache.lucene.index.DocValuesType", 0);
        static SKIP_INDEX: EnumConstant =
            EnumConstant("org.apache.lucene.index.DocValuesSkipIndexType", 0);
        // `public final int byteSize` (`VectorEncoding.java:38`).
        static VECTOR_ENCODING: EnumConstant =
            EnumConstant("org.apache.lucene.index.VectorEncoding", 1);
        static SIMILARITY: EnumConstant =
            EnumConstant("org.apache.lucene.index.VectorSimilarityFunction", 0);
        JavaObject::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&[
                1, 1, 1, 1, 1, 1, 1, 1, // the booleans
                4, 4, 4, 4, // the ints
                4, 4, 4, 4, 4, 4, // the references
            ]),
            fields: vec![
                ("indexOptions", Some(&INDEX_OPTIONS as &dyn RamObject)),
                ("docValuesType", Some(&DOC_VALUES_TYPE as &dyn RamObject)),
                ("docValuesSkipIndex", Some(&SKIP_INDEX as &dyn RamObject)),
                ("vectorEncoding", Some(&VECTOR_ENCODING as &dyn RamObject)),
                (
                    "vectorSimilarityFunction",
                    Some(&SIMILARITY as &dyn RamObject),
                ),
                ("attributes", None),
            ],
        }
    }
}

impl RamObject for EnumConstant {
    fn java_class_name(&self) -> &'static str {
        self.0
    }

    fn java_object(&self) -> JavaObject<'_> {
        // `ob instanceof Enum` answers `RamUsageEstimator.shallowSizeOf(ob)`
        // (`RamUsageTester.java:190-191`): the header, `java.lang.Enum`'s
        // `name`, `ordinal` and (JDK 25) `hash`, and the enum's own fields;
        // measured on JDK 25 as 24 bytes, 32 for `VectorEncoding`.
        let mut fields = vec![4, 4, 4];
        fields.extend(std::iter::repeat_n(4, self.1));
        JavaObject::Enum {
            shallow_size: aligned_shallow_instance_size(&fields),
        }
    }
}
