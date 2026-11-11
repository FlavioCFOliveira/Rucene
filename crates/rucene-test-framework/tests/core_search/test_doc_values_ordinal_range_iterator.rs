//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDocValuesOrdinalRangeIterator.java`.
//!
//! Tests `DocValuesRangeIterator#forOrdinalRange` for both single-valued
//! (`SortedDocValues`) and multi-valued (`SortedSetDocValues`) ordinals.
//! Parallel to `TestDocValuesRangeIterator` and
//! `TestSortedNumericDocValuesRangeIterator`.
//!
//! Java's `forOrdinalRange` is overloaded on the values' type; Rucene names
//! the `SortedSetDocValues` overload `for_sorted_set_ordinal_range`.
//! `testSortedSetYesMatchDoesNotAdvanceDocValues` reads `values.docID()`
//! after handing the values to the iterator: the fake values here share
//! their current doc with the test through an `Arc`, which stands for the
//! reference Java keeps.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::index::{DocValuesIterator, DocValuesSkipper, SortedDocValues, SortedSetDocValues};
use rucene::search::{
    DocIdSetIterator, DocValuesRangeIterator, Match, TwoPhaseIterator, NO_MORE_DOCS,
};
use rucene::util::{BytesRef, FixedBitSet};

use crate::core_search::base_doc_values_skipper_tests::{
    approx, collect_matches, doc_values_skipper, DENSE_END,
};
use crate::core_support::{assert_equals, class, test};

const QUERY_MIN: i64 = 10;
const QUERY_MAX: i64 = 20;

/// `TestDocValuesOrdinalRangeIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDocValuesOrdinalRangeIterator",
        vec![
            test(
                "testSortedDocValuesCorrectResultsSingleLevel",
                test_sorted_doc_values_correct_results_single_level,
            ),
            test(
                "testSortedDocValuesCorrectResultsMultipleLevels",
                test_sorted_doc_values_correct_results_multiple_levels,
            ),
            test(
                "testSortedDocValuesApproximationSkips",
                test_sorted_doc_values_approximation_skips,
            ),
            test(
                "testSortedDocValuesYesMatch",
                test_sorted_doc_values_yes_match,
            ),
            test(
                "testSortedDocValuesMaybeMatch",
                test_sorted_doc_values_maybe_match,
            ),
            test(
                "testSortedDocValuesYesIfPresent",
                test_sorted_doc_values_yes_if_present,
            ),
            test(
                "testSortedDocValuesDocIdRunEnd",
                test_sorted_doc_values_doc_id_run_end,
            ),
            test(
                "testSortedDocValuesIntoBitSetMatchesLinearScan",
                test_sorted_doc_values_into_bit_set_matches_linear_scan,
            ),
            test(
                "testSortedSetDocValuesCorrectResultsSingleLevel",
                test_sorted_set_doc_values_correct_results_single_level,
            ),
            test(
                "testSortedSetDocValuesCorrectResultsMultipleLevels",
                test_sorted_set_doc_values_correct_results_multiple_levels,
            ),
            test(
                "testSortedSetMatchOnFirstOrdinal",
                test_sorted_set_match_on_first_ordinal,
            ),
            test(
                "testSortedSetMatchOnSecondOrdinal",
                test_sorted_set_match_on_second_ordinal,
            ),
            test(
                "testSortedSetNoMatchWhenOrdinalsBracketRange",
                test_sorted_set_no_match_when_ordinals_bracket_range,
            ),
            test(
                "testSortedSetApproximationSkipsAboveRange",
                test_sorted_set_approximation_skips_above_range,
            ),
            test(
                "testSortedSetApproximationSkipsBelowRange",
                test_sorted_set_approximation_skips_below_range,
            ),
            test(
                "testSortedSetApproximationSkipsSparseNonMatching",
                test_sorted_set_approximation_skips_sparse_non_matching,
            ),
            test(
                "testSortedSetApproximationExhausted",
                test_sorted_set_approximation_exhausted,
            ),
            test("testSortedSetYesMatch", test_sorted_set_yes_match),
            test(
                "testSortedSetYesMatchDoesNotAdvanceDocValues",
                test_sorted_set_yes_match_does_not_advance_doc_values,
            ),
            test(
                "testSortedSetMaybeMatchChecksMultipleOrdinals",
                test_sorted_set_maybe_match_checks_multiple_ordinals,
            ),
            test("testSortedSetYesIfPresent", test_sorted_set_yes_if_present),
            test("testSortedSetDocIdRunEnd", test_sorted_set_doc_id_run_end),
            test(
                "testSortedSetDocValuesIntoBitSetMatchesLinearScan",
                test_sorted_set_doc_values_into_bit_set_matches_linear_scan,
            ),
        ],
    )
}

