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

//! Port of `org.apache.lucene.tests.analysis.CollationTestBase`: base test
//! class for testing Unicode collation.
//!
//! The class's four methods take the analyzer and the collated range bounds
//! from a subclass, which passes the sort keys its collator gives the Farsi
//! range endpoints. Lucene 10.5.0 has no such subclass in `lucene-core`:
//! they live in `analysis/common` (`TestCollationDocValuesField`'s
//! neighbours, `TestCollationKeyAnalyzer`) and `analysis/icu`
//! (`TestICUCollationKeyAnalyzer`), outside this port. The methods are
//! exercised over a core analyzer in this crate's tests.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::Store;
use rucene::document::{Document, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::{BooleanQueryBuilder, IndexSearcher, Query, TermQuery, TermRangeQuery};
use rucene::util::BytesRef;

use super::attributes::{term_to_bytes_ref_id, Att};
use crate::util::{LuceneTestCase, TestUtil};

/// Port of `org.apache.lucene.tests.analysis.CollationTestBase`.
///
/// Java's protected fields are the Farsi range endpoints a subclass collates
/// into the `BytesRef` bounds it passes to the test methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollationTestBase {
    /// `firstRangeBeginningOriginal`, `"د"`.
    pub first_range_beginning_original: String,
    /// `firstRangeEndOriginal`, `"ژ"`.
    pub first_range_end_original: String,
    /// `secondRangeBeginningOriginal`, `"س"`.
    pub second_range_beginning_original: String,
    /// `secondRangeEndOriginal`, `"ظ"`.
    pub second_range_end_original: String,
}

impl Default for CollationTestBase {
    fn default() -> Self {
        Self {
            first_range_beginning_original: "\u{062F}".to_string(),
            first_range_end_original: "\u{0698}".to_string(),
            second_range_beginning_original: "\u{0633}".to_string(),
            second_range_end_original: "\u{0638}".to_string(),
        }
    }
}

/// `new TermRangeQuery("content", beg, end, true, true)`.
fn content_range(beg: &BytesRef, end: &BytesRef) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(TermRangeQuery::new(
        "content",
        Some(beg.clone()),
        Some(end.clone()),
        true,
        true,
    )?))
}

/// The single document of the Farsi tests: `content` holds the term
/// `"ساب"`, and `body` the term `body` unless `body` is
/// `false`.
fn farsi_document(body: bool) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "content",
        "\u{0633}\u{0627}\u{0628}".to_string(),
        Store::YES,
    )?));
    if body {
        doc.add(Box::new(StringField::new(
            "body",
            "body".to_string(),
            Store::YES,
        )?));
    }
    Ok(doc)
}

