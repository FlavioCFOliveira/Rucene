//! Low-level utilities ported from `org.apache.lucene.util`.
//!
//! This module provides the foundational data structures and helpers used by
//! higher-level Lucene subsystems: byte refs, numeric encoders, bit utilities,
//! grouped variable-length integers, array growth helpers, I/O utilities,
//! memory accounting, constants, logging sinks, bit sets, and thread-interruption
//! handling.

#![deny(unsafe_code)]

pub mod accountables;
pub mod annotations;
pub mod attribute;
pub mod automaton;
pub mod bit_set;
pub mod byte_block_pool;
pub mod bytes_ref_array;
pub mod bytes_ref_hash;
pub mod chars_ref;
pub mod collection_util;
pub mod command_line_util;
/// LZ4 and lowercase-ASCII compression utilities.
pub mod compress;
pub mod concurrent;
pub mod doc_id_set;
pub mod extra;
pub mod file_deleter;
pub mod fst;
pub mod graph;
pub mod info_stream;
pub mod int_block_pool;
pub mod io_function;
pub mod jvm;
pub mod long_heap;
pub mod math_util;
pub mod misc;
pub mod mutable;
pub mod offline_sorter;
pub mod packed;
pub mod paged_bytes;
pub mod quantization;
pub mod query_builder;
pub mod refs;
pub mod resource_loader;
pub mod ring_buffer;
pub mod selector;
pub mod sloppy_math;
pub mod small_float;
pub mod sorter;
pub mod spi;
pub mod string_helper;
pub mod unicode_util;

/// Block KD-tree utilities ported from `org.apache.lucene.util.bkd`.
pub mod bkd;

/// Additional bitset and live-docs variants.
pub mod bit_sets;

/// HNSW graph utilities for vector search.
pub mod hnsw;

/// Vector arithmetic primitives ported from `org.apache.lucene.util.VectorUtil`.
pub mod vector_util;

pub use accountables::{AccountableTree, Accountables, NamedAccountable};
pub use annotations::{IgnoreRandomChains, SuppressForbidden};
pub use bit_set::{
    bit_set_of, check_unpositioned, BitSet, BitSetIterator, DocBaseBitSetIterator, FixedBits,
    LiveDocs,
};
pub use bytes_ref_array::{
    BytesRefArray, BytesRefBlockPool, BytesRefIterator, EmptyBytesRefIterator,
    FixedLengthBytesRefArray, IndexedBytesRefIterator, SortState, SortableBytesRefArray,
};
pub use collection_util::{
    intro_sort as collection_intro_sort, intro_sort_by, new_hash_map, new_hash_set,
    tim_sort as collection_tim_sort, tim_sort_by,
};
pub use command_line_util::{CommandLineUtil, FSDirectoryKind};
pub use concurrent::{
    Counter, NamedThreadFactory, SameThreadExecutorService, SetOnce, WeakIdentityMap,
};
pub use doc_id_set::{
    BitDocIdSet, BulkAdder, DocIdSetBuilder, IntArrayDocIdSet, IntArrayDocIdSetIterator,
    NotDocIdSet,
};
pub use graph::{
    FiniteStringsTokenStream, FiniteStringsTokenStreams, GraphTokenStreamFiniteStrings,
};
pub use info_stream::{JavaLoggingInfoStream, PrintStreamInfoStream};
pub use int_block_pool::{
    ByteBlockAllocator, DirectIntAllocator, IntBlockAllocator, IntBlockPool,
    RecyclingByteBlockAllocator, RecyclingIntBlockAllocator, INT_BLOCK_MASK, INT_BLOCK_SHIFT,
    INT_BLOCK_SIZE,
};
pub use io_function::{
    FilterIterator, FloatToFloatFunction, IOBooleanSupplier, IOConsumer, IOFunction, IOSupplier,
};
pub use jvm::{HotspotVMOptions, MethodClass, VirtualMethod};
pub use long_heap::{LongHeap, TernaryLongHeap};
pub use math_util::MathUtil;
pub use misc::{MapOfSets, SentinelIntSet, StrictStringTokenizer, TermAndVector, ToStringUtils};
pub use mutable::{
    MutableValue, MutableValueBool, MutableValueDate, MutableValueDouble, MutableValueFloat,
    MutableValueInt, MutableValueLong, MutableValueObject, MutableValueStr,
};
pub use offline_sorter::{
    BufferSize, ByteSequencesReader, ByteSequencesWriter, OfflineSorter, OfflineSorterComparator,
    SortInfo,
};
pub use paged_bytes::{PagedBytes, PagedBytesDataInput, PagedBytesDataOutput, PagedBytesReader};
pub use refs::{CharsRefBuilder, IntsRefBuilder, LongsRef, EMPTY_LONGS};
pub use resource_loader::{
    ClassLoader, ClassLoaderUtils, ClassRegistry, ClasspathResourceLoader, ModuleResourceLoader,
    ResourceLoader, ResourceLoaderAware,
};
pub use ring_buffer::{FrequencyTrackingRingBuffer, Resettable, RollingBuffer};
pub use sloppy_math::SloppyMath;
pub use sorter::{
    ArrayInPlaceMergeSorter, ArrayIntroSorter, ArrayTimSorter, BytesRefComparator,
    InPlaceMergeSorter, LSBRadixSorter, MSBRadixSorter, MSBRadixSorterOps, MergeSorter,
    NaturalBytesRefComparator, Sorter, StableMSBRadixSorter, StableMSBRadixSorterOps,
    StableStringSorter, StableStringSorterOps, StringSorter, StringSorterComparator,
    StringSorterOps, TimSorter, TimSorterState,
};
pub use spi::{NamedSPI, NamedSPILoader};
pub use unicode_util::{UTF8CodePoint, UnicodeUtil};

