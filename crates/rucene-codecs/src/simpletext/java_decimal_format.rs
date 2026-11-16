/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The JDK 25 `java.text.DecimalFormat` behaviour the `SimpleText` readers
//! depend on: `new DecimalFormat(pattern, DecimalFormatSymbols.getInstance(
//! Locale.ROOT))`, `setParseBigDecimal`, and the lenient `parse(String)`,
//! plus the `BigDecimal.toBigIntegerExact()` and `BigInteger` range checks
//! `SimpleTextDocValuesReader` applies to the parsed value.
//!
//! Written from the JDK 25 sources of `DecimalFormat` (`applyPattern`,
//! `expandAffix`, `parse`, `subparse`, `subparseNumber`), `DigitList`,
//! `BigDecimal` and `BigInteger`, read as specification only, and checked
//! against a JDK 25 probe over a grid of patterns and inputs
//! (`testdata/decimal_format_grid.tsv`). The `Locale.ROOT` symbols, measured
//! on that JDK: zero `0`, grouping `,`, decimal `.` (monetary separators the
//! same), percent `%`, per mille `‰`, minus `-`, exponent `E`, `NaN`, `∞`,
//! currency `¤`, international currency `XXX`.

use rucene::error::{LuceneError, Result};
use rucene::util::extra::ParseException;

const QUOTE: u16 = b'\'' as u16;
const CURRENCY_SIGN: u16 = 0x00A4;
const PATTERN_DIGIT: u16 = b'#' as u16;
const PATTERN_ZERO_DIGIT: u16 = b'0' as u16;
const PATTERN_GROUPING_SEPARATOR: u16 = b',' as u16;
const PATTERN_DECIMAL_SEPARATOR: u16 = b'.' as u16;
const PATTERN_PERCENT: u16 = b'%' as u16;
const PATTERN_PER_MILLE: u16 = 0x2030;
const PATTERN_SEPARATOR: u16 = b';' as u16;
const PATTERN_EXPONENT: u16 = b'E' as u16;
const PATTERN_MINUS: u16 = b'-' as u16;

/// `DecimalFormatSymbols(Locale.ROOT)`: the decimal separator.
const DECIMAL_SEPARATOR: u16 = b'.' as u16;
/// `DecimalFormatSymbols(Locale.ROOT)`: the grouping separator.
const GROUPING_SEPARATOR: u16 = b',' as u16;
/// `DecimalFormatSymbols(Locale.ROOT)`: the exponent separator.
const EXPONENT_SEPARATOR: u16 = b'E' as u16;
/// `DecimalFormatSymbols(Locale.ROOT)`: the minus sign text.
const MINUS_SIGN: u16 = b'-' as u16;
/// `DecimalFormatSymbols(Locale.ROOT).getNaN()`.
const NAN: [u16; 3] = [b'N' as u16, b'a' as u16, b'N' as u16];
/// `DecimalFormatSymbols(Locale.ROOT).getInfinity()`.
const INFINITY: u16 = 0x221E;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn region_matches(text: &[u16], position: usize, what: &[u16]) -> bool {
    text.len() >= position && text[position..].starts_with(what)
}

/// A `DecimalFormat` over `DecimalFormatSymbols(Locale.ROOT)`: the expanded
/// affixes, the multiplier and the grouping flag of the applied pattern —
/// what `parse` reads.
#[derive(Debug, Clone)]
pub(crate) struct DecimalFormat {
    positive_prefix: Vec<u16>,
    positive_suffix: Vec<u16>,
    negative_prefix: Vec<u16>,
    negative_suffix: Vec<u16>,
    multiplier: i32,
    grouping_used: bool,
    parse_big_decimal: bool,
}

/// Which affix the pattern loop appends to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Affix {
    Prefix,
    Suffix,
}

