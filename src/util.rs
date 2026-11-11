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
pub mod atomic_ref_cell;
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
pub mod java_fmt;
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

pub use accountables::{Accountables, NamedAccountable};
pub use annotations::{IgnoreRandomChains, SuppressForbidden};
pub use bit_set::{
    bit_set_of, check_unpositioned, BitSet, BitSetIterator, BitSetIteratorOverrides,
    DocBaseBitSetIterator, FixedBits, LiveDocs,
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
    wait_interruptibly, Counter, InterruptHandle, InterruptedException, NamedThreadFactory,
    SameThreadExecutorService, SetOnce, WeakIdentityMap,
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
    ResourceLoader, ResourceLoaderAware, ServiceProvider,
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
pub use spi::{NamedSPI, NamedSPILoader, NamedSPIProvider, ServiceClass};
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
    IdentityLongValues, IntoIter as PriorityQueueIntoIter, JavaComparable, LongBitSet, LongValues,
    MergedIterator, PriorityQueue, PriorityQueueComparator, Version, ZeroesLongValues,
};
pub use query_builder::{QueryBuilder, QueryBuilderOps, TermAndBoost as QueryBuilderTermAndBoost};
pub use small_float::SmallFloat;
pub use string_helper::{compare_utf16, IntsRef, StringHelper, ID_LENGTH};
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
    hash::{Hash, Hasher},
    io,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    sync::{Arc, LazyLock},
};

use crate::store::java_nio_file as nio;

use crate::error::{LuceneError, LuceneErrorKind};
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

/// Java's `NumberFormatException` with `message` (`null` for `None`), which
/// `ArrayUtil.parseInt` throws (`ArrayUtil.java:66-110`).
fn number_format_exception(message: Option<&str>) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class: "java.lang.NumberFormatException",
        message: message.map(str::to_string),
        cause: None,
    })
}

/// The name `System.arraycopy`'s messages give the component type of an array
/// of `T`: the Java primitive a Rust primitive stands for, `object array`
/// otherwise.
pub(crate) fn java_array_element_type<T>() -> &'static str {
    match std::any::type_name::<T>() {
        "u8" | "i8" => "byte",
        "i16" => "short",
        "u16" => "char",
        "i32" => "int",
        "i64" => "long",
        "f32" => "float",
        "f64" => "double",
        "bool" => "boolean",
        _ => "object array",
    }
}

