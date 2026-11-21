//! Port of `com.carrotsearch.randomizedtesting.TestCustomMethodProvider`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{AnnotationType, ClassRef, TestMethodProviders};
use randomizedtesting::jdk::reflect::{modifier, Class, Marker, Method};
use randomizedtesting::{AnnotatedMethodProvider, RandomizedRunner, TestMethodProvider};

use crate::support::{assert_equals, cls, fail_msg, nested, run_randomized_runner, test, unit};

const O: &str = "TestCustomMethodProvider";

/// `@Retention(RUNTIME) @Target(METHOD) @Inherited public static @interface MyTest`.
static MY_TEST: AnnotationType = AnnotationType::plain(
    "com.carrotsearch.randomizedtesting.TestCustomMethodProvider$MyTest",
    "MyTest",
    true,
);

/// `MyTestMethodProvider extends AnnotatedMethodProvider` over `@MyTest`.
static BASE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Base")
        .annotate(TestMethodProviders {
            value: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.TestCustomMethodProvider$MyTestMethodProvider",
                || Arc::new(AnnotatedMethodProvider::new(&MY_TEST)) as Arc<dyn TestMethodProvider>,
            )],
        })
        .constructor(unit())
        .build()
});

fn sub(name: &str, m: Method) -> Arc<Class> {
    nested(O, name)
        .extends(&BASE)
        .constructor(unit())
        .method(m.annotate(Marker(&MY_TEST)))
        .build()
}

fn instance(mods: i32) -> Method {
    Method::new("test1").modifiers(mods).body_any(|| {})
}

fn statik(mods: i32) -> Method {
    Method::new("test1").modifiers(mods).static_body(|| {})
}

/// `TestCustomMethodProvider extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&crate::with_nested_test_class::CLASS)
        .constructor(unit())
        .method(test("testJUnit4Valid", || {
            let valid = [
                sub("T1", instance(modifier::PUBLIC)),
                sub("RT1", instance(modifier::PUBLIC).returns("int")),
            ];
            for cl in &valid {
                let r = run_randomized_runner(cl);
                assert_equals(0, r.failure_count());
                assert_equals(1, r.run_count());
            }
        }))
        .method(test("testJUnit4Invalid", || {
            let invalid = [
                sub("T2", instance(0)),
                sub("T3", instance(modifier::PROTECTED)),
                sub("T4", instance(modifier::PRIVATE)),
                sub("ST1", statik(modifier::PUBLIC)),
                sub("ST2", statik(0)),
                sub("ST3", statik(modifier::PROTECTED)),
                sub("ST4", statik(modifier::PRIVATE)),
                sub("AT1", instance(modifier::PUBLIC).params(&["int"])),
            ];
            for cl in &invalid {
                if RandomizedRunner::new(cl).is_ok() {
                    fail_msg(&format!("Expected to fail for: class {}", cl.name()));
                }
            }
        }))
        .build()]
}
