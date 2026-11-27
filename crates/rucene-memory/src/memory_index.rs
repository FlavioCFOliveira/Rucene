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

//! Port of `MemoryIndex.java`, of the Java package
//! `org.apache.lucene.index.memory`.
//!
//! # Shared state
//!
//! Java's `MemoryIndexReader`, `MemoryFields`, `MemoryTermsEnum` and
//! `MemoryPostingsEnum` are inner classes: they read the enclosing
//! `MemoryIndex`'s `fields` map, pools and flags in place, and an `Info` they
//! were handed stays reachable from them. The port keeps the same sharing:
//! the index state lives behind an [`Arc`] (the map in a [`Mutex`], each
//! `Info` in its own `Arc<Mutex<Info>>`, the pools in a [`Mutex`]), and every
//! reader-side object holds the `Arc`s Java's objects hold references to. The
//! locks only make that sharing expressible; Java's single-thread contract
//! (`MemoryIndex` is not thread-safe until frozen) is unchanged. Locks are
//! always taken in the order state map, `Info`, pools, slice arrays.

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PayloadAttribute, PositionIncrementAttribute,
    TermToBytesRefAttribute,
};
use rucene::analysis::{default_token_attribute_factory, Analyzer, AttributeHandle, TokenStream};
use rucene::document::{FieldType, NumericValue};
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use rucene::index::point_values::{IntersectVisitor, PointTree};
use rucene::index::vector_values::DocIndexIterator;
use rucene::index::{
    BinaryDocValues, ByteVectorValues, DocValuesIterator, DocValuesSkipIndexType, DocValuesSkipper,
    DocValuesType, FieldInfo, FieldInfos, FieldInvertState, Fields, FloatVectorValues, ImpactsEnum,
    IndexOptions, IndexReader, IndexableField, IndexableFieldType, KnnVectorValues, LeafMetaData,
    LeafReader, NumericDocValues, OrdTermState, PointValues, PostingsEnum, SeekStatus,
    SlowImpactsEnum, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
    StoredFieldVisitor, StoredFieldVisitorStatus, TermState, TermVectors, Terms, TermsEnum,
    VectorEncoding,
};
use rucene::search::knn::KnnCollector;
use rucene::search::similarities::Similarity;
use rucene::search::vector_scorer::{
    default_bulk, SharedVectorScorer, VectorScorer, VectorScorerBulk,
};
use rucene::search::{
    AcceptDocs, CollectionResult, CollectorManager, DocIdSetIterator, IndexSearcher, LeafCollector,
    Query, Scorable, ScoreMode, SimpleCollector, SimpleCollectorImpl, NO_MORE_DOCS,
};
use rucene::util::attribute::AttributeSource;
use rucene::util::byte_block_pool::{Allocator, ByteBlockPool, BYTE_BLOCK_SIZE};
use rucene::util::bytes_ref_array::{BytesRefArray, SortableBytesRefArray};
use rucene::util::bytes_ref_hash::{BytesRefHash, BytesStartArray, DEFAULT_CAPACITY};
use rucene::util::concurrent::Counter;
use rucene::util::extra::Version;
use rucene::util::int_block_pool::{
    ByteBlockAllocator, IntBlockPool, RecyclingByteBlockAllocator, RecyclingIntBlockAllocator,
    INT_BLOCK_MASK, INT_BLOCK_SHIFT, INT_BLOCK_SIZE,
};
use rucene::util::{ArrayUtil, Bits, BytesRef};

/// Locks `mutex`, ignoring poisoning: a panic while the lock was held is
/// already propagating, as an exception thrown through Java's unlocked state
/// would.
fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `Number.longValue()` of a stored numeric value.
fn number_long_value(value: &NumericValue) -> i64 {
    match *value {
        NumericValue::Int(v) => v as i64,
        NumericValue::Long(v) => v,
        // `(long) float` / `(long) double`: saturating, NaN to 0, as `as` is.
        NumericValue::Float(v) => v as i64,
        NumericValue::Double(v) => v as i64,
    }
}

// ---------------------------------------------------------------------------
// SlicedIntBlockPool
// ---------------------------------------------------------------------------

/// An `IntBlockPool` that writes linked slices of ints, as `ByteSlicePool`
/// does for bytes.
///
/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.SlicedIntBlockPool`.
///
/// Java's member is package-private, reached by the same package's tests
/// (`TestSlicedIntBlockPool`); Rust's integration tests are another crate, so
/// the port makes it public (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug)]
pub struct SlicedIntBlockPool {
    pool: IntBlockPool,
}

impl SlicedIntBlockPool {
    /// `NEXT_LEVEL_ARRAY`: the index of the next level size.
    const NEXT_LEVEL_ARRAY: [usize; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 9];
    /// `LEVEL_SIZE_ARRAY`: the size of each level.
    const LEVEL_SIZE_ARRAY: [usize; 10] = [2, 4, 8, 16, 16, 32, 32, 64, 64, 128];
    /// `FIRST_LEVEL_SIZE`: the first level size of new slices.
    const FIRST_LEVEL_SIZE: usize = Self::LEVEL_SIZE_ARRAY[0];

    /// Equivalent to `SlicedIntBlockPool(Allocator)`.
    pub fn new(allocator: Box<dyn rucene::util::int_block_pool::IntBlockAllocator>) -> Self {
        Self {
            pool: IntBlockPool::with_allocator(allocator),
        }
    }

    /// Equivalent to `assertSliceBuffer(int[])`.
    fn assert_slice_buffer(buffer: &[i32]) -> bool {
        buffer.iter().all(|&value| value == 0)
    }

    fn head_index(&self) -> usize {
        self.pool.buffer_upto() as usize
    }

    /// Equivalent to `newSlice(int)`: creates a new int slice of the given
    /// size and returns its offset within the head buffer.
    fn new_slice(&mut self, size: usize) -> Result<usize> {
        if self.pool.int_upto() > INT_BLOCK_SIZE - size {
            self.pool.next_buffer()?;
            debug_assert!(Self::assert_slice_buffer(
                self.pool.buffer(self.head_index())
            ));
        }
        let upto = self.pool.int_upto();
        self.pool.set_int_upto(upto + size);
        let int_upto = self.pool.int_upto();
        let head = self.head_index();
        self.pool.buffer_mut(head)[int_upto - 1] = 16;
        Ok(upto)
    }

    /// Equivalent to `allocSlice(int[], int)`: allocates a new slice from
    /// the given slice (block `slice_block`, end marker at `slice_offset`)
    /// and returns its offset within the head buffer.
    fn alloc_slice(&mut self, slice_block: usize, slice_offset: usize) -> Result<usize> {
        let level = (self.pool.buffer(slice_block)[slice_offset] & 15) as usize;
        let new_level = Self::NEXT_LEVEL_ARRAY[level];
        let new_size = Self::LEVEL_SIZE_ARRAY[new_level];
        // Maybe allocate another block
        if self.pool.int_upto() > INT_BLOCK_SIZE - new_size {
            self.pool.next_buffer()?;
            debug_assert!(Self::assert_slice_buffer(
                self.pool.buffer(self.head_index())
            ));
        }

        let new_upto = self.pool.int_upto();
        let offset = new_upto as i32 + self.pool.int_offset();
        self.pool.set_int_upto(new_upto + new_size);
        // Write forwarding address at end of last slice:
        self.pool.buffer_mut(slice_block)[slice_offset] = offset;

        // Write new level:
        let int_upto = self.pool.int_upto();
        let head = self.head_index();
        self.pool.buffer_mut(head)[int_upto - 1] = 16 | new_level as i32;

        Ok(new_upto)
    }

    /// Equivalent to `IntBlockPool.reset(boolean, boolean)`.
    pub fn reset(&mut self, zero_fill_buffers: bool, reuse_first: bool) {
        self.pool.reset(zero_fill_buffers, reuse_first);
    }
}

/// Writes ints into the slices of a [`SlicedIntBlockPool`].
///
/// Equivalent to
/// `org.apache.lucene.index.memory.MemoryIndex.SlicedIntBlockPool.SliceWriter`.
/// Java's writer keeps a reference to its pool; here the pool is passed to
/// each call, because it lives in the same locked state as the writer.
///
/// Java's member is package-private, reached by the same package's tests
/// (`TestSlicedIntBlockPool`); Rust's integration tests are another crate, so
/// the port makes it public (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct SliceWriter {
    offset: i32,
}

impl SliceWriter {
    /// Equivalent to `SliceWriter.reset(int)`.
    pub fn reset(&mut self, slice_offset: i32) {
        self.offset = slice_offset;
    }

    /// Equivalent to `SliceWriter.writeInt(int)`: writes the given value
    /// into the slice and resizes the slice if needed.
    pub fn write_int(&mut self, pool: &mut SlicedIntBlockPool, value: i32) -> Result<()> {
        let mut block = (self.offset >> INT_BLOCK_SHIFT) as usize;
        let mut relative_offset = self.offset as usize & INT_BLOCK_MASK;
        if pool.pool.buffer(block)[relative_offset] != 0 {
            // End of slice; allocate a new one
            relative_offset = pool.alloc_slice(block, relative_offset)?;
            block = pool.head_index();
            self.offset = relative_offset as i32 + pool.pool.int_offset();
        }
        pool.pool.buffer_mut(block)[relative_offset] = value;
        self.offset += 1;
        Ok(())
    }

    /// Equivalent to `SliceWriter.startNewSlice()`: starts a new slice and
    /// returns the start offset, the "address" of the slice.
    pub fn start_new_slice(&mut self, pool: &mut SlicedIntBlockPool) -> Result<i32> {
        self.offset =
            pool.new_slice(SlicedIntBlockPool::FIRST_LEVEL_SIZE)? as i32 + pool.pool.int_offset();
        Ok(self.offset)
    }

    /// Equivalent to `SliceWriter.getCurrentOffset()`.
    pub fn get_current_offset(&self) -> i32 {
        self.offset
    }
}

/// Reads the ints a [`SliceWriter`] wrote.
///
/// Equivalent to
/// `org.apache.lucene.index.memory.MemoryIndex.SlicedIntBlockPool.SliceReader`.
/// Java's reader keeps a reference to its pool and to the current block; here
/// the pool is passed to each call and the block is kept as its index.
///
/// Java's member is package-private, reached by the same package's tests
/// (`TestSlicedIntBlockPool`); Rust's integration tests are another crate, so
/// the port makes it public (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug, Default, Clone)]
pub struct SliceReader {
    upto: usize,
    buffer_upto: usize,
    buffer_offset: i32,
    limit: usize,
    level: usize,
    end: i32,
}

impl SliceReader {
    /// Equivalent to `SliceReader.reset(int, int)`.
    pub fn reset(&mut self, start_offset: i32, end_offset: i32) {
        self.buffer_upto = (start_offset / INT_BLOCK_SIZE as i32) as usize;
        self.buffer_offset = self.buffer_upto as i32 * INT_BLOCK_SIZE as i32;
        self.end = end_offset;
        self.level = 0;

        self.upto = start_offset as usize & INT_BLOCK_MASK;

        let first_size = SlicedIntBlockPool::LEVEL_SIZE_ARRAY[0] as i32;
        if start_offset + first_size >= end_offset {
            // There is only this one slice to read
            self.limit = end_offset as usize & INT_BLOCK_MASK;
        } else {
            self.limit = self.upto + first_size as usize - 1;
        }
    }

    /// Equivalent to `SliceReader.endOfSlice()`.
    pub fn end_of_slice(&self) -> bool {
        debug_assert!(self.upto as i32 + self.buffer_offset <= self.end);
        self.upto as i32 + self.buffer_offset == self.end
    }

    /// Equivalent to `SliceReader.readInt()`.
    pub fn read_int(&mut self, pool: &SlicedIntBlockPool) -> i32 {
        debug_assert!(!self.end_of_slice());
        debug_assert!(self.upto <= self.limit);
        if self.upto == self.limit {
            self.next_slice(pool);
        }
        let value = pool.pool.buffer(self.buffer_upto)[self.upto];
        self.upto += 1;
        value
    }

    /// Equivalent to `SliceReader.nextSlice()`.
    fn next_slice(&mut self, pool: &SlicedIntBlockPool) {
        // Skip to our next slice
        let next_index = pool.pool.buffer(self.buffer_upto)[self.limit];
        self.level = SlicedIntBlockPool::NEXT_LEVEL_ARRAY[self.level];
        let new_size = SlicedIntBlockPool::LEVEL_SIZE_ARRAY[self.level] as i32;

        self.buffer_upto = (next_index / INT_BLOCK_SIZE as i32) as usize;
        self.buffer_offset = self.buffer_upto as i32 * INT_BLOCK_SIZE as i32;

        self.upto = next_index as usize & INT_BLOCK_MASK;

        if next_index + new_size >= self.end {
            // We are advancing to the final slice
            debug_assert!(self.end - next_index > 0);
            self.limit = (self.end - self.buffer_offset) as usize;
        } else {
            // This is not the final slice (subtract 4 for the
            // forwarding address at the end of this new slice)
            self.limit = self.upto + new_size as usize - 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Byte allocator adapter
// ---------------------------------------------------------------------------

/// The `RecyclingByteBlockAllocator` Java hands to `new ByteBlockPool(...)`
/// (`MemoryIndex.java:309-310`).
///
/// Java's `RecyclingByteBlockAllocator` extends `ByteBlockPool.Allocator`;
/// Rucene's implements the sibling trait
/// [`ByteBlockAllocator`] with `&mut self` methods, so this wrapper presents
/// it through the pool's [`Allocator`] trait behind a lock. The blocks it
/// hands out and recycles are those of the wrapped allocator.
#[derive(Debug)]
struct RecyclingByteAllocator(Mutex<RecyclingByteBlockAllocator>);

impl Allocator for RecyclingByteAllocator {
    fn block_size(&self) -> usize {
        lock(&self.0).block_size()
    }

    fn recycle_byte_blocks(&self, blocks: &mut [Vec<u8>], start: usize, end: usize) {
        let mut slots: Vec<Option<Vec<u8>>> = blocks
            .iter_mut()
            .map(|block| Some(std::mem::take(block)))
            .collect();
        lock(&self.0).recycle_byte_blocks(&mut slots, start, end);
    }

    fn get_byte_block(&self) -> Vec<u8> {
        lock(&self.0).get_byte_block()
    }
}

// ---------------------------------------------------------------------------
// SliceByteStartArray
// ---------------------------------------------------------------------------

/// The per-term postings arrays of a field, grown with its term hash.
#[derive(Debug, Default)]
struct SliceArrays {
    /// The start offset in the IntBlockPool per term.
    start: Vec<i32>,
    /// The end pointer in the IntBlockPool for the postings slice per term.
    end: Vec<i32>,
    /// The term frequency.
    freq: Vec<i32>,
}

/// A `DirectBytesStartArray` that also keeps the start, end and frequency of
/// each term's postings slice.
///
/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.SliceByteStartArray`.
/// Java's `Info` reads the three arrays through its reference to this
/// object, which its `BytesRefHash` also holds; here the hash owns the start
/// array, so the three arrays sit in a cell both the hash's start array and
/// the `Info` share.
#[derive(Debug)]
pub(crate) struct SliceByteStartArray {
    /// `DirectBytesStartArray.initSize`.
    init_size: usize,
    /// `DirectBytesStartArray.bytesUsed`: `Counter.newCounter()`, as
    /// `DirectBytesStartArray(int)` makes one.
    bytes_used: Arc<AtomicI64>,
    arrays: Arc<Mutex<SliceArrays>>,
}

