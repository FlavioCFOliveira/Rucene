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

//! The types declared in `DateRangePrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use rucene::util::java_math;
use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock, OnceLock, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::extra::ParseException;
use rucene::util::BytesRef;
use rucene_queryparser::jdk::calendar::{
    Calendar, CalendarKind, DAY_OF_MONTH, ERA, HOUR_OF_DAY, MILLISECOND, MINUTE, MONDAY, MONTH,
    SECOND, YEAR,
};
use rucene_queryparser::jdk::date::Date;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;

use crate::jdk::parse_int;
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::number_range_prefix_tree::{
    compare_prefix, NumberRangePrefixTree, NumberRangePrefixTreeImpl, UnitNRShape,
};
use crate::prefix::tree::spatial_prefix_tree::{SpatialPrefixTree, SpatialPrefixTreeBase};

/// `UTC`.
fn utc() -> TimeZone {
    TimeZone::get_time_zone("UTC")
}

/// `DEFAULT_CAL`: the Java platform default `Calendar` with UTC & ROOT
/// Locale. Generally a `GregorianCalendar`. Do *not* modify this!
pub static DEFAULT_CAL: LazyLock<Calendar> = LazyLock::new(|| {
    let mut cal =
        Calendar::get_instance(&utc(), &Locale::root()).expect("Calendar.getInstance(UTC, ROOT)");
    cal.clear();
    cal
});

/// `JAVA_UTIL_TIME_COMPAT_CAL`: a `Calendar` instance compatible with
/// `java.time.ZonedDateTime` as seen from `GregorianCalendar.from(ZonedDateTime)`.
/// Do *not* modify this!
pub static JAVA_UTIL_TIME_COMPAT_CAL: LazyLock<Calendar> = LazyLock::new(|| {
    // see source of GregorianCalendar.from(ZonedDateTime)
    let mut cal =
        Calendar::new_gregorian(&utc(), &Locale::root()).expect("new GregorianCalendar(UTC, ROOT)");
    cal.set_gregorian_change(Date::new(i64::MIN).get_time());
    cal.set_first_day_of_week(MONDAY); // might not matter?
    cal.set_minimal_days_in_first_week(4); // might not matter
    cal.clear();
    cal
});

/// `FIELD_BY_LEVEL`: the `Calendar` field of each level (-1 for the
/// million, thousand and (unused) world levels).
const FIELD_BY_LEVEL: [i32; 10] = [
    -1, // unused
    -1,
    -1,
    YEAR as i32,
    MONTH as i32,
    DAY_OF_MONTH as i32,
    HOUR_OF_DAY as i32,
    MINUTE as i32,
    SECOND as i32,
    MILLISECOND as i32,
];

/// `YEAR_LEVEL`.
const YEAR_LEVEL: i32 = 3;

/// `NUM_MYEARS`: how many million years are there? We assert how this was
/// computed in the constructor.
const NUM_MYEARS: i32 = 586;

// `Calendar.FEBRUARY`, `APRIL`, `JUNE`, `SEPTEMBER`, `NOVEMBER`.
const FEBRUARY: i32 = 1;
const APRIL: i32 = 3;
const JUNE: i32 = 5;
const SEPTEMBER: i32 = 8;
const NOVEMBER: i32 = 10;

