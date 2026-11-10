//! Plumbing the `core_index` ports share.

#![allow(dead_code)]

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::Throttling;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::random;

/// `if (dir instanceof MockDirectoryWrapper) ((MockDirectoryWrapper)
/// dir).setThrottling(MockDirectoryWrapper.Throttling.NEVER)`.
pub fn never_throttle(dir: &TestDirectory) {
    if let Some(mock) = dir.as_mock() {
        mock.set_throttling(Throttling::Never);
    }
}

/// The `IndexWriterConfig` of the `Test2B*` classes:
/// `new IndexWriterConfig(new MockAnalyzer(random()))
/// .setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH)
/// .setRAMBufferSizeMB(256.0) .setMergeScheduler(new ConcurrentMergeScheduler())
/// .setMergePolicy(newLogMergePolicy(false, 10))
/// .setOpenMode(IndexWriterConfig.OpenMode.CREATE)
/// .setCodec(TestUtil.getDefaultCodec())`.
pub fn two_b_config() -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    iwc.set_ram_buffer_size_mb(256.0)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
    )))?;
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_codec(TestUtil::get_default_codec())?;
    Ok(iwc)
}

/// `DirectoryReader.open(dir)`.
pub fn open_reader(dir: &TestDirectory) -> Result<Arc<dyn DirectoryReader>> {
    directory_reader::open(dir.as_directory())
}

/// `System.out.println(s); System.out.flush();`
pub fn println_flush(s: &str) {
    use std::io::Write;
    println!("{s}");
    let _ = std::io::stdout().flush();
}

/// `MergePolicy mp = w.getConfig().getMergePolicy(); if (mp instanceof
/// LogByteSizeMergePolicy) { // 1 petabyte:
/// ((LogByteSizeMergePolicy) mp).setMaxMergeMB(1024 * 1024 * 1024); }`.
pub fn petabyte_max_merge(w: &rucene::index::index_writer::IndexWriter) {
    let mp = w.get_config().merge_policy();
    if let Some(lmp) = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<rucene::index::merge_policy::LogMergePolicy>())
    {
        if lmp.is_log_byte_size_merge_policy() {
            // 1 petabyte:
            lmp.set_max_merge_mb((1024 * 1024 * 1024) as f64);
        }
    }
}

/// `((ConcurrentMergeScheduler) iwc.getMergeScheduler())`.
pub fn concurrent_merge_scheduler(
    iwc: &IndexWriterConfig,
) -> Arc<dyn rucene::index::merge_scheduler::MergeScheduler> {
    let ms = iwc.merge_scheduler();
    if ms
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .is_none()
    {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            "not a ConcurrentMergeScheduler",
        )
        .throw()
    }
    ms
}

/// The `ConcurrentMergeScheduler` [`concurrent_merge_scheduler`] returned.
pub fn as_cms(
    ms: &Arc<dyn rucene::index::merge_scheduler::MergeScheduler>,
) -> &ConcurrentMergeScheduler {
    ms.as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .expect("checked by concurrent_merge_scheduler")
}

/// `null` dereferenced: Java's `NullPointerException`.
pub fn npe<T>(what: &str) -> T {
    randomizedtesting::jdk::exception::Throwable::new("java.lang.NullPointerException", what)
        .throw()
}

/// `dir.setCheckIndexOnClose(value)` (`BaseDirectoryWrapper`).
pub fn set_check_index_on_close(dir: &TestDirectory, value: bool) {
    match dir {
        TestDirectory::Mock(mock) => mock.set_check_index_on_close(value),
        TestDirectory::Raw(raw) => raw.base().set_check_index_on_close(value),
    }
}

/// The indexing loop `TestAllFilesCheckIndexHeader`,
/// `TestAllFilesHaveChecksumFooter` and `TestAllFilesHaveCodecHeader` share:
/// 100 `LineFileDocs` documents, with random commits, deletes and
/// numeric doc-values updates.
pub fn add_line_file_docs(riw: &rucene_test_framework::index::RandomIndexWriter) -> Result<()> {
    use rucene::index::terms::Term;
    // Use LineFileDocs so we (hopefully) get most Lucene features
    // tested, e.g. IntPoint was recently added to it:
    let docs = rucene_test_framework::util::LineFileDocs::with_default_file(&*random())?;
    for i in 0..100 {
        riw.add_document(&*docs.next_doc()?.borrow())?;
        if random().next_int_bound(7) == 0 {
            riw.commit()?;
        }
        if random().next_int_bound(20) == 0 {
            riw.delete_documents(Term::from_text("docid", &i.to_string()))?;
        }
        if random().next_int_bound(15) == 0 {
            riw.update_numeric_doc_value(
                Term::from_text("docid", &i.to_string()),
                "page_views",
                i as i64,
            )?;
        }
    }
    Ok(())
}

