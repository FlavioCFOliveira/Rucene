//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/TestBooleanScorerSupplier.java`.
//!
//! **Divergence from Lucene 10.5.0.** Java fills one
//! `Map<Occur, Collection<ScorerSupplier>>` and hands it to several
//! `BooleanScorerSupplier`s, which share the clause suppliers. Rucene's
//! `BooleanScorerSupplier::new` takes ownership of its clauses
//! (`ClauseSuppliers` of boxed suppliers), so the map holds the clause
//! *specifications* ([`Subs`]) and each construction builds fresh
//! `FakeScorerSupplier`s from them; a supplier's observable state (the
//! `topLevelScoringClause` flag) is shared between the copies of one clause.
//! Rucene's constructor also takes no `Weight`: Java's `FakeWeight` (or
//! `null`) has no counterpart argument.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::boolean_scorer_supplier::{BooleanScorerSupplier, ClauseSuppliers};
use rucene::search::doc_id_set_iterator::{all, DocIdSetIterator};
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::Weight;
use rucene::search::{Occur, Query, ScoreMode, NO_MORE_DOCS};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, class, expect_throws_panic, random, test,
};

/// `TestBooleanScorerSupplier`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBooleanScorerSupplier",
        vec![
            test("testConjunctionCost", test_conjunction_cost),
            test("testDisjunctionCost", test_disjunction_cost),
            test(
                "testDisjunctionWithMinShouldMatchCost",
                test_disjunction_with_min_should_match_cost,
            ),
            test("testDuelCost", test_duel_cost),
            test("testFakeScorerSupplier", test_fake_scorer_supplier),
            test("testConjunctionLeadCost", test_conjunction_lead_cost),
            test("testDisjunctionLeadCost", test_disjunction_lead_cost),
            test(
                "testDisjunctionWithMinShouldMatchLeadCost",
                test_disjunction_with_min_should_match_lead_cost,
            ),
            test("testProhibitedLeadCost", test_prohibited_lead_cost),
            test("testMixedLeadCost", test_mixed_lead_cost),
            test(
                "testDisjunctionTopLevelScoringClause",
                test_disjunction_top_level_scoring_clause,
            ),
            test(
                "testConjunctionTopLevelScoringClause",
                test_conjunction_top_level_scoring_clause,
            ),
            test(
                "testFilterTopLevelScoringClause",
                test_filter_top_level_scoring_clause,
            ),
            test(
                "testSingleMustScoringClause",
                test_single_must_scoring_clause,
            ),
            test(
                "testSingleShouldScoringClause",
                test_single_should_scoring_clause,
            ),
            test(
                "testMaxScoreNonTopLevelScoringClause",
                test_max_score_non_top_level_scoring_clause,
            ),
        ],
    )
}

/// `FakeWeight extends Weight`: kept for fidelity, although Rucene's
/// `BooleanScorerSupplier` takes no weight (see the module documentation).
#[derive(Debug)]
#[allow(dead_code)]
struct FakeWeight;

impl SegmentCacheable for FakeWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for FakeWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        // `super(MatchNoDocsQuery.INSTANCE)`
        Arc::new(MatchNoDocsQuery::instance())
    }

    fn explain(&self, _context: &LeafReaderContext, _doc: i32) -> Result<Explanation> {
        // Java returns `null`; an `Explanation` has no null here, so an
        // empty no-match explanation stands for it.
        Ok(Explanation::no_match("", Vec::new()))
    }

    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(None)
    }
}

/// `FakeScorer extends Scorer`.
struct FakeScorer {
    it: Box<dyn DocIdSetIterator>,
}

impl FakeScorer {
    fn new(cost: i64) -> Result<FakeScorer> {
        // `Math.toIntExact(cost)`
        let cost = i32::try_from(cost).map_err(|_| {
            rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Arithmetic(
                "integer overflow".to_string(),
            ))
        })?;
        Ok(FakeScorer {
            it: Box::new(all(cost)?),
        })
    }
}

