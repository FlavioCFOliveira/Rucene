//! Port of `com.carrotsearch.randomizedtesting.TestThreadNameContainsTestName`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::annotations::Timeout;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;

use crate::support::{assert_true_msg, cls, test, unit, RANDOMIZED_TEST};

fn check(marker: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || {
        let t_name = Thread::current().name();
        assert_true_msg(&t_name, t_name.contains(marker));
    }
}

/// `TestThreadNameContainsTestName extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestThreadNameContainsTestName")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testMarkerABC", check("testMarkerABC")))
        .method(test("testMarkerXYZ", check("testMarkerXYZ")).annotate(Timeout { millis: 0 }))
        .method(test("testMarkerKJI", check("testMarkerKJI")).annotate(Timeout { millis: 1000 }))
        .build()]
}
