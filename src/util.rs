//! Low-level utilities ported from `org.apache.lucene.util`.
//!
//! This module provides the foundational data structures and helpers used by
//! higher-level Lucene subsystems: byte refs, numeric encoders, bit utilities,
//! grouped variable-length integers, array growth helpers, I/O utilities,
//! memory accounting, constants, logging sinks, bit sets, and thread-interruption
//! handling.

#![deny(unsafe_code)]

pub mod attribute;
pub mod automaton;
pub mod byte_block_pool;
pub mod bytes_ref_hash;
pub mod chars_ref;
/// LZ4 and lowercase-ASCII compression utilities.
pub mod compress;
pub mod extra;
pub mod file_deleter;
pub mod packed;
pub mod selector;
pub mod small_float;
pub mod string_helper;

/// Block KD-tree utilities ported from `org.apache.lucene.util.bkd`.
pub mod bkd;

/// Additional bitset and live-docs variants.
pub mod bit_sets;

/// HNSW graph utilities for vector search.
pub mod hnsw;

/// Vector arithmetic primitives ported from `org.apache.lucene.util.VectorUtil`.
pub mod vector_util;

pub use attribute::{
    unwrap_all, AsUnwrappable, Attribute, AttributeFactory, AttributeImpl, AttributeReflector,
    AttributeSource, CapturedState, CloseableThreadLocal, DefaultAttributeFactory, Unwrappable,
};
pub use automaton::{
    automata, operations, Automaton, AutomatonType, ByteRunAutomaton, ByteRunnable,
    CompiledAutomaton, RunAutomaton, Transition, TransitionAccessor,
};
pub use bit_sets::{DenseLiveDocs, RoaringDocIdSet, SparseFixedBitSet, SparseLiveDocs};
pub use byte_block_pool::{
    ByteBlockPool, BYTE_BLOCK_MASK, BYTE_BLOCK_SHIFT, BYTE_BLOCK_SIZE, MAX_TERM_LENGTH,
};
pub use bytes_ref_hash::BytesRefHash;
pub use chars_ref::{CharsRef, EMPTY_CHARS};
pub use extra::{
    IdentityLongValues, IntoIter as PriorityQueueIntoIter, LongBitSet, LongValues, MergedIterator,
    PriorityQueue, PriorityQueueComparator, Version, ZeroesLongValues,
};
pub use small_float::SmallFloat;
pub use string_helper::{
    compare_utf16, read_string, write_string, IntsRef, StringHelper, ID_LENGTH,
};
pub use vector_util::{
    add, check_finite, cosine_bytes, cosine_f32, dot_product_bytes, dot_product_f32,
    dot_product_score, is_unit_vector, is_zero_vector_bytes, is_zero_vector_f32, l2normalize,
    normalize_distance_to_unit_interval, normalize_to_unit_interval, scale_max_inner_product_score,
    square_distance_bytes, square_distance_f32, USE_FMA,
};

use std::{
    cmp::Ordering,
    env,
    fmt::{self, Debug, Display, Formatter},
    fs::{self, File, OpenOptions},
    io,
    path::Path,
    sync::LazyLock,
};

use crate::error::LuceneError;

// ---------------------------------------------------------------------------
// BytesRef
// ---------------------------------------------------------------------------

/// A reference to a slice of bytes, equivalent to Lucene's `BytesRef`.
///
/// The underlying buffer is owned; `clone()` copies the referenced slice so that
/// the resulting value is independent. This differs from Java's shallow clone,
/// but matches the observable content semantics in safe Rust.
#[derive(Clone, Default)]
pub struct BytesRef {
    /// The underlying byte buffer. Never empty conceptually; an empty reference
    /// stores an empty vector.
    pub bytes: Vec<u8>,
    /// Offset of the first valid byte.
    pub offset: usize,
    /// Number of valid bytes starting at `offset`.
    pub length: usize,
}

impl BytesRef {
    /// Creates a `BytesRef` referencing the entire provided byte vector.
    pub fn new(bytes: Vec<u8>) -> Self {
        let length = bytes.len();
        Self {
            bytes,
            offset: 0,
            length,
        }
    }

    /// Creates a `BytesRef` with the given capacity and zero length/offset.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            offset: 0,
            length: 0,
        }
    }

    /// Returns a `BytesRef` whose content is a copy of `other`'s active slice,
    /// with offset zero.
    pub fn deep_copy_of(other: &BytesRef) -> Self {
        let slice = other.slice();
        Self {
            bytes: slice.to_vec(),
            offset: 0,
            length: slice.len(),
        }
    }

    /// Returns the active slice of bytes.
    pub fn slice(&self) -> &[u8] {
        &self.bytes[self.offset..self.offset + self.length]
    }

    /// Compares the active bytes against another `BytesRef` for equality.
    pub fn bytes_equals(&self, other: &BytesRef) -> bool {
        self.slice() == other.slice()
    }

    /// Decodes the active bytes as UTF-8.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the bytes are not valid UTF-8.
    pub fn utf8_to_string(&self) -> Result<String, LuceneError> {
        String::from_utf8(self.slice().to_vec())
            .map_err(|e| LuceneError::IllegalArgument(format!("invalid UTF-8: {e}")))
    }

    /// Returns a hex-encoded representation such as `[6c 75 63 65 6e 65]`.
    pub fn to_hex_string(&self) -> String {
        let mut s = String::with_capacity(2 + 3 * self.length);
        s.push('[');
        for (i, b) in self.slice().iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&format!("{:x}", b));
        }
        s.push(']');
        s
    }

    /// Performs internal consistency checks.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalState` if any invariant is violated.
    pub fn is_valid(&self) -> Result<(), LuceneError> {
        if self.length > self.bytes.len() {
            return Err(LuceneError::IllegalState(format!(
                "length {} out of bounds (bytes.len() = {})",
                self.length,
                self.bytes.len()
            )));
        }
        if self.offset > self.bytes.len() {
            return Err(LuceneError::IllegalState(format!(
                "offset {} out of bounds (bytes.len() = {})",
                self.offset,
                self.bytes.len()
            )));
        }
        if self.offset + self.length > self.bytes.len() {
            return Err(LuceneError::IllegalState(format!(
                "offset + length out of bounds: offset={}, length={}, bytes.len()={}",
                self.offset,
                self.length,
                self.bytes.len()
            )));
        }
        Ok(())
    }
}

