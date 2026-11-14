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

//! `java.text.NumberFormat`: the abstract format the flexible query parser's
//! `PointsConfig` holds, and the factories `getInstance`,
//! `getNumberInstance` and `getIntegerInstance`.
//!
//! **Divergence.** An emulation of JDK 25 (`NumberFormat` and
//! `NumberFormatProviderImpl` read as specification), not a port of a Lucene
//! file. Java's abstract class is the [`NumberFormat`] trait; the factories
//! build the [`DecimalFormat`] of the locale's JDK 25 data (see
//! [`locale_data`](super::locale_data)).

use rucene::error::Result;

use super::decimal_format::{DecimalFormat, DecimalFormatSymbols};
use super::format::{parse_exception, FieldPosition, JavaNumber, ParsePosition};
use super::locale::Locale;
use super::locale_data::data;

/// `java.text.NumberFormat`: formats and parses `java.lang.Number`s.
pub trait NumberFormat: std::fmt::Debug + Send + Sync {
    /// `format(Object number, StringBuffer toAppendTo, FieldPosition pos)`,
    /// appending UTF-16 units.
    ///
    /// # Errors
    ///
    /// The exceptions of the format.
    fn format_to(
        &self,
        number: JavaNumber,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()>;

    /// `parse(String source, ParsePosition parsePosition)`: the number,
    /// `None` where Java returns `null`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the format.
    fn parse_position(&self, source: &str, pos: &mut ParsePosition) -> Result<Option<JavaNumber>>;

    /// `Format.format(Object)`: `format(number, new StringBuffer(), new
    /// FieldPosition(0))`.
    ///
    /// # Errors
    ///
    /// As [`format_to`](Self::format_to).
    fn format(&self, number: JavaNumber) -> Result<String> {
        let mut buffer = Vec::new();
        let mut pos = FieldPosition::new(0);
        self.format_to(number, &mut buffer, &mut pos)?;
        Ok(String::from_utf16_lossy(&buffer))
    }

    /// `parse(String source)`.
    ///
    /// # Errors
    ///
    /// The `java.text.ParseException` `Unparseable number: "source"`, with
    /// the error index, when nothing is parsed from the start of `source`.
    fn parse(&self, source: &str) -> Result<JavaNumber> {
        let mut pos = ParsePosition::new(0);
        let result = self.parse_position(source, &mut pos)?;
        if pos.index == 0 {
            return Err(parse_exception(
                format!("Unparseable number: \"{source}\""),
                pos.error_index,
            ));
        }
        Ok(result.expect("INVARIANT: a parse that advanced returns a number"))
    }
}

impl NumberFormat for DecimalFormat {
    fn format_to(
        &self,
        number: JavaNumber,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        self.format_number_to(number, to_append_to, pos)
    }

    fn parse_position(&self, source: &str, pos: &mut ParsePosition) -> Result<Option<JavaNumber>> {
        Ok(DecimalFormat::parse_position(self, source, pos))
    }
}

/// `NumberFormat.getNumberInstance(Locale)`: the `DecimalFormat` of the
/// locale's number pattern and symbols (those of the region override unless
/// `-u-nu-` is given).
///
/// # Errors
///
/// The `IllformedLocaleException` of the region override.
pub fn get_number_instance(locale: &Locale) -> Result<DecimalFormat> {
    let record = data().number_record(locale)?;
    DecimalFormat::new(&record.pattern, DecimalFormatSymbols::of(record))
}

/// `NumberFormat.getInstance(Locale)`: `getNumberInstance(locale)`.
///
/// # Errors
///
/// As [`get_number_instance`].
pub fn get_instance(locale: &Locale) -> Result<DecimalFormat> {
    get_number_instance(locale)
}

/// `NumberFormat.getIntegerInstance(Locale)`: the number instance with no
/// fraction digits, the decimal separator hidden, and integer-only parsing.
///
/// # Errors
///
/// As [`get_number_instance`].
pub fn get_integer_instance(locale: &Locale) -> Result<DecimalFormat> {
    let mut format = get_number_instance(locale)?;
    format.set_maximum_fraction_digits(0);
    format.set_decimal_separator_always_shown(false);
    format.set_parse_integer_only(true);
    Ok(format)
}
