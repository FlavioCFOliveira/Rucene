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
//! The parsing of `java.text.SimpleDateFormat` built by `new
//! SimpleDateFormat(pattern, locale)` or `DateFormat.getDateInstance(
//! DateFormat.DEFAULT, locale)`, which `DateRecognizerFilter` and its factory
//! call.
//!
//! **Divergence.** An emulation of JDK 25, written from the OpenJDK
//! `jdk-25-ga` sources of `SimpleDateFormat`, `DateFormat`,
//! `DateFormatProviderImpl`, `Calendar`, `DecimalFormat`, `DigitList` and
//! `String.regionMatches` (read as specification; no JDK code is
//! reproduced), over the JDK 25 locale data of [`locale_data`]. It covers
//! what parsing reads:
//!
//! * the calendar `Calendar.getInstance(locale)` creates (its `-u-ca-` type,
//!   Buddhist for `th_TH`, Japanese for `ja_JP_JP`, Gregorian otherwise) and
//!   its time zone (`-u-tz-`, else `TimeZone.getDefault()`, see
//!   [`time_zone`]);
//! * the region override of `-u-rg-` (`CalendarDataUtility.findRegionOverride`),
//!   which `DateFormat.getDateInstance` applies to the whole format and
//!   `DateFormatSymbols` and `NumberFormat` apply to their data, with the
//!   `IllformedLocaleException` it raises;
//! * the numbering system of `-u-nu-` in the integer `NumberFormat`;
//! * the display names `Calendar.getDisplayNames` gives per calendar type
//!   (`DateFormatSymbols` for the format styles, the calendar-name provider
//!   otherwise), resolved through the CLDR parent locales;
//! * the time-zone names of `DateFormatSymbols.getZoneStrings()`, matched as
//!   `subParseZoneString` matches them, with the format's time zone updated
//!   by a match as Java updates it.
//!
//! The calendar fields a text sets are not computed: a lenient calendar (the
//! default) accepts every value, so the outcome of a parse does not depend
//! on them. The JDK's `NullPointerException` carries a helpful message; the
//! one raised here carries none (see `LuceneError::null_pointer`).
//!
//! [`locale_data`]: super::locale_data
//! [`time_zone`]: super::time_zone

use std::fmt;
use std::sync::{Mutex, PoisonError};

use rucene::error::{LuceneError, Result};

use super::character::{get_type, SPACE_SEPARATOR};
use super::locale::Locale;
use super::locale_data::{
    data, LocaleData, NumberRecord, AM_PM, CAL_TYPES, DAY_OF_WEEK, ERA, LONG_FORMAT,
    LONG_STANDALONE, MONTH, SHORT_FORMAT, SHORT_STANDALONE, YEAR,
};
use super::time_zone;

/// `DateFormat.DEFAULT`, `MEDIUM`.
pub const DEFAULT: i32 = 2;

/// `DateFormatSymbols.patternChars`, the pattern letters by field index.
const PATTERN_CHARS: &str = "GyMdkHmsSEDFwWahKzZYuXL";

const PATTERN_ERA: usize = 0;
const PATTERN_YEAR: usize = 1;
const PATTERN_MONTH: usize = 2;
const PATTERN_DAY_OF_MONTH: usize = 3;
const PATTERN_HOUR_OF_DAY1: usize = 4;
const PATTERN_HOUR_OF_DAY0: usize = 5;
const PATTERN_MINUTE: usize = 6;
const PATTERN_SECOND: usize = 7;
const PATTERN_MILLISECOND: usize = 8;
const PATTERN_DAY_OF_WEEK: usize = 9;
const PATTERN_DAY_OF_YEAR: usize = 10;
const PATTERN_DAY_OF_WEEK_IN_MONTH: usize = 11;
const PATTERN_WEEK_OF_YEAR: usize = 12;
const PATTERN_WEEK_OF_MONTH: usize = 13;
const PATTERN_AM_PM: usize = 14;
const PATTERN_HOUR1: usize = 15;
const PATTERN_HOUR0: usize = 16;
const PATTERN_ZONE_NAME: usize = 17;
const PATTERN_ZONE_VALUE: usize = 18;
const PATTERN_WEEK_YEAR: usize = 19;
const PATTERN_ISO_DAY_OF_WEEK: usize = 20;
const PATTERN_ISO_ZONE: usize = 21;
const PATTERN_MONTH_STANDALONE: usize = 22;

/// The index of the Japanese calendar in [`CAL_TYPES`].
const JAPANESE: usize = 2;

/// `SimpleDateFormat.REST_OF_STYLES`.
const REST_OF_STYLES: [i32; 3] = [SHORT_STANDALONE, LONG_FORMAT, LONG_STANDALONE];

/// One element of a compiled pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Element {
    /// `TAG_QUOTE_ASCII_CHAR`: one literal ASCII character.
    QuoteAsciiChar(u16),
    /// `TAG_QUOTE_CHARS`: literal characters.
    QuoteChars(Vec<u16>),
    /// A pattern field: its index in [`PATTERN_CHARS`] and its count.
    Field(usize, usize),
}

/// `java.text.ParsePosition`: the index parsing is at, and the index of an
/// error (`-1` for none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParsePosition {
    /// `getIndex()`, in UTF-16 units.
    pub index: usize,
    /// `getErrorIndex()`, `-1` when no error was recorded.
    pub error_index: i64,
}

impl ParsePosition {
    /// `new ParsePosition(index)`.
    pub fn new(index: usize) -> Self {
        Self {
            index,
            error_index: -1,
        }
    }
}

/// `java.text.ParseException`: the message and the error offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseException {
    /// `getMessage()`.
    pub message: String,
    /// `getErrorOffset()`.
    pub error_offset: i64,
}

impl fmt::Display for ParseException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "java.text.ParseException: {}", self.message)
    }
}

impl std::error::Error for ParseException {}

/// `Character.toUpperCase(int)`, the simple mapping.
fn upper(c: i32) -> i32 {
    rucene::util::automaton::case_folding::to_upper_case(c)
}

/// `Character.toLowerCase(int)`, the simple mapping.
fn lower(c: i32) -> i32 {
    rucene::util::automaton::case_folding::to_lower_case(c)
}

/// Whether a Java string holding `units` has the `LATIN1` coder (compact
/// strings): every unit is below U+0100.
fn is_latin1(units: &[u16]) -> bool {
    units.iter().all(|&u| u < 0x100)
}

/// `String.regionMatches(toffset, other, 0, other.length())`.
fn region_matches(text: &[u16], toffset: usize, other: &[u16]) -> bool {
    toffset + other.len() <= text.len() && text[toffset..toffset + other.len()] == *other
}

