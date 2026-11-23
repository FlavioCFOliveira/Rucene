// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.UnitsConverter`.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use rucene::error::Result;

use crate::jdk::math::{BigDecimal, MathContext};
use crate::jdk::units;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::units::conversion_rates::{java_split, ConversionRates};
use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::r#impl::units::single_unit_impl::SingleUnitImpl;
use crate::util::measure_unit::MeasurePrefix;

/// `DECIMAL128` (statically imported from `MathContext`).
const DECIMAL128: MathContext = MathContext::DECIMAL128;

/// A converter between two convertible (or reciprocal) units.
///
/// Equivalent to `com.ibm.icu.impl.units.UnitsConverter`; `toString()` is
/// `Display`.
#[derive(Debug, Clone)]
pub struct UnitsConverter {
    /// `conversionRate`.
    conversion_rate: BigDecimal,
    /// `reciprocal`.
    reciprocal: bool,
    /// `offset`.
    offset: BigDecimal,
    /// `specialSource`: special mapping name of the source unit, `None` when
    /// it uses a conversion rate.
    special_source: Option<String>,
    /// `specialTarget`.
    special_target: Option<String>,
}

impl UnitsConverter {
    /// Constructor of `UnitsConverter`. NOTE: if there are special
    /// mappings, the conversion rates will be computed for the non-special
    /// unit only. Equivalent to `UnitsConverter(String, String)`.
    ///
    /// # Errors
    ///
    /// The errors of the parsing, of the data loading and of the
    /// convertibility check.
    pub fn from_identifiers(source_identifier: &str, target_identifier: &str) -> Result<Self> {
        Self::new(
            &MeasureUnitImpl::for_identifier(source_identifier)?,
            &MeasureUnitImpl::for_identifier(target_identifier)?,
            &ConversionRates::new()?,
        )
    }

    /// Constructor of `UnitsConverter`. NOTE: - source and target must be
    /// under the same category - e.g. meter to mile --> both of them are
    /// length units. Equivalent to `UnitsConverter(MeasureUnitImpl,
    /// MeasureUnitImpl, ConversionRates)`.
    ///
    /// # Errors
    ///
    /// `IllegalIcuArgumentException("input units must be convertible or
    /// reciprocal")`, and the errors of the rate computation.
    pub fn new(source: &MeasureUnitImpl, target: &MeasureUnitImpl, conversion_rates: &ConversionRates) -> Result<Self> {
        let convertibility = Self::extract_convertibility(source, target, conversion_rates)?;
        // TODO(icu-units#82): throw exception if conversion between
        // incompatible types was requested?
        if convertibility != Convertibility::Convertible && convertibility != Convertibility::Reciprocal {
            return Err(IllegalIcuArgumentException::with_message("input units must be convertible or reciprocal").error());
        }

        let special_source = conversion_rates.get_special_mapping_name(source);
        let special_target = conversion_rates.get_special_mapping_name(target);

        if special_source.is_none() && special_target.is_none() {
            let source_to_base = conversion_rates.get_factor_to_base(source)?;
            let target_to_base = conversion_rates.get_factor_to_base(target)?;

            let conversion_rate = if convertibility == Convertibility::Convertible {
                source_to_base.divide(&target_to_base)?.get_conversion_rate()?
            } else {
                debug_assert!(convertibility == Convertibility::Reciprocal);
                source_to_base.multiply(&target_to_base)?.get_conversion_rate()?
            };
            let reciprocal = convertibility == Convertibility::Reciprocal;

            // calculate the offset
            let offset = conversion_rates.get_offset(source, target, &source_to_base, &target_to_base, convertibility)?;
            // We should see no offsets for reciprocal conversions - they
            // don't make sense:
            debug_assert!(convertibility != Convertibility::Reciprocal || offset.signum() == 0);
            Ok(UnitsConverter { conversion_rate, reciprocal, offset, special_source, special_target })
        } else {
            let conversion_rate = if special_source.is_none() {
                // non-special source, special target
                conversion_rates.get_factor_to_base(source)?.get_conversion_rate()?
            } else if special_target.is_none() {
                // special source, non-special target
                conversion_rates.get_factor_to_base(target)?.get_conversion_rate()?
            } else {
                BigDecimal::one()
            };
            Ok(UnitsConverter {
                conversion_rate,
                reciprocal: false,
                offset: BigDecimal::zero(),
                special_source,
                special_target,
            })
        }
    }

