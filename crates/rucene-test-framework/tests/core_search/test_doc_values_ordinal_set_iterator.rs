//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDocValuesOrdinalSetIterator.java`.
//!
//! Tests `DocValuesRangeIterator#forOrdinalSet` for both single-valued
//! (`SortedDocValues`) and multi-valued (`SortedSetDocValues`) ordinals.
//!
//! Unlike the ordinal range tests, the ordinal set has a gap: ordinal 15
//! (the midpoint of [10, 20]) is excluded. This means docs with ordinal 15
//! are within the bounding range but NOT in the matching set, exercising the
//! per-doc predicate confirmation that arbitrary ordinal sets require.
//!
//! Java's `forOrdinalRange`/`forOrdinalSet` are overloaded on the values'
//! type; Rucene names the `SortedSetDocValues` overloads
//! `for_sorted_set_ordinal_range`/`for_sorted_set_ordinal_set`. Tests that
//! read `values.docID()` after handing the values to the iterator use the
//! handle on the fake values' current doc (an `Arc`) that stands for the
//! reference Java keeps.

use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::index::{
    DocValuesIterator, ImpactsEnum, PostingsEnum, SeekStatus, SortedDocValues, SortedSetDocValues,
    TermsEnum,
};
use rucene::search::{
    DocIdSetIterator, DocValuesRangeIterator, Match, TwoPhaseIterator, NO_MORE_DOCS,
};
use rucene::util::{AttributeSource, BytesRef};

use crate::core_search::base_doc_values_skipper_tests::{
    approx, collect_matches, doc_values_skipper,
};
use crate::core_support::{assert_equals, class, test};

const QUERY_MIN: i64 = 10;
const QUERY_MAX: i64 = 20;
const GAP_ORD: i64 = 15;

/// Matching ordinals: all of [10, 20] except 15.
const MATCHING_ORDS: &[i64] = &[10, 11, 12, 13, 14, 16, 17, 18, 19, 20];

/// `TestDocValuesOrdinalSetIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDocValuesOrdinalSetIterator",
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
                "testSortedDocValuesGapOrdinalRejected",
                test_sorted_doc_values_gap_ordinal_rejected,
            ),
            test(
                "testSortedDocValuesInSetOrdinalAccepted",
                test_sorted_doc_values_in_set_ordinal_accepted,
            ),
            test(
                "testSortedDocValuesYesBlockChecksPredicate",
                test_sorted_doc_values_yes_block_checks_predicate,
            ),
            test(
                "testSortedDocValuesApproximationSkips",
                test_sorted_doc_values_approximation_skips,
            ),
            test(
                "testSortedDocValuesContiguousOrdinalSetUsesRangeFastPath",
                test_sorted_doc_values_contiguous_ordinal_set_uses_range_fast_path,
            ),
            test(
                "testSortedDocValuesDuplicateOrdsNotMisclassifiedAsContiguous",
                test_sorted_doc_values_duplicate_ords_not_misclassified_as_contiguous,
            ),
            test(
                "testSortedDocValuesDocIdRunEndAlwaysDocPlusOne",
                test_sorted_doc_values_doc_id_run_end_always_doc_plus_one,
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
                "testSortedSetGapOrdinalRejected",
                test_sorted_set_gap_ordinal_rejected,
            ),
            test(
                "testSortedSetMatchOnFirstOrdinalInSet",
                test_sorted_set_match_on_first_ordinal_in_set,
            ),
            test(
                "testSortedSetNoMatchWhenOrdinalsBracketRange",
                test_sorted_set_no_match_when_ordinals_bracket_range,
            ),
            test(
                "testSortedSetYesBlockChecksPredicate",
                test_sorted_set_yes_block_checks_predicate,
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
            test(
                "testSortedSetDocIdRunEndAlwaysDocPlusOne",
                test_sorted_set_doc_id_run_end_always_doc_plus_one,
            ),
            test(
                "testSortedSetContiguousOrdinalSetUsesRangeFastPath",
                test_sorted_set_contiguous_ordinal_set_uses_range_fast_path,
            ),
            test("testSortedSetYesIfPresent", test_sorted_set_yes_if_present),
        ],
    )
}

fn unsupported<T>() -> Result<T> {
    Err(LuceneError::unsupported_operation(String::new()))
}

