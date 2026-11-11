//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/TestByteVectorSimilarityQuery.java`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::KnnByteVectorField;
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexableField, LeafReaderContext, VectorSimilarityFunction};
use rucene::search::abstract_vector_similarity_query::{
    vector_similarity_create_weight, vector_similarity_query_visit, AbstractVectorSimilarityQuery,
    AbstractVectorSimilarityQueryImpl,
};
use rucene::search::byte_vector_similarity_query::ByteVectorSimilarityQuery;
use rucene::search::doc_id_set_iterator::AcceptDocs;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn::knn_collector_manager::KnnCollectorManager;
use rucene::search::knn::{KnnSearchStrategy, TopDocs};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::vector_scorer::VectorScorer;
use rucene::search::weight::Weight;
use rucene::search::Query;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::base_vector_similarity_query_test_case::{
    self as base, VectorSimilarityCase, VectorSimilarityFixture,
};
use crate::core_support::{random, unchecked};

/// `TestByteVectorSimilarityQuery extends
/// BaseVectorSimilarityQueryTestCase<byte[], KnnByteVectorField,
/// ByteVectorSimilarityQuery>`.
pub struct ByteVectorSimilarityCase;

/// `TestByteVectorSimilarityQuery`.
pub fn class_def() -> Arc<Class> {
    base::class_def::<ByteVectorSimilarityCase>(
        "org.apache.lucene.search.TestByteVectorSimilarityQuery",
    )
}

/// `TestVectorUtil.randomVectorBytes(int dim)` (a public static method of the
/// core test class `org.apache.lucene.util.TestVectorUtil`, which lives in
/// another test target; reproduced here line by line).
pub fn random_vector_bytes(dim: i32) -> Vec<u8> {
    let mut v = TestUtil::random_binary_term_with(&*random(), dim);
    // clip at -127 to avoid overflow
    for i in v.offset..v.offset + v.length {
        if v.bytes[i] as i8 == -128 {
            v.bytes[i] = (-127i8) as u8;
        }
    }
    Assert::assert_true(v.offset == 0);
    v.bytes
}

impl VectorSimilarityCase for ByteVectorSimilarityCase {
    type V = Vec<u8>;
    type Q = ByteVectorSimilarityQuery;

    const SIMPLE_NAME: &'static str = "TestByteVectorSimilarityQuery";

    fn setup(f: &mut VectorSimilarityFixture<Self>) {
        f.vector_field = Some(format!("{}:VectorField", Self::SIMPLE_NAME));
        f.id_field = Some(format!("{}:IdField", Self::SIMPLE_NAME));
        f.function = Some(VectorSimilarityFunction::EUCLIDEAN);
        f.num_docs = LuceneTestCase::at_least(100);
        f.dim = LuceneTestCase::at_least(5);
    }

    fn get_random_vector(dim: i32) -> Vec<u8> {
        random_vector_bytes(dim)
    }

    fn compare(f: &VectorSimilarityFixture<Self>, vector1: &Vec<u8>, vector2: &Vec<u8>) -> f32 {
        unchecked(f.function().compare_bytes(vector1, vector2))
    }

    fn check_equals(vector1: &Vec<u8>, vector2: &Vec<u8>) -> bool {
        vector1 == vector2
    }

    fn get_vector_field(
        name: &str,
        vector: &Vec<u8>,
        function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(KnnByteVectorField::new(name, vector, function)?))
    }

    fn get_vector_query(
        field: &str,
        vector: Vec<u8>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<ByteVectorSimilarityQuery> {
        ByteVectorSimilarityQuery::with_decay(field, vector, result_similarity, decay, filter)
    }

    fn get_vector_query_with_strategy(
        field: &str,
        vector: Vec<u8>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<ByteVectorSimilarityQuery> {
        ByteVectorSimilarityQuery::with_strategy(
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
        vector: Vec<u8>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(ThrowingByteVectorSimilarityQuery {
            inner: ByteVectorSimilarityQuery::with_decay(
                field,
                vector,
                result_similarity,
                decay,
                filter,
            )?,
        }))
    }
}

/// The anonymous subclass of `ByteVectorSimilarityQuery` that
/// `getThrowingVectorQuery` creates, overriding `createVectorScorer` to throw
/// `UnsupportedOperationException`.
///
/// **Divergence from Lucene 10.5.0.** Java subclasses the concrete query;
/// Rust cannot, so this type wraps one and forwards every member but
/// `create_vector_scorer`. Like Java's anonymous class, it is a class of its
/// own: `sameClassAs` tells it from a plain `ByteVectorSimilarityQuery`.
#[derive(Debug, Clone)]
struct ThrowingByteVectorSimilarityQuery {
    inner: ByteVectorSimilarityQuery,
}

impl AbstractVectorSimilarityQueryImpl for ThrowingByteVectorSimilarityQuery {
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

impl Query for ThrowingByteVectorSimilarityQuery {
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
        // `ByteVectorSimilarityQuery.equals` starts with `sameClassAs`.
        match other
            .as_any()
            .downcast_ref::<ThrowingByteVectorSimilarityQuery>()
        {
            Some(other) => self.inner.query_eq(&other.inner),
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        self.inner.query_hash()
    }
}
