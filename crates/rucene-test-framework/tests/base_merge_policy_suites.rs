//! The `BaseMergePolicyTestCase` suites (rmp #355): the inherited cases of
//! `BaseMergePolicyTestCase` run, under the ported runner and
//! `LuceneTestCase`'s rule chain, on the merge policies Lucene's own
//! subclasses bind, with their overrides of `mergePolicy()`,
//! `assertSegmentInfos`, `assertMerge` and the simulations:
//!
//! * `org.apache.lucene.index.TestUpgradeIndexMergePolicy`: an
//!   `UpgradeIndexMergePolicy` over `newTieredMergePolicy()` with a max
//!   merged segment of 1 to 10 GB; both assertions are no-ops;
//! * `org.apache.lucene.index.TestNoMergePolicy`: `NoMergePolicy.INSTANCE`;
//!   every segment comes from a flush and no merge is ever selected; the
//!   simulations run on fewer documents;
//! * `org.apache.lucene.index.TestTieredMergePolicy`: `newTieredMergePolicy()`,
//!   with the segment-count and merge-size assertions; the simulations run
//!   with a max merged segment of 1 to 10 GB;
//! * `org.apache.lucene.index.TestLogMergePolicy`: `newLogMergePolicy(random())`,
//!   with the size assertions.
//!
//! The classes' own test methods (`testNoMergePolicy`, `testForceMergeDeletes`,
//! `testIncreasingSegmentSizes`, …) belong to the core-test port (sprint 7)
//! and are not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::index_writer::SOURCE_FLUSH;
use rucene::index::merge_policy::{
    LogMergePolicy, MergePolicy, MergeSpecification, NoMergePolicy, TieredMergePolicy,
    UpgradeIndexMergePolicy,
};
use rucene::index::segment_infos::SegmentInfos;
use rucene_test_framework::index::base_merge_policy_test_case::{
    self, fail, BaseMergePolicyTestCase, MergePolicyTestInstance, MockMergeContext, SOURCE,
};
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// `(TieredMergePolicy) policy`.
fn as_tiered(policy: &dyn MergePolicy) -> &TieredMergePolicy {
    policy
        .as_any()
        .and_then(|a| a.downcast_ref::<TieredMergePolicy>())
        .unwrap_or_else(|| fail("java.lang.ClassCastException: not a TieredMergePolicy"))
}

/// `(LogMergePolicy) policy`.
fn as_log(policy: &dyn MergePolicy) -> &LogMergePolicy {
    policy
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| fail("java.lang.ClassCastException: not a LogMergePolicy"))
}

// -----------------------------------------------------------------------------
// TestUpgradeIndexMergePolicy
// -----------------------------------------------------------------------------

/// `TestUpgradeIndexMergePolicy`.
struct UpgradeIndexSuite;

