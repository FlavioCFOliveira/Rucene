//! The `ThreadedIndexingAndSearchingTestCase` suites (rmp #355): Lucene's
//! subclasses run `runTest` with their overrides of the searcher, indexing
//! and lifecycle hooks, under the ported runner and `LuceneTestCase`'s rule
//! chain (each is `@SuppressCodecs({"SimpleText", "Direct"})`):
//!
//! * `org.apache.lucene.index.TestNRTThreads`: near-real-time readers opened
//!   and reopened from the writer, searched by a fixed searcher;
//! * `org.apache.lucene.search.TestSearcherManager`: a `SearcherManager`
//!   (from the writer or from the directory) refreshed by a reopen thread,
//!   with a `SearcherLifetimeManager` handing out past searchers;
//! * `org.apache.lucene.search.TestControlledRealTimeReopenThread`: two
//!   `SearcherManager`s (with and without deletes) refreshed by two
//!   `ControlledRealTimeReopenThread`s, whose generations the indexing
//!   threads wait for.
//!
//! `ThreadedIndexingAndSearchingTestCase` declares no JUnit test; each
//! class's `test…` method that calls `runTest` is the binding.
//! `TestControlledRealTimeReopenThread` also carries its other test methods
//! (`testThreadStarvationNoDeleteNRTReader`, `testEvilSearcherFactory`,
//! `testListenerCalled`, `testCRTReopen`, `testDeleteAll`; sprint 9, #390),
//! so that the class has one binding. The other test methods of the other
//! classes (`testIntermediateClose`, …) belong to their core-test port and
//! are not declared here, nor is
//! `TestIndexWriterOnJRECrash` (it extends `TestNRTThreads` but overrides
//! its only test to fork and crash a child JVM).
//!
//! **Divergence** (declared): `ControlledRealTimeReopenThread` is not a
//! `java.lang.Thread` in the port; `setName`, `setPriority` and `setDaemon`
//! have no counterpart and are not called.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread;
use rucene::search::{
    Executor, IndexSearcher, ManagedSearcher, PruneByAge, SearcherFactory, SearcherLifetimeManager,
    SearcherManager, TermQuery,
};
use rucene::store::NRTCachingDirectory;
use rucene::util::BytesRef;
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::index::threaded_indexing_and_searching_test_case::{
    self as threaded, run_search_threads, run_test, smoke_test_searcher, CachedThreadPoolExecutor,
    TestSearcher, ThreadedDirectory, ThreadedIndexingAndSearchingTestCase, ThreadedState,
};
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// `@SuppressCodecs({"SimpleText", "Direct"})`.
fn suppress_codecs() -> SuppressCodecs {
    SuppressCodecs {
        value: vec!["SimpleText".to_string(), "Direct".to_string()],
    }
}

/// `es` as the `Executor` an `IndexSearcher` takes.
fn executor(es: &Option<Arc<CachedThreadPoolExecutor>>) -> Option<Arc<dyn Executor>> {
    es.as_ref().map(|es| Arc::clone(es) as Arc<dyn Executor>)
}

/// `new TermQuery(new Term("body", "united"))`.
fn body_united() -> Arc<TermQuery> {
    Arc::new(TermQuery::new(Term::new(
        "body",
        BytesRef::new(b"united".to_vec()),
    )))
}

/// The instance of a threaded class: the subclass, shared with its threads.
struct ThreadedInstance<T: ThreadedIndexingAndSearchingTestCase> {
    case: Arc<T>,
}

// -----------------------------------------------------------------------------
// TestNRTThreads
// -----------------------------------------------------------------------------

/// `TestNRTThreads`.
#[derive(Default)]
struct NrtThreads {
    state: Arc<ThreadedState>,
    /// `useNonNrtReaders` (initially `true`).
    use_non_nrt_readers: AtomicBool,
    /// `fixedSearcher`.
    fixed_searcher: Mutex<Option<Arc<IndexSearcher>>>,
}

impl NrtThreads {
    fn new() -> Self {
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
fn nrt_threads_members(
    builder: randomizedtesting::jdk::reflect::ClassBuilder,
) -> randomizedtesting::jdk::reflect::ClassBuilder {
    builder
        .annotate(suppress_codecs())
        .constructor(Constructor::no_args(|| ThreadedInstance {
            case: Arc::new(NrtThreads::new()),
        }))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut ThreadedInstance<NrtThreads>| {
                LuceneTestCase::super_set_up();
                inst.case
                    .use_non_nrt_readers
                    .store(LuceneTestCase::random().next_boolean(), Ordering::SeqCst);
            },
        ))
        .method(
            Method::new("testNRTThreads")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut ThreadedInstance<NrtThreads>| {
                    run_test(Arc::clone(&inst.case) as _, "TestNRTThreads")
                        .unwrap_or_else(|e| rethrow(e));
                }),
        )
}

fn nrt_threads_class() -> Arc<Class> {
    nrt_threads_members(
        Class::builder("org.apache.lucene.index.TestNRTThreads").extends(&threaded::class()),
    )
    .build()
}

// -----------------------------------------------------------------------------
// TestSearcherManager
// -----------------------------------------------------------------------------

/// The anonymous `SearcherFactory` of the managers: a searcher over `es`,
/// marked as warmed and searched once.
struct WarmingSearcherFactory {
    es: Option<Arc<dyn Executor>>,
    warm_called: Arc<AtomicBool>,
}

impl SearcherFactory for WarmingSearcherFactory {
    fn new_searcher(
        &self,
        r: Arc<dyn IndexReader>,
        _previous: Option<Arc<dyn IndexReader>>,
    ) -> Result<IndexSearcher> {
        let s = IndexSearcher::with_executor(r, self.es.clone())?;
        self.warm_called.store(true, Ordering::SeqCst);
        s.search(body_united(), 10)?;
        Ok(s)
    }
}

