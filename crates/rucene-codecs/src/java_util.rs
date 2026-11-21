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

//! The JDK behaviours several codec packages reproduce: the exceptions and
//! messages of `new byte[n]`, array accesses, `System.arraycopy`,
//! `Arrays.compareUnsigned` and `ArrayUtil.grow`'s assertion. Each is ported
//! from the JDK 25 / Lucene 10.5.0 specification and checked against a JDK 25
//! probe; none is copied from JDK source.

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::{ByteArrayDataInput, DataInput, IndexInput};
use rucene::util::{ArrayUtil, BytesRef};

/// A `java.lang.NullPointerException` with the given helpful message.
pub(crate) fn null_pointer(message: impl Into<String>) -> LuceneError {
    LuceneError::unchecked("java.lang.NullPointerException", message.into())
}

/// `java.lang.NegativeArraySizeException` for `new T[size]`, whose message is
/// the size.
pub(crate) fn negative_array_size(size: impl std::fmt::Display) -> LuceneError {
    LuceneError::unchecked("java.lang.NegativeArraySizeException", size.to_string())
}

/// `java.lang.ArrayIndexOutOfBoundsException` for an array access.
pub(crate) fn array_index_out_of_bounds(index: i64, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// The bounds checks of `System.arraycopy` on two `byte[]`, in HotSpot's
/// order and with its messages (observed on the JDK 25 oracle): source
/// index, destination index, negative length, last source index, last
/// destination index.
///
/// # Errors
///
/// `ArrayIndexOutOfBoundsException` where `System.arraycopy` throws one.
fn arraycopy_check(
    src_len: usize,
    src_pos: i64,
    dst_len: usize,
    dst_pos: i64,
    length: i64,
) -> Result<()> {
    let aioobe = |message: String| {
        LuceneError::unchecked("java.lang.ArrayIndexOutOfBoundsException", message)
    };
    if src_pos < 0 {
        return Err(aioobe(format!(
            "arraycopy: source index {src_pos} out of bounds for byte[{src_len}]"
        )));
    }
    if dst_pos < 0 {
        return Err(aioobe(format!(
            "arraycopy: destination index {dst_pos} out of bounds for byte[{dst_len}]"
        )));
    }
    if length < 0 {
        return Err(aioobe(format!("arraycopy: length {length} is negative")));
    }
    if src_pos + length > src_len as i64 {
        return Err(aioobe(format!(
            "arraycopy: last source index {} out of bounds for byte[{src_len}]",
            src_pos + length
        )));
    }
    if dst_pos + length > dst_len as i64 {
        return Err(aioobe(format!(
            "arraycopy: last destination index {} out of bounds for byte[{dst_len}]",
            dst_pos + length
        )));
    }
    Ok(())
}

/// `System.arraycopy(src, srcPos, dst, dstPos, length)` on `byte[]`, with
/// [`arraycopy_check`]'s exceptions.
pub(crate) fn arraycopy(
    src: &[u8],
    src_pos: i64,
    dst: &mut [u8],
    dst_pos: i64,
    length: i64,
) -> Result<()> {
    arraycopy_check(src.len(), src_pos, dst.len(), dst_pos, length)?;
    let (s, d, n) = (src_pos as usize, dst_pos as usize, length as usize);
    dst[d..d + n].copy_from_slice(&src[s..s + n]);
    Ok(())
}

/// The message of `ArrayUtil.grow`/`growNoCopy`'s assertion on a negative
/// size (`ArrayUtil.java:458`).
pub(crate) fn negative_grow_message(size: i64) -> String {
    format!("size must be positive (got {size}): likely integer overflow?")
}

/// The helpful `NullPointerException` of invoking `method` (the receiver's
/// static type and the method's signature, as javac names it) on the null
/// `receiver` expression: `Cannot invoke "method" because "receiver" is
/// null` (forms observed on the JDK 25 oracle: `this.out`, `this.this$0.out`
/// in an inner class, `this$0.out` in an inner-class constructor).
pub(crate) fn null_receiver(method: &str, receiver: &str) -> LuceneError {
    null_pointer(format!(
        "Cannot invoke \"{method}\" because \"{receiver}\" is null"
    ))
}

/// `Arrays.compareUnsigned(a, aFromIndex, aToIndex, b, bFromIndex,
/// bToIndex)` (JDK 25 `java.util.Arrays`): `Arrays.rangeCheck` of each range
/// first, `a`'s then `b`'s, then the unsigned comparison of the common
/// length, else the difference of the lengths.
///
/// # Errors
///
/// `rangeCheck`'s exceptions: `IllegalArgumentException("fromIndex(f) >
/// toIndex(t)")`, then `ArrayIndexOutOfBoundsException("Array index out of
/// range: i")` for a negative start or an end past the array.
pub(crate) fn compare_unsigned(
    a: &[u8],
    a_from: i32,
    a_to: i32,
    b: &[u8],
    b_from: i32,
    b_to: i32,
) -> Result<i32> {
    range_check(a.len(), a_from, a_to)?;
    range_check(b.len(), b_from, b_to)?;
    let a = &a[a_from as usize..a_to as usize];
    let b = &b[b_from as usize..b_to as usize];
    match a.iter().zip(b).find(|(x, y)| x != y) {
        Some((x, y)) => Ok(i32::from(*x) - i32::from(*y)),
        None => Ok(a.len() as i32 - b.len() as i32),
    }
}

/// `Arrays.rangeCheck(arrayLength, fromIndex, toIndex)` (JDK 25).
fn range_check(array_length: usize, from: i32, to: i32) -> Result<()> {
    if from > to {
        return Err(LuceneError::illegal_argument(format!(
            "fromIndex({from}) > toIndex({to})"
        )));
    }
    if from < 0 {
        return Err(array_index_out_of_range(from));
    }
    if i64::from(to) > array_length as i64 {
        return Err(array_index_out_of_range(to));
    }
    Ok(())
}

/// `new ArrayIndexOutOfBoundsException(int index)`: `Array index out of
/// range: index`.
pub(crate) fn array_index_out_of_range(index: i32) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Array index out of range: {index}"),
    )
}

