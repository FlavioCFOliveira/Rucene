//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDirectoryReaderReopen.java`.
//!
//! * The anonymous `TestReopen` subclasses are [`Reopen`], a pair of
//!   closures for `openReader()` and `modifyIndex(int)`.
//! * The anonymous `ReaderThreadTask` subclasses are the two closures
//!   [`thread_safety`] starts; `stopped` is their shared flag.
//! * `Executors.newFixedThreadPool(4, new NamedThreadFactory(...))` is the
//!   `ThreadPoolExecutor` it returns ([`new_fixed_thread_pool`]).
//! * The test class's `createReaderMutex` field is [`CREATE_READER_MUTEX`]:
//!   the methods of a class run one at a time, so one lock per class serves
//!   as one lock per instance.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, DocumentStoredFieldVisitor, Field, FieldType, NumericDocValuesField, NumericValue,
    Store, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::{
    IndexDeletionPolicy, KeepOnlyLastCommitDeletionPolicy, SnapshotDeletionPolicy,
};
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy, NoMergePolicy};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::{Executor, Query, TermQuery};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::{IOUtils, NamedThreadFactory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::jdk::executor_service::ExecutorService;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::store::mock_directory_wrapper::{
    fake_io_exception, Failure, FAKE_IO_EXCEPTION,
};
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::lucene_test_case::{TestDirectory, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, class, random, test};
use crate::support::npe;
use crate::test_directory_reader::assert_index_equals;

/// `TestDirectoryReaderReopen`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDirectoryReaderReopen",
        vec![
            test("testReopen", test_reopen),
            test("testCommitReopen", test_commit_reopen),
            test("testCommitRecreate", test_commit_recreate),
            test("testThreadSafety", test_thread_safety),
            test("testReopenOnCommit", test_reopen_on_commit),
            test(
                "testOpenIfChangedNRTToCommit",
                test_open_if_changed_nrt_to_commit,
            ),
            test(
                "testOverDecRefDuringReopen",
                test_over_dec_ref_during_reopen,
            ),
            test(
                "testNPEAfterInvalidReindex1",
                test_npe_after_invalid_reindex1,
            ),
            test(
                "testNPEAfterInvalidReindex2",
                test_npe_after_invalid_reindex2,
            ),
            test("testNRTMdeletes", test_nrtm_deletes),
            test("testNRTMdeletes2", test_nrtm_deletes2),
            test("testNRTMupdates", test_nrtm_updates),
            test("testNRTMupdates2", test_nrtm_updates2),
            test(
                "testDeleteIndexFilesWhileReaderStillOpen",
                test_delete_index_files_while_reader_still_open,
            ),
            test(
                "testReuseUnchangedLeafReaderOnDVUpdate",
                test_reuse_unchanged_leaf_reader_on_dv_update,
            ),
            test("testReopenWithExecutor", test_reopen_with_executor),
            test(
                "testCommitReopenWithExecutor",
                test_commit_reopen_with_executor,
            ),
            test(
                "testCommitRecreateWithExecutor",
                test_commit_recreate_with_executor,
            ),
            test(
                "testThreadSafetyWithExecutor",
                test_thread_safety_with_executor,
            ),
            test(
                "testReopenOnCommitWithExecutor",
                test_reopen_on_commit_with_executor,
            ),
            test(
                "testOpenIfChangedNRTToCommitWithExecutor",
                test_open_if_changed_nrt_to_commit_with_executor,
            ),
            test(
                "testOverDecRefDuringReopenWithExecutor",
                test_over_dec_ref_during_reopen_with_executor,
            ),
            test(
                "testNPEAfterInvalidReindex1WithExecutor",
                test_npe_after_invalid_reindex1_with_executor,
            ),
            test(
                "testNPEAfterInvalidReindex2WithExecutor",
                test_npe_after_invalid_reindex2_with_executor,
            ),
            test(
                "testNRTMdeletesWithExecutor",
                test_nrtm_deletes_with_executor,
            ),
            test(
                "testNRTMdeletes2WithExecutor",
                test_nrtm_deletes2_with_executor,
            ),
            test(
                "testNRTMupdatesWithExecutor",
                test_nrtm_updates_with_executor,
            ),
            test(
                "testNRTMupdates2WithExecutor",
                test_nrtm_updates2_with_executor,
            ),
            test(
                "testDeleteIndexFilesWhileReaderStillOpenWithExecutor",
                test_delete_index_files_while_reader_still_open_with_executor,
            ),
            test(
                "testReuseUnchangedLeafReaderOnDVUpdateWithExecutor",
                test_reuse_unchanged_leaf_reader_on_dv_update_with_executor,
            ),
        ],
    )
}

/// An `ExecutorService` argument, `null` when absent.
type Exec = Option<Arc<dyn Executor>>;

/// `Executors.newFixedThreadPool(n, new NamedThreadFactory("TestDirectoryReaderReopen"))`.
fn new_fixed_thread_pool(n: i32) -> Arc<ThreadPoolExecutor> {
    Arc::new(ThreadPoolExecutor::new(
        n,
        n,
        Duration::ZERO,
        NamedThreadFactory::new("TestDirectoryReaderReopen"),
    ))
}

/// The pool as the `ExecutorService` argument of a `DirectoryReader` call.
fn exec(executor: &Arc<ThreadPoolExecutor>) -> Exec {
    Some(Arc::clone(executor) as Arc<dyn Executor>)
}

/// Runs `body` with a fresh four-thread pool, shutting it down in
/// `finally`.
fn with_executor(body: impl FnOnce(&Arc<ThreadPoolExecutor>) -> Result<()>) -> Result<()> {
    let executor_service = new_fixed_thread_pool(4);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| body(&executor_service)));
    // finally
    executor_service.shutdown();
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// `DirectoryReader.open(dir)` or `DirectoryReader.open(dir, executorService)`.
fn open(dir: &Arc<dyn Directory>, executor: &Exec) -> Result<Arc<dyn DirectoryReader>> {
    match executor {
        None => directory_reader::open(Arc::clone(dir)),
        Some(e) => directory_reader::open_with_executor(Arc::clone(dir), Some(Arc::clone(e))),
    }
}

/// `DirectoryReader.openIfChanged(r)` or `DirectoryReader.openIfChanged(r,
/// executorService)`.
fn open_if_changed(
    r: &Arc<dyn DirectoryReader>,
    executor: &Exec,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    match executor {
        None => directory_reader::open_if_changed(Arc::clone(r)),
        Some(e) => {
            directory_reader::open_if_changed_with_executor(Arc::clone(r), Some(Arc::clone(e)))
        }
    }
}

/// `DirectoryReader.openIfChanged(r, commit)` or `DirectoryReader.openIfChanged(r,
/// commit, executorService)`.
fn open_if_changed_commit(
    r: &Arc<dyn DirectoryReader>,
    commit: &dyn IndexCommit,
    executor: &Exec,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    match executor {
        None => directory_reader::open_if_changed_with_commit(Arc::clone(r), commit),
        Some(e) => directory_reader::open_if_changed_with_commit_and_executor(
            Arc::clone(r),
            commit,
            Some(Arc::clone(e)),
        ),
    }
}

