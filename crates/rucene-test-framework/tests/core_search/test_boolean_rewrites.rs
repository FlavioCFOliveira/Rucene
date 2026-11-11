//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBooleanRewrites.java`.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::multi_reader::MultiReader;
use rucene::index::IndexReader;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::index_searcher::{IndexSearcher, IndexSearcherOverrides};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::phrase_query::Builder as PhraseQueryBuilder;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorer::FilterScorer;
use rucene::search::{
    query_to_string, BooleanQuery, BooleanQueryBuilder, BoostQuery, Occur, Query, ScoreMode,
    TopDocs,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::random_approximation_query::RandomApproximationQuery;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{field_mut_as, term, tq};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, class, random, test,
};

/// `TestBooleanRewrites`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBooleanRewrites",
        vec![
            test(
                "testOneClauseRewriteOptimization",
                test_one_clause_rewrite_optimization,
            ),
            test("testSingleFilterClause", test_single_filter_clause),
            test("testSingleMustMatchAll", test_single_must_match_all),
            test(
                "testSingleMustMatchAllWithShouldClauses",
                test_single_must_match_all_with_should_clauses,
            ),
            test(
                "testDeduplicateMustAndFilter",
                test_deduplicate_must_and_filter,
            ),
            test(
                "testConvertShouldAndFilterToMust",
                test_convert_should_and_filter_to_must,
            ),
            test(
                "testDuplicateMustOrFilterWithMustNot",
                test_duplicate_must_or_filter_with_must_not,
            ),
            test("testMatchAllMustNot", test_match_all_must_not),
            test(
                "testDeeplyNestedBooleanRewriteShouldClauses",
                test_deeply_nested_boolean_rewrite_should_clauses,
            ),
            test(
                "testDeeplyNestedBooleanRewrite",
                test_deeply_nested_boolean_rewrite,
            ),
            test("testRemoveMatchAllFilter", test_remove_match_all_filter),
            test("testRandom", test_random),
            test(
                "testDeduplicateShouldClauses",
                test_deduplicate_should_clauses,
            ),
            test("testDeduplicateMustClauses", test_deduplicate_must_clauses),
            test(
                "testFlattenInnerDisjunctions",
                test_flatten_inner_disjunctions,
            ),
            test(
                "testFlattenInnerConjunctions",
                test_flatten_inner_conjunctions,
            ),
            test(
                "testFlattenDisjunctionInMustClause",
                test_flatten_disjunction_in_must_clause,
            ),
            test("testDiscardShouldClauses", test_discard_should_clauses),
            test(
                "testShouldMatchNoDocsQuery",
                test_should_match_no_docs_query,
            ),
            test(
                "testMustNotMatchNoDocsQuery",
                test_must_not_match_no_docs_query,
            ),
            test("testMustMatchNoDocsQuery", test_must_match_no_docs_query),
            test(
                "testFilterMatchNoDocsQuery",
                test_filter_match_no_docs_query,
            ),
            test("testEmptyBoolean", test_empty_boolean),
            test("testSimplifyFilterClauses", test_simplify_filter_clauses),
            test("testSimplifyMustNotClauses", test_simplify_must_not_clauses),
            test(
                "testSimplifyNonScoringShouldClauses",
                test_simplify_non_scoring_should_clauses,
            ),
            test(
                "testShouldClausesLessThanOrEqualToMinimumNumberShouldMatch",
                test_should_clauses_less_than_or_equal_to_minimum_number_should_match,
            ),
        ],
    )
}

/// `new BooleanQuery.Builder().setMinimumNumberShouldMatch(msm).add(...)...build()`.
fn bq(clauses: &[(Arc<dyn Query>, Occur)], msm: i32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(bq_built(clauses, msm)?))
}

fn bq_built(clauses: &[(Arc<dyn Query>, Occur)], msm: i32) -> Result<BooleanQuery> {
    let mut b = BooleanQueryBuilder::new();
    b.set_minimum_number_should_match(msm);
    for (q, o) in clauses {
        b.add(Arc::clone(q), *o)?;
    }
    Ok(b.build())
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

fn match_none() -> Arc<dyn Query> {
    // `MatchNoDocsQuery.INSTANCE`; `equals` is `sameClassAs`, so the reason
    // does not take part in the comparisons.
    Arc::new(MatchNoDocsQuery::instance())
}

fn csq(q: Arc<dyn Query>) -> Arc<dyn Query> {
    Arc::new(ConstantScoreQuery::new(q))
}

fn boost(q: Arc<dyn Query>, b: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(q, b)?))
}

