//! Port of `com.carrotsearch.randomizedtesting.TestUncaughtExceptionsDuplicated`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Before, BeforeClass};

use crate::support::{
    assert_equals, assert_true, cls, hook, nested, static_hook, test, throw, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

const O: &str = "TestUncaughtExceptionsDuplicated";

/// Starts a thread that throws `RuntimeException(message)` and joins it.
fn throw_in_thread(message: &'static str) {
    let t = Thread::unnamed(move || throw(RUNTIME_EXCEPTION, message));
    t.start();
    let _ = t.join();
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            assume_running_nested();
            throw_in_thread("foobar");
        }))
        .method(test("test", || {}))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook(
            "beforeClass",
            BeforeClass,
            assume_running_nested,
        ))
        .method(test("test1", || throw_in_thread("foobar1")))
        .method(test("test2", || throw_in_thread("foobar2")))
        .method(test("test3", || throw_in_thread("foobar3")))
        .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested3")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(hook("runBeforeTest", Before, || {
            assume_running_nested();
            throw_in_thread("foobar");
        }))
        .method(test("test1", || {}))
        .build()
});

/// Every `foobar[0-9]+` match in `s`.
fn foobars(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("foobar") {
        let tail = &rest[i + 6..];
        let n = tail.chars().take_while(|c| c.is_ascii_digit()).count();
        if n > 0 {
            out.push(format!("foobar{}", &tail[..n]));
        }
        rest = &tail[n..];
    }
    out
}

/// `TestUncaughtExceptionsDuplicated extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testExceptionInBeforeClassFailsTheTest", || {
            let r = check_tests_output(1, 0, 1, 0, &NESTED1);
            assert_true(r.get_failures()[0].trace().contains("foobar"));
        }))
        .method(test("testExceptionWithinTestFailsTheTest", || {
            let r = check_tests_output(3, 0, 3, 0, &NESTED2);
            let mut all: Vec<String> = r
                .get_failures()
                .iter()
                .flat_map(|f| foobars(&f.trace()))
                .collect();
            all.sort();
            assert_equals(
                "[foobar1, foobar2, foobar3]".to_string(),
                format!("[{}]", all.join(", ")),
            );
        }))
        .method(test("testExceptionWithinBeforeFailsTheTest", || {
            let r = check_tests_output(1, 0, 1, 0, &NESTED3);
            assert_true(r.get_failures()[0].trace().contains("foobar"));
        }))
        .build()]
}
