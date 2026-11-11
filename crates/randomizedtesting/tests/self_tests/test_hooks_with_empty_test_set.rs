//! Port of `com.carrotsearch.randomizedtesting.TestHooksWithEmptyTestSet`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): hooks do not run when
//! there is no test case to run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};

use crate::support::{assert_false, cls, nested, static_hook, test, unit};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

static BEFORE_CLASS_EXECUTED: AtomicBool = AtomicBool::new(false);
static AFTER_CLASS_EXECUTED: AtomicBool = AtomicBool::new(false);

/// `@RunWith(RandomizedRunner.class) Nested`.
static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestHooksWithEmptyTestSet", "Nested")
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            assume_running_nested();
            BEFORE_CLASS_EXECUTED.store(true, Ordering::SeqCst);
        }))
        .method(static_hook("afterClass", AfterClass, || {
            AFTER_CLASS_EXECUTED.store(true, Ordering::SeqCst);
        }))
        .build()
});

/// `TestHooksWithEmptyTestSet extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestHooksWithEmptyTestSet")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testSameMethodRandomnessWithFixedRunner", || {
            BEFORE_CLASS_EXECUTED.store(false, Ordering::SeqCst);
            AFTER_CLASS_EXECUTED.store(false, Ordering::SeqCst);
            check_tests_output(0, 0, 0, 0, &NESTED);
            assert_false(BEFORE_CLASS_EXECUTED.load(Ordering::SeqCst));
            assert_false(AFTER_CLASS_EXECUTED.load(Ordering::SeqCst));
        }))
        .build()]
}
