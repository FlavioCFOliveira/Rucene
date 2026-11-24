/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.facet.TestDrillSideways` and of its subclass
//! `org.apache.lucene.facet.TestParallelDrillSideways`.
//!
//! The instance of both classes is a [`Fixture`]: `TestParallelDrillSideways`
//! inherits the 14 test methods and overrides the four protected
//! `getNewDrillSideways*` factories to pass its static executor, which the
//! fixture's `parallel` flag selects (Java's virtual dispatch on `this`).
//!
//! Anonymous-class names are those `javac` assigns in source order
//! (`TestDrillSideways$6` is the `TermQuery` subclass of
//! `testBasicWithCollectorManager`).

use std::any::Any;
use std::cmp::Ordering as CmpOrdering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};
use std::time::Duration;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::document::{
    Document, SortedDocValuesField, SortedSetDocValuesField, Store, StringField, TextField,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::{
    directory_reader, DirectoryReader, DocValues, IndexReader, LeafReader, LeafReaderContext,
    SortedDocValues, Term, TwoPhaseCommit,
};
use rucene::search::boolean_clause::Occur;
use rucene::search::collection_terminated_exception::{CollectionError, CollectionResult};
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::doc_id_set_iterator::{self, AllDocIdSetIterator};
use rucene::search::index_searcher::{
    IndexSearcher, IndexSearcherOverrides, LeafReaderContextPartition,
};
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::query_cache::QueryCachingPolicy;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::task_executor::Executor;
use rucene::search::term_query::TermQuery;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::BooleanQueryBuilder;
use rucene::search::{
    BulkScorer, Collector, CollectorManager, DocIdSetIterator, LeafCollector, MatchAllDocsQuery,
    MatchNoDocsQuery, QueryVisitor, Scorable, ScoreMode, ScorerSupplier, NO_MORE_DOCS,
};
use rucene::util::concurrent::NamedThreadFactory;
use rucene::util::java_collections::{JavaHashMap, JavaHashSet};
use rucene::util::selector::PivotOps;
use rucene::util::sorter::{InPlaceMergeSorter, Sorter};
use rucene::util::{BytesRef, NoOutputInfoStream};
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::drill_sideways::{DrillSideways, DrillSidewaysOverrides, DrillSidewaysResult};
use rucene_facet::drill_sideways_scorer::{DocsAndCost, DrillSidewaysScorer};
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::{FacetsCollector, FacetsCollectorOverrides};
use rucene_facet::facets_collector_manager::{
    FacetsCollectorManager, FacetsCollectorManagerOverrides, FacetsTopDocs,
};
use rucene_facet::facets_config::{DrillDownTermsIndexing, FacetsConfig};
use rucene_facet::label_and_value::number_int_value;
use rucene_facet::multi_facets::MultiFacets;
use rucene_facet::sortedset::default_sorted_set_doc_values_reader_state::DefaultSortedSetDocValuesReaderState;
use rucene_facet::sortedset::sorted_set_doc_values_facet_field::SortedSetDocValuesFacetField;
use rucene_facet::sortedset::sorted_set_doc_values_reader_state::SortedSetDocValuesReaderState;
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::index::threaded_indexing_and_searching_test_case::CachedThreadPoolExecutor;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::search::{AssertingCollector, CollectorRef};
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::lucene_test_case::{Concurrency, TestDirectory, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::facet_support::core_support::{assert_equals, assert_equals_msg};
use crate::facet_support::search_support::{ftest, static_hook};
use crate::facet_support::{facet_fixture_class, get_taxonomy_facet_counts};

const IAE: &str = "java.lang.IllegalArgumentException";

/// The instance of `TestDrillSideways` (and of `TestParallelDrillSideways`):
/// its fields `aChance`, `bChance`, `cChance`, and whether `this` is a
/// `TestParallelDrillSideways`.
#[derive(Default)]
pub struct Fixture {
    parallel: bool,
    a_chance: f64,
    b_chance: f64,
    c_chance: f64,
}

/// The 14 test methods, declared by `TestDrillSideways` and inherited by
/// `TestParallelDrillSideways`.
fn test_methods() -> Vec<Method> {
    vec![
        ftest("testNoCaching", test_no_caching),
        ftest("testCollectionTerminated", test_collection_terminated),
        ftest("testBasic", test_basic),
        ftest(
            "testLeafCollectorSingleFinishCall",
            test_leaf_collector_single_finish_call,
        ),
        ftest(
            "testBasicWithCollectorManager",
            test_basic_with_collector_manager,
        ),
        ftest(
            "testSometimesInvalidDrillDown",
            test_sometimes_invalid_drill_down,
        ),
        ftest("testMultipleRequestsPerDim", test_multiple_requests_per_dim),
        ftest("testRandom", test_random),
        ftest(
            "testFinishOnAllDimsNoHitsQuery",
            test_finish_on_all_dims_no_hits_query,
        ),
        ftest("testEmptyIndex", test_empty_index),
        ftest(
            "testNoDrillDownFacetCollection",
            test_no_drill_down_facet_collection,
        ),
        ftest("testScorer", test_scorer),
        ftest(
            "testExtendedDrillSidewaysResult",
            test_extended_drill_sideways_result,
        ),
        ftest(
            "testDrillSidewaysSearchUseCorrectIterator",
            test_drill_sideways_search_use_correct_iterator,
        )
        .annotate(randomizedtesting::junit::Test::default()),
    ]
}

/// `TestDrillSideways`.
pub fn class_def() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        facet_fixture_class::<Fixture>("org.apache.lucene.facet.TestDrillSideways", test_methods())
    });
    Arc::clone(&CLASS)
}

/// `TestParallelDrillSideways extends TestDrillSideways`.
pub fn parallel_class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.facet.TestParallelDrillSideways")
        .extends(&class_def())
        .constructor(Constructor::no_args(|| Fixture {
            parallel: true,
            ..Fixture::default()
        }))
        .method(static_hook(
            "prepareExecutor",
            BeforeClass,
            prepare_executor,
        ))
        .method(static_hook(
            "shutdownExecutor",
            AfterClass,
            shutdown_executor,
        ))
        .build()
}

// ---------------------------------------------------------------------------
// TestParallelDrillSideways statics
// ---------------------------------------------------------------------------

/// `private static ExecutorService executor`.
static EXECUTOR: Mutex<Option<Arc<CachedThreadPoolExecutor>>> = Mutex::new(None);

fn executor_lock() -> MutexGuard<'static, Option<Arc<CachedThreadPoolExecutor>>> {
    EXECUTOR.lock().unwrap_or_else(|e| e.into_inner())
}

fn executor() -> Option<Arc<dyn Executor>> {
    executor_lock()
        .as_ref()
        .map(|e| Arc::clone(e) as Arc<dyn Executor>)
}

fn prepare_executor() -> Result<()> {
    *executor_lock() = Some(Arc::new(CachedThreadPoolExecutor::new(
        "TestParallelDrillSideways",
    )));
    Ok(())
}

fn shutdown_executor() -> Result<()> {
    if let Some(executor) = executor_lock().take() {
        executor.shutdown();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The protected factories (overridden by TestParallelDrillSideways)
// ---------------------------------------------------------------------------

/// The anonymous `DrillSideways` subclass of
/// `getNewDrillSidewaysScoreSubdocsAtOnce`.
struct ScoreSubDocsAtOnce;

impl DrillSidewaysOverrides for ScoreSubDocsAtOnce {
    fn score_sub_docs_at_once(&self) -> bool {
        true
    }
}

/// The anonymous `DrillSideways` subclass of
/// `getNewDrillSidewaysBuildFacetsResult`, capturing its `taxoReader` and
/// `config` parameters.
struct BuildFacetsResult {
    taxo_reader: Arc<dyn TaxonomyReader>,
    config: Arc<FacetsConfig>,
}

impl DrillSidewaysOverrides for BuildFacetsResult {
    // The facet counts hold non-`Send` readers, shared as Java shares them.
    #[allow(clippy::arc_with_non_send_sync)]
    fn build_facets_result(
        &self,
        _drill_sideways: &DrillSideways,
        drill_downs: Option<&FacetsCollector>,
        drill_sideways: Option<&[FacetsCollector]>,
        drill_sideways_dims: Option<&[String]>,
    ) -> Result<Option<Arc<dyn Facets>>> {
        let mut drill_sideways_facets: JavaHashMap<Arc<dyn Facets>> = JavaHashMap::new();
        let drill_down_facets = get_taxonomy_facet_counts(
            &self.taxo_reader,
            &self.config,
            drill_downs.ok_or_else(LuceneError::null_pointer)?,
        )?;
        if let Some(drill_sideways) = drill_sideways {
            let dims = drill_sideways_dims.ok_or_else(LuceneError::null_pointer)?;
            for (i, collector) in drill_sideways.iter().enumerate() {
                drill_sideways_facets.insert(
                    dims[i].clone(),
                    get_taxonomy_facet_counts(&self.taxo_reader, &self.config, collector)?,
                );
            }
        }

        if drill_sideways_facets.is_empty() {
            Ok(Some(drill_down_facets))
        } else {
            let by_dim: Vec<(String, Arc<dyn Facets>)> = drill_sideways_facets
                .iter()
                .map(|(k, v)| (k.clone(), Arc::clone(v)))
                .collect();
            Ok(Some(Arc::new(MultiFacets::with_default(
                by_dim,
                Some(drill_down_facets),
            ))))
        }
    }
}

/// `getNewDrillSideways(IndexSearcher, FacetsConfig,
/// SortedSetDocValuesReaderState)`.
fn get_new_drill_sideways_state(
    f: &Fixture,
    searcher: &Arc<IndexSearcher>,
    config: &Arc<FacetsConfig>,
    state: Arc<dyn SortedSetDocValuesReaderState>,
) -> DrillSideways {
    if f.parallel {
        DrillSideways::with_all(
            Arc::clone(searcher),
            Arc::clone(config),
            None,
            Some(state),
            executor(),
        )
    } else {
        DrillSideways::with_state(Arc::clone(searcher), Arc::clone(config), state)
    }
}

/// `getNewDrillSideways(IndexSearcher, FacetsConfig, TaxonomyReader)`.
fn get_new_drill_sideways(
    f: &Fixture,
    searcher: &Arc<IndexSearcher>,
    config: &Arc<FacetsConfig>,
    taxo_reader: &Arc<dyn TaxonomyReader>,
) -> DrillSideways {
    if f.parallel {
        DrillSideways::with_all(
            Arc::clone(searcher),
            Arc::clone(config),
            Some(Arc::clone(taxo_reader)),
            None,
            executor(),
        )
    } else {
        DrillSideways::new(
            Arc::clone(searcher),
            Arc::clone(config),
            Arc::clone(taxo_reader),
        )
    }
}

/// `getNewDrillSidewaysScoreSubdocsAtOnce(IndexSearcher, FacetsConfig,
/// TaxonomyReader)`.
fn get_new_drill_sideways_score_subdocs_at_once(
    f: &Fixture,
    searcher: &Arc<IndexSearcher>,
    config: &Arc<FacetsConfig>,
    taxo_reader: &Arc<dyn TaxonomyReader>,
) -> DrillSideways {
    DrillSideways::with_overrides(
        Arc::clone(searcher),
        Arc::clone(config),
        Some(Arc::clone(taxo_reader)),
        None,
        if f.parallel { executor() } else { None },
        Arc::new(ScoreSubDocsAtOnce),
    )
}

/// `getNewDrillSidewaysBuildFacetsResult(IndexSearcher, FacetsConfig,
/// TaxonomyReader)`.
fn get_new_drill_sideways_build_facets_result(
    f: &Fixture,
    searcher: &Arc<IndexSearcher>,
    config: &Arc<FacetsConfig>,
    taxo_reader: &Arc<dyn TaxonomyReader>,
) -> DrillSideways {
    DrillSideways::with_overrides(
        Arc::clone(searcher),
        Arc::clone(config),
        Some(Arc::clone(taxo_reader)),
        None,
        if f.parallel { executor() } else { None },
        Arc::new(BuildFacetsResult {
            taxo_reader: Arc::clone(taxo_reader),
            config: Arc::clone(config),
        }),
    )
}

/// `getNewSearcher(IndexReader)`.
fn get_new_searcher(reader: &Arc<dyn DirectoryReader>) -> Arc<IndexSearcher> {
    // Do not wrap with an asserting searcher, since DrillSidewaysQuery doesn't
    // implement all the required components like Weight#scorer.
    let mut searcher = LuceneTestCase::new_searcher_concurrency(
        Arc::clone(reader),
        true,
        false,
        Concurrency::InterSegment,
    );
    // DrillSideways requires the entire range of docs to be scored at once, so it doesn't support
    // timeouts whose implementation scores one window of doc IDs at a time.
    searcher.set_timeout(None);
    Arc::new(searcher)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn random_writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())
}