/// `CharacterDataLatin1.equalsIgnoreCase(byte, byte)`.
fn latin1_equals_ignore_case(b1: u16, b2: u16) -> bool {
    if b1 == b2 {
        return true;
    }
    let upper = b1 & 0xDF;
    if upper < u16::from(b'A') {
        return false;
    }
    (upper <= u16::from(b'Z') || ((0xC0..=0xDE).contains(&upper) && upper != 0xD7))
        && upper == (b2 & 0xDF)
}

/// `StringUTF16.compareCodePointCI(int, int) == 0`.
fn code_points_equal_ci(cp1: i32, cp2: i32) -> bool {
    let u1 = upper(cp1);
    let u2 = upper(cp2);
    u1 == u2 || lower(u1) == lower(u2)
}

/// `StringUTF16.codePointIncluding`: the code point the unit at `index`
/// belongs to within `[start, end)`, negated when the unit is a high
/// surrogate followed by its low surrogate.
fn code_point_including(units: &[u16], cp: i32, index: usize, start: usize, end: usize) -> i32 {
    let unit = cp as u16;
    if !(0xD800..=0xDFFF).contains(&unit) {
        return cp;
    }
    if (0xDC00..=0xDFFF).contains(&unit) {
        if index > start {
            let c = units[index - 1];
            if (0xD800..=0xDBFF).contains(&c) {
                return to_code_point(c, unit);
            }
        }
    } else if index + 1 < end {
        let c = units[index + 1];
        if (0xDC00..=0xDFFF).contains(&c) {
            return -to_code_point(unit, c);
        }
    }
    cp
}

/// `Character.toCodePoint(high, low)`.
fn to_code_point(high: u16, low: u16) -> i32 {
    ((i32::from(high) - 0xD800) << 10) + (i32::from(low) - 0xDC00) + 0x10000
}

/// `String.regionMatches(true, toffset, other, 0, other.length())`, by the
/// coders of the two strings as JDK 25 compares them: byte-wise for two
/// `LATIN1` strings, unit-wise (upper case, then lower case of it) for mixed
/// coders, and unit-wise with a code-point retry around surrogates for two
/// `UTF16` strings.
fn region_matches_ci(text: &[u16], toffset: usize, other: &[u16]) -> bool {
    let len = other.len();
    if toffset + len > text.len() {
        return false;
    }
    let text_latin1 = is_latin1(text);
    let other_latin1 = is_latin1(other);
    if text_latin1 && other_latin1 {
        return (0..len).all(|i| latin1_equals_ignore_case(text[toffset + i], other[i]));
    }
    if text_latin1 != other_latin1 {
        // `StringLatin1.regionMatchesCI_UTF16`, the Latin-1 string first.
        let (latin1, l_off, utf16, u_off) = if text_latin1 {
            (text, toffset, other, 0)
        } else {
            (other, 0, text, toffset)
        };
        return (0..len).all(|i| {
            let c1 = latin1[l_off + i];
            let c2 = utf16[u_off + i];
            if c1 == c2 {
                return true;
            }
            // `(char)` casts: a BMP unit's simple case mapping is a BMP unit.
            let u1 = upper(i32::from(c1)) as u16;
            let u2 = upper(i32::from(c2)) as u16;
            u1 == u2 || lower(i32::from(u1)) as u16 == lower(i32::from(u2)) as u16
        });
    }
    // `StringUTF16.compareToCIImpl(...) == 0` over the two regions.
    let tlast = toffset + len;
    let olast = len;
    let mut k1 = toffset;
    let mut k2 = 0;
    while k1 < tlast && k2 < olast {
        let mut cp1 = i32::from(text[k1]);
        let mut cp2 = i32::from(other[k2]);
        if cp1 != cp2 && !code_points_equal_ci(cp1, cp2) {
            cp1 = code_point_including(text, cp1, k1, toffset, tlast);
            if cp1 < 0 {
                k1 += 1;
                cp1 = -cp1;
            }
            cp2 = code_point_including(other, cp2, k2, 0, olast);
            if cp2 < 0 {
                k2 += 1;
                cp2 = -cp2;
            }
            if !code_points_equal_ci(cp1, cp2) {
                return false;
            }
        }
        k1 += 1;
        k2 += 1;
    }
    true
}

/// `Character.digit(char, 10)`, `-1` for none.
fn digit(c: u16) -> i32 {
    char::from_u32(u32::from(c))
        .and_then(|c| rucene::util::java_character_digit(c, 10))
        .map_or(-1, |d| d as i32)
}

/// `DigitList`: the significant digits of a parsed number and the position
/// of its decimal point.
#[derive(Debug, Default)]
struct DigitList {
    /// `digits[0..count]`, as values `0..=9`.
    digits: Vec<u8>,
    decimal_at: i32,
}

impl DigitList {
    /// `DigitList.MAX_COUNT`.
    const MAX_COUNT: i32 = 19;

    /// `fitsIntoLong(isPositive, ignoreNegativeZero)`, which trims the
    /// trailing zeros.
    fn fits_into_long(&mut self, is_positive: bool, ignore_negative_zero: bool) -> bool {
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        let count = self.digits.len() as i64;
        if count == 0 {
            return is_positive || ignore_negative_zero;
        }
        let decimal_at = i64::from(self.decimal_at);
        if decimal_at < count || decimal_at > i64::from(Self::MAX_COUNT) {
            return false;
        }
        if decimal_at < i64::from(Self::MAX_COUNT) {
            return true;
        }
        const LONG_MIN_REP: &[u8; 19] = b"9223372036854775808";
        for (i, &d) in self.digits.iter().enumerate() {
            let max = LONG_MIN_REP[i] - b'0';
            if d > max {
                return false;
            }
            if d < max {
                return true;
            }
        }
        if count < decimal_at {
            return true;
        }
        !is_positive
    }

    /// `getLong()` after `fitsIntoLong`: `Long.MIN_VALUE` for its
    /// representation, the value otherwise.
    fn get_long(&self) -> i64 {
        let mut value: i128 = 0;
        for &d in &self.digits {
            value = value * 10 + i128::from(d);
        }
        for _ in self.digits.len() as i64..i64::from(self.decimal_at) {
            value *= 10;
        }
        value as i64
    }