impl std::fmt::Debug for FakeScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeScorer(cost={})", self.it.cost())
    }
}

impl Scorable for FakeScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(1.0)
    }
}

impl Scorer for FakeScorer {
    fn doc_id(&self) -> i32 {
        self.it.doc_id()
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(1.0)
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.it
    }
}

/// `FakeScorerSupplier extends ScorerSupplier`.
struct FakeScorerSupplier {
    cost: i64,
    lead_cost: Option<i64>,
    top_level_scoring_clause: Arc<AtomicBool>,
}

impl FakeScorerSupplier {
    /// `new FakeScorerSupplier(cost)`.
    fn new(cost: i64) -> FakeScorerSupplier {
        FakeScorerSupplier {
            cost,
            lead_cost: None,
            top_level_scoring_clause: Arc::new(AtomicBool::new(false)),
        }
    }

    /// `new FakeScorerSupplier(cost, leadCost)`.
    fn with_lead_cost(cost: i64, lead_cost: i64) -> FakeScorerSupplier {
        FakeScorerSupplier {
            cost,
            lead_cost: Some(lead_cost),
            top_level_scoring_clause: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A copy sharing this supplier's `topLevelScoringClause` flag (see the
    /// module documentation).
    fn copy(&self) -> FakeScorerSupplier {
        FakeScorerSupplier {
            cost: self.cost,
            lead_cost: self.lead_cost,
            top_level_scoring_clause: Arc::clone(&self.top_level_scoring_clause),
        }
    }

    fn top_level_scoring_clause(&self) -> bool {
        self.top_level_scoring_clause.load(Ordering::SeqCst)
    }
}

impl std::fmt::Display for FakeScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FakeLazyScorer(cost={},leadCost={})",
            self.cost,
            match self.lead_cost {
                Some(l) => l.to_string(),
                None => "null".to_string(),
            }
        )
    }
}

impl ScorerSupplier for FakeScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        if let Some(this_lead_cost) = self.lead_cost {
            if this_lead_cost < self.cost {
                // If the expected lead cost is less than the cost, ie. another clause is leading
                // iteration, then the exact lead cost must be provided.
                assert_equals_msg(
                    Some(&format!("{self} actual leadCost={lead_cost}")),
                    &this_lead_cost,
                    &lead_cost,
                );
            } else {
                // Otherwise the lead cost may be provided as the cost of this very clause or as
                // Long.MAX_VALUE (typically for bulk scorers), both signaling that this clause is leading
                // iteration.
                Assert::assert_true_msg(
                    &format!("{self} actual leadCost={lead_cost}"),
                    lead_cost >= this_lead_cost,
                );
            }
        }
        Ok(Box::new(FakeScorer::new(self.cost)?))
    }

    fn cost(&self) -> i64 {
        self.cost
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        self.top_level_scoring_clause.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// `Map<Occur, Collection<ScorerSupplier>>` filled with every `Occur`, as
/// clause specifications (see the module documentation).
#[derive(Default)]
struct Subs {
    must: Vec<FakeScorerSupplier>,
    filter: Vec<FakeScorerSupplier>,
    should: Vec<FakeScorerSupplier>,
    must_not: Vec<FakeScorerSupplier>,
}

impl Subs {
    /// `subs.get(occur)`.
    fn get(&mut self, occur: Occur) -> &mut Vec<FakeScorerSupplier> {
        match occur {
            Occur::MUST => &mut self.must,
            Occur::FILTER => &mut self.filter,
            Occur::SHOULD => &mut self.should,
            Occur::MUST_NOT => &mut self.must_not,
        }
    }

    fn suppliers(&self) -> ClauseSuppliers {
        let boxed = |v: &[FakeScorerSupplier]| -> Vec<Box<dyn ScorerSupplier>> {
            v.iter()
                .map(|s| Box::new(s.copy()) as Box<dyn ScorerSupplier>)
                .collect()
        };
        ClauseSuppliers {
            should: boxed(&self.should),
            must: boxed(&self.must),
            filter: boxed(&self.filter),
            must_not: boxed(&self.must_not),
        }
    }
}

impl std::fmt::Display for Subs {
    /// `EnumMap.toString()`: `{MUST=[...], FILTER=[...], SHOULD=[...], MUST_NOT=[...]}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = |v: &[FakeScorerSupplier]| -> String {
            let items: Vec<String> = v.iter().map(|s| s.to_string()).collect();
            format!("[{}]", items.join(", "))
        };
        write!(
            f,
            "{{MUST={}, FILTER={}, SHOULD={}, MUST_NOT={}}}",
            list(&self.must),
            list(&self.filter),
            list(&self.should),
            list(&self.must_not)
        )
    }
}

