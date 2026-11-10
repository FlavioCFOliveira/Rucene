//! Port of `lucene/core/src/test/org/apache/lucene/util/TestQueryBuilder.java`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, AnalyzerState, AnalyzerWrapper,
    AnalyzerWrapperLogic, CharReader, GlobalReuseStrategy, SharedTokenStream, TokenFilter,
    TokenFilterLogic, TokenStream, TokenStreamComponents, Tokenizer, TokenizerLogic,
};
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query;
use rucene::search::boost_attribute::{add_boost_attribute, set_boost};
use rucene::search::boost_query::BoostQuery;
use rucene::search::multi_phrase_query;
use rucene::search::phrase_query::{self, PhraseQuery};
use rucene::search::query::Query;
use rucene::search::synonym_query;
use rucene::search::term_query::TermQuery;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{CharacterRunAutomaton, RegExp};
use rucene::util::{BytesRef, QueryBuilder, QueryBuilderOps};
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::analysis::canned_binary_token_stream::{
    BinaryToken, CannedBinaryTokenStream,
};
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::analysis::mock_synonym_filter::MockSynonymFilter;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{class, random, test};

/// `IndexSearcher.TooManyClauses`.
const TOO_MANY_CLAUSES: &str = "org.apache.lucene.search.IndexSearcher$TooManyClauses";

/// `TestQueryBuilder`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testTerm", test_term),
        test("testBoolean", test_boolean),
        test("testBooleanMust", test_boolean_must),
        test("testMinShouldMatchNone", test_min_should_match_none),
        test("testMinShouldMatchAll", test_min_should_match_all),
        test("testMinShouldMatch", test_min_should_match),
        test(
            "testPhraseQueryPositionIncrements",
            test_phrase_query_position_increments,
        ),
        test("testEmpty", test_empty),
        test("testSynonyms", test_synonyms),
        test("testSynonymsPhrase", test_synonyms_phrase),
        test(
            "testMultiWordSynonymsPhrase",
            test_multi_word_synonyms_phrase,
        ),
        test(
            "testMultiWordSynonymsPhraseWithSlop",
            test_multi_word_synonyms_phrase_with_slop,
        ),
        test(
            "testMultiWordSynonymsBoolean",
            test_multi_word_synonyms_boolean,
        ),
        test(
            "testMultiWordPhraseSynonymsBoolean",
            test_multi_word_phrase_synonyms_boolean,
        ),
        test("testCJKTerm", test_cjk_term),
        test("testCJKPhrase", test_cjk_phrase),
        test("testCJKSloppyPhrase", test_cjk_sloppy_phrase),
        test("testCJKSynonym", test_cjk_synonym),
        test("testCJKSynonymsOR", test_cjk_synonyms_or),
        test("testCJKSynonymsOR2", test_cjk_synonyms_or2),
        test("testCJKSynonymsAND", test_cjk_synonyms_and),
        test("testCJKSynonymsAND2", test_cjk_synonyms_and2),
        test("testCJKSynonymsPhrase", test_cjk_synonyms_phrase),
        test("testNoTermAttribute", test_no_term_attribute),
        test("testMaxBooleanClause", test_max_boolean_clause),
        test("testTokenStreamBoosts", test_token_stream_boosts),
    ];
    class("org.apache.lucene.util.TestQueryBuilder", methods)
}

// -----------------------------------------------------------------------------
// helpers
// -----------------------------------------------------------------------------

/// `assertEquals(expected, actual)` over queries (`Query.equals`, printed
/// with `toString()`); a `null` actual fails.
fn assert_query(expected: &Arc<dyn Query>, actual: &Option<Arc<dyn Query>>) {
    let same = actual.as_ref().is_some_and(|a| expected.query_eq(&**a));
    if !same {
        let actual = actual
            .as_ref()
            .map_or_else(|| "null".to_string(), |a| a.to_query_string(""));
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            expected.to_query_string(""),
            actual
        ))
    }
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