impl DecimalFormat {
    /// `new DecimalFormat(pattern, DecimalFormatSymbols.getInstance(Locale.ROOT))`:
    /// `applyPattern(pattern, false)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` with `applyPattern`'s messages for a
    /// malformed pattern.
    pub(crate) fn new(pattern: &str) -> Result<Self> {
        let p = units(pattern);
        let len = p.len();
        let illegal = |message: &str| {
            LuceneError::illegal_argument(format!("{message} in pattern \"{pattern}\""))
        };
        // `NumberFormat`'s defaults, which an empty pattern leaves in place.
        let mut grouping_used = true;
        let mut format_multiplier = 1;
        let mut use_exponential_notation = false;
        let mut got_negative = false;
        let mut pos_prefix_pattern: Vec<u16> = Vec::new();
        let mut pos_suffix_pattern: Vec<u16> = Vec::new();
        let mut neg_prefix_pattern: Vec<u16> = Vec::new();
        let mut neg_suffix_pattern: Vec<u16> = Vec::new();
        let mut start = 0usize;
        let mut j: i32 = 1;
        while j >= 0 && start < len {
            let mut in_quote = false;
            let mut prefix: Vec<u16> = Vec::new();
            let mut suffix: Vec<u16> = Vec::new();
            let mut decimal_pos: i32 = -1;
            let mut multiplier = 1;
            let (mut digit_left_count, mut zero_digit_count, mut digit_right_count) =
                (0i32, 0i32, 0i32);
            // A Java `byte`.
            let mut grouping_count: i8 = -1;
            let mut phase = 0;
            let mut affix = Affix::Prefix;
            let mut pos: isize = start as isize;
            while (pos as usize) < len {
                let ch = p[pos as usize];
                let target = match affix {
                    Affix::Prefix => &mut prefix,
                    Affix::Suffix => &mut suffix,
                };
                if phase == 0 || phase == 2 {
                    if in_quote {
                        if ch == QUOTE {
                            if (pos as usize) + 1 < len && p[pos as usize + 1] == QUOTE {
                                pos += 1;
                                target.extend_from_slice(&[QUOTE, QUOTE]);
                            } else {
                                in_quote = false;
                            }
                            pos += 1;
                            continue;
                        }
                    } else if ch == PATTERN_DIGIT
                        || ch == PATTERN_ZERO_DIGIT
                        || ch == PATTERN_GROUPING_SEPARATOR
                        || ch == PATTERN_DECIMAL_SEPARATOR
                    {
                        phase = 1;
                        // Reprocess this character.
                        continue;
                    } else if ch == CURRENCY_SIGN {
                        let doubled =
                            (pos as usize) + 1 < len && p[pos as usize + 1] == CURRENCY_SIGN;
                        if doubled {
                            pos += 1;
                        }
                        target.push(QUOTE);
                        target.push(CURRENCY_SIGN);
                        if doubled {
                            target.push(CURRENCY_SIGN);
                        }
                        pos += 1;
                        continue;
                    } else if ch == QUOTE {
                        if (pos as usize) + 1 < len && p[pos as usize + 1] == QUOTE {
                            pos += 1;
                            target.extend_from_slice(&[QUOTE, QUOTE]);
                        } else {
                            in_quote = true;
                        }
                        pos += 1;
                        continue;
                    } else if ch == PATTERN_SEPARATOR {
                        if phase == 0 || j == 0 {
                            return Err(illegal(&format!(
                                "Unquoted special character '{}'",
                                String::from_utf16_lossy(&[ch])
                            )));
                        }
                        start = pos as usize + 1;
                        pos = len as isize;
                        pos += 1;
                        continue;
                    } else if ch == PATTERN_PERCENT || ch == PATTERN_PER_MILLE {
                        if multiplier != 1 {
                            return Err(illegal("Too many percent/per mille characters"));
                        }
                        multiplier = if ch == PATTERN_PERCENT { 100 } else { 1000 };
                        target.push(QUOTE);
                        target.push(ch);
                        pos += 1;
                        continue;
                    } else if ch == PATTERN_MINUS {
                        target.push(QUOTE);
                        target.push(PATTERN_MINUS);
                        pos += 1;
                        continue;
                    }
                    target.push(ch);
                    pos += 1;
                    continue;
                }
                // Phase 1.
                if j == 0 {
                    // The negative subpattern only names the affixes.
                    while (pos as usize) < len {
                        let c = p[pos as usize];
                        // Phase 1 characters, and the exponent (one char).
                        if c == PATTERN_DIGIT
                            || c == PATTERN_ZERO_DIGIT
                            || c == PATTERN_GROUPING_SEPARATOR
                            || c == PATTERN_DECIMAL_SEPARATOR
                            || c == PATTERN_EXPONENT
                        {
                            pos += 1;
                        } else {
                            pos -= 1;
                            phase = 2;
                            affix = Affix::Suffix;
                            break;
                        }
                    }
                    pos += 1;
                    continue;
                }
                if ch == PATTERN_DIGIT {
                    if zero_digit_count > 0 {
                        digit_right_count += 1;
                    } else {
                        digit_left_count += 1;
                    }
                    if grouping_count >= 0 && decimal_pos < 0 {
                        grouping_count = grouping_count.wrapping_add(1);
                    }
                } else if ch == PATTERN_ZERO_DIGIT {
                    if digit_right_count > 0 {
                        return Err(illegal("Unexpected '0'"));
                    }
                    zero_digit_count += 1;
                    if grouping_count >= 0 && decimal_pos < 0 {
                        grouping_count = grouping_count.wrapping_add(1);
                    }
                } else if ch == PATTERN_GROUPING_SEPARATOR {
                    grouping_count = 0;
                } else if ch == PATTERN_DECIMAL_SEPARATOR {
                    if decimal_pos >= 0 {
                        return Err(illegal("Multiple decimal separators"));
                    }
                    decimal_pos = digit_left_count + zero_digit_count + digit_right_count;
                } else if ch == PATTERN_EXPONENT {
                    if use_exponential_notation {
                        return Err(illegal("Multiple exponential symbols"));
                    }
                    use_exponential_notation = true;
                    let mut min_exponent_digits = 0;
                    pos += 1;
                    while (pos as usize) < len && p[pos as usize] == PATTERN_ZERO_DIGIT {
                        min_exponent_digits += 1;
                        pos += 1;
                    }
                    if digit_left_count + zero_digit_count < 1 || min_exponent_digits < 1 {
                        return Err(LuceneError::illegal_argument(format!(
                            "Malformed exponential pattern \"{pattern}\""
                        )));
                    }
                    phase = 2;
                    affix = Affix::Suffix;
                    continue;
                } else {
                    phase = 2;
                    affix = Affix::Suffix;
                    continue;
                }
                pos += 1;
            }
            // "##.###" -> "#0.###", ".###" -> ".0##".
            if zero_digit_count == 0 && digit_left_count > 0 && decimal_pos >= 0 {
                let mut n = decimal_pos;
                if n == 0 {
                    n += 1;
                }
                digit_right_count = digit_left_count - n;
                digit_left_count = n - 1;
                zero_digit_count = 1;
            }
            if (decimal_pos < 0 && digit_right_count > 0)
                || (decimal_pos >= 0
                    && (decimal_pos < digit_left_count
                        || decimal_pos > digit_left_count + zero_digit_count))
                || grouping_count == 0
                || in_quote
            {
                return Err(LuceneError::illegal_argument(format!(
                    "Malformed pattern \"{pattern}\""
                )));
            }
            if j == 1 {
                pos_prefix_pattern = prefix;
                pos_suffix_pattern = suffix;
                neg_prefix_pattern = pos_prefix_pattern.clone();
                neg_suffix_pattern = pos_suffix_pattern.clone();
                grouping_used = grouping_count > 0;
                format_multiplier = multiplier;
            } else {
                neg_prefix_pattern = prefix;
                neg_suffix_pattern = suffix;
                got_negative = true;
            }
            j -= 1;
        }
        if !got_negative
            || (neg_prefix_pattern == pos_prefix_pattern
                && neg_suffix_pattern == pos_suffix_pattern)
        {
            neg_suffix_pattern = pos_suffix_pattern.clone();
            neg_prefix_pattern = [QUOTE, PATTERN_MINUS]
                .iter()
                .chain(&pos_prefix_pattern)
                .copied()
                .collect();
        }
        Ok(Self {
            positive_prefix: expand_affix(&pos_prefix_pattern),
            positive_suffix: expand_affix(&pos_suffix_pattern),
            negative_prefix: expand_affix(&neg_prefix_pattern),
            negative_suffix: expand_affix(&neg_suffix_pattern),
            multiplier: format_multiplier,
            grouping_used,
            parse_big_decimal: false,
        })
    }

