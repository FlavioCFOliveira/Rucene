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
//! Emits every input token twice, once as keyword and once not, ported from
//! `org.apache.lucene.analysis.miscellaneous.KeywordRepeatFilter`.

use rucene::analysis::{
    KeywordAttribute, PositionIncrementAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

/// The body of [`KeywordRepeatFilter`]: what Lucene's `KeywordRepeatFilter`
/// adds to `TokenFilter`.
#[derive(Debug, Default)]
pub struct KeywordRepeatFilterLogic {
    state: Option<CapturedState>,
}

fn set_keyword(source: &AttributeSource, keyword: bool) {
    source
        .get_attribute_interface_mut::<dyn KeywordAttribute>()
        .expect("INVARIANT: the constructor added the KeywordAttribute")
        .set_keyword(keyword);
}

impl TokenFilterLogic for KeywordRepeatFilterLogic {
    /// Equivalent to `KeywordRepeatFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(state) = self.state.take() {
            source.restore_state(&state)?;
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(0);
            set_keyword(source, false);
            return Ok(true);
        }
        if input.increment_token()? {
            self.state = source.capture_state();
            set_keyword(source, true);
            return Ok(true);
        }
        Ok(false)
    }

    /// Equivalent to `KeywordRepeatFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.state = None;
        Ok(())
    }
}

/// This TokenFilter emits each incoming token twice once as keyword and once
/// non-keyword, in other words once with `KeywordAttribute.setKeyword(boolean)`
/// set to `true` and once set to `false`. This is useful if used with a stem
/// filter that respects the `KeywordAttribute` to index the stemmed and the
/// un-stemmed version of a term into the same field.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.KeywordRepeatFilter`.
pub type KeywordRepeatFilter = TokenFilter<KeywordRepeatFilterLogic>;

/// Construct a token stream filtering the given input.
///
/// Equivalent to `KeywordRepeatFilter(TokenStream)`, which adds the
/// `KeywordAttribute` and the `PositionIncrementAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_keyword_repeat_filter(input: Box<dyn TokenStream>) -> Result<KeywordRepeatFilter> {
    let mut filter = TokenFilter::new(input, KeywordRepeatFilterLogic::default());
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    Ok(filter)
}
