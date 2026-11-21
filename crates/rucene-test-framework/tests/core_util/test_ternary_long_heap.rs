//! Port of `lucene/core/src/test/org/apache/lucene/util/TestTernaryLongHeap.java`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{ArrayUtil, TernaryLongHeap};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, assert_equals, class, random, test};

/// `TestTernaryLongHeap`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestTernaryLongHeap",
        vec![
            test("testPQ", test_pq),
            test("testClear", test_clear),
            test("testExceedBounds", test_exceed_bounds),
            test("testFixedSize", test_fixed_size),
            test("testDuplicateValues", test_duplicate_values),
            test("testInsertions", test_insertions),
            test("testInvalid", test_invalid),
            test("testUnbounded", test_unbounded),
        ],
    )
}

/// `checkValidity(heap)`.
fn check_validity(heap: &TernaryLongHeap) {
    let heap_array = heap.get_heap_array();
    let d = 3usize;
    let size = heap.size();
    for parent in 1..=size {
        let first_child = d * (parent - 1) + 2;
        let last_child = (first_child + d - 1).min(size);
        for c in first_child..=last_child {
            // Java `assert`.
            Assert::assert_true(heap_array[parent] <= heap_array[c]);
        }
    }
}

fn test_pq() -> Result<()> {
    test_pq_with(LuceneTestCase::at_least(10000), &*random())
}

/// `testPQ(int count, Random gen)`.
pub fn test_pq_with(count: i32, gen: &dyn Random) -> Result<()> {
    let mut pq = TernaryLongHeap::new(count as usize)?;
    let mut sum: i64 = 0;
    let mut sum2: i64 = 0;

    for _ in 0..count {
        let next = gen.next_long();
        sum = sum.wrapping_add(next);
        pq.push(next);
    }

    let mut last = i64::MIN;
    for _ in 0..count as i64 {
        let next = pq.pop()?;
        Assert::assert_true(next >= last);
        last = next;
        sum2 = sum2.wrapping_add(last);
    }

    assert_equals(&sum, &sum2);
    Ok(())
}

fn test_clear() -> Result<()> {
    let mut pq = TernaryLongHeap::new(3)?;
    pq.push(2);
    pq.push(3);
    pq.push(1);
    assert_equals(&3, &pq.size());
    pq.clear();
    assert_equals(&0, &pq.size());
    Ok(())
}

fn test_exceed_bounds() -> Result<()> {
    let mut pq = TernaryLongHeap::new(1)?;
    pq.push(2);
    pq.push(0);
    // expectThrows(ArrayIndexOutOfBoundsException.class, () -> pq.push(0));
    assert_equals(&2, &pq.size()); // the heap has been extended to a new max size
    assert_equals(&0, &pq.top());
    Ok(())
}

fn test_fixed_size() -> Result<()> {
    let mut pq = TernaryLongHeap::new(3)?;
    pq.insert_with_overflow(2);
    pq.insert_with_overflow(3);
    pq.insert_with_overflow(1);
    pq.insert_with_overflow(5);
    pq.insert_with_overflow(7);
    pq.insert_with_overflow(1);
    assert_equals(&3, &pq.size());
    assert_equals(&3, &pq.top());
    Ok(())
}

fn test_duplicate_values() -> Result<()> {
    let mut pq = TernaryLongHeap::new(3)?;
    pq.push(2);
    pq.push(3);
    pq.push(1);
    assert_equals(&1, &pq.top());
    pq.update_top(3);
    assert_equals(&3, &pq.size());
    assert_array_equals(&[0i64, 2, 3, 3], pq.get_heap_array());
    Ok(())
}

fn test_insertions() -> Result<()> {
    let random = random();
    let num_docs_in_pq = TestUtil::next_int(&*random, 1, 100);
    let mut pq = TernaryLongHeap::new(num_docs_in_pq as usize)?;
    let mut last_least: Option<i64> = None;

    // Basic insertion of new content
    let mut sds: Vec<i64> = Vec::with_capacity(num_docs_in_pq as usize);
    for _ in 0..num_docs_in_pq * 10 {
        // `Math.abs(long)`: `Long.MIN_VALUE` stays negative.
        let new_entry = random.next_long().wrapping_abs();
        sds.push(new_entry);
        pq.insert_with_overflow(new_entry);
        check_validity(&pq);
        let new_least = pq.top();
        if let Some(ll) = last_least {
            if new_least != new_entry && new_least != ll {
                // If there has been a change of least entry and it wasn't our new
                // addition we expect the scores to increase
                Assert::assert_true(new_least <= new_entry);
                Assert::assert_true(new_least >= ll);
            }
        }
        last_least = Some(new_least);
    }
    Ok(())
}

fn test_invalid() -> Result<()> {
    // Java's `-1`, passed where Rucene takes a `usize`.
    expect_throws("java.lang.IllegalArgumentException", || {
        TernaryLongHeap::new(-1i64 as usize)
    });
    expect_throws("java.lang.IllegalArgumentException", || {
        TernaryLongHeap::new(0)
    });
    expect_throws("java.lang.IllegalArgumentException", || {
        TernaryLongHeap::new(ArrayUtil::MAX_ARRAY_LENGTH)
    });
    Ok(())
}

fn test_unbounded() -> Result<()> {
    let initial_size = random().next_int_bound(10) + 1;
    let mut pq = TernaryLongHeap::new(initial_size as usize)?;
    let num = random().next_int_bound(100) + 1;
    let mut max_value = i64::MIN;
    let mut count = 0i32;
    for _ in 0..num {
        let value = random().next_long();
        if random().next_boolean() {
            pq.push(value);
            count += 1;
        } else {
            let full = pq.size() >= initial_size as usize;
            if pq.insert_with_overflow(value) && !full {
                count += 1;
            }
        }
        max_value = max_value.max(value);
    }
    assert_equals(&(count as usize), &pq.size());
    let mut last = i64::MIN;
    while pq.size() > 0 {
        let top = pq.top();
        let next = pq.pop()?;
        assert_equals(&top, &next);
        count -= 1;
        Assert::assert_true(next >= last);
        last = next;
    }
    assert_equals(&0, &count);
    assert_equals(&max_value, &last);
    Ok(())
}
