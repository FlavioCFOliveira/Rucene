//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriter.java`.
//!
//! * `addDoc`, `addDocWithIndex` and `assertNoUnreferencedFiles` are
//!   `support.rs`'s ports (`add_doc`, `add_doc_with_index`,
//!   `assert_no_unreferenced_files`), which other index tests share.
//! * `MockIndexWriter` and the other `IndexWriter` subclasses are
//!   [`IndexWriterOverrides`] implementations installed with
//!   `IndexWriter::set_overrides`.
//! * Fields whose value Java changes after `doc.add` are changed in place
//!   through the document (`support::field_mut`, `Document::get_field_mut_as`;
//!   guide section 6).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Assert, Ignore};
use rucene::analysis::tokenattributes::PayloadAttribute;
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy,
    StringCharReader, TokenStream, TokenStreamComponents, Tokenizer, TokenizerLogic,
};
use rucene::codecs::doc_values::DocValuesProducer;
use rucene::codecs::knn_vectors::KnnVectorsReader;
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::points::PointsReader;
use rucene::codecs::postings::FieldsProducer;
use rucene::codecs::stored_fields::StoredFieldsReader;
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::document::LongPoint;
use rucene::document::{
    BinaryDocValuesField, Document, Field, FieldType, InvertableType, NumericDocValuesField,
    NumericValue, SharedReader, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField, Store, StoredField, StoredValue, StringField, TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::check_index::{CheckIndex, Level};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::documents_writer::MAX_STORED_STRING_LENGTH;
use rucene::index::documents_writer::{DocumentsWriterPerThread, FlushControlHandle, FlushPolicy};
use rucene::index::filter_reader::FilterCodecReader;
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::NoDeletionPolicy;
use rucene::index::index_deletion_policy::{
    KeepOnlyLastCommitDeletionPolicy, SnapshotDeletionPolicy,
};
use rucene::index::index_file_deleter::WRITE_LOCK_NAME;
use rucene::index::index_file_names;
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::event_queue::{Event, EventQueue};
use rucene::index::index_writer::DocStats;
use rucene::index::index_writer::{IndexWriter, IndexWriterOverrides, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::LogDocSize;
use rucene::index::merge_policy::{
    CodecReaderSupplier, MergeContext, MergeSpecification, MergeTrigger, OneMerge,
    OneMergeWrappingMergePolicy, SoftDeletesRetentionMergePolicy,
};
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy, NoMergePolicy};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::index::{DefaultIndexingChain, DocValuesSkipIndexType, DocValuesType};
use rucene::index::{DocumentsIterable, DocumentsIterator, IndexableFieldIterable};
use rucene::index::{
    IndexOptions, IndexReader, IndexableField, IndexableFieldType, LeafReader, SegmentCommitInfo,
    POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::Builder as PhraseQueryBuilder;
use rucene::search::MatchAllDocsQuery;
use rucene::search::{DefaultSearcherFactory, Query, SearcherManager};
use rucene::search::{TermQuery, NO_MORE_DOCS};
use rucene::store::java_nio_file::Files;
use rucene::store::java_nio_file::Path;
use rucene::store::{
    ByteBuffersDirectory, Directory, NIOFSDirectory, NoLockFactory, SimpleFSLockFactory,
};
use rucene::store::{FilterDirectory, IndexInput, DEFAULT_IO_CONTEXT};
use rucene::store::{IOContext, IndexOutput, Lock};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{Automata, CharacterRunAutomaton};
use rucene::util::concurrent::InterruptedException;
use rucene::util::java_collections::ImmutableSet;
use rucene::util::string_helper::{StringHelper, ID_LENGTH};
use rucene::util::InfoStream;
use rucene::util::{Bits, SetOnce, Version};
use rucene::util::{BytesRef, ThreadInterruptedException};
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::CannedTokenStream;
use rucene_test_framework::analysis::Token;
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, MockAnalyzer, MockTokenFilter,
};
use rucene_test_framework::index::random_index_writer::TestPoint;
use rucene_test_framework::index::suppressing_concurrent_merge_scheduler::SuppressingConcurrentMergeScheduler;
use rucene_test_framework::index::DocHelper;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::mockfile::ExtrasFS;
use rucene_test_framework::mockfile::{FilterFileSystemProvider, WindowsFS};
use rucene_test_framework::store::Failure;
use rucene_test_framework::store::{MockDirectoryWrapper, Throttling};
use rucene_test_framework::util::java_exceptions::{
    expect_throws, is_instance, java_throwable, rethrow_java,
};
use rucene_test_framework::util::lucene_test_case::NewFieldValue;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, PrintStreamLines, TestUtil};

use crate::core_support::{
    assert_array_equals, assert_equals, assert_equals_msg, assert_not_equals, assert_not_null,
    assert_null, class, random, test, verbose, IAE,
};
use crate::support::{
    add_doc, add_doc_with_index, assert_no_unreferenced_files, assert_null_msg, field_mut, npe,
    set_check_index_on_close, CountDownLatch, KeepFullyDeletedSegmentsMergePolicy,
};

/// `conf.getMergeScheduler() instanceof ConcurrentMergeScheduler`.
fn is_cms(iwc: &IndexWriterConfig) -> bool {
    iwc.merge_scheduler()
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .is_some()
}

/// `TestIndexWriter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriter",
        vec![
            test("testDocCount", test_doc_count),
            test("testCreateWithReader", test_create_with_reader),
            test("testChangesAfterClose", test_changes_after_close),
            test("testIndexNoDocuments", test_index_no_documents),
            test("testSmallRAMBuffer", test_small_ram_buffer),
            test("testChangingRAMBuffer", test_changing_ram_buffer),
            test("testEnablingNorms", test_enabling_norms),
            test("testHighFreqTerm", test_high_freq_term),
            test("testFlushWithNoMerging", test_flush_with_no_merging),
            test(
                "testEmptyDocAfterFlushingRealDoc",
                test_empty_doc_after_flushing_real_doc,
            ),
            test("testBadSegment", test_bad_segment),
            test("testMaxThreadPriority", test_max_thread_priority),
            test("testVariableSchema", test_variable_schema),
            test(
                "testUnlimitedMaxFieldLength",
                test_unlimited_max_field_length,
            ),
            test("testEmptyFieldName", test_empty_field_name),
            test("testEmptyFieldNameTerms", test_empty_field_name_terms),
            test(
                "testEmptyFieldNameWithEmptyTerm",
                test_empty_field_name_with_empty_term,
            ),
            test("testDoBeforeAfterFlush", test_do_before_after_flush),
            test("testNegativePositions", test_negative_positions),
            test(
                "testPositionIncrementGapEmptyField",
                test_position_increment_gap_empty_field,
            ),
            test("testDeadlock", test_deadlock),
            test(
                "testThreadInterruptDeadlock",
                test_thread_interrupt_deadlock,
            ), // @SuppressForbidden(reason = "Thread sleep")
            test("testIndexStoreCombos", test_index_store_combos),
            test("testNoDocsIndex", test_no_docs_index),
            test("testDeleteUnusedFiles", test_delete_unused_files),
            test("testDeleteUnusedFiles2", test_delete_unused_files2),
            test("testEmptyFSDirWithNoLock", test_empty_fs_dir_with_no_lock),
            test("testEmptyDirRollback", test_empty_dir_rollback),
            test("testNoUnwantedTVFiles", test_no_unwanted_tv_files),
            test("testWickedLongTerm", test_wicked_long_term),
            test(
                "testDeleteAllNRTLeftoverFiles",
                test_delete_all_nrt_leftover_files,
            ),
            test("testNRTReaderVersion", test_nrt_reader_version),
            test(
                "testWhetherDeleteAllDeletesWriteLock",
                test_whether_delete_all_deletes_write_lock,
            ),
            test(
                "testHasBlocksMergeFullyDelSegments",
                test_has_blocks_merge_fully_del_segments,
            ),
            test(
                "testSingleDocsDoNotTriggerHasBlocks",
                test_single_docs_do_not_trigger_has_blocks,
            ),
            test("testCarryOverHasBlocks", test_carry_over_has_blocks),
            test("testPrepareCommitThenClose", test_prepare_commit_then_close),
            test(
                "testPrepareCommitThenRollback",
                test_prepare_commit_then_rollback,
            ),
            test(
                "testPrepareCommitThenRollback2",
                test_prepare_commit_then_rollback2,
            ),
            test(
                "testDontInvokeAnalyzerForUnAnalyzedFields",
                test_dont_invoke_analyzer_for_un_analyzed_fields,
            ),
            test("testOtherFiles", test_other_files),
            test("testStopwordsPosIncHole", test_stopwords_pos_inc_hole),
            test("testStopwordsPosIncHole2", test_stopwords_pos_inc_hole2),
            test(
                "testCommitWithUserDataOnly",
                test_commit_with_user_data_only,
            ),
            test("testGetCommitData", test_get_commit_data),
            test(
                "testGetCommitDataFromOldSnapshot",
                test_get_commit_data_from_old_snapshot,
            ),
            test("testNullAnalyzer", test_null_analyzer),
            test("testNullDocument", test_null_document),
            test("testNullDocuments", test_null_documents),
            test(
                "testIterableFieldThrowsException",
                test_iterable_field_throws_exception,
            ),
            test(
                "testIterableThrowsException",
                test_iterable_throws_exception,
            ),
            test(
                "testIterableThrowsException2",
                test_iterable_throws_exception2,
            ),
            test("testCorruptFirstCommit", test_corrupt_first_commit),
            test("testHasUncommittedChanges", test_has_uncommitted_changes),
            test("testMergeAllDeleted", test_merge_all_deleted),
            test(
                "testDeleteSameTermAcrossFields",
                test_delete_same_term_across_fields,
            ),
            test(
                "testHasUncommittedChangesAfterException",
                test_has_uncommitted_changes_after_exception,
            ),
            test("testDoubleClose", test_double_close),
            test("testRollbackThenClose", test_rollback_then_close),
            test("testCloseThenRollback", test_close_then_rollback),
            test(
                "testCloseWhileMergeIsRunning",
                test_close_while_merge_is_running,
            ),
            test("testCloseDuringCommit", test_close_during_commit), // @SuppressForbidden(reason = "Thread sleep")
            test("testIds", test_ids),
            test("testEmptyNorm", test_empty_norm),
            test("testManySeparateThreads", test_many_separate_threads),
            test("testNRTSegmentsFile", test_nrt_segments_file),
            test("testNRTAfterCommit", test_nrt_after_commit),
            test(
                "testNRTAfterSetUserDataWithoutCommit",
                test_nrt_after_set_user_data_without_commit,
            ),
            test(
                "testNRTAfterSetUserDataWithCommit",
                test_nrt_after_set_user_data_with_commit,
            ),
            test(
                "testCommitImmediatelyAfterNRTReopen",
                test_commit_immediately_after_nrt_reopen,
            ),
            test(
                "testPendingDeleteDVGeneration",
                test_pending_delete_dv_generation,
            ),
            test(
                "testPendingDeletionsRollbackWithReader",
                test_pending_deletions_rollback_with_reader,
            ),
            test("testWithPendingDeletions", test_with_pending_deletions),
            test(
                "testPendingDeletesAlreadyWrittenFiles",
                test_pending_deletes_already_written_files,
            ),
            test("testLeftoverTempFiles", test_leftover_temp_files),
            test("testMassiveField", test_massive_field).annotate(Ignore {
                value: "requires running tests with biggish heap".to_string(),
            }),
            test(
                "testRecordsIndexCreatedVersion",
                test_records_index_created_version,
            ),
            test("testFlushLargestWriter", test_flush_largest_writer),
            test(
                "testNeverCheckOutOnFullFlush",
                test_never_check_out_on_full_flush,
            ),
            test(
                "testApplyDeletesWithoutFlushes",
                test_apply_deletes_without_flushes,
            ),
            test("testDeletesAppliedOnFlush", test_deletes_applied_on_flush),
            test(
                "testHoldLockOnLargestWriter",
                test_hold_lock_on_largest_writer,
            ),
            test(
                "testCheckPendingFlushPostUpdate",
                test_check_pending_flush_post_update,
            ),
            test("testSoftUpdateDocuments", test_soft_update_documents),
            test(
                "testSoftUpdatesConcurrently",
                test_soft_updates_concurrently,
            ),
            test(
                "testSoftUpdatesConcurrentlyMixedDeletes",
                test_soft_updates_concurrently_mixed_deletes,
            ),
            test(
                "testDeleteHappensBeforeWhileFlush",
                test_delete_happens_before_while_flush,
            ),
            test(
                "testFullyDeletedSegmentsReleaseFiles",
                test_fully_deleted_segments_release_files,
            ),
            test("testSegmentInfoIsSnapshot", test_segment_info_is_snapshot),
            test(
                "testPreventChangingSoftDeletesField",
                test_prevent_changing_soft_deletes_field,
            ),
            test(
                "testPreventAddingIndexesWithDifferentSoftDeletesField",
                test_prevent_adding_indexes_with_different_soft_deletes_field,
            ),
            test(
                "testNotAllowUsingExistingFieldAsSoftDeletes",
                test_not_allow_using_existing_field_as_soft_deletes,
            ),
            test("testBrokenPayload", test_broken_payload),
            test("testSoftAndHardLiveDocs", test_soft_and_hard_live_docs),
            test(
                "testAbortFullyDeletedSegment",
                test_abort_fully_deleted_segment,
            ),
            test("testSetIndexCreatedVersion", test_set_index_created_version),
            test(
                "testFlushWhileStartingNewThreads",
                test_flush_while_starting_new_threads,
            ),
            test(
                "testRefreshAndRollbackConcurrently",
                test_refresh_and_rollback_concurrently,
            ),
            test("testCloseableQueue", test_closeable_queue),
            test("testRandomOperations", test_random_operations),
            test(
                "testRandomOperationsWithSoftDeletes",
                test_random_operations_with_soft_deletes,
            ),
            test(
                "testMaxCompletedSequenceNumber",
                test_max_completed_sequence_number,
            ),
            test(
                "testEnsureMaxSeqNoIsAccurateDuringFlush",
                test_ensure_max_seq_no_is_accurate_during_flush,
            ),
            test("testSegmentCommitInfoId", test_segment_commit_info_id),
            test(
                "testMergeZeroDocsMergeIsClosedOnce",
                test_merge_zero_docs_merge_is_closed_once,
            ),
            test(
                "testMergeOnCommitKeepFullyDeletedSegments",
                test_merge_on_commit_keep_fully_deleted_segments,
            ),
            test("testPendingNumDocs", test_pending_num_docs),
            test(
                "testIndexWriterBlocksOnStall",
                test_index_writer_blocks_on_stall,
            ),
            test("testGetFieldNames", test_get_field_names),
            test(
                "testParentAndSoftDeletesAreTheSame",
                test_parent_and_soft_deletes_are_the_same,
            ),
            test(
                "testParentFieldExistingIndex",
                test_parent_field_existing_index,
            ),
            test(
                "testIndexWithParentFieldIsCongruent",
                test_index_with_parent_field_is_congruent,
            ),
            test(
                "testParentFieldIsAlreadyUsed",
                test_parent_field_is_already_used,
            ),
            test("testParentFieldEmptyIndex", test_parent_field_empty_index),
            test(
                "testSingleDocBlockWritesParentField",
                test_single_doc_block_writes_parent_field,
            ),
            test(
                "testDocValuesMixedSkippingIndex",
                test_doc_values_mixed_skipping_index,
            ),
            test(
                "testDocValuesSkippingIndexWithoutDocValues",
                test_doc_values_skipping_index_without_doc_values,
            ),
            test(
                "testAdvanceSegmentInfosCounter",
                test_advance_segment_infos_counter,
            ),
            test(
                "testAdvanceSegmentCounterInCrashAndRecoveryScenario",
                test_advance_segment_counter_in_crash_and_recovery_scenario,
            ),
        ],
    )
}

/// `private static final FieldType storedTextType = new
/// FieldType(TextField.TYPE_NOT_STORED)`.
fn stored_text_type() -> FieldType {
    FieldType::new_from(TextField::type_not_stored())
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new IndexWriter(dir, conf)`.
fn writer(dir: &Arc<dyn Directory>, conf: IndexWriterConfig) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(Arc::clone(dir), conf)
}

/// `reader` as an `IndexReader`.
fn ir(reader: &Arc<dyn DirectoryReader>) -> Arc<dyn IndexReader> {
    Arc::clone(reader) as Arc<dyn IndexReader>
}

/// A `FieldType` copied from `TextField.TYPE_STORED` with term vectors,
/// positions and offsets.
fn tv_type() -> Result<FieldType> {
    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    custom_type.set_store_term_vector_offsets(true)?;
    Ok(custom_type)
}

/// Port of `TestIndexWriter.testDocCount`.
fn test_doc_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let writer1 = writer(&d, mock_iwc())?;

    // add 100 documents
    for i in 0..100 {
        add_doc_with_index(&writer1, i)?;
        if random().next_boolean() {
            writer1.commit()?;
        }
    }
    let doc_stats = writer1.get_doc_stats()?;
    assert_equals(&100, &doc_stats.max_doc);
    assert_equals(&100, &doc_stats.num_docs);
    writer1.close()?;

    // delete 40 documents
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(KeepFullyDeletedSegmentsMergePolicy::new(
        Arc::new(NoMergePolicy::new()),
    ))))?;
    let writer2 = writer(&d, iwc)?;

    for i in 0..40 {
        writer2.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
        if random().next_boolean() {
            writer2.commit()?;
        }
    }
    writer2.flush()?;
    let doc_stats = writer2.get_doc_stats()?;
    assert_equals(&100, &doc_stats.max_doc);
    assert_equals(&60, &doc_stats.num_docs);
    writer2.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&60, &reader.num_docs());
    reader.close()?;

    // merge the index down and check that the new doc count is correct
    let writer3 = writer(&d, mock_iwc())?;
    assert_equals(&60, &writer3.get_doc_stats()?.num_docs);
    writer3.force_merge(1)?;
    let doc_stats = writer3.get_doc_stats()?;
    assert_equals(&60, &doc_stats.max_doc);
    assert_equals(&60, &doc_stats.num_docs);
    writer3.close()?;

    // check that the index reader gives the same numbers.
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&60, &reader.max_doc());
    assert_equals(&60, &reader.num_docs());
    reader.close()?;

    // make sure opening a new index for create over this existing one works correctly:
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::CREATE)?;
    let writer4 = writer(&d, iwc)?;
    let doc_stats = writer4.get_doc_stats()?;
    assert_equals(&0, &doc_stats.max_doc);
    assert_equals(&0, &doc_stats.num_docs);
    writer4.close()?;
    drop(d);
    dir.close()
}

/// Make sure we can open an index for create even when a
/// reader holds it open (this fails pre lock-less commits on Windows):
/// Port of `TestIndexWriter.testCreateWithReader`.
fn test_create_with_reader() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    // add one document & close writer
    let w = writer(&d, mock_iwc())?;
    add_doc(&w)?;
    w.close()?;

    // now open reader:
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(Some("should be one document"), &reader.num_docs(), &1);

    // now open index for create:
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::CREATE)?;
    let w = writer(&d, iwc)?;
    assert_equals_msg(
        Some("should be zero documents"),
        &w.get_doc_stats()?.max_doc,
        &0,
    );
    add_doc(&w)?;
    w.close()?;

    assert_equals_msg(Some("should be one document"), &reader.num_docs(), &1);
    let reader2 = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(Some("should be one document"), &reader2.num_docs(), &1);
    reader.close()?;
    reader2.close()?;

    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testChangesAfterClose`.
fn test_changes_after_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;

    add_doc(&w)?;

    // close
    w.close()?;
    expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
        add_doc(&w)
    });

    drop((w, d));
    dir.close()
}

/// Port of `TestIndexWriter.testIndexNoDocuments`.
fn test_index_no_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    w.commit()?;
    w.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.max_doc());
    assert_equals(&0, &reader.num_docs());
    reader.close()?;

    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let w = writer(&d, iwc)?;
    w.commit()?;
    w.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.max_doc());
    assert_equals(&0, &reader.num_docs());
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testSmallRAMBuffer`.
fn test_small_ram_buffer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    iwc.set_ram_buffer_size_mb(0.000001)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let w = writer(&d, iwc)?;
    let mut last_num_segments = get_segment_count(&d)?;
    for j in 0..9 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            "field",
            &format!("aaa{j}"),
            &stored_text_type(),
        )));
        w.add_document(&doc)?;
        // Verify that with a tiny RAM buffer we see new segment after every doc
        let num_segments = get_segment_count(&d)?;
        Assert::assert_true(num_segments > last_num_segments);
        last_num_segments = num_segments;
    }
    w.close()?;
    drop(d);
    dir.close()
}

/// Returns how many unique segment names are in the directory.
fn get_segment_count(dir: &Arc<dyn Directory>) -> Result<usize> {
    let mut segments = HashSet::new();
    for file in dir.list_all()? {
        segments.insert(index_file_names::parse_segment_name(&file).to_string());
    }

    Ok(segments.len())
}

/// Make sure it's OK to change RAM buffer size and maxBufferedDocs in a write session
/// Port of `TestIndexWriter.testChangingRAMBuffer`.
fn test_changing_ram_buffer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let disable = LiveIndexWriterConfig::DISABLE_AUTO_FLUSH;
    w.get_config().set_max_buffered_docs(10)?;
    w.get_config().set_ram_buffer_size_mb(disable as f64)?;

    let mut last_flush_count = -1;
    for j in 1..52 {
        let mut doc = Document::new();
        doc.add(Box::new(Field::new(
            "field",
            format!("aaa{j}"),
            stored_text_type(),
        )?));
        w.add_document(&doc)?;
        TestUtil::sync_concurrent_merges_of_writer(&w);
        let flush_count = w.get_flush_count();
        if j == 1 {
            last_flush_count = flush_count;
        } else if j < 10 {
            // No new files should be created
            assert_equals(&flush_count, &last_flush_count);
        } else if 10 == j {
            Assert::assert_true(flush_count > last_flush_count);
            last_flush_count = flush_count;
            w.get_config().set_ram_buffer_size_mb(0.000001)?;
            w.get_config().set_max_buffered_docs(disable)?;
        } else if j < 20 {
            Assert::assert_true(flush_count > last_flush_count);
            last_flush_count = flush_count;
        } else if 20 == j {
            w.get_config().set_ram_buffer_size_mb(16.0)?;
            w.get_config().set_max_buffered_docs(disable)?;
            last_flush_count = flush_count;
        } else if j < 30 {
            assert_equals(&flush_count, &last_flush_count);
        } else if 30 == j {
            w.get_config().set_ram_buffer_size_mb(0.000001)?;
            w.get_config().set_max_buffered_docs(disable)?;
        } else if j < 40 {
            Assert::assert_true(flush_count > last_flush_count);
            last_flush_count = flush_count;
        } else if 40 == j {
            w.get_config().set_max_buffered_docs(10)?;
            w.get_config().set_ram_buffer_size_mb(disable as f64)?;
            last_flush_count = flush_count;
        } else if j < 50 {
            assert_equals(&flush_count, &last_flush_count);
            w.get_config().set_max_buffered_docs(10)?;
            w.get_config().set_ram_buffer_size_mb(disable as f64)?;
        } else if 50 == j {
            Assert::assert_true(flush_count > last_flush_count);
        }
    }
    w.close()?;
    drop(d);
    dir.close()
}

/// `newSearcher(reader).search(new TermQuery(term), 1000).scoreDocs.length`.
fn hit_count(reader: &Arc<dyn DirectoryReader>, term: &Term) -> Result<usize> {
    let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
    Ok(searcher
        .search(Arc::new(TermQuery::new(term.clone())), 1000)?
        .score_docs
        .len())
}

/// Port of `TestIndexWriter.testEnablingNorms`.
fn test_enabling_norms() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(10)?;
    let w = writer(&d, iwc)?;
    // Enable norms for only 1 doc, pre flush
    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_omit_norms(true)?;
    for j in 0..10 {
        let mut doc = Document::new();
        let f = if j != 8 {
            LuceneTestCase::new_field("field", "aaa", &custom_type)
        } else {
            LuceneTestCase::new_field("field", "aaa", &stored_text_type())
        };
        doc.add(Box::new(f));
        w.add_document(&doc)?;
    }
    w.close()?;

    let search_term = Term::from_text("field", "aaa");

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&10, &hit_count(&reader, &search_term)?);
    reader.close()?;

    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_max_buffered_docs(10)?;
    let w = writer(&d, iwc)?;
    // Enable norms for only 1 doc, post flush
    for j in 0..27 {
        let mut doc = Document::new();
        let f = if j != 26 {
            LuceneTestCase::new_field("field", "aaa", &custom_type)
        } else {
            LuceneTestCase::new_field("field", "aaa", &stored_text_type())
        };
        doc.add(Box::new(f));
        w.add_document(&doc)?;
    }
    w.close()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&27, &hit_count(&reader, &search_term)?);
    reader.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    reader.close()?;

    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testHighFreqTerm`.
