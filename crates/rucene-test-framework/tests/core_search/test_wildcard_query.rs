//! Port of `lucene/core/src/test/org/apache/lucene/search/TestWildcardQuery.java`.

use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::automaton_terms_enum::AutomatonTermsEnum;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::{FilterLeafReader, FilterTerms, FilterTermsEnum};
use rucene::index::index_reader::CacheHelper;
use rucene::index::terms::{FilteredTermsEnum, SeekStatus, TermState};
use rucene::index::{
    ImpactsEnum, IndexReader, LeafReader, MultiTerms, PostingsEnum, Term, Terms, TermsEnum,
};
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, constant_score_boolean_rewrite, constant_score_rewrite,
    get_terms_enum, scoring_boolean_rewrite,
};
use rucene::search::multi_term_query_constant_score_blended_wrapper::MultiTermQueryConstantScoreBlendedWrapper;
use rucene::search::multi_term_query_constant_score_wrapper::MultiTermQueryConstantScoreWrapper;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::{
    query_to_string, BooleanQueryBuilder, ConstantScoreQuery, MatchNoDocsQuery, Occur, Query,
    ScoreMode,
};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene::util::automaton::CompiledAutomaton;
use rucene::util::{BytesRef, IOBooleanSupplier};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::assert_query_equals;
use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, class, random, test, verbose,
};

/// `TestWildcardQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestWildcardQuery",
        vec![
            test("testEquals", test_equals),
            test("testTermWithoutWildcard", test_term_without_wildcard),
            test("testEmptyTerm", test_empty_term),
            test("testPrefixTerm", test_prefix_term),
            test("testAsterisk", test_asterisk),
            test("testQuestionmark", test_questionmark),
            test("testEscapes", test_escapes),
            test("testParsingAndSearching", test_parsing_and_searching),
            test("testLarge", test_large),
            test("testCostEstimate", test_cost_estimate),
            test(
                "testScorerSupplierDoesNotScanTermsEagerly",
                test_scorer_supplier_does_not_scan_terms_eagerly,
            ),
        ],
    )
}

/// `new WildcardQuery(new Term(field, text))`.
fn wq(field: &str, text: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(WildcardQuery::new(Term::from_text(field, text))?))
}

/// Port of `TestWildcardQuery.testEquals`.
fn test_equals() -> Result<()> {
    let wq1 = WildcardQuery::new(Term::from_text("field", "b*a"))?;
    let wq2 = WildcardQuery::new(Term::from_text("field", "b*a"))?;
    let wq3 = WildcardQuery::new(Term::from_text("field", "b*a"))?;

    // reflexive?
    assert_query_equals(&wq1, &wq2);
    assert_query_equals(&wq2, &wq1);

    // transitive?
    assert_query_equals(&wq2, &wq3);
    assert_query_equals(&wq1, &wq3);

    // `wq1.equals(null)`: no query is null.
    Assert::assert_false(None::<&dyn Query>.is_some_and(|q| wq1.query_eq(q)));

    let fq = FuzzyQuery::new(Term::from_text("field", "b*a"))?;
    Assert::assert_false(wq1.query_eq(&fq));
    Assert::assert_false(fq.query_eq(&wq1));
    Ok(())
}

