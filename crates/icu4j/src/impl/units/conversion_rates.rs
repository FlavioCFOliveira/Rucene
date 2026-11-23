// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.ConversionRates`.

use std::collections::HashMap;

use rucene::error::Result;

use crate::jdk::math::{BigDecimal, MathContext};
use crate::jdk::units;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::r#impl::units::measure_unit_impl::{MeasureUnitImpl, UnitsParser};
use crate::r#impl::units::single_unit_impl::SingleUnitImpl;
use crate::r#impl::units::units_converter::{Convertibility, Factor};
use crate::r#impl::units::units_data::Constants;
use crate::util::measure_unit::{Complexity, MeasurePrefix};
use crate::util::u_resource_bundle::UResourceBundle;

/// The conversion rates of the simple units, from the `units` resource.
///
/// Equivalent to `com.ibm.icu.impl.units.ConversionRates`.
#[derive(Debug, Clone)]
pub struct ConversionRates {
    /// `mapToConversionRate`: map from any simple unit (i.e. "meter",
    /// "foot", "inch") to its basic/root conversion rate info.
    map_to_conversion_rate: HashMap<String, ConversionRateInfo>,
}

/// `mapToConversionRate.get(id)`, which Java dereferences at once: a
/// missing entry is a `NullPointerException`.
fn info<'a>(map: &'a HashMap<String, ConversionRateInfo>, id: &str) -> Result<&'a ConversionRateInfo> {
    map.get(id).ok_or_else(|| crate::jdk::null_pointer(&format!("no conversion rate for {id}")))
}

