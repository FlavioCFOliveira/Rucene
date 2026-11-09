//! Shared plumbing of the `core_util_*` test targets, which port the tests of
//! `lucene/core/src/test/org/apache/lucene/util/` (sprint 9, tasks #400 and
//! #401): declaring a Java test class whose methods are Rust functions,
//! running it as a suite, the JUnit assertions the ported bodies use beyond
//! [`Assert`], and an iteration-order-faithful `java.util.HashSet`.

#![allow(dead_code)]

#[path = "../suite_seeds/mod.rs"]
pub mod suite_seeds;

pub mod java_hash_set;

use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::LuceneTestCase;

#[allow(unused_imports)]
pub use java_hash_set::{JavaHashCode, JavaHashSet};

/// The body of a ported test method: Java's `throws Exception`, as a
/// `Result` whose error is rethrown as the Java exception it stands for.
pub type TestFn = fn() -> Result<()>;

/// A public `test*` method of a Java test class, found by `LuceneTestCase`'s
/// JUnit 3 method provider; an `Err` is thrown as its Java exception.
pub fn test(name: &str, body: TestFn) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        })
}

/// The declaration of a Java test class `name` (`public class … extends
/// LuceneTestCase`) with `methods`.
pub fn class(name: &str, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// [`class`] with class-level annotations: `annotate` adds them to the
/// builder (`@Monster`, `@TimeoutSuite`, …).
pub fn class_annotated(
    name: &str,
    methods: Vec<Method>,
    annotate: impl FnOnce(
        randomizedtesting::jdk::reflect::ClassBuilder,
    ) -> randomizedtesting::jdk::reflect::ClassBuilder,
) -> Arc<Class> {
    let mut builder = annotate(
        Class::builder(name)
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ())),
    );
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// One suite of a binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Runs the suite `class` builds (on the seed `tests.seed` names, or on the
/// first `RUCENE_SUITE_SEEDS` seeds) and fails with every failure.
pub fn run(class: fn() -> Arc<Class>) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_seeds::run_suite(class);
}

/// `random()`.
pub fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `VERBOSE`.
pub fn verbose() -> bool {
    *VERBOSE
}

/// `assertEquals(message, expected, actual)` over values Java compares with
/// `equals` and prints with `toString` (here `Debug`).
pub fn assert_equals_msg<T: PartialEq + Debug + ?Sized>(
    message: Option<&str>,
    expected: &T,
    actual: &T,
) {
    if expected != actual {
        let prefix = match message {
            Some(m) if !m.is_empty() => format!("{m} "),
            _ => String::new(),
        };
        Assert::fail(&format!(
            "{prefix}expected:<{expected:?}> but was:<{actual:?}>"
        ))
    }
}

/// `assertEquals(expected, actual)` over `equals`/`toString` values.
pub fn assert_equals<T: PartialEq + Debug + ?Sized>(expected: &T, actual: &T) {
    assert_equals_msg(None, expected, actual)
}

/// `assertNotEquals(unexpected, actual)`.
pub fn assert_not_equals<T: PartialEq + Debug + ?Sized>(unexpected: &T, actual: &T) {
    if unexpected == actual {
        Assert::fail(&format!("Values should be different. Actual: {actual:?}"))
    }
}

/// `assertArrayEquals(message, expecteds, actuals)`.
pub fn assert_array_equals_msg<T: PartialEq + Debug>(
    message: Option<&str>,
    expecteds: &[T],
    actuals: &[T],
) {
    let header = match message {
        Some(m) if !m.is_empty() => format!("{m}: "),
        _ => String::new(),
    };
    if expecteds.len() != actuals.len() {
        Assert::fail(&format!(
            "{header}array lengths differed, expected.length={} actual.length={}",
            expecteds.len(),
            actuals.len()
        ))
    }
    for (i, (e, a)) in expecteds.iter().zip(actuals).enumerate() {
        if e != a {
            Assert::fail(&format!(
                "{header}arrays first differed at element [{i}]; expected:<{e:?}> but was:<{a:?}>"
            ))
        }
    }
}

/// `assertArrayEquals(expecteds, actuals)`.
pub fn assert_array_equals<T: PartialEq + Debug>(expecteds: &[T], actuals: &[T]) {
    assert_array_equals_msg(None, expecteds, actuals)
}

/// `assertNull(object)`.
pub fn assert_null<T: Debug>(object: &Option<T>) {
    if let Some(o) = object {
        Assert::fail(&format!("expected null, but was:<{o:?}>"))
    }
}

/// `assertNotNull(object)`.
pub fn assert_not_null<T>(object: &Option<T>) {
    if object.is_none() {
        Throwable::with_message(randomizedtesting::jdk::exception::ASSERTION_ERROR, None).throw()
    }
}

/// `assertEquals(message, expected, actual, delta)` over `double`s.
pub fn assert_equals_delta(expected: f64, actual: f64, delta: f64) {
    Assert::assert_equals_double_msg(None, expected, actual, delta)
}

