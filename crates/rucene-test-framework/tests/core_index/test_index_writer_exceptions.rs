//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterExceptions.java`.
//!
//! * A `RandomIndexWriter.TestPoint` or an `InfoStream` that throws an
//!   unchecked Java exception (`RuntimeException`, `OutOfMemoryError`,
//!   `AssertionError`) panics with that throwable: Rucene's `TestPoint.apply`
//!   and `InfoStream.message` return nothing. `catch (RuntimeException e)`
//!   around a writer call catches the error the call returns and the
//!   throwable it panics with ([`catching`]).
//! * The `ThreadLocal<Thread> doFail` field is a thread-local flag, reset on
//!   the test thread when a test starts (a fresh test instance).
//! * A field changed after `doc.add` is reached through
//!   `Document::get_field_mut`.
//! * `callStackContains(X.class, "javaName")` names the Rust frame of the
//!   method (`sync`, `prepare_commit`, `rollback_internal`, …).
//! * `new Throwable().printStackTrace(System.out)` prints the Rust backtrace.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, AnalyzerState, GlobalReuseStrategy,
    PerFieldReuseStrategy, StringCharReader, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{
    BinaryDocValuesField, Document, Field, FieldType, IntPoint, NumericDocValuesField,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    StringField, TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_file_names::{file_name_from_generation, SEGMENTS};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{NoMergePolicy, TieredMergePolicy};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::multi_bits::MultiBits;
use rucene::index::terms::Term;
use rucene::index::{
    DocumentsIterable, DocumentsIterator, IndexableFieldIterable, IndexableFieldType,
};
use rucene::index::{
    IndexFileDeleter, IndexOptions, IndexReader, SegmentInfos, TermVectorsConsumer, WRITE_LOCK_NAME,
};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::{MatchAllDocsQuery, Query};
use rucene::store::java_nio_file::JavaIoException;
use rucene::store::{
    BufferedChecksumIndexInput, ByteBuffersDirectory, Directory, FilterDirectory, IOContext,
    IndexInput, IndexOutput, Lock, READONCE_IO_CONTEXT,
};
use rucene::util::attribute::AttributeSource;
use rucene::util::{BytesRef, InfoStream};
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD, SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, MockAnalyzer,
};
use rucene_test_framework::index::random_index_writer::TestPoint;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::mock_directory_wrapper::{fake_io_exception, FAKE_IO_EXCEPTION};
use rucene_test_framework::store::{Failure, MockDirectoryWrapper};
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{
    NewFieldValue, Nightly, SuppressCodecs, TestDirectory, RANDOM_MULTIPLIER, TEST_NIGHTLY,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class_annotated, random,
    shuffle, test, try_catch, verbose,
};
use crate::support::{add_doc, as_cms, npe, KeepFullyDeletedSegmentsMergePolicy};

const ISE: &str = "java.lang.IllegalStateException";
const IAE: &str = "java.lang.IllegalArgumentException";
const IOE: &str = "java.io.IOException";
const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";
const OOME: &str = "java.lang.OutOfMemoryError";

/// `TestIndexWriterExceptions` (`@SuppressCodecs("SimpleText") // too slow
/// here`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterExceptions",
        vec![
            test("testRandomExceptions", test_random_exceptions),
            test(
                "testRandomExceptionsThreads",
                test_random_exceptions_threads,
            ),
            test(
                "testExceptionDocumentsWriterInit",
                test_exception_documents_writer_init,
            ),
            test(
                "testExceptionJustBeforeFlush",
                test_exception_just_before_flush,
            ),
            test("testExceptionOnMergeInit", test_exception_on_merge_init),
            test(
                "testExceptionFromTokenStream",
                test_exception_from_token_stream,
            ),
            test("testDocumentsWriterAbort", test_documents_writer_abort),
            test(
                "testDocumentsWriterExceptions",
                test_documents_writer_exceptions,
            ),
            test(
                "testDocumentsWriterExceptionFailOneDoc",
                test_documents_writer_exception_fail_one_doc,
            ),
            test(
                "testDocumentsWriterExceptionThreads",
                test_documents_writer_exception_threads,
            ),
            test("testExceptionDuringSync", test_exception_during_sync),
            test("testExceptionsDuringCommit", test_exceptions_during_commit),
            test("testForceMergeExceptions", test_force_merge_exceptions),
            test(
                "testOutOfMemoryErrorCausesCloseToFail",
                test_out_of_memory_error_causes_close_to_fail,
            ),
            test(
                "testOutOfMemoryErrorRollback",
                test_out_of_memory_error_rollback,
            ),
            test("testRollbackExceptionHang", test_rollback_exception_hang),
            test("testSegmentsChecksumError", test_segments_checksum_error),
            test("testSimulatedCorruptIndex1", test_simulated_corrupt_index1),
            test("testSimulatedCorruptIndex2", test_simulated_corrupt_index2),
            test("testTermVectorExceptions", test_term_vector_exceptions),
            test(
                "testAddDocsNonAbortingException",
                test_add_docs_non_aborting_exception,
            ),
            test(
                "testUpdateDocsNonAbortingException",
                test_update_docs_non_aborting_exception,
            ),
            test("testNullStoredField", test_null_stored_field),
            test("testNullStoredFieldReuse", test_null_stored_field_reuse),
            test("testNullStoredBytesField", test_null_stored_bytes_field),
            test(
                "testNullStoredBytesFieldReuse",
                test_null_stored_bytes_field_reuse,
            ),
            test(
                "testNullStoredBytesRefField",
                test_null_stored_bytes_ref_field,
            ),
            test(
                "testNullStoredBytesRefFieldReuse",
                test_null_stored_bytes_ref_field_reuse,
            ),
            test(
                "testNullStoredDataInputField",
                test_null_stored_data_input_field,
            ),
            test(
                "testCrazyPositionIncrementGap",
                test_crazy_position_increment_gap,
            ),
            test("testExceptionOnCtor", test_exception_on_ctor),
            test("testTooManyFileException", test_too_many_file_exception),
            test("testTooManyTokens", test_too_many_tokens).annotate(Nightly),
            test(
                "testExceptionDuringRollback",
                test_exception_during_rollback,
            ),
            test(
                "testRandomExceptionDuringRollback",
                test_random_exception_during_rollback,
            ),
            test("testMergeExceptionIsTragic", test_merge_exception_is_tragic).annotate(Nightly),
            test(
                "testOnlyRollbackOnceOnException",
                test_only_rollback_once_on_exception,
            ),
            test(
                "testExceptionOnSyncMetadata",
                test_exception_on_sync_metadata,
            ),
            test(
                "testExceptionJustBeforeFlushWithPointValues",
                test_exception_just_before_flush_with_point_values,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------

/// `try { body } catch (Throwable t)`: the body's value, or the Java
/// throwable it returned as an error or panicked with.
fn catching<T>(body: impl FnOnce() -> Result<T>) -> std::result::Result<T, Throwable> {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(java_throwable(&e)),
        Err(payload) => Err(Throwable::from_panic(payload)),
    }
}

/// `Thread.currentThread().getName()`.
fn current_thread_name() -> String {
    Thread::current().name()
}

/// `new IOException(message)`.
fn io_exception(message: &str) -> LuceneError {
    LuceneError::Io(std::io::Error::other(message.to_string()))
}

/// `new IOException()` (`getMessage()` is `null`).
fn io_exception_no_message() -> LuceneError {
    LuceneError::Io(JavaIoException::new(IOE, None).into_io_error())
}

/// `new RuntimeException(message)`, returned from a `Failure.eval`.
fn runtime_exception(message: &str) -> LuceneError {
    LuceneError::unchecked(RUNTIME_EXCEPTION, message)
}

/// `new MockAnalyzer(random())` as the `Arc<dyn Analyzer>` a config takes.
fn mock_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::new(&*random()))
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(mock_analyzer()))
}

/// `newIndexWriterConfig(null)`.
fn null_analyzer_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(None)
}

/// `new IndexWriterConfig(null)`.
fn plain_null_analyzer_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(None)
}

/// `TestUtil.checkIndex(dir)` over a test directory.
fn check_index(dir: &TestDirectory) -> Result<()> {
    TestUtil::check_index(dir.as_directory())?;
    Ok(())
}

/// The fields of the `DocCopyIterator` static field types.
static CUSTOM1: LazyLock<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(true).expect("not frozen");
    ft.set_store_term_vector_positions(true)
        .expect("not frozen");
    ft.set_store_term_vector_offsets(true).expect("not frozen");
    ft
});

/// `DocCopyIterator.custom2`.
static CUSTOM2: LazyLock<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new();
    ft.set_stored(true).expect("not frozen");
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .expect("not frozen");
    ft
});

/// `DocCopyIterator.custom3`.
static CUSTOM3: LazyLock<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new();
    ft.set_stored(true).expect("not frozen");
    ft
});

/// `DocCopyIterator.custom4`.
static CUSTOM4: LazyLock<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(StringField::type_not_stored());
    ft.set_store_term_vectors(true).expect("not frozen");
    ft.set_store_term_vector_positions(true)
        .expect("not frozen");
    ft.set_store_term_vector_offsets(true).expect("not frozen");
    ft
});

/// `DocCopyIterator.custom5`.
static CUSTOM5: LazyLock<FieldType> = LazyLock::new(|| {
    let ft = FieldType::new_from(TextField::type_stored());
    ft.set_store_term_vectors(true).expect("not frozen");
    ft.set_store_term_vector_positions(true)
        .expect("not frozen");
    ft.set_store_term_vector_offsets(true).expect("not frozen");
    ft
});

/// `idField`: the `id` field the indexer thread's document holds.
fn id_field_of(doc: &mut Document) -> &Field {
    doc.get_field_mut("id")
        .and_then(|field| field.as_any())
        .and_then(|any| any.downcast_ref::<Field>())
        .expect("INVARIANT: the document holds the Field `id`")
}

/// `record DocCopyIterator(Document doc, int count) implements
/// Iterable<Document>`: the same document `count` times.
struct DocCopyIterator<'a> {
    doc: &'a Document,
    count: i32,
}

/// The anonymous `Iterator<Document>` of `DocCopyIterator.iterator()`.
struct DocCopyIter<'a> {
    doc: &'a Document,
    count: i32,
    upto: i32,
}

impl<'a> DocumentsIterator<'a> for DocCopyIter<'a> {
    fn has_next(&mut self) -> Result<bool> {
        Ok(self.upto < self.count)
    }

    fn next(&mut self) -> Result<Option<&'a dyn IndexableFieldIterable>> {
        self.upto += 1;
        Ok(Some(self.doc))
    }
}

impl DocumentsIterable for DocCopyIterator<'_> {
    fn iterator(&self) -> Box<dyn DocumentsIterator<'_> + '_> {
        Box::new(DocCopyIter {
            doc: self.doc,
            count: self.count,
            upto: 0,
        })
    }
}

thread_local! {
    /// `ThreadLocal<Thread> doFail`: whether this thread set itself.
    static DO_FAIL: Cell<bool> = const { Cell::new(false) };
}

/// `doFail.set(this)` / `doFail.set(null)`.
fn set_do_fail(value: bool) {
    DO_FAIL.with(|d| d.set(value));
}

// ---------------------------------------------------------------------------
// IndexerThread and TestPoint1
// ---------------------------------------------------------------------------

/// `private class IndexerThread extends Thread`.
struct IndexerThread {
    name: String,
    writer: Arc<IndexWriter>,
    r: JdkRandom,
    failure: Mutex<Option<Throwable>>,
}

impl IndexerThread {
    /// `new IndexerThread(i, writer)`: `r = new Random(random().nextLong())`.
    fn new(i: i32, writer: &Arc<IndexWriter>) -> Arc<Self> {
        let r = JdkRandom::new(random().next_long());
        Arc::new(IndexerThread {
            name: format!("Indexer {i}"),
            writer: Arc::clone(writer),
            r,
            failure: Mutex::new(None),
        })
    }

