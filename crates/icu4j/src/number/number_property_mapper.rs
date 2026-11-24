// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberPropertyMapper`.

use std::sync::Arc;

use rucene::error::Result;

use crate::number::compact_notation::CompactNotation;
use crate::number::currency_precision::CurrencyPrecision;
use crate::number::fraction_precision::FractionPrecision;
use crate::number::integer_width::IntegerWidth;
use crate::number::notation::Notation;
use crate::number::number_formatter::{DecimalSeparatorDisplay, NumberFormatter, SignDisplay};
use crate::number::number_formatter_settings::NumberFormatterSettings;
use crate::number::precision::{Precision, PrecisionKind};
use crate::number::scientific_notation::ScientificNotation;
use crate::number::unlocalized_number_formatter::UnlocalizedNumberFormatter;
use crate::r#impl::number::affix_pattern_provider::AffixPatternProvider;
use crate::r#impl::number::custom_symbol_currency::CustomSymbolCurrency;
use crate::r#impl::number::decimal_format_properties::DecimalFormatProperties;
use crate::r#impl::number::grouper::Grouper;
use crate::r#impl::number::macro_props::{GroupingObject, MacroProps, SymbolsObject};
use crate::r#impl::number::padder::Padder;
use crate::r#impl::number::pattern_string_parser::PatternStringParser;
use crate::r#impl::number::pattern_string_utils::PatternStringUtils;
use crate::r#impl::number::properties_affix_pattern_provider::PropertiesAffixPatternProvider;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::text::compact_decimal_format::CompactStyle;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::util::currency::CurrencyUsage;

/// This class, as well as NumberFormatterImpl, could go into the impl
/// package, but they depend on too many package-private members of the
/// public APIs.
///
/// Equivalent to `com.ibm.icu.number.NumberPropertyMapper` (a
/// package-private final class with static members only).
pub struct NumberPropertyMapper;

impl NumberPropertyMapper {
    /// Convenience method to create a NumberFormatter directly from
    /// Properties. Equivalent to `create(DecimalFormatProperties,
    /// DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// The errors of [`old_to_new`](Self::old_to_new).
    pub fn create(
        properties: &DecimalFormatProperties,
        symbols: Arc<DecimalFormatSymbols>,
    ) -> Result<UnlocalizedNumberFormatter> {
        let macros = Self::old_to_new(properties, symbols, None)?;
        Ok(NumberFormatter::with().macros(macros))
    }

    /// Convenience method to create a NumberFormatter directly from
    /// Properties. Equivalent to `create(DecimalFormatProperties,
    /// DecimalFormatSymbols, DecimalFormatProperties)`.
    ///
    /// # Errors
    ///
    /// The errors of [`old_to_new`](Self::old_to_new).
    pub fn create_exported(
        properties: &DecimalFormatProperties,
        symbols: Arc<DecimalFormatSymbols>,
        exported_properties: Option<&mut DecimalFormatProperties>,
    ) -> Result<UnlocalizedNumberFormatter> {
        let macros = Self::old_to_new(properties, symbols, exported_properties)?;
        Ok(NumberFormatter::with().macros(macros))
    }

    /// Convenience method to create a NumberFormatter directly from a
    /// pattern string. Something like this could become public API if there
    /// is demand. Equivalent to `create(String, DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// The errors of the pattern parsing and of the mapping.
    pub fn create_from_pattern(
        pattern: &[u16],
        symbols: Arc<DecimalFormatSymbols>,
    ) -> Result<UnlocalizedNumberFormatter> {
        let properties = PatternStringParser::parse_to_properties(Some(pattern))?;
        Self::create(&properties, symbols)
    }