    /// Helper method to make it easier to know whether a conversion is
    /// convertible or reciprocal. Equivalent to
    /// `extractConvertibility(MeasureUnitImpl, MeasureUnitImpl,
    /// ConversionRates)`.
    ///
    /// # Errors
    ///
    /// The errors of the base-unit extraction.
    pub fn extract_convertibility(
        source: &MeasureUnitImpl,
        target: &MeasureUnitImpl,
        conversion_rates: &ConversionRates,
    ) -> Result<Convertibility> {
        let source_single_units = conversion_rates.extract_base_units(source)?;
        let target_single_units = conversion_rates.extract_base_units(target)?;

        let mut dimension_map: HashMap<String, i32> = HashMap::new();

        insert_in_map(&mut dimension_map, &source_single_units, 1);
        insert_in_map(&mut dimension_map, &target_single_units, -1);

        if are_dimensions_zeroes(&dimension_map) {
            return Ok(Convertibility::Convertible);
        }

        insert_in_map(&mut dimension_map, &target_single_units, 2);
        if are_dimensions_zeroes(&dimension_map) {
            return Ok(Convertibility::Reciprocal);
        }

        Ok(Convertibility::Unconvertible)
    }

    /// Equivalent to `convert(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the computation.
    pub fn convert(&self, input_value: &BigDecimal) -> Result<BigDecimal> {
        if self.special_source.is_some() || self.special_target.is_some() {
            let base = match &self.special_source {
                // We  have a special mapping from the special type to base
                // (case 1 or 3).
                Some(s) => {
                    if s == "beaufort" {
                        scale_to_base(input_value, &MIN_METERS_PER_SEC_FOR_BEAUFORT)?
                    } else {
                        input_value.clone()
                    }
                }
                // Standard conversion (case 2).
                None => input_value.multiply(&self.conversion_rate)?,
            };
            let result = match &self.special_target {
                // We  have a special mapping from base to the special type
                // (case 2 or 3).
                Some(t) => {
                    if t == "beaufort" {
                        base_to_scale(&base, &MIN_METERS_PER_SEC_FOR_BEAUFORT)
                    } else {
                        base
                    }
                }
                // Standard conversion (case 1).
                None => base.divide_mc(&self.conversion_rate, &DECIMAL128)?,
            };
            return Ok(result);
        }
        let mut result = input_value.multiply(&self.conversion_rate)?.add(&self.offset)?;
        if self.reciprocal {
            // We should see no offsets for reciprocal conversions - they
            // don't make sense:
            debug_assert!(self.offset.signum() == 0);
            if result.compare_to(&BigDecimal::zero()) == 0 {
                // TODO: demonstrate the resulting behaviour in tests... and
                // figure out desired behaviour. (Theoretical result should
                // be infinity, not 0.)
                return Ok(BigDecimal::zero());
            }
            result = BigDecimal::one().divide_mc(&result, &DECIMAL128)?;
        }

        Ok(result)
    }

    /// Equivalent to `convertInverse(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the computation.
    pub fn convert_inverse(&self, input_value: &BigDecimal) -> Result<BigDecimal> {
        let mut result = input_value.clone();
        if self.special_source.is_some() || self.special_target.is_some() {
            let base = match &self.special_target {
                // We  have a special mapping from the special type to base
                // (case 2 or 3).
                Some(t) => {
                    if t == "beaufort" {
                        scale_to_base(input_value, &MIN_METERS_PER_SEC_FOR_BEAUFORT)?
                    } else {
                        input_value.clone()
                    }
                }
                // Standard conversion (case 1).
                None => input_value.multiply(&self.conversion_rate)?,
            };
            let result = match &self.special_source {
                // We  have a special mapping from base to the special type
                // (case 1 or 3).
                Some(s) => {
                    if s == "beaufort" {
                        base_to_scale(&base, &MIN_METERS_PER_SEC_FOR_BEAUFORT)
                    } else {
                        base
                    }
                }
                // Standard conversion (case 2).
                None => base.divide_mc(&self.conversion_rate, &DECIMAL128)?,
            };
            return Ok(result);
        }
        if self.reciprocal {
            // We should see no offsets for reciprocal conversions - they
            // don't make sense:
            debug_assert!(self.offset.signum() == 0);
            if result.compare_to(&BigDecimal::zero()) == 0 {
                // TODO: demonstrate the resulting behaviour in tests... and
                // figure out desired behaviour. (Theoretical result should
                // be infinity, not 0.)
                return Ok(BigDecimal::zero());
            }
            result = BigDecimal::one().divide_mc(&result, &DECIMAL128)?;
        }
        result.subtract(&self.offset)?.divide_mc(&self.conversion_rate, &DECIMAL128)
    }

