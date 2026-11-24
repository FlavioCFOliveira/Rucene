// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.NumberParserImpl`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::{char_count, ParsePosition};
use crate::number::number_formatter::GroupingStrategy;
use crate::r#impl::number::affix_utils::{TYPE_PERCENT, TYPE_PERMILLE};
use crate::r#impl::number::custom_symbol_currency::CustomSymbolCurrency;
use crate::r#impl::number::decimal_format_properties::{DecimalFormatProperties, ParseMode};
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::r#impl::number::grouper::Grouper;
use crate::r#impl::number::parse::affix_matcher::AffixMatcher;
use crate::r#impl::number::parse::affix_token_matcher_factory::AffixTokenMatcherFactory;
use crate::r#impl::number::parse::combined_currency_matcher::CombinedCurrencyMatcher;
use crate::r#impl::number::parse::decimal_matcher::DecimalMatcher;
use crate::r#impl::number::parse::ignorables_matcher::IgnorablesMatcher;
use crate::r#impl::number::parse::infinity_matcher::InfinityMatcher;
use crate::r#impl::number::parse::minus_sign_matcher::MinusSignMatcher;
use crate::r#impl::number::parse::multiplier_parse_handler::MultiplierParseHandler;
use crate::r#impl::number::parse::nan_matcher::NanMatcher;
use crate::r#impl::number::parse::number_parse_matcher::{matchers_to_string, NumberParseMatcher};
use crate::r#impl::number::parse::padding_matcher::PaddingMatcher;
use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::number::parse::percent_matcher::PercentMatcher;
use crate::r#impl::number::parse::permille_matcher::PermilleMatcher;
use crate::r#impl::number::parse::plus_sign_matcher::PlusSignMatcher;
use crate::r#impl::number::parse::require_affix_validator::RequireAffixValidator;
use crate::r#impl::number::parse::require_currency_validator::RequireCurrencyValidator;
use crate::r#impl::number::parse::require_decimal_separator_validator::RequireDecimalSeparatorValidator;
use crate::r#impl::number::parse::require_number_validator::RequireNumberValidator;
use crate::r#impl::number::parse::scientific_matcher::ScientificMatcher;
use crate::r#impl::number::pattern_string_parser::PatternStringParser;
use crate::r#impl::number::properties_affix_pattern_provider::PropertiesAffixPatternProvider;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::util::currency::Currency;
use crate::util::currency_amount::CurrencyAmount;
use crate::util::u_locale::ULocale;

/// Primary number parsing implementation class.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.NumberParserImpl`.
#[derive(Debug)]
pub struct NumberParserImpl {
    parse_flags: i32,
    matchers: Vec<Arc<dyn NumberParseMatcher>>,
    frozen: bool,
}

impl NumberParserImpl {
    /// Creates a parser with most default options. Used for testing, not
    /// production. Equivalent to `createSimpleParser(ULocale, String, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the pattern parse, the locale data and the matchers.
    pub fn create_simple_parser(
        locale: &ULocale,
        pattern: &[u16],
        parse_flags: i32,
    ) -> Result<NumberParserImpl> {
        let mut parser = NumberParserImpl::new(parse_flags);
        let currency = Currency::get_instance(Some("USD"))?;
        let symbols = Arc::new(DecimalFormatSymbols::get_instance_ulocale(locale)?);
        let ignorables = IgnorablesMatcher::get_instance(parse_flags);

        let factory = AffixTokenMatcherFactory {
            currency: currency.clone(),
            symbols: symbols.clone(),
            ignorables: ignorables.clone(),
            locale: locale.clone(),
            parse_flags,
        };

        let pattern_info = PatternStringParser::parse_to_pattern_info(pattern)?;
        AffixMatcher::create_matchers(
            &pattern_info,
            &mut parser,
            &factory,
            &ignorables,
            parse_flags,
        )?;

        let grouper = Grouper::for_strategy(GroupingStrategy::Auto)
            .with_locale_data(locale, &pattern_info)?;

        parser.add_matcher(ignorables.clone());
        parser.add_matcher(DecimalMatcher::get_instance(
            &symbols,
            &grouper,
            parse_flags,
        )?);
        parser.add_matcher(MinusSignMatcher::get_instance(&symbols, false));
        parser.add_matcher(PlusSignMatcher::get_instance(&symbols, false));
        parser.add_matcher(PercentMatcher::get_instance(&symbols));
        parser.add_matcher(PermilleMatcher::get_instance(&symbols));
        parser.add_matcher(NanMatcher::get_instance(&symbols, parse_flags));
        parser.add_matcher(InfinityMatcher::get_instance(&symbols));
        parser.add_matcher(PaddingMatcher::get_instance(&[u16::from(b'@')]));
        parser.add_matcher(ScientificMatcher::get_instance(&symbols, &grouper)?);
        parser.add_matcher(CombinedCurrencyMatcher::get_instance(
            &currency,
            &symbols,
            parse_flags,
        )?);
        parser.add_matcher(Arc::new(RequireNumberValidator));

        parser.freeze();
        Ok(parser)
    }

