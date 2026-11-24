// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberRangeFormatterImpl`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::text::{AttributeValue, FieldRef};
use crate::jdk::{null_pointer, units};
use crate::number::formatted_number_range::FormattedNumberRange;
use crate::number::localized_number_formatter::LocalizedNumberFormatter;
use crate::number::number_formatter::NumberFormatter;
use crate::number::number_formatter_impl::{write_affixes, write_number, NumberFormatterImpl};
use crate::number::number_formatter_settings::NumberFormatterSettings;
use crate::number::number_range_formatter::{
    span_field, RangeCollapse, RangeIdentityFallback, RangeIdentityResult,
};
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::formatted_value_string_builder_impl::FormattedValueStringBuilderImpl;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::macro_props::MacroProps;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::modifier::Modifier;
use crate::r#impl::number::range::prefix_infix_suffix_length_helper::PrefixInfixSuffixLengthHelper;
use crate::r#impl::number::range::range_macro_props::RangeMacroProps;
use crate::r#impl::number::range::standard_plural_ranges::StandardPluralRanges;
use crate::r#impl::number::simple_modifier::SimpleModifier;
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::simple_formatter_impl::SimpleFormatterImpl;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::text::number_format::field as nf_field;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// Business logic behind NumberRangeFormatter.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.number.NumberRangeFormatterImpl`.
#[derive(Debug)]
pub struct NumberRangeFormatterImpl {
    formatter_impl1: NumberFormatterImpl,
    formatter_impl2: NumberFormatterImpl,
    f_same_formatters: bool,

    f_collapse: RangeCollapse,
    f_identity_fallback: RangeIdentityFallback,

    // Should be final, but it is set in a helper function, not the constructor proper.
    // TODO: Clean up to make this field actually final.
    f_range_pattern: Vec<u16>,
    f_approximately_formatter: Option<NumberFormatterImpl>,

    f_plural_ranges: Arc<StandardPluralRanges>,
}

/// `identity2d(RangeIdentityFallback, RangeIdentityResult)`.
fn identity2d(a: RangeIdentityFallback, b: RangeIdentityResult) -> i32 {
    a.ordinal() | (b.ordinal() << 4)
}

/// `NumberRangeDataSink`.
struct NumberRangeDataSink<'a> {
    range_pattern: Option<Vec<u16>>,
    sb: &'a mut Vec<u16>,
}

impl NumberRangeDataSink<'_> {
    fn has_range_data(&self) -> bool {
        self.range_pattern.is_some()
    }

    fn is_complete(&self) -> bool {
        self.has_range_data()
    }

    fn fill_in_defaults(&mut self) -> Result<()> {
        if !self.has_range_data() {
            self.range_pattern = Some(SimpleFormatterImpl::compile_to_string_min_max_arguments(
                &units("{0}–{1}"),
                self.sb,
                2,
                2,
            )?);
        }
        Ok(())
    }
}

impl Sink for NumberRangeDataSink<'_> {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let misc_table = value.get_table()?;
        let mut i = 0;
        while misc_table.get_key_and_value(i, key, value) {
            if key.content_equals("range") && !self.has_range_data() {
                let pattern = value.get_string()?;
                self.range_pattern =
                    Some(SimpleFormatterImpl::compile_to_string_min_max_arguments(
                        &pattern, self.sb, 2, 2,
                    )?);
            }
            i += 1;
        }
        Ok(())
    }
}

/// `getNumberRangeData(ULocale, String, NumberRangeFormatterImpl)`: the
/// compiled range pattern.
fn get_number_range_data(locale: &ULocale, ns_name: &str) -> Result<Vec<u16>> {
    let mut sb_units: Vec<u16> = Vec::new();
    let resource =
        UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(locale))?;
    let resource = resource.as_icu()?.clone();
    let mut key = String::new();
    key.push_str("NumberElements/");
    key.push_str(ns_name);
    key.push_str("/miscPatterns");
    let mut sink = NumberRangeDataSink {
        range_pattern: None,
        sb: &mut sb_units,
    };
    match resource.get_all_items_with_fallback(&key, &mut sink) {
        Ok(()) => {}
        Err(e) if is_missing_resource(&e) => {
            // ignore; fall back to latn
        }
        Err(e) => return Err(e),
    }

    // Fall back to latn if necessary
    if !sink.is_complete() {
        resource.get_all_items_with_fallback("NumberElements/latn/miscPatterns", &mut sink)?;
    }

    sink.fill_in_defaults()?;

    Ok(sink.range_pattern.unwrap_or_default())
}