impl std::fmt::Debug for WarmingSearcherFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarmingSearcherFactory")
            .field("es", &self.es.is_some())
            .finish_non_exhaustive()
    }
}

/// `TestSearcherManager`.
#[derive(Default)]
struct SearcherManagerSuite {
    state: Arc<ThreadedState>,
    /// `warmCalled`.
    warm_called: Arc<AtomicBool>,
    /// `pruner`.
    pruner: Mutex<Option<PruneByAge>>,
    /// `mgr`.
    mgr: RwLock<Option<Arc<SearcherManager>>>,
    /// `lifetimeMGR`.
    lifetime_mgr: RwLock<Option<Arc<SearcherLifetimeManager>>>,
    /// `pastSearchers`.
    past_searchers: Mutex<Vec<i64>>,
    /// `isNRT`.
    is_nrt: AtomicBool,
}

impl SearcherManagerSuite {
    fn mgr(&self) -> Arc<SearcherManager> {
        self.mgr
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .expect("doAfterWriter built the manager")
    }

    fn lifetime_mgr(&self) -> Arc<SearcherLifetimeManager> {
        self.lifetime_mgr
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .expect("doAfterWriter built the lifetime manager")
    }

    fn pruner(&self) -> PruneByAge {
        self.pruner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .expect("testSearcherManager built the pruner")
    }
}

impl ThreadedIndexingAndSearchingTestCase for SearcherManagerSuite {
    fn state(&self) -> &Arc<ThreadedState> {
        &self.state
    }

    fn get_final_searcher(&self) -> Result<TestSearcher> {
        if !self.is_nrt.load(Ordering::SeqCst) {
            self.state.writer().commit()?;
        }
        let mgr = self.mgr();
        Assert::assert_true(mgr.maybe_refresh()? || mgr.is_searcher_current()?);
        Ok(TestSearcher::Managed(mgr.acquire()?))
    }

    fn do_after_writer(&self, es: Option<Arc<CachedThreadPoolExecutor>>) -> Result<()> {
        let factory: Arc<dyn SearcherFactory> = Arc::new(WarmingSearcherFactory {
            es: executor(&es),
            warm_called: Arc::clone(&self.warm_called),
        });
        let mgr = if LuceneTestCase::random().next_boolean() {
            // TODO: can we randomize the applyAllDeletes?  But
            // somehow for final searcher we must apply
            // deletes...
            let mgr = SearcherManager::from_writer(
                &*self.state.writer(),
                true,
                false,
                Some(factory),
                None,
            )?;
            self.is_nrt.store(true, Ordering::SeqCst);
            mgr
        } else {
            // SearcherManager needs to see empty commit:
            self.state.writer().commit()?;
            let mgr = SearcherManager::from_directory(self.state.dir(), Some(factory))?;
            self.is_nrt.store(false, Ordering::SeqCst);
            self.state
                .assert_merged_segments_warmed
                .store(false, Ordering::SeqCst);
            mgr
        };
        *self.mgr.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(mgr));

        *self.lifetime_mgr.write().unwrap_or_else(|e| e.into_inner()) =
            Some(Arc::new(SearcherLifetimeManager::new()));
        Ok(())
    }

    fn do_searching(
        &self,
        _es: Option<Arc<CachedThreadPoolExecutor>>,
        max_iterations: i32,
    ) -> Result<()> {
        let writer = self.state.writer();
        let mgr = self.mgr();
        let lifetime_mgr = self.lifetime_mgr();
        let pruner = self.pruner();
        let failed_state = Arc::clone(&self.state);
        let reopen_thread =
            Thread::unnamed(move || {
                let outcome =
                    (|| -> Result<()> {
                        if *VERBOSE {
                            println!("[{}]: launch reopen thread", Thread::current().name());
                        }

                        let mut iterations = 0;
                        loop {
                            iterations += 1;
                            if iterations >= max_iterations {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(
                                TestUtil::next_int(&*LuceneTestCase::random(), 1, 100) as u64,
                            ));
                            writer.commit()?;
                            std::thread::sleep(std::time::Duration::from_millis(
                                TestUtil::next_int(&*LuceneTestCase::random(), 1, 5) as u64,
                            ));
                            let block = LuceneTestCase::random().next_boolean();
                            if block {
                                mgr.maybe_refresh_blocking()?;
                                lifetime_mgr.prune(&pruner)?;
                            } else if mgr.maybe_refresh()? {
                                lifetime_mgr.prune(&pruner)?;
                            }
                        }
                        Ok(())
                    })();
                if let Err(e) = outcome {
                    if *VERBOSE {
                        println!("TEST: reopen thread hit exc");
                        println!("{e:?}");
                    }
                    failed_state.failed.store(true, Ordering::SeqCst);
                    randomizedtesting::jdk::exception::Throwable::new(
                        "java.lang.RuntimeException",
                        format!("{e}"),
                    )
                    .throw();
                }
            });
        reopen_thread.set_daemon(true);
        reopen_thread.start();

        run_search_threads(&threaded::this(&self.state), max_iterations)?;

        if let Err(payload) = reopen_thread.join() {
            std::panic::resume_unwind(payload);
        }
        Ok(())
    }

    fn get_current_searcher(&self) -> Result<TestSearcher> {
        let mgr = self.mgr();
        let lifetime_mgr = self.lifetime_mgr();
        if LuceneTestCase::random().next_int_bound(10) == 7 {
            // NOTE: not best practice to call maybeRefresh
            // synchronous to your search threads, but still we
            // test as apps will presumably do this for
            // simplicity:
            if mgr.maybe_refresh()? {
                lifetime_mgr.prune(&self.pruner())?;
            }
        }

        let mut s: Option<Arc<ManagedSearcher>> = None;

        {
            let mut past_searchers = self
                .past_searchers
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            while !past_searchers.is_empty() && LuceneTestCase::random().next_double() < 0.25 {
                // 1/4 of the time pull an old searcher, ie, simulate
                // a user doing a follow-on action on a previous
                // search (drilling down/up, clicking next/prev page,
                // etc.)
                let token = past_searchers
                    [LuceneTestCase::random().next_int_bound(past_searchers.len() as i32) as usize];
                s = lifetime_mgr.acquire(token)?;
                if s.is_none() {
                    // Searcher was pruned
                    past_searchers.retain(|t| *t != token);
                } else {
                    break;
                }
            }
        }

        let s = match s {
            Some(s) => s,
            None => {
                let s = mgr.acquire()?;
                if s.searcher().get_index_reader().num_docs() != 0 {
                    let token = lifetime_mgr.record(&s)?;
                    let mut past_searchers = self
                        .past_searchers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if !past_searchers.contains(&token) {
                        past_searchers.push(token);
                    }
                }
                s
            }
        };

        Ok(TestSearcher::Managed(s))
    }

    fn release_searcher(&self, s: TestSearcher) -> Result<()> {
        s.searcher().get_index_reader().dec_ref()
    }

    fn do_close(&self) -> Result<()> {
        Assert::assert_true(self.warm_called.load(Ordering::SeqCst));
        if *VERBOSE {
            println!("TEST: now close SearcherManager");
        }
        self.mgr().close()?;
        self.lifetime_mgr().close()
    }
}

