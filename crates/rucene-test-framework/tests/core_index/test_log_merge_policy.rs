//! Port of `lucene/core/src/test/org/apache/lucene/index/TestLogMergePolicy.java`.
//!
//! The class extends `BaseMergePolicyTestCase`: its overrides of
//! `mergePolicy()`, `assertSegmentInfos` and `assertMerge` are
//! [`LogSuite`], and its inherited cases run through
//! `rucene_test_framework::index::base_merge_policy_test_case::class()`.
//! `LogDocMergePolicy` is `LogMergePolicy::by_doc_count()` and
//! `LogByteSizeMergePolicy` is `LogMergePolicy::by_byte_size()`.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::index_writer::{SOURCE_FLUSH, SOURCE_MERGE};
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy, MergeSpecification, MergeTrigger};
use rucene::index::segment_infos::SegmentInfos;
use rucene::util::Version;
use rucene_test_framework::index::base_merge_policy_test_case::{
    self, apply_merge, fail, make_segment_commit_info, merge_policy_test, BaseMergePolicyTestCase,
    IOStats, MergePolicyTestInstance, MockMergeContext,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, assert_null};

/// `(LogMergePolicy) policy`.
fn as_log(policy: &dyn MergePolicy) -> &LogMergePolicy {
    policy
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| fail("java.lang.ClassCastException: not a LogMergePolicy"))
}

/// `TestLogMergePolicy`'s overrides of `BaseMergePolicyTestCase`.
struct LogSuite;

impl BaseMergePolicyTestCase for LogSuite {
    fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        Arc::new(LuceneTestCase::new_log_merge_policy_with(
            &*LuceneTestCase::random(),
        ))
    }

    fn assert_segment_infos(&self, policy: &dyn MergePolicy, infos: &SegmentInfos) -> Result<()> {
        let mp = as_log(policy);

        let mock_merge_context = MockMergeContext::del_count();
        for info in infos.iter() {
            Assert::assert_true(
                MergePolicy::size(mp, info, &mock_merge_context)?
                    / i64::from(mp.get_merge_factor())
                    < mp.max_merge_size(),
            );
        }
        // TODO: what else can we check?
        Ok(())
    }

    fn assert_merge(&self, policy: &dyn MergePolicy, merge: &MergeSpecification) -> Result<()> {
        let lmp = as_log(policy);
        let mock_merge_context = MockMergeContext::del_count();
        for one_merge in &merge.merges {
            let mut merge_size = 0i64;
            for info in &one_merge.segments {
                merge_size += MergePolicy::size(lmp, info, &mock_merge_context)?;
            }
            Assert::assert_true_msg(
                &format!(
                    "mergeSize: {merge_size} minMergeSize: {} segmentsCount: {} mergeFactor: {}",
                    lmp.min_merge_size(),
                    one_merge.segments.len(),
                    lmp.get_merge_factor()
                ),
                merge_size <= lmp.min_merge_size()
                    || one_merge.segments.len() as i32 <= lmp.get_merge_factor(),
            );
        }
        Ok(())
    }
}

/// `TestLogMergePolicy`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestLogMergePolicy")
        .extends(&base_merge_policy_test_case::class())
        .constructor(Constructor::no_args(|| {
            MergePolicyTestInstance::new(Arc::new(LogSuite))
        }))
        .method(merge_policy_test("testDefaultForcedMergeMB", |_| {
            test_default_forced_merge_mb()
        }))
        .method(merge_policy_test("testIncreasingSegmentSizes", |_| {
            test_increasing_segment_sizes()
        }))
        .method(merge_policy_test("testOneSmallMiddleSegment", |_| {
            test_one_small_middle_segment()
        }))
        .method(merge_policy_test("testManySmallMiddleSegment", |_| {
            test_many_small_middle_segment()
        }))
        .method(merge_policy_test("testRejectUnbalancedMerges", |_| {
            test_reject_unbalanced_merges()
        }))
        .method(merge_policy_test("testPackLargeSegments", |_| {
            test_pack_large_segments()
        }))
        .method(merge_policy_test("testIgnoreLargeSegments", |_| {
            test_ignore_large_segments()
        }))
        .method(merge_policy_test(
            "testFullFlushMerges",
            test_full_flush_merges,
        ))
        .build()
}

/// `"_" + segNameGenerator.getAndIncrement()`.
fn next_name(seg_name_generator: &AtomicI64) -> String {
    format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst))
}

/// `new SegmentInfos(Version.LATEST.major)`.
fn new_segment_infos() -> Result<SegmentInfos> {
    SegmentInfos::new(i32::from(Version::LATEST.major))
}

/// Applies every merge of `spec` to `segment_infos`.
fn apply_all(
    mut segment_infos: SegmentInfos,
    spec: &MergeSpecification,
    seg_name_generator: &AtomicI64,
    stats: &mut IOStats,
) -> Result<SegmentInfos> {
    for one_merge in &spec.merges {
        segment_infos = apply_merge(
            &segment_infos,
            one_merge,
            &next_name(seg_name_generator),
            stats,
        )?;
    }
    Ok(segment_infos)
}

