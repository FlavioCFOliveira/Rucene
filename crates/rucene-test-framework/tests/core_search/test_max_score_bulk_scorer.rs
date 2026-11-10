//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMaxScoreBulkScorer.java`.
//!
//! Java's anonymous `LeafCollector`s keep the `Scorable` `setScorer` hands
//! them and switch on a running counter in `collect`; the port's
//! [`LeafCollector::collect`] receives that scorable, so each collector is an
//! [`ExpectedHits`] list: the `case`s of the `switch`, in order, with the
//! `default: fail()` past its end.

use std::any::Any;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::bulk_scorer::BulkScorer;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::dense_conjunction_bulk_scorer::DENSITY_THRESHOLD_INVERSE;
use rucene::search::doc_id_set_iterator::{
    self, AllDocIdSetIterator, DocIdSetIterator, NO_MORE_DOCS,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::max_score_bulk_scorer::{MaxScoreBulkScorer, INNER_WINDOW_SIZE};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::{FilterWeight, Weight};
use rucene::search::{
    BooleanQueryBuilder, BoostQuery, CollectionResult, LeafCollector, Occur, Query, ScoreMode,
};
use rucene::util::{Bits, FixedBitSet};
use rucene_test_framework::search::random_approximation_query::RandomApproximationQuery;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::tq;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, class, random, shuffle, test,
};

/// `TestMaxScoreBulkScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMaxScoreBulkScorer",
        vec![
            test(
                "testBasicsWithTwoDisjunctionClauses",
                test_basics_with_two_disjunction_clauses,
            ),
            test("testFilteredDisjunction", test_filtered_disjunction),
            test(
                "testFilteredDisjunctionWithSkipping",
                test_filtered_disjunction_with_skipping,
            ),
            test(
                "testBasicsWithTwoDisjunctionClausesAndSkipping",
                test_basics_with_two_disjunction_clauses_and_skipping,
            ),
            test(
                "testBasicsWithThreeDisjunctionClauses",
                test_basics_with_three_disjunction_clauses,
            ),
            test(
                "testBasicsWithThreeDisjunctionClausesAndSkipping",
                test_basics_with_three_disjunction_clauses_and_skipping,
            ),
            test("testDeletes", test_deletes),
            test(
                "testFilteredDisjunctionWithDenseFilter",
                test_filtered_disjunction_with_dense_filter,
            ),
            test(
                "testFilteredDisjunctionWithDenseFilterAndAcceptDocs",
                test_filtered_disjunction_with_dense_filter_and_accept_docs,
            ),
            test("testPartition", test_partition),
            test(
                "testFilteredDisjunctionWithFilterGapCausingNegativeWindowSize",
                test_filtered_disjunction_with_filter_gap_causing_negative_window_size,
            ),
            test(
                "testSparseFilterUsesLeapFrog",
                test_sparse_filter_uses_leap_frog,
            ),
            test("testDenseScorersUseBitSet", test_dense_scorers_use_bit_set),
        ],
    )
}

// -----------------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------------

/// `new StringField(name, value, Field.Store.NO)`.
fn string_field(name: &str, value: &str) -> Result<Box<StringField>> {
    Ok(Box::new(StringField::new(
        name,
        value.to_string(),
        Store::NO,
    )?))
}

/// `new IndexWriter(dir, newIndexWriterConfig().setMergePolicy(newLogMergePolicy()))`.
fn log_merge_writer(dir: &TestDirectory) -> Result<Arc<IndexWriter>> {
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    IndexWriter::new(dir.as_directory(), iwc)
}

fn write_documents(dir: &TestDirectory) -> Result<()> {
    let w = log_merge_writer(dir)?;
    let all: [&[&str]; 6] = [
        &["A", "B"],      // 0
        &["A"],           // 1
        &[],              // 2
        &["A", "B", "C"], // 3
        &["B"],           // 4
        &["B", "C"],      // 5
    ];
    for values in all {
        let mut doc = Document::new();
        for value in values {
            doc.add(string_field("foo", value)?);
        }
        w.add_document(&doc)?;
        for _ in 1..INNER_WINDOW_SIZE {
            w.add_document(&Document::new())?;
        }
    }
    w.force_merge(1)?;
    w.close()
}

/// `new BoostQuery(new ConstantScoreQuery(new TermQuery(new Term("foo", text))), boost)`.
fn boosted_csq(text: &str, boost: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(csq(text), boost)?))
}

/// `new ConstantScoreQuery(new TermQuery(new Term("foo", text)))`.
fn csq(text: &str) -> Arc<dyn Query> {
    Arc::new(ConstantScoreQuery::new(tq("foo", text)))
}

/// `searcher.createWeight(searcher.rewrite(q), ScoreMode.TOP_SCORES, 1f).scorer(context)`.
fn top_scorer(
    searcher: &IndexSearcher,
    q: &Arc<dyn Query>,
    context: &LeafReaderContext,
) -> Result<Box<dyn Scorer>> {
    Ok(searcher
        .create_weight(searcher.rewrite(Arc::clone(q))?, ScoreMode::TOP_SCORES, 1.0)?
        .scorer(context)?
        .expect("the clause matches on this leaf"))
}

