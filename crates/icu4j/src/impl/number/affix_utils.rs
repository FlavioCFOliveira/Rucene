// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.AffixUtils`.

use rucene::error::{LuceneError, Result};

use crate::jdk::text::{same_attribute, FieldRef};
use crate::jdk::{append_code_point, char_count, code_point_at, to_rust_string};
use crate::lang::u_character::UCharacter;
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::text::number_format::field as nf_field;
use crate::text::unicode_set::UnicodeSet;

const STATE_BASE: i32 = 0;
const STATE_FIRST_QUOTE: i32 = 1;
const STATE_INSIDE_QUOTE: i32 = 2;
const STATE_AFTER_QUOTE: i32 = 3;
const STATE_FIRST_CURR: i32 = 4;
const STATE_SECOND_CURR: i32 = 5;
const STATE_THIRD_CURR: i32 = 6;
const STATE_FOURTH_CURR: i32 = 7;
const STATE_FIFTH_CURR: i32 = 8;
const STATE_OVERFLOW_CURR: i32 = 9;

/// Represents a literal character; the value is stored in the code point
/// field.
const TYPE_CODEPOINT: i32 = 0;

/// Represents a minus sign symbol '-'.
pub const TYPE_MINUS_SIGN: i32 = -1;
/// Represents a plus sign symbol '+'.
pub const TYPE_PLUS_SIGN: i32 = -2;
/// Represents an approximately sign symbol '~'.
pub const TYPE_APPROXIMATELY_SIGN: i32 = -3;
/// Represents a percent sign symbol '%'.
pub const TYPE_PERCENT: i32 = -4;
/// Represents a permille sign symbol '‰'.
pub const TYPE_PERMILLE: i32 = -5;
/// Represents a single currency symbol '¤'.
pub const TYPE_CURRENCY_SINGLE: i32 = -6;
/// Represents a double currency symbol '¤¤'.
pub const TYPE_CURRENCY_DOUBLE: i32 = -7;
/// Represents a triple currency symbol '¤¤¤'.
pub const TYPE_CURRENCY_TRIPLE: i32 = -8;
/// Represents a quadruple currency symbol '¤¤¤¤'.
pub const TYPE_CURRENCY_QUAD: i32 = -9;
/// Represents a quintuple currency symbol '¤¤¤¤¤'.
pub const TYPE_CURRENCY_QUINT: i32 = -10;
/// Represents a sequence of six or more currency symbols.
pub const TYPE_CURRENCY_OVERFLOW: i32 = -15;

const QUOTE: i32 = '\'' as i32;
const CURRENCY_SIGN: i32 = 0xa4;
const PERMILLE_SIGN: i32 = 0x2030;

/// Provides the string of a symbol type. Equivalent to
/// `AffixUtils.SymbolProvider`.
pub trait SymbolProvider {
    /// Equivalent to `getSymbol(int)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the implementation.
    fn get_symbol(&self, type_: i32) -> Result<Vec<u16>>;
}

/// Receives the tokens of a pattern. Equivalent to
/// `AffixUtils.TokenConsumer`.
pub trait TokenConsumer {
    /// Equivalent to `consumeToken(int)`.
    fn consume_token(&mut self, type_or_cp: i32);
}

/// Performs manipulations on affix patterns: the prefix and suffix
/// strings associated with a decimal format pattern. For example:
///
/// | Affix Pattern | Example Unescaped (Formatted) String |
/// |---|---|
/// | `abc` | abc |
/// | `ab-` | ab− |
/// | `ab'-'` | ab- |
/// | `''` | ' |
///
/// To manually iterate over tokens in a literal string, use the following
/// pattern, which is zero-allocation:
///
/// ```text
/// long tag = 0L;
/// while (AffixPatternUtils.hasNext(tag, patternString)) {
///   tag = AffixPatternUtils.nextToken(tag, patternString);
///   int typeOrCp = AffixPatternUtils.getTypeOrCp(tag);
///   ...
/// }
/// ```
///
/// Equivalent to `com.ibm.icu.impl.number.AffixUtils` (static members).
/// Patterns are UTF-16; a Java `null` pattern is `None`. The
/// `IllegalArgumentException` of an unterminated quote is a `Result`.
pub struct AffixUtils;