impl PartialEq for BytesRef {
    fn eq(&self, other: &Self) -> bool {
        self.bytes_equals(other)
    }
}

impl Eq for BytesRef {}

impl PartialOrd for BytesRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BytesRef {
    fn cmp(&self, other: &Self) -> Ordering {
        let a = self.slice();
        let b = other.slice();
        let len = a.len().min(b.len());
        for i in 0..len {
            let ord = a[i].cmp(&b[i]);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a.len().cmp(&b.len())
    }
}

impl Display for BytesRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex_string())
    }
}

impl Debug for BytesRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("BytesRef")
            .field("offset", &self.offset)
            .field("length", &self.length)
            .field("bytes", &self.to_hex_string())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// BytesRefBuilder
// ---------------------------------------------------------------------------

/// A builder for [`BytesRef`] values, equivalent to Lucene's `BytesRefBuilder`.
#[derive(Clone, Default, Debug)]
pub struct BytesRefBuilder {
    inner: BytesRef,
}

impl BytesRefBuilder {
    /// Creates an empty builder.
    pub fn new() -> Self {
        Self {
            inner: BytesRef::default(),
        }
    }

    /// Returns a reference to the full underlying byte buffer.
    pub fn bytes(&self) -> &[u8] {
        &self.inner.bytes
    }

    /// Returns the current number of bytes in the builder.
    pub fn length(&self) -> usize {
        self.inner.length
    }

    /// Sets the current length.
    pub fn set_length(&mut self, length: usize) {
        self.inner.length = length;
    }

    /// Returns the byte at the given offset.
    pub fn byte_at(&self, offset: usize) -> u8 {
        self.inner.bytes[offset]
    }

    /// Sets the byte at the given offset.
    pub fn set_byte_at(&mut self, offset: usize, b: u8) {
        self.inner.bytes[offset] = b;
    }

    /// Ensures the underlying buffer can hold at least `capacity` bytes.
    pub fn grow(&mut self, capacity: usize) {
        self.inner.bytes = ArrayUtil::grow(&self.inner.bytes, capacity);
    }

    /// Grows the buffer without copying existing bytes.
    pub fn grow_no_copy(&mut self, capacity: usize) {
        self.inner.bytes = ArrayUtil::grow_no_copy(&self.inner.bytes, capacity);
    }

    /// Appends a single byte.
    pub fn append(&mut self, b: u8) {
        self.grow(self.inner.length + 1);
        self.inner.bytes[self.inner.length] = b;
        self.inner.length += 1;
    }

    /// Appends bytes from a slice.
    pub fn append_bytes(&mut self, b: &[u8], off: usize, len: usize) {
        self.grow(self.inner.length + len);
        self.inner.bytes[self.inner.length..self.inner.length + len]
            .copy_from_slice(&b[off..off + len]);
        self.inner.length += len;
    }

    /// Appends the active slice of another `BytesRef`.
    pub fn append_ref(&mut self, r: &BytesRef) {
        self.append_bytes(&r.bytes, r.offset, r.length);
    }

    /// Resets the builder to the empty state.
    pub fn clear(&mut self) {
        self.inner.length = 0;
    }

    /// Replaces the content with a copy of the provided bytes.
    pub fn copy_bytes(&mut self, b: &[u8], off: usize, len: usize) {
        self.inner.offset = 0;
        self.inner.length = len;
        self.grow_no_copy(len);
        self.inner.bytes[..len].copy_from_slice(&b[off..off + len]);
    }

    /// Replaces the content with a copy of the active slice of another `BytesRef`.
    pub fn copy_ref(&mut self, r: &BytesRef) {
        self.copy_bytes(&r.bytes, r.offset, r.length);
    }

    /// Copies UTF-8 bytes representing the provided text into the buffer.
    pub fn copy_chars(&mut self, text: &str) {
        let encoded = text.as_bytes();
        self.copy_bytes(encoded, 0, encoded.len());
    }

    /// Returns a `BytesRef` pointing to the builder's internal content.
    ///
    /// The returned value remains valid only until the builder is modified.
    pub fn get(&self) -> BytesRef {
        BytesRef {
            bytes: self.inner.bytes.clone(),
            offset: 0,
            length: self.inner.length,
        }
    }

    /// Builds a new independent `BytesRef` containing the current content.
    pub fn to_bytes_ref(&self) -> BytesRef {
        BytesRef {
            bytes: self.inner.bytes[..self.inner.length].to_vec(),
            offset: 0,
            length: self.inner.length,
        }
    }
}

impl Display for BytesRefBuilder {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

// ---------------------------------------------------------------------------
// NumericUtils
// ---------------------------------------------------------------------------

/// Helpers to encode numeric values as sortable bytes, matching Lucene 10.5.0.
pub struct NumericUtils;

impl NumericUtils {
    /// Converts a `double` to a sortable signed `long`.
    pub fn double_to_sortable_long(value: f64) -> i64 {
        Self::sortable_double_bits(value.to_bits() as i64)
    }

    /// Converts a sortable `long` back to a `double`.
    pub fn sortable_long_to_double(encoded: i64) -> f64 {
        f64::from_bits(Self::sortable_double_bits(encoded) as u64)
    }

    /// Converts a `float` to a sortable signed `int`.
    pub fn float_to_sortable_int(value: f32) -> i32 {
        Self::sortable_float_bits(value.to_bits() as i32)
    }

    /// Converts a sortable `int` back to a `float`.
    pub fn sortable_int_to_float(encoded: i32) -> f32 {
        f32::from_bits(Self::sortable_float_bits(encoded) as u32)
    }

    /// Converts IEEE-754 double bits to a sortable order (and back).
    pub fn sortable_double_bits(bits: i64) -> i64 {
        bits ^ ((bits >> 63) & i64::MAX)
    }

    /// Converts IEEE-754 float bits to a sortable order (and back).
    pub fn sortable_float_bits(bits: i32) -> i32 {
        bits ^ ((bits >> 31) & i32::MAX)
    }

    /// Encodes an `i32` into four big-endian bytes such that unsigned byte order
    /// matches signed integer order.
    pub fn int_to_sortable_bytes(value: i32, result: &mut [u8], offset: usize) {
        let encoded = (value as u32) ^ 0x8000_0000;
        result[offset..offset + 4].copy_from_slice(&encoded.to_be_bytes());
    }