    /// `getDouble()`: `Double.parseDouble(".<digits>E<decimalAt>")`.
    fn get_double(&self) -> f64 {
        if self.digits.is_empty() {
            return 0.0;
        }
        let mantissa: String = self.digits.iter().map(|d| char::from(b'0' + d)).collect();
        format!("0.{mantissa}e{}", self.decimal_at)
            .parse()
            .expect("INVARIANT: a decimal literal parses")
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

/// The name sets of one display-names map: Java's `Map<String, Integer>`
/// reduced to its keys, `None` for `null`. A map built by `putAll` is the
/// union of its sets.
type NameSets = Vec<&'static [Vec<u16>]>;

/// `SimpleDateFormat.matchString(String, int, int, Map, CalendarBuilder)`:
/// the longest name of `names` matching `text` at `start`,
/// case-insensitively (a `SortedMap`, iterated longest first, gives the same
/// result); the index after it, or `-start` when none matches or the map is
/// `null`.
fn match_names(text: &[u16], start: usize, names: Option<&[&'static [Vec<u16>]]>) -> i64 {
    let Some(sets) = names else {
        return -(start as i64);
    };
    let mut best: Option<usize> = None;
    for set in sets {
        for name in set.iter() {
            if best.is_none_or(|b| name.len() > b) && region_matches_ci(text, start, name) {
                best = Some(name.len());
            }
        }
    }
    match best {
        Some(length) => (start + length) as i64,
        None => -(start as i64),
    }
}

/// The calendar type index `Calendar.getInstance(locale)` creates (see the
/// module documentation).
fn calendar_type(locale: &Locale) -> usize {
    let th_th = locale.get_language() == "th" && locale.get_country() == "TH";
    match locale.get_unicode_locale_type("ca").as_deref() {
        None => usize::from(th_th),
        Some("gregory" | "iso8601") => 0,
        Some("buddhist") => 1,
        Some("japanese") => JAPANESE,
        Some(_) => {
            if th_th {
                1
            } else if locale.get_variant() == "JP"
                && locale.get_language() == "ja"
                && locale.get_country() == "JP"
            {
                JAPANESE
            } else {
                0
            }
        }
    }
}

/// `CalendarDataUtility.findRegionOverride(locale)`: `locale` with the
/// region of its `-u-rg-` keyword when the keyword is a region followed by
/// `zzzz`.
///
/// # Errors
///
/// The `IllformedLocaleException` of `Locale.Builder.setLocale` (see
/// [`Locale::with_region`]).
fn region_override(locale: &Locale) -> Result<Locale> {
    if let Some(rg) = locale.get_unicode_locale_type("rg") {
        if rg.encode_utf16().count() == 6 {
            let rg = rg.to_ascii_uppercase();
            let b = rg.as_bytes();
            if b[0].is_ascii_uppercase() && b[1].is_ascii_uppercase() && &rg[2..] == "ZZZZ" {
                return locale.with_region(&rg[..2]);
            }
        }
    }
    Ok(locale.clone())
}

/// Whether `Calendar.getDisplayNames(field, style, locale)` answers from
/// `DateFormatSymbols` rather than from the calendar-name provider: the
/// short and long format styles (except the short era) of a Gregorian
/// calendar; a Buddhist calendar asks the provider for its eras, and the
/// Japanese calendar for every name.
fn from_symbols(calendar_type: usize, field: i32, style: i32) -> bool {
    if calendar_type == JAPANESE || (calendar_type == 1 && field == ERA) {
        return false;
    }
    let format = style == SHORT_FORMAT || style == LONG_FORMAT;
    format && !(field == ERA && style == SHORT_FORMAT) && field != YEAR
}

/// The time-zone names of one row of `DateFormatSymbols.getZoneStrings()`.
#[derive(Debug)]
struct ZoneNames {
    /// The zone ID (column 0).
    id: &'static str,
    /// Columns 1 to 4.
    names: [Vec<u16>; 4],
}

/// The time zone `Calendar.getInstance(locale)` gives a calendar: the zone
/// of the `-u-tz-` keyword when the CLDR knows it, `TimeZone.getDefault()`
/// otherwise.
fn calendar_time_zone(data: &LocaleData, locale: &Locale) -> String {
    let default = time_zone::default_id();
    time_zone_override(data, locale).unwrap_or_else(|| default.to_string())
}

/// The zone of the `-u-tz-` keyword of `locale`
/// (`TimeZoneNameUtility.convertLDMLShortID(tz).map(TimeZone::getTimeZone)`),
/// `None` without the keyword or for an ID the CLDR does not know.
fn time_zone_override(data: &LocaleData, locale: &Locale) -> Option<String> {
    let tz = locale.get_unicode_locale_type("tz")?;
    let canonical = data.tz_canonical.get(&tz)?;
    Some(time_zone::get_time_zone_id(canonical, true).expect("INVARIANT: getTimeZone falls back"))
}

/// A compiled `java.text.SimpleDateFormat`, for parsing.
///
/// Equivalent to `java.text.SimpleDateFormat` built with `new
/// SimpleDateFormat(pattern, locale)` or `DateFormat.getDateInstance(DEFAULT,
/// locale)`, lenient (its default); see the module documentation for what
/// is emulated.
#[derive(Debug)]
pub struct SimpleDateFormat {
    pattern: String,
    compiled: Vec<Element>,
    /// `forceStandaloneForm`: the pattern is a single month field.
    force_standalone_form: bool,
    /// The calendar's type, an index into [`CAL_TYPES`].
    calendar_type: usize,
    /// The display names per `LocaleData::combos` entry, `None` for `null`.
    names: Vec<Option<&'static [Vec<u16>]>>,
    /// The integer `NumberFormat`'s parse state.
    number: &'static NumberRecord,
    /// `DateFormatSymbols.getZoneStrings()`, built when the pattern has a
    /// zone-name field.
    zones: Option<Vec<ZoneNames>>,
    /// The ID of the format's time zone (`getTimeZone()`), which a parsed
    /// zone name replaces.
    time_zone: Mutex<String>,
}

impl SimpleDateFormat {
    /// `new SimpleDateFormat(pattern, locale)`.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of the region override of `locale`
    /// (see [`Locale::with_region`]), raised before the pattern is compiled;
    /// then the `IllegalArgumentException` where `SimpleDateFormat.compile`
    /// throws it: `Illegal pattern character 'c'`, `Unterminated quote`, and
    /// `invalid ISO 8601 format: length=n`.
    pub fn new(pattern: &str, locale: &Locale) -> Result<Self> {
        let data = data();
        // `initializeCalendar`: the time zone, then the calendar's week data
        // (which reads the region override)
        let time_zone = calendar_time_zone(data, locale);
        let symbols_locale = region_override(locale)?;
        let (compiled, force_standalone_form) = compile(pattern)?;
        Ok(Self::build(
            data,
            pattern.to_string(),
            compiled,
            force_standalone_form,
            locale,
            &symbols_locale,
            time_zone,
        ))
    }

    /// `DateFormat.getDateInstance(DateFormat.DEFAULT, locale)`: the medium
    /// date pattern of `locale`'s region override, for its calendar type,
    /// with the time zone of its `-u-tz-` keyword.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of the region override of `locale`
    /// (see [`Locale::with_region`]).
    ///
    /// # Panics
    ///
    /// When the JDK 25 data holds no pattern for the locale's class and
    /// calendar type, which the generator's enumeration excludes.
    pub fn get_date_instance_default(locale: &Locale) -> Result<Self> {
        let data = data();
        // `DateFormatProviderImpl.getInstance`: `new SimpleDateFormat("", rg)`
        let rg = region_override(locale)?;
        let mut time_zone = calendar_time_zone(data, &rg);
        let symbols_locale = region_override(&rg)?;
        let calendar_type = calendar_type(&rg);
        let pattern = data.class_of(&rg).patterns[calendar_type]
            .clone()
            .unwrap_or_else(|| {
                panic!(
                    "no JDK 25 date pattern for {rg} and the {} calendar",
                    CAL_TYPES[calendar_type]
                )
            });
        let (compiled, force_standalone_form) =
            compile(&pattern).expect("INVARIANT: the JDK's own patterns compile");
        // the time-zone override of `locale`
        if let Some(tz) = time_zone_override(data, locale) {
            time_zone = tz;
        }
        Ok(Self::build(
            data,
            pattern,
            compiled,
            force_standalone_form,
            &rg,
            &symbols_locale,
            time_zone,
        ))
    }

    /// The format of `locale` (Java's `this.locale`), whose
    /// `DateFormatSymbols` data is that of `symbols_locale`.
    fn build(
        data: &'static LocaleData,
        pattern: String,
        compiled: Vec<Element>,
        force_standalone_form: bool,
        locale: &Locale,
        symbols_locale: &Locale,
        time_zone: String,
    ) -> Self {
        let calendar_type = calendar_type(locale);
        let provider = &data.name_records[data.class_of(locale).names[calendar_type]];
        let symbols = &data.name_records[data.class_of(symbols_locale).symbols];
        let names = data
            .combos
            .iter()
            .enumerate()
            .map(|(i, &(field, style))| {
                let set = if from_symbols(calendar_type, field, style) {
                    symbols[i]
                } else {
                    provider[i]
                };
                data.sets[set].as_deref()
            })
            .collect();
        // `NumberFormat.getIntegerInstance(locale)`: the region override
        // applies unless `-u-nu-` is given
        let number_locale = if locale.get_unicode_locale_type("nu").is_some() {
            locale
        } else {
            symbols_locale
        };
        let number_class = data.class_of(number_locale);
        let number = number_locale
            .get_unicode_locale_type("nu")
            .and_then(|nu| number_class.nu.get(&nu).copied())
            .unwrap_or(number_class.number);
        let number = &data.numbers[number];
        let has_zone_field = compiled.iter().any(|e| {
            matches!(e, Element::Field(tag, _) if *tag == PATTERN_ZONE_NAME || *tag == PATTERN_ZONE_VALUE)
        });
        let zones = has_zone_field.then(|| {
            let zone_class = data.zone_class_of(locale);
            let zero = number.zero_digit;
            data.tables[zone_class.table]
                .iter()
                .map(|&r| {
                    let row = &data.rows[r];
                    let mut names = row.names.clone();
                    if zone_class.latin {
                        for (i, name) in names.iter_mut().enumerate() {
                            if row.digit_mask & (1 << i) != 0 {
                                for u in name.iter_mut() {
                                    if (u16::from(b'0')..=u16::from(b'9')).contains(u) {
                                        *u = zero + (*u - u16::from(b'0'));
                                    }
                                }
                            }
                        }
                    }
                    ZoneNames {
                        id: row.id.as_str(),
                        names,
                    }
                })
                .collect()
        });
        Self {
            pattern,
            compiled,
            force_standalone_form,
            calendar_type,
            names,
            number,
            zones,
            time_zone: Mutex::new(time_zone),
        }
    }

    /// `toPattern()`.
    pub fn to_pattern(&self) -> &str {
        &self.pattern
    }

    /// `getTimeZone().getID()`.
    pub fn get_time_zone_id(&self) -> String {
        self.time_zone
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// `DateFormat.parse(String)`: whether `source` starts with a date.
    ///
    /// # Errors
    ///
    /// An unchecked exception Java's `parse` raises (the
    /// `NullPointerException` of a display-names map the JDK cannot build).
    /// The inner result is the checked `ParseException` "Unparseable date:
    /// \"source\"" when no date is parsed from the start of `source` (Java's
    /// `pos.index == 0`).
    pub fn parse(&self, source: &str) -> Result<std::result::Result<(), ParseException>> {
        let mut pos = ParsePosition::new(0);
        self.parse_position(source, &mut pos)?;
        if pos.index == 0 {
            return Ok(Err(ParseException {
                message: format!("Unparseable date: \"{source}\""),
                error_offset: pos.error_index,
            }));
        }
        Ok(Ok(()))
    }

    /// `SimpleDateFormat.parse(String, ParsePosition)`: `true` where Java
    /// returns a `Date`, `false` where it returns `null`; `pos` is updated as
    /// Java updates it.
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse).
    pub fn parse_position(&self, text: &str, pos: &mut ParsePosition) -> Result<bool> {
        let text: Vec<u16> = text.encode_utf16().collect();
        let mut start = pos.index;
        let old_start = start;
        let text_length = text.len();
        for (i, element) in self.compiled.iter().enumerate() {
            match element {
                Element::QuoteAsciiChar(c) => {
                    if start >= text_length || !char_equals(text[start], *c) {
                        pos.index = old_start;
                        pos.error_index = start as i64;
                        return Ok(false);
                    }
                    start += 1;
                }
                Element::QuoteChars(chars) => {
                    for c in chars {
                        if start >= text_length || !char_equals(text[start], *c) {
                            pos.index = old_start;
                            pos.error_index = start as i64;
                            return Ok(false);
                        }
                        start += 1;
                    }
                }
                Element::Field(tag, count) => {
                    let mut obey_count = false;
                    let mut use_following_minus_sign_as_delimiter = false;
                    if let Some(next) = self.compiled.get(i + 1) {
                        if let Element::Field(next_tag, next_count) = next {
                            obey_count = should_obey_count(*next_tag, *next_count);
                        }
                        if self.number.has_following_minus_sign {
                            // the char after the tag: the literal, or the high
                            // half of a long `TAG_QUOTE_CHARS` length
                            let next_count = match next {
                                Element::QuoteAsciiChar(c) => Some(*c),
                                Element::QuoteChars(chars) if chars.len() < 255 => Some(chars[0]),
                                Element::QuoteChars(chars) => Some((chars.len() >> 16) as u16),
                                Element::Field(..) => None,
                            };
                            if next_count == Some(self.number.minus_sign) {
                                use_following_minus_sign_as_delimiter = true;
                            }
                        }
                    }
                    match self.sub_parse(
                        &text,
                        start,
                        *tag,
                        *count,
                        obey_count,
                        pos,
                        use_following_minus_sign_as_delimiter,
                    )? {
                        Some(next) => start = next,
                        None => {
                            pos.index = old_start;
                            return Ok(false);
                        }
                    }
                }
            }
        }
        // A lenient calendar establishes every field value.
        pos.index = start;
        Ok(true)
    }

    /// The display names of `(field, style)` (`Calendar.getDisplayNames`).
    fn display_names(&self, field: i32, style: i32) -> Option<&'static [Vec<u16>]> {
        let index = data()
            .combos
            .iter()
            .position(|&c| c == (field, style))
            .expect("INVARIANT: the data holds every map parsing asks for");
        self.names[index]
    }

    /// `getDisplayNamesMap(field, locale)`: the short-format map with the
    /// other short and long styles put into it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `putAll` into a `null` short-format map.
    fn display_names_map(&self, field: i32) -> Result<Option<NameSets>> {
        let mut map: Option<NameSets> = self.display_names(field, SHORT_FORMAT).map(|s| vec![s]);
        for style in REST_OF_STYLES {
            if let Some(m) = self.display_names(field, style) {
                map.as_mut().ok_or_else(LuceneError::null_pointer)?.push(m);
            }
        }
        Ok(map)
    }

    /// `getDisplayContextNamesMap(field, locale)`: the short and long maps of
    /// the format styles, or of the standalone styles when the pattern is a
    /// single month field.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `putAll` into a `null` short map.
    fn display_context_names_map(&self, field: i32) -> Result<Option<NameSets>> {
        let (short, long) = if self.force_standalone_form {
            (SHORT_STANDALONE, LONG_STANDALONE)
        } else {
            (SHORT_FORMAT, LONG_FORMAT)
        };
        let mut map: Option<NameSets> = self.display_names(field, short).map(|s| vec![s]);
        if let Some(m) = self.display_names(field, long) {
            map.as_mut().ok_or_else(LuceneError::null_pointer)?.push(m);
        }
        Ok(map)
    }

    /// The integer `DecimalFormat.parse(text, pos)` over `text`, as
    /// `Number.intValue()`; `None` where it returns `null`.
    fn parse_number(&self, text: &[u16], pos: &mut ParsePosition) -> Option<i32> {
        let n = self.number;
        // special case NaN
        if region_matches(text, pos.index, &n.nan) {
            pos.index += n.nan.len();
            return Some(0); // `Double.NaN.intValue()`
        }
        let mut digits = DigitList::default();
        let mut status_infinite = false;
        let positive = self.subparse(
            text,
            pos,
            &n.positive_prefix,
            &n.negative_prefix,
            &mut digits,
            false,
            &mut status_infinite,
        )?;
        // special case INFINITY (the multiplier is 1)
        if status_infinite {
            return Some(if positive { i32::MAX } else { i32::MIN });
        }
        if digits.fits_into_long(positive, true) {
            let long = digits.get_long();
            let got_long_minimum = long < 0;
            let long = if !positive && !got_long_minimum {
                long.wrapping_neg()
            } else {
                long
            };
            // `Long.intValue()`: the low 32 bits.
            Some(long as i32)
        } else {
            let double = digits.get_double();
            let double = if positive { double } else { -double };
            // `Double.intValue()`: toward zero, saturating.
            Some(double as i32)
        }
    }

    /// `DecimalFormat.subparse` of the lenient integer format: whether the
    /// number is positive, `None` on failure.
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
        let old_start = position;
        // check for positivePrefix; take longest
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
            let n = self.number;
            // lenient: the text only needs to contain the suffix
            if got_positive {
                got_positive = region_matches(text, position, &n.positive_suffix);
            }
            if got_negative {
                got_negative = region_matches(text, position, &n.negative_suffix);
            }
            if got_positive && got_negative {
                if n.positive_suffix.len() > n.negative_suffix.len() {
                    got_negative = false;
                } else if n.positive_suffix.len() < n.negative_suffix.len() {
                    got_positive = false;
                }
            }
            if got_positive == got_negative {
                parse_position.error_index = position as i64;
                return None;
            }
            // integer only: the index is that of the decimal separator, if
            // any
            parse_position.index = match int_pos {
                Some(int_pos) => int_pos,
                None => {
                    position
                        + if got_positive {
                            n.positive_suffix.len()
                        } else {
                            n.negative_suffix.len()
                        }
                }
            };
        } else {
            parse_position.index = position;
        }
        if parse_position.index == old_start {
            parse_position.error_index = position as i64;
            return None;
        }
        Some(got_positive)
    }

