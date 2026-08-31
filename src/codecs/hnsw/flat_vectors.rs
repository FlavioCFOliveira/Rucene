//! Flat vector codec helpers.
//!
//! Equivalent to `org.apache.lucene.codecs.hnsw.FlatVectorsFormat`,
//! `FlatVectorsReader`, `FlatVectorsWriter`, `FlatVectorsScorer`,
//! `FlatVectorScorerUtil`, `DefaultFlatVectorScorer`, and
//! `FlatFieldVectorsWriter`.
//!
//! These abstractions provide the shared infrastructure used by concrete
//! vector formats such as `Lucene99FlatVectorsFormat`.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use crate::codecs::knn_vectors::{
    BufferingKnnVectorsWriter, ByteVectorValues, FloatVectorValues, KnnFieldVectorsWriter,
    KnnVectorsFormat, KnnVectorsReader,
};
use crate::codecs::postings::MergeState;
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::VectorSimilarityFunction;
use crate::util::hnsw::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use crate::util::{FixedBitSet, RamUsageEstimator};

// -----------------------------------------------------------------------------
// DocsWithFieldSet
// -----------------------------------------------------------------------------

/// Accumulator for documents that have a value for a vector field.
///
/// Equivalent to `org.apache.lucene.index.DocsWithFieldSet`.
///
/// This is optimized for the dense case where every document has a value: in
/// that case no bit set is allocated and the cardinality alone is tracked.
#[derive(Debug, Clone)]
pub struct DocsWithFieldSet {
    set: Option<FixedBitSet>,
    cardinality: i32,
    last_doc_id: i32,
}

impl Default for DocsWithFieldSet {
    fn default() -> Self {
        Self {
            set: None,
            cardinality: 0,
            last_doc_id: -1,
        }
    }
}

impl DocsWithFieldSet {
    /// Creates an empty `DocsWithFieldSet`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a single document ID to the set.
    ///
    /// Document IDs must be added in strictly increasing order.
    pub fn add(&mut self, doc_id: i32) -> Result<()> {
        if doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "out-of-order doc ids: last={}, next={}",
                self.last_doc_id, doc_id
            )));
        }
        match &mut self.set {
            Some(set) => {
                grow_fixed_bit_set(set, doc_id as usize + 1);
                set.set(doc_id as usize);
            }
            None if doc_id != self.cardinality => {
                let mut set = FixedBitSet::new(doc_id as usize + 1);
                set_range(&mut set, 0, self.cardinality as usize);
                set.set(doc_id as usize);
                self.set = Some(set);
            }
            None => {}
        }
        self.last_doc_id = doc_id;
        self.cardinality += 1;
        Ok(())
    }

    /// Adds a contiguous range of document IDs.
    ///
    /// `from` must be strictly greater than the last document already added.
    pub fn add_range(&mut self, from: i32, to_exclusive: i32) -> Result<()> {
        if from > to_exclusive {
            return Err(LuceneError::IllegalArgument(format!(
                "from={from} must be <= toExclusive={to_exclusive}"
            )));
        }
        if from == to_exclusive {
            return Ok(());
        }
        if from <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "out-of-order doc ids: last={}, next={}",
                self.last_doc_id, from
            )));
        }
        let count = (to_exclusive - from) as usize;
        match &mut self.set {
            Some(set) => {
                grow_fixed_bit_set(set, to_exclusive as usize);
                set_range(set, from as usize, to_exclusive as usize);
            }
            None if from != self.cardinality => {
                let mut set = FixedBitSet::new(to_exclusive as usize);
                set_range(&mut set, 0, self.cardinality as usize);
                set_range(&mut set, from as usize, to_exclusive as usize);
                self.set = Some(set);
            }
            None => {}
        }
        self.last_doc_id = to_exclusive - 1;
        self.cardinality += count as i32;
        Ok(())
    }

    /// Returns an iterator over the document IDs in this set.
    pub fn iterator(&self) -> Result<Box<dyn crate::search::DocIdSetIterator + '_>> {
        match &self.set {
            Some(set) => Ok(Box::new(FixedBitSetIterator::new(
                set,
                self.cardinality as i64,
            ))),
            None => Ok(Box::new(crate::search::AllDocIdSetIterator::new(
                self.cardinality,
            )?)),
        }
    }

    /// Returns the number of documents in this set.
    pub fn cardinality(&self) -> i32 {
        self.cardinality
    }

    /// Returns the approximate RAM bytes used by this set.
    pub fn ram_bytes_used(&self) -> i64 {
        // Approximate shallow object size: object header + three fields.
        let base = crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 16;
        crate::util::RamUsageEstimator::align_object_size(base)
            + self
                .set
                .as_ref()
                .map(fixed_bit_set_ram_bytes_used)
                .unwrap_or(0)
    }
}

