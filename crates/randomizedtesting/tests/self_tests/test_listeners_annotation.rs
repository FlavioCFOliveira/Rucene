//! Port of `com.carrotsearch.randomizedtesting.TestListenersAnnotation`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): listeners declared on
//! a suite.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{ClassRef, Listeners};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{
    Assert, Assume, Description, Failure, Ignore, JUnitResult, RunListener,
};

use crate::support::{assert_true, cls, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "TestListenersAnnotation";

/// `public static List<String> buffer`.
static BUFFER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: String) {
    BUFFER.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

fn method(d: &Description) -> String {
    d.method_name().unwrap_or_else(|| "null".into())
}

/// `NoopListener extends RunListener`.
struct NoopListener;
impl RunListener for NoopListener {}

/// `BufferAppendListener extends RunListener`.
struct BufferAppendListener;

impl RunListener for BufferAppendListener {
    fn test_run_started(&self, d: &Arc<Description>) {
        add(format!("run started: {}", method(d)));
    }
    fn test_started(&self, d: &Arc<Description>) {
        add(format!("test started: {}", method(d)));
    }
    fn test_finished(&self, d: &Arc<Description>) {
        add(format!("test finished: {}", method(d)));
    }
    fn test_assumption_failure(&self, f: &Failure) {
        add(format!("assumption failed: {}", method(f.description())));
    }
    fn test_ignored(&self, d: &Arc<Description>) {
        add(format!("test ignored: {}", method(d)));
    }
    fn test_failure(&self, f: &Failure) {
        add(format!("failure: {}", method(f.description())));
    }
    fn test_run_finished(&self, r: &JUnitResult) {
        add(format!("run finished: {}", r.run_count()));
    }
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .annotate(Listeners {
            value: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.TestListenersAnnotation$BufferAppendListener",
                || Arc::new(BufferAppendListener) as Arc<dyn RunListener>,
            )],
        })
        .constructor(unit())
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&NESTED1)
        .annotate(Listeners {
            value: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.TestListenersAnnotation$NoopListener",
                || Arc::new(NoopListener) as Arc<dyn RunListener>,
            )],
        })
        .constructor(unit())
        .method(test("ignored", || {}).annotate(Ignore::new()))
        .method(test("passing", || {}))
        .method(test("failing", || {
            assume_running_nested();
            Assert::fail_without_message();
        }))
        .method(test("assumptionFailing", || {
            assume_running_nested();
            Assume::assume_true(false);
        }))
        .build()
});

/// `TestListenersAnnotation extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkListeners", || {
            run_tests(std::slice::from_ref(&*NESTED2));
            // Perhaps this is overly simple, but we just want to know that it executed.
            assert_true(!BUFFER.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
        }))
        .build()]
}