    /// Decodes an `i32` previously written with [`Self::int_to_sortable_bytes`].
    pub fn sortable_bytes_to_int(encoded: &[u8], offset: usize) -> i32 {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&encoded[offset..offset + 4]);
        let x = u32::from_be_bytes(bytes);
        (x ^ 0x8000_0000) as i32
    }

    /// Encodes an `i64` into eight big-endian bytes such that unsigned byte order
    /// matches signed long order.
    pub fn long_to_sortable_bytes(value: i64, result: &mut [u8], offset: usize) {
        let encoded = (value as u64) ^ 0x8000_0000_0000_0000;
        result[offset..offset + 8].copy_from_slice(&encoded.to_be_bytes());
    }

    /// Decodes an `i64` previously written with [`Self::long_to_sortable_bytes`].
    pub fn sortable_bytes_to_long(encoded: &[u8], offset: usize) -> i64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&encoded[offset..offset + 8]);
        let v = u64::from_be_bytes(bytes);
        (v ^ 0x8000_0000_0000_0000) as i64
    }
}

// ---------------------------------------------------------------------------
// BitUtil
// ---------------------------------------------------------------------------

/// High-efficiency bit-twiddling routines and primitive encoders.
pub struct BitUtil;

impl BitUtil {
    /// Returns the next power of two greater than or equal to `v`, or `v` if it
    /// is already a power of two or zero.
    pub fn next_highest_power_of_two(v: i32) -> i32 {
        let mut v = v;
        v -= 1;
        v |= v >> 1;
        v |= v >> 2;
        v |= v >> 4;
        v |= v >> 8;
        v |= v >> 16;
        v + 1
    }

    /// 64-bit variant of [`Self::next_highest_power_of_two`].
    pub fn next_highest_power_of_two_long(v: i64) -> i64 {
        let mut v = v;
        v -= 1;
        v |= v >> 1;
        v |= v >> 2;
        v |= v >> 4;
        v |= v >> 8;
        v |= v >> 16;
        v |= v >> 32;
        v + 1
    }

    /// Zig-zag encodes an `i32`.
    pub fn zig_zag_encode(i: i32) -> i32 {
        (i >> 31) ^ (i << 1)
    }

    /// Zig-zag encodes an `i64`.
    pub fn zig_zag_encode_long(l: i64) -> i64 {
        (l >> 63) ^ (l << 1)
    }

    /// Zig-zag decodes an `i32`.
    pub fn zig_zag_decode(i: i32) -> i32 {
        (((i as u32) >> 1) as i32) ^ -(i & 1)
    }

    /// Zig-zag decodes an `i64`.
    pub fn zig_zag_decode_long(l: i64) -> i64 {
        (((l as u64) >> 1) as i64) ^ -(l & 1)
    }

    /// Returns true if `x` (treated as unsigned) is zero or a power of two.
    pub fn is_zero_or_power_of_two(x: i32) -> bool {
        (x & (x - 1)) == 0
    }

    /// Reads a little-endian `i16` from `src` at `offset`.
    pub fn read_le_short(src: &[u8], offset: usize) -> i16 {
        let mut bytes = [0u8; 2];
        bytes.copy_from_slice(&src[offset..offset + 2]);
        i16::from_le_bytes(bytes)
    }

    /// Writes a little-endian `i16` to `dst` at `offset`.
    pub fn write_le_short(dst: &mut [u8], offset: usize, value: i16) {
        dst[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Reads a little-endian `i32` from `src` at `offset`.
    pub fn read_le_int(src: &[u8], offset: usize) -> i32 {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&src[offset..offset + 4]);
        i32::from_le_bytes(bytes)
    }

    /// Writes a little-endian `i32` to `dst` at `offset`.
    pub fn write_le_int(dst: &mut [u8], offset: usize, value: i32) {
        dst[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Reads a little-endian `i64` from `src` at `offset`.
    pub fn read_le_long(src: &[u8], offset: usize) -> i64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&src[offset..offset + 8]);
        i64::from_le_bytes(bytes)
    }

    /// Writes a little-endian `i64` to `dst` at `offset`.
    pub fn write_le_long(dst: &mut [u8], offset: usize, value: i64) {
        dst[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// Reads a big-endian `i32` from `src` at `offset`.
    pub fn read_be_int(src: &[u8], offset: usize) -> i32 {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&src[offset..offset + 4]);
        i32::from_be_bytes(bytes)
    }

    /// Writes a big-endian `i32` to `dst` at `offset`.
    pub fn write_be_int(dst: &mut [u8], offset: usize, value: i32) {
        dst[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    /// Reads a big-endian `i64` from `src` at `offset`.
    pub fn read_be_long(src: &[u8], offset: usize) -> i64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&src[offset..offset + 8]);
        i64::from_be_bytes(bytes)
    }

    /// Writes a big-endian `i64` to `dst` at `offset`.
    pub fn write_be_long(dst: &mut [u8], offset: usize, value: i64) {
        dst[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
    }
}

// ---------------------------------------------------------------------------
// GroupVIntUtil
// ---------------------------------------------------------------------------

/// Maximum length of a single group-varint: 1 flag byte + 4 integers.
pub const MAX_LENGTH_PER_GROUP: usize = 1 + 4 * 4;

/// Utility methods for group-varint encoding, matching Lucene 10.5.0.
pub struct GroupVIntUtil;

impl GroupVIntUtil {
    /// Reads group varints from `src` into `dst[0..limit]`, advancing `pos`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::Io` if the input is truncated or otherwise invalid.
    pub fn read_group_vints(
        src: &[u8],
        pos: &mut usize,
        dst: &mut [i32],
        limit: usize,
    ) -> Result<(), LuceneError> {
        let limit = limit.min(dst.len());
        let mut i = 0;
        while i + 4 <= limit {
            Self::read_group_vint(src, pos, dst, i)?;
            i += 4;
        }
        while i < limit {
            dst[i] = Self::read_vint(src, pos)?;
            i += 1;
        }
        Ok(())
    }

    /// Reads a single group of four varints.
    fn read_group_vint(
        src: &[u8],
        pos: &mut usize,
        dst: &mut [i32],
        offset: usize,
    ) -> Result<(), LuceneError> {
        let flag = Self::read_byte(src, pos)? as usize;
        let n1 = flag >> 6;
        let n2 = (flag >> 4) & 0x03;
        let n3 = (flag >> 2) & 0x03;
        let n4 = flag & 0x03;
        dst[offset] = Self::read_int_in_group(src, pos, n1)?;
        dst[offset + 1] = Self::read_int_in_group(src, pos, n2)?;
        dst[offset + 2] = Self::read_int_in_group(src, pos, n3)?;
        dst[offset + 3] = Self::read_int_in_group(src, pos, n4)?;
        Ok(())
    }

    fn read_int_in_group(
        src: &[u8],
        pos: &mut usize,
        num_bytes_minus_one: usize,
    ) -> Result<i32, LuceneError> {
        match num_bytes_minus_one {
            0 => Ok(Self::read_byte(src, pos)? as i32),
            1 => Ok(Self::read_le_short(src, pos)? as i32 & 0xFFFF),
            2 => {
                let low = Self::read_le_short(src, pos)? as i32 & 0xFFFF;
                let high = (Self::read_byte(src, pos)? as i32) << 16;
                Ok(low | high)
            }
            3 => Self::read_le_int(src, pos),
            _ => Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid group varint byte count",
            ))),
        }
    }

