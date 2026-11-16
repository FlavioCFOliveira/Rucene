/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.store.BaseLockFactoryTestCase`: base
//! class for per-LockFactory tests.
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseLockFactoryTestCase`]. As in
//! [`base_directory_test_case`](super::base_directory_test_case),
//! `getDirectory` returns a `Box<dyn Directory>` the test owns; a test that
//! shares it with threads turns it into an `Arc` and closes it once they
//! are done.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `testObtainConcurrently`: Java's `ReentrantLock` (`isLocked`,
//!   `tryLock`, `unlock`) is a `std::sync::Mutex` (`try_lock`); a lock held
//!   by another thread is what `isLocked()` reports, so the checks are the
//!   same.
//! * `WriterThread`'s `ByteArrayOutputStream`/`PrintStream`: a shared byte
//!   buffer the `PrintStreamInfoStream` writes into; it is printed on an
//!   unexpected exception, as Java prints it.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Barrier, LazyLock, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::terms::Term;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::store::java_nio_file::Path;
use rucene::store::Directory;
use rucene::util::BytesRef;

use super::base_directory_test_case::close_shared;
use crate::analysis::MockAnalyzer;
use crate::util::java_exceptions::{expect_throws, rethrow_java, try_with};
use crate::util::lucene_test_case::VERBOSE;
use crate::util::{LuceneTestCase, TestUtil};

