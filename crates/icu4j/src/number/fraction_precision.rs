// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.FractionPrecision`.

use std::ops::Deref;

use rucene::error::{LuceneError, Result};

use crate::number::number_formatter::RoundingPriority;
use crate::number::precision::{significant_digits_message, Precision, PrecisionKind};
use crate::r#impl::number::rounding_utils::RoundingUtils;

/// A class that defines a rounding strategy based on a number of fraction
/// places and optionally significant digits to be used when formatting
/// numbers in NumberFormatter.
///
/// Equivalent to `com.ibm.icu.number.FractionPrecision`, an abstract class
/// (whose only implementation is `Precision.FractionRounderImpl`): a view
/// over a [`Precision`] of that class, which it dereferences to.
#[derive(Debug, Clone)]
pub struct FractionPrecision(Precision);

impl FractionPrecision {
    /// The view over `precision`, which must be a `FractionRounderImpl`.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` for another class.
    pub fn cast(precision: Precision) -> FractionPrecision {
        assert!(
            matches!(precision.kind(), PrecisionKind::Fraction { .. }),
            "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.number.FractionPrecision",
            precision.java_class_name()
        );
        FractionPrecision(precision)
    }

    /// `precision instanceof FractionPrecision ? (FractionPrecision)
    /// precision : null`.
    pub fn try_cast(precision: &Precision) -> Option<FractionPrecision> {
        match precision.kind() {
            PrecisionKind::Fraction { .. } => Some(FractionPrecision(precision.clone())),
            _ => None,
        }
    }

    /// The `Precision` this view is.
    pub fn into_precision(self) -> Precision {
        self.0
    }

    /// Override maximum fraction digits with maximum significant digits
    /// depending on the magnitude of the number. Equivalent to
    /// `withSignificantDigits(int, int, NumberFormatter.RoundingPriority)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid significant-digits range.
    pub fn with_significant_digits(
        &self,
        min_significant_digits: i32,
        max_significant_digits: i32,
        priority: RoundingPriority,
    ) -> Result<Precision> {
        if max_significant_digits >= 1
            && max_significant_digits >= min_significant_digits
            && max_significant_digits <= RoundingUtils::MAX_INT_FRAC_SIG
        {
            Ok(Precision::construct_fraction_significant(
                self,
                min_significant_digits,
                max_significant_digits,
                priority,
                false,
            ))
        } else {
            Err(LuceneError::illegal_argument(significant_digits_message()))
        }
    }

    /// Ensure that no less than this number of significant digits are
    /// retained when rounding according to fraction rules. Equivalent to
    /// `withMinDigits(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `1..=999`.
    pub fn with_min_digits(&self, min_significant_digits: i32) -> Result<Precision> {
        if (1..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&min_significant_digits) {
            Ok(Precision::construct_fraction_significant(
                self,
                1,
                min_significant_digits,
                RoundingPriority::Relaxed,
                true,
            ))
        } else {
            Err(LuceneError::illegal_argument(significant_digits_message()))
        }
    }

    /// Ensure that no more than this number of significant digits are
    /// retained when rounding according to fraction rules. Equivalent to
    /// `withMaxDigits(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `1..=999`.
    pub fn with_max_digits(&self, max_significant_digits: i32) -> Result<Precision> {
        if (1..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&max_significant_digits) {
            Ok(Precision::construct_fraction_significant(
                self,
                1,
                max_significant_digits,
                RoundingPriority::Strict,
                true,
            ))
        } else {
            Err(LuceneError::illegal_argument(significant_digits_message()))
        }
    }
}

impl Deref for FractionPrecision {
    type Target = Precision;

    fn deref(&self) -> &Precision {
        &self.0
    }
}

impl From<FractionPrecision> for Precision {
    fn from(p: FractionPrecision) -> Precision {
        p.0
    }
}