/// `searcher.getIndexReader().leaves().get(0)`.
fn first_leaf(searcher: &IndexSearcher) -> Result<Arc<LeafReaderContext>> {
    Ok(Arc::clone(
        &Arc::clone(searcher.get_index_reader()).leaves()?[0],
    ))
}

/// One `case` of a collector's `switch (i++)`: the expected doc, its
/// expected score, and the minimum competitive score the case then sets.
#[derive(Clone, Copy)]
struct Hit {
    doc: i32,
    score: f32,
    set_min_competitive_score: Option<f32>,
}

/// `case n: assertEquals(doc, doc); assertEquals(score, scorer.score(), 0);`.
fn hit(doc: i32, score: f32) -> Hit {
    Hit {
        doc,
        score,
        set_min_competitive_score: None,
    }
}

/// [`hit`] followed by `scorer.setMinCompetitiveScore(min)`.
fn hit_then_min(doc: i32, score: f32, min: f32) -> Hit {
    Hit {
        doc,
        score,
        set_min_competitive_score: Some(min),
    }
}

/// `Math.nextUp(float)` of a positive value.
fn next_up(v: f32) -> f32 {
    v.next_up()
}

/// The anonymous `LeafCollector`s that switch on the hit counter.
struct ExpectedHits {
    i: usize,
    hits: Vec<Hit>,
    /// `default: System.out.println(i); fail();`.
    print_on_default: bool,
}

impl ExpectedHits {
    fn new(hits: Vec<Hit>) -> Self {
        ExpectedHits {
            i: 0,
            hits,
            print_on_default: false,
        }
    }
}

impl LeafCollector for ExpectedHits {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let i = self.i;
        self.i += 1;
        match self.hits.get(i) {
            Some(expected) => {
                assert_equals(&expected.doc, &doc);
                assert_equals_delta(expected.score as f64, scorer.score()? as f64, 0.0);
                if let Some(min) = expected.set_min_competitive_score {
                    scorer.set_min_competitive_score(min)?;
                }
            }
            None => {
                if self.print_on_default {
                    println!("{}", self.i);
                }
                Assert::fail_without_message();
            }
        }
        Ok(())
    }
}

/// `new MaxScoreBulkScorer(maxDoc, scorers, filter).score(collector, acceptDocs, 0, max)`.
fn score_all(
    max_doc: i32,
    scorers: Vec<Box<dyn Scorer>>,
    filter: Option<Box<dyn Scorer>>,
    collector: &mut dyn LeafCollector,
    accept_docs: Option<&dyn Bits>,
    max: i32,
) -> Result<()> {
    let mut scorer: Box<dyn BulkScorer> =
        Box::new(MaxScoreBulkScorer::new(max_doc, scorers, filter)?);
    scorer
        .score(collector, accept_docs, 0, max)
        .map_err(|e| e.into_lucene_error())?;
    Ok(())
}

/// The body of the tests over [`write_documents`]: `clauses` scored with an
/// optional `filter`, each clause wrapped in a `RandomApproximationQuery`
/// when `maybe_approximate` and `random().nextBoolean()`.
fn run_over_documents(
    clauses: Vec<Arc<dyn Query>>,
    filter: Option<Arc<dyn Query>>,
    maybe_approximate: bool,
    mut collector: ExpectedHits,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    write_documents(&dir)?;
    {
        let reader = directory_reader::open(dir.as_directory())?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let mut clauses = clauses;
        if maybe_approximate && random().next_boolean() {
            clauses = clauses
                .into_iter()
                .map(|c| Arc::new(RandomApproximationQuery::new(c, random())) as Arc<dyn Query>)
                .collect();
        }
        let context = first_leaf(&searcher)?;
        let mut scorers = Vec::with_capacity(clauses.len());
        for clause in &clauses {
            scorers.push(top_scorer(&searcher, clause, &context)?);
        }
        let filter_scorer = match &filter {
            Some(f) => Some(top_scorer(&searcher, f, &context)?),
            None => None,
        };

        score_all(
            context.leaf_reader().max_doc(),
            scorers,
            filter_scorer,
            &mut collector,
            None,
            NO_MORE_DOCS,
        )?;
        drop(searcher);
        reader.close()?;
    }
    dir.close()
}

// -----------------------------------------------------------------------------
// Tests over writeDocuments
// -----------------------------------------------------------------------------

/// Port of `TestMaxScoreBulkScorer.testBasicsWithTwoDisjunctionClauses`.
fn test_basics_with_two_disjunction_clauses() -> Result<()> {
    run_over_documents(
        vec![boosted_csq("A", 2.0)?, csq("B")],
        None,
        false,
        ExpectedHits::new(vec![
            hit(0, 2.0 + 1.0),
            hit(4096, 2.0),
            hit(12288, 2.0 + 1.0),
            hit(16384, 1.0),
            hit(20480, 1.0),
        ]),
    )
}

/// Port of `TestMaxScoreBulkScorer.testFilteredDisjunction`.
fn test_filtered_disjunction() -> Result<()> {
    run_over_documents(
        vec![boosted_csq("A", 2.0)?, csq("C")],
        Some(tq("foo", "B")),
        true,
        ExpectedHits::new(vec![hit(0, 2.0), hit(12288, 2.0 + 1.0), hit(20480, 1.0)]),
    )
}

