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

//! Normalize token text with ICU's `Normalizer2`, ported from
//! `org.apache.lucene.analysis.icu.ICUNormalizer2CharFilter`.

use std::sync::Arc;

use icu4j::text::normalizer2::{self, Mode, Normalizer2};
use rucene::analysis::character_utils::code_point_at;
use rucene::analysis::{fill_buffer, new_character_buffer, CharFilter, CharReader, CharacterBuffer};
use rucene::error::{LuceneError, Result};
use rucene_analysis_common::analysis::charfilter::base_char_filter::BaseCharFilter;

/// Normalize token text with ICU's `Normalizer2`.
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUNormalizer2CharFilter`,
/// a final `BaseCharFilter`: the inherited state is the [`BaseCharFilter`]
/// this filter holds. Java's `StringBuilder`s `inputBuffer` and
/// `resultBuffer` are vectors of UTF-16 units.
#[derive(Debug)]
pub struct ICUNormalizer2CharFilter {
    base: BaseCharFilter,
    normalizer: Arc<dyn Normalizer2>,
    input_buffer: Vec<u16>,
    result_buffer: Vec<u16>,
    input_finished: bool,
    after_quick_check_yes: bool,
    checked_input_boundary: i32,
    char_count: i32,
    tmp_buffer: CharacterBuffer,
}

/// `Character.charCount(int)`.
fn char_count(cp: i32) -> i32 {
    if cp >= 0x10000 {
        2
    } else {
        1
    }
}

/// `StringBuilder.codePointBefore(int)` over `buf`.
///
/// # Panics
///
/// Where Java throws `StringIndexOutOfBoundsException`: `index` below 1 or
/// past the end.
fn code_point_before(buf: &[u16], index: usize) -> i32 {
    assert!(
        index >= 1 && index <= buf.len(),
        "StringIndexOutOfBoundsException: index {index}, length {}",
        buf.len()
    );
    let c2 = buf[index - 1];
    if (0xDC00..=0xDFFF).contains(&c2) && index >= 2 {
        let c1 = buf[index - 2];
        if (0xD800..=0xDBFF).contains(&c1) {
            return 0x10000 + ((i32::from(c1) - 0xD800) << 10) + (i32::from(c2) - 0xDC00);
        }
    }
    i32::from(c2)
}

impl ICUNormalizer2CharFilter {
    /// Create a new Normalizer2CharFilter that combines NFKC normalization,
    /// Case Folding, and removes Default Ignorables (NFKC_Casefold).
    ///
    /// Equivalent to `ICUNormalizer2CharFilter(Reader)`, over
    /// `Normalizer2.getInstance(null, "nfkc_cf", Normalizer2.Mode.COMPOSE)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the `nfkc_cf` data.
    pub fn new(input: Box<dyn CharReader>) -> Result<Self> {
        let normalizer: Arc<dyn Normalizer2> =
            normalizer2::get_instance(None, "nfkc_cf", Mode::Compose)?;
        Self::with_normalizer(input, normalizer)
    }

    /// Create a new Normalizer2CharFilter with the specified Normalizer2.
    ///
    /// * `input`: text
    /// * `normalizer`: normalizer to use
    ///
    /// Equivalent to `ICUNormalizer2CharFilter(Reader, Normalizer2)`,
    /// `this(in, normalizer, 128)`.
    ///
    /// # Errors
    ///
    /// None for the buffer size 128; see
    /// [`with_buffer_size`](Self::with_buffer_size).
    pub fn with_normalizer(input: Box<dyn CharReader>, normalizer: Arc<dyn Normalizer2>) -> Result<Self> {
        Self::with_buffer_size(input, normalizer, 128)
    }

    /// For testing ONLY.
    ///
    /// Equivalent to the package-private
    /// `ICUNormalizer2CharFilter(Reader, Normalizer2, int)`; public here so
    /// that the module's tests, which Java compiles into the same package,
    /// can reach it. Java's `Objects.requireNonNull(normalizer)` has no
    /// counterpart, an `Arc` being never null.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from
    /// `CharacterUtils.newCharacterBuffer(bufferSize)` when `buffer_size` is
    /// below 2.
    pub fn with_buffer_size(
        input: Box<dyn CharReader>,
        normalizer: Arc<dyn Normalizer2>,
        buffer_size: usize,
    ) -> Result<Self> {
        let base = BaseCharFilter::new(input);
        let tmp_buffer = new_character_buffer(buffer_size)?;
        Ok(Self {
            base,
            normalizer,
            input_buffer: Vec::new(),
            result_buffer: Vec::new(),
            input_finished: false,
            after_quick_check_yes: false,
            checked_input_boundary: 0,
            char_count: 0,
            tmp_buffer,
        })
    }

    /// The `BaseCharFilter` state this filter inherits.
    pub fn base(&self) -> &BaseCharFilter {
        &self.base
    }

    /// `readInputToBuffer()`.
    fn read_input_to_buffer(&mut self) -> Result<()> {
        // CharacterUtils.fill is supplementary char aware
        if !fill_buffer(&mut self.tmp_buffer, self.base.input_mut())? {
            self.input_finished = true;
        }

        debug_assert!(self.tmp_buffer.offset() == 0);
        let length = self.tmp_buffer.length();
        self.input_buffer
            .extend_from_slice(&self.tmp_buffer.buffer()[..length]);

        // if checkedInputBoundary was at the end of a buffer, we need to check that char again
        self.checked_input_boundary = (self.checked_input_boundary - 1).max(0);
        Ok(())
    }

    /// `readAndNormalizeFromInput()`.
    fn read_and_normalize_from_input(&mut self) -> Result<i32> {
        if self.input_buffer.is_empty() {
            self.after_quick_check_yes = false;
            return Ok(0);
        }
        if !self.after_quick_check_yes {
            let res_len = self.read_from_input_while_span_quick_check_yes()?;
            if res_len > 0 {
                return Ok(res_len);
            }
        }
        let res_len = self.read_from_io_normalize_upto_boundary()?;
        if res_len > 0 {
            self.after_quick_check_yes = false;
        }
        Ok(res_len)
    }

    /// `readFromInputWhileSpanQuickCheckYes()`.
    fn read_from_input_while_span_quick_check_yes(&mut self) -> Result<i32> {
        self.after_quick_check_yes = true;
        let mut end = self.normalizer.span_quick_check_yes(&self.input_buffer)?;
        if end > 0 {
            if end == self.input_buffer.len() as i32 {
                let mut cp = code_point_before(&self.input_buffer, end as usize);
                if !self.normalizer.has_boundary_after(cp)? {
                    /*
                    our quickCheckYes result is valid thru the end of current buffer, but we need to back off
                    because we're not at a normalization boundary. At a minimum this is relevant wrt imposing
                    canonical ordering of combining characters across the buffer boundary.
                     */
                    self.after_quick_check_yes = false;
                    end -= char_count(cp);
                    // NOTE: for the loop, we pivot to using `hasBoundaryBefore()` because per the docs for
                    // `Normalizer2.hasBoundaryAfter()`:
                    // "Note that this operation may be significantly slower than hasBoundaryBefore()"
                    while end > 0 && !self.normalizer.has_boundary_before(cp)? {
                        cp = code_point_before(&self.input_buffer, end as usize);
                        end -= char_count(cp);
                    }
                    if end == 0 {
                        return Ok(0);
                    }
                }
            }
            let end_u = end as usize;
            self.result_buffer
                .extend_from_slice(&self.input_buffer[..end_u]);
            self.input_buffer.drain(..end_u);
            self.checked_input_boundary = (self.checked_input_boundary - end).max(0);
            self.char_count += end;
        }
        Ok(end)
    }

    /// `readFromIoNormalizeUptoBoundary()`.
    fn read_from_io_normalize_upto_boundary(&mut self) -> Result<i32> {
        // if there's no buffer to normalize, return 0
        if self.input_buffer.is_empty() {
            return Ok(0);
        }

        let mut found_boundary = false;
        let buf_len = self.input_buffer.len() as i32;

        while self.checked_input_boundary <= buf_len - 1 {
            let char_len = char_count(code_point_at(
                &self.input_buffer,
                self.checked_input_boundary as usize,
                self.input_buffer.len(),
            ));
            self.checked_input_boundary += char_len;
            if self.checked_input_boundary < buf_len
                && self.normalizer.has_boundary_before(code_point_at(
                    &self.input_buffer,
                    self.checked_input_boundary as usize,
                    self.input_buffer.len(),
                ))?
            {
                found_boundary = true;
                break;
            }
        }
        if !found_boundary && self.checked_input_boundary >= buf_len && self.input_finished {
            found_boundary = true;
            self.checked_input_boundary = buf_len;
        }

        if !found_boundary {
            return Ok(0);
        }

        self.normalize_input_upto(self.checked_input_boundary)
    }

    /// `normalizeInputUpto(int)`.
    fn normalize_input_upto(&mut self, length: i32) -> Result<i32> {
        let dest_orig_len = self.result_buffer.len() as i32;
        let length_u = length as usize;
        self.normalizer
            .normalize_second_and_append(&mut self.result_buffer, &self.input_buffer[..length_u])?;
        self.input_buffer.drain(..length_u);
        self.checked_input_boundary = (self.checked_input_boundary - length).max(0);
        let result_length = self.result_buffer.len() as i32 - dest_orig_len;
        self.record_offset_diff(length, result_length);
        Ok(result_length)
    }

    /// `recordOffsetDiff(int, int)`.
    fn record_offset_diff(&mut self, input_length: i32, output_length: i32) {
        if input_length == output_length {
            self.char_count += output_length;
            return;
        }
        let diff = input_length - output_length;
        let cumu_diff = self.base.get_last_cumulative_diff();
        if diff < 0 {
            for i in 1..=-diff {
                self.base.add_off_correct_map(self.char_count + i, cumu_diff - i);
            }
        } else {
            self.base
                .add_off_correct_map(self.char_count + output_length, cumu_diff + diff);
        }
        self.char_count += output_length;
    }

    /// `outputFromResultBuffer(char[], int, int)`, over the window `cbuf`.
    fn output_from_result_buffer(&mut self, cbuf: &mut [u16]) -> usize {
        let len = self.result_buffer.len().min(cbuf.len());
        cbuf[..len].copy_from_slice(&self.result_buffer[..len]);
        if len > 0 {
            self.result_buffer.drain(..len);
        }
        len
    }
}

