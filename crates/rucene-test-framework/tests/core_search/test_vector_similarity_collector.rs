//! Port of `lucene/core/src/test/org/apache/lucene/search/TestVectorSimilarityCollector.java`.
//!
//! `KnnSearchStrategy` is a value in the port: `assertSame` on a strategy
//! compares it by value.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::abstract_vector_similarity_query::{
    default_similarity_strategy, DEFAULT_DECAY,
};
use rucene::search::knn::{Hnsw, KnnCollector, KnnSearchStrategy};
use rucene::search::VectorSimilarityCollector;

use crate::core_support::{
    assert_array_equals, assert_array_equals_f32, assert_equals, class, test,
};

/// `TestVectorSimilarityCollector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestVectorSimilarityCollector",
        vec![
            test("testResultCollection", test_result_collection),
            test(
                "testDefaultConstructorUsesDefaultStrategy",
                test_default_constructor_uses_default_strategy,
            ),
            test(
                "testCustomStrategyIsPropagated",
                test_custom_strategy_is_propagated,
            ),
            test(
                "testNullStrategyDefaultsToDefault",
                test_null_strategy_defaults_to_default,
            ),
        ],
    )
}

/// Port of `TestVectorSimilarityCollector.testResultCollection`.
fn test_result_collection() -> Result<()> {
    let result_similarity = 0.5f32;

    let mut collector =
        VectorSimilarityCollector::new(result_similarity, DEFAULT_DECAY, i64::from(i32::MAX));

    let nodes = [1, 5, 10, 4, 8, 3, 2, 6, 7, 9];
    let scores = [
        0.1f32, 0.2f32, 0.3f32, 0.5f32, 0.2f32, 0.6f32, 0.9f32, 0.3f32, 0.7f32, 0.8f32,
    ];

    for i in 0..nodes.len() {
        collector.collect(nodes[i], scores[i]);
    }

    let score_docs = collector.top_docs().score_docs;
    let mut result_nodes = vec![0i32; score_docs.len()];
    let mut result_scores = vec![0f32; score_docs.len()];
    for i in 0..score_docs.len() {
        result_nodes[i] = score_docs[i].doc;
        result_scores[i] = score_docs[i].score;
    }

    // All nodes above resultSimilarity appear in order of collection
    assert_array_equals(&[4, 3, 2, 7, 9], &result_nodes);
    assert_array_equals_f32(
        &[0.5f32, 0.6f32, 0.9f32, 0.7f32, 0.8f32],
        &result_scores,
        1e-3f32,
    );
    Ok(())
}

/// Port of `TestVectorSimilarityCollector.testDefaultConstructorUsesDefaultStrategy`.
fn test_default_constructor_uses_default_strategy() -> Result<()> {
    let collector = VectorSimilarityCollector::new(0.5f32, DEFAULT_DECAY, i64::from(i32::MAX));
    assert_equals(
        &Some(&default_similarity_strategy()),
        &collector.get_search_strategy(),
    );
    Ok(())
}

/// Port of `TestVectorSimilarityCollector.testCustomStrategyIsPropagated`.
fn test_custom_strategy_is_propagated() -> Result<()> {
    let strategy = KnnSearchStrategy::Hnsw(Hnsw::new(42)?);
    let collector = VectorSimilarityCollector::with_strategy(
        0.5f32,
        DEFAULT_DECAY,
        i64::from(i32::MAX),
        Some(strategy.clone()),
    );
    assert_equals(&Some(&strategy), &collector.get_search_strategy());
    Ok(())
}

/// Port of `TestVectorSimilarityCollector.testNullStrategyDefaultsToDefault`.
fn test_null_strategy_defaults_to_default() -> Result<()> {
    let collector =
        VectorSimilarityCollector::with_strategy(0.5f32, DEFAULT_DECAY, i64::from(i32::MAX), None);
    assert_equals(
        &Some(&default_similarity_strategy()),
        &collector.get_search_strategy(),
    );
    Ok(())
}
