//! Port of `lucene/core/src/test/org/apache/lucene/index/TestAddIndexes.java`.
//!
//! * The anonymous and inner `MergePolicy` subclasses that override
//!   `findMerges(CodecReader...)` are [`ReadersMergePolicy`]: a
//!   `FilterMergePolicy` over the Java superclass, with the override as a
//!   closure that receives the superclass as `super`.
//! * `RunAddIndexesThreads` and its three subclasses are one struct whose
//!   `doBody`/`handle` dispatch on [`Variant`].
//! * `new OneMerge(merge) { setMergeInfo … }` (`testSetDiagnostics`) is the
//!   same merge with a `setMergeInfo` override installed, as Rucene's
//!   wrapping merge policies wrap a merge.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::codecs::{Codec, FilterCodec, PostingsFormat};
use rucene::document::{
    Document, Field, FieldType, IntPoint, NumericDocValuesField, NumericValue, Store, StringField,
    TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, SlowCodecReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::index_writer::{self, IndexWriter, SOURCE, SOURCE_ADDINDEXES_READERS};
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{
    CodecReaderSupplier, FilterMergePolicy, LogMergePolicy, MergeContext, MergePolicy,
    MergeSpecification, MergeTrigger, NoMergePolicy, TieredMergePolicy,
};
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, MergeSource, SerialMergeScheduler,
};
use rucene::index::postings_enum::POSTINGS_ENUM_NONE;
use rucene::index::terms::Term;
use rucene::index::{IndexOptions, IndexReader, LeafReader, SegmentCommitInfo, SegmentInfos};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::{IOUtils, SameThreadExecutorService};
use rucene_codecs::memory::DirectPostingsFormat;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::index::{AllDeletedFilterReader, RandomIndexWriter};
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_equals, class, random, test, verbose, IAE,
};
use crate::support::{npe, set_check_index_on_close};

/// `TestAddIndexes`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestAddIndexes",
        vec![
            test("testSimpleCase", test_simple_case),
            test("testWithPendingDeletes", test_with_pending_deletes),
            test("testWithPendingDeletes2", test_with_pending_deletes2),
            test("testWithPendingDeletes3", test_with_pending_deletes3),
            test("testAddSelf", test_add_self),
            test("testNoTailSegments", test_no_tail_segments),
            test("testNoCopySegments", test_no_copy_segments),
            test("testNoMergeAfterCopy", test_no_merge_after_copy),
            test("testMergeAfterCopy", test_merge_after_copy),
            test("testMoreMerges", test_more_merges),
            test("testHangOnClose", test_hang_on_close),
            test(
                "testAddIndexesWithConcurrentMerges",
                test_add_indexes_with_concurrent_merges,
            ),
            test(
                "testAddIndexesWithPartialMergeFailures",
                test_add_indexes_with_partial_merge_failures,
            ),
            test(
                "testAddIndexesWithNullMergeSpec",
                test_add_indexes_with_null_merge_spec,
            ),
            test(
                "testAddIndexesWithEmptyMergeSpec",
                test_add_indexes_with_empty_merge_spec,
            ),
            test(
                "testAddIndexesWithEmptyReaders",
                test_add_indexes_with_empty_readers,
            ),
            test(
                "testCascadingMergesTriggered",
                test_cascading_merges_triggered,
            ),
            test(
                "testAddIndexesHittingMaxDocsLimit",
                test_add_indexes_hitting_max_docs_limit,
            ),
            test("testAddIndexesWithThreads", test_add_indexes_with_threads),
            test("testAddIndexesWithClose", test_add_indexes_with_close),
            // `@SuppressForbidden(reason = "Thread sleep")`: a forbidden-APIs
            // marker, with no effect on the run.
            test(
                "testAddIndexesWithCloseNoWait",
                test_add_indexes_with_close_no_wait,
            ),
            // `@SuppressForbidden(reason = "Thread sleep")`.
            test("testAddIndexesWithRollback", test_add_indexes_with_rollback),
            test("testExistingDeletes", test_existing_deletes),
            test("testSimpleCaseCustomCodec", test_simple_case_custom_codec),
            test("testNonCFSLeftovers", test_non_cfs_leftovers),
            test("testAddIndexMissingCodec", test_add_index_missing_codec),
            test("testFieldNamesChanged", test_field_names_changed),
            test("testAddEmpty", test_add_empty),
            test("testFakeAllDeleted", test_fake_all_deleted),
            test("testLocksBlock", test_locks_block),
            test(
                "testIllegalIndexSortChange1",
                test_illegal_index_sort_change1,
            ),
            test(
                "testIllegalIndexSortChange2",
                test_illegal_index_sort_change2,
            ),
            test(
                "testAddIndexesDVUpdateSameSegmentName",
                test_add_indexes_dv_update_same_segment_name,
            ),
            test(
                "testAddIndexesDVUpdateNewSegmentName",
                test_add_indexes_dv_update_new_segment_name,
            ),
            test(
                "testAddIndicesWithSoftDeletes",
                test_add_indices_with_soft_deletes,
            ),
            test("testAddIndicesWithBlocks", test_add_indices_with_blocks),
            test("testSetDiagnostics", test_set_diagnostics),
            test("testIllegalParentDocChange", test_illegal_parent_doc_change),
            test("testIllegalNonParentField", test_illegal_non_parent_field),
        ],
    )
}

// ---------------------------------------------------------------------------
// Plumbing.
// ---------------------------------------------------------------------------

/// The directory a writer or a reader takes.
fn d(dir: &TestDirectory) -> Arc<dyn Directory> {
    dir.as_directory()
}

/// `new MockAnalyzer(random())`.
fn mock_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::new(&*random()))
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn new_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(mock_analyzer())
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn plain_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(mock_analyzer())
}

/// `new MockDirectoryWrapper(random(), new ByteBuffersDirectory())`.
fn mock_byte_buffers_dir() -> TestDirectory {
    TestDirectory::Mock(MockDirectoryWrapper::new(
        &*random(),
        Arc::new(ByteBuffersDirectory::new()),
    ))
}

/// `new MockDirectoryWrapper(random(), TestUtil.ramCopyOf(dir))`.
fn mock_ram_copy_of(dir: &Arc<dyn Directory>) -> Result<MockDirectoryWrapper> {
    let random = random();
    let copy = TestUtil::ram_copy_of(&**dir)?;
    Ok(MockDirectoryWrapper::new(&*random, Arc::from(copy)))
}

/// `new NumericDocValuesField(name, value)` as the `Field` the doc-values
/// update and soft-delete methods take.
fn ndv_field(name: &str, value: i64) -> Result<Field> {
    Field::new_with_number(
        name,
        NumericValue::Long(value),
        NumericDocValuesField::type_().clone(),
    )
}

/// `(CodecReader) leaf` (a `ClassCastException` when it is not one).
fn as_codec_reader(leaf: Arc<dyn LeafReader>) -> Arc<dyn CodecReader> {
    leaf.as_codec_reader().unwrap_or_else(|| {
        Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
    })
}

/// `(SegmentReader) leaf` (a `ClassCastException` when it is not one), as
/// the `CodecReader` `addIndexes` takes.
fn as_segment_reader(leaf: Arc<dyn LeafReader>) -> Arc<dyn CodecReader> {
    if leaf.as_segment_reader().is_none() {
        Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
    }
    as_codec_reader(leaf)
}

/// `(CodecReader) reader.leaves().get(i).reader()` for every leaf.
fn codec_leaves(reader: &Arc<dyn DirectoryReader>) -> Result<Vec<Arc<dyn CodecReader>>> {
    Ok(Arc::clone(reader)
        .leaves()?
        .iter()
        .map(|ctx| as_codec_reader(ctx.leaf_reader()))
        .collect())
}

/// `IOException.getCause() instanceof MergePolicy.MergeAbortedException`.
fn cause_is_merge_aborted(e: &LuceneError) -> bool {
    use std::error::Error;
    let cause: Option<&(dyn Error + 'static)> = match e {
        LuceneError::Io(io) => io.get_ref().map(|c| c as &(dyn Error + 'static)),
        other => other.source(),
    };
    cause
        .and_then(|c| c.downcast_ref::<LuceneError>())
        .is_some_and(|c| matches!(c, LuceneError::MergeAborted(_)))
}

/// `private IndexWriter newWriter(Directory dir, IndexWriterConfig conf)`.
fn new_writer(dir: &Arc<dyn Directory>, mut conf: IndexWriterConfig) -> Result<Arc<IndexWriter>> {
    conf.set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))?;
    IndexWriter::new(Arc::clone(dir), conf)
}

/// `private void addDocs(IndexWriter writer, int numDocs)`.
fn add_docs(writer: &IndexWriter, num_docs: i32) -> Result<()> {
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            "aaa",
            Store::NO,
        )));
        doc.add(Box::new(IntPoint::new("doc", &[i])?));
        doc.add(Box::new(IntPoint::new("doc2d", &[i, i])?));
        doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
        writer.add_document(&doc)?;
    }
    Ok(())
}

/// `private void addDocs2(IndexWriter writer, int numDocs)`.
fn add_docs2(writer: &IndexWriter, num_docs: i32) -> Result<()> {
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            "bbb",
            Store::NO,
        )));
        doc.add(Box::new(IntPoint::new("doc", &[i])?));
        doc.add(Box::new(IntPoint::new("doc2d", &[i, i])?));
        doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
        writer.add_document(&doc)?;
    }
    Ok(())
}

/// `private void addDocsWithID(IndexWriter writer, int numDocs, int
/// docStart)`: just like addDocs but with ID, starting from docStart.
fn add_docs_with_id(writer: &IndexWriter, num_docs: i32, doc_start: i32) -> Result<()> {
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            "aaa",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "id",
            &(doc_start + i).to_string(),
            Store::YES,
        )));
        doc.add(Box::new(IntPoint::new("doc", &[i])?));
        doc.add(Box::new(IntPoint::new("doc2d", &[i, i])?));
        doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
        writer.add_document(&doc)?;
    }
    Ok(())
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

/// `private void verifyNumDocs(Directory dir, int numDocs)`.
fn verify_num_docs(dir: &Arc<dyn Directory>, num_docs: i32) -> Result<()> {
    let reader = directory_reader::open(Arc::clone(dir))?;
    assert_equals(&num_docs, &reader.max_doc());
    assert_equals(&num_docs, &reader.num_docs());
    reader.close()
}