    /// Equivalent to `getConversionInfo()`.
    pub fn get_conversion_info(&self) -> ConversionInfo {
        ConversionInfo {
            conversion_rate: Some(self.conversion_rate.clone()),
            offset: Some(self.offset.clone()),
            reciprocal: self.reciprocal,
        }
    }
}

impl fmt::Display for UnitsConverter {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnitsConverter [conversionRate={}, offset={}]", self.conversion_rate, self.offset)
    }
}

/// `insertInMap(HashMap, ArrayList, int)`.
fn insert_in_map(dimension_map: &mut HashMap<String, i32>, single_units: &[SingleUnitImpl], multiplier: i32) {
    for single_unit in single_units {
        let entry = dimension_map.entry(single_unit.get_simple_unit_id().to_string()).or_insert(0);
        *entry = entry.wrapping_add(single_unit.get_dimensionality().wrapping_mul(multiplier));
    }
}

/// `areDimensionsZeroes(HashMap)`.
fn are_dimensions_zeroes(dimension_map: &HashMap<String, i32>) -> bool {
    dimension_map.values().all(|&v| v == 0)
}

/// `minMetersPerSecForBeaufort`: the minimum wind speed in m/s for each
/// Beaufort number.
static MIN_METERS_PER_SEC_FOR_BEAUFORT: LazyLock<Vec<BigDecimal>> = LazyLock::new(|| {
    [
        0.0, // 0 Calm
        0.3, // 1 Light air
        1.6, // 2 Light breeze
        3.4, // 3 Gentle breeze
        5.5, // 4 Moderate breeze
        8.0, // 5 Fresh breeze
        10.8, // 6 Strong breeze
        13.9, // 7 Near gale
        17.2, // 8 Gale
        20.8, // 9 Strong gale
        24.5, // 10 Storm
        28.5, // 11 Violent storm
        32.7, // 12 Hurricane
        36.9, // 13 (Extended Beaufort scale)
        41.4, // 14
        46.1, // 15
        51.1, // 16
        55.8, // 17
        61.4, // 18 (exceeding upper limit, using as upper bound)
    ]
    .iter()
    .map(|&d| BigDecimal::value_of_double(d).expect("finite literal"))
    .collect()
});

/// `scaleToBase(BigDecimal, BigDecimal[])`.
fn scale_to_base(scale_value: &BigDecimal, min_base_for_scale_values: &[BigDecimal]) -> Result<BigDecimal> {
    let point_five = BigDecimal::value_of_double(0.5)?;
    let scale_adjust = scale_value.abs().add(&point_five)?; // adjust up for later truncation
    let scale_adjust_capped = scale_adjust.min(&BigDecimal::value_of(min_base_for_scale_values.len() as i64 - 2));
    let scale_index = scale_adjust_capped.int_value() as usize;
    // Return midpoint of range (the final range uses an
    // articial end to produce reasonable midpoint)
    min_base_for_scale_values[scale_index].add(&min_base_for_scale_values[scale_index + 1])?.multiply(&point_five)
}

/// `baseToScale(BigDecimal, BigDecimal[])`.
fn base_to_scale(base_value: &BigDecimal, min_base_for_scale_values: &[BigDecimal]) -> BigDecimal {
    let target = base_value.abs();
    // Arrays.binarySearch over a sorted array with compareTo.
    let mut scale_index = match min_base_for_scale_values.binary_search_by(|probe| probe.compare_to(&target).cmp(&0)) {
        Ok(i) => i as i32,
        Err(insertion_point) => -(insertion_point as i32) - 1,
    };
    if scale_index < 0 {
        // Not exact match, returned value is -(insertion point) - 1
        scale_index = -scale_index - 2;
    }
    // Ensure we are in range
    let scale_max = min_base_for_scale_values.len() as i32 - 2;
    if scale_index > scale_max {
        scale_index = scale_max;
    }
    BigDecimal::value_of(scale_index as i64)
}

