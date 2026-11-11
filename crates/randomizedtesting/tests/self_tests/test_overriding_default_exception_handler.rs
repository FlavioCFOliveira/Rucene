//! Port of `com.carrotsearch.randomizedtesting.TestOverridingDefaultExceptionHandler`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::{self, Throwable, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::{Thread, ThreadGroup, UncaughtExceptionHandler};
use randomizedtesting::junit::{AfterClass, BeforeClass};

use crate::support::{
    assert_that_list, cls, nested, static_hook, test, throw, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

/// `static List<String> throwableMessages`.
static THROWABLE_MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestOverridingDefaultExceptionHandler.TestException extends RuntimeException`.
const TEST_EXCEPTION: &str =
    "com.carrotsearch.randomizedtesting.TestOverridingDefaultExceptionHandler$TestException";

/// `Nested.defaultHandler`.
static DEFAULT_HANDLER: Mutex<Option<Arc<dyn UncaughtExceptionHandler>>> = Mutex::new(None);

/// The handler `Nested.beforeClass` installs.
struct Recording;

impl UncaughtExceptionHandler for Recording {
    fn uncaught_exception(&self, t: &Thread, e: Throwable) {
        if e.is_instance(TEST_EXCEPTION) {
            THROWABLE_MESSAGES
                .lock()
                .unwrap_or_else(|x| x.into_inner())
                .push(e.message().unwrap_or("null").to_string());
        } else {
            let h = DEFAULT_HANDLER
                .lock()
                .unwrap_or_else(|x| x.into_inner())
                .clone();
            match h {
                Some(h) => h.uncaught_exception(t, e),
                None => exception::throw_without_message(exception::NULL_POINTER_EXCEPTION),
            }
        }
    }
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    exception::register_class(TEST_EXCEPTION, RUNTIME_EXCEPTION);
    nested("TestOverridingDefaultExceptionHandler", "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            *DEFAULT_HANDLER.lock().unwrap_or_else(|e| e.into_inner()) =
                Thread::get_default_uncaught_exception_handler();
            Thread::set_default_uncaught_exception_handler(Some(Arc::new(Recording)));
        }))
        .method(static_hook("afterClass", AfterClass, || {
            let h = DEFAULT_HANDLER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            Thread::set_default_uncaught_exception_handler(h);
        }))
        .method(test("exceptionFromChildThread", || {
            let t = Thread::unnamed(|| throw(TEST_EXCEPTION, "exceptionFromChildThread"));
            t.start();
            let _ = t.join();
        }))
        .method(test("exceptionFromSubGroup", || {
            let subgroup = ThreadGroup::new_in_current("subgroup");
            let t =
                Thread::unnamed_in(&subgroup, || throw(TEST_EXCEPTION, "exceptionFromSubGroup"));
            t.start();
            let _ = t.join();
        }))
        .build()
});

/// `TestOverridingDefaultExceptionHandler extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestOverridingDefaultExceptionHandler")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testHandlerPropagation", || {
            run_tests(std::slice::from_ref(&*NESTED));
            let m = THROWABLE_MESSAGES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            assert_that_list(m.clone()).contains(&["exceptionFromChildThread"]);
            assert_that_list(m).contains(&["exceptionFromSubGroup"]);
        }))
        .build()]
}
