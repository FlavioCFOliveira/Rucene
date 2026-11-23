// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.PatternStringUtils`.

use rucene::error::{LuceneError, Result};

use crate::jdk::math::BigDecimal;
use crate::jdk::units;
use crate::r#impl::number::affix_pattern_provider::{
    AffixPatternProvider, Flags, FLAG_NEG_PREFIX, FLAG_NEG_SUFFIX, FLAG_POS_PREFIX, FLAG_POS_SUFFIX,
};
use crate::r#impl::number::decimal_format_properties::DecimalFormatProperties;
use crate::r#impl::number::modifier::Signum;
use crate::r#impl::number::padder::{PadPosition, FALLBACK_PADDING_STRING};
use crate::r#impl::number::properties_affix_pattern_provider::PropertiesAffixPatternProvider;
use crate::r#impl::standard_plural::StandardPlural;
use crate::number::number_formatter::SignDisplay;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;

/// Equivalent to `PatternStringUtils.PatternSignType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternSignType {
    /// Render using normal positive subpattern rules.
    Pos,
    /// Render using rules to force the display of a plus sign.
    PosSign,
    /// Render using negative subpattern rules.
    Neg,
}

impl PatternSignType {
    /// `VALUES`.
    pub const VALUES: [PatternSignType; 3] = [PatternSignType::Pos, PatternSignType::PosSign, PatternSignType::Neg];
}

/// Assorted utilities relating to decimal formatting pattern strings.
///
/// Equivalent to `com.ibm.icu.impl.number.PatternStringUtils` (static
/// members).
pub struct PatternStringUtils;

impl PatternStringUtils {
    /// Determine whether a given roundingIncrement should be ignored for
    /// formatting based on the current maxFrac value (maximum fraction
    /// digits). Equivalent to `ignoreRoundingIncrement(BigDecimal, int)`.
    pub fn ignore_rounding_increment(round_incr_dec: &BigDecimal, max_frac: i32) -> bool {
        let mut round_incr = round_incr_dec.double_value();
        if round_incr == 0.0 {
            return true;
        }
        if max_frac < 0 {
            return false;
        }
        let mut frac = 0;
        round_incr *= 2.0; // This handles the rounding up of values above e.g. 0.005 or 0.0005
        while frac <= max_frac && round_incr <= 1.0 {
            frac += 1;
            round_incr *= 10.0;
        }
        frac > max_frac
    }

