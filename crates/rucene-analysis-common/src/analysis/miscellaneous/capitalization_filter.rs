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
//! Applies normal capitalization rules to tokens, ported from
//! `org.apache.lucene.analysis.miscellaneous.CapitalizationFilter`.

use rucene::analysis::{
    CharArraySet, CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{
    chars_to_utf16, to_lower_case_unit, to_upper_case_unit, utf16_to_term_chars,
};

/// Equivalent to `CapitalizationFilter.DEFAULT_MAX_WORD_COUNT`.
pub const DEFAULT_MAX_WORD_COUNT: i32 = i32::MAX;

/// Equivalent to `CapitalizationFilter.DEFAULT_MAX_TOKEN_LENGTH`.
pub const DEFAULT_MAX_TOKEN_LENGTH: i32 = i32::MAX;

/// The body of [`CapitalizationFilter`]: what Lucene's `CapitalizationFilter`
/// adds to `TokenFilter`.
///
/// Java works on the term's `char[]`, UTF-16 units: `Character.toLowerCase(char)`
/// and `toUpperCase(char)` map one unit at a time (a surrogate maps to
/// itself), and the word lengths count units. The filter therefore works on
/// the UTF-16 units of the term and writes the result back; the units it
/// writes back are those of the term with BMP characters remapped, so they
/// always decode to the same number of `char`s.
#[derive(Debug)]
pub struct CapitalizationFilterLogic {
    only_first_word: bool,
    keep: Option<CharArraySet>,
    force_first_letter: bool,
    /// Java's `Collection<char[]> okPrefix`, each prefix as UTF-16 units.
    ok_prefix: Option<Vec<Vec<u16>>>,
    min_word_length: i32,
    max_word_count: i32,
    max_token_length: i32,
}

impl CapitalizationFilterLogic {
    /// Equivalent to the private `CapitalizationFilter.processWord(char[], int,
    /// int, int)`.
    fn process_word(&self, buffer: &mut [u16], offset: usize, length: usize, word_count: i32) {
        if length < 1 {
            return;
        }

        if self.only_first_word && word_count > 0 {
            for i in 0..length {
                buffer[offset + i] = to_lower_case_unit(buffer[offset + i]);
            }
            return;
        }

        if let Some(keep) = &self.keep {
            if keep.contains_chars(Some(buffer), offset, length) {
                if word_count == 0 && self.force_first_letter {
                    buffer[offset] = to_upper_case_unit(buffer[offset]);
                }
                return;
            }
        }

        if (length as i64) < i64::from(self.min_word_length) {
            return;
        }

        if let Some(ok_prefix) = &self.ok_prefix {
            for prefix in ok_prefix {
                // don't bother checking if the buffer length is less than the prefix
                if length >= prefix.len() {
                    let mut matched = true;
                    for i in 0..prefix.len() {
                        if prefix[i] != buffer[offset + i] {
                            matched = false;
                            break;
                        }
                    }
                    if matched {
                        return;
                    }
                }
            }
        }

        // We know it has at least one character
        buffer[offset] = to_upper_case_unit(buffer[offset]);

        for i in 1..length {
            buffer[offset + i] = to_lower_case_unit(buffer[offset + i]);
        }
    }
}

impl TokenFilterLogic for CapitalizationFilterLogic {
    /// Equivalent to `CapitalizationFilter.incrementToken()`.
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
        let mut term_buffer = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
        let term_buffer_length = term_buffer.len();
        let mut backup: Option<Vec<u16>> = None;

        if self.max_word_count < DEFAULT_MAX_WORD_COUNT {
            // make a backup in case we exceed the word count
            backup = Some(term_buffer.clone());
        }

        if (term_buffer_length as i64) < i64::from(self.max_token_length) {
            let mut word_count = 0;

            let mut last_word_start = 0;
            let mut i = 0;
            while i < term_buffer_length {
                let c = term_buffer[i];
                if c <= u16::from(b' ') || c == u16::from(b'.') {
                    let len = i - last_word_start;
                    if len > 0 {
                        self.process_word(&mut term_buffer, last_word_start, len, word_count);
                        word_count += 1;
                        last_word_start = i + 1;
                        i += 1;
                    }
                }
                i += 1;
            }

            // process the last word
            if last_word_start < term_buffer_length {
                self.process_word(
                    &mut term_buffer,
                    last_word_start,
                    term_buffer_length - last_word_start,
                    word_count,
                );
                word_count += 1;
            }

            let chars = if word_count > self.max_word_count {
                utf16_to_term_chars(backup.as_deref().expect("INVARIANT: a backup was made"))
            } else {
                utf16_to_term_chars(&term_buffer)
            };
            term_att.copy_buffer(&chars, 0, chars.len());
        }

        Ok(true)
    }
}

