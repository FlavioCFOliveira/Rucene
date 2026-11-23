// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.IntegerWidth`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::r#impl::number::rounding_utils::RoundingUtils;

/// The fields of an `IntegerWidth` object.
#[derive(Debug)]
pub(crate) struct IntegerWidthData {
    /// `minInt`.
    pub(crate) min_int: i32,
    /// `maxInt`.
    pub(crate) max_int: i32,
}

/// A class that defines the strategy for padding and truncating integers
/// before the decimal separator.
///
/// To create an IntegerWidth, use one of the factory methods.
///
/// Equivalent to `com.ibm.icu.number.IntegerWidth`. Java's objects are
/// immutable and compared by identity; this handle shares one object
/// ([`ptr_eq`](Self::ptr_eq) is Java's `==` and `equals`).
#[derive(Debug, Clone)]
pub struct IntegerWidth(pub(crate) Arc<IntegerWidthData>);

/// `DEFAULT`.
pub(crate) static DEFAULT: LazyLock<IntegerWidth> = LazyLock::new(|| IntegerWidth::new(1, -1));

impl IntegerWidth {
    /// `IntegerWidth(int, int)` (private).
    fn new(min_int: i32, max_int: i32) -> Self {
        IntegerWidth(Arc::new(IntegerWidthData { min_int, max_int }))
    }

    /// `DEFAULT`.
    pub(crate) fn default_width() -> IntegerWidth {
        DEFAULT.clone()
    }

    /// `minInt`.
    pub(crate) fn min_int(&self) -> i32 {
        self.0.min_int
    }

    /// `maxInt`.
    pub(crate) fn max_int(&self) -> i32 {
        self.0.max_int
    }

    /// Whether two handles are the same Java object.
    pub fn ptr_eq(&self, other: &IntegerWidth) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// `Object.hashCode()` (identity): see
    /// `java_identity_hash_code_stand_in`.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.number.IntegerWidth")
    }

    /// Pad numbers at the beginning with zeros to guarantee a certain number
    /// of numerals before the decimal separator. Equivalent to
    /// `zeroFillTo(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `0..=999`.
    pub fn zero_fill_to(min_int: i32) -> Result<IntegerWidth> {
        if min_int == 1 {
            Ok(DEFAULT.clone())
        } else if (0..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&min_int) {
            Ok(IntegerWidth::new(min_int, -1))
        } else {
            Err(LuceneError::illegal_argument(format!(
                "Integer digits must be between 0 and {} (inclusive)",
                RoundingUtils::MAX_INT_FRAC_SIG
            )))
        }
    }

    /// Truncate numbers exceeding a certain number of numerals before the
    /// decimal separator. Equivalent to `truncateAt(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `-1..=999`
    /// or below the minimum.
    pub fn truncate_at(&self, max_int: i32) -> Result<IntegerWidth> {
        if max_int == self.0.max_int {
            Ok(self.clone())
        } else if (0..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&max_int) && max_int >= self.0.min_int {
            Ok(IntegerWidth::new(self.0.min_int, max_int))
        } else if self.0.min_int == 1 && max_int == -1 {
            Ok(DEFAULT.clone())
        } else if max_int == -1 {
            Ok(IntegerWidth::new(self.0.min_int, -1))
        } else {
            Err(LuceneError::illegal_argument(format!(
                "Integer digits must be between -1 and {} (inclusive)",
                RoundingUtils::MAX_INT_FRAC_SIG
            )))
        }
    }
}
