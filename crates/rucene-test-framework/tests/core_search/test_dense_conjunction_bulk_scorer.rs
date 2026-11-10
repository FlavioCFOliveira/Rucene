//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDenseConjunctionBulkScorer.java`.
//!
//! Java's `new DenseConjunctionBulkScorer(List<DocIdSetIterator>,
//! List<TwoPhaseIterator>, maxDoc, score)` is Rucene's
//! `DenseConjunctionBulkScorer::new(Vec<ConjunctionMember>, …)`: the two Java
//! lists are passed as one member list, plain iterators first, in the order
//! of the Java lists (`dense` below).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set_iterator as disi;
use rucene::search::scorable::Scorable;
use rucene::search::{
    BulkScorer, CollectionError, CollectionResult, ConjunctionMember, DenseConjunctionBulkScorer,
    DocIdSetIterator, DocIdStream, FilterDocIdSetIterator, LeafCollector, TwoPhaseIterator,
    NO_MORE_DOCS, WINDOW_SIZE,
};
use rucene::util::bit_set::{BitSet, BitSetIterator};
use rucene::util::{Bits, FixedBitSet};
use rucene_test_framework::search::asserting_bulk_scorer::AssertingBulkScorer;
use rucene_test_framework::search::random_approximation_query::RandomTwoPhaseView;

use crate::core_support::{assert_array_equals, assert_equals, class, random, shuffle, test};

/// `TestDenseConjunctionBulkScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDenseConjunctionBulkScorer",
        vec![
            test("testSameMatches", test_same_matches),
            test("testApplyAcceptDocs", test_apply_accept_docs),
            test("testEmptyIntersection", test_empty_intersection),
            test("testClustered", test_clustered),
            test("testSparseAfter2ndClause", test_sparse_after2nd_clause),
            test("testMatchAllNoLiveDocs", test_match_all_no_live_docs),
            test("testMatchAllWithLiveDocs", test_match_all_with_live_docs),
            test("testOneClauseNoLiveDocs", test_one_clause_no_live_docs),
            test("testOneClauseWithLiveDocs", test_one_clause_with_live_docs),
            test(
                "testStopOnMinCompetitiveScore",
                test_stop_on_min_competitive_score,
            ),
            test(
                "testTwoPhaseRunCollectedAsRange",
                test_two_phase_run_collected_as_range,
            ),
            test("testRangeIntersection", test_range_intersection),
            test(
                "testRangeIntersectionWithLiveDocs",
                test_range_intersection_with_live_docs,
            ),
            test("testMixedRangeIntersection", test_mixed_range_intersection),
            test(
                "testMixedRangeIntersectionWithLiveDocs",
                test_mixed_range_intersection_with_live_docs,
            ),
            test("testTwoPhaseIterators", test_two_phase_iterators),
            test(
                "testTwoPhaseIteratorsWithLiveDocs",
                test_two_phase_iterators_with_live_docs,
            ),
            test("testMixedTwoPhaseIterators", test_mixed_two_phase_iterators),
            test(
                "testTwoPhaseRangeIntersection",
                test_two_phase_range_intersection,
            ),
            test(
                "testMixedTwoPhaseRangeIntersection",
                test_mixed_two_phase_range_intersection,
            ),
            test(
                "testMixedRangeIntersectionTwoPhase1",
                test_mixed_range_intersection_two_phase1,
            ),
            test(
                "testMixedRangeIntersectionTwoPhase2",
                test_mixed_range_intersection_two_phase2,
            ),
        ],
    )
}

type Disi = Box<dyn DocIdSetIterator>;
type Tpi = Box<dyn TwoPhaseIterator>;

/// `new BitSetIterator(bits, bits.approximateCardinality())`.
fn bsi(bits: &Arc<FixedBitSet>) -> Result<Disi> {
    let cost = bits.approximate_cardinality() as i64;
    Ok(Box::new(BitSetIterator::new(
        Arc::clone(bits) as Arc<dyn BitSet>,
        cost,
    )?))
}

/// `new BitSetIterator(bits, cost)`.
fn bsi_cost(bits: &Arc<FixedBitSet>, cost: i64) -> Result<Disi> {
    Ok(Box::new(BitSetIterator::new(
        Arc::clone(bits) as Arc<dyn BitSet>,
        cost,
    )?))
}

