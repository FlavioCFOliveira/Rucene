//! Port of `lucene/core/src/test/org/apache/lucene/search/TestLRUQueryCache.java`.
//!
//! A Java subclass of `LRUQueryCache` (`new LRUQueryCache(...) { ... }`) is a
//! [`LRUQueryCacheOverrides`] installed with [`LRUQueryCache::with_overrides`];
//! the reader core key Java passes to its callbacks is the leaf's core
//! `CacheKey`.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use randomizedtesting::generators::{RandomNumbers, RandomPicks};
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, Field, LongPoint, NumericDocValuesField, NumericValue, SortedNumericDocValuesField,
    Store, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::DocValues;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::{CacheHelper, CacheKey};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexReader, LeafReader, LeafReaderContext, Term};
use rucene::search::bulk_scorer::BulkScorer;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::doc_id_set_iterator::{self, DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::index_or_doc_values_query::IndexOrDocValuesQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query_cache::{QueryCache, QueryCachingPolicy};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::{Scorable, SimpleScorable};
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::searcher_factory::{DefaultSearcherFactory, SearcherFactory};
use rucene::search::searcher_manager::SearcherManager;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::term_query::TermQuery;
use rucene::search::weight::{FilterWeight, Weight};
use rucene::search::{
    min_segment_size_predicate, BooleanQuery, BooleanQueryBuilder, BoostQuery, CacheAndCount,
    CollectionResult, Collector, CollectorManager, LRUQueryCache, LRUQueryCacheOverrides,
    LeafCollector, Occur, Query, QueryKey, ScoreMode, HASHTABLE_RAM_BYTES_PER_ENTRY,
    LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY,
};
use rucene::util::{Accountable, Bits, BytesRef, RamUsageEstimator};
use rucene_test_framework::hamcrest::less_than_or_equal_to;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::asserting_index_searcher::AssertingIndexSearcher;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::dummy_total_hit_count_collector::{
    DummyTotalHitCountCollector, DummyTotalHitCountCollectorManager,
};
use rucene_test_framework::util::java_exceptions::{java_throwable, rethrow_java};
use rucene_test_framework::util::lucene_test_case::{AwaitsFix, MAYBE_CACHE_POLICY, TEST_NIGHTLY};
use rucene_test_framework::util::ram_usage_tester::{
    Accumulator, DefaultAccumulator, JavaObject, ModelBuilder, Queue, RamObject, RamUsageTester,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{field_mut_as, term, CountDownLatch};
use crate::core_support::hamcrest::assert_has_size;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_equals, assert_null, class, random, test,
    try_catch, unchecked,
};

/// `TestLRUQueryCache`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestLRUQueryCache",
        vec![
            test("testConcurrency", test_concurrency),
            // @SuppressForbidden(reason = "Thread sleep")
            test("testLRUEviction", test_lru_eviction),
            test("testClearFilter", test_clear_filter),
            test(
                "testRamBytesUsedAgreesWithRamUsageTester",
                test_ram_bytes_used_agrees_with_ram_usage_tester,
            )
            .annotate(AwaitsFix {
                bug_url: "https://issues.apache.org/jira/browse/LUCENE-7595".into(),
            }),
            test(
                "testRamBytesUsedConstantEntryOverhead",
                test_ram_bytes_used_constant_entry_overhead,
            )
            .annotate(AwaitsFix {
                bug_url: "https://issues.apache.org/jira/browse/LUCENE-7595".into(),
            }),
            test(
                "testCachingAccountableQuery",
                test_caching_accountable_query,
            ),
            test(
                "testConsistencyWithAccountableQueries",
                test_consistency_with_accountable_queries,
            ),
            test("testOnUse", test_on_use),
            test("testStats", test_stats),
            test("testFineGrainedStats", test_fine_grained_stats),
            test(
                "testUseRewrittenQueryAsCacheKey",
                test_use_rewritten_query_as_cache_key,
            ),
            test(
                "testBooleanQueryCachesSubClauses",
                test_boolean_query_caches_sub_clauses,
            ),
            test(
                "testCachePopulationUsesLeafCollectorCollectRange",
                test_cache_population_uses_leaf_collector_collect_range,
            ),
            test("testRandom", test_random),
            test("testDetectMutatedQueries", test_detect_mutated_queries).annotate(AwaitsFix {
                bug_url: "https://issues.apache.org/jira/browse/LUCENE-7604".into(),
            }),
            test(
                "testRefuseToCacheTooLargeEntries",
                test_refuse_to_cache_too_large_entries,
            ),
            test(
                "testOnUseWithRandomFirstSegmentSkipping",
                test_on_use_with_random_first_segment_skipping,
            ),
            test("testPropagateBulkScorer", test_propagate_bulk_scorer),
            test("testEvictEmptySegmentCache", test_evict_empty_segment_cache),
            test(
                "testMinSegmentSizePredicate",
                test_min_segment_size_predicate,
            ),
            test(
                "testReaderNotSuitedForCaching",
                test_reader_not_suited_for_caching,
            ),
            test("testSkipCacheIfOngoing", test_skip_cache_if_ongoing),
            test(
                "testQueryNotSuitedForCaching",
                test_query_not_suited_for_caching,
            ),
            test(
                "testPropagatesScorerSupplier",
                test_propagates_scorer_supplier,
            ),
            test(
                "testDocValuesUpdatesDontBreakCache",
                test_doc_values_updates_dont_break_cache,
            ),
            test("testQueryCacheSoftUpdate", test_query_cache_soft_update),
            test("testBulkScorerLocking", test_bulk_scorer_locking),
            test(
                "testSkipCachingForRangeQuery",
                test_skip_caching_for_range_query,
            ),
            test("testCountDelegation", test_count_delegation),
            test(
                "testSkipCachingForTermQuery",
                test_skip_caching_for_term_query,
            ),
            test("testCacheHasFastCount", test_cache_has_fast_count),
            test(
                "testSkipCacheFactorWithDynamicValues",
                test_skip_cache_factor_with_dynamic_values,
            ),
        ],
    )
}

// -----------------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------------

/// A `QueryCachingPolicy` whose `shouldCache` answers a constant.
#[derive(Debug)]
struct ConstantPolicy(bool);

impl QueryCachingPolicy for ConstantPolicy {
    fn on_use(&self, _query: &dyn Query) {}

    fn should_cache(&self, _query: &dyn Query) -> Result<bool> {
        Ok(self.0)
    }
}

/// `ALWAYS_CACHE`.
static ALWAYS_CACHE: LazyLock<Arc<dyn QueryCachingPolicy>> =
    LazyLock::new(|| Arc::new(ConstantPolicy(true)));

/// `NEVER_CACHE`.
static NEVER_CACHE: LazyLock<Arc<dyn QueryCachingPolicy>> =
    LazyLock::new(|| Arc::new(ConstantPolicy(false)));

fn always_cache() -> Arc<dyn QueryCachingPolicy> {
    Arc::clone(&ALWAYS_CACHE)
}

fn never_cache() -> Arc<dyn QueryCachingPolicy> {
    Arc::clone(&NEVER_CACHE)
}

/// `new LRUQueryCache(maxSize, maxRamBytesUsed, predicate, skipCacheFactor)`.
fn lru(
    max_size: i32,
    max_ram_bytes_used: i64,
    predicate: impl Fn(&LeafReaderContext) -> bool + Send + Sync + 'static,
    skip_cache_factor: f32,
) -> Result<Arc<LRUQueryCache>> {
    Ok(Arc::new(LRUQueryCache::with_predicate(
        max_size,
        max_ram_bytes_used,
        Arc::new(predicate),
        skip_cache_factor,
    )?))
}

/// The same, for a subclass: `overrides` carries its protected methods.
fn lru_with(
    max_size: i32,
    max_ram_bytes_used: i64,
    predicate: impl Fn(&LeafReaderContext) -> bool + Send + Sync + 'static,
    skip_cache_factor: f32,
    overrides: Arc<dyn LRUQueryCacheOverrides>,
) -> Result<Arc<LRUQueryCache>> {
    Ok(Arc::new(LRUQueryCache::with_overrides(
        max_size,
        max_ram_bytes_used,
        Arc::new(predicate),
        skip_cache_factor,
        Some(overrides),
    )?))
}

/// The cache as the `QueryCache` a searcher holds.
fn as_cache(cache: &Arc<LRUQueryCache>) -> Option<Arc<dyn QueryCache>> {
    Some(Arc::clone(cache) as Arc<dyn QueryCache>)
}

/// `queryCache.cachedQueries()` as the `List<Query>` Java compares with
/// `equals`.
fn cached(cache: &LRUQueryCache) -> Vec<QueryKey> {
    cache
        .cached_queries()
        .into_iter()
        .map(QueryKey::new)
        .collect()
}

/// `new HashSet<>(queryCache.cachedQueries())`.
fn cached_set(cache: &LRUQueryCache) -> HashSet<QueryKey> {
    cache
        .cached_queries()
        .into_iter()
        .map(QueryKey::new)
        .collect()
}

/// `Arrays.asList(queries)` as the list Java compares with `equals`.
fn keys(queries: &[&Arc<dyn Query>]) -> Vec<QueryKey> {
    queries
        .iter()
        .map(|q| QueryKey::new(Arc::clone(q)))
        .collect()
}

/// `new HashSet<>(Arrays.asList(queries))`.
fn key_set(queries: &[&Arc<dyn Query>]) -> HashSet<QueryKey> {
    queries
        .iter()
        .map(|q| QueryKey::new(Arc::clone(q)))
        .collect()
}

/// `new StringField(name, value, Store.NO)`.
fn string_field(name: &str, value: &str) -> Result<StringField> {
    StringField::new(name, value.to_string(), Store::NO)
}

/// `new TextField(name, value, Store.NO)`.
fn text_field(name: &str, value: &str) -> Result<TextField> {
    TextField::new(name, value.to_string(), Store::NO)
}

/// `new NumericDocValuesField(name, value)` as a `Field`.
fn ndv_field(name: &str, value: i64) -> Result<Field> {
    Field::new_with_number(
        name,
        NumericValue::Long(value),
        NumericDocValuesField::type_().clone(),
    )
}

/// A document with one `StringField` per `(name, value)`, `Store.YES`.
fn stored_doc(fields: &[(&str, &str)]) -> Result<Document> {
    let mut doc = Document::new();
    for (name, value) in fields {
        doc.add(Box::new(StringField::new(
            name,
            value.to_string(),
            Store::YES,
        )?));
    }
    Ok(doc)
}

/// `reader.leaves()`.
fn leaves(reader: &Arc<dyn DirectoryReader>) -> Result<Vec<Arc<LeafReaderContext>>> {
    (Arc::clone(reader) as Arc<dyn IndexReader>).leaves()
}

/// `ctx.reader().getCoreCacheHelper().getKey()`.
fn core_key(ctx: &LeafReaderContext) -> CacheKey {
    let helper = ctx
        .leaf_reader()
        .get_core_cache_helper()
        .expect("a segment reader has a core cache helper");
    helper.get_key().clone()
}

/// A copy of a query the caching policies of these tests receive (term and
/// boolean queries), to use as a map key: Java keeps the reference itself.
fn owned_query(query: &dyn Query) -> Arc<dyn Query> {
    if let Some(q) = query.as_any().downcast_ref::<TermQuery>() {
        return Arc::new(q.clone());
    }
    if let Some(q) = query.as_any().downcast_ref::<BooleanQuery>() {
        return Arc::new(q.clone());
    }
    panic!(
        "no copy of {} for a map key",
        rucene::search::query_to_string(query)
    )
}

// -----------------------------------------------------------------------------
// testConcurrency
// -----------------------------------------------------------------------------

/// The anonymous `SearcherFactory` of `testConcurrency`.
#[derive(Debug)]
struct CachingSearcherFactory {
    query_cache: Arc<LRUQueryCache>,
}

impl SearcherFactory for CachingSearcherFactory {
    fn new_searcher(
        &self,
        reader: Arc<dyn IndexReader>,
        _previous: Option<Arc<dyn IndexReader>>,
    ) -> Result<IndexSearcher> {
        let mut searcher = IndexSearcher::new(reader)?;
        searcher.set_query_caching_policy(Arc::clone(&MAYBE_CACHE_POLICY));
        searcher.set_query_cache(as_cache(&self.query_cache));
        Ok(searcher)
    }
}

/// The anonymous `FilterCollector` of `testConcurrency`, whose score mode
/// keeps the cache from being used.
#[derive(Debug)]
struct ScoringFilterCollector {
    r#in: DummyTotalHitCountCollector,
}

impl Collector for ScoringFilterCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.r#in.get_leaf_collector(context)
    }

    fn score_mode(&self) -> ScoreMode {
        // will not use the cache because of scores
        ScoreMode::COMPLETE
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.r#in.set_weight(weight);
    }
}

/// The anonymous `CollectorManager<FilterCollector, Integer>` of
/// `testConcurrency`.
struct ScoringFilterCollectorManager {
    collector_manager: DummyTotalHitCountCollectorManager,
}

impl CollectorManager for ScoringFilterCollectorManager {
    type Collector = ScoringFilterCollector;
    type Output = i32;