/// A modifier of the micros, dereferenced as Java does without a check.
fn modifier(m: &Option<Arc<dyn Modifier>>) -> &Arc<dyn Modifier> {
    m.as_ref()
        .unwrap_or_else(|| panic!("java.lang.NullPointerException: modifier"))
}

impl NumberRangeFormatterImpl {
    /// Equivalent to `NumberRangeFormatterImpl(RangeMacroProps)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Both formatters must have same numbering
    /// system")`; the errors of the formatter construction and of the locale
    /// data.
    pub fn new(macros: &RangeMacroProps) -> Result<Self> {
        let loc = macros.loc.clone().ok_or_else(|| null_pointer("loc"))?;
        let formatter1: LocalizedNumberFormatter = match macros.formatter1.as_ref() {
            Some(f) => f.locale(&loc),
            None => NumberFormatter::with_locale(&loc),
        };
        let formatter2: LocalizedNumberFormatter = match macros.formatter2.as_ref() {
            Some(f) => f.locale(&loc),
            None => NumberFormatter::with_locale(&loc),
        };
        let formatter_impl1 = NumberFormatterImpl::new(&formatter1.resolve())?;
        let formatter_impl2 = NumberFormatterImpl::new(&formatter2.resolve())?;
        let f_same_formatters = macros.same_formatters != 0;
        let f_collapse = macros.collapse.unwrap_or(RangeCollapse::Auto);
        let f_identity_fallback = macros
            .identity_fallback
            .unwrap_or(RangeIdentityFallback::Approximately);

        let ns_name = formatter_impl1.get_raw_micro_props().ns_name;
        let ns_name = match ns_name {
            Some(n)
                if f_same_formatters
                    || Some(&n) == formatter_impl2.get_raw_micro_props().ns_name.as_ref() =>
            {
                n
            }
            _ => {
                return Err(rucene::error::LuceneError::illegal_argument(
                    "Both formatters must have same numbering system",
                ))
            }
        };
        let f_range_pattern = get_number_range_data(&loc, &ns_name)?;

        // TODO: Support formatting approximately ranges on the same formatter
        let f_approximately_formatter = if f_same_formatters
            && (f_identity_fallback == RangeIdentityFallback::Approximately
                || f_identity_fallback == RangeIdentityFallback::ApproximatelyOrSingleValue)
        {
            let approximately_macros = MacroProps {
                approximately: Some(true),
                ..MacroProps::default()
            };
            Some(NumberFormatterImpl::new(
                &formatter1.macros(approximately_macros).resolve(),
            )?)
        } else {
            None
        };

        // TODO: Get locale from PluralRules instead?
        let f_plural_ranges = StandardPluralRanges::for_locale(&loc)?;

        Ok(NumberRangeFormatterImpl {
            formatter_impl1,
            formatter_impl2,
            f_same_formatters,
            f_collapse,
            f_identity_fallback,
            f_range_pattern,
            f_approximately_formatter,
            f_plural_ranges,
        })
    }

