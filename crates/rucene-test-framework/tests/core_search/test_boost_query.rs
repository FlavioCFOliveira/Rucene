//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBoostQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::multi_reader::MultiReader;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::query_to_string;
use rucene::search::{
    BooleanQueryBuilder, BoostQuery, ConstantScoreQuery, MatchAllDocsQuery, MatchNoDocsQuery,
    Occur, Query,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::tq;
use crate::core_support::{assert_equals, class, iae_message, random, test};

/// `TestBoostQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBoostQuery",
        vec![
            test("testValidation", test_validation),
            test("testEquals", test_equals),
            test("testToString", test_to_string),
            test("testRewrite", test_rewrite),
            test(
                "testRewriteBubblesUpMatchNoDocsQuery",
                test_rewrite_bubbles_up_match_no_docs_query,
            ),
        ],
    )
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

fn match_none() -> Arc<dyn Query> {
    Arc::new(MatchNoDocsQuery::instance())
}

/// `assertEquals(expected, actual)` over `Query.equals`.
pub fn assert_query_equals(expected: &Arc<dyn Query>, actual: &Arc<dyn Query>) {
    if !expected.query_eq(&**actual) {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            query_to_string(&**expected),
            query_to_string(&**actual)
        ));
    }
}

fn test_validation() -> Result<()> {
    let e = iae_message(|| BoostQuery::new(match_all(), -3.0));
    assert_equals("boost must be a positive float, got -3.0", e.as_str());
    let e = iae_message(|| BoostQuery::new(match_all(), -0.0));
    assert_equals("boost must be a positive float, got -0.0", e.as_str());
    let e = iae_message(|| BoostQuery::new(match_all(), f32::NAN));
    assert_equals("boost must be a positive float, got NaN", e.as_str());
    Ok(())
}

fn test_equals() -> Result<()> {
    let boost = random().next_float() * 3.0;
    let q1 = BoostQuery::new(match_all(), boost)?;
    let q2 = BoostQuery::new(match_all(), boost)?;
    Assert::assert_true(q1.query_eq(&q2));
    Assert::assert_equals_float_msg(None, q1.get_boost(), q2.get_boost(), 0.0);

    let mut boost2 = boost;
    while boost == boost2 {
        boost2 = random().next_float() * 3.0;
    }
    let q3 = BoostQuery::new(match_all(), boost2)?;
    Assert::assert_false(q1.query_eq(&q3));
    Assert::assert_false(q1.query_hash() == q3.query_hash());
    Ok(())
}

fn test_to_string() -> Result<()> {
    assert_equals(
        "(foo:bar)^2.0",
        query_to_string(&BoostQuery::new(tq("foo", "bar"), 2.0)?).as_str(),
    );
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("foo", "bar"), Occur::SHOULD)?;
    bq.add(tq("foo", "baz"), Occur::SHOULD)?;
    let bq: Arc<dyn Query> = Arc::new(bq.build());
    assert_equals(
        "(foo:bar foo:baz)^2.0",
        query_to_string(&BoostQuery::new(bq, 2.0)?).as_str(),
    );
    Ok(())
}

fn empty_reader() -> Result<Arc<dyn IndexReader>> {
    Ok(Arc::new(MultiReader::new(Vec::new(), true)?))
}

fn test_rewrite() -> Result<()> {
    let searcher = IndexSearcher::new(empty_reader()?)?;
    let q: Arc<dyn Query> = Arc::new(BoostQuery::new(
        Arc::new(PhraseQuery::with_slop(0, "foo", &["bar"])?),
        2.0,
    )?);
    assert_query_equals(
        &(Arc::new(BoostQuery::new(tq("foo", "bar"), 2.0)?) as Arc<dyn Query>),
        &searcher.rewrite(q)?,
    );
    let q: Arc<dyn Query> = Arc::new(BoostQuery::new(
        Arc::new(BoostQuery::new(match_all(), 3.0)?),
        2.0,
    )?);
    assert_query_equals(
        &(Arc::new(BoostQuery::new(match_all(), 6.0)?) as Arc<dyn Query>),
        &searcher.rewrite(q)?,
    );
    let q: Arc<dyn Query> = Arc::new(BoostQuery::new(match_all(), 0.0)?);
    assert_query_equals(
        &(Arc::new(BoostQuery::new(
            Arc::new(ConstantScoreQuery::new(match_all())),
            0.0,
        )?) as Arc<dyn Query>),
        &searcher.rewrite(q)?,
    );
    Ok(())
}

fn test_rewrite_bubbles_up_match_no_docs_query() -> Result<()> {
    let searcher = LuceneTestCase::new_searcher(Arc::new(MultiReader::new(Vec::new(), true)?));

    let query: Arc<dyn Query> = Arc::new(BoostQuery::new(match_none(), 2.0)?);
    assert_query_equals(&match_none(), &searcher.rewrite(query)?);

    let query: Arc<dyn Query> = Arc::new(BoostQuery::new(match_none(), 0.0)?);
    assert_query_equals(&match_none(), &searcher.rewrite(query)?);
    Ok(())
}