fn create_taxo_writer(dir: &TestDirectory) -> Result<Arc<DirectoryTaxonomyWriter>> {
    Ok(Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        dir.as_directory(),
        OpenMode::CREATE,
    )?))
}

fn nrt_taxo_reader(tw: &Arc<DirectoryTaxonomyWriter>) -> Result<Arc<dyn TaxonomyReader>> {
    Ok(Arc::new(DirectoryTaxonomyReader::from_writer(Arc::clone(
        tw,
    ))?))
}

fn add_doc(
    writer: &RandomIndexWriter,
    config: &FacetsConfig,
    taxo_writer: &DirectoryTaxonomyWriter,
    doc: &Document,
) -> Result<()> {
    writer.add_document(&config.build_with_taxonomy(Some(taxo_writer), doc)?)?;
    Ok(())
}

/// `facets.getTopChildren(topN, dim, path...)`, `null` as `None`.
fn top(
    facets: &Option<Arc<dyn Facets>>,
    top_n: i32,
    dim: &str,
    path: &[&str],
) -> Result<Option<String>> {
    let facets = facets.as_ref().ok_or_else(LuceneError::null_pointer)?;
    Ok(facets
        .get_top_children(top_n, dim, path)?
        .map(|r| r.to_string()))
}

/// `facets.getTopChildren(topN, dim).toString()`.
fn top_string(facets: &Option<Arc<dyn Facets>>, top_n: i32, dim: &str) -> Result<String> {
    Ok(top(facets, top_n, dim, &[])?.expect("java.lang.NullPointerException"))
}

fn facets_of(facets: &Option<Arc<dyn Facets>>) -> &Arc<dyn Facets> {
    facets.as_ref().expect("java.lang.NullPointerException")
}

fn total_hits(r: &DrillSidewaysResult) -> i64 {
    r.hits
        .as_ref()
        .expect("java.lang.NullPointerException")
        .total_hits()
        .value()
}

/// `facets instanceof MultiFacets`.
fn is_multi_facets(facets: &Option<Arc<dyn Facets>>) -> bool {
    facets
        .as_ref()
        .and_then(|f| f.as_any())
        .is_some_and(|any| any.is::<MultiFacets>())
}

/// `Float.compare(float, float)`.
fn java_float_compare(a: f32, b: f32) -> CmpOrdering {
    let bits = |f: f32| -> i32 {
        if f.is_nan() {
            0x7fc0_0000
        } else {
            f.to_bits() as i32
        }
    };
    if a < b {
        CmpOrdering::Less
    } else if a > b {
        CmpOrdering::Greater
    } else {
        bits(a).cmp(&bits(b))
    }
}

