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

//! Port of `org.apache.lucene.tests.analysis.MockCharFilter`.

use std::collections::BTreeMap;

use rucene::analysis::{CharFilter, CharReader};
use rucene::error::{LuceneError, Result};

/// The purpose of this charfilter is to send offsets out of bounds if the
/// analyzer doesn't use correctOffset or does incorrect offset math.
///
/// Every character whose code modulo 10 equals `remainder` is doubled, and an
/// offset correction is recorded for the inserted copy.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockCharFilter`: it reads
/// one UTF-16 code unit at a time and never doubles a surrogate.
#[derive(Debug)]
pub struct MockCharFilter {
    input: Box<dyn CharReader>,
    remainder: i32,
    current_offset: i32,
    delta: i32,
    buffered_ch: Option<u16>,
    corrections: BTreeMap<i32, i32>,
}

impl MockCharFilter {
    /// For testing only.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`invalid remainder parameter (must be
    /// 0..10): N`) unless `0 <= remainder < 10`.
    pub fn new(input: Box<dyn CharReader>, remainder: i32) -> Result<MockCharFilter> {
        // TODO: instead of fixed remainder... maybe a fixed
        // random seed?
        if !(0..10).contains(&remainder) {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid remainder parameter (must be 0..10): {}",
                remainder
            )));
        }
        Ok(MockCharFilter {
            input,
            remainder,
            current_offset: -1,
            delta: 0,
            buffered_ch: None,
            corrections: BTreeMap::new(),
        })
    }

    /// For testing only, uses a remainder of 0.
    pub fn with_default_remainder(input: Box<dyn CharReader>) -> MockCharFilter {
        Self::new(input, 0).expect("INVARIANT: 0 is a valid remainder")
    }

    /// `read()`: the next UTF-16 code unit, or `None` (Java's `-1`) at the
    /// end of the input.
    fn read_one(&mut self) -> Result<Option<u16>> {
        // we have a buffered character, add an offset correction and return it
        if let Some(ch) = self.buffered_ch.take() {
            self.current_offset += 1;
            self.add_off_correct_map(self.current_offset, self.delta - 1);
            self.delta -= 1;
            return Ok(Some(ch));
        }

        // otherwise actually read one
        let mut buf = [0u16; 1];
        if self.input.read(&mut buf)? == 0 {
            return Ok(None);
        }
        let ch = buf[0];
        self.current_offset += 1;
        if (i32::from(ch) % 10) != self.remainder
            || (0xD800..=0xDBFF).contains(&ch)
            || (0xDC00..=0xDFFF).contains(&ch)
        {
            return Ok(Some(ch));
        }

        // we will double this character, so buffer it.
        self.buffered_ch = Some(ch);
        Ok(Some(ch))
    }

    /// `addOffCorrectMap(int off, int cumulativeDiff)`.
    fn add_off_correct_map(&mut self, off: i32, cumulative_diff: i32) {
        self.corrections.insert(off, cumulative_diff);
    }
}

impl CharReader for MockCharFilter {
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        let mut num_read = 0;
        for slot in buf.iter_mut() {
            match self.read_one()? {
                Some(c) => {
                    *slot = c;
                    num_read += 1;
                }
                None => break,
            }
        }
        Ok(num_read)
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for MockCharFilter {
    /// `correct(int)`: the offset plus the correction of the greatest
    /// recorded offset `<= currentOff`.
    fn correct(&self, current_off: i32) -> i32 {
        let ret = match self.corrections.range(..=current_off).next_back() {
            None => current_off,
            Some((_, diff)) => current_off + diff,
        };
        java_assert_unchecked!(
            ret >= 0,
            "currentOff={},diff={}",
            current_off,
            ret - current_off
        );
        ret
    }

    fn input(&self) -> &dyn CharReader {
        self.input.as_ref()
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        self.input.as_mut()
    }
}