fn test_high_freq_term() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    iwc.set_ram_buffer_size_mb(0.01)?;
    let w = writer(&d, iwc)?;
    // Massive doc that has 128 K a's
    let mut b = String::with_capacity(1024 * 1024);
    for _ in 0..4096 {
        b.push_str(" a a a a a a a a");
        b.push_str(" a a a a a a a a");
        b.push_str(" a a a a a a a a");
        b.push_str(" a a a a a a a a");
    }
    let mut doc = Document::new();
    let custom_type = tv_type()?;
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        &b,
        &custom_type,
    )));
    w.add_document(&doc)?;
    w.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&1, &reader.max_doc());
    assert_equals(&1, &reader.num_docs());
    let t = Term::from_text("field", "a");
    assert_equals(&1, &reader.doc_freq(&t)?);
    let mut td = TestUtil::docs(
        &*random(),
        &ir(&reader),
        "field",
        &LuceneTestCase::new_bytes_ref_str("a"),
        None,
        POSTINGS_ENUM_FREQS,
    )?
    .unwrap_or_else(|| npe("td"));
    td.next_doc()?;
    assert_equals(&(128 * 1024), &td.freq()?);
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testFlushWithNoMerging`.
fn test_flush_with_no_merging() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let w = writer(&d, iwc)?;
    let mut doc = Document::new();
    let custom_type = tv_type()?;
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "aaa",
        &custom_type,
    )));
    for _ in 0..19 {
        w.add_document(&doc)?;
    }
    w.flush_with(false, true)?;
    // Since we flushed w/o allowing merging we should now have 10 segments
    assert_equals(&10, &w.get_segment_count()?);
    w.close()?;
    drop(d);
    dir.close()
}

/// Make sure we can flush segment w/ norms, then add empty doc (no norms) and flush
/// Port of `TestIndexWriter.testEmptyDocAfterFlushingRealDoc`.
fn test_empty_doc_after_flushing_real_doc() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut doc = Document::new();
    let custom_type = tv_type()?;
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "aaa",
        &custom_type,
    )));
    w.add_document(&doc)?;
    w.commit()?;
    if verbose() {
        println!("\nTEST: now add empty doc");
    }
    w.add_document(&Document::new())?;
    w.close()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&2, &reader.num_docs());
    reader.close()?;
    drop(d);
    dir.close()
}

/// Test that no NullPointerException will be raised, when adding one document with a single, empty
/// field and term vectors enabled.
/// Port of `TestIndexWriter.testBadSegment`.
fn test_bad_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iw = writer(&d, mock_iwc())?;

    let mut document = Document::new();
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;
    document.add(Box::new(LuceneTestCase::new_field(
        "tvtest",
        "",
        &custom_type,
    )));
    iw.add_document(&document)?;
    iw.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-1036
/// Port of `TestIndexWriter.testMaxThreadPriority`.
fn test_max_thread_priority() -> Result<()> {
    let pri = Thread::current().priority();
    let result = (|| -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        let conf = mock_iwc();
        conf.set_max_buffered_docs(2)?;
        conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        conf.merge_policy()
            .as_any()
            .and_then(|a| a.downcast_ref::<LogMergePolicy>())
            .unwrap_or_else(|| npe("(LogMergePolicy) conf.getMergePolicy()"))
            .set_merge_factor(2)?;
        let iw = writer(&d, conf)?;
        let mut document = Document::new();
        let custom_type = FieldType::new_from(TextField::type_not_stored());
        custom_type.set_store_term_vectors(true)?;
        document.add(Box::new(LuceneTestCase::new_field(
            "tvtest",
            "a b c",
            &custom_type,
        )));
        Thread::current().set_priority(Thread::MAX_PRIORITY);
        for _ in 0..4 {
            iw.add_document(&document)?;
        }
        iw.close()?;
        drop(d);
        dir.close()
    })();
    Thread::current().set_priority(pri);
    result
}

/// Port of `TestIndexWriter.testVariableSchema`.
fn test_variable_schema() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    for i in 0..20 {
        if verbose() {
            println!("TEST: iter={i}");
        }
        let iwc = mock_iwc();
        iwc.set_max_buffered_docs(2)?;
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let w = writer(&d, iwc)?;
        // LogMergePolicy lmp = (LogMergePolicy) writer.getConfig().getMergePolicy();
        // lmp.setMergeFactor(2);
        // lmp.setNoCFSRatio(0.0);
        let mut doc = Document::new();
        let contents = "aa bb cc dd ee ff gg hh ii jj kk";

        let custom_type = FieldType::new_from(TextField::type_stored());
        if i == 7 {
            // Add empty docs here
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "content3",
                "",
                Store::NO,
            )));
        } else {
            let type_ = if i % 2 == 0 {
                doc.add(Box::new(LuceneTestCase::new_field(
                    "content4",
                    contents,
                    &custom_type,
                )));
                custom_type.clone()
            } else {
                TextField::type_not_stored().clone()
            };
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "content1",
                contents,
                Store::NO,
            )));
            doc.add(Box::new(LuceneTestCase::new_field(
                "content3",
                "",
                &custom_type,
            )));
            doc.add(Box::new(LuceneTestCase::new_field("content5", "", &type_)));
        }

        for _ in 0..4 {
            w.add_document(&doc)?;
        }

        w.close()?;

        if 0 == i % 4 {
            let w = writer(&d, mock_iwc())?;
            // LogMergePolicy lmp2 = (LogMergePolicy) writer.getConfig().getMergePolicy();
            // lmp2.setNoCFSRatio(0.0);
            w.force_merge(1)?;
            w.close()?;
        }
    }
    drop(d);
    dir.close()
}

/// LUCENE-1084: test unlimited field length
/// Port of `TestIndexWriter.testUnlimitedMaxFieldLength`.
fn test_unlimited_max_field_length() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let w = writer(&d, mock_iwc())?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        &(" a".repeat(10_000) + " x"),
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    let t = Term::from_text("field", "x");
    assert_equals(&1, &reader.doc_freq(&t)?);
    reader.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-1179
/// Port of `TestIndexWriter.testEmptyFieldName`.
fn test_empty_field_name() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "",
        "a b c",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.close()?;
    drop(d);
    dir.close()
}

/// `getOnlyLeafReader(reader).terms("").iterator()` with its terms checked
/// against `expected`, then `assertNull(te.next())`.
fn assert_empty_field_terms(reader: &Arc<dyn DirectoryReader>, expected: &[&str]) -> Result<()> {
    let subreader = LuceneTestCase::get_only_leaf_reader(ir(reader))?;
    let mut te = subreader
        .terms("")?
        .unwrap_or_else(|| npe("subreader.terms(\"\")"))
        .iterator()?;
    for term in expected {
        assert_equals(&Some(LuceneTestCase::new_bytes_ref_str(term)), &te.next()?);
    }
    assert_null(&te.next()?);
    Ok(())
}

/// Port of `TestIndexWriter.testEmptyFieldNameTerms`.
fn test_empty_field_name_terms() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "",
        "a b c",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.close()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_empty_field_terms(&reader, &["a", "b", "c"])?;
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testEmptyFieldNameWithEmptyTerm`.
fn test_empty_field_name_with_empty_term() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut doc = Document::new();
    for v in ["", "a", "b", "c"] {
        doc.add(Box::new(LuceneTestCase::new_string_field("", v, Store::NO)));
    }
    w.add_document(&doc)?;
    w.close()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_empty_field_terms(&reader, &["", "a", "b", "c"])?;
    reader.close()?;
    drop(d);
    dir.close()
}

/// `private static final class MockIndexWriter extends IndexWriter`: its
/// `doAfterFlush()` and `doBeforeFlush()` overrides.
#[derive(Default)]
struct MockIndexWriter {
    after_was_called: AtomicBool,
    before_was_called: AtomicBool,
}

impl IndexWriterOverrides for MockIndexWriter {
    fn do_after_flush(&self, _writer: &IndexWriter) -> Result<()> {
        self.after_was_called.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn do_before_flush(&self, _writer: &IndexWriter) -> Result<()> {
        self.before_was_called.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// LUCENE-1222
/// Port of `TestIndexWriter.testDoBeforeAfterFlush`.
fn test_do_before_after_flush() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mock = Arc::new(MockIndexWriter::default());
    w.set_overrides(Arc::clone(&mock) as Arc<dyn IndexWriterOverrides>);
    let mut doc = Document::new();
    let custom_type = FieldType::new_from(TextField::type_stored());
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "a field",
        &custom_type,
    )));
    w.add_document(&doc)?;
    w.commit()?;
    Assert::assert_true(mock.before_was_called.load(Ordering::SeqCst));
    Assert::assert_true(mock.after_was_called.load(Ordering::SeqCst));
    mock.before_was_called.store(false, Ordering::SeqCst);
    mock.after_was_called.store(false, Ordering::SeqCst);
    w.delete_documents_by_terms(vec![Term::from_text("field", "field")])?;
    w.commit()?;
    Assert::assert_true(mock.before_was_called.load(Ordering::SeqCst));
    Assert::assert_true(mock.after_was_called.load(Ordering::SeqCst));
    w.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &reader.num_docs());
    reader.close()?;

    drop(d);
    dir.close()
}

/// The anonymous `TokenStream` of `testNegativePositions`: the terms `a`,
/// `b` and `c`, the first with a position increment of 0.
#[derive(Debug)]
struct NegativePositionsTokenStream {
    source: AttributeSource,
    term_att: Att,
    pos_incr_att: Att,
    terms: std::vec::IntoIter<&'static str>,
    first: bool,
}

impl NegativePositionsTokenStream {
    fn new() -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let pos_incr_att = Att::add(&mut source, pos_inc_id())?;
        Ok(NegativePositionsTokenStream {
            source,
            term_att,
            pos_incr_att,
            terms: vec!["a", "b", "c"].into_iter(),
            first: true,
        })
    }
}

impl TokenStream for NegativePositionsTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        let Some(term) = self.terms.next() else {
            return Ok(false);
        };
        self.source.clear_attributes();
        self.term_att.char_term_mut().append_string(Some(term));
        self.pos_incr_att
            .pos_inc_mut()
            .set_position_increment(if self.first { 0 } else { 1 });
        self.first = false;
        Ok(true)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// LUCENE-1255
/// Port of `TestIndexWriter.testNegativePositions`.
fn test_negative_positions() -> Result<()> {
    let tokens: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(NegativePositionsTokenStream::new()?));

    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new_with_token_stream("field", tokens)?));
    expect_throws("java.lang.IllegalArgumentException", || {
        w.add_document(&doc)
    });

    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-2529
/// Port of `TestIndexWriter.testPositionIncrementGapEmptyField`.
fn test_position_increment_gap_empty_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_position_increment_gap(100);
    let w = writer(
        &d,
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer))),
    )?;
    let mut doc = Document::new();
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    let f = LuceneTestCase::new_field("field", "", &custom_type);
    let f2 = LuceneTestCase::new_field("field", "crunch man", &custom_type);
    doc.add(Box::new(f));
    doc.add(Box::new(f2));
    w.add_document(&doc)?;
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    let tpv = r
        .term_vectors()?
        .get(0)?
        .unwrap_or_else(|| npe("termVectors().get(0)"))
        .terms("field")?
        .unwrap_or_else(|| npe("tpv"));
    let mut terms_enum = tpv.iterator()?;
    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum.postings(None, POSTINGS_ENUM_ALL)?;
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_equals(&1, &dp_enum.freq()?);
    assert_equals(&100, &dp_enum.next_position()?);

    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum.postings(Some(dp_enum), POSTINGS_ENUM_ALL)?;
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_equals(&1, &dp_enum.freq()?);
    assert_equals(&101, &dp_enum.next_position()?);
    assert_null(&terms_enum.next()?);

    r.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testDeadlock`.
fn test_deadlock() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    let w = writer(&d, iwc)?;
    let mut doc = Document::new();

    let custom_type = tv_type()?;

    doc.add(Box::new(LuceneTestCase::new_field(
        "content",
        "aaa bbb ccc ddd eee fff ggg hhh iii",
        &custom_type,
    )));
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    w.add_document(&doc)?;
    w.commit()?;
    // index has 2 segments

    let mut dir2 = LuceneTestCase::new_directory();
    let d2 = dir2.as_directory();
    let writer2 = writer(&d2, mock_iwc())?;
    writer2.add_document(&doc)?;
    writer2.close()?;

    let r1 = directory_reader::open(Arc::clone(&d2))?;
    TestUtil::add_indexes_slowly(&w, &[Arc::clone(&r1), Arc::clone(&r1)])?;
    w.close()?;

    let r3 = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&5, &r3.num_docs());
    r3.close()?;

    r1.close()?;

    drop(d2);
    dir2.close()?;
    drop(d);
    dir.close()
}

/// The `volatile` fields `IndexerThreadInterrupt` shares with the test
/// thread, and its `bytesLog`.
#[derive(Default)]
struct IndexerThreadInterruptState {
    failed: AtomicBool,
    finish: AtomicBool,
    allow_interrupt: AtomicBool,
    bytes_log: Mutex<String>,
}

impl IndexerThreadInterruptState {
    /// `log.println(s)`.
    fn log(&self, s: &str) {
        let mut log = self.bytes_log.lock().unwrap_or_else(|e| e.into_inner());
        log.push_str(s);
        log.push('\n');
    }
}

/// `private static class IndexerThreadInterrupt extends Thread`: the
/// thread's state lives in [`IndexerThreadInterruptState`], its body in
/// [`IndexerThreadInterrupt::run`].
struct IndexerThreadInterrupt {
    state: Arc<IndexerThreadInterruptState>,
    random: JdkRandom,
    adder: MockDirectoryWrapper,
    id: i32,
}

/// `isOK(th)` of the `SuppressingConcurrentMergeScheduler` the constructor
/// installs: `AlreadyClosedException`, or an `IllegalStateException` saying
/// "this writer hit an unrecoverable error".
fn is_closed_or_unrecoverable(e: &LuceneError) -> bool {
    is_instance(e, "org.apache.lucene.store.AlreadyClosedException")
        || (is_instance(e, "java.lang.IllegalStateException")
            && java_throwable(e)
                .message()
                .unwrap_or_default()
                .contains("this writer hit an unrecoverable error"))
}

impl IndexerThreadInterrupt {
    fn new(id: i32) -> Result<Self> {
        let random = JdkRandom::new(random().next_long());
        // make a little directory for addIndexes
        // LUCENE-2239: won't work with NIOFS/MMAP
        let adder = MockDirectoryWrapper::new(&random, Arc::new(ByteBuffersDirectory::new()));
        let conf = LuceneTestCase::new_index_writer_config_with(
            &random,
            Some(Arc::new(MockAnalyzer::new(&random))),
        );
        if is_cms(&conf) {
            conf.set_merge_scheduler(Some(Arc::new(SuppressingConcurrentMergeScheduler::create(
                Arc::new(|th| match th.downcast_ref::<LuceneError>() {
                    Some(e) => is_closed_or_unrecoverable(e),
                    None => false,
                }),
            ))))?;
        }
        let w = IndexWriter::new(adder.as_directory(), conf)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field_with(
            &random,
            "id",
            "500",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_field_with(
            &random,
            "field",
            NewFieldValue::String("some prepackaged text contents".to_string()),
            &stored_text_type(),
        )));
        doc.add(Box::new(BinaryDocValuesField::new(
            "binarydv",
            LuceneTestCase::new_bytes_ref_str("500"),
        )));
        doc.add(Box::new(NumericDocValuesField::new("numericdv", 500)));
        doc.add(Box::new(SortedDocValuesField::new(
            "sorteddv",
            LuceneTestCase::new_bytes_ref_str("500"),
        )));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "sortedsetdv",
            LuceneTestCase::new_bytes_ref_str("one"),
        )));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "sortedsetdv",
            LuceneTestCase::new_bytes_ref_str("two"),
        )));
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "sortednumericdv",
            4,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "sortednumericdv",
            3,
        )));
        w.add_document(&doc)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field_with(
            &random,
            "id",
            "501",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_field_with(
            &random,
            "field",
            NewFieldValue::String("some more contents".to_string()),
            &stored_text_type(),
        )));
        doc.add(Box::new(BinaryDocValuesField::new(
            "binarydv",
            LuceneTestCase::new_bytes_ref_str("501"),
        )));
        doc.add(Box::new(NumericDocValuesField::new("numericdv", 501)));
        doc.add(Box::new(SortedDocValuesField::new(
            "sorteddv",
            LuceneTestCase::new_bytes_ref_str("501"),
        )));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "sortedsetdv",
            LuceneTestCase::new_bytes_ref_str("two"),
        )));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "sortedsetdv",
            LuceneTestCase::new_bytes_ref_str("three"),
        )));
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "sortednumericdv",
            6,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "sortednumericdv",
            1,
        )));
        w.add_document(&doc)?;
        w.delete_documents_by_terms(vec![Term::from_text("id", "500")])?;
        w.close()?;
        Ok(IndexerThreadInterrupt {
            state: Arc::new(IndexerThreadInterruptState::default()),
            random,
            adder,
            id,
        })
    }

    /// The inner `while (!finish)` loop of `run()`, which leaves `w` open
    /// when an exception interrupts it.
    fn index_loop(
        &self,
        dir: &MockDirectoryWrapper,
        w: &mut Option<Arc<IndexWriter>>,
    ) -> Result<()> {
        let random = &self.random;
        let finish = &self.state.finish;
        while !finish.load(Ordering::SeqCst) {
            if let Some(old) = w.as_ref() {
                // If interrupt arrives inside here, it's fine: we will cycle back
                // and the first thing we do is try to close again,
                // i.e. we'll never try to open a new writer until this one successfully closes:
                // w.rollback();
                match old.close() {
                    Ok(()) => {}
                    Err(e) if is_instance(&e, "org.apache.lucene.store.AlreadyClosedException") => {
                        // OK
                    }
                    Err(e) => return Err(e),
                }
                *w = None;
            }
            let conf = LuceneTestCase::new_index_writer_config_with(
                random,
                Some(Arc::new(MockAnalyzer::new(random))),
            );
            conf.set_max_buffered_docs(2)?;
            if is_cms(&conf) {
                conf.set_merge_scheduler(Some(Arc::new(
                    SuppressingConcurrentMergeScheduler::create(Arc::new(|th| {
                        match th.downcast_ref::<LuceneError>() {
                            Some(e) => {
                                is_instance(e, "org.apache.lucene.store.AlreadyClosedException")
                                    || is_instance(
                                        e,
                                        "java.util.concurrent.RejectedExecutionException",
                                    )
                                    // Interrupt-driven rollback can race with a merge thread's _mergeInit.
                                    || is_instance(e, "java.lang.AssertionError")
                                    || (is_instance(e, "java.lang.IllegalStateException")
                                        && java_throwable(e)
                                            .message()
                                            .unwrap_or_default()
                                            .contains("this writer hit an unrecoverable error"))
                            }
                            None => false,
                        }
                    })),
                )))?;
            }
            // conf.setInfoStream(log);
            let writer = IndexWriter::new(dir.as_directory(), conf)?;
            *w = Some(Arc::clone(&writer));

            let mut doc = Document::new();
            let id_field = LuceneTestCase::new_string_field_with(random, "id", "", Store::NO);
            let binary_dv_field =
                BinaryDocValuesField::new("binarydv", LuceneTestCase::new_bytes_ref());
            let numeric_dv_field = NumericDocValuesField::new("numericdv", 0);
            let sorted_dv_field =
                SortedDocValuesField::new("sorteddv", LuceneTestCase::new_bytes_ref());
            let sorted_set_dv_field =
                SortedSetDocValuesField::new("sortedsetdv", LuceneTestCase::new_bytes_ref());
            doc.add(Box::new(id_field));
            doc.add(Box::new(LuceneTestCase::new_field_with(
                random,
                "field",
                NewFieldValue::String("some text contents".to_string()),
                &stored_text_type(),
            )));
            doc.add(Box::new(binary_dv_field));
            doc.add(Box::new(numeric_dv_field));
            doc.add(Box::new(sorted_dv_field));
            doc.add(Box::new(sorted_set_dv_field));
            for i in 0..100 {
                // log.println("\nTEST: i=" + i);
                field_mut(&mut doc, "id").set_string_value(i.to_string())?;
                let id = field_mut(&mut doc, "id").string_value().unwrap_or_default();
                doc.get_field_mut_as::<BinaryDocValuesField>("binarydv")
                    .expect("binaryDVField")
                    .set_bytes_value(LuceneTestCase::new_bytes_ref_str(&id))?;
                doc.get_field_mut_as::<NumericDocValuesField>("numericdv")
                    .expect("numericDVField")
                    .set_value(i64::from(i));
                doc.get_field_mut_as::<SortedDocValuesField>("sorteddv")
                    .expect("sortedDVField")
                    .set_bytes_value(LuceneTestCase::new_bytes_ref_str(&id))?;
                doc.get_field_mut_as::<SortedSetDocValuesField>("sortedsetdv")
                    .expect("sortedSetDVField")
                    .set_bytes_value(LuceneTestCase::new_bytes_ref_str(&id))?;
                let action = random.next_int_bound(100);
                if action == 17 {
                    writer.add_indexes_dirs(&[self.adder.as_directory()])?;
                } else if action % 30 == 0 {
                    writer.delete_all()?;
                } else if action % 2 == 0 {
                    writer.update_document(Term::from_text("id", &id), &doc)?;
                } else {
                    writer.add_document(&doc)?;
                }
                if random.next_int_bound(3) == 0 {
                    let mut r: Option<Arc<dyn DirectoryReader>> = None;
                    let result = (|| -> Result<()> {
                        let opened = directory_reader::open_with_writer_flags(
                            &*writer,
                            random.next_boolean(),
                            false,
                        )?;
                        r = Some(Arc::clone(&opened));
                        if random.next_boolean() && opened.max_doc() > 0 {
                            let docid = random.next_int_bound(opened.max_doc());
                            writer.try_delete_document(&ir(&opened), docid)?;
                        }
                        Ok(())
                    })();
                    if let Some(r) = r {
                        let _ = r.close();
                    }
                    result?;
                }
                if i % 10 == 0 {
                    writer.commit()?;
                }
                if random.next_int_bound(50) == 0 {
                    writer.force_merge(1)?;
                }
            }
            writer.close()?;
            *w = None;
            directory_reader::open(dir.as_directory())?.close()?;

            // Strangely, if we interrupt a thread before all classes are loaded,
            // the class loader seems to do scary things with the interrupt status.
            // In java 1.5, it'll throw an incorrect ClassNotFoundException.
            // In java 1.6, it'll silently clear the interrupt.
            // So, on first iteration through here we don't open ourselves up for interrupts
            // until we've done the above loop.
            self.state.allow_interrupt.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// `run()`.
    fn run(mut self) {
        let state = Arc::clone(&self.state);
        let id = self.id;
        // LUCENE-2239: won't work with NIOFS/MMAP
        let mut dir =
            MockDirectoryWrapper::new(&self.random, Arc::new(ByteBuffersDirectory::new()));

        // open/close slowly sometimes
        dir.set_use_slow_open_closers(true);

        // throttle a little
        dir.set_throttling(Throttling::Sometimes);

        let mut w: Option<Arc<IndexWriter>> = None;
        while !state.finish.load(Ordering::SeqCst) {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.index_loop(&dir, &mut w)
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(LuceneError::ThreadInterrupted(re))) => {
                    // NOTE: important to leave this verbosity/noise on!!
                    // This test doesn't repro easily so when Jenkins hits a fail
                    // we need to study where the interrupts struck!
                    state.log(&format!("TEST thread {id}: got interrupt"));
                    state.log(&format!("{re:?}"));
                    let e = std::error::Error::source(&re);
                    Assert::assert_true(e.is_some_and(|e| e.is::<InterruptedException>()));
                    if state.finish.load(Ordering::SeqCst) {
                        break;
                    }
                }
                Ok(Err(t)) => {
                    state.log(&format!("thread {id} FAILED; unexpected exception"));
                    state.log(&format!("{t:?}"));
                    list_index_files(&state, &dir);
                    state.failed.store(true, Ordering::SeqCst);
                    break;
                }
                Err(payload) => {
                    state.log(&format!("thread {id} FAILED; unexpected exception"));
                    state.log(&format!("{:?}", Throwable::from_panic(payload)));
                    list_index_files(&state, &dir);
                    state.failed.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }

        if verbose() {
            state.log(&format!(
                "TEST: thread {id}: now finish failed={}",
                state.failed.load(Ordering::SeqCst)
            ));
        }
        if !state.failed.load(Ordering::SeqCst) {
            if verbose() {
                state.log(&format!("TEST: thread {id}: now rollback"));
            }
            // clear interrupt state:
            Thread::interrupted();
            if let Some(w) = w.take() {
                if let Err(ioe) = w.rollback() {
                    rethrow_java(LuceneError::runtime_exception(ioe));
                }
            }

            if let Err(e) = TestUtil::check_index(dir.as_directory()) {
                state.failed.store(true, Ordering::SeqCst);
                state.log(&format!(
                    "thread {id}: CheckIndex FAILED: unexpected exception"
                ));
                state.log(&format!("{e:?}"));
                list_index_files(&state, &dir);
            }
            match directory_reader::open(dir.as_directory()) {
                // System.out.println("doc count=" + r.numDocs());
                Ok(r) => {
                    if let Err(e) = r.close() {
                        state.failed.store(true, Ordering::SeqCst);
                        state.log(&format!(
                            "thread {id}: DirectoryReader.open FAILED: unexpected exception"
                        ));
                        state.log(&format!("{e:?}"));
                        list_index_files(&state, &dir);
                    }
                }
                Err(e) => {
                    state.failed.store(true, Ordering::SeqCst);
                    state.log(&format!(
                        "thread {id}: DirectoryReader.open FAILED: unexpected exception"
                    ));
                    state.log(&format!("{e:?}"));
                    list_index_files(&state, &dir);
                }
            }
        }
        drop(w);
        if let Err(e) = dir.close() {
            state.failed.store(true, Ordering::SeqCst);
            rethrow_java(runtime_exception_msg(&format!("thread {id}"), e));
        }
        if let Err(e) = self.adder.close() {
            state.failed.store(true, Ordering::SeqCst);
            rethrow_java(runtime_exception_msg(&format!("thread {id}"), e));
        }
    }
}

/// `IndexerThreadInterrupt.listIndexFiles(log, dir)`.
fn list_index_files(state: &IndexerThreadInterruptState, dir: &MockDirectoryWrapper) {
    match dir.as_directory().list_all() {
        Ok(files) => state.log(&format!("index files: [{}]", files.join(", "))),
        Err(ioe) => {
            // Suppress
            state.log("failed to index files:");
            state.log(&format!("{ioe:?}"));
        }
    }
}

/// Port of `TestIndexWriter.testThreadInterruptDeadlock`.
fn test_thread_interrupt_deadlock() -> Result<()> {
    let t = IndexerThreadInterrupt::new(1)?;
    let state = Arc::clone(&t.state);
    let t = Thread::unnamed(move || t.run());
    t.set_daemon(true);
    t.start();

    // Force class loader to load ThreadInterruptedException up front...
    // else we can see a false failure if 2nd interrupt arrives while class loader is trying to
    // init this class (in servicing a first interrupt):
    Assert::assert_true(
        std::error::Error::source(&ThreadInterruptedException::new(InterruptedException))
            .is_some_and(|e| e.is::<InterruptedException>()),
    );

    // issue 100 interrupts to child thread
    let num_interrupts = LuceneTestCase::at_least(100);
    let mut i = 0;
    while i < num_interrupts {
        // TODO: would be nice to also sometimes interrupt the CMS merge threads too ...
        Thread::sleep(10);
        if state.allow_interrupt.load(Ordering::SeqCst) {
            i += 1;
            t.interrupt();
        }
        if !t.is_alive() {
            break;
        }
    }
    state.finish.store(true, Ordering::SeqCst);
    if let Err(payload) = t.join() {
        std::panic::resume_unwind(payload);
    }
    if state.failed.load(Ordering::SeqCst) {
        Assert::fail(&state.bytes_log.lock().unwrap_or_else(|e| e.into_inner()));
    }
    Ok(())
}

/// `new RuntimeException(message, cause)`.
fn runtime_exception_msg(message: &str, cause: LuceneError) -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.RuntimeException",
        message: Some(message.to_string()),
        cause: Some(Arc::new(cause)),
    }
}

/// The anonymous `Field` subclasses of `testIndexStoreCombos`: a field whose
/// `tokenStream(analyzer, reuse)` returns a shared `MockTokenizer`.
struct TokenizerField {
    field: Field,
    tokenizer: Arc<AtomicRefCell<MockTokenizer>>,
}

impl std::fmt::Display for TokenizerField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.field, f)
    }
}

impl IndexableField for TokenizerField {
    fn name(&self) -> &str {
        self.field.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.field.field_type()
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(shared(&self.tokenizer))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.field.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.field.string_value()
    }

    fn reader_value(&self) -> Option<SharedReader> {
        self.field.reader_value()
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.field.numeric_value()
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        self.field.stored_value()
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        self.field.invertable_type()
    }
}

/// `TestUtil.docs(random(), ir, field, newBytesRef(term), null,
/// PostingsEnum.NONE).nextDoc() != DocIdSetIterator.NO_MORE_DOCS`.
fn has_doc(reader: &Arc<dyn IndexReader>, field: &str, term: &str) -> Result<bool> {
    let mut docs = TestUtil::docs(
        &*random(),
        reader,
        field,
        &LuceneTestCase::new_bytes_ref_str(term),
        None,
        POSTINGS_ENUM_NONE,
    )?
    .unwrap_or_else(|| npe("TestUtil.docs"));
    Ok(docs.next_doc()? != NO_MORE_DOCS)
}

/// Port of `TestIndexWriter.testIndexStoreCombos`.
fn test_index_store_combos() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut b = vec![0u8; 50];
    for (i, v) in b.iter_mut().enumerate() {
        *v = (i + 77) as u8;
    }

    let mut doc = Document::new();

    let custom_type = Arc::new(FieldType::new_from(StoredField::type_()));
    custom_type.set_tokenized(true)?;

    let field1 = Arc::new(AtomicRefCell::new(MockTokenizer::new(
        Arc::clone(&WHITESPACE),
        false,
    )?));
    let f = Field::new_with_bytes(
        "binary",
        BytesRef {
            bytes: b.clone(),
            offset: 10,
            length: 17,
        },
        Arc::clone(&custom_type),
    )?;
    let f = TokenizerField {
        field: f,
        tokenizer: Arc::clone(&field1),
    };
    // TODO: this is evil, changing the type after creating the field:
    custom_type.set_index_options(IndexOptions::DOCS)?;
    field1
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new("doc1field1")))?;

    let custom_type2 = FieldType::new_from(TextField::type_stored());

    let field2 = Arc::new(AtomicRefCell::new(MockTokenizer::new(
        Arc::clone(&WHITESPACE),
        false,
    )?));
    let f2 = TokenizerField {
        field: Field::new("string", "value".to_string(), custom_type2)?,
        tokenizer: Arc::clone(&field2),
    };

    field2
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new("doc1field2")))?;
    doc.add(Box::new(f));
    doc.add(Box::new(f2));
    w.add_document(&doc)?;

    // add 2 docs to test in-memory merging
    field1
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new("doc2field1")))?;
    field2
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new("doc2field2")))?;
    w.add_document(&doc)?;

    // force segment flush so we can force a segment merge with doc3 later.
    w.commit()?;

    field1
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new("doc3field1")))?;
    field2
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new("doc3field2")))?;

    w.add_document(&doc)?;
    w.commit()?;
    w.force_merge(1)?; // force segment merge.
    w.close()?;

    let reader = directory_reader::open(Arc::clone(&d))?;
    let stored_fields = reader.stored_fields()?;
    let doc2 = stored_fields.document(0)?;
    let f3 = doc2
        .get_field("binary")
        .unwrap_or_else(|| npe("doc2.getField(\"binary\")"));
    let b = f3
        .binary_value()
        .unwrap_or_else(|| npe("f3.binaryValue()"))
        .bytes;
    assert_equals(&17, &b.len());
    assert_equals(&87, &b[0]);

    for i in 0..3 {
        assert_not_null(
            &stored_fields
                .document(i)?
                .get_field("binary")
                .unwrap_or_else(|| npe("getField(\"binary\")"))
                .binary_value(),
        );
    }

    for i in 0..3 {
        assert_equals(
            &Some("value".to_string()),
            &stored_fields.document(i)?.get("string"),
        );
    }

    // test that the terms were indexed.
    let r = ir(&reader);
    Assert::assert_true(has_doc(&r, "binary", "doc1field1")?);
    Assert::assert_true(has_doc(&r, "binary", "doc2field1")?);
    Assert::assert_true(has_doc(&r, "binary", "doc3field1")?);
    Assert::assert_true(has_doc(&r, "string", "doc1field2")?);
    Assert::assert_true(has_doc(&r, "string", "doc2field2")?);
    Assert::assert_true(has_doc(&r, "string", "doc3field2")?);

    drop(r);
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testNoDocsIndex`.
fn test_no_docs_index() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    w.add_document(&Document::new())?;
    w.close()?;

    drop(d);
    dir.close()
}

/// `Codec.getDefault() instanceof SimpleTextCodec`: the default codec's
/// name is `SimpleTextCodec`'s.
fn default_codec_is_simple_text() -> bool {
    rucene::codecs::get_default().is_some_and(|c| c.name() == "SimpleText")
}

/// `assertNotSame(a, b)` over two readers.
fn assert_not_same(a: &Arc<dyn DirectoryReader>, b: &Arc<dyn DirectoryReader>) {
    if std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b)) {
        Assert::fail("expected not same");
    }
}

/// Port of `TestIndexWriter.testDeleteUnusedFiles`.
fn test_delete_unused_files() -> Result<()> {
    LuceneTestCase::assume_false(
        "test relies on exact filenames",
        default_codec_is_simple_text(),
    );

    for iter in 0..2 {
        // relies on Windows semantics
        let path = LuceneTestCase::create_temp_dir();
        let provider = WindowsFS::new(Arc::clone(path.get_file_system()));
        let index_path = provider.wrap_path(&path);

        // NOTE: We cannot use MMapDir, because WindowsFS doesn't see/think it keeps file
        // handles open.
        let dir = Arc::new(NIOFSDirectory::open(index_path.clone())?);
        let d: Arc<dyn Directory> = Arc::clone(&dir) as Arc<dyn Directory>;

        let merge_policy = LuceneTestCase::new_log_merge_policy_use_cfs(true);

        // This test expects all of its segments to be in CFS
        merge_policy.set_no_cfs_ratio(1.0)?;
        merge_policy.set_max_cfs_segment_size_mb(f64::INFINITY)?;

        let iwc = mock_iwc();
        iwc.set_merge_policy(Some(merge_policy))?;
        iwc.set_use_compound_file(true)?;
        let w = writer(&d, iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            "go",
            Store::NO,
        )));
        w.add_document(&doc)?;
        let r = if iter == 0 {
            // use NRT
            directory_reader::open_with_writer(&*w)?
        } else {
            // don't use NRT
            w.commit()?;
            directory_reader::open(Arc::clone(&d))?
        };

        let exists = |name: &str| Files::exists(&index_path.resolve_str(name), &[]);
        Assert::assert_true(exists("_0.cfs"));
        Assert::assert_true(exists("_0.cfe"));
        Assert::assert_true(exists("_0.si"));
        if iter == 1 {
            // we run a full commit so there should be a segments file etc.
            Assert::assert_true(exists("segments_1"));
        } else {
            // this is an NRT reopen - no segments files yet
            Assert::assert_false(exists("segments_1"));
        }
        w.add_document(&doc)?;
        w.force_merge(1)?;
        if iter == 1 {
            w.commit()?;
        }
        let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
        assert_not_null(&r2);
        let r2 = r2.unwrap_or_else(|| npe("r2"));
        assert_not_same(&r, &r2);

        // NOTE: here we rely on "Windows" behavior, i.e. even though IW wanted to delete _0.cfs
        // since it was merged away, because we have a reader open against this file,
        // it should still be here:
        Assert::assert_true(exists("_0.cfs"));
        // forceMerge created this
        // assertTrue(files.contains("_2.cfs"));
        w.delete_unused_files()?;

        // r still holds this file open
        Assert::assert_true(exists("_0.cfs"));
        // assertTrue(files.contains("_2.cfs"));

        r.close()?;
        if iter == 0 {
            // on closing NRT reader, it calls writer.deleteUnusedFiles
            Assert::assert_false(exists("_0.cfs"));
        } else {
            // now FSDir can remove it
            dir.delete_pending_files()?;
            Assert::assert_false(exists("_0.cfs"));
        }

        w.close()?;
        r2.close()?;

        drop((w, r, r2, d));
        close_shared(dir)?;
    }
    Ok(())
}

/// Port of `TestIndexWriter.testDeleteUnusedFiles2`.
fn test_delete_unused_files2() -> Result<()> {
    // Validates that iw.deleteUnusedFiles() also deletes unused index commits
    // in case a deletion policy which holds onto commits is used.
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let sdp = Arc::new(SnapshotDeletionPolicy::new(Some(Arc::new(
        KeepOnlyLastCommitDeletionPolicy,
    ))));
    let iwc = mock_iwc();
    iwc.set_index_deletion_policy(Some(Arc::clone(&sdp) as _))?;
    let w = writer(&d, iwc)?;
    // `(SnapshotDeletionPolicy) writer.getConfig().getIndexDeletionPolicy()`:
    // the policy installed above.

    // First commit
    let mut doc = Document::new();

    let custom_type = tv_type()?;

    doc.add(Box::new(LuceneTestCase::new_field(
        "c",
        "val",
        &custom_type,
    )));
    w.add_document(&doc)?;
    w.commit()?;
    assert_equals(&1, &directory_reader::list_commits(Arc::clone(&d))?.len());

    // Keep that commit
    let id = sdp.snapshot()?;

    // Second commit - now KeepOnlyLastCommit cannot delete the prev commit.
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "c",
        "val",
        &custom_type,
    )));
    w.add_document(&doc)?;
    w.commit()?;
    assert_equals(&2, &directory_reader::list_commits(Arc::clone(&d))?.len());

    // Should delete the unreferenced commit
    sdp.release(&*id)?;
    w.delete_unused_files()?;
    assert_equals(&1, &directory_reader::list_commits(Arc::clone(&d))?.len());

    w.close()?;
    drop((id, d));
    dir.close()
}

/// Port of `TestIndexWriter.testEmptyFSDirWithNoLock`.
fn test_empty_fs_dir_with_no_lock() -> Result<()> {
    // Tests that if FSDir is opened w/ a NoLockFactory (or SingleInstanceLF),
    // then IndexWriter ctor succeeds. Previously (LUCENE-2386) it failed
    // when listAll() was called in IndexFileDeleter.
    let mut dir = LuceneTestCase::new_fs_directory_lock_factory(
        &LuceneTestCase::create_temp_dir_prefix("emptyFSDirNoLock"),
        Arc::new(NoLockFactory),
    );
    let d = dir.as_directory();
    writer(&d, mock_iwc())?.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testEmptyDirRollback`.
