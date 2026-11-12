//! Port of `lucene/core/src/test/org/apache/lucene/search/TestNeedsScores.java`.

use std::any::Any;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::weight::{FilterWeight, Weight};
use rucene::search::{
    BooleanQueryBuilder, ConstantScoreQuery, Occur, Query, ScoreMode, Sort, SortField,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, set_up, tear_down, tq,
};
use crate::core_support::{assert_equals, assert_equals_msg, random};

/// The instance fields of `TestNeedsScores`.
#[derive(Default)]
struct Fixture {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
}

impl Fixture {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }
}

/// `TestNeedsScores`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<Fixture>(
        "org.apache.lucene.search.TestNeedsScores",
        vec![
            set_up(fixture_set_up),
            tear_down(|f: &mut Fixture| {
                drop(f.searcher.take());
                close_reader(f.reader.take())?;
                close_dir(f.dir.take())
            }),
            ftest("testProhibitedClause", test_prohibited_clause),
            ftest("testConstantScoreQuery", test_constant_score_query),
            ftest("testSortByField", test_sort_by_field),
            ftest("testSortByScore", test_sort_by_score),
        ],
    )
}

fn fixture_set_up(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for i in 0..5 {
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new(
            "field",
            format!("this is document {i}"),
            Store::NO,
        )?));
        iw.add_document(&doc)?;
    }
    let reader = iw.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    // Needed so that the cache doesn't consume weights with
    // ScoreMode.COMPLETE_NO_SCORES for the purpose of populating the cache.
    searcher.set_query_cache(None);
    iw.close()?;
    f.dir = Some(dir);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    Ok(())
}

/// prohibited clauses in booleanquery don't need scoring
fn test_prohibited_clause(f: &mut Fixture) -> Result<()> {
    let required = tq("field", "this");
    let prohibited = tq("field", "3");
    let mut bq = BooleanQueryBuilder::new();
    bq.add(
        AssertNeedsScores::new(required, ScoreMode::TOP_SCORES),
        Occur::MUST,
    )?;
    bq.add(
        AssertNeedsScores::new(prohibited, ScoreMode::COMPLETE_NO_SCORES),
        Occur::MUST_NOT,
    )?;
    assert_equals(
        &4,
        &f.searcher()
            .search(Arc::new(bq.build()), 5)?
            .total_hits
            .value(),
    ); // we exclude 3
    Ok(())
}

/// nested inside constant score query
fn test_constant_score_query(f: &mut Fixture) -> Result<()> {
    let term = tq("field", "this");

    // Counting queries and top-score queries that compute the hit count should
    // use COMPLETE_NO_SCORES
    let constant_score: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(AssertNeedsScores::new(
        Arc::clone(&term),
        ScoreMode::COMPLETE_NO_SCORES,
    )));
    assert_equals(&5, &f.searcher().count(Arc::clone(&constant_score))?);
    let hits = f.searcher().search_with_manager(
        Arc::clone(&constant_score),
        &TopScoreDocCollectorManager::new(5, None, i32::MAX)?,
    )?;
    assert_equals(&5, &hits.total_hits.value());

    // Queries that support dynamic pruning like top-score or top-doc queries
    // that do not compute the hit count should use TOP_DOCS
    let constant_score: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(AssertNeedsScores::new(
        term,
        ScoreMode::TOP_DOCS,
    )));
    assert_equals(
        &5,
        &f.searcher()
            .search(Arc::clone(&constant_score), 5)?
            .total_hits
            .value(),
    );

    assert_equals(
        &5,
        &f.searcher()
            .search_with_sort(
                Arc::clone(&constant_score),
                5,
                &Sort::new_fields(vec![SortField::FIELD_DOC.clone()])?,
            )?
            .total_hits
            .value(),
    );

    assert_equals(
        &5,
        &f.searcher()
            .search_with_sort(
                constant_score,
                5,
                &Sort::new_fields(vec![
                    SortField::FIELD_DOC.clone(),
                    SortField::FIELD_SCORE.clone(),
                ])?,
            )?
            .total_hits
            .value(),
    );
    Ok(())
}