/// `newSearcher(new MultiReader())`.
fn empty_searcher() -> Result<IndexSearcher> {
    Ok(LuceneTestCase::new_searcher(Arc::new(MultiReader::new(
        Vec::new(),
        true,
    )?)))
}

/// `assertEquals(expected, searcher.rewrite(query))`.
fn assert_rewrite(
    searcher: &IndexSearcher,
    expected: &Arc<dyn Query>,
    query: &Arc<dyn Query>,
) -> Result<()> {
    assert_query_equals(expected, &searcher.rewrite(Arc::clone(query))?);
    Ok(())
}

/// `assertSame(query, searcher.rewrite(query))`: the searcher hands back the
/// very instance it was given.
fn assert_rewrite_same(searcher: &IndexSearcher, query: &Arc<dyn Query>) -> Result<()> {
    let rewritten = searcher.rewrite(Arc::clone(query))?;
    Assert::assert_true(std::ptr::addr_eq(
        Arc::as_ptr(query),
        Arc::as_ptr(&rewritten),
    ));
    Ok(())
}

fn test_one_clause_rewrite_optimization() -> Result<()> {
    const FIELD: &str = "content";
    const VALUE: &str = "foo";

    let dir = LuceneTestCase::new_directory();
    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?.close()?;
    let r = directory_reader::open(dir.as_directory())?;

    let expected = tq(FIELD, VALUE);

    let num_layers = LuceneTestCase::at_least(3);
    let mut actual = tq(FIELD, VALUE);

    for _i in 0..num_layers {
        let mut bq = BooleanQueryBuilder::new();
        bq.add(
            actual,
            if random().next_boolean() {
                Occur::SHOULD
            } else {
                Occur::MUST
            },
        )?;
        actual = Arc::new(bq.build());
    }

    let rewritten =
        IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?.rewrite(Arc::clone(&actual))?;
    Assert::assert_true_msg(
        &format!(
            "{}: {} expected:<{}> but was:<{}>",
            num_layers,
            query_to_string(&*actual),
            query_to_string(&*expected),
            query_to_string(&*rewritten)
        ),
        expected.query_eq(&*rewritten),
    );

    r.close()?;
    dir.close()?;
    Ok(())
}

