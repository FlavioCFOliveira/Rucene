//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterWithThreads.java`.
//!
//! MultiThreaded IndexWriter tests
//!
//! * A Java thread's fields the test reads after `join()` (`error`,
//!   `addCount`, `failed`, `failure`) live in a state the thread shares
//!   with the test.
//! * `catch (Throwable t)` catches both a returned error and a panic
//!   (`try_catch("java.lang.Throwable", …)`).
//! * `callStackContainsAnyOf("finishDocument")` and the other Java method
//!   names are the Rust method names (`finish_document`), which the stack
//!   walker reports.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, NumericDocValuesField, Store, StringField, TextField};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, MergeScheduler};
use rucene::index::multi_bits::MultiBits;
use rucene::index::terms::Term;
use rucene::index::{DefaultIndexingChain, IndexReader};
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::suppressing_concurrent_merge_scheduler::SuppressingConcurrentMergeScheduler;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::{Failure, MockDirectoryWrapper};
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, TEST_NIGHTLY};
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, class_annotated, random, test, try_catch, verbose,
};
use crate::support::{npe, CyclicBarrier};

const IOE: &str = "java.io.IOException";
const ISE: &str = "java.lang.IllegalStateException";
const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";
const NPE: &str = "java.lang.NullPointerException";
const LOFE: &str = "org.apache.lucene.store.LockObtainFailedException";
const THROWABLE: &str = "java.lang.Throwable";

/// `TestIndexWriterWithThreads` (`@SuppressCodecs("SimpleText")`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterWithThreads",
        vec![
            test(
                "testImmediateDiskFullWithThreads",
                test_immediate_disk_full_with_threads,
            ),
            // `@SuppressForbidden(reason = "Thread sleep")`: a forbidden-APIs
            // marker, with no runtime effect.
            test("testCloseWithThreads", test_close_with_threads),
            test("testIOExceptionDuringAbort", test_io_exception_during_abort),
            test(
                "testIOExceptionDuringAbortOnlyOnce",
                test_io_exception_during_abort_only_once,
            ),
            test(
                "testIOExceptionDuringAbortWithThreads",
                test_io_exception_during_abort_with_threads,
            ),
            test(
                "testIOExceptionDuringAbortWithThreadsOnlyOnce",
                test_io_exception_during_abort_with_threads_only_once,
            ),
            test(
                "testIOExceptionDuringWriteSegment",
                test_io_exception_during_write_segment,
            ),
            test(
                "testIOExceptionDuringWriteSegmentOnlyOnce",
                test_io_exception_during_write_segment_only_once,
            ),
            test(
                "testIOExceptionDuringWriteSegmentWithThreads",
                test_io_exception_during_write_segment_with_threads,
            ),
            test(
                "testIOExceptionDuringWriteSegmentWithThreadsOnlyOnce",
                test_io_exception_during_write_segment_with_threads_only_once,
            ),
            test(
                "testOpenTwoIndexWritersOnDifferentThreads",
                test_open_two_index_writers_on_different_threads,
            ),
            test(
                "testRollbackAndCommitWithThreads",
                test_rollback_and_commit_with_threads,
            ),
            test(
                "testUpdateSingleDocWithThreads",
                test_update_single_doc_with_threads,
            ),
            test(
                "testSoftUpdateSingleDocWithThreads",
                test_soft_update_single_doc_with_threads,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `(ConcurrentMergeScheduler) writer.getConfig().getMergeScheduler()`.
fn set_suppress_exceptions(writer: &IndexWriter) {
    let ms: Arc<dyn MergeScheduler> = writer.get_config().merge_scheduler();
    ms.as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "not a ConcurrentMergeScheduler",
            )
            .throw()
        })
        .set_suppress_exceptions();
}

/// The configuration the threaded tests share: `setMaxBufferedDocs(n)`,
/// a `ConcurrentMergeScheduler`, `newLogMergePolicy(4)` and
/// `setCommitOnClose(false)`.
fn threaded_iwc(max_buffered_docs: i32) -> Result<IndexWriterConfig> {
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(max_buffered_docs)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(4)?,
    )))?;
    iwc.set_commit_on_close(false);
    Ok(iwc)
}