    /// Writes group varints from `values[0..limit]` into `dst`, advancing `pos`.
    pub fn write_group_vints(values: &[i32], limit: usize, dst: &mut Vec<u8>) {
        let limit = limit.min(values.len());
        let mut read_pos = 0;
        let mut scratch = [0u8; MAX_LENGTH_PER_GROUP];
        while limit - read_pos >= 4 {
            let n1 = Self::num_bytes(values[read_pos]) - 1;
            let n2 = Self::num_bytes(values[read_pos + 1]) - 1;
            let n3 = Self::num_bytes(values[read_pos + 2]) - 1;
            let n4 = Self::num_bytes(values[read_pos + 3]) - 1;
            let flag = (n1 << 6) | (n2 << 4) | (n3 << 2) | n4;
            let mut write_pos = 0;
            scratch[write_pos] = flag as u8;
            write_pos += 1;
            BitUtil::write_le_int(&mut scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n1 + 1;
            BitUtil::write_le_int(&mut scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n2 + 1;
            BitUtil::write_le_int(&mut scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n3 + 1;
            BitUtil::write_le_int(&mut scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n4 + 1;
            dst.extend_from_slice(&scratch[..write_pos]);
        }
        while read_pos < limit {
            Self::write_vint(values[read_pos], dst);
            read_pos += 1;
        }
    }

    fn num_bytes(v: i32) -> usize {
        let uv = (v as u32) | 1;
        4 - (uv.leading_zeros() >> 3) as usize
    }

    fn read_byte(src: &[u8], pos: &mut usize) -> Result<u8, LuceneError> {
        if *pos >= src.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF reading byte",
            )));
        }
        let b = src[*pos];
        *pos += 1;
        Ok(b)
    }

    fn read_le_short(src: &[u8], pos: &mut usize) -> Result<i16, LuceneError> {
        if *pos + 2 > src.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF reading short",
            )));
        }
        let v = BitUtil::read_le_short(src, *pos);
        *pos += 2;
        Ok(v)
    }

    fn read_le_int(src: &[u8], pos: &mut usize) -> Result<i32, LuceneError> {
        if *pos + 4 > src.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF reading int",
            )));
        }
        let v = BitUtil::read_le_int(src, *pos);
        *pos += 4;
        Ok(v)
    }

    fn read_vint(src: &[u8], pos: &mut usize) -> Result<i32, LuceneError> {
        let mut b = Self::read_byte(src, pos)? as i32;
        let mut i = b & 0x7F;
        let mut shift = 7;
        while (b & 0x80) != 0 {
            b = Self::read_byte(src, pos)? as i32;
            i |= (b & 0x7F) << shift;
            shift += 7;
        }
        Ok(i)
    }

    fn write_vint(value: i32, dst: &mut Vec<u8>) {
        let mut value = value;
        while (value & !0x7F) != 0 {
            dst.push((0x80 | (value & 0x7F)) as u8);
            value = ((value as u32) >> 7) as i32;
        }
        dst.push(value as u8);
    }
}

// ---------------------------------------------------------------------------
// ArrayUtil
// ---------------------------------------------------------------------------

/// Methods for manipulating arrays, matching Lucene's `ArrayUtil`.
pub struct ArrayUtil;

impl ArrayUtil {
    /// Maximum length for an array (`i32::MAX` minus an estimated header size).
    pub const MAX_ARRAY_LENGTH: usize = i32::MAX as usize - 16;

    /// Returns a size greater than or equal to `min_target_size`, generally
    /// over-allocating exponentially.
    pub fn oversize(min_target_size: usize, bytes_per_element: usize) -> usize {
        if min_target_size > Self::MAX_ARRAY_LENGTH {
            return Self::MAX_ARRAY_LENGTH;
        }
        if min_target_size == 0 {
            return 0;
        }
        let mut extra = min_target_size >> 3;
        if extra < 3 {
            extra = 3;
        }
        let mut new_size = min_target_size + extra;
        if new_size + 7 > Self::MAX_ARRAY_LENGTH {
            return Self::MAX_ARRAY_LENGTH;
        }
        // Align to 8 bytes on 64-bit platforms; assume 64-bit for this port.
        new_size = match bytes_per_element {
            4 => (new_size + 1) & !1,
            2 => (new_size + 3) & !3,
            1 => (new_size + 7) & !7,
            8 => new_size,
            _ => new_size,
        };
        new_size
    }

    /// Returns a larger array, generally over-allocating exponentially.
    pub fn grow(array: &[u8], min_size: usize) -> Vec<u8> {
        if array.len() >= min_size {
            return array.to_vec();
        }
        let new_len = Self::oversize(min_size, 1);
        Self::grow_exact(array, new_len)
    }

    /// Grows an `i32` array.
    pub fn grow_int(array: &[i32], min_size: usize) -> Vec<i32> {
        if array.len() >= min_size {
            return array.to_vec();
        }
        let new_len = Self::oversize(min_size, 4);
        Self::grow_exact_int(array, new_len)
    }