/// A filter to apply normal capitalization rules to Tokens. It will make the
/// first letter capital and the rest lower case.
///
/// This filter is particularly useful to build nice looking facet
/// parameters. This filter is not appropriate if you intend to use a prefix
/// query.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.CapitalizationFilter`.
pub type CapitalizationFilter = TokenFilter<CapitalizationFilterLogic>;

/// Creates a CapitalizationFilter with the default parameters: `(in, true,
/// null, true, null, 0, DEFAULT_MAX_WORD_COUNT, DEFAULT_MAX_TOKEN_LENGTH)`.
///
/// Equivalent to `CapitalizationFilter(TokenStream)`.
///
/// # Errors
///
/// As [`new_capitalization_filter_with`].
pub fn new_capitalization_filter(input: Box<dyn TokenStream>) -> Result<CapitalizationFilter> {
    new_capitalization_filter_with(
        input,
        true,
        None,
        true,
        None,
        0,
        DEFAULT_MAX_WORD_COUNT,
        DEFAULT_MAX_TOKEN_LENGTH,
    )
}

/// Creates a CapitalizationFilter with the specified parameters.
///
/// * `input` - input tokenstream
/// * `only_first_word` - should each word be capitalized or all of the words?
/// * `keep` - a keep word list. Each word that should be kept separated by
///   whitespace.
/// * `force_first_letter` - Force the first letter to be capitalized even if
///   it is in the keep list.
/// * `ok_prefix` - do not change word capitalization if a word begins with
///   something in this list (each prefix as the UTF-16 units of Java's
///   `char[]`).
/// * `min_word_length` - how long the word needs to be to get capitalization
///   applied. If the minWordLength is 3, "and" > "And" but "or" stays "or".
/// * `max_word_count` - if the token contains more then maxWordCount words,
///   the capitalization is assumed to be correct.
/// * `max_token_length` - ???
///
/// Equivalent to `CapitalizationFilter(TokenStream, boolean, CharArraySet,
/// boolean, Collection<char[]>, int, int, int)`.
///
/// # Errors
///
/// `IllegalArgumentException`, with Java's messages, for a negative
/// `min_word_length` or a `max_word_count` or `max_token_length` below one;
/// what the attribute factory raises.
#[allow(clippy::too_many_arguments)]
pub fn new_capitalization_filter_with(
    input: Box<dyn TokenStream>,
    only_first_word: bool,
    keep: Option<CharArraySet>,
    force_first_letter: bool,
    ok_prefix: Option<Vec<Vec<u16>>>,
    min_word_length: i32,
    max_word_count: i32,
    max_token_length: i32,
) -> Result<CapitalizationFilter> {
    let mut filter = TokenFilter::new(
        input,
        CapitalizationFilterLogic {
            only_first_word,
            keep,
            force_first_letter,
            ok_prefix,
            min_word_length,
            max_word_count,
            max_token_length,
        },
    );
    // Java's field initializer `addAttribute` runs before the constructor body.
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    if min_word_length < 0 {
        return Err(LuceneError::illegal_argument(
            "minWordLength must be greater than or equal to zero",
        ));
    }
    if max_word_count < 1 {
        return Err(LuceneError::illegal_argument(
            "maxWordCount must be greater than zero",
        ));
    }
    if max_token_length < 1 {
        return Err(LuceneError::illegal_argument(
            "maxTokenLength must be greater than zero",
        ));
    }
    Ok(filter)
}
