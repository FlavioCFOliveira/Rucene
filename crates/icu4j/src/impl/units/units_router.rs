// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.UnitsRouter`.

use rucene::error::Result;

use crate::jdk::math::BigDecimal;
use crate::jdk::units;
use crate::number::precision::Precision;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::units::complex_units_converter::{ComplexConverterResult, ComplexUnitsConverter};
use crate::r#impl::units::conversion_rates::ConversionRates;
use crate::r#impl::units::measure_unit_impl::{MeasureUnitImpl, UnitsParser};
use crate::r#impl::units::units_data::UnitsData;
use crate::util::measure_unit::MeasureUnit;
use crate::util::u_locale::ULocale;

/// `UnitsRouter` responsible for converting from a single unit (such as
/// `meter` or `meter-per-second`) to one of the complex units based on the
/// limits. For example: if the input is `meter` and the output is as
/// following
/// `{ foot+inch , limit 3.0}`
/// `{ inch, limit no value (-inf)}`
/// Thus means if the input in `meter` is greater than or equal to `3.0
/// feet`, the output will be in `foot+inch`, otherwise, the output will be
/// in `inch`.
///
/// NOTE: the output units and the their limits MUST BE in order, for
/// example, if the output units, from the previous example, are the
/// following: `{ inch , limit no value (-inf)}` `{ foot+inch, limit 3.0}`
/// IN THIS CASE THE OUTPUT WILL BE ALWAYS IN `inch`.
///
/// NOTE: the output units and their limits will be extracted from the units
/// preferences database by knowing the following: - input unit - locale -
/// usage
///
/// Equivalent to `com.ibm.icu.impl.units.UnitsRouter`.
#[derive(Debug, Clone)]
pub struct UnitsRouter {
    /// `outputUnits_`: a list of all the output units.
    output_units: Vec<MeasureUnit>,
    /// `converterPreferences_`.
    converter_preferences: Vec<ConverterPreference>,
}

impl UnitsRouter {
    /// Equivalent to `UnitsRouter(String, ULocale, String)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_identifier(input_unit_identifier: &str, locale: &ULocale, usage: &str) -> Result<Self> {
        Self::new(&MeasureUnitImpl::for_identifier(input_unit_identifier)?, locale, usage)
    }

    /// Equivalent to `UnitsRouter(MeasureUnitImpl, ULocale, String)`.
    ///
    /// # Errors
    ///
    /// The errors of the data loading, the category lookup and the
    /// converters.
    ///
    /// # Panics
    ///
    /// Java's `AssertionError("Only `precision-increment` is allowed")` for
    /// another skeleton in the data.
    pub fn new(input_unit: &MeasureUnitImpl, locale: &ULocale, usage: &str) -> Result<Self> {
        // TODO: do we need to pass the ConversionRates and UnitPreferences
        // as parameters?
        // TODO: Calculate the category from the input unit.
        let data = UnitsData::new()?;

        let category = data.get_category(input_unit)?;
        let unit_preferences = data.get_preferences_for(&category, usage, locale)?;

        let mut output_units = Vec::new();
        let mut converter_preferences = Vec::new();
        for preference in &unit_preferences {
            // Extract the complex unit from preference.
            let mut complex_target_unit_impl = UnitsParser::parse_for_identifier(Some(preference.get_unit()))?
                .ok_or_else(|| crate::jdk::null_pointer("empty preference unit"))?;
            let precision = preference.get_skeleton();
            if !precision.is_empty() && !precision.starts_with("precision-increment") {
                // TODO: `precision-increment` is the only accepted skeleton
                // for now.
                panic!("java.lang.AssertionError: Only `precision-increment` is allowed");
            }

            output_units.push(complex_target_unit_impl.build()?);
            converter_preferences.push(ConverterPreference::with_limit(
                input_unit,
                &complex_target_unit_impl,
                preference.get_geq().clone(),
                precision,
                data.get_conversion_rates(),
            )?);
        }
        Ok(UnitsRouter { output_units, converter_preferences })
    }

    /// Equivalent to `route(BigDecimal, MicroProps)`: when `micros` is
    /// given, its rounder may be replaced (Java writes the field).
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the conversion and of the increment
    /// parsing.
    pub fn route(&self, quantity: &BigDecimal, micros: Option<&mut MicroProps>) -> Result<RouteResult> {
        let mut rounder: Option<Precision> = micros.as_ref().and_then(|m| m.rounder.clone());

        let mut converter_preference: Option<&ConverterPreference> = None;
        for itr in &self.converter_preferences {
            converter_preference = Some(itr);
            if itr.converter.greater_than_or_equal(&quantity.abs(), &itr.limit)? {
                break;
            }
        }
        debug_assert!(converter_preference.is_some());
        let converter_preference =
            converter_preference.unwrap_or_else(|| panic!("java.lang.NullPointerException: no converter preference"));

        // This code is a partial port of the C++ RoundingImpl class in
        // number_roundingutils.h:
        if let Some(bogus) = rounder.as_ref().filter(|r| r.is_bogus()).cloned() {
            if !converter_preference.precision.is_empty() {
                rounder = Some(bogus.bogus_into(&Self::parse_skeleton_to_precision(&converter_preference.precision)?));
            } else {
                // We use the same rounding mode as COMPACT notation: known
                // to be a human-friendly rounding mode: integers, but add a
                // decimal digit as needed to ensure we have at least 2
                // significant digits.
                rounder = Some(bogus.bogus_into(&Precision::integer().with_min_digits(2)?));
            }
        }

        if let Some(micros) = micros {
            micros.rounder = rounder.clone();
        }
        Ok(RouteResult {
            complex_converter_result: converter_preference.converter.convert(quantity, rounder.as_ref())?,
            output_unit: converter_preference.target_unit.clone(),
        })
    }

    /// `parseSkeletonToPrecision(String)`.
    fn parse_skeleton_to_precision(precision_skeleton: &str) -> Result<Precision> {
        const K_SKELETON_PREFIX: &str = "precision-increment/";
        if !precision_skeleton.starts_with(K_SKELETON_PREFIX) {
            return Err(IllegalIcuArgumentException::with_message("precisionSkeleton is only precision-increment")
                .error());
        }

        let increment_value = &precision_skeleton[K_SKELETON_PREFIX.len()..];
        Precision::increment(Some(&BigDecimal::from_string(&units(increment_value))?))
    }

    /// Returns the list of possible output units, i.e. the units that this
    /// router will route to. Equivalent to `getOutputUnits()`.
    pub fn get_output_units(&self) -> &[MeasureUnit] {
        &self.output_units
    }
}