/// Port of `TestMaxScoreBulkScorer.testFilteredDisjunctionWithSkipping`.
fn test_filtered_disjunction_with_skipping() -> Result<()> {
    let mut collector = ExpectedHits::new(vec![
        hit_then_min(0, 2.0, next_up(2.0)),
        hit_then_min(12288, 2.0 + 1.0, next_up(2.0 + 1.0)),
    ]);
    collector.print_on_default = true;
    run_over_documents(
        vec![boosted_csq("A", 2.0)?, csq("C")],
        Some(tq("foo", "B")),
        true,
        collector,
    )
}

/// Port of `TestMaxScoreBulkScorer.testBasicsWithTwoDisjunctionClausesAndSkipping`.
fn test_basics_with_two_disjunction_clauses_and_skipping() -> Result<()> {
    run_over_documents(
        vec![boosted_csq("A", 2.0)?, csq("B")],
        None,
        false,
        ExpectedHits::new(vec![
            hit(0, 2.0 + 1.0),
            // simulate top-2 retrieval
            hit_then_min(4096, 2.0, next_up(2.0)),
            hit_then_min(12288, 2.0 + 1.0, next_up(2.0 + 1.0)),
        ]),
    )
}

/// Port of `TestMaxScoreBulkScorer.testBasicsWithThreeDisjunctionClauses`.
fn test_basics_with_three_disjunction_clauses() -> Result<()> {
    run_over_documents(
        vec![boosted_csq("A", 2.0)?, csq("B"), boosted_csq("C", 3.0)?],
        None,
        false,
        ExpectedHits::new(vec![
            hit(0, 2.0 + 1.0),
            hit(4096, 2.0),
            hit(12288, 2.0 + 1.0 + 3.0),
            hit(16384, 1.0),
            hit(20480, 1.0 + 3.0),
        ]),
    )
}

/// Port of `TestMaxScoreBulkScorer.testBasicsWithThreeDisjunctionClausesAndSkipping`.
fn test_basics_with_three_disjunction_clauses_and_skipping() -> Result<()> {
    run_over_documents(
        vec![boosted_csq("A", 2.0)?, csq("B"), boosted_csq("C", 3.0)?],
        None,
        false,
        ExpectedHits::new(vec![
            hit(0, 2.0 + 1.0),
            // simulate top-2 retrieval
            hit_then_min(4096, 2.0, next_up(2.0)),
            hit_then_min(12288, 2.0 + 1.0 + 3.0, next_up(2.0 + 1.0)),
            hit_then_min(20480, 1.0 + 3.0, next_up(1.0 + 3.0)),
        ]),
    )
}

// -----------------------------------------------------------------------------
// FakeScorer and testPartition
// -----------------------------------------------------------------------------

/// The mutable fields of a `FakeScorer`, which the test sets after the bulk
/// scorer holds the scorer.
#[derive(Debug)]
struct FakeScorerState {
    to_string: &'static str,
    doc_id: AtomicI32,
    max_score_up_to: AtomicI32,
    /// `maxScore`, as `f32` bits.
    max_score: AtomicU32,
    cost: AtomicI32,
}

impl FakeScorerState {
    fn new(to_string: &'static str) -> Arc<Self> {
        Arc::new(FakeScorerState {
            to_string,
            doc_id: AtomicI32::new(-1),
            max_score_up_to: AtomicI32::new(NO_MORE_DOCS),
            max_score: AtomicU32::new(1.0f32.to_bits()),
            cost: AtomicI32::new(10),
        })
    }
}

/// `private static class FakeScorer extends Scorer`.
///
/// Java's test sets `docID`, `maxScoreUpTo`, `maxScore` and `cost` on the
/// scorer after handing it to the bulk scorer; the fields live in a shared
/// [`FakeScorerState`] the test keeps, and `assertSame(the,
/// scorer.allScorers[0].scorer)` compares that state's identity.
struct FakeScorer {
    state: Arc<FakeScorerState>,
    /// `DocIdSetIterator.all(cost)`, just so that it exposes the right cost.
    disi: AllDocIdSetIterator,
}

impl FakeScorer {
    fn new(state: &Arc<FakeScorerState>) -> Result<Self> {
        Ok(FakeScorer {
            state: Arc::clone(state),
            disi: doc_id_set_iterator::all(state.cost.load(Ordering::SeqCst))?,
        })
    }
}

impl Scorable for FakeScorer {
    fn score(&mut self) -> Result<f32> {
        Err(rucene::error::LuceneError::UnsupportedOperation(
            String::new(),
        ))
    }
}

impl Scorer for FakeScorer {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn doc_id(&self) -> i32 {
        self.state.doc_id.load(Ordering::SeqCst)
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        // just so that it exposes the right cost
        self.disi = doc_id_set_iterator::all(self.state.cost.load(Ordering::SeqCst))
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e));
        &mut self.disi
    }

    fn advance_shallow(&mut self, _target: i32) -> Result<i32> {
        Ok(self.state.max_score_up_to.load(Ordering::SeqCst))
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::from_bits(self.state.max_score.load(Ordering::SeqCst)))
    }
}