fn test_empty_dir_rollback() -> Result<()> {
    // TODO: generalize this test
    LuceneTestCase::assume_false(
        "test makes assumptions about file counts",
        default_codec_is_simple_text(),
    );
    // Tests that if IW is created over an empty Directory, some documents are
    // indexed, flushed (but not committed) and then IW rolls back, then no
    // files are left in the Directory.
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let mut orig_files = d.list_all()?;
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    iwc.set_use_compound_file(false)?;
    let w = writer(&d, iwc)?;
    let mut files = d.list_all()?;

    // Creating over empty dir should not create any files, or, at most the write.lock file
    let extra_file_count = files.len() as i64 - orig_files.len() as i64;
    if extra_file_count == 1 {
        Assert::assert_true(files.iter().any(|f| f == WRITE_LOCK_NAME));
    } else {
        orig_files.sort();
        files.sort();
        assert_array_equals(&orig_files, &files);
    }

    let mut doc = Document::new();
    let custom_type = tv_type()?;
    // create as many files as possible
    doc.add(Box::new(LuceneTestCase::new_field(
        "c",
        "val",
        &custom_type,
    )));
    w.add_document(&doc)?;
    // Adding just one document does not call flush yet.
    let mut computed_extra_file_count = 0i64;
    for file in d.list_all()? {
        if file == WRITE_LOCK_NAME
            || file.starts_with(index_file_names::SEGMENTS)
            || index_file_names::is_codec_file(&file)
        {
            match file.rfind('.') {
                None => computed_extra_file_count += 1,
                Some(dot) => {
                    // don't count stored fields and term vectors in, or any temporary files they might
                    if !["fdm", "fdt", "tvm", "tvd", "tmp"].contains(&&file[dot + 1..]) {
                        computed_extra_file_count += 1;
                    }
                }
            }
        }
    }
    assert_equals_msg(
        Some("only the stored and term vector files should exist in the directory"),
        &extra_file_count,
        &computed_extra_file_count,
    );

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "c",
        "val",
        &custom_type,
    )));
    w.add_document(&doc)?;

    // The second document should cause a flush.
    Assert::assert_true_msg(
        "flush should have occurred and files should have been created",
        d.list_all()?.len() as i64 > 5 + extra_file_count,
    );

    // After rollback, IW should remove all files
    w.rollback()?;
    let all_files = d.list_all()?;
    assert_equals_msg(
        Some("no files should exist in the directory after rollback"),
        &(orig_files.len() as i64 + extra_file_count),
        &(all_files.len() as i64),
    );

    // Since we rolled-back above, that close should be a no-op
    w.close()?;
    let all_files = d.list_all()?;
    assert_equals_msg(
        Some("expected a no-op close after IW.rollback()"),
        &(orig_files.len() as i64 + extra_file_count),
        &(all_files.len() as i64),
    );
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testNoUnwantedTVFiles`.
fn test_no_unwanted_tv_files() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    iwc.set_ram_buffer_size_mb(0.01)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let index_writer = writer(&d, iwc)?;
    index_writer
        .get_config()
        .merge_policy()
        .set_no_cfs_ratio(0.0)?;

    let big =
        "alskjhlaksjghlaksjfhalksvjepgjioefgjnsdfjgefgjhelkgjhqewlrkhgwlekgrhwelkgjhwelkgrhwlkejg";
    let big = big.repeat(4);

    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_omit_norms(true)?;
    let custom_type2 = FieldType::new_from(TextField::type_stored());
    custom_type2.set_tokenized(false)?;
    let custom_type3 = FieldType::new_from(TextField::type_stored());
    custom_type3.set_tokenized(false)?;
    custom_type3.set_omit_norms(true)?;

    for i in 0..2 {
        let mut doc = Document::new();
        doc.add(Box::new(Field::new(
            "id",
            format!("{i}{big}"),
            custom_type3.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "str",
            format!("{i}{big}"),
            custom_type2.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "str2",
            format!("{i}{big}"),
            stored_text_type(),
        )?));
        doc.add(Box::new(Field::new(
            "str3",
            format!("{i}{big}"),
            custom_type.clone(),
        )?));
        index_writer.add_document(&doc)?;
    }

    index_writer.close()?;

    TestUtil::check_index(Arc::clone(&d))?;

    assert_no_unreferenced_files(&d, "no tv files")?;
    let r0 = directory_reader::open(Arc::clone(&d))?;
    for ctx in ir(&r0).leaves()? {
        let leaf = ctx.leaf_reader();
        let sr = leaf.as_segment_reader().unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
        });
        Assert::assert_false(sr.get_field_infos().has_term_vectors());
    }

    r0.close()?;
    drop(d);
    dir.close()
}

/// `static final class StringSplitAnalyzer extends Analyzer`.
#[derive(Debug)]
struct StringSplitAnalyzer {
    state: AnalyzerState,
}

impl StringSplitAnalyzer {
    fn new() -> Self {
        StringSplitAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Analyzer for StringSplitAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriter$StringSplitAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let t = Tokenizer::with_source(
            source,
            StringSplitTokenizer {
                tokens: Vec::new(),
                upto: 0,
                term_att,
            },
        );
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(t))))
    }
}

/// `private static class StringSplitTokenizer extends Tokenizer`.
#[derive(Debug)]
struct StringSplitTokenizer {
    tokens: Vec<String>,
    upto: usize,
    term_att: Att,
}

/// Java's `s.split(" ")`: the pieces between single spaces, with the
/// trailing empty strings removed (the whole string when there is no space).
fn java_split_space(s: &str) -> Vec<String> {
    if !s.contains(' ') {
        return vec![s.to_string()];
    }
    let mut parts: Vec<String> = s.split(' ').map(str::to_string).collect();
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

impl TokenizerLogic for StringSplitTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _reader: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        if self.upto < self.tokens.len() {
            let mut term = self.term_att.char_term_mut();
            term.set_empty();
            term.append_string(Some(&self.tokens[self.upto]));
            self.upto += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn CharReader) -> Result<()> {
        self.upto = 0;
        let mut b: Vec<u16> = Vec::new();
        let mut buffer = [0u16; 1024];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            b.extend_from_slice(&buffer[..n]);
        }
        self.tokens = java_split_space(&String::from_utf16_lossy(&b));
        Ok(())
    }
}

/// Make sure we skip wicked long terms.
/// Port of `TestIndexWriter.testWickedLongTerm`.
fn test_wicked_long_term() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = RandomIndexWriter::new(
        &*random(),
        Arc::clone(&d),
        Arc::new(StringSplitAnalyzer::new()),
    )?;

    let big_term = "x".repeat(MAX_TERM_LENGTH);
    let mut huge_doc = Document::new();

    // This contents produces a too-long term:
    let contents = format!("abc xyz x{big_term} another term");
    huge_doc.add(Box::new(TextField::new("content", contents, Store::NO)?));
    expect_throws("java.lang.IllegalArgumentException", || {
        w.add_document(&huge_doc)
    });

    // Make sure we can add another normal document
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "content",
        "abc bbb ccc".to_string(),
        Store::NO,
    )?));
    w.add_document(&doc)?;

    // So we remove the deleted doc:
    w.force_merge(1)?;

    let reader = w.get_reader()?;
    w.close()?;

    // Make sure all terms < max size were indexed
    assert_equals(&1, &reader.doc_freq(&Term::from_text("content", "abc"))?);
    assert_equals(&1, &reader.doc_freq(&Term::from_text("content", "bbb"))?);
    assert_equals(&0, &reader.doc_freq(&Term::from_text("content", "term"))?);

    // Make sure the doc that has the massive term is NOT in the index:
    assert_equals_msg(
        Some("document with wicked long term is in the index!"),
        &1,
        &reader.num_docs(),
    );

    reader.close()?;
    drop((w, d));
    dir.close()?;
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    // Make sure we can add a document with exactly the maximum length term,
    // and search on that term:
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_tokenized(false)?;
    let mut doc = Document::new();
    let content_field = Field::new("content", String::new(), custom_type)?;
    doc.add(Box::new(content_field));

    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_codec(TestUtil::get_default_codec())?;

    let w2 = RandomIndexWriter::new_with_config(&*random(), Arc::clone(&d), iwc)?;

    field_mut(&mut doc, "content").set_string_value("other".to_string())?;
    w2.add_document(&doc)?;

    field_mut(&mut doc, "content").set_string_value("term".to_string())?;
    w2.add_document(&doc)?;

    field_mut(&mut doc, "content").set_string_value(big_term.clone())?;
    w2.add_document(&doc)?;

    field_mut(&mut doc, "content").set_string_value("zzz".to_string())?;
    w2.add_document(&doc)?;

    let reader = w2.get_reader()?;
    w2.close()?;
    assert_equals(
        &1,
        &reader.doc_freq(&Term::from_text("content", &big_term))?,
    );

    reader.close()?;
    drop((w2, d));
    dir.close()
}

/// Port of `TestIndexWriter.testDeleteAllNRTLeftoverFiles`.
fn test_delete_all_nrt_leftover_files() -> Result<()> {
    let mut d = MockDirectoryWrapper::new(&*random(), Arc::new(ByteBuffersDirectory::new()));
    let dd = d.as_directory();
    let w = writer(
        &dd,
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )?;
    let doc = Document::new();
    for _ in 0..20 {
        for _ in 0..100 {
            w.add_document(&doc)?;
        }
        w.commit()?;
        directory_reader::open_with_writer(&*w)?.close()?;

        w.delete_all()?;
        w.commit()?;
        // Make sure we accumulate no files except for empty segments_N and segments.gen:
        Assert::assert_true(dd.list_all()?.len() <= 2);
    }

    w.close()?;
    drop(dd);
    d.close()
}

/// Port of `TestIndexWriter.testNRTReaderVersion`.
fn test_nrt_reader_version() -> Result<()> {
    let mut dir = MockDirectoryWrapper::new(&*random(), Arc::new(ByteBuffersDirectory::new()));
    let d = dir.as_directory();
    let w = writer(
        &d,
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "0",
        Store::YES,
    )));
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let version = r.version()?;
    r.close()?;

    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let version2 = r.version()?;
    r.close()?;
    debug_assert!(version2 > version);

    w.delete_documents_by_terms(vec![Term::from_text("id", "0")])?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let version3 = r.version()?;
    r.close()?;
    debug_assert!(version3 > version2);
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testWhetherDeleteAllDeletesWriteLock`.
fn test_whether_delete_all_deletes_write_lock() -> Result<()> {
    // Must use SimpleFSLockFactory...
    // NativeFSLockFactory somehow "knows" a lock is held against write.lock
    // even if you remove that file:
    let mut dir = LuceneTestCase::new_fs_directory_lock_factory(
        &LuceneTestCase::create_temp_dir_prefix(
            "TestIndexWriter.testWhetherDeleteAllDeletesWriteLock",
        ),
        Arc::new(SimpleFSLockFactory),
    );
    let d = dir.as_directory();
    let w1 = RandomIndexWriter::new_with_mock_analyzer(&*random(), Arc::clone(&d))?;
    w1.delete_all()?;
    expect_throws("org.apache.lucene.store.LockObtainFailedException", || {
        RandomIndexWriter::new_with_config(
            &*random(),
            Arc::clone(&d),
            LuceneTestCase::new_index_writer_config_analyzer(None),
        )
    });

    w1.close()?;
    drop((w1, d));
    dir.close()
}

/// `documentSupplier.get()`: a document with `foo:bar`.
fn foo_bar_doc() -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "bar".to_string(),
        Store::NO,
    )?));
    Ok(doc)
}

/// `reader.leaves().get(i).reader()` as a `SegmentReader`'s
/// `getSegmentInfo()`.
fn segment_info_of(reader: &Arc<dyn DirectoryReader>, i: usize) -> Result<SegmentCommitInfo> {
    let leaves = ir(reader).leaves()?;
    let leaf = leaves[i].leaf_reader();
    let sr = leaf.as_segment_reader().unwrap_or_else(|| {
        Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
    });
    Ok(sr.get_segment_info().clone())
}

/// `"codec: " + segmentInfo.info.getCodec().toString()`.
fn codec_message(info: &SegmentCommitInfo) -> String {
    format!(
        "codec: {}",
        info.info
            .codec()
            .map(|c| c.to_string())
            .unwrap_or_else(|| npe("getCodec()"))
    )
}

/// Port of `TestIndexWriter.testHasBlocksMergeFullyDelSegments`.
fn test_has_blocks_merge_fully_del_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(
            &d,
            IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
        )?;
        let docs = vec![foo_bar_doc()?, foo_bar_doc()?];
        w.update_documents(Term::from_text("foo", "bar"), &docs)?;
        w.commit()?;
        if random().next_boolean() {
            w.update_documents(Term::from_text("foo", "bar"), &docs)?;
            w.commit()?; // second segment
        }
        w.update_document(Term::from_text("foo", "bar"), &foo_bar_doc()?)?;
        if random().next_boolean() {
            w.force_merge_deletes_with(true)?;
        } else {
            w.force_merge_with(1, true)?;
        }
        w.commit()?;
        {
            let reader = directory_reader::open(Arc::clone(&d))?;
            let leaves = ir(&reader).leaves()?;
            assert_equals(&1, &leaves.len());
            Assert::assert_false_msg(
                "hasBlocks should be cleared",
                leaves[0].leaf_reader().get_meta_data().has_blocks(),
            );
            drop(leaves);
            reader.close()?;
        }
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testSingleDocsDoNotTriggerHasBlocks`.
fn test_single_docs_do_not_trigger_has_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        iwc.set_max_buffered_docs(i32::MAX)?;
        iwc.set_ram_buffer_size_mb(100.0)?;
        let w = writer(&d, iwc)?;

        let docs = random().next_int_range(1, 100);
        for i in 0..docs {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
            w.add_documents(&[doc])?;
        }
        w.commit()?;
        let si = w.clone_segment_infos()?;
        assert_equals(&1, &si.size());
        Assert::assert_false(si.as_list()[0].info.get_has_blocks());

        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "XXX".to_string(),
            Store::NO,
        )?));
        let mut doc_copy = Document::new();
        doc_copy.add(Box::new(StringField::new(
            "id",
            "XXX".to_string(),
            Store::NO,
        )?));
        w.add_documents(&[doc, doc_copy])?;
        w.commit()?;
        let si = w.clone_segment_infos()?;
        assert_equals(&2, &si.size());
        Assert::assert_false(si.as_list()[0].info.get_has_blocks());
        Assert::assert_true(si.as_list()[1].info.get_has_blocks());
        w.force_merge(1)?;

        w.commit()?;
        let si = w.clone_segment_infos()?;
        assert_equals(&1, &si.size());
        Assert::assert_true(si.as_list()[0].info.get_has_blocks());
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testCarryOverHasBlocks`.
fn test_carry_over_has_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(
            &d,
            IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
        )?;

        let mut docs = vec![Document::new()];
        w.update_documents(Term::from_text("foo", "bar"), &docs)?;
        w.commit()?;
        {
            let reader = directory_reader::open(Arc::clone(&d))?;
            let segment_info = segment_info_of(&reader, 0)?;
            Assert::assert_false(segment_info.info.get_has_blocks());
            reader.close()?;
        }

        docs.push(Document::new()); // now we have 2 docs
        w.update_documents(Term::from_text("foo", "bar"), &docs)?;
        w.commit()?;
        {
            let reader = directory_reader::open(Arc::clone(&d))?;
            assert_equals(&2, &ir(&reader).leaves()?.len());
            let segment_info = segment_info_of(&reader, 0)?;
            Assert::assert_false_msg(
                &codec_message(&segment_info),
                segment_info.info.get_has_blocks(),
            );
            let segment_info = segment_info_of(&reader, 1)?;
            Assert::assert_true_msg(
                &codec_message(&segment_info),
                segment_info.info.get_has_blocks(),
            );
            reader.close()?;
        }
        w.force_merge_with(1, true)?;
        w.commit()?;
        {
            let reader = directory_reader::open(Arc::clone(&d))?;
            assert_equals(&1, &ir(&reader).leaves()?.len());
            let segment_info = segment_info_of(&reader, 0)?;
            Assert::assert_true_msg(
                &codec_message(&segment_info),
                segment_info.info.get_has_blocks(),
            );
            reader.close()?;
        }
        w.commit()?;
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// LUCENE-3872
/// Port of `TestIndexWriter.testPrepareCommitThenClose`.
fn test_prepare_commit_then_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(
        &d,
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )?;

    w.prepare_commit()?;
    expect_throws("java.lang.IllegalStateException", || w.close());
    w.commit()?;
    w.close()?;
    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &r.max_doc());
    r.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-3872
