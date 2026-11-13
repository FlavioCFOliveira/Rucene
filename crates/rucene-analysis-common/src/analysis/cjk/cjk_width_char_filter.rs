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

//! A `CharFilter` that normalizes CJK width differences, ported from
//! `org.apache.lucene.analysis.cjk.CJKWidthCharFilter`.

use rucene::analysis::{CharFilter, CharReader};
use rucene::error::Result;

use crate::analysis::charfilter::base_char_filter::BaseCharFilter;

/* halfwidth kana mappings: 0xFF65-0xFF9D
 *
 * note: 0xFF9C and 0xFF9D are only mapped to 0x3099 and 0x309A
 * as a fallback when they cannot properly combine with a preceding
 * character into a composed form.
 */
/// `CJKWidthCharFilter.KANA_NORM`.
#[rustfmt::skip]
const KANA_NORM: [u16; 59] = [
    0x30fb, 0x30f2, 0x30a1, 0x30a3, 0x30a5, 0x30a7, 0x30a9, 0x30e3, 0x30e5,
    0x30e7, 0x30c3, 0x30fc, 0x30a2, 0x30a4, 0x30a6, 0x30a8, 0x30aa, 0x30ab,
    0x30ad, 0x30af, 0x30b1, 0x30b3, 0x30b5, 0x30b7, 0x30b9, 0x30bb, 0x30bd,
    0x30bf, 0x30c1, 0x30c4, 0x30c6, 0x30c8, 0x30ca, 0x30cb, 0x30cc, 0x30cd,
    0x30ce, 0x30cf, 0x30d2, 0x30d5, 0x30d8, 0x30db, 0x30de, 0x30df, 0x30e0,
    0x30e1, 0x30e2, 0x30e4, 0x30e6, 0x30e8, 0x30e9, 0x30ea, 0x30eb, 0x30ec,
    0x30ed, 0x30ef, 0x30f3, 0x3099, 0x309A,
];

/* kana combining diffs: 0x30A6-0x30FD */
/// `CJKWidthCharFilter.KANA_COMBINE_VOICED`.
#[rustfmt::skip]
const KANA_COMBINE_VOICED: [i8; 88] = [
    78, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
    0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 1,
    0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 8, 8, 8, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
];

/// `CJKWidthCharFilter.KANA_COMBINE_SEMI_VOICED`.
#[rustfmt::skip]
const KANA_COMBINE_SEMI_VOICED: [i8; 88] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 2, 0, 0, 2,
    0, 0, 2, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// `CJKWidthCharFilter.HW_KATAKANA_VOICED_MARK`.
const HW_KATAKANA_VOICED_MARK: i32 = 0xFF9E;

/// `CJKWidthCharFilter.HW_KATAKANA_SEMI_VOICED_MARK`.
const HW_KATAKANA_SEMI_VOICED_MARK: i32 = 0xFF9F;

/// A `CharFilter` that normalizes CJK width differences:
///
/// * Folds fullwidth ASCII variants into the equivalent basic latin
/// * Folds halfwidth Katakana variants into the equivalent kana
///
/// NOTE: this char filter is the exact counterpart of
/// [`CJKWidthFilter`](super::cjk_width_filter::CJKWidthFilter).
///
/// Equivalent to `org.apache.lucene.analysis.cjk.CJKWidthCharFilter`, which
/// extends `BaseCharFilter`: the base is the [`BaseCharFilter`] this filter
/// holds.
#[derive(Debug)]
pub struct CJKWidthCharFilter {
    base: BaseCharFilter,
    prev_char: i32,
    input_off: i32,
}

impl CJKWidthCharFilter {
    /// Default constructor that takes a `Reader`.
    ///
    /// Equivalent to `CJKWidthCharFilter(Reader)`.
    pub fn new(input: Box<dyn CharReader>) -> Self {
        Self {
            base: BaseCharFilter::new(input),
            prev_char: -1,
            input_off: 0,
        }
    }