/// `a == b` on two readers: identity.
fn same_reader(a: &Arc<dyn DirectoryReader>, b: &Arc<dyn DirectoryReader>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn plain_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new IndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(NoMergePolicy.INSTANCE)`.
fn no_merge_iwc() -> Result<IndexWriterConfig> {
    let iwc = plain_iwc();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    Ok(iwc)
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new FieldType(TextField.TYPE_STORED)`, not tokenized, omitting norms
/// when asked.
fn untokenized_stored_type(omit_norms: bool) -> Result<FieldType> {
    let ft = FieldType::new_from(TextField::type_stored());
    ft.set_tokenized(false)?;
    if omit_norms {
        ft.set_omit_norms(true)?;
    }
    Ok(ft)
}

/// `FieldType customType3 = new FieldType(); customType3.setStored(true);`.
fn stored_only_type() -> Result<FieldType> {
    let ft = FieldType::new();
    ft.set_stored(true)?;
    Ok(ft)
}

/// `abstract static class TestReopen`: `openReader()` and
/// `modifyIndex(int)`.
struct Reopen {
    open_reader: Box<dyn Fn() -> Result<Arc<dyn DirectoryReader>> + Send + Sync>,
    modify_index: Box<dyn Fn(i32) -> Result<()> + Send + Sync>,
}

impl Reopen {
    /// `protected abstract DirectoryReader openReader()`.
    fn open_reader(&self) -> Result<Arc<dyn DirectoryReader>> {
        (self.open_reader)()
    }

    /// `protected abstract void modifyIndex(int i)`.
    fn modify_index(&self, i: i32) -> Result<()> {
        (self.modify_index)(i)
    }
}

/// The `TestReopen` of `testReopen` and `testReopenWithExecutor`:
/// `TestDirectoryReaderReopen.modifyIndex(i, dir)` and
/// `DirectoryReader.open(dir[, executorService])`.
fn default_reopen(dir: Arc<dyn Directory>, executor: Exec) -> Reopen {
    let modify_dir = Arc::clone(&dir);
    Reopen {
        open_reader: Box::new(move || open(&dir, &executor)),
        modify_index: Box::new(move |i| modify_index(i, &modify_dir)),
    }
}

/// The body `testReopen` and `testReopenWithExecutor` share.
fn reopen_body(executor: Option<&Arc<ThreadPoolExecutor>>, multi_segment: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let run = |dir: &TestDirectory| -> Result<()> {
        create_index(&*random(), &dir.as_directory(), multi_segment)?;
        perform_default_tests(&default_reopen(dir.as_directory(), executor.and_then(exec)))
    };
    match executor {
        None => run(&dir)?,
        Some(_) => {
            // try { … } finally { executorService.shutdown(); }
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| run(&dir)));
            if let Some(e) = executor {
                e.shutdown();
            }
            match result {
                Ok(result) => result?,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
    }
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testReopen`.
fn test_reopen() -> Result<()> {
    reopen_body(None, false)?;
    reopen_body(None, true)
}

// LUCENE-1228: IndexWriter.commit() does not update the index version
// populate an index in iterations.
// at the end of every iteration, commit the index and reopen/recreate the reader.
// in each iteration verify the work of previous iteration.
// try this once with reopen once recreate, on both RAMDir and FSDir.
/// Port of `TestDirectoryReaderReopen.testCommitReopen`.
fn test_commit_reopen() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    do_test_reopen_with_commit(&*random(), &dir.as_directory(), true)?;
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testCommitRecreate`.
fn test_commit_recreate() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    do_test_reopen_with_commit(&*random(), &dir.as_directory(), false)?;
    dir.close()
}

/// The writer config of `doTestReopenWithCommit`.
fn reopen_with_commit_iwc(
    random: &dyn randomizedtesting::jdk::random::Random,
) -> Result<IndexWriterConfig> {
    let iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(random))));
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    Ok(iwc)
}

/// `private void doTestReopenWithCommit(Random random, Directory dir,
/// boolean withReopen)`.
fn do_test_reopen_with_commit(
    random: &dyn randomizedtesting::jdk::random::Random,
    dir: &Arc<dyn Directory>,
    with_reopen: bool,
) -> Result<()> {
    let iwriter = IndexWriter::new(Arc::clone(dir), reopen_with_commit_iwc(random)?)?;
    iwriter.commit()?;
    let mut reader = directory_reader::open(Arc::clone(dir))?;
    let body = (|| -> Result<()> {
        let m = 3;
        let custom_type = untokenized_stored_type(false)?;
        let custom_type2 = untokenized_stored_type(true)?;
        let custom_type3 = stored_only_type()?;
        for i in 0..4 {
            for j in 0..m {
                let mut doc = Document::new();
                let id = format!("{i}_{j}");
                doc.add(Box::new(LuceneTestCase::new_field("id", &id, &custom_type)));
                doc.add(Box::new(LuceneTestCase::new_field(
                    "id2",
                    &id,
                    &custom_type2,
                )));
                doc.add(Box::new(LuceneTestCase::new_field(
                    "id3",
                    &id,
                    &custom_type3,
                )));
                iwriter.add_document(&doc)?;
                if i > 0 {
                    let k = i - 1;
                    let n = j + k * m;
                    let mut visitor = DocumentStoredFieldVisitor::new();
                    reader
                        .stored_fields()?
                        .document_with_visitor(n, &mut visitor)?;
                    let prev_itereation_doc = visitor.into_document();
                    let id = prev_itereation_doc.get("id");
                    assert_equals(&Some(format!("{k}_{j}")), &id);
                }
            }
            iwriter.commit()?;
            if with_reopen {
                // reopen
                if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&reader))? {
                    reader.close()?;
                    reader = r2;
                }
            } else {
                // recreate
                reader.close()?;
                reader = directory_reader::open(Arc::clone(dir))?;
            }
        }
        Ok(())
    })();
    // finally
    iwriter.close()?;
    reader.close()?;
    body
}

/// `private void performDefaultTests(TestReopen test)`.
fn perform_default_tests(test: &Reopen) -> Result<()> {
    let mut index1 = test.open_reader()?;
    let mut index2 = test.open_reader()?;

    assert_index_equals(&index1, &index2)?;

    // verify that reopen() does not return a new reader instance
    // in case the index has no changes
    let couple = refresh_reader_no_test(&index2, false)?;
    Assert::assert_true(same_reader(&couple.refreshed_reader, &index2));

    let couple = refresh_reader(&index2, Some(test), 0, true)?;
    index1.close()?;
    index1 = couple.new_reader.unwrap_or_else(|| npe("newReader"));

    let index2_refreshed = couple.refreshed_reader;
    index2.close()?;

    // test if refreshed reader and newly opened reader return equal results
    assert_index_equals(&index1, &index2_refreshed)?;

    index2_refreshed.close()?;
    assert_reader_closed(&index2, true);
    assert_reader_closed(&index2_refreshed, true);

    index2 = test.open_reader()?;

    for i in 1..4 {
        index1.close()?;
        let couple = refresh_reader(&index2, Some(test), i, true)?;
        // refresh DirectoryReader
        index2.close()?;

        index2 = couple.refreshed_reader;
        index1 = couple.new_reader.unwrap_or_else(|| npe("newReader"));
        assert_index_equals(&index1, &index2)?;
    }

    index1.close()?;
    index2.close()?;
    assert_reader_closed(&index1, true);
    assert_reader_closed(&index2, true);
    Ok(())
}