/// `private void verifyTermDocs(Directory dir, Term term, int numDocs)`.
fn verify_term_docs(dir: &Arc<dyn Directory>, term: Term, num_docs: i32) -> Result<()> {
    let reader = directory_reader::open(Arc::clone(dir))?;
    let index_reader: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let mut postings_enum = TestUtil::docs(
        &*random(),
        &index_reader,
        term.field(),
        term.bytes(),
        None,
        POSTINGS_ENUM_NONE,
    )?
    .unwrap_or_else(|| npe("postingsEnum"));
    let mut count = 0;
    while postings_enum.next_doc()? != NO_MORE_DOCS {
        count += 1;
    }
    assert_equals(&num_docs, &count);
    reader.close()
}

/// `private void setUpDirs(Directory dir, Directory aux)`.
fn set_up_dirs(dir: &Arc<dyn Directory>, aux: &Arc<dyn Directory>) -> Result<()> {
    set_up_dirs_with_id(dir, aux, false)
}

/// `private void setUpDirs(Directory dir, Directory aux, boolean withID)`.
fn set_up_dirs_with_id(
    dir: &Arc<dyn Directory>,
    aux: &Arc<dyn Directory>,
    with_id: bool,
) -> Result<()> {
    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_max_buffered_docs(1000)?;
    let mut writer = new_writer(dir, c)?;
    // add 1000 documents in 1 segment
    if with_id {
        add_docs_with_id(&writer, 1000, 0)?;
    } else {
        add_docs(&writer, 1000)?;
    }
    assert_equals(&1000, &writer.get_doc_stats()?.max_doc);
    assert_equals(&1, &writer.get_segment_count()?);
    writer.close()?;

    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_max_buffered_docs(1000)?
        .set_merge_policy(Some(Arc::new(
            LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
        )))?;
    writer = new_writer(aux, c)?;
    // add 30 documents in 3 segments
    for i in 0..3 {
        if with_id {
            add_docs_with_id(&writer, 10, 10 * i)?;
        } else {
            add_docs(&writer, 10)?;
        }
        writer.close()?;
        let mut c = new_iwc();
        c.set_open_mode(OpenMode::APPEND)?
            .set_max_buffered_docs(1000)?
            .set_merge_policy(Some(Arc::new(
                LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
            )))?;
        writer = new_writer(aux, c)?;
    }
    assert_equals(&30, &writer.get_doc_stats()?.max_doc);
    assert_equals(&3, &writer.get_segment_count()?);
    writer.close()
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setOpenMode(mode)`.
fn iwc_mode(mode: OpenMode) -> Result<IndexWriterConfig> {
    let mut c = new_iwc();
    c.set_open_mode(mode)?;
    Ok(c)
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setOpenMode(APPEND)
/// .setMaxBufferedDocs(maxBufferedDocs).setMergePolicy(newLogMergePolicy(4))`.
fn iwc_append_buffered(max_buffered_docs: i32) -> Result<IndexWriterConfig> {
    let mut c = new_iwc();
    c.set_open_mode(OpenMode::APPEND)?
        .set_max_buffered_docs(max_buffered_docs)?
        .set_merge_policy(Some(Arc::new(
            LuceneTestCase::new_log_merge_policy_merge_factor(4)?,
        )))?;
    Ok(c)
}

/// The override of `findMerges(CodecReader...)`; the first argument is the
/// superclass (`super.findMerges(readers)`).
type FindMergesForReaders = Arc<
    dyn Fn(&FilterMergePolicy, &[Arc<dyn CodecReader>]) -> Result<Option<MergeSpecification>>
        + Send
        + Sync,
>;

/// A `MergePolicy` subclass that overrides only `findMerges(CodecReader...)`:
/// every other decision is the superclass's.
struct ReadersMergePolicy {
    filter: FilterMergePolicy,
    find: FindMergesForReaders,
}

impl ReadersMergePolicy {
    fn new(superclass: Arc<dyn MergePolicy>, find: FindMergesForReaders) -> Self {
        ReadersMergePolicy {
            filter: FilterMergePolicy::new(superclass),
            find,
        }
    }
}

impl std::fmt::Display for ReadersMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.filter.get_delegate(), f)
    }
}

impl MergePolicy for ReadersMergePolicy {
    /// The subclass is an instance of its superclass.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.filter.get_delegate().as_any()
    }

    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.filter
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<Option<MergeSpecification>> {
        (self.find)(&self.filter, readers)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.filter.find_forced_merges(
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
        self.filter
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.filter
            .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.filter
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.filter.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.filter.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.filter.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.filter.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.filter.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.filter.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.filter.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.filter.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
    ) -> Result<i32> {
        self.filter
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.filter.max_full_flush_merge_size()
    }
}

/// `ConcurrentAddIndexesMergePolicy.findMerges(CodecReader...)`: create a
/// oneMerge for each reader to let them get concurrently processed by
/// addIndexes().
fn concurrent_add_indexes_find(
    _super: &FilterMergePolicy,
    readers: &[Arc<dyn CodecReader>],
) -> Result<Option<MergeSpecification>> {
    let mut merge_spec = MergeSpecification::new();
    for reader in readers {
        merge_spec.add(rucene::index::merge_policy::OneMerge::from_codec_readers(
            vec![Arc::clone(reader)],
        ));
    }
    if verbose() {
        println!(
            "No. of addIndexes merges returned by MergePolicy: {}",
            merge_spec.merges.len()
        );
    }
    Ok(Some(merge_spec))
}

/// `private class ConcurrentAddIndexesMergePolicy extends TieredMergePolicy`.
fn concurrent_add_indexes_merge_policy() -> ReadersMergePolicy {
    ReadersMergePolicy::new(
        Arc::new(TieredMergePolicy::new()),
        Arc::new(concurrent_add_indexes_find),
    )
}

/// `private class AddIndexesWithReadersSetup`.
struct AddIndexesWithReadersSetup {
    dir: TestDirectory,
    dest_dir: TestDirectory,
    dest_writer: Arc<IndexWriter>,
    readers: Vec<Arc<dyn DirectoryReader>>,
}

impl AddIndexesWithReadersSetup {
    const ADDED_DOCS_PER_READER: i32 = 15;
    const INIT_DOCS: i32 = 25;
    const NUM_READERS: i32 = 15;

    fn new(ms: Arc<dyn MergeScheduler>, mp: Arc<dyn MergePolicy>) -> Result<Self> {
        let dir = mock_byte_buffers_dir();
        let mut c = plain_iwc();
        c.set_max_buffered_docs(2)?;
        let writer = IndexWriter::new(d(&dir), c)?;
        for _ in 0..Self::ADDED_DOCS_PER_READER {
            add_doc(&writer)?;
        }
        writer.close()?;

        let dest_dir = LuceneTestCase::new_directory();
        let mut iwc = plain_iwc();
        iwc.set_merge_policy(Some(mp))?;
        iwc.set_merge_scheduler(Some(ms))?;
        let dest_writer = IndexWriter::new(d(&dest_dir), iwc)?;
        for _ in 0..Self::INIT_DOCS {
            add_doc(&dest_writer)?;
        }
        dest_writer.commit()?;

        let mut readers = Vec::with_capacity(Self::NUM_READERS as usize);
        for _ in 0..Self::NUM_READERS {
            readers.push(directory_reader::open(d(&dir))?);
        }
        Ok(AddIndexesWithReadersSetup {
            dir,
            dest_dir,
            dest_writer,
            readers,
        })
    }

    fn close_all(&mut self) -> Result<()> {
        self.dest_writer.close()?;
        for reader in &self.readers {
            reader.close()?;
        }
        self.dest_dir.close()?;
        self.dir.close()
    }
}

/// `private class PartialMergeScheduler extends MergeScheduler`.
#[derive(Debug)]
struct PartialMergeScheduler {
    merges_to_do: i32,
    merges_triggered: AtomicI32,
    executor: Arc<SameThreadExecutorService>,
}

impl PartialMergeScheduler {
    fn new(merges_to_do: i32) -> Self {
        if verbose() {
            println!(
                "PartialMergeScheduler configured to mark all merges as failed, after triggering [{merges_to_do}] merges."
            );
        }
        PartialMergeScheduler {
            merges_to_do,
            merges_triggered: AtomicI32::new(0),
            executor: Arc::new(SameThreadExecutorService::new()),
        }
    }
}

impl std::fmt::Display for PartialMergeScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PartialMergeScheduler")
    }
}

