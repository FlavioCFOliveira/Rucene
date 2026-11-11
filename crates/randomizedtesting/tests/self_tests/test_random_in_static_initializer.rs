//! Port of `com.carrotsearch.randomizedtesting.TestRandomInStaticInitializer`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): out-of-scope use of
//! the context in a static initializer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::ILLEGAL_STATE_EXCEPTION;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::RandomizedContext;

use crate::support::{assert_true, cls, test, try_catch, unit, RANDOMIZED_TEST};

/// `static boolean wasOutOfScope`.
static WAS_OUT_OF_SCOPE: AtomicBool = AtomicBool::new(false);

/// `TestRandomInStaticInitializer extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRandomInStaticInitializer")
        .extends(&RANDOMIZED_TEST)
        .static_init(|| match try_catch(RandomizedContext::current) {
            Err(e) if e.is_instance(ILLEGAL_STATE_EXCEPTION) => {
                WAS_OUT_OF_SCOPE.store(true, Ordering::SeqCst)
            }
            Err(e) => e.throw(),
            Ok(_) => {}
        })
        .constructor(unit())
        .method(test("testStaticInitializerOutOfScope", || {
            assert_true(WAS_OUT_OF_SCOPE.load(Ordering::SeqCst));
        }))
        .build()]
}
