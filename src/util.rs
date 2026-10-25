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
pub mod java_collections;
pub(crate) mod java_fmt;
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
pub use selector::intro_select;
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
    path::{Path, PathBuf},
    sync::LazyLock,
};

use crate::error::LuceneError;
use crate::store::{DataInput, DataOutput};

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

impl BytesRef {
    /// Java's `BytesRef.hashCode()`, reproduced exactly so that values hash the
    /// same way on both sides of the port.
    ///
    /// The value is `StringHelper.murmurhash3_x86_32(this,
    /// StringHelper.GOOD_FAST_HASH_SEED)`. The seed is drawn once per process
    /// and is only stable across runs when `tests.seed` is set, so this number
    /// is reproducible only under a fixed seed — exactly as in Java. See
    /// [`StringHelper::good_fast_hash_seed`](crate::util::string_helper::StringHelper::good_fast_hash_seed).
    pub fn hash_code(&self) -> i32 {
        crate::util::string_helper::StringHelper::murmurhash3_x86_32_bytes_ref(
            self,
            crate::util::string_helper::StringHelper::good_fast_hash_seed(),
        )
    }
}

impl Hash for BytesRef {
    /// Hashes the active slice, which is what [`PartialEq`] compares, so the
    /// two agree: equal `BytesRef`s hash equally.
    ///
    /// Java's `hashCode()` is the seeded MurmurHash3 value that
    /// [`BytesRef::hash_code`] reproduces; a Rust `Hasher` consumes bytes
    /// rather than producing an `i32`, so this feeds it the same bytes the
    /// Java hash is computed over instead of forcing that particular mixing
    /// function on every caller's hasher.
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash_slice(self.slice(), state);
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
///
/// # `equals` and `hashCode`
///
/// Lucene's `BytesRefBuilder.equals(Object)` and `BytesRefBuilder.hashCode()` both
/// `throw new UnsupportedOperationException()`: a mutable builder must never be
/// used as a map key or compared by value, and Lucene enforces that at runtime.
/// The faithful port of a method that only throws is to leave the trait
/// unimplemented, so this type deliberately implements neither [`PartialEq`]
/// nor [`std::hash::Hash`], which turns Java's runtime failure into a compile
/// error. Compare the built value instead, via `get()`.
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

    /// Writes `result = a - b` for the `dim`-th dimension, where `a >= b`.
    ///
    /// Equivalent to `NumericUtils.subtract(int, int, byte[], byte[], byte[])`:
    /// the tail that is not a multiple of four is borrowed byte by byte, then
    /// the aligned head four bytes at a time.
    ///
    /// # Panics
    ///
    /// Panics when `a < b`, standing in for Java's `IllegalArgumentException`.
    pub fn subtract(bytes_per_dim: usize, dim: usize, a: &[u8], b: &[u8], result: &mut [u8]) {
        let start = dim * bytes_per_dim;
        let end = start + bytes_per_dim;

        let mut borrow: i64 = 0;
        let limit = start + (bytes_per_dim & !3);

        let mut i = end;
        while i > limit {
            i -= 1;
            let diff = i32::from(a[i]) - i32::from(b[i]) - borrow as i32;
            borrow = i64::from(diff < 0);
            result[i - start] = diff as u8;
        }

        // Java steps `i -= 3` from the byte-wise loop's exit value, which is one
        // before `limit`, so the first aligned word starts at `limit - 4`.
        let mut j = i;
        while j >= start + 4 {
            j -= 4;
            let a_int = u32::from_be_bytes(a[j..j + 4].try_into().unwrap());
            let b_int = u32::from_be_bytes(b[j..j + 4].try_into().unwrap());
            let diff = i64::from(a_int) - i64::from(b_int) - borrow;
            borrow = i64::from(diff < 0);
            result[j - start..j - start + 4].copy_from_slice(&(diff as u32).to_be_bytes());
        }

        assert!(borrow == 0, "a < b");
    }

    /// Writes `result = a + b` for the `dim`-th dimension, treating both as
    /// unsigned.
    ///
    /// Equivalent to `NumericUtils.add(int, int, byte[], byte[], byte[])`.
    ///
    /// # Panics
    ///
    /// Panics on overflow, standing in for Java's `IllegalArgumentException`.
    pub fn add(bytes_per_dim: usize, dim: usize, a: &[u8], b: &[u8], result: &mut [u8]) {
        let start = dim * bytes_per_dim;
        let end = start + bytes_per_dim;

        let mut carry: i64 = 0;
        let limit = start + (bytes_per_dim & !3);

        let mut i = end;
        while i > limit {
            i -= 1;
            let digit_sum = i32::from(a[i]) + i32::from(b[i]) + carry as i32;
            carry = i64::from(digit_sum >= 256);
            result[i - start] = digit_sum as u8;
        }

        let mut j = i;
        while j >= start + 4 {
            j -= 4;
            let a_int = u32::from_be_bytes(a[j..j + 4].try_into().unwrap());
            let b_int = u32::from_be_bytes(b[j..j + 4].try_into().unwrap());
            let digit_sum = i64::from(a_int) + i64::from(b_int) + carry;
            carry = i64::from(digit_sum >= 0x1_0000_0000);
            result[j - start..j - start + 4].copy_from_slice(&(digit_sum as u32).to_be_bytes());
        }

        assert!(carry == 0, "a + b overflows bytesPerDim={bytes_per_dim}");
    }

    /// Encodes an arbitrary-precision integer so that unsigned byte order
    /// matches its numeric order, sign-extending it to `big_int_size` bytes.
    ///
    /// Equivalent to `NumericUtils.bigIntToSortableBytes(BigInteger, int,
    /// byte[], int)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java takes a `java.math.BigInteger`.
    /// The crate has no arbitrary-precision integer type and adopting one is a
    /// dependency decision reserved to the user (`CLAUDE.md` §13.3), so the
    /// value is passed in the form `BigInteger.toByteArray()` returns and
    /// `new BigInteger(byte[])` consumes: its minimal two's-complement
    /// big-endian representation, most significant byte first, never empty.
    /// Every caller of the Java method already obtains those bytes on the way
    /// in or out, so the encoding this produces is unchanged.
    ///
    /// # Panics
    ///
    /// Panics when the value needs more than `big_int_size` bytes, standing in
    /// for Java's `IllegalArgumentException`.
    pub fn big_int_to_sortable_bytes(
        big_int: &[u8],
        big_int_size: usize,
        result: &mut [u8],
        offset: usize,
    ) {
        assert!(!big_int.is_empty(), "big_int must not be empty");
        let mut full_big_int_bytes = vec![0u8; big_int_size];
        match big_int.len().cmp(&big_int_size) {
            Ordering::Less => {
                let pad = big_int_size - big_int.len();
                full_big_int_bytes[pad..].copy_from_slice(big_int);
                if (big_int[0] & 0x80) != 0 {
                    // sign extend
                    full_big_int_bytes[..pad].fill(0xff);
                }
            }
            Ordering::Equal => full_big_int_bytes.copy_from_slice(big_int),
            Ordering::Greater => {
                panic!("BigInteger requires more than {big_int_size} bytes storage")
            }
        }
        // Flip the sign bit so negative bigints sort before positive bigints:
        full_big_int_bytes[0] ^= 0x80;

        result[offset..offset + big_int_size].copy_from_slice(&full_big_int_bytes);
    }