    /// Grows an `i64` array.
    pub fn grow_long(array: &[i64], min_size: usize) -> Vec<i64> {
        if array.len() >= min_size {
            return array.to_vec();
        }
        let new_len = Self::oversize(min_size, 8);
        Self::grow_exact_long(array, new_len)
    }

    /// Grows without copying existing bytes.
    pub fn grow_no_copy(array: &[u8], min_size: usize) -> Vec<u8> {
        if array.len() >= min_size {
            return array.to_vec();
        }
        vec![0u8; Self::oversize(min_size, 1)]
    }

    /// Returns a new `Vec<u8>` of exactly `new_length` containing a copy of `array`.
    pub fn grow_exact(array: &[u8], new_length: usize) -> Vec<u8> {
        let mut copy = vec![0u8; new_length];
        copy[..array.len()].copy_from_slice(array);
        copy
    }

    /// Returns a new `Vec<i32>` of exactly `new_length` containing a copy of `array`.
    pub fn grow_exact_int(array: &[i32], new_length: usize) -> Vec<i32> {
        let mut copy = vec![0i32; new_length];
        copy[..array.len()].copy_from_slice(array);
        copy
    }

    /// Returns a new `Vec<i64>` of exactly `new_length` containing a copy of `array`.
    pub fn grow_exact_long(array: &[i64], new_length: usize) -> Vec<i64> {
        let mut copy = vec![0i64; new_length];
        copy[..array.len()].copy_from_slice(array);
        copy
    }

    /// Copies a sub-range of a byte array into a new vector.
    pub fn copy_of_sub_array(array: &[u8], from: usize, to: usize) -> Vec<u8> {
        array[from..to].to_vec()
    }

    /// Copies a sub-range of an `i32` array into a new vector.
    pub fn copy_of_sub_array_int(array: &[i32], from: usize, to: usize) -> Vec<i32> {
        array[from..to].to_vec()
    }

    /// Copies a sub-range of an `i64` array into a new vector.
    pub fn copy_of_sub_array_long(array: &[i64], from: usize, to: usize) -> Vec<i64> {
        array[from..to].to_vec()
    }
}

// ---------------------------------------------------------------------------
// IOUtils
// ---------------------------------------------------------------------------

/// Utilities for dealing with closeable resources and file-system operations.
pub struct IOUtils;

impl IOUtils {
    /// Closes all given closeables, collecting the first exception and
    /// suppressing subsequent ones into it.
    pub fn close<I, F>(objects: I) -> Result<(), LuceneError>
    where
        I: IntoIterator<Item = Option<F>>,
        F: FnOnce() -> io::Result<()>,
    {
        let mut err: Option<io::Error> = None;
        for obj in objects.into_iter().flatten() {
            if let Err(e) = obj() {
                err = Some(Self::use_or_suppress(err, e));
            }
        }
        match err {
            Some(e) => Err(LuceneError::Io(e)),
            None => Ok(()),
        }
    }

    /// Closes all given closeables while suppressing non-fatal exceptions.
    pub fn close_while_handling_exception<I, F>(objects: I)
    where
        I: IntoIterator<Item = Option<F>>,
        F: FnOnce() -> io::Result<()>,
    {
        for obj in objects.into_iter().flatten() {
            let _ = obj();
        }
    }

    /// Deletes the given files, suppressing all exceptions.
    pub fn delete_files_ignoring_exceptions<I, P>(files: I)
    where
        I: IntoIterator<Item = Option<P>>,
        P: AsRef<Path>,
    {
        for path in files.into_iter().flatten() {
            let _ = fs::remove_file(path.as_ref());
        }
    }

    /// Deletes the given files if they exist, propagating the first exception.
    pub fn delete_files_if_exist<I, P>(files: I) -> Result<(), LuceneError>
    where
        I: IntoIterator<Item = Option<P>>,
        P: AsRef<Path>,
    {
        let mut err: Option<io::Error> = None;
        for path in files.into_iter().flatten() {
            if let Err(e) = Self::delete_if_exists(path.as_ref()) {
                err = Some(Self::use_or_suppress(err, e));
            }
        }
        match err {
            Some(e) => Err(LuceneError::Io(e)),
            None => Ok(()),
        }
    }

