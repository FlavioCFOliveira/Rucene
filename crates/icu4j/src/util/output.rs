// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.Output`.

use std::fmt;

/// Simple struct-like class for output parameters.
///
/// Equivalent to `com.ibm.icu.util.Output<T>`: a public, nullable `value`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Output<T> {
    /// The value field; `None` for Java's `null`.
    pub value: Option<T>,
}

impl<T> Output<T> {
    /// `new Output()`: a null value.
    pub fn new() -> Self {
        Self { value: None }
    }

    /// `new Output(T value)`.
    pub fn with_value(value: T) -> Self {
        Self { value: Some(value) }
    }
}

impl<T: fmt::Display> fmt::Display for Output<T> {
    /// `toString()`: `"null"` for a null value, else `value.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.value {
            None => f.write_str("null"),
            Some(v) => v.fmt(f),
        }
    }
}
