//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/TestDisjunctionDISIApproximation.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::disi_wrapper::DisiWrapper;
use rucene::search::disjunction_disi_approximation::DisjunctionDISIApproximation;
use rucene::search::doc_id_set_iterator::{range, DocIdSetIterator};
use rucene::search::scorer::Scorer;
use rucene::search::ScoreMode;

use crate::core_support::{assert_equals, class, test};

/// `TestDisjunctionDISIApproximation`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDisjunctionDISIApproximation",
        vec![
            test("testDocIDRunEnd", test_doc_id_run_end),
            test(
                "testDocIDRunEndOnlyUsesLeadIteratorsWhenBothClausesMatchAndOtherHasLongerRun",
                test_doc_id_run_end_only_uses_lead_iterators_when_both_clauses_match_and_other_has_longer_run,
            ),
            test(
                "testDocIDRunEndDoesNotScanOtherIterators",
                test_doc_id_run_end_does_not_scan_other_iterators,
            ),
        ],
    )
}

/// `new ConstantScoreScorer(1f, ScoreMode.COMPLETE_NO_SCORES, DocIdSetIterator.range(min, max))`.
fn scorer(min: i32, max: i32) -> Result<Box<dyn Scorer>> {
    Ok(Box::new(ConstantScoreScorer::from_iterator(
        1.0,
        ScoreMode::COMPLETE_NO_SCORES,
        Box::new(range(min, max)?),
    )))
}

fn three_clauses(lead_cost: i64) -> Result<DisjunctionDISIApproximation> {
    let scorer1 = scorer(10_000, 30_000)?;
    let scorer2 = scorer(20_000, 50_000)?;
    let scorer3 = scorer(60_000, 60_001)?;
    Ok(DisjunctionDISIApproximation::new(
        vec![
            DisiWrapper::new(scorer1, false),
            DisiWrapper::new(scorer2, false),
            DisiWrapper::new(scorer3, false),
        ],
        lead_cost,
    ))
}

fn test_doc_id_run_end() -> Result<()> {
    // High enough to keep all clauses in the lead heap.
    let mut iterator = three_clauses(100_000)?;
    assert_equals(&10_000, &iterator.next_doc()?);
    assert_equals(&30_000, &iterator.doc_id_run_end()?);
    assert_equals(&25_000, &iterator.advance(25_000)?);
    assert_equals(&50_000, &iterator.doc_id_run_end()?);
    assert_equals(&60_000, &iterator.advance(50_000)?);
    assert_equals(&60_001, &iterator.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_only_uses_lead_iterators_when_both_clauses_match_and_other_has_longer_run(
) -> Result<()> {
    let heap_scorer = scorer(100, 110)?;
    let other_iterator_scorer = scorer(100, 200)?;
    let mut iterator = DisjunctionDISIApproximation::new(
        vec![
            DisiWrapper::new(heap_scorer, false),
            DisiWrapper::new(other_iterator_scorer, false),
        ],
        // Low enough to put the shorter, lower-cost clause in the lead heap and the longer,
        // higher-cost clause in otherIterators.
        10,
    );
    // Both clauses match doc 100, but the longer otherIterator run is ignored.
    assert_equals(&100, &iterator.next_doc()?);
    assert_equals(&110, &iterator.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_does_not_scan_other_iterators() -> Result<()> {
    let mut iterator = three_clauses(1)?;
    assert_equals(&10_000, &iterator.next_doc()?);
    assert_equals(&10_001, &iterator.doc_id_run_end()?);
    assert_equals(&25_000, &iterator.advance(25_000)?);
    assert_equals(&25_001, &iterator.doc_id_run_end()?);
    assert_equals(&60_000, &iterator.advance(50_000)?);
    assert_equals(&60_001, &iterator.doc_id_run_end()?);
    Ok(())
}