/// `DirectoryReader.open(indexStore)` and `newSearcher(reader)`.
fn open(index_store: &TestDirectory) -> Result<(Arc<dyn DirectoryReader>, IndexSearcher)> {
    let reader = directory_reader::open(index_store.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    Ok((reader, searcher))
}

/// `searcher.rewrite(new WildcardQuery(new Term(field, text), DEFAULT_DETERMINIZE_WORK_LIMIT, rewrite))`.
fn rewrite_with(
    searcher: &IndexSearcher,
    text: &str,
    rewrite: Arc<dyn rucene::search::multi_term_query::RewriteMethod>,
) -> Result<Arc<dyn Query>> {
    searcher.rewrite(Arc::new(WildcardQuery::with_rewrite_method(
        Term::from_text("field", text),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        rewrite,
    )?))
}

/// Tests if a WildcardQuery that has no wildcard in the term is rewritten to
/// a single TermQuery. The boost should be preserved, and the rewrite should
/// return a ConstantScoreQuery if the WildcardQuery had a ConstantScore
/// rewriteMethod.
///
/// Port of `TestWildcardQuery.testTermWithoutWildcard`.
fn test_term_without_wildcard() -> Result<()> {
    let mut index_store = get_index_store("field", &["nowildcard", "nowildcardx"])?;
    let (reader, searcher) = open(&index_store)?;

    let wq = wq("field", "nowildcard")?;
    assert_matches(&searcher, &wq, 1)?;

    let q = rewrite_with(&searcher, "nowildcard", scoring_boolean_rewrite())?;
    Assert::assert_true(q.as_any().is::<TermQuery>());

    let q = rewrite_with(&searcher, "nowildcard", constant_score_rewrite())?;
    Assert::assert_true(q.as_any().is::<MultiTermQueryConstantScoreWrapper>());

    let q = rewrite_with(&searcher, "nowildcard", constant_score_blended_rewrite())?;
    Assert::assert_true(q.as_any().is::<MultiTermQueryConstantScoreBlendedWrapper>());

    let q = rewrite_with(&searcher, "nowildcard", constant_score_boolean_rewrite())?;
    Assert::assert_true(q.as_any().is::<ConstantScoreQuery>());
    drop(searcher);
    reader.close()?;
    index_store.close()
}

/// Tests if a WildcardQuery with an empty term is rewritten to an empty
/// BooleanQuery
///
/// Port of `TestWildcardQuery.testEmptyTerm`.
fn test_empty_term() -> Result<()> {
    let mut index_store = get_index_store("field", &["nowildcard", "nowildcardx"])?;
    let (reader, searcher) = open(&index_store)?;

    let wq: Arc<dyn Query> = Arc::new(WildcardQuery::with_rewrite_method(
        Term::from_text("field", ""),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        scoring_boolean_rewrite(),
    )?);
    assert_matches(&searcher, &wq, 0)?;
    let q = searcher.rewrite(wq)?;
    Assert::assert_true(q.as_any().is::<MatchNoDocsQuery>());
    drop(searcher);
    reader.close()?;
    index_store.close()
}

/// Tests if a WildcardQuery that has only a trailing * in the term is
/// rewritten to a single PrefixQuery. The boost and rewriteMethod should be
/// preserved.
///
/// Port of `TestWildcardQuery.testPrefixTerm`.
fn test_prefix_term() -> Result<()> {
    let mut index_store = get_index_store("field", &["prefix", "prefixx"])?;
    let (reader, searcher) = open(&index_store)?;

    let wq1 = wq("field", "prefix*")?;
    assert_matches(&searcher, &wq1, 2)?;

    let wq = WildcardQuery::new(Term::from_text("field", "*"))?;
    assert_matches(&searcher, &(Arc::new(wq.clone()) as Arc<dyn Query>), 2)?;
    let terms: Arc<dyn Terms> = Arc::from(
        MultiTerms::get_terms(searcher.get_index_reader(), "field")?.expect("field is indexed"),
    );
    // `wq.getTermsEnum(terms).getClass().getSimpleName().contains("AutomatonTermsEnum")`:
    // the port's `AutomatonTermsEnum` is the filter of a `FilteredTermsEnum`.
    let te = get_terms_enum(&wq, &terms)?;
    let is_automaton = te
        .as_any()
        .and_then(|a| a.downcast_ref::<FilteredTermsEnum>())
        .and_then(|fte| fte.filter().as_any())
        .is_some_and(|a| a.is::<AutomatonTermsEnum>());
    Assert::assert_false(is_automaton);
    drop(searcher);
    reader.close()?;
    index_store.close()
}

/// Tests Wildcard queries with an asterisk.
///
/// Port of `TestWildcardQuery.testAsterisk`.
fn test_asterisk() -> Result<()> {
    let mut index_store = get_index_store("body", &["metal", "metals"])?;
    let (reader, searcher) = open(&index_store)?;
    let query1: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("body", "metal")));
    let query2 = wq("body", "metal*")?;
    let query3 = wq("body", "m*tal")?;
    let query4 = wq("body", "m*tal*")?;
    let query5 = wq("body", "m*tals")?;

    let mut query6 = BooleanQueryBuilder::new();
    query6.add(Arc::clone(&query5), Occur::SHOULD)?;

    let mut query7 = BooleanQueryBuilder::new();
    query7.add(Arc::clone(&query3), Occur::SHOULD)?;
    query7.add(Arc::clone(&query5), Occur::SHOULD)?;

    // Queries do not automatically lower-case search terms:
    let query8 = wq("body", "M*tal*")?;

    assert_matches(&searcher, &query1, 1)?;
    assert_matches(&searcher, &query2, 2)?;
    assert_matches(&searcher, &query3, 1)?;
    assert_matches(&searcher, &query4, 2)?;
    assert_matches(&searcher, &query5, 1)?;
    assert_matches(&searcher, &(Arc::new(query6.build()) as Arc<dyn Query>), 1)?;
    assert_matches(&searcher, &(Arc::new(query7.build()) as Arc<dyn Query>), 2)?;
    assert_matches(&searcher, &query8, 0)?;
    assert_matches(&searcher, &wq("body", "*tall")?, 0)?;
    assert_matches(&searcher, &wq("body", "*tal")?, 1)?;
    assert_matches(&searcher, &wq("body", "*tal*")?, 2)?;
    drop(searcher);
    reader.close()?;
    index_store.close()
}

