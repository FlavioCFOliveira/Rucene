//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocumentsWriterStallControl.java`.
//!
//! Tests for `DocumentsWriterStallControl`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::{State, Thread};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::documents_writer::DocumentsWriterStallControl;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};
use crate::support::CountDownLatch;

/// `TestDocumentsWriterStallControl`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocumentsWriterStallControl",
        vec![
            test("testSimpleStall", test_simple_stall),
            test("testRandom", test_random),
            test("testAcquireReleaseRace", test_acquire_release_race),
        ],
    )
}

/// Port of `TestDocumentsWriterStallControl.testSimpleStall`.
fn test_simple_stall() -> Result<()> {
    let ctrl = Arc::new(DocumentsWriterStallControl::new());

    ctrl.update_stalled(false);
    let threads = wait_threads(LuceneTestCase::at_least(1), &ctrl);
    start(&threads);
    Assert::assert_false(ctrl.has_blocked());
    Assert::assert_false(ctrl.any_stalled_threads());
    join(&threads);

    // now stall threads and wake them up again
    ctrl.update_stalled(true);
    let threads = wait_threads(LuceneTestCase::at_least(1), &ctrl);
    start(&threads);
    await_state(State::TimedWaiting, &threads);
    Assert::assert_true(ctrl.has_blocked());
    Assert::assert_true(ctrl.any_stalled_threads());
    ctrl.update_stalled(false);
    Assert::assert_false(ctrl.any_stalled_threads());
    join(&threads);
    Ok(())
}