    /// `DecimalFormat.subparseNumber` of the lenient integer format without
    /// grouping: `NumericPosition(fullPos, intPos)`, `None` for Java's `-1`.
    fn subparse_number(
        &self,
        text: &[u16],
        position: usize,
        digits: &mut DigitList,
        check_exponent: bool,
        is_exponent: bool,
        status_infinite: &mut bool,
    ) -> (Option<usize>, Option<usize>) {
        let n = self.number;
        let mut position = position;
        *status_infinite = false;
        let mut int_index: Option<usize> = None;
        if !is_exponent && region_matches(text, position, &n.infinity) {
            position += n.infinity.len();
            *status_infinite = true;
            return (Some(position), int_index);
        }
        digits.decimal_at = 0;
        digits.digits.clear();
        let zero = i32::from(n.zero_digit);
        let decimal = n.decimal_separator;
        let mut saw_decimal = false;
        let mut saw_digit = false;
        let mut exponent: i64 = 0;
        let mut digit_count: i32 = 0;
        let start_pos = position;
        while position < text.len() {
            let ch = text[position];
            let mut d = i32::from(ch) - zero;
            if !(0..=9).contains(&d) {
                d = digit(ch);
            }
            if d == 0 {
                saw_digit = true;
                if digits.digits.is_empty() {
                    // Ignore leading zeros in integer part of number.
                    if !saw_decimal {
                        position += 1;
                        continue;
                    }
                    digits.decimal_at = digits.decimal_at.wrapping_sub(1);
                } else {
                    digit_count = digit_count.wrapping_add(1);
                    if !saw_decimal {
                        digits.digits.push(0);
                    }
                }
            } else if (1..=9).contains(&d) {
                saw_digit = true;
                digit_count = digit_count.wrapping_add(1);
                if !saw_decimal {
                    digits.digits.push(d as u8);
                }
            } else if !is_exponent && ch == decimal {
                if start_pos == position {
                    // Parsing int only with no integer portion, fail
                    return (None, int_index);
                }
                if saw_decimal {
                    break;
                }
                int_index = Some(position);
                digits.decimal_at = digit_count;
                saw_decimal = true;
            } else if check_exponent
                && !is_exponent
                && region_matches(text, position, &n.exponent_separator)
            {
                // Process the exponent by recursively calling subparse.
                let mut pos = ParsePosition::new(position + n.exponent_separator.len());
                let mut exponent_digits = DigitList::default();
                let mut exponent_infinite = false;
                if let Some(exponent_positive) = self.subparse(
                    text,
                    &mut pos,
                    &[],
                    &n.minus_sign_text,
                    &mut exponent_digits,
                    true,
                    &mut exponent_infinite,
                ) {
                    exponent = if exponent_digits.fits_into_long(exponent_positive, true) {
                        let value = exponent_digits.get_long();
                        if exponent_positive {
                            value
                        } else {
                            value.wrapping_neg()
                        }
                    } else if exponent_positive {
                        i64::MAX
                    } else {
                        i64::MIN
                    };
                    position = pos.index; // Advance past the exponent
                }
                break; // Whether we fail or succeed, we exit this loop
            } else {
                break;
            }
            position += 1;
        }
        if !saw_decimal {
            // If there was no decimal point we have an integer, and the
            // exponent shifts it.
            digits.decimal_at = shift_decimal_at(digit_count, exponent);
        }
        // If none of the text string was recognized.
        if !saw_digit && digit_count == 0 {
            return (None, int_index);
        }
        (Some(position), int_index)
    }

