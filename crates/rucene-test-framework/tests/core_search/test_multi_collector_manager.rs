//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiCollectorManager.java`.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::collection_terminated_exception::{CollectionError, CollectionResult};
use rucene::search::multi_collector_manager::{ErasedCollectorManager, MultiCollectorManager};
use rucene::search::scorable::{Scorable, SimpleScorable};
use rucene::search::{Collector, CollectorManager, LeafCollector, ScoreMode};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::test_multi_collector::{ClassCheckingCollector, ScorableClass};
use crate::core_support::{assert_array_equals, assert_equals, assert_null, class, random, test};

/// `TestMultiCollectorManager`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMultiCollectorManager",
        vec![
            test("testCollection", test_collection),
            test("testNullCollectorManagers", test_null_collector_managers),
            test("testCacheScoresIfNecessary", test_cache_scores_if_necessary),
            test("testScoreWrapping", test_score_wrapping),
            test("testEarlyTermination", test_early_termination),
        ],
    )
}

/// `Predicate<Integer>` over the doc ids.
type Predicate = fn(i32) -> bool;

fn even_predicate(val: i32) -> bool {
    val % 2 == 0
}

fn odd_predicate(val: i32) -> bool {
    val % 2 == 1
}

fn all_predicate(_val: i32) -> bool {
    true
}

/// `(List<Integer>) results[i]`.
fn int_results(results: &[Box<dyn Any>], i: usize) -> Vec<i32> {
    results[i]
        .downcast_ref::<Vec<i32>>()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a List<Integer>",
            )
            .throw()
        })
        .clone()
}

/// `new MultiCollectorManager(managers...)`.
fn mcm(managers: Vec<Box<dyn ErasedCollectorManager>>) -> Result<MultiCollectorManager> {
    MultiCollectorManager::new(managers.into_iter().map(Some).collect())
}

/// Port of `TestMultiCollectorManager.testCollection`.
fn test_collection() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let reader = reader(&dir)?;
    let ctx = first_leaf(&reader)?;

    // Setup two collector managers, one that will only collect even doc ids
    // and one that only collects odd. Create some random doc ids and keep
    // track of the ones that we expect each collector manager to collect:
    let cm1 = SimpleCollectorManager::with(even_predicate);
    let cm2 = SimpleCollectorManager::with(odd_predicate);

    for _ in 0..100 {
        let docs = RandomNumbers::random_int_between(&*random(), 1000, 10000);
        let expected = generate_doc_ids(docs, &*random());
        let expected_even: Vec<i32> = expected
            .iter()
            .copied()
            .filter(|v| even_predicate(*v))
            .collect();
        let expected_odd: Vec<i32> = expected
            .iter()
            .copied()
            .filter(|v| odd_predicate(*v))
            .collect();

        // Test only wrapping one of the collector managers:
        let m = mcm(vec![Box::new(cm1)])?;
        let results = collect_all(&ctx, &expected, &m).map_err(|e| e.into_lucene_error())?;
        assert_equals(&1, &results.len());
        assert_array_equals(&expected_even, &int_results(&results, 0));

        // Test wrapping both collector managers:
        let m = mcm(vec![Box::new(cm1), Box::new(cm2)])?;
        let results = collect_all(&ctx, &expected, &m).map_err(|e| e.into_lucene_error())?;
        assert_equals(&2, &results.len());
        assert_array_equals(&expected_even, &int_results(&results, 0));
        assert_array_equals(&expected_odd, &int_results(&results, 1));
    }

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestMultiCollectorManager.testNullCollectorManagers`.
fn test_null_collector_managers() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        MultiCollectorManager::new(vec![None, None])
    });
    expect_throws("java.lang.IllegalArgumentException", || {
        MultiCollectorManager::new(vec![
            Some(Box::new(SimpleCollectorManager::new()) as Box<dyn ErasedCollectorManager>),
            None,
        ])
    });
    Ok(())
}

/// `new MultiCollectorManager(cm1, cm2).newCollector().getLeafCollector(ctx)
/// .setScorer(new SimpleScorable())`.
fn new_collector_set_scorer(
    cm1: ClassCheckingCollectorManager,
    cm2: ClassCheckingCollectorManager,
    ctx: &LeafReaderContext,
) -> Result<()> {
    let m = mcm(vec![Box::new(cm1), Box::new(cm2)])?;
    let mut c = CollectorManager::new_collector(&m)?;
    let mut lc = c
        .get_leaf_collector(ctx)
        .map_err(|e| e.into_lucene_error())?;
    lc.set_scorer(&mut SimpleScorable::new())
}

