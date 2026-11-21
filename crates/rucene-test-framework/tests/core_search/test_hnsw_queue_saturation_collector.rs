//! Port of `lucene/core/src/test/org/apache/lucene/search/TestHnswQueueSaturationCollector.java`.
//!
//! Tests for `HnswQueueSaturationCollector`.
//!
//! The tests read the `delegate` collector after handing it to the
//! saturation collector, which owns it in Rucene; the delegate is shared
//! through [`SharedCollector`], which forwards every call to the one
//! `TopKnnCollector` the test also keeps.

use std::sync::{Arc, Mutex, MutexGuard};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene::search::hnsw_queue_saturation_collector::HnswQueueSaturationCollector;
use rucene::search::knn::TopDocs;
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::{KnnCollector, KnnSearchStrategy, TopKnnCollector};

use crate::core_support::{assert_equals, class, random, test};

/// `TestHnswQueueSaturationCollector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestHnswQueueSaturationCollector",
        vec![
            test("testDelegate", test_delegate).annotate(Test::new()),
            test("testEarlyExpectedExit", test_early_expected_exit).annotate(Test::new()),
            test(
                "testDelegateVsSaturateEarlyExit",
                test_delegate_vs_saturate_early_exit,
            )
            .annotate(Test::new()),
            test("testEarlyExitRelation", test_early_exit_relation).annotate(Test::new()),
        ],
    )
}

/// The `delegate` reference the test keeps.
struct SharedCollector {
    inner: Arc<Mutex<TopKnnCollector>>,
    search_strategy: Option<KnnSearchStrategy>,
}

impl SharedCollector {
    /// `new TopKnnCollector(k, visitLimit)`, shared.
    #[allow(clippy::new_ret_no_self)] // Java's constructor, handing the object back as the type its callers use
    fn new(k: i32, visit_limit: i64) -> (Arc<Mutex<TopKnnCollector>>, Box<dyn KnnCollector>) {
        let inner = Arc::new(Mutex::new(TopKnnCollector::new(k, visit_limit)));
        let search_strategy = lock(&inner).get_search_strategy().cloned();
        (
            Arc::clone(&inner),
            Box::new(SharedCollector {
                inner,
                search_strategy,
            }),
        )
    }
}

fn lock(c: &Mutex<TopKnnCollector>) -> MutexGuard<'_, TopKnnCollector> {
    c.lock().unwrap_or_else(|e| e.into_inner())
}

impl KnnCollector for SharedCollector {
    fn early_terminated(&self) -> bool {
        lock(&self.inner).early_terminated()
    }
    fn inc_visited_count(&mut self, count: i32) {
        lock(&self.inner).inc_visited_count(count)
    }
    fn visited_count(&self) -> i64 {
        lock(&self.inner).visited_count()
    }
    fn visit_limit(&self) -> i64 {
        lock(&self.inner).visit_limit()
    }
    fn k(&self) -> i32 {
        lock(&self.inner).k()
    }
    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        lock(&self.inner).collect(doc_id, similarity)
    }
    fn min_competitive_similarity(&self) -> f32 {
        lock(&self.inner).min_competitive_similarity()
    }
    fn top_docs(&mut self) -> rucene::error::Result<TopDocs> {
        lock(&self.inner).top_docs()
    }
    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.search_strategy.as_ref()
    }
}

fn test_delegate() -> Result<()> {
    let random = random();
    let num_docs = 100;
    let k = random.next_int_range(1, 10);
    let (delegate, shared) = SharedCollector::new(k, num_docs);
    let mut queue_saturation_collector = HnswQueueSaturationCollector::new(shared, 1.0, i32::MAX);
    let mut i = 0;
    while i < random.next_int_bound(num_docs as i32) {
        queue_saturation_collector.collect(
            random.next_int_bound(num_docs as i32),
            random.next_float_bound(1.0),
        );
        i += 1;
    }
    // Each read locks the shared delegate on its own and releases it before
    // the saturation collector, which locks it too, is asked: a guard held as
    // a temporary of the comparison would deadlock.
    let delegate_k = lock(&delegate).k();
    assert_equals(&delegate_k, &queue_saturation_collector.k());
    let delegate_visited = lock(&delegate).visited_count();
    assert_equals(
        &delegate_visited,
        &queue_saturation_collector.visited_count(),
    );
    let delegate_visit_limit = lock(&delegate).visit_limit();
    assert_equals(
        &delegate_visit_limit,
        &queue_saturation_collector.visit_limit(),
    );
    let delegate_min = lock(&delegate).min_competitive_similarity();
    Assert::assert_equals_float_msg(
        None,
        delegate_min,
        queue_saturation_collector.min_competitive_similarity(),
        1e-3,
    );
    Ok(())
}

fn test_early_expected_exit() -> Result<()> {
    let num_docs = 1000;
    let k = 10;
    let (_delegate, shared) = SharedCollector::new(k, num_docs);
    let mut queue_saturation_collector = HnswQueueSaturationCollector::new(shared, 0.9, 10);
    for i in 0..num_docs as i32 {
        queue_saturation_collector.collect(i, 1.0f32 - i as f32 * 1e-3f32);
        if i % 10 == 0 {
            queue_saturation_collector.next_candidate();
        }
        if queue_saturation_collector.early_terminated() {
            assert_equals(&120, &i);
            break;
        }
    }
    Ok(())
}

fn test_delegate_vs_saturate_early_exit() -> Result<()> {
    let random = random();
    let num_docs = 10000;
    let k = random.next_int_range(1, 100);
    let (delegate, shared) = SharedCollector::new(k, num_docs);
    let mut queue_saturation_collector = HnswQueueSaturationCollector::new(shared, 0.5, 1);
    let mut i = 0;
    while i < random.next_int_bound(num_docs as i32) {
        queue_saturation_collector.collect(
            random.next_int_bound(num_docs as i32),
            random.next_float_bound(1.0),
        );
        if i % 10 == 0 {
            queue_saturation_collector.next_candidate();
        }
        let early_terminated_saturation = queue_saturation_collector.early_terminated();
        let early_terminated_delegate = lock(&delegate).early_terminated();
        Assert::assert_true(early_terminated_saturation || !early_terminated_delegate);
        i += 1;
    }
    Ok(())
}

fn test_early_exit_relation() -> Result<()> {
    let random = random();
    let num_docs = 10000;
    let k = random.next_int_range(1, 100);
    let (delegate, shared) = SharedCollector::new(k, i64::from(random.next_int_bound(num_docs)));
    let mut queue_saturation_collector = HnswQueueSaturationCollector::new(shared, 0.5, 1);
    let mut i = 0;
    while i < random.next_int_bound(num_docs) {
        queue_saturation_collector.collect(
            random.next_int_bound(num_docs),
            random.next_float_bound(1.0),
        );
        if i % 10 == 0 {
            queue_saturation_collector.next_candidate();
        }
        if lock(&delegate).early_terminated() {
            let top_docs = queue_saturation_collector.top_docs()?;
            assert_equals(
                &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                &top_docs.total_hits.relation(),
            );
        }
        if queue_saturation_collector.early_terminated() {
            let top_docs = queue_saturation_collector.top_docs()?;
            assert_equals(
                &TotalHitsRelation::EQUAL_TO,
                &top_docs.total_hits.relation(),
            );
            break;
        }
        i += 1;
    }
    Ok(())
}
