//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTragicIndexWriterDeadlock.java`.
//!
//! The anonymous `ConcurrentMergeScheduler` subclass is the scheduler's
//! override hooks (`doMerge`, `doStall`, `handleMergeException`); the
//! anonymous `IndexWriter` subclass is an `IndexWriterOverrides` whose
//! `mergeSuccess` throws.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::{IndexWriter, IndexWriterOverrides};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{LogMergePolicy, OneMerge};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::store::Directory;
use rucene_test_framework::index::suppressing_concurrent_merge_scheduler::SuppressingConcurrentMergeScheduler;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, test};
use crate::support::CountDownLatch;

/// `TestTragicIndexWriterDeadlock`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTragicIndexWriterDeadlock",
        vec![
            test(
                "testDeadlockExcNRTReaderCommit",
                test_deadlock_exc_nrt_reader_commit,
            ),
            test("testDeadlockStalledMerges", test_deadlock_stalled_merges),
            test(
                "testDeadlockStalledFullFlushMerges",
                test_deadlock_stalled_full_flush_merges,
            ),
        ],
    )
}

/// Port of `TestTragicIndexWriterDeadlock.testDeadlockExcNRTReaderCommit`.
fn test_deadlock_exc_nrt_reader_commit() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    if iwc
        .merge_scheduler()
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
        .is_some()
    {
        // `isOK(Throwable th)` returns `true`.
        iwc.set_merge_scheduler(Some(Arc::new(SuppressingConcurrentMergeScheduler::create(
            Arc::new(|_th| true),
        ))))?;
    }
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let done = Arc::new(AtomicBool::new(false));
    let commit_thread = {
        let (w, starting_gun, done) =
            (Arc::clone(&w), Arc::clone(&starting_gun), Arc::clone(&done));
        Thread::unnamed(move || {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    starting_gun.await_();
                    while !done.load(Ordering::SeqCst) {
                        w.add_document(&Document::new())?;
                        w.commit()?;
                    }
                    Ok(())
                }));
            if !matches!(outcome, Ok(Ok(()))) {
                done.store(true, Ordering::SeqCst);
                // System.out.println("commit exc:");
                // t.printStackTrace(System.out);
            }
        })
    };
    commit_thread.start();
    let r0: Arc<dyn DirectoryReader> = directory_reader::open_with_writer(&*w)?;
    let nrt_thread = {
        let (starting_gun, done) = (Arc::clone(&starting_gun), Arc::clone(&done));
        Thread::unnamed(move || {
            let mut r = r0;
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let looped =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                            starting_gun.await_();
                            while !done.load(Ordering::SeqCst) {
                                let old_reader = Arc::clone(&r);
                                let r2 =
                                    directory_reader::open_if_changed(Arc::clone(&old_reader))?;
                                if let Some(r2) = r2 {
                                    r = r2;
                                    old_reader.dec_ref()?;
                                }
                            }
                            Ok(())
                        }));
                    // finally { r.close(); }
                    let closed = r.close();
                    match looped {
                        Ok(result) => result.and(closed),
                        Err(payload) => std::panic::resume_unwind(payload),
                    }
                }));
            if !matches!(outcome, Ok(Ok(()))) {
                done.store(true, Ordering::SeqCst);
                // System.out.println("nrt exc:");
                // t.printStackTrace(System.out);
            }
        })
    };
    nrt_thread.start();
    dir.set_random_io_exception_rate(0.1);
    starting_gun.count_down();
    let _ = commit_thread.join();
    let _ = nrt_thread.join();
    dir.set_random_io_exception_rate(0.0);
    w.close()?;
    dir.close()
}

/// LUCENE-7570
///
/// Port of `TestTragicIndexWriterDeadlock.testDeadlockStalledMerges`.
fn test_deadlock_stalled_merges() -> Result<()> {
    do_test_deadlock_stalled_merges(false)
}

/// Port of `TestTragicIndexWriterDeadlock.testDeadlockStalledFullFlushMerges`.
fn test_deadlock_stalled_full_flush_merges() -> Result<()> {
    do_test_deadlock_stalled_merges(true)
}

/// The anonymous `IndexWriter` subclass of `doTestDeadlockStalledMerges`.
struct TragicMergeSuccess;

impl IndexWriterOverrides for TragicMergeSuccess {
    fn merge_success(&self, _writer: &IndexWriter, _merge: &OneMerge) {
        // tragedy strikes!
        Throwable::new("java.lang.OutOfMemoryError", "").throw()
    }
}

/// `private void doTestDeadlockStalledMerges(boolean mergeOnFlush)`.
fn do_test_deadlock_stalled_merges(merge_on_flush: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::new();
    iwc.set_max_full_flush_merge_wait_millis(if merge_on_flush { 1000 } else { 0 });

    // so we merge every 2 segments:
    let mp = LogMergePolicy::by_doc_count();
    mp.set_merge_factor(2)?;
    iwc.set_merge_policy(Some(Arc::new(mp)))?;
    let done = Arc::new(CountDownLatch::new(1));
    let cms = ConcurrentMergeScheduler::new();
    {
        let done = Arc::clone(&done);
        cms.set_do_merge_override(Some(Arc::new(move |cms, merge_source, merge| {
            // let merge takes forever, until commit thread is stalled
            done.await_();
            cms.default_do_merge(merge_source, merge)
        })));
    }
    {
        let done = Arc::clone(&done);
        cms.set_do_stall_override(Some(Arc::new(move |super_do_stall| {
            done.count_down();
            super_do_stall()
        })));
    }
    cms.set_handle_merge_exception_override(Some(Arc::new(|_exc| {})));

    // so we stall once the 2nd merge wants to run:
    cms.set_max_merges_and_threads(1, 1)?;
    iwc.set_merge_scheduler(Some(Arc::new(cms)))?;

    // so we write a segment every 2 indexed docs:
    iwc.set_max_buffered_docs(2)?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.set_overrides(Arc::new(TragicMergeSuccess));

    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // w writes first segment
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // w writes second segment, and kicks off merge, that takes forever (done.await)
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    // w writes third segment
    w.add_document(&Document::new())?;
    let e = expect_throws("java.lang.IllegalStateException", || w.commit());
    let message = e.message().unwrap_or_default().to_string();
    Assert::assert_true_msg(
        &message,
        message.starts_with("this writer hit an unrecoverable error"),
    );
    // w writes fourth segment, and commit flushes and kicks off merge that stalls
    w.close()?;
    dir.close()
}
