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

//! Port of `org.apache.lucene.tests.index.ThreadedIndexingAndSearchingTestCase`:
//! utility class that spawns multiple indexing and searching threads.
//!
//! The class is declared by [`class()`] (`public abstract class
//! ThreadedIndexingAndSearchingTestCase extends LuceneTestCase`, which
//! declares no test of its own); its members are the trait
//! [`ThreadedIndexingAndSearchingTestCase`], and its fields — which the
//! indexing and searching threads share with the subclass — are
//! [`ThreadedState`]. [`run_test`] and [`run_search_threads`] take the
//! subclass as an `Arc`, because the threads they start hold it.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `Executors.newCachedThreadPool(new NamedThreadFactory(testName))`:
//!   [`CachedThreadPoolExecutor`] runs each command on a new thread of the
//!   factory instead of reusing idle ones; what runs, and on which named
//!   threads, is the same.
//! * `warmed`, a synchronized `WeakHashMap<Object, Boolean>` keyed by
//!   segment cores: the port keeps weak references to the cores, keyed by
//!   their address, and a core counts as warmed only while it is alive.
//! * `IndexSearcher getCurrentSearcher()`: the searcher a
//!   `SearcherManager` hands out is the port's `ManagedSearcher`, which keeps
//!   the typed reader beside the searcher; [`TestSearcher`] is either kind.
//! * `LineFileDocs.nextDoc()` never returns `null` in the port (nor in
//!   Lucene's implementation); the `doc == null` checks are kept as the
//!   loop structure only.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::MergedSegmentWarmer;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader};
use rucene::internal::tests_hooks::TestSecrets;
use rucene::search::searcher_manager::ManagedSearcher;
use rucene::search::{
    Command, Executor, IndexSearcher, PhraseQuery, Query, RejectedExecutionException, Sort,
    SortField, SortFieldType, TermQuery,
};
use rucene::store::{Directory, NRTCachingDirectory};
use rucene::util::concurrent::NamedThreadFactory;
use rucene::util::{BytesRef, InfoStream, PrintStreamInfoStream};

use super::base_index_file_format_test_case::{assert_not_null, rethrow};
use super::mock_random_merge_policy::MockRandomMergePolicy;
use crate::analysis::MockAnalyzer;
use crate::store::MockDirectoryWrapper;
use crate::util::fail_on_non_bulk_merges_info_stream::FailOnNonBulkMergesInfoStream;
use crate::util::lucene_test_case::{LuceneTestCase, RANDOM_MULTIPLIER, TEST_NIGHTLY, VERBOSE};
use crate::util::{LineFileDocs, TestUtil};

/// The declaration of
/// `org.apache.lucene.tests.index.ThreadedIndexingAndSearchingTestCase`,
/// which declares no test.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.index.ThreadedIndexingAndSearchingTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// The searcher `getCurrentSearcher()` and `getFinalSearcher()` answer: a
/// plain `IndexSearcher`, or the one a `SearcherManager` manages.
#[derive(Clone)]
pub enum TestSearcher {
    /// An `IndexSearcher` the test built.
    Plain(Arc<IndexSearcher>),
    /// The searcher of a `SearcherManager` (or `SearcherLifetimeManager`).
    Managed(Arc<ManagedSearcher>),
}

impl TestSearcher {
    /// The searcher.
    pub fn searcher(&self) -> &IndexSearcher {
        match self {
            TestSearcher::Plain(s) => s,
            TestSearcher::Managed(m) => m.searcher(),
        }
    }