    /// Parses the string without returning a NumberParserImpl. Used for
    /// backwards compatibility. Equivalent to `parseStatic(String,
    /// ParsePosition, DecimalFormatProperties, DecimalFormatSymbols)`;
    /// `None` for Java's `null` (parse failure).
    ///
    /// # Errors
    ///
    /// The errors of the parser construction and of the parse.
    pub fn parse_static(
        input: &[u16],
        ppos: &mut ParsePosition,
        properties: &DecimalFormatProperties,
        symbols: &Arc<DecimalFormatSymbols>,
    ) -> Result<Option<JavaNumber>> {
        let parser = Self::create_parser_from_properties(properties, symbols, false)?;
        let mut result = ParsedNumber::new();
        parser.parse(input, true, &mut result)?;
        if result.success() {
            ppos.set_index(result.char_end);
            Ok(Some(result.get_number()?))
        } else {
            ppos.set_error_index(result.char_end);
            Ok(None)
        }
    }

    /// Equivalent to `parseStaticCurrency(String, ParsePosition,
    /// DecimalFormatProperties, DecimalFormatSymbols)`; `None` for Java's
    /// `null` (parse failure).
    ///
    /// # Errors
    ///
    /// The errors of the parser construction, of the parse and of
    /// `Currency.getInstance`.
    pub fn parse_static_currency(
        input: &[u16],
        ppos: &mut ParsePosition,
        properties: &DecimalFormatProperties,
        symbols: &Arc<DecimalFormatSymbols>,
    ) -> Result<Option<CurrencyAmount>> {
        let parser = Self::create_parser_from_properties(properties, symbols, true)?;
        let mut result = ParsedNumber::new();
        parser.parse(input, true, &mut result)?;
        if result.success() {
            ppos.set_index(result.char_end);
            debug_assert!(result.currency_code.is_some());
            let number = result.get_number()?;
            let currency = Currency::get_instance(result.currency_code.as_deref())?;
            Ok(Some(CurrencyAmount::new(Some(number), Some(currency))?))
        } else {
            ppos.set_error_index(result.char_end);
            Ok(None)
        }
    }

    /// Equivalent to `createDefaultParserForLocale(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data and of the parser construction.
    pub fn create_default_parser_for_locale(loc: &ULocale) -> Result<NumberParserImpl> {
        let symbols = Arc::new(DecimalFormatSymbols::get_instance_ulocale(loc)?);
        let properties = PatternStringParser::parse_to_properties(Some(&[u16::from(b'0')]))?;
        Self::create_parser_from_properties(&properties, &symbols, false)
    }

