//! Per-document long values, ported from
//! `org.apache.lucene.search.LongValues`.

#![deny(unsafe_code)]

use crate::error::Result;

/// The per-document values of a [`LongValuesSource`](crate::search::LongValuesSource).
///
/// Equivalent to the abstract class `org.apache.lucene.search.LongValues`.
///
/// **Divergence from Lucene 10.5.0.** As for
/// [`DoubleValues`](crate::search::DoubleValues), `longValue()` takes
/// `&mut self` because every non-trivial implementation reads through an
/// iterator it owns and advances.
pub trait LongValues {
    /// Returns the value for the current document.
    ///
    /// Equivalent to `LongValues.longValue()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the value.
    fn long_value(&mut self) -> Result<i64>;

    /// Advances to exactly `doc`, returning whether it has a value.
    ///
    /// Equivalent to `LongValues.advanceExact(int)`. Documents must be passed
    /// in increasing order.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    fn advance_exact(&mut self, doc: i32) -> Result<bool>;
}

impl<T: LongValues + ?Sized> LongValues for Box<T> {
    fn long_value(&mut self) -> Result<i64> {
        (**self).long_value()
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        (**self).advance_exact(doc)
    }
}
