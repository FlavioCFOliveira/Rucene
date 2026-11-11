//! Port of `com.carrotsearch.randomizedtesting.TestParameterized`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{ParametersFactory, Repeat, Seeds};
use randomizedtesting::jdk::exception::{illegal_argument, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::reflect::{Class, Constructor, Method, Value};
use randomizedtesting::method_name;
use randomizedtesting::RandomizedTest;

use crate::support::{
    args, assert_equals, assert_that_list, assert_that_str, cls, fail, nested, test,
    throw_no_message, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

const O: &str = "TestParameterized";

/// `@ParametersFactory public static Iterable<Object[]> parameters()`.
fn factory(body: impl Fn() -> Value + Send + Sync + 'static) -> Method {
    Method::new("parameters")
        .returns("java.lang.Iterable<java.lang.Object[]>")
        .annotate(ParametersFactory::default())
        .static_value(move |_| body())
}

/// A constructor `(@Name(name) int value)`: Java's reflective call rejects a
/// non-`Integer` argument (`argument type mismatch`).
fn int_constructor(name: &'static str) -> Constructor {
    Constructor::with_params(&[("int", Some(name))], |a| match a.first() {
        Some(Value::Int(_)) => (),
        _ => illegal_argument("argument type mismatch"),
    })
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(Constructor::with_params(
            &[("int", Some("value")), ("java.lang.String", Some("string"))],
            |_| (),
        ))
        .method(test("testOne", || {}).annotate(Repeat {
            iterations: 3,
            use_constant_seed: false,
        }))
        .method(test("testTwo", || {}))
        .method(
            test("testThree", || {})
                .annotate(Seeds::new(&["deadbeef", "cafebabe"]))
                .annotate(Repeat {
                    iterations: 2,
                    use_constant_seed: true,
                }),
        )
        .method(factory(|| {
            Value::List(vec![
                args(vec![Value::Int(1), Value::Str("abc".into())]),
                args(vec![Value::Int(2), Value::Str("def".into())]),
            ])
        }))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&RANDOMIZED_TEST)
        .constructor(int_constructor("paramName"))
        .method(test("failing", || {
            assume_running_nested();
            fail();
        }))
        .method(factory(|| {
            assume_running_nested();
            Value::List(vec![args(vec![Value::Str("xyz".into())])])
        }))
        .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested3")
        .extends(&NESTED2)
        .constructor(int_constructor("paramName"))
        .method(factory(|| Value::List(Vec::new())))
        .build()
});

static NESTED4: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested4")
        .extends(&NESTED3)
        .constructor(int_constructor("paramName"))
        .method(factory(|| {
            RandomizedTest::assume_true(false);
            throw_no_message(RUNTIME_EXCEPTION)
        }))
        .build()
});

static NESTED5: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested5")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testMe", || {}))
        // Arrays.asList(new Object[] {}, new Integer[] {}).
        .method(factory(|| {
            Value::List(vec![args(Vec::new()), args(Vec::new())])
        }))
        .build()
});

/// `TestParameterized extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testWithRepeatsAndSeeds", || {
            check_tests_output(16, 0, 0, 0, &NESTED);
        }))
        .method(test("testNameAnnotation", || {
            let r = check_tests_output(1, 0, 1, 0, &NESTED2);
            let failures = r.get_failures();
            assert_that_list(failures.iter().map(|f| f.to_string())).has_size(1);
            assert_that_str(failures[0].description().method_name().unwrap_or_default())
                .contains("paramName=xyz");
            assert_equals(
                "failing".to_string(),
                method_name(failures[0].description()),
            );
        }))
        .method(test("testEmptyParamsList", || {
            check_tests_output(0, 0, 0, 0, &NESTED3);
            check_tests_output(0, 0, 0, 0, &NESTED4);
        }))
        .method(test("testNonObjectArray", || {
            check_tests_output(2, 0, 0, 0, &NESTED5);
        }))
        .build()]
}