    /// Creates a pattern string from a property bag. Since pattern strings
    /// support only a subset of the functionality available in a property
    /// bag, a new property bag created from the string returned by this
    /// function may not be the same as the original property bag.
    /// Equivalent to `propertiesToPatternString(DecimalFormatProperties)`.
    ///
    /// # Errors
    ///
    /// The errors of `PropertiesAffixPatternProvider.forProperties` and of
    /// `BigDecimal.scaleByPowerOfTen`.
    pub fn properties_to_pattern_string(properties: &DecimalFormatProperties) -> Result<Vec<u16>> {
        let mut sb: Vec<u16> = Vec::new();

        // Convenience references
        // The Math.min() calls prevent DoS
        let dos_max = 100;
        let mut grouping1 = 0.max(properties.get_grouping_size().min(dos_max));
        let mut grouping2 = 0.max(properties.get_secondary_grouping_size().min(dos_max));
        let use_grouping = properties.get_grouping_used();
        let padding_width = properties.get_format_width().min(dos_max);
        let padding_location = properties.get_pad_position();
        let padding_string = properties.get_pad_string();
        let min_int = 0.max(properties.get_minimum_integer_digits().min(dos_max));
        let max_int = properties.get_maximum_integer_digits().min(dos_max);
        let min_frac = 0.max(properties.get_minimum_fraction_digits().min(dos_max));
        let max_frac = properties.get_maximum_fraction_digits().min(dos_max);
        let min_sig = properties.get_minimum_significant_digits().min(dos_max);
        let max_sig = properties.get_maximum_significant_digits().min(dos_max);
        let always_show_decimal = properties.get_decimal_separator_always_shown();
        let currency_as_decimal = properties.get_currency_as_decimal();
        let exponent_digits = properties.get_minimum_exponent_digits().min(dos_max);
        let exponent_show_plus_sign = properties.get_exponent_sign_always_shown();

        let affixes = PropertiesAffixPatternProvider::for_properties(properties)?;

        // Prefixes
        sb.extend_from_slice(&affixes.get_string(FLAG_POS_PREFIX));
        let mut after_prefix_pos = sb.len();

        // Figure out the grouping sizes.
        if !use_grouping {
            grouping1 = 0;
            grouping2 = 0;
        } else if grouping1 == grouping2 {
            grouping1 = 0;
        }
        let grouping_length = grouping1 + grouping2 + 1;

        // Figure out the digits we need to put in the pattern.
        let rounding_interval = properties.get_rounding_increment();
        let mut digits_string: Vec<u16> = Vec::new();
        let mut digits_string_scale: i32 = 0;
        if max_sig != dos_max.min(-1) {
            // Significant Digits.
            while (digits_string.len() as i32) < min_sig {
                digits_string.push(u16::from(b'@'));
            }
            while (digits_string.len() as i32) < max_sig {
                digits_string.push(u16::from(b'#'));
            }
        } else if let Some(rounding_interval) =
            rounding_interval.as_ref().filter(|r| !Self::ignore_rounding_increment(r, max_frac))
        {
            // Rounding Interval.
            digits_string_scale = -rounding_interval.scale();
            // TODO: Check for DoS here?
            let str = rounding_interval.scale_by_power_of_ten(rounding_interval.scale())?.to_plain_string();
            let str = units(&str);
            if str[0] == u16::from(b'-') {
                // TODO: Unsupported operation exception or fail silently?
                digits_string.extend_from_slice(&str[1..]);
            } else {
                digits_string.extend_from_slice(&str);
            }
        }
        while (digits_string.len() as i32) + digits_string_scale < min_int {
            digits_string.insert(0, u16::from(b'0'));
        }
        while -digits_string_scale < min_frac {
            digits_string.push(u16::from(b'0'));
            digits_string_scale -= 1;
        }

        // Write the digits to the string builder
        let mut m0 = grouping_length.max(digits_string.len() as i32 + digits_string_scale);
        m0 = if max_int != dos_max { max_int.max(m0) - 1 } else { m0 - 1 };
        let m_n = if max_frac != dos_max { (-max_frac).min(digits_string_scale) } else { digits_string_scale };
        let mut magnitude = m0;
        while magnitude >= m_n {
            let di = digits_string.len() as i32 + digits_string_scale - magnitude - 1;
            if di < 0 || di >= digits_string.len() as i32 {
                sb.push(u16::from(b'#'));
            } else {
                sb.push(digits_string[di as usize]);
            }
            // Decimal separator
            if magnitude == 0 && (always_show_decimal || m_n < 0) {
                if currency_as_decimal {
                    sb.push(0xA4);
                } else {
                    sb.push(u16::from(b'.'));
                }
            }
            if use_grouping {
                // Grouping separators
                if magnitude > 0 && magnitude == grouping1 {
                    sb.push(u16::from(b','));
                }
                if magnitude > grouping1 && grouping2 > 0 && (magnitude - grouping1) % grouping2 == 0 {
                    sb.push(u16::from(b','));
                }
            }
            magnitude -= 1;
        }

        // Exponential notation
        if exponent_digits != dos_max.min(-1) {
            sb.push(u16::from(b'E'));
            if exponent_show_plus_sign {
                sb.push(u16::from(b'+'));
            }
            for _ in 0..exponent_digits {
                sb.push(u16::from(b'0'));
            }
        }

        // Suffixes
        let mut before_suffix_pos = sb.len();
        sb.extend_from_slice(&affixes.get_string(FLAG_POS_SUFFIX));

        // Resolve Padding
        if padding_width > 0 {
            while padding_width - sb.len() as i32 > 0 {
                sb.insert(after_prefix_pos, u16::from(b'#'));
                before_suffix_pos += 1;
            }
            let added_length;
            // Java switches on the enum: a null padding location throws
            // NullPointerException.
            let padding_location =
                padding_location.ok_or_else(|| crate::jdk::null_pointer("padPosition"))?;
            match padding_location {
                PadPosition::BeforePrefix => {
                    added_length = Self::escape_padding_string(padding_string, &mut sb, 0);
                    sb.insert(0, u16::from(b'*'));
                    after_prefix_pos += added_length + 1;
                    before_suffix_pos += added_length + 1;
                }
                PadPosition::AfterPrefix => {
                    added_length = Self::escape_padding_string(padding_string, &mut sb, after_prefix_pos);
                    sb.insert(after_prefix_pos, u16::from(b'*'));
                    after_prefix_pos += added_length + 1;
                    before_suffix_pos += added_length + 1;
                }
                PadPosition::BeforeSuffix => {
                    Self::escape_padding_string(padding_string, &mut sb, before_suffix_pos);
                    sb.insert(before_suffix_pos, u16::from(b'*'));
                }
                PadPosition::AfterSuffix => {
                    sb.push(u16::from(b'*'));
                    let len = sb.len();
                    Self::escape_padding_string(padding_string, &mut sb, len);
                }
            }
        }

        // Negative affixes
        // Ignore if the negative prefix pattern is "-" and the negative suffix is empty
        if affixes.has_negative_subpattern() {
            sb.push(u16::from(b';'));
            sb.extend_from_slice(&affixes.get_string(FLAG_NEG_PREFIX));
            // Copy the positive digit format into the negative.
            // This is optional; the pattern is the same as if '#' were appended here instead.
            let body: Vec<u16> = sb[after_prefix_pos..before_suffix_pos].to_vec();
            sb.extend_from_slice(&body);
            sb.extend_from_slice(&affixes.get_string(FLAG_NEG_SUFFIX));
        }

        Ok(sb)
    }

