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

//! Tokenizes a path into its hierarchy of prefixes, ported from
//! `org.apache.lucene.analysis.path.PathHierarchyTokenizer`.

use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute,
    PositionIncrementAttribute, Tokenizer, TokenizerLogic,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};

use crate::jdk::character::utf16_to_term_chars;

/// Equivalent to the private `PathHierarchyTokenizer.DEFAULT_BUFFER_SIZE`.
const DEFAULT_BUFFER_SIZE: i32 = 1024;

/// Equivalent to `PathHierarchyTokenizer.DEFAULT_DELIMITER`.
pub const DEFAULT_DELIMITER: u16 = b'/' as u16;

/// Equivalent to `PathHierarchyTokenizer.DEFAULT_SKIP`.
pub const DEFAULT_SKIP: i32 = 0;

/// `Reader.read()`: one UTF-16 unit, `-1` at the end of the input.
pub(crate) fn read_unit(input: &mut dyn CharReader) -> Result<i32> {
    let mut unit = [0u16; 1];
    if input.read(&mut unit)? == 0 {
        Ok(-1)
    } else {
        Ok(i32::from(unit[0]))
    }
}

/// The body of [`PathHierarchyTokenizer`]: what Lucene's
/// `PathHierarchyTokenizer` adds to `Tokenizer`.
///
/// **Divergence from Lucene 10.5.0.** Java appends the units it reads to the
/// term's `char[]` and keeps the previous token in a `StringBuilder`;
/// `rucene`'s term buffer holds `char`s, so the token is built as UTF-16
/// units here (`term`, `result_token`) and decoded into the term when it is
/// emitted, an unpaired surrogate becoming U+FFFD
/// (`jdk::character::utf16_to_term_chars`). Lengths and offsets count UTF-16
/// units, as Java's do.
#[derive(Debug)]
pub struct PathHierarchyTokenizerLogic {
    delimiter: u16,
    replacement: u16,
    skip: i32,
    start_position: i32,
    skipped: i32,
    end_delimiter: bool,
    result_token: Vec<u16>,
    chars_read: i32,
    /// The units of Java's term buffer.
    term: Vec<u16>,
}

impl PathHierarchyTokenizerLogic {
    /// Sets the term and offsets of the token, `term[..length]`, at
    /// `start_position`.
    fn emit(&self, source: &AttributeSource, input: &dyn CharReader, length: i32) {
        let chars = utf16_to_term_chars(&self.term[..length as usize]);
        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute")
            .copy_buffer(&chars, 0, chars.len());
        let start = Tokenizer::<Self>::correct_offset_of(Some(input), self.start_position);
        let end = Tokenizer::<Self>::correct_offset_of(Some(input), self.start_position + length);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(start, end);
    }
}

impl TokenizerLogic for PathHierarchyTokenizerLogic {
    /// Equivalent to the final `PathHierarchyTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        // `termAtt.append(resultToken)`
        self.term.clear();
        self.term.extend_from_slice(&self.result_token);
        source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .set_position_increment(1);
        let mut length: i32 = 0;
        let mut added = false;
        if self.end_delimiter {
            self.term.push(self.replacement);
            length += 1;
            self.end_delimiter = false;
            added = true;
        }

        loop {
            let c = read_unit(input)?;
            if c >= 0 {
                self.chars_read += 1;
            } else {
                if self.skipped > self.skip {
                    length += self.result_token.len() as i32;
                    self.term.truncate(length as usize);
                    self.emit(source, &*input, length);
                    if added {
                        self.result_token.clear();
                        self.result_token
                            .extend_from_slice(&self.term[..length as usize]);
                    }
                    return Ok(added);
                } else {
                    return Ok(false);
                }
            }
            let c = c as u16;
            if !added {
                added = true;
                self.skipped += 1;
                if self.skipped > self.skip {
                    self.term.push(if c == self.delimiter {
                        self.replacement
                    } else {
                        c
                    });
                    length += 1;
                } else {
                    self.start_position += 1;
                }
            } else if c == self.delimiter {
                if self.skipped > self.skip {
                    self.end_delimiter = true;
                    break;
                }
                self.skipped += 1;
                if self.skipped > self.skip {
                    self.term.push(self.replacement);
                    length += 1;
                } else {
                    self.start_position += 1;
                }
            } else if self.skipped > self.skip {
                self.term.push(c);
                length += 1;
            } else {
                self.start_position += 1;
            }
        }
        length += self.result_token.len() as i32;
        self.term.truncate(length as usize);
        self.emit(source, &*input, length);
        self.result_token.clear();
        self.result_token
            .extend_from_slice(&self.term[..length as usize]);
        Ok(true)
    }

    /// Equivalent to the final `PathHierarchyTokenizer.end()`;
    /// `Tokenizer::end` has run `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        // set final offset
        let final_offset = Tokenizer::<Self>::correct_offset_of(input, self.chars_read);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(final_offset, final_offset);
        Ok(())
    }

    /// Equivalent to `PathHierarchyTokenizer.reset()`; `Tokenizer::reset`
    /// has run `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.result_token.clear();
        self.chars_read = 0;
        self.end_delimiter = false;
        self.skipped = 0;
        self.start_position = 0;
        Ok(())
    }
}