impl SliceByteStartArray {
    /// Equivalent to `SliceByteStartArray(int)`.
    fn new(init_size: usize, arrays: Arc<Mutex<SliceArrays>>) -> Self {
        Self {
            init_size,
            bytes_used: Arc::new(AtomicI64::new(0)),
            arrays,
        }
    }
}

impl BytesStartArray for SliceByteStartArray {
    fn init(&mut self) -> Vec<i32> {
        // `super.init()`.
        let ord = vec![0; ArrayUtil::oversize(self.init_size, 4)];
        let mut arrays = lock(&self.arrays);
        arrays.start = vec![0; ArrayUtil::oversize(ord.len(), 4)];
        arrays.end = vec![0; ArrayUtil::oversize(ord.len(), 4)];
        arrays.freq = vec![0; ArrayUtil::oversize(ord.len(), 4)];
        debug_assert!(arrays.start.len() >= ord.len());
        debug_assert!(arrays.end.len() >= ord.len());
        debug_assert!(arrays.freq.len() >= ord.len());
        ord
    }

    fn grow(&mut self, mut bytes_start: Vec<i32>) -> Vec<i32> {
        // `super.grow()`: `ArrayUtil.grow(bytesStart, bytesStart.length + 1)`.
        let min_size = bytes_start.len() + 1;
        bytes_start.resize(ArrayUtil::oversize(min_size, 4), 0);
        let ord = bytes_start;
        let mut arrays = lock(&self.arrays);
        if arrays.start.len() < ord.len() {
            // `ArrayUtil.grow(array, ord.length)`.
            let target = ArrayUtil::oversize(ord.len(), 4);
            arrays.start.resize(target, 0);
            arrays.end.resize(target, 0);
            arrays.freq.resize(target, 0);
        }
        debug_assert!(arrays.start.len() >= ord.len());
        debug_assert!(arrays.end.len() >= ord.len());
        debug_assert!(arrays.freq.len() >= ord.len());
        ord
    }

    fn clear(&mut self) -> Option<Vec<i32>> {
        let mut arrays = lock(&self.arrays);
        // `start = end = null;`
        arrays.start = Vec::new();
        arrays.end = Vec::new();
        // `super.clear()`.
        None
    }

    fn bytes_used(&self) -> Option<Arc<AtomicI64>> {
        Some(Arc::clone(&self.bytes_used))
    }
}

// ---------------------------------------------------------------------------
// Doc values producers
// ---------------------------------------------------------------------------

/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.BinaryDocValuesProducer`.
#[derive(Debug, Default)]
pub(crate) struct BinaryDocValuesProducer {
    dv_bytes_values_set: Option<BytesRef>,
}

/// Equivalent to
/// `org.apache.lucene.index.memory.MemoryIndex.BytesRefHashDocValuesProducer`.
#[derive(Debug, Default)]
pub(crate) struct BytesRefHashDocValuesProducer {
    dv_bytes_ref_hash_values_set: Option<BytesRefHash>,
    bytes_ids: Vec<i32>,
}

impl BytesRefHashDocValuesProducer {
    /// Equivalent to `prepareForUsage()`.
    fn prepare_for_usage(&mut self, pool: &ByteBlockPool) {
        let hash = self
            .dv_bytes_ref_hash_values_set
            .as_mut()
            .expect("INVARIANT: a SORTED_SET field has its hash");
        self.bytes_ids = hash.sort(pool).to_vec();
    }
}

/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.NumericDocValuesProducer`.
#[derive(Debug, Default)]
pub(crate) struct NumericDocValuesProducer {
    dv_long_values: Option<Vec<i64>>,
    count: usize,
}

impl NumericDocValuesProducer {
    /// Equivalent to `prepareForUsage()`.
    fn prepare_for_usage(&mut self) {
        if let Some(values) = self.dv_long_values.as_mut() {
            values[..self.count].sort_unstable();
        }
    }
}

// ---------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------

/// A value [`MemoryIndex::store_values`] keeps: Java's `List<Object>` holds
/// `BytesRef`, `Number` or `String` instances.
#[derive(Debug, Clone)]
enum StoredValue {
    Bytes(BytesRef),
    Number(NumericValue),
    String(String),
}

