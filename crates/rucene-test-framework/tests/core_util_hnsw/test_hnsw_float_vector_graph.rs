//! Port of `lucene/core/src/test/org/apache/lucene/util/hnsw/TestHnswFloatVectorGraph.java`:
//! tests HNSW KNN graphs over `float[]` vectors, and
//! `HnswGraphTestCase.CircularFloatVectorValues`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::codecs::hnsw::flat_vectors::{DefaultFlatVectorScorer, FlatVectorsScorer};
use rucene::document::KnnFloatVectorField;
use rucene::error::{LuceneError, Result};
use rucene::index::{
    FloatVectorValues, IndexableField, KnnVectorValues, LeafReader, VectorEncoding,
    VectorSimilarityFunction,
};
use rucene::search::{KnnCollector, KnnFloatVectorQuery, Query, VectorScorer, NO_MORE_DOCS};
use rucene::util::hnsw::{
    HnswBuilder, HnswGraphSearcher, RandomVectorScorer, RandomVectorScorerSupplier,
};
use rucene::util::FixedBitSet;

use crate::core_support::{assert_array_equals_delta_msg, assert_equals_msg, random};
use crate::hnsw_graph_test_case::{
    class_of, create_random_float_vectors, random_vector, ti, unit_vector2d_into, CopyMock,
    Fixture, HnswCase,
};
use crate::mock_vector_values::{MockVectorValues, SharedFloatValues};

/// `TestHnswFloatVectorGraph extends HnswGraphTestCase<float[]>`.
pub struct FloatCase;

impl CopyMock for MockVectorValues {
    fn copy_mock_values(&self) -> Self {
        self.copy_mock()
    }
}

impl HnswCase for FloatCase {
    type V = Vec<f32>;
    type Values = dyn FloatVectorValues;
    type Mock = MockVectorValues;

    const CLASS_NAME: &'static str = "org.apache.lucene.util.hnsw.TestHnswFloatVectorGraph";

    fn get_vector_encoding() -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn knn_query(field: &str, vector: &Vec<f32>, k: i32) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(KnnFloatVectorQuery::new(
            field,
            vector.clone(),
            k,
        )?))
    }

    fn random_vector(dim: i32) -> Vec<f32> {
        random_vector(dim)
    }

    fn vector_values(size: i32, dimension: i32) -> Arc<MockVectorValues> {
        Arc::new(MockVectorValues::from_values(create_random_float_vectors(
            size, dimension,
        )))
    }

    fn vector_values_from(values: &[Vec<f32>]) -> Arc<MockVectorValues> {
        Arc::new(MockVectorValues::from_values(
            values.iter().cloned().map(Some).collect(),
        ))
    }

    fn vector_values_reader(
        reader: &dyn LeafReader,
        field_name: &str,
    ) -> Result<Arc<MockVectorValues>> {
        let vector_values = reader
            .get_float_vector_values(field_name)?
            .ok_or_else(|| LuceneError::null_pointer_with_message("null FloatVectorValues"))?;
        let mut vectors: Vec<Option<Vec<f32>>> = vec![None; reader.max_doc() as usize];
        for i in 0..vector_values.size() {
            let value = vector_values.vector_value(i)?;
            vectors[vector_values.ord_to_doc(i) as usize] =
                Some(value[..vector_values.dimension() as usize].to_vec());
        }
        Ok(Arc::new(MockVectorValues::from_values(vectors)))
    }

    fn vector_values_pregen(
        size: i32,
        dimension: i32,
        pregenerated_vector_values: &MockVectorValues,
        pregenerated_offset: i32,
    ) -> Arc<MockVectorValues> {
        let pvv = pregenerated_vector_values;
        let mut vectors: Vec<Option<Vec<f32>>> = vec![None; size as usize];
        let random_vectors = create_random_float_vectors(size - pvv.values.len() as i32, dimension);

        for i in 0..pregenerated_offset as usize {
            vectors[i] = random_vectors[i].clone();
        }

        for current_ord in 0..pvv.size() as usize {
            vectors[pregenerated_offset as usize + current_ord] = pvv.values[current_ord].clone();
        }

        for i in pregenerated_offset as usize + pvv.values.len()..vectors.len() {
            vectors[i] = random_vectors[i - pvv.values.len()].clone();
        }

        Arc::new(MockVectorValues::from_values(vectors))
    }

    fn knn_vector_field(
        name: &str,
        vector: &Vec<f32>,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(KnnFloatVectorField::new(
            name,
            vector,
            similarity_function,
        )?))
    }

    fn circular_vector_values(n_doc: i32) -> Arc<dyn FloatVectorValues> {
        Arc::new(CircularFloatVectorValues::new(n_doc))
    }

    fn get_target_vector() -> Vec<f32> {
        vec![1f32, 0f32]
    }

    fn mock_values(mock: Arc<MockVectorValues>) -> Arc<dyn FloatVectorValues> {
        mock
    }

    fn value_of(vectors: &dyn FloatVectorValues, ord: i32) -> Result<Vec<f32>> {
        vectors.vector_value(ord)
    }

    fn copy_values(vectors: &dyn FloatVectorValues) -> Result<Box<dyn FloatVectorValues>> {
        vectors.copy_float()
    }

    fn share(vectors: &Arc<dyn FloatVectorValues>) -> Box<dyn FloatVectorValues> {
        Box::new(SharedFloatValues(Arc::clone(vectors)))
    }

    fn as_knn(vectors: &dyn FloatVectorValues) -> &dyn KnnVectorValues {
        vectors
    }

    fn compare(sim: VectorSimilarityFunction, a: &Vec<f32>, b: &Vec<f32>) -> Result<f32> {
        sim.compare_f32(a, b)
    }

    fn supplier(
        flat: &DefaultFlatVectorScorer,
        sim: VectorSimilarityFunction,
        vectors: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        flat.get_random_vector_scorer_supplier_float(sim, vectors)
    }

    fn scorer(
        flat: &DefaultFlatVectorScorer,
        sim: VectorSimilarityFunction,
        vectors: Box<dyn FloatVectorValues>,
        query: &Vec<f32>,
    ) -> Result<Box<dyn RandomVectorScorer>> {
        flat.get_random_vector_scorer_float(sim, vectors, query)
    }

    fn assert_vector_equals(message: &str, expected: &Vec<f32>, actual: &Vec<f32>) {
        let e: Vec<f64> = expected.iter().map(|&v| v as f64).collect();
        let a: Vec<f64> = actual.iter().map(|&v| v as f64).collect();
        assert_array_equals_delta_msg(Some(message), &e, &a, 1e-4f32 as f64);
    }
}

