//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTopFieldCollector.java`.
//!
//! Java changes a field's value after `doc.add` and adds the document again;
//! the port's `Document` gives no mutable access to its fields, so each
//! `addDocument` gets a document rebuilt with the field's current value (no
//! random draw is involved).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store, StringField, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{IndexReader, LeafReaderContext, Term};
use rucene::search::field_comparator::SortValue;
use rucene::search::field_doc::FieldDoc;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::{ChildScorable, Scorable};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::top_field_collector::TopFieldCollector;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::{
    BooleanQueryBuilder, BoostQuery, CollectionError, CollectionResult, Collector,
    CollectorManager, DocScoreEncoder, LeafCollector, MatchAllDocsQuery, Occur, Query, ScoreDoc,
    ScoreMode, TopDocsCollector, TopFieldDocs,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, assert_equals_delta, assert_null, random};

/// The instance of `TestTopFieldCollector`.
#[derive(Default)]
pub struct TopFieldCollectorTest {
    is: Option<IndexSearcher>,
    ir: Option<Arc<dyn DirectoryReader>>,
    dir: Option<TestDirectory>,
}

impl TopFieldCollectorTest {
    fn is(&self) -> &IndexSearcher {
        self.is.as_ref().expect("setUp ran")
    }

    fn ir(&self) -> &Arc<dyn DirectoryReader> {
        self.ir.as_ref().expect("setUp ran")
    }
}

/// `TestTopFieldCollector`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<TopFieldCollectorTest>(
        "org.apache.lucene.search.TestTopFieldCollector",
        vec![
            set_up(top_field_collector_set_up),
            tear_down(top_field_collector_tear_down),
            ftest("testSortWithoutFillFields", test_sort_without_fill_fields),
            ftest("testSort", test_sort),
            ftest(
                "testSharedHitcountCollector",
                test_shared_hitcount_collector,
            ),
            ftest(
                "testSortWithoutTotalHitTracking",
                test_sort_without_total_hit_tracking,
            ),
            ftest("testTotalHits", test_total_hits),
            ftest("testSetMinCompetitiveScore", test_set_min_competitive_score),
            ftest("testTotalHitsWithScore", test_total_hits_with_score),
            ftest("testSortNoResults", test_sort_no_results),
            ftest("testComputeScoresOnlyOnce", test_compute_scores_only_once),
            ftest("testPopulateScores", test_populate_scores),
            ftest("testConcurrentMinScore", test_concurrent_min_score),
            ftest(
                "testRandomMinCompetitiveScore",
                test_random_min_competitive_score,
            ),
            ftest(
                "testRelationVsTopDocsCount",
                test_relation_vs_top_docs_count,
            ),
        ],
    )
}

/// `setUp()`.
fn top_field_collector_set_up(f: &mut TopFieldCollectorTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(100);
    for _i in 0..num_docs {
        let doc = Document::new();
        iw.add_document(&doc)?;
    }
    let ir = iw.get_reader()?;
    iw.close()?;
    // cannot use threads with this IndexSearcher since some tests rely on no threads
    f.is = Some(LuceneTestCase::new_searcher_threads(
        Arc::clone(&ir),
        true,
        true,
        false,
    ));
    f.ir = Some(ir);
    f.dir = Some(dir);
    Ok(())
}

