// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.CurrencyPrecision`.

use std::ops::Deref;

use rucene::error::{LuceneError, Result};

use crate::number::precision::{Precision, PrecisionKind};
use crate::util::currency::Currency;

/// A class that defines a rounding strategy parameterized by a currency to
/// be used when formatting numbers in NumberFormatter.
///
/// Equivalent to `com.ibm.icu.number.CurrencyPrecision`, an abstract class
/// (whose only implementation is `Precision.CurrencyRounderImpl`): a view
/// over a [`Precision`] of that class, which it dereferences to.
#[derive(Debug, Clone)]
pub struct CurrencyPrecision(Precision);

impl CurrencyPrecision {
    /// The view over `precision`, which must be a `CurrencyRounderImpl`.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` for another class.
    pub fn cast(precision: Precision) -> CurrencyPrecision {
        assert!(
            matches!(precision.kind(), PrecisionKind::Currency { .. }),
            "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.number.CurrencyPrecision",
            precision.java_class_name()
        );
        CurrencyPrecision(precision)
    }

    /// `precision instanceof CurrencyPrecision ? (CurrencyPrecision)
    /// precision : null`.
    pub fn try_cast(precision: &Precision) -> Option<CurrencyPrecision> {
        match precision.kind() {
            PrecisionKind::Currency { .. } => Some(CurrencyPrecision(precision.clone())),
            _ => None,
        }
    }

    /// The `Precision` this view is.
    pub fn into_precision(self) -> Precision {
        self.0
    }

    /// Associates a currency with this rounding precision. Equivalent to
    /// `withCurrency(Currency)` (`None` for Java's `null`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Currency must not be null")`, and the
    /// arithmetic errors of the increment analysis.
    pub fn with_currency(&self, currency: Option<&Currency>) -> Result<Precision> {
        match currency {
            Some(currency) => {
                let retval = Precision::construct_from_currency(self, currency)?;
                // Java writes the field of the returned object in place,
                // also when it is a shared constant (FIXED_FRAC_2, NICKEL):
                // reproduced through the shared cell.
                retval
                    .0
                    .trailing_zero_display
                    .set(self.trailing_zero_display_value());
                Ok(retval)
            }
            None => Err(LuceneError::illegal_argument("Currency must not be null")),
        }
    }
}

impl Deref for CurrencyPrecision {
    type Target = Precision;

    fn deref(&self) -> &Precision {
        &self.0
    }
}

impl From<CurrencyPrecision> for Precision {
    fn from(p: CurrencyPrecision) -> Precision {
        p.0
    }
}