/// Fake TermsEnum that iterates over a fixed array of ordinals. Only
/// `next()` and `ord()` are needed by
/// `DocValuesRangeIterator#forOrdinalSet()`.
struct FakeTermsEnum {
    ordinals: Vec<i64>,
    idx: i32,
}

impl fmt::Display for FakeTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::core_search::support::object_to_string(
            "org.apache.lucene.search.TestDocValuesOrdinalSetIterator$1",
            self,
        ))
    }
}

impl TermsEnum for FakeTermsEnum {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.idx += 1;
        if self.idx as usize >= self.ordinals.len() {
            return Ok(None);
        }
        Ok(Some(BytesRef::new(
            format!("term{}", self.ordinals[self.idx as usize]).into_bytes(),
        )))
    }

    fn ord(&self) -> Result<i64> {
        Ok(self.ordinals[self.idx as usize])
    }

    fn attributes(&mut self) -> &mut AttributeSource {
        randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.UnsupportedOperationException",
        )
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        unsupported()
    }

    fn prepare_seek_exact<'a>(
        &'a mut self,
        _text: &BytesRef,
    ) -> Result<Option<Box<dyn rucene::util::IOBooleanSupplier + 'a>>> {
        unsupported()
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        unsupported()
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        unsupported()
    }

    fn seek_term_state(
        &mut self,
        _text: &BytesRef,
        _state: &dyn rucene::index::TermState,
    ) -> Result<()> {
        unsupported()
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        unsupported()
    }

    fn doc_freq(&mut self) -> Result<i32> {
        unsupported()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        unsupported()
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        unsupported()
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        unsupported()
    }

    fn term_state(&mut self) -> Result<Box<dyn rucene::index::TermState>> {
        unsupported()
    }
}

fn fake_terms_enum(ordinals: &[i64]) -> FakeTermsEnum {
    FakeTermsEnum {
        ordinals: ordinals.to_vec(),
        idx: -1,
    }
}

fn ord_in_set(ord: i64) -> bool {
    (QUERY_MIN..=QUERY_MAX).contains(&ord) && ord != GAP_ORD
}

// ---- Shared helpers ----

fn doc_has_value(doc: i32) -> bool {
    doc < 1024 || (doc < 2048 && (doc & 1) == 0)
}

/// The shared `advance` of the two fake doc values.
fn fake_advance(target: i32) -> i32 {
    if target < 1024 {
        target
    } else if target < 2048 {
        let doc = target + (target & 1);
        if doc < 2048 {
            doc
        } else {
            NO_MORE_DOCS
        }
    } else {
        NO_MORE_DOCS
    }
}

// ==========================================================================
// SortedDocValues (single-valued ordinals with a set)
// ==========================================================================

/// Single ordinal per doc. Uses ordinal 14 (in the set) for the in-range
/// region, and ordinal 15 (in the gap) for the mixed region's case 2.
fn single_ordinal(doc: i32) -> i32 {
    let d = doc % 1024;
    if d < 128 {
        14
    } else if d < 256 {
        (QUERY_MAX + 1) as i32
    } else if d < 512 {
        (QUERY_MIN - 1) as i32
    } else {
        match (d / 2) % 3 {
            0 => (QUERY_MIN - 1) as i32,
            1 => (QUERY_MAX + 1) as i32,
            2 => GAP_ORD as i32,
            _ => {
                randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
            }
        }
    }
}

fn expected_single_ord_matches() -> Vec<i32> {
    let mut matches = Vec::new();
    for doc in 0..2048 {
        if doc_has_value(doc) && ord_in_set(i64::from(single_ordinal(doc))) {
            matches.push(doc);
        }
    }
    matches
}

/// The anonymous `SortedDocValues` of `sortedDocValues`.
struct FakeSortedDocValues {
    doc: Arc<AtomicI32>,
}

impl DocIdSetIterator for FakeSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc.load(Ordering::Relaxed)
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc_id() + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = fake_advance(target);
        self.doc.store(doc, Ordering::Relaxed);
        Ok(doc)
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
        Ok(single_ordinal(self.doc_id()))
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok((QUERY_MAX + 10) as i32)
    }

    fn lookup_ord(&self, _ord: i32) -> Result<BytesRef> {
        unsupported()
    }
}

/// `sortedDocValues()`, with the handle on its current doc.
fn sorted_doc_values_shared() -> (Box<dyn SortedDocValues>, Arc<AtomicI32>) {
    let doc = Arc::new(AtomicI32::new(-1));
    (
        Box::new(FakeSortedDocValues {
            doc: Arc::clone(&doc),
        }),
        doc,
    )
}