/// Tests Wildcard queries with a question mark.
///
/// Port of `TestWildcardQuery.testQuestionmark`.
fn test_questionmark() -> Result<()> {
    let mut index_store = get_index_store("body", &["metal", "metals", "mXtals", "mXtXls"])?;
    let (reader, searcher) = open(&index_store)?;
    let query1 = wq("body", "m?tal")?;
    let query2 = wq("body", "metal?")?;
    let query3 = wq("body", "metals?")?;
    let query4 = wq("body", "m?t?ls")?;
    let query5 = wq("body", "M?t?ls")?;
    let query6 = wq("body", "meta??")?;

    assert_matches(&searcher, &query1, 1)?;
    assert_matches(&searcher, &query2, 1)?;
    assert_matches(&searcher, &query3, 0)?;
    assert_matches(&searcher, &query4, 3)?;
    assert_matches(&searcher, &query5, 0)?;
    assert_matches(&searcher, &query6, 1)?; // Query: 'meta??' matches 'metals' not 'metal'
    drop(searcher);
    reader.close()?;
    index_store.close()
}

/// Tests if wildcard escaping works
///
/// Port of `TestWildcardQuery.testEscapes`.
fn test_escapes() -> Result<()> {
    let mut index_store = get_index_store(
        "field",
        &[
            "foo*bar",
            "foo??bar",
            "fooCDbar",
            "fooSOMETHINGbar",
            "foo\\",
        ],
    )?;
    let (reader, searcher) = open(&index_store)?;

    // without escape: matches foo??bar, fooCDbar, foo*bar, and fooSOMETHINGbar
    let unescaped = wq("field", "foo*bar")?;
    assert_matches(&searcher, &unescaped, 4)?;

    // with escape: only matches foo*bar
    let escaped = wq("field", "foo\\*bar")?;
    assert_matches(&searcher, &escaped, 1)?;

    // without escape: matches foo??bar and fooCDbar
    let unescaped = wq("field", "foo??bar")?;
    assert_matches(&searcher, &unescaped, 2)?;

    // with escape: matches foo??bar only
    let escaped = wq("field", "foo\\?\\?bar")?;
    assert_matches(&searcher, &escaped, 1)?;

    // check escaping at end: lenient parse yields "foo\"
    let at_end = wq("field", "foo\\")?;
    assert_matches(&searcher, &at_end, 1)?;

    drop(searcher);
    reader.close()?;
    index_store.close()
}

