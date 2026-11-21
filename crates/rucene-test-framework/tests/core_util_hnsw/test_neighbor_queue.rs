//! Port of `lucene/core/src/test/org/apache/lucene/util/hnsw/TestNeighborQueue.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::hnsw::NeighborQueue;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestNeighborQueue`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testNeighborsProduct", test_neighbors_product),
        test("testNeighborsMaxHeap", test_neighbors_max_heap),
        test("testTopMaxHeap", test_top_max_heap),
        test("testTopMinHeap", test_top_min_heap),
        test("testVisitedCount", test_visited_count),
        test("testClear", test_clear),
        test("testMaxSizeQueue", test_max_size_queue),
        test("testUnboundedQueue", test_unbounded_queue),
        test("testInvalidArguments", test_invalid_arguments),
        test("testToString", test_to_string),
    ];
    class("org.apache.lucene.util.hnsw.TestNeighborQueue", methods)
}

fn test_neighbors_product() -> Result<()> {
    // make sure we have the sign correct
    let mut nn = NeighborQueue::new(2, false)?;
    Assert::assert_true(nn.insert_with_overflow(2, 0.5));
    Assert::assert_true(nn.insert_with_overflow(1, 0.2));
    Assert::assert_true(nn.insert_with_overflow(3, 1.0));
    assert_equals_delta(0.5, nn.top_score() as f64, 0.0);
    nn.pop();
    assert_equals_delta(1.0, nn.top_score() as f64, 0.0);
    nn.pop();
    Ok(())
}

fn test_neighbors_max_heap() -> Result<()> {
    let mut nn = NeighborQueue::new(2, true)?;
    Assert::assert_true(nn.insert_with_overflow(2, 2.0));
    Assert::assert_true(nn.insert_with_overflow(1, 1.0));
    Assert::assert_false(nn.insert_with_overflow(3, 3.0));
    assert_equals_delta(2.0, nn.top_score() as f64, 0.0);
    nn.pop();
    assert_equals_delta(1.0, nn.top_score() as f64, 0.0);
    Ok(())
}

fn test_top_max_heap() -> Result<()> {
    let mut nn = NeighborQueue::new(2, true)?;
    nn.add(1, 2.0);
    nn.add(2, 1.0);
    // lower scores are better; highest score on top
    assert_equals_delta(2.0, nn.top_score() as f64, 0.0);
    assert_equals(&1, &nn.top_node());
    Ok(())
}

fn test_top_min_heap() -> Result<()> {
    let mut nn = NeighborQueue::new(2, false)?;
    nn.add(1, 0.5);
    nn.add(2, -0.5);
    // higher scores are better; lowest score on top
    assert_equals_delta(-0.5, nn.top_score() as f64, 0.0);
    assert_equals(&2, &nn.top_node());
    Ok(())
}

fn test_visited_count() -> Result<()> {
    let mut nn = NeighborQueue::new(2, false)?;
    nn.set_visited_count(100);
    assert_equals(&100, &nn.visited_count());
    Ok(())
}

fn test_clear() -> Result<()> {
    let mut nn = NeighborQueue::new(2, false)?;
    nn.add(1, 1.1);
    nn.add(2, -2.2);
    nn.set_visited_count(42);
    nn.mark_incomplete();
    nn.clear();

    assert_equals(&0, &nn.size());
    assert_equals(&0, &nn.visited_count());
    Assert::assert_false(nn.incomplete());
    Ok(())
}

fn test_max_size_queue() -> Result<()> {
    let mut nn = NeighborQueue::new(2, false)?;
    nn.add(1, 1.0);
    nn.add(2, 2.0);
    assert_equals(&2, &nn.size());
    assert_equals(&1, &nn.top_node());

    // insertWithOverflow does not extend the queue
    nn.insert_with_overflow(3, 3.0);
    assert_equals(&2, &nn.size());
    assert_equals(&2, &nn.top_node());

    // add does extend the queue beyond maxSize
    nn.add(4, 1.0);
    assert_equals(&3, &nn.size());
    Ok(())
}

fn test_unbounded_queue() -> Result<()> {
    let mut nn = NeighborQueue::new(1, true)?;
    let mut max_score = -2f32;
    let mut max_node = -1;
    for i in 0..256 {
        // initial size is 32
        let score = random().next_float();
        if score > max_score {
            max_score = score;
            max_node = i;
        }
        nn.add(i, score);
    }
    assert_equals_delta(max_score as f64, nn.top_score() as f64, 0.0);
    assert_equals(&max_node, &nn.top_node());
    Ok(())
}

fn test_invalid_arguments() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        NeighborQueue::new(0, false)
    });
    Ok(())
}

fn test_to_string() -> Result<()> {
    assert_equals(
        "Neighbors[0]",
        NeighborQueue::new(2, false)?.to_string().as_str(),
    );
    Ok(())
}
