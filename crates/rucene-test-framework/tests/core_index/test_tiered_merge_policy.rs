//! The own test methods of
//! `lucene/core/src/test/org/apache/lucene/index/TestTieredMergePolicy.java`.
//!
//! The class is bound once, in `base_merge_policy_suites.rs`, with its
//! `BaseMergePolicyTestCase` overrides (`mergePolicy`, `assertSegmentInfos`,
//! `assertMerge`, `testSimulateAppendOnly`, `testSimulateUpdates`); this
//! module holds the methods it declares itself, which that binding carries.
//! `Map<SegmentCommitInfo, Boolean>` and `Set<SegmentCommitInfo>` are keyed
//! by segment name, the identity `MergePolicy` and `MergeContext` use in this
//! port.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StoredField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, SOURCE_FLUSH, SOURCE_MERGE};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{
    MergePolicy, MergeSpecification, MergeTrigger, TieredMergePolicy,
};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReaderContext, SegmentCommitInfo};
use rucene::util::{BytesRef, Version};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::base_merge_policy_test_case::{
    apply_deletes, apply_merge, make_segment_commit_info, IOStats, MockMergeContext,
};
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

/// `random()`.
fn random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    LuceneTestCase::random()
}

/// `null` dereferenced.
fn npe<T>(what: &str) -> T {
    Throwable::new("java.lang.NullPointerException", what).throw()
}

/// `assertNull(x)` for a merge specification.
fn assert_null(spec: &Option<MergeSpecification>) {
    if spec.is_some() {
        Assert::fail("expected null, but was:<MergeSpecification>");
    }
}

/// `assertNotNull(x)` for a merge specification, unwrapped.
fn assert_not_null(spec: Option<MergeSpecification>) -> MergeSpecification {
    spec.unwrap_or_else(|| Assert::fail_without_message())
}

/// `new SegmentInfos(Version.LATEST.major)`.
fn new_infos() -> Result<SegmentInfos> {
    SegmentInfos::new(Version::LATEST.major as i32)
}

/// `info.sizeInBytes()`.
fn size_in_bytes(info: &SegmentCommitInfo) -> Result<i64> {
    info.clone().size_in_bytes()
}

/// `new MockMergeContext(SegmentCommitInfo::getDelCount)`.
fn del_count_context() -> MockMergeContext {
    MockMergeContext::del_count()
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn new_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// Port of `TestTieredMergePolicy.testForceMergeDeletes`.
pub fn test_force_merge_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let tmp = Arc::new(LuceneTestCase::new_tiered_merge_policy());
    conf.set_merge_policy(Some(Arc::clone(&tmp) as Arc<dyn MergePolicy>))?;
    conf.set_max_buffered_docs(4)?;
    tmp.set_max_merge_at_once(100)?;
    tmp.set_segments_per_tier(100.0)?;
    tmp.set_deletes_pct_allowed(50.0)?;
    tmp.set_force_merge_deletes_pct_allowed(30.0)?;
    let w = IndexWriter::new(dir.as_directory(), conf)?;
    for i in 0..80 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("aaa {}", i % 4),
            Store::NO,
        )));
        w.add_document(&doc)?;
    }
    Assert::assert_equals(80, w.get_doc_stats()?.max_doc);
    Assert::assert_equals(80, w.get_doc_stats()?.num_docs);

    if *VERBOSE {
        println!("\nTEST: delete docs");
    }
    w.delete_documents_by_terms(vec![Term::from_text("content", "0")])?;
    w.force_merge_deletes()?;

    Assert::assert_equals(80, w.get_doc_stats()?.max_doc);
    Assert::assert_equals(60, w.get_doc_stats()?.num_docs);

    if *VERBOSE {
        println!("\nTEST: forceMergeDeletes2");
    }
    as_tiered(&w.get_config().merge_policy()).set_force_merge_deletes_pct_allowed(10.0)?;
    w.force_merge_deletes()?;
    Assert::assert_equals(60, w.get_doc_stats()?.max_doc);
    Assert::assert_equals(60, w.get_doc_stats()?.num_docs);
    w.close()?;
    dir.close()
}

/// `(TieredMergePolicy) policy`.
fn as_tiered(policy: &Arc<dyn MergePolicy>) -> &TieredMergePolicy {
    policy
        .as_any()
        .and_then(|a| a.downcast_ref::<TieredMergePolicy>())
        .unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a TieredMergePolicy").throw()
        })
}

