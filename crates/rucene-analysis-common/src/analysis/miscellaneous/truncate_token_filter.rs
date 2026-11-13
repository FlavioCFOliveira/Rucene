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
//! Truncates tokens to a maximum length, ported from
//! `org.apache.lucene.analysis.miscellaneous.TruncateTokenFilter`.

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{
    chars_to_utf16, is_high_surrogate, is_low_surrogate, utf16_to_term_chars,
};

/// The body of [`TruncateTokenFilter`]: what Lucene's `TruncateTokenFilter`
/// adds to `TokenFilter`.
#[derive(Debug)]
pub struct TruncateTokenFilterLogic {
    truncate_after: i32,
    use_code_points: bool,
}

impl TokenFilterLogic for TruncateTokenFilterLogic {
    /// Equivalent to the final `TruncateTokenFilter.incrementToken()`.
    ///
    /// Java's lengths count the UTF-16 units of the term, which the filter
    /// computes here. **Divergence from Lucene 10.5.0.** Truncating after
    /// `chars` (Java's `truncateAfterChars`) can cut a surrogate pair, leaving
    /// a lone high surrogate in Java's buffer; it becomes U+FFFD here (see
    /// `jdk::character::utf16_to_term_chars`).
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        let is_keyword = source
            .get_attribute_interface::<dyn KeywordAttribute>()
            .expect("INVARIANT: the constructor added the KeywordAttribute")
            .is_keyword();
        if is_keyword {
            return Ok(true);
        }

        let mut term_attribute = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let arr = chars_to_utf16(&term_attribute.buffer()[..term_attribute.length()]);
        let len = arr.len();
        let truncate_after = self.truncate_after as usize;
        if len <= truncate_after {
            // the term is short enough, so we do not need to modify it
            // (works for both chars and codepoints)
            return Ok(true);
        }

        let new_length = if self.use_code_points {
            // code based on ICU4J's com.ibm.icu.text.UTF16#findOffsetFromCodePoint(...)
            // implementation:
            let mut ofs = 0;
            let mut remaining = truncate_after;
            while ofs < len && remaining > 0 {
                let unit = arr[ofs];
                ofs += 1;
                if is_high_surrogate(unit) && ofs < len && is_low_surrogate(arr[ofs]) {
                    ofs += 1;
                }
                remaining -= 1;
            }
            // check if we actually reached the limit and set new length based on calculated
            // offset:
            if remaining == 0 {
                Some(ofs)
            } else {
                None
            }
        } else {
            Some(truncate_after)
        };
        if let Some(new_length) = new_length {
            let chars = utf16_to_term_chars(&arr[..new_length]);
            term_attribute.copy_buffer(&chars, 0, chars.len());
        }
        Ok(true)
    }
}

/// A token filter for truncating the terms into a specific length. Fixed
/// prefix truncation, as a stemming method, produces good results on Turkish
/// language. It is reported that F5, using first 5 characters, produced best
/// results in *Information Retrieval on Turkish Texts*.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.TruncateTokenFilter`.
pub type TruncateTokenFilter = TokenFilter<TruncateTokenFilterLogic>;

/// Creates a new filter that truncates tokens to the given amount of Unicode
/// code points.
///
/// Equivalent to the static `TruncateTokenFilter.truncateAfterCodePoints(TokenStream, int)`.
///
/// # Errors
///
/// As [`new_truncate_token_filter`].
pub fn truncate_after_code_points(
    input: Box<dyn TokenStream>,
    n_code_points: i32,
) -> Result<TruncateTokenFilter> {
    new_truncate_token_filter_private(input, n_code_points, true)
}

/// Creates a new filter that truncates tokens to the given amount of UTF-16
/// chars.
///
/// Equivalent to the static `TruncateTokenFilter.truncateAfterChars(TokenStream, int)`.
///
/// # Errors
///
/// As [`new_truncate_token_filter`].
pub fn truncate_after_chars(
    input: Box<dyn TokenStream>,
    n_chars: i32,
) -> Result<TruncateTokenFilter> {
    new_truncate_token_filter_private(input, n_chars, false)
}

/// Creates a new filter that truncates tokens to the given amount of UTF-16
/// chars.
///
/// Equivalent to the deprecated `TruncateTokenFilter(TokenStream, int)`:
/// use [`truncate_after_chars`] or [`truncate_after_code_points`].
///
/// # Errors
///
/// `IllegalArgumentException` "truncateAfter parameter must be a positive
/// number: n", and what the attribute factory raises.
#[deprecated(note = "Use truncate_after_chars or truncate_after_code_points")]
pub fn new_truncate_token_filter(
    input: Box<dyn TokenStream>,
    n_chars: i32,
) -> Result<TruncateTokenFilter> {
    new_truncate_token_filter_private(input, n_chars, false)
}

/// Equivalent to the private `TruncateTokenFilter(TokenStream, int, boolean)`,
/// which adds the `CharTermAttribute` and the `KeywordAttribute`.
fn new_truncate_token_filter_private(
    input: Box<dyn TokenStream>,
    truncate_after: i32,
    use_code_points: bool,
) -> Result<TruncateTokenFilter> {
    let mut filter = TokenFilter::new(
        input,
        TruncateTokenFilterLogic {
            truncate_after,
            use_code_points,
        },
    );
    // Java's field initializers `addAttribute` run before the constructor body.
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    if truncate_after < 1 {
        return Err(LuceneError::illegal_argument(format!(
            "truncateAfter parameter must be a positive number: {truncate_after}"
        )));
    }
    Ok(filter)
}