/// The declaration of `org.apache.lucene.tests.store.BaseLockFactoryTestCase`
/// (`public abstract class BaseLockFactoryTestCase extends LuceneTestCase`)
/// with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        fn t(name: &str, body: fn(&dyn BaseLockFactoryTestCase) -> Result<()>) -> Method {
            lock_factory_test(name, body)
        }
        Class::builder("org.apache.lucene.tests.store.BaseLockFactoryTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(t("testBasics", |c| c.test_basics()))
            .method(t("testDoubleClose", |c| c.test_double_close()))
            .method(t("testValidAfterAcquire", |c| c.test_valid_after_acquire()))
            .method(t("testInvalidAfterClose", |c| c.test_invalid_after_close()))
            .method(t("testObtainConcurrently", |c| {
                c.test_obtain_concurrently()
            }))
            .method(t("testStressLocks", |c| c.test_stress_locks()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test method of a `BaseLockFactoryTestCase` subclass.
pub fn lock_factory_test(
    name: &str,
    body: impl Fn(&dyn BaseLockFactoryTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut LockFactoryTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// The instance of a `BaseLockFactoryTestCase` subclass the runner creates.
pub struct LockFactoryTestInstance {
    /// The subclass.
    pub case: Arc<dyn BaseLockFactoryTestCase>,
}

impl LockFactoryTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseLockFactoryTestCase>(case: T) -> Self {
        LockFactoryTestInstance {
            case: Arc::new(case),
        }
    }
}

/// Port of the members of `BaseLockFactoryTestCase`.
pub trait BaseLockFactoryTestCase: Send + Sync + 'static {
    /// `getDirectory(Path path)`: subclass returns the Directory to be
    /// tested; if it's an FS-based directory it should point to the
    /// specified path, else it can ignore it.
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>>;

    /// `testBasics()`: test obtaining and releasing locks, checking
    /// validity.
    fn test_basics(&self) -> Result<()> {
        let temp_path = LuceneTestCase::create_temp_dir();
        let dir = self.get_directory(&temp_path)?;

        let l = dir.obtain_lock("commit")?;
        // shouldn't be able to get the lock twice
        expect_throws("org.apache.lucene.store.LockObtainFailedException", || {
            dir.obtain_lock("commit")
        });
        l.close()?;

        // Make sure we can obtain first one again:
        let l = dir.obtain_lock("commit")?;
        l.close()?;

        dir.close()
    }

    /// `testDoubleClose()`: test closing locks twice.
    fn test_double_close(&self) -> Result<()> {
        let temp_path = LuceneTestCase::create_temp_dir();
        let dir = self.get_directory(&temp_path)?;

        let l = dir.obtain_lock("commit")?;
        l.close()?;
        l.close()?; // close again, should be no exception

        dir.close()
    }

    /// `testValidAfterAcquire()`: test ensureValid returns true after
    /// acquire.
    fn test_valid_after_acquire(&self) -> Result<()> {
        let temp_path = LuceneTestCase::create_temp_dir();
        let dir = self.get_directory(&temp_path)?;
        let l = dir.obtain_lock("commit")?;
        l.ensure_valid()?; // no exception
        l.close()?;
        dir.close()
    }

    /// `testInvalidAfterClose()`: test ensureValid throws exception after
    /// close.
    fn test_invalid_after_close(&self) -> Result<()> {
        let temp_path = LuceneTestCase::create_temp_dir();
        let dir = self.get_directory(&temp_path)?;

        let l = dir.obtain_lock("commit")?;
        l.close()?;

        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            l.ensure_valid()
        });

        dir.close()
    }

    /// `testObtainConcurrently()`.
    fn test_obtain_concurrently(&self) -> Result<()> {
        let temp_path = LuceneTestCase::create_temp_dir();
        let directory: Arc<dyn Directory> = Arc::from(self.get_directory(&temp_path)?);
        let running = Arc::new(AtomicBool::new(true));
        let atomic_counter = Arc::new(AtomicI32::new(0));
        let asserting_lock = Arc::new(Mutex::new(()));
        let num_threads = 2 + LuceneTestCase::random().next_int_bound(10) as usize;
        let runs = LuceneTestCase::at_least(1000);
        let barrier = Arc::new(Barrier::new(num_threads));
        let mut threads = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            let directory = Arc::clone(&directory);
            let running = Arc::clone(&running);
            let atomic_counter = Arc::clone(&atomic_counter);
            let asserting_lock = Arc::clone(&asserting_lock);
            let barrier = Arc::clone(&barrier);
            let thread = Thread::unnamed(move || {
                barrier.wait();
                while running.load(Ordering::SeqCst) {
                    // `try (Lock lock = directory.obtainLock("foo.lock")) { ... }
                    // catch (IOException ex) {}`: the lock closes on every exit
                    // of the body; the catch also sees the obtain and the close.
                    let outcome = directory.obtain_lock("foo.lock").and_then(|lock| {
                        try_with(
                            lock,
                            |lock| lock.close(),
                            |_| {
                                match asserting_lock.try_lock() {
                                    // assertFalse(assertingLock.isLocked()), then tryLock and unlock
                                    Ok(guard) => drop(guard),
                                    Err(_) => Assert::assert_false(true),
                                }
                                Ok(())
                            },
                        )
                    });
                    match outcome {
                        Ok(()) => {}
                        Err(e) if e.is_io_exception() => {
                            //
                        }
                        Err(e) => rethrow_java(e),
                    }
                    if atomic_counter.fetch_add(1, Ordering::SeqCst) + 1 > runs {
                        running.store(false, Ordering::SeqCst);
                    }
                }
            });
            thread.start();
            threads.push(thread);
        }
        for thread in threads {
            // `Thread.join()` returns normally whatever the thread threw.
            let _ = thread.join();
        }
        close_shared(directory)
    }

    /// `testStressLocks()`: do stress test, by opening IndexReaders and
    /// IndexWriters over & over in 2 threads and making sure no unexpected
    /// exceptions are raised.
    fn test_stress_locks(&self) -> Result<()> {
        let temp_path = LuceneTestCase::create_temp_dir();
        LuceneTestCase::assume_false(
            "cannot handle buggy Files.delete",
            TestUtil::has_windows_fs_path(&temp_path),
        );

        let dir: Arc<dyn Directory> = Arc::from(self.get_directory(&temp_path)?);

        // First create a 1 doc index:
        let config = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*LuceneTestCase::random(),
        ))));
        config.set_open_mode(OpenMode::CREATE)?;
        let w = IndexWriter::new(Arc::clone(&dir), config)?;
        add_doc(&w)?;
        w.close()?;
        drop(w);

        let num_iterations = LuceneTestCase::at_least(20);
        let writer_hit = Arc::new(AtomicBool::new(false));
        let searcher_hit = Arc::new(AtomicBool::new(false));
        let writer = {
            let dir = Arc::clone(&dir);
            let hit = Arc::clone(&writer_hit);
            Thread::unnamed(move || writer_thread_run(num_iterations, &dir, &hit))
        };
        let searcher = {
            let dir = Arc::clone(&dir);
            let hit = Arc::clone(&searcher_hit);
            Thread::unnamed(move || searcher_thread_run(num_iterations, &dir, &hit))
        };
        writer.start();
        searcher.start();

        // `Thread.join()` returns normally whatever the thread threw.
        let _ = writer.join();
        let _ = searcher.join();

        Assert::assert_true_msg(
            "IndexWriter hit unexpected exceptions",
            !writer_hit.load(Ordering::SeqCst),
        );
        Assert::assert_true_msg(
            "IndexSearcher hit unexpected exceptions",
            !searcher_hit.load(Ordering::SeqCst),
        );

        close_shared(dir)
    }
}

