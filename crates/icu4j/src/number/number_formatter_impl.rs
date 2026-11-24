// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberFormatterImpl`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::text::FieldRef;
use crate::number::compact_notation::CompactNotation;
use crate::number::integer_width::IntegerWidth;
use crate::number::number_formatter::{
    DecimalSeparatorDisplay, GroupingStrategy, SignDisplay, UnitWidth,
};
use crate::number::precision::{self, Precision};
use crate::number::scientific_notation::ScientificNotation;
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::number::affix_pattern_provider::AffixPatternProvider;
use crate::r#impl::number::compact_data::CompactType;
use crate::r#impl::number::constant_affix_modifier;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::DecimalQuantity_DualStorageBCD;
use crate::r#impl::number::grouper::Grouper;
use crate::r#impl::number::long_name_handler::LongNameHandler;
use crate::r#impl::number::long_name_multiplexer::LongNameMultiplexer;
use crate::r#impl::number::macro_props::{GroupingObject, MacroProps, SymbolsObject};
use crate::r#impl::number::micro_props::{MicroProps, MicroPropsHead};
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::mixed_unit_long_name_handler::MixedUnitLongNameHandler;
use crate::r#impl::number::modifier::Modifier;
use crate::r#impl::number::multiplier_format_handler::MultiplierFormatHandler;
use crate::r#impl::number::mutable_pattern_modifier::MutablePatternModifier;
use crate::r#impl::number::padder::{self, Padder};
use crate::r#impl::number::pattern_string_parser::PatternStringParser;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::r#impl::number::unit_conversion_handler::UnitConversionHandler;
use crate::r#impl::number::usage_prefs_handler::UsagePrefsHandler;
use crate::r#impl::standard_plural::StandardPlural;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::number_format::{self, field as nf_field};
use crate::text::numbering_system::NumberingSystem;
use crate::text::plural_rules::PluralRules;
use crate::util::currency::Currency;
use crate::util::measure_unit::{Complexity, MeasureUnit};

/// `DEFAULT_CURRENCY`.
static DEFAULT_CURRENCY: LazyLock<Currency> =
    LazyLock::new(|| match Currency::get_instance(Some("XXX")) {
        Ok(c) => c,
        Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
    });

/// `Some(FieldObject::Field(field))`.
fn fo(field: FieldRef) -> Option<FieldObject> {
    Some(FieldObject::Field(field))
}

/// This is the "brain" of the number formatting pipeline. It ties all the
/// pieces together, taking in a MacroProps and a DecimalQuantity and
/// outputting a properly formatted number string.
///
/// This class, as well as NumberPropertyMapper, could go into the impl
/// package, but they depend on too many package-private members of the
/// public APIs.
///
/// Equivalent to `com.ibm.icu.number.NumberFormatterImpl` (a package-private
/// class).
#[derive(Debug)]
pub struct NumberFormatterImpl {
    /// `micros`.
    micros: Arc<MicroPropsHead>,
    /// `microPropsGenerator`.
    micro_props_generator: Arc<dyn MicroPropsGenerator>,
}

impl NumberFormatterImpl {
    /// Builds a "safe" MicroPropsGenerator, which is thread-safe and can be
    /// used repeatedly. Equivalent to `NumberFormatterImpl(MacroProps)`.
    ///
    /// # Errors
    ///
    /// The errors of the data loading and of the settings validation.
    pub fn new(macros: &MacroProps) -> Result<Self> {
        let micros = MicroPropsHead::new(MicroProps::new(true));
        let micro_props_generator = macros_to_micro_generator(macros, &micros, true)?;
        Ok(NumberFormatterImpl {
            micros,
            micro_props_generator,
        })
    }

    /// Builds and evaluates an "unsafe" MicroPropsGenerator, which is
    /// cheaper but can be used only once. Equivalent to
    /// `formatStatic(MacroProps, DecimalQuantity, FormattedStringBuilder)`.
    ///
    /// # Errors
    ///
    /// The errors of the data loading and of the pipeline.
    pub fn format_static(
        macros: &MacroProps,
        in_value: &mut dyn DecimalQuantity,
        out_string: &mut FormattedStringBuilder,
    ) -> Result<MicroProps> {
        let result = pre_process_unsafe(macros, in_value)?;
        let length = write_number(&result, in_value, out_string, 0);
        write_affixes(&result, out_string, 0, length)?;
        Ok(result)
    }

