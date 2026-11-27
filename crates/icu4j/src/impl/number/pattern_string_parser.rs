// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.PatternStringParser`.

use rucene::error::{LuceneError, Result};

use crate::jdk::{char_count, code_point_at};
use crate::r#impl::number::affix_pattern_provider::{AffixPatternProvider, Flags};
use crate::r#impl::number::affix_utils::AffixUtils;
use crate::r#impl::number::decimal_format_properties::{
    empty_literal, new_jstring, DecimalFormatProperties, JString,
};
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_abstract_bcd::DecimalQuantityAbstractBcd;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::DecimalQuantity_DualStorageBCD;
use crate::r#impl::number::padder::PadPosition;

/// Implements a recursive descent parser for decimal format patterns.
///
/// Equivalent to `com.ibm.icu.impl.number.PatternStringParser` (static
/// members).
pub struct PatternStringParser;

impl PatternStringParser {
    /// `IGNORE_ROUNDING_NEVER`.
    pub const IGNORE_ROUNDING_NEVER: i32 = 0;
    /// `IGNORE_ROUNDING_IF_CURRENCY`.
    pub const IGNORE_ROUNDING_IF_CURRENCY: i32 = 1;
    /// `IGNORE_ROUNDING_ALWAYS`.
    pub const IGNORE_ROUNDING_ALWAYS: i32 = 2;

    /// Runs the recursive descent parser on the given pattern string,
    /// returning a data structure with raw information about the pattern
    /// string. Equivalent to `parseToPatternInfo(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern.
    pub fn parse_to_pattern_info(pattern_string: &[u16]) -> Result<ParsedPatternInfo> {
        let mut state = ParserState::new(pattern_string);
        let mut result = ParsedPatternInfo::new(pattern_string);
        consume_pattern(&mut state, &mut result)?;
        Ok(result)
    }

    /// Parses a pattern string into a new property bag. Equivalent to
    /// `parseToProperties(String, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern.
    pub fn parse_to_properties_ignore(
        pattern: Option<&[u16]>,
        ignore_rounding: i32,
    ) -> Result<DecimalFormatProperties> {
        let mut properties = DecimalFormatProperties::new();
        parse_to_existing_properties_impl(pattern, &mut properties, ignore_rounding)?;
        Ok(properties)
    }

    /// Parses a pattern string into a new property bag. Equivalent to
    /// `parseToProperties(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern.
    pub fn parse_to_properties(pattern: Option<&[u16]>) -> Result<DecimalFormatProperties> {
        Self::parse_to_properties_ignore(pattern, Self::IGNORE_ROUNDING_NEVER)
    }

    /// Parses a pattern string into an existing property bag. All properties
    /// that can be encoded into a pattern string will be overwritten with
    /// either their default value or with the value coming from the pattern
    /// string. Properties that cannot be encoded into a pattern string, such
    /// as rounding mode, are not modified. Equivalent to
    /// `parseToExistingProperties(String, DecimalFormatProperties, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern.
    pub fn parse_to_existing_properties_ignore(
        pattern: Option<&[u16]>,
        properties: &mut DecimalFormatProperties,
        ignore_rounding: i32,
    ) -> Result<()> {
        parse_to_existing_properties_impl(pattern, properties, ignore_rounding)
    }

    /// Equivalent to `parseToExistingProperties(String,
    /// DecimalFormatProperties)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern.
    pub fn parse_to_existing_properties(
        pattern: Option<&[u16]>,
        properties: &mut DecimalFormatProperties,
    ) -> Result<()> {
        Self::parse_to_existing_properties_ignore(pattern, properties, Self::IGNORE_ROUNDING_NEVER)
    }
}

/// Contains raw information about the parsed decimal format pattern string.
///
/// Equivalent to `PatternStringParser.ParsedPatternInfo`. `negative` is
/// `None` for Java's `null`; `positive`, `null` only before the parse
/// finished, is always set.
#[derive(Debug, Clone)]
pub struct ParsedPatternInfo {
    /// `pattern`.
    pub pattern: Vec<u16>,
    /// `positive`.
    pub positive: ParsedSubpatternInfo,
    /// `negative`.
    pub negative: Option<ParsedSubpatternInfo>,
}

