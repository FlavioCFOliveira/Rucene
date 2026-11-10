//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDocValuesRangeIterator.java`.
//!
//! `testYesMatchDoesNotAdvanceDocValues` reads `values.docID()` after
//! handing the values to the iterator; the shared handle of
//! `base_doc_values_skipper_tests::doc_values_shared` stands for that
//! reference.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::{
    DocIdSetIterator, DocValuesRangeIterator, Match, TwoPhaseIterator, NO_MORE_DOCS,
};
use rucene::util::FixedBitSet;

use crate::core_search::base_doc_values_skipper_tests::{
    approx, collect_matches, doc_values, doc_values_shared, doc_values_skipper, DENSE_END,
};
use crate::core_support::{assert_equals, assert_equals_msg, class, test};

const QUERY_MIN: i64 = 10;
const QUERY_MAX: i64 = 20;

/// `TestDocValuesRangeIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDocValuesRangeIterator",
        vec![
            test(
                "testCorrectResultsSingleLevel",
                test_correct_results_single_level,
            ),
            test(
                "testCorrectResultsMultipleLevels",
                test_correct_results_multiple_levels,
            ),
            test(
                "testApproximationSkipsAboveRangeBlock",
                test_approximation_skips_above_range_block,
            ),
            test(
                "testApproximationSkipsBelowRangeBlock",
                test_approximation_skips_below_range_block,
            ),
            test(
                "testApproximationSkipsFromEndOfMatchingBlock",
                test_approximation_skips_from_end_of_matching_block,
            ),
            test(
                "testApproximationIteratesWithinMatchingBlock",
                test_approximation_iterates_within_matching_block,
            ),
            test(
                "testApproximationSkipsSparseNonMatchingBlocks",
                test_approximation_skips_sparse_non_matching_blocks,
            ),
            test("testApproximationExhausted", test_approximation_exhausted),
            test("testYesMatchInDenseBlock", test_yes_match_in_dense_block),
            test(
                "testYesMatchDoesNotAdvanceDocValues",
                test_yes_match_does_not_advance_doc_values,
            ),
            test(
                "testMaybeMatchClassification",
                test_maybe_match_classification,
            ),
            test("testMaybeMatchChecksValues", test_maybe_match_checks_values),
            test(
                "testYesIfPresentInSparseBlock",
                test_yes_if_present_in_sparse_block,
            ),
            test(
                "testDocIdRunEndYesBlockMultipleLevels",
                test_doc_id_run_end_yes_block_multiple_levels,
            ),
            test(
                "testDocIdRunEndExtendsFromMiddleOfBlock",
                test_doc_id_run_end_extends_from_middle_of_block,
            ),
            test(
                "testDocIdRunEndConsistentAcrossYesBlock",
                test_doc_id_run_end_consistent_across_yes_block,
            ),
            test("testDocIdRunEndMaybeBlock", test_doc_id_run_end_maybe_block),
            test(
                "testDocIdRunEndYesIfPresentBlock",
                test_doc_id_run_end_yes_if_present_block,
            ),
            test(
                "testIntoBitSetAfterMatchesOnSparseRegion",
                test_into_bit_set_after_matches_on_sparse_region,
            ),
        ],
    )
}

fn value_in_range(doc: i32) -> bool {
    let d = doc % 1024;
    let value = if d < 128 {
        (QUERY_MIN + QUERY_MAX) >> 1
    } else if d < 256 {
        QUERY_MAX + 1
    } else if d < 512 {
        QUERY_MIN - 1
    } else {
        match (d / 2) % 3 {
            0 => QUERY_MIN - 1,
            1 => QUERY_MAX + 1,
            2 => (QUERY_MIN + QUERY_MAX) >> 1,
            _ => {
                randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
            }
        }
    };
    value >= QUERY_MIN && value <= QUERY_MAX
}

fn doc_has_value(doc: i32) -> bool {
    doc < DENSE_END || (doc < 2048 && (doc & 1) == 0)
}

/// Compute expected matching docs. With the corrected aggregating skipper,
/// YES and YES_IF_PRESENT blocks only contain docs whose values are
/// genuinely within the query range, so the expected matches are simply all
/// docs that have a value in range.
fn expected_matches() -> Vec<i32> {
    let mut matches = Vec::new();
    for doc in 0..2048 {
        if doc_has_value(doc) && value_in_range(doc) {
            matches.push(doc);
        }
    }
    matches
}

fn create_iterator(multi_level: bool) -> Result<DocValuesRangeIterator> {
    let values = doc_values(QUERY_MIN, QUERY_MAX)?;
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, multi_level);
    Ok(DocValuesRangeIterator::for_range(
        values,
        Some(skipper),
        QUERY_MIN,
        QUERY_MAX,
    ))
}

// --- Correctness: all matching docs are found and no false positives ---

fn test_correct_results_single_level() -> Result<()> {
    assert_equals(
        &expected_matches(),
        &collect_matches(&mut create_iterator(false)?)?,
    );
    Ok(())
}

fn test_correct_results_multiple_levels() -> Result<()> {
    assert_equals(
        &expected_matches(),
        &collect_matches(&mut create_iterator(true)?)?,
    );
    Ok(())
}

// --- Approximation correctly skips non-matching blocks ---

fn test_approximation_skips_above_range_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // Docs 128-511 are in blocks above or below the query range.
    // The first MAYBE block starts at 512 (where mixed values begin).
    assert_equals(&512, &approx(&mut iter).advance(128)?);
    Ok(())
}

