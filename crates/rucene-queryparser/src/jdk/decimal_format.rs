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

//! `java.text.DecimalFormat`, `java.text.DecimalFormatSymbols` and
//! `java.text.DigitList`, as `NumberFormat.getNumberInstance`,
//! `getIntegerInstance` and `getInstance` build them and as
//! `SimpleDateFormat` uses its integer format.
//!
//! **Divergence.** An emulation of JDK 25, not a port of a Lucene file,
//! written from OpenJDK 25's `DecimalFormat` (`applyPattern`,
//! `expandAffix(es)`, `format(double)`, `format(long)`, the fast path
//! `fastFormat`, `subformat`, `subformatNumber`, `parse`, `subparse`,
//! `subparseNumber`, the digit-count setters), `DigitList` and
//! `NumberFormat` (read as specification; no JDK code is reproduced), over
//! the symbols of [`locale_data`](super::locale_data). What is not
//! reproduced: `BigDecimal` and `BigInteger` values (`setParseBigDecimal`,
//! formatting a `BigDecimal` or a `BigInteger` other than the `Long.MIN_VALUE`
//! `format(long)` turns into one), the currency symbols (a pattern with
//! `¤` formats them as the empty string), and `toPattern`.

use rucene::error::{LuceneError, Result};

use super::floating_decimal::binary_to_ascii;
use super::format::{FieldDelegate, FieldPosition, FormatField, JavaNumber, ParsePosition};
use super::locale_data::NumberRecord;

/// `NumberFormat.INTEGER_FIELD`.
pub const INTEGER_FIELD: i32 = 0;
/// `NumberFormat.FRACTION_FIELD`.
pub const FRACTION_FIELD: i32 = 1;

const PATTERN_ZERO_DIGIT: u16 = b'0' as u16;
const PATTERN_GROUPING_SEPARATOR: u16 = b',' as u16;
const PATTERN_DECIMAL_SEPARATOR: u16 = b'.' as u16;
const PATTERN_PER_MILLE: u16 = 0x2030;
const PATTERN_PERCENT: u16 = b'%' as u16;
const PATTERN_DIGIT: u16 = b'#' as u16;
const PATTERN_SEPARATOR: u16 = b';' as u16;
const PATTERN_EXPONENT: &str = "E";
const PATTERN_MINUS: u16 = b'-' as u16;
const CURRENCY_SIGN: u16 = 0x00A4;
const QUOTE: u16 = b'\'' as u16;

const DOUBLE_INTEGER_DIGITS: i32 = 309;
const DOUBLE_FRACTION_DIGITS: i32 = 340;
const MAXIMUM_INTEGER_DIGITS: i32 = i32::MAX;
const MAXIMUM_FRACTION_DIGITS: i32 = i32::MAX;

/// `java.math.RoundingMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode {
    /// `UP`.
    Up,
    /// `DOWN`.
    Down,
    /// `CEILING`.
    Ceiling,
    /// `FLOOR`.
    Floor,
    /// `HALF_UP`.
    HalfUp,
    /// `HALF_DOWN`.
    HalfDown,
    /// `HALF_EVEN`.
    HalfEven,
    /// `UNNECESSARY`.
    Unnecessary,
}

/// `java.text.DecimalFormatSymbols`, without the currency symbols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalFormatSymbols {
    zero_digit: u16,
    grouping_separator: u16,
    decimal_separator: u16,
    per_mill: u16,
    percent: u16,
    digit: u16,
    pattern_separator: u16,
    minus_sign: u16,
    monetary_separator: u16,
    monetary_grouping_separator: u16,
    exponential: u16,
    infinity: Vec<u16>,
    nan: Vec<u16>,
    exponent_separator: Vec<u16>,
    per_mill_text: Vec<u16>,
    percent_text: Vec<u16>,
    minus_sign_text: Vec<u16>,
}

impl DecimalFormatSymbols {
    /// The symbols of a JDK 25 number record.
    pub fn of(record: &NumberRecord) -> Self {
        let u = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        Self {
            zero_digit: record.zero_digit,
            grouping_separator: record.grouping_separator,
            decimal_separator: record.decimal_separator,
            per_mill: record.per_mill,
            percent: record.percent,
            digit: record.digit,
            pattern_separator: record.pattern_separator,
            minus_sign: record.minus_sign,
            monetary_separator: record.monetary_decimal_separator,
            monetary_grouping_separator: record.monetary_grouping_separator,
            exponential: record.exponential,
            infinity: u(&record.infinity),
            nan: u(&record.nan),
            exponent_separator: u(&record.exponent_separator),
            per_mill_text: u(&record.per_mill_text),
            percent_text: u(&record.percent_text),
            minus_sign_text: u(&record.minus_sign_text),
        }
    }

    /// `getZeroDigit()`.
    pub fn get_zero_digit(&self) -> u16 {
        self.zero_digit
    }

    /// `getGroupingSeparator()`.
    pub fn get_grouping_separator(&self) -> u16 {
        self.grouping_separator
    }

    /// `getDecimalSeparator()`.
    pub fn get_decimal_separator(&self) -> u16 {
        self.decimal_separator
    }

    /// `getMinusSign()`.
    pub fn get_minus_sign(&self) -> u16 {
        self.minus_sign
    }

    /// `getMinusSignText()`.
    pub fn get_minus_sign_text(&self) -> &[u16] {
        &self.minus_sign_text
    }

    /// `getNaN()`.
    pub fn get_nan(&self) -> &[u16] {
        &self.nan
    }

    /// `getInfinity()`.
    pub fn get_infinity(&self) -> &[u16] {
        &self.infinity
    }

    /// `getExponentSeparator()`.
    pub fn get_exponent_separator(&self) -> &[u16] {
        &self.exponent_separator
    }
}

/// `java.text.DigitList`.
#[derive(Debug, Clone, Default)]
pub(crate) struct DigitList {
    decimal_at: i32,
    /// `digits[0..count]`, as the ASCII digits `'0'..='9'`.
    digits: Vec<u8>,
    is_negative: bool,
}

/// `DigitList.LONG_MIN_REP`.
const LONG_MIN_REP: &[u8; 19] = b"9223372036854775808";

impl DigitList {
    const MAX_COUNT: i32 = 19;

    fn count(&self) -> usize {
        self.digits.len()
    }

    fn is_zero(&self) -> bool {
        !self.non_zero_after_index(0)
    }

    fn non_zero_after_index(&self, index: usize) -> bool {
        self.digits.iter().skip(index).any(|&d| d != b'0')
    }

    fn clear(&mut self) {
        self.decimal_at = 0;
        self.digits.clear();
    }

    fn append(&mut self, digit: u8) {
        self.digits.push(digit);
    }

    /// `getDouble()`.
    fn get_double(&self) -> f64 {
        if self.digits.is_empty() {
            return 0.0;
        }
        let mantissa: String = self.digits.iter().map(|&d| char::from(d)).collect();
        format!("0.{mantissa}e{}", self.decimal_at)
            .parse()
            .expect("INVARIANT: a decimal literal parses")
    }

    /// `getLong()`.
    fn get_long(&self) -> i64 {
        if self.digits.is_empty() {
            return 0;
        }
        if self.is_long_min_value() {
            return i64::MIN;
        }
        let mut value: i64 = 0;
        for &d in &self.digits {
            value = value * 10 + i64::from(d - b'0');
        }
        for _ in self.digits.len() as i32..self.decimal_at {
            value *= 10;
        }
        value
    }

    /// `isLongMIN_VALUE()`.
    fn is_long_min_value(&self) -> bool {
        if self.decimal_at != self.digits.len() as i32
            || self.digits.len() != Self::MAX_COUNT as usize
        {
            return false;
        }
        self.digits.as_slice() == LONG_MIN_REP
    }

    /// `fitsIntoLong(isPositive, ignoreNegativeZero)`.
    fn fits_into_long(&mut self, is_positive: bool, ignore_negative_zero: bool) -> bool {
        while self.digits.last() == Some(&b'0') {
            self.digits.pop();
        }
        let count = self.digits.len() as i32;
        if count == 0 {
            return is_positive || ignore_negative_zero;
        }
        if self.decimal_at < count || self.decimal_at > Self::MAX_COUNT {
            return false;
        }
        if self.decimal_at < Self::MAX_COUNT {
            return true;
        }
        for (&dig, &max) in self.digits.iter().zip(LONG_MIN_REP.iter()) {
            if dig > max {
                return false;
            }
            if dig < max {
                return true;
            }
        }
        if count < self.decimal_at {
            return true;
        }
        !is_positive
    }

    /// `set(isNegative, double, maximumDigits, fixedPoint)`.
    fn set_double(
        &mut self,
        is_negative: bool,
        source: f64,
        maximum_digits: i32,
        fixed_point: bool,
        mode: RoundingMode,
    ) -> Result<()> {
        let fd = binary_to_ascii(source);
        self.set_string(
            is_negative,
            &fd.java_format_string,
            fd.digits_rounded_up,
            fd.decimal_digits_exact,
            maximum_digits,
            fixed_point,
            mode,
        )
    }

