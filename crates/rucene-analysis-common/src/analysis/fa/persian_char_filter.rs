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

//! A `CharFilter` that replaces instances of Zero-width non-joiner with an
//! ordinary space, ported from `org.apache.lucene.analysis.fa.PersianCharFilter`.

use rucene::analysis::{CharFilter, CharReader};
use rucene::error::Result;

/// CharFilter that replaces instances of Zero-width non-joiner with an
/// ordinary space.
///
/// Equivalent to `org.apache.lucene.analysis.fa.PersianCharFilter`, which
/// extends `CharFilter`: the input is the reader this filter holds.
#[derive(Debug)]
pub struct PersianCharFilter {
    input: Box<dyn CharReader>,
}

impl PersianCharFilter {
    /// Equivalent to `PersianCharFilter(Reader)`.
    pub fn new(input: Box<dyn CharReader>) -> Self {
        Self { input }
    }

    /// optimized impl: some other charfilters consume with read()
    ///
    /// Equivalent to `PersianCharFilter.read()`: the next code unit of the
    /// input, a space for U+200C, or `-1` at the end.
    ///
    /// # Errors
    ///
    /// What the input raises.
    pub fn read_char(&mut self) -> Result<i32> {
        // `input.read()`: Java's `Reader.read()` reads one unit through
        // `read(char[], int, int)`.
        let mut one = [0u16; 1];
        let ch = if self.input.read(&mut one)? == 0 {
            -1
        } else {
            i32::from(one[0])
        };
        if ch == 0x200C {
            Ok(i32::from(b' '))
        } else {
            Ok(ch)
        }
    }
}

impl CharReader for PersianCharFilter {
    /// Equivalent to `PersianCharFilter.read(char[], int, int)`: the units
    /// the input reads into `buf`, each U+200C replaced by a space.
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        let chars_read = self.input.read(buf)?;
        for c in &mut buf[..chars_read] {
            if *c == 0x200C {
                *c = u16::from(b' ');
            }
        }
        Ok(chars_read)
    }

    /// Equivalent to the inherited `CharFilter.close()`, which closes the
    /// input.
    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for PersianCharFilter {
    /// Equivalent to `PersianCharFilter.correct(int)`.
    fn correct(&self, current_off: i32) -> i32 {
        current_off // we don't change the length of the string
    }

    fn input(&self) -> &dyn CharReader {
        self.input.as_ref()
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        self.input.as_mut()
    }
}
