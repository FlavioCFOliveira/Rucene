//! Score explanations, ported from `org.apache.lucene.search.Explanation`.
//!
//! # Why this lives under `similarities`
//!
//! In Lucene, `Explanation`, `CollectionStatistics` and `TermStatistics` are
//! top-level classes of `org.apache.lucene.search`, not of
//! `org.apache.lucene.search.similarities`. They are ported here because the
//! similarity package is the first consumer to reach the crate and it cannot be
//! expressed without them; `org.apache.lucene.search` itself has not been
//! ported yet.
//!
//! This is a placement divergence only — the type, its API and its rendering
//! are faithful. [`crate::search`] re-exports them at
//! `crate::search::Explanation`, which is where they will stay once
//! `org.apache.lucene.search` arrives and this module can simply be moved.

#![deny(unsafe_code)]

use std::fmt;
use std::hash::{Hash, Hasher};

use super::java_fmt;

/// The value of an [`Explanation`] node.
///
/// Java types this field as `java.lang.Number` and Lucene boxes four different
/// primitives into it — `Float` (most scores and factors), `Double`
/// (`IndriDirichletSimilarity.java:66`), `Long` (document and term frequencies)
/// and `Integer` (`Axiomatic.java:139`, `Normalization.java:64`). The choice is
/// observable, because `Explanation`'s rendering and its `equals` both go
/// through the boxed type: `Explanation.match(1, …)` renders as `1` where
/// `Explanation.match(1f, …)` renders as `1.0`, and a boxed `Float` is never
/// equal to a boxed `Double`. This enum therefore keeps the four apart instead
/// of collapsing them into one floating-point type.
#[derive(Debug, Clone, Copy)]
pub enum ExplanationValue {
    /// A value Lucene boxed as `java.lang.Integer`.
    Int(i32),
    /// A value Lucene boxed as `java.lang.Long`.
    Long(i64),
    /// A value Lucene boxed as `java.lang.Float`.
    Float(f32),
    /// A value Lucene boxed as `java.lang.Double`.
    Double(f64),
}

impl ExplanationValue {
    /// Returns the value as an `f32`.
    ///
    /// Equivalent to `Number.floatValue()`, which Lucene calls on explanation
    /// values throughout the similarity package.
    pub fn float_value(self) -> f32 {
        match self {
            Self::Int(value) => value as f32,
            Self::Long(value) => value as f32,
            Self::Float(value) => value,
            Self::Double(value) => value as f32,
        }
    }

    /// Returns the value as an `f64`.
    ///
    /// Equivalent to `Number.doubleValue()`.
    pub fn double_value(self) -> f64 {
        match self {
            Self::Int(value) => f64::from(value),
            Self::Long(value) => value as f64,
            Self::Float(value) => f64::from(value),
            Self::Double(value) => value,
        }
    }
}

impl From<i32> for ExplanationValue {
    fn from(value: i32) -> Self {
        Self::Int(value)
    }
}

impl From<i64> for ExplanationValue {
    fn from(value: i64) -> Self {
        Self::Long(value)
    }
}

impl From<f32> for ExplanationValue {
    fn from(value: f32) -> Self {
        Self::Float(value)
    }
}

impl From<f64> for ExplanationValue {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

/// Canonicalizes NaN the way `Float.floatToIntBits` does, so that
/// `Float.equals` semantics can be reproduced: every NaN compares equal to
/// every other NaN, and `0.0` does not compare equal to `-0.0`.
fn float_to_int_bits(value: f32) -> u32 {
    if value.is_nan() {
        0x7fc0_0000
    } else {
        value.to_bits()
    }
}

/// Canonicalizes NaN the way `Double.doubleToLongBits` does.
fn double_to_long_bits(value: f64) -> u64 {
    if value.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        value.to_bits()
    }
}

impl PartialEq for ExplanationValue {
    /// Reproduces `Objects.equals(Number, Number)` on the boxed values: two
    /// values are equal only when they were boxed as the same Java type, and
    /// `Float.equals`/`Double.equals` compare bit patterns rather than using
    /// `==`.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Long(a), Self::Long(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => float_to_int_bits(*a) == float_to_int_bits(*b),
            (Self::Double(a), Self::Double(b)) => {
                double_to_long_bits(*a) == double_to_long_bits(*b)
            }
            _ => false,
        }
    }
}

