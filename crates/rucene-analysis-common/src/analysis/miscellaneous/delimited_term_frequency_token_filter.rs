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
//! Reads a term frequency after a delimiter of the term, ported from
//! `org.apache.lucene.analysis.miscellaneous.DelimitedTermFrequencyTokenFilter`.

use rucene::analysis::{
    CharTermAttribute, TermFrequencyAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::ArrayUtil;

/// The default delimiter, `'|'`.
///
/// Equivalent to `DelimitedTermFrequencyTokenFilter.DEFAULT_DELIMITER`.
pub const DEFAULT_DELIMITER: char = '|';

/// The body of [`DelimitedTermFrequencyTokenFilter`]: what Lucene's
/// `DelimitedTermFrequencyTokenFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct DelimitedTermFrequencyTokenFilterLogic {
    /// Java's `char delimiter`: a BMP character (`getChar` refuses any
    /// other).
    delimiter: char,
}

impl TokenFilterLogic for DelimitedTermFrequencyTokenFilterLogic {
    /// Equivalent to `DelimitedTermFrequencyTokenFilter.incrementToken()`.
    ///
    /// Java compares each UTF-16 unit of the term with the delimiter, a BMP
    /// character, so comparing the term's `char`s finds the same position;
    /// the positions count `char`s here.
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
            for i in 0..length {
                if term_att.buffer()[i] == self.delimiter {
                    term_att.set_length(i); // simply set a new length
                    let i = i + 1;
                    let frequency = ArrayUtil::parse_int(term_att.buffer(), i, length - i)?;
                    drop(term_att);
                    source
                        .get_attribute_interface_mut::<dyn TermFrequencyAttribute>()
                        .expect("INVARIANT: the constructor added the TermFrequencyAttribute")
                        .set_term_frequency(frequency);
                    return Ok(true);
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// Characters before the delimiter are the "token", the textual integer
/// after is the term frequency. To use this `TokenFilter` the field must be
/// indexed with `IndexOptions#DOCS_AND_FREQS` but no positions or offsets.
///
/// For example, if the delimiter is '|', then for the string "foo|5", "foo"
/// is the token and "5" is a term frequency. If there is no delimiter, the
/// TokenFilter does not modify the term frequency.
///
/// Note make sure your Tokenizer doesn't split on the delimiter, or this
/// won't work.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.DelimitedTermFrequencyTokenFilter`.
pub type DelimitedTermFrequencyTokenFilter = TokenFilter<DelimitedTermFrequencyTokenFilterLogic>;

/// Equivalent to `DelimitedTermFrequencyTokenFilter(TokenStream)`, `this(input,
/// DEFAULT_DELIMITER)`.
///
/// # Errors
///
/// As [`new_delimited_term_frequency_token_filter_with_delimiter`].
pub fn new_delimited_term_frequency_token_filter(
    input: Box<dyn TokenStream>,
) -> Result<DelimitedTermFrequencyTokenFilter> {
    new_delimited_term_frequency_token_filter_with_delimiter(input, DEFAULT_DELIMITER)
}

/// Equivalent to `DelimitedTermFrequencyTokenFilter(TokenStream, char)`,
/// which adds the `CharTermAttribute` and the `TermFrequencyAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_delimited_term_frequency_token_filter_with_delimiter(
    input: Box<dyn TokenStream>,
    delimiter: char,
) -> Result<DelimitedTermFrequencyTokenFilter> {
    let mut filter = TokenFilter::new(input, DelimitedTermFrequencyTokenFilterLogic { delimiter });
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn TermFrequencyAttribute>()?;
    Ok(filter)
}
