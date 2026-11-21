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
//! Marks terms matching a pattern as keywords, ported from
//! `org.apache.lucene.analysis.miscellaneous.PatternKeywordMarkerFilter`.

use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::{CharTermAttribute, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::keyword_marker_filter::{
    new_keyword_marker_filter, KeywordMarkerFilter, KeywordMarkerFilterLogic,
};

/// The `isKeyword()` of [`PatternKeywordMarkerFilter`].
#[derive(Debug)]
pub struct PatternKeywordMarkerFilterLogic {
    /// Java keeps `pattern.matcher("")` and resets it on every term; the
    /// compiled pattern is the matcher's whole state here.
    matcher: JavaPattern,
}

impl KeywordMarkerFilterLogic for PatternKeywordMarkerFilterLogic {
    /// Equivalent to `PatternKeywordMarkerFilter.isKeyword()`:
    /// `matcher.reset(termAtt); return matcher.matches()`.
    fn is_keyword(&mut self, source: &AttributeSource) -> Result<bool> {
        let term: String = {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            term_att.buffer()[..term_att.length()].iter().collect()
        };
        Ok(self.matcher.matches(&term))
    }
}

/// Marks terms as keywords via the `KeywordAttribute`. Each token that
/// matches the provided pattern is marked as a keyword by setting
/// `KeywordAttribute.setKeyword(boolean)` to `true`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.PatternKeywordMarkerFilter`.
pub type PatternKeywordMarkerFilter = KeywordMarkerFilter<PatternKeywordMarkerFilterLogic>;

/// Create a new [`PatternKeywordMarkerFilter`], that marks the current token
/// as a keyword if the tokens term buffer matches the provided pattern via
/// the `KeywordAttribute`.
///
/// Equivalent to `PatternKeywordMarkerFilter(TokenStream, Pattern)`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_pattern_keyword_marker_filter(
    input: Box<dyn TokenStream>,
    pattern: JavaPattern,
) -> Result<PatternKeywordMarkerFilter> {
    let mut filter =
        new_keyword_marker_filter(input, PatternKeywordMarkerFilterLogic { matcher: pattern })?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