/// Index data structure for a field; contains the tokenized term texts and
/// their positions.
///
/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.Info`.
#[derive(Debug)]
pub(crate) struct Info {
    field_info: Arc<FieldInfo>,
    norm: Option<i64>,
    /// Term strings and their positions for this field.
    terms: BytesRefHash,
    slice_array: Arc<Mutex<SliceArrays>>,
    /// Terms sorted ascending by term text; computed on demand.
    sorted_terms: Option<Vec<i32>>,
    /// Number of added tokens for this field.
    num_tokens: i32,
    /// Number of overlapping tokens for this field.
    num_overlap_tokens: i32,
    sum_total_term_freq: i64,
    max_term_frequency: i32,
    /// The last position encountered in this field for multi field support.
    last_position: i32,
    /// The last offset encountered in this field for multi field support.
    last_offset: i32,
    bytes_ref_hash_producer: BytesRefHashDocValuesProducer,
    binary_producer: BinaryDocValuesProducer,
    numeric_producer: NumericDocValuesProducer,
    prepared_doc_values_and_point_values: bool,
    stored_values: Option<Vec<StoredValue>>,
    point_values: Option<Vec<BytesRef>>,
    float_vector_count: i32,
    float_vector_values: Option<Vec<Vec<f32>>>,
    byte_vector_count: i32,
    byte_vector_values: Option<Vec<Vec<u8>>>,
    min_packed_value: Option<Vec<u8>>,
    max_packed_value: Option<Vec<u8>>,
    point_values_count: usize,
}

impl Info {
    /// Equivalent to `Info(FieldInfo, ByteBlockPool)`.
    fn new(field_info: FieldInfo) -> Self {
        let slice_array = Arc::new(Mutex::new(SliceArrays::default()));
        let terms = BytesRefHash::with_bytes_start_array(
            DEFAULT_CAPACITY,
            Box::new(SliceByteStartArray::new(
                DEFAULT_CAPACITY,
                Arc::clone(&slice_array),
            )),
        );
        Self {
            field_info: Arc::new(field_info),
            norm: None,
            terms,
            slice_array,
            sorted_terms: None,
            num_tokens: 0,
            num_overlap_tokens: 0,
            sum_total_term_freq: 0,
            max_term_frequency: 0,
            last_position: 0,
            last_offset: 0,
            bytes_ref_hash_producer: BytesRefHashDocValuesProducer::default(),
            binary_producer: BinaryDocValuesProducer::default(),
            numeric_producer: NumericDocValuesProducer::default(),
            prepared_doc_values_and_point_values: false,
            stored_values: None,
            point_values: None,
            float_vector_count: 0,
            float_vector_values: None,
            byte_vector_count: 0,
            byte_vector_values: None,
            min_packed_value: None,
            max_packed_value: None,
            point_values_count: 0,
        }
    }

    /// Equivalent to `Info.freeze()`.
    fn freeze(&mut self, pools: &Pools, similarity: &dyn Similarity) -> Result<()> {
        self.sort_terms(pools);
        self.prepare_doc_values_and_point_values(pools);
        self.get_norm_doc_values(similarity)?;
        Ok(())
    }

    /// Sorts hashed terms into ascending order, reusing memory along the way.
    /// Note that sorting is lazily delayed until required (often it's not
    /// required at all).
    ///
    /// Equivalent to `Info.sortTerms()`.
    fn sort_terms(&mut self, pools: &Pools) {
        if self.sorted_terms.is_none() {
            let count = self.terms.size() as usize;
            let sorted = self.terms.sort(&pools.byte_block_pool);
            self.sorted_terms = Some(sorted[..count.min(sorted.len())].to_vec());
        }
    }

    fn sorted(&self) -> &[i32] {
        self.sorted_terms
            .as_deref()
            .expect("INVARIANT: sortTerms() ran before the sorted terms are read")
    }

    /// Equivalent to `Info.prepareDocValuesAndPointValues()`.
    fn prepare_doc_values_and_point_values(&mut self, pools: &Pools) {
        if !self.prepared_doc_values_and_point_values {
            let dv_type = self.field_info.get_doc_values_type();
            if dv_type == DocValuesType::NUMERIC || dv_type == DocValuesType::SORTED_NUMERIC {
                self.numeric_producer.prepare_for_usage();
            }
            if dv_type == DocValuesType::SORTED_SET {
                self.bytes_ref_hash_producer
                    .prepare_for_usage(&pools.byte_block_pool);
            }
            if let Some(point_values) = self.point_values.as_mut() {
                let num_dimensions = self.field_info.get_point_dimension_count() as usize;
                let num_bytes_per_dimension = self.field_info.get_point_num_bytes() as usize;
                let count = self.point_values_count;
                if num_dimensions == 1 {
                    // PointInSetQuery.MergePointVisitor expects values to be visited in
                    // increasing order, this is a 1d optimization which has to be done here
                    // too. Otherwise we emit values out of order which causes mismatches.
                    point_values[..count].sort_by(|a, b| a.slice().cmp(b.slice()));
                    self.min_packed_value = Some(point_values[0].slice().to_vec());
                    self.max_packed_value = Some(point_values[count - 1].slice().to_vec());
                } else {
                    let mut min = point_values[0].slice().to_vec();
                    let mut max = point_values[0].slice().to_vec();
                    for point_value in &point_values[..count] {
                        let bytes = point_value.slice();
                        for dim in 0..num_dimensions {
                            let offset = dim * num_bytes_per_dimension;
                            let range = offset..offset + num_bytes_per_dimension;
                            if bytes[range.clone()] < min[range.clone()] {
                                min[range.clone()].copy_from_slice(&bytes[range.clone()]);
                            }
                            if bytes[range.clone()] > max[range.clone()] {
                                max[range.clone()].copy_from_slice(&bytes[range]);
                            }
                        }
                    }
                    self.min_packed_value = Some(min);
                    self.max_packed_value = Some(max);
                }
            }
            self.prepared_doc_values_and_point_values = true;
        }
    }

    /// Equivalent to `Info.getNormDocValues()`: computes (once) and caches the
    /// norm, and returns a numeric doc values over it.
    fn get_norm_doc_values(
        &mut self,
        similarity: &dyn Similarity,
    ) -> Result<MemoryNumericDocValues> {
        if self.norm.is_none() {
            let invert_state = FieldInvertState::with_statistics(
                Version::LATEST.major as i32,
                self.field_info.name.clone(),
                self.field_info.get_index_options(),
                self.last_position,
                self.num_tokens,
                self.num_overlap_tokens,
                0,
                self.max_term_frequency,
                self.terms.size(),
            );
            let value = similarity.compute_norm(&invert_state)?;
            self.norm = Some(value);
        }
        Ok(numeric_doc_values(self.norm.expect("INVARIANT: set above")))
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// The pools every field of a [`MemoryIndex`] writes into.
#[derive(Debug)]
struct Pools {
    byte_block_pool: ByteBlockPool,
    sliced_int_block_pool: SlicedIntBlockPool,
    postings_writer: SliceWriter,
    /// Non-`None` only when `storePayloads`.
    payloads_bytes_refs: Option<BytesRefArray>,
}

/// The mutable fields of a [`MemoryIndex`] other than its pools.
#[derive(Debug)]
struct State {
    /// Info for each field: Map&lt;String fieldName, Info field&gt;.
    fields: BTreeMap<String, Arc<Mutex<Info>>>,
    frozen: bool,
    norm_similarity: Arc<dyn Similarity>,
    default_field_type: FieldType,
}

/// Everything a `MemoryIndex` and the objects of its reader share.
#[derive(Debug)]
struct Shared {
    store_offsets: bool,
    store_payloads: bool,
    #[allow(dead_code)]
    bytes_used: Arc<Counter>,
    state: Mutex<State>,
    pools: Mutex<Pools>,
}

// ---------------------------------------------------------------------------
// MemoryIndex
// ---------------------------------------------------------------------------

/// High-performance single-document main memory Apache Lucene fulltext
/// search index.
///
/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex`.
#[derive(Debug)]
pub struct MemoryIndex {
    shared: Arc<Shared>,
}

impl Default for MemoryIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryIndex {
    /// Constructs an empty instance that will not store offsets or payloads.
    ///
    /// Equivalent to `MemoryIndex()`.
    pub fn new() -> Self {
        Self::with_offsets(false)
    }

    /// Constructs an empty instance that can optionally store the start and
    /// end character offset of each token term in the text.
    ///
    /// Equivalent to `MemoryIndex(boolean)`.
    pub fn with_offsets(store_offsets: bool) -> Self {
        Self::with_offsets_and_payloads(store_offsets, false)
    }

    /// Constructs an empty instance with the option of storing offsets and
    /// payloads.
    ///
    /// Equivalent to `MemoryIndex(boolean, boolean)`.
    pub fn with_offsets_and_payloads(store_offsets: bool, store_payloads: bool) -> Self {
        Self::with_max_reused_bytes(store_offsets, store_payloads, 0)
    }

    /// Expert: this constructor accepts an upper limit for the number of bytes
    /// that should be reused if this instance is [`reset`](Self::reset).
    ///
    /// Equivalent to the package-private `MemoryIndex(boolean, boolean,
    /// long)`, public here so that the module's tests, which sit in Java's
    /// package, can reach it from the test crate.
    pub fn with_max_reused_bytes(
        store_offsets: bool,
        store_payloads: bool,
        max_reused_bytes: i64,
    ) -> Self {
        let default_field_type = FieldType::new();
        default_field_type
            .set_index_options(if store_offsets {
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
            } else {
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
            })
            .expect("INVARIANT: a new FieldType is not frozen");
        default_field_type
            .set_store_term_vectors(true)
            .expect("INVARIANT: a new FieldType is not frozen");
        let bytes_used = Arc::new(Counter::new_counter());
        let max_buffered_byte_blocks = ((max_reused_bytes / 2) / BYTE_BLOCK_SIZE as i64) as i32;
        let max_buffered_int_blocks = ((max_reused_bytes
            - (max_buffered_byte_blocks as i64 * BYTE_BLOCK_SIZE as i64))
            / (INT_BLOCK_SIZE as i64 * 4)) as i32;
        debug_assert!(
            (max_buffered_byte_blocks as i64 * BYTE_BLOCK_SIZE as i64)
                + (max_buffered_int_blocks as i64 * INT_BLOCK_SIZE as i64 * 4)
                <= max_reused_bytes
        );
        let byte_block_pool = ByteBlockPool::with_allocator(Box::new(RecyclingByteAllocator(
            Mutex::new(RecyclingByteBlockAllocator::with_counter(
                max_buffered_byte_blocks as usize,
                Arc::clone(&bytes_used),
            )),
        )));
        let sliced_int_block_pool =
            SlicedIntBlockPool::new(Box::new(RecyclingIntBlockAllocator::with_counter(
                INT_BLOCK_SIZE,
                max_buffered_int_blocks as usize,
                Arc::clone(&bytes_used),
            )));
        // TODO refactor BytesRefArray to allow us to apply maxReusedBytes option
        let payloads_bytes_refs = if store_payloads {
            Some(BytesRefArray::new(Arc::clone(&bytes_used)))
        } else {
            None
        };
        Self {
            shared: Arc::new(Shared {
                store_offsets,
                store_payloads,
                bytes_used,
                state: Mutex::new(State {
                    fields: BTreeMap::new(),
                    frozen: false,
                    norm_similarity: IndexSearcher::get_default_similarity(),
                    default_field_type,
                }),
                pools: Mutex::new(Pools {
                    byte_block_pool,
                    sliced_int_block_pool,
                    postings_writer: SliceWriter::default(),
                    payloads_bytes_refs,
                }),
            }),
        }
    }

    /// Convenience method; tokenizes the given field text and adds the
    /// resulting terms to the index; equivalent to adding an indexed
    /// non-keyword Lucene field that is tokenized, not stored, termVectorStored
    /// with positions (or termVectorStored with positions and offsets).
    ///
    /// Equivalent to `addField(String, String, Analyzer)`. Java's `null`
    /// checks on `fieldName`, `text` and `analyzer` are carried by the types.
    pub fn add_field_text(
        &mut self,
        field_name: &str,
        text: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<()> {
        let stream = analyzer.token_stream_from_str(field_name, text)?;
        let mut stream = stream.borrow_mut();
        let info = self.get_info(field_name, None)?;
        self.store_terms(
            &info,
            &mut *stream,
            analyzer.get_position_increment_gap(Some(field_name)),
            analyzer.get_offset_gap(Some(field_name)),
        )
    }

    /// Builds a MemoryIndex from a lucene document.
    ///
    /// Equivalent to `fromDocument(Iterable, Analyzer)`.
    pub fn from_document<'a>(
        document: impl IntoIterator<Item = &'a dyn IndexableField>,
        analyzer: Option<&dyn Analyzer>,
    ) -> Result<MemoryIndex> {
        Self::from_document_with_max_reused_bytes(document, analyzer, false, false, 0)
    }

