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

//! Port of `org.apache.lucene.tests.index.BaseCompressingDocValuesFormatTestCase`:
//! extends `BaseDocValuesFormatTestCase` to add compression checks.
//!
//! The class is declared by [`class()`] (extending
//! [`base_doc_values_format_test_case::class()`](super::base_doc_values_format_test_case::class))
//! and its members are the trait [`BaseCompressingDocValuesFormatTestCase`];
//! see [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergence** (declared): the shared `NumericDocValuesField dvf` Java
//! adds to every document and updates between additions is a field of the
//! same name and value built for each document (see the module docs of
//! `legacy_base_doc_values_format_test_case`).

use std::sync::{Arc, LazyLock};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class};
use rucene::document::{Document, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::packed::PackedInts;

use super::base_doc_values_format_test_case::BaseDocValuesFormatTestCase;
use super::base_index_file_format_test_case::{format_test, BaseIndexFileFormatTestCase};
use crate::analysis::MockAnalyzer;
use crate::hamcrest::{less_than, MatcherAssert};
use crate::util::lucene_test_case::LuceneTestCase;
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BaseCompressingDocValuesFormatTestCase`
/// (`public abstract class BaseCompressingDocValuesFormatTestCase extends
/// BaseDocValuesFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t =
            |name: &str, body: fn(&dyn BaseCompressingDocValuesFormatTestCase) -> Result<()>| {
                format_test(name, move |c| body(compressing_case(c)))
            };
        Class::builder("org.apache.lucene.tests.index.BaseCompressingDocValuesFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_doc_values_format_test_case::class())
            .method(t("testUniqueValuesCompression", |c| {
                c.test_unique_values_compression()
            }))
            .method(t("testDateCompression", |c| c.test_date_compression()))
            .method(t("testSingleBigValueCompression", |c| {
                c.test_single_big_value_compression()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseCompressingDocValuesFormatTestCase`.
fn compressing_case(
    c: &dyn BaseIndexFileFormatTestCase,
) -> &dyn BaseCompressingDocValuesFormatTestCase {
    c.as_doc_values()
        .and_then(|c| c.as_compressing_doc_values())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.IllegalArgumentException",
                "object is not an instance of org.apache.lucene.tests.index.BaseCompressingDocValuesFormatTestCase",
            )
            .throw()
        })
}

/// `dirSize(d)` (`static`).
fn dir_size(d: &dyn Directory) -> Result<i64> {
    let mut size = 0;
    for file in d.list_all()? {
        size += d.file_length(&file)?;
    }
    Ok(size)
}

/// A document holding `dvf` with the value `value`.
fn dv_doc(value: i64) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("dv", value)));
    doc
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_config() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )))
}

/// Port of the members of `BaseCompressingDocValuesFormatTestCase`.
pub trait BaseCompressingDocValuesFormatTestCase: BaseDocValuesFormatTestCase {
    /// `testUniqueValuesCompression()`.
    fn test_unique_values_compression(&self) -> Result<()> {
        let directory = Arc::new(ByteBuffersDirectory::new());
        let outcome = (|| -> Result<()> {
            let iwc = mock_config();
            let iwriter = IndexWriter::new(Arc::clone(&directory) as Arc<dyn Directory>, iwc)?;

            let unique_value_count = TestUtil::next_int(&*LuceneTestCase::random(), 1, 256);
            let mut values: Vec<i64> = Vec::new();

            for _ in 0..300 {
                let value = if (values.len() as i32) < unique_value_count {
                    let value = LuceneTestCase::random().next_long();
                    values.push(value);
                    value
                } else {
                    *RandomPicks::random_from_list(&*LuceneTestCase::random(), &values)
                };
                iwriter.add_document(&dv_doc(value))?;
            }
            iwriter.force_merge(1)?;
            let size1 = dir_size(&*directory)?;
            for _ in 0..20 {
                let value = *RandomPicks::random_from_list(&*LuceneTestCase::random(), &values);
                iwriter.add_document(&dv_doc(value))?;
            }
            iwriter.force_merge(1)?;
            let size2 = dir_size(&*directory)?;
            // make sure the new longs did not cost 8 bytes each
            MatcherAssert::assert_that(&size2, &less_than(size1 + 8 * 20));
            abandon(iwriter)
        })();
        close_directory(directory, outcome)
    }

    /// `testDateCompression()`.
    fn test_date_compression(&self) -> Result<()> {
        let directory = Arc::new(ByteBuffersDirectory::new());
        let outcome = (|| -> Result<()> {
            let iwc = mock_config();
            let iwriter = IndexWriter::new(Arc::clone(&directory) as Arc<dyn Directory>, iwc)?;

            let base: i64 = 13; // prime
            let day: i64 = 1000 * 60 * 60 * 24;

            for _ in 0..300 {
                let value = base + i64::from(LuceneTestCase::random().next_int_bound(1000)) * day;
                iwriter.add_document(&dv_doc(value))?;
            }
            iwriter.force_merge(1)?;
            let size1 = dir_size(&*directory)?;
            for _ in 0..50 {
                let value = base + i64::from(LuceneTestCase::random().next_int_bound(1000)) * day;
                iwriter.add_document(&dv_doc(value))?;
            }
            iwriter.force_merge(1)?;
            let size2 = dir_size(&*directory)?;
            // make sure the new longs costed less than if they had only been packed
            let bits = i64::from(PackedInts::bits_required(day)?);
            MatcherAssert::assert_that(&size2, &less_than(size1 + (bits * 50) / 8));
            abandon(iwriter)
        })();
        close_directory(directory, outcome)
    }

    /// `testSingleBigValueCompression()`.
    fn test_single_big_value_compression(&self) -> Result<()> {
        let directory = Arc::new(ByteBuffersDirectory::new());
        let outcome = (|| -> Result<()> {
            let iwc = mock_config();
            let iwriter = IndexWriter::new(Arc::clone(&directory) as Arc<dyn Directory>, iwc)?;

            for i in 0..20000 {
                iwriter.add_document(&dv_doc(i & 1023))?;
            }
            iwriter.force_merge(1)?;
            let size1 = dir_size(&*directory)?;
            iwriter.add_document(&dv_doc(i64::MAX))?;
            iwriter.force_merge(1)?;
            let size2 = dir_size(&*directory)?;
            // make sure the new value did not grow the bpv for every other value
            MatcherAssert::assert_that(&size2, &less_than(size1 + (20000 * (63 - 10)) / 8));
            abandon(iwriter)
        })();
        close_directory(directory, outcome)
    }
}

/// Java never closes `iwriter`: the test abandons it and the JVM collects
/// it. A dropped, unclosed port writer keeps the directory referenced, so
/// the abandoned writer is rolled back (no commit, no further files), which
/// releases it before the directory closes.
fn abandon(iwriter: Arc<IndexWriter>) -> Result<()> {
    iwriter.rollback()
}

/// The try-with-resources of the directory: closed on every exit, the
/// body's failure first.
fn close_directory(directory: Arc<ByteBuffersDirectory>, outcome: Result<()>) -> Result<()> {
    let closed = match Arc::try_unwrap(directory) {
        Ok(mut directory) => directory.close(),
        Err(_) => Err(rucene::error::LuceneError::illegal_state(
            "the directory is still referenced".to_string(),
        )),
    };
    outcome?;
    closed
}
