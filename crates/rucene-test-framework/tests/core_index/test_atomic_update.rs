//! Port of `lucene/core/src/test/org/apache/lucene/index/TestAtomicUpdate.java`.
//!
//! The `TimedThread` subclasses are one thread body, [`timed_thread`], over
//! a `doWork` closure; `failure` is shared with the test, which reads it
//! after `join`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::document::{Document, IntPoint, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::TieredMergePolicy;
use rucene::index::terms::Term;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{English, LuceneTestCase};

use crate::core_support::{assert_equals, class, random, test};

/// `TestAtomicUpdate`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestAtomicUpdate",
        vec![test("testAtomicUpdates", test_atomic_updates)],
    )
}

/// `volatile Throwable failure`.
type Failure = Arc<Mutex<Option<Throwable>>>;

/// `private abstract static class TimedThread extends Thread`: runs
/// `do_work` `num_iterations` times; a throwable is recorded as `failure`,
/// printed, and rethrown as a `RuntimeException`.
fn timed_thread(
    num_iterations: i32,
    failure: Failure,
    do_work: impl Fn(i32) -> Result<()> + Send + 'static,
) -> Thread {
    Thread::unnamed(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            for count in 0..num_iterations {
                do_work(count)?;
            }
            Ok(())
        }));
        let e = match outcome {
            Ok(Ok(())) => return,
            Ok(Err(e)) => java_throwable(&e),
            Err(payload) => Throwable::from_panic(payload),
        };
        *failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.clone());
        println!("{e}");
        Throwable::from_cause("java.lang.RuntimeException", e).throw()
    })
}

/// `IndexerThread.doWork`: update all 100 docs.
fn indexer_do_work(writer: &IndexWriter, current_iteration: i32) -> Result<()> {
    // Update all 100 docs...
    for i in 0..100 {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        d.add(Box::new(LuceneTestCase::new_text_field(
            "contents",
            &English::int_to_english(i + 10 * current_iteration),
            Store::NO,
        )));
        d.add(Box::new(IntPoint::new("doc", &[i])?));
        d.add(Box::new(IntPoint::new("doc2d", &[i, i])?));
        writer.update_document(Term::from_text("id", &i.to_string()), &d)?;
    }
    Ok(())
}

/// `SearcherThread.doWork`.
fn searcher_do_work(directory: &Arc<dyn Directory>) -> Result<()> {
    let r = directory_reader::open(Arc::clone(directory))?;
    assert_equals(&100, &r.num_docs());
    r.close()
}

/// `public void runTest(Directory directory)`: run N indexer and N searchers
/// against single index as stress test.
fn run_test(directory: Arc<dyn Directory>) -> Result<()> {
    let index_threads = if *TEST_NIGHTLY { 2 } else { 1 };
    let search_threads = if *TEST_NIGHTLY { 2 } else { 1 };
    let index_iterations = if *TEST_NIGHTLY { 10 } else { 1 };
    let search_iterations = if *TEST_NIGHTLY { 10 } else { 1 };

    let mut conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    conf.set_max_buffered_docs(7)?;
    conf.merge_policy()
        .as_any()
        .and_then(|a| a.downcast_ref::<TieredMergePolicy>())
        .unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a TieredMergePolicy").throw()
        })
        .set_max_merge_at_once(3)?;
    let writer = RandomIndexWriter::mock_index_writer(Arc::clone(&directory), conf, &*random())?;

    // Establish a base index of 100 docs:
    for i in 0..100 {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        d.add(Box::new(LuceneTestCase::new_text_field(
            "contents",
            &English::int_to_english(i),
            Store::NO,
        )));
        if (i - 1) % 7 == 0 {
            writer.commit()?;
        }
        writer.add_document(&d)?;
    }
    writer.commit()?;

    let r = directory_reader::open(Arc::clone(&directory))?;
    assert_equals(&100, &r.num_docs());
    r.close()?;

    let mut threads: Vec<(Thread, Failure)> = Vec::new();
    for _ in 0..index_threads {
        let failure: Failure = Arc::new(Mutex::new(None));
        let w = Arc::clone(&writer);
        let thread = timed_thread(index_iterations, Arc::clone(&failure), move |it| {
            indexer_do_work(&w, it)
        });
        threads.push((thread, failure));
    }
    for _ in 0..search_threads {
        let failure: Failure = Arc::new(Mutex::new(None));
        let dir = Arc::clone(&directory);
        let thread = timed_thread(search_iterations, Arc::clone(&failure), move |_| {
            searcher_do_work(&dir)
        });
        threads.push((thread, failure));
    }

    for (thread, _) in &threads {
        thread.start();
    }

    for (thread, _) in &threads {
        let _ = thread.join();
    }

    writer.close()?;

    for (thread, failure) in &threads {
        if let Some(failure) = failure.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            Throwable::with_cause(
                "java.lang.RuntimeException",
                Some(format!("hit exception from {thread}")),
                failure,
            )
            .throw()
        }
    }
    Ok(())
}

/// Port of `TestAtomicUpdate.testAtomicUpdates`.
fn test_atomic_updates() -> Result<()> {
    // run against a random directory.
    let mut directory = TestDirectory::Mock(MockDirectoryWrapper::new(
        &*random(),
        Arc::new(ByteBuffersDirectory::new()),
    ));
    run_test(directory.as_directory())?;
    directory.close()?;

    // then against an FSDirectory.
    let dir_path = LuceneTestCase::create_temp_dir_prefix("lucene.test.atomic");
    let mut directory = LuceneTestCase::new_fs_directory(&dir_path);
    run_test(directory.as_directory())?;
    directory.close()
}
