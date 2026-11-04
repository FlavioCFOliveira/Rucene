//! The part of Hamcrest 2.2 (`org.hamcrest`) `LuceneTestCase.assertThat`
//! uses: [`Matcher`], [`Description`], `MatcherAssert.assertThat` and the core
//! `equalTo` and `is` matchers.
//!
//! **Divergence.** Hamcrest is a third-party Java library (BSD-3-Clause); the
//! port adds no dependency, so this module reproduces the behaviour the test
//! framework relies on from Hamcrest's javadoc, verified against
//! `hamcrest-2.2.jar` (`ltc_statics.tsv`): the message of the
//! `AssertionError` and the way values are described
//! (`BaseDescription.appendValue`).

use std::fmt;

use randomizedtesting::jdk::exception::{self, ASSERTION_ERROR};
use randomizedtesting::jdk::lang::{double_to_string, float_to_string};

/// `org.hamcrest.Description`, as a `StringDescription`.
#[derive(Debug, Default, Clone)]
pub struct Description {
    text: String,
}

impl Description {
    /// `new StringDescription()`.
    pub fn new() -> Description {
        Description::default()
    }

    /// `appendText(text)`.
    pub fn append_text(&mut self, text: &str) -> &mut Description {
        self.text.push_str(text);
        self
    }

    /// `appendValue(value)`.
    pub fn append_value(&mut self, value: &dyn HamcrestValue) -> &mut Description {
        self.text.push_str(&value.describe_value());
        self
    }

    /// `appendDescriptionOf(matcher)`.
    pub fn append_description_of<T: ?Sized>(
        &mut self,
        matcher: &dyn Matcher<T>,
    ) -> &mut Description {
        matcher.describe_to(self);
        self
    }
}

impl fmt::Display for Description {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// How `BaseDescription.appendValue` describes a value: a string in Java
/// syntax between quotes, a `short` as `<1s>`, a `long` as `<1L>`, a `float`
/// as `<1.0F>`, anything else as `<toString()>`.
pub trait HamcrestValue {
    /// The description of the value.
    fn describe_value(&self) -> String;
}

/// `toJavaSyntax(String)`: the string between double quotes, with `"`, `\n`,
/// `\r`, `\t` and `\\` escaped.
fn java_syntax(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl HamcrestValue for str {
    fn describe_value(&self) -> String {
        java_syntax(self)
    }
}

impl HamcrestValue for String {
    fn describe_value(&self) -> String {
        java_syntax(self)
    }
}

impl HamcrestValue for &str {
    fn describe_value(&self) -> String {
        java_syntax(self)
    }
}

impl HamcrestValue for i32 {
    fn describe_value(&self) -> String {
        format!("<{self}>")
    }
}

impl HamcrestValue for i16 {
    fn describe_value(&self) -> String {
        format!("<{self}s>")
    }
}

impl HamcrestValue for i64 {
    fn describe_value(&self) -> String {
        format!("<{self}L>")
    }
}

impl HamcrestValue for f32 {
    fn describe_value(&self) -> String {
        format!("<{}F>", float_to_string(*self))
    }
}

impl HamcrestValue for f64 {
    fn describe_value(&self) -> String {
        format!("<{}>", double_to_string(*self))
    }
}

impl HamcrestValue for bool {
    fn describe_value(&self) -> String {
        format!("<{self}>")
    }
}

impl HamcrestValue for char {
    fn describe_value(&self) -> String {
        format!("\"{self}\"")
    }
}

/// `org.hamcrest.Matcher<T>`.
pub trait Matcher<T: ?Sized>: Send + Sync {
    /// `matches(item)`.
    fn matches(&self, item: &T) -> bool;

    /// `describeTo(description)`.
    fn describe_to(&self, description: &mut Description);

    /// `describeMismatch(item, description)`: `BaseMatcher`'s default, `was `
    /// and the value.
    fn describe_mismatch(&self, item: &T, description: &mut Description);
}

/// `org.hamcrest.core.IsEqual`: `equalTo(operand)`.
pub struct IsEqual<T> {
    expected: T,
}

/// `CoreMatchers.equalTo(operand)`.
pub fn equal_to<T>(operand: T) -> IsEqual<T> {
    IsEqual { expected: operand }
}

impl<T: PartialEq + HamcrestValue + Send + Sync> Matcher<T> for IsEqual<T> {
    fn matches(&self, item: &T) -> bool {
        *item == self.expected
    }

    fn describe_to(&self, description: &mut Description) {
        description.append_value(&self.expected);
    }

    fn describe_mismatch(&self, item: &T, description: &mut Description) {
        description.append_text("was ").append_value(item);
    }
}

/// `org.hamcrest.core.Is`: `is(matcher)`, decorating another matcher.
pub struct Is<M> {
    matcher: M,
}

/// `CoreMatchers.is(matcher)`.
pub fn is<M>(matcher: M) -> Is<M> {
    Is { matcher }
}

impl<T: ?Sized, M: Matcher<T>> Matcher<T> for Is<M> {
    fn matches(&self, item: &T) -> bool {
        self.matcher.matches(item)
    }

    fn describe_to(&self, description: &mut Description) {
        description.append_text("is ");
        self.matcher.describe_to(description);
    }

    fn describe_mismatch(&self, item: &T, description: &mut Description) {
        self.matcher.describe_mismatch(item, description);
    }
}

/// `org.hamcrest.MatcherAssert`.
pub struct MatcherAssert;

impl MatcherAssert {
    /// `assertThat(actual, matcher)`: `assertThat("", actual, matcher)`.
    ///
    /// # Panics
    ///
    /// As [`assert_that_reason`](Self::assert_that_reason).
    pub fn assert_that<T: ?Sized>(actual: &T, matcher: &dyn Matcher<T>) {
        Self::assert_that_reason("", actual, matcher)
    }

    /// `assertThat(reason, actual, matcher)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` with `reason`, a line break, `Expected: ` and the
    /// matcher's description, a line break, `     but: ` and the mismatch,
    /// when `actual` does not match.
    pub fn assert_that_reason<T: ?Sized>(reason: &str, actual: &T, matcher: &dyn Matcher<T>) {
        if !matcher.matches(actual) {
            let mut description = Description::new();
            description
                .append_text(reason)
                .append_text("\n")
                .append_text("Expected: ")
                .append_description_of(matcher)
                .append_text("\n")
                .append_text("     but: ");
            matcher.describe_mismatch(actual, &mut description);
            exception::throw(ASSERTION_ERROR, description.to_string())
        }
    }
}
