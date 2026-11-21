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
//! Removes tokens repeated at the same position, ported from
//! `org.apache.lucene.analysis.miscellaneous.RemoveDuplicatesTokenFilter`.

use rucene::analysis::{
    CharArraySet, CharTermAttribute, PositionIncrementAttribute, TokenFilter, TokenFilterLogic,
    TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::chars_to_utf16;

/// The body of [`RemoveDuplicatesTokenFilter`]: what Lucene's
/// `RemoveDuplicatesTokenFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct RemoveDuplicatesTokenFilterLogic {
    /// Java's `new CharArraySet(8, false)`.
    previous: CharArraySet,
}

impl TokenFilterLogic for RemoveDuplicatesTokenFilterLogic {
    /// Equivalent to `RemoveDuplicatesTokenFilter.incrementToken()`; the terms
    /// are kept as their UTF-16 units, the set's keys.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        while input.increment_token()? {
            let mut term = {
                let term_attribute = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                chars_to_utf16(&term_attribute.buffer()[..term_attribute.length()])
            };
            let length = term.len();
            let pos_increment = source
                .get_attribute_interface::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .get_position_increment();

            if pos_increment > 0 {
                self.previous.clear();
            }

            let duplicate =
                pos_increment == 0 && self.previous.contains_chars(Some(&term), 0, length);

            // clone the term, and add to the set of seen terms.
            self.previous.add_chars(&mut term, 0, length);

            if !duplicate {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Equivalent to `RemoveDuplicatesTokenFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.previous.clear();
        Ok(())
    }
}

/// A TokenFilter which filters out Tokens at the same position and Term
/// text as the previous token in the stream.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.RemoveDuplicatesTokenFilter`.
pub type RemoveDuplicatesTokenFilter = TokenFilter<RemoveDuplicatesTokenFilterLogic>;

/// Creates a new RemoveDuplicatesTokenFilter.
///
/// Equivalent to `RemoveDuplicatesTokenFilter(TokenStream)`, which adds the
/// `CharTermAttribute` and the `PositionIncrementAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_remove_duplicates_token_filter(
    input: Box<dyn TokenStream>,
) -> Result<RemoveDuplicatesTokenFilter> {
    let mut filter = TokenFilter::new(
        input,
        RemoveDuplicatesTokenFilterLogic {
            previous: CharArraySet::new(8, false),
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    Ok(filter)
}
