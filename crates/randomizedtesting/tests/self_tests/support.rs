//! Declaration helpers shared by the ported test classes, the
//! `com.carrotsearch.randomizedtesting.RandomizedTest` superclass they
//! extend, and the subset of AssertJ 2.2.0 (`org.assertj.core.api`,
//! Apache-2.0) and `org.junit.Assert` the Java tests call.
//!
//! **Divergence.** AssertJ is a test dependency of randomized-runner, not
//! part of it: its assertions are rebuilt here for the calls the suite makes.
//! Where a Java test of the suite *expects* an AssertJ failure the message
//! is AssertJ 2.2.0's (`Expecting empty but was:<…>`, `Expecting:\n <…>\nto
//! contain:\n <…> `); the other failure messages state the same facts in
//! AssertJ's words without its exact layout, since no Java test observes
//! them.

#![allow(dead_code)]

use std::fmt::Display;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR};
use randomizedtesting::jdk::reflect::{modifier, Class, ClassBuilder, Constructor, Method};
use randomizedtesting::junit::{Assert, Test};

/// `com.carrotsearch.randomizedtesting.`.
pub const P: &str = "com.carrotsearch.randomizedtesting.";

/// A class builder for `com.carrotsearch.randomizedtesting.<name>`, public.
pub fn cls(name: &str) -> ClassBuilder {
    Class::builder(format!("{P}{name}")).modifiers(modifier::PUBLIC)
}

/// A `public static` nested class (`Outer$Inner`).
pub fn nested(outer: &str, inner: &str) -> ClassBuilder {
    Class::builder(format!("{P}{outer}${inner}")).modifiers(modifier::PUBLIC | modifier::STATIC)
}

/// The implicit public no-argument constructor.
pub fn unit() -> Constructor {
    Constructor::no_args(|| ())
}

/// `@Test public void name() { body }`.
pub fn test(name: &str, body: impl Fn() + Send + Sync + 'static) -> Method {
    Method::new(name).annotate(Test::new()).body_any(body)
}

/// A public instance method with one annotation.
pub fn hook(
    name: &str,
    ann: impl randomizedtesting::jdk::reflect::JavaAnnotation,
    body: impl Fn() + Send + Sync + 'static,
) -> Method {
    Method::new(name).annotate(ann).body_any(body)
}

/// A `public static` method with one annotation.
pub fn static_hook(
    name: &str,
    ann: impl randomizedtesting::jdk::reflect::JavaAnnotation,
    body: impl Fn() + Send + Sync + 'static,
) -> Method {
    Method::new(name).annotate(ann).static_body(body)
}

/// `com.carrotsearch.randomizedtesting.RandomizedTest`: the superclass the
/// suites extend (`RandomizedTest::class()`).
pub static RANDOMIZED_TEST: LazyLock<Arc<Class>> =
    LazyLock::new(randomizedtesting::RandomizedTest::class);

/// `java.lang.Error`.
pub const ERROR: &str = "java.lang.Error";

/// `throw new <class>(message)`.
pub fn throw(class: &str, message: &str) -> ! {
    Throwable::new(class, message).throw()
}

/// `throw new <class>()`.
pub fn throw_no_message(class: &str) -> ! {
    Throwable::with_message(class, None).throw()
}

// -----------------------------------------------------------------------------
// org.junit.Assert members the suite calls.
// -----------------------------------------------------------------------------

/// `assertEquals(message, expected, actual)` on `String`s:
/// `message expected:<x> but was:<y>` (JUnit's `ComparisonFailure` layout
/// without its common-part compaction).
pub fn assert_equals_str(message: Option<&str>, expected: &str, actual: &str) {
    if expected != actual {
        let prefix = message.map(|m| format!("{m} ")).unwrap_or_default();
        Throwable::new(
            "org.junit.ComparisonFailure",
            format!("{prefix}expected:<{expected}> but was:<{actual}>"),
        )
        .throw()
    }
}

/// `assertEquals(expected, actual)` on objects.
pub fn assert_equals<T: PartialEq + Display>(expected: T, actual: T) {
    Assert::assert_equals(expected, actual)
}

/// `assertEquals(message, expected, actual)` on objects.
pub fn assert_equals_msg<T: PartialEq + Display>(message: &str, expected: T, actual: T) {
    Assert::assert_equals_msg(Some(message), expected, actual)
}

/// `assertTrue(condition)`.
pub fn assert_true(c: bool) {
    Assert::assert_true(c)
}

/// `assertTrue(message, condition)`.
pub fn assert_true_msg(message: &str, c: bool) {
    Assert::assert_true_msg(message, c)
}

