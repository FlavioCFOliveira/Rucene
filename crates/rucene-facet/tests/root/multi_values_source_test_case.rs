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

//! Port of `org.apache.lucene.facet.MultiValuesSourceTestCase` (`public
//! abstract class MultiValuesSourceTestCase extends LuceneTestCase`): the
//! instance state and `setUp`/`tearDown` shared by
//! `TestMultiLongValuesSource` and `TestMultiDoubleValuesSource`.

use std::sync::{Arc, LazyLock};

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, DoubleDocValuesField, FloatDocValuesField, NumericDocValuesField,
    SortedNumericDocValuesField,
};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{NumericDocValues, SortedNumericDocValues};
use rucene::search::double_values::DoubleValues;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::long_values::LongValues;
use rucene_facet::multi_double_values::MultiDoubleValues;
use rucene_facet::multi_long_values::MultiLongValues;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::abstract_class;

/// The class `MultiValuesSourceTestCase`.
pub fn multi_values_source_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        abstract_class(
            "org.apache.lucene.facet.MultiValuesSourceTestCase",
            &LuceneTestCase::class(),
        )
    });
    Arc::clone(&CLASS)
}

/// The instance fields `dir`, `reader`, `searcher`.
#[derive(Default)]
pub struct MultiValuesSourceTestCase {
    pub dir: Option<TestDirectory>,
    pub reader: Option<Arc<dyn DirectoryReader>>,
    pub searcher: Option<IndexSearcher>,
}

impl MultiValuesSourceTestCase {
    /// `reader`.
    pub fn reader(&self) -> &Arc<dyn DirectoryReader> {
        self.reader.as_ref().expect("reader set by setUp")
    }

    /// `searcher`.
    pub fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("searcher set by setUp")
    }
}

/// `setUp()`.
pub fn set_up(f: &mut MultiValuesSourceTestCase) -> Result<()> {
    let random = LuceneTestCase::random();
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random, dir.as_directory())?;
    let num_docs = RandomNumbers::random_int_between(&*random, 100, 1000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        if random.next_int_bound(10) < 8 {
            doc.add(NumericDocValuesField::new(
                "single_int",
                i64::from(random.next_int()),
            ));
        }
        if random.next_int_bound(10) < 8 {
            doc.add(NumericDocValuesField::new(
                "single_long",
                random.next_long(),
            ));
        }
        if random.next_int_bound(10) < 8 {
            doc.add(FloatDocValuesField::new(
                "single_float",
                random.next_float(),
            ));
        }
        if random.next_int_bound(10) < 8 {
            doc.add(DoubleDocValuesField::new(
                "single_double",
                random.next_double(),
            ));
        }
        let mut limit = RandomNumbers::random_int_between(&*random, 0, 100);
        for _ in 0..limit {
            doc.add(SortedNumericDocValuesField::new(
                "multi_int",
                i64::from(random.next_int()),
            ));
        }
        limit = RandomNumbers::random_int_between(&*random, 0, 100);
        for _ in 0..limit {
            doc.add(SortedNumericDocValuesField::new(
                "multi_long",
                random.next_long(),
            ));
        }
        limit = RandomNumbers::random_int_between(&*random, 0, 100);
        for _ in 0..limit {
            doc.add(SortedNumericDocValuesField::new(
                "multi_float",
                i64::from(random.next_float().to_bits() as i32),
            ));
        }
        limit = RandomNumbers::random_int_between(&*random, 0, 100);
        for _ in 0..limit {
            doc.add(SortedNumericDocValuesField::new(
                "multi_double",
                random.next_double().to_bits() as i64,
            ));
        }
        iw.add_document(&doc)?;
        if i % 100 == 0 && random.next_boolean() {
            iw.commit()?;
        }
    }
    let reader = iw.get_reader()?;
    iw.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    Ok(())
}