/// `String.compareTo(String)`: UTF-16 code unit order.
fn java_string_compare(a: &str, b: &str) -> CmpOrdering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// A Java `assert` in a body that returns `Result`.
fn java_assert(cond: bool) -> Result<()> {
    if cfg!(debug_assertions) && !cond {
        return Err(LuceneError::from(LuceneErrorKind::AssertionError(
            String::new(),
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The anonymous `QueryCachingPolicy` of `testNoCaching`.
#[derive(Debug)]
struct CacheEverything;

impl QueryCachingPolicy for CacheEverything {
    fn on_use(&self, _query: &dyn Query) {}

    fn should_cache(&self, _query: &dyn Query) -> Result<bool> {
        Ok(true)
    }
}

// See LUCENE-10060:
fn test_no_caching(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let writer = random_writer(&dir)?;
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());

    // Setup some basic test documents:
    for (id, color, size) in [
        ("1", "Red", "Small"),
        ("2", "Green", "Small"),
        ("3", "Blue", "Small"),
        ("4", "Red", "Medium"),
        ("5", "Blue", "Medium"),
        ("6", "Blue", "Large"),
    ] {
        let mut doc = Document::new();
        doc.add(StringField::new("id", id.to_string(), Store::NO)?);
        doc.add(SortedDocValuesField::new(
            "id",
            BytesRef::from_char_sequence(id),
        ));
        doc.add(FacetField::new("Color", &[color])?);
        doc.add(FacetField::new("Size", &[size])?);
        add_doc(&writer, &config, &taxo_writer, &doc)?;
    }

    // Delete a couple documents. We'll want to make sure they don't get counted in binning:
    writer.delete_documents(Term::from_text("id", "4"))?;
    writer.delete_documents(Term::from_text("id", "6"))?;

    // Simple DDQ that just filters all results by Color == Blue:
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Color", &["Blue"])?;

    // Set up an IndexSearcher that will try to cache queries aggressively:
    let r = writer.get_reader()?;
    let mut searcher = Arc::try_unwrap(get_new_searcher(&r)).expect("sole owner");
    searcher.set_query_caching_policy(Arc::new(CacheEverything));
    let searcher = Arc::new(searcher);

    // Set up a DS instance for searching:
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;
    let ds = get_new_drill_sideways(f, &searcher, &config, &taxo_reader);

    // We'll use a CollectorManager to trigger the trickiest caching behavior:
    let collector_manager = SimpleCollectorManager::with_score_mode(
        10,
        Arc::new(|a: &CollectedResult, b: &CollectedResult| java_string_compare(a.id(), b.id())),
        ScoreMode::COMPLETE_NO_SCORES,
    );
    // Make sure our CM produces Collectors that _do not_ need scores to ensure IndexSearcher tries
    // to cache:
    Assert::assert_false(
        collector_manager
            .new_collector()?
            .score_mode()
            .needs_scores(),
    );
    // If we incorrectly cache here, the "sideways" FacetsCollectors will get populated with counts
    // for the deleted
    // docs. Make sure they don't:
    let concurrent_result = ds.search_with_hit_manager(&Arc::new(ddq), collector_manager)?;
    Assert::assert_equals(2, concurrent_result.collector_result.len());
    Assert::assert_equals(
        "dim=Color path=[] value=4 childCount=3\n  Blue (2)\n  Red (1)\n  Green (1)\n",
        top_string(&concurrent_result.base.facets, 10, "Color")?.as_str(),
    );
    Assert::assert_equals(
        "dim=Size path=[] value=2 childCount=2\n  Small (1)\n  Medium (1)\n",
        top_string(&concurrent_result.base.facets, 10, "Size")?.as_str(),
    );

    // test getTopChildren(0, dim)
    expect_throws(IAE, || {
        facets_of(&concurrent_result.base.facets).get_top_children(0, "Color", &[])
    });

    writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

/// The anonymous `LeafCollector` of `testCollectionTerminated`.
struct TerminatingLeafCollector {
    counter: i32,
}

impl LeafCollector for TerminatingLeafCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        // doesn't matter
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.counter += 1;
        if self.counter > 1 {
            return Err(CollectionError::CollectionTerminated);
        }
        Ok(())
    }
}

/// `expectThrows(CollectionTerminatedException.class, runnable)`.
fn expect_collection_terminated<T>(result: CollectionResult<T>) {
    match result {
        Err(CollectionError::CollectionTerminated) => {}
        Err(e) => java_throwable(&e.into_lucene_error()).throw(),
        Ok(_) => Assert::fail(
            "Expected exception CollectionTerminatedException but no exception was thrown",
        ),
    }
}

fn test_collection_terminated(_f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = random_writer(&dir)?;
    let result = (|| -> Result<()> {
        let mut d = Document::new();
        d.add(TextField::new("foo", "bar".to_string(), Store::NO)?);
        w.add_document(&d)?;

        let mut d = Document::new();
        d.add(TextField::new("foo", "bar".to_string(), Store::NO)?);
        w.add_document(&d)?;

        let mut d = Document::new();
        d.add(TextField::new("foo", "baz".to_string(), Store::NO)?);
        w.add_document(&d)?;

        let r = w.get_reader()?;
        let body = (|| -> Result<()> {
            let searcher = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
            let ctx = Arc::clone(&(Arc::clone(&r) as Arc<dyn IndexReader>).leaves()?[0]);

            let base_query: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
            let base_weight =
                searcher.create_weight(base_query, ScoreMode::COMPLETE_NO_SCORES, 1f32)?;
            let base_scorer = base_weight
                .scorer(&ctx)?
                .ok_or_else(LuceneError::null_pointer)?;

            let dim_q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("foo", "bar")));
            let dim_q = searcher.rewrite(dim_q)?;
            let dim_weight = searcher.create_weight(dim_q, ScoreMode::COMPLETE_NO_SCORES, 1f32)?;
            let dim_scorer = dim_weight
                .scorer(&ctx)?
                .ok_or_else(LuceneError::null_pointer)?;

            let dim_fc = Arc::new(Mutex::new(FacetsCollector::new()));
            let docs_and_cost = DocsAndCost::new(dim_scorer, Arc::clone(&dim_fc));

            let mut base_collector = TerminatingLeafCollector { counter: 0 };

            let score_sub_docs_at_once = LuceneTestCase::random().next_boolean();
            let mut scorer = DrillSidewaysScorer::new(
                Arc::clone(&ctx),
                base_scorer,
                vec![docs_and_cost],
                score_sub_docs_at_once,
            );
            expect_collection_terminated(scorer.score(&mut base_collector, None, 0, NO_MORE_DOCS));

            // We've set things up so that our base collector with throw CollectionTerminatedException
            // after collecting the first doc. This means we'll only collect the first indexed doc for
            // both our base and sideways dim facets collectors. What we really want to test here is
            // that the matching docs are still correctly present and populated after an early
            // termination occurs (i.e., #finish is properly called in that scenario):
            Assert::assert_equals(
                1,
                dim_fc
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_matching_docs()?
                    .len(),
            );
            Ok(())
        })();
        let closed = r.close();
        body?;
        closed
    })();
    let closed_w = w.close();
    let closed_d = dir.close();
    result?;
    closed_w?;
    closed_d
}

/// The shared corpus of `testBasic`, `testBasicWithCollectorManager` and
/// `testNoDrillDownFacetCollection`; `with_content` adds Java's
/// `newStringField("content", ...)` of `testBasicWithCollectorManager`.
fn index_publications(
    writer: &RandomIndexWriter,
    config: &FacetsConfig,
    taxo_writer: &DirectoryTaxonomyWriter,
    with_content: bool,
) -> Result<()> {
    for (content, author, date) in [
        ("foo", "Bob", ["2010", "10", "15"]),
        ("foo", "Lisa", ["2010", "10", "20"]),
        ("foo", "Lisa", ["2012", "1", "1"]),
        ("bar", "Susan", ["2012", "1", "7"]),
        ("bar", "Frank", ["1999", "5", "5"]),
    ] {
        let mut doc = Document::new();
        if with_content {
            doc.add(LuceneTestCase::new_string_field(
                "content",
                content,
                Store::NO,
            ));
        }
        doc.add(FacetField::new("Author", &[author])?);
        doc.add(FacetField::new("Publish Date", &date)?);
        add_doc(writer, config, taxo_writer, &doc)?;
    }
    Ok(())
}

fn test_basic(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("Publish Date", true);

    let writer = random_writer(&dir)?;

    index_publications(&writer, &config, &taxo_writer, false)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = get_new_searcher(&r);

    // System.out.println("searcher=" + searcher);

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    // Run all the basic test cases with a standard DrillSideways implementation:
    let ds = get_new_drill_sideways(f, &searcher, &config, &taxo_reader);
    run_drill_sideways_test_cases(&config, &ds)?;

    // Run all the basic test cases but make sure DS is set to score all sub-docs at once, so
    // we exercise the doc-at-a-time scoring methodology:
    let ds = get_new_drill_sideways_score_subdocs_at_once(f, &searcher, &config, &taxo_reader);
    run_drill_sideways_test_cases(&config, &ds)?;

    writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

/// `DrillSidewaysAssertingIndexSearcher`: the overrides of its `protected
/// search(LeafReaderContextPartition[], Weight, Collector)`.
///
/// We can't use AssertingIndexSearcher unfortunately since it may randomly
/// decide to bulk score a sub-range of docs instead of all docs at once. This
/// is incompatible will drill sideways, so we have to do our own check here.
/// This just makes sure we call #finish on the last leaf. It's too bad we need
/// to do this and maybe some day we can clean this up by rethinking
/// drill-sideways:
#[derive(Debug)]
struct DrillSidewaysAssertingIndexSearcher;

impl DrillSidewaysAssertingIndexSearcher {
    /// `DrillSidewaysAssertingIndexSearcher(IndexReader)`: `super(r)`.
    fn create(r: Arc<dyn IndexReader>) -> Result<IndexSearcher> {
        let mut searcher = IndexSearcher::new(r)?;
        searcher.set_overrides(Arc::new(DrillSidewaysAssertingIndexSearcher));
        Ok(searcher)
    }
}

impl IndexSearcherOverrides for DrillSidewaysAssertingIndexSearcher {
    fn search_partitions(
        &self,
        searcher: &IndexSearcher,
        partitions: &[LeafReaderContextPartition],
        weight: &Arc<dyn Weight>,
        collector: &mut dyn rucene::search::Collector,
    ) -> Result<()> {
        let mut asserting_collector = AssertingCollector::wrap(CollectorRef(collector));
        searcher.super_search_partitions(partitions, weight, &mut asserting_collector)?;
        java_assert(asserting_collector.has_finished_collecting_previous_leaf())
    }
}

fn test_leaf_collector_single_finish_call(_f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let w = random_writer(&dir)?;
    let taxo_w = create_taxo_writer(&taxo_dir)?;
    let result = (|| -> Result<()> {
        let facets_config = Arc::new(FacetsConfig::new());

        let mut d = Document::new();
        d.add(FacetField::new("foo", &["bar"])?);
        add_doc(&w, &facets_config, &taxo_w, &d)?;

        let r = w.get_reader()?;
        let taxo_r = nrt_taxo_reader(&taxo_w)?;
        let body = (|| -> Result<()> {
            let searcher = Arc::new(DrillSidewaysAssertingIndexSearcher::create(
                Arc::clone(&r) as Arc<dyn IndexReader>
            )?);

            let base_query: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
            let mut ddq =
                DrillDownQuery::with_base_query(Arc::clone(&facets_config), Some(base_query));
            ddq.add("foo", &["bar"])?;
            let drill_sideways =
                DrillSideways::new(searcher, Arc::clone(&facets_config), Arc::clone(&taxo_r));

            // It doesn't really matter what we have our collector manager do here since all we care
            // about for this test are the assertions provided by the "asserting" collectors/leaf
            // collectors, etc. But we have this definition already for other tests and it's easy to
            // use, so we'll use it here and do a little extra checking:
            let cm = SimpleCollectorManager::new(
                10,
                Arc::new(|a: &CollectedResult, b: &CollectedResult| {
                    a.doc_and_score.doc.cmp(&b.doc_and_score.doc)
                }),
            );

            let result = drill_sideways.search_with_hit_manager(&Arc::new(ddq), cm)?;

            Assert::assert_equals(1, result.collector_result.len());
            Ok(())
        })();
        let c1 = r.close();
        let c2 = taxo_r.close();
        body?;
        c1?;
        c2
    })();
    let c1 = w.close();
    let c2 = taxo_w.close();
    let c3 = taxo_dir.close();
    let c4 = dir.close();
    result?;
    c1?;
    c2?;
    c3?;
    c4
}

fn run_drill_sideways_test_cases(config: &Arc<FacetsConfig>, ds: &DrillSideways) -> Result<()> {
    //  case: drill-down on a single field; in this
    // case the drill-sideways + drill-down counts ==
    // drill-down of just the query:
    let mut ddq = DrillDownQuery::new(Arc::clone(config));
    ddq.add("Author", &["Lisa"])?;
    let r = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(2, total_hits(&r));
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.facets, 10, "Publish Date")?.as_str(),
    );

    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published twice, and Frank/Susan/Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.facets, 10, "Author")?.as_str(),
    );

    // Another simple case: drill-down on single fields
    // but OR of two values
    let mut ddq = DrillDownQuery::new(Arc::clone(config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Author", &["Bob"])?;
    let r = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(3, total_hits(&r));
    // Publish Date is only drill-down: Lisa and Bob
    // (drill-down) published twice in 2010 and once in 2012:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=3 childCount=2\n  2010 (2)\n  2012 (1)\n",
        top_string(&r.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published twice, and Frank/Susan/Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.facets, 10, "Author")?.as_str(),
    );

    Assert::assert_true(is_multi_facets(&r.facets));
    let all_results = facets_of(&r.facets).get_all_dims(10)?;
    Assert::assert_equals(2, all_results.len());
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        all_results[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=Publish Date path=[] value=3 childCount=2\n  2010 (2)\n  2012 (1)\n",
        all_results[1].to_string().as_str(),
    );

    // test default implementation of getTopDims
    let top_n_dims_result = facets_of(&r.facets).get_top_dims(2, 1)?;
    Assert::assert_equals(2, top_n_dims_result.len());
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n",
        top_n_dims_result[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=Publish Date path=[] value=3 childCount=2\n  2010 (2)\n",
        top_n_dims_result[1].to_string().as_str(),
    );

    // test getTopDims(0, 1)
    let top_dims_results2 = facets_of(&r.facets).get_top_dims(0, 1)?;
    Assert::assert_equals(0, top_dims_results2.len());

    // test getAllDims(0)
    let final_r1 = &r;
    expect_throws(IAE, || facets_of(&final_r1.facets).get_all_dims(0));

    // More interesting case: drill-down on two fields
    let mut ddq = DrillDownQuery::new(Arc::clone(config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Publish Date", &["2010"])?;
    let r = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(1, total_hits(&r));
    // Publish Date is drill-sideways + drill-down: Lisa
    // (drill-down) published once in 2010 and once in 2012:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down:
    // only Lisa & Bob published (once each) in 2010:
    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)\n",
        top_string(&r.facets, 10, "Author")?.as_str(),
    );

    // Even more interesting case: drill down on two fields,
    // but one of them is OR
    let mut ddq = DrillDownQuery::new(Arc::clone(config));

    // Drill down on Lisa or Bob:
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Publish Date", &["2010"])?;
    ddq.add("Author", &["Bob"])?;
    let r = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(2, total_hits(&r));
    // Publish Date is both drill-sideways + drill-down:
    // Lisa or Bob published twice in 2010 and once in 2012:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=3 childCount=2\n  2010 (2)\n  2012 (1)\n",
        top_string(&r.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down:
    // only Lisa & Bob published (once each) in 2010:
    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)\n",
        top_string(&r.facets, 10, "Author")?.as_str(),
    );

    // Test drilling down on invalid field:
    let mut ddq = DrillDownQuery::new(Arc::clone(config));
    ddq.add("Foobar", &["Baz"])?;
    let r = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(0, total_hits(&r));
    Assert::assert_true(top(&r.facets, 10, "Publish Date", &[])?.is_none());
    Assert::assert_true(top(&r.facets, 10, "Foobar", &[])?.is_none());

    // Test drilling down on valid term or'd with invalid term:
    let mut ddq = DrillDownQuery::new(Arc::clone(config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Author", &["Tom"])?;
    let r = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(2, total_hits(&r));
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published twice, and Frank/Susan/Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.facets, 10, "Author")?.as_str(),
    );

    // LUCENE-4915: test drilling down on a dimension but
    // NOT facet counting it:
    let mut ddq = DrillDownQuery::new(Arc::clone(config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Author", &["Tom"])?;
    let r = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(2, total_hits(&r));
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.facets, 10, "Publish Date")?.as_str(),
    );

    // Test main query gets null scorer:
    let mut ddq = DrillDownQuery::with_base_query(
        Arc::clone(config),
        Some(Arc::new(TermQuery::new(Term::from_text("foobar", "baz")))),
    );
    ddq.add("Author", &["Lisa"])?;
    let r = ds.search_after(None, ddq, 10)?;

    Assert::assert_equals(0, total_hits(&r));
    Assert::assert_true(top(&r.facets, 10, "Publish Date", &[])?.is_none());
    Assert::assert_true(top(&r.facets, 10, "Author", &[])?.is_none());

    // test getTopChildren(0, dim)
    let final_r = &r;
    expect_throws(IAE, || {
        facets_of(&final_r.facets).get_top_children(0, "Author", &[])
    });
    Ok(())
}

/// The anonymous `DrillSideways` subclasses whose
/// `createDrillDownFacetsCollectorManager()` returns `null`.
struct NoDrillDownCollector;

impl DrillSidewaysOverrides for NoDrillDownCollector {
    fn create_drill_down_facets_collector_manager(&self) -> Option<FacetsCollectorManager> {
        None
    }
}

/// The anonymous `TermQuery` subclass of `testBasicWithCollectorManager`
/// (`TestDrillSideways$6`), whose `rewrite` returns a new instance.
#[derive(Debug, Clone)]
struct RewritingTermQuery {
    inner: TermQuery,
}

impl Query for RewritingTermQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor)
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
        self.inner.create_weight(searcher, score_mode, boost)
    }

    fn rewrite(&self, _searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        // return a new instance, forcing the DrillDownQuery to also rewrite itself, exposing
        // the bug in LUCENE-9988:
        Ok(Some(Arc::new(TermQuery::new(
            self.inner.get_term().clone(),
        ))))
    }

    fn as_term_query(&self) -> Option<&TermQuery> {
        Some(&self.inner)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<RewritingTermQuery>()
                .is_some_and(|o| self.inner.get_term() == o.inner.get_term())
    }

    fn query_hash(&self) -> i32 {
        // `classHash() ^ term.hashCode()`.
        self.class_hash() ^ self.inner.get_term().hash_code()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.TestDrillSideways$6"
    }
}

