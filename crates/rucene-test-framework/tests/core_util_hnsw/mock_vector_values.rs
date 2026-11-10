//! Ports of `lucene/core/src/test/org/apache/lucene/util/hnsw/MockVectorValues.java`
//! and `MockByteVectorValues.java`, plus the shared handle the HNSW graph
//! tests pass to the flat vector scorer.
//!
//! * Java's `float[][]`/`byte[][]` with `null` rows is a
//!   `Vec<Option<Vec<_>>>`.
//! * `vectorValue(ord)` returns a fresh `Vec` in Rust's
//!   [`FloatVectorValues`]; the random draw that picks between the row and
//!   the scratch copy is kept, and the scratch copy is made, so that the draw
//!   sequence and the copy are Java's.
//! * A `null` row: Java returns `null` on one branch and throws
//!   `NullPointerException` from `System.arraycopy` on the other; Rust's
//!   `vector_value` cannot return `null`, so both branches raise the
//!   `NullPointerException` (divergence confined to these mocks).

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use rucene::error::{LuceneError, Result};
use rucene::index::{
    ByteVectorValues, DocIndexIterator, FloatVectorValues, KnnVectorValues, VectorEncoding,
};
use rucene::search::VectorScorer;
use rucene::util::{Bits, BytesRef};

use crate::core_support::random;

/// Java's `NullPointerException` over a `null` row.
fn npe() -> LuceneError {
    Throwable::new("java.lang.NullPointerException", "values[ord] is null").throw()
}

/// `MockVectorValues extends FloatVectorValues`.
#[derive(Debug)]
pub struct MockVectorValues {
    dimension: i32,
    dense_values: Vec<Option<Vec<f32>>>,
    /// `protected final float[][] values`.
    pub values: Vec<Option<Vec<f32>>>,
    num_vectors: i32,
    scratch: Mutex<Vec<f32>>,
}

impl MockVectorValues {
    /// `static MockVectorValues fromValues(float[][] values)`.
    pub fn from_values(values: Vec<Option<Vec<f32>>>) -> MockVectorValues {
        let mut first_non_null: Option<&Vec<f32>> = None;
        let mut j = 0;
        while first_non_null.is_none() && j < values.len() {
            first_non_null = values[j].as_ref();
            j += 1;
        }
        // `firstNonNull.length`: a NullPointerException when every row is null.
        let dimension = first_non_null
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
            .len() as i32;
        let max_doc = values.len();
        let mut dense_values: Vec<Option<Vec<f32>>> = vec![None; max_doc];
        let mut count = 0;
        for i in 0..max_doc {
            if values[i].is_some() {
                dense_values[count] = values[i].clone();
                count += 1;
            }
        }
        MockVectorValues::new(values, dimension, dense_values, count as i32)
    }

    /// `MockVectorValues(float[][], int, float[][], int)`.
    pub fn new(
        values: Vec<Option<Vec<f32>>>,
        dimension: i32,
        dense_values: Vec<Option<Vec<f32>>>,
        num_vectors: i32,
    ) -> MockVectorValues {
        MockVectorValues {
            dimension,
            values,
            dense_values,
            num_vectors,
            scratch: Mutex::new(vec![0f32; dimension as usize]),
        }
    }

    /// `copy()` with its covariant `MockVectorValues` return type.
    pub fn copy_mock(&self) -> MockVectorValues {
        MockVectorValues::new(
            self.values.clone(),
            self.dimension,
            self.dense_values.clone(),
            self.num_vectors,
        )
    }
}

impl KnnVectorValues for MockVectorValues {
    fn size(&self) -> i32 {
        self.values.len() as i32
    }

    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.copy_mock()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_dense_iterator())
    }
}

impl FloatVectorValues for MockVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        if random().next_boolean() {
            self.values[ord as usize].clone().ok_or_else(npe)
        } else {
            // Sometimes use the same scratch array repeatedly, mimicing what the codec will do.
            // This should help us catch cases of aliasing where the same vector values source is
            // used twice in a single computation.
            let row = self.values[ord as usize].as_ref().ok_or_else(npe)?;
            let mut scratch = self.scratch.lock().unwrap_or_else(|e| e.into_inner());
            scratch[..self.dimension as usize].copy_from_slice(&row[..self.dimension as usize]);
            Ok(scratch.clone())
        }
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.copy_mock()))
    }
}

/// `MockByteVectorValues extends ByteVectorValues`.
#[derive(Debug)]
pub struct MockByteVectorValues {
    dimension: i32,
    dense_values: Vec<Option<Vec<u8>>>,
    /// `protected final byte[][] values`.
    pub values: Vec<Option<Vec<u8>>>,
    num_vectors: i32,
    /// `binaryValue`: used by tests that build a graph from bytes rather than floats.
    #[allow(dead_code)]
    binary_value: BytesRef,
    scratch: Mutex<Vec<u8>>,
}

impl MockByteVectorValues {
    /// `static MockByteVectorValues fromValues(byte[][] values)`.
    pub fn from_values(values: Vec<Option<Vec<u8>>>) -> MockByteVectorValues {
        let mut first_non_null: Option<&Vec<u8>> = None;
        let mut j = 0;
        while first_non_null.is_none() && j < values.len() {
            first_non_null = values[j].as_ref();
            j += 1;
        }
        let dimension = first_non_null
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
            .len() as i32;
        let max_doc = values.len();
        let mut dense_values: Vec<Option<Vec<u8>>> = vec![None; max_doc];
        let mut count = 0;
        for i in 0..max_doc {
            if values[i].is_some() {
                dense_values[count] = values[i].clone();
                count += 1;
            }
        }
        MockByteVectorValues::new(values, dimension, dense_values, count as i32)
    }

