//! Port of `com.carrotsearch.randomizedtesting.TestTestCaseOrdering`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{
    jdk_string_compare, ClassRef, ParametersFactory, Repeat, TestCaseComparatorBox,
    TestCaseOrdering,
};
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method, Value};
use randomizedtesting::junit::{Before, Test};

use crate::support::{args, assert_that_list, cls, hook, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "TestTestCaseOrdering";

/// `static List<String> buf`.
static BUF: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: String) {
    BUF.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

fn abcd(b: ClassBuilder) -> ClassBuilder {
    let mut b = b.method(hook("assumeNested", Before, assume_running_nested));
    for n in ["a", "b", "c", "d"] {
        b = b.method(test(n, move || add(n.to_string())));
    }
    b
}

static ALPHABETICAL: LazyLock<Arc<Class>> = LazyLock::new(|| {
    abcd(
        nested(O, "Alphabetical")
            .extends(&RANDOMIZED_TEST)
            .annotate(TestCaseOrdering::alphabetic_order())
            .constructor(unit()),
    )
    .build()
});

static ALPHABETICAL_WITH_REPETITIONS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    abcd(
        nested(O, "AlphabeticalWithRepetitions")
            .extends(&RANDOMIZED_TEST)
            .annotate(TestCaseOrdering::alphabetic_order())
            .annotate(Repeat {
                iterations: 3,
                use_constant_seed: false,
            })
            .constructor(unit()),
    )
    .build()
});

/// `String param`.
struct Inst {
    param: String,
}

/// `CustomOrder implements Comparator<TestMethodAndParams>`: by method name,
/// then by the first instance argument.
fn custom_order() -> TestCaseOrdering {
    TestCaseOrdering {
        value: ClassRef::new(
            "com.carrotsearch.randomizedtesting.TestTestCaseOrdering$CustomOrder",
            || {
                Arc::new(TestCaseComparatorBox(Arc::new(|o1, o2| {
                    let v = jdk_string_compare(o1.test_method().name(), o2.test_method().name());
                    if v != std::cmp::Ordering::Equal {
                        return v;
                    }
                    let s = |o: &dyn randomizedtesting::annotations::TestMethodAndParams| match o
                        .instance_arguments()
                        .first()
                    {
                        Some(Value::Str(s)) => s.clone(),
                        _ => String::new(),
                    };
                    jdk_string_compare(&s(o1), &s(o2))
                })))
            },
        ),
    }
}

static ALPHABETICAL_WITH_PARAMETERS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    let mut b = nested(O, "AlphabeticalWithParameters")
        .extends(&RANDOMIZED_TEST)
        .annotate(custom_order())
        .constructor(Constructor::with_params(
            &[("java.lang.String", None)],
            |a| Inst {
                param: a.first().map(|v| v.to_string()).unwrap_or_default(),
            },
        ))
        .method(hook("assumeNested", Before, assume_running_nested));
    for n in ["a", "b", "c", "d"] {
        b = b.method(
            Method::new(n)
                .annotate(Test::new())
                .body(move |t: &mut Inst| add(format!("{n}:{}", t.param))),
        );
    }
    b.method(
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

fn buf() -> Vec<String> {
    BUF.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `TestTestCaseOrdering extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(hook("clean", Before, || {
            BUF.lock().unwrap_or_else(|e| e.into_inner()).clear()
        }))
        .method(test("testAlphabetical", || {
            run_tests(std::slice::from_ref(&*ALPHABETICAL));
            assert_that_list(buf()).contains_exactly(&["a", "b", "c", "d"]);
        }))
        .method(test("testAlphabeticalWithRepetitions", || {
            run_tests(std::slice::from_ref(&*ALPHABETICAL_WITH_REPETITIONS));
            assert_that_list(buf())
                .contains_exactly(&["a", "a", "a", "b", "b", "b", "c", "c", "c", "d", "d", "d"]);
        }))
        .method(test("testAlphabeticalWithParameters", || {
            run_tests(std::slice::from_ref(&*ALPHABETICAL_WITH_PARAMETERS));
            assert_that_list(buf()).contains_exactly(&[
                "a:1", "a:2", "a:3", "b:1", "b:2", "b:3", "c:1", "c:2", "c:3", "d:1", "d:2", "d:3",
            ]);
        }))
        .build()]
}
