//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSeededKnnByteVectorQuery.java`.
//!
//! `TestSeededKnnByteVectorQuery extends BaseKnnVectorQueryTestCase`: the
//! inherited test methods are bound from `base_knn_vector_query_test_case`.
//! `AssertingSeededKnnVectorQuery` subclasses `SeededKnnVectorQuery` in Java;
//! here it wraps one and forwards every member but those Java overrides.

use std::any::Any;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsFormat, DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN,
};
use rucene::document::{Document, IntPoint, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{
    FieldInfo, IndexableField, LeafReaderContext, QueryTimeout, VectorSimilarityFunction,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn::{KnnCollector, KnnCollectorManager, KnnSearchStrategy, SeededOverrides};
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::seeded_knn_vector_query::{SeededCollectorManager, SeededKnnVectorQuery};
use rucene::search::vector_scorer::VectorScorer;
use rucene::search::weight::Weight;
use rucene::search::{
    knn_vector_query_rewrite, AbstractKnnVectorQuery, AbstractKnnVectorQueryImpl, AcceptDocs,
    BooleanQueryBuilder, DocIdSetIterator, FieldExistsQuery, KnnByteVectorQuery, MatchAllDocsQuery,
    Occur, Query, TopDocs,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::base_knn_vector_query_test_case::{
    self as base, ConstantTimeout, KnnVectorCase,
};
use crate::core_search::test_knn_byte_vector_query::{
    float_to_bytes, CappedResultsThrowingKnnVectorQuery, KnnByteCase, ThrowingKnnVectorQuery,
};
use crate::core_support::{assert_equals, class, random, test};

/// `private static final Query MATCH_NONE = MatchNoDocsQuery.INSTANCE`.
pub fn match_none() -> Arc<dyn Query> {
    Arc::new(MatchNoDocsQuery::instance())
}

/// `TestSeededKnnByteVectorQuery extends BaseKnnVectorQueryTestCase`.
pub struct SeededKnnByteCase;

/// `TestSeededKnnByteVectorQuery`.
pub fn class_def() -> Arc<Class> {
    let mut methods = base::methods::<SeededKnnByteCase>();
    methods.extend([
        test("testSeedWithTimeout", test_seed_with_timeout),
        test("testRandomWithSeed", test_random_with_seed),
    ]);
    class(
        "org.apache.lucene.search.TestSeededKnnByteVectorQuery",
        methods,
    )
}

impl KnnVectorCase for SeededKnnByteCase {
    type Q = SeededKnnVectorQuery;

    fn get_knn_vector_query_filtered(
        field: &str,
        query: &[f32],
        k: i32,
        query_filter: Option<Arc<dyn Query>>,
    ) -> Result<SeededKnnVectorQuery> {
        let knn_byte_vector_query =
            KnnByteVectorQuery::with_filter(field, float_to_bytes(query), k, query_filter)?;
        SeededKnnVectorQuery::from_byte_query(Arc::new(knn_byte_vector_query), match_none())
    }

    fn get_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>> {
        let knn_byte_vector_query =
            ThrowingKnnVectorQuery::new(field, &float_to_bytes(vec), k, query)?;
        Ok(Arc::new(SeededKnnVectorQuery::from_byte_query(
            Arc::new(knn_byte_vector_query),
            match_none(),
        )?))
    }

    fn get_capped_results_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
        max_results: i32,
    ) -> Result<Arc<dyn Query>> {
        let knn_byte_vector_query = CappedResultsThrowingKnnVectorQuery::new(
            field,
            &float_to_bytes(vec),
            k,
            query,
            max_results,
        )?;
        Ok(Arc::new(SeededKnnVectorQuery::from_byte_query(
            Arc::new(knn_byte_vector_query),
            match_none(),
        )?))
    }

    fn random_vector(dim: i32) -> Vec<f32> {
        KnnByteCase::random_vector(dim)
    }

    fn get_knn_vector_field_with(
        name: &str,
        vector: &[f32],
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        KnnByteCase::get_knn_vector_field_with(name, vector, similarity_function)
    }

    fn get_knn_vector_field(name: &str, vector: &[f32]) -> Result<Box<dyn IndexableField>> {
        KnnByteCase::get_knn_vector_field(name, vector)
    }
}

/// `searcher.setTimeout(() -> value)`.
pub fn constant_timeout(value: bool) -> Option<Arc<dyn QueryTimeout>> {
    Some(Arc::new(ConstantTimeout(value)))
}

/// The indexing loop of `testSeedWithTimeout`: `numDocs` documents with a
/// vector, a `tag` doc value and a `tag` point.
pub fn index_tagged_docs<C: KnnVectorCase>(
    d: &TestDirectory,
    num_docs: i32,
    dimension: i32,
) -> Result<()> {
    let iwc = IndexWriterConfig::new();
    iwc.set_codec(TestUtil::get_default_codec())?;
    let w = RandomIndexWriter::new_with_config(&*random(), d.as_directory(), iwc)?;
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(C::get_knn_vector_field(
            "field",
            &C::random_vector(dimension),
        )?);
        doc.add(Box::new(NumericDocValuesField::new("tag", i64::from(i))));
        doc.add(Box::new(IntPoint::new("tag", &[i])?));
        w.add_document(&doc)?;
    }
    w.close()
}

/// `random().nextBoolean() ? null : MatchAllDocsQuery.INSTANCE`.
pub fn random_filter() -> Option<Arc<dyn Query>> {
    if random().next_boolean() {
        None
    } else {
        Some(Arc::new(MatchAllDocsQuery::instance()))
    }
}

/// Port of `TestSeededKnnByteVectorQuery.testSeedWithTimeout`.
fn test_seed_with_timeout() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(50);
    let dimension = LuceneTestCase::at_least(5);
    let num_iters = LuceneTestCase::at_least(5);
    let d = SeededKnnByteCase::new_directory_for_test()?;
    let result = (|| -> Result<()> {
        index_tagged_docs::<SeededKnnByteCase>(&d, num_docs, dimension)?;

        let reader = directory_reader::open(d.as_directory())?;
        let r = (|| -> Result<()> {
            let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            searcher.set_timeout(constant_timeout(true));
            let k = random().next_int_bound(80) + 1;
            for _i in 0..num_iters {
                // All documents as seeds
                let seed: Arc<dyn Query> = if random().next_boolean() {
                    IntPoint::new_range_query("tag", &[1], &[6])?
                } else {
                    Arc::new(MatchAllDocsQuery::instance())
                };
                let filter = random_filter();
                let byte_vector_query: Arc<dyn AbstractKnnVectorQueryImpl> =
                    Arc::new(KnnByteVectorQuery::with_filter(
                        "field",
                        float_to_bytes(&SeededKnnByteCase::random_vector(dimension)),
                        k,
                        filter,
                    )?);
                let knn_query: Arc<dyn Query> = Arc::new(SeededKnnVectorQuery::from_byte_query(
                    Arc::clone(&byte_vector_query),
                    seed,
                )?);
                assert_equals(&0, &searcher.count(knn_query)?);
                // No seed documents -- falls back on full approx search
                let seed = match_none();
                let knn_query: Arc<dyn Query> = Arc::new(SeededKnnVectorQuery::from_byte_query(
                    Arc::clone(&byte_vector_query),
                    seed,
                )?);
                assert_equals(&0, &searcher.count(knn_query)?);
            }
            Ok(())
        })();
        reader.close()?;
        r
    })();
    d.close()?;
    result
}