    /// `s == other` on the searcher objects.
    pub fn same(&self, other: &TestSearcher) -> bool {
        match (self, other) {
            (TestSearcher::Plain(a), TestSearcher::Plain(b)) => Arc::ptr_eq(a, b),
            (TestSearcher::Managed(a), TestSearcher::Managed(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// The directory `getDirectory(newMockFSDirectory(tempDir))` answers: the
/// mock directory itself, or a wrapper that owns it.
pub enum ThreadedDirectory {
    /// The `MockDirectoryWrapper` (`dir instanceof BaseDirectoryWrapper`).
    Mock(MockDirectoryWrapper),
    /// `new NRTCachingDirectory(in, 5.0, 60.0)` over the mock directory.
    NrtCaching(Arc<NRTCachingDirectory>),
}

impl ThreadedDirectory {
    /// The directory as the writer and readers take it.
    pub fn as_directory(&self) -> Arc<dyn Directory> {
        match self {
            ThreadedDirectory::Mock(d) => d.as_directory(),
            ThreadedDirectory::NrtCaching(d) => Arc::clone(d) as Arc<dyn Directory>,
        }
    }

    /// `(MockDirectoryWrapper) dir`.
    pub fn as_mock(&self) -> Option<&MockDirectoryWrapper> {
        match self {
            ThreadedDirectory::Mock(d) => Some(d),
            ThreadedDirectory::NrtCaching(_) => None,
        }
    }

    /// `dir.close()`.
    ///
    /// # Errors
    ///
    /// The directory's close errors; `IllegalStateException` when a caching
    /// directory is still referenced.
    pub fn close(self) -> Result<()> {
        match self {
            ThreadedDirectory::Mock(mut d) => d.close(),
            ThreadedDirectory::NrtCaching(d) => match Arc::try_unwrap(d) {
                Ok(mut d) => d.close(),
                Err(_) => Err(LuceneError::illegal_state(
                    "the NRTCachingDirectory is still referenced".to_string(),
                )),
            },
        }
    }
}

/// `Executors.newCachedThreadPool(new NamedThreadFactory(name))` (see the
/// module docs).
pub struct CachedThreadPoolExecutor {
    factory: NamedThreadFactory,
    shutdown: AtomicBool,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl CachedThreadPoolExecutor {
    /// The pool over `new NamedThreadFactory(prefix)`.
    pub fn new(prefix: &str) -> Self {
        CachedThreadPoolExecutor {
            factory: NamedThreadFactory::new(prefix),
            shutdown: AtomicBool::new(false),
            threads: Mutex::new(Vec::new()),
        }
    }

    /// `ExecutorService.shutdown()`.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// `ExecutorService.awaitTermination(timeout)`: whether every command
    /// finished before the timeout.
    pub fn await_termination(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let all_done = self
                .threads
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .all(std::thread::JoinHandle::is_finished);
            if all_done {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Executor for CachedThreadPoolExecutor {
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(RejectedExecutionException {
                command,
                message: "Task rejected from a shut down CachedThreadPoolExecutor".to_string(),
            });
        }
        match self.factory.new_thread(command) {
            Ok(handle) => {
                self.threads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(handle);
                Ok(())
            }
            Err(e) => rethrow(e),
        }
    }
}

/// The private `SubDocs`: a pack of documents added together.
#[derive(Debug)]
struct SubDocs {
    pack_id: String,
    sub_ids: Vec<String>,
    deleted: AtomicBool,
}

/// The fields of `ThreadedIndexingAndSearchingTestCase`, shared by the
/// subclass and the threads.
pub struct ThreadedState {
    /// `failed`.
    pub failed: AtomicBool,
    /// `addCount`.
    pub add_count: AtomicI32,
    /// `delCount`.
    pub del_count: AtomicI32,
    /// `packCount`.
    pub pack_count: AtomicI32,
    /// `maxAdvancedSegmentCounter`.
    pub max_advanced_segment_counter: AtomicI64,
    /// `dir`.
    dir: Mutex<Option<ThreadedDirectory>>,
    /// The directory's shared handle, for the threads.
    dir_handle: RwLock<Option<Arc<dyn Directory>>>,
    /// `writer`.
    writer: RwLock<Option<Arc<IndexWriter>>>,
    /// `assertMergedSegmentsWarmed`.
    pub assert_merged_segments_warmed: AtomicBool,
    /// `warmed` (see the module docs).
    warmed: Mutex<HashMap<usize, Weak<dyn std::any::Any + Send + Sync>>>,
    /// The subclass, for the threads (`this` of the anonymous threads).
    this: RwLock<Option<Weak<dyn ThreadedIndexingAndSearchingTestCase>>>,
}

impl Default for ThreadedState {
    fn default() -> Self {
        ThreadedState {
            failed: AtomicBool::new(false),
            add_count: AtomicI32::new(0),
            del_count: AtomicI32::new(0),
            pack_count: AtomicI32::new(0),
            max_advanced_segment_counter: AtomicI64::new(0),
            dir: Mutex::new(None),
            dir_handle: RwLock::new(None),
            writer: RwLock::new(None),
            assert_merged_segments_warmed: AtomicBool::new(true),
            warmed: Mutex::new(HashMap::new()),
            this: RwLock::new(None),
        }
    }
}

impl ThreadedState {
    /// `writer`.
    ///
    /// # Panics
    ///
    /// `NullPointerException` before `runTest` opens it.
    pub fn writer(&self) -> Arc<IndexWriter> {
        self.writer
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            })
    }

    /// `dir`, as the writer and readers take it.
    ///
    /// # Panics
    ///
    /// `NullPointerException` before `runTest` opens it.
    pub fn dir(&self) -> Arc<dyn Directory> {
        self.dir_handle
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            })
    }

    /// Runs `f` on `dir` (`((MockDirectoryWrapper) dir).…` in the subclasses).
    pub fn with_dir<R>(&self, f: impl FnOnce(&ThreadedDirectory) -> R) -> R {
        let guard = self.dir.lock().unwrap_or_else(|e| e.into_inner());
        f(guard.as_ref().unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        }))
    }

    /// The subclass, as the threads reach it.
    fn this(&self) -> Arc<dyn ThreadedIndexingAndSearchingTestCase> {
        self.this
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
            .expect("runTest set the test instance")
    }

    /// `warmed.containsKey(core)`.
    fn is_warmed(&self, core: &Arc<dyn std::any::Any + Send + Sync>) -> bool {
        let key = Arc::as_ptr(core) as *const () as usize;
        self.warmed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .and_then(Weak::upgrade)
            .is_some_and(|k| Arc::ptr_eq(&k, core))
    }

    /// `warmed.put(core, Boolean.TRUE)`.
    fn put_warmed(&self, core: &Arc<dyn std::any::Any + Send + Sync>) {
        let key = Arc::as_ptr(core) as *const () as usize;
        self.warmed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, Arc::downgrade(core));
    }

    /// The text of `warmed.toString()`, for the assertion message.
    fn warmed_string(&self) -> String {
        let warmed = self.warmed.lock().unwrap_or_else(|e| e.into_inner());
        let entries: Vec<String> = warmed
            .iter()
            .filter(|(_, v)| v.upgrade().is_some())
            .map(|(k, _)| format!("{k:x}=true"))
            .collect();
        format!("{{{}}}", entries.join(", "))
    }
}

/// A Rucene error as the `Throwable` Java's `catch (Throwable t)` holds.
fn throwable_of(e: &LuceneError) -> Throwable {
    let (class, message) = crate::analysis::base_token_stream_test_case::java_class_and_message(e);
    Throwable::new(class, message)
}

