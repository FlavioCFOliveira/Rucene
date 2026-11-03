//! Port of `com.carrotsearch.randomizedtesting.TestChildTestGroupThreads`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): contexts are
//! reachable from a sub-thread group.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::{Thread, ThreadGroup};
use randomizedtesting::RandomizedContext;

use crate::support::{cls, test, unit, RANDOMIZED_TEST};

/// `volatile int guard` (an instance field; one instance runs the test).
static GUARD: AtomicI32 = AtomicI32::new(0);

/// `TestChildTestGroupThreads extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestChildTestGroupThreads")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testSubgroup", || {
            let tgroup = ThreadGroup::new_in_current("child group");
            let t = Thread::with_group(&tgroup, "child thread", || {
                let rnd = RandomizedContext::current().get_random();
                GUARD.fetch_add(rnd.next_int(), Ordering::Relaxed);
            });
            t.start();
            let _ = t.join();
        }))
        .build()]
}