    fn new_collector(&self) -> Result<ScoringFilterCollector> {
        Ok(ScoringFilterCollector {
            r#in: self.collector_manager.new_collector()?,
        })
    }

    fn reduce(&self, collectors: Vec<ScoringFilterCollector>) -> Result<i32> {
        self.collector_manager
            .reduce(collectors.into_iter().map(|c| c.r#in).collect())
    }
}

/// `error.compareAndSet(null, t)`.
fn set_first_error(error: &Mutex<Option<Throwable>>, t: Throwable) {
    let mut error = error.lock().unwrap_or_else(|e| e.into_inner());
    if error.is_none() {
        *error = Some(t);
    }
}

/// Port of `TestLRUQueryCache.testConcurrency`.
fn test_concurrency() -> Result<()> {
    let max_size = 1 + random().next_int_bound(20);
    let max_ram = 1 + random().next_int_bound(10000) as i64;
    let query_cache = lru(
        max_size,
        max_ram,
        |_context| random().next_boolean(),
        f32::INFINITY,
    )?;
    let dir = LuceneTestCase::new_directory();
    let w = Arc::new(RandomIndexWriter::new_with_mock_analyzer(
        &*random(),
        dir.as_directory(),
    )?);
    let searcher_factory: Arc<dyn SearcherFactory> = Arc::new(CachingSearcherFactory {
        query_cache: Arc::clone(&query_cache),
    });
    let apply_deletes = random().next_boolean();
    let mgr = Arc::new(SearcherManager::from_writer(
        &**w.w(),
        apply_deletes,
        false,
        Some(searcher_factory),
        None,
    )?);
    let indexing = Arc::new(AtomicBool::new(true));
    let error: Arc<Mutex<Option<Throwable>>> = Arc::new(Mutex::new(None));
    let num_docs = LuceneTestCase::at_least(1000);
    let mut threads = Vec::with_capacity(3);
    {
        let (indexing, error, w, mgr, query_cache) = (
            Arc::clone(&indexing),
            Arc::clone(&error),
            Arc::clone(&w),
            Arc::clone(&mgr),
            Arc::clone(&query_cache),
        );
        threads.push(Thread::unnamed(move || {
            let mut doc = Document::new();
            let f = unchecked(string_field("color", ""));
            doc.add(Box::new(f));
            let mut i = 0;
            while indexing.load(Ordering::SeqCst) && i < num_docs {
                let color = *RandomPicks::random_from(&*random(), &["blue", "red", "yellow"]);
                unchecked(
                    field_mut_as::<StringField>(&mut doc, "color")
                        .set_string_value(color.to_string()),
                );
                let outcome = try_catch("java.lang.Throwable", || {
                    unchecked(w.add_document(&doc));
                    if (i & 63) == 0 {
                        unchecked(mgr.maybe_refresh());
                        if LuceneTestCase::rarely() {
                            query_cache.clear();
                        }
                        if LuceneTestCase::rarely() {
                            let color =
                                *RandomPicks::random_from(&*random(), &["blue", "red", "yellow"]);
                            unchecked(w.delete_documents(term("color", color)));
                        }
                    }
                });
                if let Err(t) = outcome {
                    set_first_error(&error, t);
                    break;
                }
                i += 1;
            }
            indexing.store(false, Ordering::SeqCst);
        }));
    }
    for _ in 1..3 {
        let (indexing, error, mgr) = (Arc::clone(&indexing), Arc::clone(&error), Arc::clone(&mgr));
        threads.push(Thread::unnamed(move || {
            while indexing.load(Ordering::SeqCst) {
                let outcome = try_catch("java.lang.Throwable", || {
                    let searcher = unchecked(mgr.acquire());
                    let body = try_catch("java.lang.Throwable", || {
                        let value = *RandomPicks::random_from(
                            &*random(),
                            &["blue", "red", "yellow", "green"],
                        );
                        let q: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", value)));
                        let collector_manager = DummyTotalHitCountCollector::create_manager();
                        // will use the cache
                        let total_hits1 = unchecked(
                            searcher
                                .searcher()
                                .search_with_manager(Arc::clone(&q), &collector_manager),
                        );
                        let total_hits2 = unchecked(searcher.searcher().search_with_manager(
                            Arc::clone(&q),
                            &ScoringFilterCollectorManager { collector_manager },
                        )) as i64;
                        assert_equals(&total_hits2, &(total_hits1 as i64));
                    });
                    unchecked(mgr.release(searcher));
                    if let Err(t) = body {
                        t.throw();
                    }
                });
                if let Err(t) = outcome {
                    set_first_error(&error, t);
                }
            }
        }));
    }

    for thread in &threads {
        thread.start();
    }

    for thread in &threads {
        let _ = thread.join();
    }

    let first = error.lock().unwrap_or_else(|e| e.into_inner()).take();
    let outcome = try_catch("java.lang.Throwable", || {
        if let Some(t) = first {
            t.throw();
        }
        unchecked(query_cache.assert_consistent());
    });
    mgr.close()?;
    w.close()?;
    dir.close()?;
    query_cache.assert_consistent()?;
    if let Err(t) = outcome {
        t.throw();
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// testLRUEviction, testClearFilter
// -----------------------------------------------------------------------------

/// `Thread.sleep(200)`: lets the cache load complete.
fn let_the_cache_load_complete() {
    std::thread::sleep(Duration::from_millis(200));
}

/// Port of `TestLRUQueryCache.testLRUEviction`.
fn test_lru_eviction() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    let f = string_field("color", "blue")?;
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    field_mut_as::<StringField>(&mut doc, "color").set_string_value("red".to_string())?;
    w.add_document(&doc)?;
    field_mut_as::<StringField>(&mut doc, "color").set_string_value("green".to_string())?;
    w.add_document(&doc)?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query_cache = lru(2, 100000, |_| true, f32::INFINITY)?;

    let blue: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "blue")));
    let red: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "red")));
    let green: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "green")));
    let csq =
        |q: &Arc<dyn Query>| -> Arc<dyn Query> { Arc::new(ConstantScoreQuery::new(Arc::clone(q))) };

    assert_equals(&Vec::<QueryKey>::new(), &cached(&query_cache));

    searcher.set_query_cache(as_cache(&query_cache));
    // the filter is not cached on any segment: no changes
    searcher.set_query_caching_policy(never_cache());
    searcher.search(csq(&green), 1)?;
    assert_equals(&Vec::<QueryKey>::new(), &cached(&query_cache));

    searcher.set_query_caching_policy(always_cache());
    searcher.search(csq(&red), 1)?;

    if cached(&query_cache) != Vec::<QueryKey>::new() {
        assert_equals(&keys(&[&red]), &cached(&query_cache));
    } else {
        // Let the cache load be completed
        let_the_cache_load_complete();
        assert_equals(&keys(&[&red]), &cached(&query_cache));
    }

    searcher.search(csq(&green), 1)?;

    if cached(&query_cache) != keys(&[&red]) {
        assert_equals(&keys(&[&red, &green]), &cached(&query_cache));
    } else {
        // Let the cache load be completed
        let_the_cache_load_complete();
        assert_equals(&keys(&[&red, &green]), &cached(&query_cache));
    }

    searcher.search(csq(&red), 1)?;
    assert_equals(&keys(&[&green, &red]), &cached(&query_cache));

    searcher.search(csq(&blue), 1)?;

    if cached(&query_cache) != keys(&[&green, &red]) {
        assert_equals(&keys(&[&red, &blue]), &cached(&query_cache));
    } else {
        // Let the cache load be completed
        let_the_cache_load_complete();
        assert_equals(&keys(&[&red, &blue]), &cached(&query_cache));
    }

    searcher.search(csq(&blue), 1)?;
    assert_equals(&keys(&[&red, &blue]), &cached(&query_cache));

    searcher.search(csq(&green), 1)?;

    if cached(&query_cache) != keys(&[&red, &blue]) {
        assert_equals(&keys(&[&blue, &green]), &cached(&query_cache));
    } else {
        // Let the cache load be completed
        let_the_cache_load_complete();
        assert_equals(&keys(&[&blue, &green]), &cached(&query_cache));
    }

    searcher.set_query_caching_policy(never_cache());
    searcher.search(csq(&red), 1)?;
    assert_equals(&keys(&[&blue, &green]), &cached(&query_cache));

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testClearFilter`.
fn test_clear_filter() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    let f = string_field("color", "")?;
    doc.add(Box::new(f));
    let num_docs = LuceneTestCase::at_least(10);
    for _ in 0..num_docs {
        field_mut_as::<StringField>(&mut doc, "color").set_string_value(
            (if random().next_boolean() {
                "red"
            } else {
                "blue"
            })
            .to_string(),
        )?;
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query1: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "blue")));
    // different instance yet equal
    let query2: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "blue")));

    let query_cache = lru(i32::MAX, i64::MAX, |_| true, 1.0)?;
    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    let boost = random().next_float();
    searcher.search(
        Arc::new(BoostQuery::new(
            Arc::new(ConstantScoreQuery::new(Arc::clone(&query1))),
            boost,
        )?),
        1,
    )?;
    assert_equals(&1, &query_cache.cached_queries().len());

    query_cache.clear_query(query2);

    Assert::assert_true(query_cache.cached_queries().is_empty());
    query_cache.assert_consistent()?;

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// RAM accounting
// -----------------------------------------------------------------------------

/// `o.getClass().getSimpleName()` of a top-level or nested Java class.
fn simple_name(o: &dyn RamObject) -> &'static str {
    o.java_class_name()
        .rsplit(['.', '$'])
        .next()
        .unwrap_or_default()
}

/// `((DocIdSet) o).ramBytesUsed()`.
fn doc_id_set_ram_bytes_used(o: &dyn RamObject) -> i64 {
    o.accountable_ram_bytes_used()
        .expect("INVARIANT: a modelled DocIdSet carries its ramBytesUsed()")
}

/// The accumulator of `testRamBytesUsedAgreesWithRamUsageTester`: an
/// accumulator that only sums up memory usage of referenced filters and doc
/// id sets.
struct FiltersAndDocIdSetsAccumulator;

impl Accumulator for FiltersAndDocIdSetsAccumulator {
    fn accumulate_object<'a>(
        &mut self,
        o: &'a dyn RamObject,
        shallow_size: i64,
        field_values: &[(&'static str, Option<&'a dyn RamObject>)],
        queue: &mut Queue<'a>,
    ) -> i64 {
        if o.instance_of("org.apache.lucene.search.DocIdSet") {
            return doc_id_set_ram_bytes_used(o);
        }
        if o.instance_of("org.apache.lucene.search.Query") {
            return RamUsageEstimator::QUERY_DEFAULT_RAM_BYTES_USED;
        }
        if o.instance_of("org.apache.lucene.index.IndexReader")
            || simple_name(o) == "SegmentCoreReaders"
        {
            // do not take readers or core cache keys into account
            return 0;
        }
        if let JavaObject::JavaMap { entries, .. } = o.java_object() {
            queue.extend(entries.iter().map(|(k, _)| *k));
            queue.extend(entries.iter().map(|(_, v)| *v));
            let size_per_entry = if o.instance_of("java.util.LinkedHashMap") {
                LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY
            } else {
                HASHTABLE_RAM_BYTES_PER_ENTRY
            };
            return size_per_entry * entries.len() as i64;
        }
        // follow links to other objects, but ignore their memory usage
        DefaultAccumulator.accumulate_object(o, shallow_size, field_values, queue);
        0
    }

    fn accumulate_array<'a>(
        &mut self,
        array: &'a dyn RamObject,
        shallow_size: i64,
        values: &[Option<&'a dyn RamObject>],
        queue: &mut Queue<'a>,
    ) -> i64 {
        // follow links to other objects, but ignore their memory usage
        DefaultAccumulator.accumulate_array(array, shallow_size, values, queue);
        0
    }
}

/// This test makes sure that by making the same assumptions as LRUQueryCache,
/// RAMUsageTester computes the same memory usage.
///
/// Port of `TestLRUQueryCache.testRamBytesUsedAgreesWithRamUsageTester`.
///
/// `o instanceof Map` is answered by the object's map shape: only a JDK map
/// has one, and the walk hands a JDK map to `accumulateArray`, as Java's
/// does.
fn test_ram_bytes_used_agrees_with_ram_usage_tester() -> Result<()> {
    let max_size = 1 + random().next_int_bound(5);
    let max_ram = 1 + random().next_int_bound(10000) as i64;
    let query_cache = lru(
        max_size,
        max_ram,
        |_| random().next_boolean(),
        f32::INFINITY,
    )?;

    let dir = LuceneTestCase::new_directory();
    // serial merges so that segments do not get closed while we are measuring
    // ram usage with RamUsageTester
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let colors = ["blue", "red", "green", "yellow"];

    let mut doc = Document::new();
    let f = string_field("color", "")?;
    doc.add(Box::new(f));
    let iters = LuceneTestCase::at_least(5);
    for _ in 0..iters {
        let num_docs = LuceneTestCase::at_least(10);
        for _ in 0..num_docs {
            field_mut_as::<StringField>(&mut doc, "color")
                .set_string_value(RandomPicks::random_from(&*random(), &colors).to_string())?;
            w.add_document(&doc)?;
        }
        {
            let reader = w.get_reader()?;
            let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            searcher.set_query_cache(as_cache(&query_cache));
            searcher.set_query_caching_policy(Arc::clone(&MAYBE_CACHE_POLICY));
            for _ in 0..3 {
                let query: Arc<dyn Query> = Arc::new(TermQuery::new(term(
                    "color",
                    RandomPicks::random_from(&*random(), &colors),
                )));
                searcher.search(Arc::new(ConstantScoreQuery::new(query)), 1)?;
            }
            reader.close()?;
        }
        query_cache.assert_consistent()?;
        assert_equals(
            &RamUsageTester::ram_used_with(
                Some(&*ModelBuilder::new().lru_query_cache(&query_cache)),
                &mut FiltersAndDocIdSetsAccumulator,
            ),
            &query_cache.ram_bytes_used(),
        );
    }

    w.close()?;
    dir.close()?;
    Ok(())
}

/// `DummyQuery.COUNTER`.
static COUNTER: AtomicI32 = AtomicI32::new(0);

/// A query that doesn't match anything.
///
/// `private static class DummyQuery extends Query`.
#[derive(Debug, Clone)]
struct DummyQuery {
    id: i32,
    /// `AccountableDummyQuery extends DummyQuery implements Accountable`.
    accountable: bool,
}

impl DummyQuery {
    fn new() -> DummyQuery {
        DummyQuery {
            id: COUNTER.fetch_add(1, Ordering::SeqCst),
            accountable: false,
        }
    }
}

/// The anonymous `ConstantScoreWeight` of `DummyQuery.createWeight`.
#[derive(Debug)]
struct NullScorerWeight;

impl ConstantScoreWeightImpl for NullScorerWeight {
    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
        _score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(None)
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Query for DummyQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            NullScorerWeight,
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_accountable(&self) -> Option<&dyn Accountable> {
        if self.accountable {
            Some(self)
        } else {
            None
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<DummyQuery>()
                .is_some_and(|o| self.id == o.id && self.accountable == o.accountable)
    }

    fn query_hash(&self) -> u64 {
        self.id as u32 as u64
    }

    fn to_query_string(&self, _field: &str) -> String {
        "DummyQuery".to_string()
    }
}

impl Accountable for DummyQuery {
    /// `AccountableDummyQuery.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        10 * RamUsageEstimator::QUERY_DEFAULT_RAM_BYTES_USED
    }
}