    /// Builds a MemoryIndex from a lucene document.
    ///
    /// Equivalent to `fromDocument(Iterable, Analyzer, boolean, boolean)`.
    pub fn from_document_with_options<'a>(
        document: impl IntoIterator<Item = &'a dyn IndexableField>,
        analyzer: Option<&dyn Analyzer>,
        store_offsets: bool,
        store_payloads: bool,
    ) -> Result<MemoryIndex> {
        Self::from_document_with_max_reused_bytes(
            document,
            analyzer,
            store_offsets,
            store_payloads,
            0,
        )
    }

    /// Builds a MemoryIndex from a lucene document.
    ///
    /// Equivalent to `fromDocument(Iterable, Analyzer, boolean, boolean, long)`.
    pub fn from_document_with_max_reused_bytes<'a>(
        document: impl IntoIterator<Item = &'a dyn IndexableField>,
        analyzer: Option<&dyn Analyzer>,
        store_offsets: bool,
        store_payloads: bool,
        max_reused_bytes: i64,
    ) -> Result<MemoryIndex> {
        let mut mi =
            MemoryIndex::with_max_reused_bytes(store_offsets, store_payloads, max_reused_bytes);
        for field in document {
            mi.add_field(field, analyzer)?;
        }
        Ok(mi)
    }

    /// Convenience method; Creates and returns a token stream that generates a
    /// token for each keyword in the given collection, "as is", without any
    /// transforming text analysis.
    ///
    /// Equivalent to `keywordTokenStream(Collection)`. A `None` keyword is
    /// Java's `null` element, rejected with `IllegalArgumentException("keyword
    /// must not be null")` when the stream reaches it.
    pub fn keyword_token_stream<T: fmt::Display>(
        &self,
        keywords: impl IntoIterator<Item = Option<T>>,
    ) -> Result<Box<dyn TokenStream>> {
        let keywords: Vec<Option<String>> = keywords
            .into_iter()
            .map(|keyword| keyword.map(|k| k.to_string()))
            .collect();
        Ok(Box::new(KeywordTokenStream::new(keywords)?))
    }

    /// Adds a lucene [`IndexableField`] to the MemoryIndex using the provided
    /// analyzer. Also stores doc values based on
    /// [`IndexableFieldType::doc_values_type`] if set.
    ///
    /// Equivalent to `addField(IndexableField, Analyzer)`.
    pub fn add_field(
        &mut self,
        field: &dyn IndexableField,
        analyzer: Option<&dyn Analyzer>,
    ) -> Result<()> {
        let info = self.get_info(field.name(), Some(field.field_type()))?;

        let offset_gap;
        let position_increment_gap;
        // `field.tokenStream(analyzer, null)`, which may return `null`.
        let token_stream = field.token_stream_or_null(analyzer, &mut None)?;
        if let Some(analyzer) = analyzer {
            offset_gap = analyzer.get_offset_gap(Some(field.name()));
            position_increment_gap = analyzer.get_position_increment_gap(Some(field.name()));
        } else {
            offset_gap = 1;
            position_increment_gap = 0;
        }
        if let Some(mut token_stream) = token_stream {
            self.store_terms(
                &info,
                &mut *token_stream,
                position_increment_gap,
                offset_gap,
            )?;
        } else if field
            .field_type()
            .index_options()
            .subsumes(IndexOptions::DOCS)
        {
            let Some(binary_value) = field.binary_value() else {
                return Err(LuceneError::illegal_argument(
                    "Indexed field must provide a TokenStream or a binary value",
                ));
            };
            self.store_term(&info, &binary_value)?;
        }

        let doc_values_type = field.field_type().doc_values_type();
        let doc_values_value = match doc_values_type {
            DocValuesType::NONE => None,
            DocValuesType::BINARY | DocValuesType::SORTED | DocValuesType::SORTED_SET => {
                field.binary_value().map(DocValuesValue::Bytes)
            }
            DocValuesType::NUMERIC | DocValuesType::SORTED_NUMERIC => {
                field.numeric_value().map(DocValuesValue::Number)
            }
        };
        if let Some(doc_values_value) = doc_values_value {
            self.store_doc_values(&info, doc_values_type, doc_values_value)?;
        }

        if field.field_type().point_dimension_count() > 0 {
            let value = field.binary_value().ok_or_else(LuceneError::null_pointer)?;
            Self::store_point_values(&info, &value);
        }

        if field.field_type().stored() {
            Self::store_values(&info, field);
        }

        if field.field_type().vector_dimension() > 0 {
            Self::store_vector_values(&info, field)?;
        }
        Ok(())
    }

    /// Iterates over the given token stream and adds the resulting terms to
    /// the index; equivalent to adding a tokenized, indexed, termVectorStored,
    /// unstored, Lucene field.
    ///
    /// Equivalent to `addField(String, TokenStream)`.
    pub fn add_field_token_stream(
        &mut self,
        field_name: &str,
        stream: &mut dyn TokenStream,
    ) -> Result<()> {
        self.add_field_token_stream_with_gaps(field_name, stream, 0, 1)
    }

    /// Equivalent to `addField(String, TokenStream, int)`.
    pub fn add_field_token_stream_with_position_gap(
        &mut self,
        field_name: &str,
        stream: &mut dyn TokenStream,
        position_increment_gap: i32,
    ) -> Result<()> {
        self.add_field_token_stream_with_gaps(field_name, stream, position_increment_gap, 1)
    }

    /// Iterates over the given token stream and adds the resulting terms to
    /// the index.
    ///
    /// Equivalent to `addField(String, TokenStream, int, int)`. The stream is
    /// closed afterwards, as Java's try-with-resources closes it.
    pub fn add_field_token_stream_with_gaps(
        &mut self,
        field_name: &str,
        token_stream: &mut dyn TokenStream,
        position_increment_gap: i32,
        offset_gap: i32,
    ) -> Result<()> {
        let info = self.get_info(field_name, None)?;
        self.store_terms(&info, token_stream, position_increment_gap, offset_gap)
    }

    /// Equivalent to `getInfo(String, IndexableFieldType)`; `None` stands for
    /// the index's `defaultFieldType`.
    fn get_info(
        &self,
        field_name: &str,
        field_type: Option<&dyn IndexableFieldType>,
    ) -> Result<Arc<Mutex<Info>>> {
        let mut state = lock(&self.shared.state);
        if state.frozen {
            return Err(LuceneError::illegal_argument(
                "Cannot call addField() when MemoryIndex is frozen",
            ));
        }
        let default_field_type = state.default_field_type.clone();
        let field_type: &dyn IndexableFieldType = match field_type {
            Some(field_type) => field_type,
            None => &default_field_type,
        };
        let info = match state.fields.get(field_name) {
            Some(info) => Arc::clone(info),
            None => {
                let ord = state.fields.len() as i32;
                let info = Arc::new(Mutex::new(Info::new(
                    self.create_field_info(field_name, ord, field_type)?,
                )));
                state
                    .fields
                    .insert(field_name.to_string(), Arc::clone(&info));
                info
            }
        };
        // **Divergence.** Java mutates `info.fieldInfo` in place
        // (`MemoryIndex.java:713-724`), so a `MemoryIndexReader` built
        // earlier, whose `FieldInfos` holds the same object, sees the new
        // point dimensions / doc values type. Core `FieldInfo`'s setters take
        // `&mut self`; while a reader shares the object, `Arc::make_mut`
        // copies it, and that reader keeps the previous values. A reader
        // created after this call shares the updated object.
        {
            let mut guard = lock(&info);
            if field_type.point_dimension_count() != guard.field_info.get_point_dimension_count()
                && field_type.point_dimension_count() > 0
            {
                Arc::make_mut(&mut guard.field_info).set_point_dimensions(
                    field_type.point_dimension_count(),
                    field_type.point_index_dimension_count(),
                    field_type.point_num_bytes(),
                )?;
            }
            if field_type.doc_values_type() != guard.field_info.get_doc_values_type()
                && field_type.doc_values_type() != DocValuesType::NONE
            {
                Arc::make_mut(&mut guard.field_info)
                    .set_doc_values_type(field_type.doc_values_type())?;
            }
        }
        Ok(info)
    }

    /// Equivalent to `createFieldInfo(String, int, IndexableFieldType)`.
    fn create_field_info(
        &self,
        field_name: &str,
        ord: i32,
        field_type: &dyn IndexableFieldType,
    ) -> Result<FieldInfo> {
        let index_options = if self.shared.store_offsets {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
        } else {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
        };
        FieldInfo::new_full(
            field_name,
            ord,
            field_type.store_term_vectors(),
            field_type.omit_norms(),
            self.shared.store_payloads,
            index_options,
            field_type.doc_values_type(),
            field_type.doc_values_skip_index_type(),
            -1,
            Default::default(),
            field_type.point_dimension_count(),
            field_type.point_index_dimension_count(),
            field_type.point_num_bytes(),
            field_type.vector_dimension(),
            field_type.vector_encoding(),
            field_type.vector_similarity_function(),
            false,
            false,
        )
    }

    /// Equivalent to `storePointValues(Info, BytesRef)`.
    fn store_point_values(info: &Mutex<Info>, point_value: &BytesRef) {
        let mut info = lock(info);
        let count = info.point_values_count;
        let values = info
            .point_values
            .get_or_insert_with(|| Vec::with_capacity(4));
        // `ArrayUtil.grow(info.pointValues, count + 1)`.
        if values.len() < count + 1 {
            values.resize(count + 1, BytesRef::default());
        }
        values[count] = BytesRef::deep_copy_of(point_value);
        info.point_values_count += 1;
    }

    /// Equivalent to `storeVectorValues(Info, IndexableField)`. Java's
    /// `instanceof KnnByteVectorField` / `KnnFloatVectorField` tests are the
    /// presence of the field's byte or float vector value.
    fn store_vector_values(info: &Mutex<Info>, vector_field: &dyn IndexableField) -> Result<()> {
        let mut info = lock(info);
        let dimension = info.field_info.get_vector_dimension() as usize;
        match info.field_info.get_vector_encoding() {
            VectorEncoding::BYTE => {
                if let Some(value) = vector_field.byte_vector_value() {
                    if info.byte_vector_count == 1 {
                        return Err(LuceneError::illegal_argument(format!(
                            "Only one value per field allowed for byte vector field [{}]",
                            vector_field.name()
                        )));
                    }
                    info.byte_vector_count += 1;
                    let copy = value[..dimension].to_vec();
                    info.byte_vector_values
                        .get_or_insert_with(|| vec![Vec::new()])[0] = copy;
                    return Ok(());
                }
                Err(LuceneError::illegal_argument(format!(
                    "Field [{}] is not a byte vector field, but the field info is configured for byte vectors",
                    vector_field.name()
                )))
            }
            VectorEncoding::FLOAT32 => {
                if let Some(value) = vector_field.float_vector_value() {
                    if info.float_vector_count == 1 {
                        return Err(LuceneError::illegal_argument(format!(
                            "Only one value per field allowed for float vector field [{}]",
                            vector_field.name()
                        )));
                    }
                    info.float_vector_count += 1;
                    let copy = value[..dimension].to_vec();
                    info.float_vector_values
                        .get_or_insert_with(|| vec![Vec::new()])[0] = copy;
                    return Ok(());
                }
                Err(LuceneError::illegal_argument(format!(
                    "Field [{}] is not a float vector field, but the field info is configured for float vectors",
                    vector_field.name()
                )))
            }
        }
    }

    /// Equivalent to `storeValues(Info, IndexableField)`.
    fn store_values(info: &Mutex<Info>, field: &dyn IndexableField) {
        let mut info = lock(info);
        let stored = info.stored_values.get_or_insert_with(Vec::new);
        if let Some(binary_value) = field.binary_value() {
            stored.push(StoredValue::Bytes(binary_value));
            return;
        }
        if let Some(number_value) = field.numeric_value() {
            stored.push(StoredValue::Number(number_value));
            return;
        }
        if let Some(string_value) = field.string_value() {
            stored.push(StoredValue::String(string_value));
        }
    }

    /// Equivalent to `storeDocValues(Info, DocValuesType, Object)`.
    fn store_doc_values(
        &self,
        info: &Mutex<Info>,
        doc_values_type: DocValuesType,
        doc_values_value: DocValuesValue,
    ) -> Result<()> {
        let mut info = lock(info);
        let field_name = info.field_info.name.clone();
        let existing_doc_values_type = info.field_info.get_doc_values_type();
        if existing_doc_values_type == DocValuesType::NONE {
            // first time we add doc values for this field:
            let fi = &info.field_info;
            // Java passes `hasPayloads()` as the `omitNorms` argument
            // (`MemoryIndex.java:548-549`); kept as is.
            let new_info = FieldInfo::new_full(
                fi.name.clone(),
                fi.number,
                fi.has_term_vectors(),
                fi.has_payloads(),
                fi.has_payloads(),
                fi.get_index_options(),
                doc_values_type,
                DocValuesSkipIndexType::NONE,
                -1,
                fi.attributes(),
                fi.get_point_dimension_count(),
                fi.get_point_index_dimension_count(),
                fi.get_point_num_bytes(),
                fi.get_vector_dimension(),
                fi.get_vector_encoding(),
                fi.get_vector_similarity_function(),
                fi.is_soft_deletes_field(),
                fi.is_parent_field(),
            )?;
            info.field_info = Arc::new(new_info);
        } else if existing_doc_values_type != doc_values_type {
            return Err(LuceneError::illegal_argument(format!(
                "Can't add [{doc_values_type:?}] doc values field [{field_name}], because [{existing_doc_values_type:?}] doc values field already exists"
            )));
        }
        let only_one = || {
            LuceneError::illegal_argument(format!(
                "Only one value per field allowed for [{doc_values_type:?}] doc values field [{field_name}]"
            ))
        };
        match doc_values_type {
            DocValuesType::NUMERIC => {
                if info.numeric_producer.dv_long_values.is_some() {
                    return Err(only_one());
                }
                let DocValuesValue::Number(value) = doc_values_value else {
                    return Err(LuceneError::unchecked(
                        "java.lang.ClassCastException",
                        "class org.apache.lucene.util.BytesRef cannot be cast to class java.lang.Number",
                    ));
                };
                info.numeric_producer.dv_long_values = Some(vec![number_long_value(&value)]);
                info.numeric_producer.count += 1;
            }
            DocValuesType::SORTED_NUMERIC => {
                let DocValuesValue::Number(value) = doc_values_value else {
                    return Err(LuceneError::unchecked(
                        "java.lang.ClassCastException",
                        "class org.apache.lucene.util.BytesRef cannot be cast to class java.lang.Number",
                    ));
                };
                let count = info.numeric_producer.count;
                let values = info
                    .numeric_producer
                    .dv_long_values
                    .get_or_insert_with(|| vec![0; 4]);
                if values.len() < count + 1 {
                    // `ArrayUtil.grow(values, count + 1)`.
                    values.resize(ArrayUtil::oversize(count + 1, 8), 0);
                }
                values[count] = number_long_value(&value);
                info.numeric_producer.count += 1;
            }
            DocValuesType::BINARY | DocValuesType::SORTED => {
                if info.binary_producer.dv_bytes_values_set.is_some() {
                    return Err(only_one());
                }
                let DocValuesValue::Bytes(value) = doc_values_value else {
                    return Err(LuceneError::unchecked(
                        "java.lang.ClassCastException",
                        "class java.lang.Number cannot be cast to class org.apache.lucene.util.BytesRef",
                    ));
                };
                info.binary_producer.dv_bytes_values_set = Some(value.clone());
            }
            DocValuesType::SORTED_SET => {
                let DocValuesValue::Bytes(value) = doc_values_value else {
                    return Err(LuceneError::unchecked(
                        "java.lang.ClassCastException",
                        "class java.lang.Number cannot be cast to class org.apache.lucene.util.BytesRef",
                    ));
                };
                let mut pools = lock(&self.shared.pools);
                let hash = info
                    .bytes_ref_hash_producer
                    .dv_bytes_ref_hash_values_set
                    .get_or_insert_with(|| BytesRefHash::new(Arc::new(AtomicI64::new(0))));
                hash.add(&mut pools.byte_block_pool, value.slice())?;
            }
            DocValuesType::NONE => {
                return Err(LuceneError::unsupported_operation(format!(
                    "unknown doc values type [{doc_values_type:?}]"
                )));
            }
        }
        Ok(())
    }

    /// Equivalent to `storeTerm(Info, BytesRef)`.
    fn store_term(&self, info: &Mutex<Info>, term: &BytesRef) -> Result<()> {
        let mut info = lock(info);
        let mut pools = lock(&self.shared.pools);
        let pools = &mut *pools;
        info.num_tokens += 1;
        let mut ord = info.terms.add(&mut pools.byte_block_pool, term.slice())?;
        let slices = Arc::clone(&info.slice_array);
        let mut slices = lock(&slices);
        if ord < 0 {
            ord = -ord - 1;
            pools.postings_writer.reset(slices.end[ord as usize]);
        } else {
            slices.start[ord as usize] = pools
                .postings_writer
                .start_new_slice(&mut pools.sliced_int_block_pool)?;
        }
        let ord = ord as usize;
        slices.freq[ord] += 1;
        info.max_term_frequency = info.max_term_frequency.max(slices.freq[ord]);
        info.sum_total_term_freq += 1;
        let position = info.last_position;
        info.last_position += 1;
        pools
            .postings_writer
            .write_int(&mut pools.sliced_int_block_pool, position)?; // fake position
        if self.shared.store_offsets {
            // fake offsests
            pools
                .postings_writer
                .write_int(&mut pools.sliced_int_block_pool, 0)?;
            pools
                .postings_writer
                .write_int(&mut pools.sliced_int_block_pool, 0)?;
        }
        if self.shared.store_payloads {
            pools
                .postings_writer
                .write_int(&mut pools.sliced_int_block_pool, -1)?; // fake payload
        }
        slices.end[ord] = pools.postings_writer.get_current_offset();
        Ok(())
    }

    /// Equivalent to `storeTerms(Info, TokenStream, int, int)`. The stream is
    /// closed when this returns, as Java's try-with-resources closes it.
    fn store_terms(
        &self,
        info: &Mutex<Info>,
        stream: &mut dyn TokenStream,
        position_increment_gap: i32,
        offset_gap: i32,
    ) -> Result<()> {
        let result = self.store_terms_inner(info, stream, position_increment_gap, offset_gap);
        let closed = stream.close();
        result?;
        closed
    }

    fn store_terms_inner(
        &self,
        info: &Mutex<Info>,
        stream: &mut dyn TokenStream,
        position_increment_gap: i32,
        offset_gap: i32,
    ) -> Result<()> {
        let mut info = lock(info);
        let mut pos = -1;
        let mut offset = 0;
        if info.num_tokens > 0 {
            pos = info.last_position + position_increment_gap;
            offset = info.last_offset + offset_gap;
        }

        let term_att: Option<AttributeHandle<dyn TermToBytesRefAttribute>> =
            AttributeHandle::get(stream.attribute_source());
        let pos_incr_attribute: AttributeHandle<dyn PositionIncrementAttribute> =
            AttributeHandle::add(stream.attribute_source_mut())?;
        let offset_att: AttributeHandle<dyn OffsetAttribute> =
            AttributeHandle::add(stream.attribute_source_mut())?;
        let payload_att: Option<AttributeHandle<dyn PayloadAttribute>> =
            if self.shared.store_payloads {
                Some(AttributeHandle::add(stream.attribute_source_mut())?)
            } else {
                None
            };
        stream.reset()?;

        let mut term = BytesRef::default();
        let slices_cell = Arc::clone(&info.slice_array);
        while stream.increment_token()? {
            info.num_tokens += 1;
            let pos_incr = pos_incr_attribute.borrow().get_position_increment();
            if pos_incr == 0 {
                info.num_overlap_tokens += 1;
            }
            pos += pos_incr;
            term_att
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?
                .borrow()
                .copy_bytes_ref_to(&mut term);
            let mut pools = lock(&self.shared.pools);
            let pools = &mut *pools;
            let mut ord = info.terms.add(&mut pools.byte_block_pool, term.slice())?;
            let mut slices = lock(&slices_cell);
            if ord < 0 {
                ord = -ord - 1;
                pools.postings_writer.reset(slices.end[ord as usize]);
            } else {
                slices.start[ord as usize] = pools
                    .postings_writer
                    .start_new_slice(&mut pools.sliced_int_block_pool)?;
            }
            let ord = ord as usize;
            slices.freq[ord] += 1;
            info.max_term_frequency = info.max_term_frequency.max(slices.freq[ord]);
            info.sum_total_term_freq += 1;
            pools
                .postings_writer
                .write_int(&mut pools.sliced_int_block_pool, pos)?;
            if self.shared.store_offsets {
                let (start, end) = {
                    let att = offset_att.borrow();
                    (att.start_offset(), att.end_offset())
                };
                pools
                    .postings_writer
                    .write_int(&mut pools.sliced_int_block_pool, start + offset)?;
                pools
                    .postings_writer
                    .write_int(&mut pools.sliced_int_block_pool, end + offset)?;
            }
            if let Some(payload_att) = &payload_att {
                let payload = payload_att.borrow().get_payload().cloned();
                let p_index = match payload {
                    Some(payload) if payload.length != 0 => pools
                        .payloads_bytes_refs
                        .as_mut()
                        .expect("INVARIANT: payloads are stored")
                        .append(&payload)?
                        as i32,
                    _ => -1,
                };
                pools
                    .postings_writer
                    .write_int(&mut pools.sliced_int_block_pool, p_index)?;
            }
            slices.end[ord] = pools.postings_writer.get_current_offset();
        }
        stream.end()?;
        if info.num_tokens > 0 {
            info.last_position = pos;
            info.last_offset = offset_att.borrow().end_offset() + offset;
        }
        Ok(())
    }

    /// Set the Similarity to be used for calculating field norms.
    ///
    /// Equivalent to `setSimilarity(Similarity)`.
    pub fn set_similarity(&mut self, similarity: Arc<dyn Similarity>) -> Result<()> {
        let mut state = lock(&self.shared.state);
        if state.frozen {
            return Err(LuceneError::illegal_argument(
                "Cannot set Similarity when MemoryIndex is frozen",
            ));
        }
        if Arc::ptr_eq(&state.norm_similarity, &similarity) {
            return Ok(());
        }
        state.norm_similarity = similarity;
        // invalidate any cached norms that may exist
        for info in state.fields.values() {
            lock(info).norm = None;
        }
        Ok(())
    }

    /// Creates and returns a searcher that can be used to execute arbitrary
    /// Lucene queries and to collect the resulting query results as hits.
    ///
    /// Equivalent to `createSearcher()`.
    pub fn create_searcher(&self) -> Result<IndexSearcher> {
        let reader = MemoryIndexReader::new(Arc::clone(&self.shared));
        let similarity = Arc::clone(&lock(&self.shared.state).norm_similarity);
        let reader: Arc<dyn IndexReader> = Arc::new(reader);
        let mut searcher = IndexSearcher::new(reader)?; // ensures no auto-close !!
        searcher.set_similarity(similarity);
        searcher.set_query_cache(None);
        Ok(searcher)
    }

    /// Prepares the MemoryIndex for querying in a non-lazy way.
    ///
    /// Equivalent to `freeze()`.
    pub fn freeze(&mut self) -> Result<()> {
        let mut state = lock(&self.shared.state);
        state.frozen = true;
        let similarity = Arc::clone(&state.norm_similarity);
        for info in state.fields.values() {
            let mut info = lock(info);
            let pools = lock(&self.shared.pools);
            info.freeze(&pools, &*similarity)?;
        }
        Ok(())
    }

    /// Convenience method that efficiently returns the relevance score by
    /// matching this index against the given Lucene query expression.
    ///
    /// Equivalent to `search(Query)`.
    pub fn search(&self, query: Arc<dyn Query>) -> Result<f32> {
        let searcher = self.create_searcher()?;
        let manager = ScoreCollectorManager {
            scores: Arc::new(AtomicU32::new(0f32.to_bits())), // inits to 0.0f (no match)
        };
        searcher.search_with_manager(query, &manager)
    }

    /// Returns a String representation of the index data for debugging
    /// purposes.
    ///
    /// Equivalent to `toStringDebug()`.
    pub fn to_string_debug(&self) -> Result<String> {
        let mut result = String::with_capacity(256);
        let mut sum_positions = 0;
        let mut sum_terms = 0;
        let state = lock(&self.shared.state);
        for (field_name, info) in &state.fields {
            let mut info = lock(info);
            let pools = lock(&self.shared.pools);
            info.sort_terms(&pools);
            result.push_str(field_name);
            result.push_str(":\n");
            let slices = Arc::clone(&info.slice_array);
            let slices = lock(&slices);
            let mut num_positions = 0;
            let mut postings_reader = SliceReader::default();
            for j in 0..info.terms.size() as usize {
                let ord = info.sorted()[j];
                let spare = BytesRef::new(info.terms.get(&pools.byte_block_pool, ord).to_vec());
                let freq = slices.freq[ord as usize];
                result.push_str(&format!("\t'{spare}':{freq}:"));
                postings_reader.reset(slices.start[ord as usize], slices.end[ord as usize]);
                result.push_str(" [");
                let iters = if self.shared.store_offsets { 3 } else { 1 };
                while !postings_reader.end_of_slice() {
                    result.push('(');

                    for k in 0..iters {
                        result.push_str(
                            &postings_reader
                                .read_int(&pools.sliced_int_block_pool)
                                .to_string(),
                        );
                        if k < iters - 1 {
                            result.push_str(", ");
                        }
                    }
                    if self.shared.store_payloads {
                        let payload_index = postings_reader.read_int(&pools.sliced_int_block_pool);
                        if payload_index != -1 {
                            let payload = pools
                                .payloads_bytes_refs
                                .as_ref()
                                .expect("INVARIANT: payloads are stored")
                                .get(payload_index as usize)?;
                            result.push_str(&format!(", {payload}"));
                        }
                    }
                    result.push(')');

                    if !postings_reader.end_of_slice() {
                        result.push_str(", ");
                    }
                }
                result.push(']');
                result.push('\n');
                num_positions += freq;
            }

            result.push_str(&format!("\tterms={}", info.terms.size()));
            result.push_str(&format!(", positions={num_positions}"));
            result.push('\n');
            sum_positions += num_positions;
            sum_terms += info.terms.size();
        }

        result.push_str(&format!("\nfields={}", state.fields.len()));
        result.push_str(&format!(", terms={sum_terms}"));
        result.push_str(&format!(", positions={sum_positions}"));
        Ok(result)
    }

    /// Resets the MemoryIndex to its initial state and recycles all internal
    /// buffers.
    ///
    /// Equivalent to `reset()`.
    pub fn reset(&mut self) {
        let mut state = lock(&self.shared.state);
        state.fields.clear();
        state.norm_similarity = IndexSearcher::get_default_similarity();
        let mut pools = lock(&self.shared.pools);
        pools.byte_block_pool.reset(false, false); // no need to 0-fill the buffers
        pools.sliced_int_block_pool.reset(true, false); // here must must 0-fill since we use slices
        if let Some(payloads) = pools.payloads_bytes_refs.as_mut() {
            payloads.clear();
        }
        state.frozen = false;
    }
}

