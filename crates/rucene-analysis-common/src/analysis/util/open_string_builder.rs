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

//! A `StringBuilder` that allows one to access the array, ported from
//! `org.apache.lucene.analysis.util.OpenStringBuilder`.

/// A `StringBuilder` that allows one to access the array.
///
/// Equivalent to `org.apache.lucene.analysis.util.OpenStringBuilder`, which
/// implements `Appendable` and `CharSequence`. The buffer holds UTF-16
/// units (`u16`), as Java's `char[]` does; the `CharSequence` arguments of
/// the `append` methods are UTF-16 slices. Java's `protected` fields `buf`
/// and `len` are reached through [`get_array`](Self::get_array),
/// [`set`](Self::set) and [`set_length`](Self::set_length).
///
/// Writing past the capacity with an `unsafe_write` method panics, where
/// Java throws `ArrayIndexOutOfBoundsException`.
#[derive(Clone, Debug)]
pub struct OpenStringBuilder {
    buf: Vec<u16>,
    len: usize,
}

impl Default for OpenStringBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenStringBuilder {
    /// Equivalent to `OpenStringBuilder()`, `this(32)`.
    pub fn new() -> Self {
        Self::with_size(32)
    }

    /// Equivalent to `OpenStringBuilder(int)`: an empty builder of capacity
    /// `size`.
    pub fn with_size(size: usize) -> Self {
        Self {
            buf: vec![0; size],
            len: 0,
        }
    }

    /// Equivalent to `OpenStringBuilder(char[], int)`, `set(arr, len)`.
    pub fn from_array(arr: Vec<u16>, len: usize) -> Self {
        let mut builder = Self {
            buf: Vec::new(),
            len: 0,
        };
        builder.set(arr, len);
        builder
    }

    /// Equivalent to `setLength(int)`.
    pub fn set_length(&mut self, len: usize) {
        self.len = len;
    }

    /// Equivalent to `set(char[], int)`: `arr` becomes the buffer and
    /// `end` the length.
    pub fn set(&mut self, arr: Vec<u16>, end: usize) {
        self.buf = arr;
        self.len = end;
    }

    /// Equivalent to `getArray()`.
    pub fn get_array(&self) -> &[u16] {
        &self.buf
    }

    /// Equivalent to `getArray()`, for a caller that writes into the array
    /// as Java's callers of the shared `char[]` may.
    pub fn get_array_mut(&mut self) -> &mut [u16] {
        &mut self.buf
    }

    /// Equivalent to `size()`.
    pub fn size(&self) -> usize {
        self.len
    }

    /// Equivalent to `length()`.
    pub fn length(&self) -> usize {
        self.len
    }

    /// Equivalent to `capacity()`.
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Equivalent to `append(CharSequence)`, `append(csq, 0, csq.length())`.
    pub fn append(&mut self, csq: &[u16]) -> &mut Self {
        self.append_range(csq, 0, csq.len())
    }

    /// Equivalent to `append(CharSequence, int, int)`: reserves
    /// `end - start` units and writes the units of `csq` from `start` to
    /// `end`.
    pub fn append_range(&mut self, csq: &[u16], start: usize, end: usize) -> &mut Self {
        self.reserve(end - start);
        for &c in &csq[start..end] {
            self.unsafe_write(c);
        }
        self
    }

    /// Equivalent to `append(char)`, `write(c)`.
    pub fn append_char(&mut self, c: u16) -> &mut Self {
        self.write(c);
        self
    }

    /// Equivalent to `charAt(int)`.
    pub fn char_at(&self, index: usize) -> u16 {
        self.buf[index]
    }

    /// Equivalent to `setCharAt(int, char)`.
    pub fn set_char_at(&mut self, index: usize, ch: u16) {
        self.buf[index] = ch;
    }

    /// Equivalent to `subSequence(int, int)`, which Java leaves
    /// unimplemented (`// todo`).
    ///
    /// # Panics
    ///
    /// Always, with Java's `UnsupportedOperationException`.
    pub fn sub_sequence(&self, _start: usize, _end: usize) -> Vec<u16> {
        rucene::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }

    /// Equivalent to `unsafeWrite(char)`: `buf[len++] = b`, without a
    /// capacity check.
    pub fn unsafe_write(&mut self, b: u16) {
        self.buf[self.len] = b;
        self.len += 1;
    }

    /// Equivalent to `unsafeWrite(int)`, `unsafeWrite((char) b)`.
    pub fn unsafe_write_int(&mut self, b: i32) {
        self.unsafe_write(b as u16);
    }

    /// Equivalent to `unsafeWrite(char[], int, int)`: copies `len` units of
    /// `b` from `off`, without a capacity check.
    pub fn unsafe_write_slice(&mut self, b: &[u16], off: usize, len: usize) {
        self.buf[self.len..self.len + len].copy_from_slice(&b[off..off + len]);
        self.len += len;
    }

    /// Equivalent to the protected `resize(int)`: a buffer of
    /// `max(capacity * 2, len)` units holding the current content.
    pub fn resize(&mut self, len: usize) {
        let mut newbuf = vec![0u16; (self.buf.len() << 1).max(len)];
        newbuf[..self.size()].copy_from_slice(&self.buf[..self.size()]);
        self.buf = newbuf;
    }

    /// Equivalent to `reserve(int)`.
    pub fn reserve(&mut self, num: usize) {
        if self.len + num > self.buf.len() {
            self.resize(self.len + num);
        }
    }

    /// Equivalent to `write(char)`.
    pub fn write(&mut self, b: u16) {
        if self.len >= self.buf.len() {
            self.resize(self.len + 1);
        }
        self.unsafe_write(b);
    }

    /// Equivalent to `write(int)`, `write((char) b)`.
    pub fn write_int(&mut self, b: i32) {
        self.write(b as u16);
    }

    /// Equivalent to the final `write(char[])`, `write(b, 0, b.length)`.
    pub fn write_array(&mut self, b: &[u16]) {
        self.write_slice(b, 0, b.len());
    }

    /// Equivalent to `write(char[], int, int)`.
    pub fn write_slice(&mut self, b: &[u16], off: usize, len: usize) {
        self.reserve(len);
        self.unsafe_write_slice(b, off, len);
    }

    /// Equivalent to the final `write(OpenStringBuilder)`, which writes
    /// `write(arr.buf, 0, len)`: as many units of `arr` as **this**
    /// builder's length, exactly as Java's reads its own `len` field.
    pub fn write_builder(&mut self, arr: &OpenStringBuilder) {
        let len = self.len;
        self.write_slice(&arr.buf, 0, len);
    }

    /// Equivalent to `write(String)`: the UTF-16 units of `s`.
    pub fn write_str(&mut self, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.reserve(units.len());
        self.buf[self.len..self.len + units.len()].copy_from_slice(&units);
        self.len += units.len();
    }

    /// Equivalent to `flush()`, which does nothing.
    pub fn flush(&mut self) {}

    /// Equivalent to the final `reset()`: the length becomes 0.
    pub fn reset(&mut self) {
        self.len = 0;
    }

    /// Equivalent to `toCharArray()`: a copy of the content.
    pub fn to_char_array(&self) -> Vec<u16> {
        self.buf[..self.size()].to_vec()
    }
}

/// Equivalent to `toString()`, `new String(buf, 0, size())`.
///
/// **Divergence.** A Rust `String` cannot hold a lone surrogate, which
/// becomes U+FFFD; every well-formed content renders as Java's.
impl std::fmt::Display for OpenStringBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from_utf16_lossy(&self.buf[..self.size()]))
    }
}