    /// Decodes a value previously written with
    /// [`big_int_to_sortable_bytes`](Self::big_int_to_sortable_bytes).
    ///
    /// Equivalent to `NumericUtils.sortableBytesToBigInt(byte[], int, int)`. It
    /// returns the two's-complement big-endian bytes `new BigInteger(byte[])`
    /// would be built from — see the note on the encoder.
    pub fn sortable_bytes_to_big_int(encoded: &[u8], offset: usize, length: usize) -> Vec<u8> {
        let mut big_int_bytes = ArrayUtil::copy_of_sub_array(encoded, offset, offset + length);
        // Flip the sign bit back to the original
        big_int_bytes[0] ^= 0x80;
        big_int_bytes
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

    /// The byte order the crate reads and writes native-order values in.
    ///
    /// Equivalent to `BitUtil.NATIVE_BYTE_ORDER`: the platform's own order,
    /// except that a test run with a seed picks an order from the seed instead,
    /// so that a randomised run exercises both. Native-order values never reach
    /// the index files, so flipping this cannot change what is written.
    pub fn native_byte_order() -> ByteOrder {
        Self::get_native_byte_order()
    }

    /// Equivalent to the private `BitUtil.getNativeByteOrder()`.
    fn get_native_byte_order() -> ByteOrder {
        if let Ok(prop) = env::var("tests.seed") {
            return if Self::java_string_hash_code(&prop) % 2 == 0 {
                ByteOrder::LittleEndian
            } else {
                ByteOrder::BigEndian
            };
        }
        if cfg!(target_endian = "little") {
            ByteOrder::LittleEndian
        } else {
            ByteOrder::BigEndian
        }
    }

    /// `java.lang.String.hashCode()`, which the seed-derived order depends on.
    fn java_string_hash_code(s: &str) -> i32 {
        let mut h: i32 = 0;
        for c in s.encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(i32::from(c));
        }
        h
    }

    /// Interleaves the low 32 bits of each value, the even bits taken from
    /// `even` and the odd bits from `odd`.
    ///
    /// Equivalent to `BitUtil.interleave(int, int)`. Adapted from
    /// <http://graphics.stanford.edu/~seander/bithacks.html#InterleaveBMN>.
    pub fn interleave(even: i32, odd: i32) -> i64 {
        let mut v1 = 0x0000_0000_FFFF_FFFFu64 & (even as u32 as u64);
        let mut v2 = 0x0000_0000_FFFF_FFFFu64 & (odd as u32 as u64);
        v1 = (v1 | (v1 << SHIFT4)) & MAGIC4;
        v1 = (v1 | (v1 << SHIFT3)) & MAGIC3;
        v1 = (v1 | (v1 << SHIFT2)) & MAGIC2;
        v1 = (v1 | (v1 << SHIFT1)) & MAGIC1;
        v1 = (v1 | (v1 << SHIFT0)) & MAGIC0;
        v2 = (v2 | (v2 << SHIFT4)) & MAGIC4;
        v2 = (v2 | (v2 << SHIFT3)) & MAGIC3;
        v2 = (v2 | (v2 << SHIFT2)) & MAGIC2;
        v2 = (v2 | (v2 << SHIFT1)) & MAGIC1;
        v2 = (v2 | (v2 << SHIFT0)) & MAGIC0;
        ((v2 << 1) | v1) as i64
    }

    /// Extracts the even-bit half of an interleaved value.
    ///
    /// Equivalent to `BitUtil.deinterleave(long)`.
    pub fn deinterleave(b: i64) -> i64 {
        let mut b = b as u64;
        b &= MAGIC0;
        b = (b ^ (b >> SHIFT0)) & MAGIC1;
        b = (b ^ (b >> SHIFT1)) & MAGIC2;
        b = (b ^ (b >> SHIFT2)) & MAGIC3;
        b = (b ^ (b >> SHIFT3)) & MAGIC4;
        b = (b ^ (b >> SHIFT4)) & MAGIC5;
        b as i64
    }

    /// Flips odd bits with even bits.
    ///
    /// Equivalent to `BitUtil.flipFlop(long)`.
    pub fn flip_flop(b: i64) -> i64 {
        let b = b as u64;
        (((b & MAGIC6) >> 1) | ((b & MAGIC0) << 1)) as i64
    }
}

/// The byte order of a multi-byte value.
///
/// Equivalent to `java.nio.ByteOrder`, which Lucene's `BitUtil` returns from
/// `NATIVE_BYTE_ORDER`. Only the two orders Java defines exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ByteOrder {
    /// Least significant byte first.
    LittleEndian,
    /// Most significant byte first.
    BigEndian,
}

// magic numbers for bit interleaving
const MAGIC0: u64 = 0x5555_5555_5555_5555;
const MAGIC1: u64 = 0x3333_3333_3333_3333;
const MAGIC2: u64 = 0x0F0F_0F0F_0F0F_0F0F;
const MAGIC3: u64 = 0x00FF_00FF_00FF_00FF;
const MAGIC4: u64 = 0x0000_FFFF_0000_FFFF;
const MAGIC5: u64 = 0x0000_0000_FFFF_FFFF;
const MAGIC6: u64 = 0xAAAA_AAAA_AAAA_AAAA;

