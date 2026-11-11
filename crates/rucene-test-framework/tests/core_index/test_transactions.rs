//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTransactions.java`.
//!
//! The `TimedThread` subclasses are one thread body, [`timed_thread`], over a
//! `doWork` closure; each thread's `failed` flag is shared with the test and
//! with the other threads (`allThreads`).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, Store, StringField};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::terms::Term;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::mock_directory_wrapper::Failure;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::{English, LuceneTestCase};

use crate::core_support::{class, random, test, verbose};
use crate::support::as_cms;

/// `TestTransactions`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTransactions",
        vec![test("testTransactions", test_transactions).annotate(Nightly)],
    )
}

/// `private class RandomFailure extends MockDirectoryWrapper.Failure`, over
/// the test's `private volatile boolean doFail`.
struct RandomFailure {
    test_do_fail: Arc<AtomicBool>,
    do_fail: AtomicBool,
}

impl Failure for RandomFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.test_do_fail.load(Ordering::SeqCst) && random().next_int_bound(10) <= 3 {
            if verbose() {
                println!("{} TEST: now fail on purpose", Thread::current().name());
                println!("{}", std::backtrace::Backtrace::force_capture());
            }
            return Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other("now failing randomly but on purpose"),
            )));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// `TimedThread.MAX_ITERATIONS = atLeast(100)`, a static of the nested class.
fn max_iterations() -> i32 {
    static MAX_ITERATIONS: OnceLock<i32> = OnceLock::new();
    *MAX_ITERATIONS.get_or_init(|| LuceneTestCase::at_least(100))
}

/// `private abstract static class TimedThread extends Thread`: runs
/// `do_work` until `MAX_ITERATIONS` or until any thread failed; a throwable
/// sets `failed`.
fn timed_thread(
    failed: Arc<AtomicBool>,
    all_threads: Arc<Vec<Arc<AtomicBool>>>,
    mut do_work: impl FnMut() -> Result<()> + Send + 'static,
) -> Thread {
    Thread::unnamed(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            let mut iterations = 0;
            loop {
                iterations += 1;
                println!("{iterations}");
                // anyErrors()
                if all_threads.iter().any(|t| t.load(Ordering::SeqCst)) {
                    break;
                }
                do_work()?;
                if iterations >= max_iterations() {
                    break;
                }
            }
            Ok(())
        }));
        let e = match outcome {
            Ok(Ok(())) => return,
            Ok(Err(e)) => java_throwable(&e),
            Err(payload) => randomizedtesting::jdk::exception::Throwable::from_panic(payload),
        };
        println!("{}: exc", Thread::current());
        println!("{e}");
        failed.store(true, Ordering::SeqCst);
    })
}

/// `IndexerThread.update(writer)`.
fn update(writer: &IndexWriter, next_id: &AtomicI32) -> Result<()> {
    // Add 10 docs:
    let custom_type = FieldType::new_from(StringField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;
    for _ in 0..10 {
        let mut d = Document::new();
        let n = random().next_int();
        d.add(Box::new(LuceneTestCase::new_field(
            "id",
            &next_id.fetch_add(1, Ordering::SeqCst).to_string(),
            &custom_type,
        )));
        d.add(Box::new(LuceneTestCase::new_text_field(
            "contents",
            &English::int_to_english(n),
            Store::NO,
        )));
        writer.add_document(&d)?;
    }

    // Delete 5 docs:
    let mut delete_id = next_id.load(Ordering::SeqCst) - 1;
    for _ in 0..5 {
        writer.delete_documents_by_terms(vec![Term::from_text("id", &delete_id.to_string())])?;
        delete_id -= 2;
    }
    Ok(())
}

/// `new IndexWriter(dir, newIndexWriterConfig(new MockAnalyzer(random()))
/// .setMaxBufferedDocs(max_buffered_docs).setMergeScheduler(new
/// ConcurrentMergeScheduler()).setMergePolicy(newLogMergePolicy(merge_factor)))`,
/// with `setSuppressExceptions()` on its scheduler.
fn indexer_writer(
    dir: &Arc<dyn Directory>,
    max_buffered_docs: i32,
    merge_factor: i32,
) -> Result<Arc<IndexWriter>> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_max_buffered_docs(max_buffered_docs)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(merge_factor)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(dir), iwc)?;
    as_cms(&writer.get_config().merge_scheduler()).set_suppress_exceptions();
    Ok(writer)
}

/// `IndexerThread.doWork()`.
fn indexer_do_work(
    lock: &Mutex<()>,
    do_fail: &AtomicBool,
    dir1: &Arc<dyn Directory>,
    dir2: &Arc<dyn Directory>,
    next_id: &AtomicI32,
) -> Result<()> {
    let writer1 = indexer_writer(dir1, 3, 2)?;

    // Intentionally use different params so flush/merge
    // happen @ different times
    let writer2 = indexer_writer(dir2, 2, 3)?;

    update(&writer1, next_id)?;
    update(&writer2, next_id)?;

    do_fail.store(true, Ordering::SeqCst);
    let result = (|| -> Result<bool> {
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let prepared =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer1.prepare_commit()));
        if !matches!(prepared, Ok(Ok(_))) {
            // release resources
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer1.rollback()));
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer2.rollback()));
            return Ok(false);
        }
        let prepared =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer2.prepare_commit()));
        if !matches!(prepared, Ok(Ok(_))) {
            // release resources
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer1.rollback()));
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer2.rollback()));
            return Ok(false);
        }

        writer1.commit()?;
        writer2.commit()?;
        Ok(true)
    })();
    // finally
    do_fail.store(false, Ordering::SeqCst);
    if !result? {
        return Ok(());
    }

    writer1.close()?;
    writer2.close()
}

