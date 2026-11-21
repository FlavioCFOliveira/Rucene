//! `TestNRTThreads`'s `ThreadedIndexingAndSearchingTestCase` subclass and
//! its members (`setUp()`, `testNRTThreads()`), shared by the suites of
//! `threaded_indexing_and_searching_suites.rs` and by
//! `TestIndexWriterOnJRECrash` (`core_index_tests`), which extends
//! `TestNRTThreads`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::IndexReader;
use rucene::search::{Executor, IndexSearcher};
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::index::threaded_indexing_and_searching_test_case::{
    self as threaded, run_search_threads, run_test, smoke_test_searcher, CachedThreadPoolExecutor,
    TestSearcher, ThreadedDirectory, ThreadedIndexingAndSearchingTestCase, ThreadedState,
};
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, VERBOSE};
use rucene_test_framework::util::LuceneTestCase;

/// `@SuppressCodecs({"SimpleText", "Direct"})`.
pub(crate) fn suppress_codecs() -> SuppressCodecs {
    SuppressCodecs {
        value: vec!["SimpleText".to_string(), "Direct".to_string()],
    }
}

/// `es` as the `Executor` an `IndexSearcher` takes.
pub(crate) fn executor(es: &Option<Arc<CachedThreadPoolExecutor>>) -> Option<Arc<dyn Executor>> {
    es.as_ref().map(|es| Arc::clone(es) as Arc<dyn Executor>)
}

/// The instance of a threaded class: the subclass, shared with its threads.
pub(crate) struct ThreadedInstance<T: ThreadedIndexingAndSearchingTestCase> {
    pub(crate) case: Arc<T>,
}

// -----------------------------------------------------------------------------
// TestNRTThreads
// -----------------------------------------------------------------------------

/// `TestNRTThreads`.
#[derive(Default)]
pub(crate) struct NrtThreads {
    state: Arc<ThreadedState>,
    /// `useNonNrtReaders` (initially `true`).
    pub(crate) use_non_nrt_readers: AtomicBool,
    /// `fixedSearcher`.
    fixed_searcher: Mutex<Option<Arc<IndexSearcher>>>,
}

impl NrtThreads {
    pub(crate) fn new() -> Self {
        NrtThreads {
            use_non_nrt_readers: AtomicBool::new(true),
            ..NrtThreads::default()
        }
    }

    /// `((MockDirectoryWrapper) dir).getOpenDeletedFiles().size()`, printing
    /// them when there are some.
    fn open_deleted_files(&self) -> bool {
        let open_deleted_files = self.state.with_dir(|d| {
            d.as_mock()
                .expect("TestNRTThreads' directory is a MockDirectoryWrapper")
                .get_open_deleted_files()
        });
        if !open_deleted_files.is_empty() {
            println!("OBD files: {open_deleted_files:?}");
        }
        !open_deleted_files.is_empty()
    }
}

impl ThreadedIndexingAndSearchingTestCase for NrtThreads {
    fn state(&self) -> &Arc<ThreadedState> {
        &self.state
    }

