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

//! Tokenizes the input into n-grams of the given size(s), ported from
//! `org.apache.lucene.analysis.ngram.NGramTokenizer`.

use std::fmt;
use std::sync::Arc;

use rucene::analysis::character_utils::{self, CharacterBuffer};
use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute,
    PositionIncrementAttribute, PositionLengthAttribute, Tokenizer, TokenizerLogic,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};

use crate::jdk::character::{char_count, to_term_char};

/// Equivalent to `NGramTokenizer.DEFAULT_MIN_NGRAM_SIZE`.
pub const DEFAULT_MIN_NGRAM_SIZE: i32 = 1;

/// Equivalent to `NGramTokenizer.DEFAULT_MAX_NGRAM_SIZE`.
pub const DEFAULT_MAX_NGRAM_SIZE: i32 = 2;

/// The predicate a subclass overrides as `isTokenChar(int)`.
pub type IsTokenChar = Box<dyn Fn(i32) -> bool + Send + Sync>;

/// The body of [`NGramTokenizer`]: what Lucene's `NGramTokenizer` adds to
/// `Tokenizer`.
///
/// Java's class is not final "to allow for overriding isTokenChar"; the
/// override is the [`IsTokenChar`] predicate this logic holds
/// ([`new_ngram_tokenizer_with_token_char`]), `true` for every character by
/// default. The n-gram is written to the term as `char`s, its length counted
/// in UTF-16 units as Java's `CharacterUtils.toChars` counts it.
pub struct NGramTokenizerLogic {
    char_buffer: CharacterBuffer,
    /// like charBuffer, but converted to code points
    buffer: Vec<i32>,
    // remaining slice in buffer
    buffer_start: i32,
    buffer_end: i32,
    offset: i32,
    gram_size: i32,
    min_gram: i32,
    max_gram: i32,
    exhausted: bool,
    /// last offset in the buffer that we checked
    last_checked_char: i32,
    /// last offset that we found to not be a token char
    last_non_token_char: i32,
    /// leading edges n-grams only
    edges_only: bool,
    is_token_char: IsTokenChar,
}

impl fmt::Debug for NGramTokenizerLogic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NGramTokenizerLogic")
            .field("buffer_start", &self.buffer_start)
            .field("buffer_end", &self.buffer_end)
            .field("offset", &self.offset)
            .field("gram_size", &self.gram_size)
            .field("min_gram", &self.min_gram)
            .field("max_gram", &self.max_gram)
            .field("exhausted", &self.exhausted)
            .field("edges_only", &self.edges_only)
            .finish_non_exhaustive()
    }
}

impl NGramTokenizerLogic {
    /// Equivalent to the private `init(int, int, boolean)`, but for the term
    /// buffer's resize, which the caller performs on the source.
    fn init(
        min_gram: i32,
        max_gram: i32,
        edges_only: bool,
        is_token_char: IsTokenChar,
    ) -> Result<Self> {
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
        // 2 * maxGram in case all code points require 2 chars and + 1024 for
        // buffering to not keep polling the Reader
        // (Java's `int` arithmetic: a size that overflows is refused as below 2.)
        let buffer_size = max_gram.wrapping_mul(2).wrapping_add(1024);
        let char_buffer = character_utils::new_character_buffer(buffer_size.max(0) as usize)?;
        let buffer = vec![0; char_buffer.buffer().len()];
        Ok(Self {
            char_buffer,
            buffer,
            buffer_start: 0,
            buffer_end: 0,
            offset: 0,
            gram_size: 0,
            min_gram,
            max_gram,
            exhausted: false,
            last_checked_char: 0,
            last_non_token_char: 0,
            edges_only,
            is_token_char,
        })
    }

    /// Equivalent to the private `updateLastNonTokenChar()`.
    fn update_last_non_token_char(&mut self) {
        let term_end = self.buffer_start + self.gram_size - 1;
        if term_end > self.last_checked_char {
            let mut i = term_end;
            while i > self.last_checked_char {
                if !self.is_token_char(self.buffer[i as usize]) {
                    self.last_non_token_char = i;
                    break;
                }
                i -= 1;
            }
            self.last_checked_char = term_end;
        }
    }

    /// Consume one code point.
    ///
    /// Equivalent to the private `consume()`.
    fn consume(&mut self) {
        self.offset += char_count(self.buffer[self.buffer_start as usize]) as i32;
        self.buffer_start += 1;
    }