impl AffixUtils {
    /// Estimates the number of code points present in an unescaped version
    /// of the affix pattern string (one that would be returned by
    /// `unescape`), assuming that all interpolated symbols consume one code
    /// point and that currencies consume as many code points as their
    /// symbol width. Used for computing padding width. Equivalent to
    /// `estimateLength(CharSequence)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Unterminated quote: \"<pattern>\"")`.
    pub fn estimate_length(pattern_string: Option<&[u16]>) -> Result<i32> {
        let pattern_string = match pattern_string {
            None => return Ok(0),
            Some(p) => p,
        };
        let mut state = STATE_BASE;
        let mut offset = 0;
        let mut length = 0;
        while offset < pattern_string.len() as i32 {
            let cp = code_point_at(pattern_string, offset);

            match state {
                STATE_BASE => {
                    if cp == QUOTE {
                        // First quote
                        state = STATE_FIRST_QUOTE;
                    } else {
                        // Unquoted symbol
                        length += 1;
                    }
                }
                STATE_FIRST_QUOTE => {
                    if cp == QUOTE {
                        // Repeated quote
                        length += 1;
                        state = STATE_BASE;
                    } else {
                        // Quoted code point
                        length += 1;
                        state = STATE_INSIDE_QUOTE;
                    }
                }
                STATE_INSIDE_QUOTE => {
                    if cp == QUOTE {
                        // End of quoted sequence
                        state = STATE_AFTER_QUOTE;
                    } else {
                        // Quoted code point
                        length += 1;
                    }
                }
                STATE_AFTER_QUOTE => {
                    if cp == QUOTE {
                        // Double quote inside of quoted sequence
                        length += 1;
                        state = STATE_INSIDE_QUOTE;
                    } else {
                        // Unquoted symbol
                        length += 1;
                    }
                }
                _ => panic!("java.lang.AssertionError"),
            }

            offset += char_count(cp);
        }

        match state {
            STATE_FIRST_QUOTE | STATE_INSIDE_QUOTE => Err(LuceneError::illegal_argument(format!(
                "Unterminated quote: \"{}\"",
                to_rust_string(pattern_string)
            ))),
            _ => Ok(length),
        }
    }

    /// Takes a string and escapes (quotes) characters that have special
    /// meaning in the affix pattern syntax. This function does not reverse
    /// unescape. Answers the number of units appended. Equivalent to
    /// `escape(CharSequence, StringBuilder)`.
    pub fn escape_into(input: Option<&[u16]>, output: &mut Vec<u16>) -> i32 {
        let input = match input {
            None => return 0,
            Some(i) => i,
        };
        let mut state = STATE_BASE;
        let mut offset = 0;
        let start_length = output.len() as i32;
        while offset < input.len() as i32 {
            let cp = code_point_at(input, offset);

            match cp {
                QUOTE => {
                    output.extend_from_slice(&[QUOTE as u16, QUOTE as u16]);
                }
                0x2d | 0x2b | 0x25 | PERMILLE_SIGN | CURRENCY_SIGN => {
                    // '-', '+', '%', '‰', '¤'
                    if state == STATE_BASE {
                        output.push(QUOTE as u16);
                        let _ = append_code_point(output, cp);
                        state = STATE_INSIDE_QUOTE;
                    } else {
                        let _ = append_code_point(output, cp);
                    }
                }
                _ => {
                    if state == STATE_INSIDE_QUOTE {
                        output.push(QUOTE as u16);
                        let _ = append_code_point(output, cp);
                        state = STATE_BASE;
                    } else {
                        let _ = append_code_point(output, cp);
                    }
                }
            }
            offset += char_count(cp);
        }

        if state == STATE_INSIDE_QUOTE {
            output.push(QUOTE as u16);
        }

        output.len() as i32 - start_length
    }

    /// Version of `escape` that returns a String, or `None` if input is
    /// `None`. Equivalent to `escape(CharSequence)`.
    pub fn escape(input: Option<&[u16]>) -> Option<Vec<u16>> {
        let input = input?;
        let mut sb = Vec::new();
        Self::escape_into(Some(input), &mut sb);
        Some(sb)
    }