    /// `failure`.
    fn failure(&self) -> Option<Throwable> {
        self.failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// `run()`.
    fn run(&self) {
        let r = &self.r;
        let content1 =
            LuceneTestCase::new_text_field_with(r, "content1", "aaa bbb ccc ddd", Store::YES);
        let content6 = LuceneTestCase::new_field_with(
            r,
            "content6",
            NewFieldValue::String("aaa bbb ccc ddd".to_string()),
            &CUSTOM1,
        );
        let content2 = LuceneTestCase::new_field_with(
            r,
            "content2",
            NewFieldValue::String("aaa bbb ccc ddd".to_string()),
            &CUSTOM2,
        );
        let content3 = LuceneTestCase::new_field_with(
            r,
            "content3",
            NewFieldValue::String("aaa bbb ccc ddd".to_string()),
            &CUSTOM3,
        );
        let content4 =
            LuceneTestCase::new_text_field_with(r, "content4", "aaa bbb ccc ddd", Store::NO);
        let content5 =
            LuceneTestCase::new_string_field_with(r, "content5", "aaa bbb ccc ddd", Store::NO);
        let content7 = LuceneTestCase::new_field_with(
            r,
            "content7",
            NewFieldValue::String("aaa bbb ccc ddd".to_string()),
            &CUSTOM4,
        );
        let id_field =
            LuceneTestCase::new_field_with(r, "id", NewFieldValue::String(String::new()), &CUSTOM2);

        let mut doc = Document::new();
        for field in [content1, content6, content2, content3, content4, content5] {
            doc.add(Box::new(field));
        }
        {
            doc.add(Box::new(NumericDocValuesField::new("numericdv", 5)));
            doc.add(Box::new(BinaryDocValuesField::new(
                "binarydv",
                BytesRef::from_char_sequence("hello"),
            )));
            doc.add(Box::new(SortedDocValuesField::new(
                "sorteddv",
                BytesRef::from_char_sequence("world"),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "sortedsetdv",
                BytesRef::from_char_sequence("hellllo"),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "sortedsetdv",
                BytesRef::from_char_sequence("again"),
            )));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "sortednumericdv",
                10,
            )));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "sortednumericdv",
                5,
            )));
            doc.add(Box::new(content7));
            // `final Field idField = ...; doc.add(idField);`
            doc.add(Box::new(id_field));
        }

        let max_iterations = 250;
        let mut iterations = 0;
        loop {
            if verbose() {
                println!("{}: TEST: IndexerThread: cycle", current_thread_name());
            }
            set_do_fail(true);
            let id = format!("{}", r.next_int_bound(50));
            // `idField.setStringValue(id)`.
            id_field_of(&mut doc)
                .set_string_value(id.clone())
                .expect("a String field");
            let id_term = Term::from_text("id", &id);
            let outcome = catching(|| {
                if r.next_boolean() {
                    let docs = DocCopyIterator {
                        doc: &doc,
                        count: TestUtil::next_int(r, 1, 20),
                    };
                    self.writer.update_documents(id_term.clone(), &docs)
                } else {
                    self.writer.update_document(id_term.clone(), &doc)
                }
            });
            match outcome {
                Ok(_) => {}
                Err(re) if re.is_instance(RUNTIME_EXCEPTION) => {
                    if verbose() {
                        println!("{}: EXC: ", current_thread_name());
                        println!("{re}");
                    }
                    match TestUtil::check_index(Arc::clone(self.writer.get_directory())) {
                        Ok(_) => {}
                        Err(ioe) if ioe.is_io_exception() => {
                            println!("{}: unexpected exception1", current_thread_name());
                            println!("{ioe}");
                            *self.failure.lock().unwrap_or_else(|p| p.into_inner()) =
                                Some(java_throwable(&ioe));
                            break;
                        }
                        Err(e) => java_throwable(&e).throw(),
                    }
                }
                Err(t) => {
                    println!("{}: unexpected exception2", current_thread_name());
                    println!("{t}");
                    *self.failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(t);
                    break;
                }
            }

            set_do_fail(false);

            // After a possible exception (above) I should be able
            // to add a new document without hitting an
            // exception:
            if let Err(t) = catching(|| self.writer.update_document(id_term.clone(), &doc)) {
                println!("{}: unexpected exception3", current_thread_name());
                println!("{t}");
                *self.failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(t);
                break;
            }
            iterations += 1;
            if iterations >= max_iterations {
                break;
            }
        }
    }
}

/// `private class TestPoint1 implements RandomIndexWriter.TestPoint`.
struct TestPoint1 {
    r: JdkRandom,
}

impl TestPoint1 {
    /// `new TestPoint1()`: `r = new Random(random().nextLong())`.
    fn new() -> Self {
        TestPoint1 {
            r: JdkRandom::new(random().next_long()),
        }
    }
}

impl TestPoint for TestPoint1 {
    fn apply(&self, name: &str) {
        if DO_FAIL.with(|d| d.get()) && name != "startDoFlush" && self.r.next_int_bound(40) == 17 {
            if verbose() {
                println!("{}: NOW FAIL: {name}", current_thread_name());
                println!("{}", std::backtrace::Backtrace::force_capture());
            }
            Throwable::new(
                RUNTIME_EXCEPTION,
                format!("{}: intentionally failing at {name}", current_thread_name()),
            )
            .throw()
        }
    }
}

/// The body of `testRandomExceptions` and `testRandomExceptionsThreads`
/// after the threads ran: commit, close (or roll back), and check that a
/// document that hit an exception partway through tokenization is deleted.
fn finish_random_exceptions(writer: &Arc<IndexWriter>, mut dir: TestDirectory) -> Result<()> {
    writer.commit()?;

    if let Err(t) = catching(|| writer.close()) {
        println!("exception during close:");
        println!("{t}");
        writer.rollback()?;
    }

    // Confirm that when doc hits exception partway through tokenization, it's deleted:
    let r2 = directory_reader::open(dir.as_directory())?;
    let count = r2.doc_freq(&Term::from_text("content4", "aaa"))?;
    let count2 = r2.doc_freq(&Term::from_text("content4", "ddd"))?;
    assert_equals(&count, &count2);
    r2.close()?;

    dir.close()
}

/// Port of `TestIndexWriterExceptions.testRandomExceptions`.
fn test_random_exceptions() -> Result<()> {
    set_do_fail(false);
    if verbose() {
        println!("\nTEST: start testRandomExceptions");
    }
    let dir = LuceneTestCase::new_directory();

    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_enable_checks(false); // disable workflow checking as we forcefully close() in exceptional cases.

    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    conf.set_ram_buffer_size_mb(0.1)?;
    conf.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    let writer = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        dir.as_directory(),
        conf,
        Arc::new(TestPoint1::new()),
    )?;
    as_cms(&writer.get_config().merge_scheduler()).set_suppress_exceptions();
    // writer.setMaxBufferedDocs(10);
    if verbose() {
        println!("TEST: initial commit");
    }
    writer.commit()?;

    let thread = IndexerThread::new(0, &writer);
    thread.run();
    if let Some(failure) = thread.failure() {
        println!("{failure}");
        Assert::fail(&format!("thread {}: hit unexpected failure", thread.name));
    }

    if verbose() {
        println!("TEST: commit after thread start");
    }
    finish_random_exceptions(&writer, dir)
}

/// Port of `TestIndexWriterExceptions.testRandomExceptionsThreads`.
fn test_random_exceptions_threads() -> Result<()> {
    set_do_fail(false);
    let dir = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_enable_checks(false); // disable workflow checking as we forcefully close() in exceptional cases.
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    conf.set_ram_buffer_size_mb(0.2)?;
    conf.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    let writer = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        dir.as_directory(),
        conf,
        Arc::new(TestPoint1::new()),
    )?;
    as_cms(&writer.get_config().merge_scheduler()).set_suppress_exceptions();
    // writer.setMaxBufferedDocs(10);
    writer.commit()?;

    const NUM_THREADS: i32 = 4;

    let mut threads = Vec::new();
    for i in 0..NUM_THREADS {
        let indexer = IndexerThread::new(i, &writer);
        let body = Arc::clone(&indexer);
        let thread = Thread::new(indexer.name.clone(), move || body.run());
        thread.start();
        threads.push((indexer, thread));
    }

    for (_, thread) in &threads {
        let _ = thread.join();
    }

    for (indexer, _) in &threads {
        if indexer.failure().is_some() {
            Assert::fail(&format!("thread {}: hit unexpected failure", indexer.name));
        }
    }

    finish_random_exceptions(&writer, dir)
}

// ---------------------------------------------------------------------------
// LUCENE-1198, LUCENE-1208, LUCENE-1210, LUCENE-1072
// ---------------------------------------------------------------------------

/// `private static final class TestPoint2` (LUCENE-1198).
#[derive(Default)]
struct TestPoint2 {
    do_fail: AtomicBool,
}

impl TestPoint for TestPoint2 {
    fn apply(&self, name: &str) {
        if self.do_fail.load(Ordering::SeqCst)
            && name == "DocumentsWriterPerThread addDocuments start"
        {
            Throwable::new(RUNTIME_EXCEPTION, "intentionally failing").throw()
        }
    }
}

/// `CRASH_FAIL_MESSAGE`.
const CRASH_FAIL_MESSAGE: &str = "I'm experiencing problems";

/// `private static class CrashingFilter extends TokenFilter`.
#[derive(Debug)]
struct CrashingFilterLogic {
    field_name: String,
    count: i32,
}

impl TokenFilterLogic for CrashingFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.field_name == "crash" {
            let count = self.count;
            self.count += 1;
            if count >= 4 {
                return Err(io_exception(CRASH_FAIL_MESSAGE));
            }
        }
        input.increment_token()
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.count = 0;
        Ok(())
    }
}

/// `new CrashingFilter(fieldName, input)`.
fn crashing_filter(
    field_name: &str,
    input: Box<dyn TokenStream>,
) -> TokenFilter<CrashingFilterLogic> {
    TokenFilter::new(
        input,
        CrashingFilterLogic {
            field_name: field_name.to_string(),
            count: 0,
        },
    )
}

/// The anonymous `Analyzer(PER_FIELD_REUSE_STRATEGY)`s whose components are
/// a `MockTokenizer(WHITESPACE, false)` without checks, wrapped in a
/// `CrashingFilter`: always, or only while `do_crash` is set.
#[derive(Debug)]
struct CrashingAnalyzer {
    state: AnalyzerState,
    do_crash: Option<Arc<AtomicBool>>,
    class_name: &'static str,
}

impl CrashingAnalyzer {
    fn new(do_crash: Option<Arc<AtomicBool>>, class_name: &'static str) -> Self {
        CrashingAnalyzer {
            state: AnalyzerState::new(Box::new(PerFieldReuseStrategy)),
            do_crash,
            class_name,
        }
    }
}

impl Analyzer for CrashingAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.class_name
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        tokenizer.set_enable_checks(false); // disable workflow checking as we forcefully close() in exceptional cases.
        let tokenizer = Arc::new(AtomicRefCell::new(tokenizer));
        let crash = match &self.do_crash {
            Some(flag) => flag.load(Ordering::SeqCst),
            None => true,
        };
        if crash {
            let stream = crashing_filter(field_name, shared(&tokenizer));
            Ok(components(tokenizer, sink(stream)))
        } else {
            Ok(tokenizer_components(tokenizer))
        }
    }
}

/// LUCENE-1198
/// Port of `TestIndexWriterExceptions.testExceptionDocumentsWriterInit`.
fn test_exception_documents_writer_init() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let test_point = Arc::new(TestPoint2::default());
    let w = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        dir.as_directory(),
        mock_iwc(),
        Arc::clone(&test_point) as Arc<dyn TestPoint>,
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a field",
        Store::YES,
    )));
    w.add_document(&doc)?;

    test_point.do_fail.store(true, Ordering::SeqCst);
    expect_throws(RUNTIME_EXCEPTION, || w.add_document(&doc));

    w.close()?;
    dir.close()
}

