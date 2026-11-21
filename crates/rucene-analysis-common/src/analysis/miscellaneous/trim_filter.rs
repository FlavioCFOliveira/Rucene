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
//! Trims leading and trailing whitespace from tokens, ported from
//! `org.apache.lucene.analysis.miscellaneous.TrimFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::is_whitespace;

/// The body of [`TrimFilter`]: what Lucene's `TrimFilter` adds to
/// `TokenFilter`.
#[derive(Debug, Default)]
pub struct TrimFilterLogic;

impl TokenFilterLogic for TrimFilterLogic {
    /// Equivalent to `TrimFilter.incrementToken()`. Java's
    /// `Character.isWhitespace(char)` tests UTF-16 units; no supplementary
    /// character nor surrogate is whitespace, so testing the term's `char`s
    /// trims the same characters.
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
        let len = term_att.length();

        // TODO: Is this the right behavior or should we return false?  Currently, "  ", returns
        // true, so I think this should also return true
        if len == 0 {
            return Ok(true);
        }

        let term_buffer = term_att.buffer();
        // eat the first characters
        let mut start = 0;
        while start < len && is_whitespace(term_buffer[start] as i32) {
            start += 1;
        }
        // eat the end characters
        let mut end = len;
        while end >= start && end > 0 && is_whitespace(term_buffer[end - 1] as i32) {
            end -= 1;
        }

        if start > 0 || end < len {
            if start < end {
                let trimmed: Vec<char> = term_buffer[start..end].to_vec();
                term_att.copy_buffer(&trimmed, 0, trimmed.len());
            } else {
                term_att.set_empty();
            }
        }

        Ok(true)
    }
}

/// Trims leading and trailing whitespace from Tokens in the stream.
///
/// As of Lucene 4.4, this filter does not support updateOffsets=true
/// anymore as it can lead to broken token streams.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.TrimFilter`.
pub type TrimFilter = TokenFilter<TrimFilterLogic>;

/// Create a new [`TrimFilter`].
///
/// Equivalent to `TrimFilter(TokenStream)`, which adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_trim_filter(input: Box<dyn TokenStream>) -> Result<TrimFilter> {
    let mut filter = TokenFilter::new(input, TrimFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
