//! Port of `com.carrotsearch.randomizedtesting.TestOverlappingMethodProviders`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{ClassRef, TestMethodProviders};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::JUnitCore;
use randomizedtesting::{JUnit3MethodProvider, JUnit4MethodProvider, TestMethodProvider};

use crate::support::{assert_equals, cls, nested, test, unit};

static BASE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestOverlappingMethodProviders", "Base")
        .annotate(TestMethodProviders {
            value: vec![
                ClassRef::new(
                    "com.carrotsearch.randomizedtesting.JUnit4MethodProvider",
                    || Arc::new(JUnit4MethodProvider) as Arc<dyn TestMethodProvider>,
                ),
                ClassRef::new(
                    "com.carrotsearch.randomizedtesting.JUnit3MethodProvider",
                    || Arc::new(JUnit3MethodProvider) as Arc<dyn TestMethodProvider>,
                ),
            ],
        })
        .constructor(unit())
        .method(test("testMe", || {}))
        .build()
});

/// `TestOverlappingMethodProviders`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestOverlappingMethodProviders")
        .constructor(unit())
        .method(test("testSingleMethod", || {
            let r = JUnitCore::new().run_class(&BASE);
            assert_equals(0, r.failure_count());
            assert_equals(1, r.run_count());
        }))
        .build()]
}