/// Port of `TestTieredMergePolicy.testPartialMerge`.
pub fn test_partial_merge() -> Result<()> {
    let num = LuceneTestCase::at_least(10);
    for iter in 0..num {
        if *VERBOSE {
            println!("TEST: iter={iter}");
        }
        let mut dir = LuceneTestCase::new_directory();
        let conf = new_iwc();
        conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
        let tmp = Arc::new(LuceneTestCase::new_tiered_merge_policy());
        conf.set_merge_policy(Some(Arc::clone(&tmp) as Arc<dyn MergePolicy>))?;
        conf.set_max_buffered_docs(2)?;
        tmp.set_max_merge_at_once(3)?;
        tmp.set_segments_per_tier(6.0)?;

        let w = IndexWriter::new(dir.as_directory(), conf)?;
        let num_docs = TestUtil::next_int(&*random(), 20, 100);
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "content",
                &format!("aaa {}", i % 4),
                Store::NO,
            )));
            w.add_document(&doc)?;
        }

        w.flush_with(true, true)?;

        let segment_count = w.get_segment_count()? as i32;
        let target_count = TestUtil::next_int(&*random(), 1, segment_count);

        if *VERBOSE {
            println!("TEST: merge to {target_count} segs (current count={segment_count})");
        }
        w.force_merge(target_count)?;

        let max_segment_size = tmp
            .get_max_merged_segment_mb()
            .max(tmp.get_floor_segment_mb());
        let max125_pct = ((max_segment_size * 1024.0 * 1024.0) * 1.25) as i64;
        // Other than in the case where the target count is 1 we can't say much except no segment
        // should be > 125% of max seg size.
        if target_count == 1 {
            Assert::assert_equals_msg(
                Some("Should have merged down to one segment"),
                target_count,
                w.get_segment_count()? as i32,
            );
        } else {
            // why can't we say much? Well...
            // 1> the random numbers generated above mean we could have 10 segments and a target max
            // count of, say, 9. we
            //    could get there by combining only 2 segments. So tests like "no pair of segments
            // should total less than
            //    125% max segment size" aren't valid.
            //
            // 2> We could have 10 segments and a target count of 2. In that case there could be 5
            // segments resulting.
            //    as long as they're all < 125% max seg size, that's valid.
            for info in w.clone_segment_infos()?.iter() {
                Assert::assert_true_msg(
                    "No segment should be more than 125% of max segment size ",
                    max125_pct >= size_in_bytes(info)?,
                );
            }
        }

        w.close()?;
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestTieredMergePolicy.testForceMergeDeletesMaxSegSize`.
pub fn test_force_merge_deletes_max_seg_size() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let tmp = TieredMergePolicy::new();
    tmp.set_max_merged_segment_mb(0.01)?;
    tmp.set_force_merge_deletes_pct_allowed(0.0)?;
    conf.set_merge_policy(Some(Arc::new(tmp)))?;

    let w = IndexWriter::new(dir.as_directory(), conf)?;

    let num_docs = LuceneTestCase::at_least(200);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("aaa {i}"),
            Store::NO,
        )));
        w.add_document(&doc)?;
    }

    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    Assert::assert_equals(num_docs, r.max_doc());
    Assert::assert_equals(num_docs, r.num_docs());
    r.close()?;

    if *VERBOSE {
        println!("\nTEST: delete doc");
    }

    w.delete_documents_by_terms(vec![Term::from_text("id", &(42 + 17).to_string())])?;

    let r = directory_reader::open_with_writer(&*w)?;
    Assert::assert_equals(num_docs, r.max_doc());
    Assert::assert_equals(num_docs - 1, r.num_docs());
    r.close()?;

    w.force_merge_deletes()?;

    let r = directory_reader::open_with_writer(&*w)?;
    Assert::assert_equals(num_docs - 1, r.max_doc());
    Assert::assert_equals(num_docs - 1, r.num_docs());
    r.close()?;

    w.close()?;

    dir.close()
}

/// LUCENE-7976 makes findForceMergeDeletes and findForcedDeletes respect max segment size by
/// default, so ensure that this works.
///
/// Port of `TestTieredMergePolicy.testForcedMergesRespectSegSize`.
pub fn test_forced_merges_respect_seg_size() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let tmp = TieredMergePolicy::new();

    // Empirically, 100 docs the size below give us segments of 3,330 bytes. It's not all that
    // reliable in terms
    // of how big a segment _can_ get, so set it to prevent merges on commit.
    let mb_size = 0.004;
    // fudge it up, we're trying to catch egregious errors and segbytes
    // don't really reflect the number for original merges.
    let max_seg_bytes = (1024.0f64 * 1024.0) as i64;
    tmp.set_max_merged_segment_mb(mb_size)?;
    conf.set_max_buffered_docs(100)?;
    conf.set_merge_policy(Some(Arc::new(tmp)))?;
    conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;

    let w = IndexWriter::new(dir.as_directory(), conf)?;

    let num_docs = LuceneTestCase::at_least(2400);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("aaa {i}"),
            Store::NO,
        )));
        w.add_document(&doc)?;
    }

    w.commit()?;

    // These should be no-ops on an index with no deletions and segments are pretty big.
    let mut seg_names_before = get_segment_names(&w)?;
    w.force_merge_deletes()?;
    check_segments_in_expectations(&w, &seg_names_before, false)?; // There should have been no merges.

    w.force_merge(i32::MAX)?;
    check_segments_in_expectations(&w, &seg_names_before, true)?;
    check_segment_size_not_exceeded(&w.clone_segment_infos()?, max_seg_bytes)?;

    // Delete 12-17% of each segment and expungeDeletes. This should result in:
    // > the same number of segments as before.
    // > no segments larger than maxSegmentSize.
    // > no deleted docs left.
    let mut remaining_docs =
        num_docs - delete_pct_docs_from_each_seg(&w, random().next_int_bound(5) + 12, true)?;
    w.force_merge_deletes()?;
    w.commit()?;
    check_segment_size_not_exceeded(&w.clone_segment_infos()?, max_seg_bytes)?;
    Assert::assert_false_msg(
        "There should be no deleted docs in the index.",
        w.has_deletions()?,
    );

    // Check that deleting _fewer_ than 10% doesn't merge inappropriately. Nothing should be merged
    // since no segment
    // has had more than 10% of its docs deleted.
    seg_names_before = get_segment_names(&w)?;
    let mut deleted_this_pass =
        delete_pct_docs_from_each_seg(&w, random().next_int_bound(4) + 3, false)?;
    w.force_merge_deletes()?;
    remaining_docs -= deleted_this_pass;
    check_segments_in_expectations(&w, &seg_names_before, false)?; // There should have been no merges
    Assert::assert_equals_msg(
        Some("NumDocs should reflect removed documents "),
        remaining_docs,
        w.get_doc_stats()?.num_docs,
    );
    Assert::assert_true_msg(
        "Should still be deleted docs in the index",
        w.get_doc_stats()?.num_docs < w.get_doc_stats()?.max_doc,
    );

    // This time, forceMerge. By default, this should respect max segment size.
    // Will change for LUCENE-8236
    w.force_merge(i32::MAX)?;
    check_segment_size_not_exceeded(&w.clone_segment_infos()?, max_seg_bytes)?;

    // Now forceMerge down to one segment, there should be exactly remainingDocs in exactly one
    // segment.
    w.force_merge(1)?;
    Assert::assert_equals_msg(
        Some("There should be exactly one segment now"),
        1,
        w.get_segment_count()?,
    );
    Assert::assert_equals_msg(
        Some("maxDoc and numDocs should be identical"),
        w.get_doc_stats()?.num_docs,
        w.get_doc_stats()?.max_doc,
    );
    Assert::assert_equals_msg(
        Some("There should be an exact number of documents in that one segment"),
        remaining_docs,
        w.get_doc_stats()?.num_docs,
    );

    // Delete 5% and expunge, should be no change.
    seg_names_before = get_segment_names(&w)?;
    remaining_docs -= delete_pct_docs_from_each_seg(&w, random().next_int_bound(5) + 1, false)?;
    w.force_merge_deletes()?;
    check_segments_in_expectations(&w, &seg_names_before, false)?;
    Assert::assert_equals_msg(
        Some("There should still be only one segment. "),
        1,
        w.get_segment_count()?,
    );
    Assert::assert_true_msg(
        "The segment should have deleted documents",
        w.get_doc_stats()?.num_docs < w.get_doc_stats()?.max_doc,
    );

    w.force_merge(1)?; // back to one segment so deletePctDocsFromEachSeg still works

    // Test singleton merge for expungeDeletes
    remaining_docs -= delete_pct_docs_from_each_seg(&w, random().next_int_bound(5) + 20, true)?;
    w.force_merge_deletes()?;

    Assert::assert_equals_msg(
        Some("There should still be only one segment. "),
        1,
        w.get_segment_count()?,
    );
    Assert::assert_equals_msg(
        Some("The segment should have no deleted documents"),
        w.get_doc_stats()?.num_docs,
        w.get_doc_stats()?.max_doc,
    );

    // sanity check, at this point we should have an over`-large segment, we know we have exactly
    // one.
    Assert::assert_true_msg(
        "Our single segment should have quite a few docs",
        w.get_doc_stats()?.num_docs > 1_000,
    );

    // Delete 60% of the documents and then add a few more docs and commit. This should "singleton
    // merge" the large segment
    // created above. 60% leaves some wriggle room, LUCENE-8263 will change this assumption and
    // should be tested
    // when we deal with that JIRA.

    deleted_this_pass =
        delete_pct_docs_from_each_seg(&w, (w.get_doc_stats()?.num_docs * 60) / 100, true)?;
    remaining_docs -= deleted_this_pass;

    for i in 0..50 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &(i + num_docs).to_string(),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("aaa {i}"),
            Store::NO,
        )));
        w.add_document(&doc)?;
    }

    w.commit()?; // want to trigger merge no matter what.

    Assert::assert_equals_msg(
        Some("There should be exactly one very large and one small segment"),
        2,
        w.clone_segment_infos()?.size(),
    );
    let infos = w.clone_segment_infos()?;
    let info0 = infos.info(0);
    let info1 = infos.info(1);
    let large_seg_doc_count = info0.info.max_doc()?.max(info1.info.max_doc()?);
    let small_seg_doc_count = info0.info.max_doc()?.min(info1.info.max_doc()?);
    Assert::assert_equals_msg(
        Some("The large segment should have a bunch of docs"),
        large_seg_doc_count,
        remaining_docs,
    );
    Assert::assert_equals_msg(
        Some("Small segment should have fewer docs"),
        small_seg_doc_count,
        50,
    );

    w.close()?;

    dir.close()
}