/// The doc values value `addField` reads off a field: Java's `Object
/// docValuesValue`, a `BytesRef` or a `Number`.
#[derive(Debug)]
enum DocValuesValue {
    Bytes(BytesRef),
    Number(NumericValue),
}

// ---------------------------------------------------------------------------
// keywordTokenStream
// ---------------------------------------------------------------------------

/// The anonymous `TokenStream` of `keywordTokenStream(Collection)`.
#[derive(Debug)]
struct KeywordTokenStream {
    source: AttributeSource,
    iter: std::vec::IntoIter<Option<String>>,
    start: i32,
}

impl KeywordTokenStream {
    fn new(keywords: Vec<Option<String>>) -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        source.add_attribute_interface::<dyn CharTermAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        Ok(Self {
            source,
            iter: keywords.into_iter(),
            start: 0,
        })
    }
}

impl TokenStream for KeywordTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        let Some(obj) = self.iter.next() else {
            return Ok(false);
        };
        let Some(term) = obj else {
            return Err(LuceneError::illegal_argument("keyword must not be null"));
        };
        self.source.clear_attributes();
        let length = {
            let mut term_att = self
                .source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: added by the constructor");
            term_att.set_empty();
            term_att.append_string(Some(&term));
            term_att.length() as i32
        };
        self.source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: added by the constructor")
            .set_offset(self.start, self.start + length);
        self.start += term.encode_utf16().count() as i32 + 1; // separate words by 1 (blank) character
        Ok(true)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

