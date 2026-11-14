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
//! Removes words that are too long or too short from the stream, ported from
//! `org.apache.lucene.analysis.miscellaneous.CodepointCountFilter`.

use rucene::analysis::{
    new_filtering_token_filter, CharTermAttribute, FilteringTokenFilter, FilteringTokenFilterLogic,
    TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::utf16_length;

/// The `accept` of [`CodepointCountFilter`]: what Lucene's
/// `CodepointCountFilter` adds to `FilteringTokenFilter`.
#[derive(Debug)]
pub struct CodepointCountFilterLogic {
    min: i32,
    max: i32,
}

impl FilteringTokenFilterLogic for CodepointCountFilterLogic {
    /// Equivalent to `CodepointCountFilter.accept()`. Java's `termAtt.length()`
    /// counts UTF-16 units and `Character.codePointCount` the code points of
    /// the term; both are computed here from the term's `char`s.
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        let term_att = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let term = &term_att.buffer()[..term_att.length()];
        let max32 = utf16_length(term) as i32;
        let min32 = max32 >> 1;
        if min32 >= self.min && max32 <= self.max {
            // definitely within range
            Ok(true)
        } else if min32 > self.max || max32 < self.min {
            // definitely not
            Ok(false)
        } else {
            // we must count to be sure
            let len = term.len() as i32;
            Ok(len >= self.min && len <= self.max)
        }
    }
}

/// Removes words that are too long or too short from the stream.
///
/// Note: Length is calculated as the number of Unicode codepoints.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.CodepointCountFilter`.
pub type CodepointCountFilter = FilteringTokenFilter<CodepointCountFilterLogic>;

/// Create a new [`CodepointCountFilter`]. This will filter out tokens whose
/// `CharTermAttribute` is either too short (`Character.codePointCount` <
/// `min`) or too long (`Character.codePointCount` > `max`).
///
/// Equivalent to `CodepointCountFilter(TokenStream, int, int)`.
///
/// # Errors
///
/// `IllegalArgumentException` when `min` is negative or above `max`, with
/// Java's messages, and what the attribute factory raises.
pub fn new_codepoint_count_filter(
    input: Box<dyn TokenStream>,
    min: i32,
    max: i32,
) -> Result<CodepointCountFilter> {
    let mut filter = new_filtering_token_filter(input, CodepointCountFilterLogic { min, max })?;
    // Java's field initializer `addAttribute` runs before the constructor body.
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    if min < 0 {
        return Err(LuceneError::illegal_argument(
            "minimum length must be greater than or equal to zero",
        ));
    }
    if min > max {
        return Err(LuceneError::illegal_argument(
            "maximum length must not be greater than minimum length",
        ));
    }
    Ok(filter)
}