impl MergeScheduler for PartialMergeScheduler {
    fn merge(&self, merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
        loop {
            let Some(merge) = merge_source.get_next_merge()? else {
                break;
            };
            if self.merges_triggered.load(Ordering::SeqCst) >= self.merges_to_do {
                merge.close(false, false, |_mr| Ok(()))?;
                merge_source.on_merge_finished(&merge);
            } else {
                merge_source.merge(merge)?;
                self.merges_triggered.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    fn same_thread_executor(&self) -> &Arc<SameThreadExecutorService> {
        &self.executor
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// `private class CountingSerialMergeScheduler extends MergeScheduler`.
#[derive(Debug)]
struct CountingSerialMergeScheduler {
    explicit_merges: AtomicI32,
    add_indexes_merges: AtomicI32,
    executor: Arc<SameThreadExecutorService>,
}

impl CountingSerialMergeScheduler {
    fn new() -> Self {
        CountingSerialMergeScheduler {
            explicit_merges: AtomicI32::new(0),
            add_indexes_merges: AtomicI32::new(0),
            executor: Arc::new(SameThreadExecutorService::new()),
        }
    }
}

impl std::fmt::Display for CountingSerialMergeScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CountingSerialMergeScheduler")
    }
}

impl MergeScheduler for CountingSerialMergeScheduler {
    fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
        loop {
            let Some(merge) = merge_source.get_next_merge()? else {
                break;
            };
            merge_source.merge(merge)?;
            if trigger == MergeTrigger::Explicit {
                self.explicit_merges.fetch_add(1, Ordering::SeqCst);
            }
            if trigger == MergeTrigger::AddIndexes {
                self.add_indexes_merges.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    fn same_thread_executor(&self) -> &Arc<SameThreadExecutorService> {
        &self.executor
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// The subclass of `RunAddIndexesThreads`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// `CommitAndAddIndexes`.
    CommitAndAddIndexes,
    /// `CommitAndAddIndexes2 extends CommitAndAddIndexes`.
    CommitAndAddIndexes2,
    /// `CommitAndAddIndexes3`.
    CommitAndAddIndexes3,
}

/// The state of `RunAddIndexesThreads` its threads share.
struct RunShared {
    variant: Variant,
    dir: Arc<dyn Directory>,
    writer2: Arc<IndexWriter>,
    failures: Mutex<Vec<Throwable>>,
    did_close: AtomicBool,
    readers: Vec<Arc<dyn DirectoryReader>>,
    num_copy: usize,
}

/// `private abstract class RunAddIndexesThreads`.
struct RunAddIndexesThreads {
    shared: Arc<RunShared>,
    _dir: TestDirectory,
    dir2: TestDirectory,
    num_threads: usize,
    threads: Vec<Thread>,
}

impl RunAddIndexesThreads {
    const NUM_INIT_DOCS: i32 = 17;

    fn new(variant: Variant, num_copy: usize) -> Result<Self> {
        let dir = mock_byte_buffers_dir();
        let mut c = plain_iwc();
        c.set_max_buffered_docs(2)?;
        let writer = IndexWriter::new(d(&dir), c)?;
        for _ in 0..Self::NUM_INIT_DOCS {
            add_doc(&writer)?;
        }
        writer.close()?;

        let dir2 = LuceneTestCase::new_directory();
        let mp: Arc<dyn MergePolicy> = Arc::new(concurrent_add_indexes_merge_policy());
        let mut iwc = plain_iwc();
        iwc.set_merge_policy(Some(mp))?;
        let writer2 = IndexWriter::new(d(&dir2), iwc)?;
        writer2.commit()?;

        let mut readers = Vec::with_capacity(num_copy);
        for _ in 0..num_copy {
            readers.push(directory_reader::open(d(&dir))?);
        }
        let num_threads = if *TEST_NIGHTLY { 5 } else { 2 };
        Ok(RunAddIndexesThreads {
            shared: Arc::new(RunShared {
                variant,
                dir: d(&dir),
                writer2,
                failures: Mutex::new(Vec::new()),
                did_close: AtomicBool::new(false),
                readers,
                num_copy,
            }),
            _dir: dir,
            dir2,
            num_threads,
            threads: Vec::new(),
        })
    }

    fn writer2(&self) -> &Arc<IndexWriter> {
        &self.shared.writer2
    }

    fn failures(&self) -> Vec<Throwable> {
        self.shared
            .failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn launch_threads(&mut self, num_iter: i32) {
        for _ in 0..self.num_threads {
            let shared = Arc::clone(&self.shared);
            self.threads.push(Thread::unnamed(move || {
                let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
                    let mut mocks = Vec::with_capacity(shared.num_copy);
                    for _ in 0..shared.num_copy {
                        mocks.push(mock_ram_copy_of(&shared.dir)?);
                    }
                    let dirs: Vec<Arc<dyn Directory>> = mocks
                        .iter()
                        .map(MockDirectoryWrapper::as_directory)
                        .collect();
                    let mut j = 0;
                    loop {
                        if num_iter > 0 && j == num_iter {
                            break;
                        }
                        let current = j;
                        j += 1;
                        shared.do_body(current, &dirs)?;
                    }
                    Ok(())
                }));
                match outcome {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => shared.handle(java_throwable(&e), Some(&e)),
                    Err(payload) => shared.handle(Throwable::from_panic(payload), None),
                }
            }));
        }
        for thread in &self.threads {
            thread.start();
        }
    }

    fn join_threads(&self) {
        for thread in &self.threads {
            let _ = thread.join();
        }
    }

    fn close(&self, do_wait: bool) -> Result<()> {
        self.shared.did_close.store(true, Ordering::SeqCst);
        if !do_wait {
            self.shared.writer2.rollback()
        } else {
            self.shared.writer2.close()
        }
    }

    fn close_dir(&mut self) -> Result<()> {
        for reader in &self.shared.readers {
            reader.close()?;
        }
        self.dir2.close()
    }
}

impl RunShared {
    fn thread_name() -> String {
        Thread::current().name()
    }

    fn do_body(&self, j: i32, dirs: &[Arc<dyn Directory>]) -> Result<()> {
        match self.variant {
            Variant::CommitAndAddIndexes | Variant::CommitAndAddIndexes2 => match j % 5 {
                0 => {
                    if verbose() {
                        println!(
                            "{}: TEST: addIndexes(Dir[]) then full merge",
                            Self::thread_name()
                        );
                    }
                    self.writer2.add_indexes_dirs(dirs)?;
                    match self.writer2.force_merge(1) {
                        Ok(()) => {}
                        Err(ioe) if is_instance(&ioe, "java.io.IOException") => {
                            if cause_is_merge_aborted(&ioe) {
                                // OK
                            } else {
                                return Err(ioe);
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                1 => {
                    if verbose() {
                        println!("{}: TEST: addIndexes(Dir[])", Self::thread_name());
                    }
                    self.writer2.add_indexes_dirs(dirs)?;
                }
                2 => {
                    if verbose() {
                        println!("{}: TEST: addIndexes(LeafReader[])", Self::thread_name());
                    }
                    TestUtil::add_indexes_slowly(&self.writer2, &self.readers)?;
                }
                3 => {
                    if verbose() {
                        println!(
                            "{}: TEST: addIndexes(Dir[]) then maybeMerge",
                            Self::thread_name()
                        );
                    }
                    self.writer2.add_indexes_dirs(dirs)?;
                    self.writer2.maybe_merge()?;
                }
                4 => {
                    if verbose() {
                        println!("{}: TEST: commit", Self::thread_name());
                    }
                    self.writer2.commit()?;
                }
                _ => {}
            },
            Variant::CommitAndAddIndexes3 => match j % 5 {
                0 => {
                    if verbose() {
                        println!("TEST: {}: addIndexes + full merge", Self::thread_name());
                    }
                    self.writer2.add_indexes_dirs(dirs)?;
                    self.writer2.force_merge(1)?;
                }
                1 => {
                    if verbose() {
                        println!("TEST: {}: addIndexes", Self::thread_name());
                    }
                    self.writer2.add_indexes_dirs(dirs)?;
                }
                2 => {
                    if verbose() {
                        println!("TEST: {}: addIndexes(LR[])", Self::thread_name());
                    }
                    TestUtil::add_indexes_slowly(&self.writer2, &self.readers)?;
                }
                3 => {
                    if verbose() {
                        println!("TEST: {}: full merge", Self::thread_name());
                    }
                    self.writer2.force_merge(1)?;
                }
                4 => {
                    if verbose() {
                        println!("TEST: {}: commit", Self::thread_name());
                    }
                    self.writer2.commit()?;
                }
                _ => {}
            },
        }
        Ok(())
    }

    /// `handle(Throwable t)`; `error` is the Rucene error `t` stands for,
    /// when the body returned one (`t.getCause()` reads it).
    fn handle(&self, t: Throwable, error: Option<&LuceneError>) {
        let report = match self.variant {
            Variant::CommitAndAddIndexes => true,
            Variant::CommitAndAddIndexes2 => {
                !t.is_instance("org.apache.lucene.store.AlreadyClosedException")
                    && !t.is_instance("java.lang.NullPointerException")
            }
            Variant::CommitAndAddIndexes3 => {
                let mut report = true;
                if t.is_instance("org.apache.lucene.store.AlreadyClosedException")
                    || t.is_instance("org.apache.lucene.index.MergePolicy$MergeAbortedException")
                    || t.is_instance("java.lang.NullPointerException")
                {
                    report = !self.did_close.load(Ordering::SeqCst);
                } else if t.is_instance("java.io.FileNotFoundException")
                    || t.is_instance("java.nio.file.NoSuchFileException")
                {
                    report = !self.did_close.load(Ordering::SeqCst);
                } else if t.is_instance("java.io.IOException") {
                    if error.is_some_and(cause_is_merge_aborted) {
                        report = !self.did_close.load(Ordering::SeqCst);
                    }
                }
                report
            }
        };
        if report {
            println!("{t}");
            self.failures
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(t);
        }
    }
}

/// `List.toString()` of the failures.
fn failures_string(failures: &[Throwable]) -> String {
    let items: Vec<String> = failures.iter().map(ToString::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// `private static final class CustomPerFieldCodec extends AssertingCodec`.
fn custom_per_field_codec() -> AssertingCodec {
    let direct_format = rucene::codecs::postings_for_name("Direct").unwrap_or_else(|| {
        Throwable::new(
            IAE,
            "An SPI class of type org.apache.lucene.codecs.PostingsFormat with name 'Direct' does not exist.",
        )
        .throw()
    });
    let default_format: Arc<dyn PostingsFormat> = TestUtil::get_default_postings_format();
    AssertingCodec::with_formats_for_field(
        Some(Arc::new(move |field: &str| {
            if field == "id" {
                Arc::clone(&direct_format)
            } else {
                Arc::clone(&default_format)
            }
        })),
        None,
        None,
    )
}

/// `private static final class UnRegisteredCodec extends FilterCodec`.
fn un_registered_codec() -> FilterCodec {
    FilterCodec::new("NotRegistered", TestUtil::get_default_codec())
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

/// Port of `TestAddIndexes.testSimpleCase`.
fn test_simple_case() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // two auxiliary directories
    let mut aux = LuceneTestCase::new_directory();
    let mut aux2 = LuceneTestCase::new_directory();

    let mut writer = new_writer(&d(&dir), iwc_mode(OpenMode::CREATE)?)?;
    // add 100 documents
    add_docs(&writer, 100)?;
    assert_equals(&100, &writer.get_doc_stats()?.max_doc);
    writer.close()?;
    TestUtil::check_index(d(&dir))?;

    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(false)))?;
    writer = new_writer(&d(&aux), c)?;
    // add 40 documents in separate files
    add_docs(&writer, 40)?;
    assert_equals(&40, &writer.get_doc_stats()?.max_doc);
    writer.close()?;

    writer = new_writer(&d(&aux2), iwc_mode(OpenMode::CREATE)?)?;
    // add 50 documents in compound files
    add_docs2(&writer, 50)?;
    assert_equals(&50, &writer.get_doc_stats()?.max_doc);
    writer.close()?;

    // test doc count before segments are merged
    writer = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;
    assert_equals(&100, &writer.get_doc_stats()?.max_doc);
    writer.add_indexes_dirs(&[d(&aux), d(&aux2)])?;
    assert_equals(&190, &writer.get_doc_stats()?.max_doc);
    writer.close()?;
    TestUtil::check_index(d(&dir))?;

    // make sure the old index is correct
    verify_num_docs(&d(&aux), 40)?;

    // make sure the new index is correct
    verify_num_docs(&d(&dir), 190)?;

    // now add another set in.
    let mut aux3 = LuceneTestCase::new_directory();
    writer = new_writer(&d(&aux3), new_iwc())?;
    // add 40 documents
    add_docs(&writer, 40)?;
    assert_equals(&40, &writer.get_doc_stats()?.max_doc);
    writer.close()?;

    // test doc count before segments are merged
    writer = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;
    assert_equals(&190, &writer.get_doc_stats()?.max_doc);
    writer.add_indexes_dirs(&[d(&aux3)])?;
    assert_equals(&230, &writer.get_doc_stats()?.max_doc);
    writer.close()?;

    // make sure the new index is correct
    verify_num_docs(&d(&dir), 230)?;

    verify_term_docs(&d(&dir), Term::from_text("content", "aaa"), 180)?;

    verify_term_docs(&d(&dir), Term::from_text("content", "bbb"), 50)?;

    // now fully merge it.
    writer = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;
    writer.force_merge(1)?;
    writer.close()?;

    // make sure the new index is correct
    verify_num_docs(&d(&dir), 230)?;

    verify_term_docs(&d(&dir), Term::from_text("content", "aaa"), 180)?;

    verify_term_docs(&d(&dir), Term::from_text("content", "bbb"), 50)?;

    // now add a single document
    let mut aux4 = LuceneTestCase::new_directory();
    writer = new_writer(&d(&aux4), new_iwc())?;
    add_docs2(&writer, 1)?;
    writer.close()?;

    writer = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;
    assert_equals(&230, &writer.get_doc_stats()?.max_doc);
    writer.add_indexes_dirs(&[d(&aux4)])?;
    assert_equals(&231, &writer.get_doc_stats()?.max_doc);
    writer.close()?;

    verify_num_docs(&d(&dir), 231)?;

    verify_term_docs(&d(&dir), Term::from_text("content", "bbb"), 51)?;
    dir.close()?;
    aux.close()?;
    aux2.close()?;
    aux3.close()?;
    aux4.close()
}

/// The loop the `testWithPendingDeletes*` methods share: adds 10 docs, then
/// replaces them with another 10 docs, so 10 pending deletes.
fn update_twenty_docs(writer: &IndexWriter) -> Result<()> {
    for i in 0..20 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &(i % 10).to_string(),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("bbb {i}"),
            Store::NO,
        )));
        doc.add(Box::new(IntPoint::new("doc", &[i])?));
        doc.add(Box::new(IntPoint::new("doc2d", &[i, i])?));
        doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
        writer.update_document(Term::from_text("id", &(i % 10).to_string()), &doc)?;
    }
    Ok(())
}

/// Deletes one of the 10 added docs, leaving 9:
/// `writer.deleteDocuments(new PhraseQuery("content", "bbb", "14"))`.
fn delete_bbb_14(writer: &IndexWriter) -> Result<()> {
    let q = PhraseQuery::from_strings("content", &["bbb", "14"])?;
    writer.delete_documents_by_queries(vec![Arc::new(q)])?;
    Ok(())
}

/// The checks the `testWithPendingDeletes*` methods share.
fn verify_pending_deletes(dir: &Arc<dyn Directory>) -> Result<()> {
    verify_num_docs(dir, 1039)?;
    verify_term_docs(dir, Term::from_text("content", "aaa"), 1030)?;
    verify_term_docs(dir, Term::from_text("content", "bbb"), 9)
}

/// Port of `TestAddIndexes.testWithPendingDeletes`.
fn test_with_pending_deletes() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    set_up_dirs(&d(&dir), &d(&aux))?;
    let writer = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;
    writer.add_indexes_dirs(&[d(&aux)])?;

    // Adds 10 docs, then replaces them with another 10
    // docs, so 10 pending deletes:
    update_twenty_docs(&writer)?;
    // Deletes one of the 10 added docs, leaving 9:
    delete_bbb_14(&writer)?;

    writer.force_merge(1)?;
    writer.commit()?;

    verify_pending_deletes(&d(&dir))?;

    writer.close()?;
    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testWithPendingDeletes2`.
fn test_with_pending_deletes2() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    set_up_dirs(&d(&dir), &d(&aux))?;
    let writer = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;

    // Adds 10 docs, then replaces them with another 10
    // docs, so 10 pending deletes:
    update_twenty_docs(&writer)?;

    writer.add_indexes_dirs(&[d(&aux)])?;

    // Deletes one of the 10 added docs, leaving 9:
    delete_bbb_14(&writer)?;

    writer.force_merge(1)?;
    writer.commit()?;

    verify_pending_deletes(&d(&dir))?;

    writer.close()?;
    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testWithPendingDeletes3`.
fn test_with_pending_deletes3() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    set_up_dirs(&d(&dir), &d(&aux))?;
    let writer = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;

    // Adds 10 docs, then replaces them with another 10
    // docs, so 10 pending deletes:
    update_twenty_docs(&writer)?;

    // Deletes one of the 10 added docs, leaving 9:
    delete_bbb_14(&writer)?;

    writer.add_indexes_dirs(&[d(&aux)])?;

    writer.force_merge(1)?;
    writer.commit()?;

    verify_pending_deletes(&d(&dir))?;

    writer.close()?;
    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testAddSelf`: case 0: add self or exceed
/// maxMergeDocs, expect exception.
fn test_add_self() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    let mut writer = new_writer(&d(&dir), new_iwc())?;
    // add 100 documents
    add_docs(&writer, 100)?;
    assert_equals(&100, &writer.get_doc_stats()?.max_doc);
    writer.close()?;

    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_max_buffered_docs(1000)?
        .set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(false)))?;
    writer = new_writer(&d(&aux), c)?;
    // add 140 documents in separate files
    add_docs(&writer, 40)?;
    writer.close()?;
    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_max_buffered_docs(1000)?
        .set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(false)))?;
    writer = new_writer(&d(&aux), c)?;
    add_docs(&writer, 100)?;
    writer.close()?;

    // cannot add self
    let writer2 = new_writer(&d(&dir), iwc_mode(OpenMode::APPEND)?)?;
    expect_throws(IAE, || writer2.add_indexes_dirs(&[d(&aux), d(&dir)]));
    assert_equals(&100, &writer2.get_doc_stats()?.max_doc);
    writer2.close()?;

    // make sure the index is correct
    verify_num_docs(&d(&dir), 100)?;

    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testNoTailSegments`: in all the remaining tests,
/// make the doc count of the oldest segment in dir large so that it is never
/// merged in addIndexes(); case 1: no tail segments.
fn test_no_tail_segments() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    set_up_dirs(&d(&dir), &d(&aux))?;

    let writer = new_writer(&d(&dir), iwc_append_buffered(10)?)?;
    add_docs(&writer, 10)?;

    writer.add_indexes_dirs(&[d(&aux)])?;
    assert_equals(&1040, &writer.get_doc_stats()?.max_doc);
    assert_equals(&1000, &writer.max_doc_at(0)?);
    writer.close()?;

    // make sure the index is correct
    verify_num_docs(&d(&dir), 1040)?;
    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testNoCopySegments`: case 2: tail segments,
/// invariants hold, no copy.
fn test_no_copy_segments() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    set_up_dirs(&d(&dir), &d(&aux))?;

    let writer = new_writer(&d(&dir), iwc_append_buffered(9)?)?;
    add_docs(&writer, 2)?;

    writer.add_indexes_dirs(&[d(&aux)])?;
    assert_equals(&1032, &writer.get_doc_stats()?.max_doc);
    assert_equals(&1000, &writer.max_doc_at(0)?);
    writer.close()?;

    // make sure the index is correct
    verify_num_docs(&d(&dir), 1032)?;
    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testNoMergeAfterCopy`: case 3: tail segments,
/// invariants hold, copy, invariants hold.
fn test_no_merge_after_copy() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    set_up_dirs(&d(&dir), &d(&aux))?;

    let writer = new_writer(&d(&dir), iwc_append_buffered(10)?)?;

    let copy = mock_ram_copy_of(&d(&aux))?;
    writer.add_indexes_dirs(&[d(&aux), copy.as_directory()])?;
    assert_equals(&1060, &writer.get_doc_stats()?.max_doc);
    assert_equals(&1000, &writer.max_doc_at(0)?);
    writer.close()?;

    // make sure the index is correct
    verify_num_docs(&d(&dir), 1060)?;
    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testMergeAfterCopy`: case 4: tail segments,
/// invariants hold, copy, invariants not hold.
fn test_merge_after_copy() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();

    set_up_dirs_with_id(&d(&dir), &d(&aux), true)?;

    let mut dont_merge_config = plain_iwc();
    dont_merge_config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let writer = IndexWriter::new(d(&aux), dont_merge_config)?;
    for i in 0..20 {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }
    writer.close()?;
    let reader = directory_reader::open(d(&aux))?;
    assert_equals(&10, &reader.num_docs());
    reader.close()?;

    let writer = new_writer(&d(&dir), iwc_append_buffered(4)?)?;

    if verbose() {
        println!("\nTEST: now addIndexes");
    }
    let copy = mock_ram_copy_of(&d(&aux))?;
    writer.add_indexes_dirs(&[d(&aux), copy.as_directory()])?;
    assert_equals(&1020, &writer.get_doc_stats()?.max_doc);
    assert_equals(&1000, &writer.max_doc_at(0)?);
    writer.close()?;
    dir.close()?;
    aux.close()
}

/// Port of `TestAddIndexes.testMoreMerges`: case 5: tail segments,
/// invariants not hold.
fn test_more_merges() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // auxiliary directory
    let mut aux = LuceneTestCase::new_directory();
    let mut aux2 = LuceneTestCase::new_directory();

    set_up_dirs_with_id(&d(&dir), &d(&aux), true)?;

    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_max_buffered_docs(100)?
        .set_merge_policy(Some(Arc::new(
            LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
        )))?;
    let writer = new_writer(&d(&aux2), c)?;
    writer.add_indexes_dirs(&[d(&aux)])?;
    assert_equals(&30, &writer.get_doc_stats()?.max_doc);
    assert_equals(&3, &writer.get_segment_count()?);
    writer.close()?;

    let mut dont_merge_config = plain_iwc();
    dont_merge_config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let writer = IndexWriter::new(d(&aux), dont_merge_config)?;
    for i in 0..27 {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }
    writer.close()?;
    let reader = directory_reader::open(d(&aux))?;
    assert_equals(&3, &reader.num_docs());
    reader.close()?;

    let mut dont_merge_config = plain_iwc();
    dont_merge_config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let writer = IndexWriter::new(d(&aux2), dont_merge_config)?;
    for i in 0..8 {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
    }
    writer.close()?;
    let reader = directory_reader::open(d(&aux2))?;
    assert_equals(&22, &reader.num_docs());
    reader.close()?;

    let writer = new_writer(&d(&dir), iwc_append_buffered(6)?)?;

    writer.add_indexes_dirs(&[d(&aux), d(&aux2)])?;
    assert_equals(&1040, &writer.get_doc_stats()?.max_doc);
    assert_equals(&1000, &writer.max_doc_at(0)?);
    writer.close()?;
    dir.close()?;
    aux.close()?;
    aux2.close()
}

/// Port of `TestAddIndexes.testHangOnClose`: LUCENE-1270.
fn test_hang_on_close() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let lmp = LogMergePolicy::by_byte_size();
    lmp.set_no_cfs_ratio(0.0)?;
    lmp.set_merge_factor(100)?;
    let mut c = new_iwc();
    c.set_max_buffered_docs(5)?
        .set_merge_policy(Some(Arc::new(lmp)))?;
    let writer = IndexWriter::new(d(&dir), c)?;

    let mut doc = Document::new();
    let mut custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    custom_type.set_store_term_vector_offsets(true)?;
    doc.add(Box::new(LuceneTestCase::new_field(
        "content",
        "aaa bbb ccc ddd eee fff ggg hhh iii",
        &custom_type,
    )));
    for _ in 0..60 {
        writer.add_document(&doc)?;
    }

    let mut doc2 = Document::new();
    let mut custom_type2 = FieldType::new();
    custom_type2.set_stored(true)?;
    for _ in 0..4 {
        doc2.add(Box::new(LuceneTestCase::new_field(
            "content",
            "aaa bbb ccc ddd eee fff ggg hhh iii",
            &custom_type2,
        )));
    }
    for _ in 0..10 {
        writer.add_document(&doc2)?;
    }
    writer.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let mut lmp = LogMergePolicy::by_byte_size();
    lmp.set_min_merge_mb(0.0001);
    lmp.set_no_cfs_ratio(0.0)?;
    lmp.set_merge_factor(4)?;
    let mut c = new_iwc();
    c.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?
        .set_merge_policy(Some(Arc::new(lmp)))?;
    let writer = IndexWriter::new(d(&dir2), c)?;
    writer.add_indexes_dirs(&[d(&dir)])?;
    writer.close()?;
    dir.close()?;
    dir2.close()
}

/// Port of `TestAddIndexes.testAddIndexesWithConcurrentMerges`.
fn test_add_indexes_with_concurrent_merges() -> Result<()> {
    let mp = concurrent_add_indexes_merge_policy();
    let mut c =
        AddIndexesWithReadersSetup::new(Arc::new(ConcurrentMergeScheduler::new()), Arc::new(mp))?;
    TestUtil::add_indexes_slowly(&c.dest_writer, &c.readers)?;
    c.dest_writer.commit()?;
    {
        let reader = directory_reader::open(d(&c.dest_dir))?;
        assert_equals(
            &(AddIndexesWithReadersSetup::INIT_DOCS
                + AddIndexesWithReadersSetup::NUM_READERS
                    * AddIndexesWithReadersSetup::ADDED_DOCS_PER_READER),
            &reader.num_docs(),
        );
        reader.close()?;
    }
    c.close_all()
}

/// Port of `TestAddIndexes.testAddIndexesWithPartialMergeFailures`.
fn test_add_indexes_with_partial_merge_failures() -> Result<()> {
    let merges: Arc<Mutex<Vec<rucene::index::merge_policy::OneMerge>>> =
        Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&merges);
    let mp = ReadersMergePolicy::new(
        Arc::new(TieredMergePolicy::new()),
        Arc::new(
            move |sup: &FilterMergePolicy, readers: &[Arc<dyn CodecReader>]| {
                let spec = concurrent_add_indexes_find(sup, readers)?;
                if let Some(spec) = &spec {
                    recorded
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(spec.merges.iter().cloned());
                }
                Ok(spec)
            },
        ),
    );
    let mut c =
        AddIndexesWithReadersSetup::new(Arc::new(PartialMergeScheduler::new(2)), Arc::new(mp))?;
    expect_throws("java.lang.RuntimeException", || {
        TestUtil::add_indexes_slowly(&c.dest_writer, &c.readers)
    });
    c.dest_writer.commit()?;

    // verify no docs got added and all interim files from successful merges have been deleted
    {
        let reader = directory_reader::open(d(&c.dest_dir))?;
        assert_equals(&AddIndexesWithReadersSetup::INIT_DOCS, &reader.num_docs());
        reader.close()?;
    }
    let merges = merges.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for merge in &merges {
        if let Some(info) = merge.get_merge_info() {
            let listed: HashSet<String> = d(&c.dest_dir).list_all()?.into_iter().collect();
            let files = info.files()?;
            Assert::assert_false(files.iter().all(|f| listed.contains(f)));
        }
    }
    c.close_all()
}

/// Port of `TestAddIndexes.testAddIndexesWithNullMergeSpec`.
fn test_add_indexes_with_null_merge_spec() -> Result<()> {
    let mp = ReadersMergePolicy::new(
        Arc::new(TieredMergePolicy::new()),
        Arc::new(|_sup: &FilterMergePolicy, _readers: &[Arc<dyn CodecReader>]| Ok(None)),
    );
    let mut c =
        AddIndexesWithReadersSetup::new(Arc::new(ConcurrentMergeScheduler::new()), Arc::new(mp))?;
    TestUtil::add_indexes_slowly(&c.dest_writer, &c.readers)?;
    c.dest_writer.commit()?;
    {
        let reader = directory_reader::open(d(&c.dest_dir))?;
        assert_equals(&AddIndexesWithReadersSetup::INIT_DOCS, &reader.num_docs());
        reader.close()?;
    }
    c.close_all()
}

/// Port of `TestAddIndexes.testAddIndexesWithEmptyMergeSpec`.
fn test_add_indexes_with_empty_merge_spec() -> Result<()> {
    let mp = ReadersMergePolicy::new(
        Arc::new(TieredMergePolicy::new()),
        Arc::new(
            |_sup: &FilterMergePolicy, _readers: &[Arc<dyn CodecReader>]| {
                Ok(Some(MergeSpecification::new()))
            },
        ),
    );
    let mut c =
        AddIndexesWithReadersSetup::new(Arc::new(ConcurrentMergeScheduler::new()), Arc::new(mp))?;
    TestUtil::add_indexes_slowly(&c.dest_writer, &c.readers)?;
    c.dest_writer.commit()?;
    {
        let reader = directory_reader::open(d(&c.dest_dir))?;
        assert_equals(&AddIndexesWithReadersSetup::INIT_DOCS, &reader.num_docs());
        reader.close()?;
    }
    c.close_all()
}

/// Port of `TestAddIndexes.testAddIndexesWithEmptyReaders`.
fn test_add_indexes_with_empty_readers() -> Result<()> {
    let mut dest_dir = LuceneTestCase::new_directory();
    let mut iwc = plain_iwc();
    iwc.set_merge_policy(Some(Arc::new(concurrent_add_indexes_merge_policy())))?;
    let ms = Arc::new(CountingSerialMergeScheduler::new());
    iwc.set_merge_scheduler(Some(Arc::clone(&ms) as Arc<dyn MergeScheduler>))?;
    let dest_writer = IndexWriter::new(d(&dest_dir), iwc)?;
    let initial_docs = 15;
    for _ in 0..initial_docs {
        add_doc(&dest_writer)?;
    }
    dest_writer.commit()?;

    // create empty readers
    let mut dir = mock_byte_buffers_dir();
    let writer = IndexWriter::new(d(&dir), plain_iwc())?;
    writer.close()?;
    let num_readers = 20;
    let mut readers: Vec<Arc<dyn DirectoryReader>> = Vec::with_capacity(num_readers);
    for _ in 0..num_readers {
        readers.push(directory_reader::open(d(&dir))?);
    }

    TestUtil::add_indexes_slowly(&dest_writer, &readers)?;
    dest_writer.commit()?;

    // verify no docs were added
    {
        let reader = directory_reader::open(d(&dest_dir))?;
        assert_equals(&initial_docs, &reader.num_docs());
        reader.close()?;
    }
    // verify no merges were triggered
    assert_equals(&0, &ms.add_indexes_merges.load(Ordering::SeqCst));

    dest_writer.close()?;
    for reader in &readers {
        reader.close()?;
    }
    dest_dir.close()?;
    dir.close()
}

/// Port of `TestAddIndexes.testCascadingMergesTriggered`.
fn test_cascading_merges_triggered() -> Result<()> {
    let mp = concurrent_add_indexes_merge_policy();
    let ms = Arc::new(CountingSerialMergeScheduler::new());
    let mut c =
        AddIndexesWithReadersSetup::new(Arc::clone(&ms) as Arc<dyn MergeScheduler>, Arc::new(mp))?;
    TestUtil::add_indexes_slowly(&c.dest_writer, &c.readers)?;
    Assert::assert_true(ms.explicit_merges.load(Ordering::SeqCst) > 0);
    c.close_all()
}

/// Port of `TestAddIndexes.testAddIndexesHittingMaxDocsLimit`.
fn test_add_indexes_hitting_max_docs_limit() -> Result<()> {
    let writer_max_docs = 15;
    index_writer::set_max_docs(writer_max_docs)?;

    // create destination writer
    let mut dest_dir = LuceneTestCase::new_directory();
    let mut iwc = plain_iwc();
    iwc.set_merge_policy(Some(Arc::new(concurrent_add_indexes_merge_policy())))?;
    let ms = Arc::new(CountingSerialMergeScheduler::new());
    iwc.set_merge_scheduler(Some(Arc::clone(&ms) as Arc<dyn MergeScheduler>))?;
    let dest_writer = IndexWriter::new(d(&dest_dir), iwc)?;
    for _ in 0..writer_max_docs {
        add_doc(&dest_writer)?;
    }
    dest_writer.commit()?;

    // create readers to add
    let mut dir = mock_byte_buffers_dir();
    let writer = IndexWriter::new(d(&dir), plain_iwc())?;
    for _ in 0..10 {
        add_doc(&writer)?;
    }
    writer.close()?;
    let num_readers = 20;
    let mut readers: Vec<Arc<dyn DirectoryReader>> = Vec::with_capacity(num_readers);
    for _ in 0..num_readers {
        readers.push(directory_reader::open(d(&dir))?);
    }

    let mut success = false;
    match TestUtil::add_indexes_slowly(&dest_writer, &readers) {
        Ok(()) => success = true,
        Err(e) if is_instance(&e, IAE) => {
            let message = java_throwable(&e).message().unwrap_or_default().to_string();
            Assert::assert_true(message.contains(&format!(
                "number of documents in the index cannot exceed {writer_max_docs}"
            )));
        }
        Err(e) => return Err(e),
    }
    Assert::assert_false(success);

    // verify no docs were added
    dest_writer.commit()?;
    {
        let reader = directory_reader::open(d(&dest_dir))?;
        assert_equals(&writer_max_docs, &reader.num_docs());
        reader.close()?;
    }

    dest_writer.close()?;
    for reader in &readers {
        reader.close()?;
    }
    dest_dir.close()?;
    dir.close()
}

/// Port of `TestAddIndexes.testAddIndexesWithThreads`: LUCENE-1335: test
/// simultaneous addIndexes & commits from multiple threads.
fn test_add_indexes_with_threads() -> Result<()> {
    let num_iter = if *TEST_NIGHTLY { 15 } else { 5 };
    let num_copy = 3;
    let mut c = RunAddIndexesThreads::new(Variant::CommitAndAddIndexes, num_copy)?;
    c.launch_threads(num_iter);

    for _ in 0..100 {
        add_doc(c.writer2())?;
    }

    c.join_threads();

    let expected_num_docs = 100
        + num_copy as i32
            * (4 * num_iter / 5)
            * c.num_threads as i32
            * RunAddIndexesThreads::NUM_INIT_DOCS;
    assert_equals_msg(
        Some(&format!(
            "expected num docs don't match - failures: {}",
            failures_string(&c.failures())
        )),
        &expected_num_docs,
        &c.writer2().get_doc_stats()?.num_docs,
    );

    c.close(true)?;

    Assert::assert_true_msg(
        &format!(
            "found unexpected failures: {}",
            failures_string(&c.failures())
        ),
        c.failures().is_empty(),
    );

    let reader = directory_reader::open(d(&c.dir2))?;
    assert_equals(&expected_num_docs, &reader.num_docs());
    reader.close()?;

    c.close_dir()
}

/// Port of `TestAddIndexes.testAddIndexesWithClose`: LUCENE-1335: test
/// simultaneous addIndexes & close.
fn test_add_indexes_with_close() -> Result<()> {
    let num_copy = 3;
    let mut c = RunAddIndexesThreads::new(Variant::CommitAndAddIndexes2, num_copy)?;
    // c.writer2.setInfoStream(System.out);
    c.launch_threads(-1);

    // Close w/o first stopping/joining the threads
    c.close(true)?;
    // c.writer2.close();

    c.join_threads();

    c.close_dir()?;

    Assert::assert_true(c.failures().is_empty());
    Ok(())
}

/// Port of `TestAddIndexes.testAddIndexesWithCloseNoWait`: LUCENE-1335: test
/// simultaneous addIndexes & close.
fn test_add_indexes_with_close_no_wait() -> Result<()> {
    let num_copy = 50;
    let mut c = RunAddIndexesThreads::new(Variant::CommitAndAddIndexes3, num_copy)?;
    c.launch_threads(-1);

    Thread::sleep(TestUtil::next_int(&*random(), 10, 500) as i64);

    // Close w/o first stopping/joining the threads
    if verbose() {
        println!("TEST: now close(false)");
    }
    c.close(false)?;

    c.join_threads();

    if verbose() {
        println!("TEST: done join threads");
    }
    c.close_dir()?;

    Assert::assert_true(c.failures().is_empty());
    Ok(())
}

/// Port of `TestAddIndexes.testAddIndexesWithRollback`: LUCENE-1335: test
/// simultaneous addIndexes & close.
fn test_add_indexes_with_rollback() -> Result<()> {
    let num_copy = if *TEST_NIGHTLY { 50 } else { 5 };
    let mut c = RunAddIndexesThreads::new(Variant::CommitAndAddIndexes3, num_copy)?;
    c.launch_threads(-1);

    Thread::sleep(TestUtil::next_int(&*random(), 10, 500) as i64);

    // Close w/o first stopping/joining the threads
    if verbose() {
        println!("TEST: now force rollback");
    }
    c.shared.did_close.store(true, Ordering::SeqCst);
    let ms = c.writer2().get_config().merge_scheduler();

    c.writer2().rollback()?;

    c.join_threads();

    if let Some(cms) = ms
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
    {
        assert_equals(&0, &cms.merge_thread_count());
    }

    c.close_dir()?;

    Assert::assert_true(c.failures().is_empty());
    Ok(())
}

/// Port of `TestAddIndexes.testExistingDeletes`: LUCENE-2996: tests that
/// addIndexes(IndexReader) applies existing deletes correctly.
fn test_existing_deletes() -> Result<()> {
    let mut dirs: Vec<TestDirectory> = Vec::with_capacity(2);
    for _ in 0..2 {
        let dir = LuceneTestCase::new_directory();
        let conf = new_iwc();
        let writer = IndexWriter::new(d(&dir), conf)?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", "myid".into(), Store::NO)?));
        writer.add_document(&doc)?;
        writer.close()?;
        dirs.push(dir);
    }

    let conf = plain_iwc();
    let writer = IndexWriter::new(d(&dirs[0]), conf)?;

    // Now delete the document
    writer.delete_documents_by_terms(vec![Term::from_text("id", "myid")])?;
    {
        let r = directory_reader::open(d(&dirs[1]))?;
        let outcome = TestUtil::add_indexes_slowly(&writer, &[Arc::clone(&r)]);
        let closed = r.close();
        outcome?;
        closed?;
    }
    writer.commit()?;
    assert_equals_msg(
        Some("Documents from the incoming index should not have been deleted"),
        &1,
        &writer.get_doc_stats()?.num_docs,
    );
    writer.close()?;

    for dir in &mut dirs {
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestAddIndexes.testSimpleCaseCustomCodec`.
fn test_simple_case_custom_codec() -> Result<()> {
    // main directory
    let mut dir = LuceneTestCase::new_directory();
    // two auxiliary directories
    let mut aux = LuceneTestCase::new_directory();
    let mut aux2 = LuceneTestCase::new_directory();
    let codec: Arc<dyn Codec> = Arc::new(custom_per_field_codec());

    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_codec(Arc::clone(&codec))?;
    let mut writer = new_writer(&d(&dir), c)?;
    // add 100 documents
    add_docs_with_id(&writer, 100, 0)?;
    assert_equals(&100, &writer.get_doc_stats()?.max_doc);
    writer.commit()?;
    writer.close()?;
    TestUtil::check_index(d(&dir))?;

    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_codec(Arc::clone(&codec))?
        .set_max_buffered_docs(10)?
        .set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(false)))?;
    writer = new_writer(&d(&aux), c)?;
    // add 40 documents in separate files
    add_docs(&writer, 40)?;
    assert_equals(&40, &writer.get_doc_stats()?.max_doc);
    writer.commit()?;
    writer.close()?;

    let mut c = new_iwc();
    c.set_open_mode(OpenMode::CREATE)?
        .set_codec(Arc::clone(&codec))?;
    writer = new_writer(&d(&aux2), c)?;
    // add 40 documents in compound files
    add_docs2(&writer, 50)?;
    assert_equals(&50, &writer.get_doc_stats()?.max_doc);
    writer.commit()?;
    writer.close()?;

    // test doc count before segments are merged
    let mut c = new_iwc();
    c.set_open_mode(OpenMode::APPEND)?
        .set_codec(Arc::clone(&codec))?;
    writer = new_writer(&d(&dir), c)?;
    assert_equals(&100, &writer.get_doc_stats()?.max_doc);
    writer.add_indexes_dirs(&[d(&aux), d(&aux2)])?;
    assert_equals(&190, &writer.get_doc_stats()?.max_doc);
    writer.close()?;

    dir.close()?;
    aux.close()?;
    aux2.close()
}

/// Port of `TestAddIndexes.testNonCFSLeftovers`: LUCENE-2790: tests that the
/// non CFS files were deleted by addIndexes.
fn test_non_cfs_leftovers() -> Result<()> {
    let mut dirs: Vec<Arc<dyn Directory>> = Vec::with_capacity(2);
    for _ in 0..2 {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let w = IndexWriter::new(Arc::clone(&dir), plain_iwc())?;
        let mut doc = Document::new();
        let mut custom_type = FieldType::new_from(TextField::type_stored());
        custom_type.set_store_term_vectors(true)?;
        doc.add(Box::new(Field::new("c", "v".into(), custom_type)?));
        w.add_document(&doc)?;
        w.close()?;
        dirs.push(dir);
    }

    let readers: Vec<Arc<dyn DirectoryReader>> = vec![
        directory_reader::open(Arc::clone(&dirs[0]))?,
        directory_reader::open(Arc::clone(&dirs[1]))?,
    ];

    let mut dir = mock_byte_buffers_dir();
    let mut conf = plain_iwc();
    conf.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(true)))?;
    let lmp = conf.merge_policy();
    // Force creation of CFS:
    lmp.set_no_cfs_ratio(1.0)?;
    lmp.set_max_cfs_segment_size_mb(f64::INFINITY)?;
    let w3 = IndexWriter::new(d(&dir), conf)?;
    TestUtil::add_indexes_slowly(&w3, &readers)?;
    w3.close()?;
    // we should now see segments_X,
    // _Y.cfs,_Y.cfe, _Z.si
    let sis = SegmentInfos::read_latest_commit(d(&dir))?;
    assert_equals_msg(
        Some("Only one compound segment should exist"),
        &1,
        &sis.size(),
    );
    Assert::assert_true(sis.info(0).info.get_use_compound_file());
    dir.close()
}

