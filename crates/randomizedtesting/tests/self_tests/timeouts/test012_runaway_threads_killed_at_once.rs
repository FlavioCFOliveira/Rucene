//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test012RunawayThreadsKilledAtOnce`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, Condvar, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;

use super::common::{cls, nested};
use crate::support::{assert_equals, assert_true_msg, test, unit};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "Test012RunawayThreadsKilledAtOnce";

/// `@RunWith(RandomizedRunner.class) NestedClass`.
static NESTED_CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "NestedClass")
        .constructor(unit())
        .method(test("lotsOfStubbornThreads", || {
            assume_running_nested();
            // CountDownLatch(50).
            let latch = Arc::new((Mutex::new(50usize), Condvar::new()));
            for i in 0..50 {
                let l = Arc::clone(&latch);
                Thread::new(format!("stubborn-{i}"), move || {
                    {
                        let (m, c) = &*l;
                        *m.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
                        c.notify_all();
                    }
                    // An interrupt ends the sleep, and the thread.
                    let _ = Thread::try_sleep(20000);
                })
                .start();
            }
            // Wait for all threads to be really started.
            let (m, c) = &*latch;
            let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
            while *g > 0 {
                g = c.wait(g).unwrap_or_else(|e| e.into_inner());
            }
        }))
        .build()
});

/// `@RunWith(RandomizedRunner.class) Test012RunawayThreadsKilledAtOnce extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        // Java runs this class under RandomizedRunner (@RunWith): the default
        // leak scope, which WithNestedTestClass's declaration replaces.
        .annotate(randomizedtesting::annotations::ThreadLeakScope {
            value: randomizedtesting::annotations::Scope::Test,
        })
        .constructor(unit())
        .method(test("testLotsOfStubbornThreads", || {
            let start = system::nano_time();
            let result = run_tests(std::slice::from_ref(&*NESTED_CLASS));
            let end = system::nano_time();
            assert_equals(1, result.get_failure_count());
            let msec = (end - start) / 1_000_000;
            assert_true_msg(&format!("{msec} msec?"), msec < 1000 * 10);
        }))
        .build()]
}
