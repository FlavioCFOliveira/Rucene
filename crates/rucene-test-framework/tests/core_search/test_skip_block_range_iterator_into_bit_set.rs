//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSkipBlockRangeIteratorIntoBitSet.java`.
//!
//! Tests the `intoBitSet()` bulk path of `DocValuesRangeIterator` over single-valued
//! numeric doc values, including YES, YES_IF_PRESENT, and MAYBE block states.
//!
//! `TestSkipBlockRangeIteratorIntoBitSet extends BaseDocValuesSkipperTests`,
//! whose helpers are `base_doc_values_skipper_tests`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene104::codec::Lucene104Codec;
use rucene::document::{Document, NumericDocValuesField, SortedNumericDocValuesField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::constant_score_scorer_supplier::{
    ConstantScoreScorerSupplier, SingleIteratorSupplier,
};
use rucene::search::dense_conjunction_bulk_scorer::WINDOW_SIZE;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::two_phase_iterator::ScorerIterator;
use rucene::search::{
    BooleanQueryBuilder, DocIdSetIterator, DocValuesRangeIterator, Occur, Query, ScoreMode,
    TwoPhaseIterator, TwoPhaseIteratorAsDocIdSetIterator, NO_MORE_DOCS,
};
use rucene::store::Directory;
use rucene::util::bit_set::BitSet;
use rucene::util::FixedBitSet;
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::base_doc_values_skipper_tests::{doc_values, doc_values_skipper};
use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, assert_equals_msg, random};

// Use enough docs to span at least 4 skip blocks (default skip block size = 4096 docs).
const DOC_COUNT: i32 = 4096 * 4;

/// The instance of `TestSkipBlockRangeIteratorIntoBitSet`.
#[derive(Default)]
pub struct IntoBitSet {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
    values: Vec<i64>,
}

impl IntoBitSet {
    fn reader(&self) -> &Arc<dyn DirectoryReader> {
        self.reader.as_ref().expect("setUp ran")
    }

    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }
}

/// `TestSkipBlockRangeIteratorIntoBitSet`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<IntoBitSet>(
        "org.apache.lucene.search.TestSkipBlockRangeIteratorIntoBitSet",
        vec![
            set_up(into_bit_set_set_up),
            tear_down(into_bit_set_tear_down),
            ftest(
                "testTwoPhaseIteratorIsWired",
                test_two_phase_iterator_is_wired,
            ),
            ftest(
                "testMultiValuedTwoPhaseIteratorIsWired",
                test_multi_valued_two_phase_iterator_is_wired,
            ),
            ftest(
                "testSingleFieldRangeCorrectness",
                test_single_field_range_correctness,
            ),
            ftest(
                "testMultiFieldRangeCorrectness",
                test_multi_field_range_correctness,
            ),
            ftest("testResultsAreRepeatable", test_results_are_repeatable),
            ftest(
                "testSingleFieldWithRestrictiveSecondClause",
                test_single_field_with_restrictive_second_clause,
            ),
            ftest(
                "testIntoBitSetAdvancesWhenUpToIsBlockBoundary",
                test_into_bit_set_advances_when_up_to_is_block_boundary,
            ),
            ftest(
                "testIntoBitSetMatchesLinearScan",
                test_into_bit_set_matches_linear_scan,
            ),
            ftest(
                "testIntoBitSetMatchesLinearScanHuge",
                test_into_bit_set_matches_linear_scan_huge,
            )
            .annotate(Nightly),
            ftest(
                "testIntoBitSetAllMatchRange",
                test_into_bit_set_all_match_range,
            ),
            ftest(
                "testIntoBitSetAllMatchRangeHuge",
                test_into_bit_set_all_match_range_huge,
            )
            .annotate(Nightly),
            ftest(
                "testIntoBitSetNoMatchRange",
                test_into_bit_set_no_match_range,
            ),
            ftest(
                "testIntoBitSetNoMatchRangeHuge",
                test_into_bit_set_no_match_range_huge,
            )
            .annotate(Nightly),
            ftest("testIntoBitSetSparseField", test_into_bit_set_sparse_field),
            ftest(
                "testIntoBitSetSparseFieldHuge",
                test_into_bit_set_sparse_field_huge,
            )
            .annotate(Nightly),
            ftest("testIterationCorrectness", test_iteration_correctness),
            ftest(
                "testIterationCorrectnessHuge",
                test_iteration_correctness_huge,
            )
            .annotate(Nightly),
            ftest(
                "testAllBlockTypesWithFakeSkipper",
                test_all_block_types_with_fake_skipper,
            ),
            ftest(
                "testRangeIntoBitSetMatchesPerDocEvaluation",
                test_range_into_bit_set_matches_per_doc_evaluation,
            ),
            ftest(
                "testSortedNumericRangeIntoBitSetDenseFixedCardinality",
                test_sorted_numeric_range_into_bit_set_dense_fixed_cardinality,
            ),
            ftest(
                "testSortedNumericRangeIntoBitSetDenseVariableCardinality",
                test_sorted_numeric_range_into_bit_set_dense_variable_cardinality,
            ),
            ftest(
                "testSortedNumericRangeIntoBitSetSparseFixedCardinality",
                test_sorted_numeric_range_into_bit_set_sparse_fixed_cardinality,
            ),
            ftest(
                "testSortedNumericRangeIntoBitSetSparseVariableCardinality",
                test_sorted_numeric_range_into_bit_set_sparse_variable_cardinality,
            ),
        ],
    )
}