/// `new RandomTwoPhaseView(random(), disi)`.
fn rtpv(disi: Disi) -> Tpi {
    Box::new(RandomTwoPhaseView::new(random(), disi))
}

/// `DocIdSetIterator.range(min, max)`.
fn range(min: i32, max: i32) -> Result<Disi> {
    Ok(Box::new(disi::range(min, max)?))
}

/// `new DenseConjunctionBulkScorer(iterators, twoPhases, maxDoc, 0f)`.
fn dense(iterators: Vec<Disi>, two_phases: Vec<Tpi>, max_doc: i32) -> Result<Box<dyn BulkScorer>> {
    let mut members: Vec<ConjunctionMember> = iterators
        .into_iter()
        .map(ConjunctionMember::from_iterator)
        .collect();
    members.extend(
        two_phases
            .into_iter()
            .map(ConjunctionMember::from_two_phase),
    );
    Ok(Box::new(DenseConjunctionBulkScorer::new(
        members, max_doc, 0.0,
    )?))
}

/// `AssertingBulkScorer.wrap(random(), scorer, maxDoc)`.
fn asserting(scorer: Box<dyn BulkScorer>, max_doc: i32) -> Box<dyn BulkScorer> {
    AssertingBulkScorer::wrap(random(), Some(scorer), max_doc).expect("non-null scorer")
}

/// `scorer.score(collector, acceptDocs, 0, NO_MORE_DOCS)`.
fn score(
    scorer: &mut dyn BulkScorer,
    collector: &mut dyn LeafCollector,
    accept_docs: Option<&dyn Bits>,
) -> Result<()> {
    scorer
        .score(collector, accept_docs, 0, NO_MORE_DOCS)
        .map_err(CollectionError::into_lucene_error)?;
    Ok(())
}

/// The anonymous `LeafCollector` that sets each collected doc in `result`.
struct ResultCollector<'a> {
    result: &'a mut FixedBitSet,
}

impl LeafCollector for ResultCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.result.set(doc as usize);
        Ok(())
    }
}

/// Scores `scorer` into a new `FixedBitSet(maxDoc)` through
/// [`ResultCollector`].
fn collect_result(
    scorer: &mut dyn BulkScorer,
    max_doc: i32,
    accept_docs: Option<&dyn Bits>,
) -> Result<FixedBitSet> {
    let mut result = FixedBitSet::new(max_doc as usize);
    score(
        scorer,
        &mut ResultCollector {
            result: &mut result,
        },
        accept_docs,
    )?;
    Ok(result)
}

/// `private static class CountingLeafCollector implements LeafCollector`.
#[derive(Default)]
struct CountingLeafCollector {
    count: i32,
}

impl LeafCollector for CountingLeafCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.count += 1;
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        self.count += stream.count()?;
        Ok(())
    }
}

/// Scores `scorer` through a new [`CountingLeafCollector`] and returns its
/// count.
fn count(scorer: &mut dyn BulkScorer, accept_docs: Option<&dyn Bits>) -> Result<i32> {
    let mut collector = CountingLeafCollector::default();
    score(scorer, &mut collector, accept_docs)?;
    Ok(collector.count)
}

/// A new `FixedBitSet(maxDoc)` with every `step`-th bit from `start` below
/// `end` set (`for (int i = start; i < end; i += step) bits.set(i)`).
fn stepped(max_doc: i32, start: i32, end: i32, step: i32) -> FixedBitSet {
    let mut bits = FixedBitSet::new(max_doc as usize);
    let mut i = start;
    while i < end {
        bits.set(i as usize);
        i += step;
    }
    bits
}

fn words(bits: &FixedBitSet) -> Vec<u64> {
    bits.get_bits()
        .iter()
        .map(|w| w.load(Ordering::Relaxed))
        .collect()
}

