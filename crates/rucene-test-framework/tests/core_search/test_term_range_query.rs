//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTermRangeQuery.java`.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenStreamComponents, Tokenizer,
    TokenizerLogic,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{RewriteMethod, TopTermsScoringBooleanQueryRewrite};
use rucene::search::term_query::TermQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::{query_to_string, BooleanQuery, Query};
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::{tokenizer_components, MockAnalyzer};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, assert_equals_msg, random};

/// The instance of `TestTermRangeQuery`.
#[derive(Default)]
pub struct TermRangeQueryTest {
    doc_count: i32,
    dir: Option<TestDirectory>,
}

/// `TestTermRangeQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<TermRangeQueryTest>(
        "org.apache.lucene.search.TestTermRangeQuery",
        vec![
            set_up(term_range_query_set_up),
            tear_down(term_range_query_tear_down),
            ftest("testExclusive", test_exclusive),
            ftest("testInclusive", test_inclusive),
            ftest("testAllDocs", test_all_docs),
            ftest("testTopTermsRewrite", test_top_terms_rewrite),
            ftest("testEqualsHashcode", test_equals_hashcode),
            ftest("testExclusiveLowerNull", test_exclusive_lower_null),
            ftest("testInclusiveLowerNull", test_inclusive_lower_null),
        ],
    )
}

/// `setUp()`.
fn term_range_query_set_up(f: &mut TermRangeQueryTest) -> Result<()> {
    f.dir = Some(LuceneTestCase::new_directory());
    Ok(())
}

/// `tearDown()`.
fn term_range_query_tear_down(f: &mut TermRangeQueryTest) -> Result<()> {
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

impl TermRangeQueryTest {
    fn dir(&self) -> &TestDirectory {
        self.dir.as_ref().expect("setUp ran")
    }

    /// `DirectoryReader.open(dir)` and `newSearcher(reader)`.
    fn open(&self) -> Result<(Arc<dyn DirectoryReader>, IndexSearcher)> {
        let reader = directory_reader::open(self.dir().as_directory())?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        Ok((reader, searcher))
    }

    /// `private void initializeIndex(String[] values)`.
    fn initialize_index(&mut self, values: &[&str]) -> Result<()> {
        let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&WHITESPACE),
            false,
        ));
        self.initialize_index_with(values, analyzer)
    }

    /// `private void initializeIndex(String[] values, Analyzer analyzer)`.
    fn initialize_index_with(
        &mut self,
        values: &[&str],
        analyzer: Arc<dyn Analyzer>,
    ) -> Result<()> {
        let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
        config.set_open_mode(OpenMode::CREATE)?;
        let writer = IndexWriter::new(self.dir().as_directory(), config)?;
        for value in values {
            self.insert_doc(&writer, value)?;
        }
        writer.close()
    }

    /// shouldnt create an analyzer for every doc?
    ///
    /// `private void addDoc(String content)`.
    fn add_doc(&mut self, content: &str) -> Result<()> {
        let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&WHITESPACE),
            false,
        ));
        let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
        config.set_open_mode(OpenMode::APPEND)?;
        let writer = IndexWriter::new(self.dir().as_directory(), config)?;
        self.insert_doc(&writer, content)?;
        writer.close()
    }

    /// `private void insertDoc(IndexWriter writer, String content)`.
    fn insert_doc(&mut self, writer: &IndexWriter, content: &str) -> Result<()> {
        let mut doc = Document::new();

        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &format!("id{}", self.doc_count),
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            content,
            Store::NO,
        )));

        writer.add_document(&doc)?;
        self.doc_count += 1;
        Ok(())
    }
}

/// `assertEquals(message, expected, actual)`.
fn eq_msg<T: PartialEq + std::fmt::Debug>(message: &str, expected: &T, actual: &T) {
    assert_equals_msg(Some(message), expected, actual);
}

/// `TermRangeQuery.newStringRange(field, lower, upper, includeLower, includeUpper)`.
fn string_range(
    field: &str,
    lower: Option<&str>,
    upper: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(TermRangeQuery::new_string_range(
        field,
        lower,
        upper,
        include_lower,
        include_upper,
    )?))
}

/// `searcher.search(query, 1000).scoreDocs.length`.
fn hits(searcher: &IndexSearcher, query: &Arc<dyn Query>) -> Result<usize> {
    Ok(searcher.search(Arc::clone(query), 1000)?.score_docs.len())
}

/// `searcher.search(query, 1000).totalHits.value()`.
fn num_hits(searcher: &IndexSearcher, query: &Arc<dyn Query>) -> Result<i64> {
    Ok(searcher.search(Arc::clone(query), 1000)?.total_hits.value())
}