/// Port of `TestMultiCollectorManager.testCacheScoresIfNecessary`.
fn test_cache_scores_if_necessary() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let reader = reader(&dir)?;
    let ctx = first_leaf(&reader)?;

    // no collector needs scores => no caching
    let cm1 = collector_manager(ScoreMode::COMPLETE_NO_SCORES, ScorableClass::SimpleScorable);
    let cm2 = collector_manager(ScoreMode::COMPLETE_NO_SCORES, ScorableClass::SimpleScorable);
    new_collector_set_scorer(cm1, cm2, &ctx)?;

    // only one collector needs scores => no caching
    let cm1 = collector_manager(ScoreMode::COMPLETE, ScorableClass::SimpleScorable);
    let cm2 = collector_manager(ScoreMode::COMPLETE_NO_SCORES, ScorableClass::SimpleScorable);
    new_collector_set_scorer(cm1, cm2, &ctx)?;

    // several collectors need scores => caching
    let cm1 = collector_manager(
        ScoreMode::COMPLETE,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    let cm2 = collector_manager(
        ScoreMode::COMPLETE,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    new_collector_set_scorer(cm1, cm2, &ctx)?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestMultiCollectorManager.testScoreWrapping`.
fn test_score_wrapping() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let reader = reader(&dir)?;
    let ctx = first_leaf(&reader)?;

    // all wrapped collector managers are TOP_SCORE score mode, so they should
    // see a MinCompetitiveScoreAwareScorable passed in as their scorer:
    let cm1 = collector_manager(
        ScoreMode::TOP_SCORES,
        ScorableClass::MinCompetitiveScoreAwareScorable,
    );
    let cm2 = collector_manager(
        ScoreMode::TOP_SCORES,
        ScorableClass::MinCompetitiveScoreAwareScorable,
    );
    new_collector_set_scorer(cm1, cm2, &ctx)?;

    // both wrapped collector managers need scores, but one is exhaustive, so
    // they should see a ScoreCachingWrappingScorer pass in as their scorer:
    let cm1 = collector_manager(
        ScoreMode::COMPLETE,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    let cm2 = collector_manager(
        ScoreMode::TOP_SCORES,
        ScorableClass::ScoreCachingWrappingScorer,
    );
    new_collector_set_scorer(cm1, cm2, &ctx)?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestMultiCollectorManager.testEarlyTermination`.
fn test_early_termination() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let reader = reader(&dir)?;
    let ctx = first_leaf(&reader)?;

    let docs = RandomNumbers::random_int_between(&*random(), 1000, 10000);
    let expected = generate_doc_ids(docs, &*random());

    // The first collector manager should collect all docs even though the
    // second throws CollectionTerminatedException immediately:
    let cm1 = SimpleCollectorManager::new();
    let cm2 = TerminatingCollectorManager;
    let m = mcm(vec![Box::new(cm1), Box::new(cm2)])?;
    let results = collect_all(&ctx, &expected, &m).map_err(|e| e.into_lucene_error())?;
    assert_equals(&2, &results.len());
    let expected_all: Vec<i32> = expected.iter().copied().collect();
    assert_array_equals(&expected_all, &int_results(&results, 0));
    assert_null(
        results[1]
            .downcast_ref::<Option<()>>()
            .expect("the terminating manager's result"),
    );

    // If we wrap multiple collector managers that throw
    // CollectionTerminatedException, the exception should be thrown by the
    // MultiCollectorManager's collector:
    let cm3 = TerminatingCollectorManager;
    let m = mcm(vec![Box::new(cm2), Box::new(cm3)])?;
    match collect_all(&ctx, &expected, &m) {
        Err(CollectionError::CollectionTerminated) => {}
        Err(e) => rethrow_java(e.into_lucene_error()),
        Ok(_) => Assert::fail(
            "Expected exception CollectionTerminatedException but no exception was thrown",
        ),
    }

    reader.close()?;
    dir.close()?;
    Ok(())
}

fn reader(dir: &TestDirectory) -> Result<Arc<dyn DirectoryReader>> {
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    iw.add_document(&Document::new())?;
    iw.commit()?;
    let reader = iw.get_reader()?;
    iw.close()?;

    Ok(reader)
}

/// `reader.leaves().get(0)`.
fn first_leaf(reader: &Arc<dyn DirectoryReader>) -> Result<Arc<LeafReaderContext>> {
    Ok(Arc::clone(
        &(Arc::clone(reader) as Arc<dyn IndexReader>).leaves()?[0],
    ))
}

fn collect_all<M: CollectorManager>(
    ctx: &LeafReaderContext,
    values: &BTreeSet<i32>,
    collector_manager: &M,
) -> CollectionResult<M::Output> {
    let mut collectors: Vec<M::Collector> = Vec::new();
    let mut collector = collector_manager.new_collector()?;
    let mut values = values.iter().copied();
    // The value whose `random().nextInt(10) == 1` switched to a new collector,
    // collected by that collector's first leaf collector.
    let mut pending: Option<i32> = None;
    loop {
        let mut switched = false;
        {
            let mut leaf_collector = collector.get_leaf_collector(ctx)?;
            let mut scorer = SimpleScorable::new();
            if let Some(v) = pending.take() {
                leaf_collector.collect(v, &mut scorer)?;
            }
            for v in values.by_ref() {
                if random().next_int_bound(10) == 1 {
                    pending = Some(v);
                    switched = true;
                    break;
                }
                leaf_collector.collect(v, &mut scorer)?;
            }
        }
        if !switched {
            break;
        }
        collectors.push(collector);
        collector = collector_manager.new_collector()?;
    }
    collectors.push(collector);
    Ok(collector_manager.reduce(collectors)?)
}

/// Generate test doc ids. This will de-dupe and create a sorted collection to
/// be more realistic with real-world use-cases. Note that it's possible this
/// will generate fewer than 'count' entries because of de-duping, but that
/// should be quite rare and probably isn't worth worrying about for these
/// testing purposes.
fn generate_doc_ids(count: i32, random: &dyn Random) -> BTreeSet<i32> {
    let mut generated = BTreeSet::new();
    for _ in 0..count {
        generated.insert(random.next_int());
    }
    generated
}

/// `private static final class SimpleCollectorManager implements
/// CollectorManager<SimpleListCollector, List<Integer>>`.
#[derive(Clone, Copy)]
struct SimpleCollectorManager {
    predicate: Predicate,
}

impl SimpleCollectorManager {
    fn new() -> Self {
        SimpleCollectorManager {
            predicate: all_predicate,
        }
    }

    fn with(predicate: Predicate) -> Self {
        SimpleCollectorManager { predicate }
    }
}

impl CollectorManager for SimpleCollectorManager {
    type Collector = SimpleListCollector;
    type Output = Vec<i32>;

    fn new_collector(&self) -> Result<SimpleListCollector> {
        Ok(SimpleListCollector {
            predicate: self.predicate,
            collected: Vec::new(),
        })
    }

    fn reduce(&self, collectors: Vec<SimpleListCollector>) -> Result<Vec<i32>> {
        let mut all = Vec::new();
        for c in &collectors {
            all.extend_from_slice(&c.collected);
        }

        Ok(all)
    }
}

/// `private static final class SimpleListCollector implements Collector`.
#[derive(Debug)]
struct SimpleListCollector {
    predicate: Predicate,
    collected: Vec<i32>,
}

struct SimpleListLeaf<'a> {
    predicate: Predicate,
    collected: &'a mut Vec<i32>,
}