/// A `ByteArrayDataInput` over a `byte[]` its frame owns (Java's
/// `suffixBytes` + `suffixesReader` pairs): the buffer, the window's limit
/// and the read position.
///
/// The buffer has the length of Java's array: the frame's initial `new
/// byte[n]` ([`OwnedArrayInput::with_len`]), replaced by a zeroed `new
/// byte[ArrayUtil.oversize(len, 1)]` when a block needs more. Java's
/// `ByteArrayDataInput` checks reads against the array, not the window's
/// limit, so a read past the limit sees the array's other bytes, as here.
#[derive(Default)]
pub(crate) struct OwnedArrayInput {
    pub(crate) bytes: Vec<u8>,
    limit: usize,
    pos: usize,
}

impl OwnedArrayInput {
    /// The frame field initializer `new byte[len]`.
    pub(crate) fn with_len(len: usize) -> Self {
        Self {
            bytes: vec![0; len],
            limit: 0,
            pos: 0,
        }
    }

    /// `if (buf.length < len) buf = new byte[ArrayUtil.oversize(len, 1)];
    /// in.readBytes(buf, 0, len); reader.reset(buf, 0, len)`.
    ///
    /// A negative `len` (a corrupt block) keeps the array and is handed to
    /// the input's own `readBytes(b, 0, len)`, which rejects it or not as
    /// that input does (`MemorySegmentIndexInput` throws, the
    /// `ByteBuffersDataInput` loop `while (len > 0)` reads nothing); when it
    /// returns, `reader.reset(buf, 0, len)` puts the position at `0`.
    /// **Divergence (internal only):** Java's limit becomes `len` (negative);
    /// the window keeps a `usize` limit of `0`. The frames never read the
    /// limit (no `eof()`/`length()` on these readers in
    /// `OrdsSegmentTermsEnumFrame`/`OrdsIntersectTermsEnumFrame`), and reads
    /// are checked against the array, so no outcome differs.
    pub(crate) fn fill(&mut self, input: &mut dyn IndexInput, len: i32) -> Result<()> {
        if len < 0 {
            input.read_bytes_out_of_range(&mut self.bytes, 0, len)?;
            self.limit = 0;
            self.pos = 0;
            return Ok(());
        }
        let len = len as usize;
        if self.bytes.len() < len {
            self.bytes = vec![0; ArrayUtil::oversize(len, 1)];
        }
        input.read_bytes(Some(&mut self.bytes), 0, len)?;
        self.limit = len;
        self.pos = 0;
        Ok(())
    }