/// The writer config of `testRandomWithSeed`: always the default kNN format,
/// to have predictable behavior around when it hits visitedLimit.
pub fn random_with_seed_config() -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::new();
    iwc.set_codec(TestUtil::always_knn_vectors_format(Arc::new(
        Lucene99HnswVectorsFormat::with_max_conn_beam_width_threshold(
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            0,
        ),
    )))?;
    Ok(iwc)
}

/// Indexes the 1000 documents of `testRandomWithSeed`, some without a
/// vector, and returns how many have one.
pub fn index_random_with_seed<C: KnnVectorCase>(
    d: &TestDirectory,
    num_docs: i32,
    dimension: i32,
) -> Result<i32> {
    let mut num_docs_with_vector = 0;
    // Always use the default kNN format to have predictable behavior around when it hits
    // visitedLimit. This is fine since the test targets AbstractKnnVectorQuery logic, not the kNN
    // format
    // implementation.
    let iwc = random_with_seed_config()?;
    let w = RandomIndexWriter::new_with_config(&*random(), d.as_directory(), iwc)?;
    for i in 0..num_docs {
        let mut doc = Document::new();
        if random().next_boolean() {
            // Randomly skip some vectors to test the mapping from docid to ordinals
            doc.add(C::get_knn_vector_field(
                "field",
                &C::random_vector(dimension),
            )?);
            num_docs_with_vector += 1;
        }
        doc.add(Box::new(NumericDocValuesField::new("tag", i64::from(i))));
        doc.add(Box::new(IntPoint::new("tag", &[i])?));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.close()?;
    Ok(num_docs_with_vector)
}

/// `searcher.setTimeout(random().nextBoolean() ? () -> false : null)`:
/// verify timeout collector wrapping is used.
pub fn random_timeout(searcher: &mut IndexSearcher) {
    if random().next_boolean() {
        searcher.set_timeout(constant_timeout(false));
    } else {
        searcher.set_timeout(None);
    }
}

/// verify the results are in descending score order
pub fn assert_descending(results: &TopDocs) {
    let mut last = f32::MAX;
    for score_doc in &results.score_docs {
        Assert::assert_true(score_doc.score <= last);
        last = score_doc.score;
    }
}

/// `searcher.count(new BooleanQuery.Builder().add(seed2, MUST).add(new
/// FieldExistsQuery("field"), MUST).build())`.
pub fn seed_count(searcher: &IndexSearcher, seed2: &Arc<dyn Query>) -> Result<i32> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(Arc::clone(seed2), Occur::MUST)?;
    bq.add(Arc::new(FieldExistsQuery::new("field")), Occur::MUST)?;
    searcher.count(Arc::new(bq.build()))
}