/// Port of `TestDirectoryReaderReopen.testThreadSafety`.
fn test_thread_safety() -> Result<()> {
    thread_safety(None)
}

/// The body `testThreadSafety` and `testThreadSafetyWithExecutor` share;
/// `executor` is the latter's `executorService`.
fn thread_safety(executor: Exec) -> Result<()> {
    let mut dir_holder = LuceneTestCase::new_directory();
    let dir = dir_holder.as_directory();
    // NOTE: this also controls the number of threads!
    let n = TestUtil::next_int(&*random(), 20, 40);

    let writer = IndexWriter::new(Arc::clone(&dir), mock_iwc())?;
    for i in 0..n {
        writer.add_document(&create_document(i, 3)?)?;
    }
    writer.force_merge(1)?;
    writer.close()?;

    let test = {
        let modify_dir = Arc::clone(&dir);
        let open_dir = Arc::clone(&dir);
        let open_exec = executor.clone();
        Arc::new(Reopen {
            modify_index: Box::new(move |i| {
                let modifier = IndexWriter::new(Arc::clone(&modify_dir), plain_iwc())?;
                modifier.add_document(&create_document(n + i, 6)?)?;
                modifier.close()
            }),
            open_reader: Box::new(move || open(&open_dir, &open_exec)),
        })
    };

    let readers: Arc<Mutex<Vec<ReaderCouple>>> = Arc::new(Mutex::new(Vec::new()));
    let first_reader = open(&dir, &executor)?;
    let mut reader = Arc::clone(&first_reader);

    let mut threads: Vec<ReaderThread> = Vec::with_capacity(n as usize);
    // `Collections.synchronizedSet(new HashSet<DirectoryReader>())`: readers
    // compare by identity.
    let readers_to_close: Arc<Mutex<Vec<Arc<dyn DirectoryReader>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let add_to_close = |set: &Mutex<Vec<Arc<dyn DirectoryReader>>>,
                        r: &Arc<dyn DirectoryReader>| {
        let mut set = set.lock().unwrap_or_else(|e| e.into_inner());
        if !set.iter().any(|x| same_reader(x, r)) {
            set.push(Arc::clone(r));
        }
    };

    for i in 0..n {
        if i % 2 == 0 {
            if let Some(refreshed) = open_if_changed(&reader, &executor)? {
                add_to_close(&readers_to_close, &reader);
                reader = refreshed;
            }
        }
        let r = Arc::clone(&reader);

        let index = i;

        let stopped = Arc::new(AtomicBool::new(false));
        let task: Box<dyn FnOnce() -> Result<()> + Send> =
            if i < 4 || (10..14).contains(&i) || i > 18 {
                let stopped = Arc::clone(&stopped);
                let test = Arc::clone(&test);
                let readers = Arc::clone(&readers);
                let readers_to_close = Arc::clone(&readers_to_close);
                let executor = executor.clone();
                Box::new(move || -> Result<()> {
                    let rnd = LuceneTestCase::random();
                    while !stopped.load(Ordering::SeqCst) {
                        if index % 2 == 0 {
                            // refresh reader synchronized
                            let c = refresh_reader(&r, Some(&test), index, true)?;
                            add_to_close(
                                &readers_to_close,
                                c.new_reader.as_ref().unwrap_or_else(|| npe("newReader")),
                            );
                            add_to_close(&readers_to_close, &c.refreshed_reader);
                            readers.lock().unwrap_or_else(|e| e.into_inner()).push(c);
                            // prevent too many readers
                            break;
                        } else {
                            // not synchronized
                            let refreshed =
                                open_if_changed(&r, &executor)?.unwrap_or_else(|| Arc::clone(&r));

                            let searcher = LuceneTestCase::new_searcher(Arc::clone(&refreshed));
                            let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text(
                                "field1",
                                &format!("a{}", rnd.next_int_bound(refreshed.max_doc())),
                            )));
                            let hits = searcher.search(query, 1000)?.score_docs;
                            if !hits.is_empty() {
                                searcher.stored_fields()?.document(hits[0].doc)?;
                            }
                            if !same_reader(&refreshed, &r) {
                                refreshed.close()?;
                            }
                        }
                        // `synchronized (this) { wait(…); }`: nothing notifies this
                        // monitor, so the wait always times out.
                        Thread::sleep(i64::from(TestUtil::next_int(
                            &*LuceneTestCase::random(),
                            1,
                            100,
                        )));
                    }
                    Ok(())
                })
            } else {
                let stopped = Arc::clone(&stopped);
                let readers = Arc::clone(&readers);
                Box::new(move || -> Result<()> {
                    let rnd = LuceneTestCase::random();
                    while !stopped.load(Ordering::SeqCst) {
                        let c = {
                            let readers = readers.lock().unwrap_or_else(|e| e.into_inner());
                            let num_readers = readers.len() as i32;
                            if num_readers > 0 {
                                Some(readers[rnd.next_int_bound(num_readers) as usize].clone())
                            } else {
                                None
                            }
                        };
                        if let Some(c) = c {
                            assert_index_equals(
                                c.new_reader.as_ref().unwrap_or_else(|| npe("newReader")),
                                &c.refreshed_reader,
                            )?;
                        }

                        // `synchronized (this) { wait(…); }`: nothing notifies this
                        // monitor, so the wait always times out.
                        Thread::sleep(i64::from(TestUtil::next_int(
                            &*LuceneTestCase::random(),
                            1,
                            100,
                        )));
                    }
                    Ok(())
                })
            };

        let thread = ReaderThread::new(task, stopped);
        thread.thread.start();
        threads.push(thread);
    }

    // `synchronized (this) { wait(1000); }`.
    Thread::sleep(1000);

    for thread in &threads {
        thread.stop_thread();
    }

    for thread in &threads {
        if let Err(payload) = thread.thread.join() {
            std::panic::resume_unwind(payload);
        }
        if let Some(error) = thread
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let msg = format!(
                "Error occurred in thread {}:\n{}",
                thread.thread.name(),
                error
            );
            Assert::fail(&msg);
        }
    }

    let to_close = readers_to_close
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for reader_to_close in &to_close {
        reader_to_close.close()?;
    }

    first_reader.close()?;
    reader.close()?;

    for reader_to_close in &to_close {
        assert_reader_closed(reader_to_close, true);
    }

    assert_reader_closed(&reader, true);
    assert_reader_closed(&first_reader, true);

    drop(test);
    dir_holder.close()
}

/// `private static class ReaderCouple`.
#[derive(Clone)]
struct ReaderCouple {
    new_reader: Option<Arc<dyn DirectoryReader>>,
    refreshed_reader: Arc<dyn DirectoryReader>,
}

/// `private static class ReaderThread extends Thread`, with its
/// `ReaderThreadTask`.
struct ReaderThread {
    thread: Thread,
    /// The task's `volatile boolean stopped`.
    stopped: Arc<AtomicBool>,
    /// `Throwable error`: its `getMessage()`.
    error: Arc<Mutex<Option<String>>>,
}