/// `ArrayUtil.copyOfSubArray(array, offset, offset + length)` for the
/// `deepCopyOf` methods of the `*Ref` classes (`ArrayUtil.java:654-658` and
/// its overloads): `System.arraycopy` throws `ArrayIndexOutOfBoundsException`
/// (`arraycopy: last source index N out of bounds for T[len]`) when the range
/// runs past the array, which is thrown here as that unchecked exception.
pub(crate) fn java_copy_of_sub_array<T: Clone>(
    array: &[T],
    offset: usize,
    length: usize,
    element_type: &str,
) -> Vec<T> {
    let end = offset as u64 + length as u64;
    if end > array.len() as u64 {
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.ArrayIndexOutOfBoundsException",
            Some(&format!(
                "arraycopy: last source index {end} out of bounds for {element_type}[{}]",
                array.len()
            )),
        );
    }
    array[offset..offset + length].to_vec()
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

    /// Initializes the buffer from the UTF-8 bytes of `text`.
    ///
    /// Equivalent to `BytesRef(CharSequence)` (`BytesRef.java:79-82`), which
    /// first allocates `UnicodeUtil.maxUTF8Length(text.length())` bytes —
    /// three per UTF-16 code unit, through `BytesRef(int)`, so an empty text
    /// gets an empty array — and then sets `length` to the number of UTF-8
    /// bytes `UnicodeUtil.UTF16toUTF8` wrote. The buffer is therefore longer
    /// than the value for any text that is not empty, and that length is
    /// visible through [`bytes`](Self::bytes), as `bytes.length` is in Java.
    ///
    /// # Panics
    ///
    /// When three bytes per UTF-16 code unit overflow an `int`, which is the
    /// `ArithmeticException` (`integer overflow`) of Java's
    /// `Math.multiplyExact` in `UnicodeUtil.maxUTF8Length`.
    pub fn from_char_sequence(text: &str) -> Self {
        let utf16_length = text.encode_utf16().count();
        let capacity = i32::try_from(utf16_length)
            .ok()
            .and_then(|length| {
                length.checked_mul(crate::util::unicode_util::MAX_UTF8_BYTES_PER_CHAR as i32)
            })
            .expect("integer overflow");
        let mut bytes = vec![0u8; capacity as usize];
        let utf8 = text.as_bytes();
        bytes[..utf8.len()].copy_from_slice(utf8);
        Self {
            bytes,
            offset: 0,
            length: utf8.len(),
        }
    }

    /// Returns a `BytesRef` whose content is a copy of `other`'s active slice,
    /// with offset zero.
    ///
    /// Equivalent to `BytesRef.deepCopyOf(BytesRef)` (`BytesRef.java:176-181`),
    /// which copies through `ArrayUtil.copyOfSubArray`: an `other` whose
    /// `offset + length` is past its array throws `System.arraycopy`'s
    /// `ArrayIndexOutOfBoundsException`.
    pub fn deep_copy_of(other: &BytesRef) -> Self {
        let bytes = java_copy_of_sub_array(&other.bytes, other.offset, other.length, "byte");
        Self {
            bytes,
            offset: 0,
            length: other.length,
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
    /// Returns `LuceneErrorKind::IllegalArgument` if the bytes are not valid UTF-8.
    pub fn utf8_to_string(&self) -> Result<String, LuceneError> {
        String::from_utf8(self.slice().to_vec())
            .map_err(|e| LuceneError::illegal_argument(format!("invalid UTF-8: {e}")))
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
    /// Returns `LuceneErrorKind::IllegalState` if any invariant is violated.
    pub fn is_valid(&self) -> Result<(), LuceneError> {
        if self.length > self.bytes.len() {
            return Err(LuceneError::illegal_state(format!(
                "length {} out of bounds (bytes.len() = {})",
                self.length,
                self.bytes.len()
            )));
        }
        if self.offset > self.bytes.len() {
            return Err(LuceneError::illegal_state(format!(
                "offset {} out of bounds (bytes.len() = {})",
                self.offset,
                self.bytes.len()
            )));
        }
        if self.offset + self.length > self.bytes.len() {
            return Err(LuceneError::illegal_state(format!(
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

    /// Returns the full underlying byte buffer for writing.
    ///
    /// Equivalent to `BytesRefBuilder.bytes()` (`BytesRefBuilder.java:41-44`)
    /// when the caller writes through the returned array, as Java callers
    /// may.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.inner.bytes
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
        debug_assert!(self.inner.offset == 0);
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
    ///
    /// Equivalent to `BytesRefBuilder.copyChars(CharSequence, int, int)`
    /// (`BytesRefBuilder.java:137-140`), which first runs
    /// `growNoCopy(UnicodeUtil.maxUTF8Length(len))` for the text's UTF-16
    /// length, so the buffer reaches the same capacity as Java's.
    pub fn copy_chars(&mut self, text: &str) {
        self.grow_no_copy(UnicodeUtil::max_utf8_length(text.encode_utf16().count()));
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
    ///
    /// Equivalent to `NumericUtils.doubleToSortableLong` (`NumericUtils.java:47-49`),
    /// which reads the bits with `Double.doubleToLongBits`: every NaN collapses
    /// to the canonical `0x7ff8000000000000`.
    pub fn double_to_sortable_long(value: f64) -> i64 {
        let bits = if value.is_nan() {
            0x7ff8_0000_0000_0000_i64
        } else {
            value.to_bits() as i64
        };
        Self::sortable_double_bits(bits)
    }

    /// Converts a sortable `long` back to a `double`.
    pub fn sortable_long_to_double(encoded: i64) -> f64 {
        f64::from_bits(Self::sortable_double_bits(encoded) as u64)
    }

    /// Converts a `float` to a sortable signed `int`.
    ///
    /// Equivalent to `NumericUtils.floatToSortableInt` (`NumericUtils.java:69-71`),
    /// which reads the bits with `Float.floatToIntBits`: every NaN collapses to
    /// the canonical `0x7fc00000`.
    pub fn float_to_sortable_int(value: f32) -> i32 {
        let bits = if value.is_nan() {
            0x7fc0_0000_i32
        } else {
            value.to_bits() as i32
        };
        Self::sortable_float_bits(bits)
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
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] `"a < b"` when `a < b`, the
    /// `IllegalArgumentException` Java throws (`NumericUtils.java:124-126`).
    pub fn subtract(
        bytes_per_dim: usize,
        dim: usize,
        a: &[u8],
        b: &[u8],
        result: &mut [u8],
    ) -> Result<(), LuceneError> {
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

        if borrow != 0 {
            return Err(LuceneError::illegal_argument("a < b".to_string()));
        }
        Ok(())
    }

    /// Writes `result = a + b` for the `dim`-th dimension, treating both as
    /// unsigned.
    ///
    /// Equivalent to `NumericUtils.add(int, int, byte[], byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] on overflow, the
    /// `IllegalArgumentException` Java throws (`NumericUtils.java:165-167`).
    pub fn add(
        bytes_per_dim: usize,
        dim: usize,
        a: &[u8],
        b: &[u8],
        result: &mut [u8],
    ) -> Result<(), LuceneError> {
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

        if carry != 0 {
            return Err(LuceneError::illegal_argument(format!(
                "a + b overflows bytesPerDim={bytes_per_dim}"
            )));
        }
        Ok(())
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
        (x & x.wrapping_sub(1)) == 0
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
    ///
    /// Stands in for `BitUtil.VH_BE_LONG.get(byte[], int)` (`BitUtil.java:174`);
    /// Rust has no `VarHandle`.
    pub fn read_be_long(src: &[u8], offset: usize) -> i64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&src[offset..offset + 8]);
        i64::from_be_bytes(bytes)
    }

    /// Writes a big-endian `i64` to `dst` at `offset`.
    ///
    /// Stands in for `BitUtil.VH_BE_LONG.set(byte[], int, long)`
    /// (`BitUtil.java:174`); Rust has no `VarHandle`.
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
    /// Reads the group varints of `dst[0..limit]`, including the tail vints,
    /// from a [`DataInput`] through the scalar path.
    ///
    /// Equivalent to `GroupVIntUtil.readGroupVInts$Baseline(DataInput, int[],
    /// int)` (`GroupVIntUtil.java:100-109`), which exists so a benchmark can
    /// measure [`read_group_vints_input`](Self::read_group_vints_input) against
    /// it ("DO not use! Only visible for benchmarking purposes!").
    ///
    /// # Errors
    ///
    /// Returns any error raised by the underlying input.
    pub fn read_group_vints_baseline<I: DataInput + ?Sized>(
        input: &mut I,
        dst: &mut [i32],
        limit: usize,
    ) -> Result<(), LuceneError> {
        let mut i = 0;
        while i + 4 <= limit {
            Self::read_group_vint_baseline(input, dst, i)?;
            i += 4;
        }
        while i < limit {
            dst[i] = input.read_v_int()?;
            i += 1;
        }
        Ok(())
    }

    /// Equivalent to the private `GroupVIntUtil.readGroupVInt$Baseline(DataInput,
    /// int[], int)` (`GroupVIntUtil.java:111-124`).
    fn read_group_vint_baseline<I: DataInput + ?Sized>(
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
    /// [`LuceneErrorKind::Arithmetic`] with the message `integer overflow` when a
    /// value does not fit in an unsigned 32-bit integer: the
    /// `ArithmeticException("integer overflow")` Lucene throws.
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

    /// Equivalent to the private `GroupVIntUtil.toInt(long)`
    /// (`GroupVIntUtil.java:174-179`): narrows a `long` that is required to
    /// hold an unsigned 32-bit value.
    fn to_int(value: i64) -> Result<i32, LuceneError> {
        if (value & 0xFFFF_FFFF_0000_0000u64 as i64) != 0 {
            return Err(LuceneError::integer_overflow());
        }
        Ok(value as i32)
    }

    fn num_bytes(v: i32) -> usize {
        let uv = (v as u32) | 1;
        4 - (uv.leading_zeros() >> 3) as usize
    }
}

// ---------------------------------------------------------------------------
// ArrayUtil
// ---------------------------------------------------------------------------

/// A comparator over two byte slices at given offsets, returning a negative,
/// zero or positive `i32` as Java's comparators do.
///
/// The zero digit of every run of Unicode decimal digits (general category
/// `Nd`) in the Basic Multilingual Plane outside ASCII, each run holding the
/// digits zero to nine in order: the 360 characters for which the JDK 25 that
/// runs the Lucene 10.5.0 oracle answers `Character.digit(char, 10) >= 0`
/// beyond `'0'..='9'` (`oracle/fix/DigitTable.java`).
const UNICODE_DECIMAL_ZEROS: [u32; 36] = [
    0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6, 0x0D66,
    0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90,
    0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
];

/// `java.lang.Character.digit(char, int)` for a `radix` in `2..=36`: the value
/// of a decimal digit of any script, or of a Latin letter `A`–`Z` in either
/// case, ASCII or fullwidth (`U+FF21`–`U+FF3A`, `U+FF41`–`U+FF5A`), counting
/// from ten; `None` — Java's -1 — when the character is neither or its value
/// is not below `radix`. A character outside the Basic Multilingual Plane is
/// not a digit, as Java's `char` cannot hold one.
pub fn java_character_digit(c: char, radix: u32) -> Option<u32> {
    let code = c as u32;
    let value = match code {
        0x30..=0x39 => code - 0x30,
        0x41..=0x5A => code - 0x41 + 10,
        0x61..=0x7A => code - 0x61 + 10,
        0xFF21..=0xFF3A => code - 0xFF21 + 10,
        0xFF41..=0xFF5A => code - 0xFF41 + 10,
        _ => UNICODE_DECIMAL_ZEROS
            .iter()
            .find(|&&zero| (zero..zero + 10).contains(&code))
            .map(|&zero| code - zero)?,
    };
    (value < radix).then_some(value)
}

/// `java.lang.Double.compare(double, double)`: the numeric order, then the
/// order of `Double.doubleToLongBits`, which folds every `NaN` into one value
/// above positive infinity and puts `-0.0` before `0.0`. Unlike
/// [`f64::total_cmp`], it does not order `NaN`s by sign and payload.
pub(crate) fn double_compare(a: f64, b: f64) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    // `Double.doubleToLongBits`: every `NaN` is the canonical quiet `NaN`.
    let bits = |value: f64| {
        if value.is_nan() {
            0x7ff8_0000_0000_0000_u64 as i64
        } else {
            value.to_bits() as i64
        }
    };
    bits(a).cmp(&bits(b))
}

/// Equivalent to the functional interface `ArrayUtil.ByteArrayComparator`,
/// whose one method is `int compare(byte[] a, int aI, byte[] b, int bI)`.
///
/// Java implements it with lambdas, one of which captures the key width
/// (`ArrayUtil.java:804-816`), and the classes that use one keep it in a
/// `final` field (`PointRangeQuery.comparator`, `BKDWriter.comparator`,
/// `HeapPointWriter.dimComparator`). This handle is the shared closure: cloning
/// it shares the same comparator, as copying a Java reference does, so the
/// structs that hold one keep their `Clone`. It renders as `ByteArrayComparator`
/// in `Debug`; a lambda has no state worth showing.
#[derive(Clone)]
pub struct ByteArrayComparator(std::sync::Arc<ByteArrayCompareFn>);

/// The signature of [`ByteArrayComparator::compare`].
type ByteArrayCompareFn = dyn Fn(&[u8], usize, &[u8], usize) -> i32 + Send + Sync;

impl ByteArrayComparator {
    /// Wraps `compare` as a comparator.
    pub fn new(
        compare: impl Fn(&[u8], usize, &[u8], usize) -> i32 + Send + Sync + 'static,
    ) -> Self {
        Self(std::sync::Arc::new(compare))
    }

    /// Compares the key of `a` at `a_offset` with the key of `b` at `b_offset`,
    /// returning a negative number, zero or a positive number.
    ///
    /// Equivalent to `ByteArrayComparator.compare(byte[], int, byte[], int)`.
    pub fn compare(&self, a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> i32 {
        (self.0)(a, a_offset, b, b_offset)
    }
}

impl std::fmt::Debug for ByteArrayComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ByteArrayComparator")
    }
}

/// Methods for manipulating arrays, matching Lucene's `ArrayUtil`.
pub struct ArrayUtil;

impl ArrayUtil {
    /// Maximum length for an array (`i32::MAX` minus an estimated header size).
    pub const MAX_ARRAY_LENGTH: usize = i32::MAX as usize - 16;

    /// Returns a size greater than or equal to `min_target_size`, generally
    /// over-allocating exponentially.
    ///
    /// Equivalent to `ArrayUtil.oversize(int, int)` (`ArrayUtil.java:134-193`).
    /// Java's `minTargetSize < 0` check has no counterpart, as a `usize`
    /// cannot be negative.
    ///
    /// # Panics
    ///
    /// Panics with Java's `IllegalArgumentException` when `min_target_size`
    /// exceeds [`Self::MAX_ARRAY_LENGTH`].
    pub fn oversize(min_target_size: usize, bytes_per_element: usize) -> usize {
        if min_target_size == 0 {
            // wait until at least one element is requested
            return 0;
        }
        if min_target_size > Self::MAX_ARRAY_LENGTH {
            panic!(
                "java.lang.IllegalArgumentException: requested array size {min_target_size} \
                 exceeds maximum array in java ({})",
                Self::MAX_ARRAY_LENGTH
            );
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
        Self::grow_exact_generic(array, new_length)
    }

    /// Returns a new `Vec<i32>` of exactly `new_length` containing a copy of `array`.
    pub fn grow_exact_int(array: &[i32], new_length: usize) -> Vec<i32> {
        Self::grow_exact_generic(array, new_length)
    }

    /// Returns a new `Vec<i64>` of exactly `new_length` containing a copy of `array`.
    pub fn grow_exact_long(array: &[i64], new_length: usize) -> Vec<i64> {
        Self::grow_exact_generic(array, new_length)
    }

    /// Copies a sub-range of a byte array into a new vector.
    ///
    /// Equivalent to `ArrayUtil.copyOfSubArray(byte[], int, int)`; see
    /// [`copy_of_sub_array_generic`](Self::copy_of_sub_array_generic) for the
    /// exceptions.
    pub fn copy_of_sub_array(array: &[u8], from: usize, to: usize) -> Vec<u8> {
        Self::copy_of_sub_array_generic(array, from, to)
    }

    /// Copies a sub-range of an `i32` array into a new vector.
    ///
    /// Equivalent to `ArrayUtil.copyOfSubArray(int[], int, int)`.
    pub fn copy_of_sub_array_int(array: &[i32], from: usize, to: usize) -> Vec<i32> {
        Self::copy_of_sub_array_generic(array, from, to)
    }

    /// Copies a sub-range of an `i64` array into a new vector.
    ///
    /// Equivalent to `ArrayUtil.copyOfSubArray(long[], int, int)`.
    pub fn copy_of_sub_array_long(array: &[i64], from: usize, to: usize) -> Vec<i64> {
        Self::copy_of_sub_array_generic(array, from, to)
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
        // `assert minLength >= 0 : "length must be positive (got " + minLength +
        // "): likely integer overflow?"` (`ArrayUtil.java:330-331`): a length
        // past `i32::MAX` is a negative Java `int`.
        debug_assert!(
            i32::try_from(min_length).is_ok(),
            "length must be positive (got {}): likely integer overflow?",
            min_length as i32
        );
        if min_length > max_length {
            return Err(LuceneError::illegal_argument(format!(
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
    /// Equivalent to the `ArrayUtil.growExact` overloads
    /// (`ArrayUtil.java:242-246` and the others). Java declares one per
    /// primitive plus a generic one; Rust needs only the generic form. A
    /// `new_length` below the array's length makes Java's
    /// `System.arraycopy(array, 0, copy, 0, array.length)` throw
    /// `ArrayIndexOutOfBoundsException`, thrown here as that unchecked
    /// exception.
    pub fn grow_exact_generic<T: Copy + Default>(array: &[T], new_length: usize) -> Vec<T> {
        if new_length < array.len() {
            crate::store::java_nio_file::throw_unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                Some(&format!(
                    "arraycopy: last destination index {} out of bounds for {}[{new_length}]",
                    array.len(),
                    java_array_element_type::<T>()
                )),
            );
        }
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
    /// Equivalent to the `ArrayUtil.copyOfSubArray` overloads
    /// (`ArrayUtil.java:654-658` and the others): `new T[to - from]` throws
    /// `NegativeArraySizeException` when `to < from`, and `System.arraycopy`
    /// throws `ArrayIndexOutOfBoundsException` when `to` is past the array;
    /// both are thrown here as those unchecked exceptions.
    pub fn copy_of_sub_array_generic<T: Clone>(array: &[T], from: usize, to: usize) -> Vec<T> {
        if to < from {
            crate::store::java_nio_file::throw_unchecked(
                "java.lang.NegativeArraySizeException",
                Some(&format!("-{}", from - to)),
            );
        }
        java_copy_of_sub_array(array, from, to - from, java_array_element_type::<T>())
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
        // Character.MIN_RADIX / Character.MAX_RADIX: `throw new
        // NumberFormatException()`, whose message is `null`.
        if !(2..=36).contains(&radix) {
            return Err(number_format_exception(None));
        }
        if len == 0 {
            return Err(number_format_exception(Some("chars length is 0")));
        }
        let mut i = 0usize;
        let negative = chars[offset] == '-';
        if negative {
            i += 1;
            if i == len {
                return Err(number_format_exception(Some("can't convert to an int")));
            }
            offset += 1;
            len -= 1;
        }
        Self::parse(chars, offset, len, radix, negative)
    }

    /// Parses the code units of `chars[offset..offset + len]` as a signed
    /// decimal integer.
    ///
    /// Equivalent to `ArrayUtil.parseInt(char[], int, int)`
    /// (`ArrayUtil.java:52-54`), which delegates with radix 10.
    ///
    /// # Errors
    ///
    /// As [`parse_int_radix`](Self::parse_int_radix).
    pub fn parse_int(chars: &[char], offset: usize, len: usize) -> Result<i32, LuceneError> {
        Self::parse_int_radix(chars, offset, len, 10)
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
        let unparsable = || number_format_exception(Some("Unable to parse"));
        let max = i32::MIN / radix as i32;
        let mut result: i32 = 0;
        for i in 0..len {
            let digit = match java_character_digit(chars[i + offset], radix) {
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
    /// the two widths that dominate — 8 and 4 — and otherwise returns a lambda
    /// that compares exactly `num_bytes` unsigned bytes
    /// (`Arrays.compareUnsigned(a, aOffset, aOffset + numBytes, b, bOffset,
    /// bOffset + numBytes)`, `ArrayUtil.java:804-816`).
    pub fn get_unsigned_comparator(num_bytes: usize) -> ByteArrayComparator {
        match num_bytes {
            8 => ByteArrayComparator::new(Self::compare_unsigned8),
            4 => ByteArrayComparator::new(Self::compare_unsigned4),
            _ => ByteArrayComparator::new(
                move |a: &[u8], a_offset: usize, b: &[u8], b_offset: usize| {
                    Self::compare_unsigned_len(a, a_offset, b, b_offset, num_bytes)
                },
            ),
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

/// A deferred `close()`: what the [`IOUtils`] close family takes for each
/// `java.io.Closeable`.
///
/// Java passes the close family `Closeable` objects, or lambdas — `Closeable`
/// is a functional interface (`IndexWriter.java:2652-2669`,
/// `NRTCachingDirectory.java:214-224`). The port passes closures that perform
/// the `close()`. The family is generic over the closure, so a call closing
/// resources of one type needs no box; this boxed form lets one call mix
/// resources of different types, as Java's varargs do. Build one with
/// [`IOUtils::closeable`].
pub type Closer<'a> = Box<dyn FnOnce() -> Result<(), LuceneError> + 'a>;

/// What a `catch (Throwable t)` around one `close()` catches: the error the
/// closeable returned, or the panic it raised.
enum Thrown {
    /// A returned [`LuceneError`].
    Error(LuceneError),
    /// A panic payload, Rust's form of the unchecked throwables the port does
    /// not model as a [`LuceneError`].
    Panic(Box<dyn std::any::Any + Send>),
}

impl Thrown {
    /// Whether Java would see a `java.lang.Error`: an `AssertionError`, the
    /// only `Error` the port models as a [`LuceneError`], or a panic.
    ///
    /// **Divergence from Lucene 10.5.0.** A panic is the port's form of both
    /// Java's unchecked exceptions (`IndexOutOfBoundsException`, ...) and its
    /// `Error`s (a failed `assert`), which cannot be told apart; it is treated
    /// as an `Error`, so it is rethrown and never swallowed.
    fn is_java_error(&self) -> bool {
        match self {
            Self::Error(e) => matches!(
                e,
                LuceneError {
                    kind: LuceneErrorKind::AssertionError(_),
                    ..
                }
            ),
            Self::Panic(_) => true,
        }
    }

    /// `this.addSuppressed(other)`.
    ///
    /// A panic payload holds no suppressed list, and a panic cannot be kept
    /// as a [`LuceneError`]: suppressing into a panic, or suppressing a
    /// panic, drops the suppressed one.
    fn add_suppressed(&mut self, other: Thrown) {
        if let (Self::Error(first), Self::Error(second)) = (self, other) {
            first.add_suppressed(second);
        }
    }

    /// Java's `throw`: returns the error, or resumes the panic.
    fn rethrow(self) -> LuceneError {
        match self {
            Self::Error(e) => e,
            Self::Panic(payload) => std::panic::resume_unwind(payload),
        }
    }
}

/// A throwable `IOUtils.useOrSuppress` can add a suppressed one to.
trait Suppressible {
    /// `this.addSuppressed(other)`.
    fn suppress(&mut self, other: Self);
}

impl Suppressible for Thrown {
    fn suppress(&mut self, other: Self) {
        self.add_suppressed(other);
    }
}

impl Suppressible for io::Error {
    fn suppress(&mut self, other: Self) {
        let first = std::mem::replace(self, io::Error::other(""));
        *self = nio::add_suppressed(first, &other);
    }
}

/// Utilities for dealing with closeable resources and file-system operations.
///
/// Port of `org.apache.lucene.util.IOUtils`.
pub struct IOUtils;

impl IOUtils {
    /// The canonical name of the UTF-8 charset.
    ///
    /// Equivalent to `IOUtils.UTF_8`.
    pub const UTF_8: &'static str = "UTF-8";

    /// Boxes `close` as a non-null closeable, so one call of the close family
    /// can mix resources of different types.
    ///
    /// The Java close family takes `Closeable...`; see [`Closer`].
    pub fn closeable<'a>(
        close: impl FnOnce() -> Result<(), LuceneError> + 'a,
    ) -> Option<Closer<'a>> {
        Some(Box::new(close))
    }

    /// Runs one `close()` as the family's `try { object.close(); } catch
    /// (Throwable t)` does: a returned error and a panic are both caught.
    fn close_one<C>(object: C) -> Option<Thrown>
    where
        C: FnOnce() -> Result<(), LuceneError>,
    {
        // The closeable is not touched again whatever it left behind, and the
        // panic is resumed or dropped as Java rethrows or suppresses it.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(object)) {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(Thrown::Error(e)),
            Err(payload) => Some(Thrown::Panic(payload)),
        }
    }

    /// Closes all given closeables; `None` entries are ignored. After
    /// everything is closed, returns the first error hit while closing, or
    /// `Ok` when there was none.
    ///
    /// Equivalent to `IOUtils.close(Closeable...)` and `IOUtils.close(Iterable)`
    /// (`IOUtils.java:69-93`): every closeable is closed even after one fails,
    /// and the first throwable is rethrown, the later ones suppressed into it
    /// ([`use_or_suppress`](Self::use_or_suppress)). A closeable that panics is
    /// caught like any `Throwable`; the panic is resumed once every closeable
    /// is closed when it was the first failure, and dropped, as Java
    /// suppresses it, when it was not.
    ///
    /// # Errors
    ///
    /// Returns the first error a closeable returned, unchanged.
    pub fn close<I, C>(objects: I) -> Result<(), LuceneError>
    where
        I: IntoIterator<Item = Option<C>>,
        C: FnOnce() -> Result<(), LuceneError>,
    {
        let mut th: Option<Thrown> = None;
        for object in objects.into_iter().flatten() {
            if let Some(t) = Self::close_one(object) {
                th = Some(Self::use_or_suppress(th, t));
            }
        }
        match th {
            Some(th) => Err(th.rethrow()),
            None => Ok(()),
        }
    }

    /// Closes all given closeables, swallowing every failure but a Java
    /// `Error`; `None` entries are ignored. Even when an `Error` is hit, every
    /// closeable is closed.
    ///
    /// Equivalent to `IOUtils.closeWhileHandlingException(Closeable...)` and
    /// `IOUtils.closeWhileHandlingException(Iterable)` (`IOUtils.java:103-136`).
    /// Java's method returns nothing and throws only the first `Error`, which
    /// is unchecked; the port models Java's `AssertionError` as
    /// [`LuceneErrorKind::AssertionError`], so that one comes back as `Err`, and a
    /// panic is resumed (see [`Thrown::is_java_error`]). A caller propagates
    /// the `Err` with `?`, as the `Error` propagates out of the Java caller —
    /// out of a `finally` block too, where it replaces the exception in
    /// flight.
    ///
    /// # Errors
    ///
    /// Returns the first [`LuceneErrorKind::AssertionError`] a closeable returned.
    pub fn close_while_handling_exception<I, C>(objects: I) -> Result<(), LuceneError>
    where
        I: IntoIterator<Item = Option<C>>,
        C: FnOnce() -> Result<(), LuceneError>,
    {
        let mut first_error: Option<Thrown> = None;
        let mut first_throwable: Option<Thrown> = None;
        for object in objects.into_iter().flatten() {
            match Self::close_one(object) {
                Some(t) if t.is_java_error() => {
                    first_error = Some(Self::use_or_suppress(first_error, t));
                }
                Some(t) => first_throwable = Some(Self::use_or_suppress(first_throwable, t)),
                None => {}
            }
        }
        match first_error {
            Some(mut first_error) => {
                // we ensure that we bubble up any errors. We can't recover from these but need
                // to make sure they are bubbled up. if a non-Error is thrown we also add the
                // suppressed exceptions to it.
                if let Some(first_throwable) = first_throwable {
                    first_error.add_suppressed(first_throwable);
                }
                Err(first_error.rethrow())
            }
            None => Ok(()),
        }
    }

    /// Deletes all given files, suppressing all thrown exceptions; `None`
    /// entries are ignored.
    ///
    /// Equivalent to `IOUtils.deleteFilesIgnoringExceptions(Collection)`
    /// (`IOUtils.java:278-290`): `Files.delete` through each path's provider.
    pub fn delete_files_ignoring_exceptions<I, P>(files: I)
    where
        I: IntoIterator<Item = Option<P>>,
        P: nio::IntoNioPath,
    {
        for path in files.into_iter().flatten() {
            let path = path.into_nio_path();
            // `catch (Throwable ignored)`: an unchecked exception is ignored
            // too.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = nio::Files::delete(&path);
            }));
        }
    }

    /// Deletes all given paths if they exist; `None` entries are ignored.
    /// After everything is deleted, the first exception hit is returned.
    ///
    /// Equivalent to `IOUtils.deleteFilesIfExist(Collection)`
    /// (`IOUtils.java:310-325`): `Files.deleteIfExists` through each path's
    /// provider.
    ///
    /// # Errors
    ///
    /// The first failure; the later ones are suppressed.
    pub fn delete_files_if_exist<I, P>(files: I) -> Result<(), LuceneError>
    where
        I: IntoIterator<Item = Option<P>>,
        P: nio::IntoNioPath,
    {
        let mut err: Option<io::Error> = None;
        for path in files.into_iter().flatten() {
            if let Err(e) = nio::Files::delete_if_exists(&path.into_nio_path()) {
                err = Some(Self::use_or_suppress(err, e));
            }
        }
        match err {
            Some(e) => Err(LuceneError::from(LuceneErrorKind::Io(e))),
            None => Ok(()),
        }
    }

    /// Forces all writes for the given file to durable storage.
    ///
    /// Equivalent to `IOUtils.fsync(Path, boolean)` (`IOUtils.java:452-484`):
    /// `FileChannel.open(fileToSync, isDir ? READ : WRITE)` through the path's
    /// provider — a regular file is opened for writing, which the fsync needs
    /// to have an effect; a directory for reading — then `force(true)`, whose
    /// failure is ignored for a directory because not every file system
    /// allows it. On Windows a directory cannot be opened at all, so only its
    /// existence is checked.
    ///
    /// # Errors
    ///
    /// Returns the error opening the file raises (`NoSuchFileException` for a
    /// missing file), or the error the fsync of a regular file raises.
    pub fn fsync(file_to_sync: impl nio::IntoNioPath, is_dir: bool) -> Result<(), LuceneError> {
        let file_to_sync = file_to_sync.into_nio_path();
        if is_dir && Constants::windows() {
            // opening a directory on Windows fails, directories can not be fsynced there
            if !nio::Files::exists(&file_to_sync, &[]) {
                // yet do not suppress trying to fsync directories that do not exist
                return Err(LuceneError::from(LuceneErrorKind::Io(
                    nio::no_such_file_exception(file_to_sync.to_string()),
                )));
            }
            return Ok(());
        }
        let mode = if is_dir {
            nio::OpenOption::Read
        } else {
            nio::OpenOption::Write
        };
        let file = <dyn nio::FileChannel>::open(&file_to_sync, &[mode])?;
        // `try (final FileChannel file = ...)`: closed whatever `force`
        // does; a failure to close is suppressed by the failure of `force`.
        let forced = file.force(true);
        let result = match forced {
            Err(e) => {
                if is_dir {
                    debug_assert!(
                        !(Constants::linux() || Constants::mac_os_x()),
                        "On Linux and MacOSX fsyncing a directory should not throw IOException, \
                         we just don't want to rely on that in production (undocumented). Got: {e}"
                    );
                    // Ignore exception if it is a directory
                    Ok(())
                } else {
                    // Throw original exception
                    Err(LuceneError::from(LuceneErrorKind::Io(e)))
                }
            }
            Ok(()) => Ok(()),
        };
        let closed = file.close();
        result?;
        closed?;
        Ok(())
    }

    /// Returns `second` when there is no `first`, and `first` otherwise, with
    /// `second` added to its suppressed list.
    ///
    /// Equivalent to `IOUtils.useOrSuppress(T, T)` (`IOUtils.java:490-497`).
    fn use_or_suppress<T: Suppressible>(first: Option<T>, second: T) -> T {
        match first {
            None => second,
            Some(mut first) => {
                first.suppress(second);
                first
            }
        }
    }

    /// Closes all given closeables while the failure `ex` is in flight, and
    /// returns the failure the caller must propagate; `None` entries are
    /// ignored. Even when a Java `Error` is hit, every closeable is closed.
    ///
    /// Equivalent to `IOUtils.closeWhileSuppressingExceptions(Throwable,
    /// Closeable...)` and `IOUtils.closeWhileSuppressingExceptions(Throwable,
    /// Iterable)` (`IOUtils.java:145-177`), whose caller rethrows `ex` once it
    /// returns (`StandardDirectoryReader.java:103-105`): the return value is
    /// that `ex`, unchanged, or — when `ex` is not a Java `Error` and a
    /// closeable hits one — that first `Error`, which Java throws with `ex`
    /// suppressed into it. Java's `Error` is [`LuceneErrorKind::AssertionError`],
    /// or a panic, which is resumed once every closeable is closed (see
    /// [`Thrown::is_java_error`]). Every other failure is suppressed into
    /// `ex`.
    ///
    /// Java suppresses `ex` into the first `Error` as soon as it is hit, and
    /// keeps adding to `ex` afterwards through the shared reference; the port
    /// adds to `ex` while it owns it and suppresses it into the `Error` at the
    /// end, which yields the same two lists.
    pub fn close_while_suppressing_exceptions<I, C>(mut ex: LuceneError, objects: I) -> LuceneError
    where
        I: IntoIterator<Item = Option<C>>,
        C: FnOnce() -> Result<(), LuceneError>,
    {
        let ex_is_error = matches!(ex.kind(), LuceneErrorKind::AssertionError(_));
        let mut first_error: Option<Thrown> = None;
        for object in objects.into_iter().flatten() {
            if let Some(e) = Self::close_one(object) {
                if !ex_is_error && first_error.is_none() && e.is_java_error() {
                    // don't try and suppress it - this Error should be the thing that is thrown
                    first_error = Some(e);
                } else if let Thrown::Error(e) = e {
                    ex.add_suppressed(e);
                }
                // A suppressed panic is dropped (see `Thrown::add_suppressed`).
            }
        }
        match first_error {
            Some(mut first_error) => {
                // `firstError.addSuppressed(ex)`.
                first_error.add_suppressed(Thrown::Error(ex));
                first_error.rethrow()
            }
            None => ex,
        }
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
            return Err(LuceneError::unsupported_operation(format!(
                "unsupported charset: {char_set}"
            )));
        }
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes)?;
        let text = String::from_utf8(bytes).map_err(|e| {
            LuceneError::from(LuceneErrorKind::Other(format!(
                "MalformedInputException: {e}"
            )))
        })?;
        Ok(io::Cursor::new(text))
    }

    /// Enforces the existence of a resource that a lookup reports as absent by
    /// returning nothing.
    ///
    /// Equivalent to `IOUtils.requireResourceNonNull(T, String)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::NotFound`] when `resource` is `None`, standing in
    /// for Java's `FileNotFoundException`.
    pub fn require_resource_non_null<T>(resource: Option<T>, name: &str) -> Result<T, LuceneError> {
        match resource {
            Some(resource) => Ok(resource),
            None => Err(LuceneError::from(LuceneErrorKind::NotFound(format!(
                "The resource '{name}' was not found."
            )))),
        }
    }

    /// Deletes one or more files or directories, and everything underneath
    /// them.
    ///
    /// Equivalent to `IOUtils.rm(Path...)` (`IOUtils.java:333-348`): every
    /// location is attempted, the ones that could not be removed are collected
    /// in the order they were attempted, and a single `IOException` naming all
    /// of them — each with its absolute path and `Throwable.toString()` of its
    /// failure — is raised at the end.
    ///
    /// # Errors
    ///
    /// Returns an error listing every path that could not be removed.
    pub fn rm<I, P>(locations: I) -> Result<(), LuceneError>
    where
        I: IntoIterator<Item = P>,
        P: nio::IntoNioPath,
    {
        let mut unremoved: Vec<(nio::Path, io::Error)> = Vec::new();
        Self::rm_into(&mut unremoved, locations);
        if unremoved.is_empty() {
            return Ok(());
        }
        let mut b =
            String::from("Could not remove the following files (in the order of attempts):\n");
        for (path, cause) in &unremoved {
            b.push_str(&format!(
                "   {}: {}\n",
                path.to_absolute_path(),
                nio::java_to_string(cause)
            ));
        }
        Err(LuceneError::from(LuceneErrorKind::Io(nio::io_exception(b))))
    }

    /// Equivalent to the private `IOUtils.rm(LinkedHashMap, Path...)`
    /// (`IOUtils.java:350-404`): `Files.walkFileTree` over each existing
    /// location, deleting files before the directory that holds them, and
    /// recording — rather than propagating — every failure. A path already
    /// recorded keeps its first position and takes the later failure, as
    /// `LinkedHashMap.put` does.
    fn rm_into<I, P>(unremoved: &mut Vec<(nio::Path, io::Error)>, locations: I)
    where
        I: IntoIterator<Item = P>,
        P: nio::IntoNioPath,
    {
        struct Visitor<'a>(&'a mut Vec<(nio::Path, io::Error)>);

        impl Visitor<'_> {
            fn put(&mut self, path: &nio::Path, error: io::Error) {
                match self.0.iter_mut().find(|(p, _)| p == path) {
                    Some(entry) => entry.1 = error,
                    None => self.0.push((path.clone(), error)),
                }
            }
        }

        impl nio::FileVisitor for Visitor<'_> {
            fn pre_visit_directory(
                &mut self,
                _dir: &nio::Path,
                _attrs: &nio::BasicFileAttributes,
            ) -> io::Result<nio::FileVisitResult> {
                Ok(nio::FileVisitResult::Continue)
            }

            fn post_visit_directory(
                &mut self,
                dir: &nio::Path,
                impossible: Option<io::Error>,
            ) -> io::Result<nio::FileVisitResult> {
                debug_assert!(impossible.is_none());
                if let Err(e) = nio::Files::delete(dir) {
                    self.put(dir, e);
                }
                Ok(nio::FileVisitResult::Continue)
            }

            fn visit_file(
                &mut self,
                file: &nio::Path,
                _attrs: &nio::BasicFileAttributes,
            ) -> io::Result<nio::FileVisitResult> {
                if let Err(e) = nio::Files::delete(file) {
                    self.put(file, e);
                }
                Ok(nio::FileVisitResult::Continue)
            }

            fn visit_file_failed(
                &mut self,
                file: &nio::Path,
                exc: io::Error,
            ) -> io::Result<nio::FileVisitResult> {
                self.put(file, exc);
                Ok(nio::FileVisitResult::Continue)
            }
        }

        for location in locations {
            let location = location.into_nio_path();
            // TODO: remove this leniency!
            if nio::Files::exists(&location, &[]) {
                let _ = nio::Files::walk_file_tree(&location, &mut Visitor(unremoved));
            }
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
    /// them fails; `None` elements are skipped.
    ///
    /// Equivalent to `IOUtils.applyToAll(Collection, IOConsumer)`
    /// (`IOUtils.java:505-511`), which hands [`close`](Self::close) one
    /// closeable per non-null element: the first error is returned and the
    /// later ones are suppressed into it.
    ///
    /// # Errors
    ///
    /// Returns the first error the consumer raised.
    pub fn apply_to_all<T, C>(collection: Vec<Option<T>>, consumer: C) -> Result<(), LuceneError>
    where
        C: FnMut(T) -> Result<(), LuceneError>,
    {
        let consumer = std::cell::RefCell::new(consumer);
        let consumer = &consumer;
        Self::close(
            collection
                .into_iter()
                .flatten()
                .map(|t| Some(move || (consumer.borrow_mut())(t))),
        )
    }
}

// ---------------------------------------------------------------------------
// Accountable
// ---------------------------------------------------------------------------

/// Trait for objects whose heap memory usage can be estimated.
///
/// Equivalent to `org.apache.lucene.util.Accountable`.
pub trait Accountable {
    /// Returns an estimate of the heap memory used by this object in bytes.
    fn ram_bytes_used(&self) -> i64;

    /// Returns nested accountable resources, or an empty list.
    ///
    /// Equivalent to `Accountable.getChildResources()` (`Accountable.java:41-50`),
    /// whose default is an empty list. A child may be a resource this object
    /// holds — boxed as a reference, through the `&T` implementation below — or
    /// a described view built with [`Accountables`].
    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        Vec::new()
    }

    /// Returns the text [`Accountables`] labels this resource with.
    ///
    /// Equivalent to the `toString()` that `Accountables.toString` and
    /// `Accountables.namedAccountable` read off every `Accountable`
    /// (`Accountables.java:51, 75`).
    ///
    /// **Divergence from Lucene 10.5.0.** A Java class that does not override
    /// `toString()` answers `getClass().getName() + "@" +
    /// Integer.toHexString(hashCode())`; the default here renders the Rust
    /// type name and the object's address in that `name@hex` shape, as
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// does. A type whose Java class is named in such a description overrides
    /// it with that class's binary name; one whose Java class overrides
    /// `toString()` overrides it with that text.
    fn accountable_to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string(std::any::type_name::<Self>(), self)
    }
}

/// A borrowed resource is the resource, so that
/// [`child_resources`](Accountable::child_resources) can hand out references
/// to what an object holds, as Java hands out the objects themselves.
impl<T: Accountable + ?Sized> Accountable for &T {
    fn ram_bytes_used(&self) -> i64 {
        (**self).ram_bytes_used()
    }

    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        (**self).child_resources()
    }

    fn accountable_to_string(&self) -> String {
        (**self).accountable_to_string()
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

/// An object as `RamUsageEstimator.sizeOfObject` dispatches on it: the
/// runtime class that its chain of `instanceof` tests recognises
/// (`RamUsageEstimator.java:396-432`).
///
/// Holds every class of that chain, in its order; a `null` argument is `None`
/// at the call.
#[derive(Clone, Copy)]
pub enum SizedObject<'a> {
    /// An `Accountable`, sized by its own `ramBytesUsed()`.
    Accountable(&'a dyn Accountable),
    /// A `String`, sized by `RamUsageEstimator.sizeOf(String)`.
    String(&'a str),
    /// A `boolean[]`, sized by `RamUsageEstimator.sizeOf(boolean[])`.
    BoolArray(&'a [bool]),
    /// A `byte[]`, sized by `RamUsageEstimator.sizeOf(byte[])`.
    ByteArray(&'a [u8]),
    /// A `char[]`, sized by `RamUsageEstimator.sizeOf(char[])`.
    CharArray(&'a [u16]),
    /// A `double[]`, sized by `RamUsageEstimator.sizeOf(double[])`.
    DoubleArray(&'a [f64]),
    /// A `float[]`, sized by `RamUsageEstimator.sizeOf(float[])`.
    FloatArray(&'a [f32]),
    /// An `int[]`, sized by `RamUsageEstimator.sizeOf(int[])`.
    IntArray(&'a [i32]),
    /// A boxed `Integer`, sized by `RamUsageEstimator.sizeOf(Integer)`.
    Integer(i32),
    /// A boxed `Long`, sized by `RamUsageEstimator.sizeOf(Long)`.
    Long(i64),
    /// A `long[]`, sized by `RamUsageEstimator.sizeOf(long[])`.
    LongArray(&'a [i64]),
    /// A `short[]`, sized by `RamUsageEstimator.sizeOf(short[])`.
    ShortArray(&'a [i16]),
    /// A `String[]`, sized by `RamUsageEstimator.sizeOf(String[])`.
    StringArray(&'a [Option<&'a str>]),
    /// A `Query`, sized by `RamUsageEstimator.sizeOf(Query, long)`.
    Query(&'a dyn crate::search::Query),
    /// A `Map`, sized by `RamUsageEstimator.sizeOfMap(Map, int, long)`.
    Map(&'a dyn SizedMap),
    /// A `Collection`, sized by
    /// `RamUsageEstimator.sizeOfCollection(Collection, int, long)`.
    Collection(&'a dyn SizedCollection),
    /// An instance of a class the `instanceof` chain does not know, such as
    /// `BytesRef`, with the shallow size `RamUsageEstimator.shallowSizeOf`
    /// would measure for it.
    Unknown {
        /// The instance's shallow size.
        shallow_size: i64,
    },
}

impl Debug for SizedObject<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accountable(_) => f.write_str("Accountable"),
            Self::String(s) => f.debug_tuple("String").field(s).finish(),
            Self::BoolArray(arr) => f.debug_tuple("BoolArray").field(&arr.len()).finish(),
            Self::ByteArray(arr) => f.debug_tuple("ByteArray").field(&arr.len()).finish(),
            Self::CharArray(arr) => f.debug_tuple("CharArray").field(&arr.len()).finish(),
            Self::DoubleArray(arr) => f.debug_tuple("DoubleArray").field(&arr.len()).finish(),
            Self::FloatArray(arr) => f.debug_tuple("FloatArray").field(&arr.len()).finish(),
            Self::IntArray(arr) => f.debug_tuple("IntArray").field(&arr.len()).finish(),
            Self::Integer(v) => f.debug_tuple("Integer").field(v).finish(),
            Self::Long(v) => f.debug_tuple("Long").field(v).finish(),
            Self::LongArray(arr) => f.debug_tuple("LongArray").field(&arr.len()).finish(),
            Self::ShortArray(arr) => f.debug_tuple("ShortArray").field(&arr.len()).finish(),
            Self::StringArray(arr) => f.debug_tuple("StringArray").field(&arr.len()).finish(),
            Self::Query(q) => f.debug_tuple("Query").field(q).finish(),
            Self::Map(_) => f.write_str("Map"),
            Self::Collection(_) => f.write_str("Collection"),
            Self::Unknown { shallow_size } => f
                .debug_struct("Unknown")
                .field("shallow_size", shallow_size)
                .finish(),
        }
    }
}

/// A `java.util.Map` as `RamUsageEstimator.sizeOfMap` reads it.
///
/// Java reads the map's shallow size and its entries' by reflection
/// (`shallowSizeOf(map)`, `shallowSizeOf(entry)`); Rust has none, so the map
/// declares both, with its keys and values.
pub trait SizedMap {
    /// `RamUsageEstimator.shallowSizeOf(map)`.
    fn shallow_size(&self) -> i64;

    /// `RamUsageEstimator.shallowSizeOf(entry)` of one `Map.Entry`.
    fn entry_shallow_size(&self) -> i64;

    /// `map.entrySet()`: each key and value, `null` as `None`.
    fn entries(&self) -> Vec<(Option<SizedObject<'_>>, Option<SizedObject<'_>>)>;
}

/// A `java.util.Collection` as `RamUsageEstimator.sizeOfCollection` reads it.
///
/// Java reads the collection's shallow size by reflection
/// (`shallowSizeOf(collection)`); Rust has none, so the collection declares
/// it, with its elements.
pub trait SizedCollection {
    /// `RamUsageEstimator.shallowSizeOf(collection)`.
    fn shallow_size(&self) -> i64;

    /// The elements in iteration order, `null` as `None`; their count is
    /// `collection.size()`.
    fn elements(&self) -> Vec<Option<SizedObject<'_>>>;
}

/// The visitor `RamUsageEstimator.sizeOf(Query, long)` sums a query tree's
/// size with.
///
/// Equivalent to the private `RamUsageEstimator.RamUsageQueryVisitor`
/// (`RamUsageEstimator.java:303-352`). `query == root` is an identity test:
/// the data addresses of the two queries are compared.
struct RamUsageQueryVisitor {
    total: i64,
    def_size: i64,
    root: *const (),
}

impl RamUsageQueryVisitor {
    fn new(root: &dyn crate::search::Query, def_size: i64) -> Self {
        let total = if def_size > 0 {
            def_size
        } else {
            RamUsageEstimator::shallow_size_of_object(root)
        };
        Self {
            total,
            def_size,
            root: root as *const dyn crate::search::Query as *const (),
        }
    }

    fn is_root(&self, query: &dyn crate::search::Query) -> bool {
        std::ptr::eq(
            query as *const dyn crate::search::Query as *const (),
            self.root,
        )
    }
}

impl crate::search::QueryVisitor for RamUsageQueryVisitor {
    fn consume_terms(&mut self, query: &dyn crate::search::Query, terms: &[crate::index::Term]) {
        if !self.is_root(query) {
            if self.def_size > 0 {
                self.total += self.def_size;
            } else {
                self.total += RamUsageEstimator::shallow_size_of_object(query);
            }
        }
        // `if (terms != null)`: a slice is never `null`.
        let accountables: Vec<&dyn Accountable> =
            terms.iter().map(|t| t as &dyn Accountable).collect();
        self.total += RamUsageEstimator::size_of_accountables(&accountables);
    }

    fn visit_leaf(&mut self, query: &dyn crate::search::Query) {
        if self.is_root(query) {
            return;
        }
        if let Some(accountable) = query.as_accountable() {
            self.total += accountable.ram_bytes_used();
        } else if self.def_size > 0 {
            self.total += self.def_size;
        } else {
            self.total += RamUsageEstimator::shallow_size_of_object(query);
        }
    }

    fn get_sub_visitor<'a>(
        &'a mut self,
        _occur: crate::search::Occur,
        _parent: &dyn crate::search::Query,
    ) -> Box<dyn crate::search::QueryVisitor + 'a>
    where
        Self: 'a,
    {
        Box::new(self)
    }
}

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

    /// True, iff compressed references (oops) are enabled by this JVM.
    ///
    /// Equivalent to `RamUsageEstimator.COMPRESSED_REFS_ENABLED`, which Java
    /// reads from the running HotSpot VM at class initialisation. This port's
    /// layout is fixed to a 64-bit HotSpot VM with compressed references (see
    /// the type documentation), so the constant is `true`.
    pub const COMPRESSED_REFS_ENABLED: bool = true;

    /// True, iff the running VM is a 64-bit HotSpot VM; for testing only.
    ///
    /// Equivalent to the package-private
    /// `RamUsageEstimator.JVM_IS_HOTSPOT_64BIT`; `true` for the fixed layout
    /// this port models.
    pub const JVM_IS_HOTSPOT_64BIT: bool = true;

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
    /// reflection, so the same arithmetic is spelled out over the fields of the
    /// JDK 9+ `String` the 10.5.0 jar runs on: the object header, the reference
    /// to the value array, the `byte coder`, the `int hash` and the `boolean
    /// hashIsZero`, aligned — 24 bytes, as measured against
    /// `lucene-core-10.5.0.jar` on a 64-bit JVM with compressed object pointers.
    pub const STRING_SIZE: i64 = Self::align_object_size(
        Self::NUM_BYTES_OBJECT_HEADER + Self::NUM_BYTES_OBJECT_REF + 1 + 4 + 1,
    );

    /// Shallow size of an `Integer` instance.
    ///
    /// Equivalent to the package-private `RamUsageEstimator.INTEGER_SIZE`,
    /// `shallowSizeOfInstance(Integer.class)`: the object header and the `int
    /// value` field, aligned.
    pub const INTEGER_SIZE: i64 = Self::align_object_size(Self::NUM_BYTES_OBJECT_HEADER + 4);

    /// Shallow size of a `Long` instance.
    ///
    /// Equivalent to the package-private `RamUsageEstimator.LONG_SIZE`,
    /// `shallowSizeOfInstance(Long.class)`: the object header and the `long
    /// value` field, aligned.
    pub const LONG_SIZE: i64 = Self::align_object_size(Self::NUM_BYTES_OBJECT_HEADER + 8);

    /// Returns the shallow size of an `Integer`, ignoring the `IntegerCache`.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(Integer)`.
    pub fn size_of_integer(_ignored: i32) -> i64 {
        Self::INTEGER_SIZE
    }

    /// Returns the shallow size of a `Long`, ignoring the `LongCache`.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(Long)`.
    pub fn size_of_long_object(_ignored: i64) -> i64 {
        Self::LONG_SIZE
    }

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

    /// Returns the size in bytes of a `String[]`: its shallow size plus the
    /// size of every non-`null` string (`None` stands for Java's `null`).
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(String[])`.
    pub fn size_of_string_array(arr: &[Option<&str>]) -> i64 {
        let mut size = Self::shallow_size_of(arr);
        for s in arr.iter().flatten() {
            size += Self::size_of_string(s);
        }
        size
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

    /// Returns the size in bytes of a `Map`, including the sizes of its keys
    /// and values, charging [`UNKNOWN_DEFAULT_RAM_BYTES_USED`](Self::UNKNOWN_DEFAULT_RAM_BYTES_USED)
    /// for a type the estimate does not know; recurses up to
    /// [`MAX_DEPTH`](Self::MAX_DEPTH).
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfMap(Map)`.
    pub fn size_of_java_map(map: Option<&dyn SizedMap>) -> i64 {
        Self::size_of_map_at_depth(map, 0, Self::UNKNOWN_DEFAULT_RAM_BYTES_USED)
    }

    /// Returns the size in bytes of a `Map`, charging `def_size` for a type the
    /// estimate does not know.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfMap(Map, long)`.
    pub fn size_of_java_map_with_def_size(map: Option<&dyn SizedMap>, def_size: i64) -> i64 {
        Self::size_of_map_at_depth(map, 0, def_size)
    }

    /// Equivalent to the private `RamUsageEstimator.sizeOfMap(Map, int, long)`
    /// (`RamUsageEstimator.java:249-266`).
    fn size_of_map_at_depth(map: Option<&dyn SizedMap>, depth: i32, def_size: i64) -> i64 {
        let Some(map) = map else {
            return 0;
        };
        let mut size = map.shallow_size();
        if depth > Self::MAX_DEPTH {
            return size;
        }
        let mut size_of_entry = -1;
        for (key, value) in map.entries() {
            if size_of_entry == -1 {
                size_of_entry = map.entry_shallow_size();
            }
            size += size_of_entry;
            size += Self::size_of_object_at_depth(key, depth, def_size);
            size += Self::size_of_object_at_depth(value, depth, def_size);
        }
        Self::align_object_size(size)
    }

    /// Returns the size in bytes of a `Collection`, including the sizes of its
    /// values, charging [`UNKNOWN_DEFAULT_RAM_BYTES_USED`](Self::UNKNOWN_DEFAULT_RAM_BYTES_USED)
    /// for a type the estimate does not know; recurses up to
    /// [`MAX_DEPTH`](Self::MAX_DEPTH).
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfCollection(Collection)`.
    pub fn size_of_java_collection(collection: Option<&dyn SizedCollection>) -> i64 {
        Self::size_of_collection_at_depth(collection, 0, Self::UNKNOWN_DEFAULT_RAM_BYTES_USED)
    }

    /// Returns the size in bytes of a `Collection`, charging `def_size` for a
    /// type the estimate does not know.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfCollection(Collection, long)`.
    pub fn size_of_java_collection_with_def_size(
        collection: Option<&dyn SizedCollection>,
        def_size: i64,
    ) -> i64 {
        Self::size_of_collection_at_depth(collection, 0, def_size)
    }

    /// Equivalent to the private
    /// `RamUsageEstimator.sizeOfCollection(Collection, int, long)`
    /// (`RamUsageEstimator.java:287-301`).
    fn size_of_collection_at_depth(
        collection: Option<&dyn SizedCollection>,
        depth: i32,
        def_size: i64,
    ) -> i64 {
        let Some(collection) = collection else {
            return 0;
        };
        let mut size = collection.shallow_size();
        if depth > Self::MAX_DEPTH {
            return size;
        }
        let elements = collection.elements();
        // assume array-backed collection and add per-object references
        size += Self::NUM_BYTES_ARRAY_HEADER + elements.len() as i64 * Self::NUM_BYTES_OBJECT_REF;
        for o in elements {
            size += Self::size_of_object_at_depth(o, depth, def_size);
        }
        Self::align_object_size(size)
    }

    /// Returns the size in bytes of a query; an unknown query type is
    /// estimated as [`QUERY_DEFAULT_RAM_BYTES_USED`](Self::QUERY_DEFAULT_RAM_BYTES_USED).
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(Query)`.
    pub fn size_of_query(q: &dyn crate::search::Query) -> i64 {
        Self::size_of_query_with_def_size(q, Self::QUERY_DEFAULT_RAM_BYTES_USED)
    }

    /// Returns the size in bytes of a query; an unknown query type is
    /// estimated with its shallow size, or `def_size` when that is positive.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOf(Query, long)`
    /// (`RamUsageEstimator.java:367-375`); `q instanceof Accountable` is
    /// [`Query::as_accountable`](crate::search::Query::as_accountable).
    pub fn size_of_query_with_def_size(q: &dyn crate::search::Query, def_size: i64) -> i64 {
        if let Some(accountable) = q.as_accountable() {
            accountable.ram_bytes_used()
        } else {
            let mut visitor = RamUsageQueryVisitor::new(q, def_size);
            q.visit(&mut visitor);
            Self::align_object_size(visitor.total)
        }
    }

    /// Returns the shallow size of `obj`.
    ///
    /// Equivalent to `RamUsageEstimator.shallowSizeOf(Object)` over an object
    /// that is not an array, `shallowSizeOfInstance(obj.getClass())`; see the
    /// divergence note on [`shallow_size_of_instance`](Self::shallow_size_of_instance),
    /// whose measure this applies to the value's dynamic type.
    pub fn shallow_size_of_object<T: ?Sized>(obj: &T) -> i64 {
        Self::align_object_size(Self::NUM_BYTES_OBJECT_HEADER + size_of_val(obj) as i64)
    }

    /// Returns the shallow size of an instance of a Java class whose non-static
    /// fields, its own and its superclasses', have the given widths.
    ///
    /// Equivalent to `RamUsageEstimator.shallowSizeOfInstance(Class)` over a
    /// conventional class: the object header plus
    /// [`adjust_for_field`](Self::adjust_for_field) of every field, aligned.
    /// Java reads the fields by reflection; the caller declares them here, a
    /// reference field as [`NUM_BYTES_OBJECT_REF`](Self::NUM_BYTES_OBJECT_REF).
    pub fn shallow_size_of_declared_instance(field_sizes: &[i64]) -> i64 {
        let mut size = Self::NUM_BYTES_OBJECT_HEADER;
        for field_size in field_sizes {
            size = Self::adjust_for_field(size, *field_size);
        }
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
    /// Equivalent to `RamUsageEstimator.sizeOfObject(Object)`
    /// (`RamUsageEstimator.java:382-384`), which is
    /// `sizeOfObject(o, 0, UNKNOWN_DEFAULT_RAM_BYTES_USED)`: see
    /// [`size_of_object_with_def_size`](Self::size_of_object_with_def_size).
    pub fn size_of_object(o: Option<SizedObject<'_>>) -> i64 {
        Self::size_of_object_with_def_size(o, Self::UNKNOWN_DEFAULT_RAM_BYTES_USED)
    }

    /// Best-effort estimate of the memory an object of undetermined type
    /// costs, charging `def_size` for a class the estimate does not know.
    ///
    /// Equivalent to `RamUsageEstimator.sizeOfObject(Object, long)`
    /// (`RamUsageEstimator.java:392-394`) and the private
    /// `sizeOfObject(Object, int, long)` it calls (`:396-432`): `null` costs
    /// nothing; an `Accountable` reports its own `ramBytesUsed()`, whatever it
    /// is; a `String` and an `int[]` are sized by their formulas; any other
    /// class costs `defSize` when that is positive, and its shallow size
    /// otherwise.
    ///
    /// Java dispatches on the runtime class with a chain of `instanceof`
    /// tests. Rust has no runtime class to test, so the caller names it with
    /// a [`SizedObject`] variant, one per case of that chain; the branch taken
    /// for each is Java's.
    pub fn size_of_object_with_def_size(o: Option<SizedObject<'_>>, def_size: i64) -> i64 {
        Self::size_of_object_at_depth(o, 0, def_size)
    }

    /// Equivalent to the private `RamUsageEstimator.sizeOfObject(Object, int,
    /// long)` (`RamUsageEstimator.java:396-441`).
    fn size_of_object_at_depth(o: Option<SizedObject<'_>>, depth: i32, def_size: i64) -> i64 {
        Self::do_privileged(|| match o {
            None => 0,
            Some(SizedObject::Accountable(accountable)) => accountable.ram_bytes_used(),
            Some(SizedObject::String(s)) => Self::size_of_string(s),
            Some(SizedObject::BoolArray(arr)) => Self::size_of_bool(arr),
            Some(SizedObject::ByteArray(arr)) => Self::size_of(arr),
            Some(SizedObject::CharArray(arr)) => Self::size_of_char(arr),
            Some(SizedObject::DoubleArray(arr)) => Self::size_of_double(arr),
            Some(SizedObject::FloatArray(arr)) => Self::size_of_float(arr),
            Some(SizedObject::IntArray(arr)) => Self::size_of_int(arr),
            Some(SizedObject::Integer(v)) => Self::size_of_integer(v),
            Some(SizedObject::Long(v)) => Self::size_of_long_object(v),
            Some(SizedObject::LongArray(arr)) => Self::size_of_long(arr),
            Some(SizedObject::ShortArray(arr)) => Self::size_of_short(arr),
            Some(SizedObject::StringArray(arr)) => Self::size_of_string_array(arr),
            Some(SizedObject::Query(q)) => Self::size_of_query_with_def_size(q, def_size),
            Some(SizedObject::Map(map)) => {
                Self::size_of_map_at_depth(Some(map), depth + 1, def_size)
            }
            Some(SizedObject::Collection(collection)) => {
                Self::size_of_collection_at_depth(Some(collection), depth + 1, def_size)
            }
            Some(SizedObject::Unknown { shallow_size }) => {
                if def_size > 0 {
                    def_size
                } else {
                    shallow_size
                }
            }
        })
    }

    /// Renders `bytes` in human-readable units — GB, MB, KB, or bytes.
    ///
    /// Equivalent to `RamUsageEstimator.humanReadableUnits(long)`
    /// (`RamUsageEstimator.java:600-617`), which divides as a `float` and
    /// formats with `new DecimalFormat("0.#")` under `Locale.ROOT`: at most one
    /// fractional digit, none when it is zero, and `DecimalFormat`'s default
    /// `RoundingMode.HALF_EVEN` on the exact value (1280 bytes, exactly 1.25 KB,
    /// renders `1.2 KB`).
    pub fn human_readable_units(bytes: i64) -> String {
        /// `DecimalFormat("0.#").format((double) value)`. Rust's fixed-precision
        /// float formatting rounds the exact binary value half to even, as
        /// `DecimalFormat` does since JDK 8.
        fn format_0_hash(value: f32) -> String {
            let formatted = format!("{:.1}", f64::from(value));
            match formatted.strip_suffix(".0") {
                Some(integral) => integral.to_string(),
                None => formatted,
            }
        }
        if bytes / Self::ONE_GB > 0 {
            format!("{} GB", format_0_hash(bytes as f32 / Self::ONE_GB as f32))
        } else if bytes / Self::ONE_MB > 0 {
            format!("{} MB", format_0_hash(bytes as f32 / Self::ONE_MB as f32))
        } else if bytes / Self::ONE_KB > 0 {
            format!("{} KB", format_0_hash(bytes as f32 / Self::ONE_KB as f32))
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
            "os.arch" => Some(
                Self::jvm_os_arch(
                    env::consts::ARCH,
                    env::consts::OS,
                    cfg!(target_endian = "little"),
                    cfg!(target_pointer_width = "32"),
                )
                .to_string(),
            ),
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

    /// Operating-system architecture: the `os.arch` system property, or
    /// `Unknown`.
    ///
    /// Equivalent to `Constants.OS_ARCH` (`Constants.java:57`). The platform
    /// value is the JVM's name for the architecture (see
    /// [`jvm_os_arch`](Self::jvm_os_arch)), so the comparisons Lucene makes
    /// against `"amd64"` and `"aarch64"` hold as they do in Java.
    pub fn os_arch() -> String {
        Self::get_sys_prop_or("os.arch", Self::UNKNOWN)
    }

    /// The JVM's `os.arch` for a Rust target: `arch` is
    /// `std::env::consts::ARCH`, `os` is `std::env::consts::OS`.
    ///
    /// JDK 25 sets `os.arch` from its build CPU name
    /// (`make/autoconf/platform.m4`, `PLATFORM_EXTRACT_VARS_FROM_CPU` and
    /// `OPENJDK_$1_CPU_OSARCH`): the CPU name is the GNU one, with
    /// `powerpc*` renamed `ppc*`, `sparc64` renamed `sparcv9` and the
    /// little-endian MIPS and PowerPC 64 variants suffixed `el`/`le`; then
    /// `x86` is renamed `i386` on Linux and `x86_64` is renamed `amd64` on
    /// every OS but macOS (and is `x32` under the 32-bit x86-64 ABI). Rust
    /// names the remaining architectures as the JVM does.
    pub fn jvm_os_arch(
        arch: &'static str,
        os: &str,
        little_endian: bool,
        pointer_width_32: bool,
    ) -> &'static str {
        match arch {
            "x86" if os == "linux" => "i386",
            "x86_64" if pointer_width_32 => "x32",
            "x86_64" if os != "macos" => "amd64",
            "mips" if little_endian => "mipsel",
            "mips64" if little_endian => "mips64el",
            "powerpc" => "ppc",
            "powerpc64" if little_endian => "ppc64le",
            "powerpc64" => "ppc64",
            "sparc64" => "sparcv9",
            other => other,
        }
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

    /// Runtime version, as `IndexWriter` records it in a segment's
    /// diagnostics.
    ///
    /// Equivalent to `Runtime.version().toString()`, which the JVM also
    /// publishes as the `java.runtime.version` system property; it is read
    /// from that property, as every other runtime constant here is, and is
    /// [`UNKNOWN`](Self::UNKNOWN) when it is not set.
    pub fn runtime_version() -> String {
        Self::get_sys_prop_or("java.runtime.version", Self::UNKNOWN)
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
                if Self::os_arch() == "amd64" && !Self::has_sse4a() {
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
                if Self::os_arch() == "amd64" && !Self::has_sse4a() {
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
        if Self::os_arch() == "amd64" && Self::has_avx2() {
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
        debug_assert!(
            false,
            "message() should not be called when isEnabled returns false"
        );
    }

    fn is_enabled(&self, _component: &str) -> bool {
        false
    }

    fn close(&self) {
        // Nothing to close.
    }
}

/// The default info stream, `InfoStream.defaultInfoStream` (`InfoStream.java:57`).
static DEFAULT_INFO_STREAM: LazyLock<std::sync::RwLock<std::sync::Arc<dyn InfoStream>>> =
    LazyLock::new(|| std::sync::RwLock::new(std::sync::Arc::new(NoOutputInfoStream)));

/// Returns the info stream that newly created classes use by default.
///
/// Equivalent to `InfoStream.getDefault()` (`InfoStream.java:64-66`), whose
/// `synchronized` read is the read lock here. Lucene reads it in
/// `LiveIndexWriterConfig` (`LiveIndexWriterConfig.java:135`),
/// `HnswGraphBuilder` (`HnswGraphBuilder.java:85`) and
/// `HnswConcurrentMergeBuilder` (`HnswConcurrentMergeBuilder.java:48`).
pub fn default_info_stream() -> std::sync::Arc<dyn InfoStream> {
    std::sync::Arc::clone(
        &DEFAULT_INFO_STREAM
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// Sets the info stream that newly created classes use by default.
///
/// Equivalent to `InfoStream.setDefault(InfoStream)` (`InfoStream.java:74-81`).
/// Java rejects `null` with an `IllegalArgumentException`; an
/// `std::sync::Arc<dyn InfoStream>` cannot be null, so that check has no counterpart.
pub fn set_default_info_stream(info_stream: std::sync::Arc<dyn InfoStream>) {
    *DEFAULT_INFO_STREAM
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = info_stream;
}

// ---------------------------------------------------------------------------
// Bits / FixedBitSet
// ---------------------------------------------------------------------------

/// Bitset-like interface.
pub trait Bits: Send + Sync + Debug {
    /// Returns the value of the bit at `index`.
    fn get(&self, index: usize) -> bool;

    /// Answers `this instanceof BitSet` with the cast: `Some(self)` for a
    /// [`BitSet`](crate::util::bit_set::BitSet) implementation, `None` (the
    /// default) for every other `Bits`.
    ///
    /// **This has no counterpart in Lucene 10.5.0.** Java tests the runtime
    /// class of any object; a Rust trait object cannot be narrowed to another
    /// trait, so each `BitSet` answers with itself. Needed by
    /// `HnswGraphSearcher.search(RandomVectorScorer, KnnCollector, HnswGraph,
    /// Bits)` (`HnswGraphSearcher.java:95-105`).
    fn as_bit_set(&self) -> Option<&dyn crate::util::bit_set::BitSet> {
        None
    }

    /// Returns the value of the bit at `index`, or the error a wrapper raises
    /// while answering.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `Bits.get(int)` declares no
    /// exception, yet `ExitableDirectoryReader.ExitableAcceptDocs.bits()`
    /// returns a `Bits` whose `get` throws the unchecked
    /// `ExitingReaderException` on a sampled call
    /// (`ExitableDirectoryReader.java:359-387, 439-445`), aborting the
    /// nearest-vector search that consults it. Rust reports errors through
    /// `Result`, and making `get` fallible would reach every caller of `Bits`;
    /// this companion is the minimal surface that keeps Java's timing. The
    /// nearest-vector searches call `try_get`; every implementation whose
    /// `get` delegates to another `Bits` forwards `try_get` to it, as Java's
    /// exception propagates through any wrapper that delegates `get`; leaf
    /// implementations answer `Ok(self.get(index))`.
    ///
    /// # Errors
    ///
    /// The default never fails; an overriding wrapper may.
    fn try_get(&self, index: usize) -> crate::error::Result<bool> {
        Ok(self.get(index))
    }

    /// Returns the number of bits in this set.
    fn length(&self) -> usize;

    /// Clears the bits of `bit_set` whose bit, shifted by `offset`, is clear in
    /// these bits: bit `i` of `bit_set` stands for bit `offset + i` here.
    ///
    /// Equivalent to the default `Bits.applyMask(FixedBitSet, int)`
    /// (`Bits.java:55-63`), which walks the set bits of `bit_set` and clears
    /// those whose bit is clear. [`FixedBitSet`] and
    /// [`FixedBits`](crate::util::bit_set::FixedBits) override it with a
    /// word-at-a-time intersection, as in Java (`FixedBitSet.java:897-909`,
    /// `FixedBits.java:33-36`).
    ///
    /// The default reads through [`try_get`](Self::try_get), so an
    /// implementation whose `get` raises an error — Java's unchecked exception
    /// from a `get` such as `ExitableDirectoryReader`'s — fails the call as
    /// Java's default does.
    ///
    /// # Errors
    ///
    /// Returns the error [`try_get`](Self::try_get) raises, or, for the
    /// [`FixedBitSet`] override, the `IllegalArgumentException` Java throws
    /// when `bit_set` has bits set beyond the end of these bits.
    fn apply_mask(&self, bit_set: &mut FixedBitSet, offset: usize) -> Result<(), LuceneError> {
        let mut i = BitSet::next_set_bit(bit_set, 0);
        while i != crate::search::NO_MORE_DOCS {
            let index = i as usize;
            if !self.try_get(offset + index)? {
                bit_set.clear(index);
            }
            i = if index + 1 >= bit_set.length() {
                crate::search::NO_MORE_DOCS
            } else {
                BitSet::next_set_bit(bit_set, (index + 1) as i32)
            };
        }
        Ok(())
    }

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

    /// Returns these bits as [`std::any::Any`], so that a caller can test
    /// them for a concrete type.
    ///
    /// This hook stands for Java's `instanceof` on a `Bits`:
    /// `FixedBitSet.copyOf(Bits)` restores the `FixedBitSet` behind a
    /// `FixedBits` and clones a `FixedBitSet` (`FixedBitSet.java:866-873`).
    /// Rust cannot narrow a `dyn Bits`, so the question is asked of the bits
    /// themselves, in the shape of
    /// [`PostingsEnum::as_any_mut`](crate::index::PostingsEnum::as_any_mut).
    /// The default answers `None`, the `false` branch of the `instanceof`;
    /// the types it tests for override it, and a sharing handle Java does not
    /// have forwards it to the bits it shares.
    fn bits_as_any(&self) -> Option<&dyn std::any::Any> {
        None
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

/// The words of a [`FixedBitSet`]: Java's `long[] bits`, shared by reference.
///
/// Java's `FixedBitSet.asReadOnlyBits()` (`FixedBitSet.java:893-895`) wraps
/// the very `long[]` the set writes, so a later write to the set shows through
/// the view (`PendingDeletes.java:78` relies on it). Rust can only share
/// mutable words behind interior mutability, so the words are atomics behind
/// an `Arc`. Every access is a `Relaxed` load or store: a `FixedBitSet` has one
/// writer, the holder of `&mut`, and Java's plain `long[]` accesses carry no
/// ordering either — a view handed to another thread is published by whatever
/// hands it over.
///
/// A single `Relaxed` load or store of an `AtomicU64` compiles to the plain
/// load or store a `long[]` access does, but a *loop* of them does not compile
/// as a loop over a `long[]`: the compiler neither vectorises nor merges atomic
/// accesses, so bulk loops over the words cost more than the same loops over a
/// `Vec<u64>`. `cardinality`, `intersection_count` and the word-aligned `or`
/// therefore load [`WORD_CHUNK`] words into a local array before computing on
/// them, which lets the computation vectorise. Measured on an AMD Ryzen 9
/// 5900HX (release profile, 2^16 to 2^24 bits, median of nine): against the
/// plain loops, `cardinality` went from 2.6–2.7× to 1.1×, `intersectionCount`
/// from 2.6–2.7× to 1.4× and `or` from 1.9–2.8× to 1.6–1.9×. `clear_all` is a
/// run of stores, which nothing merges; it keeps its plain loop and costs
/// 1.6–2.0×.
type Words = Arc<[AtomicU64]>;

/// Allocates `len` zeroed words, Java's `new long[len]`.
fn new_words(len: usize) -> Words {
    (0..len).map(|_| AtomicU64::new(0)).collect()
}

/// How many words the bulk loops of [`FixedBitSet`] load before they compute on
/// them; see [`Words`].
const WORD_CHUNK: usize = 8;

/// Loads one chunk of [`WORD_CHUNK`] words, each with a `Relaxed` load.
#[inline]
fn load_word_chunk(words: &[AtomicU64; WORD_CHUNK]) -> [u64; WORD_CHUNK] {
    let mut chunk = [0u64; WORD_CHUNK];
    for (value, word) in chunk.iter_mut().zip(words) {
        *value = word.load(AtomicOrdering::Relaxed);
    }
    chunk
}

/// A fixed-size bit set, equivalent to Lucene's `FixedBitSet`.
///
/// The words are shared, as Java's `long[]` is, with the
/// [`FixedBits`](crate::util::bit_set::FixedBits) views
/// [`as_read_only_bits`](Self::as_read_only_bits) returns: they are atomics
/// behind an `Arc`, each read and written with a `Relaxed` load or store.
/// [`Clone`] copies them, as `FixedBitSet.clone()` does
/// (`FixedBitSet.java:829-834`).
pub struct FixedBitSet {
    bits: Words,
    num_bits: usize,
    num_words: usize,
}

impl Debug for FixedBitSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let words: Vec<u64> = (0..self.bits.len()).map(|i| self.word(i)).collect();
        f.debug_struct("FixedBitSet")
            .field("bits", &words)
            .field("num_bits", &self.num_bits)
            .field("num_words", &self.num_words)
            .finish()
    }
}

impl Clone for FixedBitSet {
    /// Equivalent to `FixedBitSet.clone()` (`FixedBitSet.java:829-834`): a new
    /// array of the same length holding a copy of the first `numWords` words.
    fn clone(&self) -> Self {
        let bits = new_words(self.bits.len());
        for i in 0..self.num_words {
            bits[i].store(self.word(i), AtomicOrdering::Relaxed);
        }
        Self {
            bits,
            num_bits: self.num_bits,
            num_words: self.num_words,
        }
    }
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
            bits: new_words(num_words),
            num_bits,
            num_words,
        }
    }

    /// Creates a `FixedBitSet` from an existing `u64` buffer.
    ///
    /// Equivalent to `FixedBitSet(long[], int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] with Java's message — `The
    /// given long array is too small  to hold <numBits> bits`, two spaces
    /// included (`FixedBitSet.java:162-167`) — when `stored_bits` holds fewer
    /// than `bits2words(num_bits)` words.
    pub fn from_bits(stored_bits: Vec<u64>, num_bits: usize) -> Result<Self, LuceneError> {
        Self::from_words(
            stored_bits.into_iter().map(AtomicU64::new).collect(),
            num_bits,
        )
    }

    /// `FixedBitSet(long[], int)` over words that are already shared.
    fn from_words(stored_bits: Words, num_bits: usize) -> Result<Self, LuceneError> {
        let num_words = Self::bits2words(num_bits);
        if num_words > stored_bits.len() {
            return Err(LuceneError::illegal_argument(format!(
                "The given long array is too small  to hold {num_bits} bits"
            )));
        }
        let bit_set = Self {
            bits: stored_bits,
            num_bits,
            num_words,
        };
        // `assert verifyGhostBitsClear();` (`FixedBitSet.java:171`).
        debug_assert!(bit_set.verify_ghost_bits_clear());
        Ok(bit_set)
    }

    /// Reads word `i`, Java's `bits[i]`.
    #[inline]
    fn word(&self, i: usize) -> u64 {
        self.bits[i].load(AtomicOrdering::Relaxed)
    }

    /// Writes word `i`, Java's `bits[i] = value`.
    #[inline]
    fn set_word(&mut self, i: usize, value: u64) {
        self.bits[i].store(value, AtomicOrdering::Relaxed);
    }

    /// Java's `bits[i] |= mask`: a plain read-modify-write, as in Java.
    #[inline]
    fn or_word(&mut self, i: usize, mask: u64) {
        let value = self.word(i) | mask;
        self.set_word(i, value);
    }

    /// Java's `bits[i] &= mask`.
    #[inline]
    fn and_word(&mut self, i: usize, mask: u64) {
        let value = self.word(i) & mask;
        self.set_word(i, value);
    }

    /// Java's `bits[i] ^= mask`.
    #[inline]
    fn xor_word(&mut self, i: usize, mask: u64) {
        let value = self.word(i) ^ mask;
        self.set_word(i, value);
    }

    /// Java's `Arrays.fill(bits, from, to, value)`.
    fn fill_words(&mut self, from: usize, to: usize, value: u64) {
        for i in from..to {
            self.set_word(i, value);
        }
    }

    /// Clears all bits.
    pub fn clear_all(&mut self) {
        let len = self.bits.len();
        self.fill_words(0, len, 0);
    }

    /// Sets the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn set(&mut self, index: usize) {
        assert!(
            index < self.num_bits,
            "index={index}, numBits={}",
            self.num_bits
        );
        let word = index >> 6;
        let mask = 1u64 << (index & 0x3f);
        self.or_word(word, mask);
    }

    /// Clears the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn clear(&mut self, index: usize) {
        assert!(
            index < self.num_bits,
            "index={index}, numBits={}",
            self.num_bits
        );
        let word = index >> 6;
        let mask = 1u64 << (index & 0x3f);
        self.and_word(word, !mask);
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
        (self.word(word) & mask) != 0
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
        let previous = (self.word(word) & mask) != 0;
        self.or_word(word, mask);
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
        let previous = (self.word(word) & mask) != 0;
        self.and_word(word, !mask);
        previous
    }

    /// Sets all bits in the range `[from, to)`.
    ///
    /// Equivalent to `FixedBitSet.set(long, long)`.
    ///
    /// Java only asserts the bounds (`FixedBitSet.java:776-780`) and does
    /// nothing for an empty or reversed range; so does this port.
    pub fn set_range(&mut self, from: usize, to: usize) {
        debug_assert!(
            from < self.num_bits,
            "startIndex={from}, numBits={}",
            self.num_bits
        );
        debug_assert!(
            to <= self.num_bits,
            "endIndex={to}, numBits={}",
            self.num_bits
        );
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
            self.or_word(start_word, startmask & endmask);
            return;
        }

        self.or_word(start_word, startmask);
        self.fill_words(start_word + 1, end_word, u64::MAX);
        self.or_word(end_word, endmask);
    }

    /// Creates a `FixedBitSet` that is a copy of the given `Bits`.
    ///
    /// Equivalent to `FixedBitSet.copyOf(Bits)` (`FixedBitSet.java:865-883`):
    /// a `FixedBits` gives back the `FixedBitSet` it wraps, a `FixedBitSet` is
    /// cloned, and any other `Bits` is copied by setting every bit and
    /// clearing the ones it does not hold.
    pub fn copy_of(bits: &dyn Bits) -> Self {
        let any = bits.bits_as_any();
        // Restore the original FixedBitSet.
        let fixed = any
            .and_then(|any| any.downcast_ref::<crate::util::bit_set::FixedBits>())
            .map(crate::util::bit_set::FixedBits::bit_set)
            .or_else(|| any.and_then(|any| any.downcast_ref::<FixedBitSet>()));
        if let Some(fixed) = fixed {
            return fixed.clone();
        }
        let length = bits.length();
        let mut bit_set = Self::new(length);
        bit_set.set_range(0, length);
        for i in 0..length {
            if !bits.get(i) {
                bit_set.clear(i);
            }
        }
        bit_set
    }

    /// Returns the number of set bits.
    ///
    /// Equivalent to `FixedBitSet.cardinality()` (`FixedBitSet.java:217-224`),
    /// `Long.bitCount` summed over the words, which are loaded a chunk at a time
    /// (see [`Words`]).
    pub fn cardinality(&self) -> usize {
        // Depends on the ghost bits being clear!
        let (chunks, rest) = self.bits[..self.num_words].as_chunks::<WORD_CHUNK>();
        let mut total = 0usize;
        for chunk in chunks {
            total += load_word_chunk(chunk)
                .iter()
                .map(|word| word.count_ones() as usize)
                .sum::<usize>();
        }
        total
            + rest
                .iter()
                .map(|word| word.load(AtomicOrdering::Relaxed).count_ones() as usize)
                .sum::<usize>()
    }

    /// Returns the number of bits in the set.
    pub fn length(&self) -> usize {
        self.num_bits
    }

    /// Returns the backing words.
    ///
    /// Equivalent to `FixedBitSet.getBits()` (`FixedBitSet.java:207-210`),
    /// "Expert": the array itself, shared with this set and its read-only
    /// views, not a copy. Each word is read with a `Relaxed` load, Java's plain
    /// `bits[i]`.
    pub fn get_bits(&self) -> &[AtomicU64] {
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
            if num_words >= bits.bits.len() {
                let target = ArrayUtil::oversize(num_words + 1, 8).max(num_words + 1);
                let arr = new_words(target);
                if preserve_data {
                    for i in 0..bits.bits.len() {
                        arr[i].store(bits.word(i), AtomicOrdering::Relaxed);
                    }
                }
                bits.bits = arr;
            }
            // Java keeps an array that is already long enough as it is, set
            // bits included, whether or not `preserveData` is set
            // (`FixedBitSet.java:74-83`).
            let num_bits = bits.bits.len() << 6;
            Self::from_words(bits.bits, num_bits)
                .expect("INVARIANT: arr.len() << 6 bits need exactly arr.len() words")
        }
    }

    /// Returns the cardinality of the intersection of two sets, modifying neither.
    ///
    /// Equivalent to `FixedBitSet.intersectionCount(FixedBitSet, FixedBitSet)`.
    ///
    /// The words are loaded a chunk at a time (see [`Words`]).
    pub fn intersection_count(a: &FixedBitSet, b: &FixedBitSet) -> u64 {
        // Depends on the ghost bits being clear!
        let common = a.num_words.min(b.num_words);
        let (a_chunks, a_rest) = a.bits[..common].as_chunks::<WORD_CHUNK>();
        let (b_chunks, b_rest) = b.bits[..common].as_chunks::<WORD_CHUNK>();
        let mut total = 0u64;
        for (a_chunk, b_chunk) in a_chunks.iter().zip(b_chunks) {
            let (a_words, b_words) = (load_word_chunk(a_chunk), load_word_chunk(b_chunk));
            total += a_words
                .iter()
                .zip(&b_words)
                .map(|(a_word, b_word)| (a_word & b_word).count_ones() as u64)
                .sum::<u64>();
        }
        total
            + a_rest
                .iter()
                .zip(b_rest)
                .map(|(a_word, b_word)| {
                    (a_word.load(AtomicOrdering::Relaxed) & b_word.load(AtomicOrdering::Relaxed))
                        .count_ones() as u64
                })
                .sum::<u64>()
    }

    /// Returns the cardinality of the union of two sets, modifying neither.
    ///
    /// Equivalent to `FixedBitSet.unionCount(FixedBitSet, FixedBitSet)`.
    pub fn union_count(a: &FixedBitSet, b: &FixedBitSet) -> u64 {
        // Depends on the ghost bits being clear!
        let common = a.num_words.min(b.num_words);
        let mut tot: u64 = (0..common)
            .map(|i| (a.word(i) | b.word(i)).count_ones() as u64)
            .sum();
        tot += (common..a.num_words)
            .map(|i| a.word(i).count_ones() as u64)
            .sum::<u64>();
        tot += (common..b.num_words)
            .map(|i| b.word(i).count_ones() as u64)
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
            .map(|i| (a.word(i) & !b.word(i)).count_ones() as u64)
            .sum();
        tot += (common..a.num_words)
            .map(|i| a.word(i).count_ones() as u64)
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
            if self.word(i) != 0 {
                return false;
            }
        }
        if (self.num_bits & 0x3f) == 0 {
            return true;
        }
        let mask = u64::MAX << (self.num_bits & 63);
        (self.word(self.num_words - 1) & mask) == 0
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
            let mut bits = self.word(from >> 6) >> (from & 63);
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
            cardinality += self.word(i).count_ones() as usize;
        }

        // Now handle the bits between the last complete word and `to`.
        if (to & 0x3f) != 0 {
            let bits = self.word(to >> 6) << (to.wrapping_neg() & 63);
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
                pop_count += self.word(max_word + i).count_ones() as u64;
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
            self.or_word(word_index, mask << bit_offset);
        } else {
            self.or_word(word_index, mask << bit_offset);
            self.or_word(word_index + 1, mask >> ((64 - bit_offset) & 63));
        }
    }

    /// Reads `num_bits` (between 1 and 63) bits from `bit_set` at `from`.
    ///
    /// Equivalent to the private `FixedBitSet.readNBits(long[], int, int)`.
    fn read_n_bits(bit_set: &FixedBitSet, from: usize, num_bits: usize) -> u64 {
        debug_assert!(num_bits > 0 && num_bits < 64);
        let mut bits = bit_set.word(from >> 6) >> (from & 63);
        let num_bits_so_far = 64 - (from & 0x3f);
        if num_bits_so_far < num_bits {
            bits |= bit_set.word((from >> 6) + 1) << (from.wrapping_neg() & 63);
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
            let bits = Self::read_n_bits(source, source_from, num_bits_needed) << (dest_from & 63);
            dest.or_word(dest_from >> 6, bits);

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
            if Arc::ptr_eq(&source.bits, &dest.bits) && source_word_from != dest_word_from {
                // The two ranges are one array (a set and a view of it) at
                // different offsets: a word written may be one read later, so
                // the words are ORed one by one, in Java's order.
                for i in 0..num_full_words {
                    dest.or_word(dest_word_from + i, source.word(source_word_from + i));
                }
            } else {
                Self::or_disjoint_words(
                    &source.bits[source_word_from..source_word_from + num_full_words],
                    &dest.bits[dest_word_from..dest_word_from + num_full_words],
                );
            }
        } else {
            let shift = source_from & 63;
            let back = source_from.wrapping_neg() & 63;
            for i in 0..num_full_words {
                dest.or_word(
                    dest_word_from + i,
                    (source.word(source_word_from + i) >> shift)
                        | (source.word(source_word_from + i + 1) << back),
                );
            }
        }

        source_from += num_full_words << 6;
        dest_from += num_full_words << 6;
        length -= num_full_words << 6;

        // Finally handle the tail bits.
        if length > 0 {
            let bits = Self::read_n_bits(source, source_from, length);
            dest.or_word(dest_from >> 6, bits);
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
                Self::read_n_bits(source, source_from, num_bits_needed) << (dest_from & 63);
            bits |= !(((1u64 << (num_bits_needed & 63)) - 1) << (dest_from & 63));
            dest.and_word(dest_from >> 6, bits);

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
                dest.and_word(dest_word_from + i, source.word(source_word_from + i));
            }
        } else {
            let shift = source_from & 63;
            let back = source_from.wrapping_neg() & 63;
            for i in 0..num_full_words {
                dest.and_word(
                    dest_word_from + i,
                    (source.word(source_word_from + i) >> shift)
                        | (source.word(source_word_from + i + 1) << back),
                );
            }
        }

        source_from += num_full_words << 6;
        dest_from += num_full_words << 6;
        length -= num_full_words << 6;

        // Finally handle the tail bits.
        if length > 0 {
            let mut bits = Self::read_n_bits(source, source_from, length);
            bits |= u64::MAX << (length & 63);
            dest.and_word(dest_from >> 6, bits);
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

    /// `dest[i] |= source[i]` for every word, loading a chunk of each at a time
    /// (see [`Words`]); `source` and `dest` are the same words or do not
    /// overlap, so no word written is read afterwards.
    ///
    /// The word-aligned loop of `FixedBitSet.orRange`
    /// (`FixedBitSet.java:515-519`).
    fn or_disjoint_words(source: &[AtomicU64], dest: &[AtomicU64]) {
        debug_assert_eq!(source.len(), dest.len());
        let (source_chunks, source_rest) = source.as_chunks::<WORD_CHUNK>();
        let (dest_chunks, dest_rest) = dest.as_chunks::<WORD_CHUNK>();
        for (source_chunk, dest_chunk) in source_chunks.iter().zip(dest_chunks) {
            let mut words = load_word_chunk(dest_chunk);
            for (word, source_word) in words.iter_mut().zip(load_word_chunk(source_chunk)) {
                *word |= source_word;
            }
            for (dest_word, word) in dest_chunk.iter().zip(words) {
                dest_word.store(word, AtomicOrdering::Relaxed);
            }
        }
        for (source_word, dest_word) in source_rest.iter().zip(dest_rest) {
            let word =
                dest_word.load(AtomicOrdering::Relaxed) | source_word.load(AtomicOrdering::Relaxed);
            dest_word.store(word, AtomicOrdering::Relaxed);
        }
    }

    /// `self = self XOR other`.
    ///
    /// Equivalent to `FixedBitSet.xor(FixedBitSet)`.
    pub fn xor(&mut self, other: &FixedBitSet) {
        self.xor_bits(&other.bits, other.num_words);
    }

    /// Equivalent to the private `FixedBitSet.xor(long[], int)`.
    fn xor_bits(&mut self, other_bits: &[AtomicU64], other_num_words: usize) {
        debug_assert!(
            other_num_words <= self.num_words,
            "numWords={}, other.numWords={other_num_words}",
            self.num_words
        );
        let mut pos = self.num_words.min(other_num_words);
        while pos > 0 {
            pos -= 1;
            self.xor_word(pos, other_bits[pos].load(AtomicOrdering::Relaxed));
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
            if (self.word(pos) & other.word(pos)) != 0 {
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
    fn and_bits(&mut self, other_arr: &[AtomicU64], other_num_words: usize) {
        let mut pos = self.num_words.min(other_num_words);
        while pos > 0 {
            pos -= 1;
            self.and_word(pos, other_arr[pos].load(AtomicOrdering::Relaxed));
        }
        if self.num_words > other_num_words {
            let num_words = self.num_words;
            self.fill_words(other_num_words, num_words, 0);
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
        other_arr: &[AtomicU64],
        other_num_words: usize,
    ) {
        let mut pos = self
            .num_words
            .saturating_sub(other_offset_words)
            .min(other_num_words);
        while pos > 0 {
            pos -= 1;
            self.and_word(
                pos + other_offset_words,
                !other_arr[pos].load(AtomicOrdering::Relaxed),
            );
        }
    }

    /// Does an in-place XOR of the bits `iter` provides.
    ///
    /// Equivalent to `FixedBitSet.xor(DocIdSetIterator)` (`FixedBitSet.java:616-627`):
    /// an iterator over a `FixedBitSet` is XORed word by word, any other one
    /// flips each doc it returns below `length()`. The suffix stands for
    /// Java's overload, as in [`or_set`](Self::or_set).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] when the iterator is already
    /// positioned, and propagates iteration errors.
    pub fn xor_iterator(
        &mut self,
        iter: &mut dyn crate::search::DocIdSetIterator,
    ) -> crate::error::Result<()> {
        crate::util::bit_set::check_unpositioned(iter)?;
        if let Some(bits) = crate::util::bit_set::BitSetIterator::get_fixed_bit_set_or_null(iter) {
            self.xor(bits);
        } else {
            loop {
                let doc = iter.next_doc()?;
                if doc >= self.num_bits as i32 {
                    break;
                }
                self.flip(doc as usize);
            }
        }
        Ok(())
    }

    /// `self = self AND NOT` the docs `iter` provides.
    ///
    /// Equivalent to `FixedBitSet.andNot(DocIdSetIterator)` (`FixedBitSet.java:664-679`):
    /// an iterator over a `FixedBitSet` is masked out word by word, a
    /// `DocBaseBitSetIterator` word by word from its doc base, and any other
    /// iterator clears each doc it returns.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] when the iterator is already
    /// positioned, and propagates iteration errors.
    pub fn and_not_iterator(
        &mut self,
        iter: &mut dyn crate::search::DocIdSetIterator,
    ) -> crate::error::Result<()> {
        if let Some(bits) = crate::util::bit_set::BitSetIterator::get_fixed_bit_set_or_null(iter) {
            crate::util::bit_set::check_unpositioned(iter)?;
            self.and_not(bits);
        } else if let Some(base_iter) = iter
            .disi_as_any()
            .and_then(|any| any.downcast_ref::<crate::util::bit_set::DocBaseBitSetIterator>())
        {
            crate::util::bit_set::check_unpositioned(iter)?;
            let other = base_iter.get_bit_set();
            self.and_not_offset(
                (base_iter.get_doc_base() >> 6) as usize,
                other.get_bits(),
                other.num_words,
            );
        } else {
            crate::util::bit_set::check_unpositioned(iter)?;
            let mut doc = iter.next_doc()?;
            while doc != crate::search::NO_MORE_DOCS {
                self.clear(doc as usize);
                doc = iter.next_doc()?;
            }
        }
        Ok(())
    }

    /// Scans the backing store to check whether every bit is clear.
    ///
    /// Equivalent to `FixedBitSet.scanIsEmpty()`. Deliberately not named
    /// `is_empty`, as Lucene notes, to emphasise that it is not low cost.
    pub fn scan_is_empty(&self) -> bool {
        // Depends on the ghost bits being clear!
        (0..self.num_words).all(|i| self.word(i) == 0)
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
            self.xor_word(start_word, startmask & endmask);
            return;
        }

        self.xor_word(start_word, startmask);
        for i in (start_word + 1)..end_word {
            self.set_word(i, !self.word(i));
        }
        self.xor_word(end_word, endmask);
    }

    /// Flips the bit at `index`.
    ///
    /// Equivalent to `FixedBitSet.flip(int)`.
    pub fn flip(&mut self, index: usize) {
        debug_assert!(
            index < self.num_bits,
            "index={index} numBits={}",
            self.num_bits
        );
        let word_num = index >> 6;
        let bitmask = 1u64 << (index & 63);
        self.xor_word(word_num, bitmask);
    }

    /// Clears the bits in the range `[start_index, end_index)`.
    ///
    /// Equivalent to `FixedBitSet.clear(int, int)`.
    pub fn clear_range(&mut self, start_index: usize, end_index: usize) {
        debug_assert!(
            start_index < self.num_bits,
            "startIndex={start_index}, numBits={}",
            self.num_bits
        );
        debug_assert!(
            end_index <= self.num_bits,
            "endIndex={end_index}, numBits={}",
            self.num_bits
        );
        if end_index <= start_index {
            return;
        }

        let start_word = start_index >> 6;
        let end_word = (end_index - 1) >> 6;

        // Inverted, since we are clearing.
        let startmask = !(u64::MAX << (start_index & 63));
        let endmask = !(u64::MAX >> (end_index.wrapping_neg() & 63));

        if start_word == end_word {
            self.and_word(start_word, startmask | endmask);
            return;
        }

        self.and_word(start_word, startmask);
        self.fill_words(start_word + 1, end_word, 0);
        self.and_word(end_word, endmask);
    }

    /// Returns this set as read-only [`Bits`].
    ///
    /// Equivalent to `FixedBitSet.asReadOnlyBits()` (`FixedBitSet.java:893-895`),
    /// `new FixedBits(bits, numBits)`: the view wraps this set's own words, so a
    /// later write to the set shows through it, and a consumer handed the view
    /// cannot cast its way back to write access.
    pub fn as_read_only_bits(&self) -> crate::util::bit_set::FixedBits {
        crate::util::bit_set::FixedBits::from_bit_set(
            Self::from_words(Arc::clone(&self.bits), self.num_bits)
                .expect("INVARIANT: a FixedBitSet's words always hold its num_bits"),
        )
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
            return Err(LuceneError::illegal_argument(
                "Some bits are set beyond the end of live docs".to_string(),
            ));
        }
        Ok(())
    }

    /// Calls `consumer` with `base` added to the index of every set bit in
    /// `[from, to)`.
    ///
    /// Equivalent to `FixedBitSet.forEach(int, int, int,
    /// CheckedIntConsumer<IOException>)` (`FixedBitSet.java:910-916`), which
    /// queries use when a bit set is an intermediate representation of their
    /// matches. The consumer becomes a closure returning `Result<(), E>`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java fixes the consumer's checked
    /// exception to `IOException`; unchecked ones, such as the
    /// `CollectionTerminatedException` a collector throws to stop early, pass
    /// through it untyped. Rust has no unchecked exceptions, so the error type
    /// is generic: a caller's closure returns the error it can raise — a
    /// collection outcome, or none at all — and this method hands it back
    /// unchanged. Which bits are visited, and in which order, is Java's.
    pub fn for_each<E, F>(
        &self,
        mut from: usize,
        to: usize,
        base: i32,
        consumer: &mut F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(i32) -> std::result::Result<(), E>,
    {
        assert!(
            from <= to && to <= self.length(),
            "range [{from}, {to}) out of bounds"
        );

        // First, align `from` with a word start, i.e. a multiple of 64.
        if (from & 0x3f) != 0 {
            let mut bits = self.word(from >> 6) >> (from & 63);
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
            Self::for_each_word(self.word(i), base.wrapping_add((i << 6) as i32), consumer)?;
        }

        // Now handle the remaining bits in the last partial word.
        if (to & 0x3f) != 0 {
            let bits = self.word(to >> 6) & ((1u64 << (to & 63)) - 1);
            Self::for_each_word(bits, base.wrapping_add((to & !0x3f) as i32), consumer)?;
        }

        Ok(())
    }

    /// Equivalent to the private `FixedBitSet.forEach(long, int, CheckedIntConsumer)`.
    fn for_each_word<E, F>(mut bits: u64, base: i32, consumer: &mut F) -> std::result::Result<(), E>
    where
        F: FnMut(i32) -> std::result::Result<(), E>,
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
            let mut word = self.word(from >> 6) >> (from & 63);
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
            let word = self.word(i);
            offset = Self::word_to_array(word, base.wrapping_add((i << 6) as i32), array, offset);
        }

        // Now handle the remaining bits in the last partial word.
        if (to & 0x3f) != 0 {
            let word = self.word(to >> 6) & ((1u64 << (to & 63)) - 1);
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
            h ^= self.word(i);
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

    fn as_bit_set(&self) -> Option<&dyn crate::util::bit_set::BitSet> {
        Some(self)
    }

    fn length(&self) -> usize {
        self.length()
    }

    /// Equivalent to the `FixedBitSet.applyMask(FixedBitSet, int)` override
    /// (`FixedBitSet.java:897-909`).
    fn apply_mask(&self, bit_set: &mut FixedBitSet, offset: usize) -> Result<(), LuceneError> {
        FixedBitSet::apply_mask(self, bit_set, offset)
    }

    /// A `FixedBitSet` is one of the classes `FixedBitSet.copyOf` tests for.
    fn bits_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
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
        Self::BASE_RAM_BYTES_USED
            + RamUsageEstimator::align_object_size(
                RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 8 * self.bits.len() as i64,
            )
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
        self.bits.len() == other.bits.len()
            && (0..self.bits.len()).all(|i| self.word(i) == other.word(i))
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
    /// [`LuceneErrorKind::IllegalState`] when the count went negative, which is
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
            return Err(LuceneError::illegal_state(format!(
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

/// Thrown by Lucene on detecting that `Thread.interrupt()` had been called.
/// Unlike [`InterruptedException`], it is unchecked: Lucene's waits catch the
/// former and throw this, which travels as
/// [`LuceneErrorKind::ThreadInterrupted`].
///
/// Equivalent to `org.apache.lucene.util.ThreadInterruptedException`, a
/// `RuntimeException` built over its cause; its message, Java's
/// `getMessage()`, is the cause's `toString()`, `java.lang.InterruptedException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInterruptedException {
    cause: InterruptedException,
}

impl ThreadInterruptedException {
    /// Wraps the interruption a wait raised.
    ///
    /// Equivalent to `ThreadInterruptedException(InterruptedException)`.
    pub fn new(ie: InterruptedException) -> Self {
        Self { cause: ie }
    }
}

impl Display for ThreadInterruptedException {
    /// Java's `getMessage()`: `RuntimeException(Throwable)` takes the
    /// cause's `toString()` as its message.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.cause)
    }
}

impl std::error::Error for ThreadInterruptedException {
    /// Java's `getCause()`.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fs;
    /// `new FixedBitSet(new long[1], 65)` throws Java's
    /// `IllegalArgumentException` (`FixedBitSet.java:162-167`). Measured
    /// against `lucene-core-10.5.0.jar` (`oracle/fix/MiscFix.java`, item 20):
    /// `The given long array is too small  to hold 65 bits`, two spaces.
    #[test]
    fn fixed_bit_set_from_too_few_words_is_lucenes_illegal_argument() {
        assert!(matches!(
            super::FixedBitSet::from_bits(vec![0u64], 65),
            Err(super::LuceneError { kind: super::LuceneErrorKind::IllegalArgument { ref message, .. }, .. })
                if message == "The given long array is too small  to hold 65 bits"
        ));
    }

    /// `ArrayUtil.parseInt` reads digits with `Character.digit`, and refuses
    /// a radix outside `2..=36` with a message-less `NumberFormatException`.
    /// Measured against `lucene-core-10.5.0.jar` (`oracle/fix/MiscFix.java`,
    /// item 23): Arabic-Indic `١٢` is 12, fullwidth `７ｆ` in radix 16 is 127,
    /// and radix 1 throws `NumberFormatException: null`.
    #[test]
    fn array_util_parse_int_reads_digits_as_character_digit_does() {
        let arabic: Vec<char> = "\u{0661}\u{0662}".chars().collect();
        let fullwidth: Vec<char> = "\u{FF17}\u{FF46}".chars().collect();
        assert_eq!(super::ArrayUtil::parse_int(&arabic, 0, 2).unwrap(), 12);
        assert_eq!(
            super::ArrayUtil::parse_int_radix(&fullwidth, 0, 2, 16).unwrap(),
            127
        );
        assert!(matches!(
            super::ArrayUtil::parse_int_radix(&['1'], 0, 1, 1),
            Err(super::LuceneError { kind: super::LuceneErrorKind::Unchecked { class, message: None, .. }, .. })
                if class == "java.lang.NumberFormatException"
        ));
    }

    /// `UnicodeUtil.newString` refuses a code point past `0x10FFFF` with a
    /// message-less `IllegalArgumentException` (`UnicodeUtil.java:571-575`).
    /// Measured against `lucene-core-10.5.0.jar` (`oracle/fix/MiscFix.java`,
    /// item 25): `IllegalArgumentException: null`.
    #[test]
    fn unicode_util_new_string_refuses_an_invalid_code_point_without_a_message() {
        assert!(matches!(
            super::unicode_util::UnicodeUtil::new_string(&[0x11_0000], 0, 1),
            Err(super::LuceneError { kind: super::LuceneErrorKind::IllegalArgument { ref message, .. }, .. }) if message.is_empty()
        ));
    }

    /// `new LongsRef(long[], int, int)` only asserts `isValid()`
    /// (`LongsRef.java:54-59`): a release build — Java without `-ea` — builds
    /// an inconsistent reference instead of failing.
    #[test]
    #[cfg(not(debug_assertions))]
    fn longs_ref_from_longs_only_asserts_validity() {
        let longs = super::refs::LongsRef::from_longs(vec![1, 2], 1, 5);
        assert_eq!(longs.length, 5);
    }

    /// The same, in a debug build — Java under `-ea` — where the assertion
    /// fires.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn longs_ref_from_longs_asserts_validity_in_debug_builds() {
        let _ = super::refs::LongsRef::from_longs(vec![1, 2], 1, 5);
    }

    /// `BytesRefBuilder.copyChars("abc")` grows to
    /// `oversize(maxUTF8Length(3) = 9, 1) = 16` bytes (`BytesRefBuilder.java:138`),
    /// not to the three bytes the UTF-8 text needs.
    #[test]
    fn bytes_ref_builder_copy_chars_grows_to_the_max_utf8_length() {
        let mut builder = BytesRefBuilder::new();
        builder.copy_chars("abc");
        assert_eq!(builder.length(), 3);
        assert_eq!(&builder.bytes()[..3], b"abc");
        assert_eq!(builder.bytes().len(), ArrayUtil::oversize(9, 1));
        assert_eq!(builder.bytes().len(), 16);
    }

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
        let mut out = crate::store::ByteBuffersDataOutput::new();
        let mut scratch = [0u8; MAX_LENGTH_PER_GROUP];
        GroupVIntUtil::write_group_vints_output(&mut out, &mut scratch, &values, values.len())
            .unwrap();
        let encoded = out.to_array_copy().unwrap();

        // The fast path and the benchmark baseline decode the same bytes alike.
        let mut decoded = vec![0i32; values.len()];
        let mut input = crate::store::ByteArrayDataInput::new(&encoded);
        GroupVIntUtil::read_group_vints_input(&mut input, &mut decoded, values.len()).unwrap();
        assert_eq!(input.position(), encoded.len());
        assert_eq!(decoded, values);

        let mut decoded = vec![0i32; values.len()];
        let mut input = crate::store::ByteArrayDataInput::new(&encoded);
        GroupVIntUtil::read_group_vints_baseline(&mut input, &mut decoded, values.len()).unwrap();
        assert_eq!(input.position(), encoded.len());
        assert_eq!(decoded, values);
    }

    /// `GroupVIntUtil.toInt(long)` (`GroupVIntUtil.java:174-179`) throws
    /// `ArithmeticException("integer overflow")` for a value past an unsigned
    /// 32-bit integer, in a full group and in the trailing vInts alike.
    #[test]
    fn group_vints_from_longs_refuse_a_value_past_32_bits() {
        let mut out = crate::store::ByteBuffersDataOutput::new();
        let mut scratch = [0u8; MAX_LENGTH_PER_GROUP];
        for values in [[1i64, 2, 3, 1 << 32], [1 << 32, 0, 0, 0]] {
            for limit in [4, 1] {
                if limit == 1 && values[0] != 1 << 32 {
                    continue;
                }
                let error = GroupVIntUtil::write_group_vints_output_long(
                    &mut out,
                    &mut scratch,
                    &values,
                    limit,
                )
                .unwrap_err();
                assert!(
                    matches!(&error, LuceneError { kind: LuceneErrorKind::Arithmetic(m), .. } if m == "integer overflow"),
                    "{error:?}"
                );
            }
        }
    }

    #[test]
    fn ioutils_close_helpers() {
        fn failing_close() -> Result<(), LuceneError> {
            Err(LuceneError::from(LuceneErrorKind::Io(io::Error::other(
                "close failed",
            ))))
        }

        let result = IOUtils::close([Some(failing_close), None]);
        assert!(result.is_err());

        // close_while_handling_exception swallows a non-Error failure.
        IOUtils::close_while_handling_exception([Some(failing_close), None]).unwrap();
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

    /// rmp #315 (M3): the bulk loops that load a chunk of words at a time
    /// answer exactly as word-by-word loops over a `long[]` do, at every
    /// remainder length; and an `or_range` from a set into itself one word
    /// further on (through a read-only view, which shares the words as Java's
    /// `asReadOnlyBits` shares the `long[]`) sees the words it has just
    /// written, as Java's in-order loop does (`FixedBitSet.java:515-519`).
    #[test]
    fn fixed_bit_set_chunked_bulk_loops_match_word_by_word_loops() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for words in 0..=33usize {
            let a_words: Vec<u64> = (0..words).map(|_| next()).collect();
            let b_words: Vec<u64> = (0..words).map(|_| next()).collect();
            let a = FixedBitSet::from_bits(a_words.clone(), words * 64).unwrap();
            let b = FixedBitSet::from_bits(b_words.clone(), words * 64).unwrap();
            let expected: usize = a_words.iter().map(|w| w.count_ones() as usize).sum();
            assert_eq!(a.cardinality(), expected, "words={words}");
            let expected: u64 = a_words
                .iter()
                .zip(&b_words)
                .map(|(x, y)| (x & y).count_ones() as u64)
                .sum();
            assert_eq!(
                FixedBitSet::intersection_count(&a, &b),
                expected,
                "words={words}"
            );
            let mut union = a.clone();
            union.or_set(&b);
            let got: Vec<u64> = union
                .get_bits()
                .iter()
                .map(|w| w.load(AtomicOrdering::Relaxed))
                .collect();
            let want: Vec<u64> = a_words.iter().zip(&b_words).map(|(x, y)| x | y).collect();
            assert_eq!(got, want, "words={words}");

            if words >= 2 {
                let mut dest = FixedBitSet::from_bits(a_words.clone(), words * 64).unwrap();
                let view = dest.as_read_only_bits();
                let length = (words - 1) * 64;
                FixedBitSet::or_range(view.bit_set(), 0, &mut dest, 64, length);
                let mut want = a_words.clone();
                for i in 1..words {
                    want[i] |= want[i - 1];
                }
                let got: Vec<u64> = dest
                    .get_bits()
                    .iter()
                    .map(|w| w.load(AtomicOrdering::Relaxed))
                    .collect();
                assert_eq!(got, want, "aliased or_range, words={words}");
            }
        }
    }

    /// rmp #297: `ensureCapacityAndClear` keeps an array that is already long
    /// enough, set bits included, exactly as `ensureCapacity` does
    /// (`FixedBitSet.java:64-83`): only a grown array starts empty. Oracle
    /// `scratchpad/auditDE/oracle/AuditOracle.java` over `new FixedBitSet(new
    /// long[] {1, 0, 0, 0}, 10)` and bit 70: `ecac len=256 bit0=true`, `ec
    /// len=256 bit0=true`.
    #[test]
    fn ensure_capacity_and_clear_keeps_a_long_enough_array_as_java() {
        for preserve in [false, true] {
            let bits = FixedBitSet::from_bits(vec![1, 0, 0, 0], 10).unwrap();
            let grown = if preserve {
                FixedBitSet::ensure_capacity(bits, 70)
            } else {
                FixedBitSet::ensure_capacity_and_clear(bits, 70)
            };
            assert_eq!(grown.length(), 256, "preserve={preserve}");
            assert!(grown.get(0), "preserve={preserve}");
        }
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
        stream.close();
    }

    /// `NoOutputInfoStream.message` is
    /// `assert false : "message() should not be called when isEnabled returns false"`
    /// (`InfoStream.java:36-38`): a debug build fails it.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "message() should not be called when isEnabled returns false")
    )]
    fn no_output_info_stream_message_asserts() {
        NoOutputInfoStream.message("component", "should be ignored");
    }

    /// `os.arch` is the JVM's name for the architecture (JDK 25
    /// `make/autoconf/platform.m4`): `IndexWriter` records it in every
    /// segment's diagnostics and Lucene compares it with `"amd64"`.
    #[test]
    fn os_arch_is_the_jvm_name_of_the_architecture() {
        let jvm = Constants::jvm_os_arch;
        assert_eq!(jvm("x86_64", "linux", true, false), "amd64");
        assert_eq!(jvm("x86_64", "windows", true, false), "amd64");
        assert_eq!(jvm("x86_64", "macos", true, false), "x86_64");
        assert_eq!(jvm("x86_64", "linux", true, true), "x32");
        assert_eq!(jvm("x86", "linux", true, true), "i386");
        assert_eq!(jvm("x86", "windows", true, true), "x86");
        assert_eq!(jvm("aarch64", "linux", true, false), "aarch64");
        assert_eq!(jvm("aarch64", "macos", true, false), "aarch64");
        assert_eq!(jvm("arm", "linux", true, true), "arm");
        assert_eq!(jvm("powerpc", "linux", false, true), "ppc");
        assert_eq!(jvm("powerpc64", "linux", false, false), "ppc64");
        assert_eq!(jvm("powerpc64", "linux", true, false), "ppc64le");
        assert_eq!(jvm("mips", "linux", false, true), "mips");
        assert_eq!(jvm("mips", "linux", true, true), "mipsel");
        assert_eq!(jvm("mips64", "linux", false, false), "mips64");
        assert_eq!(jvm("mips64", "linux", true, false), "mips64el");
        assert_eq!(jvm("sparc64", "linux", false, false), "sparcv9");
        assert_eq!(jvm("s390x", "linux", false, false), "s390x");
        assert_eq!(jvm("riscv64", "linux", true, false), "riscv64");
        assert_eq!(jvm("loongarch64", "linux", true, false), "loongarch64");
        if cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_pointer_width = "64"
        )) && std::env::var_os("os.arch").is_none()
        {
            assert_eq!(Constants::os_arch(), "amd64");
        }
    }

    #[test]
    fn constants_basic() {
        assert!(!Constants::os_name().is_empty());
        assert!(!Constants::os_arch().is_empty());
        assert!(Constants::jre_is_64bit());
    }

    #[test]
    fn thread_interrupted_exception_display() {
        // Oracle `InterruptOracle.java`: `getMessage()` is
        // `java.lang.InterruptedException`, the cause's `toString()`.
        let e = ThreadInterruptedException::new(InterruptedException);
        assert_eq!(e.to_string(), "java.lang.InterruptedException");
        assert!(format!("{:?}", e).contains("ThreadInterruptedException"));
        assert!(std::error::Error::source(&e).is_some());
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

    /// `ArrayUtil.getUnsignedComparator(3)` compares exactly three bytes
    /// (`ArrayUtil.java:804-816`); the Java oracle prints `cmp3=0` for these
    /// arrays, which differ only in the fourth byte.
    #[test]
    fn unsigned_comparator_of_other_widths_compares_num_bytes_only() {
        let x = [1u8, 2, 3, 9];
        let y = [1u8, 2, 3, 0];
        assert_eq!(
            ArrayUtil::get_unsigned_comparator(3).compare(&x, 0, &y, 0),
            0
        );
        assert!(ArrayUtil::get_unsigned_comparator(4).compare(&x, 0, &y, 0) > 0);
        assert!(ArrayUtil::get_unsigned_comparator(2).compare(&[1u8, 3], 0, &[1u8, 2], 0) > 0);
    }

    /// `ArrayUtil.parseInt(char[], int offset, int len)` parses only the range;
    /// the Java oracle prints `12` for `parseInt({'x','1','2'}, 1, 2)`.
    #[test]
    fn parse_int_reads_the_given_range() {
        assert_eq!(ArrayUtil::parse_int(&['x', '1', '2'], 1, 2).unwrap(), 12);
        assert_eq!(ArrayUtil::parse_int(&['-', '7', 'z'], 0, 2).unwrap(), -7);
    }

    /// `NumericUtils.subtract`/`add` throw `IllegalArgumentException` with
    /// these messages (Java oracle: `java.lang.IllegalArgumentException: a < b`,
    /// `... a + b overflows bytesPerDim=2`).
    #[test]
    fn numeric_utils_subtract_and_add_report_javas_illegal_argument() {
        let mut result = [0u8; 2];
        let error = NumericUtils::subtract(2, 0, &[0, 1], &[0, 2], &mut result).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. } if m == "a < b"),
            "{error:?}"
        );
        let error = NumericUtils::add(2, 0, &[0xff, 0xff], &[0, 1], &mut result).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. } if m == "a + b overflows bytesPerDim=2"),
            "{error:?}"
        );
        NumericUtils::subtract(2, 0, &[0x12, 0x05], &[0x10, 0x00], &mut result).unwrap();
        assert_eq!(result, [0x02, 0x05]);
    }

    /// `RamUsageEstimator.humanReadableUnits(long)` rounds half to even, as
    /// `DecimalFormat("0.#")` does; every expected string is the Java
    /// oracle's output for the same input.
    #[test]
    fn human_readable_units_rounds_half_to_even_as_decimal_format() {
        for (bytes, expected) in [
            (1280, "1.2 KB"),
            (1331, "1.3 KB"),
            (1177, "1.1 KB"),
            (3584, "3.5 KB"),
            (1075, "1 KB"),
            (1024, "1 KB"),
            (2_621_440, "2.5 MB"),
            (1_610_612_736, "1.5 GB"),
            (999, "999 bytes"),
        ] {
            assert_eq!(
                RamUsageEstimator::human_readable_units(bytes),
                expected,
                "{bytes}"
            );
        }
    }

    /// `IOUtils.useOrSuppress` keeps the first throwable (`IOUtils.java:490-497`),
    /// so `IOUtils.close` reports the first close failure.
    #[test]
    fn ioutils_close_reports_the_first_failure() {
        let first = || {
            Err(LuceneError::from(LuceneErrorKind::Io(io::Error::other(
                "first close failed",
            ))))
        };
        let second = || {
            Err(LuceneError::from(LuceneErrorKind::Io(io::Error::new(
                io::ErrorKind::NotFound,
                "second close failed",
            ))))
        };
        let error =
            IOUtils::close([IOUtils::closeable(first), IOUtils::closeable(second)]).unwrap_err();
        match error {
            LuceneError {
                kind: LuceneErrorKind::Io(e),
                ..
            } => {
                assert_eq!(e.to_string(), "first close failed");
                assert_eq!(e.kind(), io::ErrorKind::Other);
            }
            other => panic!("{other:?}"),
        }
    }

    /// One closeable of the `IOUtils` oracle (`scratchpad/oracle/e2/Oracle.java`):
    /// records its name, then succeeds, returns `error`, or panics.
    fn oracle_closeable<'a>(
        log: &'a std::cell::RefCell<Vec<String>>,
        name: &'a str,
        outcome: &'a str,
    ) -> Option<Closer<'a>> {
        IOUtils::closeable(move || {
            log.borrow_mut().push(name.to_string());
            match outcome {
                "ok" => Ok(()),
                "io" => Err(LuceneError::from(LuceneErrorKind::Io(io::Error::other(
                    name.to_string(),
                )))),
                "rte" => Err(LuceneError::illegal_state(name)),
                "err" => Err(LuceneError::from(LuceneErrorKind::AssertionError(
                    name.to_string(),
                ))),
                "panic" => panic!("{name}"),
                other => unreachable!("{other}"),
            }
        })
    }

    fn java_class_and_message(error: &LuceneError) -> String {
        let message = match error {
            LuceneError {
                kind: LuceneErrorKind::Io(e),
                ..
            } => e.to_string(),
            LuceneError {
                kind: LuceneErrorKind::IllegalState { message, .. },
                ..
            } => message.clone(),
            LuceneError {
                kind: LuceneErrorKind::AssertionError(m),
                ..
            } => m.clone(),
            other => panic!("{other:?}"),
        };
        format!("{}({message})", error.java_simple_name())
    }

    /// `IOUtils.close` closes every closeable, skips null, and throws the
    /// first throwable — Java oracle: `close/io,null,io,ok: closed=[a, b, c]
    /// -> threw IOException(a)`, `close/rte,io: closed=[a, b] -> threw
    /// IllegalStateException(a)`, `close/ok,err,io: closed=[a, b, c] -> threw
    /// AssertionError(b)`, `close/all-ok: closed=[a, b] -> normal`.
    #[test]
    fn ioutils_close_matches_the_java_oracle() {
        let log = std::cell::RefCell::new(Vec::new());
        let run = |closeables: Vec<Option<Closer<'_>>>| {
            log.borrow_mut().clear();
            let outcome = IOUtils::close(closeables);
            (
                log.borrow().join(","),
                outcome.err().map(|e| java_class_and_message(&e)),
            )
        };
        assert_eq!(
            run(vec![
                oracle_closeable(&log, "a", "io"),
                None,
                oracle_closeable(&log, "b", "io"),
                oracle_closeable(&log, "c", "ok"),
            ]),
            ("a,b,c".to_string(), Some("IOException(a)".to_string()))
        );
        assert_eq!(
            run(vec![
                oracle_closeable(&log, "a", "rte"),
                oracle_closeable(&log, "b", "io"),
            ]),
            (
                "a,b".to_string(),
                Some("IllegalStateException(a)".to_string())
            )
        );
        assert_eq!(
            run(vec![
                oracle_closeable(&log, "a", "ok"),
                oracle_closeable(&log, "b", "err"),
                oracle_closeable(&log, "c", "io"),
            ]),
            ("a,b,c".to_string(), Some("AssertionError(b)".to_string()))
        );
        assert_eq!(
            run(vec![
                oracle_closeable(&log, "a", "ok"),
                None,
                oracle_closeable(&log, "b", "ok"),
            ]),
            ("a,b".to_string(), None)
        );
    }

    /// `IOUtils.closeWhileHandlingException` swallows every non-`Error`
    /// failure and rethrows the first `Error` once every closeable is closed —
    /// Java oracle: `cwhe/io,null,rte,ok: closed=[a, b, c] -> normal`,
    /// `cwhe/io,err,err,ok: closed=[a, b, c, d] -> threw AssertionError(b)`.
    #[test]
    fn ioutils_close_while_handling_exception_matches_the_java_oracle() {
        let log = std::cell::RefCell::new(Vec::new());
        let outcome = IOUtils::close_while_handling_exception(vec![
            oracle_closeable(&log, "a", "io"),
            None,
            oracle_closeable(&log, "b", "rte"),
            oracle_closeable(&log, "c", "ok"),
        ]);
        assert!(outcome.is_ok());
        assert_eq!(log.borrow().join(","), "a,b,c");

        log.borrow_mut().clear();
        let error = IOUtils::close_while_handling_exception(vec![
            oracle_closeable(&log, "a", "io"),
            oracle_closeable(&log, "b", "err"),
            oracle_closeable(&log, "c", "err"),
            oracle_closeable(&log, "d", "ok"),
        ])
        .unwrap_err();
        assert_eq!(java_class_and_message(&error), "AssertionError(b)");
        assert_eq!(log.borrow().join(","), "a,b,c,d");
    }

    /// `IOUtils.closeWhileSuppressingExceptions(ex, ...)` leaves `ex` the
    /// throwable to rethrow unless a closeable hits an `Error` while `ex` is
    /// not one — Java oracle: `cwse/ex=io; io,null,ok: closed=[a, b] -> threw
    /// IOException(ex)`, `cwse/ex=io; err,io: closed=[a, b] -> threw
    /// AssertionError(a)`, `cwse/ex=err; err,io: closed=[a, b] -> threw
    /// AssertionError(ex)`. `ex` keeps its class and message.
    #[test]
    fn ioutils_close_while_suppressing_exceptions_matches_the_java_oracle() {
        let log = std::cell::RefCell::new(Vec::new());
        let ex = LuceneError::from(LuceneErrorKind::Io(io::Error::other("ex")));
        let thrown = IOUtils::close_while_suppressing_exceptions(
            ex,
            vec![
                oracle_closeable(&log, "a", "io"),
                None,
                oracle_closeable(&log, "b", "ok"),
            ],
        );
        assert_eq!(java_class_and_message(&thrown), "IOException(ex)");
        assert_eq!(log.borrow().join(","), "a,b");

        log.borrow_mut().clear();
        let ex = LuceneError::from(LuceneErrorKind::Io(io::Error::other("ex")));
        let thrown = IOUtils::close_while_suppressing_exceptions(
            ex,
            vec![
                oracle_closeable(&log, "a", "err"),
                oracle_closeable(&log, "b", "io"),
            ],
        );
        assert_eq!(java_class_and_message(&thrown), "AssertionError(a)");
        assert_eq!(log.borrow().join(","), "a,b");

        log.borrow_mut().clear();
        let ex = LuceneError::from(LuceneErrorKind::AssertionError("ex".to_string()));
        let thrown = IOUtils::close_while_suppressing_exceptions(
            ex,
            vec![
                oracle_closeable(&log, "a", "err"),
                oracle_closeable(&log, "b", "io"),
            ],
        );
        assert_eq!(java_class_and_message(&thrown), "AssertionError(ex)");
        assert_eq!(log.borrow().join(","), "a,b");
    }

    /// `IOUtils.applyToAll` visits every non-null element and throws the
    /// first failure — Java oracle: `applyToAll/1,null,2,3 fail 1,2:
    /// closed=[v1, v2, v3] -> threw IOException(fail1)`.
    #[test]
    fn ioutils_apply_to_all_matches_the_java_oracle() {
        let mut visited = Vec::new();
        let error = IOUtils::apply_to_all(vec![Some(1), None, Some(2), Some(3)], |i| {
            visited.push(format!("v{i}"));
            if i != 3 {
                return Err(LuceneError::from(LuceneErrorKind::Io(io::Error::other(
                    format!("fail{i}"),
                ))));
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(visited.join(","), "v1,v2,v3");
        assert_eq!(java_class_and_message(&error), "IOException(fail1)");
    }

    /// A closeable that panics is caught like any `Throwable`: the rest are
    /// still closed, and the panic is resumed when it was the first failure
    /// (`close`, `closeWhileHandlingException`) and dropped, as Java
    /// suppresses it, when an error came first.
    #[test]
    fn ioutils_close_family_closes_past_a_panic() {
        let log = std::cell::RefCell::new(Vec::new());
        let resumed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            IOUtils::close(vec![
                oracle_closeable(&log, "a", "panic"),
                oracle_closeable(&log, "b", "io"),
            ])
        }))
        .unwrap_err();
        assert_eq!(
            resumed.downcast_ref::<String>().map(String::as_str),
            Some("a")
        );
        assert_eq!(log.borrow().join(","), "a,b");

        log.borrow_mut().clear();
        let error = IOUtils::close(vec![
            oracle_closeable(&log, "a", "io"),
            oracle_closeable(&log, "b", "panic"),
        ])
        .unwrap_err();
        assert_eq!(java_class_and_message(&error), "IOException(a)");
        assert_eq!(log.borrow().join(","), "a,b");

        log.borrow_mut().clear();
        let resumed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            IOUtils::close_while_handling_exception(vec![
                oracle_closeable(&log, "a", "panic"),
                oracle_closeable(&log, "b", "ok"),
            ])
        }))
        .unwrap_err();
        assert_eq!(
            resumed.downcast_ref::<String>().map(String::as_str),
            Some("a")
        );
        assert_eq!(log.borrow().join(","), "a,b");
    }

    /// `IOUtils.fsync(path, false)` opens the file for writing, so a read-only
    /// file fails (Java oracle: `java.nio.file.AccessDeniedException`), and a
    /// missing one fails with `NoSuchFileException`.
    #[test]
    fn ioutils_fsync_opens_a_regular_file_for_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("ro");
        fs::write(&file, [1u8]).unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();
        let error = IOUtils::fsync(&file, false).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Io(e), .. } if e.kind() == io::ErrorKind::PermissionDenied),
            "{error:?}"
        );
        let error = IOUtils::fsync(tmp.path().join("missing"), false).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Io(e), .. } if e.kind() == io::ErrorKind::NotFound),
            "{error:?}"
        );
        IOUtils::fsync(tmp.path(), true).unwrap();
    }

    /// `FSDirectory.sync` fsyncs each file through `IOUtils.fsync(path, false)`
    /// (`FSDirectory.java:300-302`), which opens it for writing: syncing a
    /// read-only file fails (Java oracle: `java.nio.file.AccessDeniedException`).
    #[test]
    fn fs_directory_sync_opens_each_file_for_writing() {
        use crate::store::Directory;
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("ro");
        fs::write(&file, [1u8]).unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();
        let dir = crate::store::FSDirectory::open(tmp.path()).unwrap();
        let error = dir.sync(&["ro".to_string()]).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Io(e), .. } if e.kind() == io::ErrorKind::PermissionDenied),
            "{error:?}"
        );
        dir.sync_metadata().unwrap();
    }

    /// `InfoStream.setDefault` changes what newly created classes read through
    /// `InfoStream.getDefault()`, `LiveIndexWriterConfig` included
    /// (`LiveIndexWriterConfig.java:135`).
    #[test]
    fn a_new_writer_config_reads_the_default_info_stream() {
        let previous = default_info_stream();
        // A no-output stream, so tests running concurrently are unaffected.
        let custom: std::sync::Arc<dyn InfoStream> = std::sync::Arc::new(NoOutputInfoStream);
        set_default_info_stream(std::sync::Arc::clone(&custom));
        let picked = crate::index::IndexWriterConfig::new().info_stream();
        let defaulted = default_info_stream();
        set_default_info_stream(previous);
        assert!(std::sync::Arc::ptr_eq(&defaulted, &custom));
        assert!(std::sync::Arc::ptr_eq(&picked, &custom));
    }

    /// `new FixedBitSet(long[], int)` asserts `verifyGhostBitsClear()`
    /// (`FixedBitSet.java:171`).
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "verify_ghost_bits_clear")]
    fn fixed_bit_set_from_bits_asserts_the_ghost_bits_are_clear() {
        let _ = FixedBitSet::from_bits(vec![1u64 << 10], 5).unwrap();
    }

    /// The default `Bits.applyMask` reads through `get`, so the error a `get`
    /// raises fails the call (`Bits.java:55-63`), and bits it clears are those
    /// whose shifted bit is clear.
    #[test]
    fn bits_apply_mask_default_clears_and_propagates_get_errors() {
        #[derive(Debug)]
        struct EvenBits(bool);
        impl Bits for EvenBits {
            fn get(&self, index: usize) -> bool {
                index.is_multiple_of(2)
            }
            fn try_get(&self, index: usize) -> Result<bool, LuceneError> {
                if self.0 {
                    Err(LuceneError::from(LuceneErrorKind::Other(
                        "exiting".to_string(),
                    )))
                } else {
                    Ok(self.get(index))
                }
            }
            fn length(&self) -> usize {
                100
            }
        }
        let mut set = FixedBitSet::new(8);
        set.set_range(0, 8);
        EvenBits(false).apply_mask(&mut set, 1).unwrap();
        // bit i survives when offset + i = 1 + i is even.
        assert_eq!(
            (0..8).filter(|&i| set.get(i)).collect::<Vec<_>>(),
            vec![1, 3, 5, 7]
        );
        let error = EvenBits(true).apply_mask(&mut set, 0).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Other(m), .. } if m == "exiting"),
            "{error:?}"
        );
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

