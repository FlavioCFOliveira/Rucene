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
    let mut iwc = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    iwc.set_ram_buffer_size_mb(256.0)?;
    iwc.set_merge_scheduler(Arc::new(ConcurrentMergeScheduler::new()))?;
    iwc.set_merge_policy(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
    ))?;
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
        riw.add_document(&docs.next_doc()?)?;
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
///
/// Java adds the fields to one `Document` once and changes their values
/// before each `addDocument`; a Rucene `Document` owns its fields, so each
/// iteration builds the document again from fields of the same names, types
/// and order carrying the new values, with the same draws (guide section 6).
pub fn add_mutated_docs(riw: &rucene_test_framework::index::RandomIndexWriter) -> Result<()> {
    use rucene::document::{
        Document, Field, FieldType, KnnFloatVectorField, LongPoint, NumericDocValuesField,
        SortedDocValuesField, Store, StringField, TextField,
    };
    use rucene::util::BytesRef;

    let mut text_with_term_vectors_type = FieldType::new_from(TextField::type_stored());
    text_with_term_vectors_type.set_store_term_vectors(true)?;
    let mut vector = [0f32; 16];
    for i in 0..100 {
        let text = TestUtil::random_analysis_string(&*random(), 20, true);
        let random_string = TestUtil::random_simple_string_max(&*random(), 5);
        let number = random().next_int_bound(10) as i64;
        vector.fill((i % 4) as f32);
        let mut doc = Document::new();
        doc.add(Box::new(Field::new(
            "text",
            text,
            text_with_term_vectors_type.clone(),
        )?));
        doc.add(Box::new(StringField::new(
            "string",
            random_string.clone(),
            Store::YES,
        )?));
        doc.add(Box::new(SortedDocValuesField::new(
            "string",
            BytesRef::from_char_sequence(&random_string),
        )));
        doc.add(Box::new(LongPoint::new("long", &[number])?));
        doc.add(Box::new(NumericDocValuesField::new("long", number)));
        doc.add(Box::new(KnnFloatVectorField::with_euclidean(
            "vector", &vector,
        )?));
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
        IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random()))),
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

/// The same `Field` object added to a document again: a field with the same
/// name, current string value and type.
///
/// **Divergence from Lucene 10.5.0.** Java tests keep a reference to a field
/// they added to a `Document`, call `setStringValue` on it and add the
/// document again; a Rucene `Document` owns its fields, so the test keeps the
/// field and rebuilds the document from copies of it, in the same order and
/// without drawing again (guide section 6).
pub fn copy_field(field: &rucene::document::Field) -> Result<rucene::document::Field> {
    rucene::document::Field::new(
        field.name(),
        field.string_value().unwrap_or_default(),
        rucene::document::FieldType::new_from(field.field_type()),
    )
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