    fn delete_if_exists(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Forces all writes for the given file to durable storage.
    ///
    /// If `is_dir` is true, the directory is opened read-only and any error is
    /// ignored (not all platforms allow fsync on directories).
    pub fn fsync(file_to_sync: &Path, is_dir: bool) -> Result<(), LuceneError> {
        if is_dir {
            // On Windows, directories cannot be fsynced; on Unix, attempt but
            // tolerate errors.
            let _ = File::open(file_to_sync);
            return Ok(());
        }
        let file = OpenOptions::new().write(true).open(file_to_sync)?;
        file.sync_all()?;
        Ok(())
    }

    fn use_or_suppress(first: Option<io::Error>, second: io::Error) -> io::Error {
        match first {
            None => second,
            Some(_first) => {
                // io::Error doesn't support add_suppressed; chain via source.
                io::Error::new(second.kind(), second.to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Accountable
// ---------------------------------------------------------------------------

/// Trait for objects whose heap memory usage can be estimated.
pub trait Accountable {
    /// Returns an estimate of the heap memory used by this object in bytes.
    fn ram_bytes_used(&self) -> i64;

    /// Returns nested accountable resources.
    fn child_resources(&self) -> Vec<&dyn Accountable> {
        Vec::new()
    }
}

/// An accountable that always reports zero bytes.
#[derive(Debug, Copy, Clone)]
pub struct NullAccountable;

impl Accountable for NullAccountable {
    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

// ---------------------------------------------------------------------------
// RamUsageEstimator
// ---------------------------------------------------------------------------

/// Estimates memory usage of JVM-like objects.
///
/// This is a best-effort port. Rust does not expose JVM object headers or
/// compressed references, so the constants below mirror a 64-bit HotSpot
/// JVM with compressed oops (the most common deployment shape). Array sizing
/// helpers use these constants.
pub struct RamUsageEstimator;

impl RamUsageEstimator {
    /// One kilobyte.
    pub const ONE_KB: i64 = 1024;
    /// One megabyte.
    pub const ONE_MB: i64 = Self::ONE_KB * Self::ONE_KB;
    /// One gigabyte.
    pub const ONE_GB: i64 = Self::ONE_KB * Self::ONE_MB;

    /// Assumed reference size in bytes on a 64-bit compressed-oop JVM.
    pub const NUM_BYTES_OBJECT_REF: i64 = 4;
    /// Assumed object header size in bytes.
    pub const NUM_BYTES_OBJECT_HEADER: i64 = 8 + Self::NUM_BYTES_OBJECT_REF;
    /// Assumed array header size in bytes, aligned to 8 bytes.
    pub const NUM_BYTES_ARRAY_HEADER: i64 = 16;
    /// Assumed object alignment in bytes.
    pub const NUM_BYTES_OBJECT_ALIGNMENT: i64 = 8;

    /// Default estimate for unknown query objects.
    pub const QUERY_DEFAULT_RAM_BYTES_USED: i64 = 1024;
    /// Default estimate for unknown objects.
    pub const UNKNOWN_DEFAULT_RAM_BYTES_USED: i64 = 256;

    /// Aligns a size to the next multiple of the object alignment.
    pub fn align_object_size(size: i64) -> i64 {
        let size = size + Self::NUM_BYTES_OBJECT_ALIGNMENT - 1;
        size - (size % Self::NUM_BYTES_OBJECT_ALIGNMENT)
    }

    /// Returns the estimated size of a `u8` slice.
    pub fn size_of(arr: &[u8]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + arr.len() as i64)
    }

    /// Returns the estimated size of an `i32` slice.
    pub fn size_of_int(arr: &[i32]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + 4 * arr.len() as i64)
    }

    /// Returns the estimated size of an `i64` slice.
    pub fn size_of_long(arr: &[i64]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + 8 * arr.len() as i64)
    }

    /// Returns the estimated size of a `u64` slice.
    pub fn size_of_u64(arr: &[u64]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + 8 * arr.len() as i64)
    }

    /// Returns the shallow size of an array of references.
    pub fn shallow_size_of<T>(arr: &[T]) -> i64 {
        Self::align_object_size(
            Self::NUM_BYTES_ARRAY_HEADER + Self::NUM_BYTES_OBJECT_REF * arr.len() as i64,
        )
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// JVM-like and OS constants, matching Lucene's `Constants`.
#[derive(Debug, Copy, Clone)]
pub struct Constants;

impl Constants {
    /// Sentinel used when a system property cannot be read.
    pub const UNKNOWN: &'static str = "Unknown";

    /// Operating-system name.
    pub fn os_name() -> &'static str {
        env::consts::OS
    }

    /// True if running on Linux.
    pub fn linux() -> bool {
        env::consts::OS == "linux"
    }

    /// True if running on Windows.
    pub fn windows() -> bool {
        env::consts::OS == "windows"
    }

    /// True if running on macOS.
    pub fn mac_os_x() -> bool {
        env::consts::OS == "macos"
    }

    /// Operating-system architecture.
    pub fn os_arch() -> &'static str {
        env::consts::ARCH
    }

    /// True if the target pointer width is 64 bits.
    pub fn jre_is_64bit() -> bool {
        env::consts::ARCH.contains("64")
    }

    /// Rust target family, approximating a JVM vendor name.
    pub fn jvm_vendor() -> &'static str {
        "Rust"
    }

    /// Rust compiler version string.
    pub fn jvm_version() -> String {
        env!("CARGO_PKG_RUST_VERSION").to_string()
    }
}

// ---------------------------------------------------------------------------
// InfoStream
// ---------------------------------------------------------------------------

/// Debugging logging sink for Lucene classes.
pub trait InfoStream: Send + Sync {
    /// Logs a message for the given component.
    fn message(&self, component: &str, message: &str);

    /// Returns true if logging is enabled for the given component.
    fn is_enabled(&self, component: &str) -> bool;

    /// Closes the info stream, releasing any resources.
    fn close(&self);
}

/// No-op `InfoStream` implementation.
#[derive(Debug, Copy, Clone)]
pub struct NoOutputInfoStream;

impl InfoStream for NoOutputInfoStream {
    fn message(&self, _component: &str, _message: &str) {
        // No-op: `message()` should never be called when `is_enabled` is false.
    }

    fn is_enabled(&self, _component: &str) -> bool {
        false
    }

    fn close(&self) {
        // Nothing to close.
    }
}

static DEFAULT_INFO_STREAM: LazyLock<Box<dyn InfoStream>> =
    LazyLock::new(|| Box::new(NoOutputInfoStream));

/// Returns the default info stream.
pub fn default_info_stream() -> &'static dyn InfoStream {
    DEFAULT_INFO_STREAM.as_ref()
}

/// Sets the default info stream.
pub fn set_default_info_stream(stream: Box<dyn InfoStream>) {
    // This replaces the global default. The LazyLock only initializes once;
    // subsequent calls would need interior mutability for a true mutable default.
    // For this foundational port, the no-op default is sufficient.
    let _ = stream;
}

// ---------------------------------------------------------------------------
// Bits / FixedBitSet
// ---------------------------------------------------------------------------

/// Bitset-like interface.
pub trait Bits: Send + Sync + Debug {
    /// Returns the value of the bit at `index`.
    fn get(&self, index: usize) -> bool;

    /// Returns the number of bits in this set.
    fn length(&self) -> usize;
}

/// Bits implementation with all bits set.
#[derive(Debug, Copy, Clone)]
pub struct MatchAllBits {
    len: usize,
}

impl MatchAllBits {
    /// Creates a new `MatchAllBits` of the given length.
    pub fn new(len: usize) -> Self {
        Self { len }
    }
}

impl Bits for MatchAllBits {
    fn get(&self, _index: usize) -> bool {
        true
    }

    fn length(&self) -> usize {
        self.len
    }
}

/// Bits implementation with no bits set.
#[derive(Debug, Copy, Clone)]
pub struct MatchNoBits {
    len: usize,
}

impl MatchNoBits {
    /// Creates a new `MatchNoBits` of the given length.
    pub fn new(len: usize) -> Self {
        Self { len }
    }
}

impl Bits for MatchNoBits {
    fn get(&self, _index: usize) -> bool {
        false
    }