#[cfg(test)]
mod pass_d_bytes_ref_and_size_of_object_tests {
    use super::*;

    /// rmp #322: `BytesRef(CharSequence)` allocates
    /// `UnicodeUtil.maxUTF8Length(text.length())` bytes — three per UTF-16 code
    /// unit — and sets `length` to the UTF-8 bytes written (`BytesRef.java:79-82`).
    #[test]
    fn from_char_sequence_sizes_its_buffer_as_java() {
        let sized = |text: &str| {
            let b = BytesRef::from_char_sequence(text);
            (b.bytes.len(), b.length, b.slice().to_vec())
        };
        assert_eq!(sized("abc"), (9, 3, b"abc".to_vec()));
        assert_eq!(sized(""), (0, 0, Vec::new()));
        assert_eq!(
            sized("h\u{e9}llo"),
            (15, 6, "h\u{e9}llo".as_bytes().to_vec())
        );
        // A supplementary character is two UTF-16 code units.
        assert_eq!(
            sized("\u{1F600}x"),
            (9, 5, "\u{1F600}x".as_bytes().to_vec())
        );
    }

    /// rmp #309: `sizeOfObject` returns an `Accountable`'s own
    /// `ramBytesUsed()`, zero included (`RamUsageEstimator.java:400-401`); only
    /// a class it does not know falls back to `defSize`, or to its shallow size
    /// when `defSize` is not positive; `null` costs nothing.
    #[test]
    fn size_of_object_dispatches_on_the_class_as_java() {
        let zero = NullAccountable;
        assert_eq!(
            RamUsageEstimator::size_of_object(Some(SizedObject::Accountable(&zero))),
            0
        );
        assert_eq!(RamUsageEstimator::size_of_object(None), 0);
        assert_eq!(
            RamUsageEstimator::size_of_object(Some(SizedObject::String("field"))),
            RamUsageEstimator::size_of_string("field")
        );
        assert_eq!(
            RamUsageEstimator::size_of_object(Some(SizedObject::IntArray(&[1, 2, 3]))),
            RamUsageEstimator::size_of_int(&[1, 2, 3])
        );
        let unknown = SizedObject::Unknown { shallow_size: 24 };
        assert_eq!(RamUsageEstimator::size_of_object(Some(unknown)), 256);
        assert_eq!(
            RamUsageEstimator::size_of_object_with_def_size(Some(unknown), 0),
            24
        );
        assert_eq!(
            RamUsageEstimator::size_of_object_with_def_size(Some(unknown), 40),
            40
        );
    }
}

