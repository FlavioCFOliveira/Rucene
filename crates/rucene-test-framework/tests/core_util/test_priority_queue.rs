//! Port of `lucene/core/src/test/org/apache/lucene/util/TestPriorityQueue.java`.
//!
//! `IntegerQueue extends PriorityQueue<Integer>` is a [`PriorityQueue`] over
//! [`Integer`], a boxed `int` whose `==` is Java's reference comparison
//! (`Arc::ptr_eq`): the test compares boxes by identity (`evicted !=
//! newEntry`, `assertSame`) and `PriorityQueue.remove` finds the element by
//! identity. Value comparisons (`equals`, `<`) read the boxed `int`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{PriorityQueue, PriorityQueueComparator};
use rucene_test_framework::hamcrest::{
    equal_to, greater_than_or_equal_to, less_than_or_equal_to, MatcherAssert,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_null, class, expect_throws_panic, random, test,
};

/// `TestPriorityQueue`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestPriorityQueue",
        vec![
            test("testZeroSizedQueue", test_zero_sized_queue),
            test(
                "testNoExtraWorkOnEqualElements",
                test_no_extra_work_on_equal_elements,
            ),
            test("testPQ", test_pq),
            test("testClear", test_clear),
            test("testFixedSize", test_fixed_size),
            test("testInsertWithOverflow", test_insert_with_overflow),
            test("testAddAllWithStream", test_add_all_with_stream),
            test(
                "testAddAllWithStreamNotFitIntoQueue",
                test_add_all_with_stream_not_fit_into_queue,
            ),
            test("testAddAllToEmptyQueue", test_add_all_to_empty_queue),
            test(
                "testAddAllToPartiallyFilledQueue",
                test_add_all_to_partially_filled_queue,
            ),
            test(
                "testAddAllDoesNotFitIntoQueue",
                test_add_all_does_not_fit_into_queue,
            ),
            test("testRemovalsAndInsertions", test_removals_and_insertions),
            test("testIteratorEmpty", test_iterator_empty),
            test("testIteratorOne", test_iterator_one),
            test("testIteratorTwo", test_iterator_two),
            test("testIteratorRandom", test_iterator_random),
            test("testMaxIntSize", test_max_int_size),
        ],
    )
}

/// A `java.lang.Integer`: `==` is reference identity.
#[derive(Clone, Debug)]
struct Integer(Arc<i32>);

impl Integer {
    /// `Integer.valueOf(v)` outside the small-value cache: a fresh box.
    fn new(v: i32) -> Integer {
        Integer(Arc::new(v))
    }

    fn int_value(&self) -> i32 {
        *self.0
    }

    /// `equals(Object)`.
    fn equals(&self, other: &Integer) -> bool {
        *self.0 == *other.0
    }
}

impl PartialEq for Integer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// `IntegerQueue.lessThan(Integer a, Integer b)`: `a < b`.
struct IntegerLess;

impl PriorityQueueComparator<Integer> for IntegerLess {
    fn less_than(&self, a: &Integer, b: &Integer) -> bool {
        a.int_value() < b.int_value()
    }
}

type IntegerQueue = PriorityQueue<Integer, IntegerLess>;

/// `new IntegerQueue(count)`.
fn integer_queue(count: usize) -> IntegerQueue {
    match PriorityQueue::new(count, IntegerLess) {
        Ok(pq) => pq,
        Err(e) => rucene_test_framework::util::java_exceptions::rethrow_java(e),
    }
}

/// `IntegerQueue.checkValidity()`.
fn check_validity(pq: &IntegerQueue) {
    let heap_array = pq.heap_array();
    for i in 1..=pq.size() {
        let parent = i >> 1;
        if parent > 1 {
            let p = heap_array[parent].as_ref().expect("heap slot");
            let c = heap_array[i].as_ref().expect("heap slot");
            if !IntegerLess.less_than(p, c) {
                assert_equals(&p.int_value(), &c.int_value());
            }
        }
    }
}