/// `new TermQuery(new Term(field, text))`.
fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `new SynonymQuery.Builder(field).addTerm(t1).addTerm(t2).build()`.
fn synonym(field: &str, t1: &str, t2: &str) -> Result<Arc<dyn Query>> {
    let mut b = synonym_query::Builder::new(Some(field));
    b.add_term(&term(field, t1))?;
    b.add_term(&term(field, t2))?;
    Ok(Arc::new(b.build()))
}

/// `new PhraseQuery(slop, field, terms...)`.
fn phrase(slop: i32, field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(slop, field, terms)?))
}

/// `new MockAnalyzer(random())`.
fn mock_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::new(&*random()))
}

fn global_state() -> AnalyzerState {
    AnalyzerState::new(Box::new(GlobalReuseStrategy))
}

// -----------------------------------------------------------------------------
// tests
// -----------------------------------------------------------------------------

/// Port of `TestQueryBuilder.testTerm`.
fn test_term() -> Result<()> {
    let expected = tq("field", "test");
    let builder = QueryBuilder::new(Some(mock_analyzer()));
    assert_query(&expected, &builder.create_boolean_query("field", "test")?);
    Ok(())
}

/// Port of `TestQueryBuilder.testBoolean`.
fn test_boolean() -> Result<()> {
    let mut expected = boolean_query::Builder::new();
    expected.add(tq("field", "foo"), Occur::SHOULD)?;
    expected.add(tq("field", "bar"), Occur::SHOULD)?;
    let builder = QueryBuilder::new(Some(mock_analyzer()));
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query(
        &expected,
        &builder.create_boolean_query("field", "foo bar")?,
    );
    Ok(())
}

/// Port of `TestQueryBuilder.testBooleanMust`.
fn test_boolean_must() -> Result<()> {
    let mut expected = boolean_query::Builder::new();
    expected.add(tq("field", "foo"), Occur::MUST)?;
    expected.add(tq("field", "bar"), Occur::MUST)?;
    let builder = QueryBuilder::new(Some(mock_analyzer()));
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query(
        &expected,
        &builder.create_boolean_query_with_operator("field", "foo bar", Occur::MUST)?,
    );
    Ok(())
}

/// Port of `TestQueryBuilder.testMinShouldMatchNone`.
fn test_min_should_match_none() -> Result<()> {
    let builder = QueryBuilder::new(Some(mock_analyzer()));
    let expected = builder
        .create_boolean_query("field", "one two three four")?
        .expect("non-null query");
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 0.0)?,
    );
    Ok(())
}

/// Port of `TestQueryBuilder.testMinShouldMatchAll`.
fn test_min_should_match_all() -> Result<()> {
    let builder = QueryBuilder::new(Some(mock_analyzer()));
    let expected = builder
        .create_boolean_query_with_operator("field", "one two three four", Occur::MUST)?
        .expect("non-null query");
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 1.0)?,
    );
    Ok(())
}

/// Port of `TestQueryBuilder.testMinShouldMatch`.
fn test_min_should_match() -> Result<()> {
    let mut expected_b = boolean_query::Builder::new();
    expected_b.add(tq("field", "one"), Occur::SHOULD)?;
    expected_b.add(tq("field", "two"), Occur::SHOULD)?;
    expected_b.add(tq("field", "three"), Occur::SHOULD)?;
    expected_b.add(tq("field", "four"), Occur::SHOULD)?;
    expected_b.set_minimum_number_should_match(0);
    let _expected: Arc<dyn Query> = Arc::new(expected_b.build());

    let builder = QueryBuilder::new(Some(mock_analyzer()));
    // assertEquals(expected, builder.createMinShouldMatchQuery("field", "one two three four",
    // 0.1f));
    // assertEquals(expected, builder.createMinShouldMatchQuery("field", "one two three four",
    // 0.24f));

    expected_b.set_minimum_number_should_match(1);
    let expected: Arc<dyn Query> = Arc::new(expected_b.build());
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 0.25)?,
    );
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 0.49)?,
    );

    expected_b.set_minimum_number_should_match(2);
    let expected: Arc<dyn Query> = Arc::new(expected_b.build());
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 0.5)?,
    );
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 0.74)?,
    );

    expected_b.set_minimum_number_should_match(3);
    let expected: Arc<dyn Query> = Arc::new(expected_b.build());
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 0.75)?,
    );
    assert_query(
        &expected,
        &builder.create_min_should_match_query("field", "one two three four", 0.99)?,
    );
    Ok(())
}