    /// `set(isNegative, String, roundedUp, valueExactAsDecimal,
    /// maximumDigits, fixedPoint)`.
    #[allow(clippy::too_many_arguments)]
    fn set_string(
        &mut self,
        is_negative: bool,
        s: &str,
        rounded_up: bool,
        value_exact_as_decimal: bool,
        maximum_digits: i32,
        fixed_point: bool,
        mode: RoundingMode,
    ) -> Result<()> {
        self.is_negative = is_negative;
        let source = s.as_bytes();
        let len = source.len();
        self.decimal_at = -1;
        self.digits.clear();
        let mut exponent: i32 = 0;
        let mut leading_zeros_after_decimal = 0;
        let mut non_zero_digit_seen = false;
        let mut i = 0;
        while i < len {
            let c = source[i];
            i += 1;
            if c == b'.' {
                self.decimal_at = self.digits.len() as i32;
            } else if c == b'e' || c == b'E' {
                exponent = parse_int(&source[i..]);
                break;
            } else {
                if !non_zero_digit_seen {
                    non_zero_digit_seen = c != b'0';
                    if !non_zero_digit_seen && self.decimal_at != -1 {
                        leading_zeros_after_decimal += 1;
                    }
                }
                if non_zero_digit_seen {
                    self.digits.push(c);
                }
            }
        }
        if self.decimal_at == -1 {
            self.decimal_at = self.digits.len() as i32;
        }
        if non_zero_digit_seen {
            self.decimal_at += exponent - leading_zeros_after_decimal;
        }
        while self.digits.len() > 1 && self.digits.last() == Some(&b'0') {
            self.digits.pop();
        }
        if fixed_point {
            if -self.decimal_at > maximum_digits {
                self.digits.clear();
                return Ok(());
            } else if -self.decimal_at == maximum_digits {
                if self.should_round_up(0, rounded_up, value_exact_as_decimal, mode)? {
                    self.digits.clear();
                    self.digits.push(b'1');
                    self.decimal_at += 1;
                } else {
                    self.digits.clear();
                }
                return Ok(());
            }
        }
        let max = if fixed_point {
            maximum_digits.wrapping_add(self.decimal_at)
        } else {
            maximum_digits
        };
        self.round(max, rounded_up, value_exact_as_decimal, mode)
    }

    /// `round(maximumDigits, alreadyRounded, valueExactAsDecimal)`.
    fn round(
        &mut self,
        maximum_digits: i32,
        already_rounded: bool,
        value_exact_as_decimal: bool,
        mode: RoundingMode,
    ) -> Result<()> {
        if maximum_digits >= 0 && (maximum_digits as usize) < self.count() {
            let mut maximum_digits = maximum_digits as usize;
            if self.should_round_up(
                maximum_digits,
                already_rounded,
                value_exact_as_decimal,
                mode,
            )? {
                maximum_digits = self.round_up(maximum_digits);
            }
            self.digits.truncate(maximum_digits);
            while self.digits.len() > 1 && self.digits.last() == Some(&b'0') {
                self.digits.pop();
            }
        }
        Ok(())
    }

    /// `shouldRoundUp(maximumDigits, alreadyRounded, valueExactAsDecimal)`.
    fn should_round_up(
        &self,
        maximum_digits: usize,
        already_rounded: bool,
        value_exact_as_decimal: bool,
        mode: RoundingMode,
    ) -> Result<bool> {
        if maximum_digits < self.count() {
            match mode {
                RoundingMode::Up => return Ok(self.non_zero_after_index(maximum_digits)),
                RoundingMode::Down => {}
                RoundingMode::Ceiling => {
                    return Ok(self.non_zero_after_index(maximum_digits) && !self.is_negative)
                }
                RoundingMode::Floor => {
                    return Ok(self.non_zero_after_index(maximum_digits) && self.is_negative)
                }
                RoundingMode::HalfUp | RoundingMode::HalfDown | RoundingMode::HalfEven => {
                    let d = self.digits[maximum_digits];
                    if d > b'5' {
                        return Ok(true);
                    } else if d == b'5' {
                        if maximum_digits == self.count() - 1 {
                            if value_exact_as_decimal {
                                return Ok(mode == RoundingMode::HalfUp
                                    || (mode == RoundingMode::HalfEven
                                        && maximum_digits > 0
                                        && !(self.digits[maximum_digits - 1] - b'0')
                                            .is_multiple_of(2)));
                            } else {
                                return Ok(!already_rounded);
                            }
                        } else {
                            return Ok(self.non_zero_after_index(maximum_digits + 1));
                        }
                    }
                }
                RoundingMode::Unnecessary => {
                    if self.non_zero_after_index(maximum_digits) {
                        return Err(LuceneError::from(rucene::error::LuceneErrorKind::Arithmetic(
                            "Rounding needed with the rounding mode being set to RoundingMode.UNNECESSARY"
                                .to_string(),
                        )));
                    }
                }
            }
        }
        Ok(false)
    }

    /// `roundUp(maximumDigits)`: the new count.
    fn round_up(&mut self, maximum_digits: usize) -> usize {
        let mut m = maximum_digits as isize;
        loop {
            m -= 1;
            if m < 0 {
                self.digits[0] = b'1';
                self.decimal_at += 1;
                m = 0;
                break;
            }
            self.digits[m as usize] += 1;
            if self.digits[m as usize] <= b'9' {
                break;
            }
        }
        (m + 1) as usize
    }

    /// `set(isNegative, long, maximumDigits)`.
    fn set_long(
        &mut self,
        is_negative: bool,
        source: i64,
        maximum_digits: i32,
        mode: RoundingMode,
    ) -> Result<()> {
        self.is_negative = is_negative;
        self.digits.clear();
        if source <= 0 {
            if source == i64::MIN {
                self.decimal_at = Self::MAX_COUNT;
                self.digits.extend_from_slice(LONG_MIN_REP);
            } else {
                self.decimal_at = 0;
            }
        } else {
            let s = source.to_string();
            self.decimal_at = s.len() as i32;
            let trimmed = s.trim_end_matches('0');
            self.digits.extend_from_slice(trimmed.as_bytes());
        }
        if maximum_digits > 0 {
            self.round(maximum_digits, false, true, mode)?;
        }
        Ok(())
    }

    /// `set(isNegative, BigInteger, maximumDigits)` for the magnitude of
    /// `Long.MIN_VALUE`.
    fn set_long_min_magnitude(
        &mut self,
        is_negative: bool,
        maximum_digits: i32,
        mode: RoundingMode,
    ) -> Result<()> {
        self.is_negative = is_negative;
        self.digits.clear();
        self.digits.extend_from_slice(LONG_MIN_REP);
        self.decimal_at = LONG_MIN_REP.len() as i32;
        while self.digits.last() == Some(&b'0') {
            self.digits.pop();
        }
        if maximum_digits > 0 {
            self.round(maximum_digits, false, true, mode)?;
        }
        Ok(())
    }
}

/// `DigitList.parseInt(char[], int, int)`: an optionally signed exponent.
fn parse_int(s: &[u8]) -> i32 {
    let mut i = 0;
    let mut positive = true;
    if let Some(&c) = s.first() {
        if c == b'-' {
            positive = false;
            i += 1;
        } else if c == b'+' {
            i += 1;
        }
    }
    let mut value: i32 = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        if !c.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(i32::from(c - b'0'));
    }
    if positive {
        value
    } else {
        -value
    }
}

/// `String.regionMatches(toffset, other, 0, other.length())` over UTF-16
/// units.
fn region_matches(text: &[u16], toffset: usize, other: &[u16]) -> bool {
    toffset <= text.len()
        && toffset + other.len() <= text.len()
        && text[toffset..toffset + other.len()] == *other
}

/// `Character.digit(char, 10)`, `-1` for none.
fn character_digit(c: u16) -> i32 {
    char::from_u32(u32::from(c))
        .and_then(|c| rucene::util::java_character_digit(c, 10))
        .map_or(-1, |d| d as i32)
}

/// The prefix and suffix positions of an affix: `(attribute, begin, end)`.
type AffixPositions = Vec<(FormatField, usize, usize)>;

/// The fast-path data of `DecimalFormat.FastPathData`.
#[derive(Debug, Clone)]
struct FastPathData {
    zero_delta: i32,
    grouping_char: u16,
    fractional_max_int_bound: i32,
    fractional_scale_factor: f64,
    positive_affixes_required: bool,
    negative_affixes_required: bool,
}

/// A `java.text.DecimalFormat`.
#[derive(Debug, Clone)]
pub struct DecimalFormat {
    symbols: DecimalFormatSymbols,
    positive_prefix: Vec<u16>,
    positive_suffix: Vec<u16>,
    negative_prefix: Vec<u16>,
    negative_suffix: Vec<u16>,
    pos_prefix_pattern: Vec<u16>,
    pos_suffix_pattern: Vec<u16>,
    neg_prefix_pattern: Vec<u16>,
    neg_suffix_pattern: Vec<u16>,
    multiplier: i32,
    grouping_size: i32,
    decimal_separator_always_shown: bool,
    is_currency_format: bool,
    use_exponential_notation: bool,
    min_exponent_digits: i32,
    parse_integer_only: bool,
    parse_strict: bool,
    grouping_used: bool,
    rounding_mode: RoundingMode,
    // `DecimalFormat`'s own digit counts
    maximum_integer_digits: i32,
    minimum_integer_digits: i32,
    maximum_fraction_digits: i32,
    minimum_fraction_digits: i32,
    // `NumberFormat`'s (`super.get…`), bounded for doubles
    super_maximum_integer_digits: i32,
    super_minimum_integer_digits: i32,
    super_maximum_fraction_digits: i32,
    super_minimum_fraction_digits: i32,
}

