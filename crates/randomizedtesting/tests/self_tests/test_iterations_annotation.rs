//! Port of `com.carrotsearch.randomizedtesting.TestIterationsAnnotation`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};

use crate::support::{assert_equals, cls, static_hook, test, unit, RANDOMIZED_TEST};

/// `static int iterations`.
static ITERATIONS: AtomicI32 = AtomicI32::new(0);

/// `TestIterationsAnnotation extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestIterationsAnnotation")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("nightly", || {
                ITERATIONS.fetch_add(1, Ordering::SeqCst);
            })
            .annotate(Repeat {
                iterations: 10,
                use_constant_seed: false,
            }),
        )
        .method(static_hook("clean", BeforeClass, || {
            ITERATIONS.store(0, Ordering::SeqCst)
        }))
        .method(static_hook("cleanupAfter", AfterClass, || {
            assert_equals(10, ITERATIONS.load(Ordering::SeqCst))
        }))
        .build()]
}