    /// Creates a parser from the given DecimalFormatProperties. This
    /// parser utilizes all the properties to configure itself. Equivalent
    /// to `createParserFromProperties(DecimalFormatProperties,
    /// DecimalFormatSymbols, boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the affix provider, the currency resolution and the
    /// matchers.
    pub fn create_parser_from_properties(
        properties: &DecimalFormatProperties,
        symbols: &Arc<DecimalFormatSymbols>,
        parse_currency: bool,
    ) -> Result<NumberParserImpl> {
        let locale = symbols.get_u_locale();
        let affix_provider = PropertiesAffixPatternProvider::for_properties(properties)?;
        let currency = CustomSymbolCurrency::resolve(properties.get_currency(), &locale, symbols)?;
        let parse_mode = properties.get_parse_mode().unwrap_or(ParseMode::Lenient);
        let grouper = Grouper::for_properties(properties);
        let mut parse_flags = 0;
        if !properties.get_parse_case_sensitive() {
            parse_flags |= ParsingUtils::PARSE_FLAG_IGNORE_CASE;
        }
        if properties.get_parse_integer_only() {
            parse_flags |= ParsingUtils::PARSE_FLAG_INTEGER_ONLY;
        }
        if properties.get_parse_to_big_decimal() {
            parse_flags |= ParsingUtils::PARSE_FLAG_FORCE_BIG_DECIMAL;
        }
        if properties.get_sign_always_shown() {
            parse_flags |= ParsingUtils::PARSE_FLAG_PLUS_SIGN_ALLOWED;
        }
        if parse_mode == ParseMode::JavaCompatibility {
            parse_flags |= ParsingUtils::PARSE_FLAG_STRICT_SEPARATORS;
            parse_flags |= ParsingUtils::PARSE_FLAG_USE_FULL_AFFIXES;
            parse_flags |= ParsingUtils::PARSE_FLAG_EXACT_AFFIX;
            parse_flags |= ParsingUtils::PARSE_FLAG_JAVA_COMPATIBILITY_IGNORABLES;
        } else if parse_mode == ParseMode::Strict {
            parse_flags |= ParsingUtils::PARSE_FLAG_STRICT_GROUPING_SIZE;
            parse_flags |= ParsingUtils::PARSE_FLAG_STRICT_SEPARATORS;
            parse_flags |= ParsingUtils::PARSE_FLAG_USE_FULL_AFFIXES;
            parse_flags |= ParsingUtils::PARSE_FLAG_EXACT_AFFIX;
            parse_flags |= ParsingUtils::PARSE_FLAG_STRICT_IGNORABLES;
        } else {
            parse_flags |= ParsingUtils::PARSE_FLAG_INCLUDE_UNPAIRED_AFFIXES;
        }
        if grouper.get_primary() <= 0 {
            parse_flags |= ParsingUtils::PARSE_FLAG_GROUPING_DISABLED;
        }
        if parse_currency || affix_provider.has_currency_sign() {
            parse_flags |= ParsingUtils::PARSE_FLAG_MONETARY_SEPARATORS;
        }
        if !parse_currency {
            parse_flags |= ParsingUtils::PARSE_FLAG_NO_FOREIGN_CURRENCIES;
        }

        let mut parser = NumberParserImpl::new(parse_flags);

        let ignorables = IgnorablesMatcher::get_instance(parse_flags);

        let factory = AffixTokenMatcherFactory {
            currency: currency.clone(),
            symbols: symbols.clone(),
            ignorables: ignorables.clone(),
            locale: locale.clone(),
            parse_flags,
        };

        // ----
        // AFFIX MATCHERS
        // ----

        // Set up a pattern modifier with mostly defaults to generate AffixMatchers.
        AffixMatcher::create_matchers(
            affix_provider.as_ref(),
            &mut parser,
            &factory,
            &ignorables,
            parse_flags,
        )?;

        // ----
        // CURRENCY MATCHER
        // ----

        if parse_currency || affix_provider.has_currency_sign() {
            parser.add_matcher(CombinedCurrencyMatcher::get_instance(
                &currency,
                symbols,
                parse_flags,
            )?);
        }

        ///////////////
        // PERCENT
        ///////////////

        // ICU-TC meeting, April 11, 2018: accept percent/permille only if it is in the pattern,
        // and to maintain regressive behavior, divide by 100 even if no percent sign is present.
        if parse_mode == ParseMode::Lenient && affix_provider.contains_symbol_type(TYPE_PERCENT)? {
            parser.add_matcher(PercentMatcher::get_instance(symbols));
        }
        if parse_mode == ParseMode::Lenient && affix_provider.contains_symbol_type(TYPE_PERMILLE)? {
            parser.add_matcher(PermilleMatcher::get_instance(symbols));
        }

        // ----
        // OTHER STANDARD MATCHERS
        // ----

        if parse_mode == ParseMode::Lenient {
            parser.add_matcher(PlusSignMatcher::get_instance(symbols, false));
            parser.add_matcher(MinusSignMatcher::get_instance(symbols, false));
        }
        parser.add_matcher(NanMatcher::get_instance(symbols, parse_flags));
        parser.add_matcher(InfinityMatcher::get_instance(symbols));
        if let Some(pad_string) = properties.get_pad_string() {
            if !ignorables.get_set().contains_str(pad_string) {
                parser.add_matcher(PaddingMatcher::get_instance(pad_string));
            }
        }
        parser.add_matcher(ignorables.clone());
        parser.add_matcher(DecimalMatcher::get_instance(
            symbols,
            &grouper,
            parse_flags,
        )?);
        // NOTE: parseNoExponent doesn't disable scientific parsing if we have a scientific formatter
        if !properties.get_parse_no_exponent() || properties.get_minimum_exponent_digits() > 0 {
            parser.add_matcher(ScientificMatcher::get_instance(symbols, &grouper)?);
        }

        // ----
        // VALIDATORS
        // ----

        parser.add_matcher(Arc::new(RequireNumberValidator));
        if parse_mode != ParseMode::Lenient {
            parser.add_matcher(Arc::new(RequireAffixValidator));
        }
        if parse_currency {
            parser.add_matcher(Arc::new(RequireCurrencyValidator));
        }
        if properties.get_decimal_pattern_match_required() {
            let pattern_has_decimal_separator = properties.get_decimal_separator_always_shown()
                || properties.get_maximum_fraction_digits() != 0;
            parser.add_matcher(RequireDecimalSeparatorValidator::get_instance(
                pattern_has_decimal_separator,
            ));
        }
        // The multiplier takes care of scaling percentages.
        if let Some(multiplier) = RoundingUtils::scale_from_properties(properties)? {
            parser.add_matcher(Arc::new(MultiplierParseHandler::new(multiplier)));
        }

        parser.freeze();
        Ok(parser)
    }