/// DummyQuery with Accountable, pretending to be a memory-eating query.
///
/// `new AccountableDummyQuery()`.
fn accountable_dummy_query() -> DummyQuery {
    DummyQuery {
        accountable: true,
        ..DummyQuery::new()
    }
}

/// The accumulator of `testRamBytesUsedConstantEntryOverhead`.
struct ConstantEntryOverheadAccumulator;

impl Accumulator for ConstantEntryOverheadAccumulator {
    fn accumulate_object<'a>(
        &mut self,
        o: &'a dyn RamObject,
        shallow_size: i64,
        field_values: &[(&'static str, Option<&'a dyn RamObject>)],
        queue: &mut Queue<'a>,
    ) -> i64 {
        if o.instance_of("org.apache.lucene.search.DocIdSet") {
            return doc_id_set_ram_bytes_used(o);
        }
        if o.instance_of("org.apache.lucene.search.Query") {
            return RamUsageEstimator::QUERY_DEFAULT_RAM_BYTES_USED;
        }
        if simple_name(o) == "SegmentCoreReaders" {
            // do not follow references to core cache keys
            return 0;
        }
        DefaultAccumulator.accumulate_object(o, shallow_size, field_values, queue)
    }
}

/// Test what happens when the cache contains only filters and doc id sets that
/// require very little memory. In that case most of the memory is taken by the
/// cache itself, not cache entries, and we want to make sure that memory usage
/// is not grossly underestimated.
///
/// Port of `TestLRUQueryCache.testRamBytesUsedConstantEntryOverhead`.
fn test_ram_bytes_used_constant_entry_overhead() -> Result<()> {
    let query_cache = lru(1000000, 10000000, |_| true, f32::INFINITY)?;

    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let doc = Document::new();
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    let num_queries = LuceneTestCase::at_least(1000);
    for _ in 0..num_queries {
        let query: Arc<dyn Query> = Arc::new(DummyQuery::new());
        searcher.search(Arc::new(ConstantScoreQuery::new(query)), 1)?;
    }
    Assert::assert_true(query_cache.get_cache_count() > 0);

    let actual_ram_bytes_used = RamUsageTester::ram_used_with(
        Some(&*ModelBuilder::new().lru_query_cache(&query_cache)),
        &mut ConstantEntryOverheadAccumulator,
    );
    let expected_ram_bytes_used = query_cache.ram_bytes_used();
    // error < 30%
    assert_equals_delta(
        actual_ram_bytes_used as f64,
        expected_ram_bytes_used as f64,
        30.0 * actual_ram_bytes_used as f64 / 100.0,
    );

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testCachingAccountableQuery`.
fn test_caching_accountable_query() -> Result<()> {
    let query_cache = lru(1000000, 10000000, |_| true, f32::INFINITY)?;

    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let doc = Document::new();
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    let num_queries = random().next_int_bound(100) + 100;
    for _ in 0..num_queries {
        let query: Arc<dyn Query> = Arc::new(accountable_dummy_query());
        searcher.count(query)?;
    }
    let query_ram_bytes_used =
        num_queries as i64 * (10 * RamUsageEstimator::QUERY_DEFAULT_RAM_BYTES_USED);
    // make sure the query cache reflects the big queries
    Assert::assert_true(query_cache.ram_bytes_used() > query_ram_bytes_used);

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testConsistencyWithAccountableQueries`.
fn test_consistency_with_accountable_queries() -> Result<()> {
    let query_cache = lru(1, 10000000, |_| true, f32::INFINITY)?;

    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    writer.add_document(&Document::new())?;
    let reader = writer.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    query_cache.assert_consistent()?;

    let accountable_query = accountable_dummy_query();
    let accountable_ram = accountable_query.ram_bytes_used();
    let accountable_query: Arc<dyn Query> = Arc::new(accountable_query);
    searcher.count(Arc::clone(&accountable_query))?;
    let expected_ram_bytes_used = HASHTABLE_RAM_BYTES_PER_ENTRY
        + LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY
        + accountable_ram
        + query_cache.child_resources()[0].ram_bytes_used();
    assert_equals(&expected_ram_bytes_used, &query_cache.ram_bytes_used());
    query_cache.assert_consistent()?;

    query_cache.clear_query(accountable_query);
    assert_equals(
        &HASHTABLE_RAM_BYTES_PER_ENTRY,
        &query_cache.ram_bytes_used(),
    );
    query_cache.assert_consistent()?;

    query_cache.clear_core_cache_key(&core_key(&leaves(&reader)?[0]));
    assert_equals(&0, &query_cache.ram_bytes_used());
    query_cache.assert_consistent()?;

    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testOnUse, testStats, testFineGrainedStats
// -----------------------------------------------------------------------------

/// The anonymous counting `QueryCachingPolicy` of `testOnUse`.
#[derive(Debug)]
struct CountingPolicy {
    expected_counts: Arc<Mutex<HashMap<QueryKey, i32>>>,
}

impl QueryCachingPolicy for CountingPolicy {
    fn should_cache(&self, _query: &dyn Query) -> Result<bool> {
        Ok(random().next_boolean())
    }

    fn on_use(&self, query: &dyn Query) {
        let mut expected_counts = self
            .expected_counts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let key = QueryKey::new(owned_query(query));
        let count = 1 + expected_counts.get(&key).copied().unwrap_or(0);
        expected_counts.insert(key, count);
    }
}

/// Port of `TestLRUQueryCache.testOnUse`.
fn test_on_use() -> Result<()> {
    let max_size = 1 + random().next_int_bound(5);
    let max_ram = 1 + random().next_int_bound(1000) as i64;
    let query_cache = lru(
        max_size,
        max_ram,
        |_| random().next_boolean(),
        f32::INFINITY,
    )?;

    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let colors = ["red", "blue", "green", "yellow"];
    let mut doc = Document::new();
    let f = string_field("color", "")?;
    doc.add(Box::new(f));
    let num_docs = LuceneTestCase::at_least(10);
    for _ in 0..num_docs {
        field_mut_as::<StringField>(&mut doc, "color")
            .set_string_value(RandomPicks::random_from(&*random(), &colors).to_string())?;
        w.add_document(&doc)?;
        if random().next_boolean() {
            w.get_reader()?.close()?;
        }
    }
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    let mut actual_counts: HashMap<QueryKey, i32> = HashMap::new();
    let expected_counts: Arc<Mutex<HashMap<QueryKey, i32>>> = Arc::new(Mutex::new(HashMap::new()));

    let counting_policy: Arc<dyn QueryCachingPolicy> = Arc::new(CountingPolicy {
        expected_counts: Arc::clone(&expected_counts),
    });

    let n = 10 + random().next_int_bound(10);
    let mut queries: Vec<Arc<dyn Query>> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let tq: Arc<dyn Query> = Arc::new(TermQuery::new(term(
            "color",
            RandomPicks::random_from(&*random(), &colors),
        )));
        queries.push(Arc::new(BoostQuery::new(tq, random().next_float())?));
    }

    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(counting_policy);
    for _ in 0..20 {
        let idx = random().next_int_bound(queries.len() as i32) as usize;
        searcher.search(
            Arc::new(ConstantScoreQuery::new(Arc::clone(&queries[idx]))),
            1,
        )?;
        let mut cache_key = Arc::clone(&queries[idx]);
        while let Some(boost) = cache_key.as_any().downcast_ref::<BoostQuery>() {
            cache_key = boost.get_query();
        }
        let key = QueryKey::new(cache_key);
        let count = 1 + actual_counts.get(&key).copied().unwrap_or(0);
        actual_counts.insert(key, count);
    }

    let expected_counts = expected_counts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_equals(&actual_counts, &expected_counts);

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testStats`.
fn test_stats() -> Result<()> {
    let query_cache = lru(1, 10000000, |_| true, 1.0)?;

    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let colors = ["blue", "red", "green", "yellow"];

    let mut doc = Document::new();
    let f = string_field("color", "")?;
    doc.add(Box::new(f));
    for _ in 0..10 {
        field_mut_as::<StringField>(&mut doc, "color")
            .set_string_value(RandomPicks::random_from(&*random(), &colors).to_string())?;
        w.add_document(&doc)?;
        if random().next_boolean() {
            w.get_reader()?.close()?;
        }
    }

    let reader = w.get_reader()?;
    let segment_count = leaves(&reader)?.len() as i64;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "red")));
    let query2: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "blue")));
    let csq =
        |q: &Arc<dyn Query>| -> Arc<dyn Query> { Arc::new(ConstantScoreQuery::new(Arc::clone(q))) };

    searcher.set_query_cache(as_cache(&query_cache));
    // first pass, lookups without caching that all miss
    searcher.set_query_caching_policy(never_cache());
    for _ in 0..10 {
        searcher.search(csq(&query), 1)?;
    }
    assert_equals(&(10 * segment_count), &query_cache.get_total_count());
    assert_equals(&0, &query_cache.get_hit_count());
    assert_equals(&(10 * segment_count), &query_cache.get_miss_count());
    assert_equals(&0, &query_cache.get_cache_count());
    assert_equals(&0, &query_cache.get_eviction_count());
    assert_equals(&0, &query_cache.get_cache_size());

    // second pass, lookups + caching, only the first one is a miss
    searcher.set_query_caching_policy(always_cache());
    for _ in 0..10 {
        searcher.search(csq(&query), 1)?;
    }
    assert_equals(&(20 * segment_count), &query_cache.get_total_count());
    assert_equals(&(9 * segment_count), &query_cache.get_hit_count());
    assert_equals(&(11 * segment_count), &query_cache.get_miss_count());
    assert_equals(&segment_count, &query_cache.get_cache_count());
    assert_equals(&0, &query_cache.get_eviction_count());
    assert_equals(&segment_count, &query_cache.get_cache_size());

    // third pass lookups without caching, we only have hits
    searcher.set_query_caching_policy(never_cache());
    for _ in 0..10 {
        searcher.search(csq(&query), 1)?;
    }
    assert_equals(&(30 * segment_count), &query_cache.get_total_count());
    assert_equals(&(19 * segment_count), &query_cache.get_hit_count());
    assert_equals(&(11 * segment_count), &query_cache.get_miss_count());
    assert_equals(&segment_count, &query_cache.get_cache_count());
    assert_equals(&0, &query_cache.get_eviction_count());
    assert_equals(&segment_count, &query_cache.get_cache_size());

    // fourth pass with a different filter which will trigger evictions since
    // the size is 1
    searcher.set_query_caching_policy(always_cache());
    for _ in 0..10 {
        searcher.search(csq(&query2), 1)?;
    }
    assert_equals(&(40 * segment_count), &query_cache.get_total_count());
    assert_equals(&(28 * segment_count), &query_cache.get_hit_count());
    assert_equals(&(12 * segment_count), &query_cache.get_miss_count());
    assert_equals(&(2 * segment_count), &query_cache.get_cache_count());
    assert_equals(&segment_count, &query_cache.get_eviction_count());
    assert_equals(&segment_count, &query_cache.get_cache_size());

    // now close, causing evictions due to the closing of segment cores
    reader.close()?;
    w.close()?;
    assert_equals(&(40 * segment_count), &query_cache.get_total_count());
    assert_equals(&(28 * segment_count), &query_cache.get_hit_count());
    assert_equals(&(12 * segment_count), &query_cache.get_miss_count());
    assert_equals(&(2 * segment_count), &query_cache.get_cache_count());
    assert_equals(&(2 * segment_count), &query_cache.get_eviction_count());
    assert_equals(&0, &query_cache.get_cache_size());

    dir.close()?;
    Ok(())
}

/// The anonymous `LRUQueryCache` subclass of `testFineGrainedStats`.
#[derive(Debug)]
struct FineGrainedStats {
    index_id: HashMap<CacheKey, i32>,
    hit_count1: AtomicI64,
    hit_count2: AtomicI64,
    miss_count1: AtomicI64,
    miss_count2: AtomicI64,
    ram_bytes_usage: AtomicI64,
    cache_size: AtomicI64,
}

impl FineGrainedStats {
    /// `switch (indexId.get(readerCoreKey))`.
    fn count(&self, reader_core_key: &CacheKey, one: &AtomicI64, two: &AtomicI64) {
        match self.index_id.get(reader_core_key) {
            Some(1) => {
                one.fetch_add(1, Ordering::SeqCst);
            }
            Some(2) => {
                two.fetch_add(1, Ordering::SeqCst);
            }
            Some(_) => Throwable::new("java.lang.AssertionError", "").throw(),
            // `switch` on a null `Integer` unboxes it.
            None => Throwable::new("java.lang.NullPointerException", "").throw(),
        }
    }
}

impl LRUQueryCacheOverrides for FineGrainedStats {
    fn on_hit(&self, cache: &LRUQueryCache, reader_core_key: &CacheKey, query: &Arc<dyn Query>) {
        cache.super_on_hit(reader_core_key, query);
        self.count(reader_core_key, &self.hit_count1, &self.hit_count2);
    }

    fn on_miss(&self, cache: &LRUQueryCache, reader_core_key: &CacheKey, query: &Arc<dyn Query>) {
        cache.super_on_miss(reader_core_key, query);
        self.count(reader_core_key, &self.miss_count1, &self.miss_count2);
    }

    fn on_query_cache(&self, cache: &LRUQueryCache, query: &Arc<dyn Query>, ram_bytes_used: i64) {
        cache.super_on_query_cache(query, ram_bytes_used);
        // assertNotNull("cached query is null", query): a reference is never
        // null in Rust.
        self.ram_bytes_usage
            .fetch_add(ram_bytes_used, Ordering::SeqCst);
    }

    fn on_query_eviction(
        &self,
        cache: &LRUQueryCache,
        query: &Arc<dyn Query>,
        ram_bytes_used: i64,
    ) {
        cache.super_on_query_eviction(query, ram_bytes_used);
        // assertNotNull("evicted query is null", query): a reference is never
        // null in Rust.
        self.ram_bytes_usage
            .fetch_add(-ram_bytes_used, Ordering::SeqCst);
    }

    fn on_doc_id_set_cache(
        &self,
        cache: &LRUQueryCache,
        reader_core_key: &CacheKey,
        ram_bytes_used: i64,
    ) {
        cache.super_on_doc_id_set_cache(reader_core_key, ram_bytes_used);
        self.ram_bytes_usage
            .fetch_add(ram_bytes_used, Ordering::SeqCst);
        self.cache_size.fetch_add(1, Ordering::SeqCst);
    }

    fn on_doc_id_set_eviction(
        &self,
        cache: &LRUQueryCache,
        reader_core_key: &CacheKey,
        num_entries: i32,
        sum_ram_bytes_used: i64,
    ) {
        cache.super_on_doc_id_set_eviction(reader_core_key, num_entries, sum_ram_bytes_used);
        self.ram_bytes_usage
            .fetch_add(-sum_ram_bytes_used, Ordering::SeqCst);
        self.cache_size
            .fetch_add(-(num_entries as i64), Ordering::SeqCst);
    }

    fn on_clear(&self, cache: &LRUQueryCache) {
        cache.super_on_clear();
        self.ram_bytes_usage.store(0, Ordering::SeqCst);
        self.cache_size.store(0, Ordering::SeqCst);
    }
}

/// Port of `TestLRUQueryCache.testFineGrainedStats`.
fn test_fine_grained_stats() -> Result<()> {
    let dir1 = LuceneTestCase::new_directory();
    let w1 = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir1.as_directory())?;
    let dir2 = LuceneTestCase::new_directory();
    let w2 = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir2.as_directory())?;

    let colors = ["blue", "red", "green", "yellow"];

    let mut doc = Document::new();
    let f = string_field("color", "")?;
    doc.add(Box::new(f));
    for w in [&w1, &w2] {
        for _ in 0..10 {
            field_mut_as::<StringField>(&mut doc, "color")
                .set_string_value(RandomPicks::random_from(&*random(), &colors).to_string())?;
            w.add_document(&doc)?;
            if random().next_boolean() {
                w.get_reader()?.close()?;
            }
        }
    }

    let reader1 = w1.get_reader()?;
    let segment_count1 = leaves(&reader1)?.len() as i64;
    let mut searcher1 = IndexSearcher::new(Arc::clone(&reader1) as Arc<dyn IndexReader>)?;

    let reader2 = w2.get_reader()?;
    let segment_count2 = leaves(&reader2)?.len() as i64;
    let mut searcher2 = IndexSearcher::new(Arc::clone(&reader2) as Arc<dyn IndexReader>)?;

    let mut index_id: HashMap<CacheKey, i32> = HashMap::new();
    for ctx in leaves(&reader1)? {
        index_id.insert(core_key(&ctx), 1);
    }
    for ctx in leaves(&reader2)? {
        index_id.insert(core_key(&ctx), 2);
    }

    let stats = Arc::new(FineGrainedStats {
        index_id,
        hit_count1: AtomicI64::new(0),
        hit_count2: AtomicI64::new(0),
        miss_count1: AtomicI64::new(0),
        miss_count2: AtomicI64::new(0),
        ram_bytes_usage: AtomicI64::new(0),
        cache_size: AtomicI64::new(0),
    });
    let query_cache = lru_with(
        2,
        10000000,
        |_| true,
        1.0,
        Arc::clone(&stats) as Arc<dyn LRUQueryCacheOverrides>,
    )?;
    let hit_count1 = || stats.hit_count1.load(Ordering::SeqCst);
    let hit_count2 = || stats.hit_count2.load(Ordering::SeqCst);
    let miss_count1 = || stats.miss_count1.load(Ordering::SeqCst);
    let miss_count2 = || stats.miss_count2.load(Ordering::SeqCst);

    let query: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "red")));
    let query2: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "blue")));
    let query3: Arc<dyn Query> = Arc::new(TermQuery::new(term("color", "green")));
    let csq =
        |q: &Arc<dyn Query>| -> Arc<dyn Query> { Arc::new(ConstantScoreQuery::new(Arc::clone(q))) };

    for searcher in [&mut searcher1, &mut searcher2] {
        searcher.set_query_cache(as_cache(&query_cache));
        searcher.set_query_caching_policy(always_cache());
    }

    // search on searcher1
    for _ in 0..10 {
        searcher1.search(csq(&query), 1)?;
    }
    assert_equals(&(9 * segment_count1), &hit_count1());
    assert_equals(&0, &hit_count2());
    assert_equals(&segment_count1, &miss_count1());
    assert_equals(&0, &miss_count2());

    // then on searcher2
    for _ in 0..20 {
        searcher2.search(csq(&query2), 1)?;
    }
    assert_equals(&(9 * segment_count1), &hit_count1());
    assert_equals(&(19 * segment_count2), &hit_count2());
    assert_equals(&segment_count1, &miss_count1());
    assert_equals(&segment_count2, &miss_count2());

    // now on searcher1 again to trigger evictions
    for _ in 0..30 {
        searcher1.search(csq(&query3), 1)?;
    }
    assert_equals(&segment_count1, &query_cache.get_eviction_count());
    assert_equals(&(38 * segment_count1), &hit_count1());
    assert_equals(&(19 * segment_count2), &hit_count2());
    assert_equals(&(2 * segment_count1), &miss_count1());
    assert_equals(&segment_count2, &miss_count2());

    // check that the recomputed stats are the same as those reported by the
    // cache
    assert_equals(
        &query_cache.ram_bytes_used(),
        &((segment_count1 + segment_count2) * HASHTABLE_RAM_BYTES_PER_ENTRY
            + stats.ram_bytes_usage.load(Ordering::SeqCst)),
    );
    assert_equals(
        &query_cache.get_cache_size(),
        &stats.cache_size.load(Ordering::SeqCst),
    );

    reader1.close()?;
    reader2.close()?;
    w1.close()?;
    w2.close()?;

    assert_equals(
        &query_cache.ram_bytes_used(),
        &stats.ram_bytes_usage.load(Ordering::SeqCst),
    );
    assert_equals(&0, &stats.cache_size.load(Ordering::SeqCst));

    query_cache.clear();
    assert_equals(&0, &stats.ram_bytes_usage.load(Ordering::SeqCst));
    assert_equals(&0, &stats.cache_size.load(Ordering::SeqCst));

    dir1.close()?;
    dir2.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Cache keys and sub-clauses
// -----------------------------------------------------------------------------

/// The anonymous `QueryCachingPolicy` of `testUseRewrittenQueryAsCacheKey`.
#[derive(Debug)]
struct ExpectKeyPolicy {
    expected_cache_key: Arc<dyn Query>,
}

impl QueryCachingPolicy for ExpectKeyPolicy {
    fn should_cache(&self, query: &dyn Query) -> Result<bool> {
        assert_equals(
            &QueryKey::new(Arc::clone(&self.expected_cache_key)),
            &QueryKey::new(owned_query(query)),
        );
        Ok(true)
    }

    fn on_use(&self, query: &dyn Query) {
        assert_equals(
            &QueryKey::new(Arc::clone(&self.expected_cache_key)),
            &QueryKey::new(owned_query(query)),
        );
    }
}

/// Port of `TestLRUQueryCache.testUseRewrittenQueryAsCacheKey`.
fn test_use_rewritten_query_as_cache_key() -> Result<()> {
    let expected_cache_key: Arc<dyn Query> = Arc::new(TermQuery::new(term("foo", "bar")));
    let mut query = BooleanQueryBuilder::new();
    query.add(
        Arc::new(BoostQuery::new(Arc::clone(&expected_cache_key), 42.0)?),
        Occur::MUST,
    )?;

    let query_cache = lru(
        1000000,
        10000000,
        |_| random().next_boolean(),
        f32::INFINITY,
    )?;
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&stored_doc(&[("foo", "bar")])?)?;
    w.commit()?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    w.close()?;

    let policy: Arc<dyn QueryCachingPolicy> = Arc::new(ExpectKeyPolicy {
        expected_cache_key: Arc::clone(&expected_cache_key),
    });

    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(policy);
    searcher.search_with_manager(
        Arc::new(query.build()),
        &DummyTotalHitCountCollector::create_manager(),
    )?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testBooleanQueryCachesSubClauses`.
fn test_boolean_query_caches_sub_clauses() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&stored_doc(&[("foo", "bar"), ("foo", "quux")])?)?;
    w.commit()?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    w.close()?;

    let query_cache = lru(1000000, 10000000, |_| true, f32::INFINITY)?;
    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    let mut bq = BooleanQueryBuilder::new();
    let must: Arc<dyn Query> = Arc::new(TermQuery::new(term("foo", "bar")));
    let filter: Arc<dyn Query> = Arc::new(TermQuery::new(term("foo", "quux")));
    let must_not: Arc<dyn Query> = Arc::new(TermQuery::new(term("foo", "foo")));
    bq.add(Arc::clone(&must), Occur::MUST)?;
    bq.add(Arc::clone(&filter), Occur::FILTER)?;
    bq.add(Arc::clone(&must_not), Occur::MUST_NOT)?;

    // same bq but with FILTER instead of MUST
    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(Arc::clone(&must), Occur::FILTER)?;
    bq2.add(Arc::clone(&filter), Occur::FILTER)?;
    bq2.add(Arc::clone(&must_not), Occur::MUST_NOT)?;

    assert_equals(&HashSet::new(), &cached_set(&query_cache));
    searcher.search(Arc::new(bq.build()), 1)?;
    assert_equals(&key_set(&[&filter, &must_not]), &cached_set(&query_cache));

    query_cache.clear();
    assert_equals(&HashSet::new(), &cached_set(&query_cache));
    searcher.search(Arc::new(ConstantScoreQuery::new(Arc::new(bq.build()))), 1)?;
    let bq2: Arc<dyn Query> = Arc::new(bq2.build());
    assert_equals(
        &key_set(&[&bq2, &must, &filter, &must_not]),
        &cached_set(&query_cache),
    );

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `BulkScorer` of
/// `testCachePopulationUsesLeafCollectorCollectRange`.
struct CollectRangeBulkScorer {
    min_doc_id: i32,
    max_doc_id: i32,
}

impl BulkScorer for CollectRangeBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        _accept_docs: Option<&dyn Bits>,
        _min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        collector.collect_range(self.min_doc_id, self.max_doc_id, &mut SimpleScorable::new())?;
        Ok(max)
    }

    fn cost(&self) -> i64 {
        (self.max_doc_id - self.min_doc_id) as i64
    }
}

