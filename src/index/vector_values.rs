//! Vector value accessors ported from `org.apache.lucene.index`.
//!
//! Equivalent to `org.apache.lucene.index.KnnVectorValues`,
//! `FloatVectorValues` and `ByteVectorValues`.
//!
//! Vectors are addressed by an ordinal (`ord`). The base [`KnnVectorValues`]
//! trait provides dimension and size metadata, while [`FloatVectorValues`]
//! and [`ByteVectorValues`] add type-specific value access. Iteration over
//! the document IDs that have vectors is provided by [`DocIndexIterator`],
//! which pairs each doc ID with the ordinal of its vector.
//!
//! # Rust adaptations
//!
//! Two Java constructs have no direct Rust equivalent and are adapted here:
//!
//! * **Covariant returns.** `FloatVectorValues.copy()` narrows the return type
//!   of `KnnVectorValues.copy()`. Rust has no return covariance, so the erased
//!   [`KnnVectorValues::copy`] is kept and [`FloatVectorValues::copy_float`] /
//!   [`ByteVectorValues::copy_byte`] are added alongside it.
//! * **Non-static inner classes.** `createDenseIterator` and
//!   `createSparseIterator` are inner classes that read the enclosing
//!   `size()` and `ordToDoc()`. The Rust iterators take that state by value (or
//!   behind an [`Arc`]) at construction time instead of borrowing the owner,
//!   which keeps the iterator types free of lifetimes.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::leaf_reader::LeafReader;
use crate::index::VectorEncoding;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::Bits;

// -----------------------------------------------------------------------------
// Doc-index iterator
// -----------------------------------------------------------------------------

/// A [`DocIdSetIterator`] that also tracks a distinct ordinal for the vector
/// associated with each doc.
///
/// Equivalent to `KnnVectorValues.DocIndexIterator`.
pub trait DocIndexIterator: DocIdSetIterator {
    /// Returns the value index (ordinal) corresponding to the current doc.
    fn index(&self) -> i32;
}

/// Iterator over a dense vector field where every document has exactly one
/// vector and the ordinal equals the doc ID.
///
/// Equivalent to the iterator returned by
/// `KnnVectorValues.createDenseIterator()`.
#[derive(Debug, Clone)]
pub struct DenseDocIndexIterator {
    size: i32,
    doc: i32,
}

impl DenseDocIndexIterator {
    /// Creates an iterator over `[0, size)`.
    pub fn new(size: i32) -> Self {
        Self { size, doc: -1 }
    }
}

impl DocIdSetIterator for DenseDocIndexIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.doc >= self.size - 1 {
            self.doc = NO_MORE_DOCS;
        } else {
            self.doc += 1;
        }
        Ok(self.doc)
    }

    /// Positions the iterator on `target`.
    ///
    /// Java performs a bare assignment (`return doc = target`) with no clamp,
    /// and this port does the same: `advance` below the current position is
    /// illegal usage under the [`DocIdSetIterator`] contract, and silently
    /// clamping it would hide the caller's bug while diverging from Lucene.
    fn advance(&mut self, target: i32) -> Result<i32> {
        if target >= self.size {
            self.doc = NO_MORE_DOCS;
        } else {
            self.doc = target;
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.size as i64
    }

    // `doc_id_run_end` is deliberately not overridden. Java does not override
    // it either, so both fall back to `docID() + 1`. Returning `size` would
    // also satisfy the contract for a dense iterator, but it is not what
    // Lucene 10.5.0 returns.
}

impl DocIndexIterator for DenseDocIndexIterator {
    fn index(&self) -> i32 {
        self.doc
    }
}

