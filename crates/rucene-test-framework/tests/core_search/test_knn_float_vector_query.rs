//! Port of `lucene/core/src/test/org/apache/lucene/search/TestKnnFloatVectorQuery.java`.
//!
//! `ThrowingKnnVectorQuery` and `CappedResultsThrowingKnnVectorQuery`
//! subclass `KnnFloatVectorQuery` in Java; Rust cannot subclass, so each is a
//! type of its own that wraps a `KnnFloatVectorQuery` and forwards every
//! member but the ones the Java class overrides. As Java's subclasses, each is
//! a class of its own for `sameClassAs`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::{Document, KnnFloatVectorField, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::FieldInfo;
use rucene::index::{
    IndexReader, IndexableField, LeafReaderContext, QueryTimeout, VectorSimilarityFunction,
};
use rucene::search::doc_and_score_query::{find_segment_starts, DocAndScoreQuery};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn::{Hnsw, KnnCollectorManager, KnnSearchStrategy};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::term_query::TermQuery;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::vector_scorer::VectorScorer;
use rucene::search::{
    knn_vector_query_rewrite, knn_vector_query_visit, AbstractKnnVectorQuery,
    AbstractKnnVectorQueryImpl, AcceptDocs, DocIdSetIterator, KnnByteVectorQuery,
    KnnFloatVectorQuery, MatchAllDocsQuery, Query, ScoreDoc, ScoreMode, TopDocs,
};
use rucene::util::vector_util;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::base_knn_vector_query_test_case::{
    self as base, as_query, assert_doc_score_query_to_string, get_index_store,
    get_knn_vector_query, KnnVectorCase, AIOOBE, ISE,
};
use crate::core_search::support::term;
use crate::core_search::test_float_vector_similarity_query::random_vector;
use crate::core_support::{
    assert_array_equals_f32, assert_equals, assert_equals_delta, class, random, test,
};

/// `TestKnnFloatVectorQuery extends BaseKnnVectorQueryTestCase`.
pub struct KnnFloatCase;

/// `TestKnnFloatVectorQuery`.
pub fn class_def() -> Arc<Class> {
    let mut methods = base::methods::<KnnFloatCase>();
    methods.extend([
        test("testToString", test_to_string),
        test("testVectorEncodingMismatch", test_vector_encoding_mismatch),
        test("testGetTarget", test_get_target),
        test(
            "testScoreNegativeDotProduct",
            test_score_negative_dot_product,
        ),
        test("testScoreDotProduct", test_score_dot_product),
        test(
            "testDocAndScoreQueryBasics",
            test_doc_and_score_query_basics,
        ),
    ]);
    class("org.apache.lucene.search.TestKnnFloatVectorQuery", methods)
}

impl KnnVectorCase for KnnFloatCase {
    type Q = KnnFloatVectorQuery;

    fn get_knn_vector_query_filtered(
        field: &str,
        query: &[f32],
        k: i32,
        query_filter: Option<Arc<dyn Query>>,
    ) -> Result<KnnFloatVectorQuery> {
        KnnFloatVectorQuery::with_filter(field, query.to_vec(), k, query_filter)
    }

    fn get_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(ThrowingKnnVectorQuery::new(field, vec, k, query)?))
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
            vec,
            k,
            query,
            max_results,
        )?))
    }

    fn random_vector(dim: i32) -> Vec<f32> {
        random_vector(dim)
    }

    fn get_knn_vector_field_with(
        name: &str,
        vector: &[f32],
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(KnnFloatVectorField::new(
            name,
            vector,
            similarity_function,
        )?))
    }

    fn get_knn_vector_field(name: &str, vector: &[f32]) -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(KnnFloatVectorField::with_euclidean(name, vector)?))
    }
}

