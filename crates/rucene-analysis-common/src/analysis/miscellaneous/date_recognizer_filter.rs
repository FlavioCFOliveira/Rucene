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
//! Keeps only the tokens that parse as dates, ported from
//! `org.apache.lucene.analysis.miscellaneous.DateRecognizerFilter`.

use std::sync::Arc;

use rucene::analysis::{
    new_filtering_token_filter, CharTermAttribute, FilteringTokenFilter, FilteringTokenFilterLogic,
    TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::locale::Locale;
use crate::jdk::simple_date_format::SimpleDateFormat;

/// Equivalent to `DateRecognizerFilter.DATE_TYPE`.
pub const DATE_TYPE: &str = "date";

/// The `accept` of [`DateRecognizerFilter`]: what Lucene's
/// `DateRecognizerFilter` adds to `FilteringTokenFilter`.
#[derive(Debug)]
pub struct DateRecognizerFilterLogic {
    /// Java's `DateFormat dateFormat`, a `java.text.DateFormat` the factory
    /// shares with every filter it creates; see `jdk::simple_date_format`.
    date_format: Arc<SimpleDateFormat>,
}

impl FilteringTokenFilterLogic for DateRecognizerFilterLogic {
    /// Equivalent to `DateRecognizerFilter.accept()`: we don't care about the
    /// date, just that the term can be parsed to one
    /// (`dateFormat.parse(termAtt.toString())` throws no `ParseException`).
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        let term: String = {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            term_att.buffer()[..term_att.length()].iter().collect()
        };
        // This term is not a date when it does not parse.
        Ok(self.date_format.parse(&term)?.is_ok())
    }
}

/// Filters all tokens that cannot be parsed to a date, using the provided
/// `DateFormat`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.DateRecognizerFilter`;
/// the `DateFormat` is the `java.text` emulation of `jdk::simple_date_format`.
pub type DateRecognizerFilter = FilteringTokenFilter<DateRecognizerFilterLogic>;

/// Uses `DateFormat.DEFAULT` and `Locale.ENGLISH` to create a `DateFormat`
/// instance.
///
/// Equivalent to `DateRecognizerFilter(TokenStream)`, `this(input, null)`.
///
/// # Errors
///
/// As [`new_date_recognizer_filter_with`].
pub fn new_date_recognizer_filter(input: Box<dyn TokenStream>) -> Result<DateRecognizerFilter> {
    new_date_recognizer_filter_with(input, None)
}

/// Equivalent to `DateRecognizerFilter(TokenStream, DateFormat)`: `None`
/// (Java's `null`) stands for `DateFormat.getDateInstance(DateFormat.DEFAULT,
/// Locale.ENGLISH)`; the filter adds the `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_date_recognizer_filter_with(
    input: Box<dyn TokenStream>,
    date_format: Option<Arc<SimpleDateFormat>>,
) -> Result<DateRecognizerFilter> {
    let date_format = date_format.unwrap_or_else(|| {
        Arc::new(
            SimpleDateFormat::get_date_instance_default(&Locale::english())
                .expect("INVARIANT: Locale.ENGLISH has no region override"),
        )
    });
    let mut filter = new_filtering_token_filter(input, DateRecognizerFilterLogic { date_format })?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