/// LUCENE-1208
/// Port of `TestIndexWriterExceptions.testExceptionJustBeforeFlush`.
fn test_exception_just_before_flush() -> Result<()> {
    set_do_fail(false);
    let mut dir = LuceneTestCase::new_directory();

    let do_crash = Arc::new(AtomicBool::new(false));

    let analyzer: Arc<dyn Analyzer> = Arc::new(CrashingAnalyzer::new(
        Some(Arc::clone(&do_crash)),
        "org.apache.lucene.index.TestIndexWriterExceptions$1",
    ));

    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    conf.set_max_buffered_docs(2)?;
    let w = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        dir.as_directory(),
        conf,
        Arc::new(TestPoint1::new()),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a field",
        Store::YES,
    )));
    w.add_document(&doc)?;

    let mut crash_doc = Document::new();
    crash_doc.add(Box::new(LuceneTestCase::new_text_field(
        "crash",
        "do it on token 4",
        Store::YES,
    )));
    do_crash.store(true, Ordering::SeqCst);
    expect_throws(IOE, || w.add_document(&crash_doc));

    w.add_document(&doc)?;
    w.close()?;
    dir.close()
}

/// `private static final class TestPoint3`.
#[derive(Default)]
struct TestPoint3 {
    do_fail: AtomicBool,
    failed: AtomicBool,
}

impl TestPoint for TestPoint3 {
    fn apply(&self, name: &str) {
        if self.do_fail.load(Ordering::SeqCst) && name == "startMergeInit" {
            self.failed.store(true, Ordering::SeqCst);
            Throwable::new(RUNTIME_EXCEPTION, "intentionally failing").throw()
        }
    }
}

/// LUCENE-1210
/// Port of `TestIndexWriterExceptions.testExceptionOnMergeInit`.
fn test_exception_on_merge_init() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    conf.set_max_buffered_docs(2)?;
    let mut lmp = LuceneTestCase::new_log_merge_policy();
    let cms = ConcurrentMergeScheduler::new();
    cms.set_suppress_exceptions();
    // `((LogMergePolicy) conf.getMergePolicy()).setMergeFactor(2)` and
    // `.setTargetSearchConcurrency(1)`, on the policy before it is set.
    lmp.set_merge_factor(2)?;
    lmp.set_target_search_concurrency(1)?;
    conf.set_merge_policy(Some(Arc::new(lmp)))?;
    conf.set_merge_scheduler(Some(Arc::new(cms)))?;
    let test_point = Arc::new(TestPoint3::default());
    let w = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        dir.as_directory(),
        conf,
        Arc::clone(&test_point) as Arc<dyn TestPoint>,
    )?;
    test_point.do_fail.store(true, Ordering::SeqCst);
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a field",
        Store::YES,
    )));
    for _ in 0..10 {
        match catching(|| w.add_document(&doc)) {
            Ok(_) => {}
            Err(re) if re.is_instance(RUNTIME_EXCEPTION) => break,
            Err(t) => t.throw(),
        }
    }

    // OK: merge exc causes tragedy
    let _ = try_catch(ISE, || as_cms(&w.get_config().merge_scheduler()).sync());
    Assert::assert_true(test_point.failed.load(Ordering::SeqCst));
    w.close()?;
    dir.close()
}

/// The anonymous `TokenFilter` of `testExceptionFromTokenStream`.
#[derive(Debug, Default)]
struct FailOnSixthToken {
    count: i32,
}

impl TokenFilterLogic for FailOnSixthToken {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let count = self.count;
        self.count += 1;
        if count == 5 {
            return Err(io_exception_no_message());
        }
        input.increment_token()
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.count = 0;
        Ok(())
    }
}

/// The anonymous `Analyzer` of `testExceptionFromTokenStream`.
#[derive(Debug)]
struct TokenStreamFailureAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for TokenStreamFailureAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterExceptions$2"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut tokenizer = MockTokenizer::new(Arc::clone(&SIMPLE), true)?;
        tokenizer.set_enable_checks(false); // disable workflow checking as we forcefully close() in exceptional
                                            // cases.
        let tokenizer = Arc::new(AtomicRefCell::new(tokenizer));
        let stream = TokenFilter::new(shared(&tokenizer), FailOnSixthToken::default());
        Ok(components(tokenizer, sink(stream)))
    }
}

/// LUCENE-1072
/// Port of `TestIndexWriterExceptions.testExceptionFromTokenStream`.
fn test_exception_from_token_stream() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        TokenStreamFailureAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        },
    )));
    conf.set_max_buffered_docs(3.max(conf.max_buffered_docs()))?;
    conf.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;

    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut broken_doc = Document::new();
    let contents = "aa bb cc dd ee ff gg hh ii jj kk";
    broken_doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        contents,
        Store::NO,
    )));
    expect_throws("java.lang.Exception", || writer.add_document(&broken_doc));

    // Make sure we can add another normal document
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aa bb cc dd",
        Store::NO,
    )));
    writer.add_document(&doc)?;

    // Make sure we can add another normal document
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aa bb cc dd",
        Store::NO,
    )));
    writer.add_document(&doc)?;

    writer.close()?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let t = Term::from_text("content", "aa");
    assert_equals(&3, &reader.doc_freq(&t)?);

    // Make sure the doc that hit the exception was marked
    // as deleted:
    let mut tdocs = TestUtil::docs(
        &*random(),
        &reader,
        t.field(),
        &BytesRef::from_char_sequence(&t.text()),
        None,
        0,
    )?
    .unwrap_or_else(|| npe("tdocs"));

    let live_docs = MultiBits::get_live_docs(&reader)?;
    let mut count = 0;
    while tdocs.next_doc()? != NO_MORE_DOCS {
        if live_docs
            .as_ref()
            .is_none_or(|l| l.get(tdocs.doc_id() as usize))
        {
            count += 1;
        }
    }
    assert_equals(&2, &count);

    assert_equals(&reader.doc_freq(&Term::from_text("content", "gg"))?, &0);
    reader.close()?;
    dir.close()
}

// ---------------------------------------------------------------------------
// DocumentsWriter aborts and exceptions
// ---------------------------------------------------------------------------

/// `private static class FailOnlyOnFlush extends MockDirectoryWrapper.Failure`.
#[derive(Default)]
struct FailOnlyOnFlush {
    do_fail: AtomicBool,
    count: AtomicI32,
}

impl Failure for FailOnlyOnFlush {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail()
            && LuceneTestCase::call_stack_contains_any_of(&["flush"])
            && !LuceneTestCase::call_stack_contains_any_of(&["finish_document"])
            && self.count.fetch_add(1, Ordering::SeqCst) >= 30
        {
            self.clear_do_fail();
            return Err(io_exception("now failing during flush"));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// make sure an aborting exception closes the writer:
/// Port of `TestIndexWriterExceptions.testDocumentsWriterAbort`.
fn test_documents_writer_abort() -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let failure = Arc::new(FailOnlyOnFlush::default());
    failure.set_do_fail();
    dir.as_mock().expect("newMockDirectory").fail_on(failure);

    let conf = mock_iwc();
    conf.set_max_buffered_docs(2)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let mut doc = Document::new();
    let contents = "aa bb cc dd ee ff gg hh ii jj kk";
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        contents,
        Store::NO,
    )));
    let mut hit_error = false;
    writer.add_document(&doc)?;

    expect_throws(IOE, || writer.add_document(&doc));

    // only one flush should fail:
    Assert::assert_false(hit_error);
    hit_error = true;
    let _ = hit_error;
    Assert::assert_true(writer.is_deleter_closed()?);
    Assert::assert_true(writer.is_closed());
    Assert::assert_false(directory_reader::index_exists(&*dir.as_directory())?);

    dir.close()
}

/// `assertEquals(expected, ...)` checks over the reader of
/// `testDocumentsWriterExceptions` and `testDocumentsWriterExceptionThreads`:
/// every live document's stored fields and term vectors load; returns the
/// number of deleted documents.
fn load_live_docs(reader: &Arc<dyn IndexReader>, expect_deletions: bool) -> Result<i32> {
    let mut num_del = 0;
    let live_docs = MultiBits::get_live_docs(reader)?;
    if expect_deletions {
        assert_not_null(&live_docs);
    } else {
        assert_null(&live_docs.as_ref().map(|_| "liveDocs"));
    }
    let stored_fields = reader.stored_fields()?;
    let term_vectors = reader.term_vectors()?;
    for j in 0..reader.max_doc() {
        match &live_docs {
            Some(l) if !l.get(j as usize) => num_del += 1,
            _ => {
                stored_fields.document(j)?;
                term_vectors.get(j)?;
            }
        }
    }
    Ok(num_del)
}

/// Port of `TestIndexWriterExceptions.testDocumentsWriterExceptions`.
fn test_documents_writer_exceptions() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(CrashingAnalyzer::new(
        None,
        "org.apache.lucene.index.TestIndexWriterExceptions$3",
    ));

    for i in 0..2 {
        if verbose() {
            println!("TEST: cycle i={i}");
        }
        let mut dir = LuceneTestCase::new_directory();
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
        conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;

        // don't allow a sudden merge to clean up the deleted
        // doc below:
        let mp = writer.get_config().merge_policy();
        let lmp = mp
            .as_any()
            .and_then(|a| a.downcast_ref::<rucene::index::merge_policy::LogMergePolicy>())
            .unwrap_or_else(|| npe("LogMergePolicy"));
        lmp.set_merge_factor(lmp.get_merge_factor().max(5))?;

        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            "contents",
            "here are some contents",
            &CUSTOM5,
        )));
        writer.add_document(&doc)?;
        writer.add_document(&doc)?;
        doc.add(Box::new(LuceneTestCase::new_field(
            "crash",
            "this should crash after 4 terms",
            &CUSTOM5,
        )));
        doc.add(Box::new(LuceneTestCase::new_field(
            "other",
            "this will not get indexed",
            &CUSTOM5,
        )));
        let final_doc = &doc;
        let ioe = expect_throws(IOE, || writer.add_document(final_doc));
        if verbose() {
            println!("TEST: hit expected exception");
            println!("{ioe}");
        }

        if 0 == i {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "contents",
                "here are some contents",
                &CUSTOM5,
            )));
            writer.add_document(&doc)?;
            writer.add_document(&doc)?;
        }
        writer.close()?;

        if verbose() {
            println!("TEST: open reader");
        }
        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        if i == 0 {
            let expected = 5;
            assert_equals(
                &expected,
                &reader.doc_freq(&Term::from_text("contents", "here"))?,
            );
            assert_equals(&expected, &reader.max_doc());
            let num_del = load_live_docs(&reader, true)?;
            assert_equals(&1, &num_del);
        }
        reader.close()?;

        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
        conf.set_max_buffered_docs(10)?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            "contents",
            "here are some contents",
            &CUSTOM5,
        )));
        for _ in 0..17 {
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)?;
        writer.close()?;

        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let expected = 19 + (1 - i) * 2;
        assert_equals(
            &expected,
            &reader.doc_freq(&Term::from_text("contents", "here"))?,
        );
        assert_equals(&expected, &reader.max_doc());
        let num_del = 0;
        load_live_docs(&reader, false)?;
        reader.close()?;
        assert_equals(&0, &num_del);

        dir.close()?;
    }
    Ok(())
}