/// Port of `TestIndexWriter.testPrepareCommitThenRollback`.
fn test_prepare_commit_then_rollback() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(
        &d,
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )?;

    w.prepare_commit()?;
    w.rollback()?;
    Assert::assert_false(directory_reader::index_exists(&*d)?);
    drop(d);
    dir.close()
}

/// LUCENE-3872
/// Port of `TestIndexWriter.testPrepareCommitThenRollback2`.
fn test_prepare_commit_then_rollback2() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(
        &d,
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )?;

    w.commit()?;
    w.add_document(&Document::new())?;
    w.prepare_commit()?;
    w.rollback()?;
    Assert::assert_true(directory_reader::index_exists(&*d)?);
    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&0, &r.max_doc());
    r.close()?;
    drop(d);
    dir.close()
}

/// The anonymous `Analyzer` of `testDontInvokeAnalyzerForUnAnalyzedFields`:
/// each of its methods throws `IllegalStateException("don't invoke me!")`.
#[derive(Debug)]
struct DontInvokeMeAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for DontInvokeMeAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriter$4"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Err(LuceneError::illegal_state("don't invoke me!"))
    }

    fn get_position_increment_gap(&self, _field_name: &str) -> i32 {
        Throwable::new("java.lang.IllegalStateException", "don't invoke me!").throw()
    }

    fn get_offset_gap(&self, _field_name: &str) -> i32 {
        Throwable::new("java.lang.IllegalStateException", "don't invoke me!").throw()
    }
}

/// Port of `TestIndexWriter.testDontInvokeAnalyzerForUnAnalyzedFields`.
fn test_dont_invoke_analyzer_for_un_analyzed_fields() -> Result<()> {
    let analyzer = Arc::new(DontInvokeMeAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(
        &d,
        LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer)),
    )?;
    let mut doc = Document::new();
    let custom_type = FieldType::new_from(StringField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    custom_type.set_store_term_vector_offsets(true)?;
    let f: Arc<dyn IndexableField> =
        Arc::new(LuceneTestCase::new_field("field", "abcd", &custom_type));
    doc.add_shared(Arc::clone(&f));
    doc.add_shared(Arc::clone(&f));
    let f2 = LuceneTestCase::new_field("field", "", &custom_type);
    doc.add(Box::new(f2));
    doc.add_shared(f);
    w.add_document(&doc)?;
    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-1468 -- make sure opening an IndexWriter with
/// create=true does not remove non-index files
/// Port of `TestIndexWriter.testOtherFiles`.
fn test_other_files() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iw = writer(&d, mock_iwc())?;
    iw.add_document(&Document::new())?;
    iw.close()?;
    let result = (|| -> Result<()> {
        // Create my own random file:
        let mut out =
            d.create_output("myrandomfile", &*LuceneTestCase::new_io_context(&*random()))?;
        out.write_byte(42)?;
        out.close()?;

        writer(&d, mock_iwc())?.close()?;

        Assert::assert_true(LuceneTestCase::slow_file_exists(&*d, "myrandomfile")?);
        Ok(())
    })();
    drop(d);
    let closed = dir.close();
    result.and(closed)
}

/// The anonymous `Analyzer`s of `testStopwordsPosIncHole` and
/// `testStopwordsPosIncHole2`: a `MockTokenizer` filtered by English stop
/// words, then by `second_set` when given.
#[derive(Debug)]
struct StopwordsAnalyzer {
    state: AnalyzerState,
    second_set: Option<Arc<CharacterRunAutomaton>>,
}

impl Analyzer for StopwordsAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        if self.second_set.is_some() {
            "org.apache.lucene.index.TestIndexWriter$6"
        } else {
            "org.apache.lucene.index.TestIndexWriter$5"
        }
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::whitespace_lower_case()?));
        let stream = MockTokenFilter::new(shared(&tokenizer), Arc::clone(&ENGLISH_STOPSET))?;
        let stream: Arc<AtomicRefCell<dyn TokenStream>> = match &self.second_set {
            Some(second_set) => sink(MockTokenFilter::new(
                Box::new(stream),
                Arc::clone(second_set),
            )?),
            None => sink(stream),
        };
        Ok(components(tokenizer, stream))
    }
}

/// The body `testStopwordsPosIncHole` and `testStopwordsPosIncHole2` share:
/// index `first` and `test of gaps` into `body`, then search `just` and
/// `test` `gap` positions apart.
fn stopwords_pos_inc_hole(
    second_set: Option<Arc<CharacterRunAutomaton>>,
    first: &str,
    gap: i32,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let a = Arc::new(StopwordsAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        second_set,
    });
    let iw = RandomIndexWriter::new(&*random(), Arc::clone(&d), a)?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "body",
        first.to_string(),
        Store::NO,
    )?));
    doc.add(Box::new(TextField::new(
        "body",
        "test of gaps".to_string(),
        Store::NO,
    )?));
    iw.add_document(&doc)?;
    let reader = iw.get_reader()?;
    iw.close()?;
    let is = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(Term::from_text("body", "just"), 0)?;
    builder.add_at(Term::from_text("body", "test"), gap)?;
    let pq = builder.build()?;
    assert_equals(&1, &is.search(Arc::new(pq), 5)?.total_hits.value());
    drop(is);
    reader.close()?;
    drop((iw, d));
    dir.close()
}

/// LUCENE-3849
/// Port of `TestIndexWriter.testStopwordsPosIncHole`.
fn test_stopwords_pos_inc_hole() -> Result<()> {
    // body:"just ? test"
    stopwords_pos_inc_hole(None, "just a", 2)
}

/// LUCENE-3849
/// Port of `TestIndexWriter.testStopwordsPosIncHole2`.
fn test_stopwords_pos_inc_hole2() -> Result<()> {
    // use two stopfilters for testing here
    let second_set = Automata::make_string("foobar");
    // body:"just ? ? test"
    stopwords_pos_inc_hole(
        Some(Arc::new(CharacterRunAutomaton::new(second_set)?)),
        "just a foobar",
        3,
    )
}

/// `newIndexWriterConfig(null)`.
fn null_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(None)
}

/// `new HashMap<String, String>() {{ put(key, value); }}.entrySet()`.
fn one_entry(key: &str, value: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert(key.to_string(), value.to_string());
    map
}

/// `r.getIndexCommit().getUserData().get(key)`.
fn commit_user_data(r: &Arc<dyn DirectoryReader>, key: &str) -> Result<Option<String>> {
    Ok(r.index_commit()?.get_user_data()?.get(key).cloned())
}

/// LUCENE-4575
/// Port of `TestIndexWriter.testCommitWithUserDataOnly`.
fn test_commit_with_user_data_only() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, null_iwc())?;
    w.commit()?; // first commit to complete IW create transaction.

    // this should store the commit data, even though no other changes were made
    w.set_live_commit_data(one_entry("key", "value"))?;
    w.commit()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&Some("value".to_string()), &commit_user_data(&r, "key")?);
    r.close()?;

    // now check setCommitData and prepareCommit/commit sequence
    w.set_live_commit_data(one_entry("key", "value1"))?;
    w.prepare_commit()?;
    w.set_live_commit_data(one_entry("key", "value2"))?;
    w.commit()?; // should commit the first commitData only, per protocol

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&Some("value1".to_string()), &commit_user_data(&r, "key")?);
    r.close()?;

    // now should commit the second commitData - there was a bug where
    // IndexWriter.finishCommit overrode the second commitData
    w.commit()?;
    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals_msg(
        Some("IndexWriter.finishCommit may have overridden the second commitData"),
        &Some("value2".to_string()),
        &commit_user_data(&r, "key")?,
    );
    r.close()?;

    w.close()?;
    drop(d);
    dir.close()
}

/// `getLiveCommitData(writer)`: the writer's live commit data as a map.
fn get_live_commit_data(writer: &IndexWriter) -> Result<HashMap<String, String>> {
    let mut data = HashMap::new();
    for (key, value) in writer.get_live_commit_data()? {
        data.insert(key, value);
    }
    Ok(data)
}

/// Port of `TestIndexWriter.testGetCommitData`.
fn test_get_commit_data() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, null_iwc())?;
    w.set_live_commit_data(one_entry("key", "value"))?;
    assert_equals(
        &Some(&"value".to_string()),
        &get_live_commit_data(&w)?.get("key"),
    );
    w.close()?;

    // validate that it's also visible when opening a new IndexWriter
    let iwc = null_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let w = writer(&d, iwc)?;
    assert_equals(
        &Some(&"value".to_string()),
        &get_live_commit_data(&w)?.get("key"),
    );
    w.close()?;

    drop(d);
    dir.close()
}

/// `newSnapshotIndexWriterConfig(null)`.
fn null_snapshot_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_snapshot_index_writer_config(None)
}

/// Port of `TestIndexWriter.testGetCommitDataFromOldSnapshot`.
fn test_get_commit_data_from_old_snapshot() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, null_snapshot_iwc())?;
    w.set_live_commit_data(one_entry("key", "value"))?;
    assert_equals(
        &Some(&"value".to_string()),
        &get_live_commit_data(&w)?.get("key"),
    );
    w.commit()?;
    // Snapshot this commit to open later
    let policy = w.get_config().index_deletion_policy();
    let index_commit = policy
        .as_any()
        .and_then(|a| a.downcast_ref::<SnapshotDeletionPolicy>())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "not a SnapshotDeletionPolicy",
            )
            .throw()
        })
        .snapshot()?;
    w.close()?;

    // Modify the commit data and commit on close so the most recent commit data is different
    let w = writer(&d, null_snapshot_iwc())?;
    w.set_live_commit_data(one_entry("key", "value2"))?;
    assert_equals(
        &Some(&"value2".to_string()),
        &get_live_commit_data(&w)?.get("key"),
    );
    w.close()?;

    // validate that when opening writer from older snapshotted index commit,
    // the old commit data is visible
    let iwc = null_snapshot_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    iwc.set_index_commit(Some(index_commit));
    let w = writer(&d, iwc)?;
    assert_equals(
        &Some(&"value".to_string()),
        &get_live_commit_data(&w)?.get("key"),
    );
    w.close()?;

    drop(d);
    dir.close()
}

/// The three good documents `testNullAnalyzer`, `testNullDocument` and
/// `testNullDocuments` add.
fn add_three_good_docs(iw: &RandomIndexWriter) -> Result<()> {
    for i in 0..3 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
        iw.add_document(&doc)?;
    }
    Ok(())
}

/// Port of `TestIndexWriter.testNullAnalyzer`.
fn test_null_analyzer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iw_conf = LuceneTestCase::new_index_writer_config_analyzer(None);
    let iw = RandomIndexWriter::new_with_config(&*random(), Arc::clone(&d), iw_conf)?;

    // add 3 good docs
    add_three_good_docs(&iw)?;

    // add broken doc
    expect_throws("java.lang.NullPointerException", || {
        let mut broke = Document::new();
        broke.add(Box::new(Field::new(
            "test",
            "broken".to_string(),
            DocHelper::text_type_stored_with_tvs(),
        )?));
        iw.add_document(&broke)
    });

    // ensure good docs are still ok
    let reader = iw.get_reader()?;
    assert_equals(&3, &reader.num_docs());
    reader.close()?;
    iw.close()?;
    drop((iw, d));
    dir.close()
}

/// Port of `TestIndexWriter.testNullDocument`.
fn test_null_document() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), Arc::clone(&d))?;

    // add 3 good docs
    add_three_good_docs(&iw)?;

    // add broken doc
    expect_throws("java.lang.NullPointerException", || {
        iw.add_document(None::<&Document>)
    });

    // ensure good docs are still ok
    let reader = iw.get_reader()?;
    assert_equals(&3, &reader.num_docs());

    reader.close()?;
    iw.close()?;
    drop((iw, d));
    dir.close()
}

/// Port of `TestIndexWriter.testNullDocuments`.
fn test_null_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), Arc::clone(&d))?;

    // add 3 good docs
    add_three_good_docs(&iw)?;

    // add broken doc block
    expect_throws("java.lang.NullPointerException", || {
        iw.add_documents(None::<&[Document]>)
    });

    // ensure good docs are still ok
    let reader = iw.get_reader()?;
    assert_equals(&3, &reader.num_docs());

    reader.close()?;
    iw.close()?;
    drop((iw, d));
    dir.close()
}

/// `new RuntimeException("boom")`.
fn boom() -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.RuntimeException",
        message: Some("boom".to_string()),
        cause: None,
    }
}

/// `private static class RandomFailingIterable<T> implements Iterable<T>`,
/// over the fields of a document: each iterator's `next()` throws
/// `RuntimeException("boom")` when asked for the element at `failOn`.
struct RandomFailingIterable {
    list: Vec<Box<dyn IndexableField>>,
    fail_on: usize,
}

impl RandomFailingIterable {
    fn new(list: Vec<Box<dyn IndexableField>>, random: &dyn Random) -> Self {
        RandomFailingIterable {
            list,
            fail_on: random.next_int_bound(5) as usize,
        }
    }
}

impl IndexableFieldIterable for RandomFailingIterable {
    fn iterator(&self) -> Box<dyn Iterator<Item = Result<&dyn IndexableField>> + '_> {
        let mut doc_iter = self.list.iter();
        let mut count = 0;
        Box::new(std::iter::from_fn(move || {
            // `hasNext()`: `docIter.hasNext()`.
            if doc_iter.len() == 0 {
                return None;
            }
            // `next()`.
            if count == self.fail_on {
                return Some(Err(boom()));
            }
            count += 1;
            doc_iter.next().map(|field| Ok(field.as_ref()))
        }))
    }
}

/// The body `testIterableFieldThrowsException` and
/// `testIterableThrowsException` share.
fn iterable_throws_exception() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let iters = LuceneTestCase::at_least(100);
    let mut doc_count = 0;
    let mut doc_id = 0;
    let mut live_ids: HashSet<String> = HashSet::new();
    for _ in 0..iters {
        let num_docs = LuceneTestCase::at_least(4);
        for _ in 0..num_docs {
            let id = doc_id.to_string();
            doc_id += 1;
            let fields: Vec<Box<dyn IndexableField>> = vec![
                Box::new(StringField::new("id", id.clone(), Store::YES)?),
                Box::new(StringField::new(
                    "foo",
                    TestUtil::random_simple_string(&*random()),
                    Store::NO,
                )?),
            ];
            doc_id += 1;

            match w.add_document(&RandomFailingIterable::new(fields, &*random())) {
                Ok(_) => {
                    doc_count += 1;
                    live_ids.insert(id);
                }
                Err(e) if is_instance(&e, "java.lang.RuntimeException") => {
                    assert_equals(&Some("boom"), &java_throwable(&e).message());
                }
                Err(e) => return Err(e),
            }
        }
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    assert_equals(&doc_count, &reader.num_docs());
    let leaves = ir(&reader).leaves()?;
    for leaf_reader_context in &leaves {
        let ar = leaf_reader_context.leaf_reader();
        let live_docs = ar.get_live_docs();
        let max_doc = ar.max_doc();
        let stored_fields = ar.stored_fields()?;
        for i in 0..max_doc {
            if live_docs.as_ref().is_none_or(|l| l.get(i as usize)) {
                let id = stored_fields.document(i)?.get("id").unwrap_or_default();
                Assert::assert_true(live_ids.remove(&id));
            }
        }
    }
    Assert::assert_true(live_ids.is_empty());
    w.close()?;
    drop(leaves);
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testIterableFieldThrowsException`.
fn test_iterable_field_throws_exception() -> Result<()> {
    iterable_throws_exception()
}

/// Port of `TestIndexWriter.testIterableThrowsException`.
fn test_iterable_throws_exception() -> Result<()> {
    iterable_throws_exception()
}

/// The anonymous `Iterable<Document>` of `testIterableThrowsException2`,
/// whose iterator always has a next element and throws
/// `RuntimeException("boom")` when asked for it.
struct BoomDocuments;

/// The anonymous `Iterator<Document>` of [`BoomDocuments`].
struct BoomDocumentsIterator;

impl<'a> DocumentsIterator<'a> for BoomDocumentsIterator {
    fn has_next(&mut self) -> Result<bool> {
        Ok(true)
    }

    fn next(&mut self) -> Result<Option<&'a dyn IndexableFieldIterable>> {
        Err(boom())
    }
}

impl DocumentsIterable for BoomDocuments {
    fn iterator(&self) -> Box<dyn DocumentsIterator<'_> + '_> {
        Box::new(BoomDocumentsIterator)
    }
}

/// Port of `TestIndexWriter.testIterableThrowsException2`.
fn test_iterable_throws_exception2() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let expected = expect_throws("java.lang.Exception", || w.add_documents(&BoomDocuments));
    assert_equals(&Some("boom"), &expected.message());

    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-2727/LUCENE-2812/LUCENE-4738:
/// Port of `TestIndexWriter.testCorruptFirstCommit`.
fn test_corrupt_first_commit() -> Result<()> {
    for i in 0..6 {
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();

        // Create a corrupt first commit:
        d.create_output(
            &index_file_names::file_name_from_generation(index_file_names::PENDING_SEGMENTS, "", 0)
                .unwrap_or_else(|| npe("fileNameFromGeneration")),
            &*DEFAULT_IO_CONTEXT,
        )?
        .close()?;

        let iwc = mock_iwc();
        let mode = i / 2;
        if mode == 0 {
            iwc.set_open_mode(OpenMode::CREATE)?;
        } else if mode == 1 {
            iwc.set_open_mode(OpenMode::APPEND)?;
        } else if mode == 2 {
            iwc.set_open_mode(OpenMode::CREATE_OR_APPEND)?;
        }

        if verbose() {
            println!("\nTEST: i={i}");
        }

        let outcome = (|| -> Result<()> {
            if (i & 1) == 0 {
                writer(&d, iwc)?.close()
            } else {
                writer(&d, iwc)?.rollback()
            }
        })();
        if let Err(ioe) = outcome {
            // OpenMode.APPEND should throw an exception since no index exists:
            if mode == 0 || !ioe.is_io_exception() {
                // Unexpected
                return Err(ioe);
            }
        }

        if verbose() {
            println!("  at close: [{}]", d.list_all()?.join(", "));
        }

        if mode != 0 {
            set_check_index_on_close(&dir, false);
        }

        drop(d);
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestIndexWriter.testHasUncommittedChanges`.
fn test_has_uncommitted_changes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    // Disable merging to simplify this test, otherwise a commit might trigger
    // uncommitted merges.
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let w = writer(&d, iwc)?;
    Assert::assert_true(w.has_uncommitted_changes()?); // this will be true because a commit will create an empty
                                                       // index
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "myfield",
        "a b c",
        Store::NO,
    )));
    w.add_document(&doc)?;
    Assert::assert_true(w.has_uncommitted_changes()?);

    // Must commit, waitForMerges, commit again, to be
    // certain that hasUncommittedChanges returns false:
    w.commit()?;
    w.wait_for_merges()?;
    w.commit()?;
    Assert::assert_false(w.has_uncommitted_changes()?);
    w.add_document(&doc)?;
    Assert::assert_true(w.has_uncommitted_changes()?);
    w.commit()?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "xyz",
        Store::YES,
    )));
    w.add_document(&doc)?;
    Assert::assert_true(w.has_uncommitted_changes()?);

    w.commit()?;
    Assert::assert_false(w.has_uncommitted_changes()?);
    w.delete_documents_by_terms(vec![Term::from_text("id", "xyz")])?;
    Assert::assert_true(w.has_uncommitted_changes()?);

    w.commit()?;
    Assert::assert_false(w.has_uncommitted_changes()?);
    w.close()?;

    let w = writer(&d, mock_iwc())?;
    Assert::assert_false(w.has_uncommitted_changes()?);
    w.add_document(&doc)?;
    Assert::assert_true(w.has_uncommitted_changes()?);

    w.close()?;
    drop(d);
    dir.close()
}

/// A `TestPoint` lambda (`message -> { … }`).
struct FnTestPoint<F: Fn(&str) + Send + Sync>(F);

impl<F: Fn(&str) + Send + Sync> TestPoint for FnTestPoint<F> {
    fn apply(&self, message: &str) {
        (self.0)(message)
    }
}

/// Port of `TestIndexWriter.testMergeAllDeleted`.
fn test_merge_all_deleted() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = mock_iwc();
    let keep_fully_deleted_segments = Arc::new(AtomicBool::new(false));
    iwc.set_merge_policy(Some(Arc::new(
        KeepFullyDeletedSegmentsMergePolicy::with_flag(
            iwc.merge_policy(),
            Arc::clone(&keep_fully_deleted_segments),
        ),
    )))?;
    let keep = Arc::clone(&keep_fully_deleted_segments);
    let evil_writer = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        Arc::clone(&d),
        iwc,
        Arc::new(FnTestPoint(move |message: &str| {
            if "startCommitMerge" == message {
                keep.store(false, Ordering::SeqCst);
            } else if "startMergeInit" == message {
                keep.store(true, Ordering::SeqCst);
            }
        })),
    )?;
    // `iwRef.set(evilWriter)`: the reference is never read.
    for _ in 0..1000 {
        add_doc(&evil_writer)?;
        if random().next_int_bound(17) == 0 {
            evil_writer.commit()?;
        }
    }
    evil_writer.delete_documents_by_queries(vec![Arc::new(MatchAllDocsQuery::instance())])?;
    evil_writer.force_merge(1)?;
    evil_writer.close()?;
    drop((evil_writer, d));
    dir.close()
}

/// LUCENE-5239
/// Port of `TestIndexWriter.testDeleteSameTermAcrossFields`.
fn test_delete_same_term_across_fields() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new("a", "foo".to_string(), Store::NO)?));
    w.add_document(&doc)?;

    // Should not delete the document;
    // with LUCENE-5239 the "foo" from the 2nd delete term would incorrectly match field a's "foo":
    w.delete_documents_by_terms(vec![Term::from_text("a", "xxx")])?;
    w.delete_documents_by_terms(vec![Term::from_text("b", "foo")])?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;

    // Make sure document was not (incorrectly) deleted:
    assert_equals(&1, &r.num_docs());
    r.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testHasUncommittedChangesAfterException`.
fn test_has_uncommitted_changes_after_exception() -> Result<()> {
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));

    let mut directory = LuceneTestCase::new_directory();
    let d = directory.as_directory();
    // we don't use RandomIndexWriter because it might add more docvalues than we expect !!!!
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iwriter = writer(&d, iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(SortedDocValuesField::new(
        "dv",
        LuceneTestCase::new_bytes_ref_str("foo!"),
    )));
    doc.add(Box::new(SortedDocValuesField::new(
        "dv",
        LuceneTestCase::new_bytes_ref_str("bar!"),
    )));
    expect_throws("java.lang.IllegalArgumentException", || {
        iwriter.add_document(&doc)
    });

    iwriter.commit()?;
    Assert::assert_false(iwriter.has_uncommitted_changes()?);
    iwriter.close()?;
    drop(d);
    directory.close()
}

/// A document with one `SortedDocValuesField("dv", newBytesRef("foo!"))`.
fn dv_foo_doc() -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(SortedDocValuesField::new(
        "dv",
        LuceneTestCase::new_bytes_ref_str("foo!"),
    )));
    doc
}

/// Port of `TestIndexWriter.testDoubleClose`.
fn test_double_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    w.add_document(&dv_foo_doc())?;
    w.close()?;
    // Close again should have no effect
    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testRollbackThenClose`.
