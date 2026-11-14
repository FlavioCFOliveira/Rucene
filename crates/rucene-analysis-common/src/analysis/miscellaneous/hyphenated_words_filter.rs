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
//! Joins words split by a hyphen at a line end, ported from
//! `org.apache.lucene.analysis.miscellaneous.HyphenatedWordsFilter`.

use rucene::analysis::{
    CharTermAttribute, OffsetAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

/// The body of [`HyphenatedWordsFilter`]: what Lucene's
/// `HyphenatedWordsFilter` adds to `TokenFilter`.
#[derive(Debug, Default)]
pub struct HyphenatedWordsFilterLogic {
    /// Java's `StringBuilder hyphenated`.
    hyphenated: Vec<char>,
    saved_state: Option<CapturedState>,
    exhausted: bool,
    last_end_offset: i32,
}

impl HyphenatedWordsFilterLogic {
    /// Writes the joined hyphenated term to the term attributes.
    ///
    /// Equivalent to the private `HyphenatedWordsFilter.unhyphenate()`.
    fn unhyphenate(&mut self, source: &AttributeSource) -> Result<()> {
        let state = self
            .saved_state
            .take()
            .expect("INVARIANT: a state was saved");
        source.restore_state(&state)?;
        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute")
            .copy_buffer(&self.hyphenated, 0, self.hyphenated.len());
        let mut offset_attribute = source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute");
        let start_offset = offset_attribute.start_offset();
        offset_attribute.set_offset(start_offset, self.last_end_offset);
        self.hyphenated.clear();
        Ok(())
    }
}

impl TokenFilterLogic for HyphenatedWordsFilterLogic {
    /// Equivalent to `HyphenatedWordsFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        while !self.exhausted && input.increment_token()? {
            let term: Vec<char> = {
                let term_attribute = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                term_attribute.buffer()[..term_attribute.length()].to_vec()
            };
            let term_length = term.len();
            self.last_end_offset = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .end_offset();

            if term_length > 0 && term[term_length - 1] == '-' {
                // a hyphenated word
                // capture the state of the first token only
                if self.saved_state.is_none() {
                    self.saved_state = source.capture_state();
                }
                self.hyphenated.extend_from_slice(&term[..term_length - 1]);
            } else if self.saved_state.is_none() {
                // not part of a hyphenated word.
                return Ok(true);
            } else {
                // the final portion of a hyphenated word
                self.hyphenated.extend_from_slice(&term[..term_length]);
                self.unhyphenate(source)?;
                return Ok(true);
            }
        }

        self.exhausted = true;

        if self.saved_state.is_some() {
            // the final term ends with a hyphen
            // add back the hyphen, for backwards compatibility.
            self.hyphenated.push('-');
            self.unhyphenate(source)?;
            return Ok(true);
        }

        Ok(false)
    }

    /// Equivalent to `HyphenatedWordsFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.hyphenated.clear();
        self.saved_state = None;
        self.exhausted = false;
        self.last_end_offset = 0;
        Ok(())
    }
}

/// When the plain text is extracted from documents, we will often have many
/// words hyphenated and broken into two lines. This is often the case with
/// documents where narrow text columns are used, such as newsletters. In
/// order to increase search efficiency, this filter puts hyphenated words
/// broken into two lines back together. This filter should be used on
/// indexing time only.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.HyphenatedWordsFilter`.
pub type HyphenatedWordsFilter = TokenFilter<HyphenatedWordsFilterLogic>;

/// Creates a new HyphenatedWordsFilter.
///
/// Equivalent to `HyphenatedWordsFilter(TokenStream)`, which adds the
/// `CharTermAttribute` and the `OffsetAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_hyphenated_words_filter(input: Box<dyn TokenStream>) -> Result<HyphenatedWordsFilter> {
    let mut filter = TokenFilter::new(input, HyphenatedWordsFilterLogic::default());
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(filter)
}
