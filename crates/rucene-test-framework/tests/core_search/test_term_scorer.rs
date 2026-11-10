//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTermScorer.java`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{DocAndFloatFeatureBuffer, IndexReader, LeafReader, LeafReaderContext, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::scorable::Scorable;
use rucene::search::similarities::ClassicSimilarity;
use rucene::search::term_query::TermQuery;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{
    BooleanQueryBuilder, CollectionError, CollectionResult, LeafCollector, Occur, Query, ScoreMode,
    NO_MORE_DOCS,
};
use rucene::util::{Bits, MatchNoBits};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, assert_equals_delta, random};

const FIELD: &str = "field";

/// The instance of `TestTermScorer`.
pub struct TermScorerTest {
    directory: Option<TestDirectory>,
    values: Vec<&'static str>,
    index_searcher: Option<IndexSearcher>,
    index_reader: Option<Arc<dyn LeafReader>>,
}

impl Default for TermScorerTest {
    fn default() -> Self {
        TermScorerTest {
            directory: None,
            values: vec!["all", "dogs dogs", "like", "playing", "fetch", "all"],
            index_searcher: None,
            index_reader: None,
        }
    }
}

impl TermScorerTest {
    fn searcher(&self) -> &IndexSearcher {
        self.index_searcher.as_ref().expect("setUp ran")
    }

    fn reader(&self) -> &Arc<dyn LeafReader> {
        self.index_reader.as_ref().expect("setUp ran")
    }
}

/// `TestTermScorer`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<TermScorerTest>(
        "org.apache.lucene.search.TestTermScorer",
        vec![
            set_up(term_scorer_set_up),
            tear_down(term_scorer_tear_down),
            ftest("test", test),
            ftest("testNext", test_next),
            ftest("testAdvance", test_advance),
            ftest("testDoesNotLoadNorms", test_does_not_load_norms),
            ftest("testRandomTopDocs", test_random_top_docs),
            ftest("testNextDocsAndScores", test_next_docs_and_scores),
        ],
    )
}

/// `setUp()`.
fn term_scorer_set_up(f: &mut TermScorerTest) -> Result<()> {
    let directory = LuceneTestCase::new_directory();

    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    config.set_similarity(Some(Arc::new(ClassicSimilarity::new())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    for value in &f.values {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            FIELD,
            value,
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    let index_reader =
        LuceneTestCase::get_only_leaf_reader(writer.get_reader()? as Arc<dyn IndexReader>)?;
    writer.close()?;
    let mut index_searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&index_reader), false);
    index_searcher.set_similarity(Arc::new(ClassicSimilarity::new()));
    f.directory = Some(directory);
    f.index_reader = Some(index_reader);
    f.index_searcher = Some(index_searcher);
    Ok(())
}

/// `tearDown()`.
fn term_scorer_tear_down(f: &mut TermScorerTest) -> Result<()> {
    f.index_searcher = None;
    if let Some(reader) = f.index_reader.take() {
        reader.as_index_reader().close()?;
    }
    if let Some(mut directory) = f.directory.take() {
        directory.close()?;
    }
    Ok(())
}

/// `(LeafReaderContext) indexSearcher.getTopReaderContext()`, after
/// `assertTrue(... instanceof LeafReaderContext)`.
fn top_leaf_context(searcher: &IndexSearcher) -> Arc<LeafReaderContext> {
    let top = searcher.get_top_reader_context();
    Assert::assert_true(top.is_leaf_context());
    Arc::clone(top).as_leaf().expect("LeafReaderContext")
}

/// `private static class TestHit`.
#[derive(Debug, Clone, Copy)]
struct TestHit {
    doc: i32,
    score: f32,
}

impl fmt::Display for TestHit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TestHit{{doc={}, score={}}}",
            self.doc,
            rucene::util::java_fmt::float_to_string(self.score)
        )
    }
}

/// The anonymous `SimpleCollector` of `test`, scored as the leaf collector
/// it is: its `doSetNextReader` is never called, so `base` stays `0`.
struct HitsCollector {
    base: i32,
    docs: Vec<TestHit>,
}

impl LeafCollector for HitsCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let score = scorer.score()?;
        let doc = doc + self.base;
        self.docs.push(TestHit { doc, score });
        Assert::assert_true_msg(
            &format!(
                "score {} is not greater than 0",
                rucene::util::java_fmt::float_to_string(score)
            ),
            score > 0.0,
        );
        Assert::assert_true_msg(
            &format!("Doc: {doc} does not equal 0 or doc does not equal 5"),
            doc == 0 || doc == 5,
        );
        Ok(())
    }
}

