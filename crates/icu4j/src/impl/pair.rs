// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Pair`.

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashCode;

/// A pair of values.
///
/// Equivalent to `com.ibm.icu.impl.Pair<F, S>`; `equals` is the derived
/// `PartialEq` (both components `equals`), `hashCode` is
/// [`JavaHashCode`] when both components have one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair<F, S> {
    /// `first`.
    pub first: F,
    /// `second`.
    pub second: S,
}

impl<F, S> Pair<F, S> {
    /// The protected constructor `Pair(F first, S second)`.
    pub fn new(first: F, second: S) -> Self {
        Self { first, second }
    }

    /// `Pair.of(F first, S second)`; Java's `null` is `None`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Pair.of requires non null values.")` when
    /// either value is `None`.
    pub fn of(first: Option<F>, second: Option<S>) -> Result<Self> {
        match (first, second) {
            (Some(first), Some(second)) => Ok(Self { first, second }),
            _ => Err(LuceneError::illegal_argument(
                "Pair.of requires non null values.",
            )),
        }
    }
}

impl<F: JavaHashCode, S: JavaHashCode> JavaHashCode for Pair<F, S> {
    /// `hashCode()`: `first.hashCode() * 37 + second.hashCode()`.
    fn java_hash_code(&self) -> i32 {
        self.first
            .java_hash_code()
            .wrapping_mul(37)
            .wrapping_add(self.second.java_hash_code())
    }
}