/// `assertFalse(condition)`.
pub fn assert_false(c: bool) {
    Assert::assert_false(c)
}

/// `assertNotNull(object)`.
pub fn assert_not_null<T>(v: Option<T>) -> T {
    match v {
        Some(v) => v,
        None => Assert::fail_without_message(),
    }
}

/// `fail()`.
pub fn fail() -> ! {
    Assert::fail_without_message()
}

/// `fail(message)`.
pub fn fail_msg(message: &str) -> ! {
    Assert::fail(message)
}

// -----------------------------------------------------------------------------
// AssertJ 2.2.0 (`Assertions.assertThat(...)`), the subset the suite calls.
// -----------------------------------------------------------------------------

fn assertj_fail(description: &Option<String>, message: String) -> ! {
    let m = match description {
        Some(d) => format!("[{d}] {message}"),
        None => message,
    };
    Throwable::new(ASSERTION_ERROR, m).throw()
}

/// `Assertions.fail(message)`.
pub fn assertj_fail_msg(message: &str) -> ! {
    Throwable::new(ASSERTION_ERROR, message).throw()
}

/// `assertThat(boolean)`.
pub struct BoolAssert(bool, Option<String>);

/// `Assertions.assertThat(boolean)`.
pub fn assert_that_bool(v: bool) -> BoolAssert {
    BoolAssert(v, None)
}

impl BoolAssert {
    /// `as(description)`.
    pub fn as_(mut self, d: impl Into<String>) -> BoolAssert {
        self.1 = Some(d.into());
        self
    }
    /// `isTrue()`.
    pub fn is_true(&self) {
        if !self.0 {
            assertj_fail(
                &self.1,
                "\nExpecting:\n <false>\nto be equal to:\n <true>\nbut was not.".into(),
            )
        }
    }
    /// `isFalse()`.
    pub fn is_false(&self) {
        if self.0 {
            assertj_fail(
                &self.1,
                "\nExpecting:\n <true>\nto be equal to:\n <false>\nbut was not.".into(),
            )
        }
    }
}

/// `assertThat(String)`.
pub struct StrAssert(Option<String>, Option<String>);

/// `Assertions.assertThat(String)`.
pub fn assert_that_str(v: impl Into<String>) -> StrAssert {
    StrAssert(Some(v.into()), None)
}

/// `Assertions.assertThat((String) null)`.
pub fn assert_that_opt_str(v: Option<String>) -> StrAssert {
    StrAssert(v, None)
}

impl StrAssert {
    /// `as(description)`.
    pub fn as_(mut self, d: impl Into<String>) -> StrAssert {
        self.1 = Some(d.into());
        self
    }
    fn actual(&self) -> &str {
        self.0.as_deref().unwrap_or("null")
    }
    /// `isEqualTo(expected)`.
    pub fn is_equal_to(&self, expected: &str) -> &Self {
        if self.0.as_deref() != Some(expected) {
            assertj_fail(
                &self.1,
                format!("expected:<\"{expected}\"> but was:<\"{}\">", self.actual()),
            )
        }
        self
    }
    /// `isNotEqualTo(other)`.
    pub fn is_not_equal_to(&self, other: &str) -> &Self {
        if self.0.as_deref() == Some(other) {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpecting:\n <\"{}\">\nnot to be equal to:\n <\"{other}\">\n",
                    self.actual()
                ),
            )
        }
        self
    }
    /// `contains(sequence)`: AssertJ 2.2.0's `ShouldContainCharSequence`.
    pub fn contains(&self, s: &str) -> &Self {
        if !self.actual().contains(s) {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpecting:\n <\"{}\">\nto contain:\n <\"{s}\"> ",
                    self.actual()
                ),
            )
        }
        self
    }
    /// `doesNotContain(sequence)`.
    pub fn does_not_contain(&self, s: &str) -> &Self {
        if self.actual().contains(s) {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpecting:\n <\"{}\">\nnot to contain:\n <\"{s}\">\n",
                    self.actual()
                ),
            )
        }
        self
    }
    /// `isNull()`.
    pub fn is_null(&self) {
        if let Some(v) = &self.0 {
            assertj_fail(&self.1, format!("expected:<null> but was:<\"{v}\">"))
        }
    }
    /// `startsWith(prefix)`.
    pub fn starts_with(&self, p: &str) -> &Self {
        if !self.actual().starts_with(p) {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpecting:\n <\"{}\">\nto start with:\n <\"{p}\">\n",
                    self.actual()
                ),
            )
        }
        self
    }
}

