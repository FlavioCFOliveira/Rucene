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

//! Folds all Unicode digits to Basic Latin digits, ported from
//! `org.apache.lucene.analysis.core.DecimalDigitFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{get_numeric_value, is_digit};

/// The body of [`DecimalDigitFilter`]: what Lucene's `DecimalDigitFilter`
/// adds to `TokenFilter`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DecimalDigitFilterLogic;

impl TokenFilterLogic for DecimalDigitFilterLogic {
    /// Equivalent to `DecimalDigitFilter.incrementToken()`.
    ///
    /// Java walks the term's UTF-16 units, reading the code point at each
    /// (`Character.codePointAt(buffer, i, length)`); a digit outside Basic
    /// Latin is replaced by its Basic Latin digit, and a supplementary one
    /// also loses its low surrogate (`StemmerUtil.delete`), its two units
    /// becoming one. `rucene`'s term holds one `char` per code point (see
    /// `CharTermAttribute`), so each digit is replaced in its own slot and
    /// the term keeps its `char` length: the same term. Java's walk also
    /// visits the low surrogate of a supplementary non-digit, as a code point
    /// of its own, which is never a digit.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let length = term_att.length();
            let buffer = term_att.buffer_mut();

            for slot in &mut buffer[..length] {
                let ch = *slot as i32;
                // look for digits outside of basic latin
                if ch > 0x7F && is_digit(ch) {
                    // replace with equivalent basic latin digit
                    *slot = char::from(b'0' + get_numeric_value(ch) as u8);
                }
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Folds all Unicode digits in `[:General_Category=Decimal_Number:]` to Basic
/// Latin digits (`0-9`).
///
/// Equivalent to `org.apache.lucene.analysis.core.DecimalDigitFilter`.
pub type DecimalDigitFilter = TokenFilter<DecimalDigitFilterLogic>;

/// Creates a new DecimalDigitFilter over `input`.
///
/// Equivalent to `DecimalDigitFilter(TokenStream)`, which adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_decimal_digit_filter(input: Box<dyn TokenStream>) -> Result<DecimalDigitFilter> {
    let mut filter = TokenFilter::new(input, DecimalDigitFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
