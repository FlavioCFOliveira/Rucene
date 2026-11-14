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

//! `java.text.SimpleDateFormat` and the `java.text.DateFormat` factory
//! methods the query parsers call (`DateFormat.getDateInstance(style,
//! locale)`), with parsing to a `java.util.Date` and formatting.
//!
//! **Divergence.** An emulation of JDK 25, not a port of a Lucene file,
//! written from OpenJDK 25's `SimpleDateFormat` (`compile`, `format`,
//! `subFormat`, `zeroPaddingNumber`, `parse`, `subParse`, `matchString`,
//! `subParseZoneString`, `subParseNumericZone`, `initializeDefaultCentury`),
//! `CalendarBuilder`, `DateFormat`, `DateFormatProviderImpl` and
//! `String.regionMatches` (read as specification; no JDK code is reproduced),
//! over the JDK 25 locale data of [`locale_data`](super::locale_data), the
//! calendars of [`calendar`](super::calendar) and the integer
//! [`DecimalFormat`](super::decimal_format) of the format's locale. It
//! extends the copy of the `rucene-analysis-common` parsing emulation
//! (`crates/rucene-analysis-common/src/jdk/simple_date_format.rs`), which this
//! crate cannot depend on. What is not reproduced, each stated at its point:
//!
//! * formatting the `z` (time-zone name) field, which the JDK takes from
//!   `TimeZone.getDisplayName` (no date pattern of `getDateInstance` holds
//!   it): it raises `UnsupportedOperationException`;
//! * the week-year fields (`Y` parsed into a week date, `Y` formatted for a
//!   year up to the Gregorian cutover), which need `Calendar` computations
//!   the emulation does not carry (see [`calendar`](super::calendar));
//! * the order of a Java `HashMap` among names of equal length that all match
//!   the text (a `Buddhist` calendar's month names): the first in name order
//!   is taken;
//! * a parsed time-zone name's daylight saving amount, taken from the zone's
//!   last rule (`ZoneInfo.getDSTSavings()` is not in the rule data);
//! * `DateFormat.setNumberFormat`, `setCalendar`, `set2DigitYearStart`.
//!
//! The JDK's `NullPointerException` carries a helpful message; the one raised
//! here carries none (see `LuceneError::null_pointer`).

use std::sync::{Mutex, PoisonError};

use rucene::error::{LuceneError, Result};

use super::calendar::{
    Calendar, CalendarKind, AM_PM as C_AM_PM, DATE, DAY_OF_WEEK, DAY_OF_WEEK_IN_MONTH, DAY_OF_YEAR,
    DST_OFFSET, ERA as C_ERA, FIELD_COUNT, HOUR, HOUR_OF_DAY, LONG, MILLISECOND, MINUTE,
    MONTH as C_MONTH, SECOND, SHORT, WEEK_OF_MONTH, WEEK_OF_YEAR, YEAR as C_YEAR, ZONE_OFFSET,
};
use super::character::{get_type, SPACE_SEPARATOR};
use super::date::{current_time_millis, Date};
use super::decimal_format::{DecimalFormat, DecimalFormatSymbols};
use super::format::{parse_exception, FieldDelegate, FieldPosition, FormatField, ParsePosition};
use super::locale::Locale;
use super::locale_data::{
    data, from_symbols, region_override, LocaleData, AM_PM, DAY_OF_WEEK as D_DAY_OF_WEEK, ERA,
    LONG_FORMAT, LONG_STANDALONE, MONTH, SHORT_FORMAT, SHORT_STANDALONE, YEAR,
};
use super::time_zone::TimeZone;

/// `DateFormat.FULL`.
pub const FULL: i32 = 0;
/// `DateFormat.LONG`.
pub const DF_LONG: i32 = 1;
/// `DateFormat.MEDIUM`.
pub const MEDIUM: i32 = 2;
/// `DateFormat.SHORT`.
pub const DF_SHORT: i32 = 3;
/// `DateFormat.DEFAULT` (`MEDIUM`).
pub const DEFAULT: i32 = MEDIUM;

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

/// `CalendarBuilder.WEEK_YEAR`, a pseudo calendar field.
const WEEK_YEAR: usize = FIELD_COUNT;
/// `CalendarBuilder.ISO_DAY_OF_WEEK`, a pseudo calendar field.
const ISO_DAY_OF_WEEK: usize = 1000;

/// `SimpleDateFormat.PATTERN_INDEX_TO_CALENDAR_FIELD`.
const PATTERN_INDEX_TO_CALENDAR_FIELD: [usize; 23] = [
    C_ERA,
    C_YEAR,
    C_MONTH,
    DATE,
    HOUR_OF_DAY,
    HOUR_OF_DAY,
    MINUTE,
    SECOND,
    MILLISECOND,
    DAY_OF_WEEK,
    DAY_OF_YEAR,
    DAY_OF_WEEK_IN_MONTH,
    WEEK_OF_YEAR,
    WEEK_OF_MONTH,
    C_AM_PM,
    HOUR,
    HOUR,
    ZONE_OFFSET,
    ZONE_OFFSET,
    WEEK_YEAR,
    ISO_DAY_OF_WEEK,
    ZONE_OFFSET,
    C_MONTH,
];

/// `SimpleDateFormat.PATTERN_INDEX_TO_DATE_FORMAT_FIELD`: the `DateFormat`
/// field IDs, which also stand for the `DateFormat.Field` constants
/// (`PATTERN_INDEX_TO_DATE_FORMAT_FIELD_ID`) one to one.
const PATTERN_INDEX_TO_DATE_FORMAT_FIELD: [i32; 23] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 17, 1, 9, 17, 2,
];

/// `SimpleDateFormat.REST_OF_STYLES`.
const REST_OF_STYLES: [i32; 3] = [SHORT_STANDALONE, LONG_FORMAT, LONG_STANDALONE];

/// `SimpleDateFormat.MILLIS_PER_MINUTE`.
const MILLIS_PER_MINUTE: i32 = 60 * 1000;

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

/// `Character.isDigit(char)`.
fn is_java_digit(c: u16) -> bool {
    char::from_u32(u32::from(c))
        .and_then(|c| rucene::util::java_character_digit(c, 10))
        .is_some()
}

/// `String.equalsIgnoreCase(String)`.
fn equals_ignore_case(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && region_matches_ci(a, 0, b)
}

/// `java.text.CalendarBuilder`: the calendar fields a parse sets, with the
/// order they were set in.
#[derive(Debug, Clone)]
struct CalendarBuilder {
    /// Stamps (lower half) and values (upper half), `MAX_FIELD` each.
    field: [i32; 2 * (FIELD_COUNT + 1)],
    next_stamp: i32,
    max_field_index: i32,
}

