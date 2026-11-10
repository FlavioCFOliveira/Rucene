//! Port of `lucene/core/src/test/org/apache/lucene/search/TestCachingCollector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::scorable::Scorable;
use rucene::search::{
    CachingCollector, CollectionError, CollectionResult, Collector, LeafCollector, ScoreMode,
    SimpleCollector, SimpleCollectorImpl,
};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_search::support::null_leaf_context;
use crate::core_support::{assert_equals, class, random, test};

/// 1 byte out of MB
const ONE_BYTE: f64 = 1.0 / (1024.0 * 1024.0);

/// `TestCachingCollector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestCachingCollector",
        vec![
            test("testBasic", test_basic),
            test("testIllegalStateOnReplay", test_illegal_state_on_replay),
            test("testCachedArraysAllocation", test_cached_arrays_allocation),
            test("testNoWrappedCollector", test_no_wrapped_collector),
        ],
    )
}

struct MockScorable;

impl Scorable for MockScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }
}

/// The test's private `NoOpCollector extends SimpleCollector`.
struct NoOpCollector;

impl LeafCollector for NoOpCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

impl SimpleCollectorImpl for NoOpCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

fn no_op() -> SimpleCollector<NoOpCollector> {
    SimpleCollector::new(NoOpCollector)
}

/// The anonymous replay collector of `testBasic`.
struct Sequential {
    prev_doc_id: i32,
}

impl LeafCollector for Sequential {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        assert_equals(&(self.prev_doc_id + 1), &doc);
        self.prev_doc_id = doc;
        Ok(())
    }
}

impl SimpleCollectorImpl for Sequential {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

fn coll<T>(r: CollectionResult<T>) -> Result<T> {
    r.map_err(CollectionError::into_lucene_error)
}

fn test_basic() -> Result<()> {
    let (_reader, leaves) = null_leaf_context()?;
    for cache_scores in [false, true] {
        let mut cc = CachingCollector::create_with_max_ram(no_op(), cache_scores, 1.0);
        {
            let mut acc = coll(cc.get_leaf_collector(&leaves[0]))?;
            acc.set_scorer(&mut MockScorable)?;

            // collect 1000 docs
            for i in 0..1000 {
                coll(acc.collect(i, &mut MockScorable))?;
            }
            acc.finish()?;
        }

        // now replay them
        coll(cc.replay(
            &mut SimpleCollector::new(Sequential { prev_doc_id: -1 }),
            &leaves,
        ))?;
    }
    Ok(())
}

fn test_illegal_state_on_replay() -> Result<()> {
    let (_reader, leaves) = null_leaf_context()?;
    let mut cc = CachingCollector::create_with_max_ram(no_op(), true, 50.0 * ONE_BYTE);
    {
        let mut acc = coll(cc.get_leaf_collector(&leaves[0]))?;
        acc.set_scorer(&mut MockScorable)?;

        // collect 130 docs, this should be enough for triggering cache abort.
        for i in 0..130 {
            coll(acc.collect(i, &mut MockScorable))?;
        }
    }

    Assert::assert_false_msg(
        "CachingCollector should not be cached due to low memory limit",
        cc.is_cached(),
    );

    expect_throws("java.lang.IllegalStateException", || {
        coll(cc.replay(&mut no_op(), &leaves))
    });
    Ok(())
}

fn test_cached_arrays_allocation() -> Result<()> {
    let (_reader, leaves) = null_leaf_context()?;
    // tests the cached arrays allocation -- if the 'nextLength' was too high,
    // caching would terminate even if a smaller length would suffice.

    // set RAM limit enough for 150 docs + random(10000)
    let num_docs = random().next_int_bound(10000) + 150;
    for cache_scores in [false, true] {
        let bytes_per_doc = if cache_scores { 8.0 } else { 4.0 };
        let mut cc = CachingCollector::create_with_max_ram(
            no_op(),
            cache_scores,
            bytes_per_doc * ONE_BYTE * f64::from(num_docs),
        );
        let mut acc = coll(cc.get_leaf_collector(&leaves[0]))?;
        acc.set_scorer(&mut MockScorable)?;
        for i in 0..num_docs {
            coll(acc.collect(i, &mut MockScorable))?;
        }
        // **Divergence from Lucene 10.5.0.** Java keeps `acc` while it asks
        // `cc.isCached()`; the Rust leaf collector borrows `cc` mutably, so
        // it is released for the check and a leaf collector is taken again
        // for the same context. The cache limit counts every cached
        // document, so the next document still exceeds it.
        drop(acc);
        Assert::assert_true(cc.is_cached());

        // The 151's document should terminate caching
        let mut acc = coll(cc.get_leaf_collector(&leaves[0]))?;
        coll(acc.collect(num_docs, &mut MockScorable))?;
        drop(acc);
        Assert::assert_false(cc.is_cached());
    }
    Ok(())
}

fn test_no_wrapped_collector() -> Result<()> {
    let (_reader, leaves) = null_leaf_context()?;
    for cache_scores in [false, true] {
        // create w/ null wrapped collector, and test that the methods work
        let mut cc = CachingCollector::create_standalone(cache_scores, 50.0 * ONE_BYTE);
        {
            let mut acc = coll(cc.get_leaf_collector(&leaves[0]))?;
            acc.set_scorer(&mut MockScorable)?;
            coll(acc.collect(0, &mut MockScorable))?;
            // **Divergence from Lucene 10.5.0.** Java asks `cc.isCached()`
            // before `acc.finish()`; the Rust leaf collector borrows `cc`
            // mutably, so it finishes before the check.
            acc.finish()?;
        }
        Assert::assert_true(cc.is_cached());
        coll(cc.replay(&mut no_op(), &leaves))?;
    }
    Ok(())
}