/// The unit shapes the constructor computes (`minLV`, `maxLV`,
/// `gregorianChangeDateLV`), set once the tree is shared since they are
/// cells of it.
struct Lvs {
    min_lv: Arc<dyn UnitNRShape>,
    max_lv: Arc<dyn UnitNRShape>,
    gregorian_change_date_lv: Option<Arc<dyn UnitNRShape>>,
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.DateRangePrefixTree`
/// (`public class DateRangePrefixTree extends NumberRangePrefixTree`): a
/// `NumberRangePrefixTree` for dates. You pass in `Calendar` objects with
/// the desired fields set and the unspecified fields unset, which conveys
/// the precision. The implementation makes some optimization assumptions
/// about a `GregorianCalendar`; others could probably be supported easily.
///
/// Warning: If you construct a `Calendar` and then get something from the
/// object like a field (e.g. year) or `getTimeInMillis()`, then every field
/// is fully set, which may conflict with the precision you intended.
pub struct DateRangePrefixTree {
    nr: NumberRangePrefixTree,
    cal_tmp: Calendar, // template
    mincal: Calendar,
    maxcal: Calendar,
    bc_firstyear: i32,
    bc_lastyear: i32,
    bc_years: i32,
    ad_firstyear: i32,
    ad_lastyear: i32,
    ad_year_base: i32,
    lvs: OnceLock<Lvs>,
    me: Weak<DateRangePrefixTree>,
}

impl fmt::Debug for DateRangePrefixTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DateRangePrefixTree")
            .field("nr", &self.nr)
            .finish_non_exhaustive()
    }
}

impl DateRangePrefixTree {
    /// `calFieldLen(Calendar cal, int field)`.
    fn cal_field_len(cal: &Calendar, field: usize) -> i32 {
        cal.get_maximum(field) - cal.get_minimum(field) + 1
    }

    /// `DateRangePrefixTree(Calendar templateCal)`: constructs with the
    /// specified calendar used as a template to be cloned whenever a new
    /// `Calendar` needs to be created. See [`DEFAULT_CAL`] and
    /// [`JAVA_UTIL_TIME_COMPAT_CAL`].
    ///
    /// # Errors
    ///
    /// The errors of the `Calendar` computations.
    pub fn new(template_cal: &Calendar) -> Result<Arc<DateRangePrefixTree>> {
        let nr = NumberRangePrefixTree::new(vec![
            // sublevels by level
            NUM_MYEARS,
            1000, // 1 thousand thousand-years in a million years
            1000, // 1 thousand years in a thousand-year
            Self::cal_field_len(template_cal, MONTH),
            Self::cal_field_len(template_cal, DAY_OF_MONTH),
            Self::cal_field_len(template_cal, HOUR_OF_DAY),
            Self::cal_field_len(template_cal, MINUTE),
            Self::cal_field_len(template_cal, SECOND),
            Self::cal_field_len(template_cal, MILLISECOND),
        ])?;
        let cal_tmp = template_cal.clone(); // defensive copy
        let mut mincal = cal_tmp.clone();
        mincal.set_time_in_millis(i64::MIN);
        let mut maxcal = cal_tmp.clone();
        maxcal.set_time_in_millis(i64::MAX);
        // BC years are decreasing, remember.  Yet ActualMaximum is the numerically high value,
        // ActualMinimum is 1.
        let bc_firstyear = mincal.get_actual_maximum(YEAR)?;
        let bc_lastyear = mincal.get_actual_minimum(YEAR)?; // 1
        let bc_years = bc_firstyear - bc_lastyear + 1;
        let ad_firstyear = maxcal.get_actual_minimum(YEAR)?; // 1
        let ad_lastyear = maxcal.get_actual_maximum(YEAR)?;
        let ad_year_base = (((bc_years - 1) / 1_000_000) + 1) * 1_000_000; // align year 0 at an even # of million years
        debug_assert!(bc_lastyear == 1 && ad_firstyear == 1);
        debug_assert_eq!(NUM_MYEARS, (ad_year_base + ad_lastyear) / 1_000_000 + 1);
        let tree = Arc::new_cyclic(|me| DateRangePrefixTree {
            nr,
            cal_tmp,
            mincal,
            maxcal,
            bc_firstyear,
            bc_lastyear,
            bc_years,
            ad_firstyear,
            ad_lastyear,
            ad_year_base,
            lvs: OnceLock::new(),
            me: me.clone(),
        });
        let max_lv = tree.to_shape(&mut tree.maxcal.clone())?;
        let min_lv = tree.to_shape(&mut tree.mincal.clone())?;
        // `MAXCAL instanceof GregorianCalendar` (a BuddhistCalendar is one too)
        let gregorian_change_date_lv = if matches!(
            tree.maxcal.kind(),
            CalendarKind::Gregorian | CalendarKind::Buddhist
        ) {
            let mut date = Date::new(tree.maxcal.get_gregorian_change());
            Some(tree.to_unit_shape(&mut date)?)
        } else {
            None
        };
        let _ = tree.lvs.set(Lvs {
            min_lv,
            max_lv,
            gregorian_change_date_lv,
        });
        Ok(tree)
    }

    fn lvs(&self) -> &Lvs {
        self.lvs.get().expect("set by the constructor")
    }

    fn me(&self) -> Arc<DateRangePrefixTree> {
        self.me
            .upgrade()
            .expect("a DateRangePrefixTree is shared as an Arc")
    }

    /// `fastSubCells(UnitNRShape lv)`.
    fn fast_sub_cells(&self, lv: &dyn UnitNRShape) -> i32 {
        if lv.get_unit_level() == YEAR_LEVEL + 1 {
            // month
            match lv.get_val_at_level(lv.get_unit_level()) {
                SEPTEMBER | APRIL | JUNE | NOVEMBER => 30,
                FEBRUARY => {
                    // get the year (negative numbers for BC)
                    let mut year_adj = lv.get_val_at_level(1) * 1_000_000;
                    year_adj += lv.get_val_at_level(2) * 1000;
                    year_adj += lv.get_val_at_level(3);
                    let year = year_adj - self.ad_year_base;
                    if year % 4 == 0 && !(year % 100 == 0 && year % 400 != 0) {
                        // leap year
                        29
                    } else {
                        28
                    }
                }
                _ => 31,
            }
        } else {
            // typical:
            self.nr.max_sub_cells_by_level[lv.get_unit_level() as usize]
        }
    }

    /// `slowSubCells(UnitNRShape lv)`.
    fn slow_sub_cells(&self, lv: &dyn UnitNRShape) -> i32 {
        let field = FIELD_BY_LEVEL[(lv.get_unit_level() + 1) as usize];
        // short-circuit optimization (GregorianCalendar assumptions)
        if field == -1 || field == YEAR as i32 || field >= HOUR_OF_DAY as i32 {
            // TODO make configurable
            return self.nr.max_sub_cells_by_level[lv.get_unit_level() as usize];
        }
        // somewhat heavyweight op; ideally should be stored on UnitNRShape somehow
        let cal = self.to_calendar(lv).unwrap_or_else(|e| panic!("{e}"));
        let field = field as usize;
        cal.get_actual_maximum(field)
            .unwrap_or_else(|e| panic!("{e}"))
            - cal
                .get_actual_minimum(field)
                .unwrap_or_else(|e| panic!("{e}"))
            + 1
    }

    /// `newCal()`: Calendar utility method: returns a clone of the
    /// `Calendar` passed to the constructor with all fields cleared.
    pub fn new_cal(&self) -> Calendar {
        self.cal_tmp.clone()
    }

    /// `getTreeLevelForCalendarField(int calField)`: Calendar utility
    /// method: returns the spatial prefix tree level for the corresponding
    /// `Calendar` field, such as `Calendar.YEAR`. If there's no match, the
    /// next greatest level is returned as a negative value.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Bad calendar field?: ...")`.
    pub fn get_tree_level_for_calendar_field(&self, cal_field: i32) -> Result<i32> {
        for i in YEAR_LEVEL as usize..FIELD_BY_LEVEL.len() {
            if FIELD_BY_LEVEL[i] == cal_field {
                return Ok(i as i32);
            } else if FIELD_BY_LEVEL[i] > cal_field {
                return Ok(-(i as i32));
            }
        }
        Err(LuceneError::illegal_argument(format!(
            "Bad calendar field?: {cal_field}"
        )))
    }

