// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.NoUnit`.

use crate::util::measure_unit::{self, MeasureUnit};

/// Dimensionless unit for percent and permille. Prior to ICU 68, this
/// class was a subclass of MeasureUnit; since ICU 68, it is a holder of
/// constants.
///
/// Equivalent to `com.ibm.icu.util.NoUnit` (non-instantiable).
pub struct NoUnit {
    _private: (),
}

impl NoUnit {
    /// Constant for the base unit (no unit): Java's `null`. Equivalent to
    /// `BASE`.
    pub const BASE: Option<MeasureUnit> = None;

    /// Constant for the percent unit, or 1/100 of a base unit. Equivalent to
    /// `PERCENT` (`MeasureUnit.PERCENT`).
    pub fn percent() -> MeasureUnit {
        measure_unit::PERCENT.clone()
    }

    /// Constant for the permille unit, or 1/100 of a base unit. Equivalent
    /// to `PERMILLE` (`MeasureUnit.PERMILLE`).
    pub fn permille() -> MeasureUnit {
        measure_unit::PERMILLE.clone()
    }
}
