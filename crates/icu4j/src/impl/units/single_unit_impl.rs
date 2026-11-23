// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.SingleUnitImpl`.

use rucene::error::{LuceneError, Result};

use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::r#impl::units::units_data::UnitsData;
use crate::util::measure_unit::{MeasurePrefix, MeasureUnit};

/// A single unit (a simple unit with a prefix and a power).
///
/// Equivalent to `com.ibm.icu.impl.units.SingleUnitImpl`; `copy()` is
/// [`copy`](Self::copy) (and `Clone`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleUnitImpl {
    /// `index`: simple unit index, unique for every simple unit, -1 for the
    /// dimensionless unit.
    index: i32,
    /// `simpleUnitID`: SimpleUnit is the simplest form of a Unit (e.g.
    /// "meter").
    simple_unit_id: String,
    /// `dimensionality`: the power of the unit.
    dimensionality: i32,
    /// `unitPrefix`.
    unit_prefix: MeasurePrefix,
}

impl Default for SingleUnitImpl {
    fn default() -> Self {
        SingleUnitImpl { index: -1, simple_unit_id: String::new(), dimensionality: 1, unit_prefix: MeasurePrefix::One }
    }
}

impl SingleUnitImpl {
    /// Equivalent to `SingleUnitImpl()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `copy()`.
    pub fn copy(&self) -> SingleUnitImpl {
        self.clone()
    }

    /// Returns a MeasureUnit object with this single unit. Equivalent to
    /// `build()`.
    ///
    /// # Errors
    ///
    /// The errors of the unit data and the identifier serialization.
    pub fn build(&self) -> Result<MeasureUnit> {
        let mut measure_unit = MeasureUnitImpl::from_single_unit(self);
        measure_unit.build()
    }

    /// Returns the neutral identifier of the single unit, for example
    /// "square-kilometer". Equivalent to `getNeutralIdentifier()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Unit Identifier Syntax Error")` for a
    /// power above 15.
    pub fn get_neutral_identifier(&self) -> Result<String> {
        let mut result = String::new();
        let abs_power = self.get_dimensionality().wrapping_abs();

        debug_assert!(abs_power > 0, "this function does not support the dimensionless single units");

        if abs_power == 1 {
            // no-op
        } else if abs_power == 2 {
            result.push_str("square-");
        } else if abs_power == 3 {
            result.push_str("cubic-");
        } else if abs_power <= 15 {
            result.push_str("pow");
            result.push_str(&abs_power.to_string());
            result.push('-');
        } else {
            // TODO: IllegalArgumentException might not be appropriate here
            return Err(LuceneError::illegal_argument("Unit Identifier Syntax Error"));
        }

        result.push_str(self.get_prefix().get_identifier());
        result.push_str(self.get_simple_unit_id());

        Ok(result)
    }

    /// Compare this SingleUnitImpl to another SingleUnitImpl for the sake of
    /// sorting and coalescing. Equivalent to `compareTo(SingleUnitImpl)`
    /// (package-private).
    pub(crate) fn compare_to(&self, other: &SingleUnitImpl) -> i32 {
        // Positive dimensions first
        if self.dimensionality < 0 && other.dimensionality > 0 {
            return 1;
        }
        if self.dimensionality > 0 && other.dimensionality < 0 {
            return -1;
        }

        // Sort by official quantity order
        let this_category_index = UnitsData::get_category_index_of_simple_unit(self.index);
        let other_category_index = UnitsData::get_category_index_of_simple_unit(other.index);
        if this_category_index < other_category_index {
            return -1;
        }
        if this_category_index > other_category_index {
            return 1;
        }

        // Sort by simple unit index
        if self.index < other.index {
            return -1;
        }
        if self.index > other.index {
            return 1;
        }

        // When comparing binary prefixes vs SI prefixes, instead of
        // comparing the actual values, we can multiply the binary prefix
        // power by 3 and compare the powers. if they are equal, we can
        // compare the bases.
        // NOTE: this methodology will fail if the binary prefix more than
        // or equal 98.
        let unit_base = self.unit_prefix.get_base();
        let other_unit_base = other.unit_prefix.get_base();
        // unitPower is used to compare the prefixes without considering
        // the base.
        let unit_power_comp =
            if unit_base == 1024 { self.unit_prefix.get_power() * 3 } else { self.unit_prefix.get_power() };
        let other_unit_power_comp =
            if other_unit_base == 1024 { other.unit_prefix.get_power() * 3 } else { other.unit_prefix.get_power() };

        if unit_power_comp < other_unit_power_comp {
            return 1;
        }
        if unit_power_comp > other_unit_power_comp {
            return -1;
        }

        if unit_base < other_unit_base {
            return 1;
        }
        if unit_base > other_unit_base {
            return -1;
        }

        0
    }

    /// Checks whether this SingleUnitImpl is compatible with another for the
    /// purpose of coalescing. Equivalent to
    /// `isCompatibleWith(SingleUnitImpl)` (package-private).
    pub(crate) fn is_compatible_with(&self, other: &SingleUnitImpl) -> bool {
        self.compare_to(other) == 0
    }

    /// Equivalent to `getSimpleUnitID()`.
    pub fn get_simple_unit_id(&self) -> &str {
        &self.simple_unit_id
    }

    /// Equivalent to `setSimpleUnit(int, String[])`.
    pub fn set_simple_unit(&mut self, simple_unit_index: i32, simple_units: &[String]) {
        self.index = simple_unit_index;
        self.simple_unit_id = simple_units[simple_unit_index as usize].clone();
    }

    /// Equivalent to `getDimensionality()`.
    pub fn get_dimensionality(&self) -> i32 {
        self.dimensionality
    }

    /// Equivalent to `setDimensionality(int)`.
    pub fn set_dimensionality(&mut self, dimensionality: i32) {
        self.dimensionality = dimensionality;
    }

    /// Equivalent to `getPrefix()`.
    pub fn get_prefix(&self) -> MeasurePrefix {
        self.unit_prefix
    }

    /// Equivalent to `setPrefix(MeasurePrefix)`.
    pub fn set_prefix(&mut self, unit_prefix: MeasurePrefix) {
        self.unit_prefix = unit_prefix;
    }

    /// Equivalent to `getIndex()`.
    pub fn get_index(&self) -> i32 {
        self.index
    }
}