/// `Occur.values()`, in declaration order.
const OCCUR_VALUES: [Occur; 4] = [Occur::MUST, Occur::FILTER, Occur::SHOULD, Occur::MUST_NOT];

/// `ScoreMode.values()`, in declaration order.
const SCORE_MODE_VALUES: [ScoreMode; 5] = [
    ScoreMode::COMPLETE,
    ScoreMode::COMPLETE_NO_SCORES,
    ScoreMode::TOP_SCORES,
    ScoreMode::TOP_DOCS,
    ScoreMode::TOP_DOCS_WITH_SCORES,
];

/// `RandomPicks.randomFrom(random(), ScoreMode.values())`.
fn random_score_mode() -> ScoreMode {
    *RandomPicks::random_from(&*random(), &SCORE_MODE_VALUES)
}

/// `RandomPicks.randomFrom(random(), Arrays.asList(Occur.FILTER, Occur.MUST))`.
fn random_required() -> Occur {
    *RandomPicks::random_from_list(&*random(), &[Occur::FILTER, Occur::MUST])
}

/// `new BooleanScorerSupplier(new FakeWeight(), subs, scoreMode, minShouldMatch, maxDoc)`.
fn bss(
    subs: &Subs,
    score_mode: ScoreMode,
    min_should_match: i32,
    max_doc: i32,
) -> Result<BooleanScorerSupplier> {
    BooleanScorerSupplier::new(subs.suppliers(), score_mode, min_should_match, max_doc)
}

fn test_conjunction_cost() -> Result<()> {
    let mut subs = Subs::default();

    subs.get(random_required())
        .push(FakeScorerSupplier::new(42));
    assert_equals(&42, &bss(&subs, random_score_mode(), 0, 100)?.cost());

    subs.get(random_required())
        .push(FakeScorerSupplier::new(12));
    assert_equals(&12, &bss(&subs, random_score_mode(), 0, 100)?.cost());

    subs.get(random_required())
        .push(FakeScorerSupplier::new(20));
    assert_equals(&12, &bss(&subs, random_score_mode(), 0, 100)?.cost());
    Ok(())
}

/// `assertEquals(expected, s.cost()); assertEquals(expected,
/// s.get(random().nextInt(100)).iterator().cost())`.
fn assert_costs(mut s: BooleanScorerSupplier, expected: i64) -> Result<()> {
    assert_equals(&expected, &s.cost());
    assert_equals(
        &expected,
        &s.get(random().next_int_bound(100) as i64)?
            .iterator()
            .cost(),
    );
    Ok(())
}

fn test_disjunction_cost() -> Result<()> {
    let mut subs = Subs::default();

    subs.get(Occur::SHOULD).push(FakeScorerSupplier::new(42));
    assert_costs(bss(&subs, random_score_mode(), 0, 100)?, 42)?;

    subs.get(Occur::SHOULD).push(FakeScorerSupplier::new(12));
    assert_costs(bss(&subs, random_score_mode(), 0, 100)?, 42 + 12)?;

    subs.get(Occur::SHOULD).push(FakeScorerSupplier::new(20));
    assert_costs(bss(&subs, random_score_mode(), 0, 100)?, 42 + 12 + 20)
}

