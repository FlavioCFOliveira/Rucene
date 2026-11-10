//! Port of `lucene/core/src/test/org/apache/lucene/index/TestLockableConcurrentApproximatePriorityQueue.java`.
//!
//! * `WeightedLock implements Lock` over a `ReentrantLock` implements
//!   [`TryLockable`], the two `Lock` operations the queue uses; the queue
//!   holds it through an `Arc`, as Java holds a reference.
//! * `lock.hashCode()` is `Object.hashCode()`, the identity hash: here the
//!   address of the shared lock, narrowed to an `int`.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::ThreadId;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::error::Result;
use rucene::index::{LockableConcurrentApproximatePriorityQueue, TryLockable};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_not_null, class, random, test};
use crate::support::CountDownLatch;

/// `TestLockableConcurrentApproximatePriorityQueue`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestLockableConcurrentApproximatePriorityQueue",
        vec![test(
            "testNeverReturnNullOnNonEmptyQueue",
            test_never_return_null_on_non_empty_queue,
        )],
    )
}

/// `java.util.concurrent.locks.ReentrantLock`: `lock`, `tryLock` and
/// `unlock`, held by a thread until it releases every hold.
#[derive(Default)]
struct ReentrantLock {
    owner: Mutex<(Option<ThreadId>, usize)>,
    released: Condvar,
}

impl ReentrantLock {
    fn lock(&self) {
        let me = std::thread::current().id();
        let mut owner = self.owner.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            match owner.0 {
                None => {
                    *owner = (Some(me), 1);
                    return;
                }
                Some(holder) if holder == me => {
                    owner.1 += 1;
                    return;
                }
                Some(_) => {
                    owner = self.released.wait(owner).unwrap_or_else(|p| p.into_inner());
                }
            }
        }
    }

    fn try_lock(&self) -> bool {
        let me = std::thread::current().id();
        let mut owner = self.owner.lock().unwrap_or_else(|p| p.into_inner());
        match owner.0 {
            None => {
                *owner = (Some(me), 1);
                true
            }
            Some(holder) if holder == me => {
                owner.1 += 1;
                true
            }
            Some(_) => false,
        }
    }

    fn unlock(&self) {
        let me = std::thread::current().id();
        let mut owner = self.owner.lock().unwrap_or_else(|p| p.into_inner());
        if owner.0 != Some(me) {
            Throwable::new("java.lang.IllegalMonitorStateException", "").throw();
        }
        owner.1 -= 1;
        if owner.1 == 0 {
            owner.0 = None;
            self.released.notify_one();
        }
    }
}

/// `private static class WeightedLock implements Lock`.
#[derive(Default)]
struct WeightedLockInner {
    lock: ReentrantLock,
    weight: AtomicI64,
}

/// A reference to a `WeightedLock`.
#[derive(Clone, Default)]
struct WeightedLock(Arc<WeightedLockInner>);

impl WeightedLock {
    /// `lock()`.
    fn lock(&self) {
        self.0.lock.lock();
    }

    /// `Object.hashCode()`: the identity hash.
    fn hash_code(&self) -> i32 {
        Arc::as_ptr(&self.0) as usize as i32
    }
}

impl TryLockable for WeightedLock {
    /// `tryLock()`.
    fn try_lock_entry(&self) -> bool {
        self.0.lock.try_lock()
    }

    /// `unlock()`.
    fn unlock_entry(&self) {
        self.0.lock.unlock();
    }
}

/// Port of `TestLockableConcurrentApproximatePriorityQueue.testNeverReturnNullOnNonEmptyQueue`.
fn test_never_return_null_on_non_empty_queue() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let concurrency = TestUtil::next_int(&*random(), 1, 16);
        let queue: Arc<LockableConcurrentApproximatePriorityQueue<WeightedLock>> = Arc::new(
            LockableConcurrentApproximatePriorityQueue::with_concurrency(concurrency as usize),
        );
        let num_threads = TestUtil::next_int(&*random(), 2, 16);
        let mut threads = Vec::with_capacity(num_threads as usize);
        let starting_gun = Arc::new(CountDownLatch::new(1));
        for _ in 0..num_threads {
            let queue = Arc::clone(&queue);
            let starting_gun = Arc::clone(&starting_gun);
            threads.push(Thread::unnamed(move || {
                starting_gun.await_();
                let mut lock = WeightedLock::default();
                lock.lock();
                // Simulate a DWPT whose RAM usage increases
                let weight = lock.0.weight.fetch_add(1, Ordering::SeqCst) + 1;
                queue.add_and_unlock(lock.clone(), weight);
                for _ in 0..10_000 {
                    let polled = queue.lock_and_poll();
                    assert_not_null(&polled);
                    lock = polled.expect("checked above");
                    let hash = i64::from(lock.hash_code());
                    queue.add_and_unlock(lock.clone(), hash);
                }
            }));
        }
        for thread in &threads {
            thread.start();
        }
        starting_gun.count_down();
        for thread in &threads {
            let _ = thread.join();
        }
    }
    Ok(())
}