    /// `if (numBytes > buf.length) buf = new byte[ArrayUtil.oversize(numBytes,
    /// 1)]; System.arraycopy(src, ...); reader.reset(buf, 0, src.length)`.
    pub(crate) fn fill_from(&mut self, src: &[u8]) {
        if self.bytes.len() < src.len() {
            self.bytes = vec![0; ArrayUtil::oversize(src.len(), 1)];
        }
        self.bytes[..src.len()].copy_from_slice(src);
        self.limit = src.len();
        self.pos = 0;
    }

    /// `buf[index]`.
    ///
    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` outside the array.
    pub(crate) fn byte_at(&self, index: i32) -> Result<u8> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.bytes.get(i).copied())
            .ok_or_else(|| array_index_out_of_bounds(i64::from(index), self.bytes.len()))
    }

    /// `System.arraycopy(buf, src_pos, dest, dest_pos, length)`.
    ///
    /// # Errors
    ///
    /// `System.arraycopy`'s `ArrayIndexOutOfBoundsException`s.
    pub(crate) fn copy_to(
        &self,
        src_pos: i32,
        dest: &mut [u8],
        dest_pos: i32,
        length: i32,
    ) -> Result<()> {
        arraycopy(
            &self.bytes,
            i64::from(src_pos),
            dest,
            i64::from(dest_pos),
            i64::from(length),
        )
    }

    /// `Arrays.compareUnsigned(buf, from, to, b, b_from, b_to)`.
    ///
    /// # Errors
    ///
    /// `Arrays.rangeCheck`'s exceptions.
    pub(crate) fn compare_to(
        &self,
        from: i32,
        to: i32,
        b: &[u8],
        b_from: i32,
        b_to: i32,
    ) -> Result<i32> {
        compare_unsigned(&self.bytes, from, to, b, b_from, b_to)
    }

    /// Runs `f` on a `ByteArrayDataInput` over the window, keeping the
    /// position it leaves.
    pub(crate) fn with<R>(
        &mut self,
        f: impl FnOnce(&mut ByteArrayDataInput<'_>) -> Result<R>,
    ) -> Result<R> {
        let mut input = ByteArrayDataInput::with_range(&self.bytes, 0, self.limit);
        input.set_position(self.pos);
        let result = f(&mut input);
        self.pos = input.position();
        result
    }

    pub(crate) fn read_v_int(&mut self) -> Result<i32> {
        self.with(|i| i.read_v_int())
    }

    pub(crate) fn read_v_long(&mut self) -> Result<i64> {
        self.with(|i| i.read_v_long())
    }

    pub(crate) fn read_byte(&mut self) -> Result<u8> {
        self.with(|i| i.read_byte())
    }

    pub(crate) fn skip_bytes(&mut self, n: i32) -> Result<()> {
        self.with(|i| i.skip_bytes(i64::from(n)))
    }

    /// `readBytes(dest, offset, len)`: `System.arraycopy(buf, pos, dest,
    /// offset, len); pos += len`.
    ///
    /// # Errors
    ///
    /// `System.arraycopy`'s `ArrayIndexOutOfBoundsException`s.
    pub(crate) fn read_into(&mut self, dest: &mut [u8], offset: i32, len: i32) -> Result<()> {
        let pos = self.java_position();
        self.copy_to(pos, dest, offset, len)?;
        self.pos = pos.wrapping_add(len) as isize as usize;
        Ok(())
    }

    /// The read position as Java's `int` (a backward `skipBytes` past the
    /// start leaves it negative).
    pub(crate) fn java_position(&self) -> i32 {
        self.pos as i32
    }

    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    pub(crate) fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub(crate) fn rewind(&mut self) {
        self.pos = 0;
    }
}

/// `new byte[len]` for a length read from a file: a negative one raises
/// `NegativeArraySizeException`.
pub(crate) fn array_len(len: i32) -> Result<usize> {
    usize::try_from(len).map_err(|_| {
        LuceneError::unchecked("java.lang.NegativeArraySizeException", len.to_string())
    })
}

/// Java's `ArrayIndexOutOfBoundsException` for `index` into an array of
/// `length`.
pub(crate) fn out_of_bounds(index: usize, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// [`out_of_bounds`] for a Java `int` index, which may be negative.
pub(crate) fn out_of_bounds_i32(index: i32, length: usize) -> LuceneError {
    array_index_out_of_bounds(i64::from(index), length)
}

/// Java's `BytesRefBuilder term`: the backing array plus the length;
/// `setLength` keeps the bytes of the array, and `grow` zero-fills the new
/// ones.
///
/// It holds them as the `BytesRef` Java's `get()` returns (`bytes`, `length`,
/// `offset` 0), reached through [`Deref`](std::ops::Deref), so that a terms
/// enum lends its term with [`get_ref`](Self::get_ref) as Java returns
/// `term.get()`.
#[derive(Default)]
pub(crate) struct TermBuilder {
    term: BytesRef,
}

impl std::ops::Deref for TermBuilder {
    type Target = BytesRef;

    fn deref(&self) -> &BytesRef {
        &self.term
    }
}

impl std::ops::DerefMut for TermBuilder {
    fn deref_mut(&mut self) -> &mut BytesRef {
        &mut self.term
    }
}

impl TermBuilder {
    /// `get()`, the builder's own `BytesRef`, lent without copying.
    pub(crate) fn get_ref(&self) -> &BytesRef {
        &self.term
    }

    /// `grow(capacity)`: `ArrayUtil.grow` copies the array into one of
    /// `ArrayUtil.oversize(capacity, 1)` bytes when it is too short.
    pub(crate) fn grow(&mut self, capacity: usize) {
        if self.bytes.len() < capacity {
            self.bytes.resize(ArrayUtil::oversize(capacity, 1), 0);
        }
    }

    /// `setLength(length); grow(length)`, the frames' sequence before they
    /// copy a suffix in. A negative length (a corrupt block) fails
    /// `ArrayUtil.grow`'s assertion under `-ea`; without it Java goes on to
    /// the copy, whose negative length fails, so the port keeps the old
    /// length for that copy to fail.
    ///
    /// # Errors
    ///
    /// `AssertionError` (with `debug_assertions`) for a negative length.
    pub(crate) fn set_length_and_grow(&mut self, length: i32) -> Result<()> {
        if cfg!(debug_assertions) && length < 0 {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(
                negative_grow_message(i64::from(length)),
            )));
        }
        if length >= 0 {
            self.length = length as usize;
            self.grow(self.length);
        }
        Ok(())
    }

