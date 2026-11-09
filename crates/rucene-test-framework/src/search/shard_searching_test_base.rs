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

//! Port of `org.apache.lucene.tests.search.ShardSearchingTestBase`: base
//! test class for simulating distributed search across multiple shards.
//!
//! The class is declared by [`class()`] (it declares no test of its own);
//! its fields and methods are [`ShardSearchingTestBase`], shared as an
//! `Arc` by the subclass, the [`NodeState`]s and the `ChangeIndices`
//! thread. The inner classes are [`NodeState`],
//! [`ShardIndexSearcher`] (`NodeState.ShardIndexSearcher`) and the
//! `SearcherExpiredException` class [`SEARCHER_EXPIRED_EXCEPTION`].
//!
//! **Adaptations** (declared, each confined to the member named):
//! * `ShardIndexSearcher extends IndexSearcher`: the searcher is an
//!   [`IndexSearcher`] with [`ShardIndexSearcherOverrides`] installed (the
//!   port's subclassing seam, see [`IndexSearcherOverrides`]); its
//!   `rewrite`, `termStatistics`, `collectionStatistics`, `search(Query,
//!   int)`, `searchAfter(ScoreDoc, Query, int)` and `search(Query, int,
//!   Sort)` overrides are that trait's methods, and `localSearch…` call the
//!   inherited bodies (`super_…`).
//! * `searchNode` returns `TopDocs` that a sorted caller casts to
//!   `TopFieldDocs`; the port returns [`NodeHits`], which holds either.
//! * `SearcherExpiredException` (a `RuntimeException`) travels as
//!   [`LuceneError::Unchecked`] of class [`SEARCHER_EXPIRED_EXCEPTION`]
//!   ([`searcher_expired`], [`is_searcher_expired`]).
//! * `ConcurrentHashMap` caches are `Mutex<HashMap>`; the `volatile
//!   currentShardSearcher` is an `RwLock<Option<Arc<…>>>`.
//! * `QueryVisitor.termCollector(Set<Term>)` fills the port's
//!   `BTreeSet<Term>`; the iteration order of the set draws no randomness
//!   and only decides the order of cache insertions.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, RwLock, Weak};
use std::time::Instant;

use randomizedtesting::jdk::exception::{register_class, Throwable};
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::jdk::thread::Thread;
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::terms::{Term, TermStates};
use rucene::index::IndexReader;
use rucene::search::query_visitor::term_collector;
use rucene::search::searcher_manager::{ManagedSearcher, SearcherManager};
use rucene::search::similarities::{CollectionStatistics, TermStatistics};
use rucene::search::{
    IndexSearcher, IndexSearcherOverrides, PruneByAge, Query, ScoreDoc, SearcherLifetimeManager,
    Sort, TopDocs, TopFieldDocs,
};
use rucene::util::PrintStreamInfoStream;

use crate::analysis::MockAnalyzer;
use crate::util::lucene_test_case::{TestDirectory, VERBOSE};
use crate::util::{LineFileDocs, LuceneTestCase, TestUtil};

/// The binary name of `ShardSearchingTestBase.SearcherExpiredException`
/// (`public static class SearcherExpiredException extends
/// RuntimeException`): thrown when the lease for a searcher has expired.
pub const SEARCHER_EXPIRED_EXCEPTION: &str =
    "org.apache.lucene.tests.search.ShardSearchingTestBase$SearcherExpiredException";

/// The declaration of `org.apache.lucene.tests.search.ShardSearchingTestBase`
/// (`public abstract class ShardSearchingTestBase extends LuceneTestCase`),
/// which declares no test.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        register_class(SEARCHER_EXPIRED_EXCEPTION, "java.lang.RuntimeException");
        Class::builder("org.apache.lucene.tests.search.ShardSearchingTestBase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// `new SearcherExpiredException(message)`.
pub fn searcher_expired(message: String) -> LuceneError {
    register_class(SEARCHER_EXPIRED_EXCEPTION, "java.lang.RuntimeException");
    LuceneError::Unchecked {
        class: SEARCHER_EXPIRED_EXCEPTION,
        message: Some(message),
        cause: None,
    }
}

/// Whether `catch (SearcherExpiredException see)` catches `e`.
pub fn is_searcher_expired(e: &LuceneError) -> bool {
    matches!(e, LuceneError::Unchecked { class, .. } if *class == SEARCHER_EXPIRED_EXCEPTION)
}

/// `System.nanoTime()`: nanoseconds from a fixed, arbitrary origin.
pub fn nano_time() -> i64 {
    static ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);
    ORIGIN.elapsed().as_nanos() as i64
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `FieldAndShardVersion` (`private record`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FieldAndShardVersion {
    node_id: i32,
    version: i64,
    field: String,
}

impl fmt::Display for FieldAndShardVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FieldAndShardVersion(field={} nodeID={} version={})",
            self.field, self.node_id, self.version
        )
    }
}

/// `TermAndShardVersion` (`private record`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TermAndShardVersion {
    node_id: i32,
    version: i64,
    term: Term,
}

/// The `collectionStatsCache` of a [`NodeState`].
type CollectionStatsCache = Arc<Mutex<HashMap<FieldAndShardVersion, CollectionStatistics>>>;
/// The `termStatsCache` of a [`NodeState`].
type TermStatsCache = Arc<Mutex<HashMap<TermAndShardVersion, TermStatistics>>>;

