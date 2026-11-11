//! Port of `lucene/core/src/test/org/apache/lucene/document/TestSortedSkipperScorerSupplier.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::lucene90::doc_values::Lucene90DocValuesFormat;
use rucene::document::{Document, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{IndexSearcher, ScoreMode};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::hamcrest::fail_that;
use crate::core_support::{assert_equals, assert_equals_msg, class, test};

/// `TestSortedSkipperScorerSupplier`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestSortedSkipperScorerSupplier",
        vec![
            test(
                "testCostBothBoundariesExact",
                test_cost_both_boundaries_exact,
            ),
            test(
                "testCostMinInexactMaxExact",
                test_cost_min_inexact_max_exact,
            ),
            test(
                "testCostMinExactMaxInexact",
                test_cost_min_exact_max_inexact,
            ),
            test(
                "testCostBothBoundariesInexact",
                test_cost_both_boundaries_inexact,
            ),
            test(
                "testCostWithOversizedNeighborInterval",
                test_cost_with_oversized_neighbor_interval,
            ),
        ],
    )
}

/// `buildIndexAndGetCosts(blockSizes, skipIntervalSize, reverse)`: builds a
/// single-segment sorted index with contiguous blocks of numeric doc values
/// and returns the cost of an exact-match query on each value.
fn build_index_and_get_costs(
    block_sizes: &[i32],
    skip_interval_size: i32,
    reverse: bool,
) -> Result<Vec<i64>> {
    let dir = LuceneTestCase::new_directory();
    let outcome = (|| -> Result<Vec<i64>> {
        let config = IndexWriterConfig::new();
        config.set_codec(TestUtil::always_doc_values_format(Arc::new(
            Lucene90DocValuesFormat::with_interval(skip_interval_size),
        )))?;
        config.set_index_sort(Sort::new_fields(vec![SortField::new_reverse(
            Some("dv".to_string()),
            SortFieldType::Long,
            reverse,
        )?])?)?;
        // not RandomIndexWriter so we stay deterministic
        let iw = IndexWriter::new(dir.as_directory(), config)?;
        let written = (|| -> Result<()> {
            for (i, &size) in block_sizes.iter().enumerate() {
                for _j in 0..size {
                    let mut doc = Document::new();
                    doc.add(Box::new(NumericDocValuesField::indexed_field(
                        "dv", i as i64,
                    )));
                    iw.add_document(&doc)?;
                }
            }
            iw.force_merge(1)
        })();
        let closed = iw.close();
        written?;
        closed?;

        let mut costs = vec![0i64; block_sizes.len()];
        let reader: Arc<dyn IndexReader> =
            rucene::index::directory_reader::open(dir.as_directory())?;
        let read = (|| -> Result<()> {
            let leaves = Arc::clone(&reader).leaves()?;
            assert_equals(&1, &leaves.len());
            let ctx = Arc::clone(&leaves[0]);
            let searcher = IndexSearcher::new(Arc::clone(&reader))?;
            for (i, &size) in block_sizes.iter().enumerate() {
                let q = NumericDocValuesField::new_slow_range_query("dv", i as i64, i as i64);
                assert_equals(&size, &searcher.count(Arc::clone(&q))?);
                let rewritten = searcher.rewrite(q)?;
                let weight =
                    rewritten.create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
                let supplier = weight.scorer_supplier(&ctx)?.expect("supplier");
                costs[i] = supplier.cost();
            }
            Ok(())
        })();
        let closed = reader.close();
        read?;
        closed?;
        Ok(costs)
    })();
    let closed = dir.close();
    let costs = outcome?;
    closed?;
    Ok(costs)
}

/// `assertCostWithinBounds(cost, blockSize, skipIntervalSize, label)`.
fn assert_cost_within_bounds(cost: i64, block_size: i32, skip_interval_size: i32, label: &str) {
    // assertThat(label, cost, greaterThanOrEqualTo((long) blockSize))
    if cost < block_size as i64 {
        fail_that(
            label,
            &format!("a value equal to or greater than <{block_size}L>"),
            &format!("<{cost}L> was less than <{block_size}L>"),
        );
    }
    // assertThat(label, cost, lessThanOrEqualTo(blockSize + 2L * skipIntervalSize))
    let max = block_size as i64 + 2 * skip_interval_size as i64;
    if cost > max {
        fail_that(
            label,
            &format!("a value less than or equal to <{max}L>"),
            &format!("<{cost}L> was greater than <{max}L>"),
        );
    }
}