    /// Equivalent to `getFieldForType(int)`.
    ///
    /// # Panics
    ///
    /// Java's `AssertionError` for a type that is not a symbol type.
    pub fn get_field_for_type(type_: i32) -> FieldRef {
        match type_ {
            TYPE_MINUS_SIGN => &nf_field::SIGN,
            TYPE_PLUS_SIGN => &nf_field::SIGN,
            TYPE_APPROXIMATELY_SIGN => &nf_field::APPROXIMATELY_SIGN,
            TYPE_PERCENT => &nf_field::PERCENT,
            TYPE_PERMILLE => &nf_field::PERMILLE,
            TYPE_CURRENCY_SINGLE => &nf_field::CURRENCY,
            TYPE_CURRENCY_DOUBLE => &nf_field::CURRENCY,
            TYPE_CURRENCY_TRIPLE => &nf_field::CURRENCY,
            TYPE_CURRENCY_QUAD => &nf_field::CURRENCY,
            TYPE_CURRENCY_QUINT => &nf_field::CURRENCY,
            TYPE_CURRENCY_OVERFLOW => &nf_field::CURRENCY,
            _ => panic!("java.lang.AssertionError"),
        }
    }

    /// Executes the unescape state machine. Replaces the unquoted characters
    /// "-", "+", "%", "‰", and "¤" with the corresponding symbols provided
    /// by the `SymbolProvider`, and inserts the result into the
    /// `FormattedStringBuilder` at the requested location. Answers the
    /// number of units inserted. Equivalent to `unescape(CharSequence,
    /// FormattedStringBuilder, int, SymbolProvider, NumberFormat.Field)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Unterminated quote in pattern affix:
    /// ...")`.
    pub fn unescape(
        affix_pattern: &[u16],
        output: &mut FormattedStringBuilder,
        position: i32,
        provider: &dyn SymbolProvider,
        field: Option<FieldRef>,
    ) -> Result<i32> {
        let mut length = 0;
        let mut tag: i64 = 0;
        while Self::has_next(tag, affix_pattern) {
            tag = Self::next_token(tag, affix_pattern)?;
            let type_or_cp = Self::get_type_or_cp(tag);
            if type_or_cp == TYPE_CURRENCY_OVERFLOW {
                // Don't go to the provider for this special case
                length += output.insert_code_point(
                    position + length,
                    0xFFFD,
                    Some(FieldObject::Field(&nf_field::CURRENCY)),
                );
            } else if type_or_cp < 0 {
                length += output.insert(
                    position + length,
                    &provider.get_symbol(type_or_cp)?,
                    Some(FieldObject::Field(Self::get_field_for_type(type_or_cp))),
                );
            } else {
                length += output.insert_code_point(
                    position + length,
                    type_or_cp,
                    field.map(FieldObject::Field),
                );
            }
        }
        Ok(length)
    }

    /// Same as `unescape`, but only calculates the length or code point
    /// count. More efficient than `unescape` if you only need the length but
    /// not the string itself. Equivalent to `unescapedCount(CharSequence,
    /// boolean, SymbolProvider)`.
    ///
    /// # Errors
    ///
    /// As [`unescape`](Self::unescape).
    pub fn unescaped_count(
        affix_pattern: &[u16],
        length_or_count: bool,
        provider: &dyn SymbolProvider,
    ) -> Result<i32> {
        let mut length = 0;
        let mut tag: i64 = 0;
        while Self::has_next(tag, affix_pattern) {
            tag = Self::next_token(tag, affix_pattern)?;
            let type_or_cp = Self::get_type_or_cp(tag);
            if type_or_cp == TYPE_CURRENCY_OVERFLOW {
                // U+FFFD is one char
                length += 1;
            } else if type_or_cp < 0 {
                let symbol = provider.get_symbol(type_or_cp)?;
                length += if length_or_count {
                    symbol.len() as i32
                } else {
                    UCharacter::code_point_count(&symbol, 0, symbol.len() as i32)
                };
            } else {
                length += if length_or_count {
                    char_count(type_or_cp)
                } else {
                    1
                };
            }
        }
        Ok(length)
    }

