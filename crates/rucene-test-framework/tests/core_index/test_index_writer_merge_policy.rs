//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterMergePolicy.java`.
//!
//! * Anonymous subclasses: `new IndexWriter(...) { merge / doAfterFlush }`
//!   install `IndexWriterOverrides`; `new SerialMergeScheduler() { merge }`
//!   and `new ConcurrentMergeScheduler() { merge }` are
//!   `PrologueMergeScheduler`s; `new TieredMergePolicy() { findMerges }` and
//!   `new LogDocMergePolicy() { findMerges }` are `SubclassMergePolicy`s;
//!   `MergeOnXMergePolicy` and the anonymous `FilterMergePolicy` of
//!   `testSetDiagnostics` are `FilterMergePolicyOverrides`; the anonymous
//!   `OneMerge` subclasses install the `OneMerge` override hooks.
//! * `new OneMerge(merge) { setMergeInfo }` of `testSetDiagnostics` is
//!   `OneMerge::wrapping(merge)` with the `setMergeInfo` hook installed.
//! * `new FilterDirectory(newFSDirectory(path)) { openInput }` is
//!   `AssertFileExistDirectory`; `mockDirectory.close()` (which closes the
//!   wrapped directory) closes the `newFSDirectory` holder.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, NumericDocValuesField, NumericValue, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::index_writer::{IndexWriter, IndexWriterOverrides, SOURCE, SOURCE_MERGE};
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{
    CfsSettings, LogDocSize, LogMergePolicy, MergeContext, MergePolicy, MergeSpecification,
    MergeTrigger, NoMergePolicy, OneMerge, OneMergeWrappingMergePolicy,
    SoftDeletesRetentionMergePolicy, TieredMergePolicy,
};
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, SerialMergeScheduler};
use rucene::index::terms::Term;
use rucene::index::{IndexReader, SegmentCommitInfo, SegmentInfos};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{MatchAllDocsQuery, Query, TermQuery};
use rucene::store::java_nio_file::{no_such_file_exception, Files, Path};
use rucene::store::{DataInput, Directory, IOContext, IndexInput, IndexOutput, Lock};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::{MockIndexWriterEventListener, RandomIndexWriter};
use rucene_test_framework::mockfile::MaxOpenHandles;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::{
    AwaitsFix, Nightly, SuppressCodecs, TestDirectory, TEST_NIGHTLY,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, class_annotated, random, test,
};
use crate::support::{
    add_doc as test_index_writer_add_doc, CountDownLatch, FilterMergePolicyOverrides, FindMergesFn,
    PrologueMergeScheduler, SubclassMergePolicy,
};

/// `TestIndexWriterMergePolicy`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterMergePolicy",
        vec![
            test("testNormalCase", test_normal_case),
            test("testNoOverMerge", test_no_over_merge),
            test("testForceFlush", test_force_flush),
            test("testMergeFactorChange", test_merge_factor_change),
            test("testMaxBufferedDocsChange", test_max_buffered_docs_change).annotate(Nightly),
            test("testMergeDocCount0", test_merge_doc_count0),
            test("testSetters", test_setters),
            test("testMergeOnCommit", test_merge_on_commit),
            test(
                "testMergeOnCommitWithEventListener",
                test_merge_on_commit_with_event_listener,
            ),
            test(
                "testCarryOverNewDeletesOnCommit",
                test_carry_over_new_deletes_on_commit,
            ),
            test("testAbortMergeOnCommit", test_abort_merge_on_commit),
            test("testAbortMergeOnGetReader", test_abort_merge_on_get_reader),
            test(
                "testForceMergeWhileGetReader",
                test_force_merge_while_get_reader,
            ),
            test(
                "testFailAfterMergeCommitted",
                test_fail_after_merge_committed,
            ),
            // TODO: tests using stressUpdateSameDocumentWithMergeOnX have resource issues
            test(
                "testStressUpdateSameDocumentWithMergeOnGetReader",
                test_stress_update_same_document_with_merge_on_get_reader,
            )
            .annotate(AwaitsFix {
                bug_url: "https://github.com/apache/lucene/issues/14483".to_string(),
            }),
            // TODO: tests using stressUpdateSameDocumentWithMergeOnX have resource issues
            test(
                "testStressUpdateSameDocumentWithMergeOnCommit",
                test_stress_update_same_document_with_merge_on_commit,
            )
            .annotate(AwaitsFix {
                bug_url: "https://github.com/apache/lucene/issues/14483".to_string(),
            }),
            test("testMergeOnGetReader", test_merge_on_get_reader),
            test("testSetDiagnostics", test_set_diagnostics),
            test(
                "testForceMergeDVUpdateFileWithConcurrentFlush",
                test_force_merge_dv_update_file_with_concurrent_flush,
            ),
            test(
                "testMergeDVUpdateFileOnGetReaderWithConcurrentFlush",
                test_merge_dv_update_file_on_get_reader_with_concurrent_flush,
            ),
            test(
                "testMergeDVUpdateFileOnCommitWithConcurrentFlush",
                test_merge_dv_update_file_on_commit_with_concurrent_flush,
            ),
            test(
                "testForceMergeWithPendingHardAndSoftDeleteFile",
                test_force_merge_with_pending_hard_and_soft_delete_file,
            ),
        ],
        |b| {
            b.annotate(MaxOpenHandles {
                limit: MaxOpenHandles::MAX_OPEN_FILES * 2,
            })
            // Some of these tests are too intense for SimpleText
            .annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// A less sophisticated version of LogDocMergePolicy, only for testing the interaction between
/// IndexWriter and the MergePolicy.
struct MockMergePolicy {
    merge_factor: AtomicI32,
    /// The `MergePolicy` base fields `noCFSRatio` and `maxCFSSegmentSize`.
    cfs: CfsSettings,
}

impl MockMergePolicy {
    fn new() -> Self {
        MockMergePolicy {
            merge_factor: AtomicI32::new(10),
            cfs: CfsSettings::default(),
        }
    }

    fn get_merge_factor(&self) -> i32 {
        self.merge_factor.load(Ordering::SeqCst)
    }

    fn set_merge_factor(&self, merge_factor: i32) {
        self.merge_factor.store(merge_factor, Ordering::SeqCst);
    }
}

impl std::fmt::Display for MockMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MockMergePolicy")
    }
}