// ---------------------------------------------------------------------------
// search(Query) collector
// ---------------------------------------------------------------------------

/// The anonymous `CollectorManager` of `search(Query)`; `scores` is its
/// `float[1]`.
#[derive(Debug)]
struct ScoreCollectorManager {
    scores: Arc<AtomicU32>,
}

/// The anonymous `SimpleCollector` of `search(Query)`.
#[derive(Debug)]
struct ScoreCollector {
    scores: Arc<AtomicU32>,
}

impl LeafCollector for ScoreCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let score = scorer.score()?;
        self.scores.store(score.to_bits(), Ordering::Relaxed);
        Ok(())
    }
}

impl SimpleCollectorImpl for ScoreCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

impl CollectorManager for ScoreCollectorManager {
    type Collector = SimpleCollector<ScoreCollector>;
    type Output = f32;

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(ScoreCollector {
            scores: Arc::clone(&self.scores),
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<f32> {
        Ok(f32::from_bits(self.scores.load(Ordering::Relaxed)))
    }
}

// ---------------------------------------------------------------------------
// Doc values
// ---------------------------------------------------------------------------

/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.MemoryDocValuesIterator`.
#[derive(Debug, Clone)]
pub(crate) struct MemoryDocValuesIterator {
    doc: i32,
}

impl Default for MemoryDocValuesIterator {
    fn default() -> Self {
        Self { doc: -1 }
    }
}

impl MemoryDocValuesIterator {
    fn advance(&mut self, doc: i32) -> i32 {
        self.doc = doc;
        self.doc_id()
    }

    fn next_doc(&mut self) -> i32 {
        self.doc += 1;
        self.doc_id()
    }

    fn doc_id(&self) -> i32 {
        if self.doc > 0 {
            NO_MORE_DOCS
        } else {
            self.doc
        }
    }
}

macro_rules! memory_disi {
    ($ty:ty) => {
        impl DocIdSetIterator for $ty {
            fn doc_id(&self) -> i32 {
                self.it.doc_id()
            }

            fn next_doc(&mut self) -> Result<i32> {
                Ok(self.it.next_doc())
            }

            fn advance(&mut self, target: i32) -> Result<i32> {
                Ok(self.it.advance(target))
            }

            fn cost(&self) -> i64 {
                1
            }
        }
    };
}

/// The anonymous `SortedNumericDocValues` of `numericDocValues(long[], int)`.
#[derive(Debug)]
struct MemorySortedNumericDocValues {
    it: MemoryDocValuesIterator,
    values: Vec<i64>,
    count: usize,
    ord: usize,
}

memory_disi!(MemorySortedNumericDocValues);

impl DocValuesIterator for MemorySortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.ord = 0;
        Ok(self.it.advance(target) == target)
    }
}

impl SortedNumericDocValues for MemorySortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        let value = self.values[self.ord];
        self.ord += 1;
        Ok(value)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.count as i32)
    }
}

/// Equivalent to `numericDocValues(long[], int)`.
fn sorted_numeric_doc_values(values: Vec<i64>, count: usize) -> MemorySortedNumericDocValues {
    MemorySortedNumericDocValues {
        it: MemoryDocValuesIterator::default(),
        values,
        count,
        ord: 0,
    }
}

/// The anonymous `NumericDocValues` of `numericDocValues(long)`.
#[derive(Debug)]
pub(crate) struct MemoryNumericDocValues {
    it: MemoryDocValuesIterator,
    value: i64,
}

memory_disi!(MemoryNumericDocValues);

impl DocValuesIterator for MemoryNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        Ok(self.advance(target)? == target)
    }
}

impl NumericDocValues for MemoryNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        Ok(self.value)
    }
}

/// Equivalent to `numericDocValues(long)`.
fn numeric_doc_values(value: i64) -> MemoryNumericDocValues {
    MemoryNumericDocValues {
        it: MemoryDocValuesIterator::default(),
        value,
    }
}

/// The anonymous `SortedDocValues` of `sortedDocValues(BytesRef)`.
#[derive(Debug)]
struct MemorySortedDocValues {
    it: MemoryDocValuesIterator,
    value: BytesRef,
}

memory_disi!(MemorySortedDocValues);

impl DocValuesIterator for MemorySortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        Ok(self.it.advance(target) == target)
    }
}

impl SortedDocValues for MemorySortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(0)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(1)
    }

    fn lookup_ord(&mut self, _ord: i32) -> Result<&BytesRef> {
        Ok(&self.value)
    }
}

/// Equivalent to `sortedDocValues(BytesRef)`.
fn sorted_doc_values(value: BytesRef) -> MemorySortedDocValues {
    MemorySortedDocValues {
        it: MemoryDocValuesIterator::default(),
        value,
    }
}

/// The anonymous `BinaryDocValues` `getBinaryDocValues` wraps around a
/// `SortedDocValues` "and makes it look like its binary".
#[derive(Debug)]
struct MemoryBinaryDocValues {
    input: MemorySortedDocValues,
}

impl DocIdSetIterator for MemoryBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.input.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.input.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.input.advance(target)
    }

    fn cost(&self) -> i64 {
        self.input.cost()
    }
}

impl DocValuesIterator for MemoryBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.input.advance_exact(target)
    }
}

impl BinaryDocValues for MemoryBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        // `in.lookupOrd(in.ordValue())`: the single value.
        Ok(self.input.value.clone())
    }
}

/// The anonymous `SortedSetDocValues` of `sortedSetDocValues(BytesRefHash,
/// int[])`, reading the field's hash in place.
struct MemorySortedSetDocValues {
    it: MemoryDocValuesIterator,
    info: Arc<Mutex<Info>>,
    shared: Arc<Shared>,
    scratch: BytesRef,
    ord: i64,
}

impl fmt::Debug for MemorySortedSetDocValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemorySortedSetDocValues")
            .field("doc", &self.it.doc)
            .field("ord", &self.ord)
            .finish()
    }
}

memory_disi!(MemorySortedSetDocValues);

impl MemorySortedSetDocValues {
    fn size(&self) -> i32 {
        lock(&self.info)
            .bytes_ref_hash_producer
            .dv_bytes_ref_hash_values_set
            .as_ref()
            .map_or(0, BytesRefHash::size)
    }
}

impl DocValuesIterator for MemorySortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.ord = 0;
        Ok(self.it.advance(target) == target)
    }
}

impl SortedSetDocValues for MemorySortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        let ord = self.ord;
        self.ord += 1;
        Ok(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.size())
    }

    fn lookup_ord(&mut self, ord: i64) -> Result<&BytesRef> {
        let info = lock(&self.info);
        let pools = lock(&self.shared.pools);
        let producer = &info.bytes_ref_hash_producer;
        let hash = producer
            .dv_bytes_ref_hash_values_set
            .as_ref()
            .expect("INVARIANT: a SORTED_SET field has its hash");
        let bytes = hash.get(&pools.byte_block_pool, producer.bytes_ids[ord as usize]);
        self.scratch = BytesRef::new(bytes.to_vec());
        drop(pools);
        drop(info);
        Ok(&self.scratch)
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.size() as i64)
    }
}

// ---------------------------------------------------------------------------
// MemoryIndexReader
// ---------------------------------------------------------------------------

