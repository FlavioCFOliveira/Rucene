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

//! Tokenizes each input token into edge n-grams, ported from
//! `org.apache.lucene.analysis.ngram.EdgeNGramTokenFilter`.

use rucene::analysis::{
    CharTermAttribute, PositionIncrementAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeSource, CapturedState};

/// Equivalent to `EdgeNGramTokenFilter.DEFAULT_PRESERVE_ORIGINAL`.
pub const DEFAULT_PRESERVE_ORIGINAL: bool = false;

/// The body of [`EdgeNGramTokenFilter`]: what Lucene's
/// `EdgeNGramTokenFilter` adds to `TokenFilter`.
///
/// `rucene`'s term buffer holds one `char` per code point, so Java's
/// `Character.codePointCount` over the term is its length here, and
/// `Character.offsetByCodePoints` an index into it.
#[derive(Debug)]
pub struct EdgeNGramTokenFilterLogic {
    min_gram: i32,
    max_gram: i32,
    preserve_original: bool,
    cur_term_buffer: Option<Vec<char>>,
    cur_term_length: i32,
    cur_term_code_point_count: i32,
    cur_gram_size: i32,
    cur_pos_incr: i32,
    state: Option<CapturedState>,
}

fn set_position_increment(source: &AttributeSource, position_increment: i32) {
    source
        .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
        .set_position_increment(position_increment);
}

fn restore(source: &AttributeSource, state: &Option<CapturedState>) -> Result<()> {
    if let Some(state) = state {
        source.restore_state(state)?;
    }
    Ok(())
}

impl TokenFilterLogic for EdgeNGramTokenFilterLogic {
    /// Equivalent to the final `EdgeNGramTokenFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        loop {
            if self.cur_term_buffer.is_none() {
                if !input.increment_token()? {
                    return Ok(false);
                }
                self.state = source.capture_state();

                let term = {
                    let term_att = source
                        .get_attribute_interface::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute");
                    term_att.buffer()[..term_att.length()].to_vec()
                };
                self.cur_term_length = term.len() as i32;
                self.cur_term_code_point_count = term.len() as i32;
                self.cur_pos_incr += source
                    .get_attribute_interface::<dyn PositionIncrementAttribute>()
                    .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                    .get_position_increment();

                if self.preserve_original && self.cur_term_code_point_count < self.min_gram {
                    // Token is shorter than minGram, but we'd still like to keep it.
                    set_position_increment(source, self.cur_pos_incr);
                    self.cur_pos_incr = 0;
                    return Ok(true);
                }

                self.cur_term_buffer = Some(term);
                self.cur_gram_size = self.min_gram;
            }

            if self.cur_gram_size <= self.cur_term_code_point_count {
                if self.cur_gram_size <= self.max_gram {
                    // curGramSize is between minGram and maxGram
                    restore(source, &self.state)?;
                    // first ngram gets increment, others don't
                    set_position_increment(source, self.cur_pos_incr);
                    self.cur_pos_incr = 0;

                    let char_length = self.cur_gram_size as usize;
                    let buffer = self.cur_term_buffer.as_ref().expect("INVARIANT: set above");
                    source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute")
                        .copy_buffer(buffer, 0, char_length);
                    self.cur_gram_size += 1;
                    return Ok(true);
                } else if self.preserve_original {
                    // Token is longer than maxGram, but we'd still like to keep it.
                    restore(source, &self.state)?;
                    set_position_increment(source, 0);
                    let buffer = self.cur_term_buffer.take().expect("INVARIANT: set above");
                    source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute")
                        .copy_buffer(&buffer, 0, self.cur_term_length as usize);
                    return Ok(true);
                }
            }

            // Done with this input token, get next token on the next iteration.
            self.cur_term_buffer = None;
        }
    }

    /// Equivalent to `EdgeNGramTokenFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.cur_term_buffer = None;
        self.cur_pos_incr = 0;
        Ok(())
    }

    /// Equivalent to `EdgeNGramTokenFilter.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        set_position_increment(source, self.cur_pos_incr);
        Ok(())
    }
}

/// Tokenizes the given token into n-grams of given size(s).
///
/// This `TokenFilter` create n-grams from the beginning edge of a input
/// token.
///
/// As of Lucene 4.4, this filter handles correctly supplementary characters.
///
/// Equivalent to `org.apache.lucene.analysis.ngram.EdgeNGramTokenFilter`.
pub type EdgeNGramTokenFilter = TokenFilter<EdgeNGramTokenFilterLogic>;

/// Creates an EdgeNGramTokenFilter that, for a given input term, produces
/// all edge n-grams with lengths >= minGram and <= maxGram. Will optionally
/// preserve the original term when its length is outside of the defined
/// range.
///
/// * `input` `TokenStream` holding the input to be tokenized
/// * `min_gram` the minimum length of the generated n-grams
/// * `max_gram` the maximum length of the generated n-grams
/// * `preserve_original` Whether or not to keep the original term when it is
///   outside the min/max size range.
///
/// Equivalent to `EdgeNGramTokenFilter(TokenStream, int, int, boolean)`,
/// which adds the `CharTermAttribute` and the `PositionIncrementAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws, and `IllegalArgumentException`
/// "minGram must be greater than zero" or "minGram must not be greater than
/// maxGram".
pub fn new_edge_ngram_token_filter(
    input: Box<dyn TokenStream>,
    min_gram: i32,
    max_gram: i32,
    preserve_original: bool,
) -> Result<EdgeNGramTokenFilter> {
    let mut filter = TokenFilter::new(
        input,
        EdgeNGramTokenFilterLogic {
            min_gram,
            max_gram,
            preserve_original,
            cur_term_buffer: None,
            cur_term_length: 0,
            cur_term_code_point_count: 0,
            cur_gram_size: 0,
            cur_pos_incr: 0,
            state: None,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    if min_gram < 1 {
        return Err(LuceneError::illegal_argument(
            "minGram must be greater than zero",
        ));
    }
    if min_gram > max_gram {
        return Err(LuceneError::illegal_argument(
            "minGram must not be greater than maxGram",
        ));
    }
    Ok(filter)
}

/// Creates an EdgeNGramTokenFilter that produces edge n-grams of the given
/// size.
///
/// * `input` `TokenStream` holding the input to be tokenized
/// * `gram_size` the n-gram size to generate.
///
/// Equivalent to `EdgeNGramTokenFilter(TokenStream, int)`, `this(input,
/// gramSize, gramSize, DEFAULT_PRESERVE_ORIGINAL)`.
///
/// # Errors
///
/// As [`new_edge_ngram_token_filter`].
pub fn new_edge_ngram_token_filter_with_size(
    input: Box<dyn TokenStream>,
    gram_size: i32,
) -> Result<EdgeNGramTokenFilter> {
    new_edge_ngram_token_filter(input, gram_size, gram_size, DEFAULT_PRESERVE_ORIGINAL)
}
