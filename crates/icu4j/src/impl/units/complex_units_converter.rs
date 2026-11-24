// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.ComplexUnitsConverter`.

use std::cell::RefCell;
use std::fmt;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use crate::jdk::math::{BigDecimal, BigInteger, RoundingMode};
use crate::number::precision::Precision;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::{
    DecimalQuantity_DualStorageBCD, JavaNumber,
};
use crate::r#impl::units::conversion_rates::ConversionRates;
use crate::r#impl::units::measure_unit_impl::{
    MeasureUnitImpl, MeasureUnitImplComparator, MeasureUnitImplWithIndex,
    MeasureUnitImplWithIndexComparator,
};
use crate::r#impl::units::units_converter::UnitsConverter;
use crate::util::measure::Measure;

/// `EPSILON`: `BigDecimal.valueOf(Math.ulp(1.0))`.
pub static EPSILON: LazyLock<BigDecimal> =
    LazyLock::new(|| BigDecimal::value_of_double(f64::EPSILON).expect("finite constant"));
/// `EPSILON_MULTIPLIER`: `1 + EPSILON`.
pub static EPSILON_MULTIPLIER: LazyLock<BigDecimal> = LazyLock::new(|| {
    BigDecimal::value_of(1)
        .add(&EPSILON)
        .expect("exact addition")
});

/// Converts from single or compound unit to single, compound or mixed
/// units. For example, from `meter` to `foot+inch`.
///
/// DESIGN: This class uses `UnitsConverter` in order to perform the
/// single converter (i.e. converters from a single unit to another single
/// unit). Therefore, `ComplexUnitsConverter` needs multiple instances of
/// `UnitsConverter` to convert the mixed units.
///
/// Equivalent to `com.ibm.icu.impl.units.ComplexUnitsConverter`;
/// `toString()` is `Display`.
#[derive(Debug, Clone)]
pub struct ComplexUnitsConverter {
    /// `unitsConverters_`.
    pub units_converters: Vec<UnitsConverter>,
    /// `units_`: individual units of mixed units, sorted big to small, with
    /// indices indicating the requested output mixed unit order.
    pub units: Vec<MeasureUnitImplWithIndex>,
    /// `inputUnit_`.
    input_unit: MeasureUnitImpl,
}

impl ComplexUnitsConverter {
    /// Constructs `ComplexUnitsConverter` for an `targetUnit` that could be
    /// Single, Compound or Mixed. In case of: 1- Single and Compound units,
    /// the conversion will not perform anything, the input will be equal
    /// to the output. 2- Mixed Unit the conversion will consider the input
    /// in the biggest unit. Equivalent to
    /// `ComplexUnitsConverter(MeasureUnitImpl, ConversionRates)`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion-rate computation.
    pub fn for_target(
        target_unit: &MeasureUnitImpl,
        conversion_rates: &ConversionRates,
    ) -> Result<Self> {
        let units = target_unit.extract_individual_units_with_indices();
        debug_assert!(!units.is_empty());

        // Assign the biggest unit to inputUnit_.
        let mut input_unit = units[0].unit_impl.clone();
        {
            let comparator = MeasureUnitImplComparator::new(conversion_rates);
            for unit_with_index in &units {
                if comparator.compare(&unit_with_index.unit_impl, &input_unit)? > 0 {
                    input_unit = unit_with_index.unit_impl.clone();
                }
            }
        }

        let mut result = ComplexUnitsConverter {
            units_converters: Vec::new(),
            units,
            input_unit,
        };
        result.init(conversion_rates)?;
        Ok(result)
    }

    /// Equivalent to `ComplexUnitsConverter(String, String)`.
    ///
    /// # Errors
    ///
    /// The errors of the parsing and of the data loading.
    pub fn from_identifiers(
        input_unit_identifier: &str,
        output_units_identifier: &str,
    ) -> Result<Self> {
        Self::new(
            &MeasureUnitImpl::for_identifier(input_unit_identifier)?,
            &MeasureUnitImpl::for_identifier(output_units_identifier)?,
            &ConversionRates::new()?,
        )
    }

    /// Constructor of `ComplexUnitsConverter`. NOTE: - inputUnit and
    /// outputUnits must be under the same category. Equivalent to
    /// `ComplexUnitsConverter(MeasureUnitImpl, MeasureUnitImpl,
    /// ConversionRates)`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion-rate computation.
    pub fn new(
        input_unit: &MeasureUnitImpl,
        output_units: &MeasureUnitImpl,
        conversion_rates: &ConversionRates,
    ) -> Result<Self> {
        let units = output_units.extract_individual_units_with_indices();
        debug_assert!(!units.is_empty());

        let mut result = ComplexUnitsConverter {
            units_converters: Vec::new(),
            units,
            input_unit: input_unit.clone(),
        };
        result.init(conversion_rates)?;
        Ok(result)
    }