fn test_single_filter_clause() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let f = LuceneTestCase::new_text_field("field", "a", Store::NO);
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    w.commit()?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    let mut query1 = BooleanQueryBuilder::new();
    query1.add(tq("field", "a"), Occur::FILTER)?;

    // Single clauses rewrite to a term query
    let built1: Arc<dyn Query> = Arc::new(query1.build());
    let rewritten1 = built1
        .rewrite(&searcher)?
        .unwrap_or_else(|| Arc::clone(&built1));
    let boost_query = rewritten1.as_any().downcast_ref::<BoostQuery>();
    Assert::assert_true(boost_query.is_some());
    assert_equals_delta(0.0, boost_query.expect("checked").get_boost() as f64, 0.0);

    // When there are two clauses, we cannot rewrite, but if one of them creates
    // a null scorer we will end up with a single filter scorer and will need to
    // make sure to set score=0
    let mut query2 = BooleanQueryBuilder::new();
    query2.add(tq("field", "a"), Occur::FILTER)?;
    query2.add(tq("missing_field", "b"), Occur::SHOULD)?;
    let weight = searcher.create_weight(
        searcher.rewrite(Arc::new(query2.build()))?,
        ScoreMode::COMPLETE,
        1.0,
    )?;
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let mut scorer = weight.scorer(&leaves[0])?.expect("scorer");
    assert_equals(&0, &scorer.iterator().next_doc()?);
    // `scorer.getClass().getName()` as the message: Rucene has no run-time
    // class names, so the one class the test expects is a declared name.
    let is_filter_scorer = scorer.as_any().is_some_and(|a| a.is::<FilterScorer>());
    Assert::assert_true_msg(
        if is_filter_scorer {
            "org.apache.lucene.search.FilterScorer"
        } else {
            "<unlisted Scorer>"
        },
        is_filter_scorer,
    );
    assert_equals_delta(0.0, scorer.score()? as f64, 0.0);

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_single_must_match_all() -> Result<()> {
    let searcher = empty_searcher()?;

    let q = bq(
        &[
            (match_all(), Occur::MUST),
            (tq("foo", "bar"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &csq(tq("foo", "bar")), &q)?;

    let q = bq(
        &[
            (boost(match_all(), 42.0)?, Occur::MUST),
            (tq("foo", "bar"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &boost(csq(tq("foo", "bar")), 42.0)?, &q)?;

    let q = bq(
        &[(match_all(), Occur::MUST), (match_all(), Occur::FILTER)],
        0,
    )?;
    assert_rewrite(&searcher, &match_all(), &q)?;

    let q = bq(
        &[
            (boost(match_all(), 42.0)?, Occur::MUST),
            (match_all(), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &boost(match_all(), 42.0)?, &q)?;

    let q = bq(
        &[
            (match_all(), Occur::MUST),
            (tq("foo", "bar"), Occur::MUST_NOT),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &q, &q)?;

    let q = bq(
        &[(match_all(), Occur::MUST), (match_all(), Occur::FILTER)],
        0,
    )?;
    assert_rewrite(&searcher, &match_all(), &q)?;

    let q = bq(
        &[
            (match_all(), Occur::MUST),
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &csq(expected), &q)?;

    let q = bq(
        &[
            (match_all(), Occur::MUST),
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::MUST_NOT),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::MUST_NOT),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &csq(expected), &q)?;

    let q = bq(
        &[
            (match_all(), Occur::MUST),
            (tq("foo", "bar"), Occur::SHOULD),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &q, &q)?;
    Ok(())
}

fn test_single_must_match_all_with_should_clauses() -> Result<()> {
    let searcher = empty_searcher()?;

    let q = bq(
        &[
            (match_all(), Occur::MUST),
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (csq(tq("foo", "bar")), Occur::MUST),
            (tq("foo", "baz"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected, &q)
}

fn test_deduplicate_must_and_filter() -> Result<()> {
    let searcher = empty_searcher()?;

    let q = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "bar"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &tq("foo", "bar"), &q)?;

    let q = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected, &q)
}

// Duplicate Should and Filter query is converted to Must (with minShouldMatch -1)
fn test_convert_should_and_filter_to_must() -> Result<()> {
    let searcher = empty_searcher()?;

    // no minShouldMatch
    let q = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "bar"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &tq("foo", "bar"), &q)?;

    // minShouldMatch is set to -1
    let q = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::SHOULD),
            (tq("foo", "quz"), Occur::SHOULD),
        ],
        2,
    )?;

    let expected = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::SHOULD),
            (tq("foo", "quz"), Occur::SHOULD),
        ],
        1,
    )?;
    assert_rewrite(&searcher, &expected, &q)
}

// Duplicate Must or Filter with MustNot returns no match
fn test_duplicate_must_or_filter_with_must_not() -> Result<()> {
    let searcher = empty_searcher()?;

    // Test Must with MustNot
    let q = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            // other terms
            (tq("foo", "baz"), Occur::MUST),
            (tq("foo", "bad"), Occur::SHOULD),
            //
            (tq("foo", "bar"), Occur::MUST_NOT),
        ],
        0,
    )?;

    assert_rewrite(&searcher, &match_none(), &q)?;

    // Test Filter with MustNot
    let q2 = bq(
        &[
            (tq("foo", "bar"), Occur::FILTER),
            // other terms
            (tq("foo", "baz"), Occur::MUST),
            (tq("foo", "bad"), Occur::SHOULD),
            //
            (tq("foo", "bar"), Occur::MUST_NOT),
        ],
        0,
    )?;

    assert_rewrite(&searcher, &match_none(), &q2)
}

// MatchAllQuery as MUST_NOT clause cannot return anything
fn test_match_all_must_not() -> Result<()> {
    let searcher = empty_searcher()?;

    // Test Must with MatchAll MustNot
    let q = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::FILTER),
            (tq("foo", "bad"), Occur::SHOULD),
            //
            (match_all(), Occur::MUST_NOT),
        ],
        0,
    )?;

    assert_rewrite(&searcher, &match_none(), &q)?;

    // Test Must with MatchAll MustNot and other MustNot
    let q2 = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::FILTER),
            (tq("foo", "bad"), Occur::SHOULD),
            //
            (tq("foo", "bor"), Occur::MUST_NOT),
            (match_all(), Occur::MUST_NOT),
        ],
        0,
    )?;

    assert_rewrite(&searcher, &match_none(), &q2)
}

/// `termQueryFunction`: `(i) -> new TermQuery(new Term("layer[" + i + "]", "foo"))`.
fn layer_term_query(i: i32) -> Arc<dyn Query> {
    tq(&format!("layer[{i}]"), "foo")
}

