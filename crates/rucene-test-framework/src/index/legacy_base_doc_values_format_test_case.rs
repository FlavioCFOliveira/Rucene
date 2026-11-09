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

//! Port of `org.apache.lucene.tests.index.LegacyBaseDocValuesFormatTestCase`:
//! abstract class to do basic tests for a docvalues format. NOTE: This test
//! focuses on the docvalues impl, nothing else. The [stretch] goal is for
//! this test to be so thorough in testing a new `DocValuesFormat` that if
//! this test passes, then all Lucene tests should also pass. Ie, if there is
//! some bug in a given `DocValuesFormat` that this test fails to catch then
//! this test needs to be improved!
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`LegacyBaseDocValuesFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs. `BaseDocValuesFormatTestCase` (which extends this
//! class) and `BaseCompressingDocValuesFormatTestCase` (which extends that)
//! are ported in
//! [`base_doc_values_format_test_case`](super::base_doc_values_format_test_case)
//! and
//! [`base_compressing_doc_values_format_test_case`](super::base_compressing_doc_values_format_test_case).
//!
//! **Divergences** (declared, each confined to the member named):
//! * `SortedDocValues.termsEnum()` / `SortedSetDocValues.termsEnum()` /
//!   `intersect(...)`: the port's methods consume the doc values (a Rust
//!   terms enum owns the doc values it reads), so where Java goes on using
//!   the doc values after taking its terms enum, the port asks the reader for
//!   the field's doc values again. Asking draws nothing from `random()`.
//! * Shared `Field` instances (`doTestNumericsVsStoredFields`,
//!   `doTestBinaryVsStoredFields`, `doTestSortedVsStoredFields`, …): Java
//!   adds one `Field` object to many documents and changes its value between
//!   additions. A Rust `Document` owns its fields, so the port adds, to each
//!   document, fields of the same names, types and values; the fields Java
//!   builds with `newField`/`newStringField` (which draw) are built once,
//!   with their draws, and copied with their randomized type.
//! * `new IndexWriterConfig(null)` (the tests that pass it): see the module
//!   docs of `base_index_file_format_test_case`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Condvar, LazyLock, Mutex, PoisonError};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::codecs::Codec;
use rucene::document::{
    BinaryDocValuesField, Document, Field, FloatDocValuesField, NumericDocValuesField,
    NumericValue, SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField,
    Store, StoredField, StringField, TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::{DocValues, SortedNumericDocValues, SortedSetDocValues};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::terms::{SeekStatus, Term};
use rucene::index::{IndexReader, IndexableField, LeafReader};
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQueryBuilder, DocIdSetIterator, Query, NO_MORE_DOCS};
use rucene::store::Directory;
use rucene::util::automaton::compiled_automaton::CompiledAutomaton;
use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::automaton::reg_exp::RegExp;
use rucene::util::byte_block_pool::ByteBlockPool;
use rucene::util::bytes_ref_hash::BytesRefHash;
use rucene::util::{BytesRef, BytesRefBuilder, FixedBitSet};

use super::base_compressing_doc_values_format_test_case::BaseCompressingDocValuesFormatTestCase;
use super::base_doc_values_format_test_case::BaseDocValuesFormatTestCase;
use super::base_index_file_format_test_case::{
    assert_array_equals, format_test, BaseIndexFileFormatTestCase,
};
use super::base_stored_fields_format_test_case::java_hash_map_key_order;
use crate::analysis::MockAnalyzer;
use crate::index::random_index_writer::RandomIndexWriter;
use crate::util::lucene_test_case::{LuceneTestCase, Nightly, TestDirectory, VERBOSE};
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.LegacyBaseDocValuesFormatTestCase`
/// (`public abstract class LegacyBaseDocValuesFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn LegacyBaseDocValuesFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(doc_values_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.LegacyBaseDocValuesFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testOneNumber", |c| c.test_one_number()))
            .method(t("testOneFloat", |c| c.test_one_float()))
            .method(t("testTwoNumbers", |c| c.test_two_numbers()))
            .method(t("testTwoBinaryValues", |c| c.test_two_binary_values()))
            .method(t("testVariouslyCompressibleBinaryValues", |c| {
                c.test_variously_compressible_binary_values()
            }))
            .method(t("testTwoFieldsMixed", |c| c.test_two_fields_mixed()))
            .method(t("testThreeFieldsMixed", |c| c.test_three_fields_mixed()))
            .method(t("testThreeFieldsMixed2", |c| c.test_three_fields_mixed2()))
            .method(t("testTwoDocumentsNumeric", |c| {
                c.test_two_documents_numeric()
            }))
            .method(t("testTwoDocumentsMerged", |c| {
                c.test_two_documents_merged()
            }))
            .method(t("testBigNumericRange", |c| c.test_big_numeric_range()))
            .method(t("testBigNumericRange2", |c| c.test_big_numeric_range2()))
            .method(t("testBytes", |c| c.test_bytes()))
            .method(t("testBytesTwoDocumentsMerged", |c| {
                c.test_bytes_two_documents_merged()
            }))
            .method(t("testBytesMergeAwayAllValues", |c| {
                c.test_bytes_merge_away_all_values()
            }))
            .method(t("testSortedBytes", |c| c.test_sorted_bytes()))
            .method(t("testSortedBytesTwoDocuments", |c| {
                c.test_sorted_bytes_two_documents()
            }))
            .method(t("testSortedBytesThreeDocuments", |c| {
                c.test_sorted_bytes_three_documents()
            }))
            .method(t("testSortedBytesTwoDocumentsMerged", |c| {
                c.test_sorted_bytes_two_documents_merged()
            }))
            .method(t("testSortedMergeAwayAllValues", |c| {
                c.test_sorted_merge_away_all_values()
            }))
            .method(t("testBytesWithNewline", |c| c.test_bytes_with_newline()))
            .method(t("testMissingSortedBytes", |c| {
                c.test_missing_sorted_bytes()
            }))
            .method(t("testSortedTermsEnum", |c| c.test_sorted_terms_enum()))
            .method(t("testEmptySortedBytes", |c| c.test_empty_sorted_bytes()))
            .method(t("testEmptyBytes", |c| c.test_empty_bytes()))
            .method(t("testVeryLargeButLegalBytes", |c| {
                c.test_very_large_but_legal_bytes()
            }))
            .method(t("testVeryLargeButLegalSortedBytes", |c| {
                c.test_very_large_but_legal_sorted_bytes()
            }))
            .method(t("testCodecUsesOwnBytes", |c| {
                c.test_codec_uses_own_bytes()
            }))
            .method(t("testCodecUsesOwnSortedBytes", |c| {
                c.test_codec_uses_own_sorted_bytes()
            }))
            .method(t("testDocValuesSimple", |c| c.test_doc_values_simple()))
            .method(t("testRandomSortedBytes", |c| c.test_random_sorted_bytes()))
            .method(t("testBooleanNumericsVsStoredFields", |c| {
                c.test_boolean_numerics_vs_stored_fields()
            }))
            .method(t("testSparseBooleanNumericsVsStoredFields", |c| {
                c.test_sparse_boolean_numerics_vs_stored_fields()
            }))
            .method(t("testByteNumericsVsStoredFields", |c| {
                c.test_byte_numerics_vs_stored_fields()
            }))
            .method(t("testSparseByteNumericsVsStoredFields", |c| {
                c.test_sparse_byte_numerics_vs_stored_fields()
            }))
            .method(t("testShortNumericsVsStoredFields", |c| {
                c.test_short_numerics_vs_stored_fields()
            }))
            .method(t("testSparseShortNumericsVsStoredFields", |c| {
                c.test_sparse_short_numerics_vs_stored_fields()
            }))
            .method(t("testIntNumericsVsStoredFields", |c| {
                c.test_int_numerics_vs_stored_fields()
            }))
            .method(t("testSparseIntNumericsVsStoredFields", |c| {
                c.test_sparse_int_numerics_vs_stored_fields()
            }))
            .method(t("testLongNumericsVsStoredFields", |c| {
                c.test_long_numerics_vs_stored_fields()
            }))
            .method(t("testSparseLongNumericsVsStoredFields", |c| {
                c.test_sparse_long_numerics_vs_stored_fields()
            }))
            .method(t("testBinaryFixedLengthVsStoredFields", |c| {
                c.test_binary_fixed_length_vs_stored_fields()
            }))
            .method(t("testSparseBinaryFixedLengthVsStoredFields", |c| {
                c.test_sparse_binary_fixed_length_vs_stored_fields()
            }))
            .method(t("testBinaryVariableLengthVsStoredFields", |c| {
                c.test_binary_variable_length_vs_stored_fields()
            }))
            .method(t("testSparseBinaryVariableLengthVsStoredFields", |c| {
                c.test_sparse_binary_variable_length_vs_stored_fields()
            }))
            .method(t("testSortedFixedLengthVsStoredFields", |c| {
                c.test_sorted_fixed_length_vs_stored_fields()
            }))
            .method(t("testSparseSortedFixedLengthVsStoredFields", |c| {
                c.test_sparse_sorted_fixed_length_vs_stored_fields()
            }))
            .method(t("testSortedVariableLengthVsStoredFields", |c| {
                c.test_sorted_variable_length_vs_stored_fields()
            }))
            .method(t("testSparseSortedVariableLengthVsStoredFields", |c| {
                c.test_sparse_sorted_variable_length_vs_stored_fields()
            }))
            .method(t("testSortedSetOneValue", |c| {
                c.test_sorted_set_one_value()
            }))
            .method(t("testSortedSetTwoFields", |c| {
                c.test_sorted_set_two_fields()
            }))
            .method(t("testSortedSetTwoDocumentsMerged", |c| {
                c.test_sorted_set_two_documents_merged()
            }))
            .method(t("testSortedSetTwoValues", |c| {
                c.test_sorted_set_two_values()
            }))
            .method(t("testSortedSetTwoValuesUnordered", |c| {
                c.test_sorted_set_two_values_unordered()
            }))
            .method(t("testSortedSetThreeValuesTwoDocs", |c| {
                c.test_sorted_set_three_values_two_docs()
            }))
            .method(t("testSortedSetTwoDocumentsLastMissing", |c| {
                c.test_sorted_set_two_documents_last_missing()
            }))
            .method(t("testSortedSetTwoDocumentsLastMissingMerge", |c| {
                c.test_sorted_set_two_documents_last_missing_merge()
            }))
            .method(t("testSortedSetTwoDocumentsFirstMissing", |c| {
                c.test_sorted_set_two_documents_first_missing()
            }))
            .method(t("testSortedSetTwoDocumentsFirstMissingMerge", |c| {
                c.test_sorted_set_two_documents_first_missing_merge()
            }))
            .method(t("testSortedSetMergeAwayAllValues", |c| {
                c.test_sorted_set_merge_away_all_values()
            }))
            .method(t("testSortedSetTermsEnum", |c| {
                c.test_sorted_set_terms_enum()
            }))
            .method(t("testSortedSetFixedLengthVsStoredFields", |c| {
                c.test_sorted_set_fixed_length_vs_stored_fields()
            }))
            .method(t("testSortedNumericsSingleValuedVsStoredFields", |c| {
                c.test_sorted_numerics_single_valued_vs_stored_fields()
            }))
            .method(t(
                "testSortedNumericsSingleValuedMissingVsStoredFields",
                |c| c.test_sorted_numerics_single_valued_missing_vs_stored_fields(),
            ))
            .method(t("testSortedNumericsMultipleValuesVsStoredFields", |c| {
                c.test_sorted_numerics_multiple_values_vs_stored_fields()
            }))
            .method(t("testSortedNumericsFewUniqueSetsVsStoredFields", |c| {
                c.test_sorted_numerics_few_unique_sets_vs_stored_fields()
            }))
            .method(t("testSortedSetVariableLengthVsStoredFields", |c| {
                c.test_sorted_set_variable_length_vs_stored_fields()
            }))
            .method(t(
                "testSortedSetFixedLengthSingleValuedVsStoredFields",
                |c| c.test_sorted_set_fixed_length_single_valued_vs_stored_fields(),
            ))
            .method(t(
                "testSortedSetVariableLengthSingleValuedVsStoredFields",
                |c| c.test_sorted_set_variable_length_single_valued_vs_stored_fields(),
            ))
            .method(t(
                "testSortedSetFixedLengthFewUniqueSetsVsStoredFields",
                |c| c.test_sorted_set_fixed_length_few_unique_sets_vs_stored_fields(),
            ))
            .method(t(
                "testSortedSetVariableLengthFewUniqueSetsVsStoredFields",
                |c| c.test_sorted_set_variable_length_few_unique_sets_vs_stored_fields(),
            ))
            .method(t(
                "testSortedSetVariableLengthManyValuesPerDocVsStoredFields",
                |c| c.test_sorted_set_variable_length_many_values_per_doc_vs_stored_fields(),
            ))
            .method(t(
                "testSortedSetFixedLengthManyValuesPerDocVsStoredFields",
                |c| c.test_sorted_set_fixed_length_many_values_per_doc_vs_stored_fields(),
            ))
            .method(t("testGCDCompression", |c| c.test_gcd_compression()))
            .method(t("testSparseGCDCompression", |c| {
                c.test_sparse_gcd_compression()
            }))
            .method(t("testZeros", |c| c.test_zeros()))
            .method(t("testSparseZeros", |c| c.test_sparse_zeros()))
            .method(t("testZeroOrMin", |c| c.test_zero_or_min()))
            .method(t("testTwoNumbersOneMissing", |c| {
                c.test_two_numbers_one_missing()
            }))
            .method(t("testTwoNumbersOneMissingWithMerging", |c| {
                c.test_two_numbers_one_missing_with_merging()
            }))
            .method(t("testThreeNumbersOneMissingWithMerging", |c| {
                c.test_three_numbers_one_missing_with_merging()
            }))
            .method(t("testTwoBytesOneMissing", |c| {
                c.test_two_bytes_one_missing()
            }))
            .method(t("testTwoBytesOneMissingWithMerging", |c| {
                c.test_two_bytes_one_missing_with_merging()
            }))
            .method(t("testThreeBytesOneMissingWithMerging", |c| {
                c.test_three_bytes_one_missing_with_merging()
            }))
            .method(t("testThreads", |c| c.test_threads()))
            .method(t("testThreads2", |c| c.test_threads2()).annotate(Nightly))
            .method(t("testThreads3", |c| c.test_threads3()).annotate(Nightly))
            .method(
                t("testEmptyBinaryValueOnPageSizes", |c| {
                    c.test_empty_binary_value_on_page_sizes()
                })
                .annotate(Nightly),
            )
            .method(t("testOneSortedNumber", |c| c.test_one_sorted_number()))
            .method(t("testOneSortedNumberOneMissing", |c| {
                c.test_one_sorted_number_one_missing()
            }))
            .method(t("testNumberMergeAwayAllValues", |c| {
                c.test_number_merge_away_all_values()
            }))
            .method(t("testTwoSortedNumber", |c| c.test_two_sorted_number()))
            .method(t("testTwoSortedNumberSameValue", |c| {
                c.test_two_sorted_number_same_value()
            }))
            .method(t("testTwoSortedNumberOneMissing", |c| {
                c.test_two_sorted_number_one_missing()
            }))
            .method(t("testSortedNumberMerge", |c| c.test_sorted_number_merge()))
            .method(t("testSortedNumberMergeAwayAllValues", |c| {
                c.test_sorted_number_merge_away_all_values()
            }))
            .method(t("testSortedEnumAdvanceIndependently", |c| {
                c.test_sorted_enum_advance_independently()
            }))
            .method(t("testSortedSetEnumAdvanceIndependently", |c| {
                c.test_sorted_set_enum_advance_independently()
            }))
            .method(t("testSortedMergeAwayAllValuesLargeSegment", |c| {
                c.test_sorted_merge_away_all_values_large_segment()
            }))
            .method(t("testSortedSetMergeAwayAllValuesLargeSegment", |c| {
                c.test_sorted_set_merge_away_all_values_large_segment()
            }))
            .method(t("testNumericMergeAwayAllValuesLargeSegment", |c| {
                c.test_numeric_merge_away_all_values_large_segment()
            }))
            .method(t("testSortedNumericMergeAwayAllValuesLargeSegment", |c| {
                c.test_sorted_numeric_merge_away_all_values_large_segment()
            }))
            .method(t("testBinaryMergeAwayAllValuesLargeSegment", |c| {
                c.test_binary_merge_away_all_values_large_segment()
            }))
            .method(t("testRandomAdvanceNumeric", |c| {
                c.test_random_advance_numeric()
            }))
            .method(t("testRandomAdvanceBinary", |c| {
                c.test_random_advance_binary()
            }))
            .method(
                t("testHighOrdsSortedSetDV", |c| {
                    c.test_high_ords_sorted_set_dv()
                })
                .annotate(Nightly),
            )
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `LegacyBaseDocValuesFormatTestCase`.
fn doc_values_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn LegacyBaseDocValuesFormatTestCase {
    c.as_doc_values().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.LegacyBaseDocValuesFormatTestCase",
        )
        .throw()
    })
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// `random()`.
pub(crate) fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// A null result Java would dereference: `NullPointerException`.
pub(crate) fn npe<T>(value: Option<T>) -> T {
    value.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// `newBytesRef(s)`.
pub(crate) fn nbr(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

/// `assertEquals(long expected, long actual)`.
pub(crate) fn aeq(expected: impl Into<i64>, actual: impl Into<i64>) {
    Assert::assert_equals(expected.into(), actual.into());
}

/// `assertNotNull(object)`.
pub(crate) fn assert_not_null(present: bool) {
    if !present {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
    }
}

/// `assertNull(term)`: `expected null, but was:<term>`.
fn assert_null(actual: Option<BytesRef>) {
    if let Some(actual) = actual {
        Throwable::new(
            "java.lang.AssertionError",
            format!("expected null, but was:<{actual}>"),
        )
        .throw()
    }
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

/// `assertEquals(SeekStatus expected, SeekStatus actual)`.
pub(crate) fn assert_seek_status(expected: SeekStatus, actual: SeekStatus) {
    Assert::assert_equals(SeekStatusDisplay(expected), SeekStatusDisplay(actual));
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

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::new(field, BytesRef::from_char_sequence(text))
}

/// `new TermQuery(new Term(field, text))`.
fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `Document.get(name)`.
fn doc_get(doc: &Document, name: &str) -> Option<String> {
    doc.get_values(name).into_iter().next()
}

/// A copy of `field`: same name, same (randomized) type, value `value`.
fn copy_field(field: &Field, value: String) -> Result<Field> {
    Field::new(field.name(), value, field.field_type().clone())
}

/// `Long.parseLong(s)`.
fn parse_long(s: &str) -> i64 {
    s.parse().unwrap_or_else(|_| {
        Throwable::new(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
        .throw()
    })
}

/// `throw new RuntimeException(e)` inside a thread.
fn runtime_exception(e: LuceneError) -> ! {
    let (class, message) = crate::analysis::base_token_stream_test_case::java_class_and_message(&e);
    Throwable::new("java.lang.RuntimeException", format!("{class}: {message}")).throw()
}

/// `new MockAnalyzer(random())`.
fn mock_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::new(&*random()))
}

/// `new IndexWriterConfig(null)` (see the module docs of
/// `base_index_file_format_test_case`).
fn null_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(rucene::analysis::standard::StandardAnalyzer::new())
}

/// `Codec.getDefault().toString()`: the codec's name.
fn default_codec_name() -> String {
    rucene::codecs::get_default()
        .map(|c| c.name().to_string())
        .unwrap_or_else(|| "null".to_string())
}

/// `codec instanceof SimpleTextCodec`, by the codec's name (a codec of
/// another crate cannot be downcast from `dyn Codec`).
fn is_simple_text(codec: &dyn Codec) -> bool {
    codec.name() == "SimpleText"
}

/// `new CompiledAutomaton(new RegExp(regex).toAutomaton())`, the automaton
/// determinized first (`Operations.determinize(..,
/// DEFAULT_DETERMINIZE_WORK_LIMIT)`) when `determinize`.
fn compiled(regex: &str, determinize: bool) -> Result<CompiledAutomaton> {
    let mut automaton = RegExp::new(regex)?.to_automaton()?;
    if determinize {
        automaton = Operations::determinize(&automaton, DEFAULT_DETERMINIZE_WORK_LIMIT)
            .map_err(|e| LuceneError::IllegalArgument(e.to_string()))?;
    }
    CompiledAutomaton::from_automaton(automaton)
}

/// `maybeWrapWithMergingReader(DirectoryReader.open(directory))`.
fn open_reader<C: BaseIndexFileFormatTestCase + ?Sized>(
    case: &C,
    directory: &TestDirectory,
) -> Result<Arc<dyn IndexReader>> {
    let reader =
        case.maybe_wrap_with_merging_reader(directory_reader::open(directory.as_directory())?)?;
    Ok(reader as Arc<dyn IndexReader>)
}

/// `assert ireader.leaves().size() == 1; ireader.leaves().get(0).reader()`.
fn only_leaf(ireader: &Arc<dyn IndexReader>) -> Result<Arc<dyn LeafReader>> {
    let leaves = Arc::clone(ireader).leaves()?;
    java_assert_unchecked!(leaves.len() == 1);
    Ok(leaves[0].leaf_reader())
}

/// The long term most of the one-document tests index.
const LONG_TERM: &str = "longtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongterm";

/// The one-document tests on `new RandomIndexWriter(random(), directory)`:
/// see [`one_doc_search_test_with`].
fn one_doc_search_test<C: BaseIndexFileFormatTestCase + ?Sized>(
    case: &C,
    long_term: &str,
    add_fields: impl FnOnce(&mut Document) -> Result<()>,
    per_hit: impl Fn(&dyn LeafReader, i32) -> Result<()>,
) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let iwriter =
            RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
        one_doc_search_test_with(
            case,
            &mut directory,
            iwriter,
            long_term,
            add_fields,
            per_hit,
        )?;
    }
    directory.close()
}