/// The fields of `IndexerThread` the test reads: `error` and `addCount`.
#[derive(Default)]
struct IndexerState {
    error: Mutex<Option<Throwable>>,
    add_count: AtomicI32,
}

impl IndexerState {
    fn set_error(&self, t: Throwable) {
        *self.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(t);
    }

    fn has_error(&self) -> bool {
        self.error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }
}

/// Used by test cases below: `private static class IndexerThread extends Thread`.
struct IndexerThread {
    thread: Thread,
    state: Arc<IndexerState>,
}

impl IndexerThread {
    fn new(writer: Arc<IndexWriter>, no_errors: bool, sync_start: Arc<CyclicBarrier>) -> Self {
        let state = Arc::new(IndexerState::default());
        let s = Arc::clone(&state);
        let thread = Thread::unnamed(move || Self::run(&writer, no_errors, &sync_start, &s));
        IndexerThread { thread, state }
    }

    fn run(
        writer: &IndexWriter,
        no_errors: bool,
        sync_start: &CyclicBarrier,
        state: &IndexerState,
    ) {
        if let Err(e) = sync_start.await_() {
            let t = Throwable::new(e.java_class(), "");
            state.set_error(t.clone());
            Throwable::from_cause("java.lang.RuntimeException", t).throw();
        }

        let mut doc = Document::new();
        let custom_type = FieldType::new_from(TextField::type_stored());
        let setup = (|| -> Result<()> {
            custom_type.set_store_term_vectors(true)?;
            custom_type.set_store_term_vector_positions(true)?;
            custom_type.set_store_term_vector_offsets(true)?;
            Ok(())
        })();
        if let Err(e) = setup {
            Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw();
        }

        doc.add(Box::new(LuceneTestCase::new_field(
            "field",
            "aaa bbb ccc ddd eee fff ggg hhh iii jjj",
            &custom_type,
        )));
        doc.add(Box::new(NumericDocValuesField::new("dv", 5)));

        let mut id_upto = 0;
        let mut full_count = 0;

        loop {
            let id = id_upto;
            id_upto += 1;
            let outcome = try_catch(THROWABLE, || {
                writer.update_document(Term::from_text("id", &id.to_string()), &doc)
            });
            let err = match outcome {
                Ok(Ok(_)) => {
                    state.add_count.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                Ok(Err(e)) => e,
                Err(t) => {
                    if no_errors {
                        println!("{}: ERROR: unexpected Throwable:", Thread::current().name());
                        println!("{t}");
                        state.set_error(t);
                    }
                    break;
                }
            };
            if is_instance(&err, IOE) {
                let ioe = java_throwable(&err);
                if verbose() {
                    println!("TEST: expected exc:");
                    println!("{ioe}");
                }
                let message = ioe.message().unwrap_or_default();
                if message.starts_with("fake disk full at") || message == "now failing on purpose" {
                    if let Err(ie) = Thread::try_sleep(1) {
                        Throwable::from_cause(
                            "org.apache.lucene.util.ThreadInterruptedException",
                            ie,
                        )
                        .throw();
                    }
                    let previous = full_count;
                    full_count += 1;
                    if previous >= 5 {
                        break;
                    }
                } else {
                    if no_errors {
                        println!(
                            "{}: ERROR: unexpected IOException:",
                            Thread::current().name()
                        );
                        println!("{ioe}");
                        state.set_error(ioe);
                    }
                    break;
                }
            } else if is_instance(&err, ISE) {
                // OK: abort closes the writer
                break;
            } else {
                if no_errors {
                    println!("{}: ERROR: unexpected Throwable:", Thread::current().name());
                    let t = java_throwable(&err);
                    println!("{t}");
                    state.set_error(t);
                }
                break;
            }
        }
    }
}

/// Starts `num_threads` `IndexerThread`s over `writer`.
fn start_indexers(
    writer: &Arc<IndexWriter>,
    no_errors: bool,
    sync_start: &Arc<CyclicBarrier>,
    num_threads: usize,
) -> Vec<IndexerThread> {
    let mut threads = Vec::with_capacity(num_threads);
    for _ in 0..num_threads {
        let t = IndexerThread::new(Arc::clone(writer), no_errors, Arc::clone(sync_start));
        t.thread.start();
        threads.push(t);
    }
    threads
}

/// `syncStart.await()` on the test thread.
fn barrier_await(barrier: &CyclicBarrier) -> Result<()> {
    barrier
        .await_()
        .map(|_| ())
        .map_err(|e| Throwable::new(e.java_class(), "").throw())
}

/// LUCENE-1130: make sure immediate disk full on creating
/// an IndexWriter (hit during DWPT#updateDocuments()), with
/// multiple threads, is OK:
///
/// Port of `TestIndexWriterWithThreads.testImmediateDiskFullWithThreads`.
fn test_immediate_disk_full_with_threads() -> Result<()> {
    let num_threads = 3;
    let num_iterations = if *TEST_NIGHTLY { 10 } else { 1 };
    for iter in 0..num_iterations {
        if verbose() {
            println!("\nTEST: iter={iter}");
        }
        let mut dir = LuceneTestCase::new_mock_directory();
        let writer = IndexWriter::new(dir.as_directory(), threaded_iwc(2)?)?;
        set_suppress_exceptions(&writer);
        dir.set_max_size_in_bytes(4 * 1024 + 20 * iter);

        let sync_start = Arc::new(CyclicBarrier::new(num_threads + 1));
        let threads = start_indexers(&writer, true, &sync_start, num_threads as usize);
        barrier_await(&sync_start)?;

        for t in &threads {
            // Without fix for LUCENE-1130: one of the
            // threads will hang
            let _ = t.thread.join();
            Assert::assert_true_msg("hit unexpected Throwable", !t.state.has_error());
        }

        // Make sure once disk space is avail again, we can
        // cleanly close:
        dir.set_max_size_in_bytes(0);
        let committed = writer.commit();
        let after = match committed {
            Ok(_) => Ok(()),
            Err(e) if is_instance(&e, ACE) => {
                // OK: abort closes the writer
                writer.is_deleter_closed().map(Assert::assert_true)
            }
            Err(e) => Err(e),
        };
        // finally
        writer.close()?;
        after?;
        drop(writer);
        dir.close()?;
    }
    Ok(())
}

/// LUCENE-1130: make sure we can close() even while
/// threads are trying to add documents.  Strictly
/// speaking, this isn't valid us of Lucene's APIs, but we
/// still want to be robust to this case:
///
/// Port of `TestIndexWriterWithThreads.testCloseWithThreads`.
fn test_close_with_threads() -> Result<()> {
    let num_threads = 3;
    let num_iterations = if *TEST_NIGHTLY { 7 } else { 3 };
    for iter in 0..num_iterations {
        if verbose() {
            println!("\nTEST: iter={iter}");
        }
        let mut dir = LuceneTestCase::new_directory();

        let writer = IndexWriter::new(dir.as_directory(), threaded_iwc(10)?)?;
        set_suppress_exceptions(&writer);

        let sync_start = Arc::new(CyclicBarrier::new(num_threads + 1));
        let threads = start_indexers(&writer, false, &sync_start, num_threads as usize);
        barrier_await(&sync_start)?;

        let mut done = false;
        while !done {
            Thread::sleep(100);
            for t in &threads {
                // only stop when at least one thread has added a doc
                if t.state.add_count.load(Ordering::SeqCst) > 0 {
                    done = true;
                    break;
                } else if !t.thread.is_alive() {
                    Assert::fail("thread failed before indexing a single document");
                }
            }
        }

        if verbose() {
            println!("\nTEST: now close");
        }
        let committed = writer.commit();
        // finally
        writer.close()?;
        committed?;

        // Make sure threads that are adding docs are not hung:
        for t in &threads {
            // Without fix for LUCENE-1130: one of the
            // threads will hang
            let _ = t.thread.join();

            // [DW] this is unreachable once join() returns a thread cannot be alive.
            if t.thread.is_alive() {
                Assert::fail("thread seems to be hung");
            }
        }

        // Quick test to make sure index is not corrupt:
        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let mut tdocs = TestUtil::docs(
            &*random(),
            &reader,
            "field",
            &BytesRef::from_char_sequence("aaa"),
            None,
            0,
        )?
        .unwrap_or_else(|| npe("tdocs"));
        let mut count = 0;
        while tdocs.next_doc()? != NO_MORE_DOCS {
            count += 1;
        }
        Assert::assert_true(count > 0);
        reader.close()?;

        drop(writer);
        dir.close()?;
    }
    Ok(())
}

/// Runs test, with multiple threads, using the specific
/// failure to trigger an IOException
///
/// Port of `TestIndexWriterWithThreads._testMultipleThreadsFailure`.
fn testing_multiple_threads_failure(failure: Arc<dyn Failure>) -> Result<()> {
    let num_threads = 3;

    for iter in 0..2 {
        if verbose() {
            println!("TEST: iter={iter}");
        }
        let mut dir = LuceneTestCase::new_mock_directory();

        let writer = IndexWriter::new(dir.as_directory(), threaded_iwc(2)?)?;
        set_suppress_exceptions(&writer);

        let sync_start = Arc::new(CyclicBarrier::new(num_threads + 1));
        let threads = start_indexers(&writer, true, &sync_start, num_threads as usize);
        barrier_await(&sync_start)?;

        dir.fail_on(Arc::clone(&failure));
        failure.set_do_fail();

        for t in &threads {
            let _ = t.thread.join();
            Assert::assert_true_msg("hit unexpected Throwable", !t.state.has_error());
        }

        let mut success = false;
        let attempt = (|| -> Result<()> {
            writer.commit()?;
            writer.close()?;
            success = true;
            Ok(())
        })();
        let handled = match attempt {
            Ok(()) => Ok(()),
            Err(e) if is_instance(&e, ACE) => {
                // OK: abort closes the writer
                writer.is_deleter_closed().map(Assert::assert_true)
            }
            Err(e) if is_instance(&e, IOE) => {
                let rolled_back = writer.rollback();
                failure.clear_do_fail();
                rolled_back
            }
            Err(e) => Err(e),
        };
        // finally
        writer.close()?;
        handled?;
        if verbose() {
            println!("TEST: success={success}");
        }

        if success {
            let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
            let del_docs = MultiBits::get_live_docs(&reader)?;
            let stored_fields = reader.stored_fields()?;
            let term_vectors = reader.term_vectors()?;
            for j in 0..reader.max_doc() {
                if del_docs.as_ref().is_none_or(|d| !d.get(j as usize)) {
                    stored_fields.document(j)?;
                    term_vectors.get(j)?;
                }
            }
            reader.close()?;
        }

        drop(writer);
        dir.close()?;
    }
    Ok(())
}

/// Runs test, with one thread, using the specific failure
/// to trigger an IOException
///
/// Port of `TestIndexWriterWithThreads._testSingleThreadFailure`.
fn testing_single_thread_failure(failure: Arc<dyn Failure>) -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();

    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_commit_on_close(false);

