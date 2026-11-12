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

//! Port of `org.apache.lucene.tests.index.BaseDocValuesFormatTestCase`:
//! extends `LegacyBaseDocValuesFormatTestCase` and adds checks for
//! `DocValuesSkipper`.
//!
//! The class is declared by [`class()`] (extending
//! [`legacy_base_doc_values_format_test_case::class()`](super::legacy_base_doc_values_format_test_case::class))
//! and its members are the trait [`BaseDocValuesFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `SortedDocValues.termsEnum()` / `SortedSetDocValues.termsEnum()`: see
//!   the module docs of `legacy_base_doc_values_format_test_case`.
//! * `assertEquals(expectedBitSet, bitSet)` (`doTestRandomIntoBitSet`):
//!   `FixedBitSet.equals`, printed as `Object.toString()` prints it (the
//!   class name, `@` and the hex `hashCode()`).

use std::fmt;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader;
use rucene::index::doc_values::DocValuesSkipper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::SeekStatus;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::{BytesRef, FixedBitSet};

use super::base_index_file_format_test_case::{format_test, BaseIndexFileFormatTestCase};
use super::legacy_base_doc_values_format_test_case::{
    aeq, assert_not_null, assert_seek_status, merge_away_all_values,
    merge_away_all_values_large_segment, nbr, npe, random, LegacyBaseDocValuesFormatTestCase,
};
use crate::index::random_index_writer::RandomIndexWriter;
use crate::index::MismatchedCodecReader;
use crate::util::lucene_test_case::{LuceneTestCase, Nightly};
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BaseDocValuesFormatTestCase`
/// (`public abstract class BaseDocValuesFormatTestCase extends
/// LegacyBaseDocValuesFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseDocValuesFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(base_doc_values_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseDocValuesFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::legacy_base_doc_values_format_test_case::class())
            .method(t("testSortedMergeAwayAllValuesWithSkipper", |c| {
                c.test_sorted_merge_away_all_values_with_skipper()
            }))
            .method(t("testSortedSetMergeAwayAllValuesWithSkipper", |c| {
                c.test_sorted_set_merge_away_all_values_with_skipper()
            }))
            .method(t("testNumberMergeAwayAllValuesWithSkipper", |c| {
                c.test_number_merge_away_all_values_with_skipper()
            }))
            .method(t("testSortedNumberMergeAwayAllValuesWithSkipper", |c| {
                c.test_sorted_number_merge_away_all_values_with_skipper()
            }))
            .method(t(
                "testSortedMergeAwayAllValuesLargeSegmentWithSkipper",
                |c| c.test_sorted_merge_away_all_values_large_segment_with_skipper(),
            ))
            .method(t(
                "testSortedSetMergeAwayAllValuesLargeSegmentWithSkipper",
                |c| c.test_sorted_set_merge_away_all_values_large_segment_with_skipper(),
            ))
            .method(t(
                "testNumericMergeAwayAllValuesLargeSegmentWithSkipper",
                |c| c.test_numeric_merge_away_all_values_large_segment_with_skipper(),
            ))
            .method(t(
                "testSortedNumericMergeAwayAllValuesLargeSegmentWithSkipper",
                |c| c.test_sorted_numeric_merge_away_all_values_large_segment_with_skipper(),
            ))
            .method(t("testMaxValueCountWithSkipper", |c| {
                c.test_max_value_count_with_skipper()
            }))
            .method(t("testNumericDocValuesWithSkipperSmall", |c| {
                c.test_numeric_doc_values_with_skipper_small()
            }))
            .method(t("testNumericDocValuesWithSkipperMedium", |c| {
                c.test_numeric_doc_values_with_skipper_medium()
            }))
            .method(
                t("testNumericDocValuesWithSkipperBig", |c| {
                    c.test_numeric_doc_values_with_skipper_big()
                })
                .annotate(Nightly),
            )
            .method(t("testSortedNumericDocValuesWithSkipperSmall", |c| {
                c.test_sorted_numeric_doc_values_with_skipper_small()
            }))
            .method(t("testSortedNumericDocValuesWithSkipperMedium", |c| {
                c.test_sorted_numeric_doc_values_with_skipper_medium()
            }))
            .method(
                t("testSortedNumericDocValuesWithSkipperBig", |c| {
                    c.test_sorted_numeric_doc_values_with_skipper_big()
                })
                .annotate(Nightly),
            )
            .method(t("testSortedDocValuesWithSkipperSmall", |c| {
                c.test_sorted_doc_values_with_skipper_small()
            }))
            .method(t("testSortedDocValuesWithSkipperMedium", |c| {
                c.test_sorted_doc_values_with_skipper_medium()
            }))
            .method(
                t("testSortedDocValuesWithSkipperBig", |c| {
                    c.test_sorted_doc_values_with_skipper_big()
                })
                .annotate(Nightly),
            )
            .method(t("testSortedSetDocValuesWithSkipperSmall", |c| {
                c.test_sorted_set_doc_values_with_skipper_small()
            }))
            .method(t("testSortedSetDocValuesWithSkipperMedium", |c| {
                c.test_sorted_set_doc_values_with_skipper_medium()
            }))
            .method(
                t("testSortedSetDocValuesWithSkipperBig", |c| {
                    c.test_sorted_set_doc_values_with_skipper_big()
                })
                .annotate(Nightly),
            )
            .method(t("testMismatchedFields", |c| c.test_mismatched_fields()))
            .method(t("testRandomDenseNumericIntoBitSet", |c| {
                c.test_random_dense_numeric_into_bit_set()
            }))
            .method(t("testRandomSparseNumericIntoBitSet", |c| {
                c.test_random_sparse_numeric_into_bit_set()
            }))
            .method(t("testRandomDenseSortedNumericIntoBitSet", |c| {
                c.test_random_dense_sorted_numeric_into_bit_set()
            }))
            .method(t("testRandomSparseSortedNumericIntoBitSet", |c| {
                c.test_random_sparse_sorted_numeric_into_bit_set()
            }))
            .method(t("testRandomDenseSortedIntoBitSet", |c| {
                c.test_random_dense_sorted_into_bit_set()
            }))
            .method(t("testRandomSparseSortedIntoBitSet", |c| {
                c.test_random_sparse_sorted_into_bit_set()
            }))
            .method(t("testRandomDenseSortedSetIntoBitSet", |c| {
                c.test_random_dense_sorted_set_into_bit_set()
            }))
            .method(t("testRandomSparseSortedSetIntoBitSet", |c| {
                c.test_random_sparse_sorted_set_into_bit_set()
            }))
            .method(t("testRandomDenseBinaryIntoBitSet", |c| {
                c.test_random_dense_binary_into_bit_set()
            }))
            .method(t("testRandomSparseBinaryIntoBitSet", |c| {
                c.test_random_sparse_binary_into_bit_set()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseDocValuesFormatTestCase`.
fn base_doc_values_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseDocValuesFormatTestCase {
    c.as_doc_values()
        .and_then(|c| c.as_base_doc_values())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.IllegalArgumentException",
                "object is not an instance of org.apache.lucene.tests.index.BaseDocValuesFormatTestCase",
            )
            .throw()
        })
}

/// The skipper checks every `*MergeAwayAllValues*WithSkipper` test makes on
/// a field without values left.
fn check_empty_skipper(leaf: &dyn LeafReader) -> Result<()> {
    let mut skipper = npe(leaf.get_doc_values_skipper("field")?);
    aeq(0, skipper.global_doc_count());
    aeq(0, skipper.max_value_count());
    skipper.advance(0)?;
    aeq(NO_MORE_DOCS, skipper.min_doc_id(0));
    Ok(())
}

/// The terms checks of the sorted `*MergeAwayAllValues*WithSkipper` tests.
fn check_sorted_terms(leaf: &dyn LeafReader, set: bool) -> Result<()> {
    let lucene = BytesRef::from_char_sequence("lucene");
    let mut terms_enum = if set {
        npe(leaf.get_sorted_set_doc_values("field")?).terms_enum()?
    } else {
        npe(leaf.get_sorted_doc_values("field")?).terms_enum()?
    };
    Assert::assert_false(terms_enum.seek_exact(&lucene)?);
    assert_seek_status(SeekStatus::END, terms_enum.seek_ceil(&lucene)?);
    if set {
        aeq(
            -1,
            npe(leaf.get_sorted_set_doc_values("field")?).lookup_term(&lucene)?,
        );
    } else {
        aeq(
            -1,
            npe(leaf.get_sorted_doc_values("field")?).lookup_term(&lucene)?,
        );
    }
    Ok(())
}

/// Port of `BaseDocValuesFormatTestCase.DocValuesWrapper`.
trait DocValuesWrapper {
    fn advance(&mut self, target: i32) -> Result<i32>;

    fn advance_exact(&mut self, target: i32) -> Result<bool>;

    fn max_value(&self) -> Result<i64>;

    fn min_value(&self) -> Result<i64>;

    fn doc_id(&self) -> i32;

    fn doc_value_count(&self) -> Result<i32>;
}

/// The `docValuesWrapper(leafReader)` member of a [`TestDocValueSkipper`].
type DocValuesWrapperFn = fn(&dyn LeafReader) -> Result<Option<Box<dyn DocValuesWrapper>>>;

/// Port of `BaseDocValuesFormatTestCase.TestDocValueSkipper`.
struct TestDocValueSkipper {
    populate_doc: fn(&mut Document),
    /// `docValuesWrapper(leafReader)`. Java's wrapper captures the values
    /// the reader answers, `null` included, and dereferences them only when
    /// called; `None` stands for that `null`, dereferenced (`npe`) where the
    /// Java wrapper is first called.
    doc_values_wrapper: DocValuesWrapperFn,
}

impl TestDocValueSkipper {
    /// `docValuesSkipper(leafReader)`: every implementation answers
    /// `leafReader.getDocValuesSkipper("test")`.
    fn doc_values_skipper(
        &self,
        leaf_reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        leaf_reader.get_doc_values_skipper("test")
    }
}

/// The `DocValuesWrapper` of `doTestNumericDocValuesWithSkipper`.
struct NumericWrapper(Box<dyn rucene::index::doc_values::NumericDocValues>);

impl DocValuesWrapper for NumericWrapper {
    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.advance(target)
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.0.advance_exact(target)
    }

    fn max_value(&self) -> Result<i64> {
        self.0.long_value()
    }

    fn min_value(&self) -> Result<i64> {
        self.0.long_value()
    }

    fn doc_id(&self) -> i32 {
        self.0.doc_id()
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(1)
    }
}

/// The `DocValuesWrapper` of `doTestSortedDocValuesWithSkipper`.
struct SortedWrapper(Box<dyn rucene::index::doc_values::SortedDocValues>);

impl DocValuesWrapper for SortedWrapper {
    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.advance(target)
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.0.advance_exact(target)
    }

    fn max_value(&self) -> Result<i64> {
        Ok(i64::from(self.0.ord_value()?))
    }

    fn min_value(&self) -> Result<i64> {
        Ok(i64::from(self.0.ord_value()?))
    }

    fn doc_id(&self) -> i32 {
        self.0.doc_id()
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(1)
    }
}

/// The `DocValuesWrapper` of `doTestSortedNumericDocValuesWithSkipper`.
struct SortedNumericWrapper {
    values: Box<dyn rucene::index::doc_values::SortedNumericDocValues>,
    max: i64,
    min: i64,
}

impl SortedNumericWrapper {
    /// `readValues()` (`private`).
    fn read_values(&mut self) -> Result<()> {
        self.max = i64::MIN;
        self.min = i64::MAX;
        for _ in 0..self.values.doc_value_count()? {
            let value = self.values.next_value()?;
            self.max = self.max.max(value);
            self.min = self.min.min(value);
        }
        Ok(())
    }
}

impl DocValuesWrapper for SortedNumericWrapper {
    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.values.advance(target)?;
        if doc != NO_MORE_DOCS {
            self.read_values()?;
        }
        Ok(doc)
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.values.advance_exact(target)? {
            self.read_values()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn max_value(&self) -> Result<i64> {
        Ok(self.max)
    }

    fn min_value(&self) -> Result<i64> {
        Ok(self.min)
    }

    fn doc_id(&self) -> i32 {
        self.values.doc_id()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.values.doc_value_count()
    }
}

/// The `DocValuesWrapper` of `doTestSortedSetDocValuesWithSkipper`.
struct SortedSetWrapper {
    values: Box<dyn rucene::index::doc_values::SortedSetDocValues>,
    max: i64,
    min: i64,
}

impl SortedSetWrapper {
    /// `readValues()` (`private`).
    fn read_values(&mut self) -> Result<()> {
        self.max = i64::MIN;
        self.min = i64::MAX;
        for _ in 0..self.values.doc_value_count()? {
            let value = self.values.next_ord()?;
            self.max = self.max.max(value);
            self.min = self.min.min(value);
        }
        Ok(())
    }
}

impl DocValuesWrapper for SortedSetWrapper {
    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.values.advance(target)?;
        if doc != NO_MORE_DOCS {
            self.read_values()?;
        }
        Ok(doc)
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.values.advance_exact(target)? {
            self.read_values()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn max_value(&self) -> Result<i64> {
        Ok(self.max)
    }

    fn min_value(&self) -> Result<i64> {
        Ok(self.min)
    }

    fn doc_id(&self) -> i32 {
        self.values.doc_id()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.values.doc_value_count()
    }
}

/// The numeric `TestDocValueSkipper`.
fn numeric_skipper() -> TestDocValueSkipper {
    TestDocValueSkipper {
        populate_doc: |doc| {
            doc.add(Box::new(NumericDocValuesField::indexed_field(
                "test",
                random().next_long(),
            )));
        },
        doc_values_wrapper: |leaf| {
            Ok(leaf
                .get_numeric_doc_values("test")?
                .map(|v| Box::new(NumericWrapper(v)) as Box<dyn DocValuesWrapper>))
        },
    }
}

/// The sorted-numeric `TestDocValueSkipper`.
fn sorted_numeric_skipper() -> TestDocValueSkipper {
    TestDocValueSkipper {
        populate_doc: |doc| {
            let mut j = 0;
            while j < random().next_int_range(1, 5) {
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "test",
                    random().next_long(),
                )));
                j += 1;
            }
        },
        doc_values_wrapper: |leaf| {
            Ok(leaf.get_sorted_numeric_doc_values("test")?.map(|values| {
                Box::new(SortedNumericWrapper {
                    values,
                    max: 0,
                    min: 0,
                }) as Box<dyn DocValuesWrapper>
            }))
        },
    }
}

/// The sorted `TestDocValueSkipper`.
fn sorted_skipper() -> TestDocValueSkipper {
    TestDocValueSkipper {
        populate_doc: |doc| {
            doc.add(Box::new(SortedDocValuesField::indexed_field(
                "test",
                TestUtil::random_binary_term(&*random()),
            )));
        },
        doc_values_wrapper: |leaf| {
            Ok(leaf
                .get_sorted_doc_values("test")?
                .map(|v| Box::new(SortedWrapper(v)) as Box<dyn DocValuesWrapper>))
        },
    }
}

/// The sorted-set `TestDocValueSkipper`.
fn sorted_set_skipper() -> TestDocValueSkipper {
    TestDocValueSkipper {
        populate_doc: |doc| {
            let mut j = 0;
            while j < random().next_int_range(1, 5) {
                doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                    "test",
                    TestUtil::random_binary_term(&*random()),
                )));
                j += 1;
            }
        },
        doc_values_wrapper: |leaf| {
            Ok(leaf.get_sorted_set_doc_values("test")?.map(|values| {
                Box::new(SortedSetWrapper {
                    values,
                    max: 0,
                    min: 0,
                }) as Box<dyn DocValuesWrapper>
            }))
        },
    }
}

/// `assertDocValuesWithSkipper(totalDocs, testDocValueSkipper)` (`private`).
fn assert_doc_values_with_skipper(
    total_docs: i32,
    test_doc_value_skipper: &TestDocValueSkipper,
) -> Result<()> {
    let boolean_supplier: fn() -> bool = match random().next_int_bound(3) {
        0 => || true,
        1 => || random().next_boolean(),
        2 => || random().next_boolean() && random().next_boolean(),
        _ => randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError"),
    };
    let directory = LuceneTestCase::new_directory();
    {
        let writer =
            RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
        let mut num_docs = 0;
        for _ in 0..total_docs {
            let mut doc = Document::new();
            if boolean_supplier() {
                (test_doc_value_skipper.populate_doc)(&mut doc);
                num_docs += 1;
            }
            writer.add_document(&doc)?;
            if LuceneTestCase::rarely() {
                writer.commit()?;
            }
        }
        writer.flush()?;

        if random().next_boolean() {
            writer.force_merge(1)?;
        }

        let r: Arc<dyn IndexReader> = writer.get_reader()?;
        let mut read_docs = 0;
        for reader_context in Arc::clone(&r).leaves()? {
            let reader = reader_context.leaf_reader();
            // `new PrintStream(bos, false, UTF_8)` over a `ByteArrayOutputStream`.
            let info_stream = rucene::util::ByteArrayPrintStream::new();
            let codec_reader = Arc::clone(&reader).as_codec_reader().ok_or_else(|| {
                LuceneError::from(LuceneErrorKind::Unchecked {
                    class: "java.lang.ClassCastException",
                    message: Some("the leaf is not a CodecReader".to_string()),
                    cause: None,
                })
            })?;
            let status = rucene::index::check_index::test_doc_values(
                &*codec_reader,
                Some(&info_stream),
                true,
            )?;
            if let Some(error) = status.error {
                return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                    class: "java.lang.Exception",
                    message: Some(error.to_string()),
                    cause: Some(Arc::new(error)),
                }));
            }
            read_docs += assert_doc_values_skip_sequential(
                (test_doc_value_skipper.doc_values_wrapper)(&*reader)?,
                test_doc_value_skipper.doc_values_skipper(&*reader)?,
            )?;
            for _ in 0..10 {
                assert_doc_values_skip_random(
                    (test_doc_value_skipper.doc_values_wrapper)(&*reader)?,
                    test_doc_value_skipper.doc_values_skipper(&*reader)?,
                    reader.max_doc(),
                )?;
            }
        }
        aeq(num_docs, read_docs);
        // `IOUtils.close(r, writer, directory)`
        r.close()?;
        drop(r);
        writer.close()?;
        drop(writer);
    }
    directory.close()
}

/// `assertDocValuesSkipSequential(iterator, skipper)` (`private`).
fn assert_doc_values_skip_sequential(
    iterator: Option<Box<dyn DocValuesWrapper>>,
    skipper: Option<Box<dyn DocValuesSkipper>>,
) -> Result<i32> {
    let Some(mut skipper) = skipper else {
        return Ok(0);
    };
    let mut iterator = npe(iterator);

    aeq(-1, iterator.doc_id());
    aeq(-1, skipper.min_doc_id(0));
    aeq(-1, skipper.max_doc_id(0));

    iterator.advance(0)?;
    let mut doc_count = 0;
    let mut max_value_count = 0;
    loop {
        let previous_max_doc = skipper.max_doc_id(0);
        skipper.advance(previous_max_doc + 1)?;
        Assert::assert_true(skipper.min_doc_id(0) > previous_max_doc);
        aeq(iterator.doc_id(), skipper.min_doc_id(0));

        if skipper.min_doc_id(0) == NO_MORE_DOCS {
            aeq(NO_MORE_DOCS, skipper.max_doc_id(0));
            break;
        }
        Assert::assert_true(skipper.doc_count(0) > 0);

        let mut max_doc = -1;
        let mut min_val = i64::MAX;
        let mut max_val = i64::MIN;
        for _ in 0..skipper.doc_count(0) {
            assert_not_equals(i64::from(NO_MORE_DOCS), i64::from(iterator.doc_id()));
            max_doc = max_doc.max(iterator.doc_id());
            min_val = min_val.min(iterator.min_value()?);
            max_val = max_val.max(iterator.max_value()?);
            max_value_count = max_value_count.max(iterator.doc_value_count()?);
            let next = iterator.doc_id() + 1;
            iterator.advance(next)?;
        }
        aeq(max_doc, skipper.max_doc_id(0));
        aeq(min_val, skipper.min_value(0));
        aeq(max_val, skipper.max_value(0));
        doc_count += skipper.doc_count(0);
        for level in 1..skipper.num_levels() {
            Assert::assert_true(skipper.min_doc_id(0) >= skipper.min_doc_id(level));
            Assert::assert_true(skipper.max_doc_id(0) <= skipper.max_doc_id(level));
            Assert::assert_true(skipper.min_value(0) >= skipper.min_value(level));
            Assert::assert_true(skipper.max_value(0) <= skipper.max_value(level));
            Assert::assert_true(skipper.doc_count(0) < skipper.doc_count(level));
        }
    }

    aeq(doc_count, skipper.global_doc_count());
    aeq(max_value_count, skipper.max_value_count());
    Ok(doc_count)
}

/// `assertNotEquals(unexpected, actual)` over two longs: `Values should be
/// different. Actual: <actual>`.
fn assert_not_equals(unexpected: i64, actual: i64) {
    if unexpected == actual {
        Assert::fail(&format!("Values should be different. Actual: {actual}"));
    }
}

/// `assertDocValuesSkipRandom(iterator, skipper, maxDoc)` (`private
/// static`).
fn assert_doc_values_skip_random(
    iterator: Option<Box<dyn DocValuesWrapper>>,
    skipper: Option<Box<dyn DocValuesSkipper>>,
    max_doc: i32,
) -> Result<()> {
    let Some(mut skipper) = skipper else {
        return Ok(());
    };
    let mut iterator = npe(iterator);
    let mut next_level = 0;
    loop {
        let doc = random().next_int_range(skipper.max_doc_id(next_level), max_doc + 1) + 1;
        skipper.advance(doc)?;
        if skipper.min_doc_id(0) == NO_MORE_DOCS {
            aeq(NO_MORE_DOCS, skipper.max_doc_id(0));
            return Ok(());
        }
        if iterator.advance_exact(doc)? {
            for level in 0..skipper.num_levels() {
                Assert::assert_true(iterator.doc_id() >= skipper.min_doc_id(level));
                Assert::assert_true(iterator.doc_id() <= skipper.max_doc_id(level));
                Assert::assert_true(iterator.min_value()? >= skipper.min_value(level));
                Assert::assert_true(iterator.max_value()? <= skipper.max_value(level));
            }
        }
        next_level = random().next_int_bound(skipper.num_levels());
    }
}

/// A `FixedBitSet` as JUnit compares and prints it: `equals`, and
/// `Object.toString()` (`FixedBitSet` overrides `hashCode()` only).
#[derive(PartialEq)]
struct JavaFixedBitSet(FixedBitSet);

impl fmt::Display for JavaFixedBitSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "org.apache.lucene.util.FixedBitSet@{:x}",
            self.0.hash_code()
        )
    }
}