impl MergePolicy for MockMergePolicy {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merge_factor = self.get_merge_factor();
        let mut segments: Vec<SegmentCommitInfo> = Vec::new();
        for sci in segment_infos.iter() {
            segments.push(sci.clone());
        }
        let mut spec: Option<MergeSpecification> = None;
        let mut start: i32 = 0;
        while start <= segments.len() as i32 - merge_factor {
            let start_doc_count = segments[start as usize].info.max_doc()?;
            // Now search for the right-most segment that could be merged with the start segment
            let mut end = start + 1;
            let mut i = segments.len() as i32 - 1;
            while i > start {
                let doc_count = segments[i as usize].info.max_doc()?;
                if doc_count as i64 * merge_factor as i64 > start_doc_count as i64
                    && (doc_count as i64) < merge_factor as i64 * start_doc_count as i64
                {
                    end = i + 1;
                    break;
                }
                i -= 1;
            }

            // Now record a merge if possible
            if start + merge_factor <= end {
                spec.get_or_insert_with(MergeSpecification::new)
                    .add(OneMerge::new(
                        segments[start as usize..(start + merge_factor) as usize].to_vec(),
                    )?);
                start += merge_factor;
            } else {
                start += 1;
            }
        }
        Ok(spec)
    }

    fn find_forced_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _max_segment_count: i32,
        _segments_to_merge: &HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.cfs.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMaxBufferedDocs(n).setMergePolicy(mp)`.
fn mock_iwc_buffered(
    max_buffered_docs: i32,
    mp: Arc<dyn MergePolicy>,
) -> Result<IndexWriterConfig> {
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(max_buffered_docs)?;
    iwc.set_merge_policy(Some(mp))?;
    Ok(iwc)
}

// Test the normal case
/// Port of `TestIndexWriterMergePolicy.testNormalCase`.
fn test_normal_case() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let writer = IndexWriter::new(
        dir.as_directory(),
        mock_iwc_buffered(10, Arc::new(MockMergePolicy::new()))?,
    )?;

    for _ in 0..100 {
        add_doc(&writer)?;
        check_invariants(&writer)?;
    }

    writer.close()?;
    dir.close()
}

// Test to see if there is over merge
/// Port of `TestIndexWriterMergePolicy.testNoOverMerge`.
fn test_no_over_merge() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let writer = IndexWriter::new(
        dir.as_directory(),
        mock_iwc_buffered(10, Arc::new(MockMergePolicy::new()))?,
    )?;

    let mut no_over_merge = false;
    for _ in 0..100 {
        add_doc(&writer)?;
        check_invariants(&writer)?;
        if writer.get_num_buffered_documents() + writer.get_segment_count()? as i32 >= 18 {
            no_over_merge = true;
        }
    }
    Assert::assert_true(no_over_merge);

    writer.close()?;
    dir.close()
}

// Test the case where flush is forced after every addDoc
/// Port of `TestIndexWriterMergePolicy.testForceFlush`.
fn test_force_flush() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let mp = MockMergePolicy::new();
    mp.set_merge_factor(10);
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc_buffered(10, Arc::new(mp))?)?;

    for _ in 0..100 {
        add_doc(&writer)?;
        writer.flush()?;
    }

    writer.close()?;
    dir.close()
}

/// `((MockMergePolicy) writer.getConfig().getMergePolicy())`.
fn mock_merge_policy_of(writer: &IndexWriter) -> Arc<dyn MergePolicy> {
    let mp = writer.get_config().merge_policy();
    if mp
        .as_any()
        .and_then(|a| a.downcast_ref::<MockMergePolicy>())
        .is_none()
    {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            "not a MockMergePolicy",
        )
        .throw()
    }
    mp
}

/// `MockMergePolicy` behind `mp` (checked by `mock_merge_policy_of`).
fn as_mock(mp: &Arc<dyn MergePolicy>) -> &MockMergePolicy {
    mp.as_any()
        .and_then(|a| a.downcast_ref::<MockMergePolicy>())
        .expect("checked by mock_merge_policy_of")
}

// Test the case where mergeFactor changes
/// Port of `TestIndexWriterMergePolicy.testMergeFactorChange`.
fn test_merge_factor_change() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let iwc = mock_iwc_buffered(10, Arc::new(MockMergePolicy::new()))?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    for _ in 0..250 {
        add_doc(&writer)?;
        check_invariants(&writer)?;
    }

    as_mock(&mock_merge_policy_of(&writer)).set_merge_factor(5);

    // merge policy only fixes segments on levels where merges
    // have been triggered, so check invariants after all adds
    for _ in 0..10 {
        add_doc(&writer)?;
    }
    check_invariants(&writer)?;

    writer.close()?;
    dir.close()
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setOpenMode(APPEND)
/// .setMaxBufferedDocs(n).setMergePolicy(mp).setMergeScheduler(new SerialMergeScheduler())`.
fn append_serial_iwc(max_buffered_docs: i32, mp: MockMergePolicy) -> Result<IndexWriterConfig> {
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    iwc.set_max_buffered_docs(max_buffered_docs)?;
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    Ok(iwc)
}

// Test the case where both mergeFactor and maxBufferedDocs change
/// Port of `TestIndexWriterMergePolicy.testMaxBufferedDocsChange`.
fn test_max_buffered_docs_change() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let iwc = mock_iwc_buffered(101, Arc::new(MockMergePolicy::new()))?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let mut writer = IndexWriter::new(dir.as_directory(), iwc)?;

    // leftmost* segment has 1 doc
    // rightmost* segment has 100 docs
    for i in 1..=100 {
        for _ in 0..i {
            add_doc(&writer)?;
            check_invariants(&writer)?;
        }
        writer.close()?;

        writer = IndexWriter::new(
            dir.as_directory(),
            append_serial_iwc(101, MockMergePolicy::new())?,
        )?;
    }

    writer.close()?;
    let ldmp = MockMergePolicy::new();
    ldmp.set_merge_factor(10);
    let writer = IndexWriter::new(dir.as_directory(), append_serial_iwc(10, ldmp)?)?;

    // merge policy only fixes segments on levels where merges
    // have been triggered, so check invariants after all adds
    for _ in 0..100 {
        add_doc(&writer)?;
    }
    check_invariants(&writer)?;

    for _ in 100..1000 {
        add_doc(&writer)?;
    }
    writer.commit()?;
    writer.wait_for_merges()?;
    writer.commit()?;
    check_invariants(&writer)?;

    writer.close()?;
    dir.close()
}

// Test the case where a merge results in no doc at all
/// Port of `TestIndexWriterMergePolicy.testMergeDocCount0`.
fn test_merge_doc_count0() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let ldmp = MockMergePolicy::new();
    ldmp.set_merge_factor(100);
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc_buffered(10, Arc::new(ldmp))?)?;

    for _ in 0..250 {
        add_doc(&writer)?;
        check_invariants(&writer)?;
    }
    writer.close()?;

    // delete some docs without merging
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.delete_documents_by_terms(vec![Term::from_text("content", "aaa")])?;
    writer.close()?;

    let ldmp = MockMergePolicy::new();
    ldmp.set_merge_factor(5);
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    iwc.set_max_buffered_docs(10)?;
    iwc.set_merge_policy(Some(Arc::new(ldmp)))?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    // merge factor is changed, so check invariants after all adds
    for _ in 0..10 {
        add_doc(&writer)?;
    }
    writer.commit()?;
    writer.wait_for_merges()?;
    writer.commit()?;
    check_invariants(&writer)?;
    assert_equals(&10, &writer.get_doc_stats()?.max_doc);

    writer.close()?;
    dir.close()
}