/// The indexing loop `TestAllFilesDetectMismatchedChecksum` and
/// `TestAllFilesDetectTruncation` share: 100 documents of a text field with
/// term vectors, a string field with sorted doc values, a long point with
/// numeric doc values and a 16-dimension float vector.
pub fn add_mutated_docs(riw: &rucene_test_framework::index::RandomIndexWriter) -> Result<()> {
    use rucene::document::{
        Document, Field, FieldType, KnnFloatVectorField, LongPoint, NumericDocValuesField,
        SortedDocValuesField, Store, StringField, TextField,
    };
    use rucene::util::BytesRef;

    let mut doc = Document::new();
    let text_with_term_vectors_type = Arc::new(FieldType::new_from(TextField::type_stored()));
    text_with_term_vectors_type.set_store_term_vectors(true)?;
    let text = Field::new(
        "text",
        String::new(),
        Arc::clone(&text_with_term_vectors_type),
    )?;
    doc.add(Box::new(text));
    let term_string = StringField::new("string", String::new(), Store::YES)?;
    doc.add(Box::new(term_string));
    let dv_string = SortedDocValuesField::new("string", BytesRef::new(Vec::new()));
    doc.add(Box::new(dv_string));
    let point_number = LongPoint::new("long", &[0])?;
    doc.add(Box::new(point_number));
    let dv_number = NumericDocValuesField::new("long", 0);
    doc.add(Box::new(dv_number));
    let vector = KnnFloatVectorField::with_euclidean("vector", &[0f32; 16])?;
    doc.add(Box::new(vector));

    for i in 0..100 {
        field_mut(&mut doc, "text").set_string_value(TestUtil::random_analysis_string(
            &*random(),
            20,
            true,
        ))?;
        let random_string = TestUtil::random_simple_string_max(&*random(), 5);
        field_mut_as::<StringField>(&mut doc, "string").set_string_value(random_string.clone())?;
        field_mut_as::<SortedDocValuesField>(&mut doc, "string")
            .set_bytes_value(BytesRef::from_char_sequence(&random_string))?;
        let number = random().next_int_bound(10) as i64;
        field_mut_as::<LongPoint>(&mut doc, "long").set_long_value(number)?;
        field_mut_as::<NumericDocValuesField>(&mut doc, "long").set_value(number);
        // `Arrays.fill(vector.vectorValue(), i % 4)`: the array the field
        // holds, changed through the field.
        field_mut_as::<KnnFloatVectorField>(&mut doc, "vector")
            .set_vector_value(&[(i % 4) as f32; 16])?;
        riw.add_document(&doc)?;
    }
    Ok(())
}

/// `conf.setUseCompoundFile(false); conf.getMergePolicy().setNoCFSRatio(0.0);`
pub fn disable_cfs(conf: &mut IndexWriterConfig) -> Result<()> {
    conf.set_use_compound_file(false)?;
    conf.merge_policy().set_no_cfs_ratio(0.0)
}

/// `dirCopy.sync(Collections.singleton(name))`.
pub fn sync_one(dir: &Arc<dyn rucene::store::Directory>, name: &str) -> Result<()> {
    dir.sync(&[name.to_string()])
}

/// `java.util.concurrent.CountDownLatch`: `await` blocks until `countDown`
/// has been called `count` times.
pub struct CountDownLatch {
    count: std::sync::Mutex<i64>,
    cond: std::sync::Condvar,
}

impl CountDownLatch {
    /// `new CountDownLatch(count)`.
    pub fn new(count: i64) -> Self {
        CountDownLatch {
            count: std::sync::Mutex::new(count),
            cond: std::sync::Condvar::new(),
        }
    }

