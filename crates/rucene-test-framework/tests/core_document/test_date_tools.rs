//! Port of `lucene/core/src/test/org/apache/lucene/document/TestDateTools.java`.
//!
//! Java builds dates with a GMT `GregorianCalendar` and prints them with a
//! GMT `SimpleDateFormat("yyyy-MM-dd HH:mm:ss:SSS")`; the port computes the
//! same epoch milliseconds ([`gmt`]) and the same rendering
//! ([`iso_format`]) with the proleptic Gregorian calendar, which a
//! `GregorianCalendar` matches for every date these tests use (all after
//! 1582).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Date, DateTools, Resolution};
use rucene::error::Result;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, assert_equals_msg, class, test};

/// `java.text.ParseException`.
const PARSE_EXCEPTION: &str = rucene::util::extra::PARSE_EXCEPTION;

/// `TestDateTools`.
pub fn class_def() -> Arc<Class> {
    randomizedtesting::jdk::exception::register_class(
        PARSE_EXCEPTION,
        randomizedtesting::jdk::exception::EXCEPTION,
    );
    class(
        "org.apache.lucene.document.TestDateTools",
        vec![
            test("testStringToDate", test_string_to_date),
            test("testStringtoTime", test_stringto_time),
            test("testDateAndTimetoString", test_date_and_timeto_string),
            test("testRound", test_round),
            test("testDateToolsUTC", test_date_tools_utc),
        ],
    )
}

/// Days since 1970-01-01 of the proleptic Gregorian `year-month-day`
/// (`month` 1-based).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// `cal.set(year, month, day, hour, minute, second);
/// cal.set(Calendar.MILLISECOND, millis); cal.getTime().getTime()` on a
/// cleared GMT calendar (`month` 0-based, as `Calendar` takes it).
fn gmt(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64, millis: i64) -> i64 {
    let days = days_from_civil(year, month + 1, day);
    ((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis
}

/// `new Date(time)`.
fn date(time: i64) -> Date {
    Date::from_timestamp_millis(time).expect("in range")
}

/// Port of `TestDateTools.isoFormat`: `yyyy-MM-dd HH:mm:ss:SSS` in GMT.
fn iso_format(date: &Date) -> String {
    let millis = date.timestamp_millis();
    let days = millis.div_euclid(86_400_000);
    let ms_of_day = millis.rem_euclid(86_400_000);
    // civil_from_days
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}:{:03}",
        y,
        m,
        d,
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000
    )
}

/// Port of `TestDateTools.testStringToDate`.
fn test_string_to_date() -> Result<()> {
    let d = DateTools::string_to_date("2004")?;
    assert_equals("2004-01-01 00:00:00:000", iso_format(&d).as_str());
    let d = DateTools::string_to_date("20040705")?;
    assert_equals("2004-07-05 00:00:00:000", iso_format(&d).as_str());
    let d = DateTools::string_to_date("200407050910")?;
    assert_equals("2004-07-05 09:10:00:000", iso_format(&d).as_str());
    let d = DateTools::string_to_date("20040705091055990")?;
    assert_equals("2004-07-05 09:10:55:990", iso_format(&d).as_str());

    expect_throws(PARSE_EXCEPTION, || {
        DateTools::string_to_date("97") // no date
    });
    expect_throws(PARSE_EXCEPTION, || {
        DateTools::string_to_date("200401011235009999") // no date
    });
    expect_throws(PARSE_EXCEPTION, || {
        DateTools::string_to_date("aaaa") // no date
    });
    Ok(())
}

/// Port of `TestDateTools.testStringtoTime`.
fn test_stringto_time() -> Result<()> {
    let time = DateTools::string_to_time("197001010000")?;
    // we use default locale since LuceneTestCase randomizes it
    // year=1970, month=january, day=1; hour, minute, second
    let cal = gmt(1970, 0, 1, 0, 0, 0, 0);
    assert_equals(&cal, &time);

    // year=1980, month=february, day=2; hour, minute, second
    let cal = gmt(1980, 1, 2, 11, 5, 0, 0);
    let time = DateTools::string_to_time("198002021105")?;
    assert_equals(&cal, &time);
    Ok(())
}

