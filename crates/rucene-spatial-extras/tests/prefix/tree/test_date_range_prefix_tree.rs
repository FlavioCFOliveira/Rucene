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

//! Port of `TestDateRangePrefixTree.java` (package
//! `org.apache.lucene.spatial.prefix.tree`).
//!
//! The `java.time` values the test builds to compare with
//! (`ZonedDateTime.of(…).toInstant().toString()`, `OffsetDateTime.of(…)
//! .toInstant()`, `Instant.with(ChronoField, …)`) are reproduced by
//! [`iso_instant_string`] (JDK 25 `DateTimeFormatter.ISO_INSTANT`, with
//! `ZonedDateTime.of`'s field validation) and [`epoch_millis`] (the proleptic
//! ISO calendar); a `Calendar.toInstant()` is its epoch millisecond.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Value};
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;
use rucene_queryparser::jdk::calendar::{self as calendar, Calendar};
use rucene_spatial_extras::prefix::tree::cell::Cell;
use rucene_spatial_extras::prefix::tree::date_range_prefix_tree::{
    DateRangePrefixTree, DEFAULT_CAL, JAVA_UTIL_TIME_COMPAT_CAL,
};
use rucene_spatial_extras::prefix::tree::number_range_prefix_tree::{
    as_unit_nr_shape, NRCell, NumberRangePrefixTreeImpl,
};
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::spatial::spatial_test_case::java_assert;
use crate::spatial_support::{cal, parameters_method, random};

/// `TestDateRangePrefixTree`.
pub fn class_def() -> Arc<Class> {
    type T = TestDateRangePrefixTree;
    let t = |name: &str, body: fn(&mut T) -> Result<()>| {
        randomizedtesting::jdk::reflect::Method::new(name)
            .throws(&["java.lang.Exception"])
            .body(move |t: &mut T| {
                if let Err(e) = body(t) {
                    rucene_test_framework::util::java_exceptions::rethrow_java(e)
                }
            })
    };
    Class::builder("org.apache.lucene.spatial.prefix.tree.TestDateRangePrefixTree")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::with_params(
            &[("java.lang.String", None), ("java.util.Calendar", None)],
            |args: &[Value]| {
                let template_cal = match &args[1] {
                    Value::Object(o, _) => o
                        .downcast_ref::<Calendar>()
                        .expect("a Calendar argument")
                        .clone(),
                    _ => panic!("java.lang.IllegalArgumentException: argument type mismatch"),
                };
                TestDateRangePrefixTree::new(&template_cal)
            },
        ))
        .method(parameters_method("calendar=%s", parameters))
        .method(t("testRoundTrip", T::test_round_trip))
        .method(t("testToStringISO8601", T::test_to_string_iso8601))
        .method(t("testParseCalendar", T::test_parse_calendar))
        .method(t("testShapeRelations", T::test_shape_relations))
        .method(t("testShapeRangeOptimizer", T::test_shape_range_optimizer))
        .method(t(
            "testInvalidDateException",
            T::test_invalid_date_exception,
        ))
        .build()
}

/// `parameters()`.
fn parameters() -> Vec<Vec<Value>> {
    let cal_value = |c: &Calendar| {
        Value::Object(
            Arc::new(c.clone()),
            "java.util.GregorianCalendar".to_string(),
        )
    };
    vec![
        vec![Value::Str("default".to_string()), cal_value(&DEFAULT_CAL)],
        vec![
            Value::Str("compat".to_string()),
            cal_value(&JAVA_UTIL_TIME_COMPAT_CAL),
        ],
    ]
}

/// `CAL_FIELDS` (`private static final`): copies from DateRangePrefixTree.
const CAL_FIELDS: [i32; 7] = [
    calendar::YEAR as i32,
    calendar::MONTH as i32,
    calendar::DAY_OF_MONTH as i32,
    calendar::HOUR_OF_DAY as i32,
    calendar::MINUTE as i32,
    calendar::SECOND as i32,
    calendar::MILLISECOND as i32,
];

/// `Calendar.MAY`.
const MAY: i32 = 4;

/// The `TestDateRangePrefixTree` instance.
pub struct TestDateRangePrefixTree {
    /// `tree` (`private final`).
    tree: Arc<DateRangePrefixTree>,
}

