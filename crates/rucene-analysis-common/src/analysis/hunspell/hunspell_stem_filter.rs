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

//! The Hunspell stemming filter, ported from
//! `org.apache.lucene.analysis.hunspell.HunspellStemFilter`.

use std::collections::VecDeque;
use std::sync::Arc;

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, PositionIncrementAttribute, TokenFilter, TokenFilterLogic,
    TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::CharsRef;

use super::dictionary::Dictionary;
use super::stemmer::Stemmer;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`HunspellStemFilter`]: what Lucene's `HunspellStemFilter`
/// adds to `TokenFilter`.
#[derive(Debug)]
pub struct HunspellStemFilterLogic {
    stemmer: Stemmer,
    buffer: Option<VecDeque<CharsRef>>,
    saved_state: Option<CapturedState>,
    dedup: bool,
    longest_only: bool,
}

/// `termAtt.setEmpty().append(stem)`.
fn set_term(source: &mut AttributeSource, stem: &CharsRef) {
    let chars = utf16_to_term_chars(stem.chars());
    source
        .get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("INVARIANT: the constructor added the CharTermAttribute")
        .copy_buffer(&chars, 0, chars.len());
}

impl TokenFilterLogic for HunspellStemFilterLogic {
    /// Equivalent to `HunspellStemFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(buffer) = &mut self.buffer {
            if let Some(next_stem) = buffer.pop_front() {
                let saved_state = self
                    .saved_state
                    .as_ref()
                    .expect("INVARIANT: captured with the remaining stems");
                source.restore_state(saved_state)?;
                source
                    .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                    .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                    .set_position_increment(0);
                set_term(source, &next_stem);
                return Ok(true);
            }
        }

        if !input.increment_token()? {
            return Ok(false);
        }

        let is_keyword = source
            .get_attribute_interface::<dyn KeywordAttribute>()
            .expect("INVARIANT: the constructor added the KeywordAttribute")
            .is_keyword();
        if is_keyword {
            return Ok(true);
        }

        let term = {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            chars_to_utf16(&term_att.buffer()[..term_att.length()])
        };
        let mut buffer: Vec<CharsRef> = if self.dedup {
            self.stemmer.unique_stems(&term, term.len())?
        } else {
            self.stemmer.stem(&term, term.len())?
        };

        if buffer.is_empty() {
            // we do not know this word, return it unchanged
            self.buffer = Some(VecDeque::new());
            return Ok(true);
        }

        if self.longest_only && buffer.len() > 1 {
            buffer.sort_by(|o1, o2| {
                o2.length
                    .cmp(&o1.length)
                    // tie break on text
                    .then_with(|| o2.cmp(o1))
            });
        }

        let mut buffer: VecDeque<CharsRef> = buffer.into();
        let stem = buffer.pop_front().expect("INVARIANT: not empty");
        set_term(source, &stem);

        if self.longest_only {
            buffer.clear();
        } else if !buffer.is_empty() {
            self.saved_state = source.capture_state();
        }
        self.buffer = Some(buffer);

        Ok(true)
    }

    /// Equivalent to `HunspellStemFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.buffer = None;
        Ok(())
    }
}

/// TokenFilter that uses hunspell affix rules and words to stem tokens. Since
/// hunspell supports a word having multiple stems, this filter can emit
/// multiple tokens for each consumed token.
///
/// Note: This filter is aware of the `KeywordAttribute`. To prevent certain
/// terms from being passed to the stemmer `KeywordAttribute.isKeyword()`
/// should be set to `true` in a previous `TokenStream`.
///
/// Note: For including the original term as well as the stemmed version, see
/// `KeywordRepeatFilterFactory`.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.HunspellStemFilter`.
pub type HunspellStemFilter = TokenFilter<HunspellStemFilterLogic>;

/// Create a [`HunspellStemFilter`] outputting all possible stems.
///
/// Equivalent to `HunspellStemFilter(TokenStream, Dictionary)`, `this(input,
/// dictionary, true)`.
///
/// # Errors
///
/// As [`new_hunspell_stem_filter_with`].
pub fn new_hunspell_stem_filter(
    input: Box<dyn TokenStream>,
    dictionary: Arc<Dictionary>,
) -> Result<HunspellStemFilter> {
    new_hunspell_stem_filter_with_dedup(input, dictionary, true)
}

/// Create a [`HunspellStemFilter`] outputting all possible stems.
///
/// Equivalent to `HunspellStemFilter(TokenStream, Dictionary, boolean)`,
/// `this(input, dictionary, dedup, false)`.
///
/// # Errors
///
/// As [`new_hunspell_stem_filter_with`].
pub fn new_hunspell_stem_filter_with_dedup(
    input: Box<dyn TokenStream>,
    dictionary: Arc<Dictionary>,
    dedup: bool,
) -> Result<HunspellStemFilter> {
    new_hunspell_stem_filter_with(input, dictionary, dedup, false)
}

/// Creates a new HunspellStemFilter that will stem tokens from the given
/// TokenStream using affix rules in the provided Dictionary; `longest_only`
/// true if only the longest term should be output.
///
/// Equivalent to `HunspellStemFilter(TokenStream, Dictionary, boolean,
/// boolean)`: there is no deduplication when `longest_only` is set, and the
/// filter adds the `CharTermAttribute`, `PositionIncrementAttribute` and
/// `KeywordAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_hunspell_stem_filter_with(
    input: Box<dyn TokenStream>,
    dictionary: Arc<Dictionary>,
    dedup: bool,
    longest_only: bool,
) -> Result<HunspellStemFilter> {
    let mut filter = TokenFilter::new(
        input,
        HunspellStemFilterLogic {
            // don't waste time deduping if longestOnly is set
            dedup: dedup && !longest_only,
            stemmer: Stemmer::new(dictionary),
            longest_only,
            buffer: None,
            saved_state: None,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}