/// Iterator that pairs a delegate doc-id iterator with a sequentially
/// incremented ordinal.
///
/// Equivalent to `KnnVectorValues.fromDISI(DocIdSetIterator)`.
///
/// # Ordinal semantics
///
/// The ordinal is advanced **only** by [`next_doc`](DocIdSetIterator::next_doc).
/// [`advance`](DocIdSetIterator::advance) forwards to the delegate and leaves
/// the ordinal untouched, so after a jump [`index`](DocIndexIterator::index)
/// reports a stale value. That is exactly what Lucene 10.5.0 does
/// (`KnnVectorValues.java`, the `advance` override of the `fromDISI`
/// iterator): its only caller, `BufferingKnnVectorsWriter`, consumes the
/// iterator sequentially. Only sequential access is valid; the behaviour is
/// reproduced rather than "fixed" because functional parity is the contract.
pub struct FromDisiDocIndexIterator {
    docs: Box<dyn DocIdSetIterator>,
    ord: i32,
}

impl FromDisiDocIndexIterator {
    /// Creates an iterator from a doc-id iterator.
    pub fn new(docs: Box<dyn DocIdSetIterator>) -> Self {
        Self { docs, ord: -1 }
    }
}

impl DocIdSetIterator for FromDisiDocIndexIterator {
    fn doc_id(&self) -> i32 {
        self.docs.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.doc_id() == NO_MORE_DOCS {
            return Ok(NO_MORE_DOCS);
        }
        self.ord += 1;
        self.docs.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        // The ordinal is intentionally left untouched; see the type docs.
        self.docs.advance(target)
    }

    fn cost(&self) -> i64 {
        self.docs.cost()
    }
}

impl DocIndexIterator for FromDisiDocIndexIterator {
    fn index(&self) -> i32 {
        self.ord
    }
}

/// Iterator over a sparse vector field, driven by the ordinal-to-doc mapping.
///
/// Equivalent to the iterator returned by
/// `KnnVectorValues.createSparseIterator()`.
///
/// The mapping **must** be monotonic: the doc ID has to increase whenever the
/// ordinal does. That is Lucene's documented precondition, and
/// [`advance`](DocIdSetIterator::advance) relies on it because it is
/// implemented as a linear scan.
#[derive(Clone)]
pub struct SparseDocIndexIterator {
    ord_to_doc: Arc<dyn Fn(i32) -> i32 + Send + Sync>,
    size: i32,
    ord: i32,
}

impl SparseDocIndexIterator {
    /// Creates a sparse iterator over `size` vectors with the given monotonic
    /// ordinal-to-doc mapping.
    pub fn new(size: i32, ord_to_doc: Arc<dyn Fn(i32) -> i32 + Send + Sync>) -> Self {
        Self {
            ord_to_doc,
            size,
            ord: -1,
        }
    }

    /// Creates a sparse iterator that reads its mapping from `values`.
    ///
    /// This is the closest Rust equivalent to Java's non-static
    /// `createSparseIterator()`, which captures the enclosing
    /// `KnnVectorValues`; the [`Arc`] keeps the capture alive without imposing
    /// a lifetime on the iterator type.
    pub fn over(values: Arc<dyn KnnVectorValues>) -> Self {
        let size = values.size();
        Self::new(size, Arc::new(move |ord| values.ord_to_doc(ord)))
    }
}

impl fmt::Debug for SparseDocIndexIterator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SparseDocIndexIterator")
            .field("size", &self.size)
            .field("ord", &self.ord)
            .finish_non_exhaustive()
    }
}

impl DocIdSetIterator for SparseDocIndexIterator {
    fn doc_id(&self) -> i32 {
        if self.ord == -1 {
            return -1;
        }
        if self.ord == NO_MORE_DOCS {
            return NO_MORE_DOCS;
        }
        (self.ord_to_doc)(self.ord)
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.ord >= self.size - 1 {
            self.ord = NO_MORE_DOCS;
        } else {
            self.ord += 1;
        }
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        self.size as i64
    }
}

impl DocIndexIterator for SparseDocIndexIterator {
    fn index(&self) -> i32 {
        self.ord
    }
}

// -----------------------------------------------------------------------------
// Knn vector values
// -----------------------------------------------------------------------------

/// Base trait for document vector values indexed as KNN vector fields.
///
/// Equivalent to `org.apache.lucene.index.KnnVectorValues`.
pub trait KnnVectorValues: Send + Sync {
    /// Returns the vector dimension.
    fn dimension(&self) -> i32;