/// Port of `TestLRUQueryCache.testCachePopulationUsesLeafCollectorCollectRange`.
fn test_cache_population_uses_leaf_collector_collect_range() -> Result<()> {
    let query_cache = lru(1_000_000, i64::MAX, |_| true, f32::INFINITY)?;
    let max_doc = random().next_int_range(100, 1_000_000);
    let min_doc_id = random().next_int_range(0, max_doc - 1);
    let max_doc_id = random().next_int_range(min_doc_id + 1, max_doc);
    let cache_and_count: CacheAndCount = query_cache.cache_impl(
        Box::new(CollectRangeBulkScorer {
            min_doc_id,
            max_doc_id,
        }),
        1_000_000,
    )?;
    assert_equals(&(max_doc_id - min_doc_id), &cache_and_count.count());
    let mut iterator = cache_and_count.iterator()?;
    assert_equals(&min_doc_id, &iterator.advance(0)?);
    assert_equals(&max_doc_id, &iterator.doc_id_run_end()?);
    assert_equals(&NO_MORE_DOCS, &iterator.advance(max_doc_id)?);
    Ok(())
}

// -----------------------------------------------------------------------------
// testRandom
// -----------------------------------------------------------------------------

const OCCUR_VALUES: [Occur; 4] = [Occur::MUST, Occur::FILTER, Occur::SHOULD, Occur::MUST_NOT];

