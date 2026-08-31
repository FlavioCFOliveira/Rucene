//! Per-document double values, ported from
//! `org.apache.lucene.search.DoubleValues`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::scorable::Scorable;

/// The per-document values of a [`DoubleValuesSource`](crate::search::DoubleValuesSource).
///
/// Equivalent to the abstract class `org.apache.lucene.search.DoubleValues`.
///
/// **Divergence from Lucene 10.5.0.** Java declares `doubleValue()` on an
/// immutable-looking receiver, but every non-trivial implementation reads
/// through an iterator it owns and advances; this port therefore takes
/// `&mut self`, exactly as [`Scorable`] already does for the same reason.
pub trait DoubleValues {
    /// Returns the value for the current document.
    ///
    /// Equivalent to `DoubleValues.doubleValue()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the value.
    fn double_value(&mut self) -> Result<f64>;

    /// Advances to exactly `doc`, returning whether it has a value.
    ///
    /// Equivalent to `DoubleValues.advanceExact(int)`. Documents must be passed
    /// in increasing order.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    fn advance_exact(&mut self, doc: i32) -> Result<bool>;
}

impl<T: DoubleValues + ?Sized> DoubleValues for Box<T> {
    fn double_value(&mut self) -> Result<f64> {
        (**self).double_value()
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        (**self).advance_exact(doc)
    }
}

impl<T: DoubleValues + ?Sized> DoubleValues for &mut T {
    fn double_value(&mut self) -> Result<f64> {
        (**self).double_value()
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        (**self).advance_exact(doc)
    }
}

/// Wraps values so that every document has one, substituting `missing_value`
/// where the wrapped values have none.
///
/// Equivalent to the static
/// `DoubleValues.withDefault(DoubleValues, double)`.
pub fn with_default<V: DoubleValues>(inner: V, missing_value: f64) -> WithDefaultDoubleValues<V> {
    WithDefaultDoubleValues {
        inner,
        missing_value,
        has_value: false,
    }
}

/// The values `with_default` returns.
///
/// Equivalent to the anonymous `DoubleValues` of
/// `DoubleValues.withDefault(DoubleValues, double)`.
#[derive(Debug)]
pub struct WithDefaultDoubleValues<V: DoubleValues> {
    inner: V,
    missing_value: f64,
    has_value: bool,
}

impl<V: DoubleValues> DoubleValues for WithDefaultDoubleValues<V> {
    fn double_value(&mut self) -> Result<f64> {
        if self.has_value {
            self.inner.double_value()
        } else {
            Ok(self.missing_value)
        }
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.has_value = self.inner.advance_exact(doc)?;
        Ok(true)
    }
}

/// Values that no document has.
///
/// Equivalent to the `DoubleValues.EMPTY` constant.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyDoubleValues;

impl DoubleValues for EmptyDoubleValues {
    /// Equivalent to the constant's `doubleValue()`, which throws
    /// `UnsupportedOperationException`.
    fn double_value(&mut self) -> Result<f64> {
        Err(LuceneError::UnsupportedOperation(
            "DoubleValues.EMPTY has no values".to_string(),
        ))
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(false)
    }
}

/// Values reading the score of a [`Scorable`].
///
/// Equivalent to the anonymous `DoubleValues` that the static
/// `DoubleValuesSource.fromScorer(Scorable)` returns. It borrows the scorable
/// rather than capturing it, which is what Rust requires to express the same
/// object.
pub struct ScorerDoubleValues<'a> {
    scorer: &'a mut dyn Scorable,
}

impl std::fmt::Debug for ScorerDoubleValues<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScorerDoubleValues")
    }
}

impl<'a> ScorerDoubleValues<'a> {
    /// Creates values reading `scorer`'s score.
    ///
    /// Equivalent to `DoubleValuesSource.fromScorer(Scorable)`.
    pub fn new(scorer: &'a mut dyn Scorable) -> Self {
        Self { scorer }
    }
}

impl DoubleValues for ScorerDoubleValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.scorer.score()?))
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}