/// Port of `TestIndexWriterExceptions.testDocumentsWriterExceptionFailOneDoc`.
fn test_documents_writer_exception_fail_one_doc() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(CrashingAnalyzer::new(
        None,
        "org.apache.lucene.index.TestIndexWriterExceptions$4",
    ));
    for _ in 0..10 {
        let mut dir = LuceneTestCase::new_directory();
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
        conf.set_max_buffered_docs(-1)?;
        conf.set_ram_buffer_size_mb(if random().next_boolean() {
            0.00001
        } else {
            i32::MAX as f64
        })?;
        conf.set_merge_policy(Some(Arc::new(KeepFullyDeletedSegmentsMergePolicy::new(
            Arc::new(NoMergePolicy::new()),
        ))))?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        let body = (|| -> Result<()> {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_field(
                "contents",
                "here are some contents",
                &CUSTOM5,
            )));
            writer.add_document(&doc)?;
            doc.add(Box::new(LuceneTestCase::new_field(
                "crash",
                "this should crash after 4 terms",
                &CUSTOM5,
            )));
            doc.add(Box::new(LuceneTestCase::new_field(
                "other",
                "this will not get indexed",
                &CUSTOM5,
            )));
            expect_throws(IOE, || writer.add_document(&doc));
            writer.commit()?;
            let reader = directory_reader::open(dir.as_directory())?;
            assert_equals(&2, &reader.doc_freq(&Term::from_text("contents", "here"))?);
            assert_equals(&2, &reader.max_doc());
            assert_equals(&1, &reader.num_docs());
            reader.close()
        })();
        // try-with-resources: the writer, then the directory.
        let closed = writer.close();
        let dir_closed = dir.close();
        body?;
        closed?;
        dir_closed?;
    }
    Ok(())
}

/// Port of `TestIndexWriterExceptions.testDocumentsWriterExceptionThreads`.
fn test_documents_writer_exception_threads() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(CrashingAnalyzer::new(
        None,
        "org.apache.lucene.index.TestIndexWriterExceptions$6",
    ));

    const NUM_THREAD: i32 = 3;
    let num_iter = LuceneTestCase::at_least(10);

    for i in 0..2 {
        let mut dir = LuceneTestCase::new_directory();
        {
            let conf =
                LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
            conf.set_max_buffered_docs(i32::MAX)?;
            conf.set_ram_buffer_size_mb(-1.0)?; // we don't want to flush automatically
                                                // don't use a merge policy here they depend on the DWPThreadPool and its
                                                // max thread states etc.
                                                // we also need to keep fully deleted segments since otherwise we clean up
                                                // fully deleted ones and if we
                                                // flush the one that has only the failed document the docFreq checks will
                                                // be off below.
            conf.set_merge_policy(Some(Arc::new(KeepFullyDeletedSegmentsMergePolicy::new(
                Arc::new(NoMergePolicy::new()),
            ))))?;
            let writer = IndexWriter::new(dir.as_directory(), conf)?;

            let final_i = i;

            let mut threads = Vec::new();
            for _ in 0..NUM_THREAD {
                let writer = Arc::clone(&writer);
                let thread = Thread::unnamed(move || {
                    let outcome = catching(|| {
                        for _ in 0..num_iter {
                            let mut doc = Document::new();
                            doc.add(Box::new(LuceneTestCase::new_field(
                                "contents",
                                "here are some contents",
                                &CUSTOM5,
                            )));
                            writer.add_document(&doc)?;
                            writer.add_document(&doc)?;
                            doc.add(Box::new(LuceneTestCase::new_field(
                                "crash",
                                "this should crash after 4 terms",
                                &CUSTOM5,
                            )));
                            doc.add(Box::new(LuceneTestCase::new_field(
                                "other",
                                "this will not get indexed",
                                &CUSTOM5,
                            )));
                            expect_throws(IOE, || writer.add_document(&doc));

                            if 0 == final_i {
                                let mut extra_doc = Document::new();
                                extra_doc.add(Box::new(LuceneTestCase::new_field(
                                    "contents",
                                    "here are some contents",
                                    &CUSTOM5,
                                )));
                                writer.add_document(&extra_doc)?;
                                writer.add_document(&extra_doc)?;
                            }
                        }
                        Ok(())
                    });
                    if let Err(t) = outcome {
                        println!("{}: ERROR: hit unexpected exception", current_thread_name());
                        println!("{t}");
                        Throwable::with_message(ASSERTION_ERROR, None).throw()
                    }
                });
                thread.start();
                threads.push(thread);
            }

            for t in &threads {
                let _ = t.join();
            }

            writer.close()?;
        }

        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let mut expected = (3 + (1 - i) * 2) * NUM_THREAD * num_iter;
        assert_equals_msg(
            Some(&format!("i={i}")),
            &expected,
            &reader.doc_freq(&Term::from_text("contents", "here"))?,
        );
        assert_equals(&expected, &reader.max_doc());
        let num_del = load_live_docs(&reader, true)?;
        reader.close()?;

        assert_equals(&(NUM_THREAD * num_iter), &num_del);

        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
        conf.set_max_buffered_docs(10)?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            "contents",
            "here are some contents",
            &CUSTOM5,
        )));
        for _ in 0..17 {
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)?;
        writer.close()?;

        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        expected += 17 - NUM_THREAD * num_iter;
        assert_equals(
            &expected,
            &reader.doc_freq(&Term::from_text("contents", "here"))?,
        );
        assert_equals(&expected, &reader.max_doc());
        load_live_docs(&reader, false)?;
        reader.close()?;

        dir.close()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sync and commit failures
// ---------------------------------------------------------------------------

/// Throws IOException during MockDirectoryWrapper.sync
/// `private static class FailOnlyInSync extends MockDirectoryWrapper.Failure`.
#[derive(Default)]
struct FailOnlyInSync {
    do_fail: AtomicBool,
    did_fail: AtomicBool,
}

impl Failure for FailOnlyInSync {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail()
            && LuceneTestCase::call_stack_contains_method::<MockDirectoryWrapper>("sync")
        {
            self.did_fail.store(true, Ordering::SeqCst);
            if verbose() {
                println!("TEST: now throw exc:");
                println!("{}", std::backtrace::Backtrace::force_capture());
            }
            return Err(io_exception("now failing on purpose during sync"));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// LUCENE-1044: test exception during sync
/// Port of `TestIndexWriterExceptions.testExceptionDuringSync`.
fn test_exception_during_sync() -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let failure = Arc::new(FailOnlyInSync::default());
    dir.as_mock()
        .expect("newMockDirectory")
        .fail_on(Arc::clone(&failure) as Arc<dyn Failure>);

    let conf = mock_iwc();
    conf.set_max_buffered_docs(2)?;
    conf.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    conf.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(5)?,
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    failure.set_do_fail();

    for i in 0..23 {
        add_doc(&writer)?;
        if (i - 1) % 2 == 0 {
            expect_throws(IOE, || writer.commit());
        }
    }
    as_cms(&writer.get_config().merge_scheduler()).sync();
    Assert::assert_true(failure.did_fail.load(Ordering::SeqCst));
    failure.clear_do_fail();
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    assert_equals(&23, &reader.num_docs());
    reader.close()?;
    dir.close()
}

/// `FailOnlyInCommit.PREPARE_STAGE` (`prepareCommit`).
const PREPARE_STAGE: &str = "prepare_commit";
/// `FailOnlyInCommit.FINISH_STAGE` (`finishCommit`).
const FINISH_STAGE: &str = "finish_commit";

/// `private static class FailOnlyInCommit extends MockDirectoryWrapper.Failure`.
struct FailOnlyInCommit {
    do_fail: AtomicBool,
    fail_on_commit: AtomicBool,
    fail_on_delete_file: AtomicBool,
    fail_on_sync_metadata: AtomicBool,
    dont_fail_during_global_field_map: bool,
    dont_fail_during_sync_metadata: bool,
    stage: &'static str,
}

impl FailOnlyInCommit {
    fn new(
        dont_fail_during_global_field_map: bool,
        dont_fail_during_sync_metadata: bool,
        stage: &'static str,
    ) -> Self {
        FailOnlyInCommit {
            do_fail: AtomicBool::new(false),
            fail_on_commit: AtomicBool::new(false),
            fail_on_delete_file: AtomicBool::new(false),
            fail_on_sync_metadata: AtomicBool::new(false),
            dont_fail_during_global_field_map,
            dont_fail_during_sync_metadata,
            stage,
        }
    }
}

impl Failure for FailOnlyInCommit {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        let mut is_commit = LuceneTestCase::call_stack_contains_method::<SegmentInfos>(self.stage);
        let is_delete =
            LuceneTestCase::call_stack_contains_method::<MockDirectoryWrapper>("delete_file");
        let is_sync_metadata =
            LuceneTestCase::call_stack_contains_method::<MockDirectoryWrapper>("sync_metadata");
        let is_in_global_field_map =
            LuceneTestCase::call_stack_contains_method::<SegmentInfos>("write_global_field_map");
        if is_in_global_field_map && self.dont_fail_during_global_field_map {
            is_commit = false;
        }
        if is_sync_metadata && self.dont_fail_during_sync_metadata {
            is_commit = false;
        }
        if is_commit {
            if !is_delete {
                self.fail_on_commit.store(true, Ordering::SeqCst);
                self.fail_on_sync_metadata
                    .store(is_sync_metadata, Ordering::SeqCst);
                return Err(runtime_exception("now fail first"));
            } else {
                self.fail_on_delete_file.store(true, Ordering::SeqCst);
                return Err(io_exception("now fail during delete"));
            }
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriterExceptions.testExceptionsDuringCommit`.
fn test_exceptions_during_commit() -> Result<()> {
    let failures = vec![
        // LUCENE-1214
        Arc::new(FailOnlyInCommit::new(false, true, PREPARE_STAGE)), // fail during global field map is written
        Arc::new(FailOnlyInCommit::new(true, false, PREPARE_STAGE)), // fail during sync metadata
        Arc::new(FailOnlyInCommit::new(true, true, PREPARE_STAGE)), // fail after global field map is written
        Arc::new(FailOnlyInCommit::new(false, true, FINISH_STAGE)), // fail while running finishCommit
    ];

    for failure in failures {
        let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
        let mock = dir.as_mock().expect("newMockDirectory");
        mock.set_fail_on_create_output(false);
        let file_count = dir.as_directory().list_all()?.len();
        let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            "a field",
            Store::YES,
        )));
        w.add_document(&doc)?;
        dir.as_mock()
            .expect("newMockDirectory")
            .fail_on(Arc::clone(&failure) as Arc<dyn Failure>);
        expect_throws(RUNTIME_EXCEPTION, || w.close());
        let fail_on_commit = failure.fail_on_commit.load(Ordering::SeqCst);
        let fail_on_delete_file = failure.fail_on_delete_file.load(Ordering::SeqCst);
        let fail_on_sync_metadata = failure.fail_on_sync_metadata.load(Ordering::SeqCst);
        Assert::assert_true_msg(
            &format!(
                "failOnCommit={fail_on_commit} failOnDeleteFile={fail_on_delete_file} failOnSyncMetadata={fail_on_sync_metadata}"
            ),
            fail_on_commit && (fail_on_delete_file || fail_on_sync_metadata),
        );
        w.rollback()?;
        let files = dir.as_directory().list_all()?;
        Assert::assert_true(
            files.len() == file_count
                || (files.len() == file_count + 1 && files.iter().any(|f| f == WRITE_LOCK_NAME)),
        );
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestIndexWriterExceptions.testForceMergeExceptions`.
fn test_force_merge_exceptions() -> Result<()> {
    let mut start_dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    conf.set_max_buffered_docs(2)?;
    let lmp = LuceneTestCase::new_log_merge_policy();
    lmp.set_merge_factor(100)?;
    conf.set_merge_policy(Some(Arc::new(lmp)))?;
    let w = IndexWriter::new(start_dir.as_directory(), conf)?;
    for _ in 0..27 {
        add_doc(&w)?;
    }
    w.close()?;

    let iter = if *TEST_NIGHTLY { 200 } else { 10 };
    for i in 0..iter {
        if verbose() {
            println!("\nTEST: iter {i}");
        }
        let copy = TestUtil::ram_copy_of(&*start_dir.as_directory())?;
        let mut dir = TestDirectory::Mock(MockDirectoryWrapper::new(&*random(), Arc::from(copy)));
        let conf = mock_iwc();
        conf.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
        as_cms(&conf.merge_scheduler()).set_suppress_exceptions();
        let w = IndexWriter::new(dir.as_directory(), conf)?;
        let mock = dir.as_mock().expect("MockDirectoryWrapper");
        mock.set_random_io_exception_rate(0.5);
        match w.force_merge(1) {
            Ok(()) => {}
            Err(ise) if is_instance(&ise, ISE) => {
                // expected
            }
            Err(ioe) if ioe.is_io_exception() => {
                if java_throwable(&ioe).cause().is_none() {
                    Assert::fail("forceMerge threw IOException without root cause");
                }
            }
            Err(e) => return Err(e),
        }
        mock.set_random_io_exception_rate(0.0);
        // System.out.println("TEST: now close IW");
        match w.close() {
            Ok(()) => {}
            Err(ise) if is_instance(&ise, ISE) => {
                // ok
            }
            Err(e) => return Err(e),
        }
        dir.close()?;
    }
    start_dir.close()
}

// ---------------------------------------------------------------------------
// OutOfMemoryError (LUCENE-1429) and rollback hangs (LUCENE-1347)
// ---------------------------------------------------------------------------

/// The anonymous `InfoStream`s of the `OutOfMemoryError` tests: throws a fake
/// `OutOfMemoryError` the first time a message matches.
struct OomeInfoStream {
    thrown: Arc<AtomicBool>,
    matches: fn(&str) -> bool,
    class_name: &'static str,
}

impl InfoStream for OomeInfoStream {
    fn java_class_name(&self) -> &'static str {
        self.class_name
    }

    fn message(&self, _component: &str, message: &str) {
        if (self.matches)(message)
            && self
                .thrown
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            Throwable::new(OOME, format!("fake OOME at {message}")).throw()
        }
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}
}

/// LUCENE-1429
/// Port of `TestIndexWriterExceptions.testOutOfMemoryErrorCausesCloseToFail`.
fn test_out_of_memory_error_causes_close_to_fail() -> Result<()> {
    let thrown = Arc::new(AtomicBool::new(false));
    let mut dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    conf.set_info_stream(Arc::new(OomeInfoStream {
        thrown: Arc::clone(&thrown),
        matches: |m| m.starts_with("now flush at close"),
        class_name: "org.apache.lucene.index.TestIndexWriterExceptions$9",
    }))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    expect_throws(OOME, || writer.close());

    // throws IllegalStateEx w/o bug fix
    writer.close()?;
    dir.close()
}

/// If IW hits OOME during indexing, it should refuse to commit any further changes.
/// Port of `TestIndexWriterExceptions.testOutOfMemoryErrorRollback`.
fn test_out_of_memory_error_rollback() -> Result<()> {
    let thrown = Arc::new(AtomicBool::new(false));
    let mut dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    conf.set_info_stream(Arc::new(OomeInfoStream {
        thrown: Arc::clone(&thrown),
        matches: |m| m.contains("startFullFlush"),
        class_name: "org.apache.lucene.index.TestIndexWriterExceptions$10",
    }))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&Document::new())?;

