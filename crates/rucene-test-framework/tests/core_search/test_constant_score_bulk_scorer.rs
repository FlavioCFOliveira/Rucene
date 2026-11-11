//! Port of `lucene/core/src/test/org/apache/lucene/search/TestConstantScoreBulkScorer.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::scorable::Scorable;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::{
    BulkScorer, CollectionError, CollectionResult, ConstantScoreBulkScorer,
    ConstantScoreScorerSupplier, DocIdSetIterator, LeafCollector, ScoreMode, ScorerIterator,
    TwoPhaseIterator, TwoPhaseIteratorAsDocIdSetIterator, NO_MORE_DOCS,
};
use rucene::util::bit_set::BitSetIterator;
use rucene::util::{Bits, FixedBitSet};
use rucene_test_framework::search::asserting_bulk_scorer::AssertingBulkScorer;
use rucene_test_framework::search::random_approximation_query::RandomTwoPhaseView;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, class, iae_message, random, test};

/// `TestConstantScoreBulkScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestConstantScoreBulkScorer",
        vec![
            test(
                "testBulkTwoPhaseCollectsExactMatches",
                test_bulk_two_phase_collects_exact_matches,
            ),
            test(
                "testBulkTwoPhaseWithAcceptDocs",
                test_bulk_two_phase_with_accept_docs,
            ),
            test(
                "testBulkTwoPhaseWithCompetitiveIterator",
                test_bulk_two_phase_with_competitive_iterator,
            ),
            test(
                "testSupplierRoutesSparseBulkTwoPhase",
                test_supplier_routes_sparse_bulk_two_phase,
            ),
            test(
                "testSupplierRoutesSparseNonBulkTwoPhase",
                test_supplier_routes_sparse_non_bulk_two_phase,
            ),
            test(
                "testRejectsWrappedTwoPhaseOnPlainConstructor",
                test_rejects_wrapped_two_phase_on_plain_constructor,
            ),
        ],
    )
}

/// The test's private `BulkTwoPhase extends TwoPhaseIterator`.
struct BulkTwoPhase {
    approximation: BitSetIterator,
    match_set: FixedBitSet,
}

impl BulkTwoPhase {
    fn new(approximation: FixedBitSet, match_set: FixedBitSet) -> Result<Self> {
        let cost = approximation.approximate_cardinality() as i64;
        Ok(BulkTwoPhase {
            approximation: BitSetIterator::new(Arc::new(approximation), cost)?,
            match_set,
        })
    }
}

impl TwoPhaseIterator for BulkTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(self.match_set.get(self.approximation.doc_id() as usize))
    }

    fn match_cost(&self) -> f32 {
        1.0
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        let mut doc = self.approximation.doc_id();
        while doc < up_to {
            if self.match_set.get(doc as usize) {
                bit_set.set((doc - offset) as usize);
            }
            doc = self.approximation.next_doc()?;
        }
        Ok(())
    }
}

fn mod_bit_set(max_doc: i32, step: i32) -> FixedBitSet {
    let mut bits = FixedBitSet::new(max_doc as usize);
    let mut i = random().next_int_bound(step);
    while i < max_doc {
        bits.set(i as usize);
        i += step;
    }
    bits
}

fn superset_of(match_set: &FixedBitSet, max_doc: i32) -> FixedBitSet {
    let mut approx = match_set.clone();
    approx.or_set(&mod_bit_set(max_doc, TestUtil::next_int(&*random(), 2, 8)));
    approx
}

/// The anonymous `LeafCollector` of `collect`.
struct SetCollector<'a> {
    result: &'a mut FixedBitSet,
    competitive_it: Option<Box<dyn DocIdSetIterator>>,
}

impl LeafCollector for SetCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.result.set(doc as usize);
        Ok(())
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        // Java hands out the same iterator each time; it is taken once here.
        Ok(self.competitive_it.take())
    }
}

fn collect(
    scorer: &mut dyn BulkScorer,
    max_doc: i32,
    accept_docs: Option<&dyn Bits>,
) -> Result<FixedBitSet> {
    let mut result = FixedBitSet::new(max_doc as usize);
    scorer
        .score(
            &mut SetCollector {
                result: &mut result,
                competitive_it: None,
            },
            accept_docs,
            0,
            NO_MORE_DOCS,
        )
        .map_err(CollectionError::into_lucene_error)?;
    Ok(result)
}

fn bulk_two_phase_scorer(match_set: &FixedBitSet, max_doc: i32) -> Result<Box<dyn BulkScorer>> {
    let two_phase = BulkTwoPhase::new(superset_of(match_set, max_doc), match_set.clone())?;
    Ok(Box::new(ConstantScoreBulkScorer::from_two_phase(
        1.0,
        ScoreMode::COMPLETE_NO_SCORES,
        Box::new(two_phase),
    )?))
}