    if iwc
        .merge_scheduler()
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .is_some()
    {
        iwc.set_merge_scheduler(Some(Arc::new(SuppressingConcurrentMergeScheduler::create(
            Arc::new(|th| match th.downcast_ref::<LuceneError>() {
                Some(e) => {
                    is_instance(e, ACE)
                        || (is_instance(e, ISE)
                            && java_throwable(e)
                                .message()
                                .unwrap_or_default()
                                .contains("this writer hit an unrecoverable error"))
                }
                None => false,
            }),
        ))))?;
    }

    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    custom_type.set_store_term_vector_offsets(true)?;
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "aaa bbb ccc ddd eee fff ggg hhh iii jjj",
        &custom_type,
    )));

    for _ in 0..6 {
        writer.add_document(&doc)?;
    }

    dir.fail_on(Arc::clone(&failure));
    failure.set_do_fail();
    expect_throws(IOE, || {
        writer.add_document(&doc)?;
        writer.add_document(&doc)?;
        writer.commit()
    });

    failure.clear_do_fail();
    expect_throws(ACE, || {
        writer.add_document(&doc)?;
        writer.commit()?;
        writer.close()
    });

    Assert::assert_true(writer.is_deleter_closed()?);
    drop(writer);
    dir.close()
}

/// Throws IOException during FieldsWriter.flushDocument and during DocumentsWriter.abort
///
/// `private static class FailOnlyOnAbortOrFlush extends MockDirectoryWrapper.Failure`.
struct FailOnlyOnAbortOrFlush {
    do_fail: AtomicBool,
    only_once: bool,
}