    /// `getCalPrecisionField(Calendar cal)`: Calendar utility method: gets
    /// the `Calendar` field code of the last field that is set prior to an
    /// unset field. It only examines fields relevant to the prefix tree. If
    /// no fields are set, it returns -1.
    pub fn get_cal_precision_field(&self, cal: &Calendar) -> i32 {
        let mut last_field = -1;
        for level in YEAR_LEVEL as usize..FIELD_BY_LEVEL.len() {
            let field = FIELD_BY_LEVEL[level];
            if !cal.is_set(field as usize) {
                break;
            }
            last_field = field;
        }
        last_field
    }

    /// `clearFieldsAfter(Calendar cal, int field)`: Calendar utility
    /// method: calls `Calendar.clear(int)` for every field after `field`.
    /// Beware of Calendar underflow.
    pub fn clear_fields_after(&self, cal: &mut Calendar, field: i32) {
        // a trick to only get this if assert enabled
        #[cfg(debug_assertions)]
        let assert_era = cal.clone().get(ERA).unwrap_or(-1);
        // note: Calendar.ERA == 0;
        for f in (field + 1)..=MILLISECOND as i32 {
            cal.clear_field(f as usize);
        }
        #[cfg(debug_assertions)]
        debug_assert!(
            field + 1 == ERA as i32 || cal.clone().get(ERA).unwrap_or(-1) == assert_era,
            "Calendar underflow"
        );
    }

