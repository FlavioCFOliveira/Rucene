//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexTooManyDocs.java`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::index_writer::{self, IndexWriter};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::terms::Term;
use rucene::search::TermQuery;
use rucene_test_framework::util::java_exceptions::{is_instance, java_throwable};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, IAE};
use crate::support::CountDownLatch;

/// `TestIndexTooManyDocs`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexTooManyDocs",
        vec![test("testIndexTooManyDocs", test_index_too_many_docs)],
    )
}

/// `throw new AssertionError(e)`.
fn assertion_error(e: rucene::error::LuceneError) -> ! {
    randomizedtesting::jdk::exception::Throwable::new(
        "java.lang.AssertionError",
        &java_throwable(&e).to_string(),
    )
    .throw()
}

/// This test produces a boat load of very small segments with lot of deletes which are likely deleting
/// the entire segment. see https://issues.apache.org/jira/browse/LUCENE-8043
/// Port of `TestIndexTooManyDocs.testIndexTooManyDocs`.
fn test_index_too_many_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let num_max_doc = 25;
    let config = IndexWriterConfig::new();
    config.set_ram_buffer_size_mb(0.000001)?; // force lots of small segments and logs of concurrent deletes
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let result = (|| -> Result<()> {
        index_writer::set_max_docs(num_max_doc)?;
        let num_threads = 5 + random().next_int_bound(5);
        let latch = Arc::new(CountDownLatch::new(num_threads as i64));
        let indexing_done = Arc::new(CountDownLatch::new((num_threads - 2) as i64));
        let done = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::with_capacity(num_threads as usize);
        for i in 0..num_threads {
            let latch = Arc::clone(&latch);
            let indexing_done = Arc::clone(&indexing_done);
            let done = Arc::clone(&done);
            let writer = Arc::clone(&writer);
            let thread = if i >= 2 {
                Thread::unnamed(move || {
                    latch.count_down();
                    // `finally { indexingDone.countDown(); }`
                    struct Finally(Arc<CountDownLatch>);
                    impl Drop for Finally {
                        fn drop(&mut self) {
                            self.0.count_down();
                        }
                    }
                    let _finally = Finally(indexing_done);
                    latch.await_();
                    for _ in 0..100 {
                        let mut doc = Document::new();
                        let id = random().next_int_bound(num_max_doc * 2).to_string();
                        doc.add(Box::new(
                            StringField::new("id", id.clone(), Store::NO)
                                .unwrap_or_else(|e| assertion_error(e)),
                        ));
                        let t = Term::from_text("id", &id);
                        let r = (|| -> Result<()> {
                            if random().next_int_bound(5) == 0 {
                                writer.delete_documents_by_queries(vec![Arc::new(
                                    TermQuery::new(t.clone()),
                                )])?;
                            }
                            writer.update_document(t.clone(), &doc)?;
                            Ok(())
                        })();
                        match r {
                            Ok(()) => {}
                            Err(e) if is_instance(&e, IAE) => {
                                assert_equals(
                                    &Some(format!(
                                        "number of documents in the index cannot exceed {}",
                                        index_writer::get_actual_max_docs()
                                    )),
                                    &java_throwable(&e).message().map(str::to_string),
                                );
                            }
                            Err(e) => assertion_error(e),
                        }
                    }
                })
            } else {
                Thread::unnamed(move || {
                    let r = (|| -> Result<()> {
                        latch.count_down();
                        latch.await_();
                        let mut open =
                            directory_reader::open_with_writer_flags(&*writer, true, true)?;
                        while !done.load(Ordering::SeqCst) {
                            if let Some(directory_reader) =
                                directory_reader::open_if_changed(Arc::clone(&open))?
                            {
                                open.close()?;
                                open = directory_reader;
                            }
                        }
                        let _ = open.close();
                        Ok(())
                    })();
                    if let Err(e) = r {
                        assertion_error(e);
                    }
                })
            };
            thread.start();
            threads.push(thread);
        }

        indexing_done.await_();
        done.store(true, Ordering::SeqCst);

        for thread in &threads {
            if let Err(payload) = thread.join() {
                std::panic::resume_unwind(payload);
            }
        }
        writer.close()?;
        dir.close()
    })();
    index_writer::set_max_docs(MAX_DOCS)?;
    result
}
