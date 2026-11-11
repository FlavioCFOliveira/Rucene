//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTemporalMergePolicy.java`.
//!
//! `Map<SegmentCommitInfo, …>` keys are segment names, the identity
//! `TemporalMergePolicy`, `MergePolicy` and `MergeContext` use in this port
//! (`SegmentCommitInfo` is not hashable here); a `Set<SegmentCommitInfo>` is
//! the set of the segments' names.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs;
use rucene::document::{Document, LongPoint, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_file_names::segment_file_name;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{
    MergeContext, MergePolicy, SegmentDateRange, TemporalMergePolicy,
};
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::{IndexReader, SegmentCommitInfo, SegmentInfo};
use rucene::search::Sort;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::string_helper::StringHelper;
use rucene::util::{InfoStream, NoOutputInfoStream, Version};

use crate::core_support::{assert_equals, assert_equals_msg, assert_not_null, class, test};
use crate::support::{assert_null_msg, npe};

/// `TestTemporalMergePolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTemporalMergePolicy",
        vec![
            test(
                "testSkipsMergeWhenCompactionRatioNotMet",
                test_skips_merge_when_compaction_ratio_not_met,
            ),
            test(
                "testMergesSubsetWhenRatioSatisfied",
                test_merges_subset_when_ratio_satisfied,
            ),
            test(
                "testForcedMergeReducesSegments",
                test_forced_merge_reduces_segments,
            ),
            test(
                "testForcedDeletesMergesRespectThreshold",
                test_forced_deletes_merges_respect_threshold,
            ),
            test(
                "testForcedMergeKeepsWindowsDisjoint",
                test_forced_merge_keeps_windows_disjoint,
            ),
            test(
                "testForcedDeletesKeepWindowsDisjoint",
                test_forced_deletes_keep_windows_disjoint,
            ),
            test(
                "testMergesWorkWithUncommittedSegments",
                test_merges_work_with_uncommitted_segments,
            ),
        ],
    )
}

/// `new TemporalMergePolicy.SegmentDateRange(min, max)`.
fn date_range(min_date: i64, max_date: i64) -> SegmentDateRange {
    SegmentDateRange { min_date, max_date }
}

/// `TreeMap<Long, List<SegmentCommitInfo>>` with the single bucket `0L`.
fn single_bucket(segments: &[&SegmentCommitInfo]) -> BTreeMap<i64, Vec<SegmentCommitInfo>> {
    let mut buckets = BTreeMap::new();
    buckets.insert(0i64, segments.iter().map(|s| (*s).clone()).collect());
    buckets
}

/// The name of a segment, its key in this port's maps.
fn key(sci: &SegmentCommitInfo) -> String {
    sci.info.name.clone()
}

/// Port of `TestTemporalMergePolicy.testSkipsMergeWhenCompactionRatioNotMet`.
fn test_skips_merge_when_compaction_ratio_not_met() -> Result<()> {
    let mut policy = build_policy()?;
    policy
        .set_min_threshold(2)?
        .set_max_threshold(4)?
        .set_compaction_ratio(2.0)?;

    let large = new_segment("large", 1000)?;
    let small_one = new_segment("small1", 100)?;
    let small_two = new_segment("small2", 100)?;

    let mut ranges = HashMap::new();
    ranges.insert(key(&large), date_range(0, 3000));
    ranges.insert(key(&small_one), date_range(0, 2000));
    ranges.insert(key(&small_two), date_range(0, 1000));

    let buckets = single_bucket(&[&large, &small_one, &small_two]);

    let spec = policy.find_merge_candidates(buckets, &ranges)?;
    assert_null_msg(
        "Merge should wait for more small segments to accumulate",
        &spec,
    );
    Ok(())
}