/// `assertThat(Iterable)` / `assertThat(List)` over displayable elements.
pub struct ListAssert(Vec<String>, Option<String>);

/// `Assertions.assertThat(iterable)`: the elements' `toString()`.
pub fn assert_that_list<T: Display>(v: impl IntoIterator<Item = T>) -> ListAssert {
    ListAssert(v.into_iter().map(|x| x.to_string()).collect(), None)
}

/// AssertJ's `toString` of a list of elements (strings quoted).
fn list_repr(v: &[String], quote: bool) -> String {
    let items: Vec<String> = v
        .iter()
        .map(|s| if quote { format!("\"{s}\"") } else { s.clone() })
        .collect();
    format!("[{}]", items.join(", "))
}

impl ListAssert {
    /// `as(description)`.
    pub fn as_(mut self, d: impl Into<String>) -> ListAssert {
        self.1 = Some(d.into());
        self
    }
    /// `isEmpty()`: AssertJ 2.2.0's `ShouldBeEmpty` (elements unquoted, as
    /// `Failure.toString()` prints them).
    pub fn is_empty(&self) {
        if !self.0.is_empty() {
            assertj_fail(
                &self.1,
                format!("\nExpecting empty but was:<{}>", list_repr(&self.0, false)),
            )
        }
    }
    /// `hasSize(n)`.
    pub fn has_size(&self, n: usize) -> &Self {
        if self.0.len() != n {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpected size:<{n}> but was:<{}> in:\n<{}>",
                    self.0.len(),
                    list_repr(&self.0, false)
                ),
            )
        }
        self
    }
    /// `contains(values…)`.
    pub fn contains(&self, values: &[&str]) -> &Self {
        let missing: Vec<String> = values
            .iter()
            .filter(|v| !self.0.iter().any(|x| x == *v))
            .map(|v| v.to_string())
            .collect();
        if !missing.is_empty() {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpecting:\n <{}>\nto contain:\n <{}>\nbut could not find:\n <{}>\n",
                    list_repr(&self.0, true),
                    list_repr(
                        &values.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                        true
                    ),
                    list_repr(&missing, true)
                ),
            )
        }
        self
    }
    /// `containsOnly(values…)`: the same elements, any order, duplicates
    /// ignored.
    pub fn contains_only(&self, values: &[&str]) -> &Self {
        let unexpected: Vec<String> = self
            .0
            .iter()
            .filter(|x| !values.contains(&x.as_str()))
            .cloned()
            .collect();
        let missing: Vec<String> = values
            .iter()
            .filter(|v| !self.0.iter().any(|x| x == *v))
            .map(|v| v.to_string())
            .collect();
        if !unexpected.is_empty() || !missing.is_empty() {
            assertj_fail(&self.1, format!("\nExpecting:\n <{}>\nto contain only:\n <{}>\nelements not found:\n <{}>\nand elements not expected:\n <{}>\n", list_repr(&self.0, true), list_repr(&values.iter().map(|s| s.to_string()).collect::<Vec<_>>(), true), list_repr(&missing, true), list_repr(&unexpected, true)))
        }
        self
    }
    /// `containsExactly(values…)`.
    pub fn contains_exactly(&self, values: &[&str]) -> &Self {
        if self.0.len() != values.len() || self.0.iter().zip(values).any(|(a, b)| a != b) {
            assertj_fail(&self.1, format!("\nActual and expected should have same elements in the same order:\n actual:<{}>\n expected:<{}>\n", list_repr(&self.0, true), list_repr(&values.iter().map(|s| s.to_string()).collect::<Vec<_>>(), true)))
        }
        self
    }
    /// `isEqualTo(list)`.
    pub fn is_equal_to(&self, other: &[String]) -> &Self {
        if self.0 != other {
            assertj_fail(
                &self.1,
                format!(
                    "expected:<{}> but was:<{}>",
                    list_repr(other, true),
                    list_repr(&self.0, true)
                ),
            )
        }
        self
    }
}

/// `assertThat(int)` / `assertThat(long)`.
pub struct NumAssert(i64, Option<String>);

/// `Assertions.assertThat(number)`.
pub fn assert_that_num(v: impl Into<i64>) -> NumAssert {
    NumAssert(v.into(), None)
}