fn test_same_matches() -> Result<()> {
    let max_doc = 100_000;
    let mut clause1 = FixedBitSet::new(max_doc as usize);
    let mut clause2 = FixedBitSet::new(max_doc as usize);
    let mut clause3 = FixedBitSet::new(max_doc as usize);
    let mut i = 0;
    while i < max_doc {
        clause1.set(i as usize);
        clause2.set(i as usize);
        clause3.set(i as usize);
        i += 2;
    }
    let (clause1, clause2, clause3) = (Arc::new(clause1), Arc::new(clause2), Arc::new(clause3));
    let scorer = dense(
        vec![bsi(&clause1)?, bsi(&clause2)?, bsi(&clause3)?],
        vec![],
        max_doc,
    )?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    assert_equals(&*clause1, &result);

    // Now exercise DocIdStream.count()
    let mut scorer = dense(
        vec![bsi(&clause1)?, bsi(&clause2)?, bsi(&clause3)?],
        vec![],
        max_doc,
    )?;
    assert_equals(&(clause1.cardinality() as i32), &count(&mut *scorer, None)?);
    Ok(())
}

fn test_apply_accept_docs() -> Result<()> {
    let max_doc = 100_000;
    let mut clause1 = FixedBitSet::new(max_doc as usize);
    let mut clause2 = FixedBitSet::new(max_doc as usize);
    clause1.set_range(0, max_doc as usize);
    clause2.set_range(0, max_doc as usize);
    let accept_docs = stepped(max_doc, 0, max_doc, 2);
    let (clause1, clause2) = (Arc::new(clause1), Arc::new(clause2));
    let scorer = dense(vec![bsi(&clause1)?, bsi(&clause2)?], vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, Some(&accept_docs))?;

    assert_equals(&accept_docs, &result);

    // Now exercise DocIdStream.count()
    let mut scorer = dense(vec![bsi(&clause1)?, bsi(&clause2)?], vec![], max_doc)?;
    assert_equals(
        &(accept_docs.cardinality() as i32),
        &count(&mut *scorer, Some(&accept_docs))?,
    );
    Ok(())
}

fn test_empty_intersection() -> Result<()> {
    let max_doc = 100_000;
    let mut clause1 = FixedBitSet::new(max_doc as usize);
    let mut clause2 = FixedBitSet::new(max_doc as usize);
    let mut i = 0;
    while i < max_doc - 1 {
        clause1.set(i as usize);
        clause2.set((i + 1) as usize);
        i += 2;
    }
    let (clause1, clause2) = (Arc::new(clause1), Arc::new(clause2));
    let scorer = dense(vec![bsi(&clause1)?, bsi(&clause2)?], vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    Assert::assert_true(result.scan_is_empty());

    // Now exercise DocIdStream.count()
    let mut scorer = dense(vec![bsi(&clause1)?, bsi(&clause2)?], vec![], max_doc)?;
    assert_equals(&0, &count(&mut *scorer, None)?);
    Ok(())
}

fn test_clustered() -> Result<()> {
    let max_doc = 100_000;
    let mut clause1 = FixedBitSet::new(max_doc as usize);
    let mut clause2 = FixedBitSet::new(max_doc as usize);
    let mut clause3 = FixedBitSet::new(max_doc as usize);
    clause1.set_range(10_000, 90_000);
    clause2.set_range(0, 80_000);
    clause3.set_range(20_000, 100_000);
    let (clause1, clause2, clause3) = (Arc::new(clause1), Arc::new(clause2), Arc::new(clause3));
    let scorer = dense(
        vec![bsi(&clause1)?, bsi(&clause2)?, bsi(&clause3)?],
        vec![],
        max_doc,
    )?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    let mut expected = FixedBitSet::new(max_doc as usize);
    expected.set_range(20_000, 80_000);
    assert_array_equals(&words(&expected), &words(&result));
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let mut scorer = dense(
        vec![bsi(&clause1)?, bsi(&clause2)?, bsi(&clause3)?],
        vec![],
        max_doc,
    )?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, None)?,
    );
    Ok(())
}

fn test_sparse_after2nd_clause() -> Result<()> {
    let max_doc = 100_000;
    // 13 and 17 are primes, so their only intersection is on multiples of both 13 and 17
    // Likewise, 19 is prime, so the only intersection of the conjunction is on multiples of 13, 17
    // and 19
    let clause1 = Arc::new(stepped(max_doc, 0, max_doc, 13));
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 17));
    let clause3 = Arc::new(stepped(max_doc, 0, max_doc, 19));
    let scorer = dense(
        vec![bsi(&clause1)?, bsi(&clause2)?, bsi(&clause3)?],
        vec![],
        max_doc,
    )?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    let expected = stepped(max_doc, 0, max_doc, 13 * 17 * 19);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let mut scorer = dense(
        vec![bsi(&clause1)?, bsi(&clause2)?, bsi(&clause3)?],
        vec![],
        max_doc,
    )?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, None)?,
    );
    Ok(())
}