fn test_deeply_nested_boolean_rewrite_should_clauses() -> Result<()> {
    let searcher = empty_searcher()?;
    let depth = TestUtil::next_int(&*random(), 10, 30);
    let rewrite_query_expected = Arc::new(TestRewriteQuery::default());
    let rewrite_query = Arc::new(TestRewriteQuery::default());
    let mut expected_query_builder = BooleanQueryBuilder::new();
    expected_query_builder.add(
        Arc::clone(&rewrite_query_expected) as Arc<dyn Query>,
        Occur::FILTER,
    )?;
    let mut deep_builder = bq(
        &[(Arc::clone(&rewrite_query) as Arc<dyn Query>, Occur::SHOULD)],
        1,
    )?;
    let mut i = depth;
    while i > 0 {
        let t = layer_term_query(i);
        deep_builder = bq(
            &[
                (Arc::clone(&t), Occur::SHOULD),
                (deep_builder, Occur::SHOULD),
            ],
            2,
        )?;
        expected_query_builder.add(t, Occur::FILTER)?;
        if i == depth {
            expected_query_builder
                .add(Arc::clone(&rewrite_query) as Arc<dyn Query>, Occur::FILTER)?;
        }
        i -= 1;
    }
    let q = bq(&[(deep_builder, Occur::FILTER)], 0)?;
    let expected_query = boost(csq(Arc::new(expected_query_builder.build())), 0.0)?;
    let rewritten = searcher.rewrite(q)?;
    assert_query_equals(&expected_query, &rewritten);
    // the SHOULD clauses cause more rewrites because they incrementally change to `MUST` and then
    // `FILTER`, plus the flattening of required clauses
    assert_equals_msg(
        Some(&format!("Depth={depth}")),
        &(depth * 2),
        &rewrite_query.num_rewrites.load(Ordering::SeqCst),
    );
    Ok(())
}

fn test_deeply_nested_boolean_rewrite() -> Result<()> {
    let searcher = empty_searcher()?;
    let depth = TestUtil::next_int(&*random(), 10, 30);
    let rewrite_query_expected = Arc::new(TestRewriteQuery::default());
    let rewrite_query = Arc::new(TestRewriteQuery::default());
    let mut expected_query_builder = BooleanQueryBuilder::new();
    expected_query_builder.add(
        Arc::clone(&rewrite_query_expected) as Arc<dyn Query>,
        Occur::FILTER,
    )?;
    let mut deep_builder = bq(
        &[(Arc::clone(&rewrite_query) as Arc<dyn Query>, Occur::MUST)],
        0,
    )?;
    let mut i = depth;
    while i > 0 {
        let t = layer_term_query(i);
        deep_builder = bq(
            &[(Arc::clone(&t), Occur::MUST), (deep_builder, Occur::MUST)],
            0,
        )?;
        expected_query_builder.add(t, Occur::FILTER)?;
        if i == depth {
            expected_query_builder
                .add(Arc::clone(&rewrite_query) as Arc<dyn Query>, Occur::FILTER)?;
        }
        i -= 1;
    }
    let q = bq(&[(deep_builder, Occur::FILTER)], 0)?;
    let expected_query = boost(csq(Arc::new(expected_query_builder.build())), 0.0)?;
    let rewritten = searcher.rewrite(q)?;
    assert_query_equals(&expected_query, &rewritten);
    // `depth` rewrites because of the flattening
    assert_equals_msg(
        Some(&format!("Depth={depth}")),
        &depth,
        &rewrite_query.num_rewrites.load(Ordering::SeqCst),
    );
    Ok(())
}

fn test_remove_match_all_filter() -> Result<()> {
    let searcher = empty_searcher()?;

    let q = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (match_all(), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &tq("foo", "bar"), &q)?;

    let q = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::MUST),
            (match_all(), Occur::FILTER),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected, &q)?;

    let q = bq(
        &[
            (tq("foo", "bar"), Occur::FILTER),
            (match_all(), Occur::FILTER),
        ],
        0,
    )?;
    let expected = boost(csq(tq("foo", "bar")), 0.0)?;
    assert_rewrite(&searcher, &expected, &q)?;

    let q = bq(
        &[(match_all(), Occur::FILTER), (match_all(), Occur::FILTER)],
        0,
    )?;
    let expected = boost(csq(match_all()), 0.0)?;
    assert_rewrite(&searcher, &expected, &q)
}

/// The anonymous `IndexSearcher` of `testRandom` whose `rewrite` is a no-op.
#[derive(Debug)]
struct NoRewriteSearcher;