/// Tokenizer for path-like hierarchies.
///
/// Take something like:
///
/// ```text
///  /something/something/else
/// ```
///
/// and make:
///
/// ```text
///  /something
///  /something/something
///  /something/something/else
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.path.PathHierarchyTokenizer`.
pub type PathHierarchyTokenizer = Tokenizer<PathHierarchyTokenizerLogic>;

/// Equivalent to `PathHierarchyTokenizer()`.
pub fn new_path_hierarchy_tokenizer() -> PathHierarchyTokenizer {
    new_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )
    .expect("INVARIANT: the defaults are valid")
}

/// Equivalent to `PathHierarchyTokenizer(int skip)`.
///
/// # Errors
///
/// As [`new_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_path_hierarchy_tokenizer_skip(skip: i32) -> Result<PathHierarchyTokenizer> {
    new_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        skip,
    )
}

/// Equivalent to `PathHierarchyTokenizer(int bufferSize, char delimiter)`.
///
/// # Errors
///
/// As [`new_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_path_hierarchy_tokenizer_buffer_size(
    buffer_size: i32,
    delimiter: u16,
) -> Result<PathHierarchyTokenizer> {
    new_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        buffer_size,
        delimiter,
        delimiter,
        DEFAULT_SKIP,
    )
}

/// Equivalent to `PathHierarchyTokenizer(char delimiter, char replacement)`.
pub fn new_path_hierarchy_tokenizer_replacement(
    delimiter: u16,
    replacement: u16,
) -> PathHierarchyTokenizer {
    new_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        delimiter,
        replacement,
        DEFAULT_SKIP,
    )
    .expect("INVARIANT: the default buffer size and skip are valid")
}

/// Equivalent to `PathHierarchyTokenizer(char delimiter, char replacement,
/// int skip)`.
///
/// # Errors
///
/// As [`new_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_path_hierarchy_tokenizer_replacement_skip(
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<PathHierarchyTokenizer> {
    new_path_hierarchy_tokenizer_buffer_size_replacement_skip(
        DEFAULT_BUFFER_SIZE,
        delimiter,
        replacement,
        skip,
    )
}

/// Equivalent to `PathHierarchyTokenizer(AttributeFactory factory, char
/// delimiter, char replacement, int skip)`.
///
/// # Errors
///
/// As [`new_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_path_hierarchy_tokenizer_factory(
    factory: Arc<dyn AttributeFactory>,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<PathHierarchyTokenizer> {
    new_path_hierarchy_tokenizer_factory_buffer_size(
        factory,
        DEFAULT_BUFFER_SIZE,
        delimiter,
        replacement,
        skip,
    )
}

/// Equivalent to `PathHierarchyTokenizer(int bufferSize, char delimiter,
/// char replacement, int skip)`.
///
/// # Errors
///
/// As [`new_path_hierarchy_tokenizer_factory_buffer_size`].
pub fn new_path_hierarchy_tokenizer_buffer_size_replacement_skip(
    buffer_size: i32,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<PathHierarchyTokenizer> {
    new_path_hierarchy_tokenizer_factory_buffer_size(
        default_token_attribute_factory(),
        buffer_size,
        delimiter,
        replacement,
        skip,
    )
}

/// Equivalent to `PathHierarchyTokenizer(AttributeFactory factory, int
/// bufferSize, char delimiter, char replacement, int skip)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws, and `IllegalArgumentException` "bufferSize cannot
/// be negative" or "skip cannot be negative".
pub fn new_path_hierarchy_tokenizer_factory_buffer_size(
    factory: Arc<dyn AttributeFactory>,
    buffer_size: i32,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<PathHierarchyTokenizer> {
    let (source, logic) =
        path_hierarchy_tokenizer_parts(factory, buffer_size, delimiter, replacement, skip)?;
    Ok(Tokenizer::with_source(source, logic))
}

/// The source and the body of the tokenizer that
/// [`new_path_hierarchy_tokenizer_factory_buffer_size`] builds, for a caller
/// that holds the body behind a box (`PathHierarchyTokenizerFactory`).
///
/// # Errors
///
/// As [`new_path_hierarchy_tokenizer_factory_buffer_size`].
pub(crate) fn path_hierarchy_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
    buffer_size: i32,
    delimiter: u16,
    replacement: u16,
    skip: i32,
) -> Result<(AttributeSource, PathHierarchyTokenizerLogic)> {
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
        PathHierarchyTokenizerLogic {
            delimiter,
            replacement,
            skip,
            start_position: 0,
            skipped: 0,
            end_delimiter: false,
            result_token: Vec::with_capacity(buffer_size as usize),
            chars_read: 0,
            term: Vec::new(),
        },
    ))
}