/// The members of `TestSearcherManager`: `testSearcherManager()`.
fn searcher_manager_members(
    builder: randomizedtesting::jdk::reflect::ClassBuilder,
) -> randomizedtesting::jdk::reflect::ClassBuilder {
    builder
        .annotate(suppress_codecs())
        .constructor(Constructor::no_args(|| ThreadedInstance {
            case: Arc::new(SearcherManagerSuite::default()),
        }))
        .method(
            Method::new("testSearcherManager")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut ThreadedInstance<SearcherManagerSuite>| {
                    let max_age = if *TEST_NIGHTLY {
                        TestUtil::next_int(&*LuceneTestCase::random(), 1, 10)
                    } else {
                        1
                    };
                    *inst.case.pruner.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(PruneByAge::new(f64::from(max_age)).unwrap_or_else(|e| rethrow(e)));
                    run_test(Arc::clone(&inst.case) as _, "TestSearcherManager")
                        .unwrap_or_else(|e| rethrow(e));
                }),
        )
}

fn searcher_manager_class() -> Arc<Class> {
    searcher_manager_members(
        Class::builder("org.apache.lucene.search.TestSearcherManager").extends(&threaded::class()),
    )
    .build()
}

// -----------------------------------------------------------------------------
// TestControlledRealTimeReopenThread
// -----------------------------------------------------------------------------

thread_local! {
    /// `lastGens`, a `ThreadLocal<Long>`.
    static LAST_GENS: Cell<Option<i64>> = const { Cell::new(None) };
}

/// `TestControlledRealTimeReopenThread`.
struct ControlledRealTimeSuite {
    state: Arc<ThreadedState>,
    /// `nrtNoDeletes`: not guaranteed to reflect deletes.
    nrt_no_deletes: RwLock<Option<Arc<SearcherManager>>>,
    /// `nrtDeletes`: is guaranteed to reflect deletes.
    nrt_deletes: RwLock<Option<Arc<SearcherManager>>>,
    /// `nrtDeletesThread`.
    nrt_deletes_thread: RwLock<Option<ControlledRealTimeReopenThread<ManagedSearcher>>>,
    /// `nrtNoDeletesThread`.
    nrt_no_deletes_thread: RwLock<Option<ControlledRealTimeReopenThread<ManagedSearcher>>>,
    /// `warmCalled`.
    warm_called: Arc<AtomicBool>,
    /// `maxGen`.
    max_gen: Mutex<i64>,
}

impl Default for ControlledRealTimeSuite {
    fn default() -> Self {
        ControlledRealTimeSuite {
            state: Arc::default(),
            nrt_no_deletes: RwLock::new(None),
            nrt_deletes: RwLock::new(None),
            nrt_deletes_thread: RwLock::new(None),
            nrt_no_deletes_thread: RwLock::new(None),
            warm_called: Arc::default(),
            max_gen: Mutex::new(-1),
        }
    }
}

/// Which manager and thread a verification uses.
#[derive(Clone, Copy)]
enum Nrt {
    Deletes,
    NoDeletes,
}

impl ControlledRealTimeSuite {
    fn manager(&self, which: Nrt) -> Arc<SearcherManager> {
        match which {
            Nrt::Deletes => &self.nrt_deletes,
            Nrt::NoDeletes => &self.nrt_no_deletes,
        }
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("doAfterWriter built the managers")
    }

    /// `thread.waitForGeneration(gen); assertTrue(gen <= thread.getSearchingGen())`.
    fn wait_for(&self, which: Nrt, gen: i64) {
        let guard = match which {
            Nrt::Deletes => &self.nrt_deletes_thread,
            Nrt::NoDeletes => &self.nrt_no_deletes_thread,
        }
        .read()
        .unwrap_or_else(|e| e.into_inner());
        let thread = guard.as_ref().expect("doAfterWriter started the threads");
        thread.wait_for_generation(gen);
        Assert::assert_true(gen <= thread.get_searching_gen());
    }