/// `Thread.currentThread().getName()`.
fn thread_name() -> String {
    Thread::current().name()
}

/// `Thread.sleep(millis)`.
fn sleep_millis(millis: i32) {
    std::thread::sleep(Duration::from_millis(millis.max(0) as u64));
}

/// `doc.get(name)`.
fn doc_get(doc: &Document, name: &str) -> Option<String> {
    doc.get_values(name).into_iter().next()
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::new(field, BytesRef::new(text.as_bytes().to_vec()))
}

/// `new TermQuery(new Term(field, text))`.
fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// Port of the members of `ThreadedIndexingAndSearchingTestCase`.
pub trait ThreadedIndexingAndSearchingTestCase: Send + Sync + 'static {
    /// The fields of the class.
    fn state(&self) -> &Arc<ThreadedState>;

    /// `getCurrentSearcher()`: called per-search.
    ///
    /// # Errors
    ///
    /// The subclass's errors.
    fn get_current_searcher(&self) -> Result<TestSearcher>;

    /// `getFinalSearcher()`.
    ///
    /// # Errors
    ///
    /// The subclass's errors.
    fn get_final_searcher(&self) -> Result<TestSearcher>;

    /// `releaseSearcher(s)`.
    ///
    /// # Errors
    ///
    /// The subclass's errors.
    fn release_searcher(&self, _s: TestSearcher) -> Result<()> {
        Ok(())
    }

    /// `doSearching(es, maxIterations)`: called once to run searching.
    ///
    /// # Errors
    ///
    /// The subclass's errors.
    fn do_searching(
        &self,
        es: Option<Arc<CachedThreadPoolExecutor>>,
        max_iterations: i32,
    ) -> Result<()>;

    /// `getDirectory(in)`.
    fn get_directory(&self, r#in: MockDirectoryWrapper) -> ThreadedDirectory {
        ThreadedDirectory::Mock(r#in)
    }

    /// `updateDocuments(id, docs)`.
    ///
    /// # Errors
    ///
    /// The writer's errors.
    fn update_documents(&self, id: Term, docs: &[Document]) -> Result<()> {
        self.state().writer().update_documents(id, docs)?;
        Ok(())
    }

    /// `addDocuments(id, docs)`.
    ///
    /// # Errors
    ///
    /// The writer's errors.
    fn add_documents(&self, _id: Term, docs: &[Document]) -> Result<()> {
        self.state().writer().add_documents(docs)?;
        Ok(())
    }

    /// `addDocument(id, doc)`.
    ///
    /// # Errors
    ///
    /// The writer's errors.
    fn add_document(&self, _id: Term, doc: &Document) -> Result<()> {
        self.state().writer().add_document(doc)?;
        Ok(())
    }

    /// `updateDocument(term, doc)`.
    ///
    /// # Errors
    ///
    /// The writer's errors.
    fn update_document(&self, term: Term, doc: &Document) -> Result<()> {
        self.state().writer().update_document(term, doc)?;
        Ok(())
    }

    /// `deleteDocuments(term)`.
    ///
    /// # Errors
    ///
    /// The writer's errors.
    fn delete_documents(&self, term: Term) -> Result<()> {
        self.state()
            .writer()
            .delete_documents_by_terms(vec![term])?;
        Ok(())
    }

    /// `doAfterIndexingThreadDone()`.
    fn do_after_indexing_thread_done(&self) {}

    /// `doAfterWriter(es)`.
    ///
    /// # Errors
    ///
    /// The subclass's errors.
    fn do_after_writer(&self, _es: Option<Arc<CachedThreadPoolExecutor>>) -> Result<()> {
        Ok(())
    }

    /// `doClose()`.
    ///
    /// # Errors
    ///
    /// The subclass's errors.
    fn do_close(&self) -> Result<()> {
        Ok(())
    }
}