/// `CalendarBuilder.MAX_FIELD`.
const MAX_FIELD: usize = FIELD_COUNT + 1;
const UNSET: i32 = 0;
const MINIMUM_USER_STAMP: i32 = 2;

impl CalendarBuilder {
    fn new() -> Self {
        Self {
            field: [0; 2 * MAX_FIELD],
            next_stamp: MINIMUM_USER_STAMP,
            max_field_index: -1,
        }
    }

    /// `set(int index, int value)`.
    fn set(&mut self, index: usize, value: i32) -> &mut Self {
        let (index, value) = if index == ISO_DAY_OF_WEEK {
            (DAY_OF_WEEK, to_calendar_day_of_week(value))
        } else {
            (index, value)
        };
        self.field[index] = self.next_stamp;
        self.next_stamp += 1;
        self.field[MAX_FIELD + index] = value;
        if index as i32 > self.max_field_index && index < FIELD_COUNT {
            self.max_field_index = index as i32;
        }
        self
    }

    /// `addYear(int value)`.
    fn add_year(&mut self, value: i32) -> &mut Self {
        self.field[MAX_FIELD + C_YEAR] = self.field[MAX_FIELD + C_YEAR].wrapping_add(value);
        self.field[MAX_FIELD + WEEK_YEAR] = self.field[MAX_FIELD + WEEK_YEAR].wrapping_add(value);
        self
    }

    /// `isSet(int index)`.
    fn is_set(&self, index: usize) -> bool {
        let index = if index == ISO_DAY_OF_WEEK {
            DAY_OF_WEEK
        } else {
            index
        };
        self.field[index] > UNSET
    }

    /// `clear(int index)`.
    fn clear(&mut self, index: usize) -> &mut Self {
        let index = if index == ISO_DAY_OF_WEEK {
            DAY_OF_WEEK
        } else {
            index
        };
        self.field[index] = UNSET;
        self.field[MAX_FIELD + index] = 0;
        self
    }

    /// `establish(Calendar cal)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for a week date (see the module
    /// documentation).
    fn establish(&mut self, cal: &mut Calendar) -> Result<()> {
        let mut week_date = self.is_set(WEEK_YEAR) && self.field[WEEK_YEAR] > self.field[C_YEAR];
        if week_date && !cal.is_week_date_supported() {
            if !self.is_set(C_YEAR) {
                let v = self.field[MAX_FIELD + WEEK_YEAR];
                self.set(C_YEAR, v);
            }
            week_date = false;
        }
        cal.clear();
        for stamp in MINIMUM_USER_STAMP..self.next_stamp {
            for index in 0..=self.max_field_index.max(-1) {
                let index = index as usize;
                if self.field[index] == stamp {
                    cal.set(index, self.field[MAX_FIELD + index]);
                    break;
                }
            }
        }
        if week_date {
            return Err(LuceneError::unsupported_operation(
                "Calendar.setWeekDate is not emulated".to_string(),
            ));
        }
        Ok(())
    }
}

/// `CalendarBuilder.toISODayOfWeek(int)`.
fn to_iso_day_of_week(calendar_day_of_week: i32) -> i32 {
    if calendar_day_of_week == 1 {
        7
    } else {
        calendar_day_of_week - 1
    }
}

/// `CalendarBuilder.toCalendarDayOfWeek(int)`.
fn to_calendar_day_of_week(iso_day_of_week: i32) -> i32 {
    if !(iso_day_of_week > 0 && iso_day_of_week <= 7) {
        return iso_day_of_week;
    }
    if iso_day_of_week == 7 {
        1
    } else {
        iso_day_of_week + 1
    }
}

/// The time-zone names of one row of `DateFormatSymbols.getZoneStrings()`.
#[derive(Debug)]
struct ZoneNames {
    /// The zone ID (column 0).
    id: &'static str,
    /// Columns 1 to 4.
    names: [Vec<u16>; 4],
}

/// A display-names map of `Calendar.getDisplayNames`, as a parse matches
/// against it: its entries in iteration order, and whether it is a
/// `SortedMap` (the calendar-name provider's `TreeMap` ordered longest name
/// first) rather than a `HashMap` (`DateFormatSymbols`).
#[derive(Debug, Clone)]
struct NamesMap {
    entries: Vec<(Vec<u16>, i32)>,
    sorted: bool,
}

impl NamesMap {
    /// `map.putAll(m)`: a key already present takes the new value.
    fn put_all(&mut self, m: &[(Vec<u16>, i32)]) {
        for (name, value) in m {
            match self.entries.iter_mut().find(|(n, _)| n == name) {
                Some(entry) => entry.1 = *value,
                None => self.entries.push((name.clone(), *value)),
            }
        }
        if self.sorted {
            self.sort();
        }
    }

    /// The `LengthBasedComparator` order: longer names first, then
    /// `String.compareTo`.
    fn sort(&mut self) {
        self.entries
            .sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    }
}

/// The time zone `Calendar.getInstance(locale)` gives a calendar: the zone
/// of the `-u-tz-` keyword when the CLDR knows it, `TimeZone.getDefault()`
/// otherwise (`Calendar.defaultTimeZone`).
fn calendar_time_zone(data: &LocaleData, locale: &Locale) -> TimeZone {
    time_zone_override(data, locale).unwrap_or_else(TimeZone::get_default)
}

/// The zone of the `-u-tz-` keyword of `locale`
/// (`TimeZoneNameUtility.convertLDMLShortID(tz).map(TimeZone::getTimeZone)`),
/// `None` without the keyword or for an ID the CLDR does not know.
fn time_zone_override(data: &LocaleData, locale: &Locale) -> Option<TimeZone> {
    let tz = locale.get_unicode_locale_type("tz")?;
    let canonical = data.tz_canonical.get(&tz)?;
    Some(TimeZone::get_time_zone(canonical))
}

/// The state a format changes as it works: its calendar (Java's
/// `calendar`, which `format` and `parse` set and which a parsed zone name
/// changes the time zone of).
#[derive(Debug)]
struct State {
    calendar: Calendar,
}