    /// The random verification of each indexing hook: once in 20 (with
    /// `trigger`), waits for `gen` on `which` and expects `expected` hits of
    /// `id`.
    fn maybe_verify(
        &self,
        what: &str,
        which: Nrt,
        id: &Term,
        gen: i64,
        expected: i64,
        trigger: i32,
    ) -> Result<()> {
        if LuceneTestCase::random().next_int_bound(20) == trigger {
            if *VERBOSE {
                println!(
                    "{}: nrt: verify {what} {id} gen={gen}",
                    Thread::current().name()
                );
            }
            self.wait_for(which, gen);
            let manager = self.manager(which);
            let s = manager.acquire()?;
            if *VERBOSE {
                let kind = match which {
                    Nrt::Deletes => "deletes",
                    Nrt::NoDeletes => "noDeletes",
                };
                println!(
                    "{}: nrt: got {kind} searcher={:?}",
                    Thread::current().name(),
                    s.searcher()
                );
            }
            let outcome = s
                .searcher()
                .search(Arc::new(TermQuery::new(id.clone())), 10)
                .map(|hits| hits.total_hits.value());
            let released = manager.release(s);
            let hits = outcome?;
            released?;
            if trigger == 7 {
                Assert::assert_equals(expected, hits);
            } else {
                Assert::assert_equals_msg(Some(&format!("generation: {gen}")), expected, hits);
            }
        }
        LAST_GENS.with(|g| g.set(Some(gen)));
        Ok(())
    }

    /// `addMaxGen(gen)` (`synchronized`).
    fn add_max_gen(&self, gen: i64) {
        let mut max_gen = self.max_gen.lock().unwrap_or_else(|e| e.into_inner());
        *max_gen = (*max_gen).max(gen);
    }
}

impl ThreadedIndexingAndSearchingTestCase for ControlledRealTimeSuite {
    fn state(&self) -> &Arc<ThreadedState> {
        &self.state
    }

    fn get_final_searcher(&self) -> Result<TestSearcher> {
        let max_gen = *self.max_gen.lock().unwrap_or_else(|e| e.into_inner());
        if *VERBOSE {
            println!("TEST: finalSearcher maxGen={max_gen}");
        }
        self.nrt_deletes_thread
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .expect("doAfterWriter started the threads")
            .wait_for_generation(max_gen);
        Ok(TestSearcher::Managed(self.manager(Nrt::Deletes).acquire()?))
    }

    fn get_directory(&self, r#in: MockDirectoryWrapper) -> ThreadedDirectory {
        // Randomly swap in NRTCachingDir
        if LuceneTestCase::random().next_boolean() {
            if *VERBOSE {
                println!("TEST: wrap NRTCachingDir");
            }

            ThreadedDirectory::NrtCaching(Arc::new(NRTCachingDirectory::new(
                Box::new(r#in),
                5.0,
                60.0,
            )))
        } else {
            ThreadedDirectory::Mock(r#in)
        }
    }

    fn update_documents(&self, id: Term, docs: &[Document]) -> Result<()> {
        let gen = self.state.writer().update_documents(id.clone(), docs)?;
        // Randomly verify the update "took":
        self.maybe_verify(
            "updateDocuments",
            Nrt::Deletes,
            &id,
            gen,
            docs.len() as i64,
            2,
        )
    }

    fn add_documents(&self, id: Term, docs: &[Document]) -> Result<()> {
        let gen = self.state.writer().add_documents(docs)?;
        // Randomly verify the add "took":
        self.maybe_verify(
            "addDocuments",
            Nrt::NoDeletes,
            &id,
            gen,
            docs.len() as i64,
            2,
        )
    }

    fn add_document(&self, id: Term, doc: &Document) -> Result<()> {
        let gen = self.state.writer().add_document(doc)?;
        // Randomly verify the add "took":
        self.maybe_verify("addDocument", Nrt::NoDeletes, &id, gen, 1, 2)
    }

    fn update_document(&self, id: Term, doc: &Document) -> Result<()> {
        let gen = self.state.writer().update_document(id.clone(), doc)?;
        // Randomly verify the udpate "took":
        self.maybe_verify("updateDocument", Nrt::Deletes, &id, gen, 1, 2)
    }

    fn delete_documents(&self, id: Term) -> Result<()> {
        let gen = self
            .state
            .writer()
            .delete_documents_by_terms(vec![id.clone()])?;
        // randomly verify the delete "took":
        self.maybe_verify("deleteDocuments", Nrt::Deletes, &id, gen, 0, 7)
    }

    fn do_after_writer(&self, es: Option<Arc<CachedThreadPoolExecutor>>) -> Result<()> {
        let min_reopen_sec = 0.01 + 0.05 * LuceneTestCase::random().next_double();
        let max_reopen_sec = min_reopen_sec * (1.0 + 10.0 * LuceneTestCase::random().next_double());

        if *VERBOSE {
            println!(
                "TEST: make SearcherManager maxReopenSec={max_reopen_sec} minReopenSec={min_reopen_sec}"
            );
        }

        let gen_writer = self.state.writer();

        let sf: Arc<dyn SearcherFactory> = Arc::new(WarmingSearcherFactory {
            es: executor(&es),
            warm_called: Arc::clone(&self.warm_called),
        });

        let nrt_no_deletes = Arc::new(SearcherManager::from_writer(
            &*gen_writer,
            false,
            false,
            Some(Arc::clone(&sf)),
            None,
        )?);
        let nrt_deletes = Arc::new(SearcherManager::from_writer(
            &*gen_writer,
            true,
            false,
            Some(sf),
            None,
        )?);
        *self
            .nrt_no_deletes
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&nrt_no_deletes));
        *self.nrt_deletes.write().unwrap_or_else(|e| e.into_inner()) =
            Some(Arc::clone(&nrt_deletes));

        let mut nrt_deletes_thread = ControlledRealTimeReopenThread::new(
            Arc::clone(&gen_writer) as _,
            nrt_deletes,
            max_reopen_sec,
            min_reopen_sec,
        )?;
        nrt_deletes_thread.start();
        *self
            .nrt_deletes_thread
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(nrt_deletes_thread);

        let mut nrt_no_deletes_thread = ControlledRealTimeReopenThread::new(
            gen_writer as _,
            nrt_no_deletes,
            max_reopen_sec,
            min_reopen_sec,
        )?;
        nrt_no_deletes_thread.start();
        *self
            .nrt_no_deletes_thread
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(nrt_no_deletes_thread);
        Ok(())
    }