fn sorted_doc_values() -> Box<dyn SortedDocValues> {
    sorted_doc_values_shared().0
}

fn create_sorted_doc_values_iterator(multi_level: bool) -> Result<DocValuesRangeIterator> {
    let values = sorted_doc_values();
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, multi_level);
    DocValuesRangeIterator::for_ordinal_set(
        values,
        Some(skipper),
        &mut fake_terms_enum(MATCHING_ORDS),
    )
}

// --- SortedDocValues: Correctness ---

fn test_sorted_doc_values_correct_results_single_level() -> Result<()> {
    assert_equals(
        &expected_single_ord_matches(),
        &collect_matches(&mut create_sorted_doc_values_iterator(false)?)?,
    );
    Ok(())
}

fn test_sorted_doc_values_correct_results_multiple_levels() -> Result<()> {
    assert_equals(
        &expected_single_ord_matches(),
        &collect_matches(&mut create_sorted_doc_values_iterator(true)?)?,
    );
    Ok(())
}

// --- SortedDocValues: Gap ordinal is rejected ---

fn test_sorted_doc_values_gap_ordinal_rejected() -> Result<()> {
    // Doc 514: (514/2)%3 = 2 → ordinal 15 (in [10,20] but NOT in set)
    // With forOrdinalRange this would match; with forOrdinalSet it must not.
    let mut iter = create_sorted_doc_values_iterator(true)?;
    approx(&mut iter).advance(514)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

fn test_sorted_doc_values_in_set_ordinal_accepted() -> Result<()> {
    // Doc 0: ordinal 14 (in set) → match
    let mut iter = create_sorted_doc_values_iterator(true)?;
    approx(&mut iter).advance(0)?;
    Assert::assert_true(iter.matches()?);
    Ok(())
}

// --- SortedDocValues: YES blocks still check predicate ---

fn test_sorted_doc_values_yes_block_checks_predicate() -> Result<()> {
    let (values, values_doc) = sorted_doc_values_shared();
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_ordinal_set(
        values,
        Some(skipper),
        &mut fake_terms_enum(MATCHING_ORDS),
    )?;

    // Block 0-63 is classified YES, but predicate is still checked
    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    // ordinal 14 is in the set → match
    Assert::assert_true(iter.matches()?);
    // doc values iterator was advanced (predicate was checked)
    assert_equals(&0, &values_doc.load(Ordering::Relaxed));
    Ok(())
}

// --- SortedDocValues: Approximation skipping ---

fn test_sorted_doc_values_approximation_skips() -> Result<()> {
    let mut iter = create_sorted_doc_values_iterator(true)?;

    assert_equals(&512, &approx(&mut iter).advance(128)?);
    assert_equals(&1536, &approx(&mut iter).advance(1200)?);
    assert_equals(&NO_MORE_DOCS, &approx(&mut iter).advance(2048)?);
    Ok(())
}

// --- SortedDocValues: contiguous ordinal set is converted to a range ---

/// When the ordinal set happens to be a contiguous range [min, max] (no
/// gaps), `forOrdinalSet` should behave as if `forOrdinalRange(min, max)`
/// had been called: cheaper per-doc predicate, YES blocks short-circuit
/// without consulting the doc values, and `docIDRunEnd` extends across the
/// block instead of returning `doc + 1`.
fn test_sorted_doc_values_contiguous_ordinal_set_uses_range_fast_path() -> Result<()> {
    // Contiguous: every ordinal in [10, 20] is present.
    let contiguous_ords: &[i64] = &[10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];

    let (values, values_doc) = sorted_doc_values_shared();
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_ordinal_set(
        values,
        Some(skipper),
        &mut fake_terms_enum(contiguous_ords),
    )?;

    // YES block: predicate must not be consulted, so the doc values iterator is not advanced.
    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Assert::assert_true_msg(
        "predicate should not have advanced doc values in a YES block when the ordinal set is a contiguous range",
        values_doc.load(Ordering::Relaxed) < approx(&mut iter).doc_id(),
    );

    // docIDRunEnd should extend across the YES run rather than collapsing to doc + 1, matching
    // forOrdinalRange semantics. With three skipper levels, the YES run extends to 128.
    assert_equals(&128, &iter.doc_id_run_end()?);

    // The gap-ordinal that a non-contiguous set would reject (doc 514 → ordinal 15) must now
    // match, confirming the predicate has been widened to the [min, max] range check.
    approx(&mut iter).advance(514)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);

    // End-to-end correctness: the contiguous-set iterator must produce the exact same docs as
    // a forOrdinalRange iterator over [10, 20].
    let range_matches = collect_matches(&mut DocValuesRangeIterator::for_ordinal_range(
        sorted_doc_values(),
        Some(doc_values_skipper(QUERY_MIN, QUERY_MAX, true)),
        QUERY_MIN,
        QUERY_MAX,
    ))?;
    let set_matches = collect_matches(&mut DocValuesRangeIterator::for_ordinal_set(
        sorted_doc_values(),
        Some(doc_values_skipper(QUERY_MIN, QUERY_MAX, true)),
        &mut fake_terms_enum(contiguous_ords),
    )?)?;
    assert_equals(&range_matches, &set_matches);
    Ok(())
}

