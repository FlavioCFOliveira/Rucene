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

//! `java.util.Calendar` and the three calendars `Calendar.getInstance`
//! creates: `java.util.GregorianCalendar` (with its Julian-Gregorian cutover),
//! `sun.util.BuddhistCalendar` and `java.util.JapaneseImperialCalendar`.
//!
//! **Divergence.** An emulation of JDK 25, not a port of a Lucene file,
//! written from OpenJDK 25's `Calendar`, `GregorianCalendar`,
//! `JapaneseImperialCalendar`, `sun.util.BuddhistCalendar`,
//! `sun.util.calendar.BaseCalendar`, `Gregorian`, `JulianCalendar`,
//! `LocalGregorianCalendar`, `Era` and `CalendarUtils` (read as
//! specification; no JDK code is reproduced), over the time zones of
//! [`time_zone`](super::time_zone) and the week data of
//! [`locale_data`](super::locale_data). The field model is Java's: the field
//! values, their set-stamps (`UNSET`, `COMPUTED`, then one stamp per `set`),
//! `isTimeSet`, `areFieldsSet` and `areAllFieldsSet`, so that the field
//! resolution of `computeTime` (`selectFields`) and the lenient normalization
//! give Java's results. What is not reproduced, each stated at its point:
//!
//! * the `BaseCalendar.Date` caches of the JDK (`cachedFixedDate`, the
//!   year cache), which change no result;
//! * the system property `jdk.calendar.japanese.supplemental.era`: the eras
//!   are the five of JDK 25 (Meiji to Reiwa);
//! * `getActualMaximum` of `WEEK_OF_YEAR`, `WEEK_OF_MONTH` and
//!   `DAY_OF_WEEK_IN_MONTH`, and `getActualMinimum`/`getActualMaximum` of a
//!   Buddhist or Japanese calendar, which
//!   [`Calendar::get_actual_maximum`] and [`Calendar::get_actual_minimum`]
//!   report as an `UnsupportedOperationException`; and the week-year computation of `getWeekYear` for the years up to
//!   one after the Gregorian cutover year (its "slow path"), which
//!   [`Calendar::get_week_year`] reports as an `UnsupportedOperationException`;
//! * `getMaximum` of a `GregorianCalendar` whose cutover year is not after
//!   200 (a proleptic `iso8601` calendar), which the JDK derives from
//!   `getActualMaximum` at the cutover; the emulation answers `MAX_VALUES`.

use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};

use super::date::{current_time_millis, Date};
use super::locale::Locale;
use super::locale_data::data;
use super::time_zone::TimeZone;

/// `Calendar.ERA`.
pub const ERA: usize = 0;
/// `Calendar.YEAR`.
pub const YEAR: usize = 1;
/// `Calendar.MONTH`.
pub const MONTH: usize = 2;
/// `Calendar.WEEK_OF_YEAR`.
pub const WEEK_OF_YEAR: usize = 3;
/// `Calendar.WEEK_OF_MONTH`.
pub const WEEK_OF_MONTH: usize = 4;
/// `Calendar.DATE`.
pub const DATE: usize = 5;
/// `Calendar.DAY_OF_MONTH`.
pub const DAY_OF_MONTH: usize = 5;
/// `Calendar.DAY_OF_YEAR`.
pub const DAY_OF_YEAR: usize = 6;
/// `Calendar.DAY_OF_WEEK`.
pub const DAY_OF_WEEK: usize = 7;
/// `Calendar.DAY_OF_WEEK_IN_MONTH`.
pub const DAY_OF_WEEK_IN_MONTH: usize = 8;
/// `Calendar.AM_PM`.
pub const AM_PM: usize = 9;
/// `Calendar.HOUR`.
pub const HOUR: usize = 10;
/// `Calendar.HOUR_OF_DAY`.
pub const HOUR_OF_DAY: usize = 11;
/// `Calendar.MINUTE`.
pub const MINUTE: usize = 12;
/// `Calendar.SECOND`.
pub const SECOND: usize = 13;
/// `Calendar.MILLISECOND`.
pub const MILLISECOND: usize = 14;
/// `Calendar.ZONE_OFFSET`.
pub const ZONE_OFFSET: usize = 15;
/// `Calendar.DST_OFFSET`.
pub const DST_OFFSET: usize = 16;
/// `Calendar.FIELD_COUNT`.
pub const FIELD_COUNT: usize = 17;

/// `Calendar.SUNDAY`.
pub const SUNDAY: i32 = 1;
/// `Calendar.MONDAY`.
pub const MONDAY: i32 = 2;
/// `Calendar.SATURDAY`.
pub const SATURDAY: i32 = 7;
/// `Calendar.JANUARY`.
pub const JANUARY: i32 = 0;
/// `Calendar.DECEMBER`.
pub const DECEMBER: i32 = 11;
/// `Calendar.AM`.
pub const AM: i32 = 0;
/// `GregorianCalendar.BC`.
pub const BC: i32 = 0;
/// `GregorianCalendar.AD`.
pub const AD: i32 = 1;

/// `Calendar.SHORT` (`SHORT_FORMAT`).
pub const SHORT: i32 = 1;
/// `Calendar.LONG` (`LONG_FORMAT`).
pub const LONG: i32 = 2;
/// `Calendar.SHORT_STANDALONE`.
pub const SHORT_STANDALONE: i32 = 0x8001;
/// `Calendar.LONG_STANDALONE`.
pub const LONG_STANDALONE: i32 = 0x8002;

const UNSET: i32 = 0;
const COMPUTED: i32 = 1;
const MINIMUM_USER_STAMP: i32 = 2;
const ALL_FIELDS: i32 = (1 << FIELD_COUNT) - 1;

const fn mask(field: usize) -> i32 {
    1 << field
}

const ERA_MASK: i32 = mask(ERA);
const YEAR_MASK: i32 = mask(YEAR);
const MONTH_MASK: i32 = mask(MONTH);
const WEEK_OF_YEAR_MASK: i32 = mask(WEEK_OF_YEAR);
const WEEK_OF_MONTH_MASK: i32 = mask(WEEK_OF_MONTH);
const DAY_OF_MONTH_MASK: i32 = mask(DAY_OF_MONTH);
const DAY_OF_YEAR_MASK: i32 = mask(DAY_OF_YEAR);
const DAY_OF_WEEK_MASK: i32 = mask(DAY_OF_WEEK);
const DAY_OF_WEEK_IN_MONTH_MASK: i32 = mask(DAY_OF_WEEK_IN_MONTH);
const AM_PM_MASK: i32 = mask(AM_PM);
const HOUR_MASK: i32 = mask(HOUR);
const HOUR_OF_DAY_MASK: i32 = mask(HOUR_OF_DAY);
const MINUTE_MASK: i32 = mask(MINUTE);
const SECOND_MASK: i32 = mask(SECOND);
const MILLISECOND_MASK: i32 = mask(MILLISECOND);
const ZONE_OFFSET_MASK: i32 = mask(ZONE_OFFSET);
const DST_OFFSET_MASK: i32 = mask(DST_OFFSET);

/// `Calendar.FIELD_NAME`.
const FIELD_NAME: [&str; FIELD_COUNT] = [
    "ERA",
    "YEAR",
    "MONTH",
    "WEEK_OF_YEAR",
    "WEEK_OF_MONTH",
    "DAY_OF_MONTH",
    "DAY_OF_YEAR",
    "DAY_OF_WEEK",
    "DAY_OF_WEEK_IN_MONTH",
    "AM_PM",
    "HOUR",
    "HOUR_OF_DAY",
    "MINUTE",
    "SECOND",
    "MILLISECOND",
    "ZONE_OFFSET",
    "DST_OFFSET",
];

const ONE_SECOND: i32 = 1_000;
const ONE_MINUTE: i32 = 60 * ONE_SECOND;
const ONE_HOUR: i32 = 60 * ONE_MINUTE;
const ONE_DAY: i64 = 24 * ONE_HOUR as i64;
/// `AbstractCalendar.EPOCH_OFFSET`: the fixed date of 1970-01-01 (Gregorian).
const EPOCH_OFFSET: i64 = 719_163;
/// `GregorianCalendar.EPOCH_YEAR`.
const EPOCH_YEAR: i32 = 1970;
/// `JulianCalendar.JULIAN_EPOCH`: the fixed date of 0001-01-01 (Julian).
const JULIAN_EPOCH: i64 = -1;
/// `GregorianCalendar.DEFAULT_GREGORIAN_CUTOVER`: 1582-10-15T00:00Z.
const DEFAULT_GREGORIAN_CUTOVER: i64 = -12_219_292_800_000;

