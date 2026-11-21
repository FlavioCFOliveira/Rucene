//! Port of `lucene/core/src/test/org/apache/lucene/util/hnsw/TestNeighborArray.java`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::ASSERTION_ERROR;
use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::util::hnsw::scorer::RandomVectorScorer;
use rucene::util::hnsw::NeighborArray;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};

use crate::core_support::{
    assert_array_equals, assert_equals, assert_equals_delta, class, expect_throws_panic, test,
};

/// `TestNeighborArray`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testScoresDescOrder", test_scores_desc_order),
        test("testScoresAscOrder", test_scores_asc_order),
        test("testSortAsc", test_sort_asc),
        test("testSortDesc", test_sort_desc),
        test("testAddwithScoringFunction", test_addwith_scoring_function),
        test(
            "testAddwithScoringFunctionLargeOrd",
            test_addwith_scoring_function_large_ord,
        ),
        test("testMaxSizeAddInOrder", test_max_size_add_in_order),
        test("testMaxSizeAddOutOfOrder", test_max_size_add_out_of_order),
        test("testMaxSizeInsertSorted", test_max_size_insert_sorted),
        test("testMaxSizeMixedOperations", test_max_size_mixed_operations),
        test("testBoundaryValues", test_boundary_values),
        test("testSortOrder", test_sort_order),
        test("testRemoveOperations", test_remove_operations),
        test("testClearOperation", test_clear_operation),
        test("testComplexOperations", test_complex_operations),
        test("testScorerOperations", test_scorer_operations),
    ];
    class("org.apache.lucene.util.hnsw.TestNeighborArray", methods)
}

const ILLEGAL_STATE: &str = "java.lang.IllegalStateException";

/// `testScoresDescOrder()`.
#[allow(unused_mut, unused_assignments)]
fn test_scores_desc_order() -> Result<()> {
    let neighbors = NeighborArray::new(10, true);
    neighbors.add_in_order(0, 1.0)?;
    neighbors.add_in_order(1, 0.8)?;
    let ex = expect_throws_panic(ASSERTION_ERROR, || {
        if let Err(e) = neighbors.add_in_order(2, 0.9) {
            rethrow_java(e)
        }
    });
    assert!(
        ex.message()
            .unwrap_or("")
            .starts_with("Nodes are added in the incorrect order!"),
        "{}",
        ex.message().unwrap_or("null")
    );
    neighbors.insert_sorted(3, 0.9)?;
    assert_scores_equal(&[1.0, 0.9, 0.8], &neighbors);
    assert_nodes_equal(&[0, 3, 1], &neighbors);
    neighbors.insert_sorted(4, 1.0)?;
    assert_scores_equal(&[1.0, 1.0, 0.9, 0.8], &neighbors);
    assert_nodes_equal(&[0, 4, 3, 1], &neighbors);
    neighbors.insert_sorted(5, 1.1)?;
    assert_scores_equal(&[1.1, 1.0, 1.0, 0.9, 0.8], &neighbors);
    assert_nodes_equal(&[5, 0, 4, 3, 1], &neighbors);
    neighbors.insert_sorted(6, 0.8)?;
    assert_scores_equal(&[1.1, 1.0, 1.0, 0.9, 0.8, 0.8], &neighbors);
    assert_nodes_equal(&[5, 0, 4, 3, 1, 6], &neighbors);
    neighbors.insert_sorted(7, 0.8)?;
    assert_scores_equal(&[1.1, 1.0, 1.0, 0.9, 0.8, 0.8, 0.8], &neighbors);
    assert_nodes_equal(&[5, 0, 4, 3, 1, 6, 7], &neighbors);
    neighbors.remove_index(2);
    assert_scores_equal(&[1.1, 1.0, 0.9, 0.8, 0.8, 0.8], &neighbors);
    assert_nodes_equal(&[5, 0, 3, 1, 6, 7], &neighbors);
    neighbors.remove_index(0);
    assert_scores_equal(&[1.0, 0.9, 0.8, 0.8, 0.8], &neighbors);
    assert_nodes_equal(&[0, 3, 1, 6, 7], &neighbors);
    neighbors.remove_index(4);
    assert_scores_equal(&[1.0, 0.9, 0.8, 0.8], &neighbors);
    assert_nodes_equal(&[0, 3, 1, 6], &neighbors);
    neighbors.remove_last();
    assert_scores_equal(&[1.0, 0.9, 0.8], &neighbors);
    assert_nodes_equal(&[0, 3, 1], &neighbors);
    neighbors.insert_sorted(8, 0.9)?;
    assert_scores_equal(&[1.0, 0.9, 0.9, 0.8], &neighbors);
    assert_nodes_equal(&[0, 3, 8, 1], &neighbors);
    Ok(())
}