/// Port of `TestDocumentsWriterStallControl.testRandom`.
fn test_random() -> Result<()> {
    let ctrl = Arc::new(DocumentsWriterStallControl::new());
    ctrl.update_stalled(false);

    let num = LuceneTestCase::at_least(3);
    let mut stall_threads = Vec::with_capacity(num as usize);
    for _ in 0..num {
        let stall_probability = 1 + random().next_int_bound(10);
        let ctrl = Arc::clone(&ctrl);
        stall_threads.push(Thread::unnamed(move || {
            let iters = LuceneTestCase::at_least(100);
            for _ in 0..iters {
                ctrl.update_stalled(random().next_int_bound(stall_probability) == 0);
                if random().next_int_bound(5) == 0 {
                    // thread 0 only updates
                    ctrl.wait_if_stalled();
                }
            }
        }));
    }
    start(&stall_threads);
    // use a 100 maximum iterations check to make sure we not hang forever. join will fail in
    // that case
    let mut iterations = 0;
    loop {
        iterations += 1;
        if !(iterations < 100 && !terminated(&stall_threads)) {
            break;
        }
        ctrl.update_stalled(false);
        if random().next_boolean() {
            std::thread::yield_now();
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    join(&stall_threads);
    Ok(())
}

/// Port of `TestDocumentsWriterStallControl.testAcquireReleaseRace`.
fn test_acquire_release_race() -> Result<()> {
    let ctrl = Arc::new(DocumentsWriterStallControl::new());
    ctrl.update_stalled(false);
    let stop = Arc::new(AtomicBool::new(false));
    let check_point = Arc::new(AtomicBool::new(true));

    let num_stallers = LuceneTestCase::at_least(1);
    let num_releasers = LuceneTestCase::at_least(1);
    let num_waiters = LuceneTestCase::at_least(1);
    let sync = Arc::new(Synchronizer::new(
        num_stallers + num_releasers,
        num_stallers + num_releasers + num_waiters,
    ));
    let mut threads = Vec::new();
    let exceptions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    for _ in 0..num_releasers {
        threads.push(updater(
            &stop,
            &check_point,
            &ctrl,
            &sync,
            true,
            &exceptions,
        ));
    }
    for _ in num_releasers..num_releasers + num_stallers {
        threads.push(updater(
            &stop,
            &check_point,
            &ctrl,
            &sync,
            false,
            &exceptions,
        ));
    }
    for _ in num_releasers + num_stallers..num_releasers + num_stallers + num_waiters {
        threads.push(waiter(&stop, &check_point, &ctrl, &sync, &exceptions));
    }

    start(&threads);
    let iters = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(10000)
    } else {
        LuceneTestCase::at_least(1000)
    };
    let check_point_probability: f32 = if *TEST_NIGHTLY { 0.5 } else { 0.1 };
    for _ in 0..iters {
        if check_point.load(Ordering::SeqCst) {
            Assert::assert_true_msg(
                "timed out waiting for update threads - deadlock?",
                sync.update_join().await_millis(10_000),
            );
            let exceptions = exceptions.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if !exceptions.is_empty() {
                for throwable in &exceptions {
                    eprintln!("{throwable}");
                }
                Assert::fail("got exceptions in threads");
            }

            if ctrl.has_blocked() && ctrl.is_healthy() {
                assert_state(num_releasers, num_stallers, num_waiters, &threads, &ctrl);
            }

            check_point.store(false, Ordering::SeqCst);
            sync.waiter().count_down();
            sync.left_checkpoint().await_();
        }
        Assert::assert_false(check_point.load(Ordering::SeqCst));
        assert_equals(&0, &sync.waiter().get_count());
        if check_point_probability >= random().next_float() {
            sync.reset(
                num_stallers + num_releasers,
                num_stallers + num_releasers + num_waiters,
            );
            check_point.store(true, Ordering::SeqCst);
        }
    }
    if !check_point.load(Ordering::SeqCst) {
        sync.reset(
            num_stallers + num_releasers,
            num_stallers + num_releasers + num_waiters,
        );
        check_point.store(true, Ordering::SeqCst);
    }

    Assert::assert_true(sync.update_join().await_millis(10_000));
    assert_state(num_releasers, num_stallers, num_waiters, &threads, &ctrl);
    check_point.store(false, Ordering::SeqCst);
    stop.store(true, Ordering::SeqCst);
    sync.waiter().count_down();
    sync.left_checkpoint().await_();

    for (i, thread) in threads.iter().enumerate() {
        ctrl.update_stalled(false);
        thread.join_millis(2000);
        // `threads[i] instanceof Waiter`: the waiters are the last
        // `numWaiters` threads.
        let is_waiter = i as i32 >= num_releasers + num_stallers;
        if thread.is_alive() && is_waiter && thread.state() == State::Waiting {
            Assert::fail(&format!(
                "waiter is not released - anyThreadsStalled: {}",
                ctrl.any_stalled_threads()
            ));
        }
    }
    Ok(())
}

/// `private void assertState(int numReleasers, int numStallers, int
/// numWaiters, Thread[] threads, DocumentsWriterStallControl ctrl)`.
fn assert_state(
    num_releasers: i32,
    num_stallers: i32,
    num_waiters: i32,
    threads: &[Thread],
    ctrl: &DocumentsWriterStallControl,
) {
    let mut millis_to_sleep = 100;
    while ctrl.has_blocked() && ctrl.is_healthy() {
        for n in num_releasers + num_stallers..num_releasers + num_stallers + num_waiters {
            let queued = threads[n as usize]
                .os_thread_id()
                .is_some_and(|id| ctrl.is_thread_queued(id));
            if queued {
                if millis_to_sleep < 60000 {
                    std::thread::sleep(std::time::Duration::from_millis(millis_to_sleep));
                    millis_to_sleep *= 2;
                    break;
                } else {
                    Assert::fail(
                        "control claims no stalled threads but waiter seems to be blocked ",
                    );
                }
            }
        }
    }
}

/// `e.printStackTrace(); exceptions.add(e);` for what a thread's body
/// threw.
fn record_failure(exceptions: &Arc<Mutex<Vec<String>>>, payload: Box<dyn std::any::Any + Send>) {
    let t = randomizedtesting::jdk::exception::Throwable::from_panic(payload);
    eprintln!("{t}");
    exceptions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(t.to_string());
}

/// `public static class Waiter extends Thread` (named `waiter`).
fn waiter(
    stop: &Arc<AtomicBool>,
    check_point: &Arc<AtomicBool>,
    ctrl: &Arc<DocumentsWriterStallControl>,
    sync: &Arc<Synchronizer>,
    exceptions: &Arc<Mutex<Vec<String>>>,
) -> Thread {
    let (stop, check_point, ctrl, sync, exceptions) = (
        Arc::clone(stop),
        Arc::clone(check_point),
        Arc::clone(ctrl),
        Arc::clone(sync),
        Arc::clone(exceptions),
    );
    Thread::new("waiter", move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            while !stop.load(Ordering::SeqCst) {
                ctrl.wait_if_stalled();
                if check_point.load(Ordering::SeqCst) {
                    Assert::assert_true(sync.await_());
                }
            }
        }));
        if let Err(payload) = outcome {
            record_failure(&exceptions, payload);
        }
    })
}