    fn do_searching(
        &self,
        es: Option<Arc<CachedThreadPoolExecutor>>,
        max_iterations: i32,
    ) -> Result<()> {
        let mut any_open_del_files = false;
        let writer = self.state.writer();

        let mut r = directory_reader::open_with_writer(&*writer)?;
        let mut iterations = 0;
        loop {
            iterations += 1;
            if !(iterations < max_iterations && !self.state.failed.load(Ordering::SeqCst)) {
                break;
            }
            if LuceneTestCase::random().next_boolean() {
                if *VERBOSE {
                    println!("TEST: now reopen r={r}");
                }
                if let Some(r2) = directory_reader::open_if_changed(Arc::clone(&r))? {
                    r.close()?;
                    r = r2;
                }
            } else {
                if *VERBOSE {
                    println!("TEST: now close reader={r}");
                }
                r.close()?;
                writer.commit()?;
                any_open_del_files |= self.open_deleted_files();
                // assertEquals("open but deleted: " + openDeletedFiles, 0, openDeletedFiles.size());
                if *VERBOSE {
                    println!("TEST: now open");
                }
                r = directory_reader::open_with_writer(&*writer)?;
            }
            if *VERBOSE {
                println!("TEST: got new reader={r}");
            }

            if r.num_docs() > 0 {
                let fixed = Arc::new(IndexSearcher::with_executor(
                    Arc::clone(&r) as Arc<dyn IndexReader>,
                    executor(&es),
                )?);
                *self
                    .fixed_searcher
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&fixed));
                smoke_test_searcher(&fixed)?;
                run_search_threads(&threaded::this(&self.state), 100)?;
            }
        }
        r.close()?;
        // `fixedSearcher` keeps the closed reader, and with it the directory,
        // alive; Java's field holds it until the instance is collected. No
        // caller reads it after the search threads end, so it is released
        // here, before `runTest` closes the directory.
        *self
            .fixed_searcher
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;

        any_open_del_files |= self.open_deleted_files();

        Assert::assert_false_msg("saw non-zero open-but-deleted count", any_open_del_files);
        Ok(())
    }

    fn get_directory(&self, r#in: MockDirectoryWrapper) -> ThreadedDirectory {
        if !self.use_non_nrt_readers.load(Ordering::SeqCst) {
            r#in.set_assert_no_delete_open_file(true);
        }
        ThreadedDirectory::Mock(r#in)
    }

    fn do_after_writer(&self, _es: Option<Arc<CachedThreadPoolExecutor>>) -> Result<()> {
        // Force writer to do reader pooling, always, so that
        // all merged segments, even for merges before
        // doSearching is called, are warmed:
        directory_reader::open_with_writer(&*self.state.writer())?.close()
    }

    fn get_current_searcher(&self) -> Result<TestSearcher> {
        Ok(TestSearcher::Plain(
            self.fixed_searcher
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .expect("doSearching set the fixed searcher"),
        ))
    }

    fn release_searcher(&self, s: TestSearcher) -> Result<()> {
        let fixed = self
            .fixed_searcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let is_fixed = fixed.is_some_and(|f| s.same(&TestSearcher::Plain(f)));
        if !is_fixed {
            // Final searcher:
            s.searcher().get_index_reader().close()?;
        }
        Ok(())
    }

    fn get_final_searcher(&self) -> Result<TestSearcher> {
        let writer = self.state.writer();
        let r2: Arc<dyn DirectoryReader> = if self.use_non_nrt_readers.load(Ordering::SeqCst) {
            if LuceneTestCase::random().next_boolean() {
                directory_reader::open_with_writer(&*writer)?
            } else {
                writer.commit()?;
                directory_reader::open(self.state.dir())?
            }
        } else {
            directory_reader::open_with_writer(&*writer)?
        };
        Ok(TestSearcher::Plain(Arc::new(LuceneTestCase::new_searcher(
            r2,
        ))))
    }
}

/// The members of `TestNRTThreads`: `setUp()` and `testNRTThreads()`.
pub(crate) fn nrt_threads_members(
    builder: randomizedtesting::jdk::reflect::ClassBuilder,
) -> randomizedtesting::jdk::reflect::ClassBuilder {
    builder
        .annotate(suppress_codecs())
        .constructor(Constructor::no_args(|| ThreadedInstance {
            case: Arc::new(NrtThreads::new()),
        }))
        .method(
            Method::new("setUp")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut ThreadedInstance<NrtThreads>| nrt_threads_set_up(&inst.case)),
        )
        .method(
            Method::new("testNRTThreads")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut ThreadedInstance<NrtThreads>| {
                    nrt_threads_test(&inst.case).unwrap_or_else(|e| rethrow(e));
                }),
        )
}

/// `TestNRTThreads.setUp()`: `super.setUp()`, then `useNonNrtReaders =
/// random().nextBoolean()`.
pub(crate) fn nrt_threads_set_up(case: &NrtThreads) {
    LuceneTestCase::super_set_up();
    case.use_non_nrt_readers
        .store(LuceneTestCase::random().next_boolean(), Ordering::SeqCst);
}

/// `TestNRTThreads.testNRTThreads()`: `runTest("TestNRTThreads")`.
pub(crate) fn nrt_threads_test(case: &Arc<NrtThreads>) -> Result<()> {
    run_test(Arc::clone(case) as _, "TestNRTThreads")
}

/// The class `org.apache.lucene.index.TestNRTThreads`.
pub(crate) fn nrt_threads_class() -> Arc<randomizedtesting::jdk::reflect::Class> {
    nrt_threads_members(
        randomizedtesting::jdk::reflect::Class::builder("org.apache.lucene.index.TestNRTThreads")
            .extends(&threaded::class()),
    )
    .build()
}