/// `new IndexWriterConfig().setCodec(new Lucene104Codec())`.
fn lucene104_config() -> Result<IndexWriterConfig> {
    let mut iwc = IndexWriterConfig::new();
    iwc.set_codec(Arc::new(Lucene104Codec::new()))?;
    Ok(iwc)
}

fn into_bit_set_set_up(f: &mut IntoBitSet) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = lucene104_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    f.values = vec![0; DOC_COUNT as usize];
    for i in 0..DOC_COUNT as usize {
        f.values[i] = (i % 100) as i64; // deterministic: 0..99 repeating
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::indexed_field(
            "age",
            f.values[i],
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "multi_age",
            f.values[i] - 1000,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "multi_age",
            f.values[i],
        )));
        doc.add(Box::new(NumericDocValuesField::indexed_field(
            "score",
            (i as i64 * 7) % 1000,
        )));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    f.dir = Some(dir);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    Ok(())
}

fn into_bit_set_tear_down(f: &mut IntoBitSet) -> Result<()> {
    f.searcher = None;
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// `r.leaves()`.
fn leaves_of(r: &Arc<dyn DirectoryReader>) -> Result<Vec<Arc<LeafReaderContext>>> {
    (Arc::clone(r) as Arc<dyn IndexReader>).leaves()
}

/// `NullPointerException` for a `null` the test dereferences.
fn npe<T>() -> T {
    randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
}

/// The shared body of `testTwoPhaseIteratorIsWired` and
/// `testMultiValuedTwoPhaseIteratorIsWired`.
fn assert_two_phase_is_wired(f: &IntoBitSet, q: &Arc<dyn Query>, message: &str) -> Result<()> {
    let leaves = leaves_of(f.reader())?;
    let ctx = &leaves[0];
    let weight = Arc::clone(q).create_weight(f.searcher(), ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
    let ss = weight.scorer_supplier(ctx)?;
    Assert::assert_true_msg("ScorerSupplier must not be null", ss.is_some());
    let mut ss = ss.unwrap_or_else(npe);

    let css = ss
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<ConstantScoreScorerSupplier<SingleIteratorSupplier>>());
    Assert::assert_true_msg(
        "ScorerSupplier must be a ConstantScoreScorerSupplier",
        css.is_some(),
    );
    let css = css.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.ClassCastException")
    });
    let iter = css.iterator(i64::MAX)?;
    // `TwoPhaseIterator.unwrap(iter)`.
    let two_phase = match &iter {
        ScorerIterator::TwoPhase(two_phase) => Some(two_phase),
        ScorerIterator::Simple(_) => None,
    };
    Assert::assert_true_msg(
        &format!(
            "{message}, but got: {}",
            if two_phase.is_none() {
                "DocIdSetIterator"
            } else {
                "TwoPhaseIterator"
            }
        ),
        two_phase
            .and_then(|t| t.two_phase_as_any())
            .is_some_and(|a| a.is::<DocValuesRangeIterator>()),
    );
    Ok(())
}