/// Port of `TestTermRangeQuery.testExclusive`.
fn test_exclusive(f: &mut TermRangeQueryTest) -> Result<()> {
    let query = string_range("content", Some("A"), Some("C"), false, false)?;
    f.initialize_index(&["A", "B", "C", "D"])?;
    let (reader, searcher) = f.open()?;
    eq_msg("A,B,C,D, only B in range", &1, &hits(&searcher, &query)?);
    drop(searcher);
    reader.close()?;

    f.initialize_index(&["A", "B", "D"])?;
    let (reader, searcher) = f.open()?;
    eq_msg("A,B,D, only B in range", &1, &hits(&searcher, &query)?);
    drop(searcher);
    reader.close()?;

    f.add_doc("C")?;
    let (reader, searcher) = f.open()?;
    eq_msg(
        "C added, still only B in range",
        &1,
        &hits(&searcher, &query)?,
    );
    drop(searcher);
    reader.close()
}

/// Port of `TestTermRangeQuery.testInclusive`.
fn test_inclusive(f: &mut TermRangeQueryTest) -> Result<()> {
    let query = string_range("content", Some("A"), Some("C"), true, true)?;

    f.initialize_index(&["A", "B", "C", "D"])?;
    let (reader, searcher) = f.open()?;
    eq_msg("A,B,C,D - A,B,C in range", &3, &hits(&searcher, &query)?);
    drop(searcher);
    reader.close()?;

    f.initialize_index(&["A", "B", "D"])?;
    let (reader, searcher) = f.open()?;
    eq_msg("A,B,D - A and B in range", &2, &hits(&searcher, &query)?);
    drop(searcher);
    reader.close()?;

    f.add_doc("C")?;
    let (reader, searcher) = f.open()?;
    eq_msg("C added - A, B, C in range", &3, &hits(&searcher, &query)?);
    drop(searcher);
    reader.close()
}

/// Port of `TestTermRangeQuery.testAllDocs`.
fn test_all_docs(f: &mut TermRangeQueryTest) -> Result<()> {
    f.initialize_index(&["A", "B", "C", "D"])?;
    let (reader, searcher) = f.open()?;

    let query: Arc<dyn Query> = Arc::new(TermRangeQuery::new("content", None, None, true, true)?);
    assert_equals(&4, &hits(&searcher, &query)?);

    let query = string_range("content", Some(""), None, true, true)?;
    assert_equals(&4, &hits(&searcher, &query)?);

    let query = string_range("content", Some(""), None, true, false)?;
    assert_equals(&4, &hits(&searcher, &query)?);

    // and now another one
    let query = string_range("content", Some("B"), None, true, true)?;
    assert_equals(&3, &hits(&searcher, &query)?);
    drop(searcher);
    reader.close()
}

/// This test should not be here, but it tests the fuzzy query rewrite mode
/// (TOP_TERMS_SCORING_BOOLEAN_REWRITE) with constant score and checks, that
/// only the lower end of terms is put into the range
///
/// Port of `TestTermRangeQuery.testTopTermsRewrite`.
fn test_top_terms_rewrite(f: &mut TermRangeQueryTest) -> Result<()> {
    f.initialize_index(&["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K"])?;

    let (reader, searcher) = f.open()?;
    let rewrite_method: Arc<dyn RewriteMethod> =
        Arc::new(TopTermsScoringBooleanQueryRewrite::new(50));
    let query: Arc<dyn Query> = Arc::new(TermRangeQuery::new_string_range_with_rewrite_method(
        "content",
        Some("B"),
        Some("J"),
        true,
        true,
        rewrite_method,
    )?);
    check_boolean_terms(
        &searcher,
        &query,
        &["B", "C", "D", "E", "F", "G", "H", "I", "J"],
    )?;

    let saved_clause_count = IndexSearcher::get_max_clause_count();
    let result = (|| -> Result<()> {
        IndexSearcher::set_max_clause_count(3)?;
        check_boolean_terms(&searcher, &query, &["B", "C", "D"])
    })();
    IndexSearcher::set_max_clause_count(saved_clause_count)?;
    result?;
    drop(searcher);
    reader.close()
}