    /// The number of a numeric field, over the first `start + count` units
    /// when `obey_count`; `Err(())` where `subParse` fails before parsing
    /// (the text is shorter than that).
    fn parse_field_number(
        &self,
        text: &[u16],
        start: usize,
        count: usize,
        obey_count: bool,
        pos: &mut ParsePosition,
    ) -> std::result::Result<Option<i32>, ()> {
        if obey_count {
            if start + count > text.len() {
                return Err(());
            }
            Ok(self.parse_number(&text[..start + count], pos))
        } else {
            Ok(self.parse_number(text, pos))
        }
    }

    /// `subParse`'s treatment of a minus sign following a negative number
    /// when it may be the next literal: the number gives the sign back.
    fn give_back_following_minus_sign(
        &self,
        text: &[u16],
        value: i32,
        pos: &mut ParsePosition,
        use_following_minus_sign_as_delimiter: bool,
    ) {
        let minus_sign = self.number.minus_sign;
        if use_following_minus_sign_as_delimiter
            && value < 0
            && ((pos.index < text.len() && text[pos.index] != minus_sign)
                || (pos.index == text.len() && text[pos.index - 1] == minus_sign))
        {
            pos.index -= 1;
        }
    }

    /// `SimpleDateFormat.subParse`: the index after the field, `None`
    /// (Java's negative result) on failure, with `orig_pos.error_index` set.
    #[allow(clippy::too_many_arguments)]
    fn sub_parse(
        &self,
        text: &[u16],
        start: usize,
        pattern_char_index: usize,
        count: usize,
        obey_count: bool,
        orig_pos: &mut ParsePosition,
        use_following_minus_sign_as_delimiter: bool,
    ) -> Result<Option<usize>> {
        let mut pattern_char_index = pattern_char_index;
        let mut pos = ParsePosition::new(start);
        if pattern_char_index == PATTERN_WEEK_YEAR && !data().week_date[self.calendar_type] {
            // use calendar year 'y' instead
            pattern_char_index = PATTERN_YEAR;
        }
        let gregorian = self.calendar_type != JAPANESE;

        // If there are any spaces here, skip over them.  If we hit the end
        // of the string, then fail.
        loop {
            if pos.index >= text.len() {
                orig_pos.error_index = start as i64;
                return Ok(None);
            }
            let c = text[pos.index];
            if c != u16::from(b' ') && c != u16::from(b'\t') {
                break;
            }
            pos.index += 1;
        }

        'parsing: {
            // We handle a few special cases here where we need to parse a
            // number value.
            if pattern_char_index == PATTERN_HOUR_OF_DAY1
                || pattern_char_index == PATTERN_HOUR1
                || (pattern_char_index == PATTERN_MONTH && count <= 2)
                || (pattern_char_index == PATTERN_MONTH_STANDALONE && count <= 2)
                || pattern_char_index == PATTERN_YEAR
                || pattern_char_index == PATTERN_WEEK_YEAR
            {
                let Ok(number) = self.parse_field_number(text, start, count, obey_count, &mut pos)
                else {
                    break 'parsing;
                };
                match number {
                    None => {
                        // a non-Gregorian calendar may have text years
                        if pattern_char_index != PATTERN_YEAR || gregorian {
                            break 'parsing;
                        }
                    }
                    Some(value) => self.give_back_following_minus_sign(
                        text,
                        value,
                        &mut pos,
                        use_following_minus_sign_as_delimiter,
                    ),
                }
            }

            let matched = |index: i64| (index > 0).then_some(index as usize);
            match pattern_char_index {
                PATTERN_ERA => {
                    let map = self.display_names_map(ERA)?;
                    if let Some(index) = matched(match_names(text, start, map.as_deref())) {
                        return Ok(Some(index));
                    }
                    break 'parsing;
                }
                PATTERN_WEEK_YEAR | PATTERN_YEAR => {
                    if !gregorian {
                        // the calendar might have text representations for
                        // year values
                        let style = if count >= 4 {
                            LONG_FORMAT
                        } else {
                            SHORT_FORMAT
                        };
                        if let Some(set) = self.display_names(YEAR, style) {
                            if let Some(index) = matched(match_names(text, start, Some(&[set][..])))
                            {
                                return Ok(Some(index));
                            }
                        }
                    }
                    // the two-digit year adjustment changes no outcome
                    return Ok(Some(pos.index));
                }
                PATTERN_MONTH => {
                    if count <= 2 {
                        return Ok(Some(pos.index));
                    }
                    let map = self.display_context_names_map(MONTH)?;
                    if let Some(index) = matched(match_names(text, start, map.as_deref())) {
                        return Ok(Some(index));
                    }
                    break 'parsing;
                }
                PATTERN_MONTH_STANDALONE => {
                    if count <= 2 {
                        return Ok(Some(pos.index));
                    }
                    let map = self.display_names_map(MONTH)?;
                    if let Some(index) = matched(match_names(text, start, map.as_deref())) {
                        return Ok(Some(index));
                    }
                    break 'parsing;
                }
                PATTERN_HOUR_OF_DAY1 | PATTERN_HOUR1 => {
                    // lenient: no validation of the hour value
                    return Ok(Some(pos.index));
                }
                PATTERN_DAY_OF_WEEK => {
                    for style in [LONG_FORMAT, SHORT_FORMAT] {
                        let map = self.display_names(DAY_OF_WEEK, style);
                        let sets = map.map(|s| [s]);
                        if let Some(index) = matched(match_names(
                            text,
                            start,
                            sets.as_ref().map(|s| s.as_slice()),
                        )) {
                            return Ok(Some(index));
                        }
                    }
                    break 'parsing;
                }
                PATTERN_AM_PM => {
                    let map = self.display_names_map(AM_PM)?;
                    if let Some(index) = matched(match_names(text, start, map.as_deref())) {
                        return Ok(Some(index));
                    }
                    break 'parsing;
                }
                PATTERN_ZONE_NAME | PATTERN_ZONE_VALUE => {
                    if let Some(index) = self.sub_parse_zone(text, start, &mut pos) {
                        return Ok(Some(index));
                    }
                    break 'parsing;
                }
                PATTERN_ISO_ZONE => {
                    if text.len() <= pos.index {
                        break 'parsing;
                    }
                    let c = text[pos.index];
                    if c == u16::from(b'Z') {
                        return Ok(Some(pos.index + 1));
                    }
                    // parse text as "+/-hh[[:]mm]" based on count
                    let sign = if c == u16::from(b'+') {
                        1
                    } else if c == u16::from(b'-') {
                        -1
                    } else {
                        pos.index += 1;
                        break 'parsing;
                    };
                    pos.index += 1;
                    match sub_parse_numeric_zone(text, pos.index, sign, count, count == 3) {
                        Ok(i) => return Ok(Some(i)),
                        Err(i) => {
                            pos.index = i;
                            break 'parsing;
                        }
                    }
                }
                PATTERN_DAY_OF_MONTH
                | PATTERN_HOUR_OF_DAY0
                | PATTERN_MINUTE
                | PATTERN_SECOND
                | PATTERN_MILLISECOND
                | PATTERN_DAY_OF_YEAR
                | PATTERN_DAY_OF_WEEK_IN_MONTH
                | PATTERN_WEEK_OF_YEAR
                | PATTERN_WEEK_OF_MONTH
                | PATTERN_HOUR0
                | PATTERN_ISO_DAY_OF_WEEK => {
                    // Handle "generic" fields
                    let Ok(number) =
                        self.parse_field_number(text, start, count, obey_count, &mut pos)
                    else {
                        break 'parsing;
                    };
                    if let Some(value) = number {
                        self.give_back_following_minus_sign(
                            text,
                            value,
                            &mut pos,
                            use_following_minus_sign_as_delimiter,
                        );
                        return Ok(Some(pos.index));
                    }
                    break 'parsing;
                }
                _ => unreachable!("INVARIANT: compile accepts only the pattern letters"),
            }
        }
        // Parsing failed.
        orig_pos.error_index = pos.index as i64;
        Ok(None)
    }

    /// The `z` and `Z` fields: a `GMT` form, an RFC 822 offset, or a zone
    /// name (`subParseZoneString`).
    fn sub_parse_zone(&self, text: &[u16], start: usize, pos: &mut ParsePosition) -> Option<usize> {
        const GMT: [u16; 3] = [b'G' as u16, b'M' as u16, b'T' as u16];
        // `pos.index` is within the text (the spaces were skipped).
        let mut c = text[pos.index];
        let mut sign = 0;
        if c == u16::from(b'+') {
            sign = 1;
        } else if c == u16::from(b'-') {
            sign = -1;
        }
        if sign == 0 {
            // Try parsing a custom time zone "GMT+hh:mm" or "GMT".
            if (c == u16::from(b'G') || c == u16::from(b'g'))
                && text.len() >= start + GMT.len()
                && region_matches_ci(text, start, &GMT)
            {
                pos.index = start + GMT.len();

                if text.len() > pos.index {
                    c = text[pos.index];
                    if c == u16::from(b'+') {
                        sign = 1;
                    } else if c == u16::from(b'-') {
                        sign = -1;
                    }
                }

                if sign == 0 {
                    /* "GMT" without offset */
                    return Some(pos.index);
                }

                // Parse the rest as "hh:mm"
                pos.index += 1;
                match sub_parse_numeric_zone(text, pos.index, sign, 0, true) {
                    Ok(i) => Some(i),
                    Err(i) => {
                        pos.index = i;
                        None
                    }
                }
            } else {
                // Try parsing the text as a time zone name or abbreviation
                // (on failure Java sets `pos.index` to the index it was at).
                self.sub_parse_zone_string(text, pos.index)
            }
        } else {
            // Parse the rest as "hhmm" (RFC 822)
            pos.index += 1;
            match sub_parse_numeric_zone(text, pos.index, sign, 0, false) {
                Ok(i) => Some(i),
                Err(i) => {
                    pos.index = i;
                    None
                }
            }
        }
    }

    /// `SimpleDateFormat.subParseZoneString`: the names of the format's time
    /// zone first, then those of the default time zone, then every row in
    /// order; a match makes its zone the format's time zone. The index
    /// after the name, `None` when no name matches.
    fn sub_parse_zone_string(&self, text: &[u16], start: usize) -> Option<usize> {
        let zones = self
            .zones
            .as_ref()
            .expect("INVARIANT: built for a pattern with a zone field");
        let mut current = self
            .time_zone
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let zone_index = |id: &str| zones.iter().position(|z| z.id == id);
        let mut found: Option<(usize, usize)> = None;
        if let Some(i) = zone_index(&current) {
            if let Some(n) = match_zone_string(text, start, &zones[i].names) {
                found = Some((i, n));
            }
        }
        if found.is_none() {
            if let Some(i) = zone_index(time_zone::default_id()) {
                if let Some(n) = match_zone_string(text, start, &zones[i].names) {
                    found = Some((i, n));
                }
            }
        }
        if found.is_none() {
            found = zones
                .iter()
                .enumerate()
                .find_map(|(i, z)| match_zone_string(text, start, &z.names).map(|n| (i, n)));
        }
        let (i, n) = found?;
        // `TimeZone.getTimeZone(zoneNames[0])`, then `setTimeZone` when it
        // differs from the current zone
        let tz = time_zone::get_time_zone_id(zones[i].id, true)
            .expect("INVARIANT: getTimeZone falls back");
        if tz != *current {
            *current = tz;
        }
        Some(start + zones[i].names[n].len())
    }
}