/// Port of `TestTemporalMergePolicy.testMergesSubsetWhenRatioSatisfied`.
fn test_merges_subset_when_ratio_satisfied() -> Result<()> {
    let mut policy = build_policy()?;
    policy
        .set_min_threshold(2)?
        .set_max_threshold(3)?
        .set_compaction_ratio(1.6)?;

    let seg_a = new_segment("segA", 400)?;
    let seg_b = new_segment("segB", 200)?;
    let seg_c = new_segment("segC", 200)?;
    let seg_d = new_segment("segD", 200)?;
    let seg_e = new_segment("segE", 200)?;

    let segments = [&seg_a, &seg_b, &seg_c, &seg_d, &seg_e];
    let mut ranges = HashMap::new();
    let mut timestamp = 5000i64;
    for segment in segments {
        ranges.insert(key(segment), date_range(timestamp - 100, timestamp));
        timestamp -= 100;
    }

    let buckets = single_bucket(&segments);

    let spec = policy.find_merge_candidates(buckets, &ranges)?;
    assert_not_null(&spec);
    let spec = spec.expect("checked");
    assert_equals(&2, &spec.merges.len());

    assert_equals(&3, &spec.merges[0].segments.len());
    assert_equals(&2, &spec.merges[1].segments.len());
    Ok(())
}

/// `new SegmentInfos(Version.LATEST.major)` holding `segments`.
fn infos_of(segments: &[&SegmentCommitInfo]) -> Result<SegmentInfos> {
    let mut infos = SegmentInfos::new(Version::LATEST.major as i32)?;
    for s in segments {
        infos.add((*s).clone())?;
    }
    Ok(infos)
}

/// `segmentsToMerge`, every segment mapped to `Boolean.TRUE`.
fn all_to_merge(segments: &[&SegmentCommitInfo]) -> HashMap<String, bool> {
    segments.iter().map(|s| (key(s), true)).collect()
}

/// Port of `TestTemporalMergePolicy.testForcedMergeReducesSegments`.
fn test_forced_merge_reduces_segments() -> Result<()> {
    let mut policy = build_policy()?;
    policy
        .disable_exponential_buckets()
        .set_base_time_in_seconds(10)?;
    let seg_a = new_segment("forceA", 400)?;
    let seg_b = new_segment("forceB", 300)?;
    let seg_c = new_segment("forceC", 200)?;

    let infos = infos_of(&[&seg_a, &seg_b, &seg_c])?;

    let segments_to_merge = all_to_merge(&[&seg_a, &seg_b, &seg_c]);

    let mut ranges = HashMap::new();
    ranges.insert(key(&seg_a), range(10000));
    ranges.insert(key(&seg_b), range(11000));
    ranges.insert(key(&seg_c), range(12000));
    policy.set_segment_date_range_overrides(ranges);

    let spec =
        policy.find_forced_merges(&infos, 1, &segments_to_merge, &StubMergeContext::new())?;
    assert_not_null(&spec);
    let spec = spec.expect("checked");
    Assert::assert_false(spec.merges.is_empty());
    assert_equals(&1, &spec.merges.len());
    assert_equals(
        &3,
        &spec
            .merges
            .first()
            .unwrap_or_else(|| npe("getFirst"))
            .segments
            .len(),
    );
    Ok(())
}

/// Port of `TestTemporalMergePolicy.testForcedDeletesMergesRespectThreshold`.
fn test_forced_deletes_merges_respect_threshold() -> Result<()> {
    let mut policy = build_policy()?;
    policy
        .set_force_merge_deletes_pct_allowed(5.0)?
        .disable_exponential_buckets()
        .set_base_time_in_seconds(10)?;

    let seg_a = new_segment("delA", 100)?;
    let seg_b = new_segment("delB", 100)?;
    let seg_c = new_segment("delC", 100)?;
    let infos = infos_of(&[&seg_a, &seg_b, &seg_c])?;

    let mut deletes = HashMap::new();
    deletes.insert(key(&seg_a), 10);
    deletes.insert(key(&seg_b), 1);
    deletes.insert(key(&seg_c), 20);

    let mut ranges = HashMap::new();
    ranges.insert(key(&seg_a), range(10000));
    ranges.insert(key(&seg_b), range(11000));
    ranges.insert(key(&seg_c), range(12000));
    policy.set_segment_date_range_overrides(ranges);

    let spec =
        policy.find_forced_deletes_merges(&infos, &StubMergeContext::with_deletes(deletes))?;

    assert_not_null(&spec);
    let spec = spec.expect("checked");
    assert_equals(&1, &spec.merges.len());
    assert_equals(
        &2,
        &spec
            .merges
            .first()
            .unwrap_or_else(|| npe("getFirst"))
            .segments
            .len(),
    );
    Ok(())
}