    /// `toShape(Calendar cal)`: converts the `Calendar` into a
    /// [`UnitNRShape`]. The fields after the precision of `cal` are cleared
    /// again afterwards (the getters set all fields).
    ///
    /// # Errors
    ///
    /// The errors of the `Calendar` computations.
    pub fn to_shape(&self, cal: &mut Calendar) -> Result<Arc<dyn UnitNRShape>> {
        // Convert a Calendar into a stack of cell numbers
        let cal_prec_field = self.get_cal_precision_field(cal); // must call first; getters set all fields
        let result = (|| -> Result<Arc<dyn UnitNRShape>> {
            let mut val_stack = vec![0i32; self.nr.base.max_levels as usize]; // starts at level 1, not 0
            let mut len = 0usize;
            if cal_prec_field >= YEAR as i32 {
                // year or better precision
                let year = cal.get(YEAR)?;
                let mut year_adj = if cal.get(ERA)? == 0 {
                    self.ad_year_base - (year - 1)
                } else {
                    self.ad_year_base + year
                };
                val_stack[len] = year_adj / 1_000_000;
                len += 1;
                year_adj -= val_stack[len - 1] * 1_000_000;
                val_stack[len] = year_adj / 1000;
                len += 1;
                year_adj -= val_stack[len - 1] * 1000;
                val_stack[len] = year_adj;
                len += 1;
                for level in (YEAR_LEVEL + 1) as usize..FIELD_BY_LEVEL.len() {
                    let field = FIELD_BY_LEVEL[level];
                    if field > cal_prec_field {
                        break;
                    }
                    val_stack[len] =
                        cal.get(field as usize)? - cal.get_actual_minimum(field as usize)?;
                    len += 1;
                }
            }
            Ok(self.to_shape_stack(&val_stack, len))
        })();
        self.clear_fields_after(cal, cal_prec_field); // restore precision state modified by get()
        result
    }

    /// `toCalendar(UnitNRShape lv)`: converts the [`UnitNRShape`] shape to
    /// a corresponding `Calendar` that is cleared below its level.
    ///
    /// # Errors
    ///
    /// The errors of the `Calendar` computations.
    pub fn to_calendar(&self, lv: &dyn UnitNRShape) -> Result<Calendar> {
        if lv.get_unit_level() == 0 {
            return Ok(self.new_cal());
        }
        if compare_prefix(lv, &*self.lvs().min_lv) <= 0 {
            // shouldn't typically happen; sometimes in a debugger
            return Ok(self.mincal.clone()); // full precision; truncation would cause underflow
        }
        debug_assert!(compare_prefix(lv, &*self.lvs().max_lv) <= 0);
        let mut cal = self.new_cal();
        let mut year_adj = lv.get_val_at_level(1) * 1_000_000;
        if lv.get_unit_level() > 1 {
            year_adj += lv.get_val_at_level(2) * 1000;
            if lv.get_unit_level() > 2 {
                year_adj += lv.get_val_at_level(3);
            }
        }
        if year_adj > self.ad_year_base {
            cal.set(ERA, 1);
            cal.set(YEAR, year_adj - self.ad_year_base); // setting the year resets the era
        } else {
            cal.set(ERA, 0); // we assert this "sticks" at the end
            cal.set(YEAR, (self.ad_year_base - year_adj) + 1);
        }
        for level in (YEAR_LEVEL + 1)..=lv.get_unit_level() {
            let field = FIELD_BY_LEVEL[level as usize] as usize;
            let value = lv.get_val_at_level(level) + cal.get_actual_minimum(field)?;
            cal.set(field, value);
        }
        debug_assert!(
            year_adj > self.ad_year_base || cal.clone().get(ERA).unwrap_or(-1) == 0,
            "ERA / YEAR underflow"
        );
        Ok(cal)
    }