    /// `latch.await()`.
    pub fn await_(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count > 0 {
            count = self.cond.wait(count).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// `latch.await(millis, TimeUnit.MILLISECONDS)`: whether the count
    /// reached zero before the wait timed out.
    pub fn await_millis(&self, millis: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count > 0 {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            count = self
                .cond
                .wait_timeout(count, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }

    /// `latch.getCount()`.
    pub fn get_count(&self) -> i64 {
        *self.count.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `latch.countDown()`.
    pub fn count_down(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        if *count > 0 {
            *count -= 1;
            if *count == 0 {
                self.cond.notify_all();
            }
        }
    }
}

/// A `public static void name()` method annotated `annotation`
/// (`@BeforeClass`, `@AfterClass`).
pub fn static_hook(
    name: &str,
    annotation: impl randomizedtesting::jdk::reflect::JavaAnnotation,
    body: fn() -> Result<()>,
) -> randomizedtesting::jdk::reflect::Method {
    use randomizedtesting::jdk::reflect::{modifier, Method};
    Method::new(name)
        .modifiers(modifier::PUBLIC | modifier::STATIC)
        .throws(&["java.lang.Exception"])
        .annotate(annotation)
        .static_body(move || {
            if let Err(e) = body() {
                rucene_test_framework::util::java_exceptions::rethrow_java(e)
            }
        })
}

/// `assertNull(message, object)`.
pub fn assert_null_msg<T: std::fmt::Debug>(message: &str, object: &Option<T>) {
    if let Some(o) = object {
        randomizedtesting::junit::Assert::fail(&format!("{message} expected null, but was:<{o:?}>"))
    }
}

/// `assertNotNull(message, object)`.
pub fn assert_not_null_msg<T>(message: &str, object: &Option<T>) {
    if object.is_none() {
        randomizedtesting::junit::Assert::fail(message)
    }
}

/// `TestIndexWriter.assertNoUnreferencedFiles(dir, message)`
/// (`TestIndexWriter.java:218-234`), which other index tests call.
pub fn assert_no_unreferenced_files(
    dir: &Arc<dyn rucene::store::Directory>,
    message: &str,
) -> Result<()> {
    let mut start_files = dir.list_all()?;
    rucene::index::index_writer::IndexWriter::new(
        Arc::clone(dir),
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )?
    .rollback()?;
    let mut end_files = dir.list_all()?;

    start_files.sort();
    end_files.sort();

    if start_files != end_files {
        randomizedtesting::junit::Assert::fail(&format!(
            "{message}: before delete:\n    {}\n  after delete:\n    {}",
            start_files.join("\n    "),
            end_files.join("\n    ")
        ));
    }
    Ok(())
}

/// Forwards the named `LeafReader` members of a `FilterLeafReader` subclass
/// to its base, the core `FilterLeafReader` held in the named field: a copy
/// of `rucene_test_framework::index`'s crate-private `forward_leaf_reader!`
/// (mode `filter`), for the index tests' own subclasses.
#[macro_export]
macro_rules! forward_leaf_reader {
    ($mode:ident $inner:ident; $($m:ident),* $(,)?) => {
        $( $crate::forward_leaf_reader!(@ $mode $inner $m); )*
    };
    (@ $mode:ident $inner:ident core) => {
        fn core(&self) -> &rucene::index::index_reader::IndexReaderCore {
            rucene::index::LeafReader::core(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident filter_leaf_reader_delegate) => {
        /// `this instanceof FilterLeafReader`: the delegate `getDelegate()`
        /// returns.
        fn filter_leaf_reader_delegate(
            &self,
        ) -> Option<&std::sync::Arc<dyn rucene::index::LeafReader>> {
            rucene::index::LeafReader::filter_leaf_reader_delegate(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident do_close) => {
        /// The base class's `doClose()`: `in.close()` for a
        /// `FilterLeafReader`, `in.doClose()` for a `FilterCodecReader`.
        fn do_close(&self) -> rucene::error::Result<()> {
            rucene::index::LeafReader::do_close(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident term_vectors) => {
        fn term_vectors(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::leaf_reader::TermVectors>> {
            (&self.$inner as &dyn rucene::index::LeafReader).term_vectors()
        }
    };
    (@ $mode:ident $inner:ident num_docs) => {
        fn num_docs(&self) -> i32 {
            (&self.$inner as &dyn rucene::index::LeafReader).num_docs()
        }
    };
    (@ $mode:ident $inner:ident max_doc) => {
        fn max_doc(&self) -> i32 {
            (&self.$inner as &dyn rucene::index::LeafReader).max_doc()
        }
    };
    (@ $mode:ident $inner:ident stored_fields) => {
        fn stored_fields(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::index_reader::StoredFields>> {
            (&self.$inner as &dyn rucene::index::LeafReader).stored_fields()
        }
    };
    (@ $mode:ident $inner:ident get_reader_cache_helper) => {
        fn get_reader_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_reader_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident get_core_cache_helper) => {
        fn get_core_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_core_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident terms) => {
        fn terms(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::Terms>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).terms(field)
        }
    };
    (@ $mode:ident $inner:ident get_numeric_doc_values) => {
        fn get_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_binary_doc_values) => {
        fn get_binary_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_binary_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_doc_values) => {
        fn get_sorted_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_numeric_doc_values) => {
        fn get_sorted_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_set_doc_values) => {
        fn get_sorted_set_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_set_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_norm_values) => {
        fn get_norm_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_norm_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_doc_values_skipper) => {
        fn get_doc_values_skipper(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_doc_values_skipper(field)
        }
    };
    (@ $mode:ident $inner:ident get_float_vector_values) => {
        fn get_float_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_float_vector_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_byte_vector_values) => {
        fn get_byte_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_byte_vector_values(field)
        }
    };
    (@ $mode:ident $inner:ident search_nearest_vectors) => {
        fn search_nearest_vectors(
            &self,
            field: &str,
            target: &[f32],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            self.$inner
                .search_nearest_vectors(field, target, collector, accept_docs)
        }
    };
    (@ $mode:ident $inner:ident search_nearest_vectors_byte) => {
        fn search_nearest_vectors_byte(
            &self,
            field: &str,
            target: &[u8],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            self.$inner
                .search_nearest_vectors_byte(field, target, collector, accept_docs)
        }
    };
    (@ $mode:ident $inner:ident get_field_infos) => {
        fn get_field_infos(&self) -> rucene::index::FieldInfos {
            (&self.$inner as &dyn rucene::index::LeafReader).get_field_infos()
        }
    };
    (@ $mode:ident $inner:ident get_live_docs) => {
        fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_live_docs()
        }
    };
    (@ $mode:ident $inner:ident get_point_values) => {
        fn get_point_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::PointValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_point_values(field)
        }
    };
    (@ $mode:ident $inner:ident check_integrity) => {
        fn check_integrity(&self) -> rucene::error::Result<()> {
            (&self.$inner as &dyn rucene::index::LeafReader).check_integrity()
        }
    };
    (@ $mode:ident $inner:ident get_meta_data) => {
        fn get_meta_data(&self) -> rucene::index::LeafMetaData {
            (&self.$inner as &dyn rucene::index::LeafReader).get_meta_data()
        }
    };
}

/// `TestIndexWriter.addDoc(writer)` (`TestIndexWriter.java:203-207`), which
/// other index tests call.
pub fn add_doc(writer: &rucene::index::index_writer::IndexWriter) -> Result<()> {
    let mut doc = rucene::document::Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        rucene::document::Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `TestIndexWriter.addDocWithIndex(writer, index)`
/// (`TestIndexWriter.java:209-214`), over `storedTextType`, a copy of
/// `TextField.TYPE_NOT_STORED` (`TestIndexWriter.java:125`).
pub fn add_doc_with_index(
    writer: &rucene::index::index_writer::IndexWriter,
    index: i32,
) -> Result<()> {
    let stored_text_type =
        rucene::document::FieldType::new_from(rucene::document::TextField::type_not_stored());
    let mut doc = rucene::document::Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "content",
        &format!("aaa {index}"),
        &stored_text_type,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "id",
        &index.to_string(),
        &stored_text_type,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `new FilterMergePolicy(in) { public boolean
/// keepFullyDeletedSegment(IOSupplier<CodecReader> readerIOSupplier) {
/// return true; } }`: the anonymous subclass several index tests use to keep
/// fully deleted segments; every other decision is `FilterMergePolicy`'s
/// delegation.
pub struct KeepFullyDeletedSegmentsMergePolicy {
    filter: rucene::index::merge_policy::FilterMergePolicy,
    /// The answer of `keepFullyDeletedSegment`: `None` for `return true`,
    /// otherwise the flag's current value (`return
    /// keepFullyDeletedSegments.get()`, `TestIndexWriter.testMergeAllDeleted`).
    keep: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl KeepFullyDeletedSegmentsMergePolicy {
    /// `new FilterMergePolicy(in) { … }`.
    pub fn new(inner: Arc<dyn rucene::index::merge_policy::MergePolicy>) -> Self {
        KeepFullyDeletedSegmentsMergePolicy {
            filter: rucene::index::merge_policy::FilterMergePolicy::new(inner),
            keep: None,
        }
    }

    /// `new FilterMergePolicy(in) { … return keep.get(); }`: keeps a fully
    /// deleted segment while `keep` is set.
    pub fn with_flag(
        inner: Arc<dyn rucene::index::merge_policy::MergePolicy>,
        keep: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        KeepFullyDeletedSegmentsMergePolicy {
            filter: rucene::index::merge_policy::FilterMergePolicy::new(inner),
            keep: Some(keep),
        }
    }
}

impl std::fmt::Display for KeepFullyDeletedSegmentsMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.filter, f)
    }
}

impl rucene::index::merge_policy::MergePolicy for KeepFullyDeletedSegmentsMergePolicy {
    /// The subclass is an instance of `FilterMergePolicy`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(&self.filter)
    }

    fn find_merges(
        &self,
        merge_trigger: rucene::index::merge_policy::MergeTrigger,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn rucene::index::codec_reader::CodecReader>],
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter.find_merges_for_readers(readers)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &rucene::index::SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &std::collections::HashMap<String, bool>,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: rucene::index::merge_policy::MergeTrigger,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter
            .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
    }

    fn use_compound_file(
        &self,
        infos: &rucene::index::SegmentInfos,
        merged_info: &rucene::index::SegmentCommitInfo,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
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

    fn size(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<i64> {
        self.filter.size(info, merge_context)
    }

    /// The override: `return true`, or the flag's value.
    fn keep_fully_deleted_segment(
        &self,
        _reader_supplier: &mut rucene::index::merge_policy::CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        Ok(self
            .keep
            .as_ref()
            .is_none_or(|k| k.load(std::sync::atomic::Ordering::SeqCst)))
    }

    fn num_deletes_to_merge_default(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        del_count: i32,
    ) -> i32 {
        self.filter.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<
            Arc<dyn rucene::index::codec_reader::CodecReader>,
        >,
    ) -> Result<i32> {
        self.filter
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.filter.max_full_flush_merge_size()
    }
}

/// `java.util.concurrent.CyclicBarrier` (no barrier action): `parties`
/// threads wait for each other at [`await_millis`](Self::await_millis); the
/// barrier then resets for the next generation. A timed-out wait breaks the
/// barrier for every waiter of its generation.
pub struct CyclicBarrier {
    parties: i32,
    state: std::sync::Mutex<CyclicBarrierState>,
    cond: std::sync::Condvar,
}

struct CyclicBarrierState {
    count: i32,
    generation: u64,
    broken: bool,
}

/// Why [`CyclicBarrier::await_millis`] failed: the Java exception class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierError {
    /// `java.util.concurrent.BrokenBarrierException`.
    Broken,
    /// `java.util.concurrent.TimeoutException`.
    Timeout,
}

impl BarrierError {
    /// The Java exception class name.
    pub fn java_class(self) -> &'static str {
        match self {
            BarrierError::Broken => "java.util.concurrent.BrokenBarrierException",
            BarrierError::Timeout => "java.util.concurrent.TimeoutException",
        }
    }
}

impl CyclicBarrier {
    /// `new CyclicBarrier(parties)`.
    pub fn new(parties: i32) -> Self {
        CyclicBarrier {
            parties,
            state: std::sync::Mutex::new(CyclicBarrierState {
                count: parties,
                generation: 0,
                broken: false,
            }),
            cond: std::sync::Condvar::new(),
        }
    }

    /// `barrier.await(millis, TimeUnit.MILLISECONDS)`: the arrival index of
    /// the caller, or why the barrier failed.
    pub fn await_millis(&self, millis: u64) -> std::result::Result<i32, BarrierError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.broken {
            return Err(BarrierError::Broken);
        }
        state.count -= 1;
        let index = state.count;
        if index == 0 {
            // the last party trips the barrier and starts a new generation
            state.count = self.parties;
            state.generation += 1;
            self.cond.notify_all();
            return Ok(0);
        }
        let generation = state.generation;
        let _waiting = rucene::util::concurrent::enter_thread_wait(
            rucene::util::concurrent::ThreadWaitState::TimedWaiting,
        );
        loop {
            if state.broken {
                return Err(BarrierError::Broken);
            }
            if state.generation != generation {
                return Ok(index);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                state.broken = true;
                self.cond.notify_all();
                return Err(BarrierError::Timeout);
            }
            state = self
                .cond
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

/// A `Directory` reference the Java code shares: an `Arc<dyn Directory>` as
/// the `Box<dyn Directory>` a wrapper (`new FilterDirectory(dir) {}`) owns.
/// Every method delegates to the shared directory; `close()` does not close
/// it, since its other owners still use it (the Java tests never close the
/// wrapper).
pub struct SharedDirectory {
    r#in: Arc<dyn rucene::store::Directory>,
}

impl SharedDirectory {
    /// Wraps `r#in`.
    pub fn new(r#in: Arc<dyn rucene::store::Directory>) -> Self {
        SharedDirectory { r#in }
    }
}

impl rucene::store::Directory for SharedDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }
    fn create_output(
        &self,
        name: &str,
        context: &dyn rucene::store::IOContext,
    ) -> Result<Box<dyn rucene::store::IndexOutput>> {
        self.r#in.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn rucene::store::IOContext,
    ) -> Result<Box<dyn rucene::store::IndexOutput>> {
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
    fn open_input(
        &self,
        name: &str,
        context: &dyn rucene::store::IOContext,
    ) -> Result<Box<dyn rucene::store::IndexInput>> {
        self.r#in.open_input(name, context)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn rucene::store::Lock>> {
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
        self.r#in.directory_to_string()
    }
}

/// `super.findMerges(...)` / `super.findFullFlushMerges(...)` reached from an
/// override: the overridden policy, the trigger, the infos and the context.
pub type FindMergesFn = Arc<
    dyn Fn(
            &dyn rucene::index::merge_policy::MergePolicy,
            rucene::index::merge_policy::MergeTrigger,
            &rucene::index::SegmentInfos,
            &dyn rucene::index::merge_policy::MergeContext,
        ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>>
        + Send
        + Sync,
>;

/// An anonymous subclass of a concrete `MergePolicy` (`new
/// TieredMergePolicy() { findMerges(...) }`) that overrides `findMerges`:
/// every other method is the wrapped policy's, except
/// `findFullFlushMerges` and `isMerged`, which keep `MergePolicy`'s
/// inherited bodies so that they dispatch to the overridden `findMerges`, as
/// Java's virtual calls do. Only valid over a policy that does not override
/// those two (`TieredMergePolicy`, `LogMergePolicy`).
pub struct SubclassMergePolicy {
    base: Arc<dyn rucene::index::merge_policy::MergePolicy>,
    find_merges: FindMergesFn,
}

impl SubclassMergePolicy {
    /// `base` with `findMerges` overridden by `find_merges`.
    pub fn new(
        base: Arc<dyn rucene::index::merge_policy::MergePolicy>,
        find_merges: FindMergesFn,
    ) -> Self {
        SubclassMergePolicy { base, find_merges }
    }
}

impl std::fmt::Display for SubclassMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&*self.base, f)
    }
}

impl rucene::index::merge_policy::MergePolicy for SubclassMergePolicy {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.base.as_any()
    }
    fn find_merges(
        &self,
        merge_trigger: rucene::index::merge_policy::MergeTrigger,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        (self.find_merges)(&*self.base, merge_trigger, segment_infos, merge_context)
    }
    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn rucene::index::codec_reader::CodecReader>],
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.base.find_merges_for_readers(readers)
    }
    fn find_forced_merges(
        &self,
        segment_infos: &rucene::index::SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &std::collections::HashMap<String, bool>,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.base.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }
    fn find_forced_deletes_merges(
        &self,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.base
            .find_forced_deletes_merges(segment_infos, merge_context)
    }
    fn use_compound_file(
        &self,
        infos: &rucene::index::SegmentInfos,
        merged_info: &rucene::index::SegmentCommitInfo,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<bool> {
        self.base
            .use_compound_file(infos, merged_info, merge_context)
    }
    fn get_no_cfs_ratio(&self) -> f64 {
        self.base.get_no_cfs_ratio()
    }
    fn get_max_cfs_segment_size(&self) -> i64 {
        self.base.get_max_cfs_segment_size()
    }
    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.base.get_max_cfs_segment_size_mb()
    }
    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.base.set_no_cfs_ratio(no_cfs_ratio)
    }
    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.base.set_max_cfs_segment_size_mb(v)
    }
    fn size(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<i64> {
        self.base.size(info, merge_context)
    }
    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut rucene::index::merge_policy::CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.base.keep_fully_deleted_segment(reader_supplier)
    }
    fn num_deletes_to_merge_default(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        del_count: i32,
    ) -> i32 {
        self.base.num_deletes_to_merge_default(info, del_count)
    }
    fn num_deletes_to_merge_with_reader(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<
            Arc<dyn rucene::index::codec_reader::CodecReader>,
        >,
    ) -> Result<i32> {
        self.base
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }
    fn max_full_flush_merge_size(&self) -> i64 {
        self.base.max_full_flush_merge_size()
    }
}

/// An anonymous (or nested) subclass of `FilterMergePolicy` overriding
/// `findMerges` and/or `findFullFlushMerges`; each override receives the
/// `FilterMergePolicy` as `super`. Every other method is `FilterMergePolicy`'s.
pub struct FilterMergePolicyOverrides {
    filter: rucene::index::merge_policy::FilterMergePolicy,
    find_merges: Option<FindMergesFn>,
    find_full_flush_merges: Option<FindMergesFn>,
}

impl FilterMergePolicyOverrides {
    /// `new FilterMergePolicy(in) { ... }` with the given overrides.
    pub fn new(
        r#in: Arc<dyn rucene::index::merge_policy::MergePolicy>,
        find_merges: Option<FindMergesFn>,
        find_full_flush_merges: Option<FindMergesFn>,
    ) -> Self {
        FilterMergePolicyOverrides {
            filter: rucene::index::merge_policy::FilterMergePolicy::new(r#in),
            find_merges,
            find_full_flush_merges,
        }
    }
}

impl std::fmt::Display for FilterMergePolicyOverrides {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.filter, f)
    }
}

impl rucene::index::merge_policy::MergePolicy for FilterMergePolicyOverrides {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        rucene::index::merge_policy::MergePolicy::as_any(&self.filter)
    }
    fn find_merges(
        &self,
        merge_trigger: rucene::index::merge_policy::MergeTrigger,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        match &self.find_merges {
            Some(f) => f(&self.filter, merge_trigger, segment_infos, merge_context),
            None => self
                .filter
                .find_merges(merge_trigger, segment_infos, merge_context),
        }
    }
    fn find_full_flush_merges(
        &self,
        merge_trigger: rucene::index::merge_policy::MergeTrigger,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        match &self.find_full_flush_merges {
            Some(f) => f(&self.filter, merge_trigger, segment_infos, merge_context),
            None => self
                .filter
                .find_full_flush_merges(merge_trigger, segment_infos, merge_context),
        }
    }
    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn rucene::index::codec_reader::CodecReader>],
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter.find_merges_for_readers(readers)
    }
    fn find_forced_merges(
        &self,
        segment_infos: &rucene::index::SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &std::collections::HashMap<String, bool>,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }
    fn find_forced_deletes_merges(
        &self,
        segment_infos: &rucene::index::SegmentInfos,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.filter
            .find_forced_deletes_merges(segment_infos, merge_context)
    }
    fn use_compound_file(
        &self,
        infos: &rucene::index::SegmentInfos,
        merged_info: &rucene::index::SegmentCommitInfo,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
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
    fn size(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<i64> {
        self.filter.size(info, merge_context)
    }
    fn is_merged(
        &self,
        infos: &rucene::index::SegmentInfos,
        info: &rucene::index::SegmentCommitInfo,
        merge_context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> Result<bool> {
        self.filter.is_merged(infos, info, merge_context)
    }
    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut rucene::index::merge_policy::CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.filter.keep_fully_deleted_segment(reader_supplier)
    }
    fn num_deletes_to_merge_default(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        del_count: i32,
    ) -> i32 {
        self.filter.num_deletes_to_merge_default(info, del_count)
    }
    fn num_deletes_to_merge_with_reader(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<
            Arc<dyn rucene::index::codec_reader::CodecReader>,
        >,
    ) -> Result<i32> {
        self.filter
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }
    fn max_full_flush_merge_size(&self) -> i64 {
        self.filter.max_full_flush_merge_size()
    }
}

/// An anonymous subclass of a merge scheduler overriding
/// `merge(MergeSource, MergeTrigger)` with a prologue before
/// `super.merge(mergeSource, trigger)`; every other method is the wrapped
/// scheduler's (`instanceof` included, through `as_any`). `synchronized`
/// holds one lock across the prologue and `super.merge`, as a
/// `synchronized` override does.
pub struct PrologueMergeScheduler {
    base: Arc<dyn rucene::index::merge_scheduler::MergeScheduler>,
    prologue: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    monitor: Option<std::sync::Mutex<()>>,
}

impl PrologueMergeScheduler {
    /// `base` whose `merge` first runs `prologue`.
    pub fn new(
        base: Arc<dyn rucene::index::merge_scheduler::MergeScheduler>,
        synchronized: bool,
        prologue: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Self {
        PrologueMergeScheduler {
            base,
            prologue,
            monitor: synchronized.then(|| std::sync::Mutex::new(())),
        }
    }
}

impl std::fmt::Display for PrologueMergeScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&*self.base, f)
    }
}