impl ParsedPatternInfo {
    fn new(pattern: &[u16]) -> Self {
        ParsedPatternInfo {
            pattern: pattern.to_vec(),
            positive: ParsedSubpatternInfo::default(),
            negative: None,
        }
    }

    /// Equivalent to `getLengthFromEndpoints(long)`.
    pub fn get_length_from_endpoints(endpoints: i64) -> i32 {
        let left = (endpoints & 0xffffffff) as i32;
        let right = ((endpoints as u64) >> 32) as i32;
        right - left
    }

    /// `getString(int)` as a Java reference: the `""` literal when empty,
    /// else a new `String` (`substring`).
    pub fn get_jstring(&self, flags: i32) -> JString {
        let endpoints = self.get_endpoints(flags);
        let left = (endpoints & 0xffffffff) as i32;
        let right = ((endpoints as u64) >> 32) as i32;
        if left == right {
            return empty_literal();
        }
        new_jstring(&self.pattern[left as usize..right as usize])
    }

    fn get_endpoints(&self, flags: i32) -> i64 {
        let prefix = (flags & Flags::PREFIX) != 0;
        let is_negative = (flags & Flags::NEGATIVE_SUBPATTERN) != 0;
        let padding = (flags & Flags::PADDING) != 0;
        if is_negative && padding {
            self.negative().padding_endpoints
        } else if padding {
            self.positive.padding_endpoints
        } else if prefix && is_negative {
            self.negative().prefix_endpoints
        } else if prefix {
            self.positive.prefix_endpoints
        } else if is_negative {
            self.negative().suffix_endpoints
        } else {
            self.positive.suffix_endpoints
        }
    }

    /// `negative`, dereferenced as Java does without a check.
    fn negative(&self) -> &ParsedSubpatternInfo {
        // Java dereferences `negative` here; callers ask only after
        // `hasNegativeSubpattern()`.
        self.negative
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }
}

impl AffixPatternProvider for ParsedPatternInfo {
    /// Equivalent to `charAt(int, int)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` outside the affix.
    fn char_at(&self, flags: i32, index: i32) -> u16 {
        let endpoints = self.get_endpoints(flags);
        let left = (endpoints & 0xffffffff) as i32;
        let right = ((endpoints as u64) >> 32) as i32;
        if index < 0 || index >= right - left {
            crate::jdk::index_out_of_bounds(None);
        }
        self.pattern[(left + index) as usize]
    }

    fn length(&self, flags: i32) -> i32 {
        Self::get_length_from_endpoints(self.get_endpoints(flags))
    }

    fn get_string(&self, flags: i32) -> Vec<u16> {
        self.get_jstring(flags).to_vec()
    }

    fn positive_has_plus_sign(&self) -> Result<bool> {
        Ok(self.positive.has_plus_sign)
    }

    fn has_negative_subpattern(&self) -> bool {
        self.negative.is_some()
    }

    fn negative_has_minus_sign(&self) -> Result<bool> {
        Ok(self.negative().has_minus_sign)
    }

    fn has_currency_sign(&self) -> bool {
        self.positive.has_currency_sign
            || self.negative.as_ref().is_some_and(|n| n.has_currency_sign)
    }

    fn contains_symbol_type(&self, type_: i32) -> Result<bool> {
        AffixUtils::contains_type(Some(&self.pattern), type_)
    }

    fn has_body(&self) -> bool {
        self.positive.integer_total > 0
    }

    fn currency_as_decimal(&self) -> bool {
        self.positive.has_currency_decimal
    }
}