/// Returns how many segments are in the index after applying all merges from the `spec` to
/// an index with `starting_segment_count` segments
fn post_merges_segment_count(starting_segment_count: usize, spec: &MergeSpecification) -> usize {
    let mut count = starting_segment_count;
    // remove the segments that are merged away
    for merge in &spec.merges {
        count -= merge.segments.len();
    }

    // add one for each newly merged segment
    count += spec.merges.len();

    count
}

/// verify that all merges in the spec do not create a final merged segment size too much bigger
/// than the configured maxMergedSegmentSizeMb
fn assert_max_merged_size(
    specification: &MergeSpecification,
    max_merged_segment_size_mb: f64,
    index_total_size_in_mb: f64,
    max_merged_segment_count: i32,
) -> Result<()> {
    // When the two configs conflict, TMP favors the requested number of segments. I.e., it will
    // produce
    // too-large (> maxMergedSegmentMB) merged segments.
    let max_mb_per_segment = index_total_size_in_mb / f64::from(max_merged_segment_count);

    for merge in &specification.merges {
        // compute total size of all segments being merged
        let mut merge_total_size_in_bytes = 0i64;
        for segment in &merge.segments {
            merge_total_size_in_bytes += size_in_bytes(segment)?;
        }

        // we allow up to 50% "violation" of the configured maxMergedSegmentSizeMb (why? TMP itself is
        // on only adding 25% fudge factor):
        // TODO: drop this fudge factor back down to 25% -- TooMuchFudgeException!
        let limit_bytes =
            (1024.0 * 1024.0 * max_mb_per_segment.max(max_merged_segment_size_mb) * 1.5) as i64;
        Assert::assert_true_msg(
            &format!(
                "mergeTotalSizeInBytes={merge_total_size_in_bytes} limitBytes={limit_bytes} \
                 maxMergedSegmentSizeMb={}",
                rucene::util::java_fmt::double_to_string(max_merged_segment_size_mb)
            ),
            merge_total_size_in_bytes < limit_bytes,
        );
    }
    Ok(())
}

