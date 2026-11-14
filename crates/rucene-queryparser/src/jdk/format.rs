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

//! The parts of `java.text.Format` the date and number formats share:
//! `ParsePosition`, `FieldPosition` (with the field delegate a format reports
//! its fields to), `Format.Field`, `java.text.ParseException`, and the
//! `java.lang.Number` values `NumberFormat` formats and parses.
//!
//! **Divergence.** An emulation of the JDK classes (OpenJDK 25 read as
//! specification), not a port of a Lucene file. `java.text.ParseException`
//! is the core's [`ParseException`](rucene::util::extra::ParseException);
//! `Format.Field` is an enum of the `NumberFormat.Field` and
//! `DateFormat.Field` constants the emulated formats report, compared by
//! identity as the JDK compares them.

use rucene::error::LuceneError;

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

    /// `getIndex()`.
    pub fn get_index(&self) -> usize {
        self.index
    }

    /// `setIndex(int)`.
    pub fn set_index(&mut self, index: usize) {
        self.index = index;
    }

    /// `getErrorIndex()`.
    pub fn get_error_index(&self) -> i64 {
        self.error_index
    }

    /// `setErrorIndex(int)`.
    pub fn set_error_index(&mut self, error_index: i64) {
        self.error_index = error_index;
    }
}

/// `java.text.Format.Field`: the `NumberFormat.Field` and `DateFormat.Field`
/// constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormatField {
    /// `NumberFormat.Field.INTEGER`.
    Integer,
    /// `NumberFormat.Field.FRACTION`.
    Fraction,
    /// `NumberFormat.Field.EXPONENT`.
    Exponent,
    /// `NumberFormat.Field.DECIMAL_SEPARATOR`.
    DecimalSeparator,
    /// `NumberFormat.Field.SIGN`.
    Sign,
    /// `NumberFormat.Field.GROUPING_SEPARATOR`.
    GroupingSeparator,
    /// `NumberFormat.Field.EXPONENT_SYMBOL`.
    ExponentSymbol,
    /// `NumberFormat.Field.PERCENT`.
    Percent,
    /// `NumberFormat.Field.PERMILLE`.
    Permille,
    /// `NumberFormat.Field.CURRENCY`.
    Currency,
    /// `NumberFormat.Field.EXPONENT_SIGN`.
    ExponentSign,
    /// A `DateFormat.Field`, by its `PATTERN_INDEX_TO_DATE_FORMAT_FIELD_ID`
    /// field ID (`DateFormat.ERA_FIELD` .. `DateFormat.TIMEZONE_FIELD`).
    Date(i32),
}

/// `java.text.FieldPosition`: a field to look for, by ID or attribute, and
/// the begin and end indices of its first occurrence in a formatted text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldPosition {
    field: i32,
    attribute: Option<FormatField>,
    begin_index: usize,
    end_index: usize,
    /// The `DontCareFieldPosition.INSTANCE` singleton, which lets
    /// `DecimalFormat` take its fast path.
    dont_care: bool,
}

impl FieldPosition {
    /// `new FieldPosition(int field)`.
    pub fn new(field: i32) -> Self {
        Self {
            field,
            attribute: None,
            begin_index: 0,
            end_index: 0,
            dont_care: false,
        }
    }

    /// `new FieldPosition(Format.Field attribute)`, whose field ID is `-1`.
    pub fn with_attribute(attribute: FormatField) -> Self {
        Self::with_attribute_and_field(attribute, -1)
    }

    /// `new FieldPosition(Format.Field attribute, int fieldID)`.
    pub fn with_attribute_and_field(attribute: FormatField, field: i32) -> Self {
        Self {
            field,
            attribute: Some(attribute),
            begin_index: 0,
            end_index: 0,
            dont_care: false,
        }
    }

    /// `DontCareFieldPosition.INSTANCE`: a position that records nothing.
    pub fn dont_care() -> Self {
        Self {
            field: 0,
            attribute: None,
            begin_index: 0,
            end_index: 0,
            dont_care: true,
        }
    }

    /// Whether this is `DontCareFieldPosition.INSTANCE`.
    pub fn is_dont_care(&self) -> bool {
        self.dont_care
    }

    /// `getField()`.
    pub fn get_field(&self) -> i32 {
        self.field
    }