/// `SimpleDateFormat.charEquals`: equal, or (the format is lenient) both
/// space separators.
fn char_equals(ch1: u16, ch2: u16) -> bool {
    ch1 == ch2
        || (get_type(i32::from(ch1)) == SPACE_SEPARATOR
            && get_type(i32::from(ch2)) == SPACE_SEPARATOR)
}

/// `SimpleDateFormat.matchZoneString`: the first of the long and short
/// standard, long and short daylight names matching `text` at `start`,
/// case-insensitively, as an index into `names`.
fn match_zone_string(text: &[u16], start: usize, names: &[Vec<u16>; 4]) -> Option<usize> {
    names
        .iter()
        .position(|name| region_matches_ci(text, start, name))
}

/// `SimpleDateFormat.shouldObeyCount(int, int)`: whether the following
/// field is a number field, so that this field reads only `count`
/// characters.
fn should_obey_count(tag: usize, count: usize) -> bool {
    match tag {
        PATTERN_MONTH | PATTERN_MONTH_STANDALONE => count <= 2,
        PATTERN_YEAR
        | PATTERN_DAY_OF_MONTH
        | PATTERN_HOUR_OF_DAY1
        | PATTERN_HOUR_OF_DAY0
        | PATTERN_MINUTE
        | PATTERN_SECOND
        | PATTERN_MILLISECOND
        | PATTERN_DAY_OF_YEAR
        | PATTERN_DAY_OF_WEEK_IN_MONTH
        | PATTERN_WEEK_OF_YEAR
        | PATTERN_WEEK_OF_MONTH
        | PATTERN_HOUR1
        | PATTERN_HOUR0
        | PATTERN_WEEK_YEAR
        | PATTERN_ISO_DAY_OF_WEEK => true,
        _ => false,
    }
}