    expect_throws(OOME, || writer.commit());

    match writer.close() {
        Ok(()) => {}
        Err(ok) if is_instance(&ok, IAE) => {
            // ok
        }
        Err(e) => return Err(e),
    }

    expect_throws(ACE, || writer.add_document(&Document::new()));

    // IW should have done rollback() during close, since it hit OOME, and so no index should exist:
    Assert::assert_false(directory_reader::index_exists(&*dir.as_directory())?);

    dir.close()
}

/// LUCENE-1347
/// `private static final class TestPoint4`.
#[derive(Default)]
struct TestPoint4 {
    do_fail: AtomicBool,
}

impl TestPoint for TestPoint4 {
    fn apply(&self, name: &str) {
        if self.do_fail.load(Ordering::SeqCst) && name == "rollback before checkpoint" {
            Throwable::new(RUNTIME_EXCEPTION, "intentionally failing").throw()
        }
    }
}

/// LUCENE-1347
/// Port of `TestIndexWriterExceptions.testRollbackExceptionHang`.
fn test_rollback_exception_hang() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let test_point = Arc::new(TestPoint4::default());
    let w = RandomIndexWriter::mock_index_writer_with_test_point(
        &*random(),
        dir.as_directory(),
        mock_iwc(),
        Arc::clone(&test_point) as Arc<dyn TestPoint>,
    )?;

    add_doc(&w)?;
    test_point.do_fail.store(true, Ordering::SeqCst);
    expect_throws(RUNTIME_EXCEPTION, || w.rollback());

    test_point.do_fail.store(false, Ordering::SeqCst);
    w.rollback()?;
    dir.close()
}

// ---------------------------------------------------------------------------
// Corrupt indexes
// ---------------------------------------------------------------------------

/// Writes 100 `addDoc` documents into `dir` and closes the writer; returns
/// the last commit generation, which must be positive.
fn write_hundred_docs(
    dir: &TestDirectory,
    conf: IndexWriterConfig,
) -> Result<(Arc<IndexWriter>, i64)> {
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // add 100 documents
    for _ in 0..100 {
        add_doc(&writer)?;
    }

    // close
    writer.close()?;

    let gen = SegmentInfos::get_last_commit_generation_dir(&*dir.as_directory())?;
    Assert::assert_true_msg(
        &format!("segment generation should be > 0 but got {gen}"),
        gen > 0,
    );
    Ok((writer, gen))
}

/// LUCENE-1044: Simulate checksum error in segments_N
/// Port of `TestIndexWriterExceptions.testSegmentsChecksumError`.
fn test_segments_checksum_error() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    crate::support::set_check_index_on_close(&dir, false); // we corrupt the index

    let (_writer, gen) = write_hundred_docs(&dir, mock_iwc())?;

    let d = dir.as_directory();
    let segments_file_name = SegmentInfos::get_last_commit_segments_file_name_dir(&*d)?
        .unwrap_or_else(|| npe("segmentsFileName"));
    let mut input = d.open_input(&segments_file_name, &*READONCE_IO_CONTEXT)?;
    let mut out = d.create_output(
        &file_name_from_generation(SEGMENTS, "", 1 + gen).unwrap_or_else(|| npe("fileName")),
        &*LuceneTestCase::new_io_context(&*random()),
    )?;
    let length = input.length();
    out.copy_bytes(&mut *input, length - 1)?;
    let b = input.read_byte()?;
    out.write_byte(1u8.wrapping_add(b))?;
    out.close()?;
    input.close()?;

    expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        directory_reader::open(Arc::clone(&d))
    });

    drop(d);
    dir.close()
}

/// Simulate a corrupt index by removing last byte of
/// latest segments file and make sure we get an
/// IOException trying to open the index:
/// Port of `TestIndexWriterExceptions.testSimulatedCorruptIndex1`.
fn test_simulated_corrupt_index1() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    crate::support::set_check_index_on_close(&dir, false); // we are corrupting it!

    let (_writer, gen) = write_hundred_docs(&dir, mock_iwc())?;

    let d = dir.as_directory();
    let file_name_in = SegmentInfos::get_last_commit_segments_file_name_dir(&*d)?
        .unwrap_or_else(|| npe("fileNameIn"));
    let file_name_out =
        file_name_from_generation(SEGMENTS, "", 1 + gen).unwrap_or_else(|| npe("fileNameOut"));
    let mut input = d.open_input(&file_name_in, &*READONCE_IO_CONTEXT)?;
    let mut out = d.create_output(&file_name_out, &*LuceneTestCase::new_io_context(&*random()))?;
    let length = input.length();
    for _ in 0..length - 1 {
        out.write_byte(input.read_byte()?)?;
    }
    input.close()?;
    out.close()?;
    d.delete_file(&file_name_in)?;

    expect_throws("java.lang.Exception", || {
        directory_reader::open(Arc::clone(&d))
    });

    drop(d);
    dir.close()
}

/// Simulate a corrupt index by removing one of the
/// files and make sure we get an IOException trying to
/// open the index:
/// Port of `TestIndexWriterExceptions.testSimulatedCorruptIndex2`.
fn test_simulated_corrupt_index2() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    crate::support::set_check_index_on_close(&dir, false); // we are corrupting it!

    let conf = mock_iwc();
    conf.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(true)))?;
    conf.set_use_compound_file(true)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let lmp = writer.get_config().merge_policy();
    // Force creation of CFS:
    lmp.set_no_cfs_ratio(1.0)?;
    lmp.set_max_cfs_segment_size_mb(f64::INFINITY)?;

    // add 100 documents
    for _ in 0..100 {
        add_doc(&writer)?;
    }

    // close
    writer.close()?;

    let gen = SegmentInfos::get_last_commit_generation_dir(&*dir.as_directory())?;
    Assert::assert_true_msg(
        &format!("segment generation should be > 0 but got {gen}"),
        gen > 0,
    );

    let d = dir.as_directory();
    let mut corrupted = false;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    if let Some(si) = sis.iter().next() {
        Assert::assert_true(si.info.get_use_compound_file());
        // `new ArrayList<String>(si.info.files())`: Java's `HashSet` order.
        let mut victims: Vec<String> = si.info.files_ordered()?.into_iter().collect();
        shuffle(&mut victims, &*random());
        d.delete_file(&victims[0])?;
        corrupted = true;
    }

    Assert::assert_true_msg("failed to find cfs file to remove: ", corrupted);

    expect_throws("java.lang.Exception", || {
        directory_reader::open(Arc::clone(&d))
    });

    drop(d);
    dir.close()
}

// ---------------------------------------------------------------------------
// Term vectors and non-aborting exceptions
// ---------------------------------------------------------------------------

/// `FailOnTermVectors.INIT_STAGE` (`initTermVectorsWriter`).
const INIT_STAGE: &str = "init_term_vectors_writer";
/// `FailOnTermVectors.AFTER_INIT_STAGE` (`finishDocument`).
const AFTER_INIT_STAGE: &str = "finish_document";
/// `FailOnTermVectors.EXC_MSG`.
const EXC_MSG: &str = "FOTV";

/// `private static class FailOnTermVectors extends MockDirectoryWrapper.Failure`.
struct FailOnTermVectors {
    do_fail: AtomicBool,
    stage: &'static str,
}

impl FailOnTermVectors {
    fn new(stage: &'static str) -> Self {
        FailOnTermVectors {
            do_fail: AtomicBool::new(false),
            stage,
        }
    }
}

