//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMergeRateLimiter.java`.
//!
//! `new MergeRateLimiter(new MergePolicy.OneMergeProgress())` is
//! `MergeRateLimiter::new()`: a limiter over an abort flag, pause lock and
//! pause times of its own, which is what a fresh `OneMergeProgress` holds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::merge_scheduler::MergeRateLimiter;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_delta, class, random, test};

/// `TestMergeRateLimiter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMergeRateLimiter",
        vec![test("testInitDefaults", test_init_defaults)],
    )
}

/// Port of `TestMergeRateLimiter.testInitDefaults`.
fn test_init_defaults() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    w.close()?;
    drop(w);

    let rate_limiter = MergeRateLimiter::new();
    assert_equals_delta(f64::INFINITY, rate_limiter.get_mb_per_sec(), 0.0);
    Assert::assert_true(rate_limiter.get_min_pause_check_bytes() > 0);
    dir.close()
}
