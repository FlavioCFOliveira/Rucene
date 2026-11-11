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

//! Port of `org.apache.lucene.tests.index.BaseNormsFormatTestCase`: abstract
//! class to do basic tests for a norms format. NOTE: This test focuses on the
//! norms impl, nothing else. The [stretch] goal is for this test to be so
//! thorough in testing a new `NormsFormat` that if this test passes, then all
//! Lucene tests should also pass. Ie, if there is some bug in a given
//! `NormsFormat` that this test fails to catch then this test needs to be
//! improved!
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BaseNormsFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * Shared `Field` instances (`doTestNormsVersusDocValues`, `testThreads`):
//!   Java adds the same `idField`, `indexedField` and `dvField` to every
//!   document and changes their values between additions; the fields are
//!   built without randomness, so the port builds them afresh, with the same
//!   values, for each document.
//! * `CountDownLatch` (`testThreads`): a latch of one count over a
//!   `Mutex`/`Condvar`, with the same `await`/`countDown` behaviour.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, PoisonError};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, NumericDocValuesField, Store, StringField, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::terms::Term;
use rucene::index::{FieldInvertState, IndexReader};
use rucene::search::similarities::{CollectionStatistics, SimScorer, Similarity, TermStatistics};
use rucene::search::NO_MORE_DOCS;
use rucene::util::{BytesRef, FixedBitSet};