/// `segmentInfos.info(i).info.maxDoc()`.
fn max_doc(segment_infos: &SegmentInfos, i: usize) -> Result<i32> {
    segment_infos.info(i).info.max_doc()
}

/// Port of `TestLogMergePolicy.testDefaultForcedMergeMB`.
fn test_default_forced_merge_mb() -> Result<()> {
    let mp = LogMergePolicy::by_byte_size();
    Assert::assert_true(mp.get_max_merge_mb_for_forced_merge() > 0.0);
    Ok(())
}

/// Port of `TestLogMergePolicy.testIncreasingSegmentSizes`.
fn test_increasing_segment_sizes() -> Result<()> {
    let merge_policy = LogMergePolicy::by_doc_count();
    let mut stats = IOStats::default();
    let seg_name_generator = AtomicI64::new(0);
    let merge_context = MockMergeContext::del_count();
    let mut segment_infos = new_segment_infos()?;
    // 11 segments of increasing sizes
    for i in 0..11 {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            (i + 1) * 1000,
            0,
            0.0,
            SOURCE_MERGE,
        )?)?;
    }
    let spec = merge_policy.find_merges(MergeTrigger::Explicit, &segment_infos, &merge_context)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked above");
    segment_infos = apply_all(segment_infos, &spec, &seg_name_generator, &mut stats)?;
    assert_equals(&2, &segment_infos.size());
    assert_equals(&55_000, &max_doc(&segment_infos, 0)?);
    assert_equals(&11_000, &max_doc(&segment_infos, 1)?);
    Ok(())
}

/// Port of `TestLogMergePolicy.testOneSmallMiddleSegment`.
fn test_one_small_middle_segment() -> Result<()> {
    let merge_policy = LogMergePolicy::by_doc_count();
    let mut stats = IOStats::default();
    let seg_name_generator = AtomicI64::new(0);
    let merge_context = MockMergeContext::del_count();
    let mut segment_infos = new_segment_infos()?;
    // 5 big segments
    for _ in 0..5 {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            10_000,
            0,
            0.0,
            SOURCE_MERGE,
        )?)?;
    }
    // 1 segment on a lower tier
    segment_infos.add(make_segment_commit_info(
        &next_name(&seg_name_generator),
        100,
        0,
        0.0,
        SOURCE_MERGE,
    )?)?;
    // 5 big segments again
    for _ in 0..5 {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            10_000,
            0,
            0.0,
            SOURCE_MERGE,
        )?)?;
    }
    // Ensure that having a small segment in the middle doesn't prevent merging
    let spec = merge_policy.find_merges(MergeTrigger::Explicit, &segment_infos, &merge_context)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked above");
    segment_infos = apply_all(segment_infos, &spec, &seg_name_generator, &mut stats)?;
    assert_equals(&2, &segment_infos.size());
    assert_equals(&90_100, &max_doc(&segment_infos, 0)?);
    assert_equals(&10_000, &max_doc(&segment_infos, 1)?);
    Ok(())
}

/// Port of `TestLogMergePolicy.testManySmallMiddleSegment`.
fn test_many_small_middle_segment() -> Result<()> {
    let merge_policy = LogMergePolicy::by_doc_count();
    let mut stats = IOStats::default();
    let seg_name_generator = AtomicI64::new(0);
    let merge_context = MockMergeContext::del_count();
    let mut segment_infos = new_segment_infos()?;
    // 1 big segment
    segment_infos.add(make_segment_commit_info(
        &next_name(&seg_name_generator),
        10_000,
        0,
        0.0,
        SOURCE_MERGE,
    )?)?;
    // 9 segment on a lower tier
    for _ in 0..9 {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            100,
            0,
            0.0,
            SOURCE_MERGE,
        )?)?;
    }
    // 1 big segment again
    segment_infos.add(make_segment_commit_info(
        &next_name(&seg_name_generator),
        10_000,
        0,
        0.0,
        SOURCE_MERGE,
    )?)?;
    // Ensure that having small segments in the middle doesn't prevent merging
    let spec = merge_policy.find_merges(MergeTrigger::Explicit, &segment_infos, &merge_context)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked above");
    segment_infos = apply_all(segment_infos, &spec, &seg_name_generator, &mut stats)?;
    assert_equals(&2, &segment_infos.size());
    assert_equals(&10_900, &max_doc(&segment_infos, 0)?);
    assert_equals(&10_000, &max_doc(&segment_infos, 1)?);
    Ok(())
}

