//! Port of `lucene/core/src/test/org/apache/lucene/util/hnsw/TestHnswByteVectorGraph.java`:
//! tests HNSW KNN graphs over `byte[]` vectors, and
//! `HnswGraphTestCase.CircularByteVectorValues`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::hnsw::flat_vectors::{DefaultFlatVectorScorer, FlatVectorsScorer};
use rucene::document::KnnByteVectorField;
use rucene::error::{LuceneError, Result};
use rucene::index::{
    ByteVectorValues, IndexableField, KnnVectorValues, LeafReader, VectorEncoding,
    VectorSimilarityFunction,
};
use rucene::search::{KnnByteVectorQuery, Query, VectorScorer, NO_MORE_DOCS};
use rucene::util::hnsw::{RandomVectorScorer, RandomVectorScorerSupplier};

use crate::core_support::assert_array_equals_msg;
use crate::hnsw_graph_test_case::{
    class_of, create_random_byte_vectors, float_to_byte, random_vector8, unit_vector2d_into,
    CopyMock, HnswCase,
};
use crate::mock_vector_values::{MockByteVectorValues, SharedByteValues};

/// `TestHnswByteVectorGraph extends HnswGraphTestCase<byte[]>`.
pub struct ByteCase;

impl CopyMock for MockByteVectorValues {
    fn copy_mock_values(&self) -> Self {
        self.copy_mock()
    }
}

/// `static boolean fitsInByte(float v)`.
fn fits_in_byte(v: f32) -> bool {
    v <= 127.0 && v >= -128.0 && v % 1.0 == 0.0
}

impl HnswCase for ByteCase {
    type V = Vec<u8>;
    type Values = dyn ByteVectorValues;
    type Mock = MockByteVectorValues;

    const CLASS_NAME: &'static str = "org.apache.lucene.util.hnsw.TestHnswByteVectorGraph";

    fn get_vector_encoding() -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn knn_query(field: &str, vector: &Vec<u8>, k: i32) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(KnnByteVectorQuery::new(field, vector.clone(), k)?))
    }

    fn random_vector(dim: i32) -> Vec<u8> {
        random_vector8(dim)
    }

    fn vector_values(size: i32, dimension: i32) -> Arc<MockByteVectorValues> {
        Arc::new(MockByteVectorValues::from_values(
            create_random_byte_vectors(size, dimension),
        ))
    }

    fn vector_values_from(values: &[Vec<f32>]) -> Arc<MockByteVectorValues> {
        let mut b_values: Vec<Option<Vec<u8>>> = Vec::with_capacity(values.len());
        // The case when all floats fit within a byte already.
        let scale_simple = fits_in_byte(values[0][0]);
        for i in 0..values.len() {
            let mut row = vec![0u8; values[i].len()];
            for j in 0..values[i].len() {
                let v = if scale_simple {
                    debug_assert!(fits_in_byte(values[i][j]));
                    values[i][j]
                } else {
                    values[i][j] * 127.0
                };
                row[j] = float_to_byte(v);
            }
            b_values.push(Some(row));
        }
        Arc::new(MockByteVectorValues::from_values(b_values))
    }

    fn vector_values_pregen(
        size: i32,
        dimension: i32,
        pregenerated_vector_values: &MockByteVectorValues,
        pregenerated_offset: i32,
    ) -> Arc<MockByteVectorValues> {
        let pvv = pregenerated_vector_values;
        let mut vectors: Vec<Option<Vec<u8>>> = vec![None; size as usize];
        let random_vectors = create_random_byte_vectors(size - pvv.values.len() as i32, dimension);

        for i in 0..pregenerated_offset as usize {
            vectors[i] = random_vectors[i].clone();
        }

        for current_ord in 0..pvv.size() as usize {
            vectors[pregenerated_offset as usize + current_ord] = pvv.values[current_ord].clone();
        }

        for i in pregenerated_offset as usize + pvv.values.len()..vectors.len() {
            vectors[i] = random_vectors[i - pvv.values.len()].clone();
        }

        Arc::new(MockByteVectorValues::from_values(vectors))
    }

    fn vector_values_reader(
        reader: &dyn LeafReader,
        field_name: &str,
    ) -> Result<Arc<MockByteVectorValues>> {
        let vector_values = reader
            .get_byte_vector_values(field_name)?
            .ok_or_else(|| LuceneError::null_pointer_with_message("null ByteVectorValues"))?;
        let mut vectors: Vec<Option<Vec<u8>>> = vec![None; reader.max_doc() as usize];
        for i in 0..vector_values.size() {
            let value = vector_values.vector_value(i)?;
            vectors[vector_values.ord_to_doc(i) as usize] =
                Some(value[..vector_values.dimension() as usize].to_vec());
        }
        Ok(Arc::new(MockByteVectorValues::from_values(vectors)))
    }

    fn knn_vector_field(
        name: &str,
        vector: &Vec<u8>,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(KnnByteVectorField::new(
            name,
            vector,
            similarity_function,
        )?))
    }

    fn circular_vector_values(n_doc: i32) -> Arc<dyn ByteVectorValues> {
        Arc::new(CircularByteVectorValues::new(n_doc))
    }

    fn get_target_vector() -> Vec<u8> {
        vec![1, 0]
    }

    fn mock_values(mock: Arc<MockByteVectorValues>) -> Arc<dyn ByteVectorValues> {
        mock
    }

    fn value_of(vectors: &dyn ByteVectorValues, ord: i32) -> Result<Vec<u8>> {
        vectors.vector_value(ord)
    }

    fn copy_values(vectors: &dyn ByteVectorValues) -> Result<Box<dyn ByteVectorValues>> {
        vectors.copy_byte()
    }

    fn share(vectors: &Arc<dyn ByteVectorValues>) -> Box<dyn ByteVectorValues> {
        Box::new(SharedByteValues(Arc::clone(vectors)))
    }

    fn as_knn(vectors: &dyn ByteVectorValues) -> &dyn KnnVectorValues {
        vectors
    }

    fn compare(sim: VectorSimilarityFunction, a: &Vec<u8>, b: &Vec<u8>) -> Result<f32> {
        sim.compare_bytes(a, b)
    }

    fn supplier(
        flat: &DefaultFlatVectorScorer,
        sim: VectorSimilarityFunction,
        vectors: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        flat.get_random_vector_scorer_supplier_byte(sim, vectors)
    }

    fn scorer(
        flat: &DefaultFlatVectorScorer,
        sim: VectorSimilarityFunction,
        vectors: Box<dyn ByteVectorValues>,
        query: &Vec<u8>,
    ) -> Result<Box<dyn RandomVectorScorer>> {
        flat.get_random_vector_scorer_byte(sim, vectors, query)
    }

    fn assert_vector_equals(message: &str, expected: &Vec<u8>, actual: &Vec<u8>) {
        let e: Vec<i8> = expected.iter().map(|&b| b as i8).collect();
        let a: Vec<i8> = actual.iter().map(|&b| b as i8).collect();
        assert_array_equals_msg(Some(message), &e, &a);
    }
}

