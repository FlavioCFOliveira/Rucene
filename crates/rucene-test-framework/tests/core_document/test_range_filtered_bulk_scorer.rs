//! Port of `lucene/core/src/test/org/apache/lucene/document/TestRangeFilteredBulkScorer.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::RangeBulkScorer;
use rucene::error::Result;
use rucene::search::doc_id_set_iterator::RangeDocIdSetIterator;
use rucene::search::{
    BulkScorer, CollectionResult, DocIdSetIterator, LeafCollector, Scorable, NO_MORE_DOCS,
};
use rucene::util::FixedBitSet;

use crate::core_support::{assert_array_equals, assert_equals, class, test};

/// `TestRangeFilteredBulkScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestRangeFilteredBulkScorer",
        vec![
            test(
                "testCollectRangeClipsToQueryInterval",
                test_collect_range_clips_to_query_interval,
            ),
            test(
                "testScoreWhenWindowEndsBeforeRange",
                test_score_when_window_ends_before_range,
            ),
            test(
                "testScoreWhenWindowStartsAtOrAfterRangeEnd",
                test_score_when_window_starts_at_or_after_range_end,
            ),
            test("testCost", test_cost),
            test(
                "testCompetitiveIteratorRaisesMin",
                test_competitive_iterator_raises_min,
            ),
            test(
                "testAcceptDocsAllLiveMatchesNoAcceptDocs",
                test_accept_docs_all_live_matches_no_accept_docs,
            ),
            test(
                "testAcceptDocsSplitsAroundDeletedDoc",
                test_accept_docs_splits_around_deleted_doc,
            ),
            test(
                "testAcceptDocsNoDocsLiveInWindow",
                test_accept_docs_no_docs_live_in_window,
            ),
        ],
    )
}

/// `bs.score(collector, acceptDocs, min, max)`, its `CollectionTerminated`
/// and I/O failures surfacing as the Java exceptions they stand for.
fn score(
    bs: &mut RangeBulkScorer,
    collector: &mut RangeRecordingCollector,
    accept_docs: Option<&dyn rucene::util::Bits>,
    min: i32,
    max: i32,
) -> Result<i32> {
    bs.score(collector, accept_docs, min, max)
        .map_err(|e| e.into_lucene_error())
}

/// Port of `testCollectRangeClipsToQueryInterval`.
fn test_collect_range_clips_to_query_interval() -> Result<()> {
    let mut bs = new_bulk_scorer(20, 80)?;
    let mut collector = RangeRecordingCollector::default();
    let next = score(&mut bs, &mut collector, None, 0, 100)?;
    assert_collected_ranges(&[[20, 80]], &collector.ranges);
    Assert::assert_true(next >= 100);
    Ok(())
}

/// Port of `testScoreWhenWindowEndsBeforeRange`.
fn test_score_when_window_ends_before_range() -> Result<()> {
    let mut bs = new_bulk_scorer(50, 100)?;
    let mut collector = RangeRecordingCollector::default();
    let next = score(&mut bs, &mut collector, None, 0, 40)?;
    Assert::assert_true(collector.ranges.is_empty());
    assert_equals(&50, &next);
    Ok(())
}

/// Port of `testScoreWhenWindowStartsAtOrAfterRangeEnd`.
fn test_score_when_window_starts_at_or_after_range_end() -> Result<()> {
    let mut bs = new_bulk_scorer(10, 50)?;
    let mut collector = RangeRecordingCollector::default();
    let next = score(&mut bs, &mut collector, None, 55, 200)?;
    Assert::assert_true(collector.ranges.is_empty());
    assert_equals(&NO_MORE_DOCS, &next);
    Ok(())
}

/// Port of `testCost`.
fn test_cost() -> Result<()> {
    assert_equals(&30i64, &new_bulk_scorer(10, 40)?.cost());
    Ok(())
}

