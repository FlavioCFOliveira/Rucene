//! ISO 8601 date-times to and from epoch milliseconds.
//!
//! Lucene stores a date as a `long` (here the epoch milliseconds of the
//! instant, in a `LongField`); converting the text form is the application's
//! job. The calendar arithmetic is Howard Hinnant's `days_from_civil` /
//! `civil_from_days` (proleptic Gregorian calendar, public domain,
//! <https://howardhinnant.github.io/date_algorithms.html>).

use crate::error::{CliError, Result};

const MILLIS_PER_DAY: i64 = 86_400_000;

/// Parses an ISO 8601 calendar date or date-time into epoch milliseconds.
///
/// Accepted forms: `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM`, `YYYY-MM-DDTHH:MM:SS`,
/// each time form optionally followed by a fraction of a second (`.f` up to
/// nine digits; digits past milliseconds are truncated) and by a zone
/// designator `Z` or `±HH:MM`. A value without a zone is UTC. A date alone
/// is midnight UTC of that day.
///
/// # Errors
///
/// Returns [`CliError::Input`] when `text` is not one of these forms or names
/// a day or time that does not exist.
pub fn parse_iso8601(text: &str) -> Result<i64> {
    let invalid = || {
        CliError::Input(format!(
            "invalid ISO 8601 date-time {text:?} (expected YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS[.fff]][Z|±HH:MM])"
        ))
    };
    let bytes = text.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(invalid());
    }
    let year = digits(&bytes[0..4]).ok_or_else(invalid)?;
    let month = digits(&bytes[5..7]).ok_or_else(invalid)?;
    let day = digits(&bytes[8..10]).ok_or_else(invalid)?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return Err(invalid());
    }
    let mut millis = days_from_civil(year, month, day) * MILLIS_PER_DAY;
    let rest = &bytes[10..];
    if rest.is_empty() {
        return Ok(millis);
    }
    if rest[0] != b'T' || rest.len() < 6 || rest[3] != b':' {
        return Err(invalid());
    }
    let hour = digits(&rest[1..3]).ok_or_else(invalid)?;
    let minute = digits(&rest[4..6]).ok_or_else(invalid)?;
    let mut rest = &rest[6..];
    let mut second = 0;
    if rest.first() == Some(&b':') {
        if rest.len() < 3 {
            return Err(invalid());
        }
        second = digits(&rest[1..3]).ok_or_else(invalid)?;
        rest = &rest[3..];
        if rest.first() == Some(&b'.') {
            let count = rest[1..].iter().take_while(|b| b.is_ascii_digit()).count();
            if count == 0 || count > 9 {
                return Err(invalid());
            }
            let fraction = &rest[1..=count];
            let mut fraction_millis = 0i64;
            for position in 0..3 {
                fraction_millis *= 10;
                if let Some(digit) = fraction.get(position) {
                    fraction_millis += i64::from(digit - b'0');
                }
            }
            millis += fraction_millis;
            rest = &rest[1 + count..];
        }
    }
    if hour > 23 || minute > 59 || second > 59 {
        return Err(invalid());
    }
    millis += ((hour * 60 + minute) * 60 + second) * 1000;
    match rest {
        [] | [b'Z'] => Ok(millis),
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let offset_hours = digits(&[*h1, *h2]).ok_or_else(invalid)?;
            let offset_minutes = digits(&[*m1, *m2]).ok_or_else(invalid)?;
            if offset_hours > 23 || offset_minutes > 59 {
                return Err(invalid());
            }
            let offset = (offset_hours * 60 + offset_minutes) * 60_000;
            // Local time = UTC + offset, so UTC = local - offset.
            Ok(if *sign == b'+' {
                millis - offset
            } else {
                millis + offset
            })
        }
        _ => Err(invalid()),
    }
}

/// Formats epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`, the form the
/// USGS catalog uses.
#[must_use]
pub fn format_iso8601(millis: i64) -> String {
    let days = millis.div_euclid(MILLIS_PER_DAY);
    let in_day = millis.rem_euclid(MILLIS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let hour = in_day / 3_600_000;
    let minute = in_day / 60_000 % 60;
    let second = in_day / 1000 % 60;
    let milli = in_day % 1000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z")
}

fn digits(bytes: &[u8]) -> Option<i64> {
    bytes.iter().try_fold(0i64, |acc, byte| {
        byte.is_ascii_digit()
            .then(|| acc * 10 + i64::from(byte - b'0'))
    })
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The proleptic Gregorian date of a count of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants() {
        assert_eq!(parse_iso8601("1970-01-01").unwrap(), 0);
        assert_eq!(
            parse_iso8601("2024-01-01T00:03:15.140Z").unwrap(),
            1_704_067_395_140
        );
        assert_eq!(parse_iso8601("2000-02-29T12:00").unwrap(), 951_825_600_000);
        assert_eq!(parse_iso8601("1969-12-31T23:59:59.999Z").unwrap(), -1);
        assert_eq!(
            parse_iso8601("2024-01-01T01:00:00+01:00").unwrap(),
            parse_iso8601("2024-01-01").unwrap()
        );
        assert_eq!(
            parse_iso8601("2023-12-31T23:00:00-01:00").unwrap(),
            parse_iso8601("2024-01-01").unwrap()
        );
        assert_eq!(
            parse_iso8601("2024-01-01T00:00:00.1234567Z").unwrap(),
            parse_iso8601("2024-01-01T00:00:00.123Z").unwrap()
        );
    }

    #[test]
    fn round_trip_over_four_centuries() {
        let mut millis = parse_iso8601("1800-01-01").unwrap();
        let end = parse_iso8601("2200-01-01").unwrap();
        while millis < end {
            assert_eq!(parse_iso8601(&format_iso8601(millis)).unwrap(), millis);
            millis += 7 * MILLIS_PER_DAY + 3_723_456;
        }
        assert_eq!(format_iso8601(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn invalid_forms_are_rejected() {
        for text in [
            "",
            "2024-1-01",
            "2024-13-01",
            "2023-02-29",
            "2024-01-01T24:00",
            "2024-01-01T10",
            "2024-01-01T10:00:00.",
            "2024-01-01T10:00:00Zjunk",
            "2024-01-01 10:00",
            "2024-01-01T10:00:00+1:00",
        ] {
            assert!(parse_iso8601(text).is_err(), "{text:?} must be rejected");
        }
    }
}