/// Port of `TestLogMergePolicy.testRejectUnbalancedMerges`.
fn test_reject_unbalanced_merges() -> Result<()> {
    let mut merge_policy = LogMergePolicy::by_doc_count();
    merge_policy.set_min_merge_docs(10_000);
    let mut stats = IOStats::default();
    let seg_name_generator = AtomicI64::new(0);
    let merge_context = MockMergeContext::del_count();
    let mut segment_infos = new_segment_infos()?;
    // 1 100-docs segment
    segment_infos.add(make_segment_commit_info(
        &next_name(&seg_name_generator),
        100,
        0,
        0.0,
        SOURCE_MERGE,
    )?)?;
    // 9 1-doc segments again
    for _ in 0..9 {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            1,
            0,
            0.0,
            SOURCE_FLUSH,
        )?)?;
    }
    // Ensure though we're below the floor size, the merge would be too unbalanced
    let spec = merge_policy.find_merges(MergeTrigger::Explicit, &segment_infos, &merge_context)?;
    assert_null(&spec);

    // another 1-doc segment, now we can merge 10 1-doc segments
    segment_infos.add(make_segment_commit_info(
        &next_name(&seg_name_generator),
        1,
        0,
        0.0,
        SOURCE_FLUSH,
    )?)?;
    let spec = merge_policy.find_merges(MergeTrigger::Explicit, &segment_infos, &merge_context)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked above");
    segment_infos = apply_all(segment_infos, &spec, &seg_name_generator, &mut stats)?;
    assert_equals(&2, &segment_infos.size());
    assert_equals(&100, &max_doc(&segment_infos, 0)?);
    assert_equals(&10, &max_doc(&segment_infos, 1)?);
    Ok(())
}

/// Port of `TestLogMergePolicy.testPackLargeSegments`.
fn test_pack_large_segments() -> Result<()> {
    let merge_policy = LogMergePolicy::by_doc_count();
    let mut stats = IOStats::default();
    merge_policy.set_max_merge_docs(10_000);
    let seg_name_generator = AtomicI64::new(0);
    let merge_context = MockMergeContext::del_count();
    let mut segment_infos = new_segment_infos()?;
    // 10 segments below the max segment size, but larger than maxMergeSize/mergeFactor
    for _ in 0..10 {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            3_000,
            0,
            0.0,
            SOURCE_MERGE,
        )?)?;
    }
    let spec = merge_policy.find_merges(MergeTrigger::Explicit, &segment_infos, &merge_context)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked above");
    segment_infos = apply_all(segment_infos, &spec, &seg_name_generator, &mut stats)?;
    // LogMP packed 3 3k segments together
    assert_equals(&9_000, &max_doc(&segment_infos, 0)?);
    Ok(())
}

/// Port of `TestLogMergePolicy.testIgnoreLargeSegments`.
fn test_ignore_large_segments() -> Result<()> {
    let merge_policy = LogMergePolicy::by_doc_count();
    let mut stats = IOStats::default();
    merge_policy.set_max_merge_docs(10_000);
    let seg_name_generator = AtomicI64::new(0);
    let merge_context = MockMergeContext::del_count();
    let mut segment_infos = new_segment_infos()?;
    // 1 segment that reached the maximum segment size
    segment_infos.add(make_segment_commit_info(
        &next_name(&seg_name_generator),
        11_000,
        0,
        0.0,
        SOURCE_MERGE,
    )?)?;
    // and 10 segments below the max segment size, but within the same level
    for _ in 0..10 {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            2_000,
            0,
            0.0,
            SOURCE_MERGE,
        )?)?;
    }
    // LogMergePolicy used to have a bug that would make it exclude the first mergeFactor segments
    // from merging if any of them was above the maximum merged size
    let spec = merge_policy.find_merges(MergeTrigger::Explicit, &segment_infos, &merge_context)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked above");
    segment_infos = apply_all(segment_infos, &spec, &seg_name_generator, &mut stats)?;
    assert_equals(&11_000, &max_doc(&segment_infos, 0)?);
    assert_equals(&10_000, &max_doc(&segment_infos, 1)?);
    Ok(())
}

/// Port of `TestLogMergePolicy.testFullFlushMerges`.
fn test_full_flush_merges(this: &dyn BaseMergePolicyTestCase) -> Result<()> {
    let seg_name_generator = AtomicI64::new(0);
    let mut stats = IOStats::default();
    let merge_context = MockMergeContext::del_count();
    let mut segment_infos = new_segment_infos()?;

    let policy = this.merge_policy();
    let mp = as_log(&*policy);
    // Number of segments guaranteed to trigger a merge.
    let num_segments_for_merging = mp.get_merge_factor() + mp.get_target_search_concurrency();

    for _ in 0..num_segments_for_merging {
        segment_infos.add(make_segment_commit_info(
            &next_name(&seg_name_generator),
            1,
            0,
            // `Double.MIN_VALUE`: the smallest positive (subnormal) double.
            f64::from_bits(1),
            SOURCE_FLUSH,
        )?)?;
    }
    let spec =
        mp.find_full_flush_merges(MergeTrigger::FullFlush, &segment_infos, &merge_context)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked above");
    segment_infos = apply_all(segment_infos, &spec, &seg_name_generator, &mut stats)?;
    Assert::assert_true((segment_infos.size() as i32) < num_segments_for_merging);
    Ok(())
}