/// Port of `TestQueryBuilder.testPhraseQueryPositionIncrements`.
fn test_phrase_query_position_increments() -> Result<()> {
    let mut pq_builder = phrase_query::Builder::new();
    pq_builder.add_at(term("field", "1"), 0)?;
    pq_builder.add_at(term("field", "2"), 2)?;
    let expected: Arc<dyn Query> = Arc::new(pq_builder.build()?);
    let stop_list = Arc::new(CharacterRunAutomaton::new(
        RegExp::new("[sS][tT][oO][pP]")?.to_automaton()?,
    )?);

    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
        stop_list,
    ));

    let builder = QueryBuilder::new(Some(analyzer));
    assert_query(
        &expected,
        &builder.create_phrase_query("field", "1 stop 2")?,
    );
    Ok(())
}

/// Port of `TestQueryBuilder.testEmpty`.
fn test_empty() -> Result<()> {
    let builder = QueryBuilder::new(Some(mock_analyzer()));
    Assert::assert_true(builder.create_boolean_query("field", "")?.is_none());
    Ok(())
}

/// adds synonym of "dog" for "dogs", and synonym of "cavy" for "guinea pig".
///
/// Port of `TestQueryBuilder.MockSynonymAnalyzer`.
#[derive(Debug)]
struct MockSynonymAnalyzer {
    state: AnalyzerState,
}

impl MockSynonymAnalyzer {
    fn new() -> Self {
        Self {
            state: global_state(),
        }
    }
}

impl Analyzer for MockSynonymAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.TestQueryBuilder$MockSynonymAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Rc::new(RefCell::new(MockTokenizer::whitespace_lower_case()?));
        let filter = MockSynonymFilter::new(shared(&tokenizer))?;
        Ok(components(tokenizer, sink(filter)))
    }
}

/// simple synonyms test
///
/// Port of `TestQueryBuilder.testSynonyms`.
fn test_synonyms() -> Result<()> {
    let expected = synonym("field", "dogs", "dog")?;
    let builder = QueryBuilder::new(Some(Arc::new(MockSynonymAnalyzer::new())));
    assert_query(&expected, &builder.create_boolean_query("field", "dogs")?);
    assert_query(&expected, &builder.create_phrase_query("field", "dogs")?);
    assert_query(
        &expected,
        &builder.create_boolean_query_with_operator("field", "dogs", Occur::MUST)?,
    );
    assert_query(&expected, &builder.create_phrase_query("field", "dogs")?);
    Ok(())
}

/// forms multiphrase query
///
/// Port of `TestQueryBuilder.testSynonymsPhrase`.
fn test_synonyms_phrase() -> Result<()> {
    let mut expected_builder = multi_phrase_query::Builder::new();
    expected_builder.add(term("field", "old"))?;
    expected_builder.add_terms(vec![term("field", "dogs"), term("field", "dog")])?;
    let builder = QueryBuilder::new(Some(Arc::new(MockSynonymAnalyzer::new())));
    let expected: Arc<dyn Query> = Arc::new(expected_builder.build());
    assert_query(
        &expected,
        &builder.create_phrase_query("field", "old dogs")?,
    );
    Ok(())
}

/// forms graph query
///
/// Port of `TestQueryBuilder.testMultiWordSynonymsPhrase`.
fn test_multi_word_synonyms_phrase() -> Result<()> {
    let mut b = boolean_query::Builder::new();
    b.add(phrase(0, "field", &["guinea", "pig"])?, Occur::SHOULD)?;
    b.add(tq("field", "cavy"), Occur::SHOULD)?;
    let expected: Arc<dyn Query> = Arc::new(b.build());

    let query_builder = QueryBuilder::new(Some(Arc::new(MockSynonymAnalyzer::new())));
    assert_query(
        &expected,
        &query_builder.create_phrase_query("field", "guinea pig")?,
    );
    Ok(())
}