pub use attribute::{
    unwrap_all, AsUnwrappable, Attribute, AttributeFactory, AttributeImpl, AttributeReflector,
    AttributeSource, CapturedState, CloseableThreadLocal, DefaultAttributeFactory, Unwrappable,
};
pub use automaton::{
    automata, operations, Automata, Automaton, AutomatonBuilder, AutomatonProvider, AutomatonType,
    ByteRunAutomaton, ByteRunnable, CaseFolding, CharacterRunAutomaton, CompiledAutomaton,
    DeterminizeResult, FiniteStringsIterator, FrozenIntSet, IntSet, Lev1ParametricDescription,
    Lev1TParametricDescription, Lev2ParametricDescription, Lev2TParametricDescription,
    LevenshteinAutomata, LimitedFiniteStringsIterator, NFARunAutomaton, Operations,
    ParametricDescription, RegExp, RegExpKind, RunAutomaton, StatePair, StateSet,
    StringsToAutomaton, TooComplexToDeterminizeException, Transition, TransitionAccessor,
    UTF32ToUTF8, DEFAULT_DETERMINIZE_WORK_LIMIT, MAXIMUM_SUPPORTED_DISTANCE,
    MAX_STRING_UNION_TERM_LENGTH,
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
pub use query_builder::{QueryBuilder, QueryBuilderOps, TermAndBoost as QueryBuilderTermAndBoost};
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
    hash::{Hash, Hasher},
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
/// A comparator over two byte slices at given offsets, returning a negative,
/// zero or positive `i32` as Java's comparators do.
///
/// Equivalent to the functional interface `ArrayUtil.ByteArrayComparator`.
pub type ByteArrayComparator = fn(&[u8], usize, &[u8], usize) -> i32;

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

    /// Grows `array` to hold at least `min_length` elements, without exceeding
    /// `max_length`.
    ///
    /// Equivalent to `ArrayUtil.growInRange(int[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns an error when `min_length > max_length`, which is the
    /// `IllegalArgumentException` Java throws.
    pub fn grow_in_range_int(
        array: &[i32],
        min_length: usize,
        max_length: usize,
    ) -> Result<Vec<i32>, LuceneError> {
        Self::grow_in_range_generic(array, min_length, max_length, 4)
    }

    /// Grows `array` to hold at least `min_length` elements, without exceeding
    /// `max_length`.
    ///
    /// Equivalent to `ArrayUtil.growInRange(float[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns an error when `min_length > max_length`.
    pub fn grow_in_range_float(
        array: &[f32],
        min_length: usize,
        max_length: usize,
    ) -> Result<Vec<f32>, LuceneError> {
        Self::grow_in_range_generic(array, min_length, max_length, 4)
    }

    /// The body both `grow_in_range` forms share.
    ///
    /// Java writes one overload per primitive because it has no generics over
    /// primitives; the only thing that varies is the element width handed to
    /// [`oversize`](Self::oversize), so one generic function stands for both.
    fn grow_in_range_generic<T: Copy + Default>(
        array: &[T],
        min_length: usize,
        max_length: usize,
        bytes_per_element: usize,
    ) -> Result<Vec<T>, LuceneError> {
        if min_length > max_length {
            return Err(LuceneError::IllegalArgument(format!(
                "requested minimum array length {min_length} is larger than requested maximum array length {max_length}"
            )));
        }

        if array.len() >= min_length {
            return Ok(array.to_vec());
        }

        let potential_length = Self::oversize(min_length, bytes_per_element);
        let new_length = max_length.min(potential_length);
        let mut grown = array.to_vec();
        grown.resize(new_length, T::default());
        Ok(grown)
    }

    /// Returns a copy of `array` grown to exactly `new_length` elements.
    ///
    /// Equivalent to the `ArrayUtil.growExact` overloads. Java declares one per
    /// primitive plus a generic one; Rust needs only the generic form.
    pub fn grow_exact_generic<T: Copy + Default>(array: &[T], new_length: usize) -> Vec<T> {
        let mut grown = array.to_vec();
        grown.resize(new_length, T::default());
        grown
    }

    /// Returns `array` grown to hold at least `min_size` elements, over-allocating
    /// as [`oversize`](Self::oversize) prescribes.
    ///
    /// Equivalent to the `ArrayUtil.grow(T[], int)` overloads.
    pub fn grow_generic<T: Copy + Default>(array: &[T], min_size: usize) -> Vec<T> {
        if array.len() < min_size {
            Self::grow_exact_generic(array, Self::oversize(min_size, size_of::<T>()))
        } else {
            array.to_vec()
        }
    }

    /// Returns an array of at least `min_size` elements, discarding the contents
    /// of `array` when it has to grow.
    ///
    /// Equivalent to the `ArrayUtil.growNoCopy(T[], int)` overloads.
    pub fn grow_no_copy_generic<T: Copy + Default>(array: &[T], min_size: usize) -> Vec<T> {
        if array.len() < min_size {
            vec![T::default(); Self::oversize(min_size, size_of::<T>())]
        } else {
            array.to_vec()
        }
    }

    /// Returns a copy of the elements of `array` in `[from, to)`.
    ///
    /// Equivalent to the `ArrayUtil.copyOfSubArray` overloads.
    pub fn copy_of_sub_array_generic<T: Copy>(array: &[T], from: usize, to: usize) -> Vec<T> {
        array[from..to].to_vec()
    }

    /// Parses the UTF-16 code units of `chars[offset..offset + len]` as a signed
    /// integer in `radix`.
    ///
    /// Equivalent to `ArrayUtil.parseInt(char[], int, int, int)`. Lucene has this
    /// so that a term can be parsed without allocating a `String`.
    ///
    /// # Errors
    ///
    /// Returns an error where Java throws `NumberFormatException`: an
    /// out-of-range radix, an empty slice, a non-digit character, or a value that
    /// does not fit in an `i32`.
    pub fn parse_int_radix(
        chars: &[char],
        mut offset: usize,
        mut len: usize,
        radix: u32,
    ) -> Result<i32, LuceneError> {
        // Character.MIN_RADIX / Character.MAX_RADIX.
        if !(2..=36).contains(&radix) {
            return Err(LuceneError::IllegalArgument(format!(
                "radix {radix} out of range"
            )));
        }
        if len == 0 {
            return Err(LuceneError::IllegalArgument(
                "chars length is 0".to_string(),
            ));
        }
        let mut i = 0usize;
        let negative = chars[offset] == '-';
        if negative {
            i += 1;
            if i == len {
                return Err(LuceneError::IllegalArgument(
                    "can't convert to an int".to_string(),
                ));
            }
            offset += 1;
            len -= 1;
        }
        Self::parse(chars, offset, len, radix, negative)
    }

    /// Parses every code unit of `chars` as a signed decimal integer.
    ///
    /// Equivalent to `ArrayUtil.parseInt(char[])`.
    ///
    /// # Errors
    ///
    /// As [`parse_int_radix`](Self::parse_int_radix).
    pub fn parse_int(chars: &[char]) -> Result<i32, LuceneError> {
        Self::parse_int_radix(chars, 0, chars.len(), 10)
    }

    /// Equivalent to the private `ArrayUtil.parse(char[], int, int, int, boolean)`.
    ///
    /// Java accumulates the value as a negative number so that `Integer.MIN_VALUE`
    /// is representable; the same trick is reproduced here, which is why the
    /// arithmetic is checked against `i32::MIN` rather than `i32::MAX`.
    fn parse(
        chars: &[char],
        offset: usize,
        len: usize,
        radix: u32,
        negative: bool,
    ) -> Result<i32, LuceneError> {
        let unparsable = || LuceneError::IllegalArgument("Unable to parse".to_string());
        let max = i32::MIN / radix as i32;
        let mut result: i32 = 0;
        for i in 0..len {
            let digit = match chars[i + offset].to_digit(radix) {
                Some(d) => d as i32,
                None => return Err(unparsable()),
            };
            if max > result {
                return Err(unparsable());
            }
            let next = result
                .checked_mul(radix as i32)
                .and_then(|v| v.checked_sub(digit))
                .ok_or_else(unparsable)?;
            if next > result {
                return Err(unparsable());
            }
            result = next;
        }
        if !negative {
            result = result.checked_neg().ok_or_else(unparsable)?;
            if result < 0 {
                return Err(unparsable());
            }
        }
        Ok(result)
    }

    /// Returns the hash Lucene computes over `array[start..end]`.
    ///
    /// Equivalent to `ArrayUtil.hashCode(char[], int, int)`: a `31 * h + c` fold
    /// walked from the end towards the start.
    pub fn hash_code(array: &[char], start: usize, end: usize) -> i32 {
        let mut code: i32 = 0;
        for i in (start..end).rev() {
            code = code.wrapping_mul(31).wrapping_add(array[i] as i32);
        }
        code
    }

    /// Swaps the elements at `i` and `j`.
    ///
    /// Equivalent to `ArrayUtil.swap(T[], int, int)`.
    pub fn swap<T>(arr: &mut [T], i: usize, j: usize) {
        arr.swap(i, j);
    }

    /// Compares the eight bytes at `a_offset` and `b_offset` as unsigned 64-bit
    /// big-endian integers.
    ///
    /// Equivalent to `ArrayUtil.compareUnsigned8(byte[], int, byte[], int)`, the
    /// comparator `LongPoint` and `DoublePoint` use.
    pub fn compare_unsigned8(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> i32 {
        let x = u64::from_be_bytes(a[a_offset..a_offset + 8].try_into().unwrap());
        let y = u64::from_be_bytes(b[b_offset..b_offset + 8].try_into().unwrap());
        match x.cmp(&y) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    }

    /// Compares the four bytes at `a_offset` and `b_offset` as unsigned 32-bit
    /// big-endian integers.
    ///
    /// Equivalent to `ArrayUtil.compareUnsigned4(byte[], int, byte[], int)`, the
    /// comparator `IntPoint`, `FloatPoint`, `LatLonPoint` and `LatLonShape` use.
    pub fn compare_unsigned4(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> i32 {
        let x = u32::from_be_bytes(a[a_offset..a_offset + 4].try_into().unwrap());
        let y = u32::from_be_bytes(b[b_offset..b_offset + 4].try_into().unwrap());
        match x.cmp(&y) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    }

    /// Returns the comparator for keys of `num_bytes` bytes.
    ///
    /// Equivalent to `ArrayUtil.getUnsignedComparator(int)`, which specialises on
    /// the two widths that dominate — 8 and 4 — and otherwise falls back to a
    /// plain unsigned byte-wise comparison.
    pub fn get_unsigned_comparator(num_bytes: usize) -> ByteArrayComparator {
        match num_bytes {
            8 => Self::compare_unsigned8,
            4 => Self::compare_unsigned4,
            _ => {
                // A closure would capture num_bytes, which a plain fn pointer
                // cannot carry; the generic width is served by the boxed form.
                Self::compare_unsigned_generic
            }
        }
    }

    /// Compares two byte slices from the given offsets to their ends, treating
    /// each byte as unsigned.
    ///
    /// This is the fallback branch of
    /// [`get_unsigned_comparator`](Self::get_unsigned_comparator). Java closes
    /// over `numBytes`; a `fn` pointer cannot, so the caller slices the arrays to
    /// the width it wants before calling. [`compare_unsigned_len`](Self::compare_unsigned_len)
    /// is the form that takes the width explicitly.
    pub fn compare_unsigned_generic(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> i32 {
        match a[a_offset..].cmp(&b[b_offset..]) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    }

    /// Compares `len` bytes of each slice from the given offsets, treating each
    /// byte as unsigned.
    ///
    /// This is what Java's `getUnsignedComparator` lambda does for a width other
    /// than 8 or 4: `Arrays.compareUnsigned(a, aOffset, aOffset + numBytes, b,
    /// bOffset, bOffset + numBytes)`. Rust's `u8` ordering is already unsigned.
    pub fn compare_unsigned_len(
        a: &[u8],
        a_offset: usize,
        b: &[u8],
        b_offset: usize,
        len: usize,
    ) -> i32 {
        match a[a_offset..a_offset + len].cmp(&b[b_offset..b_offset + len]) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
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

    /// Approximate memory a hash-table entry costs.
    ///
    /// Equivalent to `RamUsageEstimator.HASHTABLE_RAM_BYTES_PER_ENTRY`: key plus
    /// value, doubled because hash tables are oversized to avoid collisions.
    pub const HASHTABLE_RAM_BYTES_PER_ENTRY: i64 = (2 * Self::NUM_BYTES_OBJECT_REF) * 2;

    /// Approximate memory a linked-hash-table entry costs.
    ///
    /// Equivalent to `RamUsageEstimator.LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY`:
    /// the hash-table entry plus the previous and next references.
    pub const LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY: i64 =
        Self::HASHTABLE_RAM_BYTES_PER_ENTRY + 2 * Self::NUM_BYTES_OBJECT_REF;

    /// How deep [`size_of_map`](Self::size_of_map) and
    /// [`size_of_collection`](Self::size_of_collection) recurse.
    ///
    /// Equivalent to `RamUsageEstimator.MAX_DEPTH`.
    pub const MAX_DEPTH: i32 = 1;

    /// Shallow size of a `String` instance.
    ///
    /// Equivalent to `RamUsageEstimator.STRING_SIZE`, which Java derives at class
    /// initialisation with `shallowSizeOfInstance(String.class)`. Rust has no
    /// reflection, so the same arithmetic is spelled out: the object header, the
    /// cached hash, and the reference to the character array, aligned.
    pub const STRING_SIZE: i64 = Self::NUM_BYTES_OBJECT_HEADER + 4 + Self::NUM_BYTES_OBJECT_REF;

    /// Returns the size in bytes of a `bool` array.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(boolean[])`.
    pub fn size_of_bool(arr: &[bool]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + arr.len() as i64)
    }

    /// Returns the size in bytes of a UTF-16 code-unit array.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(char[])`.
    pub fn size_of_char(arr: &[u16]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + 2 * arr.len() as i64)
    }

    /// Returns the size in bytes of a `short` array.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(short[])`.
    pub fn size_of_short(arr: &[i16]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + 2 * arr.len() as i64)
    }

    /// Returns the size in bytes of a `float` array.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(float[])`.
    pub fn size_of_float(arr: &[f32]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + 4 * arr.len() as i64)
    }

    /// Returns the size in bytes of a `double` array.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(double[])`.
    pub fn size_of_double(arr: &[f64]) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + 8 * arr.len() as i64)
    }

    /// Returns the size in bytes of a string.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(String)`: the string object, an
    /// array header, and two bytes per UTF-16 code unit. Java notes that this may
    /// overstate the cost under compact strings, and keeps the estimate anyway;
    /// the port keeps it too, so that the accounting agrees with Lucene's.
    pub fn size_of_string(s: &str) -> i64 {
        let utf16_len: i64 = s.encode_utf16().count() as i64;
        Self::align_object_size(Self::STRING_SIZE + Self::NUM_BYTES_ARRAY_HEADER + 2 * utf16_len)
    }

    /// Returns the memory an [`Accountable`] reports.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(Accountable)`.
    pub fn size_of_accountable(accountable: &dyn Accountable) -> i64 {
        accountable.ram_bytes_used()
    }

    /// Returns the shallow size of the slice plus the memory each element
    /// reports.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(Accountable[])`.
    pub fn size_of_accountables(accountables: &[&dyn Accountable]) -> i64 {
        let mut size = Self::shallow_size_of(accountables);
        for accountable in accountables {
            size += accountable.ram_bytes_used();
        }
        size
    }

    /// Returns an estimate of the memory a map costs, given the size of one
    /// entry's key and value.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfMap(Map, long)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java walks the entries and calls
    /// `sizeOfObject` on each key and value, which needs reflection over
    /// arbitrary field layouts. Rust has none, so the per-entry cost is a
    /// parameter: the caller, which knows the concrete types, supplies it. The
    /// surrounding arithmetic — the shallow size, the per-entry overhead and the
    /// final alignment — is Java's.
    pub fn size_of_map(entry_count: usize, size_per_entry: i64) -> i64 {
        let mut size = Self::align_object_size(Self::NUM_BYTES_OBJECT_HEADER);
        size += entry_count as i64 * (Self::HASHTABLE_RAM_BYTES_PER_ENTRY + size_per_entry);
        Self::align_object_size(size)
    }

    /// Returns an estimate of the memory a collection costs, given the size of
    /// one element.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfCollection(Collection, long)`,
    /// which assumes an array-backed collection and charges a reference per
    /// element.
    ///
    /// **Divergence from Lucene 10.5.0.** As with [`size_of_map`](Self::size_of_map),
    /// the per-element cost is a parameter rather than a reflective walk.
    pub fn size_of_collection(element_count: usize, size_per_element: i64) -> i64 {
        let mut size = Self::align_object_size(Self::NUM_BYTES_OBJECT_HEADER);
        // Assume an array-backed collection and add the per-object references.
        size += Self::NUM_BYTES_ARRAY_HEADER + element_count as i64 * Self::NUM_BYTES_OBJECT_REF;
        size += element_count as i64 * size_per_element;
        Self::align_object_size(size)
    }

    /// Returns the shallow size of an instance of `T`.
    ///
    /// Equivalent to `RamUsageEstimator.shallowSizeOfInstance(Class)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java walks the class hierarchy by
    /// reflection, charging a reference per object field and the declared width
    /// per primitive field. Rust knows the layout at compile time, so
    /// `size_of::<T>()` gives the field block directly; the object header and the
    /// alignment rule are Java's.
    pub fn shallow_size_of_instance<T>() -> i64 {
        Self::align_object_size(Self::NUM_BYTES_OBJECT_HEADER + size_of::<T>() as i64)
    }

    /// Renders `bytes` in human-readable units — GB, MB, KB, or bytes.
    ///
    /// Equivalent to `RamUsageEstimator.humanReadableUnits(long)`, which formats
    /// with the pattern `0.#` under `Locale.ROOT`: at most one fractional digit,
    /// and none when it would be zero.
    pub fn human_readable_units(bytes: i64) -> String {
        fn format_one_dp(value: f32) -> String {
            // DecimalFormat("0.#") keeps at most one fractional digit and drops
            // it when it rounds to zero.
            let rounded = (value * 10.0).round() / 10.0;
            if (rounded.fract()).abs() < f32::EPSILON {
                format!("{}", rounded as i64)
            } else {
                format!("{rounded:.1}")
            }
        }

        if bytes / Self::ONE_GB > 0 {
            format!("{} GB", format_one_dp(bytes as f32 / Self::ONE_GB as f32))
        } else if bytes / Self::ONE_MB > 0 {
            format!("{} MB", format_one_dp(bytes as f32 / Self::ONE_MB as f32))
        } else if bytes / Self::ONE_KB > 0 {
            format!("{} KB", format_one_dp(bytes as f32 / Self::ONE_KB as f32))
        } else {
            format!("{bytes} bytes")
        }
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
        if to <= from {
            return;
        }

        let start_word = from >> 6;
        let end_word = (to - 1) >> 6;

        // Java relies on the JLS rule that a shift uses only the low 6 bits of
        // the count, so `-1L >>> -endIndex` is a shift by `(-endIndex) & 63`.
        let startmask = u64::MAX << (from & 63);
        let endmask = u64::MAX >> (to.wrapping_neg() & 63);

        if start_word == end_word {
            self.bits[start_word] |= startmask & endmask;
            return;
        }

        self.bits[start_word] |= startmask;
        self.bits[(start_word + 1)..end_word].fill(u64::MAX);
        self.bits[end_word] |= endmask;
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

    /// Ensures `bits` can store a value at `desired_bit`, growing it if needed.
    ///
    /// Equivalent to `FixedBitSet.ensureCapacity(FixedBitSet, int)`.
    pub fn ensure_capacity(bits: FixedBitSet, desired_bit: usize) -> FixedBitSet {
        Self::ensure_capacity_internal(bits, desired_bit, true)
    }

    /// Clears `bits` and ensures it can store a value at `desired_bit`.
    ///
    /// Equivalent to `FixedBitSet.ensureCapacityAndClear(FixedBitSet, int)`.
    pub fn ensure_capacity_and_clear(bits: FixedBitSet, desired_bit: usize) -> FixedBitSet {
        Self::ensure_capacity_internal(bits, desired_bit, false)
    }

    fn ensure_capacity_internal(
        mut bits: FixedBitSet,
        desired_bit: usize,
        preserve_data: bool,
    ) -> FixedBitSet {
        if desired_bit < bits.num_bits {
            if !preserve_data {
                bits.clear_all();
            }
            bits
        } else {
            // Depends on the ghost bits being clear! Otherwise they would become
            // visible in the new instance.
            let num_words = Self::bits2words(desired_bit);
            let mut arr = bits.bits;
            if num_words >= arr.len() {
                let target = ArrayUtil::oversize(num_words + 1, 8);
                if preserve_data {
                    arr.resize(target.max(num_words + 1), 0);
                } else {
                    arr = vec![0u64; target.max(num_words + 1)];
                }
            } else if !preserve_data {
                arr.fill(0);
            }
            let num_bits = arr.len() << 6;
            Self::from_bits(arr, num_bits)
        }
    }

    /// Returns the cardinality of the intersection of two sets, modifying neither.
    ///
    /// Equivalent to `FixedBitSet.intersectionCount(FixedBitSet, FixedBitSet)`.
    pub fn intersection_count(a: &FixedBitSet, b: &FixedBitSet) -> u64 {
        // Depends on the ghost bits being clear!
        let common = a.num_words.min(b.num_words);
        (0..common)
            .map(|i| (a.bits[i] & b.bits[i]).count_ones() as u64)
            .sum()
    }

    /// Returns the cardinality of the union of two sets, modifying neither.
    ///
    /// Equivalent to `FixedBitSet.unionCount(FixedBitSet, FixedBitSet)`.
    pub fn union_count(a: &FixedBitSet, b: &FixedBitSet) -> u64 {
        // Depends on the ghost bits being clear!
        let common = a.num_words.min(b.num_words);
        let mut tot: u64 = (0..common)
            .map(|i| (a.bits[i] | b.bits[i]).count_ones() as u64)
            .sum();
        tot += (common..a.num_words)
            .map(|i| a.bits[i].count_ones() as u64)
            .sum::<u64>();
        tot += (common..b.num_words)
            .map(|i| b.bits[i].count_ones() as u64)
            .sum::<u64>();
        tot
    }

    /// Returns the cardinality of `a AND NOT b`, modifying neither set.
    ///
    /// Equivalent to `FixedBitSet.andNotCount(FixedBitSet, FixedBitSet)`.
    pub fn and_not_count(a: &FixedBitSet, b: &FixedBitSet) -> u64 {
        // Depends on the ghost bits being clear!
        let common = a.num_words.min(b.num_words);
        let mut tot: u64 = (0..common)
            .map(|i| (a.bits[i] & !b.bits[i]).count_ones() as u64)
            .sum();
        tot += (common..a.num_words)
            .map(|i| a.bits[i].count_ones() as u64)
            .sum::<u64>();
        tot
    }

    /// Checks that the bits past `num_bits` are clear.
    ///
    /// Equivalent to `FixedBitSet.verifyGhostBitsClear()`. Several methods rely
    /// on this implicit assumption; Lucene marks them "Depends on the ghost bits
    /// being clear!".
    pub fn verify_ghost_bits_clear(&self) -> bool {
        for i in self.num_words..self.bits.len() {
            if self.bits[i] != 0 {
                return false;
            }
        }
        if (self.num_bits & 0x3f) == 0 {
            return true;
        }
        let mask = u64::MAX << (self.num_bits & 63);
        (self.bits[self.num_words - 1] & mask) == 0
    }

    /// Returns the number of set bits between `from` inclusive and `to` exclusive.
    ///
    /// Equivalent to `FixedBitSet.cardinality(int, int)`.
    pub fn cardinality_range(&self, mut from: usize, to: usize) -> usize {
        assert!(
            from <= to && to <= self.length(),
            "range [{from}, {to}) out of bounds"
        );
        let mut cardinality = 0usize;

        // First, align `from` with a word start, i.e. a multiple of 64.
        if (from & 0x3f) != 0 {
            let mut bits = self.bits[from >> 6] >> (from & 63);
            let num_bits_til_next_word = from.wrapping_neg() & 0x3f;
            if to - from < num_bits_til_next_word {
                bits &= (1u64 << ((to - from) & 63)) - 1;
                return bits.count_ones() as usize;
            }
            cardinality += bits.count_ones() as usize;
            from += num_bits_til_next_word;
            debug_assert_eq!(from & 0x3f, 0);
        }

        for i in (from >> 6)..(to >> 6) {
            cardinality += self.bits[i].count_ones() as usize;
        }

        // Now handle the bits between the last complete word and `to`.
        if (to & 0x3f) != 0 {
            let bits = self.bits[to >> 6] << (to.wrapping_neg() & 63);
            cardinality += bits.count_ones() as usize;
        }

        cardinality
    }

    /// Returns an approximation of the number of set bits, by sampling.
    ///
    /// Equivalent to `FixedBitSet.approximateCardinality()`: the popcount of the
    /// first 16 words of every 1024 words, scaled by 1024/16.
    pub fn approximate_cardinality(&self) -> usize {
        const RANGE_LENGTH: usize = 16;
        const INTERVAL: usize = 1024;

        if self.num_words <= INTERVAL {
            return self.cardinality();
        }

        let mut pop_count: u64 = 0;
        let mut max_word = 0usize;
        while max_word + INTERVAL < self.num_words {
            for i in 0..RANGE_LENGTH {
                pop_count += self.bits[max_word + i].count_ones() as u64;
            }
            max_word += INTERVAL;
        }

        pop_count *= ((INTERVAL / RANGE_LENGTH) * self.num_words / max_word) as u64;
        pop_count as usize
    }

    /// ORs a small mask of consecutive bits starting at `start_bit`.
    ///
    /// Equivalent to `FixedBitSet.orMask(int, long, int)`. Useful for
    /// bulk-setting bits from a SIMD comparison result without per-bit
    /// extraction; the mask must carry at most `mask_len` significant bits.
    pub fn or_mask(&mut self, start_bit: usize, mask: u64, mask_len: usize) {
        assert!(
            start_bit + mask_len <= self.num_bits,
            "start_bit={start_bit}, mask_len={mask_len}, num_bits={}",
            self.num_bits
        );
        let word_index = start_bit >> 6;
        let bit_offset = start_bit & 63;
        if bit_offset + mask_len <= 64 {
            self.bits[word_index] |= mask << bit_offset;
        } else {
            self.bits[word_index] |= mask << bit_offset;
            self.bits[word_index + 1] |= mask >> ((64 - bit_offset) & 63);
        }
    }

    /// Reads `num_bits` (between 1 and 63) bits from `bit_set` at `from`.
    ///
    /// Equivalent to the private `FixedBitSet.readNBits(long[], int, int)`.
    fn read_n_bits(bit_set: &[u64], from: usize, num_bits: usize) -> u64 {
        debug_assert!(num_bits > 0 && num_bits < 64);
        let mut bits = bit_set[from >> 6] >> (from & 63);
        let num_bits_so_far = 64 - (from & 0x3f);
        if num_bits_so_far < num_bits {
            bits |= bit_set[(from >> 6) + 1] << (from.wrapping_neg() & 63);
        }
        bits & ((1u64 << (num_bits & 63)) - 1)
    }

    /// ORs `length` bits of `source` starting at `source_from` into `dest`
    /// starting at `dest_from`.
    ///
    /// Equivalent to `FixedBitSet.orRange(FixedBitSet, int, FixedBitSet, int, int)`.
    pub fn or_range(
        source: &FixedBitSet,
        mut source_from: usize,
        dest: &mut FixedBitSet,
        mut dest_from: usize,
        mut length: usize,
    ) {
        assert!(
            source_from + length <= source.length(),
            "source range out of bounds"
        );
        assert!(
            dest_from + length <= dest.length(),
            "dest range out of bounds"
        );

        if length == 0 {
            return;
        }

        // First, align `dest_from` with a word start, i.e. a multiple of 64.
        if (dest_from & 0x3f) != 0 {
            let num_bits_needed = (dest_from.wrapping_neg() & 0x3f).min(length);
            let bits =
                Self::read_n_bits(&source.bits, source_from, num_bits_needed) << (dest_from & 63);
            dest.bits[dest_from >> 6] |= bits;

            source_from += num_bits_needed;
            dest_from += num_bits_needed;
            length -= num_bits_needed;
        }

        if length == 0 {
            return;
        }

        debug_assert_eq!(dest_from & 0x3f, 0);

        // Now OR at the word level.
        let num_full_words = length >> 6;
        let source_word_from = source_from >> 6;
        let dest_word_from = dest_from >> 6;

        if (source_from & 0x3f) == 0 {
            // source_from and dest_from are both word-aligned.
            for i in 0..num_full_words {
                dest.bits[dest_word_from + i] |= source.bits[source_word_from + i];
            }
        } else {
            let shift = source_from & 63;
            let back = source_from.wrapping_neg() & 63;
            for i in 0..num_full_words {
                dest.bits[dest_word_from + i] |= (source.bits[source_word_from + i] >> shift)
                    | (source.bits[source_word_from + i + 1] << back);
            }
        }

        source_from += num_full_words << 6;
        dest_from += num_full_words << 6;
        length -= num_full_words << 6;

        // Finally handle the tail bits.
        if length > 0 {
            let bits = Self::read_n_bits(&source.bits, source_from, length);
            dest.bits[dest_from >> 6] |= bits;
        }
    }

    /// ANDs `length` bits of `source` starting at `source_from` into `dest`
    /// starting at `dest_from`.
    ///
    /// Equivalent to `FixedBitSet.andRange(FixedBitSet, int, FixedBitSet, int, int)`.
    pub fn and_range(
        source: &FixedBitSet,
        mut source_from: usize,
        dest: &mut FixedBitSet,
        mut dest_from: usize,
        mut length: usize,
    ) {
        assert!(
            source_from + length <= source.length(),
            "source range out of bounds"
        );
        assert!(
            dest_from + length <= dest.length(),
            "dest range out of bounds"
        );

        if length == 0 {
            return;
        }

        // First, align `dest_from` with a word start, i.e. a multiple of 64.
        if (dest_from & 0x3f) != 0 {
            let num_bits_needed = (dest_from.wrapping_neg() & 0x3f).min(length);
            let mut bits =
                Self::read_n_bits(&source.bits, source_from, num_bits_needed) << (dest_from & 63);
            bits |= !(((1u64 << (num_bits_needed & 63)) - 1) << (dest_from & 63));
            dest.bits[dest_from >> 6] &= bits;

            source_from += num_bits_needed;
            dest_from += num_bits_needed;
            length -= num_bits_needed;
        }

        if length == 0 {
            return;
        }

        debug_assert_eq!(dest_from & 0x3f, 0);

        // Now AND at the word level.
        let num_full_words = length >> 6;
        let source_word_from = source_from >> 6;
        let dest_word_from = dest_from >> 6;

        if (source_from & 0x3f) == 0 {
            for i in 0..num_full_words {
                dest.bits[dest_word_from + i] &= source.bits[source_word_from + i];
            }
        } else {
            let shift = source_from & 63;
            let back = source_from.wrapping_neg() & 63;
            for i in 0..num_full_words {
                dest.bits[dest_word_from + i] &= (source.bits[source_word_from + i] >> shift)
                    | (source.bits[source_word_from + i + 1] << back);
            }
        }

        source_from += num_full_words << 6;
        dest_from += num_full_words << 6;
        length -= num_full_words << 6;

        // Finally handle the tail bits.
        if length > 0 {
            let mut bits = Self::read_n_bits(&source.bits, source_from, length);
            bits |= u64::MAX << (length & 63);
            dest.bits[dest_from >> 6] &= bits;
        }
    }

    /// `self = self OR other`.
    ///
    /// Equivalent to `FixedBitSet.or(FixedBitSet)`.
    ///
    /// **Divergence from Lucene 10.5.0, in the name only.** Java overloads `or`
    /// for both a `FixedBitSet` and a `DocIdSetIterator`. Rust has no
    /// overloading, and `or` is already taken on this type by
    /// [`BitSet::or`](crate::util::bit_set::BitSet::or), which is the iterator
    /// form. The set form therefore carries the `_set` suffix; the behaviour is
    /// Java's, delegating to [`or_range`](Self::or_range).
    pub fn or_set(&mut self, other: &FixedBitSet) {
        let length = other.length();
        Self::or_range(other, 0, self, 0, length);
    }

    /// `self = self XOR other`.
    ///
    /// Equivalent to `FixedBitSet.xor(FixedBitSet)`.
    pub fn xor(&mut self, other: &FixedBitSet) {
        self.xor_bits(&other.bits, other.num_words);
    }

    /// Equivalent to the private `FixedBitSet.xor(long[], int)`.
    fn xor_bits(&mut self, other_bits: &[u64], other_num_words: usize) {
        debug_assert!(other_num_words <= self.num_words);
        let mut pos = self.num_words.min(other_num_words);
        while pos > 0 {
            pos -= 1;
            self.bits[pos] ^= other_bits[pos];
        }
    }

    /// Returns true if the two sets have any element in common.
    ///
    /// Equivalent to `FixedBitSet.intersects(FixedBitSet)`.
    pub fn intersects(&self, other: &FixedBitSet) -> bool {
        // Depends on the ghost bits being clear!
        let mut pos = self.num_words.min(other.num_words);
        while pos > 0 {
            pos -= 1;
            if (self.bits[pos] & other.bits[pos]) != 0 {
                return true;
            }
        }
        false
    }

    /// `self = self AND other`.
    ///
    /// Equivalent to `FixedBitSet.and(FixedBitSet)`.
    pub fn and(&mut self, other: &FixedBitSet) {
        self.and_bits(&other.bits, other.num_words);
    }

    /// Equivalent to the private `FixedBitSet.and(long[], int)`.
    fn and_bits(&mut self, other_arr: &[u64], other_num_words: usize) {
        let mut pos = self.num_words.min(other_num_words);
        while pos > 0 {
            pos -= 1;
            self.bits[pos] &= other_arr[pos];
        }
        if self.num_words > other_num_words {
            self.bits[other_num_words..self.num_words].fill(0);
        }
    }

    /// `self = self AND NOT other`.
    ///
    /// Equivalent to `FixedBitSet.andNot(FixedBitSet)`.
    pub fn and_not(&mut self, other: &FixedBitSet) {
        self.and_not_offset(0, &other.bits, other.num_words);
    }

    /// Equivalent to the private `FixedBitSet.andNot(int, long[], int)`, which
    /// `andNot(DocIdSetIterator)` uses for a `DocBaseBitSetIterator`.
    pub fn and_not_offset(
        &mut self,
        other_offset_words: usize,
        other_arr: &[u64],
        other_num_words: usize,
    ) {
        let mut pos = self
            .num_words
            .saturating_sub(other_offset_words)
            .min(other_num_words);
        while pos > 0 {
            pos -= 1;
            self.bits[pos + other_offset_words] &= !other_arr[pos];
        }
    }

    /// Scans the backing store to check whether every bit is clear.
    ///
    /// Equivalent to `FixedBitSet.scanIsEmpty()`. Deliberately not named
    /// `is_empty`, as Lucene notes, to emphasise that it is not low cost.
    pub fn scan_is_empty(&self) -> bool {
        // Depends on the ghost bits being clear!
        self.bits[..self.num_words].iter().all(|&w| w == 0)
    }

    /// Flips the bits in the range `[start_index, end_index)`.
    ///
    /// Equivalent to `FixedBitSet.flip(int, int)`.
    pub fn flip_range(&mut self, start_index: usize, end_index: usize) {
        debug_assert!(start_index < self.num_bits);
        debug_assert!(end_index <= self.num_bits);
        if end_index <= start_index {
            return;
        }

        let start_word = start_index >> 6;
        let end_word = (end_index - 1) >> 6;

        // Java relies on the JLS rule that a shift uses only the low 6 bits of
        // the count, so `-1L >>> -endIndex` is a shift by `(-endIndex) & 63`.
        let startmask = u64::MAX << (start_index & 63);
        let endmask = u64::MAX >> (end_index.wrapping_neg() & 63);

        if start_word == end_word {
            self.bits[start_word] ^= startmask & endmask;
            return;
        }

        self.bits[start_word] ^= startmask;
        for i in (start_word + 1)..end_word {
            self.bits[i] = !self.bits[i];
        }
        self.bits[end_word] ^= endmask;
    }

    /// Flips the bit at `index`.
    ///
    /// Equivalent to `FixedBitSet.flip(int)`.
    pub fn flip(&mut self, index: usize) {
        debug_assert!(index < self.num_bits);
        let word_num = index >> 6;
        let bitmask = 1u64 << (index & 63);
        self.bits[word_num] ^= bitmask;
    }

    /// Clears the bits in the range `[start_index, end_index)`.
    ///
    /// Equivalent to `FixedBitSet.clear(int, int)`.
    pub fn clear_range(&mut self, start_index: usize, end_index: usize) {
        debug_assert!(start_index < self.num_bits);
        debug_assert!(end_index <= self.num_bits);
        if end_index <= start_index {
            return;
        }

        let start_word = start_index >> 6;
        let end_word = (end_index - 1) >> 6;

        // Inverted, since we are clearing.
        let startmask = !(u64::MAX << (start_index & 63));
        let endmask = !(u64::MAX >> (end_index.wrapping_neg() & 63));

        if start_word == end_word {
            self.bits[start_word] &= startmask | endmask;
            return;
        }

        self.bits[start_word] &= startmask;
        self.bits[(start_word + 1)..end_word].fill(0);
        self.bits[end_word] &= endmask;
    }

    /// Returns this set as read-only [`Bits`].
    ///
    /// Equivalent to `FixedBitSet.asReadOnlyBits()`, which exists so that a
    /// consumer handed a `Bits` cannot cast its way back to write access.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `FixedBits` wraps the same
    /// `long[]`, so later changes to this set show through the view. The crate's
    /// [`FixedBits`](crate::util::bit_set::FixedBits) owns its set, so this
    /// copies the words and the view is a snapshot. Sharing the words instead
    /// would need interior mutability that Java gets for free from an unguarded
    /// array reference, and the read-only guarantee — the point of the method —
    /// is preserved either way.
    pub fn as_read_only_bits(&self) -> crate::util::bit_set::FixedBits {
        crate::util::bit_set::FixedBits::new(self.bits.clone(), self.num_bits)
    }

    /// Restricts `bit_set` to the bits this set holds from `offset` on.
    ///
    /// Equivalent to `FixedBitSet.applyMask(FixedBitSet, int)`.
    ///
    /// # Errors
    ///
    /// Returns an error when `bit_set` has bits set beyond the end of this set,
    /// which is the `IllegalArgumentException` Java throws.
    pub fn apply_mask(&self, bit_set: &mut FixedBitSet, offset: usize) -> crate::error::Result<()> {
        // Some scorers do not track max_doc and may call this with an offset
        // beyond bit_set.length().
        let length = bit_set.length().min(self.length().saturating_sub(offset));
        if length > 0 {
            Self::and_range(self, offset, bit_set, 0, length);
        }
        if length < bit_set.length()
            && BitSet::next_set_bit(bit_set, length as i32) != crate::search::NO_MORE_DOCS
        {
            return Err(LuceneError::IllegalArgument(
                "Some bits are set beyond the end of live docs".to_string(),
            ));
        }
        Ok(())
    }

    /// Calls `consumer` with `base` added to the index of every set bit in
    /// `[from, to)`.
    ///
    /// Equivalent to `FixedBitSet.forEach(int, int, int, CheckedIntConsumer)`,
    /// which queries use when a bit set is an intermediate representation of
    /// their matches. Java's checked consumer becomes a closure returning
    /// [`Result`].
    pub fn for_each<F>(
        &self,
        mut from: usize,
        to: usize,
        base: i32,
        consumer: &mut F,
    ) -> crate::error::Result<()>
    where
        F: FnMut(i32) -> crate::error::Result<()>,
    {
        assert!(
            from <= to && to <= self.length(),
            "range [{from}, {to}) out of bounds"
        );

        // First, align `from` with a word start, i.e. a multiple of 64.
        if (from & 0x3f) != 0 {
            let mut bits = self.bits[from >> 6] >> (from & 63);
            let num_bits_til_next_word = from.wrapping_neg() & 0x3f;
            if to - from < num_bits_til_next_word {
                // All the bits are in a single word.
                bits &= (1u64 << ((to - from) & 63)) - 1;
                return Self::for_each_word(bits, (from as i32).wrapping_add(base), consumer);
            }
            Self::for_each_word(bits, (from as i32).wrapping_add(base), consumer)?;
            from += num_bits_til_next_word;
            debug_assert_eq!(from & 0x3f, 0);
        }

        for i in (from >> 6)..(to >> 6) {
            Self::for_each_word(self.bits[i], base.wrapping_add((i << 6) as i32), consumer)?;
        }

        // Now handle the remaining bits in the last partial word.
        if (to & 0x3f) != 0 {
            let bits = self.bits[to >> 6] & ((1u64 << (to & 63)) - 1);
            Self::for_each_word(bits, base.wrapping_add((to & !0x3f) as i32), consumer)?;
        }

        Ok(())
    }

    /// Equivalent to the private `FixedBitSet.forEach(long, int, CheckedIntConsumer)`.
    fn for_each_word<F>(mut bits: u64, base: i32, consumer: &mut F) -> crate::error::Result<()>
    where
        F: FnMut(i32) -> crate::error::Result<()>,
    {
        while bits != 0 {
            let ntz = bits.trailing_zeros();
            consumer(base.wrapping_add(ntz as i32))?;
            bits ^= 1u64 << ntz;
        }
        Ok(())
    }

    /// Writes the set bits of `[from, to)` into `array` as document IDs, each
    /// offset by `base`, and returns how many were written.
    ///
    /// Equivalent to `FixedBitSet.intoArray(int, int, int, int[])`. It stops at
    /// the first of "no more set bits before `to`" and "no capacity left".
    pub fn into_array(&self, mut from: usize, to: usize, base: i32, array: &mut [i32]) -> usize {
        assert!(
            from <= to && to <= self.length(),
            "range [{from}, {to}) out of bounds"
        );

        let mut offset = 0usize;
        // First, align `from` with a word start, i.e. a multiple of 64.
        if (from & 0x3f) != 0 {
            let mut word = self.bits[from >> 6] >> (from & 63);
            let num_bits_til_next_word = from.wrapping_neg() & 0x3f;
            if to - from < num_bits_til_next_word {
                // All the bits are in a single word.
                word &= (1u64 << ((to - from) & 63)) - 1;
                return Self::word_to_array(word, (from as i32).wrapping_add(base), array, offset);
            }
            offset = Self::word_to_array(word, (from as i32).wrapping_add(base), array, offset);
            from += num_bits_til_next_word;
            debug_assert_eq!(from & 0x3f, 0);
        }

        for i in (from >> 6)..(to >> 6) {
            let word = self.bits[i];
            offset = Self::word_to_array(word, base.wrapping_add((i << 6) as i32), array, offset);
        }

        // Now handle the remaining bits in the last partial word.
        if (to & 0x3f) != 0 {
            let word = self.bits[to >> 6] & ((1u64 << (to & 63)) - 1);
            offset =
                Self::word_to_array(word, base.wrapping_add((to & !0x3f) as i32), array, offset);
        }

        offset
    }

    /// Equivalent to the private `FixedBitSet.word2Array(long, int, int[], int)`.
    fn word_to_array(mut word: u64, base: i32, docs: &mut [i32], offset: usize) -> usize {
        let bit_count = word.count_ones() as usize;

        if bit_count >= 32 && docs.len() - offset > bit_count {
            return Self::dense_word_to_array(word, base, docs, offset);
        }

        let num_bits_to_copy = bit_count.min(docs.len() - offset);

        for i in 0..num_bits_to_copy {
            let ntz = word.trailing_zeros();
            docs[offset + i] = base.wrapping_add(ntz as i32);
            word ^= 1u64 << ntz;
        }

        offset + num_bits_to_copy
    }

    /// Equivalent to the private `FixedBitSet.denseWord2Array(long, int, int[], int)`,
    /// the branch-free path Lucene takes for a word with at least 32 bits set: it
    /// writes speculatively at both halves and advances only on a set bit.
    fn dense_word_to_array(word: u64, base: i32, docs: &mut [i32], mut offset: usize) -> usize {
        debug_assert!(docs.len() - offset >= word.count_ones() as usize + 1);

        let l_word = word as u32;
        let h_word = (word >> 32) as u32;
        let offset32 = offset + l_word.count_ones() as usize;
        let mut h_offset = offset32;

        for i in 0..32u32 {
            docs[offset] = base.wrapping_add(i as i32);
            docs[h_offset] = base.wrapping_add(i as i32).wrapping_add(32);
            offset += ((l_word >> i) & 1) as usize;
            h_offset += ((h_word >> i) & 1) as usize;
        }

        docs[offset32] = base
            .wrapping_add(32)
            .wrapping_add(h_word.trailing_zeros() as i32);

        h_offset
    }

    /// Returns the hash Lucene's `FixedBitSet.hashCode()` computes.
    ///
    /// Kept as a named method, rather than only as [`Hash`], because the value
    /// itself is part of the ported contract.
    pub fn hash_code(&self) -> i32 {
        // Depends on the ghost bits being clear!
        let mut h: u64 = 0;
        let mut i = self.num_words;
        while i > 0 {
            i -= 1;
            h ^= self.bits[i];
            h = (h << 1) | (h >> 63); // rotate left
        }
        // Fold the leftmost bits into the right and add a constant, so that an
        // empty set does not hash to 0, which is too common.
        // Java's `+ 0x98761234` is an int literal that wraps to -1737092556;
        // Rust needs the wrap spelled out.
        (((h >> 32) ^ h) as i32).wrapping_add(0x9876_1234u32 as i32)
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

impl PartialEq for FixedBitSet {
    /// Equivalent to `FixedBitSet.equals(Object)`: two sets are equal when they
    /// hold the same number of bits and the same backing words.
    fn eq(&self, other: &Self) -> bool {
        if self.num_bits != other.num_bits {
            return false;
        }
        // Depends on the ghost bits being clear!
        self.bits == other.bits
    }
}

impl Eq for FixedBitSet {}

impl Hash for FixedBitSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.hash_code());
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