/// `testScoresAscOrder()`.
#[allow(unused_mut, unused_assignments)]
fn test_scores_asc_order() -> Result<()> {
    let neighbors = NeighborArray::new(10, false);
    neighbors.add_in_order(0, 0.1)?;
    neighbors.add_in_order(1, 0.3)?;
    let ex = expect_throws_panic(ASSERTION_ERROR, || {
        if let Err(e) = neighbors.add_in_order(2, 0.15) {
            rethrow_java(e)
        }
    });
    assert!(
        ex.message()
            .unwrap_or("")
            .starts_with("Nodes are added in the incorrect order!"),
        "{}",
        ex.message().unwrap_or("null")
    );
    neighbors.insert_sorted(3, 0.3)?;
    assert_scores_equal(&[0.1, 0.3, 0.3], &neighbors);
    assert_nodes_equal(&[0, 1, 3], &neighbors);
    neighbors.insert_sorted(4, 0.2)?;
    assert_scores_equal(&[0.1, 0.2, 0.3, 0.3], &neighbors);
    assert_nodes_equal(&[0, 4, 1, 3], &neighbors);
    neighbors.insert_sorted(5, 0.05)?;
    assert_scores_equal(&[0.05, 0.1, 0.2, 0.3, 0.3], &neighbors);
    assert_nodes_equal(&[5, 0, 4, 1, 3], &neighbors);
    neighbors.insert_sorted(6, 0.2)?;
    assert_scores_equal(&[0.05, 0.1, 0.2, 0.2, 0.3, 0.3], &neighbors);
    assert_nodes_equal(&[5, 0, 4, 6, 1, 3], &neighbors);
    neighbors.insert_sorted(7, 0.2)?;
    assert_scores_equal(&[0.05, 0.1, 0.2, 0.2, 0.2, 0.3, 0.3], &neighbors);
    assert_nodes_equal(&[5, 0, 4, 6, 7, 1, 3], &neighbors);
    neighbors.remove_index(2);
    assert_scores_equal(&[0.05, 0.1, 0.2, 0.2, 0.3, 0.3], &neighbors);
    assert_nodes_equal(&[5, 0, 6, 7, 1, 3], &neighbors);
    neighbors.remove_index(0);
    assert_scores_equal(&[0.1, 0.2, 0.2, 0.3, 0.3], &neighbors);
    assert_nodes_equal(&[0, 6, 7, 1, 3], &neighbors);
    neighbors.remove_index(4);
    assert_scores_equal(&[0.1, 0.2, 0.2, 0.3], &neighbors);
    assert_nodes_equal(&[0, 6, 7, 1], &neighbors);
    neighbors.remove_last();
    assert_scores_equal(&[0.1, 0.2, 0.2], &neighbors);
    assert_nodes_equal(&[0, 6, 7], &neighbors);
    neighbors.insert_sorted(8, 0.01)?;
    assert_scores_equal(&[0.01, 0.1, 0.2, 0.2], &neighbors);
    assert_nodes_equal(&[8, 0, 6, 7], &neighbors);
    Ok(())
}