/// `private void addDoc(IndexWriter writer)`.
fn add_doc(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `private void checkInvariants(IndexWriter writer)`.
fn check_invariants(writer: &IndexWriter) -> Result<()> {
    writer.wait_for_merges()?;
    let max_buffered_docs = writer.get_config().max_buffered_docs();
    let merge_factor = as_mock(&mock_merge_policy_of(writer)).get_merge_factor();

    let ram_segment_count = writer.get_num_buffered_documents();
    Assert::assert_true(ram_segment_count < max_buffered_docs);

    let segment_count = writer.get_segment_count()?;
    let mut lower_bound = i32::MAX;
    for i in 0..segment_count {
        lower_bound = lower_bound.min(writer.max_doc_at(i)?);
    }
    let upper_bound = lower_bound.wrapping_mul(merge_factor);

    let mut segments_across_levels = 0;
    while segments_across_levels < segment_count {
        let mut segments_on_current_level = 0;
        for i in 0..segment_count {
            let doc_count = writer.max_doc_at(i)?;
            if doc_count >= lower_bound && doc_count < upper_bound {
                segments_on_current_level += 1;
            }
        }

        Assert::assert_true(segments_on_current_level < merge_factor);
        segments_across_levels += segments_on_current_level as usize;
    }
    Ok(())
}

const EPSILON: f64 = 1E-14;

/// Port of `TestIndexWriterMergePolicy.testSetters`.
fn test_setters() -> Result<()> {
    assert_setters(&LogMergePolicy::new(Arc::new(
        rucene::index::merge_policy::LogByteSize,
    )))?;
    assert_setters(&MockMergePolicy::new())?;
    Ok(())
}

/// `TestIndexWriter.addDoc(firstWriter); firstWriter.flush()` five times over a
/// `NoMergePolicy` writer, then the five-leaf check.
fn five_flushed_segments(dir: &TestDirectory) -> Result<()> {
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let first_writer = IndexWriter::new(dir.as_directory(), iwc)?;
    for _ in 0..5 {
        test_index_writer_add_doc(&first_writer)?;
        first_writer.flush()?;
    }
    let first_reader = directory_reader::open_with_writer(&*first_writer)?;
    assert_equals(&5, &Arc::clone(&first_reader).leaves()?.len());
    first_reader.close()?;
    first_writer.close() // When this writer closes, it does not merge on commit.
}

// Test basic semantics of merge on commit
/// Port of `TestIndexWriterMergePolicy.testMergeOnCommit`.
fn test_merge_on_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    five_flushed_segments(&dir)?;

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(merge_on_x_merge_policy(
        LuceneTestCase::new_merge_policy(),
        MergeTrigger::Commit,
    )))?;
    iwc.set_max_full_flush_merge_wait_millis(i32::MAX as i64);

    let writer_with_merge_policy = IndexWriter::new(dir.as_directory(), iwc)?;

    // No changes. Refresh doesn't trigger a merge.
    let unmerged_reader = directory_reader::open_with_writer(&*writer_with_merge_policy)?;
    assert_equals(&5, &Arc::clone(&unmerged_reader).leaves()?.len());
    unmerged_reader.close()?;

    writer_with_merge_policy.commit()?; // Do merge on commit.
    assert_equals(&1, &writer_with_merge_policy.get_segment_count()?); //

    let merged_reader = directory_reader::open_with_writer(&*writer_with_merge_policy)?;
    assert_equals(&1, &Arc::clone(&merged_reader).leaves()?.len());
    merged_reader.close()?;

    {
        let reader = directory_reader::open_with_writer(&*writer_with_merge_policy)?;
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        assert_equals(&5, &reader.num_docs());
        assert_equals(
            &5,
            &searcher.count(Arc::new(MatchAllDocsQuery::instance()))?,
        );
        reader.close()?;
    }

    writer_with_merge_policy.close()?;
    dir.close()
}

// Test basic semantics of merge on commit and events recording invocation
/// Port of `TestIndexWriterMergePolicy.testMergeOnCommitWithEventListener`.
fn test_merge_on_commit_with_event_listener() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    five_flushed_segments(&dir)?;

    let event_listener = Arc::new(MockIndexWriterEventListener::new());

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(merge_on_x_merge_policy(
        LuceneTestCase::new_merge_policy(),
        MergeTrigger::Commit,
    )))?;
    iwc.set_max_full_flush_merge_wait_millis(i32::MAX as i64);
    iwc.set_index_writer_event_listener(Arc::clone(&event_listener) as _);

    let writer_with_merge_policy = IndexWriter::new(dir.as_directory(), iwc)?;

    // No changes. Refresh doesn't trigger a merge.
    let unmerged_reader = directory_reader::open_with_writer(&*writer_with_merge_policy)?;
    assert_equals(&5, &Arc::clone(&unmerged_reader).leaves()?.len());
    unmerged_reader.close()?;

    Assert::assert_false(event_listener.is_events_recorded());
    writer_with_merge_policy.commit()?; // Do merge on commit.
    assert_equals(&1, &writer_with_merge_policy.get_segment_count()?); //
    Assert::assert_true(event_listener.is_events_recorded());

    writer_with_merge_policy.close()?;
    dir.close()
}

/// `private void assertSetters(MergePolicy lmp)`.
fn assert_setters(lmp: &dyn MergePolicy) -> Result<()> {
    lmp.set_max_cfs_segment_size_mb(2.0)?;
    assert_equals_delta(2.0, lmp.get_max_cfs_segment_size_mb(), EPSILON);

    lmp.set_max_cfs_segment_size_mb(f64::INFINITY)?;
    assert_equals_delta(
        i64::MAX as f64 / 1024. / 1024.,
        lmp.get_max_cfs_segment_size_mb(),
        EPSILON * i64::MAX as f64,
    );

    lmp.set_max_cfs_segment_size_mb(i64::MAX as f64 / 1024. / 1024.)?;
    assert_equals_delta(
        i64::MAX as f64 / 1024. / 1024.,
        lmp.get_max_cfs_segment_size_mb(),
        EPSILON * i64::MAX as f64,
    );

    expect_throws("java.lang.IllegalArgumentException", || {
        lmp.set_max_cfs_segment_size_mb(-2.0)
    });

    // TODO: Add more checks for other non-double setters!
    Ok(())
}

/// `new NumericDocValuesField(name, value)` as the `Field` the soft-update
/// methods take.
fn soft_field(name: &str, value: i64) -> Result<Field> {
    Field::new_with_number(
        name,
        NumericValue::Long(value),
        NumericDocValuesField::type_().clone(),
    )
}

/// A document with one `StringField("id", id, Store.NO)`.
fn id_doc(id: &str) -> Result<Document> {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
    Ok(d)
}

/// The anonymous `IndexWriter` of `testCarryOverNewDeletesOnCommit`.
struct WaitingMergeWriter {
    wait_for_merge: Arc<CountDownLatch>,
    wait_for_update: Arc<CountDownLatch>,
}

impl IndexWriterOverrides for WaitingMergeWriter {
    fn merge(&self, writer: &IndexWriter, merge: OneMerge) -> Result<()> {
        self.wait_for_merge.count_down();
        self.wait_for_update.await_();
        writer.super_merge(merge)
    }
}