/// `SearcherThread.doWork()`.
fn searcher_do_work(
    lock: &Mutex<()>,
    dir1: &Arc<dyn Directory>,
    dir2: &Arc<dyn Directory>,
) -> Result<()> {
    let (r1, r2) = {
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut r1 = None;
        let opened = (|| -> Result<_> {
            r1 = Some(directory_reader::open(Arc::clone(dir1))?);
            directory_reader::open(Arc::clone(dir2))
        })();
        match opened {
            Ok(r2) => (r1.expect("opened first"), r2),
            Err(e) => {
                // can be rethrown as RuntimeException if it happens during a close listener
                let message = java_throwable(&e).message().unwrap_or_default().to_string();
                if !message.contains("on purpose") {
                    return Err(e);
                }
                // release resources
                if let Some(r1) = r1 {
                    let _ = r1.close();
                }
                return Ok(());
            }
        }
    };
    if r1.num_docs() != r2.num_docs() {
        return Err(LuceneError::unchecked(
            "java.lang.RuntimeException",
            format!(
                "doc counts differ: r1={} r2={}",
                r1.num_docs(),
                r2.num_docs()
            ),
        ));
    }
    let _ = r1.close();
    let _ = r2.close();
    Ok(())
}

/// `public void initIndex(Directory dir)`.
fn init_index(dir: &Arc<dyn Directory>) -> Result<()> {
    let writer = IndexWriter::new(
        Arc::clone(dir),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    for _ in 0..7 {
        let mut d = Document::new();
        let n = random().next_int();
        d.add(Box::new(LuceneTestCase::new_text_field(
            "contents",
            &English::int_to_english(n),
            Store::NO,
        )));
        writer.add_document(&d)?;
    }
    writer.close()
}

/// Port of `TestTransactions.testTransactions`.
fn test_transactions() -> Result<()> {
    let do_fail = Arc::new(AtomicBool::new(false));
    // we cant use non-ramdir on windows, because this test needs to double-write.
    let mut dir1 = MockDirectoryWrapper::new(&*random(), Arc::new(ByteBuffersDirectory::new()));
    let mut dir2 = MockDirectoryWrapper::new(&*random(), Arc::new(ByteBuffersDirectory::new()));
    dir1.fail_on(Arc::new(RandomFailure {
        test_do_fail: Arc::clone(&do_fail),
        do_fail: AtomicBool::new(false),
    }));
    dir2.fail_on(Arc::new(RandomFailure {
        test_do_fail: Arc::clone(&do_fail),
        do_fail: AtomicBool::new(false),
    }));
    dir1.set_fail_on_open_input(false);
    dir2.set_fail_on_open_input(false);

    // We throw exceptions in deleteFile, which creates
    // leftover files:
    dir1.set_assert_no_unrefenced_files_on_close(false);
    dir2.set_assert_no_unrefenced_files_on_close(false);

    let d1 = dir1.as_directory();
    let d2 = dir2.as_directory();
    init_index(&d1)?;
    init_index(&d2)?;

    let lock = Arc::new(Mutex::new(()));
    let failed: Vec<Arc<AtomicBool>> = (0..3).map(|_| Arc::new(AtomicBool::new(false))).collect();
    let all_threads = Arc::new(failed.clone());
    let mut threads: Vec<Thread> = Vec::new();

    let indexer_thread = {
        let (lock, do_fail, d1, d2) = (
            Arc::clone(&lock),
            Arc::clone(&do_fail),
            Arc::clone(&d1),
            Arc::clone(&d2),
        );
        let next_id = AtomicI32::new(0);
        timed_thread(
            Arc::clone(&failed[0]),
            Arc::clone(&all_threads),
            move || indexer_do_work(&lock, &do_fail, &d1, &d2, &next_id),
        )
    };
    threads.push(indexer_thread.clone());
    indexer_thread.start();

    for i in 1..3 {
        let (lock, d1, d2) = (Arc::clone(&lock), Arc::clone(&d1), Arc::clone(&d2));
        let searcher_thread = timed_thread(
            Arc::clone(&failed[i]),
            Arc::clone(&all_threads),
            move || searcher_do_work(&lock, &d1, &d2),
        );
        threads.push(searcher_thread.clone());
        searcher_thread.start();
    }

    for thread in &threads {
        let _ = thread.join();
    }

    for f in &failed {
        Assert::assert_true(!f.load(Ordering::SeqCst));
    }
    drop((d1, d2));
    dir1.close()?;
    dir2.close()
}