// shift values for bit interleaving
const SHIFT0: u32 = 1;
const SHIFT1: u32 = 2;
const SHIFT2: u32 = 4;
const SHIFT3: u32 = 8;
const SHIFT4: u32 = 16;

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

    /// Reads group varints from `src` into `dst[0..limit]` through the scalar
    /// path, bypassing any specialised decoder.
    ///
    /// Equivalent to `GroupVIntUtil.readGroupVInts$Baseline(DataInput, int[],
    /// int)`, which exists so a benchmark can measure the generic decoder
    /// against a specialised one. This port has only the generic decoder, so
    /// the baseline is the same code, kept under Lucene's name so the
    /// comparison it exists for stays expressible.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::Io` if the input is truncated or otherwise invalid.
    ///
    /// Equivalent to `org.apache.lucene.util.GroupVIntUtil#readGroupVInts$Baseline`.
    pub fn read_group_vints_baseline(
        src: &[u8],
        pos: &mut usize,
        dst: &mut [i32],
        limit: usize,
    ) -> Result<(), LuceneError> {
        let limit = limit.min(dst.len());
        let mut i = 0;
        while i + 4 <= limit {
            Self::read_group_vint_baseline(src, pos, dst, i)?;
            i += 4;
        }
        while i < limit {
            dst[i] = Self::read_vint(src, pos)?;
            i += 1;
        }
        Ok(())
    }

    /// Equivalent to the private `GroupVIntUtil.readGroupVInt$Baseline(DataInput,
    /// int[], int)`.
    ///
    /// Equivalent to `org.apache.lucene.util.GroupVIntUtil#readGroupVInt$Baseline`.
    fn read_group_vint_baseline(
        src: &[u8],
        pos: &mut usize,
        dst: &mut [i32],
        offset: usize,
    ) -> Result<(), LuceneError> {
        Self::read_group_vint(src, pos, dst, offset)
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

    /// Reads all the group varints of `dst[0..limit]`, including the tail
    /// vints, from a [`DataInput`].
    ///
    /// Equivalent to `GroupVIntUtil.readGroupVInts(DataInput, int[], int)`.
    ///
    /// # Errors
    ///
    /// Returns any error raised by the underlying input.
    pub fn read_group_vints_input<I: DataInput + ?Sized>(
        input: &mut I,
        dst: &mut [i32],
        limit: usize,
    ) -> Result<(), LuceneError> {
        let mut i = 0;
        while i + 4 <= limit {
            Self::read_group_vint_input(input, dst, i)?;
            i += 4;
        }
        while i < limit {
            dst[i] = input.read_v_int()?;
            i += 1;
        }
        Ok(())
    }

    /// Reads a single group of four varints from a [`DataInput`].
    ///
    /// Equivalent to `GroupVIntUtil.readGroupVInt(DataInput, int[], int)`.
    ///
    /// Divergence: Lucene first tests, with `instanceof`, whether the input is
    /// both a `RandomAccessInput` and an `IndexInput` and, when it is and four
    /// whole ints remain, decodes the group branch-lessly through absolute
    /// reads followed by a single `seek`. Rust has no `instanceof` over a
    /// `&mut dyn DataInput`, so this port always takes Lucene's own
    /// fall-through path. That path decodes the identical bytes to the
    /// identical values — the branch-less form is a speed optimisation, not a
    /// different encoding — so only the speed of this call differs, never its
    /// result. See `GroupVIntUtil.java` lines 65-96 in Apache Lucene Core
    /// 10.5.0.
    ///
    /// # Errors
    ///
    /// Returns any error raised by the underlying input.
    pub fn read_group_vint_input<I: DataInput + ?Sized>(
        input: &mut I,
        dst: &mut [i32],
        offset: usize,
    ) -> Result<(), LuceneError> {
        let flag = input.read_byte()? as usize;

        let n1_minus1 = flag >> 6;
        let n2_minus1 = (flag >> 4) & 0x03;
        let n3_minus1 = (flag >> 2) & 0x03;
        let n4_minus1 = flag & 0x03;

        dst[offset] = Self::read_int_in_group_input(input, n1_minus1)?;
        dst[offset + 1] = Self::read_int_in_group_input(input, n2_minus1)?;
        dst[offset + 2] = Self::read_int_in_group_input(input, n3_minus1)?;
        dst[offset + 3] = Self::read_int_in_group_input(input, n4_minus1)?;
        Ok(())
    }

    /// Reads all the group varints of `dst[0..limit]` into an `i64` array.
    ///
    /// Equivalent to `GroupVIntUtil.readGroupVInts(DataInput, long[], int)`,
    /// which Lucene keeps only for the backwards codecs.
    ///
    /// # Errors
    ///
    /// Returns any error raised by the underlying input.
    pub fn read_group_vints_input_long<I: DataInput + ?Sized>(
        input: &mut I,
        dst: &mut [i64],
        limit: usize,
    ) -> Result<(), LuceneError> {
        let mut i = 0;
        while i + 4 <= limit {
            Self::read_group_vint_input_long(input, dst, i)?;
            i += 4;
        }
        while i < limit {
            dst[i] = input.read_v_int()? as i64 & 0xFFFF_FFFF;
            i += 1;
        }
        Ok(())
    }

    /// Reads a single group of four varints into an `i64` array.
    ///
    /// Equivalent to `GroupVIntUtil.readGroupVInt(DataInput, long[], int)`.
    ///
    /// # Errors
    ///
    /// Returns any error raised by the underlying input.
    pub fn read_group_vint_input_long<I: DataInput + ?Sized>(
        input: &mut I,
        dst: &mut [i64],
        offset: usize,
    ) -> Result<(), LuceneError> {
        let flag = input.read_byte()? as usize;

        let n1_minus1 = flag >> 6;
        let n2_minus1 = (flag >> 4) & 0x03;
        let n3_minus1 = (flag >> 2) & 0x03;
        let n4_minus1 = flag & 0x03;

        dst[offset] = Self::read_int_in_group_input(input, n1_minus1)? as i64 & 0xFFFF_FFFF;
        dst[offset + 1] = Self::read_int_in_group_input(input, n2_minus1)? as i64 & 0xFFFF_FFFF;
        dst[offset + 2] = Self::read_int_in_group_input(input, n3_minus1)? as i64 & 0xFFFF_FFFF;
        dst[offset + 3] = Self::read_int_in_group_input(input, n4_minus1)? as i64 & 0xFFFF_FFFF;
        Ok(())
    }

    /// Equivalent to the private `GroupVIntUtil.readIntInGroup(DataInput, int)`.
    fn read_int_in_group_input<I: DataInput + ?Sized>(
        input: &mut I,
        num_bytes_minus_one: usize,
    ) -> Result<i32, LuceneError> {
        match num_bytes_minus_one {
            0 => Ok(input.read_byte()? as i32),
            1 => Ok(input.read_short()? as i32 & 0xFFFF),
            2 => {
                let low = input.read_short()? as i32 & 0xFFFF;
                let high = (input.read_byte()? as i32) << 16;
                Ok(low | high)
            }
            _ => input.read_int(),
        }
    }

    /// Encodes `values[0..limit]` with group-varint onto a [`DataOutput`],
    /// using `scratch` — at most [`MAX_LENGTH_PER_GROUP`] bytes — as the
    /// per-group staging buffer.
    ///
    /// Equivalent to `GroupVIntUtil.writeGroupVInts(DataOutput, byte[], int[],
    /// int)`.
    ///
    /// # Errors
    ///
    /// Returns any error raised by the underlying output.
    pub fn write_group_vints_output<O: DataOutput + ?Sized>(
        out: &mut O,
        scratch: &mut [u8],
        values: &[i32],
        limit: usize,
    ) -> Result<(), LuceneError> {
        let mut read_pos = 0;

        // encode each group
        while limit - read_pos >= 4 {
            let mut write_pos = 0;
            let n1_minus1 = Self::num_bytes(values[read_pos]) - 1;
            let n2_minus1 = Self::num_bytes(values[read_pos + 1]) - 1;
            let n3_minus1 = Self::num_bytes(values[read_pos + 2]) - 1;
            let n4_minus1 = Self::num_bytes(values[read_pos + 3]) - 1;
            let flag = (n1_minus1 << 6) | (n2_minus1 << 4) | (n3_minus1 << 2) | n4_minus1;
            scratch[write_pos] = flag as u8;
            write_pos += 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n1_minus1 + 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n2_minus1 + 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n3_minus1 + 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos]);
            read_pos += 1;
            write_pos += n4_minus1 + 1;

            out.write_bytes_full(scratch, write_pos)?;
        }

        // tail vints
        while read_pos < limit {
            out.write_v_int(values[read_pos])?;
            read_pos += 1;
        }
        Ok(())
    }

    /// Encodes `values[0..limit]` with group-varint onto a [`DataOutput`], from
    /// an `i64` array whose entries are all required to be integers.
    ///
    /// Equivalent to `GroupVIntUtil.writeGroupVInts(DataOutput, byte[], long[],
    /// int)`, which Lucene keeps only for the backwards codecs.
    ///
    /// # Errors
    ///
    /// Returns any error raised by the underlying output, and
    /// `LuceneError::IllegalArgument("integer overflow")` when a value does not
    /// fit in an unsigned 32-bit integer. Lucene throws `ArithmeticException`
    /// with that message; this port has no arithmetic-error variant, so the
    /// nearest one carries it.
    pub fn write_group_vints_output_long<O: DataOutput + ?Sized>(
        out: &mut O,
        scratch: &mut [u8],
        values: &[i64],
        limit: usize,
    ) -> Result<(), LuceneError> {
        let mut read_pos = 0;

        // encode each group
        while limit - read_pos >= 4 {
            let mut write_pos = 0;
            let n1_minus1 = Self::num_bytes(Self::to_int(values[read_pos])?) - 1;
            let n2_minus1 = Self::num_bytes(Self::to_int(values[read_pos + 1])?) - 1;
            let n3_minus1 = Self::num_bytes(Self::to_int(values[read_pos + 2])?) - 1;
            let n4_minus1 = Self::num_bytes(Self::to_int(values[read_pos + 3])?) - 1;
            let flag = (n1_minus1 << 6) | (n2_minus1 << 4) | (n3_minus1 << 2) | n4_minus1;
            scratch[write_pos] = flag as u8;
            write_pos += 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos] as i32);
            read_pos += 1;
            write_pos += n1_minus1 + 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos] as i32);
            read_pos += 1;
            write_pos += n2_minus1 + 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos] as i32);
            read_pos += 1;
            write_pos += n3_minus1 + 1;
            BitUtil::write_le_int(scratch, write_pos, values[read_pos] as i32);
            read_pos += 1;
            write_pos += n4_minus1 + 1;

            out.write_bytes_full(scratch, write_pos)?;
        }

        // tail vints
        while read_pos < limit {
            out.write_v_int(Self::to_int(values[read_pos])?)?;
            read_pos += 1;
        }
        Ok(())
    }

    /// Equivalent to the private `GroupVIntUtil.toInt(long)`: narrows a `long`
    /// that is required to hold an unsigned 32-bit value.
    fn to_int(value: i64) -> Result<i32, LuceneError> {
        if (value & 0xFFFF_FFFF_0000_0000u64 as i64) != 0 {
            return Err(LuceneError::IllegalArgument("integer overflow".to_string()));
        }
        Ok(value as i32)
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

    /// Sorts `a[from_index..to_index]` with the intro-sort algorithm, falling
    /// back to insertion sort for small slices.
    ///
    /// Equivalent to `ArrayUtil.introSort(T[], int, int, Comparator)`.
    pub fn intro_sort<T, C>(a: &mut [T], from_index: usize, to_index: usize, comp: C)
    where
        T: Clone,
        C: FnMut(&T, &T) -> Ordering,
    {
        if to_index - from_index <= 1 {
            return;
        }
        ArrayIntroSorter::new(a, comp).sort(from_index, to_index);
    }

    /// Sorts the whole slice with the intro-sort algorithm.
    ///
    /// Equivalent to `ArrayUtil.introSort(T[], Comparator)`.
    pub fn intro_sort_all<T, C>(a: &mut [T], comp: C)
    where
        T: Clone,
        C: FnMut(&T, &T) -> Ordering,
    {
        let len = a.len();
        Self::intro_sort(a, 0, len, comp);
    }

    /// Sorts `a[from_index..to_index]` in natural order with the intro-sort
    /// algorithm.
    ///
    /// Equivalent to `ArrayUtil.introSort(T[], int, int)`.
    pub fn intro_sort_natural<T: Ord + Clone>(a: &mut [T], from_index: usize, to_index: usize) {
        if to_index - from_index <= 1 {
            return;
        }
        Self::intro_sort(a, from_index, to_index, |x: &T, y: &T| x.cmp(y));
    }

    /// Sorts `a[from_index..to_index]` with TimSort, falling back to binary
    /// sort for small slices.
    ///
    /// Equivalent to `ArrayUtil.timSort(T[], int, int, Comparator)`, which sizes
    /// the temporary storage at an eighth of the slice.
    pub fn tim_sort<T, C>(a: &mut [T], from_index: usize, to_index: usize, comp: C)
    where
        T: Clone,
        C: FnMut(&T, &T) -> Ordering,
    {
        if to_index - from_index <= 1 {
            return;
        }
        let max_temp_slots = a.len() / 64;
        ArrayTimSorter::new(a, comp, max_temp_slots).sort(from_index, to_index);
    }

    /// Sorts the whole slice with TimSort.
    ///
    /// Equivalent to `ArrayUtil.timSort(T[], Comparator)`.
    pub fn tim_sort_all<T, C>(a: &mut [T], comp: C)
    where
        T: Clone,
        C: FnMut(&T, &T) -> Ordering,
    {
        let len = a.len();
        Self::tim_sort(a, 0, len, comp);
    }

    /// Sorts `a[from_index..to_index]` in natural order with TimSort.
    ///
    /// Equivalent to `ArrayUtil.timSort(T[], int, int)`.
    pub fn tim_sort_natural<T: Ord + Clone>(a: &mut [T], from_index: usize, to_index: usize) {
        if to_index - from_index <= 1 {
            return;
        }
        Self::tim_sort(a, from_index, to_index, |x: &T, y: &T| x.cmp(y));
    }

    /// Reorganises `arr[from..to]` so that the element at offset `k` sits where
    /// it would sit if the range were sorted, with every element to its left no
    /// greater and every element to its right no smaller.
    ///
    /// Equivalent to `ArrayUtil.select(T[], int, int, int, Comparator)`. Runs in
    /// linear time on average and `n log n` in the worst case.
    pub fn select<T, C>(arr: &mut [T], from: usize, to: usize, k: usize, comparator: C)
    where
        T: Clone,
        C: FnMut(&T, &T) -> Ordering,
    {
        // Java builds an anonymous IntroSelector whose swap/setPivot/comparePivot
        // read the array; ArrayIntroSorter offers exactly those three operations
        // over the same array, so the selector runs on it unchanged.
        let mut ops = ArrayIntroSorter::new(arr, comparator);
        intro_select(&mut ops, from, to, k);
    }

    /// Copies a slice into a new vector.
    ///
    /// Equivalent to the `ArrayUtil.copyArray` overloads. Java declares one per
    /// primitive plus a generic form; a single generic function serves them all
    /// here.
    pub fn copy_array<T: Clone>(array: &[T]) -> Vec<T> {
        array.to_vec()
    }
}

