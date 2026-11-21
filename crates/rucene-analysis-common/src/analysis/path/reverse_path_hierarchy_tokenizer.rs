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

//! Tokenizes a domain-like name into its hierarchy of suffixes, ported from
//! `org.apache.lucene.analysis.path.ReversePathHierarchyTokenizer`.

use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute,
    PositionIncrementAttribute, Tokenizer, TokenizerLogic,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};

use super::path_hierarchy_tokenizer::read_unit;
use crate::jdk::character::utf16_to_term_chars;

/// Equivalent to the private `ReversePathHierarchyTokenizer.DEFAULT_BUFFER_SIZE`.
const DEFAULT_BUFFER_SIZE: i32 = 1024;

/// Equivalent to `ReversePathHierarchyTokenizer.DEFAULT_DELIMITER`.
pub const DEFAULT_DELIMITER: u16 = b'/' as u16;

/// Equivalent to `ReversePathHierarchyTokenizer.DEFAULT_SKIP`.
pub const DEFAULT_SKIP: i32 = 0;

/// The body of [`ReversePathHierarchyTokenizer`]: what Lucene's
/// `ReversePathHierarchyTokenizer` adds to `Tokenizer`.
///
/// The input is held as UTF-16 units (Java's `resultTokenBuffer`); a token
/// copied into the term has its unpaired surrogates turned into U+FFFD
/// (`jdk::character::utf16_to_term_chars`). Positions and offsets count
/// UTF-16 units, as Java's do.
#[derive(Debug)]
pub struct ReversePathHierarchyTokenizerLogic {
    delimiter: u16,
    replacement: u16,
    skip: i32,
    end_position: i32,
    final_offset: i32,
    skipped: i32,
    result_token: Vec<u16>,
    delimiter_positions: Vec<i32>,
    delimiters_count: i32,
    result_token_buffer: Vec<u16>,
}

impl TokenizerLogic for ReversePathHierarchyTokenizerLogic {
    /// Equivalent to the final `ReversePathHierarchyTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        if self.delimiters_count == -1 {
            let mut length = 0;
            self.delimiter_positions.push(0);
            loop {
                let c = read_unit(input)?;
                if c < 0 {
                    break;
                }
                length += 1;
                let c = c as u16;
                if c == self.delimiter {
                    self.delimiter_positions.push(length);
                    self.result_token.push(self.replacement);
                } else {
                    self.result_token.push(c);
                }
            }
            self.delimiters_count = self.delimiter_positions.len() as i32;
            if self.delimiter_positions[self.delimiters_count as usize - 1] < length {
                self.delimiter_positions.push(length);
                self.delimiters_count += 1;
            }
            if self.result_token_buffer.len() < self.result_token.len() {
                self.result_token_buffer = vec![0; self.result_token.len()];
            }
            self.result_token_buffer[..self.result_token.len()].copy_from_slice(&self.result_token);
            self.result_token.clear();
            let idx = self.delimiters_count - 1 - self.skip;
            if idx >= 0 {
                // otherwise it's ok, because we will skip and return false
                self.end_position = self.delimiter_positions[idx as usize];
            }
            self.final_offset = Tokenizer::<Self>::correct_offset_of(Some(&*input), length);
        }
        source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .set_position_increment(1);

        if self.skipped < self.delimiters_count - self.skip - 1 {
            let start = self.delimiter_positions[self.skipped as usize];
            let chars = utf16_to_term_chars(
                &self.result_token_buffer[start as usize..self.end_position as usize],
            );
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute")
                .copy_buffer(&chars, 0, chars.len());
            let start_offset = Tokenizer::<Self>::correct_offset_of(Some(&*input), start);
            let end_offset = Tokenizer::<Self>::correct_offset_of(Some(&*input), self.end_position);
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .set_offset(start_offset, end_offset);
            self.skipped += 1;
            return Ok(true);
        }
        Ok(false)
    }

    /// Equivalent to the final `ReversePathHierarchyTokenizer.end()`;
    /// `Tokenizer::end` has run `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, _input: Option<&dyn CharReader>) -> Result<()> {
        // set final offset
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    /// Equivalent to `ReversePathHierarchyTokenizer.reset()`;
    /// `Tokenizer::reset` has run `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.result_token.clear();
        self.final_offset = 0;
        self.end_position = 0;
        self.skipped = 0;
        self.delimiters_count = -1;
        self.delimiter_positions.clear();
        Ok(())
    }
}