/// Port of `TestIndexWriterMergePolicy.testCarryOverNewDeletesOnCommit`.
fn test_carry_over_new_deletes_on_commit() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let use_soft_deletes = random().next_boolean();
        let wait_for_merge = Arc::new(CountDownLatch::new(1));
        let wait_for_update = Arc::new(CountDownLatch::new(1));
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(merge_on_x_merge_policy(
            Arc::new(NoMergePolicy::new()),
            MergeTrigger::Commit,
        )))?;
        iwc.set_max_full_flush_merge_wait_millis(30 * 1000);
        iwc.set_soft_deletes_field(Some("soft_delete".to_string()));
        iwc.set_max_buffered_docs(i32::MAX)?;
        iwc.set_ram_buffer_size_mb(100.0)?;
        iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
        let writer = IndexWriter::new(directory.as_directory(), iwc)?;
        writer.set_overrides(Arc::new(WaitingMergeWriter {
            wait_for_merge: Arc::clone(&wait_for_merge),
            wait_for_update: Arc::clone(&wait_for_update),
        }));

        let d1 = id_doc("1")?;
        let d2 = Arc::new(id_doc("2")?);
        let d3 = id_doc("3")?;
        writer.add_document(&d1)?;
        writer.flush()?;
        writer.add_document(&*d2)?;
        let add_three_docs = random().next_boolean();
        let mut expected_num_docs = 2;
        if add_three_docs {
            // sometimes add another doc to ensure we don't have a fully deleted
            // segment
            expected_num_docs = 3;
            writer.add_document(&d3)?;
        }
        let t = {
            let writer = Arc::clone(&writer);
            let wait_for_merge = Arc::clone(&wait_for_merge);
            let wait_for_update = Arc::clone(&wait_for_update);
            let d2 = Arc::clone(&d2);
            Thread::unnamed(move || {
                let result = (|| -> Result<()> {
                    wait_for_merge.await_();
                    if use_soft_deletes {
                        writer.soft_update_document(
                            Term::from_text("id", "2"),
                            &*d2,
                            &[soft_field("soft_delete", 1)?],
                        )?;
                    } else {
                        writer.update_document(Term::from_text("id", "2"), &*d2)?;
                    }
                    writer.flush()
                })();
                wait_for_update.count_down();
                if let Err(e) = result {
                    rethrow_java(assertion_error_from(e));
                }
            })
        };
        t.start();
        writer.commit()?;
        let _ = t.join();
        {
            let open = SoftDeletesDirectoryReaderWrapper::new("soft_delete")
                .wrap(directory_reader::open(directory.as_directory())?)?;
            assert_equals(&expected_num_docs, &open.num_docs());
            assert_equals_msg(
                Some("we should not have any deletes"),
                &expected_num_docs,
                &open.max_doc(),
            );
            open.close()?;
        }

        {
            let open = directory_reader::open_with_writer(&*writer)?;
            assert_equals(&expected_num_docs, &open.num_docs());
            assert_equals_msg(
                Some("we should not have one delete"),
                &(expected_num_docs + 1),
                &open.max_doc(),
            );
            open.close()?;
        }
        writer.close()?;
    }
    directory.close()
}

/// `new AssertionError(e)`: an `AssertionError` whose cause is `e`.
fn assertion_error_from(e: LuceneError) -> LuceneError {
    let message = rucene_test_framework::util::java_exceptions::java_throwable(&e).to_string();
    LuceneError::Unchecked {
        class: "java.lang.AssertionError",
        message: Some(message),
        cause: Some(Arc::new(e)),
    }
}

/// This test makes sure we release the merge readers on abort. MDW will fail if it can't close all
/// files
/// Port of `TestIndexWriterMergePolicy.testAbortMergeOnCommit`.
fn test_abort_merge_on_commit() -> Result<()> {
    abort_merge_on_x(false)
}

/// Port of `TestIndexWriterMergePolicy.testAbortMergeOnGetReader`.
fn test_abort_merge_on_get_reader() -> Result<()> {
    abort_merge_on_x(true)
}

/// `void abortMergeOnX(boolean useGetReader)`.
fn abort_merge_on_x(use_get_reader: bool) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let wait_for_merge = Arc::new(CountDownLatch::new(1));
        let wait_for_delete_all = Arc::new(CountDownLatch::new(1));
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(merge_on_x_merge_policy(
            LuceneTestCase::new_merge_policy(),
            if use_get_reader {
                MergeTrigger::GetReader
            } else {
                MergeTrigger::Commit
            },
        )))?;
        iwc.set_max_full_flush_merge_wait_millis(30 * 1000);
        {
            let wait_for_merge = Arc::clone(&wait_for_merge);
            let wait_for_delete_all = Arc::clone(&wait_for_delete_all);
            iwc.set_merge_scheduler(Some(Arc::new(PrologueMergeScheduler::new(
                Arc::new(SerialMergeScheduler::new()),
                true,
                Arc::new(move || {
                    wait_for_merge.count_down();
                    wait_for_delete_all.await_();
                    Ok(())
                }),
            ))))?;
        }
        let writer = IndexWriter::new(directory.as_directory(), iwc)?;

        let d1 = id_doc("1")?;
        let d2 = id_doc("2")?;
        let _d3 = id_doc("3")?;
        writer.add_document(&d1)?;
        writer.flush()?;
        writer.add_document(&d2)?;
        let t = {
            let writer = Arc::clone(&writer);
            let wait_for_merge = Arc::clone(&wait_for_merge);
            Thread::unnamed(move || {
                let mut success = false;
                let result = if use_get_reader {
                    directory_reader::open_with_writer(&*writer).and_then(|r| r.close())
                } else {
                    writer.commit().map(|_| ())
                };
                if result.is_ok() {
                    success = true;
                }
                if !success {
                    wait_for_merge.count_down();
                }
                if let Err(e) = result {
                    rethrow_java(assertion_error_from(e));
                }
            })
        };
        t.start();
        wait_for_merge.await_();
        writer.delete_all()?;
        wait_for_delete_all.count_down();
        let _ = t.join();
        writer.close()?;
    }
    directory.close()
}

