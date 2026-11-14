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

//! The `java.nio.ByteBuffer` the `analysis.morph` dictionaries read and
//! write: a heap buffer in big-endian order (the JDK's default), with its
//! position, limit and capacity, the relative and absolute `put`/`get`
//! methods of one byte, `short`, `char` and `int`, and `flip`/`clear`.
//!
//! Java's direct buffers (`allocateDirect`) and read-only views
//! (`asReadOnlyBuffer`) read and write the same bytes; a read-only view is a
//! shared reference here, which cannot call the `put` methods.
//!
//! The JDK's exceptions are returned as errors: `BufferOverflowException`
//! and `BufferUnderflowException` (no message), `IndexOutOfBoundsException`
//! (`Index i out of bounds for length n`, as `Buffer.checkIndex` reports it)
//! and `IllegalArgumentException` for a position or limit out of range.

use rucene::error::{LuceneError, LuceneErrorKind, Result};

/// A heap `java.nio.ByteBuffer` in big-endian order.
///
/// Equivalent to the `java.nio.HeapByteBuffer` that `ByteBuffer.allocate`
/// returns (and, for the bytes they hold, `allocateDirect` and
/// `asReadOnlyBuffer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteBuffer {
    hb: Vec<u8>,
    position: usize,
    limit: usize,
}

/// An unchecked JDK exception without a message.
fn without_message(class: &'static str) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class,
        message: None,
        cause: None,
    })
}

/// `Objects.checkIndex(index, length)`'s `IndexOutOfBoundsException`.
fn index_out_of_bounds(index: i64, length: i64) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.IndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

impl ByteBuffer {
    /// Allocates a new byte buffer of `capacity` bytes, all zero, with
    /// position 0 and limit `capacity`.
    ///
    /// Equivalent to `ByteBuffer.allocate(int)` and `ByteBuffer.allocateDirect(int)`.
    pub fn allocate(capacity: usize) -> Self {
        Self {
            hb: vec![0; capacity],
            position: 0,
            limit: capacity,
        }
    }

    /// Wraps `array`: position 0, limit and capacity its length.
    ///
    /// Equivalent to `ByteBuffer.wrap(byte[])`.
    pub fn wrap(array: Vec<u8>) -> Self {
        let limit = array.len();
        Self {
            hb: array,
            position: 0,
            limit,
        }
    }

    /// Equivalent to `Buffer.capacity()`.
    pub fn capacity(&self) -> usize {
        self.hb.len()
    }

    /// Equivalent to `Buffer.position()`.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Equivalent to `Buffer.position(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a position beyond the limit.
    pub fn set_position(&mut self, new_position: usize) -> Result<()> {
        if new_position > self.limit {
            return Err(LuceneError::illegal_argument(format!(
                "newPosition > limit: ({new_position} > {})",
                self.limit
            )));
        }
        self.position = new_position;
        Ok(())
    }

    /// Equivalent to `Buffer.limit()`.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Equivalent to `Buffer.limit(int)`: the position moves back to a new
    /// limit below it.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a limit beyond the capacity.
    pub fn set_limit(&mut self, new_limit: usize) -> Result<()> {
        if new_limit > self.hb.len() {
            return Err(LuceneError::illegal_argument(format!(
                "newLimit > capacity: ({new_limit} > {})",
                self.hb.len()
            )));
        }
        self.limit = new_limit;
        if self.position > new_limit {
            self.position = new_limit;
        }
        Ok(())
    }