fn test_disjunction_with_min_should_match_cost() -> Result<()> {
    let mut subs = Subs::default();

    subs.get(Occur::SHOULD).push(FakeScorerSupplier::new(42));
    subs.get(Occur::SHOULD).push(FakeScorerSupplier::new(12));
    assert_costs(bss(&subs, random_score_mode(), 1, 100)?, 42 + 12)?;

    subs.get(Occur::SHOULD).push(FakeScorerSupplier::new(20));
    assert_costs(bss(&subs, random_score_mode(), 1, 100)?, 42 + 12 + 20)?;
    assert_costs(bss(&subs, random_score_mode(), 2, 100)?, 12 + 20)?;

    subs.get(Occur::SHOULD).push(FakeScorerSupplier::new(30));
    assert_costs(bss(&subs, random_score_mode(), 1, 100)?, 42 + 12 + 20 + 30)?;
    assert_costs(bss(&subs, random_score_mode(), 2, 100)?, 12 + 20 + 30)?;
    assert_costs(bss(&subs, random_score_mode(), 3, 100)?, 12 + 20)
}

fn test_duel_cost() -> Result<()> {
    let iters = LuceneTestCase::at_least(1000);
    for _iter in 0..iters {
        let mut subs = Subs::default();
        let mut num_clauses = TestUtil::next_int(&*random(), 1, 10);
        let mut num_shoulds = 0;
        let mut num_required = 0;
        for _j in 0..num_clauses {
            let occur = *RandomPicks::random_from(&*random(), &OCCUR_VALUES);
            subs.get(occur)
                .push(FakeScorerSupplier::new(random().next_int_bound(100) as i64));
            if occur == Occur::SHOULD {
                num_shoulds += 1;
            } else if occur == Occur::FILTER || occur == Occur::MUST {
                num_required += 1;
            }
        }
        let score_mode = random_score_mode();
        if !score_mode.needs_scores() && num_required > 0 {
            num_clauses -= num_shoulds;
            num_shoulds = 0;
            subs.get(Occur::SHOULD).clear();
        }
        let _ = num_clauses;
        if num_shoulds + num_required == 0 {
            // only negative clauses, invalid
            continue;
        }
        let min_should_match = if num_shoulds == 0 {
            0
        } else {
            TestUtil::next_int(&*random(), 0, num_shoulds - 1)
        };
        let mut supplier = bss(&subs, score_mode, min_should_match, 100)?;
        let cost1 = supplier.cost();
        let cost2 = supplier.get(i64::MAX)?.iterator().cost();
        assert_equals_msg(
            Some(&format!(
                "clauses={subs}, minShouldMatch={min_should_match}"
            )),
            &cost1,
            &cost2,
        );
    }
    Ok(())
}

// test the tester...
fn test_fake_scorer_supplier() -> Result<()> {
    let mut random_access_supplier =
        FakeScorerSupplier::with_lead_cost(TestUtil::next_int(&*random(), 31, 100) as i64, 30);
    expect_throws_panic("java.lang.AssertionError", || {
        let _ = random_access_supplier.get(70);
    });
    let mut sequential_supplier =
        FakeScorerSupplier::with_lead_cost(random().next_int_bound(70) as i64, 70);
    expect_throws_panic("java.lang.AssertionError", || {
        let _ = sequential_supplier.get(30);
    });
    Ok(())
}