/// What `searchNode` returns: Java's `TopDocs`, which is a `TopFieldDocs`
/// for a sorted search.
#[derive(Debug)]
pub enum NodeHits {
    /// The hits of an unsorted search.
    Docs(TopDocs),
    /// The hits of a sorted search.
    Fields(TopFieldDocs),
}

impl NodeHits {
    /// The hits as `TopDocs`.
    pub fn into_top_docs(self) -> TopDocs {
        match self {
            NodeHits::Docs(d) => d,
            NodeHits::Fields(f) => f.to_top_docs(),
        }
    }

    /// `(TopFieldDocs) hits`: `ClassCastException` for unsorted hits.
    pub fn into_top_field_docs(self) -> TopFieldDocs {
        match self {
            NodeHits::Fields(f) => f,
            NodeHits::Docs(_) => randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.ClassCastException",
            ),
        }
    }
}

/// The fields and methods of `ShardSearchingTestBase`.
pub struct ShardSearchingTestBase {
    /// The weak self reference the nodes and the searchers hold.
    this: Weak<ShardSearchingTestBase>,
    /// `fieldsToShare`: we share collection stats for these fields on each
    /// node reopen.
    fields_to_share: [&'static str; 2],
    /// `protected NodeState[] nodes`.
    nodes: OnceLock<Vec<Arc<NodeState>>>,
    /// `int maxSearcherAgeSeconds`.
    max_searcher_age_seconds: AtomicI32,
    /// `public long endTimeNanos`.
    end_time_nanos: AtomicI64,
    /// `private Thread changeIndicesThread`.
    change_indices_thread: Mutex<Option<Thread>>,
}

impl fmt::Debug for ShardSearchingTestBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShardSearchingTestBase")
            .field("nodes", &self.nodes.get().map(Vec::len))
            .finish_non_exhaustive()
    }
}