impl Failure for FailOnTermVectors {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if LuceneTestCase::call_stack_contains_method::<TermVectorsConsumer>(self.stage) {
            return Err(runtime_exception(EXC_MSG));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriterExceptions.testTermVectorExceptions`.
fn test_term_vector_exceptions() -> Result<()> {
    let failures = [
        Arc::new(FailOnTermVectors::new(AFTER_INIT_STAGE)),
        Arc::new(FailOnTermVectors::new(INIT_STAGE)),
    ];
    let num = LuceneTestCase::at_least(1);
    'iters: for _ in 0..num {
        for failure in &failures {
            let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
            let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
            dir.as_mock()
                .expect("newMockDirectory")
                .fail_on(Arc::clone(failure) as Arc<dyn Failure>);
            let num_docs = 10 + random().next_int_bound(30);
            for _ in 0..num_docs {
                let mut doc = Document::new();
                // random TV
                let field =
                    LuceneTestCase::new_text_field_with(&*random(), "field", "a field", Store::YES);
                let store_term_vectors = field.field_type().store_term_vectors();
                doc.add(Box::new(field));
                match catching(|| w.add_document(&doc)) {
                    Ok(_) => Assert::assert_false(store_term_vectors),
                    Err(e) if e.is_instance(RUNTIME_EXCEPTION) => {
                        Assert::assert_true(
                            e.message()
                                .unwrap_or_else(|| npe("message"))
                                .starts_with(EXC_MSG),
                        );
                        // This is an aborting exception, so writer is closed:
                        Assert::assert_true(w.is_deleter_closed()?);
                        Assert::assert_true(w.is_closed());
                        dir.close()?;
                        continue 'iters;
                    }
                    Err(t) => t.throw(),
                }
                if random().next_int_bound(20) == 0 {
                    w.commit()?;
                    check_index(&dir)?;
                }
            }
            let mut document = Document::new();
            document.add(Box::new(TextField::new(
                "field",
                "a field".to_string(),
                Store::YES,
            )?));
            w.add_document(&document)?;

            for _ in 0..num_docs {
                let mut doc = Document::new();
                let field =
                    LuceneTestCase::new_text_field_with(&*random(), "field", "a field", Store::YES);
                let store_term_vectors = field.field_type().store_term_vectors();
                doc.add(Box::new(field));
                // random TV
                match catching(|| w.add_document(&doc)) {
                    Ok(_) => Assert::assert_false(store_term_vectors),
                    Err(e) if e.is_instance(RUNTIME_EXCEPTION) => {
                        Assert::assert_true(
                            e.message()
                                .unwrap_or_else(|| npe("message"))
                                .starts_with(EXC_MSG),
                        );
                    }
                    Err(t) => t.throw(),
                }
                if random().next_int_bound(20) == 0 {
                    w.commit()?;
                    check_index(&dir)?;
                }
            }
            let mut document = Document::new();
            document.add(Box::new(TextField::new(
                "field",
                "a field".to_string(),
                Store::YES,
            )?));
            w.add_document(&document)?;
            w.close()?;
            let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
            Assert::assert_true(reader.num_docs() > 0);
            SegmentInfos::read_latest_commit(dir.as_directory())?;
            for context in Arc::clone(&reader).leaves()? {
                Assert::assert_false(context.leaf_reader().get_field_infos().has_term_vectors());
            }
            reader.close()?;
            dir.close()?;
        }
    }
    Ok(())
}

/// `new Field("crash", new CrashingFilter("crash", tokenizer),
/// TextField.TYPE_NOT_STORED)` over a `MockTokenizer(WHITESPACE, false)`
/// reading "crash me on the 4th token", without checks.
fn crash_field() -> Result<Field> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new("crash me on the 4th token")))?;
    tokenizer.set_enable_checks(false); // disable workflow checking as we forcefully close() in exceptional cases.
    let stream: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(
        crashing_filter("crash", Box::new(tokenizer)),
    ));
    Field::new_with_token_stream(
        "crash",
        stream,
        FieldType::new_from(TextField::type_not_stored()),
    )
}

/// `doc.add(newTextField("content", "good content", Field.Store.NO));
/// w.addDocument(doc);`
fn add_good_doc(w: &RandomIndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "good content",
        Store::NO,
    )));
    w.add_document(&doc)?;
    Ok(())
}

/// Port of `TestIndexWriterExceptions.testAddDocsNonAbortingException`.
fn test_add_docs_non_aborting_exception() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs1 = random().next_int_bound(25);
    for _ in 0..num_docs1 {
        add_good_doc(&w)?;
    }

    let mut docs = Vec::new();
    for doc_count in 0..7 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &format!("{doc_count}"),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("silly content {doc_count}"),
            Store::NO,
        )));
        if doc_count == 4 {
            doc.add(Box::new(crash_field()?));
        }
        docs.push(doc);
    }

    let expected = expect_throws(IOE, || w.add_documents(&docs));
    assert_equals(&Some(CRASH_FAIL_MESSAGE), &expected.message());

    let num_docs2 = random().next_int_bound(25);
    for _ in 0..num_docs2 {
        add_good_doc(&w)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let pq = PhraseQuery::from_strings("content", &["silly", "good"])?;
    assert_equals(&0, &s.count(Arc::new(pq) as Arc<dyn Query>)?);

    let pq = PhraseQuery::from_strings("content", &["good", "content"])?;
    assert_equals(
        &(num_docs1 + num_docs2),
        &s.count(Arc::new(pq) as Arc<dyn Query>)?,
    );
    r.close()?;
    dir.close()
}

/// Port of `TestIndexWriterExceptions.testUpdateDocsNonAbortingException`.
fn test_update_docs_non_aborting_exception() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs1 = random().next_int_bound(25);
    for _ in 0..num_docs1 {
        add_good_doc(&w)?;
    }

    // Use addDocs (no exception) to get docs in the index:
    let mut docs = Vec::new();
    let num_docs2 = random().next_int_bound(25);
    for doc_count in 0..num_docs2 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "subid",
            "subs",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &format!("{doc_count}"),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("silly content {doc_count}"),
            Store::NO,
        )));
        docs.push(doc);
    }
    w.add_documents(&docs)?;

    let num_docs3 = random().next_int_bound(25);
    for _ in 0..num_docs3 {
        add_good_doc(&w)?;
    }

    docs.clear();
    let limit = TestUtil::next_int(&*random(), 2, 25);
    let crash_at = random().next_int_bound(limit);
    for doc_count in 0..limit {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &format!("{doc_count}"),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("silly content {doc_count}"),
            Store::NO,
        )));
        if doc_count == crash_at {
            doc.add(Box::new(crash_field()?));
        }
        docs.push(doc);
    }

    let expected = expect_throws(IOE, || {
        w.update_documents(Term::from_text("subid", "subs"), &docs)
    });
    assert_equals(&Some(CRASH_FAIL_MESSAGE), &expected.message());

    let num_docs4 = random().next_int_bound(25);
    for _ in 0..num_docs4 {
        add_good_doc(&w)?;
    }

    let r = w.get_reader()?;
    w.close()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let pq = PhraseQuery::from_strings("content", &["silly", "content"])?;
    assert_equals(&num_docs2, &s.count(Arc::new(pq) as Arc<dyn Query>)?);

    let pq = PhraseQuery::from_strings("content", &["good", "content"])?;
    assert_equals(
        &(num_docs1 + num_docs3 + num_docs4),
        &s.count(Arc::new(pq) as Arc<dyn Query>)?,
    );
    r.close()?;
    dir.close()
}

// ---------------------------------------------------------------------------
// Null stored values
// ---------------------------------------------------------------------------

/// `new IndexWriter(dir, new IndexWriterConfig(new MockAnalyzer(random())))`.
fn plain_mock_writer(dir: &TestDirectory) -> Result<Arc<IndexWriter>> {
    let analyzer = mock_analyzer();
    IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Some(analyzer)),
    )
}

/// The tail every null-stored-value test shares: no tragedy, close, and the
/// good document is the only one.
fn assert_only_good_doc(iw: &Arc<IndexWriter>, mut dir: TestDirectory) -> Result<()> {
    assert_null(&iw.get_tragic_exception().map(|e| e.to_string()));
    iw.close()?;
    // make sure we see our good doc
    let r = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &r.num_docs());
    r.close()?;
    dir.close()
}

/// test a null string value doesn't abort the entire segment
/// Port of `TestIndexWriterExceptions.testNullStoredField`.
fn test_null_stored_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_mock_writer(&dir)?;
    // add good document
    let mut doc = Document::new();
    iw.add_document(&doc)?;
    expect_throws(IAE, || {
        // set to null value
        let value: Option<String> = None;
        doc.add(Box::new(StoredField::new_string("foo", value)?));
        iw.add_document(&doc)
    });

    assert_only_good_doc(&iw, dir)
}

/// test a null string value doesn't abort the entire segment
/// Port of `TestIndexWriterExceptions.testNullStoredFieldReuse`.
fn test_null_stored_field_reuse() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_mock_writer(&dir)?;
    // add good document
    let mut doc = Document::new();
    let the_field = Field::new("foo", "hello".to_string(), StoredField::type_().clone())?;
    doc.add(Box::new(the_field));
    iw.add_document(&doc)?;
    expect_throws(IAE, || {
        // set to null value
        crate::support::field_mut(&mut doc, "foo").set_string_value(None::<String>)?;
        iw.add_document(&doc)
    });

    assert_only_good_doc(&iw, dir)
}

/// test a null byte[] value doesn't abort the entire segment
/// Port of `TestIndexWriterExceptions.testNullStoredBytesField`.
fn test_null_stored_bytes_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_mock_writer(&dir)?;
    // add good document
    let mut doc = Document::new();
    iw.add_document(&doc)?;

    expect_throws("java.lang.NullPointerException", || {
        // set to null value
        let v: Option<&[u8]> = None;
        let the_field = StoredField::new_byte_array("foo", v)?;
        doc.add(Box::new(the_field));
        iw.add_document(&doc)
    });

    assert_only_good_doc(&iw, dir)
}

/// `new StoredField(name, bytes)` as the `Field` a test keeps: a field of
/// `StoredField.TYPE` over the bytes (`Field(name, value, TYPE)`).
fn stored_bytes_field(name: &str, bytes: BytesRef) -> Result<Field> {
    Field::new_with_bytes(name, bytes, StoredField::type_().clone())
}

/// A copy of a binary field, for the document added again.
fn copy_bytes_field(field: &Field) -> Result<Field> {
    Field::new_with_bytes(
        field.name(),
        field.binary_value().unwrap_or_else(|| npe("binaryValue")),
        FieldType::new_from(field.field_type()),
    )
}

/// test a null byte[] value doesn't abort the entire segment
/// Port of `TestIndexWriterExceptions.testNullStoredBytesFieldReuse`.
fn test_null_stored_bytes_field_reuse() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_mock_writer(&dir)?;
    // add good document
    let the_field = stored_bytes_field("foo", BytesRef::from_char_sequence("hello"))?;
    let mut doc = Document::new();
    doc.add(Box::new(copy_bytes_field(&the_field)?));
    iw.add_document(&doc)?;
    expect_throws("java.lang.NullPointerException", || {
        // set to null value
        let v: Option<&[u8]> = None;
        the_field.set_bytes_value_array(v)?;
        let mut doc = Document::new();
        doc.add(Box::new(copy_bytes_field(&the_field)?));
        iw.add_document(&doc)
    });

    assert_only_good_doc(&iw, dir)
}

/// test a null bytesref value doesn't abort the entire segment
/// Port of `TestIndexWriterExceptions.testNullStoredBytesRefField`.
fn test_null_stored_bytes_ref_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_mock_writer(&dir)?;
    // add good document
    let mut doc = Document::new();
    iw.add_document(&doc)?;

    expect_throws(IAE, || {
        // set to null value
        let v: Option<BytesRef> = None;
        let the_field = StoredField::new_bytes("foo", v)?;
        doc.add(Box::new(the_field));
        iw.add_document(&doc)
    });

    assert_only_good_doc(&iw, dir)
}

/// test a null bytesref value doesn't abort the entire segment
/// Port of `TestIndexWriterExceptions.testNullStoredBytesRefFieldReuse`.
fn test_null_stored_bytes_ref_field_reuse() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_mock_writer(&dir)?;
    // add good document
    let the_field = stored_bytes_field("foo", BytesRef::from_char_sequence("hello"))?;
    let mut doc = Document::new();
    doc.add(Box::new(copy_bytes_field(&the_field)?));
    iw.add_document(&doc)?;
    expect_throws(IAE, || {
        // set to null value
        let v: Option<BytesRef> = None;
        the_field.set_bytes_value(v)?;
        let mut doc = Document::new();
        doc.add(Box::new(copy_bytes_field(&the_field)?));
        iw.add_document(&doc)
    });

    assert_only_good_doc(&iw, dir)
}

/// test a null data input value doesn't abort the entire segment
/// Port of `TestIndexWriterExceptions.testNullStoredDataInputField`.
fn test_null_stored_data_input_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_mock_writer(&dir)?;
    // add good document
    let mut doc = Document::new();
    iw.add_document(&doc)?;

    expect_throws(IAE, || {
        // set to null value
        let v: Option<Box<dyn rucene::store::DataInput + Send + Sync>> = None;
        let the_field = StoredField::new_stored_input("foo", v, 0)?;
        doc.add(Box::new(the_field));
        iw.add_document(&doc)
    });

    assert_only_good_doc(&iw, dir)
}