/// Tests with random vectors and a random seed. Uses RandomIndexWriter.
///
/// Port of `TestSeededKnnByteVectorQuery.testRandomWithSeed`.
fn test_random_with_seed() -> Result<()> {
    let num_docs = 1000;
    let dimension = LuceneTestCase::at_least(5);
    let num_iters = LuceneTestCase::at_least(10);
    let d = SeededKnnByteCase::new_directory_for_test()?;
    let result = (|| -> Result<()> {
        let num_docs_with_vector =
            index_random_with_seed::<SeededKnnByteCase>(&d, num_docs, dimension)?;

        let reader = directory_reader::open(d.as_directory())?;
        let r = (|| -> Result<()> {
            let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            for _i in 0..num_iters {
                random_timeout(&mut searcher);
                let k = random().next_int_bound(10) + 1;
                let n = random().next_int_bound(100) + 1;
                // we may get fewer results than requested if there are deletions, but this test doesn't
                // check that
                debug_assert!(!reader.has_deletions());

                // All documents as seeds
                let seed_calls = Arc::new(AtomicI32::new(0));
                let seed1: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
                let filter = random_filter();
                let byte_vector_query: Arc<dyn AbstractKnnVectorQueryImpl> =
                    Arc::new(KnnByteVectorQuery::with_filter(
                        "field",
                        float_to_bytes(&SeededKnnByteCase::random_vector(dimension)),
                        k,
                        filter,
                    )?);
                let query = AssertingSeededKnnVectorQuery::new(
                    Arc::clone(&byte_vector_query),
                    seed1,
                    None,
                    Some(Arc::clone(&seed_calls)),
                )?;
                let results = searcher.search(Arc::new(query), n)?;
                assert_equals(&seed_calls.load(Ordering::SeqCst), &1);
                let expected = n.min(k).min(num_docs_with_vector);

                assert_equals(&(expected as usize), &results.score_docs.len());
                Assert::assert_true(results.total_hits.value() >= results.score_docs.len() as i64);
                // verify the results are in descending score order
                assert_descending(&results);

                // Restrictive seed query -- 6 documents
                let seed2 = IntPoint::new_range_query("tag", &[1], &[6])?;
                let seed_count = seed_count(&searcher, &seed2)?;
                let byte_vector_query: Arc<dyn AbstractKnnVectorQueryImpl> =
                    Arc::new(KnnByteVectorQuery::with_filter(
                        "field",
                        float_to_bytes(&SeededKnnByteCase::random_vector(dimension)),
                        k,
                        None,
                    )?);
                let query = AssertingSeededKnnVectorQuery::new(
                    Arc::clone(&byte_vector_query),
                    seed2,
                    None,
                    if seed_count > 0 {
                        Some(Arc::clone(&seed_calls))
                    } else {
                        None
                    },
                )?;
                let results = searcher.search(Arc::new(query), n)?;
                assert_equals(
                    &seed_calls.load(Ordering::SeqCst),
                    &(if seed_count > 0 { 2 } else { 1 }),
                );
                let expected = n.min(k).min(reader.num_docs());
                assert_equals(&(expected as usize), &results.score_docs.len());
                Assert::assert_true(results.total_hits.value() >= results.score_docs.len() as i64);
                // verify the results are in descending score order
                assert_descending(&results);

                // No seed documents -- falls back on full approx search
                let seed3 = match_none();
                let query = AssertingSeededKnnVectorQuery::new(
                    Arc::clone(&byte_vector_query),
                    seed3,
                    None,
                    None,
                )?;
                let results = searcher.search(Arc::new(query), n)?;
                let expected = n.min(k).min(reader.num_docs());
                assert_equals(&(expected as usize), &results.score_docs.len());
                Assert::assert_true(results.total_hits.value() >= results.score_docs.len() as i64);
                // verify the results are in descending score order
                assert_descending(&results);
            }
            Ok(())
        })();
        reader.close()?;
        r
    })();
    d.close()?;
    result
}