    /// Prints only the prefix and suffix; used for DecimalFormat getters.
    /// Equivalent to `getPrefixSuffixStatic(MacroProps, byte,
    /// StandardPlural, FormattedStringBuilder)`: the length of the prefix.
    ///
    /// # Errors
    ///
    /// The errors of the data loading and of the pipeline.
    pub fn get_prefix_suffix_static(
        macros: &MacroProps,
        signum: i8,
        _plural: StandardPlural,
        output: &mut FormattedStringBuilder,
    ) -> Result<i32> {
        let micros = MicroPropsHead::new(MicroProps::new(false));
        let micro_props_generator = macros_to_micro_generator(macros, &micros, false)?;
        get_prefix_suffix_impl(micro_props_generator.as_ref(), signum, output)
    }

    /// Evaluates the "safe" MicroPropsGenerator created by "fromMacros".
    /// Equivalent to `format(DecimalQuantity, FormattedStringBuilder)`.
    ///
    /// # Errors
    ///
    /// The errors of the pipeline.
    pub fn format(
        &self,
        in_value: &mut dyn DecimalQuantity,
        out_string: &mut FormattedStringBuilder,
    ) -> Result<MicroProps> {
        let result = self.pre_process(in_value)?;
        let length = write_number(&result, in_value, out_string, 0);
        write_affixes(&result, out_string, 0, length)?;
        Ok(result)
    }

    /// Like format(), but saves the result into an output MicroProps
    /// without additional processing. Equivalent to
    /// `preProcess(DecimalQuantity)`.
    ///
    /// # Errors
    ///
    /// The errors of the pipeline.
    pub fn pre_process(&self, in_value: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let micros = self.micro_props_generator.process_quantity(in_value)?;
        apply_integer_width(&micros, in_value);
        Ok(micros)
    }

    /// Equivalent to `getPrefixSuffix(byte, StandardPlural,
    /// FormattedStringBuilder)`.
    ///
    /// # Errors
    ///
    /// The errors of the pipeline.
    pub fn get_prefix_suffix(
        &self,
        signum: i8,
        _plural: StandardPlural,
        output: &mut FormattedStringBuilder,
    ) -> Result<i32> {
        get_prefix_suffix_impl(self.micro_props_generator.as_ref(), signum, output)
    }

    /// Equivalent to `getRawMicroProps()`: the quantity-independent micros
    /// (a copy of the immutable head object).
    pub fn get_raw_micro_props(&self) -> MicroProps {
        self.micros.lock().clone()
    }
}

/// The integer-width step shared by `preProcess` and `preProcessUnsafe`.
fn apply_integer_width(micros: &MicroProps, in_value: &mut dyn DecimalQuantity) {
    let iw = micros
        .integer_width
        .as_ref()
        .unwrap_or_else(|| panic!("java.lang.NullPointerException: integerWidth"));
    if iw.max_int() == -1 {
        in_value.set_min_integer(iw.min_int());
    } else {
        in_value.set_min_integer(iw.min_int());
        in_value.apply_max_integer(iw.max_int());
    }
}

/// `preProcessUnsafe(MacroProps, DecimalQuantity)`.
fn pre_process_unsafe(
    macros: &MacroProps,
    in_value: &mut dyn DecimalQuantity,
) -> Result<MicroProps> {
    let micros_head = MicroPropsHead::new(MicroProps::new(false));
    let micro_props_generator = macros_to_micro_generator(macros, &micros_head, false)?;
    let micros = micro_props_generator.process_quantity(in_value)?;
    apply_integer_width(&micros, in_value);
    Ok(micros)
}