/// Equivalent to `PatternStringParser.ParsedSubpatternInfo`.
#[derive(Debug, Clone)]
pub struct ParsedSubpatternInfo {
    /// `groupingSizes`.
    pub grouping_sizes: i64,
    /// `integerLeadingHashSigns`.
    pub integer_leading_hash_signs: i32,
    /// `integerTrailingHashSigns`.
    pub integer_trailing_hash_signs: i32,
    /// `integerNumerals`.
    pub integer_numerals: i32,
    /// `integerAtSigns`.
    pub integer_at_signs: i32,
    /// `integerTotal`: for convenience.
    pub integer_total: i32,
    /// `fractionNumerals`.
    pub fraction_numerals: i32,
    /// `fractionHashSigns`.
    pub fraction_hash_signs: i32,
    /// `fractionTotal`: for convenience.
    pub fraction_total: i32,
    /// `hasDecimal`.
    pub has_decimal: bool,
    /// `widthExceptAffixes`.
    pub width_except_affixes: i32,
    /// `paddingLocation`.
    pub padding_location: Option<PadPosition>,
    /// `rounding`.
    pub rounding: Option<DecimalQuantity_DualStorageBCD>,
    /// `exponentHasPlusSign`.
    pub exponent_has_plus_sign: bool,
    /// `exponentZeros`.
    pub exponent_zeros: i32,
    /// `hasPercentSign`.
    pub has_percent_sign: bool,
    /// `hasPerMilleSign`.
    pub has_per_mille_sign: bool,
    /// `hasCurrencySign`.
    pub has_currency_sign: bool,
    /// `hasCurrencyDecimal`.
    pub has_currency_decimal: bool,
    /// `hasMinusSign`.
    pub has_minus_sign: bool,
    /// `hasPlusSign`.
    pub has_plus_sign: bool,
    /// `prefixEndpoints`.
    pub prefix_endpoints: i64,
    /// `suffixEndpoints`.
    pub suffix_endpoints: i64,
    /// `paddingEndpoints`.
    pub padding_endpoints: i64,
}

impl Default for ParsedSubpatternInfo {
    fn default() -> Self {
        ParsedSubpatternInfo {
            grouping_sizes: 0x0000ffffffff0000,
            integer_leading_hash_signs: 0,
            integer_trailing_hash_signs: 0,
            integer_numerals: 0,
            integer_at_signs: 0,
            integer_total: 0,
            fraction_numerals: 0,
            fraction_hash_signs: 0,
            fraction_total: 0,
            has_decimal: false,
            width_except_affixes: 0,
            padding_location: None,
            rounding: None,
            exponent_has_plus_sign: false,
            exponent_zeros: 0,
            has_percent_sign: false,
            has_per_mille_sign: false,
            has_currency_sign: false,
            has_currency_decimal: false,
            has_minus_sign: false,
            has_plus_sign: false,
            prefix_endpoints: 0,
            suffix_endpoints: 0,
            padding_endpoints: 0,
        }
    }
}

/// `PatternStringParser.ParserState`.
struct ParserState<'a> {
    pattern: &'a [u16],
    offset: i32,
}

impl<'a> ParserState<'a> {
    fn new(pattern: &'a [u16]) -> Self {
        ParserState { pattern, offset: 0 }
    }

    fn peek(&self) -> i32 {
        if self.offset == self.pattern.len() as i32 {
            -1
        } else {
            code_point_at(self.pattern, self.offset)
        }
    }

    fn peek2(&self) -> i32 {
        if self.offset == self.pattern.len() as i32 {
            return -1;
        }
        let cp1 = code_point_at(self.pattern, self.offset);
        let offset2 = self.offset + char_count(cp1);
        if offset2 == self.pattern.len() as i32 {
            return -1;
        }
        code_point_at(self.pattern, offset2)
    }

    fn next(&mut self) -> i32 {
        let code_point = self.peek();
        self.offset += char_count(code_point);
        code_point
    }

    fn to_parse_exception(&self, message: &str) -> LuceneError {
        let mut sb = String::new();
        sb.push_str("Malformed pattern for ICU DecimalFormat: \"");
        sb.push_str(&String::from_utf16_lossy(self.pattern));
        sb.push_str("\": ");
        sb.push_str(message);
        sb.push_str(" at position ");
        sb.push_str(&self.offset.to_string());
        LuceneError::illegal_argument(sb)
    }
}

fn consume_pattern(state: &mut ParserState<'_>, result: &mut ParsedPatternInfo) -> Result<()> {
    // pattern := subpattern (';' subpattern)?
    result.positive = ParsedSubpatternInfo::default();
    consume_subpattern(state, &mut result.positive)?;
    if state.peek() == i32::from(b';') {
        state.next(); // consume the ';'
                      // Disallow "a;b;" in ICU4J.
        if state.peek() != -1 {
            let mut negative = ParsedSubpatternInfo::default();
            let r = consume_subpattern(state, &mut negative);
            result.negative = Some(negative);
            r?;
        }
    }
    if state.peek() != -1 {
        return Err(state.to_parse_exception("Found unquoted special character"));
    }
    Ok(())
}

