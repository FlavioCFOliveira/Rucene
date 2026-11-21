//! Port of `com.carrotsearch.randomizedtesting.TestManyThreadsOOM`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): recreating many
//! threads, each holding a large blob and touching the context, does not
//! exhaust memory.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::RandomizedContext;

use crate::support::{cls, test, unit, RANDOMIZED_TEST};

/// `volatile int guard`.
static GUARD: AtomicI32 = AtomicI32::new(0);

/// `TestManyThreadsOOM extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestManyThreadsOOM")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("testSpawnManyThreads", || {
                for _ in 0..500 {
                    // `final byte [] hold = new byte [1024 * 1024 * 10];`
                    let hold = vec![0u8; 1024 * 1024 * 10];
                    let t = Thread::unnamed(move || {
                        let rnd = RandomizedContext::current().get_random();
                        GUARD.fetch_add(
                            rnd.next_int().wrapping_add(hold.len() as i32),
                            Ordering::Relaxed,
                        );
                    });
                    t.start();
                    let _ = t.join();
                }
            })
            .annotate(Repeat {
                iterations: 10,
                use_constant_seed: false,
            }),
        )
        .build()]
}