/// `getPrefixSuffixImpl(MicroPropsGenerator, byte, FormattedStringBuilder)`.
fn get_prefix_suffix_impl(
    generator: &dyn MicroPropsGenerator,
    signum: i8,
    output: &mut FormattedStringBuilder,
) -> Result<i32> {
    // TODO: Use a static code path for this?
    let mut quantity = DecimalQuantity_DualStorageBCD::from_int(0);
    if signum < 0 {
        quantity.negate();
    }
    let micros = generator.process_quantity(&mut quantity)?;
    let mod_middle = micros
        .mod_middle
        .as_ref()
        .unwrap_or_else(|| panic!("java.lang.NullPointerException: modMiddle"));
    mod_middle.apply(output, 0, 0)?;
    mod_middle.get_prefix_length()
}

/// `unitIsCurrency(MeasureUnit)`.
fn unit_is_currency(unit: Option<&MeasureUnit>) -> bool {
    // TODO: Check using "instanceof" operator instead?
    unit.is_some_and(|u| u.get_type() == Some("currency"))
}

/// `unitIsBaseUnit(MeasureUnit)`.
fn unit_is_base_unit(unit: Option<&MeasureUnit>) -> bool {
    unit.is_none()
}

/// `unitIsPercent(MeasureUnit)`.
fn unit_is_percent(unit: Option<&MeasureUnit>) -> bool {
    unit.is_some_and(|u| u.get_subtype() == Some("percent"))
}

/// `unitIsPermille(MeasureUnit)`.
fn unit_is_permille(unit: Option<&MeasureUnit>) -> bool {
    unit.is_some_and(|u| u.get_subtype() == Some("permille"))
}