/// The body of one indexing thread of `launchIndexingThreads`.
#[allow(clippy::too_many_arguments)]
fn indexing_thread_body(
    case: &Arc<dyn ThreadedIndexingAndSearchingTestCase>,
    docs: &LineFileDocs,
    max_iterations: i32,
    del_ids: &Mutex<HashSet<String>>,
    del_pack_ids: &Mutex<HashSet<String>>,
    all_sub_docs: &Mutex<Vec<Arc<SubDocs>>>,
) -> Result<()> {
    let state = Arc::clone(case.state());
    // TODO: would be better if this were cross thread, so that we make sure one thread
    // deleting anothers added docs works:
    let mut to_delete_ids: Vec<String> = Vec::new();
    let mut to_delete_sub_docs: Vec<Arc<SubDocs>> = Vec::new();
    let mut iterations = 0;
    loop {
        iterations += 1;
        if !(iterations < max_iterations && !state.failed.load(Ordering::SeqCst)) {
            break;
        }
        // Occasional longish pause if running
        // nightly
        if *TEST_NIGHTLY && random().next_int_bound(6) == 3 {
            if *VERBOSE {
                println!("{}: now long sleep", thread_name());
            }
            sleep_millis(TestUtil::next_int(&*random(), 50, 500));
        }

        // Rate limit ingest rate:
        if random().next_int_bound(7) == 5 {
            sleep_millis(TestUtil::next_int(&*random(), 1, 10));
            if *VERBOSE {
                println!("{}: done sleep", thread_name());
            }
        }

        let mut doc = docs.next_doc()?;

        // Maybe add randomly named field
        let added_field = if random().next_boolean() {
            let added_field = format!("extra{}", random().next_int_bound(40));
            doc.add(Box::new(LuceneTestCase::new_text_field(
                &added_field,
                "a random field",
                Store::YES,
            )));
            Some(added_field)
        } else {
            None
        };

        // Maybe advance segment counter. Run with a relatively low probability to avoid
        // testing slowdowns caused by synchronous operations.
        if random().next_int_bound(7) == 5 {
            let writer = state.writer();
            let new_segment_counter = writer.get_segment_infos_counter()? + 100;
            writer.advance_segment_infos_counter(new_segment_counter)?;
            if *VERBOSE {
                println!(
                    "{} advance segment counter to {new_segment_counter}",
                    thread_name()
                );
            }
            state
                .max_advanced_segment_counter
                .fetch_max(new_segment_counter, Ordering::SeqCst);
        }

        if random().next_boolean() {
            if random().next_boolean() {
                // Add/update doc block:
                let pack_id: String;
                let del_sub_docs: Option<Arc<SubDocs>>;
                if !to_delete_sub_docs.is_empty() && random().next_boolean() {
                    let d = Arc::clone(
                        &to_delete_sub_docs
                            [random().next_int_bound(to_delete_sub_docs.len() as i32) as usize],
                    );
                    debug_assert!(!d.deleted.load(Ordering::SeqCst));
                    to_delete_sub_docs.retain(|s| !Arc::ptr_eq(s, &d));
                    // Update doc block, replacing prior packID
                    pack_id = d.pack_id.clone();
                    del_sub_docs = Some(d);
                } else {
                    del_sub_docs = None;
                    // Add doc block, using new packID
                    pack_id = state.pack_count.fetch_add(1, Ordering::SeqCst).to_string();
                }

                let pack_id_field =
                    LuceneTestCase::new_string_field("packID", &pack_id, Store::YES);
                let mut doc_ids: Vec<String> = Vec::new();
                let mut docs_list: Vec<Document> = Vec::new();

                doc.add(Box::new(pack_id_field));
                docs_list.push(TestUtil::clone_document(&doc)?);
                doc_ids.push(doc_get(&doc, "docid").unwrap_or_default());

                let max_doc_count = TestUtil::next_int(&*random(), 1, 10);
                while (docs_list.len() as i32) < max_doc_count {
                    // Java's `docs.nextDoc()` returns the thread's one
                    // `DocState.doc` (`LineFileDocs.java:295-345`), which
                    // still holds the fields this iteration added to `doc`:
                    // the `extra` field and `packIDField`. The port's
                    // `next_doc` builds a fresh document, so those fields
                    // are carried over, in the order Java's document holds
                    // them.
                    let mut next = docs.next_doc()?;
                    for f in TestUtil::clone_document(&doc)?.into_fields() {
                        if f.name() == "packID" || added_field.as_deref() == Some(f.name()) {
                            next.add(f);
                        }
                    }
                    docs_list.push(TestUtil::clone_document(&next)?);
                    doc_ids.push(doc_get(&next, "docid").unwrap_or_default());
                    doc = next;
                }
                let sub_docs = Arc::new(SubDocs {
                    pack_id: pack_id.clone(),
                    sub_ids: doc_ids.clone(),
                    deleted: AtomicBool::new(false),
                });
                all_sub_docs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(Arc::clone(&sub_docs));
                state
                    .add_count
                    .fetch_add(docs_list.len() as i32, Ordering::SeqCst);

                let pack_id_term = term("packID", &pack_id);

                if let Some(del_sub_docs) = &del_sub_docs {
                    del_sub_docs.deleted.store(true, Ordering::SeqCst);
                    del_ids
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(del_sub_docs.sub_ids.iter().cloned());
                    state
                        .del_count
                        .fetch_add(del_sub_docs.sub_ids.len() as i32, Ordering::SeqCst);
                    if *VERBOSE {
                        println!(
                            "{}: update pack packID={} count={} docs={doc_ids:?}",
                            thread_name(),
                            del_sub_docs.pack_id,
                            docs_list.len()
                        );
                    }
                    case.update_documents(pack_id_term, &docs_list)?;
                } else {
                    if *VERBOSE {
                        println!(
                            "{}: add pack packID={pack_id} count={} docs={doc_ids:?}",
                            thread_name(),
                            docs_list.len()
                        );
                    }
                    case.add_documents(pack_id_term, &docs_list)?;
                }
                doc.remove_field("packID");

                if random().next_int_bound(5) == 2 {
                    if *VERBOSE {
                        println!("{}: buffer del id:{pack_id}", thread_name());
                    }
                    to_delete_sub_docs.push(sub_docs);
                }
            } else {
                // Add single doc
                let docid = doc_get(&doc, "docid").unwrap_or_default();
                if *VERBOSE {
                    println!("{}: add doc docid:{docid}", thread_name());
                }
                case.add_document(term("docid", &docid), &doc)?;
                state.add_count.fetch_add(1, Ordering::SeqCst);

                if random().next_int_bound(5) == 3 {
                    if *VERBOSE {
                        println!("{}: buffer del id:{docid}", thread_name());
                    }
                    to_delete_ids.push(docid);
                }
            }
        } else {
            // Update single doc, but we never re-use
            // and ID so the delete will never
            // actually happen:
            let docid = doc_get(&doc, "docid").unwrap_or_default();
            if *VERBOSE {
                println!("{}: update doc id:{docid}", thread_name());
            }
            case.update_document(term("docid", &docid), &doc)?;
            state.add_count.fetch_add(1, Ordering::SeqCst);

            if random().next_int_bound(5) == 3 {
                if *VERBOSE {
                    println!("{}: buffer del id:{docid}", thread_name());
                }
                to_delete_ids.push(docid);
            }
        }

        if random().next_int_bound(30) == 17 {
            if *VERBOSE {
                println!("{}: apply {} deletes", thread_name(), to_delete_ids.len());
            }
            for id in &to_delete_ids {
                if *VERBOSE {
                    println!("{}: del term=id:{id}", thread_name());
                }
                case.delete_documents(term("docid", id))?;
            }
            let count = state
                .del_count
                .fetch_add(to_delete_ids.len() as i32, Ordering::SeqCst)
                + to_delete_ids.len() as i32;
            if *VERBOSE {
                println!("{}: tot {count} deletes", thread_name());
            }
            del_ids
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(to_delete_ids.drain(..));

            for sub_docs in to_delete_sub_docs.drain(..) {
                debug_assert!(!sub_docs.deleted.load(Ordering::SeqCst));
                del_pack_ids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(sub_docs.pack_id.clone());
                case.delete_documents(term("packID", &sub_docs.pack_id))?;
                sub_docs.deleted.store(true, Ordering::SeqCst);
                if *VERBOSE {
                    println!(
                        "{}: del subs: {:?} packID={}",
                        thread_name(),
                        sub_docs.sub_ids,
                        sub_docs.pack_id
                    );
                }
                del_ids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(sub_docs.sub_ids.iter().cloned());
                state
                    .del_count
                    .fetch_add(sub_docs.sub_ids.len() as i32, Ordering::SeqCst);
            }
        }
        if let Some(added_field) = &added_field {
            doc.remove_field(added_field);
        }
    }
    Ok(())
}