fn test_to_string() -> Result<()> {
    let mut index_store =
        get_index_store::<KnnFloatCase>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?;
    let reader = directory_reader::open(index_store.as_directory())?;
    let result = (|| -> Result<()> {
        let query = get_knn_vector_query::<KnnFloatCase>("field", &[0.0, 1.0], 10)?;
        assert_equals(
            "KnnFloatVectorQuery:field[0.0,...][10]",
            query.to_query_string("ignored").as_str(),
        );

        let rewritten = query
            .rewrite(&LuceneTestCase::new_searcher(Arc::clone(&reader)))?
            .unwrap_or_else(|| as_query(query.clone()));
        assert_doc_score_query_to_string(&*rewritten);

        // test with filter
        let filter: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "text")));
        let query =
            KnnFloatCase::get_knn_vector_query_filtered("field", &[0.0, 1.0], 10, Some(filter))?;
        assert_equals(
            "KnnFloatVectorQuery:field[0.0,...][10][id:text]",
            query.to_query_string("ignored").as_str(),
        );
        Ok(())
    })();
    reader.close()?;
    index_store.close()?;
    result
}

fn test_vector_encoding_mismatch() -> Result<()> {
    let mut index_store =
        get_index_store::<KnnFloatCase>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?;
    let reader = directory_reader::open(index_store.as_directory())?;
    let result = (|| -> Result<()> {
        let mut filter: Option<Arc<dyn Query>> = None;
        if random().next_boolean() {
            filter = Some(Arc::new(MatchAllDocsQuery::instance()));
        }
        let query = as_query(KnnByteVectorQuery::with_filter(
            "field",
            vec![0, 1],
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

fn test_get_target() -> Result<()> {
    let query_vector = vec![0.0f32, 1.0];
    let q1 = KnnFloatVectorQuery::new("f1", query_vector.clone(), 10)?;

    assert_array_equals_f32(&query_vector, &q1.get_target_copy(), 0.0);
    // assertNotEquals(queryVector, q1.getTargetCopy()): `float[]` equality is
    // identity, and the copy is a fresh array.
    let copy = q1.get_target_copy();
    Assert::assert_false(std::ptr::eq(query_vector.as_ptr(), copy.as_ptr()));
    Ok(())
}

fn test_score_negative_dot_product() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let w = IndexWriter::new(d.as_directory(), IndexWriterConfig::new())?;
    let mut doc = Document::new();
    doc.add(KnnFloatCase::get_knn_vector_field_with(
        "field",
        &[-1.0, 0.0],
        VectorSimilarityFunction::DOT_PRODUCT,
    )?);
    w.add_document(&doc)?;
    doc = Document::new();
    doc.add(KnnFloatCase::get_knn_vector_field_with(
        "field",
        &[1.0, 0.0],
        VectorSimilarityFunction::DOT_PRODUCT,
    )?);
    w.add_document(&doc)?;
    w.close()?;
    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
        assert_equals(&1, &leaves.len());
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let query = get_knn_vector_query::<KnnFloatCase>("field", &[1.0, 0.0], 2)?;
        let rewritten = query
            .rewrite(&searcher)?
            .unwrap_or_else(|| as_query(query.clone()));
        let weight = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
        let mut scorer = weight.scorer(&leaves[0])?.expect("scorer");

        // scores are normalized to lie in [0, 1]
        assert_equals(&2, &scorer.iterator().cost());
        assert_equals(&0, &scorer.iterator().next_doc()?);
        Assert::assert_true(0.0 <= scorer.score()?);
        assert_equals(&1, &scorer.iterator().advance(1)?);
        assert_equals_delta(1.0, f64::from(scorer.score()?), 0.0);
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

/// `VectorUtil.l2normalize(v)`, which returns `v`.
fn l2normalize(mut v: Vec<f32>) -> Result<Vec<f32>> {
    vector_util::l2normalize(&mut v, true)?;
    Ok(v)
}

fn test_score_dot_product() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let w = IndexWriter::new(d.as_directory(), IndexWriterConfig::new())?;
    let written = (|| -> Result<()> {
        for j in 1..=5 {
            let mut doc = Document::new();
            doc.add(KnnFloatCase::get_knn_vector_field_with(
                "field",
                &l2normalize(vec![j as f32, (j * j) as f32])?,
                VectorSimilarityFunction::DOT_PRODUCT,
            )?);
            w.add_document(&doc)?;
        }
        Ok(())
    })();
    w.close()?;
    written?;
    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
        assert_equals(&1, &leaves.len());
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let query =
            get_knn_vector_query::<KnnFloatCase>("field", &l2normalize(vec![2.0, 3.0])?, 3)?;
        let rewritten = query
            .rewrite(&searcher)?
            .unwrap_or_else(|| as_query(query.clone()));
        let weight = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
        let mut scorer = weight.scorer(&leaves[0])?.expect("scorer");

        // prior to advancing, score is undefined
        assert_equals(&-1, &scorer.doc_id());
        expect_throws(AIOOBE, || scorer.score());

        /* score0 = ((2,3) * (1, 1) = 5) / (||2, 3|| * ||1, 1|| = sqrt(26)), then
         * normalized by (1 + x) /2.
         */
        let score0 = ((1.0
            + f64::from(2 * 1 + 3 * 1) / f64::from((2 * 2 + 3 * 3) * (1 * 1 + 1 * 1)).sqrt())
            / 2.0) as f32;

        /* score1 = ((2,3) * (2, 4) = 16) / (||2, 3|| * ||2, 4|| = sqrt(260)), then
         * normalized by (1 + x) /2
         */
        let score1 = ((1.0
            + f64::from(2 * 2 + 3 * 4) / f64::from((2 * 2 + 3 * 3) * (2 * 2 + 4 * 4)).sqrt())
            / 2.0) as f32;

        // doc 1 happens to have the max score
        assert_equals_delta(
            f64::from(score1),
            f64::from(scorer.get_max_score(2)?),
            0.0001,
        );
        assert_equals_delta(
            f64::from(score1),
            f64::from(scorer.get_max_score(i32::MAX)?),
            0.0001,
        );

        assert_equals(&3, &scorer.iterator().cost());
        assert_equals(&0, &scorer.iterator().next_doc()?);
        // doc 0 has (1, 1)
        assert_equals_delta(f64::from(score0), f64::from(scorer.score()?), 0.0001);
        assert_equals(&1, &scorer.iterator().advance(1)?);
        assert_equals_delta(f64::from(score1), f64::from(scorer.score()?), 0.0001);

        // since topK was 3
        assert_equals(&NO_MORE_DOCS, &scorer.iterator().advance(4)?);
        expect_throws(AIOOBE, || scorer.score());
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

fn test_doc_and_score_query_basics() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let reader;
    {
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
        for i in 0..50 {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "field",
                format!("value{i}"),
                Store::NO,
            )?));
            iw.add_document(&doc)?;
            if i % 10 == 0 {
                iw.flush()?;
            }
        }
        reader = iw.get_reader()?;
        iw.close()?;
    }
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let mut score_docs_list = Vec::new();
        let mut doc = 0;
        while doc < 30 {
            score_docs_list.push(ScoreDoc::new(doc, RandomizedTest::random_float()));
            doc += 1 + random().next_int_bound(5);
        }
        let score_docs = score_docs_list;
        let mut docs = vec![0i32; score_docs.len()];
        let mut scores = vec![0f32; score_docs.len()];
        let mut max_score = f32::from_bits(1); // Float.MIN_VALUE
        for i in 0..score_docs.len() {
            docs[i] = score_docs[i].doc;
            scores[i] = score_docs[i].score;
            max_score = max_score.max(scores[i]);
        }
        let segments = find_segment_starts(searcher.get_leaf_contexts(), &docs);

        // `indexReader.getContext().id()`: the searcher's top-level context is
        // the reader's context.
        let query = DocAndScoreQuery::new(
            docs,
            scores,
            max_score,
            segments,
            score_docs.len() as i64,
            searcher.get_top_reader_context().id(),
        );
        let visited = query.visited();
        let query = as_query(query);

        let w = query.create_weight(&searcher, ScoreMode::TOP_SCORES, 1.0)?;
        let mut top_docs = searcher.search(Arc::clone(&query), 100)?;
        assert_equals(&(score_docs.len() as i64), &top_docs.total_hits.value());
        assert_equals(&visited, &top_docs.total_hits.value());
        assert_equals(
            &TotalHitsRelation::EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        top_docs.score_docs.sort_by_key(|score_doc| score_doc.doc);
        assert_equals(&score_docs.len(), &top_docs.score_docs.len());
        for i in 0..score_docs.len() {
            assert_equals(&score_docs[i].doc, &top_docs.score_docs[i].doc);
            assert_equals_delta(
                f64::from(score_docs[i].score),
                f64::from(top_docs.score_docs[i].score),
                f64::from(0.0001f32),
            );
            Assert::assert_true(
                searcher
                    .explain(Arc::clone(&query), score_docs[i].doc)?
                    .is_match(),
            );
        }

        for leaf_reader_context in searcher.get_leaf_contexts() {
            let scorer = w.scorer(leaf_reader_context)?;
            let count = w.count(leaf_reader_context)?;
            match scorer {
                None => assert_equals(&0, &count),
                Some(mut scorer) => {
                    Assert::assert_true_msg(
                        &leaf_reader_context.to_string(),
                        scorer.get_max_score(NO_MORE_DOCS)? > 0.0,
                    );
                    Assert::assert_true(count > 0);
                    let mut iterator_count = 0;
                    while scorer.iterator().next_doc()? != NO_MORE_DOCS {
                        iterator_count += 1;
                    }
                    assert_equals(&iterator_count, &count);
                }
            }
        }
        Ok(())
    })();
    reader.close()?;
    directory.close()?;
    result
}