/// The body of the one-document tests: indexes `fieldname` with the text
/// ending in `long_term` and the fields `add_fields` adds, searches it back
/// and runs `per_hit` on the only leaf for each hit.
fn one_doc_search_test_with<C: BaseIndexFileFormatTestCase + ?Sized>(
    case: &C,
    directory: &mut TestDirectory,
    iwriter: RandomIndexWriter,
    long_term: &str,
    add_fields: impl FnOnce(&mut Document) -> Result<()>,
    per_hit: impl Fn(&dyn LeafReader, i32) -> Result<()>,
) -> Result<()> {
    let mut doc = Document::new();
    let text = format!("This is the text to be indexed. {long_term}");
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "fieldname",
        &text,
        Store::YES,
    )));
    add_fields(&mut doc)?;
    iwriter.add_document(&doc)?;
    iwriter.close()?;
    drop(iwriter);

    // Now search the index:
    let ireader = open_reader(case, directory)?; // read-only=true
    let isearcher = IndexSearcher::new(Arc::clone(&ireader))?;
    let stored_fields = isearcher.stored_fields()?;

    aeq(1, isearcher.count(term_query("fieldname", long_term))?);
    let query = term_query("fieldname", "text");
    let hits = isearcher.search(query, 1)?;
    aeq(1, hits.total_hits.value());
    // Iterate through the results:
    for score_doc in &hits.score_docs {
        let doc_id = score_doc.doc;
        let hit_doc = stored_fields.document(doc_id)?;
        Assert::assert_equals_str(None, Some(&text), doc_get(&hit_doc, "fieldname").as_deref());
        let leaf = only_leaf(&ireader)?;
        per_hit(&*leaf, doc_id)?;
    }

    drop((stored_fields, isearcher));
    ireader.close()
}

/// `new MockAnalyzer(random())`, `newDirectory()`,
/// `newIndexWriterConfig(analyzer)` with `newLogMergePolicy()`, and the
/// `RandomIndexWriter` over them.
fn log_mp_writer() -> Result<(TestDirectory, RandomIndexWriter)> {
    let analyzer = mock_analyzer();
    let directory = LuceneTestCase::new_directory();
    let mut conf = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    conf.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let iwriter = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;
    Ok((directory, iwriter))
}

/// `newDirectory()`, then `new RandomIndexWriter(random(), directory)` or,
/// with `log_mp`, a `MockAnalyzer`, a config with `newLogMergePolicy()` and
/// the writer over them.
fn sorted_set_writer(log_mp: bool) -> Result<(TestDirectory, RandomIndexWriter)> {
    let directory = LuceneTestCase::new_directory();
    if !log_mp {
        let iwriter =
            RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
        return Ok((directory, iwriter));
    }
    let analyzer = mock_analyzer();
    let mut iwconfig = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    iwconfig.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let iwriter =
        RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwconfig)?;
    Ok((directory, iwriter))
}

/// A document holding one `SortedSetDocValuesField` of `field` per value,
/// in order.
fn sorted_set_doc(field: &str, values: &[&str]) -> Document {
    let mut doc = Document::new();
    for value in values {
        doc.add(Box::new(SortedSetDocValuesField::new(field, nbr(value))));
    }
    doc
}

/// `testBigNumericRange`/`testBigNumericRange2`: two documents with the
/// values `first` and `second`, merged.
fn two_numbers_merged<C: BaseIndexFileFormatTestCase + ?Sized>(
    case: &C,
    first: i64,
    second: i64,
) -> Result<()> {
    let (mut directory, iwriter) = log_mp_writer()?;
    {
        let iwriter = iwriter;
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("dv", first)));
        iwriter.add_document(&doc)?;
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("dv", second)));
        iwriter.add_document(&doc)?;
        iwriter.force_merge(1)?;
        iwriter.close()?;
        drop(iwriter);

        // Now search the index:
        let ireader = open_reader(case, &directory)?; // read-only=true
        let leaf = only_leaf(&ireader)?;
        let mut dv = npe(leaf.get_numeric_doc_values("dv")?);
        aeq(0, dv.next_doc()?);
        aeq(first, dv.long_value()?);
        aeq(1, dv.next_doc()?);
        aeq(second, dv.long_value()?);

        drop((dv, leaf));
        ireader.close()?;
    }
    directory.close()
}

/// The body of the `*MergeAwayAllValues` tests: the only document with the
/// field `field` builds is deleted and merged away; `check` runs on the
/// only leaf.
pub(crate) fn merge_away_all_values(
    field: Box<dyn Fn() -> Box<dyn IndexableField> + '_>,
    check: impl FnOnce(&dyn LeafReader) -> Result<()>,
) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let analyzer = mock_analyzer();
        let mut iwconfig = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
        iwconfig.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        let iwriter =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwconfig)?;

        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "0".to_string(),
            Store::NO,
        )?));
        iwriter.add_document(&doc)?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::NO,
        )?));
        doc.add(field());
        iwriter.add_document(&doc)?;
        iwriter.commit()?;
        iwriter.delete_documents(term("id", "1"))?;
        iwriter.force_merge(1)?;

        let ireader = iwriter.get_reader()?;
        iwriter.close()?;
        drop(iwriter);

        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
        check(&*leaf)?;

        drop(leaf);
        ireader.close()?;
    }
    directory.close()
}

/// The body of the `*MergeAwayAllValuesLargeSegment*` tests: the only
/// document with the field `field` builds, followed by `atLeast(1024)` empty
/// ones, is deleted and merged away; `check` runs on the only leaf.
pub(crate) fn merge_away_all_values_large_segment(
    field: Box<dyn Fn() -> Box<dyn IndexableField> + '_>,
    check: impl FnOnce(&dyn LeafReader) -> Result<()>,
) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let analyzer = mock_analyzer();
        let mut iwconfig = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
        iwconfig.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        let iwriter =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwconfig)?;

        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::NO,
        )?));
        doc.add(field());
        iwriter.add_document(&doc)?;
        let num_empty_docs = LuceneTestCase::at_least(1024);
        for _ in 0..num_empty_docs {
            iwriter.add_document(&Document::new())?;
        }
        iwriter.commit()?;
        iwriter.delete_documents(term("id", "1"))?;
        iwriter.force_merge(1)?;

        let ireader = iwriter.get_reader()?;
        iwriter.close()?;
        drop(iwriter);

        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
        check(&*leaf)?;

        drop(leaf);
        ireader.close()?;
    }
    directory.close()
}

/// `testSortedSetTwoValues` / `testSortedSetTwoValuesUnordered`: one
/// document with `hello` and `world`, added in the order `values` gives.
fn sorted_set_two_values(values: &[&str]) -> Result<()> {
    let (mut directory, iwriter) = sorted_set_writer(false)?;
    {
        let iwriter = iwriter;
        iwriter.add_document(&sorted_set_doc("field", values))?;

        let ireader = iwriter.get_reader()?;
        iwriter.close()?;
        drop(iwriter);

        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
        let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
        aeq(0, dv.next_doc()?);

        aeq(2, dv.doc_value_count()?);
        aeq(0, dv.next_ord()?);
        aeq(1, dv.next_ord()?);

        let bytes = dv.lookup_ord(0)?;
        Assert::assert_equals(nbr("hello"), bytes);

        let bytes = dv.lookup_ord(1)?;
        Assert::assert_equals(nbr("world"), bytes);

        drop((dv, leaf));
        ireader.close()?;
    }
    directory.close()
}

/// The `testSortedSetTwoDocuments{Last,First}Missing[Merge]` tests: a
/// document with `hello` and an empty one (the empty one second when
/// `last_missing`), committed between the two when `merge`.
fn sorted_set_one_missing(last_missing: bool, merge: bool) -> Result<()> {
    let (mut directory, iwriter) = sorted_set_writer(true)?;
    {
        let iwriter = iwriter;
        if last_missing {
            iwriter.add_document(&sorted_set_doc("field", &["hello"]))?;
            if merge {
                iwriter.commit()?;
            }
            iwriter.add_document(&Document::new())?;
        } else {
            iwriter.add_document(&Document::new())?;
            if merge {
                iwriter.commit()?;
            }
            iwriter.add_document(&sorted_set_doc("field", &["hello"]))?;
        }
        iwriter.force_merge(1)?;

        let ireader = iwriter.get_reader()?;
        iwriter.close()?;
        drop(iwriter);

        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
        let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
        aeq(1, dv.get_value_count()?);
        aeq(if last_missing { 0 } else { 1 }, dv.next_doc()?);

        aeq(1, dv.doc_value_count()?);
        aeq(0, dv.next_ord()?);

        let bytes = dv.lookup_ord(0)?;
        Assert::assert_equals(nbr("hello"), bytes);

        drop((dv, leaf));
        ireader.close()?;
    }
    directory.close()
}

/// The sorted-set `*VsStoredFields` tests whose lengths are fixed: `atLeast(1)`
/// runs of `doTestSortedSetVsStoredFields(atLeast(num_docs), ...)`.
fn sorted_set_vs_stored_fields_test<C: LegacyBaseDocValuesFormatTestCase + ?Sized>(
    case: &C,
    num_docs: i32,
    min_length: i32,
    max_length: i32,
    max_values_per_doc: i32,
    max_unique_values: i32,
) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        let num_docs = LuceneTestCase::at_least(num_docs);
        case.do_test_sorted_set_vs_stored_fields(
            num_docs,
            min_length,
            max_length,
            max_values_per_doc,
            max_unique_values,
        )?;
    }
    Ok(())
}

/// The repeated reads of `compareStoredFieldWithSortedNumericsDV` at `doc`.
fn check_sorted_numeric_repeats(
    doc_values: &mut dyn SortedNumericDocValues,
    doc: i32,
    stored_values: &[String],
) -> Result<()> {
    let repeats = 1 + random().next_int_bound(3);
    for r in 0..repeats {
        if r > 0 || random().next_boolean() {
            Assert::assert_true(doc_values.advance_exact(doc)?);
        }
        aeq(stored_values.len() as i64, doc_values.doc_value_count()?);
        for v in 0..doc_values.doc_value_count()? {
            let value = doc_values.next_value()?;
            Assert::assert_equals_str(
                None,
                stored_values.get(v as usize).map(String::as_str),
                Some(&value.to_string()),
            );
        }
    }
    Ok(())
}

/// The repeated reads of `compareStoredFieldWithSortedSetDV` at `doc`.
fn check_sorted_set_repeats(
    doc_values: &mut dyn SortedSetDocValues,
    doc: i32,
    stored_values: &[String],
) -> Result<()> {
    let repeats = 1 + random().next_int_bound(3);
    for r in 0..repeats {
        if r > 0 || random().next_boolean() {
            Assert::assert_true(doc_values.advance_exact(doc)?);
        }
        for v in 0..doc_values.doc_value_count()? {
            let ord = doc_values.next_ord()?;
            Assert::assert_equals_str(
                None,
                stored_values.get(v as usize).map(String::as_str),
                Some(&doc_values.lookup_ord(ord)?.utf8_to_string()?),
            );
        }
    }
    Ok(())
}

/// `testTwoNumbersOneMissing[WithMerging]` / `testThreeNumbersOneMissingWithMerging`.
fn numbers_one_missing<C: BaseIndexFileFormatTestCase + ?Sized>(
    case: &C,
    merging: bool,
    three: bool,
) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let mut conf = LuceneTestCase::new_index_writer_config_analyzer(null_analyzer());
        conf.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        let iw = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "0".to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(NumericDocValuesField::new("dv1", 0)));
        iw.add_document(&doc)?;
        if merging && !three {
            iw.commit()?;
        }
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::YES,
        )?));
        iw.add_document(&doc)?;
        if three {
            iw.commit()?;
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "id",
                "2".to_string(),
                Store::YES,
            )?));
            doc.add(Box::new(NumericDocValuesField::new("dv1", 5)));
            iw.add_document(&doc)?;
        }
        iw.force_merge(1)?;
        iw.close()?;
        drop(iw);

        let ir = open_reader(case, &directory)?;
        aeq(1, Arc::clone(&ir).leaves()?.len() as i64);
        let ar = Arc::clone(&ir).leaves()?[0].leaf_reader();
        let mut dv = npe(ar.get_numeric_doc_values("dv1")?);
        aeq(0, dv.next_doc()?);
        aeq(0, dv.long_value()?);
        if three {
            aeq(2, dv.next_doc()?);
            aeq(5, dv.long_value()?);
        } else {
            aeq(NO_MORE_DOCS, dv.next_doc()?);
        }
        drop((dv, ar));
        ir.close()?;
    }
    directory.close()
}

/// `testTwoBytesOneMissing[WithMerging]` / `testThreeBytesOneMissingWithMerging`.
fn bytes_one_missing<C: BaseIndexFileFormatTestCase + ?Sized>(
    case: &C,
    merging: bool,
    three: bool,
) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let mut conf = LuceneTestCase::new_index_writer_config_analyzer(null_analyzer());
        conf.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        let iw = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "0".to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(BinaryDocValuesField::new(
            "dv1",
            LuceneTestCase::new_bytes_ref(),
        )));
        iw.add_document(&doc)?;
        if merging && !three {
            iw.commit()?;
        }
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::YES,
        )?));
        iw.add_document(&doc)?;
        if three {
            iw.commit()?;
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "id",
                "2".to_string(),
                Store::YES,
            )?));
            doc.add(Box::new(BinaryDocValuesField::new("dv1", nbr("boo"))));
            iw.add_document(&doc)?;
        }
        iw.force_merge(1)?;
        iw.close()?;
        drop(iw);

        let ir = open_reader(case, &directory)?;
        aeq(1, Arc::clone(&ir).leaves()?.len() as i64);
        let ar = Arc::clone(&ir).leaves()?[0].leaf_reader();
        let mut dv = npe(ar.get_binary_doc_values("dv1")?);
        aeq(0, dv.next_doc()?);
        Assert::assert_equals(LuceneTestCase::new_bytes_ref(), dv.binary_value()?);
        if three {
            aeq(2, dv.next_doc()?);
            Assert::assert_equals(nbr("boo"), dv.binary_value()?);
        }
        aeq(NO_MORE_DOCS, dv.next_doc()?);
        drop((dv, ar));
        ir.close()?;
    }
    directory.close()
}

/// `testTwoSortedNumber` / `testTwoSortedNumberSameValue`: one document
/// with the two values, read back as `expected`.
fn two_sorted_numbers<C: BaseIndexFileFormatTestCase + ?Sized>(
    case: &C,
    first: i64,
    second: i64,
    expected: &[i64; 2],
) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let writer =
            RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
        let mut doc = Document::new();
        doc.add(Box::new(SortedNumericDocValuesField::new("dv", first)));
        doc.add(Box::new(SortedNumericDocValuesField::new("dv", second)));
        writer.add_document(&doc)?;
        writer.close()?;
        drop(writer);

        // Now search the index:
        let reader = open_reader(case, &directory)?;
        let leaf = only_leaf(&reader)?;
        let mut dv = npe(leaf.get_sorted_numeric_doc_values("dv")?);
        aeq(0, dv.next_doc()?);
        aeq(2, dv.doc_value_count()?);
        aeq(expected[0], dv.next_value()?);
        aeq(expected[1], dv.next_value()?);

        drop((dv, leaf));
        reader.close()?;
    }
    directory.close()
}

