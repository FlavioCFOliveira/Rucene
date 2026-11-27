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

//! The types declared in `ReadTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::any::TypeId;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_reader::{IndexReader, StoredFields};
use rucene::index::multi_bits::MultiBits;
use rucene::search::collector::CollectorManager;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::sort::Sort;
use rucene::search::top_docs::TopDocs;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::PerfTask;

/// `DEFAULT_SEARCH_NUM_HITS`.
pub(crate) const DEFAULT_SEARCH_NUM_HITS: i32 = 10;

/// A `CollectorManager<?, ?>` whose result the caller ignores:
/// `searcher.search(q, collectorManager)`.
pub trait AnyCollectorManager: Send + Sync {
    /// `searcher.search(q, this)`, discarding the reduced result.
    ///
    /// # Errors
    ///
    /// The exception of the search.
    fn search(&self, searcher: &IndexSearcher, query: Arc<dyn Query>) -> Result<()>;
}

/// Adapts a typed [`CollectorManager`] to [`AnyCollectorManager`].
pub struct ErasedCollectorManager<M>(pub M);

impl<M> AnyCollectorManager for ErasedCollectorManager<M>
where
    M: CollectorManager + Send + Sync,
    M::Collector: Send + 'static,
{
    fn search(&self, searcher: &IndexSearcher, query: Arc<dyn Query>) -> Result<()> {
        searcher.search_with_manager(query, &self.0).map(|_| ())
    }
}

/// The state of `org.apache.lucene.benchmark.byTask.tasks.ReadTask`:
/// `queryMaker` (final) and `numHits`.
pub struct ReadTaskBase {
    query_maker: Option<Arc<dyn QueryMaker>>,
    num_hits: AtomicI32,
}

impl ReadTaskBase {
    /// `ReadTask(PerfRunData runData)` for the read task class `class`:
    /// `queryMaker = withSearch() ? getQueryMaker() : null`, where
    /// `get_query_maker` is the class's `getQueryMaker()` (Java calls the
    /// virtual method from the constructor).
    ///
    /// # Errors
    ///
    /// The `RuntimeException` of creating the query maker.
    pub fn new(
        with_search: bool,
        get_query_maker: impl FnOnce() -> Result<Option<Arc<dyn QueryMaker>>>,
    ) -> Result<Self> {
        let query_maker = if with_search {
            get_query_maker()?
        } else {
            None
        };
        Ok(ReadTaskBase {
            query_maker,
            num_hits: AtomicI32::new(0),
        })
    }

    /// `super.clone()`.
    pub fn clone_base(&self) -> Self {
        ReadTaskBase {
            query_maker: self.query_maker.clone(),
            num_hits: AtomicI32::new(self.num_hits.load(Ordering::SeqCst)),
        }
    }

    /// `queryMaker`.
    pub fn query_maker(&self) -> Option<&Arc<dyn QueryMaker>> {
        self.query_maker.as_ref()
    }
}