    /// `toString(Calendar cal)`: calendar utility method consistent with
    /// `java.time.format.DateTimeFormatter.ISO_INSTANT` except has no
    /// trailing 'Z', and will be truncated to the units given according to
    /// `Calendar.isSet(int)`. A fully cleared calendar will yield the string
    /// "*". The isSet() state of the Calendar is re-instated when done.
    ///
    /// # Errors
    ///
    /// The errors of the `Calendar` computations.
    pub fn to_string_calendar(&self, cal: &mut Calendar) -> Result<String> {
        let cal_prec_field = self.get_cal_precision_field(cal); // must call first; getters set all fields
        if cal_prec_field == -1 {
            return Ok("*".to_string());
        }
        let result = (|| -> Result<String> {
            let mut builder = String::with_capacity("yyyy-MM-dd'T'HH:mm:ss.SSS".len()); // typical
            let mut year = cal.get(YEAR)?; // within the era (thus always positve).  >= 1.
            if cal.get(ERA)? == 0 {
                // BC
                year -= 1; // 1BC should be "0000", so shift by one
                if year > 0 {
                    builder.push('-');
                }
            } else if year > 9999 {
                builder.push('+');
            }
            Self::append_padded(&mut builder, year, 4);
            if cal_prec_field >= MONTH as i32 {
                builder.push('-');
                Self::append_padded(&mut builder, cal.get(MONTH)? + 1, 2); // +1 since first is 0
            }
            if cal_prec_field >= DAY_OF_MONTH as i32 {
                builder.push('-');
                Self::append_padded(&mut builder, cal.get(DAY_OF_MONTH)?, 2);
            }
            if cal_prec_field >= HOUR_OF_DAY as i32 {
                builder.push('T');
                Self::append_padded(&mut builder, cal.get(HOUR_OF_DAY)?, 2);
            }
            if cal_prec_field >= MINUTE as i32 {
                builder.push(':');
                Self::append_padded(&mut builder, cal.get(MINUTE)?, 2);
            }
            if cal_prec_field >= SECOND as i32 {
                builder.push(':');
                Self::append_padded(&mut builder, cal.get(SECOND)?, 2);
            }
            if cal_prec_field >= MILLISECOND as i32 && cal.get(MILLISECOND)? > 0 {
                // only if non-zero
                builder.push('.');
                Self::append_padded(&mut builder, cal.get(MILLISECOND)?, 3);
            }
            Ok(builder)
        })();
        self.clear_fields_after(cal, cal_prec_field); // restore precision state modified by get()
        result
    }

    /// `appendPadded(StringBuilder builder, int integer, short positions)`.
    fn append_padded(builder: &mut String, integer: i32, positions: i16) {
        debug_assert!(integer >= 0 && (1..=4).contains(&positions));
        let int_str_len = if integer > 999 {
            4
        } else if integer > 99 {
            3
        } else if integer > 9 {
            2
        } else {
            1
        };
        for _ in 0..(i32::from(positions) - int_str_len) {
            builder.push('0');
        }
        builder.push_str(&integer.to_string());
    }

    /// `parseCalendar(String str)`: calendar utility method: the reverse of
    /// [`to_string_calendar`](Self::to_string_calendar). It will only set
    /// the fields found, leaving the remainder in an un-set state. A leading
    /// '-' or '+' is optional (positive assumed), and a trailing 'Z' is also
    /// optional.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a null or blank string;
    /// `ParseException("Improperly formatted datetime: ...")`, whose cause is
    /// the error found, for a malformed one.
    pub fn parse_calendar(&self, str: &str) -> Result<Calendar> {
        // example: +2014-10-23T21:22:33.159Z
        if str.is_empty() {
            return Err(LuceneError::illegal_argument("str is null or blank"));
        }
        let mut cal = self.new_cal();
        if str == "*" {
            return Ok(cal);
        }
        let units: Vec<u16> = str.encode_utf16().collect();
        let mut offset: i32 = 0; // a pointer
        let result = Self::parse_calendar_fields(&units, &mut cal, &mut offset);
        match result {
            Ok(()) => Ok(cal),
            Err(e) => Err(ParseException::error_with_cause(
                format!("Improperly formatted datetime: {str}"),
                offset,
                e,
            )),
        }
    }