fn test_basic_with_collector_manager(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("Publish Date", true);

    let writer = random_writer(&dir)?;

    index_publications(&writer, &config, &taxo_writer, true)?;

    // NRT open
    let reader = writer.get_reader()?;
    let searcher = get_new_searcher(&reader);

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let ds = get_new_drill_sideways(f, &searcher, &config, &taxo_reader);

    let comparator: Comparator = Arc::new(|a: &CollectedResult, b: &CollectedResult| {
        let mut cmp = java_float_compare(b.doc_and_score.score, a.doc_and_score.score); // highest-to-lowest
        if cmp == CmpOrdering::Equal {
            cmp = a.doc_and_score.doc.cmp(&b.doc_and_score.doc); // tie-break
        }
        cmp
    });

    let manager = SimpleCollectorManager::new(10, comparator);
    let search = |ds: &DrillSideways, ddq: DrillDownQuery| {
        ds.search_with_hit_manager(&Arc::new(ddq), manager.clone())
    };

    // case: drill-down on a single field; in this
    // case the drill-sideways + drill-down counts ==
    // drill-down of just the query:
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(2, r.collector_result.len());
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );

    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published twice, and Frank/Susan/Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    // Same simple case, but no baseQuery (pure browse):
    // drill-down on a single field; in this case the
    // drill-sideways + drill-down counts == drill-down of
    // just the query:
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    let r = search(&ds, ddq)?;

    Assert::assert_equals(2, r.collector_result.len());
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );

    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published twice, and Frank/Susan/Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    // Another simple case: drill-down on single fields
    // but OR of two values
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Author", &["Bob"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(3, r.collector_result.len());
    // Publish Date is only drill-down: Lisa and Bob
    // (drill-down) published twice in 2010 and once in 2012:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=3 childCount=2\n  2010 (2)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published twice, and Frank/Susan/Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    Assert::assert_true(is_multi_facets(&r.base.facets));
    let all_results = facets_of(&r.base.facets).get_all_dims(10)?;
    Assert::assert_equals(2, all_results.len());
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        all_results[0].to_string().as_str(),
    );
    Assert::assert_equals(
        "dim=Publish Date path=[] value=3 childCount=2\n  2010 (2)\n  2012 (1)\n",
        all_results[1].to_string().as_str(),
    );

    // test default implementation of getTopDims
    let top_n_dims_result = facets_of(&r.base.facets).get_top_dims(1, 2)?;
    Assert::assert_equals(1, top_n_dims_result.len());
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n",
        top_n_dims_result[0].to_string().as_str(),
    );

    // test getTopDims(10, 10) and expect same results from getAllDims(10)
    let all_dims_results = facets_of(&r.base.facets).get_top_dims(10, 10)?;
    assert_equals(&all_results, &all_dims_results);

    // More interesting case: drill-down on two fields
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Publish Date", &["2010"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(1, r.collector_result.len());
    // Publish Date is drill-sideways + drill-down: Lisa
    // (drill-down) published once in 2010 and once in 2012:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down:
    // only Lisa & Bob published (once each) in 2010:
    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    // Even more interesting case: drill down on two fields,
    // but one of them is OR
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));

    // Drill down on Lisa or Bob:
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Publish Date", &["2010"])?;
    ddq.add("Author", &["Bob"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(2, r.collector_result.len());
    // Publish Date is both drill-sideways + drill-down:
    // Lisa or Bob published twice in 2010 and once in 2012:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=3 childCount=2\n  2010 (2)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down:
    // only Lisa & Bob published (once each) in 2010:
    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    // Test drilling down on invalid field:
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Foobar", &["Baz"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(0, r.collector_result.len());
    Assert::assert_true(top(&r.base.facets, 10, "Publish Date", &[])?.is_none());
    Assert::assert_true(top(&r.base.facets, 10, "Foobar", &[])?.is_none());

    // Test drilling down on valid term or'd with invalid term:
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Author", &["Tom"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(2, r.collector_result.len());
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published twice, and Frank/Susan/Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    // LUCENE-4915: test drilling down on a dimension but
    // NOT facet counting it:
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Author", &["Tom"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(2, r.collector_result.len());
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );

    // Test main query gets null scorer:
    let mut ddq = DrillDownQuery::with_base_query(
        Arc::clone(&config),
        Some(Arc::new(TermQuery::new(Term::from_text("foobar", "baz")))),
    );
    ddq.add("Author", &["Lisa"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(0, r.collector_result.len());
    Assert::assert_true(top(&r.base.facets, 10, "Publish Date", &[])?.is_none());
    Assert::assert_true(top(&r.base.facets, 10, "Author", &[])?.is_none());

    // Test no drill down dims:
    let ddq = DrillDownQuery::with_base_query(
        Arc::clone(&config),
        Some(Arc::new(MatchAllDocsQuery::instance())),
    );
    let r = search(&ds, ddq)?;
    Assert::assert_equals(5, r.collector_result.len());
    Assert::assert_equals(
        "dim=Publish Date path=[] value=5 childCount=3\n  2010 (2)\n  2012 (2)\n  1999 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    // Test no drill down dims with null FacetsCollectorManager for the main query:
    let ddq = DrillDownQuery::with_base_query(
        Arc::clone(&config),
        Some(Arc::new(MatchAllDocsQuery::instance())),
    );
    let r = search(
        &DrillSideways::with_overrides(
            Arc::clone(&searcher),
            Arc::clone(&config),
            Some(Arc::clone(&taxo_reader)),
            None,
            None,
            Arc::new(NoDrillDownCollector),
        ),
        ddq,
    )?;
    Assert::assert_equals(5, r.collector_result.len());
    // Expect null facets since we provided a null FacetsCollectorManager
    Assert::assert_true(r.base.facets.is_none());

    // Test the case where the base query rewrites itself. See LUCENE-9988:
    let base_query: Arc<dyn Query> = Arc::new(RewritingTermQuery {
        inner: TermQuery::new(Term::from_text("content", "foo")),
    });
    let mut ddq = DrillDownQuery::with_base_query(Arc::clone(&config), Some(base_query));
    ddq.add("Author", &["Lisa"])?;
    let r = search(&ds, ddq)?;
    Assert::assert_equals(2, r.collector_result.len());
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r.base.facets, 10, "Publish Date")?.as_str(),
    );
    Assert::assert_equals(
        "dim=Author path=[] value=3 childCount=2\n  Lisa (2)\n  Bob (1)\n",
        top_string(&r.base.facets, 10, "Author")?.as_str(),
    );

    writer.close()?;
    reader.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_sometimes_invalid_drill_down(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("Publish Date", true);

    let mut doc = Document::new();
    doc.add(FacetField::new("Author", &["Bob"])?);
    doc.add(FacetField::new("Publish Date", &["2010", "10", "15"])?);
    add_doc(&writer, &config, &taxo_writer, &doc)?;

    let mut doc = Document::new();
    doc.add(FacetField::new("Author", &["Lisa"])?);
    doc.add(FacetField::new("Publish Date", &["2010", "10", "20"])?);
    add_doc(&writer, &config, &taxo_writer, &doc)?;

    writer.commit()?;

    // 2nd segment has no Author:
    let mut doc = Document::new();
    doc.add(FacetField::new("Foobar", &["Lisa"])?);
    doc.add(FacetField::new("Publish Date", &["2012", "1", "1"])?);
    add_doc(&writer, &config, &taxo_writer, &doc)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = get_new_searcher(&r);

    // System.out.println("searcher=" + searcher);

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    let r2 =
        get_new_drill_sideways(f, &searcher, &config, &taxo_reader).search_after(None, ddq, 10)?;

    Assert::assert_equals(1, total_hits(&r2));
    // Publish Date is only drill-down, and Lisa published
    // one in 2012 and one in 2010:
    Assert::assert_equals(
        "dim=Publish Date path=[] value=1 childCount=1\n  2010 (1)\n",
        top_string(&r2.facets, 10, "Publish Date")?.as_str(),
    );
    // Author is drill-sideways + drill-down: Lisa
    // (drill-down) published once, and Bob
    // published once:
    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)\n",
        top_string(&r2.facets, 10, "Author")?.as_str(),
    );

    writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_multiple_requests_per_dim(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("dim", true);

    for path in [
        &["a", "x"][..],
        &["a", "y"][..],
        &["a", "z"][..],
        &["b"][..],
        &["c"][..],
        &["d"][..],
    ] {
        let mut doc = Document::new();
        doc.add(FacetField::new("dim", path)?);
        add_doc(&writer, &config, &taxo_writer, &doc)?;
    }

    // NRT open
    let r = writer.get_reader()?;
    let searcher = get_new_searcher(&r);

    // System.out.println("searcher=" + searcher);

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("dim", &["a"])?;
    let r2 =
        get_new_drill_sideways(f, &searcher, &config, &taxo_reader).search_after(None, ddq, 10)?;

    Assert::assert_equals(3, total_hits(&r2));
    Assert::assert_equals(
        "dim=dim path=[] value=6 childCount=4\n  a (3)\n  b (1)\n  c (1)\n  d (1)\n",
        top_string(&r2.facets, 10, "dim")?.as_str(),
    );
    Assert::assert_equals(
        "dim=dim path=[a] value=3 childCount=3\n  x (1)\n  y (1)\n  z (1)\n",
        top(&r2.facets, 10, "dim", &["a"])?
            .expect("java.lang.NullPointerException")
            .as_str(),
    );

    writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

/// `private static class Doc implements Comparable<Doc>`.
#[derive(Clone, Debug, Default)]
struct Doc {
    id: String,
    content_token: String,
    // -1 if the doc is missing this dim, else the index
    // -into the values for this dim:
    dims: Vec<i32>,
    // 2nd value per dim for the doc (so we test
    // multivalued fields):
    dims2: Vec<i32>,
    deleted: bool,
}

impl Fixture {
    /// `randomContentToken(boolean isQuery)`.
    fn random_content_token(&self, is_query: bool) -> String {
        let d = LuceneTestCase::random().next_double();
        if is_query {
            if d < 0.33 {
                "a"
            } else if d < 0.66 {
                "b"
            } else {
                "c"
            }
        } else if d <= self.a_chance {
            "a"
        } else if d < self.a_chance + self.b_chance {
            "b"
        } else {
            "c"
        }
        .to_string()
    }
}

/// The anonymous filter `Query` of `testRandom` (`TestDrillSideways$7`):
/// only even ids, equal only to itself.
#[derive(Debug)]
struct OnlyEvenFilter {
    this: Weak<OnlyEvenFilter>,
}

impl OnlyEvenFilter {
    fn new() -> Arc<OnlyEvenFilter> {
        Arc::new_cyclic(|this| OnlyEvenFilter { this: this.clone() })
    }
}

/// The anonymous `ConstantScoreWeight` of the filter (`TestDrillSideways$7$1`).
#[derive(Debug)]
struct OnlyEvenWeight {
    score_mode: ScoreMode,
}

/// The anonymous `TwoPhaseIterator` of the filter (`TestDrillSideways$7$1$1`).
struct OnlyEvenTwoPhase {
    approximation: AllDocIdSetIterator,
    reader: Arc<dyn LeafReader>,
}

impl TwoPhaseIterator for OnlyEvenTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let doc_id = self.approximation.doc_id();
        let id = self
            .reader
            .stored_fields()?
            .document(doc_id)?
            .get("id")
            .ok_or_else(LuceneError::null_pointer)?;
        let id: i32 = id
            .parse()
            .map_err(|_| LuceneError::illegal_argument(format!("For input string: \"{id}\"")))?;
        Ok((id & 1) == 0)
    }

    fn match_cost(&self) -> f32 {
        1000f32
    }
}

