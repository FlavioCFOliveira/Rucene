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

//! Port of `BreakIteratorBoundaryScanner.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::sync::{Mutex, MutexGuard, PoisonError};

use rucene_analysis_common::jdk::break_iterator::BreakIterator;

use crate::vectorhighlight::boundary_scanner::BoundaryScanner;

/// A `BoundaryScanner` implementation that uses `BreakIterator` to find
/// boundaries in the text.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.BreakIteratorBoundaryScanner`.
/// The iterator, which each scan resets, is behind a lock so the scanner is
/// shared as Java shares it.
#[derive(Debug)]
pub struct BreakIteratorBoundaryScanner {
    pub(crate) bi: Mutex<Box<dyn BreakIterator>>,
}

impl BreakIteratorBoundaryScanner {
    /// Equivalent to `new BreakIteratorBoundaryScanner(BreakIterator)`.
    pub fn new(bi: Box<dyn BreakIterator>) -> Self {
        Self { bi: Mutex::new(bi) }
    }

    fn bi(&self) -> MutexGuard<'_, Box<dyn BreakIterator>> {
        self.bi.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl BoundaryScanner for BreakIteratorBoundaryScanner {
    fn find_start_offset(&self, buffer: &[u16], start: i32) -> i32 {
        // avoid illegal start offset
        if start > buffer.len() as i32 || start < 1 {
            return start;
        }
        let mut bi = self.bi();
        bi.set_text_units(&buffer[..start as usize]);
        bi.last();
        bi.previous()
    }

    fn find_end_offset(&self, buffer: &[u16], start: i32) -> i32 {
        // avoid illegal start offset
        if start > buffer.len() as i32 || start < 0 {
            return start;
        }
        let mut bi = self.bi();
        bi.set_text_units(&buffer[start as usize..]);
        bi.next() + start
    }
}