fn test_rollback_then_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    w.add_document(&dv_foo_doc())?;
    w.rollback()?;
    // Close after rollback should have no effect
    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testCloseThenRollback`.
fn test_close_then_rollback() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    w.add_document(&dv_foo_doc())?;
    w.close()?;
    // Rollback after close should have no effect
    w.rollback()?;
    drop(d);
    dir.close()
}

/// The anonymous `InfoStream` of `testCloseWhileMergeIsRunning`: enabled for
/// every component, it counts `closeStarted` down on the `rollback` message.
struct RollbackInfoStream {
    close_started: Arc<CountDownLatch>,
}

impl InfoStream for RollbackInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriter$7"
    }

    fn message(&self, _component: &str, message: &str) {
        if message == "rollback" {
            self.close_started.count_down();
        }
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}
}

/// Port of `TestIndexWriter.testCloseWhileMergeIsRunning`.
fn test_close_while_merge_is_running() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let merge_started = Arc::new(CountDownLatch::new(1));
    let close_started = Arc::new(CountDownLatch::new(1));

    let iwc = LuceneTestCase::new_index_writer_config_with(
        &*random(),
        Some(Arc::new(MockAnalyzer::new(&*random()))),
    );
    iwc.set_commit_on_close(false);
    let mp = LogMergePolicy::new(Arc::new(LogDocSize));
    mp.set_merge_factor(2)?;
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    iwc.set_info_stream(Arc::new(RollbackInfoStream {
        close_started: Arc::clone(&close_started),
    }))?;

    let cms = ConcurrentMergeScheduler::new();
    {
        let merge_started = Arc::clone(&merge_started);
        let close_started = Arc::clone(&close_started);
        cms.set_do_merge_override(Some(Arc::new(move |cms, merge_source, merge| {
            merge_started.count_down();
            close_started.await_();
            cms.default_do_merge(merge_source, merge)
        })));
    }
    cms.set_close_override(Some(Arc::new(|_| Ok(()))));
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;
    let w = writer(&d, iwc)?;
    let doc = dv_foo_doc();
    w.add_document(&doc)?;
    w.commit()?;
    w.add_document(&doc)?;
    w.commit()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// Make sure that close waits for any still-running commits.
/// Port of `TestIndexWriter.testCloseDuringCommit`.
fn test_close_during_commit() -> Result<()> {
    let start_commit = Arc::new(CountDownLatch::new(1));
    let finish_commit = Arc::new(CountDownLatch::new(1));

    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(None);
    // use an InfoStream that "takes a long time" to commit
    let start = Arc::clone(&start_commit);
    let iw = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        Arc::clone(&d),
        iwc,
        Arc::new(FnTestPoint(move |message: &str| {
            if message == "finishStartCommit" {
                start.count_down();
                Thread::sleep(10);
            }
        })),
    )?;
    {
        let iw = Arc::clone(&iw);
        let finish_commit = Arc::clone(&finish_commit);
        Thread::unnamed(move || {
            if let Err(ioe) = iw.commit() {
                rethrow_java(LuceneError::runtime_exception(ioe));
            }
            finish_commit.count_down();
        })
        .start();
    }
    start_commit.await_();
    match iw.close() {
        Ok(()) => {}
        Err(ise) if is_instance(&ise, "java.lang.IllegalStateException") => {
            // OK, but not required (depends on thread scheduling)
        }
        Err(e) => return Err(e),
    }
    finish_commit.await_();
    iw.close()?;
    drop((iw, d));
    dir.close()
}

/// LUCENE-5895:
/// Make sure we see ids per segment and per commit.
/// Port of `TestIndexWriter.testIds`.
fn test_ids() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    w.add_document(&Document::new())?;
    w.close()?;

    let sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    let id1 = sis.id();
    assert_equals(&ID_LENGTH, &id1.len());

    let id2 = sis.info(0).info.id();
    let sci_id2 = sis.info(0).id();
    assert_equals(&ID_LENGTH, &id2.len());
    assert_equals(&ID_LENGTH, &sci_id2.len());

    // Make sure CheckIndex includes id output:
    let bos = Arc::new(Mutex::new(Vec::with_capacity(1024)));
    let mut checker = CheckIndex::new(Arc::clone(&d))?;
    checker.set_level(Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS)?;
    checker.set_info_stream_verbose(Some(Arc::new(PrintStreamLines(Arc::clone(&bos)))), false);
    let index_status = checker.check_index_segments(None)?;
    let s = String::from_utf8_lossy(&bos.lock().unwrap_or_else(|e| e.into_inner())).into_owned();
    checker.close()?;
    // Make sure CheckIndex didn't fail
    Assert::assert_true_msg(&s, index_status.clean);

    // Commit id is always stored:
    let id1_string = StringHelper::id_to_string(Some(&id1));
    Assert::assert_true_msg(
        &format!("missing id={id1_string} in:\n{s}"),
        s.contains(&format!("id={id1_string}")),
    );

    Assert::assert_true_msg(
        &format!("missing id={id1_string} in:\n{s}"),
        s.contains(&format!("id={id1_string}")),
    );
    drop(d);
    dir.close()?;

    let mut ids: HashSet<String> = HashSet::new();
    for i in 0..100000 {
        let id = StringHelper::id_to_string(Some(&StringHelper::random_id()));
        Assert::assert_false_msg(&format!("id={id} i={i}"), ids.contains(&id));
        ids.insert(id);
    }
    Ok(())
}

/// Port of `TestIndexWriter.testEmptyNorm`.
fn test_empty_norm() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut doc = Document::new();
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(CannedTokenStream::new(Vec::new())));
    doc.add(Box::new(TextField::new_with_token_stream("foo", stream)?));
    w.add_document(&doc)?;
    w.commit()?;
    w.close()?;
    let r = directory_reader::open(Arc::clone(&d))?;
    let mut norms = LuceneTestCase::get_only_leaf_reader(ir(&r))?
        .get_norm_values("foo")?
        .unwrap_or_else(|| npe("norms"));
    assert_equals(&0, &norms.next_doc()?);
    assert_equals(&0, &norms.long_value()?);
    r.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testManySeparateThreads`.
fn test_many_separate_threads() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_max_buffered_docs(1000)?;
    let w = writer(&d, iwc)?;
    // Index 100 docs, each from a new thread, but always only 1 thread is in IW at once:
    for _ in 0..100 {
        let w = Arc::clone(&w);
        let thread = Thread::unnamed(move || {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "foo",
                "bar",
                Store::NO,
            )));
            if let Err(ioe) = w.add_document(&doc) {
                rethrow_java(LuceneError::runtime_exception(ioe));
            }
        });
        thread.start();
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
    }
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&1, &ir(&r).leaves()?.len());
    r.close()?;
    drop(d);
    dir.close()
}

/// `r.getIndexCommit().getGeneration()` and `getSegmentsFileName()`.
fn assert_commit(r: &Arc<dyn DirectoryReader>, generation: i64, file: &str) -> Result<()> {
    let commit = r.index_commit()?;
    assert_equals(&generation, &commit.get_generation());
    assert_equals(&file.to_string(), &commit.get_segments_file_name());
    Ok(())
}

/// LUCENE-6505
/// Port of `TestIndexWriter.testNRTSegmentsFile`.
fn test_nrt_segments_file() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;
    // creates segments_1
    w.commit()?;

    // newly opened NRT reader should see gen=1 segments file
    let r = directory_reader::open_with_writer(&*w)?;
    assert_commit(&r, 1, "segments_1")?;

    // newly opened non-NRT reader should see gen=1 segments file
    let r2 = directory_reader::open(Arc::clone(&d))?;
    assert_commit(&r2, 1, "segments_1")?;
    r2.close()?;

    // make a change and another commit
    w.add_document(&Document::new())?;
    w.commit()?;
    let r3 = directory_reader::open_if_changed(Arc::clone(&r))?;
    r.close()?;
    assert_not_null(&r3);
    let r3 = r3.unwrap_or_else(|| npe("r3"));

    // reopened NRT reader should see gen=2 segments file
    assert_commit(&r3, 2, "segments_2")?;
    r3.close()?;

    // newly opened non-NRT reader should see gen=2 segments file
    let r4 = directory_reader::open(Arc::clone(&d))?;
    assert_commit(&r4, 2, "segments_2")?;
    r4.close()?;

    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-6505
/// Port of `TestIndexWriter.testNRTAfterCommit`.
fn test_nrt_after_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;
    w.commit()?;

    w.add_document(&Document::new())?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.commit()?;

    // commit even with no other changes counts as a "change" that NRT reader reopen will see:
    let r2 = directory_reader::open(Arc::clone(&d))?;
    assert_commit(&r2, 2, "segments_2")?;

    r.close()?;
    r2.close()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-6505
/// Port of `TestIndexWriter.testNRTAfterSetUserDataWithoutCommit`.
fn test_nrt_after_set_user_data_without_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;
    w.commit()?;

    let r = directory_reader::open_with_writer(&*w)?;
    w.set_live_commit_data(one_entry("foo", "bar"))?;

    // setLiveCommitData with no other changes should count as an NRT change:
    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_not_null(&r2);

    if let Some(r2) = r2 {
        r2.close()?;
    }
    r.close()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-6505
/// Port of `TestIndexWriter.testNRTAfterSetUserDataWithCommit`.
fn test_nrt_after_set_user_data_with_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;
    w.commit()?;

    let r = directory_reader::open_with_writer(&*w)?;
    w.set_live_commit_data(one_entry("foo", "bar"))?;
    w.commit()?;
    // setLiveCommitData and also commit, with no other changes, should count as an NRT change:
    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_not_null(&r2);
    r.close()?;
    if let Some(r2) = r2 {
        r2.close()?;
    }
    w.close()?;
    drop(d);
    dir.close()
}

/// LUCENE-6523
/// Port of `TestIndexWriter.testCommitImmediatelyAfterNRTReopen`.
fn test_commit_immediately_after_nrt_reopen() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;
    w.commit()?;

    w.add_document(&Document::new())?;

    let r = directory_reader::open_with_writer(&*w)?;
    w.commit()?;

    Assert::assert_false(r.is_current()?);

    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_not_null(&r2);
    let r2 = r2.unwrap_or_else(|| npe("r2"));
    // segments_N should have changed:
    assert_not_equals(
        &r2.index_commit()?.get_segments_file_name(),
        &r.index_commit()?.get_segments_file_name(),
    );
    r.close()?;
    r2.close()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// `dir.close()` on a directory this test shares through an `Arc`, once
/// the writers and readers holding it are gone.
fn close_shared<D: Directory + ?Sized>(mut dir: Arc<D>) -> Result<()> {
    match Arc::get_mut(&mut dir) {
        Some(dir) => dir.close(),
        None => Err(LuceneError::illegal_state(
            "the directory is still shared and cannot be closed",
        )),
    }
}

/// `createTempDir()` wrapped in a `WindowsFS` ("Use WindowsFS to prevent open
/// files from being deleted").
fn windows_root() -> Path {
    let path = LuceneTestCase::create_temp_dir();
    let provider = WindowsFS::new(Arc::clone(path.get_file_system()));
    provider.wrap_path(&path)
}

/// The `IndexWriterConfig` of `testPendingDeleteDVGeneration`.
fn pending_delete_dv_iwc() -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_use_compound_file(false)?;
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?; // avoid merging away the randomFile
    iwc.set_max_buffered_docs(2)?;
    iwc.set_ram_buffer_size_mb(-1.0)?;
    Ok(iwc)
}

/// A document with `StringField("id", id)` and `NumericDocValuesField("nvd",
/// value)`.
fn id_nvd_doc(id: &str, value: i64) -> Result<Document> {
    let mut d = Document::new();
    d.add(Box::new(StringField::new(
        "id",
        id.to_string(),
        Store::YES,
    )?));
    d.add(Box::new(NumericDocValuesField::new("nvd", value)));
    Ok(d)
}

/// Port of `TestIndexWriter.testPendingDeleteDVGeneration`.
fn test_pending_delete_dv_generation() -> Result<()> {
    let root = windows_root();

    // MMapDirectory doesn't work because it closes its file handles after mapping!
    let mut to_close: Vec<Box<dyn IndexInput>> = Vec::new();
    let dir = NIOFSDirectory::open(root)?;
    let d: Arc<dyn Directory> = Arc::new(dir);
    let result = (|| -> Result<()> {
        let iwc = pending_delete_dv_iwc()?;
        let mut w = writer(&d, iwc)?;
        w.add_document(&id_nvd_doc("1", 1)?)?;
        w.add_document(&id_nvd_doc("2", 2)?)?;
        w.flush()?;
        w.update_document(Term::from_text("id", "1"), &id_nvd_doc("1", 1)?)?;
        w.commit()?;
        let files: HashSet<String> = d.list_all()?.into_iter().collect();
        let num_iters = 10 + random().next_int_bound(50);
        for _ in 0..num_iters {
            if random().next_boolean() {
                w.update_document(Term::from_text("id", "1"), &id_nvd_doc("1", 1)?)?;
            } else if random().next_boolean() {
                w.delete_documents_by_terms(vec![Term::from_text("id", "2")])?;
            } else {
                w.update_numeric_doc_value(Term::from_text("id", "1"), "nvd", 2)?;
            }
            w.prepare_commit()?;
            let new_files: Vec<String> = d
                .list_all()?
                .into_iter()
                .filter(|f| !files.contains(f))
                .collect();
            let random_file = RandomPicks::random_from(&*random(), &new_files).clone();
            to_close.push(d.open_input(&random_file, &*DEFAULT_IO_CONTEXT)?);
            w.rollback()?;
            let iwc = pending_delete_dv_iwc()?;
            w = writer(&d, iwc)?;
            expect_throws("java.nio.file.NoSuchFileException", || {
                d.delete_file(&random_file)
            });
        }
        w.close()
    })();
    // try-with-resources: `closeable` (`IOUtils.close(toClose)`), then `dir`.
    let mut closed = Ok(());
    for mut input in to_close {
        let r = input.close();
        if closed.is_ok() {
            closed = r;
        }
    }
    let dir_closed = close_shared(d);
    result.and(closed).and(dir_closed)
}

/// Port of `TestIndexWriter.testPendingDeletionsRollbackWithReader`.
fn test_pending_deletions_rollback_with_reader() -> Result<()> {
    let root = windows_root();
    // `new FilterDirectory(_dir) {}`: the filter owns `_dir`, which
    // try-with-resources closes; closing the filter closes it.
    let dir: Arc<dyn Directory> =
        Arc::new(FilterDirectory::new(Box::new(NIOFSDirectory::open(root)?)));
    let result = (|| -> Result<()> {
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        let w = writer(&dir, iwc)?;
        let mut d = Document::new();
        d.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::YES,
        )?));
        d.add(Box::new(NumericDocValuesField::new("numval", 1)));
        w.add_document(&d)?;
        w.commit()?;
        w.add_document(&d)?;
        w.flush()?;
        let reader = directory_reader::open_with_writer(&*w)?;
        w.rollback()?;

        // try-delete superfluous files (some will fail due to windows-fs)
        let iwc2 = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        writer(&dir, iwc2)?.close()?;

        // test that we can index on top of pending deletions
        let iwc3 = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        let w = writer(&dir, iwc3)?;
        w.add_document(&d)?;
        w.commit()?;

        reader.close()?;
        w.close()
    })();
    let closed = close_shared(dir);
    result.and(closed)
}

/// Port of `TestIndexWriter.testWithPendingDeletions`.
fn test_with_pending_deletions() -> Result<()> {
    let root = windows_root();
    // MMapDirectory doesn't work because it closes its file handles after mapping!
    let dir: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(root)?);
    let result = (|| -> Result<()> {
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
        let w = writer(&dir, iwc)?;
        w.commit()?;
        let reader = directory_reader::open_with_writer(&*w)?;
        // we pull this commit to open it again later to check that we fail if a future file delete is
        // pending
        let index_commit: Arc<dyn IndexCommit> = Arc::from(reader.index_commit()?);
        w.close()?;
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
        let w = writer(&dir, iwc)?;
        w.add_document(&Document::new())?;
        w.close()?;
        let mut input = dir.open_input("segments_2", &*DEFAULT_IO_CONTEXT)?;
        dir.delete_file("segments_2")?;
        Assert::assert_true(!dir.get_pending_deletions()?.is_empty());

        // make sure we get NoSuchFileException if we try to delete and already-pending-delete file:
        expect_throws("java.nio.file.NoSuchFileException", || {
            dir.delete_file("segments_2")
        });

        {
            let iwc =
                IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
            iwc.set_index_commit(Some(index_commit));
            let w = writer(&dir, iwc)?;
            let body = (|| -> Result<()> {
                w.add_document(&Document::new())?;
                w.commit()?;
                assert_equals(&1, &w.get_doc_stats()?.max_doc);
                // now check that we moved to 3
                dir.open_input("segments_3", &*DEFAULT_IO_CONTEXT)?.close()
            })();
            let closed = w.close();
            body.and(closed)?;
        }
        reader.close()?;
        input.close()
    })();
    let closed = close_shared(dir);
    result.and(closed)
}

/// Port of `TestIndexWriter.testPendingDeletesAlreadyWrittenFiles`.
fn test_pending_deletes_already_written_files() -> Result<()> {
    let root = windows_root();
    // MMapDirectory doesn't work because it closes its file handles after mapping!
    let dir: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(root)?);
    let result = (|| -> Result<()> {
        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        let w = writer(&dir, iwc)?;
        w.commit()?;
        let mut input = dir.open_input("segments_1", &*DEFAULT_IO_CONTEXT)?;
        w.add_document(&Document::new())?;
        w.close()?;

        Assert::assert_true(!dir.get_pending_deletions()?.is_empty());

        // make sure we get NoSuchFileException if we try to delete and already-pending-delete file:
        expect_throws("java.nio.file.NoSuchFileException", || {
            dir.delete_file("segments_1")
        });
        writer(
            &dir,
            IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
        )?
        .close()?;
        input.close()
    })();
    let closed = close_shared(dir);
    result.and(closed)
}

/// Port of `TestIndexWriter.testLeftoverTempFiles`.
fn test_leftover_temp_files() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;
    w.close()?;

    let mut out = d.create_temp_output("_0", "bkd", &*DEFAULT_IO_CONTEXT)?;
    let temp_name = out.name().to_string();
    out.close()?;
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;

    // Make sure IW deleted the unref'd file:
    match d.open_input(&temp_name, &*DEFAULT_IO_CONTEXT) {
        Ok(_) => Assert::fail("did not hit exception"),
        Err(e)
            if is_instance(&e, "java.io.FileNotFoundException")
                || is_instance(&e, "java.nio.file.NoSuchFileException") =>
        {
            // expected
        }
        Err(e) => return Err(e),
    }
    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testMassiveField`.
fn test_massive_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let w = writer(&d, iwc)?;

    let mut b = String::new();
    while b.len() <= MAX_STORED_STRING_LENGTH {
        b.push_str("x ");
    }

    let mut doc = Document::new();
    // doc.add(new TextField("big", b.toString(), Field.Store.YES));
    doc.add(Box::new(StoredField::new_string("big", b.clone())?));
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        w.add_document(&doc)
    });
    assert_equals(
        &Some(format!(
            "stored field \"big\" is too large ({} characters) to store",
            b.len()
        ))
        .as_deref(),
        &e.message(),
    );

    // make sure writer is still usable:
    let mut doc2 = Document::new();
    doc2.add(Box::new(StringField::new(
        "id",
        "foo".to_string(),
        Store::YES,
    )?));
    w.add_document(&doc2)?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.num_docs());
    r.close()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testRecordsIndexCreatedVersion`.
fn test_records_index_created_version() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
    w.commit()?;
    w.close()?;
    assert_equals(
        &i32::from(Version::LATEST.major),
        &SegmentInfos::read_latest_commit(Arc::clone(&d))?.index_created_version_major(),
    );
    drop(d);
    dir.close()
}

/// `indexDocsForMultipleDWPTs(w)`: three threads index the same number of
/// documents at once, so that the writer holds more than one DWPT.
fn index_docs_for_multiple_dwpts(w: &Arc<IndexWriter>) -> Result<i32> {
    const THREADS: usize = 3;
    let latch = Arc::new(CountDownLatch::new(THREADS as i64));
    let num_docs_per_thread = 10 + random().next_int_bound(30);
    // ensure we have more than on thread state
    let mut threads = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let latch = Arc::clone(&latch);
        let w = Arc::clone(w);
        let t = Thread::unnamed(move || {
            latch.count_down();
            latch.await_();
            for _ in 0..num_docs_per_thread {
                let result = (|| -> Result<i64> {
                    let mut doc = Document::new();
                    doc.add(Box::new(StringField::new(
                        "id",
                        "foo".to_string(),
                        Store::YES,
                    )?));
                    w.add_document(&doc)
                })();
                if let Err(e) = result {
                    rethrow_java(assertion_error_of(e));
                }
            }
        });
        t.start();
        threads.push(t);
    }
    for t in &threads {
        join(t);
    }
    Ok(num_docs_per_thread * THREADS as i32)
}

/// `new AssertionError(e)`.
fn assertion_error_of(e: LuceneError) -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.AssertionError",
        message: Some(e.to_string()),
        cause: Some(Arc::new(e)),
    }
}

/// `t.join()`, rethrowing what the thread threw.
fn join(t: &Thread) {
    if let Err(payload) = t.join() {
        std::panic::resume_unwind(payload);
    }
}

/// `w.docWriter.flushControl.findLargestNonPendingWriter()`.
fn largest_non_pending_writer(w: &IndexWriter) -> Arc<DocumentsWriterPerThread> {
    w.get_docs_writer()
        .flush_control()
        .find_largest_non_pending_writer()
        .unwrap_or_else(|| npe("largestNonPendingWriter"))
}

/// Port of `TestIndexWriter.testFlushLargestWriter`.
fn test_flush_largest_writer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, IndexWriterConfig::new())?;
    let num_docs = index_docs_for_multiple_dwpts(&w)?;
    let largest_non_pending_writer = largest_non_pending_writer(&w);
    Assert::assert_false(largest_non_pending_writer.is_flush_pending());

    let num_ram_docs = w.num_ram_docs()?;
    let num_docs_in_dwpt = largest_non_pending_writer.num_docs_in_ram();
    Assert::assert_true(w.flush_next_buffer()?);
    Assert::assert_true(largest_non_pending_writer.has_flushed());
    assert_equals(&(num_ram_docs - num_docs_in_dwpt), &w.num_ram_docs()?);

    // make sure it's not locked
    DocumentsWriterPerThread::lock(&largest_non_pending_writer).unlock();
    if random().next_boolean() {
        w.commit()?;
    }
    let reader = directory_reader::open_with_writer_flags(&*w, true, true)?;
    assert_equals(&num_docs, &reader.num_docs());
    reader.close()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testNeverCheckOutOnFullFlush`.
fn test_never_check_out_on_full_flush() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, IndexWriterConfig::new())?;
    index_docs_for_multiple_dwpts(&w)?;
    let largest_non_pending_writer = largest_non_pending_writer(&w);
    Assert::assert_false(largest_non_pending_writer.is_flush_pending());
    Assert::assert_false(largest_non_pending_writer.has_flushed());
    let flush_control = w.get_docs_writer().flush_control();
    let thread_pool_size = w.get_docs_writer().per_thread_pool().size();
    flush_control.mark_for_full_flush()?;
    let documents_writer_per_thread = flush_control.checkout_largest_non_pending_writer();
    assert_null(&documents_writer_per_thread.map(|_| ()));
    assert_equals(&thread_pool_size, &flush_control.num_queued_flushes());
    flush_control.abort_full_flushes()?;
    assert_null_msg(
        "was aborted",
        &flush_control
            .checkout_largest_non_pending_writer()
            .map(|_| ()),
    );
    assert_equals(&0, &flush_control.num_queued_flushes());
    w.close()?;
    drop(d);
    dir.close()
}

/// The anonymous `FlushPolicy` of `testApplyDeletesWithoutFlushes`: it asks
/// for all deletes to be applied while `flush_deletes` is set.
#[derive(Debug)]
struct ApplyDeletesFlushPolicy {
    flush_deletes: Arc<AtomicBool>,
}

impl FlushPolicy for ApplyDeletesFlushPolicy {
    fn on_change(
        &self,
        control: &mut FlushControlHandle<'_>,
        _per_thread: Option<&DocumentsWriterPerThread>,
    ) {
        if self.flush_deletes.load(Ordering::SeqCst) {
            control.set_apply_all_deletes();
        }
    }

    fn init(&self, _config: Arc<LiveIndexWriterConfig>) {}
}

/// Port of `TestIndexWriter.testApplyDeletesWithoutFlushes`.
fn test_apply_deletes_without_flushes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let index_writer_config = IndexWriterConfig::new();
        let flush_deletes = Arc::new(AtomicBool::new(false));
        index_writer_config.set_flush_policy(Arc::new(ApplyDeletesFlushPolicy {
            flush_deletes: Arc::clone(&flush_deletes),
        }))?;
        let w = writer(&d, index_writer_config)?;
        let flush_control = w.get_docs_writer().flush_control();
        assert_equals(&0, &flush_control.delete_bytes_used());
        w.delete_documents_by_terms(vec![Term::from_text("foo", "bar")])?;
        let bytes_used = flush_control.delete_bytes_used();
        Assert::assert_true_msg(&format!("{bytes_used} > 0"), bytes_used > 0);
        w.delete_documents_by_terms(vec![Term::from_text("foo", "baz")])?;
        let bytes_used = flush_control.delete_bytes_used();
        Assert::assert_true_msg(&format!("{bytes_used} > 0"), bytes_used > 0);
        assert_equals(&2, &w.get_buffered_delete_terms_size());
        assert_equals(&0, &w.get_flush_deletes_count());
        flush_deletes.store(true, Ordering::SeqCst);
        w.delete_documents_by_terms(vec![Term::from_text("foo", "bar")])?;
        assert_equals(&0, &flush_control.delete_bytes_used());
        assert_equals(&1, &w.get_flush_deletes_count());
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// A document with `newField("id", id, storedTextType)`.
fn stored_id_doc(id: &str) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "id",
        id,
        &stored_text_type(),
    )));
    doc
}

/// Port of `TestIndexWriter.testDeletesAppliedOnFlush`.
fn test_deletes_applied_on_flush() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, IndexWriterConfig::new())?;
        let flush_control = w.get_docs_writer().flush_control();
        let doc = stored_id_doc("1");
        w.add_document(&doc)?;
        w.update_document(Term::from_text("id", "1"), &doc)?;
        let delete_bytes_used = flush_control.delete_bytes_used();
        Assert::assert_true_msg(
            &format!("deletedBytesUsed: {delete_bytes_used}"),
            delete_bytes_used > 0,
        );
        assert_equals(&0, &w.get_flush_deletes_count());
        Assert::assert_true(w.flush_next_buffer()?);
        assert_equals(&1, &w.get_flush_deletes_count());
        assert_equals(&0, &flush_control.delete_bytes_used());
        w.delete_all()?;
        w.commit()?;
        assert_equals(&2, &w.get_flush_deletes_count());
        if random().next_boolean() {
            w.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
        } else {
            w.update_doc_values(Term::from_text("id", "1"), &[soft_field("foo", 1)?])?;
        }
        let delete_bytes_used = flush_control.delete_bytes_used();
        Assert::assert_true_msg(
            &format!("deletedBytesUsed: {delete_bytes_used}"),
            delete_bytes_used > 0,
        );
        w.add_document(&stored_id_doc("5"))?;
        Assert::assert_true(w.flush_next_buffer()?);
        assert_equals(&0, &flush_control.delete_bytes_used());
        assert_equals(&3, &w.get_flush_deletes_count());
        w.close()?;
    }
    {
        let w = RandomIndexWriter::new_with_config(
            &*random(),
            Arc::clone(&d),
            IndexWriterConfig::new(),
        )?;
        let num_docs = random().next_int_range(1, 100);
        for i in 0..num_docs {
            w.add_document(&stored_id_doc(&i.to_string()))?;
        }
        for i in 0..num_docs {
            if random().next_boolean() {
                w.update_document(
                    Term::from_text("id", &i.to_string()),
                    &stored_id_doc(&i.to_string()),
                )?;
            }
        }

        let delete_bytes_used = w.w().get_docs_writer().flush_control().delete_bytes_used();
        if delete_bytes_used > 0 {
            Assert::assert_true(w.w().flush_next_buffer()?);
            assert_equals(
                &0,
                &w.w().get_docs_writer().flush_control().delete_bytes_used(),
            );
        }
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testHoldLockOnLargestWriter`.
fn test_hold_lock_on_largest_writer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, IndexWriterConfig::new())?;
    let num_docs = index_docs_for_multiple_dwpts(&w)?;
    let largest_non_pending_writer = largest_non_pending_writer(&w);
    Assert::assert_false(largest_non_pending_writer.is_flush_pending());
    Assert::assert_false(largest_non_pending_writer.has_flushed());

    let wait = Arc::new(CountDownLatch::new(1));
    let locked = Arc::new(CountDownLatch::new(1));
    let lock_thread = {
        let dwpt = Arc::clone(&largest_non_pending_writer);
        let wait = Arc::clone(&wait);
        let locked = Arc::clone(&locked);
        Thread::unnamed(move || {
            let guard = DocumentsWriterPerThread::lock(&dwpt);
            locked.count_down();
            wait.await_();
            guard.unlock();
        })
    };
    lock_thread.start();
    let flush_thread = {
        let w = Arc::clone(&w);
        let locked = Arc::clone(&locked);
        Thread::unnamed(move || {
            locked.await_();
            match w.flush_next_buffer() {
                Ok(flushed) => Assert::assert_true(flushed),
                Err(e) => rethrow_java(assertion_error_of(e)),
            }
        })
    };
    flush_thread.start();

    locked.await_();
    // access a synced method to ensure we never lock while we hold the flush control monitor
    w.get_docs_writer().flush_control().active_bytes();
    wait.count_down();
    join(&lock_thread);
    join(&flush_thread);

    Assert::assert_true_msg(
        "largest DWPT should be flushed",
        largest_non_pending_writer.has_flushed(),
    );
    // make sure it's not locked
    DocumentsWriterPerThread::lock(&largest_non_pending_writer).unlock();
    if random().next_boolean() {
        w.commit()?;
    }
    let reader = directory_reader::open_with_writer_flags(&*w, true, true)?;
    assert_equals(&num_docs, &reader.num_docs());
    reader.close()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// The anonymous `MockDirectoryWrapper.Failure` of