    /// The `try` body of `parseCalendar`; `offset` is Java's pointer, read
    /// by the `catch`.
    fn parse_calendar_fields(units: &[u16], cal: &mut Calendar, offset: &mut i32) -> Result<()> {
        let len = units.len() as i32;
        // year & era:
        let last_offset = if units[units.len() - 1] == u16::from(b'Z') {
            len - 1
        } else {
            len
        };
        let mut hyphen_idx = index_of(units, u16::from(b'-'), 1); // look past possible leading hyphen
        if hyphen_idx < 0 {
            hyphen_idx = last_offset;
        }
        let year = parse_int(&substring(units, *offset, hyphen_idx)?)?;
        cal.set(ERA, if year <= 0 { 0 } else { 1 });
        cal.set(YEAR, if year <= 0 { -year + 1 } else { year });
        *offset = hyphen_idx + 1;
        if last_offset < *offset {
            return Ok(());
        }

        // NOTE: We aren't validating separator chars, and we unintentionally accept leading +/-.
        // The str.substring()'s hopefully get optimized to be stack-allocated.

        // month:
        let parsed_val = parse_and_check(units, *offset, 1, 12)?;
        cal.set(MONTH, parsed_val - 1); // starts at 0
        *offset += 3;
        if last_offset < *offset {
            return Ok(());
        }
        // day:
        check_delimeter(units, *offset - 1, '-')?;
        let parsed_val = parse_and_check(units, *offset, 1, 31)?;
        cal.set(DAY_OF_MONTH, parsed_val);
        *offset += 3;
        if last_offset < *offset {
            return Ok(());
        }
        check_delimeter(units, *offset - 1, 'T')?;
        // hour:
        let parsed_val = parse_and_check(units, *offset, 0, 24)?;
        cal.set(HOUR_OF_DAY, parsed_val);
        *offset += 3;
        if last_offset < *offset {
            return Ok(());
        }
        check_delimeter(units, *offset - 1, ':')?;
        // minute:
        let parsed_val = parse_and_check(units, *offset, 0, 59)?;
        cal.set(MINUTE, parsed_val);
        *offset += 3;
        if last_offset < *offset {
            return Ok(());
        }
        check_delimeter(units, *offset - 1, ':')?;
        // second:
        let parsed_val = parse_and_check(units, *offset, 0, 59)?;
        cal.set(SECOND, parsed_val);
        *offset += 3;
        if last_offset < *offset {
            return Ok(());
        }
        check_delimeter(units, *offset - 1, '.')?;
        // ms:
        let max_offset = last_offset - *offset; // assume remaining is all digits to compute milliseconds
                                                // we truncate off > millisecond precision (3 digits only)
        let digits = parse_int(&substring(units, *offset, *offset + max_offset)?)?;
        let millis = (f64::from(digits) / java_math::pow(10.0, f64::from(max_offset - 3))) as i32;
        cal.set(MILLISECOND, millis);
        Ok(())
    }
}

/// `String.indexOf(int ch, int fromIndex)` over UTF-16 units.
fn index_of(units: &[u16], ch: u16, from: usize) -> i32 {
    units
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, &u)| u == ch)
        .map(|(i, _)| i as i32)
        .unwrap_or(-1)
}

/// `String.substring(int beginIndex, int endIndex)` over UTF-16 units, with
/// Java's `StringIndexOutOfBoundsException`.
fn substring(units: &[u16], begin: i32, end: i32) -> Result<String> {
    let len = units.len() as i32;
    if begin < 0 || end > len || begin > end {
        return Err(LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin {begin}, end {end}, length {len}"),
        ));
    }
    Ok(String::from_utf16_lossy(
        &units[begin as usize..end as usize],
    ))
}

/// `String.charAt(int index)` over UTF-16 units, with Java's
/// `StringIndexOutOfBoundsException`.
fn char_at(units: &[u16], index: i32) -> Result<u16> {
    let len = units.len() as i32;
    if index < 0 || index >= len {
        return Err(LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("Index {index} out of bounds for length {len}"),
        ));
    }
    Ok(units[index as usize])
}

/// `checkDelimeter(String str, int offset, char delim)`.
fn check_delimeter(units: &[u16], offset: i32, delim: char) -> Result<()> {
    let c = char_at(units, offset)?;
    if c != delim as u16 {
        return Err(LuceneError::illegal_argument(format!(
            "Invalid delimeter: '{}', expecting '{delim}'",
            String::from_utf16_lossy(&[c])
        )));
    }
    Ok(())
}

/// `parseAndCheck(String str, int offset, int min, int max)`.
fn parse_and_check(units: &[u16], offset: i32, min: i32, max: i32) -> Result<i32> {
    let val = parse_int(&substring(units, offset, offset + 2)?)?;
    if val < min || val > max {
        return Err(LuceneError::illegal_argument(format!(
            "Invalid value: {val}, expecting from {min} to {max}]"
        )));
    }
    Ok(val)
}