    /// Equivalent to `Buffer.remaining()`.
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.position)
    }

    /// Equivalent to `Buffer.hasRemaining()`.
    pub fn has_remaining(&self) -> bool {
        self.position < self.limit
    }

    /// Equivalent to `Buffer.flip()`: the limit becomes the position, the
    /// position zero.
    pub fn flip(&mut self) {
        self.limit = self.position;
        self.position = 0;
    }

    /// Equivalent to `Buffer.clear()`: position zero, limit the capacity.
    pub fn clear(&mut self) {
        self.position = 0;
        self.limit = self.hb.len();
    }

    /// Equivalent to `Buffer.rewind()`: position zero.
    pub fn rewind(&mut self) {
        self.position = 0;
    }

    /// The backing array.
    ///
    /// Equivalent to `ByteBuffer.array()`.
    pub fn array(&self) -> &[u8] {
        &self.hb
    }

    /// The bytes from the position to the limit, the ones a channel's
    /// `write(ByteBuffer)` writes.
    pub fn remaining_bytes(&self) -> &[u8] {
        &self.hb[self.position..self.limit]
    }

    /// The next position for `n` more bytes, or `BufferOverflowException`.
    fn next_put_index(&mut self, n: usize) -> Result<usize> {
        if self.limit - self.position < n {
            return Err(without_message("java.nio.BufferOverflowException"));
        }
        let p = self.position;
        self.position += n;
        Ok(p)
    }

    /// The next position for `n` more bytes, or `BufferUnderflowException`.
    fn next_get_index(&mut self, n: usize) -> Result<usize> {
        if self.limit - self.position < n {
            return Err(without_message("java.nio.BufferUnderflowException"));
        }
        let p = self.position;
        self.position += n;
        Ok(p)
    }

    /// `Buffer.checkIndex(i, nb)`: `i` with `nb` bytes after it within the
    /// limit.
    fn check_index(&self, index: i32, nb: usize) -> Result<usize> {
        let length = self.limit as i64 - nb as i64 + 1;
        if index < 0 || index as i64 >= length {
            return Err(index_out_of_bounds(index as i64, length));
        }
        Ok(index as usize)
    }

    /// Equivalent to `ByteBuffer.put(byte)`.
    ///
    /// # Errors
    ///
    /// `BufferOverflowException` when the buffer is full.
    pub fn put(&mut self, b: i8) -> Result<()> {
        let p = self.next_put_index(1)?;
        self.hb[p] = b as u8;
        Ok(())
    }

    /// Equivalent to `ByteBuffer.put(byte[])`.
    ///
    /// # Errors
    ///
    /// `BufferOverflowException` when `src` does not fit.
    pub fn put_bytes(&mut self, src: &[u8]) -> Result<()> {
        let p = self.next_put_index(src.len())?;
        self.hb[p..p + src.len()].copy_from_slice(src);
        Ok(())
    }

    /// Equivalent to `ByteBuffer.put(ByteBuffer src)`: the remaining bytes of
    /// `src`, whose position moves to its limit.
    ///
    /// # Errors
    ///
    /// `BufferOverflowException` when they do not fit.
    pub fn put_buffer(&mut self, src: &mut ByteBuffer) -> Result<()> {
        let bytes = src.remaining_bytes().to_vec();
        self.put_bytes(&bytes)?;
        src.position = src.limit;
        Ok(())
    }

    /// Equivalent to `ByteBuffer.putShort(short)`.
    ///
    /// # Errors
    ///
    /// `BufferOverflowException` when two bytes do not fit.
    pub fn put_short(&mut self, value: i16) -> Result<()> {
        let p = self.next_put_index(2)?;
        self.hb[p..p + 2].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Equivalent to `ByteBuffer.putShort(int index, short)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException` for an index outside the limit.
    pub fn put_short_at(&mut self, index: i32, value: i16) -> Result<()> {
        let i = self.check_index(index, 2)?;
        self.hb[i..i + 2].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Equivalent to `ByteBuffer.putChar(char)`.
    ///
    /// # Errors
    ///
    /// `BufferOverflowException` when two bytes do not fit.
    pub fn put_char(&mut self, value: u16) -> Result<()> {
        let p = self.next_put_index(2)?;
        self.hb[p..p + 2].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Equivalent to `ByteBuffer.putInt(int)`.
    ///
    /// # Errors
    ///
    /// `BufferOverflowException` when four bytes do not fit.
    pub fn put_int(&mut self, value: i32) -> Result<()> {
        let p = self.next_put_index(4)?;
        self.hb[p..p + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Equivalent to `ByteBuffer.get()`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` at the limit.
    pub fn get(&mut self) -> Result<i8> {
        let p = self.next_get_index(1)?;
        Ok(self.hb[p] as i8)
    }

    /// Equivalent to `ByteBuffer.get(int index)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException` for an index outside the limit.
    pub fn get_at(&self, index: i32) -> Result<i8> {
        let i = self.check_index(index, 1)?;
        Ok(self.hb[i] as i8)
    }

    /// Equivalent to `ByteBuffer.getShort()`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` with fewer than two bytes left.
    pub fn get_short(&mut self) -> Result<i16> {
        let p = self.next_get_index(2)?;
        Ok(i16::from_be_bytes([self.hb[p], self.hb[p + 1]]))
    }

    /// Equivalent to `ByteBuffer.getShort(int index)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException` for an index outside the limit.
    pub fn get_short_at(&self, index: i32) -> Result<i16> {
        let i = self.check_index(index, 2)?;
        Ok(i16::from_be_bytes([self.hb[i], self.hb[i + 1]]))
    }

    /// Equivalent to `ByteBuffer.getChar(int index)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException` for an index outside the limit.
    pub fn get_char_at(&self, index: i32) -> Result<u16> {
        let i = self.check_index(index, 2)?;
        Ok(u16::from_be_bytes([self.hb[i], self.hb[i + 1]]))
    }

    /// Equivalent to `ByteBuffer.getInt(int index)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException` for an index outside the limit.
    pub fn get_int_at(&self, index: i32) -> Result<i32> {
        let i = self.check_index(index, 4)?;
        Ok(i32::from_be_bytes([
            self.hb[i],
            self.hb[i + 1],
            self.hb[i + 2],
            self.hb[i + 3],
        ]))
    }
}