/// `testSortAsc()`.
#[allow(unused_mut, unused_assignments)]
fn test_sort_asc() -> Result<()> {
    let neighbors = NeighborArray::new(10, false);
    neighbors.add_out_of_order(1, 2.0)?;
    // we disallow calling addInOrder after addOutOfOrder even if they're actual in order
    expect_throws_panic(ASSERTION_ERROR, || {
        if let Err(e) = neighbors.add_in_order(1, 2.0) {
            rethrow_java(e)
        }
    });
    neighbors.add_out_of_order(2, 3.0)?;
    neighbors.add_out_of_order(5, 6.0)?;
    neighbors.add_out_of_order(3, 4.0)?;
    neighbors.add_out_of_order(7, 8.0)?;
    neighbors.add_out_of_order(6, 7.0)?;
    neighbors.add_out_of_order(4, 5.0)?;
    let mut unchecked = neighbors.sort(&mut null_scorer())?;
    assert_array_equals(
        &[0, 1, 2, 3, 4, 5, 6],
        &unchecked.clone().unwrap_or_default(),
    );
    assert_nodes_equal(&[1, 2, 3, 4, 5, 6, 7], &neighbors);
    assert_scores_equal(&[2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], &neighbors);
    let neighbors2 = NeighborArray::new(10, false);
    neighbors2.add_in_order(0, 1.0)?;
    neighbors2.add_in_order(1, 2.0)?;
    neighbors2.add_in_order(4, 5.0)?;
    neighbors2.add_out_of_order(2, 3.0)?;
    neighbors2.add_out_of_order(6, 7.0)?;
    neighbors2.add_out_of_order(5, 6.0)?;
    neighbors2.add_out_of_order(3, 4.0)?;
    unchecked = neighbors2.sort(&mut null_scorer())?;
    assert_array_equals(&[2, 3, 5, 6], &unchecked.clone().unwrap_or_default());
    assert_nodes_equal(&[0, 1, 2, 3, 4, 5, 6], &neighbors2);
    assert_scores_equal(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], &neighbors2);
    Ok(())
}

/// `testSortDesc()`.
#[allow(unused_mut, unused_assignments)]
fn test_sort_desc() -> Result<()> {
    let neighbors = NeighborArray::new(10, true);
    neighbors.add_out_of_order(1, 7.0)?;
    // we disallow calling addInOrder after addOutOfOrder even if they're actual in order
    expect_throws_panic(ASSERTION_ERROR, || {
        if let Err(e) = neighbors.add_in_order(1, 2.0) {
            rethrow_java(e)
        }
    });
    neighbors.add_out_of_order(2, 6.0)?;
    neighbors.add_out_of_order(5, 3.0)?;
    neighbors.add_out_of_order(3, 5.0)?;
    neighbors.add_out_of_order(7, 1.0)?;
    neighbors.add_out_of_order(6, 2.0)?;
    neighbors.add_out_of_order(4, 4.0)?;
    let mut unchecked = neighbors.sort(&mut null_scorer())?;
    assert_array_equals(
        &[0, 1, 2, 3, 4, 5, 6],
        &unchecked.clone().unwrap_or_default(),
    );
    assert_nodes_equal(&[1, 2, 3, 4, 5, 6, 7], &neighbors);
    assert_scores_equal(&[7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0], &neighbors);
    let neighbors2 = NeighborArray::new(10, true);
    neighbors2.add_in_order(1, 7.0)?;
    neighbors2.add_in_order(2, 6.0)?;
    neighbors2.add_in_order(5, 3.0)?;
    neighbors2.add_out_of_order(3, 5.0)?;
    neighbors2.add_out_of_order(7, 1.0)?;
    neighbors2.add_out_of_order(6, 2.0)?;
    neighbors2.add_out_of_order(4, 4.0)?;
    unchecked = neighbors2.sort(&mut null_scorer())?;
    assert_array_equals(&[2, 3, 5, 6], &unchecked.clone().unwrap_or_default());
    assert_nodes_equal(&[1, 2, 3, 4, 5, 6, 7], &neighbors2);
    assert_scores_equal(&[7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0], &neighbors2);
    Ok(())
}

/// `testAddwithScoringFunction()`.
#[allow(unused_mut, unused_assignments)]
fn test_addwith_scoring_function() -> Result<()> {
    let neighbors = NeighborArray::new(10, true);
    neighbors.add_out_of_order(1, f32::NAN)?;
    expect_throws_panic(ASSERTION_ERROR, || {
        if let Err(e) = neighbors.add_in_order(1, 2.0) {
            rethrow_java(e)
        }
    });
    neighbors.add_out_of_order(2, f32::NAN)?;
    neighbors.add_out_of_order(5, f32::NAN)?;
    neighbors.add_out_of_order(3, f32::NAN)?;
    neighbors.add_out_of_order(7, f32::NAN)?;
    neighbors.add_out_of_order(6, f32::NAN)?;
    neighbors.add_out_of_order(4, f32::NAN)?;
    let mut unchecked = neighbors.sort(&mut TestRandomVectorScorer(|node_id: i32| {
        (7 - node_id + 1) as f32
    }))?;
    assert_array_equals(
        &[0, 1, 2, 3, 4, 5, 6],
        &unchecked.clone().unwrap_or_default(),
    );
    assert_nodes_equal(&[1, 2, 3, 4, 5, 6, 7], &neighbors);
    assert_scores_equal(&[7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0], &neighbors);
    Ok(())
}