    /// `setParseBigDecimal(value)`.
    pub(crate) fn set_parse_big_decimal(&mut self, value: bool) {
        self.parse_big_decimal = value;
    }

    /// `DecimalFormat.parse(String)` (`NumberFormat.parse(String)` over the
    /// lenient `parse(String, ParsePosition)`).
    ///
    /// # Errors
    ///
    /// `ParseException("Unparseable number: \"text\"", errorIndex)` when the
    /// text does not start with a number; in `BigDecimal` mode, the
    /// `NumberFormatException("Exponent overflow.")` and the
    /// `ArithmeticException` (`Underflow`, `Overflow`) `DigitList
    /// .getBigDecimal()` raises for scales outside `int`.
    pub(crate) fn parse(&self, text: &str) -> Result<ParsedNumber> {
        let t = units(text);
        let unparseable = |error_index: usize| {
            ParseException::error(
                format!("Unparseable number: \"{text}\""),
                error_index as i32,
            )
        };
        if region_matches(&t, 0, &NAN) {
            return Ok(ParsedNumber::Double(f64::NAN));
        }
        // `subparse(text, pos, positivePrefix, negativePrefix, digitList,
        // false, status)`.
        let mut got_positive = region_matches(&t, 0, &self.positive_prefix);
        let mut got_negative = region_matches(&t, 0, &self.negative_prefix);
        if got_positive && got_negative {
            if self.positive_prefix.len() > self.negative_prefix.len() {
                got_negative = false;
            } else if self.positive_prefix.len() < self.negative_prefix.len() {
                got_positive = false;
            }
        }
        let position = if got_positive {
            self.positive_prefix.len()
        } else if got_negative {
            self.negative_prefix.len()
        } else {
            return Err(unparseable(0));
        };
        let Some(number) = self.subparse_number(&t, position) else {
            return Err(unparseable(0));
        };
        let position = number.position;
        if got_positive {
            got_positive = region_matches(&t, position, &self.positive_suffix);
        }
        if got_negative {
            got_negative = region_matches(&t, position, &self.negative_suffix);
        }
        if got_positive && got_negative {
            if self.positive_suffix.len() > self.negative_suffix.len() {
                got_negative = false;
            } else if self.positive_suffix.len() < self.negative_suffix.len() {
                got_positive = false;
            }
        }
        if got_positive == got_negative {
            return Err(unparseable(position));
        }
        let index = position
            + if got_positive {
                self.positive_suffix.len()
            } else {
                self.negative_suffix.len()
            };
        if index == 0 {
            return Err(unparseable(position));
        }
        let positive = got_positive;
        if number.infinite {
            return Ok(ParsedNumber::Double(
                if positive == (self.multiplier >= 0) {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                },
            ));
        }
        let mut digits = number.digits;
        let decimal_at = number.decimal_at;
        if self.parse_big_decimal {
            let mut result = JavaBigDecimal::from_digit_list(&digits, decimal_at)?;
            if self.multiplier != 1 {
                let k = if self.multiplier == 100 { 2 } else { 3 };
                result = result.divide_by_power_of_ten(k);
            }
            result.negative = !positive && !result.unscaled.is_empty();
            return Ok(ParsedNumber::BigDecimal(result));
        }
        let mut got_double = true;
        let mut got_long_minimum = false;
        let mut double_result = 0.0f64;
        let mut long_result = 0i64;
        if fits_into_long(&mut digits, decimal_at, positive, false) {
            got_double = false;
            long_result = get_long(&digits, decimal_at);
            if long_result < 0 {
                got_long_minimum = true;
            }
        } else {
            double_result = get_double(&digits, decimal_at);
        }
        let multiplier = i64::from(self.multiplier);
        if multiplier != 1 {
            if got_double {
                double_result /= multiplier as f64;
            } else if long_result % multiplier == 0 {
                long_result /= multiplier;
            } else {
                double_result = long_result as f64 / multiplier as f64;
                got_double = true;
            }
        }
        if !positive && !got_long_minimum {
            double_result = -double_result;
            long_result = long_result.wrapping_neg();
        }
        if multiplier != 1 && got_double {
            long_result = double_result as i64;
            got_double = double_result != long_result as f64
                || (double_result == 0.0 && (1.0 / double_result) < 0.0);
        }
        Ok(if got_double {
            ParsedNumber::Double(double_result)
        } else {
            ParsedNumber::Long(long_result)
        })
    }

    /// The number part of `subparseNumber` (not an exponent): `None` where
    /// Java returns position -1.
    fn subparse_number(&self, t: &[u16], start: usize) -> Option<SubparsedNumber> {
        let mut position = start;
        if region_matches(t, position, &[INFINITY]) {
            return Some(SubparsedNumber {
                position: position + 1,
                digits: Vec::new(),
                decimal_at: 0,
                infinite: true,
            });
        }
        let mut digits: Vec<u8> = Vec::new();
        let mut decimal_at: i32 = 0;
        let mut saw_decimal = false;
        let mut saw_digit = false;
        let mut exponent: i64 = 0;
        let mut digit_count: i32 = 0;
        let mut backup: Option<usize> = None;
        while position < t.len() {
            let ch = t[position];
            let digit = unit_digit(ch);
            if digit == 0 {
                backup = None;
                saw_digit = true;
                if digits.is_empty() {
                    if !saw_decimal {
                        position += 1;
                        continue;
                    }
                    decimal_at = decimal_at.wrapping_sub(1);
                } else {
                    digit_count += 1;
                    digits.push(b'0');
                }
            } else if (1..=9).contains(&digit) {
                saw_digit = true;
                digit_count += 1;
                digits.push(b'0' + digit as u8);
                backup = None;
            } else if ch == DECIMAL_SEPARATOR {
                if saw_decimal {
                    break;
                }
                decimal_at = digit_count;
                saw_decimal = true;
            } else if ch == GROUPING_SEPARATOR && self.grouping_used {
                if saw_decimal {
                    break;
                }
                backup = Some(position);
            } else if ch == EXPONENT_SEPARATOR {
                if let Some((value, end)) = subparse_exponent(t, position + 1) {
                    exponent = value;
                    position = end;
                }
                break;
            } else {
                break;
            }
            position += 1;
        }
        if let Some(backup) = backup {
            position = backup;
        }
        if !saw_decimal {
            decimal_at = digit_count;
        }
        decimal_at = shift_decimal_at(decimal_at, exponent);
        if !saw_digit && digit_count == 0 {
            return None;
        }
        Some(SubparsedNumber {
            position,
            digits,
            decimal_at,
            infinite: false,
        })
    }
}