/// Port of `TestAddIndexes.testAddIndexMissingCodec`: simple test that
/// ensures we getting expected exceptions.
fn test_add_index_missing_codec() -> Result<()> {
    let mut to_add = LuceneTestCase::new_directory();
    // Disable checkIndex, else we get an exception because
    // of the unregistered codec:
    set_check_index_on_close(&to_add, false);
    {
        let mut conf = new_iwc();
        conf.set_codec(Arc::new(un_registered_codec()))?;
        let w = IndexWriter::new(d(&to_add), conf)?;
        let mut doc = Document::new();
        let mut custom_type = FieldType::new();
        custom_type.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
        doc.add(Box::new(LuceneTestCase::new_field(
            "foo",
            "bar",
            &custom_type,
        )));
        w.add_document(&doc)?;
        w.close()?;
    }

    {
        let mut dir = LuceneTestCase::new_directory();
        let mut conf = new_iwc();
        conf.set_codec(TestUtil::always_postings_format(Arc::new(
            DirectPostingsFormat::new(),
        )))?;
        let w = IndexWriter::new(d(&dir), conf)?;
        expect_throws(IAE, || w.add_indexes_dirs(&[d(&to_add)]));
        w.close()?;
        let open = directory_reader::open(d(&dir))?;
        assert_equals(&0, &open.num_docs());
        open.close()?;
        dir.close()?;
    }

    expect_throws(IAE, || directory_reader::open(d(&to_add)));
    to_add.close()
}