/// `private Directory getIndexStore(String field, String[] contents)`.
fn get_index_store(field: &str, contents: &[&str]) -> Result<TestDirectory> {
    let index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    for content in contents {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            field,
            content,
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    writer.close()?;

    Ok(index_store)
}

/// `private void assertMatches(IndexSearcher searcher, Query q, int expectedMatches)`.
fn assert_matches(
    searcher: &IndexSearcher,
    q: &Arc<dyn Query>,
    expected_matches: usize,
) -> Result<()> {
    let result = searcher.search(Arc::clone(q), 1000)?.score_docs;
    assert_equals(&expected_matches, &result.len());
    Ok(())
}

/// `q.getClass().getName()` for the queries of `testParsingAndSearching`.
fn class_name(q: &dyn Query) -> &'static str {
    if q.as_any().is::<PrefixQuery>() {
        "org.apache.lucene.search.PrefixQuery"
    } else {
        "org.apache.lucene.search.WildcardQuery"
    }
}

/// Test that wild card queries are parsed to the correct type and are
/// searched correctly. This test looks at both parsing and execution of
/// wildcard queries. Although placed here, it also tests prefix queries,
/// verifying that prefix queries are not parsed into wild card queries, and
/// vice-versa.
///
/// Port of `TestWildcardQuery.testParsingAndSearching`.
fn test_parsing_and_searching() -> Result<()> {
    let field = "content";
    let docs = ["\\ abcdefg1", "\\79 hijklmn1", "\\\\ opqrstu1"];

    // queries that should find all docs
    let match_all: Vec<Arc<dyn Query>> = vec![
        wq(field, "*")?,
        wq(field, "*1")?,
        wq(field, "**1")?,
        wq(field, "*?")?,
        wq(field, "*?1")?,
        wq(field, "?*1")?,
        wq(field, "**")?,
        wq(field, "***")?,
        wq(field, "\\\\*")?,
    ];

    // queries that should find no docs
    let match_none: Vec<Arc<dyn Query>> = vec![
        wq(field, "a*h")?,
        wq(field, "a?h")?,
        wq(field, "*a*h")?,
        wq(field, "?a")?,
        wq(field, "a?")?,
    ];

    let pq = |text: &str| -> Result<Arc<dyn Query>> {
        Ok(Arc::new(PrefixQuery::new(Term::from_text(field, text))?))
    };
    let match_one_doc_prefix: Vec<Vec<Arc<dyn Query>>> = vec![
        vec![pq("a")?, pq("ab")?, pq("abc")?], // these should find only doc 0
        vec![pq("h")?, pq("hi")?, pq("hij")?, pq("\\7")?], // these should find only doc 1
        vec![pq("o")?, pq("op")?, pq("opq")?, pq("\\\\")?], // these should find only doc 2
    ];

    let match_one_doc_wild: Vec<Vec<Arc<dyn Query>>> = vec![
        vec![
            wq(field, "*a*")?, // these should find only doc 0
            wq(field, "*ab*")?,
            wq(field, "*abc**")?,
            wq(field, "ab*e*")?,
            wq(field, "*g?")?,
            wq(field, "*f?1")?,
        ],
        vec![
            wq(field, "*h*")?, // these should find only doc 1
            wq(field, "*hi*")?,
            wq(field, "*hij**")?,
            wq(field, "hi*k*")?,
            wq(field, "*n?")?,
            wq(field, "*m?1")?,
            wq(field, "hij**")?,
        ],
        vec![
            wq(field, "*o*")?, // these should find only doc 2
            wq(field, "*op*")?,
            wq(field, "*opq**")?,
            wq(field, "op*q*")?,
            wq(field, "*u?")?,
            wq(field, "*t?1")?,
            wq(field, "opq**")?,
        ],
    ];

    // prepare the index
    let mut dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    for d in &docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            field,
            d,
            Store::NO,
        )));
        iw.add_document(&doc)?;
    }
    iw.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // test queries that must find all
    for q in &match_all {
        if verbose() {
            println!("matchAll: q={} {}", query_to_string(&**q), class_name(&**q));
        }
        let hits = searcher.search(Arc::clone(q), 1000)?.score_docs;
        assert_equals(&docs.len(), &hits.len());
    }

    // test queries that must find none
    for q in &match_none {
        if verbose() {
            println!(
                "matchNone: q={} {}",
                query_to_string(&**q),
                class_name(&**q)
            );
        }
        let hits = searcher.search(Arc::clone(q), 1000)?.score_docs;
        assert_equals(&0, &hits.len());
    }

    // thest the prefi queries find only one doc
    for i in 0..match_one_doc_prefix.len() {
        for j in 0..match_one_doc_prefix[i].len() {
            let q = &match_one_doc_prefix[i][j];
            if verbose() {
                println!(
                    "match 1 prefix: doc={} q={} {}",
                    docs[i],
                    query_to_string(&**q),
                    class_name(&**q)
                );
            }
            let hits = searcher.search(Arc::clone(q), 1000)?.score_docs;
            assert_equals(&1, &hits.len());
            assert_equals(&(i as i32), &hits[0].doc);
        }
    }

    // test the wildcard queries find only one doc
    for i in 0..match_one_doc_wild.len() {
        for j in 0..match_one_doc_wild[i].len() {
            let q = &match_one_doc_wild[i][j];
            if verbose() {
                println!(
                    "match 1 wild: doc={} q={} {}",
                    docs[i],
                    query_to_string(&**q),
                    class_name(&**q)
                );
            }
            let hits = searcher.search(Arc::clone(q), 1000)?.score_docs;
            assert_equals(&1, &hits.len());
            assert_equals(&(i as i32), &hits[0].doc);
        }
    }

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Tests large Wildcard queries
///
/// Port of `TestWildcardQuery.testLarge`.
fn test_large() -> Result<()> {
    // big string from a user
    let big = "{group-bm-http-server-02083.node.dm.reg,group-bm-http-server-02082.node.dm.reg,group-bm-http-server-02081.node.dm.reg,group-bm-http-server-02080.node.dm.reg,group-bm-http-server-02079.node.dm.reg,group-bm-http-server-02078.node.dm.reg,group-bm-http-server-02077.node.dm.reg,group-bm-http-server-02076.node.dm.reg,group-bm-http-server-02073.node.dm.reg,group-bm-http-server-02070.node.dm.reg,group-bm-http-server-02067.node.dm.reg,group-bm-http-server-02064.node.dm.reg,group-bm-http-server-02029.node.dm.reg,group-bm-http-server-02028.node.dm.reg,group-bm-http-server-02027.node.dm.reg,group-bm-http-server-02026.node.dm.reg,group-bm-http-server-02025.node.dm.reg,group-bm-http-server-02023.node.dm.reg,group-bm-http-server-02022.node.dm.reg,group-bm-http-server-02021.node.dm.reg,group-bm-http-server-02020.node.dm.reg,group-bm-http-server-02019.node.dm.reg,group-bm-http-server-02018.node.dm.reg,group-bm-http-server-02016.node.dm.reg,group-bm-http-server-02015.node.dm.reg,group-bm-http-server-02014.node.dm.reg,group-bm-http-server-02009.node.dm.reg,group-bm-http-server-02007.node.dm.reg,group-bm-http-server-02004.node.dm.reg,group-bm-http-server-02003.node.dm.reg,group-bm-http-server-02002.node.dm.reg,group-bm-http-server-01311.node.dm.reg,group-bm-http-server-01309.node.dm.reg,group-bm-http-server-01307.node.dm.reg}";
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "body",
        big,
        Store::YES,
    )));
    writer.add_document(&doc)?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query = wq("body", &format!("{big}*"))?;
    assert_matches(&searcher, &query, 1)?;

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Port of `TestWildcardQuery.testCostEstimate`.
fn test_cost_estimate() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for i in 0..1000 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "body",
            "foo bar",
            Store::NO,
        )));
        writer.add_document(&doc)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "body",
            "foo wuzzle",
            Store::NO,
        )));
        writer.add_document(&doc)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "body",
            &format!("bar {i}"),
            Store::NO,
        )));
        writer.add_document(&doc)?;
    }
    writer.flush()?;
    writer.force_merge(1)?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let lrc = &leaves[0];

    let query = wq("body", "foo*")?;
    let rewritten = searcher.rewrite(query)?;
    let weight = rewritten.create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0f32)?;
    let supplier = weight.scorer_supplier(lrc)?.expect("scorerSupplier");
    // Automaton queries have an unknown term count, so term collection is deferred to get() and the
    // cost is the worst-case estimate (sum of doc freqs across all terms) rather than the sum over
    // the matching terms only.
    assert_equals(&3000, &supplier.cost());

    let query = wq("body", "bar*")?;
    let rewritten = searcher.rewrite(query)?;
    let weight = rewritten.create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0f32)?;
    let supplier = weight.scorer_supplier(lrc)?.expect("scorerSupplier");
    assert_equals(&3000, &supplier.cost()); // Too many terms, assume worst-case all terms match

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// A leading wildcard is an automaton MultiTermQuery with an unknown term
/// count (getTermsCount() == -1). Building its ScorerSupplier must not scan
/// the term dictionary -- that is the cheap "planning" phase, and a leading
/// wildcard such as "*foo*" cannot seek, so collecting terms there would walk
/// the whole dictionary. The scan must be deferred to ScorerSupplier#get(),
/// so a parent conjunction can short-circuit (a sibling clause matching no
/// documents) before it runs.
///
/// Port of `TestWildcardQuery.testScorerSupplierDoesNotScanTermsEagerly`.
fn test_scorer_supplier_does_not_scan_terms_eagerly() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for i in 0..1000 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "body",
            &format!("foo {i}"),
            Store::NO,
        )));
        writer.add_document(&doc)?;
    }
    writer.flush()?;
    writer.force_merge(1)?;
    writer.close()?;

    let terms_enum_next_calls = Arc::new(AtomicI32::new(0));
    let reader = next_counting_reader_wrapper(
        directory_reader::open(dir.as_directory())?,
        Arc::clone(&terms_enum_next_calls),
    )?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let lrc = &leaves[0];

    // Leading wildcard => automaton query, getTermsCount() == -1.
    let query = wq("body", "*foo*")?;
    let rewritten = searcher.rewrite(query)?;
    let weight = rewritten.create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0f32)?;

    terms_enum_next_calls.store(0, Ordering::SeqCst);
    let supplier = weight.scorer_supplier(lrc)?;
    assert_not_null(&supplier);
    assert_equals_msg(
        Some("scorerSupplier() must not scan the term dictionary for an automaton MultiTermQuery"),
        &0,
        &terms_enum_next_calls.load(Ordering::SeqCst),
    );

    // The scan is deferred to get(): building the scorer is where the terms are actually walked.
    let mut supplier = supplier.expect("checked");
    let _scorer = supplier.get(i64::MAX)?;
    Assert::assert_true_msg(
        "get() should scan the term dictionary",
        terms_enum_next_calls.load(Ordering::SeqCst) > 0,
    );

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// `private static TermsEnum nextCountingTermsEnum(TermsEnum in, AtomicInteger counter)`.
fn next_counting_terms_enum(
    r#in: Box<dyn TermsEnum>,
    counter: Arc<AtomicI32>,
) -> Box<dyn TermsEnum> {
    Box::new(NextCountingTermsEnum {
        inner: FilterTermsEnum::new(r#in),
        counter,
    })
}

/// The anonymous `FilterTermsEnum` of `nextCountingTermsEnum`.
struct NextCountingTermsEnum {
    inner: FilterTermsEnum,
    counter: Arc<AtomicI32>,
}

impl fmt::Display for NextCountingTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl TermsEnum for NextCountingTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.inner.seek_exact(text)
    }

    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        self.inner.prepare_seek_exact(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        self.inner.next()
    }

    fn term(&self) -> Result<BytesRef> {
        self.inner.term()
    }

    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.inner.doc_freq()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.inner.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.inner.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }
}