impl ReaderThread {
    /// `ReaderThread(ReaderThreadTask task)`.
    fn new(task: Box<dyn FnOnce() -> Result<()> + Send>, stopped: Arc<AtomicBool>) -> Self {
        let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let thread = {
            let error = Arc::clone(&error);
            Thread::unnamed(move || {
                let outcome = std::panic::catch_unwind(AssertUnwindSafe(task));
                let failure = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(java_throwable(&e)),
                    Err(payload) => Some(
                        match payload.downcast::<randomizedtesting::jdk::exception::Throwable>() {
                            Ok(t) => *t,
                            Err(payload) => randomizedtesting::jdk::exception::Throwable::new(
                                "java.lang.AssertionError",
                                rucene::util::concurrent::panic_text(payload.as_ref()),
                            ),
                        },
                    ),
                };
                if let Some(r) = failure {
                    // `r.printStackTrace(System.out);`
                    println!("{r}");
                    *error.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(r.message().unwrap_or_default().to_string());
                }
            })
        };
        Self {
            thread,
            stopped,
            error,
        }
    }

    /// `public void stopThread()`: `task.stop()`.
    fn stop_thread(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

/// `private Object createReaderMutex = new Object();`.
static CREATE_READER_MUTEX: Mutex<()> = Mutex::new(());

/// `private ReaderCouple refreshReader(DirectoryReader reader, boolean
/// hasChanges)`.
fn refresh_reader_no_test(
    reader: &Arc<dyn DirectoryReader>,
    has_changes: bool,
) -> Result<ReaderCouple> {
    refresh_reader(reader, None, -1, has_changes)
}

/// `ReaderCouple refreshReader(DirectoryReader reader, TestReopen test, int
/// modify, boolean hasChanges)`.
fn refresh_reader(
    reader: &Arc<dyn DirectoryReader>,
    test: Option<&Reopen>,
    modify: i32,
    has_changes: bool,
) -> Result<ReaderCouple> {
    let _guard = CREATE_READER_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut r = None;
    if let Some(test) = test {
        test.modify_index(modify)?;
        r = Some(test.open_reader()?);
    }

    let refreshed = match directory_reader::open_if_changed(Arc::clone(reader)) {
        Ok(refreshed) => refreshed.unwrap_or_else(|| Arc::clone(reader)),
        Err(e) => {
            // finally: hit exception -- close opened reader
            if let Some(r) = &r {
                r.close()?;
            }
            return Err(e);
        }
    };

    if has_changes {
        if same_reader(&refreshed, reader) {
            Assert::fail("No new DirectoryReader instance created during refresh.");
        }
    } else if !same_reader(&refreshed, reader) {
        Assert::fail(
            "New DirectoryReader instance created during refresh even though index had no changes.",
        );
    }

    Ok(ReaderCouple {
        new_reader: r,
        refreshed_reader: refreshed,
    })
}

/// `public static void createIndex(Random random, Directory dir, boolean
/// multiSegment)`.
pub fn create_index(
    random: &dyn randomizedtesting::jdk::random::Random,
    dir: &Arc<dyn Directory>,
    multi_segment: bool,
) -> Result<()> {
    let mp: Arc<dyn MergePolicy> = if multi_segment {
        NoMergePolicy::instance()
    } else {
        Arc::new(LogMergePolicy::by_doc_count())
    };

    let iwc = LuceneTestCase::new_index_writer_config_with(
        random,
        Some(Arc::new(MockAnalyzer::new(random))),
    );
    iwc.set_merge_policy(Some(mp))?;
    let w = IndexWriter::new(Arc::clone(dir), iwc)?;

    for i in 0..100 {
        w.add_document(&create_document(i, 4)?)?;
        if multi_segment && (i % 10) == 0 {
            w.commit()?;
        }
    }

    if !multi_segment {
        w.force_merge(1)?;
    }

    w.close()?;

    let r = directory_reader::open(Arc::clone(dir))?;
    let leaves = (Arc::clone(&r) as Arc<dyn IndexReader>).leaves()?.len();
    if multi_segment {
        Assert::assert_true(leaves > 1);
    } else {
        Assert::assert_true(leaves == 1);
    }
    r.close()
}

/// `public static Document createDocument(int n, int numFields)`.
pub fn create_document(n: i32, num_fields: i32) -> Result<Document> {
    let mut sb = String::new();
    let mut doc = Document::new();
    sb.push('a');
    sb.push_str(&n.to_string());
    let custom_type2 = untokenized_stored_type(true)?;
    let custom_type3 = stored_only_type()?;
    doc.add(Box::new(TextField::new("field1", sb.clone(), Store::YES)?));
    doc.add(Box::new(Field::new("fielda", sb.clone(), custom_type2)?));
    doc.add(Box::new(Field::new("fieldb", sb.clone(), custom_type3)?));
    sb.push_str(" b");
    sb.push_str(&n.to_string());
    for i in 1..num_fields {
        doc.add(Box::new(TextField::new(
            &format!("field{}", i + 1),
            sb.clone(),
            Store::YES,
        )?));
    }
    Ok(doc)
}

/// `static void modifyIndex(int i, Directory dir)`.
pub fn modify_index(i: i32, dir: &Arc<dyn Directory>) -> Result<()> {
    match i {
        0 => {
            if *VERBOSE {
                println!("TEST: modify index");
            }
            let w = IndexWriter::new(Arc::clone(dir), no_merge_iwc()?)?;
            w.delete_documents_by_terms(vec![Term::from_text("field2", "a11")])?;
            w.delete_documents_by_terms(vec![Term::from_text("field2", "b30")])?;
            w.close()?;
        }
        1 => {
            let w = IndexWriter::new(Arc::clone(dir), plain_iwc())?;
            w.force_merge(1)?;
            w.close()?;
        }
        2 => {
            let w = IndexWriter::new(Arc::clone(dir), plain_iwc())?;
            w.add_document(&create_document(101, 4)?)?;
            w.force_merge(1)?;
            w.add_document(&create_document(102, 4)?)?;
            w.add_document(&create_document(103, 4)?)?;
            w.close()?;
        }
        3 => {
            let w = IndexWriter::new(Arc::clone(dir), no_merge_iwc()?)?;
            w.add_document(&create_document(101, 4)?)?;
            w.close()?;
        }
        _ => {}
    }
    Ok(())
}

/// `static void assertReaderClosed(IndexReader reader, boolean
/// checkSubReaders)`, for a `DirectoryReader`: a `CompositeReader` whose
/// sequential sub-readers are leaves, so the recursion checks their
/// reference count and stops.
fn assert_reader_closed(reader: &Arc<dyn DirectoryReader>, check_sub_readers: bool) {
    assert_equals(&0, &reader.get_ref_count());

    if check_sub_readers {
        // we cannot use reader context here, as reader is
        // already closed and calling getTopReaderContext() throws AlreadyClosed!
        let sub_readers = reader.get_sequential_sub_readers();
        for r in &sub_readers {
            assert_equals(&0, &r.get_ref_count());
        }
    }
}

/// `static class KeepAllCommits extends IndexDeletionPolicy`.
#[derive(Debug)]
struct KeepAllCommits;

impl IndexDeletionPolicy for KeepAllCommits {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDirectoryReaderReopen$KeepAllCommits"
    }

    fn on_init(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }

    fn on_commit(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }
}

