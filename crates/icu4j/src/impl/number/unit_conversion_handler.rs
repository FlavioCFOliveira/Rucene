// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.UnitConversionHandler`.

use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::usage_prefs_handler::UsagePrefsHandler;
use crate::r#impl::units::complex_units_converter::ComplexUnitsConverter;
use crate::r#impl::units::conversion_rates::ConversionRates;
use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::util::measure_unit::MeasureUnit;

/// A MicroPropsGenerator which converts a measurement from one MeasureUnit
/// to another. In particular, the output MeasureUnit may be a mixed unit.
/// (The input unit may not be a mixed unit.)
///
/// Equivalent to `com.ibm.icu.impl.number.UnitConversionHandler`.
#[derive(Debug)]
pub struct UnitConversionHandler {
    /// `fParent`.
    f_parent: Arc<dyn MicroPropsGenerator>,
    /// `fOutputUnit`.
    f_output_unit: MeasureUnit,
    /// `fComplexUnitConverter`.
    f_complex_unit_converter: ComplexUnitsConverter,
}

impl UnitConversionHandler {
    /// Equivalent to `UnitConversionHandler(MeasureUnit,
    /// MicroPropsGenerator)`: `target_unit` is the output unit (the input
    /// unit is the same unit), `parent` the parent generator.
    ///
    /// # Errors
    ///
    /// The errors of the unit parsing and of the conversion data.
    pub fn new(target_unit: &MeasureUnit, parent: Arc<dyn MicroPropsGenerator>) -> Result<Self> {
        let target_unit_impl = MeasureUnitImpl::for_identifier(&target_unit.get_identifier())?;
        let f_complex_unit_converter = ComplexUnitsConverter::for_target(&target_unit_impl, &ConversionRates::new()?)?;
        Ok(UnitConversionHandler { f_parent: parent, f_output_unit: target_unit.clone(), f_complex_unit_converter })
    }
}

impl MicroPropsGenerator for UnitConversionHandler {
    /// Obtains the appropriate output values from the Unit Converter.
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let mut result = self.f_parent.process_quantity(quantity)?;

        quantity.round_to_infinity(); // Enables toDouble
        let complex_converter_result =
            self.f_complex_unit_converter.convert(&quantity.to_big_decimal()?, result.rounder.as_ref())?;

        result.output_unit = Some(self.f_output_unit.clone());
        UsagePrefsHandler::mixed_measures_to_micros(&complex_converter_result, quantity, &mut result)?;

        Ok(result)
    }
}