/// Synthesizes the MacroProps into a MicroPropsGenerator. All information,
/// including the locale, is encoded into the MicroPropsGenerator, except
/// for the quantity itself, which is left abstract and must be provided to
/// the returned MicroPropsGenerator instance. Equivalent to
/// `macrosToMicroGenerator(MacroProps, MicroProps, boolean)`: `safe` tells
/// whether the generator must be thread-safe (`micros` is the head of the
/// chain).
fn macros_to_micro_generator(
    macros: &MacroProps,
    micros_head: &Arc<MicroPropsHead>,
    safe: bool,
) -> Result<Arc<dyn MicroPropsGenerator>> {
    let mut chain: Arc<dyn MicroPropsGenerator> = micros_head.clone();
    let mut micros = micros_head.lock();

    // TODO: Normalize the currency (accept symbols from DecimalFormatSymbols)?
    // currency = CustomSymbolCurrency.resolve(currency, input.loc,
    // micros.symbols);

    // Pre-compute a few values for efficiency.
    let unit = macros.unit.as_ref();
    let is_currency = unit_is_currency(unit);
    let is_base_unit = unit_is_base_unit(unit);
    let is_percent = unit_is_percent(unit);
    let is_permille = unit_is_permille(unit);
    let is_compact_notation = macros
        .notation
        .as_ref()
        .is_some_and(|n| CompactNotation::try_cast(n).is_some());
    let is_accounting = matches!(
        macros.sign,
        Some(SignDisplay::Accounting)
            | Some(SignDisplay::AccountingAlways)
            | Some(SignDisplay::AccountingExceptZero)
            | Some(SignDisplay::AccountingNegative)
    );
    let currency: Currency = if is_currency {
        Currency::cast(unit.cloned().expect("a currency unit is present"))
    } else {
        DEFAULT_CURRENCY.clone()
    };
    let unit_width = macros.unit_width.unwrap_or(UnitWidth::Short);
    // Use CLDR unit data for all MeasureUnits (not currency and not
    // no-unit), except use the dedicated percent pattern for percent and
    // permille. However, use the CLDR unit data for percent/permille if a
    // long name was requested OR if compact notation is being used, since
    // compact notation overrides the middle modifier (micros.modMiddle)
    // normally used for the percent pattern.
    let is_cldr_unit = !is_currency
        && !is_base_unit
        && (unit_width == UnitWidth::FullName
            || !(is_percent || is_permille)
            || is_compact_notation);
    let is_mixed_unit = is_cldr_unit
        && unit.is_some_and(|u| u.get_type().is_none())
        && unit.expect("a CLDR unit is present").get_complexity()? == Complexity::Mixed;

    // Java NPE when no locale was set.
    let loc = macros
        .loc
        .clone()
        .ok_or_else(|| crate::jdk::null_pointer("locale"))?;

    let mut rules: Option<Arc<PluralRules>> = macros.rules.clone();

    // Select the numbering system.
    let ns: Arc<NumberingSystem> = match &macros.symbols {
        Some(SymbolsObject::NumberingSystem(ns)) => ns.clone(),
        // TODO: Is there a way to avoid creating the NumberingSystem
        // object?
        _ => NumberingSystem::get_instance_for(&loc)?,
    };
    micros.ns_name = ns.get_name().map(str::to_string);

    // Default gender: none.
    micros.gender = Some(String::new());

    // Resolve the symbols. Do this here because currency may need to
    // customize them.
    match &macros.symbols {
        Some(SymbolsObject::Symbols(s)) => micros.symbols = Some(s.clone()),
        _ => {
            let mut symbols = DecimalFormatSymbols::for_numbering_system(&loc, &ns)?;
            if is_currency {
                symbols.set_currency(Some(currency.clone()))?;
            }
            micros.symbols = Some(Arc::new(symbols));
        }
    }
    let symbols = micros.symbols().clone();
    let ns_name = micros.ns_name.clone();

    // Load and parse the pattern string. It is used for grouping sizes and
    // affixes only.
    // TODO: Even though the pattern is not being used for most units, it
    // is still being loaded.
    let mut pattern: Option<Vec<u16>> = None;
    if is_currency {
        if let Some(cp) = symbols.get_currency_pattern() {
            pattern = Some(cp.to_vec());
        }
    }
    let pattern = match pattern {
        Some(p) => p,
        None => {
            let pattern_style = if is_cldr_unit {
                number_format::NUMBERSTYLE
            } else if is_percent || is_permille {
                number_format::PERCENTSTYLE
            } else if !is_currency || unit_width == UnitWidth::FullName {
                number_format::NUMBERSTYLE
            } else if is_accounting {
                // NOTE: Although ACCOUNTING and ACCOUNTING_ALWAYS are only
                // supported in currencies right now, the API contract
                // allows us to add support to other units in the future.
                number_format::ACCOUNTINGCURRENCYSTYLE
            } else {
                number_format::CURRENCYSTYLE
            };
            number_format::get_pattern_for_style_and_numbering_system(
                &loc,
                ns_name.as_deref().unwrap_or("null"),
                pattern_style,
            )?
        }
    };
    let pattern_info = Arc::new(PatternStringParser::parse_to_pattern_info(&pattern)?);

    // START POPULATING THE DEFAULT MICROPROPS AND BUILDING THE MICROPROPS
    // GENERATOR

    // Unit Preferences and Conversions as our first step
    let mut usage_prefs_handler: Option<Arc<UsagePrefsHandler>> = None;
    if let Some(usage) = &macros.usage {
        if !is_cldr_unit {
            return Err(IllegalIcuArgumentException::with_message(
                "We only support \"usage\" when the input unit is specified, and is a CLDR Unit.",
            )
            .error());
        }
        let handler = Arc::new(UsagePrefsHandler::new(
            &loc,
            unit.expect("a CLDR unit is present"),
            usage,
            chain,
        )?);
        usage_prefs_handler = Some(handler.clone());
        chain = handler;
    } else if is_mixed_unit {
        chain = Arc::new(UnitConversionHandler::new(
            unit.expect("a mixed unit is present"),
            chain,
        )?);
    }

    // Multiplier
    if let Some(scale) = &macros.scale {
        chain = Arc::new(MultiplierFormatHandler::new(scale.clone(), chain));
    }

    // Rounding strategy
    let mut rounder = if let Some(p) = &macros.precision {
        p.clone()
    } else if is_compact_notation {
        precision::COMPACT_STRATEGY.clone()
    } else if is_currency {
        precision::MONETARY_STANDARD.clone()
    } else if macros.usage.is_some() {
        // Bogus Precision - it will get set in the UnitConversionHandler
        // instead
        precision::BOGUS_PRECISION.clone()
    } else {
        precision::DEFAULT_MAX_FRAC_6.clone()
    };
    if let Some(rounding_mode) = macros.rounding_mode {
        rounder = rounder.with_mode(&RoundingUtils::math_context_unlimited(rounding_mode));
    }
    let rounder: Precision = rounder.with_locale_data(&currency)?;
    micros.rounder = Some(rounder);

    // Grouping strategy
    let grouping = match &macros.grouping {
        Some(GroupingObject::Grouper(g)) => g.clone(),
        Some(GroupingObject::Strategy(s)) => Grouper::for_strategy(*s),
        None if is_compact_notation => {
            // Compact notation uses minGrouping by default since ICU 59
            Grouper::for_strategy(GroupingStrategy::Min2)
        }
        None => Grouper::for_strategy(GroupingStrategy::Auto),
    };
    micros.grouping = Some(grouping.with_locale_data(&loc, &pattern_info)?);

    // Padding strategy
    micros.padding = Some(match &macros.padder {
        Some(p) => p.clone(),
        None => padder::NONE.clone(),
    });

    // Integer width
    micros.integer_width = Some(match &macros.integer_width {
        Some(iw) => iw.clone(),
        None => IntegerWidth::default_width(),
    });

    // Sign display
    micros.sign = Some(macros.sign.unwrap_or(SignDisplay::Auto));

    // Decimal mark display
    micros.decimal = Some(macros.decimal.unwrap_or(DecimalSeparatorDisplay::Auto));

    // Use monetary separator symbols
    micros.use_currency = is_currency;

    // Inner modifier (scientific notation)
    match macros
        .notation
        .as_ref()
        .and_then(ScientificNotation::try_cast)
    {
        Some(sci) => {
            chain = sci.with_locale_data(symbols.clone(), safe, chain);
        }
        None => {
            // No inner modifier required
            micros.mod_inner = Some(constant_affix_modifier::EMPTY.clone());
        }
    }

    // Middle modifier (patterns, positive/negative, currency symbols,
    // percent)
    let pattern_mod = MutablePatternModifier::new(false);

    // The default affix provider is the pattern info, unless compact
    // notation is used with a different currency status.
    let affix_provider: Arc<dyn AffixPatternProvider> = match &macros.affix_provider {
        Some(p) if !is_compact_notation || is_currency == p.has_currency_sign() => p.clone(),
        _ => pattern_info.clone(),
    };
    pattern_mod.set_pattern_info(affix_provider.clone(), None);
    let approximately = macros.approximately.unwrap_or(false);
    pattern_mod.set_pattern_attributes(micros.sign.expect("set above"), is_permille, approximately);
    if pattern_mod.needs_plurals()? {
        if rules.is_none() {
            // Lazily create PluralRules
            rules = Some(PluralRules::for_locale(&loc)?);
        }
        pattern_mod.set_symbols(symbols.clone(), currency.clone(), unit_width, rules.clone());
    } else {
        pattern_mod.set_symbols(symbols.clone(), currency.clone(), unit_width, None);
    }
    let imm_pattern_mod = if safe {
        Some(pattern_mod.create_immutable()?)
    } else {
        None
    };

    // currencyAsDecimal
    if affix_provider.currency_as_decimal() {
        micros.currency_as_decimal = Some(pattern_mod.get_currency_symbol_for_unit_width()?);
    }

    // Outer modifier (CLDR units and currency long names)
    if is_cldr_unit {
        let unit = unit.expect("a CLDR unit is present");
        let unit_display_case: Option<&str> = macros.unit_display_case.as_deref();
        if rules.is_none() {
            // Lazily create PluralRules
            rules = Some(PluralRules::for_locale(&loc)?);
        }
        let plural_rules: Arc<PluralRules> = match &macros.rules {
            Some(r) => r.clone(),
            None => PluralRules::for_locale(&loc)?,
        };

        if macros.usage.is_some() {
            debug_assert!(usage_prefs_handler.is_some());
            let handler = usage_prefs_handler.as_ref().expect("set with the usage");
            chain = Arc::new(LongNameMultiplexer::for_measure_units(
                &loc,
                handler.get_output_units(),
                unit_width,
                unit_display_case,
                &plural_rules,
                chain,
            )?);
        } else if is_mixed_unit {
            chain = MixedUnitLongNameHandler::for_measure_unit(
                &loc,
                unit,
                unit_width,
                unit_display_case,
                &plural_rules,
                Some(chain),
            )?;
        } else {
            let mut unit = unit.clone();
            if let Some(per_unit) = &macros.per_unit {
                unit = unit.product(Some(&per_unit.reciprocal()?))?;
                // This isn't strictly necessary, but was what we produced
                // for ICU 67:
                if unit.get_type().is_none()
                    && (macros.unit.as_ref().expect("present").get_type().is_none()
                        || per_unit.get_type().is_none())
                {
                    return Err(LuceneError::unsupported_operation(
                        "perUnit() can only be used if unit and perUnit are both built-ins, or the combination is a built-in",
                    ));
                }
            }
            chain = LongNameHandler::for_measure_unit(
                &loc,
                &unit,
                unit_width,
                unit_display_case,
                &plural_rules,
                Some(chain),
            )?;
        }
    } else if is_currency && unit_width == UnitWidth::FullName {
        if rules.is_none() {
            // Lazily create PluralRules
            rules = Some(PluralRules::for_locale(&loc)?);
        }
        chain = LongNameHandler::for_currency_long_names(
            &loc,
            &currency,
            rules.as_ref().expect("set above"),
            Some(chain),
        )?;
    } else {
        // No outer modifier required
        micros.mod_outer = Some(constant_affix_modifier::EMPTY.clone());
    }

    // Compact notation
    if is_compact_notation {
        if rules.is_none() {
            // Lazily create PluralRules
            rules = Some(PluralRules::for_locale(&loc)?);
        }
        let compact_type = if macros
            .unit
            .as_ref()
            .is_some_and(|u| Currency::try_cast(u).is_some())
            && macros.unit_width != Some(UnitWidth::FullName)
        {
            CompactType::Currency
        } else {
            CompactType::Decimal
        };
        let compact = CompactNotation::try_cast(
            macros
                .notation
                .as_ref()
                .expect("compact notation is present"),
        )
        .expect("compact notation");
        chain = compact.with_locale_data(
            &loc,
            ns_name.as_deref().unwrap_or("null"),
            compact_type,
            rules.clone().expect("set above"),
            &pattern_mod,
            safe,
            chain,
        )?;
    }

    // Always add the pattern modifier as the last element of the chain.
    drop(micros);
    if let Some(imm) = imm_pattern_mod {
        chain = imm.add_to_chain(chain);
    } else {
        chain = pattern_mod.add_to_chain(chain);
    }

    Ok(chain)
}