impl BaseMergePolicyTestCase for UpgradeIndexSuite {
    fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        let r#in = LuceneTestCase::new_tiered_merge_policy();
        // Avoid low values of the max merged segment size which prevent this merge policy from
        // scaling well
        r#in.set_max_merged_segment_mb(f64::from(TestUtil::next_int(
            &*LuceneTestCase::random(),
            1024,
            10 * 1024,
        )))
        .unwrap_or_else(|e| base_merge_policy_test_case::fail(&e.to_string()));
        Arc::new(UpgradeIndexMergePolicy::new(Arc::new(r#in)))
    }

    fn assert_segment_infos(&self, _policy: &dyn MergePolicy, _infos: &SegmentInfos) -> Result<()> {
        // no-op
        Ok(())
    }

    fn assert_merge(&self, _policy: &dyn MergePolicy, _merge: &MergeSpecification) -> Result<()> {
        // no-op
        Ok(())
    }
}

fn upgrade_index_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestUpgradeIndexMergePolicy")
        .extends(&base_merge_policy_test_case::class())
        .constructor(Constructor::no_args(|| {
            MergePolicyTestInstance::new(Arc::new(UpgradeIndexSuite))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// TestNoMergePolicy
// -----------------------------------------------------------------------------

/// `TestNoMergePolicy`.
struct NoMergeSuite;

impl BaseMergePolicyTestCase for NoMergeSuite {
    fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        Arc::new(NoMergePolicy::new())
    }

    fn assert_segment_infos(&self, _policy: &dyn MergePolicy, infos: &SegmentInfos) -> Result<()> {
        for info in infos.iter() {
            Assert::assert_equals_str(
                None,
                Some(SOURCE_FLUSH),
                info.info.get_attribute(SOURCE).as_deref(),
            );
        }
        Ok(())
    }

    fn assert_merge(&self, _policy: &dyn MergePolicy, _merge: &MergeSpecification) -> Result<()> {
        Assert::fail_without_message() // should never happen
    }

    fn test_simulate_append_only(&self) -> Result<()> {
        // Reduce numbers as this merge policy doesn't work well with lots of data
        self.do_test_simulate_append_only(&*self.merge_policy(), 1_000_000, 10_000)
    }

    fn test_simulate_updates(&self) -> Result<()> {
        // Reduce numbers as this merge policy doesn't work well with lots of data
        self.do_test_simulate_updates(&*self.merge_policy(), 100_000, 1000)
    }
}

fn no_merge_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestNoMergePolicy")
        .extends(&base_merge_policy_test_case::class())
        .constructor(Constructor::no_args(|| {
            MergePolicyTestInstance::new(Arc::new(NoMergeSuite))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// TestTieredMergePolicy
// -----------------------------------------------------------------------------

/// `String.format(Locale.ROOT, "%g", value)`: six significant digits, in
/// scientific notation outside `[1e-4, 1e6)`.
fn java_g(value: f64) -> String {
    if value == 0.0 {
        return "0.00000".to_string();
    }
    let abs = value.abs();
    if (1e-4..1e6).contains(&abs) {
        let exponent = abs.log10().floor() as i32;
        let decimals = (5 - exponent).max(0) as usize;
        format!("{value:.decimals$}")
    } else {
        let s = format!("{value:.5e}");
        let (mantissa, exponent) = s.split_once('e').expect("scientific");
        let exponent: i32 = exponent.parse().expect("exponent");
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exponent.abs())
    }
}

/// `String.format(Locale.ROOT, "%,d", value)`.
fn java_grouped(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if value < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// `TestTieredMergePolicy`.
struct TieredSuite;

/// The record `DocCountAndSizeInBytes(int docCount, long sizeInBytes)`.
#[derive(Debug, Clone, Copy)]
struct DocCountAndSizeInBytes {
    doc_count: i32,
    size_in_bytes: i64,
}

impl TieredSuite {
    /// `mergePolicy()`, typed: `newTieredMergePolicy()`.
    fn tiered(&self) -> TieredMergePolicy {
        LuceneTestCase::new_tiered_merge_policy()
    }
}

impl BaseMergePolicyTestCase for TieredSuite {
    fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        Arc::new(self.tiered())
    }

    fn assert_segment_infos(&self, policy: &dyn MergePolicy, infos: &SegmentInfos) -> Result<()> {
        let tmp = as_tiered(policy);

        let max_merged_segment_bytes = (tmp.get_max_merged_segment_mb() * 1024.0 * 1024.0) as i64;

        let mut min_segment_bytes = i64::MAX;
        let mut total_del_count = 0i32;
        let mut total_max_doc = 0i32;
        let mut total_bytes = 0i64;
        let mut segment_sizes: Vec<DocCountAndSizeInBytes> = Vec::new();
        for sci in infos.iter() {
            total_del_count += sci.get_del_count();
            total_max_doc += sci.info.max_doc()?;
            let byte_size = sci.size_in_bytes_uncached()?;
            let live_ratio = 1.0 - f64::from(sci.get_del_count()) / f64::from(sci.info.max_doc()?);
            let weighted_byte_size = (live_ratio * byte_size as f64) as i64;
            total_bytes += weighted_byte_size;
            segment_sizes.push(DocCountAndSizeInBytes {
                doc_count: sci.info.max_doc()? - sci.get_del_count(),
                size_in_bytes: weighted_byte_size,
            });
            min_segment_bytes = min_segment_bytes.min(weighted_byte_size);
        }
        // `Collections.sort`, stable, by size.
        segment_sizes.sort_by_key(|s| s.size_in_bytes);

        let del_percentage = 100.0 * f64::from(total_del_count) / f64::from(total_max_doc);
        Assert::assert_true_msg(
            &format!(
                "Percentage of deleted docs {} is larger than the target: {}",
                rucene::util::java_fmt::double_to_string(del_percentage),
                rucene::util::java_fmt::double_to_string(tmp.get_deletes_pct_allowed())
            ),
            del_percentage <= tmp.get_deletes_pct_allowed(),
        );

        let mut level_size_bytes =
            min_segment_bytes.max((tmp.get_floor_segment_mb() * 1024.0 * 1024.0) as i64);
        let mut bytes_left = total_bytes;
        let mut allowed_seg_count = 0f64;
        let mut biggest_segments: &[DocCountAndSizeInBytes] = &segment_sizes;
        let concurrency = tmp.get_target_search_concurrency() as usize;
        if biggest_segments.len() > concurrency.wrapping_sub(1) {
            biggest_segments = &biggest_segments[biggest_segments.len() - concurrency + 1..];
        }
        // Allow whole segments for the targetSearchConcurrency-1 biggest segments
        for size in biggest_segments {
            bytes_left -= size.size_in_bytes;
            allowed_seg_count += 1.0;
        }

        let mut too_big_count = 0i32;
        for size in &segment_sizes {
            if size.size_in_bytes >= max_merged_segment_bytes / 2 {
                too_big_count += 1;
            }
        }

        // below we make the assumption that segments that reached the max segment
        // size divided by 2 don't need merging anymore
        let merge_factor = tmp
            .get_segments_per_tier()
            .min(f64::from(tmp.get_max_merge_at_once())) as i32;
        loop {
            let seg_count_level = bytes_left as f64 / level_size_bytes as f64;
            if seg_count_level <= tmp.get_segments_per_tier()
                || level_size_bytes >= max_merged_segment_bytes / 2
            {
                allowed_seg_count += seg_count_level.ceil();
                break;
            }
            allowed_seg_count += tmp.get_segments_per_tier();
            // `bytesLeft -= tmp.getSegmentsPerTier() * levelSizeBytes`
            bytes_left =
                (bytes_left as f64 - tmp.get_segments_per_tier() * level_size_bytes as f64) as i64;
            level_size_bytes = (level_size_bytes.wrapping_mul(i64::from(merge_factor)))
                .min(max_merged_segment_bytes / 2);
        }
        // Allow at least a full tier in addition of the too big segments.
        allowed_seg_count =
            allowed_seg_count.max(f64::from(too_big_count) + tmp.get_segments_per_tier());
        // Allow at least `targetSearchConcurrency` segments.
        allowed_seg_count = allowed_seg_count.max(f64::from(tmp.get_target_search_concurrency()));

        // It's ok to be over the allowed segment count if none of the merges are legal, because
        // they are either not balanced or because they exceed the max merged segment doc count.
        // We only check pairwise merges instead of every possible merge to keep things simple. If
        // none of the pairwise merges are legal, chances are high that no merge is legal.
        let max_docs_per_segment = tmp.max_allowed_docs(infos.total_max_doc()?, total_del_count);
        let mut has_legal_merges = false;
        for pair in segment_sizes.windows(2) {
            let (size1, size2) = (pair[0], pair[1]);
            let merged_segment_size_in_bytes = size1.size_in_bytes + size2.size_in_bytes;
            let merged_segment_doc_count = size1.doc_count + size2.doc_count;

            if merged_segment_size_in_bytes <= max_merged_segment_bytes
                && size2.size_in_bytes as f64 * 1.5 <= merged_segment_size_in_bytes as f64
                && merged_segment_doc_count <= max_docs_per_segment
            {
                has_legal_merges = true;
                break;
            }
        }

        let num_segments = infos.as_list().len();
        Assert::assert_true_msg(
            &format!(
                "mergeFactor={merge_factor} minSegmentBytes={} maxMergedSegmentBytes={} segmentsPerTier={} maxMergeAtOnce={} numSegments={num_segments} allowed={} totalBytes={} delPercentage={} deletesPctAllowed={} targetNumSegments={}",
                java_grouped(min_segment_bytes),
                java_grouped(max_merged_segment_bytes),
                java_g(tmp.get_segments_per_tier()),
                tmp.get_max_merge_at_once(),
                java_g(allowed_seg_count),
                java_grouped(total_bytes),
                java_g(del_percentage),
                java_g(tmp.get_deletes_pct_allowed()),
                tmp.get_target_search_concurrency()
            ),
            num_segments as f64 <= allowed_seg_count || !has_legal_merges,
        );
        Ok(())
    }

    fn assert_merge(&self, policy: &dyn MergePolicy, merges: &MergeSpecification) -> Result<()> {
        let tmp = as_tiered(policy);
        for merge in &merges.merges {
            Assert::assert_true(merge.segments.len() as i32 <= tmp.get_max_merge_at_once());
        }
        Ok(())
    }

    fn test_simulate_append_only(&self) -> Result<()> {
        let merge_policy = self.tiered();
        // Avoid low values of the max merged segment size which prevent this merge policy from
        // scaling well
        merge_policy.set_max_merged_segment_mb(f64::from(TestUtil::next_int(
            &*LuceneTestCase::random(),
            1024,
            10 * 1024,
        )))?;
        self.do_test_simulate_append_only(&merge_policy, 100_000_000, 10_000)
    }

    fn test_simulate_updates(&self) -> Result<()> {
        let merge_policy = self.tiered();
        // Avoid low values of the max merged segment size which prevent this merge policy from
        // scaling well
        merge_policy.set_max_merged_segment_mb(f64::from(TestUtil::next_int(
            &*LuceneTestCase::random(),
            1024,
            10 * 1024,
        )))?;
        let num_docs = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(10_000_000)
        } else {
            LuceneTestCase::at_least(1_000_000)
        };
        self.do_test_simulate_updates(&merge_policy, num_docs, 2500)
    }
}

fn tiered_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestTieredMergePolicy")
        .extends(&base_merge_policy_test_case::class())
        .constructor(Constructor::no_args(|| {
            MergePolicyTestInstance::new(Arc::new(TieredSuite))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// TestLogMergePolicy
// -----------------------------------------------------------------------------

/// `TestLogMergePolicy`.
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

fn log_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestLogMergePolicy")
        .extends(&base_merge_policy_test_case::class())
        .constructor(Constructor::no_args(|| {
            MergePolicyTestInstance::new(Arc::new(LogSuite))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestMergePolicySuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestMergePolicySuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestMergePolicySuiteDraws` of `BaseFormatSuitesOracle`: the binding of
/// `TestTieredMergePolicy`, recording the next `random().nextLong()` after
/// each test (in `tearDown`, before `super.tearDown()`) and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestMergePolicySuiteDraws")
        .extends(&base_merge_policy_test_case::class())
        .constructor(Constructor::no_args(|| {
            MergePolicyTestInstance::new(Arc::new(TieredSuite))
        }))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|_inst: &mut MergePolicyTestInstance| {
                    DRAWS.lock().unwrap().push(format!(
                        "{}\tnext\t{}",
                        LuceneTestCase::get_test_name().unwrap_or_default(),
                        LuceneTestCase::random().next_long()
                    ));
                    LuceneTestCase::super_tear_down();
                }),
        )
        .method(
            Method::new("recordClassDraw")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(randomizedtesting::junit::AfterClass)
                .static_body(|| {
                    DRAWS.lock().unwrap().push(format!(
                        "#class\tnext\t{}",
                        LuceneTestCase::random().next_long()
                    ));
                }),
        )
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #355): every row of
/// `base_merge_policy_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_merge_policy_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_merge_policy_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_upgrade_index_merge_policy() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(upgrade_index_class);
}

#[test]
fn test_no_merge_policy() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(no_merge_class);
}

#[test]
fn test_tiered_merge_policy() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(tiered_class);
}

#[test]
fn test_log_merge_policy() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(log_class);
}