/// Equivalent to `UnitsConverter.Convertibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convertibility {
    /// `CONVERTIBLE`.
    Convertible,
    /// `RECIPROCAL`.
    Reciprocal,
    /// `UNCONVERTIBLE`.
    Unconvertible,
}

/// Equivalent to `UnitsConverter.ConversionInfo` (public mutable fields).
#[derive(Debug, Clone, Default)]
pub struct ConversionInfo {
    /// `conversionRate`.
    pub conversion_rate: Option<BigDecimal>,
    /// `offset`.
    pub offset: Option<BigDecimal>,
    /// `reciprocal`.
    pub reciprocal: bool,
}

/// The index of each constant exponent in `Factor.exponents`, in the
/// declaration order of the Java fields (also the order
/// `getConversionRate` applies them in).
mod exp {
    pub const FT_TO_M: usize = 0;
    pub const PI: usize = 1;
    pub const GRAVITY: usize = 2;
    pub const G: usize = 3;
    pub const GAL_IMP_TO_M3: usize = 4;
    pub const LB_TO_KG: usize = 5;
    pub const GLUCOSE_MOLAR_MASS: usize = 6;
    pub const ITEM_PER_MOLE: usize = 7;
    pub const METERS_PER_AU: usize = 8;
    pub const SEC_PER_JULIAN_YEAR: usize = 9;
    pub const SPEED_OF_LIGHT_METERS_PER_SECOND: usize = 10;
    pub const SHO_TO_M3: usize = 11;
    pub const TSUBO_TO_M2: usize = 12;
    pub const SHAKU_TO_M: usize = 13;
    pub const AMU: usize = 14;
    pub const COUNT: usize = 15;
}

/// Responsible for all the Factor operation. NOTE: this class is immutable.
///
/// Equivalent to `UnitsConverter.Factor` (package-private). The fifteen
/// `exponent*` fields are the array `exponents`, indexed in their
/// declaration order; every operation applies to each of them exactly as
/// the Java code does field by field.
#[derive(Debug, Clone)]
pub struct Factor {
    /// `factorNum`.
    factor_num: BigDecimal,
    /// `factorDen`.
    factor_den: BigDecimal,
    /// The exponents of the constants (`exponentFtToM`, `exponentPi`, ...).
    exponents: [i32; exp::COUNT],
}

impl Default for Factor {
    fn default() -> Self {
        Self::new()
    }
}

/// `new BigDecimal(String)` of a literal.
fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_string(&units(s)).expect("valid decimal literal")
}

impl Factor {
    /// Equivalent to `Factor()`.
    pub fn new() -> Self {
        Factor { factor_num: BigDecimal::value_of(1), factor_den: BigDecimal::value_of(1), exponents: [0; exp::COUNT] }
    }

    /// Equivalent to `processFactor(String)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of the number parsing.
    pub fn process_factor(factor: &str) -> Result<Factor> {
        debug_assert!(!factor.is_empty());

        // Remove all spaces in the factor
        let factor: String = factor.chars().filter(|c| !is_java_regex_whitespace(*c)).collect();

        let fractions = java_split(&factor, '/');
        debug_assert!(fractions.len() == 1 || fractions.len() == 2);

        if fractions.len() == 1 {
            return Self::process_factor_without_division(&fractions[0]);
        }

        let num = Self::process_factor_without_division(&fractions[0])?;
        let den = Self::process_factor_without_division(&fractions[1])?;
        num.divide(&den)
    }

    /// `processFactorWithoutDivision(String)`.
    fn process_factor_without_division(factor_without_division: &str) -> Result<Factor> {
        let mut result = Factor::new();
        for powered_entity in java_split(factor_without_division, '*') {
            result.add_powered_entity(&powered_entity)?;
        }

        Ok(result)
    }

    /// Equivalent to `copy()` (protected).
    pub(crate) fn copy(&self) -> Factor {
        self.clone()
    }

