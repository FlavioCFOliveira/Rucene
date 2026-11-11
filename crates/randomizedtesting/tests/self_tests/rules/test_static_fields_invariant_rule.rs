//! Port of `com.carrotsearch.randomizedtesting.rules.TestStaticFieldsInvariantRule`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).
//!
//! On JDK 25 the Java class fails three of its tests: the rule cannot
//! measure `Base.classRules`, a JUnit `RuleChain` whose `ArrayList` lives in
//! a module it may not open, and reports that instead of the sizes. The port
//! fails the same way (the estimator's `RuleChain` case).
//!
//! **Divergence.** `Base.assumeNotNestedRule` is an instance of an anonymous
//! `TestRule` without fields, which Java's estimator measures
//! (`TestStaticFieldsInvariantRule.java:22`). The port's estimator cannot
//! measure an opaque Rust rule (`RamUsageEstimator` divergences), so the
//! declaration keeps the rule in the class rule chain without exposing it as
//! a static field; every other field is declared as Java declares it.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{Scope, ThreadLeakScope};
use randomizedtesting::jdk::reflect::{modifier, Class, Field, Method, Value};
use randomizedtesting::junit::{BeforeClass, ClassRule, Ignore, RuleChain};
use randomizedtesting::rules::StaticFieldsInvariantRule;
use randomizedtesting::RandomizedTest;

use super::{cls, nested, AssumeNotNestedRule};
use crate::support::{assert_that_list, assert_that_str, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

const O: &str = "TestStaticFieldsInvariantRule";

/// `static int LEAK_THRESHOLD`.
const LEAK_THRESHOLD: i64 = 5 * 1024 * 1024;

static BASE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Base")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope {
            value: Scope::Suite,
        })
        .constructor(unit())
        .field(
            Field::new_static("classRules", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(
                    RuleChain::outer_rule(Arc::new(AssumeNotNestedRule)).around(Arc::new(
                        StaticFieldsInvariantRule::with_threshold(LEAK_THRESHOLD, true),
                    )),
                ))
            })
            .annotate(ClassRule),
        )
        .method(test("testEmpty", || {}))
        .build()
});

type Slot = Mutex<Value>;

static FIELD0: Slot = Mutex::new(Value::Null);
static FIELD1: Slot = Mutex::new(Value::Null);
static FIELD2: Slot = Mutex::new(Value::Null);
static FIELD3: Slot = Mutex::new(Value::Null);
static FIELD5: LazyLock<Value> = LazyLock::new(|| Value::primitive_array("long", 1024));
static REF: Slot = Mutex::new(Value::Null);

fn get(m: &'static Slot) -> impl Fn() -> Value + Send + Sync + 'static {
    move || m.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(m: &'static Slot, v: Value) {
    *m.lock().unwrap_or_else(|e| e.into_inner()) = v;
}

/// A package-private `static` field.
fn static_field(name: &str, ty: &str, m: &'static Slot) -> Field {
    Field::new_static(name, ty, get(m)).modifiers(0)
}

fn private_setup(body: impl Fn() + Send + Sync + 'static) -> Method {
    Method::new("setup")
        .modifiers(modifier::PRIVATE)
        .annotate(BeforeClass)
        .static_body(body)
}

static SMALLER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Smaller")
        .extends(&BASE)
        .constructor(unit())
        .field(static_field("field0", "byte[]", &FIELD0))
        .method(private_setup(|| {
            set(
                &FIELD0,
                Value::primitive_array("byte", (LEAK_THRESHOLD / 2) as usize),
            )
        }))
        .build()
});

static EXCEEDING: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Exceeding")
        .extends(&SMALLER)
        .constructor(unit())
        .field(static_field("field1", "byte[]", &FIELD1))
        .field(static_field("field2", "byte[]", &FIELD2))
        .field(static_field("field3", "int[]", &FIELD3))
        .field(Field::new_static("field4", "long", || Value::Long(0)).modifiers(0))
        .field(Field::new_static("field5", "long[]", || FIELD5.clone()).modifiers(modifier::FINAL))
        .method(private_setup(|| {
            set(
                &FIELD1,
                Value::primitive_array("byte", (LEAK_THRESHOLD / 2) as usize),
            );
            set(&FIELD2, Value::primitive_array("byte", 100));
            set(&FIELD3, Value::primitive_array("int", 100));
        }))
        .build()
});

/// `MultipleReferences`: six fields referencing one `HashMap` holding a
/// 2 MiB `byte[]`, and a `null` seventh.
static MULTIPLE_REFERENCES: LazyLock<Arc<Class>> = LazyLock::new(|| {
    let mut b = nested(O, "MultipleReferences")
        .extends(&BASE)
        .constructor(unit());
    for name in ["ref1", "ref2", "ref3", "ref4", "ref5", "ref6"] {
        b = b.field(static_field(name, "java.lang.Object", &REF));
    }
    b.field(Field::new_static("ref7", "java.lang.Object", || Value::Null).modifiers(0))
        .static_init(|| {
            let mut map: HashMap<String, Value> = HashMap::new();
            map.insert(
                "key".into(),
                Value::primitive_array("byte", 1024 * 1024 * 2),
            );
            set(&REF, Value::object(map, "{key=[B}"));
        })
        .build()
});

/// `FailsJava9`'s `Holder`: assumes `Paths.get(".")` is a JVM-internal class.
static FAILS_JAVA9: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "FailsJava9")
        .extends(&BASE)
        .constructor(unit())
        .field(static_field(
            "field0",
            "com.carrotsearch.randomizedtesting.rules.TestStaticFieldsInvariantRule$Holder",
            &FIELD0,
        ))
        .method(private_setup(|| {
            RandomizedTest::assume_true_msg(
                "java.nio.file.Path is not implemented by internal class in this JVM",
                false,
            );
        }))
        .build()
});

/// `TestStaticFieldsInvariantRule extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testReferencesCountedMultipleTimes", || {
            let failures = run_tests(std::slice::from_ref(&*MULTIPLE_REFERENCES)).get_failures();
            assert_that_list(failures).is_empty();
        }))
        .method(test("testPassingUnderThreshold", || {
            let failures = run_tests(std::slice::from_ref(&*SMALLER)).get_failures();
            assert_that_list(failures).is_empty();
        }))
        .method(test("testFailingAboveThreshold", || {
            let failures = run_tests(std::slice::from_ref(&*EXCEEDING)).get_failures();
            assert_that_list(failures.iter().map(|f| f.to_string())).has_size(1);
            assert_that_str(failures[0].trace())
                .contains(".field0")
                .contains(".field1")
                .contains(".field2")
                .contains(".field3")
                .does_not_contain(".field5");
        }))
        .method(
            test("testJava9Jigsaw", || {
                let failures = run_tests(std::slice::from_ref(&*FAILS_JAVA9)).get_failures();
                assert_that_list(failures.iter().map(|f| f.to_string())).has_size(1);
                assert_that_str(failures[0].trace())
                    .contains("sizes cannot be measured due to security restrictions or Java 9")
                    .contains(".field0");
            })
            .annotate(Ignore::new()),
        )
        .build()]
}
