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

//! Port of `CustomSeparatorBreakIterator.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::break_iterator::{BreakIterator, DONE};
use rucene_analysis_common::jdk::character_iterator::{
    CharacterIterator, StringCharacterIterator, DONE as CHAR_DONE,
};

/// A `BreakIterator` that breaks the text whenever a certain separator,
/// provided as a constructor argument, is found.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.CustomSeparatorBreakIterator`.
#[derive(Debug)]
pub struct CustomSeparatorBreakIterator {
    separator: u16,
    text: Box<dyn CharacterIterator>,
    current: i32,
}

impl CustomSeparatorBreakIterator {
    /// Equivalent to `new CustomSeparatorBreakIterator(char)`. Java's text
    /// is `null` until `setText`; the port's is empty.
    pub fn new(separator: u16) -> Self {
        Self {
            separator,
            text: Box::new(StringCharacterIterator::new(Vec::new())),
            current: 0,
        }
    }

    fn advance_forward(&mut self) -> i32 {
        loop {
            let c = self.text.next();
            if c == CHAR_DONE {
                break;
            }
            if c == self.separator {
                self.current = self.text.get_index() + 1;
                return self.current;
            }
        }
        debug_assert!(self.text.get_index() == self.text.get_end_index());
        self.current = self.text.get_index();
        self.current
    }

    fn advance_backward(&mut self) -> i32 {
        loop {
            let c = self.text.previous();
            if c == CHAR_DONE {
                break;
            }
            if c == self.separator {
                self.current = self.text.get_index() + 1;
                return self.current;
            }
        }
        debug_assert!(self.text.get_index() == self.text.get_begin_index());
        self.current = self.text.get_index();
        self.current
    }
}

impl Clone for CustomSeparatorBreakIterator {
    fn clone(&self) -> Self {
        Self {
            separator: self.separator,
            text: self.text.clone_box(),
            current: self.current,
        }
    }
}

impl BreakIterator for CustomSeparatorBreakIterator {
    fn current(&self) -> i32 {
        self.current
    }

    fn first(&mut self) -> i32 {
        let begin = self.text.get_begin_index();
        let _ = self.text.set_index(begin);
        self.current = self.text.get_index();
        self.current
    }

    fn last(&mut self) -> i32 {
        let end = self.text.get_end_index();
        let _ = self.text.set_index(end);
        self.current = self.text.get_index();
        self.current
    }

    fn next(&mut self) -> i32 {
        if self.text.get_index() == self.text.get_end_index() {
            DONE
        } else {
            self.advance_forward()
        }
    }

    fn following(&mut self, pos: i32) -> Result<i32> {
        if pos < self.text.get_begin_index() || pos > self.text.get_end_index() {
            Err(LuceneError::illegal_argument("offset out of bounds"))
        } else if pos == self.text.get_end_index() {
            // this conflicts with the javadocs, but matches actual behavior
            // (Oracle has a bug in something) https://bugs.openjdk.java.net/browse/JDK-8015110
            let end = self.text.get_end_index();
            let _ = self.text.set_index(end);
            self.current = self.text.get_index();
            Ok(DONE)
        } else {
            self.text.set_index(pos)?;
            self.current = self.text.get_index();
            Ok(self.advance_forward())
        }
    }

    fn previous(&mut self) -> i32 {
        if self.text.get_index() == self.text.get_begin_index() {
            DONE
        } else {
            self.advance_backward()
        }
    }

    fn preceding(&mut self, pos: i32) -> Result<i32> {
        if pos < self.text.get_begin_index() || pos > self.text.get_end_index() {
            Err(LuceneError::illegal_argument("offset out of bounds"))
        } else if pos == self.text.get_begin_index() {
            // this conflicts with the javadocs, but matches actual behavior
            // (Oracle has a bug in something) https://bugs.openjdk.java.net/browse/JDK-8015110
            let begin = self.text.get_begin_index();
            let _ = self.text.set_index(begin);
            self.current = self.text.get_index();
            Ok(DONE)
        } else {
            self.text.set_index(pos)?;
            self.current = self.text.get_index();
            Ok(self.advance_backward())
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

    /// The base class's `isBoundary(int)`: `following(offset - 1)` lands on
    /// `offset`, `offset` being the begin index or a later boundary.
    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        java_is_boundary(self, offset)
    }

    fn get_text(&mut self) -> &dyn CharacterIterator {
        &*self.text
    }

    fn set_text(&mut self, new_text: Box<dyn CharacterIterator>) {
        self.text = new_text;
        self.current = self.text.get_begin_index();
    }

    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(self.clone())
    }
}

/// `java.text.BreakIterator.isBoundary(int)`, the base class's
/// implementation: `offset == first()` or, after `checkOffset`,
/// `following(offset - 1) == offset`.
pub(crate) fn java_is_boundary(bi: &mut dyn BreakIterator, offset: i32) -> Result<bool> {
    if offset == 0 {
        return Ok(true);
    }
    let boundary = bi.following(offset - 1)?;
    if boundary == DONE {
        return Err(LuceneError::illegal_argument(""));
    }
    Ok(boundary == offset)
}