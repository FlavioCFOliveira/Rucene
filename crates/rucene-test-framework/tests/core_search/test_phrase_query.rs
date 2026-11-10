//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPhraseQuery.java`.
//!
//! Tests `PhraseQuery`.
//!
//! @see TestPositionIncrement

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::tokenattributes::{CharTermAttribute, PositionIncrementAttribute};
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStream, TokenStreamComponents,
};
use rucene::codecs::term_state::Impact;
use rucene::document::{Document, Store, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::{
    FreqAndNormBuffer, Impacts, ImpactsEnum, ImpactsSource, IndexReader, PostingsEnum,
};
use rucene::search::exact_phrase_matcher::merge_impacts;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_matcher::SharedPostings;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::similarities::{BM25Similarity, ClassicSimilarity};
use rucene::search::term_query::TermQuery;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{query_to_string, BooleanQueryBuilder, DocIdSetIterator, Occur, Query};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::{
    tokenizer_components, CannedTokenStream, MockAnalyzer, Token,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{close_dir, close_reader, doc_of, static_hook, term};
use crate::core_support::{assert_equals, assert_equals_msg, class, random, shuffle, test, IAE};

const NPE: &str = "java.lang.NullPointerException";

/// threshold for comparing floats
pub const SCORE_COMP_THRESH: f32 = 1e-6;

/// The static fields `searcher`, `reader` and `directory`.
struct Statics {
    searcher: Arc<IndexSearcher>,
    reader: Arc<dyn DirectoryReader>,
    directory: TestDirectory,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// The static `searcher`.
fn searcher() -> Arc<IndexSearcher> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("beforeClass ran").searcher)
}

/// `TestPhraseQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestPhraseQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testNotCloseEnough", test_not_close_enough),
            test("testBarelyCloseEnough", test_barely_close_enough),
            test("testExact", test_exact),
            test("testSlop1", test_slop1),
            test("testOrderDoesntMatter", test_order_doesnt_matter),
            test("testMultipleTerms", test_multiple_terms),
            test(
                "testPhraseQueryWithStopAnalyzer",
                test_phrase_query_with_stop_analyzer,
            ),
            test(
                "testPhraseQueryInConjunctionScorer",
                test_phrase_query_in_conjunction_scorer,
            ),
            test("testSlopScoring", test_slop_scoring),
            test("testToString", test_to_string),
            test("testWrappedPhrase", test_wrapped_phrase),
            test("testNonExistingPhrase", test_non_existing_phrase),
            test("testPalyndrome2", test_palyndrome2),
            test("testPalyndrome3", test_palyndrome3),
            test("testEmptyPhraseQuery", test_empty_phrase_query),
            test("testRewrite", test_rewrite),
            test("testZeroPosIncr", test_zero_pos_incr),
            test("testRandomPhrases", test_random_phrases),
            test("testNegativeSlop", test_negative_slop),
            test("testNegativePosition", test_negative_position),
            test("testBackwardPositions", test_backward_positions),
            test("testPhraseQueryMaxTerms", test_phrase_query_max_terms),
            test("testTopPhrases", test_top_phrases),
            test("testMergeImpacts", test_merge_impacts),
            test("testRandomTopDocs", test_random_top_docs),
            test("testNullTerm", test_null_term),
        ],
    )
}

/// The anonymous `Analyzer` of `beforeClass`: a whitespace `MockTokenizer`
/// that does not lowercase, and a position increment gap of 100.
#[derive(Debug)]
struct GapAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for GapAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestPhraseQuery$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Ok(tokenizer_components(Rc::new(RefCell::new(
            MockTokenizer::new(Arc::clone(&WHITESPACE), false)?,
        ))))
    }

    fn get_position_increment_gap(&self, _field_name: &str) -> i32 {
        100
    }
}

fn text_field(name: &str, value: &str) -> Box<rucene::document::Field> {
    Box::new(LuceneTestCase::new_text_field(name, value, Store::YES))
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(GapAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let writer = RandomIndexWriter::new(&*random(), directory.as_directory(), analyzer)?;

    let mut doc = Document::new();
    doc.add(text_field("field", "one two three four five"));
    doc.add(text_field(
        "repeated",
        "this is a repeated field - first part",
    ));
    let repeated_field = text_field("repeated", "second part of a repeated field");
    doc.add(repeated_field);
    doc.add(text_field("palindrome", "one two three two one"));
    writer.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(text_field("nonexist", "phrase exist notexist exist found"));
    writer.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(text_field("nonexist", "phrase exist notexist exist found"));
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = Arc::new(IndexSearcher::new(
        Arc::clone(&reader) as Arc<dyn IndexReader>
    )?);
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        searcher,
        reader,
        directory,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(s) = statics {
        drop(s.searcher);
        close_reader(Some(s.reader))?;
        close_dir(Some(s.directory))?;
    }
    Ok(())
}

/// `new PhraseQuery(slop, field, terms...)`.
fn pq(slop: i32, field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(slop, field, terms)?))
}

/// `searcher.search(query, 1000).scoreDocs.length`.
fn hit_count(searcher: &IndexSearcher, query: &Arc<dyn Query>) -> Result<usize> {
    Ok(searcher.search(Arc::clone(query), 1000)?.score_docs.len())
}

