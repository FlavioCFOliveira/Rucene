//! Port of `lucene/core/src/test/org/apache/lucene/search/TestKnnByteVectorQuery.java`
//! and its subclass `TestKnnByteVectorQueryMMap.java`.
//!
//! `ThrowingKnnVectorQuery` and `CappedResultsThrowingKnnVectorQuery`
//! subclass `KnnByteVectorQuery` in Java; Rust cannot subclass, so each is a
//! type of its own that wraps a `KnnByteVectorQuery` and forwards every
//! member but the ones the Java class overrides.
//!
//! Java bytes are signed; Rucene's byte vectors are `u8` holding the same
//! two's-complement bits (`(byte) f` is `f as i8 as u8`).

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::KnnByteVectorField;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::{
    FieldInfo, IndexableField, LeafReaderContext, QueryTimeout, VectorSimilarityFunction,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn::{Hnsw, KnnCollectorManager, KnnSearchStrategy};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::term_query::TermQuery;
use rucene::search::vector_scorer::VectorScorer;
use rucene::search::{
    knn_vector_query_rewrite, knn_vector_query_visit, AbstractKnnVectorQuery,
    AbstractKnnVectorQueryImpl, AcceptDocs, DocIdSetIterator, KnnByteVectorQuery,
    KnnFloatVectorQuery, MatchAllDocsQuery, Query, TopDocs,
};
use rucene::store::mmap::MMapDirectory;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::base_knn_vector_query_test_case::{
    self as base, as_query, assert_doc_score_query_to_string, get_index_store,
    get_knn_vector_query, KnnVectorCase, ISE,
};
use crate::core_search::support::term;
use crate::core_search::test_byte_vector_similarity_query::random_vector_bytes;
use crate::core_search::test_knn_float_vector_query::cap_results;
use crate::core_support::{assert_array_equals, assert_equals, class, random, test};

/// `TestKnnByteVectorQuery extends BaseKnnVectorQueryTestCase`.
pub struct KnnByteCase;

/// `TestKnnByteVectorQueryMMap extends TestKnnByteVectorQuery`, overriding
/// `newDirectoryForTest()`.
pub struct KnnByteMMapCase;

/// The declared test methods of `TestKnnByteVectorQuery`, inherited by
/// `TestKnnByteVectorQueryMMap`.
fn byte_methods<C: KnnVectorCase<Q = KnnByteVectorQuery>>(
) -> Vec<randomizedtesting::jdk::reflect::Method> {
    let mut methods = base::methods::<C>();
    methods.extend([
        test("testToString", test_to_string::<C>),
        test("testGetTarget", test_get_target),
        test(
            "testVectorEncodingMismatch",
            test_vector_encoding_mismatch::<C>,
        ),
    ]);
    methods
}

/// `TestKnnByteVectorQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestKnnByteVectorQuery",
        byte_methods::<KnnByteCase>(),
    )
}

/// `TestKnnByteVectorQueryMMap`.
pub fn mmap_class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestKnnByteVectorQueryMMap",
        byte_methods::<KnnByteMMapCase>(),
    )
}

/// `static byte[] floatToBytes(float[] query)`.
pub fn float_to_bytes(query: &[f32]) -> Vec<u8> {
    let mut bytes = vec![0u8; query.len()];
    for i in 0..query.len() {
        debug_assert!(
            query[i] <= f32::from(i8::MAX)
                && query[i] >= f32::from(i8::MIN)
                && (query[i] % 1.0) == 0.0,
            "float value cannot be converted to byte; provided: {}",
            query[i]
        );
        // `(byte) query[i]`: float to int, then the low byte.
        bytes[i] = (query[i] as i32) as i8 as u8;
    }
    bytes
}

