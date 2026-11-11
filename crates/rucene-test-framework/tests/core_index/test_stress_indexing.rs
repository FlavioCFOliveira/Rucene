//! Port of `lucene/core/src/test/org/apache/lucene/index/TestStressIndexing.java`.
//!
//! The `TimedThread` subclasses are one thread body, [`timed_thread`], over a
//! `doWork` closure; each thread's `failed` flag is shared with the test and
//! with the other threads (`allThreads`).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, OnceLock};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, MergeScheduler};
use rucene::index::terms::Term;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{English, LuceneTestCase};

use crate::core_support::{class, random, test};

/// `TestStressIndexing`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestStressIndexing",
        vec![test(
            "testStressIndexAndSearching",
            test_stress_index_and_searching,
        )],
    )
}

/// `TimedThread.RUN_ITERATIONS`, a static of the nested class.
fn run_iterations() -> i32 {
    static RUN_ITERATIONS: OnceLock<i32> = OnceLock::new();
    *RUN_ITERATIONS.get_or_init(|| {
        if *TEST_NIGHTLY {
            LuceneTestCase::at_least(100)
        } else {
            LuceneTestCase::at_least(20)
        }
    })
}

/// `private abstract static class TimedThread extends Thread`.
fn timed_thread(
    failed: Arc<AtomicBool>,
    all_threads: Arc<Vec<Arc<AtomicBool>>>,
    mut do_work: impl FnMut() -> Result<()> + Send + 'static,
) -> Thread {
    Thread::unnamed(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            let mut iterations = 0;
            loop {
                // anyErrors()
                if all_threads.iter().any(|t| t.load(Ordering::SeqCst)) {
                    break;
                }
                do_work()?;
                iterations += 1;
                if iterations >= run_iterations() {
                    break;
                }
            }
            Ok(())
        }));
        let e = match outcome {
            Ok(Ok(())) => return,
            Ok(Err(e)) => java_throwable(&e),
            Err(payload) => Throwable::from_panic(payload),
        };
        println!("{}: exc", Thread::current());
        println!("{e}");
        failed.store(true, Ordering::SeqCst);
    })
}

/// `IndexerThread.doWork()`.
fn indexer_do_work(writer: &IndexWriter, next_id: &AtomicI32) -> Result<()> {
    // Add 10 docs:
    for _ in 0..10 {
        let mut d = Document::new();
        let n = random().next_int();
        d.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &next_id.fetch_add(1, Ordering::SeqCst).to_string(),
            Store::YES,
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

/// `SearcherThread.doWork()`.
fn searcher_do_work(directory: &Arc<dyn Directory>) -> Result<()> {
    for _ in 0..100 {
        let ir = directory_reader::open(Arc::clone(directory))?;
        LuceneTestCase::new_searcher(Arc::clone(&ir));
        ir.close()?;
    }
    Ok(())
}

/// Run one indexer and 2 searchers against single index as
/// stress test.
fn run_stress_test(
    directory: &Arc<dyn Directory>,
    merge_scheduler: Arc<dyn MergeScheduler>,
) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_max_buffered_docs(10)?;
    iwc.set_merge_scheduler(Some(merge_scheduler))?;
    let modifier = IndexWriter::new(Arc::clone(directory), iwc)?;
    modifier.commit()?;

    let failed: Vec<Arc<AtomicBool>> = (0..4).map(|_| Arc::new(AtomicBool::new(false))).collect();
    let all_threads = Arc::new(failed.clone());
    let mut threads: Vec<Thread> = Vec::new();

    // One modifier that writes 10 docs then removes 5, over
    // and over:
    for i in 0..2 {
        let w = Arc::clone(&modifier);
        let next_id = AtomicI32::new(0);
        let indexer_thread = timed_thread(
            Arc::clone(&failed[i]),
            Arc::clone(&all_threads),
            move || indexer_do_work(&w, &next_id),
        );
        threads.push(indexer_thread.clone());
        indexer_thread.start();
    }

    // Two searchers that constantly just re-instantiate the
    // searcher:
    for i in 2..4 {
        let dir = Arc::clone(directory);
        let searcher_thread = timed_thread(
            Arc::clone(&failed[i]),
            Arc::clone(&all_threads),
            move || searcher_do_work(&dir),
        );
        threads.push(searcher_thread.clone());
        searcher_thread.start();
    }

    for thread in &threads {
        let _ = thread.join();
    }

    modifier.close()?;

    for f in &failed {
        Assert::assert_false(f.load(Ordering::SeqCst));
    }

    // System.out.println("    Writer: " + indexerThread.count + " iterations");
    // System.out.println("Searcher 1: " + searcherThread1.count + " searchers created");
    // System.out.println("Searcher 2: " + searcherThread2.count + " searchers created");
    Ok(())
}

/// Port of `TestStressIndexing.testStressIndexAndSearching`.
fn test_stress_index_and_searching() -> Result<()> {
    let directory = if *TEST_NIGHTLY {
        LuceneTestCase::new_maybe_virus_checking_directory()
    } else {
        TestDirectory::Mock(MockDirectoryWrapper::new(
            &*random(),
            Arc::new(ByteBuffersDirectory::new()),
        ))
    };
    if let Some(mock) = directory.as_mock() {
        mock.set_assert_no_unrefenced_files_on_close(true);
    }

    run_stress_test(
        &directory.as_directory(),
        Arc::new(ConcurrentMergeScheduler::new()),
    )?;
    directory.close()
}