    /// Equivalent to `format(DecimalQuantity, DecimalQuantity, boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting pipelines.
    pub fn format(
        &self,
        mut quantity1: Box<dyn DecimalQuantity>,
        mut quantity2: Box<dyn DecimalQuantity>,
        equal_before_rounding: bool,
    ) -> Result<FormattedNumberRange> {
        let mut string = FormattedStringBuilder::new();
        let micros1 = self.formatter_impl1.pre_process(quantity1.as_mut())?;
        let micros2 = if self.f_same_formatters {
            self.formatter_impl1.pre_process(quantity2.as_mut())?
        } else {
            self.formatter_impl2.pre_process(quantity2.as_mut())?
        };

        // If any of the affixes are different, an identity is not possible
        // and we must use formatRange().
        // TODO: Write this as MicroProps operator==() ?
        // TODO: Avoid the redundancy of these equality operations with the
        // ones in formatRange?
        if !modifier(&micros1.mod_inner)
            .semantically_equivalent(modifier(&micros2.mod_inner).as_ref())
            || !modifier(&micros1.mod_middle)
                .semantically_equivalent(modifier(&micros2.mod_middle).as_ref())
            || !modifier(&micros1.mod_outer)
                .semantically_equivalent(modifier(&micros2.mod_outer).as_ref())
        {
            self.format_range(
                quantity1.as_mut(),
                quantity2.as_mut(),
                &mut string,
                &micros1,
                &micros2,
            )?;
            return Ok(FormattedNumberRange::new(
                string,
                quantity1,
                quantity2,
                RangeIdentityResult::NotEqual,
            ));
        }

        // Check for identity
        let identity_result = if equal_before_rounding {
            RangeIdentityResult::EqualBeforeRounding
        } else if quantity1.equals(quantity2.as_ref()) {
            RangeIdentityResult::EqualAfterRounding
        } else {
            RangeIdentityResult::NotEqual
        };

        // Java's switch over identity2d(fIdentityFallback, identityResult).
        match identity2d(self.f_identity_fallback, identity_result) {
            // RANGE, NOT_EQUAL | RANGE, EQUAL_AFTER_ROUNDING | RANGE, EQUAL_BEFORE_ROUNDING
            // | APPROXIMATELY, NOT_EQUAL | APPROXIMATE_OR_SINGLE_VALUE, NOT_EQUAL | SINGLE_VALUE, NOT_EQUAL
            0x23 | 0x13 | 0x03 | 0x22 | 0x21 | 0x20 => {
                self.format_range(
                    quantity1.as_mut(),
                    quantity2.as_mut(),
                    &mut string,
                    &micros1,
                    &micros2,
                )?;
            }
            // APPROXIMATELY, EQUAL_AFTER_ROUNDING | APPROXIMATELY, EQUAL_BEFORE_ROUNDING
            // | APPROXIMATE_OR_SINGLE_VALUE, EQUAL_AFTER_ROUNDING
            0x12 | 0x02 | 0x11 => {
                self.format_approximately(
                    quantity1.as_mut(),
                    quantity2.as_mut(),
                    &mut string,
                    &micros1,
                    &micros2,
                )?;
            }
            // APPROXIMATE_OR_SINGLE_VALUE, EQUAL_BEFORE_ROUNDING | SINGLE_VALUE, EQUAL_AFTER_ROUNDING
            // | SINGLE_VALUE, EQUAL_BEFORE_ROUNDING
            0x01 | 0x10 | 0x00 => {
                self.format_single_value(
                    quantity1.as_mut(),
                    quantity2.as_mut(),
                    &mut string,
                    &micros1,
                    &micros2,
                )?;
            }
            _ => debug_assert!(false),
        }

        Ok(FormattedNumberRange::new(
            string,
            quantity1,
            quantity2,
            identity_result,
        ))
    }

    fn format_single_value(
        &self,
        quantity1: &mut dyn DecimalQuantity,
        quantity2: &mut dyn DecimalQuantity,
        string: &mut FormattedStringBuilder,
        micros1: &MicroProps,
        micros2: &MicroProps,
    ) -> Result<()> {
        if self.f_same_formatters {
            let length = write_number(micros1, quantity1, string, 0);
            write_affixes(micros1, string, 0, length)?;
            Ok(())
        } else {
            self.format_range(quantity1, quantity2, string, micros1, micros2)
        }
    }

    fn format_approximately(
        &self,
        quantity1: &mut dyn DecimalQuantity,
        quantity2: &mut dyn DecimalQuantity,
        string: &mut FormattedStringBuilder,
        micros1: &MicroProps,
        micros2: &MicroProps,
    ) -> Result<()> {
        if self.f_same_formatters {
            // Re-format using the approximately formatter:
            quantity1.reset_exponent()?;
            let approximately = self.f_approximately_formatter.as_ref().unwrap_or_else(|| {
                panic!("java.lang.NullPointerException: fApproximatelyFormatter")
            });
            let micros_appx = approximately.pre_process(quantity1)?;
            let mut length = write_number(&micros_appx, quantity1, string, 0);
            // HEURISTIC: Desired modifier order: inner, middle, approximately, outer.
            length += modifier(&micros_appx.mod_inner).apply(string, 0, length)?;
            length += modifier(&micros_appx.mod_middle).apply(string, 0, length)?;
            modifier(&micros_appx.mod_outer).apply(string, 0, length)?;
            Ok(())
        } else {
            self.format_range(quantity1, quantity2, string, micros1, micros2)
        }
    }