/// `GregorianCalendar.MONTH_LENGTH`.
const MONTH_LENGTH: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
/// `GregorianCalendar.LEAP_MONTH_LENGTH`.
const LEAP_MONTH_LENGTH: [i32; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
/// `BaseCalendar.DAYS_IN_MONTH`, indexed from 0 (December of the year
/// before) to 12.
const DAYS_IN_MONTH: [i32; 13] = [31, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
/// `BaseCalendar.ACCUMULATED_DAYS_IN_MONTH`.
const ACCUMULATED_DAYS_IN_MONTH: [i32; 13] =
    [-30, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
/// `BaseCalendar.ACCUMULATED_DAYS_IN_MONTH_LEAP`.
const ACCUMULATED_DAYS_IN_MONTH_LEAP: [i32; 13] =
    [-30, 0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];

/// `GregorianCalendar.MIN_VALUES` (the Buddhist calendar shares it).
const GREGORIAN_MIN_VALUES: [i32; FIELD_COUNT] = [
    BC,
    1,
    JANUARY,
    1,
    0,
    1,
    1,
    SUNDAY,
    1,
    AM,
    0,
    0,
    0,
    0,
    0,
    -13 * ONE_HOUR,
    0,
];
/// `GregorianCalendar.LEAST_MAX_VALUES`.
const GREGORIAN_LEAST_MAX_VALUES: [i32; FIELD_COUNT] = [
    AD,
    292_269_054,
    DECEMBER,
    52,
    4,
    28,
    365,
    SATURDAY,
    4,
    1,
    11,
    23,
    59,
    59,
    999,
    14 * ONE_HOUR,
    20 * ONE_MINUTE,
];
/// `GregorianCalendar.MAX_VALUES`.
const GREGORIAN_MAX_VALUES: [i32; FIELD_COUNT] = [
    AD,
    292_278_994,
    DECEMBER,
    53,
    6,
    31,
    366,
    SATURDAY,
    6,
    1,
    11,
    23,
    59,
    59,
    999,
    14 * ONE_HOUR,
    2 * ONE_HOUR,
];
/// `JapaneseImperialCalendar.MIN_VALUES`.
const JAPANESE_MIN_VALUES: [i32; FIELD_COUNT] = [
    0,
    -292_275_055,
    JANUARY,
    1,
    0,
    1,
    1,
    SUNDAY,
    1,
    AM,
    0,
    0,
    0,
    0,
    0,
    -13 * ONE_HOUR,
    0,
];
/// `JapaneseImperialCalendar.MAX_VALUES`, its `ERA` entry as the class
/// initializer sets it (`eras.length - 1`).
const JAPANESE_MAX_VALUES: [i32; FIELD_COUNT] = [
    JAPANESE_ERA_COUNT as i32,
    292_278_994,
    DECEMBER,
    53,
    6,
    31,
    366,
    SATURDAY,
    6,
    1,
    11,
    23,
    59,
    59,
    999,
    14 * ONE_HOUR,
    2 * ONE_HOUR,
];

/// One era of `LocalGregorianCalendar.JAPANESE_ERAS`: its start, a local
/// time in milliseconds, and the Gregorian date of it (`getSinceDate()`).
struct JapaneseEra {
    since: i64,
    since_year: i32,
    since_month: i32,
    since_day: i32,
}

/// The number of eras of `LocalGregorianCalendar.JAPANESE_ERAS`.
const JAPANESE_ERA_COUNT: usize = 5;

/// `LocalGregorianCalendar.JAPANESE_ERAS`: Meiji, Taisho, Showa, Heisei,
/// Reiwa, all local-time eras starting at midnight.
static JAPANESE_ERAS: LazyLock<[JapaneseEra; JAPANESE_ERA_COUNT]> = LazyLock::new(|| {
    [
        -3_218_832_000_000,
        -1_812_153_600_000,
        -1_357_603_200_000,
        600_220_800_000,
        1_556_668_800_000,
    ]
    .map(|since: i64| {
        // `Era(name, abbr, since, localTime)`: `gCal.getCalendarDate(since,
        // d)` without a time zone.
        let fd = since.div_euclid(ONE_DAY) + EPOCH_OFFSET;
        let d = gregorian_date_from_fixed(fd);
        JapaneseEra {
            since,
            since_year: d.year,
            since_month: d.month,
            since_day: d.day_of_month,
        }
    })
});

/// `JapaneseImperialCalendar.BEFORE_MEIJI` .. `REIWA`: the indices of its
/// `eras` array, whose entry 0 is `BEFORE_MEIJI_ERA`.
const SHOWA: i32 = 3;
const REIWA: i32 = 5;

/// `JapaneseImperialCalendar.sinceFixedDates`: the fixed date each era
/// starts on, entry 0 being `BEFORE_MEIJI_ERA`'s
/// (`gcal.getFixedDate(Long.MIN_VALUE's date)`).
static SINCE_FIXED_DATES: LazyLock<[i64; JAPANESE_ERA_COUNT + 1]> = LazyLock::new(|| {
    let mut r = [0i64; JAPANESE_ERA_COUNT + 1];
    r[0] = i64::MIN.div_euclid(ONE_DAY) + EPOCH_OFFSET;
    for (i, e) in JAPANESE_ERAS.iter().enumerate() {
        r[i + 1] = gregorian_fixed_date(e.since_year, e.since_month, e.since_day);
    }
    r
});

/// `JapaneseImperialCalendar.currentEra`: the index of the last era that has
/// started (`since < System.currentTimeMillis()`), computed once, as the
/// class initializer computes it.
static CURRENT_ERA: LazyLock<i32> = LazyLock::new(|| {
    let now = current_time_millis();
    let mut current = 0;
    for (i, e) in JAPANESE_ERAS.iter().enumerate() {
        if e.since < now {
            current = i as i32 + 1;
        }
    }
    current
});

/// `JapaneseImperialCalendar.LEAST_MAX_VALUES`, with the `ERA`, `YEAR` and
/// `DAY_OF_YEAR` entries the class initializer computes.
static JAPANESE_LEAST_MAX_VALUES: LazyLock<[i32; FIELD_COUNT]> = LazyLock::new(|| {
    let mut v = [
        0,
        0,
        JANUARY,
        0,
        4,
        28,
        0,
        SATURDAY,
        4,
        1,
        11,
        23,
        59,
        59,
        999,
        14 * ONE_HOUR,
        20 * ONE_MINUTE,
    ];
    v[ERA] = JAPANESE_ERA_COUNT as i32;
    let mut year = i32::MAX;
    let mut day_of_year = i32::MAX;
    for i in 1..=JAPANESE_ERA_COUNT {
        let fd = SINCE_FIXED_DATES[i];
        let e = &JAPANESE_ERAS[i - 1];
        let fdd = gregorian_fixed_date(e.since_year, 1, 1);
        if fd != fdd {
            day_of_year = day_of_year.min((fd - fdd) as i32 + 1);
        }
        let fdd = gregorian_fixed_date(e.since_year, 12, 31);
        if fd != fdd {
            day_of_year = day_of_year.min((fdd - fd) as i32 + 1);
        }
        let lgd = local_date_from_fixed(fd - 1);
        let mut y = lgd.year;
        if !(lgd.month == 1 && lgd.day_of_month == 1) {
            y -= 1;
        }
        year = year.min(y);
    }
    v[YEAR] = year;
    v[DAY_OF_YEAR] = day_of_year;
    v
});

// ---------------------------------------------------------------- arithmetic

/// `CalendarUtils.floorDivide(long, long)`.
fn floor_divide_l(n: i64, d: i64) -> i64 {
    if n >= 0 {
        n / d
    } else {
        ((n + 1) / d) - 1
    }
}

/// `CalendarUtils.floorDivide(int, int)`.
fn floor_divide(n: i32, d: i32) -> i32 {
    if n >= 0 {
        n / d
    } else {
        ((n + 1) / d) - 1
    }
}

/// `CalendarUtils.floorDivide(int, int, int[])`: the quotient and `mod(n, d)`.
fn floor_divide_rem(n: i32, d: i32) -> (i32, i32) {
    if n >= 0 {
        return (n / d, n % d);
    }
    let q = ((n + 1) / d) - 1;
    (q, n - (q * d))
}

/// `CalendarUtils.mod(long, long)`.
fn mod_l(x: i64, y: i64) -> i64 {
    x - y * floor_divide_l(x, y)
}

/// `CalendarUtils.mod(int, int)`.
fn mod_i(x: i32, y: i32) -> i32 {
    x - y * floor_divide(x, y)
}

/// `CalendarUtils.isGregorianLeapYear(int)`.
fn is_gregorian_leap_year(year: i32) -> bool {
    (year % 4 == 0) && ((year % 100 != 0) || (year % 400 == 0))
}

/// `CalendarUtils.isJulianLeapYear(int)`.
fn is_julian_leap_year(normalized_julian_year: i32) -> bool {
    (normalized_julian_year % 4) == 0
}

/// `BaseCalendar.getDayOfWeekFromFixedDate(long)`.
fn day_of_week_from_fixed(fixed_date: i64) -> i32 {
    if fixed_date >= 0 {
        return (fixed_date % 7) as i32 + SUNDAY;
    }
    mod_l(fixed_date, 7) as i32 + SUNDAY
}

/// `AbstractCalendar.getDayOfWeekDateOnOrBefore(long, int)`.
fn day_of_week_date_on_or_before(fixed_date: i64, day_of_week: i32) -> i64 {
    let fd = fixed_date - i64::from(day_of_week - 1);
    if fd >= 0 {
        return fixed_date - (fd % 7);
    }
    fixed_date - mod_l(fd, 7)
}

/// A date of `BaseCalendar` (Gregorian or Julian): the normalized year, the
/// 1-based month, the day of the month, the day of the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BaseDate {
    /// `getNormalizedYear()`.
    year: i32,
    month: i32,
    day_of_month: i32,
    day_of_week: i32,
    leap_year: bool,
}

/// `Gregorian.getFixedDate(year, month, dayOfMonth, null)`.
fn gregorian_fixed_date(year: i32, month: i32, day_of_month: i32) -> i64 {
    let prevyear = i64::from(year) - 1;
    let mut days = i64::from(day_of_month);
    if prevyear >= 0 {
        days += (365 * prevyear) + (prevyear / 4) - (prevyear / 100)
            + (prevyear / 400)
            + i64::from((367 * month - 362) / 12);
    } else {
        days += (365 * prevyear) + floor_divide_l(prevyear, 4) - floor_divide_l(prevyear, 100)
            + floor_divide_l(prevyear, 400)
            + i64::from(floor_divide(367 * month - 362, 12));
    }
    if month > 2 {
        days -= if is_gregorian_leap_year(year) { 1 } else { 2 };
    }
    days
}

/// `BaseCalendar.getGregorianYearFromFixedDate(long)`.
fn gregorian_year_from_fixed(fixed_date: i64) -> i32 {
    let (n400, n100, n4, n1);
    if fixed_date > 0 {
        let d0 = fixed_date - 1;
        n400 = (d0 / 146_097) as i32;
        let d1 = (d0 % 146_097) as i32;
        n100 = d1 / 36_524;
        let d2 = d1 % 36_524;
        n4 = d2 / 1_461;
        let d3 = d2 % 1_461;
        n1 = d3 / 365;
    } else {
        let d0 = fixed_date - 1;
        n400 = floor_divide_l(d0, 146_097) as i32;
        let d1 = mod_l(d0, 146_097) as i32;
        n100 = floor_divide(d1, 36_524);
        let d2 = mod_i(d1, 36_524);
        n4 = floor_divide(d2, 1_461);
        let d3 = mod_i(d2, 1_461);
        n1 = floor_divide(d3, 365);
    }
    let mut year = 400 * n400 + 100 * n100 + 4 * n4 + n1;
    if !(n100 == 4 || n1 == 4) {
        year += 1;
    }
    year
}

/// `BaseCalendar.getCalendarDateFromFixedDate(date, fixedDate)` (Gregorian).
fn gregorian_date_from_fixed(fixed_date: i64) -> BaseDate {
    let year = gregorian_year_from_fixed(fixed_date);
    let jan1 = gregorian_fixed_date(year, 1, 1);
    let is_leap = is_gregorian_leap_year(year);
    let mut prior_days = (fixed_date - jan1) as i32;
    let mut mar1 = jan1 + 31 + 28;
    if is_leap {
        mar1 += 1;
    }
    if fixed_date >= mar1 {
        prior_days += if is_leap { 1 } else { 2 };
    }
    let mut month = 12 * prior_days + 373;
    if month > 0 {
        month /= 367;
    } else {
        month = floor_divide(month, 367);
    }
    let mut month1 = jan1 + i64::from(ACCUMULATED_DAYS_IN_MONTH[month as usize]);
    if is_leap && month >= 3 {
        month1 += 1;
    }
    let day_of_month = (fixed_date - month1) as i32 + 1;
    BaseDate {
        year,
        month,
        day_of_month,
        day_of_week: day_of_week_from_fixed(fixed_date),
        leap_year: is_leap,
    }
}

/// `BaseCalendar.getMonthLength(year, month)` (Gregorian), `month` from 0
/// (December of the year before) to 12.
fn gregorian_month_length(year: i32, month: i32) -> i32 {
    let mut days = DAYS_IN_MONTH[month as usize];
    if month == 2 && is_gregorian_leap_year(year) {
        days += 1;
    }
    days
}

/// `BaseCalendar.getDayOfYear(year, month, dayOfMonth)` (Gregorian).
fn gregorian_day_of_year(year: i32, month: i32, day_of_month: i32) -> i64 {
    i64::from(day_of_month)
        + i64::from(if is_gregorian_leap_year(year) {
            ACCUMULATED_DAYS_IN_MONTH_LEAP[month as usize]
        } else {
            ACCUMULATED_DAYS_IN_MONTH[month as usize]
        })
}

/// `JulianCalendar.getFixedDate(jyear, month, dayOfMonth, null)`.
fn julian_fixed_date(jyear: i32, month: i32, day_of_month: i32) -> i64 {
    let y = i64::from(jyear);
    let mut days = JULIAN_EPOCH - 1 + (365 * (y - 1)) + i64::from(day_of_month);
    if y > 0 {
        days += (y - 1) / 4;
    } else {
        days += floor_divide_l(y - 1, 4);
    }
    if month > 0 {
        days += ((367 * i64::from(month)) - 362) / 12;
    } else {
        days += floor_divide_l((367 * i64::from(month)) - 362, 12);
    }
    if month > 2 {
        days -= if is_julian_leap_year(jyear) { 1 } else { 2 };
    }
    days
}

/// `JulianCalendar.getCalendarDateFromFixedDate(date, fixedDate)`; the year
/// is normalized (`1 - year` for a year before Christ).
fn julian_date_from_fixed(fixed_date: i64) -> BaseDate {
    let fd = 4 * (fixed_date - JULIAN_EPOCH) + 1464;
    let year = if fd >= 0 {
        (fd / 1461) as i32
    } else {
        floor_divide_l(fd, 1461) as i32
    };
    let mut prior_days = (fixed_date - julian_fixed_date(year, 1, 1)) as i32;
    let is_leap = is_julian_leap_year(year);
    if fixed_date >= julian_fixed_date(year, 3, 1) {
        prior_days += if is_leap { 1 } else { 2 };
    }
    let mut month = 12 * prior_days + 373;
    if month > 0 {
        month /= 367;
    } else {
        month = floor_divide(month, 367);
    }
    let day_of_month = (fixed_date - julian_fixed_date(year, month, 1)) as i32 + 1;
    BaseDate {
        year,
        month,
        day_of_month,
        day_of_week: day_of_week_from_fixed(fixed_date),
        leap_year: is_leap,
    }
}

/// `CalendarSystem.FIELD_UNDEFINED`.
const FIELD_UNDEFINED: i32 = i32::MIN;

/// A `LocalGregorianCalendar.Date` of the Japanese calendar: the era (an
/// index into [`JAPANESE_ERAS`], `None` before Meiji), the year of the era,
/// the Gregorian year (`getNormalizedYear()`), the 1-based month, the day of
/// the month, and Java's `normalized` flag. Its time of day is always
/// midnight on the paths the calendar takes.
#[derive(Debug, Clone, Copy)]
struct LocalDate {
    era: Option<usize>,
    year: i32,
    gregorian_year: i32,
    month: i32,
    day_of_month: i32,
    day_of_week: i32,
    leap_year: bool,
    normalized: bool,
}

impl LocalDate {
    /// `jcal.newCalendarDate(TimeZone.NO_TIMEZONE)`.
    fn new() -> Self {
        Self {
            era: None,
            year: 0,
            gregorian_year: FIELD_UNDEFINED,
            month: 0,
            day_of_month: 0,
            day_of_week: FIELD_UNDEFINED,
            leap_year: false,
            normalized: false,
        }
    }

    /// `setEra(era)`.
    fn set_era(&mut self, era: Option<usize>) {
        if self.era != era {
            self.era = era;
            self.normalized = false;
            self.gregorian_year = FIELD_UNDEFINED;
        }
    }

    /// `setYear(localYear)`.
    fn set_year(&mut self, year: i32) {
        if self.year != year {
            self.year = year;
            self.normalized = false;
            self.gregorian_year = FIELD_UNDEFINED;
        }
    }

    /// `setMonth(month)`.
    fn set_month(&mut self, month: i32) {
        if self.month != month {
            self.month = month;
            self.normalized = false;
        }
    }

    /// `setDayOfMonth(date)`.
    fn set_day_of_month(&mut self, day: i32) {
        if self.day_of_month != day {
            self.day_of_month = day;
            self.normalized = false;
        }
    }

    /// `setDate(year, month, dayOfMonth)`.
    fn set_date(&mut self, year: i32, month: i32, day: i32) {
        self.set_year(year);
        self.set_month(month);
        self.set_day_of_month(day);
    }

    /// `addYear(localYear)`.
    fn add_year(&mut self, n: i32) {
        if n != 0 {
            self.year = self.year.wrapping_add(n);
            self.normalized = false;
        }
        self.gregorian_year = self.gregorian_year.wrapping_add(n);
    }

    /// `addMonth(n)` (`CalendarDate.addMonth`).
    fn add_month(&mut self, n: i32) {
        if n != 0 {
            self.month = self.month.wrapping_add(n);
            self.normalized = false;
        }
    }
}

/// `LocalGregorianCalendar.adjustYear(ldate, millis, zoneOffset)`.
fn adjust_year(d: &mut LocalDate, millis: i64, zone_offset: i32) {
    let mut found = false;
    for i in (0..JAPANESE_ERA_COUNT).rev() {
        let era = &JAPANESE_ERAS[i];
        // every Japanese era is a local-time era
        let since = era.since - i64::from(zone_offset);
        if millis >= since {
            d.era = Some(i);
            d.year = d.gregorian_year - era.since_year + 1;
            found = true;
            break;
        }
    }
    if !found {
        d.era = None;
        d.year = d.gregorian_year;
    }
    d.normalized = true;
}

/// `LocalGregorianCalendar.getCalendarDateFromFixedDate(date, fixedDate)`.
fn local_date_from_fixed(fixed_date: i64) -> LocalDate {
    let g = gregorian_date_from_fixed(fixed_date);
    let mut d = LocalDate::new();
    d.gregorian_year = g.year;
    d.month = g.month;
    d.day_of_month = g.day_of_month;
    d.day_of_week = g.day_of_week;
    d.leap_year = g.leap_year;
    adjust_year(&mut d, (fixed_date - EPOCH_OFFSET).wrapping_mul(ONE_DAY), 0);
    d
}

/// `LocalGregorianCalendar.getCalendarDate(millis, zone)`: the local date of
/// the instant `millis` in `zone`.
fn local_date_of_millis(millis: i64, zone: &TimeZone) -> LocalDate {
    let (zone_offset, _) = zone.get_offsets(millis);
    let mut days = i64::from(zone_offset) / ONE_DAY;
    let mut ms = (i64::from(zone_offset) % ONE_DAY) as i32;
    days += millis / ONE_DAY;
    ms += (millis % ONE_DAY) as i32;
    if i64::from(ms) >= ONE_DAY {
        // the time of day (`ms - ONE_DAY`) is not kept
        days += 1;
    } else {
        while ms < 0 {
            ms += ONE_DAY as i32;
            days -= 1;
        }
    }
    days += EPOCH_OFFSET;
    let g = gregorian_date_from_fixed(days);
    let mut d = LocalDate::new();
    d.gregorian_year = g.year;
    d.month = g.month;
    d.day_of_month = g.day_of_month;
    d.day_of_week = g.day_of_week;
    d.leap_year = g.leap_year;
    adjust_year(&mut d, millis, zone_offset);
    d
}

/// `LocalGregorianCalendar.normalizeYear(date)`.
fn local_normalize_year(d: &mut LocalDate) {
    match d.era {
        None => d.gregorian_year = d.year,
        Some(e) => d.gregorian_year = JAPANESE_ERAS[e].since_year + d.year - 1,
    }
}

/// `BaseCalendar.normalizeMonth(date)` on the Gregorian year of `d`.
fn local_normalize_month(d: &mut LocalDate) {
    let mut year = d.gregorian_year;
    let mut month = i64::from(d.month);
    if month <= 0 {
        let xm = 1 - month;
        year -= ((xm / 12) + 1) as i32;
        month = 13 - (xm % 12);
        if month == 13 {
            year += 1;
            month = 1;
        }
        d.gregorian_year = year;
        d.set_month(month as i32);
    } else if month > 12 {
        year += ((month - 1) / 12) as i32;
        month = ((month - 1) % 12) + 1;
        d.gregorian_year = year;
        d.set_month(month as i32);
    }
}

/// `LocalGregorianCalendar.normalize(date)`.
fn local_normalize(d: &mut LocalDate) {
    if d.normalized {
        return;
    }
    local_normalize_year(d);
    // `BaseCalendar.normalize`: no time zone, a time of day of midnight
    local_normalize_year(d);
    local_normalize_month(d);
    let day = i64::from(d.day_of_month);
    let mut m = d.month;
    let y = d.gregorian_year;
    let mut ml = gregorian_month_length(y, m);
    if !(day > 0 && day <= i64::from(ml)) {
        if day <= 0 && day > -28 {
            m -= 1;
            ml = gregorian_month_length(y, m);
            d.day_of_month = (day + i64::from(ml)) as i32;
            if m == 0 {
                m = 12;
                d.gregorian_year = y - 1;
            }
            d.month = m;
        } else if day > i64::from(ml) && day < i64::from(ml) + 28 {
            m += 1;
            d.day_of_month = (day - i64::from(ml)) as i32;
            if m > 12 {
                d.gregorian_year = y + 1;
                m = 1;
            }
            d.month = m;
        } else {
            let fixed_date = day + gregorian_fixed_date(y, m, 1) - 1;
            let era_before = d.era;
            *d = local_date_from_fixed(fixed_date);
            let _ = era_before;
        }
    } else {
        d.day_of_week = day_of_week_from_fixed(gregorian_fixed_date(
            d.gregorian_year,
            d.month,
            d.day_of_month,
        ));
    }
    d.leap_year = is_gregorian_leap_year(d.gregorian_year);
    d.normalized = true;
    // the era of the normalized date
    let year = d.gregorian_year;
    let mut index: i32 = JAPANESE_ERA_COUNT as i32 - 1;
    let mut chosen: Option<usize> = None;
    while index >= 0 {
        let era = &JAPANESE_ERAS[index as usize];
        chosen = Some(index as usize);
        let since_year = era.since_year;
        if year > since_year {
            break;
        }
        if year == since_year {
            let month = d.month;
            let since_month = era.since_month;
            if month > since_month {
                break;
            }
            if month == since_month {
                let day = d.day_of_month;
                let since_day = era.since_day;
                if day > since_day {
                    break;
                }
                if day == since_day {
                    // both times of day are midnight: `timeOfDay >=
                    // sinceTimeOfDay`
                    break;
                }
            }
        }
        index -= 1;
    }
    if index >= 0 {
        let e = chosen.expect("INVARIANT: an era was examined");
        d.era = Some(e);
        d.year = d.gregorian_year - JAPANESE_ERAS[e].since_year + 1;
    } else {
        d.era = None;
        d.gregorian_year = year;
        d.year = year;
    }
    d.normalized = true;
}

/// `jcal.getFixedDate(date)` (`BaseCalendar.getFixedDate(CalendarDate)`).
fn local_fixed_date(d: &mut LocalDate) -> i64 {
    if !d.normalized {
        local_normalize_year(d);
        local_normalize_month(d);
    }
    gregorian_fixed_date(d.gregorian_year, d.month, d.day_of_month)
}

/// `jcal.getMonthLength(date)`.
fn local_month_length(d: &LocalDate) -> Result<i32> {
    if d.month < 1 || d.month > 12 {
        return Err(LuceneError::illegal_argument(format!(
            "Illegal month value: {}",
            d.month
        )));
    }
    Ok(gregorian_month_length(d.gregorian_year, d.month))
}

/// `JapaneseImperialCalendar.getEraIndex(date)`.
fn era_index(d: &LocalDate) -> i32 {
    d.era.map_or(0, |e| e as i32 + 1)
}

/// The since date of `eras[index]` of `JapaneseImperialCalendar`, entry 0
/// being `BEFORE_MEIJI_ERA` (since `Long.MIN_VALUE`).
fn japanese_since_date(index: i32) -> BaseDate {
    if index == 0 {
        return gregorian_date_from_fixed(SINCE_FIXED_DATES[0]);
    }
    let e = &JAPANESE_ERAS[index as usize - 1];
    BaseDate {
        year: e.since_year,
        month: e.since_month,
        day_of_month: e.since_day,
        day_of_week: 0,
        leap_year: is_gregorian_leap_year(e.since_year),
    }
}

// ------------------------------------------------------------------ calendar

/// The calendar systems `Calendar.getInstance` creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarKind {
    /// `java.util.GregorianCalendar` (`"gregory"`, or `"iso8601"` with a
    /// proleptic Gregorian calendar).
    Gregorian,
    /// `sun.util.BuddhistCalendar` (`"buddhist"`).
    Buddhist,
    /// `java.util.JapaneseImperialCalendar` (`"japanese"`).
    Japanese,
}

/// A `java.util.Calendar`.
///
/// Equivalent to the calendar `Calendar.getInstance(zone, locale)` returns;
/// see the module documentation.
#[derive(Debug, Clone)]
pub struct Calendar {
    kind: CalendarKind,
    time: i64,
    is_time_set: bool,
    are_fields_set: bool,
    are_all_fields_set: bool,
    fields: [i32; FIELD_COUNT],
    is_set: [bool; FIELD_COUNT],
    stamp: [i32; FIELD_COUNT],
    next_stamp: i32,
    lenient: bool,
    zone: TimeZone,
    first_day_of_week: i32,
    minimal_days_in_first_week: i32,
    // GregorianCalendar
    gregorian_cutover: i64,
    gregorian_cutover_date: i64,
    gregorian_cutover_year: i32,
    gregorian_cutover_year_julian: i32,
    /// `calsys`: `None` for `null`, `Some(false)` for `gcal`, `Some(true)`
    /// for the Julian calendar.
    calsys_julian: Option<bool>,
    /// `cdate`, the date the last `computeFields` computed, with its era
    /// (`true` for BCE in the Julian calendar).
    cdate: BaseDate,
    // BuddhistCalendar
    year_offset: i32,
    // JapaneseImperialCalendar
    jdate: LocalDate,
    cached_fixed_date: i64,
}

impl Calendar {
    /// `Calendar.getInstance(zone, aLocale)`: through `CalendarProviderImpl`,
    /// a `Calendar.Builder` of `aLocale` and `zone` at the current instant:
    /// the type of the `-u-ca-` keyword (`gregory`, `iso8601` — a proleptic
    /// Gregorian calendar with the ISO 8601 week definition —, `buddhist`,
    /// `japanese`), else Buddhist for `th_TH`, else Gregorian; an unknown
    /// type makes the builder throw, and `createCalendar` then falls back to
    /// Buddhist for `th_TH`, Japanese for `ja_JP_JP` and Gregorian otherwise.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of the region override the week data
    /// reads (`CalendarDataUtility.findRegionOverride`).
    pub fn get_instance(zone: &TimeZone, locale: &Locale) -> Result<Calendar> {
        let ca = locale.get_unicode_locale_type("ca");
        let th_th = locale.get_country() == "TH" && locale.get_language() == "th";
        let (kind, iso) = match ca.as_deref() {
            None => (
                if th_th {
                    CalendarKind::Buddhist
                } else {
                    CalendarKind::Gregorian
                },
                false,
            ),
            Some("gregory") => (CalendarKind::Gregorian, false),
            Some("iso8601") => (CalendarKind::Gregorian, true),
            Some("buddhist") => (CalendarKind::Buddhist, false),
            Some("japanese") => (CalendarKind::Japanese, false),
            Some(_) => {
                // `Builder.build` throws; `createCalendar`'s fallback
                let kind = if th_th {
                    CalendarKind::Buddhist
                } else if locale.get_variant() == "JP"
                    && locale.get_language() == "ja"
                    && locale.get_country() == "JP"
                {
                    CalendarKind::Japanese
                } else {
                    CalendarKind::Gregorian
                };
                (kind, false)
            }
        };
        let mut cal = Self::with_kind(kind, zone, locale)?;
        if iso {
            // `gcal.setGregorianChange(new Date(Long.MIN_VALUE))` and
            // `setWeekDefinition(MONDAY, 4)`
            cal.set_gregorian_change(i64::MIN);
            cal.first_day_of_week = MONDAY;
            cal.minimal_days_in_first_week = 4;
        }
        if kind == CalendarKind::Buddhist {
            cal.clear();
        }
        cal.set_time_in_millis(current_time_millis());
        cal.complete()?;
        Ok(cal)
    }

    /// `new GregorianCalendar(zone, aLocale)`: a Gregorian calendar at the
    /// current instant, whatever the locale's calendar type.
    ///
    /// # Errors
    ///
    /// As [`get_instance`](Self::get_instance).
    pub fn new_gregorian(zone: &TimeZone, locale: &Locale) -> Result<Calendar> {
        let mut cal = Self::with_kind(CalendarKind::Gregorian, zone, locale)?;
        cal.set_time_in_millis(current_time_millis());
        Ok(cal)
    }

    /// `Calendar(zone, aLocale)` and the constructor of the calendar class,
    /// without a time set.
    fn with_kind(kind: CalendarKind, zone: &TimeZone, locale: &Locale) -> Result<Calendar> {
        let (first_day_of_week, minimal_days) = week_count_data(locale)?;
        let mut cal = Calendar {
            kind,
            time: 0,
            is_time_set: false,
            are_fields_set: false,
            are_all_fields_set: false,
            fields: [0; FIELD_COUNT],
            is_set: [false; FIELD_COUNT],
            stamp: [UNSET; FIELD_COUNT],
            next_stamp: MINIMUM_USER_STAMP,
            lenient: true,
            zone: zone.clone(),
            first_day_of_week,
            minimal_days_in_first_week: minimal_days,
            gregorian_cutover: 0,
            gregorian_cutover_date: 0,
            gregorian_cutover_year: 0,
            gregorian_cutover_year_julian: 0,
            calsys_julian: None,
            cdate: BaseDate {
                year: 0,
                month: 0,
                day_of_month: 0,
                day_of_week: 0,
                leap_year: false,
            },
            year_offset: if kind == CalendarKind::Buddhist {
                543
            } else {
                0
            },
            jdate: LocalDate::new(),
            cached_fixed_date: i64::MIN,
        };
        cal.set_gregorian_change(DEFAULT_GREGORIAN_CUTOVER);
        Ok(cal)
    }

    /// Sets the `GregorianCalendar` change date, as the millisecond instant
    /// of the cutover (`Long.MIN_VALUE` for a pure Gregorian calendar,
    /// `Long.MAX_VALUE` for a pure Julian one).
    ///
    /// Equivalent to `GregorianCalendar.setGregorianChange(Date)` with
    /// `new Date(cutoverTime)`.
    pub fn set_gregorian_change(&mut self, cutover_time: i64) {
        self.gregorian_cutover = cutover_time;
        self.gregorian_cutover_date = floor_divide_l(cutover_time, ONE_DAY) + EPOCH_OFFSET;
        if cutover_time == i64::MAX {
            self.gregorian_cutover_date += 1;
        }
        let d = self.calendar_date(self.gregorian_cutover_date);
        self.gregorian_cutover_year = d.year;
        let j = julian_date_from_fixed(self.gregorian_cutover_date - 1);
        self.gregorian_cutover_year_julian = j.year;
        if self.time < self.gregorian_cutover {
            self.set_unnormalized();
        }
    }

    /// The `GregorianCalendar` change date, as a millisecond instant.
    ///
    /// Equivalent to `GregorianCalendar.getGregorianChange().getTime()`.
    pub fn get_gregorian_change(&self) -> i64 {
        self.gregorian_cutover
    }

    /// `getCalendarType()`.
    pub fn get_calendar_type(&self) -> &'static str {
        match self.kind {
            CalendarKind::Gregorian => "gregory",
            CalendarKind::Buddhist => "buddhist",
            CalendarKind::Japanese => "japanese",
        }
    }

    /// The calendar system of this calendar.
    pub fn kind(&self) -> CalendarKind {
        self.kind
    }

    /// `isLenient()`.
    pub fn is_lenient(&self) -> bool {
        self.lenient
    }

    /// `setLenient(boolean)`.
    pub fn set_lenient(&mut self, lenient: bool) {
        self.lenient = lenient;
    }

    /// `getTimeZone()`.
    pub fn get_time_zone(&self) -> &TimeZone {
        &self.zone
    }

    /// `setTimeZone(TimeZone)`.
    pub fn set_time_zone(&mut self, zone: &TimeZone) {
        self.zone = zone.clone();
        self.are_all_fields_set = false;
        self.are_fields_set = false;
    }

    /// `getFirstDayOfWeek()`.
    pub fn get_first_day_of_week(&self) -> i32 {
        self.first_day_of_week
    }

    /// `setFirstDayOfWeek(int)`.
    pub fn set_first_day_of_week(&mut self, value: i32) {
        if self.first_day_of_week == value {
            return;
        }
        self.first_day_of_week = value;
        self.invalidate_week_fields();
    }

    /// `getMinimalDaysInFirstWeek()`.
    pub fn get_minimal_days_in_first_week(&self) -> i32 {
        self.minimal_days_in_first_week
    }

    /// `setMinimalDaysInFirstWeek(int)`.
    pub fn set_minimal_days_in_first_week(&mut self, value: i32) {
        if self.minimal_days_in_first_week == value {
            return;
        }
        self.minimal_days_in_first_week = value;
        self.invalidate_week_fields();
    }

    /// `Calendar.invalidateWeekFields()`.
    fn invalidate_week_fields(&mut self) {
        if self.stamp[WEEK_OF_MONTH] != COMPUTED && self.stamp[WEEK_OF_YEAR] != COMPUTED {
            return;
        }
        // We have to check the new values of these fields after changing
        // firstDayOfWeek and/or minimalDaysInFirstWeek. If the field values
        // have been changed, then set the new values.
        let mut cal = self.clone();
        cal.set_lenient(true);
        cal.clear_field(WEEK_OF_MONTH);
        cal.clear_field(WEEK_OF_YEAR);
        if cal.complete().is_err() {
            return;
        }
        if self.stamp[WEEK_OF_MONTH] == COMPUTED {
            let week_of_month = cal.fields[WEEK_OF_MONTH];
            if self.fields[WEEK_OF_MONTH] != week_of_month {
                self.fields[WEEK_OF_MONTH] = week_of_month;
            }
        }
        if self.stamp[WEEK_OF_YEAR] == COMPUTED {
            let week_of_year = cal.fields[WEEK_OF_YEAR];
            if self.fields[WEEK_OF_YEAR] != week_of_year {
                self.fields[WEEK_OF_YEAR] = week_of_year;
            }
        }
    }

    /// `isWeekDateSupported()`: `true` for the Gregorian and Buddhist
    /// calendars, `false` for the Japanese one.
    pub fn is_week_date_supported(&self) -> bool {
        self.kind != CalendarKind::Japanese
    }

    // ------------------------------------------------------- field access

    /// `get(int field)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` a non-lenient calendar's
    /// `computeTime` raises for a field out of range.
    pub fn get(&mut self, field: usize) -> Result<i32> {
        self.complete()?;
        let v = self.fields[field];
        if field == YEAR && self.kind == CalendarKind::Buddhist {
            return Ok(v.wrapping_add(self.year_offset));
        }
        Ok(v)
    }

    /// `internalGet(int field)`.
    fn internal_get(&self, field: usize) -> i32 {
        self.fields[field]
    }

    /// `internalSet(int field, int value)`.
    fn internal_set(&mut self, field: usize, value: i32) {
        self.fields[field] = value;
    }

    /// `set(int field, int value)`, with `BuddhistCalendar`'s year offset.
    pub fn set(&mut self, field: usize, value: i32) {
        let value = if field == YEAR && self.kind == CalendarKind::Buddhist {
            value.wrapping_sub(self.year_offset)
        } else {
            value
        };
        if self.are_fields_set && !self.are_all_fields_set {
            self.compute_fields();
        }
        self.internal_set(field, value);
        self.is_time_set = false;
        self.are_fields_set = false;
        self.is_set[field] = true;
        self.stamp[field] = self.next_stamp;
        self.next_stamp += 1;
        if self.next_stamp == i32::MAX {
            self.adjust_stamp();
        }
    }

    /// `set(int year, int month, int date)`.
    pub fn set_date(&mut self, year: i32, month: i32, date: i32) {
        self.set(YEAR, year);
        self.set(MONTH, month);
        self.set(DATE, date);
    }

    /// `set(int year, int month, int date, int hourOfDay, int minute)`.
    pub fn set_date_time(
        &mut self,
        year: i32,
        month: i32,
        date: i32,
        hour_of_day: i32,
        minute: i32,
    ) {
        self.set(YEAR, year);
        self.set(MONTH, month);
        self.set(DATE, date);
        self.set(HOUR_OF_DAY, hour_of_day);
        self.set(MINUTE, minute);
    }

    /// `set(int year, int month, int date, int hourOfDay, int minute, int
    /// second)`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_date_time_seconds(
        &mut self,
        year: i32,
        month: i32,
        date: i32,
        hour_of_day: i32,
        minute: i32,
        second: i32,
    ) {
        self.set(YEAR, year);
        self.set(MONTH, month);
        self.set(DATE, date);
        self.set(HOUR_OF_DAY, hour_of_day);
        self.set(MINUTE, minute);
        self.set(SECOND, second);
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        for i in 0..FIELD_COUNT {
            self.stamp[i] = UNSET;
            self.fields[i] = 0;
            self.is_set[i] = false;
        }
        self.are_all_fields_set = false;
        self.are_fields_set = false;
        self.is_time_set = false;
    }

    /// `clear(int field)`.
    pub fn clear_field(&mut self, field: usize) {
        self.fields[field] = 0;
        self.stamp[field] = UNSET;
        self.is_set[field] = false;
        self.are_all_fields_set = false;
        self.are_fields_set = false;
        self.is_time_set = false;
    }

    /// `isSet(int field)`.
    pub fn is_set(&self, field: usize) -> bool {
        self.stamp[field] != UNSET
    }

    /// `getTime()`.
    ///
    /// # Errors
    ///
    /// As [`get`](Self::get).
    pub fn get_time(&mut self) -> Result<Date> {
        Ok(Date::new(self.get_time_in_millis()?))
    }

    /// `setTime(Date)`.
    pub fn set_time(&mut self, date: Date) {
        self.set_time_in_millis(date.get_time());
    }

    /// `getTimeInMillis()`.
    ///
    /// # Errors
    ///
    /// As [`get`](Self::get).
    pub fn get_time_in_millis(&mut self) -> Result<i64> {
        if !self.is_time_set {
            self.update_time()?;
        }
        Ok(self.time)
    }

    /// `setTimeInMillis(long)`.
    pub fn set_time_in_millis(&mut self, millis: i64) {
        if self.time == millis && self.is_time_set && self.are_fields_set && self.are_all_fields_set
        {
            return;
        }
        self.time = millis;
        self.is_time_set = true;
        self.are_fields_set = false;
        self.compute_fields();
        self.are_all_fields_set = true;
        self.are_fields_set = true;
    }

    /// `complete()`.
    fn complete(&mut self) -> Result<()> {
        if !self.is_time_set {
            self.update_time()?;
        }
        if !self.are_fields_set || !self.are_all_fields_set {
            self.compute_fields();
            self.are_all_fields_set = true;
            self.are_fields_set = true;
        }
        Ok(())
    }

    /// `updateTime()`.
    fn update_time(&mut self) -> Result<()> {
        self.compute_time()?;
        self.is_time_set = true;
        Ok(())
    }

    /// `adjustStamp()`.
    fn adjust_stamp(&mut self) {
        let mut max = MINIMUM_USER_STAMP;
        let mut new_stamp = MINIMUM_USER_STAMP;
        loop {
            let mut min = i32::MAX;
            for &v in &self.stamp {
                if v >= new_stamp && min > v {
                    min = v;
                }
                if max < v {
                    max = v;
                }
            }
            if max != min && min == i32::MAX {
                break;
            }
            for s in self.stamp.iter_mut() {
                if *s == min {
                    *s = new_stamp;
                }
            }
            new_stamp += 1;
            if min == max {
                break;
            }
        }
        self.next_stamp = new_stamp;
    }

    fn is_externally_set(&self, field: usize) -> bool {
        self.stamp[field] >= MINIMUM_USER_STAMP
    }

    fn get_set_state_fields(&self) -> i32 {
        let mut mask = 0;
        for i in 0..FIELD_COUNT {
            if self.stamp[i] != UNSET {
                mask |= 1 << i;
            }
        }
        mask
    }

    fn set_fields_computed(&mut self, field_mask: i32) {
        if field_mask == ALL_FIELDS {
            for i in 0..FIELD_COUNT {
                self.stamp[i] = COMPUTED;
                self.is_set[i] = true;
            }
            self.are_fields_set = true;
            self.are_all_fields_set = true;
        } else {
            let mut m = field_mask;
            for i in 0..FIELD_COUNT {
                if (m & 1) == 1 {
                    self.stamp[i] = COMPUTED;
                    self.is_set[i] = true;
                } else if self.are_all_fields_set && !self.is_set[i] {
                    self.are_all_fields_set = false;
                }
                m = ((m as u32) >> 1) as i32;
            }
        }
    }

    fn set_fields_normalized(&mut self, field_mask: i32) {
        if field_mask != ALL_FIELDS {
            let mut m = field_mask;
            for i in 0..FIELD_COUNT {
                if (m & 1) == 0 {
                    self.stamp[i] = UNSET;
                    self.fields[i] = 0;
                    self.is_set[i] = false;
                }
                m >>= 1;
            }
        }
        self.are_fields_set = true;
        self.are_all_fields_set = false;
    }

    fn is_partially_normalized(&self) -> bool {
        self.are_fields_set && !self.are_all_fields_set
    }

    fn is_fully_normalized(&self) -> bool {
        self.are_fields_set && self.are_all_fields_set
    }

    fn set_unnormalized(&mut self) {
        self.are_fields_set = false;
        self.are_all_fields_set = false;
    }

    /// `Calendar.selectFields()`.
    fn select_fields(&self) -> i32 {
        let stamp = &self.stamp;
        let mut field_mask = YEAR_MASK;
        if stamp[ERA] != UNSET {
            field_mask |= ERA_MASK;
        }
        let dow_stamp = stamp[DAY_OF_WEEK];
        let month_stamp = stamp[MONTH];
        let mut dom_stamp = stamp[DAY_OF_MONTH];
        let mut wom_stamp = aggregate_stamp(stamp[WEEK_OF_MONTH], dow_stamp);
        let mut dowim_stamp = aggregate_stamp(stamp[DAY_OF_WEEK_IN_MONTH], dow_stamp);
        let doy_stamp = stamp[DAY_OF_YEAR];
        let mut woy_stamp = aggregate_stamp(stamp[WEEK_OF_YEAR], dow_stamp);
        let mut best_stamp = dom_stamp;
        if wom_stamp > best_stamp {
            best_stamp = wom_stamp;
        }
        if dowim_stamp > best_stamp {
            best_stamp = dowim_stamp;
        }
        if doy_stamp > best_stamp {
            best_stamp = doy_stamp;
        }
        if woy_stamp > best_stamp {
            best_stamp = woy_stamp;
        }
        if best_stamp == UNSET {
            wom_stamp = stamp[WEEK_OF_MONTH];
            dowim_stamp = stamp[DAY_OF_WEEK_IN_MONTH].max(dow_stamp);
            woy_stamp = stamp[WEEK_OF_YEAR];
            best_stamp = wom_stamp.max(dowim_stamp).max(woy_stamp);
            if best_stamp == UNSET {
                dom_stamp = month_stamp;
                best_stamp = dom_stamp;
            }
        }
        if best_stamp == dom_stamp
            || (best_stamp == wom_stamp && stamp[WEEK_OF_MONTH] >= stamp[WEEK_OF_YEAR])
            || (best_stamp == dowim_stamp && stamp[DAY_OF_WEEK_IN_MONTH] >= stamp[WEEK_OF_YEAR])
        {
            field_mask |= MONTH_MASK;
            if best_stamp == dom_stamp {
                field_mask |= DAY_OF_MONTH_MASK;
            } else {
                if dow_stamp != UNSET {
                    field_mask |= DAY_OF_WEEK_MASK;
                }
                if wom_stamp == dowim_stamp {
                    if stamp[WEEK_OF_MONTH] >= stamp[DAY_OF_WEEK_IN_MONTH] {
                        field_mask |= WEEK_OF_MONTH_MASK;
                    } else {
                        field_mask |= DAY_OF_WEEK_IN_MONTH_MASK;
                    }
                } else if best_stamp == wom_stamp {
                    field_mask |= WEEK_OF_MONTH_MASK;
                } else if stamp[DAY_OF_WEEK_IN_MONTH] != UNSET {
                    field_mask |= DAY_OF_WEEK_IN_MONTH_MASK;
                }
            }
        } else if best_stamp == doy_stamp {
            field_mask |= DAY_OF_YEAR_MASK;
        } else {
            if dow_stamp != UNSET {
                field_mask |= DAY_OF_WEEK_MASK;
            }
            field_mask |= WEEK_OF_YEAR_MASK;
        }
        let hour_of_day_stamp = stamp[HOUR_OF_DAY];
        let hour_stamp = aggregate_stamp(stamp[HOUR], stamp[AM_PM]);
        let mut best_stamp = if hour_stamp > hour_of_day_stamp {
            hour_stamp
        } else {
            hour_of_day_stamp
        };
        if best_stamp == UNSET {
            best_stamp = stamp[HOUR].max(stamp[AM_PM]);
        }
        if best_stamp != UNSET {
            if best_stamp == hour_of_day_stamp {
                field_mask |= HOUR_OF_DAY_MASK;
            } else {
                field_mask |= HOUR_MASK;
                if stamp[AM_PM] != UNSET {
                    field_mask |= AM_PM_MASK;
                }
            }
        }
        if stamp[MINUTE] != UNSET {
            field_mask |= MINUTE_MASK;
        }
        if stamp[SECOND] != UNSET {
            field_mask |= SECOND_MASK;
        }
        if stamp[MILLISECOND] != UNSET {
            field_mask |= MILLISECOND_MASK;
        }
        if stamp[ZONE_OFFSET] >= MINIMUM_USER_STAMP {
            field_mask |= ZONE_OFFSET_MASK;
        }
        if stamp[DST_OFFSET] >= MINIMUM_USER_STAMP {
            field_mask |= DST_OFFSET_MASK;
        }
        field_mask
    }

    // ----------------------------------------------------------- limits

    /// `getMinimum(int field)`.
    pub fn get_minimum(&self, field: usize) -> i32 {
        match self.kind {
            CalendarKind::Japanese => JAPANESE_MIN_VALUES[field],
            _ => GREGORIAN_MIN_VALUES[field],
        }
    }

    /// `getMaximum(int field)` (see the module documentation for the
    /// proleptic Gregorian calendar).
    pub fn get_maximum(&self, field: usize) -> i32 {
        match self.kind {
            CalendarKind::Japanese => {
                if field == YEAR {
                    let d = local_date_of_millis(i64::MAX, &self.zone);
                    JAPANESE_LEAST_MAX_VALUES[YEAR].max(d.year)
                } else {
                    JAPANESE_MAX_VALUES[field]
                }
            }
            _ => GREGORIAN_MAX_VALUES[field],
        }
    }

    /// `getLeastMaximum(int field)`.
    pub fn get_least_maximum(&self, field: usize) -> i32 {
        match self.kind {
            CalendarKind::Japanese => {
                if field == YEAR {
                    JAPANESE_LEAST_MAX_VALUES[YEAR].min(self.get_maximum(YEAR))
                } else {
                    JAPANESE_LEAST_MAX_VALUES[field]
                }
            }
            _ => GREGORIAN_LEAST_MAX_VALUES[field],
        }
    }

    /// `GregorianCalendar.getActualMinimum(int field)`: the minimum value
    /// `field` could have, given the time value of this calendar.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for a Buddhist or Japanese calendar
    /// (not emulated), and the errors of completing the fields.
    pub fn get_actual_minimum(&self, field: usize) -> Result<i32> {
        if self.kind != CalendarKind::Gregorian {
            return Err(LuceneError::unsupported_operation(
                "getActualMinimum of a non-Gregorian calendar",
            ));
        }
        if field == DAY_OF_MONTH {
            let gc = self.normalized_calendar()?;
            let year = gc.cdate.year;
            if year == self.gregorian_cutover_year || year == self.gregorian_cutover_year_julian {
                let month1 = self.fixed_date_month1(&gc.cdate, gc.current_fixed_date());
                let d = self.calendar_date(month1);
                return Ok(d.day_of_month);
            }
        }
        Ok(self.get_minimum(field))
    }

    /// `GregorianCalendar.isCutoverYear(int normalizedYear)`.
    fn is_cutover_year(&self, normalized_year: i32) -> bool {
        let year = if self.calsys_julian == Some(true) {
            self.gregorian_cutover_year_julian
        } else {
            self.gregorian_cutover_year
        };
        normalized_year == year
    }

    /// `GregorianCalendar.getYearOffsetInMillis()`.
    fn year_offset_in_millis(&self) -> i64 {
        let mut t = i64::from(self.internal_get(DAY_OF_YEAR) - 1) * 24;
        t += i64::from(self.internal_get(HOUR_OF_DAY));
        t *= 60;
        t += i64::from(self.internal_get(MINUTE));
        t *= 60;
        t += i64::from(self.internal_get(SECOND));
        t *= 1000;
        t + i64::from(self.internal_get(MILLISECOND))
            - i64::from(self.internal_get(ZONE_OFFSET) + self.internal_get(DST_OFFSET))
    }

    /// `GregorianCalendar.getActualMaximum(int field)`: the maximum value
    /// `field` could have, given the time value of this calendar.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for `WEEK_OF_YEAR`, `WEEK_OF_MONTH`
    /// and `DAY_OF_WEEK_IN_MONTH` and for a Buddhist or Japanese calendar
    /// (not emulated); Java's `ArrayIndexOutOfBoundsException` for an
    /// unknown field; and the errors of completing the fields.
    pub fn get_actual_maximum(&self, field: usize) -> Result<i32> {
        if self.kind != CalendarKind::Gregorian {
            return Err(LuceneError::unsupported_operation(
                "getActualMaximum of a non-Gregorian calendar",
            ));
        }
        let fields_for_fixed_max = ERA_MASK
            | DAY_OF_WEEK_MASK
            | HOUR_MASK
            | AM_PM_MASK
            | HOUR_OF_DAY_MASK
            | MINUTE_MASK
            | SECOND_MASK
            | MILLISECOND_MASK
            | ZONE_OFFSET_MASK
            | DST_OFFSET_MASK;
        if field < FIELD_COUNT && (fields_for_fixed_max & mask(field)) != 0 {
            return Ok(self.get_maximum(field));
        }

        let mut gc = self.normalized_calendar()?;
        let date = gc.cdate;
        let julian = gc.calsys_julian == Some(true);
        let cal_fixed_date = |y: i32, m: i32, d: i32| -> i64 {
            if julian {
                julian_fixed_date(y, m, d)
            } else {
                gregorian_fixed_date(y, m, d)
            }
        };
        let cal_date_from_fixed = |fd: i64| -> BaseDate {
            if julian {
                julian_date_from_fixed(fd)
            } else {
                gregorian_date_from_fixed(fd)
            }
        };
        let cal_month_length = |d: &BaseDate| -> i32 {
            if julian {
                let mut days = DAYS_IN_MONTH[d.month as usize];
                if d.month == 2 && is_julian_leap_year(d.year) {
                    days += 1;
                }
                days
            } else {
                gregorian_month_length(d.year, d.month)
            }
        };
        let mut normalized_year = date.year;

        let value = match field {
            MONTH => {
                if !gc.is_cutover_year(normalized_year) {
                    DECEMBER
                } else {
                    // January 1 of the next year may or may not exist.
                    let mut next_jan1;
                    loop {
                        normalized_year += 1;
                        next_jan1 = gregorian_fixed_date(normalized_year, 1, 1);
                        if next_jan1 >= self.gregorian_cutover_date {
                            break;
                        }
                    }
                    let d = cal_date_from_fixed(next_jan1 - 1);
                    d.month - 1
                }
            }
            DAY_OF_MONTH => {
                let mut value = cal_month_length(&date);
                if gc.is_cutover_year(normalized_year) && date.day_of_month != value {
                    // Handle cutover year.
                    let fd = gc.current_fixed_date();
                    if fd < self.gregorian_cutover_date {
                        let month_length = gc.actual_month_length();
                        let month_end =
                            gc.fixed_date_month1(&gc.cdate, fd) + i64::from(month_length) - 1;
                        // Convert the fixed date to its calendar date.
                        let d = gc.calendar_date(month_end);
                        value = d.day_of_month;
                    }
                }
                value
            }
            DAY_OF_YEAR => {
                if !gc.is_cutover_year(normalized_year) {
                    let leap = if julian {
                        is_julian_leap_year(date.year)
                    } else {
                        is_gregorian_leap_year(date.year)
                    };
                    if leap {
                        366
                    } else {
                        365
                    }
                } else {
                    // Handle cutover year.
                    let jan1 = if self.gregorian_cutover_year == self.gregorian_cutover_year_julian
                    {
                        if gc.cutover_calendar_is_julian() {
                            julian_fixed_date(normalized_year, 1, 1)
                        } else {
                            gregorian_fixed_date(normalized_year, 1, 1)
                        }
                    } else if normalized_year == self.gregorian_cutover_year_julian {
                        cal_fixed_date(normalized_year, 1, 1)
                    } else {
                        self.gregorian_cutover_date
                    };
                    // January 1 of the next year may or may not exist.
                    let mut next_jan1 = gregorian_fixed_date(normalized_year + 1, 1, 1);
                    if next_jan1 < self.gregorian_cutover_date {
                        next_jan1 = self.gregorian_cutover_date;
                    }
                    (next_jan1 - jan1) as i32
                }
            }
            YEAR => {
                // Calculate the millisecond offset from the beginning of
                // the year of this calendar and adjust the max year value
                // if we are beyond the limit in the max year.
                let current = gc.year_offset_in_millis();

                if gc.internal_get_era() == AD {
                    gc.set_time_in_millis(i64::MAX);
                    let mut value = gc.get(YEAR)?;
                    let max_end = gc.year_offset_in_millis();
                    if current > max_end {
                        value -= 1;
                    }
                    value
                } else {
                    // The minimum instant, in the calendar system of the
                    // era (Julian before the cutover).
                    let mut d = gc.clone();
                    d.set_time_in_millis(i64::MIN);
                    d.complete()?;
                    let mut max_end = i64::from(d.internal_get(DAY_OF_YEAR) - 1) * 24
                        + i64::from(d.internal_get(HOUR_OF_DAY));
                    max_end *= 60;
                    max_end += i64::from(d.internal_get(MINUTE));
                    max_end *= 60;
                    max_end += i64::from(d.internal_get(SECOND));
                    max_end *= 1000;
                    max_end += i64::from(d.internal_get(MILLISECOND));
                    let mut value = d.internal_get(YEAR);
                    if current < max_end {
                        value -= 1;
                    }
                    value
                }
            }
            WEEK_OF_YEAR | WEEK_OF_MONTH | DAY_OF_WEEK_IN_MONTH => {
                return Err(LuceneError::unsupported_operation(format!(
                    "getActualMaximum({})",
                    FIELD_NAME[field]
                )));
            }
            _ => {
                return Err(LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!("Array index out of range: {field}"),
                ));
            }
        };
        Ok(value)
    }

    // ---------------------------------------------------- Gregorian helpers

    /// `GregorianCalendar.getCalendarDate(long fd)`.
    fn calendar_date(&self, fd: i64) -> BaseDate {
        if fd >= self.gregorian_cutover_date {
            gregorian_date_from_fixed(fd)
        } else {
            julian_date_from_fixed(fd)
        }
    }

    /// `GregorianCalendar.isLeapYear(int year)`.
    pub fn is_leap_year(&self, year: i32) -> bool {
        if (year & 3) != 0 {
            return false;
        }
        if year > self.gregorian_cutover_year {
            return (year % 100 != 0) || (year % 400 == 0);
        }
        if year < self.gregorian_cutover_year_julian {
            return true;
        }
        let gregorian = if self.gregorian_cutover_year == self.gregorian_cutover_year_julian {
            let d = self.calendar_date(self.gregorian_cutover_date);
            d.month < 3
        } else {
            year == self.gregorian_cutover_year
        };
        if gregorian {
            (year % 100 != 0) || (year % 400 == 0)
        } else {
            true
        }
    }

    /// `GregorianCalendar.monthLength(int month, int year)`.
    fn month_length_of(&self, month: i32, year: i32) -> i32 {
        if self.is_leap_year(year) {
            LEAP_MONTH_LENGTH[month as usize]
        } else {
            MONTH_LENGTH[month as usize]
        }
    }

    /// `GregorianCalendar.internalGetEra()`.
    fn internal_get_era(&self) -> i32 {
        if self.is_set(ERA) {
            self.internal_get(ERA)
        } else {
            match self.kind {
                CalendarKind::Japanese => *CURRENT_ERA,
                _ => AD,
            }
        }
    }

    /// `GregorianCalendar.getCutoverCalendarSystem()`: `true` for Julian.
    fn cutover_calendar_is_julian(&self) -> bool {
        self.gregorian_cutover_year_julian >= self.gregorian_cutover_year
    }

    /// `GregorianCalendar.getFixedDateJan1(date, fixedDate)`.
    fn fixed_date_jan1(&self, date: &BaseDate, fixed_date: i64) -> i64 {
        if self.gregorian_cutover_year != self.gregorian_cutover_year_julian
            && fixed_date >= self.gregorian_cutover_date
        {
            return self.gregorian_cutover_date;
        }
        julian_fixed_date(date.year, 1, 1)
    }

    /// `GregorianCalendar.getFixedDateMonth1(date, fixedDate)`.
    fn fixed_date_month1(&self, date: &BaseDate, fixed_date: i64) -> i64 {
        let g_cutover = self.calendar_date(self.gregorian_cutover_date);
        if g_cutover.month == 1 && g_cutover.day_of_month == 1 {
            return fixed_date - i64::from(date.day_of_month) + 1;
        }
        if date.month == g_cutover.month {
            let j_last_date = self.calendar_date(self.gregorian_cutover_date - 1);
            if self.gregorian_cutover_year == self.gregorian_cutover_year_julian
                && g_cutover.month == j_last_date.month
            {
                julian_fixed_date(date.year, date.month, 1)
            } else {
                self.gregorian_cutover_date
            }
        } else {
            fixed_date - i64::from(date.day_of_month) + 1
        }
    }

    /// `getWeekNumber(long fixedDay1, long fixedDate)`.
    fn week_number(&self, fixed_day1: i64, fixed_date: i64) -> i32 {
        let mut fixed_day1st =
            day_of_week_date_on_or_before(fixed_day1 + 6, self.first_day_of_week);
        let ndays = (fixed_day1st - fixed_day1) as i32;
        if ndays >= self.minimal_days_in_first_week {
            fixed_day1st -= 7;
        }
        let normalized_day_of_period = (fixed_date - fixed_day1st) as i32;
        if normalized_day_of_period >= 0 {
            return normalized_day_of_period / 7 + 1;
        }
        floor_divide(normalized_day_of_period, 7) + 1
    }

    // --------------------------------------------------------- computation

    /// `computeFields()`.
    fn compute_fields(&mut self) {
        let mask;
        if self.is_partially_normalized() {
            let set = self.get_set_state_fields();
            let field_mask = !set & ALL_FIELDS;
            let pending = match self.kind {
                CalendarKind::Japanese => self.cached_fixed_date == i64::MIN,
                _ => self.calsys_julian.is_none(),
            };
            let mut m = set;
            if field_mask != 0 || pending {
                m |= self
                    .compute_fields_masked(field_mask, set & (ZONE_OFFSET_MASK | DST_OFFSET_MASK));
            }
            mask = m;
        } else {
            mask = ALL_FIELDS;
            self.compute_fields_masked(mask, 0);
        }
        self.set_fields_computed(mask);
    }

    fn compute_fields_masked(&mut self, field_mask: i32, tz_mask: i32) -> i32 {
        match self.kind {
            CalendarKind::Japanese => self.japanese_compute_fields(field_mask, tz_mask),
            _ => self.gregorian_compute_fields(field_mask, tz_mask),
        }
    }

    /// The fixed date and time of day of `time` with the zone offsets: the
    /// first part of `computeFields(int, int)`.
    fn local_fixed_date(&mut self, tz_mask: i32) -> (i64, i32, [i32; 2]) {
        let mut zone_offset = 0;
        let mut zone_offsets = [0i32; 2];
        if tz_mask != (ZONE_OFFSET_MASK | DST_OFFSET_MASK) {
            let (total, offsets) = self.zone.get_offsets(self.time);
            zone_offset = total;
            zone_offsets = offsets;
        }
        if tz_mask != 0 {
            if (tz_mask & ZONE_OFFSET_MASK) != 0 {
                zone_offsets[0] = self.internal_get(ZONE_OFFSET);
            }
            if (tz_mask & DST_OFFSET_MASK) != 0 {
                zone_offsets[1] = self.internal_get(DST_OFFSET);
            }
            zone_offset = zone_offsets[0].wrapping_add(zone_offsets[1]);
        }
        let mut fixed_date = i64::from(zone_offset) / ONE_DAY;
        let mut time_of_day = zone_offset % ONE_DAY as i32;
        fixed_date += self.time / ONE_DAY;
        time_of_day += (self.time % ONE_DAY) as i32;
        if i64::from(time_of_day) >= ONE_DAY {
            time_of_day -= ONE_DAY as i32;
            fixed_date += 1;
        } else {
            while time_of_day < 0 {
                time_of_day += ONE_DAY as i32;
                fixed_date -= 1;
            }
        }
        fixed_date += EPOCH_OFFSET;
        (fixed_date, time_of_day, zone_offsets)
    }

    /// The time-of-day and zone-offset fields of `computeFields(int, int)`.
    fn set_time_fields(
        &mut self,
        field_mask: i32,
        time_of_day: i32,
        zone_offsets: [i32; 2],
        mask: &mut i32,
    ) {
        if (field_mask
            & (HOUR_OF_DAY_MASK
                | AM_PM_MASK
                | HOUR_MASK
                | MINUTE_MASK
                | SECOND_MASK
                | MILLISECOND_MASK))
            != 0
        {
            if time_of_day != 0 {
                let hours = time_of_day / ONE_HOUR;
                self.internal_set(HOUR_OF_DAY, hours);
                self.internal_set(AM_PM, hours / 12);
                self.internal_set(HOUR, hours % 12);
                let mut r = time_of_day % ONE_HOUR;
                self.internal_set(MINUTE, r / ONE_MINUTE);
                r %= ONE_MINUTE;
                self.internal_set(SECOND, r / ONE_SECOND);
                self.internal_set(MILLISECOND, r % ONE_SECOND);
            } else {
                self.internal_set(HOUR_OF_DAY, 0);
                self.internal_set(AM_PM, AM);
                self.internal_set(HOUR, 0);
                self.internal_set(MINUTE, 0);
                self.internal_set(SECOND, 0);
                self.internal_set(MILLISECOND, 0);
            }
            *mask |= HOUR_OF_DAY_MASK
                | AM_PM_MASK
                | HOUR_MASK
                | MINUTE_MASK
                | SECOND_MASK
                | MILLISECOND_MASK;
        }
        if (field_mask & (ZONE_OFFSET_MASK | DST_OFFSET_MASK)) != 0 {
            self.internal_set(ZONE_OFFSET, zone_offsets[0]);
            self.internal_set(DST_OFFSET, zone_offsets[1]);
            *mask |= ZONE_OFFSET_MASK | DST_OFFSET_MASK;
        }
    }

    /// `GregorianCalendar.computeFields(int fieldMask, int tzMask)`.
    fn gregorian_compute_fields(&mut self, field_mask: i32, tz_mask: i32) -> i32 {
        let (fixed_date, time_of_day, zone_offsets) = self.local_fixed_date(tz_mask);
        let mut era = AD;
        let year;
        let cdate;
        if fixed_date >= self.gregorian_cutover_date {
            cdate = gregorian_date_from_fixed(fixed_date);
            let mut y = cdate.year;
            if y <= 0 {
                y = 1 - y;
                era = BC;
            }
            year = y;
            self.calsys_julian = Some(false);
        } else {
            cdate = julian_date_from_fixed(fixed_date);
            if cdate.year <= 0 {
                era = BC;
                year = 1 - cdate.year;
            } else {
                year = cdate.year;
            }
            self.calsys_julian = Some(true);
        }
        self.cdate = cdate;
        self.internal_set(ERA, era);
        self.internal_set(YEAR, year);
        let mut mask = field_mask | (ERA_MASK | YEAR_MASK);
        let month = cdate.month - 1;
        let day_of_month = cdate.day_of_month;
        if (field_mask & (MONTH_MASK | DAY_OF_MONTH_MASK | DAY_OF_WEEK_MASK)) != 0 {
            self.internal_set(MONTH, month);
            self.internal_set(DAY_OF_MONTH, day_of_month);
            self.internal_set(DAY_OF_WEEK, cdate.day_of_week);
            mask |= MONTH_MASK | DAY_OF_MONTH_MASK | DAY_OF_WEEK_MASK;
        }
        self.set_time_fields(field_mask, time_of_day, zone_offsets, &mut mask);
        if (field_mask
            & (DAY_OF_YEAR_MASK
                | WEEK_OF_YEAR_MASK
                | WEEK_OF_MONTH_MASK
                | DAY_OF_WEEK_IN_MONTH_MASK))
            != 0
        {
            let julian = self.calsys_julian == Some(true);
            let normalized_year = cdate.year;
            let mut fixed_date_jan1 = if julian {
                julian_fixed_date(normalized_year, 1, 1)
            } else {
                gregorian_fixed_date(normalized_year, 1, 1)
            };
            let mut day_of_year = (fixed_date - fixed_date_jan1) as i32 + 1;
            let mut fixed_date_month1 = fixed_date - i64::from(day_of_month) + 1;
            let cutover_year = if julian {
                self.gregorian_cutover_year_julian
            } else {
                self.gregorian_cutover_year
            };
            let mut relative_day_of_month = day_of_month - 1;
            if normalized_year == cutover_year {
                if self.gregorian_cutover_year_julian <= self.gregorian_cutover_year {
                    fixed_date_jan1 = self.fixed_date_jan1(&cdate, fixed_date);
                    if fixed_date >= self.gregorian_cutover_date {
                        fixed_date_month1 = self.fixed_date_month1(&cdate, fixed_date);
                    }
                }
                day_of_year = (fixed_date - fixed_date_jan1) as i32 + 1;
                relative_day_of_month = (fixed_date - fixed_date_month1) as i32;
            }
            self.internal_set(DAY_OF_YEAR, day_of_year);
            self.internal_set(DAY_OF_WEEK_IN_MONTH, relative_day_of_month / 7 + 1);
            let mut week_of_year = self.week_number(fixed_date_jan1, fixed_date);
            if week_of_year == 0 {
                let fixed_dec31 = fixed_date_jan1 - 1;
                let mut prev_jan1 = fixed_date_jan1 - 365;
                if normalized_year > (cutover_year + 1) {
                    if is_gregorian_leap_year(normalized_year - 1) {
                        prev_jan1 -= 1;
                    }
                } else if normalized_year <= self.gregorian_cutover_year_julian {
                    if is_julian_leap_year(normalized_year - 1) {
                        prev_jan1 -= 1;
                    }
                } else {
                    let prev_year = self.calendar_date(fixed_dec31).year;
                    if prev_year == self.gregorian_cutover_year {
                        if self.cutover_calendar_is_julian() {
                            prev_jan1 = julian_fixed_date(prev_year, 1, 1);
                        } else {
                            prev_jan1 = self.gregorian_cutover_date;
                        }
                    } else if prev_year <= self.gregorian_cutover_year_julian {
                        prev_jan1 = julian_fixed_date(prev_year, 1, 1);
                    }
                }
                week_of_year = self.week_number(prev_jan1, fixed_dec31);
            } else if normalized_year > self.gregorian_cutover_year
                || normalized_year < (self.gregorian_cutover_year_julian - 1)
            {
                if week_of_year >= 52 {
                    let mut next_jan1 = fixed_date_jan1 + 365;
                    if cdate.leap_year {
                        next_jan1 += 1;
                    }
                    let next_jan1st =
                        day_of_week_date_on_or_before(next_jan1 + 6, self.first_day_of_week);
                    let ndays = (next_jan1st - next_jan1) as i32;
                    if ndays >= self.minimal_days_in_first_week && fixed_date >= (next_jan1st - 7) {
                        week_of_year = 1;
                    }
                }
            } else {
                let mut next_year = normalized_year + 1;
                if next_year == (self.gregorian_cutover_year_julian + 1)
                    && next_year < self.gregorian_cutover_year
                {
                    next_year = self.gregorian_cutover_year;
                }
                let mut cal_julian = julian;
                if next_year == self.gregorian_cutover_year {
                    cal_julian = self.cutover_calendar_is_julian();
                }
                let next_jan1 = if next_year > self.gregorian_cutover_year
                    || self.gregorian_cutover_year_julian == self.gregorian_cutover_year
                    || next_year == self.gregorian_cutover_year_julian
                {
                    if cal_julian {
                        julian_fixed_date(next_year, 1, 1)
                    } else {
                        gregorian_fixed_date(next_year, 1, 1)
                    }
                } else {
                    self.gregorian_cutover_date
                };
                let next_jan1st =
                    day_of_week_date_on_or_before(next_jan1 + 6, self.first_day_of_week);
                let ndays = (next_jan1st - next_jan1) as i32;
                if ndays >= self.minimal_days_in_first_week && fixed_date >= (next_jan1st - 7) {
                    week_of_year = 1;
                }
            }
            self.internal_set(WEEK_OF_YEAR, week_of_year);
            let week_of_month = self.week_number(fixed_date_month1, fixed_date);
            self.internal_set(WEEK_OF_MONTH, week_of_month);
            mask |= DAY_OF_YEAR_MASK
                | WEEK_OF_YEAR_MASK
                | WEEK_OF_MONTH_MASK
                | DAY_OF_WEEK_IN_MONTH_MASK;
        }
        mask
    }

    /// The non-lenient check `computeTime` starts with: the externally set
    /// fields must be within `getMinimum`..`getMaximum`; returns the field
    /// values to compare after the computation.
    fn non_lenient_check(&self) -> Result<Option<[i32; FIELD_COUNT]>> {
        if self.lenient {
            return Ok(None);
        }
        let mut original = [0i32; FIELD_COUNT];
        for (field, slot) in original.iter_mut().enumerate() {
            let value = self.internal_get(field);
            if self.is_externally_set(field)
                && (value < self.get_minimum(field) || value > self.get_maximum(field))
            {
                return Err(LuceneError::illegal_argument(FIELD_NAME[field].to_string()));
            }
            *slot = value;
        }
        Ok(Some(original))
    }

    /// The time of day the fields give, in milliseconds, and the fixed-date
    /// carry of it: the part of `computeTime` common to the calendars.
    fn time_of_day_of_fields(&self, field_mask: i32) -> (i64, i64) {
        let mut time_of_day: i64 = 0;
        if (field_mask & HOUR_OF_DAY_MASK) != 0 {
            time_of_day += i64::from(self.internal_get(HOUR_OF_DAY));
        } else {
            time_of_day += i64::from(self.internal_get(HOUR));
            if (field_mask & AM_PM_MASK) != 0 {
                time_of_day += i64::from(12i32.wrapping_mul(self.internal_get(AM_PM)));
            }
        }
        time_of_day = time_of_day.wrapping_mul(60);
        time_of_day = time_of_day.wrapping_add(i64::from(self.internal_get(MINUTE)));
        time_of_day = time_of_day.wrapping_mul(60);
        time_of_day = time_of_day.wrapping_add(i64::from(self.internal_get(SECOND)));
        time_of_day = time_of_day.wrapping_mul(1000);
        time_of_day = time_of_day.wrapping_add(i64::from(self.internal_get(MILLISECOND)));
        let mut fixed_date = time_of_day / ONE_DAY;
        time_of_day %= ONE_DAY;
        while time_of_day < 0 {
            time_of_day += ONE_DAY;
            fixed_date -= 1;
        }
        (fixed_date, time_of_day)
    }

    /// The end of `computeTime`: the zone offsets of the wall time `millis`,
    /// the time, and the fields recomputed and checked.
    fn finish_compute_time(
        &mut self,
        mut millis: i64,
        field_mask: i32,
        original: Option<[i32; FIELD_COUNT]>,
    ) -> Result<()> {
        let mut zone_offsets = [0i32; 2];
        let tz_mask = field_mask & (ZONE_OFFSET_MASK | DST_OFFSET_MASK);
        if tz_mask != (ZONE_OFFSET_MASK | DST_OFFSET_MASK) {
            let (_, offsets) = self.zone.get_offsets_by_wall(millis);
            zone_offsets = offsets;
        }
        if tz_mask != 0 {
            if (tz_mask & ZONE_OFFSET_MASK) != 0 {
                zone_offsets[0] = self.internal_get(ZONE_OFFSET);
            }
            if (tz_mask & DST_OFFSET_MASK) != 0 {
                zone_offsets[1] = self.internal_get(DST_OFFSET);
            }
        }
        millis = millis.wrapping_sub(i64::from(zone_offsets[0].wrapping_add(zone_offsets[1])));
        self.time = millis;
        let mask = self.compute_fields_masked(field_mask | self.get_set_state_fields(), tz_mask);
        if let Some(original) = original {
            for (field, &orig) in original.iter().enumerate() {
                if !self.is_externally_set(field) {
                    continue;
                }
                if orig != self.internal_get(field) {
                    let message = match self.kind {
                        CalendarKind::Japanese => format!(
                            "{}={}, expected {}",
                            FIELD_NAME[field],
                            self.internal_get(field),
                            orig
                        ),
                        _ => format!(
                            "{}: {} -> {}",
                            FIELD_NAME[field],
                            orig,
                            self.internal_get(field)
                        ),
                    };
                    self.fields = original;
                    return Err(LuceneError::illegal_argument(message));
                }
            }
        }
        self.set_fields_normalized(mask);
        Ok(())
    }

    /// `computeTime()`.
    fn compute_time(&mut self) -> Result<()> {
        match self.kind {
            CalendarKind::Japanese => self.japanese_compute_time(),
            _ => self.gregorian_compute_time(),
        }
    }

    /// `GregorianCalendar.computeTime()`.
    fn gregorian_compute_time(&mut self) -> Result<()> {
        let original = self.non_lenient_check()?;
        let mut field_mask = self.select_fields();
        let mut year = if self.is_set(YEAR) {
            self.internal_get(YEAR)
        } else {
            EPOCH_YEAR
        };
        let era = self.internal_get_era();
        if era == BC {
            year = 1i32.wrapping_sub(year);
        } else if era != AD {
            return Err(LuceneError::illegal_argument("Invalid era"));
        }
        if year <= 0 && !self.is_set(ERA) {
            field_mask |= ERA_MASK;
            self.set_fields_computed(ERA_MASK);
        }
        let (mut fixed_date, time_of_day) = self.time_of_day_of_fields(field_mask);
        'calculate: {
            let gfd;
            let jfd;
            if year > self.gregorian_cutover_year && year > self.gregorian_cutover_year_julian {
                let g = fixed_date + self.gregorian_fixed_date_of_fields(false, year, field_mask);
                if g >= self.gregorian_cutover_date {
                    fixed_date = g;
                    break 'calculate;
                }
                gfd = g;
                jfd = fixed_date + self.gregorian_fixed_date_of_fields(true, year, field_mask);
            } else if year < self.gregorian_cutover_year
                && year < self.gregorian_cutover_year_julian
            {
                let j = fixed_date + self.gregorian_fixed_date_of_fields(true, year, field_mask);
                if j < self.gregorian_cutover_date {
                    fixed_date = j;
                    break 'calculate;
                }
                jfd = j;
                gfd = j;
            } else {
                jfd = fixed_date + self.gregorian_fixed_date_of_fields(true, year, field_mask);
                gfd = fixed_date + self.gregorian_fixed_date_of_fields(false, year, field_mask);
            }
            if (field_mask & DAY_OF_YEAR_MASK) != 0 || (field_mask & WEEK_OF_YEAR_MASK) != 0 {
                if self.gregorian_cutover_year == self.gregorian_cutover_year_julian {
                    fixed_date = jfd;
                    break 'calculate;
                } else if year == self.gregorian_cutover_year {
                    fixed_date = gfd;
                    break 'calculate;
                }
            }
            if gfd >= self.gregorian_cutover_date {
                // `calsys == gcal || calsys == null` takes the Gregorian date
                if jfd >= self.gregorian_cutover_date || self.calsys_julian != Some(true) {
                    fixed_date = gfd;
                } else {
                    fixed_date = jfd;
                }
            } else if jfd < self.gregorian_cutover_date {
                fixed_date = jfd;
            } else {
                if !self.lenient {
                    return Err(LuceneError::illegal_argument(
                        "the specified date doesn't exist",
                    ));
                }
                fixed_date = jfd;
            }
        }
        let millis = (fixed_date - EPOCH_OFFSET)
            .wrapping_mul(ONE_DAY)
            .wrapping_add(time_of_day);
        self.finish_compute_time(millis, field_mask, original)
    }

    /// `GregorianCalendar.getFixedDate(BaseCalendar cal, int year, int
    /// fieldMask)`: `julian` selects the Julian calendar.
    fn gregorian_fixed_date_of_fields(&self, julian: bool, year: i32, field_mask: i32) -> i64 {
        let mut year = year;
        let mut month = JANUARY;
        if (field_mask & MONTH_MASK) != 0 {
            month = self.internal_get(MONTH);
            if month > DECEMBER {
                year = year.wrapping_add(month / 12);
                month %= 12;
            } else if month < JANUARY {
                let (q, r) = floor_divide_rem(month, 12);
                year = year.wrapping_add(q);
                month = r;
            }
        }
        let fd_of = |y: i32, m: i32, d: i32| {
            if julian {
                julian_fixed_date(y, m, d)
            } else {
                gregorian_fixed_date(y, m, d)
            }
        };
        let mut fixed_date = fd_of(year, month + 1, 1);
        if (field_mask & MONTH_MASK) != 0 {
            if (field_mask & DAY_OF_MONTH_MASK) != 0 {
                if self.is_set(DAY_OF_MONTH) {
                    fixed_date += i64::from(self.internal_get(DAY_OF_MONTH));
                    fixed_date -= 1;
                }
            } else if (field_mask & WEEK_OF_MONTH_MASK) != 0 {
                let mut first_day_of_week =
                    day_of_week_date_on_or_before(fixed_date + 6, self.first_day_of_week);
                if (first_day_of_week - fixed_date) >= i64::from(self.minimal_days_in_first_week) {
                    first_day_of_week -= 7;
                }
                if (field_mask & DAY_OF_WEEK_MASK) != 0 {
                    first_day_of_week = day_of_week_date_on_or_before(
                        first_day_of_week + 6,
                        self.internal_get(DAY_OF_WEEK),
                    );
                }
                fixed_date =
                    first_day_of_week + 7 * i64::from(self.internal_get(WEEK_OF_MONTH) - 1);
            } else {
                let day_of_week = if (field_mask & DAY_OF_WEEK_MASK) != 0 {
                    self.internal_get(DAY_OF_WEEK)
                } else {
                    self.first_day_of_week
                };
                let dowim = if (field_mask & DAY_OF_WEEK_IN_MONTH_MASK) != 0 {
                    self.internal_get(DAY_OF_WEEK_IN_MONTH)
                } else {
                    1
                };
                if dowim >= 0 {
                    fixed_date = day_of_week_date_on_or_before(
                        fixed_date + (7 * i64::from(dowim)) - 1,
                        day_of_week,
                    );
                } else {
                    let last_date = self.month_length_of(month, year) + (7 * (dowim + 1));
                    fixed_date = day_of_week_date_on_or_before(
                        fixed_date + i64::from(last_date) - 1,
                        day_of_week,
                    );
                }
            }
        } else {
            if year == self.gregorian_cutover_year
                && !julian
                && fixed_date < self.gregorian_cutover_date
                && self.gregorian_cutover_year != self.gregorian_cutover_year_julian
            {
                fixed_date = self.gregorian_cutover_date;
            }
            if (field_mask & DAY_OF_YEAR_MASK) != 0 {
                fixed_date += i64::from(self.internal_get(DAY_OF_YEAR));
                fixed_date -= 1;
            } else {
                let mut first_day_of_week =
                    day_of_week_date_on_or_before(fixed_date + 6, self.first_day_of_week);
                if (first_day_of_week - fixed_date) >= i64::from(self.minimal_days_in_first_week) {
                    first_day_of_week -= 7;
                }
                if (field_mask & DAY_OF_WEEK_MASK) != 0 {
                    let day_of_week = self.internal_get(DAY_OF_WEEK);
                    if day_of_week != self.first_day_of_week {
                        first_day_of_week =
                            day_of_week_date_on_or_before(first_day_of_week + 6, day_of_week);
                    }
                }
                fixed_date =
                    first_day_of_week + 7 * (i64::from(self.internal_get(WEEK_OF_YEAR)) - 1);
            }
        }
        fixed_date
    }

    // ---------------------------------------------------- Japanese calendar

    /// `JapaneseImperialCalendar.isTransitionYear(int)`.
    fn is_transition_year(normalized_year: i32) -> bool {
        for i in (1..=JAPANESE_ERA_COUNT).rev() {
            let transition_year = JAPANESE_ERAS[i - 1].since_year;
            if normalized_year == transition_year {
                return true;
            }
            if normalized_year > transition_year {
                break;
            }
        }
        false
    }

    /// `JapaneseImperialCalendar.getTransitionEraIndex(date)`.
    fn transition_era_index(date: &LocalDate) -> i32 {
        let mut index = era_index(date);
        let transition_date = japanese_since_date(index);
        if transition_date.year == date.gregorian_year && transition_date.month == date.month {
            return index;
        }
        if index < JAPANESE_ERA_COUNT as i32 {
            index += 1;
            let transition_date = japanese_since_date(index);
            if transition_date.year == date.gregorian_year && transition_date.month == date.month {
                return index;
            }
        }
        -1
    }

    /// `JapaneseImperialCalendar.getFixedDateJan1(date, fixedDate)`.
    fn japanese_fixed_date_jan1(date: &LocalDate, fixed_date: i64) -> i64 {
        if date.era.is_some() && date.year == 1 {
            let mut index = era_index(date);
            while index > 0 {
                let d = japanese_since_date(index);
                let fd = gregorian_fixed_date(d.year, d.month, d.day_of_month);
                if fd > fixed_date {
                    index -= 1;
                    continue;
                }
                return fd;
            }
        }
        gregorian_fixed_date(date.gregorian_year, 1, 1)
    }

    /// `JapaneseImperialCalendar.getFixedDateMonth1(date, fixedDate)`.
    fn japanese_fixed_date_month1(date: &LocalDate, fixed_date: i64) -> i64 {
        let index = Self::transition_era_index(date);
        if index != -1 {
            let transition = SINCE_FIXED_DATES[index as usize];
            if transition <= fixed_date {
                return transition;
            }
        }
        fixed_date - i64::from(date.day_of_month) + 1
    }

    /// `JapaneseImperialCalendar.computeFields(int fieldMask, int tzMask)`.
    fn japanese_compute_fields(&mut self, field_mask: i32, tz_mask: i32) -> i32 {
        let (fixed_date, time_of_day, zone_offsets) = self.local_fixed_date(tz_mask);
        if fixed_date != self.cached_fixed_date || fixed_date < 0 {
            self.jdate = local_date_from_fixed(fixed_date);
            self.cached_fixed_date = fixed_date;
        }
        let jdate = self.jdate;
        let era = era_index(&jdate);
        let year = jdate.year;
        self.internal_set(ERA, era);
        self.internal_set(YEAR, year);
        let mut mask = field_mask | (ERA_MASK | YEAR_MASK);
        let month = jdate.month - 1;
        let day_of_month = jdate.day_of_month;
        if (field_mask & (MONTH_MASK | DAY_OF_MONTH_MASK | DAY_OF_WEEK_MASK)) != 0 {
            self.internal_set(MONTH, month);
            self.internal_set(DAY_OF_MONTH, day_of_month);
            self.internal_set(DAY_OF_WEEK, jdate.day_of_week);
            mask |= MONTH_MASK | DAY_OF_MONTH_MASK | DAY_OF_WEEK_MASK;
        }
        self.set_time_fields(field_mask, time_of_day, zone_offsets, &mut mask);
        if (field_mask
            & (DAY_OF_YEAR_MASK
                | WEEK_OF_YEAR_MASK
                | WEEK_OF_MONTH_MASK
                | DAY_OF_WEEK_IN_MONTH_MASK))
            != 0
        {
            let normalized_year = jdate.gregorian_year;
            let transition_year = Self::is_transition_year(jdate.gregorian_year);
            let day_of_year;
            let fixed_date_jan1;
            if transition_year {
                fixed_date_jan1 = Self::japanese_fixed_date_jan1(&jdate, fixed_date);
                day_of_year = (fixed_date - fixed_date_jan1) as i32 + 1;
            } else if normalized_year == JAPANESE_MIN_VALUES[YEAR] {
                let dx = local_date_of_millis(i64::MIN, &self.zone);
                fixed_date_jan1 =
                    gregorian_fixed_date(dx.gregorian_year, dx.month, dx.day_of_month);
                day_of_year = (fixed_date - fixed_date_jan1) as i32 + 1;
            } else {
                day_of_year =
                    gregorian_day_of_year(jdate.gregorian_year, jdate.month, jdate.day_of_month)
                        as i32;
                fixed_date_jan1 = fixed_date - i64::from(day_of_year) + 1;
            }
            let fixed_date_month1 = if transition_year {
                Self::japanese_fixed_date_month1(&jdate, fixed_date)
            } else {
                fixed_date - i64::from(day_of_month) + 1
            };
            self.internal_set(DAY_OF_YEAR, day_of_year);
            self.internal_set(DAY_OF_WEEK_IN_MONTH, (day_of_month - 1) / 7 + 1);
            let mut week_of_year = self.week_number(fixed_date_jan1, fixed_date);
            if week_of_year == 0 {
                let fixed_dec31 = fixed_date_jan1 - 1;
                let prev_jan1;
                let mut d = local_date_from_fixed(fixed_dec31);
                if !(transition_year || Self::is_transition_year(d.gregorian_year)) {
                    let mut p = fixed_date_jan1 - 365;
                    if d.leap_year {
                        p -= 1;
                    }
                    prev_jan1 = p;
                } else if transition_year {
                    if jdate.year == 1 {
                        if era > REIWA {
                            let pd = japanese_since_date(era - 1);
                            if normalized_year == pd.year {
                                d.set_month(pd.month);
                                d.set_day_of_month(pd.day_of_month);
                            }
                        } else {
                            d.set_month(1);
                            d.set_day_of_month(1);
                        }
                        local_normalize(&mut d);
                        prev_jan1 = local_fixed_date(&mut d);
                    } else {
                        let mut p = fixed_date_jan1 - 365;
                        if d.leap_year {
                            p -= 1;
                        }
                        prev_jan1 = p;
                    }
                } else {
                    let cd = japanese_since_date(era_index(&jdate));
                    d.set_month(cd.month);
                    d.set_day_of_month(cd.day_of_month);
                    local_normalize(&mut d);
                    prev_jan1 = local_fixed_date(&mut d);
                }
                week_of_year = self.week_number(prev_jan1, fixed_dec31);
            } else if !transition_year {
                if week_of_year >= 52 {
                    let mut next_jan1 = fixed_date_jan1 + 365;
                    if jdate.leap_year {
                        next_jan1 += 1;
                    }
                    let next_jan1st =
                        day_of_week_date_on_or_before(next_jan1 + 6, self.first_day_of_week);
                    let ndays = (next_jan1st - next_jan1) as i32;
                    if ndays >= self.minimal_days_in_first_week && fixed_date >= (next_jan1st - 7) {
                        week_of_year = 1;
                    }
                }
            } else {
                let mut d = jdate;
                let next_jan1 = if jdate.year == 1 {
                    d.add_year(1);
                    d.set_month(1);
                    d.set_day_of_month(1);
                    local_fixed_date(&mut d)
                } else {
                    let next_era_index = era_index(&d) + 1;
                    let cd = japanese_since_date(next_era_index);
                    d.set_era(Some(next_era_index as usize - 1));
                    d.set_date(1, cd.month, cd.day_of_month);
                    local_normalize(&mut d);
                    local_fixed_date(&mut d)
                };
                let next_jan1st =
                    day_of_week_date_on_or_before(next_jan1 + 6, self.first_day_of_week);
                let ndays = (next_jan1st - next_jan1) as i32;
                if ndays >= self.minimal_days_in_first_week && fixed_date >= (next_jan1st - 7) {
                    week_of_year = 1;
                }
            }
            self.internal_set(WEEK_OF_YEAR, week_of_year);
            let week_of_month = self.week_number(fixed_date_month1, fixed_date);
            self.internal_set(WEEK_OF_MONTH, week_of_month);
            mask |= DAY_OF_YEAR_MASK
                | WEEK_OF_YEAR_MASK
                | WEEK_OF_MONTH_MASK
                | DAY_OF_WEEK_IN_MONTH_MASK;
        }
        mask
    }

    /// `JapaneseImperialCalendar.computeTime()`.
    fn japanese_compute_time(&mut self) -> Result<()> {
        let original = self.non_lenient_check()?;
        let field_mask = self.select_fields();
        let year;
        let era;
        if self.is_set(ERA) {
            era = self.internal_get(ERA);
            if era > JAPANESE_ERA_COUNT as i32 {
                return Err(LuceneError::illegal_argument("Invalid era"));
            }
            year = if self.is_set(YEAR) {
                self.internal_get(YEAR)
            } else {
                1
            };
        } else if self.is_set(YEAR) {
            era = *CURRENT_ERA;
            year = self.internal_get(YEAR);
        } else {
            era = SHOWA;
            year = 45;
        }
        let (mut fixed_date, time_of_day) = self.time_of_day_of_fields(field_mask);
        fixed_date += self.japanese_fixed_date_of_fields(era, year, field_mask);
        let millis = (fixed_date - EPOCH_OFFSET)
            .wrapping_mul(ONE_DAY)
            .wrapping_add(time_of_day);
        self.finish_compute_time(millis, field_mask, original)
    }

    /// `JapaneseImperialCalendar.getFixedDate(int era, int year, int
    /// fieldMask)`.
    fn japanese_fixed_date_of_fields(&self, era: i32, year: i32, field_mask: i32) -> i64 {
        let mut year = year;
        let mut month = JANUARY;
        let mut first_day_of_month = 1;
        if (field_mask & MONTH_MASK) != 0 {
            month = self.internal_get(MONTH);
            if month > DECEMBER {
                year = year.wrapping_add(month / 12);
                month %= 12;
            } else if month < JANUARY {
                let (q, r) = floor_divide_rem(month, 12);
                year = year.wrapping_add(q);
                month = r;
            }
        } else if year == 1 && era != 0 {
            let d = japanese_since_date(era);
            month = d.month - 1;
            first_day_of_month = d.day_of_month;
        }
        if year == JAPANESE_MIN_VALUES[YEAR] {
            let dx = local_date_of_millis(i64::MIN, &self.zone);
            let m = dx.month - 1;
            if month < m {
                month = m;
            }
            if month == m {
                first_day_of_month = dx.day_of_month;
            }
        }
        let mut date = LocalDate::new();
        date.set_era(if era > 0 {
            Some(era as usize - 1)
        } else {
            None
        });
        date.set_date(year, month + 1, first_day_of_month);
        local_normalize(&mut date);
        let mut fixed_date = local_fixed_date(&mut date);
        if (field_mask & MONTH_MASK) != 0 {
            if (field_mask & DAY_OF_MONTH_MASK) != 0 {
                if self.is_set(DAY_OF_MONTH) {
                    fixed_date += i64::from(self.internal_get(DAY_OF_MONTH));
                    fixed_date -= i64::from(first_day_of_month);
                }
            } else if (field_mask & WEEK_OF_MONTH_MASK) != 0 {
                let mut first_day_of_week =
                    day_of_week_date_on_or_before(fixed_date + 6, self.first_day_of_week);
                if (first_day_of_week - fixed_date) >= i64::from(self.minimal_days_in_first_week) {
                    first_day_of_week -= 7;
                }
                if (field_mask & DAY_OF_WEEK_MASK) != 0 {
                    first_day_of_week = day_of_week_date_on_or_before(
                        first_day_of_week + 6,
                        self.internal_get(DAY_OF_WEEK),
                    );
                }
                fixed_date =
                    first_day_of_week + 7 * i64::from(self.internal_get(WEEK_OF_MONTH) - 1);
            } else {
                let day_of_week = if (field_mask & DAY_OF_WEEK_MASK) != 0 {
                    self.internal_get(DAY_OF_WEEK)
                } else {
                    self.first_day_of_week
                };
                let dowim = if (field_mask & DAY_OF_WEEK_IN_MONTH_MASK) != 0 {
                    self.internal_get(DAY_OF_WEEK_IN_MONTH)
                } else {
                    1
                };
                if dowim >= 0 {
                    fixed_date = day_of_week_date_on_or_before(
                        fixed_date + (7 * i64::from(dowim)) - 1,
                        day_of_week,
                    );
                } else {
                    // `monthLength(month, year)` with the year of the era,
                    // as Java passes it
                    let length = if is_gregorian_leap_year(year) {
                        LEAP_MONTH_LENGTH[month as usize]
                    } else {
                        MONTH_LENGTH[month as usize]
                    };
                    let last_date = length + (7 * (dowim + 1));
                    fixed_date = day_of_week_date_on_or_before(
                        fixed_date + i64::from(last_date) - 1,
                        day_of_week,
                    );
                }
            }
        } else if (field_mask & DAY_OF_YEAR_MASK) != 0 {
            if Self::is_transition_year(date.gregorian_year) {
                fixed_date = Self::japanese_fixed_date_jan1(&date, fixed_date);
            }
            fixed_date += i64::from(self.internal_get(DAY_OF_YEAR));
            fixed_date -= 1;
        } else {
            let mut first_day_of_week =
                day_of_week_date_on_or_before(fixed_date + 6, self.first_day_of_week);
            if (first_day_of_week - fixed_date) >= i64::from(self.minimal_days_in_first_week) {
                first_day_of_week -= 7;
            }
            if (field_mask & DAY_OF_WEEK_MASK) != 0 {
                let day_of_week = self.internal_get(DAY_OF_WEEK);
                if day_of_week != self.first_day_of_week {
                    first_day_of_week =
                        day_of_week_date_on_or_before(first_day_of_week + 6, day_of_week);
                }
            }
            fixed_date = first_day_of_week + 7 * (i64::from(self.internal_get(WEEK_OF_YEAR)) - 1);
        }
        fixed_date
    }

    /// `JapaneseImperialCalendar.pinDayOfMonth(date)`.
    fn japanese_pin_day_of_month(&self, date: &mut LocalDate) -> Result<()> {
        let year = date.year;
        let dom = date.day_of_month;
        if year != self.get_minimum(YEAR) {
            date.set_day_of_month(1);
            local_normalize(date);
            let month_length = local_month_length(date)?;
            if dom > month_length {
                date.set_day_of_month(month_length);
            } else {
                date.set_day_of_month(dom);
            }
            local_normalize(date);
        } else {
            let d = local_date_of_millis(i64::MIN, &self.zone);
            let mut real_date = local_date_of_millis(self.time, &self.zone);
            real_date.add_year(400);
            real_date.set_month(date.month);
            real_date.set_day_of_month(1);
            local_normalize(&mut real_date);
            let month_length = local_month_length(&real_date)?;
            if dom > month_length {
                real_date.set_day_of_month(month_length);
            } else if dom < d.day_of_month {
                real_date.set_day_of_month(d.day_of_month);
            } else {
                real_date.set_day_of_month(dom);
            }
            // both times of day are those of the instants; the time of day of
            // `Long.MIN_VALUE` in the zone is compared to that of `time`
            let tod = mod_l(
                self.time
                    .wrapping_add(i64::from(self.zone.get_offsets(self.time).0)),
                ONE_DAY,
            );
            let min_tod = mod_l(
                i64::MIN.wrapping_add(i64::from(self.zone.get_offsets(i64::MIN).0)),
                ONE_DAY,
            );
            if real_date.day_of_month == d.day_of_month && tod < min_tod {
                real_date.set_day_of_month((dom + 1).min(month_length));
            }
            date.set_date(year, real_date.month, real_date.day_of_month);
        }
        Ok(())
    }

    // ------------------------------------------------------------------ add

    /// `add(int field, int amount)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a field from `ZONE_OFFSET` on, and the
    /// errors of [`get`](Self::get).
    pub fn add(&mut self, field: usize, amount: i32) -> Result<()> {
        if self.kind == CalendarKind::Buddhist {
            let saved = self.year_offset;
            self.year_offset = 0;
            let r = self.add_impl(field, amount);
            self.year_offset = saved;
            return r;
        }
        self.add_impl(field, amount)
    }

    fn add_impl(&mut self, field: usize, amount: i32) -> Result<()> {
        if amount == 0 {
            return Ok(());
        }
        if field >= ZONE_OFFSET {
            return Err(LuceneError::illegal_argument(String::new()));
        }
        self.complete()?;
        if self.kind == CalendarKind::Japanese {
            match field {
                YEAR | MONTH => {
                    let mut d = self.jdate;
                    if field == YEAR {
                        d.add_year(amount);
                    } else {
                        d.add_month(amount);
                    }
                    self.japanese_pin_day_of_month(&mut d)?;
                    self.set(ERA, era_index(&d));
                    self.set(YEAR, d.year);
                    self.set(MONTH, d.month - 1);
                    self.set(DAY_OF_MONTH, d.day_of_month);
                    return Ok(());
                }
                ERA => {
                    let mut era = self.internal_get(ERA).wrapping_add(amount);
                    if era < 0 {
                        era = 0;
                    } else if era > JAPANESE_ERA_COUNT as i32 {
                        era = JAPANESE_ERA_COUNT as i32;
                    }
                    self.set(ERA, era);
                    return Ok(());
                }
                _ => return self.add_time_based(field, amount, self.cached_fixed_date),
            }
        }
        match field {
            YEAR => {
                let mut year = self.internal_get(YEAR);
                if self.internal_get_era() == AD {
                    year = year.wrapping_add(amount);
                    if year > 0 {
                        self.set(YEAR, year);
                    } else {
                        self.set(YEAR, 1 - year);
                        self.set(ERA, BC);
                    }
                } else {
                    year = year.wrapping_sub(amount);
                    if year > 0 {
                        self.set(YEAR, year);
                    } else {
                        self.set(YEAR, 1 - year);
                        self.set(ERA, AD);
                    }
                }
                self.pin_day_of_month()
            }
            MONTH => {
                let mut month = self.internal_get(MONTH).wrapping_add(amount);
                let mut year = self.internal_get(YEAR);
                let y_amount = if month >= 0 {
                    month / 12
                } else {
                    (month + 1) / 12 - 1
                };
                if y_amount != 0 {
                    if self.internal_get_era() == AD {
                        year = year.wrapping_add(y_amount);
                        if year > 0 {
                            self.set(YEAR, year);
                        } else {
                            self.set(YEAR, 1 - year);
                            self.set(ERA, BC);
                        }
                    } else {
                        year = year.wrapping_sub(y_amount);
                        if year > 0 {
                            self.set(YEAR, year);
                        } else {
                            self.set(YEAR, 1 - year);
                            self.set(ERA, AD);
                        }
                    }
                }
                if month >= 0 {
                    self.set(MONTH, month % 12);
                } else {
                    month %= 12;
                    if month < 0 {
                        month += 12;
                    }
                    self.set(MONTH, JANUARY + month);
                }
                self.pin_day_of_month()
            }
            ERA => {
                let era = self.internal_get(ERA).wrapping_add(amount).clamp(0, 1);
                self.set(ERA, era);
                Ok(())
            }
            _ => {
                let fd = if self.calsys_julian == Some(true) {
                    julian_fixed_date(self.cdate.year, self.cdate.month, self.cdate.day_of_month)
                } else {
                    gregorian_fixed_date(self.cdate.year, self.cdate.month, self.cdate.day_of_month)
                };
                self.add_time_based(field, amount, fd)
            }
        }
    }

    /// The `else` branch of `add`: a time- or day-based field, from the
    /// current fixed date `fd`.
    fn add_time_based(&mut self, field: usize, amount: i32, fd: i64) -> Result<()> {
        let mut delta = i64::from(amount);
        let mut time_of_day: i64 = 0;
        match field {
            HOUR | HOUR_OF_DAY => delta = delta.wrapping_mul(60 * 60 * 1000),
            MINUTE => delta = delta.wrapping_mul(60 * 1000),
            SECOND => delta = delta.wrapping_mul(1000),
            WEEK_OF_YEAR | WEEK_OF_MONTH | DAY_OF_WEEK_IN_MONTH => delta = delta.wrapping_mul(7),
            AM_PM => {
                delta = i64::from(amount / 2);
                time_of_day = i64::from(12 * (amount % 2));
            }
            _ => {}
        }
        if field >= HOUR {
            let t = self.time.wrapping_add(delta);
            self.set_time_in_millis(t);
            return Ok(());
        }
        let mut fd = fd;
        time_of_day += i64::from(self.internal_get(HOUR_OF_DAY));
        time_of_day *= 60;
        time_of_day += i64::from(self.internal_get(MINUTE));
        time_of_day *= 60;
        time_of_day += i64::from(self.internal_get(SECOND));
        time_of_day *= 1000;
        time_of_day += i64::from(self.internal_get(MILLISECOND));
        if time_of_day >= ONE_DAY {
            fd += 1;
            time_of_day -= ONE_DAY;
        } else if time_of_day < 0 {
            fd -= 1;
            time_of_day += ONE_DAY;
        }
        fd = fd.wrapping_add(delta);
        let mut zone_offset = self
            .internal_get(ZONE_OFFSET)
            .wrapping_add(self.internal_get(DST_OFFSET));
        self.set_time_in_millis(
            (fd - EPOCH_OFFSET)
                .wrapping_mul(ONE_DAY)
                .wrapping_add(time_of_day)
                .wrapping_sub(i64::from(zone_offset)),
        );
        zone_offset = zone_offset.wrapping_sub(
            self.internal_get(ZONE_OFFSET)
                .wrapping_add(self.internal_get(DST_OFFSET)),
        );
        if zone_offset != 0 {
            self.set_time_in_millis(self.time.wrapping_add(i64::from(zone_offset)));
            let fd2 = self.current_fixed_date();
            if fd2 != fd {
                self.set_time_in_millis(self.time.wrapping_sub(i64::from(zone_offset)));
            }
        }
        Ok(())
    }

    /// `getCurrentFixedDate()` / `cachedFixedDate`: the fixed date of the
    /// computed fields.
    fn current_fixed_date(&self) -> i64 {
        match self.kind {
            CalendarKind::Japanese => self.cached_fixed_date,
            _ => {
                if self.calsys_julian == Some(true) {
                    julian_fixed_date(self.cdate.year, self.cdate.month, self.cdate.day_of_month)
                } else {
                    gregorian_fixed_date(self.cdate.year, self.cdate.month, self.cdate.day_of_month)
                }
            }
        }
    }

    /// `GregorianCalendar.pinDayOfMonth()`.
    fn pin_day_of_month(&mut self) -> Result<()> {
        let year = self.internal_get(YEAR);
        let month_len =
            if year > self.gregorian_cutover_year || year < self.gregorian_cutover_year_julian {
                let mut y = year;
                if self.internal_get_era() == BC {
                    y = 1 - y;
                }
                self.month_length_of(self.internal_get(MONTH), y)
            } else {
                let mut gc = self.normalized_calendar()?;
                gc.actual_month_length()
            };
        let dom = self.internal_get(DAY_OF_MONTH);
        if dom > month_len {
            self.set(DAY_OF_MONTH, month_len);
        }
        Ok(())
    }

    /// `getNormalizedCalendar()`.
    fn normalized_calendar(&self) -> Result<Calendar> {
        if self.is_fully_normalized() {
            return Ok(self.clone());
        }
        let mut gc = self.clone();
        gc.set_lenient(true);
        gc.complete()?;
        Ok(gc)
    }

    /// `GregorianCalendar.actualMonthLength()`.
    fn actual_month_length(&mut self) -> i32 {
        let date = self.cdate;
        let year = date.year;
        let julian = self.calsys_julian == Some(true);
        let month_length = |y: i32, m: i32| -> i32 {
            if julian {
                let mut days = DAYS_IN_MONTH[m as usize];
                if m == 2 && is_julian_leap_year(y) {
                    days += 1;
                }
                days
            } else {
                gregorian_month_length(y, m)
            }
        };
        if year != self.gregorian_cutover_year && year != self.gregorian_cutover_year_julian {
            return month_length(year, date.month);
        }
        let fd = if julian {
            julian_fixed_date(date.year, date.month, date.day_of_month)
        } else {
            gregorian_fixed_date(date.year, date.month, date.day_of_month)
        };
        let month1 = self.fixed_date_month1(&date, fd);
        let next1 = month1 + i64::from(month_length(date.year, date.month));
        if next1 < self.gregorian_cutover_date {
            return (next1 - month1) as i32;
        }
        let d = gregorian_date_from_fixed(next1);
        let next1 = self.fixed_date_month1(&d, next1);
        (next1 - month1) as i32
    }

    /// `getWeekYear()`, on its fast path (see the module documentation).
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` where the JDK takes the slow path or
    /// the calendar does not support week dates, and the errors of
    /// [`get`](Self::get).
    pub fn get_week_year(&mut self) -> Result<i32> {
        if !self.is_week_date_supported() {
            return Err(LuceneError::unsupported_operation(String::new()));
        }
        let mut year = self.get(YEAR)?;
        if self.kind == CalendarKind::Buddhist {
            year -= self.year_offset;
        }
        if self.internal_get_era() == BC {
            year = 1 - year;
        }
        if year > self.gregorian_cutover_year + 1 {
            let week_of_year = self.internal_get(WEEK_OF_YEAR);
            if self.internal_get(MONTH) == JANUARY {
                if week_of_year >= 52 {
                    year -= 1;
                }
            } else if week_of_year == 1 {
                year += 1;
            }
            if self.kind == CalendarKind::Buddhist {
                year += self.year_offset;
            }
            return Ok(year);
        }
        Err(LuceneError::unsupported_operation(
            "GregorianCalendar.getWeekYear is not emulated for the years up to the Gregorian cutover"
                .to_string(),
        ))
    }

    // ---------------------------------------------------------- display names

    /// `getDisplayName(int field, int style, Locale locale)` for the fields
    /// and styles `SimpleDateFormat.subFormat` asks for (`ERA`, `MONTH`,
    /// `DAY_OF_WEEK`, `AM_PM` in the short and long format styles, `MONTH`
    /// also in the standalone styles), from the JDK 25 data of the locale's
    /// class; `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// As [`get`](Self::get).
    pub fn get_display_name(
        &mut self,
        field: usize,
        style: i32,
        locale: &Locale,
    ) -> Result<Option<String>> {
        let value = self.get(field)?;
        data().display_name(
            self.calendar_type_index(),
            field as i32,
            style,
            value,
            locale,
        )
    }

    /// The index of this calendar's type in
    /// [`CAL_TYPES`](super::locale_data::CAL_TYPES).
    pub fn calendar_type_index(&self) -> usize {
        match self.kind {
            CalendarKind::Gregorian => 0,
            CalendarKind::Buddhist => 1,
            CalendarKind::Japanese => 2,
        }
    }
}

/// `Calendar.aggregateStamp(int, int)`.
fn aggregate_stamp(stamp_a: i32, stamp_b: i32) -> i32 {
    if stamp_a == UNSET || stamp_b == UNSET {
        return UNSET;
    }
    stamp_a.max(stamp_b)
}

/// `Calendar.setWeekCountData(Locale)`: `CalendarDataUtility.
/// retrieveFirstDayOfWeek` and `retrieveMinimalDaysInFirstWeek` (the
/// `-u-fw-` keyword, then the region of the region override, a locale without
/// a region taking `US`'s data).
///
/// # Errors
///
/// The `IllformedLocaleException` of the region override.
fn week_count_data(locale: &Locale) -> Result<(i32, i32)> {
    let r = super::locale_data::region_override(locale)?;
    let region = if r.get_country().is_empty() {
        "US"
    } else {
        r.get_country()
    };
    let (mut first, minimal) = data().week_data(region);
    if locale.has_extensions() {
        if let Some(fw) = locale.get_unicode_locale_type("fw") {
            let days = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
            if let Some(i) = days.iter().position(|d| *d == fw.to_ascii_lowercase()) {
                first = i as i32 + 1;
            }
        }
    }
    Ok((first, minimal))
}
