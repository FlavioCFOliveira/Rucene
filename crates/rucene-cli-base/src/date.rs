//! Calendar dates of the Portal BASE data, to and from epoch milliseconds.
//!
//! The source writes contract and announcement dates as `YYYY-MM-DD` and
//! modification dates as `DD/MM/YYYY 00:00:00`; both name a calendar day with
//! no time zone. A date is indexed as a `LongField` holding the epoch
//! milliseconds of that day's **UTC midnight** (one unit for every index, the
//! unit of the existing example), and stored and printed as `YYYY-MM-DD`.
//!
//! The conversion is the proleptic Gregorian `days_from_civil` /
//! `civil_from_days` pair of Howard Hinnant's public-domain date algorithms.

use crate::error::{CliError, Result};

/// Milliseconds in one day.
pub const MILLIS_PER_DAY: i64 = 86_400_000;

/// Parses a date in either accepted form into epoch milliseconds (UTC
/// midnight of that day):
///
/// * `YYYY-MM-DD`
/// * `DD/MM/YYYY`
///
/// Either may be followed by a space and a time `HH:MM:SS`, which is
/// validated and ignored (the source always writes `00:00:00`). Years run
/// from 0000 to 9999.
///
/// # Errors
///
/// Returns [`CliError::Input`] when `raw` matches neither form or names a day
/// that does not exist (`2023-02-29`).
pub fn parse_date(raw: &str) -> Result<i64> {
    let invalid = || {
        CliError::Input(format!(
            "{raw:?} is not a date (YYYY-MM-DD or DD/MM/YYYY, optionally followed by HH:MM:SS)"
        ))
    };
    let (date, time) = match raw.split_once(' ') {
        Some((date, time)) => (date, Some(time)),
        None => (raw, None),
    };
    if let Some(time) = time {
        let parts: Vec<&str> = time.split(':').collect();
        let valid = parts.len() == 3
            && parts.iter().all(|part| part.len() == 2)
            && number(parts[0]).is_some_and(|hour| hour < 24)
            && number(parts[1]).is_some_and(|minute| minute < 60)
            && number(parts[2]).is_some_and(|second| second < 60);
        if !valid {
            return Err(invalid());
        }
    }
    let (year, month, day) = if let [year, month, day] = split(date, '-')[..] {
        if year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return Err(invalid());
        }
        (year, month, day)
    } else if let [day, month, year] = split(date, '/')[..] {
        if year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return Err(invalid());
        }
        (year, month, day)
    } else {
        return Err(invalid());
    };
    let (year, month, day) = match (number(year), number(month), number(day)) {
        (Some(year), Some(month), Some(day)) => (year, month, day),
        _ => return Err(invalid()),
    };
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return Err(invalid());
    }
    Ok(days_from_civil(year, month, day) * MILLIS_PER_DAY)
}

fn split(text: &str, separator: char) -> Vec<&str> {
    text.split(separator).collect()
}

/// The value of an all-ASCII-digit string, `None` otherwise.
fn number(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
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

/// Days since 1970-01-01 of the proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12; // March = 0
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The proleptic Gregorian date of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Formats epoch milliseconds as the `YYYY-MM-DD` of the UTC day holding
/// them.
#[must_use]
pub fn format_date(millis: i64) -> String {
    let (year, month, day) = civil_from_days(millis.div_euclid(MILLIS_PER_DAY));
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_forms_name_the_same_day() {
        assert_eq!(parse_date("1970-01-01").unwrap(), 0);
        assert_eq!(parse_date("01/01/1970").unwrap(), 0);
        let iso = parse_date("2026-04-15").unwrap();
        assert_eq!(parse_date("15/04/2026 00:00:00").unwrap(), iso);
        assert_eq!(parse_date("15/04/2026").unwrap(), iso);
        assert_eq!(iso, 1_776_211_200_000);
        assert_eq!(format_date(iso), "2026-04-15");
    }

    #[test]
    fn round_trips_every_day_of_four_centuries() {
        let start = parse_date("1900-01-01").unwrap() / MILLIS_PER_DAY;
        let end = parse_date("2300-12-31").unwrap() / MILLIS_PER_DAY;
        for days in start..=end {
            let text = format_date(days * MILLIS_PER_DAY);
            assert_eq!(parse_date(&text).unwrap(), days * MILLIS_PER_DAY, "{text}");
        }
        assert_eq!(format_date(-1), "1969-12-31");
    }

    #[test]
    fn rejects_impossible_and_malformed_dates() {
        for bad in [
            "2023-02-29",
            "2024-13-01",
            "2024-00-10",
            "2024-1-01",
            "24-01-01",
            "2024/01/01",
            "31/04/2024",
            "2024-01-01T00:00:00",
            "2024-01-01 24:00:00",
            "2024-01-01 0:00:00",
            "",
            "abcd-ef-gh",
            "+202-01-01",
        ] {
            assert!(parse_date(bad).is_err(), "{bad:?} must not parse");
        }
        assert!(parse_date("2024-02-29").is_ok());
        assert!(parse_date("29/02/2000 23:59:59").is_ok());
    }
}