/// A `Supplier<Boolean>` of `doTestRandomIntoBitSet`.
type AddDocument<'a> = &'a dyn Fn() -> bool;

/// An `IOFunction<LeafReader, DocIdSetIterator>` of
/// `doTestRandomIntoBitSet`.
type Producer<'a> = &'a dyn Fn(&dyn LeafReader) -> Result<Option<Box<dyn DocIdSetIterator>>>;

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `BaseDocValuesFormatTestCase`.
pub trait BaseDocValuesFormatTestCase: LegacyBaseDocValuesFormatTestCase {
    /// `testSortedMergeAwayAllValuesWithSkipper()`.
    fn test_sorted_merge_away_all_values_with_skipper(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(SortedDocValuesField::indexed_field("field", nbr("hello")))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                check_empty_skipper(leaf)?;
                check_sorted_terms(leaf, false)
            },
        )
    }

    /// `testSortedSetMergeAwayAllValuesWithSkipper()`.
    fn test_sorted_set_merge_away_all_values_with_skipper(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| {
                Box::new(SortedSetDocValuesField::indexed_field(
                    "field",
                    nbr("hello"),
                ))
            }),
            |leaf| {
                let dv = npe(leaf.get_sorted_set_doc_values("field")?);
                aeq(0, dv.get_value_count()?);
                check_empty_skipper(leaf)?;
                check_sorted_terms(leaf, true)
            },
        )
    }

    /// `testNumberMergeAwayAllValuesWithSkipper()`.
    fn test_number_merge_away_all_values_with_skipper(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(NumericDocValuesField::indexed_field("field", 5))),
            |leaf| {
                let mut dv = npe(leaf.get_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                check_empty_skipper(leaf)
            },
        )
    }

    /// `testSortedNumberMergeAwayAllValuesWithSkipper()`.
    fn test_sorted_number_merge_away_all_values_with_skipper(&self) -> Result<()> {
        merge_away_all_values(
            Box::new(|| Box::new(SortedNumericDocValuesField::indexed_field("field", 5))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                check_empty_skipper(leaf)
            },
        )
    }

    /// `testSortedMergeAwayAllValuesLargeSegmentWithSkipper()`.
    fn test_sorted_merge_away_all_values_large_segment_with_skipper(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(SortedDocValuesField::indexed_field("field", nbr("hello")))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                check_empty_skipper(leaf)?;
                check_sorted_terms(leaf, false)
            },
        )
    }

    /// `testSortedSetMergeAwayAllValuesLargeSegmentWithSkipper()`.
    fn test_sorted_set_merge_away_all_values_large_segment_with_skipper(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| {
                Box::new(SortedSetDocValuesField::indexed_field(
                    "field",
                    nbr("hello"),
                ))
            }),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_set_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                check_empty_skipper(leaf)?;
                check_sorted_terms(leaf, true)
            },
        )
    }

    /// `testNumericMergeAwayAllValuesLargeSegmentWithSkipper()`.
    fn test_numeric_merge_away_all_values_large_segment_with_skipper(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(NumericDocValuesField::indexed_field("field", 42))),
            |leaf| {
                let mut dv = npe(leaf.get_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                check_empty_skipper(leaf)
            },
        )
    }

    /// `testSortedNumericMergeAwayAllValuesLargeSegmentWithSkipper()`.
    fn test_sorted_numeric_merge_away_all_values_large_segment_with_skipper(&self) -> Result<()> {
        merge_away_all_values_large_segment(
            Box::new(|| Box::new(SortedNumericDocValuesField::indexed_field("field", 42))),
            |leaf| {
                let mut dv = npe(leaf.get_sorted_numeric_doc_values("field")?);
                aeq(NO_MORE_DOCS, dv.next_doc()?);
                check_empty_skipper(leaf)
            },
        )
    }

    /// `testMaxValueCountWithSkipper()`.
    fn test_max_value_count_with_skipper(&self) -> Result<()> {
        let directory = LuceneTestCase::new_directory();
        {
            let writer =
                RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;

            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::indexed_field("numeric", 1)));
            doc.add(Box::new(SortedDocValuesField::indexed_field(
                "sorted",
                nbr("a"),
            )));
            doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                "sorted_numeric",
                1,
            )));
            doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                "sorted_set",
                nbr("a"),
            )));
            writer.add_document(&doc)?;

            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::indexed_field("numeric", 2)));
            doc.add(Box::new(SortedDocValuesField::indexed_field(
                "sorted",
                nbr("b"),
            )));
            doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                "sorted_numeric",
                1,
            )));
            doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                "sorted_numeric",
                2,
            )));
            doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                "sorted_numeric",
                3,
            )));
            doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                "sorted_set",
                nbr("a"),
            )));
            doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                "sorted_set",
                nbr("b"),
            )));
            writer.add_document(&doc)?;

            writer.force_merge(1)?;
            let reader = writer.get_reader()?;
            writer.close()?;
            drop(writer);

            let leaf_reader = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
            aeq(
                1,
                npe(leaf_reader.get_doc_values_skipper("numeric")?).max_value_count(),
            );
            aeq(
                1,
                npe(leaf_reader.get_doc_values_skipper("sorted")?).max_value_count(),
            );
            aeq(
                3,
                npe(leaf_reader.get_doc_values_skipper("sorted_numeric")?).max_value_count(),
            );
            aeq(
                2,
                npe(leaf_reader.get_doc_values_skipper("sorted_set")?).max_value_count(),
            );

            drop(leaf_reader);
            reader.close()?;
        }
        directory.close()
    }

    /// `testNumericDocValuesWithSkipperSmall()`.
    fn test_numeric_doc_values_with_skipper_small(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(1, 1000), &numeric_skipper())
    }

    /// `testNumericDocValuesWithSkipperMedium()`.
    fn test_numeric_doc_values_with_skipper_medium(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(1000, 20000), &numeric_skipper())
    }

    /// `testNumericDocValuesWithSkipperBig()` (`@Nightly`).
    fn test_numeric_doc_values_with_skipper_big(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(50000, 100000), &numeric_skipper())
    }

    /// `testSortedNumericDocValuesWithSkipperSmall()`.
    fn test_sorted_numeric_doc_values_with_skipper_small(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(1, 1000), &sorted_numeric_skipper())
    }

    /// `testSortedNumericDocValuesWithSkipperMedium()`.
    fn test_sorted_numeric_doc_values_with_skipper_medium(&self) -> Result<()> {
        assert_doc_values_with_skipper(
            random().next_int_range(1000, 20000),
            &sorted_numeric_skipper(),
        )
    }

    /// `testSortedNumericDocValuesWithSkipperBig()` (`@Nightly`).
    fn test_sorted_numeric_doc_values_with_skipper_big(&self) -> Result<()> {
        assert_doc_values_with_skipper(
            random().next_int_range(50000, 100000),
            &sorted_numeric_skipper(),
        )
    }

    /// `testSortedDocValuesWithSkipperSmall()`.
    fn test_sorted_doc_values_with_skipper_small(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(1, 1000), &sorted_skipper())
    }

    /// `testSortedDocValuesWithSkipperMedium()`.
    fn test_sorted_doc_values_with_skipper_medium(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(1000, 20000), &sorted_skipper())
    }

    /// `testSortedDocValuesWithSkipperBig()` (`@Nightly`).
    fn test_sorted_doc_values_with_skipper_big(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(50000, 100000), &sorted_skipper())
    }

    /// `testSortedSetDocValuesWithSkipperSmall()`.
    fn test_sorted_set_doc_values_with_skipper_small(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(1, 1000), &sorted_set_skipper())
    }

    /// `testSortedSetDocValuesWithSkipperMedium()`.
    fn test_sorted_set_doc_values_with_skipper_medium(&self) -> Result<()> {
        assert_doc_values_with_skipper(random().next_int_range(10000, 20000), &sorted_set_skipper())
    }

    /// `testSortedSetDocValuesWithSkipperBig()` (`@Nightly`).
    fn test_sorted_set_doc_values_with_skipper_big(&self) -> Result<()> {
        assert_doc_values_with_skipper(
            random().next_int_range(50000, 100000),
            &sorted_set_skipper(),
        )
    }

    /// `testMismatchedFields()`.
    fn test_mismatched_fields(&self) -> Result<()> {
        let dir1 = LuceneTestCase::new_directory();
        // Created where Java creates it; declared here so it outlives the
        // block that releases every handle on it before it closes.
        let dir2;
        {
            {
                let w1 = IndexWriter::new(
                    dir1.as_directory(),
                    LuceneTestCase::new_index_writer_config(),
                )?;
                let mut doc = Document::new();
                doc.add(Box::new(BinaryDocValuesField::new(
                    "binary",
                    BytesRef::from_char_sequence("lucene"),
                )));
                doc.add(Box::new(NumericDocValuesField::new("numeric", 0)));
                doc.add(Box::new(SortedDocValuesField::new(
                    "sorted",
                    BytesRef::from_char_sequence("search"),
                )));
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "sorted_numeric",
                    1,
                )));
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "sorted_set",
                    BytesRef::from_char_sequence("engine"),
                )));
                w1.add_document(&doc)?;

                dir2 = LuceneTestCase::new_directory();
                let iwc2 = LuceneTestCase::new_index_writer_config();
                iwc2.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
                let w2 = IndexWriter::new(dir2.as_directory(), iwc2)?;
                w2.add_document(&doc)?;
                w2.commit()?;

                let reader = directory_reader::open_with_writer(&*w1)?;
                w1.close()?;
                drop(w1);
                let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
                let codec_reader = leaf.as_codec_reader().ok_or_else(|| {
                    LuceneError::from(LuceneErrorKind::Unchecked {
                        class: "java.lang.ClassCastException",
                        message: Some("the leaf is not a CodecReader".to_string()),
                        cause: None,
                    })
                })?;
                let mismatched = MismatchedCodecReader::new(codec_reader, &*random())?;
                w2.add_indexes_codec_readers(&[Arc::new(mismatched)])?;
                reader.close()?;
                drop(reader);
                w2.force_merge(1)?;
                let reader = directory_reader::open_with_writer(&*w2)?;
                w2.close()?;
                drop(w2);

                let leaf_reader = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;

                let bdv = leaf_reader.get_binary_doc_values("binary")?;
                assert_not_null(bdv.is_some());
                let mut bdv = npe(bdv);
                aeq(0, bdv.next_doc()?);
                Assert::assert_equals(BytesRef::from_char_sequence("lucene"), bdv.binary_value()?);
                aeq(1, bdv.next_doc()?);
                Assert::assert_equals(BytesRef::from_char_sequence("lucene"), bdv.binary_value()?);
                aeq(NO_MORE_DOCS, bdv.next_doc()?);

                let ndv = leaf_reader.get_numeric_doc_values("numeric")?;
                assert_not_null(ndv.is_some());
                let mut ndv = npe(ndv);
                aeq(0, ndv.next_doc()?);
                aeq(0, ndv.long_value()?);
                aeq(1, ndv.next_doc()?);
                aeq(0, ndv.long_value()?);
                aeq(NO_MORE_DOCS, ndv.next_doc()?);

                let sdv = leaf_reader.get_sorted_doc_values("sorted")?;
                assert_not_null(sdv.is_some());
                let mut sdv = npe(sdv);
                aeq(0, sdv.next_doc()?);
                Assert::assert_equals(
                    BytesRef::from_char_sequence("search"),
                    sdv.lookup_ord(sdv.ord_value()?)?,
                );
                aeq(1, sdv.next_doc()?);
                Assert::assert_equals(
                    BytesRef::from_char_sequence("search"),
                    sdv.lookup_ord(sdv.ord_value()?)?,
                );
                aeq(NO_MORE_DOCS, sdv.next_doc()?);

                let sndv = leaf_reader.get_sorted_numeric_doc_values("sorted_numeric")?;
                assert_not_null(sndv.is_some());
                let mut sndv = npe(sndv);
                aeq(0, sndv.next_doc()?);
                aeq(1, sndv.next_value()?);
                aeq(1, sndv.next_doc()?);
                aeq(1, sndv.next_value()?);
                aeq(NO_MORE_DOCS, sndv.next_doc()?);

                let ssdv = leaf_reader.get_sorted_set_doc_values("sorted_set")?;
                assert_not_null(ssdv.is_some());
                let mut ssdv = npe(ssdv);
                aeq(0, ssdv.next_doc()?);
                let ord = ssdv.next_ord()?;
                Assert::assert_equals(
                    BytesRef::from_char_sequence("engine"),
                    ssdv.lookup_ord(ord)?,
                );
                aeq(1, ssdv.next_doc()?);
                let ord = ssdv.next_ord()?;
                Assert::assert_equals(
                    BytesRef::from_char_sequence("engine"),
                    ssdv.lookup_ord(ord)?,
                );
                aeq(NO_MORE_DOCS, ssdv.next_doc()?);

                // `IOUtils.close(reader, w2, dir1, dir2)`
                drop((bdv, ndv, sdv, sndv, ssdv, leaf_reader));
                reader.close()?;
                drop(reader);
            }
        }
        dir1.close()?;
        dir2.close()
    }

    /// `testRandomDenseNumericIntoBitSet()`.
    fn test_random_dense_numeric_into_bit_set(&self) -> Result<()> {
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(NumericDocValuesField::new(
                    "num",
                    random().next_long(),
                )))
            },
            &|reader| {
                Ok(reader
                    .get_numeric_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| true,
        )
    }

    /// `testRandomSparseNumericIntoBitSet()`.
    fn test_random_sparse_numeric_into_bit_set(&self) -> Result<()> {
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(NumericDocValuesField::new(
                    "num",
                    random().next_long(),
                )))
            },
            &|reader| {
                Ok(reader
                    .get_numeric_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| {
                if random().next_boolean() {
                    random().next_boolean()
                } else {
                    random().next_int_bound(50) == 0
                }
            },
        )
    }

    /// `testRandomDenseSortedNumericIntoBitSet()`.
    fn test_random_dense_sorted_numeric_into_bit_set(&self) -> Result<()> {
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "num",
                    random().next_long(),
                )));
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "num",
                    random().next_long(),
                )));
            },
            &|reader| {
                Ok(reader
                    .get_sorted_numeric_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| true,
        )
    }

    /// `testRandomSparseSortedNumericIntoBitSet()`.
    fn test_random_sparse_sorted_numeric_into_bit_set(&self) -> Result<()> {
        let n = random().next_int_bound(50) + 1;
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "num",
                    random().next_long(),
                )));
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "num",
                    random().next_long(),
                )));
            },
            &|reader| {
                Ok(reader
                    .get_sorted_numeric_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| random().next_int_bound(n) == 0,
        )
    }

    /// `testRandomDenseSortedIntoBitSet()`.
    fn test_random_dense_sorted_into_bit_set(&self) -> Result<()> {
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(SortedDocValuesField::new(
                    "num",
                    BytesRef::from_char_sequence(&random().next_long().to_string()),
                )))
            },
            &|reader| {
                Ok(reader
                    .get_sorted_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| true,
        )
    }

    /// `testRandomSparseSortedIntoBitSet()`.
    fn test_random_sparse_sorted_into_bit_set(&self) -> Result<()> {
        let n = random().next_int_bound(50) + 1;
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(SortedDocValuesField::new(
                    "num",
                    BytesRef::from_char_sequence(&random().next_long().to_string()),
                )))
            },
            &|reader| {
                Ok(reader
                    .get_sorted_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| random().next_int_bound(n) == 0,
        )
    }

    /// `testRandomDenseSortedSetIntoBitSet()`.
    fn test_random_dense_sorted_set_into_bit_set(&self) -> Result<()> {
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "num",
                    BytesRef::from_char_sequence(&random().next_long().to_string()),
                )));
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "num",
                    BytesRef::from_char_sequence(&random().next_long().to_string()),
                )));
            },
            &|reader| {
                Ok(reader
                    .get_sorted_set_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| true,
        )
    }

    /// `testRandomSparseSortedSetIntoBitSet()`.
    fn test_random_sparse_sorted_set_into_bit_set(&self) -> Result<()> {
        let n = random().next_int_bound(50) + 1;
        self.do_test_random_into_bit_set(
            &|doc| {
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "num",
                    BytesRef::from_char_sequence(&random().next_long().to_string()),
                )));
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "num",
                    BytesRef::from_char_sequence(&random().next_long().to_string()),
                )));
            },
            &|reader| {
                Ok(reader
                    .get_sorted_set_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| random().next_int_bound(n) == 0,
        )
    }

    /// `testRandomDenseBinaryIntoBitSet()`.
    fn test_random_dense_binary_into_bit_set(&self) -> Result<()> {
        self.do_test_random_into_bit_set(
            &|doc| {
                let mut bytes = vec![0u8; 10];
                random().next_bytes(&mut bytes);
                doc.add(Box::new(BinaryDocValuesField::new(
                    "num",
                    BytesRef::new(bytes),
                )));
            },
            &|reader| {
                Ok(reader
                    .get_binary_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| true,
        )
    }

    /// `testRandomSparseBinaryIntoBitSet()`.
    fn test_random_sparse_binary_into_bit_set(&self) -> Result<()> {
        let n = random().next_int_bound(50) + 1;
        self.do_test_random_into_bit_set(
            &|doc| {
                let mut bytes = vec![0u8; 10];
                random().next_bytes(&mut bytes);
                doc.add(Box::new(BinaryDocValuesField::new(
                    "num",
                    BytesRef::new(bytes),
                )));
            },
            &|reader| {
                Ok(reader
                    .get_binary_doc_values("num")?
                    .map(|v| v as Box<dyn DocIdSetIterator>))
            },
            &|| random().next_int_bound(n) == 0,
        )
    }

    /// `doTestRandomIntoBitSet(consumer, producer, addDocument)` (`public`).
    fn do_test_random_into_bit_set(
        &self,
        consumer: &dyn Fn(&mut Document),
        producer: Producer<'_>,
        add_document: AddDocument<'_>,
    ) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(100);
        let mut docs_with_field = 0;
        let dir = LuceneTestCase::new_directory();
        let outcome = (|| -> Result<()> {
            let w = IndexWriter::new(
                dir.as_directory(),
                LuceneTestCase::new_index_writer_config(),
            )?;
            let indexed = (|| -> Result<()> {
                for _ in 0..num_docs {
                    let mut doc = Document::new();
                    if add_document() {
                        docs_with_field += 1;
                        consumer(&mut doc);
                    }
                    w.add_document(&doc)?;
                }
                w.commit()?;
                w.force_merge(1)?;

                let reader = directory_reader::open_with_writer(&*w)?;
                let checked = (|| -> Result<()> {
                    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
                    for _ in 0..20 {
                        let start = random().next_int_bound(num_docs - 1);
                        let up_to = random().next_int_range(start, num_docs);

                        let offset = if start == 0 {
                            0
                        } else {
                            random().next_int_bound(start)
                        };
                        let mut bit_set = FixedBitSet::new((num_docs - offset) as usize);
                        let mut expected_bit_set = FixedBitSet::new((num_docs - offset) as usize);

                        let values = producer(&*leaf)?;
                        let expected = producer(&*leaf)?;
                        if docs_with_field == 0 {
                            if values.is_some() {
                                Throwable::new(
                                    "java.lang.AssertionError",
                                    "expected null, but was:<DocIdSetIterator>",
                                )
                                .throw()
                            }
                            return Ok(()); // no more to be tested
                        }
                        assert_not_null(values.is_some());
                        let mut values = npe(values);
                        let mut expected = npe(expected);
                        values.advance(start)?;
                        expected.advance(start)?;
                        aeq(expected.doc_id(), values.doc_id());
                        if values.doc_id() == NO_MORE_DOCS {
                            continue;
                        }
                        values.into_bit_set(up_to, &mut bit_set, offset)?;
                        let mut doc = expected.doc_id();
                        while doc < up_to {
                            expected_bit_set.set((doc - offset) as usize);
                            doc = expected.next_doc()?;
                        }
                        aeq(expected.doc_id(), values.doc_id());
                        Assert::assert_equals(
                            JavaFixedBitSet(expected_bit_set),
                            JavaFixedBitSet(bit_set),
                        );
                    }
                    Ok(())
                })();
                let closed = reader.close();
                checked?;
                closed
            })();
            let closed = w.close();
            indexed?;
            closed
        })();
        let closed = dir.close();
        outcome?;
        closed
    }
}