    /// Creates a new, empty parser. Equivalent to
    /// `NumberParserImpl(int)`.
    pub fn new(parse_flags: i32) -> Self {
        NumberParserImpl {
            parse_flags,
            matchers: Vec::new(),
            frozen: false,
        }
    }

    /// Equivalent to `addMatcher(NumberParseMatcher)`.
    pub fn add_matcher(&mut self, matcher: Arc<dyn NumberParseMatcher>) {
        debug_assert!(!self.frozen);
        self.matchers.push(matcher);
    }

    /// Equivalent to `addMatchers(Collection)`.
    pub fn add_matchers(
        &mut self,
        matchers: impl IntoIterator<Item = Arc<dyn NumberParseMatcher>>,
    ) {
        debug_assert!(!self.frozen);
        self.matchers.extend(matchers);
    }

    /// Equivalent to `freeze()`.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Equivalent to `getParseFlags()`.
    pub fn get_parse_flags(&self) -> i32 {
        self.parse_flags
    }

    /// Equivalent to `parse(String, boolean, ParsedNumber)`.
    ///
    /// # Errors
    ///
    /// The errors of the matchers.
    pub fn parse(&self, input: &[u16], greedy: bool, result: &mut ParsedNumber) -> Result<()> {
        self.parse_from(input, 0, greedy, result)
    }