/// What `subparseNumber` leaves in the `DigitList` and the parse position.
struct SubparsedNumber {
    position: usize,
    digits: Vec<u8>,
    decimal_at: i32,
    infinite: bool,
}

/// `expandAffix(String, StringBuilder)` with the `Locale.ROOT` symbols.
fn expand_affix(pattern: &[u16]) -> Vec<u16> {
    let mut buffer = Vec::new();
    let mut i = 0;
    while i < pattern.len() {
        let mut c = pattern[i];
        i += 1;
        if c == QUOTE {
            // `applyPattern` never ends an affix pattern with a lone quote.
            debug_assert!(i < pattern.len());
            c = pattern[i];
            i += 1;
            match c {
                CURRENCY_SIGN => {
                    if i < pattern.len() && pattern[i] == CURRENCY_SIGN {
                        i += 1;
                        buffer.extend_from_slice(&units("XXX"));
                    } else {
                        buffer.push(CURRENCY_SIGN);
                    }
                    continue;
                }
                PATTERN_PERCENT | PATTERN_PER_MILLE | PATTERN_MINUS => {
                    buffer.push(c);
                    continue;
                }
                _ => {}
            }
        }
        buffer.push(c);
    }
    buffer
}

/// The exponent `subparse(text, pos, "", minusSignText, exponentDigits,
/// true, expStat)`: the value (clamped to `long`) and the end position, or
/// `None` when no digit follows.
fn subparse_exponent(t: &[u16], start: usize) -> Option<(i64, usize)> {
    let mut p = start;
    let negative = t.get(p) == Some(&MINUS_SIGN);
    if negative {
        p += 1;
    }
    let mut digits: Vec<u8> = Vec::new();
    let mut saw_digit = false;
    while p < t.len() {
        let d = unit_digit(t[p]);
        if d == 0 {
            saw_digit = true;
            if !digits.is_empty() {
                digits.push(b'0');
            }
        } else if (1..=9).contains(&d) {
            saw_digit = true;
            digits.push(b'0' + d as u8);
        } else {
            break;
        }
        p += 1;
    }
    if !saw_digit {
        return None;
    }
    let decimal_at = digits.len() as i32;
    let value = if fits_into_long(&mut digits, decimal_at, !negative, true) {
        let v = get_long(&digits, decimal_at);
        if negative {
            v.wrapping_neg()
        } else {
            v
        }
    } else if negative {
        i64::MIN
    } else {
        i64::MAX
    };
    Some((value, p))
}

/// The value of one UTF-16 unit as a decimal digit in `subparseNumber`:
/// `ch - '0'`, else `Character.digit(ch, 10)`, else -1.
fn unit_digit(ch: u16) -> i32 {
    let digit = i32::from(ch) - i32::from(b'0');
    if (0..=9).contains(&digit) {
        return digit;
    }
    char::from_u32(u32::from(ch))
        .and_then(|c| rucene::util::java_character_digit(c, 10))
        .map_or(-1, |d| d as i32)
}

/// `DecimalFormat.shiftDecimalAt(decimalAt, exponent)`.
fn shift_decimal_at(decimal_at: i32, exponent: i64) -> i32 {
    match i64::from(decimal_at).checked_add(exponent) {
        None => {
            if exponent > 0 && decimal_at > 0 {
                i32::MAX
            } else {
                i32::MIN
            }
        }
        Some(e) => i32::try_from(e).unwrap_or(if e > 0 { i32::MAX } else { i32::MIN }),
    }
}

/// `DigitList.LONG_MIN_REP`.
const LONG_MIN_REP: &[u8] = b"9223372036854775808";

/// `DigitList.fitsIntoLong(isPositive, ignoreNegativeZero)`, which also trims
/// the trailing zeros of `digits`.
fn fits_into_long(
    digits: &mut Vec<u8>,
    decimal_at: i32,
    is_positive: bool,
    ignore_negative_zero: bool,
) -> bool {
    while digits.last() == Some(&b'0') {
        digits.pop();
    }
    let count = digits.len() as i64;
    if count == 0 {
        return is_positive || ignore_negative_zero;
    }
    let decimal_at = i64::from(decimal_at);
    if decimal_at < count || decimal_at > 19 {
        return false;
    }
    if decimal_at < 19 {
        return true;
    }
    for (dig, max) in digits.iter().zip(LONG_MIN_REP) {
        if dig > max {
            return false;
        }
        if dig < max {
            return true;
        }
    }
    if count < decimal_at {
        return true;
    }
    !is_positive
}

/// `DigitList.getLong()` after `fitsIntoLong` returned `true`.
fn get_long(digits: &[u8], decimal_at: i32) -> i64 {
    if digits.is_empty() {
        return 0;
    }
    if decimal_at as usize == digits.len() && digits == LONG_MIN_REP {
        return i64::MIN;
    }
    let mut v: i64 = 0;
    for d in digits {
        v = v * 10 + i64::from(d - b'0');
    }
    for _ in digits.len()..decimal_at.max(0) as usize {
        v *= 10;
    }
    v
}