/// Sets all bits in `[from, to)` on `set`.
fn set_range(set: &mut FixedBitSet, from: usize, to: usize) {
    for i in from..to {
        set.set(i);
    }
}

/// Grows `set` so it can hold at least `min_bits` bits, preserving existing
/// set bits.
fn grow_fixed_bit_set(set: &mut FixedBitSet, min_bits: usize) {
    let current = set.length();
    if min_bits <= current {
        return;
    }
    let num_words = FixedBitSet::bits2words(min_bits);
    let mut words = set.get_bits().to_vec();
    words.resize(num_words, 0u64);
    *set = FixedBitSet::from_bits(words, min_bits);
}

/// Returns the approximate RAM bytes used by a `FixedBitSet`.
fn fixed_bit_set_ram_bytes_used(set: &FixedBitSet) -> i64 {
    RamUsageEstimator::size_of_u64(set.get_bits())
}

/// Iterator over the set bits of a `FixedBitSet`.
struct FixedBitSetIterator<'a> {
    set: &'a FixedBitSet,
    doc: i32,
    cardinality: i64,
    count: i64,
}

impl<'a> FixedBitSetIterator<'a> {
    fn new(set: &'a FixedBitSet, cardinality: i64) -> FixedBitSetIterator<'a> {
        Self {
            set,
            doc: -1,
            cardinality,
            count: 0,
        }
    }
}

impl crate::search::DocIdSetIterator for FixedBitSetIterator<'_> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let mut doc = target.max(0);
        let length = self.set.length() as i32;
        while doc < length {
            if self.set.get(doc as usize) {
                self.doc = doc;
                self.count += 1;
                return Ok(doc);
            }
            doc += 1;
        }
        self.doc = crate::search::NO_MORE_DOCS;
        Ok(crate::search::NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        self.cardinality
    }
}

// -----------------------------------------------------------------------------
// FlatFieldVectorsWriter
// -----------------------------------------------------------------------------

/// Writer for a single vector field.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.FlatFieldVectorsWriter`.
pub trait FlatFieldVectorsWriter<T>: KnnFieldVectorsWriter<T> {
    /// Returns the buffered vectors to be written.
    fn vectors(&self) -> &[T];

    /// Returns the set of documents that have a value for this field.
    fn docs_with_field_set(&self) -> &DocsWithFieldSet;

    /// Marks this writer as finished; no new vectors may be added afterwards.
    fn finish(&mut self) -> Result<()>;

    /// Returns `true` if this writer is finished.
    fn is_finished(&self) -> bool;
}

// -----------------------------------------------------------------------------
// DefaultFlatFieldVectorsWriter
// -----------------------------------------------------------------------------

/// Default on-heap buffer for a single vector field.
///
/// Equivalent to `Lucene99FlatVectorsWriter.FieldWriter`, the one concrete
/// `FlatFieldVectorsWriter` Lucene 10.5.0 ships
/// (`Lucene99FlatVectorsWriter.java:370-434`). It buffers every added vector
/// together with the set of documents that have a value, and — like Java's —
/// it carries the field's [`FieldInfo`], which it needs for the duplicate-value
/// message and for its RAM footprint.
#[derive(Debug, Clone)]
pub struct DefaultFlatFieldVectorsWriter<T> {
    field_info: FieldInfo,
    vectors: Vec<T>,
    docs_with_field_set: DocsWithFieldSet,
    /// The last doc id added, `-1` before the first. Java keeps the same field
    /// and compares against it to reject a second value in one document.
    last_doc_id: i32,
    finished: bool,
}

impl<T> DefaultFlatFieldVectorsWriter<T> {
    /// Creates a new empty field writer for `field_info`.
    pub fn new(field_info: FieldInfo) -> Self {
        Self {
            field_info,
            vectors: Vec::new(),
            docs_with_field_set: DocsWithFieldSet::new(),
            last_doc_id: -1,
            finished: false,
        }
    }

