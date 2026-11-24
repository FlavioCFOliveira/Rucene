// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.MixedUnitLongNameHandler`.

use std::sync::{Arc, Weak};

use rucene::error::Result;

use crate::jdk::units;
use crate::number::localized_number_formatter::LocalizedNumberFormatter;
use crate::number::number_formatter::{NumberFormatter, UnitWidth};
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::DecimalQuantity_DualStorageBCD;
use crate::r#impl::number::long_name_handler::{
    get_measure_data, get_with_plural, new_unit_data, UnitData,
};
use crate::r#impl::number::long_name_multiplexer::ParentlessMicroPropsGenerator;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::modifier::{Modifier, Parameters, Signum};
use crate::r#impl::number::modifier_store::ModifierStore;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::r#impl::number::simple_modifier::SimpleModifier;
use crate::r#impl::simple_formatter_impl::{Arg, SimpleFormatterImpl};
use crate::r#impl::standard_plural::StandardPlural;
use crate::text::list_formatter::{self, ListFormatter};
use crate::text::plural_rules::PluralRules;
use crate::text::simple_formatter::SimpleFormatter;
use crate::util::measure_unit::{Complexity, MeasureUnit};
use crate::util::u_locale::ULocale;

/// The generator of the long names of mixed units (for example "3 feet, 4
/// inches").
///
/// Equivalent to `com.ibm.icu.impl.number.MixedUnitLongNameHandler` (a
/// `MicroPropsGenerator`, `ModifierStore` and
/// `LongNameMultiplexer.ParentlessMicroPropsGenerator`). Handlers are
/// shared (`Arc`): the modifiers they create refer to them
/// (`Parameters.obj = this`).
#[derive(Debug)]
pub struct MixedUnitLongNameHandler {
    /// `rules`.
    rules: Arc<PluralRules>,
    /// `parent` (`None` for Java's `null`).
    parent: Option<Arc<dyn MicroPropsGenerator>>,

    /// Stores unit data for each of the individual units. For each unit,
    /// it stores ARRAY_LENGTH strings, as returned by getMeasureData
    /// (`fMixedUnitData`).
    f_mixed_unit_data: Vec<UnitData>,

    /// A localized NumberFormatter used to format the integer-valued
    /// bigger units of Mixed Unit measurements (`fIntegerFormatter`).
    f_integer_formatter: LocalizedNumberFormatter,

    /// A localised list formatter for joining mixed units together
    /// (`fListFormatter`).
    f_list_formatter: Arc<ListFormatter>,

    /// This object, for `params.obj = this`.
    self_ref: Weak<MixedUnitLongNameHandler>,
}

impl MixedUnitLongNameHandler {
    /// Construct a localized MixedUnitLongNameHandler for the specified
    /// MeasureUnit. It must be a MIXED unit. Equivalent to
    /// `forMeasureUnit(ULocale, MeasureUnit, UnitWidth, String, PluralRules,
    /// MicroPropsGenerator)`.
    ///
    /// # Errors
    ///
    /// The errors of the unit data and of the list formatter loading.
    pub fn for_measure_unit(
        locale: &ULocale,
        mixed_unit: &MeasureUnit,
        width: UnitWidth,
        unit_display_case: Option<&str>,
        rules: &Arc<PluralRules>,
        parent: Option<Arc<dyn MicroPropsGenerator>>,
    ) -> Result<Arc<MixedUnitLongNameHandler>> {
        debug_assert!(
            mixed_unit.get_complexity()? == Complexity::Mixed,
            "MixedUnitLongNameHandler only supports MIXED units"
        );

        let individual_units = mixed_unit.split_to_single_units()?;
        let mut f_mixed_unit_data = Vec::with_capacity(individual_units.len());
        for unit in &individual_units {
            // Grab data for each of the components.
            let mut unit_data = new_unit_data();
            get_measure_data(locale, unit, width, unit_display_case, &mut unit_data)?;
            f_mixed_unit_data.push(unit_data);
        }

        // TODO(icu-units#120): Make sure ICU doesn't output zero-valued
        // high-magnitude fields
        // * for mixed units count, the number of fields that should be
        //   displayed
        let list_width = match width {
            UnitWidth::Narrow => list_formatter::Width::Narrow,
            UnitWidth::FullName => list_formatter::Width::Wide,
            // This might be the same as SHORT in most languages:
            _ => list_formatter::Width::Short,
        };

        let f_list_formatter =
            ListFormatter::get_instance_typed(locale, list_formatter::Type::Units, list_width)?;

        // We need a localised NumberFormatter for the integers of the
        // bigger units (providing Arabic numerals, for example).
        let f_integer_formatter = NumberFormatter::with_locale(locale);

        Ok(Arc::new_cyclic(|self_ref| MixedUnitLongNameHandler {
            rules: rules.clone(),
            parent,
            f_mixed_unit_data,
            f_integer_formatter,
            f_list_formatter,
            self_ref: self_ref.clone(),
        }))
    }

    /// Equivalent to `getModifier(Signum, StandardPlural)` (the
    /// `ModifierStore` method): unreachable in Java.
    pub fn get_modifier(
        &self,
        _signum: Signum,
        _plural: StandardPlural,
    ) -> Option<Arc<dyn Modifier>> {
        // TODO(icu-units#28): investigate this method while investigating
        // where LongNameHandler.getModifier() gets used. To be sure it
        // remains unreachable:
        debug_assert!(false, "should be unreachable");
        None
    }