/// `launchIndexingThreads(docs, numThreads, maxIterations, delIDs,
/// delPackIDs, allSubDocs)` (`private`).
fn launch_indexing_threads(
    case: &Arc<dyn ThreadedIndexingAndSearchingTestCase>,
    docs: &Arc<LineFileDocs>,
    num_threads: i32,
    max_iterations: i32,
    del_ids: &Arc<Mutex<HashSet<String>>>,
    del_pack_ids: &Arc<Mutex<HashSet<String>>>,
    all_sub_docs: &Arc<Mutex<Vec<Arc<SubDocs>>>>,
) -> Vec<Thread> {
    let mut threads = Vec::with_capacity(num_threads as usize);
    for _ in 0..num_threads {
        let case = Arc::clone(case);
        let docs = Arc::clone(docs);
        let del_ids = Arc::clone(del_ids);
        let del_pack_ids = Arc::clone(del_pack_ids);
        let all_sub_docs = Arc::clone(all_sub_docs);
        let thread = Thread::unnamed(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                indexing_thread_body(
                    &case,
                    &docs,
                    max_iterations,
                    &del_ids,
                    &del_pack_ids,
                    &all_sub_docs,
                )
            }));
            let failure = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(throwable_of(&e)),
                Err(payload) => Some(Throwable::from_panic(payload)),
            };
            if let Some(t) = failure {
                println!("{}: hit exc", thread_name());
                eprintln!("{t:?}");
                case.state().failed.store(true, Ordering::SeqCst);
                Throwable::from_cause("java.lang.RuntimeException", t).throw();
            }
            if *VERBOSE {
                println!("{}: indexing done", thread_name());
            }

            case.do_after_indexing_thread_done();
        });
        thread.start();
        threads.push(thread);
    }

    threads
}

/// `runSearchThreads(maxIterations)` (`protected`).
///
/// # Errors
///
/// The errors of joining the threads.
pub fn run_search_threads(
    case: &Arc<dyn ThreadedIndexingAndSearchingTestCase>,
    max_iterations: i32,
) -> Result<()> {
    let num_threads = if *TEST_NIGHTLY {
        TestUtil::next_int(&*random(), 1, 5)
    } else {
        2
    };
    let tot_hits = Arc::new(AtomicI64::new(0));

    // silly starting guess:
    let tot_term_count = Arc::new(AtomicI32::new(100));

    // TODO: we should enrich this to do more interesting searches
    let mut search_threads = Vec::with_capacity(num_threads as usize);
    for _ in 0..num_threads {
        let case = Arc::clone(case);
        let tot_hits = Arc::clone(&tot_hits);
        let tot_term_count = Arc::clone(&tot_term_count);
        let thread = Thread::unnamed(move || {
            if *VERBOSE {
                println!("{}: launch search thread", thread_name());
            }
            let state = Arc::clone(case.state());
            let mut iterations = 0;
            loop {
                iterations += 1;
                if !(iterations < max_iterations && !state.failed.load(Ordering::SeqCst)) {
                    break;
                }
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    search_once(&case, max_iterations, &tot_hits, &tot_term_count)
                }));
                let failure = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(throwable_of(&e)),
                    Err(payload) => Some(Throwable::from_panic(payload)),
                };
                if let Some(t) = failure {
                    println!("{}: hit exc", thread_name());
                    state.failed.store(true, Ordering::SeqCst);
                    println!("{t:?}");
                    Throwable::from_cause("java.lang.RuntimeException", t).throw();
                }
            }
        });
        thread.start();
        search_threads.push(thread);
    }

    for thread in &search_threads {
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
    }

    if *VERBOSE {
        println!(
            "TEST: DONE search: totHits={}",
            tot_hits.load(Ordering::SeqCst)
        );
    }
    Ok(())
}

