//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTopKnnResults.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::knn::{KnnCollector, TopKnnCollector};

use crate::core_support::{assert_array_equals, assert_array_equals_f32, class, test};

/// `TestTopKnnResults`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTopKnnResults",
        vec![test(
            "testCollectAndProvideResults",
            test_collect_and_provide_results,
        )],
    )
}

/// Port of `TestTopKnnResults.testCollectAndProvideResults`.
fn test_collect_and_provide_results() -> Result<()> {
    let mut results = TopKnnCollector::new(5, i64::from(i32::MAX));
    let nodes = [4, 1, 5, 7, 8, 10, 2];
    let scores = [1f32, 0.5f32, 0.6f32, 2f32, 2f32, 1.2f32, 4f32];
    for i in 0..nodes.len() {
        results.collect(nodes[i], scores[i]);
    }
    let top_docs = results.top_docs()?;
    let mut sorted_nodes = vec![0i32; top_docs.score_docs.len()];
    let mut sorted_scores = vec![0f32; top_docs.score_docs.len()];
    for i in 0..top_docs.score_docs.len() {
        sorted_nodes[i] = top_docs.score_docs[i].doc;
        sorted_scores[i] = top_docs.score_docs[i].score;
    }
    assert_array_equals(&[2, 7, 8, 10, 4], &sorted_nodes);
    assert_array_equals_f32(&[4f32, 2f32, 2f32, 1.2f32, 1f32], &sorted_scores, 0f32);
    Ok(())
}