/// `testCheckPendingFlushPostUpdate`: records the name of each thread that
/// writes a file from `DocumentsWriterPerThread.flush`.
struct FlushingThreadsFailure {
    flushing_threads: Arc<Mutex<HashSet<String>>>,
    do_fail: AtomicBool,
}

impl Failure for FlushingThreadsFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if LuceneTestCase::call_stack_contains_method::<DocumentsWriterPerThread>("flush") {
            self.flushing_threads
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(Thread::current().name());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriter.testCheckPendingFlushPostUpdate`.
fn test_check_pending_flush_post_update() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    let flushing_threads: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    dir.fail_on(Arc::new(FlushingThreadsFailure {
        flushing_threads: Arc::clone(&flushing_threads),
        do_fail: AtomicBool::new(false),
    }));
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::new();
    iwc.set_check_pending_flush_update(false)?;
    iwc.set_max_buffered_docs(i32::MAX)?;
    iwc.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)?;
    let w = writer(&d, iwc)?;
    let done = Arc::new(AtomicBool::new(false));
    let num_threads = 2 + random().next_int_bound(3);
    let latch = Arc::new(CountDownLatch::new(i64::from(num_threads)));
    let mut indexing_threads: HashSet<String> = HashSet::new();
    let mut threads = Vec::new();
    for _ in 0..num_threads {
        let latch = Arc::clone(&latch);
        let done = Arc::clone(&done);
        let w = Arc::clone(&w);
        let t = Thread::unnamed(move || {
            latch.count_down();
            let mut num_docs = 0;
            while !done.load(Ordering::SeqCst) {
                let result = (|| -> Result<i64> {
                    let mut doc = Document::new();
                    doc.add(Box::new(StringField::new(
                        "id",
                        "foo".to_string(),
                        Store::YES,
                    )?));
                    w.add_document(&doc)
                })();
                if let Err(e) = result {
                    rethrow_java(assertion_error_of(e));
                }
                let n = num_docs;
                num_docs += 1;
                if n % 10 == 0 {
                    std::thread::yield_now();
                }
            }
        });
        indexing_threads.insert(t.name());
        t.start();
        threads.push(t);
    }
    latch.await_();
    let result = (|| -> Result<()> {
        let num_iters = if LuceneTestCase::rarely() {
            1 + random().next_int_bound(5)
        } else {
            1
        };
        for _ in 0..num_iters {
            wait_for_docs_in_buffers(&w, 2.min(threads.len()));
            w.commit()?;
            let mut ft = flushing_threads.lock().unwrap_or_else(|e| e.into_inner());
            Assert::assert_true_msg(&format!("{ft:?}"), ft.contains(&Thread::current().name()));
            ft.retain(|n| indexing_threads.contains(n));
            Assert::assert_true_msg(&format!("{ft:?}"), ft.is_empty());
        }
        w.get_config().set_check_pending_flush_update(true);
        let mut num_iters = 0;
        loop {
            Assert::assert_false_msg("should finish in less than 100 iterations", {
                let n = num_iters;
                num_iters += 1;
                n >= 100
            });
            wait_for_docs_in_buffers(&w, 2.min(threads.len()));
            w.flush()?;
            let mut ft = flushing_threads.lock().unwrap_or_else(|e| e.into_inner());
            ft.retain(|n| indexing_threads.contains(n));
            if !ft.is_empty() {
                break;
            }
        }
        Ok(())
    })();
    done.store(true, Ordering::SeqCst);
    for t in &threads {
        join(t);
    }
    let closed = w.close();
    drop((w, d));
    result.and(closed).and(dir.close())
}

/// `waitForDocsInBuffers(w, buffersWithDocs)`: wait until at least N DWPTs
/// have a doc in order to observe who flushes the segments.
fn wait_for_docs_in_buffers(w: &IndexWriter, buffers_with_docs: usize) {
    loop {
        let mut num_states_with_docs = 0;
        let per_thread_pool = w.get_docs_writer().per_thread_pool();
        for dwpt in per_thread_pool.snapshot() {
            let guard = DocumentsWriterPerThread::lock(&dwpt);
            if guard.num_docs_in_ram() > 1 {
                num_states_with_docs += 1;
            }
            guard.unlock();
        }
        if num_states_with_docs >= buffers_with_docs {
            return;
        }
    }
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

/// A document with `id` and `version` string fields.
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

/// `new IndexSearcher(reader).search(new TermQuery(term), 10)`.
fn search_term(reader: &Arc<dyn DirectoryReader>, term: Term) -> Result<rucene::search::TopDocs> {
    let searcher = IndexSearcher::new(ir(reader))?;
    searcher.search(Arc::new(TermQuery::new(term)), 10)
}

/// `assertNull(((SegmentReader) leaf).getHardLiveDocs())` for every leaf.
fn assert_no_hard_live_docs(reader: &Arc<dyn DirectoryReader>) -> Result<()> {
    for context in ir(reader).leaves()? {
        let leaf = context.leaf_reader();
        let sr = leaf.as_segment_reader().unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
        });
        assert_null(&sr.get_hard_live_docs().map(|_| ()));
    }
    Ok(())
}

/// Port of `TestIndexWriter.testSoftUpdateDocuments`.
fn test_soft_update_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    iwc.set_soft_deletes_field(Some("soft_delete".to_string()));
    let w = writer(&d, iwc)?;
    expect_throws(IAE, || {
        w.soft_update_document(None, &Document::new(), &[soft_field("soft_delete", 1)?])
    });

    expect_throws(IAE, || {
        w.soft_update_document(Term::from_text("id", "1"), &Document::new(), &[])
    });

    expect_throws(IAE, || {
        w.soft_update_documents(None, &[Document::new()], &[soft_field("soft_delete", 1)?])
    });

    expect_throws(IAE, || {
        w.soft_update_documents(Term::from_text("id", "1"), &[Document::new()], &[])
    });

    w.add_document(&id_version_doc("1", "1")?)?;
    let doc = id_version_doc("1", "2")?;
    let field = soft_field("soft_delete", 1)?;
    w.soft_update_document(Term::from_text("id", "1"), &doc, &[field])?;
    let reader = directory_reader::open_with_writer(&*w)?;
    assert_equals(&2, &reader.doc_freq(&Term::from_text("id", "1"))?);
    let top_docs = search_term(&reader, Term::from_text("id", "1"))?;
    assert_equals(&1, &top_docs.total_hits.value());
    let document = reader
        .stored_fields()?
        .document(top_docs.score_docs[0].doc)?;
    assert_equals(&Some("2".to_string()), &document.get("version"));

    // update the on-disk version
    let doc = id_version_doc("1", "3")?;
    let field = soft_field("soft_delete", 1)?;
    w.soft_update_document(
        Term::from_text("id", "1"),
        &doc,
        &[copy_soft_field(&field)?],
    )?;
    let old_reader = reader;
    let reader = directory_reader::open_if_changed_with_writer(Arc::clone(&old_reader), &*w, true)?
        .unwrap_or_else(|| npe("reader"));
    assert_not_same(&reader, &old_reader);
    old_reader.close()?;
    let top_docs = search_term(&reader, Term::from_text("id", "1"))?;
    assert_equals(&1, &top_docs.total_hits.value());
    let document = reader
        .stored_fields()?
        .document(top_docs.score_docs[0].doc)?;
    assert_equals(&Some("3".to_string()), &document.get("version"));

    // now delete it
    w.update_doc_values(Term::from_text("id", "1"), &[field])?;
    let old_reader = reader;
    let reader = directory_reader::open_if_changed_with_writer(Arc::clone(&old_reader), &*w, true)?;
    assert_not_null(&reader);
    let reader = reader.unwrap_or_else(|| npe("reader"));
    assert_not_same(&reader, &old_reader);
    old_reader.close()?;
    let top_docs = search_term(&reader, Term::from_text("id", "1"))?;
    assert_equals(&0, &top_docs.total_hits.value());
    let mut num_soft_deleted = 0;
    for info in w.clone_segment_infos()?.as_list() {
        num_soft_deleted += info.get_soft_del_count();
    }
    let doc_stats: DocStats = w.get_doc_stats()?;
    assert_equals(&(doc_stats.max_doc - doc_stats.num_docs), &num_soft_deleted);
    assert_no_hard_live_docs(&reader)?;
    w.close()?;
    reader.close()?;
    drop(d);
    dir.close()
}

/// A copy of a soft-deletes doc-values field, for a second call with the
/// same `Field` object.
fn copy_soft_field(field: &Field) -> Result<Field> {
    let value = match field.numeric_value() {
        Some(NumericValue::Long(v)) => v,
        _ => npe("numericValue"),
    };
    soft_field(field.name(), value)
}

/// Port of `TestIndexWriter.testSoftUpdatesConcurrently`.
fn test_soft_updates_concurrently() -> Result<()> {
    soft_updates_concurrently(false)
}

/// Port of `TestIndexWriter.testSoftUpdatesConcurrentlyMixedDeletes`.
fn test_soft_updates_concurrently_mixed_deletes() -> Result<()> {
    soft_updates_concurrently(true)
}

/// The anonymous `FilterCodecReader` of `softUpdatesConcurrently`: every
/// document is live.
struct AllLiveCodecReader {
    base: FilterCodecReader,
    r#in: Arc<dyn CodecReader>,
}

impl std::fmt::Display for AllLiveCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.base, f)
    }
}

impl std::fmt::Debug for AllLiveCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllLiveCodecReader").finish_non_exhaustive()
    }
}

impl LeafReader for AllLiveCodecReader {
    crate::forward_leaf_reader!(filter base;
        core,
        max_doc,
        do_close,
        check_integrity,
        get_meta_data,
        get_field_infos,
        term_vectors,
        stored_fields,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_point_values,
    );

    fn as_codec_reader(self: Arc<Self>) -> Option<Arc<dyn CodecReader>> {
        Some(self)
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_reader_cache_helper()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        None // everything is live
    }

    fn num_docs(&self) -> i32 {
        LeafReader::max_doc(self)
    }
}

impl CodecReader for AllLiveCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.base.get_fields_reader()
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.base.get_term_vectors_reader()
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.base.get_norms_reader()
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.base.get_doc_values_reader()
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        self.base.get_postings_reader()
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.base.get_points_reader()
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        self.base.get_vector_reader()
    }

    /// `this instanceof FilterCodecReader`: the delegate.
    fn filter_codec_reader_delegate(&self) -> Option<&Arc<dyn CodecReader>> {
        Some(&self.r#in)
    }
}

/// The anonymous `OneMergeWrappingMergePolicy` subclass of
/// `softUpdatesConcurrently`: its `numDeletesToMerge` counts no deletes until
/// `merge_away_soft_deletes` is set.
struct SoftDeletesWrappingMergePolicy {
    wrapping: OneMergeWrappingMergePolicy,
    merge_away_soft_deletes: Arc<AtomicBool>,
}

impl std::fmt::Display for SoftDeletesWrappingMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.wrapping, f)
    }
}

impl MergePolicy for SoftDeletesWrappingMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.wrapping
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<Option<MergeSpecification>> {
        self.wrapping.find_merges_for_readers(readers)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.wrapping.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.wrapping
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.wrapping
            .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.wrapping
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.wrapping.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.wrapping.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.wrapping.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.wrapping.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.wrapping.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.wrapping.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.wrapping.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        if self.merge_away_soft_deletes.load(Ordering::SeqCst) {
            self.wrapping.num_deletes_to_merge_default(info, del_count)
        } else {
            0
        }
    }

    /// The override.
    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
    ) -> Result<i32> {
        if self.merge_away_soft_deletes.load(Ordering::SeqCst) {
            self.wrapping
                .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
        } else {
            Ok(0)
        }
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.wrapping.max_full_flush_merge_size()
    }
}

/// `softUpdatesConcurrently(mixDeletes)`.
fn soft_updates_concurrently(mix_deletes: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_soft_deletes_field(Some("soft_delete".to_string()));
    let merge_away_soft_deletes = Arc::new(AtomicBool::new(random().next_boolean()));
    if !mix_deletes {
        let away = Arc::clone(&merge_away_soft_deletes);
        let wrapping = OneMergeWrappingMergePolicy::new(
            index_writer_config.merge_policy(),
            Arc::new(move |towrap: OneMerge| {
                let mut merge =
                    OneMerge::new(towrap.segments.clone()).unwrap_or_else(|e| rethrow_java(e));
                let away = Arc::clone(&away);
                merge.set_wrap_for_merge(Arc::new(move |reader: Arc<dyn CodecReader>| {
                    if away.load(Ordering::SeqCst) {
                        towrap.wrap_for_merge(reader)
                    } else {
                        let wrapped = towrap.wrap_for_merge(reader)?;
                        Ok(Arc::new(AllLiveCodecReader {
                            base: FilterCodecReader::new(Arc::clone(&wrapped)),
                            r#in: wrapped,
                        }) as Arc<dyn CodecReader>)
                    }
                }));
                merge
            }),
        );
        index_writer_config.set_merge_policy(Some(Arc::new(SoftDeletesWrappingMergePolicy {
            wrapping,
            merge_away_soft_deletes: Arc::clone(&merge_away_soft_deletes),
        })))?;
    }
    let w = writer(&d, index_writer_config)?;
    let num_threads = 2 + random().next_int_bound(3);
    let start_latch = Arc::new(CountDownLatch::new(1));
    let started = Arc::new(CountDownLatch::new(i64::from(num_threads)));
    let update_several_docs = random().next_boolean();
    let ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut threads = Vec::new();
    for _ in 0..num_threads {
        let w = Arc::clone(&w);
        let start_latch = Arc::clone(&start_latch);
        let started = Arc::clone(&started);
        let ids = Arc::clone(&ids);
        let t = Thread::unnamed(move || {
            let result = (|| -> Result<()> {
                started.count_down();
                start_latch.await_();
                for _ in 0..100 {
                    let id = random().next_int_bound(10).to_string();
                    if update_several_docs {
                        let mut doc = Document::new();
                        doc.add(Box::new(StringField::new("id", id.clone(), Store::YES)?));
                        let docs = [copy_document(&doc)?, doc];
                        if mix_deletes && random().next_boolean() {
                            if random().next_boolean() {
                                w.update_documents(Term::from_text("id", &id), &docs)?;
                            } else {
                                w.update_documents_by_query(
                                    Some(Arc::new(TermQuery::new(Term::from_text("id", &id)))),
                                    &docs,
                                )?;
                            }
                        } else {
                            w.soft_update_documents(
                                Term::from_text("id", &id),
                                &docs,
                                &[soft_field("soft_delete", 1)?],
                            )?;
                        }
                    } else {
                        let mut doc = Document::new();
                        doc.add(Box::new(StringField::new("id", id.clone(), Store::YES)?));
                        if mix_deletes && random().next_boolean() {
                            w.update_document(Term::from_text("id", &id), &doc)?;
                        } else {
                            w.soft_update_document(
                                Term::from_text("id", &id),
                                &doc,
                                &[soft_field("soft_delete", 1)?],
                            )?;
                        }
                    }
                    ids.lock().unwrap_or_else(|e| e.into_inner()).insert(id);
                }
                Ok(())
            })();
            if let Err(e) = result {
                rethrow_java(assertion_error_of(e));
            }
        });
        t.start();
        threads.push(t);
    }
    started.await_();
    start_latch.count_down();

    for t in &threads {
        join(t);
    }
    let ids: HashSet<String> = ids.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let reader = directory_reader::open_with_writer(&*w)?;
    for id in &ids {
        let top_docs = search_term(&reader, Term::from_text("id", id))?;
        if update_several_docs {
            assert_equals(&2, &top_docs.total_hits.value());
            assert_equals(
                &(top_docs.score_docs[0].doc - top_docs.score_docs[1].doc).abs(),
                &1,
            );
        } else {
            assert_equals(&1, &top_docs.total_hits.value());
        }
    }
    if !mix_deletes {
        assert_no_hard_live_docs(&reader)?;
    }
    merge_away_soft_deletes.store(true, Ordering::SeqCst);
    w.add_document(&Document::new())?; // add a dummy doc to trigger a segment here
    w.flush()?;
    w.force_merge(1)?;
    let old_reader = reader;
    let reader =
        match directory_reader::open_if_changed_with_writer(Arc::clone(&old_reader), &*w, true)? {
            Some(reader) => {
                old_reader.close()?;
                assert_not_same(&old_reader, &reader);
                reader
            }
            None => old_reader,
        };
    for id in &ids {
        let expected = if update_several_docs { 2 } else { 1 };
        assert_equals(&expected, &reader.doc_freq(&Term::from_text("id", id))?);
    }
    let mut num_soft_deleted = 0;
    for info in w.clone_segment_infos()?.as_list() {
        num_soft_deleted += info.get_soft_del_count() + info.get_del_count();
    }
    let doc_stats = w.get_doc_stats()?;
    assert_equals(&(doc_stats.max_doc - doc_stats.num_docs), &num_soft_deleted);
    w.commit()?;
    {
        let dir_reader = directory_reader::open(Arc::clone(&d))?;
        let mut del_count = 0;
        for ctx in ir(&dir_reader).leaves()? {
            let leaf = ctx.leaf_reader();
            let sr = leaf.as_segment_reader().unwrap_or_else(|| {
                Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
            });
            let segment_info = sr.get_segment_info();
            del_count += segment_info.get_soft_del_count() + segment_info.get_del_count();
        }
        assert_equals(&num_soft_deleted, &del_count);
        dir_reader.close()?;
    }
    reader.close()?;
    w.close()?;
    drop(d);
    dir.close()
}

/// A copy of a document of `StringField`s, for `Arrays.asList(doc, doc)`.
fn copy_document(doc: &Document) -> Result<Document> {
    let mut copy = Document::new();
    for field in doc.get_fields() {
        copy.add(Box::new(StringField::new(
            field.name(),
            field.string_value().unwrap_or_default(),
            if field.field_type().stored() {
                Store::YES
            } else {
                Store::NO
            },
        )?));
    }
    Ok(copy)
}

/// The anonymous `FilterDirectory` of `testDeleteHappensBeforeWhileFlush`:
/// `createOutput` called from `IndexingChain.flush` counts `inFlush` down and
/// waits for `latch`.
struct FlushBlockingDirectory {
    inner: Arc<dyn Directory>,
    latch: Arc<CountDownLatch>,
    in_flush: Arc<CountDownLatch>,
}

impl Directory for FlushBlockingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }
    /// The override.
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        if LuceneTestCase::call_stack_contains_method::<DefaultIndexingChain>("flush") {
            self.in_flush.count_down();
            self.latch.await_();
        }
        self.inner.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }
    /// `FilterDirectory.close()`: `in.close()`, which the test's
    /// try-with-resources reaches through the `TestDirectory` it closes.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }
    fn directory_to_string(&self) -> String {
        format!("FilterDirectory({})", self.inner.directory_to_string())
    }
}

/// Port of `TestIndexWriter.testDeleteHappensBeforeWhileFlush`.
fn test_delete_happens_before_while_flush() -> Result<()> {
    let latch = Arc::new(CountDownLatch::new(1));
    let in_flush = Arc::new(CountDownLatch::new(1));
    let mut base = LuceneTestCase::new_directory();
    let dir: Arc<dyn Directory> = Arc::new(FlushBlockingDirectory {
        inner: base.as_directory(),
        latch: Arc::clone(&latch),
        in_flush: Arc::clone(&in_flush),
    });
    {
        let w = writer(&dir, LuceneTestCase::new_index_writer_config())?;
        let mut document = Document::new();
        document.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::YES,
        )?));
        w.add_document(&document)?;
        let t = {
            let w = Arc::clone(&w);
            let latch = Arc::clone(&latch);
            let in_flush = Arc::clone(&in_flush);
            Thread::unnamed(move || {
                let result = (|| -> Result<()> {
                    in_flush.await_();
                    w.get_docs_writer().flush_control().set_apply_all_deletes();
                    if random().next_boolean() {
                        let mut document = Document::new();
                        document.add(Box::new(StringField::new(
                            "id",
                            "1".to_string(),
                            Store::YES,
                        )?));
                        w.update_document(Term::from_text("id", "1"), &document)?;
                    } else {
                        w.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
                    }
                    Ok(())
                })();
                latch.count_down();
                if let Err(e) = result {
                    rethrow_java(assertion_error_of(e));
                }
            })
        };
        t.start();
        {
            let reader = directory_reader::open_with_writer(&*w)?;
            assert_equals(&1, &reader.num_docs());
            reader.close()?;
        }
        join(&t);
        w.close()?;
    }
    drop(dir);
    base.close()
}

/// `assertFiles(writer)`.
fn assert_files(writer: &IndexWriter) -> Result<()> {
    let filter = |file: &String| !file.starts_with("segments") && file != "write.lock";
    // remove segment files we don't know if we have committed and what is kept around
    let seg_files: HashSet<String> = writer
        .clone_segment_infos()?
        .files(true)?
        .into_iter()
        .filter(filter)
        .collect();
    let dir_files: HashSet<String> = writer
        .get_directory()
        .list_all()?
        .into_iter()
        .filter(|file| !ExtrasFS::is_extra(file)) // ExtraFS might add files, ignore them
        .filter(filter)
        .collect();
    assert_equals(&seg_files.len(), &dir_files.len());
    Ok(())
}

/// `newIndexWriterConfig()` with `setRAMBufferSizeMB(Integer.MAX_VALUE)` and
/// `setMaxBufferedDocs(2)`.
fn no_auto_flush_iwc() -> Result<IndexWriterConfig> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_ram_buffer_size_mb(f64::from(i32::MAX))?;
    config.set_max_buffered_docs(2)?; // no auto flush
    Ok(config)
}

/// A document with one stored `StringField("id", id)`.
fn string_id_doc(id: &str) -> Result<Document> {
    let mut d = Document::new();
    d.add(Box::new(StringField::new(
        "id",
        id.to_string(),
        Store::YES,
    )?));
    Ok(d)
}

/// Port of `TestIndexWriter.testFullyDeletedSegmentsReleaseFiles`.
fn test_fully_deleted_segments_release_files() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, no_auto_flush_iwc()?)?;
    w.add_document(&string_id_doc("doc-0")?)?;
    w.flush()?;
    w.add_document(&string_id_doc("doc-1")?)?;
    w.delete_documents_by_terms(vec![Term::from_text("id", "doc-1")])?;
    assert_equals(&1, &w.clone_segment_infos()?.size());
    w.flush()?;
    assert_equals(&1, &w.clone_segment_infos()?.size());
    w.commit()?;
    assert_files(&w)?;
    assert_equals(&1, &w.clone_segment_infos()?.size());
    w.close()?;
    drop((w, d));
    dir.close()
}

/// Port of `TestIndexWriter.testSegmentInfoIsSnapshot`.
fn test_segment_info_is_snapshot() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, no_auto_flush_iwc()?)?;
    w.add_document(&string_id_doc("doc-0")?)?;
    w.add_document(&string_id_doc("doc-1")?)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let leaves = ir(&reader).leaves()?;
    let leaf = leaves[0].leaf_reader();
    let sr = leaf.as_segment_reader().unwrap_or_else(|| {
        Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
    });
    let segment_info = sr.get_segment_info();
    let original_info = sr.get_original_segment_info();
    assert_equals(&0, &original_info.get_del_count());
    assert_equals(&0, &segment_info.get_del_count());
    w.delete_documents_by_terms(vec![Term::from_text("id", "doc-0")])?;
    w.commit()?;
    assert_equals(&0, &segment_info.get_del_count());
    assert_equals(&1, &original_info.get_del_count());
    drop(leaves);
    reader.close()?;
    w.close()?;
    drop((w, d));
    dir.close()
}

/// `IndexWriter.readFieldInfos(si)` checked to hold `field` as its
/// soft-deletes field.
fn assert_soft_deletes_field(si: &SegmentCommitInfo, field: &str) -> Result<()> {
    let field_infos = IndexWriter::read_field_infos(si)?;
    assert_equals(&Some(field), &field_infos.get_soft_deletes_field());
    Assert::assert_true(
        field_infos
            .field_info(field)
            .unwrap_or_else(|| npe("fieldInfo"))
            .is_soft_deletes_field(),
    );
    Ok(())
}

/// `newIndexWriterConfig().setSoftDeletesField(field)`.
fn soft_deletes_iwc(field: &str) -> IndexWriterConfig {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_soft_deletes_field(Some(field.to_string()));
    iwc
}

/// Port of `TestIndexWriter.testPreventChangingSoftDeletesField`.
fn test_prevent_changing_soft_deletes_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, soft_deletes_iwc("my_deletes"))?;
    w.add_document(&id_version_doc("1", "1")?)?;
    let v2 = id_version_doc("1", "2")?;
    w.soft_update_document(
        Term::from_text("id", "1"),
        &v2,
        &[soft_field("my_deletes", 1)?],
    )?;
    w.commit()?;
    w.close()?;
    for si in SegmentInfos::read_latest_commit(Arc::clone(&d))?.as_list() {
        assert_soft_deletes_field(si, "my_deletes")?;
    }

    let illegal_error = expect_throws(IAE, || writer(&d, soft_deletes_iwc("your_deletes")));
    assert_equals(
        &Some(
            "cannot configure [your_deletes] as soft-deletes; \
             this index uses [my_deletes] as soft-deletes already",
        ),
        &illegal_error.message(),
    );

    let soft_delete_config = soft_deletes_iwc("my_deletes");
    soft_delete_config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "my_deletes",
        Arc::new(|| Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>),
        LuceneTestCase::new_merge_policy(),
    ))))?;
    let w = writer(&d, soft_delete_config)?;
    let mut tombstone = Document::new();
    tombstone.add(Box::new(StringField::new(
        "id",
        "tombstone".to_string(),
        Store::YES,
    )?));
    tombstone.add(Box::new(NumericDocValuesField::new("my_deletes", 1)));
    w.add_document(&tombstone)?;
    w.flush()?;
    for si in w.clone_segment_infos()?.as_list() {
        assert_soft_deletes_field(si, "my_deletes")?;
    }
    w.close()?;
    // reopen writer without soft-deletes field should be prevented
    let reopen_error = expect_throws(IAE, || {
        writer(&d, LuceneTestCase::new_index_writer_config())
    });
    assert_equals(
        &Some(
            "this index has [my_deletes] as soft-deletes already \
             but soft-deletes field is not configured in IWC",
        ),
        &reopen_error.message(),
    );
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testPreventAddingIndexesWithDifferentSoftDeletesField`.
fn test_prevent_adding_indexes_with_different_soft_deletes_field() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let d1 = dir1.as_directory();
    let w1 = writer(&d1, soft_deletes_iwc("soft_deletes_1"))?;
    for i in 0..2 {
        let d = id_version_doc("1", &i.to_string())?;
        w1.soft_update_document(
            Term::from_text("id", "1"),
            &d,
            &[soft_field("soft_deletes_1", 1)?],
        )?;
    }
    w1.commit()?;
    w1.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let d2 = dir2.as_directory();
    let w2 = writer(&d2, soft_deletes_iwc("soft_deletes_2"))?;
    let error = expect_throws(IAE, || w2.add_indexes_dirs(&[Arc::clone(&d1)]));
    assert_equals(
        &Some(
            "cannot configure [soft_deletes_2] as soft-deletes; this index uses [soft_deletes_1] as soft-deletes already",
        ),
        &error.message(),
    );
    w2.close()?;