impl CollationTestBase {
    /// `testFarsiRangeFilterCollating(analyzer, firstBeg, firstEnd,
    /// secondBeg, secondEnd)`: a `TermQuery` on `body` filtered by a
    /// `TermRangeQuery` on `content` matches nothing in the first range and
    /// the document in the second.
    ///
    /// Unicode order would include U+0633 in [ U+062F - U+0698 ], but Farsi
    /// orders the U+0698 character before the U+0633 character, so the single
    /// index Term below should NOT be returned by a TermRangeFilter with a
    /// Farsi Collator (or an Arabic one for the case when Farsi searcher not
    /// supported).
    ///
    /// # Errors
    ///
    /// Returns what indexing, searching and closing return.
    ///
    /// # Panics
    ///
    /// `AssertionError` when a range matches otherwise.
    pub fn test_farsi_range_filter_collating(
        &self,
        analyzer: Arc<dyn Analyzer>,
        first_beg: &BytesRef,
        first_end: &BytesRef,
        second_beg: &BytesRef,
        second_end: &BytesRef,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = IndexWriter::new(
            dir.as_directory(),
            IndexWriterConfig::with_analyzer(Some(analyzer)),
        )?;
        writer.add_document(&farsi_document(true)?)?;
        writer.close()?;
        drop(writer);
        let reader = directory_reader::open(dir.as_directory())?;
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("body", "body")));

        let mut bq = BooleanQueryBuilder::new();
        bq.add(Arc::clone(&query), Occur::MUST)?;
        bq.add(content_range(first_beg, first_end)?, Occur::FILTER)?;
        let result = searcher.search(Arc::new(bq.build()), 1)?.score_docs;
        Assert::assert_equals_msg(
            Some("The index Term should not be included."),
            0,
            result.len(),
        );

        let mut bq = BooleanQueryBuilder::new();
        bq.add(Arc::clone(&query), Occur::MUST)?;
        bq.add(content_range(second_beg, second_end)?, Occur::FILTER)?;
        let result = searcher.search(Arc::new(bq.build()), 1)?.score_docs;
        Assert::assert_equals_msg(Some("The index Term should be included."), 1, result.len());

        drop(searcher);
        reader.close()?;
        drop(reader);
        dir.close()
    }

    /// `testFarsiRangeQueryCollating(analyzer, firstBeg, firstEnd, secondBeg,
    /// secondEnd)`: a `TermRangeQuery` on `content` matches nothing in the
    /// first range and the document in the second.
    ///
    /// # Errors
    ///
    /// Returns what indexing, searching and closing return.
    ///
    /// # Panics
    ///
    /// `AssertionError` when a range matches otherwise.
    pub fn test_farsi_range_query_collating(
        &self,
        analyzer: Arc<dyn Analyzer>,
        first_beg: &BytesRef,
        first_end: &BytesRef,
        second_beg: &BytesRef,
        second_end: &BytesRef,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let writer = IndexWriter::new(
            dir.as_directory(),
            IndexWriterConfig::with_analyzer(Some(analyzer)),
        )?;
        // Unicode order would include U+0633 in [ U+062F - U+0698 ], but Farsi
        // orders the U+0698 character before the U+0633 character, so the
        // single index Term below should NOT be returned by a TermRangeQuery
        // with a Farsi Collator (or an Arabic one for the case when Farsi is
        // not supported).
        writer.add_document(&farsi_document(false)?)?;
        writer.close()?;
        drop(writer);
        let reader = directory_reader::open(dir.as_directory())?;
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

        let hits = searcher
            .search(content_range(first_beg, first_end)?, 1000)?
            .score_docs;
        Assert::assert_equals_msg(
            Some("The index Term should not be included."),
            0,
            hits.len(),
        );

        let hits = searcher
            .search(content_range(second_beg, second_end)?, 1000)?
            .score_docs;
        Assert::assert_equals_msg(Some("The index Term should be included."), 1, hits.len());

        drop(searcher);
        reader.close()?;
        drop(reader);
        dir.close()
    }

    /// `testFarsiTermRangeQuery(analyzer, firstBeg, firstEnd, secondBeg,
    /// secondEnd)`: as
    /// [`test_farsi_range_query_collating`](Self::test_farsi_range_query_collating),
    /// over a document that also has a `body`, searched through
    /// `newSearcher(reader)`.
    ///
    /// # Errors
    ///
    /// Returns what indexing, searching and closing return.
    ///
    /// # Panics
    ///
    /// `AssertionError` when a range matches otherwise.
    pub fn test_farsi_term_range_query(
        &self,
        analyzer: Arc<dyn Analyzer>,
        first_beg: &BytesRef,
        first_end: &BytesRef,
        second_beg: &BytesRef,
        second_end: &BytesRef,
    ) -> Result<()> {
        let mut farsi_index = LuceneTestCase::new_directory();
        let writer = IndexWriter::new(
            farsi_index.as_directory(),
            IndexWriterConfig::with_analyzer(Some(analyzer)),
        )?;
        writer.add_document(&farsi_document(true)?)?;
        writer.close()?;
        drop(writer);

        let reader = directory_reader::open(farsi_index.as_directory())?;
        let search = LuceneTestCase::new_searcher(Arc::clone(&reader));

        // Unicode order would include U+0633 in [ U+062F - U+0698 ], but Farsi
        // orders the U+0698 character before the U+0633 character, so the
        // single index Term below should NOT be returned by a TermRangeQuery
        // with a Farsi Collator (or an Arabic one for the case when Farsi is
        // not supported).
        let result = search
            .search(content_range(first_beg, first_end)?, 1000)?
            .score_docs;
        Assert::assert_equals_msg(
            Some("The index Term should not be included."),
            0,
            result.len(),
        );

        let result = search
            .search(content_range(second_beg, second_end)?, 1000)?
            .score_docs;
        Assert::assert_equals_msg(Some("The index Term should be included."), 1, result.len());

        drop(search);
        reader.close()?;
        drop(reader);
        farsi_index.close()
    }

    /// `assertThreadSafe(analyzer)`: the sort keys (the single term's
    /// `TermToBytesRefAttribute` bytes) of 100 random simple strings,
    /// computed serially, are computed again, identically, by 3 to 5 threads
    /// at once.
    ///
    /// The threads are `java.lang.Thread`s of the runner's emulation: a
    /// failure in one reaches the uncaught exception handler, as in Java.
    ///
    /// # Errors
    ///
    /// Returns what the analysis returns on the calling thread.
    ///
    /// # Panics
    ///
    /// `AssertionError` when a string does not analyze to exactly one token.
    pub fn assert_thread_safe(&self, analyzer: Arc<dyn Analyzer>) -> Result<()> {
        let num_test_points = 100;
        let num_threads = TestUtil::next_int(&*LuceneTestCase::random(), 3, 5);
        let mut map: HashMap<String, BytesRef> = HashMap::new();

        // create a map<String,SortKey> up front.
        // then with multiple threads, generate sort keys for all the keys in the map
        // and ensure they are the same as the ones we produced in serial fashion.

        for _ in 0..num_test_points {
            let term = TestUtil::random_simple_string(&*LuceneTestCase::random());
            let bytes = single_term_bytes(&*analyzer, &term)?;
            // ensure we make a copy of the actual bytes too
            map.insert(term, BytesRef::new(bytes.slice().to_vec()));
        }

        let map = Arc::new(map);
        let threads: Vec<Thread> = (0..num_threads)
            .map(|_| {
                let analyzer = Arc::clone(&analyzer);
                let map = Arc::clone(&map);
                Thread::unnamed(move || {
                    for (term, expected) in map.iter() {
                        match single_term_bytes(&*analyzer, term) {
                            Ok(actual) => Assert::assert_equals(expected.clone(), actual),
                            // `catch (IOException e) { throw new RuntimeException(e); }`
                            Err(e) => {
                                Throwable::new("java.lang.RuntimeException", e.to_string()).throw()
                            }
                        }
                    }
                })
            })
            .collect();
        for thread in &threads {
            thread.start();
        }
        for thread in &threads {
            let _ = thread.join();
        }
        Ok(())
    }
}

/// `analyzer.tokenStream("fake", term)` reset, its single token's
/// `TermToBytesRefAttribute.getBytesRef()`, then `end()` and `close()`.
fn single_term_bytes(analyzer: &dyn Analyzer, term: &str) -> Result<BytesRef> {
    let ts = analyzer.token_stream_from_str("fake", term)?;
    let mut ts = ts.borrow_mut();
    let term_att = Att::add(ts.attribute_source_mut(), term_to_bytes_ref_id())?;
    let result = (|| -> Result<BytesRef> {
        ts.reset()?;
        Assert::assert_true(ts.increment_token()?);
        let bytes = term_att.term_to_bytes_ref().get_bytes_ref();
        Assert::assert_false(ts.increment_token()?);
        ts.end()?;
        Ok(bytes)
    })();
    // try-with-resources
    let closed = ts.close();
    let bytes = result?;
    closed?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_endpoints_are_lucenes() {
        let base = CollationTestBase::default();
        assert_eq!(base.first_range_beginning_original, "\u{062F}");
        assert_eq!(base.first_range_end_original, "\u{0698}");
        assert_eq!(base.second_range_beginning_original, "\u{0633}");
        assert_eq!(base.second_range_end_original, "\u{0638}");
    }
}
