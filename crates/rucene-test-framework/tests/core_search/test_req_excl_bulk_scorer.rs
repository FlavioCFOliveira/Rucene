//! Port of `lucene/core/src/test/org/apache/lucene/search/TestReqExclBulkScorer.java`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::req_excl_bulk_scorer::ReqExclBulkScorer;
use rucene::search::scorable::{Scorable, SimpleScorable};
use rucene::search::{BulkScorer, CollectionResult, DocIdSetIterator, LeafCollector, NO_MORE_DOCS};
use rucene::util::bit_set::BitSet;
use rucene::util::{Bits, DocIdSetBuilder, FixedBitSet};
use rucene_test_framework::search::random_approximation_query::RandomTwoPhaseView;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_array_equals, class, random, test};

/// `TestReqExclBulkScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestReqExclBulkScorer",
        vec![
            test("testRandom", test_random),
            test("testRandomTwoPhase", test_random_two_phase),
        ],
    )
}

/// Port of `TestReqExclBulkScorer.testRandom`.
fn test_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _iter in 0..iters {
        do_test_random(false)?;
    }
    Ok(())
}

/// Port of `TestReqExclBulkScorer.testRandomTwoPhase`.
fn test_random_two_phase() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _iter in 0..iters {
        do_test_random(true)?;
    }
    Ok(())
}

/// The anonymous `BulkScorer` over `req.iterator()`.
struct ReqBulkScorer {
    iterator: Box<dyn DocIdSetIterator>,
}

impl BulkScorer for ReqBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        // Java's `collect(doc)` takes no scorer; this one is never read.
        let mut scorable = SimpleScorable::new();
        let mut doc = self.iterator.doc_id();
        if self.iterator.doc_id() < min {
            doc = self.iterator.advance(min)?;
        }
        while doc < max {
            if accept_docs.is_none_or(|a| a.get(doc as usize)) {
                collector.collect(doc, &mut scorable)?;
            }
            doc = self.iterator.next_doc()?;
        }
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.iterator.cost()
    }
}

/// The anonymous `LeafCollector` setting `actualMatches`.
struct MatchesCollector<'a> {
    actual_matches: &'a mut FixedBitSet,
}

impl LeafCollector for MatchesCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.actual_matches.set(doc as usize);
        Ok(())
    }
}

/// `FixedBitSet.getBits()`.
fn words(set: &FixedBitSet) -> Vec<u64> {
    set.get_bits()
        .iter()
        .map(|w| w.load(Ordering::Relaxed))
        .collect()
}

/// `public void doTestRandom(boolean twoPhase)`.
fn do_test_random(two_phase: bool) -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 1000);
    let mut req_builder = DocIdSetBuilder::new(max_doc);
    let mut excl_builder = DocIdSetBuilder::new(max_doc);
    let num_included_docs = TestUtil::next_int(&*random(), 1, max_doc);
    let num_excluded_docs = TestUtil::next_int(&*random(), 1, max_doc);
    {
        let mut req_adder = req_builder.grow(num_included_docs);
        for _i in 0..num_included_docs {
            req_adder.add(random().next_int_bound(max_doc));
        }
    }
    {
        let mut excl_adder = excl_builder.grow(num_excluded_docs);
        for _i in 0..num_excluded_docs {
            excl_adder.add(random().next_int_bound(max_doc));
        }
    }

    let req: Box<dyn DocIdSet> = req_builder.build()?;
    let excl: Box<dyn DocIdSet> = excl_builder.build()?;

    let req_bulk_scorer = Box::new(ReqBulkScorer {
        iterator: req.iterator()?,
    });

    let mut req_excl = if two_phase {
        ReqExclBulkScorer::from_two_phase(
            req_bulk_scorer,
            Box::new(RandomTwoPhaseView::new(random(), excl.iterator()?)),
        )
    } else {
        ReqExclBulkScorer::from_iterator(req_bulk_scorer, excl.iterator()?)
    };
    let mut actual_matches = FixedBitSet::new(max_doc as usize);
    if random().next_boolean() {
        req_excl
            .score(
                &mut MatchesCollector {
                    actual_matches: &mut actual_matches,
                },
                None,
                0,
                NO_MORE_DOCS,
            )
            .map_err(|e| e.into_lucene_error())?;
    } else {
        let mut next = 0;
        while next < max_doc {
            let min = next;
            let max = min + random().next_int_bound(10);
            next = req_excl
                .score(
                    &mut MatchesCollector {
                        actual_matches: &mut actual_matches,
                    },
                    None,
                    min,
                    max,
                )
                .map_err(|e| e.into_lucene_error())?;
            Assert::assert_true(next >= max);
        }
    }

    let mut expected_matches = FixedBitSet::new(max_doc as usize);
    expected_matches.or(&mut *req.iterator()?)?;
    let mut excluded_set = FixedBitSet::new(max_doc as usize);
    excluded_set.or(&mut *excl.iterator()?)?;
    expected_matches.and_not(&excluded_set);

    assert_array_equals(&words(&expected_matches), &words(&actual_matches));
    Ok(())
}