/// `tearDown()`.
fn top_field_collector_tear_down(f: &mut TopFieldCollectorTest) -> Result<()> {
    f.is = None;
    if let Some(ir) = f.ir.take() {
        ir.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// `private static TopDocs doSearchWithThreshold(int numResults, int thresHold, Query q, Sort sort, IndexReader indexReader)`.
fn do_search_with_threshold(
    num_results: i32,
    thres_hold: i32,
    q: &Arc<dyn Query>,
    sort: &Sort,
    index_reader: &Arc<dyn DirectoryReader>,
) -> Result<TopFieldDocs> {
    let searcher = LuceneTestCase::new_searcher(Arc::clone(index_reader));
    let manager = TopFieldCollectorManager::new(sort.clone(), num_results, None, thres_hold)?;
    searcher.search_with_manager(Arc::clone(q), &manager)
}

/// `private static TopDocs doConcurrentSearchWithThreshold(int numResults, int threshold, Query q, Sort sort, IndexReader indexReader)`.
fn do_concurrent_search_with_threshold(
    num_results: i32,
    threshold: i32,
    q: &Arc<dyn Query>,
    sort: &Sort,
    index_reader: &Arc<dyn DirectoryReader>,
) -> Result<TopFieldDocs> {
    let searcher = LuceneTestCase::new_searcher_threads(Arc::clone(index_reader), true, true, true);

    let collector_manager =
        TopFieldCollectorManager::new(sort.clone(), num_results, None, threshold)?;

    let top_doc = searcher.search_with_manager(Arc::clone(q), &collector_manager)?;

    Ok(top_doc)
}

/// `new Sort[] {new Sort(SortField.FIELD_DOC), new Sort()}`.
fn two_sorts() -> Result<Vec<Sort>> {
    Ok(vec![
        Sort::new_fields(vec![SortField::FIELD_DOC])?,
        Sort::new(),
    ])
}

/// `TopFieldDocs.scoreDocs` as the `ScoreDoc[]` `CheckHits.checkEqual` reads.
fn score_docs(top_docs: &TopFieldDocs) -> Vec<ScoreDoc> {
    top_docs.score_docs.iter().map(|fd| fd.score_doc).collect()
}

/// Port of `TestTopFieldCollector.testSortWithoutFillFields`.
fn test_sort_without_fill_fields(f: &mut TopFieldCollectorTest) -> Result<()> {
    // There was previously a bug in TopFieldCollector when fillFields was set
    // to false - the same doc and score was set in ScoreDoc[] array. This test
    // asserts that if fillFields is false, the documents are set properly. It
    // does not use Searcher's default search methods (with Sort) since all set
    // fillFields to true.
    let sort = two_sorts()?;
    for s in &sort {
        let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
        let collector_manager = TopFieldCollectorManager::with_num_hits(s.clone(), 10, i32::MAX)?;

        let sd = f
            .is()
            .search_with_manager(q, &collector_manager)?
            .score_docs;
        for j in 1..sd.len() {
            Assert::assert_true(sd[j].doc() != sd[j - 1].doc());
        }
    }
    Ok(())
}

/// Port of `TestTopFieldCollector.testSort`.
fn test_sort(f: &mut TopFieldCollectorTest) -> Result<()> {
    // Two Sort criteria to instantiate the multi/single comparators.
    let sort = two_sorts()?;
    for s in &sort {
        let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
        let tdc = TopFieldCollectorManager::new(s.clone(), 10, None, i32::MAX)?;
        let td = f.is().search_with_manager(q, &tdc)?;
        let sd = &td.score_docs;
        for j in 0..sd.len() {
            Assert::assert_true(sd[j].score().is_nan());
        }
    }
    Ok(())
}

/// Port of `TestTopFieldCollector.testSharedHitcountCollector`.
fn test_shared_hitcount_collector(f: &mut TopFieldCollectorTest) -> Result<()> {
    let concurrent_searcher =
        LuceneTestCase::new_searcher_threads(Arc::clone(f.ir()), true, true, true);
    let single_threaded_searcher =
        LuceneTestCase::new_searcher_threads(Arc::clone(f.ir()), true, true, false);

    // Two Sort criteria to instantiate the multi/single comparators.
    let sort = two_sorts()?;
    for s in &sort {
        let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
        let tdc = TopFieldCollectorManager::with_num_hits(s.clone(), 10, i32::MAX)?;
        let td = single_threaded_searcher.search_with_manager(Arc::clone(&q), &tdc)?;

        let tsdc = TopFieldCollectorManager::with_num_hits(s.clone(), 10, i32::MAX)?;
        let td2 = concurrent_searcher.search_with_manager(Arc::clone(&q), &tsdc)?;

        let sd = &td.score_docs;
        for j in 0..sd.len() {
            Assert::assert_true(sd[j].score().is_nan());
        }

        CheckHits::check_equal(&*q, &score_docs(&td), &score_docs(&td2));
    }
    Ok(())
}

/// Port of `TestTopFieldCollector.testSortWithoutTotalHitTracking`.
fn test_sort_without_total_hit_tracking(f: &mut TopFieldCollectorTest) -> Result<()> {
    let sort = Sort::new_fields(vec![SortField::FIELD_DOC])?;
    for i in 0..2 {
        let q: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
        // check that setting trackTotalHits to false does not throw an NPE because
        // the index is not sorted
        let manager = if i % 2 == 0 {
            TopFieldCollectorManager::with_num_hits(sort.clone(), 10, 1)?
        } else {
            let field_doc = FieldDoc::with_fields(1, f32::NAN, vec![SortValue::Int(1)]);
            TopFieldCollectorManager::new(sort.clone(), 10, Some(field_doc), 1)?
        };

        let td = f.is().search_with_manager(q, &manager)?;
        let sd = &td.score_docs;
        for j in 0..sd.len() {
            Assert::assert_true(sd[j].score().is_nan());
        }
    }
    Ok(())
}

/// `expectThrows(CollectionTerminatedException.class, () -> leafCollector.collect(doc))`.
fn expect_collection_terminated(result: CollectionResult<()>) {
    match result {
        Err(CollectionError::CollectionTerminated) => {}
        Err(e) => rethrow_java(e.into_lucene_error()),
        Ok(()) => Assert::fail(
            "Expected exception CollectionTerminatedException but no exception was thrown",
        ),
    }
}

/// `leafCollector.collect(doc)` with the scorer Java set on it.
fn collect(leaf: &mut dyn LeafCollector, doc: i32, scorer: &mut Score) -> Result<()> {
    leaf.collect(doc, scorer)
        .map_err(CollectionError::into_lucene_error)
}

/// `collector.getLeafCollector(context)`.
fn leaf_of<'a>(
    collector: &'a mut TopFieldCollector,
    context: &LeafReaderContext,
) -> Result<Box<dyn LeafCollector + 'a>> {
    collector
        .get_leaf_collector(context)
        .map_err(CollectionError::into_lucene_error)
}

