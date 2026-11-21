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

//! Tokenizes each input token into n-grams, ported from
//! `org.apache.lucene.analysis.ngram.NGramTokenFilter`.

use rucene::analysis::{
    CharTermAttribute, PositionIncrementAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeSource, CapturedState};

/// Equivalent to `NGramTokenFilter.DEFAULT_PRESERVE_ORIGINAL`.
pub const DEFAULT_PRESERVE_ORIGINAL: bool = false;

/// The body of [`NGramTokenFilter`]: what Lucene's `NGramTokenFilter` adds
/// to `TokenFilter`.
///
/// `rucene`'s term buffer holds one `char` per code point, so Java's
/// `Character.codePointCount` over the term is its length here, and
/// `Character.offsetByCodePoints` an index into it.
#[derive(Debug)]
pub struct NGramTokenFilterLogic {
    min_gram: i32,
    max_gram: i32,
    preserve_original: bool,
    cur_term_buffer: Option<Vec<char>>,
    cur_term_length: i32,
    cur_term_code_point_count: i32,
    cur_gram_size: i32,
    cur_pos: i32,
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

impl TokenFilterLogic for NGramTokenFilterLogic {
    /// Equivalent to the final `NGramTokenFilter.incrementToken()`.
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
                self.cur_pos = 0;

                if self.preserve_original && self.cur_term_code_point_count < self.min_gram {
                    // Token is shorter than minGram, but we'd still like to keep it.
                    set_position_increment(source, self.cur_pos_incr);
                    self.cur_pos_incr = 0;
                    return Ok(true);
                }

                self.cur_term_buffer = Some(term);
                self.cur_gram_size = self.min_gram;
            }

            if self.cur_gram_size > self.max_gram
                || (self.cur_pos + self.cur_gram_size) > self.cur_term_code_point_count
            {
                self.cur_pos += 1;
                self.cur_gram_size = self.min_gram;
            }
            if (self.cur_pos + self.cur_gram_size) <= self.cur_term_code_point_count {
                restore(source, &self.state)?;
                let start = self.cur_pos as usize;
                let end = start + self.cur_gram_size as usize;
                let buffer = self.cur_term_buffer.as_ref().expect("INVARIANT: set above");
                source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute")
                    .copy_buffer(buffer, start, end - start);
                set_position_increment(source, self.cur_pos_incr);
                self.cur_pos_incr = 0;
                self.cur_gram_size += 1;
                return Ok(true);
            } else if self.preserve_original && self.cur_term_code_point_count > self.max_gram {
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

            // Done with this input token, get next token on next iteration.
            self.cur_term_buffer = None;
        }
    }

    /// Equivalent to `NGramTokenFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.cur_term_buffer = None;
        self.cur_pos_incr = 0;
        Ok(())
    }

    /// Equivalent to `NGramTokenFilter.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        set_position_increment(source, self.cur_pos_incr);
        Ok(())
    }
}

/// Tokenizes the input into n-grams of the given size(s). As of Lucene 4.4,
/// this token filter:
///
/// * handles supplementary characters correctly,
/// * emits all n-grams for the same token at the same position,
/// * does not modify offsets,
/// * sorts n-grams by their offset in the original token first, then
///   increasing length (meaning that "abc" will give "a", "ab", "abc", "b",
///   "bc", "c").
///
/// If you were using this `TokenFilter` to perform partial highlighting,
/// this won't work anymore since this filter doesn't update offsets. You
/// should modify your analysis chain to use `NGramTokenizer`, and
/// potentially override `NGramTokenizer#isTokenChar(int)` to perform
/// pre-tokenization.
///
/// Equivalent to `org.apache.lucene.analysis.ngram.NGramTokenFilter`.
pub type NGramTokenFilter = TokenFilter<NGramTokenFilterLogic>;

/// Creates an NGramTokenFilter that, for a given input term, produces all
/// contained n-grams with lengths >= minGram and <= maxGram. Will optionally
/// preserve the original term when its length is outside of the defined
/// range.
///
/// Note: Care must be taken when choosing minGram and maxGram; depending on
/// the input token size, this filter potentially produces a huge number of
/// terms.
///
/// * `input` `TokenStream` holding the input to be tokenized
/// * `min_gram` the minimum length of the generated n-grams
/// * `max_gram` the maximum length of the generated n-grams
/// * `preserve_original` Whether or not to keep the original term when it is
///   shorter than minGram or longer than maxGram
///
/// Equivalent to `NGramTokenFilter(TokenStream, int, int, boolean)`, which
/// adds the `CharTermAttribute` and the `PositionIncrementAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws, and `IllegalArgumentException`
/// "minGram must be greater than zero" or "minGram must not be greater than
/// maxGram".
pub fn new_ngram_token_filter(
    input: Box<dyn TokenStream>,
    min_gram: i32,
    max_gram: i32,
    preserve_original: bool,
) -> Result<NGramTokenFilter> {
    let mut filter = TokenFilter::new(
        input,
        NGramTokenFilterLogic {
            min_gram,
            max_gram,
            preserve_original,
            cur_term_buffer: None,
            cur_term_length: 0,
            cur_term_code_point_count: 0,
            cur_gram_size: 0,
            cur_pos: 0,
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

/// Creates an NGramTokenFilter that produces n-grams of the indicated size.
///
/// * `input` `TokenStream` holding the input to be tokenized
/// * `gram_size` the size of n-grams to generate.
///
/// Equivalent to `NGramTokenFilter(TokenStream, int)`, `this(input,
/// gramSize, gramSize, DEFAULT_PRESERVE_ORIGINAL)`.
///
/// # Errors
///
/// As [`new_ngram_token_filter`].
pub fn new_ngram_token_filter_with_size(
    input: Box<dyn TokenStream>,
    gram_size: i32,
) -> Result<NGramTokenFilter> {
    new_ngram_token_filter(input, gram_size, gram_size, DEFAULT_PRESERVE_ORIGINAL)
}
