// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.LongNameMultiplexer`.

use std::sync::Arc;

use rucene::error::Result;

use crate::number::number_formatter::UnitWidth;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::long_name_handler::LongNameHandler;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::mixed_unit_long_name_handler::MixedUnitLongNameHandler;
use crate::text::plural_rules::PluralRules;
use crate::util::measure_unit::{Complexity, MeasureUnit};
use crate::util::u_locale::ULocale;

/// A generator that can process a quantity on micros it is handed, without
/// a parent. Equivalent to the interface
/// `LongNameMultiplexer.ParentlessMicroPropsGenerator`.
pub trait ParentlessMicroPropsGenerator: std::fmt::Debug + Send + Sync {
    /// Equivalent to `processQuantityWithMicros(DecimalQuantity,
    /// MicroProps)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the plural rounding, and the errors of
    /// the formatting the mixed-unit handler performs.
    fn process_quantity_with_micros(&self, quantity: &mut dyn DecimalQuantity, micros: MicroProps) -> Result<MicroProps>;
}

/// A generator that selects, by the output unit of the micros, the
/// long-name handler of one of several units.
///
/// Equivalent to `com.ibm.icu.impl.number.LongNameMultiplexer`.
#[derive(Debug)]
pub struct LongNameMultiplexer {
    /// `fParent`.
    f_parent: Arc<dyn MicroPropsGenerator>,
    /// `fHandlers`: the handlers of the units, one per unit in
    /// `fMeasureUnits`.
    f_handlers: Vec<Arc<dyn ParentlessMicroPropsGenerator>>,
    /// `fMeasureUnits`: the units the handlers are for.
    f_measure_units: Vec<MeasureUnit>,
}

impl LongNameMultiplexer {
    /// Equivalent to `LongNameMultiplexer(MicroPropsGenerator)`.
    pub fn new(f_parent: Arc<dyn MicroPropsGenerator>) -> Self {
        LongNameMultiplexer { f_parent, f_handlers: Vec::new(), f_measure_units: Vec::new() }
    }

    /// Produces a multiplexer that uses `LongNameHandler` or
    /// `MixedUnitLongNameHandler` instances for each of the units, depending
    /// on their complexity. Equivalent to `forMeasureUnits(ULocale,
    /// List<MeasureUnit>, UnitWidth, String, PluralRules,
    /// MicroPropsGenerator)`.
    ///
    /// # Errors
    ///
    /// The errors of the handler construction.
    pub fn for_measure_units(
        locale: &ULocale,
        units: &[MeasureUnit],
        width: UnitWidth,
        unit_display_case: Option<&str>,
        rules: &Arc<PluralRules>,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Result<LongNameMultiplexer> {
        let mut result = LongNameMultiplexer::new(parent);

        debug_assert!(!units.is_empty());

        for unit in units {
            result.f_measure_units.push(unit.clone());
            if unit.get_complexity()? == Complexity::Mixed {
                let mlnh = MixedUnitLongNameHandler::for_measure_unit(locale, unit, width, unit_display_case, rules, None)?;
                result.f_handlers.push(mlnh);
            } else {
                let lnh = LongNameHandler::for_measure_unit(locale, unit, width, unit_display_case, rules, None)?;
                result.f_handlers.push(lnh);
            }
        }

        Ok(result)
    }
}

impl MicroPropsGenerator for LongNameMultiplexer {
    /// The output unit must be provided via `micros.outputUnit`, it selects
    /// the unit to be used.
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        // We call parent->processQuantity() from the Multiplexer, instead of
        // letting LongNameHandler handle it: we don't know which
        // LongNameHandler to call until we've called the parent!
        let micros = self.f_parent.process_quantity(quantity)?;

        // Call the correct LongNameHandler based on outputUnit
        for (i, unit) in self.f_measure_units.iter().enumerate() {
            let matches = match &micros.output_unit {
                Some(output) => unit.java_equals(output),
                None => false,
            };
            if matches {
                let handler = self.f_handlers[i].clone();
                return handler.process_quantity_with_micros(quantity, micros);
            }
        }

        panic!("java.lang.AssertionError:  We shouldn't receive any outputUnit for which we haven't already got a LongNameHandler");
    }
}
