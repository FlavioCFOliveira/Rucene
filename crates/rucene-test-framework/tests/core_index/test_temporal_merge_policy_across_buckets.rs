//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTemporalMergePolicyAcrossBuckets.java`.
//!
//! Comprehensive test to ensure TemporalMergePolicy NEVER merges segments from different time
//! buckets, even under aggressive force merge scenarios.
//!
//! This test creates multiple segments across different year buckets and verifies that they
//! remain in separate buckets after force merge operations.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, LongPoint, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::TemporalMergePolicy;
use rucene::index::IndexReader;
use rucene::store::{ByteBuffersDirectory, Directory};

use crate::core_support::{assert_equals_msg, class, test};
use crate::test_temporal_merge_policy::close_directory;

/// `TestTemporalMergePolicyAcrossBuckets`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTemporalMergePolicyAcrossBuckets",
        vec![
            test(
                "testMultipleYearBucketsNeverMerge",
                test_multiple_year_buckets_never_merge,
            ),
            test(
                "testTenYearBucketsRemainSeparate",
                test_ten_year_buckets_remain_separate,
            ),
            test(
                "testOldDataBucketDoesNotMergeAcrossYearBoundaries",
                test_old_data_bucket_does_not_merge_across_year_boundaries,
            ),
            test(
                "testOldDataBucketWithMultipleOldSegments",
                test_old_data_bucket_with_multiple_old_segments,
            ),
        ],
    )
}

/// `TimeUnit.DAYS.toSeconds(days)`.
fn days_to_seconds(days: i64) -> i64 {
    days * 24 * 3600
}

/// `TimeUnit.DAYS.toMillis(days)`.
fn days_to_millis(days: i64) -> i64 {
    days_to_seconds(days) * 1000
}

/// The policy every test builds, with `maxAgeSeconds` of `max_age_years`
/// years of 365 days.
fn policy(max_age_years: i64) -> Result<TemporalMergePolicy> {
    let mut policy = TemporalMergePolicy::new();
    policy
        .set_temporal_field("timestamp_tl")?
        .set_base_time_in_seconds(days_to_seconds(365))?
        .set_max_age_seconds(days_to_seconds(365 * max_age_years))?
        .set_min_threshold(2)?
        .set_max_threshold(10)?
        .disable_exponential_buckets()
        .set_compaction_ratio(1.0)?;
    Ok(policy)
}

/// `new IndexWriter(dir, config)` with `policy` and `setMaxBufferedDocs(max_buffered_docs)`.
fn writer(
    dir: &Arc<dyn Directory>,
    policy: TemporalMergePolicy,
    max_buffered_docs: i32,
) -> Result<Arc<IndexWriter>> {
    let config = IndexWriterConfig::new();
    config.set_merge_policy(Some(Arc::new(policy)))?;
    config.set_max_buffered_docs(max_buffered_docs)?;
    IndexWriter::new(Arc::clone(dir), config)
}

/// A document with `timestamp * 1000` as a `LongPoint` and a `NumericDocValuesField`.
fn timestamp_doc(timestamp: i64) -> Result<Document> {
    let mut d = Document::new();
    d.add(Box::new(LongPoint::new(
        "timestamp_tl",
        &[timestamp * 1000],
    )?));
    d.add(Box::new(NumericDocValuesField::new(
        "timestamp_tl",
        timestamp * 1000,
    )));
    Ok(d)
}

/// Port of `TestTemporalMergePolicyAcrossBuckets.testMultipleYearBucketsNeverMerge`.
fn test_multiple_year_buckets_never_merge() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    let writer = writer(&dir, policy(10)?, 1000)?;

    // Create segments for 5 different years: 2020, 2021, 2022, 2023, 2024
    // Each year gets 3 segments with unique timestamps
    let year2020 = 1577836800000i64; // 2020-01-01 00:00:00 UTC
    let year2021 = 1609459200000i64; // 2021-01-01 00:00:00 UTC
    let year2022 = 1640995200000i64; // 2022-01-01 00:00:00 UTC
    let year2023 = 1672531200000i64; // 2023-01-01 00:00:00 UTC
    let year2024 = 1704067200000i64; // 2024-01-01 00:00:00 UTC

    let years = [year2020, year2021, year2022, year2023, year2024];

    for year_timestamp in years {
        for seg_in_year in 0..3i64 {
            for doc in 0..10i64 {
                let timestamp = year_timestamp + (seg_in_year * 1000000) + (doc * 1000);
                writer.add_document(&timestamp_doc(timestamp)?)?;
            }
            writer.flush()?;
        }
    }

    writer.force_merge_with(1, true)?;
    writer.commit()?;

    let reader = directory_reader::open(Arc::clone(&dir))?;
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let final_segment_count = leaves.len();

    for (i, ctx) in leaves.iter().enumerate() {
        let point_values = ctx.leaf_reader().get_point_values("timestamp_tl")?;

        if let Some(point_values) = point_values {
            let min_value = point_values.min_packed_value()?.unwrap_or_default();
            let max_value = point_values.max_packed_value()?.unwrap_or_default();

            let min_timestamp = LongPoint::decode_dimension(&min_value, 0) / 1000;
            let max_timestamp = LongPoint::decode_dimension(&max_value, 0) / 1000;

            let min_year_bucket = (min_timestamp / 1000) / days_to_seconds(365);
            let max_year_bucket = (max_timestamp / 1000) / days_to_seconds(365);

            // Verify segment doesn't span multiple year buckets
            // Allow up to 1 year difference (since we're using 1-year buckets)
            let bucket_span = (max_year_bucket - min_year_bucket).abs();
            Assert::assert_true_msg(
                &format!(
                    "Segment {i} spans {bucket_span} year buckets (should be ≤ 1). \
                     This indicates cross-bucket merging occurred!"
                ),
                bucket_span <= 1,
            );
        }
    }

    reader.close()?;
    writer.close()?;
    drop((leaves, reader, writer));
    close_directory(dir)?;

    // With 5 year buckets and 3 segments per bucket, after merging:
    // - Within each bucket: 3 segments should merge into 1 (minThreshold=2)
    // - Across buckets: NEVER merge
    // Expected: 5 segments (one per year bucket)

    // Could be more if segments within a bucket didn't meet merge criteria
    Assert::assert_true_msg(
        &format!(
            "Expected at least 5 segments (one per year bucket), but got {final_segment_count}. \
             This indicates segments from different year buckets were merged together!"
        ),
        final_segment_count >= 5,
    );
    Ok(())
}