/// `pq.top().intValue()`: a `NullPointerException` on an empty queue.
fn top_int(pq: &IntegerQueue) -> i32 {
    match pq.top() {
        Some(t) => t.int_value(),
        None => Throwable::new("java.lang.NullPointerException", "null".to_string()).throw(),
    }
}

/// `String.hashCode()`.
fn string_hash_code(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

fn test_zero_sized_queue() -> Result<()> {
    let mut pq = integer_queue(0);
    let one = Integer::new(1);
    assert_equals(
        &Some(1),
        &pq.insert_with_overflow(one).map(|i| i.int_value()),
    );
    assert_equals(&0, &pq.size());

    // should fail, but passes and modifies the top...
    pq.add(Integer::new(1));
    assert_equals(&1, &top_int(&pq));
    Ok(())
}

fn test_no_extra_work_on_equal_elements() -> Result<()> {
    struct Value {
        index: i32,
        value: i32,
    }
    struct ValueLess;
    impl PriorityQueueComparator<Value> for ValueLess {
        fn less_than(&self, a: &Value, b: &Value) -> bool {
            a.value < b.value
        }
    }

    let mut pq = PriorityQueue::new(5, ValueLess)?;
    for i in 0..100 {
        pq.insert_with_overflow(Value { index: i, value: 0 });
    }

    let mut indexes: Vec<i32> = Vec::new();
    for e in pq.iter() {
        indexes.push(e.index);
    }

    // When sorting by priority only, the queue should not do any extra work
    // on equal elements. assertThat(indexes, containsInAnyOrder(0, 1, 2, 3, 4)):
    indexes.sort();
    assert_equals(&vec![0, 1, 2, 3, 4], &indexes);
    Ok(())
}

fn test_pq() -> Result<()> {
    test_pq_with(LuceneTestCase::at_least(10000), &*random());
    Ok(())
}

/// `testPQ(int count, Random gen)`.
pub fn test_pq_with(count: i32, gen: &dyn Random) {
    let mut pq = integer_queue(count as usize);
    let mut sum: i32 = 0;
    let mut sum2: i32 = 0;

    for _ in 0..count {
        let next = gen.next_int();
        sum = sum.wrapping_add(next);
        pq.add(Integer::new(next));
    }

    let mut last = i32::MIN;
    for _ in 0..count {
        let next = pq.pop().map(|i| i.int_value()).unwrap_or_else(|| {
            Throwable::new("java.lang.NullPointerException", "null".to_string()).throw()
        });
        MatcherAssert::assert_that(&next, &greater_than_or_equal_to(last));
        last = next;
        sum2 = sum2.wrapping_add(last);
    }

    assert_equals(&sum, &sum2);
}

fn test_clear() -> Result<()> {
    let mut pq = integer_queue(3);
    pq.add(Integer::new(2));
    pq.add(Integer::new(3));
    pq.add(Integer::new(1));
    assert_equals(&3, &pq.size());
    pq.clear();
    assert_equals(&0, &pq.size());
    Ok(())
}

fn test_fixed_size() -> Result<()> {
    let mut pq = integer_queue(3);
    for v in [2, 3, 1, 5, 7, 1] {
        pq.insert_with_overflow(Integer::new(v));
    }
    assert_equals(&3, &pq.size());
    assert_equals(&3, &top_int(&pq));
    Ok(())
}

fn test_insert_with_overflow() -> Result<()> {
    let size = 4;
    let mut pq = integer_queue(size);
    let i1 = Integer::new(2);
    let i2 = Integer::new(3);
    let i3 = Integer::new(1);
    let i4 = Integer::new(5);
    let i5 = Integer::new(7);
    let i6 = Integer::new(1);

    assert_null(&pq.insert_with_overflow(i1));
    assert_null(&pq.insert_with_overflow(i2));
    assert_null(&pq.insert_with_overflow(i3.clone()));
    assert_null(&pq.insert_with_overflow(i4));
    // i3 should have been dropped (`equalTo` is `equals`)
    let dropped = pq.insert_with_overflow(i5).expect("dropped element");
    MatcherAssert::assert_that(&dropped.int_value(), &equal_to(i3.int_value()));
    // i6 should not have been inserted
    let dropped = pq
        .insert_with_overflow(i6.clone())
        .expect("dropped element");
    MatcherAssert::assert_that(&dropped.int_value(), &equal_to(i6.int_value()));
    MatcherAssert::assert_that(&(pq.size() as i32), &equal_to(size as i32));
    MatcherAssert::assert_that(&top_int(&pq), &equal_to(2));
    Ok(())
}

/// The letters of `letters`, each added first or last on a coin flip.
fn shuffled_letters(letters: &[&'static str]) -> Vec<&'static str> {
    let mut elements: Vec<&'static str> = Vec::new();
    for &s in letters {
        if random().next_boolean() {
            elements.insert(0, s);
        } else {
            elements.push(s);
        }
    }
    elements
}

fn test_add_all_with_stream() -> Result<()> {
    let mut pq = integer_queue(6);
    let elements = shuffled_letters(&["a", "b", "c", "d", "e", "f"]);
    pq.add_all(elements.iter().map(|s| Integer::new(string_hash_code(s))));
    assert_equals(&string_hash_code("a"), &top_int(&pq));
    Ok(())
}

fn test_add_all_with_stream_not_fit_into_queue() -> Result<()> {
    let mut pq = integer_queue(10);
    let elements = shuffled_letters(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]);
    // assertThrows(message, ArrayIndexOutOfBoundsException.class, ...): the
    // message is assertThrows' failure message.
    expect_throws_panic("java.lang.ArrayIndexOutOfBoundsException", || {
        pq.add_all(elements.iter().map(|s| Integer::new(string_hash_code(s))))
    });
    assert_equals(&10, &pq.size());
    Assert::assert_true(assert_heap(&pq));
    Ok(())
}

/// `assertHeap(pq)`.
fn assert_heap(pq: &IntegerQueue) -> bool {
    let heap_array = pq.heap_array();
    let v = |i: usize| -> i32 {
        heap_array[i]
            .as_ref()
            .map(|e| e.int_value())
            .unwrap_or_else(|| {
                Throwable::new("java.lang.NullPointerException", "null".to_string()).throw()
            })
    };
    let mut i = heap_array.len() >> 1;
    while i >= 1 {
        let left = i << 1;
        let right = left + 1;
        if right < heap_array.len() {
            if v(i) > v(right) {
                return false;
            }
            if v(i) > v(left) {
                return false;
            }
        } else if left < heap_array.len() && v(i) > v(left) {
            return false;
        }
        i -= 1;
    }
    true
}

fn test_add_all_to_empty_queue() -> Result<()> {
    let random = random();
    let size = 10;
    let mut list: Vec<i32> = Vec::new();
    for _ in 0..size {
        list.push(random.next_int());
    }
    let mut pq = integer_queue(size);
    pq.add_all(list.iter().map(|&v| Integer::new(v)));

    check_validity(&pq);
    assert_ordered_when_drained(&mut pq, &mut list);
    Ok(())
}

fn test_add_all_to_partially_filled_queue() -> Result<()> {
    let mut pq = integer_queue(20);
    let mut one_by_one: Vec<i32> = Vec::new();
    let mut bulk_added: Vec<i32> = Vec::new();
    let random = random();
    for _ in 0..10 {
        bulk_added.push(random.next_int());

        let x = random.next_int();
        pq.add(Integer::new(x));
        one_by_one.push(x);
    }

    pq.add_all(bulk_added.iter().map(|&v| Integer::new(v)));
    check_validity(&pq);

    one_by_one.extend_from_slice(&bulk_added); // Gather all "reference" data.
    assert_ordered_when_drained(&mut pq, &mut one_by_one);
    Ok(())
}

fn test_add_all_does_not_fit_into_queue() -> Result<()> {
    let mut pq = integer_queue(20);
    let mut list: Vec<i32> = Vec::new();
    let random = random();
    for _ in 0..11 {
        list.push(random.next_int());
        pq.add(Integer::new(random.next_int()));
    }

    assert_equals(&11, &pq.size());
    expect_throws_panic("java.lang.ArrayIndexOutOfBoundsException", || {
        pq.add_all(list.iter().map(|&v| Integer::new(v)))
    });
    assert_equals(&20, &pq.size());
    Assert::assert_true(assert_heap(&pq));
    Ok(())
}

/// `ArrayList.remove(Object)`: removes the first element `equals` to `o`.
fn list_remove(list: &mut Vec<Integer>, o: &Integer) -> bool {
    match list.iter().position(|e| e.equals(o)) {
        Some(p) => {
            list.remove(p);
            true
        }
        None => false,
    }
}

/// `Math.abs(random.nextInt())` boxed (`Integer.MIN_VALUE` stays negative).
fn abs_next_int(random: &dyn Random) -> Integer {
    Integer::new(random.next_int().wrapping_abs())
}

fn test_removals_and_insertions() -> Result<()> {
    let random = random();
    let num_docs_in_pq = TestUtil::next_int(&*random, 1, 100);
    let mut pq = integer_queue(num_docs_in_pq as usize);
    let mut last_least: Option<Integer> = None;

    // Basic insertion of new content
    let mut sds: Vec<Integer> = Vec::with_capacity(num_docs_in_pq as usize);
    for _ in 0..num_docs_in_pq * 10 {
        let new_entry = abs_next_int(&*random);
        sds.push(new_entry.clone());
        let evicted = pq.insert_with_overflow(new_entry.clone());
        check_validity(&pq);
        if let Some(evicted) = evicted {
            Assert::assert_true(list_remove(&mut sds, &evicted));
            if evicted != new_entry {
                // assertSame(evicted, lastLeast)
                Assert::assert_true(Some(&evicted) == last_least.as_ref());
            }
        }
        let new_least = pq.top().cloned();
        if let (Some(ll), Some(nl)) = (&last_least, &new_least) {
            if *nl != new_entry && nl != ll {
                // If there has been a change of least entry and it wasn't our
                // new addition we expect the scores to increase
                MatcherAssert::assert_that(
                    &nl.int_value(),
                    &less_than_or_equal_to(new_entry.int_value()),
                );
                MatcherAssert::assert_that(
                    &nl.int_value(),
                    &greater_than_or_equal_to(ll.int_value()),
                );
            }
        }
        last_least = new_least;
    }

    // Try many random additions to existing entries - we should always see
    // increasing scores in the lowest entry in the PQ
    for _ in 0..500000 {
        let element = (random.next_float() * (sds.len() as f32 - 1.0)) as i32 as usize;
        let object_to_remove = sds[element].clone();
        // assertSame(sds.remove(element), objectToRemove)
        Assert::assert_true(sds.remove(element) == object_to_remove);
        Assert::assert_true(pq.remove(&object_to_remove));
        check_validity(&pq);
        let new_entry = abs_next_int(&*random);
        sds.push(new_entry.clone());
        assert_null(&pq.insert_with_overflow(new_entry.clone()));
        check_validity(&pq);
        let new_least = pq.top().cloned();
        if let (Some(ll), Some(nl)) = (&last_least, &new_least) {
            if object_to_remove != *ll && *nl != new_entry {
                // If there has been a change of least entry and it wasn't our
                // new addition or the loss of our randomly removed entry we
                // expect the scores to increase
                MatcherAssert::assert_that(
                    &nl.int_value(),
                    &less_than_or_equal_to(new_entry.int_value()),
                );
                MatcherAssert::assert_that(
                    &nl.int_value(),
                    &greater_than_or_equal_to(ll.int_value()),
                );
            }
        }
        last_least = new_least;
    }
    Ok(())
}

/// Java's `Iterator.next()` over the queue: the next element, or
/// `NoSuchElementException` (Rust's `Iterator::next` returns `None`).
fn it_next<'a>(it: &mut impl Iterator<Item = &'a Integer>) -> i32 {
    match it.next() {
        Some(v) => v.int_value(),
        None => Throwable::new("java.util.NoSuchElementException", "").throw(),
    }
}

fn test_iterator_empty() -> Result<()> {
    let queue = integer_queue(3);

    let mut it = queue.iter().peekable();
    Assert::assert_false(it.peek().is_some());
    expect_throws_panic("java.util.NoSuchElementException", || {
        it_next(&mut it);
    });
    Ok(())
}

fn test_iterator_one() -> Result<()> {
    let mut queue = integer_queue(3);

    queue.add(Integer::new(1));
    let mut it = queue.iter().peekable();
    Assert::assert_true(it.peek().is_some());
    assert_equals(&1, &it_next(&mut it));
    Assert::assert_false(it.peek().is_some());
    expect_throws_panic("java.util.NoSuchElementException", || {
        it_next(&mut it);
    });
    Ok(())
}

fn test_iterator_two() -> Result<()> {
    let mut queue = integer_queue(3);

    queue.add(Integer::new(1));
    queue.add(Integer::new(2));
    let mut it = queue.iter().peekable();
    Assert::assert_true(it.peek().is_some());
    assert_equals(&1, &it_next(&mut it));
    Assert::assert_true(it.peek().is_some());
    assert_equals(&2, &it_next(&mut it));
    Assert::assert_false(it.peek().is_some());
    expect_throws_panic("java.util.NoSuchElementException", || {
        it_next(&mut it);
    });
    Ok(())
}

fn test_iterator_random() -> Result<()> {
    let max_size = TestUtil::next_int(&*random(), 1, 20);
    let mut queue = integer_queue(max_size as usize);
    let iters = LuceneTestCase::at_least(100);
    let mut expected: Vec<i32> = Vec::new();
    for _ in 0..iters {
        if queue.size() == 0 || (queue.size() < max_size as usize && random().next_boolean()) {
            let value = random().next_int_bound(10);
            queue.add(Integer::new(value));
            expected.push(value);
        } else {
            // expected.remove(queue.pop()): `remove(Object)`, by `equals`.
            if let Some(popped) = queue.pop() {
                if let Some(p) = expected.iter().position(|&e| e == popped.int_value()) {
                    expected.remove(p);
                }
            }
        }
        let mut actual: Vec<i32> = Vec::new();
        for value in queue.iter() {
            actual.push(value.int_value());
        }
        expected.sort();
        actual.sort();
        assert_equals_msg(None, &expected, &actual);
    }
    Ok(())
}

fn test_max_int_size() -> Result<()> {
    struct AlwaysLess;
    impl PriorityQueueComparator<bool> for AlwaysLess {
        fn less_than(&self, _a: &bool, _b: &bool) -> bool {
            true
        }
    }
    expect_throws("java.lang.IllegalArgumentException", || {
        PriorityQueue::<bool, AlwaysLess>::new(i32::MAX as usize, AlwaysLess)
    });
    Ok(())
}

/// `assertOrderedWhenDrained(pq, referenceDataList)`.
fn assert_ordered_when_drained(pq: &mut IntegerQueue, reference_data_list: &mut [i32]) {
    reference_data_list.sort();
    let mut i = 0;
    while pq.size() > 0 {
        let popped = pq.pop().map(|v| v.int_value());
        assert_equals(&popped, &Some(reference_data_list[i]));
        i += 1;
    }
}