/// Port of `TestQueryBuilder.testMultiWordSynonymsPhraseWithSlop`.
fn test_multi_word_synonyms_phrase_with_slop() -> Result<()> {
    let mut pq = phrase_query::Builder::new();
    pq.set_slop(4);
    pq.add(term("field", "guinea"))?;
    pq.add(term("field", "pig"))?;
    let mut b = boolean_query::Builder::new();
    b.add(Arc::new(pq.build()?), Occur::SHOULD)?;
    b.add(tq("field", "cavy"), Occur::SHOULD)?;
    let expected: Arc<dyn Query> = Arc::new(b.build());
    let query_builder = QueryBuilder::new(Some(Arc::new(MockSynonymAnalyzer::new())));
    assert_query(
        &expected,
        &query_builder.create_phrase_query_with_slop("field", "guinea pig", 4)?,
    );
    Ok(())
}

/// The four `createBooleanQuery` checks shared by
/// `testMultiWordSynonymsBoolean` and `testMultiWordPhraseSynonymsBoolean`.
fn check_multi_word_boolean(
    query_builder: &QueryBuilder,
    syn_query: &Arc<dyn Query>,
    occur: Occur,
) -> Result<()> {
    let mut b = boolean_query::Builder::new();
    b.add(Arc::clone(syn_query), occur)?;
    let expected_graph_query: Arc<dyn Query> = Arc::new(b.build());
    assert_query(
        &expected_graph_query,
        &query_builder.create_boolean_query_with_operator("field", "guinea pig", occur)?,
    );

    let mut b = boolean_query::Builder::new();
    b.add(Arc::clone(syn_query), occur)?;
    b.add(tq("field", "story"), occur)?;
    let expected_boolean_query: Arc<dyn Query> = Arc::new(b.build());
    assert_query(
        &expected_boolean_query,
        &query_builder.create_boolean_query_with_operator("field", "guinea pig story", occur)?,
    );

    let mut b = boolean_query::Builder::new();
    b.add(tq("field", "the"), occur)?;
    b.add(Arc::clone(syn_query), occur)?;
    b.add(tq("field", "story"), occur)?;
    let expected_boolean_query: Arc<dyn Query> = Arc::new(b.build());
    assert_query(
        &expected_boolean_query,
        &query_builder.create_boolean_query_with_operator(
            "field",
            "the guinea pig story",
            occur,
        )?,
    );

    let mut b = boolean_query::Builder::new();
    b.add(tq("field", "the"), occur)?;
    b.add(Arc::clone(syn_query), occur)?;
    b.add(tq("field", "story"), occur)?;
    b.add(Arc::clone(syn_query), occur)?;
    let expected_boolean_query: Arc<dyn Query> = Arc::new(b.build());
    assert_query(
        &expected_boolean_query,
        &query_builder.create_boolean_query_with_operator(
            "field",
            "the guinea pig story guinea pig",
            occur,
        )?,
    );
    Ok(())
}

/// forms graph query
///
/// Port of `TestQueryBuilder.testMultiWordSynonymsBoolean`.
fn test_multi_word_synonyms_boolean() -> Result<()> {
    for occur in [Occur::SHOULD, Occur::MUST] {
        let mut b = boolean_query::Builder::new();
        b.add(tq("field", "guinea"), Occur::MUST)?;
        b.add(tq("field", "pig"), Occur::MUST)?;
        let syn1: Arc<dyn Query> = Arc::new(b.build());
        let syn2 = tq("field", "cavy");

        let mut b = boolean_query::Builder::new();
        b.add(syn1, Occur::SHOULD)?;
        b.add(syn2, Occur::SHOULD)?;
        let syn_query: Arc<dyn Query> = Arc::new(b.build());

        let query_builder = QueryBuilder::new(Some(Arc::new(MockSynonymAnalyzer::new())));
        check_multi_word_boolean(&query_builder, &syn_query, occur)?;
    }
    Ok(())
}