/// A `java.util.HashMap<String, String>` (or the `HashSet<String>` of its
/// keys) built with `new HashMap<>()`, remembering what its iteration order
/// depends on.
struct JavaStringMap {
    map: HashMap<String, String>,
    inserted: Vec<String>,
    max_size: usize,
}

impl JavaStringMap {
    fn new() -> Self {
        JavaStringMap {
            map: HashMap::new(),
            inserted: Vec::new(),
            max_size: 0,
        }
    }

    /// `put(key, value)` (`add(key)` of a set).
    fn put(&mut self, key: String, value: String) {
        if self.map.insert(key.clone(), value).is_none() {
            self.inserted.push(key);
            self.max_size = self.max_size.max(self.map.len());
        }
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    /// `keySet()` (`toArray(String[]::new)` of a set) in Java's order.
    fn keys(&self) -> Vec<String> {
        java_hash_map_key_order(&self.inserted, self.max_size)
    }

    /// `entrySet()` in Java's order.
    fn entries(&self) -> Vec<(String, String)> {
        self.keys()
            .into_iter()
            .map(|k| {
                let v = self.map[&k].clone();
                (k, v)
            })
            .collect()
    }
}

/// A `CountDownLatch` of one count.
struct Latch {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Latch {
    fn new() -> Self {
        Latch {
            open: Mutex::new(false),
            opened: Condvar::new(),
        }
    }

    /// `await()`.
    fn wait(&self) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        while !*open {
            open = self
                .opened
                .wait(open)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// `countDown()`.
    fn count_down(&self) {
        *self.open.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.opened.notify_all();
    }
}

/// `FieldCreator.iterator(r)`: the doc values of `field` over the reader.
pub type FieldIterator = Box<dyn Fn(&Arc<dyn IndexReader>) -> Result<Box<dyn DocIdSetIterator>>>;

/// Port of `LegacyBaseDocValuesFormatTestCase.FieldCreator`.
pub struct FieldCreator {
    /// `next()`.
    pub next: Box<dyn Fn() -> Box<dyn IndexableField>>,
    /// `iterator(r)`.
    pub iterator: FieldIterator,
}

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `LegacyBaseDocValuesFormatTestCase`.
pub trait LegacyBaseDocValuesFormatTestCase: BaseIndexFileFormatTestCase {
    /// The subclass as a `BaseDocValuesFormatTestCase`, when it is one.
    fn as_base_doc_values(&self) -> Option<&dyn BaseDocValuesFormatTestCase> {
        None
    }

    /// The subclass as a `BaseCompressingDocValuesFormatTestCase`, when it
    /// is one.
    fn as_compressing_doc_values(&self) -> Option<&dyn BaseCompressingDocValuesFormatTestCase> {
        None
    }

    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn doc_values_add_random_fields(&self, doc: &mut Document) {
        if LuceneTestCase::usually() {
            doc.add(Box::new(NumericDocValuesField::new(
                "ndv",
                i64::from(random().next_int_bound(1 << 12)),
            )));
            doc.add(Box::new(BinaryDocValuesField::new(
                "bdv",
                nbr(&TestUtil::random_simple_string(&*random())),
            )));
            doc.add(Box::new(SortedDocValuesField::new(
                "sdv",
                nbr(&TestUtil::random_simple_string_max(&*random(), 2)),
            )));
        }
        let num_values = random().next_int_bound(5);
        for _ in 0..num_values {
            doc.add(Box::new(SortedSetDocValuesField::new(
                "ssdv",
                nbr(&TestUtil::random_simple_string_max(&*random(), 2)),
            )));
        }
        let num_values = random().next_int_bound(5);
        for _ in 0..num_values {
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "sndv",
                TestUtil::next_long(&*random(), i64::MIN, i64::MAX),
            )));
        }
    }

    /// `testOneNumber()`.
    fn test_one_number(&self) -> Result<()> {
        let long_term = concat!(
            "longtermlongtermlongtermlongtermlongtermlongtermlongtermlongterm",
            "longtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongterm"
        );
        one_doc_search_test(
            self,
            long_term,
            |doc| {
                doc.add(Box::new(NumericDocValuesField::new("dv", 5)));
                Ok(())
            },
            |leaf, doc_id| {
                let mut dv = npe(leaf.get_numeric_doc_values("dv")?);
                aeq(doc_id, dv.advance(doc_id)?);
                aeq(5, dv.long_value()?);
                Ok(())
            },
        )
    }

    /// `testOneFloat()`.
    fn test_one_float(&self) -> Result<()> {
        one_doc_search_test(
            self,
            LONG_TERM,
            |doc| {
                doc.add(Box::new(FloatDocValuesField::new("dv", 5.7f32)));
                Ok(())
            },
            |leaf, doc_id| {
                let mut dv = npe(leaf.get_numeric_doc_values("dv")?);
                aeq(doc_id, dv.advance(doc_id)?);
                aeq(5.7f32.to_bits() as i32, dv.long_value()?);
                Ok(())
            },
        )
    }

    /// `testTwoNumbers()`.
    fn test_two_numbers(&self) -> Result<()> {
        one_doc_search_test(
            self,
            LONG_TERM,
            |doc| {
                doc.add(Box::new(NumericDocValuesField::new("dv1", 5)));
                doc.add(Box::new(NumericDocValuesField::new("dv2", 17)));
                Ok(())
            },
            |leaf, doc_id| {
                let mut dv = npe(leaf.get_numeric_doc_values("dv1")?);
                aeq(doc_id, dv.advance(doc_id)?);
                aeq(5, dv.long_value()?);
                let mut dv = npe(leaf.get_numeric_doc_values("dv2")?);
                aeq(doc_id, dv.advance(doc_id)?);
                aeq(17, dv.long_value()?);
                Ok(())
            },
        )
    }

    /// `testTwoBinaryValues()`.
    fn test_two_binary_values(&self) -> Result<()> {
        let text = format!("This is the text to be indexed. {LONG_TERM}");
        one_doc_search_test(
            self,
            LONG_TERM,
            |doc| {
                doc.add(Box::new(BinaryDocValuesField::new("dv1", nbr(LONG_TERM))));
                doc.add(Box::new(BinaryDocValuesField::new("dv2", nbr(&text))));
                Ok(())
            },
            |leaf, hit_doc_id| {
                let mut dv = npe(leaf.get_binary_doc_values("dv1")?);
                aeq(hit_doc_id, dv.advance(hit_doc_id)?);
                let scratch = dv.binary_value()?;
                Assert::assert_equals(nbr(LONG_TERM), scratch);
                let mut dv = npe(leaf.get_binary_doc_values("dv2")?);
                aeq(hit_doc_id, dv.advance(hit_doc_id)?);
                let scratch = dv.binary_value()?;
                Assert::assert_equals(nbr(&text), scratch);
                Ok(())
            },
        )
    }

