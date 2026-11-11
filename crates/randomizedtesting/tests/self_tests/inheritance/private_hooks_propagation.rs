//! Port of `com.carrotsearch.randomizedtesting.inheritance.PrivateHooksPropagation`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::{Before, BeforeClass};

use super::{cls, nested};
use crate::support::{assert_that_list, run_randomized_runner, test, unit, RANDOMIZED_TEST};

const O: &str = "PrivateHooksPropagation";

/// `final static List<String> order`.
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn step(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || {
        ORDER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(s.into())
    }
}

fn class_hook(name: &str, mods: i32, s: &'static str) -> Method {
    Method::new(name)
        .modifiers(mods)
        .annotate(BeforeClass)
        .static_body(step(s))
}

fn hook(name: &str, mods: i32, s: &'static str) -> Method {
    Method::new(name)
        .modifiers(mods)
        .annotate(Before)
        .body_any(step(s))
}

static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(class_hook(
            "beforeClass1",
            modifier::PRIVATE,
            "super.beforeclass1",
        ))
        .method(class_hook(
            "beforeClass2",
            modifier::PROTECTED,
            "super.beforeclass2",
        ))
        .method(hook("before1", modifier::PRIVATE, "super.before1"))
        .method(hook("before2", modifier::PROTECTED, "super.before2"))
        .method(test("testMethod1", step("super.testMethod1")))
        .build()
});

static SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Sub")
        .extends(&SUPER)
        .constructor(unit())
        .method(class_hook(
            "beforeClass1",
            modifier::PRIVATE,
            "sub.beforeclass1",
        ))
        .method(class_hook(
            "beforeClass2",
            modifier::PROTECTED,
            "sub.beforeclass2",
        ))
        .method(hook("before1", modifier::PRIVATE, "sub.before1"))
        .method(hook("before2", modifier::PROTECTED, "sub.before2"))
        .build()
});

/// `PrivateHooksPropagation`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .constructor(unit())
        .method(test("checkOldMethodRules", || {
            run_randomized_runner(&SUB);
            let order1 = std::mem::take(&mut *ORDER.lock().unwrap_or_else(|e| e.into_inner()));
            let msg = format!("# RR order:\n[{}]", order1.join(", "));
            assert_that_list(order1).as_(msg).contains_only(&[
                "super.beforeclass1",
                "sub.beforeclass1",
                "sub.beforeclass2",
                "super.before1",
                "sub.before1",
                "sub.before2",
                "super.testMethod1",
            ]);
        }))
        .build()]
}
