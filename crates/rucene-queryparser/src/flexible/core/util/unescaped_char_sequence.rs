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

//! Port of `UnescapedCharSequence.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.util`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::jdk::char_sequence::{CharSequence, JavaCharSequence};
use crate::jdk::locale::Locale;
use crate::jdk::string::to_lower_case;

/// The `ArrayIndexOutOfBoundsException` of an array access.
fn array_index_out_of_bounds(index: usize, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// CharsSequence with escaped chars information.
///
/// Equivalent to the final class
/// `org.apache.lucene.queryparser.flexible.core.util.UnescapedCharSequence`,
/// over UTF-16 code units; it is a [`CharSequence::Object`] (see
/// [`into_char_sequence`](Self::into_char_sequence)).
#[derive(Debug, Clone)]
pub struct UnescapedCharSequence {
    chars: Vec<u16>,
    was_escaped: Vec<bool>,
}

impl UnescapedCharSequence {
    /// Create an escaped CharSequence
    ///
    /// Equivalent to `new UnescapedCharSequence(char[] chars, boolean[]
    /// wasEscaped, int offset, int length)`.
    ///
    /// # Errors
    ///
    /// The `ArrayIndexOutOfBoundsException` of `System.arraycopy` for a
    /// range outside either array.
    pub fn new(chars: &[u16], was_escaped: &[bool], offset: usize, length: usize) -> Result<Self> {
        let end = offset.saturating_add(length);
        if end > chars.len() {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!(
                    "arraycopy: last source index {end} out of bounds for char[{}]",
                    chars.len()
                ),
            ));
        }
        if end > was_escaped.len() {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!(
                    "arraycopy: last source index {end} out of bounds for boolean[{}]",
                    was_escaped.len()
                ),
            ));
        }
        Ok(Self {
            chars: chars[offset..end].to_vec(),
            was_escaped: was_escaped[offset..end].to_vec(),
        })
    }

    /// Create a non-escaped CharSequence
    ///
    /// Equivalent to `new UnescapedCharSequence(CharSequence text)`.
    pub fn from_text(text: &CharSequence) -> Self {
        let chars = text.units();
        let was_escaped = vec![false; chars.len()];
        Self { chars, was_escaped }
    }

    /// This sequence as a [`CharSequence`] reference.
    pub fn into_char_sequence(self) -> CharSequence {
        CharSequence::Object(Arc::new(self))
    }

    /// Equivalent to `charAt(int index)`.
    ///
    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` for an index outside the sequence.
    pub fn char_at(&self, index: usize) -> Result<u16> {
        self.chars
            .get(index)
            .copied()
            .ok_or_else(|| array_index_out_of_bounds(index, self.chars.len()))
    }

    /// Equivalent to `length()`.
    pub fn length(&self) -> usize {
        self.chars.len()
    }

    /// Equivalent to `subSequence(int start, int end)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the array copy for a range outside the sequence.
    pub fn sub_sequence(&self, start: usize, end: usize) -> Result<CharSequence> {
        if end < start {
            return Err(LuceneError::unchecked(
                "java.lang.NegativeArraySizeException",
                format!("{}", end as i64 - start as i64),
            ));
        }
        let new_length = end - start;
        Ok(Self::new(&self.chars, &self.was_escaped, start, new_length)?.into_char_sequence())
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        String::from_utf16_lossy(&self.chars)
    }

    /// Return a escaped String
    ///
    /// Equivalent to `toStringEscaped()`: a backslash before every backslash
    /// and every escaped character.
    pub fn to_string_escaped(&self) -> String {
        // non efficient implementation
        let mut result: Vec<u16> = Vec::with_capacity(self.length());
        for i in 0..self.length() {
            if self.chars[i] == u16::from(b'\\') || self.was_escaped[i] {
                result.push(u16::from(b'\\'));
            }
            result.push(self.chars[i]);
        }
        String::from_utf16_lossy(&result)
    }

    /// Return a escaped String
    ///
    /// Equivalent to `toStringEscaped(char[] enabledChars)`: a backslash
    /// before every backslash and every escaped character of
    /// `enabled_chars`.
    pub fn to_string_escaped_with(&self, enabled_chars: &[u16]) -> String {
        // TODO: non efficient implementation, refactor this code
        let mut result: Vec<u16> = Vec::with_capacity(self.length());
        for i in 0..self.length() {
            if self.chars[i] == u16::from(b'\\') {
                result.push(u16::from(b'\\'));
            } else {
                for &character in enabled_chars {
                    if self.chars[i] == character && self.was_escaped[i] {
                        result.push(u16::from(b'\\'));
                        break;
                    }
                }
            }
            result.push(self.chars[i]);
        }
        String::from_utf16_lossy(&result)
    }

    /// Equivalent to `wasEscaped(int index)`.
    ///
    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` for an index outside the sequence.
    pub fn was_escaped(&self, index: usize) -> Result<bool> {
        self.was_escaped
            .get(index)
            .copied()
            .ok_or_else(|| array_index_out_of_bounds(index, self.was_escaped.len()))
    }

    /// Equivalent to the static `wasEscaped(CharSequence text, int index)`:
    /// `false` for a sequence that is not an `UnescapedCharSequence`.
    ///
    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` for an index outside the sequence.
    pub fn was_escaped_in(text: &CharSequence, index: usize) -> Result<bool> {
        match text
            .as_any()
            .and_then(|a| a.downcast_ref::<UnescapedCharSequence>())
        {
            Some(u) => u.was_escaped(index),
            None => Ok(false),
        }
    }

    /// Equivalent to the static `toLowerCase(CharSequence text, Locale
    /// locale)`: the text lower-cased for `locale`, keeping the escape flags
    /// of an `UnescapedCharSequence` (by position, as Java copies them).
    ///
    /// # Errors
    ///
    /// The `ArrayIndexOutOfBoundsException` of the copy when lower-casing
    /// lengthens the text past its escape flags.
    pub fn to_lower_case(text: &CharSequence, locale: &Locale) -> Result<CharSequence> {
        let chars = to_lower_case(&text.units(), locale);
        match text
            .as_any()
            .and_then(|a| a.downcast_ref::<UnescapedCharSequence>())
        {
            Some(u) => Ok(Self::new(&chars, &u.was_escaped, 0, chars.len())?.into_char_sequence()),
            None => {
                let was_escaped = vec![false; chars.len()];
                Ok(Self { chars, was_escaped }.into_char_sequence())
            }
        }
    }
}

impl JavaCharSequence for UnescapedCharSequence {
    fn length(&self) -> usize {
        UnescapedCharSequence::length(self)
    }

    fn char_at(&self, index: usize) -> Result<u16> {
        UnescapedCharSequence::char_at(self, index)
    }

    fn sub_sequence(&self, start: usize, end: usize) -> Result<CharSequence> {
        UnescapedCharSequence::sub_sequence(self, start, end)
    }

    fn to_java_string(&self) -> String {
        UnescapedCharSequence::to_java_string(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