/// LUCENE-8688 reports that force-merges merged more segments that necessary to respect
/// maxSegmentCount as a result
/// of LUCENE-7976, so we ensure that it only does the minimum number of merges here.
///
/// Port of `TestTieredMergePolicy.testForcedMergesUseLeastNumberOfMerges`.
pub fn test_forced_merges_use_least_number_of_merges() -> Result<()> {
    let tmp = TieredMergePolicy::new();
    let mut one_segment_size_mb = 1.0f64;
    let max_merged_segment_size_mb = 10.0 * one_segment_size_mb;
    tmp.set_max_merged_segment_mb(max_merged_segment_size_mb)?;
    if *VERBOSE {
        println!("TEST: maxMergedSegmentSizeMB={max_merged_segment_size_mb:.2}");
    }

    // create simulated 30 segment index where each segment is 1 MB
    let mut infos = new_infos()?;
    let segment_count = 30;
    for j in 0..segment_count {
        infos.add(make_segment_commit_info(
            &format!("_{j}"),
            1000,
            0,
            one_segment_size_mb,
            SOURCE_MERGE,
        )?)?;
    }

    let mut index_total_size_mb = f64::from(segment_count) * one_segment_size_mb;

    let max_segment_count_after_force_merge = random().next_int_bound(10) + 3;
    if *VERBOSE {
        println!("TEST: maxSegmentCountAfterForceMerge={max_segment_count_after_force_merge}");
    }
    let specification = assert_not_null_spec(tmp.find_forced_merges(
        &infos,
        max_segment_count_after_force_merge,
        &segments_to_merge(&infos),
        &del_count_context(),
    )?);
    if *VERBOSE {
        println!("TEST: specification={specification}");
    }
    assert_max_merged_size(
        &specification,
        max_merged_segment_size_mb,
        index_total_size_mb,
        max_segment_count_after_force_merge,
    )?;

    // verify we achieved exactly the max segment count post-force-merge (which is a bit odd -- the
    // API only ensures <= segments, not ==)
    // TODO: change this to a <= equality like the last assert below?
    Assert::assert_equals(
        max_segment_count_after_force_merge as usize,
        post_merges_segment_count(infos.size(), &specification),
    );

    // now create many segments index, containing 0.1 MB sized segments
    infos = new_infos()?;
    let many_segments_count = LuceneTestCase::at_least(100);
    if *VERBOSE {
        println!("TEST: manySegmentsCount={many_segments_count}");
    }
    one_segment_size_mb = 0.1;
    for j in 0..many_segments_count {
        infos.add(make_segment_commit_info(
            &format!("_{j}"),
            1000,
            0,
            one_segment_size_mb,
            SOURCE_MERGE,
        )?)?;
    }

    index_total_size_mb = f64::from(many_segments_count) * one_segment_size_mb;

    let specification = assert_not_null_spec(tmp.find_forced_merges(
        &infos,
        max_segment_count_after_force_merge,
        &segments_to_merge(&infos),
        &del_count_context(),
    )?);
    if *VERBOSE {
        println!("TEST: specification={specification}");
    }
    assert_max_merged_size(
        &specification,
        max_merged_segment_size_mb,
        index_total_size_mb,
        max_segment_count_after_force_merge,
    )?;
    Assert::assert_true(
        post_merges_segment_count(infos.size(), &specification)
            >= max_segment_count_after_force_merge as usize,
    );
    Ok(())
}