fn consume_subpattern(
    state: &mut ParserState<'_>,
    result: &mut ParsedSubpatternInfo,
) -> Result<()> {
    // subpattern := literals? number exponent? literals?
    consume_padding(state, result, PadPosition::BeforePrefix)?;
    result.prefix_endpoints = consume_affix(state, result)?;
    consume_padding(state, result, PadPosition::AfterPrefix)?;
    consume_format(state, result)?;
    consume_exponent(state, result)?;
    consume_padding(state, result, PadPosition::BeforeSuffix)?;
    result.suffix_endpoints = consume_affix(state, result)?;
    consume_padding(state, result, PadPosition::AfterSuffix)?;
    Ok(())
}

fn consume_padding(
    state: &mut ParserState<'_>,
    result: &mut ParsedSubpatternInfo,
    padding_location: PadPosition,
) -> Result<()> {
    if state.peek() != i32::from(b'*') {
        return Ok(());
    }
    if result.padding_location.is_some() {
        return Err(state.to_parse_exception("Cannot have multiple pad specifiers"));
    }
    result.padding_location = Some(padding_location);
    state.next(); // consume the '*'
    result.padding_endpoints |= i64::from(state.offset);
    consume_literal(state)?;
    result.padding_endpoints |= i64::from(state.offset) << 32;
    Ok(())
}

fn consume_affix(state: &mut ParserState<'_>, result: &mut ParsedSubpatternInfo) -> Result<i64> {
    // literals := { literal }
    let mut endpoints = i64::from(state.offset);
    loop {
        match state.peek() {
            // characters that cannot be unquoted in a literal
            0x23 /* # */ | 0x40 /* @ */ | 0x3B /* ; */ | 0x2A /* * */ | 0x2E /* . */ | 0x2C /* , */
            | 0x30..=0x39 | -1 => break,
            0x25 /* % */ => result.has_percent_sign = true,
            0x2030 /* ‰ */ => result.has_per_mille_sign = true,
            0xA4 /* ¤ */ => result.has_currency_sign = true,
            0x2D /* - */ => result.has_minus_sign = true,
            0x2B /* + */ => result.has_plus_sign = true,
            _ => {}
        }
        consume_literal(state)?;
    }
    endpoints |= i64::from(state.offset) << 32;
    Ok(endpoints)
}

fn consume_literal(state: &mut ParserState<'_>) -> Result<()> {
    if state.peek() == -1 {
        return Err(state.to_parse_exception("Expected unquoted literal but found EOL"));
    } else if state.peek() == i32::from(b'\'') {
        state.next(); // consume the starting quote
        while state.peek() != i32::from(b'\'') {
            if state.peek() == -1 {
                return Err(state.to_parse_exception("Expected quoted literal but found EOL"));
            } else {
                state.next(); // consume a quoted character
            }
        }
        state.next(); // consume the ending quote
    } else {
        // consume a non-quoted literal character
        state.next();
    }
    Ok(())
}

fn consume_format(state: &mut ParserState<'_>, result: &mut ParsedSubpatternInfo) -> Result<()> {
    consume_integer_format(state, result)?;
    if state.peek() == i32::from(b'.') {
        state.next(); // consume the decimal point
        result.has_decimal = true;
        result.width_except_affixes += 1;
        consume_fraction_format(state, result)?;
    } else if state.peek() == 0xA4 {
        // Check if currency is a decimal separator
        match state.peek2() {
            0x23 /* # */ | 0x30..=0x39 => {}
            _ => {
                // Currency symbol followed by a non-numeric character;
                // treat as a normal affix.
                return Ok(());
            }
        }
        // Currency symbol is followed by a numeric character;
        // treat as a decimal separator.
        result.has_currency_sign = true;
        result.has_currency_decimal = true;
        result.has_decimal = true;
        result.width_except_affixes += 1;
        state.next(); // consume the symbol
        consume_fraction_format(state, result)?;
    }
    Ok(())
}