impl FailOnlyOnAbortOrFlush {
    fn new(only_once: bool) -> Arc<dyn Failure> {
        Arc::new(FailOnlyOnAbortOrFlush {
            do_fail: AtomicBool::new(false),
            only_once,
        })
    }
}

impl Failure for FailOnlyOnAbortOrFlush {
    fn eval(&self, dir: &MockDirectoryWrapper) -> Result<()> {
        // Since we throw exc during abort, eg when IW is
        // attempting to delete files, we will leave
        // leftovers:
        dir.set_assert_no_unrefenced_files_on_close(false);

        if self.do_fail()
            && LuceneTestCase::call_stack_contains_any_of(&["abort", "finish_document"])
            && !LuceneTestCase::call_stack_contains_any_of(&["merge", "close"])
        {
            if self.only_once {
                self.do_fail.store(false, Ordering::SeqCst);
            }
            return Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other("now failing on purpose"),
            )));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// LUCENE-1130: make sure initial IOException, and then 2nd
/// IOException during rollback(), is OK:
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringAbort`.
fn test_io_exception_during_abort() -> Result<()> {
    testing_single_thread_failure(FailOnlyOnAbortOrFlush::new(false))
}

/// LUCENE-1130: make sure initial IOException, and then 2nd
/// IOException during rollback(), is OK:
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringAbortOnlyOnce`.
fn test_io_exception_during_abort_only_once() -> Result<()> {
    testing_single_thread_failure(FailOnlyOnAbortOrFlush::new(true))
}

