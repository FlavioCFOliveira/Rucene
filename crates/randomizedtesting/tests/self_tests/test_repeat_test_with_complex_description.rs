//! Port of `com.carrotsearch.randomizedtesting.TestRepeatTestWithComplexDescription`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): `tests.method`
//! selects each test by its full name, even with special characters.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{ClassRef, Listeners, ParametersFactory, Repeat, Seed, Seeds};
use randomizedtesting::jdk::reflect::{Class, Constructor, Field, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Description, Rule, RunListener};
use randomizedtesting::rules::SystemPropertiesRestoreRule;
use randomizedtesting::SysGlobals;

use crate::support::{
    args, assert_that_bool, assert_that_list, cls, nested, test, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

/// `static ArrayList<String> buf`.
static BUF: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `CaptureFailuresListener extends RunListener`.
struct CaptureFailuresListener;

impl RunListener for CaptureFailuresListener {
    fn test_started(&self, d: &Arc<Description>) {
        BUF.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(d.method_name().unwrap_or_default());
    }
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestRepeatTestWithComplexDescription", "Nested")
        .extends(&RANDOMIZED_TEST)
        .annotate(Seed::new("deadbeef"))
        .annotate(Listeners {
            value: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.TestRepeatTestWithComplexDescription$CaptureFailuresListener",
                || Arc::new(CaptureFailuresListener) as Arc<dyn RunListener>,
            )],
        })
        .constructor(Constructor::with_params(&[("java.lang.String", Some("value"))], |_| ()))
        .method(
            test("testFoo", || {})
                .annotate(Seeds::new(&["random", "deadbeef"]))
                .annotate(Repeat {
                    iterations: 5,
                    use_constant_seed: false,
                }),
        )
        .method(test("testBar", || {}))
        .method(
            Method::new("parameters")
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory::default())
                .static_value(|_| {
                    assume_running_nested();
                    Value::List(
                        ["", "a/b/c", "\\", "$[]{}\\"]
                            .iter()
                            .map(|s| args(vec![Value::Str((*s).into())]))
                            .collect(),
                    )
                }),
        )
        .build()
});

/// `TestRepeatTestWithComplexDescription extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRepeatTestWithComplexDescription")
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
        .method(test("checkFilteringByName", || {
            // Collect all test names first.
            BUF.lock().unwrap_or_else(|e| e.into_inner()).clear();
            run_tests(std::slice::from_ref(&*NESTED));
            let names = BUF.lock().unwrap_or_else(|e| e.into_inner()).clone();
            for test_name in names {
                BUF.lock().unwrap_or_else(|e| e.into_inner()).clear();
                system::set_property(&SysGlobals::sysprop_testmethod(), &test_name);
                assert_that_bool(run_tests(std::slice::from_ref(&*NESTED)).was_successful())
                    .is_true();
                let buf = BUF.lock().unwrap_or_else(|e| e.into_inner()).clone();
                assert_that_list(buf).contains_only(&[test_name.as_str()]);
            }
        }))
        .build()]
}