impl ShardSearchingTestBase {
    /// The state of a new instance.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|this| ShardSearchingTestBase {
            this: this.clone(),
            fields_to_share: ["body", "title"],
            nodes: OnceLock::new(),
            max_searcher_age_seconds: AtomicI32::new(0),
            end_time_nanos: AtomicI64::new(0),
            change_indices_thread: Mutex::new(None),
        })
    }

    /// `nodes`; `NullPointerException` before [`start`](Self::start).
    pub fn nodes(&self) -> &[Arc<NodeState>] {
        match self.nodes.get() {
            Some(nodes) => nodes,
            None => randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            ),
        }
    }

    /// `endTimeNanos`.
    pub fn end_time_nanos(&self) -> i64 {
        self.end_time_nanos.load(Ordering::SeqCst)
    }

    /// `broadcastNodeReopen(int nodeID, long version, IndexSearcher
    /// newSearcher)`: called by one node once it has reopened, to notify all
    /// other nodes. This is just a mock (since it goes and directly updates
    /// all other nodes, in RAM)... in a real env this would hit the wire,
    /// sending version & collection stats to all other nodes.
    pub fn broadcast_node_reopen(
        &self,
        node_id: i32,
        version: i64,
        new_searcher: &IndexSearcher,
    ) -> Result<()> {
        if *VERBOSE {
            println!(
                "REOPEN: nodeID={node_id} version={version} maxDoc={}",
                new_searcher.get_index_reader().max_doc()
            );
        }

        // Broadcast new collection stats for this node to all
        // other nodes:
        for field in self.fields_to_share {
            let stats = new_searcher.collection_statistics(field)?;
            if let Some(stats) = stats {
                for node in self.nodes() {
                    // Don't put my own collection stats into the cache;
                    // we pull locally:
                    if node.my_node_id != node_id {
                        lock(&node.collection_stats_cache).insert(
                            FieldAndShardVersion {
                                node_id,
                                version,
                                field: field.to_string(),
                            },
                            stats.clone(),
                        );
                    }
                }
            }
        }
        for node in self.nodes() {
            node.update_node_version(node_id, version)?;
        }
        Ok(())
    }

    // TODO: broadcastNodeExpire?  then we can purge the
    // known-stale cache entries...

    /// `searchNode(int nodeID, long[] nodeVersions, Query q, Sort sort, int
    /// numHits, ScoreDoc searchAfter)`: MOCK: in a real env you have to hit
    /// the wire (send this query to all remote nodes concurrently).
    ///
    /// # Errors
    ///
    /// [`SEARCHER_EXPIRED_EXCEPTION`] when the node's searcher of that
    /// version has expired, and the errors of the search.
    pub fn search_node(
        &self,
        node_id: i32,
        node_versions: &[i64],
        q: Arc<dyn Query>,
        sort: Option<&Sort>,
        num_hits: i32,
        search_after: Option<ScoreDoc>,
    ) -> Result<NodeHits> {
        let node = &self.nodes()[node_id as usize];
        let s = node.acquire_versions(node_versions)?;
        let result = (|| -> Result<NodeHits> {
            match sort {
                None => {
                    if search_after.is_some() {
                        Ok(NodeHits::Docs(s.local_search_after(
                            search_after,
                            q,
                            num_hits,
                        )?))
                    } else {
                        Ok(NodeHits::Docs(s.local_search(q, num_hits)?))
                    }
                }
                Some(sort) => {
                    assert!(search_after.is_none()); // not supported yet
                    Ok(NodeHits::Fields(
                        s.local_search_with_sort(q, num_hits, sort)?,
                    ))
                }
            }
        })();
        // finally
        let released = node.release(&s);
        let hits = result?;
        released?;
        Ok(hits)
    }

    /// `getNodeTermStats(Set<Term> terms, int nodeID, long version)`: Mock:
    /// in a real env, this would hit the wire and get term stats from
    /// remote node.
    ///
    /// # Errors
    ///
    /// [`SEARCHER_EXPIRED_EXCEPTION`] when the node's searcher of that
    /// version has expired, and the errors of reading the statistics.
    pub fn get_node_term_stats(
        &self,
        terms: &BTreeSet<Term>,
        node_id: i32,
        version: i64,
    ) -> Result<HashMap<Term, TermStatistics>> {
        let node = &self.nodes()[node_id as usize];
        let mut stats = HashMap::new();
        let Some(s) = node.searchers().acquire(version)? else {
            return Err(searcher_expired(format!(
                "node={node_id} version={version}"
            )));
        };
        let result = (|| -> Result<()> {
            for term in terms {
                let ts = TermStates::build(s.searcher(), term, true)?;
                if ts.doc_freq()? > 0 {
                    stats.insert(
                        term.clone(),
                        s.searcher().term_statistics(
                            term,
                            ts.doc_freq()?,
                            ts.total_term_freq()?,
                        )?,
                    );
                }
            }
            Ok(())
        })();
        // finally
        let released = node.searchers().release(&s);
        result?;
        released?;
        Ok(stats)
    }

    /// `start(int numNodes, double runTimeSec, int maxSearcherAgeSeconds)`
    /// (`protected`).
    ///
    /// # Errors
    ///
    /// The errors of opening the nodes and recording their searchers.
    pub fn start(
        &self,
        num_nodes: i32,
        run_time_sec: f64,
        max_searcher_age_seconds: i32,
    ) -> Result<()> {
        self.end_time_nanos.store(
            nano_time() + (run_time_sec * 1000000000.0) as i64,
            Ordering::SeqCst,
        );
        self.max_searcher_age_seconds
            .store(max_searcher_age_seconds, Ordering::SeqCst);

        let mut nodes = Vec::with_capacity(num_nodes as usize);
        for node_id in 0..num_nodes {
            nodes.push(NodeState::new(
                &*random(),
                node_id,
                num_nodes,
                self.this.clone(),
            )?);
        }
        if self.nodes.set(nodes).is_err() {
            return Err(LuceneError::IllegalState {
                message: "start was already called".to_string(),
                cause: None,
            });
        }
        let nodes = self.nodes();

        let mut node_versions = vec![0i64; nodes.len()];
        for node_id in 0..num_nodes as usize {
            let s = nodes[node_id].mgr().acquire()?;
            let recorded = nodes[node_id].searchers().record(&s);
            let released = nodes[node_id].mgr().release(s);
            node_versions[node_id] = recorded?;
            released?;
        }

        for node_id in 0..num_nodes as usize {
            let s = nodes[node_id].mgr().acquire()?;
            assert_eq!(
                node_versions[node_id],
                nodes[node_id].searchers().record(&s)?
            );
            let result =
                self.broadcast_node_reopen(node_id as i32, node_versions[node_id], s.searcher());
            let released = nodes[node_id].mgr().release(s);
            result?;
            released?;
        }

        let base = self
            .this
            .upgrade()
            .expect("INVARIANT: the test instance holds the base state");
        let thread = Thread::unnamed(move || change_indices_run(&base));
        thread.start();
        *lock(&self.change_indices_thread) = Some(thread);
        Ok(())
    }

    /// `finish()` (`protected`).
    ///
    /// # Errors
    ///
    /// The errors of closing the nodes.
    pub fn finish(&self) -> Result<()> {
        let thread = lock(&self.change_indices_thread).take();
        if let Some(thread) = thread {
            if let Err(payload) = thread.join() {
                std::panic::resume_unwind(payload);
            }
        }
        for node in self.nodes() {
            node.close()?;
        }
        Ok(())
    }

    /// `maxSearcherAgeSeconds`.
    fn max_searcher_age_seconds(&self) -> i32 {
        self.max_searcher_age_seconds.load(Ordering::SeqCst)
    }
}

/// `lock()` that ignores poisoning, as Java's monitors have none.
/// Read-locks `l`, ignoring poisoning.
fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Write-locks `l`, ignoring poisoning.
fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The Java throwable of `e`.
fn throwable_of(e: &LuceneError) -> Throwable {
    crate::util::java_exceptions::java_throwable(e)
}

