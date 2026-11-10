//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSimilarity.java`.
//!
//! Similarity unit test.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Field, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::{IndexOptions, LeafReaderContext, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::scorable::Scorable;
use rucene::search::similarities::{
    tfidf_scorer, CollectionStatistics, Explanation, SimScorer, Similarity, TFIDFSimilarity,
    TermStatistics,
};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, CollectionResult, CollectorManager, LeafCollector, Occur, Query,
    ScoreMode, SimpleCollector, SimpleCollectorImpl,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestSimilarity`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSimilarity",
        vec![
            test("testSimilarity", test_similarity),
            test("testSimilarityTermDocNoTf", test_similarity_term_doc_no_tf),
            test("testTermDocSimilarity", test_term_doc_similarity),
        ],
    )
}

/// `public static class SimpleSimilarity extends ClassicSimilarity`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleSimilarity;

impl Similarity for SimpleSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestSimilarity$SimpleSimilarity"
    }

    /// `ClassicSimilarity()` discounts overlaps.
    fn discount_overlaps(&self) -> bool {
        true
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        tfidf_scorer(self, boost, collection_stats, term_stats)
    }
}

impl TFIDFSimilarity for SimpleSimilarity {
    fn length_norm(&self, _length: i32) -> f32 {
        1.0
    }

    fn tf(&self, freq: f32) -> f32 {
        freq
    }

    fn idf(&self, _doc_freq: i64, _doc_count: i64) -> f32 {
        1.0
    }

    /// `ClassicSimilarity.idfExplain(CollectionStatistics, TermStatistics)`,
    /// inherited.
    fn idf_explain(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
    ) -> Explanation {
        let df = term_stats.doc_freq();
        let doc_count = collection_stats.doc_count();
        let idf = self.idf(df, doc_count);
        Explanation::matched(
            idf,
            "idf, computed as log((docCount+1)/(docFreq+1)) + 1 from:",
            vec![
                Explanation::matched(df, "docFreq, number of documents containing term", vec![]),
                Explanation::matched(
                    doc_count,
                    "docCount, total number of documents with field",
                    vec![],
                ),
            ],
        )
    }

    fn idf_explain_phrase(
        &self,
        _collection_stats: &CollectionStatistics,
        _stats: &[TermStatistics],
    ) -> Explanation {
        Explanation::matched(1.0f32, "Inexplicable", vec![])
    }
}

/// The `newIndexWriterConfig(analyzer).setSimilarity(new
/// SimpleSimilarity()).setMergePolicy(newMergePolicy(random(), false))` of
/// the three tests.
fn writer(
    store: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    analyzer: MockAnalyzer,
) -> Result<RandomIndexWriter> {
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    iwc.set_similarity(Some(Arc::new(SimpleSimilarity)))?;
    iwc.set_merge_policy(Some(LuceneTestCase::new_merge_policy_include_mock(
        &*random(),
        false,
    )))?;
    RandomIndexWriter::new_with_config(&*random(), store.as_directory(), iwc)
}

/// The anonymous `ScoreAssertingCollector` of `testSimilarity`.
struct DocPlusBaseCollector {
    base: i32,
}

impl LeafCollector for DocPlusBaseCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        assert_equals_float(doc as f32 + self.base as f32 + 1.0, scorer.score()?);
        Ok(())
    }
}

impl SimpleCollectorImpl for DocPlusBaseCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }

    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.base = context.doc_base();
        Ok(())
    }
}

/// `assertEquals(expected, actual, 0)` over floats.
fn assert_equals_float(expected: f32, actual: f32) {
    randomizedtesting::junit::Assert::assert_equals_float_msg(None, expected, actual, 0.0);
}

/// The anonymous `CollectorManager<SimpleCollector, Void>` of
/// `testSimilarity`.
struct DocPlusBaseManager;