/// `SimpleDateFormat.subParseNumericZone`: `Ok(index after the offset)`, or
/// `Err(index)` where Java returns `1 - index`.
fn sub_parse_numeric_zone(
    text: &[u16],
    start: usize,
    _sign: i32,
    count: usize,
    colon: bool,
) -> std::result::Result<usize, usize> {
    let is_digit = |c: u16| (u16::from(b'0')..=u16::from(b'9')).contains(&c);
    let mut index = start;
    let result: Option<usize> = (|| {
        // Java catches the IndexOutOfBoundsException of `charAt`.
        let next = |index: &mut usize| -> Option<u16> {
            let c = text.get(*index).copied();
            *index += 1;
            c
        };
        let mut c = next(&mut index)?;
        // Parse hh
        if !is_digit(c) {
            return None;
        }
        let mut hours = i32::from(c - u16::from(b'0'));
        c = next(&mut index)?;
        if is_digit(c) {
            hours = hours * 10 + i32::from(c - u16::from(b'0'));
        } else {
            // If no colon in RFC 822 or 'X' (ISO), two digits are
            // required.
            if count > 0 || !colon {
                return None;
            }
            index -= 1;
        }
        if hours > 23 {
            return None;
        }
        if count != 1 {
            // Proceed with parsing mm
            c = next(&mut index)?;
            if colon {
                if c != u16::from(b':') {
                    return None;
                }
                c = next(&mut index)?;
            }
            if !is_digit(c) {
                return None;
            }
            let mut minutes = i32::from(c - u16::from(b'0'));
            c = next(&mut index)?;
            if !is_digit(c) {
                return None;
            }
            minutes = minutes * 10 + i32::from(c - u16::from(b'0'));
            if minutes > 59 {
                return None;
            }
        }
        Some(index)
    })();
    match result {
        Some(index) => Ok(index),
        None => Err(index.saturating_sub(1)),
    }
}