/// Search support for Lucene framework integration; implements all methods
/// required by the Lucene `LeafReader` contracts.
///
/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.MemoryIndexReader`.
pub(crate) struct MemoryIndexReader {
    core: IndexReaderCore,
    shared: Arc<Shared>,
    memory_fields: MemoryFields,
    field_infos: FieldInfos,
}

impl fmt::Debug for MemoryIndexReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryIndexReader").finish_non_exhaustive()
    }
}

impl fmt::Display for MemoryIndexReader {
    /// `Object.toString()` of the inner class, without its hash code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("org.apache.lucene.index.memory.MemoryIndex$MemoryIndexReader")
    }
}

impl MemoryIndexReader {
    /// Equivalent to `MemoryIndexReader()`.
    fn new(shared: Arc<Shared>) -> Self {
        let mut field_infos_arr = Vec::new();
        {
            let state = lock(&shared.state);
            for info in state.fields.values() {
                let mut info = lock(info);
                let pools = lock(&shared.pools);
                info.prepare_doc_values_and_point_values(&pools);
                // the Info's own FieldInfo object, as Java
                // (`fieldInfosArr[i++] = info.fieldInfo`)
                field_infos_arr.push(Arc::clone(&info.field_info));
            }
        }
        let field_infos = FieldInfos::from_shared(field_infos_arr)
            .expect("INVARIANT: the fields of a MemoryIndex have distinct names and numbers");
        Self {
            core: IndexReaderCore::new(),
            memory_fields: MemoryFields {
                shared: Arc::clone(&shared),
            },
            shared,
            field_infos,
        }
    }

    fn info(&self, field: &str) -> Option<Arc<Mutex<Info>>> {
        lock(&self.shared.state).fields.get(field).cloned()
    }

    /// Equivalent to `getInfoForExpectedDocValuesType(String, DocValuesType)`.
    fn get_info_for_expected_doc_values_type(
        &self,
        field_name: &str,
        expected_type: DocValuesType,
    ) -> Option<Arc<Mutex<Info>>> {
        if expected_type == DocValuesType::NONE {
            return None;
        }
        let info = self.info(field_name)?;
        if lock(&info).field_info.get_doc_values_type() != expected_type {
            return None;
        }
        Some(info)
    }

    /// Equivalent to `getSortedDocValues(String, DocValuesType)`.
    fn get_sorted_doc_values_of_type(
        &self,
        field: &str,
        doc_values_type: DocValuesType,
    ) -> Option<MemorySortedDocValues> {
        let info = self.get_info_for_expected_doc_values_type(field, doc_values_type)?;
        let value = lock(&info)
            .binary_producer
            .dv_bytes_values_set
            .clone()
            .unwrap_or_default();
        Some(sorted_doc_values(value))
    }
}

impl LeafReader for MemoryIndexReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Ok(Box::new(MemoryTermVectors {
            shared: Arc::clone(&self.shared),
        }))
    }

    fn num_docs(&self) -> i32 {
        1
    }

    fn max_doc(&self) -> i32 {
        1
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(MemoryStoredFields {
            shared: Arc::clone(&self.shared),
        }))
    }

    fn do_close(&self) -> Result<()> {
        Ok(())
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
        self.memory_fields.terms(field)
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        let Some(info) = self.get_info_for_expected_doc_values_type(field, DocValuesType::NUMERIC)
        else {
            return Ok(None);
        };
        let value = lock(&info)
            .numeric_producer
            .dv_long_values
            .as_ref()
            .expect("INVARIANT: a NUMERIC field has its value")[0];
        Ok(Some(Box::new(numeric_doc_values(value))))
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        Ok(self
            .get_sorted_doc_values_of_type(field, DocValuesType::BINARY)
            .map(|input| Box::new(MemoryBinaryDocValues { input }) as Box<dyn BinaryDocValues>))
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        Ok(self
            .get_sorted_doc_values_of_type(field, DocValuesType::SORTED)
            .map(|dv| Box::new(dv) as Box<dyn SortedDocValues>))
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let Some(info) =
            self.get_info_for_expected_doc_values_type(field, DocValuesType::SORTED_NUMERIC)
        else {
            return Ok(None);
        };
        let info = lock(&info);
        let values = info
            .numeric_producer
            .dv_long_values
            .clone()
            .unwrap_or_default();
        Ok(Some(Box::new(sorted_numeric_doc_values(
            values,
            info.numeric_producer.count,
        ))))
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let Some(info) =
            self.get_info_for_expected_doc_values_type(field, DocValuesType::SORTED_SET)
        else {
            return Ok(None);
        };
        Ok(Some(Box::new(MemorySortedSetDocValues {
            it: MemoryDocValuesIterator::default(),
            info,
            shared: Arc::clone(&self.shared),
            scratch: BytesRef::default(),
            ord: 0,
        })))
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        let Some(info) = self.info(field) else {
            return Ok(None);
        };
        let similarity = Arc::clone(&lock(&self.shared.state).norm_similarity);
        let mut info = lock(&info);
        if info.field_info.omits_norms() {
            return Ok(None);
        }
        Ok(Some(Box::new(info.get_norm_doc_values(&*similarity)?)))
    }

    fn get_doc_values_skipper(&self, _field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        // Skipping isn't needed on a 1-doc index.
        Ok(None)
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        let Some(info) = self.info(field) else {
            return Ok(None);
        };
        if lock(&info).float_vector_values.is_none() {
            return Ok(None);
        }
        Ok(Some(Box::new(MemoryFloatVectorValues { info })))
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        let Some(info) = self.info(field) else {
            return Ok(None);
        };
        if lock(&info).byte_vector_values.is_none() {
            return Ok(None);
        }
        Ok(Some(Box::new(MemoryByteVectorValues { info })))
    }

    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        None
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        let Some(info) = self.info(field) else {
            return Ok(None);
        };
        if lock(&info).point_values.is_none() {
            return Ok(None);
        }
        Ok(Some(Box::new(MemoryIndexPointValues::new(info))))
    }

    fn check_integrity(&self) -> Result<()> {
        // no-op
        Ok(())
    }

    fn get_meta_data(&self) -> LeafMetaData {
        LeafMetaData::new(
            Version::LATEST.major as i32,
            Some(Version::LATEST),
            None,
            false,
        )
        .expect("INVARIANT: the latest version is a valid created version")
    }
}

/// The anonymous `TermVectors` of `MemoryIndexReader.termVectors()`.
#[derive(Debug)]
struct MemoryTermVectors {
    shared: Arc<Shared>,
}

impl TermVectors for MemoryTermVectors {
    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        if doc == 0 {
            Ok(Some(Box::new(MemoryFields {
                shared: Arc::clone(&self.shared),
            })))
        } else {
            Ok(None)
        }
    }
}

/// The anonymous `StoredFields` of `MemoryIndexReader.storedFields()`.
#[derive(Debug)]
struct MemoryStoredFields {
    shared: Arc<Shared>,
}

impl StoredFields for MemoryStoredFields {
    fn document_with_visitor(
        &self,
        _doc_id: i32,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        let infos: Vec<Arc<Mutex<Info>>> =
            lock(&self.shared.state).fields.values().cloned().collect();
        for info in infos {
            let (field_info, stored_values) = {
                let info = lock(&info);
                (Arc::clone(&info.field_info), info.stored_values.clone())
            };
            let status = visitor.needs_field(&field_info)?;
            if status == StoredFieldVisitorStatus::Stop {
                return Ok(());
            }
            if status == StoredFieldVisitorStatus::No {
                continue;
            }
            if let Some(stored_values) = stored_values {
                for value in stored_values {
                    match value {
                        StoredValue::Bytes(bytes) => {
                            visitor.binary_field(&field_info, bytes.slice())?
                        }
                        StoredValue::Number(NumericValue::Double(d)) => {
                            visitor.double_field(&field_info, d)?
                        }
                        StoredValue::Number(NumericValue::Float(f)) => {
                            visitor.float_field(&field_info, f)?
                        }
                        StoredValue::Number(NumericValue::Long(l)) => {
                            visitor.long_field(&field_info, l)?
                        }
                        StoredValue::Number(NumericValue::Int(i)) => {
                            visitor.int_field(&field_info, i)?
                        }
                        StoredValue::String(s) => visitor.string_field(&field_info, s)?,
                    }
                }
            }
        }
        Ok(())
    }
}

/// Equivalent to
/// `org.apache.lucene.index.memory.MemoryIndex.MemoryIndexReader.MemoryFields`.
pub(crate) struct MemoryFields {
    shared: Arc<Shared>,
}

impl fmt::Debug for MemoryFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MemoryFields")
    }
}

impl Fields for MemoryFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        let names: Vec<String> = lock(&self.shared.state)
            .fields
            .iter()
            .filter(|(_, info)| lock(info).num_tokens > 0)
            .map(|(name, _)| name.clone())
            .collect();
        Box::new(names.into_iter())
    }

    fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
        let Some(info) = lock(&self.shared.state).fields.get(field).cloned() else {
            return Ok(None);
        };
        if lock(&info).num_tokens <= 0 {
            return Ok(None);
        }
        Ok(Some(Arc::new(MemoryTerms {
            info,
            shared: Arc::clone(&self.shared),
        })))
    }

    fn size(&self) -> i32 {
        lock(&self.shared.state)
            .fields
            .values()
            .filter(|info| lock(info).num_tokens > 0)
            .count() as i32
    }
}

/// The anonymous `Terms` of `MemoryFields.terms(String)`.
struct MemoryTerms {
    info: Arc<Mutex<Info>>,
    shared: Arc<Shared>,
}

impl Terms for MemoryTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(MemoryTermsEnum::new(
            Arc::clone(&self.info),
            Arc::clone(&self.shared),
        )))
    }

    fn size(&self) -> Result<i64> {
        Ok(lock(&self.info).terms.size() as i64)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(lock(&self.info).sum_total_term_freq)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        // each term has df=1
        Ok(lock(&self.info).terms.size() as i64)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(if self.size()? > 0 { 1 } else { 0 })
    }

    fn has_freqs(&self) -> bool {
        true
    }

    fn has_offsets(&self) -> bool {
        self.shared.store_offsets
    }

    fn has_positions(&self) -> bool {
        true
    }

    fn has_payloads(&self) -> bool {
        self.shared.store_payloads
    }
}

/// Equivalent to
/// `org.apache.lucene.index.memory.MemoryIndex.MemoryIndexReader.MemoryTermsEnum`.
pub(crate) struct MemoryTermsEnum {
    info: Arc<Mutex<Info>>,
    shared: Arc<Shared>,
    attributes: AttributeSource,
    br: BytesRef,
    term_upto: i32,
}

impl fmt::Display for MemoryTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("org.apache.lucene.index.memory.MemoryIndex$MemoryIndexReader$MemoryTermsEnum")
    }
}

impl MemoryTermsEnum {
    /// Equivalent to `MemoryTermsEnum(Info)`.
    fn new(info: Arc<Mutex<Info>>, shared: Arc<Shared>) -> Self {
        {
            let mut guard = lock(&info);
            let pools = lock(&shared.pools);
            guard.sort_terms(&pools);
        }
        Self {
            info,
            shared,
            attributes: AttributeSource::new(),
            br: BytesRef::default(),
            term_upto: -1,
        }
    }

    /// Equivalent to `binarySearch(BytesRef, BytesRef, int, int, BytesRefHash,
    /// int[])`; `self.br` is the scratch `bytesRef`.
    fn binary_search(&mut self, b: &BytesRef, mut low: i32, mut high: i32) -> i32 {
        let info = lock(&self.info);
        let pools = lock(&self.shared.pools);
        let ords = info.sorted();
        let mut mid;
        while low <= high {
            mid = ((low + high) as u32 >> 1) as i32;
            let bytes = info.terms.get(&pools.byte_block_pool, ords[mid as usize]);
            self.br = BytesRef::new(bytes.to_vec());
            match self.br.slice().cmp(b.slice()) {
                std::cmp::Ordering::Less => low = mid + 1,
                std::cmp::Ordering::Greater => high = mid - 1,
                std::cmp::Ordering::Equal => return mid,
            }
        }
        debug_assert!(self.br.slice() != b.slice());
        -(low + 1)
    }

    fn load_term(&mut self, index: usize) {
        let info = lock(&self.info);
        let pools = lock(&self.shared.pools);
        let bytes = info.terms.get(&pools.byte_block_pool, info.sorted()[index]);
        self.br = BytesRef::new(bytes.to_vec());
    }

    fn size(&self) -> i32 {
        lock(&self.info).terms.size()
    }
}