fn consume_integer_format(
    state: &mut ParserState<'_>,
    result: &mut ParsedSubpatternInfo,
) -> Result<()> {
    loop {
        match state.peek() {
            0x2C /* , */ => {
                result.width_except_affixes += 1;
                result.grouping_sizes <<= 16;
            }
            0x23 /* # */ => {
                if result.integer_numerals > 0 {
                    return Err(state.to_parse_exception("# cannot follow 0 before decimal point"));
                }
                result.width_except_affixes += 1;
                result.grouping_sizes += 1;
                if result.integer_at_signs > 0 {
                    result.integer_trailing_hash_signs += 1;
                } else {
                    result.integer_leading_hash_signs += 1;
                }
                result.integer_total += 1;
            }
            0x40 /* @ */ => {
                if result.integer_numerals > 0 {
                    return Err(state.to_parse_exception("Cannot mix 0 and @"));
                }
                if result.integer_trailing_hash_signs > 0 {
                    return Err(state.to_parse_exception("Cannot nest # inside of a run of @"));
                }
                result.width_except_affixes += 1;
                result.grouping_sizes += 1;
                result.integer_at_signs += 1;
                result.integer_total += 1;
            }
            0x30..=0x39 => {
                if result.integer_at_signs > 0 {
                    return Err(state.to_parse_exception("Cannot mix @ and 0"));
                }
                result.width_except_affixes += 1;
                result.grouping_sizes += 1;
                result.integer_numerals += 1;
                result.integer_total += 1;
                if state.peek() != i32::from(b'0') && result.rounding.is_none() {
                    result.rounding = Some(DecimalQuantity_DualStorageBCD::new());
                }
                if let Some(rounding) = result.rounding.as_mut() {
                    rounding.append_digit((state.peek() - i32::from(b'0')) as i8, 0, true);
                }
            }
            _ => break,
        }
        state.next(); // consume the symbol
    }

    // Disallow patterns with a trailing ',' or with two ',' next to each other
    let grouping1 = (result.grouping_sizes & 0xffff) as i16;
    let grouping2 = (((result.grouping_sizes as u64) >> 16) & 0xffff) as i16;
    let grouping3 = (((result.grouping_sizes as u64) >> 32) & 0xffff) as i16;
    if grouping1 == 0 && grouping2 != -1 {
        return Err(state.to_parse_exception("Trailing grouping separator is invalid"));
    }
    if grouping2 == 0 && grouping3 != -1 {
        return Err(state.to_parse_exception("Grouping width of zero is invalid"));
    }
    Ok(())
}

fn consume_fraction_format(
    state: &mut ParserState<'_>,
    result: &mut ParsedSubpatternInfo,
) -> Result<()> {
    let mut zero_counter = 0;
    loop {
        match state.peek() {
            0x23 /* # */ => {
                result.width_except_affixes += 1;
                result.fraction_hash_signs += 1;
                result.fraction_total += 1;
                zero_counter += 1;
            }
            0x30..=0x39 => {
                if result.fraction_hash_signs > 0 {
                    return Err(state.to_parse_exception("0 cannot follow # after decimal point"));
                }
                result.width_except_affixes += 1;
                result.fraction_numerals += 1;
                result.fraction_total += 1;
                if state.peek() == i32::from(b'0') {
                    zero_counter += 1;
                } else {
                    let rounding = result.rounding.get_or_insert_with(DecimalQuantity_DualStorageBCD::new);
                    rounding.append_digit((state.peek() - i32::from(b'0')) as i8, zero_counter, false);
                    zero_counter = 0;
                }
            }
            _ => return Ok(()),
        }
        state.next(); // consume the symbol
    }
}

fn consume_exponent(state: &mut ParserState<'_>, result: &mut ParsedSubpatternInfo) -> Result<()> {
    if state.peek() != i32::from(b'E') {
        return Ok(());
    }
    if (result.grouping_sizes & 0xffff0000) != 0xffff0000 {
        return Err(
            state.to_parse_exception("Cannot have grouping separator in scientific notation")
        );
    }
    state.next(); // consume the E
    result.width_except_affixes += 1;
    if state.peek() == i32::from(b'+') {
        state.next(); // consume the +
        result.exponent_has_plus_sign = true;
        result.width_except_affixes += 1;
    }
    while state.peek() == i32::from(b'0') {
        state.next(); // consume the 0
        result.exponent_zeros += 1;
        result.width_except_affixes += 1;
    }
    Ok(())
}