impl ConstantScoreWeightImpl for OnlyEvenWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let approximation = doc_id_set_iterator::all(reader.max_doc())?;
        let scorer = ConstantScoreScorer::from_two_phase(
            score,
            self.score_mode,
            Box::new(OnlyEvenTwoPhase {
                approximation,
                reader,
            }),
        );
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Query for OnlyEvenFilter {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let this: Arc<dyn Query> = self.this.upgrade().ok_or_else(LuceneError::null_pointer)?;
        Ok(Arc::new(ConstantScoreWeight::new(
            this,
            boost,
            OnlyEvenWeight { score_mode },
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        "drillSidewaysTestFilter".to_string()
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        std::ptr::eq(
            self as *const OnlyEvenFilter as *const u8,
            other.as_any() as *const dyn Any as *const u8,
        )
    }

    fn query_hash(&self) -> i32 {
        // `System.identityHashCode(this)`.
        let address = self as *const OnlyEvenFilter as usize as u64;
        (address ^ (address >> 32)) as i32
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.TestDrillSideways$7"
    }
}

fn test_random(f: &mut Fixture) -> Result<()> {
    let random = LuceneTestCase::random();
    while f.a_chance == 0.0 {
        f.a_chance = random.next_double();
    }
    while f.b_chance == 0.0 {
        f.b_chance = random.next_double();
    }
    while f.c_chance == 0.0 {
        f.c_chance = random.next_double();
    }
    // aChance = .01;
    // bChance = 0.5;
    // cChance = 1.0;
    let sum = f.a_chance + f.b_chance + f.c_chance;
    f.a_chance /= sum;
    f.b_chance /= sum;
    f.c_chance /= sum;

    let num_dims = TestUtil::next_int(&*random, 2, 5) as usize;
    // int numDims = 3;
    let num_docs = LuceneTestCase::at_least(300);
    // int numDocs = 20;
    if *VERBOSE {
        println!(
            "numDims={num_dims} numDocs={num_docs} aChance={} bChance={} cChance={}",
            rucene::util::java_fmt::double_to_string(f.a_chance),
            rucene::util::java_fmt::double_to_string(f.b_chance),
            rucene::util::java_fmt::double_to_string(f.c_chance)
        );
    }
    let mut dim_values: Vec<Vec<String>> = Vec::with_capacity(num_dims);
    let mut value_count = 2usize;

    for _dim in 0..num_dims {
        let mut values: JavaHashSet = JavaHashSet::new();
        while values.len() < value_count {
            let s = TestUtil::random_realistic_unicode_string(&*random);
            // String s = _TestUtil.randomString(random());
            if !s.is_empty() {
                values.insert(s);
            }
        }
        dim_values.push(values.iter().cloned().collect());
        value_count *= 2;
    }

    let mut docs: Vec<Doc> = Vec::new();
    for i in 0..num_docs {
        let mut doc = Doc {
            id: i.to_string(),
            content_token: f.random_content_token(false),
            dims: vec![0; num_dims],
            dims2: vec![0; num_dims],
            deleted: false,
        };
        for dim in 0..num_dims {
            let len = dim_values[dim].len() as i32;
            if random.next_int_bound(5) == 3 {
                // This doc is missing this dim:
                doc.dims[dim] = -1;
            } else if len <= 4 {
                let mut dim_upto = 0;
                doc.dims[dim] = len - 1;
                while dim_upto < len {
                    if random.next_boolean() {
                        doc.dims[dim] = dim_upto;
                        break;
                    }
                    dim_upto += 1;
                }
            } else {
                doc.dims[dim] = random.next_int_bound(len);
            }

            if random.next_int_bound(5) == 3 {
                // 2nd value:
                doc.dims2[dim] = random.next_int_bound(len);
            } else {
                doc.dims2[dim] = -1;
            }
        }
        docs.push(doc);
    }

    let d = LuceneTestCase::new_directory();
    let td = LuceneTestCase::new_directory();

    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random,
    ))));
    iwc.set_info_stream(Arc::new(NoOutputInfoStream))?;
    let w = RandomIndexWriter::new_with_config(&*random, d.as_directory(), iwc)?;
    let tw = Arc::new(DirectoryTaxonomyWriter::with_open_mode(
        td.as_directory(),
        OpenMode::CREATE,
    )?);
    let config = Arc::new(FacetsConfig::new());
    for i in 0..num_dims {
        config.set_multi_valued(&format!("dim{i}"), true);
    }

    let do_use_dv = random.next_boolean();

    for raw_doc in &docs {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_string_field(
            "id",
            &raw_doc.id,
            Store::YES,
        ));
        doc.add(SortedDocValuesField::new(
            "id",
            BytesRef::from_char_sequence(&raw_doc.id),
        ));
        doc.add(LuceneTestCase::new_string_field(
            "content",
            &raw_doc.content_token,
            Store::NO,
        ));

        if *VERBOSE {
            println!("  doc id={} token={}", raw_doc.id, raw_doc.content_token);
        }
        for dim in 0..num_dims {
            let dim_name = format!("dim{dim}");
            for (j, dim_value) in [raw_doc.dims[dim], raw_doc.dims2[dim]]
                .into_iter()
                .enumerate()
            {
                if dim_value != -1 {
                    let value = dim_values[dim][dim_value as usize].as_str();
                    if do_use_dv {
                        doc.add(SortedSetDocValuesFacetField::new(&dim_name, &[value])?);
                    } else {
                        doc.add(FacetField::new(&dim_name, &[value])?);
                    }
                    doc.add(StringField::new(
                        dim_name.clone(),
                        value.to_string(),
                        Store::YES,
                    )?);
                    doc.add(SortedSetDocValuesField::new(
                        dim_name.clone(),
                        BytesRef::from_char_sequence(value),
                    ));
                    if *VERBOSE {
                        let indent = if j == 0 { "    " } else { "      " };
                        println!("{indent}dim{dim}={}", BytesRef::from_char_sequence(value));
                    }
                }
            }
        }

        w.add_document(&config.build_with_taxonomy(Some(&*tw), &doc)?)?;
    }

    if random.next_boolean() {
        // Randomly delete a few docs:
        let num_del = TestUtil::next_int(&*random, 1, (num_docs as f64 * 0.05) as i32);
        if *VERBOSE {
            println!("delete {num_del}");
        }
        let mut del_count = 0;
        while del_count < num_del {
            let idx = random.next_int_bound(docs.len() as i32) as usize;
            let doc = &mut docs[idx];
            if !doc.deleted {
                if *VERBOSE {
                    println!("  delete id={}", doc.id);
                }
                doc.deleted = true;
                w.delete_documents(Term::from_text("id", &doc.id))?;
                del_count += 1;
            }
        }
    }

    if random.next_boolean() {
        if *VERBOSE {
            println!("TEST: forceMerge(1)...");
        }
        w.force_merge(1)?;
    }
    let r = w.get_reader()?;

    let s = get_new_searcher(&r);

    let sorted_set_dv_state: Option<Arc<dyn SortedSetDocValuesReaderState>> = if do_use_dv {
        Some(Arc::new(DefaultSortedSetDocValuesReaderState::new(
            Arc::clone(s.get_index_reader()),
            Arc::clone(&config),
        )?))
    } else {
        None
    };

    if *VERBOSE {
        println!("r.numDocs() = {}", r.num_docs());
    }

    // NRT open
    let tr = nrt_taxo_reader(&tw)?;

    let num_iters = LuceneTestCase::at_least(10);

    for iter in 0..num_iters {
        let content_token = if random.next_int_bound(30) == 17 {
            None
        } else {
            Some(f.random_content_token(true))
        };
        let num_drill_down = TestUtil::next_int(&*random, 1, 4.min(num_dims as i32));
        if *VERBOSE {
            println!(
                "\nTEST: iter={iter} baseQuery={} numDrillDown={num_drill_down} useSortedSetDV={do_use_dv}",
                content_token.as_deref().unwrap_or("null")
            );
        }

        let mut drill_downs: Vec<Option<Vec<String>>> = vec![None; num_dims];

        let mut count = 0;
        while count < num_drill_down {
            let dim = random.next_int_bound(num_dims as i32) as usize;
            if drill_downs[dim].is_none() {
                let len = dim_values[dim].len() as i32;
                if random.next_boolean() {
                    // Drill down on one value:
                    drill_downs[dim] = Some(vec![dim_values[dim]
                        [random.next_int_bound(len) as usize]
                        .clone()]);
                } else {
                    let or_count = TestUtil::next_int(&*random, 1, 5.min(len)) as usize;
                    let mut values: Vec<String> = Vec::with_capacity(or_count);
                    for i in 0..or_count {
                        loop {
                            let mut value =
                                Some(dim_values[dim][random.next_int_bound(len) as usize].clone());
                            for existing in values.iter().take(i) {
                                if value.as_deref() == Some(existing.as_str()) {
                                    value = None;
                                    break;
                                }
                            }
                            if let Some(value) = value {
                                values.push(value);
                                break;
                            }
                        }
                    }
                    drill_downs[dim] = Some(values);
                }
                if *VERBOSE {
                    let values: Vec<String> = drill_downs[dim]
                        .as_ref()
                        .expect("set above")
                        .iter()
                        .map(|v| BytesRef::from_char_sequence(v).to_string())
                        .collect();
                    println!("  dim{dim}=[{}]", values.join(", "));
                }
                count += 1;
            }
        }

        let base_query: Arc<dyn Query> = match &content_token {
            None => Arc::new(MatchAllDocsQuery::instance()),
            Some(token) => Arc::new(TermQuery::new(Term::from_text("content", token))),
        };

        let mut ddq =
            DrillDownQuery::with_base_query(Arc::clone(&config), Some(Arc::clone(&base_query)));

        for (dim, values) in drill_downs.iter().enumerate() {
            if let Some(values) = values {
                for value in values {
                    // Sometimes use a "traditional" term query and sometimes use a two-phase approach to
                    // ensure code coverage:
                    let dim_name = format!("dim{dim}");
                    if random.next_boolean() {
                        ddq.add(&dim_name, &[value.as_str()])?;
                    } else {
                        ddq.add_query(
                            &dim_name,
                            SortedSetDocValuesField::new_slow_exact_query(
                                &dim_name,
                                &BytesRef::from_char_sequence(value),
                            ),
                        )?;
                    }
                }
            }
        }

        let mut filter: Option<Arc<dyn Query>> = if random.next_int_bound(7) == 6 {
            if *VERBOSE {
                println!("  only-even filter");
            }
            Some(OnlyEvenFilter::new())
        } else {
            None
        };

        // Verify docs are always collected in order.  If we
        // had an AssertingScorer it could catch it when
        // Weight.scoresDocsOutOfOrder lies!:
        get_new_drill_sideways(f, &s, &config, &tr).search_with_hit_manager(
            &Arc::new(ddq.clone_query()),
            SimpleCollectorManager::with_score_mode(
                num_docs as usize,
                Arc::new(|a: &CollectedResult, b: &CollectedResult| {
                    a.doc_and_score.doc.cmp(&b.doc_and_score.doc)
                }),
                ScoreMode::COMPLETE_NO_SCORES,
            ),
        )?;

        let sort = Sort::new_fields(vec![SortField::new(
            Some("id".to_string()),
            SortFieldType::String,
        )?])?;
        let ds = if do_use_dv {
            get_new_drill_sideways_state(
                f,
                &s,
                &config,
                Arc::clone(sorted_set_dv_state.as_ref().expect("doUseDV")),
            )
        } else {
            get_new_drill_sideways_build_facets_result(f, &s, &config, &tr)
        };

        let actual = if random.next_boolean() {
            let dsr = ds.search_sorted(
                ddq.clone_query(),
                filter.clone(),
                None,
                num_docs,
                Some(&sort),
                true,
            )?;
            let hits = dsr.hits.as_ref().expect("java.lang.NullPointerException");
            let results = match hits {
                FacetsTopDocs::TopDocs(td) => td
                    .score_docs
                    .iter()
                    .map(|sd| DocAndScore::new(sd.doc, sd.score))
                    .collect(),
                FacetsTopDocs::TopFieldDocs(td) => td
                    .score_docs
                    .iter()
                    .map(|fd| DocAndScore::new(fd.score_doc.doc, fd.score_doc.score))
                    .collect(),
            };
            DSResults {
                result_count: *hits.total_hits(),
                results,
                facets: dsr.facets,
            }
        } else {
            // can't support filter queries with this impl
            filter = None;

            let manager = SimpleCollectorManager::new(
                num_docs as usize,
                Arc::new(|a: &CollectedResult, b: &CollectedResult| {
                    java_string_compare(a.id(), b.id())
                }),
            );
            // Because we validate the scores computed through DrillSideways against those found through
            // a direct search
            // against IndexSearcher, make sure our Collectors announce themselves as requiring scores.
            // See conversation
            // in LUCENE-10060 where this bug was introduced and then discovered:
            Assert::assert_true(manager.new_collector()?.score_mode.needs_scores());
            let cr = ds.search_with_hit_manager(&Arc::new(ddq.clone_query()), manager)?;
            let results = cr.collector_result;
            DSResults {
                result_count: TotalHits::new(results.len() as i64, TotalHitsRelation::EQUAL_TO)?,
                results,
                facets: cr.base.facets,
            }
        };

        let expected = slow_drill_sideways_search(
            &s,
            &docs,
            content_token.as_deref(),
            &drill_downs,
            &dim_values,
            filter.as_ref(),
        )?;

        let hits = s.search(Arc::clone(&base_query), num_docs)?;
        let mut scores: HashMap<String, f32> = HashMap::new();
        for sd in &hits.score_docs {
            scores.insert(
                s.stored_fields()?
                    .document(sd.doc)?
                    .get("id")
                    .ok_or_else(LuceneError::null_pointer)?,
                sd.score,
            );
        }
        if *VERBOSE {
            println!("  verify all facets");
        }
        verify_equals(&dim_values, &s, &expected, &actual, &scores, do_use_dv)?;

        // Make sure drill down doesn't change score:
        let mut q: Arc<dyn Query> = Arc::new(ddq);
        if let Some(filter) = &filter {
            let mut builder = BooleanQueryBuilder::new();
            builder.add(q, Occur::MUST)?;
            builder.add(Arc::clone(filter), Occur::FILTER)?;
            q = Arc::new(builder.build());
        }
        let ddq_hits = s.search(q, num_docs)?;
        Assert::assert_equals(expected.hits.len() as i64, ddq_hits.total_hits.value());
        for (i, hit) in expected.hits.iter().enumerate() {
            // Score should be IDENTICAL:
            Assert::assert_equals_float_msg(
                None,
                *scores.get(&hit.id).expect("java.lang.NullPointerException"),
                ddq_hits.score_docs[i].score,
                0.0f32,
            );
        }
    }

    w.close()?;
    r.close()?;
    tr.close()?;
    tw.close()?;
    d.close()?;
    td.close()
}