/// Port of `TestAddIndexes.testFieldNamesChanged`: LUCENE-3575.
fn test_field_names_changed() -> Result<()> {
    let mut d1 = LuceneTestCase::new_directory();
    let mut w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&d1))?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "f1",
        "doc1 field1",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::YES,
    )));
    w.add_document(&doc)?;
    let r1 = w.get_reader()?;
    w.close()?;

    let mut d2 = LuceneTestCase::new_directory();
    w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&d2))?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "f2",
        "doc2 field2",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "2",
        Store::YES,
    )));
    w.add_document(&doc)?;
    let r2 = w.get_reader()?;
    w.close()?;

    let mut d3 = LuceneTestCase::new_directory();
    w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&d3))?;
    TestUtil::add_indexes_slowly(w.w(), &[Arc::clone(&r1), Arc::clone(&r2)])?;
    r1.close()?;
    d1.close()?;
    r2.close()?;
    d2.close()?;

    let r3 = w.get_reader()?;
    w.close()?;
    assert_equals(&2, &r3.num_docs());
    let stored_fields = r3.stored_fields()?;
    for doc_id in 0..2 {
        let d = stored_fields.document(doc_id)?;
        if d.get("id").unwrap_or_else(|| npe("d.get(\"id\")")) == "1" {
            assert_equals(&Some("doc1 field1".to_string()), &d.get("f1"));
        } else {
            assert_equals(&Some("doc2 field2".to_string()), &d.get("f2"));
        }
    }
    r3.close()?;
    d3.close()
}