// ---- Shared helpers ----

fn doc_has_value(doc: i32) -> bool {
    doc < DENSE_END || (doc < 2048 && (doc & 1) == 0)
}

/// Drives `DocValuesRangeIterator#intoBitSet` over the whole doc space and
/// asserts the bulk result matches the documents a per-doc
/// `TwoPhaseIterator#matches()` linear scan would produce (the `expected`
/// list, computed independently from the mock's ordinals).
fn assert_into_bit_set_matches_linear_scan(
    iter: &mut DocValuesRangeIterator,
    expected_matches: &[i32],
) -> Result<()> {
    assert_equals(&0, &iter.approximation().next_doc()?);

    let mut actual = FixedBitSet::new(2048);
    iter.into_bit_set(2048, &mut actual, 0)?;
    // intoBitSet must leave the approximation on the first doc >= upTo (exhausted here).
    assert_equals(&NO_MORE_DOCS, &iter.approximation().doc_id());

    let mut expected = FixedBitSet::new(2048);
    for &doc in expected_matches {
        expected.set(doc as usize);
    }
    assert_equals(&expected.cardinality(), &actual.cardinality());
    expected.xor(&actual);
    assert_equals(&0, &expected.cardinality());
    Ok(())
}

/// The shared `advance` of the two fake doc values.
fn fake_advance(target: i32) -> i32 {
    if target >= 2048 {
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
    }
}

fn unsupported<T>() -> Result<T> {
    Err(LuceneError::unsupported_operation(String::new()))
}

// ==========================================================================
// SortedDocValues (single-valued ordinals)
// ==========================================================================

fn single_ordinal(doc: i32, query_min: i64, query_max: i64) -> i32 {
    let d = doc % 1024;
    if d < 128 {
        ((query_min + query_max) >> 1) as i32
    } else if d < 256 {
        (query_max + 1) as i32
    } else if d < 512 {
        (query_min - 1) as i32
    } else {
        match (d / 2) % 3 {
            0 => (query_min - 1) as i32,
            1 => (query_max + 1) as i32,
            2 => ((query_min + query_max) >> 1) as i32,
            _ => {
                randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
            }
        }
    }
}

fn single_ord_in_range(doc: i32) -> bool {
    let ord = i64::from(single_ordinal(doc, QUERY_MIN, QUERY_MAX));
    (QUERY_MIN..=QUERY_MAX).contains(&ord)
}

fn expected_single_ord_matches() -> Vec<i32> {
    let mut matches = Vec::new();
    for doc in 0..2048 {
        if doc_has_value(doc) && single_ord_in_range(doc) {
            matches.push(doc);
        }
    }
    matches
}

/// The anonymous `SortedDocValues` of `sortedDocValues`.
struct FakeSortedDocValues {
    doc: i32,
    query_min: i64,
    query_max: i64,
}

impl DocIdSetIterator for FakeSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = fake_advance(target);
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        42
    }
}

impl DocValuesIterator for FakeSortedDocValues {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        unsupported()
    }
}

impl SortedDocValues for FakeSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(single_ordinal(self.doc, self.query_min, self.query_max))
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok((self.query_max + 10) as i32)
    }

    fn lookup_ord(&self, _ord: i32) -> Result<BytesRef> {
        unsupported()
    }
}

fn sorted_doc_values(query_min: i64, query_max: i64) -> Box<dyn SortedDocValues> {
    Box::new(FakeSortedDocValues {
        doc: -1,
        query_min,
        query_max,
    })
}

fn create_sorted_doc_values_iterator(multi_level: bool) -> DocValuesRangeIterator {
    let values = sorted_doc_values(QUERY_MIN, QUERY_MAX);
    let skipper: Box<dyn DocValuesSkipper> = doc_values_skipper(QUERY_MIN, QUERY_MAX, multi_level);
    DocValuesRangeIterator::for_ordinal_range(values, Some(skipper), QUERY_MIN, QUERY_MAX)
}

// --- SortedDocValues: Correctness ---

fn test_sorted_doc_values_correct_results_single_level() -> Result<()> {
    assert_equals(
        &expected_single_ord_matches(),
        &collect_matches(&mut create_sorted_doc_values_iterator(false))?,
    );
    Ok(())
}

fn test_sorted_doc_values_correct_results_multiple_levels() -> Result<()> {
    assert_equals(
        &expected_single_ord_matches(),
        &collect_matches(&mut create_sorted_doc_values_iterator(true))?,
    );
    Ok(())
}

// --- SortedDocValues: Approximation skipping ---