/// `private void checkBooleanTerms(IndexSearcher searcher, TermRangeQuery query, String... terms)`.
fn check_boolean_terms(
    searcher: &IndexSearcher,
    query: &Arc<dyn Query>,
    terms: &[&str],
) -> Result<()> {
    let rewritten = searcher.rewrite(Arc::clone(query))?;
    let bq = rewritten
        .as_any()
        .downcast_ref::<BooleanQuery>()
        .unwrap_or_else(|| {
            panic!("java.lang.ClassCastException: rewritten query is not a BooleanQuery")
        });
    let mut allowed_terms: HashSet<String> = terms.iter().map(|t| t.to_string()).collect();
    assert_equals(&allowed_terms.len(), &bq.clauses().len());
    for c in bq.clauses() {
        Assert::assert_true(c.query().as_any().is::<TermQuery>());
        let tq = c
            .query()
            .as_any()
            .downcast_ref::<TermQuery>()
            .expect("TermQuery");
        let term = tq.get_term().text();
        Assert::assert_true_msg(
            &format!("invalid term: {term}"),
            allowed_terms.contains(&term),
        );
        allowed_terms.remove(&term); // remove to fail on double terms
    }
    assert_equals(&0, &allowed_terms.len());
    Ok(())
}

/// `assertEquals(message, expected, actual)` over `Query.equals`.
fn assert_query_equals_msg(message: &str, expected: &Arc<dyn Query>, actual: &Arc<dyn Query>) {
    if !expected.query_eq(&**actual) {
        Assert::fail(&format!(
            "{message} expected:<{}> but was:<{}>",
            query_to_string(&**expected),
            query_to_string(&**actual)
        ));
    }
}

/// Port of `TestTermRangeQuery.testEqualsHashcode`.
fn test_equals_hashcode(_f: &mut TermRangeQueryTest) -> Result<()> {
    let query = string_range("content", Some("A"), Some("C"), true, true)?;

    let other = string_range("content", Some("A"), Some("C"), true, true)?;

    assert_query_equals_msg("query equals itself is true", &query, &query);
    assert_query_equals_msg("equivalent queries are equal", &query, &other);
    eq_msg(
        "hashcode must return same value when equals is true",
        &query.query_hash(),
        &other.query_hash(),
    );

    let other = string_range("notcontent", Some("A"), Some("C"), true, true)?;
    Assert::assert_false_msg("Different fields are not equal", query.query_eq(&*other));

    let other = string_range("content", Some("X"), Some("C"), true, true)?;
    Assert::assert_false_msg(
        "Different lower terms are not equal",
        query.query_eq(&*other),
    );

    let other = string_range("content", Some("A"), Some("Z"), true, true)?;
    Assert::assert_false_msg(
        "Different upper terms are not equal",
        query.query_eq(&*other),
    );

    let query = string_range("content", None, Some("C"), true, true)?;
    let other = string_range("content", None, Some("C"), true, true)?;
    assert_query_equals_msg(
        "equivalent queries with null lowerterms are equal()",
        &query,
        &other,
    );
    eq_msg(
        "hashcode must return same value when equals is true",
        &query.query_hash(),
        &other.query_hash(),
    );

    let query = string_range("content", Some("C"), None, true, true)?;
    let other = string_range("content", Some("C"), None, true, true)?;
    assert_query_equals_msg(
        "equivalent queries with null upperterms are equal()",
        &query,
        &other,
    );
    eq_msg(
        "hashcode returns same value",
        &query.query_hash(),
        &other.query_hash(),
    );

    let query = string_range("content", None, Some("C"), true, true)?;
    let other = string_range("content", Some("C"), None, true, true)?;
    Assert::assert_false_msg(
        "queries with different upper and lower terms are not equal",
        query.query_eq(&*other),
    );

    let query = string_range("content", Some("A"), Some("C"), false, false)?;
    let other = string_range("content", Some("A"), Some("C"), true, true)?;
    Assert::assert_false_msg(
        "queries with different inclusive are not equal",
        query.query_eq(&*other),
    );
    Ok(())
}

/// `private static class SingleCharTokenizer extends Tokenizer`.
#[derive(Debug)]
struct SingleCharTokenizer {
    buffer: [u16; 1],
    done: bool,
    term_att: Att,
}

impl TokenizerLogic for SingleCharTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.done {
            return Ok(false);
        }
        let count = input.read(&mut self.buffer)?;
        source.clear_attributes();
        self.done = true;
        if count == 1 {
            // The port's `CharTermAttribute` holds `char`s: a lone surrogate
            // unit, which no test value carries, has no `char`.
            let c = char::from_u32(u32::from(self.buffer[0])).unwrap_or('\u{FFFD}');
            self.term_att.char_term_mut().copy_buffer(&[c], 0, 1);
        }
        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.done = false;
        Ok(())
    }
}

