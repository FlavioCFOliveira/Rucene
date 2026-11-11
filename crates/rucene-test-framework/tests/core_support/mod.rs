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
use rucene::error::{LuceneError, LuceneErrorKind, Result};
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
    LuceneError::from(LuceneErrorKind::AssertionError(message.into()))
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
/// or that exception thrown, as the core's
/// `LuceneErrorKind::TooComplexToDeterminize` maps it.
pub fn det<T>(r: rucene::util::automaton::DeterminizeResult<T>) -> T {
    unchecked(r.map_err(rucene::error::LuceneError::from))
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

    /// `assertThat(actual, lessThan(value))`.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn assert_less_than<T: PartialOrd + Display>(actual: T, value: T) {
        if !(actual < value) {
            fail(
                &format!("a value less than <{value}>"),
                &format!("<{actual}> was greater than or equal to <{value}>"),
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

    /// `assertThat(reason, actual, matcher)` failing with `expected` and
    /// `but` descriptions.
    pub fn fail_that(reason: &str, expected: &str, but: &str) -> ! {
        Assert::fail(&format!("{reason}\nExpected: {expected}\n     but: {but}"))
    }

    /// `assertThat(collection, hasSize(size))`.
    pub fn assert_has_size(actual_size: usize, size: usize) {
        if actual_size != size {
            fail(
                &format!("a collection with size <{size}>"),
                &format!("collection size was <{actual_size}>"),
            )
        }
    }

    /// `assertThat(array, arrayWithSize(size))`.
    pub fn assert_array_with_size(actual_size: usize, size: usize) {
        if actual_size != size {
            fail(
                &format!("an array with size <{size}>"),
                &format!("array size was <{actual_size}>"),
            )
        }
    }

    /// `assertThat(collection, empty())`.
    pub fn assert_empty(actual_size: usize) {
        if actual_size != 0 {
            fail(
                "an empty collection",
                &format!("collection size was <{actual_size}>"),
            )
        }
    }

    /// `assertThat(value, notNullValue())`.
    pub fn assert_not_null_value(is_null: bool) {
        if is_null {
            fail("not null", "was null")
        }
    }

    /// `assertThat(value, nullValue())`.
    pub fn assert_null_value<T: std::fmt::Debug>(actual: &Option<T>) {
        if let Some(a) = actual {
            fail("null", &format!("was {a:?}"))
        }
    }

    /// `assertThat(actual, not(equalTo(value)))`.
    pub fn assert_not_equal_to<T: PartialEq + Display>(actual: T, value: T) {
        if actual == value {
            fail(&format!("not <{value}>"), &format!("was <{actual}>"))
        }
    }

    /// `assertThat(array, arrayContaining(items...))`: same elements, same order.
    pub fn assert_array_containing(actual: &[String], items: &[&str]) {
        if actual.len() != items.len() || actual.iter().zip(items).any(|(a, i)| a != i) {
            fail(&format!("{items:?}"), &format!("was {actual:?}"))
        }
    }

    /// `assertThat(reason, actual, containsInAnyOrder(items...))`.
    pub fn assert_contains_in_any_order(reason: &str, actual: &[String], items: &[&str]) {
        let mut remaining: Vec<&str> = items.to_vec();
        let mut ok = actual.len() == items.len();
        if ok {
            for a in actual {
                match remaining.iter().position(|i| i == a) {
                    Some(p) => {
                        remaining.remove(p);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            Assert::fail(&format!(
                "{reason}\nExpected: iterable with items {items:?} in any order\n     but: was {actual:?}"
            ))
        }
    }

    /// `assertThat(actual, hasToString(not(emptyString())))`.
    pub fn assert_has_non_empty_to_string(to_string: &str) {
        if to_string.is_empty() {
            fail("with toString() not an empty string", "toString() was \"\"")
        }
    }
}

/// `java.lang.IllegalArgumentException`.
pub const IAE: &str = "java.lang.IllegalArgumentException";

/// `expectThrows(IllegalArgumentException.class, runnable).getMessage()`
/// (`""` for a `null` message, which no `contains` check matches).
pub fn iae_message<T>(runnable: impl FnOnce() -> Result<T>) -> String {
    rucene_test_framework::util::java_exceptions::expect_throws(IAE, runnable)
        .message()
        .unwrap_or_default()
        .to_string()
}

/// `assertTrue(message.contains(part))`.
pub fn assert_contains(message: &str, part: &str) {
    Assert::assert_true_msg(message, message.contains(part))
}

/// `java.lang.Number`'s conversions over `IndexableField.numericValue()`:
/// Java dereferences the returned `Number`, so a `null` (`None`) throws
/// `NullPointerException`; the narrowing and widening follow Java's casts
/// (Rust `as` wraps integers and saturates floats exactly as Java does).
pub mod java_number {
    use randomizedtesting::jdk::exception::Throwable;
    use rucene::document::NumericValue;

    fn npe() -> ! {
        Throwable::new(
            "java.lang.NullPointerException",
            "Cannot invoke \"java.lang.Number\" method because the value is null",
        )
        .throw()
    }

    /// `Number.longValue()`.
    pub fn long_value(n: Option<NumericValue>) -> i64 {
        match n {
            Some(NumericValue::Int(v)) => v as i64,
            Some(NumericValue::Long(v)) => v,
            Some(NumericValue::Float(v)) => v as i64,
            Some(NumericValue::Double(v)) => v as i64,
            None => npe(),
        }
    }

    /// `Number.intValue()`.
    pub fn int_value(n: Option<NumericValue>) -> i32 {
        match n {
            Some(NumericValue::Int(v)) => v,
            Some(NumericValue::Long(v)) => v as i32,
            Some(NumericValue::Float(v)) => v as i32,
            Some(NumericValue::Double(v)) => v as i32,
            None => npe(),
        }
    }

    /// `Number.floatValue()`.
    pub fn float_value(n: Option<NumericValue>) -> f32 {
        match n {
            Some(NumericValue::Int(v)) => v as f32,
            Some(NumericValue::Long(v)) => v as f32,
            Some(NumericValue::Float(v)) => v,
            Some(NumericValue::Double(v)) => v as f32,
            None => npe(),
        }
    }

    /// `Number.doubleValue()`.
    pub fn double_value(n: Option<NumericValue>) -> f64 {
        match n {
            Some(NumericValue::Int(v)) => v as f64,
            Some(NumericValue::Long(v)) => v as f64,
            Some(NumericValue::Float(v)) => v as f64,
            Some(NumericValue::Double(v)) => v,
            None => npe(),
        }
    }
}

/// `assertArrayEquals(message, float[] expecteds, float[] actuals, delta)`
/// (and the `double[]` overload, widened): lengths first, then each element
/// as `assertEquals(float, float, delta)` does (equal when `Float.compare`
/// says so, or within `delta`).
pub fn assert_array_equals_delta_msg(
    message: Option<&str>,
    expecteds: &[f64],
    actuals: &[f64],
    delta: f64,
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
    for (i, (&e, &a)) in expecteds.iter().zip(actuals).enumerate() {
        let same = e.to_bits() == a.to_bits() || (e.is_nan() && a.is_nan());
        if !same && !((e - a).abs() <= delta) {
            Assert::fail(&format!(
                "{header}arrays first differed at element [{i}]; expected:<{e}> but was:<{a}>"
            ))
        }
    }
}

/// `assertArrayEquals(float[] expecteds, float[] actuals, float delta)`.
pub fn assert_array_equals_f32(expecteds: &[f32], actuals: &[f32], delta: f32) {
    let e: Vec<f64> = expecteds.iter().map(|&v| v as f64).collect();
    let a: Vec<f64> = actuals.iter().map(|&v| v as f64).collect();
    assert_array_equals_delta_msg(None, &e, &a, delta as f64)
}

/// [`test`] over a closure: a test method inherited from an abstract Java
/// base class, bound to the concrete class that declares it.
pub fn test_closure(name: &str, body: impl Fn() -> Result<()> + Send + Sync + 'static) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        })
}