// END OF MICROPROPS GENERATOR / START OF MICROPROPS RENDERING

/// Adds the affixes. Intended to be called immediately after
/// formatNumber. Equivalent to `writeAffixes(MicroProps,
/// FormattedStringBuilder, int, int)`.
///
/// # Errors
///
/// The errors of the modifiers.
pub fn write_affixes(
    micros: &MicroProps,
    string: &mut FormattedStringBuilder,
    start: i32,
    end: i32,
) -> Result<i32> {
    let npe = |name: &str| -> ! { panic!("java.lang.NullPointerException: {name}") };
    let mod_inner: &dyn Modifier = micros
        .mod_inner
        .as_deref()
        .unwrap_or_else(|| npe("modInner"));
    let mod_middle: &dyn Modifier = micros
        .mod_middle
        .as_deref()
        .unwrap_or_else(|| npe("modMiddle"));
    let mod_outer: &dyn Modifier = micros
        .mod_outer
        .as_deref()
        .unwrap_or_else(|| npe("modOuter"));
    let padding: &Padder = micros.padding.as_ref().unwrap_or_else(|| npe("padding"));
    // Always apply the inner modifier (which is "strong").
    let mut length = mod_inner.apply(string, start, end)?;
    if padding.is_valid() {
        padding.pad_and_apply(mod_middle, mod_outer, string, start, end + length)?;
    } else {
        length += mod_middle.apply(string, start, end + length)?;
        length += mod_outer.apply(string, start, end + length)?;
    }
    Ok(length)
}

