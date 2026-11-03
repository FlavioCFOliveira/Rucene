//! Port of `com.carrotsearch.randomizedtesting.TestJUnit3MethodProvider`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::TestMethodProviders;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::RandomizedRunner;

use crate::support::{
    assert_equals, cls, fail_msg, junit3_provider, nested, run_randomized_runner, test, unit,
};

const O: &str = "TestJUnit3MethodProvider";

static BASE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Base")
        .annotate(TestMethodProviders {
            value: vec![junit3_provider()],
        })
        .constructor(unit())
        .build()
});

/// `public static class <name> extends <superclass> { <method> }`.
fn sub(name: &str, superclass: &Arc<Class>, m: Method) -> Arc<Class> {
    nested(O, name)
        .extends(superclass)
        .constructor(unit())
        .method(m)
        .build()
}

fn instance(mods: i32) -> Method {
    Method::new("test1").modifiers(mods).body_any(|| {})
}

fn statik(mods: i32) -> Method {
    Method::new("test1").modifiers(mods).static_body(|| {})
}

/// `public static class S1/S2/S3`: a chain of overrides of `test1`.
static S3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    let s1 = sub("S1", &BASE, instance(modifier::PUBLIC));
    let s2 = sub("S2", &s1, instance(modifier::PUBLIC));
    sub("S3", &s2, instance(modifier::PUBLIC))
});

/// `TestJUnit3MethodProvider`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .constructor(unit())
        .method(test("testJUnit3Valid", || {
            let valid = [
                sub("T1", &BASE, instance(modifier::PUBLIC)),
                sub("RT1", &BASE, instance(modifier::PUBLIC).returns("int")),
            ];
            for cl in &valid {
                let r = run_randomized_runner(cl);
                assert_equals(0, r.failure_count());
                assert_equals(1, r.run_count());
            }
        }))
        .method(test("testJUnit3Invalid", || {
            let invalid = [
                sub("T2", &BASE, instance(0)),
                sub("T3", &BASE, instance(modifier::PROTECTED)),
                sub("T4", &BASE, instance(modifier::PRIVATE)),
                sub("ST1", &BASE, statik(modifier::PUBLIC)),
                sub("ST2", &BASE, statik(0)),
                sub("ST3", &BASE, statik(modifier::PROTECTED)),
                sub("ST4", &BASE, statik(modifier::PRIVATE)),
                sub("AT1", &BASE, instance(modifier::PUBLIC).params(&["int"])),
            ];
            for cl in &invalid {
                if RandomizedRunner::new(cl).is_ok() {
                    fail_msg(&format!("Expected to fail for: class {}", cl.name()));
                }
                // InitializationError: expected.
            }
        }))
        .method(test("testJUnit3Overrides", || {
            let r = run_randomized_runner(&S3);
            assert_equals(0, r.failure_count());
            assert_equals(1, r.run_count());
        }))
        .build()]
}