    /// Returns the number of vectors for this field.
    fn size(&self) -> i32;

    /// Returns the doc ID of the document indexed with the given vector ordinal.
    ///
    /// The default implementation returns `ord`, which is correct for dense
    /// implementations where every doc has a single value.
    fn ord_to_doc(&self, ord: i32) -> i32 {
        ord
    }

    /// Hints that the vectors for the first `num_ords` entries of
    /// `ords_to_prefetch` are about to be read.
    ///
    /// Equivalent to `KnnVectorValues.prefetch(int[], int)`; the default is a
    /// no-op, as in Java.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying storage cannot be hinted.
    fn prefetch(&self, _ords_to_prefetch: &[i32], _num_ords: i32) -> Result<()> {
        Ok(())
    }

    /// Returns a fresh copy of this instance.
    ///
    /// Copies exist so that several vectors can be read concurrently without
    /// clobbering the buffer that a single instance reuses.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying input cannot be cloned.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>>;

    /// Returns the vector byte length.
    fn vector_byte_length(&self) -> i32 {
        self.dimension() * self.encoding().byte_size()
    }

    /// Returns the vector encoding.
    fn encoding(&self) -> VectorEncoding;

    /// Returns an iterator over the document IDs that have vectors.
    ///
    /// Equivalent to `KnnVectorValues.iterator()`, whose default
    /// implementation throws `UnsupportedOperationException`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] unless the implementation
    /// overrides this method.
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Err(LuceneError::UnsupportedOperation(
            "KnnVectorValues::iterator".to_string(),
        ))
    }
}

/// [`Bits`] view mapping a vector ordinal onto the acceptance of its document.
///
/// Equivalent to the anonymous `Bits` returned by
/// `KnnVectorValues.getAcceptOrds(Bits)`.
pub struct AcceptOrds<'a> {
    values: &'a dyn KnnVectorValues,
    accept_docs: &'a dyn Bits,
}

impl fmt::Debug for AcceptOrds<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcceptOrds")
            .field("length", &self.values.size())
            .field("accept_docs", &self.accept_docs)
            .finish()
    }
}

impl Bits for AcceptOrds<'_> {
    fn get(&self, index: usize) -> bool {
        self.accept_docs
            .get(self.values.ord_to_doc(index as i32) as usize)
    }

    fn length(&self) -> usize {
        self.values.size() as usize
    }
}

/// Returns a [`Bits`] accepting the ordinals whose documents `accept_docs`
/// accepts, or `None` when `accept_docs` is `None`.
///
/// Equivalent to `KnnVectorValues.getAcceptOrds(Bits)`.
///
/// # Why a free function
///
/// Java declares this as an instance method whose anonymous `Bits` captures
/// `this`. A Rust trait method cannot build such a view from `&self` while the
/// trait stays object safe, because `&Self` with `Self: ?Sized` cannot be
/// coerced to `&dyn KnnVectorValues`. Taking the receiver explicitly keeps the
/// view lazy — no ordinal is mapped until it is queried — and keeps
/// [`KnnVectorValues`] usable behind `dyn`.
pub fn accept_ords<'a>(
    values: &'a dyn KnnVectorValues,
    accept_docs: Option<&'a dyn Bits>,
) -> Option<Box<dyn Bits + 'a>> {
    accept_docs.map(|accept_docs| {
        Box::new(AcceptOrds {
            values,
            accept_docs,
        }) as Box<dyn Bits + 'a>
    })
}

// -----------------------------------------------------------------------------
// Float vector values
// -----------------------------------------------------------------------------

/// Access to per-document float vector values.
///
/// Equivalent to `org.apache.lucene.index.FloatVectorValues`.
///
/// # Encoding invariant
///
/// [`KnnVectorValues::encoding`] must return [`VectorEncoding::FLOAT32`] for
/// every implementation, which Java enforces with a non-abstract override.
/// Rust cannot make a supertrait method final, so the invariant is stated here
/// and checked by unit tests over every implementation in this crate.
pub trait FloatVectorValues: KnnVectorValues {
    /// Returns the vector value for the given ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `ord` is outside
    /// `[0, size())`, which is Java's `IndexOutOfBoundsException`, or an I/O
    /// error when the value cannot be read.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>>;