impl IndexSearcherOverrides for NoRewriteSearcher {
    fn rewrite(
        &self,
        _searcher: &IndexSearcher,
        original: Arc<dyn Query>,
    ) -> Result<Arc<dyn Query>> {
        // no-op: disable rewriting
        Ok(original)
    }
}

fn test_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let f = TextField::new("body", "a b c".to_string(), Store::NO)?;
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    field_mut_as::<TextField>(&mut doc, "body").set_string_value(String::new())?;
    w.add_document(&doc)?;
    field_mut_as::<TextField>(&mut doc, "body").set_string_value("a b".to_string())?;
    w.add_document(&doc)?;
    field_mut_as::<TextField>(&mut doc, "body").set_string_value("b c".to_string())?;
    w.add_document(&doc)?;
    field_mut_as::<TextField>(&mut doc, "body").set_string_value("a".to_string())?;
    w.add_document(&doc)?;
    field_mut_as::<TextField>(&mut doc, "body").set_string_value("c".to_string())?;
    w.add_document(&doc)?;
    let num_random_docs = LuceneTestCase::at_least(3);
    for _i in 0..num_random_docs {
        let num_terms = random().next_int_bound(20);
        let mut text = String::new();
        for _j in 0..num_terms {
            text.push((b'a' + random().next_int_bound(4) as u8) as char);
            text.push(' ');
        }
        field_mut_as::<TextField>(&mut doc, "body").set_string_value(text)?;
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    w.close()?;
    let searcher1 = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let mut searcher2 = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher2.set_overrides(Arc::new(NoRewriteSearcher));
    searcher2.set_similarity(Arc::clone(searcher1.get_similarity()));

    let iters = LuceneTestCase::at_least(1000);
    let r = random();
    for _i in 0..iters {
        let mut query = random_boolean_query(&r)?;
        let td1 = searcher1.search(Arc::clone(&query), 100)?;
        let td2 = searcher2.search(Arc::clone(&query), 100)?;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_top_docs_equals(&td1, &td2)
        }));
        if let Err(e) = outcome {
            println!("{}", query_to_string(&*query));
            let mut rewritten = Arc::clone(&query);
            loop {
                query = rewritten;
                rewritten = query
                    .rewrite(&searcher1)?
                    .unwrap_or_else(|| Arc::clone(&query));
                println!("{}", query_to_string(&*rewritten));
                let tdx = searcher2.search(Arc::clone(&rewritten), 100)?;
                if td2.total_hits.value() != tdx.total_hits.value() {
                    println!("Bad");
                }
                if std::ptr::addr_eq(Arc::as_ptr(&query), Arc::as_ptr(&rewritten)) {
                    break;
                }
            }
            std::panic::resume_unwind(e);
        }
    }

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `Occur.values()`, in declaration order.
const OCCUR_VALUES: [Occur; 4] = [Occur::MUST, Occur::FILTER, Occur::SHOULD, Occur::MUST_NOT];

fn random_boolean_query(random: &Arc<dyn Random>) -> Result<Arc<dyn Query>> {
    let num_clauses = random.next_int_bound(5);
    let mut b = BooleanQueryBuilder::new();
    let mut num_shoulds = 0;
    for _i in 0..num_clauses {
        let occur = OCCUR_VALUES[random.next_int_bound(OCCUR_VALUES.len() as i32) as usize];
        if occur == Occur::SHOULD {
            num_shoulds += 1;
        }
        let query = random_query(random)?;
        b.add(query, occur)?;
    }
    b.set_minimum_number_should_match(if crate::core_support::random().next_boolean() {
        0
    } else {
        TestUtil::next_int(&*crate::core_support::random(), 0, num_shoulds + 1)
    });
    let mut query: Arc<dyn Query> = Arc::new(b.build());
    if random.next_boolean() {
        query = random_wrapper(random, query)?;
    }
    Ok(query)
}

fn random_wrapper(random: &Arc<dyn Random>, query: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    match random.next_int_bound(3) {
        0 => boost(query, TestUtil::next_int(&**random, 0, 4) as f32),
        1 => Ok(csq(query)),
        2 => Ok(Arc::new(RandomApproximationQuery::new(
            query,
            Arc::clone(random),
        ))),
        _ => unreachable!("throw new AssertionError()"),
    }
}

fn random_query(random: &Arc<dyn Random>) -> Result<Arc<dyn Query>> {
    if random.next_int_bound(5) == 0 {
        return random_wrapper(random, random_query(random)?);
    }
    match crate::core_support::random().next_int_bound(6) {
        0 => Ok(match_all()),
        1 => Ok(tq("body", "a")),
        2 => Ok(tq("body", "b")),
        3 => Ok(tq("body", "c")),
        4 => Ok(tq("body", "d")),
        5 => random_boolean_query(random),
        _ => unreachable!("throw new AssertionError()"),
    }
}