/// `public static class Updater extends Thread` (named `updater`).
fn updater(
    stop: &Arc<AtomicBool>,
    check_point: &Arc<AtomicBool>,
    ctrl: &Arc<DocumentsWriterStallControl>,
    sync: &Arc<Synchronizer>,
    release: bool,
    exceptions: &Arc<Mutex<Vec<String>>>,
) -> Thread {
    let (stop, check_point, ctrl, sync, exceptions) = (
        Arc::clone(stop),
        Arc::clone(check_point),
        Arc::clone(ctrl),
        Arc::clone(sync),
        Arc::clone(exceptions),
    );
    Thread::new("updater", move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            while !stop.load(Ordering::SeqCst) {
                let internal_iters = if release && random().next_boolean() {
                    LuceneTestCase::at_least(5)
                } else {
                    1
                };
                for _ in 0..internal_iters {
                    ctrl.update_stalled(random().next_boolean());
                }
                if check_point.load(Ordering::SeqCst) {
                    sync.update_join().count_down();
                    Assert::assert_true(sync.await_());
                    sync.left_checkpoint().count_down();
                }
                if random().next_boolean() {
                    std::thread::yield_now();
                }
            }
        }));
        if let Err(payload) = outcome {
            record_failure(&exceptions, payload);
        }
        sync.update_join().count_down();
    })
}

/// `public static boolean terminated(Thread[] threads)`.
fn terminated(threads: &[Thread]) -> bool {
    threads.iter().all(|t| t.state() == State::Terminated)
}

/// `public static void start(Thread[] tostart)`.
fn start(tostart: &[Thread]) {
    for thread in tostart {
        thread.start();
    }
    std::thread::sleep(std::time::Duration::from_millis(1)); // let them start
}

/// `public static void join(Thread[] toJoin)`.
fn join(to_join: &[Thread]) {
    for thread in to_join {
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
    }
}

/// `public static Thread[] waitThreads(int num, final
/// DocumentsWriterStallControl ctrl)`.
fn wait_threads(num: i32, ctrl: &Arc<DocumentsWriterStallControl>) -> Vec<Thread> {
    (0..num)
        .map(|_| {
            let ctrl = Arc::clone(ctrl);
            Thread::unnamed(move || ctrl.wait_if_stalled())
        })
        .collect()
}

/// Waits for all incoming threads to be in wait() methods.
///
/// `public static void awaitState(Thread.State state, Thread... threads)`.
fn await_state(state: State, threads: &[Thread]) {
    loop {
        let mut done = true;
        for thread in threads {
            if thread.state() != state {
                done = false;
                break;
            }
        }
        if done {
            return;
        }
        if random().next_boolean() {
            std::thread::yield_now();
        } else {
            Thread::sleep(1);
        }
    }
}

/// `private static final class Synchronizer`: its `volatile` latches.
struct Synchronizer {
    latches: Mutex<(
        Arc<CountDownLatch>,
        Arc<CountDownLatch>,
        Arc<CountDownLatch>,
    )>,
}

impl Synchronizer {
    fn new(num_updater: i32, num_threads: i32) -> Self {
        let s = Synchronizer {
            latches: Mutex::new((
                Arc::new(CountDownLatch::new(1)),
                Arc::new(CountDownLatch::new(0)),
                Arc::new(CountDownLatch::new(0)),
            )),
        };
        s.reset(num_updater, num_threads);
        s
    }

    fn reset(&self, num_updaters: i32, _num_threads: i32) {
        *self.latches.lock().unwrap_or_else(|e| e.into_inner()) = (
            Arc::new(CountDownLatch::new(1)),
            Arc::new(CountDownLatch::new(i64::from(num_updaters))),
            Arc::new(CountDownLatch::new(i64::from(num_updaters))),
        );
    }

    fn waiter(&self) -> Arc<CountDownLatch> {
        Arc::clone(&self.latches.lock().unwrap_or_else(|e| e.into_inner()).0)
    }

    fn update_join(&self) -> Arc<CountDownLatch> {
        Arc::clone(&self.latches.lock().unwrap_or_else(|e| e.into_inner()).1)
    }

    fn left_checkpoint(&self) -> Arc<CountDownLatch> {
        Arc::clone(&self.latches.lock().unwrap_or_else(|e| e.into_inner()).2)
    }

    /// `waiter.await(10, TimeUnit.SECONDS)`.
    fn await_(&self) -> bool {
        self.waiter().await_millis(10_000)
    }
}