    /// `setLength(length)`; the port's array grows with it, where Java's
    /// callers `grow` right after.
    pub(crate) fn set_length(&mut self, length: usize) {
        self.grow(length);
        self.length = length;
    }

    /// `copyBytes(src)`.
    pub(crate) fn copy_bytes(&mut self, src: &[u8]) {
        self.set_length(src.len());
        self.bytes[..src.len()].copy_from_slice(src);
    }

    /// `get()`, the term's bytes.
    pub(crate) fn get(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    /// `byteAt(i)`, which reads the backing array.
    pub(crate) fn byte_at(&self, i: usize) -> Result<u8> {
        self.bytes
            .get(i)
            .copied()
            .ok_or_else(|| out_of_bounds(i, self.bytes.len()))
    }

    /// `byteAt(i)` for a Java `int` index, which may be negative.
    pub(crate) fn byte_at_java(&self, i: i32) -> Result<u8> {
        usize::try_from(i)
            .ok()
            .and_then(|u| self.bytes.get(u).copied())
            .ok_or_else(|| out_of_bounds_i32(i, self.bytes.len()))
    }

    /// `setByteAt(i, b)`.
    pub(crate) fn set_byte_at(&mut self, i: usize, b: u8) {
        self.grow(i + 1);
        self.bytes[i] = b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (rmp #527): `Arrays.compareUnsigned` checks both ranges
    /// with `rangeCheck` before comparing, with the JDK 25 messages.
    #[test]
    fn compare_unsigned_follows_the_jdk() {
        let a = [1u8, 2, 0xff];
        let b = [1u8, 2, 3, 4];
        assert!(compare_unsigned(&a, 0, 3, &b, 0, 3).unwrap() > 0);
        assert_eq!(compare_unsigned(&a, 0, 2, &b, 0, 4).unwrap(), -2);
        assert_eq!(compare_unsigned(&a, 1, 1, &b, 4, 4).unwrap(), 0);
        let msg = |r: Result<i32>| r.unwrap_err().to_string();
        assert!(msg(compare_unsigned(&a, 2, 1, &b, 0, 0)).contains("fromIndex(2) > toIndex(1)"));
        assert!(msg(compare_unsigned(&a, 0, 4, &b, 0, 0)).contains("Array index out of range: 4"));
        assert!(msg(compare_unsigned(&a, 0, 0, &b, -1, 0)).contains("Array index out of range: -1"));
    }
}

#[cfg(test)]
mod frame_buffer_tests {
    use super::*;
    use rucene::store::{ByteBuffersDataOutput, ByteBuffersIndexInput, DataOutput};

    fn input_of(bytes: &[u8]) -> ByteBuffersIndexInput {
        let mut out = ByteBuffersDataOutput::new();
        out.write_bytes(bytes, 0, bytes.len()).unwrap();
        ByteBuffersIndexInput::new(out.to_data_input().unwrap(), "frame")
    }

    /// Regression (rmp #527): a frame's buffers have the length of Java's
    /// arrays (`new byte[128]`, then `new byte[ArrayUtil.oversize(n, 1)]`,
    /// zeroed), and their reader checks reads against the array, not the
    /// block: a read past the block sees the array's zeros; the first index
    /// past the array is Java's `ArrayIndexOutOfBoundsException`.
    #[test]
    fn frame_buffers_have_javas_array_lengths() {
        let mut suffixes = OwnedArrayInput::with_len(128);
        let mut input = input_of(&[7u8; 300]);
        suffixes.fill(&mut input, 3).unwrap();
        assert_eq!(suffixes.bytes.len(), 128);
        suffixes.skip_bytes(3).unwrap();
        // Past the block's 3 bytes: the array's fourth byte.
        assert_eq!(suffixes.read_byte().unwrap(), 0);

        suffixes.fill(&mut input, 200).unwrap();
        let capacity = ArrayUtil::oversize(200, 1);
        assert_eq!(suffixes.bytes.len(), capacity);
        assert!(suffixes.bytes[200..].iter().all(|&b| b == 0));
        let err = suffixes.byte_at(capacity as i32).unwrap_err().to_string();
        assert!(
            err.contains(&format!(
                "Index {capacity} out of bounds for length {capacity}"
            )),
            "{err}"
        );
    }

    /// Regression (rmp #527): a negative suffix length is copied by
    /// `ByteArrayDataInput.readBytes`'s `System.arraycopy`, which rejects
    /// it, not refused as `new byte[n]`.
    #[test]
    fn negative_suffix_copy_is_an_arraycopy_error() {
        let mut suffixes = OwnedArrayInput::with_len(128);
        let mut input = input_of(&[1, 2, 3]);
        suffixes.fill(&mut input, 3).unwrap();
        let mut dest = vec![0u8; 8];
        let err = suffixes
            .read_into(&mut dest, 0, -1)
            .unwrap_err()
            .to_string();
        assert!(err.contains("ArrayIndexOutOfBoundsException"), "{err}");
        assert!(err.contains("arraycopy: length -1 is negative"), "{err}");
    }

    /// Regression (rmp #527): `setLength(n); grow(n)` with a negative `n`
    /// fails `ArrayUtil.grow`'s assertion under `-ea` only.
    #[test]
    fn negative_term_length_fails_the_grow_assertion() {
        let mut term = TermBuilder::default();
        let result = term.set_length_and_grow(-2);
        if cfg!(debug_assertions) {
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("size must be positive (got -2): likely integer overflow?"),
                "{err}"
            );
        } else {
            result.unwrap();
        }
        term.set_length_and_grow(9).unwrap();
        assert_eq!(term.length, 9);
        assert_eq!(term.bytes.len(), ArrayUtil::oversize(9, 1));
    }

    /// Regression (rmp #527): a negative block length read from a file is
    /// passed to the input's `readBytes(buf, 0, len)`, not cast to a huge
    /// `usize` (an allocation abort) nor refused as `new byte[n]`. A
    /// `ByteBuffersIndexInput` accepts it (`ByteBuffersDataInput.readBytes`
    /// loops `while (len > 0)`, `ByteBuffersDataInput.java`), so the array is
    /// kept and `reader.reset(buf, 0, len)` rewinds the position to `0`.
    #[test]
    fn negative_fill_length_is_handed_to_the_inputs_read_bytes() {
        let mut stats = OwnedArrayInput::with_len(64);
        let mut input = input_of(&[1, 2, 3]);
        stats.fill(&mut input, 3).unwrap();
        assert_eq!(stats.read_byte().unwrap(), 1);
        stats.fill(&mut input, -1).unwrap();
        assert_eq!(stats.bytes.len(), 64);
        assert_eq!(stats.pos, 0);
        assert_eq!(input.file_pointer().unwrap(), 3);
    }
}