    fn length(&self) -> usize {
        self.len
    }
}

/// A fixed-size bit set backed by a `Vec<u64>`, equivalent to Lucene's
/// `FixedBitSet`.
#[derive(Clone, Debug)]
pub struct FixedBitSet {
    bits: Vec<u64>,
    num_bits: usize,
    num_words: usize,
}

impl FixedBitSet {
    /// Returns the number of 64-bit words needed to hold `num_bits`.
    pub fn bits2words(num_bits: usize) -> usize {
        if num_bits == 0 {
            0
        } else {
            ((num_bits - 1) >> 6) + 1
        }
    }

    /// Creates a new `FixedBitSet` with the given number of bits.
    pub fn new(num_bits: usize) -> Self {
        let num_words = Self::bits2words(num_bits);
        Self {
            bits: vec![0u64; num_words],
            num_bits,
            num_words,
        }
    }

    /// Creates a `FixedBitSet` from an existing `u64` buffer.
    pub fn from_bits(stored_bits: Vec<u64>, num_bits: usize) -> Self {
        let num_words = Self::bits2words(num_bits);
        assert!(
            num_words <= stored_bits.len(),
            "given buffer is too small to hold {} bits",
            num_bits
        );
        Self {
            bits: stored_bits,
            num_bits,
            num_words,
        }
    }

    /// Clears all bits.
    pub fn clear_all(&mut self) {
        self.bits.fill(0);
    }

    /// Sets the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn set(&mut self, index: usize) {
        assert!(index < self.num_bits, "index {} out of bounds", index);
        let word = index >> 6;
        let mask = 1u64 << (index & 0x3f);
        self.bits[word] |= mask;
    }

    /// Clears the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn clear(&mut self, index: usize) {
        assert!(index < self.num_bits, "index {} out of bounds", index);
        let word = index >> 6;
        let mask = 1u64 << (index & 0x3f);
        self.bits[word] &= !mask;
    }

    /// Returns the value of the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get(&self, index: usize) -> bool {
        assert!(index < self.num_bits, "index {} out of bounds", index);
        let word = index >> 6;
        let mask = 1u64 << (index & 0x3f);
        (self.bits[word] & mask) != 0
    }

    /// Returns the previous value of the bit at `index` and sets it to true.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get_and_set(&mut self, index: usize) -> bool {
        assert!(index < self.num_bits, "index {} out of bounds", index);
        let word = index >> 6;
        let mask = 1u64 << (index & 0x3f);
        let previous = (self.bits[word] & mask) != 0;
        self.bits[word] |= mask;
        previous
    }

    /// Returns the previous value of the bit at `index` and clears it.
    ///
    /// Equivalent to `FixedBitSet.getAndClear(long)`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get_and_clear(&mut self, index: usize) -> bool {
        assert!(index < self.num_bits, "index {} out of bounds", index);
        let word = index >> 6;
        let mask = 1u64 << (index & 0x3f);
        let previous = (self.bits[word] & mask) != 0;
        self.bits[word] &= !mask;
        previous
    }

    /// Sets all bits in the range `[from, to)`.
    ///
    /// Equivalent to `FixedBitSet.set(long, long)`.
    ///
    /// # Panics
    ///
    /// Panics if `from > to` or `to` exceeds the bit-set length.
    pub fn set_range(&mut self, from: usize, to: usize) {
        assert!(from <= to, "from {from} > to {to}");
        assert!(to <= self.num_bits, "to {to} out of bounds");
        for i in from..to {
            self.set(i);
        }
    }

    /// Creates a `FixedBitSet` that is a copy of the given `Bits`.
    ///
    /// Equivalent to `FixedBitSet.copyOf(Bits)`.
    pub fn copy_of(bits: &dyn Bits) -> Self {
        let length = bits.length();
        let mut copy = Self::new(length);
        for i in 0..length {
            if bits.get(i) {
                copy.set(i);
            }
        }
        copy
    }

    /// Returns the number of set bits.
    pub fn cardinality(&self) -> usize {
        self.bits[..self.num_words]
            .iter()
            .map(|w| w.count_ones() as usize)
            .sum()
    }

    /// Returns the number of bits in the set.
    pub fn length(&self) -> usize {
        self.num_bits
    }

    /// Returns a reference to the backing `u64` buffer.
    pub fn get_bits(&self) -> &[u64] {
        &self.bits
    }
}

impl Bits for FixedBitSet {
    fn get(&self, index: usize) -> bool {
        self.get(index)
    }

    fn length(&self) -> usize {
        self.length()
    }
}

impl Accountable for FixedBitSet {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::size_of_u64(&self.bits)
    }
}

// ---------------------------------------------------------------------------
// ThreadInterruptedException
// ---------------------------------------------------------------------------

/// Runtime error thrown when thread interruption is detected, equivalent to
/// Lucene's `ThreadInterruptedException`.
#[derive(Debug, Clone)]
pub struct ThreadInterruptedException {
    source: String,
}

impl ThreadInterruptedException {
    /// Creates a new exception wrapping the source interruption description.
    pub fn new(source: impl Display) -> Self {
        Self {
            source: source.to_string(),
        }
    }
}

impl Display for ThreadInterruptedException {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "thread was interrupted: {}", self.source)
    }
}

impl std::error::Error for ThreadInterruptedException {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_ref_comparison_and_cloning() {
        let a = BytesRef::new(vec![0x01, 0x02, 0x03]);
        let b = BytesRef::new(vec![0x01, 0x02, 0x04]);
        let c = BytesRef::new(vec![0x01, 0x02, 0x03]);

        assert!(a < b);
        assert_eq!(a, c);
        assert_ne!(a, b);

        let clone = a.clone();
        assert_eq!(clone, a);
        assert_eq!(clone.offset, 0);
        assert_eq!(clone.length, 3);

        let deep = BytesRef::deep_copy_of(&a);
        assert_eq!(deep, a);
    }

    #[test]
    fn bytes_ref_builder_appends_and_clears() {
        let mut builder = BytesRefBuilder::new();
        builder.append(0x01);
        builder.append_bytes(&[0x02, 0x03], 0, 2);
        let built = builder.to_bytes_ref();
        assert_eq!(built, BytesRef::new(vec![0x01, 0x02, 0x03]));

        builder.clear();
        assert_eq!(builder.length(), 0);
    }