/// `AssertingFacetsCollectorManager extends FacetsCollectorManager`.
struct AssertingFacetsCollectorManager;

impl AssertingFacetsCollectorManager {
    /// `new AssertingFacetsCollectorManager()`.
    fn create() -> FacetsCollectorManager {
        FacetsCollectorManager::with_overrides(false, Arc::new(AssertingFacetsCollectorManager))
    }
}

impl FacetsCollectorManagerOverrides for AssertingFacetsCollectorManager {
    fn new_collector(&self, _manager: &FacetsCollectorManager) -> Result<FacetsCollector> {
        Ok(AssertingFacetsCollector::create())
    }

    fn reduce(
        &self,
        manager: &FacetsCollectorManager,
        collectors: Vec<FacetsCollector>,
    ) -> Result<FacetsCollector> {
        for fc in &collectors {
            let asserting = fc
                .overrides()
                .and_then(|o| o.as_any().downcast_ref::<AssertingFacetsCollector>());
            java_assert(asserting.is_some())?;
            if let Some(asserting) = asserting {
                java_assert(asserting.has_finished_last_leaf.load(Ordering::SeqCst))?;
            }
        }
        manager.super_reduce(collectors)
    }
}

/// `AssertingFacetsCollector extends FacetsCollector`, with its field
/// `hasFinishedLastLeaf`.
struct AssertingFacetsCollector {
    has_finished_last_leaf: AtomicBool,
}

impl AssertingFacetsCollector {
    /// `new AssertingFacetsCollector()`.
    fn create() -> FacetsCollector {
        FacetsCollector::with_overrides(
            false,
            Arc::new(AssertingFacetsCollector {
                has_finished_last_leaf: AtomicBool::new(true),
            }),
        )
    }
}

impl FacetsCollectorOverrides for AssertingFacetsCollector {
    fn do_set_next_reader(
        &self,
        collector: &mut FacetsCollector,
        context: &LeafReaderContext,
    ) -> Result<()> {
        java_assert(self.has_finished_last_leaf.load(Ordering::SeqCst))?;
        self.has_finished_last_leaf.store(false, Ordering::SeqCst);
        collector.super_do_set_next_reader(context)
    }

    fn finish(&self, collector: &mut FacetsCollector) -> Result<()> {
        self.has_finished_last_leaf.store(true, Ordering::SeqCst);
        collector.super_finish()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The anonymous `DrillSideways` subclass of `testFinishOnAllDimsNoHitsQuery`.
struct AssertingCollectorManagers;

impl DrillSidewaysOverrides for AssertingCollectorManagers {
    fn create_drill_down_facets_collector_manager(&self) -> Option<FacetsCollectorManager> {
        Some(AssertingFacetsCollectorManager::create())
    }

    fn create_drill_sideways_facets_collector_manager(&self) -> FacetsCollectorManager {
        AssertingFacetsCollectorManager::create()
    }
}

fn test_finish_on_all_dims_no_hits_query(_f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let w = random_writer(&dir)?;
    let taxo_w = create_taxo_writer(&taxo_dir)?;
    let result = (|| -> Result<()> {
        let facets_config = Arc::new(FacetsConfig::new());

        let mut d = Document::new();
        d.add(FacetField::new("foo", &["bar"])?);
        add_doc(&w, &facets_config, &taxo_w, &d)?;

        let r = w.get_reader()?;
        let taxo_r = nrt_taxo_reader(&taxo_w)?;
        let body = (|| -> Result<()> {
            let searcher = Arc::new(DrillSidewaysAssertingIndexSearcher::create(
                Arc::clone(&r) as Arc<dyn IndexReader>
            )?);

            // Creating a query that matches nothing to make sure #finish still gets called on all
            // facet collectors:
            let base_query: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
            let mut ddq =
                DrillDownQuery::with_base_query(Arc::clone(&facets_config), Some(base_query));
            ddq.add("foo", &["bar"])?;
            let drill_sideways = DrillSideways::with_overrides(
                searcher,
                Arc::clone(&facets_config),
                Some(Arc::clone(&taxo_r)),
                None,
                None,
                Arc::new(AssertingCollectorManagers),
            );

            let cm = SimpleCollectorManager::new(
                10,
                Arc::new(|a: &CollectedResult, b: &CollectedResult| {
                    a.doc_and_score.doc.cmp(&b.doc_and_score.doc)
                }),
            );
            let result = drill_sideways.search_with_hit_manager(&Arc::new(ddq), cm)?;
            Assert::assert_equals(0, result.collector_result.len());

            // Make sure the "matching docs" are still populated with the appropriate leaf reader
            // context, which happens as part of #finish getting called:
            let drill_down = result
                .base
                .drill_down_facets_collector
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?;
            Assert::assert_equals(1, drill_down.get_matching_docs()?.len());
            Assert::assert_equals(
                1,
                drill_down.get_matching_docs()?[0]
                    .context()
                    .leaf_reader()
                    .max_doc(),
            );
            let sideways = result
                .base
                .drill_sideways_facets_collector
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?;
            Assert::assert_equals(1, sideways.len());
            Assert::assert_equals(1, sideways[0].get_matching_docs()?.len());
            Assert::assert_equals(
                1,
                sideways[0].get_matching_docs()?[0]
                    .context()
                    .leaf_reader()
                    .max_doc(),
            );
            Ok(())
        })();
        let c1 = r.close();
        let c2 = taxo_r.close();
        body?;
        c1?;
        c2
    })();
    let c1 = w.close();
    let c2 = taxo_w.close();
    let c3 = taxo_dir.close();
    let c4 = dir.close();
    result?;
    c1?;
    c2?;
    c3?;
    c4
}

/// `private static class Counters`.
struct Counters {
    counts: Vec<Vec<i32>>,
}

impl Counters {
    fn new(dim_values: &[Vec<String>]) -> Self {
        Counters {
            counts: dim_values.iter().map(|v| vec![0; v.len()]).collect(),
        }
    }

    /// `inc(int[] dims, int[] dims2)`.
    fn inc(&mut self, dims: &[i32], dims2: &[i32]) {
        self.inc_only(dims, dims2, -1);
    }

    /// `inc(int[] dims, int[] dims2, int onlyDim)`.
    fn inc_only(&mut self, dims: &[i32], dims2: &[i32], only_dim: i32) {
        debug_assert!(dims.len() == self.counts.len());
        debug_assert!(dims2.len() == self.counts.len());
        for dim in 0..dims.len() {
            if only_dim == -1 || dim as i32 == only_dim {
                if dims[dim] != -1 {
                    self.counts[dim][dims[dim] as usize] += 1;
                }
                if dims2[dim] != -1 && dims2[dim] != dims[dim] {
                    self.counts[dim][dims2[dim] as usize] += 1;
                }
            }
        }
    }
}

/// `private static class TestFacetResult`.
struct TestFacetResult {
    hits: Vec<Doc>,
    counts: Vec<Vec<i32>>,
    unique_counts: Vec<i32>,
}

/// `private static class DocAndScore`.
#[derive(Clone, Copy, Debug)]
struct DocAndScore {
    doc: i32,
    score: f32,
}

impl DocAndScore {
    fn new(doc: i32, score: f32) -> Self {
        DocAndScore { doc, score }
    }
}

/// `private static class DSResults`.
struct DSResults {
    result_count: TotalHits,
    results: Vec<DocAndScore>,
    facets: Option<Arc<dyn Facets>>,
}

/// `private record CollectedResult(DocAndScore docAndScore, String id)`.
#[derive(Clone, Debug)]
struct CollectedResult {
    doc_and_score: DocAndScore,
    id: Option<String>,
}

impl CollectedResult {
    /// `cr.id` dereferenced, as `Comparator.comparing(cr -> cr.id)` does.
    fn id(&self) -> &str {
        self.id.as_deref().expect("java.lang.NullPointerException")
    }
}

type Comparator = Arc<dyn Fn(&CollectedResult, &CollectedResult) -> CmpOrdering + Send + Sync>;

/// `private static class SimpleCollector implements Collector`.
struct SimpleCollector {
    score_mode: ScoreMode,
    hits: Vec<CollectedResult>,
}

/// The `SimpleLeafCollector` of `SimpleCollector.getLeafCollector`; the
/// port lends the scorer to `collect` instead of keeping it.
struct SimpleLeafCollector<'a> {
    hits: &'a mut Vec<CollectedResult>,
    doc_base: i32,
    id_vals: Box<dyn SortedDocValues>,
}

impl LeafCollector for SimpleLeafCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let score = scorer.score()?;
        let mut id = None;
        if self.id_vals.advance_exact(doc)? {
            let ord = self.id_vals.ord_value()?;
            id = Some(self.id_vals.lookup_ord(ord)?.utf8_to_string()?);
        }
        self.hits.push(CollectedResult {
            doc_and_score: DocAndScore::new(self.doc_base + doc, score),
            id,
        });
        Ok(())
    }
}