fn random_term() -> Term {
    let t = *RandomPicks::random_from(&*random(), &["foo", "bar", "baz"]);
    term("foo", t)
}

fn build_random_query(level: i32) -> Result<Arc<dyn Query>> {
    if level == 10 {
        // at most 10 levels
        return Ok(Arc::new(MatchAllDocsQuery::instance()));
    }
    match random().next_int_bound(6) {
        0 => Ok(Arc::new(TermQuery::new(random_term()))),
        1 => {
            let mut bq = BooleanQueryBuilder::new();
            let num_clauses = TestUtil::next_int(&*random(), 1, 3);
            let mut num_should = 0;
            for _ in 0..num_clauses {
                let occur = *RandomPicks::random_from(&*random(), &OCCUR_VALUES);
                bq.add(build_random_query(level + 1)?, occur)?;
                if occur == Occur::SHOULD {
                    num_should += 1;
                }
            }
            bq.set_minimum_number_should_match(TestUtil::next_int(&*random(), 0, num_should));
            Ok(Arc::new(bq.build()))
        }
        2 => {
            let t1 = random_term();
            let t2 = random_term();
            let slop = random().next_int_bound(2);
            let pq = PhraseQuery::with_slop_bytes(
                slop,
                t1.field(),
                &[t1.bytes().clone(), t2.bytes().clone()],
            )?;
            Ok(Arc::new(pq))
        }
        3 => Ok(Arc::new(MatchAllDocsQuery::instance())),
        4 => Ok(Arc::new(ConstantScoreQuery::new(build_random_query(
            level + 1,
        )?))),
        5 => {
            let mut disjuncts = Vec::new();
            let num_queries = TestUtil::next_int(&*random(), 1, 3);
            for _ in 0..num_queries {
                disjuncts.push(build_random_query(level + 1)?);
            }
            Ok(Arc::new(DisjunctionMaxQuery::new(
                disjuncts,
                random().next_float(),
            )?))
        }
        _ => Throwable::new("java.lang.AssertionError", "").throw(),
    }
}

/// Port of `TestLRUQueryCache.testRandom`.
fn test_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let f = text_field("foo", "foo")?;
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    let mut reader = w.get_reader()?;

    let (max_size, max_ram_bytes_used, iters) = if *TEST_NIGHTLY {
        (
            TestUtil::next_int(&*random(), 1, 10000),
            TestUtil::next_long(&*random(), 1, 5000000),
            LuceneTestCase::at_least(20000),
        )
    } else {
        (
            TestUtil::next_int(&*random(), 1, 1000),
            TestUtil::next_long(&*random(), 1, 500000),
            LuceneTestCase::at_least(2000),
        )
    };

    let query_cache = lru(
        max_size,
        max_ram_bytes_used,
        |_| random().next_boolean(),
        f32::INFINITY,
    )?;
    let mut uncached_searcher: Option<IndexSearcher> = None;
    let mut cached_searcher: Option<IndexSearcher> = None;

    for i in 0..iters {
        if i == 0 || random().next_int_bound(100) == 1 {
            reader.close()?;
            field_mut_as::<TextField>(&mut doc, "foo").set_string_value(
                RandomPicks::random_from(&*random(), &["foo", "bar", "bar baz"]).to_string(),
            )?;
            w.add_document(&doc)?;
            if random().next_boolean() {
                w.delete_documents_by_query(build_random_query(0)?)?;
            }
            reader = w.get_reader()?;
            let mut uncached = LuceneTestCase::new_searcher(Arc::clone(&reader));
            uncached.set_query_cache(None);
            uncached_searcher = Some(uncached);
            let mut cached = LuceneTestCase::new_searcher(Arc::clone(&reader));
            cached.set_query_cache(as_cache(&query_cache));
            cached.set_query_caching_policy(always_cache());
            cached_searcher = Some(cached);
        }
        let uncached = uncached_searcher
            .as_ref()
            .expect("set on the first iteration");
        let cached = cached_searcher
            .as_ref()
            .expect("set on the first iteration");
        let q = build_random_query(0)?;
        // Counts are the same. If the query has already been cached this'll
        // use the O(1) Weight#count method.
        assert_equals(
            &uncached.count(Arc::clone(&q))?,
            &cached.count(Arc::clone(&q))?,
        );
        // Just to make sure we can iterate every time also check that the same
        // docs are returned in the same order.
        let size = 1 + random().next_int_bound(1000);
        CheckHits::check_equal(
            &*q,
            &uncached.search(Arc::clone(&q), size)?.score_docs,
            &cached.search(Arc::clone(&q), size)?.score_docs,
        );
        if LuceneTestCase::rarely() {
            query_cache.assert_consistent()?;
        }
    }
    query_cache.assert_consistent()?;
    w.close()?;
    reader.close()?;
    dir.close()?;
    query_cache.assert_consistent()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testDetectMutatedQueries
// -----------------------------------------------------------------------------

/// `private static class BadQuery extends Query`.
#[derive(Debug, Clone)]
struct BadQuery {
    /// An array so that clone keeps the reference.
    i: Arc<AtomicI32>,
}

impl Query for BadQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            NullScorerWeight,
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        "BadQuery".to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_hash(&self) -> u64 {
        (self.class_hash() as u32 ^ self.i.load(Ordering::SeqCst) as u32) as u64
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<BadQuery>()
                .is_some_and(|o| self.i.load(Ordering::SeqCst) == o.i.load(Ordering::SeqCst))
    }
}

/// Port of `TestLRUQueryCache.testDetectMutatedQueries`.
fn test_detect_mutated_queries() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    let reader = w.get_reader()?;

    // size of 1 so that 2nd query evicts from the cache
    let query_cache = lru(1, 10000, |_| true, f32::INFINITY)?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    let query = BadQuery {
        i: Arc::new(AtomicI32::new(42)),
    };
    searcher.search_with_manager(
        Arc::new(query.clone()),
        &DummyTotalHitCountCollector::create_manager(),
    )?;
    query.i.fetch_add(1, Ordering::SeqCst); // change the hashCode!

    // trigger an eviction
    match searcher.search_with_manager(
        Arc::new(MatchAllDocsQuery::instance()),
        &DummyTotalHitCountCollector::create_manager(),
    ) {
        Ok(_) => Assert::fail(""),
        Err(e) => {
            let t = java_throwable(&e);
            if t.is_instance("java.util.ConcurrentModificationException") {
                // expected
            } else if t.is_instance("java.lang.RuntimeException") {
                // expected: wrapped when executor is in use
                let cause = t.cause();
                Assert::assert_true(
                    cause.is_some_and(|c| c.is_instance("java.util.concurrent.ExecutionException")),
                );
                Assert::assert_true(
                    cause.and_then(|c| c.cause()).is_some_and(|c| {
                        c.is_instance("java.util.ConcurrentModificationException")
                    }),
                );
            } else {
                rethrow_java(e);
            }
        }
    }

    w.close()?;
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testRefuseToCacheTooLargeEntries`.
fn test_refuse_to_cache_too_large_entries() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for _ in 0..100 {
        w.add_document(&Document::new())?;
    }
    let reader = w.get_reader()?;

    // size of 1 byte
    let query_cache = lru(1, 1, |_| random().next_boolean(), f32::INFINITY)?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    searcher.count(Arc::new(MatchAllDocsQuery::instance()))?;
    assert_equals(&0, &query_cache.get_cache_count());
    assert_equals(&0, &query_cache.get_eviction_count());

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testOnUseWithRandomFirstSegmentSkipping
// -----------------------------------------------------------------------------

/// `private static class FrequencyCountingPolicy implements QueryCachingPolicy`.
#[derive(Debug, Default)]
struct FrequencyCountingPolicy {
    counts: Mutex<HashMap<QueryKey, Arc<AtomicI32>>>,
}

impl FrequencyCountingPolicy {
    fn frequency(&self, query: &Arc<dyn Query>) -> i32 {
        let count = {
            let counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
            counts.get(&QueryKey::new(Arc::clone(query))).cloned()
        };
        count.map_or(0, |c| c.load(Ordering::SeqCst))
    }
}

impl QueryCachingPolicy for FrequencyCountingPolicy {
    fn on_use(&self, query: &dyn Query) {
        let count = {
            let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                counts
                    .entry(QueryKey::new(owned_query(query)))
                    .or_insert_with(|| Arc::new(AtomicI32::new(0))),
            )
        };
        count.fetch_add(1, Ordering::SeqCst);
    }

    fn should_cache(&self, _query: &dyn Query) -> Result<bool> {
        Ok(true)
    }
}

/// Tests CachingWrapperWeight.scorer() propagation of
/// `QueryCachingPolicy#onUse(Query)` when the first segment is skipped.
///
/// #f:foo #f:bar causes all frequencies to increment #f:bar #f:foo does not
/// increment the frequency for f:foo
///
/// Port of `TestLRUQueryCache.testOnUseWithRandomFirstSegmentSkipping`.
fn test_on_use_with_random_first_segment_skipping() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    {
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
        let index_writer =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(string_field("f", "bar")?));
        index_writer.add_document(&doc)?;
        if random().next_boolean() {
            index_writer.get_reader()?.close()?;
        }
        let mut doc = Document::new();
        doc.add(Box::new(string_field("f", "foo")?));
        doc.add(Box::new(string_field("f", "bar")?));
        index_writer.add_document(&doc)?;
        index_writer.commit()?;
        index_writer.close()?;
    }
    {
        let index_reader = directory_reader::open(directory.as_directory())?;
        let policy = Arc::new(FrequencyCountingPolicy::default());
        let mut index_searcher =
            IndexSearcher::new(Arc::clone(&index_reader) as Arc<dyn IndexReader>)?;
        let cache = lru(100, 10240, |_| random().next_boolean(), f32::INFINITY)?;
        index_searcher.set_query_cache(as_cache(&cache));
        index_searcher.set_query_caching_policy(Arc::clone(&policy) as Arc<dyn QueryCachingPolicy>);
        let foo: Arc<dyn Query> = Arc::new(TermQuery::new(term("f", "foo")));
        let bar: Arc<dyn Query> = Arc::new(TermQuery::new(term("f", "bar")));
        let mut query = BooleanQueryBuilder::new();
        if random().next_boolean() {
            query.add(Arc::clone(&foo), Occur::FILTER)?;
            query.add(Arc::clone(&bar), Occur::FILTER)?;
        } else {
            query.add(Arc::clone(&bar), Occur::FILTER)?;
            query.add(Arc::clone(&foo), Occur::FILTER)?;
        }
        index_searcher.search_with_manager(
            Arc::new(query.build()),
            &DummyTotalHitCountCollector::create_manager(),
        )?;
        let built: Arc<dyn Query> = Arc::new(query.build());
        assert_equals(&1, &policy.frequency(&built));
        assert_equals(&1, &policy.frequency(&foo));
        assert_equals(&1, &policy.frequency(&bar));
        index_reader.close()?;
    }
    directory.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testPropagateBulkScorer
// -----------------------------------------------------------------------------

/// `private static class WeightWrapper extends FilterWeight`.
#[derive(Debug)]
struct WeightWrapper {
    base: FilterWeight,
    scorer_called: Arc<AtomicBool>,
    bulk_scorer_called: Arc<AtomicBool>,
}