/// forms graph query
///
/// Port of `TestQueryBuilder.testMultiWordPhraseSynonymsBoolean`.
fn test_multi_word_phrase_synonyms_boolean() -> Result<()> {
    for occur in [Occur::SHOULD, Occur::MUST] {
        let mut pq = phrase_query::Builder::new();
        pq.add(term("field", "guinea"))?;
        pq.add(term("field", "pig"))?;
        let syn1: Arc<dyn Query> = Arc::new(pq.build()?);
        let syn2 = tq("field", "cavy");

        let mut b = boolean_query::Builder::new();
        b.add(syn1, Occur::SHOULD)?;
        b.add(syn2, Occur::SHOULD)?;
        let syn_query: Arc<dyn Query> = Arc::new(b.build());
        let mut query_builder = QueryBuilder::new(Some(Arc::new(MockSynonymAnalyzer::new())));
        query_builder.set_auto_generate_multi_term_synonyms_phrase_query(true);
        check_multi_word_boolean(&query_builder, &syn_query, occur)?;
    }
    Ok(())
}

/// Port of `TestQueryBuilder.SimpleCJKTokenizer`: every char is a term.
#[derive(Debug)]
struct SimpleCJKTokenizer {
    term_att: Att,
}

impl SimpleCJKTokenizer {
    /// `new SimpleCJKTokenizer()`.
    fn create() -> Result<Tokenizer<SimpleCJKTokenizer>> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        Ok(Tokenizer::with_source(
            source,
            SimpleCJKTokenizer { term_att },
        ))
    }
}

impl TokenizerLogic for SimpleCJKTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        let mut buf = [0u16; 1];
        // int ch = input.read(); if (ch < 0) return false;
        if reader.read(&mut buf)? == 0 {
            return Ok(false);
        }
        source.clear_attributes();
        // termAtt.setEmpty().append((char) ch): one UTF-16 unit
        self.term_att.set_term(&String::from_utf16_lossy(&buf));
        Ok(true)
    }
}

/// Port of `TestQueryBuilder.SimpleCJKAnalyzer`.
#[derive(Debug)]
struct SimpleCJKAnalyzer {
    state: AnalyzerState,
}

impl SimpleCJKAnalyzer {
    fn new() -> Self {
        Self {
            state: global_state(),
        }
    }
}

impl Analyzer for SimpleCJKAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.TestQueryBuilder$SimpleCJKAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Ok(TokenStreamComponents::from_tokenizer(Rc::new(
            RefCell::new(SimpleCJKTokenizer::create()?),
        )))
    }
}

/// Port of `TestQueryBuilder.testCJKTerm`.
fn test_cjk_term() -> Result<()> {
    // individual CJK chars as terms
    let analyzer = Arc::new(SimpleCJKAnalyzer::new());

    let mut expected = boolean_query::Builder::new();
    expected.add(tq("field", "中"), Occur::SHOULD)?;
    expected.add(tq("field", "国"), Occur::SHOULD)?;

    let builder = QueryBuilder::new(Some(analyzer));
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query(&expected, &builder.create_boolean_query("field", "中国")?);
    Ok(())
}

/// Port of `TestQueryBuilder.testCJKPhrase`.
fn test_cjk_phrase() -> Result<()> {
    // individual CJK chars as terms
    let analyzer = Arc::new(SimpleCJKAnalyzer::new());

    let expected = phrase(0, "field", &["中", "国"])?;

    let builder = QueryBuilder::new(Some(analyzer));
    assert_query(&expected, &builder.create_phrase_query("field", "中国")?);
    Ok(())
}

/// Port of `TestQueryBuilder.testCJKSloppyPhrase`.
fn test_cjk_sloppy_phrase() -> Result<()> {
    // individual CJK chars as terms
    let analyzer = Arc::new(SimpleCJKAnalyzer::new());

    let expected = phrase(3, "field", &["中", "国"])?;

    let builder = QueryBuilder::new(Some(analyzer));
    assert_query(
        &expected,
        &builder.create_phrase_query_with_slop("field", "中国", 3)?,
    );
    Ok(())
}