/// `tearDown()`.
pub fn tear_down(f: &mut MultiValuesSourceTestCase) -> Result<()> {
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// `validateFieldBasedSource(NumericDocValues, LongValues, int)`.
pub fn validate_numeric_long(
    doc_values: &mut dyn NumericDocValues,
    values: &mut dyn LongValues,
    max_doc: i32,
) -> Result<()> {
    for doc in 0..max_doc {
        let has_values = doc_values.advance_exact(doc)?;
        Assert::assert_equals(has_values, values.advance_exact(doc)?);
        if has_values {
            Assert::assert_equals(doc_values.long_value()?, values.long_value()?);
        }
    }
    Ok(())
}

/// `validateFieldBasedSource(SortedNumericDocValues, MultiLongValues, int)`.
pub fn validate_sorted_long(
    doc_values: &mut dyn SortedNumericDocValues,
    values: &mut dyn MultiLongValues,
    max_doc: i32,
) -> Result<()> {
    for doc in 0..max_doc {
        let has_values = doc_values.advance_exact(doc)?;
        Assert::assert_equals(has_values, values.advance_exact(doc)?);
        if has_values {
            let value_count = doc_values.doc_value_count()?;
            Assert::assert_equals(i64::from(value_count), values.get_value_count()?);
            for _ in 0..value_count {
                Assert::assert_equals(doc_values.next_value()?, values.next_value()?);
            }
        }
    }
    Ok(())
}

/// `validateFieldBasedSource(NumericDocValues, DoubleValues, int)` (unused
/// by the Java subclasses too).
#[allow(dead_code)]
pub fn validate_numeric_double(
    doc_values: &mut dyn NumericDocValues,
    values: &mut dyn DoubleValues,
    max_doc: i32,
) -> Result<()> {
    for doc in 0..max_doc {
        let has_values = doc_values.advance_exact(doc)?;
        Assert::assert_equals(has_values, values.advance_exact(doc)?);
        if has_values {
            Assert::assert_equals_double_msg(
                None,
                doc_values.long_value()? as f64,
                values.double_value()?,
                0.00001,
            );
        }
    }
    Ok(())
}

/// `validateFieldBasedSource(SortedNumericDocValues, MultiDoubleValues,
/// int)`.
pub fn validate_sorted_double(
    doc_values: &mut dyn SortedNumericDocValues,
    values: &mut dyn MultiDoubleValues,
    max_doc: i32,
) -> Result<()> {
    for doc in 0..max_doc {
        let has_values = doc_values.advance_exact(doc)?;
        Assert::assert_equals(has_values, values.advance_exact(doc)?);
        if has_values {
            let value_count = doc_values.doc_value_count()?;
            Assert::assert_equals(i64::from(value_count), values.get_value_count()?);
            for _ in 0..value_count {
                Assert::assert_equals_double_msg(
                    None,
                    doc_values.next_value()? as f64,
                    values.next_value()?,
                    0.00001,
                );
            }
        }
    }
    Ok(())
}

/// `validateFieldBasedSource(NumericDocValues, DoubleValues, int, boolean
/// useDoublePrecision)`.
pub fn validate_numeric_double_precision(
    doc_values: &mut dyn NumericDocValues,
    values: &mut dyn DoubleValues,
    max_doc: i32,
    use_double_precision: bool,
) -> Result<()> {
    for doc in 0..max_doc {
        let has_values = doc_values.advance_exact(doc)?;
        Assert::assert_equals(has_values, values.advance_exact(doc)?);
        if has_values {
            if use_double_precision {
                let as_long = values.double_value()?.to_bits() as i64;
                Assert::assert_equals(doc_values.long_value()?, as_long);
            } else {
                let as_int = (values.double_value()? as f32).to_bits() as i32;
                Assert::assert_equals(doc_values.long_value()? as i32, as_int);
            }
        }
    }
    Ok(())
}

/// `validateFieldBasedSource(SortedNumericDocValues, MultiDoubleValues, int,
/// boolean useDoublePrecision)`.
pub fn validate_sorted_double_precision(
    doc_values: &mut dyn SortedNumericDocValues,
    values: &mut dyn MultiDoubleValues,
    max_doc: i32,
    use_double_precision: bool,
) -> Result<()> {
    for doc in 0..max_doc {
        let has_values = doc_values.advance_exact(doc)?;
        Assert::assert_equals(has_values, values.advance_exact(doc)?);
        if has_values {
            let value_count = doc_values.doc_value_count()?;
            Assert::assert_equals(i64::from(value_count), values.get_value_count()?);
            for _ in 0..value_count {
                if use_double_precision {
                    let as_long = values.next_value()?.to_bits() as i64;
                    Assert::assert_equals(doc_values.next_value()?, as_long);
                } else {
                    let as_int = (values.next_value()? as f32).to_bits() as i32;
                    Assert::assert_equals(doc_values.next_value()?, i64::from(as_int));
                }
            }
        }
    }
    Ok(())
}