/// Returns vectors evenly distributed around the upper unit semicircle.
///
/// `HnswGraphTestCase.CircularByteVectorValues`; `docID()`, `nextDoc()`
/// and `advance(int)` are its own public methods, kept as inherent methods.
// Java declares `docID()`, `nextDoc()`, `advance()` and `vectorValue()`,
// which no test calls.
#[allow(dead_code)]
#[derive(Debug)]
pub struct CircularByteVectorValues {
    size: i32,
    /// `value` and `bValue`, the scratch arrays.
    scratch: Mutex<(Vec<f32>, Vec<u8>)>,
    doc: i32,
}

#[allow(dead_code)]
impl CircularByteVectorValues {
    /// `CircularByteVectorValues(int size)`.
    pub fn new(size: i32) -> Self {
        CircularByteVectorValues {
            size,
            scratch: Mutex::new((vec![0f32; 2], vec![0u8; 2])),
            doc: -1,
        }
    }

    /// `vectorValue()`.
    pub fn vector_value_current(&self) -> Result<Vec<u8>> {
        self.vector_value(self.doc)
    }

    /// `docID()`.
    pub fn doc_id(&self) -> i32 {
        self.doc
    }

    /// `nextDoc()`.
    pub fn next_doc(&mut self) -> i32 {
        self.advance(self.doc + 1)
    }

    /// `advance(target)`.
    pub fn advance(&mut self, target: i32) -> i32 {
        if target >= 0 && target < self.size {
            self.doc = target;
        } else {
            self.doc = NO_MORE_DOCS;
        }
        self.doc
    }
}

impl KnnVectorValues for CircularByteVectorValues {
    fn dimension(&self) -> i32 {
        2
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(CircularByteVectorValues::new(self.size)))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }
}

impl ByteVectorValues for CircularByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        let mut scratch = self.scratch.lock().unwrap_or_else(|e| e.into_inner());
        let (value, b_value) = &mut *scratch;
        unit_vector2d_into(ord as f64 / self.size as f64, value);
        for i in 0..value.len() {
            b_value[i] = float_to_byte(value[i] * 127.0);
        }
        Ok(b_value.clone())
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(CircularByteVectorValues::new(self.size)))
    }

    fn scorer(&self, _target: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }
}

/// `TestHnswByteVectorGraph`.
pub fn class_def() -> Arc<Class> {
    class_of::<ByteCase>(Vec::new())
}
