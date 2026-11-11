//! Port of `lucene/core/src/test/org/apache/lucene/search/TestIndexSearcher.java`.
//!
//! The anonymous `IndexSearcher` subclasses that override the protected
//! `slices(List<LeafReaderContext>)` are [`IndexSearcherOverrides`]
//! implementations (J2R guide, section 6); the lambda executors
//! (`Runnable::run`, the counting `task -> { …; task.run(); }`) are
//! [`Executor`] implementations.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, SortedDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::multi_reader::MultiReader;
use rucene::index::reader_context::LeafReaderContext;
use rucene::index::IndexReader;
use rucene::search::field_comparator::SortValue;
use rucene::search::field_doc::FieldDoc;
use rucene::search::index_searcher::{
    IndexSearcher, IndexSearcherOverrides, LeafReaderContextPartition, LeafSlice,
};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, Command, ConstantScoreQuery, Executor, MatchAllDocsQuery,
    MatchNoDocsQuery, Occur, Query, QueryCache, QueryCachingPolicy, RejectedExecutionException,
    Sort, SortField, SortFieldType, Weight,
};
use rucene::util::NamedThreadFactory;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down, term};
use crate::core_support::{assert_equals, assert_not_null, random};

/// The instance of `TestIndexSearcher`.
#[derive(Default)]
pub struct IndexSearcherTest {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
}

impl IndexSearcherTest {
    fn reader(&self) -> Arc<dyn IndexReader> {
        Arc::clone(self.reader.as_ref().expect("setUp ran")) as Arc<dyn IndexReader>
    }
}

/// `TestIndexSearcher`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<IndexSearcherTest>(
        "org.apache.lucene.search.TestIndexSearcher",
        vec![
            set_up(index_searcher_set_up),
            tear_down(index_searcher_tear_down),
            ftest("testHugeN", test_huge_n),
            ftest(
                "testSearchAfterPassedMaxDoc",
                test_search_after_passed_max_doc,
            ),
            ftest("testCount", test_count),
            ftest("testGetQueryCache", test_get_query_cache),
            ftest("testGetQueryCachingPolicy", test_get_query_caching_policy),
            ftest(
                "testGetSlicesNoLeavesNoExecutor",
                test_get_slices_no_leaves_no_executor,
            ),
            ftest(
                "testGetSlicesNoLeavesWithExecutor",
                test_get_slices_no_leaves_with_executor,
            ),
            ftest("testGetSlices", test_get_slices),
            ftest(
                "testSlicesOffloadedToTheExecutor",
                test_slices_offloaded_to_the_executor,
            ),
            ftest(
                "testNullExecutorNonNullTaskExecutor",
                test_null_executor_non_null_task_executor,
            ),
            ftest(
                "testSegmentPartitionsSameSlice",
                test_segment_partitions_same_slice,
            ),
        ],
    )
}

fn index_searcher_set_up(f: &mut IndexSearcherTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let random = random();
    for i in 0..100 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            &i.to_string(),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field2",
            &(i % 2 == 0).to_string(),
            Store::NO,
        )));
        doc.add(Box::new(SortedDocValuesField::new(
            "field2",
            LuceneTestCase::new_bytes_ref_str(&(i % 2 == 0).to_string()),
        )));
        iw.add_document(&doc)?;

        if random.next_boolean() {
            iw.commit()?;
        }
    }
    f.reader = Some(iw.get_reader()?);
    iw.close()?;
    f.dir = Some(dir);
    Ok(())
}

fn index_searcher_tear_down(f: &mut IndexSearcherTest) -> Result<()> {
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// Java's `Runnable::run` as an `Executor`.
struct DirectExecutor;

impl Executor for DirectExecutor {
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        command();
        Ok(())
    }
}