    /// Checks whether the given affix pattern contains at least one token of
    /// the given type. Equivalent to `containsType(CharSequence, int)`.
    ///
    /// # Errors
    ///
    /// As [`unescape`](Self::unescape).
    pub fn contains_type(affix_pattern: Option<&[u16]>, type_: i32) -> Result<bool> {
        let affix_pattern = match affix_pattern {
            None => return Ok(false),
            Some([]) => return Ok(false),
            Some(p) => p,
        };
        let mut tag: i64 = 0;
        while Self::has_next(tag, affix_pattern) {
            tag = Self::next_token(tag, affix_pattern)?;
            if Self::get_type_or_cp(tag) == type_ {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Checks whether the specified affix pattern has any unquoted currency
    /// symbols ("¤"). Equivalent to `hasCurrencySymbols(CharSequence)`.
    ///
    /// # Errors
    ///
    /// As [`unescape`](Self::unescape).
    pub fn has_currency_symbols(affix_pattern: Option<&[u16]>) -> Result<bool> {
        let affix_pattern = match affix_pattern {
            None => return Ok(false),
            Some([]) => return Ok(false),
            Some(p) => p,
        };
        let mut tag: i64 = 0;
        while Self::has_next(tag, affix_pattern) {
            tag = Self::next_token(tag, affix_pattern)?;
            let type_or_cp = Self::get_type_or_cp(tag);
            if type_or_cp < 0
                && same_attribute(Self::get_field_for_type(type_or_cp), &nf_field::CURRENCY)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Replaces all occurrences of tokens with the given type with the given
    /// replacement char. Equivalent to `replaceType(CharSequence, int,
    /// char)`.
    ///
    /// # Errors
    ///
    /// As [`unescape`](Self::unescape).
    pub fn replace_type(
        affix_pattern: Option<&[u16]>,
        type_: i32,
        replacement_char: u16,
    ) -> Result<Vec<u16>> {
        let affix_pattern = match affix_pattern {
            None => return Ok(Vec::new()),
            Some([]) => return Ok(Vec::new()),
            Some(p) => p,
        };
        let mut chars = affix_pattern.to_vec();
        let mut tag: i64 = 0;
        while Self::has_next(tag, affix_pattern) {
            tag = Self::next_token(tag, affix_pattern)?;
            if Self::get_type_or_cp(tag) == type_ {
                let offset = Self::get_offset(tag);
                chars[(offset - 1) as usize] = replacement_char;
            }
        }
        Ok(chars)
    }

    /// Returns whether the given affix pattern contains only symbols and
    /// ignorables as defined by the given ignorables set. Equivalent to
    /// `containsOnlySymbolsAndIgnorables(CharSequence, UnicodeSet)`.
    ///
    /// # Errors
    ///
    /// As [`unescape`](Self::unescape).
    pub fn contains_only_symbols_and_ignorables(
        affix_pattern: Option<&[u16]>,
        ignorables: &UnicodeSet,
    ) -> Result<bool> {
        let affix_pattern = match affix_pattern {
            None => return Ok(true),
            Some(p) => p,
        };
        let mut tag: i64 = 0;
        while Self::has_next(tag, affix_pattern) {
            tag = Self::next_token(tag, affix_pattern)?;
            let type_or_cp = Self::get_type_or_cp(tag);
            if type_or_cp >= 0 && !ignorables.contains(type_or_cp)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Iterates over the affix pattern, calling the TokenConsumer for each
    /// token. Equivalent to `iterateWithConsumer(CharSequence,
    /// TokenConsumer)`.
    ///
    /// # Errors
    ///
    /// As [`unescape`](Self::unescape).
    pub fn iterate_with_consumer(
        affix_pattern: &[u16],
        consumer: &mut dyn TokenConsumer,
    ) -> Result<()> {
        let mut tag: i64 = 0;
        while Self::has_next(tag, affix_pattern) {
            tag = Self::next_token(tag, affix_pattern)?;
            let type_or_cp = Self::get_type_or_cp(tag);
            consumer.consume_token(type_or_cp);
        }
        Ok(())
    }

    /// Returns the next token from the affix pattern. Equivalent to
    /// `nextToken(long, CharSequence)`.
    fn next_token(tag: i64, pattern_string: &[u16]) -> Result<i64> {
        let mut offset = Self::get_offset(tag);
        let mut state = Self::get_state(tag);
        while offset < pattern_string.len() as i32 {
            let cp = code_point_at(pattern_string, offset);
            let count = char_count(cp);

            match state {
                STATE_BASE => match cp {
                    QUOTE => {
                        state = STATE_FIRST_QUOTE;
                        offset += count;
                        // continue to the next code point
                    }
                    0x2d => {
                        return Ok(Self::make_tag(
                            offset + count,
                            TYPE_MINUS_SIGN,
                            STATE_BASE,
                            0,
                        ))
                    }
                    0x2b => {
                        return Ok(Self::make_tag(
                            offset + count,
                            TYPE_PLUS_SIGN,
                            STATE_BASE,
                            0,
                        ))
                    }
                    0x7e => {
                        return Ok(Self::make_tag(
                            offset + count,
                            TYPE_APPROXIMATELY_SIGN,
                            STATE_BASE,
                            0,
                        ))
                    }
                    0x25 => return Ok(Self::make_tag(offset + count, TYPE_PERCENT, STATE_BASE, 0)),
                    PERMILLE_SIGN => {
                        return Ok(Self::make_tag(offset + count, TYPE_PERMILLE, STATE_BASE, 0))
                    }
                    CURRENCY_SIGN => {
                        state = STATE_FIRST_CURR;
                        offset += count;
                        // continue to the next code point
                    }
                    _ => {
                        return Ok(Self::make_tag(
                            offset + count,
                            TYPE_CODEPOINT,
                            STATE_BASE,
                            cp,
                        ))
                    }
                },
                STATE_FIRST_QUOTE => {
                    return if cp == QUOTE {
                        // Repeated quote
                        Ok(Self::make_tag(
                            offset + count,
                            TYPE_CODEPOINT,
                            STATE_BASE,
                            cp,
                        ))
                    } else {
                        // Quoted code point
                        Ok(Self::make_tag(
                            offset + count,
                            TYPE_CODEPOINT,
                            STATE_INSIDE_QUOTE,
                            cp,
                        ))
                    };
                }
                STATE_INSIDE_QUOTE => {
                    if cp == QUOTE {
                        state = STATE_AFTER_QUOTE;
                        offset += count;
                        // continue to the next code point
                    } else {
                        // Quoted code point
                        return Ok(Self::make_tag(
                            offset + count,
                            TYPE_CODEPOINT,
                            STATE_INSIDE_QUOTE,
                            cp,
                        ));
                    }
                }
                STATE_AFTER_QUOTE => {
                    if cp == QUOTE {
                        // Double quote inside of quoted sequence
                        return Ok(Self::make_tag(
                            offset + count,
                            TYPE_CODEPOINT,
                            STATE_INSIDE_QUOTE,
                            cp,
                        ));
                    } else {
                        // Unquoted symbol
                        state = STATE_BASE;
                        // Go to state 0, but don't increase offset
                    }
                }
                STATE_FIRST_CURR => {
                    if cp == CURRENCY_SIGN {
                        state = STATE_SECOND_CURR;
                        offset += count;
                    } else {
                        return Ok(Self::make_tag(offset, TYPE_CURRENCY_SINGLE, STATE_BASE, 0));
                    }
                }
                STATE_SECOND_CURR => {
                    if cp == CURRENCY_SIGN {
                        state = STATE_THIRD_CURR;
                        offset += count;
                    } else {
                        return Ok(Self::make_tag(offset, TYPE_CURRENCY_DOUBLE, STATE_BASE, 0));
                    }
                }
                STATE_THIRD_CURR => {
                    if cp == CURRENCY_SIGN {
                        state = STATE_FOURTH_CURR;
                        offset += count;
                    } else {
                        return Ok(Self::make_tag(offset, TYPE_CURRENCY_TRIPLE, STATE_BASE, 0));
                    }
                }
                STATE_FOURTH_CURR => {
                    if cp == CURRENCY_SIGN {
                        state = STATE_FIFTH_CURR;
                        offset += count;
                    } else {
                        return Ok(Self::make_tag(offset, TYPE_CURRENCY_QUAD, STATE_BASE, 0));
                    }
                }
                STATE_FIFTH_CURR => {
                    if cp == CURRENCY_SIGN {
                        state = STATE_OVERFLOW_CURR;
                        offset += count;
                    } else {
                        return Ok(Self::make_tag(offset, TYPE_CURRENCY_QUINT, STATE_BASE, 0));
                    }
                }
                STATE_OVERFLOW_CURR => {
                    if cp == CURRENCY_SIGN {
                        offset += count;
                    } else {
                        return Ok(Self::make_tag(
                            offset,
                            TYPE_CURRENCY_OVERFLOW,
                            STATE_BASE,
                            0,
                        ));
                    }
                }
                _ => panic!("java.lang.AssertionError"),
            }
        }
        // End of string
        match state {
            // No more tokens in string.
            STATE_BASE => Ok(-1),
            STATE_FIRST_QUOTE | STATE_INSIDE_QUOTE => {
                // For consistent behavior with the JDK and ICU 58, throw an
                // exception here.
                Err(LuceneError::illegal_argument(format!(
                    "Unterminated quote in pattern affix: \"{}\"",
                    to_rust_string(pattern_string)
                )))
            }
            // No more tokens in string.
            STATE_AFTER_QUOTE => Ok(-1),
            STATE_FIRST_CURR => Ok(Self::make_tag(offset, TYPE_CURRENCY_SINGLE, STATE_BASE, 0)),
            STATE_SECOND_CURR => Ok(Self::make_tag(offset, TYPE_CURRENCY_DOUBLE, STATE_BASE, 0)),
            STATE_THIRD_CURR => Ok(Self::make_tag(offset, TYPE_CURRENCY_TRIPLE, STATE_BASE, 0)),
            STATE_FOURTH_CURR => Ok(Self::make_tag(offset, TYPE_CURRENCY_QUAD, STATE_BASE, 0)),
            STATE_FIFTH_CURR => Ok(Self::make_tag(offset, TYPE_CURRENCY_QUINT, STATE_BASE, 0)),
            STATE_OVERFLOW_CURR => Ok(Self::make_tag(
                offset,
                TYPE_CURRENCY_OVERFLOW,
                STATE_BASE,
                0,
            )),
            _ => panic!("java.lang.AssertionError"),
        }
    }

    /// Returns whether the affix pattern string has any more tokens to be
    /// retrieved from a call to `nextToken`. Equivalent to `hasNext(long,
    /// CharSequence)`.
    fn has_next(tag: i64, string: &[u16]) -> bool {
        debug_assert!(tag >= 0);
        let state = Self::get_state(tag);
        let offset = Self::get_offset(tag);
        // Special case: the last character in string is an end quote.
        if state == STATE_INSIDE_QUOTE
            && offset == string.len() as i32 - 1
            && string[offset as usize] == QUOTE as u16
        {
            false
        } else if state != STATE_BASE {
            true
        } else {
            offset < string.len() as i32
        }
    }

    /// This function helps determine the identity of the token consumed by
    /// `nextToken`. Converts from a bitmask tag, based on a call to
    /// `nextToken`, to its corresponding symbol type or code point.
    /// Equivalent to `getTypeOrCp(long)`.
    fn get_type_or_cp(tag: i64) -> i32 {
        debug_assert!(tag >= 0);
        let type_ = Self::get_type(tag);
        if type_ == TYPE_CODEPOINT {
            Self::get_code_point(tag)
        } else {
            -type_
        }
    }

    /// Encodes the given values into a 64-bit tag. Equivalent to
    /// `makeTag(int, int, int, int)`.
    fn make_tag(offset: i32, type_: i32, state: i32, cp: i32) -> i64 {
        let mut tag: i64 = 0;
        tag |= offset as i64;
        tag |= (-(type_ as i64)) << 32;
        tag |= (state as i64) << 36;
        tag |= (cp as i64) << 40;
        debug_assert!(tag >= 0);
        tag
    }

    fn get_offset(tag: i64) -> i32 {
        // `tag & 0xffffffff`: the int literal -1 widens to -1L.
        tag as i32
    }

    fn get_type(tag: i64) -> i32 {
        (((tag as u64) >> 32) & 0xf) as i32
    }

    fn get_state(tag: i64) -> i32 {
        (((tag as u64) >> 36) & 0xf) as i32
    }

    fn get_code_point(tag: i64) -> i32 {
        ((tag as u64) >> 40) as i32
    }
}