/// `DigitList.getDouble()`: `Double.parseDouble("." + digits + "E" +
/// decimalAt)`.
fn get_double(digits: &[u8], decimal_at: i32) -> f64 {
    if digits.is_empty() {
        return 0.0;
    }
    let s = format!(
        ".{}E{decimal_at}",
        std::str::from_utf8(digits).expect("INVARIANT: ASCII digits")
    );
    s.parse().expect("INVARIANT: a well-formed decimal")
}

/// The `Number` that `DecimalFormat.parse` returns.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ParsedNumber {
    /// A `java.lang.Long`.
    Long(i64),
    /// A `java.lang.Double`.
    Double(f64),
    /// A `java.math.BigDecimal` (only with `setParseBigDecimal(true)`).
    BigDecimal(JavaBigDecimal),
}

impl ParsedNumber {
    /// `Number.longValue()` (`BigDecimal.longValue()` drops the fraction).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the `BigInteger` a `BigDecimal` builds.
    pub(crate) fn long_value(&self) -> Result<i64> {
        match self {
            ParsedNumber::Long(v) => Ok(*v),
            ParsedNumber::Double(d) => Ok(*d as i64),
            ParsedNumber::BigDecimal(bd) => bd.to_big_integer_low_bits(false),
        }
    }

    /// `Number.intValue()`: a `Long` is narrowed (wrapping), a `Double` is
    /// cast (saturating), a `BigDecimal` keeps the low 32 bits.
    ///
    /// # Errors
    ///
    /// As [`long_value`](Self::long_value).
    pub(crate) fn int_value(&self) -> Result<i32> {
        match self {
            ParsedNumber::Double(d) => Ok(*d as i32),
            other => Ok(other.long_value()? as i32),
        }
    }

    /// `BigInteger.valueOf(origin).add(((BigDecimal) number).toBigIntegerExact()).longValue()`
    /// (`SimpleTextDocValuesReader.java:303-311`).
    ///
    /// # Errors
    ///
    /// `ClassCastException` for a `Double` (`NaN`, `∞`);
    /// `ArithmeticException` for a non-zero fraction (`Rounding necessary`)
    /// or a value outside `BigInteger`'s range.
    pub(crate) fn add_big_integer_exact(&self, origin: i64) -> Result<i64> {
        match self {
            ParsedNumber::BigDecimal(bd) => {
                Ok(origin.wrapping_add(bd.to_big_integer_low_bits(true)?))
            }
            _ => Err(LuceneError::unchecked(
                "java.lang.ClassCastException",
                "class java.lang.Double cannot be cast to class java.math.BigDecimal \
                 (java.lang.Double and java.math.BigDecimal are in module java.base of loader \
                 'bootstrap')",
            )),
        }
    }
}

/// A `java.math.BigDecimal` as `DecimalFormat` builds it:
/// `unscaled × 10^-scale`, negated when `negative`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JavaBigDecimal {
    /// The sign (never set for zero).
    pub(crate) negative: bool,
    /// The decimal digits of the unscaled value, without leading zeros;
    /// empty for zero.
    pub(crate) unscaled: Vec<u8>,
    /// The scale.
    pub(crate) scale: i32,
}

/// `BigInteger.MAX_MAG_LENGTH * 32`: the smallest bit length outside
/// `BigInteger`'s range.
const BIG_INTEGER_RANGE_BITS: i64 = 1 << 31;

/// The smallest `n` for which `BigInteger.TEN.pow(n)` overflows: the final
/// `shiftLeft(n)` of `5^n` builds a magnitude of `2^31` bits from
/// `n = 646456993` (`floor(n · log2 10) + 1`), before the earlier
/// `3n >= Integer.MAX_VALUE` check (`n >= 715827883`) can trip.
const TEN_POW_OVERFLOW: i64 = 646_456_993;

/// `BigDecimal.BIG_TEN_POWERS_TABLE_MAX`: powers of ten below it come from a
/// table, not `BigInteger.pow`.
const BIG_TEN_POWERS_TABLE_MAX: i64 = 320;

// The pow overflow threshold lies past both power-of-ten tables, so a
// check against it alone stands for "not from a table, and overflowing".
const _: () = assert!(
    TEN_POW_OVERFLOW >= BIG_TEN_POWERS_TABLE_MAX && BIG_TEN_POWERS_TABLE_MAX >= LONG_TEN_POWERS
);

/// `BigDecimal.LONG_TEN_POWERS_TABLE.length`.
const LONG_TEN_POWERS: i64 = 19;

/// `BigInteger.KARATSUBA_THRESHOLD` and `TOOM_COOK_THRESHOLD`, in ints.
const KARATSUBA_THRESHOLD: i64 = 80;
const TOOM_COOK_THRESHOLD: i64 = 240;

/// `log2(5)` in Q90 fixed point (`0x949a784bcd1b8afe492bf6f`).
const LOG2_5_Q90: u128 = 0x949a784bcd1b8afe492bf6f;

fn big_integer_overflow() -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArithmeticException",
        "BigInteger would overflow supported range",
    )
}

/// `checkScale(long)` of a non-zero value.
fn check_scale(val: i64) -> Result<i32> {
    i32::try_from(val).map_err(|_| {
        LuceneError::unchecked(
            "java.lang.ArithmeticException",
            if val > 0 { "Underflow" } else { "Overflow" },
        )
    })
}

impl JavaBigDecimal {
    /// `DigitList.getBigDecimal()` over the digits `subparseNumber` kept.
    ///
    /// # Errors
    ///
    /// `NumberFormatException("Exponent overflow.")` for the zero built
    /// from `"0E" + decimalAt` when `decimalAt` is `Integer.MIN_VALUE`;
    /// `ArithmeticException` from `checkScale` in
    /// `scaleByPowerOfTen(decimalAt - count)`, whose `int` subtraction
    /// wraps.
    fn from_digit_list(digits: &[u8], decimal_at: i32) -> Result<Self> {
        if digits.is_empty() {
            if decimal_at == 0 {
                return Ok(Self::zero(0));
            }
            // `new BigDecimal("0E" + decimalAt)`: scale `-decimalAt`.
            let scale = -i64::from(decimal_at);
            return match i32::try_from(scale) {
                Ok(scale) => Ok(Self::zero(scale)),
                Err(_) => Err(LuceneError::illegal_argument(
                    "Exponent overflow.".to_string(),
                )),
            };
        }
        let count = digits.len() as i32;
        let scale = if decimal_at == count {
            0
        } else {
            check_scale(-i64::from(decimal_at.wrapping_sub(count)))?
        };
        Ok(Self {
            negative: false,
            unscaled: digits.to_vec(),
            scale,
        })
    }