fn test_conjunction_lead_cost() -> Result<()> {
    let mut subs = Subs::default();

    // If the clauses are less costly than the lead cost, the min cost is the new lead cost
    subs.get(random_required())
        .push(FakeScorerSupplier::with_lead_cost(42, 12));
    subs.get(random_required())
        .push(FakeScorerSupplier::with_lead_cost(12, 12));
    bss(&subs, random_score_mode(), 0, 100)?.get(i64::MAX)?; // triggers assertions as a side-effect
    bss(&subs, random_score_mode(), 0, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    let mut subs = Subs::default();

    // If the lead cost is less that the clauses' cost, then we don't modify it
    subs.get(random_required())
        .push(FakeScorerSupplier::with_lead_cost(42, 7));
    subs.get(random_required())
        .push(FakeScorerSupplier::with_lead_cost(12, 7));
    bss(&subs, random_score_mode(), 0, 100)?.get(7)?; // triggers assertions as a side-effect
    Ok(())
}

fn test_disjunction_lead_cost() -> Result<()> {
    let mut subs = Subs::default();
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(42, 54));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(12, 54));
    bss(&subs, random_score_mode(), 0, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, random_score_mode(), 0, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    subs.get(Occur::SHOULD).clear();
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(42, 20));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(12, 20));
    bss(&subs, random_score_mode(), 0, 100)?.get(20)?; // triggers assertions as a side-effect
    Ok(())
}

fn test_disjunction_with_min_should_match_lead_cost() -> Result<()> {
    let mut subs = Subs::default();

    // minShouldMatch is 2 so the 2 least costly clauses will lead iteration
    // and their cost will be 30+12=42
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(50, 42));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(12, 42));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(30, 42));
    bss(&subs, random_score_mode(), 2, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, random_score_mode(), 2, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    let mut subs = Subs::default();

    // If the leadCost is less than the msm cost, then it wins
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(42, 20));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(12, 20));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(30, 20));
    bss(&subs, random_score_mode(), 2, 100)?.get(20)?; // triggers assertions as a side-effect

    let mut subs = Subs::default();

    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(42, 62));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(12, 62));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(30, 62));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(20, 62));
    bss(&subs, random_score_mode(), 2, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, random_score_mode(), 2, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    let mut subs = Subs::default();

    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(42, 32));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(12, 32));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(30, 32));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(20, 32));
    bss(&subs, random_score_mode(), 3, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, random_score_mode(), 3, 100)?.bulk_scorer()?; // triggers assertions as a side-effect
    Ok(())
}

