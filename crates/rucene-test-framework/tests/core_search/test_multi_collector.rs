//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiCollector.java`.
//!
//! Java's tests keep references to the collectors they hand to
//! `MultiCollector.wrap` and read their fields afterwards; a Rust collector is
//! moved into the wrapper, so each test collector keeps the fields Java reads
//! in shared cells (`Arc` of atomics) that the test holds as well.

use std::any::TypeId;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::collection_terminated_exception::CollectionError;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::multi_collector::{MinCompetitiveScoreAwareScorable, MultiCollector};
use rucene::search::scorable::{FilterScorable, Scorable, SimpleScorable};
use rucene::search::score_caching_wrapping_scorer::ScoreCachingWrappingScorer;
use rucene::search::{CollectionResult, Collector, CollectorManager, LeafCollector, ScoreMode};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::null_leaf_context;
use crate::core_support::{
    assert_equals, assert_equals_delta, class, expect_throws_panic, random, shuffle, test,
};

/// `TestMultiCollector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMultiCollector",
        vec![
            test(
                "testCollectionTerminatedExceptionHandling",
                test_collection_terminated_exception_handling,
            ),
            test(
                "testSetScorerAfterCollectionTerminated",
                test_set_scorer_after_collection_terminated,
            ),
            test("testDisablesSetMinScore", test_disables_set_min_score),
            test(
                "testDisablesSetMinScoreWithEarlyTermination",
                test_disables_set_min_score_with_early_termination,
            ),
            test("testNullCollectors", test_null_collectors),
            test("testSingleCollector", test_single_collector),
            test("testCollector", test_collector),
            test("testCacheScoresIfNecessary", test_cache_scores_if_necessary),
            test(
                "testScorerWrappingForTopScores",
                test_scorer_wrapping_for_top_scores,
            ),
            test("testMinCompetitiveScore", test_min_competitive_score),
            test("testCollectionTermination", test_collection_termination),
            test(
                "testSetScorerOnCollectionTerminationSkipNonCompetitive",
                test_set_scorer_on_collection_termination_skip_non_competitive,
            ),
            test(
                "testSetScorerOnCollectionTerminationSkipNoSkips",
                test_set_scorer_on_collection_termination_skip_no_skips,
            ),
            test("testMergeScoreModes", test_merge_score_modes),
        ],
    )
}

const SCORE_MODE_VALUES: [ScoreMode; 5] = [
    ScoreMode::COMPLETE,
    ScoreMode::COMPLETE_NO_SCORES,
    ScoreMode::TOP_SCORES,
    ScoreMode::TOP_DOCS,
    ScoreMode::TOP_DOCS_WITH_SCORES,
];

/// `throw new CollectionTerminatedException()`.
fn terminated<T>() -> CollectionResult<T> {
    Err(CollectionError::CollectionTerminated)
}

/// `expectThrows(CollectionTerminatedException.class, () -> body)`.
fn expect_terminated(r: CollectionResult<()>) {
    match r {
        Err(CollectionError::CollectionTerminated) => {}
        Err(e) => rucene_test_framework::util::java_exceptions::rethrow_java(e.into_lucene_error()),
        Ok(()) => Assert::fail(
            "Expected exception CollectionTerminatedException but no exception was thrown",
        ),
    }
}

/// `new DummyTotalHitCountCollector()` whose `totalHits` the test reads
/// after the collector was moved into a `MultiCollector`: the same counting,
/// over a shared counter.
#[derive(Debug, Default, Clone)]
struct SharedTotalHitCountCollector {
    total_hits: Arc<AtomicI32>,
}

struct SharedTotalHitCountLeaf {
    total_hits: Arc<AtomicI32>,
}

impl LeafCollector for SharedTotalHitCountLeaf {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.total_hits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Collector for SharedTotalHitCountCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(SharedTotalHitCountLeaf {
            total_hits: Arc::clone(&self.total_hits),
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

/// `private static class TerminateAfterCollector extends FilterCollector`.
///
/// `count` is shared by the copies of the collector that the collector
/// manager of `testCollectionTerminatedExceptionHandling` hands out, as
/// Java's manager hands out the same instance.
struct TerminateAfterCollector<C: Collector = Box<dyn Collector>> {
    r#in: C,
    count: Arc<AtomicI32>,
    terminate_after: i32,
}

impl<C: Collector> TerminateAfterCollector<C> {
    fn new(r#in: C, terminate_after: i32) -> Self {
        TerminateAfterCollector {
            r#in,
            count: Arc::new(AtomicI32::new(0)),
            terminate_after,
        }
    }
}

impl<C: Collector> std::fmt::Debug for TerminateAfterCollector<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TerminateAfterCollector")
    }
}

/// The anonymous `FilterLeafCollector` of `TerminateAfterCollector`.
struct TerminateAfterLeaf<'a> {
    r#in: Box<dyn LeafCollector + 'a>,
    count: Arc<AtomicI32>,
    terminate_after: i32,
}