#[cfg(test)]
mod pass_e_fixed_bit_set_sharing_tests {
    use super::*;

    /// rmp #315: `FixedBitSet.asReadOnlyBits()` wraps the set's own `long[]`
    /// (`FixedBitSet.java:893-895`), so a write after the call shows through
    /// the view. Measured against `lucene-core-10.5.0.jar`
    /// (`oracle/passE/PassEOracle.java`, `fbs`): `view5=true view70=true
    /// view6=false cleared5=false copy7=false view7=true
    /// class=org.apache.lucene.util.FixedBits`.
    #[test]
    fn as_read_only_bits_shares_the_words() {
        let mut set = FixedBitSet::new(100);
        let view = set.as_read_only_bits();
        set.set(5);
        set.set(70);
        assert_eq!(
            (view.get(5), view.get(70), view.get(6)),
            (true, true, false)
        );
        set.clear(5);
        assert!(!view.get(5), "a clear shows through too");
        // `copyOf(view)` clones the set behind it (`FixedBitSet.java:866-870`):
        // a deep copy, which later writes do not reach.
        let copy = FixedBitSet::copy_of(&view);
        set.set(7);
        assert!(!copy.get(7));
        assert!(view.get(7));
    }

    /// `FixedBitSet.clone()` copies the words (`FixedBitSet.java:829-834`): the
    /// clone and the original never share.
    #[test]
    fn clone_copies_the_words() {
        let mut set = FixedBitSet::new(10);
        set.set(1);
        let clone = set.clone();
        set.set(2);
        assert!(clone.get(1) && !clone.get(2));
        assert_eq!(clone.get_bits().len(), set.get_bits().len());
    }
}