    /// Returns the field this writer buffers.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Returns the RAM this writer currently holds.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter.FieldWriter.ramBytesUsed()`
    /// (`Lucene99FlatVectorsWriter.java:419-430`): a shallow size, plus the
    /// document set, plus one array header and reference per buffered vector,
    /// plus the vector payload itself. The shallow size is an approximation —
    /// Java measures a JVM object layout that has no Rust counterpart — but the
    /// terms that scale with the data are computed exactly as Java computes
    /// them, and those are the ones that decide when a segment flushes.
    pub fn ram_bytes_used(&self) -> i64 {
        let shallow = RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 5 * RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        );
        if self.vectors.is_empty() {
            return shallow;
        }
        let count = self.vectors.len() as i64;
        shallow
            + self.docs_with_field_set.ram_bytes_used()
            + count
                * (RamUsageEstimator::NUM_BYTES_OBJECT_REF
                    + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER)
            + count
                * i64::from(self.field_info.vector_dimension)
                * i64::from(self.field_info.vector_encoding.byte_size())
    }
}

impl<T: Clone> DefaultFlatFieldVectorsWriter<T> {
    /// Adds a document with its vector value.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once [`FlatFieldVectorsWriter::finish`]
    /// has run, and [`LuceneError::IllegalArgument`] when the same document
    /// offers a second value for this field, or when doc ids do not increase.
    pub fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "already finished, cannot add more values".to_string(),
            ));
        }
        // Java's guard, verbatim (`Lucene99FlatVectorsWriter.java:406-412`): a
        // vector field is single-valued, and a document that offers two is a
        // caller error rather than a corrupt index. `DocsWithFieldSet::add`
        // would also reject the repeat, but with the generic out-of-order
        // message that says nothing about which field the caller duplicated.
        if doc_id == self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "VectorValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
                self.field_info.name
            )));
        }
        // Java asserts `docID > lastDocID` here, which is disabled in
        // production; `DocsWithFieldSet::add` enforces it unconditionally.
        self.docs_with_field_set.add(doc_id)?;
        self.vectors.push(vector_value);
        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Adds a contiguous doc-id range without a vector.
    pub fn add_docs_with_no_vector(&mut self, from: i32, to_exclusive: i32) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "already finished, cannot add more values".to_string(),
            ));
        }
        self.docs_with_field_set.add_range(from, to_exclusive)?;
        if to_exclusive > from {
            self.last_doc_id = to_exclusive - 1;
        }
        Ok(())
    }
}

impl<T: Clone + Send + Sync + 'static> KnnFieldVectorsWriter<T>
    for DefaultFlatFieldVectorsWriter<T>
{
    fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()> {
        DefaultFlatFieldVectorsWriter::add_value(self, doc_id, vector_value)
    }

    fn copy_value(&self, vector_value: T) -> T {
        vector_value
    }

    fn ram_bytes_used(&self) -> i64 {
        DefaultFlatFieldVectorsWriter::ram_bytes_used(self)
    }
}