impl rucene::index::merge_scheduler::MergeScheduler for PrologueMergeScheduler {
    fn merge(
        &self,
        merge_source: Arc<dyn rucene::index::merge_scheduler::MergeSource>,
        trigger: rucene::index::merge_policy::MergeTrigger,
    ) -> Result<()> {
        let _monitor = self
            .monitor
            .as_ref()
            .map(|m| m.lock().unwrap_or_else(|e| e.into_inner()));
        (self.prologue)()?;
        self.base.merge(merge_source, trigger)
    }
    fn wrap_for_merge(
        &self,
        merge: &rucene::index::merge_policy::OneMerge,
        directory: Arc<dyn rucene::store::Directory>,
    ) -> Result<Arc<dyn rucene::store::Directory>> {
        self.base.wrap_for_merge(merge, directory)
    }
    fn same_thread_executor(&self) -> &Arc<rucene::util::SameThreadExecutorService> {
        self.base.same_thread_executor()
    }
    fn get_intra_merge_executor(
        &self,
        merge: &rucene::index::merge_policy::OneMerge,
    ) -> Result<Option<Arc<dyn rucene::search::Executor>>> {
        self.base.get_intra_merge_executor(merge)
    }
    fn close(&self) -> Result<()> {
        self.base.close()
    }
    fn initialize(
        &self,
        info_stream: Arc<dyn rucene::util::InfoStream>,
        directory: &Arc<dyn rucene::store::Directory>,
    ) -> Result<()> {
        self.base.initialize(info_stream, directory)
    }
    fn set_info_stream(&self, info_stream: Arc<dyn rucene::util::InfoStream>) {
        self.base.set_info_stream(info_stream)
    }
    fn get_info_stream(&self) -> Option<Arc<dyn rucene::util::InfoStream>> {
        self.base.get_info_stream()
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.base.as_any()
    }
}

/// The `Field` named `name` that the Java test keeps a reference to after
/// `doc.add(field)` and changes in place, reached through the document.
///
/// # Panics
///
/// `ClassCastException` when the document has no `Field` of that name.
pub fn field_mut<'a>(
    doc: &'a mut rucene::document::Document,
    name: &str,
) -> &'a rucene::document::Field {
    field_mut_as::<rucene::document::Field>(doc, name)
}

/// The field of class `T` named `name` that the Java test keeps a reference
/// to after `doc.add(field)` and changes in place, reached through the
/// document ([`rucene::document::Document::get_field_mut_as`]).
///
/// # Panics
///
/// `ClassCastException` when the document has no field of that name and
/// class.
pub fn field_mut_as<'a, T: rucene::index::IndexableField + 'static>(
    doc: &'a mut rucene::document::Document,
    name: &str,
) -> &'a T {
    doc.get_field_mut_as::<T>(name).unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            format!("no {} named {name}", std::any::type_name::<T>()),
        )
        .throw()
    })
}