// should not throw exception
fn test_huge_n(f: &mut IndexSearcherTest) -> Result<()> {
    let service = Arc::new(ThreadPoolExecutor::new(
        4,
        4,
        Duration::from_millis(0),
        NamedThreadFactory::new("TestIndexSearcher"),
    ));

    let searchers = [
        IndexSearcher::new(f.reader())?,
        IndexSearcher::with_executor(f.reader(), Some(Arc::clone(&service) as Arc<dyn Executor>))?,
    ];
    let queries: [Arc<dyn Query>; 2] = [
        Arc::new(MatchAllDocsQuery::instance()),
        Arc::new(TermQuery::new(term("field", "1"))),
    ];
    let sorts = [
        None,
        Some(Sort::new_fields(vec![SortField::new(
            Some("field2".to_string()),
            SortFieldType::String,
        )?])?),
    ];
    let afters = [
        None,
        Some(FieldDoc::with_fields(
            0,
            0.0,
            vec![SortValue::Bytes(LuceneTestCase::new_bytes_ref_str("boo!"))],
        )),
    ];

    for searcher in &searchers {
        for after in &afters {
            for query in &queries {
                for sort in &sorts {
                    searcher.search(Arc::clone(query), i32::MAX)?;
                    searcher.search_after(
                        after.as_ref().map(|a| a.score_doc),
                        Arc::clone(query),
                        i32::MAX,
                    )?;
                    if let Some(sort) = sort {
                        searcher.search_with_sort(Arc::clone(query), i32::MAX, sort)?;
                        searcher.search_with_sort_and_scores(
                            Arc::clone(query),
                            i32::MAX,
                            sort,
                            true,
                        )?;
                        searcher.search_with_sort_and_scores(
                            Arc::clone(query),
                            i32::MAX,
                            sort,
                            false,
                        )?;
                        searcher.search_after_with_sort(
                            after.clone(),
                            Arc::clone(query),
                            i32::MAX,
                            sort,
                        )?;
                        searcher.search_after_with_sort_and_scores(
                            after.clone(),
                            Arc::clone(query),
                            i32::MAX,
                            sort,
                            true,
                        )?;
                        searcher.search_after_with_sort_and_scores(
                            after.clone(),
                            Arc::clone(query),
                            i32::MAX,
                            sort,
                            false,
                        )?;
                    }
                }
            }
        }
    }

    TestUtil::shutdown_executor_service(Some(&*service));
    Ok(())
}