    /// Returns a single `BigDecimal` that represent the conversion rate
    /// after substituting all the constants. Equivalent to
    /// `getConversionRate()`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the computation.
    pub fn get_conversion_rate(&self) -> Result<BigDecimal> {
        // TODO: this copies all the exponents then doesn't use them at all.
        let mut result_collector = self.copy();

        result_collector.multiply_power(&bd("0.3048"), self.exponents[exp::FT_TO_M])?;
        result_collector.multiply_power(
            &bd("411557987.0").divide_mc(&bd("131002976.0"), &DECIMAL128)?,
            self.exponents[exp::PI],
        )?;
        result_collector.multiply_power(&bd("9.80665"), self.exponents[exp::GRAVITY])?;
        result_collector.multiply_power(&bd("6.67408E-11"), self.exponents[exp::G])?;
        result_collector.multiply_power(&bd("0.00454609"), self.exponents[exp::GAL_IMP_TO_M3])?;
        result_collector.multiply_power(&bd("0.45359237"), self.exponents[exp::LB_TO_KG])?;
        result_collector.multiply_power(&bd("180.1557"), self.exponents[exp::GLUCOSE_MOLAR_MASS])?;
        result_collector.multiply_power(&bd("6.02214076E+23"), self.exponents[exp::ITEM_PER_MOLE])?;
        result_collector.multiply_power(&bd("149597870700"), self.exponents[exp::METERS_PER_AU])?;
        result_collector.multiply_power(&bd("31557600"), self.exponents[exp::SEC_PER_JULIAN_YEAR])?;
        result_collector.multiply_power(&bd("299792458"), self.exponents[exp::SPEED_OF_LIGHT_METERS_PER_SECOND])?;
        result_collector.multiply_power(&bd("0.001803906836964688204"), self.exponents[exp::SHO_TO_M3])?;
        result_collector.multiply_power(&bd("3.305785123966942"), self.exponents[exp::TSUBO_TO_M2])?;
        result_collector.multiply_power(&bd("0.033057851239669"), self.exponents[exp::SHAKU_TO_M])?;
        result_collector.multiply_power(&bd("1.66053878283E-27"), self.exponents[exp::AMU])?;

        result_collector.factor_num.divide_mc(&result_collector.factor_den, &DECIMAL128)
    }

    /// Multiplies the Factor by a value to the power of `power`. NOTE:
    /// mutates this object. Equivalent to the private `multiply(BigDecimal,
    /// int)`.
    fn multiply_power(&mut self, value: &BigDecimal, power: i32) -> Result<()> {
        if power == 0 {
            return Ok(());
        }

        let abs_powered_value = value.pow_mc(power.wrapping_abs(), &DECIMAL128)?;
        if power > 0 {
            self.factor_num = self.factor_num.multiply(&abs_powered_value)?;
        } else {
            self.factor_den = self.factor_den.multiply(&abs_powered_value)?;
        }
        Ok(())
    }

    /// Apply SI or binary prefix to the Factor. Equivalent to
    /// `applyPrefix(MeasurePrefix)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the power.
    pub fn apply_prefix(&self, unit_prefix: MeasurePrefix) -> Result<Factor> {
        let mut result = self.copy();
        if unit_prefix == MeasurePrefix::One {
            return Ok(result);
        }

        let base = unit_prefix.get_base();
        let power = unit_prefix.get_power();
        let abs_factor = BigDecimal::value_of(base as i64).pow_mc(power.wrapping_abs(), &DECIMAL128)?;

        if power < 0 {
            result.factor_den = self.factor_den.multiply(&abs_factor)?;
            return Ok(result);
        }

        result.factor_num = self.factor_num.multiply(&abs_factor)?;
        Ok(result)
    }

    /// Equivalent to `power(int)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the power.
    pub fn power(&self, power: i32) -> Result<Factor> {
        let mut result = Factor::new();
        if power == 0 {
            return Ok(result);
        }
        if power > 0 {
            result.factor_num = self.factor_num.pow(power)?;
            result.factor_den = self.factor_den.pow(power)?;
        } else {
            result.factor_num = self.factor_den.pow(power.wrapping_mul(-1))?;
            result.factor_den = self.factor_num.pow(power.wrapping_mul(-1))?;
        }

        for i in 0..exp::COUNT {
            result.exponents[i] = self.exponents[i].wrapping_mul(power);
        }

        Ok(result)
    }