fn test_match_all_no_live_docs() -> Result<()> {
    let max_doc = 100_000;
    let scorer = dense(vec![Box::new(disi::all(max_doc)?)], vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let mut result = collect_result(&mut *scorer, max_doc, None)?;

    result.flip_range(0, max_doc as usize);
    assert_equals(&NO_MORE_DOCS, &BitSet::next_set_bit(&result, 0));

    // Now exercise DocIdStream.count()
    let mut scorer = dense(vec![Box::new(disi::all(max_doc)?)], vec![], max_doc)?;
    assert_equals(&max_doc, &count(&mut *scorer, None)?);
    Ok(())
}

fn test_match_all_with_live_docs() -> Result<()> {
    let max_doc = 100_000;
    let scorer = dense(vec![Box::new(disi::all(max_doc)?)], vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let mut accept_docs = stepped(max_doc, 30_000, max_doc, 3);
    accept_docs.set_range(10_000, 20_000);
    let result = collect_result(&mut *scorer, max_doc, Some(&accept_docs))?;

    assert_equals(&accept_docs, &result);

    // Now exercise DocIdStream.count()
    let mut scorer = dense(vec![Box::new(disi::all(max_doc)?)], vec![], max_doc)?;
    assert_equals(
        &(accept_docs.cardinality() as i32),
        &count(&mut *scorer, Some(&accept_docs))?,
    );
    Ok(())
}

fn test_one_clause_no_live_docs() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = Arc::new(stepped(max_doc, 0, max_doc, 2));
    let scorer = dense(vec![bsi(&clause1)?], vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    assert_equals(&*clause1, &result);

    // Now exercise DocIdStream.count()
    let mut scorer = dense(vec![bsi(&clause1)?], vec![], max_doc)?;
    assert_equals(&(clause1.cardinality() as i32), &count(&mut *scorer, None)?);
    Ok(())
}

fn test_one_clause_with_live_docs() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = Arc::new(stepped(max_doc, 0, max_doc, 2));
    let scorer = dense(vec![bsi(&clause1)?], vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let mut accept_docs = stepped(max_doc, 30_000, max_doc, 3);
    accept_docs.set_range(10_000, 20_000);
    let result = collect_result(&mut *scorer, max_doc, Some(&accept_docs))?;

    let mut expected = FixedBitSet::new(max_doc as usize);
    expected.or_set(&accept_docs);
    expected.and(&clause1);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let mut scorer = dense(vec![bsi(&clause1)?], vec![], max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, Some(&accept_docs))?,
    );
    Ok(())
}

/// The anonymous `LeafCollector` of `testStopOnMinCompetitiveScore`; the
/// scorable Java stores in `setScorer` is the one each `collect` receives.
struct MinCompetitiveCollector<'a> {
    result: &'a mut FixedBitSet,
}

impl LeafCollector for MinCompetitiveCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorable: &mut dyn Scorable) -> CollectionResult<()> {
        self.result.set(doc as usize);
        if doc == 50_000 {
            // `Float.MIN_VALUE`.
            scorable.set_min_competitive_score(f32::from_bits(1))?;
        }
        // It should never go above the doc when setMinCompetitiveScore was called, plus the
        // window size
        Assert::assert_true(doc < 50_000 + WINDOW_SIZE);
        Ok(())
    }
}

fn test_stop_on_min_competitive_score() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = Arc::new(stepped(max_doc, 0, max_doc, 2));
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 5));
    let scorer = dense(vec![bsi(&clause1)?, bsi(&clause2)?], vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let mut result = FixedBitSet::new(max_doc as usize);
    score(
        &mut *scorer,
        &mut MinCompetitiveCollector {
            result: &mut result,
        },
        None,
    )?;
    Ok(())
}

/// The anonymous `TwoPhaseIterator` of `testTwoPhaseRunCollectedAsRange`.
struct RunTwoPhase {
    approximation: Disi,
}

