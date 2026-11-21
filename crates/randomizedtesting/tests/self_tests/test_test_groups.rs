//! Port of `com.carrotsearch.randomizedtesting.TestTestGroups`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): custom test groups.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{AnnotationType, TestGroup, TestGroupUtilities as U, NIGHTLY};
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Marker};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use randomizedtesting::SysGlobals;

use crate::support::{
    assert_equals, assert_false, assert_true, cls, nested, static_hook, test, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{check_tests_output, CLASS as WITH_NESTED};

const O: &str = "TestTestGroups";

/// `@Inherited @TestGroup(enabled = true) public static @interface Group1`.
static GROUP1: AnnotationType = AnnotationType {
    name: "com.carrotsearch.randomizedtesting.TestTestGroups$Group1",
    simple_name: "Group1",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: true,
    }),
    inherited: true,
};

/// `@Inherited @TestGroup(enabled = false, name = "abc", sysProperty = "custom.abc") Group2`.
static GROUP2: AnnotationType = AnnotationType {
    name: "com.carrotsearch.randomizedtesting.TestTestGroups$Group2",
    simple_name: "Group2",
    test_group: Some(TestGroup {
        name: "abc",
        sys_property: "custom.abc",
        enabled: false,
    }),
    inherited: true,
};

/// `public static boolean beforeClassRan, afterClassRan`.
static BEFORE_CLASS_RAN: AtomicBool = AtomicBool::new(false);
static AFTER_CLASS_RAN: AtomicBool = AtomicBool::new(false);

fn hooks(b: ClassBuilder) -> ClassBuilder {
    b.method(static_hook("beforeClass", BeforeClass, || {
        BEFORE_CLASS_RAN.store(true, Ordering::SeqCst)
    }))
    .method(static_hook("afterClass", AfterClass, || {
        AFTER_CLASS_RAN.store(true, Ordering::SeqCst)
    }))
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    hooks(
        nested(O, "Nested1")
            .extends(&RANDOMIZED_TEST)
            .constructor(unit())
            .method(
                test("test1", || {})
                    .annotate(Marker(&GROUP1))
                    .annotate(Marker(&GROUP2)),
            ),
    )
    .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested3")
        .extends(&NESTED1)
        .constructor(unit())
        .method(test("testUnconditional", || {}))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    hooks(
        nested(O, "Nested2")
            .extends(&RANDOMIZED_TEST)
            .annotate(Marker(&GROUP1))
            .annotate(Marker(&GROUP2))
            .constructor(unit())
            .method(test("test1", || {})),
    )
    .build()
});

fn reset() {
    AFTER_CLASS_RAN.store(false, Ordering::SeqCst);
    BEFORE_CLASS_RAN.store(false, Ordering::SeqCst);
}

fn hooks_ran(expected: bool) {
    let check = if expected { assert_true } else { assert_false };
    check(AFTER_CLASS_RAN.load(Ordering::SeqCst));
    check(BEFORE_CLASS_RAN.load(Ordering::SeqCst));
}

/// The four steps of each `groups*` test: (`run`, `assumptions`, hooks ran)
/// before any property, with group1, with both, with group1 disabled.
fn groups_test(class: &Arc<Class>, run: usize, steps: [(usize, bool); 4]) {
    let g1 = U::get_sys_property(&GROUP1);
    let g2 = U::get_sys_property(&GROUP2);
    let r = crate::support::try_catch(|| {
        reset();
        check_tests_output(run, 0, 0, steps[0].0, class);
        hooks_ran(steps[0].1);
        reset();
        system::set_property(&g1, "true");
        check_tests_output(run, 0, 0, steps[1].0, class);
        hooks_ran(steps[1].1);
        reset();
        system::set_property(&g2, "true");
        check_tests_output(run, 0, 0, steps[2].0, class);
        hooks_ran(steps[2].1);
        reset();
        system::set_property(&g1, "false");
        check_tests_output(run, 0, 0, steps[3].0, class);
        hooks_ran(steps[3].1);
    });
    system::clear_property(&g1);
    system::clear_property(&g2);
    if let Err(e) = r {
        e.throw();
    }
}

/// `TestTestGroups extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkDefaultNames", || {
            assert_equals("group1".to_string(), U::get_group_name(&GROUP1));
            assert_equals("abc".to_string(), U::get_group_name(&GROUP2));
            assert_equals(
                format!("{}.group1", SysGlobals::current_prefix()),
                U::get_sys_property(&GROUP1),
            );
            assert_equals("custom.abc".to_string(), U::get_sys_property(&GROUP2));
            assert_equals(
                format!("{}.nightly", SysGlobals::current_prefix()),
                U::get_sys_property(&NIGHTLY),
            );
            assert_equals("nightly".to_string(), U::get_group_name(&NIGHTLY));
        }))
        .method(test("groupsOnMethods", || {
            groups_test(&NESTED1, 1, [(1, false), (1, false), (0, true), (1, false)])
        }))
        .method(test("groupsOnASubsetOfMethods", || {
            groups_test(&NESTED3, 2, [(1, true), (1, true), (0, true), (1, true)])
        }))
        .method(test("groupsOnClass", || {
            groups_test(&NESTED2, 1, [(1, false), (1, false), (0, true), (1, false)])
        }))
        .build()]
}
