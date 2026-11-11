//! Port of `lucene/core/src/test/org/apache/lucene/search/TestConstantScoreQuery.java`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::multi_reader::MultiReader;

use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::{
    BooleanQueryBuilder, ConstantScoreQuery, MatchNoDocsQuery, Occur, Query, ScoreMode, Weight,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::tq;
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{assert_equals, assert_not_null, class, random, test};

/// `TestConstantScoreQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestConstantScoreQuery",
        vec![
            test("testCSQ", test_csq),
            test(
                "testConstantScoreQueryAndFilter",
                test_constant_score_query_and_filter,
            ),
            test(
                "testPropagatesApproximations",
                test_propagates_approximations,
            ),
            test(
                "testRewriteBubblesUpMatchNoDocsQuery",
                test_rewrite_bubbles_up_match_no_docs_query,
            ),
        ],
    )
}

fn test_csq() -> Result<()> {
    let q1: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(tq("a", "b")));
    let q2: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(tq("a", "c")));
    let q3: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(Arc::new(
        TermRangeQuery::new_string_range("a", Some("b"), Some("c"), true, true)?,
    )));
    QueryUtils::check(&*q1);
    QueryUtils::check(&*q2);
    QueryUtils::check_equal(&*q1, &*q1);
    QueryUtils::check_equal(&*q2, &*q2);
    QueryUtils::check_equal(&*q3, &*q3);
    QueryUtils::check_unequal(&*q1, &*q2);
    QueryUtils::check_unequal(&*q2, &*q3);
    QueryUtils::check_unequal(&*q1, &*q3);
    QueryUtils::check_unequal(&*q1, &*tq("a", "b"));
    Ok(())
}

/// The test's private `QueryWrapper extends Query`.
#[derive(Debug)]
struct QueryWrapper {
    in_: Arc<dyn Query>,
}

impl Query for QueryWrapper {
    fn to_query_string(&self, _field: &str) -> String {
        "MockQuery".to_string()
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        self.in_.create_weight(searcher, score_mode, boost)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.in_.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<QueryWrapper>()
                .is_some_and(|o| self.in_.query_eq(&*o.in_))
    }

    fn query_hash(&self) -> u64 {
        31u64
            .wrapping_mul(self.class_hash())
            .wrapping_add(self.in_.query_hash())
    }
}

fn test_constant_score_query_and_filter() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "a",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "b",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r = w.get_reader()?;
    w.close()?;

    let filter_b: Arc<dyn Query> = Arc::new(QueryWrapper {
        in_: tq("field", "b"),
    });
    let query: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(Arc::clone(&filter_b)));

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let mut filtered = BooleanQueryBuilder::new();
    filtered.add(query, Occur::MUST)?;
    filtered.add(Arc::clone(&filter_b), Occur::FILTER)?;
    assert_equals(&1, &s.count(Arc::new(filtered.build()))?); // Query for field:b, Filter field:b

    let filter_a: Arc<dyn Query> = Arc::new(QueryWrapper {
        in_: tq("field", "a"),
    });
    let query: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(filter_a));

    let mut filtered = BooleanQueryBuilder::new();
    filtered.add(query, Occur::MUST)?;
    filtered.add(filter_b, Occur::FILTER)?;
    assert_equals(&0, &s.count(Arc::new(filtered.build()))?); // Query field:b, Filter field:a

    drop(s);
    r.close()?;
    d.close()?;
    Ok(())
}

// LUCENE-5307
// don't reuse the scorer of filters since they have been created with bulkScorer=false
fn test_propagates_approximations() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let f = LuceneTestCase::new_text_field("field", "a b", Store::NO);
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    w.commit()?;

    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_cache(None); // to still have approximations

    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(0, "field", &["a", "b"])?);

    let q = searcher.rewrite(Arc::new(ConstantScoreQuery::new(pq)))?;

    let weight = searcher.create_weight(q, ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let mut scorer = weight.scorer(&leaves[0])?.expect("matches");
    assert_not_null(&scorer.two_phase_iterator().map(|_| ()));

    drop((scorer, weight, searcher));
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_rewrite_bubbles_up_match_no_docs_query() -> Result<()> {
    let searcher = LuceneTestCase::new_searcher(Arc::new(MultiReader::new(Vec::new(), true)?));

    let query: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(Arc::new(
        MatchNoDocsQuery::instance(),
    )));
    assert_query_equals(
        &(Arc::new(MatchNoDocsQuery::instance()) as Arc<dyn Query>),
        &searcher.rewrite(query)?,
    );
    Ok(())
}