impl LeafCollector for SimpleListLeaf<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        if (self.predicate)(doc) {
            self.collected.push(doc);
        }
        Ok(())
    }
}

impl Collector for SimpleListCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(SimpleListLeaf {
            predicate: self.predicate,
            collected: &mut self.collected,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// `private static final class TerminatingCollectorManager implements
/// CollectorManager<Collector, Object>`; the `null` result is `None`.
#[derive(Clone, Copy)]
struct TerminatingCollectorManager;

/// The anonymous `SimpleCollector` of `TerminatingCollectorManager`.
#[derive(Debug)]
struct TerminatingCollector;

struct TerminatingLeaf;

impl LeafCollector for TerminatingLeaf {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Err(CollectionError::CollectionTerminated)
    }
}

impl Collector for TerminatingCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(TerminatingLeaf))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

impl CollectorManager for TerminatingCollectorManager {
    type Collector = TerminatingCollector;
    type Output = Option<()>;

    fn new_collector(&self) -> Result<TerminatingCollector> {
        Ok(TerminatingCollector)
    }

    fn reduce(&self, _collectors: Vec<TerminatingCollector>) -> Result<Option<()>> {
        Ok(None)
    }
}

/// The anonymous `CollectorManager` `collectorManager(scoreMode,
/// expectedScorer)` returns; its `null` result is `None`.
#[derive(Clone, Copy)]
struct ClassCheckingCollectorManager {
    score_mode: ScoreMode,
    expected_scorer: ScorableClass,
}

impl CollectorManager for ClassCheckingCollectorManager {
    type Collector = ClassCheckingCollector;
    type Output = Option<()>;

    fn new_collector(&self) -> Result<ClassCheckingCollector> {
        Ok(ClassCheckingCollector::new(
            self.score_mode,
            self.expected_scorer,
        ))
    }

    fn reduce(&self, _collectors: Vec<ClassCheckingCollector>) -> Result<Option<()>> {
        Ok(None)
    }
}

fn collector_manager(
    score_mode: ScoreMode,
    expected_scorer: ScorableClass,
) -> ClassCheckingCollectorManager {
    ClassCheckingCollectorManager {
        score_mode,
        expected_scorer,
    }
}