/// A `MergeSpecification` the Java test dereferences: `null` is a
/// `NullPointerException`.
fn assert_not_null_spec(spec: Option<MergeSpecification>) -> MergeSpecification {
    spec.unwrap_or_else(|| npe("specification"))
}

/// Make sure that TieredMergePolicy doesn't do the final merge while there are merges ongoing, but
/// does do non-final
/// merges while merges are ongoing.
///
/// Port of `TestTieredMergePolicy.testForcedMergeWithPending`.
pub fn test_forced_merge_with_pending() -> Result<()> {
    let tmp = TieredMergePolicy::new();
    let max_segment_size = 10.0f64;
    tmp.set_max_merged_segment_mb(max_segment_size)?;

    let mut infos = new_infos()?;
    for j in 0..30 {
        infos.add(make_segment_commit_info(
            &format!("_{j}"),
            1000,
            0,
            1.0,
            SOURCE_MERGE,
        )?)?;
    }
    let merge_context = del_count_context();
    merge_context.set_merging_segments(HashSet::from([infos.as_list()[0].info.name.clone()]));
    let expected_count = random().next_int_bound(10) + 3;
    let specification = tmp.find_forced_merges(
        &infos,
        expected_count,
        &segments_to_merge(&infos),
        &merge_context,
    )?;
    // Since we have fewer than 30 (the max merge count) segments more than the final size this
    // would have been the final merge, so we check that it was prevented.
    assert_null(&specification);
    Ok(())
}

/// `private static Map<SegmentCommitInfo, Boolean> segmentsToMerge(SegmentInfos infos)`.
fn segments_to_merge(infos: &SegmentInfos) -> HashMap<String, bool> {
    let mut segments_to_merge = HashMap::new();
    for info in infos.iter() {
        segments_to_merge.insert(info.info.name.clone(), true);
    }
    segments_to_merge
}

/// `List.toString()` of segment names.
fn names_to_string(names: &[String]) -> String {
    format!("[{}]", names.join(", "))
}

// Having a segment with very few documents in it can happen because of the random nature of the
// docs added to the index. For instance, let's say it just happens that the last segment has 3
// docs in it.
// It can easily be merged with a close-to-max sized segment during a forceMerge and still respect
// the max segment
// size.
//
// If the above is possible, the "twoMayHaveBeenMerged" will be true and we allow for a little
// slop, checking that
// exactly two segments are gone from the old list and exactly one is in the new list. Otherwise,
// the lists must match
// exactly.
//
// So forceMerge may not be a no-op, allow for that. There are two possibilities in forceMerge
// only:
// > there were no small segments, in which case the two lists will be identical
// > two segments in the original list are replaced by one segment in the final list.
//
// finally, there are some cases of forceMerge where the expectation is that there be exactly no
// differences.
// this should be called after forceDeletesMerges with the boolean always false,
// Depending on the state, forceMerge may call with the boolean true or false.

/// `void checkSegmentsInExpectations(IndexWriter w, List<String> segNamesBefore, boolean
/// twoMayHaveBeenMerged)`.
fn check_segments_in_expectations(
    w: &IndexWriter,
    seg_names_before: &[String],
    two_may_have_been_merged: bool,
) -> Result<()> {
    let seg_names_after = get_segment_names(w)?;

    if !two_may_have_been_merged || seg_names_after.len() == seg_names_before.len() {
        if seg_names_after.len() != seg_names_before.len() {
            Assert::fail(&format!(
                "Segment lists different sizes!: {} After list: {}",
                names_to_string(seg_names_before),
                names_to_string(&seg_names_after)
            ));
        }

        if !seg_names_before.iter().all(|n| seg_names_after.contains(n)) {
            Assert::fail(&format!(
                "Segment lists should be identical: {} After list: {}",
                names_to_string(seg_names_before),
                names_to_string(&seg_names_after)
            ));
        }
        return Ok(());
    }

    // forceMerge merged a tiny segment into a not-quite-max-sized segment so check that:
    // Two segments in the before list have been merged into one segment in the after list.
    if seg_names_after.len() != seg_names_before.len() - 1 {
        Assert::fail(&format!(
            "forceMerge didn't merge a small and large segment into one segment as expected: {} \
             After list: {}",
            names_to_string(seg_names_before),
            names_to_string(&seg_names_after)
        ));
    }

    // There should be exactly two segments in the before not in after and one in after not in
    // before.
    let test_before: Vec<&String> = seg_names_before
        .iter()
        .filter(|n| !seg_names_after.contains(n))
        .collect();
    let test_after: Vec<&String> = seg_names_after
        .iter()
        .filter(|n| !seg_names_before.contains(n))
        .collect();

    if test_before.len() != 2 || test_after.len() != 1 {
        Assert::fail(&format!(
            "Expected two unique 'before' segments and one unique 'after' segment: {} After list: \
             {}",
            names_to_string(seg_names_before),
            names_to_string(&seg_names_after)
        ));
    }
    Ok(())
}