/// Synthesizes the output string from a MicroProps and DecimalQuantity.
/// This method formats only the main number, not affixes. Equivalent to
/// `writeNumber(MicroProps, DecimalQuantity, FormattedStringBuilder, int)`.
pub fn write_number(
    micros: &MicroProps,
    quantity: &dyn DecimalQuantity,
    string: &mut FormattedStringBuilder,
    index: i32,
) -> i32 {
    let symbols = micros.symbols();
    let mut length = 0;
    if quantity.is_infinite() {
        length += string.insert(
            length + index,
            symbols.get_infinity(),
            fo(&nf_field::INTEGER),
        );
    } else if quantity.is_nan() {
        length += string.insert(length + index, symbols.get_nan(), fo(&nf_field::INTEGER));
    } else {
        // Add the integer digits
        length += write_integer_digits(micros, quantity, string, length + index);

        // Add the decimal point
        if quantity.get_lower_display_magnitude() < 0
            || micros.decimal == Some(DecimalSeparatorDisplay::Always)
        {
            if let Some(cad) = &micros.currency_as_decimal {
                // Note: This assumes that the currency code is not empty.
                length += string.insert(length + index, cad, fo(&nf_field::CURRENCY));
            } else if micros.use_currency {
                length += string.insert(
                    length + index,
                    symbols.get_monetary_decimal_separator_string(),
                    fo(&nf_field::DECIMAL_SEPARATOR),
                );
            } else {
                length += string.insert(
                    length + index,
                    symbols.get_decimal_separator_string(),
                    fo(&nf_field::DECIMAL_SEPARATOR),
                );
            }
        }

        // Add the fraction digits
        length += write_fraction_digits(micros, quantity, string, length + index);

        if length == 0 {
            // Force output of the digit for value 0
            if symbols.get_code_point_zero() != -1 {
                length += string.insert_code_point(
                    index,
                    symbols.get_code_point_zero(),
                    fo(&nf_field::INTEGER),
                );
            } else {
                length += string.insert(
                    index,
                    &symbols.get_digit_strings_local()[0],
                    fo(&nf_field::INTEGER),
                );
            }
        }
    }

    length
}