impl ConversionRates {
    /// Equivalent to `ConversionRates()`.
    ///
    /// # Errors
    ///
    /// The errors of the data loading.
    pub fn new() -> Result<Self> {
        // Read the conversion rates from the data (units.txt).
        let resource = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "units")?;
        let mut sink = ConversionRatesSink::default();
        resource.as_icu()?.get_all_items_with_fallback(Constants::CONVERSION_UNIT_TABLE_NAME, &mut sink)?;
        Ok(ConversionRates { map_to_conversion_rate: sink.get_map_to_conversion_rate() })
    }

    /// Extracts the factor from a `SingleUnitImpl` to its Basic Unit.
    /// Equivalent to the private `getFactorToBase(SingleUnitImpl)`.
    fn get_factor_to_base_single(&self, single_unit: &SingleUnitImpl) -> Result<Factor> {
        let power = single_unit.get_dimensionality();
        let unit_prefix = single_unit.get_prefix();
        let result = Factor::process_factor(
            info(&self.map_to_conversion_rate, single_unit.get_simple_unit_id())?.get_conversion_rate()?,
        )?;

        // Prefix before power, because:
        // - square-kilometer to square-meter: (1000)^2
        // - square-kilometer to square-foot (approximate): (3.28*1000)^2
        result.apply_prefix(unit_prefix)?.power(power)
    }

    /// Equivalent to `getFactorToBase(MeasureUnitImpl)`.
    ///
    /// # Errors
    ///
    /// The errors of the factor computation; `IllegalIcuArgumentException`
    /// for a unit without a conversion rate (a special unit).
    pub fn get_factor_to_base(&self, measure_unit: &MeasureUnitImpl) -> Result<Factor> {
        let mut result = Factor::new();
        for single_unit in measure_unit.get_single_units() {
            result = result.multiply(&self.get_factor_to_base_single(single_unit)?)?;
        }

        // Apply the constant denominator
        if measure_unit.get_constant_denominator() != 0 {
            result = result.divide_by(&BigDecimal::value_of(measure_unit.get_constant_denominator()))?;
        }

        Ok(result)
    }

    /// Equivalent to `getOffset(MeasureUnitImpl, MeasureUnitImpl, Factor,
    /// Factor, Convertibility)` (protected).
    ///
    /// # Errors
    ///
    /// The errors of the factor computation and of the division.
    pub(crate) fn get_offset(
        &self,
        source: &MeasureUnitImpl,
        target: &MeasureUnitImpl,
        _source_to_base: &Factor,
        target_to_base: &Factor,
        convertibility: Convertibility,
    ) -> Result<BigDecimal> {
        if convertibility != Convertibility::Convertible {
            return Ok(BigDecimal::value_of(0));
        }
        if !(Self::check_simple_unit(source) && Self::check_simple_unit(target)) {
            return Ok(BigDecimal::value_of(0));
        }

        let source_simple_identifier = source.get_single_units()[0].get_simple_unit_id();
        let target_simple_identifier = target.get_single_units()[0].get_simple_unit_id();

        let source_offset = info(&self.map_to_conversion_rate, source_simple_identifier)?.get_offset();
        let target_offset = info(&self.map_to_conversion_rate, target_simple_identifier)?.get_offset();
        source_offset.subtract(target_offset)?.divide_mc(&target_to_base.get_conversion_rate()?, &MathContext::DECIMAL128)
    }

    /// Equivalent to `getSpecialMappingName(MeasureUnitImpl)` (protected).
    pub(crate) fn get_special_mapping_name(&self, simple_unit: &MeasureUnitImpl) -> Option<String> {
        if !Self::check_simple_unit(simple_unit) {
            return None;
        }
        let simple_identifier = simple_unit.get_single_units()[0].get_simple_unit_id();
        // Java NPE for an unknown simple unit; the parser only produces
        // known simple units.
        self.map_to_conversion_rate
            .get(simple_identifier)
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: no conversion rate for {simple_identifier}"))
            .get_special_mapping_name()
            .map(str::to_string)
    }

    /// Equivalent to `extractCompoundBaseUnit(MeasureUnitImpl)`.
    ///
    /// # Errors
    ///
    /// The errors of the target parsing.
    pub fn extract_compound_base_unit(&self, measure_unit: &MeasureUnitImpl) -> Result<MeasureUnitImpl> {
        let base_units = self.extract_base_units(measure_unit)?;

        let mut result = MeasureUnitImpl::new();
        for base_unit in &base_units {
            result.append_single_unit(Some(base_unit));
        }

        Ok(result)
    }

    /// Equivalent to `extractBaseUnits(MeasureUnitImpl)`.
    ///
    /// # Errors
    ///
    /// The errors of the target parsing.
    pub fn extract_base_units(&self, measure_unit_impl: &MeasureUnitImpl) -> Result<Vec<SingleUnitImpl>> {
        let mut result = Vec::new();
        for single_unit in measure_unit_impl.get_single_units() {
            result.extend(self.extract_base_units_single(single_unit)?);
        }

        Ok(result)
    }

    /// Equivalent to `extractBaseUnits(SingleUnitImpl)`.
    ///
    /// # Errors
    ///
    /// The errors of the target parsing.
    pub fn extract_base_units_single(&self, single_unit: &SingleUnitImpl) -> Result<Vec<SingleUnitImpl>> {
        let target = info(&self.map_to_conversion_rate, single_unit.get_simple_unit_id())?.get_target();
        let mut target_impl = UnitsParser::parse_for_identifier(Some(target))?
            .ok_or_else(|| crate::jdk::null_pointer("empty conversion target"))?;

        // Each unit must be powered by the same dimension
        target_impl.apply_dimensionality(single_unit.get_dimensionality());

        // NOTE: we do not need to apply the SI prefixes.

        Ok(target_impl.get_single_units().clone())
    }

    /// Equivalent to `extractSystems(SingleUnitImpl)`: the systems string,
    /// `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for an unknown simple unit.
    pub fn extract_systems(&self, single_unit: &SingleUnitImpl) -> Result<Option<String>> {
        Ok(info(&self.map_to_conversion_rate, single_unit.get_simple_unit_id())?.get_systems().map(str::to_string))
    }

    /// Checks if the `MeasureUnitImpl` is simple or not. Equivalent to
    /// `checkSimpleUnit(MeasureUnitImpl)`.
    fn check_simple_unit(measure_unit_impl: &MeasureUnitImpl) -> bool {
        if measure_unit_impl.get_complexity() != Complexity::Single {
            return false;
        }
        let single_unit = &measure_unit_impl.get_single_units()[0];

        if single_unit.get_prefix() != MeasurePrefix::One {
            return false;
        }

        if single_unit.get_dimensionality() != 1 {
            return false;
        }

        true
    }
}

/// Equivalent to `ConversionRates.ConversionRatesSink`.
#[derive(Debug, Default)]
pub struct ConversionRatesSink {
    /// `mapToConversionRate`: map from any simple unit (i.e. "meter",
    /// "foot", "inch") to its basic/root conversion rate info.
    map_to_conversion_rate: HashMap<String, ConversionRateInfo>,
}

