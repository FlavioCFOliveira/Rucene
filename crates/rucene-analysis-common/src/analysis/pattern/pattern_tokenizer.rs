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

//! A tokenizer that splits or extracts tokens with a regular expression,
//! ported from `org.apache.lucene.analysis.pattern.PatternTokenizer`.

use std::sync::Arc;

use rucene::analysis::java_regex::{JavaMatcher, JavaPattern};
use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute, Tokenizer,
    TokenizerLogic,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};

use crate::jdk::character::utf16_to_term_chars;

/// The body of [`PatternTokenizer`]: what Lucene's `PatternTokenizer` adds
/// to `Tokenizer`.
///
/// Java's `StringBuilder str` holds the whole input as UTF-16 units; the
/// matcher runs over it (see
/// [`JavaMatcher`] for its
/// declared limits). A token copies its units into the term, where an
/// unpaired surrogate becomes U+FFFD (`jdk::character::utf16_to_term_chars`).
#[derive(Debug)]
pub struct PatternTokenizerLogic {
    str: Vec<u16>,
    index: i32,
    group: i32,
    matcher: JavaMatcher,
    /// Java's `final char[] buffer = new char[8192]`.
    buffer: Vec<u16>,
}

/// Sets the term to the units `units` of the input.
fn set_term(source: &AttributeSource, units: &[u16]) {
    let chars = utf16_to_term_chars(units);
    source
        .get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("INVARIANT: the constructor added the CharTermAttribute")
        .copy_buffer(&chars, 0, chars.len());
}

/// Sets the offsets of the token, corrected through `input`.
fn set_offset(source: &AttributeSource, input: &dyn CharReader, start: i32, end: i32) {
    let start = Tokenizer::<PatternTokenizerLogic>::correct_offset_of(Some(input), start);
    let end = Tokenizer::<PatternTokenizerLogic>::correct_offset_of(Some(input), end);
    source
        .get_attribute_interface_mut::<dyn OffsetAttribute>()
        .expect("INVARIANT: the constructor added the OffsetAttribute")
        .set_offset(start, end);
}

impl PatternTokenizerLogic {
    /// Equivalent to the private `fillBuffer(Reader)`: the whole input,
    /// read 8192 units at a time.
    fn fill_buffer(&mut self, input: &mut dyn CharReader) -> Result<()> {
        self.str.clear();
        loop {
            let len = input.read(&mut self.buffer)?;
            if len == 0 {
                break;
            }
            self.str.extend_from_slice(&self.buffer[..len]);
        }
        Ok(())
    }
}

impl TokenizerLogic for PatternTokenizerLogic {
    /// Equivalent to `PatternTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.index >= self.str.len() as i32 {
            return Ok(false);
        }
        source.clear_attributes();
        if self.group >= 0 {
            // match a specific group
            while self.matcher.find() {
                self.index = self.matcher.start_group(self.group)?;
                let end_index = self.matcher.end_group(self.group)?;
                if self.index == end_index {
                    continue;
                }
                set_term(source, &self.str[self.index as usize..end_index as usize]);
                set_offset(source, input, self.index, end_index);
                return Ok(true);
            }

            self.index = i32::MAX; // mark exhausted
            Ok(false)
        } else {
            // String.split() functionality
            while self.matcher.find() {
                let start = self.matcher.start()?;
                if start - self.index > 0 {
                    // found a non-zero-length token
                    set_term(source, &self.str[self.index as usize..start as usize]);
                    set_offset(source, input, self.index, start);
                    self.index = self.matcher.end()?;
                    return Ok(true);
                }

                self.index = self.matcher.end()?;
            }

            if self.str.len() as i32 - self.index == 0 {
                self.index = i32::MAX; // mark exhausted
                return Ok(false);
            }

            set_term(source, &self.str[self.index as usize..]);
            set_offset(source, input, self.index, self.str.len() as i32);
            self.index = i32::MAX; // mark exhausted
            Ok(true)
        }
    }

    /// Equivalent to `PatternTokenizer.end()`; `Tokenizer::end` has run
    /// `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        let ofs = Tokenizer::<Self>::correct_offset_of(input, self.str.len() as i32);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(ofs, ofs);
        Ok(())
    }

    /// Equivalent to `PatternTokenizer.close()`: after `super.close()`, the
    /// buffered input is released (`str.setLength(0); str.trimToSize()`).
    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        self.str.clear();
        self.str.shrink_to_fit();
        Ok(())
    }

    /// Equivalent to `PatternTokenizer.reset()`; `Tokenizer::reset` has run
    /// `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, reader: &mut dyn CharReader) -> Result<()> {
        self.fill_buffer(reader)?;
        self.matcher.reset_input(&self.str);
        self.index = 0;
        Ok(())
    }
}

