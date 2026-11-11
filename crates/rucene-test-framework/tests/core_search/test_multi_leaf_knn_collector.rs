//! Port of `lucene/core/src/test/org/apache/lucene/search/knn/TestMultiLeafKnnCollector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::knn::multi_leaf_knn_collector::MultiLeafKnnCollector;
use rucene::search::knn::{KnnCollector, TopKnnCollector};
use rucene::util::hnsw::BlockingFloatHeap;

use crate::core_support::{class, test};

/// `TestMultiLeafKnnCollector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.knn.TestMultiLeafKnnCollector",
        vec![test(
            "testGlobalScoreCoordination",
            test_global_score_coordination,
        )],
    )
}

/// Validates a fix for GH#13462
fn test_global_score_coordination() -> Result<()> {
    let k = 7;
    let global_heap = Arc::new(BlockingFloatHeap::new(k as usize));
    let mut collector1 = MultiLeafKnnCollector::new(
        k,
        Arc::clone(&global_heap),
        Box::new(TopKnnCollector::new(k, i64::from(i32::MAX))),
    )?;
    let mut collector2 = MultiLeafKnnCollector::new(
        k,
        Arc::clone(&global_heap),
        Box::new(TopKnnCollector::new(k, i64::from(i32::MAX))),
    )?;

    // Collect k (7) hits in collector1 with scores [100, 106]:
    for i in 0..k {
        collector1.collect(0, 100.0 + i as f32);
    }

    // The global heap should be updated since k hits were collected, and have a min score of
    // 100:
    Assert::assert_equals_float_msg(None, 100.0, global_heap.peek(), 0.0);
    Assert::assert_equals_float_msg(None, 100.0, collector1.min_competitive_similarity(), 0.0);

    // Collect k (7) hits in collector2 with only two that are competitive (200 and 300),
    // which also forces an update of the global heap with collector2's hits. This is a tricky
    // case where the heap will not be fully ordered, so it ensures global queue updates don't
    // incorrectly short-circuit (see GH#13462):
    collector2.collect(0, 10.0);
    collector2.collect(0, 11.0);
    collector2.collect(0, 12.0);
    collector2.collect(0, 13.0);
    collector2.collect(0, 200.0);
    collector2.collect(0, 14.0);
    collector2.collect(0, 300.0);

    // At this point, our global heap should contain [102, 103, 104, 105, 106, 200, 300] since
    // values 200 and 300 from collector2 should have pushed out 100 and 101 from collector1.
    // The min value on the global heap should be 102:
    Assert::assert_equals_float_msg(None, 102.0, global_heap.peek(), 0.0);
    Assert::assert_equals_float_msg(None, 102.0, collector2.min_competitive_similarity(), 0.0);
    Ok(())
}
