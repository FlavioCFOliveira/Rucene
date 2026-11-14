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

//! `java.text.DateFormat`: the abstract format the query parsers and the
//! flexible parser's `NumberDateFormat` use, and the factory
//! `getDateInstance`.
//!
//! **Divergence.** An emulation of JDK 25 (`DateFormat` read as
//! specification), not a port of a Lucene file. Java's abstract class is the
//! [`DateFormat`] trait; the factory builds the
//! [`super::simple_date_format::SimpleDateFormat`] of the
//! locale's JDK 25 data.

use rucene::error::Result;

use super::date::Date;
use super::format::{parse_exception, FieldPosition, ParsePosition};
use super::locale::Locale;
use super::simple_date_format::SimpleDateFormat;

pub use super::simple_date_format::{DEFAULT, DF_LONG as LONG, DF_SHORT as SHORT, FULL, MEDIUM};

/// `java.text.DateFormat`: formats and parses `java.util.Date`s.
pub trait DateFormat: std::fmt::Debug + Send + Sync {
    /// `format(Date date, StringBuffer toAppendTo, FieldPosition
    /// fieldPosition)`, appending UTF-16 units.
    ///
    /// # Errors
    ///
    /// The exceptions of the format.
    fn format_to(
        &self,
        date: Date,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()>;

    /// `parse(String source, ParsePosition pos)`: the date, `None` where
    /// Java returns `null`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the format.
    fn parse_position(&self, source: &str, pos: &mut ParsePosition) -> Result<Option<Date>>;

    /// `setLenient(boolean)`.
    fn set_lenient(&self, lenient: bool);

    /// `isLenient()`.
    fn is_lenient(&self) -> bool;

    /// `format(Date)`.
    ///
    /// # Errors
    ///
    /// As [`format_to`](Self::format_to).
    fn format(&self, date: Date) -> Result<String> {
        let mut buffer = Vec::new();
        let mut pos = FieldPosition::dont_care();
        self.format_to(date, &mut buffer, &mut pos)?;
        Ok(String::from_utf16_lossy(&buffer))
    }

    /// `parse(String source)`.
    ///
    /// # Errors
    ///
    /// The `java.text.ParseException` `Unparseable date: "source"`, with the
    /// error index, when nothing is parsed from the start of `source`.
    fn parse(&self, source: &str) -> Result<Date> {
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
}

impl DateFormat for SimpleDateFormat {
    fn format_to(
        &self,
        date: Date,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        SimpleDateFormat::format_to(self, date, to_append_to, pos)
    }

    fn parse_position(&self, source: &str, pos: &mut ParsePosition) -> Result<Option<Date>> {
        SimpleDateFormat::parse_position(self, source, pos)
    }

    fn set_lenient(&self, lenient: bool) {
        SimpleDateFormat::set_lenient(self, lenient);
    }

    fn is_lenient(&self) -> bool {
        SimpleDateFormat::is_lenient(self)
    }
}

/// `DateFormat.getDateInstance(int style, Locale aLocale)`.
///
/// # Errors
///
/// `IllegalArgumentException("Illegal date style " + style)` for a style
/// outside `FULL`..`SHORT`, and the `IllformedLocaleException` of the region
/// override of `locale`.
pub fn get_date_instance(style: i32, locale: &Locale) -> Result<SimpleDateFormat> {
    SimpleDateFormat::get_date_instance(style, locale)
}
