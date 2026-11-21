//! Port of `com.carrotsearch.randomizedtesting.TestSeedFixing`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): seed fixing for static
//! fixtures and methods with annotations.

use std::sync::Arc;

use randomizedtesting::annotations::Seed;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::BeforeClass;

use crate::support::{assert_equals, assert_that_str, cls, current_seed, static_hook, test, unit};

/// `@RunWith(RandomizedRunner.class) @Seed("deadbeef") TestSeedFixing`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestSeedFixing")
        .annotate(Seed::new("deadbeef"))
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            assert_equals(0xdeadbeef_i64, current_seed());
        }))
        .method(
            test("dummy", || {
                assert_that_str(format!("{:x}", current_seed())).is_equal_to("cafebabe");
            })
            .annotate(Seed::new("cafebabe")),
        )
        .build()]
}