    /// `testVariouslyCompressibleBinaryValues()`.
    fn test_variously_compressible_binary_values(&self) -> Result<()> {
        let mut directory = LuceneTestCase::new_directory();
        {
            let iwriter =
                RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
            let num_docs = 1 + random().next_int_bound(100);

            let mut written_values: HashMap<i32, BytesRef> = HashMap::new();

            // Small vocabulary ranges will be highly compressible
            let vocab_range = 1 + random().next_int_bound(i32::from(i8::MAX) - 1);

            for i in 0..num_docs {
                let mut doc = Document::new();

                // Generate random-sized byte array with random choice of bytes in vocab range
                let mut value = vec![0u8; (500 + random().next_int_bound(1024)) as usize];
                for b in value.iter_mut() {
                    *b = random().next_int_bound(vocab_range) as u8;
                }
                let bytes_ref = LuceneTestCase::new_bytes_ref_bytes(&value);
                written_values.insert(i, bytes_ref.clone());
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "id",
                    &i.to_string(),
                    Store::YES,
                )));
                doc.add(Box::new(BinaryDocValuesField::new("dv1", bytes_ref)));
                iwriter.add_document(&doc)?;
            }
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let isearcher = IndexSearcher::new(Arc::clone(&ireader))?;
            let stored_fields = isearcher.stored_fields()?;

            for i in 0..num_docs {
                let id = i.to_string();
                let query = term_query("id", &id);
                let hits = isearcher.search(query, 1)?;
                aeq(1, hits.total_hits.value());
                // Iterate through the results:
                let hit_doc_id = hits.score_docs[0].doc;
                let hit_doc = stored_fields.document(hit_doc_id)?;
                Assert::assert_equals_str(
                    None,
                    Some(id.as_str()),
                    doc_get(&hit_doc, "id").as_deref(),
                );
                let leaf = only_leaf(&ireader)?;
                let mut dv = npe(leaf.get_binary_doc_values("dv1")?);
                aeq(hit_doc_id, dv.advance(hit_doc_id)?);
                let scratch = dv.binary_value()?;
                Assert::assert_equals(Boxed(written_values.get(&i).cloned()), Boxed(Some(scratch)));
            }

            drop((stored_fields, isearcher));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testTwoFieldsMixed()`.
    fn test_two_fields_mixed(&self) -> Result<()> {
        one_doc_search_test(
            self,
            LONG_TERM,
            |doc| {
                doc.add(Box::new(NumericDocValuesField::new("dv1", 5)));
                doc.add(Box::new(BinaryDocValuesField::new(
                    "dv2",
                    nbr("hello world"),
                )));
                Ok(())
            },
            |leaf, doc_id| {
                let mut dv = npe(leaf.get_numeric_doc_values("dv1")?);
                aeq(doc_id, dv.advance(doc_id)?);
                aeq(5, dv.long_value()?);
                let mut dv2 = npe(leaf.get_binary_doc_values("dv2")?);
                aeq(doc_id, dv2.advance(doc_id)?);
                Assert::assert_equals(nbr("hello world"), dv2.binary_value()?);
                Ok(())
            },
        )
    }

    /// `testThreeFieldsMixed()`.
    fn test_three_fields_mixed(&self) -> Result<()> {
        one_doc_search_test(
            self,
            LONG_TERM,
            |doc| {
                doc.add(Box::new(SortedDocValuesField::new(
                    "dv1",
                    nbr("hello hello"),
                )));
                doc.add(Box::new(NumericDocValuesField::new("dv2", 5)));
                doc.add(Box::new(BinaryDocValuesField::new(
                    "dv3",
                    nbr("hello world"),
                )));
                Ok(())
            },
            |leaf, doc_id| {
                let mut dv = npe(leaf.get_sorted_doc_values("dv1")?);
                aeq(doc_id, dv.advance(doc_id)?);
                let ord = dv.ord_value()?;
                let scratch = dv.lookup_ord(ord)?;
                Assert::assert_equals(nbr("hello hello"), scratch);
                let mut dv2 = npe(leaf.get_numeric_doc_values("dv2")?);
                aeq(doc_id, dv2.advance(doc_id)?);
                aeq(5, dv2.long_value()?);
                let mut dv3 = npe(leaf.get_binary_doc_values("dv3")?);
                aeq(doc_id, dv3.advance(doc_id)?);
                Assert::assert_equals(nbr("hello world"), dv3.binary_value()?);
                Ok(())
            },
        )
    }

    /// `testThreeFieldsMixed2()`.
    fn test_three_fields_mixed2(&self) -> Result<()> {
        one_doc_search_test(
            self,
            LONG_TERM,
            |doc| {
                doc.add(Box::new(BinaryDocValuesField::new(
                    "dv1",
                    nbr("hello world"),
                )));
                doc.add(Box::new(SortedDocValuesField::new(
                    "dv2",
                    nbr("hello hello"),
                )));
                doc.add(Box::new(NumericDocValuesField::new("dv3", 5)));
                Ok(())
            },
            |leaf, doc_id| {
                let mut dv = npe(leaf.get_sorted_doc_values("dv2")?);
                aeq(doc_id, dv.advance(doc_id)?);
                let ord = dv.ord_value()?;
                let scratch = dv.lookup_ord(ord)?;
                Assert::assert_equals(nbr("hello hello"), scratch);
                let mut dv2 = npe(leaf.get_numeric_doc_values("dv3")?);
                aeq(doc_id, dv2.advance(doc_id)?);
                aeq(5, dv2.long_value()?);
                let mut dv3 = npe(leaf.get_binary_doc_values("dv1")?);
                aeq(doc_id, dv3.advance(doc_id)?);
                Assert::assert_equals(nbr("hello world"), dv3.binary_value()?);
                Ok(())
            },
        )
    }

    /// `testTwoDocumentsNumeric()`.
    fn test_two_documents_numeric(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::new("dv", 1)));
            iwriter.add_document(&doc)?;
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::new("dv", 2)));
            iwriter.add_document(&doc)?;
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_numeric_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            aeq(1, dv.long_value()?);
            aeq(1, dv.next_doc()?);
            aeq(2, dv.long_value()?);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testTwoDocumentsMerged()`.
    fn test_two_documents_merged(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "id",
                "0",
                StringField::type_stored(),
            )));
            doc.add(Box::new(NumericDocValuesField::new("dv", -10)));
            iwriter.add_document(&doc)?;
            iwriter.commit()?;
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "id",
                "1",
                StringField::type_stored(),
            )));
            doc.add(Box::new(NumericDocValuesField::new("dv", 99)));
            iwriter.add_document(&doc)?;
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_numeric_doc_values("dv")?);
            let stored_fields = leaf.stored_fields()?;
            for i in 0..2 {
                let doc2 = stored_fields.document(i)?;
                let expected = if npe(doc_get(&doc2, "id")) == "0" {
                    -10
                } else {
                    99
                };
                aeq(i, dv.next_doc()?);
                aeq(expected, dv.long_value()?);
            }

            drop((dv, stored_fields, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testBigNumericRange()`.
    fn test_big_numeric_range(&self) -> Result<()> {
        two_numbers_merged(self, i64::MIN, i64::MAX)
    }

    /// `testBigNumericRange2()`.
    fn test_big_numeric_range2(&self) -> Result<()> {
        two_numbers_merged(self, -8841491950446638677, 9062230939892376225)
    }

    /// `testBytes()`.
    fn test_bytes(&self) -> Result<()> {
        let analyzer = mock_analyzer();
        let mut directory = LuceneTestCase::new_directory();
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
            let iwriter =
                RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;
            one_doc_search_test_with(
                self,
                &mut directory,
                iwriter,
                LONG_TERM,
                |doc| {
                    doc.add(Box::new(BinaryDocValuesField::new(
                        "dv",
                        nbr("hello world"),
                    )));
                    Ok(())
                },
                |leaf, hit_doc_id| {
                    let mut dv = npe(leaf.get_binary_doc_values("dv")?);
                    aeq(hit_doc_id, dv.advance(hit_doc_id)?);
                    Assert::assert_equals(nbr("hello world"), dv.binary_value()?);
                    Ok(())
                },
            )?;
        }
        directory.close()
    }

    /// `testBytesTwoDocumentsMerged()`.
    fn test_bytes_two_documents_merged(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "id",
                "0",
                StringField::type_stored(),
            )));
            doc.add(Box::new(BinaryDocValuesField::new(
                "dv",
                nbr("hello world 1"),
            )));
            iwriter.add_document(&doc)?;
            iwriter.commit()?;
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "id",
                "1",
                StringField::type_stored(),
            )));
            doc.add(Box::new(BinaryDocValuesField::new("dv", nbr("hello 2"))));
            iwriter.add_document(&doc)?;
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_binary_doc_values("dv")?);
            let stored_fields = leaf.stored_fields()?;
            for i in 0..2 {
                let doc2 = stored_fields.document(i)?;
                let expected = if npe(doc_get(&doc2, "id")) == "0" {
                    "hello world 1"
                } else {
                    "hello 2"
                };
                aeq(i, dv.next_doc()?);
                Assert::assert_equals(expected.to_string(), dv.binary_value()?.utf8_to_string()?);
            }

            drop((dv, stored_fields, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testBytesMergeAwayAllValues()`.
    fn test_bytes_merge_away_all_values(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(BinaryDocValuesField::new("field", nbr("hi")))),
            |leaf| {
                let mut dv = npe(leaf.get_binary_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                Ok(())
            },
        )
    }

    /// `testSortedBytes()`.
    fn test_sorted_bytes(&self) -> Result<()> {
        let analyzer = mock_analyzer();
        let mut directory = LuceneTestCase::new_directory();
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
            let iwriter =
                RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;
            one_doc_search_test_with(
                self,
                &mut directory,
                iwriter,
                LONG_TERM,
                |doc| {
                    doc.add(Box::new(SortedDocValuesField::new(
                        "dv",
                        nbr("hello world"),
                    )));
                    Ok(())
                },
                |leaf, doc_id| {
                    let mut dv = npe(leaf.get_sorted_doc_values("dv")?);
                    aeq(doc_id, dv.advance(doc_id)?);
                    let scratch = dv.lookup_ord(dv.ord_value()?)?;
                    Assert::assert_equals(nbr("hello world"), scratch);
                    Ok(())
                },
            )?;
        }
        directory.close()
    }

    /// `testSortedBytesTwoDocuments()`.
    fn test_sorted_bytes_two_documents(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(SortedDocValuesField::new(
                "dv",
                nbr("hello world 1"),
            )));
            iwriter.add_document(&doc)?;
            let mut doc = Document::new();
            doc.add(Box::new(SortedDocValuesField::new(
                "dv",
                nbr("hello world 2"),
            )));
            iwriter.add_document(&doc)?;
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_sorted_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            let scratch = dv.lookup_ord(dv.ord_value()?)?;
            Assert::assert_equals("hello world 1".to_string(), scratch.utf8_to_string()?);
            aeq(1, dv.next_doc()?);
            let scratch = dv.lookup_ord(dv.ord_value()?)?;
            Assert::assert_equals("hello world 2".to_string(), scratch.utf8_to_string()?);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedBytesThreeDocuments()`.
    fn test_sorted_bytes_three_documents(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            for value in ["hello world 1", "hello world 2", "hello world 1"] {
                let mut doc = Document::new();
                doc.add(Box::new(SortedDocValuesField::new("dv", nbr(value))));
                iwriter.add_document(&doc)?;
            }
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_sorted_doc_values("dv")?);
            aeq(2, dv.get_value_count()?);
            aeq(0, dv.next_doc()?);
            aeq(0, dv.ord_value()?);
            let scratch = dv.lookup_ord(0)?;
            Assert::assert_equals("hello world 1".to_string(), scratch.utf8_to_string()?);
            aeq(1, dv.next_doc()?);
            aeq(1, dv.ord_value()?);
            let scratch = dv.lookup_ord(1)?;
            Assert::assert_equals("hello world 2".to_string(), scratch.utf8_to_string()?);
            aeq(2, dv.next_doc()?);
            aeq(0, dv.ord_value()?);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedBytesTwoDocumentsMerged()`.
    fn test_sorted_bytes_two_documents_merged(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "id",
                "0",
                StringField::type_stored(),
            )));
            doc.add(Box::new(SortedDocValuesField::new(
                "dv",
                nbr("hello world 1"),
            )));
            iwriter.add_document(&doc)?;
            iwriter.commit()?;
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "id",
                "1",
                StringField::type_stored(),
            )));
            doc.add(Box::new(SortedDocValuesField::new(
                "dv",
                nbr("hello world 2"),
            )));
            iwriter.add_document(&doc)?;
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_sorted_doc_values("dv")?);
            aeq(2, dv.get_value_count()?); // 2 ords
            aeq(0, dv.next_doc()?);
            let scratch = dv.lookup_ord(dv.ord_value()?)?;
            Assert::assert_equals(nbr("hello world 1"), scratch);
            let scratch = dv.lookup_ord(1)?;
            Assert::assert_equals(nbr("hello world 2"), scratch);
            let stored_fields = leaf.stored_fields()?;
            for i in 0..2 {
                let doc2 = stored_fields.document(i)?;
                let expected = if npe(doc_get(&doc2, "id")) == "0" {
                    "hello world 1"
                } else {
                    "hello world 2"
                };
                if dv.doc_id() < i {
                    aeq(i, dv.next_doc()?);
                }
                let scratch = dv.lookup_ord(dv.ord_value()?)?;
                Assert::assert_equals(expected.to_string(), scratch.utf8_to_string()?);
            }

            drop((dv, stored_fields, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedMergeAwayAllValues()`.
    fn test_sorted_merge_away_all_values(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(SortedDocValuesField::new("field", nbr("hello")))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);

                let mut terms_enum = dv.terms_enum()?;
                Assert::assert_false(
                    terms_enum.seek_exact(&BytesRef::from_char_sequence("lucene"))?,
                );
                assert_seek_status(
                    SeekStatus::END,
                    terms_enum.seek_ceil(&BytesRef::from_char_sequence("lucene"))?,
                );
                let dv = npe(leaf.get_sorted_doc_values("field")?);
                aeq(-1, dv.lookup_term(&BytesRef::from_char_sequence("lucene"))?);
                Ok(())
            },
        )
    }

    /// `testBytesWithNewline()`.
    fn test_bytes_with_newline(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(BinaryDocValuesField::new(
                "dv",
                nbr("hello\nworld\r1"),
            )));
            iwriter.add_document(&doc)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_binary_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            Assert::assert_equals(nbr("hello\nworld\r1"), dv.binary_value()?);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testMissingSortedBytes()`.
    fn test_missing_sorted_bytes(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(SortedDocValuesField::new(
                "dv",
                nbr("hello world 2"),
            )));
            iwriter.add_document(&doc)?;
            // 2nd doc missing the DV field
            iwriter.add_document(&Document::new())?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_sorted_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            let scratch = dv.lookup_ord(dv.ord_value()?)?;
            Assert::assert_equals(nbr("hello world 2"), scratch);
            aeq(NO_MORE_DOCS, dv.next_doc()?);
            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedTermsEnum()`.
    fn test_sorted_terms_enum(&self) -> Result<()> {
        let mut directory = LuceneTestCase::new_directory();
        {
            let analyzer = mock_analyzer();
            let mut iwconfig = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
            iwconfig.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
            let iwriter =
                RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwconfig)?;

            for value in ["hello", "world", "beer"] {
                let mut doc = Document::new();
                doc.add(Box::new(SortedDocValuesField::new("field", nbr(value))));
                iwriter.add_document(&doc)?;
            }
            iwriter.force_merge(1)?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let dv = npe(leaf.get_sorted_doc_values("field")?);
            aeq(3, dv.get_value_count()?);

            let mut terms_enum = dv.terms_enum()?;

            // next()
            Assert::assert_equals(
                "beer".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(0, terms_enum.ord()?);
            Assert::assert_equals(
                "hello".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(1, terms_enum.ord()?);
            Assert::assert_equals(
                "world".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(2, terms_enum.ord()?);

            // seekCeil()
            assert_seek_status(SeekStatus::NOT_FOUND, terms_enum.seek_ceil(&nbr("ha!"))?);
            Assert::assert_equals("hello".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(1, terms_enum.ord()?);
            assert_seek_status(SeekStatus::FOUND, terms_enum.seek_ceil(&nbr("beer"))?);
            Assert::assert_equals("beer".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(0, terms_enum.ord()?);
            assert_seek_status(SeekStatus::END, terms_enum.seek_ceil(&nbr("zzz"))?);
            assert_seek_status(SeekStatus::NOT_FOUND, terms_enum.seek_ceil(&nbr("aba"))?);
            aeq(0, terms_enum.ord()?);

            // seekExact()
            Assert::assert_true(terms_enum.seek_exact(&nbr("beer"))?);
            Assert::assert_equals("beer".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(0, terms_enum.ord()?);
            Assert::assert_true(terms_enum.seek_exact(&nbr("hello"))?);
            Assert::assert_equals_str(
                Some(&default_codec_name()),
                Some("hello"),
                Some(&terms_enum.term()?.utf8_to_string()?),
            );
            aeq(1, terms_enum.ord()?);
            Assert::assert_true(terms_enum.seek_exact(&nbr("world"))?);
            Assert::assert_equals("world".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(2, terms_enum.ord()?);
            Assert::assert_false(terms_enum.seek_exact(&nbr("bogus"))?);

            // seek(ord)
            terms_enum.seek_ord(0)?;
            Assert::assert_equals("beer".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(0, terms_enum.ord()?);
            terms_enum.seek_ord(1)?;
            Assert::assert_equals("hello".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(1, terms_enum.ord()?);
            terms_enum.seek_ord(2)?;
            Assert::assert_equals("world".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(2, terms_enum.ord()?);

            // NORMAL automaton
            let dv = npe(leaf.get_sorted_doc_values("field")?);
            let mut terms_enum = dv.intersect(&compiled(".*l.*", true)?)?;
            Assert::assert_equals(
                "hello".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(1, terms_enum.ord()?);
            Assert::assert_equals(
                "world".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(2, terms_enum.ord()?);
            assert_null(terms_enum.next()?);

            // SINGLE automaton
            let dv = npe(leaf.get_sorted_doc_values("field")?);
            let mut terms_enum = dv.intersect(&compiled("hello", false)?)?;
            Assert::assert_equals(
                "hello".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(1, terms_enum.ord()?);
            assert_null(terms_enum.next()?);

            drop((terms_enum, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testEmptySortedBytes()`.
    fn test_empty_sorted_bytes(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            for _ in 0..2 {
                let mut doc = Document::new();
                doc.add(Box::new(SortedDocValuesField::new("dv", nbr(""))));
                iwriter.add_document(&doc)?;
            }
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_sorted_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            aeq(0, dv.ord_value()?);
            aeq(1, dv.next_doc()?);
            aeq(0, dv.ord_value()?);
            let scratch = dv.lookup_ord(0)?;
            Assert::assert_equals(String::new(), scratch.utf8_to_string()?);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testEmptyBytes()`.
    fn test_empty_bytes(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            for _ in 0..2 {
                let mut doc = Document::new();
                doc.add(Box::new(BinaryDocValuesField::new("dv", nbr(""))));
                iwriter.add_document(&doc)?;
            }
            iwriter.force_merge(1)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_binary_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            Assert::assert_equals(String::new(), dv.binary_value()?.utf8_to_string()?);
            aeq(1, dv.next_doc()?);
            Assert::assert_equals(String::new(), dv.binary_value()?.utf8_to_string()?);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testVeryLargeButLegalBytes()`.
    fn test_very_large_but_legal_bytes(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            let mut bytes = vec![0u8; 32766];
            random().next_bytes(&mut bytes);
            let b = LuceneTestCase::new_bytes_ref_bytes(&bytes);
            doc.add(Box::new(BinaryDocValuesField::new("dv", b)));
            iwriter.add_document(&doc)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_binary_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            Assert::assert_equals(
                LuceneTestCase::new_bytes_ref_bytes(&bytes),
                dv.binary_value()?,
            );

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testVeryLargeButLegalSortedBytes()`.
    fn test_very_large_but_legal_sorted_bytes(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            let mut bytes = vec![0u8; 32766];
            random().next_bytes(&mut bytes);
            let b = LuceneTestCase::new_bytes_ref_bytes(&bytes);
            doc.add(Box::new(SortedDocValuesField::new("dv", b)));
            iwriter.add_document(&doc)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = DocValues::get_sorted(&*leaf, "dv")?;
            aeq(0, dv.next_doc()?);
            Assert::assert_equals(
                LuceneTestCase::new_bytes_ref_bytes(&bytes),
                dv.lookup_ord(dv.ord_value()?)?,
            );
            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testCodecUsesOwnBytes()`.
    fn test_codec_uses_own_bytes(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(BinaryDocValuesField::new("dv", nbr("boo!"))));
            iwriter.add_document(&doc)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = npe(leaf.get_binary_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            Assert::assert_equals("boo!".to_string(), dv.binary_value()?.utf8_to_string()?);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testCodecUsesOwnSortedBytes()`.
    fn test_codec_uses_own_sorted_bytes(&self) -> Result<()> {
        let (mut directory, iwriter) = log_mp_writer()?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(SortedDocValuesField::new("dv", nbr("boo!"))));
            iwriter.add_document(&doc)?;
            iwriter.close()?;
            drop(iwriter);

            // Now search the index:
            let ireader = open_reader(self, &directory)?; // read-only=true
            let leaf = only_leaf(&ireader)?;
            let mut dv = DocValues::get_sorted(&*leaf, "dv")?;
            let mybytes = [0u8; 20];
            aeq(0, dv.next_doc()?);
            Assert::assert_equals(
                "boo!".to_string(),
                dv.lookup_ord(dv.ord_value()?)?.utf8_to_string()?,
            );
            // `lookupOrd(...).bytes == mybytes`: an identity comparison.
            let looked_up = dv.lookup_ord(dv.ord_value()?)?;
            Assert::assert_false(std::ptr::eq(looked_up.bytes.as_ptr(), mybytes.as_ptr()));

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testDocValuesSimple()`: simple test case to show how to use the API.
    fn test_doc_values_simple(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let analyzer = mock_analyzer();
            let mut conf = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
            conf.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
            let writer = IndexWriter::new(dir.as_directory(), conf)?;
            for i in 0..5 {
                let mut doc = Document::new();
                doc.add(Box::new(NumericDocValuesField::new("docId", i)));
                doc.add(Box::new(TextField::new("docId", i.to_string(), Store::NO)?));
                writer.add_document(&doc)?;
            }
            writer.commit()?;
            writer.force_merge_with(1, true)?;

            writer.close()?;
            drop(writer);

            let reader =
                self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
            let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
            aeq(1, Arc::clone(&ir).leaves()?.len() as i64);

            let searcher = IndexSearcher::new(Arc::clone(&ir))?;

            let mut query = BooleanQueryBuilder::new();
            for id in ["0", "1", "2", "3", "4"] {
                query.add(term_query("docId", id), Occur::SHOULD)?;
            }

            let search = searcher.search(Arc::new(query.build()), 10)?;
            aeq(5, search.total_hits.value());
            let score_docs = &search.score_docs;
            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir))?;
            let mut doc_values = npe(leaf.get_numeric_doc_values("docId")?);
            for (i, score_doc) in score_docs.iter().enumerate() {
                let i = i as i32;
                aeq(i, score_doc.doc);
                aeq(i, doc_values.advance(i)?);
                aeq(i, doc_values.long_value()?);
            }
            drop((doc_values, leaf, searcher, ir));
            reader.close()?;
            drop(reader);
        }
        dir.close()
    }

    /// `testRandomSortedBytes()`.
    fn test_random_sorted_bytes(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let cfg = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
            let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), cfg)?;
            let num_docs = LuceneTestCase::at_least(100);
            let bytes_used = Arc::new(AtomicI64::new(0));
            let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
            let mut hash = BytesRefHash::new(bytes_used);
            let mut doc_to_string = JavaStringMap::new();
            let max_length = TestUtil::next_int(&*random(), 1, 50);
            for i in 0..num_docs {
                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "id",
                    &i.to_string(),
                    Store::YES,
                )));
                let string =
                    TestUtil::random_realistic_unicode_string_between(&*random(), 1, max_length);
                let br = nbr(&string);
                doc.add(Box::new(SortedDocValuesField::new("field", br.clone())));
                hash.add(&mut pool, br.slice())?;
                doc_to_string.put(i.to_string(), string);
                w.add_document(&doc)?;
            }
            if LuceneTestCase::rarely() {
                w.commit()?;
            }
            let num_docs_no_value = LuceneTestCase::at_least(10);
            for _ in 0..num_docs_no_value {
                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "id",
                    "noValue",
                    Store::YES,
                )));
                w.add_document(&doc)?;
            }
            if LuceneTestCase::rarely() {
                w.commit()?;
            }
            for i in 0..num_docs {
                let mut doc = Document::new();
                let id = (i + num_docs).to_string();
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "id",
                    &id,
                    Store::YES,
                )));
                let string =
                    TestUtil::random_realistic_unicode_string_between(&*random(), 1, max_length);
                let br = nbr(&string);
                hash.add(&mut pool, br.slice())?;
                doc_to_string.put(id, string);
                doc.add(Box::new(SortedDocValuesField::new("field", br)));
                w.add_document(&doc)?;
            }
            w.commit()?;
            let reader: Arc<dyn IndexReader> = w.get_reader()?;
            let doc_values = npe(MultiDocValues::get_sorted_values(&reader, "field")?);
            let size = hash.size();
            let sort: Vec<i32> = hash.sort(&pool).to_vec();
            let _expected = LuceneTestCase::new_bytes_ref();
            aeq(size, doc_values.get_value_count()?);
            for (i, &id) in sort.iter().enumerate().take(size as usize) {
                let expected = BytesRef::new(hash.get(&pool, id).to_vec());
                let actual = doc_values.lookup_ord(i as i32)?;
                Assert::assert_equals(expected.utf8_to_string()?, actual.utf8_to_string()?);
                let ord = doc_values.lookup_term(&expected)?;
                aeq(i as i32, ord);
            }

            for (key, value) in doc_to_string.entries() {
                // pk lookup
                let mut term_postings_enum = npe(TestUtil::docs(
                    &*random(),
                    &reader,
                    "id",
                    &nbr(&key),
                    None,
                    0,
                )?);
                let doc_id = term_postings_enum.next_doc()?;
                let expected = nbr(&value);
                let mut doc_values = npe(MultiDocValues::get_sorted_values(&reader, "field")?);
                aeq(doc_id, doc_values.advance(doc_id)?);
                let actual = doc_values.lookup_ord(doc_values.ord_value()?)?;
                Assert::assert_equals(expected, actual);
            }

            drop(doc_values);
            reader.close()?;
            drop(reader);
            w.close()?;
            drop(w);
        }
        dir.close()
    }

    /// `doTestNumericsVsStoredFields(density, longs)` (`private`).
    fn do_test_numerics_vs_stored_fields(
        &self,
        density: f64,
        longs: &mut dyn FnMut() -> i64,
    ) -> Result<()> {
        self.do_test_numerics_vs_stored_fields_min(density, longs, 256)
    }

    /// `doTestNumericsVsStoredFields(density, longs, minDocs)` (`private`).
    fn do_test_numerics_vs_stored_fields_min(
        &self,
        density: f64,
        longs: &mut dyn FnMut() -> i64,
        min_docs: i32,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
            let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
            let stored_field = LuceneTestCase::new_string_field("stored", "", Store::YES);

            // index some docs
            let num_docs = LuceneTestCase::at_least((f64::from(min_docs) * 1.172) as i32);
            // numDocs should be always > 256 so that in case of a codec that optimizes
            // for numbers of values <= 256, all storage layouts are tested
            java_assert_unchecked!(num_docs > 256);
            for i in 0..num_docs {
                if random().next_double() > density {
                    writer.add_document(&Document::new())?;
                    continue;
                }
                let value = longs();
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                doc.add(Box::new(copy_field(&stored_field, value.to_string())?));
                doc.add(Box::new(NumericDocValuesField::new("dv", value)));
                writer.add_document(&doc)?;
                if random().next_int_bound(31) == 0 {
                    writer.commit()?;
                }
            }

            // delete some docs
            let num_deletions = random().next_int_bound(num_docs / 10);
            for _ in 0..num_deletions {
                let id = random().next_int_bound(num_docs);
                writer.delete_documents(term("id", &id.to_string()))?;
            }

            // merge some segments and ensure that at least one of them has more than
            // max(256, minDocs) values
            writer.force_merge(num_docs / 256.max(min_docs))?;

            writer.close()?;
            drop(writer);
            // compare
            self.assert_dv_iterate(&dir.as_directory())?;
        }
        dir.close()
    }

    /// `assertDVIterate(dir)` (`protected`): asserts equality of stored value
    /// vs. DocValue by iterating DocValues one at a time.
    fn assert_dv_iterate(&self, dir: &Arc<dyn Directory>) -> Result<()> {
        let ir = self.maybe_wrap_with_merging_reader(directory_reader::open(Arc::clone(dir))?)?;
        let reader: Arc<dyn IndexReader> = Arc::clone(&ir) as Arc<dyn IndexReader>;
        TestUtil::check_reader(&reader)?;
        for context in Arc::clone(&reader).leaves()? {
            let r = context.leaf_reader();
            let mut doc_values = DocValues::get_numeric(&*r, "dv")?;
            doc_values.next_doc()?;
            let stored_fields = r.stored_fields()?;
            for i in 0..r.max_doc() {
                let stored_value = doc_get(&stored_fields.document(i)?, "stored");
                match stored_value {
                    None => Assert::assert_true(doc_values.doc_id() > i),
                    Some(stored_value) => {
                        aeq(i, doc_values.doc_id());
                        aeq(parse_long(&stored_value), doc_values.long_value()?);
                        doc_values.next_doc()?;
                    }
                }
            }
            aeq(NO_MORE_DOCS, doc_values.doc_id());
        }
        drop(reader);
        ir.close()
    }

    /// `compareStoredFieldWithSortedNumericsDV(directoryReader, storedField,
    /// dvField)` (`protected`).
    fn compare_stored_field_with_sorted_numerics_dv(
        &self,
        directory_reader: &Arc<dyn DirectoryReader>,
        stored_field: &str,
        dv_field: &str,
    ) -> Result<()> {
        let ir: Arc<dyn IndexReader> = Arc::clone(directory_reader) as Arc<dyn IndexReader>;
        for leaf in ir.leaves()? {
            let reader = leaf.leaf_reader();
            let stored_fields = reader.stored_fields()?;
            let values_of = |doc: i32| -> Result<Vec<String>> {
                Ok(stored_fields.document(doc)?.get_values(stored_field))
            };
            let Some(mut doc_values) = reader.get_sorted_numeric_doc_values(dv_field)? else {
                // no stored values at all
                for doc in 0..reader.max_doc() {
                    assert_array_equals::<String>(&[], &values_of(doc)?);
                }
                continue;
            };
            for doc in 0..reader.max_doc() {
                let stored_values = values_of(doc)?;
                if stored_values.is_empty() {
                    Assert::assert_false(doc_values.advance_exact(doc)?);
                    continue;
                }
                match random().next_int_bound(3) {
                    0 => aeq(doc, doc_values.next_doc()?),
                    1 => aeq(doc, doc_values.advance(doc)?),
                    _ => Assert::assert_true(doc_values.advance_exact(doc)?),
                }
                aeq(doc, doc_values.doc_id());
                check_sorted_numeric_repeats(&mut *doc_values, doc, &stored_values)?;
            }
            // jump with advanceExact
            let iters = 1 + random().next_int_bound(3);
            for _ in 0..iters {
                let mut doc_values = npe(reader.get_sorted_numeric_doc_values(dv_field)?);
                let mut doc = random().next_int_bound(leaf.leaf_reader().max_doc());
                while doc < reader.max_doc() {
                    let stored_values = values_of(doc)?;
                    if doc_values.advance_exact(doc)? {
                        aeq(doc, doc_values.doc_id());
                        check_sorted_numeric_repeats(&mut *doc_values, doc, &stored_values)?;
                    } else {
                        assert_array_equals::<String>(&[], &stored_values);
                    }
                    doc += random().next_int_bound(5); // skip some docs
                    doc += 1;
                }
            }
            // jump with advance
            for _ in 0..iters {
                let mut doc_values = npe(reader.get_sorted_numeric_doc_values(dv_field)?);
                let mut doc = random().next_int_bound(leaf.leaf_reader().max_doc());
                while doc != NO_MORE_DOCS {
                    let next_doc = doc_values.advance(doc)?;
                    // no stored fields in between
                    let end = if next_doc == NO_MORE_DOCS {
                        reader.max_doc()
                    } else {
                        next_doc
                    };
                    for d in doc..end {
                        let stored_values = values_of(d)?;
                        assert_array_equals::<String>(&[], &stored_values);
                    }
                    doc = next_doc;
                    if doc != NO_MORE_DOCS {
                        let stored_values = values_of(doc)?;
                        check_sorted_numeric_repeats(&mut *doc_values, doc, &stored_values)?;
                        doc = next_doc + 1;
                        doc += random().next_int_bound(5); // skip some docs
                    }
                }
            }
        }
        Ok(())
    }

    /// `doTestSortedNumericsVsStoredFields(counts, values)` (`private`).
    fn do_test_sorted_numerics_vs_stored_fields(
        &self,
        counts: &mut dyn FnMut() -> i64,
        values: &mut dyn FnMut() -> i64,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
            let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;

            // index some docs
            let num_docs = LuceneTestCase::at_least(300);
            // numDocs should be always > 256 so that in case of a codec that optimizes
            // for numbers of values <= 256, all storage layouts are tested
            java_assert_unchecked!(num_docs > 256);
            for i in 0..num_docs {
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));

                let value_count = counts() as i32;
                let mut value_array = Vec::with_capacity(value_count.max(0) as usize);
                for _ in 0..value_count {
                    let value = values();
                    value_array.push(value);
                    doc.add(Box::new(SortedNumericDocValuesField::new("dv", value)));
                }
                value_array.sort_unstable();
                for value in &value_array {
                    doc.add(Box::new(StoredField::new_string(
                        "stored",
                        value.to_string(),
                    )?));
                }
                writer.add_document(&doc)?;
                if random().next_int_bound(31) == 0 {
                    writer.commit()?;
                }
            }

            // delete some docs
            let num_deletions = random().next_int_bound(num_docs / 10);
            for _ in 0..num_deletions {
                let id = random().next_int_bound(num_docs);
                writer.delete_documents(term("id", &id.to_string()))?;
            }

            writer.commit()?;

            {
                let reader = self
                    .maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
                let checked =
                    TestUtil::check_reader(&(Arc::clone(&reader) as Arc<dyn IndexReader>))
                        .and_then(|_| {
                            self.compare_stored_field_with_sorted_numerics_dv(
                                &reader, "stored", "dv",
                            )
                        });
                let closed = reader.close();
                checked?;
                closed?;
            }
            // merge some segments and ensure that at least one of them has more than
            // 256 values
            writer.force_merge(num_docs / 256)?;
            {
                let reader = self
                    .maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
                let checked =
                    TestUtil::check_reader(&(Arc::clone(&reader) as Arc<dyn IndexReader>))
                        .and_then(|_| {
                            self.compare_stored_field_with_sorted_numerics_dv(
                                &reader, "stored", "dv",
                            )
                        });
                let closed = reader.close();
                checked?;
                closed?;
            }
            // `IOUtils.close(writer, dir)`
            writer.close()?;
            drop(writer);
        }
        dir.close()
    }

    /// The body of the `test*NumericsVsStoredFields` tests: `atLeast(1)`
    /// runs of `doTestNumericsVsStoredFields`, the density drawn per run when
    /// `sparse`.
    fn numerics_vs_stored_fields_test(&self, sparse: bool, longs: fn() -> i64) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let density = if sparse { random().next_double() } else { 1.0 };
            let mut longs = longs;
            self.do_test_numerics_vs_stored_fields(density, &mut longs)?;
        }
        Ok(())
    }

    /// `testBooleanNumericsVsStoredFields()`.
    fn test_boolean_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(false, || i64::from(random().next_int_bound(2)))
    }

    /// `testSparseBooleanNumericsVsStoredFields()`.
    fn test_sparse_boolean_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(true, || i64::from(random().next_int_bound(2)))
    }

    /// `testByteNumericsVsStoredFields()`.
    fn test_byte_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(false, || {
            i64::from(TestUtil::next_int(
                &*random(),
                i32::from(i8::MIN),
                i32::from(i8::MAX),
            ))
        })
    }

    /// `testSparseByteNumericsVsStoredFields()`.
    fn test_sparse_byte_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(true, || {
            i64::from(TestUtil::next_int(
                &*random(),
                i32::from(i8::MIN),
                i32::from(i8::MAX),
            ))
        })
    }

    /// `testShortNumericsVsStoredFields()`.
    fn test_short_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(false, || {
            i64::from(TestUtil::next_int(
                &*random(),
                i32::from(i16::MIN),
                i32::from(i16::MAX),
            ))
        })
    }

    /// `testSparseShortNumericsVsStoredFields()`.
    fn test_sparse_short_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(true, || {
            i64::from(TestUtil::next_int(
                &*random(),
                i32::from(i16::MIN),
                i32::from(i16::MAX),
            ))
        })
    }

    /// `testIntNumericsVsStoredFields()`.
    fn test_int_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(false, || i64::from(random().next_int()))
    }

    /// `testSparseIntNumericsVsStoredFields()`.
    fn test_sparse_int_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(true, || i64::from(random().next_int()))
    }

    /// `testLongNumericsVsStoredFields()`.
    fn test_long_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(false, || random().next_long())
    }

    /// `testSparseLongNumericsVsStoredFields()`.
    fn test_sparse_long_numerics_vs_stored_fields(&self) -> Result<()> {
        self.numerics_vs_stored_fields_test(true, || random().next_long())
    }

    /// `doTestBinaryVsStoredFields(density, bytes)` (`private`).
    fn do_test_binary_vs_stored_fields(
        &self,
        density: f64,
        bytes: &mut dyn FnMut() -> Vec<u8>,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
            let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
            // `new BinaryDocValuesField("dv", newBytesRef())`
            let _ = LuceneTestCase::new_bytes_ref();

            // index some docs
            let num_docs = LuceneTestCase::at_least(300);
            for i in 0..num_docs {
                if random().next_double() > density {
                    writer.add_document(&Document::new())?;
                    continue;
                }
                let buffer = bytes();
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                doc.add(Box::new(StoredField::new_bytes(
                    "stored",
                    BytesRef::new(buffer.clone()),
                )?));
                doc.add(Box::new(BinaryDocValuesField::new(
                    "dv",
                    BytesRef::new(buffer),
                )));
                writer.add_document(&doc)?;
                if random().next_int_bound(31) == 0 {
                    writer.commit()?;
                }
            }

            // delete some docs
            let num_deletions = random().next_int_bound(num_docs / 10);
            for _ in 0..num_deletions {
                let id = random().next_int_bound(num_docs);
                writer.delete_documents(term("id", &id.to_string()))?;
            }

            // compare
            for merge in [false, true] {
                if merge {
                    // compare again
                    writer.force_merge(1)?;
                }
                let ir: Arc<dyn IndexReader> = writer.get_reader()?;
                TestUtil::check_reader(&ir)?;
                for context in Arc::clone(&ir).leaves()? {
                    let r = context.leaf_reader();
                    let stored_fields = r.stored_fields()?;
                    let mut doc_values = DocValues::get_binary(&*r, "dv")?;
                    doc_values.next_doc()?;
                    for i in 0..r.max_doc() {
                        let binary_value = stored_fields.document(i)?.get_binary_value("stored");
                        match binary_value {
                            None => Assert::assert_true(doc_values.doc_id() > i),
                            Some(binary_value) => {
                                aeq(i, doc_values.doc_id());
                                Assert::assert_equals(binary_value, doc_values.binary_value()?);
                                doc_values.next_doc()?;
                            }
                        }
                    }
                    aeq(NO_MORE_DOCS, doc_values.doc_id());
                }
                ir.close()?;
            }
            writer.close()?;
            drop(writer);
        }
        dir.close()
    }

    /// `testBinaryFixedLengthVsStoredFields()`.
    fn test_binary_fixed_length_vs_stored_fields(&self) -> Result<()> {
        self.do_test_binary_fixed_length_vs_stored_fields(1.0)
    }

    /// `testSparseBinaryFixedLengthVsStoredFields()`.
    fn test_sparse_binary_fixed_length_vs_stored_fields(&self) -> Result<()> {
        self.do_test_binary_fixed_length_vs_stored_fields(random().next_double())
    }

    /// `doTestBinaryFixedLengthVsStoredFields(density)` (`private`).
    fn do_test_binary_fixed_length_vs_stored_fields(&self, density: f64) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let fixed_length = TestUtil::next_int(&*random(), 0, 10) as usize;
            self.do_test_binary_vs_stored_fields(density, &mut || {
                let mut buffer = vec![0u8; fixed_length];
                random().next_bytes(&mut buffer);
                buffer
            })?;
        }
        Ok(())
    }

    /// `testBinaryVariableLengthVsStoredFields()`.
    fn test_binary_variable_length_vs_stored_fields(&self) -> Result<()> {
        self.do_test_binary_variable_length_vs_stored_fields(1.0)
    }

    /// `testSparseBinaryVariableLengthVsStoredFields()`.
    fn test_sparse_binary_variable_length_vs_stored_fields(&self) -> Result<()> {
        self.do_test_binary_variable_length_vs_stored_fields(random().next_double())
    }

    /// `doTestBinaryVariableLengthVsStoredFields(density)` (`public`).
    fn do_test_binary_variable_length_vs_stored_fields(&self, density: f64) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            self.do_test_binary_vs_stored_fields(density, &mut || {
                let length = random().next_int_bound(10) as usize;
                let mut buffer = vec![0u8; length];
                random().next_bytes(&mut buffer);
                buffer
            })?;
        }
        Ok(())
    }

    /// `doTestSortedVsStoredFields(numDocs, density, bytes)` (`protected`).
    fn do_test_sorted_vs_stored_fields(
        &self,
        num_docs: i32,
        density: f64,
        bytes: &mut dyn FnMut() -> Vec<u8>,
    ) -> Result<()> {
        let mut dir =
            LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("dvduel"));
        let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
        let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
        // `new SortedDocValuesField("dv", newBytesRef())`
        let _ = LuceneTestCase::new_bytes_ref();

        // index some docs
        for i in 0..num_docs {
            if random().next_double() > density {
                writer.add_document(&Document::new())?;
                continue;
            }
            let buffer = bytes();
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
            doc.add(Box::new(StoredField::new_bytes(
                "stored",
                BytesRef::new(buffer.clone()),
            )?));
            doc.add(Box::new(SortedDocValuesField::new(
                "dv",
                BytesRef::new(buffer),
            )));
            writer.add_document(&doc)?;
            if random().next_int_bound(31) == 0 {
                writer.commit()?;
            }
        }

        // delete some docs
        let num_deletions = random().next_int_bound(num_docs / 10);
        for _ in 0..num_deletions {
            let id = random().next_int_bound(num_docs);
            writer.delete_documents(term("id", &id.to_string()))?;
        }

        // compare
        for merge in [false, true] {
            if merge {
                writer.force_merge(1)?;
                // compare again
            }
            let ir: Arc<dyn IndexReader> = writer.get_reader()?;
            TestUtil::check_reader(&ir)?;
            for context in Arc::clone(&ir).leaves()? {
                let r = context.leaf_reader();
                let stored_fields = r.stored_fields()?;
                let mut doc_values = DocValues::get_sorted(&*r, "dv")?;
                doc_values.next_doc()?;
                for i in 0..r.max_doc() {
                    let binary_value = stored_fields.document(i)?.get_binary_value("stored");
                    match binary_value {
                        None => Assert::assert_true(doc_values.doc_id() > i),
                        Some(binary_value) => {
                            aeq(i, doc_values.doc_id());
                            Assert::assert_equals(
                                binary_value,
                                doc_values.lookup_ord(doc_values.ord_value()?)?,
                            );
                            doc_values.next_doc()?;
                        }
                    }
                }
                aeq(NO_MORE_DOCS, doc_values.doc_id());
            }
            ir.close()?;
        }
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testSortedFixedLengthVsStoredFields()`.
    fn test_sorted_fixed_length_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let fixed_length = TestUtil::next_int(&*random(), 1, 10);
            let num_docs = LuceneTestCase::at_least(300);
            self.do_test_sorted_vs_stored_fields_lengths(
                num_docs,
                1.0,
                fixed_length,
                fixed_length,
            )?;
        }
        Ok(())
    }

    /// `testSparseSortedFixedLengthVsStoredFields()`.
    fn test_sparse_sorted_fixed_length_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let fixed_length = TestUtil::next_int(&*random(), 1, 10);
            let num_docs = LuceneTestCase::at_least(300);
            let density = random().next_double();
            self.do_test_sorted_vs_stored_fields_lengths(
                num_docs,
                density,
                fixed_length,
                fixed_length,
            )?;
        }
        Ok(())
    }

    /// `testSortedVariableLengthVsStoredFields()`.
    fn test_sorted_variable_length_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let num_docs = LuceneTestCase::at_least(300);
            self.do_test_sorted_vs_stored_fields_lengths(num_docs, 1.0, 1, 10)?;
        }
        Ok(())
    }

    /// `testSparseSortedVariableLengthVsStoredFields()`.
    fn test_sparse_sorted_variable_length_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let num_docs = LuceneTestCase::at_least(300);
            let density = random().next_double();
            self.do_test_sorted_vs_stored_fields_lengths(num_docs, density, 1, 10)?;
        }
        Ok(())
    }

    /// `doTestSortedVsStoredFields(numDocs, density, minLength, maxLength)`
    /// (`protected`).
    fn do_test_sorted_vs_stored_fields_lengths(
        &self,
        num_docs: i32,
        density: f64,
        min_length: i32,
        max_length: i32,
    ) -> Result<()> {
        self.do_test_sorted_vs_stored_fields(num_docs, density, &mut || {
            let length = TestUtil::next_int(&*random(), min_length, max_length) as usize;
            let mut buffer = vec![0u8; length];
            random().next_bytes(&mut buffer);
            buffer
        })
    }

    /// `testSortedSetOneValue()`.
    fn test_sorted_set_one_value(&self) -> Result<()> {
        let (mut directory, iwriter) = sorted_set_writer(false)?;
        {
            let iwriter = iwriter;
            iwriter.add_document(&sorted_set_doc("field", &["hello"]))?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
            aeq(0, dv.next_doc()?);

            aeq(1, dv.doc_value_count()?);
            aeq(0, dv.next_ord()?);

            let bytes = dv.lookup_ord(0)?;
            Assert::assert_equals(nbr("hello"), bytes);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedSetTwoFields()`.
    fn test_sorted_set_two_fields(&self) -> Result<()> {
        let (mut directory, iwriter) = sorted_set_writer(false)?;
        {
            let iwriter = iwriter;
            let mut doc = Document::new();
            doc.add(Box::new(SortedSetDocValuesField::new(
                "field",
                nbr("hello"),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "field2",
                nbr("world"),
            )));
            iwriter.add_document(&doc)?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
            aeq(0, dv.next_doc()?);

            aeq(1, dv.doc_value_count()?);
            aeq(0, dv.next_ord()?);

            let bytes = dv.lookup_ord(0)?;
            Assert::assert_equals(nbr("hello"), bytes);

            let mut dv = npe(leaf.get_sorted_set_doc_values("field2")?);
            aeq(0, dv.next_doc()?);

            aeq(1, dv.doc_value_count()?);
            aeq(0, dv.next_ord()?);

            let bytes = dv.lookup_ord(0)?;
            Assert::assert_equals(nbr("world"), bytes);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedSetTwoDocumentsMerged()`.
    fn test_sorted_set_two_documents_merged(&self) -> Result<()> {
        let (mut directory, iwriter) = sorted_set_writer(true)?;
        {
            let iwriter = iwriter;
            iwriter.add_document(&sorted_set_doc("field", &["hello"]))?;
            iwriter.commit()?;

            iwriter.add_document(&sorted_set_doc("field", &["world"]))?;
            iwriter.force_merge(1)?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
            aeq(2, dv.get_value_count()?);

            aeq(0, dv.next_doc()?);
            aeq(1, dv.doc_value_count()?);
            aeq(0, dv.next_ord()?);

            let bytes = dv.lookup_ord(0)?;
            Assert::assert_equals(nbr("hello"), bytes);

            aeq(1, dv.next_doc()?);
            aeq(1, dv.doc_value_count()?);
            aeq(1, dv.next_ord()?);

            let bytes = dv.lookup_ord(1)?;
            Assert::assert_equals(nbr("world"), bytes);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedSetTwoValues()`.
    fn test_sorted_set_two_values(&self) -> Result<()> {
        sorted_set_two_values(&["hello", "world"])
    }

    /// `testSortedSetTwoValuesUnordered()`.
    fn test_sorted_set_two_values_unordered(&self) -> Result<()> {
        sorted_set_two_values(&["world", "hello"])
    }

    /// `testSortedSetThreeValuesTwoDocs()`.
    fn test_sorted_set_three_values_two_docs(&self) -> Result<()> {
        let (mut directory, iwriter) = sorted_set_writer(true)?;
        {
            let iwriter = iwriter;
            iwriter.add_document(&sorted_set_doc("field", &["hello", "world"]))?;
            iwriter.commit()?;

            iwriter.add_document(&sorted_set_doc("field", &["hello", "beer"]))?;
            iwriter.force_merge(1)?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
            aeq(3, dv.get_value_count()?);

            aeq(0, dv.next_doc()?);
            aeq(2, dv.doc_value_count()?);
            aeq(1, dv.next_ord()?);
            aeq(2, dv.next_ord()?);

            aeq(1, dv.next_doc()?);
            aeq(2, dv.doc_value_count()?);
            aeq(0, dv.next_ord()?);
            aeq(1, dv.next_ord()?);

            let bytes = dv.lookup_ord(0)?;
            Assert::assert_equals(nbr("beer"), bytes);

            let bytes = dv.lookup_ord(1)?;
            Assert::assert_equals(nbr("hello"), bytes);

            let bytes = dv.lookup_ord(2)?;
            Assert::assert_equals(nbr("world"), bytes);

            drop((dv, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedSetTwoDocumentsLastMissing()`.
    fn test_sorted_set_two_documents_last_missing(&self) -> Result<()> {
        sorted_set_one_missing(true, false)
    }

    /// `testSortedSetTwoDocumentsLastMissingMerge()`.
    fn test_sorted_set_two_documents_last_missing_merge(&self) -> Result<()> {
        sorted_set_one_missing(true, true)
    }

    /// `testSortedSetTwoDocumentsFirstMissing()`.
    fn test_sorted_set_two_documents_first_missing(&self) -> Result<()> {
        sorted_set_one_missing(false, false)
    }

    /// `testSortedSetTwoDocumentsFirstMissingMerge()`.
    fn test_sorted_set_two_documents_first_missing_merge(&self) -> Result<()> {
        sorted_set_one_missing(false, true)
    }

    /// `testSortedSetMergeAwayAllValues()`.
    fn test_sorted_set_merge_away_all_values(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(SortedSetDocValuesField::new("field", nbr("hello")))),
            |leaf| {
                let dv = npe(leaf.get_sorted_set_doc_values("field")?);
                aeq(0, dv.get_value_count()?);

                let mut terms_enum = dv.terms_enum()?;
                Assert::assert_false(
                    terms_enum.seek_exact(&BytesRef::from_char_sequence("lucene"))?,
                );
                assert_seek_status(
                    SeekStatus::END,
                    terms_enum.seek_ceil(&BytesRef::from_char_sequence("lucene"))?,
                );
                let dv = npe(leaf.get_sorted_set_doc_values("field")?);
                aeq(-1, dv.lookup_term(&BytesRef::from_char_sequence("lucene"))?);
                Ok(())
            },
        )
    }

    /// `testSortedSetTermsEnum()`.
    fn test_sorted_set_terms_enum(&self) -> Result<()> {
        let (mut directory, iwriter) = sorted_set_writer(true)?;
        {
            let iwriter = iwriter;
            iwriter.add_document(&sorted_set_doc("field", &["hello", "world", "beer"]))?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let dv = npe(leaf.get_sorted_set_doc_values("field")?);
            aeq(3, dv.get_value_count()?);

            let mut terms_enum = dv.terms_enum()?;

            // next()
            Assert::assert_equals(
                "beer".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(0, terms_enum.ord()?);
            Assert::assert_equals(
                "hello".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(1, terms_enum.ord()?);
            Assert::assert_equals(
                "world".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(2, terms_enum.ord()?);

            // seekCeil()
            assert_seek_status(SeekStatus::NOT_FOUND, terms_enum.seek_ceil(&nbr("ha!"))?);
            Assert::assert_equals("hello".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(1, terms_enum.ord()?);
            assert_seek_status(SeekStatus::FOUND, terms_enum.seek_ceil(&nbr("beer"))?);
            Assert::assert_equals("beer".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(0, terms_enum.ord()?);
            assert_seek_status(SeekStatus::END, terms_enum.seek_ceil(&nbr("zzz"))?);

            // seekExact()
            Assert::assert_true(terms_enum.seek_exact(&nbr("beer"))?);
            Assert::assert_equals("beer".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(0, terms_enum.ord()?);
            Assert::assert_true(terms_enum.seek_exact(&nbr("hello"))?);
            Assert::assert_equals("hello".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(1, terms_enum.ord()?);
            Assert::assert_true(terms_enum.seek_exact(&nbr("world"))?);
            Assert::assert_equals("world".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(2, terms_enum.ord()?);
            Assert::assert_false(terms_enum.seek_exact(&nbr("bogus"))?);

            // seek(ord)
            terms_enum.seek_ord(0)?;
            Assert::assert_equals("beer".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(0, terms_enum.ord()?);
            terms_enum.seek_ord(1)?;
            Assert::assert_equals("hello".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(1, terms_enum.ord()?);
            terms_enum.seek_ord(2)?;
            Assert::assert_equals("world".to_string(), terms_enum.term()?.utf8_to_string()?);
            aeq(2, terms_enum.ord()?);

            // NORMAL automaton
            let dv = npe(leaf.get_sorted_set_doc_values("field")?);
            let mut terms_enum = dv.intersect(&compiled(".*l.*", true)?)?;
            Assert::assert_equals(
                "hello".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(1, terms_enum.ord()?);
            Assert::assert_equals(
                "world".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(2, terms_enum.ord()?);
            assert_null(terms_enum.next()?);

            // SINGLE automaton
            let dv = npe(leaf.get_sorted_set_doc_values("field")?);
            let mut terms_enum = dv.intersect(&compiled("hello", false)?)?;
            Assert::assert_equals(
                "hello".to_string(),
                npe(terms_enum.next()?).utf8_to_string()?,
            );
            aeq(1, terms_enum.ord()?);
            assert_null(terms_enum.next()?);

            drop((terms_enum, leaf));
            ireader.close()?;
        }
        directory.close()
    }

    /// `compareStoredFieldWithSortedSetDV(directoryReader, storedField,
    /// dvField)` (`protected`).
    fn compare_stored_field_with_sorted_set_dv(
        &self,
        directory_reader: &Arc<dyn DirectoryReader>,
        stored_field: &str,
        dv_field: &str,
    ) -> Result<()> {
        let ir: Arc<dyn IndexReader> = Arc::clone(directory_reader) as Arc<dyn IndexReader>;
        for leaf in ir.leaves()? {
            let reader = leaf.leaf_reader();
            let stored_fields = reader.stored_fields()?;
            let values_of = |doc: i32| -> Result<Vec<String>> {
                Ok(stored_fields.document(doc)?.get_values(stored_field))
            };
            let Some(mut doc_values) = reader.get_sorted_set_doc_values(dv_field)? else {
                // no stored values at all
                for doc in 0..reader.max_doc() {
                    assert_array_equals::<String>(&[], &values_of(doc)?);
                }
                continue;
            };
            // sequentially
            for doc in 0..reader.max_doc() {
                let stored_values = values_of(doc)?;
                if stored_values.is_empty() {
                    Assert::assert_false(doc_values.advance_exact(doc)?);
                    continue;
                }
                match random().next_int_bound(3) {
                    0 => aeq(doc, doc_values.next_doc()?),
                    1 => aeq(doc, doc_values.advance(doc)?),
                    _ => Assert::assert_true(doc_values.advance_exact(doc)?),
                }
                aeq(doc, doc_values.doc_id());
                aeq(stored_values.len() as i64, doc_values.doc_value_count()?);
                check_sorted_set_repeats(&mut *doc_values, doc, &stored_values)?;
            }
            // jump with advanceExact
            let iters = 1 + random().next_int_bound(3);
            for _ in 0..iters {
                let mut doc_values = npe(reader.get_sorted_set_doc_values(dv_field)?);
                let mut doc = random().next_int_bound(leaf.leaf_reader().max_doc());
                while doc < reader.max_doc() {
                    let stored_values = values_of(doc)?;
                    if doc_values.advance_exact(doc)? {
                        aeq(doc, doc_values.doc_id());
                        aeq(stored_values.len() as i64, doc_values.doc_value_count()?);
                        check_sorted_set_repeats(&mut *doc_values, doc, &stored_values)?;
                    } else {
                        assert_array_equals::<String>(&[], &stored_values);
                    }
                    doc += random().next_int_bound(5); // skip some docs
                    doc += 1;
                }
            }
            // jump with advance
            for _ in 0..iters {
                let mut doc_values = npe(reader.get_sorted_set_doc_values(dv_field)?);
                let mut doc = random().next_int_bound(leaf.leaf_reader().max_doc());
                while doc != NO_MORE_DOCS {
                    let next_doc = doc_values.advance(doc)?;
                    // no stored fields in between
                    let end = if next_doc == NO_MORE_DOCS {
                        reader.max_doc()
                    } else {
                        next_doc
                    };
                    for d in doc..end {
                        let stored_values = values_of(d)?;
                        assert_array_equals::<String>(&[], &stored_values);
                    }
                    doc = next_doc;
                    if doc != NO_MORE_DOCS {
                        let repeats = 1 + random().next_int_bound(3);
                        let stored_values = values_of(doc)?;
                        for r in 0..repeats {
                            if r > 0 || random().next_boolean() {
                                Assert::assert_true(doc_values.advance_exact(doc)?);
                            }
                            for v in 0..doc_values.doc_value_count()? {
                                let ord = doc_values.next_ord()?;
                                Assert::assert_equals_str(
                                    None,
                                    stored_values.get(v as usize).map(String::as_str),
                                    Some(&doc_values.lookup_ord(ord)?.utf8_to_string()?),
                                );
                            }
                        }
                        doc = next_doc + 1;
                        doc += random().next_int_bound(5); // skip some docs
                    }
                }
            }
        }
        Ok(())
    }

    /// `doTestSortedSetVsStoredFields(numDocs, minLength, maxLength,
    /// maxValuesPerDoc, maxUniqueValues)` (`protected`).
    fn do_test_sorted_set_vs_stored_fields(
        &self,
        num_docs: i32,
        min_length: i32,
        max_length: i32,
        max_values_per_doc: i32,
        max_unique_values: i32,
    ) -> Result<()> {
        let mut dir =
            LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("dvduel"));
        let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
        let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;

        let mut value_set = JavaStringMap::new();
        let mut i = 0;
        while i < 10000 && (value_set.len() as i32) < max_unique_values {
            let length = TestUtil::next_int(&*random(), min_length, max_length);
            value_set.put(
                TestUtil::random_simple_string_max(&*random(), length),
                String::new(),
            );
            i += 1;
        }
        let unique_values: Vec<String> = value_set.keys();

        // index some docs
        if *VERBOSE {
            println!("\nTEST: now add numDocs={num_docs}");
        }
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
            let num_values = TestUtil::next_int(&*random(), 0, max_values_per_doc);
            // create a random set of strings
            let mut values: BTreeSet<String> = BTreeSet::new();
            for _ in 0..num_values {
                values.insert(RandomPicks::random_from(&*random(), &unique_values).clone());
            }

            // add ordered to the stored field
            for v in &values {
                doc.add(Box::new(StoredField::new_string("stored", v.clone())?));
            }

            // add in any order to the dv field
            let mut unordered: Vec<String> = values.into_iter().collect();
            randomizedtesting::jdk::util::shuffle(&mut unordered, &*random());
            for v in &unordered {
                doc.add(Box::new(SortedSetDocValuesField::new("dv", nbr(v))));
            }

            writer.add_document(&doc)?;
            if random().next_int_bound(31) == 0 {
                writer.commit()?;
            }
        }
        // delete some docs
        let num_deletions = random().next_int_bound(num_docs / 10);
        for _ in 0..num_deletions {
            let id = random().next_int_bound(num_docs);
            writer.delete_documents(term("id", &id.to_string()))?;
        }

        for merge in [false, true] {
            if merge {
                writer.force_merge(1)?;
            }
            let reader = writer.get_reader()?;
            let checked = TestUtil::check_reader(&(Arc::clone(&reader) as Arc<dyn IndexReader>))
                .and_then(|_| {
                    self.compare_stored_field_with_sorted_set_dv(&reader, "stored", "dv")
                });
            let closed = reader.close();
            checked?;
            closed?;
        }
        // `IOUtils.close(writer, dir)`
        writer.close()?;
        drop(writer);
        dir.close()
    }

    /// `testSortedSetFixedLengthVsStoredFields()`.
    fn test_sorted_set_fixed_length_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let fixed_length = TestUtil::next_int(&*random(), 1, 10);
            let num_docs = LuceneTestCase::at_least(300);
            self.do_test_sorted_set_vs_stored_fields(
                num_docs,
                fixed_length,
                fixed_length,
                16,
                100,
            )?;
        }
        Ok(())
    }

    /// `testSortedNumericsSingleValuedVsStoredFields()`.
    fn test_sorted_numerics_single_valued_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            self.do_test_sorted_numerics_vs_stored_fields(&mut || 1, &mut || random().next_long())?;
        }
        Ok(())
    }

    /// `testSortedNumericsSingleValuedMissingVsStoredFields()`.
    fn test_sorted_numerics_single_valued_missing_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            self.do_test_sorted_numerics_vs_stored_fields(
                &mut || if random().next_boolean() { 0 } else { 1 },
                &mut || random().next_long(),
            )?;
        }
        Ok(())
    }

    /// `testSortedNumericsMultipleValuesVsStoredFields()`.
    fn test_sorted_numerics_multiple_values_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            self.do_test_sorted_numerics_vs_stored_fields(
                &mut || TestUtil::next_long(&*random(), 0, 50),
                &mut || random().next_long(),
            )?;
        }
        Ok(())
    }

    /// `testSortedNumericsFewUniqueSetsVsStoredFields()`.
    fn test_sorted_numerics_few_unique_sets_vs_stored_fields(&self) -> Result<()> {
        let values: Vec<i64> = (0..TestUtil::next_int(&*random(), 2, 6))
            .map(|_| random().next_long())
            .collect();
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            self.do_test_sorted_numerics_vs_stored_fields(
                &mut || TestUtil::next_long(&*random(), 0, 6),
                &mut || values[random().next_int_bound(values.len() as i32) as usize],
            )?;
        }
        Ok(())
    }

    /// `testSortedSetVariableLengthVsStoredFields()`.
    fn test_sorted_set_variable_length_vs_stored_fields(&self) -> Result<()> {
        sorted_set_vs_stored_fields_test(self, 300, 1, 10, 16, 100)
    }

    /// `testSortedSetFixedLengthSingleValuedVsStoredFields()`.
    fn test_sorted_set_fixed_length_single_valued_vs_stored_fields(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let fixed_length = TestUtil::next_int(&*random(), 1, 10);
            let num_docs = LuceneTestCase::at_least(300);
            self.do_test_sorted_set_vs_stored_fields(num_docs, fixed_length, fixed_length, 1, 100)?;
        }
        Ok(())
    }

    /// `testSortedSetVariableLengthSingleValuedVsStoredFields()`.
    fn test_sorted_set_variable_length_single_valued_vs_stored_fields(&self) -> Result<()> {
        sorted_set_vs_stored_fields_test(self, 300, 1, 10, 1, 100)
    }

    /// `testSortedSetFixedLengthFewUniqueSetsVsStoredFields()`.
    fn test_sorted_set_fixed_length_few_unique_sets_vs_stored_fields(&self) -> Result<()> {
        sorted_set_vs_stored_fields_test(self, 300, 10, 10, 6, 6)
    }

    /// `testSortedSetVariableLengthFewUniqueSetsVsStoredFields()`.
    fn test_sorted_set_variable_length_few_unique_sets_vs_stored_fields(&self) -> Result<()> {
        sorted_set_vs_stored_fields_test(self, 300, 1, 10, 6, 6)
    }

    /// `testSortedSetVariableLengthManyValuesPerDocVsStoredFields()`.
    fn test_sorted_set_variable_length_many_values_per_doc_vs_stored_fields(&self) -> Result<()> {
        sorted_set_vs_stored_fields_test(self, 20, 1, 10, 500, 1000)
    }

    /// `testSortedSetFixedLengthManyValuesPerDocVsStoredFields()`.
    fn test_sorted_set_fixed_length_many_values_per_doc_vs_stored_fields(&self) -> Result<()> {
        sorted_set_vs_stored_fields_test(self, 20, 10, 10, 500, 1000)
    }

    /// `testGCDCompression()`.
    fn test_gcd_compression(&self) -> Result<()> {
        self.do_test_gcd_compression(1.0)
    }

    /// `testSparseGCDCompression()`.
    fn test_sparse_gcd_compression(&self) -> Result<()> {
        self.do_test_gcd_compression(random().next_double())
    }

    /// `doTestGCDCompression(density)` (`private`).
    fn do_test_gcd_compression(&self, density: f64) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let min = -((i64::from(random().next_int_bound(1 << 30))) << 32);
            let mul = i64::from(random().next_int()) & 0xFFFF_FFFF;
            let mut longs =
                || min.wrapping_add(mul.wrapping_mul(i64::from(random().next_int_bound(1 << 20))));
            self.do_test_numerics_vs_stored_fields(density, &mut longs)?;
        }
        Ok(())
    }

    /// `testZeros()`.
    fn test_zeros(&self) -> Result<()> {
        self.do_test_numerics_vs_stored_fields(1.0, &mut || 0)
    }

    /// `testSparseZeros()`.
    fn test_sparse_zeros(&self) -> Result<()> {
        self.do_test_numerics_vs_stored_fields(random().next_double(), &mut || 0)
    }

    /// `testZeroOrMin()`: try to make GCD compression fail if the format did
    /// not anticipate that the GCD of 0 and MIN_VALUE is negative.
    fn test_zero_or_min(&self) -> Result<()> {
        let num_iterations = LuceneTestCase::at_least(1);
        for _ in 0..num_iterations {
            let mut longs = || if random().next_boolean() { 0 } else { i64::MIN };
            self.do_test_numerics_vs_stored_fields(1.0, &mut longs)?;
        }
        Ok(())
    }

    /// `testTwoNumbersOneMissing()`.
    fn test_two_numbers_one_missing(&self) -> Result<()> {
        numbers_one_missing(self, false, false)
    }

    /// `testTwoNumbersOneMissingWithMerging()`.
    fn test_two_numbers_one_missing_with_merging(&self) -> Result<()> {
        numbers_one_missing(self, true, false)
    }

    /// `testThreeNumbersOneMissingWithMerging()`.
    fn test_three_numbers_one_missing_with_merging(&self) -> Result<()> {
        numbers_one_missing(self, true, true)
    }

    /// `testTwoBytesOneMissing()`.
    fn test_two_bytes_one_missing(&self) -> Result<()> {
        bytes_one_missing(self, false, false)
    }

    /// `testTwoBytesOneMissingWithMerging()`.
    fn test_two_bytes_one_missing_with_merging(&self) -> Result<()> {
        bytes_one_missing(self, true, false)
    }

    /// `testThreeBytesOneMissingWithMerging()`.
    fn test_three_bytes_one_missing_with_merging(&self) -> Result<()> {
        bytes_one_missing(self, true, true)
    }

    /// `testThreads()`: tests dv against stored fields with threads
    /// (binary/numeric/sorted, no missing).
    fn test_threads(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
            let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
            // `new BinaryDocValuesField("dvBin", newBytesRef())`,
            // `new SortedDocValuesField("dvSorted", newBytesRef())`
            let _ = LuceneTestCase::new_bytes_ref();
            let _ = LuceneTestCase::new_bytes_ref();

            // index some docs
            let num_docs = LuceneTestCase::at_least(300);
            for i in 0..num_docs {
                let length = TestUtil::next_int(&*random(), 0, 8) as usize;
                let mut buffer = vec![0u8; length];
                random().next_bytes(&mut buffer);
                let numeric_value = random().next_long();
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                doc.add(Box::new(StoredField::new_bytes(
                    "storedBin",
                    BytesRef::new(buffer.clone()),
                )?));
                doc.add(Box::new(BinaryDocValuesField::new(
                    "dvBin",
                    BytesRef::new(buffer.clone()),
                )));
                doc.add(Box::new(SortedDocValuesField::new(
                    "dvSorted",
                    BytesRef::new(buffer),
                )));
                doc.add(Box::new(StoredField::new_string(
                    "storedNum",
                    numeric_value.to_string(),
                )?));
                doc.add(Box::new(NumericDocValuesField::new("dvNum", numeric_value)));
                writer.add_document(&doc)?;
                if random().next_int_bound(31) == 0 {
                    writer.commit()?;
                }
            }

            // delete some docs
            let num_deletions = random().next_int_bound(num_docs / 10);
            for _ in 0..num_deletions {
                let id = random().next_int_bound(num_docs);
                writer.delete_documents(term("id", &id.to_string()))?;
            }
            writer.close()?;
            drop(writer);

            // compare
            let ir =
                self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
            let num_threads = TestUtil::next_int(&*random(), 2, 7);
            let starting_gun = Arc::new(Latch::new());
            let mut threads = Vec::with_capacity(num_threads as usize);
            for _ in 0..num_threads {
                let (ir, starting_gun) = (
                    Arc::clone(&ir) as Arc<dyn IndexReader>,
                    Arc::clone(&starting_gun),
                );
                let thread = Thread::unnamed(move || {
                    starting_gun.wait();
                    let run = (|| -> Result<()> {
                        for context in Arc::clone(&ir).leaves()? {
                            let r = context.leaf_reader();
                            let stored_fields = r.stored_fields()?;
                            let mut binaries = npe(r.get_binary_doc_values("dvBin")?);
                            let mut sorted = npe(r.get_sorted_doc_values("dvSorted")?);
                            let mut numerics = npe(r.get_numeric_doc_values("dvNum")?);
                            for j in 0..r.max_doc() {
                                let binary_value =
                                    stored_fields.document(j)?.get_binary_value("storedBin");
                                aeq(j, binaries.next_doc()?);
                                let scratch = binaries.binary_value()?;
                                Assert::assert_equals(
                                    Boxed(binary_value.clone()),
                                    Boxed(Some(scratch)),
                                );
                                aeq(j, sorted.next_doc()?);
                                let scratch = sorted.lookup_ord(sorted.ord_value()?)?;
                                Assert::assert_equals(Boxed(binary_value), Boxed(Some(scratch)));
                                let expected = doc_get(&stored_fields.document(j)?, "storedNum");
                                aeq(j, numerics.next_doc()?);
                                aeq(parse_long(&npe(expected)), numerics.long_value()?);
                            }
                        }
                        TestUtil::check_reader(&ir)
                    })();
                    if let Err(e) = run {
                        runtime_exception(e)
                    }
                });
                thread.start();
                threads.push(thread);
            }
            starting_gun.count_down();
            for t in &threads {
                if let Err(payload) = t.join() {
                    std::panic::resume_unwind(payload);
                }
            }
            ir.close()?;
            drop(ir);
        }
        dir.close()
    }

    /// `testThreads2()` (`@Nightly`): tests dv against stored fields with
    /// threads (all types + missing).
    fn test_threads2(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
            let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
            // `new BinaryDocValuesField("dvBin", newBytesRef())`,
            // `new SortedDocValuesField("dvSorted", newBytesRef())`
            let _ = LuceneTestCase::new_bytes_ref();
            let _ = LuceneTestCase::new_bytes_ref();

            // index some docs
            let num_docs = TestUtil::next_int(&*random(), 1025, 2047);
            for i in 0..num_docs {
                let length = TestUtil::next_int(&*random(), 0, 8) as usize;
                let mut buffer = vec![0u8; length];
                random().next_bytes(&mut buffer);
                let numeric_value = random().next_long();
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                if random().next_int_bound(4) > 0 {
                    doc.add(Box::new(StoredField::new_bytes(
                        "storedBin",
                        BytesRef::new(buffer.clone()),
                    )?));
                    doc.add(Box::new(BinaryDocValuesField::new(
                        "dvBin",
                        BytesRef::new(buffer.clone()),
                    )));
                    doc.add(Box::new(SortedDocValuesField::new(
                        "dvSorted",
                        BytesRef::new(buffer),
                    )));
                }
                if random().next_int_bound(4) > 0 {
                    doc.add(Box::new(StoredField::new_string(
                        "storedNum",
                        numeric_value.to_string(),
                    )?));
                    doc.add(Box::new(NumericDocValuesField::new("dvNum", numeric_value)));
                }
                let num_sorted_set_fields = random().next_int_bound(3);
                let mut values: BTreeSet<String> = BTreeSet::new();
                for _ in 0..num_sorted_set_fields {
                    values.insert(TestUtil::random_simple_string(&*random()));
                }
                for v in &values {
                    doc.add(Box::new(SortedSetDocValuesField::new(
                        "dvSortedSet",
                        nbr(v),
                    )));
                    doc.add(Box::new(StoredField::new_string(
                        "storedSortedSet",
                        v.clone(),
                    )?));
                }
                let num_sorted_numeric_fields = random().next_int_bound(3);
                let mut num_values: BTreeSet<i64> = BTreeSet::new();
                for _ in 0..num_sorted_numeric_fields {
                    num_values.insert(TestUtil::next_long(&*random(), i64::MIN, i64::MAX));
                }
                for l in &num_values {
                    doc.add(Box::new(SortedNumericDocValuesField::new(
                        "dvSortedNumeric",
                        *l,
                    )));
                    doc.add(Box::new(StoredField::new_string(
                        "storedSortedNumeric",
                        l.to_string(),
                    )?));
                }
                writer.add_document(&doc)?;
                if random().next_int_bound(31) == 0 {
                    writer.commit()?;
                }
            }

            // delete some docs
            let num_deletions = random().next_int_bound(num_docs / 10);
            for _ in 0..num_deletions {
                let id = random().next_int_bound(num_docs);
                writer.delete_documents(term("id", &id.to_string()))?;
            }
            writer.close()?;
            drop(writer);

            // compare
            let ir =
                self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
            let num_threads = TestUtil::next_int(&*random(), 2, 7);
            let starting_gun = Arc::new(Latch::new());
            let mut threads = Vec::with_capacity(num_threads as usize);
            for _ in 0..num_threads {
                let (ir, starting_gun) = (
                    Arc::clone(&ir) as Arc<dyn IndexReader>,
                    Arc::clone(&starting_gun),
                );
                let thread = Thread::unnamed(move || {
                    starting_gun.wait();
                    let run = (|| -> Result<()> {
                        for context in Arc::clone(&ir).leaves()? {
                            let r = context.leaf_reader();
                            let stored_fields = r.stored_fields()?;
                            let mut binaries = r.get_binary_doc_values("dvBin")?;
                            let mut sorted = r.get_sorted_doc_values("dvSorted")?;
                            let mut numerics = r.get_numeric_doc_values("dvNum")?;
                            let mut sorted_set = r.get_sorted_set_doc_values("dvSortedSet")?;
                            let mut sorted_numeric =
                                r.get_sorted_numeric_doc_values("dvSortedNumeric")?;
                            for j in 0..r.max_doc() {
                                let binary_value =
                                    stored_fields.document(j)?.get_binary_value("storedBin");
                                if let Some(binary_value) = binary_value {
                                    if let Some(binaries) = binaries.as_mut() {
                                        aeq(j, binaries.next_doc()?);
                                        let scratch = binaries.binary_value()?;
                                        Assert::assert_equals(binary_value.clone(), scratch);
                                        let sorted = npe(sorted.as_mut());
                                        aeq(j, sorted.next_doc()?);
                                        let scratch = sorted.lookup_ord(sorted.ord_value()?)?;
                                        Assert::assert_equals(binary_value, scratch);
                                    }
                                }

                                let number = doc_get(&stored_fields.document(j)?, "storedNum");
                                if let Some(number) = number {
                                    if let Some(numerics) = numerics.as_mut() {
                                        aeq(j, numerics.advance(j)?);
                                        aeq(parse_long(&number), numerics.long_value()?);
                                    }
                                }

                                let values =
                                    stored_fields.document(j)?.get_values("storedSortedSet");
                                if !values.is_empty() {
                                    assert_not_null(sorted_set.is_some());
                                    let sorted_set = npe(sorted_set.as_mut());
                                    aeq(j, sorted_set.next_doc()?);
                                    aeq(values.len() as i64, sorted_set.doc_value_count()?);
                                    for s in &values {
                                        let ord = sorted_set.next_ord()?;
                                        let value = sorted_set.lookup_ord(ord)?;
                                        Assert::assert_equals(s.clone(), value.utf8_to_string()?);
                                    }
                                }

                                let num_values =
                                    stored_fields.document(j)?.get_values("storedSortedNumeric");
                                if !num_values.is_empty() {
                                    assert_not_null(sorted_numeric.is_some());
                                    let sorted_numeric = npe(sorted_numeric.as_mut());
                                    aeq(j, sorted_numeric.next_doc()?);
                                    aeq(num_values.len() as i64, sorted_numeric.doc_value_count()?);
                                    for num_value in &num_values {
                                        let v = sorted_numeric.next_value()?;
                                        Assert::assert_equals(num_value.clone(), v.to_string());
                                    }
                                }
                            }
                        }
                        TestUtil::check_reader(&ir)
                    })();
                    if let Err(e) = run {
                        runtime_exception(e)
                    }
                });
                thread.start();
                threads.push(thread);
            }
            starting_gun.count_down();
            for t in &threads {
                if let Err(payload) = t.join() {
                    std::panic::resume_unwind(payload);
                }
            }
            ir.close()?;
            drop(ir);
        }
        dir.close()
    }

    /// `testThreads3()` (`@Nightly`).
    fn test_threads3(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
        {
            let conf = LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer());
            let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;

            let num_sorted_sets = random().next_int_bound(21);
            let num_binaries = random().next_int_bound(21);
            let num_sorted_nums = random().next_int_bound(21);

            let num_docs = TestUtil::next_int(&*random(), 2025, 2047);
            for _ in 0..num_docs {
                let mut doc = Document::new();

                for j in 0..num_sorted_sets {
                    doc.add(Box::new(SortedSetDocValuesField::new(
                        &format!("ss{j}"),
                        nbr(&TestUtil::random_simple_string(&*random())),
                    )));
                    doc.add(Box::new(SortedSetDocValuesField::new(
                        &format!("ss{j}"),
                        nbr(&TestUtil::random_simple_string(&*random())),
                    )));
                }

                for j in 0..num_binaries {
                    doc.add(Box::new(BinaryDocValuesField::new(
                        &format!("b{j}"),
                        nbr(&TestUtil::random_simple_string(&*random())),
                    )));
                }

                for j in 0..num_sorted_nums {
                    doc.add(Box::new(SortedNumericDocValuesField::new(
                        &format!("sn{j}"),
                        TestUtil::next_long(&*random(), i64::MIN, i64::MAX),
                    )));
                    doc.add(Box::new(SortedNumericDocValuesField::new(
                        &format!("sn{j}"),
                        TestUtil::next_long(&*random(), i64::MIN, i64::MAX),
                    )));
                }
                writer.add_document(&doc)?;
            }
            writer.close()?;
            drop(writer);

            // now check with threads
            for _ in 0..10 {
                let r = self
                    .maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
                let starting_gun = Arc::new(Latch::new());
                let thread_count = TestUtil::next_int(&*random(), 4, 10);
                let mut threads = Vec::with_capacity(thread_count as usize);
                for _ in 0..thread_count {
                    let (r, starting_gun) = (
                        Arc::clone(&r) as Arc<dyn IndexReader>,
                        Arc::clone(&starting_gun),
                    );
                    threads.push(Thread::unnamed(move || {
                        let info_stream =
                            rucene::util::PrintStreamInfoStream::new(Box::new(Vec::<u8>::new()));
                        starting_gun.wait();
                        let run = (|| -> Result<()> {
                            for leaf in Arc::clone(&r).leaves()? {
                                let codec_reader = leaf
                                    .leaf_reader()
                                    .as_codec_reader()
                                    .ok_or_else(|| LuceneError::Unchecked {
                                        class: "java.lang.ClassCastException",
                                        message: Some("the leaf is not a CodecReader".to_string()),
                                        cause: None,
                                    })?;
                                let status = rucene::index::check_index::test_doc_values(
                                    &*codec_reader,
                                    Some(&info_stream),
                                    true,
                                )?;
                                if let Some(error) = status.error {
                                    return Err(error);
                                }
                            }
                            Ok(())
                        })();
                        if let Err(e) = run {
                            runtime_exception(e)
                        }
                    }));
                }
                for thread in &threads {
                    thread.start();
                }
                starting_gun.count_down();
                for thread in &threads {
                    if let Err(payload) = thread.join() {
                        std::panic::resume_unwind(payload);
                    }
                }
                r.close()?;
            }
        }
        dir.close()
    }

    /// `testEmptyBinaryValueOnPageSizes()` (`@Nightly`): LUCENE-5218.
    // TODO: incredibly slow
    fn test_empty_binary_value_on_page_sizes(&self) -> Result<()> {
        // Test larger and larger power-of-two sized values,
        // followed by empty string value:
        for i in 0..20 {
            if i > 14 && !self.codec_accepts_huge_binary_values("field") {
                break;
            }
            let mut dir = LuceneTestCase::new_directory();
            {
                let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
                let bytes = LuceneTestCase::new_bytes_ref_slice(&vec![0u8; 1 << i], 0, 1 << i);
                for _ in 0..4 {
                    let mut doc = Document::new();
                    doc.add(Box::new(BinaryDocValuesField::new("field", bytes.clone())));
                    w.add_document(&doc)?;
                }
                let mut doc = Document::new();
                doc.add(Box::new(StoredField::new_string("id", "5".to_string())?));
                doc.add(Box::new(BinaryDocValuesField::new(
                    "field",
                    LuceneTestCase::new_bytes_ref(),
                )));
                w.add_document(&doc)?;
                let r: Arc<dyn IndexReader> = w.get_reader()?;
                w.close()?;
                drop(w);

                let mut values = npe(MultiDocValues::get_binary_values(&r, "field")?);
                for j in 0..5 {
                    aeq(j, values.next_doc()?);
                    let result = values.binary_value()?;
                    Assert::assert_true(result.length == 0 || result.length == 1 << i);
                }
                drop(values);
                r.close()?;
                drop(r);
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `testOneSortedNumber()`.
    fn test_one_sorted_number(&self) -> Result<()> {
        let mut directory = LuceneTestCase::new_directory();
        {
            let writer =
                RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
            let mut doc = Document::new();
            doc.add(Box::new(SortedNumericDocValuesField::new("dv", 5)));
            writer.add_document(&doc)?;
            writer.close()?;
            drop(writer);

            // Now search the index:
            let reader = open_reader(self, &directory)?;
            let leaf = only_leaf(&reader)?;
            let mut dv = npe(leaf.get_sorted_numeric_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            aeq(1, dv.doc_value_count()?);
            aeq(5, dv.next_value()?);

            drop((dv, leaf));
            reader.close()?;
        }
        directory.close()
    }

    /// `testOneSortedNumberOneMissing()`.
    fn test_one_sorted_number_one_missing(&self) -> Result<()> {
        let mut directory = LuceneTestCase::new_directory();
        {
            let writer = IndexWriter::new(
                directory.as_directory(),
                IndexWriterConfig::with_analyzer(null_analyzer()),
            )?;
            let mut doc = Document::new();
            doc.add(Box::new(SortedNumericDocValuesField::new("dv", 5)));
            writer.add_document(&doc)?;
            writer.add_document(&Document::new())?;
            writer.close()?;
            drop(writer);

            // Now search the index:
            let reader = open_reader(self, &directory)?;
            let leaf = only_leaf(&reader)?;
            let mut dv = npe(leaf.get_sorted_numeric_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            aeq(1, dv.doc_value_count()?);
            aeq(5, dv.next_value()?);
            aeq(NO_MORE_DOCS, dv.next_doc()?);

            drop((dv, leaf));
            reader.close()?;
        }
        directory.close()
    }

    /// `testNumberMergeAwayAllValues()`.
    fn test_number_merge_away_all_values(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(NumericDocValuesField::new("field", 5))),
            |leaf| {
                let mut dv = npe(leaf.get_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                Ok(())
            },
        )
    }

    /// `testTwoSortedNumber()`.
    fn test_two_sorted_number(&self) -> Result<()> {
        two_sorted_numbers(self, 11, -5, &[-5, 11])
    }

    /// `testTwoSortedNumberSameValue()`.
    fn test_two_sorted_number_same_value(&self) -> Result<()> {
        two_sorted_numbers(self, 11, 11, &[11, 11])
    }

    /// `testTwoSortedNumberOneMissing()`.
    fn test_two_sorted_number_one_missing(&self) -> Result<()> {
        let mut directory = LuceneTestCase::new_directory();
        {
            let writer = IndexWriter::new(
                directory.as_directory(),
                IndexWriterConfig::with_analyzer(null_analyzer()),
            )?;
            let mut doc = Document::new();
            doc.add(Box::new(SortedNumericDocValuesField::new("dv", 11)));
            doc.add(Box::new(SortedNumericDocValuesField::new("dv", -5)));
            writer.add_document(&doc)?;
            writer.add_document(&Document::new())?;
            writer.close()?;
            drop(writer);

            // Now search the index:
            let reader = open_reader(self, &directory)?;
            let leaf = only_leaf(&reader)?;
            let mut dv = npe(leaf.get_sorted_numeric_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            aeq(2, dv.doc_value_count()?);
            aeq(-5, dv.next_value()?);
            aeq(11, dv.next_value()?);
            aeq(NO_MORE_DOCS, dv.next_doc()?);

            drop((dv, leaf));
            reader.close()?;
        }
        directory.close()
    }

    /// `testSortedNumberMerge()`.
    fn test_sorted_number_merge(&self) -> Result<()> {
        let mut directory = LuceneTestCase::new_directory();
        {
            let mut iwc = IndexWriterConfig::with_analyzer(null_analyzer());
            iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
            let writer = IndexWriter::new(directory.as_directory(), iwc)?;
            let mut doc = Document::new();
            doc.add(Box::new(SortedNumericDocValuesField::new("dv", 11)));
            writer.add_document(&doc)?;
            writer.commit()?;
            let mut doc = Document::new();
            doc.add(Box::new(SortedNumericDocValuesField::new("dv", -5)));
            writer.add_document(&doc)?;
            writer.force_merge(1)?;
            writer.close()?;
            drop(writer);

            // Now search the index:
            let reader = open_reader(self, &directory)?;
            let leaf = only_leaf(&reader)?;
            let mut dv = npe(leaf.get_sorted_numeric_doc_values("dv")?);
            aeq(0, dv.next_doc()?);
            aeq(1, dv.doc_value_count()?);
            aeq(11, dv.next_value()?);
            aeq(1, dv.next_doc()?);
            aeq(1, dv.doc_value_count()?);
            aeq(-5, dv.next_value()?);

            drop((dv, leaf));
            reader.close()?;
        }
        directory.close()
    }

    /// `testSortedNumberMergeAwayAllValues()`.
    fn test_sorted_number_merge_away_all_values(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(SortedNumericDocValuesField::new("field", 5))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                Ok(())
            },
        )
    }

    /// `testSortedEnumAdvanceIndependently()`.
    fn test_sorted_enum_advance_independently(&self) -> Result<()> {
        let (mut directory, iwriter) = sorted_set_writer(true)?;
        {
            let iwriter = iwriter;

            for value in ["2", "1", "3"] {
                let mut doc = Document::new();
                doc.add(Box::new(SortedDocValuesField::new("field", nbr(value))));
                iwriter.add_document(&doc)?;
            }

            iwriter.commit()?;
            iwriter.force_merge(1)?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let dv = || -> Result<Box<dyn SortedSetDocValues>> {
                let sorted = npe(leaf.get_sorted_doc_values("field")?);
                Ok(Box::new(DocValues::singleton_sorted(sorted)))
            };
            self.do_test_sorted_set_enum_advance_independently(&dv)?;

            drop(leaf);
            ireader.close()?;
        }
        directory.close()
    }

    /// `testSortedSetEnumAdvanceIndependently()`.
    fn test_sorted_set_enum_advance_independently(&self) -> Result<()> {
        let (mut directory, iwriter) = sorted_set_writer(true)?;
        {
            let iwriter = iwriter;

            // field1 "2", field2 "3"; then field1 "1"; then field2 "2"
            let mut values = ["2".to_string(), "3".to_string()];
            let first = (nbr("2"), nbr("3"));
            let mut doc = Document::new();
            doc.add(Box::new(SortedSetDocValuesField::new("field", first.0)));
            doc.add(Box::new(SortedSetDocValuesField::new("field", first.1)));
            iwriter.add_document(&doc)?;
            values[0] = "1".to_string();
            let mut doc = Document::new();
            doc.add(Box::new(SortedSetDocValuesField::new(
                "field",
                nbr(&values[0]),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "field",
                BytesRef::from_char_sequence(&values[1]),
            )));
            iwriter.add_document(&doc)?;
            values[1] = "2".to_string();
            let mut doc = Document::new();
            doc.add(Box::new(SortedSetDocValuesField::new(
                "field",
                BytesRef::from_char_sequence(&values[0]),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "field",
                nbr(&values[1]),
            )));
            iwriter.add_document(&doc)?;

            iwriter.commit()?;
            iwriter.force_merge(1)?;

            let ireader = iwriter.get_reader()?;
            iwriter.close()?;
            drop(iwriter);

            let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as _)?;
            let dv = || -> Result<Box<dyn SortedSetDocValues>> {
                Ok(npe(leaf.get_sorted_set_doc_values("field")?))
            };
            self.do_test_sorted_set_enum_advance_independently(&dv)?;

            drop(leaf);
            ireader.close()?;
        }
        directory.close()
    }

    /// `doTestSortedSetEnumAdvanceIndependently(dv)` (`protected`); `dv`
    /// gives the field's doc values again for each `termsEnum()` (see the
    /// module docs).
    fn do_test_sorted_set_enum_advance_independently(
        &self,
        dv: &dyn Fn() -> Result<Box<dyn SortedSetDocValues>>,
    ) -> Result<()> {
        if dv()?.get_value_count()? < 2 {
            return Ok(());
        }
        let mut terms: Vec<BytesRef> = Vec::new();
        let mut te = dv()?.terms_enum()?;
        terms.push(BytesRef::deep_copy_of(&npe(te.next()?)));
        terms.push(BytesRef::deep_copy_of(&npe(te.next()?)));

        // Make sure that calls to next() does not modify the term of the other enum
        let mut enum1 = dv()?.terms_enum()?;
        let mut enum2 = dv()?.terms_enum()?;
        let mut term1 = BytesRefBuilder::new();
        let mut term2 = BytesRefBuilder::new();

        term1.copy_ref(&npe(enum1.next()?));
        term2.copy_ref(&npe(enum2.next()?));
        term1.copy_ref(&npe(enum1.next()?));

        Assert::assert_equals(term1.get(), enum1.term()?);
        Assert::assert_equals(term2.get(), enum2.term()?);

        // Same for seekCeil
        let mut enum1 = dv()?.terms_enum()?;
        let mut enum2 = dv()?.terms_enum()?;
        let mut term1 = BytesRefBuilder::new();
        let mut term2 = BytesRefBuilder::new();

        term2.copy_ref(&npe(enum2.next()?));
        let mut seek_term = BytesRefBuilder::new();
        seek_term.append_ref(&terms[0]);
        seek_term.append(0);
        enum1.seek_ceil(&seek_term.get())?;
        term1.copy_ref(&enum1.term()?);

        Assert::assert_equals(term1.get(), enum1.term()?);
        Assert::assert_equals(term2.get(), enum2.term()?);

        // Same for seekCeil on an exact value
        let mut enum1 = dv()?.terms_enum()?;
        let mut enum2 = dv()?.terms_enum()?;
        let mut term1 = BytesRefBuilder::new();
        let mut term2 = BytesRefBuilder::new();

        term2.copy_ref(&npe(enum2.next()?));
        enum1.seek_ceil(&terms[1])?;
        term1.copy_ref(&enum1.term()?);

        Assert::assert_equals(term1.get(), enum1.term()?);
        Assert::assert_equals(term2.get(), enum2.term()?);

        // Same for seekExact
        let mut enum1 = dv()?.terms_enum()?;
        let mut enum2 = dv()?.terms_enum()?;
        let mut term1 = BytesRefBuilder::new();
        let mut term2 = BytesRefBuilder::new();

        term2.copy_ref(&npe(enum2.next()?));
        let found = enum1.seek_exact(&terms[1])?;
        Assert::assert_true(found);
        term1.copy_ref(&enum1.term()?);

        // Same for seek by ord
        let mut enum1 = dv()?.terms_enum()?;
        let mut enum2 = dv()?.terms_enum()?;
        let mut term1 = BytesRefBuilder::new();
        let mut term2 = BytesRefBuilder::new();

        term2.copy_ref(&npe(enum2.next()?));
        enum1.seek_ord(1)?;
        term1.copy_ref(&enum1.term()?);

        Assert::assert_equals(term1.get(), enum1.term()?);
        Assert::assert_equals(term2.get(), enum2.term()?);
        Ok(())
    }

    /// `testSortedMergeAwayAllValuesLargeSegment()`: same as
    /// `testSortedMergeAwayAllValues` but on more than 1024 docs to have
    /// sparse encoding on.
    fn test_sorted_merge_away_all_values_large_segment(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(SortedDocValuesField::new("field", nbr("hello")))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);

                let mut terms_enum = dv.terms_enum()?;
                Assert::assert_false(
                    terms_enum.seek_exact(&BytesRef::from_char_sequence("lucene"))?,
                );
                assert_seek_status(
                    SeekStatus::END,
                    terms_enum.seek_ceil(&BytesRef::from_char_sequence("lucene"))?,
                );
                let dv = npe(leaf.get_sorted_doc_values("field")?);
                aeq(-1, dv.lookup_term(&BytesRef::from_char_sequence("lucene"))?);
                Ok(())
            },
        )
    }

    /// `testSortedSetMergeAwayAllValuesLargeSegment()`: same as
    /// `testSortedSetMergeAwayAllValues` but on more than 1024 docs to have
    /// sparse encoding on.
    fn test_sorted_set_merge_away_all_values_large_segment(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(SortedSetDocValuesField::new("field", nbr("hello")))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);

                let mut terms_enum = dv.terms_enum()?;
                Assert::assert_false(
                    terms_enum.seek_exact(&BytesRef::from_char_sequence("lucene"))?,
                );
                assert_seek_status(
                    SeekStatus::END,
                    terms_enum.seek_ceil(&BytesRef::from_char_sequence("lucene"))?,
                );
                let dv = npe(leaf.get_sorted_set_doc_values("field")?);
                aeq(-1, dv.lookup_term(&BytesRef::from_char_sequence("lucene"))?);
                Ok(())
            },
        )
    }

    /// `testNumericMergeAwayAllValuesLargeSegment()`: same as
    /// `testNumericMergeAwayAllValues` but on more than 1024 docs to have
    /// sparse encoding on.
    fn test_numeric_merge_away_all_values_large_segment(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(NumericDocValuesField::new("field", 42))),
            |leaf| {
                let mut dv = npe(leaf.get_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                Ok(())
            },
        )
    }

    /// `testSortedNumericMergeAwayAllValuesLargeSegment()`: same as
    /// `testSortedNumericMergeAwayAllValues` but on more than 1024 docs to
    /// have sparse encoding on.
    fn test_sorted_numeric_merge_away_all_values_large_segment(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(SortedNumericDocValuesField::new("field", 42))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                Ok(())
            },
        )
    }

    /// `testBinaryMergeAwayAllValuesLargeSegment()`: same as
    /// `testBinaryMergeAwayAllValues` but on more than 1024 docs to have
    /// sparse encoding on.
    fn test_binary_merge_away_all_values_large_segment(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(BinaryDocValuesField::new("field", nbr("hello")))),
            |leaf| {
                let mut dv = npe(leaf.get_binary_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                Ok(())
            },
        )
    }

    /// `testRandomAdvanceNumeric()`.
    fn test_random_advance_numeric(&self) -> Result<()> {
        let long_range = if random().next_boolean() {
            i64::from(TestUtil::next_int(&*random(), 1, 1024))
        } else {
            TestUtil::next_long(&*random(), 1, i64::MAX)
        };
        self.do_test_random_advance(&FieldCreator {
            next: Box::new(move || {
                Box::new(NumericDocValuesField::new(
                    "field",
                    TestUtil::next_long(&*random(), 0, long_range),
                ))
            }),
            iterator: Box::new(|r| {
                Ok(npe(MultiDocValues::get_numeric_values(r, "field")?)
                    as Box<dyn DocIdSetIterator>)
            }),
        })
    }

    /// `testRandomAdvanceBinary()`.
    fn test_random_advance_binary(&self) -> Result<()> {
        self.do_test_random_advance(&FieldCreator {
            next: Box::new(|| {
                let mut bytes = vec![0u8; random().next_int_bound(10) as usize];
                random().next_bytes(&mut bytes);
                Box::new(BinaryDocValuesField::new(
                    "field",
                    LuceneTestCase::new_bytes_ref_bytes(&bytes),
                ))
            }),
            iterator: Box::new(|r| {
                Ok(npe(MultiDocValues::get_binary_values(r, "field")?)
                    as Box<dyn DocIdSetIterator>)
            }),
        })
    }

    /// `testHighOrdsSortedSetDV()` (`@Nightly`): tests where a DVField uses a
    /// high number of packed bits to store its ords. See LUCENE-10159.
    fn test_high_ords_sorted_set_dv(&self) -> Result<()> {
        LuceneTestCase::assume_false(
            "This test with SimpleTextCodec requires a lot of memory",
            is_simple_text(&*self.get_codec()),
        );
        let mut dir = LuceneTestCase::new_directory();
        {
            let mut iwc = IndexWriterConfig::new();
            iwc.set_ram_buffer_size_mb(f64::from(8 + random().next_int_bound(64)))?;
            let writer = IndexWriter::new(dir.as_directory(), iwc)?;
            // many docs with some of them have very high ords
            let num_docs = 20_000 + random().next_int_bound(10_000);
            for _ in 1..num_docs {
                let num_ords = if random().next_int_bound(100) <= 5 {
                    1000 + random().next_int_bound(500)
                } else {
                    random().next_int_bound(10)
                };
                let mut doc = Document::new();
                for _ in 0..num_ords {
                    doc.add(Box::new(SortedSetDocValuesField::new(
                        "sorted_set_dv",
                        TestUtil::random_binary_term_with(&*random(), 2),
                    )));
                }
                writer.add_document(&doc)?;
            }
            writer.force_merge_with(1, true)?;
            {
                let reader = directory_reader::open_with_writer(&*writer)?;
                let checked =
                    TestUtil::check_reader(&(Arc::clone(&reader) as Arc<dyn IndexReader>));
                let closed = reader.close();
                checked?;
                closed?;
            }
            // `IOUtils.close(writer, dir)`
            writer.close()?;
            drop(writer);
        }
        dir.close()
    }

    /// `doTestRandomAdvance(fieldCreator)` (`private`).
    fn do_test_random_advance(&self, field_creator: &FieldCreator) -> Result<()> {
        let analyzer = mock_analyzer();

        let mut directory = LuceneTestCase::new_directory();
        {
            let mut conf = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
            conf.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
            let w = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;
            let num_chunks = LuceneTestCase::at_least(10);
            let mut id = 0i32;
            let mut missing_set: HashSet<i32> = HashSet::new();
            for _ in 0..num_chunks {
                // change sparseness for each chunk
                let sparse_chance = random().next_double();
                let doc_count = LuceneTestCase::at_least(1000);
                for _ in 0..doc_count {
                    let mut doc = Document::new();
                    doc.add(Box::new(StoredField::new_number(
                        "id",
                        NumericValue::Int(id),
                    )?));
                    if random().next_double() > sparse_chance {
                        doc.add((field_creator.next)());
                    } else {
                        missing_set.insert(id);
                    }
                    id += 1;
                    w.add_document(&doc)?;
                }
            }

            if random().next_boolean() {
                w.force_merge(1)?;
            }

            // Now search the index:
            let r: Arc<dyn IndexReader> = w.get_reader()?;
            let stored_fields = r.stored_fields()?;
            let mut missing = FixedBitSet::new(r.max_doc() as usize);
            for doc_id in 0..r.max_doc() {
                let doc = stored_fields.document(doc_id)?;
                let value = npe(npe(doc.get_field("id")).numeric_value());
                if let NumericValue::Int(value) = value {
                    if missing_set.contains(&value) {
                        missing.set(doc_id as usize);
                    }
                }
            }

            let num_iters = LuceneTestCase::at_least(10);
            for _ in 0..num_iters {
                let mut values = (field_creator.iterator)(&r)?;
                aeq(-1, values.doc_id());

                loop {
                    let doc_id = if random().next_boolean() {
                        values.next_doc()?
                    } else {
                        let range = if random().next_int_bound(10) == 7 {
                            // big jump
                            r.max_doc() - values.doc_id()
                        } else {
                            // small jump
                            25
                        };
                        let inc = TestUtil::next_int(&*random(), 1, range);
                        values.advance(values.doc_id() + inc)?
                    };
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    Assert::assert_false(missing.get(doc_id as usize));
                }
            }

            // `IOUtils.close(r, w, directory)`
            drop(stored_fields);
            r.close()?;
            drop(r);
            w.close()?;
            drop(w);
        }
        directory.close()
    }

    /// `codecAcceptsHugeBinaryValues(field)` (`protected`).
    fn codec_accepts_huge_binary_values(&self, _field: &str) -> bool {
        true
    }
}