/// The private `assertEquals(TopDocs td1, TopDocs td2)`.
fn assert_top_docs_equals(td1: &TopDocs, td2: &TopDocs) {
    assert_equals(&td1.total_hits.value(), &td2.total_hits.value());
    assert_equals(&td1.score_docs.len(), &td2.score_docs.len());
    let expected_scores: HashMap<i32, f32> =
        td1.score_docs.iter().map(|sd| (sd.doc, sd.score)).collect();
    let actual_result_set: HashSet<i32> = td2.score_docs.iter().map(|sd| sd.doc).collect();

    let expected_keys: HashSet<i32> = expected_scores.keys().copied().collect();
    assert_equals_msg(
        Some("Set of matching documents differs"),
        &expected_keys,
        &actual_result_set,
    );

    for score_doc in &td2.score_docs {
        let expected_score = expected_scores[&score_doc.doc];
        let actual_score = score_doc.score;
        assert_equals_delta(
            expected_score as f64,
            actual_score as f64,
            (expected_score / 100.0) as f64,
        ); // error under 1%
    }
}

fn test_deduplicate_should_clauses() -> Result<()> {
    let searcher = empty_searcher()?;

    let query = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "bar"), Occur::SHOULD),
        ],
        0,
    )?;
    let expected = boost(tq("foo", "bar"), 2.0)?;
    assert_rewrite(&searcher, &expected, &query)?;

    let query = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (boost(tq("foo", "bar"), 2.0)?, Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (boost(tq("foo", "bar"), 3.0)?, Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected, &query)?;

    let query = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
        ],
        2,
    )?;
    let expected = Arc::clone(&query);
    assert_rewrite(&searcher, &expected, &query)
}

fn test_deduplicate_must_clauses() -> Result<()> {
    let searcher = empty_searcher()?;

    let query = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "bar"), Occur::MUST),
        ],
        0,
    )?;
    let expected = boost(tq("foo", "bar"), 2.0)?;
    assert_rewrite(&searcher, &expected, &query)?;

    let query = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (boost(tq("foo", "bar"), 2.0)?, Occur::MUST),
            (tq("foo", "quux"), Occur::MUST),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (boost(tq("foo", "bar"), 3.0)?, Occur::MUST),
            (tq("foo", "quux"), Occur::MUST),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected, &query)
}

fn test_flatten_inner_disjunctions() -> Result<()> {
    let searcher = empty_searcher()?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
        ],
        0,
    )?;
    let query = bq(
        &[
            (Arc::clone(&inner), Occur::SHOULD),
            (tq("foo", "baz"), Occur::SHOULD),
        ],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
            (tq("foo", "baz"), Occur::SHOULD),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let query = bq(
        &[
            (Arc::clone(&inner), Occur::SHOULD),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let query = bq(
        &[
            (Arc::clone(&inner), Occur::SHOULD),
            (tq("foo", "baz"), Occur::MUST),
        ],
        1,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
            (tq("foo", "baz"), Occur::MUST),
        ],
        1,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let query = bq(
        &[
            (Arc::clone(&inner), Occur::SHOULD),
            (tq("foo", "baz"), Occur::MUST),
        ],
        2,
    )?;
    assert_rewrite(&searcher, &match_none(), &query)?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
            (tq("foo", "baz"), Occur::SHOULD),
        ],
        2,
    )?;
    let query = bq(
        &[(inner, Occur::SHOULD), (tq("foo", "baz"), Occur::SHOULD)],
        0,
    )?;
    assert_rewrite_same(&searcher, &query)
}

fn test_flatten_inner_conjunctions() -> Result<()> {
    let searcher = empty_searcher()?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::MUST),
        ],
        0,
    )?;
    let query = bq(
        &[
            (Arc::clone(&inner), Occur::MUST),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::MUST),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let query = bq(
        &[
            (Arc::clone(&inner), Occur::MUST),
            (tq("foo", "baz"), Occur::SHOULD),
        ],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::MUST),
            (tq("foo", "baz"), Occur::SHOULD),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let query = bq(
        &[
            (Arc::clone(&inner), Occur::MUST),
            (tq("foo", "baz"), Occur::MUST_NOT),
        ],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::MUST),
            (tq("foo", "baz"), Occur::MUST_NOT),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::FILTER),
        ],
        0,
    )?;
    let query = bq(
        &[
            (Arc::clone(&inner), Occur::MUST),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::FILTER),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::FILTER),
        ],
        0,
    )?;
    let query = bq(
        &[
            (Arc::clone(&inner), Occur::FILTER),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "quux"), Occur::FILTER),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "quux"), Occur::MUST_NOT),
        ],
        0,
    )?;
    let query = bq(
        &[(inner, Occur::FILTER), (tq("foo", "baz"), Occur::MUST)],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "quux"), Occur::MUST_NOT),
            (tq("foo", "baz"), Occur::MUST),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)
}