/// This tokenizer uses regex pattern matching to construct distinct tokens
/// for the input stream. It takes two arguments: "pattern" and "group".
///
/// * "pattern" is the regular expression.
/// * "group" says which group to extract into tokens.
///
/// group=-1 (the default) is equivalent to "split". In this case, the tokens
/// will be equivalent to the output from (without empty tokens):
/// `String#split(java.lang.String)`
///
/// Using group >= 0 selects the matching group as the token. For example, if
/// you have:
///
/// ```text
///  pattern = \'([^\']+)\'
///  group = 0
///  input = aaa 'bbb' 'ccc'
/// ```
///
/// the output will be two tokens: 'bbb' and 'ccc' (including the ' marks).
/// With the same input but using group=1, the output would be: bbb and ccc
/// (no ' marks)
///
/// NOTE: This Tokenizer does not output tokens that are of zero length.
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternTokenizer`.
pub type PatternTokenizer = Tokenizer<PatternTokenizerLogic>;

/// creates a new PatternTokenizer returning tokens from group (-1 for split
/// functionality)
///
/// Equivalent to `PatternTokenizer(Pattern, int)`, over
/// `DEFAULT_TOKEN_ATTRIBUTE_FACTORY`.
///
/// # Errors
///
/// As [`new_pattern_tokenizer_with_factory`].
pub fn new_pattern_tokenizer(pattern: &JavaPattern, group: i32) -> Result<PatternTokenizer> {
    new_pattern_tokenizer_with_factory(default_token_attribute_factory(), pattern, group)
}

/// creates a new PatternTokenizer returning tokens from group (-1 for split
/// functionality)
///
/// Equivalent to `PatternTokenizer(AttributeFactory, Pattern, int)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws, and `IllegalArgumentException` "invalid group
/// specified: pattern only has: N capturing groups" for a group the pattern
/// does not have.
pub fn new_pattern_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
    pattern: &JavaPattern,
    group: i32,
) -> Result<PatternTokenizer> {
    let (source, logic) = pattern_tokenizer_parts(factory, pattern, group)?;
    Ok(Tokenizer::with_source(source, logic))
}

/// The source and the body of the `PatternTokenizer` that
/// [`new_pattern_tokenizer_with_factory`] builds, for a caller that holds the
/// body behind a box (`PatternTokenizerFactory`).
///
/// # Errors
///
/// As [`new_pattern_tokenizer_with_factory`].
pub(crate) fn pattern_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
    pattern: &JavaPattern,
    group: i32,
) -> Result<(AttributeSource, PatternTokenizerLogic)> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;

    // Use "" instead of str so don't consume chars
    // (fillBuffer) from the input on throwing IAE below:
    let matcher = pattern.matcher(&[]);

    // confusingly group count depends ENTIRELY on the pattern but is only accessible via matcher
    if group >= 0 && group as usize > matcher.group_count() {
        return Err(LuceneError::illegal_argument(format!(
            "invalid group specified: pattern only has: {} capturing groups",
            matcher.group_count()
        )));
    }
    Ok((
        source,
        PatternTokenizerLogic {
            str: Vec::new(),
            index: 0,
            group,
            matcher,
            buffer: vec![0; 8192],
        },
    ))
}
