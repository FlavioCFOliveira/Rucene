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
//! Marks terms of a set as keywords, ported from
//! `org.apache.lucene.analysis.miscellaneous.SetKeywordMarkerFilter`.

use rucene::analysis::{CharArraySet, CharTermAttribute, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::keyword_marker_filter::{
    new_keyword_marker_filter, KeywordMarkerFilter, KeywordMarkerFilterLogic,
};
use crate::jdk::character::chars_to_utf16;

/// The `isKeyword()` of [`SetKeywordMarkerFilter`].
#[derive(Debug)]
pub struct SetKeywordMarkerFilterLogic {
    keyword_set: CharArraySet,
}

impl KeywordMarkerFilterLogic for SetKeywordMarkerFilterLogic {
    /// Equivalent to `SetKeywordMarkerFilter.isKeyword()`,
    /// `keywordSet.contains(termAtt.buffer(), 0, termAtt.length())`, over the
    /// term's UTF-16 units (the set's keys).
    fn is_keyword(&mut self, source: &AttributeSource) -> Result<bool> {
        let term_att = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let units = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
        Ok(self
            .keyword_set
            .contains_chars(Some(&units), 0, units.len()))
    }
}

/// Marks terms as keywords via the `KeywordAttribute`. Each token contained
/// in the provided set is marked as a keyword by setting
/// `KeywordAttribute.setKeyword(boolean)` to `true`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.SetKeywordMarkerFilter`.
pub type SetKeywordMarkerFilter = KeywordMarkerFilter<SetKeywordMarkerFilterLogic>;

/// Create a new [`SetKeywordMarkerFilter`], that marks the current token as a
/// keyword if the tokens term buffer is contained in the given set via the
/// `KeywordAttribute`.
///
/// Equivalent to `SetKeywordMarkerFilter(TokenStream, CharArraySet)`, whose
/// `super(in)` adds the `KeywordAttribute` and whose field adds the
/// `CharTermAttribute`. Java's `Objects.requireNonNull(keywordSet,
/// "keywordSet")` has no counterpart: the set cannot be null here.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_set_keyword_marker_filter(
    input: Box<dyn TokenStream>,
    keyword_set: CharArraySet,
) -> Result<SetKeywordMarkerFilter> {
    let mut filter = new_keyword_marker_filter(input, SetKeywordMarkerFilterLogic { keyword_set })?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
