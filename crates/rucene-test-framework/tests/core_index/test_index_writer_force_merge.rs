//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterForceMerge.java`.
//!
//! * The anonymous `ConcurrentMergeScheduler` of `testMergePerField` that
//!   overrides `getIntraMergeExecutor` is a `ConcurrentMergeScheduler` with
//!   that override installed; the anonymous `FilterCodec` is a
//!   `FilterCodec` whose postings and doc-values formats are the per-field
//!   formats the overrides return.
//! * `new IndexWriterConfig()` is `IndexWriterConfig::new()`.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::codecs::{
    Codec, DocValuesConsumer, DocValuesFormat, DocValuesProducer, FieldsConsumer, FieldsProducer,
    FilterCodec, MergeState, NormsProducer, PerFieldDocValuesFormat, PerFieldPostingsFormat,
    PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::document::{BinaryDocValuesField, Document, LongPoint, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::{FieldInfo, Fields, IndexReader, SegmentInfos};
use rucene::store::Directory;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{tokenizer_components, MockAnalyzer};
use rucene_test_framework::util::lucene_test_case::{AwaitsFix, TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, verbose};
use crate::support::{add_doc_with_index, npe, CyclicBarrier};

/// `TestIndexWriterForceMerge`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterForceMerge",
        vec![
            test("testPartialMerge", test_partial_merge),
            test("testMaxNumSegments2", test_max_num_segments2),
            test(
                "testForceMergeTempSpaceUsage",
                test_force_merge_temp_space_usage,
            ),
            test("testBackgroundForceMerge", test_background_force_merge),
            test("testMergePerField", test_merge_per_field).annotate(AwaitsFix {
                bug_url: "https://github.com/apache/lucene/issues/13478".to_string(),
            }),
        ],
    )
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))` through
/// `newIndexWriterConfig`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `SegmentInfos.readLatestCommit(dir).size()`.
fn segment_count(dir: &TestDirectory) -> Result<usize> {
    Ok(SegmentInfos::read_latest_commit(dir.as_directory())?.size())
}

/// Port of `TestIndexWriterForceMerge.testPartialMerge`.
fn test_partial_merge() -> Result<()> {
    let dir = LuceneTestCase::new_directory();

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "content",
        "aaa",
        Store::NO,
    )));
    let incr_min = if *TEST_NIGHTLY { 15 } else { 40 };
    let mut num_docs = 10;
    while num_docs < 500 {
        let mut ldmp = LogMergePolicy::by_doc_count();
        ldmp.set_min_merge_docs(1);
        ldmp.set_merge_factor(5)?;
        let conf = mock_iwc();
        conf.set_open_mode(OpenMode::CREATE)?;
        conf.set_max_buffered_docs(2)?;
        conf.set_merge_policy(Some(Arc::new(ldmp)))?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        for _ in 0..num_docs {
            writer.add_document(&doc)?;
        }
        writer.close()?;

        let seg_count = segment_count(&dir)?;

        let ldmp = LogMergePolicy::by_doc_count();
        ldmp.set_merge_factor(5)?;
        let conf = mock_iwc();
        conf.set_merge_policy(Some(Arc::new(ldmp)))?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        writer.force_merge(3)?;
        writer.close()?;

        let opt_seg_count = segment_count(&dir)?;

        if seg_count < 3 {
            assert_equals(&seg_count, &opt_seg_count);
        } else {
            assert_equals(&3, &opt_seg_count);
        }
        num_docs += TestUtil::next_int(&*random(), incr_min, 5 * incr_min);
    }
    dir.close()
}

/// Port of `TestIndexWriterForceMerge.testMaxNumSegments2`.
fn test_max_num_segments2() -> Result<()> {
    let dir = LuceneTestCase::new_directory();

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "content",
        "aaa",
        Store::NO,
    )));

    let mut ldmp = LogMergePolicy::by_doc_count();
    ldmp.set_min_merge_docs(1);
    ldmp.set_merge_factor(4)?;
    let conf = mock_iwc();
    conf.set_max_buffered_docs(2)?;
    conf.set_merge_policy(Some(Arc::new(ldmp)))?;
    conf.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    for _ in 0..10 {
        for _ in 0..19 {
            writer.add_document(&doc)?;
        }

        writer.commit()?;
        writer.wait_for_merges()?;
        writer.commit()?;

        let seg_count = segment_count(&dir)?;

        writer.force_merge(7)?;
        writer.commit()?;
        writer.wait_for_merges()?;

        let opt_seg_count = segment_count(&dir)?;

        if seg_count < 7 {
            assert_equals(&seg_count, &opt_seg_count);
        } else {
            assert_equals_msg(Some(&format!("seg: {seg_count}")), &7, &opt_seg_count);
        }
    }
    writer.close()?;
    dir.close()
}

/// The anonymous `Analyzer` of `testForceMergeTempSpaceUsage`.
#[derive(Debug)]
struct WhitespaceAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for WhitespaceAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterForceMerge$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            MockTokenizer::new(Arc::clone(&WHITESPACE), true)?,
        ))))
    }
}

/// Make sure forceMerge doesn't use any more than 1X starting index size as its temporary free
/// space required.
/// Port of `TestIndexWriterForceMerge.testForceMergeTempSpaceUsage`.
fn test_force_merge_temp_space_usage() -> Result<()> {
    let holder = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let dir = holder.as_mock().expect("newMockDirectory");
    let d = dir.as_directory();
    // don't use MockAnalyzer, variable length payloads can cause merge to make things bigger,
    // since things are optimized for fixed length case. this is a problem for MemoryPF's encoding.
    // (it might have other problems too)
    let analyzer: Arc<dyn Analyzer> = Arc::new(WhitespaceAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    conf.set_max_buffered_docs(10)?;
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(Arc::clone(&d), conf)?;

    if verbose() {
        println!("TEST: config1={}", writer.get_config());
    }

    for j in 0..500 {
        add_doc_with_index(&writer, j)?;
    }
    // force one extra segment w/ different doc store so
    // we see the doc stores get merged
    writer.commit()?;
    add_doc_with_index(&writer, 500)?;
    writer.close()?;

    let mut start_disk_usage = 0;
    for f in d.list_all()? {
        start_disk_usage += d.file_length(&f)?;
        if verbose() {
            println!("{f}: {}", d.file_length(&f)?);
        }
    }
    if verbose() {
        println!("TEST: start disk usage = {start_disk_usage}");
    }
    let start_listing = list_files(&d)?;

    dir.reset_max_used_size_in_bytes()?;
    dir.set_track_disk_usage(true);

    let conf = mock_iwc();
    conf.set_open_mode(OpenMode::APPEND)?;
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(Arc::clone(&d), conf)?;

    if verbose() {
        println!("TEST: config2={}", writer.get_config());
    }

    writer.force_merge(1)?;
    writer.close()?;

    let mut final_disk_usage = 0;
    for f in d.list_all()? {
        final_disk_usage += d.file_length(&f)?;
        if verbose() {
            println!("{f}: {}", d.file_length(&f)?);
        }
    }

    if verbose() {
        println!("TEST: final disk usage = {final_disk_usage}");
    }

    // The result of the merged index is often smaller, but sometimes it could
    // be bigger (compression slightly changes, Codec changes etc.). Therefore
    // we compare the temp space used to the max of the initial and final index
    // size
    let max_start_final_disk_usage = start_disk_usage.max(final_disk_usage);
    let max_disk_usage = dir.get_max_used_size_in_bytes();
    let message = format!(
        "forceMerge used too much temporary space: starting usage was {start_disk_usage} bytes; final usage was {final_disk_usage} bytes; max temp usage was {max_disk_usage} but should have been at most {} (= 4X starting usage), BEFORE={start_listing}AFTER={}",
        4 * max_start_final_disk_usage,
        list_files(&d)?
    );
    Assert::assert_true_msg(&message, max_disk_usage <= 4 * max_start_final_disk_usage);
    drop(d);
    holder.close()
}

/// `String.format(Locale.ROOT, "%-20s%d%n", file, length)`.
fn padded(file: &str, length: i64) -> String {
    format!("{file:<20}{length}\n")
}

/// print out listing of files and sizes, but recurse into CFS to debug nested files there.
/// Port of the private `TestIndexWriterForceMerge.listFiles(Directory)`.
fn list_files(dir: &Arc<dyn Directory>) -> Result<String> {
    let infos = SegmentInfos::read_latest_commit(Arc::clone(dir))?;
    let mut sb = String::new();
    sb.push('\n');
    for info in infos.iter() {
        for file in info.files_ordered()?.iter() {
            sb.push_str(&padded(file, dir.file_length(file)?));
        }
        if info.info.get_use_compound_file() {
            let codec = info.info.codec().unwrap_or_else(|| npe("getCodec()"));
            let cfs = codec
                .compound_format()
                .get_compound_reader(&**dir, &info.info)?;
            let outcome = (|| -> Result<()> {
                for file in cfs.list_all()? {
                    sb.push_str(&format!(
                        " |- (inside compound file) {}",
                        padded(&file, cfs.file_length(&file)?)
                    ));
                }
                Ok(())
            })();
            let closed = cfs.close();
            outcome?;
            closed?;
        }
    }
    sb.push('\n');
    Ok(sb)
}

/// Test calling forceMerge(1, false) whereby forceMerge is kicked
/// off but we don't wait for it to finish (but
/// writer.close()) does wait
/// Port of `TestIndexWriterForceMerge.testBackgroundForceMerge`.
fn test_background_force_merge() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    for pass in 0..2 {
        let conf = mock_iwc();
        conf.set_open_mode(OpenMode::CREATE)?;
        conf.set_max_buffered_docs(2)?;
        conf.set_merge_policy(Some(Arc::new(
            LuceneTestCase::new_log_merge_policy_merge_factor(51)?,
        )))?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            "aaa",
            Store::NO,
        )));
        for _ in 0..100 {
            writer.add_document(&doc)?;
        }
        writer.force_merge_with(1, false)?;

        if 0 == pass {
            writer.close()?;
            let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
            assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
            reader.close()?;
        } else {
            // Get another segment to flush so we can verify it is
            // NOT included in the merging
            writer.add_document(&doc)?;
            writer.add_document(&doc)?;
            writer.close()?;

            let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
            Assert::assert_true(Arc::clone(&reader).leaves()?.len() > 1);
            reader.close()?;

            assert_equals(&2, &segment_count(&dir)?);
        }
    }

    dir.close()
}

/// `barrier.await(1, TimeUnit.SECONDS)`, failing with
/// `AssertionError("broken barrier", e)`.
fn await_barrier(barrier: &CyclicBarrier) {
    if let Err(e) = barrier.await_millis(1000) {
        Throwable::with_cause(
            ASSERTION_ERROR,
            Some("broken barrier".to_string()),
            Throwable::with_message(e.java_class(), None),
        )
        .throw()
    }
}

/// Port of `TestIndexWriterForceMerge.testMergePerField`.
fn test_merge_per_field() -> Result<()> {
    let config = IndexWriterConfig::new();
    let merge_scheduler = ConcurrentMergeScheduler::new();
    merge_scheduler.set_intra_merge_executor_override(Some(Arc::new(|cms, _merge| {
        // always enable parallel merges
        Ok(cms.intra_merge_executor())
    })));
    merge_scheduler.set_max_merges_and_threads(4, 4)?;
    config.set_merge_scheduler(Some(Arc::new(merge_scheduler)))?;
    let codec = TestUtil::get_default_codec();
    let barrier = Arc::new(CyclicBarrier::new(2));
    let postings_barrier = Arc::clone(&barrier);
    let doc_values_barrier = Arc::clone(&barrier);
    let filter = FilterCodec::new(codec.name().to_string(), Arc::clone(&codec))
        .with_postings_format(PerFieldPostingsFormat::new(move |_field| {
            Arc::new(BlockingOnMergePostingsFormat::new(
                TestUtil::get_default_postings_format(),
                Arc::clone(&postings_barrier),
            )) as Arc<dyn PostingsFormat>
        }))
        .with_doc_values_format(PerFieldDocValuesFormat::new(move |_field| {
            Arc::new(BlockingOnMergeDocValuesFormat::new(
                TestUtil::get_default_doc_values_format(),
                Arc::clone(&doc_values_barrier),
            )) as Arc<dyn DocValuesFormat>
        }));
    config.set_codec(Arc::new(filter) as Arc<dyn Codec>)?;
    let directory = LuceneTestCase::new_directory();
    let body = (|| -> Result<()> {
        let writer = IndexWriter::new(directory.as_directory(), config)?;
        let inner = (|| -> Result<()> {
            let num_docs = 50 + random().next_int_bound(100);
            let num_fields = 5 + random().next_int_bound(5);
            for _ in 0..num_docs {
                let mut doc = Document::new();
                for f in 0..num_fields * 2 {
                    let field = format!("f{f}");
                    let value = format!("v-{}", random().next_int_bound(100));
                    if f % 2 == 0 {
                        doc.add(Box::new(StringField::new(&field, value, Store::NO)?));
                    } else {
                        doc.add(Box::new(BinaryDocValuesField::new(
                            &field,
                            BytesRef::from_char_sequence(&value),
                        )));
                    }
                    doc.add(Box::new(LongPoint::new(
                        &format!("p{f}"),
                        &[random().next_int_bound(10000) as i64],
                    )?));
                }
                writer.add_document(&doc)?;
                if random().next_int_bound(100) < 10 {
                    writer.flush()?;
                }
            }
            writer.force_merge(1)?;
            let reader = directory_reader::open_with_writer(&*writer)?;
            let checked = (|| -> Result<()> {
                assert_equals(&num_docs, &reader.num_docs());
                Ok(())
            })();
            let closed = reader.close();
            checked?;
            closed
        })();
        let closed = writer.close();
        inner?;
        closed
    })();
    let dir_closed = directory.close();
    body?;
    dir_closed
}

/// `static class BlockingOnMergePostingsFormat extends PostingsFormat`.
struct BlockingOnMergePostingsFormat {
    postings_format: Arc<dyn PostingsFormat>,
    barrier: Arc<CyclicBarrier>,
}

impl BlockingOnMergePostingsFormat {
    /// `BlockingOnMergePostingsFormat(postingsFormat, barrier)`:
    /// `super(postingsFormat.getName())`.
    fn new(postings_format: Arc<dyn PostingsFormat>, barrier: Arc<CyclicBarrier>) -> Self {
        BlockingOnMergePostingsFormat {
            postings_format,
            barrier,
        }
    }
}

impl fmt::Debug for BlockingOnMergePostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PostingsFormat(name={})", self.name())
    }
}

impl PostingsFormat for BlockingOnMergePostingsFormat {
    fn name(&self) -> &str {
        self.postings_format.name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let r#in = self.postings_format.fields_consumer(state)?;
        Ok(Box::new(BlockingFieldsConsumer {
            r#in,
            barrier: Arc::clone(&self.barrier),
        }))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        self.postings_format.fields_producer(state)
    }
}

/// The anonymous `FieldsConsumer` of `BlockingOnMergePostingsFormat`.
struct BlockingFieldsConsumer<'a> {
    r#in: Box<dyn FieldsConsumer + 'a>,
    barrier: Arc<CyclicBarrier>,
}

impl FieldsConsumer for BlockingFieldsConsumer<'_> {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        self.r#in.write(fields, norms)
    }

    fn merge(&mut self, merge_state: &MergeState, norms: Option<&dyn NormsProducer>) -> Result<()> {
        await_barrier(&self.barrier);
        self.r#in.merge(merge_state, norms)
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }
}

/// `static class BlockingOnMergeDocValuesFormat extends DocValuesFormat`.
struct BlockingOnMergeDocValuesFormat {
    doc_values_format: Arc<dyn DocValuesFormat>,
    barrier: Arc<CyclicBarrier>,
}

impl BlockingOnMergeDocValuesFormat {
    /// `BlockingOnMergeDocValuesFormat(docValuesFormat, barrier)`:
    /// `super(docValuesFormat.getName())`.
    fn new(doc_values_format: Arc<dyn DocValuesFormat>, barrier: Arc<CyclicBarrier>) -> Self {
        BlockingOnMergeDocValuesFormat {
            doc_values_format,
            barrier,
        }
    }
}

impl fmt::Debug for BlockingOnMergeDocValuesFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DocValuesFormat(name={})", self.name())
    }
}

impl DocValuesFormat for BlockingOnMergeDocValuesFormat {
    fn name(&self) -> &str {
        self.doc_values_format.name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        let r#in = self.doc_values_format.fields_consumer(state)?;
        Ok(Box::new(BlockingDocValuesConsumer {
            r#in,
            barrier: Arc::clone(&self.barrier),
        }))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        self.doc_values_format.fields_producer(state)
    }
}

/// The anonymous `DocValuesConsumer` of `BlockingOnMergeDocValuesFormat`.
struct BlockingDocValuesConsumer<'a> {
    r#in: Box<dyn DocValuesConsumer + 'a>,
    barrier: Arc<CyclicBarrier>,
}

impl fmt::Debug for BlockingDocValuesConsumer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlockingDocValuesConsumer({:?})", self.r#in)
    }
}

impl DocValuesConsumer for BlockingDocValuesConsumer<'_> {
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.r#in.add_numeric_field(field, values_producer)
    }

    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.r#in.add_binary_field(field, values_producer)
    }

    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.r#in.add_sorted_field(field, values_producer)
    }

    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.r#in.add_sorted_numeric_field(field, values_producer)
    }

    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values_producer: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.r#in.add_sorted_set_field(field, values_producer)
    }

    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        await_barrier(&self.barrier);
        self.r#in.merge(merge_state)
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }
}
