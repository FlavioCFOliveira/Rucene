//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/TestFloatVectorSimilarityQuery.java`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::KnnFloatVectorField;
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexableField, LeafReaderContext, VectorSimilarityFunction};
use rucene::search::abstract_vector_similarity_query::{
    vector_similarity_create_weight, vector_similarity_query_visit, AbstractVectorSimilarityQuery,
    AbstractVectorSimilarityQueryImpl,
};
use rucene::search::doc_id_set_iterator::AcceptDocs;
use rucene::search::float_vector_similarity_query::FloatVectorSimilarityQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn::knn_collector_manager::KnnCollectorManager;
use rucene::search::knn::{KnnSearchStrategy, TopDocs};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::vector_scorer::VectorScorer;
use rucene::search::weight::Weight;
use rucene::search::Query;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::base_vector_similarity_query_test_case::{
    self as base, VectorSimilarityCase, VectorSimilarityFixture,
};
use crate::core_support::{random, unchecked};

/// `TestFloatVectorSimilarityQuery extends
/// BaseVectorSimilarityQueryTestCase<float[], KnnFloatVectorField,
/// FloatVectorSimilarityQuery>`.
pub struct FloatVectorSimilarityCase;

/// `TestFloatVectorSimilarityQuery`.
pub fn class_def() -> Arc<Class> {
    base::class_def::<FloatVectorSimilarityCase>(
        "org.apache.lucene.search.TestFloatVectorSimilarityQuery",
    )
}

/// `TestVectorUtil.randomVector(int dim)` (a public static method of the
/// core test class `org.apache.lucene.util.TestVectorUtil`, which lives in
/// another test target; reproduced here line by line).
pub fn random_vector(dim: i32) -> Vec<f32> {
    let mut v = vec![0f32; dim as usize];
    let random = random();
    for i in 0..dim as usize {
        v[i] = random.next_float();
    }
    v
}

/// `Float.floatToIntBits`: every NaN collapses to the canonical one.
fn java_float_to_int_bits(f: f32) -> u32 {
    if f.is_nan() {
        0x7fc0_0000
    } else {
        f.to_bits()
    }
}

impl VectorSimilarityCase for FloatVectorSimilarityCase {
    type V = Vec<f32>;
    type Q = FloatVectorSimilarityQuery;

    const SIMPLE_NAME: &'static str = "TestFloatVectorSimilarityQuery";

    fn setup(f: &mut VectorSimilarityFixture<Self>) {
        f.vector_field = Some(format!("{}:VectorField", Self::SIMPLE_NAME));
        f.id_field = Some(format!("{}:IdField", Self::SIMPLE_NAME));
        f.function = Some(VectorSimilarityFunction::EUCLIDEAN);
        f.num_docs = LuceneTestCase::at_least(100);
        f.dim = LuceneTestCase::at_least(5);
    }

    fn get_random_vector(dim: i32) -> Vec<f32> {
        random_vector(dim)
    }

    fn compare(f: &VectorSimilarityFixture<Self>, vector1: &Vec<f32>, vector2: &Vec<f32>) -> f32 {
        unchecked(f.function().compare_f32(vector1, vector2))
    }

    /// `Arrays.equals(float[], float[])`: `floatToIntBits` equality.
    fn check_equals(vector1: &Vec<f32>, vector2: &Vec<f32>) -> bool {
        vector1.len() == vector2.len()
            && vector1
                .iter()
                .zip(vector2)
                .all(|(a, b)| java_float_to_int_bits(*a) == java_float_to_int_bits(*b))
    }

    fn get_vector_field(
        name: &str,
        vector: &Vec<f32>,
        function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(KnnFloatVectorField::new(name, vector, function)?))
    }

    fn get_vector_query(
        field: &str,
        vector: Vec<f32>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<FloatVectorSimilarityQuery> {
        FloatVectorSimilarityQuery::with_decay(field, vector, result_similarity, decay, filter)
    }

    fn get_vector_query_with_strategy(
        field: &str,
        vector: Vec<f32>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<FloatVectorSimilarityQuery> {
        FloatVectorSimilarityQuery::with_strategy(
            field,
            vector,
            result_similarity,
            decay,
            filter,
            search_strategy,
        )
    }

    fn get_throwing_vector_query(
        field: &str,
        vector: Vec<f32>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(ThrowingFloatVectorSimilarityQuery {
            inner: FloatVectorSimilarityQuery::with_decay(
                field,
                vector,
                result_similarity,
                decay,
                filter,
            )?,
        }))
    }
}

/// The anonymous subclass of `FloatVectorSimilarityQuery` that
/// `getThrowingVectorQuery` creates, overriding `createVectorScorer` to throw
/// `UnsupportedOperationException`.
///
/// **Divergence from Lucene 10.5.0.** Java subclasses the concrete query;
/// Rust cannot, so this type wraps one and forwards every member but
/// `create_vector_scorer`. Like Java's anonymous class, it is a class of its
/// own: `sameClassAs` tells it from a plain `FloatVectorSimilarityQuery`.
#[derive(Debug, Clone)]
struct ThrowingFloatVectorSimilarityQuery {
    inner: FloatVectorSimilarityQuery,
}

impl AbstractVectorSimilarityQueryImpl for ThrowingFloatVectorSimilarityQuery {
    fn similarity_base(&self) -> &AbstractVectorSimilarityQuery {
        self.inner.similarity_base()
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn clone_similarity_query(&self) -> Arc<dyn AbstractVectorSimilarityQueryImpl> {
        Arc::new(self.clone())
    }

    fn clone_as_query(&self) -> Arc<dyn Query> {
        Arc::new(self.clone())
    }

    fn create_vector_scorer(
        &self,
        _context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        // `throw new UnsupportedOperationException()` (no message).
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        visit_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs> {
        self.inner
            .approximate_search(context, accept_docs, visit_limit, knn_collector_manager)
    }
}

impl Query for ThrowingFloatVectorSimilarityQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        vector_similarity_query_visit(self.inner.similarity_base(), self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        vector_similarity_create_weight(self.clone_similarity_query(), searcher, score_mode, boost)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        // `FloatVectorSimilarityQuery.equals` starts with `sameClassAs`.
        match other
            .as_any()
            .downcast_ref::<ThrowingFloatVectorSimilarityQuery>()
        {
            Some(other) => self.inner.query_eq(&other.inner),
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        self.inner.query_hash()
    }
}
