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

//! Port of `WholeBreakIterator.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::break_iterator::{BreakIterator, DONE};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, StringCharacterIterator};

use crate::uhighlight::custom_separator_break_iterator::java_is_boundary;

/// Just produces one single fragment for the entire text.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.WholeBreakIterator`.
#[derive(Debug)]
pub struct WholeBreakIterator {
    text: Box<dyn CharacterIterator>,
    start: i32,
    end: i32,
    current: i32,
}

impl Default for WholeBreakIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl WholeBreakIterator {
    /// Equivalent to `new WholeBreakIterator()`. Java's text is `null` until
    /// `setText`; the port's is empty.
    pub fn new() -> Self {
        Self {
            text: Box::new(StringCharacterIterator::new(Vec::new())),
            start: 0,
            end: 0,
            current: 0,
        }
    }
}

impl Clone for WholeBreakIterator {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone_box(),
            start: self.start,
            end: self.end,
            current: self.current,
        }
    }
}

impl BreakIterator for WholeBreakIterator {
    fn current(&self) -> i32 {
        self.current
    }

    fn first(&mut self) -> i32 {
        self.current = self.start;
        self.current
    }

    fn following(&mut self, pos: i32) -> Result<i32> {
        if pos < self.start || pos > self.end {
            Err(LuceneError::illegal_argument("offset out of bounds"))
        } else if pos == self.end {
            // this conflicts with the javadocs, but matches actual behavior
            // (Oracle has a bug in something) https://bugs.openjdk.java.net/browse/JDK-8015110
            self.current = self.end;
            Ok(DONE)
        } else {
            Ok(self.last())
        }
    }

    fn get_text(&mut self) -> &dyn CharacterIterator {
        &*self.text
    }

    fn last(&mut self) -> i32 {
        self.current = self.end;
        self.current
    }

    fn next(&mut self) -> i32 {
        if self.current == self.end {
            DONE
        } else {
            self.last()
        }
    }

    fn next_n(&mut self, n: i32) -> i32 {
        if n < 0 {
            for _ in 0..-(n as i64) {
                self.previous();
            }
        } else {
            for _ in 0..n {
                self.next();
            }
        }
        self.current()
    }

    fn preceding(&mut self, pos: i32) -> Result<i32> {
        if pos < self.start || pos > self.end {
            Err(LuceneError::illegal_argument("offset out of bounds"))
        } else if pos == self.start {
            // this conflicts with the javadocs, but matches actual behavior
            // (Oracle has a bug in something) https://bugs.openjdk.java.net/browse/JDK-8015110
            self.current = self.start;
            Ok(DONE)
        } else {
            Ok(self.first())
        }
    }

    fn previous(&mut self) -> i32 {
        if self.current == self.start {
            DONE
        } else {
            self.first()
        }
    }

    /// The base class's `isBoundary(int)`.
    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        java_is_boundary(self, offset)
    }

    fn set_text(&mut self, new_text: Box<dyn CharacterIterator>) {
        self.start = new_text.get_begin_index();
        self.end = new_text.get_end_index();
        self.text = new_text;
        self.current = self.start;
    }

    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(self.clone())
    }
}