impl DecimalFormat {
    /// `new DecimalFormat(pattern, symbols)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `applyPattern` for a malformed
    /// pattern.
    pub fn new(pattern: &str, symbols: DecimalFormatSymbols) -> Result<Self> {
        let mut df = Self {
            symbols,
            positive_prefix: Vec::new(),
            positive_suffix: Vec::new(),
            negative_prefix: vec![PATTERN_MINUS],
            negative_suffix: Vec::new(),
            pos_prefix_pattern: Vec::new(),
            pos_suffix_pattern: Vec::new(),
            neg_prefix_pattern: Vec::new(),
            neg_suffix_pattern: Vec::new(),
            multiplier: 1,
            grouping_size: 3,
            decimal_separator_always_shown: false,
            is_currency_format: false,
            use_exponential_notation: false,
            min_exponent_digits: 0,
            parse_integer_only: false,
            parse_strict: false,
            grouping_used: true,
            rounding_mode: RoundingMode::HalfEven,
            maximum_integer_digits: 40,
            minimum_integer_digits: 1,
            maximum_fraction_digits: 3,
            minimum_fraction_digits: 0,
            super_maximum_integer_digits: 40,
            super_minimum_integer_digits: 1,
            super_maximum_fraction_digits: 3,
            super_minimum_fraction_digits: 0,
        };
        df.apply_pattern(pattern)?;
        Ok(df)
    }

