// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.UsagePrefsHandler`.

use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::units::complex_units_converter::ComplexConverterResult;
use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::r#impl::units::units_router::UnitsRouter;
use crate::util::measure_unit::MeasureUnit;
use crate::util::u_locale::ULocale;

/// The generator that converts a quantity to the unit the locale prefers
/// for a usage.
///
/// Equivalent to `com.ibm.icu.impl.number.UsagePrefsHandler`.
#[derive(Debug)]
pub struct UsagePrefsHandler {
    /// `fParent`.
    f_parent: Arc<dyn MicroPropsGenerator>,
    /// `fUnitsRouter`.
    f_units_router: UnitsRouter,
}

impl UsagePrefsHandler {
    /// Equivalent to `UsagePrefsHandler(ULocale, MeasureUnit, String,
    /// MicroPropsGenerator)`.
    ///
    /// # Errors
    ///
    /// The errors of the unit parsing and of the router construction.
    pub fn new(
        locale: &ULocale,
        input_unit: &MeasureUnit,
        usage: &str,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Result<Self> {
        let f_units_router = UnitsRouter::new(
            &MeasureUnitImpl::for_identifier(&input_unit.get_identifier())?,
            locale,
            usage,
        )?;
        Ok(UsagePrefsHandler {
            f_parent: parent,
            f_units_router,
        })
    }

    /// Populates micros.mixedMeasures and modifies quantity, based on the
    /// values in measures. Equivalent to `mixedMeasuresToMicros(
    /// ComplexConverterResult, DecimalQuantity, MicroProps)` (protected).
    ///
    /// # Errors
    ///
    /// Java's `ClassCastException` when the measure at the quantity index
    /// does not hold a `BigDecimal`, and the errors of the quantity.
    pub(crate) fn mixed_measures_to_micros(
        complex_converter_result: &ComplexConverterResult,
        quantity: &mut dyn DecimalQuantity,
        out_micros: &mut MicroProps,
    ) -> Result<()> {
        let measures: Vec<_> = complex_converter_result
            .measures
            .iter()
            .map(|m| {
                m.clone()
                    .unwrap_or_else(|| panic!("java.lang.NullPointerException: unset measure"))
            })
            .collect();
        out_micros.mixed_measures = Some(measures);
        out_micros.index_of_quantity = complex_converter_result.index_of_quantity;
        let measure = &out_micros.mixed_measures.as_ref().expect("just set")
            [out_micros.index_of_quantity as usize];
        match measure.get_number() {
            JavaNumber::BigDecimal(b) => quantity.set_to_big_decimal(b),
            other => Err(rucene::error::LuceneError::unchecked(
                "java.lang.ClassCastException",
                format!(
                    "class {} cannot be cast to class java.math.BigDecimal",
                    other.class_name()
                ),
            )),
        }
    }

    /// Equivalent to `getOutputUnits()`.
    pub fn get_output_units(&self) -> &[MeasureUnit] {
        self.f_units_router.get_output_units()
    }
}

impl MicroPropsGenerator for UsagePrefsHandler {
    /// Obtains the appropriate output value, MeasureUnit and rounding/precision
    /// behaviour from the UnitsRouter.
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let mut micros = self.f_parent.process_quantity(quantity)?;

        quantity.round_to_infinity(); // Enables toDouble
        let routed = self
            .f_units_router
            .route(&quantity.to_big_decimal()?, Some(&mut micros))?;
        let mut output_unit = routed.output_unit.clone();
        micros.output_unit = Some(output_unit.build()?);

        UsagePrefsHandler::mixed_measures_to_micros(
            &routed.complex_converter_result,
            quantity,
            &mut micros,
        )?;
        Ok(micros)
    }
}