// ---------------------------------------------------------------------------
// IOUtils
// ---------------------------------------------------------------------------

/// Utilities for dealing with closeable resources and file-system operations.
pub struct IOUtils;

impl IOUtils {
    /// The canonical name of the UTF-8 charset.
    ///
    /// Equivalent to `IOUtils.UTF_8`.
    pub const UTF_8: &'static str = "UTF-8";

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

    /// Closes all given closeables, folding every failure into `ex`.
    ///
    /// Equivalent to `IOUtils.closeWhileSuppressingExceptions(Throwable,
    /// Iterable)`. Java suppresses each new throwable into the one already in
    /// flight and rethrows any `Error` in preference to it; Rust has neither
    /// suppressed-exception lists nor a separate `Error` hierarchy, so the
    /// failures are appended to the message of `ex`, which is the same
    /// information in the one place the language offers for it.
    pub fn close_while_suppressing_exceptions<I, F>(ex: LuceneError, objects: I) -> LuceneError
    where
        I: IntoIterator<Item = Option<F>>,
        F: FnOnce() -> io::Result<()>,
    {
        let mut suppressed: Vec<String> = Vec::new();
        for object in objects.into_iter().flatten() {
            if let Err(e) = object() {
                suppressed.push(e.to_string());
            }
        }
        if suppressed.is_empty() {
            return ex;
        }
        LuceneError::Other(format!("{ex} (suppressed: {})", suppressed.join("; ")))
    }