fn test_prohibited_lead_cost() -> Result<()> {
    let mut subs = Subs::default();

    // The MUST_NOT clause is called with the same lead cost as the MUST clause
    subs.get(Occur::MUST)
        .push(FakeScorerSupplier::with_lead_cost(42, 42));
    subs.get(Occur::MUST_NOT)
        .push(FakeScorerSupplier::with_lead_cost(30, 42));
    bss(&subs, random_score_mode(), 0, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, random_score_mode(), 0, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    subs.get(Occur::MUST).clear();
    subs.get(Occur::MUST_NOT).clear();
    subs.get(Occur::MUST)
        .push(FakeScorerSupplier::with_lead_cost(42, 42));
    subs.get(Occur::MUST_NOT)
        .push(FakeScorerSupplier::with_lead_cost(80, 42));
    bss(&subs, random_score_mode(), 0, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, random_score_mode(), 0, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    subs.get(Occur::MUST).clear();
    subs.get(Occur::MUST_NOT).clear();
    subs.get(Occur::MUST)
        .push(FakeScorerSupplier::with_lead_cost(42, 20));
    subs.get(Occur::MUST_NOT)
        .push(FakeScorerSupplier::with_lead_cost(30, 20));
    bss(&subs, random_score_mode(), 0, 100)?.get(20)?; // triggers assertions as a side-effect
    Ok(())
}

fn test_mixed_lead_cost() -> Result<()> {
    let mut subs = Subs::default();

    // The SHOULD clause is always called with the same lead cost as the MUST clause
    subs.get(Occur::MUST)
        .push(FakeScorerSupplier::with_lead_cost(42, 42));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(30, 42));
    bss(&subs, ScoreMode::COMPLETE, 0, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, ScoreMode::COMPLETE, 0, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    subs.get(Occur::MUST).clear();
    subs.get(Occur::SHOULD).clear();
    subs.get(Occur::MUST)
        .push(FakeScorerSupplier::with_lead_cost(42, 42));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(80, 42));
    bss(&subs, ScoreMode::COMPLETE, 0, 100)?.get(100)?; // triggers assertions as a side-effect
    bss(&subs, ScoreMode::COMPLETE, 0, 100)?.bulk_scorer()?; // triggers assertions as a side-effect

    subs.get(Occur::MUST).clear();
    subs.get(Occur::SHOULD).clear();
    subs.get(Occur::MUST)
        .push(FakeScorerSupplier::with_lead_cost(42, 20));
    subs.get(Occur::SHOULD)
        .push(FakeScorerSupplier::with_lead_cost(80, 20));
    bss(&subs, ScoreMode::COMPLETE, 0, 100)?.get(20)?; // triggers assertions as a side-effect
    Ok(())
}

/// The body of the `*TopLevelScoringClause` tests: clauses `occur1` and
/// `occur2`, `setTopLevelScoringClause()` on a `TOP_SCORES` supplier, and
/// the expected flags.
fn top_level_scoring_clause(
    occur1: Occur,
    occur2: Occur,
    expected1: bool,
    expected2: bool,
) -> Result<()> {
    let mut subs = Subs::default();

    let clause1 = FakeScorerSupplier::with_lead_cost(10, 10);
    let flag1 = clause1.copy();
    subs.get(occur1).push(clause1);
    let clause2 = FakeScorerSupplier::with_lead_cost(10, 10);
    let flag2 = clause2.copy();
    subs.get(occur2).push(clause2);

    bss(&subs, ScoreMode::TOP_SCORES, 0, 100)?.set_top_level_scoring_clause()?;
    assert_equals(&expected1, &flag1.top_level_scoring_clause());
    assert_equals(&expected2, &flag2.top_level_scoring_clause());
    Ok(())
}

fn test_disjunction_top_level_scoring_clause() -> Result<()> {
    // assertFalse(clause1...); assertFalse(clause2...)
    top_level_scoring_clause(Occur::SHOULD, Occur::SHOULD, false, false)
}

fn test_conjunction_top_level_scoring_clause() -> Result<()> {
    top_level_scoring_clause(Occur::MUST, Occur::MUST, false, false)
}

fn test_filter_top_level_scoring_clause() -> Result<()> {
    top_level_scoring_clause(Occur::FILTER, Occur::FILTER, false, false)
}

fn test_single_must_scoring_clause() -> Result<()> {
    // assertTrue(clause1...); assertFalse(clause2...)
    top_level_scoring_clause(Occur::MUST, Occur::FILTER, true, false)
}

fn test_single_should_scoring_clause() -> Result<()> {
    top_level_scoring_clause(Occur::SHOULD, Occur::MUST_NOT, true, false)
}

fn test_max_score_non_top_level_scoring_clause() -> Result<()> {
    let mut subs = Subs::default();

    let clause1 = FakeScorerSupplier::with_lead_cost(10, 10);
    let clause2 = FakeScorerSupplier::with_lead_cost(10, 10);
    subs.get(Occur::MUST).push(clause1.copy());
    subs.get(Occur::MUST).push(clause2.copy());

    let mut scorer = bss(&subs, ScoreMode::TOP_SCORES, 0, 100)?.get(10)?;
    assert_equals_delta(2.0, scorer.get_max_score(NO_MORE_DOCS)? as f64, 0.0);

    let mut subs = Subs::default();

    subs.get(Occur::SHOULD).push(clause1);
    subs.get(Occur::SHOULD).push(clause2);

    let mut scorer = bss(&subs, ScoreMode::TOP_SCORES, 0, 100)?.get(10)?;
    assert_equals_delta(2.0, scorer.get_max_score(NO_MORE_DOCS)? as f64, 0.0);
    Ok(())
}