/// `List<String> getSegmentNames(IndexWriter w)`.
fn get_segment_names(w: &IndexWriter) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for info in w.clone_segment_infos()?.iter() {
        names.push(info.info.name.clone());
    }
    Ok(names)
}

/// Deletes some docs from each segment
fn delete_pct_docs_from_each_seg(w: &IndexWriter, pct: i32, round_up: bool) -> Result<i32> {
    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(w)?;
    let mut to_delete: Vec<Term> = Vec::new();
    for ctx in Arc::clone(&reader).leaves()? {
        to_delete.extend(get_rand_terms(&ctx, pct, round_up)?);
    }
    reader.close()?;

    let size = to_delete.len() as i32;
    w.delete_documents_by_terms(to_delete)?;
    w.commit()?;
    Ok(size)
}

/// Get me some Ids to delete.
/// So far this supposes that there are no deleted docs in the segment.
/// When the numbers of docs in segments is small, rounding matters. So tests that want over a
/// percentage
/// pass "true" for roundUp, tests that want to be sure they're under some limit pass false.
fn get_rand_terms(ctx: &LeafReaderContext, pct: i32, round_up: bool) -> Result<Vec<Term>> {
    let leaf = ctx.leaf_reader();
    Assert::assert_false_msg(
        "This method assumes no deleted documents",
        Arc::clone(&leaf).as_index_reader().has_deletions(),
    );
    // The indeterminate last segment is a pain, if we're there the number of docs is much less than
    // we expect
    let mut ret: Vec<Term> = Vec::with_capacity(100);

    let num_docs = f64::from(leaf.num_docs());
    let tmp = (num_docs * f64::from(pct)) / 100.0;

    if tmp <= 1.0 {
        // Calculations break down for segments with very few documents, the "tail end
        // Charlie"
        return Ok(ret);
    }
    let r#mod = (num_docs / tmp) as i32;

    if r#mod == 0 {
        return Ok(ret);
    }

    let terms = leaf.terms("id")?.unwrap_or_else(|| npe("terms(\"id\")"));
    let mut iter = terms.iterator()?;
    let mut counter = 0;

    // Small numbers are tricky, they're subject to off-by-one errors. bail if we're going to exceed
    // our target if we add another doc.
    let mut lim = (num_docs * f64::from(pct) / 100.0) as usize;
    if round_up {
        lim += 1;
    }

    let mut br: Option<BytesRef> = iter.next()?;
    while let Some(b) = br {
        if ret.len() >= lim {
            break;
        }
        if (counter % r#mod) == 0 {
            ret.push(Term::new("id", b));
        }
        counter += 1;
        br = iter.next()?;
    }
    Ok(ret)
}

/// `private void checkSegmentSizeNotExceeded(SegmentInfos infos, long maxSegBytes)`.
fn check_segment_size_not_exceeded(infos: &SegmentInfos, max_seg_bytes: i64) -> Result<()> {
    for info in infos.iter() {
        // assertTrue("Found an unexpectedly large segment: " + info.toString(), info.info.maxDoc() -
        // info.getDelCount() <= docLim);
        Assert::assert_true_msg(
            &format!("Found an unexpectedly large segment: {info}"),
            size_in_bytes(info)? <= max_seg_bytes,
        );
    }
    Ok(())
}

/// `private static final double EPSILON = 1E-14`.
const EPSILON: f64 = 1E-14;

/// `assertEquals(expected, actual, delta)` for doubles.
fn assert_double(expected: f64, actual: f64, delta: f64) {
    Assert::assert_equals_double_msg(None, expected, actual, delta);
}

/// `expectThrows(IllegalArgumentException.class, runnable)`.
fn expect_iae<T>(runnable: impl FnOnce() -> Result<T>) {
    rucene_test_framework::util::java_exceptions::expect_throws(
        "java.lang.IllegalArgumentException",
        runnable,
    );
}