/// `testAddwithScoringFunctionLargeOrd()`.
#[allow(unused_mut, unused_assignments)]
fn test_addwith_scoring_function_large_ord() -> Result<()> {
    let neighbors = NeighborArray::new(10, true);
    neighbors.add_out_of_order(11, f32::NAN)?;
    expect_throws_panic(ASSERTION_ERROR, || {
        if let Err(e) = neighbors.add_in_order(1, 2.0) {
            rethrow_java(e)
        }
    });
    neighbors.add_out_of_order(12, f32::NAN)?;
    neighbors.add_out_of_order(15, f32::NAN)?;
    neighbors.add_out_of_order(13, f32::NAN)?;
    neighbors.add_out_of_order(17, f32::NAN)?;
    neighbors.add_out_of_order(16, f32::NAN)?;
    neighbors.add_out_of_order(14, f32::NAN)?;
    let mut unchecked = neighbors.sort(&mut TestRandomVectorScorer(|node_id: i32| {
        (7 - node_id + 11) as f32
    }))?;
    assert_array_equals(
        &[0, 1, 2, 3, 4, 5, 6],
        &unchecked.clone().unwrap_or_default(),
    );
    assert_nodes_equal(&[11, 12, 13, 14, 15, 16, 17], &neighbors);
    assert_scores_equal(&[7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0], &neighbors);
    Ok(())
}

/// `testMaxSizeAddInOrder()`.
#[allow(unused_mut, unused_assignments)]
fn test_max_size_add_in_order() -> Result<()> {
    let neighbors = NeighborArray::new(3, true);
    neighbors.add_in_order(0, 1.0)?;
    neighbors.add_in_order(1, 0.8)?;
    neighbors.add_in_order(2, 0.6)?;
    // Should throw when trying to add beyond max size
    expect_throws(ILLEGAL_STATE, || neighbors.add_in_order(3, 0.4));
    assert_scores_equal(&[1.0, 0.8, 0.6], &neighbors);
    assert_nodes_equal(&[0, 1, 2], &neighbors);
    Ok(())
}

/// `testMaxSizeAddOutOfOrder()`.
#[allow(unused_mut, unused_assignments)]
fn test_max_size_add_out_of_order() -> Result<()> {
    let neighbors = NeighborArray::new(3, true);
    neighbors.add_out_of_order(0, 0.8)?;
    neighbors.add_out_of_order(1, 1.0)?;
    neighbors.add_out_of_order(2, 0.6)?;
    // Should throw when trying to add beyond max size
    expect_throws(ILLEGAL_STATE, || neighbors.add_out_of_order(3, 0.4));
    // Sort to verify contents
    neighbors.sort(&mut null_scorer())?;
    assert_scores_equal(&[1.0, 0.8, 0.6], &neighbors);
    assert_nodes_equal(&[1, 0, 2], &neighbors);
    Ok(())
}

/// `testMaxSizeInsertSorted()`.
#[allow(unused_mut, unused_assignments)]
fn test_max_size_insert_sorted() -> Result<()> {
    let neighbors = NeighborArray::new(3, true);
    neighbors.insert_sorted(0, 1.0)?;
    neighbors.insert_sorted(1, 0.8)?;
    neighbors.insert_sorted(2, 0.6)?;
    // Should throw when trying to insert beyond max size
    expect_throws(ILLEGAL_STATE, || neighbors.insert_sorted(3, 0.4));
    assert_scores_equal(&[1.0, 0.8, 0.6], &neighbors);
    assert_nodes_equal(&[0, 1, 2], &neighbors);
    Ok(())
}