/// `ChangeIndices.run()` (`private final class ChangeIndices extends
/// Thread`).
// TODO: make this more realistic, ie, each node should
// have its own thread, so we have true node to node
// concurrency
fn change_indices_run(base: &Arc<ShardSearchingTestBase>) {
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        let docs = LineFileDocs::with_default_file(&*random()).map_err(LuceneError::from)?;
        let result = (|| -> Result<()> {
            let nodes = base.nodes();
            let mut num_docs = 0;
            while nano_time() < base.end_time_nanos() {
                let what = random().next_int_bound(3);
                let node = &nodes[random().next_int_bound(nodes.len() as i32) as usize];
                if num_docs == 0 || what == 0 {
                    node.writer().add_document(&docs.next_doc()?)?;
                    num_docs += 1;
                } else if what == 1 {
                    let term = Term::new(
                        "docid",
                        rucene::util::BytesRef::from_char_sequence(
                            &random().next_int_bound(num_docs).to_string(),
                        ),
                    );
                    node.writer().update_document(term, &docs.next_doc()?)?;
                    num_docs += 1;
                } else {
                    node.writer().delete_documents_by_terms(vec![Term::new(
                        "docid",
                        rucene::util::BytesRef::from_char_sequence(
                            &random().next_int_bound(num_docs).to_string(),
                        ),
                    )])?;
                }
                // TODO: doc blocks too

                if random().next_int_bound(17) == 12 {
                    node.writer().commit()?;
                }

                if random().next_int_bound(17) == 12 {
                    nodes[random().next_int_bound(nodes.len() as i32) as usize].reopen()?;
                }
            }
            Ok(())
        })();
        // try-with-resources
        docs.close();
        result
    }));
    let failure = match outcome {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(throwable_of(&e)),
        Err(payload) => Some(Throwable::from_panic(payload)),
    };
    if let Some(t) = failure {
        println!("FAILED:");
        println!("{t:?}");
        Throwable::from_cause("java.lang.RuntimeException", t).throw();
    }
}

/// `NodeState` (`protected final class NodeState implements Closeable`):
/// simulated shard node under test.
pub struct NodeState {
    /// `public final Directory dir`.
    pub dir: Mutex<TestDirectory>,
    /// `public final IndexWriter writer`, read through [`writer`](Self::writer).
    writer: RwLock<Option<Arc<IndexWriter>>>,
    /// `public final SearcherLifetimeManager searchers`, read through
    /// [`searchers`](Self::searchers).
    searchers: RwLock<Option<Arc<SearcherLifetimeManager>>>,
    /// `public final SearcherManager mgr`, read through [`mgr`](Self::mgr).
    mgr: RwLock<Option<Arc<SearcherManager>>>,
    /// `public final int myNodeID`.
    pub my_node_id: i32,
    /// `public final long[] currentNodeVersions`.
    pub current_node_versions: Mutex<Vec<i64>>,

    // TODO: nothing evicts from here!!!  Somehow, on searcher
    // expiration on remote nodes we must evict from our
    // local cache...?  And still LRU otherwise (for the
    // still-live searchers).
    collection_stats_cache: CollectionStatsCache,
    term_stats_cache: TermStatsCache,

    /// `private volatile ShardIndexSearcher currentShardSearcher`.
    current_shard_searcher: RwLock<Option<Arc<ShardIndexSearcher>>>,
    /// The enclosing `ShardSearchingTestBase.this`.
    base: Weak<ShardSearchingTestBase>,
}

impl fmt::Debug for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeState")
            .field("my_node_id", &self.my_node_id)
            .finish_non_exhaustive()
    }
}