    /// Equivalent to `divide(Factor)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the products.
    pub fn divide(&self, other: &Factor) -> Result<Factor> {
        let mut result = Factor::new();
        result.factor_num = self.factor_num.multiply(&other.factor_den)?;
        result.factor_den = self.factor_den.multiply(&other.factor_num)?;

        for i in 0..exp::COUNT {
            result.exponents[i] = self.exponents[i].wrapping_sub(other.exponents[i]);
        }

        Ok(result)
    }

    /// Equivalent to `multiply(Factor)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the products.
    pub fn multiply(&self, other: &Factor) -> Result<Factor> {
        let mut result = Factor::new();
        result.factor_num = self.factor_num.multiply(&other.factor_num)?;
        result.factor_den = self.factor_den.multiply(&other.factor_den)?;

        for i in 0..exp::COUNT {
            result.exponents[i] = self.exponents[i].wrapping_add(other.exponents[i]);
        }

        Ok(result)
    }

    /// Divide by a constant value. Equivalent to `divide(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the product.
    pub fn divide_by(&self, value: &BigDecimal) -> Result<Factor> {
        let mut result = self.copy();
        result.factor_den = self.factor_den.multiply(value)?;
        Ok(result)
    }

    /// Adds Entity with power or not. For example, `12 ^ 3` or `12`.
    /// Equivalent to `addPoweredEntity(String)`.
    fn add_powered_entity(&mut self, powered_entity: &str) -> Result<()> {
        let entities = java_split(powered_entity, '^');
        debug_assert!(entities.len() == 1 || entities.len() == 2);

        let power = if entities.len() == 2 { crate::jdk::parse_int(&units(&entities[1]), 10)? } else { 1 };
        // Java indexes entities[0], which throws for an empty split.
        let entity = entities.first().map(String::as_str).unwrap_or_else(|| crate::jdk::array_index_out_of_bounds(0));
        self.add_entity(entity, power)
    }

    /// Adds Entity with power. Equivalent to `addEntity(String, int)`.
    fn add_entity(&mut self, entity: &str, power: i32) -> Result<()> {
        match entity {
            "ft_to_m" => self.exponents[exp::FT_TO_M] += power,
            "ft2_to_m2" => self.exponents[exp::FT_TO_M] += 2 * power,
            "ft3_to_m3" => self.exponents[exp::FT_TO_M] += 3 * power,
            "in3_to_m3" => {
                self.exponents[exp::FT_TO_M] += 3 * power;
                self.factor_den = self.factor_den.multiply(&BigDecimal::value_of_double(12f64.powi(3))?)?;
            }
            "gal_to_m3" => {
                self.factor_num = self.factor_num.multiply(&BigDecimal::value_of(231))?;
                self.exponents[exp::FT_TO_M] += 3 * power;
                self.factor_den = self.factor_den.multiply(&BigDecimal::value_of(12 * 12 * 12))?;
            }
            "gal_imp_to_m3" => self.exponents[exp::GAL_IMP_TO_M3] += power,
            "G" => self.exponents[exp::G] += power,
            "gravity" => self.exponents[exp::GRAVITY] += power,
            "lb_to_kg" => self.exponents[exp::LB_TO_KG] += power,
            "glucose_molar_mass" => self.exponents[exp::GLUCOSE_MOLAR_MASS] += power,
            "item_per_mole" => self.exponents[exp::ITEM_PER_MOLE] += power,
            "meters_per_AU" => self.exponents[exp::METERS_PER_AU] += power,
            "PI" => self.exponents[exp::PI] += power,
            "sec_per_julian_year" => self.exponents[exp::SEC_PER_JULIAN_YEAR] += power,
            "speed_of_light_meters_per_second" => self.exponents[exp::SPEED_OF_LIGHT_METERS_PER_SECOND] += power,
            "sho_to_m3" => self.exponents[exp::SHO_TO_M3] += power,
            "tsubo_to_m2" => self.exponents[exp::TSUBO_TO_M2] += power,
            "shaku_to_m" => self.exponents[exp::SHAKU_TO_M] += power,
            "AMU" => self.exponents[exp::AMU] += power,
            _ => {
                let decimal_entity = BigDecimal::from_string(&units(entity))?.pow_mc(power, &DECIMAL128)?;
                self.factor_num = self.factor_num.multiply(&decimal_entity)?;
            }
        }
        Ok(())
    }
}

/// The characters of the regex class `\s` (`[ \t\n\x0B\f\r]`).
fn is_java_regex_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}
