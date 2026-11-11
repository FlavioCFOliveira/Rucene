//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPatienceFloatVectorQuery.java`.
//!
//! `TestPatienceFloatVectorQuery extends BaseKnnVectorQueryTestCase`: the
//! inherited test methods are bound from `base_knn_vector_query_test_case`.
//! The instance field `wrapSeeded`, which `setUp` draws and the case's
//! static methods read, is a process-wide flag: the suite runs its tests one
//! at a time, each after its own `setUp`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::index::{directory_reader, IndexableField, VectorSimilarityFunction};
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::patience_knn_vector_query::PatienceKnnVectorQuery;
use rucene::search::seeded_knn_vector_query::SeededKnnVectorQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::base_knn_vector_query_test_case::{
    self as base, as_query, assert_doc_score_query_to_string, get_index_store,
    get_knn_vector_query, KnnVectorCase,
};
use crate::core_search::support::term;
use crate::core_search::test_knn_float_vector_query::{KnnFloatCase, ThrowingKnnVectorQuery};
use crate::core_support::{assert_equals, class, random, test};

/// `wrapSeeded`.
static WRAP_SEEDED: AtomicBool = AtomicBool::new(false);

/// `TestPatienceFloatVectorQuery extends BaseKnnVectorQueryTestCase`.
pub struct PatienceFloatCase;

/// `TestPatienceFloatVectorQuery`.
pub fn class_def() -> Arc<Class> {
    let mut methods = vec![Method::new("setUp")
        .throws(&["java.lang.Exception"])
        .body_any(|| {
            LuceneTestCase::super_set_up();
            WRAP_SEEDED.store(random().next_boolean(), Ordering::SeqCst);
        })];
    methods.extend(base::methods::<PatienceFloatCase>());
    methods.push(test("testToString", test_to_string));
    class(
        "org.apache.lucene.search.TestPatienceFloatVectorQuery",
        methods,
    )
}

impl KnnVectorCase for PatienceFloatCase {
    type Q = PatienceKnnVectorQuery;

    fn get_knn_vector_query_filtered(
        field: &str,
        query: &[f32],
        k: i32,
        query_filter: Option<Arc<dyn Query>>,
    ) -> Result<PatienceKnnVectorQuery> {
        let knn_query = KnnFloatCase::get_knn_vector_query_filtered(field, query, k, query_filter)?;
        if WRAP_SEEDED.load(Ordering::SeqCst) {
            PatienceKnnVectorQuery::from_query(Arc::new(SeededKnnVectorQuery::from_float_query(
                Arc::new(knn_query),
                Arc::new(MatchNoDocsQuery::instance()),
            )?))
        } else {
            PatienceKnnVectorQuery::from_query(Arc::new(knn_query))
        }
    }

    fn get_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(PatienceKnnVectorQuery::from_query(Arc::new(
            ThrowingKnnVectorQuery::patience(field, &vec.to_vec(), k, query)?,
        ))?))
    }

    fn get_capped_results_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
        max_results: i32,
    ) -> Result<Arc<dyn Query>> {
        KnnFloatCase::get_capped_results_throwing_knn_vector_query(
            field,
            vec,
            k,
            query,
            max_results,
        )
    }

    fn random_vector(dim: i32) -> Vec<f32> {
        KnnFloatCase::random_vector(dim)
    }

    fn get_knn_vector_field_with(
        name: &str,
        vector: &[f32],
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        KnnFloatCase::get_knn_vector_field_with(name, vector, similarity_function)
    }

    fn get_knn_vector_field(name: &str, vector: &[f32]) -> Result<Box<dyn IndexableField>> {
        KnnFloatCase::get_knn_vector_field(name, vector)
    }
}

/// The expected `toString` of the Patience query over `inner`.
fn expected_to_string(inner: &str) -> String {
    let wrap_seeded = WRAP_SEEDED.load(Ordering::SeqCst);
    format!(
        "PatienceKnnVectorQuery{{saturationThreshold=0.995, patience=7, delegate={}{inner}{}}}",
        if wrap_seeded {
            "SeededKnnVectorQuery{seed=MatchNoDocsQuery(\"\"), seedWeight=null, delegate="
        } else {
            ""
        },
        if wrap_seeded { "}" } else { "" }
    )
}

/// Port of `TestPatienceFloatVectorQuery.testToString`.
fn test_to_string() -> Result<()> {
    let index_store =
        get_index_store::<PatienceFloatCase>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?;
    let reader = directory_reader::open(index_store.as_directory())?;
    let result = (|| -> Result<()> {
        let query = get_knn_vector_query::<PatienceFloatCase>("field", &[0.0, 1.0], 10)?;
        assert_equals(
            &expected_to_string("KnnFloatVectorQuery:field[0.0,...][10]"),
            &query.to_query_string("ignored"),
        );
        let rewritten = query
            .rewrite(&LuceneTestCase::new_searcher(Arc::clone(&reader)))?
            .unwrap_or_else(|| as_query(query.clone()));
        assert_doc_score_query_to_string(&*rewritten);

        // test with filter
        let filter: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "text")));
        let query = PatienceFloatCase::get_knn_vector_query_filtered(
            "field",
            &[0.0, 1.0],
            10,
            Some(filter),
        )?;
        assert_equals(
            &expected_to_string("KnnFloatVectorQuery:field[0.0,...][10][id:text]"),
            &query.to_query_string("ignored"),
        );
        Ok(())
    })();
    reader.close()?;
    index_store.close()?;
    result
}