impl NodeState {
    /// `new NodeState(Random random, int nodeID, int numNodes)`.
    ///
    /// # Errors
    ///
    /// The errors of opening the writer and the searcher manager.
    pub fn new(
        _random: &dyn Random,
        node_id: i32,
        num_nodes: i32,
        base: Weak<ShardSearchingTestBase>,
    ) -> Result<Arc<NodeState>> {
        let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
            "ShardSearchingTestBase",
        ));
        // TODO: set warmer
        let analyzer = MockAnalyzer::new(&*random());
        analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
        let mut iwc = IndexWriterConfig::with_analyzer(Arc::new(analyzer));
        iwc.set_open_mode(OpenMode::CREATE)?;
        if *VERBOSE {
            iwc.set_info_stream(Arc::new(PrintStreamInfoStream::new(Box::new(
                std::io::stdout(),
            ))))?;
        }
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        let mgr = SearcherManager::from_writer(&*writer, true, false, None, None)?;
        let searchers = SearcherLifetimeManager::new();

        // Init w/ 0s... caller above will do initial
        // "broadcast" by calling initSearcher:
        Ok(Arc::new(NodeState {
            dir: Mutex::new(dir),
            writer: RwLock::new(Some(writer)),
            searchers: RwLock::new(Some(Arc::new(searchers))),
            mgr: RwLock::new(Some(Arc::new(mgr))),
            my_node_id: node_id,
            current_node_versions: Mutex::new(vec![0; num_nodes as usize]),
            collection_stats_cache: Arc::new(Mutex::new(HashMap::new())),
            term_stats_cache: Arc::new(Mutex::new(HashMap::new())),
            current_shard_searcher: RwLock::new(None),
            base,
        }))
    }

    /// `writer`.
    ///
    /// # Panics
    ///
    /// After [`close`](Self::close), as Java's closed writer would throw.
    pub fn writer(&self) -> Arc<IndexWriter> {
        Arc::clone(read(&self.writer).as_ref().expect("NodeState is closed"))
    }

    /// `searchers`.
    ///
    /// # Panics
    ///
    /// After [`close`](Self::close).
    pub fn searchers(&self) -> Arc<SearcherLifetimeManager> {
        Arc::clone(read(&self.searchers).as_ref().expect("NodeState is closed"))
    }

    /// `mgr`.
    ///
    /// # Panics
    ///
    /// After [`close`](Self::close).
    pub fn mgr(&self) -> Arc<SearcherManager> {
        Arc::clone(read(&self.mgr).as_ref().expect("NodeState is closed"))
    }

    /// `new ShardIndexSearcher(nodeVersions, localReader, nodeID)` of this
    /// node.
    fn new_shard_searcher(
        &self,
        node_versions: Vec<i64>,
        local_reader: Arc<dyn IndexReader>,
        node_id: i32,
    ) -> Result<ShardIndexSearcher> {
        ShardIndexSearcher::new(
            node_versions,
            local_reader,
            node_id,
            self.my_node_id,
            Arc::clone(&self.collection_stats_cache),
            Arc::clone(&self.term_stats_cache),
            self.base.clone(),
        )
    }

    /// The reader of the searcher `mgr.acquire()` answers.
    fn acquire_mgr_reader(&self) -> Result<Arc<dyn IndexReader>> {
        let managed: Arc<ManagedSearcher> = self.mgr().acquire()?;
        Ok(Arc::clone(managed.searcher().get_index_reader()))
    }

    /// `initSearcher(long[] nodeVersions)`.
    ///
    /// # Errors
    ///
    /// The errors of acquiring the searcher.
    pub fn init_searcher(&self, node_versions: &[i64]) -> Result<()> {
        let mut current = self
            .current_shard_searcher
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(current.is_none());
        let mut versions = lock(&self.current_node_versions);
        let n = versions.len();
        versions.copy_from_slice(&node_versions[..n]);
        *current = Some(Arc::new(self.new_shard_searcher(
            versions.clone(),
            self.acquire_mgr_reader()?,
            self.my_node_id,
        )?));
        Ok(())
    }

    /// `updateNodeVersion(int nodeID, long version)`.
    ///
    /// # Errors
    ///
    /// The errors of releasing the old searcher and acquiring the new one.
    pub fn update_node_version(&self, node_id: i32, version: i64) -> Result<()> {
        let mut versions = lock(&self.current_node_versions);
        versions[node_id as usize] = version;
        let mut current = self
            .current_shard_searcher
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(old) = current.as_ref() {
            old.get_index_reader().dec_ref()?;
        }
        *current = Some(Arc::new(self.new_shard_searcher(
            versions.clone(),
            self.acquire_mgr_reader()?,
            self.my_node_id,
        )?));
        Ok(())
    }

    /// `acquire()`: get the current (fresh) searcher for this node.
    pub fn acquire(&self) -> Arc<ShardIndexSearcher> {
        loop {
            let s = self
                .current_shard_searcher
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
                .unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::throw_without_message(
                        "java.lang.NullPointerException",
                    )
                });
            // In theory the reader could get decRef'd to 0
            // before we have a chance to incRef, ie if a reopen
            // happens right after the above line, this thread
            // gets stalled, and the old IR is closed.  So we
            // must try/retry until incRef succeeds:
            if s.get_index_reader().try_inc_ref() {
                return s;
            }
        }
    }

    /// `release(ShardIndexSearcher s)`.
    ///
    /// # Errors
    ///
    /// The errors of closing the reader.
    pub fn release(&self, s: &ShardIndexSearcher) -> Result<()> {
        s.get_index_reader().dec_ref()
    }

    /// `acquire(long[] nodeVersions)`: get an old searcher matching the
    /// specified versions.
    ///
    /// # Errors
    ///
    /// [`SEARCHER_EXPIRED_EXCEPTION`] when the searcher of that version has
    /// expired.
    pub fn acquire_versions(&self, node_versions: &[i64]) -> Result<Arc<ShardIndexSearcher>> {
        let version = node_versions[self.my_node_id as usize];
        let Some(s) = self.searchers().acquire(version)? else {
            return Err(searcher_expired(format!(
                "nodeID={} version={version}",
                self.my_node_id
            )));
        };
        Ok(Arc::new(self.new_shard_searcher(
            node_versions.to_vec(),
            Arc::clone(s.searcher().get_index_reader()),
            self.my_node_id,
        )?))
    }

    /// `reopen()`: reopen local reader.
    ///
    /// # Errors
    ///
    /// The errors of refreshing, recording, pruning and broadcasting.
    pub fn reopen(&self) -> Result<()> {
        let before = self.mgr().acquire()?;
        self.mgr().release(Arc::clone(&before))?;

        self.mgr().maybe_refresh()?;
        let after = self.mgr().acquire()?;
        let result = (|| -> Result<()> {
            if !Arc::ptr_eq(&after, &before) {
                // New searcher was opened
                let base = self
                    .base
                    .upgrade()
                    .expect("INVARIANT: the test instance holds the base state");
                let version = self.searchers().record(&after)?;
                self.searchers().prune(&PruneByAge::new(f64::from(
                    base.max_searcher_age_seconds(),
                ))?)?;
                base.broadcast_node_reopen(self.my_node_id, version, after.searcher())?;
            }
            Ok(())
        })();
        // finally
        let released = self.mgr().release(after);
        result?;
        released
    }

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The errors of closing the searchers, the manager, the writer and the
    /// directory.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `close()`
    /// (`ShardSearchingTestBase.java`, `NodeState.close`) closes the
    /// searchers, the manager, the writer and the directory, and leaves the
    /// closed objects to the garbage collector. The port's
    /// `MockDirectoryWrapper` can close its delegate only once no handle
    /// shares it, so this also drops the current shard searcher, the
    /// searchers, the manager and the writer before closing the directory;
    /// [`writer`](Self::writer), [`searchers`](Self::searchers) and
    /// [`mgr`](Self::mgr) panic afterwards.
    pub fn close(&self) -> Result<()> {
        let current = self
            .current_shard_searcher
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(current) = current {
            current.get_index_reader().dec_ref()?;
        }
        let searchers = write(&self.searchers).take();
        let mgr = write(&self.mgr).take();
        let writer = write(&self.writer).take();
        if let Some(searchers) = searchers {
            searchers.close()?;
        }
        if let Some(mgr) = mgr {
            mgr.close()?;
        }
        if let Some(writer) = writer {
            writer.close()?;
        }
        lock(&self.dir).close()
    }
}