/// `assertSame(expected, scorer.allScorers[i].scorer)`.
fn assert_same_scorer(expected: &Arc<FakeScorerState>, scorer: &mut MaxScoreBulkScorer, i: usize) {
    let actual = scorer
        .all_scorer(i)
        .as_any()
        .and_then(|a| a.downcast_ref::<FakeScorer>())
        .map(|f| Arc::clone(&f.state));
    if !actual.is_some_and(|a| Arc::ptr_eq(&a, expected)) {
        Assert::fail(&format!("expected same:<{}>", expected.to_string));
    }
}

/// `scorer.scorable.minCompetitiveScore = v;
/// Collections.shuffle(Arrays.asList(scorer.allScorers), random());
/// scorer.updateMaxWindowScores(4, 100);`.
fn repartition(scorer: &mut MaxScoreBulkScorer, min_competitive_score: Option<f32>) -> Result<()> {
    if let Some(v) = min_competitive_score {
        scorer.scorable_mut().set_min_competitive_score(v)?;
    }
    shuffle(scorer.all_scorers_mut(), &*random());
    scorer.update_max_window_scores(4, 100)
}

/// This test simulates what happens over time for the query `the quick fox`
/// as collection progresses and the minimum competitive score increases.
///
/// Port of `TestMaxScoreBulkScorer.testPartition`.
fn test_partition() -> Result<()> {
    let the = FakeScorerState::new("the");
    the.cost.store(9_000, Ordering::SeqCst);
    the.max_score.store(0.1f32.to_bits(), Ordering::SeqCst);
    let quick = FakeScorerState::new("quick");
    quick.cost.store(1_000, Ordering::SeqCst);
    quick.max_score.store(1.0f32.to_bits(), Ordering::SeqCst);
    let fox = FakeScorerState::new("fox");
    fox.cost.store(900, Ordering::SeqCst);
    fox.max_score.store(1.1f32.to_bits(), Ordering::SeqCst);
    let mut scorer = MaxScoreBulkScorer::new(
        10_000,
        vec![
            Box::new(FakeScorer::new(&the)?),
            Box::new(FakeScorer::new(&quick)?),
            Box::new(FakeScorer::new(&fox)?),
        ],
        None,
    )?;

    the.doc_id.store(4, Ordering::SeqCst);
    the.max_score_up_to.store(130, Ordering::SeqCst);

    quick.doc_id.store(4, Ordering::SeqCst);
    quick.max_score_up_to.store(999, Ordering::SeqCst);

    fox.doc_id.store(10, Ordering::SeqCst);
    fox.max_score_up_to.store(1_200, Ordering::SeqCst);

    let s = &mut scorer;
    repartition(s, None)?;
    Assert::assert_true(s.partition_scorers());
    assert_equals(&0, &s.first_essential_scorer()); // all clauses are essential
    assert_equals(&3, &s.first_required_scorer()); // no required clauses

    // less than the minimum score of every clause
    repartition(s, Some(0.09))?;
    Assert::assert_true(s.partition_scorers());
    assert_equals(&0, &s.first_essential_scorer()); // all clauses are still essential
    assert_equals(&3, &s.first_required_scorer()); // no required clauses

    // equal to the maximum score of `the`
    repartition(s, Some(0.1))?;
    Assert::assert_true(s.partition_scorers());
    assert_equals(&0, &s.first_essential_scorer()); // all clauses are still essential
    assert_equals(&3, &s.first_required_scorer()); // no required clauses

    // gt than the minimum score of `the`
    repartition(s, Some(0.11))?;
    Assert::assert_true(s.partition_scorers());
    assert_equals(&1, &s.first_essential_scorer()); // the is non essential
    assert_equals(&3, &s.first_required_scorer()); // no required clauses
    assert_same_scorer(&the, s, 0);

    // equal to the sum of the max scores of the and quick
    repartition(s, Some(1.1))?;
    Assert::assert_true(s.partition_scorers());
    assert_equals(&1, &s.first_essential_scorer()); // the is non essential
    assert_equals(&3, &s.first_required_scorer()); // no required clauses
    assert_same_scorer(&the, s, 0);

    // greater than the sum of the max scores of the and quick, then equal to
    // the sum of the max scores of the and fox
    for min in [1.11f32, 1.2] {
        repartition(s, Some(min))?;
        Assert::assert_true(s.partition_scorers());
        assert_equals(&2, &s.first_essential_scorer()); // the and quick are non essential
        assert_equals(&2, &s.first_required_scorer()); // fox is required
        assert_same_scorer(&the, s, 0);
        assert_same_scorer(&quick, s, 1);
        assert_same_scorer(&fox, s, 2);
    }

    // greater than the sum of the max scores of the and fox, then equal to the
    // sum of the max scores of quick and fox
    for min in [1.21f32, 2.1] {
        repartition(s, Some(min))?;
        Assert::assert_true(s.partition_scorers());
        assert_equals(&2, &s.first_essential_scorer()); // the and quick are non essential
        assert_equals(&1, &s.first_required_scorer()); // quick and fox are required
        assert_same_scorer(&the, s, 0);
        assert_same_scorer(&quick, s, 1);
        assert_same_scorer(&fox, s, 2);
    }

    // greater than the sum of the max scores of quick and fox (twice), then
    // equal to the sum of the max scores of all terms
    for min in [2.11f32, 2.11, 2.2] {
        repartition(s, Some(min))?;
        Assert::assert_true(s.partition_scorers());
        assert_equals(&2, &s.first_essential_scorer()); // the and quick are non essential
        assert_equals(&0, &s.first_required_scorer()); // all terms are required
        assert_same_scorer(&the, s, 0);
        assert_same_scorer(&quick, s, 1);
        assert_same_scorer(&fox, s, 2);
    }

    // greater than the sum of the max scores of all terms
    repartition(s, Some(2.21))?;
    Assert::assert_false(s.partition_scorers()); // no possible match in this window
    Ok(())
}