/// Port of `TestAddIndexes.testAddEmpty`.
fn test_add_empty() -> Result<()> {
    let mut d1 = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&d1))?;
    w.add_indexes_codec_readers(&[])?;
    w.close()?;
    let dr = directory_reader::open(d(&d1))?;
    for ctx in Arc::clone(&dr).leaves()? {
        Assert::assert_true_msg(
            "empty segments should be dropped by addIndexes",
            ctx.leaf_reader().max_doc() > 0,
        );
    }
    dr.close()?;
    d1.close()
}

/// Port of `TestAddIndexes.testFakeAllDeleted`: Currently it's impossible to
/// end up with a segment with all documents deleted, as such segments are
/// dropped. Still, to validate that addIndexes works with such segments, or
/// readers that end up in such state, we fake an all deleted segment.
fn test_fake_all_deleted() -> Result<()> {
    let mut src = LuceneTestCase::new_directory();
    let mut dest = LuceneTestCase::new_directory();
    let mut w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&src))?;
    w.add_document(&Document::new())?;
    let leaf = w.get_reader()?.leaves()?[0].leaf_reader();
    let all_deleted_reader: Arc<dyn LeafReader> = Arc::new(AllDeletedFilterReader::new(leaf)?);
    w.close()?;
    w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&dest))?;
    w.add_indexes_codec_readers(&[SlowCodecReaderWrapper::wrap(Arc::clone(
        &all_deleted_reader,
    ))?])?;
    w.close()?;
    let dr = directory_reader::open(d(&src))?;
    for ctx in Arc::clone(&dr).leaves()? {
        Assert::assert_true_msg(
            "empty segments should be dropped by addIndexes",
            ctx.leaf_reader().max_doc() > 0,
        );
    }
    dr.close()?;
    all_deleted_reader.as_index_reader().close()?;
    src.close()?;
    dest.close()
}