/// `NodeState.ShardIndexSearcher` (`public class ShardIndexSearcher
/// extends IndexSearcher`): matches docs in the local shard but scores
/// based on aggregated stats ("mock distributed scoring") from all nodes.
#[derive(Debug)]
pub struct ShardIndexSearcher {
    searcher: IndexSearcher,
    overrides: Arc<ShardIndexSearcherOverrides>,
}

impl ShardIndexSearcher {
    /// `new ShardIndexSearcher(long[] nodeVersions, IndexReader
    /// localReader, int nodeID)`; `enclosing_node_id` is
    /// `NodeState.this.myNodeID`.
    #[allow(clippy::too_many_arguments)]
    fn new(
        node_versions: Vec<i64>,
        local_reader: Arc<dyn IndexReader>,
        node_id: i32,
        enclosing_node_id: i32,
        collection_stats_cache: CollectionStatsCache,
        term_stats_cache: TermStatsCache,
        base: Weak<ShardSearchingTestBase>,
    ) -> Result<Self> {
        let mut searcher = IndexSearcher::new(local_reader)?;
        assert_eq!(
            node_id, enclosing_node_id,
            "myNodeID={node_id} NodeState.this.myNodeID={enclosing_node_id}"
        );
        let overrides = Arc::new(ShardIndexSearcherOverrides {
            node_versions,
            my_node_id: node_id,
            collection_stats_cache,
            term_stats_cache,
            base,
        });
        searcher.set_overrides(Arc::clone(&overrides) as Arc<dyn IndexSearcherOverrides>);
        Ok(ShardIndexSearcher {
            searcher,
            overrides,
        })
    }

    /// The searcher, whose virtual methods are this class's overrides.
    pub fn searcher(&self) -> &IndexSearcher {
        &self.searcher
    }

    /// `nodeVersions` (`public final long[]`): version for the node
    /// searchers we search.
    pub fn node_versions(&self) -> &[i64] {
        &self.overrides.node_versions
    }

    /// `myNodeID` (`public final int`).
    pub fn my_node_id(&self) -> i32 {
        self.overrides.my_node_id
    }

    /// `getIndexReader()`.
    pub fn get_index_reader(&self) -> &Arc<dyn IndexReader> {
        self.searcher.get_index_reader()
    }

    /// `search(Query query, int numHits)` (the override).
    ///
    /// # Errors
    ///
    /// The errors of the shard searches and of the merge.
    pub fn search(&self, query: Arc<dyn Query>, num_hits: i32) -> Result<TopDocs> {
        self.searcher.search(query, num_hits)
    }

    /// `searchAfter(ScoreDoc after, Query query, int numHits)` (the
    /// override).
    ///
    /// # Errors
    ///
    /// The errors of the shard searches and of the merge.
    pub fn search_after(
        &self,
        after: Option<ScoreDoc>,
        query: Arc<dyn Query>,
        num_hits: i32,
    ) -> Result<TopDocs> {
        self.searcher.search_after(after, query, num_hits)
    }

    /// `search(Query query, int numHits, Sort sort)` (the override).
    ///
    /// # Errors
    ///
    /// The errors of the shard searches and of the merge.
    pub fn search_with_sort(
        &self,
        query: Arc<dyn Query>,
        num_hits: i32,
        sort: &Sort,
    ) -> Result<TopFieldDocs> {
        self.searcher.search_with_sort(query, num_hits, sort)
    }

    /// `localSearch(Query query, int numHits)`: `super.search(query,
    /// numHits)`.
    ///
    /// # Errors
    ///
    /// The errors of the search.
    pub fn local_search(&self, query: Arc<dyn Query>, num_hits: i32) -> Result<TopDocs> {
        self.searcher.super_search(query, num_hits)
    }

    /// `localSearchAfter(ScoreDoc after, Query query, int numHits)`:
    /// `super.searchAfter(after, query, numHits)`.
    ///
    /// # Errors
    ///
    /// The errors of the search.
    pub fn local_search_after(
        &self,
        after: Option<ScoreDoc>,
        query: Arc<dyn Query>,
        num_hits: i32,
    ) -> Result<TopDocs> {
        self.searcher.super_search_after(after, query, num_hits)
    }