fn test_flatten_disjunction_in_must_clause() -> Result<()> {
    let searcher = empty_searcher()?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
        ],
        0,
    )?;
    let query = bq(
        &[(inner, Occur::MUST), (tq("foo", "baz"), Occur::FILTER)],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        1,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)?;

    let inner = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
            (tq("foo", "foo"), Occur::SHOULD),
        ],
        2,
    )?;
    let query = bq(
        &[(inner, Occur::MUST), (tq("foo", "baz"), Occur::FILTER)],
        0,
    )?;
    let expected_rewritten = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "quux"), Occur::SHOULD),
            (tq("foo", "foo"), Occur::SHOULD),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        2,
    )?;
    assert_rewrite(&searcher, &expected_rewritten, &query)
}

fn test_discard_should_clauses() -> Result<()> {
    let searcher = empty_searcher()?;

    let query1 = csq(bq(
        &[
            (tq("field", "a"), Occur::MUST),
            (tq("field", "b"), Occur::SHOULD),
        ],
        0,
    )?);
    let rewritten1 = csq(tq("field", "a"));
    assert_rewrite(&searcher, &rewritten1, &query1)?;

    let query2 = csq(bq(
        &[
            (tq("field", "a"), Occur::MUST),
            (tq("field", "b"), Occur::SHOULD),
            (tq("field", "c"), Occur::FILTER),
        ],
        0,
    )?);
    let rewritten2 = csq(bq(
        &[
            (tq("field", "a"), Occur::FILTER),
            (tq("field", "c"), Occur::FILTER),
        ],
        0,
    )?);
    assert_rewrite(&searcher, &rewritten2, &query2)?;

    let query3 = csq(bq(
        &[
            (tq("field", "a"), Occur::SHOULD),
            (tq("field", "b"), Occur::SHOULD),
        ],
        0,
    )?);
    assert_rewrite_same(&searcher, &query3)?;

    let query4 = csq(bq(
        &[
            (tq("field", "a"), Occur::SHOULD),
            (tq("field", "b"), Occur::MUST_NOT),
        ],
        0,
    )?);
    assert_rewrite_same(&searcher, &query4)?;

    let query5 = csq(bq(
        &[
            (tq("field", "a"), Occur::SHOULD),
            (tq("field", "b"), Occur::SHOULD),
            (tq("field", "c"), Occur::FILTER),
        ],
        1,
    )?);
    assert_rewrite_same(&searcher, &query5)
}

fn test_should_match_no_docs_query() -> Result<()> {
    let searcher = empty_searcher()?;
    let query = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (match_none(), Occur::SHOULD),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &tq("foo", "bar"), &query)
}

fn test_must_not_match_no_docs_query() -> Result<()> {
    let searcher = empty_searcher()?;
    let query = bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (match_none(), Occur::MUST_NOT),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &tq("foo", "bar"), &query)
}

fn test_must_match_no_docs_query() -> Result<()> {
    let searcher = empty_searcher()?;
    let query = bq(
        &[(tq("foo", "bar"), Occur::MUST), (match_none(), Occur::MUST)],
        0,
    )?;
    assert_rewrite(&searcher, &match_none(), &query)
}

fn test_filter_match_no_docs_query() -> Result<()> {
    let searcher = empty_searcher()?;
    let query = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (match_none(), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &match_none(), &query)
}

fn test_empty_boolean() -> Result<()> {
    let searcher = empty_searcher()?;
    let query: Arc<dyn Query> = Arc::new(BooleanQueryBuilder::new().build());
    assert_rewrite(&searcher, &match_none(), &query)
}

fn test_simplify_filter_clauses() -> Result<()> {
    let searcher = empty_searcher()?;

    let query1 = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (csq(tq("foo", "baz")), Occur::FILTER),
        ],
        0,
    )?;
    let expected1 = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::FILTER),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected1, &query1)?;

    let query2 = bq(
        &[
            (tq("foo", "bar"), Occur::FILTER),
            (csq(tq("foo", "bar")), Occur::FILTER),
        ],
        0,
    )?;
    let expected2 = boost(csq(tq("foo", "bar")), 0.0)?;
    assert_rewrite(&searcher, &expected2, &query2)
}

