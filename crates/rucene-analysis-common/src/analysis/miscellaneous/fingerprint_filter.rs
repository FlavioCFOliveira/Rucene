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
//! Builds one fingerprint token from the sorted unique terms of a stream,
//! ported from `org.apache.lucene.analysis.miscellaneous.FingerprintFilter`.

use rucene::analysis::{
    CharArraySet, CharTermAttribute, OffsetAttribute, PositionIncrementAttribute,
    PositionLengthAttribute, TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// Equivalent to `FingerprintFilter.DEFAULT_MAX_OUTPUT_TOKEN_SIZE`.
pub const DEFAULT_MAX_OUTPUT_TOKEN_SIZE: i32 = 1024;

/// Equivalent to `FingerprintFilter.DEFAULT_SEPARATOR`.
pub const DEFAULT_SEPARATOR: char = ' ';

/// The body of [`FingerprintFilter`]: what Lucene's `FingerprintFilter` adds
/// to `TokenFilter`.
///
/// The terms are kept as their UTF-16 units, as Java's `char[]` keys of the
/// `CharArraySet` are, so the lengths and the sort order are Java's.
#[derive(Debug)]
pub struct FingerprintFilterLogic {
    unique_terms: Option<CharArraySet>,
    max_output_token_size: i32,
    final_state: Option<CapturedState>,
    separator: char,
    input_ended: bool,
}

impl FingerprintFilterLogic {
    /// Gathers all tokens from input, de-duplicates, sorts then concatenates.
    ///
    /// Equivalent to the private `FingerprintFilter.buildSingleOutputToken()`.
    fn build_single_output_token(
        &mut self,
        source: &AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        self.input_ended = false;

        let mut cloned_last_term: Option<Vec<u16>> = None;
        let mut unique_terms = CharArraySet::new(8, false);
        let mut output_token_size: i32 = 0;
        while input.increment_token()? {
            if output_token_size > self.max_output_token_size {
                continue;
            }

            let mut term = {
                let term_attribute = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                chars_to_utf16(&term_attribute.buffer()[..term_attribute.length()])
            };
            let length = term.len();
            if !unique_terms.contains_chars(Some(&term), 0, length) {
                // clone the term, and add to the set of seen terms.
                cloned_last_term = Some(term.clone());
                if unique_terms.size() > 0 {
                    output_token_size = output_token_size.wrapping_add(1); // Add 1 for the separator char we will output
                }
                unique_terms.add_chars(&mut term, 0, length);
                output_token_size = output_token_size.wrapping_add(length as i32);
            }
        }
        // Force end-of-stream operations to get the final state.
        input.end()?;
        self.input_ended = true;

        // Gathering complete - now output exactly zero or one token:

        // Set the attributes for the single output token
        {
            let mut offset_att = source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute");
            let end_offset = offset_att.end_offset();
            offset_att.set_offset(0, end_offset);
        }
        source
            .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
            .expect("INVARIANT: the constructor added the PositionLengthAttribute")
            .set_position_length(1);
        source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .set_position_increment(1);
        source
            .get_attribute_interface_mut::<dyn TypeAttribute>()
            .expect("INVARIANT: the constructor added the TypeAttribute")
            .set_type_static("fingerprint");

        let mut term_attribute = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");

        // No tokens gathered - no output
        if unique_terms.size() < 1 {
            term_attribute.set_empty();
            self.unique_terms = Some(unique_terms);
            return Ok(false);
        }

        // Tokens gathered are too large - no output
        if output_token_size > self.max_output_token_size {
            term_attribute.set_empty();
            unique_terms.clear();
            self.unique_terms = Some(unique_terms);
            return Ok(false);
        }

        // Special case - faster option when we have a single token
        if unique_terms.size() == 1 {
            let chars = utf16_to_term_chars(
                cloned_last_term
                    .as_deref()
                    .expect("INVARIANT: a term was added"),
            );
            term_attribute.copy_buffer(&chars, 0, chars.len());
            unique_terms.clear();
            self.unique_terms = Some(unique_terms);
            return Ok(true);
        }

        // Sort the set of deduplicated tokens and combine
        let mut items: Vec<Vec<u16>> = unique_terms.iter().map(<[u16]>::to_vec).collect();
        items.sort_by(|v1, v2| {
            let lim = v1.len().min(v2.len());
            let mut k = 0;
            while k < lim {
                let c1 = v1[k];
                let c2 = v2[k];
                if c1 != c2 {
                    return (i32::from(c1) - i32::from(c2)).cmp(&0);
                }
                k += 1;
            }
            (v1.len() as i64 - v2.len() as i64).cmp(&0)
        });

        // TODO lets append directly to termAttribute?
        let mut sb: Vec<u16> = Vec::new();
        let mut separator = [0u16; 2];
        let separator = self.separator.encode_utf16(&mut separator);
        for item in &items {
            if !sb.is_empty() {
                sb.extend_from_slice(separator);
            }
            sb.extend_from_slice(item);
        }
        let chars = utf16_to_term_chars(&sb);
        term_attribute.copy_buffer(&chars, 0, chars.len());
        unique_terms.clear();
        self.unique_terms = Some(unique_terms);
        Ok(true)
    }
}

impl TokenFilterLogic for FingerprintFilterLogic {
    /// Equivalent to the final `FingerprintFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.input_ended {
            return Ok(false);
        }
        let result = self.build_single_output_token(source, input)?;
        self.final_state = source.capture_state();
        Ok(result)
    }

    /// Equivalent to the final `FingerprintFilter.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        if !self.input_ended {
            // Rare case - If an IOException occurs while performing buildSingleOutputToken
            // we may not have called input.end() already
            input.end()?;
            self.input_ended = true;
        }

        if let Some(final_state) = &self.final_state {
            source.restore_state(final_state)?;
        }
        Ok(())
    }

    /// Equivalent to `FingerprintFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.input_ended = false;
        self.unique_terms = None;
        Ok(())
    }
}

/// Filter outputs a single token which is a concatenation of the sorted and
/// de-duplicated set of input tokens. This can be useful for clustering/
/// linking use cases.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.FingerprintFilter`.
pub type FingerprintFilter = TokenFilter<FingerprintFilterLogic>;

/// Create a new FingerprintFilter with default settings.
///
/// Equivalent to `FingerprintFilter(TokenStream)`, `this(input,
/// DEFAULT_MAX_OUTPUT_TOKEN_SIZE, DEFAULT_SEPARATOR)`.
///
/// # Errors
///
/// As [`new_fingerprint_filter_with`].
pub fn new_fingerprint_filter(input: Box<dyn TokenStream>) -> Result<FingerprintFilter> {
    new_fingerprint_filter_with(input, DEFAULT_MAX_OUTPUT_TOKEN_SIZE, DEFAULT_SEPARATOR)
}

/// Create a new FingerprintFilter with control over all settings:
/// `max_output_token_size` is the maximum length of the fingerprint token
/// output (if exceeded, no token is emitted), and `separator` the character
/// used to separate tokens combined into the single output token.
///
/// Equivalent to `FingerprintFilter(TokenStream, int, char)`, which adds the
/// `CharTermAttribute`, `OffsetAttribute`, `PositionIncrementAttribute`,
/// `PositionLengthAttribute` and `TypeAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_fingerprint_filter_with(
    input: Box<dyn TokenStream>,
    max_output_token_size: i32,
    separator: char,
) -> Result<FingerprintFilter> {
    let mut filter = TokenFilter::new(
        input,
        FingerprintFilterLogic {
            unique_terms: None,
            max_output_token_size,
            final_state: None,
            separator,
            input_ended: false,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}