/// adds synonym of "國" for "国".
///
/// Port of `TestQueryBuilder.MockCJKSynonymFilter`.
#[derive(Debug)]
struct MockCJKSynonymFilter {
    term_att: Att,
    pos_inc_att: Att,
    add_synonym: bool,
}

impl MockCJKSynonymFilter {
    fn create(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<MockCJKSynonymFilter>> {
        let source = input.attribute_source_mut();
        let term_att = Att::add(source, char_term_id())?;
        let pos_inc_att = Att::add(source, pos_inc_id())?;
        Ok(TokenFilter::new(
            input,
            MockCJKSynonymFilter {
                term_att,
                pos_inc_att,
                add_synonym: false,
            },
        ))
    }
}

impl TokenFilterLogic for MockCJKSynonymFilter {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.add_synonym {
            // inject our synonym
            source.clear_attributes();
            self.term_att.set_term("國");
            self.pos_inc_att.pos_inc_mut().set_position_increment(0);
            self.add_synonym = false;
            return Ok(true);
        }

        if input.increment_token()? {
            self.add_synonym = self.term_att.term_string() == "国";
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Port of `TestQueryBuilder.MockCJKSynonymAnalyzer`.
#[derive(Debug)]
struct MockCJKSynonymAnalyzer {
    state: AnalyzerState,
}

impl MockCJKSynonymAnalyzer {
    fn new() -> Self {
        Self {
            state: global_state(),
        }
    }
}

impl Analyzer for MockCJKSynonymAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.TestQueryBuilder$MockCJKSynonymAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Rc::new(RefCell::new(SimpleCJKTokenizer::create()?));
        let filter = MockCJKSynonymFilter::create(shared(&tokenizer))?;
        Ok(components(tokenizer, sink(filter)))
    }
}

/// simple CJK synonym test
///
/// Port of `TestQueryBuilder.testCJKSynonym`.
fn test_cjk_synonym() -> Result<()> {
    let expected = synonym("field", "国", "國")?;
    let builder = QueryBuilder::new(Some(Arc::new(MockCJKSynonymAnalyzer::new())));
    assert_query(&expected, &builder.create_boolean_query("field", "国")?);
    assert_query(&expected, &builder.create_phrase_query("field", "国")?);
    assert_query(
        &expected,
        &builder.create_boolean_query_with_operator("field", "国", Occur::MUST)?,
    );
    Ok(())
}

/// synonyms with default OR operator
///
/// Port of `TestQueryBuilder.testCJKSynonymsOR`.
fn test_cjk_synonyms_or() -> Result<()> {
    let mut expected = boolean_query::Builder::new();
    expected.add(tq("field", "中"), Occur::SHOULD)?;
    let inner = synonym("field", "国", "國")?;
    expected.add(inner, Occur::SHOULD)?;
    let builder = QueryBuilder::new(Some(Arc::new(MockCJKSynonymAnalyzer::new())));
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query(&expected, &builder.create_boolean_query("field", "中国")?);
    Ok(())
}

/// more complex synonyms with default OR operator
///
/// Port of `TestQueryBuilder.testCJKSynonymsOR2`.
fn test_cjk_synonyms_or2() -> Result<()> {
    let mut expected = boolean_query::Builder::new();
    expected.add(tq("field", "中"), Occur::SHOULD)?;
    let inner = synonym("field", "国", "國")?;
    expected.add(inner, Occur::SHOULD)?;
    let inner2 = synonym("field", "国", "國")?;
    expected.add(inner2, Occur::SHOULD)?;
    let builder = QueryBuilder::new(Some(Arc::new(MockCJKSynonymAnalyzer::new())));
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query(&expected, &builder.create_boolean_query("field", "中国国")?);
    Ok(())
}

/// synonyms with default AND operator
///
/// Port of `TestQueryBuilder.testCJKSynonymsAND`.
fn test_cjk_synonyms_and() -> Result<()> {
    let mut expected = boolean_query::Builder::new();
    expected.add(tq("field", "中"), Occur::MUST)?;
    let inner = synonym("field", "国", "國")?;
    expected.add(inner, Occur::MUST)?;
    let builder = QueryBuilder::new(Some(Arc::new(MockCJKSynonymAnalyzer::new())));
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query(
        &expected,
        &builder.create_boolean_query_with_operator("field", "中国", Occur::MUST)?,
    );
    Ok(())
}

/// more complex synonyms with default AND operator
///
/// Port of `TestQueryBuilder.testCJKSynonymsAND2`.
fn test_cjk_synonyms_and2() -> Result<()> {
    let mut expected = boolean_query::Builder::new();
    expected.add(tq("field", "中"), Occur::MUST)?;
    let inner = synonym("field", "国", "國")?;
    expected.add(inner, Occur::MUST)?;
    let inner2 = synonym("field", "国", "國")?;
    expected.add(inner2, Occur::MUST)?;
    let builder = QueryBuilder::new(Some(Arc::new(MockCJKSynonymAnalyzer::new())));
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query(
        &expected,
        &builder.create_boolean_query_with_operator("field", "中国国", Occur::MUST)?,
    );
    Ok(())
}

/// forms multiphrase query
///
/// Port of `TestQueryBuilder.testCJKSynonymsPhrase`.
fn test_cjk_synonyms_phrase() -> Result<()> {
    let mut expected_builder = multi_phrase_query::Builder::new();
    expected_builder.add(term("field", "中"))?;
    expected_builder.add_terms(vec![term("field", "国"), term("field", "國")])?;
    let builder = QueryBuilder::new(Some(Arc::new(MockCJKSynonymAnalyzer::new())));
    let expected: Arc<dyn Query> = Arc::new(expected_builder.build());
    assert_query(&expected, &builder.create_phrase_query("field", "中国")?);
    expected_builder.set_slop(3)?;
    let expected: Arc<dyn Query> = Arc::new(expected_builder.build());
    assert_query(
        &expected,
        &builder.create_phrase_query_with_slop("field", "中国", 3)?,
    );
    Ok(())
}

/// The anonymous `Tokenizer` of `testNoTermAttribute`: no
/// `CharTermAttribute`, asserting that `reset()` precedes
/// `incrementToken()`.
#[derive(Debug)]
struct NoTermTokenizer {
    was_reset: bool,
}

impl TokenizerLogic for NoTermTokenizer {
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        // super.reset() is performed by `Tokenizer::reset`.
        Assert::assert_false(self.was_reset);
        self.was_reset = true;
        Ok(())
    }

    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        _reader: &mut dyn CharReader,
    ) -> Result<bool> {
        Assert::assert_true(self.was_reset);
        Ok(false)
    }
}