fn test_sorted_doc_values_approximation_skips() -> Result<()> {
    let mut iter = create_sorted_doc_values_iterator(true);

    assert_equals(&512, &approx(&mut iter).advance(128)?);
    assert_equals(&1536, &approx(&mut iter).advance(1200)?);
    assert_equals(&NO_MORE_DOCS, &approx(&mut iter).advance(2048)?);
    Ok(())
}

// --- SortedDocValues: Match classification ---

fn test_sorted_doc_values_yes_match() -> Result<()> {
    let mut iter = create_sorted_doc_values_iterator(true);

    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Ok(())
}

fn test_sorted_doc_values_maybe_match() -> Result<()> {
    let mut iter = create_sorted_doc_values_iterator(true);

    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());

    // d=512: (512/2)%3 = 1 → ord above range
    Assert::assert_false(iter.matches()?);

    // d=514: (514/2)%3 = 2 → ord in range
    approx(&mut iter).advance(514)?;
    Assert::assert_true(iter.matches()?);

    // d=516: (516/2)%3 = 0 → ord below range
    approx(&mut iter).advance(516)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

fn test_sorted_doc_values_yes_if_present() -> Result<()> {
    let mut iter = create_sorted_doc_values_iterator(true);

    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);

    approx(&mut iter).advance(1089)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

// --- SortedDocValues: docIdRunEnd ---

fn test_sorted_doc_values_doc_id_run_end() -> Result<()> {
    let mut iter = create_sorted_doc_values_iterator(true);

    approx(&mut iter).advance(0)?;
    assert_equals(&128, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(64)?;
    assert_equals(&128, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(512)?;
    assert_equals(&513, &iter.doc_id_run_end()?);

    // YES block in second repetition (dense up to DENSE_END=1088)
    approx(&mut iter).advance(1024)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&1088, &iter.doc_id_run_end()?);

    // YES_IF_PRESENT block
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    assert_equals(&1089, &iter.doc_id_run_end()?);
    Ok(())
}

// --- SortedDocValues: intoBitSet bulk evaluation ---

fn test_sorted_doc_values_into_bit_set_matches_linear_scan() -> Result<()> {
    assert_into_bit_set_matches_linear_scan(
        &mut create_sorted_doc_values_iterator(true),
        &expected_single_ord_matches(),
    )
}

// ==========================================================================
// SortedSetDocValues (multi-valued ordinals)
// ==========================================================================

/// Returns the two sorted ordinals for a given doc. Same block-level
/// min/max as the single-valued case, but with multi-value behaviors:
///
/// * Case 0 (mixed region): ords bracket the range — no match
/// * Case 1 (mixed region): first ord in range — immediate match
/// * Case 2 (mixed region): only second ord in range — match via iteration
fn ordinal_pair(doc: i32, query_min: i64, query_max: i64) -> [i64; 2] {
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

fn multi_ord_in_range(doc: i32) -> bool {
    let ords = ordinal_pair(doc, QUERY_MIN, QUERY_MAX);
    for ord in ords {
        if (QUERY_MIN..=QUERY_MAX).contains(&ord) {
            return true;
        }
    }
    false
}

fn expected_multi_ord_matches() -> Vec<i32> {
    let mut matches = Vec::new();
    for doc in 0..2048 {
        if doc_has_value(doc) && multi_ord_in_range(doc) {
            matches.push(doc);
        }
    }
    matches
}

/// The anonymous `SortedSetDocValues` of `sortedSetDocValues`; `doc` is
/// shared with the test (see the module documentation).
struct FakeSortedSetDocValues {
    doc: Arc<AtomicI32>,
    ord_idx: usize,
    query_min: i64,
    query_max: i64,
}

impl DocIdSetIterator for FakeSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.doc.load(Ordering::Relaxed)
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc_id() + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.ord_idx = 0;
        let doc = fake_advance(target);
        self.doc.store(doc, Ordering::Relaxed);
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        42
    }
}

impl DocValuesIterator for FakeSortedSetDocValues {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        unsupported()
    }
}

impl SortedSetDocValues for FakeSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        let ords = ordinal_pair(self.doc_id(), self.query_min, self.query_max);
        let ord = ords[self.ord_idx];
        self.ord_idx += 1;
        Ok(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(2)
    }

    fn lookup_ord(&self, _ord: i64) -> Result<BytesRef> {
        unsupported()
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.query_max + 10)
    }
}

/// `sortedSetDocValues(queryMin, queryMax)`, with the handle on its current
/// doc.
fn sorted_set_doc_values(
    query_min: i64,
    query_max: i64,
) -> (Box<dyn SortedSetDocValues>, Arc<AtomicI32>) {
    let doc = Arc::new(AtomicI32::new(-1));
    (
        Box::new(FakeSortedSetDocValues {
            doc: Arc::clone(&doc),
            ord_idx: 0,
            query_min,
            query_max,
        }),
        doc,
    )
}