impl NumAssert {
    /// `isEqualTo(expected)`.
    pub fn is_equal_to(&self, expected: i64) -> &Self {
        if self.0 != expected {
            assertj_fail(
                &self.1,
                format!("expected:<[{expected}]> but was:<[{}]>", self.0),
            )
        }
        self
    }
    /// `isGreaterThan(other)`.
    pub fn is_greater_than(&self, other: i64) -> &Self {
        if self.0 <= other {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpecting:\n <{}>\nto be greater than:\n <{other}> ",
                    self.0
                ),
            )
        }
        self
    }
    /// `isLessThanOrEqualTo(other)`.
    pub fn is_less_than_or_equal_to(&self, other: i64) -> &Self {
        if self.0 > other {
            assertj_fail(
                &self.1,
                format!(
                    "\nExpecting:\n <{}>\nto be less than or equal to:\n <{other}> ",
                    self.0
                ),
            )
        }
        self
    }
}

/// `Assertions.assertThat(float/double).isNotNaN()`.
pub fn assert_not_nan(v: f64) {
    if v.is_nan() {
        assertj_fail(
            &None,
            "\nExpecting:\n <NaN>\nnot to be equal to:\n <NaN>\n".into(),
        )
    }
}

/// `Assertions.assertThat(throwable).isInstanceOf(class)`.
pub fn assert_instance_of(t: &Throwable, class: &str) {
    if !t.is_instance(class) {
        assertj_fail(&None, format!("\nExpecting:\n <{t}>\nto be an instance of:\n <{class}>\nbut was instance of:\n <{}>", t.class_name()))
    }
}

// -----------------------------------------------------------------------------
// Masks shared with the driver.
// -----------------------------------------------------------------------------

/// Replaces the decimal digits after each `prefix` with `N`.
pub fn mask_digits(s: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(prefix) {
        out.push_str(&rest[..i + prefix.len()]);
        rest = &rest[i + prefix.len()..];
        let n = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if n > 0 {
            out.push('N');
            rest = &rest[n..];
        }
    }
    out.push_str(rest);
    out
}

/// Replaces the hexadecimal digits after each `prefix` with `H`.
pub fn mask_hex_after(s: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(prefix) {
        out.push_str(&rest[..i + prefix.len()]);
        rest = &rest[i + prefix.len()..];
        let n = rest.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        out.push('H');
        rest = &rest[n..];
    }
    out.push_str(rest);
    out
}

/// `try { f } catch (Throwable t) { … }`: the throwable `f` threw.
pub fn try_catch<T>(f: impl FnOnce() -> T) -> Result<T, Throwable> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(Throwable::from_panic)
}

/// `RandomizedContext.current().getRandom()`.
pub fn get_random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    randomizedtesting::RandomizedContext::current().get_random()
}

/// `RandomizedContext.current().getRandomness().getSeed()`.
pub fn current_seed() -> i64 {
    randomizedtesting::RandomizedContext::current()
        .get_randomness()
        .expect("a context randomness")
        .get_seed()
}

/// The `$(...)` of `RandomizedTest`: an `Object[]` of the arguments.
pub fn args(
    values: Vec<randomizedtesting::jdk::reflect::Value>,
) -> randomizedtesting::jdk::reflect::Value {
    randomizedtesting::jdk::reflect::Value::Array(values)
}

/// `new JUnitCore().run(new RandomizedRunner(clazz))`: the constructor's
/// `InitializationError` is thrown, as Java's propagates.
pub fn run_randomized_runner(class: &Arc<Class>) -> Arc<randomizedtesting::junit::JUnitResult> {
    match randomizedtesting::RandomizedRunner::new(class) {
        Ok(r) => randomizedtesting::junit::JUnitCore::new()
            .run(&randomizedtesting::junit::Request::of_runner(Arc::new(r))),
        Err(e) => initialization_error(&e).throw(),
    }
}

/// The `InitializationError` throwable of a failed runner construction.
pub fn initialization_error(e: &randomizedtesting::junit::InitializationError) -> Throwable {
    let mut t = Throwable::with_message("org.junit.runners.model.InitializationError", None);
    if let Some(c) = e.causes().first() {
        t = Throwable::with_cause(
            "org.junit.runners.model.InitializationError",
            None,
            c.clone(),
        );
    }
    t
}

/// A `ClassRef` to `JUnit3MethodProvider`.
pub fn junit3_provider(
) -> randomizedtesting::annotations::ClassRef<dyn randomizedtesting::TestMethodProvider> {
    randomizedtesting::annotations::ClassRef::new(
        "com.carrotsearch.randomizedtesting.JUnit3MethodProvider",
        || {
            Arc::new(randomizedtesting::JUnit3MethodProvider)
                as Arc<dyn randomizedtesting::TestMethodProvider>
        },
    )
}
