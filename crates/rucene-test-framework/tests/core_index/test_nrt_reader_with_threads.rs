//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNRTReaderWithThreads.java`.
//!
//! `RunThread`'s fields the test reads after `join()` (`failure`) live in a
//! state it shares with the test; the instance field `seq` is shared the
//! same way.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::doc_helper::DocHelper;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, random, test, try_catch};

/// `TestNRTReaderWithThreads`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestNRTReaderWithThreads",
        vec![test("testIndexing", test_indexing)],
    )
}

/// Port of `TestNRTReaderWithThreads.testIndexing`.
fn test_indexing() -> Result<()> {
    // `AtomicInteger seq = new AtomicInteger(1)`: an instance field.
    let seq = Arc::new(AtomicI32::new(1));
    let main_dir = LuceneTestCase::new_directory();
    if let Some(mock) = main_dir.as_mock() {
        mock.set_assert_no_delete_open_file(true);
    }
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    iwc.set_max_buffered_docs(10)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 2)?,
    )))?;
    LuceneTestCase::ensure_sane_iwc_on_nightly(&mut iwc);
    let writer = IndexWriter::new(main_dir.as_directory(), iwc)?;
    let reader = directory_reader::open_with_writer(&*writer)?; // start pooling readers
    reader.close()?;
    let num_threads = if *TEST_NIGHTLY { 4 } else { 2 };
    let num_iterations = if *TEST_NIGHTLY { 2000 } else { 50 };
    let mut index_threads = Vec::with_capacity(num_threads);
    for x in 0..num_threads {
        let t = RunThread::new((x % 2) as i32, Arc::clone(&writer), num_iterations, &seq);
        t.thread.set_name(format!("Thread {x}"));
        t.thread.start();
        index_threads.push(t);
    }
    for thread in &index_threads {
        let _ = thread.thread.join();
    }
    writer.close()?;
    drop(writer);
    main_dir.close()?;

    for thread in &index_threads {
        let failure = thread
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(failure) = failure {
            Throwable::with_cause(
                "java.lang.RuntimeException",
                Some(format!("hit exception from {}", thread.thread)),
                failure,
            )
            .throw();
        }
    }
    Ok(())
}

/// `public class RunThread extends Thread`.
struct RunThread {
    thread: Thread,
    failure: Arc<Mutex<Option<Throwable>>>,
}

impl RunThread {
    fn new(
        r#type: i32,
        writer: Arc<IndexWriter>,
        num_iterations: i32,
        seq: &Arc<AtomicI32>,
    ) -> Self {
        let failure: Arc<Mutex<Option<Throwable>>> = Arc::new(Mutex::new(None));
        // `final Random r = new Random(random().nextLong())`, drawn at construction.
        let r = JdkRandom::new(random().next_long());
        let (seq, fail) = (Arc::clone(seq), Arc::clone(&failure));
        let thread = Thread::unnamed(move || {
            let mut del_count = 0;
            let mut add_count = 0;
            let outcome = try_catch("java.lang.Throwable", || -> Result<()> {
                for _ in 0..num_iterations {
                    // int n = random.nextInt(2);
                    if r#type == 0 {
                        let i = seq.fetch_add(1, Ordering::SeqCst) + 1;
                        let doc = DocHelper::create_document(i, "index1", 10);
                        writer.add_document(&doc)?;
                        add_count += 1;
                    } else if r#type == 1 {
                        // we may or may not delete because the term may not exist,
                        // however we're opening and closing the reader rapidly
                        let reader: Arc<dyn IndexReader> =
                            directory_reader::open_with_writer(&*writer)?;
                        let id = r.next_int_bound(seq.load(Ordering::SeqCst));
                        let term = Term::from_text("id", &id.to_string());
                        let count = crate::test_index_writer_reader::count(&term, &reader)?;
                        writer.delete_documents_by_terms(vec![term])?;
                        reader.close()?;
                        del_count += count;
                    }
                }
                Ok(())
            });
            let _ = (del_count, add_count);
            let caught = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(java_throwable(&e)),
                Err(t) => Some(t),
            };
            if let Some(ex) = caught {
                println!("{ex}");
                *fail.lock().unwrap_or_else(|p| p.into_inner()) = Some(ex.clone());
                Throwable::from_cause("java.lang.RuntimeException", ex).throw();
            }
        });
        RunThread { thread, failure }
    }
}