fn test_bulk_two_phase_collects_exact_matches() -> Result<()> {
    let max_doc = 100_000;
    let match_set = mod_bit_set(max_doc, TestUtil::next_int(&*random(), 2, 50));
    let scorer = bulk_two_phase_scorer(&match_set, max_doc)?;
    let mut scorer = AssertingBulkScorer::wrap(random(), Some(scorer), max_doc).expect("wrapped");
    assert_equals(&match_set, &collect(&mut *scorer, max_doc, None)?);
    Ok(())
}

fn test_bulk_two_phase_with_accept_docs() -> Result<()> {
    let max_doc = 100_000;
    let match_set = mod_bit_set(max_doc, 5);
    let accept_docs = mod_bit_set(max_doc, 2);
    let scorer = bulk_two_phase_scorer(&match_set, max_doc)?;
    let mut scorer = AssertingBulkScorer::wrap(random(), Some(scorer), max_doc).expect("wrapped");
    let mut expected = match_set.clone();
    expected.and(&accept_docs);
    assert_equals(
        &expected,
        &collect(&mut *scorer, max_doc, Some(&accept_docs))?,
    );
    Ok(())
}

fn test_bulk_two_phase_with_competitive_iterator() -> Result<()> {
    let max_doc = 100_000;
    let match_set = mod_bit_set(max_doc, 5);
    let competitive = mod_bit_set(max_doc, 7);
    let mut scorer = bulk_two_phase_scorer(&match_set, max_doc)?;
    let mut result = FixedBitSet::new(max_doc as usize);
    let competitive_cost = competitive.approximate_cardinality() as i64;
    scorer
        .score(
            &mut SetCollector {
                result: &mut result,
                competitive_it: Some(Box::new(BitSetIterator::new(
                    Arc::new(competitive.clone()),
                    competitive_cost,
                )?)),
            },
            None,
            0,
            NO_MORE_DOCS,
        )
        .map_err(CollectionError::into_lucene_error)?;
    let mut expected = match_set.clone();
    expected.and(&competitive);
    assert_equals(&expected, &result);
    Ok(())
}

fn assert_routes_to_constant_score_bulk_scorer(
    iterator: ScorerIterator,
    match_set: &FixedBitSet,
    max_doc: i32,
) -> Result<()> {
    let mut supplier = ConstantScoreScorerSupplier::from_iterator(
        iterator,
        1.0,
        ScoreMode::COMPLETE_NO_SCORES,
        max_doc,
    );
    let mut bulk_scorer = supplier.bulk_scorer()?;
    Assert::assert_true_msg(
        // `bulkScorer.getClass().getName()`: a `dyn BulkScorer` carries no
        // class name, so the message names the trait.
        "expected ConstantScoreBulkScorer but got another BulkScorer",
        bulk_scorer
            .as_any()
            .is_some_and(|a| a.is::<ConstantScoreBulkScorer>()),
    );
    assert_equals(match_set, &collect(&mut *bulk_scorer, max_doc, None)?);
    Ok(())
}

fn test_supplier_routes_sparse_bulk_two_phase() -> Result<()> {
    let max_doc = 100_000;
    let match_set = mod_bit_set(max_doc, 1000);
    // `TwoPhaseIterator.asDocIdSetIterator(two-phase)`: the supplier's
    // iterator, which Rucene keeps as the two-phase variant.
    let iterator = ScorerIterator::TwoPhase(Box::new(BulkTwoPhase::new(
        match_set.clone(),
        match_set.clone(),
    )?));
    assert_routes_to_constant_score_bulk_scorer(iterator, &match_set, max_doc)
}

fn test_supplier_routes_sparse_non_bulk_two_phase() -> Result<()> {
    let max_doc = 100_000;
    let match_set = mod_bit_set(max_doc, 1000);
    let cost = match_set.approximate_cardinality() as i64;
    let iterator = ScorerIterator::TwoPhase(Box::new(RandomTwoPhaseView::new(
        random(),
        Box::new(BitSetIterator::new(Arc::new(match_set.clone()), cost)?),
    )));
    assert_routes_to_constant_score_bulk_scorer(iterator, &match_set, max_doc)
}

fn test_rejects_wrapped_two_phase_on_plain_constructor() -> Result<()> {
    let max_doc = 100_000;
    let match_set = mod_bit_set(max_doc, 5);
    let wrapped: Box<dyn DocIdSetIterator> =
        Box::new(TwoPhaseIteratorAsDocIdSetIterator::new(Box::new(
            BulkTwoPhase::new(superset_of(&match_set, max_doc), match_set.clone())?,
        )));
    let e = iae_message(move || {
        ConstantScoreBulkScorer::from_iterator(1.0, ScoreMode::COMPLETE_NO_SCORES, wrapped)
    });
    assert_equals("Iterator must not wrap a TwoPhaseIterator", e.as_str());
    Ok(())
}