    fn zero(scale: i32) -> Self {
        Self {
            negative: false,
            unscaled: Vec::new(),
            scale,
        }
    }

    /// `divide(BigDecimal.valueOf(10^k))` as `DecimalFormat.parse` calls it:
    /// the exact quotient with trailing zeros stripped down to the dividend's
    /// scale, or — when the exact division's `checkScaleNonZero(preferred +
    /// yscale - xscale + mcp)` overflows and the `ArithmeticException` is
    /// caught — `divide(divisor, roundingMode)`: the quotient rounded
    /// `HALF_EVEN` at the dividend's scale.
    fn divide_by_power_of_ten(self, k: usize) -> Self {
        if self.unscaled.is_empty() {
            return self;
        }
        // `scl = preferredScale + yscale - xscale + mcp`, where `yscale`
        // (the divisor's precision, 3 or 4) drops by one when the dividend's
        // significand is above 1, and `mcp = xscale + ceil(10 * yscale / 3)`.
        let divisor_precision: i64 = k as i64 + 1;
        let extra = if k == 2 { 10 } else { 14 };
        let significand_above_one =
            self.unscaled[0] != b'1' || self.unscaled[1..].iter().any(|d| *d != b'0');
        let scl =
            i64::from(self.scale) + divisor_precision - i64::from(significand_above_one) + extra;
        if scl > i64::from(i32::MAX) {
            let split = self.unscaled.len().saturating_sub(k);
            let (quotient, remainder) = self.unscaled.split_at(split);
            let mut quotient = quotient.to_vec();
            let mut padded = vec![b'0'; k - remainder.len()];
            padded.extend_from_slice(remainder);
            let half: Vec<u8> = std::iter::once(b'5')
                .chain(std::iter::repeat_n(b'0', k - 1))
                .collect();
            let round_up = match padded.cmp(&half) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                std::cmp::Ordering::Equal => quotient.last().is_some_and(|d| (d - b'0') % 2 == 1),
            };
            if round_up {
                increment(&mut quotient);
            }
            while quotient.first() == Some(&b'0') {
                quotient.remove(0);
            }
            return Self {
                negative: self.negative,
                unscaled: quotient,
                scale: self.scale,
            };
        }
        let trailing = self
            .unscaled
            .iter()
            .rev()
            .take_while(|d| **d == b'0')
            .count();
        let stripped = trailing.min(k);
        let mut unscaled = self.unscaled;
        unscaled.truncate(unscaled.len() - stripped);
        Self {
            negative: self.negative,
            unscaled,
            scale: (i64::from(self.scale) + k as i64 - stripped as i64) as i32,
        }
    }

    /// The low 64 bits of `toBigIntegerExact()` (`exact`) or `toBigInteger()`
    /// — `setScale(0, UNNECESSARY | DOWN)` — with the `ArithmeticException`s
    /// of `BigDecimal.setScale`, `bigTenToThe` (`BigInteger.TEN.pow`) and
    /// `BigInteger.multiply` on the way.
    fn to_big_integer_low_bits(&self, exact: bool) -> Result<i64> {
        if self.unscaled.is_empty() {
            return Ok(0);
        }
        let count = self.unscaled.len();
        let scale = i64::from(self.scale);
        let magnitude = if scale <= 0 {
            let raise = -scale;
            self.check_raise(raise)?;
            let mut v = low_bits(&self.unscaled);
            if raise >= 64 {
                v = 0;
            } else {
                for _ in 0..raise {
                    v = v.wrapping_mul(10);
                }
            }
            v
        } else {
            let drop = scale;
            // `bigTenToThe(drop)` (`drop` beyond both tables) overflows in
            // `BigInteger.TEN.pow`.
            if drop >= TEN_POW_OVERFLOW {
                return Err(big_integer_overflow());
            }
            let trailing = self
                .unscaled
                .iter()
                .rev()
                .take_while(|d| **d == b'0')
                .count() as i64;
            if exact && trailing < drop {
                return Err(LuceneError::unchecked(
                    "java.lang.ArithmeticException",
                    "Rounding necessary",
                ));
            }
            let keep = (count as i64 - drop).max(0) as usize;
            low_bits(&self.unscaled[..keep])
        };
        let v = magnitude as i64;
        Ok(if self.negative { v.wrapping_neg() } else { v })
    }

    /// The overflow checks of `setScale(0, ...)` raising the scale by
    /// `raise`: `bigTenToThe(raise)` and the product with the unscaled
    /// value.
    fn check_raise(&self, raise: i64) -> Result<()> {
        if raise == 0 {
            return Ok(());
        }
        let compact = fits_in_long(&self.unscaled);
        if compact {
            let v = low_bits(&self.unscaled) as i64;
            // `longMultiplyPowerTen`.
            if raise < LONG_TEN_POWERS && v <= i64::MAX / 10i64.pow(raise as u32) {
                return Ok(());
            }
        }
        // `bigTenToThe(raise)`: past `BIG_TEN_POWERS_TABLE_MAX`,
        // `BigInteger.TEN.pow(raise)`, which overflows from
        // `TEN_POW_OVERFLOW` on.
        if raise >= TEN_POW_OVERFLOW {
            return Err(big_integer_overflow());
        }
        let ten_bits = ten_pow_bit_length(raise);
        let value_bits = Digits::new(&self.unscaled);
        // A non-compact value times `LONG_TEN_POWERS_TABLE[raise]` is
        // `multiply(long)`; above, `multiply(bigTenToThe(raise))`.
        if !compact && raise >= LONG_TEN_POWERS {
            // `BigInteger.multiply`: the Toom-Cook branch checks the sum of
            // the operands' bit lengths before multiplying.
            let x_len = (value_bits.bit_length + 31) / 32;
            let y_len = (ten_bits + 31) / 32;
            let karatsuba_or_less = x_len < KARATSUBA_THRESHOLD
                || y_len < KARATSUBA_THRESHOLD
                || (x_len < TOOM_COOK_THRESHOLD && y_len < TOOM_COOK_THRESHOLD);
            if !karatsuba_or_less && value_bits.bit_length + ten_bits > BIG_INTEGER_RANGE_BITS {
                return Err(big_integer_overflow());
            }
        }
        // The product's magnitude: `checkRange()` in its constructor.
        if product_bit_length(&value_bits, raise) >= BIG_INTEGER_RANGE_BITS {
            return Err(big_integer_overflow());
        }
        Ok(())
    }
}