fn parse_to_existing_properties_impl(
    pattern: Option<&[u16]>,
    properties: &mut DecimalFormatProperties,
    ignore_rounding: i32,
) -> Result<()> {
    let pattern = match pattern {
        Some(p) if !p.is_empty() => p,
        _ => {
            // Backwards compatibility requires that we reset to the default
            // values. TODO: Only overwrite the properties that "saveToProperties"
            // normally touches?
            properties.clear();
            return Ok(());
        }
    };
    let pattern_info = PatternStringParser::parse_to_pattern_info(pattern)?;
    pattern_info_to_properties(properties, &pattern_info, ignore_rounding)
}

/// Finalizes the temporary data stored in the ParsedPatternInfo to the
/// Properties.
fn pattern_info_to_properties(
    properties: &mut DecimalFormatProperties,
    pattern_info: &ParsedPatternInfo,
    ignore_rounding_: i32,
) -> Result<()> {
    // Translate from PatternParseResult to Properties.
    // Note that most data from "negative" is ignored per the specification of DecimalFormat.

    let positive = &pattern_info.positive;

    let ignore_rounding = if ignore_rounding_ == PatternStringParser::IGNORE_ROUNDING_NEVER {
        false
    } else if ignore_rounding_ == PatternStringParser::IGNORE_ROUNDING_IF_CURRENCY {
        positive.has_currency_sign
    } else {
        debug_assert!(ignore_rounding_ == PatternStringParser::IGNORE_ROUNDING_ALWAYS);
        true
    };

    // Grouping settings
    let grouping1 = (positive.grouping_sizes & 0xffff) as i16;
    let grouping2 = (((positive.grouping_sizes as u64) >> 16) & 0xffff) as i16;
    let grouping3 = (((positive.grouping_sizes as u64) >> 32) & 0xffff) as i16;
    if grouping2 != -1 {
        properties.set_grouping_size(i32::from(grouping1));
        properties.set_grouping_used(true);
    } else {
        properties.set_grouping_size(-1);
        properties.set_grouping_used(false);
    }
    if grouping3 != -1 {
        properties.set_secondary_grouping_size(i32::from(grouping2));
    } else {
        properties.set_secondary_grouping_size(-1);
    }

    // For backwards compatibility, require that the pattern emit at least one min digit.
    let (min_int, min_frac);
    if positive.integer_total == 0 && positive.fraction_total > 0 {
        // patterns like ".##"
        min_int = 0;
        min_frac = 1.max(positive.fraction_numerals);
    } else if positive.integer_numerals == 0 && positive.fraction_numerals == 0 {
        // patterns like "#.##"
        min_int = 1;
        min_frac = 0;
    } else {
        min_int = positive.integer_numerals;
        min_frac = positive.fraction_numerals;
    }

    // Rounding settings
    // Don't set basic rounding when there is a currency sign; defer to CurrencyUsage
    if positive.integer_at_signs > 0 {
        properties.set_minimum_fraction_digits(-1);
        properties.set_maximum_fraction_digits(-1);
        properties.set_rounding_increment(None);
        properties.set_minimum_significant_digits(positive.integer_at_signs);
        properties.set_maximum_significant_digits(
            positive.integer_at_signs + positive.integer_trailing_hash_signs,
        );
    } else if let Some(rounding) = positive.rounding.as_ref() {
        if !ignore_rounding {
            properties.set_minimum_fraction_digits(min_frac);
            properties.set_maximum_fraction_digits(positive.fraction_total);
            let mut rounding = rounding.clone();
            properties.set_rounding_increment(Some(
                rounding
                    .to_big_decimal()?
                    .set_scale_exact(positive.fraction_numerals)?,
            ));
        } else {
            properties.set_minimum_fraction_digits(-1);
            properties.set_maximum_fraction_digits(-1);
            properties.set_rounding_increment(None);
        }
        properties.set_minimum_significant_digits(-1);
        properties.set_maximum_significant_digits(-1);
    } else {
        if !ignore_rounding {
            properties.set_minimum_fraction_digits(min_frac);
            properties.set_maximum_fraction_digits(positive.fraction_total);
            properties.set_rounding_increment(None);
        } else {
            properties.set_minimum_fraction_digits(-1);
            properties.set_maximum_fraction_digits(-1);
            properties.set_rounding_increment(None);
        }
        properties.set_minimum_significant_digits(-1);
        properties.set_maximum_significant_digits(-1);
    }

    // If the pattern ends with a '.' then force the decimal point.
    if positive.has_decimal && positive.fraction_total == 0 {
        properties.set_decimal_separator_always_shown(true);
    } else {
        properties.set_decimal_separator_always_shown(false);
    }

    // Persist the currency as decimal separator
    properties.set_currency_as_decimal(positive.has_currency_decimal);

    // Scientific notation settings
    if positive.exponent_zeros > 0 {
        properties.set_exponent_sign_always_shown(positive.exponent_has_plus_sign);
        properties.set_minimum_exponent_digits(positive.exponent_zeros);
        if positive.integer_at_signs == 0 {
            // patterns without '@' can define max integer digits, used for engineering notation
            properties.set_minimum_integer_digits(positive.integer_numerals);
            properties.set_maximum_integer_digits(positive.integer_total);
        } else {
            // patterns with '@' cannot define max integer digits
            properties.set_minimum_integer_digits(1);
            properties.set_maximum_integer_digits(-1);
        }
    } else {
        properties.set_exponent_sign_always_shown(false);
        properties.set_minimum_exponent_digits(-1);
        properties.set_minimum_integer_digits(min_int);
        properties.set_maximum_integer_digits(-1);
    }

    // Compute the affix patterns (required for both padding and affixes)
    let pos_prefix = pattern_info.get_jstring(Flags::PREFIX);
    let pos_suffix = pattern_info.get_jstring(0);

    // Padding settings
    if let Some(padding_location) = positive.padding_location {
        // The width of the positive prefix and suffix templates are included in the padding
        let padding_width = positive.width_except_affixes
            + AffixUtils::estimate_length(Some(&pos_prefix))?
            + AffixUtils::estimate_length(Some(&pos_suffix))?;
        properties.set_format_width(padding_width);
        let raw_padding_string = pattern_info.get_jstring(Flags::PADDING);
        if raw_padding_string.len() == 1 {
            properties.set_pad_string_jstring(Some(raw_padding_string));
        } else if raw_padding_string.len() == 2 {
            if raw_padding_string[0] == u16::from(b'\'') {
                properties.set_pad_string_jstring(Some(new_jstring(&[u16::from(b'\'')])));
            } else {
                properties.set_pad_string_jstring(Some(raw_padding_string));
            }
        } else {
            properties.set_pad_string_jstring(Some(new_jstring(
                &raw_padding_string[1..raw_padding_string.len() - 1],
            )));
        }
        properties.set_pad_position(Some(padding_location));
    } else {
        properties.set_format_width(-1);
        properties.set_pad_string_jstring(None);
        properties.set_pad_position(None);
    }

    // Set the affixes
    // Always call the setter, even if the prefixes are empty, especially in the case of the
    // negative prefix pattern, to prevent default values from overriding the pattern.
    properties.set_positive_prefix_pattern_jstring(Some(pos_prefix));
    properties.set_positive_suffix_pattern_jstring(Some(pos_suffix));
    if pattern_info.negative.is_some() {
        properties.set_negative_prefix_pattern_jstring(Some(
            pattern_info.get_jstring(Flags::NEGATIVE_SUBPATTERN | Flags::PREFIX),
        ));
        properties.set_negative_suffix_pattern_jstring(Some(
            pattern_info.get_jstring(Flags::NEGATIVE_SUBPATTERN),
        ));
    } else {
        properties.set_negative_prefix_pattern_jstring(None);
        properties.set_negative_suffix_pattern_jstring(None);
    }

    // Set the magnitude multiplier
    if positive.has_percent_sign {
        properties.set_magnitude_multiplier(2);
    } else if positive.has_per_mille_sign {
        properties.set_magnitude_multiplier(3);
    } else {
        properties.set_magnitude_multiplier(0);
    }
    Ok(())
}
