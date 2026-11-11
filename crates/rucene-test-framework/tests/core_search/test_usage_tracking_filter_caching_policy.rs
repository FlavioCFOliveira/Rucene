//! Port of `lucene/core/src/test/org/apache/lucene/search/TestUsageTrackingFilterCachingPolicy.java`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, IntPoint};
use rucene::error::Result;
use rucene::index::{IndexReader, LeafReaderContext, Term};
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::doc_id_set_iterator as disi;
use rucene::search::field_exists_query::FieldExistsQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::lru_query_cache::LRUQueryCache;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::term_query::TermQuery;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::{
    is_costly, BooleanQueryBuilder, MatchAllDocsQuery, Occur, Query, QueryCache,
    QueryCachingPolicy, ScoreMode, UsageTrackingQueryCachingPolicy,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestUsageTrackingFilterCachingPolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestUsageTrackingFilterCachingPolicy",
        vec![
            test("testCostlyFilter", test_costly_filter),
            test("testNeverCacheMatchAll", test_never_cache_match_all),
            test("testNeverCacheTermFilter", test_never_cache_term_filter),
            test(
                "testNeverCacheDocValuesFieldExistsFilter",
                test_never_cache_doc_values_field_exists_filter,
            ),
            test("testBooleanQueries", test_boolean_queries),
        ],
    )
}

/// Port of `TestUsageTrackingFilterCachingPolicy.testCostlyFilter`.
fn test_costly_filter() -> Result<()> {
    Assert::assert_true(is_costly(&PrefixQuery::new(Term::from_text(
        "field", "prefix",
    ))?));
    Assert::assert_true(is_costly(&*IntPoint::new_range_query(
        "intField",
        &[1],
        &[1000],
    )?));
    Assert::assert_false(is_costly(&TermQuery::new(Term::from_text(
        "field", "value",
    ))));
    Ok(())
}

/// The body the `testNeverCache*` methods share.
fn never_cache(q: &dyn Query) -> Result<()> {
    let policy = UsageTrackingQueryCachingPolicy::new()?;
    for _i in 0..1000 {
        policy.on_use(q);
    }
    Assert::assert_false(policy.should_cache(q)?);
    Ok(())
}

/// Port of `TestUsageTrackingFilterCachingPolicy.testNeverCacheMatchAll`.
fn test_never_cache_match_all() -> Result<()> {
    let q = MatchAllDocsQuery::instance();
    never_cache(&q)
}

/// Port of `TestUsageTrackingFilterCachingPolicy.testNeverCacheTermFilter`.
fn test_never_cache_term_filter() -> Result<()> {
    let q = TermQuery::new(Term::from_text("foo", "bar"));
    never_cache(&q)
}

/// Port of `TestUsageTrackingFilterCachingPolicy.testNeverCacheDocValuesFieldExistsFilter`.
fn test_never_cache_doc_values_field_exists_filter() -> Result<()> {
    let q = FieldExistsQuery::new("foo");
    never_cache(&q)
}

/// Port of `TestUsageTrackingFilterCachingPolicy.testBooleanQueries`.
fn test_boolean_queries() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    let reader = w.get_reader()?;
    w.close()?;

    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let policy = Arc::new(UsageTrackingQueryCachingPolicy::new()?);
    let cache = Arc::new(LRUQueryCache::with_predicate(
        10,
        i64::MAX,
        Arc::new(|_ctx: &LeafReaderContext| true),
        f32::INFINITY,
    )?);
    searcher.set_query_cache(Some(Arc::clone(&cache) as Arc<dyn QueryCache>));
    searcher.set_query_caching_policy(policy as Arc<dyn QueryCachingPolicy>);

    let q1: Arc<dyn Query> = Arc::new(DummyQuery::new(1));
    let q2: Arc<dyn Query> = Arc::new(DummyQuery::new(2));
    let mut bq = BooleanQueryBuilder::new();
    bq.add(Arc::clone(&q1), Occur::SHOULD)?;
    bq.add(q2, Occur::SHOULD)?;
    let bq: Arc<dyn Query> = Arc::new(bq.build());

    for _i in 0..3 {
        searcher.count(Arc::clone(&bq))?;
    }
    assert_equals(&0, &cache.get_cache_size()); // nothing cached yet, too early

    searcher.count(Arc::clone(&bq))?;
    assert_equals(&1, &cache.get_cache_size()); // the bq got cached, but not q1 and q2

    for _i in 0..10 {
        searcher.count(Arc::clone(&bq))?;
    }
    assert_equals(&1, &cache.get_cache_size()); // q1 and q2 still not cached since we do not pull scorers on them

    searcher.count(q1)?;
    assert_equals(&2, &cache.get_cache_size()); // q1 used on its own -> cached

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// `private static class DummyQuery extends Query`.
#[derive(Debug, Clone)]
struct DummyQuery {
    id: i32,
}

impl DummyQuery {
    fn new(id: i32) -> Self {
        DummyQuery { id }
    }
}

/// The anonymous `ConstantScoreWeight` of `DummyQuery.createWeight`.
#[derive(Debug)]
struct DummyWeight {
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for DummyWeight {
    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let scorer =
            ConstantScoreScorer::from_iterator(score, self.score_mode, Box::new(disi::all(1)?));
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Query for DummyQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "dummy".to_string()
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<DummyQuery>()
                .is_some_and(|o| o.id == self.id)
    }

    fn query_hash(&self) -> u64 {
        self.id as u32 as u64
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            DummyWeight { score_mode },
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
}