/// The anonymous `Analyzer` of `testCrazyPositionIncrementGap`.
#[derive(Debug)]
struct CrazyGapAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for CrazyGapAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterExceptions$11"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&KEYWORD),
            false,
        )?));
        Ok(tokenizer_components(tokenizer))
    }

    fn get_position_increment_gap(&self, _field_name: &str) -> i32 {
        -2
    }
}

/// Port of `TestIndexWriterExceptions.testCrazyPositionIncrementGap`.
fn test_crazy_position_increment_gap() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(CrazyGapAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let iw = IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Some(analyzer)),
    )?;
    // add good document
    let mut doc = Document::new();
    iw.add_document(&doc)?;
    expect_throws(IAE, || {
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "foo",
            "bar",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "foo",
            "bar",
            Store::NO,
        )));
        iw.add_document(&doc)
    });

    assert_null(&iw.get_tragic_exception().map(|e| e.to_string()));
    iw.close()?;

    // make sure we see our good doc
    let r = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &r.num_docs());
    r.close()?;
    dir.close()
}

// TODO: we could also check isValid, to catch "broken" bytesref values, might be too much?

/// `static class UOEDirectory extends FilterDirectory`.
struct UoeDirectory {
    filter: Mutex<FilterDirectory>,
    do_fail: AtomicBool,
}

impl UoeDirectory {
    /// `protected UOEDirectory()`: `super(new ByteBuffersDirectory())`.
    fn new() -> Self {
        UoeDirectory {
            filter: Mutex::new(FilterDirectory::new(Box::new(ByteBuffersDirectory::new()))),
            do_fail: AtomicBool::new(false),
        }
    }

    fn filter(&self) -> std::sync::MutexGuard<'_, FilterDirectory> {
        self.filter.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Directory for UoeDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.filter().list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.filter().delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.filter().file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.filter().create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.filter().create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.filter().sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.filter().sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.filter().rename(source, dest)
    }
    /// `openInput(name, context)` (`@Override`): fails with
    /// `UnsupportedOperationException("expected UOE")` for a `segments_*`
    /// file read by `readCommit` or `readLatestCommit` while `doFail`.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        if self.do_fail.load(Ordering::SeqCst)
            && name.starts_with("segments_")
            && LuceneTestCase::call_stack_contains_any_of(&["read_commit", "read_latest_commit"])
        {
            return Err(LuceneError::UnsupportedOperation(
                "expected UOE".to_string(),
            ));
        }
        self.filter().open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        // `Directory.openChecksumInput` opens through `openInput`.
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*READONCE_IO_CONTEXT)?,
        )))
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.filter().obtain_lock(name)
    }
    fn close(&mut self) -> Result<()> {
        self.filter().close()
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.filter().get_pending_deletions()
    }
    fn directory_to_string(&self) -> String {
        format!("UOEDirectory({})", self.filter().directory_to_string())
    }
}

/// Port of `TestIndexWriterExceptions.testExceptionOnCtor`.
fn test_exception_on_ctor() -> Result<()> {
    let uoe = Arc::new(UoeDirectory::new());
    let mut d = TestDirectory::Mock(MockDirectoryWrapper::new(
        &*random(),
        Arc::clone(&uoe) as Arc<dyn Directory>,
    ));
    let iw = IndexWriter::new(d.as_directory(), null_analyzer_iwc())?;
    iw.add_document(&Document::new())?;
    iw.close()?;
    uoe.do_fail.store(true, Ordering::SeqCst);
    expect_throws("java.lang.UnsupportedOperationException", || {
        IndexWriter::new(d.as_directory(), null_analyzer_iwc())
    });

    uoe.do_fail.store(false, Ordering::SeqCst);
    d.close()
}

/// The anonymous `MockDirectoryWrapper.Failure` of
/// `testTooManyFileException`: throws "Too many open files" randomly.
#[derive(Default)]
struct TooManyOpenFilesFailure {
    do_fail: AtomicBool,
}

impl Failure for TooManyOpenFilesFailure {
    fn reset(&self) {
        self.clear_do_fail();
    }

    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail() && random().next_boolean() {
            return Err(LuceneError::NotFound(
                "some/file/name.ext (Too many open files)".to_string(),
            ));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// See LUCENE-4870 TooManyOpenFiles errors are thrown as
/// FNFExceptions which can trigger data loss.
/// Port of `TestIndexWriterExceptions.testTooManyFileException`.
fn test_too_many_file_exception() -> Result<()> {
    // Create failure that throws Too many open files exception randomly
    let failure = Arc::new(TooManyOpenFilesFailure::default());

    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let mock = dir.as_mock().expect("newMockDirectory");
    // The exception is only thrown on open input
    mock.set_fail_on_open_input(true);
    mock.fail_on(Arc::clone(&failure) as Arc<dyn Failure>);

    // Create an index with one document
    let iwc = IndexWriterConfig::with_analyzer(Some(mock_analyzer()));
    let mut iw = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "bar".to_string(),
        Store::NO,
    )?));
    iw.add_document(&doc)?; // add a document
    iw.commit()?;
    let ir = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &ir.num_docs());
    ir.close()?;
    iw.close()?;

    // Open and close the index a few times
    for i in 0..10 {
        failure.set_do_fail();
        let iwc = IndexWriterConfig::with_analyzer(Some(mock_analyzer()));
        match catching(|| IndexWriter::new(dir.as_directory(), iwc)) {
            Ok(opened) => iw = opened,
            Err(ex) if ex.is_instance(ASSERTION_ERROR) => {
                // This is fine: we tripped IW's assert that all files it's about to fsync do exist:
                Assert::assert_true(regex_matches_file_does_not_exist(
                    ex.message().unwrap_or_else(|| npe("message")),
                ));
            }
            Err(ex) if ex.is_instance("org.apache.lucene.index.CorruptIndexException") => {
                // Exceptions are fine - we are running out of file handlers here
                continue;
            }
            Err(ex)
                if ex.is_instance("java.io.FileNotFoundException")
                    || ex.is_instance("java.nio.file.NoSuchFileException") =>
            {
                continue;
            }
            Err(t) => t.throw(),
        }
        failure.clear_do_fail();
        iw.close()?;
        let ir = directory_reader::open(dir.as_directory())?;
        assert_equals_msg(
            Some(&format!("lost document after iteration: {i}")),
            &1,
            &ir.num_docs(),
        );
        ir.close()?;
    }

    // Check if document is still there
    failure.clear_do_fail();
    let ir = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &ir.num_docs());
    ir.close()?;

    dir.close()
}

/// `message.matches("file .* does not exist; files=\\[.*\\]")`: the whole
/// message, `.` not matching line terminators.
fn regex_matches_file_does_not_exist(message: &str) -> bool {
    if message.contains('\n') || message.contains('\r') {
        return false;
    }
    let Some(rest) = message.strip_prefix("file ") else {
        return false;
    };
    let Some(pos) = rest.rfind(" does not exist; files=[") else {
        return false;
    };
    let tail = &rest[pos + " does not exist; files=[".len()..];
    tail.ends_with(']')
}

/// The anonymous `TokenStream` of `testTooManyTokens`.
#[derive(Debug)]
struct TooManyTokensStream {
    source: AttributeSource,
    term_att: Att,
    pos_inc_att: Att,
    num: i64,
}

impl TokenStream for TooManyTokensStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.num == i32::MAX as i64 + 1 {
            return Ok(false);
        }
        self.source.clear_attributes();
        if self.num == 0 {
            self.pos_inc_att.pos_inc_mut().set_position_increment(1);
        } else {
            self.pos_inc_att.pos_inc_mut().set_position_increment(0);
        }
        self.term_att.char_term_mut().append_string(Some("a"));
        self.num += 1;
        if verbose() && self.num % 1000000 == 0 {
            println!("indexed: {}", self.num);
        }
        Ok(true)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// kind of slow, but omits positions, so just CPU
/// Port of `TestIndexWriterExceptions.testTooManyTokens`.
fn test_too_many_tokens() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(dir.as_directory(), null_analyzer_iwc())?;
    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
    let term_att = Att::add(&mut source, char_term_id())?;
    let pos_inc_att = Att::add(&mut source, pos_inc_id())?;
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(TooManyTokensStream {
            source,
            term_att,
            pos_inc_att,
            num: 0,
        }));
    doc.add(Box::new(Field::new_with_token_stream("foo", stream, ft)?));

    let expected = expect_throws(IAE, || iw.add_document(&doc));
    Assert::assert_true(
        expected
            .message()
            .unwrap_or_else(|| npe("message"))
            .contains("too many tokens"),
    );

    iw.close()?;
    dir.close()
}

// ---------------------------------------------------------------------------
// Rollback failures
// ---------------------------------------------------------------------------

/// The infostream of `testExceptionDuringRollback` that throws exception
/// during rollback.
struct EvilInfoStream {
    message_to_fail_on: &'static str,
}

impl InfoStream for EvilInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterExceptions$14"
    }

    fn message(&self, _component: &str, message: &str) {
        if self.message_to_fail_on == message {
            Throwable::new(RUNTIME_EXCEPTION, "BOOM!").throw()
        }
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}
}

/// Port of `TestIndexWriterExceptions.testExceptionDuringRollback`.
fn test_exception_during_rollback() -> Result<()> {
    // currently: fail in two different places
    let message_to_fail_on = if random().next_boolean() {
        "rollback: done finish merges"
    } else {
        "rollback before checkpoint"
    };

    // infostream that throws exception during rollback
    let evil_info_stream = Arc::new(EvilInfoStream { message_to_fail_on });

    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory()); // we want to ensure we don't leak any locks or file handles
    let iwc = plain_null_analyzer_iwc();
    iwc.set_info_stream(evil_info_stream)?;
    // TODO: cutover to RandomIndexWriter.mockIndexWriter?
    let iw = IndexWriter::new_with_test_points(dir.as_directory(), iwc)?;

    let doc = Document::new();
    for _ in 0..10 {
        iw.add_document(&doc)?;
    }
    iw.commit()?;

    iw.add_document(&doc)?;

    // pool readers
    let r = directory_reader::open_with_writer(&*iw)?;

    // sometimes sneak in a pending commit: we don't want to leak a file handle to that segments_N
    if random().next_boolean() {
        iw.prepare_commit()?;
    }

    let expected = expect_throws(RUNTIME_EXCEPTION, || iw.rollback());
    assert_equals(&Some("BOOM!"), &expected.message());

    r.close()?;

    // even though we hit exception: we are closed, no locks or files held, index in good state
    Assert::assert_true(iw.is_closed());
    dir.as_directory().obtain_lock(WRITE_LOCK_NAME)?.close()?;

    let r = directory_reader::open(dir.as_directory())?;
    assert_equals(&10, &r.max_doc());
    r.close()?;

    // no leaks
    dir.close()
}

/// The anonymous `MockDirectoryWrapper.Failure` of
/// `testRandomExceptionDuringRollback`: fail in random places on i/o.
#[derive(Default)]
struct RollbackFailure {
    do_fail: AtomicBool,
}