impl TestDateRangePrefixTree {
    /// `TestDateRangePrefixTree(String suiteName, Calendar templateCal)`.
    fn new(template_cal: &Calendar) -> Self {
        Self {
            tree: DateRangePrefixTree::new(template_cal)
                .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e)),
        }
    }

    /// `testRoundTrip()`.
    fn test_round_trip(&mut self) -> Result<()> {
        let tree = &self.tree;
        let mut c = tree.new_cal();
        Assert::assert_equals_str(None, Some("*"), Some(&tree.to_string_calendar(&mut c)?));

        // test no underflow
        Assert::assert_true(tree.to_shape_stack(&[0], 1).to_string().starts_with('-'));

        // Some arbitrary date
        c.set_date(2014, MAY, 9);
        self.round_trip(&c)?;
        Assert::assert_equals_str(
            None,
            Some("2014-05-09"),
            Some(&tree.to_string_calendar(&mut c)?),
        );

        // Earliest date
        c.set_time_in_millis(i64::MIN);
        self.round_trip(&c)?;

        // Farthest date
        c.set_time_in_millis(i64::MAX);
        self.round_trip(&c)?;

        // 1BC is "0000".
        c.clear();
        c.set(calendar::ERA, calendar::BC);
        c.set(calendar::YEAR, 1);
        self.round_trip(&c)?;
        Assert::assert_equals_str(None, Some("0000"), Some(&tree.to_string_calendar(&mut c)?));
        // adding a "+" parses to the same; and a trailing 'Z' is fine too
        cal::assert_equals(None, &c, &tree.parse_calendar("+0000Z")?);

        // 2BC is "-0001"
        c.clear();
        c.set(calendar::ERA, calendar::BC);
        c.set(calendar::YEAR, 2);
        self.round_trip(&c)?;
        Assert::assert_equals_str(None, Some("-0001"), Some(&tree.to_string_calendar(&mut c)?));

        // 1AD is "0001"
        c.clear();
        c.set(calendar::YEAR, 1);
        self.round_trip(&c)?;
        Assert::assert_equals_str(None, Some("0001"), Some(&tree.to_string_calendar(&mut c)?));

        // test random
        c.set_time_in_millis(random().next_long());
        self.round_trip(&c)
    }

    /// `testToStringISO8601()`.
    fn test_to_string_iso8601(&mut self) -> Result<()> {
        let tree = &self.tree;
        let mut c = tree.new_cal();
        c.set_time_in_millis(random().next_long());
        //  create ZonedDateTime from the calendar, then get toInstant.toString which is the ISO8601 we
        // emulate
        //   note: we don't simply init off of millisEpoch because of possible GregorianChangeDate
        // discrepancy.
        let mut year = c.get(calendar::YEAR)?;
        if c.get(calendar::ERA)? == 0 {
            // BC
            year = -year + 1;
        }
        let expected_iso8601 = iso_instant_string(
            i64::from(year),
            c.get(calendar::MONTH)? + 1,
            c.get(calendar::DAY_OF_MONTH)?,
            c.get(calendar::HOUR_OF_DAY)?,
            c.get(calendar::MINUTE)?,
            c.get(calendar::SECOND)?,
            c.get(calendar::MILLISECOND)? * 1_000_000,
        )?;
        let result_to_string = format!("{}Z", tree.to_string_calendar(&mut c)?);
        Assert::assert_equals_str(None, Some(&expected_iso8601), Some(&result_to_string));
        cal::assert_equals(None, &c, &tree.parse_calendar(&expected_iso8601)?);
        Ok(())
    }

    /// `testParseCalendar()`.
    fn test_parse_calendar(&mut self) -> Result<()> {
        let tree = &self.tree;
        // `OffsetDateTime.of(1984, 12, 18, 12, 34, 56, 100000000, ZoneOffset.UTC).toInstant()`
        let expected = epoch_millis(1984, 12, 18, 12, 34, 56, 100);
        // `expected.with(ChronoField.MILLI_OF_SECOND, ms)` / `with(NANO_OF_SECOND, 0)`
        let with_milli = |ms: i64| expected - 100 + ms;
        let instant = |s: &str| -> Result<i64> { Ok(cal::millis_of(&tree.parse_calendar(s)?)) };

        assert_instant(expected, instant("1984-12-18T12:34:56.1Z")?);
        assert_instant(with_milli(10), instant("1984-12-18T12:34:56.01Z")?);
        assert_instant(with_milli(1), instant("1984-12-18T12:34:56.001Z")?);
        assert_instant(expected, instant("1984-12-18T12:34:56.1000Z")?);
        assert_instant(expected, instant("1984-12-18T12:34:56.100000000Z")?);
        assert_instant(with_milli(0), instant("1984-12-18T12:34:56Z")?);
        // decimal places are simply cut off as rounding may affect the "seconds" part of the calender
        // which was set before
        assert_instant(with_milli(999), instant("1984-12-18T12:34:56.9999Z")?);

        assert_instant(expected, instant("1984-12-18T12:34:56.1")?);
        assert_instant(with_milli(10), instant("1984-12-18T12:34:56.01")?);
        assert_instant(with_milli(1), instant("1984-12-18T12:34:56.001")?);
        assert_instant(expected, instant("1984-12-18T12:34:56.1000")?);
        assert_instant(expected, instant("1984-12-18T12:34:56.100000000")?);
        assert_instant(with_milli(0), instant("1984-12-18T12:34:56")?);
        assert_instant(with_milli(999), instant("1984-12-18T12:34:56.9999")?);

        // `OffsetDateTime.parse("1984-12-18T12:34:56.01Z", ISO_DATE_TIME).get(ChronoField.MILLI_OF_SECOND)`
        Assert::assert_equals(iso_milli_of_second("1984-12-18T12:34:56.01Z"), 10);
        Ok(())
    }

    /// `roundTrip(Calendar calOrig)`.
    fn round_trip(&self, cal_orig: &Calendar) -> Result<()> {
        let tree = &self.tree;
        let mut c = cal_orig.clone();
        loop {
            let cal_string;
            {
                let pre_to_string_cal_clone = c.clone();
                cal_string = tree.to_string_calendar(&mut c)?;
                cal::assert_equals(None, &pre_to_string_cal_clone, &c); // ensure toString doesn't modify cal state
            }

            // test parseCalendar
            cal::assert_equals(None, &c, &tree.parse_calendar(&cal_string)?);

            // to Shape and back to Cal
            let shape = tree.to_shape(&mut c)?;
            let mut cal2 = tree.to_calendar(&*shape)?;
            Assert::assert_equals_str(
                None,
                Some(&cal_string),
                Some(&tree.to_string_calendar(&mut cal2)?),
            );

            if cal_string != "*" {
                // not world cell
                // to Term and back to Cell
                let cell = shape
                    .as_any()
                    .downcast_ref::<NRCell>()
                    .expect("java.lang.ClassCastException: the shape is a Cell");
                let term = cell.get_token_bytes_no_leaf(None);
                let cell2 = tree.read_cell(&BytesRef::deep_copy_of(&term), None);
                if !cell.cell_equals(&*cell2) {
                    Assert::fail(&format!("{cal_string} expected:<{cell}> but was:<{cell2}>"));
                }
                let cell2_shape = cell2.get_shape();
                let mut cal3 =
                    tree.to_calendar(&*as_unit_nr_shape(&*cell2_shape).expect("a UnitNRShape"))?;
                Assert::assert_equals_str(
                    None,
                    Some(&cal_string),
                    Some(&tree.to_string_calendar(&mut cal3)?),
                );

                // setLeaf comparison
                cell2.set_leaf();
                let term_leaf = cell2.get_token_bytes_with_leaf(None);
                Assert::assert_true(term < term_leaf);
                Assert::assert_equals(term_leaf.length as i32, term.length as i32 + 1);
                Assert::assert_equals(
                    0u8,
                    term_leaf.bytes[term_leaf.offset + term_leaf.length - 1],
                );
                Assert::assert_true(cell.is_prefix_of(&*cell2));
            }

            // end of loop; decide if should loop again with lower precision
            let cal_prec_field = tree.get_cal_precision_field(&c);
            if cal_prec_field == -1 {
                break;
            }
            let mut field_idx = CAL_FIELDS
                .binary_search(&cal_prec_field)
                .map_or_else(|i| -(i as i32) - 1, |i| i as i32);
            java_assert(field_idx >= 0);
            let prev_prec_field = if field_idx == 0 {
                -1
            } else {
                field_idx -= 1;
                CAL_FIELDS[field_idx as usize]
            };
            if cal::clear_fields_after_catching_underflow(tree, &mut c, prev_prec_field) {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `testShapeRelations()`.
    fn test_shape_relations(&mut self) -> Result<()> {
        let tree = &self.tree;
        // note: left range is 264000 at the thousand year level whereas right value is exact year
        Assert::assert_equals(
            SpatialRelation::Within,
            tree.parse_shape("[-264000 TO -264000-11-20]")?
                .relate(tree.parse_shape("-264000")?.as_shape()),
        );

        let mut shape_a = tree.parse_shape("[3122-01-23 TO 3122-11-27]")?;
        let mut shape_b = tree.parse_shape("[3122-08 TO 3122-11]")?;
        Assert::assert_equals(
            SpatialRelation::Intersects,
            shape_a.relate(shape_b.as_shape()),
        );

        shape_a = tree.parse_shape("3122")?;
        shape_b = tree.parse_shape("[* TO 3122-10-31]")?;
        Assert::assert_equals(
            SpatialRelation::Intersects,
            shape_a.relate(shape_b.as_shape()),
        );

        shape_a = tree.parse_shape("[3122-05-28 TO 3122-06-29]")?;
        shape_b = tree.parse_shape("[3122 TO 3122-04]")?;
        Assert::assert_equals(
            SpatialRelation::Disjoint,
            shape_a.relate(shape_b.as_shape()),
        );
        Ok(())
    }

    /// `testShapeRangeOptimizer()`.
    fn test_shape_range_optimizer(&mut self) -> Result<()> {
        let tree = &self.tree;
        let check = |expected: &str, input: &str| -> Result<()> {
            Assert::assert_equals_str(
                None,
                Some(expected),
                Some(&tree.parse_shape(input)?.to_string()),
            );
            Ok(())
        };
        check("[2014-08 TO 2014-09]", "[2014-08-01 TO 2014-09-30]")?;

        check("2014", "[2014-01-01 TO 2014-12-31]")?;

        check("2014", "[2014-01 TO 2014]")?;
        check("2014-01", "[2014 TO 2014-01]")?;
        check("2014-12", "[2014-12 TO 2014]")?;

        check("[2014 TO 2014-04-06]", "[2014-01 TO 2014-04-06]")?;

        check("*", "[* TO *]")?;

        check("2014-08-01", "[2014-08-01 TO 2014-08-01]")?;

        check("[2014 TO 2014-09-15]", "[2014 TO 2014-09-15]")?;

        check("[* TO 2014-09-15]", "[* TO 2014-09-15]")
    }

    /// `testInvalidDateException()`.
    fn test_invalid_date_exception(&mut self) -> Result<()> {
        let tree = &self.tree;
        {
            let mut jurasic = tree.parse_calendar("-187183959-07-06T11:00:57.156")?;
            Assert::assert_equals(187183960, jurasic.get(calendar::YEAR)?);
            Assert::assert_equals(0, jurasic.get(calendar::ERA)?);
        }
        const PE: &str = "java.text.ParseException";
        expect_throws(PE, || tree.parse_calendar("2000-11T13"));
        expect_throws(PE, || tree.parse_calendar("2000-11-10T13-1"));
        {
            let cause_message = cause_message(&expect_throws(PE, || {
                tree.parse_calendar("2000-11-10T13Z1")
            }));
            Assert::assert_true_msg(
                &format!("{cause_message} has actual delimeter"),
                cause_message.contains('Z'),
            );
            Assert::assert_true_msg(
                &format!("{cause_message} has expected delimeter"),
                cause_message.contains(':'),
            );
            Assert::assert_false_msg(
                &format!("{cause_message} has no input"),
                cause_message.contains("2000-11-10"),
            );
        }
        expect_throws(PE, || tree.parse_calendar("2000T13Z"));
        expect_throws(PE, || tree.parse_calendar("2000-11T13Z"));
        {
            let cause_message =
                cause_message(&expect_throws(PE, || tree.parse_calendar("2000-13-12")));
            Assert::assert_true_msg(
                &format!("{cause_message} has actual value"),
                cause_message.contains("13"),
            );
            Assert::assert_false_msg(
                &format!("{cause_message} has no input"),
                cause_message.contains("2000-13-12"),
            );
        }
        expect_throws(PE, || tree.parse_calendar("2000-13-41T13Z"));
        expect_throws(PE, || tree.parse_calendar("2000-11-12T25Z"));
        expect_throws(PE, || tree.parse_calendar("2000-11-12T25:61Z"));
        expect_throws(PE, || tree.parse_calendar("2000-11-12T25:14:61Z"));
        Ok(())
    }
}

/// `throwable.getCause().getMessage()` (Java's `NullPointerException` for a
/// missing cause; `"null"` for a `null` message, as string concatenation
/// renders it).
fn cause_message(t: &randomizedtesting::jdk::exception::Throwable) -> String {
    let cause = t
        .cause()
        .unwrap_or_else(|| panic!("java.lang.NullPointerException: getCause() is null"));
    cause.message().unwrap_or("null").to_string()
}

/// `assertEquals(Instant expected, Instant actual)` over epoch milliseconds.
fn assert_instant(expected: i64, actual: i64) {
    Assert::assert_equals(expected, actual);
}

/// The proleptic ISO day number of `year-month-day` from 1970-01-01
/// (`LocalDate.toEpochDay()`).
fn epoch_day(year: i64, month: i64, day: i64) -> i64 {
    // JDK LocalDate.toEpochDay
    let y = year;
    let m = month;
    let mut total = 365 * y;
    if y >= 0 {
        total += (y + 3) / 4 - (y + 99) / 100 + (y + 399) / 400;
    } else {
        total -= y / -4 - y / -100 + y / -400;
    }
    total += (367 * m - 362) / 12;
    total += day - 1;
    if m > 2 {
        total -= 1;
        if !is_leap(year) {
            total -= 1;
        }
    }
    total - 719_528 // DAYS_0000_TO_1970
}

/// `IsoChronology.isLeapYear(year)`.
fn is_leap(year: i64) -> bool {
    (year & 3) == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// `OffsetDateTime.of(year, month, day, hour, minute, second, ms * 1_000_000,
/// ZoneOffset.UTC).toInstant().toEpochMilli()`.
fn epoch_millis(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    ms: i64,
) -> i64 {
    ((epoch_day(year, month, day) * 24 + hour) * 60 + minute) * 60 * 1000 + second * 1000 + ms
}

/// `ZonedDateTime.of(year, month, day, hour, minute, second, nano,
/// ZoneOffset.UTC).toInstant().toString()`: Java's `DateTimeException` for
/// a field out of range or an invalid date; else the `ISO_INSTANT` form
/// (`[+|-]yyyy-MM-ddTHH:mm:ss[.SSS…]Z`, a `+` before a year above 9999).
fn iso_instant_string(
    year: i64,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    nano: i32,
) -> Result<String> {
    let invalid = |m: String| LuceneError::unchecked("java.time.DateTimeException", m);
    if !(-999_999_999..=999_999_999).contains(&year) {
        return Err(invalid(format!(
            "Invalid value for Year (valid values -999999999 - 999999999): {year}"
        )));
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(invalid(format!("Invalid date: {year}-{month}-{day}")));
    }
    let month_len = match month {
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day > month_len {
        return Err(invalid(format!("Invalid date '{year}-{month}-{day}'")));
    }
    let year_str = if year > 9999 {
        format!("+{year}")
    } else if year >= 0 {
        format!("{year:04}")
    } else {
        format!("-{:04}", -year)
    };
    let mut out = format!("{year_str}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    if nano > 0 {
        if nano % 1_000_000 == 0 {
            out.push_str(&format!(".{:03}", nano / 1_000_000));
        } else if nano % 1_000 == 0 {
            out.push_str(&format!(".{:06}", nano / 1_000));
        } else {
            out.push_str(&format!(".{nano:09}"));
        }
    }
    out.push('Z');
    Ok(out)
}

/// `OffsetDateTime.parse(text, ISO_DATE_TIME).get(ChronoField.MILLI_OF_SECOND)`
/// for a text whose seconds carry a fraction: its first three digits.
fn iso_milli_of_second(text: &str) -> i32 {
    let fraction = text
        .split_once('.')
        .map(|(_, f)| f.trim_end_matches('Z'))
        .unwrap_or("");
    let digits: String = format!("{fraction:0<9}").chars().take(3).collect();
    digits.parse().expect("ISO fraction digits")
}
