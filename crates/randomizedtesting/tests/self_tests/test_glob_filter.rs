//! Port of `com.carrotsearch.randomizedtesting.TestGlobFilter`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::MethodGlobFilter;

use crate::support::{assert_true, cls, test, unit};

/// `TestGlobFilter`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestGlobFilter")
        .constructor(unit())
        .method(test("testBrackets", || {
            let gf = MethodGlobFilter::new("ab(*)");
            assert_true(gf.glob_matches("ab()"));
            assert_true(gf.glob_matches("ab(foo)"));
            assert_true(gf.glob_matches("ab(bar=xxx)"));
            let gf = MethodGlobFilter::new("test {yaml=resthandler/10_Foo/Bar (Hello)}");
            assert_true(gf.glob_matches("test {yaml=resthandler/10_Foo/Bar (Hello)}"));
        }))
        .build()]
}