/// `throw new AssertionError(message)` as a `Result` error, for bodies that
/// `return` it.
pub fn assertion_error(message: impl Into<String>) -> LuceneError {
    LuceneError::AssertionError(message.into())
}

/// `RamUsageTester.ramUsed(object)` over a Rucene type that has no
/// [`RamObject`](rucene_test_framework::util::ram_usage_tester::RamObject)
/// model of its Java object graph yet: the measurement cannot be made, so
/// the test fails with an `UnsupportedOperationException` naming the type.
///
/// **Divergence from Lucene 10.5.0.** Java walks any object by reflection;
/// the port measures only the types that describe their Java layout, and
/// `type_name` does not yet.
pub fn ram_used_unmodelled(type_name: &str) -> i64 {
    Throwable::new(
        "java.lang.UnsupportedOperationException",
        format!("RamUsageTester.ramUsed: no RamObject model of {type_name}"),
    )
    .throw()
}

/// `String.valueOf(int[])`-free rendering of `Arrays.toString(int[])`.
pub fn arrays_to_string<T: std::fmt::Display>(a: &[T]) -> String {
    let parts: Vec<String> = a.iter().map(|v| v.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

#[allow(unused_imports)]
pub use randomizedtesting::jdk::util::shuffle;

/// `org.apache.lucene.util.automaton.TooComplexToDeterminizeException`.
pub const TOO_COMPLEX_TO_DETERMINIZE: &str =
    "org.apache.lucene.util.automaton.TooComplexToDeterminizeException";

/// A call Java declares unchecked (`Operations.determinize` and the other
/// operations that may throw `TooComplexToDeterminizeException`): its value,
/// or that exception thrown.
pub fn det<T>(r: rucene::util::automaton::DeterminizeResult<T>) -> T {
    r.unwrap_or_else(|e| Throwable::new(TOO_COMPLEX_TO_DETERMINIZE, e.to_string()).throw())
}

/// A Rucene call whose Java counterpart throws an unchecked exception (or
/// one the test method declares): its value, or the error thrown as the Java
/// exception it stands for.
pub fn unchecked<T>(r: Result<T>) -> T {
    r.unwrap_or_else(|e| rethrow_java(e))
}

/// `try { body } catch (class e) { … }`: the body's value, or the throwable
/// it threw when that is an instance of `class`; any other throwable
/// propagates.
pub fn try_catch<T>(class: &str, body: impl FnOnce() -> T) -> std::result::Result<T, Throwable> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(v) => Ok(v),
        Err(payload) => {
            let t = Throwable::from_panic(payload);
            if t.is_instance(class) {
                Err(t)
            } else {
                t.throw()
            }
        }
    }
}

/// `expectThrows(class, runnable)` over a body that throws (panics) rather
/// than returning an error: the throwable, which must be an instance of
/// `class`.
pub fn expect_throws_panic(class: &str, body: impl FnOnce()) -> Throwable {
    LuceneTestCase::expect_throws(class, || {
        body();
        Ok(())
    })
}

/// The `org.hamcrest` matchers the ported tests use beyond
/// `rucene_test_framework::hamcrest`, as `assertThat(actual, matcher)`
/// calls: each throws `AssertionError` with Hamcrest's
/// `\nExpected: <description>\n     but: <mismatch>` message.
pub mod hamcrest {
    use std::fmt::Display;

    use randomizedtesting::junit::Assert;

    fn fail(expected: &str, but: &str) -> ! {
        Assert::fail(&format!("\nExpected: {expected}\n     but: {but}"))
    }

    /// `assertThat(actual, greaterThan(value))`.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn assert_greater_than<T: PartialOrd + Display>(actual: T, value: T) {
        if !(actual > value) {
            fail(
                &format!("a value greater than <{value}>"),
                &format!("<{actual}> was less than or equal to <{value}>"),
            )
        }
    }

    /// `assertThat(actual, equalTo(value))`.
    pub fn assert_equal_to<T: PartialEq + Display>(actual: T, value: T) {
        if actual != value {
            fail(&format!("<{value}>"), &format!("was <{actual}>"))
        }
    }

    /// `assertThat(actual, containsString(substring))`.
    pub fn assert_contains_string(actual: Option<&str>, substring: &str) {
        match actual {
            Some(a) if a.contains(substring) => {}
            Some(a) => fail(
                &format!("a string containing \"{substring}\""),
                &format!("was \"{a}\""),
            ),
            None => fail(&format!("a string containing \"{substring}\""), "was null"),
        }
    }

    /// `assertThat(actual, hasToString(not(emptyString())))`.
    pub fn assert_has_non_empty_to_string(to_string: &str) {
        if to_string.is_empty() {
            fail("with toString() not an empty string", "toString() was \"\"")
        }
    }
}