    fn do_after_indexing_thread_done(&self) {
        if let Some(gen) = LAST_GENS.with(Cell::get) {
            self.add_max_gen(gen);
        }
    }

    fn do_searching(
        &self,
        _es: Option<Arc<CachedThreadPoolExecutor>>,
        max_iterations: i32,
    ) -> Result<()> {
        run_search_threads(&threaded::this(&self.state), max_iterations)
    }

    fn get_current_searcher(&self) -> Result<TestSearcher> {
        // Test doesn't assert deletions until the end, so we
        // can randomize whether dels must be applied
        let nrt = if LuceneTestCase::random().next_boolean() {
            self.manager(Nrt::Deletes)
        } else {
            self.manager(Nrt::NoDeletes)
        };

        Ok(TestSearcher::Managed(nrt.acquire()?))
    }

    fn release_searcher(&self, s: TestSearcher) -> Result<()> {
        // NOTE: a bit iffy... technically you should release
        // against the same SearcherManager you acquired from... but
        // both impls just decRef the underlying reader so we
        // can get away w/ cheating:
        match s {
            TestSearcher::Managed(m) => self.manager(Nrt::NoDeletes).release(m),
            TestSearcher::Plain(p) => p.get_index_reader().dec_ref(),
        }
    }

    fn do_close(&self) -> Result<()> {
        Assert::assert_true(self.warm_called.load(Ordering::SeqCst));
        if *VERBOSE {
            println!("TEST: now close SearcherManagers");
        }
        if let Some(mut t) = self
            .nrt_deletes_thread
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            t.close();
        }
        self.manager(Nrt::Deletes).close()?;
        if let Some(mut t) = self
            .nrt_no_deletes_thread
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            t.close();
        }
        self.manager(Nrt::NoDeletes).close()
    }
}

/// The members of `TestControlledRealTimeReopenThread`:
/// `testControlledRealTimeReopenThread()`.
fn controlled_real_time_members(
    builder: randomizedtesting::jdk::reflect::ClassBuilder,
) -> randomizedtesting::jdk::reflect::ClassBuilder {
    builder
        .annotate(suppress_codecs())
        .constructor(Constructor::no_args(|| ThreadedInstance {
            case: Arc::new(ControlledRealTimeSuite::default()),
        }))
        .method(
            Method::new("testControlledRealTimeReopenThread")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut ThreadedInstance<ControlledRealTimeSuite>| {
                    run_test(
                        Arc::clone(&inst.case) as _,
                        "TestControlledRealTimeReopenThread",
                    )
                    .unwrap_or_else(|e| rethrow(e));
                }),
        )
        .method(crt_test(
            "testThreadStarvationNoDeleteNRTReader",
            crt::test_thread_starvation_no_delete_nrt_reader,
        ))
        .method(crt_test(
            "testEvilSearcherFactory",
            crt::test_evil_searcher_factory,
        ))
        .method(crt_test("testListenerCalled", crt::test_listener_called))
        .method(
            crt_test("testCRTReopen", crt::test_crt_reopen)
                // Relies on wall clock time, so it can easily false-fail when the machine is otherwise busy:
                .annotate(rucene_test_framework::util::lucene_test_case::AwaitsFix {
                    bug_url: "https://issues.apache.org/jira/browse/LUCENE-5737".to_string(),
                }),
        )
        .method(crt_test("testDeleteAll", crt::test_delete_all))
}

/// A `public void name() throws Exception` of `TestControlledRealTimeReopenThread`.
fn crt_test(
    name: &str,
    body: fn(&mut ThreadedInstance<ControlledRealTimeSuite>) -> Result<()>,
) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut ThreadedInstance<ControlledRealTimeSuite>| {
            body(inst).unwrap_or_else(|e| rethrow(e));
        },
    )
}

/// The test methods of `TestControlledRealTimeReopenThread` that do not run
/// `runTest`.
mod crt {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    use randomizedtesting::jdk::thread::Thread;
    use randomizedtesting::junit::Assert;
    use rucene::document::{Document, Store, TextField};
    use rucene::error::Result;
    use rucene::index::directory_reader::{self, DirectoryReader};
    use rucene::index::documents_writer::SharedDirectory;
    use rucene::index::index_deletion_policy::{
        KeepOnlyLastCommitDeletionPolicy, SnapshotDeletionPolicy,
    };
    use rucene::index::index_writer::IndexWriter;
    use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
    use rucene::index::merge_policy::NoMergePolicy;
    use rucene::index::terms::Term;
    use rucene::index::IndexReader;
    use rucene::search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread;
    use rucene::search::reference_manager::RefreshListener;
    use rucene::search::{
        DefaultSearcherFactory, IndexSearcher, SearcherFactory, SearcherManager, TermQuery,
    };
    use rucene::store::{Directory, NRTCachingDirectory};
    use rucene_test_framework::analysis::MockAnalyzer;
    use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
    use rucene_test_framework::util::java_exceptions::expect_throws;
    use rucene_test_framework::util::lucene_test_case::VERBOSE;
    use rucene_test_framework::util::LuceneTestCase;

    use super::{ControlledRealTimeSuite, ThreadedInstance};

    /// `java.util.concurrent.CountDownLatch`.
    pub struct CountDownLatch {
        count: Mutex<i64>,
        zero: Condvar,
    }

    impl CountDownLatch {
        pub fn new(count: i64) -> CountDownLatch {
            CountDownLatch {
                count: Mutex::new(count),
                zero: Condvar::new(),
            }
        }