impl Sink for ConversionRatesSink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        debug_assert!(Constants::CONVERSION_UNIT_TABLE_NAME == key.as_string());

        let conversion_rate_table = value.get_table()?;
        let mut i = 0;
        while conversion_rate_table.get_key_and_value(i, key, value) {
            i += 1;
            debug_assert!(value.get_type() == UResourceBundle::TABLE);

            let simple_unit = key.as_string();

            let simple_unit_conversion_info = value.get_table()?;
            let mut target = None;
            let mut factor = None;
            let mut offset = "0".to_string();
            let mut special = None;
            let mut systems = None;
            let mut j = 0;
            while simple_unit_conversion_info.get_key_and_value(j, key, value) {
                j += 1;
                debug_assert!(value.get_type() == UResourceBundle::STRING);

                let key_string = key.as_string();
                let value_string = value.to_java_string().replace(' ', "");
                match key_string.as_str() {
                    "target" => target = Some(value_string),
                    "factor" => factor = Some(value_string),
                    "offset" => offset = value_string,
                    "special" => special = Some(value_string),
                    // just want the raw string
                    "systems" => systems = Some(value.to_java_string()),
                    _ => debug_assert!(false, "The key must be target, factor, offset, special, or systems"),
                }
            }

            // HashMap.put replaces.
            debug_assert!(target.is_some());
            debug_assert!(factor.is_some() || special.is_some());
            let info = ConversionRateInfo::new(&simple_unit, target, factor, &offset, special, systems)?;
            self.map_to_conversion_rate.insert(simple_unit, info);
        }
        Ok(())
    }
}

impl ConversionRatesSink {
    /// Equivalent to `getMapToConversionRate()`.
    pub fn get_map_to_conversion_rate(self) -> HashMap<String, ConversionRateInfo> {
        self.map_to_conversion_rate
    }
}

/// Equivalent to `ConversionRates.ConversionRateInfo`.
#[derive(Debug, Clone)]
pub struct ConversionRateInfo {
    /// `simpleUnit` (unused in Java too).
    #[allow(dead_code)]
    simple_unit: String,
    /// `target`.
    target: Option<String>,
    /// `conversionRate`.
    conversion_rate: Option<String>,
    /// `offset`.
    offset: BigDecimal,
    /// `specialMappingName`: the name of a special mapping used instead of
    /// a conversion rate.
    special_mapping_name: Option<String>,
    /// `systems`.
    systems: Option<String>,
}

impl ConversionRateInfo {
    /// Equivalent to `ConversionRateInfo(String, String, String, String,
    /// String, String)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of the offset parsing.
    pub fn new(
        simple_unit: &str,
        target: Option<String>,
        conversion_rate: Option<String>,
        offset: &str,
        special: Option<String>,
        systems: Option<String>,
    ) -> Result<Self> {
        Ok(ConversionRateInfo {
            simple_unit: simple_unit.to_string(),
            target,
            conversion_rate,
            offset: Self::for_number_with_division(offset)?,
            special_mapping_name: special,
            systems,
        })
    }

    /// `forNumberWithDivision(String)`.
    fn for_number_with_division(number_with_division: &str) -> Result<BigDecimal> {
        let numbers = java_split(number_with_division, '/');
        debug_assert!(numbers.len() <= 2);

        if numbers.len() == 1 {
            return BigDecimal::from_string(&units(&numbers[0]));
        }

        BigDecimal::from_string(&units(&numbers[0]))?
            .divide_mc(&BigDecimal::from_string(&units(&numbers[1]))?, &MathContext::DECIMAL128)
    }

    /// Equivalent to `getTarget()` (`""` for Java's `null`, which the sink
    /// never stores: every unit has a target).
    pub fn get_target(&self) -> &str {
        self.target.as_deref().unwrap_or("")
    }

    /// Equivalent to `getOffset()`.
    pub fn get_offset(&self) -> &BigDecimal {
        &self.offset
    }

    /// Equivalent to `getConversionRate()`.
    ///
    /// # Errors
    ///
    /// `IllegalIcuArgumentException("trying to use a null conversion rate
    /// (for special?)")`.
    pub fn get_conversion_rate(&self) -> Result<&str> {
        match &self.conversion_rate {
            Some(r) => Ok(r),
            None => Err(IllegalIcuArgumentException::with_message("trying to use a null conversion rate (for special?)")
                .error()),
        }
    }

    /// Equivalent to `getSpecialMappingName()`.
    pub fn get_special_mapping_name(&self) -> Option<&str> {
        self.special_mapping_name.as_deref()
    }

    /// Equivalent to `getSystems()`.
    pub fn get_systems(&self) -> Option<&str> {
        self.systems.as_deref()
    }
}

/// `String.split(String)` with a one-character literal separator: the
/// pieces, trailing empty strings removed.
pub(crate) fn java_split(s: &str, sep: char) -> Vec<String> {
    let mut parts: Vec<String> = s.split(sep).map(str::to_string).collect();
    while parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() && !s.is_empty() {
        parts.clear();
    }
    parts
}
