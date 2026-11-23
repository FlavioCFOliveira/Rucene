// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Grego`.

use crate::r#impl::pair::Pair;

/// A utility class providing proleptic Gregorian calendar functions used by
/// time zone and calendar code.
///
/// Equivalent to `com.ibm.icu.impl.Grego` (static members only).
pub struct Grego;

const JULIAN_1_CE: i64 = 1721426;
const JULIAN_1970_CE: i64 = 2440588;

const MONTH_LENGTH: [i32; 24] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

const DAYS_BEFORE: [i32; 24] =
    [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];

impl Grego {
    /// `MIN_MILLIS`.
    pub const MIN_MILLIS: i64 = -184303902528000000;
    /// `MAX_MILLIS`.
    pub const MAX_MILLIS: i64 = 183882168921600000;
    /// `MILLIS_PER_SECOND`.
    pub const MILLIS_PER_SECOND: i32 = 1000;
    /// `MILLIS_PER_MINUTE`.
    pub const MILLIS_PER_MINUTE: i32 = 60 * Self::MILLIS_PER_SECOND;
    /// `MILLIS_PER_HOUR`.
    pub const MILLIS_PER_HOUR: i32 = 60 * Self::MILLIS_PER_MINUTE;
    /// `MILLIS_PER_DAY`.
    pub const MILLIS_PER_DAY: i32 = 24 * Self::MILLIS_PER_HOUR;

    /// `isLeapYear(int year)`.
    pub fn is_leap_year(year: i32) -> bool {
        ((year & 0x3) == 0) && ((year % 100 != 0) || (year % 400 == 0))
    }

    /// `monthLength(int year, int month)` (`month` 0-based).
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for a month outside `0..12`.
    pub fn month_length(year: i32, month: i32) -> i32 {
        MONTH_LENGTH[(month + if Self::is_leap_year(year) { 12 } else { 0 }) as usize]
    }

    /// `previousMonthLength(int year, int month)`.
    pub fn previous_month_length(year: i32, month: i32) -> i32 {
        if month > 0 {
            Self::month_length(year, month - 1)
        } else {
            31
        }
    }

    /// `fieldsToDay(int year, int month, int dom)`: days since 1970-01-01.
    pub fn fields_to_day(year: i32, month: i32, dom: i32) -> i64 {
        let y = (year - 1) as i64;
        let julian = 365 * y
            + Self::floor_divide(y, 4)
            + (JULIAN_1_CE - 3)
            + Self::floor_divide(y, 400)
            - Self::floor_divide(y, 100)
            + 2
            + DAYS_BEFORE[(month + if Self::is_leap_year(year) { 12 } else { 0 }) as usize] as i64
            + dom as i64;
        julian - JULIAN_1970_CE
    }

    /// `dayOfWeek(long day)`: 1 (Sunday) to 7 (Saturday).
    pub fn day_of_week(day: i64) -> i32 {
        let result = Self::floor_divide_and_remainer(day + 5, 7);
        let day_of_week = result.second;
        if day_of_week == 0 {
            7
        } else {
            day_of_week
        }
    }

    /// `dayToYear(long day)`: the year and the 1-based day of the year.
    pub fn day_to_year(mut day: i64) -> Pair<i32, i32> {
        day += JULIAN_1970_CE - JULIAN_1_CE;
        let n400 = Self::floor_divide_and_remainer(day, 146097);
        let n100 = Self::floor_divide_and_remainer(n400.second as i64, 36524);
        let n4 = Self::floor_divide_and_remainer(n100.second as i64, 1461);
        let n1 = Self::floor_divide_and_remainer(n4.second as i64, 365);
        let mut year = (400 * n400.first + 100 * n100.first + 4 * n4.first + n1.first) as i32;
        let mut day_of_year = n1.second;
        if n100.first == 4 || n1.first == 4 {
            day_of_year = 365;
        } else {
            year += 1;
        }
        day_of_year += 1;
        Pair::new(year, day_of_year)
    }

