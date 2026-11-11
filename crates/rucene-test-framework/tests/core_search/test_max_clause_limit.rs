//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMaxClauseLimit.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::multi_reader::MultiReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQueryBuilder, DisjunctionMaxQuery, Occur, Query};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::term;
use crate::core_support::{assert_equals_msg, class, test, IAE};

/// `IndexSearcher.TooManyClauses`.
const TOO_MANY_CLAUSES: &str = "org.apache.lucene.search.IndexSearcher$TooManyClauses";
/// `IndexSearcher.TooManyNestedClauses`.
const TOO_MANY_NESTED_CLAUSES: &str = "org.apache.lucene.search.IndexSearcher$TooManyNestedClauses";

/// `TestMaxClauseLimit`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMaxClauseLimit",
        vec![
            test(
                "testIllegalArgumentExceptionOnZero",
                test_illegal_argument_exception_on_zero,
            ),
            test(
                "testFlattenInnerDisjunctionsWithMoreThan1024Terms",
                test_flatten_inner_disjunctions_with_more_than1024_terms,
            ),
            test("testLargeTermsNestedFirst", test_large_terms_nested_first),
            test("testLargeTermsNestedLast", test_large_terms_nested_last),
            test(
                "testLargeDisjunctionMaxQuery",
                test_large_disjunction_max_query,
            ),
            test("testMultiExactWithRepeats", test_multi_exact_with_repeats),
        ],
    )
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `newSearcher(new MultiReader())`.
fn empty_searcher() -> Result<IndexSearcher> {
    Ok(LuceneTestCase::new_searcher(Arc::new(MultiReader::new(
        Vec::new(),
        true,
    )?)))
}

fn test_illegal_argument_exception_on_zero() -> Result<()> {
    let current = IndexSearcher::get_max_clause_count();
    expect_throws(IAE, || IndexSearcher::set_max_clause_count(0));
    assert_equals_msg(
        Some("attempt to change to 0 should have failed w/o modifying"),
        &current,
        &IndexSearcher::get_max_clause_count(),
    );
    Ok(())
}

fn test_flatten_inner_disjunctions_with_more_than1024_terms() -> Result<()> {
    let searcher = empty_searcher()?;

    let mut builder1024 = BooleanQueryBuilder::new();
    for i in 0..1024 {
        builder1024.add(tq("foo", &format!("bar-{i}")), Occur::SHOULD)?;
    }
    let inner: Arc<dyn Query> = Arc::new(builder1024.build());
    let mut query = BooleanQueryBuilder::new();
    query.add(inner, Occur::SHOULD)?;
    query.add(tq("foo", "baz"), Occur::SHOULD)?;
    let query: Arc<dyn Query> = Arc::new(query.build());

    let e = expect_throws(TOO_MANY_CLAUSES, || searcher.rewrite(Arc::clone(&query)));
    Assert::assert_false_msg(
        "Should have been caught during flattening and not required full nested walk",
        e.is_instance(TOO_MANY_NESTED_CLAUSES),
    );
    Ok(())
}

fn test_large_terms_nested_first() -> Result<()> {
    let searcher = empty_searcher()?;

    let mut nested_builder = BooleanQueryBuilder::new();
    nested_builder.set_minimum_number_should_match(5);
    for i in 0..600 {
        nested_builder.add(tq("foo", &format!("bar-{i}")), Occur::SHOULD)?;
    }
    let inner: Arc<dyn Query> = Arc::new(nested_builder.build());
    let mut builder_mixed = BooleanQueryBuilder::new();
    builder_mixed.add(inner, Occur::SHOULD)?;
    builder_mixed.set_minimum_number_should_match(5);
    for _i in 0..600 {
        builder_mixed.add(tq("foo", "bar"), Occur::SHOULD)?;
    }
    let query: Arc<dyn Query> = Arc::new(builder_mixed.build());

    // Can't be flattened, but high clause count should still be cause during nested walk...
    expect_throws(TOO_MANY_NESTED_CLAUSES, || {
        searcher.rewrite(Arc::clone(&query))
    });
    Ok(())
}

fn test_large_terms_nested_last() -> Result<()> {
    let searcher = empty_searcher()?;

    let mut nested_builder = BooleanQueryBuilder::new();
    nested_builder.set_minimum_number_should_match(5);
    for i in 0..600 {
        nested_builder.add(tq("foo", &format!("bar-{i}")), Occur::SHOULD)?;
    }
    let inner: Arc<dyn Query> = Arc::new(nested_builder.build());
    let mut builder_mixed = BooleanQueryBuilder::new();
    builder_mixed.set_minimum_number_should_match(5);
    for _i in 0..600 {
        builder_mixed.add(tq("foo", "bar"), Occur::SHOULD)?;
    }
    builder_mixed.add(inner, Occur::SHOULD)?;
    let query: Arc<dyn Query> = Arc::new(builder_mixed.build());

    // Can't be flattened, but high clause count should still be cause during nested walk...
    expect_throws(TOO_MANY_NESTED_CLAUSES, || {
        searcher.rewrite(Arc::clone(&query))
    });
    Ok(())
}

fn test_large_disjunction_max_query() -> Result<()> {
    let searcher = empty_searcher()?;
    let mut clauses_query_array: Vec<Arc<dyn Query>> = Vec::with_capacity(1050);
    for _i in 0..1049 {
        clauses_query_array.push(tq("field", "a"));
    }
    let pq = PhraseQuery::with_slop(0, "field", &[])?;
    clauses_query_array.push(Arc::new(pq));
    let dmq: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(clauses_query_array, 0.5)?);

    // Can't be flattened, but high clause count should still be cause during nested walk...
    expect_throws(TOO_MANY_NESTED_CLAUSES, || {
        searcher.rewrite(Arc::clone(&dmq))
    });
    Ok(())
}

fn test_multi_exact_with_repeats() -> Result<()> {
    let searcher = empty_searcher()?;
    let mut qb = MultiPhraseQueryBuilder::new();
    for i in 0..1050 {
        qb.add_at(
            vec![
                term("foo", &format!("bar-{i}")),
                term("foo", &format!("bar+{i}")),
            ],
            0,
        )?;
    }

    // Can't be flattened, but high clause count should still be cause during nested walk...
    expect_throws(TOO_MANY_NESTED_CLAUSES, || {
        searcher.rewrite(Arc::new(qb.build()))
    });
    Ok(())
}