/// `QueryUtils.check(random(), query, searcher)`.
fn check(query: &Arc<dyn Query>, searcher: &IndexSearcher) {
    QueryUtils::check_searcher(&*random(), query, Some(searcher));
}

/// Port of `TestPhraseQuery.testNotCloseEnough`.
fn test_not_close_enough() -> Result<()> {
    let searcher = searcher();
    let query = pq(2, "field", &["one", "five"])?;
    assert_equals(&0, &hit_count(&searcher, &query)?);
    check(&query, &searcher);
    Ok(())
}

/// Port of `TestPhraseQuery.testBarelyCloseEnough`.
fn test_barely_close_enough() -> Result<()> {
    let searcher = searcher();
    let query = pq(3, "field", &["one", "five"])?;
    assert_equals(&1, &hit_count(&searcher, &query)?);
    check(&query, &searcher);
    Ok(())
}

/// Ensures slop of 0 works for exact matches, but not reversed
///
/// Port of `TestPhraseQuery.testExact`.
fn test_exact() -> Result<()> {
    let searcher = searcher();
    // slop is zero by default
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["four", "five"])?);
    assert_equals_msg(Some("exact match"), &1, &hit_count(&searcher, &query)?);
    check(&query, &searcher);

    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["two", "one"])?);
    assert_equals_msg(
        Some("reverse not exact"),
        &0,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);
    Ok(())
}

/// Port of `TestPhraseQuery.testSlop1`.
fn test_slop1() -> Result<()> {
    let searcher = searcher();
    // Ensures slop of 1 works with terms in order.
    let query = pq(1, "field", &["one", "two"])?;
    assert_equals_msg(Some("in order"), &1, &hit_count(&searcher, &query)?);
    check(&query, &searcher);

    // Ensures slop of 1 does not work for phrases out of order;
    // must be at least 2.
    let query = pq(1, "field", &["two", "one"])?;
    assert_equals_msg(
        Some("reversed, slop not 2 or more"),
        &0,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);
    Ok(())
}

/// As long as slop is at least 2, terms can be reversed
///
/// Port of `TestPhraseQuery.testOrderDoesntMatter`.
fn test_order_doesnt_matter() -> Result<()> {
    let searcher = searcher();
    // must be at least two for reverse order match
    let query = pq(2, "field", &["two", "one"])?;
    assert_equals_msg(
        Some("just sloppy enough"),
        &1,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);

    let query = pq(2, "field", &["three", "one"])?;
    assert_equals_msg(
        Some("not sloppy enough"),
        &0,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);
    Ok(())
}

/// slop is the total number of positional moves allowed to line up a phrase
///
/// Port of `TestPhraseQuery.testMultipleTerms`.
fn test_multiple_terms() -> Result<()> {
    let searcher = searcher();
    let query = pq(2, "field", &["one", "three", "five"])?;
    assert_equals_msg(Some("two total moves"), &1, &hit_count(&searcher, &query)?);
    check(&query, &searcher);

    // it takes six moves to match this phrase
    let query = pq(5, "field", &["five", "three", "one"])?;
    assert_equals_msg(
        Some("slop of 5 not close enough"),
        &0,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);

    let query = pq(6, "field", &["five", "three", "one"])?;
    assert_equals_msg(
        Some("slop of 6 just right"),
        &1,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);
    Ok(())
}

/// Port of `TestPhraseQuery.testPhraseQueryWithStopAnalyzer`.
fn test_phrase_query_with_stop_analyzer() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let stop_analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&SIMPLE),
        true,
        Arc::clone(&ENGLISH_STOPSET),
    ));
    let writer = RandomIndexWriter::new_with_config(
        &*random(),
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(stop_analyzer),
    )?;
    let mut doc = Document::new();
    doc.add(text_field("field", "the stop words are here"));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // valid exact phrase query
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["stop", "words"])?);
    assert_equals(&1, &hit_count(&searcher, &query)?);
    check(&query, &searcher);

    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestPhraseQuery.testPhraseQueryInConjunctionScorer`.
fn test_phrase_query_in_conjunction_scorer() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;

    let mut doc = Document::new();
    doc.add(text_field("source", "marketing info"));
    writer.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(text_field("contents", "foobar"));
    doc.add(text_field("source", "marketing info"));
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let phrase_query: Arc<dyn Query> =
        Arc::new(PhraseQuery::from_strings("source", &["marketing", "info"])?);
    assert_equals(&2, &hit_count(&searcher, &phrase_query)?);
    check(&phrase_query, &searcher);

    let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(term("contents", "foobar")));
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(Arc::clone(&term_query), Occur::MUST)?;
    boolean_query.add(Arc::clone(&phrase_query), Occur::MUST)?;
    let bq: Arc<dyn Query> = Arc::new(boolean_query.build());
    assert_equals(&1, &hit_count(&searcher, &bq)?);
    check(&term_query, &searcher);

    drop(searcher);
    reader.close()?;

    let mut iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_open_mode(OpenMode::CREATE)?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(text_field("contents", "map entry woo"));
    writer.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(text_field("contents", "woo map entry"));
    writer.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(text_field("contents", "map foobarword entry woo"));
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(term("contents", "woo")));
    let phrase_query: Arc<dyn Query> =
        Arc::new(PhraseQuery::from_strings("contents", &["map", "entry"])?);

    assert_equals(&3, &hit_count(&searcher, &term_query)?);
    assert_equals(&2, &hit_count(&searcher, &phrase_query)?);

    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(Arc::clone(&term_query), Occur::MUST)?;
    boolean_query.add(Arc::clone(&phrase_query), Occur::MUST)?;
    let bq: Arc<dyn Query> = Arc::new(boolean_query.build());
    assert_equals(&2, &hit_count(&searcher, &bq)?);

    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(Arc::clone(&phrase_query), Occur::MUST)?;
    boolean_query.add(Arc::clone(&term_query), Occur::MUST)?;
    let bq: Arc<dyn Query> = Arc::new(boolean_query.build());
    assert_equals(&2, &hit_count(&searcher, &bq)?);
    let bq: Arc<dyn Query> = Arc::new(boolean_query.build());
    check(&bq, &searcher);

    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestPhraseQuery.testSlopScoring`.