/// LUCENE-1130: make sure initial IOException, and then 2nd
/// IOException during rollback(), with multiple threads, is OK:
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringAbortWithThreads`.
fn test_io_exception_during_abort_with_threads() -> Result<()> {
    testing_multiple_threads_failure(FailOnlyOnAbortOrFlush::new(false))
}

/// LUCENE-1130: make sure initial IOException, and then 2nd
/// IOException during rollback(), with multiple threads, is OK:
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringAbortWithThreadsOnlyOnce`.
fn test_io_exception_during_abort_with_threads_only_once() -> Result<()> {
    testing_multiple_threads_failure(FailOnlyOnAbortOrFlush::new(true))
}

/// Throws IOException during DocumentsWriter.writeSegment
///
/// `private static class FailOnlyInWriteSegment extends MockDirectoryWrapper.Failure`.
struct FailOnlyInWriteSegment {
    do_fail: AtomicBool,
    only_once: bool,
}

impl FailOnlyInWriteSegment {
    fn new(only_once: bool) -> Arc<dyn Failure> {
        Arc::new(FailOnlyInWriteSegment {
            do_fail: AtomicBool::new(false),
            only_once,
        })
    }
}

impl Failure for FailOnlyInWriteSegment {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail()
            && LuceneTestCase::call_stack_contains_method::<DefaultIndexingChain>("flush")
        {
            if self.only_once {
                self.do_fail.store(false, Ordering::SeqCst);
            }
            return Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other("now failing on purpose"),
            )));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// LUCENE-1130: test IOException in writeSegment
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringWriteSegment`.
fn test_io_exception_during_write_segment() -> Result<()> {
    testing_single_thread_failure(FailOnlyInWriteSegment::new(false))
}

/// LUCENE-1130: test IOException in writeSegment
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringWriteSegmentOnlyOnce`.
fn test_io_exception_during_write_segment_only_once() -> Result<()> {
    testing_single_thread_failure(FailOnlyInWriteSegment::new(true))
}

