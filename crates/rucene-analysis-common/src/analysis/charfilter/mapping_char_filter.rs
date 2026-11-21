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

//! Simplistic `CharFilter` that applies the mappings contained in a
//! `NormalizeCharMap` to the character stream, ported from
//! `org.apache.lucene.analysis.charfilter.MappingCharFilter`.

use std::sync::Arc;

use rucene::analysis::{CharFilter, CharReader};
use rucene::error::Result;
use rucene::util::fst::{target_has_arcs, Arc as FSTArc, CharSequenceOutputs, Outputs};
use rucene::util::CharsRef;

use super::base_char_filter::BaseCharFilter;
use super::normalize_char_map::NormalizeCharMap;
use crate::analysis::util::rolling_char_buffer::RollingCharBuffer;

/// Simplistic `CharFilter` that applies the mappings contained in a
/// [`NormalizeCharMap`] to the character stream, and correcting the resulting
/// changes to the offsets. Matching is greedy (longest pattern matching at a
/// given point wins). Replacement is allowed to be the empty string.
///
/// Equivalent to `org.apache.lucene.analysis.charfilter.MappingCharFilter`,
/// a `BaseCharFilter`: the [`BaseCharFilter`] it holds carries the input and
/// the offset corrections.
///
/// **Divergence from Lucene 10.5.0.** Java takes one `FST.BytesReader` from
/// the map when it is built and keeps it in the `fstReader` field. `rucene`'s
/// `BytesReader` is neither `Send` nor `Sync`, which a
/// [`CharReader`] must be, so the filter takes a reader from the map at each
/// [`read_char`](Self::read_char) that walks the FST beyond its first
/// character. `findTargetArc` positions the reader before every read, so no
/// state of the reader carries from one call to the next, and the matches
/// are the same.
#[derive(Debug)]
pub struct MappingCharFilter {
    /// The `BaseCharFilter` (and `CharFilter`) state: the input and the offset
    /// corrections.
    base: BaseCharFilter,
    outputs: CharSequenceOutputs,
    /// The map: Java's `map` and `cachedRootArcs` fields, both read from it.
    norm_map: Arc<NormalizeCharMap>,
    buffer: RollingCharBuffer,
    scratch_arc: FSTArc<CharsRef>,
    replacement: Option<CharsRef>,
    replacement_pointer: usize,
    input_off: i32,
}

impl MappingCharFilter {
    /// Default constructor that takes a `Reader`.
    ///
    /// Equivalent to `MappingCharFilter(NormalizeCharMap, Reader)`.
    pub fn new(norm_map: Arc<NormalizeCharMap>, input: Box<dyn CharReader>) -> Self {
        let mut buffer = RollingCharBuffer::new();
        buffer.reset();
        Self {
            base: BaseCharFilter::new(input),
            outputs: CharSequenceOutputs::get_singleton(),
            norm_map,
            buffer,
            scratch_arc: FSTArc::default(),
            replacement: None,
            replacement_pointer: 0,
            input_off: 0,
        }
    }