impl Failure for RollbackFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if random().next_int_bound(10) != 0 {
            return Ok(());
        }
        if LuceneTestCase::call_stack_contains_any_of(&["rollback_internal"]) {
            if verbose() {
                println!("TEST: now fail; thread={} exc:", current_thread_name());
                println!("{}", std::backtrace::Backtrace::force_capture());
            }
            return Err(fake_io_exception());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriterExceptions.testRandomExceptionDuringRollback`.
fn test_random_exception_during_rollback() -> Result<()> {
    // fail in random places on i/o
    let num_iters = *RANDOM_MULTIPLIER * 75;
    for _ in 0..num_iters {
        let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
        dir.as_mock()
            .expect("newMockDirectory")
            .fail_on(Arc::new(RollbackFailure::default()));

        let iwc = plain_null_analyzer_iwc();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let doc = Document::new();
        for _ in 0..10 {
            iw.add_document(&doc)?;
        }
        iw.commit()?;

        iw.add_document(&doc)?;

        // pool readers
        let r = directory_reader::open_with_writer(&*iw)?;

        // sometimes sneak in a pending commit: we don't want to leak a file handle to that segments_N
        if random().next_boolean() {
            iw.prepare_commit()?;
        }

        match iw.rollback() {
            Ok(()) => {}
            Err(expected) if is_instance(&expected, FAKE_IO_EXCEPTION) => {
                // ok, we randomly hit exc here
            }
            Err(e) => return Err(e),
        }

        r.close()?;

        // even though we hit exception: we are closed, no locks or files held, index in good state
        Assert::assert_true(iw.is_closed());
        dir.as_directory().obtain_lock(WRITE_LOCK_NAME)?.close()?;

        let r = directory_reader::open(dir.as_directory())?;
        assert_equals(&10, &r.max_doc());
        r.close()?;

        // no leaks
        dir.close()?;
    }
    Ok(())
}

/// The anonymous `MockDirectoryWrapper.Failure` of
/// `testMergeExceptionIsTragic`.
struct MergeFailure {
    do_fail: AtomicBool,
    did_fail: Arc<AtomicBool>,
}

impl Failure for MergeFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if random().next_int_bound(10) != 0 {
            return Ok(());
        }
        if self.did_fail.load(Ordering::SeqCst) {
            // Already failed
            return Ok(());
        }

        if LuceneTestCase::call_stack_contains_any_of(&["merge"]) {
            if verbose() {
                println!("TEST: now fail; thread={} exc:", current_thread_name());
                println!("{}", std::backtrace::Backtrace::force_capture());
            }
            self.did_fail.store(true, Ordering::SeqCst);
            return Err(fake_io_exception());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// TODO: can be super slow in pathological cases (merge config?)
/// Port of `TestIndexWriterExceptions.testMergeExceptionIsTragic`.
fn test_merge_exception_is_tragic() -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let did_fail = Arc::new(AtomicBool::new(false));
    dir.as_mock()
        .expect("newMockDirectory")
        .fail_on(Arc::new(MergeFailure {
            do_fail: AtomicBool::new(false),
            did_fail: Arc::clone(&did_fail),
        }));

    let iwc = LuceneTestCase::new_index_writer_config();
    let mp = iwc.merge_policy();
    if let Some(tmp) = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<TieredMergePolicy>())
    {
        if tmp.get_max_merged_segment_mb() < 0.2 {
            tmp.set_max_merged_segment_mb(0.2)?;
        }
    }
    let ms = iwc.merge_scheduler();
    let is_cms = ms
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .is_some();
    if is_cms {
        as_cms(&ms).set_suppress_exceptions();
    }
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    loop {
        let attempt = (|| -> Result<()> {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "field",
                "string",
                Store::NO,
            )));
            w.add_document(&doc)?;
            if random().next_int_bound(10) == 7 {
                // Flush new segment:
                directory_reader::open_with_writer(&*w)?.close()?;
            }
            Ok(())
        })();
        match attempt {
            Ok(()) => {}
            Err(ace) if is_instance(&ace, ACE) => {
                // OK: e.g. CMS hit the exc in BG thread and closed the writer
                break;
            }
            Err(fioe) if is_instance(&fioe, FAKE_IO_EXCEPTION) => {
                // OK: e.g. SMS hit the exception
                break;
            }
            Err(ise) if is_instance(&ise, ISE) => {
                // OK: Merge-on-refresh refuses to run because IndexWriter hit a tragedy
                break;
            }
            Err(e) => return Err(e),
        }
    }

    Assert::assert_true(w.get_tragic_exception().is_some());
    Assert::assert_false(w.is_open());
    Assert::assert_true(did_fail.load(Ordering::SeqCst));

    if is_cms {
        // Sneaky: CMS's merge thread will be concurrently rolling back IW due
        // to the tragedy, with this main thread, so we have to wait here
        // to ensure the rollback has finished, else MDW still sees open files:
        as_cms(&ms).sync();
    }

    dir.close()
}

/// The anonymous `InfoStream` of `testOnlyRollbackOnceOnException`.
struct RollbackOnceInfoStream {
    once: AtomicBool,
}

impl InfoStream for RollbackOnceInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterExceptions$18"
    }

    fn message(&self, component: &str, message: &str) {
        if "TP" == component && "rollback before checkpoint" == message {
            if self
                .once
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                Throwable::new(RUNTIME_EXCEPTION, "boom").throw()
            } else {
                Throwable::new(ASSERTION_ERROR, "has been rolled back twice").throw()
            }
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        "TP" == component
    }

    fn close(&self) {}
}

/// Port of `TestIndexWriterExceptions.testOnlyRollbackOnceOnException`.
fn test_only_rollback_once_on_exception() -> Result<()> {
    let stream = Arc::new(RollbackOnceInfoStream {
        once: AtomicBool::new(false),
    });
    let mut dir = LuceneTestCase::new_directory();
    let body = (|| -> Result<()> {
        let conf = LuceneTestCase::new_index_writer_config();
        conf.set_info_stream(stream)?;
        let writer = IndexWriter::new_with_test_points(dir.as_directory(), conf)?;
        let inner = (|| -> Result<()> {
            let e = expect_throws(RUNTIME_EXCEPTION, || writer.rollback());
            assert_equals(&Some("boom"), &e.message());
            let suppressed: Vec<String> = e.suppressed().iter().map(|s| s.to_string()).collect();
            assert_equals_msg(
                Some(&format!(
                    "has suppressed exceptions: {}",
                    crate::core_support::arrays_to_string(&suppressed)
                )),
                &0,
                &e.suppressed().len(),
            );
            Assert::assert_true(e.cause().is_none());
            Ok(())
        })();
        // try-with-resources: `writer.close()`.
        let closed = writer.close();
        inner?;
        closed
    })();
    let dir_closed = dir.close();
    body?;
    dir_closed
}

/// The anonymous `MockDirectoryWrapper.Failure` of
/// `testExceptionOnSyncMetadata`.
struct SyncMetadataFailure {
    do_fail: AtomicBool,
    maybe_fail_delete: Arc<AtomicBool>,
}

impl SyncMetadataFailure {
    /// `failDelete = () -> random().nextBoolean() && maybeFailDelete.get()`.
    fn fail_delete(&self) -> bool {
        random().next_boolean() && self.maybe_fail_delete.load(Ordering::SeqCst)
    }
}

impl Failure for SyncMetadataFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if LuceneTestCase::call_stack_contains_method::<MockDirectoryWrapper>("sync_metadata")
            && LuceneTestCase::call_stack_contains_method::<SegmentInfos>("finish_commit")
        {
            Throwable::new(RUNTIME_EXCEPTION, "boom").throw()
        } else if self.fail_delete()
            && LuceneTestCase::call_stack_contains_method::<IndexWriter>(
                "rollback_internal_no_commit",
            )
            && LuceneTestCase::call_stack_contains_method::<IndexFileDeleter>("delete_files")
        {
            Throwable::new(RUNTIME_EXCEPTION, "bang").throw()
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriterExceptions.testExceptionOnSyncMetadata`.
fn test_exception_on_sync_metadata() -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let body = (|| -> Result<()> {
        let conf = LuceneTestCase::new_index_writer_config();
        conf.set_commit_on_close(false);
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        writer.commit()?;
        let maybe_fail_delete = Arc::new(AtomicBool::new(false));
        dir.as_mock()
            .expect("newMockDirectory")
            .fail_on(Arc::new(SyncMetadataFailure {
                do_fail: AtomicBool::new(false),
                maybe_fail_delete: Arc::clone(&maybe_fail_delete),
            }));
        for i in 0..5 {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "id",
                &i.to_string(),
                Store::NO,
            )));
            doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
            doc.add(Box::new(BinaryDocValuesField::new(
                "dv2",
                BytesRef::from_char_sequence(&i.to_string()),
            )));
            doc.add(Box::new(SortedDocValuesField::new(
                "dv3",
                BytesRef::from_char_sequence(&i.to_string()),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "dv4",
                BytesRef::from_char_sequence(&i.to_string()),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "dv4",
                BytesRef::from_char_sequence(&(i - 1).to_string()),
            )));
            doc.add(Box::new(SortedNumericDocValuesField::new("dv5", i as i64)));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "dv5",
                (i - 1) as i64,
            )));
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "text1",
                &TestUtil::random_analysis_string(&*random(), 20, true),
                Store::NO,
            )));
            // ensure we store something
            doc.add(Box::new(StoredField::new_string(
                "stored1",
                "foo".to_string(),
            )?));
            doc.add(Box::new(StoredField::new_string(
                "stored1",
                "bar".to_string(),
            )?));
            // ensure we get some payloads
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "text_payloads",
                &TestUtil::random_analysis_string(&*random(), 6, true),
                Store::NO,
            )));
            // ensure we get some vectors
            let ft = FieldType::new_from(TextField::type_not_stored());
            ft.set_store_term_vectors(true)?;
            doc.add(Box::new(LuceneTestCase::new_field(
                "text_vectors",
                &TestUtil::random_analysis_string(&*random(), 6, true),
                &ft,
            )));
            doc.add(Box::new(IntPoint::new("point", &[random().next_int()])?));
            let x = random().next_int();
            let y = random().next_int();
            doc.add(Box::new(IntPoint::new("point2d", &[x, y])?));
            let _ = doc;
            writer.add_document(&Document::new())?;
        }
        let expected = expect_throws(RUNTIME_EXCEPTION, || writer.commit());
        assert_equals(&Some("boom"), &expected.message());

        maybe_fail_delete.store(true, Ordering::SeqCst);
        match catching(|| writer.rollback()) {
            Ok(()) => {}
            Err(e) if e.is_instance(RUNTIME_EXCEPTION) => {
                assert_equals(&Some("bang"), &e.message());
            }
            Err(t) => t.throw(),
        }
        maybe_fail_delete.store(false, Ordering::SeqCst);
        Assert::assert_true(writer.is_closed());
        Assert::assert_true(directory_reader::index_exists(&*dir.as_directory())?);
        directory_reader::open(dir.as_directory())?.close()?;
        Ok(())
    })();
    // try-with-resources: `dir.close()`.
    let closed = dir.close();
    body?;
    closed
}

/// Port of `TestIndexWriterExceptions.testExceptionJustBeforeFlushWithPointValues`.
fn test_exception_just_before_flush_with_point_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(CrashingAnalyzer::new(
        None,
        "org.apache.lucene.index.TestIndexWriterExceptions$21",
    ));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    iwc.set_commit_on_close(false);
    iwc.set_max_buffered_docs(3)?;
    let mp = iwc.merge_policy();
    iwc.set_merge_policy(Some(Arc::new(
        rucene::index::merge_policy::SoftDeletesRetentionMergePolicy::new(
            "soft_delete",
            Arc::new(|| Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>),
            mp,
        ),
    )))?;
    let w = RandomIndexWriter::mock_index_writer(dir.as_directory(), iwc, &*random())?;
    let mut newdoc = Document::new();
    newdoc.add(Box::new(LuceneTestCase::new_text_field(
        "crash",
        "do it on token 4",
        Store::NO,
    )));
    newdoc.add(Box::new(IntPoint::new("int", &[42])?));
    expect_throws(IOE, || w.add_document(&newdoc));
    let r = directory_reader::open_with_writer_flags(&*w, false, false)?;
    let only_reader = LuceneTestCase::get_only_leaf_reader(r as Arc<dyn IndexReader>)?;
    // we mark the failed doc as deleted
    assert_equals(
        &Arc::clone(&only_reader)
            .as_index_reader()
            .num_deleted_docs(),
        &1,
    );
    // there are not points values (rather than an empty set of values)
    Assert::assert_true(only_reader.get_point_values("field")?.is_none());
    only_reader.as_index_reader().close()?;
    w.close()?;
    dir.close()
}