fn test_search_after_passed_max_doc(_f: &mut IndexSearcherTest) -> Result<()> {
    // LUCENE-5128: ensure we get a meaningful message if searchAfter exceeds maxDoc
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    let r = w.get_reader()?;
    w.close()?;

    let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    expect_throws("java.lang.IllegalArgumentException", || {
        s.search_after(
            Some(rucene::search::ScoreDoc::new(r.max_doc(), 0.54)),
            Arc::new(MatchAllDocsQuery::instance()),
            10,
        )
    });

    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

fn test_count(_f: &mut IndexSearcherTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(100);
    for _i in 0..num_docs {
        let mut doc = Document::new();
        if random().next_boolean() {
            doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
        }
        if random().next_boolean() {
            doc.add(Box::new(StringField::new("foo", "baz".into(), Store::NO)?));
        }
        if LuceneTestCase::rarely() {
            doc.add(Box::new(StringField::new(
                "delete",
                "yes".into(),
                Store::NO,
            )?));
        }
        w.add_document(&doc)?;
    }
    for delete in [false, true] {
        if delete {
            w.delete_documents(term("delete", "yes"))?;
        }
        let reader = w.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        // Test multiple queries, some of them are optimized by IndexSearcher.count()
        let mut bq = BooleanQueryBuilder::new();
        bq.add(Arc::new(TermQuery::new(term("foo", "bar"))), Occur::SHOULD)?;
        bq.add(Arc::new(TermQuery::new(term("foo", "baz"))), Occur::SHOULD)?;
        let queries: Vec<Arc<dyn Query>> = vec![
            Arc::new(MatchAllDocsQuery::instance()),
            Arc::new(MatchNoDocsQuery::instance()),
            Arc::new(TermQuery::new(term("foo", "bar"))),
            Arc::new(ConstantScoreQuery::new(Arc::new(TermQuery::new(term(
                "foo", "baz",
            ))))),
            Arc::new(bq.build()),
        ];
        for query in queries {
            assert_equals(
                &i64::from(searcher.count(Arc::clone(&query))?),
                &searcher.search(query, 1)?.total_hits.value(),
            );
        }
        drop(searcher);
        reader.close()?;
    }
    w.close()?;
    dir.close()?;
    Ok(())
}

/// `assertEquals(expected, actual)` over two shared instances (Java's
/// `Object.equals` is identity for these anonymous classes).
fn assert_same_arc<T: ?Sized>(expected: Option<&Arc<T>>, actual: Option<&Arc<T>>) {
    let same = match (expected, actual) {
        (None, None) => true,
        (Some(e), Some(a)) => Arc::ptr_eq(e, a),
        _ => false,
    };
    randomizedtesting::junit::Assert::assert_true(same);
}

/// The anonymous `QueryCache` of `testGetQueryCache`.
#[derive(Debug)]
struct DummyCache;

impl QueryCache for DummyCache {
    fn do_cache(
        &self,
        weight: Arc<dyn Weight>,
        _policy: Arc<dyn QueryCachingPolicy>,
    ) -> Arc<dyn Weight> {
        weight
    }
}

fn empty_multi_reader() -> Result<Arc<dyn IndexReader>> {
    Ok(Arc::new(MultiReader::new(Vec::new(), true)?))
}

fn test_get_query_cache(_f: &mut IndexSearcherTest) -> Result<()> {
    let mut searcher = IndexSearcher::new(empty_multi_reader()?)?;
    assert_same_arc(
        IndexSearcher::get_default_query_cache().as_ref(),
        searcher.get_query_cache(),
    );
    let dummy_cache: Arc<dyn QueryCache> = Arc::new(DummyCache);
    searcher.set_query_cache(Some(Arc::clone(&dummy_cache)));
    assert_same_arc(Some(&dummy_cache), searcher.get_query_cache());

    IndexSearcher::set_default_query_cache(Some(Arc::clone(&dummy_cache)));
    searcher = IndexSearcher::new(empty_multi_reader()?)?;
    assert_same_arc(Some(&dummy_cache), searcher.get_query_cache());

    searcher.set_query_cache(None);
    assert_same_arc(None, searcher.get_query_cache());

    IndexSearcher::set_default_query_cache(None);
    searcher = IndexSearcher::new(empty_multi_reader()?)?;
    assert_same_arc(None, searcher.get_query_cache());
    Ok(())
}

/// The anonymous `QueryCachingPolicy` of `testGetQueryCachingPolicy`.
#[derive(Debug)]
struct DummyPolicy;

impl QueryCachingPolicy for DummyPolicy {
    fn should_cache(&self, _query: &dyn Query) -> Result<bool> {
        Ok(false)
    }

    fn on_use(&self, _query: &dyn Query) {}
}

fn test_get_query_caching_policy(_f: &mut IndexSearcherTest) -> Result<()> {
    let mut searcher = IndexSearcher::new(empty_multi_reader()?)?;
    assert_same_arc(
        IndexSearcher::get_default_query_caching_policy().as_ref(),
        searcher.get_query_caching_policy(),
    );
    let dummy_policy: Arc<dyn QueryCachingPolicy> = Arc::new(DummyPolicy);
    searcher.set_query_caching_policy(Arc::clone(&dummy_policy));
    assert_same_arc(Some(&dummy_policy), searcher.get_query_caching_policy());

    IndexSearcher::set_default_query_caching_policy(Some(Arc::clone(&dummy_policy)));
    searcher = IndexSearcher::new(empty_multi_reader()?)?;
    assert_same_arc(Some(&dummy_policy), searcher.get_query_caching_policy());
    Ok(())
}

fn test_get_slices_no_leaves_no_executor(_f: &mut IndexSearcherTest) -> Result<()> {
    let searcher = IndexSearcher::new(empty_multi_reader()?)?;
    let slices = searcher.get_slices()?;
    assert_equals(&0, &slices.len());
    Ok(())
}

fn test_get_slices_no_leaves_with_executor(_f: &mut IndexSearcherTest) -> Result<()> {
    let searcher =
        IndexSearcher::with_executor(empty_multi_reader()?, Some(Arc::new(DirectExecutor)))?;
    let slices = searcher.get_slices()?;
    assert_equals(&0, &slices.len());
    Ok(())
}

/// The anonymous subclass of `testGetSlices`: `slices(leaves, 1, 1, false)`.
#[derive(Debug)]
struct OneSegmentPerSlice;

impl IndexSearcherOverrides for OneSegmentPerSlice {
    fn slices(
        &self,
        _searcher: &IndexSearcher,
        leaves: &[Arc<LeafReaderContext>],
    ) -> Result<Vec<LeafSlice>> {
        IndexSearcher::compute_slices(leaves, 1, 1, false)
    }
}

fn test_get_slices(_f: &mut IndexSearcherTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for _i in 0..10 {
        w.add_document(&Document::new())?;
        // manually flush, so we get to create multiple segments almost all the times, as well as
        // multiple slices
        w.flush()?;
    }
    let r = w.get_reader()?;
    w.close()?;
    let num_leaves = (Arc::clone(&r) as Arc<dyn IndexReader>).leaves()?.len();

    {
        // without executor
        let searcher = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
        let slices = searcher.get_slices()?;
        assert_equals(&1, &slices.len());
        assert_equals(&num_leaves, &slices[0].partitions().len());
    }
    {
        // force creation of multiple slices, and provide an executor
        let mut searcher = IndexSearcher::with_executor(
            Arc::clone(&r) as Arc<dyn IndexReader>,
            Some(Arc::new(DirectExecutor)),
        )?;
        searcher.set_overrides(Arc::new(OneSegmentPerSlice));
        let slices = searcher.get_slices()?;
        for slice in slices {
            assert_equals(&1, &slice.partitions().len());
        }
        assert_equals(&num_leaves, &slices.len());
    }
    r.close()?;
    dir.close()?;
    Ok(())
}

/// The counting lambda executor of `testSlicesOffloadedToTheExecutor`.
struct CountingExecutor(Arc<AtomicI32>);

impl Executor for CountingExecutor {
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        self.0.fetch_add(1, Ordering::SeqCst);
        command();
        Ok(())
    }
}