/// Port of `TestIndexWriterMergePolicy.testForceMergeWhileGetReader`.
fn test_force_merge_while_get_reader() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let wait_for_merge = Arc::new(CountDownLatch::new(1));
        let wait_for_force_merge_called = Arc::new(CountDownLatch::new(1));
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(merge_on_x_merge_policy(
            LuceneTestCase::new_merge_policy(),
            MergeTrigger::GetReader,
        )))?;
        iwc.set_max_full_flush_merge_wait_millis(30 * 1000);
        {
            let wait_for_merge = Arc::clone(&wait_for_merge);
            let wait_for_force_merge_called = Arc::clone(&wait_for_force_merge_called);
            iwc.set_merge_scheduler(Some(Arc::new(PrologueMergeScheduler::new(
                Arc::new(SerialMergeScheduler::new()),
                false,
                Arc::new(move || {
                    wait_for_merge.count_down();
                    wait_for_force_merge_called.await_();
                    Ok(())
                }),
            ))))?;
        }
        let writer = IndexWriter::new(directory.as_directory(), iwc)?;
        writer.add_document(&id_doc("1")?)?;
        writer.flush()?;
        writer.add_document(&id_doc("2")?)?;
        let t = {
            let writer = Arc::clone(&writer);
            Thread::unnamed(move || {
                let result = directory_reader::open_with_writer(&*writer).and_then(|reader| {
                    assert_equals(&2, &reader.max_doc());
                    reader.close()
                });
                if let Err(e) = result {
                    rethrow_java(assertion_error_from(e));
                }
            })
        };
        t.start();
        wait_for_merge.await_();
        writer.add_document(&id_doc("3")?)?;
        wait_for_force_merge_called.count_down();
        writer.force_merge(1)?;
        let _ = t.join();
        writer.close()?;
    }
    directory.close()
}

/// The anonymous `IndexWriter` of `testFailAfterMergeCommitted`.
struct FailAfterFlushWriter {
    merge_and_fail: Arc<AtomicBool>,
}

impl IndexWriterOverrides for FailAfterFlushWriter {
    fn do_after_flush(&self, writer: &IndexWriter) -> Result<()> {
        if self.merge_and_fail.load(Ordering::SeqCst) && writer.has_pending_merges_pub()? {
            writer.execute_merge(MergeTrigger::GetReader)?;
            return Err(LuceneError::unchecked("java.lang.RuntimeException", "boom"));
        }
        Ok(())
    }
}

/// Port of `TestIndexWriterMergePolicy.testFailAfterMergeCommitted`.
fn test_fail_after_merge_committed() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let merge_and_fail = Arc::new(AtomicBool::new(false));
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(merge_on_x_merge_policy(
            Arc::new(NoMergePolicy::new()),
            MergeTrigger::GetReader,
        )))?;
        iwc.set_max_full_flush_merge_wait_millis(30 * 1000);
        iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
        let writer = IndexWriter::new(directory.as_directory(), iwc)?;
        writer.set_overrides(Arc::new(FailAfterFlushWriter {
            merge_and_fail: Arc::clone(&merge_and_fail),
        }));
        writer.add_document(&id_doc("1")?)?;
        writer.flush()?;
        writer.add_document(&id_doc("2")?)?;
        writer.flush()?;
        merge_and_fail.store(true, Ordering::SeqCst);
        {
            struct Reset<'a>(&'a AtomicBool);
            impl Drop for Reset<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _finally = Reset(&merge_and_fail);
            let e = expect_throws("java.lang.RuntimeException", || {
                directory_reader::open_with_writer(&*writer)
            });
            assert_equals(&Some("boom"), &e.message());
        }
        writer.close()?;
    }
    directory.close()
}

// TODO: tests using stressUpdateSameDocumentWithMergeOnX have resource issues
/// Port of `TestIndexWriterMergePolicy.testStressUpdateSameDocumentWithMergeOnGetReader`.
fn test_stress_update_same_document_with_merge_on_get_reader() -> Result<()> {
    stress_update_same_document_with_merge_on_x(true)
}

// TODO: tests using stressUpdateSameDocumentWithMergeOnX have resource issues
/// Port of `TestIndexWriterMergePolicy.testStressUpdateSameDocumentWithMergeOnCommit`.
fn test_stress_update_same_document_with_merge_on_commit() -> Result<()> {
    stress_update_same_document_with_merge_on_x(false)
}

/// `void stressUpdateSameDocumentWithMergeOnX(boolean useGetReader)`.
fn stress_update_same_document_with_merge_on_x(use_get_reader: bool) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(merge_on_x_merge_policy(
            LuceneTestCase::new_merge_policy(),
            if use_get_reader {
                MergeTrigger::GetReader
            } else {
                MergeTrigger::Commit
            },
        )))?;
        iwc.set_max_full_flush_merge_wait_millis(10 + random().next_int_bound(2000) as i64);
        iwc.set_soft_deletes_field(Some("soft_delete".to_string()));
        iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
        let writer = Arc::new(RandomIndexWriter::new_with_config(
            &*random(),
            directory.as_directory(),
            iwc,
        )?);
        let d1 = Arc::new(id_doc("1")?);
        writer.update_document(Term::from_text("id", "1"), &*d1)?;
        writer.commit()?;

        let iters = Arc::new(AtomicI32::new(
            100 + random().next_int_bound(if *TEST_NIGHTLY { 5000 } else { 1000 }),
        ));
        let num_full_flushes = Arc::new(AtomicI32::new(
            10 + random().next_int_bound(if *TEST_NIGHTLY { 500 } else { 100 }),
        ));
        let done = Arc::new(AtomicBool::new(false));
        let mut threads: Vec<Thread> = Vec::new();
        let n = 1 + random().next_int_bound(4);
        for _ in 0..n {
            let writer = Arc::clone(&writer);
            let iters = Arc::clone(&iters);
            let num_full_flushes = Arc::clone(&num_full_flushes);
            let done = Arc::clone(&done);
            let d1 = Arc::clone(&d1);
            let t = Thread::unnamed(move || {
                let result = (|| -> Result<()> {
                    while iters.fetch_sub(1, Ordering::SeqCst) - 1 > 0
                        || num_full_flushes.load(Ordering::SeqCst) > 0
                    {
                        writer.update_document(Term::from_text("id", "1"), &*d1)?;
                        if random().next_boolean() {
                            writer.add_document(&Document::new())?;
                        }
                    }
                    Ok(())
                })();
                done.store(true, Ordering::SeqCst);
                if let Err(e) = result {
                    rethrow_java(assertion_error_from(e));
                }
            });
            t.start();
            threads.push(t);
        }
        let body = (|| -> Result<()> {
            while !done.load(Ordering::SeqCst) {
                if use_get_reader {
                    let reader = writer.get_reader()?;
                    assert_equals(
                        &1,
                        &IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?
                            .search(
                                Arc::new(TermQuery::new(Term::from_text("id", "1")))
                                    as Arc<dyn Query>,
                                10,
                            )?
                            .total_hits
                            .value(),
                    );
                    reader.close()?;
                } else {
                    if random().next_boolean() {
                        writer.commit()?;
                    }
                    let delegate = directory_reader::open(directory.as_directory())?;
                    let open =
                        SoftDeletesDirectoryReaderWrapper::new("___soft_deletes").wrap(delegate)?;
                    assert_equals(
                        &1,
                        &IndexSearcher::new(Arc::clone(&open) as Arc<dyn IndexReader>)?
                            .search(
                                Arc::new(TermQuery::new(Term::from_text("id", "1")))
                                    as Arc<dyn Query>,
                                10,
                            )?
                            .total_hits
                            .value(),
                    );
                    open.close()?;
                }
                num_full_flushes.fetch_sub(1, Ordering::SeqCst);
            }
            Ok(())
        })();
        num_full_flushes.store(0, Ordering::SeqCst);
        for t in &threads {
            let _ = t.join();
        }
        body?;
        writer.close()?;
    }
    directory.close()
}