impl<T: Clone + Send + Sync + 'static> FlatFieldVectorsWriter<T>
    for DefaultFlatFieldVectorsWriter<T>
{
    fn vectors(&self) -> &[T] {
        &self.vectors
    }

    fn docs_with_field_set(&self) -> &DocsWithFieldSet {
        &self.docs_with_field_set
    }

    fn finish(&mut self) -> Result<()> {
        self.finished = true;
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

// -----------------------------------------------------------------------------
// FlatVectorsScorer
// -----------------------------------------------------------------------------

/// Scoring interface for flat stored vectors.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.FlatVectorsScorer`.
///
/// The Rust API uses the typed `FloatVectorValues` / `ByteVectorValues` traits
/// directly instead of a unified `KnnVectorValues` supertype, which is more
/// idiomatic and removes the need for runtime encoding dispatch.
pub trait FlatVectorsScorer: Send + Sync + fmt::Debug {
    /// Returns a scorer supplier for float vectors.
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>>;

    /// Returns a scorer supplier for byte vectors.
    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>>;

    /// Returns a one-off random scorer for float vectors against `target`.
    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>>;

    /// Returns a one-off random scorer for byte vectors against `target`.
    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>>;
}

// -----------------------------------------------------------------------------
// DefaultFlatVectorScorer
// -----------------------------------------------------------------------------

/// Default implementation of [`FlatVectorsScorer`].
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.DefaultFlatVectorScorer`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultFlatVectorScorer;

impl DefaultFlatVectorScorer {
    /// The singleton instance.
    pub const INSTANCE: Self = Self;
}

impl FlatVectorsScorer for DefaultFlatVectorScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(FloatScoringSupplier::new(
            Arc::from(vector_values),
            similarity_function,
        )))
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(ByteScoringSupplier::new(
            Arc::from(vector_values),
            similarity_function,
        )))
    }

    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        if target.len() as i32 != vector_values.dimension() {
            return Err(LuceneError::IllegalArgument(format!(
                "vector query dimension: {} differs from field dimension: {}",
                target.len(),
                vector_values.dimension()
            )));
        }
        Ok(Box::new(FloatVectorScorer::new(
            Arc::from(vector_values),
            target.to_vec(),
            similarity_function,
        )))
    }

    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        if target.len() as i32 != vector_values.dimension() {
            return Err(LuceneError::IllegalArgument(format!(
                "vector query dimension: {} differs from field dimension: {}",
                target.len(),
                vector_values.dimension()
            )));
        }
        Ok(Box::new(ByteVectorScorer::new(
            Arc::from(vector_values),
            target.to_vec(),
            similarity_function,
        )))
    }
}

struct FloatVectorScorer {
    values: Arc<dyn FloatVectorValues>,
    query: Vec<f32>,
    similarity_function: VectorSimilarityFunction,
}

impl FloatVectorScorer {
    fn new(
        values: Arc<dyn FloatVectorValues>,
        query: Vec<f32>,
        similarity_function: VectorSimilarityFunction,
    ) -> Self {
        Self {
            values,
            query,
            similarity_function,
        }
    }
}

impl RandomVectorScorer for FloatVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let v = self.values.vector_value(node)?;
        self.similarity_function.compare_f32(&self.query, &v)
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }
}

struct ByteVectorScorer {
    values: Arc<dyn ByteVectorValues>,
    query: Vec<u8>,
    similarity_function: VectorSimilarityFunction,
}

impl ByteVectorScorer {
    fn new(
        values: Arc<dyn ByteVectorValues>,
        query: Vec<u8>,
        similarity_function: VectorSimilarityFunction,
    ) -> Self {
        Self {
            values,
            query,
            similarity_function,
        }
    }
}

impl RandomVectorScorer for ByteVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let v = self.values.vector_value(node)?;
        self.similarity_function.compare_bytes(&self.query, &v)
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }
}

struct FloatScoringSupplier {
    vectors: Arc<dyn FloatVectorValues>,
    target_vectors: Arc<dyn FloatVectorValues>,
    similarity_function: VectorSimilarityFunction,
}

impl FloatScoringSupplier {
    fn new(
        vectors: Arc<dyn FloatVectorValues>,
        similarity_function: VectorSimilarityFunction,
    ) -> Self {
        let target_vectors = Arc::clone(&vectors);
        Self {
            vectors,
            target_vectors,
            similarity_function,
        }
    }
}

impl RandomVectorScorerSupplier for FloatScoringSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        let dimension = self.vectors.dimension();
        let query = vec![0.0f32; dimension as usize];
        Ok(Box::new(UpdateableFloatVectorScorer {
            values: Arc::clone(&self.vectors),
            target_vectors: Arc::clone(&self.target_vectors),
            query,
            similarity_function: self.similarity_function,
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::new(
            Arc::clone(&self.vectors),
            self.similarity_function,
        )))
    }
}

struct UpdateableFloatVectorScorer {
    values: Arc<dyn FloatVectorValues>,
    target_vectors: Arc<dyn FloatVectorValues>,
    query: Vec<f32>,
    similarity_function: VectorSimilarityFunction,
}

impl RandomVectorScorer for UpdateableFloatVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let v = self.target_vectors.vector_value(node)?;
        self.similarity_function.compare_f32(&self.query, &v)
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }
}

impl UpdateableRandomVectorScorer for UpdateableFloatVectorScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let v = self.target_vectors.vector_value(node)?;
        self.query.copy_from_slice(&v);
        Ok(())
    }
}

struct ByteScoringSupplier {
    vectors: Arc<dyn ByteVectorValues>,
    target_vectors: Arc<dyn ByteVectorValues>,
    similarity_function: VectorSimilarityFunction,
}

impl ByteScoringSupplier {
    fn new(
        vectors: Arc<dyn ByteVectorValues>,
        similarity_function: VectorSimilarityFunction,
    ) -> Self {
        let target_vectors = Arc::clone(&vectors);
        Self {
            vectors,
            target_vectors,
            similarity_function,
        }
    }
}

impl RandomVectorScorerSupplier for ByteScoringSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        let dimension = self.vectors.dimension();
        let query = vec![0u8; dimension as usize];
        Ok(Box::new(UpdateableByteVectorScorer {
            values: Arc::clone(&self.vectors),
            target_vectors: Arc::clone(&self.target_vectors),
            query,
            similarity_function: self.similarity_function,
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::new(
            Arc::clone(&self.vectors),
            self.similarity_function,
        )))
    }
}

struct UpdateableByteVectorScorer {
    values: Arc<dyn ByteVectorValues>,
    target_vectors: Arc<dyn ByteVectorValues>,
    query: Vec<u8>,
    similarity_function: VectorSimilarityFunction,
}

impl RandomVectorScorer for UpdateableByteVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let v = self.target_vectors.vector_value(node)?;
        self.similarity_function.compare_bytes(&self.query, &v)
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }
}

impl UpdateableRandomVectorScorer for UpdateableByteVectorScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let v = self.target_vectors.vector_value(node)?;
        self.query.copy_from_slice(&v);
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// FlatVectorsFormat
// -----------------------------------------------------------------------------

/// Encodes/decodes per-document flat vectors.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.FlatVectorsFormat`.
pub trait FlatVectorsFormat: KnnVectorsFormat {
    /// Returns a writer to write the vectors to the index.
    fn fields_writer_flat(
        &self,
        state: &SegmentWriteState<'_>,
    ) -> Result<Box<dyn FlatVectorsWriter>>;

    /// Returns a reader to read the vectors from the index.
    fn fields_reader_flat(
        &self,
        state: &SegmentReadState<'_>,
    ) -> Result<Box<dyn FlatVectorsReader>>;
}

// -----------------------------------------------------------------------------
// FlatVectorsReader
// -----------------------------------------------------------------------------

/// Reads flat vectors from an index.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.FlatVectorsReader`.
pub trait FlatVectorsReader: KnnVectorsReader {
    /// Returns the flat vector scorer for the given field.
    fn get_flat_vector_scorer(&self, field: &str) -> Result<Box<dyn FlatVectorsScorer>>;

    /// Returns a random vector scorer for the given float target vector.
    fn get_random_vector_scorer_float(
        &self,
        field: &str,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>>;

    /// Returns a random vector scorer for the given byte target vector.
    fn get_random_vector_scorer_byte(
        &self,
        field: &str,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>>;

    /// Returns an instance optimized for merging.
    fn get_merge_instance_flat(&self) -> Result<Box<dyn FlatVectorsReader>>;
}

// -----------------------------------------------------------------------------
// FlatVectorsWriter
// -----------------------------------------------------------------------------

/// Writes flat vectors to an index.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.FlatVectorsWriter`.
pub trait FlatVectorsWriter: BufferingKnnVectorsWriter {
    /// Returns the scorer used to score vectors while writing.
    fn vectors_scorer(&self) -> &dyn FlatVectorsScorer;

    /// Merges vectors for a single field.
    fn merge_one_flat_vector_field(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<()>;
}

// -----------------------------------------------------------------------------
// FlatVectorScorerUtil
// -----------------------------------------------------------------------------

/// Utilities for obtaining a flat-vector scorer.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.FlatVectorScorerUtil`.
///
/// Rucene does not implement Lucene's vectorization provider, so this always
/// returns the portable [`DefaultFlatVectorScorer`].
pub struct FlatVectorScorerUtil;

impl FlatVectorScorerUtil {
    /// Returns a scorer suitable for the Lucene99 flat-vectors format.
    pub fn get_lucene99_flat_vectors_scorer() -> DefaultFlatVectorScorer {
        DefaultFlatVectorScorer::INSTANCE
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::vector_values::from_floats;
    use crate::search::{DocIdSetIterator, NO_MORE_DOCS};

    #[test]
    fn docs_with_field_set_dense() {
        let mut set = DocsWithFieldSet::new();
        for i in 0..5 {
            set.add(i).unwrap();
        }
        assert_eq!(set.cardinality(), 5);
        assert!(set.ram_bytes_used() > 0);
        let mut it = set.iterator().unwrap();
        for i in 0..5 {
            assert_eq!(it.next_doc().unwrap(), i);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn docs_with_field_set_sparse() {
        let mut set = DocsWithFieldSet::new();
        set.add(0).unwrap();
        set.add(5).unwrap();
        set.add(100).unwrap();
        assert_eq!(set.cardinality(), 3);
        let mut it = set.iterator().unwrap();
        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.next_doc().unwrap(), 5);
        assert_eq!(it.next_doc().unwrap(), 100);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn docs_with_field_set_add_range() {
        let mut set = DocsWithFieldSet::new();
        set.add_range(0, 3).unwrap();
        set.add(5).unwrap();
        assert_eq!(set.cardinality(), 4);
        let mut it = set.iterator().unwrap();
        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.next_doc().unwrap(), 1);
        assert_eq!(it.next_doc().unwrap(), 2);
        assert_eq!(it.next_doc().unwrap(), 5);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn docs_with_field_set_rejects_out_of_order() {
        let mut set = DocsWithFieldSet::new();
        set.add(5).unwrap();
        assert!(set.add(3).is_err());
    }

    /// A minimal float-vector `FieldInfo`, which the default field writer now
    /// needs for its duplicate-value message and its RAM footprint.
    fn test_float_field_info(name: &str, dim: i32) -> FieldInfo {
        FieldInfo::new_full(
            name,
            0,
            false,
            false,
            false,
            crate::index::IndexOptions::NONE,
            crate::index::DocValuesType::NONE,
            crate::index::DocValuesSkipIndexType::NONE,
            -1,
            std::collections::HashMap::new(),
            0,
            0,
            0,
            dim,
            crate::index::VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
        .expect("field info")
    }

    #[test]
    fn default_flat_field_writer_buffers_vectors() {
        let mut writer: DefaultFlatFieldVectorsWriter<Vec<f32>> =
            DefaultFlatFieldVectorsWriter::new(test_float_field_info("v", 2));
        writer.add_value(0, vec![1.0, 2.0]).unwrap();
        writer.add_value(2, vec![3.0, 4.0]).unwrap();
        assert_eq!(writer.vectors().len(), 2);
        assert_eq!(writer.docs_with_field_set().cardinality(), 2);
        writer.finish().unwrap();
        assert!(writer.is_finished());
    }

    #[test]
    fn flat_vector_scorer_util_returns_default() {
        let scorer = FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer();
        assert_eq!(format!("{:?}", scorer), "DefaultFlatVectorScorer");
    }

    #[test]
    fn default_flat_vector_scorer_scores_float_vectors() {
        let vectors = from_floats(
            vec![
                vec![1.0, 0.0, 0.0],
                vec![0.0, 1.0, 0.0],
                vec![0.0, 0.0, 1.0],
            ],
            3,
        );
        let scorer = DefaultFlatVectorScorer::INSTANCE
            .get_random_vector_scorer_float(
                VectorSimilarityFunction::DOT_PRODUCT,
                Box::new(vectors),
                &[1.0, 0.0, 0.0],
            )
            .unwrap();
        assert_eq!(scorer.max_ord(), 3);
        let mut scorer = scorer;
        assert!(scorer.score(0).unwrap() > scorer.score(1).unwrap());
    }

    #[test]
    fn default_flat_vector_scorer_supplier_updates_query() {
        let vectors = from_floats(
            vec![
                vec![1.0, 0.0, 0.0],
                vec![0.0, 1.0, 0.0],
                vec![0.0, 0.0, 1.0],
            ],
            3,
        );
        let supplier = DefaultFlatVectorScorer::INSTANCE
            .get_random_vector_scorer_supplier_float(
                VectorSimilarityFunction::DOT_PRODUCT,
                Box::new(vectors),
            )
            .unwrap();
        let mut scorer = supplier.scorer().unwrap();
        scorer.set_scoring_ordinal(0).unwrap();
        let score_same = scorer.score(0).unwrap();
        let score_other = scorer.score(1).unwrap();
        assert!(score_same > score_other);
    }
}
