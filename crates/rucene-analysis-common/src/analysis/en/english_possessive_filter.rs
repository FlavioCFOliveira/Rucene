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

//! TokenFilter that removes possessives (trailing 's) from words, ported
//! from `org.apache.lucene.analysis.en.EnglishPossessiveFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The body of [`EnglishPossessiveFilter`]: what Lucene's
/// `EnglishPossessiveFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct EnglishPossessiveFilterLogic;

impl TokenFilterLogic for EnglishPossessiveFilterLogic {
    /// Equivalent to `EnglishPossessiveFilter.incrementToken()`.
    ///
    /// Java tests the last two UTF-16 units of the term; the characters it
    /// looks for are of the Basic Multilingual Plane and not surrogates, so
    /// the last two `char`s of the term match exactly when those two units
    /// do, and the test reads the `char`s.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        let mut term_att = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let buffer_length = term_att.length();
        let buffer = term_att.buffer();

        if buffer_length >= 2
            && (buffer[buffer_length - 2] == '\''
                || buffer[buffer_length - 2] == '\u{2019}'
                || buffer[buffer_length - 2] == '\u{FF07}')
            && (buffer[buffer_length - 1] == 's' || buffer[buffer_length - 1] == 'S')
        {
            term_att.set_length(buffer_length - 2); // Strip last 2 characters off
        }

        Ok(true)
    }
}

/// TokenFilter that removes possessives (trailing 's) from words.
///
/// Equivalent to `org.apache.lucene.analysis.en.EnglishPossessiveFilter`.
pub type EnglishPossessiveFilter = TokenFilter<EnglishPossessiveFilterLogic>;

/// Equivalent to `EnglishPossessiveFilter(TokenStream)`, whose field adds
/// the `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_english_possessive_filter(
    input: Box<dyn TokenStream>,
) -> Result<EnglishPossessiveFilter> {
    let mut filter = TokenFilter::new(input, EnglishPossessiveFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