impl CollectorManager for DocPlusBaseManager {
    type Collector = SimpleCollector<DocPlusBaseCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(DocPlusBaseCollector { base: 0 }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

/// Port of `TestSimilarity.testSimilarity`.
fn test_similarity() -> Result<()> {
    let mut store = LuceneTestCase::new_directory();
    let writer = writer(&store, MockAnalyzer::new(&*random()))?;

    let mut d1 = Document::new();
    d1.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a c",
        Store::YES,
    )));

    let mut d2 = Document::new();
    d2.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a c b",
        Store::YES,
    )));

    writer.add_document(&d1)?;
    writer.add_document(&d2)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(SimpleSimilarity));

    let a = Term::from_text("field", "a");
    let b = Term::from_text("field", "b");
    let c = Term::from_text("field", "c");

    assert_score(&searcher, Arc::new(TermQuery::new(b.clone())), 1.0)?;

    let mut bq = BooleanQueryBuilder::new();
    bq.add(Arc::new(TermQuery::new(a.clone())), Occur::SHOULD)?;
    bq.add(Arc::new(TermQuery::new(b.clone())), Occur::SHOULD)?;
    searcher.search_with_manager(Arc::new(bq.build()), &DocPlusBaseManager)?;

    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_bytes(
        a.field(),
        &[a.bytes().clone(), c.bytes().clone()],
    )?);
    assert_score(&searcher, pq, 1.0)?;

    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop_bytes(
        2,
        a.field(),
        &[a.bytes().clone(), b.bytes().clone()],
    )?);
    assert_score(&searcher, pq, 0.5)?;

    drop(searcher);
    reader.close()?;
    store.close()?;
    Ok(())
}

/// The anonymous `ScoreAssertingCollector` of `testSimilarityTermDocNoTf`
/// and of `assertResults`: `assertEquals(scores[base + doc], scorer.score(),
/// 0)`, counting the hits.
struct ScoresCollector {
    scores: Arc<Vec<f32>>,
    count: Arc<AtomicI64>,
    base: i32,
}

impl LeafCollector for ScoresCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        assert_equals_float(self.scores[(self.base + doc) as usize], scorer.score()?);
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl SimpleCollectorImpl for ScoresCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }

    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.base = context.doc_base();
        Ok(())
    }
}

/// The anonymous `CollectorManager<SimpleCollector, Void>` of
/// `testSimilarityTermDocNoTf`, whose `newCollector` makes a fresh
/// `scores = {1, 2}` for each collector.
struct NoTfManager;