/// A `java.text.SimpleDateFormat`.
///
/// Equivalent to `java.text.SimpleDateFormat` built with `new
/// SimpleDateFormat(pattern, locale)` or by `DateFormat.getDateInstance(style,
/// locale)`; see the module documentation for what is emulated. Its calendar
/// is behind a mutex, so that formatting and parsing take `&self` as the
/// `DateFormat` trait does (Java's instances are not thread-safe either).
#[derive(Debug)]
pub struct SimpleDateFormat {
    pattern: String,
    compiled: Vec<Element>,
    /// `forceStandaloneForm`: the pattern is a single month field.
    force_standalone_form: bool,
    /// The format's `locale`.
    locale: Locale,
    /// The calendar's type, an index into
    /// [`CAL_TYPES`](super::locale_data::CAL_TYPES).
    calendar_type: usize,
    /// The `DateFormatSymbols` arrays: eras, months, short months, weekdays,
    /// short weekdays, AM/PM strings.
    symbols: [Vec<Option<String>>; 6],
    /// The display-names maps per `LocaleData::combos` entry (`None` for
    /// `null`), as `Calendar.getDisplayNames` answers them.
    names: Vec<Option<NamesMap>>,
    /// The integer `NumberFormat`, grouping off.
    number_format: DecimalFormat,
    /// `zeroDigit`.
    zero_digit: u16,
    /// `hasFollowingMinusSign` and `minusSign`.
    has_following_minus_sign: bool,
    minus_sign: u16,
    /// `DateFormatSymbols.getZoneStrings()`, built when the pattern has a
    /// zone field.
    zones: Option<Vec<ZoneNames>>,
    /// `defaultCenturyStart` and `defaultCenturyStartYear`.
    default_century_start: Date,
    default_century_start_year: i32,
    state: Mutex<State>,
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
        let zone = calendar_time_zone(data, locale);
        let calendar = Calendar::get_instance(&zone, locale)?;
        let (compiled, force_standalone_form) = compile(pattern)?;
        Self::build(
            data,
            pattern.to_string(),
            compiled,
            force_standalone_form,
            locale,
            calendar,
        )
    }

    /// `DateFormat.getDateInstance(style, locale)`: the date pattern of the
    /// style for `locale`'s region override and its calendar type, with the
    /// time zone of its `-u-tz-` keyword (`DateFormatProviderImpl`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal date style " + style)` for a style
    /// outside `FULL`..`SHORT`, and the `IllformedLocaleException` of the
    /// region override of `locale`.
    ///
    /// # Panics
    ///
    /// When the JDK 25 data holds no pattern for the locale's class and
    /// calendar type, which the generator's enumeration excludes.
    pub fn get_date_instance(style: i32, locale: &Locale) -> Result<Self> {
        if !(0..=3).contains(&style) {
            return Err(LuceneError::illegal_argument(format!(
                "Illegal date style {style}"
            )));
        }
        let data = data();
        // `new SimpleDateFormat("", rg)`
        let rg = region_override(locale)?;
        let zone = calendar_time_zone(data, &rg);
        let calendar = Calendar::get_instance(&zone, &rg)?;
        let calendar_type = calendar.calendar_type_index();
        let pattern = data.class_of(&rg).patterns[style as usize][calendar_type]
            .clone()
            .unwrap_or_else(|| {
                panic!(
                    "no JDK 25 date pattern for {rg} and the {} calendar",
                    super::locale_data::CAL_TYPES[calendar_type]
                )
            });
        let (compiled, force_standalone_form) =
            compile(&pattern).expect("INVARIANT: the JDK's own patterns compile");
        let format = Self::build(
            data,
            pattern,
            compiled,
            force_standalone_form,
            &rg,
            calendar,
        )?;
        // the time-zone override of `locale`
        if let Some(tz) = time_zone_override(data, locale) {
            format.set_time_zone(&tz);
        }
        Ok(format)
    }

    /// The format of `locale` (Java's `this.locale`) with `calendar`.
    fn build(
        data: &'static LocaleData,
        pattern: String,
        compiled: Vec<Element>,
        force_standalone_form: bool,
        locale: &Locale,
        mut calendar: Calendar,
    ) -> Result<Self> {
        let symbols_locale = region_override(locale)?;
        let calendar_type = calendar.calendar_type_index();
        let symbols_arrays = data.format_symbols(locale)?;
        let symbols: [Vec<Option<String>>; 6] = std::array::from_fn(|i| symbols_arrays[i].to_vec());
        let provider = &data.name_records[data.class_of(locale).names[calendar_type]];
        let symbol_maps = &data.name_records[data.class_of(&symbols_locale).symbols];
        let names = data
            .combos
            .iter()
            .enumerate()
            .map(|(i, &(field, style))| {
                let dfs = from_symbols(calendar_type, field, style);
                let set = if dfs { symbol_maps[i] } else { provider[i] };
                data.sets[set].as_ref().map(|entries| {
                    let mut m = NamesMap {
                        entries: entries.clone(),
                        sorted: !dfs,
                    };
                    if m.sorted {
                        m.sort();
                    }
                    m
                })
            })
            .collect();
        // `NumberFormat.getIntegerInstance(locale)` with grouping off
        let record = data.number_record(locale)?;
        let mut number_format =
            DecimalFormat::new(&record.pattern, DecimalFormatSymbols::of(record))?;
        number_format.set_maximum_fraction_digits(0);
        number_format.set_decimal_separator_always_shown(false);
        number_format.set_parse_integer_only(true);
        number_format.set_grouping_used(false);
        // `checkNegativeNumberExpression` (see `has_following_minus_sign`)
        let (has_following_minus_sign, minus_sign) = {
            let p: Vec<char> = record.pattern.chars().collect();
            match p.iter().position(|&c| c == ';') {
                Some(separator_index) => {
                    let minus_index = p[separator_index..]
                        .iter()
                        .position(|&c| c == '-')
                        .map(|i| i + separator_index);
                    let last0 = p.iter().rposition(|&c| c == '0');
                    let last_hash = p.iter().rposition(|&c| c == '#');
                    let follows = match minus_index {
                        Some(m) => last0.is_none_or(|l| m > l) && last_hash.is_none_or(|l| m > l),
                        None => false,
                    };
                    (follows, record.minus_sign)
                }
                None => (false, record.minus_sign),
            }
        };
        let has_zone_field = compiled.iter().any(|e| {
            matches!(e, Element::Field(tag, _) if *tag == PATTERN_ZONE_NAME || *tag == PATTERN_ZONE_VALUE)
        });
        let zones = has_zone_field.then(|| {
            let zone_class = data.zone_class_of(locale);
            let zero = record.zero_digit;
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
        // `initializeDefaultCentury()`
        calendar.set_time_in_millis(current_time_millis());
        calendar.add(C_YEAR, -80)?;
        let default_century_start = calendar.get_time()?;
        calendar.set_time(default_century_start);
        let default_century_start_year = calendar.get(C_YEAR)?;
        Ok(Self {
            pattern,
            compiled,
            force_standalone_form,
            locale: locale.clone(),
            calendar_type,
            symbols,
            names,
            zero_digit: record.zero_digit,
            number_format,
            has_following_minus_sign,
            minus_sign,
            zones,
            default_century_start,
            default_century_start_year,
            state: Mutex::new(State { calendar }),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `toPattern()`.
    pub fn to_pattern(&self) -> &str {
        &self.pattern
    }

    /// `getTimeZone()`.
    pub fn get_time_zone(&self) -> TimeZone {
        self.state().calendar.get_time_zone().clone()
    }

    /// `setTimeZone(TimeZone)`.
    pub fn set_time_zone(&self, zone: &TimeZone) {
        self.state().calendar.set_time_zone(zone);
    }

    /// `isLenient()`.
    pub fn is_lenient(&self) -> bool {
        self.state().calendar.is_lenient()
    }

    /// `setLenient(boolean)`.
    pub fn set_lenient(&self, lenient: bool) {
        self.state().calendar.set_lenient(lenient);
    }

    /// `useDateFormatSymbols()`: the calendar is Gregorian (`"gregory"`).
    fn use_date_format_symbols(&self) -> bool {
        self.calendar_type == 0
    }

    // ------------------------------------------------------------ format

    /// `format(Date)`.
    ///
    /// # Errors
    ///
    /// As [`format_to`](Self::format_to).
    pub fn format(&self, date: Date) -> Result<String> {
        let mut buffer = Vec::new();
        let mut pos = FieldPosition::dont_care();
        self.format_to(date, &mut buffer, &mut pos)?;
        Ok(String::from_utf16_lossy(&buffer))
    }

    /// `format(Date, StringBuffer, FieldPosition)`, appending UTF-16 units.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for the fields the module
    /// documentation lists, and the errors of the calendar.
    pub fn format_to(
        &self,
        date: Date,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        pos.set_begin_index(0);
        pos.set_end_index(0);
        let mut delegate = pos.delegate();
        let mut state = self.state();
        let calendar = &mut state.calendar;
        calendar.set_time(date);
        let use_date_format_symbols = self.use_date_format_symbols();
        for element in &self.compiled {
            match element {
                Element::QuoteAsciiChar(c) => to_append_to.push(*c),
                Element::QuoteChars(chars) => to_append_to.extend_from_slice(chars),
                Element::Field(tag, count) => self.sub_format(
                    calendar,
                    *tag,
                    *count,
                    &mut delegate,
                    to_append_to,
                    use_date_format_symbols,
                )?,
            }
        }
        Ok(())
    }

    /// `subFormat(patternCharIndex, count, delegate, buffer,
    /// useDateFormatSymbols)`.
    fn sub_format(
        &self,
        calendar: &mut Calendar,
        pattern_char_index: usize,
        count: usize,
        delegate: &mut FieldDelegate<'_>,
        buffer: &mut Vec<u16>,
        use_date_format_symbols: bool,
    ) -> Result<()> {
        let max_int_count = i32::MAX;
        let mut current: Option<String> = None;
        let begin_offset = buffer.len();
        let mut pattern_char_index = pattern_char_index;
        let mut field = PATTERN_INDEX_TO_CALENDAR_FIELD[pattern_char_index];
        let mut value;
        if field == WEEK_YEAR {
            if calendar.is_week_date_supported() {
                value = calendar.get_week_year()?;
            } else {
                pattern_char_index = PATTERN_YEAR;
                field = PATTERN_INDEX_TO_CALENDAR_FIELD[pattern_char_index];
                value = calendar.get(field)?;
            }
        } else if field == ISO_DAY_OF_WEEK {
            value = to_iso_day_of_week(calendar.get(DAY_OF_WEEK)?);
        } else {
            value = calendar.get(field)?;
        }
        let count_i = count as i32;
        let style = if count >= 4 { LONG } else { SHORT };
        if !use_date_format_symbols
            && field < ZONE_OFFSET
            && pattern_char_index != PATTERN_MONTH_STANDALONE
        {
            current = calendar.get_display_name(field, style, &self.locale)?;
        }
        let symbol = |array: usize, index: i32| -> Option<String> {
            usize::try_from(index)
                .ok()
                .and_then(|i| self.symbols[array].get(i))
                .cloned()
                .flatten()
        };
        match pattern_char_index {
            PATTERN_ERA => {
                if use_date_format_symbols && (value as usize) < self.symbols[0].len() {
                    current = symbol(0, value);
                }
                if current.is_none() {
                    current = Some(String::new());
                }
            }
            PATTERN_WEEK_YEAR | PATTERN_YEAR => {
                if calendar.kind() != CalendarKind::Japanese {
                    if count != 2 {
                        self.zero_padding_number(value, count_i, max_int_count, buffer)?;
                    } else {
                        self.zero_padding_number(value, 2, 2, buffer)?;
                    }
                } else if current.is_none() {
                    self.zero_padding_number(
                        value,
                        if style == LONG { 1 } else { count_i },
                        max_int_count,
                        buffer,
                    )?;
                }
            }
            PATTERN_MONTH => {
                if use_date_format_symbols {
                    if count >= 4 {
                        current = symbol(1, value);
                    } else if count == 3 {
                        current = symbol(2, value);
                    }
                } else if count < 3 {
                    current = None;
                } else if self.force_standalone_form {
                    current = calendar.get_display_name(field, style | 0x8000, &self.locale)?;
                    if current.is_none() {
                        current = calendar.get_display_name(field, style, &self.locale)?;
                    }
                }
                if current.is_none() {
                    self.zero_padding_number(value + 1, count_i, max_int_count, buffer)?;
                }
            }
            PATTERN_MONTH_STANDALONE => {
                if count >= 3 {
                    current = calendar.get_display_name(field, style | 0x8000, &self.locale)?;
                }
                if current.is_none() {
                    self.zero_padding_number(value + 1, count_i, max_int_count, buffer)?;
                }
            }
            PATTERN_HOUR_OF_DAY1 => {
                if current.is_none() {
                    if value == 0 {
                        self.zero_padding_number(
                            calendar.get_maximum(HOUR_OF_DAY) + 1,
                            count_i,
                            max_int_count,
                            buffer,
                        )?;
                    } else {
                        self.zero_padding_number(value, count_i, max_int_count, buffer)?;
                    }
                }
            }
            PATTERN_DAY_OF_WEEK => {
                if use_date_format_symbols {
                    if count >= 4 {
                        current = symbol(3, value);
                    } else {
                        current = symbol(4, value);
                    }
                }
            }
            PATTERN_AM_PM => {
                if use_date_format_symbols {
                    current = symbol(5, value);
                }
            }
            PATTERN_HOUR1 => {
                if current.is_none() {
                    if value == 0 {
                        self.zero_padding_number(
                            calendar.get_least_maximum(HOUR) + 1,
                            count_i,
                            max_int_count,
                            buffer,
                        )?;
                    } else {
                        self.zero_padding_number(value, count_i, max_int_count, buffer)?;
                    }
                }
            }
            PATTERN_ZONE_NAME => {
                if current.is_none() {
                    return Err(LuceneError::unsupported_operation(
                        "SimpleDateFormat: formatting the time-zone name field 'z' is not emulated"
                            .to_string(),
                    ));
                }
            }
            PATTERN_ZONE_VALUE => {
                value = (calendar
                    .get(ZONE_OFFSET)?
                    .wrapping_add(calendar.get(DST_OFFSET)?))
                    / 60000;
                let mut width = 4;
                if value >= 0 {
                    buffer.push(u16::from(b'+'));
                } else {
                    width += 1;
                }
                let num = (value / 60) * 100 + (value % 60);
                sprintf0d(buffer, num, width);
            }
            PATTERN_ISO_ZONE => {
                value = calendar
                    .get(ZONE_OFFSET)?
                    .wrapping_add(calendar.get(DST_OFFSET)?);
                if value == 0 {
                    buffer.push(u16::from(b'Z'));
                } else {
                    value /= 60000;
                    if value >= 0 {
                        buffer.push(u16::from(b'+'));
                    } else {
                        buffer.push(u16::from(b'-'));
                        value = -value;
                    }
                    sprintf0d(buffer, value / 60, 2);
                    if count != 1 {
                        if count == 3 {
                            buffer.push(u16::from(b':'));
                        }
                        sprintf0d(buffer, value % 60, 2);
                    }
                }
            }
            _ => {
                // 'd', 'H', 'm', 's', 'S', 'D', 'F', 'w', 'W', 'K', 'u'
                if current.is_none() {
                    self.zero_padding_number(value, count_i, max_int_count, buffer)?;
                }
            }
        }
        if let Some(c) = current {
            buffer.extend(c.encode_utf16());
        }
        let field_id = PATTERN_INDEX_TO_DATE_FORMAT_FIELD[pattern_char_index];
        delegate.formatted(
            field_id,
            FormatField::Date(field_id),
            begin_offset,
            buffer.len(),
        );
        Ok(())
    }

    /// `zeroPaddingNumber(value, minDigits, maxDigits, buffer)`.
    fn zero_padding_number(
        &self,
        value: i32,
        min_digits: i32,
        max_digits: i32,
        buffer: &mut Vec<u16>,
    ) -> Result<()> {
        let zero = self.zero_digit;
        let digit = |v: i32| (i32::from(zero) + v) as u16;
        if value >= 0 {
            if value < 100 && (1..=2).contains(&min_digits) {
                if value < 10 {
                    if min_digits == 2 {
                        buffer.push(zero);
                    }
                    buffer.push(digit(value));
                } else {
                    buffer.push(digit(value / 10));
                    buffer.push(digit(value % 10));
                }
                return Ok(());
            } else if (1000..10000).contains(&value) {
                if min_digits == 4 {
                    let mut v = value;
                    buffer.push(digit(v / 1000));
                    v %= 1000;
                    buffer.push(digit(v / 100));
                    v %= 100;
                    buffer.push(digit(v / 10));
                    buffer.push(digit(v % 10));
                    return Ok(());
                }
                if min_digits == 2 && max_digits == 2 {
                    return self.zero_padding_number(value % 100, 2, 2, buffer);
                }
            }
        }
        let mut nf = self.number_format.clone();
        nf.set_minimum_integer_digits(min_digits);
        nf.set_maximum_integer_digits(max_digits);
        let mut pos = FieldPosition::dont_care();
        nf.format_long_to(i64::from(value), buffer, &mut pos)
    }

    // ------------------------------------------------------------- parse

    /// `DateFormat.parse(String)`.
    ///
    /// # Errors
    ///
    /// The `java.text.ParseException` `Unparseable date: "source"` (with the
    /// error index) when nothing is parsed from the start of `source`, and
    /// the errors of [`parse_position`](Self::parse_position).
    pub fn parse(&self, source: &str) -> Result<Date> {
        let mut pos = ParsePosition::new(0);
        let result = self.parse_position(source, &mut pos)?;
        if pos.index == 0 {
            return Err(parse_exception(
                format!("Unparseable date: \"{source}\""),
                pos.error_index,
            ));
        }
        Ok(result.expect("INVARIANT: a parse that advanced returns a date"))
    }

    /// `SimpleDateFormat.parse(String, ParsePosition)`: the date, `None`
    /// where Java returns `null`; `pos` is updated as Java updates it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a display-names map the JDK cannot
    /// merge, and `UnsupportedOperationException` for a week date (see the
    /// module documentation).
    pub fn parse_position(&self, text: &str, pos: &mut ParsePosition) -> Result<Option<Date>> {
        let text: Vec<u16> = text.encode_utf16().collect();
        let mut state = self.state();
        let calendar = &mut state.calendar;
        let mut start = pos.index;
        let old_start = start;
        let text_length = text.len();
        let mut ambiguous_year = false;
        let mut calb = CalendarBuilder::new();
        let lenient = calendar.is_lenient();
        for (i, element) in self.compiled.iter().enumerate() {
            match element {
                Element::QuoteAsciiChar(c) => {
                    if start >= text_length || !char_equals(text[start], *c, lenient) {
                        pos.index = old_start;
                        pos.error_index = start as i64;
                        return Ok(None);
                    }
                    start += 1;
                }
                Element::QuoteChars(chars) => {
                    for c in chars {
                        if start >= text_length || !char_equals(text[start], *c, lenient) {
                            pos.index = old_start;
                            pos.error_index = start as i64;
                            return Ok(None);
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
                        if self.has_following_minus_sign {
                            let next_count = match next {
                                Element::QuoteAsciiChar(c) => Some(*c),
                                Element::QuoteChars(chars) if chars.len() < 255 => Some(chars[0]),
                                Element::QuoteChars(chars) => Some((chars.len() >> 16) as u16),
                                Element::Field(..) => None,
                            };
                            if next_count == Some(self.minus_sign) {
                                use_following_minus_sign_as_delimiter = true;
                            }
                        }
                    }
                    match self.sub_parse(
                        calendar,
                        &text,
                        start,
                        *tag,
                        *count,
                        obey_count,
                        &mut ambiguous_year,
                        pos,
                        use_following_minus_sign_as_delimiter,
                        &mut calb,
                    )? {
                        Some(next) => start = next,
                        None => {
                            pos.index = old_start;
                            return Ok(None);
                        }
                    }
                }
            }
        }
        pos.index = start;
        let parsed: Result<Date> = (|| {
            calb.establish(calendar)?;
            let mut parsed_date = calendar.get_time()?;
            if ambiguous_year && parsed_date.before(&self.default_century_start) {
                calb.add_year(100).establish(calendar)?;
                parsed_date = calendar.get_time()?;
            }
            Ok(parsed_date)
        })();
        match parsed {
            Ok(date) => Ok(Some(date)),
            Err(e) if e.is_illegal_argument_exception() => {
                pos.error_index = start as i64;
                pos.index = old_start;
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// The display names of `(field, style)` (`Calendar.getDisplayNames`).
    fn display_names(&self, field: i32, style: i32) -> Option<&NamesMap> {
        let index = data()
            .combos
            .iter()
            .position(|&c| c == (field, style))
            .expect("INVARIANT: the data holds every map parsing asks for");
        self.names[index].as_ref()
    }

    /// `getDisplayNamesMap(field, locale)`: the short-format map with the
    /// other short and long styles put into it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `putAll` into a `null` short-format map.
    fn display_names_map(&self, field: i32) -> Result<Option<NamesMap>> {
        let mut map = self.display_names(field, SHORT_FORMAT).cloned();
        for style in REST_OF_STYLES {
            if let Some(m) = self.display_names(field, style) {
                map.as_mut()
                    .ok_or_else(LuceneError::null_pointer)?
                    .put_all(&m.entries);
            }
        }
        Ok(map)
    }

    /// `getDisplayContextNamesMap(field, locale)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `putAll` into a `null` short map.
    fn display_context_names_map(&self, field: i32) -> Result<Option<NamesMap>> {
        let (short, long) = if self.force_standalone_form {
            (SHORT_STANDALONE, LONG_STANDALONE)
        } else {
            (SHORT_FORMAT, LONG_FORMAT)
        };
        let mut map = self.display_names(field, short).cloned();
        if let Some(m) = self.display_names(field, long) {
            map.as_mut()
                .ok_or_else(LuceneError::null_pointer)?
                .put_all(&m.entries);
        }
        Ok(map)
    }

    /// `matchString(text, start, field, String[] data, calb)`.
    fn match_string_array(
        &self,
        text: &[u16],
        start: usize,
        field: usize,
        array: usize,
        calb: &mut CalendarBuilder,
    ) -> i64 {
        let data = &self.symbols[array];
        let first = if field == DAY_OF_WEEK { 1 } else { 0 };
        let mut best_match_length = 0;
        let mut best_match: Option<usize> = None;
        for (i, name) in data.iter().enumerate().skip(first) {
            let name: Vec<u16> = name.as_deref().unwrap_or("").encode_utf16().collect();
            let length = name.len();
            if length > best_match_length && region_matches_ci(text, start, &name) {
                best_match = Some(i);
                best_match_length = length;
            }
        }
        match best_match {
            Some(i) => {
                calb.set(field, i as i32);
                (start + best_match_length) as i64
            }
            None => -(start as i64),
        }
    }

    /// `matchString(text, start, field, Map data, calb)`.
    fn match_string_map(
        text: &[u16],
        start: usize,
        field: usize,
        data: Option<&NamesMap>,
        calb: &mut CalendarBuilder,
    ) -> i64 {
        let Some(data) = data else {
            return -(start as i64);
        };
        if data.sorted {
            for (name, value) in &data.entries {
                if region_matches_ci(text, start, name) {
                    calb.set(field, *value);
                    return (start + name.len()) as i64;
                }
            }
            return -(start as i64);
        }
        let mut best: Option<&(Vec<u16>, i32)> = None;
        for entry in &data.entries {
            let length = entry.0.len();
            if best.is_none_or(|b| length > b.0.len()) && region_matches_ci(text, start, &entry.0) {
                best = Some(entry);
            }
        }
        match best {
            Some((name, value)) => {
                calb.set(field, *value);
                (start + name.len()) as i64
            }
            None => -(start as i64),
        }
    }

    /// The integer `DecimalFormat.parse(text, pos)`.
    fn parse_number(&self, text: &[u16], pos: &mut ParsePosition) -> Option<i32> {
        self.number_format
            .parse_units(text, pos)
            .map(|n| n.int_value())
    }

    /// `SimpleDateFormat.subParse`: the index after the field, `None`
    /// (Java's negative result) on failure, with `orig_pos.error_index` set.
    #[allow(clippy::too_many_arguments)]
    fn sub_parse(
        &self,
        calendar: &mut Calendar,
        text: &[u16],
        start: usize,
        pattern_char_index: usize,
        count: usize,
        obey_count: bool,
        ambiguous_year: &mut bool,
        orig_pos: &mut ParsePosition,
        use_following_minus_sign_as_delimiter: bool,
        calb: &mut CalendarBuilder,
    ) -> Result<Option<usize>> {
        let mut value: i32 = 0;
        let mut pos = ParsePosition::new(start);
        let mut pattern_char_index = pattern_char_index;
        if pattern_char_index == PATTERN_WEEK_YEAR && !calendar.is_week_date_supported() {
            pattern_char_index = PATTERN_YEAR;
        }
        let field = PATTERN_INDEX_TO_CALENDAR_FIELD[pattern_char_index];
        let gregorian = calendar.kind() != CalendarKind::Japanese;
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
        let actual_start = pos.index;
        let matched = |index: i64| (index > 0).then_some(index as usize);
        'parsing: {
            if pattern_char_index == PATTERN_HOUR_OF_DAY1
                || pattern_char_index == PATTERN_HOUR1
                || (pattern_char_index == PATTERN_MONTH && count <= 2)
                || (pattern_char_index == PATTERN_MONTH_STANDALONE && count <= 2)
                || pattern_char_index == PATTERN_YEAR
                || pattern_char_index == PATTERN_WEEK_YEAR
            {
                let number = if obey_count {
                    if start + count > text.len() {
                        break 'parsing;
                    }
                    self.parse_number(&text[..start + count], &mut pos)
                } else {
                    self.parse_number(text, &mut pos)
                };
                match number {
                    None => {
                        if pattern_char_index != PATTERN_YEAR || gregorian {
                            break 'parsing;
                        }
                    }
                    Some(v) => {
                        value = v;
                        if use_following_minus_sign_as_delimiter
                            && value < 0
                            && ((pos.index < text.len() && text[pos.index] != self.minus_sign)
                                || (pos.index == text.len()
                                    && text[pos.index - 1] == self.minus_sign))
                        {
                            value = value.wrapping_neg();
                            pos.index -= 1;
                        }
                    }
                }
            }
            let use_date_format_symbols = self.use_date_format_symbols();
            match pattern_char_index {
                PATTERN_ERA => {
                    if use_date_format_symbols {
                        if let Some(index) =
                            matched(self.match_string_array(text, start, C_ERA, 0, calb))
                        {
                            return Ok(Some(index));
                        }
                    } else {
                        let map = self.display_names_map(ERA)?;
                        if let Some(index) = matched(Self::match_string_map(
                            text,
                            start,
                            field,
                            map.as_ref(),
                            calb,
                        )) {
                            return Ok(Some(index));
                        }
                    }
                    break 'parsing;
                }
                PATTERN_WEEK_YEAR | PATTERN_YEAR => {
                    if !gregorian {
                        let style = if count >= 4 {
                            LONG_FORMAT
                        } else {
                            SHORT_FORMAT
                        };
                        if let Some(map) = self.display_names(YEAR, style) {
                            if let Some(index) =
                                matched(Self::match_string_map(text, start, field, Some(map), calb))
                            {
                                return Ok(Some(index));
                            }
                        }
                        calb.set(field, value);
                        return Ok(Some(pos.index));
                    }
                    if count <= 2
                        && (pos.index - actual_start) == 2
                        && is_java_digit(text[actual_start])
                        && is_java_digit(text[actual_start + 1])
                    {
                        let ambiguous_two_digit_year = self.default_century_start_year % 100;
                        *ambiguous_year = value == ambiguous_two_digit_year;
                        value += (self.default_century_start_year / 100) * 100
                            + if value < ambiguous_two_digit_year {
                                100
                            } else {
                                0
                            };
                    }
                    calb.set(field, value);
                    return Ok(Some(pos.index));
                }
                PATTERN_MONTH => {
                    if count <= 2 {
                        calb.set(C_MONTH, value.wrapping_sub(1));
                        return Ok(Some(pos.index));
                    }
                    if use_date_format_symbols {
                        if let Some(index) =
                            matched(self.match_string_array(text, start, C_MONTH, 1, calb))
                        {
                            return Ok(Some(index));
                        }
                        if let Some(index) =
                            matched(self.match_string_array(text, start, C_MONTH, 2, calb))
                        {
                            return Ok(Some(index));
                        }
                    } else {
                        let map = self.display_context_names_map(MONTH)?;
                        if let Some(index) = matched(Self::match_string_map(
                            text,
                            start,
                            field,
                            map.as_ref(),
                            calb,
                        )) {
                            return Ok(Some(index));
                        }
                    }
                    break 'parsing;
                }
                PATTERN_MONTH_STANDALONE => {
                    if count <= 2 {
                        calb.set(C_MONTH, value.wrapping_sub(1));
                        return Ok(Some(pos.index));
                    }
                    let map = self.display_names_map(MONTH)?;
                    if let Some(index) = matched(Self::match_string_map(
                        text,
                        start,
                        field,
                        map.as_ref(),
                        calb,
                    )) {
                        return Ok(Some(index));
                    }
                    break 'parsing;
                }
                PATTERN_HOUR_OF_DAY1 => {
                    if !calendar.is_lenient() && !(1..=24).contains(&value) {
                        break 'parsing;
                    }
                    if value == calendar.get_maximum(HOUR_OF_DAY) + 1 {
                        value = 0;
                    }
                    calb.set(HOUR_OF_DAY, value);
                    return Ok(Some(pos.index));
                }
                PATTERN_DAY_OF_WEEK => {
                    if use_date_format_symbols {
                        if let Some(index) =
                            matched(self.match_string_array(text, start, DAY_OF_WEEK, 3, calb))
                        {
                            return Ok(Some(index));
                        }
                        if let Some(index) =
                            matched(self.match_string_array(text, start, DAY_OF_WEEK, 4, calb))
                        {
                            return Ok(Some(index));
                        }
                    } else {
                        for style in [LONG_FORMAT, SHORT_FORMAT] {
                            let map = self.display_names(D_DAY_OF_WEEK, style);
                            if let Some(index) =
                                matched(Self::match_string_map(text, start, field, map, calb))
                            {
                                return Ok(Some(index));
                            }
                        }
                    }
                    break 'parsing;
                }
                PATTERN_AM_PM => {
                    if use_date_format_symbols {
                        if let Some(index) =
                            matched(self.match_string_array(text, start, C_AM_PM, 5, calb))
                        {
                            return Ok(Some(index));
                        }
                    } else {
                        let map = self.display_names_map(AM_PM)?;
                        if let Some(index) = matched(Self::match_string_map(
                            text,
                            start,
                            field,
                            map.as_ref(),
                            calb,
                        )) {
                            return Ok(Some(index));
                        }
                    }
                    break 'parsing;
                }
                PATTERN_HOUR1 => {
                    if !calendar.is_lenient() && !(1..=12).contains(&value) {
                        break 'parsing;
                    }
                    if value == calendar.get_least_maximum(HOUR) + 1 {
                        value = 0;
                    }
                    calb.set(HOUR, value);
                    return Ok(Some(pos.index));
                }
                PATTERN_ZONE_NAME | PATTERN_ZONE_VALUE => {
                    if let Some(index) = self.sub_parse_zone(calendar, text, start, &mut pos, calb)
                    {
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
                        calb.set(ZONE_OFFSET, 0).set(DST_OFFSET, 0);
                        pos.index += 1;
                        return Ok(Some(pos.index));
                    }
                    let sign = if c == u16::from(b'+') {
                        1
                    } else if c == u16::from(b'-') {
                        -1
                    } else {
                        pos.index += 1;
                        break 'parsing;
                    };
                    pos.index += 1;
                    match sub_parse_numeric_zone(text, pos.index, sign, count, count == 3, calb) {
                        Ok(i) => return Ok(Some(i)),
                        Err(i) => {
                            pos.index = i;
                            break 'parsing;
                        }
                    }
                }
                _ => {
                    // Handle "generic" fields
                    let number = if obey_count {
                        if start + count > text.len() {
                            break 'parsing;
                        }
                        self.parse_number(&text[..start + count], &mut pos)
                    } else {
                        self.parse_number(text, &mut pos)
                    };
                    if let Some(v) = number {
                        value = v;
                        if use_following_minus_sign_as_delimiter
                            && value < 0
                            && ((pos.index < text.len() && text[pos.index] != self.minus_sign)
                                || (pos.index == text.len()
                                    && text[pos.index - 1] == self.minus_sign))
                        {
                            value = value.wrapping_neg();
                            pos.index -= 1;
                        }
                        calb.set(field, value);
                        return Ok(Some(pos.index));
                    }
                    break 'parsing;
                }
            }
        }
        // Parsing failed.
        orig_pos.error_index = pos.index as i64;
        Ok(None)
    }

    /// The `z` and `Z` fields: a `GMT` form, an RFC 822 offset, or a zone
    /// name (`subParseZoneString`).
    fn sub_parse_zone(
        &self,
        calendar: &mut Calendar,
        text: &[u16],
        start: usize,
        pos: &mut ParsePosition,
        calb: &mut CalendarBuilder,
    ) -> Option<usize> {
        const GMT: [u16; 3] = [b'G' as u16, b'M' as u16, b'T' as u16];
        // Java catches the IndexOutOfBoundsException of `charAt`.
        let mut c = *text.get(pos.index)?;
        let mut sign = 0;
        if c == u16::from(b'+') {
            sign = 1;
        } else if c == u16::from(b'-') {
            sign = -1;
        }
        if sign == 0 {
            if (c == u16::from(b'G') || c == u16::from(b'g'))
                && (text.len() - start) >= GMT.len()
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
                    calb.set(ZONE_OFFSET, 0).set(DST_OFFSET, 0);
                    return Some(pos.index);
                }
                pos.index += 1;
                match sub_parse_numeric_zone(text, pos.index, sign, 0, true, calb) {
                    Ok(i) => Some(i),
                    Err(i) => {
                        pos.index = i;
                        None
                    }
                }
            } else {
                match self.sub_parse_zone_string(calendar, text, pos.index, calb) {
                    Ok(i) => Some(i),
                    Err(i) => {
                        pos.index = i;
                        None
                    }
                }
            }
        } else {
            pos.index += 1;
            match sub_parse_numeric_zone(text, pos.index, sign, 0, false, calb) {
                Ok(i) => Some(i),
                Err(i) => {
                    pos.index = i;
                    None
                }
            }
        }
    }

    /// `SimpleDateFormat.subParseZoneString`: `Ok(index after the name)`, or
    /// `Err(start)` where Java returns `-start`.
    fn sub_parse_zone_string(
        &self,
        calendar: &mut Calendar,
        text: &[u16],
        start: usize,
        calb: &mut CalendarBuilder,
    ) -> std::result::Result<usize, usize> {
        let zones = self
            .zones
            .as_ref()
            .expect("INVARIANT: built for a pattern with a zone field");
        let current_time_zone = calendar.get_time_zone().clone();
        let zone_index = |id: &str| zones.iter().position(|z| z.id == id);
        let mut use_same_name = false;
        let mut found: Option<(usize, usize)> = None;
        let try_row = |i: usize, use_same_name: &mut bool| -> Option<(usize, usize)> {
            let n = match_zone_string(text, start, &zones[i].names)?;
            let name_index = n + 1;
            if name_index <= 2 {
                *use_same_name = equals_ignore_case(&zones[i].names[n], &zones[i].names[n + 2]);
            }
            Some((i, name_index))
        };
        if let Some(i) = zone_index(current_time_zone.get_id()) {
            found = try_row(i, &mut use_same_name);
        }
        if found.is_none() {
            if let Some(i) = zone_index(TimeZone::get_default().get_id()) {
                found = try_row(i, &mut use_same_name);
            }
        }
        if found.is_none() {
            for i in 0..zones.len() {
                if let Some(f) = try_row(i, &mut use_same_name) {
                    found = Some(f);
                    break;
                }
            }
        }
        let Some((i, name_index)) = found else {
            return Err(start);
        };
        let tz = TimeZone::get_time_zone(zones[i].id);
        if tz != current_time_zone {
            calendar.set_time_zone(&tz);
        }
        let dst_amount = if name_index >= 3 {
            tz.get_dst_savings()
        } else {
            0
        };
        if !(use_same_name || (name_index >= 3 && dst_amount == 0)) {
            calb.clear(ZONE_OFFSET).set(DST_OFFSET, dst_amount);
        }
        Ok(start + zones[i].names[name_index - 1].len())
    }
}

/// `CalendarUtils.sprintf0d(sb, value, width)`.
fn sprintf0d(buffer: &mut Vec<u16>, value: i32, width: i32) {
    let mut d = i64::from(value);
    let mut width = width;
    if d < 0 {
        buffer.push(u16::from(b'-'));
        d = -d;
        width -= 1;
    }
    let mut n: i32 = 10;
    for _ in 2..width {
        n = n.wrapping_mul(10);
    }
    let mut i = 1;
    while i < width && d < i64::from(n) {
        buffer.push(u16::from(b'0'));
        n /= 10;
        i += 1;
    }
    buffer.extend(d.to_string().encode_utf16());
}

/// `SimpleDateFormat.charEquals`: equal, or (the format is lenient) both
/// space separators.
fn char_equals(ch1: u16, ch2: u16, lenient: bool) -> bool {
    ch1 == ch2
        || (lenient
            && get_type(i32::from(ch1)) == SPACE_SEPARATOR
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

/// `SimpleDateFormat.subParseNumericZone`: `Ok(index after the offset)`
/// with the offset set into `calb`, or `Err(index)` where Java returns
/// `1 - index`.
fn sub_parse_numeric_zone(
    text: &[u16],
    start: usize,
    sign: i32,
    count: usize,
    colon: bool,
    calb: &mut CalendarBuilder,
) -> std::result::Result<usize, usize> {
    let is_digit = |c: u16| (u16::from(b'0')..=u16::from(b'9')).contains(&c);
    let mut index = start;
    let result: Option<(usize, i32)> = (|| {
        let next = |index: &mut usize| -> Option<u16> {
            let c = text.get(*index).copied();
            *index += 1;
            c
        };
        let mut c = next(&mut index)?;
        if !is_digit(c) {
            return None;
        }
        let mut hours = i32::from(c - u16::from(b'0'));
        c = next(&mut index)?;
        if is_digit(c) {
            hours = hours * 10 + i32::from(c - u16::from(b'0'));
        } else {
            if count > 0 || !colon {
                return None;
            }
            index -= 1;
        }
        if hours > 23 {
            return None;
        }
        let mut minutes = 0;
        if count != 1 {
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
            minutes = i32::from(c - u16::from(b'0'));
            c = next(&mut index)?;
            if !is_digit(c) {
                return None;
            }
            minutes = minutes * 10 + i32::from(c - u16::from(b'0'));
            if minutes > 59 {
                return None;
            }
        }
        minutes += hours * 60;
        Some((index, minutes))
    })();
    match result {
        Some((index, minutes)) => {
            calb.set(ZONE_OFFSET, minutes * MILLIS_PER_MINUTE * sign)
                .set(DST_OFFSET, 0);
            Ok(index)
        }
        None => Err(index.saturating_sub(1)),
    }
}

/// `SimpleDateFormat.compile(String)`: the elements and
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