/// Wraps a reader so every TermsEnum#next() (via iterator() or intersect())
/// is counted.
///
/// `new NextCountingReaderWrapper(in, counter)`: `private static class
/// NextCountingReaderWrapper extends FilterDirectoryReader`.
fn next_counting_reader_wrapper(
    r#in: Arc<dyn DirectoryReader>,
    counter: Arc<AtomicI32>,
) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(NextCountingSubReaderWrapper {
            counter: Arc::clone(&counter),
        }),
        Arc::new(NextCountingDirectoryReaderWrapper { counter }),
    )?))
}

/// `NextCountingReaderWrapper.doWrapDirectoryReader` and its
/// `getReaderCacheHelper()`, which returns `null`.
struct NextCountingDirectoryReaderWrapper {
    counter: Arc<AtomicI32>,
}

impl DirectoryReaderWrapper for NextCountingDirectoryReaderWrapper {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        next_counting_reader_wrapper(inner, Arc::clone(&self.counter))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "NextCountingReaderWrapper"
    }

    fn reader_cache_helper_is_null(&self) -> bool {
        true
    }
}

/// The anonymous `SubReaderWrapper` of `NextCountingReaderWrapper`.
struct NextCountingSubReaderWrapper {
    counter: Arc<AtomicI32>,
}

