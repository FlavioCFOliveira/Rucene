//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTopDocsCollector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{IndexReader, LeafReaderContext, MultiTerms, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::term_query::TermQuery;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::{
    empty_top_docs, BooleanQueryBuilder, CollectionError, CollectionResult, Collector,
    CollectorManager, DocScoreEncoder, HitQueue, LeafCollector, MatchAllDocsQuery, Occur, Query,
    ScoreDoc, ScoreMode, TopDocs, TopDocsCollector, TopScoreDocCollectorManager,
};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::line_file_docs::LineFileDocs;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, assert_equals_delta, assert_null, random, IAE};

/// The instance of `TestTopDocsCollector`.
#[derive(Default)]
pub struct TopDocsCollectorTest {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
}

/// `TestTopDocsCollector`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<TopDocsCollectorTest>(
        "org.apache.lucene.search.TestTopDocsCollector",
        vec![
            set_up(top_docs_collector_set_up),
            tear_down(top_docs_collector_tear_down),
            ftest("testInvalidArguments", test_invalid_arguments),
            ftest("testZeroResults", test_zero_results),
            ftest("testFirstResultsPage", test_first_results_page),
            ftest("testSecondResultsPages", test_second_results_pages),
            ftest("testGetAllResults", test_get_all_results),
            ftest("testGetResultsFromStart", test_get_results_from_start),
            ftest("testIllegalArguments", test_illegal_arguments),
            ftest("testResultsOrder", test_results_order),
            ftest("testSetMinCompetitiveScore", test_set_min_competitive_score),
            ftest(
                "testSharedCountCollectorManager",
                test_shared_count_collector_manager,
            ),
            ftest("testTotalHits", test_total_hits),
            ftest(
                "testRelationVsTopDocsCount",
                test_relation_vs_top_docs_count,
            ),
            ftest("testConcurrentMinScore", test_concurrent_min_score),
            ftest(
                "testRandomMinCompetitiveScore",
                test_random_min_competitive_score,
            ),
            ftest(
                "testRealisticConcurrentMinimumScore",
                test_realistic_concurrent_minimum_score,
            ),
        ],
    )
}

/// `private static final class MyTopDocsCollectorMananger`.
struct MyTopDocsCollectorMananger {
    num_hits: i32,
}

impl CollectorManager for MyTopDocsCollectorMananger {
    type Collector = MyTopDocsCollector;
    type Output = MyTopDocsCollector;

    fn new_collector(&self) -> Result<MyTopDocsCollector> {
        MyTopDocsCollector::new(self.num_hits)
    }

    fn reduce(&self, collectors: Vec<MyTopDocsCollector>) -> Result<MyTopDocsCollector> {
        let mut total_hits = 0;
        let mut my_top_docs_collector = MyTopDocsCollector::new(self.num_hits)?;
        for mut collector in collectors {
            total_hits += collector.total_hits;
            // `for (ScoreDoc scoreDoc : collector.pq)`: the port's queue has no
            // iterator, so its entries are popped; the merged queue's result
            // does not depend on the insertion order, since `HitQueue` orders
            // by score then doc.
            while let Some(score_doc) = collector.pq.pop() {
                my_top_docs_collector.pq.insert_with_overflow(score_doc);
            }
        }
        my_top_docs_collector.total_hits = total_hits;
        Ok(my_top_docs_collector)
    }
}

/// `private static final class MyTopDocsCollector extends TopDocsCollector<ScoreDoc>`.
struct MyTopDocsCollector {
    pq: HitQueue,
    total_hits: i32,
    total_hits_relation: TotalHitsRelation,
}

impl MyTopDocsCollector {
    fn new(size: i32) -> Result<Self> {
        Ok(MyTopDocsCollector {
            pq: HitQueue::new(size as usize, false)?,
            total_hits: 0,
            total_hits_relation: TotalHitsRelation::EQUAL_TO,
        })
    }
}

/// The anonymous `LeafCollector` of `MyTopDocsCollector.getLeafCollector`.
struct MyLeafCollector<'a> {
    parent: &'a mut MyTopDocsCollector,
    base: i32,
    doc_base: i32,
}

impl LeafCollector for MyLeafCollector<'_> {
    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.parent.total_hits += 1;
        self.parent.pq.insert_with_overflow(ScoreDoc::new(
            doc + self.base,
            SCORES[(self.doc_base + doc) as usize],
        ));
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        // Don't do anything. Assign scores in random
        Ok(())
    }
}