/// `getRunData().getQueryMaker(this)` for the read task class `T`.
///
/// # Errors
///
/// As [`PerfRunData::get_query_maker`].
pub fn query_maker_of<T: 'static>(run_data: &PerfRunData) -> Result<Option<Arc<dyn QueryMaker>>> {
    run_data.query_maker_for_class(TypeId::of::<T>()).map(Some)
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.ReadTask`
/// (`public abstract class ReadTask extends PerfTask`): read index (abstract)
/// task. Sub classes implement withSearch(), withWarm(), withTraverse() and
/// withRetrieve() methods to configure the actual action.
///
/// Note: All ReadTasks reuse the reader if it is already open. Otherwise a
/// reader is opened at start and closed at the end.
///
/// The `search.num.hits` config parameter sets the top number of hits to
/// collect during searching. If `print.hits.field` is set, then each hit is
/// printed along with the value of that field.
///
/// Other side effects: none.
///
/// A read task's [`PerfTask::do_logic`] and [`PerfTask::setup`] are
/// [`do_logic`] and [`setup`].
pub trait ReadTask: PerfTask {
    /// The embedded `ReadTask` state.
    fn read_base(&self) -> &ReadTaskBase;

    /// `getQueryMaker()`: return query maker used for this task.
    ///
    /// # Errors
    ///
    /// As [`PerfRunData::get_query_maker`].
    fn get_query_maker(&self) -> Result<Option<Arc<dyn QueryMaker>>>;

    /// `withSearch()`: return true if search should be performed.
    fn with_search(&self) -> bool;

    /// `withCollector()`.
    fn with_collector(&self) -> bool {
        false
    }

    /// `withWarm()`: return true if warming should be performed.
    fn with_warm(&self) -> bool;

    /// `withTraverse()`: return true if, with search, results should be
    /// traversed.
    fn with_traverse(&self) -> bool;

    /// `withTotalHits()`: whether totalHits should be computed (only useful
    /// with field sort).
    fn with_total_hits(&self) -> bool {
        false
    }

    /// `traversalSize()`: specify the number of hits to traverse. Tasks
    /// should override this if they want to restrict the number of hits that
    /// are traversed when `withTraverse()` is true. Must be greater than 0.
    ///
    /// Read task calculates the traversal as: `Math.min(hits.length(),
    /// traversalSize())`.
    fn traversal_size(&self) -> i32 {
        i32::MAX
    }

    /// `numHits()`: specify the number of hits to retrieve. Tasks should
    /// override this if they want to restrict the number of hits that are
    /// collected during searching. Must be greater than 0. 10 by default, or
    /// search.num.hits config if set.
    fn num_hits(&self) -> i32 {
        self.read_base().num_hits.load(Ordering::SeqCst)
    }

    /// `withRetrieve()`: return true if, with search and results traversing,
    /// docs should be retrieved.
    fn with_retrieve(&self) -> bool;

    /// `getSort()`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` where a subclass has no sort.
    fn get_sort(&self) -> Result<Option<Sort>> {
        Ok(None)
    }

    /// `withTopDocs(IndexSearcher searcher, Query q, TopDocs hits)`.
    ///
    /// # Errors
    ///
    /// The exception of retrieving documents.
    fn with_top_docs(
        &self,
        searcher: &Arc<IndexSearcher>,
        q: &Arc<dyn Query>,
        hits: &TopDocs,
    ) -> Result<i32> {
        with_top_docs(self, searcher, q, hits)
    }

    /// `createCollectorManager()`.
    ///
    /// # Errors
    ///
    /// The exception of building the manager.
    fn create_collector_manager(&self) -> Result<Box<dyn AnyCollectorManager>> {
        create_collector_manager(self)
    }

    /// `retrieveDoc(StoredFields storedFields, int id)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of reading the document.
    fn retrieve_doc(&self, stored_fields: &dyn StoredFields, id: i32) -> Result<Option<Document>> {
        stored_fields.document(id).map(Some)
    }
}

/// `ReadTask.doLogic()`.
///
/// # Errors
///
/// The exception of opening the reader, searching or retrieving.
pub fn do_logic<T: ReadTask + ?Sized>(task: &T) -> Result<i32> {
    let mut res = 0;
    let run_data = task.base().get_run_data();

    // open reader or use existing one
    let existing = run_data.get_index_searcher()?; // (will incRef the reader)

    let (searcher, reader, close_searcher): (Arc<IndexSearcher>, Arc<dyn IndexReader>, bool) =
        match existing {
            None => {
                // open our own reader
                let dir = run_data
                    .get_directory()
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                let reader: Arc<dyn IndexReader> = directory_reader::open(dir)?;
                let searcher = Arc::new(IndexSearcher::new(Arc::clone(&reader))?);
                (searcher, reader, true)
            }
            Some(searcher) => {
                // use existing one; this passes +1 ref to us
                let reader = Arc::clone(searcher.get_index_reader());
                (searcher, reader, false)
            }
        };

    let stored_fields = reader.stored_fields()?;

    // optionally warm and add num docs traversed to count
    if task.with_warm() {
        let live_docs = MultiBits::get_live_docs(&reader)?;
        for m in 0..reader.max_doc() {
            if live_docs.as_ref().is_none_or(|l| l.get(m as usize)) {
                let _doc = stored_fields.document(m)?;
                res += 1;
            }
        }
    }

    if task.with_search() {
        res += 1;
        let q = task
            .read_base()
            .query_maker()
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .make_query()?;
        let sort = task.get_sort()?;
        let mut hits: Option<TopDocs> = None;
        let num_hits = task.num_hits();
        if num_hits > 0 {
            if !task.with_collector() {
                if let Some(sort) = sort {
                    // TODO: instead of always passing false we
                    // should detect based on the query; if we make
                    // the IndexSearcher search methods that take
                    // Weight public again, we can go back to
                    // pulling the Weight ourselves:
                    let total_hits_threshold = if task.with_total_hits() { i32::MAX } else { 1 };
                    let collector_manager =
                        TopFieldCollectorManager::new(sort, num_hits, None, total_hits_threshold)?;
                    hits = Some(
                        searcher
                            .search_with_manager(Arc::clone(&q), &collector_manager)?
                            .to_top_docs(),
                    );
                } else {
                    hits = Some(searcher.search(Arc::clone(&q), num_hits)?);
                }
            } else {
                task.create_collector_manager()?
                    .search(&searcher, Arc::clone(&q))?;
                // hits = collector.topDocs();
            }

            if let Some(hits) = &hits {
                let print_hits_field = run_data.get_config().get_str("print.hits.field", None)?;
                if let Some(print_hits_field) = print_hits_field.filter(|f| !f.is_empty()) {
                    println!("totalHits = {}", hits.total_hits);
                    println!("maxDoc()  = {}", reader.max_doc());
                    println!("numDocs() = {}", reader.num_docs());
                    for (i, score_doc) in hits.score_docs.iter().enumerate() {
                        let doc_id = score_doc.doc;
                        let doc = stored_fields.document(doc_id)?;
                        println!(
                            "  {i}: doc={doc_id} score={} {print_hits_field} ={}",
                            crate::jdk::float_to_string(score_doc.score),
                            doc.get(&print_hits_field)
                                .unwrap_or_else(|| "null".to_string())
                        );
                    }
                }

                res += task.with_top_docs(&searcher, &q, hits)?;
            }
        }
    }

    if close_searcher {
        reader.close()?;
    } else {
        // Release our +1 ref from above
        reader.dec_ref()?;
    }
    Ok(res)
}