impl LeafCollector for TerminateAfterLeaf<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.r#in.set_scorer(scorer)
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        if self.count.load(Ordering::SeqCst) >= self.terminate_after {
            return terminated();
        }
        self.r#in.collect(doc, scorer)?;
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.r#in.finish()
    }
}

impl<C: Collector> Collector for TerminateAfterCollector<C> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        if self.count.load(Ordering::SeqCst) >= self.terminate_after {
            return terminated();
        }
        let r#in = self.r#in.get_leaf_collector(context)?;
        Ok(Box::new(TerminateAfterLeaf {
            r#in,
            count: Arc::clone(&self.count),
            terminate_after: self.terminate_after,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        self.r#in.score_mode()
    }
}

/// `private static class SetScorerCollector extends FilterCollector`.
struct SetScorerCollector {
    r#in: Box<dyn Collector>,
    set_scorer_called: Arc<AtomicBool>,
}

/// The anonymous `FilterLeafCollector` of `SetScorerCollector`.
struct SetScorerLeaf<'a> {
    r#in: Box<dyn LeafCollector + 'a>,
    set_scorer_called: Arc<AtomicBool>,
}

impl LeafCollector for SetScorerLeaf<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.r#in.set_scorer(scorer)?;
        self.set_scorer_called.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.r#in.collect(doc, scorer)
    }

    fn finish(&mut self) -> Result<()> {
        self.r#in.finish()
    }
}

impl std::fmt::Debug for SetScorerCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SetScorerCollector")
    }
}

impl Collector for SetScorerCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(SetScorerLeaf {
            r#in: self.r#in.get_leaf_collector(context)?,
            set_scorer_called: Arc::clone(&self.set_scorer_called),
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        self.r#in.score_mode()
    }
}

/// A `TerminateAfterCollector` over a [`SharedTotalHitCountCollector`] with
/// shared `count`: the one instance Java's collector manager hands out.
#[derive(Clone)]
struct SharedTerminateAfter {
    collector: SharedTotalHitCountCollector,
    count: Arc<AtomicI32>,
    terminate_after: i32,
}

impl SharedTerminateAfter {
    fn instance(&self) -> TerminateAfterCollector<SharedTotalHitCountCollector> {
        TerminateAfterCollector {
            r#in: self.collector.clone(),
            count: Arc::clone(&self.count),
            terminate_after: self.terminate_after,
        }
    }
}

/// `MultiCollector.wrap(collectors)` over the collectors of
/// `testCollectionTerminatedExceptionHandling`.
///
/// A search collector manager hands its collectors to the slices' threads, so
/// they must be `Send`; [`MultiCollector::wrap`] erases them to
/// `Box<dyn Collector>`, which is not. The manager reproduces `wrap` over the
/// concrete collector type instead: the single collector itself when there is
/// one, a [`MultiCollector`] over all of them otherwise (none is `null`).
#[derive(Debug)]
enum Wrapped {
    Single(TerminateAfterCollector<SharedTotalHitCountCollector>),
    Multi(MultiCollector<TerminateAfterCollector<SharedTotalHitCountCollector>>),
}

impl Collector for Wrapped {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        match self {
            Wrapped::Single(c) => c.get_leaf_collector(context),
            Wrapped::Multi(c) => c.get_leaf_collector(context),
        }
    }

    fn score_mode(&self) -> ScoreMode {
        match self {
            Wrapped::Single(c) => c.score_mode(),
            Wrapped::Multi(c) => c.score_mode(),
        }
    }
}

/// The anonymous `CollectorManager<Collector, Void>` of
/// `testCollectionTerminatedExceptionHandling`.
struct WrapManager {
    collectors: Vec<SharedTerminateAfter>,
}

impl CollectorManager for WrapManager {
    type Collector = Wrapped;
    type Output = ();

    fn new_collector(&self) -> Result<Wrapped> {
        let mut collectors: Vec<_> = self.collectors.iter().map(|c| c.instance()).collect();
        Ok(if collectors.len() == 1 {
            Wrapped::Single(collectors.pop().expect("one collector"))
        } else {
            Wrapped::Multi(MultiCollector::new(collectors))
        })
    }

    fn reduce(&self, _collectors: Vec<Wrapped>) -> Result<()> {
        Ok(())
    }
}