/// Port of `TestAddIndexes.testLocksBlock`: Make sure an open IndexWriter on
/// an incoming Directory causes a LockObtainFailedException.
fn test_locks_block() -> Result<()> {
    let mut src = LuceneTestCase::new_directory();
    let w1 = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&src))?;
    w1.add_document(&Document::new())?;
    w1.commit()?;

    let mut dest = LuceneTestCase::new_directory();

    let iwc = new_iwc();
    let w2 = RandomIndexWriter::new_with_config(&*random(), d(&dest), iwc)?;

    expect_throws("org.apache.lucene.store.LockObtainFailedException", || {
        w2.add_indexes_dirs(&[d(&src)])
    });

    w1.close()?;
    w2.close()?;
    IOUtils::close([
        IOUtils::closeable(|| src.close()),
        IOUtils::closeable(|| dest.close()),
    ])
}

/// `new Sort(new SortField("foo", type))`.
fn sort_foo(field_type: SortFieldType) -> Result<Sort> {
    Sort::new_fields(vec![SortField::new(Some("foo".to_string()), field_type)?])
}

/// The index both `testIllegalIndexSortChange*` methods add: sorted by
/// `<int: "foo">`, burned into the index.
fn int_sorted_index() -> Result<TestDirectory> {
    let dir1 = LuceneTestCase::new_directory();
    let mut iwc1 = new_iwc();
    iwc1.set_index_sort(sort_foo(SortFieldType::Int)?)?;
    let w1 = RandomIndexWriter::new_with_config(&*random(), d(&dir1), iwc1)?;
    w1.add_document(&Document::new())?;
    w1.commit()?;
    w1.add_document(&Document::new())?;
    w1.commit()?;
    // so the index sort is in fact burned into the index:
    w1.force_merge(1)?;
    w1.close()?;
    Ok(dir1)
}

/// The writer both `testIllegalIndexSortChange*` methods add to: sorted by
/// `<string: "foo">`.
fn string_sorted_writer(dir2: &TestDirectory) -> Result<RandomIndexWriter> {
    let mut iwc2 = new_iwc();
    iwc2.set_index_sort(sort_foo(SortFieldType::String)?)?;
    RandomIndexWriter::new_with_config(&*random(), d(dir2), iwc2)
}

/// Port of `TestAddIndexes.testIllegalIndexSortChange1`.
fn test_illegal_index_sort_change1() -> Result<()> {
    let mut dir1 = int_sorted_index()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = string_sorted_writer(&dir2)?;
    let message = expect_throws(IAE, || w2.add_indexes_dirs(&[d(&dir1)]))
        .message()
        .unwrap_or_default()
        .to_string();
    assert_equals(
        &"cannot change index sort from <int: \"foo\"> to <string: \"foo\">",
        &message.as_str(),
    );
    IOUtils::close([
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// Port of `TestAddIndexes.testIllegalIndexSortChange2`.
fn test_illegal_index_sort_change2() -> Result<()> {
    let mut dir1 = int_sorted_index()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = string_sorted_writer(&dir2)?;
    let r1 = directory_reader::open(d(&dir1))?;
    let message = expect_throws(IAE, || {
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r1) as Arc<dyn IndexReader>)?;
        w2.add_indexes_codec_readers(&[as_segment_reader(leaf)])
    })
    .message()
    .unwrap_or_default()
    .to_string();
    assert_equals(
        &"cannot change index sort from <int: \"foo\"> to <string: \"foo\">",
        &message.as_str(),
    );
    IOUtils::close([
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// The source index of the `testAddIndexesDVUpdate*` methods: one document
/// whose "soft_delete" doc value is updated after a flush.
fn dv_update_source(dir1: &TestDirectory) -> Result<()> {
    let iwc1 = new_iwc();
    let w1 = IndexWriter::new(d(dir1), iwc1)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "1".into(), Store::YES)?));
    doc.add(Box::new(StringField::new(
        "version",
        "1".into(),
        Store::YES,
    )?));
    doc.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
    w1.add_document(&doc)?;
    w1.flush()?;

    w1.update_doc_values(Term::from_text("id", "1"), &[ndv_field("soft_delete", 1)?])?;
    w1.commit()?;
    w1.close()
}

/// The two writers the `testAddIndexesDVUpdate*` methods open on `dir2`.
fn reopen_twice(dir2: &TestDirectory) -> Result<()> {
    if verbose() {
        println!("\nTEST: now open w3");
    }
    let mut iwc3 = new_iwc();
    if verbose() {
        iwc3.set_info_stream_print_stream(Box::new(std::io::stdout()))?;
    }
    let w3 = IndexWriter::new(d(dir2), iwc3)?;
    w3.close()?;

    let iwc3 = new_iwc();
    let w3 = IndexWriter::new(d(dir2), iwc3)?;
    w3.close()
}

/// Port of `TestAddIndexes.testAddIndexesDVUpdateSameSegmentName`.
fn test_add_indexes_dv_update_same_segment_name() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    dv_update_source(&dir1)?;

    let iwc2 = new_iwc();
    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(d(&dir2), iwc2)?;
    w2.add_indexes_dirs(&[d(&dir1)])?;
    w2.commit()?;
    w2.close()?;

    reopen_twice(&dir2)?;
    dir1.close()?;
    dir2.close()
}

/// Port of `TestAddIndexes.testAddIndexesDVUpdateNewSegmentName`.
fn test_add_indexes_dv_update_new_segment_name() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    dv_update_source(&dir1)?;

    let iwc2 = new_iwc();
    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(d(&dir2), iwc2)?;
    w2.add_document(&Document::new())?;
    w2.commit()?;

    w2.add_indexes_dirs(&[d(&dir1)])?;
    w2.commit()?;
    w2.close()?;

    reopen_twice(&dir2)?;
    dir1.close()?;
    dir2.close()
}

/// Port of `TestAddIndexes.testAddIndicesWithSoftDeletes`.
fn test_add_indices_with_soft_deletes() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let mut iwc1 = new_iwc();
    iwc1.set_soft_deletes_field(Some("soft_delete".to_string()));
    let writer = IndexWriter::new(d(&dir1), iwc1)?;
    for _ in 0..30 {
        let mut doc = Document::new();
        let doc_id = random().next_int_bound(5);
        doc.add(Box::new(StringField::new(
            "id",
            doc_id.to_string(),
            Store::YES,
        )?));
        writer.soft_update_document(
            Term::from_text("id", &doc_id.to_string()),
            &doc,
            &[ndv_field("soft_delete", 1)?],
        )?;
        if random().next_boolean() {
            writer.flush()?;
        }
    }
    writer.commit()?;
    writer.close()?;
    let reader = directory_reader::open(d(&dir1))?;
    // wrappedReader filters out soft deleted docs
    let wrapped_reader =
        SoftDeletesDirectoryReaderWrapper::new("soft_delete").wrap(Arc::clone(&reader))?;
    let mut dir2 = LuceneTestCase::new_directory();
    let num_docs = reader.num_docs();
    let max_doc = reader.max_doc();
    assert_equals(&num_docs, &max_doc);
    let mut iwc1 = new_iwc();
    iwc1.set_soft_deletes_field(Some("soft_delete".to_string()));
    let writer = IndexWriter::new(d(&dir2), iwc1)?;
    let readers = codec_leaves(&reader)?;
    writer.add_indexes_codec_readers(&readers)?;
    assert_equals(
        &wrapped_reader.num_docs(),
        &writer.get_doc_stats()?.num_docs,
    );
    assert_equals(&max_doc, &writer.get_doc_stats()?.max_doc);
    writer.commit()?;
    let soft_delete_count: i32 = writer
        .clone_segment_infos()?
        .as_list()
        .iter()
        .map(SegmentCommitInfo::get_soft_del_count)
        .sum();
    assert_equals(&(max_doc - wrapped_reader.num_docs()), &soft_delete_count);
    writer.close()?;

    let mut dir3 = LuceneTestCase::new_directory();
    let mut iwc1 = new_iwc();
    iwc1.set_soft_deletes_field(Some("soft_delete".to_string()));
    let writer = IndexWriter::new(d(&dir3), iwc1)?;
    // Resize as some fully deleted sub-readers might be dropped in the wrappedReader
    let readers = codec_leaves(&wrapped_reader)?;
    writer.add_indexes_codec_readers(&readers)?;
    assert_equals(
        &wrapped_reader.num_docs(),
        &writer.get_doc_stats()?.num_docs,
    );
    // soft deletes got filtered out when wrappedReader(s) were added.
    assert_equals(&wrapped_reader.num_docs(), &writer.get_doc_stats()?.max_doc);
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| writer.close()),
        IOUtils::closeable(|| dir3.close()),
        IOUtils::closeable(|| dir2.close()),
        IOUtils::closeable(|| dir1.close()),
    ])
}