impl SubReaderWrapper for NextCountingSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(NextCountingLeafReader {
            inner: FilterLeafReader::new(reader)?,
            counter: Arc::clone(&self.counter),
        }))
    }
}

/// The anonymous `FilterTerms` of the counting leaf reader.
struct NextCountingTerms {
    inner: FilterTerms,
    counter: Arc<AtomicI32>,
}

impl Terms for NextCountingTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(next_counting_terms_enum(
            self.inner.get_delegate().iterator()?,
            Arc::clone(&self.counter),
        ))
    }

    fn intersect(
        &self,
        automaton: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        Ok(next_counting_terms_enum(
            self.inner.get_delegate().intersect(automaton, start_term)?,
            Arc::clone(&self.counter),
        ))
    }

    fn size(&self) -> Result<i64> {
        self.inner.size()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        self.inner.sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        self.inner.sum_doc_freq()
    }

    fn doc_count(&self) -> Result<i32> {
        self.inner.doc_count()
    }

    fn has_freqs(&self) -> bool {
        self.inner.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.inner.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.inner.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.inner.has_payloads()
    }

    fn stats(&self) -> Result<String> {
        self.inner.stats()
    }
}

/// The anonymous `FilterLeafReader` of `NextCountingReaderWrapper`, whose
/// cache helpers are `null`.
struct NextCountingLeafReader {
    inner: FilterLeafReader,
    counter: Arc<AtomicI32>,
}

impl fmt::Debug for NextCountingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NextCountingLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for NextCountingLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for NextCountingLeafReader {
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
        None
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let Some(terms) = self.inner.terms(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(NextCountingTerms {
            inner: FilterTerms::new(terms),
            counter: Arc::clone(&self.counter),
        })))
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
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_norm_values(field)
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
    fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
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
