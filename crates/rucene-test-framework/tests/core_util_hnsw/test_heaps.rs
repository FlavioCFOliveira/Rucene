//! Ports of `lucene/core/src/test/org/apache/lucene/util/hnsw/TestFloatHeap.java`
//! and `TestBlockingFloatHeap.java`.

use std::sync::{Arc, Condvar, Mutex};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene::util::hnsw::{BlockingFloatHeap, FloatHeap};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestFloatHeap`.
pub fn float_heap_class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBasicOperations", float_test_basic_operations),
        test("testBasicOperations2", float_test_basic_operations2),
        test("testClear", float_test_clear),
    ];
    class("org.apache.lucene.util.hnsw.TestFloatHeap", methods)
}

/// `TestBlockingFloatHeap`.
pub fn blocking_float_heap_class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBasicOperations", blocking_test_basic_operations),
        test("testBasicOperations2", blocking_test_basic_operations2),
        // `@SuppressForbidden(reason = "Thread sleep")`.
        test("testMultipleThreads", blocking_test_multiple_threads),
    ];
    class("org.apache.lucene.util.hnsw.TestBlockingFloatHeap", methods)
}

fn float_test_basic_operations() -> Result<()> {
    let mut heap = FloatHeap::new(3);
    heap.offer(2.0);
    heap.offer(4.0);
    heap.offer(1.0);
    heap.offer(3.0);
    assert_equals(&3, &heap.size());
    assert_equals_delta(2.0, heap.peek() as f64, 0.0);

    assert_equals_delta(2.0, heap.poll() as f64, 0.0);
    assert_equals_delta(3.0, heap.poll() as f64, 0.0);
    assert_equals_delta(4.0, heap.poll() as f64, 0.0);
    assert_equals_delta(0.0, heap.size() as f64, 0.0);
    Ok(())
}

fn float_test_basic_operations2() -> Result<()> {
    let size = LuceneTestCase::at_least(10);
    let mut heap = FloatHeap::new(size as usize);
    let mut sum = 0f64;
    let mut sum2 = 0f64;

    for _ in 0..size {
        let next = random().next_float_bound(100.0);
        sum += next as f64;
        heap.offer(next);
    }

    let mut last = f32::NEG_INFINITY;
    for _ in 0..size {
        let next = heap.poll();
        Assert::assert_true(next >= last);
        last = next;
        sum2 += last as f64;
    }
    assert_equals_delta(sum, sum2, 0.01);
    Ok(())
}

fn float_test_clear() -> Result<()> {
    let mut heap = FloatHeap::new(3);
    heap.offer(20.0);
    heap.offer(40.0);
    heap.offer(30.0);
    assert_equals(&3, &heap.size());
    assert_equals_delta(20.0, heap.peek() as f64, 0.0);

    heap.clear();
    assert_equals_delta(0.0, heap.size() as f64, 0.0);
    assert_equals_delta(20.0, heap.peek() as f64, 0.0);

    heap.offer(15.0);
    heap.offer(35.0);
    assert_equals(&2, &heap.size());
    assert_equals_delta(15.0, heap.peek() as f64, 0.0);

    assert_equals_delta(15.0, heap.poll() as f64, 0.0);
    assert_equals_delta(35.0, heap.poll() as f64, 0.0);
    assert_equals_delta(0.0, heap.size() as f64, 0.0);
    Ok(())
}

fn blocking_test_basic_operations() -> Result<()> {
    let heap = BlockingFloatHeap::new(3);
    heap.offer(2.0);
    heap.offer(4.0);
    heap.offer(1.0);
    heap.offer(3.0);
    assert_equals(&3, &heap.size());
    assert_equals_delta(2.0, heap.peek() as f64, 0.0);

    assert_equals_delta(2.0, heap.poll() as f64, 0.0);
    assert_equals_delta(3.0, heap.poll() as f64, 0.0);
    assert_equals_delta(4.0, heap.poll() as f64, 0.0);
    assert_equals_delta(0.0, heap.size() as f64, 0.0);
    Ok(())
}

fn blocking_test_basic_operations2() -> Result<()> {
    let size = LuceneTestCase::at_least(10);
    let heap = BlockingFloatHeap::new(size as usize);
    let mut sum = 0f64;
    let mut sum2 = 0f64;

    for _ in 0..size {
        let next = random().next_float_bound(100.0);
        sum += next as f64;
        heap.offer(next);
    }

    let mut last = f32::NEG_INFINITY;
    for _ in 0..size {
        let next = heap.poll();
        Assert::assert_true(next >= last);
        last = next;
        sum2 += last as f64;
    }
    assert_equals_delta(sum, sum2, 0.01);
    Ok(())
}

/// `java.util.concurrent.CountDownLatch(1)`.
struct CountDownLatch {
    open: Mutex<bool>,
    cond: Condvar,
}

impl CountDownLatch {
    fn await_latch(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.cond.wait(open).unwrap();
        }
    }

    fn count_down(&self) {
        *self.open.lock().unwrap() = true;
        self.cond.notify_all();
    }
}

fn blocking_test_multiple_threads() -> Result<()> {
    let num_threads = RandomizedTest::random_int_between(3, 20);
    let latch = Arc::new(CountDownLatch {
        open: Mutex::new(false),
        cond: Condvar::new(),
    });
    let global_heap = Arc::new(BlockingFloatHeap::new(1));

    let mut threads = Vec::with_capacity(num_threads as usize);
    for i in 0..num_threads {
        let latch = Arc::clone(&latch);
        let global_heap = Arc::clone(&global_heap);
        let thread = Thread::new(format!("Thread-{i}"), move || {
            latch.await_latch();
            let mut num_iterations = RandomizedTest::random_int_between(10, 100);
            let mut bottom_value = 0f32;
            while num_iterations > 0 {
                num_iterations -= 1;
                bottom_value += RandomizedTest::random_int_between(0, 5) as f32;
                global_heap.offer(bottom_value);
                Thread::sleep(RandomizedTest::random_int_between(0, 50) as i64);
                let global_bottom_value = global_heap.peek();
                Assert::assert_true(global_bottom_value >= bottom_value);
                bottom_value = global_bottom_value;
            }
        });
        thread.start();
        threads.push(thread);
    }

    latch.count_down();
    for t in &threads {
        let _ = t.join();
    }
    Ok(())
}