/// Port of `TestTieredMergePolicy.testSetters`.
pub fn test_setters() -> Result<()> {
    let tmp = TieredMergePolicy::new();
    let max_mb = i64::MAX as f64 / 1024. / 1024.;

    tmp.set_max_merged_segment_mb(0.5)?;
    assert_double(0.5, tmp.get_max_merged_segment_mb(), EPSILON);

    tmp.set_max_merged_segment_mb(f64::INFINITY)?;
    assert_double(
        max_mb,
        tmp.get_max_merged_segment_mb(),
        EPSILON * i64::MAX as f64,
    );

    tmp.set_max_merged_segment_mb(max_mb)?;
    assert_double(
        max_mb,
        tmp.get_max_merged_segment_mb(),
        EPSILON * i64::MAX as f64,
    );

    expect_iae(|| tmp.set_max_merged_segment_mb(-2.0).map(|_| ()));

    tmp.set_floor_segment_mb(2.0)?;
    assert_double(2.0, tmp.get_floor_segment_mb(), EPSILON);

    tmp.set_floor_segment_mb(f64::INFINITY)?;
    assert_double(
        max_mb,
        tmp.get_floor_segment_mb(),
        EPSILON * i64::MAX as f64,
    );

    tmp.set_floor_segment_mb(max_mb)?;
    assert_double(
        max_mb,
        tmp.get_floor_segment_mb(),
        EPSILON * i64::MAX as f64,
    );

    expect_iae(|| tmp.set_floor_segment_mb(-2.0).map(|_| ()));

    tmp.set_max_cfs_segment_size_mb(2.0)?;
    assert_double(2.0, tmp.get_max_cfs_segment_size_mb(), EPSILON);

    tmp.set_max_cfs_segment_size_mb(f64::INFINITY)?;
    assert_double(
        max_mb,
        tmp.get_max_cfs_segment_size_mb(),
        EPSILON * i64::MAX as f64,
    );

    tmp.set_max_cfs_segment_size_mb(max_mb)?;
    assert_double(
        max_mb,
        tmp.get_max_cfs_segment_size_mb(),
        EPSILON * i64::MAX as f64,
    );

    expect_iae(|| tmp.set_max_cfs_segment_size_mb(-2.0));

    // TODO: Add more checks for other non-double setters!
    Ok(())
}