    /// `localSearch(Query query, int numHits, Sort sort)`:
    /// `super.search(query, numHits, sort)`.
    ///
    /// # Errors
    ///
    /// The errors of the search.
    pub fn local_search_with_sort(
        &self,
        query: Arc<dyn Query>,
        num_hits: i32,
        sort: &Sort,
    ) -> Result<TopFieldDocs> {
        self.searcher.super_search_with_sort(query, num_hits, sort)
    }
}

/// The overrides of `ShardIndexSearcher`, with the state they read.
pub struct ShardIndexSearcherOverrides {
    node_versions: Vec<i64>,
    my_node_id: i32,
    collection_stats_cache: CollectionStatsCache,
    term_stats_cache: TermStatsCache,
    base: Weak<ShardSearchingTestBase>,
}

impl fmt::Debug for ShardIndexSearcherOverrides {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShardIndexSearcher")
            .field("node_versions", &self.node_versions)
            .field("my_node_id", &self.my_node_id)
            .finish_non_exhaustive()
    }
}

impl ShardIndexSearcherOverrides {
    /// `ShardSearchingTestBase.this`.
    fn base(&self) -> Arc<ShardSearchingTestBase> {
        self.base
            .upgrade()
            .expect("INVARIANT: the test instance holds the base state")
    }
}

impl IndexSearcherOverrides for ShardIndexSearcherOverrides {
    fn rewrite(
        &self,
        searcher: &IndexSearcher,
        original: Arc<dyn Query>,
    ) -> Result<Arc<dyn Query>> {
        let local_searcher = IndexSearcher::new(Arc::clone(searcher.get_index_reader()))?;
        let original = local_searcher.rewrite(original)?;
        let mut terms: BTreeSet<Term> = BTreeSet::new();
        original.visit(&mut term_collector(&mut terms));

        // Make a single request to remote nodes for term
        // stats:
        for node_id in 0..self.node_versions.len() as i32 {
            if node_id == self.my_node_id {
                continue;
            }

            let version = self.node_versions[node_id as usize];
            let mut missing: BTreeSet<Term> = BTreeSet::new();
            {
                let cache = lock(&self.term_stats_cache);
                for term in &terms {
                    let key = TermAndShardVersion {
                        node_id,
                        version,
                        term: term.clone(),
                    };
                    if !cache.contains_key(&key) {
                        missing.insert(term.clone());
                    }
                }
            }
            if !missing.is_empty() {
                let stats = self
                    .base()
                    .get_node_term_stats(&missing, node_id, version)?;
                let mut cache = lock(&self.term_stats_cache);
                for (term, value) in stats {
                    let key = TermAndShardVersion {
                        node_id,
                        version,
                        term,
                    };
                    cache.insert(key, value);
                }
            }
        }

        Ok(original)
    }

    fn term_statistics(
        &self,
        searcher: &IndexSearcher,
        term: &Term,
        doc_freq: i32,
        total_term_freq: i64,
    ) -> Result<TermStatistics> {
        let mut distributed_doc_freq: i64 = 0;
        let mut distributed_total_term_freq: i64 = 0;
        for node_id in 0..self.node_versions.len() as i32 {
            let sub_stats = if node_id == self.my_node_id {
                searcher.super_term_statistics(term, doc_freq, total_term_freq)?
            } else {
                let key = TermAndShardVersion {
                    node_id,
                    version: self.node_versions[node_id as usize],
                    term: term.clone(),
                };
                match lock(&self.term_stats_cache).get(&key) {
                    Some(stats) => stats.clone(),
                    None => continue, // term not found
                }
            };

            let node_doc_freq = sub_stats.doc_freq();
            distributed_doc_freq += node_doc_freq;

            let node_total_term_freq = sub_stats.total_term_freq();
            distributed_total_term_freq += node_total_term_freq;
        }
        assert!(distributed_doc_freq > 0);
        TermStatistics::new(
            term.bytes().clone(),
            distributed_doc_freq,
            distributed_total_term_freq,
        )
    }

    fn collection_statistics(
        &self,
        searcher: &IndexSearcher,
        field: &str,
    ) -> Result<Option<CollectionStatistics>> {
        // TODO: we could compute this on init and cache,
        // since we are re-inited whenever any nodes have a
        // new reader
        let mut doc_count: i64 = 0;
        let mut sum_total_term_freq: i64 = 0;
        let mut sum_doc_freq: i64 = 0;
        let mut max_doc: i64 = 0;

        for node_id in 0..self.node_versions.len() as i32 {
            let key = FieldAndShardVersion {
                node_id,
                version: self.node_versions[node_id as usize],
                field: field.to_string(),
            };
            let node_stats = if node_id == self.my_node_id {
                searcher.super_collection_statistics(field)?
            } else {
                lock(&self.collection_stats_cache).get(&key).cloned()
            };
            let Some(node_stats) = node_stats else {
                continue; // field not in sub at all
            };

            let node_doc_count = node_stats.doc_count();
            doc_count += node_doc_count;

            let node_sum_total_term_freq = node_stats.sum_total_term_freq();
            sum_total_term_freq += node_sum_total_term_freq;

            let node_sum_doc_freq = node_stats.sum_doc_freq();
            sum_doc_freq += node_sum_doc_freq;

            assert!(node_stats.max_doc() >= 0);
            max_doc += node_stats.max_doc();
        }

        if max_doc == 0 {
            Ok(None) // field not found across any node whatsoever
        } else {
            Ok(Some(CollectionStatistics::new(
                field,
                max_doc,
                doc_count,
                sum_total_term_freq,
                sum_doc_freq,
            )?))
        }
    }