/// The blocks `testAddIndicesWithBlocks` writes: `numBlocks` blocks of
/// `random().nextInt(2, 10)` documents when `has_blocks`, else of one.
fn add_value_blocks(writer: &RandomIndexWriter, num_blocks: i32, has_blocks: bool) -> Result<()> {
    for _ in 0..num_blocks {
        let num_docs = if has_blocks {
            random().next_int_range(2, 10)
        } else {
            1
        };
        let mut docs = Vec::new();
        for _ in 0..num_docs {
            let mut doc = Document::new();
            let value = random().next_int_bound(5);
            doc.add(Box::new(StringField::new(
                "value",
                value.to_string(),
                Store::YES,
            )?));
            docs.push(doc);
        }
        writer.add_documents(&docs)?;
    }
    Ok(())
}

/// Port of `TestAddIndexes.testAddIndicesWithBlocks`.
fn test_add_indices_with_blocks() -> Result<()> {
    let add_has_blocks_perm = [true, true, false, false];
    let base_has_blocks_perm = [true, false, true, false];
    for perm in 0..add_has_blocks_perm.len() {
        let add_has_blocks = add_has_blocks_perm[perm];
        let base_has_blocks = base_has_blocks_perm[perm];
        let mut dir = LuceneTestCase::new_directory();
        {
            let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&dir))?;
            let num_blocks = random().next_int_range(1, 10);
            add_value_blocks(&writer, num_blocks, base_has_blocks)?;
            writer.commit()?;
            writer.close()?;
        }
        {
            let mut add_dir = LuceneTestCase::new_directory();
            let num_blocks = random().next_int_range(1, 10);
            {
                let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), d(&add_dir))?;
                add_value_blocks(&writer, num_blocks, add_has_blocks)?;
                writer.commit()?;
                writer.close()?;
            }
            {
                let writer = IndexWriter::new(d(&dir), LuceneTestCase::new_index_writer_config())?;
                if random().next_boolean() {
                    writer.add_indexes_dirs(&[d(&add_dir)])?;
                } else {
                    let reader = directory_reader::open(d(&add_dir))?;
                    let readers = codec_leaves(&reader)?;
                    writer.add_indexes_codec_readers(&readers)?;
                    reader.close()?;
                }
                writer.force_merge_with(1, true)?;
                writer.close()?;
            }

            {
                let reader = directory_reader::open(d(&dir))?;
                let leaves = Arc::clone(&reader).leaves()?;
                let leaf = leaves[0].leaf_reader();
                let codec_reader = leaf.as_segment_reader().unwrap_or_else(|| {
                    Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
                });
                assert_equals(&1, &leaves.len());
                if add_has_blocks || base_has_blocks {
                    Assert::assert_true_msg(
                        &format!("addHasBlocks: {add_has_blocks} baseHasBlocks: {base_has_blocks}"),
                        codec_reader.get_segment_info().info.get_has_blocks(),
                    );
                } else {
                    Assert::assert_false(codec_reader.get_segment_info().info.get_has_blocks());
                }
                reader.close()?;
            }
            add_dir.close()?;
        }
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestAddIndexes.testSetDiagnostics`.
fn test_set_diagnostics() -> Result<()> {
    let my_merge_policy = ReadersMergePolicy::new(
        Arc::new(FilterMergePolicy::new(Arc::new(
            LuceneTestCase::new_log_merge_policy_merge_factor(4)?,
        ))),
        Arc::new(
            |sup: &FilterMergePolicy, readers: &[Arc<dyn CodecReader>]| {
                let Some(spec) = sup.find_merges_for_readers(readers)? else {
                    return Ok(None);
                };
                let mut new_spec = MergeSpecification::new();
                for merge in spec.merges {
                    let mut merge = merge;
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
                Ok(Some(new_spec))
            },
        ),
    );
    let mut source_dir = LuceneTestCase::new_directory();
    {
        let w = IndexWriter::new(d(&source_dir), LuceneTestCase::new_index_writer_config())?;
        let doc = Document::new();
        w.add_document(&doc)?;
        w.close()?;
    }
    let reader = directory_reader::open(d(&source_dir))?;
    let codec_reader =
        SlowCodecReaderWrapper::wrap(Arc::clone(&reader).leaves()?[0].leaf_reader())?;

    let mut target_dir = LuceneTestCase::new_directory();
    {
        let mut c = LuceneTestCase::new_index_writer_config();
        c.set_merge_policy(Some(Arc::new(my_merge_policy)))?;
        let w = IndexWriter::new(d(&target_dir), c)?;
        w.add_indexes_codec_readers(&[codec_reader])?;
        w.close()?;
    }
    let si = SegmentInfos::read_latest_commit(d(&target_dir))?;
    assert_not_equals(&0, &si.size());
    for sci in si.as_list() {
        assert_equals(
            &Some(SOURCE_ADDINDEXES_READERS.to_string()),
            &sci.info
                .get_diagnostics()
                .get(SOURCE)
                .map(|s| s.to_string()),
        );
        assert_equals(
            &Some("my_merge_policy".to_string()),
            &sci.info
                .get_diagnostics()
                .get("merge_policy")
                .map(|s| s.to_string()),
        );
    }
    reader.close()?;
    target_dir.close()?;
    source_dir.close()
}

/// The parent index of the `testIllegalParentDocChange` method.
fn parent_field_index(dir1: &TestDirectory) -> Result<()> {
    let mut iwc1 = new_iwc();
    iwc1.set_parent_field(Some("foobar".to_string()));
    let w1 = RandomIndexWriter::new_with_config(&*random(), d(dir1), iwc1)?;
    w1.add_documents(&[Document::new(), Document::new(), Document::new()])?;
    w1.commit()?;
    w1.add_documents(&[Document::new(), Document::new(), Document::new()])?;
    w1.commit()?;
    // so the index sort is in fact burned into the index:
    w1.force_merge(1)?;
    w1.close()
}

/// Port of `TestAddIndexes.testIllegalParentDocChange`.
fn test_illegal_parent_doc_change() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    parent_field_index(&dir1)?;

    let mut dir2 = LuceneTestCase::new_directory();
    let mut iwc2 = new_iwc();
    iwc2.set_parent_field(Some("foo".to_string()));
    let w2 = RandomIndexWriter::new_with_config(&*random(), d(&dir2), iwc2)?;

    let r1 = directory_reader::open(d(&dir1))?;
    let message = expect_throws(IAE, || {
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r1) as Arc<dyn IndexReader>)?;
        w2.add_indexes_codec_readers(&[as_segment_reader(leaf)])
    })
    .message()
    .unwrap_or_default()
    .to_string();
    assert_equals(
        &"can't add field [foobar] as parent document field; this IndexWriter is configured with [foo] as parent document field",
        &message.as_str(),
    );

    let message = expect_throws(IAE, || w2.add_indexes_dirs(&[d(&dir1)]))
        .message()
        .unwrap_or_default()
        .to_string();
    assert_equals(
        &"can't add field [foobar] as parent document field; this IndexWriter is configured with [foo] as parent document field",
        &message.as_str(),
    );

    let mut dir3 = LuceneTestCase::new_directory();
    let mut iwc3 = new_iwc();
    iwc3.set_parent_field(Some("foobar".to_string()));
    let w3 = RandomIndexWriter::new_with_config(&*random(), d(&dir3), iwc3)?;

    w3.add_indexes_codec_readers(&[as_segment_reader(LuceneTestCase::get_only_leaf_reader(
        Arc::clone(&r1) as Arc<dyn IndexReader>,
    )?)])?;
    w3.add_indexes_dirs(&[d(&dir1)])?;

    IOUtils::close([
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir2.close()),
        IOUtils::closeable(|| w3.close()),
        IOUtils::closeable(|| dir3.close()),
    ])
}

/// Port of `TestAddIndexes.testIllegalNonParentField`.
fn test_illegal_non_parent_field() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let iwc1 = new_iwc();
    let w1 = RandomIndexWriter::new_with_config(&*random(), d(&dir1), iwc1)?;
    let mut parent = Document::new();
    parent.add(Box::new(StringField::new("foo", "XXX".into(), Store::NO)?));
    w1.add_document(&parent)?;
    w1.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let mut iwc2 = new_iwc();
    iwc2.set_parent_field(Some("foo".to_string()));
    let w2 = RandomIndexWriter::new_with_config(&*random(), d(&dir2), iwc2)?;

    let r1 = directory_reader::open(d(&dir1))?;
    let message = expect_throws(IAE, || {
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r1) as Arc<dyn IndexReader>)?;
        w2.add_indexes_codec_readers(&[as_segment_reader(leaf)])
    })
    .message()
    .unwrap_or_default()
    .to_string();
    assert_equals(
        &"can't add [foo] as non parent document field; this IndexWriter is configured with [foo] as parent document field",
        &message.as_str(),
    );

    let message = expect_throws(IAE, || w2.add_indexes_dirs(&[d(&dir1)]))
        .message()
        .unwrap_or_default()
        .to_string();
    assert_equals(
        &"can't add [foo] as non parent document field; this IndexWriter is configured with [foo] as parent document field",
        &message.as_str(),
    );

    IOUtils::close([
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}