// --- SortedDocValues: duplicate ords from TermsEnum must not fool contiguity detection ---

/// A misbehaving (or simply unusual) `TermsEnum` could yield the same ord
/// twice. The contiguity check must rely on the number of *distinct* ords,
/// not on the call count, otherwise a set like {10, 10, 12} would be
/// misclassified as the contiguous range [10, 12] and ordinal 11 would start
/// matching incorrectly.
fn test_sorted_doc_values_duplicate_ords_not_misclassified_as_contiguous() -> Result<()> {
    // {10, 10, 12}: three next() calls, but only two distinct ords with ord 11 missing.
    let ords_with_duplicate: &[i64] = &[10, 10, 12];

    let values = sorted_doc_values();
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_ordinal_set(
        values,
        Some(skipper),
        &mut fake_terms_enum(ords_with_duplicate),
    )?;

    // Doc 0 → ord 14: outside the {10, 12} set, must not match. With a buggy contiguity check
    // it would be treated as the range [10, 12] and still reject 14 — so we also probe an ord
    // inside the false range (11) to make sure it is rejected.
    approx(&mut iter).advance(0)?;
    Assert::assert_false(iter.matches()?);

    // The key signal: docIDRunEnd must collapse to doc+1 because the set is not contiguous and
    // every doc still needs per-doc predicate confirmation.
    assert_equals(&1, &iter.doc_id_run_end()?);
    Ok(())
}

// --- SortedDocValues: docIdRunEnd is conservative with ordinal sets ---

fn test_sorted_doc_values_doc_id_run_end_always_doc_plus_one() -> Result<()> {
    let mut iter = create_sorted_doc_values_iterator(true)?;

    // Even in a YES block, docIdRunEnd returns doc+1 because ordinal sets
    // may have gaps that the block-level check cannot detect.
    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&1, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(64)?;
    assert_equals(&65, &iter.doc_id_run_end()?);

    // MAYBE block: same behavior as forOrdinalRange
    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    assert_equals(&513, &iter.doc_id_run_end()?);

    // YES block in second repetition (dense up to DENSE_END=1088): also doc+1
    approx(&mut iter).advance(1024)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&1025, &iter.doc_id_run_end()?);

    // YES_IF_PRESENT block: also doc+1
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    assert_equals(&1089, &iter.doc_id_run_end()?);
    Ok(())
}

// ==========================================================================
// SortedSetDocValues (multi-valued ordinals with a set)
// ==========================================================================

/// Two sorted ordinals per doc. In the mixed region:
///
/// * Case 0: [9, 21] — both outside set → no match
/// * Case 1: [10, 21] — first ordinal 10 is in set → match
/// * Case 2: [9, 15] — second ordinal 15 is in gap → no match (unlike
///   forOrdinalRange!)
fn ordinal_pair(doc: i32) -> [i64; 2] {
    let d = doc % 1024;
    if d < 128 {
        [QUERY_MIN, QUERY_MAX]
    } else if d < 256 {
        [QUERY_MAX + 1, QUERY_MAX + 1]
    } else if d < 512 {
        [QUERY_MIN - 1, QUERY_MIN - 1]
    } else {
        match (d / 2) % 3 {
            0 => [QUERY_MIN - 1, QUERY_MAX + 1],
            1 => [QUERY_MIN, QUERY_MAX + 1],
            2 => [QUERY_MIN - 1, GAP_ORD],
            _ => {
                randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
            }
        }
    }
}