    fn search(&self, searcher: &IndexSearcher, query: Arc<dyn Query>, n: i32) -> Result<TopDocs> {
        let mut shard_hits: Vec<TopDocs> = Vec::with_capacity(self.node_versions.len());
        for node_id in 0..self.node_versions.len() as i32 {
            let mut hits = if node_id == self.my_node_id {
                // My node; run using local shard searcher we
                // already aquired:
                searcher.super_search(Arc::clone(&query), n)?
            } else {
                self.base()
                    .search_node(
                        node_id,
                        &self.node_versions,
                        Arc::clone(&query),
                        None,
                        n,
                        None,
                    )?
                    .into_top_docs()
            };

            for sd in hits.score_docs.iter_mut() {
                sd.shard_index = node_id;
            }
            shard_hits.push(hits);
        }

        // Merge:
        TopDocs::merge(n as usize, &shard_hits)
    }

    fn search_after(
        &self,
        searcher: &IndexSearcher,
        after: Option<ScoreDoc>,
        query: Arc<dyn Query>,
        num_hits: i32,
    ) -> Result<TopDocs> {
        let Some(after) = after else {
            return searcher.super_search_after(None, query, num_hits);
        };
        let mut shard_hits: Vec<TopDocs> = Vec::with_capacity(self.node_versions.len());
        // results are merged in that order: score, shardIndex, doc. therefore we set
        // after to after.score and depending on the nodeID we set doc to either:
        // - not collect any more documents with that score (only with worse score)
        // - collect more documents with that score (and worse) following the last collected
        // document
        // - collect all documents with that score (and worse)
        let mut shard_after = ScoreDoc::new(after.doc, after.score);
        for node_id in 0..self.node_versions.len() as i32 {
            if node_id < after.shard_index {
                // all documents with after.score were already collected, so collect
                // only documents with worse scores.
                let base = self.base();
                let node = &base.nodes()[node_id as usize];
                let s = node.acquire_versions(&self.node_versions)?;
                // Setting after.doc to reader.maxDoc-1 is a way to tell
                // TopScoreDocCollector that no more docs with that score should
                // be collected. note that in practice the shard which sends the
                // request to a remote shard won't have reader.maxDoc at hand, so
                // it will send some arbitrary value which will be fixed on the
                // other end.
                shard_after.doc = s.get_index_reader().max_doc() - 1;
                node.release(&s)?;
            } else if node_id == after.shard_index {
                // collect all documents following the last collected doc with
                // after.score + documents with worse scores.
                shard_after.doc = after.doc;
            } else {
                // all documents with after.score (and worse) should be collected
                // because they didn't make it to top-N in the previous round.
                shard_after.doc = -1;
            }
            let mut hits = if node_id == self.my_node_id {
                // My node; run using local shard searcher we
                // already aquired:
                searcher.super_search_after(Some(shard_after), Arc::clone(&query), num_hits)?
            } else {
                self.base()
                    .search_node(
                        node_id,
                        &self.node_versions,
                        Arc::clone(&query),
                        None,
                        num_hits,
                        Some(shard_after),
                    )?
                    .into_top_docs()
            };

            for sd in hits.score_docs.iter_mut() {
                sd.shard_index = node_id;
            }
            shard_hits.push(hits);
            // System.out.println("  node=" + nodeID + " totHits=" + shardHits[nodeID].totalHits);
        }

        // Merge:
        TopDocs::merge(num_hits as usize, &shard_hits)
    }

    fn search_with_sort(
        &self,
        searcher: &IndexSearcher,
        query: Arc<dyn Query>,
        n: i32,
        sort: &Sort,
    ) -> Result<TopFieldDocs> {
        let mut shard_hits: Vec<TopFieldDocs> = Vec::with_capacity(self.node_versions.len());
        for node_id in 0..self.node_versions.len() as i32 {
            let mut hits = if node_id == self.my_node_id {
                // My node; run using local shard searcher we
                // already aquired:
                searcher.super_search_with_sort(Arc::clone(&query), n, sort)?
            } else {
                self.base()
                    .search_node(
                        node_id,
                        &self.node_versions,
                        Arc::clone(&query),
                        Some(sort),
                        n,
                        None,
                    )?
                    .into_top_field_docs()
            };

            for sd in hits.score_docs.iter_mut() {
                sd.score_doc.shard_index = node_id;
            }
            shard_hits.push(hits);
        }

        // Merge:
        TopDocs::merge_sorted(sort, n as usize, &shard_hits)
    }
}

/// `SearcherAndVersion` (`protected record`): an `IndexSearcher` and
/// associated version (lease).
#[derive(Debug, Clone)]
pub struct SearcherAndVersion {
    /// `searcher`.
    pub searcher: Arc<ManagedSearcher>,
    /// `version`.
    pub version: i64,
}