/// Adds one to an ASCII digit string.
fn increment(digits: &mut Vec<u8>) {
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d += 1;
            return;
        }
    }
    digits.insert(0, b'1');
}

/// The value of an ASCII digit string modulo 2^64.
fn low_bits(digits: &[u8]) -> u64 {
    digits.iter().fold(0u64, |v, d| {
        v.wrapping_mul(10).wrapping_add(u64::from(d - b'0'))
    })
}

/// Whether an ASCII digit string fits a non-negative `long` (`compactValFor`).
fn fits_in_long(digits: &[u8]) -> bool {
    if digits.len() < 19 {
        return true;
    }
    if digits.len() > 19 {
        return false;
    }
    digits <= b"9223372036854775807".as_slice()
}

/// `bitLength(10^n) = n + floor(n · log2 5) + 1`.
fn ten_pow_bit_length(n: i64) -> i64 {
    n + ((n as u128 * LOG2_5_Q90) >> 90) as i64 + 1
}

/// A digit string's binary magnitude: its exact bit length and its top 64
/// bits (normalised, bit 63 set).
struct Digits {
    bit_length: i64,
    top64: u64,
}

impl Digits {
    fn new(digits: &[u8]) -> Self {
        // Little-endian base-2^32 limbs, built by multiply-add.
        let mut limbs: Vec<u32> = Vec::new();
        for chunk in digits.chunks(9) {
            let mut mul: u64 = 1;
            let mut add: u64 = 0;
            for d in chunk {
                mul *= 10;
                add = add * 10 + u64::from(d - b'0');
            }
            let mut carry = add;
            for limb in limbs.iter_mut() {
                let t = u64::from(*limb) * mul + carry;
                *limb = t as u32;
                carry = t >> 32;
            }
            while carry != 0 {
                limbs.push(carry as u32);
                carry >>= 32;
            }
        }
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        let n = limbs.len();
        if n == 0 {
            return Self {
                bit_length: 0,
                top64: 0,
            };
        }
        let top = limbs[n - 1];
        let bit_length = (n as i64 - 1) * 32 + i64::from(32 - top.leading_zeros());
        // The top 96 bits, then normalised to 64.
        let mut window: u128 = 0;
        for i in 0..3 {
            window <<= 32;
            if n > i {
                window |= u128::from(limbs[n - 1 - i]);
            }
        }
        // The top bit sits at `63 + b`; move it to bit 127, keep the top half.
        let shift = 64 - (32 - top.leading_zeros());
        let top64 = ((window << shift) >> 64) as u64;
        Self { bit_length, top64 }
    }
}

/// `log2(m / 2^63)` in Q64 for `m` with bit 63 set: the binary digits of the
/// logarithm by repeated squaring.
fn log2_fraction_q64(m: u64) -> u64 {
    let mut x = u128::from(m);
    let mut fraction: u64 = 0;
    for i in 1..=62 {
        x = (x * x) >> 63;
        if x >= 1 << 64 {
            fraction |= 1 << (64 - i);
            x >>= 1;
        }
    }
    fraction
}