fn create_sorted_set_doc_values_iterator(multi_level: bool) -> DocValuesRangeIterator {
    let (values, _) = sorted_set_doc_values(QUERY_MIN, QUERY_MAX);
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, multi_level);
    DocValuesRangeIterator::for_sorted_set_ordinal_range(
        values,
        Some(skipper),
        QUERY_MIN,
        QUERY_MAX,
    )
}

// --- SortedSetDocValues: Correctness ---

fn test_sorted_set_doc_values_correct_results_single_level() -> Result<()> {
    assert_equals(
        &expected_multi_ord_matches(),
        &collect_matches(&mut create_sorted_set_doc_values_iterator(false))?,
    );
    Ok(())
}

fn test_sorted_set_doc_values_correct_results_multiple_levels() -> Result<()> {
    assert_equals(
        &expected_multi_ord_matches(),
        &collect_matches(&mut create_sorted_set_doc_values_iterator(true))?,
    );
    Ok(())
}

// --- SortedSetDocValues: Multi-value specific behavior ---

fn test_sorted_set_match_on_first_ordinal() -> Result<()> {
    // Doc 512: (512/2)%3 = 1 → ords [queryMin, queryMax+1]
    // First ordinal is in range → immediate match.
    let mut iter = create_sorted_set_doc_values_iterator(true);
    approx(&mut iter).advance(512)?;
    Assert::assert_true(iter.matches()?);
    Ok(())
}

fn test_sorted_set_match_on_second_ordinal() -> Result<()> {
    // Doc 514: (514/2)%3 = 2 → ords [queryMin-1, mid]
    // First ordinal below range, second ordinal in range → match.
    let mut iter = create_sorted_set_doc_values_iterator(true);
    approx(&mut iter).advance(514)?;
    Assert::assert_true(iter.matches()?);
    Ok(())
}

fn test_sorted_set_no_match_when_ordinals_bracket_range() -> Result<()> {
    // Doc 516: (516/2)%3 = 0 → ords [queryMin-1, queryMax+1]
    // Ordinals span below and above range, but none falls within it.
    let mut iter = create_sorted_set_doc_values_iterator(true);
    approx(&mut iter).advance(516)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

// --- SortedSetDocValues: Approximation skipping ---

fn test_sorted_set_approximation_skips_above_range() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);
    assert_equals(&512, &approx(&mut iter).advance(128)?);
    Ok(())
}

fn test_sorted_set_approximation_skips_below_range() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);
    assert_equals(&512, &approx(&mut iter).advance(300)?);
    Ok(())
}

fn test_sorted_set_approximation_skips_sparse_non_matching() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);
    assert_equals(&1536, &approx(&mut iter).advance(1200)?);
    Ok(())
}

fn test_sorted_set_approximation_exhausted() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);
    assert_equals(&NO_MORE_DOCS, &approx(&mut iter).advance(2048)?);
    Ok(())
}

// --- SortedSetDocValues: Match classification ---

fn test_sorted_set_yes_match() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);

    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Ok(())
}

fn test_sorted_set_yes_match_does_not_advance_doc_values() -> Result<()> {
    let (values, values_doc) = sorted_set_doc_values(QUERY_MIN, QUERY_MAX);
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_sorted_set_ordinal_range(
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

fn test_sorted_set_maybe_match_checks_multiple_ordinals() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);

    // d=512: case 1 → [queryMin, queryMax+1] → match on first ordinal
    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);

    // d=514: case 2 → [queryMin-1, mid] → match on second ordinal
    approx(&mut iter).advance(514)?;
    Assert::assert_true(iter.matches()?);

    // d=516: case 0 → [queryMin-1, queryMax+1] → no match (brackets range)
    approx(&mut iter).advance(516)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

fn test_sorted_set_yes_if_present() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);

    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);

    approx(&mut iter).advance(1089)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

// --- SortedSetDocValues: docIdRunEnd ---

fn test_sorted_set_doc_id_run_end() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true);

    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&128, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(64)?;
    assert_equals(&128, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    assert_equals(&513, &iter.doc_id_run_end()?);

    // YES block in second repetition (dense up to DENSE_END=1088)
    approx(&mut iter).advance(1024)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&1088, &iter.doc_id_run_end()?);

    // YES_IF_PRESENT block
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    assert_equals(&1089, &iter.doc_id_run_end()?);
    Ok(())
}

// --- SortedSetDocValues: intoBitSet bulk evaluation ---

fn test_sorted_set_doc_values_into_bit_set_matches_linear_scan() -> Result<()> {
    assert_into_bit_set_matches_linear_scan(
        &mut create_sorted_set_doc_values_iterator(true),
        &expected_multi_ord_matches(),
    )
}