    /// `MockByteVectorValues(byte[][], int, byte[][], int)`.
    pub fn new(
        values: Vec<Option<Vec<u8>>>,
        dimension: i32,
        dense_values: Vec<Option<Vec<u8>>>,
        num_vectors: i32,
    ) -> MockByteVectorValues {
        // `binaryValue = new BytesRef(dimension); binaryValue.length = dimension;`
        let binary_value = BytesRef::new(vec![0u8; dimension as usize]);
        MockByteVectorValues {
            dimension,
            values,
            dense_values,
            num_vectors,
            binary_value,
            scratch: Mutex::new(vec![0u8; dimension as usize]),
        }
    }

    /// `copy()` with its covariant `MockByteVectorValues` return type.
    pub fn copy_mock(&self) -> MockByteVectorValues {
        MockByteVectorValues::new(
            self.values.clone(),
            self.dimension,
            self.dense_values.clone(),
            self.num_vectors,
        )
    }
}

impl KnnVectorValues for MockByteVectorValues {
    fn size(&self) -> i32 {
        self.values.len() as i32
    }

    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.copy_mock()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_dense_iterator())
    }
}

impl ByteVectorValues for MockByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        if random().next_boolean() {
            self.values[ord as usize].clone().ok_or_else(npe)
        } else {
            // Sometimes use the same scratch array repeatedly, mimicing what the codec will do.
            // This should help us catch cases of aliasing where the same ByteVectorValues source
            // is used twice in a single computation.
            let row = self.values[ord as usize].as_ref().ok_or_else(npe)?;
            let mut scratch = self.scratch.lock().unwrap_or_else(|e| e.into_inner());
            scratch[..self.dimension as usize].copy_from_slice(&row[..self.dimension as usize]);
            Ok(scratch.clone())
        }
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.copy_mock()))
    }
}

/// The same `FloatVectorValues` object handed to a second owner.
///
/// **Divergence from Lucene 10.5.0 (test plumbing).** Java passes the one
/// object to `getRandomVectorScorerSupplier` and keeps using it; Rust's
/// supplier takes ownership of a box, so the box is this handle over the
/// shared object, delegating every member.
pub struct SharedFloatValues(pub Arc<dyn FloatVectorValues>);

impl std::fmt::Debug for SharedFloatValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedFloatValues").finish_non_exhaustive()
    }
}

impl KnnVectorValues for SharedFloatValues {
    fn dimension(&self) -> i32 {
        self.0.dimension()
    }
    fn try_dimension(&self) -> Result<i32> {
        self.0.try_dimension()
    }
    fn size(&self) -> i32 {
        self.0.size()
    }
    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.0.ord_to_doc(ord)
    }
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.0.get_accept_ords(accept_docs)
    }
    fn prefetch(&self, ords_to_prefetch: Option<&[i32]>, num_ords: i32) -> Result<()> {
        self.0.prefetch(ords_to_prefetch, num_ords)
    }
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        self.0.copy()
    }
    fn vector_byte_length(&self) -> i32 {
        self.0.vector_byte_length()
    }
    fn encoding(&self) -> VectorEncoding {
        self.0.encoding()
    }
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        self.0.iterator()
    }
}

impl FloatVectorValues for SharedFloatValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        self.0.vector_value(ord)
    }
    fn vector_value_into(&self, ord: i32, dest: &mut [f32]) -> Result<()> {
        self.0.vector_value_into(ord, dest)
    }
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        self.0.copy_float()
    }
    fn scorer(&self, target: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.0.scorer(target)
    }
    fn rescorer(&self, target: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.0.rescorer(target)
    }
}

/// [`SharedFloatValues`] for `ByteVectorValues`.
pub struct SharedByteValues(pub Arc<dyn ByteVectorValues>);

impl std::fmt::Debug for SharedByteValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedByteValues").finish_non_exhaustive()
    }
}

impl KnnVectorValues for SharedByteValues {
    fn dimension(&self) -> i32 {
        self.0.dimension()
    }
    fn try_dimension(&self) -> Result<i32> {
        self.0.try_dimension()
    }
    fn size(&self) -> i32 {
        self.0.size()
    }
    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.0.ord_to_doc(ord)
    }
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.0.get_accept_ords(accept_docs)
    }
    fn prefetch(&self, ords_to_prefetch: Option<&[i32]>, num_ords: i32) -> Result<()> {
        self.0.prefetch(ords_to_prefetch, num_ords)
    }
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        self.0.copy()
    }
    fn vector_byte_length(&self) -> i32 {
        self.0.vector_byte_length()
    }
    fn encoding(&self) -> VectorEncoding {
        self.0.encoding()
    }
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        self.0.iterator()
    }
}

impl ByteVectorValues for SharedByteValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        self.0.vector_value(ord)
    }
    fn vector_value_into(&self, ord: i32, dest: &mut [u8]) -> Result<()> {
        self.0.vector_value_into(ord, dest)
    }
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        self.0.copy_byte()
    }
    fn scorer(&self, target: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.0.scorer(target)
    }
    fn rescorer(&self, target: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.0.rescorer(target)
    }
}