/// Port of `TestTemporalMergePolicy.testForcedMergeKeepsWindowsDisjoint`.
fn test_forced_merge_keeps_windows_disjoint() -> Result<()> {
    let mut policy = build_policy()?;
    policy
        .disable_exponential_buckets()
        .set_base_time_in_seconds(10)?;

    let seg_a1 = new_segment("bucketA1", 100)?;
    let seg_a2 = new_segment("bucketA2", 100)?;
    let seg_b1 = new_segment("bucketB1", 100)?;
    let seg_b2 = new_segment("bucketB2", 100)?;

    let infos = infos_of(&[&seg_a1, &seg_a2, &seg_b1, &seg_b2])?;

    let segments_to_merge = all_to_merge(&[&seg_a1, &seg_a2, &seg_b1, &seg_b2]);

    let mut ranges = HashMap::new();
    ranges.insert(key(&seg_a1), range(5000));
    ranges.insert(key(&seg_a2), range(6000));
    ranges.insert(key(&seg_b1), range(25000));
    ranges.insert(key(&seg_b2), range(26000));
    policy.set_segment_date_range_overrides(ranges);

    let spec =
        policy.find_forced_merges(&infos, 1, &segments_to_merge, &StubMergeContext::new())?;

    assert_not_null(&spec);
    let spec = spec.expect("checked");
    assert_equals_msg(Some("one merge per window"), &2, &spec.merges.len());

    let mut actual: BTreeSet<BTreeSet<String>> = BTreeSet::new();
    for merge in &spec.merges {
        actual.insert(merge.segments.iter().map(key).collect());
    }

    let mut expected: BTreeSet<BTreeSet<String>> = BTreeSet::new();
    expected.insert([key(&seg_a1), key(&seg_a2)].into_iter().collect());
    expected.insert([key(&seg_b1), key(&seg_b2)].into_iter().collect());

    assert_equals(&expected, &actual);
    Ok(())
}

/// Port of `TestTemporalMergePolicy.testForcedDeletesKeepWindowsDisjoint`.
fn test_forced_deletes_keep_windows_disjoint() -> Result<()> {
    let mut policy = build_policy()?;
    policy
        .set_force_merge_deletes_pct_allowed(5.0)?
        .disable_exponential_buckets()
        .set_base_time_in_seconds(10)?;

    let seg_a1 = new_segment("delA1", 100)?;
    let seg_a2 = new_segment("delA2", 100)?;
    let seg_b1 = new_segment("delB1", 100)?;

    let infos = infos_of(&[&seg_a1, &seg_a2, &seg_b1])?;

    let mut deletes = HashMap::new();
    deletes.insert(key(&seg_a1), 20);
    deletes.insert(key(&seg_a2), 15);
    deletes.insert(key(&seg_b1), 12);

    let mut ranges = HashMap::new();
    ranges.insert(key(&seg_a1), range(5000));
    ranges.insert(key(&seg_a2), range(6000));
    ranges.insert(key(&seg_b1), range(25000));
    policy.set_segment_date_range_overrides(ranges);

    let spec =
        policy.find_forced_deletes_merges(&infos, &StubMergeContext::with_deletes(deletes))?;

    assert_not_null(&spec);
    let spec = spec.expect("checked");
    assert_equals(&1, &spec.merges.len());
    let merged_segments: Vec<String> = spec
        .merges
        .first()
        .unwrap_or_else(|| npe("getFirst"))
        .segments
        .iter()
        .map(key)
        .collect();
    assert_equals(&2, &merged_segments.len());
    Assert::assert_true(merged_segments.contains(&key(&seg_a1)));
    Assert::assert_true(merged_segments.contains(&key(&seg_a2)));
    Ok(())
}

