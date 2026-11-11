//! Port of `lucene/core/src/test/org/apache/lucene/index/TestConcurrentApproximatePriorityQueue.java`.
//!
//! Java keeps a `ReentrantLock` per shard beside the shard's queue
//! (`locks[i]`, `queues[i]`); Rucene's shard is one `Mutex` around the
//! queue, so `pq.locks[i].tryLock()` is `pq.queues()[i].try_lock()`, held
//! as the guard until `unlock()`, and reading `queues[i].isEmpty()` takes
//! that lock for the read.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::index_utilities::{
    ConcurrentApproximatePriorityQueue, MAX_CONCURRENCY, MIN_CONCURRENCY,
};
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, assert_null, class, random, test};
use crate::support::CountDownLatch;

/// `TestConcurrentApproximatePriorityQueue`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestConcurrentApproximatePriorityQueue",
        vec![
            test("testPollFromSameThread", test_poll_from_same_thread),
            test(
                "testPollFromDifferentThread",
                test_poll_from_different_thread,
            ),
            test("testCurrentLockIsBusy", test_current_lock_is_busy),
        ],
    )
}

/// `t.join()`, rethrowing what the thread threw.
fn join(t: &Thread) {
    if let Err(payload) = t.join() {
        std::panic::resume_unwind(payload);
    }
}

/// Port of `TestConcurrentApproximatePriorityQueue.testPollFromSameThread`.
fn test_poll_from_same_thread() -> Result<()> {
    let pq: ConcurrentApproximatePriorityQueue<i32> =
        ConcurrentApproximatePriorityQueue::with_concurrency(TestUtil::next_int(
            &*random(),
            MIN_CONCURRENCY as i32,
            MAX_CONCURRENCY as i32,
        ) as usize);
    pq.add(3, 3);
    pq.add(10, 10);
    pq.add(7, 7);
    assert_equals(&Some(10), &pq.poll(|_| true));
    assert_equals(&Some(7), &pq.poll(|_| true));
    assert_equals(&Some(3), &pq.poll(|_| true));
    assert_null(&pq.poll(|_| true));
    Ok(())
}

/// Port of `TestConcurrentApproximatePriorityQueue.testPollFromDifferentThread`.
fn test_poll_from_different_thread() -> Result<()> {
    let pq: Arc<ConcurrentApproximatePriorityQueue<i32>> =
        Arc::new(ConcurrentApproximatePriorityQueue::with_concurrency(
            TestUtil::next_int(&*random(), MIN_CONCURRENCY as i32, MAX_CONCURRENCY as i32) as usize,
        ));
    pq.add(3, 3);
    pq.add(10, 10);
    pq.add(7, 7);
    let pq2 = Arc::clone(&pq);
    let t = Thread::unnamed(move || {
        assert_equals(&Some(10), &pq2.poll(|_| true));
        assert_equals(&Some(7), &pq2.poll(|_| true));
        assert_equals(&Some(3), &pq2.poll(|_| true));
        assert_null(&pq2.poll(|_| true));
    });
    t.start();
    join(&t);
    Ok(())
}

/// Port of `TestConcurrentApproximatePriorityQueue.testCurrentLockIsBusy`.
fn test_current_lock_is_busy() -> Result<()> {
    // This test needs a concurrency of 2 or more.
    let pq: Arc<ConcurrentApproximatePriorityQueue<i32>> =
        Arc::new(ConcurrentApproximatePriorityQueue::with_concurrency(
            TestUtil::next_int(&*random(), 2, MAX_CONCURRENCY as i32) as usize,
        ));
    pq.add(3, 3);
    let take_lock = Arc::new(CountDownLatch::new(1));
    let release_lock = Arc::new(CountDownLatch::new(1));
    let t = {
        let pq = Arc::clone(&pq);
        let take_lock = Arc::clone(&take_lock);
        let release_lock = Arc::clone(&release_lock);
        Thread::unnamed(move || {
            let mut queue_index: i32 = -1;
            for (i, queue) in pq.queues().iter().enumerate() {
                let is_empty = queue.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
                if !is_empty {
                    queue_index = i as i32;
                    break;
                }
            }
            let guard = pq.queues()[queue_index as usize].try_lock();
            Assert::assert_true(guard.is_ok());
            take_lock.count_down();
            release_lock.await_();
            // `pq.locks[queueIndex].unlock()`
            drop(guard);
        })
    };
    t.start();
    take_lock.await_();
    pq.add(1, 1); // The lock is taken so this needs to go to a different queue
    assert_equals(&Some(1), &pq.poll(|_| true));
    release_lock.count_down();
    assert_equals(&Some(3), &pq.poll(|_| true));
    assert_null(&pq.poll(|_| true));
    Ok(())
}