macro_rules! byte_case {
    ($case:ty) => {
        impl KnnVectorCase for $case {
            type Q = KnnByteVectorQuery;

            fn get_knn_vector_query_filtered(
                field: &str,
                query: &[f32],
                k: i32,
                query_filter: Option<Arc<dyn Query>>,
            ) -> Result<KnnByteVectorQuery> {
                KnnByteVectorQuery::with_filter(field, float_to_bytes(query), k, query_filter)
            }

            fn get_throwing_knn_vector_query(
                field: &str,
                vec: &[f32],
                k: i32,
                query: Option<Arc<dyn Query>>,
            ) -> Result<Arc<dyn Query>> {
                Ok(Arc::new(ThrowingKnnVectorQuery::new(
                    field,
                    &float_to_bytes(vec),
                    k,
                    query,
                )?))
            }

            fn get_capped_results_throwing_knn_vector_query(
                field: &str,
                vec: &[f32],
                k: i32,
                query: Option<Arc<dyn Query>>,
                max_results: i32,
            ) -> Result<Arc<dyn Query>> {
                Ok(Arc::new(CappedResultsThrowingKnnVectorQuery::new(
                    field,
                    &float_to_bytes(vec),
                    k,
                    query,
                    max_results,
                )?))
            }

            fn random_vector(dim: i32) -> Vec<f32> {
                let b = random_vector_bytes(dim);
                let mut v = vec![0f32; b.len()];
                let mut vi = 0;
                for i in 0..v.len() {
                    v[vi] = f32::from(b[i] as i8);
                    vi += 1;
                }
                v
            }

            fn get_knn_vector_field_with(
                name: &str,
                vector: &[f32],
                similarity_function: VectorSimilarityFunction,
            ) -> Result<Box<dyn IndexableField>> {
                Ok(Box::new(KnnByteVectorField::new(
                    name,
                    &float_to_bytes(vector),
                    similarity_function,
                )?))
            }

            fn get_knn_vector_field(name: &str, vector: &[f32]) -> Result<Box<dyn IndexableField>> {
                Ok(Box::new(KnnByteVectorField::new(
                    name,
                    &float_to_bytes(vector),
                    VectorSimilarityFunction::EUCLIDEAN,
                )?))
            }

            fn new_directory_for_test() -> Result<TestDirectory> {
                <$case>::directory_for_test()
            }
        }
    };
}

byte_case!(KnnByteCase);
byte_case!(KnnByteMMapCase);

impl KnnByteCase {
    /// The inherited `BaseKnnVectorQueryTestCase.newDirectoryForTest()`.
    fn directory_for_test() -> Result<TestDirectory> {
        Ok(LuceneTestCase::new_directory_with(&*random()))
    }
}

impl KnnByteMMapCase {
    /// `TestKnnByteVectorQueryMMap.newDirectoryForTest()`.
    fn directory_for_test() -> Result<TestDirectory> {
        // An `IOException` is rethrown as `UncheckedIOException`.
        let mmap = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix(
            "TestKnnByteVectorQueryMMap",
        ))
        .map_err(|e| match e {
            LuceneError::Io(_) => LuceneError::Unchecked {
                class: "java.io.UncheckedIOException",
                message: Some(e.to_string()),
                cause: Some(Arc::new(e)),
            },
            other => other,
        })?;
        Ok(TestDirectory::Mock(MockDirectoryWrapper::new(
            &*random(),
            Arc::new(mmap),
        )))
    }
}

fn test_to_string<C: KnnVectorCase<Q = KnnByteVectorQuery>>() -> Result<()> {
    let mut index_store = get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?;
    let reader = directory_reader::open(index_store.as_directory())?;
    let result = (|| -> Result<()> {
        let query = get_knn_vector_query::<C>("field", &[0.0, 1.0], 10)?;
        assert_equals(
            "KnnByteVectorQuery:field[0,...][10]",
            query.to_query_string("ignored").as_str(),
        );

        let rewritten = query
            .rewrite(&LuceneTestCase::new_searcher(Arc::clone(&reader)))?
            .unwrap_or_else(|| as_query(query.clone()));
        assert_doc_score_query_to_string(&*rewritten);

        // test with filter
        let filter: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "text")));
        let query = C::get_knn_vector_query_filtered("field", &[0.0, 1.0], 10, Some(filter))?;
        assert_equals(
            "KnnByteVectorQuery:field[0,...][10][id:text]",
            query.to_query_string("ignored").as_str(),
        );
        Ok(())
    })();
    reader.close()?;
    index_store.close()?;
    result
}

fn test_get_target() -> Result<()> {
    let query_vector_bytes = float_to_bytes(&[0.0, 1.0]);
    let q1 = KnnByteVectorQuery::new("f1", query_vector_bytes.clone(), 10)?;
    assert_array_equals(&query_vector_bytes, &q1.get_target_copy());
    // assertNotSame(queryVectorBytes, q1.getTargetCopy()): the copy is a
    // fresh array.
    let copy = q1.get_target_copy();
    Assert::assert_false(std::ptr::eq(query_vector_bytes.as_ptr(), copy.as_ptr()));
    Ok(())
}

fn test_vector_encoding_mismatch<C: KnnVectorCase<Q = KnnByteVectorQuery>>() -> Result<()> {
    let mut index_store = get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?;
    let reader = directory_reader::open(index_store.as_directory())?;
    let result = (|| -> Result<()> {
        let mut filter: Option<Arc<dyn Query>> = None;
        if random().next_boolean() {
            filter = Some(Arc::new(MatchAllDocsQuery::instance()));
        }
        let query = as_query(KnnFloatVectorQuery::with_filter(
            "field",
            vec![0.0, 1.0],
            10,
            filter,
        )?);
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        expect_throws(ISE, || searcher.search(query, 10));
        Ok(())
    })();
    reader.close()?;
    index_store.close()?;
    result
}

