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

//! A `CHECKCOMPOUNDPATTERN` rule, ported from
//! `org.apache.lucene.analysis.hunspell.CheckCompoundPattern`.

use std::fmt;

use rucene::error::{LuceneError, Result};
use rucene::util::CharsRef;

use super::dictionary::{Dictionary, FlagParsingStrategy};
use super::root::Root;
use crate::jdk::string::{concat, index_of_unit, split_whitespace, to_rust_string};

/// A pattern forbidding (or, with a replacement, rewriting) a compound
/// boundary between a part ending with `endChars` and one beginning with
/// `beginChars`, optionally restricted to roots carrying flags.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.CheckCompoundPattern`.
///
/// **Divergence.** Java keeps the `Dictionary` the pattern belongs to; a
/// Rust value cannot point back at the dictionary that owns it, so the
/// methods that ask it for flags take the dictionary.
#[derive(Debug, Clone)]
pub(crate) struct CheckCompoundPattern {
    end_chars: Vec<u16>,
    begin_chars: Vec<u16>,
    replacement: Option<Vec<u16>>,
    end_flags: Vec<u16>,
    begin_flags: Vec<u16>,
}

impl CheckCompoundPattern {
    /// Parses the `CHECKCOMPOUNDPATTERN` line `unparsed` with `strategy`.
    ///
    /// Equivalent to `CheckCompoundPattern(String, FlagParsingStrategy,
    /// Dictionary)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid pattern: " + unparsed)` for fewer
    /// than three parts, and what parsing the flags raises.
    pub(crate) fn new(unparsed: &[u16], strategy: &dyn FlagParsingStrategy) -> Result<Self> {
        let parts = split_whitespace(unparsed, true);
        if parts.len() < 3 {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid pattern: {}",
                to_rust_string(unparsed)
            )));
        }
        let slash = u16::from(b'/');
        let (end_chars, end_flags) = match index_of_unit(&parts[1], slash, 0) {
            None => (parts[1].clone(), Vec::new()),
            Some(flag_sep) => (
                parts[1][..flag_sep].to_vec(),
                strategy.parse_flags(&parts[1][flag_sep + 1..])?,
            ),
        };
        let (begin_chars, begin_flags) = match index_of_unit(&parts[2], slash, 0) {
            None => (parts[2].clone(), Vec::new()),
            Some(flag_sep) => (
                parts[2][..flag_sep].to_vec(),
                strategy.parse_flags(&parts[2][flag_sep + 1..])?,
            ),
        };
        let replacement = if parts.len() == 3 {
            None
        } else {
            Some(parts[3].clone())
        };
        Ok(Self {
            end_chars,
            begin_chars,
            replacement,
            end_flags,
            begin_flags,
        })
    }

    /// Equivalent to `prohibitsCompounding(CharsRef, int, Root, Root)`.
    pub(crate) fn prohibits_compounding(
        &self,
        dictionary: &Dictionary,
        word: &CharsRef,
        break_pos: usize,
        root_before: &Root,
        root_after: &Root,
    ) -> bool {
        let break_pos = break_pos as i64;
        if is_non_affixed_pattern(&self.end_chars) {
            if !chars_match(
                word,
                break_pos - root_before.word.len() as i64,
                &root_before.word,
            ) {
                return false;
            }
        } else if !chars_match(
            word,
            break_pos - self.end_chars.len() as i64,
            &self.end_chars,
        ) {
            return false;
        }
        if is_non_affixed_pattern(&self.begin_chars) {
            if !chars_match(word, break_pos, &root_after.word) {
                return false;
            }
        } else if !chars_match(word, break_pos, &self.begin_chars) {
            return false;
        }
        if !self.end_flags.is_empty() && !has_all_flags(dictionary, root_before, &self.end_flags) {
            return false;
        }
        if !self.begin_flags.is_empty() && !has_all_flags(dictionary, root_after, &self.begin_flags)
        {
            return false;
        }
        true
    }

    /// The word with the replacement at `break_pos` expanded into
    /// `endChars + beginChars`, or `None` when the replacement does not occur
    /// there.
    ///
    /// Equivalent to `expandReplacement(CharsRef, int)`.
    pub(crate) fn expand_replacement(&self, word: &CharsRef, break_pos: usize) -> Option<CharsRef> {
        let replacement = self.replacement.as_ref()?;
        if !chars_match(word, break_pos as i64, replacement) {
            return None;
        }
        // `new String(word.chars, 0, word.offset + breakPos)`
        let head = &word.chars[..word.offset + break_pos];
        // `word.subSequence(breakPos + replacement.length(), word.length)`
        let tail_start = break_pos + replacement.len();
        let tail = &word.chars[word.offset + tail_start..word.offset + word.length];
        Some(CharsRef::from_chars(concat(&[
            head,
            &self.end_chars,
            &self.begin_chars,
            tail,
        ])))
    }

    /// Equivalent to `endLength()`.
    pub(crate) fn end_length(&self) -> usize {
        self.end_chars.len()
    }
}

impl fmt::Display for CheckCompoundPattern {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}",
            to_rust_string(&self.end_chars),
            to_rust_string(&self.begin_chars)
        )?;
        if let Some(replacement) = &self.replacement {
            write!(f, " -> {}", to_rust_string(replacement))?;
        }
        Ok(())
    }
}

/// Equivalent to `isNonAffixedPattern(String)`.
fn is_non_affixed_pattern(pattern: &[u16]) -> bool {
    pattern.len() == 1 && pattern[0] == u16::from(b'0')
}

/// Equivalent to `hasAllFlags(Root, char[])`.
fn has_all_flags(dictionary: &Dictionary, root: &Root, flags: &[u16]) -> bool {
    flags
        .iter()
        .all(|&flag| dictionary.has_flag(root.entry_id, flag))
}

/// Equivalent to `charsMatch(CharsRef, int, CharSequence)`.
fn chars_match(word: &CharsRef, offset: i64, pattern: &[u16]) -> bool {
    let len = pattern.len() as i64;
    let word_length = word.length as i64;
    if word_length - offset < len || offset < 0 || offset > word_length {
        return false;
    }
    let start = word.offset + offset as usize;
    word.chars[start..start + pattern.len()] == *pattern
}