    /// Reads the vector value for `ord` into a caller-provided buffer.
    ///
    /// Java returns an internal array that "may be shared across calls"; the
    /// Rust equivalent hands allocation control to the caller. The default
    /// implementation goes through [`vector_value`](Self::vector_value);
    /// implementations backed by an input should override it to read straight
    /// into `dest`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `dest` is not exactly
    /// `dimension()` long, or whatever [`vector_value`](Self::vector_value)
    /// returns.
    fn vector_value_into(&self, ord: i32, dest: &mut [f32]) -> Result<()> {
        let value = self.vector_value(ord)?;
        if dest.len() != value.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination length {} does not match vector dimension {}",
                dest.len(),
                value.len()
            )));
        }
        dest.copy_from_slice(&value);
        Ok(())
    }

    /// Returns a fresh copy of this instance, keeping the float value accessor.
    ///
    /// Equivalent to `FloatVectorValues.copy()`, whose covariant return type
    /// Rust cannot express on [`KnnVectorValues::copy`].
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying input cannot be cloned.
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>>;
}

/// Checks that `field`, if it has vectors, is encoded as
/// [`VectorEncoding::FLOAT32`].
///
/// Equivalent to `FloatVectorValues.checkField(LeafReader, String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] with Lucene's message when the field
/// has vectors under a different encoding.
pub fn check_float_field(reader: &dyn LeafReader, field: &str) -> Result<()> {
    check_field_encoding(reader, field, VectorEncoding::FLOAT32)
}

/// Checks that `field`, if it has vectors, is encoded as
/// [`VectorEncoding::BYTE`].
///
/// Equivalent to `ByteVectorValues.checkField(LeafReader, String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] with Lucene's message when the field
/// has vectors under a different encoding.
pub fn check_byte_field(reader: &dyn LeafReader, field: &str) -> Result<()> {
    check_field_encoding(reader, field, VectorEncoding::BYTE)
}

fn check_field_encoding(
    reader: &dyn LeafReader,
    field: &str,
    expected: VectorEncoding,
) -> Result<()> {
    let infos = reader.get_field_infos();
    if let Some(info) = infos.field_info(field) {
        if info.has_vector_values() && info.get_vector_encoding() != expected {
            // Message verbatim from Lucene, including the missing space before
            // "(expected=".
            return Err(LuceneError::IllegalState(format!(
                "Unexpected vector encoding ({:?}) for field {}(expected={:?})",
                info.get_vector_encoding(),
                field,
                expected
            )));
        }
    }
    Ok(())
}

/// Float vector values backed by an in-memory list.
///
/// Equivalent to the instance returned by
/// `FloatVectorValues.fromFloats(List<float[]>, int)`.
#[derive(Debug, Clone)]
pub struct ListFloatVectorValues {
    vectors: Arc<Vec<Vec<f32>>>,
    dimension: i32,
}

/// Creates [`FloatVectorValues`] over an in-memory list of vectors.
///
/// Equivalent to `FloatVectorValues.fromFloats(List<float[]>, int)`.
pub fn from_floats(vectors: Vec<Vec<f32>>, dim: i32) -> ListFloatVectorValues {
    ListFloatVectorValues {
        vectors: Arc::new(vectors),
        dimension: dim,
    }
}

impl KnnVectorValues for ListFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.vectors.len() as i32
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(DenseDocIndexIterator::new(self.size())))
    }
}

impl FloatVectorValues for ListFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        check_ord(ord, self.vectors.len())?;
        Ok(self.vectors[ord as usize].clone())
    }

    /// Java's `fromFloats` returns `this` from `copy()`, because the backing
    /// list is immutable and can be shared. The [`Arc`] reproduces that: the
    /// clone shares the same vectors instead of duplicating them.
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

// -----------------------------------------------------------------------------
// Byte vector values
// -----------------------------------------------------------------------------