fn test_slop_scoring() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let mut iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    iwc.set_similarity(Arc::new(BM25Similarity::new()))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(text_field("field", "foo firstname lastname foo"));
    writer.add_document(&doc)?;

    let mut doc2 = Document::new();
    doc2.add(text_field("field", "foo firstname zzz lastname foo"));
    writer.add_document(&doc2)?;

    let mut doc3 = Document::new();
    doc3.add(text_field("field", "foo firstname zzz yyy lastname foo"));
    writer.add_document(&doc3)?;

    let reader = writer.get_reader()?;
    writer.close()?;

    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));
    let query = pq(i32::MAX, "field", &["firstname", "lastname"])?;
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals(&3, &hits.len());
    // Make sure that those matches where the terms appear closer to
    // each other get a higher score:
    Assert::assert_equals_double_msg(None, 1.0, f64::from(hits[0].score), 0.01);
    assert_equals(&0, &hits[0].doc);
    Assert::assert_equals_double_msg(None, 0.63, f64::from(hits[1].score), 0.01);
    assert_equals(&1, &hits[1].doc);
    Assert::assert_equals_double_msg(None, 0.47, f64::from(hits[2].score), 0.01);
    assert_equals(&2, &hits[2].doc);
    check(&query, &searcher);
    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestPhraseQuery.testToString`.
fn test_to_string() -> Result<()> {
    let q = PhraseQuery::from_strings("field", &[])?;
    assert_equals("\"\"", query_to_string(&q).as_str());

    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(term("field", "hi"), 1)?;
    let q = builder.build()?;
    assert_equals("field:\"? hi\"", query_to_string(&q).as_str());

    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(term("field", "hi"), 1)?;
    builder.add_at(term("field", "test"), 5)?;
    let q = builder.build()?; // Query "this hi this is a test is"

    assert_equals("field:\"? hi ? ? ? test\"", query_to_string(&q).as_str());

    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(term("field", "hi"), 1)?;
    builder.add_at(term("field", "hello"), 1)?;
    builder.add_at(term("field", "test"), 5)?;
    let q = builder.build()?;
    assert_equals(
        "field:\"? hi|hello ? ? ? test\"",
        query_to_string(&q).as_str(),
    );

    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(term("field", "hi"), 1)?;
    builder.add_at(term("field", "hello"), 1)?;
    builder.add_at(term("field", "test"), 5)?;
    builder.set_slop(5);
    let q = builder.build()?;
    assert_equals(
        "field:\"? hi|hello ? ? ? test\"~5",
        query_to_string(&q).as_str(),
    );
    Ok(())
}

/// Port of `TestPhraseQuery.testWrappedPhrase`.
fn test_wrapped_phrase() -> Result<()> {
    let searcher = searcher();
    let query = pq(100, "repeated", &["first", "part", "second", "part"])?;

    assert_equals_msg(
        Some("slop of 100 just right"),
        &1,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);

    let query = pq(99, "repeated", &["first", "part", "second", "part"])?;

    assert_equals_msg(
        Some("slop of 99 not enough"),
        &0,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);
    Ok(())
}

/// work on two docs like this: "phrase exist notexist exist found"
///
/// Port of `TestPhraseQuery.testNonExistingPhrase`.
fn test_non_existing_phrase() -> Result<()> {
    let searcher = searcher();
    // phrase without repetitions that exists in 2 docs
    let query = pq(2, "nonexist", &["phrase", "notexist", "found"])?;

    assert_equals_msg(
        Some("phrase without repetitions exists in 2 docs"),
        &2,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);

    // phrase with repetitions that exists in 2 docs
    let query = pq(1, "nonexist", &["phrase", "exist", "exist"])?;

    assert_equals_msg(
        Some("phrase with repetitions exists in two docs"),
        &2,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);

    // phrase I with repetitions that does not exist in any doc
    let query = pq(1000, "nonexist", &["phrase", "notexist", "phrase"])?;

    assert_equals_msg(
        Some("nonexisting phrase with repetitions does not exist in any doc"),
        &0,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);

    // phrase II with repetitions that does not exist in any doc
    let query = pq(1000, "nonexist", &["phrase", "exist", "exist", "exist"])?;

    assert_equals_msg(
        Some("nonexisting phrase with repetitions does not exist in any doc"),
        &0,
        &hit_count(&searcher, &query)?,
    );
    check(&query, &searcher);
    Ok(())
}

/// Working on a 2 fields like this: Field("field", "one two three four five") Field("palindrome",
/// "one two three two one") Phrase of size 2 occuriong twice, once in order and once in reverse,
/// because doc is a palyndrome, is counted twice. Also, in this case order in query does not
/// matter. Also, when an exact match is found, both sloppy scorer and exact scorer scores the
/// same.
///
/// Port of `TestPhraseQuery.testPalyndrome2`.
fn test_palyndrome2() -> Result<()> {
    let searcher = searcher();

    // search on non palyndrome, find phrase with no slop, using exact phrase scorer
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["two", "three"])?); // to use exact phrase scorer
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals_msg(
        Some("phrase found with exact phrase scorer"),
        &1,
        &hits.len(),
    );
    let score0 = hits[0].score;
    check(&query, &searcher);

    // search on non palyndrome, find phrase with slop 2, though no slop required here.
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["two", "three"])?); // to use sloppy scorer
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals_msg(Some("just sloppy enough"), &1, &hits.len());
    let score1 = hits[0].score;
    Assert::assert_equals_float_msg(
        Some("exact scorer and sloppy scorer score the same when slop does not matter"),
        score0,
        score1,
        SCORE_COMP_THRESH,
    );
    check(&query, &searcher);

    // search ordered in palyndrome, find it twice
    let query = pq(2, "palindrome", &["two", "three"])?; // must be at least two for both ordered and reversed to match
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals_msg(Some("just sloppy enough"), &1, &hits.len());
    check(&query, &searcher);

    // commented out for sloppy-phrase efficiency (issue 736) - see SloppyPhraseScorer.phraseFreq().
    // assertTrue("ordered scores higher in palindrome",score1+SCORE_COMP_THRESH<score2);

    // search reveresed in palyndrome, find it twice
    let query = pq(2, "palindrome", &["three", "two"])?; // must be at least two for both ordered and reversed to match
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals_msg(Some("just sloppy enough"), &1, &hits.len());
    check(&query, &searcher);

    // commented out for sloppy-phrase efficiency (issue 736) - see SloppyPhraseScorer.phraseFreq().
    // assertTrue("reversed scores higher in palindrome",score1+SCORE_COMP_THRESH<score3);
    // assertEquals("ordered or reversed does not matter",score2, score3, SCORE_COMP_THRESH);
    Ok(())
}

/// Working on a 2 fields like this: Field("field", "one two three four five") Field("palindrome",
/// "one two three two one") Phrase of size 3 occuriong twice, once in order and once in reverse,
/// because doc is a palyndrome, is counted twice. Also, in this case order in query does not
/// matter. Also, when an exact match is found, both sloppy scorer and exact scorer scores the
/// same.
///
/// Port of `TestPhraseQuery.testPalyndrome3`.
fn test_palyndrome3() -> Result<()> {
    let searcher = searcher();

    // search on non palyndrome, find phrase with no slop, using exact phrase scorer
    // slop=0 to use exact phrase scorer
    let query = pq(0, "field", &["one", "two", "three"])?;
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals_msg(
        Some("phrase found with exact phrase scorer"),
        &1,
        &hits.len(),
    );
    let score0 = hits[0].score;
    check(&query, &searcher);

    // just make sure no exc:
    searcher.explain(Arc::clone(&query), 0)?;

    // search on non palyndrome, find phrase with slop 3, though no slop required here.
    // slop=4 to use sloppy scorer
    let query = pq(4, "field", &["one", "two", "three"])?;
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals_msg(Some("just sloppy enough"), &1, &hits.len());
    let score1 = hits[0].score;
    Assert::assert_equals_float_msg(
        Some("exact scorer and sloppy scorer score the same when slop does not matter"),
        score0,
        score1,
        SCORE_COMP_THRESH,
    );
    check(&query, &searcher);

    // search ordered in palyndrome, find it twice
    // slop must be at least four for both ordered and reversed to match
    let query = pq(4, "palindrome", &["one", "two", "three"])?;
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;

    // just make sure no exc:
    searcher.explain(Arc::clone(&query), 0)?;

    assert_equals_msg(Some("just sloppy enough"), &1, &hits.len());
    check(&query, &searcher);

    // commented out for sloppy-phrase efficiency (issue 736) - see SloppyPhraseScorer.phraseFreq().
    // assertTrue("ordered scores higher in palindrome",score1+SCORE_COMP_THRESH<score2);

    // search reveresed in palyndrome, find it twice
    // must be at least four for both ordered and reversed to match
    let query = pq(4, "palindrome", &["three", "two", "one"])?;
    let hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    assert_equals_msg(Some("just sloppy enough"), &1, &hits.len());
    check(&query, &searcher);

    // commented out for sloppy-phrase efficiency (issue 736) - see SloppyPhraseScorer.phraseFreq().
    // assertTrue("reversed scores higher in palindrome",score1+SCORE_COMP_THRESH<score3);
    // assertEquals("ordered or reversed does not matter",score2, score3, SCORE_COMP_THRESH);
    Ok(())
}

/// LUCENE-1280
///
/// Port of `TestPhraseQuery.testEmptyPhraseQuery`.
fn test_empty_phrase_query() -> Result<()> {
    let mut q2 = BooleanQueryBuilder::new();
    q2.add(
        Arc::new(PhraseQuery::from_strings("field", &[])?),
        Occur::MUST,
    )?;
    query_to_string(&q2.build());
    Ok(())
}

/// test that a single term is rewritten to a term query
///
/// Port of `TestPhraseQuery.testRewrite`.
fn test_rewrite() -> Result<()> {
    let searcher = searcher();
    let pq = PhraseQuery::from_strings("foo", &["bar"])?;
    // `None` is "rewrites to itself", a PhraseQuery.
    let rewritten = pq.rewrite(&searcher)?;
    Assert::assert_true(
        rewritten
            .as_ref()
            .is_some_and(|q| q.as_any().is::<TermQuery>()),
    );
    Ok(())
}

/// A `Token` holding `text` at position increment `pos_incr`.
fn make_token(text: &str, pos_incr: i32) -> Token {
    let mut t = Token::new();
    t.append_string(Some(text));
    t.set_position_increment(pos_incr);
    t
}

/// Tests PhraseQuery with terms at the same position in the query.
///
/// Port of `TestPhraseQuery.testZeroPosIncr`.
fn test_zero_pos_incr() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let tokens = vec![make_token("a", 1), make_token("aa", 0), make_token("b", 1)];

    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let stream: Rc<RefCell<dyn TokenStream>> =
        Rc::new(RefCell::new(CannedTokenStream::new(tokens)));
    doc.add(Box::new(TextField::new_with_token_stream("field", stream)?));
    writer.add_document(&doc)?;
    let r = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    // Sanity check; simple "a b" phrase:
    let mut pq_builder = PhraseQueryBuilder::new();
    pq_builder.add_at(term("field", "a"), 0)?;
    pq_builder.add_at(term("field", "b"), 1)?;
    assert_equals(&1, &searcher.count(Arc::new(pq_builder.build()?))?);

    // Now with "a|aa b"
    let mut pq_builder = PhraseQueryBuilder::new();
    pq_builder.add_at(term("field", "a"), 0)?;
    pq_builder.add_at(term("field", "aa"), 0)?;
    pq_builder.add_at(term("field", "b"), 1)?;
    assert_equals(&1, &searcher.count(Arc::new(pq_builder.build()?))?);

    // Now with "a|z b" which should not match; this isn't a MultiPhraseQuery
    let mut pq_builder = PhraseQueryBuilder::new();
    pq_builder.add_at(term("field", "a"), 0)?;
    pq_builder.add_at(term("field", "z"), 0)?;
    pq_builder.add_at(term("field", "b"), 1)?;
    assert_equals(&0, &searcher.count(Arc::new(pq_builder.build()?))?);

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPhraseQuery.testRandomPhrases`.
fn test_random_phrases() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));

    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Arc::clone(&analyzer));
    iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut docs: Vec<Vec<String>> = Vec::new();
    // `Document d` holding `Field f`: rebuilt from `f` for each `addDocument`.
    let mut f = LuceneTestCase::new_text_field("f", "", Store::NO);

    let r = random();

    let num_docs = LuceneTestCase::at_least(10);
    for _i in 0..num_docs {
        // at night, must be > 4096 so it spans multiple chunks
        let term_count = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(4097)
        } else {
            LuceneTestCase::at_least(200)
        };

        let mut doc: Vec<String> = Vec::new();

        let mut sb = String::new();
        while (doc.len() as i32) < term_count {
            if r.next_int_bound(5) == 1 || docs.is_empty() {
                // make new non-empty-string term
                let term = loop {
                    let term = TestUtil::random_unicode_string(&*r);
                    if !term.is_empty() {
                        break term;
                    }
                };
                let ts = analyzer.token_stream_from_str("ignore", &term)?;
                let mut ts = ts.borrow_mut();
                let term_attr = Att::add(ts.attribute_source_mut(), char_term_id())?;
                ts.reset()?;
                while ts.increment_token()? {
                    let text = term_attr.term_string();
                    sb.push_str(&text);
                    sb.push(' ');
                    doc.push(text);
                }
                ts.end()?;
                ts.close()?;
            } else {
                // pick existing sub-phrase
                let last_doc = &docs[r.next_int_bound(docs.len() as i32) as usize];
                let len = TestUtil::next_int(&*r, 1, 10);
                let start = r.next_int_bound(last_doc.len() as i32 - len);
                for k in start..start + len {
                    let t = last_doc[k as usize].clone();
                    sb.push_str(&t);
                    sb.push(' ');
                    doc.push(t);
                }
            }
        }
        docs.push(doc);
        f.set_string_value(sb)?;
        w.add_document(&doc_of(&[&f])?)?;
    }

    let reader = w.get_reader()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&reader));
    w.close()?;

    // now search
    let num = LuceneTestCase::at_least(3);
    for i in 0..num {
        let doc_id = r.next_int_bound(docs.len() as i32);
        let doc = &docs[doc_id as usize];

        let num_term = TestUtil::next_int(&*r, 2, 20);
        let start = r.next_int_bound(doc.len() as i32 - num_term);
        let mut builder = PhraseQueryBuilder::new();
        let mut sb = String::new();
        for t in start..start + num_term {
            builder.add_at(term("f", &doc[t as usize]), t)?;
            sb.push_str(&doc[t as usize]);
            sb.push(' ');
        }
        let pq = builder.build()?;

        let hits = s.search(Arc::new(pq), num_docs)?;
        let mut found = false;
        for j in 0..hits.score_docs.len() {
            if hits.score_docs[j].doc == doc_id {
                found = true;
                break;
            }
        }

        Assert::assert_true_msg(
            &format!("phrase '{sb}' not found; start={start}, it={i}, expected doc {doc_id}"),
            found,
        );
    }

    drop(s);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPhraseQuery.testNegativeSlop`.
fn test_negative_slop() -> Result<()> {
    expect_throws(IAE, || PhraseQuery::with_slop(-2, "field", &["two", "one"]));
    Ok(())
}

/// Port of `TestPhraseQuery.testNegativePosition`.
fn test_negative_position() -> Result<()> {
    let mut builder = PhraseQueryBuilder::new();
    expect_throws(IAE, || {
        builder.add_at(term("field", "two"), -42).map(|_| ())
    });
    Ok(())
}

/// Port of `TestPhraseQuery.testBackwardPositions`.
fn test_backward_positions() -> Result<()> {
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(term("field", "one"), 1)?;
    builder.add_at(term("field", "two"), 5)?;
    expect_throws(IAE, || {
        builder.add_at(term("field", "three"), 4).map(|_| ())
    });
    Ok(())
}

/// Port of `TestPhraseQuery.testPhraseQueryMaxTerms`.
fn test_phrase_query_max_terms() -> Result<()> {
    let mut builder = PhraseQueryBuilder::new();
    let term_threshold = 5;
    builder.set_max_terms(term_threshold);
    for i in 0..term_threshold {
        builder.add_at(term("field", &format!("one{i}")), i)?;
    }
    expect_throws(IAE, || {
        builder
            .add_at(term("field", "three"), term_threshold)
            .map(|_| ())
    });
    Ok(())
}

const DOCS: [&str; 6] = [
    "a b c d e f g h",
    "b c b",
    "c d d d e f g b",
    "c b a b c",
    "a a b b c c d d",
    "a b c d a b c d a b c d",
];

/// Port of `TestPhraseQuery.testTopPhrases`.
fn test_top_phrases() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut docs: Vec<&str> = DOCS.to_vec();
    shuffle(&mut docs, &*random());
    for value in DOCS {
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new("f", value.to_string(), Store::NO)?));
        w.add_document(&doc)?;
    }
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let queries: Vec<Arc<dyn Query>> = vec![
        Arc::new(PhraseQuery::from_strings("f", &["b", "c"])?), // common phrase
        Arc::new(PhraseQuery::from_strings("f", &["e", "f"])?), // always appear next to each other
        Arc::new(PhraseQuery::from_strings("f", &["d", "d"])?), // repeated term
    ];
    for query in queries {
        for top_n in 1..=2 {
            let collector_manager = TopScoreDocCollectorManager::new(top_n, None, i32::MAX)?;
            let hits1 = searcher
                .search_with_manager(Arc::clone(&query), &collector_manager)?
                .score_docs;

            let collector_manager = TopScoreDocCollectorManager::new(top_n, None, 1)?;
            let top_docs2 = searcher.search_with_manager(Arc::clone(&query), &collector_manager)?;
            let hits2 = top_docs2.score_docs;

            Assert::assert_true_msg(&query_to_string(&*query), !hits1.is_empty());
            CheckHits::check_equal(&*query, &hits1, &hits2);
        }
    }
    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// `new Impact(freq, norm)`.
fn imp(freq: i32, norm: i64) -> Impact {
    Impact::new(freq, norm)
}

/// Port of `TestPhraseQuery.testMergeImpacts`.
fn test_merge_impacts() -> Result<()> {
    let impacts1 = DummyImpactsEnum::new(1000);
    let impacts2 = DummyImpactsEnum::new(2000);
    let mut merged_impacts = merge_impacts(vec![
        SharedPostings::new(Box::new(impacts1.clone())),
        SharedPostings::new(Box::new(impacts2.clone())),
    ]);

    impacts1.reset(
        vec![
            vec![imp(3, 10), imp(5, 12), imp(8, 13)],
            vec![imp(3, 10), imp(5, 11), imp(8, 13), imp(12, 14)],
        ],
        vec![110, 945],
    );

    // Merge with empty impacts
    impacts2.reset(vec![], vec![]);
    assert_impacts(
        &[
            vec![imp(3, 10), imp(5, 12), imp(8, 13)],
            vec![imp(3, 10), imp(5, 11), imp(8, 13), imp(12, 14)],
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );

    // Merge with dummy impacts
    impacts2.reset(vec![vec![imp(i32::MAX, 1)]], vec![5000]);
    assert_impacts(
        &[
            vec![imp(3, 10), imp(5, 12), imp(8, 13)],
            vec![imp(3, 10), imp(5, 11), imp(8, 13), imp(12, 14)],
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );

    // Merge with dummy impacts that we don't special case
    impacts2.reset(vec![vec![imp(i32::MAX, 2)]], vec![5000]);
    assert_impacts(
        &[
            vec![imp(3, 10), imp(5, 12), imp(8, 13)],
            vec![imp(3, 10), imp(5, 11), imp(8, 13), imp(12, 14)],
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );

    // First level of impacts2 doesn't cover the first level of impacts1
    impacts2.reset(
        vec![
            vec![imp(2, 10), imp(6, 13)],
            vec![imp(3, 9), imp(5, 11), imp(7, 13)],
        ],
        vec![90, 1000],
    );
    assert_impacts(
        &[
            vec![imp(3, 10), imp(5, 12), imp(7, 13)],
            vec![imp(3, 10), imp(5, 11), imp(7, 13)],
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );

    // Second level of impacts2 doesn't cover the first level of impacts1
    impacts2.reset(
        vec![
            vec![imp(2, 10), imp(6, 11)],
            vec![imp(3, 9), imp(5, 11), imp(7, 13)],
        ],
        vec![150, 900],
    );
    assert_impacts(
        &[
            vec![imp(2, 10), imp(3, 11), imp(5, 12), imp(6, 13)],
            vec![imp(3, 10), imp(5, 11), imp(8, 13), imp(12, 14)], // same as impacts1
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );

    impacts2.reset(
        vec![
            vec![imp(4, 10), imp(9, 13)],
            vec![
                imp(1, 1),
                imp(4, 10),
                imp(5, 11),
                imp(8, 13),
                imp(12, 14),
                imp(13, 15),
            ],
        ],
        vec![113, 950],
    );
    assert_impacts(
        &[
            vec![imp(3, 10), imp(4, 12), imp(8, 13)],
            vec![imp(3, 10), imp(5, 11), imp(8, 13), imp(12, 14)],
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );

    // Make sure negative norms are treated as unsigned
    impacts1.reset(
        vec![
            vec![imp(3, 10), imp(5, -10), imp(8, -5)],
            vec![imp(3, 10), imp(5, -15), imp(8, -5), imp(12, -3)],
        ],
        vec![110, 945],
    );
    impacts2.reset(
        vec![
            vec![imp(2, 10), imp(12, -4)],
            vec![imp(3, 9), imp(12, -4), imp(20, -1)],
        ],
        vec![150, 960],
    );
    assert_impacts(
        &[
            vec![imp(2, 10), imp(8, -4)],
            vec![imp(3, 10), imp(8, -4), imp(12, -3)],
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );
    Ok(())
}

/// `private static void assertEquals(Impact[][] impacts, int[] docIdUpTo, Impacts actual)`.
fn assert_impacts(impacts: &[Vec<Impact>], doc_id_up_to: &[i32], actual: &dyn Impacts) {
    assert_equals(&(impacts.len() as i32), &actual.num_levels());
    for i in 0..impacts.len() {
        assert_equals(&doc_id_up_to[i], &actual.doc_id_up_to(i as i32));
        assert_equals(&impacts[i], &copy_of(&actual.get_impacts(i as i32)));
    }
}

/// `private static List<Impact> copyOf(FreqAndNormBuffer buffer)`.
fn copy_of(buffer: &FreqAndNormBuffer) -> Vec<Impact> {
    let mut copy = Vec::new();
    for i in 0..buffer.size {
        copy.push(Impact::new(buffer.freqs[i], buffer.norms[i]));
    }
    copy
}

/// The `impacts` and `docIdUpTo` fields of `DummyImpactsEnum`, which the
/// test resets after handing the enum to `mergeImpacts`.
#[derive(Debug, Default)]
struct DummyState {
    impacts: Vec<Vec<Impact>>,
    doc_id_up_to: Vec<i32>,
}

/// `private static class DummyImpactsEnum extends ImpactsEnum`.
///
/// The test keeps a reference to each enum it gives `mergeImpacts`, to reset
/// it; the clones share the state.
#[derive(Debug, Clone)]
struct DummyImpactsEnum {
    cost: i64,
    state: Rc<RefCell<DummyState>>,
}

impl DummyImpactsEnum {
    fn new(cost: i64) -> Self {
        Self {
            cost,
            state: Rc::new(RefCell::new(DummyState::default())),
        }
    }

    fn reset(&self, impacts: Vec<Vec<Impact>>, doc_id_up_to: Vec<i32>) {
        let mut state = self.state.borrow_mut();
        state.impacts = impacts;
        state.doc_id_up_to = doc_id_up_to;
    }
}

fn uoe() -> LuceneError {
    LuceneError::UnsupportedOperation(String::new())
}

fn throw_uoe() -> ! {
    randomizedtesting::jdk::exception::throw_without_message(
        "java.lang.UnsupportedOperationException",
    )
}

impl std::fmt::Display for DummyImpactsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::core_search::support::object_to_string(
            "org.apache.lucene.search.TestPhraseQuery$DummyImpactsEnum",
            self,
        ))
    }
}

impl DocIdSetIterator for DummyImpactsEnum {
    fn doc_id(&self) -> i32 {
        throw_uoe()
    }

    fn next_doc(&mut self) -> Result<i32> {
        Err(uoe())
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(uoe())
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl PostingsEnum for DummyImpactsEnum {
    fn freq(&self) -> Result<i32> {
        Err(uoe())
    }

    fn next_position(&mut self) -> Result<i32> {
        Err(uoe())
    }

    fn start_offset(&self) -> i32 {
        throw_uoe()
    }

    fn end_offset(&self) -> i32 {
        throw_uoe()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Err(uoe())
    }
}

impl ImpactsSource for DummyImpactsEnum {
    fn advance_shallow(&mut self, _target: i32) -> Result<()> {
        Err(uoe())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        let state = self.state.borrow();
        Ok(Box::new(DummyImpacts {
            impacts: state.impacts.clone(),
            doc_id_up_to: state.doc_id_up_to.clone(),
        }))
    }
}

impl ImpactsEnum for DummyImpactsEnum {}

/// The anonymous `Impacts` of `DummyImpactsEnum.getImpacts()`.
struct DummyImpacts {
    impacts: Vec<Vec<Impact>>,
    doc_id_up_to: Vec<i32>,
}

impl Impacts for DummyImpacts {
    fn num_levels(&self) -> i32 {
        self.impacts.len() as i32
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        self.doc_id_up_to[level as usize]
    }

    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer {
        let mut buffer = FreqAndNormBuffer::new();
        self.impacts[level as usize]
            .iter()
            .for_each(|impact| buffer.add(impact.freq, impact.norm));
        buffer
    }
}

/// Port of `TestPhraseQuery.testRandomTopDocs`.
fn test_random_top_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(128 * 8 * 8 * 3)
    } else {
        LuceneTestCase::at_least(100)
    }; // at night, make sure some terms have skip data
    for _i in 0..num_docs {
        let mut doc = Document::new();
        let num_terms = random().next_int_bound(1 << random().next_int_bound(5));
        let text = (0..num_terms)
            .map(|_index| {
                if random().next_boolean() {
                    "a"
                } else if random().next_boolean() {
                    "b"
                } else {
                    "c"
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        doc.add(Box::new(TextField::new("foo", text, Store::NO)?));
        w.add_document(&doc)?;
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    for first_term in ["a", "b", "c"] {
        for second_term in ["a", "b", "c"] {
            let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_bytes(
                "foo",
                &[
                    LuceneTestCase::new_bytes_ref_str(first_term),
                    LuceneTestCase::new_bytes_ref_str(second_term),
                ],
            )?);

            let complete_manager = TopScoreDocCollectorManager::new(10, None, i32::MAX)?; // COMPLETE
            let top_scores_manager = TopScoreDocCollectorManager::new(10, None, 10)?; // TOP_SCORES

            let complete = searcher.search_with_manager(Arc::clone(&query), &complete_manager)?;
            let top_scores =
                searcher.search_with_manager(Arc::clone(&query), &top_scores_manager)?;
            CheckHits::check_equal(&*query, &complete.score_docs, &top_scores.score_docs);

            let mut filtered = BooleanQueryBuilder::new();
            filtered.add(Arc::clone(&query), Occur::MUST)?;
            filtered.add(Arc::new(TermQuery::new(term("foo", "b"))), Occur::FILTER)?;
            let filtered_query: Arc<dyn Query> = Arc::new(filtered.build());

            let complete_manager = TopScoreDocCollectorManager::new(10, None, i32::MAX)?; // COMPLETE
            let top_scores_manager = TopScoreDocCollectorManager::new(10, None, 10)?; // TOP_SCORES

            let complete =
                searcher.search_with_manager(Arc::clone(&filtered_query), &complete_manager)?;
            let top_scores =
                searcher.search_with_manager(Arc::clone(&filtered_query), &top_scores_manager)?;
            CheckHits::check_equal(&*query, &complete.score_docs, &top_scores.score_docs);
        }
    }
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPhraseQuery.testNullTerm`.
fn test_null_term() -> Result<()> {
    let e = expect_throws(NPE, || {
        PhraseQueryBuilder::new().add_nullable(None).map(|_| ())
    });
    Assert::assert_equals_str(
        None,
        Some("Cannot add a null term to PhraseQuery"),
        e.message(),
    );

    let e = expect_throws(NPE, || {
        PhraseQuery::from_nullable_bytes("field", &[None::<BytesRef>])
    });
    Assert::assert_equals_str(
        None,
        Some("Cannot add a null term to PhraseQuery"),
        e.message(),
    );

    let e = expect_throws(NPE, || PhraseQuery::from_nullable_strings("field", &[None]));
    Assert::assert_equals_str(
        None,
        Some("Cannot add a null term to PhraseQuery"),
        e.message(),
    );
    Ok(())
}