#[cfg(test)]
mod fixed_bit_set_copy_of_tests {
    use super::*;
    use std::time::Instant;

    /// A `Bits` that is neither a `FixedBitSet` nor a `FixedBits`: the
    /// `else` branch of `FixedBitSet.copyOf` (`FixedBitSet.java:874-882`),
    /// which copies bit by bit.
    #[derive(Debug)]
    struct PlainBits<'a>(&'a FixedBitSet);

    impl Bits for PlainBits<'_> {
        fn get(&self, index: usize) -> bool {
            self.0.get(index)
        }
        fn length(&self) -> usize {
            self.0.length()
        }
    }

    fn random_set(num_bits: usize, seed: u64) -> FixedBitSet {
        let mut set = FixedBitSet::new(num_bits);
        let mut x = seed | 1;
        for i in 0..num_bits {
            // xorshift64: a fixed, dependency-free bit pattern.
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            if x & 3 == 0 {
                set.set(i);
            }
        }
        set
    }

    fn assert_same_bits(a: &FixedBitSet, b: &FixedBitSet) {
        assert_eq!(a.length(), b.length());
        assert_eq!(a.cardinality(), b.cardinality());
        for i in 0..a.length() {
            assert_eq!(a.get(i), b.get(i), "bit {i}");
        }
    }

    /// rmp #253: `FixedBitSet.copyOf(Bits)` (`FixedBitSet.java:866-883`)
    /// restores the `FixedBitSet` behind a `FixedBits`, clones a
    /// `FixedBitSet` word by word (`clone()`, `FixedBitSet.java:829-834`),
    /// and copies any other `Bits` bit by bit. The three paths give equal
    /// sets, and the two word copies equal words.
    #[test]
    fn copy_of_is_equal_on_every_path() {
        for num_bits in [0usize, 1, 63, 64, 65, 1000, 4097] {
            let set = random_set(num_bits, 0x9E37_79B9 + num_bits as u64);
            let from_set = FixedBitSet::copy_of(&set);
            let from_view = FixedBitSet::copy_of(&set.as_read_only_bits());
            assert_same_bits(&set, &from_set);
            assert_same_bits(&set, &from_view);
            // The bit-by-bit path starts with `bitSet.set(0, length)`, whose
            // `assert startIndex >= 0 && startIndex < numBits`
            // (`FixedBitSet.java:777-778`) fails for an empty set under `-ea`;
            // `copy_of_an_empty_plain_bits_asserts_as_java` pins that case.
            if num_bits > 0 {
                let from_plain = FixedBitSet::copy_of(&PlainBits(&set));
                assert_same_bits(&set, &from_plain);
            }
            let words = |s: &FixedBitSet| -> Vec<u64> {
                s.get_bits()
                    .iter()
                    .map(|w| w.load(AtomicOrdering::Relaxed))
                    .collect()
            };
            assert_eq!(words(&from_set), words(&set));
            assert_eq!(words(&from_view), words(&set));
        }
    }

    /// `FixedBitSet.copyOf` of an empty plain `Bits` calls `set(0, 0)`, which
    /// asserts `startIndex < numBits` (`FixedBitSet.java:777-778`).
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "startIndex=0, numBits=0"))]
    fn copy_of_an_empty_plain_bits_asserts_as_java() {
        let set = FixedBitSet::new(0);
        let copy = FixedBitSet::copy_of(&PlainBits(&set));
        assert_eq!(copy.length(), 0);
    }

    /// rmp #253 measurement: the word copy of a `FixedBitSet` against the
    /// bit-by-bit copy of the same bits seen through a plain `Bits`. Prints
    /// the mean time per copy; the two results are equal.
    #[test]
    fn copy_of_word_copy_measurement() {
        const NUM_BITS: usize = 1 << 20;
        const ROUNDS: u32 = 16;
        let set = random_set(NUM_BITS, 0xC0FF_EE00);
        let start = Instant::now();
        let mut words = FixedBitSet::new(0);
        for _ in 0..ROUNDS {
            words = std::hint::black_box(FixedBitSet::copy_of(std::hint::black_box(&set)));
        }
        let word_copy = start.elapsed() / ROUNDS;
        let plain = PlainBits(&set);
        let start = Instant::now();
        let mut bitwise = FixedBitSet::new(0);
        for _ in 0..ROUNDS {
            bitwise = std::hint::black_box(FixedBitSet::copy_of(std::hint::black_box(&plain)));
        }
        let bit_copy = start.elapsed() / ROUNDS;
        assert_same_bits(&words, &bitwise);
        println!(
            "FixedBitSet::copy_of({NUM_BITS} bits): word copy {word_copy:?}, \
             bit-by-bit copy {bit_copy:?}"
        );
    }
}