    /// `escapePaddingString(CharSequence, StringBuilder, int)`: the number
    /// of units inserted.
    fn escape_padding_string(input: Option<&[u16]>, output: &mut Vec<u16>, start_index: usize) -> usize {
        let fallback = units(FALLBACK_PADDING_STRING);
        let input: &[u16] = match input {
            Some(i) if !i.is_empty() => i,
            _ => &fallback,
        };
        let start_length = output.len();
        if input.len() == 1 {
            if input == [u16::from(b'\'')] {
                output.splice(start_index..start_index, [u16::from(b'\''), u16::from(b'\'')]);
            } else {
                output.splice(start_index..start_index, input.iter().copied());
            }
        } else {
            output.insert(start_index, u16::from(b'\''));
            let mut offset = 1;
            for &ch in input {
                // it's okay to deal in chars here because the quote mark is the only interesting thing.
                if ch == u16::from(b'\'') {
                    output.splice(start_index + offset..start_index + offset, [u16::from(b'\''), u16::from(b'\'')]);
                    offset += 2;
                } else {
                    output.insert(start_index + offset, ch);
                    offset += 1;
                }
            }
            output.insert(start_index + offset, u16::from(b'\''));
        }
        output.len() - start_length
    }

    /// Converts a pattern between standard notation and localized notation.
    /// Localized notation means that instead of using generic placeholders
    /// in the pattern, you use the corresponding locale-specific characters
    /// instead. Equivalent to `convertLocalized(String,
    /// DecimalFormatSymbols, boolean)`; `None` for a `null` input.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Malformed localized pattern: unterminated
    /// quote")`.
    pub fn convert_localized(
        input: Option<&[u16]>,
        symbols: &DecimalFormatSymbols,
        to_localized: bool,
    ) -> Result<Option<Vec<u16>>> {
        let Some(input) = input else {
            return Ok(None);
        };

        // Construct a table of strings to be converted between localized and standard.
        let mut table: Vec<[Vec<u16>; 2]> = vec![[Vec::new(), Vec::new()]; 21];
        let stand_idx = if to_localized { 0 } else { 1 };
        let local_idx = if to_localized { 1 } else { 0 };
        table[0][stand_idx] = units("%");
        table[0][local_idx] = symbols.get_percent_string().to_vec();
        table[1][stand_idx] = units("‰");
        table[1][local_idx] = symbols.get_per_mill_string().to_vec();
        table[2][stand_idx] = units(".");
        table[2][local_idx] = symbols.get_decimal_separator_string().to_vec();
        table[3][stand_idx] = units(",");
        table[3][local_idx] = symbols.get_grouping_separator_string().to_vec();
        table[4][stand_idx] = units("-");
        table[4][local_idx] = symbols.get_minus_sign_string().to_vec();
        table[5][stand_idx] = units("+");
        table[5][local_idx] = symbols.get_plus_sign_string().to_vec();
        table[6][stand_idx] = units(";");
        table[6][local_idx] = vec![symbols.get_pattern_separator()];
        table[7][stand_idx] = units("@");
        table[7][local_idx] = vec![symbols.get_significant_digit()];
        table[8][stand_idx] = units("E");
        table[8][local_idx] = symbols.get_exponent_separator().to_vec();
        table[9][stand_idx] = units("*");
        table[9][local_idx] = vec![symbols.get_pad_escape()];
        table[10][stand_idx] = units("#");
        table[10][local_idx] = vec![symbols.get_digit()];
        for i in 0..10 {
            table[11 + i][stand_idx] = vec![u16::from(b'0') + i as u16];
            table[11 + i][local_idx] = symbols.get_digit_strings_local()[i].clone();
        }

        // Special case: quotes are NOT allowed to be in any localIdx strings.
        // Substitute them with '’' instead.
        for row in table.iter_mut() {
            for u in row[local_idx].iter_mut() {
                if *u == u16::from(b'\'') {
                    *u = 0x2019;
                }
            }
        }

        // Iterate through the string and convert.
        // State table:
        // 0 => base state
        // 1 => first char inside a quoted sequence in input and output string
        // 2 => inside a quoted sequence in input and output string
        // 3 => first char after a close quote in input string;
        // close quote still needs to be written to output string
        // 4 => base state in input string; inside quoted sequence in output string
        // 5 => first char inside a quoted sequence in input string;
        // inside quoted sequence in output string
        let mut result: Vec<u16> = Vec::new();
        let mut state = 0;
        let mut offset = 0usize;
        'outer: while offset < input.len() {
            let ch = input[offset];

            // Handle a quote character (state shift)
            if ch == u16::from(b'\'') {
                if state == 0 {
                    result.push(u16::from(b'\''));
                    state = 1;
                } else if state == 1 {
                    result.push(u16::from(b'\''));
                    state = 0;
                } else if state == 2 {
                    state = 3;
                } else if state == 3 {
                    result.push(u16::from(b'\''));
                    result.push(u16::from(b'\''));
                    state = 1;
                } else if state == 4 {
                    state = 5;
                } else {
                    debug_assert!(state == 5);
                    result.push(u16::from(b'\''));
                    result.push(u16::from(b'\''));
                    state = 4;
                }
                offset += 1;
                continue;
            }

            if state == 0 || state == 3 || state == 4 {
                for pair in &table {
                    // Perform a greedy match on this symbol string
                    if region_matches(input, offset, &pair[0]) {
                        // Skip ahead past this region for the next iteration
                        offset += pair[0].len();
                        if state == 3 || state == 4 {
                            result.push(u16::from(b'\''));
                            state = 0;
                        }
                        result.extend_from_slice(&pair[1]);
                        continue 'outer;
                    }
                }
                // No replacement found. Check if a special quote is necessary
                for pair in &table {
                    if region_matches(input, offset, &pair[1]) {
                        if state == 0 {
                            result.push(u16::from(b'\''));
                            state = 4;
                        }
                        result.push(ch);
                        offset += 1;
                        continue 'outer;
                    }
                }
                // Still nothing. Copy the char verbatim. (Add a close quote if necessary)
                if state == 3 || state == 4 {
                    result.push(u16::from(b'\''));
                    state = 0;
                }
                result.push(ch);
            } else {
                debug_assert!(state == 1 || state == 2 || state == 5);
                result.push(ch);
                state = 2;
            }
            offset += 1;
        }
        // Resolve final quotes
        if state == 3 || state == 4 {
            result.push(u16::from(b'\''));
            state = 0;
        }
        if state != 0 {
            return Err(LuceneError::illegal_argument("Malformed localized pattern: unterminated quote"));
        }
        Ok(Some(result))
    }

    /// This method contains the heart of the logic for rendering LDML affix
    /// strings. It handles sign-always-shown resolution, whether to use the
    /// positive or negative subpattern, permille substitution, and plural
    /// forms for CurrencyPluralInfo. Equivalent to
    /// `patternInfoToStringBuilder(AffixPatternProvider, boolean,
    /// PatternSignType, boolean, StandardPlural, boolean, StringBuilder)`
    /// (`None` for a `null` plural).
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of a malformed affix pattern.
    pub fn pattern_info_to_string_builder(
        pattern_info: &dyn AffixPatternProvider,
        is_prefix: bool,
        pattern_sign_type: PatternSignType,
        approximately: bool,
        plural: Option<StandardPlural>,
        per_mille_replaces_percent: bool,
        output: &mut Vec<u16>,
    ) -> Result<()> {
        let plus_replaces_minus_sign =
            pattern_sign_type == PatternSignType::PosSign && !pattern_info.positive_has_plus_sign()?;

        // Should we use the affix from the pattern string, or will we need to synthesize one?
        let use_negative_affix_pattern = pattern_info.has_negative_subpattern()
            && (pattern_sign_type == PatternSignType::Neg
                || (pattern_info.negative_has_minus_sign()? && (plus_replaces_minus_sign || approximately)));

        // Resolve the flags for the affix pattern.
        let mut flags = 0;
        if use_negative_affix_pattern {
            flags |= Flags::NEGATIVE_SUBPATTERN;
        }
        if is_prefix {
            flags |= Flags::PREFIX;
        }
        if let Some(plural) = plural {
            debug_assert!(plural as i32 == (Flags::PLURAL_MASK & plural as i32));
            flags |= plural as i32;
        }

        // Should we prepend a sign to the pattern?
        let prepend_sign = if !is_prefix || use_negative_affix_pattern {
            false
        } else if pattern_sign_type == PatternSignType::Neg {
            true
        } else {
            plus_replaces_minus_sign || approximately
        };

        // What symbols should take the place of the sign placeholder?
        let sign_symbols: &[u16] = if approximately {
            if plus_replaces_minus_sign {
                &[0x7E, 0x2B] // "~+"
            } else if pattern_sign_type == PatternSignType::Neg {
                &[0x7E, 0x2D] // "~-"
            } else {
                &[0x7E] // "~"
            }
        } else if plus_replaces_minus_sign {
            &[0x2B] // "+"
        } else {
            &[0x2D] // "-"
        };

        // Compute the length of the affix pattern.
        let length = pattern_info.length(flags) + i32::from(prepend_sign);

        // Finally, set the result into the StringBuilder.
        output.clear();
        for index in 0..length {
            let mut candidate = if prepend_sign && index == 0 {
                u16::from(b'-')
            } else if prepend_sign {
                pattern_info.char_at(flags, index - 1)
            } else {
                pattern_info.char_at(flags, index)
            };
            if candidate == u16::from(b'-') {
                if sign_symbols.len() == 1 {
                    candidate = sign_symbols[0];
                } else {
                    output.push(sign_symbols[0]);
                    candidate = sign_symbols[1];
                }
            }
            if per_mille_replaces_percent && candidate == u16::from(b'%') {
                candidate = 0x2030; // '‰'
            }
            output.push(candidate);
        }
        Ok(())
    }

    /// Equivalent to `resolveSignDisplay(SignDisplay, Signum)`.
    pub fn resolve_sign_display(sign_display: SignDisplay, signum: Signum) -> PatternSignType {
        match sign_display {
            SignDisplay::Auto | SignDisplay::Accounting => match signum {
                Signum::Neg | Signum::NegZero => PatternSignType::Neg,
                Signum::PosZero | Signum::Pos => PatternSignType::Pos,
            },
            SignDisplay::Always | SignDisplay::AccountingAlways => match signum {
                Signum::Neg | Signum::NegZero => PatternSignType::Neg,
                Signum::PosZero | Signum::Pos => PatternSignType::PosSign,
            },
            SignDisplay::ExceptZero | SignDisplay::AccountingExceptZero => match signum {
                Signum::Neg => PatternSignType::Neg,
                Signum::NegZero | Signum::PosZero => PatternSignType::Pos,
                Signum::Pos => PatternSignType::PosSign,
            },
            SignDisplay::Negative | SignDisplay::AccountingNegative => match signum {
                Signum::Neg => PatternSignType::Neg,
                Signum::NegZero | Signum::PosZero | Signum::Pos => PatternSignType::Pos,
            },
            SignDisplay::Never => PatternSignType::Pos,
        }
    }
}

/// `String.regionMatches(offset, other, 0, other.length())`.
fn region_matches(input: &[u16], offset: usize, other: &[u16]) -> bool {
    offset + other.len() <= input.len() && input[offset..offset + other.len()] == *other
}
