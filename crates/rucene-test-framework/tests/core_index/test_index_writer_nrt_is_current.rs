//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterNRTIsCurrent.java`.
//!
//! * The `volatile` fields of `ReaderHolder` are a mutex-guarded reader and
//!   an atomic flag; `holder.reader != currentReader` compares the readers'
//!   identity (`Arc::ptr_eq`).
//! * The threads' `Throwable failed` fields hold what each thread's
//!   `catch (Throwable e)` caught: an error a call returned, or a throwable
//!   (an assertion) the body raised.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::terms::Term;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test, try_catch, verbose};
use crate::support::CountDownLatch;

/// `TestIndexWriterNRTIsCurrent`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterNRTIsCurrent",
        vec![test(
            "testIsCurrentWithThreads",
            test_is_current_with_threads,
        )],
    )
}

/// `public static class ReaderHolder`.
#[derive(Default)]
struct ReaderHolder {
    reader: Mutex<Option<Arc<dyn DirectoryReader>>>,
    stop: AtomicBool,
}

impl ReaderHolder {
    fn reader(&self) -> Option<Arc<dyn DirectoryReader>> {
        self.reader
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_reader(&self, reader: Option<Arc<dyn DirectoryReader>>) {
        *self.reader.lock().unwrap_or_else(|e| e.into_inner()) = reader;
    }
}

/// A thread's `Throwable failed` field.
type Failed = Arc<Mutex<Option<Throwable>>>;

/// `try { body } catch (Throwable e) { failed = e; }`.
fn catch_throwable(failed: &Failed, body: impl FnOnce() -> Result<()>) -> bool {
    let caught = match try_catch("java.lang.Throwable", body) {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(java_throwable(&e)),
        Err(t) => Some(t),
    };
    match caught {
        Some(t) => {
            *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
            false
        }
        None => true,
    }
}

/// `failed.printStackTrace()` when `failed != null`; whether it was.
fn report(failed: &Failed) -> bool {
    match failed.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(t) => {
            println!("{t}");
            true
        }
        None => false,
    }
}

/// Port of `TestIndexWriterNRTIsCurrent.testIsCurrentWithThreads`.
fn test_is_current_with_threads() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let holder = Arc::new(ReaderHolder::default());
    let num_reader_threads = if *TEST_NIGHTLY {
        TestUtil::next_int(&*random(), 2, 5)
    } else {
        2
    };
    let latch = Arc::new(CountDownLatch::new(1));
    let num_ops = LuceneTestCase::at_least(500);
    let (writer_thread, writer_failed) = writer_thread(
        Arc::clone(&holder),
        Arc::clone(&writer),
        num_ops,
        Arc::clone(&latch),
    );
    let mut threads = Vec::new();
    for _ in 0..num_reader_threads {
        let (t, failed) = reader_thread(Arc::clone(&holder), Arc::clone(&latch));
        t.start();
        threads.push((t, failed));
    }
    writer_thread.start();

    let _ = writer_thread.join();
    let mut failed = report(&writer_failed);
    for (t, thread_failed) in &threads {
        let _ = t.join();
        if report(thread_failed) {
            failed = true;
        }
    }
    Assert::assert_false(failed);
    writer.close()?;
    dir.close()
}

/// `public static class WriterThread extends Thread`: the thread and its
/// `failed` field.
fn writer_thread(
    holder: Arc<ReaderHolder>,
    writer: Arc<IndexWriter>,
    num_ops: i32,
    latch: Arc<CountDownLatch>,
) -> (Thread, Failed) {
    let failed: Failed = Arc::new(Mutex::new(None));
    let thread_failed = Arc::clone(&failed);
    let t = Thread::unnamed(move || {
        let mut countdown = true;
        let current_reader: Mutex<Option<Arc<dyn DirectoryReader>>> = Mutex::new(None);
        let random = LuceneTestCase::random();
        catch_throwable(&thread_failed, || {
            let mut doc = Document::new();
            doc.add(Box::new(TextField::new("id", "1".to_string(), Store::NO)?));
            writer.add_document(&doc)?;
            let first = directory_reader::open_with_writer(&*writer)?;
            *current_reader.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&first));
            holder.set_reader(Some(first));
            let mut term = Term::empty("id");
            let mut i = 0;
            while i < num_ops && !holder.stop.load(Ordering::SeqCst) {
                let next_op = random.next_float();
                if next_op < 0.3 {
                    term.set("id", BytesRef::from_char_sequence("1"));
                    writer.update_document(term.clone(), &doc)?;
                } else if next_op < 0.5 {
                    writer.add_document(&doc)?;
                } else {
                    term.set("id", BytesRef::from_char_sequence("1"));
                    writer.delete_documents_by_terms(vec![term.clone()])?;
                }
                let current = current_reader
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let same = match (holder.reader(), current.as_ref()) {
                    (Some(a), Some(b)) => Arc::ptr_eq(&a, b),
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    holder.set_reader(current.clone());
                    if countdown {
                        countdown = false;
                        latch.count_down();
                    }
                }
                if random.next_boolean() {
                    writer.commit()?;
                    let current = current.expect("currentReader was opened");
                    let new_reader = directory_reader::open_if_changed(Arc::clone(&current))?;
                    let current = match new_reader {
                        Some(new_reader) => {
                            current.dec_ref()?;
                            *current_reader.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(Arc::clone(&new_reader));
                            new_reader
                        }
                        None => current,
                    };
                    if current.num_docs() == 0 {
                        writer.add_document(&doc)?;
                    }
                }
                i += 1;
            }
            Ok(())
        });
        // finally
        holder.set_reader(None);
        if countdown {
            latch.count_down();
        }
        if let Some(current) = current_reader
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = current.dec_ref();
        }
        if verbose() {
            println!(
                "writer stopped - forced by reader: {}",
                holder.stop.load(Ordering::SeqCst)
            );
        }
    });
    (t, failed)
}

/// `public static final class ReaderThread extends Thread`: the thread and
/// its `failed` field.
fn reader_thread(holder: Arc<ReaderHolder>, latch: Arc<CountDownLatch>) -> (Thread, Failed) {
    let failed: Failed = Arc::new(Mutex::new(None));
    let thread_failed = Arc::clone(&failed);
    let t = Thread::unnamed(move || {
        latch.await_();
        while let Some(reader) = holder.reader() {
            if reader.try_inc_ref() {
                let ok = catch_throwable(&thread_failed, || {
                    let current = reader.is_current()?;
                    if verbose() {
                        println!(
                            "Thread: {} Reader: {reader} isCurrent:{current}",
                            Thread::current()
                        );
                    }
                    Assert::assert_false(current);
                    Ok(())
                });
                if !ok && verbose() {
                    println!(
                        "FAILED Thread: {} Reader: {reader} isCurrent: false",
                        Thread::current()
                    );
                }
                if !ok {
                    holder.stop.store(true, Ordering::SeqCst);
                }
                // finally
                if let Err(e) = reader.dec_ref() {
                    let mut failed = thread_failed.lock().unwrap_or_else(|e| e.into_inner());
                    if failed.is_none() {
                        *failed = Some(java_throwable(&e));
                    }
                }
                if !ok {
                    return;
                }
            }
        }
    });
    (t, failed)
}