// -----------------------------------------------------------------------------
// testDeletes
// -----------------------------------------------------------------------------

/// The anonymous `Bits` of `testDeletes`: only doc 1 is live.
#[derive(Debug)]
struct OnlyDoc1;

impl Bits for OnlyDoc1 {
    fn get(&self, index: usize) -> bool {
        index == 1
    }

    fn length(&self) -> usize {
        3
    }
}

/// The anonymous `LeafCollector` of `testDeletes`.
struct DeletesCollector {
    i: i32,
    min_competitive_score: f32,
}

impl LeafCollector for DeletesCollector {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        scorer.set_min_competitive_score(self.min_competitive_score)
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        assert_equals(&1, &doc);
        assert_equals(&0, &self.i);
        self.i += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        assert_equals(&1, &self.i);
        Ok(())
    }
}

/// Port of `TestMaxScoreBulkScorer.testDeletes`.
fn test_deletes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = log_merge_writer(&dir)?;
    // doc2 and doc3 hold the same field instances as doc1, then their own id
    let shared = |id: &str| -> Result<Document> {
        let mut doc = Document::new();
        doc.add(string_field("field", "foo")?);
        doc.add(string_field("field", "bar")?);
        doc.add(string_field("field", "quux")?);
        doc.add(string_field("id", id)?);
        Ok(doc)
    };
    let doc1 = shared("1")?;
    let doc2 = shared("2")?;
    let doc3 = shared("3")?;
    w.add_document(&doc1)?;
    w.add_document(&doc2)?;
    w.add_document(&doc3)?;

    w.force_merge(1)?;

    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;

    let field_csq = |text: &str, boost: f32| -> Result<Arc<dyn Query>> {
        Ok(Arc::new(BoostQuery::new(
            Arc::new(ConstantScoreQuery::new(tq("field", text))),
            boost,
        )?))
    };
    let query: Arc<dyn Query> = Arc::new(
        BooleanQueryBuilder::new()
            .add(field_csq("foo", 1.0)?, Occur::SHOULD)?
            .add(field_csq("bar", 1.5)?, Occur::SHOULD)?
            .add(field_csq("quux", 0.1)?, Occur::SHOULD)?
            .build(),
    );

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let weight = searcher.create_weight(searcher.rewrite(query)?, ScoreMode::TOP_SCORES, 1.0)?;

    let live_docs = OnlyDoc1;

    // Test min competitive scores that exercise different execution modes
    for min_competitive_score in [
        0.0f32, // 3 essential clauses
        1.0,    // 2 essential clauses
        1.2,    // 1 essential clause
        2.0,    // two required clauses
    ] {
        let mut scorer = weight
            .bulk_scorer(&*first_leaf(&searcher)?)?
            .expect("the query matches on this leaf");
        let mut collector = DeletesCollector {
            i: 0,
            min_competitive_score,
        };
        scorer
            .score(&mut collector, Some(&live_docs), 0, NO_MORE_DOCS)
            .map_err(|e| e.into_lucene_error())?;
        collector.finish()?;
    }

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Dense filters
// -----------------------------------------------------------------------------

/// `assertTrue("filter should be dense enough for bitset path", ...)`.
fn assert_dense_filter(
    searcher: &IndexSearcher,
    filter: &Arc<dyn Query>,
    context: &LeafReaderContext,
) -> Result<()> {
    // Verify the filter is dense enough to trigger the bitset path
    let mut filter_check = top_scorer(searcher, filter, context)?;
    Assert::assert_true_msg(
        "filter should be dense enough for bitset path",
        filter_check.iterator().cost()
            >= (context.leaf_reader().max_doc() / DENSITY_THRESHOLD_INVERSE) as i64,
    );
    Ok(())
}