impl WeightWrapper {
    fn new(
        r#in: Arc<dyn Weight>,
        scorer_called: Arc<AtomicBool>,
        bulk_scorer_called: Arc<AtomicBool>,
    ) -> Self {
        WeightWrapper {
            base: FilterWeight::new(r#in),
            scorer_called,
            bulk_scorer_called,
        }
    }
}

impl SegmentCacheable for WeightWrapper {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.base.is_cacheable(ctx)
    }
}

impl Weight for WeightWrapper {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.base.explain(context, doc)
    }

    fn matches(
        &self,
        context: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Arc<dyn rucene::search::matches::Matches>>> {
        self.base.matches(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.base.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(mut scorer_supplier) = self.base.inner.scorer_supplier(context)? else {
            return Ok(None);
        };
        let mut scorer = scorer_supplier.get(i64::MAX)?;
        let cost = scorer.iterator().cost();
        // `in.bulkScorer(context)` is `in.scorerSupplier(context)` set as the
        // top-level scoring clause, then `bulkScorer()`.
        let bulk_supplier = self.base.inner.scorer_supplier(context)?;
        Ok(Some(Box::new(WrapperScorerSupplier {
            scorer: Some(scorer),
            cost,
            bulk_supplier,
            scorer_called: Arc::clone(&self.scorer_called),
            bulk_scorer_called: Arc::clone(&self.bulk_scorer_called),
        })))
    }
}

/// The anonymous `ScorerSupplier` of `WeightWrapper.scorerSupplier`.
///
/// **Divergence from Lucene 10.5.0.** Java's `bulkScorer()` calls
/// `in.bulkScorer(context)` on the captured context; the port's supplier
/// outlives the borrowed context, so the wrapped weight's supplier that
/// `in.bulkScorer(context)` would build is built when this supplier is, and
/// `bulkScorer()` sets it as the top-level scoring clause and returns its
/// bulk scorer, which is what `Weight.bulkScorer` does.
struct WrapperScorerSupplier {
    scorer: Option<Box<dyn Scorer>>,
    /// `scorer.iterator().cost()`, read when the scorer is built: a scorer's
    /// iterator cost is stable.
    cost: i64,
    bulk_supplier: Option<Box<dyn ScorerSupplier>>,
    scorer_called: Arc<AtomicBool>,
    bulk_scorer_called: Arc<AtomicBool>,
}

impl ScorerSupplier for WrapperScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        self.scorer_called.store(true, Ordering::SeqCst);
        Ok(self.scorer.take().expect("get is called once"))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        self.bulk_scorer_called.store(true, Ordering::SeqCst);
        let mut supplier = self
            .bulk_supplier
            .take()
            .expect("the wrapped weight has a supplier");
        supplier.set_top_level_scoring_clause()?;
        supplier.bulk_scorer()
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// Port of `TestLRUQueryCache.testPropagateBulkScorer`.
fn test_propagate_bulk_scorer() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let leaf = Arc::clone(&searcher.get_leaf_contexts()[0]);
    let scorer_called = Arc::new(AtomicBool::new(false));
    let bulk_scorer_called = Arc::new(AtomicBool::new(false));
    let cache = lru(1, i64::MAX, |_| true, f32::INFINITY)?;

    // test that the bulk scorer is propagated when a scorer should not be
    // cached
    let mut weight = searcher.create_weight(
        Arc::new(MatchAllDocsQuery::instance()),
        ScoreMode::COMPLETE_NO_SCORES,
        1.0,
    )?;
    weight = Arc::new(WeightWrapper::new(
        weight,
        Arc::clone(&scorer_called),
        Arc::clone(&bulk_scorer_called),
    ));
    weight = cache.do_cache(weight, never_cache());
    weight.bulk_scorer(&leaf)?;
    Assert::assert_true(bulk_scorer_called.load(Ordering::SeqCst));
    Assert::assert_false(scorer_called.load(Ordering::SeqCst));
    assert_equals(&0, &cache.get_cache_count());

    searcher.get_index_reader().close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testEvictEmptySegmentCache, testMinSegmentSizePredicate
// -----------------------------------------------------------------------------

/// The anonymous `LRUQueryCache` subclass of `testEvictEmptySegmentCache`.
#[derive(Debug)]
struct NonEmptyEvictions;

impl LRUQueryCacheOverrides for NonEmptyEvictions {
    fn on_doc_id_set_eviction(
        &self,
        cache: &LRUQueryCache,
        reader_core_key: &CacheKey,
        num_entries: i32,
        sum_ram_bytes_used: i64,
    ) {
        cache.super_on_doc_id_set_eviction(reader_core_key, num_entries, sum_ram_bytes_used);
        Assert::assert_true(num_entries > 0);
    }
}

/// Port of `TestLRUQueryCache.testEvictEmptySegmentCache`.
fn test_evict_empty_segment_cache() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query_cache = lru_with(
        2,
        100000,
        |_| true,
        f32::INFINITY,
        Arc::new(NonEmptyEvictions),
    )?;

    searcher.set_query_cache(as_cache(&query_cache));
    searcher.set_query_caching_policy(always_cache());

    let query: Arc<dyn Query> = Arc::new(DummyQuery::new());
    searcher.count(Arc::clone(&query))?;
    assert_equals(&keys(&[&query]), &cached(&query_cache));
    query_cache.clear_query(query);

    reader.close()?; // make sure this does not trigger eviction of segment caches with no entries
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testMinSegmentSizePredicate`.
fn test_min_segment_size_predicate() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::new();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let new_segment = |num_docs: i32| {
        let outcome: Result<()> = (|| {
            for _ in 0..num_docs {
                w.add_document(&Document::new())?;
            }
            w.flush()
        })();
        // throw new UncheckedIOException(e)
        unchecked(outcome);
    };
    new_segment(1);
    new_segment(4);
    new_segment(10);
    new_segment(35);
    let num_large_segments = RandomNumbers::random_int_between(&*random(), 2, 40);
    for _ in 0..num_large_segments {
        new_segment(RandomNumbers::random_int_between(&*random(), 50, 55));
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    let reader_leaves = leaves(&reader)?;
    for i in 0..3 {
        let predicate =
            min_segment_size_predicate(RandomNumbers::random_int_between(&*random(), 1, i32::MAX));
        Assert::assert_false(predicate(&reader_leaves[i]));
    }
    for i in 3..reader_leaves.len() {
        let leaf = &reader_leaves[i];
        let small =
            min_segment_size_predicate(RandomNumbers::random_int_between(&*random(), 60, i32::MAX));
        Assert::assert_false(small(leaf));
        let big = min_segment_size_predicate(RandomNumbers::random_int_between(&*random(), 10, 30));
        Assert::assert_true(big(leaf));
    }
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// DummyDirectoryReader: a reader whose sole purpose is to not be cacheable
// -----------------------------------------------------------------------------

/// The anonymous `SubReaderWrapper` of `DummyDirectoryReader`.
struct DummySubReaderWrapper;

impl SubReaderWrapper for DummySubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(NoCacheHelperLeafReader {
            inner: FilterLeafReader::new(reader)?,
        }))
    }
}

/// `DummyDirectoryReader.doWrapDirectoryReader` and its
/// `getReaderCacheHelper()`, which returns `null`.
struct DummyDirectoryReaderWrapper;

impl DirectoryReaderWrapper for DummyDirectoryReaderWrapper {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        dummy_directory_reader(inner)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "DummyDirectoryReader"
    }

    fn reader_cache_helper_is_null(&self) -> bool {
        true
    }
}

/// `new DummyDirectoryReader(in)`.
fn dummy_directory_reader(r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(DummySubReaderWrapper),
        Arc::new(DummyDirectoryReaderWrapper),
    )?))
}

/// The anonymous `FilterLeafReader` of `DummyDirectoryReader`, whose cache
/// helpers are `null`.
struct NoCacheHelperLeafReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for NoCacheHelperLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoCacheHelperLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for NoCacheHelperLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for NoCacheHelperLeafReader {
    fn core(&self) -> &rucene::index::IndexReaderCore {
        LeafReader::core(&self.inner)
    }
    fn term_vectors(&self) -> Result<Box<dyn rucene::index::TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }
    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }
    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }
    fn stored_fields(&self) -> Result<Box<dyn rucene::index::StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }
    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn terms(&self, field: &str) -> Result<Option<Box<dyn rucene::index::Terms>>> {
        self.inner.terms(field)
    }
    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }
    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }
    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }
    fn get_norm_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }
    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }
    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }
    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }
    fn get_field_infos(&self) -> rucene::index::FieldInfos {
        self.inner.get_field_infos()
    }
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        LeafReader::get_live_docs(&self.inner)
    }
    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn rucene::index::PointValues>>> {
        self.inner.get_point_values(field)
    }
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }
    fn get_meta_data(&self) -> rucene::index::LeafMetaData {
        self.inner.get_meta_data()
    }
}

/// Port of `TestLRUQueryCache.testReaderNotSuitedForCaching`.
fn test_reader_not_suited_for_caching() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    let reader = dummy_directory_reader(w.get_reader()?)?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_caching_policy(always_cache());

    // don't cache if the reader does not expose a cache helper
    assert_null(
        &leaves(&reader)?[0]
            .leaf_reader()
            .get_core_cache_helper()
            .map(|_| "CacheHelper"),
    );
    let cache = lru(2, 10000, |_| true, f32::INFINITY)?;
    searcher.set_query_cache(as_cache(&cache));
    assert_equals(&0, &searcher.count(Arc::new(DummyQuery::new()))?);
    assert_equals(&0, &cache.get_cache_count());
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testSkipCacheIfOngoing
// -----------------------------------------------------------------------------

/// The `DocIdSetIterator` `TrackingScorer.iterator()` returns, which counts
/// the documents it visits.
struct TrackingIterator {
    scorer: Box<dyn Scorer>,
    /// `disi.cost()`, read when the iterator is built: it is stable.
    cost: i64,
    visited: Arc<AtomicI32>,
    num_docs: i32,
}

impl DocIdSetIterator for TrackingIterator {
    fn doc_id(&self) -> i32 {
        self.scorer.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.scorer.iterator().next_doc()?;
        if doc_id < self.num_docs {
            self.visited.fetch_add(1, Ordering::SeqCst);
        }
        Ok(doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc_id = self.scorer.iterator().advance(target)?;
        if doc_id < self.num_docs {
            self.visited.fetch_add(1, Ordering::SeqCst);
        }
        Ok(doc_id)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// The local `class TrackingScorer extends Scorer` of
/// `testSkipCacheIfOngoing`.
///
/// Java's `iterator()` returns a new counting view over the wrapped scorer's
/// iterator on every call; the counting view here holds the wrapped scorer
/// and is the one iterator this scorer hands out, which counts the same
/// documents.
struct TrackingScorer {
    disi: TrackingIterator,
}

impl Scorable for TrackingScorer {
    fn score(&mut self) -> Result<f32> {
        self.disi.scorer.score()
    }
}

impl Scorer for TrackingScorer {
    fn doc_id(&self) -> i32 {
        self.disi.scorer.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.disi
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.disi.scorer.get_max_score(up_to)
    }
}

/// The anonymous `ConstantScoreWeight` of `testSkipCacheIfOngoing`.
#[derive(Debug)]
struct TrackingWeight {
    visited: Arc<AtomicI32>,
    num_docs: i32,
}

impl ConstantScoreWeightImpl for TrackingWeight {
    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
        _score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(Some(Box::new(TrackingScorerSupplier {
            visited: Arc::clone(&self.visited),
            num_docs: self.num_docs,
        })))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        Ok(context.leaf_reader().num_docs())
    }
}

/// The anonymous `ScorerSupplier` of the weight of `testSkipCacheIfOngoing`.
struct TrackingScorerSupplier {
    visited: Arc<AtomicI32>,
    num_docs: i32,
}

impl ScorerSupplier for TrackingScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let mut scorer = ConstantScoreScorerSupplier::match_all(
            0.0,
            ScoreMode::COMPLETE_NO_SCORES,
            self.num_docs,
        )?
        .get(lead_cost)?;
        Ok(Box::new(TrackingScorer {
            disi: TrackingIterator {
                cost: scorer.iterator().cost(),
                scorer,
                visited: Arc::clone(&self.visited),
                num_docs: self.num_docs,
            },
        }))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        Ok(Box::new(TrackingBulkScorer {
            visited: Arc::clone(&self.visited),
            num_docs: self.num_docs,
        }))
    }

    fn cost(&self) -> i64 {
        0
    }
}

/// The anonymous `BulkScorer` of `testSkipCacheIfOngoing`.
struct TrackingBulkScorer {
    visited: Arc<AtomicI32>,
    num_docs: i32,
}

impl BulkScorer for TrackingBulkScorer {
    fn score(
        &mut self,
        _collector: &mut dyn LeafCollector,
        _accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        for doc in min..max {
            if doc < self.num_docs {
                self.visited.fetch_add(1, Ordering::SeqCst);
            } else {
                return Ok(NO_MORE_DOCS);
            }
        }
        Ok(max)
    }

    fn cost(&self) -> i64 {
        0
    }
}

/// The anonymous `LRUQueryCache` subclass of `testSkipCacheIfOngoing`.
#[derive(Debug)]
struct SkipIfOngoing {
    latch: Arc<CountDownLatch>,
    populated: AtomicBool,
}

impl LRUQueryCacheOverrides for SkipIfOngoing {
    fn try_populate_cache(
        &self,
        cache: &LRUQueryCache,
        cache_key: &dyn CacheHelper,
        weight: &Arc<dyn Weight>,
        scorer_supplier: &mut dyn ScorerSupplier,
        max_doc: i32,
    ) -> Result<Option<CacheAndCount>> {
        self.latch.count_down();
        Assert::assert_true(self.latch.await_timeout(Duration::from_secs(5)));
        if self
            .populated
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            cache.super_try_populate_cache(cache_key, weight, scorer_supplier, max_doc)
        } else {
            Ok(None)
        }
    }
}

/// The anonymous `LeafCollector` of `testSkipCacheIfOngoing`.
struct NoOpLeafCollector;

impl LeafCollector for NoOpLeafCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

/// Tests that we can override `LRUQueryCache#tryPopulateCache` to skip caching
/// when multiple threads concurrently attempt to cache the same segment for
/// the same query.
///
/// Port of `TestLRUQueryCache.testSkipCacheIfOngoing`.
fn test_skip_cache_if_ongoing() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::new();
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let num_threads = RandomNumbers::random_int_between(&*random(), 2, 6);
    let num_docs = num_threads * 100;
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(string_field("doc", &i.to_string())?));
        w.add_document(&doc)?;
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    let reader_leaves = leaves(&reader)?;
    assert_has_size(reader_leaves.len(), 1);
    let single_leaf = Arc::clone(&reader_leaves[0]);

    let visited = Arc::new(AtomicI32::new(0));

    let weight: Arc<dyn Weight> = Arc::new(ConstantScoreWeight::new(
        Arc::new(MatchAllDocsQuery::instance()),
        1.0,
        TrackingWeight {
            visited: Arc::clone(&visited),
            num_docs,
        },
    ));

    let latch = Arc::new(CountDownLatch::new(num_threads as i64));
    let cache = lru_with(
        2,
        10000,
        |_| true,
        f32::INFINITY,
        Arc::new(SkipIfOngoing {
            latch: Arc::clone(&latch),
            populated: AtomicBool::new(false),
        }),
    )?;
    let caching_weight = cache.do_cache(weight, always_cache());
    let mut threads = Vec::with_capacity(num_threads as usize);
    for t in 0..num_threads {
        let from = t * 100;
        let to = (from + 100).min(num_docs);
        let caching_weight = Arc::clone(&caching_weight);
        let single_leaf = Arc::clone(&single_leaf);
        let thread = Thread::unnamed(move || {
            let outcome = try_catch("java.lang.Exception", || {
                let mut bulk_scorer = unchecked(
                    unchecked(caching_weight.scorer_supplier(&single_leaf))
                        .expect("the weight has a supplier")
                        .bulk_scorer(),
                );
                bulk_scorer
                    .score(&mut NoOpLeafCollector, None, from, to)
                    .unwrap_or_else(|e| rethrow_java(e.into_lucene_error()));
            });
            if let Err(e) = outcome {
                Throwable::from_cause("java.lang.AssertionError", e).throw();
            }
        });
        thread.start();
        threads.push(thread);
    }
    for thread in &threads {
        let _ = thread.join();
    }
    LuceneTestCase::assert_that(
        &visited.load(Ordering::SeqCst),
        &less_than_or_equal_to(num_threads * 100 + num_docs),
    );
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testQueryNotSuitedForCaching
// -----------------------------------------------------------------------------

