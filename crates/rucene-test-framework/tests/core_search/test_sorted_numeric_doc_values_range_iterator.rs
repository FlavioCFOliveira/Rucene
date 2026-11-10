//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSortedNumericDocValuesRangeIterator.java`.
//!
//! Tests `DocValuesRangeIterator#forRange(SortedNumericDocValues, DocValuesSkipper, long,
//! long)` with multi-valued documents. Parallel to `TestDocValuesRangeIterator` which tests
//! single-valued `org.apache.lucene.index.NumericDocValues`.
//!
//! Each document has two values in sorted order. The three cases in the mixed region exercise
//! distinct multi-value behaviors:
//!
//! * Case 0: values bracket the query range (below and above) — no match
//! * Case 1: first value is in range — immediate match
//! * Case 2: only the second value is in range — match via iteration
//!
//! `testYesMatchDoesNotAdvanceDocValues` reads `values.docID()` after
//! handing the values to the iterator; a shared handle on the fake values'
//! `doc` stands for that reference (the asserting wrapper's `docID()` is the
//! wrapped values' one).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::{DocValuesIterator, SortedNumericDocValues};
use rucene::search::{
    DocIdSetIterator, DocValuesRangeIterator, Match, TwoPhaseIterator, NO_MORE_DOCS,
};
use rucene::util::FixedBitSet;
use rucene_test_framework::index::asserting_leaf_reader::AssertingSortedNumericDocValues;

use crate::core_search::base_doc_values_skipper_tests::{
    approx, collect_matches, doc_values_skipper, DENSE_END, MAX_DOC,
};
use crate::core_support::{assert_equals, assert_equals_msg, class, test};

const QUERY_MIN: i64 = 10;
const QUERY_MAX: i64 = 20;

/// `TestSortedNumericDocValuesRangeIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSortedNumericDocValuesRangeIterator",
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
                "testTwoPhaseIntoBitSetMatchesExpected",
                test_two_phase_into_bit_set_matches_expected,
            ),
            test(
                "testDefaultRangeIntoBitSetMatchesExpected",
                test_default_range_into_bit_set_matches_expected,
            ),
            test(
                "testMatchOnFirstValueInRange",
                test_match_on_first_value_in_range,
            ),
            test("testMatchOnSecondValue", test_match_on_second_value),
            test(
                "testNoMatchWhenValuesBracketRange",
                test_no_match_when_values_bracket_range,
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
            test(
                "testMaybeMatchChecksMultipleValues",
                test_maybe_match_checks_multiple_values,
            ),
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

/// Returns the two sorted values for a given doc. Block-level min/max values are consistent with
/// the skipper from `BaseDocValuesSkipperTests#docValuesSkipper`.
fn doc_value_pair(doc: i32, query_min: i64, query_max: i64) -> [i64; 2] {
    let d = doc % 1024;
    let mid = (query_min + query_max) >> 1;
    if d < 128 {
        [query_min, query_max]
    } else if d < 256 {
        [query_max + 1, query_max + 1]
    } else if d < 512 {
        [query_min - 1, query_min - 1]
    } else {
        match (d / 2) % 3 {
            0 => [query_min - 1, query_max + 1],
            1 => [query_min, query_max + 1],
            2 => [query_min - 1, mid],
            _ => {
                randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
            }
        }
    }
}

fn value_in_range(doc: i32) -> bool {
    let vals = doc_value_pair(doc, QUERY_MIN, QUERY_MAX);
    for v in vals {
        if (QUERY_MIN..=QUERY_MAX).contains(&v) {
            return true;
        }
    }
    false
}

fn doc_has_value(doc: i32) -> bool {
    doc < DENSE_END || (doc < 2048 && (doc & 1) == 0)
}

fn expected_matches() -> Vec<i32> {
    let mut matches = Vec::new();
    for doc in 0..2048 {
        if doc_has_value(doc) && value_in_range(doc) {
            matches.push(doc);
        }
    }
    matches
}

/// `sortedNumericDocValues(queryMin, queryMax)` with a handle on the fake
/// values' current doc.
fn sorted_numeric_doc_values_shared(
    query_min: i64,
    query_max: i64,
) -> Result<(Box<dyn SortedNumericDocValues>, Arc<AtomicI32>)> {
    let doc = Arc::new(AtomicI32::new(-1));
    let values = new_sorted_numeric_doc_values(query_min, query_max, Arc::clone(&doc));
    // Enforce the forward-only iteration contracts like real sparse implementations rely on.
    Ok((
        AssertingSortedNumericDocValues::create(Box::new(values), MAX_DOC)?,
        doc,
    ))
}

/// `private static SortedNumericDocValues sortedNumericDocValues(long queryMin, long queryMax)`.
fn sorted_numeric_doc_values(
    query_min: i64,
    query_max: i64,
) -> Result<Box<dyn SortedNumericDocValues>> {
    Ok(sorted_numeric_doc_values_shared(query_min, query_max)?.0)
}

/// The anonymous `SortedNumericDocValues` of `newSortedNumericDocValues`.
struct FakeSortedNumericDocValues {
    doc: Arc<AtomicI32>,
    value_idx: usize,
    query_min: i64,
    query_max: i64,
}

/// `private static SortedNumericDocValues newSortedNumericDocValues(long queryMin, long queryMax)`.
fn new_sorted_numeric_doc_values(
    query_min: i64,
    query_max: i64,
    doc: Arc<AtomicI32>,
) -> FakeSortedNumericDocValues {
    FakeSortedNumericDocValues {
        doc,
        value_idx: 0,
        query_min,
        query_max,
    }
}

impl DocIdSetIterator for FakeSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc.load(Ordering::Relaxed)
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc_id() + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.value_idx = 0;
        let doc = if target >= 2048 {
            NO_MORE_DOCS
        } else if target < DENSE_END {
            target
        } else {
            let d = target + (target & 1);
            if d >= 2048 {
                NO_MORE_DOCS
            } else {
                d
            }
        };
        self.doc.store(doc, Ordering::Relaxed);
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        42
    }
}