impl Collector for SimpleCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let doc_base = context.doc_base();
        let id_vals = DocValues::get_sorted(&*context.leaf_reader(), "id")?;
        Ok(Box::new(SimpleLeafCollector {
            hits: &mut self.hits,
            doc_base,
            id_vals,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        self.score_mode
    }
}

/// `private static class SimpleCollectorManager implements
/// CollectorManager<SimpleCollector, List<DocAndScore>>`.
#[derive(Clone)]
struct SimpleCollectorManager {
    num_docs: usize,
    comparator: Comparator,
    score_mode: ScoreMode,
}

impl SimpleCollectorManager {
    fn new(num_docs: usize, comparator: Comparator) -> Self {
        Self::with_score_mode(num_docs, comparator, ScoreMode::COMPLETE)
    }

    fn with_score_mode(num_docs: usize, comparator: Comparator, score_mode: ScoreMode) -> Self {
        SimpleCollectorManager {
            num_docs,
            comparator,
            score_mode,
        }
    }
}

impl CollectorManager for SimpleCollectorManager {
    type Collector = SimpleCollector;
    type Output = Vec<DocAndScore>;

    fn new_collector(&self) -> Result<SimpleCollector> {
        Ok(SimpleCollector {
            score_mode: self.score_mode,
            hits: Vec::new(),
        })
    }

    fn reduce(&self, collectors: Vec<SimpleCollector>) -> Result<Vec<DocAndScore>> {
        let mut all: Vec<CollectedResult> = Vec::new();
        for c in collectors {
            all.extend(c.hits);
        }

        // Just sorts using the provided comparator returns up to 'numDocs'.
        // This is an inefficient way to go about doing this, but it's only for unit testing.
        all.sort_by(|a, b| (self.comparator)(a, b));
        Ok(all
            .into_iter()
            .map(|cr| cr.doc_and_score)
            .take(self.num_docs)
            .collect())
    }
}

/// The anonymous `InPlaceMergeSorter` of `getTopNOrds`.
struct TopNOrdsSorter<'a> {
    ids: Vec<i32>,
    counts: &'a [i32],
    values: &'a [String],
    pivot_index: usize,
}

impl PivotOps for TopNOrdsSorter<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.ids.swap(i, j);
    }

    /// `Sorter.setPivot(int)`: `pivotIndex = i`.
    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    /// `Sorter.comparePivot(int)`: `compare(pivotIndex, j)`.
    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        let counti = self.counts[self.ids[i] as usize];
        let countj = self.counts[self.ids[j] as usize];
        // Sort by count descending...
        if counti > countj {
            -1
        } else if counti < countj {
            1
        } else {
            // ... then by label ascending:
            match self.values[self.ids[i] as usize]
                .as_bytes()
                .cmp(self.values[self.ids[j] as usize].as_bytes())
            {
                CmpOrdering::Less => -1,
                CmpOrdering::Equal => 0,
                CmpOrdering::Greater => 1,
            }
        }
    }
}

impl Sorter for TopNOrdsSorter<'_> {
    fn sort(&mut self, from: usize, to: usize) {
        self.in_place_merge_sort(from, to);
    }
}

/// `getTopNOrds(int[] counts, String[] values, int topN)`.
fn get_top_n_ords(counts: &[i32], values: &[String], top_n: i32) -> Vec<i32> {
    let ids: Vec<i32> = (0..counts.len() as i32).collect();

    // Naive (on purpose, to reduce bug in tester/gold):
    // sort all ids, then return top N slice:
    let mut sorter = TopNOrdsSorter {
        ids,
        counts,
        values,
        pivot_index: 0,
    };
    let len = sorter.ids.len();
    sorter.sort(0, len);
    let ids = sorter.ids;

    let top_n = (top_n as usize).min(ids.len());

    let mut num_set = top_n;
    for (i, &id) in ids.iter().enumerate().take(top_n) {
        if counts[id as usize] == 0 {
            num_set = i;
            break;
        }
    }

    ids[..num_set].to_vec()
}

fn slow_drill_sideways_search(
    s: &IndexSearcher,
    docs: &[Doc],
    content_token: Option<&str>,
    drill_downs: &[Option<Vec<String>>],
    dim_values: &[Vec<String>],
    only_even: Option<&Arc<dyn Query>>,
) -> Result<TestFacetResult> {
    let num_dims = dim_values.len();

    let mut hits: Vec<Doc> = Vec::new();
    let mut drill_down_counts = Counters::new(dim_values);
    let mut drill_sideways_counts: Vec<Counters> =
        (0..num_dims).map(|_| Counters::new(dim_values)).collect();

    if *VERBOSE {
        println!("  compute expected");
    }

    'next_doc: for doc in docs {
        if doc.deleted {
            continue;
        }
        let id: i32 = doc.id.parse().expect("numeric id");
        if only_even.is_some() & ((id & 1) != 0) {
            continue;
        }
        if content_token.is_none_or(|t| doc.content_token == t) {
            let mut fail_dim: i32 = -1;
            for dim in 0..num_dims {
                if let Some(values) = &drill_downs[dim] {
                    let doc_value = (doc.dims[dim] != -1)
                        .then(|| dim_values[dim][doc.dims[dim] as usize].as_str());
                    let doc_value2 = (doc.dims2[dim] != -1)
                        .then(|| dim_values[dim][doc.dims2[dim] as usize].as_str());
                    let mut matches = false;
                    for value in values {
                        if Some(value.as_str()) == doc_value || Some(value.as_str()) == doc_value2 {
                            matches = true;
                            break;
                        }
                    }
                    if !matches {
                        if fail_dim == -1 {
                            // Doc could be a near-miss, if no other dim fails
                            fail_dim = dim as i32;
                        } else {
                            // Doc isn't a hit nor a near-miss
                            continue 'next_doc;
                        }
                    }
                }
            }

            if fail_dim == -1 {
                if *VERBOSE {
                    println!("    exp: id={} is a hit", doc.id);
                }
                // Hit:
                hits.push(doc.clone());
                drill_down_counts.inc(&doc.dims, &doc.dims2);
                for counters in drill_sideways_counts.iter_mut() {
                    counters.inc(&doc.dims, &doc.dims2);
                }
            } else {
                if *VERBOSE {
                    println!("    exp: id={} is a near-miss on dim={fail_dim}", doc.id);
                }
                drill_sideways_counts[fail_dim as usize].inc_only(&doc.dims, &doc.dims2, fail_dim);
            }
        }
    }

    let mut id_to_doc_id: HashMap<String, i32> = HashMap::new();
    for i in 0..s.get_index_reader().max_doc() {
        id_to_doc_id.insert(
            s.stored_fields()?
                .document(i)?
                .get("id")
                .ok_or_else(LuceneError::null_pointer)?,
            i,
        );
    }

    hits.sort_by(|a, b| java_string_compare(&a.id, &b.id));

    let mut counts = Vec::with_capacity(num_dims);
    let mut unique_counts = Vec::with_capacity(num_dims);
    for dim in 0..num_dims {
        let dim_counts = if drill_downs[dim].is_some() {
            drill_sideways_counts[dim].counts[dim].clone()
        } else {
            drill_down_counts.counts[dim].clone()
        };
        let unique_count = dim_counts.iter().filter(|&&c| c != 0).count() as i32;
        counts.push(dim_counts);
        unique_counts.push(unique_count);
    }

    Ok(TestFacetResult {
        hits,
        counts,
        unique_counts,
    })
}

fn verify_equals(
    dim_values: &[Vec<String>],
    s: &IndexSearcher,
    expected: &TestFacetResult,
    actual: &DSResults,
    scores: &HashMap<String, f32>,
    is_sorted_set_dv: bool,
) -> Result<()> {
    if *VERBOSE {
        println!("  verify totHits={}", expected.hits.len());
    }
    Assert::assert_equals(expected.hits.len() as i64, actual.result_count.value());
    Assert::assert_equals(expected.hits.len(), actual.results.len());
    for (i, hit) in expected.hits.iter().enumerate() {
        if *VERBOSE {
            println!("    hit {i} expected={}", hit.id);
        }
        Assert::assert_equals(
            hit.id.as_str(),
            s.stored_fields()?
                .document(actual.results[i].doc)?
                .get("id")
                .ok_or_else(LuceneError::null_pointer)?
                .as_str(),
        );
        // Score should be IDENTICAL:
        Assert::assert_equals_float_msg(
            None,
            *scores.get(&hit.id).expect("java.lang.NullPointerException"),
            actual.results[i].score,
            0.0f32,
        );
    }

    let random = LuceneTestCase::random();
    for dim in 0..expected.counts.len() {
        let len = dim_values[dim].len() as i32;
        let top_n = if random.next_boolean() {
            len
        } else {
            TestUtil::next_int(&*random, 1, len)
        };
        let fr = facets_of(&actual.facets).get_top_children(top_n, &format!("dim{dim}"), &[])?;
        if *VERBOSE {
            println!("    dim{dim} topN={top_n} (vs {len} unique values)");
            println!("      actual");
        }

        let mut idx = 0;
        let mut actual_values: HashMap<String, i32> = HashMap::new();

        if let Some(fr) = &fr {
            for label_value in &fr.label_values {
                actual_values.insert(
                    label_value.label.clone(),
                    number_int_value(&label_value.value),
                );
                if *VERBOSE {
                    println!(
                        "        {idx}: {}: {}",
                        BytesRef::from_char_sequence(&label_value.label),
                        label_value.value
                    );
                    idx += 1;
                }
            }
            assert_equals_msg(
                Some(&format!("dim={dim}")),
                &expected.unique_counts[dim],
                &fr.child_count,
            );
        }

        if top_n < len {
            let top_n_ids = get_top_n_ords(&expected.counts[dim], &dim_values[dim], top_n);
            if *VERBOSE {
                idx = 0;
                println!("      expected (sorted)");
                for &expected_ord in &top_n_ids {
                    let value = &dim_values[dim][expected_ord as usize];
                    println!(
                        "        {idx}: {}: {}",
                        BytesRef::from_char_sequence(value),
                        expected.counts[dim][expected_ord as usize]
                    );
                    idx += 1;
                }
            }
            if *VERBOSE {
                println!("      topN={top_n} expectedTopN={}", top_n_ids.len());
            }

            match &fr {
                Some(fr) => Assert::assert_equals(top_n_ids.len(), fr.label_values.len()),
                None => Assert::assert_equals(0, top_n_ids.len()),
            }
            for (i, &expected_ord) in top_n_ids.iter().enumerate() {
                let fr = fr.as_ref().expect("java.lang.NullPointerException");
                Assert::assert_equals(
                    expected.counts[dim][expected_ord as usize],
                    number_int_value(&fr.label_values[i].value),
                );
                if is_sorted_set_dv {
                    // Tie-break facet labels are only in unicode
                    // order with SortedSetDVFacets:
                    assert_equals_msg(
                        Some(&format!("value @ idx={i}")),
                        dim_values[dim][expected_ord as usize].as_str(),
                        fr.label_values[i].label.as_str(),
                    );
                }
            }
        } else {
            if *VERBOSE {
                idx = 0;
                println!("      expected (unsorted)");
                for (i, value) in dim_values[dim].iter().enumerate() {
                    if expected.counts[dim][i] != 0 {
                        println!(
                            "        {idx}: {}: {}",
                            BytesRef::from_char_sequence(value),
                            expected.counts[dim][i]
                        );
                        idx += 1;
                    }
                }
            }

            let mut set_count = 0;
            for (i, value) in dim_values[dim].iter().enumerate() {
                if expected.counts[dim][i] != 0 {
                    Assert::assert_true(actual_values.contains_key(value));
                    Assert::assert_equals(expected.counts[dim][i], actual_values[value]);
                    set_count += 1;
                } else {
                    Assert::assert_false(actual_values.contains_key(value));
                }
            }
            Assert::assert_equals(set_count, actual_values.len());
        }
    }
    Ok(())
}

