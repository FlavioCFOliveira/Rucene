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

//! Emits the entire input as a single token, ported from
//! `org.apache.lucene.analysis.core.KeywordTokenizer`.

use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute, Tokenizer,
    TokenizerLogic, MAX_TOKEN_LENGTH_LIMIT,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};
use rucene::util::ArrayUtil;

/// Default read buffer size.
///
/// Equivalent to `KeywordTokenizer.DEFAULT_BUFFER_SIZE`.
pub const DEFAULT_BUFFER_SIZE: i32 = 256;

/// The length of the `char[]` Java's `CharTermAttributeImpl` starts with,
/// `ArrayUtil.oversize(MIN_BUFFER_SIZE, Character.BYTES)` with
/// `MIN_BUFFER_SIZE = 10`.
fn initial_term_buffer_length() -> usize {
    ArrayUtil::oversize(10, 2)
}

/// The body of [`KeywordTokenizer`]: what Lucene's `KeywordTokenizer` adds
/// to `Tokenizer`.
///
/// **Divergence from Lucene 10.5.0.** Java reads the input straight into the
/// term's `char[]`, growing it when full (`termAtt.resizeBuffer(1 +
/// buffer.length)`), so each `read` asks for the room left in that array.
/// `rucene`'s term buffer holds `char`s, not UTF-16 units, so the units are
/// read into a buffer of this logic that grows exactly as Java's term array
/// does (from `resizeBuffer(bufferSize)`, with `ArrayUtil.oversize(n,
/// Character.BYTES)`), and decoded into the term once the input is
/// exhausted: the reader sees the same requests as Java's while no other
/// component resizes the term between tokens. A lone surrogate of the input
/// is decoded to U+FFFD, the limit `rucene`'s `CharTermAttribute` declares.
#[derive(Debug)]
pub struct KeywordTokenizerLogic {
    done: bool,
    final_offset: i32,
    /// The UTF-16 units read, standing for Java's term `char[]`.
    buffer: Vec<u16>,
    /// The token decoded into `char`s.
    text: Vec<char>,
}

impl KeywordTokenizerLogic {
    /// The fields of a `KeywordTokenizer`, whose term buffer was resized to
    /// `buffer_size` (`termAtt.resizeBuffer(bufferSize)`).
    fn new(buffer_size: i32) -> Self {
        let mut length = initial_term_buffer_length();
        if length < buffer_size as usize {
            length = ArrayUtil::oversize(buffer_size as usize, 2);
        }
        Self {
            done: false,
            final_offset: 0,
            buffer: vec![0; length],
            text: Vec::new(),
        }
    }
}

impl TokenizerLogic for KeywordTokenizerLogic {
    /// Equivalent to the final `KeywordTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        if !self.done {
            source.clear_attributes();
            self.done = true;
            let mut upto = 0;
            loop {
                let length = input.read(&mut self.buffer[upto..])?;
                if length == 0 {
                    // `length == -1`: a `CharReader` answers 0 at the end of
                    // the stream, and the request is never empty here.
                    break;
                }
                upto += length;
                if upto == self.buffer.len() {
                    // `termAtt.resizeBuffer(1 + buffer.length)`
                    let new_length = ArrayUtil::oversize(1 + self.buffer.len(), 2);
                    self.buffer.resize(new_length, 0);
                }
            }
            self.text.clear();
            self.text.extend(
                char::decode_utf16(self.buffer[..upto].iter().copied())
                    .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)),
            );
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute")
                .copy_buffer(&self.text, 0, self.text.len());
            self.final_offset = Tokenizer::<Self>::correct_offset_of(Some(&*input), upto as i32);
            let start = Tokenizer::<Self>::correct_offset_of(Some(&*input), 0);
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .set_offset(start, self.final_offset);
            return Ok(true);
        }
        Ok(false)
    }

    /// Equivalent to the final `KeywordTokenizer.end()`; `Tokenizer::end`
    /// has run `super.end()`. It sets the final offset.
    fn end(&mut self, source: &mut AttributeSource, _input: Option<&dyn CharReader>) -> Result<()> {
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    /// Equivalent to `KeywordTokenizer.reset()`; `Tokenizer::reset` has run
    /// `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.done = false;
        Ok(())
    }
}

/// Emits the entire input as a single token.
///
/// Equivalent to `org.apache.lucene.analysis.core.KeywordTokenizer`.
pub type KeywordTokenizer = Tokenizer<KeywordTokenizerLogic>;

/// Equivalent to `KeywordTokenizer()`, `this(DEFAULT_BUFFER_SIZE)`.
pub fn new_keyword_tokenizer() -> KeywordTokenizer {
    new_keyword_tokenizer_with_buffer_size(DEFAULT_BUFFER_SIZE)
        .expect("INVARIANT: DEFAULT_BUFFER_SIZE is a valid buffer size")
}

/// Equivalent to `KeywordTokenizer(int)`, whose attributes come from
/// `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`.
///
/// # Errors
///
/// As [`new_keyword_tokenizer_with_factory`].
pub fn new_keyword_tokenizer_with_buffer_size(buffer_size: i32) -> Result<KeywordTokenizer> {
    new_keyword_tokenizer_with_factory(default_token_attribute_factory(), buffer_size)
}

/// Equivalent to `KeywordTokenizer(AttributeFactory, int)`: the fields add
/// the `CharTermAttribute` and the `OffsetAttribute`, in that order, and
/// the term buffer is resized to `buffer_size`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws, and `IllegalArgumentException` with Java's
/// message when `buffer_size` is not greater than 0 or exceeds
/// `MAX_TOKEN_LENGTH_LIMIT`.
pub fn new_keyword_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
    buffer_size: i32,
) -> Result<KeywordTokenizer> {
    let (source, logic) = keyword_tokenizer_parts(factory, buffer_size)?;
    Ok(Tokenizer::with_source(source, logic))
}

/// The source and the body of the `KeywordTokenizer` that
/// [`new_keyword_tokenizer_with_factory`] builds, for a caller that holds the
/// body behind a box (`KeywordTokenizerFactory`).
///
/// # Errors
///
/// As [`new_keyword_tokenizer_with_factory`].
pub(crate) fn keyword_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
    buffer_size: i32,
) -> Result<(AttributeSource, KeywordTokenizerLogic)> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    if buffer_size as i64 > MAX_TOKEN_LENGTH_LIMIT as i64 || buffer_size <= 0 {
        return Err(LuceneError::illegal_argument(format!(
            "maxTokenLen must be greater than 0 and less than {MAX_TOKEN_LENGTH_LIMIT} passed: {buffer_size}"
        )));
    }
    source
        .get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("INVARIANT: added above")
        .resize_buffer(buffer_size as usize);
    Ok((source, KeywordTokenizerLogic::new(buffer_size)))
}