        pub fn count_down(&self) {
            let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
            if *count > 0 {
                *count -= 1;
                if *count == 0 {
                    self.zero.notify_all();
                }
            }
        }

        pub fn await_(&self) {
            let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
            while *count > 0 {
                count = self.zero.wait(count).unwrap_or_else(|e| e.into_inner());
            }
        }
    }

    /// `LatchedIndexWriter extends IndexWriter`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java subclasses `IndexWriter` and
    /// overrides `updateDocument`; Rucene's `IndexWriter` is a concrete
    /// type, so this wrapper holds the writer and adds the latch handshake
    /// to the `updateDocument` calls the test makes through it. The
    /// `SearcherManager` and the reopen thread take the wrapped writer;
    /// neither calls `updateDocument`.
    pub struct LatchedIndexWriter {
        pub writer: Arc<IndexWriter>,
        latch: Arc<CountDownLatch>,
        pub wait_after_update: AtomicBool,
        signal: Arc<CountDownLatch>,
    }

    impl LatchedIndexWriter {
        pub fn new(
            d: Arc<dyn Directory>,
            conf: IndexWriterConfig,
            latch: Arc<CountDownLatch>,
            signal: Arc<CountDownLatch>,
        ) -> Result<LatchedIndexWriter> {
            Ok(LatchedIndexWriter {
                writer: IndexWriter::new(d, conf)?,
                latch,
                wait_after_update: AtomicBool::new(false),
                signal,
            })
        }

        /// `updateDocument(term, doc)`.
        pub fn update_document(&self, term: Term, doc: &Document) -> Result<i64> {
            let result = self.writer.update_document(term, doc)?;
            // Java maps an `InterruptedException` of `latch.await()` to a
            // `ThreadInterruptedException`; this latch is not interruptible.
            if self.wait_after_update.load(Ordering::SeqCst) {
                self.signal.count_down();
                self.latch.await_();
            }
            Ok(result)
        }
    }