    /// Wraps `stream` in a reader that reports, rather than replaces, malformed
    /// input and unmappable characters.
    ///
    /// Equivalent to `IOUtils.getDecodingReader(InputStream, Charset)`. Java
    /// builds a `CharsetDecoder` set to `CodingErrorAction.REPORT`; this reads
    /// the whole stream and decodes it strictly, so an ill-formed sequence is an
    /// error instead of a replacement character. Only UTF-8 is supported, which
    /// is the only charset Lucene passes here.
    ///
    /// # Errors
    ///
    /// Returns an error when the stream cannot be read, or when its bytes are
    /// not valid UTF-8.
    pub fn get_decoding_reader<R: io::Read>(
        mut stream: R,
        char_set: &str,
    ) -> Result<io::Cursor<String>, LuceneError> {
        if !char_set.eq_ignore_ascii_case(Self::UTF_8) {
            return Err(LuceneError::UnsupportedOperation(format!(
                "unsupported charset: {char_set}"
            )));
        }
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| LuceneError::Other(format!("MalformedInputException: {e}")))?;
        Ok(io::Cursor::new(text))
    }

    /// Enforces the existence of a resource that a lookup reports as absent by
    /// returning nothing.
    ///
    /// Equivalent to `IOUtils.requireResourceNonNull(T, String)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::NotFound`] when `resource` is `None`, standing in
    /// for Java's `FileNotFoundException`.
    pub fn require_resource_non_null<T>(resource: Option<T>, name: &str) -> Result<T, LuceneError> {
        match resource {
            Some(resource) => Ok(resource),
            None => Err(LuceneError::NotFound(format!(
                "The resource '{name}' was not found."
            ))),
        }
    }

    /// Deletes one or more files or directories, and everything underneath
    /// them.
    ///
    /// Equivalent to `IOUtils.rm(Path...)`: every location is attempted, the
    /// ones that could not be removed are collected in the order they were
    /// attempted, and a single error naming all of them is raised at the end.
    ///
    /// # Errors
    ///
    /// Returns an error listing every path that could not be removed.
    pub fn rm<I, P>(locations: I) -> Result<(), LuceneError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut unremoved: Vec<(PathBuf, io::Error)> = Vec::new();
        Self::rm_into(&mut unremoved, locations);
        if unremoved.is_empty() {
            return Ok(());
        }
        let mut b =
            String::from("Could not remove the following files (in the order of attempts):\n");
        for (path, cause) in &unremoved {
            let absolute = path.canonicalize().unwrap_or_else(|_| path.clone());
            b.push_str(&format!("   {}: {cause}\n", absolute.display()));
        }
        Err(LuceneError::Io(io::Error::other(b)))
    }

    /// Equivalent to the private `IOUtils.rm(LinkedHashMap, Path...)`: walks
    /// each location depth-first, deleting files before the directory that
    /// holds them, and records — rather than propagates — every failure.
    fn rm_into<I, P>(unremoved: &mut Vec<(PathBuf, io::Error)>, locations: I)
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        for location in locations {
            let location = location.as_ref();
            // TODO: remove this leniency!
            if !location.exists() {
                continue;
            }
            Self::rm_walk(unremoved, location);
        }
    }

    /// The file-tree visitor Java hands to `Files.walkFileTree`: post-order, so
    /// a directory is removed only once its entries have been.
    fn rm_walk(unremoved: &mut Vec<(PathBuf, io::Error)>, path: &Path) {
        let is_dir = path.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
        if is_dir {
            match fs::read_dir(path) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => Self::rm_walk(unremoved, &entry.path()),
                            Err(e) => unremoved.push((path.to_path_buf(), e)),
                        }
                    }
                }
                Err(e) => {
                    // visitFileFailed: the directory could not be opened.
                    unremoved.push((path.to_path_buf(), e));
                    return;
                }
            }
            if let Err(e) = fs::remove_dir(path) {
                unremoved.push((path.to_path_buf(), e));
            }
        } else if let Err(e) = fs::remove_file(path) {
            unremoved.push((path.to_path_buf(), e));
        }
    }

    /// Rethrows a previously caught error, and never returns.
    ///
    /// Equivalent to `IOUtils.rethrowAlways(Throwable)`. Java declares a return
    /// type of `Error` purely so the compiler can see that the call site is
    /// unreachable afterwards; Rust says the same thing with the never type,
    /// so the signature carries the guarantee instead of documenting it.
    ///
    /// # Panics
    ///
    /// Always panics, with the message of `th`.
    pub fn rethrow_always(th: LuceneError) -> ! {
        panic!("{th}");
    }

    /// Applies `consumer` to every element of `collection`, even after one of
    /// them fails.
    ///
    /// Equivalent to `IOUtils.applyToAll(Collection, IOConsumer)`: the first
    /// error is returned and the later ones are suppressed into it, exactly as
    /// [`close`](Self::close) does for the closeables it is built on.
    ///
    /// # Errors
    ///
    /// Returns the first error the consumer raised.
    pub fn apply_to_all<T, C>(
        collection: Vec<Option<T>>,
        mut consumer: C,
    ) -> Result<(), LuceneError>
    where
        C: FnMut(T) -> Result<(), LuceneError>,
    {
        let mut err: Option<LuceneError> = None;
        for t in collection.into_iter().flatten() {
            if let Err(e) = consumer(t) {
                if err.is_none() {
                    err = Some(e);
                }
            }
        }
        match err {
            Some(e) => Err(e),
            None => Ok(()),
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
    ///
    /// `const` so that a type whose Java counterpart declares its shallow size
    /// as a `static final` — `Transition.BYTES_USED`, for one — can declare the
    /// same thing as an associated constant.
    pub const fn align_object_size(size: i64) -> i64 {
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

    /// Returns the shallow size of an array of `len` elements of type `T`.
    ///
    /// Equivalent to the private `RamUsageEstimator.shallowSizeOfArray(Object)`:
    /// the array header plus the element width times the length, aligned. Java
    /// reads the length and the component type by reflection; here both are
    /// known statically, and a reference element is charged
    /// [`NUM_BYTES_OBJECT_REF`](Self::NUM_BYTES_OBJECT_REF) through
    /// [`shallow_size_of`](Self::shallow_size_of) instead.
    pub fn shallow_size_of_array<T>(len: usize) -> i64 {
        let mut size = Self::NUM_BYTES_ARRAY_HEADER;
        if len > 0 {
            size += len as i64 * size_of::<T>() as i64;
        }
        Self::align_object_size(size)
    }

    /// Adds the representation size of one field to the size measured so far.
    ///
    /// Equivalent to `RamUsageEstimator.adjustForField(long, Field)`. Java reads
    /// the field's type by reflection to decide between a primitive width and
    /// [`NUM_BYTES_OBJECT_REF`](Self::NUM_BYTES_OBJECT_REF); Rust knows the width
    /// at the call site, so it is passed in. The arithmetic — Java's TODO about
    /// the absence of per-field alignment included — is unchanged.
    pub fn adjust_for_field(size_so_far: i64, field_size: i64) -> i64 {
        size_so_far + field_size
    }

    /// Runs `action` with the privileges Lucene's `RamUsageEstimator` asks for.
    ///
    /// Equivalent to the private `RamUsageEstimator.doPrivileged(PrivilegedAction)`,
    /// which wraps the reflective field lookup so the JVM's `SecurityManager`
    /// grants it. Rust has neither, so the privileged block is the action; see
    /// [`Constants::do_privileged`](Constants) for the same reasoning.
    fn do_privileged<T>(action: impl FnOnce() -> T) -> T {
        action()
    }

    /// Best-effort estimate of the memory an object of undetermined type costs.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfObject(Object, long)`: an
    /// [`Accountable`] reports its own size, and anything else falls back to
    /// `def_size` when that is positive, or to
    /// [`UNKNOWN_DEFAULT_RAM_BYTES_USED`](Self::UNKNOWN_DEFAULT_RAM_BYTES_USED).
    ///
    /// **Divergence from Lucene 10.5.0.** Java dispatches on the runtime class
    /// through a chain of `instanceof` tests covering `String`, the boxed
    /// primitives and every primitive array, and recurses into maps, collections
    /// and object arrays up to [`MAX_DEPTH`](Self::MAX_DEPTH). Rust has no
    /// runtime type information to dispatch on, so the one case the crate can
    /// recognise — an `Accountable` — is recognised, and every other type is
    /// charged the default, exactly as Java charges it for a class it does not
    /// know. The typed cases are reachable directly through
    /// [`size_of_string`](Self::size_of_string) and the `size_of_*` family,
    /// which is how the crate's callers, which know their types, reach them.
    pub fn size_of_object(o: Option<&dyn Accountable>, def_size: i64) -> i64 {
        Self::do_privileged(|| match o {
            None => 0,
            Some(accountable) => {
                let size = accountable.ram_bytes_used();
                if size > 0 {
                    size
                } else if def_size > 0 {
                    def_size
                } else {
                    Self::UNKNOWN_DEFAULT_RAM_BYTES_USED
                }
            }
        })
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
///
/// Equivalent to `org.apache.lucene.util.Constants`.
///
/// Lucene reads every value through `System.getProperty`. Rust's equivalent of
/// the JVM system-property table is the process environment, so [`get_sys_prop`]
/// consults it first and falls back to what the Rust runtime itself knows
/// (`std::env::consts`) for the handful of properties the JVM answers from the
/// platform rather than from the command line. Every derived constant is then
/// computed exactly as Lucene computes it.
///
/// [`get_sys_prop`]: Constants::get_sys_prop
#[derive(Debug, Copy, Clone)]
pub struct Constants;

impl Constants {
    /// Sentinel used when a system property cannot be read.
    pub const UNKNOWN: &'static str = "Unknown";

    /// Runs `action` with the privileges Lucene's `Constants.doPrivileged` asks
    /// for.
    ///
    /// Equivalent to `Constants.doPrivileged(PrivilegedAction)`, which exists
    /// only to wrap a property read in `AccessController.doPrivileged` so the
    /// JVM's `SecurityManager` grants it. Rust has neither an access controller
    /// nor a security manager: the caller's own privileges are the process's
    /// privileges, so the privileged block *is* the action. The wrapper is kept
    /// because Lucene routes both property readers through it, and keeping the
    /// seam means the two readers stay identical to Lucene's.
    fn do_privileged<T>(action: impl FnOnce() -> T) -> T {
        action()
    }

    /// Logs the warning Lucene logs when a property cannot be read.
    ///
    /// Equivalent to `Constants.logSecurityWarning(String)`. Java reaches here
    /// when the `SecurityManager` denies the read; this port reaches here when
    /// the environment holds a value that is not valid Unicode, the only way a
    /// lookup can fail other than being absent.
    fn log_security_warning(property: &str) {
        log::warn!("SecurityManager prevented access to system property: {property}");
    }

    /// Returns the value of a system property, or `None` when it is not set.
    ///
    /// Equivalent to `Constants.getSysProp(String)`.
    fn get_sys_prop(property: &str) -> Option<String> {
        match Self::do_privileged(|| env::var(property)) {
            Ok(value) => Some(value),
            Err(env::VarError::NotPresent) => Self::platform_prop(property),
            Err(env::VarError::NotUnicode(_)) => {
                Self::log_security_warning(property);
                None
            }
        }
    }

    /// Returns the value of a system property, or `def` when it is not set.
    ///
    /// Equivalent to `Constants.getSysProp(String, String)`.
    fn get_sys_prop_or(property: &str, def: &str) -> String {
        Self::get_sys_prop(property).unwrap_or_else(|| def.to_string())
    }

    /// The values the Rust runtime answers for the JVM properties that describe
    /// the platform rather than the invocation. The JVM fills these in the same
    /// way — from the platform, before any command line is consulted — so
    /// answering them here keeps `get_sys_prop` the single source every constant
    /// below reads from, exactly as in Lucene.
    fn platform_prop(property: &str) -> Option<String> {
        match property {
            "os.name" => Some(env::consts::OS.to_string()),
            "os.arch" => Some(env::consts::ARCH.to_string()),
            "sun.arch.data.model" => Some(if cfg!(target_pointer_width = "64") {
                "64".to_string()
            } else {
                usize::BITS.to_string()
            }),
            _ => None,
        }
    }

    /// JVM vendor info.
    ///
    /// Equivalent to `Constants.JVM_VENDOR`.
    pub fn jvm_vendor() -> String {
        Self::get_sys_prop_or("java.vm.vendor", Self::UNKNOWN)
    }

    /// JVM vendor name.
    ///
    /// Equivalent to `Constants.JVM_NAME`.
    pub fn jvm_name() -> String {
        Self::get_sys_prop_or("java.vm.name", Self::UNKNOWN)
    }

    /// Rust compiler version string.
    pub fn jvm_version() -> String {
        env!("CARGO_PKG_RUST_VERSION").to_string()
    }

    /// Operating-system name.
    ///
    /// Equivalent to `Constants.OS_NAME`.
    pub fn os_name() -> &'static str {
        env::consts::OS
    }

    /// True if running on Linux.
    ///
    /// Equivalent to `Constants.LINUX`.
    pub fn linux() -> bool {
        env::consts::OS == "linux"
    }

    /// True if running on Windows.
    ///
    /// Equivalent to `Constants.WINDOWS`.
    pub fn windows() -> bool {
        env::consts::OS == "windows"
    }

    /// True if running on SunOS.
    ///
    /// Equivalent to `Constants.SUN_OS`.
    pub fn sun_os() -> bool {
        env::consts::OS == "solaris"
    }

    /// True if running on macOS.
    ///
    /// Equivalent to `Constants.MAC_OS_X`.
    pub fn mac_os_x() -> bool {
        env::consts::OS == "macos"
    }

    /// True if running on FreeBSD.
    ///
    /// Equivalent to `Constants.FREE_BSD`.
    pub fn free_bsd() -> bool {
        env::consts::OS == "freebsd"
    }

    /// Operating-system architecture.
    ///
    /// Equivalent to `Constants.OS_ARCH`. Java spells x86-64 `amd64`; the Rust
    /// runtime spells the same architecture `x86_64`, and the crate compares
    /// against the Rust spelling throughout.
    pub fn os_arch() -> &'static str {
        env::consts::ARCH
    }

    /// Operating-system version.
    ///
    /// Equivalent to `Constants.OS_VERSION`.
    pub fn os_version() -> String {
        Self::get_sys_prop_or("os.version", Self::UNKNOWN)
    }

    /// Runtime vendor.
    ///
    /// Equivalent to `Constants.JAVA_VENDOR`.
    pub fn java_vendor() -> String {
        Self::get_sys_prop_or("java.vendor", Self::UNKNOWN)
    }

    /// True if the runtime is a client runtime with the optimising compiler
    /// disabled.
    ///
    /// Equivalent to `Constants.IS_CLIENT_VM`.
    pub fn is_client_vm() -> bool {
        Self::get_sys_prop_or("java.vm.info", "").contains("emulated-client")
    }

    /// True if the runtime is HotSpot-based and exposes its options.
    ///
    /// Equivalent to `Constants.IS_HOTSPOT_VM`.
    pub fn is_hotspot_vm() -> bool {
        HotspotVMOptions::is_hotspot_vm()
    }

    /// True if JVMCI is enabled (for example on GraalVM).
    ///
    /// Equivalent to `Constants.IS_JVMCI_VM`.
    pub fn is_jvmci_vm() -> bool {
        HotspotVMOptions::get("UseJVMCICompiler")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false)
    }

    /// True if the target pointer width is 64 bits.
    ///
    /// Equivalent to `Constants.JRE_IS_64BIT`.
    pub fn jre_is_64bit() -> bool {
        Self::is_64bit()
    }

    /// Equivalent to `Constants.is64Bit()`.
    fn is_64bit() -> bool {
        match Self::get_sys_prop("sun.arch.data.model") {
            Some(datamodel) => datamodel.contains("64"),
            None => Self::os_arch().contains("64"),
        }
    }

    /// True when FMA likely means a CPU instruction rather than software
    /// arithmetic.
    ///
    /// Equivalent to the private `Constants.HAS_FMA`.
    fn has_fma() -> bool {
        !Self::is_client_vm()
            && HotspotVMOptions::get("UseFMA")
                .and_then(|v| v.parse::<bool>().ok())
                .unwrap_or(false)
    }

    /// Equivalent to the private `Constants.MAX_VECTOR_SIZE`.
    fn max_vector_size() -> i32 {
        HotspotVMOptions::get("MaxVectorSize")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
    }

    /// Equivalent to the private `Constants.HAS_SSE4A`: an AMD CPU with SSE4a.
    fn has_sse4a() -> bool {
        HotspotVMOptions::get("UseXmmI2F")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false)
    }

    /// Equivalent to the private `Constants.HAS_AVX2`: AVX support of at least
    /// AVX2.
    fn has_avx2() -> bool {
        HotspotVMOptions::get("UseAVX")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
            >= 2
    }

    /// Equivalent to the private `Constants.HAS_SVE`: an Arm CPU with SVE.
    fn has_sve() -> bool {
        HotspotVMOptions::get("UseSVE")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
            >= 1
    }

    /// True when vector FMA is known to have faster throughput than a separate
    /// multiply and add.
    ///
    /// Equivalent to `Constants.HAS_FAST_VECTOR_FMA`.
    pub fn has_fast_vector_fma() -> bool {
        if Self::has_fma() {
            let value = Self::get_sys_prop_or("lucene.useVectorFMA", "auto");
            if value == "auto" {
                // newer Neoverse cores have their act together
                // the problem is just apple silicon (this is a practical heuristic)
                if Self::os_arch() == "aarch64" && !Self::mac_os_x() {
                    return true;
                }
                // zen cores or newer, its a wash, turn it on as it doesn't hurt
                // starts to yield gains for vectors only at zen4+
                if Self::has_sse4a() && Self::max_vector_size() >= 32 {
                    return true;
                }
                // intel has their act together
                if Self::os_arch() == "x86_64" && !Self::has_sse4a() {
                    return true;
                }
            } else {
                return value.eq_ignore_ascii_case("true");
            }
        }
        // everyone else is slow, until proven otherwise by benchmarks
        false
    }

    /// True when scalar FMA is known to have faster throughput than a separate
    /// multiply and add.
    ///
    /// Equivalent to `Constants.HAS_FAST_SCALAR_FMA`.
    pub fn has_fast_scalar_fma() -> bool {
        if Self::has_fma() {
            let value = Self::get_sys_prop_or("lucene.useScalarFMA", "auto");
            if value == "auto" {
                // newer Neoverse cores have their act together
                // the problem is just apple silicon (this is a practical heuristic)
                if Self::os_arch() == "aarch64" && !Self::mac_os_x() {
                    return true;
                }
                // latency becomes 4 for the Zen3 (0x19h), but still a wash
                // until the Zen4 anyway, and big drop on previous zens:
                if Self::has_sse4a() && Self::max_vector_size() >= 64 {
                    return true;
                }
                // intel has their act together
                if Self::os_arch() == "x86_64" && !Self::has_sse4a() {
                    return true;
                }
            } else {
                return value.eq_ignore_ascii_case("true");
            }
        }
        // everyone else is slow, until proven otherwise by benchmarks
        false
    }

    /// True when compress-and-cast is known to have fast throughput.
    ///
    /// Equivalent to `Constants.HAS_FAST_COMPRESS_MASK_CAST`.
    pub fn has_fast_compress_mask_cast() -> bool {
        if Self::os_arch() == "aarch64" && Self::has_sve() {
            return true;
        }
        if Self::os_arch() == "x86_64" && Self::has_avx2() {
            return true;
        }
        false
    }

    /// The default read advice used when opening index files.
    ///
    /// Equivalent to `Constants.DEFAULT_READADVICE`: `ReadAdvice::Normal`
    /// unless the `org.apache.lucene.store.defaultReadAdvice` property names
    /// another value.
    ///
    /// # Panics
    ///
    /// Panics when the property names no known advice, standing in for Java's
    /// `IllegalArgumentException` from `ReadAdvice.valueOf`.
    pub fn default_read_advice() -> crate::store::ReadAdvice {
        use crate::store::ReadAdvice;
        match Self::get_sys_prop("org.apache.lucene.store.defaultReadAdvice") {
            None => ReadAdvice::Normal,
            Some(a) => match a.to_uppercase().as_str() {
                "NORMAL" => ReadAdvice::Normal,
                "RANDOM" => ReadAdvice::Random,
                "SEQUENTIAL" => ReadAdvice::Sequential,
                other => panic!("No enum constant ReadAdvice.{other}"),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// InfoStream
// ---------------------------------------------------------------------------

/// Debugging logging sink for Lucene classes.
pub trait InfoStream: Send + Sync {
    /// Returns the binary name of the Lucene class this stream ports.
    ///
    /// Stands in for `getClass().getName()`, which Rust cannot answer: there is
    /// no runtime class object, and `std::any::type_name` names the Rust type,
    /// not the Java one. `LiveIndexWriterConfig.toString()` prints this
    /// stream's class name, so the name has to be carried by the type itself.
    ///
    /// There is no default: Java has no instance whose class is unknown, so
    /// every implementor states its own.
    fn java_class_name(&self) -> &'static str;

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
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.InfoStream$NoOutput"
    }

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

    /// Consumes these bits and returns them as
    /// [`LiveDocs`](crate::util::bit_set::LiveDocs) when they are one, or
    /// `None` when they are not.
    ///
    /// **Divergence from Lucene 10.5.0.** Java asks `bits instanceof LiveDocs`
    /// and casts (`LeafReader.java:409-411`). Rust cannot narrow an owned
    /// `Box<dyn Bits>` to a `Box<dyn LiveDocs>`, so the narrowing is a method
    /// the implementations that *are* live docs override. The default answers
    /// `None`, which is what the `instanceof` answers for every other
    /// implementation.
    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn crate::util::bit_set::LiveDocs>> {
        None
    }

    /// Returns the address of the bit set these bits read, which is how two
    /// handles on the same bits are told apart from two equal copies.
    ///
    /// **Addition, not a port.** Java compares `Bits` by reference
    /// (`PendingDeletes.needsRefresh`, `PendingDeletes.java:241-243`). This port
    /// hands bits around behind shared handles, so a handle reports the
    /// identity of the bits it shares; any other implementation is its own
    /// identity.
    fn identity(&self) -> *const () {
        self as *const Self as *const ()
    }
}

/// Returns whether `left` and `right` are the same bits, or both absent.
///
/// The port's equivalent of Java's reference comparison of two `Bits`
/// (`left == right`); see [`Bits::identity`].
pub fn same_bits(left: Option<&dyn Bits>, right: Option<&dyn Bits>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left.identity() == right.identity(),
        _ => false,
    }
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
    ///
    /// Equivalent to `org.apache.lucene.util.FixedBitSet#word2Array`.
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
    ///
    /// Equivalent to `org.apache.lucene.util.FixedBitSet#denseWord2Array`.
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

impl FixedBitSet {
    /// Shallow size of the Java class, the fixed part of
    /// [`FixedBitSet::ram_bytes_used`](Accountable::ram_bytes_used).
    ///
    /// Equivalent to the private `FixedBitSet.BASE_RAM_BYTES_USED`, which Java
    /// computes as
    /// `RamUsageEstimator.shallowSizeOfInstance(FixedBitSet.class)`: an object
    /// header, the `bits` reference and the `int` fields `numBits` and
    /// `numWords`. Measured against `lucene-core-10.5.0.jar` on a JVM with
    /// compressed references, that is 24 bytes.
    pub const BASE_RAM_BYTES_USED: i64 = RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 2 * 4,
    );
}

impl Accountable for FixedBitSet {
    /// Returns an estimate of the heap this bit set occupies.
    ///
    /// Equivalent to `org.apache.lucene.util.FixedBitSet#ramBytesUsed`
    /// (`FixedBitSet.java:203-205`), which is
    /// `BASE_RAM_BYTES_USED + RamUsageEstimator.sizeOf(bits)`.
    fn ram_bytes_used(&self) -> i64 {
        Self::BASE_RAM_BYTES_USED + RamUsageEstimator::size_of_u64(&self.bits)
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
// RefCount
// ---------------------------------------------------------------------------

/// The hook a [`RefCount`] runs when its count reaches zero.
///
/// Equivalent to what a Java subclass installs by overriding
/// `RefCount.release()`.
pub type RefCountRelease<T> = Box<dyn Fn(&T) -> Result<(), LuceneError> + Send + Sync>;

/// Manages reference counting for a given object.
///
/// Equivalent to `org.apache.lucene.util.RefCount`.
///
/// **Divergence from Lucene 10.5.0.** Java makes [`Self::release`] a
/// `protected` method that extensions override to free resources attached to
/// the managed object (`RefCount.java:40`); Rust has no subclassing, so the
/// override is a closure handed to [`Self::with_release`] and
/// [`Self::new`] keeps Java's default no-op. This mirrors how
/// [`crate::util::io_function::FilterIterator`] ports Lucene's other
/// single-overridable-method class.
pub struct RefCount<T> {
    ref_count: std::sync::atomic::AtomicI32,
    object: T,
    release: Option<RefCountRelease<T>>,
}

impl<T: Debug> Debug for RefCount<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefCount")
            .field("refCount", &self.ref_count)
            .field("object", &self.object)
            .finish()
    }
}

impl<T> RefCount<T> {
    /// Wraps `object` with a reference count of one and Lucene's default
    /// no-op release.
    ///
    /// Equivalent to `org.apache.lucene.util.RefCount#RefCount`.
    pub fn new(object: T) -> Self {
        Self {
            ref_count: std::sync::atomic::AtomicI32::new(1),
            object,
            release: None,
        }
    }

    /// Wraps `object` with a reference count of one, running `release` when
    /// the count reaches zero.
    ///
    /// Equivalent to instantiating a Java subclass that overrides
    /// `RefCount.release()`.
    pub fn with_release(object: T, release: RefCountRelease<T>) -> Self {
        Self {
            ref_count: std::sync::atomic::AtomicI32::new(1),
            object,
            release: Some(release),
        }
    }

    /// Called when reference counting hits 0. By default this does nothing,
    /// but the closure given to [`Self::with_release`] can e.g. release
    /// resources attached to the object managed by this instance.
    ///
    /// # Errors
    ///
    /// Returns whatever the release closure raised, standing in for Java's
    /// `IOException`.
    ///
    /// Equivalent to `org.apache.lucene.util.RefCount#release`.
    pub fn release(&self) -> Result<(), LuceneError> {
        match &self.release {
            Some(release) => release(&self.object),
            None => Ok(()),
        }
    }

    /// Decrements the reference count of this object. When it hits 0, calls
    /// [`Self::release`].
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::release`] raised, having put the reference
    /// back exactly as Java's `finally` block does, or
    /// [`LuceneError::IllegalState`] when the count went negative, which is
    /// Java's `IllegalStateException`.
    ///
    /// Equivalent to `org.apache.lucene.util.RefCount#decRef`.
    pub fn dec_ref(&self) -> Result<(), LuceneError> {
        let rc = self
            .ref_count
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
            - 1;
        if rc == 0 {
            if let Err(e) = self.release() {
                // Put the reference back on failure.
                self.ref_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Err(e);
            }
        } else if rc < 0 {
            return Err(LuceneError::IllegalState(format!(
                "too many decRef calls: refCount is {rc} after decrement"
            )));
        }
        Ok(())
    }

    /// Returns the managed object.
    ///
    /// Equivalent to `org.apache.lucene.util.RefCount#get`; Java hands back the
    /// reference itself, which in this owning port is a borrow.
    pub fn get(&self) -> &T {
        &self.object
    }

    /// Returns the current reference count.
    ///
    /// Equivalent to `org.apache.lucene.util.RefCount#getRefCount`.
    pub fn ref_count(&self) -> i32 {
        self.ref_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Increments the reference count. Calls to this method must be matched
    /// with calls to [`Self::dec_ref`].
    ///
    /// Equivalent to `org.apache.lucene.util.RefCount#incRef`.
    pub fn inc_ref(&self) {
        self.ref_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

// ---------------------------------------------------------------------------
// FixedBitSet: the inherited `Object.toString()`
// ---------------------------------------------------------------------------

impl Display for FixedBitSet {
    /// Lucene's `FixedBitSet` does not override `toString()`, so Java renders it
    /// with `Object.toString()`: the class name, `@`, and the identity hash in
    /// hexadecimal. That string reaches the outside world through
    /// `BitDocIdSet.toString()`, which concatenates the set it wraps, so the
    /// shape has to exist here too.
    ///
    /// **Divergence from Lucene 10.5.0.** Rust has no identity hash. Following
    /// the convention this port already uses for `Object.hashCode()` (see
    /// `IndexCommit`), the address of the value stands in for it. The digits
    /// therefore differ from a JVM's, exactly as two JVM runs differ from each
    /// other; nothing in Lucene parses them.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FixedBitSet@{}",
            crate::util::java_fmt::to_hex_string(self as *const Self as usize as i32)
        )
    }
}