/// `bitLength(v · 10^raise) = floor(log2 v + raise · log2 10) + 1`, from the
/// exact bit length of `v`, a 62-bit logarithm of its mantissa and a 90-bit
/// `log2 5`. The product is never a power of two (it has the factor
/// `5^raise`), so the floor is exact away from integers closer than 2^-55.
fn product_bit_length(v: &Digits, raise: i64) -> i64 {
    let log_v_fraction = u128::from(log2_fraction_q64(v.top64));
    let r5 = raise as u128 * LOG2_5_Q90;
    let r5_int = (r5 >> 90) as i64;
    let r5_fraction = (r5 >> 26) & u128::from(u64::MAX);
    let carry = ((log_v_fraction + r5_fraction) >> 64) as i64;
    (v.bit_length - 1) + raise + r5_int + carry + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use rucene::error::LuceneErrorKind;

    /// Decodes the `\uXXXX` escapes of the probe output.
    fn unescape(s: &str) -> String {
        let mut units = Vec::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' && chars.peek() == Some(&'u') {
                chars.next();
                let hex: String = chars.by_ref().take(4).collect();
                units.push(u16::from_str_radix(&hex, 16).unwrap());
            } else {
                let mut buf = [0u16; 2];
                units.extend_from_slice(c.encode_utf16(&mut buf));
            }
        }
        String::from_utf16(&units).unwrap()
    }

    fn message(error: &LuceneError) -> String {
        if let Some(pe) = ParseException::of(error) {
            return format!("PE {}@{}", pe.get_message(), pe.get_error_offset());
        }
        match &error.kind {
            LuceneErrorKind::IllegalArgument { message, .. } => format!("IAE {message}"),
            LuceneErrorKind::Unchecked { class, message, .. } => {
                let m = message.clone().unwrap_or_default();
                match *class {
                    "java.lang.ArithmeticException" => format!("AE {m}"),
                    "java.lang.ClassCastException" => "CCE".to_string(),
                    other => format!("{other} {m}"),
                }
            }
            other => format!("{other:?}"),
        }
    }

    fn plain(format: &DecimalFormat, input: &str) -> String {
        match format.parse(input) {
            Ok(n @ ParsedNumber::Long(v)) => format!(
                "Long {v} {} {}",
                n.long_value().unwrap(),
                n.int_value().unwrap()
            ),
            Ok(n @ ParsedNumber::Double(d)) => format!(
                "Double {} {} {}",
                rucene::util::java_fmt::double_to_string(d),
                n.long_value().unwrap(),
                n.int_value().unwrap()
            ),
            Ok(other) => panic!("{other:?}"),
            Err(e) => message(&e),
        }
    }

    fn big(format: &DecimalFormat, input: &str) -> String {
        match format.parse(input) {
            Ok(ParsedNumber::BigDecimal(bd)) => {
                let unscaled = if bd.unscaled.is_empty() {
                    "0".to_string()
                } else {
                    format!(
                        "{}{}",
                        if bd.negative { "-" } else { "" },
                        String::from_utf8(bd.unscaled.clone()).unwrap()
                    )
                };
                let exact = match ParsedNumber::BigDecimal(bd.clone()).add_big_integer_exact(5) {
                    Ok(v) => v.to_string(),
                    Err(e) => message(&e),
                };
                format!("BD {unscaled} {} {exact}", bd.scale)
            }
            Ok(_) => "CCE".to_string(),
            Err(e) => message(&e),
        }
    }

    /// Regression (rmp #526): every row of the JDK 25 probe grid — 41
    /// patterns × 35 inputs, `parse` with and without
    /// `setParseBigDecimal(true)`, and the constructor's
    /// `IllegalArgumentException` for the malformed patterns.
    #[test]
    fn parse_matches_the_jdk_grid() {
        let grid = include_str!("testdata/decimal_format_grid.tsv");
        let mut rows = 0;
        for line in grid.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let pattern = unescape(fields[0]);
            let format = DecimalFormat::new(&pattern);
            if fields[2].starts_with("IAE ") {
                let error = format.expect_err(&pattern);
                assert_eq!(message(&error), unescape(fields[2]), "pattern {pattern:?}");
                rows += 1;
                continue;
            }
            let format = format.unwrap_or_else(|e| panic!("{pattern:?}: {e}"));
            let input = unescape(fields[1]);
            assert_eq!(
                plain(&format, &input),
                unescape(fields[2]),
                "pattern {pattern:?} input {input:?}"
            );
            let mut big_format = format.clone();
            big_format.set_parse_big_decimal(true);
            assert_eq!(
                big(&big_format, &input),
                unescape(fields[3]),
                "pattern {pattern:?} input {input:?} (BigDecimal)"
            );
            rows += 1;
        }
        assert_eq!(rows, 1027);
    }

    /// Regression (rmp #526): scales at the edges of `int` and powers of ten
    /// past `BigInteger`'s range, as the JDK 25 oracle printed them for the
    /// pattern `000` with `setParseBigDecimal(true)` and
    /// `BigInteger.valueOf(5).add(bd.toBigIntegerExact())`.
    #[test]
    fn big_integer_range_matches_the_jdk() {
        let mut format = DecimalFormat::new("000").unwrap();
        format.set_parse_big_decimal(true);
        let overflow = "AE BigInteger would overflow supported range";
        let cases = [
            (
                "1E-99999999999999999999",
                "BD 1 -2147483647 ".to_string() + overflow,
            ),
            (
                "0E-99999999999999999999",
                "IAE Exponent overflow.".to_string(),
            ),
            ("0E-9999999999", "IAE Exponent overflow.".to_string()),
            ("1E-2147483647", "BD 1 2147483647 ".to_string() + overflow),
            ("1E-2147483648", "AE Underflow".to_string()),
            ("1E-2147483649", "BD 1 -2147483647 ".to_string() + overflow),
            ("12E2147483647", "BD 12 -2147483645 ".to_string() + overflow),
            ("1E715827883", "BD 1 -715827883 ".to_string() + overflow),
            ("5E-2147483646", "BD 5 2147483646 ".to_string() + overflow),
            ("10E-2147483647", "BD 10 2147483647 ".to_string() + overflow),
        ];
        for (input, expected) in cases {
            assert_eq!(big(&format, input), expected, "input {input}");
        }
        // The `0%`/`0‰` divisions that overflow the exact quotient's scale
        // fall back to `HALF_EVEN` at the dividend's scale.
        let mut percent = DecimalFormat::new("0%").unwrap();
        percent.set_parse_big_decimal(true);
        assert_eq!(big(&percent, "1E-2147483646%"), "BD 0 2147483646 5");
        assert_eq!(big(&percent, "0E-5%"), "BD 0 5 5");
        assert_eq!(big(&percent, "100%"), "BD 1 0 6");
        let mut per_mille = DecimalFormat::new("0\u{2030}").unwrap();
        per_mille.set_parse_big_decimal(true);
        assert_eq!(
            big(&per_mille, "1200\u{2030}"),
            "BD 12 1 AE Rounding necessary"
        );
    }

    /// The threshold constants: `bitLength(10^n)` first reaches `2^31` at
    /// `n = 646456993` (`floor(n · log2 10) + 1`: `2147483644` for
    /// `n = 646456992`, `2147483648` for `n = 646456993`), and the mantissa
    /// logarithm is exact for powers of two.
    #[test]
    fn power_of_ten_bit_lengths() {
        assert_eq!(ten_pow_bit_length(646_456_992), 2_147_483_644);
        assert_eq!(ten_pow_bit_length(646_456_993), 1 << 31);
        assert_eq!(ten_pow_bit_length(1), 4);
        assert_eq!(ten_pow_bit_length(19), 64);
        let one = Digits::new(b"1");
        assert_eq!((one.bit_length, one.top64), (1, 1 << 63));
        assert_eq!(product_bit_length(&one, 646_456_992), 2_147_483_644);
        let big = Digits::new(b"18446744073709551616");
        assert_eq!(big.bit_length, 65);
        assert_eq!(product_bit_length(&Digits::new(b"8"), 3), 13);
    }
}