/// `testMaxSizeMixedOperations()`.
#[allow(unused_mut, unused_assignments)]
fn test_max_size_mixed_operations() -> Result<()> {
    let neighbors = NeighborArray::new(3, true);
    // Add first node
    neighbors.add_in_order(0, 1.0)?;
    // Add second node out of order
    neighbors.add_out_of_order(1, 0.8)?;
    // Try to add third node in order - should fail because we used addOutOfOrder
    expect_throws_panic(ASSERTION_ERROR, || {
        if let Err(e) = neighbors.add_in_order(2, 0.6) {
            rethrow_java(e)
        }
    });
    // Add third node out of order
    neighbors.add_out_of_order(2, 0.6)?;
    // Try to add fourth node - should fail due to max size
    expect_throws(ILLEGAL_STATE, || neighbors.add_out_of_order(3, 0.4));
    // Sort to verify contents
    neighbors.sort(&mut null_scorer())?;
    assert_scores_equal(&[1.0, 0.8, 0.6], &neighbors);
    assert_nodes_equal(&[0, 1, 2], &neighbors);
    Ok(())
}

/// `testBoundaryValues()`.
#[allow(unused_mut, unused_assignments)]
fn test_boundary_values() -> Result<()> {
    // Test with empty array
    let neighbors = NeighborArray::new(3, true);
    assert_equals(&0, &neighbors.size());
    // Test with single element
    neighbors.add_in_order(0, f32::MAX)?;
    assert_scores_equal(&[f32::MAX], &neighbors);
    assert_nodes_equal(&[0], &neighbors);
    // Test with NaN values
    neighbors.clear();
    neighbors.add_out_of_order(0, f32::NAN)?;
    neighbors.add_out_of_order(1, f32::NAN)?;
    assert_scores_equal(&[f32::NAN, f32::NAN], &neighbors);
    assert_nodes_equal(&[0, 1], &neighbors);
    Ok(())
}

/// `testSortOrder()`.
#[allow(unused_mut, unused_assignments)]
fn test_sort_order() -> Result<()> {
    // Test ascending order
    let asc_neighbors = NeighborArray::new(5, false);
    asc_neighbors.add_in_order(0, 0.1)?;
    asc_neighbors.add_in_order(1, 0.2)?;
    asc_neighbors.add_in_order(2, 0.3)?;
    assert_scores_equal(&[0.1, 0.2, 0.3], &asc_neighbors);
    // Test descending order
    let desc_neighbors = NeighborArray::new(5, true);
    desc_neighbors.add_in_order(0, 0.3)?;
    desc_neighbors.add_in_order(1, 0.2)?;
    desc_neighbors.add_in_order(2, 0.1)?;
    assert_scores_equal(&[0.3, 0.2, 0.1], &desc_neighbors);
    // Test equal scores
    let equal_neighbors = NeighborArray::new(5, true);
    equal_neighbors.add_in_order(0, 0.5)?;
    equal_neighbors.add_in_order(1, 0.5)?;
    equal_neighbors.add_in_order(2, 0.5)?;
    assert_scores_equal(&[0.5, 0.5, 0.5], &equal_neighbors);
    Ok(())
}

/// `testRemoveOperations()`.
#[allow(unused_mut, unused_assignments)]
fn test_remove_operations() -> Result<()> {
    let neighbors = NeighborArray::new(5, true);
    neighbors.add_in_order(0, 1.0)?;
    neighbors.add_in_order(1, 0.8)?;
    neighbors.add_in_order(2, 0.6)?;
    neighbors.add_in_order(3, 0.4)?;
    // Test remove last
    neighbors.remove_last();
    assert_scores_equal(&[1.0, 0.8, 0.6], &neighbors);
    assert_nodes_equal(&[0, 1, 2], &neighbors);
    // Test remove middle
    neighbors.remove_index(1);
    assert_scores_equal(&[1.0, 0.6], &neighbors);
    assert_nodes_equal(&[0, 2], &neighbors);
    // Test remove first
    neighbors.remove_index(0);
    assert_scores_equal(&[0.6], &neighbors);
    assert_nodes_equal(&[2], &neighbors);
    // Test remove last remaining
    neighbors.remove_last();
    assert_equals(&0, &neighbors.size());
    Ok(())
}