// Test basic semantics of merge on getReader
/// Port of `TestIndexWriterMergePolicy.testMergeOnGetReader`.
fn test_merge_on_get_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    five_flushed_segments(&dir)?;

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(merge_on_x_merge_policy(
        LuceneTestCase::new_merge_policy(),
        MergeTrigger::GetReader,
    )))?;
    iwc.set_max_full_flush_merge_wait_millis(i32::MAX as i64);

    let writer_with_merge_policy = IndexWriter::new(dir.as_directory(), iwc)?;

    {
        // No changes. GetReader doesn't trigger a merge.
        let unmerged_reader = directory_reader::open(dir.as_directory())?;
        assert_equals(&5, &Arc::clone(&unmerged_reader).leaves()?.len());
        unmerged_reader.close()?;
    }

    test_index_writer_add_doc(&writer_with_merge_policy)?;
    {
        let merged_reader = directory_reader::open_with_writer(&*writer_with_merge_policy)?;
        // Doc added, do merge on getReader.
        assert_equals(&1, &Arc::clone(&merged_reader).leaves()?.len());
        merged_reader.close()?;
    }

    writer_with_merge_policy.close()?;
    dir.close()
}

/// `private static class MergeOnXMergePolicy extends FilterMergePolicy`:
/// `new MergeOnXMergePolicy(in, trigger)`.
fn merge_on_x_merge_policy(
    r#in: Arc<dyn MergePolicy>,
    trigger: MergeTrigger,
) -> Arc<dyn MergePolicy> {
    let find_full_flush_merges: FindMergesFn = Arc::new(
        move |_super: &dyn MergePolicy,
              merge_trigger: MergeTrigger,
              segment_infos: &SegmentInfos,
              merge_context: &dyn MergeContext| {
            // Optimize down to a single segment on commit
            if merge_trigger == trigger && segment_infos.size() > 1 {
                let mut non_merging_segments: Vec<SegmentCommitInfo> = Vec::new();
                let merging = merge_context.get_merging_segments();
                for sci in segment_infos.iter() {
                    if !merging.contains(&sci.info.name) {
                        non_merging_segments.push(sci.clone());
                    }
                }
                if non_merging_segments.len() > 1 {
                    let mut merge_specification = MergeSpecification::new();
                    merge_specification.add(OneMerge::new(non_merging_segments)?);
                    return Ok(Some(merge_specification));
                }
            }
            Ok(None)
        },
    );
    Arc::new(FilterMergePolicyOverrides::new(
        r#in,
        None,
        Some(find_full_flush_merges),
    ))
}

/// `wrapSpecification(spec)` of the anonymous `FilterMergePolicy` of
/// `testSetDiagnostics`.
fn wrap_specification(spec: Option<MergeSpecification>) -> Option<MergeSpecification> {
    let spec = spec?;
    let mut new_spec = MergeSpecification::new();
    for merge in &spec.merges {
        let mut merge = OneMerge::wrapping(merge);
        merge.set_set_merge_info_override(Arc::new(|merge, info| {
            merge.super_set_merge_info(info);
            if let Some(info) = merge.info().as_mut() {
                info.info.add_diagnostics(HashMap::from([(
                    "merge_policy".to_string(),
                    "my_merge_policy".to_string(),
                )]));
            }
        }));
        new_spec.add(merge);
    }
    Some(new_spec)
}

/// Port of `TestIndexWriterMergePolicy.testSetDiagnostics`.
fn test_set_diagnostics() -> Result<()> {
    let mut log_mp = LuceneTestCase::new_log_merge_policy_merge_factor(4)?;
    log_mp.set_target_search_concurrency(1)?;
    let my_merge_policy: Arc<dyn MergePolicy> = Arc::new(FilterMergePolicyOverrides::new(
        Arc::new(log_mp),
        Some(Arc::new(
            |sup: &dyn MergePolicy,
             merge_trigger: MergeTrigger,
             segment_infos: &SegmentInfos,
             merge_context: &dyn MergeContext| {
                Ok(wrap_specification(sup.find_merges(
                    merge_trigger,
                    segment_infos,
                    merge_context,
                )?))
            },
        )),
        Some(Arc::new(
            |sup: &dyn MergePolicy,
             merge_trigger: MergeTrigger,
             segment_infos: &SegmentInfos,
             merge_context: &dyn MergeContext| {
                Ok(wrap_specification(sup.find_full_flush_merges(
                    merge_trigger,
                    segment_infos,
                    merge_context,
                )?))
            },
        )),
    ));
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(my_merge_policy))?;
    iwc.set_max_buffered_docs(2)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let doc = Document::new();
    for _ in 0..20 {
        w.add_document(&doc)?;
    }
    w.close()?;
    let si = SegmentInfos::read_latest_commit(dir.as_directory())?;
    let mut has_one_merged_segment = false;
    for sci in si.iter() {
        if sci.info.get_diagnostics().get(SOURCE).map(|s| s.as_str()) == Some(SOURCE_MERGE) {
            assert_equals(
                &Some("my_merge_policy"),
                &sci.info
                    .get_diagnostics()
                    .get("merge_policy")
                    .map(|s| s.as_str()),
            );
            has_one_merged_segment = true;
        }
    }
    Assert::assert_true(has_one_merged_segment);
    w.close()?;
    dir.close()
}

/// `private static final class MockAssertFileExistIndexInput extends IndexInput`.
struct MockAssertFileExistIndexInput {
    resource_description: String,
    name: String,
    delegate: Box<dyn IndexInput>,
    file_path: Path,
}

impl MockAssertFileExistIndexInput {
    fn new(name: &str, delegate: Box<dyn IndexInput>, file_path: Path) -> Self {
        MockAssertFileExistIndexInput {
            resource_description: format!(
                "MockAssertFileExistIndexInput(name={name} delegate={})",
                delegate.to_java_string()
            ),
            name: name.to_string(),
            delegate,
            file_path,
        }
    }

    fn check_file_exist(&self) -> Result<()> {
        if !Files::exists(&self.file_path, &[]) {
            return Err(no_such_file_exception(self.file_path.to_string()).into());
        }
        Ok(())
    }
}

impl DataInput for MockAssertFileExistIndexInput {
    /// The inherited `IndexInput.toString()`: the resource description.
    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    fn read_byte(&mut self) -> Result<u8> {
        self.check_file_exist()?;
        self.delegate.read_byte()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.check_file_exist()?;
        self.delegate.read_bytes(b, offset, len)
    }

    /// The inherited `IndexInput.skipBytes(numBytes)`: `seek(getFilePointer()
    /// + numBytes)` after the argument check.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let pos = self.file_pointer() + num_bytes;
        self.seek(pos)
    }
}

impl IndexInput for MockAssertFileExistIndexInput {
    fn close(&mut self) -> Result<()> {
        self.delegate.close()
    }

    fn file_pointer(&self) -> i64 {
        self.delegate.file_pointer()
    }

