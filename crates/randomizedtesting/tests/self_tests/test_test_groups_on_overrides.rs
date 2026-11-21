//! Port of `com.carrotsearch.randomizedtesting.TestTestGroupsOnOverrides`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{AnnotationType, TestGroup};
use randomizedtesting::jdk::reflect::{Class, Marker, Method};
use randomizedtesting::junit::{Before, Ignore};

use crate::support::{assert_that_list, cls, hook, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "TestTestGroupsOnOverrides";

/// `static List<String> buf`.
static BUF: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || BUF.lock().unwrap_or_else(|e| e.into_inner()).push(s.into())
}

static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(hook("assumeNested", Before, assume_running_nested))
        .method(test("method", add("super:method")))
        .build()
});

/// `@Documented @Inherited @TestGroup(enabled = false) public @interface Disabled`.
static DISABLED: AnnotationType = AnnotationType {
    name: "com.carrotsearch.randomizedtesting.TestTestGroupsOnOverrides$Disabled",
    simple_name: "Disabled",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: false,
    }),
    inherited: true,
};

fn sub(name: &str, m: Method) -> Arc<Class> {
    nested(O, name)
        .extends(&SUPER)
        .constructor(unit())
        .method(m)
        .build()
}

fn contents() -> Vec<String> {
    BUF.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `TestTestGroupsOnOverrides extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(hook("clean", Before, || {
            BUF.lock().unwrap_or_else(|e| e.into_inner()).clear()
        }))
        .method(test("testIgnored", || {
            run_tests(&[sub(
                "Ignored",
                Method::new("method")
                    .annotate(Ignore::new())
                    .body_any(add("sub:method")),
            )]);
            assert_that_list(contents()).is_empty();
        }))
        .method(test("testDisabledGroup", || {
            run_tests(&[sub(
                "DisabledGroup",
                Method::new("method")
                    .annotate(Marker(&DISABLED))
                    .body_any(add("sub:method")),
            )]);
            assert_that_list(contents()).is_empty();
        }))
        .method(test("testNormal", || {
            run_tests(&[sub(
                "Normal",
                Method::new("method").body_any(add("sub:method")),
            )]);
            assert_that_list(contents()).contains_exactly(&["sub:method"]);
        }))
        .build()]
}