    /// Creates a new {@link MacroProps} object based on the content of a
    /// {@link DecimalFormatProperties} object. In other words, maps
    /// Properties to MacroProps. This function is used by the JDK-compatibility
    /// API to call into the ICU 60 fluent number formatting pipeline.
    /// Equivalent to `oldToNew(DecimalFormatProperties, DecimalFormatSymbols,
    /// DecimalFormatProperties)`.
    ///
    /// # Errors
    ///
    /// The errors of the currency resolution, of the affix provider and of
    /// the settings validation.
    pub fn old_to_new(
        properties: &DecimalFormatProperties,
        symbols: Arc<DecimalFormatSymbols>,
        exported_properties: Option<&mut DecimalFormatProperties>,
    ) -> Result<MacroProps> {
        let mut macros = MacroProps::default();
        let locale = symbols.get_u_locale();

        /////////////
        // SYMBOLS //
        /////////////

        macros.symbols = Some(SymbolsObject::Symbols(symbols.clone()));

        //////////////////
        // PLURAL RULES //
        //////////////////

        let mut rules = properties.get_plural_rules();
        if rules.is_none() {
            if let Some(cpi) = properties.get_currency_plural_info() {
                rules = cpi.get_plural_rules();
            }
        }
        macros.rules = rules;

        /////////////
        // AFFIXES //
        /////////////

        let affix_provider: Arc<dyn AffixPatternProvider> =
            PropertiesAffixPatternProvider::for_properties(properties)?;
        macros.affix_provider = Some(affix_provider.clone());

        ///////////
        // UNITS //
        ///////////

        let use_currency = properties.get_currency().is_some()
            || properties.get_currency_plural_info().is_some()
            || properties.get_currency_usage().is_some()
            || affix_provider.has_currency_sign();
        let currency = CustomSymbolCurrency::resolve(properties.get_currency(), &locale, &symbols)?;
        let mut currency_usage = properties.get_currency_usage();
        let explicit_currency_usage = currency_usage.is_some();
        if !explicit_currency_usage {
            currency_usage = Some(CurrencyUsage::Standard);
        }
        let currency_usage = currency_usage.expect("set above");
        if use_currency {
            macros.unit = Some(currency.as_measure_unit().clone());
        }

        ///////////////////////
        // ROUNDING STRATEGY //
        ///////////////////////

        let mut max_int = properties.get_maximum_integer_digits();
        let mut min_int = properties.get_minimum_integer_digits();
        let mut max_frac = properties.get_maximum_fraction_digits();
        let mut min_frac = properties.get_minimum_fraction_digits();
        let mut min_sig = properties.get_minimum_significant_digits();
        let mut max_sig = properties.get_maximum_significant_digits();
        let rounding_increment = properties.get_rounding_increment();
        let math_context = RoundingUtils::get_math_context_or_unlimited(properties);
        // Validate min/max int/frac.
        // For backwards compatibility, minimum overrides maximum if the two
        // conflict.
        let explicit_min_max_frac = min_frac != -1 || max_frac != -1;
        let explicit_min_max_sig = min_sig != -1 || max_sig != -1;
        // Resolve min/max frac for currencies, required for the validation
        // logic and for when minFrac or maxFrac was set (but not both) on a
        // currency instance.
        // NOTE: Increments are handled in "Precision.constructCurrency()".
        if use_currency {
            if min_frac == -1 && max_frac == -1 {
                min_frac = currency.get_default_fraction_digits_usage(currency_usage)?;
                max_frac = currency.get_default_fraction_digits_usage(currency_usage)?;
            } else if min_frac == -1 {
                min_frac =
                    max_frac.min(currency.get_default_fraction_digits_usage(currency_usage)?);
            } else if max_frac == -1 {
                max_frac =
                    min_frac.max(currency.get_default_fraction_digits_usage(currency_usage)?);
            } else {
                // No-op: user override for both minFrac and maxFrac
            }
        }
        // Validate min/max int/frac.
        // For backwards compatibility, minimum overrides maximum if the two
        // conflict.
        if min_int == 0 && max_frac != 0 {
            min_frac = if min_frac < 0 || (min_frac == 0 && max_int == 0) {
                1
            } else {
                min_frac
            };
            max_frac = if max_frac < 0 {
                -1
            } else if max_frac < min_frac {
                min_frac
            } else {
                max_frac
            };
            min_int = 0;
            max_int = if !(0..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&max_int) {
                -1
            } else {
                max_int
            };
        } else {
            // Force a digit before the decimal point.
            min_frac = if min_frac < 0 { 0 } else { min_frac };
            max_frac = if max_frac < 0 {
                -1
            } else if max_frac < min_frac {
                min_frac
            } else {
                max_frac
            };
            min_int = if min_int <= 0 || min_int > RoundingUtils::MAX_INT_FRAC_SIG {
                1
            } else {
                min_int
            };
            max_int = if max_int < 0 {
                -1
            } else if max_int < min_int {
                min_int
            } else if max_int > RoundingUtils::MAX_INT_FRAC_SIG {
                -1
            } else {
                max_int
            };
        }
        let mut rounding: Option<Precision> = None;
        if explicit_currency_usage {
            rounding =
                Some(Precision::construct_currency(currency_usage).with_currency(Some(&currency))?);
        } else if let Some(inc) = rounding_increment.clone() {
            if PatternStringUtils::ignore_rounding_increment(&inc, max_frac) {
                rounding = Some(Precision::construct_fraction(min_frac, max_frac).into_precision());
            } else {
                let mut inc = inc;
                if min_frac > inc.scale() {
                    inc = inc.set_scale_exact(min_frac)?;
                }
                rounding = Some(Precision::construct_increment(&inc)?);
            }
        } else if explicit_min_max_sig {
            min_sig = min_sig.clamp(1, RoundingUtils::MAX_INT_FRAC_SIG);
            max_sig = if max_sig < 0 {
                RoundingUtils::MAX_INT_FRAC_SIG
            } else if max_sig < min_sig {
                min_sig
            } else if max_sig > RoundingUtils::MAX_INT_FRAC_SIG {
                RoundingUtils::MAX_INT_FRAC_SIG
            } else {
                max_sig
            };
            rounding = Some(Precision::construct_significant(min_sig, max_sig));
        } else if explicit_min_max_frac {
            rounding = Some(Precision::construct_fraction(min_frac, max_frac).into_precision());
        } else if use_currency {
            rounding = Some(Precision::construct_currency(currency_usage).into_precision());
        }
        if let Some(r) = rounding.take() {
            let r = r.with_mode(&math_context);
            macros.precision = Some(r.clone());
            rounding = Some(r);
        }

        ///////////////////
        // INTEGER WIDTH //
        ///////////////////

        macros.integer_width = Some(IntegerWidth::zero_fill_to(min_int)?.truncate_at(max_int)?);

        ///////////////////////
        // GROUPING STRATEGY //
        ///////////////////////

        macros.grouping = Some(GroupingObject::Grouper(Grouper::for_properties(properties)));

        /////////////
        // PADDING //
        /////////////

        if properties.get_format_width() > 0 {
            macros.padder = Some(Padder::for_properties(properties));
        }

        ///////////////////////////////
        // DECIMAL MARK ALWAYS SHOWN //
        ///////////////////////////////

        macros.decimal = Some(if properties.get_decimal_separator_always_shown() {
            DecimalSeparatorDisplay::Always
        } else {
            DecimalSeparatorDisplay::Auto
        });

        ///////////////////////
        // SIGN ALWAYS SHOWN //
        ///////////////////////

        macros.sign = Some(if properties.get_sign_always_shown() {
            SignDisplay::Always
        } else {
            SignDisplay::Auto
        });

        /////////////////////////
        // SCIENTIFIC NOTATION //
        /////////////////////////

        if properties.get_minimum_exponent_digits() != -1 {
            // Scientific notation is required.
            // This whole section feels like a hack, but it is needed for
            // regression tests. The mapping from property bag to scientific
            // notation is nontrivial due to LDML rules.
            if max_int > 8 {
                // But #13110: The maximum of 8 digits has unknown origins
                // and is not in the spec. If maxInt is greater than 8, it is
                // set to minInt, even if minInt is greater than 8.
                max_int = min_int;
                macros.integer_width =
                    Some(IntegerWidth::zero_fill_to(min_int)?.truncate_at(max_int)?);
            } else if max_int > min_int && min_int > 1 {
                // Bug #13289: if maxInt > minInt > 1, then minInt should be
                // 1.
                min_int = 1;
                macros.integer_width =
                    Some(IntegerWidth::zero_fill_to(min_int)?.truncate_at(max_int)?);
            }
            let engineering = if max_int < 0 { -1 } else { max_int };
            macros.notation = Some(
                ScientificNotation::new(
                    // Engineering interval:
                    engineering,
                    // Enforce minimum integer digits (for patterns like
                    // "000.00E0"):
                    engineering == min_int,
                    // Minimum exponent digits:
                    properties.get_minimum_exponent_digits(),
                    // Exponent sign always shown:
                    if properties.get_exponent_sign_always_shown() {
                        SignDisplay::Always
                    } else {
                        SignDisplay::Auto
                    },
                )
                .into_notation(),
            );
            // Scientific notation also involves overriding the rounding
            // mode.
            // TODO: Overriding here is a bit of a hack. Should this logic go
            // earlier?
            if macros
                .precision
                .as_ref()
                .is_some_and(|p| FractionPrecision::try_cast(p).is_some())
            {
                // For the purposes of rounding, get the original min/max int
                // and min/max frac.
                let max_int_ = properties.get_maximum_integer_digits();
                let mut min_int_ = properties.get_minimum_integer_digits();
                let min_frac_ = properties.get_minimum_fraction_digits();
                let max_frac_ = properties.get_maximum_fraction_digits();
                if min_int_ == 0 && max_frac_ == 0 {
                    // Patterns like "#E0" and "##E0", which mean no rounding!
                    macros.precision =
                        Some(Precision::construct_infinite().with_mode(&math_context));
                } else if min_int_ == 0 && min_frac_ == 0 {
                    // Patterns like "#.##E0" (no zeros in the mantissa),
                    // which mean round to maxFrac+1
                    macros.precision = Some(
                        Precision::construct_significant(1, max_frac_ + 1).with_mode(&math_context),
                    );
                } else {
                    let max_sig_ = min_int_ + max_frac_;
                    // Bug #20058: if maxInt > minInt > 1, then minInt should
                    // be 1.
                    if max_int_ > min_int_ && min_int_ > 1 {
                        min_int_ = 1;
                    }
                    let min_sig_ = min_int_ + min_frac_;
                    // To avoid regression, maxSig is not reset when minInt_
                    // set to 1.
                    // TODO: Reset maxSig_ = 1 + minFrac_ to follow the spec.
                    macros.precision = Some(
                        Precision::construct_significant(min_sig_, max_sig_)
                            .with_mode(&math_context),
                    );
                }
            }
        }

        //////////////////////
        // COMPACT NOTATION //
        //////////////////////

        if let Some(compact_style) = properties.get_compact_style() {
            if let Some(custom) = properties.get_compact_custom_data() {
                macros.notation =
                    Some(CompactNotation::with_custom_data(Some(custom)).into_notation());
            } else if compact_style == CompactStyle::Long {
                macros.notation = Some(Notation::compact_long().into_notation());
            } else {
                macros.notation = Some(Notation::compact_short().into_notation());
            }
        }

        /////////////////
        // MULTIPLIERS //
        /////////////////

        macros.scale = RoundingUtils::scale_from_properties(properties)?;

        //////////////////////
        // PROPERTY EXPORTS //
        //////////////////////

        if let Some(exported_properties) = exported_properties {
            exported_properties.set_currency(Some(currency.clone()));
            exported_properties.set_math_context(Some(math_context));
            exported_properties.set_rounding_mode(Some(math_context.get_rounding_mode()));
            exported_properties.set_minimum_integer_digits(min_int);
            exported_properties.set_maximum_integer_digits(if max_int == -1 {
                i32::MAX
            } else {
                max_int
            });

            let rounding_ = match rounding.as_ref().and_then(CurrencyPrecision::try_cast) {
                Some(cp) => Some(cp.with_currency(Some(&currency))?),
                None => rounding.clone(),
            };
            let mut min_frac_ = min_frac;
            let mut max_frac_ = max_frac;
            let mut min_sig_ = min_sig;
            let mut max_sig_ = max_sig;
            let mut increment_ = None;
            match rounding_.as_ref().map(Precision::kind) {
                Some(PrecisionKind::Fraction { min_frac, max_frac }) => {
                    min_frac_ = *min_frac;
                    max_frac_ = *max_frac;
                }
                Some(PrecisionKind::Increment { increment })
                | Some(PrecisionKind::IncrementOne { increment, .. })
                | Some(PrecisionKind::IncrementFive { increment, .. }) => {
                    min_frac_ = increment.scale();
                    max_frac_ = increment.scale();
                    increment_ = Some(increment.clone());
                }
                Some(PrecisionKind::Significant { min_sig, max_sig }) => {
                    min_sig_ = *min_sig;
                    max_sig_ = *max_sig;
                }
                _ => {}
            }

            exported_properties.set_minimum_fraction_digits(min_frac_);
            exported_properties.set_maximum_fraction_digits(max_frac_);
            exported_properties.set_minimum_significant_digits(min_sig_);
            exported_properties.set_maximum_significant_digits(max_sig_);
            exported_properties.set_rounding_increment(increment_);
        }

        Ok(macros)
    }
}