/// `SimpleDateFormat.compile(String)`, for parsing: the elements and
/// `forceStandaloneForm` (the pattern holds one field, a month).
fn compile(pattern: &str) -> Result<(Vec<Element>, bool)> {
    let pattern: Vec<u16> = pattern.encode_utf16().collect();
    let length = pattern.len();
    let mut in_quote = false;
    let mut compiled: Vec<Element> = Vec::new();
    let mut tmp_buffer: Vec<u16> = Vec::new();
    let mut count = 0;
    let mut tagcount = 0;
    let mut last_tag: Option<usize> = None;
    let mut prev_tag: Option<usize> = None;
    let is_letter = |c: u16| {
        (u16::from(b'a')..=u16::from(b'z')).contains(&c)
            || (u16::from(b'A')..=u16::from(b'Z')).contains(&c)
    };
    // `encode` of a field, with the `tagcount` and `prevTag` bookkeeping
    let encode = |tag: usize,
                  length: usize,
                  compiled: &mut Vec<Element>,
                  tagcount: &mut usize,
                  prev_tag: &mut Option<usize>|
     -> Result<()> {
        if tag == PATTERN_ISO_ZONE && length >= 4 {
            return Err(LuceneError::illegal_argument(format!(
                "invalid ISO 8601 format: length={length}"
            )));
        }
        compiled.push(Element::Field(tag, length));
        *tagcount += 1;
        *prev_tag = Some(tag);
        Ok(())
    };
    let mut i = 0;
    while i < length {
        let mut c = pattern[i];
        if c == u16::from(b'\'') {
            // '' is treated as a single quote regardless of being in a
            // quoted section.
            if i + 1 < length {
                c = pattern[i + 1];
                if c == u16::from(b'\'') {
                    i += 1;
                    if count != 0 {
                        let tag = last_tag.expect("INVARIANT: count");
                        encode(tag, count, &mut compiled, &mut tagcount, &mut prev_tag)?;
                        last_tag = None;
                        count = 0;
                    }
                    if in_quote {
                        tmp_buffer.push(c);
                    } else {
                        compiled.push(Element::QuoteAsciiChar(c));
                    }
                    i += 1;
                    continue;
                }
            }
            if !in_quote {
                if count != 0 {
                    let tag = last_tag.expect("INVARIANT: count");
                    encode(tag, count, &mut compiled, &mut tagcount, &mut prev_tag)?;
                    last_tag = None;
                    count = 0;
                }
                tmp_buffer.clear();
                in_quote = true;
            } else {
                if tmp_buffer.len() == 1 && tmp_buffer[0] < 128 {
                    compiled.push(Element::QuoteAsciiChar(tmp_buffer[0]));
                } else {
                    compiled.push(Element::QuoteChars(tmp_buffer.clone()));
                }
                in_quote = false;
            }
            i += 1;
            continue;
        }
        if in_quote {
            tmp_buffer.push(c);
            i += 1;
            continue;
        }
        if !is_letter(c) {
            if count != 0 {
                let tag = last_tag.expect("INVARIANT: count");
                encode(tag, count, &mut compiled, &mut tagcount, &mut prev_tag)?;
                last_tag = None;
                count = 0;
            }
            if c < 128 {
                // In most cases, c would be a delimiter, such as ':'.
                compiled.push(Element::QuoteAsciiChar(c));
            } else {
                // Take any contiguous non-ASCII alphabet characters and put
                // them in a single TAG_QUOTE_CHARS.
                let mut j = i + 1;
                while j < length {
                    let d = pattern[j];
                    if d == u16::from(b'\'') || is_letter(d) {
                        break;
                    }
                    j += 1;
                }
                compiled.push(Element::QuoteChars(pattern[i..j].to_vec()));
                i = j;
                continue;
            }
            i += 1;
            continue;
        }

        let Some(tag) = PATTERN_CHARS.encode_utf16().position(|p| p == c) else {
            return Err(LuceneError::illegal_argument(format!(
                "Illegal pattern character '{}'",
                char::from_u32(u32::from(c)).unwrap_or(char::REPLACEMENT_CHARACTER)
            )));
        };
        if last_tag.is_none() || last_tag == Some(tag) {
            last_tag = Some(tag);
            count += 1;
            i += 1;
            continue;
        }
        let previous = last_tag.expect("INVARIANT: checked");
        encode(previous, count, &mut compiled, &mut tagcount, &mut prev_tag)?;
        last_tag = Some(tag);
        count = 1;
        i += 1;
    }

    if in_quote {
        return Err(LuceneError::illegal_argument("Unterminated quote"));
    }

    if count != 0 {
        let tag = last_tag.expect("INVARIANT: count");
        encode(tag, count, &mut compiled, &mut tagcount, &mut prev_tag)?;
    }

    let force_standalone_form = tagcount == 1 && prev_tag == Some(PATTERN_MONTH);
    Ok((compiled, force_standalone_form))
}
