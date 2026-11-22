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

//! Port of `CharSequenceIterator.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};

/// A `CharacterIterator` over a `CharSequence`.
///
/// Equivalent to the package-private final class
/// `org.apache.lucene.search.matchhighlight.CharSequenceIterator`; the
/// sequence is UTF-16 units.
#[derive(Debug, Clone)]
pub(crate) struct CharSequenceIterator {
    text: Arc<[u16]>,

    begin: i32,
    end: i32,
    pos: i32,
}

impl CharSequenceIterator {
    pub(crate) fn new(text: Arc<[u16]>) -> Self {
        let end = text.len() as i32;
        Self {
            text,
            begin: 0,
            end,
            pos: 0,
        }
    }

    fn char_at(&self, pos: i32) -> u16 {
        self.text[pos as usize]
    }
}

impl CharacterIterator for CharSequenceIterator {
    fn first(&mut self) -> u16 {
        self.pos = self.begin;
        self.current()
    }

    fn last(&mut self) -> u16 {
        if self.end != self.begin {
            self.pos = self.end - 1;
        } else {
            self.pos = self.end;
        }
        self.current()
    }

    fn set_index(&mut self, p: i32) -> Result<u16> {
        if p < self.begin || p > self.end {
            return Err(LuceneError::illegal_argument("Invalid index"));
        }
        self.pos = p;
        Ok(self.current())
    }

    fn current(&self) -> u16 {
        if self.pos >= self.begin && self.pos < self.end {
            self.char_at(self.pos)
        } else {
            DONE
        }
    }

    fn next(&mut self) -> u16 {
        if self.pos < self.end - 1 {
            self.pos += 1;
            self.char_at(self.pos)
        } else {
            self.pos = self.end;
            DONE
        }
    }

    fn previous(&mut self) -> u16 {
        if self.pos > self.begin {
            self.pos -= 1;
            self.char_at(self.pos)
        } else {
            DONE
        }
    }

    fn get_begin_index(&self) -> i32 {
        self.begin
    }

    fn get_end_index(&self) -> i32 {
        self.end
    }

    fn get_index(&self) -> i32 {
        self.pos
    }

    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        Box::new(self.clone())
    }
}