/// Port of `testCompetitiveIteratorRaisesMin`.
fn test_competitive_iterator_raises_min() -> Result<()> {
    let mut bs = new_bulk_scorer(10, 60)?;
    let mut collector = RangeRecordingCollector::default();
    let mut competitive = RangeDocIdSetIterator::range(25, 100)?;
    assert_equals(&25, &competitive.next_doc()?);
    collector.set_competitive_iterator(Box::new(competitive));
    let next = score(&mut bs, &mut collector, None, 0, 100)?;
    assert_collected_ranges(&[[25, 60]], &collector.ranges);
    Assert::assert_true(next >= 100);
    Ok(())
}

/// Port of `testAcceptDocsAllLiveMatchesNoAcceptDocs`.
fn test_accept_docs_all_live_matches_no_accept_docs() -> Result<()> {
    let mut bs = new_bulk_scorer(10, 40)?;
    let mut bits = FixedBitSet::new(200);
    bits.set_range(0, 200);
    let mut collector = RangeRecordingCollector::default();
    let next = score(&mut bs, &mut collector, Some(&bits), 0, 100)?;
    assert_collected_ranges(&[[10, 40]], &collector.ranges);
    Assert::assert_true(next >= 100);
    Ok(())
}

/// Port of `testAcceptDocsSplitsAroundDeletedDoc`.
fn test_accept_docs_splits_around_deleted_doc() -> Result<()> {
    let mut bs = new_bulk_scorer(10, 20)?;
    let mut bits = FixedBitSet::new(30);
    bits.set_range(10, 20);
    bits.clear(15);
    let mut collector = RangeRecordingCollector::default();
    score(&mut bs, &mut collector, Some(&bits), 0, 100)?;
    assert_collected_ranges(&[[10, 15], [16, 20]], &collector.ranges);
    Ok(())
}

/// Port of `testAcceptDocsNoDocsLiveInWindow`.
fn test_accept_docs_no_docs_live_in_window() -> Result<()> {
    let mut bs = new_bulk_scorer(10, 30)?;
    let bits = FixedBitSet::new(50);
    // nothing set => all deleted
    let mut collector = RangeRecordingCollector::default();
    let next = score(&mut bs, &mut collector, Some(&bits), 0, 100)?;
    Assert::assert_true(collector.ranges.is_empty());
    Assert::assert_true(next >= 100);
    Ok(())
}

/// `newBulkScorer(rangeMin, rangeMaxExclusive)`.
fn new_bulk_scorer(range_min: i32, range_max_exclusive: i32) -> Result<RangeBulkScorer> {
    let iterator = RangeDocIdSetIterator::range(range_min, range_max_exclusive)?;
    RangeBulkScorer::new(Box::new(iterator), 1.0, range_min, range_max_exclusive)
}

/// `RangeRecordingCollector`: records `LeafCollector#collectRange` calls as
/// `[min inclusive, max exclusive)`.
///
/// **Divergence from Lucene 10.5.0.** Java's `competitiveIterator()` returns
/// the same iterator instance on every call; the port's trait returns an
/// owned `Box`, so the collector hands its iterator over on the first call
/// (the one `RangeBulkScorer.score` makes) and `None` afterwards.
#[derive(Default)]
struct RangeRecordingCollector {
    ranges: Vec<[i32; 2]>,
    competitive_iterator: Option<Box<dyn DocIdSetIterator>>,
}

impl RangeRecordingCollector {
    fn set_competitive_iterator(&mut self, competitive_iterator: Box<dyn DocIdSetIterator>) {
        self.competitive_iterator = Some(competitive_iterator);
    }
}

impl LeafCollector for RangeRecordingCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Assert::fail("collect should not be used when testing collectRange path")
    }

    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        self.ranges.push([min, max]);
        Ok(())
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(self.competitive_iterator.take())
    }
}

/// `assertCollectedRanges(expected, actual)`.
fn assert_collected_ranges(expected: &[[i32; 2]], actual: &[[i32; 2]]) {
    assert_equals(&expected.len(), &actual.len());
    for i in 0..expected.len() {
        assert_array_equals(&expected[i], &actual[i]);
    }
}