/// Port of `TestTermScorer.test`.
fn test(f: &mut TermScorerTest) -> Result<()> {
    let all_term = Term::from_text(FIELD, "all");
    let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(all_term));

    let searcher = f.searcher();
    let weight = searcher.create_weight(term_query, ScoreMode::COMPLETE, 1.0)?;
    let context = top_leaf_context(searcher);
    let mut ts = weight.bulk_scorer(&context)?.expect("bulkScorer");
    // we have 2 documents with the term all in them, one document for all the
    // other values
    // must call next first
    let mut collector = HitsCollector {
        base: 0,
        docs: Vec::new(),
    };

    ts.score(&mut collector, None, 0, NO_MORE_DOCS)
        .map_err(CollectionError::into_lucene_error)?;
    let docs = collector.docs;
    Assert::assert_true_msg(
        &format!("docs Size: {} is not: 2", docs.len()),
        docs.len() == 2,
    );
    let doc0 = docs[0];
    let doc5 = docs[1];
    // The scores should be the same
    Assert::assert_true_msg(
        &format!(
            "{} does not equal: {}",
            rucene::util::java_fmt::float_to_string(doc0.score),
            rucene::util::java_fmt::float_to_string(doc5.score)
        ),
        doc0.score == doc5.score,
    );
    Ok(())
}

/// Port of `TestTermScorer.testNext`.
fn test_next(f: &mut TermScorerTest) -> Result<()> {
    let all_term = Term::from_text(FIELD, "all");
    let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(all_term));

    let searcher = f.searcher();
    let weight = searcher.create_weight(term_query, ScoreMode::COMPLETE, 1.0)?;
    let context = top_leaf_context(searcher);
    let mut ts = weight.scorer(&context)?.expect("scorer");
    Assert::assert_true_msg(
        "next did not return a doc",
        ts.iterator().next_doc()? != NO_MORE_DOCS,
    );
    Assert::assert_true_msg(
        "next did not return a doc",
        ts.iterator().next_doc()? != NO_MORE_DOCS,
    );
    Assert::assert_true_msg(
        "next returned a doc and it should not have",
        ts.iterator().next_doc()? == NO_MORE_DOCS,
    );
    Ok(())
}

/// Port of `TestTermScorer.testAdvance`.
fn test_advance(f: &mut TermScorerTest) -> Result<()> {
    let all_term = Term::from_text(FIELD, "all");
    let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(all_term));

    let searcher = f.searcher();
    let weight = searcher.create_weight(term_query, ScoreMode::COMPLETE, 1.0)?;
    let context = top_leaf_context(searcher);
    let mut ts = weight.scorer(&context)?.expect("scorer");
    Assert::assert_true_msg("Didn't skip", ts.iterator().advance(3)? != NO_MORE_DOCS);
    // The next doc should be doc 5
    Assert::assert_true_msg("doc should be number 5", ts.doc_id() == 5);
    Ok(())
}

/// Port of `TestTermScorer.testDoesNotLoadNorms`.
fn test_does_not_load_norms(f: &mut TermScorerTest) -> Result<()> {
    let all_term = Term::from_text(FIELD, "all");
    let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(all_term));

    let forbidden_norms: Arc<dyn LeafReader> = Arc::new(ForbiddenNormsLeafReader {
        inner: FilterLeafReader::new(Arc::clone(f.reader()))?,
    });
    // We don't use newSearcher because it sometimes runs checkIndex which loads norms
    let index_searcher = IndexSearcher::new(Arc::clone(&forbidden_norms).as_index_reader())?;
    let context = Arc::clone(&forbidden_norms)
        .as_index_reader()
        .get_context()?
        .as_leaf()
        .expect("LeafReaderContext");

    let weight = index_searcher.create_weight(Arc::clone(&term_query), ScoreMode::COMPLETE, 1.0)?;
    expect_throws("java.lang.AssertionError", || {
        weight
            .scorer(&context)?
            .expect("scorer")
            .iterator()
            .next_doc()
    });

    let weight2 = index_searcher.create_weight(term_query, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
    // should not fail this time since norms are not necessary
    weight2
        .scorer(&context)?
        .expect("scorer")
        .iterator()
        .next_doc()?;
    Ok(())
}

