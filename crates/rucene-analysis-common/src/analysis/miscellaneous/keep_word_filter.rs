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
//! Keeps only the tokens of a word set, ported from
//! `org.apache.lucene.analysis.miscellaneous.KeepWordFilter`.

use rucene::analysis::{
    new_filtering_token_filter, CharArraySet, CharTermAttribute, FilteringTokenFilter,
    FilteringTokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::chars_to_utf16;

/// The `accept` of [`KeepWordFilter`]: what Lucene's `KeepWordFilter` adds to
/// `FilteringTokenFilter`.
#[derive(Debug)]
pub struct KeepWordFilterLogic {
    words: CharArraySet,
}

impl FilteringTokenFilterLogic for KeepWordFilterLogic {
    /// Equivalent to `KeepWordFilter.accept()`, `words.contains(termAtt.buffer(),
    /// 0, termAtt.length())`, over the term's UTF-16 units (the set's keys).
    fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
        let term_att = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let units = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
        Ok(self.words.contains_chars(Some(&units), 0, units.len()))
    }
}

/// A TokenFilter that only keeps tokens with text contained in the required
/// words. This filter behaves like the inverse of StopFilter.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.KeepWordFilter`
/// (since solr 1.3).
pub type KeepWordFilter = FilteringTokenFilter<KeepWordFilterLogic>;

/// Create a new [`KeepWordFilter`]. **NOTE**: The words set passed to this
/// constructor will be directly used by this filter and should not be
/// modified.
///
/// Equivalent to `KeepWordFilter(TokenStream, CharArraySet)`. Java's
/// `Objects.requireNonNull(words, "words")` has no counterpart: `words`
/// cannot be null here.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_keep_word_filter(
    input: Box<dyn TokenStream>,
    words: CharArraySet,
) -> Result<KeepWordFilter> {
    let mut filter = new_filtering_token_filter(input, KeepWordFilterLogic { words })?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