    /// `getFieldAttribute()`.
    pub fn get_field_attribute(&self) -> Option<FormatField> {
        self.attribute
    }

    /// `getBeginIndex()`.
    pub fn get_begin_index(&self) -> usize {
        self.begin_index
    }

    /// `getEndIndex()`.
    pub fn get_end_index(&self) -> usize {
        self.end_index
    }

    /// `setBeginIndex(int)`; ignored by the don't-care position.
    pub fn set_begin_index(&mut self, index: usize) {
        if !self.dont_care {
            self.begin_index = index;
        }
    }

    /// `setEndIndex(int)`; ignored by the don't-care position.
    pub fn set_end_index(&mut self, index: usize) {
        if !self.dont_care {
            self.end_index = index;
        }
    }

    /// `getFieldDelegate()`.
    pub(crate) fn delegate(&mut self) -> FieldDelegate<'_> {
        FieldDelegate {
            position: self,
            encountered_field: false,
        }
    }

    /// `matchesField(Format.Field)`.
    fn matches_attribute(&self, attribute: FormatField) -> bool {
        self.attribute == Some(attribute)
    }

    /// `matchesField(Format.Field, int)`.
    fn matches_field(&self, attribute: FormatField, field: i32) -> bool {
        match self.attribute {
            Some(a) => a == attribute,
            None => field == self.field,
        }
    }
}

/// `FieldPosition.Delegate`: records the first non-empty occurrence of the
/// position's field.
pub(crate) struct FieldDelegate<'a> {
    position: &'a mut FieldPosition,
    encountered_field: bool,
}

impl FieldDelegate<'_> {
    /// `formatted(Format.Field attr, Object value, int start, int end,
    /// StringBuffer buffer)`.
    pub(crate) fn formatted_attr(&mut self, attribute: FormatField, start: usize, end: usize) {
        if !self.encountered_field && self.position.matches_attribute(attribute) {
            self.position.set_begin_index(start);
            self.position.set_end_index(end);
            self.encountered_field = start != end;
        }
    }

    /// `formatted(int fieldID, Format.Field attr, Object value, int start,
    /// int end, StringBuffer buffer)`.
    pub(crate) fn formatted(
        &mut self,
        field: i32,
        attribute: FormatField,
        start: usize,
        end: usize,
    ) {
        if !self.encountered_field && self.position.matches_field(attribute, field) {
            self.position.set_begin_index(start);
            self.position.set_end_index(end);
            self.encountered_field = start != end;
        }
    }
}

/// A `java.lang.Number` a `NumberFormat` formats or parses, by its class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JavaNumber {
    /// `java.lang.Integer`.
    Integer(i32),
    /// `java.lang.Long`.
    Long(i64),
    /// `java.lang.Float`.
    Float(f32),
    /// `java.lang.Double`.
    Double(f64),
}

impl JavaNumber {
    /// `intValue()`.
    pub fn int_value(&self) -> i32 {
        match *self {
            JavaNumber::Integer(v) => v,
            JavaNumber::Long(v) => v as i32,
            JavaNumber::Float(v) => v as i32,
            JavaNumber::Double(v) => v as i32,
        }
    }

    /// `longValue()`.
    pub fn long_value(&self) -> i64 {
        match *self {
            JavaNumber::Integer(v) => i64::from(v),
            JavaNumber::Long(v) => v,
            JavaNumber::Float(v) => v as i64,
            JavaNumber::Double(v) => v as i64,
        }
    }

    /// `floatValue()`.
    pub fn float_value(&self) -> f32 {
        match *self {
            JavaNumber::Integer(v) => v as f32,
            JavaNumber::Long(v) => v as f32,
            JavaNumber::Float(v) => v,
            JavaNumber::Double(v) => v as f32,
        }
    }

    /// `doubleValue()`.
    pub fn double_value(&self) -> f64 {
        match *self {
            JavaNumber::Integer(v) => f64::from(v),
            JavaNumber::Long(v) => v as f64,
            JavaNumber::Float(v) => f64::from(v),
            JavaNumber::Double(v) => v,
        }
    }
}

/// `new java.text.ParseException(message, errorOffset)`.
pub fn parse_exception(message: impl Into<String>, error_offset: i64) -> LuceneError {
    rucene::util::extra::ParseException::error(message, error_offset as i32)
}