fn multi_ord_in_set(doc: i32) -> bool {
    let ords = ordinal_pair(doc);
    for ord in ords {
        if ord_in_set(ord) {
            return true;
        }
    }
    false
}

fn expected_multi_ord_matches() -> Vec<i32> {
    let mut matches = Vec::new();
    for doc in 0..2048 {
        if doc_has_value(doc) && multi_ord_in_set(doc) {
            matches.push(doc);
        }
    }
    matches
}

/// The anonymous `SortedSetDocValues` of `sortedSetDocValues`.
struct FakeSortedSetDocValues {
    doc: Arc<AtomicI32>,
    ord_idx: usize,
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
        let ords = ordinal_pair(self.doc_id());
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
        Ok(QUERY_MAX + 10)
    }
}

/// `sortedSetDocValues()`, with the handle on its current doc.
fn sorted_set_doc_values_shared() -> (Box<dyn SortedSetDocValues>, Arc<AtomicI32>) {
    let doc = Arc::new(AtomicI32::new(-1));
    (
        Box::new(FakeSortedSetDocValues {
            doc: Arc::clone(&doc),
            ord_idx: 0,
        }),
        doc,
    )
}

fn sorted_set_doc_values() -> Box<dyn SortedSetDocValues> {
    sorted_set_doc_values_shared().0
}

fn create_sorted_set_doc_values_iterator(multi_level: bool) -> Result<DocValuesRangeIterator> {
    let values = sorted_set_doc_values();
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, multi_level);
    DocValuesRangeIterator::for_sorted_set_ordinal_set(
        values,
        Some(skipper),
        &mut fake_terms_enum(MATCHING_ORDS),
    )
}

// --- SortedSetDocValues: Correctness ---

fn test_sorted_set_doc_values_correct_results_single_level() -> Result<()> {
    assert_equals(
        &expected_multi_ord_matches(),
        &collect_matches(&mut create_sorted_set_doc_values_iterator(false)?)?,
    );
    Ok(())
}

fn test_sorted_set_doc_values_correct_results_multiple_levels() -> Result<()> {
    assert_equals(
        &expected_multi_ord_matches(),
        &collect_matches(&mut create_sorted_set_doc_values_iterator(true)?)?,
    );
    Ok(())
}

// --- SortedSetDocValues: Gap ordinal behavior ---

fn test_sorted_set_gap_ordinal_rejected() -> Result<()> {
    // Doc 514: case 2 → ords [9, 15]. Ordinal 15 is in [10,20] but NOT in set.
    // With forOrdinalRange this would match; with forOrdinalSet it must not.
    let mut iter = create_sorted_set_doc_values_iterator(true)?;
    approx(&mut iter).advance(514)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

fn test_sorted_set_match_on_first_ordinal_in_set() -> Result<()> {
    // Doc 512: case 1 → ords [10, 21]. Ordinal 10 is in set → match.
    let mut iter = create_sorted_set_doc_values_iterator(true)?;
    approx(&mut iter).advance(512)?;
    Assert::assert_true(iter.matches()?);
    Ok(())
}

fn test_sorted_set_no_match_when_ordinals_bracket_range() -> Result<()> {
    // Doc 516: case 0 → ords [9, 21]. Neither ordinal is in the set.
    let mut iter = create_sorted_set_doc_values_iterator(true)?;
    approx(&mut iter).advance(516)?;
    Assert::assert_false(iter.matches()?);
    Ok(())
}

// --- SortedSetDocValues: YES blocks still check predicate ---

fn test_sorted_set_yes_block_checks_predicate() -> Result<()> {
    let (values, values_doc) = sorted_set_doc_values_shared();
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_sorted_set_ordinal_set(
        values,
        Some(skipper),
        &mut fake_terms_enum(MATCHING_ORDS),
    )?;

    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    // Ords [10, 20]: 10 is in set → match, but only because predicate was checked
    Assert::assert_true(iter.matches()?);
    // doc values iterator was advanced (predicate was checked)
    assert_equals(&0, &values_doc.load(Ordering::Relaxed));
    Ok(())
}

// --- SortedSetDocValues: Approximation skipping ---

fn test_sorted_set_approximation_skips_above_range() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true)?;
    assert_equals(&512, &approx(&mut iter).advance(128)?);
    Ok(())
}