/// Tokenizer for domain-like hierarchies.
///
/// Take something like:
///
/// ```text
/// www.site.co.uk
/// ```
///
/// and make:
///
/// ```text
/// www.site.co.uk
/// site.co.uk
/// co.uk
/// uk
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.path.ReversePathHierarchyTokenizer`.
pub type ReversePathHierarchyTokenizer = Tokenizer<ReversePathHierarchyTokenizerLogic>;

/// Equivalent to `ReversePathHierarchyTokenizer()`.
pub fn new_reverse_path_hierarchy_tokenizer() -> ReversePathHierarchyTokenizer {
    new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )
    .expect("INVARIANT: the defaults are valid")
}

/// Equivalent to `ReversePathHierarchyTokenizer(int skip)`.
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_reverse_path_hierarchy_tokenizer_skip(
    skip: i32,
) -> Result<ReversePathHierarchyTokenizer> {
    new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        skip,
    )
}

/// Equivalent to `ReversePathHierarchyTokenizer(int bufferSize, char
/// delimiter)`.
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_reverse_path_hierarchy_tokenizer_buffer_size(
    buffer_size: i32,
    delimiter: u16,
) -> Result<ReversePathHierarchyTokenizer> {
    new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        buffer_size,
        delimiter,
        delimiter,
        DEFAULT_SKIP,
    )
}

/// Equivalent to `ReversePathHierarchyTokenizer(char delimiter, char
/// replacement)`.
pub fn new_reverse_path_hierarchy_tokenizer_replacement(
    delimiter: u16,
    replacement: u16,
) -> ReversePathHierarchyTokenizer {
    new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        delimiter,
        replacement,
        DEFAULT_SKIP,
    )
    .expect("INVARIANT: the default buffer size and skip are valid")
}

/// Equivalent to `ReversePathHierarchyTokenizer(int bufferSize, char
/// delimiter, char replacement)`.
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_reverse_path_hierarchy_tokenizer_buffer_size_replacement(
    buffer_size: i32,
    delimiter: u16,
    replacement: u16,
) -> Result<ReversePathHierarchyTokenizer> {
    new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        buffer_size,
        delimiter,
        replacement,
        DEFAULT_SKIP,
    )
}

/// Equivalent to `ReversePathHierarchyTokenizer(char delimiter, int skip)`.
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_reverse_path_hierarchy_tokenizer_delimiter_skip(
    delimiter: u16,
    skip: i32,
) -> Result<ReversePathHierarchyTokenizer> {
    new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        delimiter,
        delimiter,
        skip,
    )
}

/// Equivalent to `ReversePathHierarchyTokenizer(char delimiter, char
/// replacement, int skip)`.
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_reverse_path_hierarchy_tokenizer_replacement_skip(
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<ReversePathHierarchyTokenizer> {
    new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        delimiter,
        replacement,
        skip,
    )
}

/// Equivalent to `ReversePathHierarchyTokenizer(AttributeFactory factory,
/// char delimiter, char replacement, int skip)`.
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_reverse_path_hierarchy_tokenizer_factory(
    factory: Arc<dyn AttributeFactory>,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<ReversePathHierarchyTokenizer> {
    new_reverse_path_hierarchy_tokenizer_factory_buffer_size(
        factory,
        DEFAULT_BUFFER_SIZE,
        delimiter,
        replacement,
        skip,
    )
}

/// Equivalent to `ReversePathHierarchyTokenizer(int bufferSize, char
/// delimiter, char replacement, int skip)`.
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_reverse_path_hierarchy_tokenizer_buffer_size_replacement_skip(
    buffer_size: i32,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<ReversePathHierarchyTokenizer> {
    new_reverse_path_hierarchy_tokenizer_factory_buffer_size(
        default_token_attribute_factory(),
        buffer_size,
        delimiter,
        replacement,
        skip,
    )
}

/// Equivalent to `ReversePathHierarchyTokenizer(AttributeFactory factory,
/// int bufferSize, char delimiter, char replacement, int skip)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws, and `IllegalArgumentException` "bufferSize cannot
/// be negative" or "skip cannot be negative".
pub fn new_reverse_path_hierarchy_tokenizer_factory_buffer_size(
    factory: Arc<dyn AttributeFactory>,
    buffer_size: i32,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<ReversePathHierarchyTokenizer> {
    let (source, logic) =
        reverse_path_hierarchy_tokenizer_parts(factory, buffer_size, delimiter, replacement, skip)?;
    Ok(Tokenizer::with_source(source, logic))
}

/// The source and the body of the tokenizer that
/// [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`] builds, for a
/// caller that holds the body behind a box (`PathHierarchyTokenizerFactory`).
///
/// # Errors
///
/// As [`new_reverse_path_hierarchy_tokenizer_factory_buffer_size`].
pub(crate) fn reverse_path_hierarchy_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
    buffer_size: i32,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<(AttributeSource, ReversePathHierarchyTokenizerLogic)> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    if buffer_size < 0 {
        return Err(LuceneError::illegal_argument(
            "bufferSize cannot be negative",
        ));
    }
    if skip < 0 {
        return Err(LuceneError::illegal_argument("skip cannot be negative"));
    }
    source
        .get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("INVARIANT: added above")
        .resize_buffer(buffer_size as usize);
    Ok((
        source,
        ReversePathHierarchyTokenizerLogic {
            delimiter,
            replacement,
            skip,
            end_position: 0,
            final_offset: 0,
            skipped: 0,
            result_token: Vec::with_capacity(buffer_size as usize),
            delimiter_positions: Vec::with_capacity((buffer_size / 10) as usize),
            delimiters_count: -1,
            result_token_buffer: vec![0; buffer_size as usize],
        },
    ))
}