fn test_empty_index(f: &mut Fixture) -> Result<()> {
    // LUCENE-5045: make sure DrillSideways works with an empty index
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;
    let taxo_writer = create_taxo_writer(&taxo_dir)?;
    let r = writer.get_reader()?;
    let searcher = get_new_searcher(&r);
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    // Count "Author"
    let config = Arc::new(FacetsConfig::new());
    let ds = get_new_drill_sideways(f, &searcher, &config, &taxo_reader);
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;

    let r2 = ds.search(ddq.clone_query(), 10)?; // this used to fail on IllegalArgEx
    Assert::assert_equals(0, total_hits(&r2));

    let r2 = ds.search_sorted(
        ddq,
        None,
        None,
        10,
        Some(&Sort::new_fields(vec![SortField::new(
            Some("foo".to_string()),
            SortFieldType::Int,
        )?])?),
        false,
    )?; // this used to fail on IllegalArgEx
    Assert::assert_equals(0, total_hits(&r2));

    writer.close()?;
    taxo_writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_no_drill_down_facet_collection(_f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("Publish Date", true);

    let writer = random_writer(&dir)?;

    index_publications(&writer, &config, &taxo_writer, false)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = get_new_searcher(&r);

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    // Sometimes pass an ExecutorService to test both paths for providing null drill down collectors
    // The actual configuration of the ExecutorService doesn't matter at all:
    let executor_service: Option<Arc<ThreadPoolExecutor>> =
        if LuceneTestCase::random().next_boolean() {
            // `Executors.newSingleThreadExecutor(new NamedThreadFactory("ds_test"))`.
            Some(Arc::new(ThreadPoolExecutor::new(
                1,
                1,
                Duration::ZERO,
                NamedThreadFactory::new("ds_test"),
            )))
        } else {
            None
        };

    // Don't collect for drill down faceting
    let ds = DrillSideways::with_overrides(
        Arc::clone(&searcher),
        Arc::clone(&config),
        Some(Arc::clone(&taxo_reader)),
        None,
        executor_service
            .as_ref()
            .map(|e| Arc::clone(e) as Arc<dyn Executor>),
        Arc::new(NoDrillDownCollector),
    );

    // Another simple case: drill-down on single fields
    // but OR of two values
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Author", &["Bob"])?;
    let r2 = ds.search_after(None, ddq, 10)?;
    let facets = &r2.facets;
    Assert::assert_equals(3, total_hits(&r2));
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        top_string(facets, 10, "Author")?.as_str(),
    );
    // Because we don't collect drill-downs, we shouldn't be able to get counts for Publish Date
    expect_throws(IAE, || {
        facets_of(facets).get_top_children(10, "Publish Date", &[])
    });

    Assert::assert_true(is_multi_facets(facets));
    let all_results = facets_of(facets).get_all_dims(10)?;
    // Should only have the one dimension because we didn't collect for drill down
    Assert::assert_equals(1, all_results.len());
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n  Susan (1)\n  Frank (1)\n",
        all_results[0].to_string().as_str(),
    );

    // test default implementation of getTopDims
    let top_n_dims_result = facets_of(facets).get_top_dims(1, 2)?;
    Assert::assert_equals(1, top_n_dims_result.len());
    Assert::assert_equals(
        "dim=Author path=[] value=5 childCount=4\n  Lisa (2)\n  Bob (1)\n",
        top_n_dims_result[0].to_string().as_str(),
    );

    // test getAllDims(0)
    expect_throws(IAE, || facets_of(facets).get_all_dims(0));
    // More interesting case: drill-down on two fields
    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("Author", &["Lisa"])?;
    ddq.add("Publish Date", &["2010"])?;
    let r2 = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(1, total_hits(&r2));
    // Should be able to count on both fields since they're both drill sideways cases
    Assert::assert_equals(
        "dim=Publish Date path=[] value=2 childCount=2\n  2010 (1)\n  2012 (1)\n",
        top_string(&r2.facets, 10, "Publish Date")?.as_str(),
    );
    Assert::assert_equals(
        "dim=Author path=[] value=2 childCount=2\n  Bob (1)\n  Lisa (1)\n",
        top_string(&r2.facets, 10, "Author")?.as_str(),
    );

    if let Some(executor_service) = &executor_service {
        rucene_test_framework::jdk::executor_service::ExecutorService::shutdown(
            &**executor_service,
        );
    }
    writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_scorer(f: &mut Fixture) -> Result<()> {
    // LUCENE-6001 some scorers, e.g. ReqExlScorer, can hit NPE if cost is called after nextDoc
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    // Writes facet ords to a separate directory from the
    // main index:
    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());

    let writer = random_writer(&dir)?;

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field(
        "field",
        "foo bar",
        Store::NO,
    ));
    doc.add(FacetField::new("Author", &["Bob"])?);
    doc.add(FacetField::new("dim", &["a"])?);
    add_doc(&writer, &config, &taxo_writer, &doc)?;

    // NRT open
    let r = writer.get_reader()?;
    let searcher = get_new_searcher(&r);

    // NRT open
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let ds = get_new_drill_sideways(f, &searcher, &config, &taxo_reader);

    let mut bq = BooleanQueryBuilder::new();
    bq.add(
        Arc::new(TermQuery::new(Term::from_text("field", "foo"))),
        Occur::MUST,
    )?;
    bq.add(
        Arc::new(TermQuery::new(Term::from_text("field", "bar"))),
        Occur::MUST_NOT,
    )?;
    let mut ddq = DrillDownQuery::with_base_query(Arc::clone(&config), Some(Arc::new(bq.build())));
    ddq.add("field", &["foo"])?;
    ddq.add_query("author", Arc::new(bq.build()))?;
    ddq.add_query("dim", Arc::new(bq.build()))?;
    let r2 = ds.search_after(None, ddq, 10)?;
    Assert::assert_equals(0, total_hits(&r2));

    writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

// The facet counts hold non-`Send` readers, shared as Java shares them.
#[allow(clippy::arc_with_non_send_sync)]
fn test_extended_drill_sideways_result(f: &mut Fixture) -> Result<()> {
    // LUCENE-9945: Extend DrillSideways to support exposing FacetCollectors directly
    let dir = LuceneTestCase::new_directory();
    let taxo_dir = LuceneTestCase::new_directory();

    let writer = random_writer(&dir)?;

    let taxo_writer = create_taxo_writer(&taxo_dir)?;

    let config = Arc::new(FacetsConfig::new());
    config.set_hierarchical("dim", true);

    let mut doc = Document::new();
    doc.add(FacetField::new("dim", &["a"])?);
    add_doc(&writer, &config, &taxo_writer, &doc)?;

    let doc2 = Document::new();
    doc.add(FacetField::new("dim", &["x"])?);
    add_doc(&writer, &config, &taxo_writer, &doc2)?;

    // open NRT
    let r = writer.get_reader()?;
    let searcher = get_new_searcher(&r);
    let taxo_reader = nrt_taxo_reader(&taxo_writer)?;

    let mut ddq = DrillDownQuery::new(Arc::clone(&config));
    ddq.add("dim", &["x"])?;

    let ds = get_new_drill_sideways_build_facets_result(f, &searcher, &config, &taxo_reader);

    let manager = SimpleCollectorManager::new(
        10,
        Arc::new(|a: &CollectedResult, b: &CollectedResult| {
            java_float_compare(b.doc_and_score.score, a.doc_and_score.score)
        }),
    );

    let r2 = ds.search_with_hit_manager(&Arc::new(ddq), manager)?.base;

    // compute Facets using exposed FacetCollectors from DrillSidewaysResult
    let mut drill_sideways_facets: JavaHashMap<Arc<dyn Facets>> = JavaHashMap::new();
    let drill_down_facets = get_taxonomy_facet_counts(
        &taxo_reader,
        &config,
        r2.drill_down_facets_collector
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?,
    )?;
    if let Some(collectors) = &r2.drill_sideways_facets_collector {
        let dims = r2
            .drill_sideways_dims
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?;
        for (i, collector) in collectors.iter().enumerate() {
            drill_sideways_facets.insert(
                dims[i].clone(),
                get_taxonomy_facet_counts(&taxo_reader, &config, collector)?,
            );
        }
    }

    let facets: Arc<dyn Facets> = if drill_sideways_facets.is_empty() {
        drill_down_facets
    } else {
        let by_dim: Vec<(String, Arc<dyn Facets>)> = drill_sideways_facets
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();
        Arc::new(MultiFacets::with_default(by_dim, Some(drill_down_facets)))
    };

    // Facets computed using FacetsCollector exposed in DrillSidewaysResult
    // should match the Facets computed by {@link DrillSideways#buildFacetsResult}
    let facet_result_actual = facets
        .get_top_children(2, "dim", &[])?
        .ok_or_else(LuceneError::null_pointer)?;
    let facet_result_expected = facets_of(&r2.facets)
        .get_top_children(2, "dim", &[])?
        .ok_or_else(LuceneError::null_pointer)?;

    Assert::assert_equals(
        facet_result_expected.dim.as_str(),
        facet_result_actual.dim.as_str(),
    );
    Assert::assert_equals(
        facet_result_expected.path.len(),
        facet_result_actual.path.len(),
    );
    assert_equals(&facet_result_expected.value, &facet_result_actual.value);
    Assert::assert_equals(
        facet_result_expected.child_count,
        facet_result_actual.child_count,
    );

    writer.close()?;
    r.close()?;
    taxo_reader.close()?;
    taxo_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}

fn test_drill_sideways_search_use_correct_iterator(_f: &mut Fixture) -> Result<()> {
    // This test reproduces an issue (see GitHub #12211) where DrillSidewaysScorer would ultimately
    // cause multiple consecutive calls to TwoPhaseIterator::matches, which results in a failed
    // assert in the PostingsReaderBase implementation (or a failing to match a document that should
    // have matched, if asserts are disabled).
    let dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(StandardAnalyzer::new())));
    let index_writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let taxo_dir = LuceneTestCase::new_directory();
    let taxonomy_writer = DirectoryTaxonomyWriter::new(taxo_dir.as_directory())?;
    let facets_config = Arc::new(FacetsConfig::new());
    facets_config.set_require_dim_count("dim1", true);
    facets_config.set_drill_down_terms_indexing("dim1", DrillDownTermsIndexing::ALL);
    // Add a doc that we'll try to match
    let mut doc = Document::new();
    doc.add(TextField::new(
        "content",
        "bt tv v1 1b b1 10 04 40 08 81 14 48".to_string(),
        Store::NO,
    )?);
    doc.add(FacetField::new("dim1", &["dim1"])?);
    index_writer.add_document(&facets_config.build_with_taxonomy(Some(&taxonomy_writer), &doc)?)?;
    // Add some more docs as filler in the index
    for _ in 0..25 {
        let mut filler_doc = Document::new();
        filler_doc.add(TextField::new("content", "content".to_string(), Store::NO)?);
        filler_doc.add(FacetField::new("dim1", &["dim1"])?);
        index_writer.add_document(
            &facets_config.build_with_taxonomy(Some(&taxonomy_writer), &filler_doc)?,
        )?;
    }
    taxonomy_writer.commit()?;
    index_writer.commit()?;
    let taxonomy_reader: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(taxo_dir.as_directory())?);
    let index_reader = directory_reader::open_with_writer(&*index_writer)?;
    let searcher = Arc::new(IndexSearcher::new(
        Arc::clone(&index_reader) as Arc<dyn IndexReader>
    )?);
    let drill = DrillSideways::new(
        searcher,
        Arc::clone(&facets_config),
        Arc::clone(&taxonomy_reader),
    );
    let mut drill_down_query = DrillDownQuery::with_base_query(
        Arc::clone(&facets_config),
        Some(Arc::new(PhraseQuery::from_strings(
            "content",
            &[
                "bt", "tv", "v1", "1b", "b1", "10", "04", "40", "08", "81", "14", "48",
            ],
        )?)),
    );
    drill_down_query.add("dim1", &["dim1"])?;
    let result = drill.search(drill_down_query, 99)?;
    // We expect to match exactly one document from the query above
    Assert::assert_equals(1, total_hits(&result));

    index_reader.close()?;
    taxonomy_reader.close()?;
    taxonomy_writer.close()?;
    index_writer.close()?;
    dir.close()?;
    taxo_dir.close()
}