/// Opens `dir`, counts its leaves, and closes everything.
fn final_segment_count(dir: Arc<dyn Directory>, writer: Arc<IndexWriter>) -> Result<usize> {
    let reader = directory_reader::open(Arc::clone(&dir))?;
    let final_segment_count = (Arc::clone(&reader) as Arc<dyn IndexReader>)
        .leaves()?
        .len();

    reader.close()?;
    writer.close()?;
    drop((reader, writer));
    close_directory(dir)?;
    Ok(final_segment_count)
}

/// Port of `TestTemporalMergePolicyAcrossBuckets.testTenYearBucketsRemainSeparate`.
fn test_ten_year_buckets_remain_separate() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    // 1-year buckets, 10 years
    let writer = writer(&dir, policy(10)?, 1000)?;

    let base_year = 1577836800000i64; // 2020-01-01 00:00:00 UTC
    let one_year_millis = days_to_millis(365);

    for year in 0..10i64 {
        let year_timestamp = base_year + (year * one_year_millis);

        for seg in 0..2i64 {
            for doc in 0..10i64 {
                let timestamp = year_timestamp + (seg * 1000000) + (doc * 1000);
                writer.add_document(&timestamp_doc(timestamp)?)?;
            }
            writer.flush()?;
        }
    }

    writer.force_merge_with(1, true)?;
    writer.commit()?;

    let final_segment_count = final_segment_count(dir, writer)?;

    // With 10 year buckets and minThreshold=2, each bucket should merge its 2 segments into 1
    // Expected: exactly 10 segments (one per year bucket)
    Assert::assert_true_msg(
        &format!(
            "Expected at least 10 segments (one per year bucket), but got {final_segment_count}"
        ),
        final_segment_count >= 10,
    );
    Ok(())
}

/// Port of `TestTemporalMergePolicyAcrossBuckets.testOldDataBucketDoesNotMergeAcrossYearBoundaries`.
fn test_old_data_bucket_does_not_merge_across_year_boundaries() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    let writer = writer(&dir, policy(10)?, 100)?;

    // Segment A: Contains data from 1970 to 2024 (max_timestamp = 2024)
    // Should be assigned to 2024 bucket
    let timestamp2024 = 1735689600000i64; // 2025-01-01 00:00:00 UTC in millis
    let timestamp1970 = 0i64;

    for i in 0..10i64 {
        let timestamp = timestamp1970 + (i * 1000); // Old data
        writer.add_document(&timestamp_doc(timestamp)?)?;
    }
    // Add one recent doc to set max_timestamp = 2024
    writer.add_document(&timestamp_doc(timestamp2024)?)?;
    writer.flush()?;

    // Segment B: Contains data from 1970 to 2023 (max_timestamp = 2023)
    // Should be assigned to 2023 bucket
    let timestamp2023 = 1704067200000i64; // 2024-01-01 00:00:00 UTC in millis

    for i in 0..10i64 {
        let timestamp = timestamp1970 + (i * 2000); // Old data
        writer.add_document(&timestamp_doc(timestamp)?)?;
    }
    // Add one recent doc to set max_timestamp = 2023
    writer.add_document(&timestamp_doc(timestamp2023)?)?;
    writer.flush()?;

    // Trigger merging - this should NOT merge segments A and B together
    // because they're in different year buckets
    writer.force_merge_with(1, true)?;
    writer.commit()?;

    // Check final segment count
    let final_segment_count = final_segment_count(dir, writer)?;

    // Assert that segments from different buckets don't merge
    assert_equals_msg(
        Some("Segments from different year buckets should NOT be merged together"),
        &2,
        &final_segment_count,
    );
    Ok(())
}

/// Port of `TestTemporalMergePolicyAcrossBuckets.testOldDataBucketWithMultipleOldSegments`.
fn test_old_data_bucket_with_multiple_old_segments() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    let writer = writer(&dir, policy(3)?, 100)?;

    let now = 1769540949799i64;
    let five_years_ago = now - days_to_millis(365 * 5);

    // Create 3 segments, all containing data older than 3 years
    // These should all go into bucket -1 "old data bucket" which is NOT merged
    for seg in 0..3i64 {
        for i in 0..10i64 {
            let timestamp = five_years_ago + (seg * 1000000) + (i * 1000);
            writer.add_document(&timestamp_doc(timestamp)?)?;
        }
        writer.flush()?;
    }

    writer.force_merge_with(1, true)?;
    writer.commit()?;

    let final_segment_count = final_segment_count(dir, writer)?;

    // Old data segments (older than maxAgeSeconds) should NOT merge - they get skipped
    assert_equals_msg(
        Some("Old data segments (all >3 years) should NOT be merged (skipped)"),
        &3,
        &final_segment_count,
    );
    Ok(())
}
