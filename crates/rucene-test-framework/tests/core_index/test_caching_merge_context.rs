//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCachingMergeContext.java`.
//!
//! Java passes `null` as the `SegmentCommitInfo`; a Rust reference cannot be
//! null, so the test passes one segment ([`null_info`]) wherever Java passes
//! `null`, and `cachedNumDeletesToMerge.getOrDefault(null, -1)` reads that
//! segment's entry (Rucene's cache is keyed by segment name).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::lucene104::Lucene104Codec;
use rucene::error::Result;
use rucene::index::merge_policy::{CachingMergeContext, MergeContext};
use rucene::index::segment_info::{SegmentCommitInfo, SegmentInfo};
use rucene::store::ByteBuffersDirectory;
use rucene::util::{InfoStream, NoOutputInfoStream, StringHelper, Version};

use crate::core_support::{assert_equals, class, test};

/// `TestCachingMergeContext`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestCachingMergeContext",
        vec![test("testNumDeletesToMerge", test_num_deletes_to_merge)],
    )
}

/// The segment standing for Java's `null` `SegmentCommitInfo`.
fn null_info() -> Result<SegmentCommitInfo> {
    let si = SegmentInfo::new(
        Arc::new(ByteBuffersDirectory::new()),
        Version::LATEST,
        Some(Version::LATEST),
        "null".to_string(),
        1,
        false,
        false,
        Arc::new(Lucene104Codec::new()),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        None,
    )?;
    SegmentCommitInfo::new(si, 0, 0, -1, -1, -1, StringHelper::random_id())
}

/// Port of `TestCachingMergeContext.testNumDeletesToMerge`.
fn test_num_deletes_to_merge() -> Result<()> {
    let null = null_info()?;
    let merge_context = MockMergeContext::default();
    let caching_merge_context = CachingMergeContext::new(&merge_context);
    assert_equals(&caching_merge_context.num_deletes_to_merge(&null)?, &1);
    assert_equals(
        &caching_merge_context.cached_num_deletes_to_merge().len(),
        &1,
    );
    assert_equals(
        &caching_merge_context
            .cached_num_deletes_to_merge()
            .get(&null.info.name)
            .copied()
            .unwrap_or(-1),
        &1,
    );
    assert_equals(&merge_context.count.load(Ordering::SeqCst), &1);

    // increase the mock count
    merge_context.num_deletes_to_merge(&null)?;
    assert_equals(&merge_context.count.load(Ordering::SeqCst), &2);

    // assert the cache result still one
    assert_equals(&caching_merge_context.num_deletes_to_merge(&null)?, &1);
    assert_equals(
        &caching_merge_context.cached_num_deletes_to_merge().len(),
        &1,
    );
    assert_equals(
        &caching_merge_context
            .cached_num_deletes_to_merge()
            .get(&null.info.name)
            .copied()
            .unwrap_or(-1),
        &1,
    );
    Ok(())
}

/// `private static final class MockMergeContext implements
/// MergePolicy.MergeContext`.
#[derive(Default)]
struct MockMergeContext {
    count: AtomicI32,
}

impl MergeContext for MockMergeContext {
    fn num_deletes_to_merge(&self, _info: &SegmentCommitInfo) -> Result<i32> {
        Ok(self.count.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn num_deleted_docs(&self, _info: &SegmentCommitInfo) -> rucene::error::Result<i32> {
        Ok(0)
    }

    /// Java returns `null`; the trait returns a stream, so this is the one
    /// that writes nothing.
    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::new(NoOutputInfoStream)
    }

    /// Java returns `null`; the trait returns a set, so this is an empty one.
    fn get_merging_segments(&self) -> HashSet<String> {
        HashSet::new()
    }
}