/// Test that the bitset-based filter path in MaxScoreBulkScorer is exercised
/// when the filter is dense enough. This creates a filter whose cost exceeds
/// maxDoc/32 (the density threshold) so that filterMatches is non-null and
/// fillScoreBufferViaBitSet is used.
///
/// Port of `TestMaxScoreBulkScorer.testFilteredDisjunctionWithDenseFilter`.
fn test_filtered_disjunction_with_dense_filter() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    // We need maxDoc >= INNER_WINDOW_SIZE (4096) and filter.cost >= maxDoc/32.
    // Create INNER_WINDOW_SIZE + 1 docs. Add the filter term "F" to all of
    // them, and scoring terms "A" and "C" to specific docs.
    let num_docs = INNER_WINDOW_SIZE + 1;
    {
        let w = log_merge_writer(&dir)?;
        for i in 0..num_docs {
            let mut doc = Document::new();
            // Dense filter: every doc matches
            doc.add(string_field("foo", "F")?);
            if i == 0 || i == 1 {
                doc.add(string_field("foo", "A")?);
            }
            if i == 1 || i == 2 {
                doc.add(string_field("foo", "C")?);
            }
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;
        w.close()?;
    }

    {
        let reader = directory_reader::open(dir.as_directory())?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let clause1 = boosted_csq("A", 2.0)?;
        let clause2 = csq("C");
        let filter = tq("foo", "F");
        let context = first_leaf(&searcher)?;

        assert_dense_filter(&searcher, &filter, &context)?;

        let scorer1 = top_scorer(&searcher, &clause1, &context)?;
        let scorer2 = top_scorer(&searcher, &clause2, &context)?;
        let filter_scorer = top_scorer(&searcher, &filter, &context)?;

        // Doc 0: has A (score=2), filter matches
        // Doc 1: has A+C (score=2+1=3), filter matches
        // Doc 2: has C (score=1), filter matches
        score_all(
            context.leaf_reader().max_doc(),
            vec![scorer1, scorer2],
            Some(filter_scorer),
            &mut ExpectedHits::new(vec![hit(0, 2.0), hit(1, 2.0 + 1.0), hit(2, 1.0)]),
            None,
            NO_MORE_DOCS,
        )?;
        drop(searcher);
        reader.close()?;
    }
    dir.close()
}

/// The anonymous `Bits` of `testFilteredDisjunctionWithDenseFilterAndAcceptDocs`:
/// every doc but doc 1.
#[derive(Debug)]
struct AllButDoc1 {
    num_docs: i32,
}

impl Bits for AllButDoc1 {
    fn get(&self, index: usize) -> bool {
        index != 1
    }

    fn length(&self) -> usize {
        self.num_docs as usize
    }
}

/// Test the bitset-based filter path with acceptDocs (live docs) to verify
/// that deleted docs are correctly excluded when the filter is loaded into a
/// bitset.
///
/// Port of `TestMaxScoreBulkScorer.testFilteredDisjunctionWithDenseFilterAndAcceptDocs`.
fn test_filtered_disjunction_with_dense_filter_and_accept_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let num_docs = INNER_WINDOW_SIZE + 1;
    {
        let w = log_merge_writer(&dir)?;
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(string_field("foo", "F")?);
            if i == 0 || i == 1 || i == 2 {
                doc.add(string_field("foo", "A")?);
            }
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;
        w.close()?;
    }

    {
        let reader = directory_reader::open(dir.as_directory())?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let clause1 = boosted_csq("A", 2.0)?;
        let filter = tq("foo", "F");
        let context = first_leaf(&searcher)?;
        let scorer1 = top_scorer(&searcher, &clause1, &context)?;
        // Need at least 2 scoring clauses for MaxScoreBulkScorer
        let clause2 = csq("A");
        let scorer2 = top_scorer(&searcher, &clause2, &context)?;
        let filter_scorer = top_scorer(&searcher, &filter, &context)?;

        // acceptDocs that excludes doc 1
        let accept_docs = AllButDoc1 { num_docs };

        score_all(
            context.leaf_reader().max_doc(),
            vec![scorer1, scorer2],
            Some(filter_scorer),
            &mut ExpectedHits::new(vec![
                hit(0, 2.0 + 1.0),
                // doc 1 skipped by acceptDocs
                hit(2, 2.0 + 1.0),
            ]),
            Some(&accept_docs),
            NO_MORE_DOCS,
        )?;
        drop(searcher);
        reader.close()?;
    }
    dir.close()
}

/// The anonymous `LeafCollector` of
/// `testFilteredDisjunctionWithFilterGapCausingNegativeWindowSize`.
struct OnlyDoc0Collector;

impl LeafCollector for OnlyDoc0Collector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        // Doc 0 is the only doc matching both filter and scoring clauses
        assert_equals(&0, &doc);
        Ok(())
    }
}