/// `static class ThrowingKnnVectorQuery extends KnnFloatVectorQuery`.
#[derive(Debug, Clone)]
pub struct ThrowingKnnVectorQuery {
    inner: KnnFloatVectorQuery,
    /// Whether this is `TestPatienceFloatVectorQuery.ThrowingKnnVectorQuery`, a
    /// class of its own whose `toString(field)` is
    /// `"ThrowingKnnVectorQuery{" + field + "}"`, rather than this class.
    patience: bool,
}

impl ThrowingKnnVectorQuery {
    /// `new TestPatienceFloatVectorQuery.ThrowingKnnVectorQuery(field, target,
    /// k, filter)`: the same query, as the Patience test's own class.
    pub fn patience(
        field: &str,
        target: &[f32],
        k: i32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Self> {
        Ok(ThrowingKnnVectorQuery {
            patience: true,
            ..Self::new(field, target, k, filter)?
        })
    }

    pub fn new(
        field: &str,
        target: &[f32],
        k: i32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Self> {
        Ok(ThrowingKnnVectorQuery {
            patience: false,
            inner: KnnFloatVectorQuery::with_strategy(
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
    Err(LuceneError::unsupported_operation(
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
    /// Java returns `null`; the Patience test's class renders
    /// `"ThrowingKnnVectorQuery{" + field + "}"`.
    fn to_query_string(&self, field: &str) -> String {
        if self.patience {
            format!("ThrowingKnnVectorQuery{{{field}}}")
        } else {
            "null".to_string()
        }
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
            Some(other) => self.patience == other.patience && self.inner.query_eq(&other.inner),
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
        target: &[f32],
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

/// `CappedResultsThrowingKnnVectorQuery.approximateSearch` over the result of
/// `super.approximateSearch(context, acceptDocs, Integer.MAX_VALUE, knnCollectorManager)`.
pub fn cap_results(top_docs: TopDocs, max_results: i32) -> Result<TopDocs> {
    let results = top_docs.total_hits.value().min(i64::from(max_results));
    let mut score_docs = top_docs.score_docs;
    score_docs.truncate(results as usize);

    Ok(TopDocs::new(
        TotalHits::new(results, TotalHitsRelation::EQUAL_TO)?,
        score_docs,
    ))
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