fn test_simplify_must_not_clauses() -> Result<()> {
    let searcher = empty_searcher()?;

    let query = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (csq(tq("foo", "baz")), Occur::MUST_NOT),
        ],
        0,
    )?;
    let expected = bq(
        &[
            (tq("foo", "bar"), Occur::MUST),
            (tq("foo", "baz"), Occur::MUST_NOT),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected, &query)
}

fn test_simplify_non_scoring_should_clauses() -> Result<()> {
    let searcher = empty_searcher()?;

    let query = csq(bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (csq(tq("foo", "baz")), Occur::SHOULD),
        ],
        0,
    )?);
    let expected = csq(bq(
        &[
            (tq("foo", "bar"), Occur::SHOULD),
            (tq("foo", "baz"), Occur::SHOULD),
        ],
        0,
    )?);
    assert_rewrite(&searcher, &expected, &query)
}

/// `new PhraseQuery.Builder().add(new Term(field, t))...build()`.
fn phrase(terms: &[&str]) -> Result<Arc<dyn Query>> {
    let mut b = PhraseQueryBuilder::new();
    for t in terms {
        b.add(term("field", t))?;
    }
    Ok(Arc::new(b.build()?))
}

fn test_should_clauses_less_than_or_equal_to_minimum_number_should_match() -> Result<()> {
    let searcher = empty_searcher()?;

    // The only one SHOULD clause is MatchNoDocsQuery
    let query = bq(&[(phrase(&[])?, Occur::SHOULD)], 1)?;
    assert_rewrite(&searcher, &match_none(), &query)?;
    let query = bq(&[(phrase(&[])?, Occur::SHOULD)], 0)?;
    assert_rewrite(&searcher, &match_none(), &query)?;

    // Meaningful SHOULD clause count is less than MinimumNumberShouldMatch
    let query = bq(
        &[
            (phrase(&[])?, Occur::SHOULD),
            (phrase(&["a"])?, Occur::SHOULD),
        ],
        2,
    )?;
    assert_rewrite(&searcher, &match_none(), &query)?;

    // Meaningful SHOULD clause count is equal to MinimumNumberShouldMatch
    let query = bq(
        &[
            (phrase(&["b"])?, Occur::SHOULD),
            (phrase(&["a", "c"])?, Occur::SHOULD),
        ],
        2,
    )?;
    let expected = bq(
        &[
            (tq("field", "b"), Occur::MUST),
            (phrase(&["a", "c"])?, Occur::MUST),
        ],
        0,
    )?;
    assert_rewrite(&searcher, &expected, &query)?;

    // Invalid Inner query get removed after rewrite
    let inner = bq(
        &[
            (phrase(&[])?, Occur::SHOULD),
            (phrase(&["a"])?, Occur::SHOULD),
        ],
        2,
    )?;

    let query = bq(
        &[
            (Arc::clone(&inner), Occur::SHOULD),
            (phrase(&["b"])?, Occur::SHOULD),
            (phrase(&["a", "c"])?, Occur::SHOULD),
        ],
        2,
    )?;
    assert_rewrite(&searcher, &expected, &query)?;

    let query = bq(
        &[(inner, Occur::SHOULD), (phrase(&["b"])?, Occur::SHOULD)],
        2,
    )?;
    assert_rewrite(&searcher, &match_none(), &query)
}

/// Test query to count number of rewrites for its lifetime.
#[derive(Debug, Default)]
struct TestRewriteQuery {
    num_rewrites: AtomicI32,
}

impl Query for TestRewriteQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "TestRewriteQuery{{rewrites={}}}",
            self.num_rewrites.load(Ordering::SeqCst)
        )
    }

    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        self.num_rewrites.fetch_add(1, Ordering::SeqCst);
        // `return this`
        Ok(None)
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn rucene::search::weight::Weight>> {
        // `Query.createWeight` default: `throw new UnsupportedOperationException(
        // "Query " + this + " does not implement createWeight")`.
        Err(rucene::error::LuceneError::unsupported_operation(format!(
            "Query {} does not implement createWeight",
            query_to_string(self)
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        // `obj instanceof TestRewriteQuery`
        other.as_any().is::<TestRewriteQuery>()
    }

    fn query_hash(&self) -> u64 {
        // `TestRewriteQuery.class.hashCode()`
        self.class_hash()
    }
}