/// The anonymous subclass of `testSlicesOffloadedToTheExecutor`: one slice
/// per leaf.
#[derive(Debug)]
struct SlicePerLeaf;

impl IndexSearcherOverrides for SlicePerLeaf {
    fn slices(
        &self,
        _searcher: &IndexSearcher,
        leaves: &[Arc<LeafReaderContext>],
    ) -> Result<Vec<LeafSlice>> {
        let mut slices = Vec::new();
        for ctx in leaves {
            slices.push(LeafSlice::new(vec![
                LeafReaderContextPartition::create_for_entire_segment(Arc::clone(ctx))?,
            ]));
        }
        Ok(slices)
    }
}

fn test_slices_offloaded_to_the_executor(f: &mut IndexSearcherTest) -> Result<()> {
    let leaves = f.reader().leaves()?;
    let num_executions = Arc::new(AtomicI32::new(0));
    let mut searcher = IndexSearcher::with_executor(
        f.reader(),
        Some(Arc::new(CountingExecutor(Arc::clone(&num_executions)))),
    )?;
    searcher.set_overrides(Arc::new(SlicePerLeaf));
    searcher.search(Arc::new(MatchAllDocsQuery::instance()), 10)?;
    assert_equals(
        &(leaves.len() as i32 - 1),
        &num_executions.load(Ordering::SeqCst),
    );
    Ok(())
}

fn test_null_executor_non_null_task_executor(f: &mut IndexSearcherTest) -> Result<()> {
    let index_searcher = IndexSearcher::new(f.reader())?;
    assert_not_null(&Some(index_searcher.get_task_executor()));
    Ok(())
}

/// The anonymous subclass of `testSegmentPartitionsSameSlice`: two
/// partitions of each leaf in the same slice.
#[derive(Debug)]
struct TwoPartitionsPerSlice;

impl IndexSearcherOverrides for TwoPartitionsPerSlice {
    fn slices(
        &self,
        _searcher: &IndexSearcher,
        leaves: &[Arc<LeafReaderContext>],
    ) -> Result<Vec<LeafSlice>> {
        let mut slices = Vec::new();
        for ctx in leaves {
            slices.push(LeafSlice::new(vec![
                LeafReaderContextPartition::create_from_and_to(Arc::clone(ctx), 0, 1)?,
                LeafReaderContextPartition::create_from_and_to(
                    Arc::clone(ctx),
                    1,
                    ctx.leaf_reader().max_doc(),
                )?,
            ]));
        }
        Ok(slices)
    }
}

fn test_segment_partitions_same_slice(f: &mut IndexSearcherTest) -> Result<()> {
    let mut index_searcher =
        IndexSearcher::with_executor(f.reader(), Some(Arc::new(DirectExecutor)))?;
    index_searcher.set_overrides(Arc::new(TwoPartitionsPerSlice));

    LuceneTestCase::assume_true(
        "Needs at least 2 docs in the same segment",
        index_searcher
            .get_leaf_contexts()
            .iter()
            .all(|ctx| ctx.leaf_reader().max_doc() > 1),
    );
    let e = expect_throws("java.lang.IllegalStateException", || {
        index_searcher.get_slices().map(|_| ())
    });
    assert_equals(
        "The same slice targets multiple leaf partitions of the same leaf reader context. A physical segment should rather get partitioned to be searched concurrently from as many slices as the number of leaf partitions it is split into.",
        e.message().unwrap_or_default(),
    );
    Ok(())
}