impl CharReader for ICUNormalizer2CharFilter {
    /// Equivalent to `ICUNormalizer2CharFilter.read(char[], int, int)` over
    /// the window `buf` (0 here where Java answers `-1`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("len <= 0")` for an empty window (Java's
    /// `off < 0` and `off >= cbuf.length` checks have no counterpart: a
    /// window has no offset), the errors of the input and of the
    /// normalizer.
    fn read(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        if cbuf.is_empty() {
            return Err(LuceneError::illegal_argument("len <= 0".to_string()));
        }

        while !self.input_finished || !self.input_buffer.is_empty() || !self.result_buffer.is_empty() {
            let mut ret_len;

            if !self.result_buffer.is_empty() {
                ret_len = self.output_from_result_buffer(cbuf);
                if ret_len > 0 {
                    return Ok(ret_len);
                }
            }

            let res_len = self.read_and_normalize_from_input()?;
            if res_len > 0 {
                ret_len = self.output_from_result_buffer(cbuf);
                if ret_len > 0 {
                    return Ok(ret_len);
                }
            }

            self.read_input_to_buffer()?;
        }

        Ok(0)
    }

    /// Equivalent to the inherited `CharFilter.close()`, `input.close()`.
    fn close(&mut self) -> Result<()> {
        self.base.input_mut().close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for ICUNormalizer2CharFilter {
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