impl Collector for MyTopDocsCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let base = context.doc_base();
        Ok(Box::new(MyLeafCollector {
            parent: self,
            base,
            doc_base: context.doc_base(),
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

impl TopDocsCollector for MyTopDocsCollector {
    type Hit = ScoreDoc;
    type Docs = TopDocs;

    fn total_hits(&self) -> i32 {
        self.total_hits
    }

    fn total_hits_relation(&self) -> TotalHitsRelation {
        self.total_hits_relation
    }

    fn pq_size(&self) -> usize {
        self.pq.size()
    }

    fn pop(&mut self) -> Option<ScoreDoc> {
        self.pq.pop()
    }

    fn new_top_docs(&self, results: Option<Vec<ScoreDoc>>, _start: i32) -> Result<TopDocs> {
        let Some(results) = results else {
            return Ok(empty_top_docs());
        };

        Ok(TopDocs::new(
            TotalHits::new(i64::from(self.total_hits), self.total_hits_relation)?,
            results,
        ))
    }
}

// Scores array to be used by MyTopDocsCollector. If it is changed, MAX_SCORE
// must also change.
const SCORES: [f32; 30] = [
    0.7767749, 1.7839992, 8.9925785, 7.9608946, 0.07948637, 2.6356435, 7.4950366, 7.1490803,
    8.108544, 4.961808, 2.2423935, 7.285586, 4.6699767, 2.9655676, 6.953706, 5.383931, 6.9916306,
    8.365894, 7.888485, 8.723962, 3.1796896, 0.39971232, 1.3077754, 6.8489285, 9.17561, 5.060466,
    7.9793315, 8.601509, 4.1858315, 0.28146625,
];

const MAX_SCORE: f32 = 9.17561;

/// `private TopDocsCollector<ScoreDoc> doSearch(int numResults)`.
fn do_search(f: &TopDocsCollectorTest, num_results: i32) -> Result<MyTopDocsCollector> {
    let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let searcher = LuceneTestCase::new_searcher(Arc::clone(f.reader.as_ref().expect("setUp ran")));
    searcher.search_with_manager(
        q,
        &MyTopDocsCollectorMananger {
            num_hits: num_results,
        },
    )
}

/// `private TopDocs doSearchWithThreshold(int numResults, int thresHold, Query q, IndexReader indexReader)`.
fn do_search_with_threshold(
    num_results: i32,
    thres_hold: i32,
    q: &Arc<dyn Query>,
    index_reader: &Arc<dyn DirectoryReader>,
) -> Result<TopDocs> {
    let searcher =
        LuceneTestCase::new_searcher_threads(Arc::clone(index_reader), true, true, false);
    let collector_manager = TopScoreDocCollectorManager::new(num_results, None, thres_hold)?;
    searcher.search_with_manager(Arc::clone(q), &collector_manager)
}

/// `private static TopDocs doConcurrentSearchWithThreshold(int numResults, int threshold, Query q, IndexReader indexReader)`.
fn do_concurrent_search_with_threshold(
    num_results: i32,
    threshold: i32,
    q: &Arc<dyn Query>,
    index_reader: &Arc<dyn DirectoryReader>,
) -> Result<TopDocs> {
    let searcher = LuceneTestCase::new_searcher_threads(Arc::clone(index_reader), true, true, true);
    let collector_manager = TopScoreDocCollectorManager::new(num_results, None, threshold)?;
    searcher.search_with_manager(Arc::clone(q), &collector_manager)
}

/// `setUp()`.
fn top_docs_collector_set_up(f: &mut TopDocsCollectorTest) -> Result<()> {
    // populate an index with 30 documents, this should be enough for the test.
    // The documents have no content - the test uses MatchAllDocsQuery().
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for _i in 0..30 {
        writer.add_document(&Document::new())?;
    }
    f.reader = Some(writer.get_reader()?);
    writer.close()?;
    f.dir = Some(dir);
    Ok(())
}

/// `tearDown()`.
fn top_docs_collector_tear_down(f: &mut TopDocsCollectorTest) -> Result<()> {
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestTopDocsCollector.testInvalidArguments`.
fn test_invalid_arguments(f: &mut TopDocsCollectorTest) -> Result<()> {
    let num_results = 5;
    let mut tdc = do_search(f, num_results)?;

    // start < 0
    let exception = expect_throws(IAE, || tdc.top_docs_from(-1));

    assert_equals(
        &Some("Expected value of starting position is between 0 and 5, got -1"),
        &exception.message(),
    );

    // start == pq.size()
    assert_equals(&0, &tdc.top_docs_from(num_results)?.score_docs.len());

    // howMany < 0
    let exception = expect_throws(IAE, || tdc.top_docs_range(0, -1));

    assert_equals(
        &Some("Number of hits requested must be greater than 0 but value was -1"),
        &exception.message(),
    );
    Ok(())
}

/// Port of `TestTopDocsCollector.testZeroResults`.
fn test_zero_results(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut tdc = MyTopDocsCollector::new(5)?;
    assert_equals(&0, &tdc.top_docs_range(0, 1)?.score_docs.len());
    Ok(())
}

/// Port of `TestTopDocsCollector.testFirstResultsPage`.
fn test_first_results_page(f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut tdc = do_search(f, 15)?;
    assert_equals(&10, &tdc.top_docs_range(0, 10)?.score_docs.len());
    Ok(())
}

/// Port of `TestTopDocsCollector.testSecondResultsPages`.
fn test_second_results_pages(f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut tdc = do_search(f, 15)?;
    // ask for more results than are available
    assert_equals(&5, &tdc.top_docs_range(10, 10)?.score_docs.len());

    // ask for 5 results (exactly what there should be
    let mut tdc = do_search(f, 15)?;
    assert_equals(&5, &tdc.top_docs_range(10, 5)?.score_docs.len());

    // ask for less results than there are
    let mut tdc = do_search(f, 15)?;
    assert_equals(&4, &tdc.top_docs_range(10, 4)?.score_docs.len());
    Ok(())
}

/// Port of `TestTopDocsCollector.testGetAllResults`.
fn test_get_all_results(f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut tdc = do_search(f, 15)?;
    assert_equals(&15, &tdc.top_docs()?.score_docs.len());
    Ok(())
}

/// Port of `TestTopDocsCollector.testGetResultsFromStart`.
fn test_get_results_from_start(f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut tdc = do_search(f, 15)?;
    // should bring all results
    assert_equals(&15, &tdc.top_docs_from(0)?.score_docs.len());

    let mut tdc = do_search(f, 15)?;
    // get the last 5 only.
    assert_equals(&5, &tdc.top_docs_from(10)?.score_docs.len());
    Ok(())
}

/// Port of `TestTopDocsCollector.testIllegalArguments`.
fn test_illegal_arguments(f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut tdc = do_search(f, 15)?;

    let expected = expect_throws(IAE, || tdc.top_docs_from(-1));

    assert_equals(
        &Some("Expected value of starting position is between 0 and 15, got -1"),
        &expected.message(),
    );

    let expected = expect_throws(IAE, || tdc.top_docs_range(9, -1));

    assert_equals(
        &Some("Number of hits requested must be greater than 0 but value was -1"),
        &expected.message(),
    );
    Ok(())
}

/// This does not test the PQ's correctness, but whether topDocs()
/// implementations return the results in decreasing score order.
///
/// Port of `TestTopDocsCollector.testResultsOrder`.
fn test_results_order(f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut tdc = do_search(f, 15)?;
    let sd = tdc.top_docs()?.score_docs;

    assert_equals_delta(f64::from(MAX_SCORE), f64::from(sd[0].score), 0.0);
    for i in 1..sd.len() {
        Assert::assert_true(sd[i - 1].score >= sd[i].score);
    }
    Ok(())
}

/// `private static class Score extends Scorable`.
#[derive(Debug, Default)]
struct Score {
    score: f32,
    min_competitive_score: Option<f32>,
}

impl Scorable for Score {
    fn set_min_competitive_score(&mut self, score: f32) -> Result<()> {
        debug_assert!(
            self.min_competitive_score.is_none()
                || score >= self.min_competitive_score.expect("checked")
        );
        self.min_competitive_score = Some(score);
        Ok(())
    }

    fn score(&mut self) -> Result<f32> {
        Ok(self.score)
    }
}

/// `scorer.minCompetitiveScore` unboxed, as `assertEquals(float, Float,
/// float)` does: `null` throws `NullPointerException`.
fn min_score(scorer: &Score) -> f32 {
    scorer
        .min_competitive_score
        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
}

/// `Math.nextUp(f)`.
fn next_up(f: f32) -> f32 {
    f.next_up()
}

/// `assertEquals(expected, actual, 0f)`.
fn assert_float(expected: f32, actual: f32) {
    assert_equals_delta(f64::from(expected), f64::from(actual), 0.0);
}

/// `leafCollector.collect(doc)` with the scorer Java set on it.
fn collect(leaf: &mut dyn LeafCollector, doc: i32, scorer: &mut Score) -> Result<()> {
    leaf.collect(doc, scorer)
        .map_err(CollectionError::into_lucene_error)
}

/// `w.addDocuments(Arrays.asList(doc × n))`.
fn add_empty_docs(w: &IndexWriter, n: usize) -> Result<()> {
    let docs: Vec<Document> = (0..n).map(|_| Document::new()).collect();
    w.add_documents(&docs)?;
    Ok(())
}

/// `new IndexWriter(dir, newIndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE))`.
fn no_merge_writer(dir: &TestDirectory) -> Result<Arc<IndexWriter>> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    IndexWriter::new(dir.as_directory(), config)
}

/// `reader.leaves()`.
fn leaves(reader: &Arc<dyn DirectoryReader>) -> Result<Vec<Arc<LeafReaderContext>>> {
    (Arc::clone(reader) as Arc<dyn IndexReader>).leaves()
}

/// Port of `TestTopDocsCollector.testSetMinCompetitiveScore`.
fn test_set_min_competitive_score(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = no_merge_writer(&dir)?;
    add_empty_docs(&w, 4)?;
    w.flush()?;
    add_empty_docs(&w, 2)?;
    w.flush()?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let leaves = leaves(&reader)?;
    assert_equals(&2, &leaves.len());
    w.close()?;

    let collector_manager = TopScoreDocCollectorManager::with_num_hits(2, 2)?;
    let mut collector = collector_manager.new_collector()?;
    let mut scorer = Score::default();

    {
        let mut leaf_collector = collector
            .get_leaf_collector(&leaves[0])
            .map_err(CollectionError::into_lucene_error)?;
        leaf_collector.set_scorer(&mut scorer)?;
        assert_null(&scorer.min_competitive_score);

        scorer.score = 1.0;
        collect(&mut *leaf_collector, 0, &mut scorer)?;
        assert_null(&scorer.min_competitive_score);

        scorer.score = 2.0;
        collect(&mut *leaf_collector, 1, &mut scorer)?;
        assert_null(&scorer.min_competitive_score);

        scorer.score = 3.0;
        collect(&mut *leaf_collector, 2, &mut scorer)?;
        assert_float(next_up(2f32), min_score(&scorer));

        scorer.score = 0.5;
        // Make sure we do not call setMinCompetitiveScore for non-competitive hits
        scorer.min_competitive_score = None;
        collect(&mut *leaf_collector, 3, &mut scorer)?;
        assert_null(&scorer.min_competitive_score);

        scorer.score = 4.0;
        collect(&mut *leaf_collector, 4, &mut scorer)?;
        assert_float(next_up(3f32), min_score(&scorer));
    }

    // Make sure the min score is set on scorers on new segments
    let mut scorer = Score::default();
    {
        let mut leaf_collector = collector
            .get_leaf_collector(&leaves[1])
            .map_err(CollectionError::into_lucene_error)?;
        leaf_collector.set_scorer(&mut scorer)?;
        assert_float(next_up(3f32), min_score(&scorer));

        scorer.score = 1.0;
        collect(&mut *leaf_collector, 0, &mut scorer)?;
        assert_float(next_up(3f32), min_score(&scorer));

        scorer.score = 4.0;
        collect(&mut *leaf_collector, 1, &mut scorer)?;
        assert_float(next_up(4f32), min_score(&scorer));
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestTopDocsCollector.testSharedCountCollectorManager`.
fn test_shared_count_collector_manager(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let mut dir = LuceneTestCase::new_directory();
    let w = no_merge_writer(&dir)?;
    add_empty_docs(&w, 4)?;
    w.flush()?;
    add_empty_docs(&w, 2)?;
    w.flush()?;
    let reader = directory_reader::open_with_writer(&*w)?;
    assert_equals(&2, &leaves(&reader)?.len());
    w.close()?;

    let tdc = do_concurrent_search_with_threshold(5, 10, &q, &reader)?;
    let tdc2 = do_search_with_threshold(5, 10, &q, &reader)?;

    CheckHits::check_equal(&*q, &tdc.score_docs, &tdc2.score_docs);

    reader.close()?;
    dir.close()
}

/// Port of `TestTopDocsCollector.testTotalHits`.
fn test_total_hits(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = no_merge_writer(&dir)?;
    add_empty_docs(&w, 4)?;
    w.flush()?;
    add_empty_docs(&w, 6)?;
    w.flush()?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let leaves = leaves(&reader)?;
    assert_equals(&2, &leaves.len());
    w.close()?;

    for total_hits_threshold in 0..20 {
        let collector_manager =
            TopScoreDocCollectorManager::with_num_hits(2, total_hits_threshold)?;
        let mut collector = collector_manager.new_collector()?;
        let mut scorer = Score::default();

        {
            let mut leaf_collector = collector
                .get_leaf_collector(&leaves[0])
                .map_err(CollectionError::into_lucene_error)?;
            leaf_collector.set_scorer(&mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector, 0, &mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector, 1, &mut scorer)?;
        }

        {
            let mut leaf_collector = collector
                .get_leaf_collector(&leaves[1])
                .map_err(CollectionError::into_lucene_error)?;
            leaf_collector.set_scorer(&mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector, 1, &mut scorer)?;

            scorer.score = 4.0;
            collect(&mut *leaf_collector, 1, &mut scorer)?;
        }

        let top_docs = collector.top_docs()?;
        assert_equals(&4, &top_docs.total_hits.value());
        assert_equals(
            &(total_hits_threshold < 4),
            &scorer.min_competitive_score.is_some(),
        );
        assert_equals(
            &TotalHits::new(
                4,
                if total_hits_threshold < 4 {
                    TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO
                } else {
                    TotalHitsRelation::EQUAL_TO
                },
            )?,
            &top_docs.total_hits,
        );
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestTopDocsCollector.testRelationVsTopDocsCount`.
fn test_relation_vs_top_docs_count(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let w = no_merge_writer(&dir)?;
        let result = (|| -> Result<()> {
            let mut doc = Document::new();
            doc.add(Box::new(TextField::new(
                "f",
                "foo bar".to_string(),
                Store::NO,
            )?));
            w.add_documents(&[&doc, &doc, &doc, &doc, &doc])?;
            w.flush()?;
            w.add_documents(&[&doc, &doc, &doc, &doc, &doc])?;
            w.flush()?;

            let reader = directory_reader::open_with_writer(&*w)?;
            let result = (|| -> Result<()> {
                let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
                let foo =
                    || -> Arc<dyn Query> { Arc::new(TermQuery::new(Term::from_text("f", "foo"))) };
                let collector_manager = TopScoreDocCollectorManager::with_num_hits(2, 10)?;
                let top_docs = searcher.search_with_manager(foo(), &collector_manager)?;
                assert_equals(&10, &top_docs.total_hits.value());
                assert_equals(
                    &TotalHitsRelation::EQUAL_TO,
                    &top_docs.total_hits.relation(),
                );

                let collector_manager = TopScoreDocCollectorManager::with_num_hits(2, 2)?;
                let top_docs = searcher.search_with_manager(foo(), &collector_manager)?;
                Assert::assert_true(10 >= top_docs.total_hits.value());
                assert_equals(
                    &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                    &top_docs.total_hits.relation(),
                );

                let collector_manager = TopScoreDocCollectorManager::with_num_hits(10, 2)?;
                let top_docs = searcher.search_with_manager(foo(), &collector_manager)?;
                assert_equals(&10, &top_docs.total_hits.value());
                assert_equals(
                    &TotalHitsRelation::EQUAL_TO,
                    &top_docs.total_hits.relation(),
                );
                Ok(())
            })();
            result.and(reader.close())
        })();
        result.and(w.close())
    })();
    result.and(dir.close())
}

/// Port of `TestTopDocsCollector.testConcurrentMinScore`.
fn test_concurrent_min_score(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = no_merge_writer(&dir)?;
    add_empty_docs(&w, 5)?;
    w.flush()?;
    add_empty_docs(&w, 6)?;
    w.flush()?;
    add_empty_docs(&w, 2)?;
    w.flush()?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let leaves = leaves(&reader)?;
    assert_equals(&3, &leaves.len());
    w.close()?;

    let manager = TopScoreDocCollectorManager::with_num_hits(2, 0)?;
    let mut collector = manager.new_collector()?;
    let mut collector2 = manager.new_collector()?;
    Assert::assert_true(Arc::ptr_eq(
        collector.min_score_acc().expect("minScoreAcc"),
        collector2.min_score_acc().expect("minScoreAcc"),
    ));
    let min_value_checker = Arc::clone(collector.min_score_acc().expect("minScoreAcc"));
    // force the check of the global minimum score on every round
    min_value_checker.set_mod_interval(0);

    let mut scorer = Score::default();
    let mut scorer2 = Score::default();
    let mut collector3 = manager.new_collector()?;
    let mut scorer3 = Score::default();

    {
        let mut leaf_collector = collector
            .get_leaf_collector(&leaves[0])
            .map_err(CollectionError::into_lucene_error)?;
        leaf_collector.set_scorer(&mut scorer)?;
        let mut leaf_collector2 = collector2
            .get_leaf_collector(&leaves[1])
            .map_err(CollectionError::into_lucene_error)?;
        leaf_collector2.set_scorer(&mut scorer2)?;

        scorer.score = 3.0;
        collect(&mut *leaf_collector, 0, &mut scorer)?;
        assert_equals(&i64::MIN, &min_value_checker.get_raw());
        assert_null(&scorer.min_competitive_score);

        scorer2.score = 6.0;
        collect(&mut *leaf_collector2, 0, &mut scorer2)?;
        assert_equals(&i64::MIN, &min_value_checker.get_raw());
        assert_null(&scorer2.min_competitive_score);

        scorer.score = 2.0;
        collect(&mut *leaf_collector, 1, &mut scorer)?;
        assert_equals(&i64::MIN, &min_value_checker.get_raw());
        assert_null(&scorer.min_competitive_score);

        scorer2.score = 9.0;
        collect(&mut *leaf_collector2, 1, &mut scorer2)?;
        assert_equals(&i64::MIN, &min_value_checker.get_raw());
        assert_null(&scorer2.min_competitive_score);

        scorer2.score = 7.0;
        collect(&mut *leaf_collector2, 2, &mut scorer2)?;
        assert_float(DocScoreEncoder::to_score(min_value_checker.get_raw()), 7f32);
        assert_null(&scorer.min_competitive_score);
        assert_float(next_up(7f32), min_score(&scorer2));

        scorer2.score = 1.0;
        collect(&mut *leaf_collector2, 3, &mut scorer2)?;
        assert_float(DocScoreEncoder::to_score(min_value_checker.get_raw()), 7f32);
        assert_null(&scorer.min_competitive_score);
        assert_float(next_up(7f32), min_score(&scorer2));

        scorer.score = 10.0;
        collect(&mut *leaf_collector, 2, &mut scorer)?;
        assert_float(DocScoreEncoder::to_score(min_value_checker.get_raw()), 7f32);
        assert_float(7f32, min_score(&scorer));
        assert_float(next_up(7f32), min_score(&scorer2));

        scorer.score = 11.0;
        collect(&mut *leaf_collector, 3, &mut scorer)?;
        assert_float(DocScoreEncoder::to_score(min_value_checker.get_raw()), 10.0);
        assert_float(next_up(10f32), min_score(&scorer));
        assert_float(next_up(7f32), min_score(&scorer2));

        let mut leaf_collector3 = collector3
            .get_leaf_collector(&leaves[2])
            .map_err(CollectionError::into_lucene_error)?;
        leaf_collector3.set_scorer(&mut scorer3)?;
        assert_float(next_up(10f32), min_score(&scorer3));

        scorer3.score = 1f32;
        collect(&mut *leaf_collector3, 0, &mut scorer3)?;
        assert_float(
            10f32,
            DocScoreEncoder::to_score(min_value_checker.get_raw()),
        );
        assert_float(next_up(10f32), min_score(&scorer3));

        scorer.score = 11.0;
        collect(&mut *leaf_collector, 4, &mut scorer)?;
        assert_float(
            11f32,
            DocScoreEncoder::to_score(min_value_checker.get_raw()),
        );
        assert_float(next_up(11f32), min_score(&scorer));
        assert_float(next_up(7f32), min_score(&scorer2));
        assert_float(next_up(10f32), min_score(&scorer3));

        scorer3.score = 2f32;
        collect(&mut *leaf_collector3, 1, &mut scorer3)?;
        assert_float(
            DocScoreEncoder::to_score(min_value_checker.get_raw()),
            11f32,
        );
        assert_float(next_up(11f32), min_score(&scorer));
        assert_float(next_up(7f32), min_score(&scorer2));
        assert_float(next_up(11f32), min_score(&scorer3));

        // Java calls these three `setScorer` after `manager.reduce`; the leaf
        // collectors borrow their collectors, which `reduce` consumes, so they
        // run first. `reduce` reads only each collector's queue and hit
        // count, which `setScorer` does not change, and nothing is asserted
        // after them.
        leaf_collector.set_scorer(&mut scorer)?;
        leaf_collector2.set_scorer(&mut scorer2)?;
        leaf_collector3.set_scorer(&mut scorer3)?;
    }

    let top_docs = manager.reduce(vec![collector, collector2, collector3])?;
    assert_equals(&11, &top_docs.total_hits.value());
    assert_equals(
        &TotalHits::new(11, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
        &top_docs.total_hits,
    );

    reader.close()?;
    dir.close()
}

/// Port of `TestTopDocsCollector.testRandomMinCompetitiveScore`.
fn test_random_min_competitive_score(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_docs = LuceneTestCase::at_least(1000);
    for _i in 0..num_docs {
        let num_as = 1 + random().next_int_bound(5);
        let num_bs = if random().next_float() < 0.5f32 {
            0
        } else {
            1 + random().next_int_bound(5)
        };
        let num_cs = if random().next_float() < 0.1f32 {
            0
        } else {
            1 + random().next_int_bound(5)
        };
        let mut doc = Document::new();
        for _j in 0..num_as {
            doc.add(Box::new(StringField::new("f", "A".to_string(), Store::NO)?));
        }
        for _j in 0..num_bs {
            doc.add(Box::new(StringField::new("f", "B".to_string(), Store::NO)?));
        }
        for _j in 0..num_cs {
            doc.add(Box::new(StringField::new("f", "C".to_string(), Store::NO)?));
        }
        w.add_document(&doc)?;
    }
    let index_reader = w.get_reader()?;
    w.close()?;
    let tq =
        |text: &str| -> Arc<dyn Query> { Arc::new(TermQuery::new(Term::from_text("f", text))) };
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("A"), Occur::MUST)?;
    bq.add(tq("B"), Occur::SHOULD)?;
    let queries: Vec<Arc<dyn Query>> = vec![tq("A"), tq("B"), tq("C"), Arc::new(bq.build())];
    for query in &queries {
        let tdc = do_concurrent_search_with_threshold(5, 0, query, &index_reader)?;
        let tdc2 = do_search_with_threshold(5, 0, query, &index_reader)?;

        Assert::assert_true(tdc.total_hits.value() > 0);
        Assert::assert_true(tdc2.total_hits.value() > 0);
        CheckHits::check_equal(&**query, &tdc.score_docs, &tdc2.score_docs);
    }

    index_reader.close()?;
    dir.close()
}

/// Port of `TestTopDocsCollector.testRealisticConcurrentMinimumScore`.
fn test_realistic_concurrent_minimum_score(_f: &mut TopDocsCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    {
        let docs = LineFileDocs::with_default_file(&*random())?;
        let result = (|| -> Result<()> {
            let num_docs = LuceneTestCase::at_least(100);
            for _i in 0..num_docs {
                writer.add_document(&*docs.next_doc()?.borrow())?;
            }
            Ok(())
        })();
        docs.close();
        result?;
    }

    let reader = writer.get_reader()?;
    writer.close()?;

    let terms = MultiTerms::get_terms(&(Arc::clone(&reader) as Arc<dyn IndexReader>), "body")?
        .expect("terms");
    let mut term_count = 0;
    let mut terms_enum = terms.iterator()?;
    while terms_enum.next()?.is_some() {
        term_count += 1;
    }
    Assert::assert_true(term_count > 0);

    // Target ~10 terms to search:
    let chance = 10.0 / f64::from(term_count);
    let mut terms_enum = terms.iterator()?;
    while terms_enum.next()?.is_some() {
        if random().next_double() <= chance {
            let term = BytesRef::deep_copy_of(
                &terms_enum
                    .term()?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?,
            );
            let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new("body", term)));

            let tdc = do_concurrent_search_with_threshold(5, 0, &query, &reader)?;
            let tdc2 = do_search_with_threshold(5, 0, &query, &reader)?;

            CheckHits::check_equal(&*query, &tdc.score_docs, &tdc2.score_docs);
        }
    }

    reader.close()?;
    dir.close()
}
