//! Port of `com.carrotsearch.randomizedtesting.TestFormattingRandomSeeds`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::{default_supplier, RandomizedContext, Randomness, SeedUtils};

use crate::support::{assert_equals, cls, test, unit};

/// `check(seed)`.
fn check(seed: i64) {
    let r = Randomness::new(seed, default_supplier(), Vec::new());
    let as_string = SeedUtils::format_seed_chain(&[&r]);
    assert_equals(seed, SeedUtils::parse_seed_chain(&as_string)[0]);
}

/// `@RunWith(RandomizedRunner.class) TestFormattingRandomSeeds`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestFormattingRandomSeeds")
        .constructor(unit())
        .method(test("minusOne", || check(-1)))
        .method(test("zero", || check(0)))
        .method(test("maxLong", || check(i64::MAX)))
        .method(
            test("noise", || {
                check(RandomizedContext::current().get_random().next_long())
            })
            .annotate(Repeat {
                iterations: 1000,
                use_constant_seed: false,
            }),
        )
        .build()]
}
