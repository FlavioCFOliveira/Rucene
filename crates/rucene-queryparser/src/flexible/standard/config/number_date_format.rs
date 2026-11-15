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

//! Port of `NumberDateFormat.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.config`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::class::{JavaClass, NUMBER_FORMAT};
use crate::jdk::date::Date;
use crate::jdk::date_format::DateFormat;
use crate::jdk::format::{FieldPosition, JavaNumber, ParsePosition};
use crate::jdk::number_format::NumberFormat;

/// `org.apache.lucene.queryparser.flexible.standard.config.NumberDateFormat`.
pub static NUMBER_DATE_FORMAT_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.config.NumberDateFormat",
    Some(&NUMBER_FORMAT),
    &[],
);

/// This [`NumberFormat`] parses `long` into date strings and vice-versa. It
/// uses the given `DateFormat` to parse and format dates, but before, it
/// converts `long` to `Date` objects or vice-versa.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.config.NumberDateFormat`,
/// which extends `java.text.NumberFormat` (the trait of E2's emulation). Its
/// `NumberFormat` state (the digit counts, grouping and integer-only parsing
/// of `NumberFormat`'s defaults, which it never uses to format) takes part in
/// `equals` and `hashCode`, which the class inherits from `NumberFormat`.
#[derive(Debug, Clone)]
pub struct NumberDateFormat {
    date_format: Arc<dyn DateFormat>,
    maximum_integer_digits: i32,
    minimum_integer_digits: i32,
    maximum_fraction_digits: i32,
    minimum_fraction_digits: i32,
    grouping_used: bool,
    parse_integer_only: bool,
}

impl NumberDateFormat {
    /// Constructs a [`NumberDateFormat`] object using the given
    /// `dateFormat`.
    ///
    /// Equivalent to `new NumberDateFormat(DateFormat dateFormat)`: the
    /// `NumberFormat` state of `new NumberFormat()` (40, 1, 3 and 0 digits,
    /// grouping, not integer-only).
    pub fn new(date_format: Arc<dyn DateFormat>) -> Self {
        Self {
            date_format,
            maximum_integer_digits: 40,
            minimum_integer_digits: 1,
            maximum_fraction_digits: 3,
            minimum_fraction_digits: 0,
            grouping_used: true,
            parse_integer_only: false,
        }
    }

    /// Equivalent to `format(double number, StringBuffer toAppendTo,
    /// FieldPosition pos)`: the date of `(long) number`.
    ///
    /// # Errors
    ///
    /// The exceptions of the date format.
    pub fn format_double_to(
        &self,
        number: f64,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        self.date_format
            .format_to(Date::new(number as i64), to_append_to, pos)
    }

    /// Equivalent to `format(long number, StringBuffer toAppendTo,
    /// FieldPosition pos)`: the date of `number`.
    ///
    /// # Errors
    ///
    /// The exceptions of the date format.
    pub fn format_long_to(
        &self,
        number: i64,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        self.date_format
            .format_to(Date::new(number), to_append_to, pos)
    }

    /// Equivalent to `NumberFormat.setMaximumIntegerDigits(int newValue)`.
    pub fn set_maximum_integer_digits(&mut self, new_value: i32) {
        self.maximum_integer_digits = new_value.max(0);
        if self.minimum_integer_digits > self.maximum_integer_digits {
            self.minimum_integer_digits = self.maximum_integer_digits;
        }
    }

    /// Equivalent to `NumberFormat.setMinimumIntegerDigits(int newValue)`.
    pub fn set_minimum_integer_digits(&mut self, new_value: i32) {
        self.minimum_integer_digits = new_value.max(0);
        if self.minimum_integer_digits > self.maximum_integer_digits {
            self.maximum_integer_digits = self.minimum_integer_digits;
        }
    }

    /// Equivalent to `NumberFormat.setMaximumFractionDigits(int newValue)`.
    pub fn set_maximum_fraction_digits(&mut self, new_value: i32) {
        self.maximum_fraction_digits = new_value.max(0);
        if self.maximum_fraction_digits < self.minimum_fraction_digits {
            self.minimum_fraction_digits = self.maximum_fraction_digits;
        }
    }

    /// Equivalent to `NumberFormat.setMinimumFractionDigits(int newValue)`.
    pub fn set_minimum_fraction_digits(&mut self, new_value: i32) {
        self.minimum_fraction_digits = new_value.max(0);
        if self.maximum_fraction_digits < self.minimum_fraction_digits {
            self.maximum_fraction_digits = self.minimum_fraction_digits;
        }
    }

    /// Equivalent to `NumberFormat.setGroupingUsed(boolean newValue)`.
    pub fn set_grouping_used(&mut self, new_value: bool) {
        self.grouping_used = new_value;
    }

    /// Equivalent to `NumberFormat.setParseIntegerOnly(boolean value)`.
    pub fn set_parse_integer_only(&mut self, value: bool) {
        self.parse_integer_only = value;
    }
}

impl NumberFormat for NumberDateFormat {
    fn java_class(&self) -> &'static JavaClass {
        &NUMBER_DATE_FORMAT_CLASS
    }

    /// `NumberFormat.equals(Object)`, which the class inherits: the same
    /// class and the same `NumberFormat` state (the date formats are not
    /// compared).
    fn java_equals(&self, other: &dyn NumberFormat) -> bool {
        match other.as_any().downcast_ref::<NumberDateFormat>() {
            Some(other) => {
                std::ptr::eq(self, other)
                    || (self.maximum_integer_digits == other.maximum_integer_digits
                        && self.minimum_integer_digits == other.minimum_integer_digits
                        && self.maximum_fraction_digits == other.maximum_fraction_digits
                        && self.minimum_fraction_digits == other.minimum_fraction_digits
                        && self.grouping_used == other.grouping_used
                        && self.parse_integer_only == other.parse_integer_only)
            }
            None => false,
        }
    }

    /// `NumberFormat.hashCode()`, which the class inherits:
    /// `maximumIntegerDigits * 37 + maxFractionDigits`, the serialization
    /// field `maxFractionDigits` keeping its initial value 3.
    fn java_hash_code(&self) -> i32 {
        self.maximum_integer_digits.wrapping_mul(37).wrapping_add(3)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `format(Object number, StringBuffer toAppendTo,
    /// FieldPosition pos)`: `dateFormat.format(number, ...)`, which formats
    /// a `Number` as the date of its `longValue()`.
    fn format_to(
        &self,
        number: JavaNumber,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        self.date_format
            .format_to(Date::new(number.long_value()), to_append_to, pos)
    }

    /// Equivalent to `parse(String source, ParsePosition parsePosition)`:
    /// the time of the parsed date, as a `Long`.
    fn parse_position(
        &self,
        source: &str,
        parse_position: &mut ParsePosition,
    ) -> Result<Option<JavaNumber>> {
        let date = self.date_format.parse_position(source, parse_position)?;
        Ok(date.map(|date| JavaNumber::Long(date.get_time())))
    }
}