    /*
     * LUCENE-3528 - NRTManager hangs in certain situations
     */
    pub fn test_thread_starvation_no_delete_nrt_reader(
        _inst: &mut ThreadedInstance<ControlledRealTimeSuite>,
    ) -> Result<()> {
        let mut conf = LuceneTestCase::new_index_writer_config_analyzer(Arc::new(
            MockAnalyzer::new(&*LuceneTestCase::random()),
        ));
        conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
        let mut d = LuceneTestCase::new_directory();
        let latch = Arc::new(CountDownLatch::new(1));
        let signal = Arc::new(CountDownLatch::new(1));

        let writer = Arc::new(LatchedIndexWriter::new(
            d.as_directory(),
            conf,
            Arc::clone(&latch),
            Arc::clone(&signal),
        )?);
        let manager = Arc::new(SearcherManager::from_writer(
            &*writer.writer,
            false,
            false,
            None,
            None,
        )?);
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "test",
            "test",
            Store::YES,
        )));
        writer.writer.add_document(&doc)?;
        manager.maybe_refresh()?;
        let t = {
            let (signal, latch) = (Arc::clone(&signal), Arc::clone(&latch));
            let (manager, writer) = (Arc::clone(&manager), Arc::clone(&writer));
            Thread::unnamed(move || {
                let run =
                    || -> Result<()> {
                        signal.await_();
                        manager.maybe_refresh()?;
                        writer.writer.delete_documents_by_queries(vec![Arc::new(
                            TermQuery::new(Term::from_text("foo", "barista")),
                        )])?;
                        manager.maybe_refresh()?; // kick off another reopen so we inc. the internal gen
                        Ok(())
                    };
                if let Err(e) = run() {
                    // e.printStackTrace()
                    eprintln!("{e}");
                }
                latch.count_down(); // let the add below finish
            })
        };
        t.start();
        writer.wait_after_update.store(true, Ordering::SeqCst); // wait in addDocument to let some reopens go through

        let last_gen = writer.update_document(Term::from_text("foo", "bar"), &doc)?; // once this returns the doc is already reflected in the last reopen

        // We now eagerly resolve deletes so the manager should see it after update:
        Assert::assert_true(manager.is_searcher_current()?);

        let searcher = manager.acquire()?;
        let num_docs = searcher.searcher().get_index_reader().num_docs();
        manager.release(searcher)?;
        Assert::assert_equals(2i64, num_docs as i64);
        let mut thread = ControlledRealTimeReopenThread::new(
            Arc::clone(&writer.writer) as _,
            Arc::clone(&manager),
            0.01,
            0.01,
        )?;
        thread.start(); // start reopening
        if *VERBOSE {
            println!("waiting now for generation {last_gen}");
        }

        let thread = Arc::new(thread);
        let finished = Arc::new(AtomicBool::new(false));
        let waiter = {
            let (thread, finished) = (Arc::clone(&thread), Arc::clone(&finished));
            // Java catches the `InterruptedException` of `waitForGeneration`;
            // Rucene's wait is not interruptible.
            Thread::unnamed(move || {
                thread.wait_for_generation(last_gen);
                finished.store(true, Ordering::SeqCst);
            })
        };
        waiter.start();
        manager.maybe_refresh()?;
        waiter.join_millis(1000);
        if !finished.load(Ordering::SeqCst) {
            waiter.interrupt();
            Assert::fail("thread deadlocked on waitForGeneration");
        }
        // The waiter dropped its handle on the reopen thread when it ended;
        // `close()` needs the only one.
        let _ = waiter.join();
        let mut thread = Arc::try_unwrap(thread)
            .unwrap_or_else(|_| panic!("the waiter released the reopen thread"));
        thread.close();
        // `thread.join()`: `close()` already joined the reopen thread.
        writer.writer.close()?;
        // IOUtils.close(manager, d)
        manager.close()?;
        d.close()?;
        Ok(())
    }

    /// The anonymous `SearcherFactory` of `testEvilSearcherFactory`.
    #[derive(Debug)]
    struct EvilSearcherFactory {
        other: Arc<dyn DirectoryReader>,
    }

    impl SearcherFactory for EvilSearcherFactory {
        fn new_searcher(
            &self,
            _ignored: Arc<dyn IndexReader>,
            _previous: Option<Arc<dyn IndexReader>>,
        ) -> Result<IndexSearcher> {
            Ok(LuceneTestCase::new_searcher(Arc::clone(&self.other)))
        }
    }

    pub fn test_evil_searcher_factory(
        _inst: &mut ThreadedInstance<ControlledRealTimeSuite>,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(
            &*LuceneTestCase::random(),
            dir.as_directory(),
        )?;
        w.commit()?;

        let other = directory_reader::open(dir.as_directory())?;

        let the_evil_one: Arc<dyn SearcherFactory> = Arc::new(EvilSearcherFactory {
            other: Arc::clone(&other),
        });

        expect_throws("java.lang.IllegalStateException", || {
            SearcherManager::from_writer(
                &**w.w(),
                false,
                false,
                Some(Arc::clone(&the_evil_one)),
                None,
            )
        });

        w.close()?;
        other.close()?;
        dir.close()?;
        Ok(())
    }

    /// The anonymous `ReferenceManager.RefreshListener` of `testListenerCalled`.
    struct AfterRefreshListener {
        after_refresh_called: Arc<AtomicBool>,
    }

    impl RefreshListener for AfterRefreshListener {
        fn before_refresh(&self) -> Result<()> {
            Ok(())
        }

        fn after_refresh(&self, did_refresh: bool) -> Result<()> {
            if did_refresh {
                self.after_refresh_called.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    /// `new IndexWriterConfig(null)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java passes a `null` analyzer;
    /// Rucene's configuration always holds one, so the default
    /// `IndexWriterConfig()` analyzer stands for it. The tests that use it
    /// index no tokenized field, so no analyzer is ever asked for.
    fn iwc_null_analyzer() -> IndexWriterConfig {
        IndexWriterConfig::new()
    }

    pub fn test_listener_called(
        _inst: &mut ThreadedInstance<ControlledRealTimeSuite>,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc_null_analyzer())?;
        let after_refresh_called = Arc::new(AtomicBool::new(false));
        let sm = SearcherManager::from_writer(
            &*iw,
            true,
            false,
            Some(Arc::new(DefaultSearcherFactory)),
            None,
        )?;
        sm.add_listener(Arc::new(AfterRefreshListener {
            after_refresh_called: Arc::clone(&after_refresh_called),
        }));
        iw.add_document(&Document::new())?;
        iw.commit()?;
        Assert::assert_false(after_refresh_called.load(Ordering::SeqCst));
        sm.maybe_refresh_blocking()?;
        Assert::assert_true(after_refresh_called.load(Ordering::SeqCst));
        sm.close()?;
        iw.close()?;
        dir.close()?;
        Ok(())
    }

    // LUCENE-5461
    pub fn test_crt_reopen(_inst: &mut ThreadedInstance<ControlledRealTimeSuite>) -> Result<()> {
        // test behaving badly

        // should be high enough
        let max_stale_secs = 20;

        // build crap data just to store it.
        let s = "        abcdefghijklmnopqrstuvwxyz     ";
        let chars: Vec<char> = s.chars().collect();
        let mut builder = String::with_capacity(2048);
        for _i in 0..2048 {
            builder
                .push(chars[LuceneTestCase::random().next_int_bound(chars.len() as i32) as usize]);
        }
        let content = builder;

        let sdp = Arc::new(SnapshotDeletionPolicy::new(Arc::new(
            KeepOnlyLastCommitDeletionPolicy,
        )));
        let mut fs_dir =
            LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("nrt"));
        let dir: Arc<dyn Directory> = Arc::new(NRTCachingDirectory::new(
            Box::new(SharedDirectory(fs_dir.as_directory())),
            5.0,
            128.0,
        ));
        let mut config = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(
            &*LuceneTestCase::random(),
        )));
        config.set_commit_on_close(true);
        config.set_index_deletion_policy(Arc::clone(&sdp) as _)?;
        config.set_open_mode(OpenMode::CREATE_OR_APPEND)?;
        let iw = IndexWriter::new(Arc::clone(&dir), config)?;
        let sm = Arc::new(SearcherManager::from_writer(
            &*iw,
            true,
            false,
            Some(Arc::new(DefaultSearcherFactory)),
            None,
        )?);
        let mut controlled_real_time_reopen_thread = ControlledRealTimeReopenThread::new(
            Arc::clone(&iw) as _,
            Arc::clone(&sm),
            max_stale_secs as f64,
            0.0,
        )?;

        // `setDaemon(true)`: not a `java.lang.Thread` here (see the module
        // documentation).
        controlled_real_time_reopen_thread.start();

        let mut commit_threads: Vec<Thread> = Vec::new();

        for i in 0..500 {
            if i > 0 && i % 50 == 0 {
                let (iw, sdp, dir) = (Arc::clone(&iw), Arc::clone(&sdp), Arc::clone(&dir));
                let commit_thread = Thread::unnamed(move || {
                    let run = || -> Result<()> {
                        iw.commit()?;
                        let ic = sdp.snapshot()?;
                        for name in ic.get_file_names()? {
                            // distribute, and backup
                            // System.out.println(names);
                            Assert::assert_true(LuceneTestCase::slow_file_exists(&*dir, &name)?);
                        }
                        Ok(())
                    };
                    if let Err(e) = run() {
                        // `throw new RuntimeException(e)`
                        rucene_test_framework::util::java_exceptions::rethrow_java(e);
                    }
                });
                commit_thread.start();
                commit_threads.push(commit_thread);
            }
            let mut d = Document::new();
            d.add(Box::new(TextField::new(
                "count",
                format!("{i}"),
                Store::NO,
            )?));
            d.add(Box::new(TextField::new(
                "content",
                content.clone(),
                Store::YES,
            )?));
            let start = std::time::Instant::now();
            let l = iw.add_document(&d)?;
            controlled_real_time_reopen_thread.wait_for_generation(l);
            let wait = start.elapsed().as_nanos() as i64;
            Assert::assert_true_msg(
                &format!("waited too long for generation {wait}"),
                wait < (max_stale_secs as i64 * 1_000_000_000),
            );
            let searcher = sm.acquire()?;
            let td = searcher.searcher().search(
                Arc::new(TermQuery::new(Term::from_text("count", &format!("{i}")))),
                10,
            );
            sm.release(searcher)?;
            Assert::assert_equals(1i64, td?.total_hits.value());
        }

        for commit_thread in &commit_threads {
            let _ = commit_thread.join();
        }

        controlled_real_time_reopen_thread.close();
        sm.close()?;
        iw.close()?;
        // `dir.close()`: the caching directory, then the wrapped
        // `newFSDirectory` it closes in Java.
        drop(iw);
        let mut dir = dir;
        if let Some(d) = Arc::get_mut(&mut dir) {
            d.close()?;
        }
        fs_dir.close()?;
        Ok(())
    }

    pub fn test_delete_all(inst: &mut ThreadedInstance<ControlledRealTimeSuite>) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let mgr = Arc::new(SearcherManager::from_writer(
            &*w,
            true,
            false,
            Some(Arc::new(DefaultSearcherFactory)),
            None,
        )?);
        let mut nrt_deletes_thread =
            ControlledRealTimeReopenThread::new(Arc::clone(&w) as _, mgr, 0.1, 0.01)?;
        // `setName`, `setDaemon`: not a `java.lang.Thread` here (see the
        // module documentation).
        nrt_deletes_thread.start();
        *inst
            .case
            .nrt_deletes_thread
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(nrt_deletes_thread);

        w.add_document(&Document::new())?;
        let gen2 = w.delete_all()?;
        if let Some(t) = inst
            .case
            .nrt_deletes_thread
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            t.wait_for_generation(gen2);
        }
        // IOUtils.close(nrtDeletesThread, nrtDeletes, w, dir): `nrtDeletes`
        // is still `null` in this test, which `IOUtils.close` skips.
        if let Some(mut t) = inst
            .case
            .nrt_deletes_thread
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            t.close();
        }
        if let Some(nrt_deletes) = inst
            .case
            .nrt_deletes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            nrt_deletes.close()?;
        }
        w.close()?;
        dir.close()?;
        Ok(())
    }
}