/// The anonymous `Analyzer` of `testNoTermAttribute`.
#[derive(Debug)]
struct NoTermAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for NoTermAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.TestQueryBuilder$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        // Can't use MockTokenizer because it adds TermAttribute and we don't want that
        let source = AttributeSource::new_with_factory(default_token_attribute_factory());
        Ok(TokenStreamComponents::from_tokenizer(Rc::new(
            RefCell::new(Tokenizer::with_source(
                source,
                NoTermTokenizer { was_reset: false },
            )),
        )))
    }
}

/// Port of `TestQueryBuilder.testNoTermAttribute`.
fn test_no_term_attribute() -> Result<()> {
    let analyzer = Arc::new(NoTermAnalyzer {
        state: global_state(),
    });
    let builder = QueryBuilder::new(Some(analyzer));
    Assert::assert_true(builder.create_boolean_query("field", "whatever")?.is_none());
    Ok(())
}

/// Port of `TestQueryBuilder.testMaxBooleanClause`.
fn test_max_boolean_clause() -> Result<()> {
    let size = 34usize;
    let mut tokens: Vec<Option<BinaryToken>> = vec![None; size];
    let term1 = BytesRef::from_char_sequence("ff");
    let term2 = BytesRef::from_char_sequence("f");
    let mut i = 0;
    while i < size {
        if i % 2 == 0 {
            tokens[i] = Some(BinaryToken::with_positions(term2.clone(), 1, 1));
            tokens[i + 1] = Some(BinaryToken::with_positions(term1.clone(), 0, 2));
            i += 2;
        } else {
            tokens[i] = Some(BinaryToken::with_positions(term2.clone(), 1, 1));
            i += 1;
        }
    }
    let tokens: Vec<BinaryToken> = tokens.into_iter().map(|t| t.expect("filled")).collect();
    let qb = QueryBuilder::new(None);
    {
        let mut ts = CannedBinaryTokenStream::new(tokens.clone());
        expect_throws(TOO_MANY_CLAUSES, || {
            qb.analyze_graph_boolean("", &mut ts, Occur::MUST)
        });
        ts.close()?;
    }
    {
        let mut ts = CannedBinaryTokenStream::new(tokens.clone());
        expect_throws(TOO_MANY_CLAUSES, || {
            qb.analyze_graph_boolean("", &mut ts, Occur::SHOULD)
        });
        ts.close()?;
    }
    {
        let mut ts = CannedBinaryTokenStream::new(tokens);
        expect_throws(TOO_MANY_CLAUSES, || qb.analyze_graph_phrase(&mut ts, "", 0));
        ts.close()?;
    }
    Ok(())
}