    fn format_range(
        &self,
        quantity1: &mut dyn DecimalQuantity,
        quantity2: &mut dyn DecimalQuantity,
        string: &mut FormattedStringBuilder,
        micros1: &MicroProps,
        micros2: &MicroProps,
    ) -> Result<()> {
        // modInner is always notation (scientific); collapsable in ALL.
        // modOuter is always units; collapsable in ALL, AUTO, and UNIT.
        // modMiddle could be either; collapsable in ALL and sometimes AUTO and UNIT.
        // Never collapse an outer mod but not an inner mod.
        let (collapse_outer, mut collapse_middle, collapse_inner);
        match self.f_collapse {
            RangeCollapse::All | RangeCollapse::Auto | RangeCollapse::Unit => 'block: {
                // OUTER MODIFIER
                collapse_outer = modifier(&micros1.mod_outer)
                    .semantically_equivalent(modifier(&micros2.mod_outer).as_ref());
                if !collapse_outer {
                    // Never collapse inner mods if outer mods are not collapsable
                    collapse_middle = false;
                    collapse_inner = false;
                    break 'block;
                }

                // MIDDLE MODIFIER
                collapse_middle = modifier(&micros1.mod_middle)
                    .semantically_equivalent(modifier(&micros2.mod_middle).as_ref());
                if !collapse_middle {
                    // Never collapse inner mods if outer mods are not collapsable
                    collapse_inner = false;
                    break 'block;
                }

                // MIDDLE MODIFIER HEURISTICS
                // (could disable collapsing of the middle modifier)
                // The modifiers are equal by this point, so we can look at just one of them.
                let mm = modifier(&micros1.mod_middle);
                if self.f_collapse == RangeCollapse::Unit {
                    // Only collapse if the modifier is a unit.
                    // TODO: Make a better way to check for a unit?
                    // TODO: Handle case where the modifier has both notation and unit (compact currency)?
                    if !mm.contains_field(&nf_field::CURRENCY as FieldRef)
                        && !mm.contains_field(&nf_field::PERCENT as FieldRef)
                    {
                        collapse_middle = false;
                    }
                } else if self.f_collapse == RangeCollapse::Auto {
                    // Heuristic as of ICU 63: collapse only if the modifier is more than one code point.
                    if mm.get_code_point_count()? <= 1 {
                        collapse_middle = false;
                    }
                }

                if !collapse_middle || self.f_collapse != RangeCollapse::All {
                    collapse_inner = false;
                    break 'block;
                }

                // INNER MODIFIER
                collapse_inner = modifier(&micros1.mod_inner)
                    .semantically_equivalent(modifier(&micros2.mod_inner).as_ref());

                // All done checking for collapsibility.
            }
            _ => {
                collapse_outer = false;
                collapse_middle = false;
                collapse_inner = false;
            }
        }

        let mut h = PrefixInfixSuffixLengthHelper::default();

        SimpleModifier::format_two_arg_pattern(&self.f_range_pattern, string, 0, &mut h, None)?;
        debug_assert!(h.length_infix > 0);

        // SPACING HEURISTIC
        // Add spacing unless all modifiers are collapsed.
        // TODO: add API to control this?
        // TODO: Use a data-driven heuristic to determine this.
        {
            let repeat_inner =
                !collapse_inner && modifier(&micros1.mod_inner).get_code_point_count()? > 0;
            let repeat_middle =
                !collapse_middle && modifier(&micros1.mod_middle).get_code_point_count()? > 0;
            let repeat_outer =
                !collapse_outer && modifier(&micros1.mod_outer).get_code_point_count()? > 0;
            if repeat_inner || repeat_middle || repeat_outer {
                // Add spacing if there is not already spacing
                if !PatternProps::is_white_space(i32::from(string.char_at(h.index1()))) {
                    h.length_infix += string.insert_code_point(h.index1(), 0x20, None);
                }
                if !PatternProps::is_white_space(i32::from(string.char_at(h.index2() - 1))) {
                    h.length_infix += string.insert_code_point(h.index2(), 0x20, None);
                }
            }
        }