fn controlled_real_time_class() -> Arc<Class> {
    controlled_real_time_members(
        Class::builder("org.apache.lucene.search.TestControlledRealTimeReopenThread")
            .extends(&threaded::class()),
    )
    .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestThreadedSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestThreadedSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestThreadedSuiteDraws` of `BaseFormatSuitesOracle`: the members of
/// `TestNRTThreads`, recording the next `random().nextLong()` after each
/// test (in `tearDown`, before `super.tearDown()`) and after the class.
fn draws_class() -> Arc<Class> {
    nrt_threads_members(
        Class::builder("org.apache.lucene.tests.index.TestThreadedSuiteDraws")
            .extends(&threaded::class()),
    )
    .method(
        Method::new("tearDown")
            .throws(&["java.lang.Exception"])
            .body(|_inst: &mut ThreadedInstance<NrtThreads>| {
                DRAWS.lock().unwrap().push(format!(
                    "{}\tnext\t{}",
                    LuceneTestCase::get_test_name().unwrap_or_default(),
                    LuceneTestCase::random().next_long()
                ));
                LuceneTestCase::super_tear_down();
            }),
    )
    .method(
        Method::new("recordClassDraw")
            .modifiers(modifier::PUBLIC | modifier::STATIC)
            .annotate(randomizedtesting::junit::AfterClass)
            .static_body(|| {
                DRAWS.lock().unwrap().push(format!(
                    "#class\tnext\t{}",
                    LuceneTestCase::random().next_long()
                ));
            }),
    )
    .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #355): every row of
/// `threaded_draws.tsv` (`BaseFormatSuitesOracle`) must be equal, run under
/// the JVM system properties the fixture records.
///
/// Two rows are left out: the `next` rows of `testNRTThreads` on DEADBEEF and
/// on `1`. Java itself does not reproduce them: in 8 Java runs (5 at once on
/// all cores, 3 pinned to 1, 1 and 2 cores) DEADBEEF gave 6 different values,
/// and `1` gave -6132905986733097946 seven times and -9113230656829955279
/// once (on 2 cores); the Rust runs give values from the same sets. The
/// cause is inferred, not traced: the merged-segment warmer `runTest`
/// installs (`ThreadedIndexingAndSearchingTestCase.java:538-562`) calls
/// `newSearcher(reader, false)`, which draws from `random()` of whichever
/// thread runs the merge, the main thread or an indexing thread, depending
/// on timing. The test still runs, and its failure and `#counts` rows are
/// still compared.
#[test]
fn test_threaded_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match_excluding(
        "threaded_draws.tsv",
        draws_class,
        &DRAWS,
        &[("DEADBEEF", "testNRTThreads"), ("1", "testNRTThreads")],
    );
}

#[test]
fn test_nrt_threads() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(nrt_threads_class);
}

#[test]
fn test_searcher_manager() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(searcher_manager_class);
}

#[test]
fn test_controlled_real_time_reopen_thread() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(controlled_real_time_class);
}