/// LUCENE-1130: test IOException in writeSegment, with threads
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringWriteSegmentWithThreads`.
fn test_io_exception_during_write_segment_with_threads() -> Result<()> {
    testing_multiple_threads_failure(FailOnlyInWriteSegment::new(false))
}

/// LUCENE-1130: test IOException in writeSegment, with threads
///
/// Port of `TestIndexWriterWithThreads.testIOExceptionDuringWriteSegmentWithThreadsOnlyOnce`.
fn test_io_exception_during_write_segment_with_threads_only_once() -> Result<()> {
    testing_multiple_threads_failure(FailOnlyInWriteSegment::new(true))
}

/// The fields of `DelayedIndexAndCloseRunnable` the test reads.
#[derive(Default)]
struct DelayedState {
    failed: AtomicBool,
    failure: Mutex<Option<Throwable>>,
}

/// `static class DelayedIndexAndCloseRunnable extends Thread`.
fn delayed_index_and_close_runnable(
    dir: Arc<dyn Directory>,
    sync_start: Arc<CyclicBarrier>,
) -> (Thread, Arc<DelayedState>) {
    let state = Arc::new(DelayedState::default());
    let s = Arc::clone(&state);
    let thread = Thread::unnamed(move || {
        let outcome = try_catch(THROWABLE, || -> Result<()> {
            let mut doc = Document::new();
            let field = LuceneTestCase::new_text_field("field", "testData", Store::YES);
            doc.add(Box::new(field));

            if let Err(e) = sync_start.await_() {
                Throwable::new(e.java_class(), "").throw();
            }
            let writer = IndexWriter::new(Arc::clone(&dir), mock_iwc())?;
            writer.add_document(&doc)?;
            writer.close()
        });
        let caught = match outcome {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(java_throwable(&e)),
            Err(t) => Some(t),
        };
        if let Some(t) = caught {
            s.failed.store(true, Ordering::SeqCst);
            *s.failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(t);
        }
    });
    (thread, state)
}

/// LUCENE-3365: Test adding two documents with the same field from two different IndexWriters
/// that we attempt to open at the same time.  As long as the first IndexWriter completes
/// and closes before the second IndexWriter time's out trying to get the Lock,
/// we should see both documents
///
/// Port of `TestIndexWriterWithThreads.testOpenTwoIndexWritersOnDifferentThreads`.
fn test_open_two_index_writers_on_different_threads() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let body = (|| -> Result<()> {
        let sync_start = Arc::new(CyclicBarrier::new(2));
        let (thread1, state1) =
            delayed_index_and_close_runnable(dir.as_directory(), Arc::clone(&sync_start));
        let (thread2, state2) =
            delayed_index_and_close_runnable(dir.as_directory(), Arc::clone(&sync_start));
        thread1.start();
        thread2.start();
        let _ = thread1.join();
        let _ = thread2.join();

        let failure1 = state1
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let failure2 = state2
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if failure1.as_ref().is_some_and(|t| t.is_instance(LOFE))
            || failure2.as_ref().is_some_and(|t| t.is_instance(LOFE))
        {
            // We only care about the situation when the two writers succeeded.
            return Ok(());
        }

        let describe = |t: &Option<Throwable>| match t {
            Some(t) => t.to_string(),
            None => "null".to_string(),
        };
        Assert::assert_false_msg(
            &format!("Failed due to: {}", describe(&failure1)),
            state1.failed.load(Ordering::SeqCst),
        );
        Assert::assert_false_msg(
            &format!("Failed due to: {}", describe(&failure2)),
            state2.failed.load(Ordering::SeqCst),
        );

        // now verify that we have two documents in the index
        let reader = directory_reader::open(dir.as_directory())?;
        assert_equals_msg(
            Some("IndexReader should have one document per thread running"),
            &2,
            &reader.num_docs(),
        );

        reader.close()
    })();
    // try-with-resources closes the directory
    let closed = dir.close();
    body?;
    closed
}

/// LUCENE-4147
///
/// Port of `TestIndexWriterWithThreads.testRollbackAndCommitWithThreads`.
fn test_rollback_and_commit_with_threads() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();

    let thread_count = TestUtil::next_int(&*random(), 2, 6);

    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));

    let dir = d.as_directory();
    let writer_ref: Arc<Mutex<Arc<IndexWriter>>> = Arc::new(Mutex::new(IndexWriter::new(
        Arc::clone(&dir),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer))),
    )?));
    let get =
        |r: &Mutex<Arc<IndexWriter>>| Arc::clone(&r.lock().unwrap_or_else(|p| p.into_inner()));
    // Make initial commit so the test doesn't trip "corrupt first commit" when virus checker
    // refuses to delete partial segments_N file:
    get(&writer_ref).commit()?;
    let docs = Arc::new(LineFileDocs::with_default_file(&*random())?);
    let iters = LuceneTestCase::at_least(100);
    let failed = Arc::new(AtomicBool::new(false));
    let rollback_lock = Arc::new(Mutex::new(()));
    let commit_lock = Arc::new(Mutex::new(()));
    let mut threads = Vec::with_capacity(thread_count as usize);
    for _ in 0..thread_count {
        let writer_ref = Arc::clone(&writer_ref);
        let docs = Arc::clone(&docs);
        let failed = Arc::clone(&failed);
        let rollback_lock = Arc::clone(&rollback_lock);
        let commit_lock = Arc::clone(&commit_lock);
        let dir = Arc::clone(&dir);
        let t = Thread::unnamed(move || {
            let get = |r: &Mutex<Arc<IndexWriter>>| {
                Arc::clone(&r.lock().unwrap_or_else(|p| p.into_inner()))
            };
            let mut iter = 0;
            while iter < iters && !failed.load(Ordering::SeqCst) {
                // final int x = random().nextInt(5);
                let x = random().next_int_bound(3);
                let outcome = try_catch(THROWABLE, || -> Result<()> {
                    match x {
                        0 => {
                            let _guard = rollback_lock.lock().unwrap_or_else(|p| p.into_inner());
                            if verbose() {
                                println!("\nTEST: {}: now rollback", Thread::current().name());
                            }
                            get(&writer_ref).rollback()?;
                            if verbose() {
                                println!(
                                    "TEST: {}: rollback done; now open new writer",
                                    Thread::current().name()
                                );
                            }
                            let w = IndexWriter::new(Arc::clone(&dir), mock_iwc())?;
                            *writer_ref.lock().unwrap_or_else(|p| p.into_inner()) = w;
                        }
                        1 => {
                            let _guard = commit_lock.lock().unwrap_or_else(|p| p.into_inner());
                            if verbose() {
                                println!("\nTEST: {}: now commit", Thread::current().name());
                            }
                            let committed = (|| -> Result<()> {
                                if random().next_boolean() {
                                    get(&writer_ref).prepare_commit()?;
                                }
                                get(&writer_ref).commit()?;
                                Ok(())
                            })();
                            match committed {
                                Err(e) if is_instance(&e, ACE) || is_instance(&e, NPE) => {
                                    // ok
                                }
                                other => other?,
                            }
                        }
                        2 => {
                            if verbose() {
                                println!("\nTEST: {}: now add", Thread::current().name());
                            }
                            let added = try_catch("java.lang.AssertionError", || {
                                let doc = docs.next_doc()?;
                                let doc = doc.borrow();
                                get(&writer_ref).add_document(&*doc)
                            });
                            match added {
                                Ok(Err(e)) if is_instance(&e, ACE) || is_instance(&e, NPE) => {
                                    // ok
                                }
                                Ok(other) => {
                                    other?;
                                }
                                Err(_assertion_error) => {
                                    // ok
                                }
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                });
                let caught = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(java_throwable(&e)),
                    Err(t) => Some(t),
                };
                if let Some(t) = caught {
                    failed.store(true, Ordering::SeqCst);
                    Throwable::from_cause("java.lang.RuntimeException", t).throw();
                }
                iter += 1;
            }
        });
        t.start();
        threads.push(t);
    }

    for t in &threads {
        let _ = t.join();
    }

    Assert::assert_true(!failed.load(Ordering::SeqCst));
    get(&writer_ref).close()?;
    drop(writer_ref);
    d.close()
}

/// Port of `TestIndexWriterWithThreads.testUpdateSingleDocWithThreads`.
fn test_update_single_doc_with_threads() -> Result<()> {
    stress_update_single_doc_with_threads(false, LuceneTestCase::rarely())
}

/// Port of `TestIndexWriterWithThreads.testSoftUpdateSingleDocWithThreads`.
fn test_soft_update_single_doc_with_threads() -> Result<()> {
    stress_update_single_doc_with_threads(true, LuceneTestCase::rarely())
}

/// Port of `TestIndexWriterWithThreads.stressUpdateSingleDocWithThreads`.
fn stress_update_single_doc_with_threads(use_soft_deletes: bool, force_merge: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_max_buffered_docs(-1)?;
    iwc.set_ram_buffer_size_mb(0.00001)?;
    let writer = match RandomIndexWriter::new_with_soft_deletes(
        &*random(),
        dir.as_directory(),
        iwc,
        use_soft_deletes,
    ) {
        Ok(w) => Arc::new(w),
        Err(e) => {
            let _ = dir.close();
            return Err(e);
        }
    };
    let body = (|| -> Result<()> {
        let num_threads = if *TEST_NIGHTLY {
            3 + random().next_int_bound(3)
        } else {
            3
        };
        let done = Arc::new(AtomicI32::new(0));
        let barrier = Arc::new(CyclicBarrier::new(num_threads + 1));
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", "1".into(), Store::NO)?));
        writer.update_document(Term::from_text("id", "1"), &doc)?;
        let iters_per_thread = 100 + random().next_int_bound(2000);
        let mut threads = Vec::with_capacity(num_threads as usize);
        for _ in 0..num_threads {
            let barrier = Arc::clone(&barrier);
            let writer = Arc::clone(&writer);
            let done = Arc::clone(&done);
            let t = Thread::unnamed(move || {
                let outcome = try_catch("java.lang.Exception", || -> Result<()> {
                    if let Err(e) = barrier.await_() {
                        Throwable::new(e.java_class(), "").throw();
                    }
                    for _ in 0..iters_per_thread {
                        let mut d = Document::new();
                        d.add(Box::new(StringField::new("id", "1".into(), Store::NO)?));
                        writer.update_document(Term::from_text("id", "1"), &d)?;
                    }
                    Ok(())
                });
                // finally
                done.fetch_add(1, Ordering::SeqCst);
                let caught = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(java_throwable(&e)),
                    Err(t) => Some(t),
                };
                if let Some(e) = caught {
                    Throwable::from_cause("java.lang.AssertionError", e).throw();
                }
            });
            t.start();
            threads.push(t);
        }
        let mut open = directory_reader::open_with_writer(&**writer.w())?;
        assert_equals(&open.num_docs(), &1);
        barrier_await(&barrier)?;
        let looped = (|| -> Result<()> {
            loop {
                if force_merge && random().next_boolean() {
                    writer.force_merge(1)?;
                }
                if let Some(new_reader) = directory_reader::open_if_changed(Arc::clone(&open))? {
                    open.close()?;
                    open = new_reader;
                }
                assert_equals(&open.num_docs(), &1);
                if done.load(Ordering::SeqCst) >= num_threads {
                    break;
                }
            }
            Ok(())
        })();
        // finally
        let closed = open.close();
        for t in &threads {
            let _ = t.join();
        }
        looped?;
        closed
    })();
    // try-with-resources closes the writer, then the directory
    let writer_closed = writer.close();
    drop(writer);
    let dir_closed = dir.close();
    body?;
    writer_closed?;
    dir_closed
}