/// `private TemporalMergePolicy buildPolicy()`.
fn build_policy() -> Result<TemporalMergePolicy> {
    let mut policy = TemporalMergePolicy::new();
    policy.set_temporal_field("ts")?;
    Ok(policy)
}

/// `private SegmentCommitInfo newSegment(String name, int maxDoc)`.
fn new_segment(name: &str, max_doc: i32) -> Result<SegmentCommitInfo> {
    let segment_name = if name.starts_with('_') {
        name.to_lowercase()
    } else {
        format!("_{}", name.to_lowercase())
    };
    let info = SegmentInfo::new(
        Arc::new(ByteBuffersDirectory::new()) as Arc<dyn Directory>,
        Version::LATEST,
        Some(Version::LATEST),
        segment_name.clone(),
        max_doc,
        false,
        true,
        codecs::get_default().unwrap_or_else(|| npe("Codec.getDefault()")),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )?;
    info.set_files([segment_file_name(&segment_name, "", "si")])?;
    SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id())
}

/// Port of `TestTemporalMergePolicy.testMergesWorkWithUncommittedSegments`.
fn test_merges_work_with_uncommitted_segments() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    let mut policy = TemporalMergePolicy::new();
    policy
        .set_temporal_field("timestamp_tl")?
        .set_base_time_in_seconds(3600)?
        .set_min_threshold(3)?
        .set_max_threshold(4)?
        .set_compaction_ratio(1.0)?;

    let config = IndexWriterConfig::new();
    config.set_merge_policy(Some(Arc::new(policy)))?;
    config.set_max_buffered_docs(10)?;

    let writer = IndexWriter::new(Arc::clone(&dir), config)?;

    let now_millis = 1769540949799i64;
    let now_micros = now_millis * 1000;

    for _segment in 0..4 {
        for doc in 0..10i64 {
            let mut d = Document::new();
            let timestamp = now_micros + (doc * 1000000);
            d.add(Box::new(LongPoint::new("timestamp_tl", &[timestamp])?));
            d.add(Box::new(NumericDocValuesField::new(
                "timestamp_tl",
                timestamp,
            )));
            writer.add_document(&d)?;
        }
        writer.flush()?; // Flush to create segment but DON'T commit
    }

    // Force a merge check - this will use TemporalMergePolicy.findMerges()
    // which calls extractSegmentDateRanges() to read uncommitted segments
    writer.maybe_merge()?;

    writer.force_merge_with(1, true)?;

    writer.commit()?;

    let reader = directory_reader::open(Arc::clone(&dir))?;
    let final_segment_count = (Arc::clone(&reader) as Arc<dyn IndexReader>)
        .leaves()?
        .len();

    reader.close()?;
    writer.close()?;
    drop((reader, writer));
    close_directory(dir)?;

    Assert::assert_true_msg(
        "Temporal merging should work on uncommitted segments (Codec approach)",
        final_segment_count < 4,
    );
    Ok(())
}

/// `dir.close()` of the `ByteBuffersDirectory` the test created.
pub fn close_directory(dir: Arc<dyn Directory>) -> Result<()> {
    dir.close()
}

/// `private TemporalMergePolicy.SegmentDateRange range(long maxMillis)`.
fn range(max_millis: i64) -> SegmentDateRange {
    date_range(0, max_millis)
}

/// `private record StubMergeContext(Map<SegmentCommitInfo, Integer> deleteCounts,
/// Set<SegmentCommitInfo> mergingSegments) implements MergePolicy.MergeContext`.
struct StubMergeContext {
    delete_counts: HashMap<String, i32>,
    merging_segments: HashSet<String>,
}

impl StubMergeContext {
    fn new() -> Self {
        Self::with_deletes(HashMap::new())
    }

    fn with_deletes(delete_counts: HashMap<String, i32>) -> Self {
        Self {
            delete_counts,
            merging_segments: HashSet::new(),
        }
    }
}

impl MergeContext for StubMergeContext {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        Ok(*self.delete_counts.get(&key(info)).unwrap_or(&0))
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        *self.delete_counts.get(&key(info)).unwrap_or(&0)
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::new(NoOutputInfoStream)
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        self.merging_segments.clone()
    }
}