/// Port of `testCostBothBoundariesExact`: each block is >= skipIntervalSize
/// and has a single value, so the skip index creates one interval per block
/// with exact boundaries on both sides.
fn test_cost_both_boundaries_exact() -> Result<()> {
    let skip_interval_size = 16;
    let block_sizes = [32, 48, 32];
    for reverse in [false, true] {
        let costs = build_index_and_get_costs(&block_sizes, skip_interval_size, reverse)?;
        for i in 0..block_sizes.len() {
            assert_equals_msg(
                Some(&format!("block {i} reverse={reverse}")),
                &(block_sizes[i] as i64),
                &costs[i],
            );
        }
    }
    Ok(())
}

/// Port of `testCostMinInexactMaxExact`.
fn test_cost_min_inexact_max_exact() -> Result<()> {
    // Block B (val=1, 8 docs) shares its interval with block A (val=0, 8 docs).
    // The next interval starts with val=2, so the max boundary for val=1 is exact,
    // but the min boundary is inexact (interval also contains val=0).
    let skip_interval_size = 16;
    let block_sizes = [8, 8, 32];
    for reverse in [false, true] {
        let costs = build_index_and_get_costs(&block_sizes, skip_interval_size, reverse)?;
        assert_cost_within_bounds(
            costs[1],
            block_sizes[1],
            skip_interval_size,
            &format!("reverse={reverse}"),
        );
    }
    Ok(())
}

/// Port of `testCostMinExactMaxInexact`.
fn test_cost_min_exact_max_inexact() -> Result<()> {
    // Block B (val=1, 4 docs) starts at its own interval boundary but shares the
    // interval tail with block C (val=2, 12 docs). The min boundary is exact
    // (block A fills prior intervals cleanly), but the max boundary is inexact.
    let skip_interval_size = 16;
    let block_sizes = [32, 4, 12];
    for reverse in [false, true] {
        let costs = build_index_and_get_costs(&block_sizes, skip_interval_size, reverse)?;
        assert_cost_within_bounds(
            costs[1],
            block_sizes[1],
            skip_interval_size,
            &format!("reverse={reverse}"),
        );
    }
    Ok(())
}

/// Port of `testCostBothBoundariesInexact`.
fn test_cost_both_boundaries_inexact() -> Result<()> {
    // Block C (val=2, 4 docs) sits in the middle of an interval that also contains
    // vals 1 and 3, so both boundaries are inexact.
    let skip_interval_size = 16;
    let block_sizes = [32, 4, 4, 8, 32];
    for reverse in [false, true] {
        let costs = build_index_and_get_costs(&block_sizes, skip_interval_size, reverse)?;
        assert_cost_within_bounds(
            costs[2],
            block_sizes[2],
            skip_interval_size,
            &format!("reverse={reverse}"),
        );
    }
    Ok(())
}

/// Port of `testCostWithOversizedNeighborInterval`.
fn test_cost_with_oversized_neighbor_interval() -> Result<()> {
    // Block B (val=1, 8 docs) shares its first interval with block A (val=0, 8 docs).
    // Block C (val=2, 64 docs) forms a single oversized interval (docCount >> skipIntervalSize)
    // because all its docs share the same value.
    // The max boundary for val=1 is exact (next interval starts with val=2), so the
    // cost must NOT include the oversized interval's docCount as padding.
    // Without the fix, cost would be 16 + 64 = 80, exceeding the bound of 8 + 32 = 40.
    let skip_interval_size = 16;
    let block_sizes = [8, 8, 64];
    for reverse in [false, true] {
        let costs = build_index_and_get_costs(&block_sizes, skip_interval_size, reverse)?;
        assert_cost_within_bounds(
            costs[1],
            block_sizes[1],
            skip_interval_size,
            &format!("reverse={reverse}"),
        );
    }
    Ok(())
}
