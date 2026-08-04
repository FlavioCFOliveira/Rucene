//! Vector value accessors ported from `org.apache.lucene.index`.
//!
//! Equivalent to `org.apache.lucene.index.ByteVectorValues`,
//! `FloatVectorValues`, `KnnVectorValues` and `VectorValues`.
//!
//! Vectors are addressed by an ordinal (`ord`). The base [`KnnVectorValues`]
//! trait provides dimension and size metadata, while [`FloatVectorValues`]
//! and [`ByteVectorValues`] add type-specific value access. Iteration over
//! the document IDs that have vectors is provided by [`DocIndexIterator`].

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::VectorEncoding;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};

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

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target >= self.size {
            self.doc = NO_MORE_DOCS;
        } else {
            self.doc = target.max(0);
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.size as i64
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        Ok(self.size)
    }
}

impl DocIndexIterator for DenseDocIndexIterator {
    fn index(&self) -> i32 {
        self.doc
    }
}

/// Iterator over a sparse vector field whose ordinals increase monotonically
/// with doc ID and are provided by a delegate doc-id iterator.
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
        let doc = self.docs.advance(target)?;
        if doc != NO_MORE_DOCS {
            // We cannot recover the ord after a jump; this implementation is
            // only correct when used with sequential access. A real codec keeps
            // the correspondence explicitly.
            self.ord += 1;
        }
        Ok(doc)
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

    /// Returns a copy of this vector-values instance.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>>;

    /// Returns the vector byte length.
    fn vector_byte_length(&self) -> i32 {
        self.dimension() * self.encoding().byte_size()
    }

    /// Returns the vector encoding.
    fn encoding(&self) -> VectorEncoding;

    /// Returns an iterator over the document IDs that have vectors.
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>>;
}

// -----------------------------------------------------------------------------
// Float vector values
// -----------------------------------------------------------------------------

/// Iterator over float vector values.
///
/// Equivalent to `org.apache.lucene.index.FloatVectorValues`.
pub trait FloatVectorValues: KnnVectorValues {
    /// Returns the vector value for the given ordinal.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>>;
}

// -----------------------------------------------------------------------------
// Byte vector values
// -----------------------------------------------------------------------------

/// Iterator over byte vector values.
///
/// Equivalent to `org.apache.lucene.index.ByteVectorValues`.
pub trait ByteVectorValues: KnnVectorValues {
    /// Returns the vector value for the given ordinal.
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>>;
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
    fn vector_value(&self, _ord: i32) -> Result<Vec<f32>> {
        Ok(Vec::new())
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
    fn vector_value(&self, _ord: i32) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;

    /// Float vector values backed by a list of vectors.
    struct VecFloatVectorValues {
        vectors: Vec<Vec<f32>>,
        dimension: i32,
    }

    impl VecFloatVectorValues {
        fn new(vectors: Vec<Vec<f32>>, dimension: i32) -> Self {
            Self { vectors, dimension }
        }
    }

    impl KnnVectorValues for VecFloatVectorValues {
        fn dimension(&self) -> i32 {
            self.dimension
        }

        fn size(&self) -> i32 {
            self.vectors.len() as i32
        }

        fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
            Ok(Box::new(Self {
                vectors: self.vectors.clone(),
                dimension: self.dimension,
            }))
        }

        fn encoding(&self) -> VectorEncoding {
            VectorEncoding::FLOAT32
        }

        fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
            Ok(Box::new(DenseDocIndexIterator::new(self.size())))
        }
    }

    impl FloatVectorValues for VecFloatVectorValues {
        fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
            if ord < 0 || ord as usize >= self.vectors.len() {
                return Err(crate::error::LuceneError::IllegalArgument(format!(
                    "ordinal {ord} out of range [0, {})",
                    self.vectors.len()
                )));
            }
            Ok(self.vectors[ord as usize].clone())
        }
    }

    /// Byte vector values backed by a list of vectors.
    struct VecByteVectorValues {
        vectors: Vec<Vec<u8>>,
        dimension: i32,
    }

    impl VecByteVectorValues {
        fn new(vectors: Vec<Vec<u8>>, dimension: i32) -> Self {
            Self { vectors, dimension }
        }
    }

    impl KnnVectorValues for VecByteVectorValues {
        fn dimension(&self) -> i32 {
            self.dimension
        }

        fn size(&self) -> i32 {
            self.vectors.len() as i32
        }

        fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
            Ok(Box::new(Self {
                vectors: self.vectors.clone(),
                dimension: self.dimension,
            }))
        }

        fn encoding(&self) -> VectorEncoding {
            VectorEncoding::BYTE
        }

        fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
            Ok(Box::new(DenseDocIndexIterator::new(self.size())))
        }
    }

    impl ByteVectorValues for VecByteVectorValues {
        fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
            if ord < 0 || ord as usize >= self.vectors.len() {
                return Err(crate::error::LuceneError::IllegalArgument(format!(
                    "ordinal {ord} out of range [0, {})",
                    self.vectors.len()
                )));
            }
            Ok(self.vectors[ord as usize].clone())
        }
    }

    #[test]
    fn float_vector_values_contract() {
        let values =
            VecFloatVectorValues::new(vec![vec![1.0, 2.0], vec![3.0, 4.0], vec![5.0, 6.0]], 2);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.size(), 3);
        assert_eq!(values.encoding(), VectorEncoding::FLOAT32);
        assert_eq!(values.vector_byte_length(), 8);
        assert_eq!(values.vector_value(1).unwrap(), vec![3.0, 4.0]);
    }

    #[test]
    fn byte_vector_values_contract() {
        let values = VecByteVectorValues::new(vec![vec![1, 2], vec![3, 4], vec![5, 6]], 2);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.size(), 3);
        assert_eq!(values.encoding(), VectorEncoding::BYTE);
        assert_eq!(values.vector_byte_length(), 2);
        assert_eq!(values.vector_value(1).unwrap(), vec![3, 4]);
    }

    #[test]
    fn dense_doc_index_iterator_contract() {
        let mut it = DenseDocIndexIterator::new(4);
        assert_eq!(it.doc_id(), -1);
        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.index(), 0);
        assert_eq!(it.advance(3).unwrap(), 3);
        assert_eq!(it.index(), 3);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.cost(), 4);
    }

    #[test]
    fn empty_float_vector_values_is_empty() {
        let values = EmptyFloatVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        assert!(values.vector_value(0).unwrap().is_empty());
        let mut it = values.iterator().unwrap();
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn empty_byte_vector_values_is_empty() {
        let values = EmptyByteVectorValues;
        assert_eq!(values.dimension(), 0);
        assert_eq!(values.size(), 0);
        assert_eq!(values.encoding(), VectorEncoding::BYTE);
        assert!(values.vector_value(0).unwrap().is_empty());
    }

    #[test]
    fn empty_knn_vector_values_copy_is_empty() {
        let values = EmptyKnnVectorValues;
        let copy = values.copy().unwrap();
        assert_eq!(copy.size(), 0);
    }
}
