//! The part of Hamcrest 2.2 (`org.hamcrest`) `LuceneTestCase.assertThat` and
//! `TestUtil.assertConsistent` use: [`Matcher`], [`Description`],
//! `MatcherAssert.assertThat`, the core `equalTo`, `is`, `not` and
//! `instanceOf` matchers, `OrderingComparison`'s `lessThanOrEqualTo` and
//! `greaterThanOrEqualTo`, and `IsArrayWithSize.arrayWithSize`.
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

/// `org.hamcrest.core.IsNot`: `not(matcher)`, the logical negation of
/// another matcher.
pub struct IsNot<M> {
    matcher: M,
}

/// `CoreMatchers.not(matcher)`.
pub fn not<M>(matcher: M) -> IsNot<M> {
    IsNot { matcher }
}

impl<T: HamcrestValue, M: Matcher<T>> Matcher<T> for IsNot<M> {
    fn matches(&self, item: &T) -> bool {
        !self.matcher.matches(item)
    }

    /// `"not "` and the negated matcher's description.
    fn describe_to(&self, description: &mut Description) {
        description.append_text("not ");
        self.matcher.describe_to(description);
    }

    /// `BaseMatcher.describeMismatch`: `was ` and the value (`IsNot` does not
    /// override it).
    fn describe_mismatch(&self, item: &T, description: &mut Description) {
        description.append_text("was ").append_value(item);
    }
}

/// What `IsInstanceOf` asks of a value: its run-time class, as
/// `getClass().getName()` reports it, and whether it is an instance of a
/// class (`Class.isInstance`), which a Rust value answers for itself.
pub trait JavaObject: HamcrestValue {
    /// `getClass().getName()`.
    fn java_class_name(&self) -> String;

    /// `Class.forName(class_name).isInstance(this)`: `true` for the value's
    /// own class and each of its superclasses and interfaces.
    fn is_instance_of(&self, class_name: &str) -> bool;
}

/// `org.hamcrest.core.IsInstanceOf`: `instanceOf(type)`.
pub struct IsInstanceOf {
    expected_class: String,
}

/// `CoreMatchers.instanceOf(type)`, with the class given by its name
/// (`Class.getName()`).
pub fn instance_of(class_name: &str) -> IsInstanceOf {
    IsInstanceOf {
        expected_class: class_name.to_string(),
    }
}

impl<T: JavaObject> Matcher<T> for IsInstanceOf {
    fn matches(&self, item: &T) -> bool {
        item.is_instance_of(&self.expected_class)
    }

    /// `"an instance of "` and the class name.
    fn describe_to(&self, description: &mut Description) {
        description
            .append_text("an instance of ")
            .append_text(&self.expected_class);
    }

    /// The value, `" is a "` and its class name (`IsInstanceOf.matches(Object,
    /// Description)`; a `null` item, `"null"`, has no Rust counterpart).
    fn describe_mismatch(&self, item: &T, description: &mut Description) {
        description
            .append_value(item)
            .append_text(&format!(" is a {}", item.java_class_name()));
    }
}

/// `org.hamcrest.number.OrderingComparison`: compares a value with an
/// expected one, matching when `actual.compareTo(expected)` has a sign in
/// `[min_compare, max_compare]`.
pub struct OrderingComparison<T> {
    expected: T,
    min_compare: i32,
    max_compare: i32,
}

/// `OrderingComparison.lessThanOrEqualTo(value)`.
pub fn less_than_or_equal_to<T>(value: T) -> OrderingComparison<T> {
    OrderingComparison {
        expected: value,
        min_compare: -1,
        max_compare: 0,
    }
}

/// `OrderingComparison.greaterThanOrEqualTo(value)`.
pub fn greater_than_or_equal_to<T>(value: T) -> OrderingComparison<T> {
    OrderingComparison {
        expected: value,
        min_compare: 0,
        max_compare: 1,
    }
}

/// `OrderingComparison.asText(comparison)`.
fn comparison_text(comparison: i32) -> &'static str {
    match comparison {
        c if c < 0 => "less than",
        0 => "equal to",
        _ => "greater than",
    }
}

impl<T: PartialOrd + HamcrestValue + Send + Sync> OrderingComparison<T> {
    /// `signum(actual.compareTo(expected))`.
    fn compare(&self, actual: &T) -> i32 {
        match actual.partial_cmp(&self.expected) {
            Some(std::cmp::Ordering::Less) => -1,
            Some(std::cmp::Ordering::Equal) | None => 0,
            Some(std::cmp::Ordering::Greater) => 1,
        }
    }
}

impl<T: PartialOrd + HamcrestValue + Send + Sync> Matcher<T> for OrderingComparison<T> {
    fn matches(&self, item: &T) -> bool {
        let c = self.compare(item);
        self.min_compare <= c && c <= self.max_compare
    }

    /// `"a value "`, the comparison (`"equal to or "` between the two words
    /// when the range covers 0 and one side) and the expected value.
    fn describe_to(&self, description: &mut Description) {
        description
            .append_text("a value ")
            .append_text(comparison_text(self.min_compare));
        if self.min_compare != self.max_compare {
            description
                .append_text(" or ")
                .append_text(comparison_text(self.max_compare));
        }
        description.append_text(" ").append_value(&self.expected);
    }

    /// The value, `" was "`, the comparison and the expected value.
    fn describe_mismatch(&self, item: &T, description: &mut Description) {
        description
            .append_value(item)
            .append_text(" was ")
            .append_text(comparison_text(self.compare(item)))
            .append_text(" ")
            .append_value(&self.expected);
    }
}

/// `org.hamcrest.collection.IsArrayWithSize`: `arrayWithSize(size)`, the
/// `FeatureMatcher` of the array's length with `equalTo(size)`.
pub struct IsArrayWithSize {
    size: IsEqual<i32>,
}

/// `IsArrayWithSize.arrayWithSize(size)`.
pub fn array_with_size(size: i32) -> IsArrayWithSize {
    IsArrayWithSize {
        size: equal_to(size),
    }
}

impl<E> Matcher<[E]> for IsArrayWithSize {
    fn matches(&self, item: &[E]) -> bool {
        self.size.matches(&(item.len() as i32))
    }

    /// `"an array with size "` and the size matcher's description.
    fn describe_to(&self, description: &mut Description) {
        description.append_text("an array with size ");
        self.size.describe_to(description);
    }

    /// `FeatureMatcher`: `"array size "` and the size matcher's mismatch.
    fn describe_mismatch(&self, item: &[E], description: &mut Description) {
        description.append_text("array size ");
        self.size
            .describe_mismatch(&(item.len() as i32), description);
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
