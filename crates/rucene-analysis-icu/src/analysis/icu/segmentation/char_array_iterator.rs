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

//! Wraps a `char[]` as `CharacterIterator` for processing with a
//! `BreakIterator`, ported from
//! `org.apache.lucene.analysis.icu.segmentation.CharArrayIterator`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};

/// Wraps a `char[]` as `CharacterIterator` for processing with a
/// `BreakIterator`.
///
/// Equivalent to `org.apache.lucene.analysis.icu.segmentation.CharArrayIterator`
/// (package-private, `@lucene.experimental`; public here so that the
/// module's tests, which Java compiles into the same package, reach it).
///
/// **Divergence from Lucene 10.5.0.** Java keeps a reference to the
/// caller's `char[]`; here the iterator shares the caller's units through
/// an `Arc<[u16]>`, which the caller (`ICUTokenizer`) rebuilds whenever it
/// refills its buffer, and then sets again on every iterator, as Java's
/// `refill` calls `setText` again after every change to the buffer (the
/// shape of `rucene-analysis-common`'s `CharArrayIterator`, for the same
/// reason: Rust does not let the iterator alias a buffer its owner keeps
/// writing). The characters each iterator sees are the same.
#[derive(Clone, Debug)]
pub struct CharArrayIterator {
    array: Arc<[u16]>,
    start: i32,
    index: i32,
    length: i32,
    limit: i32,
}

impl Default for CharArrayIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl CharArrayIterator {
    /// An iterator over no text: Java's implicit constructor, whose fields
    /// start as `null` and `0`.
    pub fn new() -> Self {
        Self {
            array: Arc::from(Vec::new()),
            start: 0,
            index: 0,
            length: 0,
            limit: 0,
        }
    }

    /// Equivalent to `getText()`.
    pub fn get_text(&self) -> &Arc<[u16]> {
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

    /// Set a new region of text to be examined by this iterator.
    ///
    /// * `array`: text buffer to examine
    /// * `start`: offset into buffer
    /// * `length`: maximum length to examine
    ///
    /// Equivalent to the package-private `setText(char[], int, int)`.
    pub fn set_text(&mut self, array: Arc<[u16]>, start: i32, length: i32) {
        self.array = array;
        self.start = start;
        self.index = start;
        self.length = length;
        self.limit = start + length;
    }
}

impl CharacterIterator for CharArrayIterator {
    /// Equivalent to `current()`.
    fn current(&self) -> u16 {
        if self.index == self.limit {
            DONE
        } else {
            self.array[self.index as usize]
        }
    }

    /// Equivalent to `first()`.
    fn first(&mut self) -> u16 {
        self.index = self.start;
        self.current()
    }

    /// Equivalent to `getBeginIndex()`: 0.
    fn get_begin_index(&self) -> i32 {
        0
    }

    /// Equivalent to `getEndIndex()`: the length.
    fn get_end_index(&self) -> i32 {
        self.length
    }

    /// Equivalent to `getIndex()`.
    fn get_index(&self) -> i32 {
        self.index - self.start
    }

    /// Equivalent to `last()`.
    fn last(&mut self) -> u16 {
        self.index = if self.limit == self.start {
            self.limit
        } else {
            self.limit - 1
        };
        self.current()
    }

    /// Equivalent to `next()`.
    fn next(&mut self) -> u16 {
        self.index += 1;
        if self.index >= self.limit {
            self.index = self.limit;
            DONE
        } else {
            self.current()
        }
    }

    /// Equivalent to `previous()`.
    fn previous(&mut self) -> u16 {
        self.index -= 1;
        if self.index < self.start {
            self.index = self.start;
            DONE
        } else {
            self.current()
        }
    }

    /// Equivalent to `setIndex(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal Position: " + position)` outside
    /// `getBeginIndex()..=getEndIndex()`.
    fn set_index(&mut self, position: i32) -> Result<u16> {
        if position < self.get_begin_index() || position > self.get_end_index() {
            return Err(LuceneError::illegal_argument(format!(
                "Illegal Position: {position}"
            )));
        }
        self.index = self.start + position;
        Ok(self.current())
    }

    /// Equivalent to `clone()`: an iterator over the same text, at the same
    /// index.
    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        let mut clone = CharArrayIterator::new();
        clone.set_text(Arc::clone(&self.array), self.start, self.length);
        clone.index = self.index;
        Box::new(clone)
    }
}