/// `reader.leaves()`.
fn leaves(reader: &Arc<dyn DirectoryReader>) -> Result<Vec<Arc<LeafReaderContext>>> {
    (Arc::clone(reader) as Arc<dyn IndexReader>).leaves()
}

/// Port of `TestTopFieldCollector.testTotalHits`.
fn test_total_hits(_f: &mut TopFieldCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let sort = Sort::new_fields(vec![SortField::new(
        Some("foo".to_string()),
        SortFieldType::Long,
    )?])?;
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    config.set_index_sort(sort.clone())?;
    config.set_max_buffered_docs(7)?;
    config.set_ram_buffer_size_mb(f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH))?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let doc = || -> Document {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("foo", 3)));
        doc
    };
    for _d in 0..4 {
        w.add_document(&doc())?;
    }
    w.flush()?;
    for _d in 0..6 {
        w.add_document(&doc())?;
    }
    w.flush()?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let leaves = leaves(&reader)?;
    assert_equals(&2, &leaves.len());
    w.close()?;

    for total_hits_threshold in 0..20 {
        for after in [
            None,
            Some(FieldDoc::with_fields(4, f32::NAN, vec![SortValue::Long(2)])),
        ] {
            let mut collector =
                TopFieldCollectorManager::new(sort.clone(), 2, after, total_hits_threshold)?
                    .new_collector()?;
            let mut scorer = Score::default();

            let mut leaf_collector1 = leaf_of(&mut collector, &leaves[0])?;
            leaf_collector1.set_scorer(&mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector1, 0, &mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector1, 1, &mut scorer)?;
            drop(leaf_collector1);

            let mut leaf_collector2 = leaf_of(&mut collector, &leaves[1])?;
            leaf_collector2.set_scorer(&mut scorer)?;

            scorer.score = 3.0;
            if total_hits_threshold < 3 {
                expect_collection_terminated(leaf_collector2.collect(1, &mut scorer));
                drop(leaf_collector2);
                let top_docs = collector.top_docs()?;
                assert_equals(
                    &TotalHits::new(3, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
                    &top_docs.total_hits,
                );
                continue;
            } else {
                collect(&mut *leaf_collector2, 1, &mut scorer)?;
            }

            scorer.score = 4.0;
            if total_hits_threshold == 3 {
                expect_collection_terminated(leaf_collector2.collect(1, &mut scorer));
                drop(leaf_collector2);
                let top_docs = collector.top_docs()?;
                assert_equals(
                    &TotalHits::new(4, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?,
                    &top_docs.total_hits,
                );
                continue;
            } else {
                collect(&mut *leaf_collector2, 1, &mut scorer)?;
            }
            drop(leaf_collector2);

            let top_docs = collector.top_docs()?;
            assert_equals(
                &TotalHits::new(4, TotalHitsRelation::EQUAL_TO)?,
                &top_docs.total_hits,
            );
        }
    }

    reader.close()?;
    dir.close()
}

/// `private static class Score extends Scorable`.
#[derive(Debug, Default)]
struct Score {
    score: f32,
    min_competitive_score: Option<f32>,
}

impl Scorable for Score {
    fn set_min_competitive_score(&mut self, min_competitive_score: f32) -> Result<()> {
        self.min_competitive_score = Some(min_competitive_score);
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

/// `assertEquals(expected, actual, 0f)`.
fn assert_float(expected: f32, actual: f32) {
    assert_equals_delta(f64::from(expected), f64::from(actual), 0.0);
}

/// `w.addDocuments(Arrays.asList(doc × n))` of empty documents.
fn add_empty_docs(w: &IndexWriter, n: usize) -> Result<()> {
    let docs: Vec<Document> = (0..n).map(|_| Document::new()).collect();
    w.add_documents(&docs)?;
    Ok(())
}

/// `new IndexWriter(dir, newIndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE))`.
fn no_merge_writer(dir: &TestDirectory) -> Result<Arc<IndexWriter>> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    IndexWriter::new(dir.as_directory(), config)
}

/// `new Sort(FIELD_SCORE, new SortField("foo", SortField.Type.LONG))`.
fn score_then_foo() -> Result<Sort> {
    Sort::new_fields(vec![
        SortField::FIELD_SCORE,
        SortField::new(Some("foo".to_string()), SortFieldType::Long)?,
    ])
}

/// Port of `TestTopFieldCollector.testSetMinCompetitiveScore`.
fn test_set_min_competitive_score(_f: &mut TopFieldCollectorTest) -> Result<()> {
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

    let sort = score_then_foo()?;
    let mut collector = TopFieldCollectorManager::with_num_hits(sort, 2, 2)?.new_collector()?;
    let mut scorer = Score::default();

    {
        let mut leaf_collector = leaf_of(&mut collector, &leaves[0])?;
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
        assert_float(2f32, min_score(&scorer));

        scorer.score = 0.5;
        // Make sure we do not call setMinCompetitiveScore for non-competitive hits
        scorer.min_competitive_score = Some(f32::NAN);
        collect(&mut *leaf_collector, 3, &mut scorer)?;
        Assert::assert_true(min_score(&scorer).is_nan());

        scorer.score = 4.0;
        collect(&mut *leaf_collector, 4, &mut scorer)?;
        assert_float(3f32, min_score(&scorer));
    }

    // Make sure the min score is set on scorers on new segments
    let mut scorer = Score::default();
    {
        let mut leaf_collector = leaf_of(&mut collector, &leaves[1])?;
        leaf_collector.set_scorer(&mut scorer)?;
        assert_float(3f32, min_score(&scorer));

        scorer.score = 1.0;
        collect(&mut *leaf_collector, 0, &mut scorer)?;
        assert_float(3f32, min_score(&scorer));

        scorer.score = 4.0;
        collect(&mut *leaf_collector, 1, &mut scorer)?;
        assert_float(4f32, min_score(&scorer));
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestTopFieldCollector.testTotalHitsWithScore`.
fn test_total_hits_with_score(_f: &mut TopFieldCollectorTest) -> Result<()> {
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
        let sort = score_then_foo()?;
        let mut collector = TopFieldCollectorManager::with_num_hits(sort, 2, total_hits_threshold)?
            .new_collector()?;
        let mut scorer = Score::default();

        {
            let mut leaf_collector = leaf_of(&mut collector, &leaves[0])?;
            leaf_collector.set_scorer(&mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector, 0, &mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector, 1, &mut scorer)?;
        }

        {
            let mut leaf_collector = leaf_of(&mut collector, &leaves[1])?;
            leaf_collector.set_scorer(&mut scorer)?;

            scorer.score = 3.0;
            collect(&mut *leaf_collector, 1, &mut scorer)?;

            scorer.score = 4.0;
            collect(&mut *leaf_collector, 1, &mut scorer)?;
        }

        let top_docs = collector.top_docs()?;
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

/// Port of `TestTopFieldCollector.testSortNoResults`.
fn test_sort_no_results(_f: &mut TopFieldCollectorTest) -> Result<()> {
    // Two Sort criteria to instantiate the multi/single comparators.
    let sort = two_sorts()?;
    for s in &sort {
        let mut tdc =
            TopFieldCollectorManager::new(s.clone(), 10, None, i32::MAX)?.new_collector()?;
        let td = tdc.top_docs()?;
        assert_equals(&0, &td.total_hits.value());
    }
    Ok(())
}

/// The anonymous `FilterScorable` of `testComputeScoresOnlyOnce`, which fails
/// when the same document is scored twice.
struct OnceScorable<'a> {
    scorer: &'a mut dyn Scorable,
    last_computed_doc: &'a mut i32,
    current_doc: i32,
}

impl Scorable for OnceScorable<'_> {
    fn score(&mut self) -> Result<f32> {
        if *self.last_computed_doc == self.current_doc {
            return Err(LuceneError::AssertionError(format!(
                "Score computed twice on {}",
                self.current_doc
            )));
        }
        *self.last_computed_doc = self.current_doc;
        self.scorer.score()
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        Ok(vec![ChildScorable::new(&mut *self.scorer, "FILTER")])
    }
}

/// The anonymous `FilterLeafCollector` of `testComputeScoresOnlyOnce`: the
/// scorer it hands its delegate is the [`OnceScorable`] over the scorer it
/// receives, both in `setScorer` and, as the port passes the scorer to each
/// `collect`, in `collect`.
struct OnceLeafCollector<'a> {
    r#in: Box<dyn LeafCollector + 'a>,
    current_doc: i32,
    /// The `lastComputedDoc` of the `FilterScorable` the last `setScorer`
    /// built.
    last_computed_doc: i32,
}

impl LeafCollector for OnceLeafCollector<'_> {
    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.current_doc = doc;
        let mut s = OnceScorable {
            scorer,
            last_computed_doc: &mut self.last_computed_doc,
            current_doc: self.current_doc,
        };
        self.r#in.collect(doc, &mut s)
    }

    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.last_computed_doc = -1;
        let mut s = OnceScorable {
            scorer,
            last_computed_doc: &mut self.last_computed_doc,
            current_doc: self.current_doc,
        };
        self.r#in.set_scorer(&mut s)
    }
}

/// The anonymous `Collector` of `testComputeScoresOnlyOnce`.
struct OnceCollector {
    top_collector: TopFieldCollector,
}

impl Collector for OnceCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let r#in = self.top_collector.get_leaf_collector(context)?;
        Ok(Box::new(OnceLeafCollector {
            r#in,
            current_doc: -1,
            last_computed_doc: -1,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `CollectorManager<>` of `testComputeScoresOnlyOnce`.
struct OnceCollectorManager {
    top_field_collector_manager: TopFieldCollectorManager,
}

impl CollectorManager for OnceCollectorManager {
    type Collector = OnceCollector;
    type Output = ();

    fn new_collector(&self) -> Result<OnceCollector> {
        let top_collector = self.top_field_collector_manager.new_collector()?;
        Ok(OnceCollector { top_collector })
    }

    fn reduce(&self, _collectors: Vec<OnceCollector>) -> Result<()> {
        Ok(())
    }
}

/// Port of `TestTopFieldCollector.testComputeScoresOnlyOnce`.
fn test_compute_scores_only_once(_f: &mut TopFieldCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let doc_with = |text: &str| -> Result<Document> {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "text",
            text.to_string(),
            Store::NO,
        )?));
        doc.add(Box::new(NumericDocValuesField::new("relevance", 1)));
        Ok(doc)
    };
    w.add_document(&doc_with("foo")?)?;
    // `text.setStringValue("bar")`
    w.add_document(&doc_with("bar")?)?;
    // `text.setStringValue("baz")`
    w.add_document(&doc_with("baz")?)?;
    let reader = w.get_reader()?;
    let foo: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("text", "foo")));
    let bar: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("text", "bar")));
    let foo: Arc<dyn Query> = Arc::new(BoostQuery::new(foo, 2.0)?);
    let baz: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("text", "baz")));
    let baz: Arc<dyn Query> = Arc::new(BoostQuery::new(baz, 3.0)?);
    let mut builder = BooleanQueryBuilder::new();
    builder.add(foo, Occur::SHOULD)?;
    builder.add(bar, Occur::SHOULD)?;
    builder.add(baz, Occur::SHOULD)?;
    let query: Arc<dyn Query> = Arc::new(builder.build());
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    for sort in [
        Sort::new_fields(vec![SortField::FIELD_SCORE])?,
        Sort::new_fields(vec![SortField::new(
            Some("f".to_string()),
            SortFieldType::Score,
        )?])?,
    ] {
        let manager = OnceCollectorManager {
            top_field_collector_manager: TopFieldCollectorManager::with_num_hits(
                sort,
                TestUtil::next_int(&*random(), 1, 2),
                i32::MAX,
            )?,
        };
        searcher.search_with_manager(Arc::clone(&query), &manager)?;
    }
    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestTopFieldCollector.testPopulateScores`.
fn test_populate_scores(_f: &mut TopFieldCollectorTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let doc_with = |text: &str, sort: i64| -> Result<Document> {
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new("f", text.to_string(), Store::NO)?));
        doc.add(Box::new(NumericDocValuesField::new("sort", sort)));
        Ok(doc)
    };
    w.add_document(&doc_with("foo bar", 0)?)?;

    // `field.setStringValue(""); sortField.setLongValue(3);`
    w.add_document(&doc_with("", 3)?)?;

    // `field.setStringValue("foo foo bar"); sortField.setLongValue(2);`
    w.add_document(&doc_with("foo foo bar", 2)?)?;

    w.flush()?;

    // `field.setStringValue("foo"); sortField.setLongValue(2);`
    w.add_document(&doc_with("foo", 2)?)?;

    // `field.setStringValue("bar bar bar"); sortField.setLongValue(0);`
    w.add_document(&doc_with("bar bar bar", 0)?)?;

    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    for query_text in ["foo", "bar"] {
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("f", query_text)));
        for reverse in [false, true] {
            let mut sorted_by_doc = searcher.search(Arc::clone(&query), 10)?.score_docs;
            sorted_by_doc.sort_by_key(|sd| sd.doc);

            let sort = Sort::new_fields(vec![SortField::new_reverse(
                Some("sort".to_string()),
                SortFieldType::Long,
                reverse,
            )?])?;
            let sorted_by_field = searcher
                .search_with_sort(Arc::clone(&query), 10, &sort)?
                .score_docs;
            // `sortedByField.clone()`: the `ScoreDoc` parts the scores are
            // populated on; the `fields` of each clone are its original's.
            let mut sorted_by_field_clone: Vec<ScoreDoc> =
                sorted_by_field.iter().map(|fd| fd.score_doc).collect();
            TopFieldCollector::populate_scores(
                &mut sorted_by_field_clone,
                &searcher,
                Arc::clone(&query),
            )?;
            for i in 0..sorted_by_field_clone.len() {
                assert_equals(&sorted_by_field_clone[i].doc, &sorted_by_field[i].doc());
                // `assertSame(((FieldDoc) clone[i]).fields, ((FieldDoc) sortedByField[i]).fields)`:
                // the clone shares its original's fields.
                let idx = sorted_by_doc
                    .binary_search_by_key(&sorted_by_field_clone[i].doc, |sd| sd.doc)
                    .unwrap_or_else(|e| e);
                assert_float(sorted_by_field_clone[i].score, sorted_by_doc[idx].score);
            }
        }
    }

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Port of `TestTopFieldCollector.testConcurrentMinScore`.
fn test_concurrent_min_score(_f: &mut TopFieldCollectorTest) -> Result<()> {
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

    let sort = Sort::new_fields(vec![SortField::FIELD_SCORE, SortField::FIELD_DOC])?;
    let manager = TopFieldCollectorManager::with_num_hits(sort, 2, 0)?;
    let mut collector = manager.new_collector()?;
    let mut collector2 = manager.new_collector()?;
    Assert::assert_true(Arc::ptr_eq(
        &collector.min_score_acc().expect("minScoreAcc"),
        &collector2.min_score_acc().expect("minScoreAcc"),
    ));
    let min_value_checker = collector.min_score_acc().expect("minScoreAcc");
    // force the check of the global minimum score on every round
    min_value_checker.set_mod_interval(0);

    let mut scorer = Score::default();
    let mut scorer2 = Score::default();
    let mut collector3 = manager.new_collector()?;
    let mut scorer3 = Score::default();

    {
        let mut leaf_collector = leaf_of(&mut collector, &leaves[0])?;
        leaf_collector.set_scorer(&mut scorer)?;
        let mut leaf_collector2 = leaf_of(&mut collector2, &leaves[1])?;
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
        assert_float(7f32, DocScoreEncoder::to_score(min_value_checker.get_raw()));
        assert_null(&scorer.min_competitive_score);
        assert_float(7f32, min_score(&scorer2));

        scorer2.score = 1.0;
        collect(&mut *leaf_collector2, 3, &mut scorer2)?;
        assert_float(7f32, DocScoreEncoder::to_score(min_value_checker.get_raw()));
        assert_null(&scorer.min_competitive_score);
        assert_float(7f32, min_score(&scorer2));

        scorer.score = 10.0;
        collect(&mut *leaf_collector, 2, &mut scorer)?;
        assert_float(7f32, DocScoreEncoder::to_score(min_value_checker.get_raw()));
        assert_float(7f32, min_score(&scorer));
        assert_float(7f32, min_score(&scorer2));

        scorer.score = 11.0;
        collect(&mut *leaf_collector, 3, &mut scorer)?;
        assert_float(
            10f32,
            DocScoreEncoder::to_score(min_value_checker.get_raw()),
        );
        assert_float(10f32, min_score(&scorer));
        assert_float(7f32, min_score(&scorer2));

        let mut leaf_collector3 = leaf_of(&mut collector3, &leaves[2])?;
        leaf_collector3.set_scorer(&mut scorer3)?;
        assert_float(10f32, min_score(&scorer3));

        scorer3.score = 1f32;
        collect(&mut *leaf_collector3, 0, &mut scorer3)?;
        assert_float(
            10f32,
            DocScoreEncoder::to_score(min_value_checker.get_raw()),
        );
        assert_float(10f32, min_score(&scorer3));

        scorer.score = 11.0;
        collect(&mut *leaf_collector, 4, &mut scorer)?;
        assert_float(
            11f32,
            DocScoreEncoder::to_score(min_value_checker.get_raw()),
        );
        assert_float(11f32, min_score(&scorer));
        assert_float(7f32, min_score(&scorer2));
        assert_float(10f32, min_score(&scorer3));

        scorer3.score = 2f32;
        collect(&mut *leaf_collector3, 1, &mut scorer3)?;
        assert_float(
            11f32,
            DocScoreEncoder::to_score(min_value_checker.get_raw()),
        );
        assert_float(11f32, min_score(&scorer));
        assert_float(7f32, min_score(&scorer2));
        assert_float(11f32, min_score(&scorer3));

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

/// Port of `TestTopFieldCollector.testRandomMinCompetitiveScore`.
fn test_random_min_competitive_score(_f: &mut TopFieldCollectorTest) -> Result<()> {
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
        let sort = Sort::new_fields(vec![SortField::FIELD_SCORE, SortField::FIELD_DOC])?;
        let tdc = do_concurrent_search_with_threshold(5, 0, query, &sort, &index_reader)?;
        let tdc2 = do_search_with_threshold(5, 0, query, &sort, &index_reader)?;

        Assert::assert_true(tdc.total_hits.value() > 0);
        Assert::assert_true(tdc2.total_hits.value() > 0);
        CheckHits::check_equal(&**query, &score_docs(&tdc), &score_docs(&tdc2));
    }

    index_reader.close()?;
    dir.close()
}

/// Port of `TestTopFieldCollector.testRelationVsTopDocsCount`.
fn test_relation_vs_top_docs_count(_f: &mut TopFieldCollectorTest) -> Result<()> {
    let sort = Sort::new_fields(vec![SortField::FIELD_SCORE, SortField::FIELD_DOC])?;
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let w = no_merge_writer(&dir)?;
        let result = (|| -> Result<()> {
            let doc_of = || -> Result<Document> {
                let mut doc = Document::new();
                doc.add(Box::new(TextField::new(
                    "f",
                    "foo bar".to_string(),
                    Store::NO,
                )?));
                Ok(doc)
            };
            let docs: Vec<Document> = (0..5).map(|_| doc_of()).collect::<Result<_>>()?;
            w.add_documents(&docs)?;
            w.flush()?;
            let docs: Vec<Document> = (0..5).map(|_| doc_of()).collect::<Result<_>>()?;
            w.add_documents(&docs)?;
            w.flush()?;

            let reader = directory_reader::open_with_writer(&*w)?;
            let result = (|| -> Result<()> {
                let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
                let foo =
                    || -> Arc<dyn Query> { Arc::new(TermQuery::new(Term::from_text("f", "foo"))) };
                let collector_manager = TopFieldCollectorManager::new(sort.clone(), 2, None, 10)?;
                let top_docs = searcher.search_with_manager(foo(), &collector_manager)?;
                assert_equals(&10, &top_docs.total_hits.value());
                assert_equals(
                    &TotalHitsRelation::EQUAL_TO,
                    &top_docs.total_hits.relation(),
                );

                let collector_manager = TopFieldCollectorManager::new(sort.clone(), 2, None, 2)?;
                let top_docs = searcher.search_with_manager(foo(), &collector_manager)?;
                Assert::assert_true(10 >= top_docs.total_hits.value());
                assert_equals(
                    &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                    &top_docs.total_hits.relation(),
                );

                let collector_manager = TopFieldCollectorManager::new(sort.clone(), 10, None, 2)?;
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