/// Verifies that SortedNumericDocValuesRangeQuery wires a DocValuesRangeIterator two-phase when
/// the field has a skip index and is single-valued.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testTwoPhaseIteratorIsWired`.
fn test_two_phase_iterator_is_wired(f: &mut IntoBitSet) -> Result<()> {
    // Use a range that won't be rewritten to MatchAll or MatchNone
    let q = SortedNumericDocValuesField::new_slow_range_query("age", 20, 40);
    assert_two_phase_is_wired(
        f,
        &q,
        "Range query on single-valued field with skip index must use a DocValuesRangeIterator",
    )
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testMultiValuedTwoPhaseIteratorIsWired`.
fn test_multi_valued_two_phase_iterator_is_wired(f: &mut IntoBitSet) -> Result<()> {
    let q = SortedNumericDocValuesField::new_slow_range_query("multi_age", 20, 40);
    assert_two_phase_is_wired(
        f,
        &q,
        "Range query on multi-valued field with skip index must use a DocValuesRangeIterator",
    )?;
    assert_equals(
        &f.searcher()
            .count(SortedNumericDocValuesField::new_slow_range_query(
                "age", 20, 40,
            ))?,
        &f.searcher().count(q)?,
    );
    Ok(())
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testSingleFieldRangeCorrectness`.
fn test_single_field_range_correctness(f: &mut IntoBitSet) -> Result<()> {
    // Count expected matches by scanning values[] directly
    let mut expected = 0;
    for &v in &f.values {
        if (20..=40).contains(&v) {
            expected += 1;
        }
    }
    Assert::assert_true_msg(
        "Should have some matches with controlled data",
        expected > 0,
    );

    let q = SortedNumericDocValuesField::new_slow_range_query("age", 20, 40);
    assert_equals_msg(
        Some("Single-field count must match linear scan"),
        &expected,
        &f.searcher().count(q)?,
    );
    Ok(())
}

/// `new BooleanQuery.Builder().add(a, FILTER).add(b, FILTER).build()`.
fn filters(a: Arc<dyn Query>, b: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    let mut builder = BooleanQueryBuilder::new();
    builder.add(a, Occur::FILTER)?;
    builder.add(b, Occur::FILTER)?;
    Ok(Arc::new(builder.build()))
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testMultiFieldRangeCorrectness`.
fn test_multi_field_range_correctness(f: &mut IntoBitSet) -> Result<()> {
    let mut expected_both = 0;
    for i in 0..DOC_COUNT as usize {
        let age = f.values[i];
        let score = (i as i64 * 7) % 1000;
        if (20..=40).contains(&age) && (100..=500).contains(&score) {
            expected_both += 1;
        }
    }

    let age_only = SortedNumericDocValuesField::new_slow_range_query("age", 20, 40);
    let score_only = SortedNumericDocValuesField::new_slow_range_query("score", 100, 500);
    let both = filters(Arc::clone(&age_only), Arc::clone(&score_only))?;

    let s = f.searcher();
    assert_equals_msg(
        Some("Multi-field count must match linear scan"),
        &expected_both,
        &s.count(Arc::clone(&both))?,
    );
    Assert::assert_true_msg(
        "Conjunction must be <= age-only count",
        s.count(Arc::clone(&both))? <= s.count(age_only)?,
    );
    Assert::assert_true_msg(
        "Conjunction must be <= score-only count",
        s.count(both)? <= s.count(score_only)?,
    );
    Ok(())
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testResultsAreRepeatable`.
fn test_results_are_repeatable(f: &mut IntoBitSet) -> Result<()> {
    let q = filters(
        SortedNumericDocValuesField::new_slow_range_query("age", 20, 40),
        SortedNumericDocValuesField::new_slow_range_query("score", 100, 500),
    )?;

    let count1 = f.searcher().count(Arc::clone(&q))?;
    let count2 = f.searcher().count(q)?;
    assert_equals_msg(Some("Results must be deterministic"), &count1, &count2);
    Ok(())
}

/// Single-field range with a restrictive second clause exercises the single-field bitset path via
/// DenseConjunctionBulkScorer. Uses a second range query on a different field as the restrictive
/// clause — MatchAllDocsQuery is not used because BooleanQuery rewrites it away.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testSingleFieldWithRestrictiveSecondClause`.
fn test_single_field_with_restrictive_second_clause(f: &mut IntoBitSet) -> Result<()> {
    // age in [20,40] AND score in [0,0]..score=0 only for docs where (i*7)%1000==0, i.e., i=0
    let age_q = SortedNumericDocValuesField::new_slow_range_query("age", 0, 99); // matches all
    let score_q = SortedNumericDocValuesField::new_slow_range_query("score", 0, 0); // very restrictive

    let mut expected_score0 = 0;
    for i in 0..i64::from(DOC_COUNT) {
        if (i * 7) % 1000 == 0 {
            expected_score0 += 1;
        }
    }

    let combined = filters(age_q, score_q)?;

    assert_equals_msg(
        Some("Should match only docs where score=0"),
        &expected_score0,
        &f.searcher().count(combined)?,
    );
    Ok(())
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetAdvancesWhenUpToIsBlockBoundary`.
fn test_into_bit_set_advances_when_up_to_is_block_boundary(_f: &mut IntoBitSet) -> Result<()> {
    let query_min = 10;
    let query_max = 20;

    let values = doc_values(query_min, query_max)?;
    let skipper = doc_values_skipper(query_min, query_max, true);
    let mut iter = DocValuesRangeIterator::for_range(values, Some(skipper), query_min, query_max);
    assert_equals(&0, &iter.approximation().next_doc()?);

    let mut bit_set = FixedBitSet::new(2048);
    iter.into_bit_set(128, &mut bit_set, 0)?;

    assert_equals_msg(
        Some("All docs in the YES block must match"),
        &128,
        &bit_set.cardinality(),
    );
    let doc = iter.approximation().doc_id();
    Assert::assert_true_msg(
        &format!(
            "approximation must skip the NO block that starts at the block boundary, but was {doc}"
        ),
        doc >= 512,
    );

    // Continue filling the bitset in a second window, as DenseConjunctionBulkScorer does.
    iter.into_bit_set(2048, &mut bit_set, 0)?;
    let mut expected = FixedBitSet::new(2048);
    let mut values = doc_values(query_min, query_max)?;
    let mut d = values.next_doc()?;
    while d != NO_MORE_DOCS && d < 2048 {
        if values.long_value()? >= query_min && values.long_value()? <= query_max {
            expected.set(d as usize);
        }
        d = values.next_doc()?;
    }
    let mut diff = expected.clone();
    diff.and_not(&bit_set);
    assert_equals_msg(
        Some("Second intoBitSet window must not miss matching docs"),
        &0,
        &diff.cardinality(),
    );
    Ok(())
}

/// Directly tests intoBitSet() against a linear scan reference.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetMatchesLinearScan`.
fn test_into_bit_set_matches_linear_scan(f: &mut IntoBitSet) -> Result<()> {
    do_test_into_bit_set_matches_linear_scan(f.reader())
}

/// `try (Directory hugeDir = newDirectory()) { buildIndex(hugeDir, 4096 * 100); try
/// (DirectoryReader r = DirectoryReader.open(hugeDir)) { body(r); } }`.
fn with_huge_index(body: fn(&Arc<dyn DirectoryReader>) -> Result<()>) -> Result<()> {
    let mut huge_dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        build_index(&huge_dir.as_directory(), 4096 * 100)?;
        let r = directory_reader::open(huge_dir.as_directory())?;
        let outcome = body(&r);
        let closed = r.close();
        outcome.and(closed)
    })();
    let closed = huge_dir.close();
    result.and(closed)
}

/// Large-scale version — 100 skip blocks
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetMatchesLinearScanHuge`.
fn test_into_bit_set_matches_linear_scan_huge(_f: &mut IntoBitSet) -> Result<()> {
    with_huge_index(do_test_into_bit_set_matches_linear_scan)
}

/// `private void doTestIntoBitSetMatchesLinearScan(DirectoryReader r)`.
fn do_test_into_bit_set_matches_linear_scan(r: &Arc<dyn DirectoryReader>) -> Result<()> {
    let leaves = leaves_of(r)?;
    let ctx = &leaves[0];
    let max_doc = ctx.leaf_reader().max_doc();
    let window_size = WINDOW_SIZE; // 4096

    let mut expected = FixedBitSet::new(window_size as usize);
    let mut ref_values = ctx
        .leaf_reader()
        .get_numeric_doc_values("age")?
        .unwrap_or_else(npe);
    for d in 0..max_doc.min(window_size) {
        if ref_values.advance_exact(d)?
            && ref_values.long_value()? >= 20
            && ref_values.long_value()? <= 40
        {
            expected.set(d as usize);
        }
    }

    let skipper = ctx.leaf_reader().get_doc_values_skipper("age")?;
    let dv = ctx
        .leaf_reader()
        .get_numeric_doc_values("age")?
        .unwrap_or_else(npe);
    Assert::assert_true_msg("Field must have a skip index", skipper.is_some());

    let mut iter = DocValuesRangeIterator::for_range(dv, skipper, 20, 40);
    iter.approximation().next_doc()?;

    let mut actual = FixedBitSet::new(window_size as usize);
    iter.into_bit_set(max_doc.min(window_size), &mut actual, 0)?;

    assert_equals_msg(
        Some("intoBitSet must set exactly the same bits as linear scan"),
        &expected.cardinality(),
        &actual.cardinality(),
    );
    let mut diff = expected.clone();
    diff.xor(&actual);
    assert_equals_msg(Some("No bits should differ"), &0, &diff.cardinality());
    Ok(())
}

/// Tests YES block path — range covers all values, all docs match.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetAllMatchRange`.
fn test_into_bit_set_all_match_range(f: &mut IntoBitSet) -> Result<()> {
    do_test_into_bit_set_all_match_range(f.reader())
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetAllMatchRangeHuge`.
fn test_into_bit_set_all_match_range_huge(_f: &mut IntoBitSet) -> Result<()> {
    with_huge_index(do_test_into_bit_set_all_match_range)
}

/// `private void doTestIntoBitSetAllMatchRange(DirectoryReader r)`.
fn do_test_into_bit_set_all_match_range(r: &Arc<dyn DirectoryReader>) -> Result<()> {
    let leaves = leaves_of(r)?;
    let ctx = &leaves[0];
    let max_doc = ctx.leaf_reader().max_doc();

    let skipper = ctx.leaf_reader().get_doc_values_skipper("age")?;
    let dv = ctx
        .leaf_reader()
        .get_numeric_doc_values("age")?
        .unwrap_or_else(npe);
    Assert::assert_true(skipper.is_some());

    let mut iter = DocValuesRangeIterator::for_range(dv, skipper, 0, 99);
    iter.approximation().next_doc()?;

    let mut bit_set = FixedBitSet::new(max_doc as usize);
    iter.into_bit_set(max_doc, &mut bit_set, 0)?;

    assert_equals_msg(
        Some("All docs should match range [0,99]"),
        &(max_doc as usize),
        &bit_set.cardinality(),
    );
    Ok(())
}

/// Tests NO block path — range matches nothing.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetNoMatchRange`.
fn test_into_bit_set_no_match_range(f: &mut IntoBitSet) -> Result<()> {
    do_test_into_bit_set_no_match_range(f.reader())
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetNoMatchRangeHuge`.
fn test_into_bit_set_no_match_range_huge(_f: &mut IntoBitSet) -> Result<()> {
    with_huge_index(do_test_into_bit_set_no_match_range)
}

/// `private void doTestIntoBitSetNoMatchRange(DirectoryReader r)`.
fn do_test_into_bit_set_no_match_range(r: &Arc<dyn DirectoryReader>) -> Result<()> {
    let leaves = leaves_of(r)?;
    let ctx = &leaves[0];
    let max_doc = ctx.leaf_reader().max_doc();

    let skipper = ctx.leaf_reader().get_doc_values_skipper("age")?;
    let dv = ctx
        .leaf_reader()
        .get_numeric_doc_values("age")?
        .unwrap_or_else(npe);
    Assert::assert_true(skipper.is_some());

    let mut iter = DocValuesRangeIterator::for_range(dv, skipper, 200, 300);
    iter.approximation().next_doc()?;

    let mut bit_set = FixedBitSet::new(max_doc as usize);
    iter.into_bit_set(max_doc, &mut bit_set, 0)?;

    assert_equals_msg(
        Some("No docs should match out-of-range query"),
        &0,
        &bit_set.cardinality(),
    );
    Ok(())
}

/// Tests YES_IF_PRESENT block path — sparse field, only even docs have a value.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetSparseField`.
fn test_into_bit_set_sparse_field(_f: &mut IntoBitSet) -> Result<()> {
    do_test_into_bit_set_sparse_field(1000)
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIntoBitSetSparseFieldHuge`.
fn test_into_bit_set_sparse_field_huge(_f: &mut IntoBitSet) -> Result<()> {
    do_test_into_bit_set_sparse_field(4096 * 20)
}

/// `private void doTestIntoBitSetSparseField(int numDocs)`.
fn do_test_into_bit_set_sparse_field(num_docs: i32) -> Result<()> {
    let mut sparse_dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let iwc = lucene104_config()?;
        let w = IndexWriter::new(sparse_dir.as_directory(), iwc)?;
        let mut expected_docs: Vec<i32> = Vec::new();
        for i in 0..num_docs {
            let mut doc = Document::new();
            if i % 2 == 0 {
                // All values within [20, 40] so blocks are YES_IF_PRESENT (not MAYBE)
                let val = 20 + i64::from(i % 21);
                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "sparse", val,
                )));
                expected_docs.push(i);
            }
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;
        let sparse_reader = directory_reader::open_with_writer(&*w)?;
        let r = (|| -> Result<()> {
            w.close()?;
            let leaves = leaves_of(&sparse_reader)?;
            let ctx = &leaves[0];

            let skipper = ctx.leaf_reader().get_doc_values_skipper("sparse")?;
            let dv = ctx
                .leaf_reader()
                .get_numeric_doc_values("sparse")?
                .unwrap_or_else(npe);
            if skipper.is_none() {
                return Ok(());
            }

            let mut iter = DocValuesRangeIterator::for_range(dv, skipper, 20, 40);
            iter.approximation().next_doc()?;

            let mut bit_set = FixedBitSet::new(num_docs as usize);
            iter.into_bit_set(num_docs, &mut bit_set, 0)?;

            assert_equals_msg(
                Some("Sparse field intoBitSet must match expected count"),
                &expected_docs.len(),
                &bit_set.cardinality(),
            );
            let mut i = 1;
            while i < num_docs {
                Assert::assert_false_msg(
                    &format!("Odd doc {i} has no value and must not be set"),
                    bit_set.get(i as usize),
                );
                i += 2;
            }
            Ok(())
        })();
        let closed = sparse_reader.close();
        r.and(closed)
    })();
    let closed = sparse_dir.close();
    result.and(closed)
}

/// Tests nextDoc() and advance() return the same docs as a linear scan.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIterationCorrectness`.
fn test_iteration_correctness(f: &mut IntoBitSet) -> Result<()> {
    do_test_iteration_correctness(f.reader())
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testIterationCorrectnessHuge`.
fn test_iteration_correctness_huge(_f: &mut IntoBitSet) -> Result<()> {
    with_huge_index(do_test_iteration_correctness)
}

/// `TwoPhaseIterator.asDocIdSetIterator(DocValuesRangeIterator.forRange(dv, skipper, 20, 40))`
/// over a fresh skipper and doc values of `age`.
fn age_iterator(
    ctx: &LeafReaderContext,
) -> Result<TwoPhaseIteratorAsDocIdSetIterator<DocValuesRangeIterator>> {
    let skipper = ctx.leaf_reader().get_doc_values_skipper("age")?;
    let dv = ctx
        .leaf_reader()
        .get_numeric_doc_values("age")?
        .unwrap_or_else(npe);
    Ok(TwoPhaseIteratorAsDocIdSetIterator::new(Box::new(
        DocValuesRangeIterator::for_range(dv, skipper, 20, 40),
    )))
}

/// `private void doTestIterationCorrectness(DirectoryReader r)`.
fn do_test_iteration_correctness(r: &Arc<dyn DirectoryReader>) -> Result<()> {
    let leaves = leaves_of(r)?;
    let ctx = &leaves[0];

    let mut expected: Vec<i32> = Vec::new();
    let mut ref_values = ctx
        .leaf_reader()
        .get_numeric_doc_values("age")?
        .unwrap_or_else(npe);
    for d in 0..ctx.leaf_reader().max_doc() {
        if ref_values.advance_exact(d)?
            && ref_values.long_value()? >= 20
            && ref_values.long_value()? <= 40
        {
            expected.push(d);
        }
    }
    Assert::assert_true_msg("Test requires some matching docs", !expected.is_empty());

    // Test 1: nextDoc() returns all matching docs in order
    let skipper = ctx.leaf_reader().get_doc_values_skipper("age")?;
    Assert::assert_true(skipper.is_some());
    let dv = ctx
        .leaf_reader()
        .get_numeric_doc_values("age")?
        .unwrap_or_else(npe);

    let mut iter = TwoPhaseIteratorAsDocIdSetIterator::new(Box::new(
        DocValuesRangeIterator::for_range(dv, skipper, 20, 40),
    ));
    let mut actual: Vec<i32> = Vec::new();
    let mut d = iter.next_doc()?;
    while d != NO_MORE_DOCS {
        actual.push(d);
        d = iter.next_doc()?;
    }
    assert_equals_msg(
        Some("nextDoc() must return same docs as expected"),
        &expected,
        &actual,
    );

    // Test 2: advance() forward through the iterator with random skips.
    // Uses a single iterator instance and advances forward multiple times,
    // verifying each advance lands on the correct next matching doc.
    let mut iter = age_iterator(ctx)?;
    let rng = random();
    let mut idx = 0usize;
    while idx < expected.len() {
        let target = expected[idx];
        let doc = iter.advance(target)?;
        assert_equals_msg(
            Some(&format!("advance({target}) must return the target")),
            &target,
            &doc,
        );
        // Skip forward by a random amount (1-5 matching docs)
        idx += (rng.next_int_bound(5) + 1) as usize;
    }

    // Test 3: advance() to a non-matching doc lands on the next match.
    // Picks random targets between consecutive matches and verifies
    // advance returns the next matching doc after the target.
    let mut iter = age_iterator(ctx)?;
    let mut prev_doc = -1;
    let mut i = 0;
    while i < expected.len() && i < 20 {
        let match_doc = expected[i];
        if match_doc > prev_doc + 1 {
            // Target a gap between previous position and this match
            let target = prev_doc + 1 + rng.next_int_bound(match_doc - prev_doc - 1);
            let doc = iter.advance(target)?;
            assert_equals_msg(
                Some(&format!(
                    "advance({target}) must skip to next match at {match_doc}"
                )),
                &match_doc,
                &doc,
            );
            prev_doc = doc;
        } else {
            prev_doc = match_doc;
        }
        i += 1;
    }

    // Test 4: advance() past all docs returns NO_MORE_DOCS
    let mut iter = age_iterator(ctx)?;
    let doc = iter.advance(ctx.leaf_reader().max_doc())?;
    assert_equals(&NO_MORE_DOCS, &doc);
    Ok(())
}

/// Helper: builds an index with `numDocs` docs, values = i % 100.
///
/// `private void buildIndex(Directory dir, int numDocs)`.
fn build_index(dir: &Arc<dyn Directory>, num_docs: i32) -> Result<()> {
    let iwc = lucene104_config()?;
    let w = IndexWriter::new(Arc::clone(dir), iwc)?;
    let result = (|| -> Result<()> {
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::indexed_field(
                "age",
                i64::from(i % 100),
            )));
            w.add_document(&doc)?;
        }
        w.force_merge(1)
    })();
    let closed = w.close();
    result.and(closed)
}

/// Tests the DocValuesRangeIterator iteration and intoBitSet paths using the fake NumericDocValues
/// and DocValuesSkipper from BaseDocValuesSkipperTests, which exercises all block types (YES, NO,
/// MAYBE, YES_IF_PRESENT) across both dense and sparse regions.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testAllBlockTypesWithFakeSkipper`.
fn test_all_block_types_with_fake_skipper(_f: &mut IntoBitSet) -> Result<()> {
    let query_min = 10;
    let query_max = 20;

    // Collect expected matching docs by brute-force scan (up to 2048, the skipper's range)
    let mut expected: Vec<i32> = Vec::new();
    let mut ref_values = doc_values(query_min, query_max)?;
    let mut d = ref_values.next_doc()?;
    while d != NO_MORE_DOCS && d < 2048 {
        let v = ref_values.long_value()?;
        if v >= query_min && v <= query_max {
            expected.push(d);
        }
        d = ref_values.next_doc()?;
    }
    Assert::assert_true_msg("Should have matching docs", !expected.is_empty());

    // Test nextDoc() through all block types
    let values = doc_values(query_min, query_max)?;
    let skipper = doc_values_skipper(query_min, query_max, true);
    let mut iter = TwoPhaseIteratorAsDocIdSetIterator::new(Box::new(
        DocValuesRangeIterator::for_range(values, Some(skipper), query_min, query_max),
    ));
    let mut actual: Vec<i32> = Vec::new();
    let mut d = iter.next_doc()?;
    while d != NO_MORE_DOCS {
        actual.push(d);
        d = iter.next_doc()?;
    }
    assert_equals_msg(
        Some("Must return same docs across all block types"),
        &expected,
        &actual,
    );

    // Test advance() across block types:
    // - advance into YES block (docs 0-127)
    // - advance into NO block, should skip to MAYBE (docs 512+)
    // - advance into MAYBE block
    // - advance into YES_IF_PRESENT block (docs 1024+, sparse)
    // - advance past end
    let values = doc_values(query_min, query_max)?;
    let skipper = doc_values_skipper(query_min, query_max, true);
    let mut iter = TwoPhaseIteratorAsDocIdSetIterator::new(Box::new(
        DocValuesRangeIterator::for_range(values, Some(skipper), query_min, query_max),
    ));

    // advance into YES block — should land exactly on target
    assert_equals(&50, &iter.advance(50)?);
    // advance within same YES block
    assert_equals(&100, &iter.advance(100)?);
    // advance into NO block (128-511) — should skip to first MAYBE match (doc 514)
    let doc = iter.advance(200)?;
    Assert::assert_true_msg(
        &format!("advance(200) should skip NO blocks, got {doc}"),
        doc >= 512,
    );
    Assert::assert_true_msg(
        "advance(200) result must be in expected",
        expected.contains(&doc),
    );
    // advance into sparse region (YES_IF_PRESENT, docs 1024+)
    let doc = iter.advance(1024)?;
    Assert::assert_true_msg(
        &format!("advance(1024) should find a doc >= 1024, got {doc}"),
        doc >= 1024,
    );
    Assert::assert_true_msg(
        "advance(1024) result must be in expected",
        expected.contains(&doc),
    );
    // advance past all docs
    assert_equals(&NO_MORE_DOCS, &iter.advance(2048)?);

    // Test intoBitSet() across all block types
    let values = doc_values(query_min, query_max)?;
    let skipper = doc_values_skipper(query_min, query_max, true);
    let mut range_iter =
        DocValuesRangeIterator::for_range(values, Some(skipper), query_min, query_max);
    let first_doc = range_iter.approximation().next_doc()?;

    let mut bit_set = FixedBitSet::new(2048);
    range_iter.into_bit_set(2048, &mut bit_set, 0)?;

    // All expected docs after firstDoc should be set
    for &expected_doc in &expected {
        if expected_doc > first_doc {
            Assert::assert_true_msg(
                &format!("Doc {expected_doc} should be set in bitset"),
                bit_set.get(expected_doc as usize),
            );
        }
    }
    // No unexpected docs should be set
    let mut d = bit_set.next_set_bit(0);
    while d != NO_MORE_DOCS {
        Assert::assert_true_msg(
            &format!("Doc {d} set in bitset but not in expected"),
            expected.contains(&d),
        );
        d = if ((d + 1) as usize) < bit_set.length() {
            bit_set.next_set_bit(d + 1)
        } else {
            NO_MORE_DOCS
        };
    }
    Ok(())
}

/// Tests that rangeIntoBitSet (SIMD/fast path) produces the exact same results as per-doc
/// evaluation (slow path) across random data with various densities and range selectivities.
///
/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testRangeIntoBitSetMatchesPerDocEvaluation`.
fn test_range_into_bit_set_matches_per_doc_evaluation(_f: &mut IntoBitSet) -> Result<()> {
    let rng = random();
    for _iter in 0..10 {
        let num_docs = rng.next_int_range(4096, 4096 * 5);
        let max_value = i64::from(rng.next_int_range(100, 10000));
        // Random range selectivity
        let range_min = rng.next_long_range(0, max_value / 2);
        let range_max = range_min + rng.next_long_range(1, max_value / 2);

        let mut dir = LuceneTestCase::new_directory();
        let result = (|| -> Result<()> {
            {
                let iwc = lucene104_config()?;
                let w = IndexWriter::new(dir.as_directory(), iwc)?;
                let written = (|| -> Result<()> {
                    for _i in 0..num_docs {
                        let mut doc = Document::new();
                        doc.add(Box::new(NumericDocValuesField::indexed_field(
                            "val",
                            rng.next_long_range(0, max_value),
                        )));
                        w.add_document(&doc)?;
                    }
                    w.force_merge(1)
                })();
                let closed = w.close();
                written.and(closed)?;
            }

            let reader = directory_reader::open(dir.as_directory())?;
            let r = (|| -> Result<()> {
                let leaves = leaves_of(&reader)?;
                let ctx = &leaves[0];

                // Slow path: per-doc evaluation
                let mut expected = FixedBitSet::new(num_docs as usize);
                let mut slow_dv = ctx
                    .leaf_reader()
                    .get_numeric_doc_values("val")?
                    .unwrap_or_else(npe);
                for d in 0..num_docs {
                    if slow_dv.advance_exact(d)? {
                        let v = slow_dv.long_value()?;
                        if v >= range_min && v <= range_max {
                            expected.set(d as usize);
                        }
                    }
                }

                // Fast path: rangeIntoBitSet
                let mut actual = FixedBitSet::new(num_docs as usize);
                let mut fast_dv = ctx
                    .leaf_reader()
                    .get_numeric_doc_values("val")?
                    .unwrap_or_else(npe);
                fast_dv.range_into_bit_set(0, num_docs, range_min, range_max, &mut actual, 0)?;

                assert_equals_msg(
                    Some(&format!(
                        "rangeIntoBitSet must match per-doc evaluation (numDocs={num_docs}, range=[{range_min},{range_max}])"
                    )),
                    &expected,
                    &actual,
                );
                Ok(())
            })();
            let closed = reader.close();
            r.and(closed)
        })();
        let closed = dir.close();
        result.and(closed)?;
    }
    Ok(())
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testSortedNumericRangeIntoBitSetDenseFixedCardinality`.
fn test_sorted_numeric_range_into_bit_set_dense_fixed_cardinality(
    _f: &mut IntoBitSet,
) -> Result<()> {
    do_test_sorted_numeric_range_into_bit_set(true, true)
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testSortedNumericRangeIntoBitSetDenseVariableCardinality`.
fn test_sorted_numeric_range_into_bit_set_dense_variable_cardinality(
    _f: &mut IntoBitSet,
) -> Result<()> {
    do_test_sorted_numeric_range_into_bit_set(true, false)
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testSortedNumericRangeIntoBitSetSparseFixedCardinality`.
fn test_sorted_numeric_range_into_bit_set_sparse_fixed_cardinality(
    _f: &mut IntoBitSet,
) -> Result<()> {
    do_test_sorted_numeric_range_into_bit_set(false, true)
}

/// Port of `TestSkipBlockRangeIteratorIntoBitSet.testSortedNumericRangeIntoBitSetSparseVariableCardinality`.
fn test_sorted_numeric_range_into_bit_set_sparse_variable_cardinality(
    _f: &mut IntoBitSet,
) -> Result<()> {
    do_test_sorted_numeric_range_into_bit_set(false, false)
}

/// `private void doTestSortedNumericRangeIntoBitSet(boolean dense, boolean fixedCardinality)`.
fn do_test_sorted_numeric_range_into_bit_set(dense: bool, fixed_cardinality: bool) -> Result<()> {
    let num_docs = 4096 * 2;
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        {
            let iwc = lucene104_config()?;
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            let written = (|| -> Result<()> {
                for doc_id in 0..num_docs {
                    let mut doc = Document::new();
                    if dense || doc_id % 3 != 0 {
                        let value_count = if fixed_cardinality {
                            4
                        } else {
                            1 + (doc_id & 3)
                        };
                        let first_value = (i64::from(doc_id) * 13) % 100;
                        for i in 0..value_count {
                            doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                                "sn",
                                first_value + i64::from(i) * 3,
                            )));
                        }
                    }
                    w.add_document(&doc)?;
                }
                w.force_merge(1)
            })();
            let closed = w.close();
            written.and(closed)?;
        }

        let reader = directory_reader::open(dir.as_directory())?;
        let r = (|| -> Result<()> {
            let leaves = leaves_of(&reader)?;
            let ctx = &leaves[0];
            let mut expected = FixedBitSet::new(num_docs as usize);
            let mut expected_values = ctx
                .leaf_reader()
                .get_sorted_numeric_doc_values("sn")?
                .unwrap_or_else(npe);
            for doc_id in 0..num_docs {
                if expected_values.advance_exact(doc_id)? {
                    let count = expected_values.doc_value_count()?;
                    for _i in 0..count {
                        let value = expected_values.next_value()?;
                        if value >= 20 {
                            if value <= 40 {
                                expected.set(doc_id as usize);
                            }
                            break;
                        }
                    }
                }
            }

            let mut actual = FixedBitSet::new(num_docs as usize);
            ctx.leaf_reader()
                .get_sorted_numeric_doc_values("sn")?
                .unwrap_or_else(npe)
                .range_into_bit_set(0, num_docs, 20, 40, &mut actual, 0)?;
            assert_equals(&expected, &actual);
            Ok(())
        })();
        let closed = reader.close();
        r.and(closed)
    })();
    let closed = dir.close();
    result.and(closed)
}