use super::base_index_file_format_test_case::{
    assert_array_equals, format_test, BaseIndexFileFormatTestCase,
};
use crate::analysis::mock_tokenizer::WHITESPACE;
use crate::analysis::MockAnalyzer;
use crate::index::random_index_writer::RandomIndexWriter;
use crate::util::lucene_test_case::{LuceneTestCase, Nightly};
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BaseNormsFormatTestCase`
/// (`public abstract class BaseNormsFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests and its override of
/// `testMergeStability()`.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseNormsFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(norms_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseNormsFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testByteRange", |c| c.test_byte_range()))
            .method(t("testSparseByteRange", |c| c.test_sparse_byte_range()))
            .method(t("testShortRange", |c| c.test_short_range()))
            .method(t("testSparseShortRange", |c| c.test_sparse_short_range()))
            .method(t("testLongRange", |c| c.test_long_range()))
            .method(t("testSparseLongRange", |c| c.test_sparse_long_range()))
            .method(t("testFullLongRange", |c| c.test_full_long_range()))
            .method(t("testSparseFullLongRange", |c| {
                c.test_sparse_full_long_range()
            }))
            .method(t("testFewValues", |c| c.test_few_values()))
            .method(t("testFewSparseValues", |c| c.test_few_sparse_values()))
            .method(t("testFewLargeValues", |c| c.test_few_large_values()))
            .method(t("testFewSparseLargeValues", |c| {
                c.test_few_sparse_large_values()
            }))
            .method(t("testAllZeros", |c| c.test_all_zeros()))
            .method(t("testSparseAllZeros", |c| c.test_sparse_all_zeros()))
            .method(t("testMostZeros", |c| c.test_most_zeros()))
            .method(t("testOutliers", |c| c.test_outliers()))
            .method(t("testSparseOutliers", |c| c.test_sparse_outliers()))
            .method(t("testOutliers2", |c| c.test_outliers2()))
            .method(t("testSparseOutliers2", |c| c.test_sparse_outliers2()))
            .method(t("testNCommon", |c| c.test_n_common()))
            .method(t("testSparseNCommon", |c| c.test_sparse_n_common()))
            .method(t("testNCommonBig", |c| c.test_n_common_big()).annotate(Nightly))
            .method(t("testSparseNCommonBig", |c| c.test_sparse_n_common_big()).annotate(Nightly))
            .method(t("testMergeStability", |c| c.norms_test_merge_stability()))
            .method(t("testUndeadNorms", |c| c.test_undead_norms()))
            .method(t("testThreads", |c| c.test_threads()))
            .method(t("testIndependantIterators", |c| {
                c.test_independant_iterators()
            }))
            .method(t("testIndependantSparseIterators", |c| {
                c.test_independant_sparse_iterators()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseNormsFormatTestCase`.
fn norms_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseNormsFormatTestCase {
    c.as_norms().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BaseNormsFormatTestCase",
        )
        .throw()
    })
}

/// `Byte.MIN_VALUE`, `Byte.MAX_VALUE`, `Short.MIN_VALUE`, `Short.MAX_VALUE`.
const BYTE_MIN: i64 = i8::MIN as i64;
const BYTE_MAX: i64 = i8::MAX as i64;
const SHORT_MIN: i64 = i16::MIN as i64;
const SHORT_MAX: i64 = i16::MAX as i64;

/// `random()`.
fn random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    LuceneTestCase::random()
}

/// A `LongSupplier`.
type LongSupplier<'a> = Box<dyn FnMut() -> i64 + 'a>;

/// The `LongSupplier` of the `N`-common tests: an `otherValues` value once
/// in a hundred draws, else a `commonValues` one.
fn n_common_supplier(
    r: Arc<dyn randomizedtesting::jdk::random::Random>,
    common_values: Vec<i64>,
    other_values: Vec<i64>,
) -> LongSupplier<'static> {
    let n = common_values.len() as i32;
    let num_other_values = other_values.len() as i32;
    Box::new(move || {
        if r.next_int_bound(100) == 0 {
            other_values[r.next_int_bound(num_other_values - 1) as usize]
        } else {
            common_values[r.next_int_bound(n - 1) as usize]
        }
    })
}

/// The `commonValues` and `otherValues` arrays of the `N`-common tests.
fn n_common_values(r: &dyn randomizedtesting::jdk::random::Random, n: i32) -> (Vec<i64>, Vec<i64>) {
    let common_values: Vec<i64> = (0..n)
        .map(|_| TestUtil::next_long(r, BYTE_MIN, BYTE_MAX))
        .collect();
    let num_other_values = TestUtil::next_int(r, 2, 256 - n);
    let other_values: Vec<i64> = (0..num_other_values)
        .map(|_| TestUtil::next_long(r, BYTE_MIN, BYTE_MAX))
        .collect();
    (common_values, other_values)
}

/// The `docsWithField` of `doTestNormsVersusDocValues` and `testThreads`.
fn random_docs_with_field(num_docs: i32, num_docs_with_field: i32) -> FixedBitSet {
    let mut docs_with_field = FixedBitSet::new(num_docs as usize);
    if num_docs_with_field == num_docs {
        docs_with_field.set_range(0, num_docs as usize);
    } else {
        let mut i = 0;
        while i < num_docs_with_field {
            let doc = random().next_int_bound(num_docs);
            if !docs_with_field.get(doc as usize) {
                docs_with_field.set(doc as usize);
                i += 1;
            }
        }
    }
    docs_with_field
}

/// The documents of `doTestNormsVersusDocValues` and `testThreads`: `id`,
/// and, for a document with the field, `indexed` and `dv`; a commit once in
/// 31 documents.
fn add_norms_documents(
    writer: &RandomIndexWriter,
    num_docs: i32,
    docs_with_field: &FixedBitSet,
    norms: &[i64],
) -> Result<()> {
    let mut j = 0;
    for i in 0..num_docs {
        let id_field = StringField::new("id", i.to_string(), Store::NO)?;
        if !docs_with_field.get(i as usize) {
            let mut doc2 = Document::new();
            doc2.add(Box::new(id_field));
            writer.add_document(&doc2)?;
        } else {
            let value = norms[j];
            j += 1;
            let mut doc = Document::new();
            doc.add(Box::new(id_field));
            // only empty fields may have 0 as a norm
            doc.add(Box::new(TextField::new(
                "indexed",
                if value == 0 { "" } else { "a" }.to_string(),
                Store::NO,
            )?));
            doc.add(Box::new(NumericDocValuesField::new("dv", value)));
            writer.add_document(&doc)?;
        }
        if random().next_int_bound(31) == 0 {
            writer.commit()?;
        }
    }
    Ok(())
}

/// `new MockAnalyzer(random(), MockTokenizer.WHITESPACE, false)`.
fn whitespace_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ))
}

/// `checkNormsVsDocValues(ir)` (`private`).
fn check_norms_vs_doc_values(ir: &Arc<dyn IndexReader>) -> Result<()> {
    for context in Arc::clone(ir).leaves()? {
        let r = context.leaf_reader();
        let expected = r.get_numeric_doc_values("dv")?;
        let actual = r.get_norm_values("indexed")?;
        Assert::assert_equals(expected.is_none(), actual.is_none());
        if let (Some(mut expected), Some(mut actual)) = (expected, actual) {
            let mut d = expected.next_doc()?;
            while d != NO_MORE_DOCS {
                Assert::assert_equals(i64::from(d), i64::from(actual.next_doc()?));
                Assert::assert_equals_msg(
                    Some(&format!("doc {d}")),
                    expected.long_value()?,
                    actual.long_value()?,
                );
                d = expected.next_doc()?;
            }
            Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(actual.next_doc()?));

            // Now check bulk fetching
            let mut expected = npe(r.get_numeric_doc_values("dv")?);
            let mut actual = npe(r.get_norm_values("indexed")?);

            let mut docs = [0i32; 16];
            let mut expected_values = [0i64; 16];
            let mut actual_values = [0i64; 16];
            let mut doc = -1;
            while doc < r.max_doc() {
                let mut size = 0;
                for j in 0..docs.len() {
                    doc += 1 + (j as i32 & 0x03);
                    if doc >= r.max_doc() {
                        break;
                    }
                    docs[size] = doc;
                    size += 1;
                }

                expected.long_values(
                    size as i32,
                    &docs,
                    0,
                    &mut expected_values,
                    0,
                    size as i64,
                )?;
                actual.long_values(size as i32, &docs, 0, &mut actual_values, 0, size as i64)?;
                assert_array_equals(&expected_values, &actual_values);
            }
        }
    }
    Ok(())
}

/// A null result Java would dereference: `NullPointerException`.
fn npe<T>(value: Option<T>) -> T {
    value.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// Port of `BaseNormsFormatTestCase.CannedNormSimilarity`: every norm comes
/// from `norms`, in order, skipping zeros.
#[derive(Debug)]
pub struct CannedNormSimilarity {
    norms: Vec<i64>,
    index: AtomicUsize,
}

impl CannedNormSimilarity {
    /// `CannedNormSimilarity(long[] norms)`.
    pub fn new(norms: Vec<i64>) -> Self {
        CannedNormSimilarity {
            norms,
            index: AtomicUsize::new(0),
        }
    }
}

impl Similarity for CannedNormSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.index.BaseNormsFormatTestCase$CannedNormSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        java_assert!(state.length() > 0);
        loop {
            // `norms[index++]`: one indexing thread at a time calls it.
            let index = self.index.fetch_add(1, Ordering::Relaxed);
            let Some(&norm) = self.norms.get(index) else {
                return Err(LuceneError::Unchecked {
                    class: "java.lang.ArrayIndexOutOfBoundsException",
                    message: Some(format!(
                        "Index {index} out of bounds for length {}",
                        self.norms.len()
                    )),
                    cause: None,
                });
            };
            if norm != 0 {
                return Ok(norm);
            }
        }
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.UnsupportedOperationException",
        )
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

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `BaseNormsFormatTestCase`.
pub trait BaseNormsFormatTestCase: BaseIndexFileFormatTestCase {
    /// `codecSupportsSparsity()`: whether the codec supports sparse values.
    fn codec_supports_sparsity(&self) -> bool {
        true
    }

    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn norms_add_random_fields(&self, doc: &mut Document) {
        // TODO: improve
        doc.add(Box::new(
            TextField::new(
                "foobar",
                TestUtil::random_simple_string(&*random()),
                Store::NO,
            )
            .unwrap_or_else(|e| super::base_index_file_format_test_case::rethrow(e)),
        ));
    }

    /// `testMergeStability()`, overridden.
    fn norms_test_merge_stability(&self) -> Result<()> {
        // TODO: can we improve this base test to just have subclasses declare the extensions to check,
        // rather than a blacklist to exclude? we need to index stuff to get norms, but we dont care
        // about testing
        // the PFs actually doing that...
        LuceneTestCase::assume_true(
            "The MockRandom PF randomizes content on the fly, so we can't check it",
            false,
        );
        Ok(())
    }

    /// The body of the range tests: `atLeast(1)` runs of
    /// `doTestNormsVersusDocValues` with the density `density` gives (drawn
    /// per run, as Java's argument is) and the supplier `supplier` builds
    /// over `r`.
    fn range_test(
        &self,
        sparse: bool,
        supplier: fn(Arc<dyn randomizedtesting::jdk::random::Random>) -> LongSupplier<'static>,
    ) -> Result<()> {
        if sparse {
            LuceneTestCase::assume_true(
                "Requires sparse norms support",
                self.codec_supports_sparsity(),
            );
        }
        let iterations = LuceneTestCase::at_least(1);
        let r = random();
        for _ in 0..iterations {
            let density = if sparse { random().next_double() } else { 1.0 };
            let mut longs = supplier(Arc::clone(&r));
            self.do_test_norms_versus_doc_values(density, &mut longs)?;
        }
        Ok(())
    }

    /// `testByteRange()`.
    fn test_byte_range(&self) -> Result<()> {
        self.range_test(false, |r| {
            Box::new(move || TestUtil::next_long(&*r, BYTE_MIN, BYTE_MAX))
        })
    }

    /// `testSparseByteRange()`.
    fn test_sparse_byte_range(&self) -> Result<()> {
        self.range_test(true, |r| {
            Box::new(move || TestUtil::next_long(&*r, BYTE_MIN, BYTE_MAX))
        })
    }

    /// `testShortRange()`.
    fn test_short_range(&self) -> Result<()> {
        self.range_test(false, |r| {
            Box::new(move || TestUtil::next_long(&*r, SHORT_MIN, SHORT_MAX))
        })
    }

    /// `testSparseShortRange()`.
    fn test_sparse_short_range(&self) -> Result<()> {
        self.range_test(true, |r| {
            Box::new(move || TestUtil::next_long(&*r, SHORT_MIN, SHORT_MAX))
        })
    }

    /// `testLongRange()`.
    fn test_long_range(&self) -> Result<()> {
        self.range_test(false, |r| {
            Box::new(move || TestUtil::next_long(&*r, i64::MIN, i64::MAX))
        })
    }

    /// `testSparseLongRange()`.
    fn test_sparse_long_range(&self) -> Result<()> {
        self.range_test(true, |r| {
            Box::new(move || TestUtil::next_long(&*r, i64::MIN, i64::MAX))
        })
    }

    /// `testFullLongRange()`.
    fn test_full_long_range(&self) -> Result<()> {
        self.range_test(false, full_long_range)
    }

    /// `testSparseFullLongRange()`.
    fn test_sparse_full_long_range(&self) -> Result<()> {
        self.range_test(true, full_long_range)
    }

    /// `testFewValues()`.
    fn test_few_values(&self) -> Result<()> {
        self.range_test(false, |r| {
            Box::new(move || if r.next_boolean() { 20 } else { 3 })
        })
    }

    /// `testFewSparseValues()`.
    fn test_few_sparse_values(&self) -> Result<()> {
        self.range_test(true, |r| {
            Box::new(move || if r.next_boolean() { 20 } else { 3 })
        })
    }

    /// `testFewLargeValues()`.
    fn test_few_large_values(&self) -> Result<()> {
        self.range_test(false, |r| {
            Box::new(move || if r.next_boolean() { 1000000 } else { -5000 })
        })
    }

    /// `testFewSparseLargeValues()`.
    fn test_few_sparse_large_values(&self) -> Result<()> {
        self.range_test(true, |r| {
            Box::new(move || if r.next_boolean() { 1000000 } else { -5000 })
        })
    }

    /// `testAllZeros()`.
    fn test_all_zeros(&self) -> Result<()> {
        let iterations = LuceneTestCase::at_least(1);
        for _ in 0..iterations {
            self.do_test_norms_versus_doc_values(1.0, &mut || 0)?;
        }
        Ok(())
    }

    /// `testSparseAllZeros()`.
    fn test_sparse_all_zeros(&self) -> Result<()> {
        LuceneTestCase::assume_true(
            "Requires sparse norms support",
            self.codec_supports_sparsity(),
        );
        let iterations = LuceneTestCase::at_least(1);
        for _ in 0..iterations {
            self.do_test_norms_versus_doc_values(random().next_double(), &mut || 0)?;
        }
        Ok(())
    }

    /// `testMostZeros()`.
    fn test_most_zeros(&self) -> Result<()> {
        self.range_test(false, |r| {
            Box::new(move || {
                if r.next_int_bound(100) == 0 {
                    TestUtil::next_long(&*r, BYTE_MIN, BYTE_MAX)
                } else {
                    0
                }
            })
        })
    }

    /// The body of `testOutliers` (`sparse` false) and `testSparseOutliers`.
    fn outliers_test(&self, sparse: bool) -> Result<()> {
        if sparse {
            LuceneTestCase::assume_true(
                "Requires sparse norms support",
                self.codec_supports_sparsity(),
            );
        }
        let iterations = LuceneTestCase::at_least(1);
        let r = random();
        for _ in 0..iterations {
            let common_value = TestUtil::next_long(&*r, BYTE_MIN, BYTE_MAX);
            let density = if sparse { random().next_double() } else { 1.0 };
            let rr = Arc::clone(&r);
            self.do_test_norms_versus_doc_values(density, &mut move || {
                if rr.next_int_bound(100) == 0 {
                    TestUtil::next_long(&*rr, BYTE_MIN, BYTE_MAX)
                } else {
                    common_value
                }
            })?;
        }
        Ok(())
    }

    /// `testOutliers()`.
    fn test_outliers(&self) -> Result<()> {
        self.outliers_test(false)
    }

    /// `testSparseOutliers()`.
    fn test_sparse_outliers(&self) -> Result<()> {
        self.outliers_test(true)
    }

    /// The body of `testOutliers2` (`sparse` false) and
    /// `testSparseOutliers2`.
    fn outliers2_test(&self, sparse: bool) -> Result<()> {
        if sparse {
            LuceneTestCase::assume_true(
                "Requires sparse norms support",
                self.codec_supports_sparsity(),
            );
        }
        let iterations = LuceneTestCase::at_least(1);
        let r = random();
        for _ in 0..iterations {
            let common_value = TestUtil::next_long(&*r, BYTE_MIN, BYTE_MAX);
            let uncommon_value = TestUtil::next_long(&*r, BYTE_MIN, BYTE_MAX);
            let density = if sparse { random().next_double() } else { 1.0 };
            let rr = Arc::clone(&r);
            self.do_test_norms_versus_doc_values(density, &mut move || {
                if rr.next_int_bound(100) == 0 {
                    uncommon_value
                } else {
                    common_value
                }
            })?;
        }
        Ok(())
    }

    /// `testOutliers2()`.
    fn test_outliers2(&self) -> Result<()> {
        self.outliers2_test(false)
    }

    /// `testSparseOutliers2()`.
    fn test_sparse_outliers2(&self) -> Result<()> {
        self.outliers2_test(true)
    }

    /// The body of `testNCommon` (`sparse` false) and `testSparseNCommon`.
    fn n_common_test(&self, sparse: bool) -> Result<()> {
        if sparse {
            LuceneTestCase::assume_true(
                "Requires sparse norms support",
                self.codec_supports_sparsity(),
            );
        }
        let r = random();
        let n = TestUtil::next_int(&*r, 2, 15);
        let (common_values, other_values) = n_common_values(&*r, n);
        let density = if sparse { random().next_double() } else { 1.0 };
        let mut longs = n_common_supplier(Arc::clone(&r), common_values, other_values);
        self.do_test_norms_versus_doc_values(density, &mut longs)
    }

    /// `testNCommon()`.
    fn test_n_common(&self) -> Result<()> {
        self.n_common_test(false)
    }

    /// `testSparseNCommon()`.
    fn test_sparse_n_common(&self) -> Result<()> {
        self.n_common_test(true)
    }

    /// The body of `testNCommonBig` (`sparse` false) and
    /// `testSparseNCommonBig`: a more thorough n-common that tests all low
    /// bpv (and sparse docs).
    fn n_common_big_test(&self, sparse: bool) -> Result<()> {
        if sparse {
            LuceneTestCase::assume_true(
                "Requires sparse norms support",
                self.codec_supports_sparsity(),
            );
        }
        let iterations = LuceneTestCase::at_least(1);
        let r = random();
        for _ in 0..iterations {
            // 16 is 4 bpv, the max before we jump to 8bpv
            for n in 2..16 {
                let (common_values, other_values) = n_common_values(&*r, n);
                let density = if sparse { random().next_double() } else { 1.0 };
                let mut longs = n_common_supplier(Arc::clone(&r), common_values, other_values);
                self.do_test_norms_versus_doc_values(density, &mut longs)?;
            }
        }
        Ok(())
    }

    /// `testNCommonBig()` (`@Nightly`).
    fn test_n_common_big(&self) -> Result<()> {
        self.n_common_big_test(false)
    }

    /// `testSparseNCommonBig()` (`@Nightly`).
    fn test_sparse_n_common_big(&self) -> Result<()> {
        self.n_common_big_test(true)
    }

    /// `doTestNormsVersusDocValues(density, longs)` (`private`).
    fn do_test_norms_versus_doc_values(
        &self,
        density: f64,
        longs: &mut dyn FnMut() -> i64,
    ) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(500);
        let num_docs_with_field = 1.max((density * f64::from(num_docs)) as i32);
        let docs_with_field = random_docs_with_field(num_docs, num_docs_with_field);
        let norms: Vec<i64> = (0..num_docs_with_field).map(|_| longs()).collect();

        let mut dir = LuceneTestCase::new_directory();
        self.apply_created_version_major(&*dir.as_directory())?;
        let analyzer = whitespace_analyzer();
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
        let sim = Arc::new(CannedNormSimilarity::new(norms.clone()));
        conf.set_similarity(Some(sim))?;
        let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
        add_norms_documents(&writer, num_docs, &docs_with_field, &norms)?;

        // delete some docs
        let num_deletions = random().next_int_bound(num_docs / 20);
        for _ in 0..num_deletions {
            let id = random().next_int_bound(num_docs);
            writer.delete_documents(Term::new(
                "id",
                BytesRef::from_char_sequence(&id.to_string()),
            ))?;
        }

        writer.commit()?;

        // compare
        let ir =
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
        check_norms_vs_doc_values(&(Arc::clone(&ir) as Arc<dyn IndexReader>))?;
        ir.close()?;
        drop(ir);

        writer.force_merge(1)?;

        // compare again
        let ir =
            self.maybe_wrap_with_merging_reader(directory_reader::open(dir.as_directory())?)?;
        check_norms_vs_doc_values(&(Arc::clone(&ir) as Arc<dyn IndexReader>))?;

        writer.close()?;
        drop(writer);
        ir.close()?;
        drop(ir);
        dir.close()
    }

    /// `testUndeadNorms()`: LUCENE-6006: tests undead norms.
    fn test_undead_norms(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        self.apply_created_version_major(&*dir.as_directory())?;
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let num_docs = LuceneTestCase::at_least(500);
        let mut to_delete: Vec<i32> = Vec::new();
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
            if random().next_int_bound(5) == 1 {
                to_delete.push(i);
                doc.add(Box::new(TextField::new(
                    "content",
                    "some content".to_string(),
                    Store::NO,
                )?));
            }
            w.add_document(&doc)?;
        }
        for id in to_delete {
            w.delete_documents(Term::new(
                "id",
                BytesRef::from_char_sequence(&id.to_string()),
            ))?;
        }
        w.force_merge(1)?;
        let r: Arc<dyn IndexReader> = self.maybe_wrap_with_merging_reader(w.get_reader()?)?;
        Assert::assert_false(r.has_deletions());

        // Confusingly, norms should exist, and should all be 0, even though we deleted all docs that
        // had the field "content".  They should not
        // be undead:
        let norms = MultiDocValues::get_norm_values(&r, "content")?;
        Assert::assert_true(norms.is_some());
        let mut norms = npe(norms);
        if self.codec_supports_sparsity() {
            Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(norms.next_doc()?));
        } else {
            for i in 0..r.max_doc() {
                Assert::assert_equals(i64::from(i), i64::from(norms.next_doc()?));
                Assert::assert_equals(0, norms.long_value()?);
            }
        }

        drop(norms);
        r.close()?;
        drop(r);
        w.close()?;
        drop(w);
        dir.close()
    }

    /// `testThreads()`.
    fn test_threads(&self) -> Result<()> {
        let density: f32 = if !self.codec_supports_sparsity() || random().next_boolean() {
            1.0
        } else {
            random().next_float()
        };
        let num_docs = LuceneTestCase::at_least(500);
        let num_docs_with_field = 1.max((density * num_docs as f32) as i32);
        let docs_with_field = random_docs_with_field(num_docs, num_docs_with_field);

        let norms: Vec<i64> = (0..num_docs_with_field)
            .map(|_| random().next_long())
            .collect();

        let mut dir = LuceneTestCase::new_directory();
        self.apply_created_version_major(&*dir.as_directory())?;
        let analyzer = whitespace_analyzer();
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
        conf.set_merge_policy(Some(NoMergePolicy::instance()))?;
        conf.set_similarity(Some(Arc::new(CannedNormSimilarity::new(norms.clone()))))?;
        let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
        add_norms_documents(&writer, num_docs, &docs_with_field, &norms)?;

        let reader: Arc<dyn DirectoryReader> =
            self.maybe_wrap_with_merging_reader(writer.get_reader()?)?;
        writer.close()?;
        drop(writer);

        let num_threads = TestUtil::next_int(&*random(), 3, 30);
        let latch = Arc::new(Latch::new());
        let mut threads: Vec<Thread> = Vec::with_capacity(num_threads as usize);
        for _ in 0..num_threads {
            let (reader, latch) = (Arc::clone(&reader), Arc::clone(&latch));
            threads.push(Thread::unnamed(move || {
                latch.wait();
                let reader = reader as Arc<dyn IndexReader>;
                let checked = check_norms_vs_doc_values(&reader)
                    .and_then(|()| TestUtil::check_reader(&reader));
                if let Err(e) = checked {
                    // `throw new RuntimeException(e)`
                    let (class, message) =
                        crate::analysis::base_token_stream_test_case::java_class_and_message(&e);
                    Throwable::new("java.lang.RuntimeException", format!("{class}: {message}"))
                        .throw()
                }
            }));
        }

        for thread in &threads {
            thread.start();
        }
        latch.count_down();
        for thread in &threads {
            if let Err(payload) = thread.join() {
                std::panic::resume_unwind(payload);
            }
        }

        reader.close()?;
        drop(reader);
        dir.close()
    }

    /// `testIndependantIterators()`.
    fn test_independant_iterators(&self) -> Result<()> {
        independant_iterators(self, false)
    }

    /// `testIndependantSparseIterators()`.
    fn test_independant_sparse_iterators(&self) -> Result<()> {
        independant_iterators(self, true)
    }
}

/// The `LongSupplier` of `testFullLongRange` and `testSparseFullLongRange`.
fn full_long_range(r: Arc<dyn randomizedtesting::jdk::random::Random>) -> LongSupplier<'static> {
    Box::new(move || {
        let thing_to_do = r.next_int_bound(3);
        match thing_to_do {
            0 => i64::MIN,
            1 => i64::MAX,
            _ => TestUtil::next_long(&*r, i64::MIN, i64::MAX),
        }
    })
}

/// The body of `testIndependantIterators` (`sparse` false) and
/// `testIndependantSparseIterators`: three documents with the norms 42, 10
/// and 20 (each followed by an empty document when `sparse`), read by two
/// interleaved iterators.
fn independant_iterators<C: BaseNormsFormatTestCase + ?Sized>(
    case: &C,
    sparse: bool,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config();
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let sim = Arc::new(CannedNormSimilarity::new(vec![42, 10, 20]));
    conf.set_similarity(Some(sim))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "indexed",
        "a".to_string(),
        Store::NO,
    )?));
    let empty_doc = Document::new();
    for _ in 0..3 {
        writer.add_document(&doc)?;
        if sparse {
            writer.add_document(&empty_doc)?;
        }
    }
    writer.force_merge(1)?;
    let r = LuceneTestCase::get_only_leaf_reader(
        case.maybe_wrap_with_merging_reader(writer.get_reader()?)? as Arc<dyn IndexReader>,
    )?;
    let mut n1 = npe(r.get_norm_values("indexed")?);
    let mut n2 = npe(r.get_norm_values("indexed")?);
    let (d0, d1, d2) = if sparse { (0, 2, 4) } else { (0, 1, 2) };
    Assert::assert_equals(d0, n1.next_doc()?);
    Assert::assert_equals(42, n1.long_value()?);
    Assert::assert_equals(d1, n1.next_doc()?);
    Assert::assert_equals(10, n1.long_value()?);
    Assert::assert_equals(d0, n2.next_doc()?);
    Assert::assert_equals(42, n2.long_value()?);
    Assert::assert_equals(d1, n2.next_doc()?);
    Assert::assert_equals(10, n2.long_value()?);
    Assert::assert_equals(d2, n2.next_doc()?);
    Assert::assert_equals(20, n2.long_value()?);
    Assert::assert_equals(d2, n1.next_doc()?);
    Assert::assert_equals(20, n1.long_value()?);
    Assert::assert_equals(NO_MORE_DOCS, n1.next_doc()?);
    Assert::assert_equals(NO_MORE_DOCS, n2.next_doc()?);
    drop((n1, n2));
    // `IOUtils.close(r, writer, dir)`
    Arc::clone(&r).as_index_reader().close()?;
    drop(r);
    writer.close()?;
    drop(writer);
    dir.close()
}