/// One iteration of a search thread of `runSearchThreads`.
fn search_once(
    case: &Arc<dyn ThreadedIndexingAndSearchingTestCase>,
    max_iterations: i32,
    tot_hits: &AtomicI64,
    tot_term_count: &AtomicI32,
) -> Result<()> {
    let state = case.state();
    let s = case.get_current_searcher()?;
    let outcome = (|| -> Result<()> {
        let searcher = s.searcher();
        // Verify 1) IW is correctly setting
        // diagnostics, and 2) segment warming for
        // merged segments is actually happening:
        for sub in Arc::clone(searcher.get_index_reader()).leaves()? {
            let reader = sub.leaf_reader();
            let seg_reader = reader.as_segment_reader().unwrap_or_else(|| {
                Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
            });
            let diagnostics = seg_reader.get_segment_info().info.get_diagnostics();
            let source = diagnostics.get("source");
            assert_not_null(source.is_some());
            if source.map(String::as_str) == Some("merge") {
                let core = TestSecrets::get_segment_reader_access()?.get_core(seg_reader);
                Assert::assert_true_msg(
                    &format!(
                        "sub reader {sub} wasn't warmed: warmed={} diagnostics={diagnostics} si={}",
                        state.warmed_string(),
                        seg_reader.get_segment_info()
                    ),
                    !state.assert_merged_segments_warmed.load(Ordering::SeqCst)
                        || state.is_warmed(&core),
                );
            }
        }
        if searcher.get_index_reader().num_docs() > 0 {
            smoke_test_searcher(searcher)?;
            let Some(terms) = MultiTerms::get_terms(searcher.get_index_reader(), "body")? else {
                return Ok(());
            };
            let mut terms_enum = terms.iterator()?;
            let mut seen_term_count = 0;
            let shift;
            let trigger;
            if tot_term_count.load(Ordering::SeqCst) < 30 {
                shift = 0;
                trigger = 1;
            } else {
                trigger = tot_term_count.load(Ordering::SeqCst) / 30;
                shift = random().next_int_bound(trigger);
            }
            let mut iters = 0;
            loop {
                iters += 1;
                if iters >= max_iterations {
                    break;
                }
                let Some(term) = terms_enum.next()? else {
                    tot_term_count.store(seen_term_count, Ordering::SeqCst);
                    break;
                };
                seen_term_count += 1;
                // search 30 terms
                if (seen_term_count + shift) % trigger == 0 {
                    tot_hits.fetch_add(
                        run_query(searcher, Arc::new(TermQuery::new(Term::new("body", term))))?,
                        Ordering::SeqCst,
                    );
                }
            }
        }
        Ok(())
    })();
    let released = case.release_searcher(s);
    outcome?;
    released
}

/// `runQuery(s, q)` (`private`).
fn run_query(s: &IndexSearcher, q: Arc<dyn Query>) -> Result<i64> {
    s.search(Arc::clone(&q), 10)?;
    let title_dv = || -> Result<Sort> {
        Sort::new_fields(vec![SortField::new(
            Some("titleDV".to_string()),
            SortFieldType::String,
        )?])
    };
    let hit_count = s
        .search_with_sort(Arc::clone(&q), 10, &title_dv()?)?
        .total_hits
        .value();
    let dv_sort = title_dv()?;
    let hit_count2 = s.search_with_sort(q, 10, &dv_sort)?.total_hits.value();
    Assert::assert_equals(hit_count, hit_count2);
    Ok(hit_count)
}

/// `smokeTestSearcher(s)` (`protected`).
///
/// # Errors
///
/// The searcher's errors.
pub fn smoke_test_searcher(s: &IndexSearcher) -> Result<()> {
    run_query(s, term_query("body", "united"))?;
    run_query(s, term_query("titleTokenized", "states"))?;
    let pq = PhraseQuery::with_slop(0, "body", &["united", "states"])?;
    run_query(s, Arc::new(pq))?;
    Ok(())
}

/// The merged-segment warmer `runTest` installs.
#[derive(Debug)]
struct WarmedRecorder {
    state: Weak<ThreadedState>,
}

impl std::fmt::Display for WarmedRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadedIndexingAndSearchingTestCase$$Lambda")
    }
}

impl MergedSegmentWarmer for WarmedRecorder {
    fn warm(&self, reader: &Arc<dyn LeafReader>) -> Result<()> {
        if *VERBOSE {
            println!("TEST: now warm merged reader={reader}");
        }
        let seg_reader = reader.as_segment_reader().unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
        });
        let core = TestSecrets::get_segment_reader_access()?.get_core(seg_reader);
        if let Some(state) = self.state.upgrade() {
            state.put_warmed(&core);
        }
        let max_doc = reader.max_doc();
        let live_docs = reader.get_live_docs();
        let mut sum = 0i64;
        let inc = 1.max(max_doc / 50);
        let stored_fields = reader.stored_fields()?;
        let mut doc_id = 0;
        while doc_id < max_doc {
            if live_docs.as_ref().is_none_or(|l| l.get(doc_id as usize)) {
                let doc = stored_fields.document(doc_id)?;
                sum += doc.get_fields().len() as i64;
            }
            doc_id += inc;
        }

        let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(reader), false);
        sum += searcher
            .search(term_query("body", "united"), 10)?
            .total_hits
            .value();

        if *VERBOSE {
            println!("TEST: warm visited {sum} fields");
        }
        Ok(())
    }
}

/// The `PrintStreamInfoStream(System.out)` of `VERBOSE` runs, which ignores
/// test points.
#[derive(Debug)]
struct NoTestPointsInfoStream(PrintStreamInfoStream);

