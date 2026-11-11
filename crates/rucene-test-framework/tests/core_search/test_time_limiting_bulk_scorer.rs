//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTimeLimitingBulkScorer.java`.
//!
//! Tests the `TimeLimitingBulkScorer`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, QueryTimeout, QueryTimeoutImpl, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::term_query::TermQuery;
use rucene::search::time_limiting_bulk_scorer::{TimeLimitingBulkScorer, INTERVAL};
use rucene::search::{BulkScorer, CollectionResult, LeafCollector, Query, NO_MORE_DOCS};
use rucene::util::{Bits, MatchAllBits};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, random, test};

/// `TestTimeLimitingBulkScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTimeLimitingBulkScorer",
        vec![
            test("testTimeLimitingBulkScorer", test_time_limiting_bulk_scorer),
            test("testExponentialRate", test_exponential_rate),
        ],
    )
}

/// Port of `TestTimeLimitingBulkScorer.testTimeLimitingBulkScorer`.
fn test_time_limiting_bulk_scorer() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    let n = 10000;
    for _i in 0..n {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_text_field(
            "default",
            "ones ",
            Store::YES,
        )));
        writer.add_document(&d)?;
    }
    writer.force_merge(1)?;
    writer.commit()?;
    writer.close()?;

    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("default", "ones")));
    let directory_reader = directory_reader::open(directory.as_directory())?;
    let mut searcher = IndexSearcher::new(Arc::clone(&directory_reader) as Arc<dyn IndexReader>)?;
    searcher.set_timeout(Some(counting_query_timeout(10)));
    let top = searcher.search(query, n)?;
    let hits = top.score_docs;
    Assert::assert_true_msg(
        "Partial result and is aborted is true",
        !hits.is_empty() && (hits.len() as i32) < n && searcher.timed_out(),
    );
    drop(searcher);
    directory_reader.close()?;
    directory.close()?;
    Ok(())
}

/// The anonymous `BulkScorer` of `testExponentialRate`.
struct RateCheckingBulkScorer {
    expected_interval: i32,
    last_max: i32,
    last_interval: i32,
}

/// `int MAX_DOCS = DocIdSetIterator.NO_MORE_DOCS - 1`.
const MAX_DOCS: i32 = NO_MORE_DOCS - 1;

impl BulkScorer for RateCheckingBulkScorer {
    fn score(
        &mut self,
        _collector: &mut dyn LeafCollector,
        _accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let difference = max - min;
        // the rate shouldn't overflow - only increase or remain equal
        Assert::assert_true_msg("Rate should only go up", difference >= self.last_interval);
        // make sure no documents end up being skipped
        Assert::assert_true_msg("Documents skipped", self.last_max == min);
        // the difference should be exactly the step except the last run where the difference is
        // smaller since there are not enough docs
        Assert::assert_true_msg(
            "Incorrect rate encountered",
            if max == MAX_DOCS {
                self.expected_interval >= difference
            } else {
                self.expected_interval == difference
            },
        );

        self.last_max = max;
        self.last_interval = difference;

        // use integer sum since the exponential growth formula yields different result due to
        // rounding
        self.expected_interval = self
            .expected_interval
            .wrapping_add(self.expected_interval / 2);
        // overflow - stop at the previous one
        if self.expected_interval < 0 {
            self.expected_interval = self.last_interval;
        }
        Ok(max)
    }

    fn cost(&self) -> i64 {
        1
    }
}

/// Port of `TestTimeLimitingBulkScorer.testExponentialRate`.
fn test_exponential_rate() -> Result<()> {
    let bulk_scorer = RateCheckingBulkScorer {
        expected_interval: INTERVAL,
        last_max: 0,
        last_interval: 0,
    };

    let mut scorer =
        TimeLimitingBulkScorer::new(Box::new(bulk_scorer), Arc::new(QueryTimeoutImpl::new(-1)));
    scorer
        .score(
            &mut dummy_collector(),
            Some(&MatchAllBits::new(i32::MAX as usize)),
            0,
            MAX_DOCS,
        )
        .map_err(|e| e.into_lucene_error())?;
    Ok(())
}

/// The anonymous `QueryTimeout` of `countingQueryTimeout`.
#[derive(Debug)]
struct CountingQueryTimeout {
    counter: AtomicI32,
    timeallowed: i32,
}

impl QueryTimeout for CountingQueryTimeout {
    fn should_exit(&self) -> bool {
        let counter = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        counter == self.timeallowed
    }
}

/// `private static QueryTimeout countingQueryTimeout(int timeallowed)`.
fn counting_query_timeout(timeallowed: i32) -> Arc<dyn QueryTimeout> {
    Arc::new(CountingQueryTimeout {
        counter: AtomicI32::new(0),
        timeallowed,
    })
}

/// The anonymous `SimpleCollector` of `dummyCollector()`.
struct DummyCollector;

impl LeafCollector for DummyCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

/// `private static LeafCollector dummyCollector()`.
fn dummy_collector() -> DummyCollector {
    DummyCollector
}