/// `static class ThrowingKnnVectorQuery extends KnnByteVectorQuery`.
#[derive(Debug, Clone)]
pub struct ThrowingKnnVectorQuery {
    inner: KnnByteVectorQuery,
}

impl ThrowingKnnVectorQuery {
    pub fn new(field: &str, target: &[u8], k: i32, filter: Option<Arc<dyn Query>>) -> Result<Self> {
        Ok(ThrowingKnnVectorQuery {
            inner: KnnByteVectorQuery::with_strategy(
                field,
                target.to_vec(),
                k,
                filter,
                Some(KnnSearchStrategy::Hnsw(Hnsw::new(0)?)),
            )?,
        })
    }
}

/// `exactSearch` of `ThrowingKnnVectorQuery`.
fn throwing_exact_search() -> Result<TopDocs> {
    Err(LuceneError::UnsupportedOperation(
        "exact search is not supported".to_string(),
    ))
}

impl AbstractKnnVectorQueryImpl for ThrowingKnnVectorQuery {
    fn knn_base(&self) -> &AbstractKnnVectorQuery {
        self.inner.knn_base()
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn clone_knn_query(&self) -> Arc<dyn AbstractKnnVectorQueryImpl> {
        Arc::new(self.clone())
    }

    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        visited_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs> {
        self.inner
            .approximate_search(context, accept_docs, visited_limit, knn_collector_manager)
    }

    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        self.inner.create_vector_scorer(context, field_info)
    }

    fn exact_search(
        &self,
        _context: &LeafReaderContext,
        _accept_iterator: Box<dyn DocIdSetIterator>,
        _query_timeout: Option<&Arc<dyn QueryTimeout>>,
    ) -> Result<TopDocs> {
        throwing_exact_search()
    }
}

impl Query for ThrowingKnnVectorQuery {
    /// Java returns `null`.
    fn to_query_string(&self, _field: &str) -> String {
        "null".to_string()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        knn_vector_query_visit(self.inner.knn_base(), self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        knn_vector_query_rewrite(self.clone_knn_query(), searcher)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<ThrowingKnnVectorQuery>() {
            Some(other) => self.inner.query_eq(&other.inner),
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        self.inner.query_hash()
    }
}

/// `static class CappedResultsThrowingKnnVectorQuery extends ThrowingKnnVectorQuery`.
#[derive(Debug, Clone)]
pub struct CappedResultsThrowingKnnVectorQuery {
    inner: ThrowingKnnVectorQuery,
    max_results: i32,
}

impl CappedResultsThrowingKnnVectorQuery {
    pub fn new(
        field: &str,
        target: &[u8],
        k: i32,
        filter: Option<Arc<dyn Query>>,
        max_results: i32,
    ) -> Result<Self> {
        Ok(CappedResultsThrowingKnnVectorQuery {
            inner: ThrowingKnnVectorQuery::new(field, target, k, filter)?,
            max_results,
        })
    }
}

impl AbstractKnnVectorQueryImpl for CappedResultsThrowingKnnVectorQuery {
    fn knn_base(&self) -> &AbstractKnnVectorQuery {
        self.inner.knn_base()
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn clone_knn_query(&self) -> Arc<dyn AbstractKnnVectorQueryImpl> {
        Arc::new(self.clone())
    }

    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        _visited_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs> {
        let top_docs =
            self.inner
                .approximate_search(context, accept_docs, i32::MAX, knn_collector_manager)?;
        cap_results(top_docs, self.max_results)
    }

    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        self.inner.create_vector_scorer(context, field_info)
    }

    fn exact_search(
        &self,
        _context: &LeafReaderContext,
        _accept_iterator: Box<dyn DocIdSetIterator>,
        _query_timeout: Option<&Arc<dyn QueryTimeout>>,
    ) -> Result<TopDocs> {
        throwing_exact_search()
    }
}

impl Query for CappedResultsThrowingKnnVectorQuery {
    /// The inherited `ThrowingKnnVectorQuery.toString(String)`: `null`.
    fn to_query_string(&self, _field: &str) -> String {
        "null".to_string()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        knn_vector_query_visit(self.inner.knn_base(), self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        knn_vector_query_rewrite(self.clone_knn_query(), searcher)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other
            .as_any()
            .downcast_ref::<CappedResultsThrowingKnnVectorQuery>()
        {
            Some(other) => self.inner.inner.query_eq(&other.inner.inner),
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        self.inner.query_hash()
    }
}