impl InfoStream for NoTestPointsInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.index.ThreadedIndexingAndSearchingTestCase$1"
    }

    fn message(&self, component: &str, message: &str) {
        if component == "TP" {
            return; // ignore test points!
        }
        self.0.message(component, message);
    }

    fn is_enabled(&self, component: &str) -> bool {
        self.0.is_enabled(component)
    }

    fn close(&self) {
        self.0.close();
    }
}

/// `runTest(testName)` (`public`).
///
/// # Errors
///
/// The errors of indexing, searching and checking the index.
pub fn run_test(
    case: Arc<dyn ThreadedIndexingAndSearchingTestCase>,
    test_name: &str,
) -> Result<()> {
    let state = Arc::clone(case.state());
    *state.this.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(&case));

    state.failed.store(false, Ordering::SeqCst);
    state.add_count.store(0, Ordering::SeqCst);
    state.del_count.store(0, Ordering::SeqCst);
    state.pack_count.store(0, Ordering::SeqCst);

    let t0 = Instant::now();

    let random_for_docs = JdkRandom::new(random().next_long());
    let docs =
        Arc::new(LineFileDocs::with_default_file(&random_for_docs).map_err(LuceneError::from)?);
    let temp_dir = LuceneTestCase::create_temp_dir_prefix(test_name);
    // some subclasses rely on this being MDW
    let dir = case.get_directory(LuceneTestCase::new_mock_fs_directory(&temp_dir));
    if let ThreadedDirectory::Mock(mdw) = &dir {
        mdw.set_check_index_on_close(false); // don't double-checkIndex, we do it ourselves.
    }
    *state.dir_handle.write().unwrap_or_else(|e| e.into_inner()) = Some(dir.as_directory());
    *state.dir.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    let mut conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    conf.set_commit_on_close(false);
    conf.set_info_stream(Arc::new(FailOnNonBulkMergesInfoStream))?;
    if let Some(mp) = conf
        .merge_policy()
        .as_any()
        .and_then(|a| a.downcast_ref::<MockRandomMergePolicy>())
    {
        mp.set_do_non_bulk_merges(false);
    }

    LuceneTestCase::ensure_sane_iwc_on_nightly(&mut conf);

    conf.set_merged_segment_warmer(Some(Arc::new(WarmedRecorder {
        state: Arc::downgrade(&state),
    })))?;

    if *VERBOSE {
        conf.set_info_stream(Arc::new(NoTestPointsInfoStream(
            PrintStreamInfoStream::new(Box::new(std::io::stdout())),
        )))?;
    }
    let writer = IndexWriter::new(state.dir(), conf)?;
    *state.writer.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&writer));
    TestUtil::reduce_open_files(&writer)?;

    let es: Option<Arc<CachedThreadPoolExecutor>> = if random().next_boolean() {
        None
    } else {
        Some(Arc::new(CachedThreadPoolExecutor::new(test_name)))
    };

    case.do_after_writer(es.clone())?;

    let num_index_threads = TestUtil::next_int(&*random(), 2, 4);

    let max_iterations = if *TEST_NIGHTLY {
        200
    } else {
        10 * *RANDOM_MULTIPLIER
    };

    let del_ids = Arc::new(Mutex::new(HashSet::<String>::new()));
    let del_pack_ids = Arc::new(Mutex::new(HashSet::<String>::new()));
    let all_sub_docs = Arc::new(Mutex::new(Vec::<Arc<SubDocs>>::new()));

    let index_threads = launch_indexing_threads(
        &case,
        &docs,
        num_index_threads,
        max_iterations,
        &del_ids,
        &del_pack_ids,
        &all_sub_docs,
    );

    if *VERBOSE {
        println!(
            "TEST: DONE start {num_index_threads} indexing threads [{} ms]",
            t0.elapsed().as_millis()
        );
    }

    // Let index build up a bit
    sleep_millis(100);

    case.do_searching(es.clone(), max_iterations)?;

    if *VERBOSE {
        println!("TEST: all searching done [{} ms]", t0.elapsed().as_millis());
    }

    for thread in &index_threads {
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
    }

    Assert::assert_true(
        writer.get_segment_infos_counter()?
            >= state.max_advanced_segment_counter.load(Ordering::SeqCst),
    );

    if *VERBOSE {
        println!(
            "TEST: done join indexing threads [{} ms]; addCount={} delCount={} segmentCounter={}",
            t0.elapsed().as_millis(),
            state.add_count.load(Ordering::SeqCst),
            state.del_count.load(Ordering::SeqCst),
            writer.get_segment_infos_counter()?
        );
    }

    let s = case.get_final_searcher()?;
    if *VERBOSE {
        println!("TEST: finalSearcher={:?}", s.searcher());
    }

    Assert::assert_false(state.failed.load(Ordering::SeqCst));

    let mut do_fail = false;
    let searcher = s.searcher();

    // Verify: make sure delIDs are in fact deleted:
    let del_ids_now: Vec<String> = del_ids
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    for id in &del_ids_now {
        let hits = searcher.search(term_query("docid", id), 1)?;
        if hits.total_hits.value() != 0 {
            println!(
                "doc id={id} is supposed to be deleted, but got {} hits; first docID={}",
                hits.total_hits.value(),
                hits.score_docs[0].doc
            );
            do_fail = true;
        }
    }

    // Verify: make sure delPackIDs are in fact deleted:
    let del_pack_ids_now: Vec<String> = del_pack_ids
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    for id in &del_pack_ids_now {
        let hits = searcher.search(term_query("packID", id), 1)?;
        if hits.total_hits.value() != 0 {
            println!(
                "packID={id} is supposed to be deleted, but got {} matches",
                hits.total_hits.value()
            );
            do_fail = true;
        }
    }

    // Verify: make sure each group of sub-docs are still in docID order:
    let all_sub_docs_now: Vec<Arc<SubDocs>> = all_sub_docs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for sub_docs in &all_sub_docs_now {
        let hits = searcher.search(term_query("packID", &sub_docs.pack_id), 20)?;
        let stored_fields = searcher.stored_fields()?;
        if !sub_docs.deleted.load(Ordering::SeqCst) {
            // We sort by relevance but the scores should be identical so sort falls back to by
            // docID:
            if hits.total_hits.value() != sub_docs.sub_ids.len() as i64 {
                println!(
                    "packID={}: expected {} hits but got {}",
                    sub_docs.pack_id,
                    sub_docs.sub_ids.len(),
                    hits.total_hits.value()
                );
                do_fail = true;
            } else {
                let mut last_doc_id = -1;
                let mut start_doc_id = -1;
                for score_doc in &hits.score_docs {
                    let doc_id = score_doc.doc;
                    if last_doc_id != -1 {
                        Assert::assert_equals(i64::from(1 + last_doc_id), i64::from(doc_id));
                    } else {
                        start_doc_id = doc_id;
                    }
                    last_doc_id = doc_id;
                    let doc = stored_fields.document(doc_id)?;
                    Assert::assert_equals_str(
                        None,
                        Some(&sub_docs.pack_id),
                        doc_get(&doc, "packID").as_deref(),
                    );
                }

                last_doc_id = start_doc_id - 1;
                for sub_id in &sub_docs.sub_ids {
                    let hits = searcher.search(term_query("docid", sub_id), 1)?;
                    Assert::assert_equals(1i64, hits.total_hits.value());
                    let doc_id = hits.score_docs[0].doc;
                    if last_doc_id != -1 {
                        Assert::assert_equals(i64::from(1 + last_doc_id), i64::from(doc_id));
                    }
                    last_doc_id = doc_id;
                }
            }
        } else {
            // Pack was deleted -- make sure its docs are
            // deleted.  We can't verify packID is deleted
            // because we can re-use packID for update:
            for sub_id in &sub_docs.sub_ids {
                Assert::assert_equals(
                    0i64,
                    searcher
                        .search(term_query("docid", sub_id), 1)?
                        .total_hits
                        .value(),
                );
            }
        }
    }

    // Verify: make sure all not-deleted docs are in fact
    // not deleted:
    let end_id: i32 = doc_get(&docs.next_doc()?, "docid")
        .unwrap_or_default()
        .parse()
        .unwrap_or_else(|_| Throwable::new("java.lang.NumberFormatException", "docid").throw());
    docs.close();

    let del_ids_set: HashSet<String> = del_ids_now.iter().cloned().collect();
    for id in 0..end_id {
        let string_id = id.to_string();
        if !del_ids_set.contains(&string_id) {
            let hits = searcher.search(term_query("docid", &string_id), 1)?;
            if hits.total_hits.value() != 1 {
                println!(
                    "doc id={string_id} is not supposed to be deleted, but got hitCount={}; delIDs={del_ids_now:?}",
                    hits.total_hits.value()
                );
                do_fail = true;
            }
        }
    }
    Assert::assert_false(do_fail);

    let index_writer_access = TestSecrets::get_index_writer_access()?;
    Assert::assert_equals_msg(
        Some(&format!(
            "index={} addCount={} delCount={}",
            index_writer_access.seg_string(&writer)?,
            state.add_count.load(Ordering::SeqCst),
            state.del_count.load(Ordering::SeqCst)
        )),
        i64::from(state.add_count.load(Ordering::SeqCst) - state.del_count.load(Ordering::SeqCst)),
        i64::from(searcher.get_index_reader().num_docs()),
    );
    case.release_searcher(s)?;

    writer.commit()?;

    Assert::assert_equals_msg(
        Some(&format!(
            "index={} addCount={} delCount={}",
            index_writer_access.seg_string(&writer)?,
            state.add_count.load(Ordering::SeqCst),
            state.del_count.load(Ordering::SeqCst)
        )),
        i64::from(state.add_count.load(Ordering::SeqCst) - state.del_count.load(Ordering::SeqCst)),
        i64::from(writer.get_doc_stats()?.num_docs),
    );

    case.do_close()?;

    let committed = writer.commit();
    let closed = writer.close();
    committed?;
    closed?;

    // Cannot close until after writer is closed because
    // writer has merged segment warmer that uses IS to run
    // searches, and that IS may be using this es!
    if let Some(es) = &es {
        es.shutdown();
        es.await_termination(Duration::from_secs(1));
    }

    TestUtil::check_index(state.dir())?;
    *state.writer.write().unwrap_or_else(|e| e.into_inner()) = None;
    drop(writer);
    *state.dir_handle.write().unwrap_or_else(|e| e.into_inner()) = None;
    let dir = state
        .dir
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .expect("runTest opened the directory");
    dir.close()?;

    if *VERBOSE {
        println!("TEST: done [{} ms]", t0.elapsed().as_millis());
    }
    Ok(())
}

/// The subclass, as `runSearchThreads` needs it from inside
/// `doSearching(&self, …)`.
pub fn this(state: &ThreadedState) -> Arc<dyn ThreadedIndexingAndSearchingTestCase> {
    state.this()
}

/// `IndexReader` handle of a searcher, for subclasses.
pub fn reader_of(s: &TestSearcher) -> &Arc<dyn IndexReader> {
    s.searcher().get_index_reader()
}