    fn length(&self) -> i64 {
        self.delegate.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.check_file_exist()?;
        self.delegate.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.check_file_exist()?;
        let slice = self.delegate.slice(slice_description, offset, length)?;
        Ok(Box::new(MockAssertFileExistIndexInput::new(
            slice_description,
            slice,
            self.file_path.clone(),
        )))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(MockAssertFileExistIndexInput::new(
            &self.name,
            self.delegate.clone_input()?,
            self.file_path.clone(),
        )))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

/// `new FilterDirectory(newFSDirectory(path)) { openInput }`: every input
/// it opens is a `MockAssertFileExistIndexInput` over `path.resolve(name)`.
/// Closing it is closing the `newFSDirectory` holder (see the module docs).
struct AssertFileExistDirectory {
    r#in: Arc<dyn Directory>,
    path: Path,
}

impl Directory for AssertFileExistDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.r#in.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let index_input = self.r#in.open_input(name, context)?;
        Ok(Box::new(MockAssertFileExistIndexInput::new(
            name,
            index_input,
            self.path.resolve_str(name),
        )))
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn ensure_open(&self) -> Result<()> {
        self.r#in.ensure_open()
    }
    fn directory_to_string(&self) -> String {
        format!("FilterDirectory({})", self.r#in.directory_to_string())
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(&*self.r#in)
    }
}

/// `createTempDir(name)` and the `FilterDirectory` over `newFSDirectory(path)`:
/// the holder (closed as `mockDirectory.close()`) and the wrapper.
fn assert_file_exist_directory(name: &str) -> (TestDirectory, Arc<dyn Directory>) {
    let path = LuceneTestCase::create_temp_dir_prefix(name);
    let holder = LuceneTestCase::new_fs_directory(&path);
    let mock_directory: Arc<dyn Directory> = Arc::new(AssertFileExistDirectory {
        r#in: holder.as_directory(),
        path,
    });
    (holder, mock_directory)
}

/// A document with `StringField("id", id, YES)` and `StringField("version", version, YES)`.
fn id_version_doc(id: &str, version: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "id",
        id.to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(StringField::new(
        "version",
        version.to_string(),
        Store::YES,
    )?));
    Ok(doc)
}

/// `new LogDocMergePolicy() { findMerges -> null }` / `new TieredMergePolicy()
/// { findMerges -> null }`: only allow force merge.
fn only_force_merge(base: Arc<dyn MergePolicy>) -> Arc<dyn MergePolicy> {
    Arc::new(SubclassMergePolicy::new(
        base,
        Arc::new(|_super, _trigger, _infos, _context| {
            // only allow force merge
            Ok(None)
        }),
    ))
}

/// Port of `TestIndexWriterMergePolicy.testForceMergeDVUpdateFileWithConcurrentFlush`.
fn test_force_merge_dv_update_file_with_concurrent_flush() -> Result<()> {
    let wait_for_init_merge_reader = Arc::new(CountDownLatch::new(1));
    let wait_for_dv_update = Arc::new(CountDownLatch::new(1));
    let wait_for_merge_finished = Arc::new(CountDownLatch::new(1));

    let (mut holder, mock_directory) =
        assert_file_exist_directory("testForceMergeDVUpdateFileWithConcurrentFlush");

    let mock_merge_policy: Arc<dyn MergePolicy> = {
        let wait_for_init_merge_reader = Arc::clone(&wait_for_init_merge_reader);
        let wait_for_dv_update = Arc::clone(&wait_for_dv_update);
        Arc::new(OneMergeWrappingMergePolicy::new(
            Arc::new(SoftDeletesRetentionMergePolicy::new(
                "soft_delete",
                Arc::new(|| Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>),
                only_force_merge(Arc::new(LogMergePolicy::new(Arc::new(LogDocSize)))),
            )),
            Arc::new(move |merge: OneMerge| {
                let mut wrapped =
                    OneMerge::new(merge.segments.clone()).unwrap_or_else(|e| rethrow_java(e));
                let wait_for_init_merge_reader = Arc::clone(&wait_for_init_merge_reader);
                wrapped.set_init_merge_readers_override(Arc::new(
                    move |merge: &OneMerge, reader_factory| {
                        merge.super_init_merge_readers(reader_factory)?;
                        wait_for_init_merge_reader.count_down();
                        Ok(())
                    },
                ));
                let wait_for_dv_update = Arc::clone(&wait_for_dv_update);
                wrapped.set_wrap_for_merge(Arc::new(move |reader: Arc<dyn CodecReader>| {
                    wait_for_dv_update.await_();
                    // super.wrapForMerge(reader)
                    Ok(reader)
                }));
                wrapped
            }),
        ))
    };

    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(mock_merge_policy))?;
    iwc.set_soft_deletes_field(Some("soft_delete".to_string()));
    let writer = IndexWriter::new(Arc::clone(&mock_directory), iwc)?;

    writer.add_document(&id_version_doc("1", "1")?)?;
    writer.flush()?;
    writer.add_document(&id_version_doc("2", "1")?)?;
    let field = soft_field("soft_delete", 1)?;
    writer.soft_update_document(
        Term::from_text("id", "2"),
        &id_version_doc("2", "2")?,
        &[field],
    )?;
    writer.flush()?;

    let t = {
        let writer = Arc::clone(&writer);
        let wait_for_merge_finished = Arc::clone(&wait_for_merge_finished);
        Thread::unnamed(move || {
            let result = writer.force_merge(1);
            wait_for_merge_finished.count_down();
            if let Err(e) = result {
                rethrow_java(assertion_error_from(e));
            }
        })
    };
    t.start();
    wait_for_init_merge_reader.await_();

    let field = soft_field("soft_delete", 1)?;
    writer.soft_update_document(
        Term::from_text("id", "2"),
        &id_version_doc("2", "3")?,
        &[field],
    )?;
    writer.flush()?;

    wait_for_dv_update.count_down();
    wait_for_merge_finished.await_();

    writer.close()?;
    drop((writer, mock_directory));
    holder.close()
}

/// The two-segment index the `testMergeDVUpdateFileOn*WithConcurrentFlush`
/// tests start from, written through `mock_directory`.
fn two_segments_with_soft_update(mock_directory: &Arc<dyn Directory>) -> Result<()> {
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let first_writer = IndexWriter::new(Arc::clone(mock_directory), iwc)?;

    first_writer.add_document(&id_version_doc("1", "1")?)?;
    first_writer.flush()?;
    first_writer.add_document(&id_version_doc("2", "1")?)?;
    let field = soft_field("soft_delete", 1)?;
    first_writer.soft_update_document(
        Term::from_text("id", "2"),
        &id_version_doc("2", "2")?,
        &[field],
    )?;
    first_writer.flush()?;
    let first_reader = directory_reader::open_with_writer(&*first_writer)?;
    assert_equals(&2, &Arc::clone(&first_reader).leaves()?.len());
    first_reader.close()?;
    first_writer.close()
}