    /// `dayToFields(long day, int[] fields)`: year, month (0-based), day of
    /// month, day of week, day of year; a `fields` shorter than 5 (or
    /// `None`) is replaced by a new array.
    pub fn day_to_fields(mut day: i64, fields: Option<Vec<i32>>) -> Vec<i32> {
        let mut fields = match fields {
            Some(f) if f.len() >= 5 => f,
            _ => vec![0; 5],
        };
        let result = Self::day_to_year(day);
        let year = result.first;
        let day_of_year = result.second;
        day += JULIAN_1970_CE - JULIAN_1_CE;
        let is_leap = Self::is_leap_year(year);
        let mut correction = 0;
        let march1 = if is_leap { 60 } else { 59 };
        if day_of_year > march1 {
            correction = if is_leap { 1 } else { 2 };
        }
        let month = (12 * (day_of_year - 1 + correction) + 6) / 367;
        let day_of_month = day_of_year - DAYS_BEFORE[(if is_leap { month + 12 } else { month }) as usize];
        let mut day_of_week = ((day + 2) % 7) as i32;
        if day_of_week < 1 {
            day_of_week += 7;
        }
        fields[0] = year;
        fields[1] = month;
        fields[2] = day_of_month;
        fields[3] = day_of_week;
        fields[4] = day_of_year;
        fields
    }

    /// `timeToFields(long time, int[] fields)`: the fields of
    /// [`day_to_fields`](Self::day_to_fields) plus the millisecond of the
    /// day; a `fields` shorter than 6 (or `None`) is replaced.
    pub fn time_to_fields(time: i64, fields: Option<Vec<i32>>) -> Vec<i32> {
        let fields = match fields {
            Some(f) if f.len() >= 6 => f,
            _ => vec![0; 6],
        };
        let result = Self::floor_divide_and_remainer(time, 24 * 60 * 60 * 1000);
        let mut fields = Self::day_to_fields(result.first, Some(fields));
        fields[5] = result.second;
        fields
    }

    /// `timeToYear(long time)`.
    pub fn time_to_year(time: i64) -> i32 {
        Self::day_to_year(Self::floor_divide_and_remainer(time, 24 * 60 * 60 * 1000).first).first
    }

    /// `floorDivide(long numerator, long denominator)`.
    pub fn floor_divide(numerator: i64, denominator: i64) -> i64 {
        if numerator >= 0 {
            numerator / denominator
        } else {
            ((numerator + 1) / denominator) - 1
        }
    }

    /// `floorDivideAndRemainer(long numerator, int denominator)`.
    fn floor_divide_and_remainer(numerator: i64, denominator: i32) -> Pair<i64, i32> {
        if numerator >= 0 {
            return Pair::new(Self::floor_divide(numerator, denominator as i64), (numerator % denominator as i64) as i32);
        }
        let quotient = Self::floor_divide(numerator, denominator as i64);
        Pair::new(quotient, (numerator - (quotient * denominator as i64)) as i32)
    }

    /// `getDayOfWeekInMonth(int year, int month, int dayOfMonth)`.
    pub fn get_day_of_week_in_month(year: i32, month: i32, day_of_month: i32) -> i32 {
        let mut week_in_month = (day_of_month + 6) / 7;
        if week_in_month == 4 {
            if day_of_month + 7 > Self::month_length(year, month) {
                week_in_month = -1;
            }
        } else if week_in_month == 5 {
            week_in_month = -1;
        }
        week_in_month
    }

    /// `timeToString(long time)`: `yyyy-MM-ddTHH:mm:ss.SSSZ`, the
    /// `String.format("%04d-%02d-%02dT%02d:%02d:%02d.%03dZ", ...)` of the
    /// fields.
    pub fn time_to_string(time: i64) -> String {
        let fields = Self::time_to_fields(time, None);
        let mut millis = fields[5];
        let hour = millis / Self::MILLIS_PER_HOUR;
        millis %= Self::MILLIS_PER_HOUR;
        let min = millis / Self::MILLIS_PER_MINUTE;
        millis %= Self::MILLIS_PER_MINUTE;
        let sec = millis / Self::MILLIS_PER_SECOND;
        millis %= Self::MILLIS_PER_SECOND;
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            fields[0],
            fields[1] + 1,
            fields[2],
            hour,
            min,
            sec,
            millis
        )
    }
}