/// Port of `TestTermScorer.testRandomTopDocs`.
fn test_random_top_docs(_f: &mut TermScorerTest) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(128 * 8 * 8 * 3)
    } else {
        LuceneTestCase::at_least(500)
    }; // at night, make sure some terms have skip data
    for _i in 0..num_docs {
        let mut doc = Document::new();
        let num_values = random().next_int_bound(1 << random().next_int_bound(5));
        let start = random().next_int_bound(10);
        for j in 0..num_values {
            let freq = TestUtil::next_int(&*random(), 1, 1 << random().next_int_bound(3));
            for _k in 0..freq {
                doc.add(Box::new(TextField::new(
                    "foo",
                    (start + j).to_string(),
                    Store::NO,
                )?));
            }
        }
        w.add_document(&doc)?;
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    for iter in 0..15 {
        let query: Arc<dyn Query> =
            Arc::new(TermQuery::new(Term::from_text("foo", &iter.to_string())));

        let complete_manager = TopScoreDocCollectorManager::with_num_hits(10, i32::MAX)?; // COMPLETE
        let top_scores_manager = TopScoreDocCollectorManager::with_num_hits(10, 1)?; // TOP_SCORES

        let complete = searcher.search_with_manager(Arc::clone(&query), &complete_manager)?;
        let top_scores = searcher.search_with_manager(Arc::clone(&query), &top_scores_manager)?;
        CheckHits::check_equal(&*query, &complete.score_docs, &top_scores.score_docs);

        let filter_term = random().next_int_bound(15);
        let mut filtered = BooleanQueryBuilder::new();
        filtered.add(Arc::clone(&query), Occur::MUST)?;
        filtered.add(
            Arc::new(TermQuery::new(Term::from_text(
                "foo",
                &filter_term.to_string(),
            ))),
            Occur::FILTER,
        )?;
        let filtered_query: Arc<dyn Query> = Arc::new(filtered.build());

        let complete_manager = TopScoreDocCollectorManager::with_num_hits(10, i32::MAX)?; // COMPLETE
        let top_scores_manager = TopScoreDocCollectorManager::with_num_hits(10, 1)?; // TOP_SCORES
        let complete =
            searcher.search_with_manager(Arc::clone(&filtered_query), &complete_manager)?;
        let top_scores = searcher.search_with_manager(filtered_query, &top_scores_manager)?;
        CheckHits::check_equal(&*query, &complete.score_docs, &top_scores.score_docs);
    }
    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Port of `TestTermScorer.testNextDocsAndScores`.
fn test_next_docs_and_scores(f: &mut TermScorerTest) -> Result<()> {
    let all_term = Term::from_text(FIELD, "all");
    let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(all_term));
    let searcher = f.searcher();
    let weight = searcher.create_weight(term_query, ScoreMode::TOP_SCORES, 1f32)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let context = &leaves[0];
    let live_docs = context.leaf_reader().get_live_docs();
    let mut scorer1 = weight.scorer(context)?.expect("scorer");
    let mut scorer2 = weight.scorer(context)?.expect("scorer");
    scorer1.iterator().next_doc()?;
    scorer2.iterator().next_doc()?;
    let mut buffer = DocAndFloatFeatureBuffer::new();
    loop {
        let cur_doc = scorer2.iterator().doc_id();
        let up_to = TestUtil::next_int(
            &*random(),
            cur_doc,
            (i64::from(i32::MAX)).min(i64::from(cur_doc) + 512) as i32,
        );
        scorer1.next_docs_and_scores(up_to, live_docs.as_deref(), &mut buffer)?;
        assert_equals(&(buffer.size == 0), &(cur_doc >= up_to));

        for i in 0..buffer.size {
            while live_docs
                .as_ref()
                .is_some_and(|l| !l.get(scorer2.iterator().doc_id() as usize))
            {
                scorer2.iterator().next_doc()?;
            }
            assert_equals(&scorer2.iterator().doc_id(), &buffer.docs[i]);
            assert_equals_delta(
                f64::from(scorer2.score()?),
                f64::from(buffer.features[i]),
                0.0,
            );
            scorer2.iterator().next_doc()?;
        }

        assert_equals(&scorer2.iterator().doc_id(), &scorer1.iterator().doc_id());
        if scorer1.iterator().doc_id() == NO_MORE_DOCS {
            break;
        }
    }

    let mut scorer3 = weight.scorer(context)?.expect("scorer");
    scorer3.iterator().next_doc()?;
    let match_no_bits = MatchNoBits::new(context.leaf_reader().max_doc() as usize);
    scorer3.next_docs_and_scores(NO_MORE_DOCS, Some(&match_no_bits as &dyn Bits), &mut buffer)?;
    assert_equals(&0, &buffer.size);
    Ok(())
}

/// The anonymous `FilterLeafReader` of `testDoesNotLoadNorms`, whose
/// `getNormValues` fails and whose cache helpers are `in`'s.
struct ForbiddenNormsLeafReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for ForbiddenNormsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForbiddenNormsLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ForbiddenNormsLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for ForbiddenNormsLeafReader {
    fn core(&self) -> &rucene::index::IndexReaderCore {
        LeafReader::core(&self.inner)
    }
    fn filter_leaf_reader_delegate(&self) -> Option<&Arc<dyn LeafReader>> {
        LeafReader::filter_leaf_reader_delegate(&self.inner)
    }
    fn term_vectors(&self) -> Result<Box<dyn rucene::index::TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }
    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }
    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }
    fn stored_fields(&self) -> Result<Box<dyn rucene::index::StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }
    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }
    fn terms(&self, field: &str) -> Result<Option<Box<dyn rucene::index::Terms>>> {
        self.inner.terms(field)
    }
    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }
    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }
    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }
    fn get_norm_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        // `fail("Norms should not be loaded")`: the `AssertionError` it
        // throws.
        Err(LuceneError::AssertionError(
            "Norms should not be loaded".to_string(),
        ))
    }
    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }
    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }
    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }
    fn get_field_infos(&self) -> rucene::index::FieldInfos {
        self.inner.get_field_infos()
    }
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        LeafReader::get_live_docs(&self.inner)
    }
    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn rucene::index::PointValues>>> {
        self.inner.get_point_values(field)
    }
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }
    fn get_meta_data(&self) -> rucene::index::LeafMetaData {
        self.inner.get_meta_data()
    }
}