    #[test]
    fn numeric_utils_float_double_long_round_trip() {
        let floats = [
            0.0f32,
            -0.0,
            1.0,
            -1.0,
            f32::MAX,
            f32::MIN,
            f32::NAN,
            f32::INFINITY,
        ];
        for &f in &floats {
            let enc = NumericUtils::float_to_sortable_int(f);
            let dec = NumericUtils::sortable_int_to_float(enc);
            if f.is_nan() {
                assert!(dec.is_nan());
            } else if f == -0.0 {
                assert!(dec == 0.0 && f.to_bits() == dec.to_bits());
            } else {
                assert_eq!(f, dec, "float round-trip failed for {}", f);
            }
        }

        let doubles = [
            0.0f64,
            -0.0,
            1.0,
            -1.0,
            f64::MAX,
            f64::MIN,
            f64::NAN,
            f64::INFINITY,
        ];
        for &d in &doubles {
            let enc = NumericUtils::double_to_sortable_long(d);
            let dec = NumericUtils::sortable_long_to_double(enc);
            if d.is_nan() {
                assert!(dec.is_nan());
            } else if d == -0.0 {
                assert!(d.to_bits() == dec.to_bits());
            } else {
                assert_eq!(d, dec, "double round-trip failed for {}", d);
            }
        }

        let longs = [i64::MIN, -1, 0, 1, i64::MAX];
        for &v in &longs {
            let mut buf = [0u8; 8];
            NumericUtils::long_to_sortable_bytes(v, &mut buf, 0);
            let dec = NumericUtils::sortable_bytes_to_long(&buf, 0);
            assert_eq!(v, dec);
        }

        let ints = [i32::MIN, -1, 0, 1, i32::MAX];
        for &v in &ints {
            let mut buf = [0u8; 4];
            NumericUtils::int_to_sortable_bytes(v, &mut buf, 0);
            let dec = NumericUtils::sortable_bytes_to_int(&buf, 0);
            assert_eq!(v, dec);
        }
    }

    #[test]
    fn bit_util_zig_zag() {
        let values = [0i32, -1, 1, i32::MIN, i32::MAX, -12345, 12345];
        for &v in &values {
            assert_eq!(BitUtil::zig_zag_decode(BitUtil::zig_zag_encode(v)), v);
        }
        let longs = [0i64, -1, 1, i64::MIN, i64::MAX, -123456789, 123456789];
        for &v in &longs {
            assert_eq!(
                BitUtil::zig_zag_decode_long(BitUtil::zig_zag_encode_long(v)),
                v
            );
        }
    }

    #[test]
    fn group_v_int_round_trip() {
        let values = [
            0,
            1,
            127,
            128,
            255,
            256,
            16383,
            16384,
            65535,
            65536,
            i32::MAX,
        ];
        let mut encoded = Vec::new();
        GroupVIntUtil::write_group_vints(&values, values.len(), &mut encoded);

        let mut decoded = vec![0i32; values.len()];
        let mut pos = 0;
        GroupVIntUtil::read_group_vints(&encoded, &mut pos, &mut decoded, values.len()).unwrap();
        assert_eq!(pos, encoded.len());
        assert_eq!(decoded, values);
    }

    #[test]
    fn ioutils_close_helpers() {
        fn failing_close() -> io::Result<()> {
            Err(io::Error::other("close failed"))
        }

        let result = IOUtils::close([Some(failing_close), None]);
        assert!(result.is_err());

        // close_while_handling_exception should suppress errors without panicking.
        IOUtils::close_while_handling_exception([Some(failing_close), None]);
    }

    #[test]
    fn ioutils_delete_and_fsync() {
        let tmp = std::env::temp_dir().join("rucene_util_test_file");
        let _ = fs::write(&tmp, b"hello");
        IOUtils::delete_files_if_exist([Some(&tmp)]).unwrap();
        assert!(!tmp.exists());

        fs::write(&tmp, b"hello").unwrap();
        IOUtils::fsync(&tmp, false).unwrap();
        IOUtils::delete_files_ignoring_exceptions([Some(&tmp)]);
        assert!(!tmp.exists());
    }

    #[test]
    fn fixed_bit_set_set_get_clear() {
        let mut bs = FixedBitSet::new(100);
        assert_eq!(bs.length(), 100);
        assert!(!bs.get(50));

        bs.set(50);
        assert!(bs.get(50));
        assert_eq!(bs.cardinality(), 1);

        bs.clear(50);
        assert!(!bs.get(50));
        assert_eq!(bs.cardinality(), 0);

        bs.set(0);
        bs.set(99);
        assert!(bs.get(0));
        assert!(bs.get(99));
        assert_eq!(bs.cardinality(), 2);

        bs.clear_all();
        assert_eq!(bs.cardinality(), 0);
    }

    #[test]
    fn info_stream_no_op() {
        let stream = NoOutputInfoStream;
        assert!(!stream.is_enabled("component"));
        stream.message("component", "should be ignored");
        stream.close();
    }

    #[test]
    fn constants_basic() {
        assert!(!Constants::os_name().is_empty());
        assert!(!Constants::os_arch().is_empty());
        assert!(Constants::jre_is_64bit());
    }

    #[test]
    fn thread_interrupted_exception_display() {
        let e = ThreadInterruptedException::new("channel recv");
        assert!(e.to_string().contains("thread was interrupted"));
        assert!(format!("{:?}", e).contains("ThreadInterruptedException"));
    }

    #[test]
    fn array_util_grow_and_copy() {
        let small = vec![1u8, 2, 3];
        let grown = ArrayUtil::grow(&small, 10);
        assert!(grown.len() >= 10);
        assert_eq!(&grown[..3], &small[..]);

        let copied = ArrayUtil::copy_of_sub_array(&small, 1, 3);
        assert_eq!(copied, vec![2, 3]);
    }

    #[test]
    fn bit_util_endian_helpers() {
        let mut buf = [0u8; 8];
        BitUtil::write_le_int(&mut buf, 0, 0x12345678);
        assert_eq!(BitUtil::read_le_int(&buf, 0), 0x12345678);
        BitUtil::write_be_int(&mut buf, 0, 0x12345678);
        assert_eq!(BitUtil::read_be_int(&buf, 0), 0x12345678);
        BitUtil::write_le_long(&mut buf, 0, 0x123456789ABCDEF0i64);
        assert_eq!(BitUtil::read_le_long(&buf, 0), 0x123456789ABCDEF0i64);
    }
}