/// The body `testReopenOnCommit` and `testReopenOnCommitWithExecutor`
/// share: only the first `open` takes the executor.
fn reopen_on_commit(dir: &Arc<dyn Directory>, executor: &Exec) -> Result<()> {
    let iwc = mock_iwc();
    iwc.set_index_deletion_policy(Some(Arc::new(KeepAllCommits)))?;
    iwc.set_max_buffered_docs(-1)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(dir), iwc)?;
    for i in 0..4 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::NO,
        )));
        writer.add_document(&doc)?;
        let mut data = HashMap::new();
        data.insert("index".to_string(), i.to_string());
        writer.set_live_commit_data(data)?;
        writer.commit()?;
    }
    for i in 0..4 {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
        let mut data = HashMap::new();
        data.insert("index".to_string(), (4 + i).to_string());
        writer.set_live_commit_data(data)?;
        writer.commit()?;
    }
    writer.close()?;

    let mut r = open(dir, executor)?;
    assert_equals(&0, &r.num_docs());

    let commits = directory_reader::list_commits(Arc::clone(dir))?;
    for commit in &commits {
        let r2 = directory_reader::open_if_changed_with_commit(Arc::clone(&r), commit.as_ref())?;
        assert_not_null(&r2);
        let r2 = r2.unwrap_or_else(|| npe("r2"));
        Assert::assert_true(!same_reader(&r2, &r));

        let s = commit.get_user_data()?;
        let v: i32 = if s.is_empty() {
            // First commit created by IW
            -1
        } else {
            s.get("index")
                .unwrap_or_else(|| npe("index"))
                .parse()
                .unwrap_or_else(|_| {
                    randomizedtesting::jdk::exception::Throwable::new(
                        "java.lang.NumberFormatException",
                        "index",
                    )
                    .throw()
                })
        };
        if v < 4 {
            assert_equals(&(1 + v), &r2.num_docs());
        } else {
            assert_equals(&(7 - v), &r2.num_docs());
        }
        r.close()?;
        r = r2;
    }
    r.close()
}

/// Port of `TestDirectoryReaderReopen.testReopenOnCommit`.
fn test_reopen_on_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    reopen_on_commit(&dir.as_directory(), &None)?;
    dir.close()
}

/// The body `testOpenIfChangedNRTToCommit` and its executor variant share.
fn open_if_changed_nrt_to_commit(dir: &Arc<dyn Directory>, executor: &Exec) -> Result<()> {
    // Can't use RIW because it randomly commits:
    let w = IndexWriter::new(Arc::clone(dir), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "value",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.commit()?;
    let commits = directory_reader::list_commits(Arc::clone(dir))?;
    assert_equals(&1, &commits.len());
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(w.as_ref())?;

    assert_equals(&2, &r.num_docs());
    let r2 = open_if_changed_commit(&r, commits[0].as_ref(), executor)?;
    assert_not_null(&r2);
    let r2 = r2.unwrap_or_else(|| npe("r2"));
    r.close()?;
    assert_equals(&1, &r2.num_docs());
    w.close()?;
    r2.close()
}

/// Port of `TestDirectoryReaderReopen.testOpenIfChangedNRTToCommit`.
fn test_open_if_changed_nrt_to_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    open_if_changed_nrt_to_commit(&dir.as_directory(), &None)?;
    dir.close()
}

/// The `MockDirectoryWrapper.Failure` of `testOverDecRefDuringReopen`: fail
/// once, when reopen tries to open the live docs file.
struct FailOnReadLiveDocs {
    failed: AtomicBool,
    /// `MockDirectoryWrapper.Failure.doFail`, unused by this failure.
    do_fail: AtomicBool,
}

impl Failure for FailOnReadLiveDocs {
    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }

    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.failed.load(Ordering::SeqCst) {
            return Ok(());
        }
        // System.out.println("failOn: ");
        // new Throwable().printStackTrace(System.out);
        if LuceneTestCase::call_stack_contains_any_of(&["read_live_docs"]) {
            if *VERBOSE {
                println!("TEST: now fail; exc:");
            }
            self.failed.store(true, Ordering::SeqCst);
            return Err(fake_io_exception());
        }
        Ok(())
    }
}

/// The body `testOverDecRefDuringReopen` and its executor variant share.
fn over_dec_ref_during_reopen(executor: &Exec) -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());

    let iwc = no_merge_iwc()?;
    iwc.set_codec(TestUtil::get_default_codec())?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "id",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "id2",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.commit()?;

    // Open reader w/ one segment w/ 2 docs:
    let r = open(&dir.as_directory(), executor)?;

    // Delete 1 doc from the segment:
    // System.out.println("TEST: now delete");
    w.delete_documents_by_terms(vec![Term::from_text("id", "id")])?;
    // System.out.println("TEST: now commit");
    w.commit()?;

    // Fail when reopen tries to open the live docs file:
    dir.as_mock()
        .unwrap_or_else(|| npe("MockDirectoryWrapper"))
        .fail_on(Arc::new(FailOnReadLiveDocs {
            failed: AtomicBool::new(false),
            do_fail: AtomicBool::new(false),
        }));

    // Now reopen:
    // System.out.println("TEST: now reopen");
    expect_throws(FAKE_IO_EXCEPTION, || open_if_changed(&r, executor));

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    assert_equals(
        &1,
        &s.count(Arc::new(TermQuery::new(Term::from_text("id", "id"))))?,
    );

    r.close()?;
    w.close()?;
    drop(s);
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testOverDecRefDuringReopen`.
fn test_over_dec_ref_during_reopen() -> Result<()> {
    over_dec_ref_during_reopen(&None)
}

/// Deletes every file of `dir`: `for (String fileName : dir.listAll())
/// dir.deleteFile(fileName);`.
fn blow_away(dir: &Arc<dyn Directory>) -> Result<()> {
    for file_name in dir.list_all()? {
        dir.delete_file(&file_name)?;
    }
    Ok(())
}

/// The first index `testNPEAfterInvalidReindex1` and `…2` write: one
/// segment, 2 docs, 1 deleted.
fn two_docs_one_deleted(dir: &Arc<dyn Directory>) -> Result<()> {
    let w = IndexWriter::new(Arc::clone(dir), no_merge_iwc()?)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "id",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "id2",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.delete_documents_by_terms(vec![Term::from_text("id", "id")])?;
    w.commit()?;
    w.close()
}

/// `doc.add(newStringField("id", id, NO)); doc.add(new
/// NumericDocValuesField("ndv", 13));`.
fn id_ndv_doc(id: &str) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        id,
        Store::NO,
    )));
    doc.add(Box::new(NumericDocValuesField::new("ndv", 13)));
    doc
}

/// `doc.add(newStringField("id", id, NO))`.
fn id_doc(id: &str) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        id,
        Store::NO,
    )));
    doc
}

/// The body `testNPEAfterInvalidReindex1` and its executor variant share.
fn npe_after_invalid_reindex1(executor: &Exec) -> Result<()> {
    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    two_docs_one_deleted(&dir)?;

    // Open reader w/ one segment w/ 2 docs, 1 deleted:
    let r = open(&dir, executor)?;

    // Blow away the index:
    blow_away(&dir)?;

    let w = IndexWriter::new(Arc::clone(&dir), plain_iwc())?;
    w.add_document(&id_ndv_doc("id"))?;
    w.add_document(&id_doc("id2"))?;
    w.commit()?;
    w.add_document(&id_doc("id2"))?;
    w.update_numeric_doc_value(Term::from_text("id", "id"), "ndv", 17)?;
    w.commit()?;
    w.close()?;

    expect_throws("java.lang.IllegalStateException", || {
        open_if_changed(&r, executor)
    });

    r.close()?;
    if executor.is_none() {
        w.close()?;
    }
    drop((r, w));
    close_dir(&mut dir)
}