/// when not sorting by score
fn test_sort_by_field(f: &mut Fixture) -> Result<()> {
    let query =
        AssertNeedsScores::new(Arc::new(MatchAllDocsQuery::instance()), ScoreMode::TOP_DOCS);
    // `Sort.INDEXORDER`.
    let index_order = Sort::new_fields(vec![SortField::FIELD_DOC.clone()])?;
    assert_equals(
        &5,
        &f.searcher()
            .search_with_sort(query, 5, &index_order)?
            .total_hits
            .value(),
    );
    Ok(())
}

/// when sorting by score
fn test_sort_by_score(f: &mut Fixture) -> Result<()> {
    let query = AssertNeedsScores::new(
        Arc::new(MatchAllDocsQuery::instance()),
        ScoreMode::TOP_SCORES,
    );
    // `Sort.RELEVANCE`.
    assert_equals(
        &5,
        &f.searcher()
            .search_with_sort(query, 5, &Sort::new())?
            .total_hits
            .value(),
    );
    Ok(())
}

/// Wraps a query, checking that the needsScores param passed to Weight.scorer
/// is the expected value.
///
/// `static class AssertNeedsScores extends Query`.
#[derive(Debug, Clone)]
struct AssertNeedsScores {
    r#in: Arc<dyn Query>,
    value: ScoreMode,
}

impl AssertNeedsScores {
    #[allow(clippy::new_ret_no_self)] // Java's constructor, handing the object back as the type its callers use
    fn new(r#in: Arc<dyn Query>, value: ScoreMode) -> Arc<dyn Query> {
        Arc::new(AssertNeedsScores { r#in, value })
    }
}

/// The anonymous `FilterWeight` of `AssertNeedsScores.createWeight`.
#[derive(Debug)]
struct AssertNeedsScoresWeight {
    base: FilterWeight,
    r#in: Arc<dyn Query>,
    value: ScoreMode,
    score_mode: ScoreMode,
}

impl SegmentCacheable for AssertNeedsScoresWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.base.is_cacheable(ctx)
    }
}

impl Weight for AssertNeedsScoresWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.base.explain(context, doc)
    }

    fn matches(
        &self,
        context: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Arc<dyn rucene::search::matches::Matches>>> {
        self.base.matches(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.base.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(mut scorer_supplier) = self.base.inner.scorer_supplier(context)? else {
            return Ok(None);
        };
        let mut scorer = scorer_supplier.get(i64::MAX)?;
        let cost = scorer.iterator().cost();
        Ok(Some(Box::new(AssertingSupplier {
            scorer: Some(scorer),
            cost,
            r#in: Arc::clone(&self.r#in),
            value: self.value,
            score_mode: self.score_mode,
        })))
    }
}

/// The anonymous `ScorerSupplier` of `AssertNeedsScores`' weight.
struct AssertingSupplier {
    scorer: Option<Box<dyn Scorer>>,
    /// `scorer.iterator().cost()`, read when the scorer is built: it is
    /// stable.
    cost: i64,
    r#in: Arc<dyn Query>,
    value: ScoreMode,
    score_mode: ScoreMode,
}

impl ScorerSupplier for AssertingSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        assert_equals_msg(
            Some(&format!(
                "query={}",
                rucene::search::query_to_string(&*self.r#in)
            )),
            &self.value,
            &self.score_mode,
        );
        Ok(self.scorer.take().expect("get is called once"))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl Query for AssertNeedsScores {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let w = self.r#in.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(AssertNeedsScoresWeight {
            base: FilterWeight::with_query(Arc::new(self.clone()), w),
            r#in: Arc::clone(&self.r#in),
            value: self.value,
            score_mode,
        }))
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.r#in.rewrite(index_searcher)? {
            // `in2 == in`: `super.rewrite(indexSearcher)`, which is `this`.
            None => Ok(None),
            Some(in2) => Ok(Some(AssertNeedsScores::new(in2, self.value))),
        }
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.r#in.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `value.hashCode()` is the enum constant's identity hash in Java; the
    /// port hashes the constant.
    fn query_hash(&self) -> i32 {
        let prime = 31i32;
        let mut result = self.class_hash();
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.r#in.query_hash());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.value.hash(&mut hasher);
        prime
            .wrapping_mul(result)
            .wrapping_add(hasher.finish() as i32)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<AssertNeedsScores>()
                .is_some_and(|o| self.r#in.query_eq(&*o.r#in) && self.value == o.value)
    }

    fn to_query_string(&self, field: &str) -> String {
        format!("asserting({})", self.r#in.to_query_string(field))
    }
}