    /// Sorts units_, which must be populated before calling this, and
    /// populates unitsConverters_. Equivalent to `init(ConversionRates)`.
    fn init(&mut self, conversion_rates: &ConversionRates) -> Result<()> {
        // Sort the units in a descending order.
        {
            let comparator = MeasureUnitImplWithIndexComparator::new(conversion_rates);
            let error: RefCell<Option<LuceneError>> = RefCell::new(None);
            // Collections.reverseOrder(cmp).compare(a, b) == cmp.compare(b, a)
            self.units.sort_by(|a, b| {
                if error.borrow().is_some() {
                    return std::cmp::Ordering::Equal;
                }
                match comparator.compare(b, a) {
                    Ok(c) => c.cmp(&0),
                    Err(e) => {
                        *error.borrow_mut() = Some(e);
                        std::cmp::Ordering::Equal
                    }
                }
            });
            if let Some(e) = error.into_inner() {
                return Err(e);
            }
        }

        // If the `outputUnits` is `UMEASURE_UNIT_MIXED` such as
        // `foot+inch` then the first converter will convert from the input
        // unit to the biggest unit, the second converter will convert from
        // the biggest unit to the next unit, and so on.
        self.units_converters = Vec::new();
        for i in 0..self.units.len() {
            if i == 0 {
                // first element
                self.units_converters.push(UnitsConverter::new(
                    &self.input_unit,
                    &self.units[i].unit_impl,
                    conversion_rates,
                )?);
            } else {
                self.units_converters.push(UnitsConverter::new(
                    &self.units[i - 1].unit_impl,
                    &self.units[i].unit_impl,
                    conversion_rates,
                )?);
            }
        }
        Ok(())
    }

    /// Returns true if the specified `quantity` of the `inputUnit`,
    /// expressed in terms of the biggest unit in the MeasureUnit
    /// `outputUnit`, is greater than or equal to `limit`. Equivalent to
    /// `greaterThanOrEqual(BigDecimal, BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the conversion.
    pub fn greater_than_or_equal(&self, quantity: &BigDecimal, limit: &BigDecimal) -> Result<bool> {
        debug_assert!(!self.units.is_empty());

        // NOTE: First converter converts to the biggest quantity.
        Ok(self.units_converters[0]
            .convert(quantity)?
            .multiply(&EPSILON_MULTIPLIER)?
            .compare_to(limit)
            >= 0)
    }

    /// Returns outputMeasures which is an array with the corresponding
    /// values. Equivalent to `convert(BigDecimal, Precision)` (`None` for
    /// Java's `null` rounder).
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the conversion and the rounding.
    pub fn convert(
        &self,
        quantity: &BigDecimal,
        rounder: Option<&Precision>,
    ) -> Result<ComplexConverterResult> {
        let mut quantity = quantity.clone();
        let mut sign = BigInteger::one();
        if quantity.compare_to(&BigDecimal::zero()) < 0 && self.units_converters.len() > 1 {
            quantity = quantity.abs();
            sign = sign.negate();
        }

        // For N converters:
        // - the first converter converts from the input unit to the largest
        //   unit,
        // - N-1 converters convert to bigger units for which we want
        //   integers,
        // - the Nth converter (index N-1) converts to the smallest unit,
        //   which isn't (necessarily) an integer.
        let mut int_values: Vec<BigInteger> =
            Vec::with_capacity(self.units_converters.len().saturating_sub(1));

        let n = self.units_converters.len();
        for (i, converter) in self.units_converters.iter().enumerate() {
            quantity = converter.convert(&quantity)?;

            if i < n - 1 {
                // The double type has 15 decimal digits of precision. For
                // choosing whether to use the current unit or the next
                // smaller unit, we therefore nudge up the number with which
                // the thresholding decision is made. However after the
                // thresholding, we use the original values to ensure
                // unbiased accuracy (to the extent of double's capabilities).
                let floored_quantity = quantity
                    .multiply(&EPSILON_MULTIPLIER)?
                    .set_scale(0, RoundingMode::Floor)?
                    .to_big_integer();
                let floored_long = floored_quantity.long_value();
                int_values.push(floored_quantity);

                // Keep the residual of the quantity.
                //   For example: `3.6 feet`, keep only `0.6 feet`
                let remainder = quantity.subtract(&BigDecimal::value_of(floored_long))?;
                if remainder.compare_to(&BigDecimal::zero()) == -1 {
                    quantity = BigDecimal::zero();
                } else {
                    quantity = remainder;
                }
            }
        }

        quantity = self.apply_rounder(&mut int_values, quantity, rounder)?;

        // Initialize empty measures.
        let mut measures: Vec<Option<Measure>> = vec![None; self.units_converters.len()];

        // Package values into Measure instances in measures:
        let mut index_of_quantity = -1;
        for i in 0..n {
            if i < n - 1 {
                let measure = Measure::new(
                    Some(JavaNumber::BigInteger(int_values[i].multiply(&sign))),
                    Some(self.units[i].unit_impl.clone().build()?),
                )?;
                measures[self.units[i].index as usize] = Some(measure);
            } else {
                index_of_quantity = self.units[i].index;
                let measure = Measure::new(
                    Some(JavaNumber::BigDecimal(
                        quantity.multiply(&BigDecimal::value_of(sign.long_value()))?,
                    )),
                    Some(self.units[i].unit_impl.clone().build()?),
                )?;
                measures[index_of_quantity as usize] = Some(measure);
            }
        }

        Ok(ComplexConverterResult {
            index_of_quantity,
            measures,
        })
    }