/// `dir.close()` on a `Directory` the test shares as `Arc`: closed once
/// the readers and writers that held it are dropped, as the ported store
/// tests do.
fn close_dir(dir: &mut Arc<dyn Directory>) -> Result<()> {
    Arc::get_mut(dir).map_or(Ok(()), |d| d.close())
}

/// Port of `TestDirectoryReaderReopen.testNPEAfterInvalidReindex1`.
fn test_npe_after_invalid_reindex1() -> Result<()> {
    npe_after_invalid_reindex1(&None)
}

/// The body `testNPEAfterInvalidReindex2` and its executor variant share.
fn npe_after_invalid_reindex2(executor: &Exec) -> Result<()> {
    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    two_docs_one_deleted(&dir)?;

    // Open reader w/ one segment w/ 2 docs, 1 deleted:
    let r = open(&dir, executor)?;

    // Blow away the index:
    blow_away(&dir)?;

    let w = IndexWriter::new(Arc::clone(&dir), plain_iwc())?;
    w.add_document(&id_ndv_doc("id"))?;
    w.commit()?;
    w.add_document(&id_doc("id2"))?;
    w.commit()?;
    w.close()?;

    expect_throws("java.lang.IllegalStateException", || {
        open_if_changed(&r, executor)
    });

    r.close()?;
    drop((r, w));
    close_dir(&mut dir)
}

/// Port of `TestDirectoryReaderReopen.testNPEAfterInvalidReindex2`.
fn test_npe_after_invalid_reindex2() -> Result<()> {
    npe_after_invalid_reindex2(&None)
}

/// `new StringField("key", value, Field.Store.YES)` alone in a document.
fn key_doc(value: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "key",
        value.to_string(),
        Store::YES,
    )?));
    Ok(doc)
}

/// `assertSame(a.leaves().get(0).reader().getCoreCacheHelper().getKey(),
/// b.leaves().get(0).reader().getCoreCacheHelper().getKey())`.
fn assert_same_first_core(
    a: &Arc<dyn DirectoryReader>,
    b: &Arc<dyn DirectoryReader>,
) -> Result<()> {
    let core = |r: &Arc<dyn DirectoryReader>| -> Result<Box<dyn CacheHelper>> {
        let leaves = (Arc::clone(r) as Arc<dyn IndexReader>).leaves()?;
        Ok(leaves[0]
            .leaf_reader()
            .get_core_cache_helper()
            .unwrap_or_else(|| npe("getCoreCacheHelper()")))
    };
    let (ca, cb) = (core(a)?, core(b)?);
    Assert::assert_true(ca.get_key() == cb.get_key());
    Ok(())
}

/// `r.leaves().size()`.
fn leaf_count(r: &Arc<dyn DirectoryReader>) -> Result<usize> {
    Ok((Arc::clone(r) as Arc<dyn IndexReader>).leaves()?.len())
}

/// The snapshotting writer config of the `testNRTM*` tests.
fn snapshot_iwc(no_merge: bool) -> Result<(IndexWriterConfig, Arc<SnapshotDeletionPolicy>)> {
    let iwc = if no_merge {
        no_merge_iwc()?
    } else {
        plain_iwc()
    };
    let snapshotter = Arc::new(SnapshotDeletionPolicy::new(Some(Arc::new(
        KeepOnlyLastCommitDeletionPolicy,
    ))));
    iwc.set_index_deletion_policy(Some(Arc::clone(&snapshotter) as _))?;
    Ok((iwc, snapshotter))
}

/// test reopening backwards from a non-NRT reader (with document deletes)
///
/// The body of `testNRTMdeletes` and its executor variant.
fn nrtm_deletes(executor: &Exec) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (iwc, snapshotter) = snapshot_iwc(true)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.commit()?; // make sure all index metadata is written out

    writer.add_document(&key_doc("value1")?)?;

    writer.add_document(&key_doc("value2")?)?;

    writer.commit()?;

    let ic1 = snapshotter.snapshot()?;

    writer.update_document(Term::from_text("key", "value1"), &key_doc("value3")?)?;

    writer.commit()?;

    let ic2 = snapshotter.snapshot()?;
    let latest = match executor {
        None => directory_reader::open_with_commit(ic2.as_ref())?,
        Some(e) => {
            directory_reader::open_with_commit_and_executor(ic2.as_ref(), Some(Arc::clone(e)))?
        }
    };
    assert_equals(&2, &leaf_count(&latest)?);

    // This reader will be used for searching against commit point 1
    let oldest =
        open_if_changed_commit(&latest, ic1.as_ref(), executor)?.unwrap_or_else(|| npe("oldest"));
    assert_equals(&1, &leaf_count(&oldest)?);

    // sharing same core
    assert_same_first_core(&latest, &oldest)?;

    latest.close()?;
    oldest.close()?;

    snapshotter.release(ic1.as_ref())?;
    snapshotter.release(ic2.as_ref())?;
    writer.close()?;
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testNRTMdeletes`.
fn test_nrtm_deletes() -> Result<()> {
    nrtm_deletes(&None)
}

/// test reopening backwards from an NRT reader (with document deletes)
///
/// The body of `testNRTMdeletes2` and its executor variant.
fn nrtm_deletes2(executor: &Exec) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (iwc, snapshotter) = snapshot_iwc(true)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.commit()?; // make sure all index metadata is written out

    writer.add_document(&key_doc("value1")?)?;

    writer.add_document(&key_doc("value2")?)?;

    writer.commit()?;

    let ic1 = snapshotter.snapshot()?;

    writer.update_document(Term::from_text("key", "value1"), &key_doc("value3")?)?;

    let latest = directory_reader::open_with_writer(writer.as_ref())?;
    assert_equals(&2, &leaf_count(&latest)?);

    // This reader will be used for searching against commit point 1
    let oldest =
        open_if_changed_commit(&latest, ic1.as_ref(), executor)?.unwrap_or_else(|| npe("oldest"));

    // This reader should not see the deletion:
    assert_equals(&2, &oldest.num_docs());
    Assert::assert_false(oldest.has_deletions());

    snapshotter.release(ic1.as_ref())?;
    assert_equals(&1, &leaf_count(&oldest)?);

    // sharing same core
    assert_same_first_core(&latest, &oldest)?;

    latest.close()?;
    oldest.close()?;

    writer.close()?;
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testNRTMdeletes2`.
fn test_nrtm_deletes2() -> Result<()> {
    nrtm_deletes2(&None)
}

/// `getOnlyLeafReader(r).getNumericDocValues("dv")`: its first document
/// is 0 and holds `expected`.
fn assert_dv(r: &Arc<dyn DirectoryReader>, expected: i64) -> Result<()> {
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(r) as Arc<dyn IndexReader>)?;
    let mut values = leaf
        .get_numeric_doc_values("dv")?
        .unwrap_or_else(|| npe("getNumericDocValues(\"dv\")"));
    assert_equals(&0, &values.next_doc()?);
    assert_equals(&expected, &values.long_value()?);
    Ok(())
}

