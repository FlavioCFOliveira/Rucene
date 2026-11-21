//! Port of `com.carrotsearch.randomizedtesting.TestTestCaseInstanceProviders`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{
    InstanceProviderType, ParametersFactory, Repeat, Seeds, TestCaseInstanceProvider,
};
use randomizedtesting::jdk::reflect::{Class, Constructor, Method, Value};
use randomizedtesting::junit::{Before, Test};

use crate::support::{args, assert_that_num, cls, hook, nested, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

/// `IdentityHashMap<Object,Object> set`: instance identities.
static SET: LazyLock<Mutex<HashSet<usize>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// A suite instance; one byte so that every instance has its own address.
struct Inst {
    _identity: u8,
}

fn put(t: &mut Inst) {
    SET.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(t as *mut Inst as usize);
}

fn instance_test(name: &str) -> Method {
    Method::new(name).annotate(Test::new()).body(put)
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestTestCaseInstanceProviders", "Nested")
        .extends(&RANDOMIZED_TEST)
        .annotate(TestCaseInstanceProvider {
            value: InstanceProviderType::InstancePerConstructorArgs,
        })
        .constructor(Constructor::no_args(|| Inst { _identity: 0 }))
        .method(Method::new("setup").annotate(Before).body(|t: &mut Inst| {
            assume_running_nested();
            put(t);
        }))
        .method(instance_test("testOne").annotate(Repeat {
            iterations: 3,
            use_constant_seed: false,
        }))
        .method(instance_test("testTwo"))
        .method(
            instance_test("testThree")
                .annotate(Seeds::new(&["deadbeef", "cafebabe"]))
                .annotate(Repeat {
                    iterations: 2,
                    use_constant_seed: true,
                }),
        )
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestTestCaseInstanceProviders", "Nested2")
        .extends(&NESTED)
        .constructor(Constructor::with_params(
            &[("java.lang.String", None)],
            |_| Inst { _identity: 0 },
        ))
        .method(
            Method::new("params")
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory::default())
                .static_value(|_| {
                    Value::List(
                        ["1", "2", "3"]
                            .iter()
                            .map(|s| args(vec![Value::Str((*s).into())]))
                            .collect(),
                    )
                }),
        )
        .build()
});

/// `TestTestCaseInstanceProviders extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestTestCaseInstanceProviders")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(hook("setup", Before, || {
            SET.lock().unwrap_or_else(|e| e.into_inner()).clear()
        }))
        .method(crate::support::test("testDefaultConstructor", || {
            check_tests_output(3 + 1 + 2 * 2, 0, 0, 0, &NESTED);
            assert_that_num(SET.lock().unwrap_or_else(|e| e.into_inner()).len() as i64)
                .is_equal_to(1);
        }))
        .method(crate::support::test("testParameterProviders", || {
            check_tests_output(3 * (3 + 1 + 2 * 2), 0, 0, 0, &NESTED2);
            assert_that_num(SET.lock().unwrap_or_else(|e| e.into_inner()).len() as i64)
                .is_equal_to(3);
        }))
        .build()]
}