    /// Primary entrypoint to parsing code path. Equivalent to
    /// `parse(String, int, boolean, ParsedNumber)`.
    ///
    /// # Errors
    ///
    /// The errors of the matchers.
    pub fn parse_from(
        &self,
        input: &[u16],
        start: i32,
        greedy: bool,
        result: &mut ParsedNumber,
    ) -> Result<()> {
        debug_assert!(self.frozen);
        debug_assert!(start >= 0 && start < input.len() as i32);
        let mut segment = StringSegment::new(
            input,
            0 != (self.parse_flags & ParsingUtils::PARSE_FLAG_IGNORE_CASE),
        );
        segment.adjust_offset(start);
        if greedy {
            self.parse_greedy(&mut segment, result)?;
        } else if 0 != (self.parse_flags & ParsingUtils::PARSE_FLAG_ALLOW_INFINITE_RECURSION) {
            // Start at 1 so that recursionLevels never gets to 0
            self.parse_longest_recursive(&mut segment, result, 1)?;
        } else {
            // Arbitrary recursion safety limit: 100 levels.
            self.parse_longest_recursive(&mut segment, result, -100)?;
        }
        for matcher in &self.matchers {
            matcher.post_process(result)?;
        }
        result.post_process();
        Ok(())
    }

    fn parse_greedy(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<()> {
        // Note: this method gets called a lot, so it is not using the for-each loop.
        let mut i = 0;
        while i < self.matchers.len() {
            // Base Case
            if segment.length() == 0 {
                return Ok(());
            }
            let matcher = &self.matchers[i];
            if !matcher.smoke_test(segment) {
                // Matcher failed smoke test: try the next one
                i += 1;
                continue;
            }
            let initial_offset = segment.get_offset();
            matcher.match_(segment, result)?;
            if segment.get_offset() != initial_offset {
                // Greedy heuristic: accept the match and loop back
                i = 0;
            } else {
                // Matcher did not match: try the next one
                i += 1;
            }
            // NOTE: If we get here, the greedy parse completed without consuming the entire string.
        }
        Ok(())
    }

    fn parse_longest_recursive(
        &self,
        segment: &mut StringSegment,
        result: &mut ParsedNumber,
        recursion_levels: i32,
    ) -> Result<()> {
        // Base Case
        if segment.length() == 0 {
            return Ok(());
        }

        // Safety against stack overflow
        if recursion_levels == 0 {
            return Ok(());
        }

        // TODO: Give a nice way for the matcher to reset the ParsedNumber?
        let mut initial = ParsedNumber::new();
        initial.copy_from(result);
        let mut candidate = ParsedNumber::new();

        let initial_offset = segment.get_offset();
        for matcher in &self.matchers {
            // In a non-greedy parse, we attempt all possible matches and pick the best.
            if !matcher.smoke_test(segment) {
                continue;
            }

            // Try matching a number of chars
            let mut chars_to_consume = 0;
            while chars_to_consume < segment.length() {
                chars_to_consume += char_count(segment.code_point_at(chars_to_consume));

                // Run the matcher on a segment of the current length.
                candidate.copy_from(&initial);
                segment.set_length(chars_to_consume);
                let maybe_more = matcher.match_(segment, &mut candidate);
                segment.reset_length();
                let maybe_more = maybe_more?;

                // If the entire segment was consumed, recurse.
                if segment.get_offset() - initial_offset == chars_to_consume {
                    self.parse_longest_recursive(segment, &mut candidate, recursion_levels + 1)?;
                    if candidate.is_better_than(result) {
                        result.copy_from(&candidate);
                    }
                }

                // Since the segment can be re-used, reset the offset.
                // This does not have an effect if the matcher did not consume any chars.
                segment.set_offset(initial_offset);

                // Unless the matcher wants to see the next char, continue to the next matcher.
                if !maybe_more {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        format!(
            "<NumberParserImpl matchers={}>",
            matchers_to_string(&self.matchers)
        )
    }
}