/// Test that fillScoreBufferViaBitSet doesn't produce a negative
/// innerWindowSize when the filter iterator gets advanced past the outer
/// window max, causing essentials to advance past max on the next inner
/// window iteration.
///
/// Port of
/// `TestMaxScoreBulkScorer.testFilteredDisjunctionWithFilterGapCausingNegativeWindowSize`.
fn test_filtered_disjunction_with_filter_gap_causing_negative_window_size() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    // Need 2 * INNER_WINDOW_SIZE docs so there's room for a second inner
    // window. Filter only matches first INNER_WINDOW_SIZE docs, creating a gap.
    let num_docs = 2 * INNER_WINDOW_SIZE;
    {
        let w = log_merge_writer(&dir)?;
        for i in 0..num_docs {
            let mut doc = Document::new();
            if i < INNER_WINDOW_SIZE {
                // Dense filter: matches only in the first window
                doc.add(string_field("filter", "F")?);
            }
            if i == 0 {
                doc.add(string_field("foo", "A")?);
                doc.add(string_field("foo", "B")?);
            }
            // Place a scoring match beyond the filter range but within
            // outerWindowMax
            if i == INNER_WINDOW_SIZE + 4 {
                doc.add(string_field("foo", "A")?);
            }
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;
        w.close()?;
    }

    {
        let reader = directory_reader::open(dir.as_directory())?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let clause1 = boosted_csq("A", 2.0)?;
        let clause2 = csq("B");
        let filter: Arc<dyn Query> = tq("filter", "F");
        let context = first_leaf(&searcher)?;

        assert_dense_filter(&searcher, &filter, &context)?;

        let scorer1 = top_scorer(&searcher, &clause1, &context)?;
        let scorer2 = top_scorer(&searcher, &clause2, &context)?;
        let filter_scorer = top_scorer(&searcher, &filter, &context)?;

        // Score with a max that allows a second inner window. After the first
        // window, filter.doc = NO_MORE_DOCS (no filter matches >= 4096).
        // Essential queue top is at INNER_WINDOW_SIZE + 4 (< max), triggering
        // the bug in scoreInnerWindowWithFilter where the while loop advances
        // essentials past max.
        score_all(
            context.leaf_reader().max_doc(),
            vec![scorer1, scorer2],
            Some(filter_scorer),
            &mut OnlyDoc0Collector,
            None,
            num_docs,
        )?;
        drop(searcher);
        reader.close()?;
    }
    dir.close()
}

// -----------------------------------------------------------------------------
// Leap-frog versus bitset filter paths
// -----------------------------------------------------------------------------

/// The anonymous `LeafCollector`s of the leap-frog and bitset tests.
struct NoOpCollector;

impl LeafCollector for NoOpCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

/// A `TextField` with `Store.NO`.
fn text(name: &str, value: &str) -> Result<Box<TextField>> {
    Ok(Box::new(TextField::new(
        name,
        value.to_string(),
        Store::NO,
    )?))
}

/// The search of `testSparseFilterUsesLeapFrog` and
/// `testDenseScorersUseBitSet`: `(intoBitSet calls, advance calls)`.
fn count_filter_calls(
    dir: &TestDirectory,
    body1: &str,
    body2: &str,
) -> Result<(Arc<dyn DirectoryReader>, i32, i32)> {
    let reader = directory_reader::open(dir.as_directory())?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None);

    let counters = Arc::new(FilterCallCounters::default());
    let filter_query: Arc<dyn Query> = Arc::new(CountingFilterQuery {
        delegate: tq("filter", "yes"),
        counters: Arc::clone(&counters),
    });
    let inner_or: Arc<dyn Query> = Arc::new(
        BooleanQueryBuilder::new()
            .add(tq("body", body1), Occur::SHOULD)?
            .add(tq("body", body2), Occur::SHOULD)?
            .build(),
    );
    let outer_query: Arc<dyn Query> = Arc::new(
        BooleanQueryBuilder::new()
            .add(inner_or, Occur::MUST)?
            .add(filter_query, Occur::FILTER)?
            .build(),
    );

    let rewritten = searcher.rewrite(outer_query)?;
    let weight = searcher.create_weight(rewritten, ScoreMode::TOP_SCORES, 1.0)?;

    for ctx in (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()? {
        if let Some(mut ss) = weight.scorer_supplier(&ctx)? {
            let mut bs = ss.bulk_scorer()?;
            Assert::assert_true_msg(
                &format!(
                    "Expected MaxScoreBulkScorer but got {}",
                    std::any::type_name_of_val(&*bs)
                ),
                bs.as_any()
                    .is_some_and(|a| a.downcast_ref::<MaxScoreBulkScorer>().is_some()),
            );
            bs.score(&mut NoOpCollector, None, 0, NO_MORE_DOCS)
                .map_err(|e| e.into_lucene_error())?;
        }
    }
    Ok((
        reader,
        counters.into_bit_set_calls.load(Ordering::SeqCst),
        counters.advance_calls.load(Ordering::SeqCst),
    ))
}