/// Port of `TestDateTools.testDateAndTimetoString`.
fn test_date_and_timeto_string() -> Result<()> {
    // we use default locale since LuceneTestCase randomizes it
    // year=2004, month=february(!), day=3; hour, minute, second
    let cal = date(gmt(2004, 1, 3, 22, 8, 56, 333));

    let checks = [
        (Resolution::YEAR, "2004", "2004-01-01 00:00:00:000"),
        (Resolution::MONTH, "200402", "2004-02-01 00:00:00:000"),
        (Resolution::DAY, "20040203", "2004-02-03 00:00:00:000"),
        (Resolution::HOUR, "2004020322", "2004-02-03 22:00:00:000"),
        (
            Resolution::MINUTE,
            "200402032208",
            "2004-02-03 22:08:00:000",
        ),
        (
            Resolution::SECOND,
            "20040203220856",
            "2004-02-03 22:08:56:000",
        ),
        (
            Resolution::MILLISECOND,
            "20040203220856333",
            "2004-02-03 22:08:56:333",
        ),
    ];
    for (resolution, expected, iso) in checks {
        let date_string = DateTools::date_to_string(cal, resolution);
        assert_equals(expected, date_string.as_str());
        assert_equals(
            iso,
            iso_format(&DateTools::string_to_date(&date_string)?).as_str(),
        );
    }

    // date before 1970:
    // year=1961, month=march(!), day=5; hour, minute, second
    let cal = date(gmt(1961, 2, 5, 23, 9, 51, 444));
    let date_string = DateTools::date_to_string(cal, Resolution::MILLISECOND);
    assert_equals("19610305230951444", date_string.as_str());
    assert_equals(
        "1961-03-05 23:09:51:444",
        iso_format(&DateTools::string_to_date(&date_string)?).as_str(),
    );

    let date_string = DateTools::date_to_string(cal, Resolution::HOUR);
    assert_equals("1961030523", date_string.as_str());
    assert_equals(
        "1961-03-05 23:00:00:000",
        iso_format(&DateTools::string_to_date(&date_string)?).as_str(),
    );

    // timeToString:
    // year=1970, month=january, day=1; hour, minute, second
    let cal = gmt(1970, 0, 1, 0, 0, 0, 0);
    let date_string = DateTools::time_to_string(cal, Resolution::MILLISECOND);
    assert_equals("19700101000000000", date_string.as_str());

    let cal = gmt(1970, 0, 1, 1, 2, 3, 0);
    let date_string = DateTools::time_to_string(cal, Resolution::MILLISECOND);
    assert_equals("19700101010203000", date_string.as_str());
    Ok(())
}

/// Port of `TestDateTools.testRound`.
fn test_round() -> Result<()> {
    // we use default locale since LuceneTestCase randomizes it
    // year=2004, month=february(!), day=3; hour, minute, second
    let date = date(gmt(2004, 1, 3, 22, 8, 56, 333));
    assert_equals("2004-02-03 22:08:56:333", iso_format(&date).as_str());

    let date_year = DateTools::round_date(date, Resolution::YEAR);
    assert_equals("2004-01-01 00:00:00:000", iso_format(&date_year).as_str());

    let date_month = DateTools::round_date(date, Resolution::MONTH);
    assert_equals("2004-02-01 00:00:00:000", iso_format(&date_month).as_str());

    let date_day = DateTools::round_date(date, Resolution::DAY);
    assert_equals("2004-02-03 00:00:00:000", iso_format(&date_day).as_str());

    let date_hour = DateTools::round_date(date, Resolution::HOUR);
    assert_equals("2004-02-03 22:00:00:000", iso_format(&date_hour).as_str());

    let date_minute = DateTools::round_date(date, Resolution::MINUTE);
    assert_equals("2004-02-03 22:08:00:000", iso_format(&date_minute).as_str());

    let date_second = DateTools::round_date(date, Resolution::SECOND);
    assert_equals("2004-02-03 22:08:56:000", iso_format(&date_second).as_str());

    let date_millisecond = DateTools::round_date(date, Resolution::MILLISECOND);
    assert_equals(
        "2004-02-03 22:08:56:333",
        iso_format(&date_millisecond).as_str(),
    );

    // long parameter:
    let date_year_long = DateTools::round(date.timestamp_millis(), Resolution::YEAR);
    assert_equals(
        "2004-01-01 00:00:00:000",
        iso_format(&self::date(date_year_long)).as_str(),
    );
    let date_millisecond_long = DateTools::round(date.timestamp_millis(), Resolution::MILLISECOND);
    assert_equals(
        "2004-02-03 22:08:56:333",
        iso_format(&self::date(date_millisecond_long)).as_str(),
    );
    Ok(())
}

/// Port of `TestDateTools.testDateToolsUTC`.
///
/// Java sets the JVM default time zone to `Europe/London` around the body
/// (`TimeZone.setDefault`) to prove `DateTools` ignores it. Rust has no
/// process-wide default time zone, and `DateTools` computes in GMT, so the
/// body runs as is.
fn test_date_tools_utc() -> Result<()> {
    // Sun, 30 Oct 2005 00:00:00 +0000 -- the last second of 2005's DST in Europe/London
    let time: i64 = 1130630400;
    let d1 = DateTools::date_to_string(date(time * 1000), Resolution::MINUTE);
    let d2 = DateTools::date_to_string(date((time + 3600) * 1000), Resolution::MINUTE);
    Assert::assert_false_msg("different times", d1 == d2);
    assert_equals_msg(
        Some("midnight"),
        &DateTools::string_to_time(&d1)?,
        &(time * 1000),
    );
    assert_equals_msg(
        Some("later"),
        &DateTools::string_to_time(&d2)?,
        &((time + 3600) * 1000),
    );
    Ok(())
}