/// `private static class SingleCharAnalyzer extends Analyzer`.
#[derive(Debug)]
struct SingleCharAnalyzer {
    state: AnalyzerState,
}

impl SingleCharAnalyzer {
    fn new() -> Self {
        SingleCharAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Analyzer for SingleCharAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestTermRangeQuery$SingleCharAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let t = Tokenizer::with_source(
            source,
            SingleCharTokenizer {
                buffer: [0; 1],
                done: false,
                term_att,
            },
        );
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(t))))
    }
}

/// LUCENE-38
///
/// Port of `TestTermRangeQuery.testExclusiveLowerNull`.
fn test_exclusive_lower_null(f: &mut TermRangeQueryTest) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(SingleCharAnalyzer::new());
    // http://issues.apache.org/jira/browse/LUCENE-38
    let query = string_range("content", None, Some("C"), false, false)?;
    f.initialize_index_with(&["A", "B", "", "C", "D"], Arc::clone(&analyzer))?;
    let (reader, searcher) = f.open()?;
    let n = num_hits(&searcher, &query)?;
    // When Lucene-38 is fixed, use the assert on the next line:
    eq_msg(
        "A,B,<empty string>,C,D => A, B & <empty string> are in range",
        &3,
        &n,
    );
    // until Lucene-38 is fixed, use this assert:
    // assertEquals("A,B,<empty string>,C,D => A, B & <empty string> are in range", 2,
    // hits.length());

    drop(searcher);
    reader.close()?;
    f.initialize_index_with(&["A", "B", "", "D"], Arc::clone(&analyzer))?;
    let (reader, searcher) = f.open()?;
    let n = num_hits(&searcher, &query)?;
    // When Lucene-38 is fixed, use the assert on the next line:
    eq_msg(
        "A,B,<empty string>,D => A, B & <empty string> are in range",
        &3,
        &n,
    );
    // until Lucene-38 is fixed, use this assert:
    // assertEquals("A,B,<empty string>,D => A, B & <empty string> are in range", 2, hits.length());
    drop(searcher);
    reader.close()?;
    f.add_doc("C")?;
    let (reader, searcher) = f.open()?;
    let n = num_hits(&searcher, &query)?;
    // When Lucene-38 is fixed, use the assert on the next line:
    eq_msg("C added, still A, B & <empty string> are in range", &3, &n);
    // until Lucene-38 is fixed, use this assert
    // assertEquals("C added, still A, B & <empty string> are in range", 2, hits.length());
    drop(searcher);
    reader.close()
}

/// LUCENE-38
///
/// Port of `TestTermRangeQuery.testInclusiveLowerNull`.
fn test_inclusive_lower_null(f: &mut TermRangeQueryTest) -> Result<()> {
    // http://issues.apache.org/jira/browse/LUCENE-38
    let analyzer: Arc<dyn Analyzer> = Arc::new(SingleCharAnalyzer::new());
    let query = string_range("content", None, Some("C"), true, true)?;
    f.initialize_index_with(&["A", "B", "", "C", "D"], Arc::clone(&analyzer))?;
    let (reader, searcher) = f.open()?;
    let n = num_hits(&searcher, &query)?;
    // When Lucene-38 is fixed, use the assert on the next line:
    eq_msg(
        "A,B,<empty string>,C,D => A,B,<empty string>,C in range",
        &4,
        &n,
    );
    // until Lucene-38 is fixed, use this assert
    // assertEquals("A,B,<empty string>,C,D => A,B,<empty string>,C in range", 3, hits.length());
    drop(searcher);
    reader.close()?;
    f.initialize_index_with(&["A", "B", "", "D"], Arc::clone(&analyzer))?;
    let (reader, searcher) = f.open()?;
    let n = num_hits(&searcher, &query)?;
    // When Lucene-38 is fixed, use the assert on the next line:
    eq_msg(
        "A,B,<empty string>,D - A, B and <empty string> in range",
        &3,
        &n,
    );
    // until Lucene-38 is fixed, use this assert
    // assertEquals("A,B,<empty string>,D => A, B and <empty string> in range", 2, hits.length());
    drop(searcher);
    reader.close()?;
    f.add_doc("C")?;
    let (reader, searcher) = f.open()?;
    let n = num_hits(&searcher, &query)?;
    // When Lucene-38 is fixed, use the assert on the next line:
    eq_msg("C added => A,B,<empty string>,C in range", &4, &n);
    // until Lucene-38 is fixed, use this assert
    // assertEquals("C added => A,B,<empty string>,C in range", 3, hits.length());
    drop(searcher);
    reader.close()
}
