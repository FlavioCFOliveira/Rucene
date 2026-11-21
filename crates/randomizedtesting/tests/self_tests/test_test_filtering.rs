//! Port of `com.carrotsearch.randomizedtesting.TestTestFiltering`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): a custom test group
//! against `tests.filter`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{AnnotationType, TestGroup, TestGroupUtilities};
use randomizedtesting::jdk::reflect::{Class, Field, Marker, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::Rule;
use randomizedtesting::rules::SystemPropertiesRestoreRule;
use randomizedtesting::SysGlobals;

use crate::support::{assert_that_num, cls, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{check_tests_output, CLASS as WITH_NESTED};

/// `@Inherited @TestGroup(enabled = false) public static @interface Foo`.
pub static FOO: AnnotationType = AnnotationType {
    name: "com.carrotsearch.randomizedtesting.TestTestFiltering$Foo",
    simple_name: "Foo",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: false,
    }),
    inherited: true,
};

/// `static AtomicInteger counter`.
static COUNTER: AtomicI32 = AtomicI32::new(0);

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestTestFiltering", "Nested1")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("test1", || {
                COUNTER.fetch_add(1, Ordering::SeqCst);
            })
            .annotate(Marker(&FOO)),
        )
        .build()
});

/// `TestTestFiltering extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestTestFiltering")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .field(
            Field::new_instance::<()>(
                "restoreProperties",
                "com.carrotsearch.randomizedtesting.rules.SystemPropertiesRestoreRule",
                |_| Value::TestRule(Arc::new(SystemPropertiesRestoreRule::new())),
            )
            .annotate(Rule),
        )
        .method(test("filterVsRulePriority", || {
            system::set_property(&TestGroupUtilities::get_sys_property(&FOO), "false");
            // Don't run by default (group is disabled by default).
            COUNTER.store(0, Ordering::SeqCst);
            system::set_property(&SysGlobals::sysprop_testfilter(), "");
            check_tests_output(1, 0, 0, 1, &NESTED1);
            assert_that_num(COUNTER.load(Ordering::SeqCst)).is_equal_to(0);
            // Run @foo methods even though the group is disabled (but the filtering rule takes priority).
            COUNTER.store(0, Ordering::SeqCst);
            system::set_property(&SysGlobals::sysprop_testfilter(), "@foo");
            check_tests_output(1, 0, 0, 0, &NESTED1);
            assert_that_num(COUNTER.load(Ordering::SeqCst)).is_equal_to(1);
            // Run the "default" filter.
            COUNTER.store(0, Ordering::SeqCst);
            system::set_property(&SysGlobals::sysprop_testfilter(), "default");
            check_tests_output(0, 0, 0, 0, &NESTED1);
            assert_that_num(COUNTER.load(Ordering::SeqCst)).is_equal_to(0);
        }))
        .build()]
}
