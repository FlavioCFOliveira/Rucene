// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.OutputInt`.

use std::fmt;

/// Simple holder class for an int output parameter (deprecated in ICU).
///
/// Equivalent to `com.ibm.icu.util.OutputInt`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputInt {
    /// The value field.
    pub value: i32,
}

impl OutputInt {
    /// `new OutputInt()`: value 0.
    pub fn new() -> Self {
        Self { value: 0 }
    }

    /// `new OutputInt(int value)`.
    pub fn with_value(value: i32) -> Self {
        Self { value }
    }
}

impl fmt::Display for OutputInt {
    /// `toString()`: `Integer.toString(value)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}
