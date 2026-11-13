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

//! A forever-growing `char[]` over a reader, ported from
//! `org.apache.lucene.analysis.util.RollingCharBuffer`.

use rucene::analysis::CharReader;
use rucene::error::Result;
use rucene::util::ArrayUtil;

/// Acts like a forever growing `char[]` as you read characters into it from
/// the provided reader, but internally it uses a circular buffer to only hold
/// the characters that haven't been freed yet. This is like a
/// `PushbackReader`, except you don't have to specify up-front the max size
/// of the buffer, but you do have to periodically call
/// [`free_before`](Self::free_before).
///
/// Equivalent to `org.apache.lucene.analysis.util.RollingCharBuffer`. The
/// buffer holds UTF-16 units, as Java's `char[]` does.
///
/// **Divergence from Lucene 10.5.0.** Java's `reset(Reader)` keeps the
/// reader, and `get(int)` reads from it. Its callers are tokenizers, which
/// hand it their own `input`; in `rucene` a `Tokenizer` owns its input and
/// lends it to its logic for the length of each call, so the buffer cannot
/// keep it. [`reset`](Self::reset) therefore takes no reader, and
/// [`get`](Self::get) is handed the reader to read from — the tokenizer's
/// input, the reader Java's `reset` would have kept.
#[derive(Clone, Debug)]
pub struct RollingCharBuffer {
    buffer: Vec<u16>,

    /// Next array index to write to in buffer:
    next_write: usize,

    /// Next absolute position to read from reader:
    next_pos: i32,

    /// How many valid chars (wrapped) are in the buffer:
    count: usize,

    /// True if we hit EOF
    end: bool,
}

impl Default for RollingCharBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl RollingCharBuffer {
    /// Creates an empty buffer of 512 units, the size of Java's initial
    /// `new char[512]`.
    pub fn new() -> Self {
        Self {
            buffer: vec![0; 512],
            next_write: 0,
            next_pos: 0,
            count: 0,
            end: false,
        }
    }

    /// Clear array and switch to new reader.
    ///
    /// Equivalent to `reset(Reader)`; see the type's documentation for the
    /// reader.
    pub fn reset(&mut self) {
        self.next_pos = 0;
        self.next_write = 0;
        self.count = 0;
        self.end = false;
    }

    /// Absolute position read. NOTE: `pos` must not jump ahead by more than
    /// 1! Ie, it's OK to read arbitarily far back (just not prior to the last
    /// [`free_before`](Self::free_before)), but NOT ok to read arbitrarily
    /// far ahead. Returns -1 if you hit EOF.
    ///
    /// Equivalent to `get(int)`, reading from `reader` (see the type's
    /// documentation).
    ///
    /// # Errors
    ///
    /// Returns what `reader` returns.
    pub fn get(&mut self, pos: i32, reader: &mut dyn CharReader) -> Result<i32> {
        if pos == self.next_pos {
            if self.end {
                return Ok(-1);
            }
            if self.count == self.buffer.len() {
                // Grow
                let mut new_buffer = vec![0u16; ArrayUtil::oversize(1 + self.count, 2)];
                let tail = self.buffer.len() - self.next_write;
                new_buffer[..tail].copy_from_slice(&self.buffer[self.next_write..]);
                new_buffer[tail..tail + self.next_write]
                    .copy_from_slice(&self.buffer[..self.next_write]);
                self.next_write = self.buffer.len();
                self.buffer = new_buffer;
            }
            if self.next_write == self.buffer.len() {
                self.next_write = 0;
            }

            let to_read = self.buffer.len() - self.count.max(self.next_write);
            let read_count =
                reader.read(&mut self.buffer[self.next_write..self.next_write + to_read])?;
            if read_count == 0 {
                // `readCount == -1`: a `CharReader` answers 0 at the end of
                // the stream, and `toRead` is never 0 here.
                self.end = true;
                return Ok(-1);
            }
            let ch = i32::from(self.buffer[self.next_write]);
            self.next_write += read_count;
            self.count += read_count;
            self.next_pos += read_count as i32;
            Ok(ch)
        } else {
            // Cannot read from future (except by 1):
            debug_assert!(pos < self.next_pos);

            // Cannot read from already freed past:
            debug_assert!(
                (self.next_pos - pos) as usize <= self.count,
                "nextPos={} pos={} count={}",
                self.next_pos,
                pos,
                self.count
            );

            Ok(i32::from(self.buffer[self.get_index(pos)]))
        }
    }

    /// For assert: `inBounds(int)`.
    fn in_bounds(&self, pos: i32) -> bool {
        pos >= 0 && pos < self.next_pos && pos >= self.next_pos - self.count as i32
    }

    /// `getIndex(int)`.
    fn get_index(&self, pos: i32) -> usize {
        let mut index = self.next_write as i64 - i64::from(self.next_pos - pos);
        if index < 0 {
            // Wrap:
            index += self.buffer.len() as i64;
            debug_assert!(index >= 0);
        }
        index as usize
    }

    /// Returns the `length` units from the absolute position `pos_start`.
    ///
    /// Equivalent to `get(int, int)`.
    pub fn get_range(&self, pos_start: i32, length: usize) -> Vec<u16> {
        debug_assert!(length > 0);
        debug_assert!(
            self.in_bounds(pos_start),
            "posStart={pos_start} length={length}"
        );

        let start_index = self.get_index(pos_start);
        let end_index = self.get_index(pos_start + length as i32);

        let mut result = vec![0u16; length];
        if end_index >= start_index && length < self.buffer.len() {
            result[..end_index - start_index].copy_from_slice(&self.buffer[start_index..end_index]);
        } else {
            // Wrapped:
            let part1 = self.buffer.len() - start_index;
            result[..part1].copy_from_slice(&self.buffer[start_index..]);
            result[self.buffer.len() - start_index..]
                .copy_from_slice(&self.buffer[..length - part1]);
        }
        result
    }

    /// Call this to notify us that no chars before this absolute position are
    /// needed anymore.
    ///
    /// Equivalent to `freeBefore(int)`.
    pub fn free_before(&mut self, pos: i32) {
        debug_assert!(pos >= 0);
        debug_assert!(pos <= self.next_pos);
        let new_count = (self.next_pos - pos) as usize;
        debug_assert!(
            new_count <= self.count,
            "newCount={new_count} count={}",
            self.count
        );
        debug_assert!(
            new_count <= self.buffer.len(),
            "newCount={new_count} buf.length={}",
            self.buffer.len()
        );
        self.count = new_count;
    }
}
