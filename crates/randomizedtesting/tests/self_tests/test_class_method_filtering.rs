//! Port of `com.carrotsearch.randomizedtesting.TestClassMethodFiltering`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): global class and
//! method filters.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{After, Before, BeforeClass};
use randomizedtesting::SysGlobals;

use crate::support::{assert_equals, assert_true, cls, hook, nested, static_hook, test, unit};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "TestClassMethodFiltering";

/// `static List<String> methods`.
static METHODS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: &'static str) {
    METHODS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(s.into());
}

fn nested_method(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || {
        assume_running_nested();
        add(s);
    }
}

/// `@RunWith(RandomizedRunner.class) Nested1`.
static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            add("beforeClass1")
        }))
        .method(test("method1", nested_method("method1")))
        .method(test("method2", nested_method("method2")))
        .build()
});

/// `@RunWith(RandomizedRunner.class) Nested2`.
static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            add("beforeClass2")
        }))
        .method(test("method1", nested_method("method1")))
        .build()
});

fn run() -> Vec<String> {
    run_tests(&[NESTED1.clone(), NESTED2.clone()]);
    METHODS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn is(v: &[String], expected: &[&str]) -> bool {
    v.len() == expected.len() && v.iter().zip(expected).all(|(a, b)| a == b)
}

fn list(v: &[&str]) -> String {
    format!("[{}]", v.join(", "))
}

fn cleanup_after() {
    system::clear_property(&SysGlobals::sysprop_testclass());
    system::clear_property(&SysGlobals::sysprop_testmethod());
    METHODS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// `TestClassMethodFiltering extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        // Class filter (all methods).
        .method(test("testClassFilter", || {
            system::set_property(&SysGlobals::sysprop_testclass(), NESTED1.name());
            let m = run();
            assert_true(
                is(&m, &["beforeClass1", "method1", "method2"])
                    || is(&m, &["beforeClass1", "method2", "method1"]),
            );
        }))
        // Class and method filter (single method).
        .method(test("testClassMethodFilter", || {
            system::set_property(&SysGlobals::sysprop_testclass(), NESTED1.name());
            system::set_property(&SysGlobals::sysprop_testmethod(), "method2");
            let m = run();
            assert_equals(
                list(&["beforeClass1", "method2"]),
                format!("[{}]", m.join(", ")),
            );
        }))
        // Awkward case: only method filter.
        .method(test("testMethodFilter", || {
            system::set_property(&SysGlobals::sysprop_testmethod(), "method1");
            let m = run();
            assert_equals(
                list(&["beforeClass1", "method1", "beforeClass2", "method1"]),
                format!("[{}]", m.join(", ")),
            );
        }))
        // Glob class name filter.
        .method(test("testGlobClassName", || {
            system::set_property(&SysGlobals::sysprop_testclass(), "*Nested1");
            let m = run();
            assert_true(
                is(&m, &["beforeClass1", "method1", "method2"])
                    || is(&m, &["beforeClass1", "method2", "method1"]),
            );
        }))
        // Glob method name filter.
        .method(test("testGlobMethodName", || {
            system::set_property(&SysGlobals::sysprop_testmethod(), "*hod1");
            let m = run();
            assert_equals(
                list(&["beforeClass1", "method1", "beforeClass2", "method1"]),
                format!("[{}]", m.join(", ")),
            );
        }))
        .method(hook("cleanupBefore", Before, cleanup_after))
        .method(hook("cleanupAfter", After, cleanup_after))
        .build()]
}
