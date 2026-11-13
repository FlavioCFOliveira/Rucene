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

//! A `CharacterIterator` over a `char[]` region, ported from
//! `org.apache.lucene.analysis.util.CharArrayIterator`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::jdk::character_iterator::{CharacterIterator, DONE};

/// True if this JRE has a buggy `BreakIterator` implementation.
///
/// Equivalent to `CharArrayIterator.HAS_BUGGY_BREAKITERATORS`.
///
/// **Divergence from Lucene 10.5.0.** Java computes it in a static
/// initializer, by asking `BreakIterator.getSentenceInstance(Locale.US)` to
/// break `"\udb40\udc53"` and recording whether `next()` threw. That
/// factory is the JDK's, which the `BreakIterator` emulation (rmp task #447)
/// delivers; this port holds the value JDK 25 computes, which a JDK 25.0.5
/// oracle printed: `HAS_BUGGY_BREAKITERATORS=false`.
pub const HAS_BUGGY_BREAKITERATORS: bool = false;

/// A `CharacterIterator` used internally for use with `BreakIterator`.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.analysis.util.CharArrayIterator` (`@lucene.internal`).
/// Its abstract method `jreBugWorkaround(char)`, which each subclass
/// supplies, is the function this iterator is built with; the two subclasses
/// Lucene declares come from [`new_sentence_instance`](Self::new_sentence_instance)
/// and [`new_word_instance`](Self::new_word_instance). The text is UTF-16
/// units, as Java's `char[]` is.
///
/// **Divergence from Lucene 10.5.0.** Java keeps a reference to the caller's
/// array, so a later write into that array shows through the iterator; Rust
/// does not let the iterator alias a buffer its owner keeps writing, so
/// [`set_text`](Self::set_text) takes a copy of the array. Lucene's callers
/// set the text again after every change to their buffer
/// (`SegmentingTokenizerBase.refill`, `ThaiTokenizer.setNextSentence`), so
/// they see the same characters either way.
#[derive(Clone, Debug)]
pub struct CharArrayIterator {
    array: Arc<[u16]>,
    start: i32,
    index: i32,
    length: i32,
    limit: i32,
    jre_bug_workaround: fn(u16) -> u16,
}

impl CharArrayIterator {
    /// Creates an iterator whose `jreBugWorkaround` is `jre_bug_workaround`,
    /// over no text: the subclass constructor of Java's abstract class, whose
    /// fields start as `null` and `0`.
    pub fn new(jre_bug_workaround: fn(u16) -> u16) -> Self {
        Self {
            array: Arc::from(Vec::new()),
            start: 0,
            index: 0,
            length: 0,
            limit: 0,
            jre_bug_workaround,
        }
    }

    /// Equivalent to `getText()`.
    pub fn get_text(&self) -> &[u16] {
        &self.array
    }

    /// Equivalent to `getStart()`.
    pub fn get_start(&self) -> i32 {
        self.start
    }

    /// Equivalent to `getLength()`.
    pub fn get_length(&self) -> i32 {
        self.length
    }

    /// Set a new region of text to be examined by this iterator: `array` is
    /// the text buffer to examine, `start` the offset into the buffer, and
    /// `length` the maximum length to examine.
    ///
    /// Equivalent to `setText(char[], int, int)`; see the type's
    /// documentation for the copy of `array`.
    pub fn set_text(&mut self, array: &[u16], start: i32, length: i32) {
        self.array = Arc::from(array);
        self.start = start;
        self.index = start;
        self.length = length;
        self.limit = start + length;
    }

    /// `jreBugWorkaround(char)`.
    fn jre_bug_workaround(&self, ch: u16) -> u16 {
        (self.jre_bug_workaround)(ch)
    }

    /// Create a new `CharArrayIterator` that works around JRE bugs in a
    /// manner suitable for `BreakIterator.getSentenceInstance()`.
    ///
    /// Equivalent to `CharArrayIterator.newSentenceInstance()`.
    pub fn new_sentence_instance() -> Self {
        if HAS_BUGGY_BREAKITERATORS {
            // work around this for now by lying about all surrogates to
            // the sentence tokenizer, instead we treat them all as
            // SContinue so we won't break around them.
            Self::new(|ch| {
                if (0xD800..=0xDFFF).contains(&ch) {
                    0x002C
                } else {
                    ch
                }
            })
        } else {
            // no bugs
            Self::new(|ch| ch)
        }
    }

    /// Create a new `CharArrayIterator` that works around JRE bugs in a
    /// manner suitable for `BreakIterator.getWordInstance()`.
    ///
    /// Equivalent to `CharArrayIterator.newWordInstance()`.
    pub fn new_word_instance() -> Self {
        if HAS_BUGGY_BREAKITERATORS {
            // work around this for now by lying about all surrogates to the
            // word, instead we treat them all as ALetter so we won't break
            // around them.
            Self::new(|ch| {
                if (0xD800..=0xDFFF).contains(&ch) {
                    0x0041
                } else {
                    ch
                }
            })
        } else {
            // no bugs
            Self::new(|ch| ch)
        }
    }
}

impl CharacterIterator for CharArrayIterator {
    fn current(&self) -> u16 {
        if self.index == self.limit {
            DONE
        } else {
            self.jre_bug_workaround(self.array[self.index as usize])
        }
    }

    fn first(&mut self) -> u16 {
        self.index = self.start;
        self.current()
    }

    fn get_begin_index(&self) -> i32 {
        0
    }

    fn get_end_index(&self) -> i32 {
        self.length
    }

    fn get_index(&self) -> i32 {
        self.index - self.start
    }

    fn last(&mut self) -> u16 {
        self.index = if self.limit == self.start {
            self.limit
        } else {
            self.limit - 1
        };
        self.current()
    }

    fn next(&mut self) -> u16 {
        self.index += 1;
        if self.index >= self.limit {
            self.index = self.limit;
            DONE
        } else {
            self.current()
        }
    }

    fn previous(&mut self) -> u16 {
        self.index -= 1;
        if self.index < self.start {
            self.index = self.start;
            DONE
        } else {
            self.current()
        }
    }

    fn set_index(&mut self, position: i32) -> Result<u16> {
        if position < self.get_begin_index() || position > self.get_end_index() {
            return Err(LuceneError::illegal_argument(format!(
                "Illegal Position: {position}"
            )));
        }
        self.index = self.start + position;
        Ok(self.current())
    }

    /// Equivalent to `CharArrayIterator.clone()`, `super.clone()`: a copy
    /// with the same text, region and position.
    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        Box::new(self.clone())
    }
}