/// `static class AssertingSeededKnnVectorQuery extends SeededKnnVectorQuery`.
#[derive(Debug, Clone)]
pub struct AssertingSeededKnnVectorQuery {
    inner: SeededKnnVectorQuery,
    seed_calls: Option<Arc<AtomicI32>>,
}

impl AssertingSeededKnnVectorQuery {
    /// `public AssertingSeededKnnVectorQuery(AbstractKnnVectorQuery query, Query seed, Weight seedWeight, AtomicInteger seedCalls)`.
    pub fn new(
        query: Arc<dyn AbstractKnnVectorQueryImpl>,
        seed: Arc<dyn Query>,
        seed_weight: Option<Arc<dyn Weight>>,
        seed_calls: Option<Arc<AtomicI32>>,
    ) -> Result<Self> {
        let base = query.knn_base().clone();
        let inner = SeededKnnVectorQuery::new(
            query,
            seed,
            seed_weight,
            base.get_field(),
            base.get_k(),
            base.get_filter().cloned(),
            base.get_search_strategy().cloned(),
        )?;
        Ok(AssertingSeededKnnVectorQuery { inner, seed_calls })
    }
}

impl AbstractKnnVectorQueryImpl for AssertingSeededKnnVectorQuery {
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
        let manager = AssertingSeededCollectorManager {
            knn_collector_manager: SeededCollectorManager::new(&self.inner, knn_collector_manager),
            seed_calls: self.seed_calls.clone(),
        };
        self.inner
            .delegate()
            .approximate_search(context, accept_docs, visited_limit, &manager)
    }

    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        self.inner.create_vector_scorer(context, field_info)
    }

    fn get_knn_collector_manager(
        &self,
        k: i32,
        searcher: &IndexSearcher,
    ) -> Arc<dyn KnnCollectorManager> {
        self.inner.get_knn_collector_manager(k, searcher)
    }

    fn exact_search(
        &self,
        context: &LeafReaderContext,
        accept_iterator: Box<dyn DocIdSetIterator>,
        query_timeout: Option<&Arc<dyn QueryTimeout>>,
    ) -> Result<TopDocs> {
        self.inner
            .exact_search(context, accept_iterator, query_timeout)
    }

    fn merge_leaf_results(&self, per_leaf_results: &[TopDocs]) -> Result<TopDocs> {
        self.inner.merge_leaf_results(per_leaf_results)
    }
}

impl Query for AssertingSeededKnnVectorQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.inner.seed_weight().is_some() {
            return knn_vector_query_rewrite(self.clone_knn_query(), index_searcher);
        }
        let rewritten = AssertingSeededKnnVectorQuery::new(
            Arc::clone(self.inner.delegate()),
            Arc::clone(self.inner.seed()),
            Some(self.inner.create_seed_weight(index_searcher)?),
            self.seed_calls.clone(),
        )?;
        Ok(Some(match rewritten.rewrite(index_searcher)? {
            Some(q) => q,
            None => Arc::new(rewritten),
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other
            .as_any()
            .downcast_ref::<AssertingSeededKnnVectorQuery>()
        {
            Some(other) => self.inner.query_eq(&other.inner),
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        self.inner.query_hash()
    }
}

/// `class AssertingSeededCollectorManager extends SeededCollectorManager`:
/// its `knnCollectorManager` is the `SeededCollectorManager` it is built
/// over.
#[derive(Debug)]
struct AssertingSeededCollectorManager<'a> {
    knn_collector_manager: SeededCollectorManager<'a>,
    seed_calls: Option<Arc<AtomicI32>>,
}

impl KnnCollectorManager for AssertingSeededCollectorManager<'_> {
    fn new_collector(
        &self,
        num_visited: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        context: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        let knn_collector =
            self.knn_collector_manager
                .new_collector(num_visited, search_strategy, context)?;
        if let Some(KnnSearchStrategy::Seeded(seeded)) = knn_collector.get_search_strategy() {
            if self.seed_calls.is_none() && seeded.number_of_entry_points() > 0 {
                Assert::fail("Expected non-seeded collector but received: KnnCollector");
            }
            return Ok(Box::new(AssertingKnnCollector::new(
                knn_collector,
                self.seed_calls.clone(),
            )));
        }
        if self.seed_calls.is_some() {
            Assert::fail("Expected seeded collector but received: KnnCollector");
        }
        Ok(knn_collector)
    }
}

/// `class AssertingKnnCollector extends KnnCollector.Decorator`.
///
/// Java's `getSearchStrategy()` wraps the collector's `Seeded` strategy in a
/// new `AssertingSeededStrategy` on each call; the strategy here is built once,
/// over the same entry points, as `getSearchStrategy()` returns a reference.
struct AssertingKnnCollector {
    collector: Box<dyn KnnCollector>,
    search_strategy: Option<KnnSearchStrategy>,
}

impl AssertingKnnCollector {
    fn new(collector: Box<dyn KnnCollector>, seed_calls: Option<Arc<AtomicI32>>) -> Self {
        let search_strategy = match collector.get_search_strategy() {
            Some(KnnSearchStrategy::Seeded(seeded)) => Some(KnnSearchStrategy::Seeded(
                seeded.with_overrides(Arc::new(AssertingSeededStrategy { seed_calls })),
            )),
            other => other.cloned(),
        };
        AssertingKnnCollector {
            collector,
            search_strategy,
        }
    }
}

impl KnnCollector for AssertingKnnCollector {
    fn early_terminated(&self) -> bool {
        self.collector.early_terminated()
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.collector.inc_visited_count(count);
    }

    fn visited_count(&self) -> i64 {
        self.collector.visited_count()
    }

    fn visit_limit(&self) -> i64 {
        self.collector.visit_limit()
    }

    fn k(&self) -> i32 {
        self.collector.k()
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        self.collector.collect(doc_id, similarity)
    }

    fn min_competitive_similarity(&self) -> f32 {
        self.collector.min_competitive_similarity()
    }

    fn top_docs(&mut self) -> TopDocs {
        self.collector.top_docs()
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.search_strategy.as_ref()
    }
}

/// `class AssertingSeededStrategy extends KnnSearchStrategy.Seeded`, which
/// counts the calls of `entryPoints()`.
#[derive(Debug)]
struct AssertingSeededStrategy {
    seed_calls: Option<Arc<AtomicI32>>,
}

impl SeededOverrides for AssertingSeededStrategy {
    fn entry_points(&self, entry_points: &dyn DocIdSetIterator) {
        debug_assert!(entry_points.cost() > 0);
        // `seedCalls.incrementAndGet()`: a `null` counter is a
        // `NullPointerException`.
        match &self.seed_calls {
            Some(seed_calls) => {
                seed_calls.fetch_add(1, Ordering::SeqCst);
            }
            None => randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            ),
        }
    }
}