/// The anonymous `ConcurrentMergeScheduler` of the
/// `testMergeDVUpdateFileOn*WithConcurrentFlush` tests.
fn waiting_cms(
    wait_for_init_merge_reader: &Arc<CountDownLatch>,
    wait_for_dv_update: &Arc<CountDownLatch>,
) -> Arc<PrologueMergeScheduler> {
    let wait_for_init_merge_reader = Arc::clone(wait_for_init_merge_reader);
    let wait_for_dv_update = Arc::clone(wait_for_dv_update);
    Arc::new(PrologueMergeScheduler::new(
        Arc::new(ConcurrentMergeScheduler::new()),
        false,
        Arc::new(move || {
            wait_for_init_merge_reader.count_down();
            wait_for_dv_update.await_();
            Ok(())
        }),
    ))
}

/// The thread of the `testMergeDVUpdateFileOn*WithConcurrentFlush` tests.
fn soft_update_thread(
    writer_with_merge_policy: &Arc<IndexWriter>,
    wait_for_init_merge_reader: &Arc<CountDownLatch>,
    wait_for_dv_update: &Arc<CountDownLatch>,
) -> Thread {
    let writer_with_merge_policy = Arc::clone(writer_with_merge_policy);
    let wait_for_init_merge_reader = Arc::clone(wait_for_init_merge_reader);
    let wait_for_dv_update = Arc::clone(wait_for_dv_update);
    Thread::unnamed(move || {
        let result = (|| -> Result<()> {
            wait_for_init_merge_reader.await_();

            let update_doc = id_version_doc("2", "3")?;
            let soft_delete_field = soft_field("soft_delete", 1)?;
            writer_with_merge_policy.soft_update_document(
                Term::from_text("id", "2"),
                &update_doc,
                &[soft_delete_field],
            )?;
            let reader =
                directory_reader::open_with_writer_flags(&*writer_with_merge_policy, true, false)?;
            reader.close()?;

            wait_for_dv_update.count_down();
            Ok(())
        })();
        if let Err(e) = result {
            rethrow_java(assertion_error_from(e));
        }
    })
}

/// Port of `TestIndexWriterMergePolicy.testMergeDVUpdateFileOnGetReaderWithConcurrentFlush`.
fn test_merge_dv_update_file_on_get_reader_with_concurrent_flush() -> Result<()> {
    let wait_for_init_merge_reader = Arc::new(CountDownLatch::new(1));
    let wait_for_dv_update = Arc::new(CountDownLatch::new(1));

    let (mut holder, mock_directory) =
        assert_file_exist_directory("testMergeDVUpdateFileOnGetReaderWithConcurrentFlush");

    two_segments_with_soft_update(&mock_directory)?;

    let mock_concurrent_merge_scheduler =
        waiting_cms(&wait_for_init_merge_reader, &wait_for_dv_update);

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(merge_on_x_merge_policy(
        LuceneTestCase::new_merge_policy(),
        MergeTrigger::GetReader,
    )))?;
    iwc.set_max_full_flush_merge_wait_millis(i32::MAX as i64);
    iwc.set_merge_scheduler(Some(mock_concurrent_merge_scheduler))?;

    let writer_with_merge_policy = IndexWriter::new(Arc::clone(&mock_directory), iwc)?;

    let t = soft_update_thread(
        &writer_with_merge_policy,
        &wait_for_init_merge_reader,
        &wait_for_dv_update,
    );
    t.start();

    {
        let merged_reader = directory_reader::open_with_writer(&*writer_with_merge_policy)?;
        assert_equals(&1, &Arc::clone(&merged_reader).leaves()?.len());
        merged_reader.close()?;
    }

    writer_with_merge_policy.close()?;
    drop((writer_with_merge_policy, mock_directory));
    holder.close()
}

/// Port of `TestIndexWriterMergePolicy.testMergeDVUpdateFileOnCommitWithConcurrentFlush`.
fn test_merge_dv_update_file_on_commit_with_concurrent_flush() -> Result<()> {
    let wait_for_init_merge_reader = Arc::new(CountDownLatch::new(1));
    let wait_for_dv_update = Arc::new(CountDownLatch::new(1));

    let (mut holder, mock_directory) =
        assert_file_exist_directory("testMergeDVUpdateFileOnCommitWithConcurrentFlush");

    two_segments_with_soft_update(&mock_directory)?;

    let mock_concurrent_merge_scheduler =
        waiting_cms(&wait_for_init_merge_reader, &wait_for_dv_update);

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(merge_on_x_merge_policy(
        LuceneTestCase::new_merge_policy(),
        MergeTrigger::Commit,
    )))?;
    iwc.set_max_full_flush_merge_wait_millis(i32::MAX as i64);
    iwc.set_merge_scheduler(Some(mock_concurrent_merge_scheduler))?;

    let writer_with_merge_policy = IndexWriter::new(Arc::clone(&mock_directory), iwc)?;

    let t = soft_update_thread(
        &writer_with_merge_policy,
        &wait_for_init_merge_reader,
        &wait_for_dv_update,
    );
    t.start();

    writer_with_merge_policy.commit()?;
    assert_equals(&2, &writer_with_merge_policy.get_segment_count()?);

    writer_with_merge_policy.close()?;
    drop((writer_with_merge_policy, mock_directory));
    holder.close()
}

/// Port of `TestIndexWriterMergePolicy.testForceMergeWithPendingHardAndSoftDeleteFile`.
fn test_force_merge_with_pending_hard_and_soft_delete_file() -> Result<()> {
    let (mut holder, mock_directory) =
        assert_file_exist_directory("testForceMergeWithPendingHardAndSoftDeleteFile");

    let mock_merge_policy: Arc<dyn MergePolicy> = Arc::new(OneMergeWrappingMergePolicy::new(
        only_force_merge(Arc::new(TieredMergePolicy::new())),
        Arc::new(|merge: OneMerge| {
            OneMerge::new(merge.segments.clone()).unwrap_or_else(|e| rethrow_java(e))
        }),
    ));

    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(mock_merge_policy))?;
    let writer = IndexWriter::new(Arc::clone(&mock_directory), iwc)?;

    writer.add_document(&id_version_doc("1", "1")?)?;
    writer.commit()?;

    writer.add_document(&id_version_doc("2", "1")?)?;

    writer.add_document(&id_version_doc("3", "1")?)?;

    writer.add_document(&id_version_doc("4", "1")?)?;

    writer.add_document(&id_version_doc("5", "1")?)?;
    writer.commit()?;

    writer.update_document(Term::from_text("id", "2"), &id_version_doc("2", "2")?)?;
    writer.commit()?;

    writer.update_document(Term::from_text("id", "3"), &id_version_doc("3", "2")?)?;

    let field = soft_field("soft_delete", 1)?;
    writer.soft_update_document(
        Term::from_text("id", "4"),
        &id_version_doc("4", "2")?,
        &[field],
    )?;

    let reader = directory_reader::open_with_writer_flags(&*writer, true, false)?;
    reader.close()?;
    writer.commit()?;

    writer.force_merge(1)?;

    writer.close()?;
    drop((writer, mock_directory));
    holder.close()
}