    /// `input.read()`: the next code unit of the input, or `-1` at its end.
    fn read_input(&mut self) -> Result<i32> {
        let mut one = [0u16; 1];
        if self.base.input_mut().read(&mut one)? == 0 {
            return Ok(-1);
        }
        Ok(i32::from(one[0]))
    }

    /// Reads one unit: the next normalized code unit, or `-1` at the end.
    ///
    /// Equivalent to `CJKWidthCharFilter.read()`.
    ///
    /// # Errors
    ///
    /// What the input raises.
    pub fn read_char(&mut self) -> Result<i32> {
        loop {
            let ch = self.read_input()?;
            if ch == -1 {
                // reached end of the input
                let ret = self.prev_char;
                self.prev_char = ch;
                return Ok(ret);
            }

            self.input_off += 1;
            let mut ret = -1;
            // if the current char is a voice mark, then try to combine it with the previous char.
            if ch == HW_KATAKANA_SEMI_VOICED_MARK || ch == HW_KATAKANA_VOICED_MARK {
                let combined_char = self.combine_voice_mark(self.prev_char, ch);
                if self.prev_char != combined_char {
                    // successfully combined. returns the combined char immediately
                    self.prev_char = -1;
                    // offset needs to be corrected
                    let prev_cumulative_diff = self.base.get_last_cumulative_diff();
                    self.base.add_off_correct_map(
                        self.input_off - 1 - prev_cumulative_diff,
                        prev_cumulative_diff + 1,
                    );
                    return Ok(combined_char);
                }
            }

            if self.prev_char != -1 {
                ret = self.prev_char;
            }

            if (0xFF01..=0xFF5E).contains(&ch) {
                // Fullwidth ASCII variants
                self.prev_char = ch - 0xFEE0;
            } else if (0xFF65..=0xFF9F).contains(&ch) {
                // Halfwidth Katakana variants
                self.prev_char = i32::from(KANA_NORM[(ch - 0xFF65) as usize]);
            } else {
                // no need to normalize
                self.prev_char = ch;
            }

            if ret != -1 {
                return Ok(ret);
            }
        }
    }

    /// returns combined char if we successfully combined the voice mark,
    /// otherwise original char
    ///
    /// Equivalent to the private `combineVoiceMark(int, int)`, which indexes
    /// its tables with the field `prevChar` rather than its parameter `ch`;
    /// every caller passes `prevChar` as `ch`, and so does this port.
    fn combine_voice_mark(&self, mut ch: i32, voice_mark: i32) -> i32 {
        debug_assert!(
            voice_mark == HW_KATAKANA_SEMI_VOICED_MARK || voice_mark == HW_KATAKANA_VOICED_MARK
        );
        if (0x30A6..=0x30FD).contains(&ch) {
            ch += i32::from(if voice_mark == HW_KATAKANA_SEMI_VOICED_MARK {
                KANA_COMBINE_SEMI_VOICED[(self.prev_char - 0x30A6) as usize]
            } else {
                KANA_COMBINE_VOICED[(self.prev_char - 0x30A6) as usize]
            });
        }
        ch
    }
}

impl CharReader for CJKWidthCharFilter {
    /// Equivalent to `CJKWidthCharFilter.read(char[], int, int)`: one
    /// [`read_char`](CJKWidthCharFilter::read_char) per unit, until the end
    /// of the input.
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        let mut num_read = 0;
        for slot in buf.iter_mut() {
            let c = self.read_char()?;
            if c == -1 {
                break;
            }
            *slot = c as u16;
            num_read += 1;
        }
        Ok(num_read)
    }

    /// Equivalent to the inherited `CharFilter.close()`, which closes the
    /// input.
    fn close(&mut self) -> Result<()> {
        self.base.input_mut().close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for CJKWidthCharFilter {
    /// Equivalent to the inherited `BaseCharFilter.correct(int)`.
    fn correct(&self, current_off: i32) -> i32 {
        self.base.correct(current_off)
    }

    fn input(&self) -> &dyn CharReader {
        self.base.input()
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        self.base.input_mut()
    }
}