/// A document with `key=value1` and the numeric doc value `dv=1`.
fn key_dv_doc() -> Result<Document> {
    let mut doc = key_doc("value1")?;
    doc.add(Box::new(NumericDocValuesField::new("dv", 1)));
    Ok(doc)
}

/// test reopening backwards from a non-NRT reader with DV updates
///
/// The body of `testNRTMupdates` and its executor variant.
fn nrtm_updates(executor: &Exec) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (iwc, snapshotter) = snapshot_iwc(false)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.commit()?; // make sure all index metadata is written out

    writer.add_document(&key_dv_doc()?)?;

    writer.commit()?;

    let ic1 = snapshotter.snapshot()?;

    writer.update_numeric_doc_value(Term::from_text("key", "value1"), "dv", 2)?;

    writer.commit()?;

    let ic2 = snapshotter.snapshot()?;
    let latest = match executor {
        None => directory_reader::open_with_commit(ic2.as_ref())?,
        Some(e) => {
            directory_reader::open_with_commit_and_executor(ic2.as_ref(), Some(Arc::clone(e)))?
        }
    };
    assert_equals(&1, &leaf_count(&latest)?);

    // This reader will be used for searching against commit point 1
    let oldest =
        open_if_changed_commit(&latest, ic1.as_ref(), executor)?.unwrap_or_else(|| npe("oldest"));
    assert_equals(&1, &leaf_count(&oldest)?);

    // sharing same core
    assert_same_first_core(&latest, &oldest)?;

    assert_dv(&oldest, 1)?;

    assert_dv(&latest, 2)?;

    latest.close()?;
    oldest.close()?;

    snapshotter.release(ic1.as_ref())?;
    snapshotter.release(ic2.as_ref())?;
    writer.close()?;
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testNRTMupdates`.
fn test_nrtm_updates() -> Result<()> {
    nrtm_updates(&None)
}

/// test reopening backwards from an NRT reader with DV updates
///
/// The body of `testNRTMupdates2` and its executor variant.
fn nrtm_updates2(executor: &Exec) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let (iwc, snapshotter) = snapshot_iwc(false)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.commit()?; // make sure all index metadata is written out

    writer.add_document(&key_dv_doc()?)?;

    writer.commit()?;

    let ic1 = snapshotter.snapshot()?;

    writer.update_numeric_doc_value(Term::from_text("key", "value1"), "dv", 2)?;

    let latest = directory_reader::open_with_writer(writer.as_ref())?;
    assert_equals(&1, &leaf_count(&latest)?);

    // This reader will be used for searching against commit point 1
    let oldest =
        open_if_changed_commit(&latest, ic1.as_ref(), executor)?.unwrap_or_else(|| npe("oldest"));
    assert_equals(&1, &leaf_count(&oldest)?);

    // sharing same core
    assert_same_first_core(&latest, &oldest)?;

    assert_dv(&oldest, 1)?;

    assert_dv(&latest, 2)?;

    latest.close()?;
    oldest.close()?;

    snapshotter.release(ic1.as_ref())?;
    writer.close()?;
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testNRTMupdates2`.
fn test_nrtm_updates2() -> Result<()> {
    nrtm_updates2(&None)
}

/// `doc.add(newStringField("field", value, NO))`.
fn field_doc(value: &str) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        value,
        Store::NO,
    )));
    doc
}

// LUCENE-5931: we make a "best effort" to catch this abuse and throw a clear(er)
// exception than what would otherwise look like hard to explain index corruption during searching
/// The body of `testDeleteIndexFilesWhileReaderStillOpen` and its executor
/// variant (which also closes the reader and the directory).
fn delete_index_files_while_reader_still_open(executor: &Exec) -> Result<()> {
    let mut dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let w = IndexWriter::new(Arc::clone(&dir), plain_iwc())?;
    w.add_document(&field_doc("value"))?;
    // Creates single segment index:
    w.close()?;

    let r = open(&dir, executor)?;

    // Abuse: remove all files while reader is open; one is supposed to use IW.deleteAll, or open a
    // new IW with OpenMode.CREATE instead:
    blow_away(&dir)?;

    let w = IndexWriter::new(Arc::clone(&dir), no_merge_iwc()?)?;
    w.add_document(&field_doc("value"))?;

    let doc = field_doc("value2");
    w.add_document(&doc)?;

    // Writes same segment, this time with two documents:
    w.commit()?;

    w.delete_documents_by_terms(vec![Term::from_text("field", "value2")])?;

    w.add_document(&doc)?;

    // Writes another segments file, so openIfChanged sees that the index has in fact changed:
    w.close()?;

    expect_throws("java.lang.IllegalStateException", || {
        open_if_changed(&r, executor)
    });

    if executor.is_some() {
        r.close()?;
        drop((r, w));
        close_dir(&mut dir)?;
    }
    Ok(())
}

/// Port of `TestDirectoryReaderReopen.testDeleteIndexFilesWhileReaderStillOpen`.
fn test_delete_index_files_while_reader_still_open() -> Result<()> {
    delete_index_files_while_reader_still_open(&None)
}

/// `new StringField(name, value, Field.Store.YES)`.
fn stored_string(name: &str, value: &str) -> Result<Box<StringField>> {
    Ok(Box::new(StringField::new(
        name,
        value.to_string(),
        Store::YES,
    )?))
}

/// `assertNotSame(a, b)` on two readers.
fn assert_not_same(a: &Option<Arc<dyn DirectoryReader>>, b: &Arc<dyn DirectoryReader>) {
    if a.as_ref().is_some_and(|a| same_reader(a, b)) {
        Assert::fail("expected not same");
    }
}

/// The body of `testReuseUnchangedLeafReaderOnDVUpdate` and its executor
/// variant.
fn reuse_unchanged_leaf_reader_on_dv_update(executor: &Exec) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_writer_config = LuceneTestCase::new_index_writer_config();
    index_writer_config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;

    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    doc.add(stored_string("version", "1")?);
    doc.add(Box::new(NumericDocValuesField::new("some_docvalue", 2)));
    writer.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(stored_string("id", "2")?);
    doc.add(stored_string("version", "1")?);
    writer.add_document(&doc)?;
    writer.commit()?;
    let mut reader = open(&dir.as_directory(), executor)?;
    assert_equals(&2, &reader.num_docs());
    assert_equals(&2, &reader.max_doc());
    assert_equals(&0, &reader.num_deleted_docs());

    // The document Java builds here is never added.
    let mut doc = Document::new();
    doc.add(stored_string("id", "1")?);
    doc.add(stored_string("version", "2")?);
    drop(doc);
    writer.update_doc_values(
        Term::from_text("id", "1"),
        &[Field::new_with_number(
            "some_docvalue",
            NumericValue::Long(1),
            NumericDocValuesField::type_().clone(),
        )?],
    )?;
    writer.commit()?;
    let new_reader = open_if_changed(&reader, executor)?;
    assert_not_same(&new_reader, &reader);
    reader.close()?;
    reader = new_reader.unwrap_or_else(|| npe("newReader"));
    assert_equals(&2, &reader.num_docs());
    assert_equals(&2, &reader.max_doc());
    assert_equals(&0, &reader.num_deleted_docs());

    let mut doc = Document::new();
    doc.add(stored_string("id", "3")?);
    doc.add(stored_string("version", "3")?);
    writer.update_document(Term::from_text("id", "3"), &doc)?;
    writer.commit()?;

    let new_reader = open_if_changed(&reader, executor)?;
    assert_not_same(&new_reader, &reader);
    let new_reader = new_reader.unwrap_or_else(|| npe("newReader"));
    assert_equals(&2, &new_reader.get_sequential_sub_readers().len());
    assert_equals(&1, &reader.get_sequential_sub_readers().len());
    Assert::assert_true(std::ptr::addr_eq(
        Arc::as_ptr(&reader.get_sequential_sub_readers()[0]),
        Arc::as_ptr(&new_reader.get_sequential_sub_readers()[0]),
    ));
    reader.close()?;
    reader = new_reader;
    assert_equals(&3, &reader.num_docs());
    assert_equals(&3, &reader.max_doc());
    assert_equals(&0, &reader.num_deleted_docs());
    // `IOUtils.close(reader, writer, dir)`.
    let dir_handle = &mut dir;
    IOUtils::close([
        Some(Box::new(|| reader.close()) as Box<dyn FnOnce() -> Result<()>>),
        Some(Box::new(|| writer.close())),
        Some(Box::new(|| dir_handle.close())),
    ])
}