    /// Applies the rounder to the quantity (last element) and bubbles up
    /// any carried value to all the intValues. Equivalent to
    /// `applyRounder(List<BigInteger>, BigDecimal, Precision)`.
    fn apply_rounder(
        &self,
        int_values: &mut [BigInteger],
        quantity: BigDecimal,
        rounder: Option<&Precision>,
    ) -> Result<BigDecimal> {
        let rounder = match rounder {
            None => return Ok(quantity),
            Some(r) => r,
        };

        let mut quantity_bcd = DecimalQuantity_DualStorageBCD::from_big_decimal(&quantity)?;
        rounder.apply(&mut quantity_bcd)?;
        let mut quantity = quantity_bcd.to_big_decimal()?;

        if int_values.is_empty() {
            // There is only one element, Therefore, nothing to be done.
            return Ok(quantity);
        }

        // Check if there's a carry, and bubble it back up the resulting
        // intValues.
        let last_index = self.units_converters.len() - 1;
        let mut carry = self.units_converters[last_index]
            .convert_inverse(&quantity)?
            .multiply(&EPSILON_MULTIPLIER)?
            .set_scale(0, RoundingMode::Floor)?;
        if carry.compare_to(&BigDecimal::zero()) <= 0 {
            // carry is not greater than zero
            return Ok(quantity);
        }
        quantity = quantity.subtract(&self.units_converters[last_index].convert(&carry)?)?;
        int_values[last_index - 1] = int_values[last_index - 1].add(&carry.to_big_integer());

        // We don't use the first converter: that one is for the input unit
        let mut j = last_index - 1;
        while j > 0 {
            carry = self.units_converters[j]
                .convert_inverse(&BigDecimal::value_of(int_values[j].long_value()))?
                .multiply(&EPSILON_MULTIPLIER)?
                .set_scale(0, RoundingMode::Floor)?;
            if carry.compare_to(&BigDecimal::zero()) <= 0 {
                // carry is not greater than zero
                break;
            }
            int_values[j] =
                int_values[j].subtract(&self.units_converters[j].convert(&carry)?.to_big_integer());
            int_values[j - 1] = int_values[j - 1].add(&carry.to_big_integer());
            j -= 1;
        }

        Ok(quantity)
    }
}

impl fmt::Display for ComplexUnitsConverter {
    /// Equivalent to `toString()`: `"ComplexUnitsConverter
    /// [unitsConverters_=" + unitsConverters_ + ", units_=" + units_ +
    /// "]"`, the lists in `AbstractCollection.toString` form; the elements
    /// of `units_` do not override `toString`: Java prints
    /// `ClassName@<hex identityHashCode>`, the hash being the crate's
    /// stable identity-hash stand-in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let converters: Vec<String> = self
            .units_converters
            .iter()
            .map(ToString::to_string)
            .collect();
        const CLASS: &str = "com.ibm.icu.impl.units.MeasureUnitImpl$MeasureUnitImplWithIndex";
        let units: Vec<String> = self
            .units
            .iter()
            .map(|_| {
                format!(
                    "{CLASS}@{:x}",
                    rucene::util::java_collections::java_identity_hash_code_stand_in(CLASS) as u32
                )
            })
            .collect();
        write!(
            f,
            "ComplexUnitsConverter [unitsConverters_=[{}], units_=[{}]]",
            converters.join(", "),
            units.join(", ")
        )
    }
}

/// Equivalent to `ComplexUnitsConverter.ComplexConverterResult`.
#[derive(Debug, Clone)]
pub struct ComplexConverterResult {
    /// `indexOfQuantity`.
    pub index_of_quantity: i32,
    /// `measures` (every slot set by `convert`).
    pub measures: Vec<Option<Measure>>,
}