impl CollectorManager for NoTfManager {
    type Collector = SimpleCollector<ScoresCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        // score is the sum(1) for each matching term in the doc
        let scores = Arc::new(vec![1.0f32, 2.0]);
        Ok(SimpleCollector::new(ScoresCollector {
            scores,
            count: Arc::new(AtomicI64::new(0)),
            base: 0,
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

/// `new FieldType(TextField.TYPE_STORED)` with `DOCS_AND_CUSTOM_FREQS`.
fn custom_freqs_type() -> Result<FieldType> {
    let mut field_type = FieldType::new_from(TextField::type_stored());
    field_type.set_index_options(IndexOptions::DOCS_AND_CUSTOM_FREQS)?;
    Ok(field_type)
}

/// Port of `TestSimilarity.testSimilarityTermDocNoTf`.
fn test_similarity_term_doc_no_tf() -> Result<()> {
    let mut store = LuceneTestCase::new_directory();
    let writer = writer(&store, MockAnalyzer::new(&*random()))?;

    let field_type = custom_freqs_type()?;

    let mut d1 = Document::new();
    d1.add(Box::new(Field::new(
        "field",
        "a c".to_string(),
        field_type.clone(),
    )?));

    let mut d2 = Document::new();
    d2.add(Box::new(Field::new(
        "field",
        "a c b".to_string(),
        field_type.clone(),
    )?));

    writer.add_document(&d1)?;
    writer.add_document(&d2)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(SimpleSimilarity));

    let a = Term::from_text("field", "a");
    let b = Term::from_text("field", "b");
    let c = Term::from_text("field", "c");

    // when no custom term freq is provided, default to 1
    assert_score(&searcher, Arc::new(TermQuery::new(a.clone())), 1.0)?;
    assert_score(&searcher, Arc::new(TermQuery::new(b.clone())), 1.0)?;
    assert_score(&searcher, Arc::new(TermQuery::new(c)), 1.0)?;

    let mut bq = BooleanQueryBuilder::new();
    bq.add(Arc::new(TermQuery::new(a)), Occur::SHOULD)?;
    bq.add(Arc::new(TermQuery::new(b)), Occur::SHOULD)?;
    searcher.search_with_manager(Arc::new(bq.build()), &NoTfManager)?;
    drop(searcher);
    reader.close()?;
    store.close()?;
    Ok(())
}

/// Port of `TestSimilarity.testTermDocSimilarity`.
fn test_term_doc_similarity() -> Result<()> {
    let mut store = LuceneTestCase::new_directory();
    let term_freq: rucene_test_framework::analysis::mock_token_filter::CustomTermFreq =
        Arc::new(|term: &str| match term {
            "a" => 17,
            "b" => i32::MAX,
            "d" => i32::MAX,
            _ => 1,
        });

    let writer = writer(&store, MockAnalyzer::with_term_freq(&*random(), term_freq))?;

    let field_type = custom_freqs_type()?;

    let mut d1 = Document::new();
    d1.add(Box::new(Field::new(
        "field",
        "a c".to_string(),
        field_type.clone(),
    )?));

    let mut d2 = Document::new();
    d2.add(Box::new(Field::new(
        "field",
        "a c b d".to_string(),
        field_type.clone(),
    )?));

    writer.add_document(&d1)?;
    writer.add_document(&d2)?;
    let reader = writer.get_reader()?;
    let result = (|| -> Result<()> {
        writer.close()?;

        let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        searcher.set_similarity(Arc::new(SimpleSimilarity));

        let a = Term::from_text("field", "a");
        let b = Term::from_text("field", "b");
        let c = Term::from_text("field", "c");
        let d = Term::from_text("field", "d");

        assert_score(&searcher, Arc::new(TermQuery::new(a.clone())), 17.0)?;
        assert_score(
            &searcher,
            Arc::new(TermQuery::new(b.clone())),
            i32::MAX as f32,
        )?;
        assert_score(&searcher, Arc::new(TermQuery::new(c)), 1.0)?;

        let mut bq = BooleanQueryBuilder::new();
        bq.add(Arc::new(TermQuery::new(a)), Occur::SHOULD)?;
        bq.add(Arc::new(TermQuery::new(b.clone())), Occur::SHOULD)?;
        // BooleanScorer takes the sum of scores of disjunctive terms
        // but loss of floating precision means MAX_VALUE + 17 == MAX_VALUE.
        assert_results(&searcher, Arc::new(bq.build()), &[17.0, i32::MAX as f32])?;

        let mut bq = BooleanQueryBuilder::new();
        bq.add(Arc::new(TermQuery::new(b)), Occur::SHOULD)?;
        bq.add(Arc::new(TermQuery::new(d)), Occur::SHOULD)?;
        // here we see we can sum max int value twice; there are no problems with overflow
        assert_results(
            &searcher,
            Arc::new(bq.build()),
            &[-1.0, 2.0 * i32::MAX as f32],
        )?;
        Ok(())
    })();
    let closed = reader.close();
    result.and(closed)?;
    store.close()?;
    Ok(())
}

/// The anonymous `CollectorManager<SimpleCollector, Integer>` of
/// `assertResults`, whose `LongAdder count` the collectors share.
struct ResultsManager {
    scores: Arc<Vec<f32>>,
    count: Arc<AtomicI64>,
}

impl CollectorManager for ResultsManager {
    type Collector = SimpleCollector<ScoresCollector>;
    type Output = i32;

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(ScoresCollector {
            scores: Arc::clone(&self.scores),
            count: Arc::clone(&self.count),
            base: 0,
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<i32> {
        Ok(self.count.load(Ordering::SeqCst) as i32)
    }
}

/// `private static void assertResults(IndexSearcher searcher, Query query, float[] scores)`.
fn assert_results(searcher: &IndexSearcher, query: Arc<dyn Query>, scores: &[f32]) -> Result<()> {
    let collector = ResultsManager {
        scores: Arc::new(scores.to_vec()),
        count: Arc::new(AtomicI64::new(0)),
    };

    let mut expected_count = 0;
    for &score in scores {
        if score >= 0.0 {
            expected_count += 1;
        }
    }
    let count = searcher.search_with_manager(query, &collector)?;
    assert_equals(&expected_count, &count);
    Ok(())
}

/// The anonymous `ScoreAssertingCollector` of `assertScore`.
struct ScoreCollector {
    score: f32,
}

impl LeafCollector for ScoreCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        assert_equals_float(self.score, scorer.score()?);
        Ok(())
    }
}

impl SimpleCollectorImpl for ScoreCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `CollectorManager<SimpleCollector, Void>` of `assertScore`.
struct ScoreManager {
    score: f32,
}

impl CollectorManager for ScoreManager {
    type Collector = SimpleCollector<ScoreCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(ScoreCollector { score: self.score }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

/// `private static void assertScore(IndexSearcher searcher, Query query, float score)`.
fn assert_score(searcher: &IndexSearcher, query: Arc<dyn Query>, score: f32) -> Result<()> {
    searcher.search_with_manager(query, &ScoreManager { score })
}
