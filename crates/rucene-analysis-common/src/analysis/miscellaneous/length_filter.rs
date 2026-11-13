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
//! `org.apache.lucene.analysis.miscellaneous.LengthFilter`.

use rucene::analysis::{
    new_filtering_token_filter, CharTermAttribute, FilteringTokenFilter, FilteringTokenFilterLogic,
    TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::utf16_length;

/// The `accept` of [`LengthFilter`]: what Lucene's `LengthFilter` adds to
/// `FilteringTokenFilter`.
#[derive(Debug)]
pub struct LengthFilterLogic {
    min: i32,
    max: i32,
}

impl FilteringTokenFilterLogic for LengthFilterLogic {
    /// Equivalent to `LengthFilter.accept()`: Java's `termAtt.length()`
    /// counts UTF-16 units, so the length is the term's UTF-16 length.
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        let term_att = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let len = utf16_length(&term_att.buffer()[..term_att.length()]) as i32;
        Ok(len >= self.min && len <= self.max)
    }
}

/// Removes words that are too long or too short from the stream.
///
/// Note: Length is calculated as the number of UTF-16 code units.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.LengthFilter`.
pub type LengthFilter = FilteringTokenFilter<LengthFilterLogic>;

/// Create a new [`LengthFilter`]. This will filter out tokens whose
/// `CharTermAttribute` is either too short (`length()` < `min`) or too long
/// (`length()` > `max`).
///
/// Equivalent to `LengthFilter(TokenStream, int, int)`.
///
/// # Errors
///
/// `IllegalArgumentException` when `min` is negative or above `max`, with
/// Java's messages, and what the attribute factory raises.
pub fn new_length_filter(input: Box<dyn TokenStream>, min: i32, max: i32) -> Result<LengthFilter> {
    let mut filter = new_filtering_token_filter(input, LengthFilterLogic { min, max })?;
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