    /// Only collect characters which satisfy this condition.
    ///
    /// Equivalent to the protected `NGramTokenizer.isTokenChar(int)`.
    pub fn is_token_char(&self, chr: i32) -> bool {
        (self.is_token_char)(chr)
    }
}

impl TokenizerLogic for NGramTokenizerLogic {
    /// Equivalent to the final `NGramTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();

        // termination of this loop is guaranteed by the fact that every iteration
        // either advances the buffer (calls consumes()) or increases gramSize
        loop {
            // compact
            if self.buffer_start >= self.buffer_end - self.max_gram - 1 && !self.exhausted {
                let (start, end) = (self.buffer_start as usize, self.buffer_end as usize);
                self.buffer.copy_within(start..end, 0);
                self.buffer_end -= self.buffer_start;
                self.last_checked_char -= self.buffer_start;
                self.last_non_token_char -= self.buffer_start;
                self.buffer_start = 0;

                // fill in remaining space
                let num_chars = self.buffer.len() - self.buffer_end as usize;
                self.exhausted = !character_utils::fill(&mut self.char_buffer, input, num_chars)?;
                // convert to code points
                self.buffer_end += character_utils::to_code_points(
                    self.char_buffer.buffer(),
                    0,
                    self.char_buffer.length(),
                    &mut self.buffer,
                    self.buffer_end as usize,
                )? as i32;
            }

            // should we go to the next offset?
            if self.gram_size > self.max_gram
                || (self.buffer_start + self.gram_size) > self.buffer_end
            {
                if self.buffer_start + 1 + self.min_gram > self.buffer_end {
                    debug_assert!(self.exhausted);
                    return Ok(false);
                }
                self.consume();
                self.gram_size = self.min_gram;
            }

            self.update_last_non_token_char();

            // retry if the token to be emitted was going to not only contain token chars
            let term_contains_non_token_char = self.last_non_token_char >= self.buffer_start
                && self.last_non_token_char < (self.buffer_start + self.gram_size);
            let is_edge_and_previous_char_is_token_char =
                self.edges_only && self.last_non_token_char != self.buffer_start - 1;
            if term_contains_non_token_char || is_edge_and_previous_char_is_token_char {
                self.consume();
                self.gram_size = self.min_gram;
                continue;
            }

            // `CharacterUtils.toChars(buffer, bufferStart, gramSize, termAtt.buffer(), 0)`
            let gram = &self.buffer
                [self.buffer_start as usize..(self.buffer_start + self.gram_size) as usize];
            let length: i32 = gram.iter().map(|&cp| char_count(cp) as i32).sum();
            let chars: Vec<char> = gram.iter().map(|&cp| to_term_char(cp)).collect();
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute")
                .copy_buffer(&chars, 0, chars.len());
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(1);
            source
                .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                .expect("INVARIANT: the constructor added the PositionLengthAttribute")
                .set_position_length(1);
            let start = Tokenizer::<Self>::correct_offset_of(Some(&*input), self.offset);
            let end = Tokenizer::<Self>::correct_offset_of(Some(&*input), self.offset + length);
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .set_offset(start, end);
            self.gram_size += 1;
            return Ok(true);
        }
    }

    /// Equivalent to the final `NGramTokenizer.end()`; `Tokenizer::end` has
    /// run `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        debug_assert!(self.buffer_start <= self.buffer_end);
        let mut end_offset = self.offset;
        for i in self.buffer_start..self.buffer_end {
            end_offset += char_count(self.buffer[i as usize]) as i32;
        }
        end_offset = Tokenizer::<Self>::correct_offset_of(input, end_offset);
        // set final offset
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(end_offset, end_offset);
        Ok(())
    }

    /// Equivalent to the final `NGramTokenizer.reset()`; `Tokenizer::reset`
    /// has run `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.buffer_start = self.buffer.len() as i32;
        self.buffer_end = self.buffer_start;
        self.last_checked_char = self.buffer_start - 1;
        self.last_non_token_char = self.last_checked_char;
        self.offset = 0;
        self.gram_size = self.min_gram;
        self.exhausted = false;
        self.char_buffer.reset();
        Ok(())
    }
}