    /// Reads one UTF-16 code unit, or `-1` at the end of the stream.
    ///
    /// Equivalent to `MappingCharFilter.read()`.
    ///
    /// # Errors
    ///
    /// What the input and the FST raise.
    pub fn read_char(&mut self) -> Result<i32> {
        // System.out.println("\nread");
        loop {
            if let Some(replacement) = &self.replacement {
                if self.replacement_pointer < replacement.length {
                    // System.out.println("  return repl[" + replacementPointer + "]=" +
                    // replacement.chars[replacement.offset + replacementPointer]);
                    let c = replacement.chars[replacement.offset + self.replacement_pointer];
                    self.replacement_pointer += 1;
                    return Ok(i32::from(c));
                }
            }

            // TODO: a more efficient approach would be Aho/Corasick's
            // algorithm
            // (http://en.wikipedia.org/wiki/Aho%E2%80%93Corasick_string_matching_algorithm)
            // or this generalizatio: www.cis.uni-muenchen.de/people/Schulz/Pub/dictle5.ps
            //
            // I think this would be (almost?) equivalent to 1) adding
            // epsilon arcs from all final nodes back to the init
            // node in the FST, 2) adding a .* (skip any char)
            // loop on the initial node, and 3) determinizing
            // that.  Then we would not have to restart matching
            // at each position.

            let mut last_match_len: i32 = -1;
            let mut last_match: Option<CharsRef> = None;

            let first_ch = self.buffer.get(self.input_off, self.base.input_mut())?;
            if first_ch != -1 {
                let norm_map = Arc::clone(&self.norm_map);
                if let Some(cached_arc) = norm_map.cached_root_arcs.get(first_ch as u16) {
                    if !target_has_arcs(cached_arc) {
                        // Fast pass for single character match:
                        debug_assert!(cached_arc.is_final());
                        last_match_len = 1;
                        last_match = Some(cached_arc.output().clone());
                    } else {
                        let map = norm_map
                            .map
                            .as_ref()
                            .expect("INVARIANT: a cached root arc comes from the map");
                        let mut fst_reader = map.get_bytes_reader()?;
                        // `arc` is the cached root arc, then `scratchArc`
                        // (`findTargetArc` fills it in place).
                        self.scratch_arc.copy_from(cached_arc);
                        let mut lookahead = 0;
                        let mut output = self.scratch_arc.output().clone();
                        loop {
                            lookahead += 1;

                            if self.scratch_arc.is_final() {
                                // Match! (to node is final)
                                last_match_len = lookahead;
                                last_match = Some(
                                    self.outputs
                                        .add(&output, self.scratch_arc.next_final_output()),
                                );
                                // Greedy: keep searching to see if there's a
                                // longer match...
                            }

                            if !target_has_arcs(&self.scratch_arc) {
                                break;
                            }

                            let ch = self
                                .buffer
                                .get(self.input_off + lookahead, self.base.input_mut())?;
                            if ch == -1 {
                                break;
                            }
                            if !map.find_target_arc_in_place(
                                ch,
                                &mut self.scratch_arc,
                                &mut *fst_reader,
                            )? {
                                // Dead end
                                break;
                            }
                            output = self.outputs.add(&output, self.scratch_arc.output());
                        }
                    }
                }
            }

            if let Some(last_match) = last_match {
                self.input_off += last_match_len;
                // System.out.println("  match!  len=" + lastMatchLen + " repl=" + lastMatch);

                let diff = last_match_len - last_match.length as i32;

                if diff != 0 {
                    let prev_cumulative_diff = self.base.get_last_cumulative_diff();
                    if diff > 0 {
                        // Replacement is shorter than matched input:
                        self.base.add_off_correct_map(
                            self.input_off - diff - prev_cumulative_diff,
                            prev_cumulative_diff + diff,
                        );
                    } else {
                        // Replacement is longer than matched input: remap
                        // the "extra" chars all back to the same input
                        // offset:
                        let output_start = self.input_off - prev_cumulative_diff;
                        for extra_idx in 0..-diff {
                            self.base.add_off_correct_map(
                                output_start + extra_idx,
                                prev_cumulative_diff - extra_idx - 1,
                            );
                        }
                    }
                }

                self.replacement = Some(last_match);
                self.replacement_pointer = 0;
            } else {
                let ret = self.buffer.get(self.input_off, self.base.input_mut())?;
                if ret != -1 {
                    self.input_off += 1;
                    self.buffer.free_before(self.input_off);
                }
                return Ok(ret);
            }
        }
    }
}

impl CharReader for MappingCharFilter {
    /// Equivalent to `MappingCharFilter.read(char[], int, int)`: the units
    /// [`read_char`](MappingCharFilter::read_char) returns, until `buf` is
    /// full or the stream ends (0 here where Java answers `-1`).
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

    /// Equivalent to the inherited `CharFilter.close()`, `input.close()`.
    fn close(&mut self) -> Result<()> {
        self.base.input_mut().close()
    }

    /// Equivalent to `MappingCharFilter.reset()`: the input is reset, and the
    /// filter reads it again from the start; the recorded offset corrections
    /// stay, as Java's do.
    fn reset(&mut self) -> Result<()> {
        self.base.input_mut().reset()?;
        self.buffer.reset();
        self.replacement = None;
        self.input_off = 0;
        Ok(())
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for MappingCharFilter {
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