/// A query that returns null from Weight.getCacheHelper.
///
/// `private static class NoCacheQuery extends Query`.
#[derive(Debug, Clone)]
struct NoCacheQuery;

/// The anonymous `Weight` of `NoCacheQuery.createWeight`.
#[derive(Debug)]
struct NoCacheWeight;

impl SegmentCacheable for NoCacheWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for NoCacheWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::new(NoCacheQuery)
    }

    /// Java returns `null`; the port's `explain` returns a value, and no
    /// test calls it.
    fn explain(&self, _context: &LeafReaderContext, _doc: i32) -> Result<Explanation> {
        Ok(Explanation::no_match("NoCacheQuery", Vec::new()))
    }

    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(None)
    }
}

impl Query for NoCacheQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(NoCacheWeight))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        "NoCacheQuery".to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
    }

    fn query_hash(&self) -> u64 {
        0
    }
}

/// Port of `TestLRUQueryCache.testQueryNotSuitedForCaching`.
fn test_query_not_suited_for_caching() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_caching_policy(always_cache());

    let cache = lru(2, 10000, |_| true, f32::INFINITY)?;
    searcher.set_query_cache(as_cache(&cache));

    assert_equals(&0, &searcher.count(Arc::new(NoCacheQuery))?);
    assert_equals(&0, &cache.get_cache_count());

    // BooleanQuery wrapping an uncacheable query should also not be cached
    let bq = BooleanQueryBuilder::new()
        .add(Arc::new(NoCacheQuery), Occur::MUST)?
        .add(Arc::new(TermQuery::new(term("field", "term"))), Occur::MUST)?
        .build();
    assert_equals(&0, &searcher.count(Arc::new(bq))?);
    assert_equals(&0, &cache.get_cache_count());

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testPropagatesScorerSupplier
// -----------------------------------------------------------------------------

/// `private static class DummyQuery2 extends Query`.
#[derive(Debug, Clone)]
struct DummyQuery2 {
    scorer_created: Arc<AtomicBool>,
}

/// The anonymous `ConstantScoreWeight` of `DummyQuery2.createWeight`.
#[derive(Debug)]
struct DummyQuery2Weight {
    scorer_created: Arc<AtomicBool>,
    boost: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for DummyQuery2Weight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }

    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
        _score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let scorer = ConstantScoreScorer::from_iterator(
            self.boost,
            self.score_mode,
            Box::new(doc_id_set_iterator::all(1)?),
        );
        Ok(Some(Box::new(DummyQuery2Supplier {
            scorer: Some(Box::new(scorer)),
            scorer_created: Arc::clone(&self.scorer_created),
        })))
    }
}

/// The anonymous `ScorerSupplier` of `DummyQuery2`'s weight.
struct DummyQuery2Supplier {
    scorer: Option<Box<dyn Scorer>>,
    scorer_created: Arc<AtomicBool>,
}

impl ScorerSupplier for DummyQuery2Supplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        self.scorer_created.store(true, Ordering::SeqCst);
        Ok(self.scorer.take().expect("get is called once"))
    }

    fn cost(&self) -> i64 {
        1
    }
}

impl Query for DummyQuery2 {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            DummyQuery2Weight {
                scorer_created: Arc::clone(&self.scorer_created),
                boost,
                score_mode,
            },
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
    }

    fn query_hash(&self) -> u64 {
        0
    }

    fn to_query_string(&self, _field: &str) -> String {
        "DummyQuery2".to_string()
    }
}

/// Port of `TestLRUQueryCache.testPropagatesScorerSupplier`.
fn test_propagates_scorer_supplier() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_caching_policy(never_cache());

    let cache = Arc::new(LRUQueryCache::new(1, 1000)?);
    searcher.set_query_cache(as_cache(&cache));

    let scorer_created = Arc::new(AtomicBool::new(false));
    let query: Arc<dyn Query> = Arc::new(DummyQuery2 {
        scorer_created: Arc::clone(&scorer_created),
    });
    let weight =
        searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
    let mut supplier = weight
        .scorer_supplier(&searcher.get_leaf_contexts()[0])?
        .expect("the weight has a supplier");
    Assert::assert_false(scorer_created.load(Ordering::SeqCst));
    supplier.get(random().next_long() & 0x7FFFFFFFFFFFFFFF)?;
    Assert::assert_true(scorer_created.load(Ordering::SeqCst));

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// testDocValuesUpdatesDontBreakCache
// -----------------------------------------------------------------------------

/// `static class DVCacheQuery extends Query`.
#[derive(Debug, Clone)]
struct DVCacheQuery {
    field: String,
    scorer_created_count: Arc<AtomicI32>,
}

impl DVCacheQuery {
    fn new(field: &str) -> DVCacheQuery {
        DVCacheQuery {
            field: field.to_string(),
            scorer_created_count: Arc::new(AtomicI32::new(0)),
        }
    }
}

/// The anonymous `ConstantScoreWeight` of `DVCacheQuery.createWeight`.
#[derive(Debug)]
struct DVCacheWeight {
    field: String,
    scorer_created_count: Arc<AtomicI32>,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for DVCacheWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        _score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(Some(Box::new(DVCacheSupplier {
            scorer_created_count: Arc::clone(&self.scorer_created_count),
            score_mode: self.score_mode,
            max_doc: context.leaf_reader().max_doc(),
        })))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        DocValues::is_cacheable(ctx, &[&self.field])
    }
}

/// The anonymous `ScorerSupplier` of `DVCacheQuery`'s weight.
struct DVCacheSupplier {
    scorer_created_count: Arc<AtomicI32>,
    score_mode: ScoreMode,
    max_doc: i32,
}

impl ScorerSupplier for DVCacheSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        self.scorer_created_count.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(ConstantScoreScorer::from_iterator(
            1.0,
            self.score_mode,
            Box::new(doc_id_set_iterator::all(self.max_doc)?),
        )))
    }

    fn cost(&self) -> i64 {
        self.max_doc as i64
    }
}

impl Query for DVCacheQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "DVCacheQuery".to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
    }

    fn query_hash(&self) -> u64 {
        0
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            1.0,
            DVCacheWeight {
                field: self.field.clone(),
                scorer_created_count: Arc::clone(&self.scorer_created_count),
                score_mode,
            },
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}
}

/// `new AssertingIndexSearcher(random(), reader)` with the cache and
/// `ALWAYS_CACHE`.
fn asserting_caching_searcher(
    reader: &Arc<dyn DirectoryReader>,
    cache: &Arc<LRUQueryCache>,
) -> Result<IndexSearcher> {
    // no newSearcher(reader) - see comment in the test
    let mut searcher =
        AssertingIndexSearcher::new(&*random(), Arc::clone(reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_caching_policy(always_cache());
    searcher.set_query_cache(as_cache(cache));
    Ok(searcher)
}

/// Port of `TestLRUQueryCache.testDocValuesUpdatesDontBreakCache`.
fn test_doc_values_updates_dont_break_cache() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    w.commit()?;
    let mut reader = directory_reader::open_with_writer(&*w)?;

    // IMPORTANT:
    // Don't use newSearcher(), because that will sometimes use an
    // ExecutorService, and we need to be single threaded to ensure that
    // LRUQueryCache doesn't skip the cache due to thread contention
    let mut searcher =
        AssertingIndexSearcher::new(&*random(), Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_caching_policy(always_cache());

    let cache = lru(1, 10000, |_| true, f32::INFINITY)?;
    searcher.set_query_cache(as_cache(&cache));

    let query = DVCacheQuery::new("field");
    let count = |q: &DVCacheQuery| q.scorer_created_count.load(Ordering::SeqCst);
    assert_equals(&1, &searcher.count(Arc::new(query.clone()))?);
    assert_equals(&1, &count(&query));
    assert_equals(&1, &searcher.count(Arc::new(query.clone()))?);
    assert_equals(&1, &count(&query)); // should be cached

    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("field", 1)));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "text",
        "text",
        Store::NO,
    )));
    w.add_document(&doc)?;
    reader.close()?;
    reader = directory_reader::open_with_writer(&*w)?;
    searcher = asserting_caching_searcher(&reader, &cache)?;

    assert_equals(&2, &searcher.count(Arc::new(query.clone()))?);
    assert_equals(&2, &count(&query)); // first segment cached

    reader.close()?;
    reader = directory_reader::open_with_writer(&*w)?;
    searcher = asserting_caching_searcher(&reader, &cache)?;

    assert_equals(&2, &searcher.count(Arc::new(query.clone()))?);
    assert_equals(&2, &count(&query)); // both segments cached

    w.update_numeric_doc_value(term("text", "text"), "field", 2)?;
    reader.close()?;
    reader = directory_reader::open_with_writer(&*w)?;
    searcher = asserting_caching_searcher(&reader, &cache)?;

    assert_equals(&2, &searcher.count(Arc::new(query.clone()))?);
    assert_equals(&3, &count(&query)); // second segment no longer cached due to DV update

    assert_equals(&2, &searcher.count(Arc::new(query.clone()))?);
    assert_equals(&4, &count(&query)); // still no caching

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testQueryCacheSoftUpdate`.
fn test_query_cache_soft_update() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_soft_deletes_field(Some("soft_delete".to_string()));
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let query_cache = lru(10, 1000 * 1000, |_| true, f32::INFINITY)?;
    IndexSearcher::set_default_query_cache(as_cache(&query_cache));
    IndexSearcher::set_default_query_caching_policy(Some(always_cache()));

    let sm = SearcherManager::from_writer(
        &*w,
        true,
        false,
        Some(Arc::new(DefaultSearcherFactory)),
        None,
    )?;

    w.add_document(&stored_doc(&[("id", "1")])?)?;

    w.add_document(&stored_doc(&[("id", "2")])?)?;

    sm.maybe_refresh_blocking()?;

    let searcher = sm.acquire()?;
    let query = BooleanQueryBuilder::new()
        .add(Arc::new(TermQuery::new(term("id", "1"))), Occur::FILTER)?
        .build();
    searcher.searcher().search(Arc::new(query), 10)?;
    assert_equals(&1, &query_cache.get_cache_size());
    assert_equals(&0, &query_cache.get_eviction_count());

    let soft_delete = true;
    if soft_delete {
        let mut tombstone = Document::new();
        tombstone.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
        w.soft_update_document(term("id", "1"), &tombstone, &[ndv_field("soft_delete", 1)?])?;
        w.soft_update_document(term("id", "2"), &tombstone, &[ndv_field("soft_delete", 1)?])?;
    } else {
        w.delete_documents_by_terms(vec![term("id", "1")])?;
        w.delete_documents_by_terms(vec![term("id", "2")])?;
    }
    sm.maybe_refresh_blocking()?;
    // All docs in the first segment are deleted - we should drop it with the
    // default merge policy.
    sm.release(searcher)?;
    assert_equals(&0, &query_cache.get_cache_size());
    assert_equals(&1, &query_cache.get_eviction_count());
    sm.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testBulkScorerLocking`.