/// Access to per-document byte vector values.
///
/// Equivalent to `org.apache.lucene.index.ByteVectorValues`.
///
/// # Encoding invariant
///
/// [`KnnVectorValues::encoding`] must return [`VectorEncoding::BYTE`] for every
/// implementation; see the note on [`FloatVectorValues`] for why Rust cannot
/// enforce it in the type system.
pub trait ByteVectorValues: KnnVectorValues {
    /// Returns the vector value for the given ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `ord` is outside
    /// `[0, size())`, or an I/O error when the value cannot be read.
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>>;

    /// Reads the vector value for `ord` into a caller-provided buffer.
    ///
    /// See [`FloatVectorValues::vector_value_into`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `dest` is not exactly
    /// `dimension()` long, or whatever [`vector_value`](Self::vector_value)
    /// returns.
    fn vector_value_into(&self, ord: i32, dest: &mut [u8]) -> Result<()> {
        let value = self.vector_value(ord)?;
        if dest.len() != value.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination length {} does not match vector dimension {}",
                dest.len(),
                value.len()
            )));
        }
        dest.copy_from_slice(&value);
        Ok(())
    }

    /// Returns a fresh copy of this instance, keeping the byte value accessor.
    ///
    /// Equivalent to `ByteVectorValues.copy()`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying input cannot be cloned.
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>>;
}

/// Byte vector values backed by an in-memory list.
///
/// Equivalent to the instance returned by
/// `ByteVectorValues.fromBytes(List<byte[]>, int)`.
#[derive(Debug, Clone)]
pub struct ListByteVectorValues {
    vectors: Arc<Vec<Vec<u8>>>,
    dimension: i32,
}

/// Creates [`ByteVectorValues`] over an in-memory list of vectors.
///
/// Equivalent to `ByteVectorValues.fromBytes(List<byte[]>, int)`.
pub fn from_bytes(vectors: Vec<Vec<u8>>, dim: i32) -> ListByteVectorValues {
    ListByteVectorValues {
        vectors: Arc::new(vectors),
        dimension: dim,
    }
}

impl KnnVectorValues for ListByteVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.vectors.len() as i32
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(DenseDocIndexIterator::new(self.size())))
    }
}

impl ByteVectorValues for ListByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        check_ord(ord, self.vectors.len())?;
        Ok(self.vectors[ord as usize].clone())
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

/// Rejects an ordinal outside `[0, size)`.
///
/// Equivalent to the `IndexOutOfBoundsException` that Java's
/// `vectorValue(int)` is documented to throw.
fn check_ord(ord: i32, size: usize) -> Result<()> {
    if ord < 0 || ord as usize >= size {
        return Err(LuceneError::IllegalArgument(format!(
            "ordinal {ord} out of range [0, {size})"
        )));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Empty implementations
// -----------------------------------------------------------------------------

/// A no-op KNN vector values instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyKnnVectorValues;

impl KnnVectorValues for EmptyKnnVectorValues {
    fn dimension(&self) -> i32 {
        0
    }

    fn size(&self) -> i32 {
        0
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(*self))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(DenseDocIndexIterator::new(0)))
    }
}

/// A no-op float vector values instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyFloatVectorValues;

impl KnnVectorValues for EmptyFloatVectorValues {
    fn dimension(&self) -> i32 {
        0
    }

    fn size(&self) -> i32 {
        0
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(*self))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(DenseDocIndexIterator::new(0)))
    }
}

impl FloatVectorValues for EmptyFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        check_ord(ord, 0)?;
        unreachable!("INVARIANT: size() is 0, so every ordinal is rejected above")
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(*self))
    }
}

/// A no-op byte vector values instance.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyByteVectorValues;

impl KnnVectorValues for EmptyByteVectorValues {
    fn dimension(&self) -> i32 {
        0
    }

    fn size(&self) -> i32 {
        0
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(*self))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(DenseDocIndexIterator::new(0)))
    }
}