fn test_approximation_skips_below_range_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // Docs 256-511 are below range; skip to the first MAYBE block at 512.
    assert_equals(&512, &approx(&mut iter).advance(300)?);
    Ok(())
}

fn test_approximation_skips_from_end_of_matching_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    assert_equals(&100, &approx(&mut iter).advance(100)?);
    // Advancing past the last YES block boundary at 127 should skip to 512
    assert_equals(&512, &approx(&mut iter).advance(128)?);
    Ok(())
}

fn test_approximation_iterates_within_matching_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    assert_equals(&10, &approx(&mut iter).advance(10)?);
    assert_equals(&11, &approx(&mut iter).next_doc()?);
    assert_equals(&12, &approx(&mut iter).next_doc()?);
    assert_equals(&50, &approx(&mut iter).advance(50)?);
    Ok(())
}

fn test_approximation_skips_sparse_non_matching_blocks() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // In the sparse region (1024-2047), docs 1152-1535 are in non-matching
    // blocks (above-range then below-range), so they should be skipped.
    assert_equals(&1536, &approx(&mut iter).advance(1200)?);
    Ok(())
}

fn test_approximation_exhausted() -> Result<()> {
    let mut iter = create_iterator(true)?;

    assert_equals(&NO_MORE_DOCS, &approx(&mut iter).advance(2048)?);
    Ok(())
}

// --- Match classification ---

fn test_yes_match_in_dense_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Ok(())
}

fn test_yes_match_does_not_advance_doc_values() -> Result<()> {
    let (values, values_doc) = doc_values_shared(QUERY_MIN, QUERY_MAX)?;
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_range(values, Some(skipper), QUERY_MIN, QUERY_MAX);

    approx(&mut iter).advance(100)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Assert::assert_true(values_doc.load(Ordering::Relaxed) < approx(&mut iter).doc_id());
    Ok(())
}

fn test_maybe_match_classification() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // Docs 512+ are in blocks with mixed values -> MAYBE
    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    Ok(())
}

fn test_maybe_match_checks_values() -> Result<()> {
    let mut iter = create_iterator(true)?;

    approx(&mut iter).advance(512)?;

    // d=512: (512/2)%3 = 1 -> value above range
    Assert::assert_false(iter.matches()?);

    // d=514: (514/2)%3 = 2 -> value in range
    approx(&mut iter).advance(514)?;
    Assert::assert_true(iter.matches()?);

    // d=515: (515/2)%3 = 2 -> value in range
    approx(&mut iter).advance(515)?;
    Assert::assert_true(iter.matches()?);

    // d=516: (516/2)%3 = 0 -> value below range
    approx(&mut iter).advance(516)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

fn test_yes_if_present_in_sparse_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // Sparse block with values in range: all values match but not all docs
    // have a value, so the block is classified YES_IF_PRESENT.
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    // Even doc has a value -> match
    Assert::assert_true(iter.matches()?);

    approx(&mut iter).advance(1089)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    // Odd doc has no value -> no match
    Assert::assert_false(iter.matches()?);
    Ok(())
}

// --- docIdRunEnd ---

fn test_doc_id_run_end_yes_block_multiple_levels() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // 3 levels: level 0 blocks are 64 docs wide.
    // At doc 0: level 0 block is 0-63 (YES). Level 1 block 0-127 also has
    // all values in range, so docIdRunEnd extends to 128. But level 2 block
    // 0-255 includes docs 128-255 with values above range, so the
    // containment check fails and the run stops at the level 1 boundary.
    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&128, &iter.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_extends_from_middle_of_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // At doc 64: level 0 block is 64-127, level 1 is 0-127 (all in range).
    // Level 2 is 0-255 which includes above-range values -> stop at level 1.
    approx(&mut iter).advance(64)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&128, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(100)?;
    assert_equals(&128, &iter.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_consistent_across_yes_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // With 3 levels, docs 0-127 are in YES blocks (two level-0 blocks:
    // 0-63 and 64-127). Both extend to the level 1 boundary at 128, but
    // not further (level 2 includes above-range docs 128-255).
    for doc in 0..128 {
        approx(&mut iter).advance(doc)?;
        assert_equals(&Match::Yes, &approx(&mut iter).get_match());
        assert_equals_msg(
            Some(&format!(
                "docIdRunEnd at doc {doc} should extend to level 1 boundary"
            )),
            &128,
            &iter.doc_id_run_end()?,
        );
    }
    Ok(())
}

fn test_doc_id_run_end_maybe_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // MAYBE blocks cannot guarantee all docs match, so docIdRunEnd = doc + 1
    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    assert_equals(&513, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(800)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    assert_equals(&801, &iter.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_yes_if_present_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // YES_IF_PRESENT blocks have gaps, so docIdRunEnd = doc + 1
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    assert_equals(&1089, &iter.doc_id_run_end()?);
    Ok(())
}

fn test_into_bit_set_after_matches_on_sparse_region() -> Result<()> {
    let mut iter = create_iterator(true)?;
    // Doc 1537 is in the sparse region, in a block with mixed values -> MAYBE
    approx(&mut iter).advance(1537)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    Assert::assert_false(iter.matches()?);
    let mut bit_set = FixedBitSet::new(2048);
    iter.into_bit_set(2048, &mut bit_set, 0)?;
    let mut expected = FixedBitSet::new(2048);
    for doc in 1537..2048 {
        if doc_has_value(doc) && value_in_range(doc) {
            expected.set(doc as usize);
        }
    }
    assert_equals(&expected, &bit_set);
    Ok(())
}