    /// For a mixed unit, returns a Modifier that takes only one parameter:
    /// the smallest and negative most unit. For the other units, the result
    /// is formatted using the integer formatter of the localized
    /// NumberFormatter. Equivalent to `getMixedUnitModifier(DecimalQuantity,
    /// MicroProps)`.
    fn get_mixed_unit_modifier(
        &self,
        quantity: &mut dyn DecimalQuantity,
        micros: &MicroProps,
    ) -> Result<Arc<dyn Modifier>> {
        let mixed_measures = micros.mixed_measures.as_deref().unwrap_or(&[]);
        if mixed_measures.is_empty() {
            debug_assert!(false, "Mixed unit: we must have more than one unit value");
            return Err(crate::jdk::unchecked_no_message(
                "java.lang.UnsupportedOperationException",
            ));
        }

        // Algorithm:
        //
        // For the mixed-units measurement of: "3 yard, 1 foot, 2.6 inch",
        // we should first compute the "3 yard" and "1 foot" using the
        // `fIntegerFormatter`.
        //
        // For each mixed unit, we format the number using a pattern for
        // the unit and its plural form.
        let mut output_measures_list: Vec<Vec<u16>> = Vec::new();

        let mut quantity_plural = StandardPlural::Other;
        for (i, measure) in mixed_measures.iter().enumerate() {
            if i as i32 == micros.index_of_quantity {
                if i > 0 && quantity.is_negative() {
                    // If numbers are negative, only the first number needs
                    // to have its negative sign formatted.
                    quantity.negate();
                }

                quantity_plural = RoundingUtils::get_plural_safe(
                    micros.rounder.as_ref(),
                    Some(&self.rules),
                    quantity,
                )?;
                let quantity_simple_format =
                    get_with_plural(&self.f_mixed_unit_data[i], quantity_plural)?;
                let final_formatter =
                    SimpleFormatter::compile_min_max_arguments(&quantity_simple_format, 0, 1)?;
                output_measures_list.push(final_formatter.format(&[Arg::Text(&units("{0}"))])?);
                continue;
            }

            let mut fdec = DecimalQuantity_DualStorageBCD::from_number(measure.get_number())?;
            if i > 0 && fdec.is_negative() {
                // If numbers are negative, only the first number needs to
                // have its negative sign formatted.
                fdec.negate();
            }

            let plural_form =
                RoundingUtils::get_plural_safe(micros.rounder.as_ref(), Some(&self.rules), &fdec)?;

            let simple_format = get_with_plural(&self.f_mixed_unit_data[i], plural_form)?;
            let compiled_formatter =
                SimpleFormatter::compile_min_max_arguments(&simple_format, 0, 1)?;

            let mut appendable = FormattedStringBuilder::new();
            self.f_integer_formatter
                .format_impl(&mut fdec, &mut appendable)?;
            output_measures_list
                .push(compiled_formatter.format(&[Arg::Text(&appendable.to_java_string())])?);
            // TODO(icu-units#67): fix field positions
        }

        // Combine list into a "premixed" pattern
        let premixed_format_pattern = self.f_list_formatter.format(&output_measures_list);
        let mut sb = Vec::new();
        let premixed_compiled = SimpleFormatterImpl::compile_to_string_min_max_arguments(
            &premixed_format_pattern,
            &mut sb,
            0,
            1,
        )?;

        // TODO(icu-units#67): fix field positions
        let this: Arc<dyn ModifierStore> = self
            .self_ref
            .upgrade()
            .expect("the handler is alive while it runs");
        let params = Parameters {
            obj: Some(this),
            signum: Some(Signum::PosZero),
            plural: Some(quantity_plural),
        };
        // Return a SimpleModifier for the "premixed" pattern
        Ok(Arc::new(SimpleModifier::with_parameters(
            &premixed_compiled,
            None,
            false,
            Some(params),
        )))
    }
}

impl ModifierStore for MixedUnitLongNameHandler {
    fn get_modifier(&self, signum: Signum, plural: StandardPlural) -> Option<Arc<dyn Modifier>> {
        MixedUnitLongNameHandler::get_modifier(self, signum, plural)
    }
}

impl MicroPropsGenerator for MixedUnitLongNameHandler {
    /// Produces a plural-appropriate Modifier for a mixed unit: `quantity`
    /// is taken as the final smallest unit, while the larger unit values
    /// must be provided via `micros.mixedMeasures`.
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        debug_assert!(self.f_mixed_unit_data.len() > 1);
        let parent = self
            .parent
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: parent"));
        let mut micros = parent.process_quantity(quantity)?;
        micros.mod_outer = Some(self.get_mixed_unit_modifier(quantity, &micros)?);
        Ok(micros)
    }
}

impl ParentlessMicroPropsGenerator for MixedUnitLongNameHandler {
    /// Produces a plural-appropriate Modifier for a mixed unit: `quantity`
    /// is taken as the final smallest unit, while the larger unit values
    /// must be provided by `micros.mixedMeasures`, micros being the
    /// MicroProps instance to populate. (Equivalent to MicroProps.processQuantity()
    /// without parent.)
    fn process_quantity_with_micros(
        &self,
        quantity: &mut dyn DecimalQuantity,
        mut micros: MicroProps,
    ) -> Result<MicroProps> {
        debug_assert!(self.f_mixed_unit_data.len() > 1);
        micros.mod_outer = Some(self.get_mixed_unit_modifier(quantity, &micros)?);
        Ok(micros)
    }
}