impl Eq for ExplanationValue {}

impl Hash for ExplanationValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Int(value) => (0u8, i64::from(*value)).hash(state),
            Self::Long(value) => (1u8, *value).hash(state),
            Self::Float(value) => (2u8, float_to_int_bits(*value)).hash(state),
            Self::Double(value) => (3u8, double_to_long_bits(*value)).hash(state),
        }
    }
}

impl fmt::Display for ExplanationValue {
    /// Renders the value the way the boxed Java type's `toString()` does, which
    /// is what `Explanation`'s summary and every description built by string
    /// concatenation depend on.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(value) => write!(f, "{value}"),
            Self::Long(value) => write!(f, "{value}"),
            Self::Float(value) => f.write_str(&java_fmt::float_to_string(*value)),
            Self::Double(value) => f.write_str(&java_fmt::double_to_string(*value)),
        }
    }
}

/// Expert: describes the score computation for a document and query.
///
/// Equivalent to `org.apache.lucene.search.Explanation`. The Java class is
/// `final` and immutable, and so is this port: a node is built by
/// [`Explanation::matched`] or [`Explanation::no_match`] and never changed
/// afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Explanation {
    is_match: bool,
    value: ExplanationValue,
    description: String,
    details: Vec<Explanation>,
}

impl Explanation {
    /// Creates a new explanation for a match.
    ///
    /// Equivalent to `Explanation.match(Number, String, Collection<Explanation>)`
    /// (`Explanation.java:37-40`); Java's varargs overload
    /// (`Explanation.java:48-50`) is the same call with a list built at the call
    /// site. The name is spelled `matched` because `match` is a Rust keyword.
    ///
    /// * `value` — the contribution to the score of the document;
    /// * `description` — how `value` was computed;
    /// * `details` — sub-explanations that contributed to this one.
    pub fn matched(
        value: impl Into<ExplanationValue>,
        description: impl Into<String>,
        details: Vec<Explanation>,
    ) -> Self {
        Self {
            is_match: true,
            value: value.into(),
            description: description.into(),
            details,
        }
    }

    /// Creates a new explanation for a document which does not match.
    ///
    /// Equivalent to `Explanation.noMatch(String, Collection<Explanation>)`
    /// (`Explanation.java:53-55`). The value of a non-matching node is `0f`, as
    /// in Java.
    pub fn no_match(description: impl Into<String>, details: Vec<Explanation>) -> Self {
        Self {
            is_match: false,
            value: ExplanationValue::Float(0.0),
            description: description.into(),
            details,
        }
    }

    /// Indicates whether this explanation models a match.
    ///
    /// Equivalent to `Explanation.isMatch()`.
    pub fn is_match(&self) -> bool {
        self.is_match
    }

    /// Returns the value assigned to this explanation node.
    ///
    /// Equivalent to `Explanation.getValue()`.
    pub fn value(&self) -> ExplanationValue {
        self.value
    }

    /// Returns the description of this explanation node.
    ///
    /// Equivalent to `Explanation.getDescription()`.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the sub-nodes of this explanation node.
    ///
    /// Equivalent to `Explanation.getDetails()`, which copies the list into an
    /// array; the immutable slice serves the same purpose without the copy.
    pub fn details(&self) -> &[Explanation] {
        &self.details
    }

    /// Renders this node and its sub-nodes at the given depth, two spaces per
    /// level, as `Explanation.toString(int)` does
    /// (`Explanation.java:106-119`).
    fn write(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        for _ in 0..depth {
            f.write_str("  ")?;
        }
        writeln!(f, "{} = {}", self.value, self.description)?;
        for detail in &self.details {
            detail.write(f, depth + 1)?;
        }
        Ok(())
    }
}

impl fmt::Display for Explanation {
    /// Renders an explanation as text, exactly as `Explanation.toString()`
    /// does — including the trailing newline on every line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, 0)
    }
}