impl TwoPhaseIterator for RunTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(true) // every doc in the run matches, like a YES block
    }

    fn match_cost(&self) -> f32 {
        1.0
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        self.approximation().doc_id_run_end()
    }
}

/// The anonymous `LeafCollector` of `testTwoPhaseRunCollectedAsRange`.
struct RangeOnlyCollector<'a> {
    collected_via_range: &'a mut [i32; 1],
}

impl LeafCollector for RangeOnlyCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Assert::fail("a fully-matching run must be collected via collectRange, not per-doc")
    }

    fn collect_stream(
        &mut self,
        _stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        Assert::fail(
            "a fully-matching run must be collected via collectRange, not as a bit-set stream",
        )
    }

    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        Assert::assert_true(min >= 10_000);
        Assert::assert_true(max <= 60_000);
        self.collected_via_range[0] += max - min;
        Ok(())
    }
}

/// A two-phase clause that reports a real `TwoPhaseIterator#docIDRunEnd()`
/// run — as `DocValuesRangeIterator`'s block iterator does for
/// fully-matching YES blocks — must be collected via
/// `LeafCollector#collectRange`, never materialized into a bit set or
/// visited per-doc. This guards the collectRange fast path for doc-values
/// ranges: the two-phase unification routes ranges through this scorer as
/// two-phase clauses, so the run has to survive `DisiWrapper#docIDRunEnd()`
/// delegating to the two-phase. The existing two-phase tests use
/// `RandomTwoPhaseView`, whose approximation reports no run, so they
/// exercise only the bit-set path.
fn test_two_phase_run_collected_as_range() -> Result<()> {
    let max_doc = 100_000;
    let two_phase: Tpi = Box::new(RunTwoPhase {
        approximation: range(10_000, 60_000)?,
    });
    let mut scorer = dense(vec![], vec![two_phase], max_doc)?;
    let mut collected_via_range = [0];
    score(
        &mut *scorer,
        &mut RangeOnlyCollector {
            collected_via_range: &mut collected_via_range,
        },
        None,
    )?;
    assert_equals(&50_000, &collected_via_range[0]);
    Ok(())
}

/// The anonymous `LeafCollector` of `testRangeIntersection`: matches are
/// collected as a single `DocIdStream`.
struct SingleStreamCollector {
    called: bool,
    expected: i32,
}

impl LeafCollector for SingleStreamCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Assert::fail_without_message()
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        Assert::assert_false(self.called);
        self.called = true;
        let expected = &mut self.expected;
        stream.for_each(&mut |doc: i32| -> CollectionResult<()> {
            assert_equals(&*expected, &doc);
            *expected += 1;
            Ok(())
        })
    }

    fn finish(&mut self) -> Result<()> {
        assert_equals(&60_001, &self.expected);
        Ok(())
    }
}

fn test_range_intersection() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let mut clauses = vec![clause1, clause2];
    shuffle(&mut clauses, &*random());

    let mut scorer = dense(clauses, vec![], max_doc)?;
    // Matches are collected as a single DocIdStream
    score(
        &mut *scorer,
        &mut SingleStreamCollector {
            called: false,
            expected: 30_000,
        },
        None,
    )?;

    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let mut clauses = vec![clause1, clause2];
    shuffle(&mut clauses, &*random());
    let mut scorer = dense(clauses, vec![], max_doc)?;
    assert_equals(&30_000, &count(&mut *scorer, None)?);
    Ok(())
}

fn test_range_intersection_with_live_docs() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let mut clauses = vec![clause1, clause2];
    shuffle(&mut clauses, &*random());

    let accept_docs = stepped(max_doc, 0, max_doc, 2);

    let scorer = dense(clauses, vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, Some(&accept_docs))?;

    let expected = stepped(max_doc, 30_000, 60_000, 2);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let mut clauses = vec![clause1, clause2];
    shuffle(&mut clauses, &*random());
    let mut scorer = dense(clauses, vec![], max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, Some(&accept_docs))?,
    );
    Ok(())
}

/// The anonymous `FilterDocIdSetIterator` of `testMixedRangeIntersection`.
struct NoIntoBitSetIterator {
    filter: FilterDocIdSetIterator,
}

impl DocIdSetIterator for NoIntoBitSetIterator {
    fn doc_id(&self) -> i32 {
        self.filter.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.filter.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.filter.advance(target)
    }