/// Tokenizes the input into n-grams of the given size(s).
///
/// On the contrary to `NGramTokenFilter`, this class sets offsets so that
/// characters between startOffset and endOffset in the original stream are
/// the same as the term chars.
///
/// For example, "abcde" would be tokenized as (minGram=2, maxGram=3):
///
/// | Term | ab | abc | bc | bcd | cd | cde | de |
/// |---|---|---|---|---|---|---|---|
/// | Position increment | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
/// | Position length | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
/// | Offsets | [0,2[ | [0,3[ | [1,3[ | [1,4[ | [2,4[ | [2,5[ | [3,5[ |
///
/// This tokenizer changed a lot in Lucene 4.4 in order to:
///
/// * tokenize in a streaming fashion to support streams which are larger than
///   1024 chars (limit of the previous version),
/// * count grams based on unicode code points instead of java chars (and
///   never split in the middle of surrogate pairs),
/// * give the ability to pre-tokenize (`isTokenChar(int)`) the stream before
///   computing n-grams.
///
/// Additionally, this class doesn't trim trailing whitespaces and emits
/// tokens in a different order, tokens are now emitted by increasing start
/// offsets while they used to be emitted by increasing lengths (which
/// prevented from supporting large input streams).
///
/// Equivalent to `org.apache.lucene.analysis.ngram.NGramTokenizer`.
pub type NGramTokenizer = Tokenizer<NGramTokenizerLogic>;

/// The predicate of a tokenizer that does not override `isTokenChar`.
fn all_token_chars() -> IsTokenChar {
    Box::new(|_| true)
}

/// Creates NGramTokenizer with given min and max n-grams.
///
/// Equivalent to `NGramTokenizer(int, int)`.
///
/// # Errors
///
/// As [`new_ngram_tokenizer_with_token_char`].
pub fn new_ngram_tokenizer(min_gram: i32, max_gram: i32) -> Result<NGramTokenizer> {
    new_ngram_tokenizer_with_factory(default_token_attribute_factory(), min_gram, max_gram)
}

/// Creates NGramTokenizer with given min and max n-grams.
///
/// Equivalent to `NGramTokenizer(AttributeFactory, int, int)`.
///
/// # Errors
///
/// As [`new_ngram_tokenizer_with_token_char`].
pub fn new_ngram_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
    min_gram: i32,
    max_gram: i32,
) -> Result<NGramTokenizer> {
    new_ngram_tokenizer_with_token_char(factory, min_gram, max_gram, false, all_token_chars())
}

/// Creates NGramTokenizer with default min and max n-grams.
///
/// Equivalent to `NGramTokenizer()`.
pub fn new_ngram_tokenizer_default() -> NGramTokenizer {
    new_ngram_tokenizer(DEFAULT_MIN_NGRAM_SIZE, DEFAULT_MAX_NGRAM_SIZE)
        .expect("INVARIANT: the default sizes are valid")
}

/// The constructor of a subclass that overrides `isTokenChar(int)` with
/// `is_token_char`: the package-private `NGramTokenizer(AttributeFactory,
/// int, int, boolean)` with `edges_only` (`EdgeNGramTokenizer` passes
/// `true`), followed by the override.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws, and `IllegalArgumentException` "minGram must be
/// greater than zero" or "minGram must not be greater than maxGram".
pub fn new_ngram_tokenizer_with_token_char(
    factory: Arc<dyn AttributeFactory>,
    min_gram: i32,
    max_gram: i32,
    edges_only: bool,
    is_token_char: IsTokenChar,
) -> Result<NGramTokenizer> {
    let (source, logic) =
        ngram_tokenizer_parts(factory, min_gram, max_gram, edges_only, is_token_char)?;
    Ok(Tokenizer::with_source(source, logic))
}

/// The source and the body of the tokenizer that
/// [`new_ngram_tokenizer_with_token_char`] builds, for a caller that holds
/// the body behind a box (`NGramTokenizerFactory`, `EdgeNGramTokenizerFactory`).
///
/// # Errors
///
/// As [`new_ngram_tokenizer_with_token_char`].
pub(crate) fn ngram_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
    min_gram: i32,
    max_gram: i32,
    edges_only: bool,
    is_token_char: IsTokenChar,
) -> Result<(AttributeSource, NGramTokenizerLogic)> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    let logic = NGramTokenizerLogic::init(min_gram, max_gram, edges_only, is_token_char)?;
    // Make the term att large enough (`resizeBuffer` of a negative size,
    // which Java's `int` arithmetic can produce, leaves the buffer as it is)
    let term_size = max_gram.wrapping_mul(2);
    if term_size > 0 {
        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: added above")
            .resize_buffer(term_size as usize);
    }
    Ok((source, logic))
}