impl TermsEnum for MemoryTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        let size = self.size();
        self.term_upto = self.binary_search(text, 0, size - 1);
        Ok(self.term_upto >= 0)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let size = self.size();
        self.term_upto = self.binary_search(text, 0, size - 1);
        if self.term_upto < 0 {
            // not found; choose successor
            self.term_upto = -self.term_upto - 1;
            if self.term_upto >= size {
                Ok(SeekStatus::END)
            } else {
                self.load_term(self.term_upto as usize);
                Ok(SeekStatus::NOT_FOUND)
            }
        } else {
            Ok(SeekStatus::FOUND)
        }
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        debug_assert!(ord < self.size() as i64);
        self.term_upto = ord as i32;
        self.load_term(self.term_upto as usize);
        Ok(())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, state: &dyn TermState) -> Result<()> {
        let ord = state
            .as_any()
            .downcast_ref::<OrdTermState>()
            .ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "the term state is not an org.apache.lucene.index.OrdTermState",
                )
            })?
            .ord;
        self.seek_ord(ord)
    }

    fn term(&self) -> Result<Option<&BytesRef>> {
        Ok(Some(&self.br))
    }

    fn ord(&self) -> Result<i64> {
        Ok(self.term_upto as i64)
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Ok(1)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        let info = lock(&self.info);
        let ord = info.sorted()[self.term_upto as usize] as usize;
        let freq = lock(&info.slice_array).freq[ord];
        Ok(freq as i64)
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        // `if (reuse == null || !(reuse instanceof MemoryPostingsEnum))`.
        let mut reuse: Box<dyn PostingsEnum> = match reuse {
            Some(mut reuse) => {
                if reuse
                    .as_any_mut()
                    .is_some_and(|any| any.is::<MemoryPostingsEnum>())
                {
                    reuse
                } else {
                    Box::new(MemoryPostingsEnum::new(Arc::clone(&self.shared)))
                }
            }
            None => Box::new(MemoryPostingsEnum::new(Arc::clone(&self.shared))),
        };
        let (start, end, freq) = {
            let info = lock(&self.info);
            let ord = info.sorted()[self.term_upto as usize] as usize;
            let slices = lock(&info.slice_array);
            (slices.start[ord], slices.end[ord], slices.freq[ord])
        };
        reuse
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<MemoryPostingsEnum>())
            .expect("INVARIANT: checked above")
            .reset(start, end, freq);
        Ok(Some(reuse))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        let postings = self
            .postings(None, flags)?
            .expect("INVARIANT: MemoryTermsEnum always returns postings");
        Ok(Box::new(SlowImpactsEnum::new(postings)))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        let ts = OrdTermState {
            ord: self.term_upto as i64,
        };
        Ok(Box::new(ts))
    }

    fn next(&mut self) -> Result<Option<&BytesRef>> {
        self.term_upto += 1;
        if self.term_upto >= self.size() {
            Ok(None)
        } else {
            self.load_term(self.term_upto as usize);
            Ok(Some(&self.br))
        }
    }
}

/// Equivalent to
/// `org.apache.lucene.index.memory.MemoryIndex.MemoryIndexReader.MemoryPostingsEnum`.
pub(crate) struct MemoryPostingsEnum {
    shared: Arc<Shared>,
    slice_reader: SliceReader,
    /// For assert.
    pos_upto: i32,
    has_next: bool,
    doc: i32,
    freq: i32,
    start_offset: i32,
    end_offset: i32,
    payload_index: i32,
    /// Only `Some` when `storePayloads`; holds the current payload.
    payload_builder: Option<BytesRef>,
}

impl fmt::Display for MemoryPostingsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "org.apache.lucene.index.memory.MemoryIndex$MemoryIndexReader$MemoryPostingsEnum",
        )
    }
}

impl MemoryPostingsEnum {
    /// Equivalent to `MemoryPostingsEnum()`.
    fn new(shared: Arc<Shared>) -> Self {
        let payload_builder = if shared.store_payloads {
            Some(BytesRef::default())
        } else {
            None
        };
        Self {
            shared,
            slice_reader: SliceReader::default(),
            pos_upto: 0,
            has_next: false,
            doc: -1,
            freq: 0,
            start_offset: 0,
            end_offset: 0,
            payload_index: 0,
            payload_builder,
        }
    }

    /// Equivalent to `reset(int, int, int)`.
    fn reset(&mut self, start: i32, end: i32, freq: i32) -> &mut Self {
        self.slice_reader.reset(start, end);
        self.pos_upto = 0; // for assert
        self.has_next = true;
        self.doc = -1;
        self.freq = freq;
        self
    }
}

impl DocIdSetIterator for MemoryPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.has_next {
            self.has_next = false;
            self.doc = 0;
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        1
    }
}

impl PostingsEnum for MemoryPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        self.pos_upto += 1;
        debug_assert!(self.pos_upto <= self.freq);
        debug_assert!(
            !self.slice_reader.end_of_slice(),
            " stores offsets : {}",
            self.start_offset
        );
        let pools = lock(&self.shared.pools);
        let int_pool = &pools.sliced_int_block_pool;
        let pos = self.slice_reader.read_int(int_pool);
        if self.shared.store_offsets {
            self.start_offset = self.slice_reader.read_int(int_pool);
            self.end_offset = self.slice_reader.read_int(int_pool);
        }
        if self.shared.store_payloads {
            self.payload_index = self.slice_reader.read_int(int_pool);
            if self.payload_index != -1 {
                let payload = pools
                    .payloads_bytes_refs
                    .as_ref()
                    .expect("INVARIANT: payloads are stored")
                    .get(self.payload_index as usize)?;
                self.payload_builder = Some(payload);
            }
        }
        Ok(pos)
    }

    fn start_offset(&self) -> i32 {
        self.start_offset
    }

    fn end_offset(&self) -> i32 {
        self.end_offset
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        if self.payload_builder.is_none() || self.payload_index == -1 {
            return Ok(None);
        }
        Ok(self.payload_builder.as_ref().map(BytesRef::slice))
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Points
// ---------------------------------------------------------------------------

/// Equivalent to
/// `org.apache.lucene.index.memory.MemoryIndex.MemoryIndexReader.MemoryIndexPointValues`.
pub(crate) struct MemoryIndexPointValues {
    info: Arc<Mutex<Info>>,
}

impl MemoryIndexPointValues {
    /// Equivalent to `MemoryIndexPointValues(Info)`.
    fn new(info: Arc<Mutex<Info>>) -> Self {
        debug_assert!(
            lock(&info).point_values.is_some(),
            "Field does not have points"
        );
        Self { info }
    }
}

impl PointValues for MemoryIndexPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        let info = lock(&self.info);
        Ok(Box::new(MemoryPointTree {
            min_packed_value: info.min_packed_value.clone().unwrap_or_default(),
            max_packed_value: info.max_packed_value.clone().unwrap_or_default(),
            values: info.point_values.clone().unwrap_or_default(),
            count: info.point_values_count,
        }))
    }

    fn size(&self) -> Result<i64> {
        Ok(lock(&self.info).point_values_count as i64)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(1)
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(lock(&self.info).min_packed_value.clone())
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Ok(lock(&self.info).max_packed_value.clone())
    }

    fn num_dimensions(&self) -> Result<i32> {
        Ok(lock(&self.info).field_info.get_point_dimension_count())
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Ok(lock(&self.info).field_info.get_point_dimension_count())
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Ok(lock(&self.info).field_info.get_point_num_bytes())
    }
}

/// The anonymous single-leaf `PointTree` of `MemoryIndexPointValues`.
#[derive(Debug, Clone)]
struct MemoryPointTree {
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
    values: Vec<BytesRef>,
    count: usize,
}

impl PointTree for MemoryPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        // `return this;` — the tree is stateless.
        Ok(Box::new(self.clone()))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        Ok(&self.min_packed_value)
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        Ok(&self.max_packed_value)
    }

    fn size(&self) -> Result<i64> {
        Ok(self.count as i64)
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        visitor.grow(self.count as i32)?;
        for _ in 0..self.count {
            visitor.visit(0)?;
        }
        Ok(())
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        visitor.grow(self.count as i32)?;
        for value in &self.values[..self.count] {
            visitor.visit_with_value(0, value.slice())?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Vectors
// ---------------------------------------------------------------------------

/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.MemoryFloatVectorValues`.
pub(crate) struct MemoryFloatVectorValues {
    info: Arc<Mutex<Info>>,
}

impl KnnVectorValues for MemoryFloatVectorValues {
    fn dimension(&self) -> i32 {
        lock(&self.info).field_info.get_vector_dimension()
    }

    fn size(&self) -> i32 {
        lock(&self.info).float_vector_count
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(MemoryFloatVectorValues {
            info: Arc::clone(&self.info),
        }))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_dense_iterator())
    }
}

impl FloatVectorValues for MemoryFloatVectorValues {
    /// Java returns `null` for an ordinal other than `0`; the port returns
    /// the `NullPointerException` dereferencing it raises.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        if ord == 0 {
            Ok(lock(&self.info)
                .float_vector_values
                .as_ref()
                .expect("INVARIANT: the field has a float vector")[0]
                .clone())
        } else {
            Err(LuceneError::null_pointer())
        }
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(MemoryFloatVectorValues {
            info: Arc::clone(&self.info),
        }))
    }

    fn scorer(&self, query: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        let dimension = self.dimension();
        if query.len() as i32 != dimension {
            return Err(LuceneError::illegal_argument(format!(
                "query vector dimension {} does not match field dimension {}",
                query.len(),
                dimension
            )));
        }
        let vector_values = MemoryFloatVectorValues {
            info: Arc::clone(&self.info),
        };
        let iterator = vector_values.iterator()?;
        Ok(Some(Box::new(MemoryVectorScorer {
            info: Arc::clone(&self.info),
            query: VectorQuery::Float(query.to_vec()),
            iterator,
        })))
    }
}

/// Equivalent to `org.apache.lucene.index.memory.MemoryIndex.MemoryByteVectorValues`.
pub(crate) struct MemoryByteVectorValues {
    info: Arc<Mutex<Info>>,
}

impl KnnVectorValues for MemoryByteVectorValues {
    fn dimension(&self) -> i32 {
        lock(&self.info).field_info.get_vector_dimension()
    }

    fn size(&self) -> i32 {
        lock(&self.info).byte_vector_count
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(MemoryByteVectorValues {
            info: Arc::clone(&self.info),
        }))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_dense_iterator())
    }
}

impl ByteVectorValues for MemoryByteVectorValues {
    /// Java returns `null` for an ordinal other than `0`; the port returns
    /// the `NullPointerException` dereferencing it raises.
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        if ord == 0 {
            Ok(lock(&self.info)
                .byte_vector_values
                .as_ref()
                .expect("INVARIANT: the field has a byte vector")[0]
                .clone())
        } else {
            Err(LuceneError::null_pointer())
        }
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(MemoryByteVectorValues {
            info: Arc::clone(&self.info),
        }))
    }

    fn scorer(&self, query: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        let dimension = self.dimension();
        if query.len() as i32 != dimension {
            return Err(LuceneError::illegal_argument(format!(
                "query vector dimension {} does not match field dimension {}",
                query.len(),
                dimension
            )));
        }
        let vector_values = MemoryByteVectorValues {
            info: Arc::clone(&self.info),
        };
        let iterator = vector_values.iterator()?;
        Ok(Some(Box::new(MemoryVectorScorer {
            info: Arc::clone(&self.info),
            query: VectorQuery::Byte(query.to_vec()),
            iterator,
        })))
    }
}

/// The query vector of a [`MemoryVectorScorer`].
enum VectorQuery {
    Float(Vec<f32>),
    Byte(Vec<u8>),
}

/// The anonymous `VectorScorer`s of `MemoryFloatVectorValues.scorer` and
/// `MemoryByteVectorValues.scorer`.
struct MemoryVectorScorer {
    info: Arc<Mutex<Info>>,
    query: VectorQuery,
    iterator: Box<dyn DocIndexIterator>,
}

impl VectorScorer for MemoryVectorScorer {
    fn score(&mut self) -> Result<f32> {
        debug_assert!(self.iterator.doc_id() == 0);
        let info = lock(&self.info);
        let function = info.field_info.get_vector_similarity_function();
        match &self.query {
            VectorQuery::Float(query) => function.compare_f32(
                &info
                    .float_vector_values
                    .as_ref()
                    .expect("INVARIANT: the field has a float vector")[0],
                query,
            ),
            VectorQuery::Byte(query) => function.compare_bytes(
                &info
                    .byte_vector_values
                    .as_ref()
                    .expect("INVARIANT: the field has a byte vector")[0],
                query,
            ),
        }
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.iterator
    }

    /// The interface's default `bulk(DocIdSetIterator)`.
    fn bulk(
        self: Box<Self>,
        matching_docs: Option<Box<dyn DocIdSetIterator>>,
    ) -> Result<Box<dyn VectorScorerBulk>> {
        default_bulk(SharedVectorScorer::new(self), matching_docs)
    }
}