/// Port of `TestMultiCollector.testCollectionTerminatedExceptionHandling`.
fn test_collection_terminated_exception_handling() -> Result<()> {
    let iters = LuceneTestCase::at_least(3);
    for _ in 0..iters {
        let dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let num_docs = TestUtil::next_int(&*random(), 100, 1000);
        let doc = Document::new();
        for _ in 0..num_docs {
            w.add_document(&doc)?;
        }
        let reader = w.get_reader()?;
        w.close()?;
        let searcher = LuceneTestCase::new_searcher_threads(Arc::clone(&reader), true, true, false);
        let mut expected_counts: Vec<(Arc<AtomicI32>, i32)> = Vec::new();
        let mut collectors = Vec::new();
        let num_collectors = TestUtil::next_int(&*random(), 1, 5);
        for _ in 0..num_collectors {
            let terminate_after = random().next_int_bound(num_docs + 10);
            let expected_count = if terminate_after > num_docs {
                num_docs
            } else {
                terminate_after
            };
            let collector = SharedTotalHitCountCollector::default();
            expected_counts.push((Arc::clone(&collector.total_hits), expected_count));
            collectors.push(SharedTerminateAfter {
                collector,
                count: Arc::new(AtomicI32::new(0)),
                terminate_after,
            });
        }
        searcher.search_with_manager(
            Arc::new(MatchAllDocsQuery::instance()),
            &WrapManager { collectors },
        )?;
        for (total_hits, expected_count) in &expected_counts {
            assert_equals(expected_count, &total_hits.load(Ordering::SeqCst));
        }
        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestMultiCollector.testSetScorerAfterCollectionTerminated`.
fn test_set_scorer_after_collection_terminated() -> Result<()> {
    let collector1: Box<dyn Collector> = Box::new(SharedTotalHitCountCollector::default());
    let collector2: Box<dyn Collector> = Box::new(SharedTotalHitCountCollector::default());

    let set_scorer_called1 = Arc::new(AtomicBool::new(false));
    let collector1: Box<dyn Collector> = Box::new(SetScorerCollector {
        r#in: collector1,
        set_scorer_called: Arc::clone(&set_scorer_called1),
    });

    let set_scorer_called2 = Arc::new(AtomicBool::new(false));
    let collector2: Box<dyn Collector> = Box::new(SetScorerCollector {
        r#in: collector2,
        set_scorer_called: Arc::clone(&set_scorer_called2),
    });

    let collector1: Box<dyn Collector> = Box::new(TerminateAfterCollector::new(collector1, 1));
    let collector2: Box<dyn Collector> = Box::new(TerminateAfterCollector::new(collector2, 2));

    let mut scorer = SimpleScorable::new();

    let mut collectors = vec![collector1, collector2];
    shuffle(&mut collectors, &*random());
    let mut collector = MultiCollector::wrap(collectors.into_iter().map(Some))?;

    let (_null_reader, null_leaves) = null_leaf_context()?;
    let mut leaf_collector = collector
        .get_leaf_collector(&null_leaves[0])
        .map_err(|e| e.into_lucene_error())?;
    leaf_collector.set_scorer(&mut scorer)?;
    Assert::assert_true(set_scorer_called1.load(Ordering::SeqCst));
    Assert::assert_true(set_scorer_called2.load(Ordering::SeqCst));

    leaf_collector
        .collect(0, &mut scorer)
        .map_err(|e| e.into_lucene_error())?;
    leaf_collector
        .collect(1, &mut scorer)
        .map_err(|e| e.into_lucene_error())?;

    set_scorer_called1.store(false, Ordering::SeqCst);
    set_scorer_called2.store(false, Ordering::SeqCst);
    leaf_collector.set_scorer(&mut scorer)?;
    Assert::assert_false(set_scorer_called1.load(Ordering::SeqCst));
    Assert::assert_true(set_scorer_called2.load(Ordering::SeqCst));

    expect_terminated(leaf_collector.collect(1, &mut scorer));

    set_scorer_called1.store(false, Ordering::SeqCst);
    set_scorer_called2.store(false, Ordering::SeqCst);
    leaf_collector.set_scorer(&mut scorer)?;
    Assert::assert_false(set_scorer_called1.load(Ordering::SeqCst));
    Assert::assert_false(set_scorer_called2.load(Ordering::SeqCst));
    Ok(())
}

/// The anonymous `Scorable` of `testDisablesSetMinScore*`.
#[derive(Debug)]
struct NoSetMinScorable;

impl Scorable for NoSetMinScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }

    fn set_min_competitive_score(&mut self, _min_score: f32) -> Result<()> {
        randomizedtesting::jdk::exception::Throwable::new("java.lang.AssertionError", "").throw()
    }
}

/// The anonymous `SimpleCollector` of `testDisablesSetMinScore*`: shared
/// `minScore`, as the tests reuse the one instance.
#[derive(Debug, Clone, Default)]
struct RaisingMinScoreCollector {
    min_score: Arc<std::sync::atomic::AtomicU32>,
}

struct RaisingMinScoreLeaf {
    min_score: Arc<std::sync::atomic::AtomicU32>,
}

impl LeafCollector for RaisingMinScoreLeaf {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let min_score = f32::from_bits(self.min_score.load(Ordering::SeqCst)).next_up();
        self.min_score.store(min_score.to_bits(), Ordering::SeqCst);
        scorer.set_min_competitive_score(min_score)?;
        Ok(())
    }
}

impl Collector for RaisingMinScoreCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(RaisingMinScoreLeaf {
            min_score: Arc::clone(&self.min_score),
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::TOP_SCORES
    }
}

/// A one-document index opened from its writer.
fn one_doc_index() -> Result<(
    rucene_test_framework::util::lucene_test_case::TestDirectory,
    Arc<dyn rucene::index::directory_reader::DirectoryReader>,
)> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    w.add_document(&Document::new())?;
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    Ok((dir, reader))
}

/// `reader.leaves().get(0)`.
fn first_leaf(
    reader: &Arc<dyn rucene::index::directory_reader::DirectoryReader>,
) -> Result<Arc<LeafReaderContext>> {
    Ok(Arc::clone(
        &(Arc::clone(reader) as Arc<dyn IndexReader>).leaves()?[0],
    ))
}

/// Port of `TestMultiCollector.testDisablesSetMinScore`.
fn test_disables_set_min_score() -> Result<()> {
    let (dir, reader) = one_doc_index()?;

    let mut scorer = NoSetMinScorable;

    let collector: Box<dyn Collector> = Box::new(RaisingMinScoreCollector::default());
    let mut multi_collector = MultiCollector::wrap([
        Some(collector),
        Some(Box::new(SharedTotalHitCountCollector::default()) as Box<dyn Collector>),
    ])?;
    let ctx = first_leaf(&reader)?;
    let mut leaf_collector = multi_collector
        .get_leaf_collector(&ctx)
        .map_err(|e| e.into_lucene_error())?;
    leaf_collector.set_scorer(&mut scorer)?;
    leaf_collector
        .collect(0, &mut scorer)
        .map_err(|e| e.into_lucene_error())?; // no exception

    drop(leaf_collector);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestMultiCollector.testDisablesSetMinScoreWithEarlyTermination`.
fn test_disables_set_min_score_with_early_termination() -> Result<()> {
    let (dir, reader) = one_doc_index()?;

    let mut scorer = NoSetMinScorable;

    let collector = RaisingMinScoreCollector::default();
    for num_col in 1..4 {
        let mut cols: Vec<Box<dyn Collector>> = vec![Box::new(collector.clone())];
        for _ in 0..num_col {
            cols.push(Box::new(TerminateAfterCollector::new(
                SharedTotalHitCountCollector::default(),
                0,
            )));
        }
        shuffle(&mut cols, &*random());
        let mut multi_collector = MultiCollector::wrap(cols.into_iter().map(Some))?;
        let ctx = first_leaf(&reader)?;
        let mut leaf_collector = multi_collector
            .get_leaf_collector(&ctx)
            .map_err(|e| e.into_lucene_error())?;
        leaf_collector.set_scorer(&mut scorer)?;
        leaf_collector
            .collect(0, &mut scorer)
            .map_err(|e| e.into_lucene_error())?; // no exception
    }

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// The fields of a `DummyCollector` the tests read and reset.
#[derive(Debug, Default)]
struct DummyFlags {
    collect_called: AtomicBool,
    set_next_reader_called: AtomicBool,
    set_scorer_called: AtomicBool,
}

impl DummyFlags {
    fn collect_called(&self) -> bool {
        self.collect_called.load(Ordering::SeqCst)
    }
    fn set_scorer_called(&self) -> bool {
        self.set_scorer_called.load(Ordering::SeqCst)
    }
}

/// `private static class DummyCollector extends SimpleCollector`, and its
/// subclass `TerminatingDummyCollector` when `terminate_on_doc` is set.
#[derive(Debug)]
struct DummyCollector {
    flags: Arc<DummyFlags>,
    /// `TerminatingDummyCollector.terminateOnDoc`.
    terminate_on_doc: Option<i32>,
    score_mode: ScoreMode,
}

impl DummyCollector {
    /// `new DummyCollector()`.
    fn new() -> Self {
        DummyCollector {
            flags: Arc::default(),
            terminate_on_doc: None,
            score_mode: ScoreMode::COMPLETE,
        }
    }

    /// `new TerminatingDummyCollector(terminateOnDoc, scoreMode)`.
    fn terminating(terminate_on_doc: i32, score_mode: ScoreMode) -> Self {
        DummyCollector {
            flags: Arc::default(),
            terminate_on_doc: Some(terminate_on_doc),
            score_mode,
        }
    }
}

struct DummyLeaf {
    flags: Arc<DummyFlags>,
    terminate_on_doc: Option<i32>,
}

impl LeafCollector for DummyLeaf {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        self.flags.set_scorer_called.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        if self.terminate_on_doc == Some(doc) {
            return terminated();
        }
        self.flags.collect_called.store(true, Ordering::SeqCst);
        Ok(())
    }
}

impl Collector for DummyCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // doSetNextReader
        self.flags
            .set_next_reader_called
            .store(true, Ordering::SeqCst);
        Ok(Box::new(DummyLeaf {
            flags: Arc::clone(&self.flags),
            terminate_on_doc: self.terminate_on_doc,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        self.score_mode
    }
}

/// Port of `TestMultiCollector.testNullCollectors`.
fn test_null_collectors() -> Result<()> {
    // Tests that the collector rejects all null collectors.
    expect_throws("java.lang.IllegalArgumentException", || {
        MultiCollector::wrap([None, None])
    });

    // Tests that the collector handles some null collectors well. If it
    // doesn't, an NPE would be thrown.
    let mut c = MultiCollector::wrap([
        Some(Box::new(DummyCollector::new()) as Box<dyn Collector>),
        None,
        Some(Box::new(DummyCollector::new()) as Box<dyn Collector>),
    ])?;
    Assert::assert_true(c.as_any().is_some_and(|a| a.is::<MultiCollector>()));
    let (_null_reader, null_leaves) = null_leaf_context()?;
    let null = &null_leaves[0];
    {
        let mut ac = c
            .get_leaf_collector(null)
            .map_err(|e| e.into_lucene_error())?;
        ac.collect(1, &mut SimpleScorable::new())
            .map_err(|e| e.into_lucene_error())?;
    }
    c.get_leaf_collector(null)
        .map_err(|e| e.into_lucene_error())?;
    c.get_leaf_collector(null)
        .map_err(|e| e.into_lucene_error())?
        .set_scorer(&mut SimpleScorable::new())?;
    Ok(())
}

/// `assertSame(expected, actual)` of a boxed collector: the same allocation.
fn assert_same_collector(expected: *const (), actual: &dyn Collector) {
    if expected != actual as *const dyn Collector as *const () {
        Assert::fail("expected same collector");
    }
}

/// Port of `TestMultiCollector.testSingleCollector`.
fn test_single_collector() -> Result<()> {
    // Tests that if a single Collector is input, it is returned (and not
    // MultiCollector).
    let dc: Box<dyn Collector> = Box::new(DummyCollector::new());
    let dc_ptr = &*dc as *const dyn Collector as *const ();
    let dc = MultiCollector::wrap([Some(dc)])?;
    assert_same_collector(dc_ptr, &*dc);
    let dc = MultiCollector::wrap([Some(dc), None])?;
    assert_same_collector(dc_ptr, &*dc);
    Ok(())
}

/// Port of `TestMultiCollector.testCollector`.
fn test_collector() -> Result<()> {
    // Tests that the collector delegates calls to input collectors properly.

    // Tests that the collector handles some null collectors well. If it
    // doesn't, an NPE would be thrown.
    let dcs = [DummyCollector::new(), DummyCollector::new()];
    let flags: Vec<Arc<DummyFlags>> = dcs.iter().map(|dc| Arc::clone(&dc.flags)).collect();
    let mut c = MultiCollector::wrap(
        dcs.into_iter()
            .map(|dc| Some(Box::new(dc) as Box<dyn Collector>)),
    )?;
    let (_null_reader, null_leaves) = null_leaf_context()?;
    let null = &null_leaves[0];
    {
        let mut ac = c
            .get_leaf_collector(null)
            .map_err(|e| e.into_lucene_error())?;
        ac.collect(1, &mut SimpleScorable::new())
            .map_err(|e| e.into_lucene_error())?;
    }
    {
        let mut ac = c
            .get_leaf_collector(null)
            .map_err(|e| e.into_lucene_error())?;
        ac.set_scorer(&mut SimpleScorable::new())?;
    }

    for dc in &flags {
        Assert::assert_true(dc.collect_called());
        Assert::assert_true(dc.set_next_reader_called.load(Ordering::SeqCst));
        Assert::assert_true(dc.set_scorer_called());
    }
    Ok(())
}

/// The Java classes the scorable checks of `collector(ScoreMode, Class)`
/// compare.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScorableClass {
    SimpleScorable,
    ScoreCachingWrappingScorer,
    MinCompetitiveScoreAwareScorable,
}

impl ScorableClass {
    fn type_id(self) -> TypeId {
        match self {
            ScorableClass::SimpleScorable => TypeId::of::<SimpleScorable>(),
            ScorableClass::ScoreCachingWrappingScorer => {
                TypeId::of::<ScoreCachingWrappingScorer<'static>>()
            }
            ScorableClass::MinCompetitiveScoreAwareScorable => {
                TypeId::of::<MinCompetitiveScoreAwareScorable<'static>>()
            }
        }
    }
}

/// `scorer.getClass()`, rendered as Java's `Class.toString()`.
fn class_name(id: Option<TypeId>) -> String {
    for (class, name) in [
        (ScorableClass::SimpleScorable, "SimpleScorable"),
        (
            ScorableClass::ScoreCachingWrappingScorer,
            "ScoreCachingWrappingScorer",
        ),
        (
            ScorableClass::MinCompetitiveScoreAwareScorable,
            "MultiCollector$MinCompetitiveScoreAwareScorable",
        ),
    ] {
        if id == Some(class.type_id()) {
            return format!("class org.apache.lucene.search.{name}");
        }
    }
    if id == Some(TypeId::of::<FilterScorable<'static>>()) {
        return "class org.apache.lucene.search.FilterScorable".to_string();
    }
    format!("{id:?}")
}

/// `scorer instanceof FilterScorable`.
fn is_filter_scorable(id: Option<TypeId>) -> bool {
    id == Some(TypeId::of::<FilterScorable<'static>>())
        || id == Some(ScorableClass::MinCompetitiveScoreAwareScorable.type_id())
}

/// `while (expectedScorer.equals(scorer.getClass()) == false && scorer
/// instanceof FilterScorable) scorer = ((FilterScorable) scorer).in;
/// assertEquals(expectedScorer, scorer.getClass());`.
fn assert_scorer_class(scorer: &mut dyn Scorable, expected: ScorableClass) -> Result<()> {
    let id = scorer.class_id();
    if id != Some(expected.type_id()) && is_filter_scorable(id) {
        let mut children = scorer.children()?;
        let r#in = children.remove(0);
        return assert_scorer_class(r#in.child, expected);
    }
    if id != Some(expected.type_id()) {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            class_name(Some(expected.type_id())),
            class_name(id)
        ));
    }
    Ok(())
}

/// `private static Collector collector(ScoreMode scoreMode, Class<?> expectedScorer)`.
#[derive(Debug)]
pub struct ClassCheckingCollector {
    score_mode: ScoreMode,
    expected_scorer: ScorableClass,
}

impl ClassCheckingCollector {
    /// The collector `collector(scoreMode, expectedScorer)` returns
    /// (`TestMultiCollectorManager.collectorManager` builds the same one).
    pub fn new(score_mode: ScoreMode, expected_scorer: ScorableClass) -> Self {
        ClassCheckingCollector {
            score_mode,
            expected_scorer,
        }
    }
}

struct ClassCheckingLeaf {
    expected_scorer: ScorableClass,
}

impl LeafCollector for ClassCheckingLeaf {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        assert_scorer_class(scorer, self.expected_scorer)
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

impl Collector for ClassCheckingCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(ClassCheckingLeaf {
            expected_scorer: self.expected_scorer,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        self.score_mode
    }
}

fn collector(score_mode: ScoreMode, expected_scorer: ScorableClass) -> Box<dyn Collector> {
    Box::new(ClassCheckingCollector {
        score_mode,
        expected_scorer,
    })
}

/// `MultiCollector.wrap(c1, c2).getLeafCollector(ctx).setScorer(new SimpleScorable())`.
fn wrap_and_set_scorer(
    c1: Box<dyn Collector>,
    c2: Box<dyn Collector>,
    ctx: &LeafReaderContext,
) -> Result<()> {
    let mut mc = MultiCollector::wrap([Some(c1), Some(c2)])?;
    let mut lc = mc
        .get_leaf_collector(ctx)
        .map_err(|e| e.into_lucene_error())?;
    lc.set_scorer(&mut SimpleScorable::new())
}

/// A one-document index opened from a `RandomIndexWriter`.
fn random_one_doc_index(
    commit: bool,
) -> Result<(
    rucene_test_framework::util::lucene_test_case::TestDirectory,
    Arc<dyn rucene::index::directory_reader::DirectoryReader>,
)> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    iw.add_document(&Document::new())?;
    if commit {
        iw.commit()?;
    }
    let reader = iw.get_reader()?;
    iw.close()?;
    Ok((dir, reader))
}

/// Port of `TestMultiCollector.testCacheScoresIfNecessary`.
fn test_cache_scores_if_necessary() -> Result<()> {
    let (dir, reader) = random_one_doc_index(true)?;

    let ctx = first_leaf(&reader)?;

    expect_throws_panic("java.lang.AssertionError", || {
        let mut c = collector(
            ScoreMode::COMPLETE_NO_SCORES,
            ScorableClass::ScoreCachingWrappingScorer,
        );
        let mut lc = c.get_leaf_collector(&ctx).unwrap_or_else(|e| {
            rucene_test_framework::util::java_exceptions::rethrow_java(e.into_lucene_error())
        });
        lc.set_scorer(&mut SimpleScorable::new())
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e));
    });

    // no collector needs scores => no caching
    let c1 = collector(ScoreMode::COMPLETE_NO_SCORES, ScorableClass::SimpleScorable);
    let c2 = collector(ScoreMode::COMPLETE_NO_SCORES, ScorableClass::SimpleScorable);
    wrap_and_set_scorer(c1, c2, &ctx)?;

    // only one collector needs scores => no caching
    let c1 = collector(ScoreMode::COMPLETE, ScorableClass::SimpleScorable);
    let c2 = collector(ScoreMode::COMPLETE_NO_SCORES, ScorableClass::SimpleScorable);
    wrap_and_set_scorer(c1, c2, &ctx)?;

    // several collectors need scores => caching
    let c1 = collector(
        ScoreMode::COMPLETE,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    let c2 = collector(
        ScoreMode::COMPLETE,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    wrap_and_set_scorer(c1, c2, &ctx)?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestMultiCollector.testScorerWrappingForTopScores`.
fn test_scorer_wrapping_for_top_scores() -> Result<()> {
    let (dir, reader) = random_one_doc_index(false)?;
    let ctx = first_leaf(&reader)?;
    let c1 = collector(
        ScoreMode::TOP_SCORES,
        ScorableClass::MinCompetitiveScoreAwareScorable,
    );
    let c2 = collector(
        ScoreMode::TOP_SCORES,
        ScorableClass::MinCompetitiveScoreAwareScorable,
    );
    wrap_and_set_scorer(c1, c2, &ctx)?;

    let c1 = collector(
        ScoreMode::TOP_SCORES,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    let c2 = collector(
        ScoreMode::COMPLETE,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    wrap_and_set_scorer(c1, c2, &ctx)?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `Scorable` of `testMinCompetitiveScore`, which records the
/// minimum competitive score in `minCompetitiveScore[0]`.
#[derive(Debug, Default)]
struct RecordingScorable {
    min_competitive_score: f32,
}

impl Scorable for RecordingScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.min_competitive_score = min_score;
        Ok(())
    }
}

/// Port of `TestMultiCollector.testMinCompetitiveScore`.
///
/// Java builds `s0`, `s1` and `s2` once, over the same scorer and
/// `currentMinScores` array; Rust allows one mutable borrow of each at a
/// time, so each call builds `s<idx>` again over the same two, which holds
/// exactly the state Java's instances share (they have no other state).
fn test_min_competitive_score() -> Result<()> {
    let mut current_min_scores = [0.0f32; 3];
    let mut scorer = RecordingScorable::default();
    let mut set = |idx: usize, min: f32, scorer: &mut RecordingScorable| -> Result<f32> {
        MinCompetitiveScoreAwareScorable::new(scorer, idx, &mut current_min_scores)
            .set_min_competitive_score(min)?;
        Ok(scorer.min_competitive_score)
    };
    assert_equals_delta(0.0, scorer.min_competitive_score as f64, 0.0);
    let m = set(0, 0.5, &mut scorer)?;
    assert_equals_delta(0.0, m as f64, 0.0);
    let m = set(1, 0.8, &mut scorer)?;
    assert_equals_delta(0.0, m as f64, 0.0);
    let m = set(2, 0.3, &mut scorer)?;
    assert_equals_delta(0.3f32 as f64, m as f64, 0.0);
    let m = set(2, 0.1, &mut scorer)?;
    assert_equals_delta(0.3f32 as f64, m as f64, 0.0);
    let m = set(1, f32::MAX, &mut scorer)?;
    assert_equals_delta(0.3f32 as f64, m as f64, 0.0);
    let m = set(2, f32::MAX, &mut scorer)?;
    assert_equals_delta(0.5f32 as f64, m as f64, 0.0);
    let m = set(0, f32::MAX, &mut scorer)?;
    assert_equals_delta(f32::MAX as f64, m as f64, 0.0);
    Ok(())
}

/// Port of `TestMultiCollector.testCollectionTermination`.
fn test_collection_termination() -> Result<()> {
    let (dir, reader) = random_one_doc_index(false)?;
    let ctx = first_leaf(&reader)?;
    let c1 = DummyCollector::terminating(1, ScoreMode::COMPLETE);
    let c2 = DummyCollector::terminating(2, ScoreMode::COMPLETE);
    let (f1, f2) = (Arc::clone(&c1.flags), Arc::clone(&c2.flags));

    let mut mc = MultiCollector::wrap([
        Some(Box::new(c1) as Box<dyn Collector>),
        Some(Box::new(c2) as Box<dyn Collector>),
    ])?;
    {
        let mut lc = mc
            .get_leaf_collector(&ctx)
            .map_err(|e| e.into_lucene_error())?;
        let mut scorer = SimpleScorable::new();
        lc.set_scorer(&mut scorer)?;
        lc.collect(0, &mut scorer)
            .map_err(|e| e.into_lucene_error())?; // OK
        Assert::assert_true_msg("c1's collect should be called", f1.collect_called());
        Assert::assert_true_msg("c2's collect should be called", f2.collect_called());
        f1.collect_called.store(false, Ordering::SeqCst);
        f2.collect_called.store(false, Ordering::SeqCst);
        lc.collect(1, &mut scorer)
            .map_err(|e| e.into_lucene_error())?; // OK, but c1 should terminate
        Assert::assert_false_msg("c1 should be removed already", f1.collect_called());
        Assert::assert_true_msg("c2's collect should be called", f2.collect_called());
        f2.collect_called.store(false, Ordering::SeqCst);

        expect_terminated(lc.collect(2, &mut scorer));
        Assert::assert_false_msg("c1 should be removed already", f1.collect_called());
        Assert::assert_false_msg("c2 should be removed already", f2.collect_called());
    }

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestMultiCollector.testSetScorerOnCollectionTerminationSkipNonCompetitive`.
fn test_set_scorer_on_collection_termination_skip_non_competitive() -> Result<()> {
    do_test_set_scorer_on_collection_termination(true)
}

/// Port of `TestMultiCollector.testSetScorerOnCollectionTerminationSkipNoSkips`.
fn test_set_scorer_on_collection_termination_skip_no_skips() -> Result<()> {
    do_test_set_scorer_on_collection_termination(false)
}

fn do_test_set_scorer_on_collection_termination(allow_skip_non_competitive: bool) -> Result<()> {
    let (dir, reader) = random_one_doc_index(false)?;
    let ctx = first_leaf(&reader)?;

    let mode = if allow_skip_non_competitive {
        ScoreMode::TOP_SCORES
    } else {
        ScoreMode::COMPLETE
    };
    let c1 = DummyCollector::terminating(1, mode);
    let c2 = DummyCollector::terminating(2, mode);
    let (f1, f2) = (Arc::clone(&c1.flags), Arc::clone(&c2.flags));

    let mut mc = MultiCollector::wrap([
        Some(Box::new(c1) as Box<dyn Collector>),
        Some(Box::new(c2) as Box<dyn Collector>),
    ])?;
    {
        let mut lc = mc
            .get_leaf_collector(&ctx)
            .map_err(|e| e.into_lucene_error())?;
        Assert::assert_false(f1.set_scorer_called());
        Assert::assert_false(f2.set_scorer_called());
        lc.set_scorer(&mut SimpleScorable::new())?;
        Assert::assert_true(f1.set_scorer_called());
        Assert::assert_true(f2.set_scorer_called());
        f1.set_scorer_called.store(false, Ordering::SeqCst);
        f2.set_scorer_called.store(false, Ordering::SeqCst);
        lc.collect(0, &mut SimpleScorable::new())
            .map_err(|e| e.into_lucene_error())?; // OK

        lc.set_scorer(&mut SimpleScorable::new())?;
        Assert::assert_true(f1.set_scorer_called());
        Assert::assert_true(f2.set_scorer_called());
        f1.set_scorer_called.store(false, Ordering::SeqCst);
        f2.set_scorer_called.store(false, Ordering::SeqCst);

        lc.collect(1, &mut SimpleScorable::new())
            .map_err(|e| e.into_lucene_error())?; // OK, but c1 should terminate
        lc.set_scorer(&mut SimpleScorable::new())?;
        Assert::assert_false(f1.set_scorer_called());
        Assert::assert_true(f2.set_scorer_called());
        f2.set_scorer_called.store(false, Ordering::SeqCst);

        expect_terminated(lc.collect(2, &mut SimpleScorable::new()));
        lc.set_scorer(&mut SimpleScorable::new())?;
        Assert::assert_false(f1.set_scorer_called());
        Assert::assert_false(f2.set_scorer_called());
    }

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestMultiCollector.testMergeScoreModes`.
fn test_merge_score_modes() -> Result<()> {
    for sm1 in SCORE_MODE_VALUES {
        for sm2 in SCORE_MODE_VALUES {
            let c1 = DummyCollector::terminating(0, sm1);
            let c2 = DummyCollector::terminating(0, sm2);
            let c = MultiCollector::wrap([
                Some(Box::new(c1) as Box<dyn Collector>),
                Some(Box::new(c2) as Box<dyn Collector>),
            ])?;
            if sm1 == sm2 {
                assert_equals(&sm1, &c.score_mode());
            } else if sm1.needs_scores() || sm2.needs_scores() {
                assert_equals(&ScoreMode::COMPLETE, &c.score_mode());
            } else {
                assert_equals(&ScoreMode::COMPLETE_NO_SCORES, &c.score_mode());
            }
        }
    }
    Ok(())
}