/// `testClearOperation()`.
#[allow(unused_mut, unused_assignments)]
fn test_clear_operation() -> Result<()> {
    let neighbors = NeighborArray::new(5, true);
    neighbors.add_in_order(0, 1.0)?;
    neighbors.add_in_order(1, 0.8)?;
    neighbors.add_in_order(2, 0.6)?;
    // Test clear
    neighbors.clear();
    assert_equals(&0, &neighbors.size());
    // Test adding after clear
    neighbors.add_in_order(3, 1.0)?;
    assert_scores_equal(&[1.0], &neighbors);
    assert_nodes_equal(&[3], &neighbors);
    Ok(())
}

/// `testComplexOperations()`.
#[allow(unused_mut, unused_assignments)]
fn test_complex_operations() -> Result<()> {
    let neighbors = NeighborArray::new(5, true);
    // Add some nodes in order
    neighbors.add_in_order(0, 1.0)?;
    neighbors.add_in_order(1, 0.8)?;
    // Add some nodes out of order
    neighbors.add_out_of_order(2, 0.9)?;
    neighbors.add_out_of_order(3, 0.7)?;
    // Sort and verify
    neighbors.sort(&mut null_scorer())?;
    assert_scores_equal(&[1.0, 0.9, 0.8, 0.7], &neighbors);
    assert_nodes_equal(&[0, 2, 1, 3], &neighbors);
    // Remove some nodes
    neighbors.remove_index(1);
    neighbors.remove_last();
    // Add more nodes
    neighbors.insert_sorted(4, 0.85)?;
    neighbors.insert_sorted(5, 0.75)?;
    // Final verification
    assert_scores_equal(&[1.0, 0.85, 0.8, 0.75], &neighbors);
    assert_nodes_equal(&[0, 4, 1, 5], &neighbors);
    Ok(())
}

/// `testScorerOperations()`.
fn test_scorer_operations() -> Result<()> {
    let neighbors = NeighborArray::new(5, true);
    let mut scorer = TestRandomVectorScorer(|node_id: i32| match node_id {
        0 => 1.0f32,
        1 => 0.8,
        2 => 0.9,
        3 => 0.7,
        _ => 0.0,
    });
    neighbors.add_out_of_order(0, f32::NAN)?;
    neighbors.add_out_of_order(1, f32::NAN)?;
    neighbors.add_out_of_order(2, f32::NAN)?;
    neighbors.add_out_of_order(3, f32::NAN)?;
    neighbors.sort(&mut scorer)?;
    assert_scores_equal(&[1.0, 0.9, 0.8, 0.7], &neighbors);
    assert_nodes_equal(&[0, 2, 1, 3], &neighbors);
    Ok(())
}

/// `assertScoresEqual(scores, neighbors)`.
fn assert_scores_equal(scores: &[f32], neighbors: &NeighborArray) {
    for (i, &score) in scores.iter().enumerate() {
        assert_equals_delta(score as f64, neighbors.score(i as i32) as f64, 0.01);
    }
}

/// `assertNodesEqual(nodes, neighbors)`.
fn assert_nodes_equal(nodes: &[i32], neighbors: &NeighborArray) {
    for (i, node) in nodes.iter().enumerate() {
        assert_equals(node, &neighbors.nodes()[i]);
    }
}

/// `interface TestRandomVectorScorer extends RandomVectorScorer`: a
/// scorer given as its `score(int)` lambda, whose other members throw
/// `UnsupportedOperationException`.
struct TestRandomVectorScorer<F: FnMut(i32) -> f32>(F);

impl<F: FnMut(i32) -> f32> RandomVectorScorer for TestRandomVectorScorer<F> {
    fn score(&mut self, node: i32) -> Result<f32> {
        Ok((self.0)(node))
    }

    fn max_ord(&self) -> i32 {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.UnsupportedOperationException",
            String::new(),
        )
        .throw()
    }

    fn ord_to_doc(&self, _ord: i32) -> i32 {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.UnsupportedOperationException",
            String::new(),
        )
        .throw()
    }
}

/// The `null` scorer of `neighbors.sort(null)`.
///
/// **Divergence from Lucene 10.5.0.** Java passes `null`; Rucene's
/// `NeighborArray::sort` takes a scorer, so the port passes one whose every
/// call throws the `NullPointerException` dereferencing Java's `null` would.
fn null_scorer() -> impl RandomVectorScorer {
    TestRandomVectorScorer(|_node_id: i32| -> f32 {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.NullPointerException",
            String::new(),
        )
        .throw()
    })
}