impl fmt::Display for DateRangePrefixTree {
    /// `NumberRangePrefixTree.toString()`: `getClass().getSimpleName()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DateRangePrefixTree")
    }
}

impl SpatialPrefixTree for DateRangePrefixTree {
    fn base(&self) -> &SpatialPrefixTreeBase {
        &self.nr.base
    }

    /// `NumberRangePrefixTree.getLevelForDistance(double dist)`: always
    /// full precision; we don't do approximations in this tree/strategy.
    fn get_level_for_distance(&self, _dist: f64) -> i32 {
        // note: it might be useful to compute which level has a raw width (counted in
        // bottom units, e.g. milliseconds), that covers the provided dist in those units?
        self.nr.base.max_levels // thus always use full precision. We don't do approximations in this tree/strategy.
                                // throw new UnsupportedOperationException("Not applicable.");
    }

    /// `NumberRangePrefixTree.getDistanceForLevel(int level)`.
    fn get_distance_for_level(&self, _level: i32) -> Result<f64> {
        // note: we could compute this... should we?
        Err(LuceneError::unsupported_operation("Not applicable."))
    }

    fn get_world_cell(&self) -> CellRef {
        self.nr_get_world_cell()
    }

    fn read_cell(&self, term: &BytesRef, scratch: Option<CellRef>) -> CellRef {
        self.nr_read_cell(term, scratch)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_number_range_prefix_tree(&self) -> Option<&dyn NumberRangePrefixTreeImpl> {
        Some(self)
    }
}

impl NumberRangePrefixTreeImpl for DateRangePrefixTree {
    fn nr(&self) -> &NumberRangePrefixTree {
        &self.nr
    }

    fn me_nr(&self) -> Arc<dyn NumberRangePrefixTreeImpl> {
        self.me()
    }

    fn get_num_sub_cells(&self, lv: &dyn UnitNRShape) -> i32 {
        let lvs = self.lvs.get();
        let Some(lvs) = lvs else {
            // only reachable while the constructor computes its shapes,
            // which never asks for sub-cell counts
            return self.nr.max_sub_cells_by_level[lv.get_unit_level() as usize];
        };
        let cmp = compare_prefix(lv, &*lvs.max_lv);
        debug_assert!(cmp <= 0);
        if cmp == 0 {
            // edge case (literally!)
            return lvs.max_lv.get_val_at_level(lv.get_unit_level() + 1) + 1;
        }
        // if using GregorianCalendar and we're after the "Gregorian change date" then we'll compute
        //  the sub-cells ourselves more efficiently without the need to construct a Calendar.
        let cmp = match &lvs.gregorian_change_date_lv {
            Some(g) => compare_prefix(lv, &**g),
            None => -1,
        };
        // TODO consider also doing fast-path if field is <= hours even if before greg change date
        if cmp >= 0 {
            let result = self.fast_sub_cells(lv);
            debug_assert_eq!(
                result,
                self.slow_sub_cells(lv),
                "fast/slow numSubCells inconsistency"
            );
            result
        } else {
            self.slow_sub_cells(lv)
        }
    }

    fn to_unit_shape(&self, value: &mut dyn Any) -> Result<Arc<dyn UnitNRShape>> {
        if let Some(cal) = value.downcast_mut::<Calendar>() {
            return self.to_shape(cal);
        }
        if let Some(date) = value.downcast_ref::<Date>() {
            let mut cal = self.new_cal();
            cal.set_time(date.clone());
            return self.to_shape(&mut cal);
        }
        Err(LuceneError::illegal_argument(format!(
            "Expecting Calendar or Date but got: {:?}",
            (*value).type_id()
        )))
    }

    fn to_object(&self, shape: &dyn UnitNRShape) -> Result<Box<dyn Any>> {
        Ok(Box::new(self.to_calendar(shape)?))
    }

    fn to_string_unit(&self, lv: &dyn UnitNRShape) -> String {
        let mut cal = self.to_calendar(lv).unwrap_or_else(|e| panic!("{e}"));
        self.to_string_calendar(&mut cal)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn parse_unit_shape(&self, str: &str) -> Result<Arc<dyn UnitNRShape>> {
        let mut cal = self.parse_calendar(str)?;
        self.to_shape(&mut cal)
    }

    fn nr_to_string(&self) -> String {
        "DateRangePrefixTree".to_string()
    }
}