    fn cost(&self) -> i64 {
        self.filter.cost()
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.filter.inner.doc_id_run_end()
    }

    fn into_bit_set(
        &mut self,
        _up_to: i32,
        _bit_set: &mut FixedBitSet,
        _offset: i32,
    ) -> Result<()> {
        Assert::fail(
            "intoBitSet should not be called if scoring windows get aligned with #docIDRunEnd",
        )
    }
}

fn test_mixed_range_intersection() -> Result<()> {
    let max_doc = 80_000;
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 2));
    let range_iterator: Disi = Box::new(NoIntoBitSetIterator {
        filter: FilterDocIdSetIterator::new(range(10_000, 50_000)?),
    });
    let mut clauses = vec![range_iterator, bsi_cost(&clause2, 40_000)?];
    shuffle(&mut clauses, &*random());
    let mut scorer = dense(clauses, vec![], max_doc)?;
    let result = collect_result(&mut *scorer, max_doc, None)?;

    let expected = stepped(max_doc, 10_000, 50_000, 2);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let mut clauses = vec![range(10_000, 50_000)?, bsi_cost(&clause2, 40_000)?];
    shuffle(&mut clauses, &*random());
    let mut scorer = dense(clauses, vec![], max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, None)?,
    );
    Ok(())
}

fn test_mixed_range_intersection_with_live_docs() -> Result<()> {
    let max_doc = 100_000;
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 3));
    let mut clauses = vec![range(10_000, 60_000)?, bsi_cost(&clause2, 50_000)?];
    shuffle(&mut clauses, &*random());

    let accept_docs = stepped(max_doc, 0, max_doc, 2);

    let scorer = dense(clauses, vec![], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, Some(&accept_docs))?;

    let expected = stepped(max_doc, 10_002, 60_000, 6);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let mut clauses = vec![range(10_000, 60_000)?, bsi_cost(&clause2, 50_000)?];
    shuffle(&mut clauses, &*random());
    let mut scorer = dense(clauses, vec![], max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, Some(&accept_docs))?,
    );
    Ok(())
}

fn test_two_phase_iterators() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = Arc::new(stepped(max_doc, 0, max_doc, 3));
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 5));
    let clause3 = Arc::new(stepped(max_doc, 0, max_doc, 7));
    let two_phases = vec![
        rtpv(bsi(&clause1)?),
        rtpv(bsi(&clause2)?),
        rtpv(bsi(&clause3)?),
    ];
    let scorer = dense(vec![], two_phases, max_doc)?;
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    let expected = stepped(max_doc, 0, max_doc, 3 * 5 * 7);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let two_phases = vec![
        rtpv(bsi(&clause1)?),
        rtpv(bsi(&clause2)?),
        rtpv(bsi(&clause3)?),
    ];
    let mut scorer = dense(vec![], two_phases, max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, None)?,
    );
    Ok(())
}

fn test_two_phase_iterators_with_live_docs() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = Arc::new(stepped(max_doc, 0, max_doc, 3));
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 5));
    let accept_docs = stepped(max_doc, 0, max_doc, 2);
    let two_phases = vec![rtpv(bsi(&clause1)?), rtpv(bsi(&clause2)?)];
    let scorer = dense(vec![], two_phases, max_doc)?;
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, Some(&accept_docs))?;

    let expected = stepped(max_doc, 0, max_doc, 2 * 3 * 5);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let two_phases = vec![rtpv(bsi(&clause1)?), rtpv(bsi(&clause2)?)];
    let mut scorer = dense(vec![], two_phases, max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, Some(&accept_docs))?,
    );
    Ok(())
}

fn test_mixed_two_phase_iterators() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = Arc::new(stepped(max_doc, 0, max_doc, 3));
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 5));
    let clause3 = Arc::new(stepped(max_doc, 0, max_doc, 7));
    let iterators = vec![bsi(&clause2)?];
    let two_phases = vec![rtpv(bsi(&clause1)?), rtpv(bsi(&clause3)?)];
    let scorer = dense(iterators, two_phases, max_doc)?;
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    let expected = stepped(max_doc, 0, max_doc, 3 * 5 * 7);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let iterators = vec![bsi(&clause2)?];
    let two_phases = vec![rtpv(bsi(&clause1)?), rtpv(bsi(&clause3)?)];
    let mut scorer = dense(iterators, two_phases, max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, None)?,
    );
    Ok(())
}