/// Contains the complex unit converter and the limit which representing
/// the smallest value that the converter should accept.
///
/// Equivalent to `UnitsRouter.ConverterPreference`.
#[derive(Debug, Clone)]
pub struct ConverterPreference {
    /// `targetUnit`: the output unit for this ConverterPreference.
    target_unit: MeasureUnitImpl,
    /// `converter`.
    converter: ComplexUnitsConverter,
    /// `limit`.
    limit: BigDecimal,
    /// `precision`.
    precision: String,
}

impl ConverterPreference {
    /// In case there is no limit, the limit will be -inf. Equivalent to
    /// `ConverterPreference(MeasureUnitImpl, MeasureUnitImpl, String,
    /// ConversionRates)`.
    ///
    /// # Errors
    ///
    /// The errors of the converter.
    pub fn new(
        source: &MeasureUnitImpl,
        target_unit: &MeasureUnitImpl,
        precision: &str,
        conversion_rates: &ConversionRates,
    ) -> Result<Self> {
        Self::with_limit(source, target_unit, BigDecimal::value_of_double(f64::from_bits(1))?, precision, conversion_rates)
    }

    /// Equivalent to `ConverterPreference(MeasureUnitImpl, MeasureUnitImpl,
    /// BigDecimal, String, ConversionRates)`.
    ///
    /// # Errors
    ///
    /// The errors of the converter.
    pub fn with_limit(
        source: &MeasureUnitImpl,
        target_unit: &MeasureUnitImpl,
        limit: BigDecimal,
        precision: &str,
        conversion_rates: &ConversionRates,
    ) -> Result<Self> {
        Ok(ConverterPreference {
            converter: ComplexUnitsConverter::new(source, target_unit, conversion_rates)?,
            limit,
            precision: precision.to_string(),
            target_unit: target_unit.clone(),
        })
    }
}

/// Equivalent to `UnitsRouter.RouteResult`.
#[derive(Debug, Clone)]
pub struct RouteResult {
    /// `complexConverterResult`: contains the complex converter result.
    pub complex_converter_result: ComplexConverterResult,
    /// `outputUnit`: the output unit for this RouteResult. This may be a
    /// MIXED unit - for example: "yard-and-foot-and-inch".
    pub output_unit: MeasureUnitImpl,
}
