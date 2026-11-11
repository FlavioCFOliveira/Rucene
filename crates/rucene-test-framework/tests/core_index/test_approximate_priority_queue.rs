//! Port of `lucene/core/src/test/org/apache/lucene/index/TestApproximatePriorityQueue.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::index_utilities::ApproximatePriorityQueue;

use crate::core_support::{assert_equals, assert_null, class, test};

/// `TestApproximatePriorityQueue`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestApproximatePriorityQueue",
        vec![
            test("testBasics", test_basics),
            test("testPollThenAdd", test_poll_then_add),
            test("testCollision", test_collision),
            test("testPollWithPredicate", test_poll_with_predicate),
            test(
                "testCollisionPollWithPredicate",
                test_collision_poll_with_predicate,
            ),
            test("testRemove", test_remove),
        ],
    )
}

/// Port of `TestApproximatePriorityQueue.testBasics`.
fn test_basics() -> Result<()> {
    let mut pq: ApproximatePriorityQueue<i64> = ApproximatePriorityQueue::new();
    pq.add(8, 8);
    pq.add(32, 32);
    pq.add(0, 0);
    Assert::assert_false(pq.is_empty());
    assert_equals(&Some(32), &pq.poll(|_| true));
    Assert::assert_false(pq.is_empty());
    assert_equals(&Some(8), &pq.poll(|_| true));
    Assert::assert_false(pq.is_empty());
    assert_equals(&Some(0), &pq.poll(|_| true));
    Assert::assert_true(pq.is_empty());
    assert_null(&pq.poll(|_| true));
    Ok(())
}

/// Port of `TestApproximatePriorityQueue.testPollThenAdd`.
fn test_poll_then_add() -> Result<()> {
    let mut pq: ApproximatePriorityQueue<i64> = ApproximatePriorityQueue::new();
    pq.add(8, 8);
    assert_equals(&Some(8), &pq.poll(|_| true));
    assert_null(&pq.poll(|_| true));
    pq.add(0, 0);
    assert_equals(&Some(0), &pq.poll(|_| true));
    assert_null(&pq.poll(|_| true));
    pq.add(0, 0);
    assert_equals(&Some(0), &pq.poll(|_| true));
    assert_null(&pq.poll(|_| true));
    Ok(())
}

/// Port of `TestApproximatePriorityQueue.testCollision`.
fn test_collision() -> Result<()> {
    let mut pq: ApproximatePriorityQueue<i64> = ApproximatePriorityQueue::new();
    pq.add(2, 2);
    pq.add(1, 1);
    pq.add(0, 0);
    pq.add(3, 3); // Same nlz as 2
    Assert::assert_false(pq.is_empty());
    assert_equals(&Some(2), &pq.poll(|_| true));
    Assert::assert_false(pq.is_empty());
    assert_equals(&Some(1), &pq.poll(|_| true));
    Assert::assert_false(pq.is_empty());
    assert_equals(&Some(3), &pq.poll(|_| true));
    Assert::assert_false(pq.is_empty());
    assert_equals(&Some(0), &pq.poll(|_| true));
    Assert::assert_true(pq.is_empty());
    assert_null(&pq.poll(|_| true));
    Ok(())
}

/// Port of `TestApproximatePriorityQueue.testPollWithPredicate`.
fn test_poll_with_predicate() -> Result<()> {
    let mut pq: ApproximatePriorityQueue<i64> = ApproximatePriorityQueue::new();
    pq.add(8, 8);
    pq.add(32, 32);
    pq.add(0, 0);
    assert_equals(&Some(8), &pq.poll(|x| *x == 8));
    assert_null(&pq.poll(|x| *x == 8));
    Assert::assert_false(pq.is_empty());
    Ok(())
}

/// Port of `TestApproximatePriorityQueue.testCollisionPollWithPredicate`.
fn test_collision_poll_with_predicate() -> Result<()> {
    let mut pq: ApproximatePriorityQueue<i64> = ApproximatePriorityQueue::new();
    pq.add(2, 2);
    pq.add(1, 1);
    pq.add(0, 0);
    pq.add(3, 3); // Same nlz as 2
    assert_equals(&Some(1), &pq.poll(|x| x % 2 == 1));
    assert_equals(&Some(3), &pq.poll(|x| x % 2 == 1));
    assert_null(&pq.poll(|x| x % 2 == 1));
    Assert::assert_false(pq.is_empty());
    Ok(())
}

/// Port of `TestApproximatePriorityQueue.testRemove`.
fn test_remove() -> Result<()> {
    let mut pq: ApproximatePriorityQueue<i64> = ApproximatePriorityQueue::new();
    pq.add(8, 8);
    pq.add(32, 32);
    pq.add(0, 0);

    Assert::assert_false(pq.remove(&16));
    Assert::assert_false(pq.remove(&9));
    Assert::assert_true(pq.remove(&8));
    Assert::assert_true(pq.remove(&0));
    Assert::assert_false(pq.remove(&0));
    Assert::assert_true(pq.remove(&32));
    Assert::assert_true(pq.is_empty());
    Ok(())
}