/// Port of `TestDirectoryReaderReopen.testReuseUnchangedLeafReaderOnDVUpdate`.
fn test_reuse_unchanged_leaf_reader_on_dv_update() -> Result<()> {
    reuse_unchanged_leaf_reader_on_dv_update(&None)
}

/// Port of `TestDirectoryReaderReopen.testReopenWithExecutor`.
fn test_reopen_with_executor() -> Result<()> {
    let executor_service1 = new_fixed_thread_pool(4);
    reopen_body(Some(&executor_service1), false)?;
    let executor_service2 = new_fixed_thread_pool(4);
    reopen_body(Some(&executor_service2), true)
}

/// Port of `TestDirectoryReaderReopen.testCommitReopenWithExecutor`.
fn test_commit_reopen_with_executor() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    with_executor(|e| {
        do_test_reopen_with_commit_executor(&*random(), &dir.as_directory(), true, e)
    })?;
    dir.close()
}

/// `private void doTestReopenWithCommit(Random random, Directory dir,
/// boolean withReopen, ExecutorService executorService)`.
fn do_test_reopen_with_commit_executor(
    random: &dyn randomizedtesting::jdk::random::Random,
    dir: &Arc<dyn Directory>,
    with_reopen: bool,
    executor_service: &Arc<ThreadPoolExecutor>,
) -> Result<()> {
    let executor = exec(executor_service);
    let iwriter = IndexWriter::new(Arc::clone(dir), reopen_with_commit_iwc(random)?)?;
    iwriter.commit()?;
    let mut reader = open(dir, &executor)?;

    let body = (|| -> Result<()> {
        let m = 3;
        let custom_type = untokenized_stored_type(false)?;
        let custom_type2 = untokenized_stored_type(true)?;
        let custom_type3 = stored_only_type()?;
        for i in 0..4 {
            for j in 0..m {
                let mut doc = Document::new();
                let id = format!("{i}_{j}");
                doc.add(Box::new(LuceneTestCase::new_field("id", &id, &custom_type)));
                doc.add(Box::new(LuceneTestCase::new_field(
                    "id2",
                    &id,
                    &custom_type2,
                )));
                doc.add(Box::new(LuceneTestCase::new_field(
                    "id3",
                    &id,
                    &custom_type3,
                )));

                if i > 0 {
                    let k = i - 1;
                    let n = j + k * m;
                    let prev_itereation_doc = reader.stored_fields()?.document(n)?;
                    let id = prev_itereation_doc.get("id");
                    assert_equals(&Some(format!("{k}_{j}")), &id);
                }

                iwriter.add_document(&doc)?;
            }
            iwriter.commit()?;
            if with_reopen {
                if let Some(r2) = open_if_changed(&reader, &executor)? {
                    reader.close()?;
                    reader = r2;
                }
            } else {
                reader.close()?;
                reader = open(dir, &executor)?;
            }
        }
        Ok(())
    })();
    // finally
    iwriter.close()?;
    reader.close()?;
    body
}

/// Port of `TestDirectoryReaderReopen.testCommitRecreateWithExecutor`.
fn test_commit_recreate_with_executor() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    with_executor(|e| {
        do_test_reopen_with_commit_executor(&*random(), &dir.as_directory(), false, e)
    })?;
    dir.close()
}

/// Port of `TestDirectoryReaderReopen.testThreadSafetyWithExecutor`.
fn test_thread_safety_with_executor() -> Result<()> {
    with_executor(|e| thread_safety(exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testReopenOnCommitWithExecutor`.
fn test_reopen_on_commit_with_executor() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    with_executor(|e| {
        reopen_on_commit(&dir.as_directory(), &exec(e))?;
        dir.close()
    })
}

/// Port of `TestDirectoryReaderReopen.testOpenIfChangedNRTToCommitWithExecutor`.
fn test_open_if_changed_nrt_to_commit_with_executor() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    with_executor(|e| {
        open_if_changed_nrt_to_commit(&dir.as_directory(), &exec(e))?;
        dir.close()
    })
}

/// Port of `TestDirectoryReaderReopen.testOverDecRefDuringReopenWithExecutor`.
fn test_over_dec_ref_during_reopen_with_executor() -> Result<()> {
    with_executor(|e| over_dec_ref_during_reopen(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testNPEAfterInvalidReindex1WithExecutor`.
fn test_npe_after_invalid_reindex1_with_executor() -> Result<()> {
    with_executor(|e| npe_after_invalid_reindex1(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testNPEAfterInvalidReindex2WithExecutor`.
fn test_npe_after_invalid_reindex2_with_executor() -> Result<()> {
    with_executor(|e| npe_after_invalid_reindex2(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testNRTMdeletesWithExecutor`.
fn test_nrtm_deletes_with_executor() -> Result<()> {
    with_executor(|e| nrtm_deletes(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testNRTMdeletes2WithExecutor`.
fn test_nrtm_deletes2_with_executor() -> Result<()> {
    with_executor(|e| nrtm_deletes2(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testNRTMupdatesWithExecutor`.
fn test_nrtm_updates_with_executor() -> Result<()> {
    with_executor(|e| nrtm_updates(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testNRTMupdates2WithExecutor`.
fn test_nrtm_updates2_with_executor() -> Result<()> {
    with_executor(|e| nrtm_updates2(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testDeleteIndexFilesWhileReaderStillOpenWithExecutor`.
fn test_delete_index_files_while_reader_still_open_with_executor() -> Result<()> {
    with_executor(|e| delete_index_files_while_reader_still_open(&exec(e)))
}

/// Port of `TestDirectoryReaderReopen.testReuseUnchangedLeafReaderOnDVUpdateWithExecutor`.
fn test_reuse_unchanged_leaf_reader_on_dv_update_with_executor() -> Result<()> {
    with_executor(|e| reuse_unchanged_leaf_reader_on_dv_update(&exec(e)))
}
