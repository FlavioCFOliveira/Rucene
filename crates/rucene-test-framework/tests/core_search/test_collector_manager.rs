//! Port of `lucene/core/src/test/org/apache/lucene/search/TestCollectorManager.java`.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::reader_context::LeafReaderContext;
use rucene::index::IndexReader;
use rucene::search::multi_collector::MultiCollector;
use rucene::search::scorable::Scorable;
use rucene::search::{
    CollectionError, CollectionResult, Collector, CollectorManager, LeafCollector, ScoreMode,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, class, random, test, IAE};

/// `TestCollectorManager`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestCollectorManager",
        vec![
            test("testCollection", test_collection),
            test("testEmptyCollectors", test_empty_collectors),
        ],
    )
}

type Predicate = fn(i32) -> bool;

fn test_collection() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let reader = reader(&dir)?;
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let ctx = &leaves[0];

    let even_predicate: Predicate = |val| val % 2 == 0;
    let odd_predicate: Predicate = |val| val % 2 == 1;

    let cm = CompositeCollectorManager {
        predicates: vec![even_predicate, odd_predicate],
    };

    for _iter in 0..100 {
        let docs = RandomNumbers::random_int_between(&*random(), 1000, 10000);
        let expected = generate_doc_ids(docs, &*random());
        let expected_even = expected.iter().copied().filter(|v| even_predicate(*v));
        let expected_odd = expected.iter().copied().filter(|v| odd_predicate(*v));

        let result = collect_all(ctx, &expected, &cm)?;
        // `assertThat(result, instanceOf(List.class))`: the reduction is a
        // list by type.
        let mut int_results = result;
        int_results.sort();
        let mut all: Vec<i32> = expected_even.chain(expected_odd).collect();
        all.sort();
        assert_array_equals(&all, &int_results);
    }
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_empty_collectors() -> Result<()> {
    rucene_test_framework::util::java_exceptions::expect_throws(IAE, || {
        CompositeCollectorManager {
            predicates: Vec::new(),
        }
        .new_collector()
    });
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

fn collect_all<M: CollectorManager>(
    ctx: &LeafReaderContext,
    values: &BTreeSet<i32>,
    collector_manager: &M,
) -> Result<M::Output> {
    let coll = |r: CollectionResult<()>| r.map_err(CollectionError::into_lucene_error);
    let mut collectors = vec![collector_manager.new_collector()?];
    // Java keeps the current leaf collector across values, switching to a
    // new collector's leaf collector at random; a Rust leaf collector borrows
    // its collector, so the current one is re-created per value from the
    // last collector (the collectors keep their state, not the leaf
    // collectors).
    for &v in values {
        if random().next_int_bound(10) == 1 {
            collectors.push(collector_manager.new_collector()?);
        }
        let collector = collectors.last_mut().expect("one collector");
        let mut leaf_collector = collector
            .get_leaf_collector(ctx)
            .map_err(CollectionError::into_lucene_error)?;
        coll(leaf_collector.collect(v, &mut NoScorable))?;
    }
    collector_manager.reduce(collectors)
}

/// The scorable a Rust `collect` takes; Java's `collect(int)` has none.
struct NoScorable;

impl Scorable for NoScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }
}

fn generate_doc_ids(count: i32, random: &dyn Random) -> BTreeSet<i32> {
    let mut generated = BTreeSet::new();
    for _ in 0..count {
        generated.insert(random.next_int());
    }
    generated
}

/// The test's private `record CompositeCollectorManager`.
struct CompositeCollectorManager {
    predicates: Vec<Predicate>,
}

impl CollectorManager for CompositeCollectorManager {
    type Collector = Box<dyn Collector>;
    type Output = Vec<i32>;

    fn new_collector(&self) -> Result<Box<dyn Collector>> {
        MultiCollector::wrap(self.predicates.iter().map(|p| {
            Some(Box::new(SimpleListCollector {
                predicate: *p,
                collected: Arc::new(Mutex::new(Vec::new())),
            }) as Box<dyn Collector>)
        }))
    }

    fn reduce(&self, collectors: Vec<Box<dyn Collector>>) -> Result<Vec<i32>> {
        let mut all = Vec::new();
        for m in &collectors {
            let multi = m
                .as_any()
                .and_then(|a| a.downcast_ref::<MultiCollector>())
                .expect("ClassCastException: MultiCollector");
            for c in multi.get_collectors() {
                let list = c
                    .as_any()
                    .and_then(|a| a.downcast_ref::<SimpleListCollector>())
                    .expect("ClassCastException: SimpleListCollector");
                all.extend(
                    list.collected
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter(),
                );
            }
        }
        Ok(all)
    }
}

/// The test's private `SimpleListCollector implements Collector`.
struct SimpleListCollector {
    predicate: Predicate,
    collected: Arc<Mutex<Vec<i32>>>,
}

struct SimpleListLeafCollector {
    predicate: Predicate,
    collected: Arc<Mutex<Vec<i32>>>,
}

impl LeafCollector for SimpleListLeafCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        if (self.predicate)(doc) {
            self.collected
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(doc);
        }
        Ok(())
    }
}

impl Collector for SimpleListCollector {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(SimpleListLeafCollector {
            predicate: self.predicate,
            collected: Arc::clone(&self.collected),
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}