        h.length1 += write_number(micros1, quantity1, string, h.index0());
        // ICU-21684: Write the second number to a temp string to avoid repeated insert operations
        let mut temp_string = FormattedStringBuilder::new();
        write_number(micros2, quantity2, &mut temp_string, 0);
        h.length2 += string.insert_builder(h.index2(), &temp_string);

        // TODO: Support padding?

        if collapse_inner {
            // Note: this is actually a mix of prefix and suffix, but adding to infix length works
            let m = self.resolve_modifier_plurals(
                modifier(&micros1.mod_inner),
                modifier(&micros2.mod_inner),
            );
            h.length_suffix += m.apply(string, h.index0(), h.index4())?;
            h.length_prefix += m.get_prefix_length()?;
            h.length_suffix -= m.get_prefix_length()?;
        } else {
            h.length1 += modifier(&micros1.mod_inner).apply(string, h.index0(), h.index1())?;
            h.length2 += modifier(&micros2.mod_inner).apply(string, h.index2(), h.index4())?;
        }

        if collapse_middle {
            // Note: this is actually a mix of prefix and suffix, but adding to infix length works
            let m = self.resolve_modifier_plurals(
                modifier(&micros1.mod_middle),
                modifier(&micros2.mod_middle),
            );
            h.length_suffix += m.apply(string, h.index0(), h.index4())?;
            h.length_prefix += m.get_prefix_length()?;
            h.length_suffix -= m.get_prefix_length()?;
        } else {
            h.length1 += modifier(&micros1.mod_middle).apply(string, h.index0(), h.index1())?;
            h.length2 += modifier(&micros2.mod_middle).apply(string, h.index2(), h.index4())?;
        }

        if collapse_outer {
            // Note: this is actually a mix of prefix and suffix, but adding to infix length works
            let m = self.resolve_modifier_plurals(
                modifier(&micros1.mod_outer),
                modifier(&micros2.mod_outer),
            );
            h.length_suffix += m.apply(string, h.index0(), h.index4())?;
            h.length_prefix += m.get_prefix_length()?;
            h.length_suffix -= m.get_prefix_length()?;
        } else {
            h.length1 += modifier(&micros1.mod_outer).apply(string, h.index0(), h.index1())?;
            h.length2 += modifier(&micros2.mod_outer).apply(string, h.index2(), h.index4())?;
        }

        // Now that all pieces are added, save the span info.
        FormattedValueStringBuilderImpl::apply_span_range(
            string,
            &span_field::NUMBER_RANGE_SPAN,
            Some(AttributeValue::Integer(0)),
            h.index0(),
            h.index1(),
        );
        FormattedValueStringBuilderImpl::apply_span_range(
            string,
            &span_field::NUMBER_RANGE_SPAN,
            Some(AttributeValue::Integer(1)),
            h.index2(),
            h.index3(),
        );
        Ok(())
    }

    /// Equivalent to `resolveModifierPlurals(Modifier, Modifier)`.
    fn resolve_modifier_plurals(
        &self,
        first: &Arc<dyn Modifier>,
        second: &Arc<dyn Modifier>,
    ) -> Arc<dyn Modifier> {
        let Some(first_parameters) = first.get_parameters() else {
            // No plural form; return a fallback (e.g., the first)
            return first.clone();
        };
        let Some(second_parameters) = second.get_parameters() else {
            // No plural form; return a fallback (e.g., the first)
            return first.clone();
        };

        // Get the required plural form from data
        let npe = || -> ! { panic!("java.lang.NullPointerException") };
        let result_plural = self.f_plural_ranges.resolve(
            first_parameters.plural.unwrap_or_else(|| npe()),
            second_parameters.plural.unwrap_or_else(|| npe()),
        );

        // Get and return the new Modifier
        let obj = first_parameters.obj.as_ref().unwrap_or_else(|| npe());
        let m = obj.get_modifier(
            first_parameters.signum.unwrap_or_else(|| npe()),
            result_plural,
        );
        debug_assert!(m.is_some());
        m.unwrap_or_else(|| npe())
    }
}