/// Returns vectors evenly distributed around the upper unit semicircle.
///
/// `HnswGraphTestCase.CircularFloatVectorValues`; `docID()`, `nextDoc()`
/// and `advance(int)` are its own public methods (not overrides), kept as
/// inherent methods over `&mut self`.
// Java declares `docID()`, `nextDoc()`, `advance()` and `vectorValue()`,
// which no test calls.
#[allow(dead_code)]
#[derive(Debug)]
pub struct CircularFloatVectorValues {
    size: i32,
    value: Mutex<Vec<f32>>,
    doc: i32,
}

#[allow(dead_code)]
impl CircularFloatVectorValues {
    /// `CircularFloatVectorValues(int size)`.
    pub fn new(size: i32) -> Self {
        CircularFloatVectorValues {
            size,
            value: Mutex::new(vec![0f32; 2]),
            doc: -1,
        }
    }

    /// `vectorValue()`.
    pub fn vector_value_current(&self) -> Result<Vec<f32>> {
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

impl KnnVectorValues for CircularFloatVectorValues {
    fn dimension(&self) -> i32 {
        2
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(CircularFloatVectorValues::new(self.size)))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }
}

impl FloatVectorValues for CircularFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        let mut value = self.value.lock().unwrap_or_else(|e| e.into_inner());
        unit_vector2d_into(ord as f64 / self.size as f64, &mut value);
        Ok(value.clone())
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(CircularFloatVectorValues::new(self.size)))
    }

    fn scorer(&self, _target: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }
}

/// `TestHnswFloatVectorGraph`.
pub fn class_def() -> Arc<Class> {
    let own: Vec<Method> = vec![ti::<FloatCase>(
        "testSearchWithSkewedAcceptOrds",
        test_search_with_skewed_accept_ords,
    )];
    class_of::<FloatCase>(own)
}

fn test_search_with_skewed_accept_ords(this: &mut Fixture<FloatCase>) -> Result<()> {
    let n_doc = 1000;
    this.similarity_function = Some(VectorSimilarityFunction::EUCLIDEAN);
    let vectors = FloatCase::circular_vector_values(n_doc);
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = rucene::util::hnsw::HnswGraphBuilder::create(
        &*scorer_supplier,
        16,
        100,
        random().next_int() as i64 as u64,
        -1,
    )?;
    builder.build(vectors.size())?;
    let hnsw = builder.into_graph()?;

    // Skip over half of the documents that are closest to the query vector
    let mut accept_ords = FixedBitSet::new(n_doc as usize);
    for i in 500..n_doc as usize {
        accept_ords.set(i);
    }
    let mut nn = HnswGraphSearcher::search_on_heap(
        this.build_scorer(&*vectors, &FloatCase::get_target_vector())?
            .as_mut(),
        10,
        &hnsw,
        Some(&accept_ords),
        i32::MAX as i64,
    )?;

    let nodes = nn.top_docs();
    assert_equals_msg(
        Some("Number of found results is not equal to [10]."),
        &10,
        &nodes.score_docs.len(),
    );
    let mut sum = 0;
    for node in &nodes.score_docs {
        randomizedtesting::junit::Assert::assert_true_msg(
            &format!("the results include a deleted document: {node}"),
            accept_ords.get(node.doc as usize),
        );
        sum += node.doc;
    }
    // We still expect to get reasonable recall. The lowest non-skipped docIds
    // are closest to the query vector: sum(500,509) = 5045
    randomizedtesting::junit::Assert::assert_true_msg(
        &format!("sum(result docs)={sum}"),
        sum < 5100,
    );
    Ok(())
}