/// `addDoc(IndexWriter writer)` (`private`).
fn add_doc(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `ByteArrayOutputStream`, shared between the `PrintStream` the
/// `PrintStreamInfoStream` writes into and the thread that prints it.
#[derive(Clone, Default)]
struct SharedBytes(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBytes {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedBytes {
    /// `toString(baos)`: the bytes as UTF-8.
    fn text(&self) -> String {
        String::from_utf8_lossy(
            &self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .into_owned()
    }
}

/// `WriterThread.run()`.
fn writer_thread_run(num_iteration: i32, dir: &Arc<dyn Directory>, hit_exception: &AtomicBool) {
    let baos = SharedBytes::default();
    for i in 0..num_iteration {
        if *VERBOSE {
            randomizedtesting::jdk::system::out_println(&format!("TEST: WriterThread iter={i}"));
        }

        let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*LuceneTestCase::random(),
        ))));

        // We only print the IW infoStream output on exc, below:
        let mut print_stream = baos.clone();
        let _ = iwc.set_info_stream_print_stream(Box::new(baos.clone()));

        let _ = writeln!(print_stream, "\nTEST: WriterThread iter={i}");
        let _ = iwc.set_open_mode(OpenMode::APPEND);
        // Each `catch (Throwable t)`: an error or any panic.
        let writer = match catch_throwable(|| IndexWriter::new(Arc::clone(dir), iwc)) {
            Ok(writer) => writer,
            Err(t) => {
                report(hit_exception, "creation", &t, &baos);
                break;
            }
        };
        if let Err(t) = catch_throwable(|| add_doc(&writer)) {
            report(hit_exception, "addDoc", &t, &baos);
            break;
        }
        if let Err(t) = catch_throwable(|| writer.close()) {
            report(hit_exception, "close", &t, &baos);
            break;
        }
    }
}

/// `try { body } catch (Throwable t)`: the throwable of an error or of any
/// panic.
fn catch_throwable<T>(body: impl FnOnce() -> Result<T>) -> std::result::Result<T, Throwable> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(crate::util::java_exceptions::java_throwable(&e)),
        Err(payload) => Err(crate::mockfile::panic_throwable(&*payload)),
    }
}

/// The `catch (Throwable t)` of `WriterThread.run()` (`Constants.WINDOWS`
/// is false on the platforms the port runs on): the message,
/// `t.printStackTrace(System.out)`, then the infoStream output.
fn report(hit_exception: &AtomicBool, what: &str, t: &Throwable, baos: &SharedBytes) {
    hit_exception.store(true, Ordering::SeqCst);
    randomizedtesting::jdk::system::out_println(&format!(
        "Stress Test Index Writer: {what} hit unexpected exception: {t}"
    ));
    randomizedtesting::jdk::system::out_println(
        &randomizedtesting::jdk::system::format_stack_trace(t),
    );
    randomizedtesting::jdk::system::out_println(&baos.text());
}

/// A `catch` clause of `SearcherThread.run()`: the flag, the message and
/// `e.printStackTrace(System.out)`.
fn report_searcher(hit_exception: &AtomicBool, what: &str, e: &Throwable) {
    hit_exception.store(true, Ordering::SeqCst);
    randomizedtesting::jdk::system::out_println(&format!(
        "Stress Test Index Searcher: {what} hit unexpected exception: {e}"
    ));
    randomizedtesting::jdk::system::out_println(
        &randomizedtesting::jdk::system::format_stack_trace(e),
    );
}

/// `catch (IOException e)`: the throwable of an I/O error; any other error
/// propagates out of `run()` as the Java exception it stands for.
fn caught_io(e: &LuceneError) -> Throwable {
    let t = crate::util::java_exceptions::java_throwable(e);
    if !t.is_instance(randomizedtesting::jdk::exception::IO_EXCEPTION) {
        t.throw()
    }
    t
}

/// `SearcherThread.run()`.
fn searcher_thread_run(num_iteration: i32, dir: &Arc<dyn Directory>, hit_exception: &AtomicBool) {
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new(
        "content",
        BytesRef::new(b"aaa".to_vec()),
    )));
    for _ in 0..num_iteration {
        // `catch (Exception e)` around opening the reader and the searcher:
        // an error, or a panic that is not an `Error`.
        let opened = crate::util::java_exceptions::caught_value(std::panic::catch_unwind(
            std::panic::AssertUnwindSafe(|| {
                let reader = directory_reader::open(Arc::clone(dir))?;
                let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
                Ok((reader, searcher))
            }),
        ));
        let (reader, searcher) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                report_searcher(hit_exception, "create", &e);
                break;
            }
        };
        if let Err(e) = searcher.search(Arc::clone(&query), 1000) {
            report_searcher(hit_exception, "search", &caught_io(&e));
            break;
        }
        // System.out.println(hits.length() + " total results");
        drop(searcher);
        if let Err(e) = reader.close() {
            report_searcher(hit_exception, "close", &caught_io(&e));
            break;
        }
    }
}