fn test_sorted_set_approximation_skips_below_range() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true)?;
    assert_equals(&512, &approx(&mut iter).advance(300)?);
    Ok(())
}

fn test_sorted_set_approximation_skips_sparse_non_matching() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true)?;
    assert_equals(&1536, &approx(&mut iter).advance(1200)?);
    Ok(())
}

fn test_sorted_set_approximation_exhausted() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true)?;
    assert_equals(&NO_MORE_DOCS, &approx(&mut iter).advance(2048)?);
    Ok(())
}

// --- SortedSetDocValues: docIdRunEnd ---

fn test_sorted_set_doc_id_run_end_always_doc_plus_one() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true)?;

    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&1, &iter.doc_id_run_end()?);

    approx(&mut iter).advance(512)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    assert_equals(&513, &iter.doc_id_run_end()?);

    // YES block in second repetition (dense up to DENSE_END=1088)
    approx(&mut iter).advance(1024)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    assert_equals(&1025, &iter.doc_id_run_end()?);

    // YES_IF_PRESENT block
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    assert_equals(&1089, &iter.doc_id_run_end()?);
    Ok(())
}

// --- SortedSetDocValues: contiguous ordinal set is converted to a range ---

/// Multi-valued counterpart of
/// `testSortedDocValuesContiguousOrdinalSetUsesRangeFastPath`. When the
/// TermsEnum yields a contiguous run of ordinals, `forOrdinalSet` must
/// behave like `forOrdinalRange`: YES blocks short-circuit, `docIDRunEnd`
/// extends across the block, and the previously-gap ordinal is accepted.
fn test_sorted_set_contiguous_ordinal_set_uses_range_fast_path() -> Result<()> {
    let contiguous_ords: &[i64] = &[10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];

    let (values, values_doc) = sorted_set_doc_values_shared();
    let skipper = doc_values_skipper(QUERY_MIN, QUERY_MAX, true);
    let mut iter = DocValuesRangeIterator::for_sorted_set_ordinal_set(
        values,
        Some(skipper),
        &mut fake_terms_enum(contiguous_ords),
    )?;

    // YES block: predicate must not be consulted, so the doc values iterator is not advanced.
    approx(&mut iter).advance(0)?;
    assert_equals(&Match::Yes, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);
    Assert::assert_true_msg(
        "predicate should not have advanced doc values in a YES block when the ordinal set is a contiguous range",
        values_doc.load(Ordering::Relaxed) < approx(&mut iter).doc_id(),
    );

    // docIDRunEnd should extend across the YES run rather than collapsing to doc + 1.
    assert_equals(&128, &iter.doc_id_run_end()?);

    // The gap-ordinal that a non-contiguous set would reject (doc 514 → ords [9, 15]) must now
    // match, confirming the predicate has been widened to the [min, max] range check.
    approx(&mut iter).advance(514)?;
    assert_equals(&Match::Maybe, &approx(&mut iter).get_match());
    Assert::assert_true(iter.matches()?);

    // End-to-end correctness: the contiguous-set iterator must produce the exact same docs as
    // a forOrdinalRange iterator over [10, 20].
    let range_matches =
        collect_matches(&mut DocValuesRangeIterator::for_sorted_set_ordinal_range(
            sorted_set_doc_values(),
            Some(doc_values_skipper(QUERY_MIN, QUERY_MAX, true)),
            QUERY_MIN,
            QUERY_MAX,
        ))?;
    let set_matches = collect_matches(&mut DocValuesRangeIterator::for_sorted_set_ordinal_set(
        sorted_set_doc_values(),
        Some(doc_values_skipper(QUERY_MIN, QUERY_MAX, true)),
        &mut fake_terms_enum(contiguous_ords),
    )?)?;
    assert_equals(&range_matches, &set_matches);
    Ok(())
}

// --- SortedSetDocValues: YES_IF_PRESENT ---

fn test_sorted_set_yes_if_present() -> Result<()> {
    let mut iter = create_sorted_set_doc_values_iterator(true)?;

    // Sparse region: even doc has value, odd doc does not
    approx(&mut iter).advance(1088)?;
    assert_equals(&Match::YesIfPresent, &approx(&mut iter).get_match());
    // Ords [10, 20], 10 in set → match
    Assert::assert_true(iter.matches()?);

    approx(&mut iter).advance(1089)?;
    // Odd doc has no value → no match
    Assert::assert_false(iter.matches()?);
    Ok(())
}