/// The anonymous `LeafCollector` of the two-phase range intersections:
/// matches arrive in order, as ranges and/or bit-set windows.
struct InOrderCollector {
    expected: i32,
}

impl LeafCollector for InOrderCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        assert_equals(&self.expected, &doc);
        self.expected += 1;
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        let expected = &mut self.expected;
        stream.for_each(&mut |doc: i32| -> CollectionResult<()> {
            assert_equals(&*expected, &doc);
            *expected += 1;
            Ok(())
        })
    }

    fn finish(&mut self) -> Result<()> {
        assert_equals(&60_001, &self.expected);
        Ok(())
    }
}

fn test_two_phase_range_intersection() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let clause3 = range(20_000, 70_000)?;
    let mut clauses = vec![rtpv(clause1), rtpv(clause2), rtpv(clause3)];
    shuffle(&mut clauses, &*random());

    let mut scorer = dense(vec![], clauses, max_doc)?;
    // Matches arrive in order, as ranges and/or bit-set windows.
    score(
        &mut *scorer,
        &mut InOrderCollector { expected: 30_000 },
        None,
    )?;

    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let clause3 = range(20_000, 70_000)?;
    let mut clauses = vec![rtpv(clause1), rtpv(clause2), rtpv(clause3)];
    shuffle(&mut clauses, &*random());

    let mut scorer = dense(vec![], clauses, max_doc)?;
    assert_equals(&30_000, &count(&mut *scorer, None)?);
    Ok(())
}

fn test_mixed_two_phase_range_intersection() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let clause3 = range(20_000, 70_000)?;

    let two_phases = vec![rtpv(clause2), rtpv(clause3)];
    let mut scorer = dense(vec![clause1], two_phases, max_doc)?;
    // Matches arrive in order, as ranges and/or bit-set windows.
    score(
        &mut *scorer,
        &mut InOrderCollector { expected: 30_000 },
        None,
    )?;

    let clause1 = range(10_000, 60_000)?;
    let clause2 = range(30_000, 80_000)?;
    let clause3 = range(20_000, 70_000)?;

    let two_phases = vec![rtpv(clause2), rtpv(clause3)];
    let mut scorer = dense(vec![clause1], two_phases, max_doc)?;
    assert_equals(&30_000, &count(&mut *scorer, None)?);
    Ok(())
}

fn test_mixed_range_intersection_two_phase1() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = range(10_000, 60_000)?;
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 2));

    let iterators = vec![bsi_cost(&clause2, clause2.cardinality() as i64)?];
    let two_phases = vec![rtpv(clause1)];
    let scorer = dense(iterators, two_phases, max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    let expected = stepped(max_doc, 10_000, 60_000, 2);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let clause1 = range(10_000, 60_000)?;
    let iterators = vec![bsi_cost(&clause2, clause2.cardinality() as i64)?];
    let two_phases = vec![rtpv(clause1)];
    let mut scorer = dense(iterators, two_phases, max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, None)?,
    );
    Ok(())
}

fn test_mixed_range_intersection_two_phase2() -> Result<()> {
    let max_doc = 100_000;
    let clause1 = range(10_000, 60_000)?;
    let clause2 = Arc::new(stepped(max_doc, 0, max_doc, 2));

    let a = rtpv(clause1);
    let b = rtpv(bsi_cost(&clause2, clause2.cardinality() as i64)?);
    let scorer = dense(vec![], vec![a, b], max_doc)?;
    // AssertingBulkScorer randomly splits the scored range into smaller ranges
    let mut scorer = asserting(scorer, max_doc);
    let result = collect_result(&mut *scorer, max_doc, None)?;

    let expected = stepped(max_doc, 10_000, 60_000, 2);
    assert_equals(&expected, &result);

    // Now exercise DocIdStream.count()
    let clause1 = range(10_000, 60_000)?;
    let a = rtpv(clause1);
    let b = rtpv(bsi_cost(&clause2, clause2.cardinality() as i64)?);
    let mut scorer = dense(vec![], vec![a, b], max_doc)?;
    assert_equals(
        &(expected.cardinality() as i32),
        &count(&mut *scorer, None)?,
    );
    Ok(())
}