impl ByteVectorValues for EmptyByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        check_ord(ord, 0)?;
        unreachable!("INVARIANT: size() is 0, so every ordinal is rejected above")
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(*self))
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;
    use crate::util::MatchAllBits;

    /// Sparse doc-id iterator used to drive [`FromDisiDocIndexIterator`].
    #[derive(Debug, Clone)]
    struct VecDocs {
        docs: Vec<i32>,
        pos: i32,
    }

    impl VecDocs {
        fn new(docs: Vec<i32>) -> Self {
            Self { docs, pos: -1 }
        }
    }

    impl DocIdSetIterator for VecDocs {
        fn doc_id(&self) -> i32 {
            if self.pos < 0 {
                -1
            } else if self.pos as usize >= self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.pos as usize]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.pos += 1;
            Ok(self.doc_id())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            loop {
                let doc = self.next_doc()?;
                if doc >= target {
                    return Ok(doc);
                }
            }
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    /// Bits accepting only the doc IDs listed at construction.
    #[derive(Debug)]
    struct AcceptSet {
        accepted: Vec<i32>,
        length: usize,
    }

    impl Bits for AcceptSet {
        fn get(&self, index: usize) -> bool {
            self.accepted.contains(&(index as i32))
        }

        fn length(&self) -> usize {
            self.length
        }
    }

    #[test]
    fn float_vector_values_contract() {
        let values = from_floats(vec![vec![1.0, 2.0], vec![3.0, 4.0], vec![5.0, 6.0]], 2);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.size(), 3);
        assert_eq!(values.encoding(), VectorEncoding::FLOAT32);
        assert_eq!(values.vector_byte_length(), 8);
        assert_eq!(values.vector_value(1).unwrap(), vec![3.0, 4.0]);
        assert_eq!(values.ord_to_doc(2), 2);
    }

    #[test]
    fn byte_vector_values_contract() {
        let values = from_bytes(vec![vec![1, 2], vec![3, 4], vec![5, 6]], 2);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.size(), 3);
        assert_eq!(values.encoding(), VectorEncoding::BYTE);
        assert_eq!(values.vector_byte_length(), 2);
        assert_eq!(values.vector_value(1).unwrap(), vec![3, 4]);
    }

    #[test]
    fn vector_value_rejects_out_of_range_ordinals() {
        let values = from_floats(vec![vec![1.0], vec![2.0]], 1);
        assert!(values.vector_value(-1).is_err());
        assert!(values.vector_value(2).is_err());
        assert!(values.vector_value(1).is_ok());

        let values = from_bytes(vec![vec![1]], 1);
        assert!(values.vector_value(-1).is_err());
        assert!(values.vector_value(1).is_err());
    }

    #[test]
    fn vector_value_into_writes_into_the_caller_buffer() {
        let values = from_floats(vec![vec![1.0, 2.0], vec![3.0, 4.0]], 2);
        let mut dest = [0.0f32; 2];
        values.vector_value_into(1, &mut dest).unwrap();
        assert_eq!(dest, [3.0, 4.0]);
        // A mis-sized destination is rejected rather than silently truncated.
        let mut short = [0.0f32; 1];
        assert!(values.vector_value_into(1, &mut short).is_err());

        let values = from_bytes(vec![vec![7, 8]], 2);
        let mut dest = [0u8; 2];
        values.vector_value_into(0, &mut dest).unwrap();
        assert_eq!(dest, [7, 8]);
    }

    #[test]
    fn typed_copies_preserve_the_value_accessor() {
        let values = from_floats(vec![vec![1.0, 2.0]], 2);
        let copy = values.copy_float().unwrap();
        assert_eq!(copy.vector_value(0).unwrap(), vec![1.0, 2.0]);
        assert_eq!(copy.encoding(), VectorEncoding::FLOAT32);

        let values = from_bytes(vec![vec![9, 10]], 2);
        let copy = values.copy_byte().unwrap();
        assert_eq!(copy.vector_value(0).unwrap(), vec![9, 10]);
        assert_eq!(copy.encoding(), VectorEncoding::BYTE);
    }

    #[test]
    fn dense_doc_index_iterator_contract() {
        let mut it = DenseDocIndexIterator::new(4);
        assert_eq!(it.doc_id(), -1);
        assert_eq!(it.index(), -1);
        for expected in 0..4 {
            assert_eq!(it.next_doc().unwrap(), expected);
            assert_eq!(it.index(), expected);
            assert_eq!(it.doc_id(), expected);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.cost(), 4);
    }

    #[test]
    fn dense_doc_index_iterator_advance_matches_java() {
        let mut it = DenseDocIndexIterator::new(4);
        assert_eq!(it.advance(3).unwrap(), 3);
        assert_eq!(it.index(), 3);
        // Past the end is exhausted.
        let mut it = DenseDocIndexIterator::new(4);
        assert_eq!(it.advance(4).unwrap(), NO_MORE_DOCS);
        assert_eq!(it.doc_id(), NO_MORE_DOCS);
    }

    /// Java does not override `docIDRunEnd()` on the dense iterator, so it
    /// inherits `docID() + 1`. Rucene previously returned `size`, which is
    /// legal under the contract but is not the value Lucene 10.5.0 produces.
    #[test]
    fn dense_doc_index_iterator_run_end_is_doc_plus_one() {
        let mut it = DenseDocIndexIterator::new(4);
        it.next_doc().unwrap();
        assert_eq!(it.doc_id_run_end().unwrap(), 1);
        it.next_doc().unwrap();
        assert_eq!(it.doc_id_run_end().unwrap(), 2);
    }

    /// Regression for the `fromDISI` divergence: Java's `advance` forwards to
    /// the delegate and never touches the ordinal, so `index()` keeps whatever
    /// value the last `nextDoc()` left behind.
    #[test]
    fn from_disi_advance_does_not_touch_the_ordinal() {
        let mut it = FromDisiDocIndexIterator::new(Box::new(VecDocs::new(vec![0, 5, 9, 12])));
        assert_eq!(it.index(), -1);
        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.index(), 0);
        assert_eq!(it.next_doc().unwrap(), 5);
        assert_eq!(it.index(), 1);

        // Jump over doc 9 straight to 12: Java leaves ord at 1.
        assert_eq!(it.advance(12).unwrap(), 12);
        assert_eq!(
            it.index(),
            1,
            "advance must leave the ordinal stale, exactly as Lucene does"
        );

        // Sequential access resumes incrementing from the stale value.
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.index(), 2);
    }

    #[test]
    fn from_disi_next_doc_stops_at_no_more_docs() {
        let mut it = FromDisiDocIndexIterator::new(Box::new(VecDocs::new(vec![3])));
        assert_eq!(it.next_doc().unwrap(), 3);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.index(), 1);
        // Once exhausted the ordinal freezes, because `nextDoc` returns early.
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.index(), 1);
        assert_eq!(it.cost(), 1);
    }

    #[test]
    fn sparse_doc_index_iterator_contract() {
        let docs = vec![2, 4, 9];
        let mapping = docs.clone();
        let mut it = SparseDocIndexIterator::new(
            docs.len() as i32,
            Arc::new(move |ord| mapping[ord as usize]),
        );

        assert_eq!(it.doc_id(), -1);
        assert_eq!(it.index(), -1);
        assert_eq!(it.next_doc().unwrap(), 2);
        assert_eq!(it.index(), 0);
        assert_eq!(it.next_doc().unwrap(), 4);
        assert_eq!(it.index(), 1);
        assert_eq!(it.next_doc().unwrap(), 9);
        assert_eq!(it.index(), 2);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.index(), NO_MORE_DOCS);
        assert_eq!(it.doc_id(), NO_MORE_DOCS);
        assert_eq!(it.cost(), 3);
    }

    #[test]
    fn sparse_doc_index_iterator_advances_by_scanning() {
        let docs = vec![2, 4, 9];
        let mapping = docs.clone();
        let mut it = SparseDocIndexIterator::new(
            docs.len() as i32,
            Arc::new(move |ord| mapping[ord as usize]),
        );
        // slowAdvance stops at the first doc >= target, and the ordinal follows.
        assert_eq!(it.advance(4).unwrap(), 4);
        assert_eq!(it.index(), 1);
        assert_eq!(it.advance(5).unwrap(), 9);
        assert_eq!(it.index(), 2);
        assert_eq!(it.advance(10).unwrap(), NO_MORE_DOCS);
        assert_eq!(it.index(), NO_MORE_DOCS);
    }

    #[test]
    fn sparse_doc_index_iterator_over_values_uses_ord_to_doc() {
        let values: Arc<dyn KnnVectorValues> = Arc::new(from_floats(vec![vec![1.0], vec![2.0]], 1));
        let mut it = SparseDocIndexIterator::over(values);
        // The default `ord_to_doc` is the identity, so this behaves densely.
        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.next_doc().unwrap(), 1);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn sparse_doc_index_iterator_over_an_empty_field_is_exhausted() {
        let mut it = SparseDocIndexIterator::new(0, Arc::new(|ord| ord));
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn accept_ords_is_none_when_no_filter_is_given() {
        let values = from_floats(vec![vec![1.0], vec![2.0]], 1);
        assert!(accept_ords(&values, None).is_none());
    }

    #[test]
    fn accept_ords_maps_ordinals_through_ord_to_doc() {
        let values = from_floats(vec![vec![1.0], vec![2.0], vec![3.0]], 1);
        let all: MatchAllBits = MatchAllBits::new(3);
        let bits = accept_ords(&values, Some(&all)).unwrap();
        assert_eq!(bits.length(), 3);
        assert!(bits.get(0) && bits.get(1) && bits.get(2));

        let subset = AcceptSet {
            accepted: vec![1],
            length: 3,
        };
        let bits = accept_ords(&values, Some(&subset)).unwrap();
        assert!(!bits.get(0));
        assert!(bits.get(1));
        assert!(!bits.get(2));
    }

    #[test]
    fn prefetch_defaults_to_a_no_op() {
        let values = from_floats(vec![vec![1.0]], 1);
        values.prefetch(&[0], 1).unwrap();
    }

    #[test]
    fn iterator_defaults_to_unsupported() {
        struct Bare;
        impl KnnVectorValues for Bare {
            fn dimension(&self) -> i32 {
                4
            }
            fn size(&self) -> i32 {
                1
            }
            fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
                Ok(Box::new(Bare))
            }
            fn encoding(&self) -> VectorEncoding {
                VectorEncoding::FLOAT32
            }
        }
        assert!(matches!(
            Bare.iterator(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert_eq!(Bare.vector_byte_length(), 16);
    }

    #[test]
    fn empty_float_vector_values_is_empty() {
        let values = EmptyFloatVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        assert!(values.vector_value(0).is_err());
        let mut it = values.iterator().unwrap();
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(values.copy_float().unwrap().size(), 0);
    }

    #[test]
    fn empty_byte_vector_values_is_empty() {
        let values = EmptyByteVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        assert_eq!(values.encoding(), VectorEncoding::BYTE);
        assert!(values.vector_value(0).is_err());
        assert_eq!(values.copy_byte().unwrap().size(), 0);
    }

    #[test]
    fn empty_knn_vector_values_copy_is_empty() {
        let values = EmptyKnnVectorValues;
        let copy = values.copy().unwrap();
        assert_eq!(copy.size(), 0);
    }

    /// Java fixes `getEncoding()` on `FloatVectorValues`/`ByteVectorValues`.
    /// Rust cannot make a supertrait method final, so the invariant is pinned
    /// here for every implementation this module owns.
    #[test]
    fn typed_values_report_their_fixed_encoding() {
        let float_values: Vec<Box<dyn FloatVectorValues>> = vec![
            Box::new(from_floats(vec![vec![1.0]], 1)),
            Box::new(EmptyFloatVectorValues),
        ];
        for values in float_values {
            assert_eq!(values.encoding(), VectorEncoding::FLOAT32);
        }

        let byte_values: Vec<Box<dyn ByteVectorValues>> = vec![
            Box::new(from_bytes(vec![vec![1]], 1)),
            Box::new(EmptyByteVectorValues),
        ];
        for values in byte_values {
            assert_eq!(values.encoding(), VectorEncoding::BYTE);
        }
    }
}
