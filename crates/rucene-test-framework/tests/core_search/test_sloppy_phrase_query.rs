//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSloppyPhraseQuery.java`.
//!
//! The `static final` documents and queries are rebuilt where Java reads
//! them: their construction draws nothing.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::phrase_scorer::PhraseScorer;
use rucene::search::scorable::Scorable;
use rucene::search::{
    query_to_string, CollectionResult, Collector, CollectorManager, LeafCollector, Query,
    ScoreMode, SimpleCollector, SimpleCollectorImpl,
};
use rucene::store::ByteBuffersDirectory;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::doc_of;
use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `TestSloppyPhraseQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSloppyPhraseQuery",
        vec![
            test(
                "testDoc4_Query4_All_Slops_Should_match",
                test_doc4_query4_all_slops_should_match,
            ),
            test(
                "testDoc1_Query1_All_Slops_Should_match",
                test_doc1_query1_all_slops_should_match,
            ),
            test(
                "testDoc2_Query1_Slop_6_or_more_Should_match",
                test_doc2_query1_slop_6_or_more_should_match,
            ),
            test(
                "testDoc2_Query2_All_Slops_Should_match",
                test_doc2_query2_all_slops_should_match,
            ),
            test(
                "testDoc3_Query1_All_Slops_Should_match",
                test_doc3_query1_all_slops_should_match,
            ),
            test(
                "testDoc5_Query5_Any_Slop_Should_be_consistent",
                test_doc5_query5_any_slop_should_be_consistent,
            ),
            test("testSlopWithHoles", test_slop_with_holes),
            test("testInfiniteFreq1", test_infinite_freq1),
            test("testInfiniteFreq2", test_infinite_freq2),
        ],
    )
}

const S_1: &str = "A A A";
const S_2: &str = "A 1 2 3 A 4 5 6 A";

fn doc_1() -> Result<Document> {
    make_document(&format!("X {S_1} Y"))
}
fn doc_2() -> Result<Document> {
    make_document(&format!("X {S_2} Y"))
}
fn doc_3() -> Result<Document> {
    make_document(&format!("X {S_1} A Y"))
}
fn doc_1_b() -> Result<Document> {
    make_document(&format!("X {S_1} Y N N N N {S_1} Z"))
}
fn doc_2_b() -> Result<Document> {
    make_document(&format!("X {S_2} Y N N N N {S_2} Z"))
}
fn doc_3_b() -> Result<Document> {
    make_document(&format!("X {S_1} A Y N N N N {S_1} A Y"))
}
fn doc_4() -> Result<Document> {
    make_document("A A X A X B A X B B A A X B A A")
}
fn doc_5_3() -> Result<Document> {
    make_document("H H H X X X H H H X X X H H H")
}
fn doc_5_4() -> Result<Document> {
    make_document("H H H H")
}

fn query_1() -> Result<PhraseQuery> {
    make_phrase_query(S_1)
}
fn query_2() -> Result<PhraseQuery> {
    make_phrase_query(S_2)
}
fn query_4() -> Result<PhraseQuery> {
    make_phrase_query("X A A")
}
fn query_5_4() -> Result<PhraseQuery> {
    make_phrase_query("H H H H")
}

/// Test DOC_4 and QUERY_4. QUERY_4 has a fuzzy (len=1) match to DOC_4, so all slop values &gt; 0
/// should succeed. But only the 3rd sequence of A's in DOC_4 will do.
///
/// Port of `TestSloppyPhraseQuery.testDoc4_Query4_All_Slops_Should_match`.
fn test_doc4_query4_all_slops_should_match() -> Result<()> {
    for slop in 0..30 {
        let num_results_expected = if slop < 1 { 0 } else { 1 };
        check_phrase_query(&doc_4()?, &query_4()?, slop, num_results_expected)?;
    }
    Ok(())
}

/// Test DOC_1 and QUERY_1. QUERY_1 has an exact match to DOC_1, so all slop values should succeed.
/// Before LUCENE-1310, a slop value of 1 did not succeed.
///
/// Port of `TestSloppyPhraseQuery.testDoc1_Query1_All_Slops_Should_match`.
fn test_doc1_query1_all_slops_should_match() -> Result<()> {
    for slop in 0..30 {
        let freq1 = check_phrase_query(&doc_1()?, &query_1()?, slop, 1)?;
        let freq2 = check_phrase_query(&doc_1_b()?, &query_1()?, slop, 1)?;
        Assert::assert_true_msg(
            &format!("slop={slop} freq2={freq2} should be greater than score1 {freq1}"),
            freq2 > freq1,
        );
    }
    Ok(())
}

/// Test DOC_2 and QUERY_1. 6 should be the minimum slop to make QUERY_1 match DOC_2. Before
/// LUCENE-1310, 7 was the minimum.
///
/// Port of `TestSloppyPhraseQuery.testDoc2_Query1_Slop_6_or_more_Should_match`.
fn test_doc2_query1_slop_6_or_more_should_match() -> Result<()> {
    for slop in 0..30 {
        let num_results_expected = if slop < 6 { 0 } else { 1 };
        let freq1 = check_phrase_query(&doc_2()?, &query_1()?, slop, num_results_expected)?;
        if num_results_expected > 0 {
            let freq2 = check_phrase_query(&doc_2_b()?, &query_1()?, slop, 1)?;
            Assert::assert_true_msg(
                &format!("slop={slop} freq2={freq2} should be greater than freq1 {freq1}"),
                freq2 > freq1,
            );
        }
    }
    Ok(())
}

/// Test DOC_2 and QUERY_2. QUERY_2 has an exact match to DOC_2, so all slop values should succeed.
/// Before LUCENE-1310, 0 succeeds, 1 through 7 fail, and 8 or greater succeeds.
///
/// Port of `TestSloppyPhraseQuery.testDoc2_Query2_All_Slops_Should_match`.
fn test_doc2_query2_all_slops_should_match() -> Result<()> {
    for slop in 0..30 {
        let freq1 = check_phrase_query(&doc_2()?, &query_2()?, slop, 1)?;
        let freq2 = check_phrase_query(&doc_2_b()?, &query_2()?, slop, 1)?;
        Assert::assert_true_msg(
            &format!("slop={slop} freq2={freq2} should be greater than freq1 {freq1}"),
            freq2 > freq1,
        );
    }
    Ok(())
}

/// Test DOC_3 and QUERY_1. QUERY_1 has an exact match to DOC_3, so all slop values should succeed.
///
/// Port of `TestSloppyPhraseQuery.testDoc3_Query1_All_Slops_Should_match`.
fn test_doc3_query1_all_slops_should_match() -> Result<()> {
    for slop in 0..30 {
        let freq1 = check_phrase_query(&doc_3()?, &query_1()?, slop, 1)?;
        let freq2 = check_phrase_query(&doc_3_b()?, &query_1()?, slop, 1)?;
        Assert::assert_true_msg(
            &format!("slop={slop} freq2={freq2} should be greater than freq1 {freq1}"),
            freq2 > freq1,
        );
    }
    Ok(())
}

/// LUCENE-3412
///
/// Port of `TestSloppyPhraseQuery.testDoc5_Query5_Any_Slop_Should_be_consistent`.
fn test_doc5_query5_any_slop_should_be_consistent() -> Result<()> {
    let n_repeats = 5;
    for slop in 0..3 {
        for _trial in 0..n_repeats {
            // should steadily always find this one
            check_phrase_query(&doc_5_4()?, &query_5_4()?, slop, 1)?;
        }
        for _trial in 0..n_repeats {
            // should steadily never find this one
            check_phrase_query(&doc_5_3()?, &query_5_4()?, slop, 0)?;
        }
    }
    Ok(())
}

/// `private float checkPhraseQuery(Document doc, PhraseQuery query, int slop, int expectedNumResults)`.
fn check_phrase_query(
    doc: &Document,
    query: &PhraseQuery,
    slop: i32,
    expected_num_results: i32,
) -> Result<f32> {
    let mut builder = PhraseQueryBuilder::new();
    let terms = query.get_terms();
    let positions = query.get_positions();
    for i in 0..terms.len() {
        builder.add_at(terms[i].clone(), positions[i])?;
    }
    builder.set_slop(slop);
    let query = builder.build()?;

    let mut ram_dir = rucene_test_framework::util::lucene_test_case::TestDirectory::Mock(
        MockDirectoryWrapper::new(&*random(), Arc::new(ByteBuffersDirectory::default())),
    );
    let writer = RandomIndexWriter::new(
        &*random(),
        ram_dir.as_directory(),
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&WHITESPACE),
            false,
        )),
    )?;
    writer.add_document(doc)?;

    let reader = writer.get_reader()?;

    // Disable assertions as the collector expects a PhraseScorer, not an asserting wrapper.
    let searcher =
        LuceneTestCase::new_searcher_assert(Arc::clone(&reader), random().next_boolean(), false);
    let result = searcher.search_with_manager(Arc::new(query.clone()), &MaxFreqCollectorManager)?;
    assert_equals_msg(
        Some(&format!(
            "slop: {slop}  query: {}  doc: {doc}  Wrong number of hits",
            query_to_string(&query)
        )),
        &expected_num_results,
        &result.total_hits,
    );

    // QueryUtils.check(query,searcher);
    writer.close()?;
    drop(searcher);
    reader.close()?;
    ram_dir.close()?;

    // returns the max Scorer.freq() found, because even though norms are omitted, many index stats
    // are different
    // with these different tokens/distributions/lengths.. otherwise this test is very fragile.
    Ok(result.max)
}

/// `new FieldType(TextField.TYPE_NOT_STORED)` with `setOmitNorms(true)`.
fn omit_norms_type() -> Result<FieldType> {
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_omit_norms(true)?;
    Ok(custom_type)
}

/// `private static Document makeDocument(String docText)`.
fn make_document(doc_text: &str) -> Result<Document> {
    let mut doc = Document::new();
    let custom_type = omit_norms_type()?;
    let f = Field::new("f", doc_text.to_string(), custom_type)?;
    doc.add(Box::new(f));
    Ok(doc)
}

/// `private static PhraseQuery makePhraseQuery(String terms)`.
fn make_phrase_query(terms: &str) -> Result<PhraseQuery> {
    // `terms.split(" +")`.
    let t: Vec<&str> = terms.split(' ').filter(|s| !s.is_empty()).collect();
    PhraseQuery::from_strings("f", &t)
}

/// `static class Result`.
#[derive(Debug, Default)]
struct MaxFreqResult {
    max: f32,
    total_hits: i32,
}

/// `static class MaxFreqCollectorManager implements CollectorManager<MaxFreqCollector, Result>`.
struct MaxFreqCollectorManager;

impl CollectorManager for MaxFreqCollectorManager {
    type Collector = MaxFreqCollector;
    type Output = MaxFreqResult;

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(MaxFreqCollector::default())
    }

    fn reduce(&self, collectors: Vec<Self::Collector>) -> Result<MaxFreqResult> {
        let mut result = MaxFreqResult::default();
        for collector in collectors {
            result.max = result.max.max(collector.max);
            result.total_hits += collector.total_hits;
        }
        Ok(result)
    }
}

/// `static class MaxFreqCollector extends SimpleCollector`.
#[derive(Debug, Default)]
struct MaxFreqCollector {
    max: f32,
    total_hits: i32,
}

/// `(PhraseScorer) scorer`.
fn as_phrase_scorer(scorer: &mut dyn Scorable) -> &mut PhraseScorer {
    scorer
        .as_scorer()
        .and_then(|s| s.as_any_mut())
        .and_then(|a| a.downcast_mut::<PhraseScorer>())
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message("java.lang.ClassCastException")
        })
}

impl LeafCollector for MaxFreqCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.total_hits += 1;
        let ps = as_phrase_scorer(scorer);
        let mut freq = ps.matcher().sloppy_weight();
        while ps.matcher().next_match()? {
            freq += ps.matcher().sloppy_weight();
        }
        self.max = self.max.max(freq);
        Ok(())
    }
}

/// A `SimpleCollector` is its own leaf collector.
impl Collector for MaxFreqCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `SimpleCollector` of `assertSaneScoring`.
struct SaneScoringCollector;

impl LeafCollector for SaneScoringCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        // `(Scorer) scorer`.
        let scorer = scorer.as_scorer().unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message("java.lang.ClassCastException")
        });
        Assert::assert_false(scorer.score()?.is_infinite());
        Ok(())
    }
}

impl SimpleCollectorImpl for SaneScoringCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `CollectorManager<SimpleCollector, Void>` of
/// `assertSaneScoring`.
struct SaneScoringManager;

impl CollectorManager for SaneScoringManager {
    type Collector = SimpleCollector<SaneScoringCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(SaneScoringCollector))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

/// checks that no scores are infinite
///
/// `private void assertSaneScoring(PhraseQuery pq, IndexSearcher searcher)`.
fn assert_sane_scoring(pq: &PhraseQuery, searcher: &IndexSearcher) -> Result<()> {
    let pq: Arc<dyn Query> = Arc::new(pq.clone());
    searcher.search_with_manager(Arc::clone(&pq), &SaneScoringManager)?;

    QueryUtils::check_searcher(&*random(), &pq, Some(searcher));
    Ok(())
}

/// LUCENE-3215
///
/// Port of `TestSloppyPhraseQuery.testSlopWithHoles`.
fn test_slop_with_holes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let custom_type = omit_norms_type()?;
    // `Document doc` holding `Field f`: rebuilt from `f` for each
    // `addDocument`.
    let mut f = Field::new("lyrics", String::new(), custom_type)?;
    f.set_string_value("drug drug".to_string())?;
    iw.add_document(&doc_of(&[&f])?)?;
    f.set_string_value("drug druggy drug".to_string())?;
    iw.add_document(&doc_of(&[&f])?)?;
    f.set_string_value("drug druggy druggy drug".to_string())?;
    iw.add_document(&doc_of(&[&f])?)?;
    f.set_string_value("drug druggy drug druggy drug".to_string())?;
    iw.add_document(&doc_of(&[&f])?)?;
    let ir = iw.get_reader()?;
    iw.close()?;
    let is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(Term::from_text("lyrics", "drug"), 1)?;
    builder.add_at(Term::from_text("lyrics", "drug"), 4)?;
    let pq = builder.build()?;
    // "drug the drug"~1
    assert_equals(&1, &is.search(Arc::new(pq), 4)?.total_hits.value());
    builder.set_slop(1);
    let pq = builder.build()?;
    assert_equals(&3, &is.search(Arc::new(pq), 4)?.total_hits.value());
    builder.set_slop(2);
    let pq = builder.build()?;
    assert_equals(&4, &is.search(Arc::new(pq), 4)?.total_hits.value());
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// LUCENE-3215
///
/// Port of `TestSloppyPhraseQuery.testInfiniteFreq1`.
fn test_infinite_freq1() -> Result<()> {
    let document = "drug druggy drug drug drug";

    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "lyrics",
        document,
        &FieldType::new_from(TextField::type_not_stored()),
    )));
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;

    // Disable assertions as the collector expects a PhraseScorer, not an asserting wrapper.
    let is = LuceneTestCase::new_searcher_assert(Arc::clone(&ir), random().next_boolean(), false);
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(Term::from_text("lyrics", "drug"), 1)?;
    builder.add_at(Term::from_text("lyrics", "drug"), 3)?;
    builder.set_slop(1);
    let pq = builder.build()?;
    // "drug the drug"~1
    assert_sane_scoring(&pq, &is)?;
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// LUCENE-3215
///
/// Port of `TestSloppyPhraseQuery.testInfiniteFreq2`.
fn test_infinite_freq2() -> Result<()> {
    let document = concat!(
        "So much fun to be had in my head ",
        "No more sunshine ",
        "So much fun just lying in my bed ",
        "No more sunshine ",
        "I can't face the sunlight and the dirt outside ",
        "Wanna stay in 666 where this darkness don't lie ",
        "Drug drug druggy ",
        "Got a feeling sweet like honey ",
        "Drug drug druggy ",
        "Need sensation like my baby ",
        "Show me your scars you're so aware ",
        "I'm not barbaric I just care ",
        "Drug drug drug ",
        "I need a reflection to prove I exist ",
        "No more sunshine ",
        "I am a victim of designer blitz ",
        "No more sunshine ",
        "Dance like a robot when you're chained at the knee ",
        "The C.I.A say you're all they'll ever need ",
        "Drug drug druggy ",
        "Got a feeling sweet like honey ",
        "Drug drug druggy ",
        "Need sensation like my baby ",
        "Snort your lines you're so aware ",
        "I'm not barbaric I just care ",
        "Drug drug druggy ",
        "Got a feeling sweet like honey ",
        "Drug drug druggy ",
        "Need sensation like my baby"
    );

    let mut dir = LuceneTestCase::new_directory();

    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "lyrics",
        document,
        &FieldType::new_from(TextField::type_not_stored()),
    )));
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;

    // Disable assertions as the collector expects a PhraseScorer, not an asserting wrapper.
    let is = LuceneTestCase::new_searcher_assert(Arc::clone(&ir), random().next_boolean(), false);

    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(Term::from_text("lyrics", "drug"), 1)?;
    builder.add_at(Term::from_text("lyrics", "drug"), 4)?;
    builder.set_slop(5);
    let pq = builder.build()?;
    // "drug the drug"~5
    assert_sane_scoring(&pq, &is)?;
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}
