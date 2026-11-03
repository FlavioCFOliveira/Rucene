//! Port of `com.carrotsearch.randomizedtesting.TestParameterizedShufflesOrder`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::ParametersFactory;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method, Value};
use randomizedtesting::junit::{BeforeClass, Test};

use crate::support::{
    args, assert_that_bool, assert_that_num, cls, nested, static_hook, test, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "TestParameterizedShufflesOrder";

/// `static StringBuilder buf`.
static BUF: Mutex<String> = Mutex::new(String::new());

/// `Base`'s `private final int value`.
struct Inst {
    value: i32,
}

/// `public static abstract class Base extends RandomizedTest`.
static BASE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Base")
        .modifiers(modifier::PUBLIC | modifier::STATIC | modifier::ABSTRACT)
        .extends(&RANDOMIZED_TEST)
        .constructor(Constructor::with_params(&[("int", None)], value_of))
        .method(static_hook(
            "checkNested",
            BeforeClass,
            assume_running_nested,
        ))
        .method(
            Method::new("testFoo")
                .annotate(Test::new())
                .body(|t: &mut Inst| {
                    BUF.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_str(&format!("{},", t.value));
                }),
        )
        .build()
});

fn value_of(a: &[Value]) -> Inst {
    Inst {
        value: match a.first() {
            Some(Value::Int(i)) => *i,
            _ => 0,
        },
    }
}

fn sub(name: &str, shuffle: bool) -> Arc<Class> {
    nested(O, name)
        .extends(&BASE)
        .constructor(Constructor::with_params(&[("int", None)], value_of))
        .method(
            Method::new("parameters")
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory {
                    shuffle,
                    ..ParametersFactory::default()
                })
                .static_value(|_| {
                    Value::List((0..10).map(|i| args(vec![Value::Int(i)])).collect())
                }),
        )
        .build()
}

static NO_SHUFFLE: LazyLock<Arc<Class>> = LazyLock::new(|| sub("NoShuffle", false));
static WITH_SHUFFLE: LazyLock<Arc<Class>> = LazyLock::new(|| sub("WithShuffle", true));

fn runs(c: &Arc<Class>, iters: usize) -> HashSet<String> {
    let mut runs = HashSet::new();
    for _ in 0..iters {
        BUF.lock().unwrap_or_else(|e| e.into_inner()).clear();
        assert_that_bool(run_tests(std::slice::from_ref(c)).was_successful()).is_true();
        runs.insert(BUF.lock().unwrap_or_else(|e| e.into_inner()).clone());
    }
    runs
}

/// `TestParameterizedShufflesOrder extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testWithoutShuffle", || {
            assert_that_num(runs(&NO_SHUFFLE, 10).len() as i64).is_equal_to(1);
        }))
        .method(test("testWithShuffle", || {
            let iters = 10;
            assert_that_num(runs(&WITH_SHUFFLE, iters).len() as i64).is_equal_to(iters as i64);
        }))
        .build()]
}
