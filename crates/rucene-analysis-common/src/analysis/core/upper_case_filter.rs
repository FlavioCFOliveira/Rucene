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

//! Normalizes token text to UPPER CASE, ported from
//! `org.apache.lucene.analysis.core.UpperCaseFilter`.

use rucene::analysis::character_utils::to_upper_case;
use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The body of [`UpperCaseFilter`]: what Lucene's `UpperCaseFilter` adds to
/// `TokenFilter`.
#[derive(Clone, Copy, Debug, Default)]
pub struct UpperCaseFilterLogic;

impl TokenFilterLogic for UpperCaseFilterLogic {
    /// Equivalent to the final `UpperCaseFilter.incrementToken()`:
    /// `CharacterUtils.toUpperCase(termAtt.buffer(), 0, termAtt.length())`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let length = term_att.length();
            to_upper_case(term_att.buffer_mut(), 0, length);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Normalizes token text to UPPER CASE.
///
/// **NOTE:** In Unicode, this transformation may lose information when the
/// upper case character represents more than one lower case character. Use
/// this filter when you require uppercase tokens. Use the `LowerCaseFilter`
/// for general search matching.
///
/// Equivalent to `org.apache.lucene.analysis.core.UpperCaseFilter`.
pub type UpperCaseFilter = TokenFilter<UpperCaseFilterLogic>;

/// Create a new UpperCaseFilter, that normalizes token text to upper case.
///
/// Equivalent to `UpperCaseFilter(TokenStream)`, which adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_upper_case_filter(input: Box<dyn TokenStream>) -> Result<UpperCaseFilter> {
    let mut filter = TokenFilter::new(input, UpperCaseFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