    let mut dir3 = LuceneTestCase::new_directory();
    let d3 = dir3.as_directory();
    let config = soft_deletes_iwc("soft_deletes_1");
    let w3 = writer(&d3, config)?;
    w3.add_indexes_dirs(&[Arc::clone(&d1)])?;
    for si in w3.clone_segment_infos()?.as_list() {
        let soft_delete_field = IndexWriter::read_field_infos(si)?
            .field_info("soft_deletes_1")
            .unwrap_or_else(|| npe("softDeleteField"))
            .clone();
        Assert::assert_true(soft_delete_field.is_soft_deletes_field());
    }
    w3.close()?;
    drop((w1, w2, w3, d1, d2, d3));
    let r1 = dir1.close();
    let r2 = dir2.close();
    let r3 = dir3.close();
    r1.and(r2).and(r3)
}

/// Port of `TestIndexWriter.testNotAllowUsingExistingFieldAsSoftDeletes`.
fn test_not_allow_using_existing_field_as_soft_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
    for _ in 0..2 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::YES,
        )?));
        if random().next_boolean() {
            doc.add(Box::new(NumericDocValuesField::new("dv_field", 1)));
            w.update_document(Term::from_text("id", "1"), &doc)?;
        } else {
            w.soft_update_document(
                Term::from_text("id", "1"),
                &doc,
                &[soft_field("dv_field", 1)?],
            )?;
        }
    }
    w.commit()?;
    w.close()?;
    let soft_deletes_field = if random().next_boolean() {
        "id"
    } else {
        "dv_field"
    };
    let error = expect_throws(IAE, || {
        let config = soft_deletes_iwc(soft_deletes_field);
        writer(&d, config)
    });
    let expected = format!(
        "cannot configure [{soft_deletes_field}] as soft-deletes; this index uses [{soft_deletes_field}] as non-soft-deletes already"
    );
    assert_equals(&Some(expected.as_str()), &error.message());
    let config = soft_deletes_iwc("non-existing-field");
    let w = writer(&d, config)?;
    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testBrokenPayload`.
fn test_broken_payload() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;
    let mut doc = Document::new();
    let mut token = Token::with_offsets("bar", 0, 3);
    let mut evil = LuceneTestCase::new_bytes_ref_bytes(&[0u8; 1024]);
    evil.offset = 1000; // offset + length is now out of bounds.
    token.set_payload(Some(evil));
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(CannedTokenStream::new(vec![token])));
    doc.add(Box::new(TextField::new_with_token_stream("foo", stream)?));
    expect_throws("java.lang.IndexOutOfBoundsException", || {
        w.add_document(&doc)
    });
    w.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testSoftAndHardLiveDocs`.
fn test_soft_and_hard_live_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let soft_deletes_field = "soft_delete";
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_soft_deletes_field(Some(soft_deletes_field.to_string()));
    let w = writer(&d, index_writer_config)?;
    let mut unique_docs: HashSet<i32> = HashSet::new();
    for _ in 0..100 {
        let doc_id = random().next_int_bound(5);
        unique_docs.insert(doc_id);
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            doc_id.to_string(),
            Store::YES,
        )?));
        if doc_id % 2 == 0 {
            w.update_document(Term::from_text("id", &doc_id.to_string()), &doc)?;
        } else {
            w.soft_update_document(
                Term::from_text("id", &doc_id.to_string()),
                &doc,
                &[soft_field(soft_deletes_field, 0)?],
            )?;
        }
        if random().next_boolean() {
            assert_hard_live_docs(&w, &unique_docs)?;
        }
    }

    if random().next_boolean() {
        w.commit()?;
    }
    assert_hard_live_docs(&w, &unique_docs)?;

    w.close()?;
    drop((w, d));
    dir.close()
}

/// Port of `TestIndexWriter.testAbortFullyDeletedSegment`.
///
/// The anonymous `OneMergeWrappingMergePolicy` subclass, whose
/// `keepFullyDeletedSegment` returns `true`, is a
/// [`KeepFullyDeletedSegmentsMergePolicy`] over the wrapping policy: the same
/// decisions, delegated.
fn test_abort_fully_deleted_segment() -> Result<()> {
    let abort_merge_before_commit = Arc::new(AtomicBool::new(false));
    let abort = Arc::clone(&abort_merge_before_commit);
    let merge_policy =
        KeepFullyDeletedSegmentsMergePolicy::new(Arc::new(OneMergeWrappingMergePolicy::new(
            LuceneTestCase::new_merge_policy(),
            Arc::new(move |to_wrap: OneMerge| {
                let mut merge =
                    OneMerge::new(to_wrap.segments.clone()).unwrap_or_else(|e| rethrow_java(e));
                let abort = Arc::clone(&abort);
                merge.set_on_merge_complete(Arc::new(move |merge: &OneMerge| {
                    // super.onMergeComplete() does nothing
                    if abort.load(Ordering::SeqCst) {
                        merge.set_aborted();
                    }
                    Ok(())
                }));
                merge
            }),
        )));

    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_merge_policy(Some(Arc::new(merge_policy)))?;
    index_writer_config.set_commit_on_close(false);
    let w = writer(&d, index_writer_config)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "id",
        "1".to_string(),
        Store::YES,
    )?));
    w.add_document(&doc)?;
    w.flush()?;

    w.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
    abort_merge_before_commit.store(true, Ordering::SeqCst);
    w.flush()?;
    w.force_merge(1)?;
    w.close()?;
    drop((w, d));
    dir.close()
}

/// `assertHardLiveDocs(writer, uniqueDocs)`.
fn assert_hard_live_docs(w: &IndexWriter, unique_docs: &HashSet<i32>) -> Result<()> {
    let reader = directory_reader::open_with_writer(w)?;
    let result = (|| -> Result<()> {
        assert_equals(&unique_docs.len(), &(reader.num_docs() as usize));
        let leaves = ir(&reader).leaves()?;
        for ctx in &leaves {
            let leaf = ctx.leaf_reader();
            Assert::assert_true(leaf.as_segment_reader().is_some());
            let sr = leaf.as_segment_reader().unwrap_or_else(|| npe("sr"));
            if sr.get_hard_live_docs().is_some() {
                let id = sr.terms("id")?.unwrap_or_else(|| npe("id"));
                let mut iterator = id.iterator()?;
                let hard_live_docs = sr
                    .get_hard_live_docs()
                    .unwrap_or_else(|| npe("hardLiveDocs"));
                let live_docs = sr.get_live_docs().unwrap_or_else(|| npe("liveDocs"));
                for d_id in unique_docs {
                    let must_be_hard_deleted = d_id % 2 == 0;
                    if iterator.seek_exact(&LuceneTestCase::new_bytes_ref_str(&d_id.to_string()))? {
                        let mut postings = iterator.postings(None, POSTINGS_ENUM_FREQS)?;
                        while postings.next_doc()? != NO_MORE_DOCS {
                            let doc = postings.doc_id() as usize;
                            if live_docs.get(doc) {
                                Assert::assert_true(hard_live_docs.get(doc));
                            } else if must_be_hard_deleted {
                                Assert::assert_false(hard_live_docs.get(doc));
                            } else {
                                Assert::assert_true(hard_live_docs.get(doc));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    let closed = reader.close();
    result.and(closed)
}

/// Port of `TestIndexWriter.testSetIndexCreatedVersion`.
fn test_set_index_created_version() -> Result<()> {
    let latest = i32::from(Version::LATEST.major);
    let e = expect_throws(IAE, || {
        IndexWriterConfig::new()
            .set_index_created_version_major(latest + 1)
            .map(|_| ())
    });
    assert_equals(
        &Some(
            format!(
                "indexCreatedVersionMajor may not be in the future: current major version is {latest}, but got: {}",
                latest + 1
            )
            .as_str(),
        ),
        &e.message(),
    );
    let e = expect_throws(IAE, || {
        IndexWriterConfig::new()
            .set_index_created_version_major(latest - 2)
            .map(|_| ())
    });
    assert_equals(
        &Some(
            format!(
                "indexCreatedVersionMajor may not be less than the minimum supported version: {}, but got: {}",
                latest - 1,
                latest - 2
            )
            .as_str(),
        ),
        &e.message(),
    );

    for previous_major in (latest - 1)..=latest {
        for new_major in (latest - 1)..=latest {
            for open_mode in [
                OpenMode::CREATE,
                OpenMode::APPEND,
                OpenMode::CREATE_OR_APPEND,
            ] {
                let mut dir = LuceneTestCase::new_directory();
                let d = dir.as_directory();
                {
                    let iwc = LuceneTestCase::new_index_writer_config();
                    iwc.set_index_created_version_major(previous_major)?;
                    let w = writer(&d, iwc)?;
                    w.close()?;
                }
                let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
                assert_equals(&previous_major, &infos.index_created_version_major());
                {
                    let iwc = LuceneTestCase::new_index_writer_config();
                    iwc.set_open_mode(open_mode)?;
                    iwc.set_index_created_version_major(new_major)?;
                    let w = writer(&d, iwc)?;
                    w.close()?;
                }
                let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
                if open_mode == OpenMode::CREATE {
                    assert_equals(&new_major, &infos.index_created_version_major());
                } else {
                    assert_equals(&previous_major, &infos.index_created_version_major());
                }
                drop(d);
                dir.close()?;
            }
        }
    }
    Ok(())
}

/// see LUCENE-8639
/// Port of `TestIndexWriter.testFlushWhileStartingNewThreads`.
fn test_flush_while_starting_new_threads() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, IndexWriterConfig::new())?;
    w.add_document(&Document::new())?;
    assert_equals(&1, &w.get_docs_writer().per_thread_pool().size());
    let latch = Arc::new(CountDownLatch::new(1));
    let thread = {
        let w = Arc::clone(&w);
        let latch = Arc::clone(&latch);
        Thread::unnamed(move || {
            latch.count_down();
            let mut states = Vec::new();
            for _ in 0..100 {
                match w.get_docs_writer().per_thread_pool().get_and_lock() {
                    Ok(state) => {
                        state.delete_queue().next_sequence_number();
                        states.push(state);
                    }
                    Err(e) => {
                        // `finally { IOUtils.closeWhileHandlingException(states); }`
                        drop(states);
                        rethrow_java(e);
                    }
                }
            }
            // `IOUtils.closeWhileHandlingException(states)`: `state::unlock`.
            for state in states {
                state.unlock();
            }
        })
    };
    thread.start();
    latch.await_();
    w.get_docs_writer().flush_control().mark_for_full_flush()?;
    join(&thread);
    w.get_docs_writer().flush_control().abort_full_flushes()?;
    w.close()?;
    drop((w, d));
    dir.close()
}

/// `java.util.concurrent.Semaphore`: `release`, `acquire` and `tryAcquire`
/// over a count of permits.
struct Semaphore {
    permits: Mutex<i64>,
    cond: std::sync::Condvar,
}

impl Semaphore {
    fn new(permits: i64) -> Self {
        Semaphore {
            permits: Mutex::new(permits),
            cond: std::sync::Condvar::new(),
        }
    }

    /// `release(n)`.
    fn release(&self, n: i64) {
        *self.permits.lock().unwrap_or_else(|e| e.into_inner()) += n;
        self.cond.notify_all();
    }

    /// `acquire(n)`.
    fn acquire(&self, n: i64) {
        let mut permits = self.permits.lock().unwrap_or_else(|e| e.into_inner());
        while *permits < n {
            permits = self.cond.wait(permits).unwrap_or_else(|e| e.into_inner());
        }
        *permits -= n;
    }

    /// `tryAcquire()`.
    fn try_acquire(&self) -> bool {
        let mut permits = self.permits.lock().unwrap_or_else(|e| e.into_inner());
        if *permits > 0 {
            *permits -= 1;
            true
        } else {
            false
        }
    }
}

/// `new SearcherManager(writer, new SearcherFactory())`.
fn searcher_manager(w: &IndexWriter) -> Result<Arc<SearcherManager>> {
    Ok(Arc::new(SearcherManager::from_writer(
        w,
        true,
        false,
        Some(Arc::new(DefaultSearcherFactory)),
        None,
    )?))
}

/// Port of `TestIndexWriter.testRefreshAndRollbackConcurrently`.
fn test_refresh_and_rollback_concurrently() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
    let stopped = Arc::new(AtomicBool::new(false));
    let indexed_docs = Arc::new(Semaphore::new(0));
    let indexer = {
        let w = Arc::clone(&w);
        let stopped = Arc::clone(&stopped);
        let indexed_docs = Arc::clone(&indexed_docs);
        Thread::unnamed(move || {
            while !stopped.load(Ordering::SeqCst) {
                let result = (|| -> Result<i64> {
                    let id = random().next_int_bound(100).to_string();
                    let mut doc = Document::new();
                    doc.add(Box::new(StringField::new("id", id.clone(), Store::YES)?));
                    w.update_document(Term::from_text("id", &id), &doc)
                })();
                match result {
                    Ok(_) => indexed_docs.release(1),
                    Err(e) if is_instance(&e, "org.apache.lucene.store.AlreadyClosedException") => {
                        return;
                    }
                    Err(e) => rethrow_java(assertion_error_of(e)),
                }
            }
        })
    };

    let sm = searcher_manager(&w)?;
    let refresher = {
        let sm = Arc::clone(&sm);
        let stopped = Arc::clone(&stopped);
        Thread::unnamed(move || {
            while !stopped.load(Ordering::SeqCst) {
                match sm.maybe_refresh_blocking() {
                    Ok(()) => {}
                    Err(e) if is_instance(&e, "org.apache.lucene.store.AlreadyClosedException") => {
                        return;
                    }
                    Err(e) => rethrow_java(assertion_error_of(e)),
                }
            }
        })
    };

    let result = (|| -> Result<()> {
        indexer.start();
        refresher.start();
        indexed_docs.acquire(1 + i64::from(random().next_int_bound(100)));
        w.rollback()
    })();
    stopped.store(true, Ordering::SeqCst);
    join(&indexer);
    join(&refresher);
    let e = w.get_tragic_exception();
    let supplier = || match &e {
        Some(e) => format!("{e:?}"),
        None => String::new(),
    };
    assert_null_msg(
        &format!(
            "should not consider ACE a tragedy on a closed IW: {}",
            supplier()
        ),
        &w.get_tragic_exception().map(|_| ()),
    );
    let closed = sm.close();
    drop((sm, w, d));
    result.and(closed).and(dir.close())
}

/// An event of `testCloseableQueue`: `w -> { assertNotNull(w);
/// executed.incrementAndGet(); }`.
fn counting_event(executed: &Arc<AtomicI32>) -> Event {
    let executed = Arc::clone(executed);
    Box::new(move |_w: &IndexWriter| {
        // `assertNotNull(w)`: the writer reference cannot be null.
        executed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
}

/// Port of `TestIndexWriter.testCloseableQueue`.
fn test_closeable_queue() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
        let queue = Arc::new(EventQueue::new(&w));
        let executed = Arc::new(AtomicI32::new(0));

        queue.add(counting_event(&executed))?;
        queue.add(counting_event(&executed))?;
        queue.process_events()?;
        assert_equals(&2, &executed.load(Ordering::SeqCst));
        queue.process_events()?;
        assert_equals(&2, &executed.load(Ordering::SeqCst));

        queue.add(counting_event(&executed))?;
        queue.add(counting_event(&executed))?;

        let t = {
            let queue = Arc::clone(&queue);
            Thread::unnamed(move || match queue.process_events() {
                Ok(()) => {}
                Err(e) if is_instance(&e, "org.apache.lucene.store.AlreadyClosedException") => {
                    // possible
                }
                Err(e) => rethrow_java(assertion_error_of(e)),
            })
        };
        t.start();
        queue.close()?;
        join(&t);
        assert_equals(&4, &executed.load(Ordering::SeqCst));
        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            queue.process_events()
        });
        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            queue.add(Box::new(|_w: &IndexWriter| Ok(())))
        });
        drop(queue);
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// The thread body `testRandomOperations` and
/// `testRandomOperationsWithSoftDeletes` share: `num_operations` random
/// operations, with `index` choosing how a document is indexed.
fn random_operations(
    writer: Arc<IndexWriter>,
    sm: Arc<SearcherManager>,
    num_operations: i64,
    index: Arc<dyn Fn(&IndexWriter, &str) -> Result<()> + Send + Sync>,
) -> Result<()> {
    let num_operations = Arc::new(Semaphore::new(num_operations));
    let single_doc = random().next_boolean();
    let num_threads = 1 + random().next_int_bound(4);
    let latch = Arc::new(CountDownLatch::new(i64::from(num_threads)));
    let mut threads = Vec::new();
    for _ in 0..num_threads {
        let writer = Arc::clone(&writer);
        let sm = Arc::clone(&sm);
        let latch = Arc::clone(&latch);
        let num_operations = Arc::clone(&num_operations);
        let index = Arc::clone(&index);
        let t = Thread::unnamed(move || {
            latch.count_down();
            let result = (|| -> Result<()> {
                latch.await_();
                while num_operations.try_acquire() {
                    let id = if single_doc {
                        "1".to_string()
                    } else {
                        random().next_int_bound(10).to_string()
                    };
                    index(&writer, &id)?;
                    if random().next_int_bound(100) < 10 {
                        sm.maybe_refresh_blocking()?;
                    }
                    if random().next_int_bound(100) < 5 {
                        writer.commit()?;
                    }
                    if random().next_int_bound(100) < 1 {
                        writer.force_merge_with(
                            1 + random().next_int_bound(10),
                            random().next_boolean(),
                        )?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                rethrow_java(assertion_error_of(e));
            }
        });
        t.start();
        threads.push(t);
    }
    for t in &threads {
        join(t);
    }
    Ok(())
}

/// Port of `TestIndexWriter.testRandomOperations`.
fn test_random_operations() -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();
    let keep_fully_deleted_segment = Arc::new(AtomicBool::new(false));
    let inner = LuceneTestCase::new_merge_policy();
    keep_fully_deleted_segment.store(random().next_boolean(), Ordering::SeqCst);
    iwc.set_merge_policy(Some(Arc::new(
        KeepFullyDeletedSegmentsMergePolicy::with_flag(inner, keep_fully_deleted_segment),
    )))?;
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, iwc)?;
        let sm = searcher_manager(&w)?;
        let num_operations = 10 + i64::from(random().next_int_bound(1000));
        let result = random_operations(
            Arc::clone(&w),
            Arc::clone(&sm),
            num_operations,
            Arc::new(|writer: &IndexWriter, id: &str| {
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new(
                    "id",
                    id.to_string(),
                    Store::YES,
                )?));
                if random().next_int_bound(10) <= 2 {
                    writer.update_document(Term::from_text("id", id), &doc)?;
                } else if random().next_int_bound(10) <= 2 {
                    writer.delete_documents_by_terms(vec![Term::from_text("id", id)])?;
                } else {
                    writer.add_document(&doc)?;
                }
                Ok(())
            }),
        );
        // try-with-resources closes `sm`, then `writer`.
        let sm_closed = sm.close();
        let w_closed = w.close();
        result.and(sm_closed).and(w_closed)?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testRandomOperationsWithSoftDeletes`.
fn test_random_operations_with_soft_deletes() -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();
    let seq_no = Arc::new(AtomicI32::new(-1));
    let retaining_seq_no = Arc::new(AtomicI32::new(0));
    iwc.set_soft_deletes_field(Some("soft_deletes".to_string()));
    {
        let retaining_seq_no = Arc::clone(&retaining_seq_no);
        iwc.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
            "soft_deletes",
            Arc::new(move || {
                LongPoint::new_range_query(
                    "seq_no",
                    &[i64::from(retaining_seq_no.load(Ordering::SeqCst))],
                    &[i64::MAX],
                )
                .unwrap_or_else(|e| rethrow_java(e))
            }),
            LuceneTestCase::new_merge_policy(),
        ))))?;
    }
    let soft_deletes_field = iwc.soft_deletes_field().unwrap_or_default().to_string();
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, iwc)?;
        let sm = searcher_manager(&w)?;
        let num_operations = 10 + i64::from(random().next_int_bound(1000));
        let result = random_operations(
            Arc::clone(&w),
            Arc::clone(&sm),
            num_operations,
            Arc::new(move |writer: &IndexWriter, id: &str| {
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new(
                    "id",
                    id.to_string(),
                    Store::YES,
                )?));
                doc.add(Box::new(LongPoint::new(
                    "seq_no",
                    &[i64::from(seq_no.fetch_add(1, Ordering::SeqCst))],
                )?));
                if random().next_int_bound(10) <= 2 {
                    if random().next_boolean() {
                        doc.add(Box::new(NumericDocValuesField::new(&soft_deletes_field, 1)));
                    }
                    writer.soft_update_document(
                        Term::from_text("id", id),
                        &doc,
                        &[soft_field(&soft_deletes_field, 1)?],
                    )?;
                } else {
                    writer.add_document(&doc)?;
                }
                if random().next_int_bound(100) < 10 {
                    let min = retaining_seq_no.load(Ordering::SeqCst);
                    let max = seq_no.load(Ordering::SeqCst);
                    if min < max && random().next_boolean() {
                        let _ = retaining_seq_no.compare_exchange(
                            min,
                            min - random().next_int_bound(max - min),
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        );
                    }
                }
                Ok(())
            }),
        );
        let sm_closed = sm.close();
        let w_closed = w.close();
        result.and(sm_closed).and(w_closed)?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testMaxCompletedSequenceNumber`.
fn test_max_completed_sequence_number() -> Result<()> {
    {
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        let w = writer(&d, IndexWriterConfig::new())?;
        assert_equals(&1, &w.add_document(&Document::new())?);
        assert_equals(
            &2,
            &w.update_document(Term::from_text("foo", "bar"), &Document::new())?,
        );
        w.flush_next_buffer()?;
        assert_equals(&3, &w.commit()?);
        assert_equals(&4, &w.add_document(&Document::new())?);
        assert_equals(&4, &w.get_max_completed_sequence_number()?);
        // commit moves seqNo by 2 since there is one DWPT that could still be in-flight
        assert_equals(&6, &w.commit()?);
        assert_equals(&6, &w.get_max_completed_sequence_number()?);
        assert_equals(&7, &w.add_document(&Document::new())?);
        directory_reader::open_with_writer(&*w)?.close()?;
        // getReader moves seqNo by 2 since there is one DWPT that could still be in-flight
        assert_equals(&9, &w.get_max_completed_sequence_number()?);
        w.close()?;
        drop((w, d));
        dir.close()?;
    }
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
        let manager = searcher_manager(&w)?;
        let start = Arc::new(CountDownLatch::new(1));
        let num_docs = if *TEST_NIGHTLY {
            TestUtil::next_int(&*random(), 100, 600)
        } else {
            TestUtil::next_int(&*random(), 10, 60)
        };
        let max_completed_seq_id = Arc::new(AtomicI64::new(-1));
        let num_threads = 2 + random().next_int_bound(2);
        let mut threads = Vec::new();
        for idx in 0..num_threads {
            let w = Arc::clone(&w);
            let manager = Arc::clone(&manager);
            let start = Arc::clone(&start);
            let max_completed_seq_id = Arc::clone(&max_completed_seq_id);
            let t = Thread::unnamed(move || {
                let result = (|| -> Result<()> {
                    start.await_();
                    for j in 0..num_docs {
                        let mut doc = Document::new();
                        let id = format!("{idx}-{j}");
                        doc.add(Box::new(StringField::new("id", id.clone(), Store::NO)?));
                        let seq_no = w.add_document(&doc)?;
                        if max_completed_seq_id.load(Ordering::SeqCst) < seq_no {
                            let max_completed_sequence_number =
                                w.get_max_completed_sequence_number()?;
                            manager.maybe_refresh_blocking()?;
                            max_completed_seq_id
                                .fetch_max(max_completed_sequence_number, Ordering::SeqCst);
                        }
                        let acquire = manager.acquire()?;
                        let hits = acquire
                            .searcher()
                            .search(Arc::new(TermQuery::new(Term::from_text("id", &id))), 10)
                            .map(|top_docs| top_docs.total_hits.value());
                        manager.release(acquire)?;
                        assert_equals(&1, &hits?);
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    rethrow_java(assertion_error_of(e));
                }
            });
            t.start();
            threads.push(t);
        }
        start.count_down();
        for t in &threads {
            join(t);
        }
        manager.close()?;
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// The anonymous `InfoStream` of `testEnsureMaxSeqNoIsAccurateDuringFlush`:
/// it blocks a DWPT at the `addDocuments start` test point.
struct BlockingTestPointStream {
    wait_ref: Arc<Mutex<Arc<CountDownLatch>>>,
    arrived_ref: Arc<Mutex<Arc<CountDownLatch>>>,
}

impl InfoStream for BlockingTestPointStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriter$8"
    }

    fn message(&self, component: &str, message: &str) {
        if "TP" == component && "DocumentsWriterPerThread addDocuments start" == message {
            let arrived = Arc::clone(&self.arrived_ref.lock().unwrap_or_else(|e| e.into_inner()));
            arrived.count_down();
            let wait = Arc::clone(&self.wait_ref.lock().unwrap_or_else(|e| e.into_inner()));
            wait.await_();
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        "TP" == component
    }

    fn close(&self) {}
}

/// Port of `TestIndexWriter.testEnsureMaxSeqNoIsAccurateDuringFlush`.
fn test_ensure_max_seq_no_is_accurate_during_flush() -> Result<()> {
    let wait_ref = Arc::new(Mutex::new(Arc::new(CountDownLatch::new(0))));
    let arrived_ref = Arc::new(Mutex::new(Arc::new(CountDownLatch::new(0))));
    let stream = Arc::new(BlockingTestPointStream {
        wait_ref: Arc::clone(&wait_ref),
        arrived_ref: Arc::clone(&arrived_ref),
    });
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_info_stream(stream)?;
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        // `new IndexWriter(dir, indexWriterConfig) { protected boolean
        // isEnableTestPoints() { return true; } }`.
        let w = IndexWriter::new_with_test_points(Arc::clone(&d), index_writer_config)?;
        // we produce once DWPT with 1 doc
        w.add_document(&Document::new())?;
        assert_equals(&1, &w.get_docs_writer().per_thread_pool().size());
        let max_completed_sequence_number = w.get_max_completed_sequence_number()?;
        // safe the seqNo and use the latches to block this DWPT such that a refresh must wait for it
        *wait_ref.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(CountDownLatch::new(1));
        *arrived_ref.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(CountDownLatch::new(1));
        let waiter_thread = {
            let w = Arc::clone(&w);
            Thread::unnamed(move || {
                if let Err(e) = w.add_document(&Document::new()) {
                    rethrow_java(assertion_error_of(e));
                }
            })
        };
        waiter_thread.start();
        let arrived = Arc::clone(&arrived_ref.lock().unwrap_or_else(|e| e.into_inner()));
        arrived.await_();
        let refresh_thread = {
            let w = Arc::clone(&w);
            Thread::unnamed(move || {
                if let Err(e) = directory_reader::open_with_writer(&*w).and_then(|r| r.close()) {
                    rethrow_java(assertion_error_of(e));
                }
            })
        };
        let delete_queue = w.get_docs_writer().delete_queue();
        refresh_thread.start();
        // now we wait until the refresh has swapped the deleted queue and assert that
        // we see an accurate seqId
        while Arc::ptr_eq(&w.get_docs_writer().delete_queue(), &delete_queue) {
            std::thread::yield_now(); // busy wait for refresh to swap the queue
        }
        let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            w.get_max_completed_sequence_number()
                .map(|v| assert_equals(&max_completed_sequence_number, &v))
        }));
        Arc::clone(&wait_ref.lock().unwrap_or_else(|e| e.into_inner())).count_down();
        join(&waiter_thread);
        join(&refresh_thread);
        match checked {
            Ok(r) => r?,
            Err(payload) => std::panic::resume_unwind(payload),
        }
        assert_equals(
            &(max_completed_sequence_number + 2),
            &w.get_max_completed_sequence_number()?,
        );
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// `new IndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE)`.
fn no_merge_iwc() -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::new();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    Ok(iwc)
}

/// A document with `NumericDocValuesField("num", num)` and
/// `StringField("id", id)`.
fn num_id_doc(num: i64, id: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("num", num)));
    doc.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
    Ok(doc)
}