    /// `applyPattern(String)` (not localized).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern, with Java's
    /// message.
    pub fn apply_pattern(&mut self, pattern: &str) -> Result<()> {
        let pattern: Vec<u16> = pattern.encode_utf16().collect();
        let pattern_str = String::from_utf16_lossy(&pattern);
        let zero_digit = PATTERN_ZERO_DIGIT;
        let grouping_separator = PATTERN_GROUPING_SEPARATOR;
        let decimal_separator = PATTERN_DECIMAL_SEPARATOR;
        let percent = PATTERN_PERCENT;
        let per_mill = PATTERN_PER_MILLE;
        let digit = PATTERN_DIGIT;
        let separator = PATTERN_SEPARATOR;
        let exponent: Vec<u16> = PATTERN_EXPONENT.encode_utf16().collect();
        let minus = PATTERN_MINUS;
        let mut got_negative = false;
        self.decimal_separator_always_shown = false;
        self.is_currency_format = false;
        self.use_exponential_notation = false;
        let mut start = 0usize;
        let mut j: i32 = 1;
        while j >= 0 && start < pattern.len() {
            let mut in_quote = false;
            let mut prefix: Vec<u16> = Vec::new();
            let mut suffix: Vec<u16> = Vec::new();
            let mut decimal_pos: i32 = -1;
            let mut multiplier = 1;
            let mut digit_left_count = 0;
            let mut zero_digit_count = 0;
            let mut digit_right_count = 0;
            let mut grouping_count: i8 = -1;
            let mut phase = 0;
            let mut in_suffix = false;
            let mut pos = start as isize;
            while (pos as usize) < pattern.len() {
                let ch = pattern[pos as usize];
                let affix = if in_suffix { &mut suffix } else { &mut prefix };
                match phase {
                    0 | 2 => {
                        if in_quote {
                            if ch == QUOTE {
                                if (pos as usize + 1) < pattern.len()
                                    && pattern[pos as usize + 1] == QUOTE
                                {
                                    pos += 1;
                                    affix.extend_from_slice(&[QUOTE, QUOTE]);
                                } else {
                                    in_quote = false;
                                }
                                pos += 1;
                                continue;
                            }
                        } else if ch == digit
                            || ch == zero_digit
                            || ch == grouping_separator
                            || ch == decimal_separator
                        {
                            phase = 1;
                            continue;
                        } else if ch == CURRENCY_SIGN {
                            let doubled = (pos as usize + 1) < pattern.len()
                                && pattern[pos as usize + 1] == CURRENCY_SIGN;
                            if doubled {
                                pos += 1;
                            }
                            self.is_currency_format = true;
                            if doubled {
                                affix.extend_from_slice(&[QUOTE, CURRENCY_SIGN, CURRENCY_SIGN]);
                            } else {
                                affix.extend_from_slice(&[QUOTE, CURRENCY_SIGN]);
                            }
                            pos += 1;
                            continue;
                        } else if ch == QUOTE {
                            if (pos as usize + 1) < pattern.len()
                                && pattern[pos as usize + 1] == QUOTE
                            {
                                pos += 1;
                                affix.extend_from_slice(&[QUOTE, QUOTE]);
                            } else {
                                in_quote = true;
                            }
                            pos += 1;
                            continue;
                        } else if ch == separator {
                            if phase == 0 || j == 0 {
                                return Err(LuceneError::illegal_argument(format!(
                                    "Unquoted special character '{}' in pattern \"{}\"",
                                    char::from_u32(u32::from(ch)).unwrap_or('\u{FFFD}'),
                                    pattern_str
                                )));
                            }
                            start = pos as usize + 1;
                            pos = pattern.len() as isize;
                            continue;
                        } else if ch == percent {
                            if multiplier != 1 {
                                return Err(LuceneError::illegal_argument(format!(
                                    "Too many percent/per mille characters in pattern \"{pattern_str}\""
                                )));
                            }
                            multiplier = 100;
                            affix.extend_from_slice(&[QUOTE, PATTERN_PERCENT]);
                            pos += 1;
                            continue;
                        } else if ch == per_mill {
                            if multiplier != 1 {
                                return Err(LuceneError::illegal_argument(format!(
                                    "Too many percent/per mille characters in pattern \"{pattern_str}\""
                                )));
                            }
                            multiplier = 1000;
                            affix.extend_from_slice(&[QUOTE, PATTERN_PER_MILLE]);
                            pos += 1;
                            continue;
                        } else if ch == minus {
                            affix.extend_from_slice(&[QUOTE, PATTERN_MINUS]);
                            pos += 1;
                            continue;
                        }
                        affix.push(ch);
                    }
                    _ => {
                        if j == 0 {
                            while (pos as usize) < pattern.len() {
                                let neg_pattern_char = pattern[pos as usize];
                                if neg_pattern_char == digit
                                    || neg_pattern_char == zero_digit
                                    || neg_pattern_char == grouping_separator
                                    || neg_pattern_char == decimal_separator
                                {
                                    pos += 1;
                                } else if region_matches(&pattern, pos as usize, &exponent) {
                                    pos += exponent.len() as isize;
                                } else {
                                    pos -= 1;
                                    phase = 2;
                                    in_suffix = true;
                                    break;
                                }
                            }
                            pos += 1;
                            continue;
                        }
                        if ch == digit {
                            if zero_digit_count > 0 {
                                digit_right_count += 1;
                            } else {
                                digit_left_count += 1;
                            }
                            if grouping_count >= 0 && decimal_pos < 0 {
                                grouping_count = grouping_count.wrapping_add(1);
                            }
                        } else if ch == zero_digit {
                            if digit_right_count > 0 {
                                return Err(LuceneError::illegal_argument(format!(
                                    "Unexpected '0' in pattern \"{pattern_str}\""
                                )));
                            }
                            zero_digit_count += 1;
                            if grouping_count >= 0 && decimal_pos < 0 {
                                grouping_count = grouping_count.wrapping_add(1);
                            }
                        } else if ch == grouping_separator {
                            grouping_count = 0;
                        } else if ch == decimal_separator {
                            if decimal_pos >= 0 {
                                return Err(LuceneError::illegal_argument(format!(
                                    "Multiple decimal separators in pattern \"{pattern_str}\""
                                )));
                            }
                            decimal_pos = digit_left_count + zero_digit_count + digit_right_count;
                        } else if region_matches(&pattern, pos as usize, &exponent) {
                            if self.use_exponential_notation {
                                return Err(LuceneError::illegal_argument(format!(
                                    "Multiple exponential symbols in pattern \"{pattern_str}\""
                                )));
                            }
                            self.use_exponential_notation = true;
                            self.min_exponent_digits = 0;
                            pos += exponent.len() as isize;
                            while (pos as usize) < pattern.len()
                                && pattern[pos as usize] == zero_digit
                            {
                                self.min_exponent_digits += 1;
                                pos += 1;
                            }
                            if (digit_left_count + zero_digit_count) < 1
                                || self.min_exponent_digits < 1
                            {
                                return Err(LuceneError::illegal_argument(format!(
                                    "Malformed exponential pattern \"{pattern_str}\""
                                )));
                            }
                            phase = 2;
                            in_suffix = true;
                            continue;
                        } else {
                            phase = 2;
                            in_suffix = true;
                            continue;
                        }
                    }
                }
                pos += 1;
            }
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
                        || decimal_pos > (digit_left_count + zero_digit_count)))
                || grouping_count == 0
                || in_quote
            {
                return Err(LuceneError::illegal_argument(format!(
                    "Malformed pattern \"{pattern_str}\""
                )));
            }
            if j == 1 {
                self.pos_prefix_pattern = prefix.clone();
                self.pos_suffix_pattern = suffix.clone();
                self.neg_prefix_pattern = prefix;
                self.neg_suffix_pattern = suffix;
                let digit_total_count = digit_left_count + zero_digit_count + digit_right_count;
                let effective_decimal_pos = if decimal_pos >= 0 {
                    decimal_pos
                } else {
                    digit_total_count
                };
                self.set_minimum_integer_digits(effective_decimal_pos - digit_left_count);
                let max_int = if self.use_exponential_notation {
                    digit_left_count + self.get_minimum_integer_digits()
                } else {
                    MAXIMUM_INTEGER_DIGITS
                };
                self.set_maximum_integer_digits(max_int);
                self.set_maximum_fraction_digits(if decimal_pos >= 0 {
                    digit_total_count - decimal_pos
                } else {
                    0
                });
                self.set_minimum_fraction_digits(if decimal_pos >= 0 {
                    digit_left_count + zero_digit_count - decimal_pos
                } else {
                    0
                });
                self.set_grouping_used(grouping_count > 0);
                self.grouping_size = if grouping_count > 0 {
                    i32::from(grouping_count)
                } else {
                    0
                };
                self.multiplier = multiplier;
                self.set_decimal_separator_always_shown(
                    decimal_pos == 0 || decimal_pos == digit_total_count,
                );
            } else {
                self.neg_prefix_pattern = prefix;
                self.neg_suffix_pattern = suffix;
                got_negative = true;
            }
            j -= 1;
        }
        if pattern.is_empty() {
            self.pos_prefix_pattern.clear();
            self.pos_suffix_pattern.clear();
            self.set_minimum_integer_digits(0);
            self.set_maximum_integer_digits(MAXIMUM_INTEGER_DIGITS);
            self.set_minimum_fraction_digits(0);
            self.set_maximum_fraction_digits(DOUBLE_FRACTION_DIGITS);
        }
        if !got_negative
            || (self.neg_prefix_pattern == self.pos_prefix_pattern
                && self.neg_suffix_pattern == self.pos_suffix_pattern)
        {
            self.neg_suffix_pattern = self.pos_suffix_pattern.clone();
            let mut p = vec![QUOTE, PATTERN_MINUS];
            p.extend_from_slice(&self.pos_prefix_pattern);
            self.neg_prefix_pattern = p;
        }
        self.expand_affixes();
        Ok(())
    }

    /// `expandAffixes()`.
    fn expand_affixes(&mut self) {
        self.positive_prefix = self.expand_affix(&self.pos_prefix_pattern.clone());
        self.positive_suffix = self.expand_affix(&self.pos_suffix_pattern.clone());
        self.negative_prefix = self.expand_affix(&self.neg_prefix_pattern.clone());
        self.negative_suffix = self.expand_affix(&self.neg_suffix_pattern.clone());
    }

    /// `expandAffix(String, StringBuilder)`.
    fn expand_affix(&self, pattern: &[u16]) -> Vec<u16> {
        let mut buffer = Vec::new();
        let mut i = 0;
        while i < pattern.len() {
            let mut c = pattern[i];
            i += 1;
            if c == QUOTE {
                c = pattern[i];
                i += 1;
                match c {
                    CURRENCY_SIGN => {
                        if i < pattern.len() && pattern[i] == CURRENCY_SIGN {
                            i += 1;
                        }
                        // the currency symbols are not emulated
                        continue;
                    }
                    PATTERN_PERCENT => {
                        buffer.extend_from_slice(&self.symbols.percent_text);
                        continue;
                    }
                    PATTERN_PER_MILLE => {
                        buffer.extend_from_slice(&self.symbols.per_mill_text);
                        continue;
                    }
                    PATTERN_MINUS => {
                        buffer.extend_from_slice(&self.symbols.minus_sign_text);
                        continue;
                    }
                    _ => {}
                }
            }
            buffer.push(c);
        }
        buffer
    }

    /// `expandAffix(String)`: the field positions of an affix pattern.
    fn affix_positions(&self, pattern: &[u16]) -> AffixPositions {
        let mut positions = Vec::new();
        let mut string_index = 0usize;
        let mut i = 0;
        while i < pattern.len() {
            let mut c = pattern[i];
            i += 1;
            if c == QUOTE {
                c = pattern[i];
                i += 1;
                let entry: Option<(FormatField, &[u16])> = match c {
                    CURRENCY_SIGN => {
                        if i < pattern.len() && pattern[i] == CURRENCY_SIGN {
                            i += 1;
                        }
                        Some((FormatField::Currency, &[]))
                    }
                    PATTERN_PERCENT => Some((FormatField::Percent, &self.symbols.percent_text)),
                    PATTERN_PER_MILLE => Some((FormatField::Permille, &self.symbols.per_mill_text)),
                    PATTERN_MINUS => Some((FormatField::Sign, &self.symbols.minus_sign_text)),
                    _ => None,
                };
                if let Some((field, string)) = entry {
                    if !string.is_empty() {
                        positions.push((field, string_index, string_index + string.len()));
                        string_index += string.len();
                        continue;
                    }
                }
            }
            string_index += 1;
        }
        positions
    }

    // ------------------------------------------------------ properties

    /// `setMaximumIntegerDigits(int)`.
    pub fn set_maximum_integer_digits(&mut self, new_value: i32) {
        self.maximum_integer_digits = new_value.clamp(0, MAXIMUM_INTEGER_DIGITS);
        self.super_set_maximum_integer_digits(
            self.maximum_integer_digits.min(DOUBLE_INTEGER_DIGITS),
        );
        if self.minimum_integer_digits > self.maximum_integer_digits {
            self.minimum_integer_digits = self.maximum_integer_digits;
            self.super_set_minimum_integer_digits(
                self.minimum_integer_digits.min(DOUBLE_INTEGER_DIGITS),
            );
        }
    }

    /// `setMinimumIntegerDigits(int)`.
    pub fn set_minimum_integer_digits(&mut self, new_value: i32) {
        self.minimum_integer_digits = new_value.clamp(0, MAXIMUM_INTEGER_DIGITS);
        self.super_set_minimum_integer_digits(
            self.minimum_integer_digits.min(DOUBLE_INTEGER_DIGITS),
        );
        if self.minimum_integer_digits > self.maximum_integer_digits {
            self.maximum_integer_digits = self.minimum_integer_digits;
            self.super_set_maximum_integer_digits(
                self.maximum_integer_digits.min(DOUBLE_INTEGER_DIGITS),
            );
        }
    }

    /// `setMaximumFractionDigits(int)`.
    pub fn set_maximum_fraction_digits(&mut self, new_value: i32) {
        self.maximum_fraction_digits = new_value.clamp(0, MAXIMUM_FRACTION_DIGITS);
        self.super_set_maximum_fraction_digits(
            self.maximum_fraction_digits.min(DOUBLE_FRACTION_DIGITS),
        );
        if self.minimum_fraction_digits > self.maximum_fraction_digits {
            self.minimum_fraction_digits = self.maximum_fraction_digits;
            self.super_set_minimum_fraction_digits(
                self.minimum_fraction_digits.min(DOUBLE_FRACTION_DIGITS),
            );
        }
    }

    /// `setMinimumFractionDigits(int)`.
    pub fn set_minimum_fraction_digits(&mut self, new_value: i32) {
        self.minimum_fraction_digits = new_value.clamp(0, MAXIMUM_FRACTION_DIGITS);
        self.super_set_minimum_fraction_digits(
            self.minimum_fraction_digits.min(DOUBLE_FRACTION_DIGITS),
        );
        if self.minimum_fraction_digits > self.maximum_fraction_digits {
            self.maximum_fraction_digits = self.minimum_fraction_digits;
            self.super_set_maximum_fraction_digits(
                self.maximum_fraction_digits.min(DOUBLE_FRACTION_DIGITS),
            );
        }
    }

    fn super_set_maximum_integer_digits(&mut self, new_value: i32) {
        self.super_maximum_integer_digits = new_value.max(0);
        if self.super_minimum_integer_digits > self.super_maximum_integer_digits {
            self.super_minimum_integer_digits = self.super_maximum_integer_digits;
        }
    }

    fn super_set_minimum_integer_digits(&mut self, new_value: i32) {
        self.super_minimum_integer_digits = new_value.max(0);
        if self.super_minimum_integer_digits > self.super_maximum_integer_digits {
            self.super_maximum_integer_digits = self.super_minimum_integer_digits;
        }
    }

    fn super_set_maximum_fraction_digits(&mut self, new_value: i32) {
        self.super_maximum_fraction_digits = new_value.max(0);
        if self.super_maximum_fraction_digits < self.super_minimum_fraction_digits {
            self.super_minimum_fraction_digits = self.super_maximum_fraction_digits;
        }
    }

    fn super_set_minimum_fraction_digits(&mut self, new_value: i32) {
        self.super_minimum_fraction_digits = new_value.max(0);
        if self.super_maximum_fraction_digits < self.super_minimum_fraction_digits {
            self.super_maximum_fraction_digits = self.super_minimum_fraction_digits;
        }
    }

    /// `getMaximumIntegerDigits()`.
    pub fn get_maximum_integer_digits(&self) -> i32 {
        self.maximum_integer_digits
    }

    /// `getMinimumIntegerDigits()`.
    pub fn get_minimum_integer_digits(&self) -> i32 {
        self.minimum_integer_digits
    }

    /// `getMaximumFractionDigits()`.
    pub fn get_maximum_fraction_digits(&self) -> i32 {
        self.maximum_fraction_digits
    }

    /// `getMinimumFractionDigits()`.
    pub fn get_minimum_fraction_digits(&self) -> i32 {
        self.minimum_fraction_digits
    }

    /// `setGroupingUsed(boolean)`.
    pub fn set_grouping_used(&mut self, new_value: bool) {
        self.grouping_used = new_value;
    }

    /// `isGroupingUsed()`.
    pub fn is_grouping_used(&self) -> bool {
        self.grouping_used
    }

    /// `setParseIntegerOnly(boolean)`.
    pub fn set_parse_integer_only(&mut self, value: bool) {
        self.parse_integer_only = value;
    }

    /// `isParseIntegerOnly()`.
    pub fn is_parse_integer_only(&self) -> bool {
        self.parse_integer_only
    }

    /// `setStrict(boolean)`.
    pub fn set_strict(&mut self, strict: bool) {
        self.parse_strict = strict;
    }

    /// `isStrict()`.
    pub fn is_strict(&self) -> bool {
        self.parse_strict
    }

    /// `setDecimalSeparatorAlwaysShown(boolean)`.
    pub fn set_decimal_separator_always_shown(&mut self, new_value: bool) {
        self.decimal_separator_always_shown = new_value;
    }

    /// `setRoundingMode(RoundingMode)`.
    pub fn set_rounding_mode(&mut self, rounding_mode: RoundingMode) {
        self.rounding_mode = rounding_mode;
    }

    /// `getRoundingMode()`.
    pub fn get_rounding_mode(&self) -> RoundingMode {
        self.rounding_mode
    }

    /// `equals(Object)` against another `DecimalFormat`: `NumberFormat.
    /// equals` (the bounded digit counts, grouping and integer-only
    /// parsing), then the affix patterns, the multiplier, the grouping size,
    /// the decimal separator display, the exponential notation, the
    /// unbounded digit counts, the rounding mode, the symbols and strict
    /// parsing, as JDK 25's `DecimalFormat.equals` compares them (the affix
    /// patterns are never `null` here, and `parseBigDecimal` is always
    /// `false`).
    ///
    /// **Divergence.** The emulated [`DecimalFormatSymbols`] hold no
    /// currency and no locale, which JDK 25's `DecimalFormatSymbols.equals`
    /// also compares: two formats whose symbols differ only in those are
    /// equal here.
    pub fn java_equals(&self, other: &DecimalFormat) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        self.super_maximum_integer_digits == other.super_maximum_integer_digits
            && self.super_minimum_integer_digits == other.super_minimum_integer_digits
            && self.super_maximum_fraction_digits == other.super_maximum_fraction_digits
            && self.super_minimum_fraction_digits == other.super_minimum_fraction_digits
            && self.grouping_used == other.grouping_used
            && self.parse_integer_only == other.parse_integer_only
            && self.pos_prefix_pattern == other.pos_prefix_pattern
            && self.pos_suffix_pattern == other.pos_suffix_pattern
            && self.neg_prefix_pattern == other.neg_prefix_pattern
            && self.neg_suffix_pattern == other.neg_suffix_pattern
            && self.multiplier == other.multiplier
            && self.grouping_size == other.grouping_size
            && self.decimal_separator_always_shown == other.decimal_separator_always_shown
            && self.use_exponential_notation == other.use_exponential_notation
            && (!self.use_exponential_notation
                || self.min_exponent_digits == other.min_exponent_digits)
            && self.maximum_integer_digits == other.maximum_integer_digits
            && self.minimum_integer_digits == other.minimum_integer_digits
            && self.maximum_fraction_digits == other.maximum_fraction_digits
            && self.minimum_fraction_digits == other.minimum_fraction_digits
            && self.rounding_mode == other.rounding_mode
            && self.symbols == other.symbols
            && self.parse_strict == other.parse_strict
    }

    /// `hashCode()`: `super.hashCode() * 37 + positivePrefix.hashCode()`,
    /// where `NumberFormat.hashCode()` is `maximumIntegerDigits * 37 +
    /// maxFractionDigits` over the serialization field `maxFractionDigits`,
    /// which keeps its initial value 3 in a format that was not deserialized.
    pub fn java_hash_code(&self) -> i32 {
        let super_hash = self
            .super_maximum_integer_digits
            .wrapping_mul(37)
            .wrapping_add(3);
        super_hash
            .wrapping_mul(37)
            .wrapping_add(super::string::hash_code(&self.positive_prefix))
    }

    /// `getDecimalFormatSymbols()`.
    pub fn get_decimal_format_symbols(&self) -> &DecimalFormatSymbols {
        &self.symbols
    }

    /// `getPositivePrefix()`.
    pub fn get_positive_prefix(&self) -> String {
        String::from_utf16_lossy(&self.positive_prefix)
    }

    /// `getNegativePrefix()`.
    pub fn get_negative_prefix(&self) -> String {
        String::from_utf16_lossy(&self.negative_prefix)
    }

    /// `getPositiveSuffix()`.
    pub fn get_positive_suffix(&self) -> String {
        String::from_utf16_lossy(&self.positive_suffix)
    }

    /// `getNegativeSuffix()`.
    pub fn get_negative_suffix(&self) -> String {
        String::from_utf16_lossy(&self.negative_suffix)
    }

    // ------------------------------------------------------------ format

    /// `NumberFormat.format(double)`: the fast path, else the standard path
    /// with `DontCareFieldPosition`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `RoundingMode.UNNECESSARY`.
    pub fn format_double(&self, number: f64) -> Result<String> {
        if let Some(s) = self.fast_format(number) {
            return Ok(s);
        }
        let mut result = Vec::new();
        let mut pos = FieldPosition::dont_care();
        self.format_double_to(number, &mut result, &mut pos)?;
        Ok(String::from_utf16_lossy(&result))
    }

    /// `NumberFormat.format(long)`.
    ///
    /// # Errors
    ///
    /// As [`format_double`](Self::format_double).
    pub fn format_long(&self, number: i64) -> Result<String> {
        let mut result = Vec::new();
        let mut pos = FieldPosition::dont_care();
        self.format_long_to(number, &mut result, &mut pos)?;
        Ok(String::from_utf16_lossy(&result))
    }

    /// `Format.format(Object)` of a `java.lang.Number`:
    /// `format(number, new StringBuffer(), new FieldPosition(0))`.
    ///
    /// # Errors
    ///
    /// As [`format_double`](Self::format_double).
    pub fn format_number(&self, number: JavaNumber) -> Result<String> {
        let mut result = Vec::new();
        let mut pos = FieldPosition::new(0);
        self.format_number_to(number, &mut result, &mut pos)?;
        Ok(String::from_utf16_lossy(&result))
    }

    /// `NumberFormat.format(Object, StringBuffer, FieldPosition)`: `Long` and
    /// `Integer` as longs, `Float` and `Double` as doubles.
    ///
    /// # Errors
    ///
    /// As [`format_double`](Self::format_double).
    pub fn format_number_to(
        &self,
        number: JavaNumber,
        result: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        match number {
            JavaNumber::Integer(v) => self.format_long_to(i64::from(v), result, pos),
            JavaNumber::Long(v) => self.format_long_to(v, result, pos),
            JavaNumber::Float(v) => self.format_double_to(f64::from(v), result, pos),
            JavaNumber::Double(v) => self.format_double_to(v, result, pos),
        }
    }

    /// `format(double, StringBuffer, FieldPosition)`.
    ///
    /// # Errors
    ///
    /// As [`format_double`](Self::format_double).
    pub fn format_double_to(
        &self,
        number: f64,
        result: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        if pos.is_dont_care() {
            if let Some(s) = self.fast_format(number) {
                result.extend(s.encode_utf16());
                return Ok(());
            }
        } else {
            pos.set_begin_index(0);
            pos.set_end_index(0);
        }
        let mut delegate = pos.delegate();
        self.format_double_delegate(number, result, &mut delegate)
    }

    fn format_double_delegate(
        &self,
        number: f64,
        result: &mut Vec<u16>,
        delegate: &mut FieldDelegate<'_>,
    ) -> Result<()> {
        let mut number = number;
        if number.is_nan() || (number.is_infinite() && self.multiplier == 0) {
            let start = result.len();
            result.extend_from_slice(&self.symbols.nan);
            delegate.formatted(INTEGER_FIELD, FormatField::Integer, start, result.len());
            return Ok(());
        }
        let is_negative =
            ((number < 0.0) || (number == 0.0 && 1.0 / number < 0.0)) ^ (self.multiplier < 0);
        if self.multiplier != 1 {
            number *= f64::from(self.multiplier);
        }
        if number.is_infinite() {
            self.append_prefix(result, delegate, is_negative);
            let start = result.len();
            result.extend_from_slice(&self.symbols.infinity);
            delegate.formatted(INTEGER_FIELD, FormatField::Integer, start, result.len());
            self.append_suffix(result, delegate, is_negative);
            return Ok(());
        }
        if is_negative {
            number = -number;
        }
        let mut digit_list = DigitList::default();
        let max_int = self.super_maximum_integer_digits;
        let min_int = self.super_minimum_integer_digits;
        let max_fra = self.super_maximum_fraction_digits;
        let min_fra = self.super_minimum_fraction_digits;
        digit_list.set_double(
            is_negative,
            number,
            if self.use_exponential_notation {
                max_int.wrapping_add(max_fra)
            } else {
                max_fra
            },
            !self.use_exponential_notation,
            self.rounding_mode,
        )?;
        self.subformat(
            result,
            delegate,
            &mut digit_list,
            is_negative,
            false,
            max_int,
            min_int,
            max_fra,
            min_fra,
        );
        Ok(())
    }

    /// `format(long, StringBuffer, FieldPosition)`.
    ///
    /// # Errors
    ///
    /// As [`format_double`](Self::format_double).
    pub fn format_long_to(
        &self,
        number: i64,
        result: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        pos.set_begin_index(0);
        pos.set_end_index(0);
        let mut delegate = pos.delegate();
        let mut number = number;
        let mut is_negative = number < 0;
        if is_negative {
            number = number.wrapping_neg();
        }
        let mut use_big_integer = false;
        if number < 0 {
            if self.multiplier != 0 {
                use_big_integer = true;
            }
        } else if self.multiplier != 1 && self.multiplier != 0 {
            let mut cutoff = i64::MAX / i64::from(self.multiplier);
            if cutoff < 0 {
                cutoff = -cutoff;
            }
            use_big_integer = number > cutoff;
        }
        let max_int = self.super_maximum_integer_digits;
        let min_int = self.super_minimum_integer_digits;
        let max_fra = self.super_maximum_fraction_digits;
        let min_fra = self.super_minimum_fraction_digits;
        let mut digit_list = DigitList::default();
        if use_big_integer {
            if self.multiplier != 1 || number != i64::MIN {
                return Err(LuceneError::unsupported_operation(
                    "DecimalFormat.format(BigInteger) is not emulated".to_string(),
                ));
            }
            // `format(BigInteger.valueOf(Long.MIN_VALUE), result, delegate,
            // true)`
            let maximum_digits = max_int.wrapping_add(max_fra);
            digit_list.set_long_min_magnitude(
                true,
                if self.use_exponential_notation {
                    if maximum_digits < 0 {
                        i32::MAX
                    } else {
                        maximum_digits
                    }
                } else {
                    0
                },
                self.rounding_mode,
            )?;
            self.subformat(
                result,
                &mut delegate,
                &mut digit_list,
                true,
                true,
                max_int,
                min_int,
                max_fra,
                min_fra,
            );
            return Ok(());
        }
        number = number.wrapping_mul(i64::from(self.multiplier));
        if number == 0 {
            is_negative = false;
        } else if self.multiplier < 0 {
            number = -number;
            is_negative = !is_negative;
        }
        digit_list.set_long(
            is_negative,
            number,
            if self.use_exponential_notation {
                max_int.wrapping_add(max_fra)
            } else {
                0
            },
            self.rounding_mode,
        )?;
        self.subformat(
            result,
            &mut delegate,
            &mut digit_list,
            is_negative,
            true,
            max_int,
            min_int,
            max_fra,
            min_fra,
        );
        Ok(())
    }

    fn append_affix(
        result: &mut Vec<u16>,
        delegate: &mut FieldDelegate<'_>,
        string: &[u16],
        positions: &AffixPositions,
    ) {
        let start = result.len();
        if !string.is_empty() {
            result.extend_from_slice(string);
            for &(attribute, begin, end) in positions {
                delegate.formatted_attr(attribute, start + begin, start + end);
            }
        }
    }

    fn append_prefix(
        &self,
        result: &mut Vec<u16>,
        delegate: &mut FieldDelegate<'_>,
        is_negative: bool,
    ) {
        if is_negative {
            let p = self.affix_positions(&self.neg_prefix_pattern);
            Self::append_affix(result, delegate, &self.negative_prefix, &p);
        } else {
            let p = self.affix_positions(&self.pos_prefix_pattern);
            Self::append_affix(result, delegate, &self.positive_prefix, &p);
        }
    }

    fn append_suffix(
        &self,
        result: &mut Vec<u16>,
        delegate: &mut FieldDelegate<'_>,
        is_negative: bool,
    ) {
        if is_negative {
            let p = self.affix_positions(&self.neg_suffix_pattern);
            Self::append_affix(result, delegate, &self.negative_suffix, &p);
        } else {
            let p = self.affix_positions(&self.pos_suffix_pattern);
            Self::append_affix(result, delegate, &self.positive_suffix, &p);
        }
    }

    /// `subformat(...)`.
    #[allow(clippy::too_many_arguments)]
    fn subformat(
        &self,
        result: &mut Vec<u16>,
        delegate: &mut FieldDelegate<'_>,
        digit_list: &mut DigitList,
        is_negative: bool,
        is_integer: bool,
        max_int_digits: i32,
        min_int_digits: i32,
        max_fra_digits: i32,
        min_fra_digits: i32,
    ) {
        self.append_prefix(result, delegate, is_negative);
        self.subformat_number(
            result,
            delegate,
            digit_list,
            is_integer,
            max_int_digits,
            min_int_digits,
            max_fra_digits,
            min_fra_digits,
        );
        self.append_suffix(result, delegate, is_negative);
    }

    /// `subformatNumber(...)`.
    #[allow(clippy::too_many_arguments)]
    fn subformat_number(
        &self,
        result: &mut Vec<u16>,
        delegate: &mut FieldDelegate<'_>,
        digit_list: &mut DigitList,
        is_integer: bool,
        max_int_digits: i32,
        min_int_digits: i32,
        max_fra_digits: i32,
        min_fra_digits: i32,
    ) {
        let grouping = if self.is_currency_format {
            self.symbols.monetary_grouping_separator
        } else {
            self.symbols.grouping_separator
        };
        let zero = self.symbols.zero_digit;
        let zero_delta = i32::from(zero) - i32::from(b'0');
        let decimal = if self.is_currency_format {
            self.symbols.monetary_separator
        } else {
            self.symbols.decimal_separator
        };
        if digit_list.is_zero() {
            digit_list.decimal_at = 0;
        }
        let digit = |d: u8| (i32::from(d) + zero_delta) as u16;
        if self.use_exponential_notation {
            let i_field_start = result.len();
            let mut i_field_end: Option<usize> = None;
            let mut f_field_start: Option<usize> = None;
            let mut exponent = digit_list.decimal_at;
            let repeat = max_int_digits;
            let mut minimum_integer_digits = min_int_digits;
            if repeat > 1 && repeat > min_int_digits {
                if exponent >= 1 {
                    exponent = ((exponent - 1) / repeat) * repeat;
                } else {
                    exponent = ((exponent - repeat) / repeat) * repeat;
                }
                minimum_integer_digits = 1;
            } else {
                exponent -= minimum_integer_digits;
            }
            let mut minimum_digits = min_int_digits.wrapping_add(min_fra_digits);
            if minimum_digits < 0 {
                minimum_digits = i32::MAX;
            }
            let integer_digits = if digit_list.is_zero() {
                minimum_integer_digits
            } else {
                digit_list.decimal_at - exponent
            };
            if minimum_digits < integer_digits {
                minimum_digits = integer_digits;
            }
            let mut total_digits = digit_list.count() as i32;
            if minimum_digits > total_digits {
                total_digits = minimum_digits;
            }
            let mut added_decimal_separator = false;
            for i in 0..total_digits {
                if i == integer_digits {
                    i_field_end = Some(result.len());
                    result.push(decimal);
                    added_decimal_separator = true;
                    f_field_start = Some(result.len());
                }
                result.push(if (i as usize) < digit_list.count() {
                    digit(digit_list.digits[i as usize])
                } else {
                    zero
                });
            }
            if self.decimal_separator_always_shown && total_digits == integer_digits {
                i_field_end = Some(result.len());
                result.push(decimal);
                added_decimal_separator = true;
                f_field_start = Some(result.len());
            }
            let i_field_end = i_field_end.unwrap_or(result.len());
            delegate.formatted(
                INTEGER_FIELD,
                FormatField::Integer,
                i_field_start,
                i_field_end,
            );
            if added_decimal_separator {
                delegate.formatted_attr(
                    FormatField::DecimalSeparator,
                    i_field_end,
                    f_field_start.unwrap_or(result.len()),
                );
            }
            let f_field_start = f_field_start.unwrap_or(result.len());
            delegate.formatted(
                FRACTION_FIELD,
                FormatField::Fraction,
                f_field_start,
                result.len(),
            );
            let field_start = result.len();
            result.extend_from_slice(&self.symbols.exponent_separator);
            delegate.formatted_attr(FormatField::ExponentSymbol, field_start, result.len());
            if digit_list.is_zero() {
                exponent = 0;
            }
            let negative_exponent = exponent < 0;
            if negative_exponent {
                exponent = -exponent;
                let field_start = result.len();
                result.extend_from_slice(&self.symbols.minus_sign_text);
                delegate.formatted_attr(FormatField::ExponentSign, field_start, result.len());
            }
            let mut exp_list = DigitList::default();
            let _ = exp_list.set_long(
                negative_exponent,
                i64::from(exponent),
                0,
                self.rounding_mode,
            );
            let e_field_start = result.len();
            for _ in exp_list.decimal_at..self.min_exponent_digits {
                result.push(zero);
            }
            for i in 0..exp_list.decimal_at {
                result.push(if (i as usize) < exp_list.count() {
                    digit(exp_list.digits[i as usize])
                } else {
                    zero
                });
            }
            delegate.formatted_attr(FormatField::Exponent, e_field_start, result.len());
        } else {
            let i_field_start = result.len();
            let mut count = min_int_digits;
            let mut digit_index: i32 = 0;
            if digit_list.decimal_at > 0 && count < digit_list.decimal_at {
                count = digit_list.decimal_at;
            }
            if count > max_int_digits {
                count = max_int_digits;
                digit_index = digit_list.decimal_at - count;
            }
            let size_before_integer_part = result.len();
            let mut i = count - 1;
            while i >= 0 {
                if i < digit_list.decimal_at && (digit_index as usize) < digit_list.count() {
                    result.push(digit(digit_list.digits[digit_index as usize]));
                    digit_index += 1;
                } else {
                    result.push(zero);
                }
                if self.grouping_used
                    && i > 0
                    && self.grouping_size != 0
                    && (i % self.grouping_size == 0)
                {
                    let g_start = result.len();
                    result.push(grouping);
                    delegate.formatted_attr(FormatField::GroupingSeparator, g_start, result.len());
                }
                i -= 1;
            }
            let fraction_present = (min_fra_digits > 0)
                || (!is_integer && (digit_index as usize) < digit_list.count());
            if !fraction_present && result.len() == size_before_integer_part {
                result.push(zero);
            }
            delegate.formatted(
                INTEGER_FIELD,
                FormatField::Integer,
                i_field_start,
                result.len(),
            );
            let s_start = result.len();
            if self.decimal_separator_always_shown || fraction_present {
                result.push(decimal);
            }
            if s_start != result.len() {
                delegate.formatted_attr(FormatField::DecimalSeparator, s_start, result.len());
            }
            let f_field_start = result.len();
            for i in 0..max_fra_digits {
                if i >= min_fra_digits
                    && (is_integer || (digit_index as usize) >= digit_list.count())
                {
                    break;
                }
                if -1 - i > (digit_list.decimal_at - 1) {
                    result.push(zero);
                    continue;
                }
                if !is_integer && (digit_index as usize) < digit_list.count() {
                    result.push(digit(digit_list.digits[digit_index as usize]));
                    digit_index += 1;
                } else {
                    result.push(zero);
                }
            }
            delegate.formatted(
                FRACTION_FIELD,
                FormatField::Fraction,
                f_field_start,
                result.len(),
            );
        }
    }

    // ------------------------------------------------------- fast path

    /// `checkAndSetFastPathStatus()` and `resetFastPathData`, computed at each
    /// call rather than cached across calls (Java's `fastPathCheckNeeded`
    /// cache gives the same data).
    fn fast_path_data(&self) -> Option<FastPathData> {
        let is_fast_path = self.rounding_mode == RoundingMode::HalfEven
            && self.grouping_used
            && self.grouping_size == 3
            && self.multiplier == 1
            && !self.decimal_separator_always_shown
            && !self.use_exponential_notation
            && self.minimum_integer_digits == 1
            && self.maximum_integer_digits >= 10
            && if self.is_currency_format {
                self.minimum_fraction_digits == 2 && self.maximum_fraction_digits == 2
            } else {
                self.minimum_fraction_digits == 0 && self.maximum_fraction_digits == 3
            };
        if !is_fast_path {
            return None;
        }
        Some(FastPathData {
            zero_delta: i32::from(self.symbols.zero_digit) - i32::from(b'0'),
            grouping_char: if self.is_currency_format {
                self.symbols.monetary_grouping_separator
            } else {
                self.symbols.grouping_separator
            },
            fractional_max_int_bound: if self.is_currency_format { 99 } else { 999 },
            fractional_scale_factor: if self.is_currency_format {
                100.0
            } else {
                1000.0
            },
            positive_affixes_required: !self.positive_prefix.is_empty()
                || !self.positive_suffix.is_empty(),
            negative_affixes_required: !self.negative_prefix.is_empty()
                || !self.negative_suffix.is_empty(),
        })
    }

    /// `fastFormat(double)`: `None` where Java returns `null`.
    fn fast_format(&self, d: f64) -> Option<String> {
        let data = self.fast_path_data()?;
        if !d.is_finite() {
            return None;
        }
        let mut d = d;
        let mut negative = false;
        if d < 0.0 {
            negative = true;
            d = -d;
        } else if d == 0.0 {
            negative = 1.0f64.copysign(d) == -1.0;
            d = 0.0;
        }
        if d > f64::from(i32::MAX) {
            return None;
        }
        Some(self.fast_double_format(&data, d, negative))
    }

    /// `fastDoubleFormat(d, negative)`, building the text directly.
    fn fast_double_format(&self, data: &FastPathData, d: f64, negative: bool) -> String {
        let mut integral_part_as_int = d as i32;
        let exact_fractional_part = d - f64::from(integral_part_as_int);
        let mut scaled_fractional = exact_fractional_part * data.fractional_scale_factor;
        let mut fractional_part_as_int = scaled_fractional as i32;
        scaled_fractional -= f64::from(fractional_part_as_int);
        if scaled_fractional >= 0.5 {
            let round_it_up = if scaled_fractional == 0.5 {
                self.exact_round_up(exact_fractional_part, fractional_part_as_int)
            } else {
                true
            };
            if round_it_up {
                if fractional_part_as_int < data.fractional_max_int_bound {
                    fractional_part_as_int += 1;
                } else {
                    fractional_part_as_int = 0;
                    integral_part_as_int += 1;
                }
            }
        }
        let decimal = if self.is_currency_format {
            self.symbols.monetary_separator
        } else {
            self.symbols.decimal_separator
        };
        // `collectIntegralDigits`
        let mut integral: Vec<u16> = Vec::new();
        let mut number = integral_part_as_int;
        let ones = |r: i32| (b'0' as i32 + r % 10) as u16;
        let tens = |r: i32| (b'0' as i32 + (r / 10) % 10) as u16;
        let hundreds = |r: i32| (b'0' as i32 + r / 100) as u16;
        let mut groups: Vec<[u16; 3]> = Vec::new();
        while number > 999 {
            let q = number / 1000;
            let r = number - q * 1000;
            number = q;
            groups.push([hundreds(r), tens(r), ones(r)]);
        }
        if number > 99 {
            integral.push(hundreds(number));
        }
        if number > 9 {
            integral.push(tens(number));
        }
        integral.push(ones(number));
        // digits are localized; grouping chars are not
        let localize = |c: u16| (i32::from(c) + data.zero_delta) as u16;
        let mut out: Vec<u16> = integral.iter().map(|&c| localize(c)).collect();
        for g in groups.iter().rev() {
            out.push(data.grouping_char);
            out.extend(g.iter().map(|&c| localize(c)));
        }
        // `collectFractionalDigits`
        let mut fraction: Vec<u16> = Vec::new();
        let f = fractional_part_as_int;
        if self.is_currency_format {
            fraction.push(tens(f));
            fraction.push(ones(f));
        } else if f != 0 {
            fraction.push(hundreds(f));
            if ones(f) != u16::from(b'0') {
                fraction.push(tens(f));
                fraction.push(ones(f));
            } else if tens(f) != u16::from(b'0') {
                fraction.push(tens(f));
            }
        }
        if !fraction.is_empty() {
            out.push(decimal);
            out.extend(fraction.iter().map(|&c| localize(c)));
        }
        let (prefix, suffix, required) = if negative {
            (
                &self.negative_prefix,
                &self.negative_suffix,
                data.negative_affixes_required,
            )
        } else {
            (
                &self.positive_prefix,
                &self.positive_suffix,
                data.positive_affixes_required,
            )
        };
        let mut text: Vec<u16> = Vec::new();
        if required {
            text.extend_from_slice(prefix);
        }
        text.extend_from_slice(&out);
        if required {
            text.extend_from_slice(suffix);
        }
        String::from_utf16_lossy(&text)
    }

    /// `exactRoundUp(fractionalPart, scaledFractionalPartAsInt)`.
    fn exact_round_up(&self, fractional_part: f64, scaled_fractional_part_as_int: i32) -> bool {
        let (approx_max, approx_medium, approx_min) = if self.is_currency_format {
            (
                fractional_part * 128.0,
                -(fractional_part * 32.0),
                fractional_part * 4.0,
            )
        } else {
            (
                fractional_part * 1024.0,
                -(fractional_part * 16.0),
                -(fractional_part * 8.0),
            )
        };
        let mut fast_two_sum_approximation = approx_medium + approx_min;
        let mut b_virtual = fast_two_sum_approximation - approx_medium;
        let mut fast_two_sum_round_off = approx_min - b_virtual;
        let approx_s1 = fast_two_sum_approximation;
        let roundoff_s1 = fast_two_sum_round_off;
        fast_two_sum_approximation = approx_max + approx_s1;
        b_virtual = fast_two_sum_approximation - approx_max;
        fast_two_sum_round_off = approx_s1 - b_virtual;
        let roundoff1000 = fast_two_sum_round_off;
        let approx1000 = fast_two_sum_approximation;
        let roundoff_total = roundoff_s1 + roundoff1000;
        fast_two_sum_approximation = approx1000 + roundoff_total;
        b_virtual = fast_two_sum_approximation - approx1000;
        let scaled_fractional_roundoff = roundoff_total - b_virtual;
        if scaled_fractional_roundoff > 0.0 {
            true
        } else if scaled_fractional_roundoff < 0.0 {
            false
        } else {
            (scaled_fractional_part_as_int & 1) != 0
        }
    }

    // ------------------------------------------------------------- parse

    /// `parse(String text, ParsePosition pos)` over UTF-16 units: the
    /// `Long` or `Double` parsed, `None` where Java returns `null`.
    pub fn parse_units(&self, text: &[u16], pos: &mut ParsePosition) -> Option<JavaNumber> {
        self.parse_units_with(text, pos, &mut DigitList::default())
    }

    /// [`parse_units`](Self::parse_units) into `digit_list`, a digit list the
    /// caller keeps across parses, as Java's `DecimalFormat.parse` works in
    /// the instance's own `digitList` field; `subparseNumber` clears it
    /// before use.
    ///
    /// PERF: no digit-list allocation per parse (rmp #679).
    pub(crate) fn parse_units_with(
        &self,
        text: &[u16],
        pos: &mut ParsePosition,
        digit_list: &mut DigitList,
    ) -> Option<JavaNumber> {
        if region_matches(text, pos.index, &self.symbols.nan) {
            pos.index += self.symbols.nan.len();
            return Some(JavaNumber::Double(f64::NAN));
        }
        let mut status_infinite = false;
        let status_positive = self.subparse(
            text,
            pos,
            &self.positive_prefix,
            &self.negative_prefix,
            digit_list,
            false,
            &mut status_infinite,
        )?;
        if status_infinite {
            return Some(JavaNumber::Double(
                if status_positive == (self.multiplier >= 0) {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                },
            ));
        }
        if self.multiplier == 0 {
            return Some(JavaNumber::Double(if digit_list.is_zero() {
                f64::NAN
            } else if status_positive {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            }));
        }
        let mut got_double = true;
        let mut got_long_minimum = false;
        let mut double_result = 0.0;
        let mut long_result: i64 = 0;
        if digit_list.fits_into_long(status_positive, self.parse_integer_only) {
            got_double = false;
            long_result = digit_list.get_long();
            if long_result < 0 {
                got_long_minimum = true;
            }
        } else {
            double_result = digit_list.get_double();
        }
        let multiplier = i64::from(self.multiplier);
        if self.multiplier != 1 {
            if got_double {
                double_result /= f64::from(self.multiplier);
            } else if long_result % multiplier == 0 {
                long_result /= multiplier;
            } else {
                double_result = (long_result as f64) / f64::from(self.multiplier);
                got_double = true;
            }
        }
        if !status_positive && !got_long_minimum {
            double_result = -double_result;
            long_result = long_result.wrapping_neg();
        }
        if self.multiplier != 1 && got_double {
            long_result = double_result as i64;
            got_double = ((double_result != long_result as f64)
                || (double_result == 0.0 && 1.0 / double_result < 0.0))
                && !self.parse_integer_only;
        }
        Some(if got_double {
            JavaNumber::Double(double_result)
        } else {
            JavaNumber::Long(long_result)
        })
    }

    /// `parse(String text, ParsePosition pos)`.
    pub fn parse_position(&self, text: &str, pos: &mut ParsePosition) -> Option<JavaNumber> {
        let units: Vec<u16> = text.encode_utf16().collect();
        self.parse_units(&units, pos)
    }

    /// `NumberFormat.parse(String)`.
    ///
    /// # Errors
    ///
    /// The `java.text.ParseException` `Unparseable number: "text"` with the
    /// error index, when nothing is parsed from the start.
    pub fn parse(&self, text: &str) -> Result<JavaNumber> {
        let mut pos = ParsePosition::new(0);
        let result = self.parse_position(text, &mut pos);
        if pos.index == 0 {
            return Err(super::format::parse_exception(
                format!("Unparseable number: \"{text}\""),
                pos.error_index,
            ));
        }
        Ok(result.expect("INVARIANT: a parse that advanced returns a number"))
    }

    /// `subparse(...)`: `Some(isPositive)` where Java returns `true`.
    #[allow(clippy::too_many_arguments)]
    fn subparse(
        &self,
        text: &[u16],
        parse_position: &mut ParsePosition,
        positive_prefix: &[u16],
        negative_prefix: &[u16],
        digits: &mut DigitList,
        is_exponent: bool,
        status_infinite: &mut bool,
    ) -> Option<bool> {
        let mut position = parse_position.index;
        let old_start = parse_position.index;
        let mut got_positive = region_matches(text, position, positive_prefix);
        let mut got_negative = region_matches(text, position, negative_prefix);
        if got_positive && got_negative {
            if positive_prefix.len() > negative_prefix.len() {
                got_negative = false;
            } else if positive_prefix.len() < negative_prefix.len() {
                got_positive = false;
            }
        }
        if got_positive {
            position += positive_prefix.len();
        } else if got_negative {
            position += negative_prefix.len();
        } else {
            parse_position.error_index = position as i64;
            return None;
        }
        let (full_pos, int_pos) =
            self.subparse_number(text, position, digits, true, is_exponent, status_infinite);
        let Some(full_pos) = full_pos else {
            parse_position.index = old_start;
            parse_position.error_index = old_start as i64;
            return None;
        };
        position = full_pos;
        if !is_exponent {
            if got_positive {
                let contains = region_matches(text, position, &self.positive_suffix);
                let ends = contains && text.len() == position + self.positive_suffix.len();
                got_positive = if self.parse_strict { ends } else { contains };
            }
            if got_negative {
                let contains = region_matches(text, position, &self.negative_suffix);
                let ends = contains && text.len() == position + self.negative_suffix.len();
                got_negative = if self.parse_strict { ends } else { contains };
            }
            if got_positive && got_negative {
                if self.positive_suffix.len() > self.negative_suffix.len() {
                    got_negative = false;
                } else if self.positive_suffix.len() < self.negative_suffix.len() {
                    got_positive = false;
                }
            }
            if got_positive == got_negative {
                parse_position.error_index = position as i64;
                return None;
            }
            if let (true, Some(int_pos)) = (self.parse_integer_only, int_pos) {
                parse_position.index = int_pos;
            } else {
                parse_position.index = position
                    + if got_positive {
                        self.positive_suffix.len()
                    } else {
                        self.negative_suffix.len()
                    };
            }
        } else {
            parse_position.index = position;
        }
        if parse_position.index == old_start {
            parse_position.error_index = position as i64;
            return None;
        }
        Some(got_positive)
    }

    /// `subparseNumber(...)`: `NumericPosition(fullPos, intPos)`, `None` for
    /// Java's `-1`.
    fn subparse_number(
        &self,
        text: &[u16],
        position: usize,
        digits: &mut DigitList,
        check_exponent: bool,
        is_exponent: bool,
        status_infinite: &mut bool,
    ) -> (Option<usize>, Option<usize>) {
        let mut position = position;
        *status_infinite = false;
        let mut int_index: Option<usize> = None;
        if !is_exponent && region_matches(text, position, &self.symbols.infinity) {
            position += self.symbols.infinity.len();
            *status_infinite = true;
            return (Some(position), int_index);
        }
        digits.clear();
        let zero = i32::from(self.symbols.zero_digit);
        let decimal = if self.is_currency_format {
            self.symbols.monetary_separator
        } else {
            self.symbols.decimal_separator
        };
        let grouping = if self.is_currency_format {
            self.symbols.monetary_grouping_separator
        } else {
            self.symbols.grouping_separator
        };
        let exponent_string = &self.symbols.exponent_separator;
        let mut saw_decimal = false;
        let mut saw_digit = false;
        let mut exponent: i64 = 0;
        let mut digit_count: i32 = 0;
        let grouping_size = self.grouping_size as i64;
        let mut prev_separator_index: i64 = -grouping_size;
        let start_pos = position;
        let mut backup: Option<usize> = None;
        while position < text.len() {
            let ch = text[position];
            let mut digit = i32::from(ch) - zero;
            if !(0..=9).contains(&digit) {
                digit = character_digit(ch);
            }
            if self.parse_strict
                && self.grouping_used
                && position as i64 == start_pos as i64 + grouping_size
                && prev_separator_index == -grouping_size
                && !saw_decimal
                && (0..=9).contains(&digit)
            {
                return (Some(position), int_index);
            }
            if digit == 0 {
                backup = None;
                saw_digit = true;
                if digits.count() == 0 {
                    if !saw_decimal {
                        position += 1;
                        continue;
                    }
                    digits.decimal_at -= 1;
                } else {
                    digit_count += 1;
                    if !saw_decimal || !self.parse_integer_only {
                        digits.append(b'0');
                    }
                }
            } else if (1..=9).contains(&digit) {
                saw_digit = true;
                digit_count += 1;
                if !saw_decimal || !self.parse_integer_only {
                    digits.append(b'0' + digit as u8);
                }
                backup = None;
            } else if !is_exponent && ch == decimal {
                if self.parse_integer_only && start_pos == position {
                    return (None, int_index);
                }
                if self.parse_strict
                    && self.is_grouping_violation(position as i64, prev_separator_index)
                {
                    return (
                        Some(self.grouping_violation_index(position as i64, prev_separator_index)),
                        int_index,
                    );
                }
                if saw_decimal {
                    break;
                }
                int_index = Some(position);
                digits.decimal_at = digit_count;
                saw_decimal = true;
            } else if !is_exponent && ch == grouping && self.grouping_used {
                if self.parse_strict {
                    if position == start_pos {
                        return (Some(start_pos), int_index);
                    }
                    if saw_decimal
                        || self.is_grouping_violation(position as i64, prev_separator_index)
                    {
                        return (
                            Some(
                                self.grouping_violation_index(
                                    position as i64,
                                    prev_separator_index,
                                ),
                            ),
                            int_index,
                        );
                    }
                    prev_separator_index = position as i64;
                } else if saw_decimal {
                    break;
                }
                backup = Some(position);
            } else if check_exponent
                && !is_exponent
                && region_matches(text, position, exponent_string)
            {
                let mut pos = ParsePosition::new(position + exponent_string.len());
                let mut exponent_digits = DigitList::default();
                let mut exponent_infinite = false;
                if let Some(exp_positive) = self.subparse(
                    text,
                    &mut pos,
                    &[],
                    &self.symbols.minus_sign_text,
                    &mut exponent_digits,
                    true,
                    &mut exponent_infinite,
                ) {
                    if exponent_digits.fits_into_long(exp_positive, true) {
                        exponent = exponent_digits.get_long();
                        if !exp_positive {
                            exponent = exponent.wrapping_neg();
                        }
                    } else {
                        exponent = if exp_positive { i64::MAX } else { i64::MIN };
                    }
                    position = pos.index;
                }
                break;
            } else {
                break;
            }
            position += 1;
        }
        if self.parse_strict
            && !saw_decimal
            && self.is_grouping_violation(position as i64, prev_separator_index)
        {
            return (
                Some(self.grouping_violation_index(position as i64 - 1, prev_separator_index)),
                int_index,
            );
        }
        if let Some(b) = backup {
            position = b;
        }
        if !saw_decimal {
            digits.decimal_at = digit_count;
        }
        if !saw_decimal || !self.parse_integer_only {
            digits.decimal_at = shift_decimal_at(digits.decimal_at, exponent);
        }
        if !saw_digit && digit_count == 0 {
            return (None, int_index);
        }
        (Some(position), int_index)
    }

    /// `isGroupingViolation(pos, prevGroupingPos)`.
    fn is_grouping_violation(&self, pos: i64, prev_grouping_pos: i64) -> bool {
        let grouping_size = i64::from(self.grouping_size);
        self.grouping_used
            && prev_grouping_pos != -grouping_size
            && pos - 1 != prev_grouping_pos + grouping_size
    }

    /// `groupingViolationIndex(pos, prevGroupingPos)`.
    fn grouping_violation_index(&self, pos: i64, prev_grouping_pos: i64) -> usize {
        pos.min(prev_grouping_pos + i64::from(self.grouping_size) + 1) as usize
    }
}

/// `DecimalFormat.shiftDecimalAt(int, long)`.
fn shift_decimal_at(decimal_at: i32, exponent: i64) -> i32 {
    match i64::from(decimal_at).checked_add(exponent) {
        None => {
            if exponent > 0 && decimal_at > 0 {
                i32::MAX
            } else {
                i32::MIN
            }
        }
        Some(sum) => i32::try_from(sum).unwrap_or(if sum > 0 { i32::MAX } else { i32::MIN }),
    }
}