/// `writeIntegerDigits(MicroProps, DecimalQuantity, FormattedStringBuilder,
/// int)`.
fn write_integer_digits(
    micros: &MicroProps,
    quantity: &dyn DecimalQuantity,
    string: &mut FormattedStringBuilder,
    index: i32,
) -> i32 {
    let symbols = micros.symbols();
    let grouping = micros
        .grouping
        .as_ref()
        .unwrap_or_else(|| panic!("java.lang.NullPointerException: grouping"));
    let mut length = 0;
    let integer_count = quantity.get_upper_display_magnitude() + 1;
    for i in 0..integer_count {
        // Add grouping separator
        if grouping.group_at_position(i, quantity) {
            let sep = if micros.use_currency {
                symbols.get_monetary_grouping_separator_string()
            } else {
                symbols.get_grouping_separator_string()
            };
            length += string.insert(index, sep, fo(&nf_field::GROUPING_SEPARATOR));
        }

        // Get and append the next digit value
        let next_digit = quantity.get_digit(i);
        // Get and append the next digit value
        if symbols.get_code_point_zero() != -1 {
            length += string.insert_code_point(
                index,
                symbols.get_code_point_zero() + next_digit as i32,
                fo(&nf_field::INTEGER),
            );
        } else {
            length += string.insert(
                index,
                &symbols.get_digit_strings_local()[next_digit as usize],
                fo(&nf_field::INTEGER),
            );
        }
    }
    length
}

/// `writeFractionDigits(MicroProps, DecimalQuantity, FormattedStringBuilder,
/// int)`.
fn write_fraction_digits(
    micros: &MicroProps,
    quantity: &dyn DecimalQuantity,
    string: &mut FormattedStringBuilder,
    index: i32,
) -> i32 {
    let symbols = micros.symbols();
    let mut length = 0;
    let fraction_count = -quantity.get_lower_display_magnitude();
    for i in 0..fraction_count {
        // Get and append the next digit value
        let next_digit = quantity.get_digit(-i - 1);
        if symbols.get_code_point_zero() != -1 {
            length += string.insert_code_point(
                length + index,
                symbols.get_code_point_zero() + next_digit as i32,
                fo(&nf_field::FRACTION),
            );
        } else {
            length += string.insert(
                length + index,
                &symbols.get_digit_strings_local()[next_digit as usize],
                fo(&nf_field::FRACTION),
            );
        }
    }
    length
}