/// `StringHelper.idToString(id)`.
fn id_string(id: &[u8]) -> String {
    StringHelper::id_to_string(Some(id))
}

/// Port of `TestIndexWriter.testSegmentCommitInfoId`.
fn test_segment_commit_info_id() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let segment_commit_infos;
    {
        let w = writer(&d, no_merge_iwc()?)?;
        w.add_document(&num_id_doc(1, "1")?)?;
        w.add_document(&num_id_doc(1, "2")?)?;
        w.commit()?;
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
        let id = infos.info(0).id();
        let seg_info_id = infos.info(0).info.id();

        w.update_numeric_doc_value(Term::from_text("id", "1"), "num", 2)?;
        w.commit()?;
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
        assert_equals(&1, &infos.size());
        assert_not_equals(&id_string(&id), &id_string(&infos.info(0).id()));
        assert_equals(
            &id_string(&seg_info_id),
            &id_string(&infos.info(0).info.id()),
        );
        let id = infos.info(0).id();
        w.add_document(&Document::new())?; // second segment
        w.commit()?;
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
        assert_equals(&2, &infos.size());
        assert_equals(&id_string(&id), &id_string(&infos.info(0).id()));
        assert_equals(
            &id_string(&seg_info_id),
            &id_string(&infos.info(0).info.id()),
        );

        w.update_document(Term::from_text("id", "1"), &num_id_doc(5, "1")?)?;
        w.commit()?;
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
        assert_equals(&3, &infos.size());
        assert_not_equals(&id_string(&id), &id_string(&infos.info(0).id()));
        assert_equals(
            &id_string(&seg_info_id),
            &id_string(&infos.info(0).info.id()),
        );
        segment_commit_infos = infos;
        w.close()?;
    }

    {
        let mut dir2 = LuceneTestCase::new_directory();
        let d2 = dir2.as_directory();
        let w2 = writer(&d2, no_merge_iwc()?)?;
        w2.add_indexes_dirs(&[Arc::clone(&d)])?;
        w2.commit()?;
        let infos2 = SegmentInfos::read_latest_commit(Arc::clone(&d2))?;
        assert_equals(&infos2.size(), &segment_commit_infos.size());
        for i in 0..infos2.size() {
            assert_equals(
                &id_string(&infos2.info(i).id()),
                &id_string(&segment_commit_infos.info(i).id()),
            );
            assert_equals(
                &id_string(&infos2.info(i).info.id()),
                &id_string(&segment_commit_infos.info(i).info.id()),
            );
        }
        w2.close()?;
        drop((w2, d2));
        dir2.close()?;
    }
    drop(d);
    dir.close()?;

    let mut ids: HashSet<String> = HashSet::new();
    for _ in 0..2 {
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        {
            let w = writer(&d, no_merge_iwc()?)?;
            w.add_document(&num_id_doc(1, "1")?)?;
            w.commit()?;
            let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
            let id = id_string(&infos.info(0).id());
            Assert::assert_true(ids.insert(id));
            w.update_numeric_doc_value(Term::from_text("id", "1"), "num", 2)?;
            w.commit()?;
            let infos = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
            let id = id_string(&infos.info(0).id());
            Assert::assert_true(ids.insert(id));
            w.close()?;
        }
        drop(d);
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestIndexWriter.testMergeZeroDocsMergeIsClosedOnce`.
fn test_merge_zero_docs_merge_is_closed_once() -> Result<()> {
    // `new LogDocMergePolicy() { keepFullyDeletedSegment(...) { return true; } }`.
    let keep_all_segments: Arc<dyn MergePolicy> =
        Arc::new(KeepFullyDeletedSegmentsMergePolicy::new(Arc::new(
            LogMergePolicy::new(Arc::new(LogDocSize)),
        )));
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let iwc = IndexWriterConfig::new();
        iwc.set_merge_policy(Some(Arc::new(OneMergeWrappingMergePolicy::new(
            keep_all_segments,
            Arc::new(|merge: OneMerge| {
                let only_finish_once: Arc<SetOnce<bool>> = Arc::new(SetOnce::new());
                let mut wrapped =
                    OneMerge::new(merge.segments.clone()).unwrap_or_else(|e| rethrow_java(e));
                wrapped.set_merge_finished(Arc::new(
                    move |_merge: &OneMerge, _success: bool, _segment_dropped: bool, _deleter| {
                        // super.mergeFinished(success, segmentDropped) does nothing
                        only_finish_once.set(true)
                    },
                ));
                wrapped
            }),
        ))))?;
        let w = writer(&d, iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::NO,
        )?));
        w.add_document(&doc)?;
        w.flush()?;
        w.add_document(&doc)?;
        w.flush()?;
        w.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
        w.flush()?;
        assert_equals(&2, &w.get_segment_count()?);
        assert_equals(&0, &w.get_doc_stats()?.num_docs);
        assert_equals(&2, &w.get_doc_stats()?.max_doc);
        w.force_merge(1)?;
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// The anonymous `FilterMergePolicy` of
/// `testMergeOnCommitKeepFullyDeletedSegments`: it keeps fully deleted
/// segments and merges them on full flush.
struct FullyDeletedOnCommitMergePolicy {
    keep: KeepFullyDeletedSegmentsMergePolicy,
}

impl std::fmt::Display for FullyDeletedOnCommitMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.keep, f)
    }
}

impl MergePolicy for FullyDeletedOnCommitMergePolicy {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.keep.as_any()
    }

    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.keep
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<Option<MergeSpecification>> {
        self.keep.find_merges_for_readers(readers)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.keep.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.keep
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    /// The override.
    fn find_full_flush_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let fully_deleted_segments: Vec<SegmentCommitInfo> = segment_infos
            .as_list()
            .iter()
            .filter(|s| {
                s.info.max_doc().unwrap_or_else(|e| rethrow_java(e)) - s.get_del_count() == 0
            })
            .cloned()
            .collect();
        if fully_deleted_segments.is_empty() {
            return Ok(None);
        }
        let mut spec = MergeSpecification::new();
        spec.add(OneMerge::new(fully_deleted_segments)?);
        Ok(Some(spec))
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.keep
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.keep.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.keep.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.keep.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.keep.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.keep.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.keep.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.keep.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.keep.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
    ) -> Result<i32> {
        self.keep
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.keep.max_full_flush_merge_size()
    }
}

/// Port of `TestIndexWriter.testMergeOnCommitKeepFullyDeletedSegments`.
fn test_merge_on_commit_keep_fully_deleted_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_max_full_flush_merge_wait_millis(30 * 1000);
    // `iwc.mergePolicy = …`: the field is assigned directly.
    iwc.set_merge_policy(Some(Arc::new(FullyDeletedOnCommitMergePolicy {
        keep: KeepFullyDeletedSegmentsMergePolicy::new(LuceneTestCase::new_merge_policy()),
    })))?;
    let w = writer(&d, iwc)?;
    let doc = string_id_doc("1")?;
    w.add_document(&doc)?;
    w.commit()?;
    w.update_document(Term::from_text("id", "1"), &doc)?;
    w.commit()?;
    {
        let reader = directory_reader::open_with_writer(&*w)?;
        assert_equals(&1, &reader.num_docs());
        reader.close()?;
    }
    w.close()?;
    drop((w, d));
    dir.close()
}

/// Port of `TestIndexWriter.testPendingNumDocs`.
fn test_pending_num_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_docs = random().next_int_bound(100);
    {
        let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
        for i in 0..num_docs {
            w.add_document(&string_id_doc(&i.to_string())?)?;
            assert_equals(&(i64::from(i) + 1), &w.get_pending_num_docs());
        }
        assert_equals(&i64::from(num_docs), &w.get_pending_num_docs());
        w.flush()?;
        assert_equals(&i64::from(num_docs), &w.get_pending_num_docs());
        w.close()?;
    }
    {
        let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
        assert_equals(&i64::from(num_docs), &w.get_pending_num_docs());
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testIndexWriterBlocksOnStall`.
fn test_index_writer_blocks_on_stall() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, LuceneTestCase::new_index_writer_config())?;
        let stall_control = &w.get_docs_writer().flush_control().stall_control;
        stall_control.update_stalled(true);
        let num_threads = random().next_int_bound(3) + 1;
        let num_threads_completed = Arc::new(AtomicI64::new(0));
        let mut threads = Vec::new();
        for _ in 0..num_threads {
            let w = Arc::clone(&w);
            let num_threads_completed = Arc::clone(&num_threads_completed);
            let t = Thread::unnamed(move || {
                let result = string_id_doc("0").and_then(|d| w.add_document(&d));
                if let Err(e) = result {
                    rethrow_java(assertion_error_of(e));
                }
                num_threads_completed.fetch_add(1, Ordering::SeqCst);
            });
            t.start();
            threads.push(t);
        }
        let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for _ in 0..10 {
                stall_control.notify_all();
                while stall_control.num_waiting() != threads.len() {
                    // wait for all threads to be stalled again
                    assert_equals(&0, &w.get_pending_num_docs());
                    assert_equals(&0, &num_threads_completed.load(Ordering::SeqCst));
                }
            }
        }));
        stall_control.update_stalled(false);
        for t in &threads {
            join(t);
        }
        if let Err(payload) = checked {
            std::panic::resume_unwind(payload);
        }
        w.commit()?;
        assert_equals(&(threads.len() as i32), &w.get_doc_stats()?.max_doc);
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// `Set.of(names…)`.
fn set_of(names: &[&str]) -> ImmutableSet<String> {
    ImmutableSet::copy_of(names.iter().map(|n| n.to_string()))
}

/// Port of `TestIndexWriter.testGetFieldNames`.
fn test_get_field_names() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let w = writer(&d, mock_iwc())?;

    assert_equals(&set_of(&[]), &w.get_field_names());

    add_doc_with_field(&w, "f1")?;
    assert_equals(&set_of(&["f1"]), &w.get_field_names());

    // should be unmodifiable:
    let field_set = w.get_field_names();
    expect_throws("java.lang.UnsupportedOperationException", || {
        field_set.add("cannot modify".to_string())
    });
    expect_throws("java.lang.UnsupportedOperationException", || {
        field_set.remove("f1")
    });

    add_doc_with_field(&w, "f2")?;
    assert_equals(&set_of(&["f1", "f2"]), &w.get_field_names());

    // set from a previous call is an independent immutable copy, cannot be modified.
    assert_equals(&set_of(&["f1"]), &field_set);

    // flush should not have an effect on field names
    w.flush()?;
    assert_equals(&set_of(&["f1", "f2"]), &w.get_field_names());

    // commit should not have an effect on field names
    w.commit()?;
    assert_equals(&set_of(&["f1", "f2"]), &w.get_field_names());

    w.close()?;

    // new writer should identify committed fields
    let w = writer(&d, mock_iwc())?;
    assert_equals(&set_of(&["f1", "f2"]), &w.get_field_names());

    w.delete_all()?;
    assert_equals(&set_of(&[]), &w.get_field_names());

    w.close()?;
    drop(d);
    dir.close()
}

/// `addDocWithField(writer, field)`.
fn add_doc_with_field(writer: &IndexWriter, field: &str) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        field,
        "value",
        &stored_text_type(),
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))` with a parent field.
fn parent_iwc(parent: Option<&str>) -> IndexWriterConfig {
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    if let Some(parent) = parent {
        iwc.set_parent_field(Some(parent.to_string()));
    }
    iwc
}

/// Port of `TestIndexWriter.testParentAndSoftDeletesAreTheSame`.
fn test_parent_and_soft_deletes_are_the_same() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let index_writer_config = mock_iwc();
    index_writer_config.set_soft_deletes_field(Some("foo".to_string()));
    index_writer_config.set_parent_field(Some("foo".to_string()));
    let iae = expect_throws(IAE, || writer(&d, index_writer_config));
    assert_equals(
        &Some("parent document and soft-deletes field can't be the same field \"foo\""),
        &iae.message(),
    );
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testParentFieldExistingIndex`.
fn test_parent_field_existing_index() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, parent_iwc(None))?;
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new("f", "a".to_string(), Store::NO)?));
        w.add_document(&doc)?;
        w.close()?;
    }
    for open_mode in [OpenMode::APPEND, OpenMode::CREATE_OR_APPEND] {
        let iae = expect_throws(IAE, || {
            let iwc = parent_iwc(None);
            iwc.set_open_mode(open_mode)?;
            iwc.set_parent_field(Some("foo".to_string()));
            writer(&d, iwc)
        });
        assert_equals(
            &Some("can't add a parent field to an already existing index without a parent field"),
            &iae.message(),
        );
    }

    {
        let iwc = parent_iwc(None);
        iwc.set_open_mode(OpenMode::CREATE)?;
        iwc.set_parent_field(Some("foo".to_string()));
        let w = writer(&d, iwc)?;
        w.add_document(&Document::new())?;
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testIndexWithParentFieldIsCongruent`.
fn test_index_with_parent_field_is_congruent() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, parent_iwc(Some("parent")))?;
        if random().next_boolean() {
            let block = || -> Result<Vec<Document>> {
                Ok(vec![
                    string_id_doc("1")?,
                    string_id_doc("1")?,
                    string_id_doc("1")?,
                ])
            };
            w.add_documents(&block()?)?;
            w.flush()?;
            if random().next_boolean() {
                w.add_documents(&block()?)?;
            }
        } else {
            w.add_document(&Document::new())?;
        }
        w.commit()?;
        w.close()?;
    }
    let ex = expect_throws(IAE, || writer(&d, parent_iwc(Some("someOtherField"))));
    assert_equals(
        &Some(
            "can't add field [parent] as parent document field; this IndexWriter is configured with [someOtherField] as parent document field",
        ),
        &ex.message(),
    );
    let ex = expect_throws(IAE, || writer(&d, parent_iwc(None)));
    assert_equals(
        &Some(
            "can't add field [parent] as parent document field; this IndexWriter has no parent document field configured",
        ),
        &ex.message(),
    );
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testParentFieldIsAlreadyUsed`.
fn test_parent_field_is_already_used() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, parent_iwc(None))?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "parent",
            "1".to_string(),
            Store::YES,
        )?));
        w.add_document(&doc)?;
        w.commit()?;
        w.close()?;
    }
    let iae = expect_throws(IAE, || writer(&d, parent_iwc(Some("parent"))));
    assert_equals(
        &Some(
            "can't add [parent] as non parent document field; this IndexWriter is configured with [parent] as parent document field",
        ),
        &iae.message(),
    );
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testParentFieldEmptyIndex`.
fn test_parent_field_empty_index() -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let d = dir.as_directory();
    {
        let w = writer(&d, parent_iwc(Some("parent")))?;
        w.commit()?;
        w.close()?;
    }
    {
        let w = writer(&d, parent_iwc(Some("parent")))?;
        w.commit()?;
        w.close()?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testSingleDocBlockWritesParentField`.
fn test_single_doc_block_writes_parent_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let w = writer(&d, parent_iwc(Some("parent")))?;
        // Single-document "block" — the lone doc is both first and last in the block
        w.add_documents(&[string_id_doc("s0")?])?;

        // Multi-document block for comparison
        w.add_documents(&[string_id_doc("c0")?, string_id_doc("p0")?])?;
        w.close()?;
    }

    {
        let reader = directory_reader::open(Arc::clone(&d))?;
        // 3 docs total: doc0=single, doc1=child, doc2=parent
        assert_equals(&3, &reader.num_docs());
        let leaves = ir(&reader).leaves()?;
        let leaf = leaves[0].leaf_reader();
        let parent_dv = leaf.get_numeric_doc_values("parent")?;
        assert_not_null(&parent_dv);
        let mut parent_dv = parent_dv.unwrap_or_else(|| npe("parentDV"));

        // doc 0 (the single-doc block) should be a parent
        assert_equals(&0, &parent_dv.next_doc()?);
        assert_equals(&-1, &parent_dv.long_value()?);

        // doc 2 (last doc of the multi-doc block) should be a parent
        assert_equals(&2, &parent_dv.next_doc()?);
        assert_equals(&-1, &parent_dv.long_value()?);

        // no more parents
        assert_equals(&NO_MORE_DOCS, &parent_dv.next_doc()?);
        drop((parent_dv, leaves));
        reader.close()?;
    }
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testDocValuesMixedSkippingIndex`.
fn test_doc_values_mixed_skipping_index() -> Result<()> {
    {
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        {
            let w = writer(&d, parent_iwc(None))?;
            let mut doc1 = Document::new();
            doc1.add(Box::new(SortedNumericDocValuesField::indexed_field(
                "test",
                random().next_long(),
            )));
            w.add_document(&doc1)?;

            let mut doc2 = Document::new();
            doc2.add(Box::new(SortedNumericDocValuesField::new(
                "test",
                random().next_long(),
            )));
            let ex = expect_throws(IAE, || w.add_document(&doc2));
            assert_equals(
                &Some(
                    "Inconsistency of field data structures across documents for field [test] of doc [1]. doc values skip index type: expected 'RANGE', but it has 'NONE'.",
                ),
                &ex.message(),
            );
            w.close()?;
        }
        drop(d);
        dir.close()?;
    }
    {
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        {
            let w = writer(&d, parent_iwc(None))?;
            let mut doc1 = Document::new();
            doc1.add(Box::new(SortedSetDocValuesField::new(
                "test",
                TestUtil::random_binary_term(&*random()),
            )));
            w.add_document(&doc1)?;

            let mut doc2 = Document::new();
            doc2.add(Box::new(SortedSetDocValuesField::indexed_field(
                "test",
                TestUtil::random_binary_term(&*random()),
            )));
            let ex = expect_throws(IAE, || w.add_document(&doc2));
            assert_equals(
                &Some(
                    "Inconsistency of field data structures across documents for field [test] of doc [1]. doc values skip index type: expected 'NONE', but it has 'RANGE'.",
                ),
                &ex.message(),
            );
            w.close()?;
        }
        drop(d);
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestIndexWriter.testDocValuesSkippingIndexWithoutDocValues`.
fn test_doc_values_skipping_index_without_doc_values() -> Result<()> {
    for doc_values_type in [DocValuesType::NONE, DocValuesType::BINARY] {
        let field_type = FieldType::new();
        field_type.set_stored(true)?;
        field_type.set_doc_values_type(doc_values_type)?;
        field_type.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)?;
        field_type.freeze();
        let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
        let d = dir.as_directory();
        {
            let w = writer(&d, parent_iwc(None))?;
            let mut doc1 = Document::new();
            doc1.add(Box::new(Field::new_with_bytes(
                "test",
                BytesRef::new(vec![0u8; 10]),
                field_type.clone(),
            )?));
            let ex = expect_throws(IAE, || w.add_document(&doc1));
            Assert::assert_true(
                ex.message()
                    .unwrap_or_default()
                    .starts_with("field 'test' cannot have docValuesSkipIndexType=RANGE"),
            );
            w.close()?;
        }
        drop(d);
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestIndexWriter.testAdvanceSegmentInfosCounter`.
fn test_advance_segment_infos_counter() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;

    // add 10 documents
    for i in 0..10 {
        add_doc_with_index(&w, i)?;
        w.commit()?;
    }
    w.advance_segment_infos_counter(1)?;
    Assert::assert_true(w.get_segment_infos_counter()? >= 1);

    w.advance_segment_infos_counter(1000)?;
    // add 40 documents
    for i in 10..50 {
        add_doc_with_index(&w, i)?;
        w.commit()?;
    }

    // There may be merge operations in the background, here only verifies that the current segment
    // counter is greater than 1000.
    Assert::assert_true(w.get_segment_infos_counter()? >= 1000);

    let doc_stats = w.get_doc_stats()?;
    assert_equals(&50, &doc_stats.max_doc);
    assert_equals(&50, &doc_stats.num_docs);
    w.close()?;

    // check that the index reader gives the same numbers.
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&50, &reader.max_doc());
    assert_equals(&50, &reader.num_docs());
    reader.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriter.testAdvanceSegmentCounterInCrashAndRecoveryScenario`.
fn test_advance_segment_counter_in_crash_and_recovery_scenario() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = writer(&d, mock_iwc())?;

    // add 100 documents
    for i in 0..100 {
        add_doc_with_index(&w, i)?;
        if random().next_boolean() {
            w.commit()?;
        }
    }
    let doc_stats = w.get_doc_stats()?;
    assert_equals(&100, &doc_stats.max_doc);
    assert_equals(&100, &doc_stats.num_docs);
    w.commit()?;
    w.close()?;

    // recovery and advance segment counter
    let w = writer(&d, mock_iwc())?;
    assert_equals(&100, &w.get_doc_stats()?.num_docs);
    let new_segment_counter = w.get_segment_infos_counter()? + 1000;
    w.advance_segment_infos_counter(new_segment_counter)?;

    // add 10 documents
    for i in 0..10 {
        add_doc_with_index(&w, i)?;
        if random().next_boolean() {
            w.commit()?;
        }
    }

    Assert::assert_true(w.get_segment_infos_counter()? >= new_segment_counter);

    assert_equals(&110, &w.get_doc_stats()?.num_docs);
    // check that the index reader gives the same numbers.
    w.commit()?;
    let reader = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&110, &reader.max_doc());
    assert_equals(&110, &reader.num_docs());
    reader.close()?;
    w.close()?;
    drop(d);
    dir.close()
}