/// `ReadTask.withTopDocs(IndexSearcher searcher, Query q, TopDocs hits)`.
///
/// # Errors
///
/// The exception of retrieving documents.
pub fn with_top_docs<T: ReadTask + ?Sized>(
    task: &T,
    searcher: &Arc<IndexSearcher>,
    _q: &Arc<dyn Query>,
    hits: &TopDocs,
) -> Result<i32> {
    let mut res = 0;
    if task.with_traverse() {
        let score_docs = &hits.score_docs;
        let traversal_size = (score_docs.len() as i64).min(i64::from(task.traversal_size()));
        let stored_fields = searcher.stored_fields()?;
        if traversal_size > 0 {
            let retrieve = task.with_retrieve();
            for score_doc in &score_docs[..traversal_size as usize] {
                let id = score_doc.doc;
                res += 1;
                if retrieve {
                    let document = task.retrieve_doc(&*stored_fields, id)?;
                    res += if document.is_some() { 1 } else { 0 };
                }
            }
        }
    }
    Ok(res)
}

/// `ReadTask.createCollectorManager()`.
///
/// # Errors
///
/// The `IllegalArgumentException` of the manager.
pub fn create_collector_manager<T: ReadTask + ?Sized>(
    task: &T,
) -> Result<Box<dyn AnyCollectorManager>> {
    Ok(Box::new(ErasedCollectorManager(
        TopScoreDocCollectorManager::new(
            task.num_hits(),
            None,
            if task.with_total_hits() { i32::MAX } else { 1 },
        )?,
    )))
}

/// `ReadTask.setup()`.
///
/// # Errors
///
/// The exception of reading `search.num.hits`.
pub fn setup<T: ReadTask + ?Sized>(task: &T) -> Result<()> {
    let num_hits = task
        .base()
        .get_run_data()
        .get_config()
        .get_int("search.num.hits", DEFAULT_SEARCH_NUM_HITS)?;
    task.read_base().num_hits.store(num_hits, Ordering::SeqCst);
    Ok(())
}

/// The `PerfTask` methods every read task shares: `doLogic()` and
/// `setup()` of `ReadTask`, the base accessors, and `instanceof` over the
/// given class names.
macro_rules! read_task_perf_methods {
    ($($class:literal),*) => {
        fn base(&self) -> &$crate::by_task::tasks::perf_task::PerfTaskBase {
            &self.base
        }

        fn as_any(&self) -> &dyn ::std::any::Any {
            self
        }

        fn do_logic(&self) -> ::rucene::error::Result<i32> {
            $crate::by_task::tasks::read_task::do_logic(self)
        }

        fn setup(&self) -> ::rucene::error::Result<()> {
            $crate::by_task::tasks::read_task::setup(self)
        }

        fn is_instance_of(&self, simple_class_name: &str) -> bool {
            matches!(simple_class_name, $($class)|* | "ReadTask")
        }
    };
}
pub(crate) use read_task_perf_methods;