impl DocValuesIterator for FakeSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc.store(target, Ordering::Relaxed);
        self.value_idx = 0;
        Ok(doc_has_value(target))
    }
}

impl SortedNumericDocValues for FakeSortedNumericDocValues {
    fn doc_value_count(&self) -> Result<i32> {
        Ok(2)
    }

    fn next_value(&mut self) -> Result<i64> {
        let vals = doc_value_pair(self.doc_id(), self.query_min, self.query_max);
        let v = vals[self.value_idx];
        self.value_idx += 1;
        Ok(v)
    }
}

fn create_iterator(multi_level: bool) -> Result<DocValuesRangeIterator> {
    let values = sorted_numeric_doc_values(QUERY_MIN, QUERY_MAX)?;
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, multi_level);
    Ok(DocValuesRangeIterator::for_sorted_numeric_range(
        values,
        Some(skipper),
        QUERY_MIN,
        QUERY_MAX,
    ))
}

fn expected_bit_set() -> FixedBitSet {
    let mut bit_set = FixedBitSet::new(2048);
    for doc in expected_matches() {
        bit_set.set(doc as usize);
    }
    bit_set
}

// --- Correctness: all matching docs are found and no false positives ---

/// Port of `TestSortedNumericDocValuesRangeIterator.testCorrectResultsSingleLevel`.
fn test_correct_results_single_level() -> Result<()> {
    assert_equals(
        &expected_matches(),
        &collect_matches(&mut create_iterator(false)?)?,
    );
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testCorrectResultsMultipleLevels`.
fn test_correct_results_multiple_levels() -> Result<()> {
    assert_equals(
        &expected_matches(),
        &collect_matches(&mut create_iterator(true)?)?,
    );
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testTwoPhaseIntoBitSetMatchesExpected`.
fn test_two_phase_into_bit_set_matches_expected() -> Result<()> {
    let mut iterator = create_iterator(true)?;
    assert_equals(&0, &iterator.approximation().next_doc()?);

    let mut actual = FixedBitSet::new(2048);
    iterator.into_bit_set(2048, &mut actual, 0)?;

    assert_equals(&expected_bit_set(), &actual);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testDefaultRangeIntoBitSetMatchesExpected`.
fn test_default_range_into_bit_set_matches_expected() -> Result<()> {
    let mut values = sorted_numeric_doc_values(QUERY_MIN, QUERY_MAX)?;
    let mut actual = FixedBitSet::new(2048);
    values.range_into_bit_set(0, 2048, QUERY_MIN, QUERY_MAX, &mut actual, 0)?;

    assert_equals(&expected_bit_set(), &actual);
    Ok(())
}

// --- Multi-value specific behavior ---

/// Port of `TestSortedNumericDocValuesRangeIterator.testMatchOnFirstValueInRange`.
fn test_match_on_first_value_in_range() -> Result<()> {
    // Doc 512: (512/2)%3 = 1 → values [queryMin, queryMax+1]
    // First value is in range → immediate match.
    let mut iter = create_iterator(true)?;
    approx(&mut iter).advance(512)?;
    Assert::assert_true(iter.matches()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testMatchOnSecondValue`.
fn test_match_on_second_value() -> Result<()> {
    // Doc 514: (514/2)%3 = 2 → values [queryMin-1, mid]
    // First value below range (skipped), second value in range → match.
    let mut iter = create_iterator(true)?;
    approx(&mut iter).advance(514)?;
    Assert::assert_true(iter.matches()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testNoMatchWhenValuesBracketRange`.
fn test_no_match_when_values_bracket_range() -> Result<()> {
    // Doc 516: (516/2)%3 = 0 → values [queryMin-1, queryMax+1]
    // Values span below and above the range, but none falls within it.
    // The sorted-value optimization correctly rejects: first value < min
    // is skipped, second value >= min but > max → no match.
    let mut iter = create_iterator(true)?;
    approx(&mut iter).advance(516)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

// --- Approximation correctly skips non-matching blocks ---

/// Port of `TestSortedNumericDocValuesRangeIterator.testApproximationSkipsAboveRangeBlock`.
fn test_approximation_skips_above_range_block() -> Result<()> {
    let mut iter = create_iterator(true)?;
    assert_equals(&512, &approx(&mut iter).advance(128)?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testApproximationSkipsBelowRangeBlock`.
fn test_approximation_skips_below_range_block() -> Result<()> {
    let mut iter = create_iterator(true)?;
    assert_equals(&512, &approx(&mut iter).advance(300)?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testApproximationSkipsFromEndOfMatchingBlock`.
fn test_approximation_skips_from_end_of_matching_block() -> Result<()> {
    let mut iter = create_iterator(true)?;
    assert_equals(&100, &approx(&mut iter).advance(100)?);
    assert_equals(&512, &approx(&mut iter).advance(128)?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testApproximationIteratesWithinMatchingBlock`.
fn test_approximation_iterates_within_matching_block() -> Result<()> {
    let mut iter = create_iterator(true)?;
    assert_equals(&10, &approx(&mut iter).advance(10)?);
    assert_equals(&11, &approx(&mut iter).next_doc()?);
    assert_equals(&12, &approx(&mut iter).next_doc()?);
    assert_equals(&50, &approx(&mut iter).advance(50)?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testApproximationSkipsSparseNonMatchingBlocks`.
fn test_approximation_skips_sparse_non_matching_blocks() -> Result<()> {
    let mut iter = create_iterator(true)?;
    assert_equals(&1536, &approx(&mut iter).advance(1200)?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testApproximationExhausted`.
fn test_approximation_exhausted() -> Result<()> {
    let mut iter = create_iterator(true)?;
    assert_equals(&NO_MORE_DOCS, &approx(&mut iter).advance(2048)?);
    Ok(())
}

// --- Match classification ---

/// Port of `TestSortedNumericDocValuesRangeIterator.testYesMatchInDenseBlock`.
fn test_yes_match_in_dense_block() -> Result<()> {
    let mut iter = create_iterator(true)?;
    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testYesMatchDoesNotAdvanceDocValues`.
fn test_yes_match_does_not_advance_doc_values() -> Result<()> {
    let (values, values_doc) = sorted_numeric_doc_values_shared(QUERY_MIN, QUERY_MAX)?;
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_sorted_numeric_range(
        values,
        Some(skipper),
        QUERY_MIN,
        QUERY_MAX,
    );

    approx(&mut iter).advance(100)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Assert::assert_true(values_doc.load(Ordering::Relaxed) < approx(&mut iter).doc_id());
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testMaybeMatchClassification`.
fn test_maybe_match_classification() -> Result<()> {
    let mut iter = create_iterator(true)?;
    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testMaybeMatchChecksMultipleValues`.
fn test_maybe_match_checks_multiple_values() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // d=512: case 1 → [queryMin, queryMax+1] → match on first value
    approx(&mut iter).advance(512)?;
    Assert::assert_true(iter.matches()?);

    // d=514: case 2 → [queryMin-1, mid] → match on second value
    approx(&mut iter).advance(514)?;
    Assert::assert_true(iter.matches()?);

    // d=515: case 2 → same pair → match on second value
    approx(&mut iter).advance(515)?;
    Assert::assert_true(iter.matches()?);

    // d=516: case 0 → [queryMin-1, queryMax+1] → no match (brackets range)
    approx(&mut iter).advance(516)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testYesIfPresentInSparseBlock`.
fn test_yes_if_present_in_sparse_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    // Even doc has a value → match
    Assert::assert_true(iter.matches()?);

    approx(&mut iter).advance(1089)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    // Odd doc has no value → no match
    Assert::assert_false(iter.matches()?);
    Ok(())
}

// --- docIdRunEnd ---

/// Port of `TestSortedNumericDocValuesRangeIterator.testDocIdRunEndYesBlockMultipleLevels`.
fn test_doc_id_run_end_yes_block_multiple_levels() -> Result<()> {
    let mut iter = create_iterator(true)?;

    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&128, &iter.doc_id_run_end()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testDocIdRunEndExtendsFromMiddleOfBlock`.
fn test_doc_id_run_end_extends_from_middle_of_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    approx(&mut iter).advance(64)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&128, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(100)?;
    assert_equals(&128, &iter.doc_id_run_end()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testDocIdRunEndConsistentAcrossYesBlock`.
fn test_doc_id_run_end_consistent_across_yes_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

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

/// Port of `TestSortedNumericDocValuesRangeIterator.testDocIdRunEndMaybeBlock`.
fn test_doc_id_run_end_maybe_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    assert_equals(&513, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(800)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    assert_equals(&801, &iter.doc_id_run_end()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testDocIdRunEndYesIfPresentBlock`.
fn test_doc_id_run_end_yes_if_present_block() -> Result<()> {
    let mut iter = create_iterator(true)?;

    // YES block in second repetition (dense up to DENSE_END=1088)
    approx(&mut iter).advance(1024)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&1088, &iter.doc_id_run_end()?);

    // YES_IF_PRESENT blocks have gaps, so docIdRunEnd = doc + 1
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    assert_equals(&1089, &iter.doc_id_run_end()?);
    Ok(())
}

/// Port of `TestSortedNumericDocValuesRangeIterator.testIntoBitSetAfterMatchesOnSparseRegion`.
fn test_into_bit_set_after_matches_on_sparse_region() -> Result<()> {
    let mut iter = create_iterator(true)?;
    // Doc 1537 is in the sparse region, in a block with mixed values -> MAYBE
    approx(&mut iter).advance(1537)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    Assert::assert_false(iter.matches()?);
    let mut bit_set = FixedBitSet::new(MAX_DOC as usize);
    iter.into_bit_set(MAX_DOC, &mut bit_set, 0)?;
    let mut expected = FixedBitSet::new(MAX_DOC as usize);
    for doc in 1537..MAX_DOC {
        if doc_has_value(doc) && value_in_range(doc) {
            expected.set(doc as usize);
        }
    }
    assert_equals(&expected, &bit_set);
    Ok(())
}