/// Port of `TestQueryBuilder.MockBoostTokenFilter`.
#[derive(Debug)]
struct MockBoostTokenFilter {
    term_att: Att,
}

impl MockBoostTokenFilter {
    fn create(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<MockBoostTokenFilter>> {
        let source = input.attribute_source_mut();
        // boostAtt = addAttribute(BoostAttribute.class)
        add_boost_attribute(source);
        let term_att = Att::add(source, char_term_id())?;
        Ok(TokenFilter::new(input, MockBoostTokenFilter { term_att }))
    }
}

impl TokenFilterLogic for MockBoostTokenFilter {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        if self.term_att.term_length16() == 3 {
            set_boost(source, 0.5);
        }
        Ok(true)
    }
}

/// The anonymous `AnalyzerWrapper` of `testTokenStreamBoosts`.
#[derive(Debug)]
struct BoostWrapper {
    msa: Arc<dyn Analyzer>,
}

impl AnalyzerWrapperLogic for BoostWrapper {
    fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
        &*self.msa
    }

    fn wrap_components(
        &self,
        _field_name: &str,
        components: TokenStreamComponents,
    ) -> TokenStreamComponents {
        let input: Box<dyn TokenStream> =
            Box::new(SharedTokenStream::new(components.token_stream()));
        let filter = MockBoostTokenFilter::create(input)
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e));
        components.with_sink(sink(filter))
    }
}

/// Port of `TestQueryBuilder.testTokenStreamBoosts`.
fn test_token_stream_boosts() -> Result<()> {
    let msa: Arc<dyn Analyzer> = Arc::new(MockSynonymAnalyzer::new());
    // new AnalyzerWrapper(msa.getReuseStrategy()): MockSynonymAnalyzer uses
    // the default GLOBAL_REUSE_STRATEGY.
    let a: Arc<dyn Analyzer> = Arc::new(AnalyzerWrapper::new(
        Box::new(BoostWrapper { msa }),
        Box::new(GlobalReuseStrategy),
    ));

    let builder = QueryBuilder::new(Some(a));
    let q = builder.create_boolean_query("field", "hot dogs")?;
    let mut syn = synonym_query::Builder::new(Some("field"));
    syn.add_term(&term("field", "dogs"))?;
    syn.add_term_with_boost(&term("field", "dog"), 0.5)?;
    let mut b = boolean_query::Builder::new();
    b.add(
        Arc::new(BoostQuery::new(tq("field", "hot"), 0.5)?),
        Occur::SHOULD,
    )?;
    b.add(Arc::new(syn.build()), Occur::SHOULD)?;
    let expected: Arc<dyn Query> = Arc::new(b.build());

    assert_query(&expected, &q);
    Ok(())
}