/// LUCENE-5668
///
/// Port of `TestTieredMergePolicy.testUnbalancedMergeSelection`.
pub fn test_unbalanced_merge_selection() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let mp = iwc.merge_policy();
    let tmp = as_tiered(&mp);
    tmp.set_floor_segment_mb(0.00001)?;
    // We need stable sizes for each segment:
    iwc.set_codec(TestUtil::get_default_codec())?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    iwc.set_max_buffered_docs(100)?;
    iwc.set_ram_buffer_size_mb(-1.0)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for _ in 0..15000 * *RANDOM_MULTIPLIER {
        let mut doc = Document::new();
        // Incompressible content so that merging 10 segments of size x creates a segment whose size
        // is about 10x
        let mut id_bytes = vec![0u8; 128];
        random().next_bytes(&mut id_bytes);
        doc.add(Box::new(StoredField::new_bytes(
            "id",
            BytesRef::new(id_bytes),
        )?));
        w.add_document(&doc)?;
    }
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;

    // Make sure TMP always merged equal-number-of-docs segments:
    for ctx in Arc::clone(&r).leaves()? {
        let num_docs = ctx.leaf_reader().num_docs();
        Assert::assert_true_msg(
            &format!("got numDocs={num_docs}"),
            num_docs == 100 || num_docs == 800 || num_docs == 6400,
        );
    }
    r.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestTieredMergePolicy.testManyMaxSizeSegments`.
pub fn test_many_max_size_segments() -> Result<()> {
    let policy = TieredMergePolicy::new();
    policy.set_max_merged_segment_mb(1024.0)?; // 1GB
    let mut infos = new_infos()?;
    let i = 0;
    for _ in 0..30 {
        infos.add(make_segment_commit_info(
            &format!("_{i}"),
            1000,
            0,
            1024.0,
            SOURCE_MERGE,
        )?)?; // max size
    }
    for _ in 0..8 {
        infos.add(make_segment_commit_info(
            &format!("_{i}"),
            1000,
            0,
            102.0,
            SOURCE_FLUSH,
        )?)?; // 102MB flushes
    }

    // Only 8 segments on 1 tier in addition to the max-size segments, nothing to do
    let merge_spec =
        policy.find_merges(MergeTrigger::SegmentFlush, &infos, &del_count_context())?;
    assert_null(&merge_spec);

    for _ in 0..5 {
        infos.add(make_segment_commit_info(
            &format!("_{i}"),
            1000,
            0,
            102.0,
            SOURCE_FLUSH,
        )?)?; // 102MB flushes
    }

    // Now 13 segments on 1 tier in addition to the max-size segments, 10 of them should get merged
    // in one merge
    let merge_spec = assert_not_null(policy.find_merges(
        MergeTrigger::SegmentFlush,
        &infos,
        &del_count_context(),
    )?);
    Assert::assert_equals(1, merge_spec.merges.len());
    let merge = &merge_spec.merges[0];
    Assert::assert_equals(8, merge.segments.len());
    Ok(())
}

/// Make sure that singleton merges are considered when the max number of deletes is crossed.
///
/// Port of `TestTieredMergePolicy.testMergePurelyToReclaimDeletes`.
pub fn test_merge_purely_to_reclaim_deletes() -> Result<()> {
    // `mergePolicy()`: `newTieredMergePolicy()`.
    let merge_policy = LuceneTestCase::new_tiered_merge_policy();
    let mut infos = new_infos()?;
    // single 1GB segment with no deletes
    infos.add(make_segment_commit_info(
        "_0",
        1_000_000,
        0,
        1024.0,
        SOURCE_MERGE,
    )?)?;

    // not eligible for merging
    assert_null(&merge_policy.find_merges(MergeTrigger::Explicit, &infos, &del_count_context())?);

    // introduce 15% deletes, still not eligible
    infos = apply_deletes(&infos, (0.15 * 1_000_000.0) as i32)?;
    assert_null(&merge_policy.find_merges(MergeTrigger::Explicit, &infos, &del_count_context())?);

    // now cross the delete threshold, becomes eligible
    infos = apply_deletes(
        &infos,
        ((merge_policy.get_deletes_pct_allowed() - 15.0 + 1.0) / 100.0 * 1_000_000.0) as i32,
    )?;
    Assert::assert_true(
        merge_policy
            .find_merges(MergeTrigger::Explicit, &infos, &del_count_context())?
            .is_some(),
    );
    Ok(())
}

/// Port of `TestTieredMergePolicy.testMergeSizeIsLessThanFloorSize`.
pub fn test_merge_size_is_less_than_floor_size() -> Result<()> {
    let merge_context = del_count_context();

    let mut infos = new_infos()?;
    // 5*mergeFactor 1MB segments
    for _ in 0..5 * 8 {
        infos.add(make_segment_commit_info(
            "_0",
            1_000_000,
            0,
            1.0,
            SOURCE_FLUSH,
        )?)?;
    }

    let merge_policy = TieredMergePolicy::new();
    merge_policy.set_max_merge_at_once(30)?;
    merge_policy.set_floor_segment_mb(0.1)?;

    // Segments are above the floor segment size, we get 4 merges of mergeFactor=8 segments each
    let merge_spec = assert_not_null(merge_policy.find_merges(
        MergeTrigger::FullFlush,
        &infos,
        &merge_context,
    )?);
    Assert::assert_equals(4, merge_spec.merges.len());
    for one_merge in &merge_spec.merges {
        assert_double(
            merge_policy.get_segments_per_tier(),
            one_merge.segments.len() as f64,
            0.0,
        );
    }

    // Segments are below the floor segment size and it takes 12 segments to go above the floor
    // segment size. We get 3 merges of 12 segments each.
    merge_policy.set_floor_segment_mb(12.0)?;
    let merge_spec = assert_not_null(merge_policy.find_merges(
        MergeTrigger::FullFlush,
        &infos,
        &merge_context,
    )?);
    Assert::assert_equals(3, merge_spec.merges.len());
    for one_merge in &merge_spec.merges {
        Assert::assert_equals(12, one_merge.segments.len());
    }

    // Segments are below the floor segment size and we'd need to merge more than maxMergeAtOnce
    // segments to go above the minimum segment size. We get 1 merge of maxMergeAtOnce=30 segments
    // and 1 merge of 40-30=10 segments.
    merge_policy.set_floor_segment_mb(60.0)?;
    let merge_spec = assert_not_null(merge_policy.find_merges(
        MergeTrigger::FullFlush,
        &infos,
        &merge_context,
    )?);
    Assert::assert_equals(2, merge_spec.merges.len());
    Assert::assert_equals(30, merge_spec.merges[0].segments.len());
    Assert::assert_equals(10, merge_spec.merges[1].segments.len());
    Ok(())
}

/// Port of `TestTieredMergePolicy.testFullFlushMerges`.
pub fn test_full_flush_merges() -> Result<()> {
    let seg_name_generator = AtomicI64::new(0);
    let mut stats = IOStats::default();
    let merge_context = del_count_context();
    let mut segment_infos = new_infos()?;

    let mp = TieredMergePolicy::new();

    for _ in 0..11 {
        segment_infos.add(make_segment_commit_info(
            &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
            1,
            0,
            f64::from_bits(1), // Double.MIN_VALUE
            SOURCE_FLUSH,
        )?)?;
    }
    let spec = assert_not_null(mp.find_full_flush_merges(
        MergeTrigger::FullFlush,
        &segment_infos,
        &merge_context,
    )?);
    for merge in &spec.merges {
        segment_infos = apply_merge(
            &segment_infos,
            merge,
            &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
            &mut stats,
        )?;
    }
    Assert::assert_equals(2, segment_infos.size());
    Ok(())
}