fn test_bulk_scorer_locking() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    // the test framework sometimes sets crazy low values, prevent this since
    // we are indexing many docs
    iwc.set_max_buffered_docs(-1)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let num_docs = LuceneTestCase::at_least(10);
    let empty_doc = Document::new();
    for _ in 0..num_docs {
        let mut i = random().next_int_bound(5000);
        while i >= 0 {
            w.add_document(&empty_doc)?;
            i -= 1;
        }
        let mut doc = Document::new();
        for value in ["foo", "bar", "baz"] {
            if random().next_boolean() {
                doc.add(Box::new(string_field("field", value)?));
            }
        }
    }
    let mut i = TestUtil::next_int(&*random(), 3000, 5000);
    while i >= 0 {
        w.add_document(&empty_doc)?;
        i -= 1;
    }
    if random().next_boolean() {
        w.force_merge(1)?;
    }

    let reader = directory_reader::open_with_writer(&*w)?;
    let no_cache_reader = dummy_directory_reader(Arc::clone(&reader))?;

    let cache = lru(1, 100000, |_| true, f32::INFINITY)?;
    let mut searcher =
        AssertingIndexSearcher::new(&*random(), Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(as_cache(&cache));
    searcher.set_query_caching_policy(always_cache());

    let boosted = |value: &str| -> Result<Arc<dyn Query>> {
        Ok(Arc::new(BoostQuery::new(
            Arc::new(TermQuery::new(term("field", value))),
            3.0,
        )?))
    };
    let query: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(Arc::new(
        BooleanQueryBuilder::new()
            .add(boosted("foo")?, Occur::SHOULD)?
            .add(boosted("bar")?, Occur::SHOULD)?
            .add(boosted("baz")?, Occur::SHOULD)?
            .build(),
    )));

    searcher.search(Arc::clone(&query), 1)?;

    let mut no_cache_helper_searcher = AssertingIndexSearcher::new(
        &*random(),
        Arc::clone(&no_cache_reader) as Arc<dyn IndexReader>,
    )?;
    no_cache_helper_searcher.set_query_cache(as_cache(&cache));
    no_cache_helper_searcher.set_query_caching_policy(always_cache());
    no_cache_helper_searcher.search(Arc::clone(&query), 1)?;

    drop(searcher);
    drop(no_cache_helper_searcher);
    let t = Thread::unnamed(move || {
        let outcome: Result<()> = (|| {
            no_cache_reader.close()?;
            w.close()?;
            dir.close()?;
            Ok(())
        })();
        if let Err(e) = outcome {
            Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw();
        }
    });
    t.start();
    let _ = t.join();
    Ok(())
}

// -----------------------------------------------------------------------------
// Skip-cache factor
// -----------------------------------------------------------------------------

/// Port of `TestLRUQueryCache.testSkipCachingForRangeQuery`.
fn test_skip_caching_for_range_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let person = |name: &str, age: i64| -> Result<Document> {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "name",
            name.to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(LongPoint::new("age", &[age])?));
        doc.add(Box::new(SortedNumericDocValuesField::new("age", age)));
        Ok(doc)
    };
    let doc1 = person("tom", 15)?;
    let doc2 = person("alice", 20)?;
    let doc3 = person("steve", 25)?;
    w.add_documents(&[doc1, doc2, doc3])?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_caching_policy(always_cache());
    w.close()?;

    // lead cost is 1, cost of subQuery1 is 1, cost of subQuery2 is 2
    let mut bq = BooleanQueryBuilder::new();
    let sub_query1: Arc<dyn Query> = Arc::new(TermQuery::new(term("name", "tom")));
    let sub_query2: Arc<dyn Query> = Arc::new(IndexOrDocValuesQuery::new(
        LongPoint::new_range_query("age", &[10], &[20])?,
        SortedNumericDocValuesField::new_slow_range_query("age", 10, 20),
    ));
    let query: Arc<dyn Query> = Arc::new(
        bq.add(Arc::clone(&sub_query1), Occur::FILTER)?
            .add(Arc::clone(&sub_query2), Occur::FILTER)?
            .build(),
    );
    let mut cache_set: HashSet<QueryKey> = HashSet::new();

    // only term query is cached
    let part_cache = lru(1000000, 10000000, |_| true, 1.0)?;
    searcher.set_query_cache(as_cache(&part_cache));
    searcher.search(Arc::clone(&query), 1)?;
    cache_set.insert(QueryKey::new(Arc::clone(&sub_query1)));
    assert_equals(&cache_set, &cached_set(&part_cache));

    // both queries are cached
    let all_cache = lru(1000000, 10000000, |_| true, f32::INFINITY)?;
    searcher.set_query_cache(as_cache(&all_cache));
    searcher.search(Arc::clone(&query), 1)?;
    cache_set.insert(QueryKey::new(Arc::clone(&sub_query2)));
    assert_equals(&cache_set, &cached_set(&all_cache));

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testCountDelegation`.
fn test_count_delegation() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(string_field("foo", "bar")?));
    let num_docs = random().next_int_bound(100) + 20;
    for _ in 0..num_docs {
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_caching_policy(always_cache());

    let q: Arc<dyn Query> = Arc::new(TermQuery::new(term("foo", "bar")));
    searcher.count(Arc::clone(&q))?; // add to cache

    let weight = searcher.create_weight(
        searcher.rewrite(Arc::clone(&q))?,
        ScoreMode::COMPLETE_NO_SCORES,
        1.0,
    )?;
    assert_not_equals(&-1, &weight.count(&searcher.get_leaf_contexts()[0])?);

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `QueryCachingPolicy` of `testSkipCachingForTermQuery` and
/// `testSkipCacheFactorWithDynamicValues`: everything but term queries.
#[derive(Debug)]
struct NoTermQueryPolicy;

impl QueryCachingPolicy for NoTermQueryPolicy {
    fn should_cache(&self, query: &dyn Query) -> Result<bool> {
        Ok(query.as_any().downcast_ref::<TermQuery>().is_none())
    }

    fn on_use(&self, _query: &dyn Query) {
        // no-op
    }
}

/// The index and query of `testSkipCachingForTermQuery` and
/// `testSkipCacheFactorWithDynamicValues`: the reader, its searcher, the
/// query and its first sub-query.
fn hobby_index(
    dir: &mut rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<(
    Arc<dyn DirectoryReader>,
    IndexSearcher,
    Arc<dyn Query>,
    Arc<dyn Query>,
)> {
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let doc1 = stored_doc(&[("name", "tom"), ("hobby", "movie")])?;
    let doc2 = stored_doc(&[("name", "alice"), ("hobby", "book")])?;
    let doc3 = stored_doc(&[("name", "alice"), ("hobby", "movie")])?;
    w.add_documents(&[doc1, doc2, doc3])?;
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_caching_policy(Arc::new(NoTermQueryPolicy));
    w.close()?;

    // lead cost is 2, cost of subQuery1 is 3, cost of subQuery2 is 2
    let mut inner = BooleanQueryBuilder::new();
    let inner_sub_query1: Arc<dyn Query> = Arc::new(TermQuery::new(term("hobby", "book")));
    let inner_sub_query2: Arc<dyn Query> = Arc::new(TermQuery::new(term("hobby", "movie")));
    let sub_query1: Arc<dyn Query> = Arc::new(
        inner
            .add(inner_sub_query1, Occur::SHOULD)?
            .add(inner_sub_query2, Occur::SHOULD)?
            .build(),
    );

    let mut bq = BooleanQueryBuilder::new();
    let sub_query2: Arc<dyn Query> = Arc::new(TermQuery::new(term("name", "alice")));
    let query: Arc<dyn Query> = Arc::new(
        bq.add(
            Arc::new(ConstantScoreQuery::new(Arc::clone(&sub_query1))),
            Occur::FILTER,
        )?
        .add(sub_query2, Occur::FILTER)?
        .build(),
    );
    Ok((reader, searcher, query, sub_query1))
}

/// Port of `TestLRUQueryCache.testSkipCachingForTermQuery`.
fn test_skip_caching_for_term_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (reader, mut searcher, query, sub_query1) = hobby_index(&mut dir)?;
    let mut cache_set: HashSet<QueryKey> = HashSet::new();

    // both queries are not cached
    let part_cache = lru(1000000, 10000000, |_| true, 1.0)?;
    searcher.set_query_cache(as_cache(&part_cache));
    searcher.search(Arc::clone(&query), 1)?;
    assert_equals(&cache_set, &cached_set(&part_cache));

    // only subQuery1 is cached
    let all_cache = lru(1000000, 10000000, |_| true, f32::INFINITY)?;
    searcher.set_query_cache(as_cache(&all_cache));
    searcher.search(Arc::clone(&query), 1)?;
    cache_set.insert(QueryKey::new(sub_query1));
    assert_equals(&cache_set, &cached_set(&all_cache));

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testCacheHasFastCount`.
fn test_cache_has_fast_count() -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_bytes(
        "words",
        &[
            BytesRef::from_char_sequence("alice"),
            BytesRef::from_char_sequence("ran"),
        ],
    )?);

    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let words_doc = |words: &str, f: Option<&str>| -> Result<Document> {
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new(
            "words",
            words.to_string(),
            Store::NO,
        )?));
        if let Some(f) = f {
            doc.add(Box::new(string_field("f", f)?));
        }
        Ok(doc)
    };
    let doc1 = words_doc("tom ran", None)?;
    let doc2 = words_doc("alice ran", Some("a"))?;
    let doc3 = words_doc("alice ran", Some("b"))?;
    w.add_documents(&[doc1, doc2, doc3])?;

    let fast_count = |expected_after: i32| -> Result<()> {
        let reader = w.get_reader()?;
        let outcome: Result<()> = (|| {
            let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            searcher.set_query_caching_policy(always_cache());
            let all_cache = lru(1000000, 10000000, |_| true, f32::INFINITY)?;
            searcher.set_query_cache(as_cache(&all_cache));
            let weight =
                searcher.create_weight(Arc::clone(&query), ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let context = Arc::clone(&leaves(&reader)?[0]);
            // We don't have a fast count before the cache is filled
            assert_equals(&-1, &weight.count(&context)?);
            // Fetch the scorer to populate the cache
            weight.scorer(&context)?;
            assert_equals(&keys(&[&query]), &cached(&all_cache));
            // Now we *do* have a fast count (first block); we still don't have
            // a fast count because we have deleted documents (second block)
            assert_equals(&expected_after, &weight.count(&context)?);
            Ok(())
        })();
        reader.close()?;
        outcome
    };
    fast_count(2)?;

    w.delete_documents_by_query(Arc::new(TermQuery::new(Term::new(
        "f",
        BytesRef::from_char_sequence("b"),
    ))))?;
    fast_count(-1)?;

    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestLRUQueryCache.testSkipCacheFactorWithDynamicValues`.
fn test_skip_cache_factor_with_dynamic_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (reader, mut searcher, query, sub_query1) = hobby_index(&mut dir)?;
    let cache_set: HashSet<QueryKey> = HashSet::new();

    // Define skip cache factor to be 1, so that query is not cached.
    let cache = lru(1000000, 10000000, |_| true, 1.0)?;
    searcher.set_query_cache(as_cache(&cache));
    searcher.search(Arc::clone(&query), 1)?;
    assert_equals(&cache_set, &cached_set(&cache));
    assert_equals_delta(1.0, cache.get_skip_cache_factor() as f64, 0.0);

    // Now change the skipCacheFactor to 3.0f, now same query should get cached.
    cache.set_skip_cache_factor(3.0);
    searcher.search(Arc::clone(&query), 1)?;
    assert_equals(&key_set(&[&sub_query1]), &cached_set(&cache));
    assert_equals_delta(3.0, cache.get_skip_cache_factor() as f64, 0.0);

    reader.close()?;
    dir.close()?;
    Ok(())
}