/// When the filter is sparse relative to the scorers, MaxScoreBulkScorer
/// should use the leap-frog path (calling advance() on the filter) rather than
/// the bitset path (calling intoBitSet()). This simulates a FilteredOrHighMed
/// scenario: one dense scorer + one scorer sparser than the filter.
///
/// Port of `TestMaxScoreBulkScorer.testSparseFilterUsesLeapFrog`.
fn test_sparse_filter_uses_leap_frog() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    for i in 0..10000 {
        let mut doc = Document::new();
        doc.add(text("body", "high")?);
        if i % 50 == 0 {
            doc.add(text("body", "medium")?);
        }
        if i % 20 == 0 {
            doc.add(text("filter", "yes")?);
        }
        w.add_document(&doc)?;
    }
    w.close()?;

    // minScorerCost = min(10000, 200) = 200 < filter.cost(500) → leap-frog
    let (reader, into_bit_set_calls, advance_calls) = count_filter_calls(&dir, "high", "medium")?;

    Assert::assert_true_msg(
        &format!(
            "Expected advance() calls (leap-frog) but got intoBitSet={into_bit_set_calls} advance={advance_calls}"
        ),
        advance_calls > 0,
    );
    assert_equals_msg(
        Some("Expected no intoBitSet() calls for sparse filter"),
        &0,
        &into_bit_set_calls,
    );
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// When all scorers are individually denser than the filter,
/// MaxScoreBulkScorer should use the bitset path (calling intoBitSet() on the
/// filter). This simulates a FilteredOrHighHigh scenario: both scorers have
/// higher cost than the filter.
///
/// Port of `TestMaxScoreBulkScorer.testDenseScorersUseBitSet`.
fn test_dense_scorers_use_bit_set() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    for i in 0..10000 {
        let mut doc = Document::new();
        if i % 2 == 0 {
            doc.add(text("body", "dense1")?);
        }
        if i % 2 == 1 {
            doc.add(text("body", "dense2")?);
        }
        if i % 20 == 1 {
            doc.add(text("filter", "yes")?);
        }
        w.add_document(&doc)?;
    }
    w.close()?;

    // minScorerCost = min(5000, 5000) = 5000 >= filter.cost(500) → bitset path
    let (reader, into_bit_set_calls, advance_calls) = count_filter_calls(&dir, "dense1", "dense2")?;

    Assert::assert_true_msg(
        &format!(
            "Expected intoBitSet() calls (bitset path) but got intoBitSet={into_bit_set_calls} advance={advance_calls}"
        ),
        into_bit_set_calls > 0,
    );
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `intoBitSetCalls` and `advanceCalls`: the one-element arrays Java shares
/// with the query.
#[derive(Debug, Default)]
struct FilterCallCounters {
    into_bit_set_calls: AtomicI32,
    advance_calls: AtomicI32,
}

/// A query wrapper that counts intoBitSet() and advance() calls on the filter
/// iterator. This lets us detect which path MaxScoreBulkScorer takes without
/// exposing internal state.
///
/// `private static class CountingFilterQuery extends Query`.
#[derive(Debug, Clone)]
struct CountingFilterQuery {
    delegate: Arc<dyn Query>,
    counters: Arc<FilterCallCounters>,
}

impl Query for CountingFilterQuery {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let inner = self.delegate.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(CountingFilterWeight {
            base: FilterWeight::with_query(Arc::new(self.clone()), inner),
            counters: Arc::clone(&self.counters),
            score_mode,
        }))
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.delegate.visit(visitor);
    }

    fn to_query_string(&self, field: &str) -> String {
        format!("CountingFilter({})", self.delegate.to_query_string(field))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<CountingFilterQuery>()
            .is_some_and(|cfq| self.delegate.query_eq(&*cfq.delegate))
    }

    fn query_hash(&self) -> u64 {
        (31u32
            .wrapping_mul(self.class_hash() as u32)
            .wrapping_add(self.delegate.query_hash() as u32)) as u64
    }
}

/// The anonymous `FilterWeight` of `CountingFilterQuery.createWeight`.
#[derive(Debug)]
struct CountingFilterWeight {
    base: FilterWeight,
    counters: Arc<FilterCallCounters>,
    score_mode: ScoreMode,
}

impl SegmentCacheable for CountingFilterWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.base.is_cacheable(ctx)
    }
}

impl Weight for CountingFilterWeight {
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
        let Some(inner_ss) = self.base.inner.scorer_supplier(context)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(CountingScorerSupplier {
            inner_ss,
            counters: Arc::clone(&self.counters),
            score_mode: self.score_mode,
        })))
    }
}

/// The anonymous `ScorerSupplier` of `CountingFilterQuery`'s weight.
struct CountingScorerSupplier {
    inner_ss: Box<dyn ScorerSupplier>,
    counters: Arc<FilterCallCounters>,
    score_mode: ScoreMode,
}

impl ScorerSupplier for CountingScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let mut inner_scorer = self.inner_ss.get(lead_cost)?;
        let cost = inner_scorer.iterator().cost();
        let counting_iter = CountingIterator {
            inner_scorer,
            cost,
            counters: Arc::clone(&self.counters),
        };
        Ok(Box::new(ConstantScoreScorer::from_iterator(
            0.0,
            self.score_mode,
            Box::new(counting_iter),
        )))
    }

    fn cost(&self) -> i64 {
        self.inner_ss.cost()
    }
}

/// The anonymous `FilterDocIdSetIterator` of `CountingFilterQuery`, over
/// `innerScorer.iterator()`.
///
/// Java keeps the scorer's iterator; a Rust scorer lends its iterator, so
/// this iterator holds the scorer and reaches the iterator through it.
struct CountingIterator {
    inner_scorer: Box<dyn Scorer>,
    /// `in.cost()`, read when the iterator is built: it is stable.
    cost: i64,
    counters: Arc<FilterCallCounters>,
}

impl DocIdSetIterator for CountingIterator {
    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.counters
            .into_bit_set_calls
            .fetch_add(1, Ordering::SeqCst);
        self.inner_scorer
            .iterator()
            .into_bit_set(up_to, bit_set, offset)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.counters.advance_calls.fetch_add(1, Ordering::SeqCst);
        self.inner_scorer.iterator().advance(target)
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner_scorer.iterator().next_doc()
    }

    fn doc_id(&self) -> i32 {
        self.inner_scorer.doc_id()
    }

    /// `FilterDocIdSetIterator.cost()`: `in.cost()`.
    fn cost(&self) -> i64 {
        self.cost
    }
}
