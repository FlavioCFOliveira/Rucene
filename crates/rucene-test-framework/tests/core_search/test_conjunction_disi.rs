//! Port of `lucene/core/src/test/org/apache/lucene/search/TestConjunctionDISI.java`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::conjunction_scorer::ConjunctionScorer;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::{
    all, range, ConjunctionUtils, ConstantScoreScorer, DocIdSetIterator, ScoreMode, ScorerIterator,
    TwoPhaseIterator, TwoPhaseIteratorAsDocIdSetIterator, NO_MORE_DOCS,
};
use rucene::util::bit_set::BitSetIterator;
use rucene::util::FixedBitSet;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, expect_throws_panic, random, test};

/// `TestConjunctionDISI`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestConjunctionDISI",
        vec![
            test("testConjunction", test_conjunction),
            test(
                "testConjunctionApproximation",
                test_conjunction_approximation,
            ),
            test(
                "testRecursiveConjunctionApproximation",
                test_recursive_conjunction_approximation,
            ),
            test("testCollapseSubConjunctionDISIs", || {
                test_collapse_sub_conjunctions(false)
            }),
            test("testCollapseSubConjunctionScorers", || {
                test_collapse_sub_conjunctions(true)
            }),
            test(
                "testIllegalAdvancementOfSubIteratorsTripsAssertion",
                test_illegal_advancement_of_sub_iterators_trips_assertion,
            ),
            test(
                "testBitSetConjunctionDISIDocIDOnExhaust",
                test_bit_set_conjunction_disi_doc_id_on_exhaust,
            ),
        ],
    )
}

const UOE: &str = "java.lang.UnsupportedOperationException";

fn uoe() -> ! {
    Throwable::new(UOE, "ConjunctionDISI should call the two-phase iterator").throw()
}

/// `new BitDocIdSet(set).iterator()`: a `BitSetIterator` whose cost is the
/// set's approximate cardinality.
fn bit_doc_id_set_iterator(set: &FixedBitSet) -> Result<Box<dyn DocIdSetIterator>> {
    let cost = set.approximate_cardinality() as i64;
    Ok(Box::new(BitSetIterator::new(Arc::new(set.clone()), cost)?))
}

/// The anonymous `TwoPhaseIterator` of `approximation(iterator, confirmed)`.
struct Approximation {
    approximation: Box<dyn DocIdSetIterator>,
    confirmed: FixedBitSet,
}

impl TwoPhaseIterator for Approximation {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(self.confirmed.get(self.approximation.doc_id() as usize))
    }

    fn match_cost(&self) -> f32 {
        5.0 // #operations in FixedBitSet#get()
    }
}

/// Create a {@link TwoPhaseIterator} that will match documents that are
/// both in the provided iterator and in `confirmed`.
fn approximation(
    iterator: Box<dyn DocIdSetIterator>,
    confirmed: FixedBitSet,
) -> Box<dyn TwoPhaseIterator> {
    let approximation = if random().next_boolean() {
        anonymize_iterator(iterator)
    } else {
        iterator
    };
    Box::new(Approximation {
        approximation,
        confirmed,
    })
}

/// The anonymous `DocIdSetIterator` of `anonymizeIterator`.
struct Anonymized(Box<dyn DocIdSetIterator>);

impl DocIdSetIterator for Anonymized {
    fn next_doc(&mut self) -> Result<i32> {
        self.0.next_doc()
    }

    fn doc_id(&self) -> i32 {
        self.0.doc_id()
    }

    fn cost(&self) -> i64 {
        // Java returns `it.docID()` here.
        i64::from(self.0.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.advance(target)
    }
}

/// Return an anonym class so that ConjunctionDISI cannot optimize it like it
/// does eg. for BitSetIterators.
fn anonymize_iterator(it: Box<dyn DocIdSetIterator>) -> Box<dyn DocIdSetIterator> {
    Box::new(Anonymized(it))
}

/// The iteration the anonymous `Scorer` of `scorer(it, twoPhaseIterator)`
/// holds: `it`, which is `asDocIdSetIterator(twoPhaseIterator)` when the
/// two-phase iterator is not null.
enum ScorerView {
    Plain(Box<dyn DocIdSetIterator>),
    TwoPhase(Box<dyn TwoPhaseIterator>),
}

impl DocIdSetIterator for ScorerView {
    fn doc_id(&self) -> i32 {
        match self {
            ScorerView::Plain(it) => it.doc_id(),
            // `asDocIdSetIterator(tpi).docID()` is the approximation's.
            ScorerView::TwoPhase(tpi) => tpi.approximation_ref().doc_id(),
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        match self {
            ScorerView::Plain(it) => it.next_doc(),
            ScorerView::TwoPhase(_) => uoe(),
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        match self {
            ScorerView::Plain(it) => it.advance(target),
            ScorerView::TwoPhase(_) => uoe(),
        }
    }

    fn cost(&self) -> i64 {
        match self {
            ScorerView::Plain(it) => it.cost(),
            ScorerView::TwoPhase(_) => uoe(),
        }
    }
}

/// The anonymous `Scorer`.
struct TestScorer {
    view: ScorerView,
}

impl Scorable for TestScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }
}

impl Scorer for TestScorer {
    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.view
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        match &mut self.view {
            ScorerView::Plain(_) => None,
            ScorerView::TwoPhase(tpi) => Some(&mut **tpi),
        }
    }

    fn doc_id(&self) -> i32 {
        match &self.view {
            ScorerView::Plain(it) => it.doc_id(),
            ScorerView::TwoPhase(_) => uoe(),
        }
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(0.0)
    }
}

/// `scorer(TwoPhaseIterator)`.
fn scorer_two_phase(two_phase_iterator: Box<dyn TwoPhaseIterator>) -> Box<dyn Scorer> {
    Box::new(TestScorer {
        view: ScorerView::TwoPhase(two_phase_iterator),
    })
}

/// `scorer(conj, TwoPhaseIterator.unwrap(conj))`.
fn test_scorer_of(conj: ScorerIterator) -> TestScorer {
    TestScorer {
        view: match conj {
            ScorerIterator::Simple(it) => ScorerView::Plain(it),
            ScorerIterator::TwoPhase(tpi) => ScorerView::TwoPhase(tpi),
        },
    }
}

fn random_set(max_doc: i32) -> FixedBitSet {
    let step = TestUtil::next_int(&*random(), 1, 10);
    let mut set = FixedBitSet::new(max_doc as usize);
    let mut doc = random().next_int_bound(step);
    while doc < max_doc {
        set.set(doc as usize);
        doc += TestUtil::next_int(&*random(), 1, step);
    }
    set
}

fn clear_random_bits(other: &FixedBitSet) -> FixedBitSet {
    let mut set = FixedBitSet::new(other.length());
    set.or_set(other);
    for i in 0..set.length() {
        if random().next_boolean() {
            set.clear(i);
        }
    }
    set
}

fn intersect(bit_sets: &[FixedBitSet]) -> FixedBitSet {
    let mut intersection = FixedBitSet::new(bit_sets[0].length());
    intersection.or_set(&bit_sets[0]);
    for b in &bit_sets[1..] {
        intersection.and(b);
    }
    intersection
}

fn to_bit_set(max_doc: i32, iterator: &mut dyn DocIdSetIterator) -> Result<FixedBitSet> {
    let mut set = FixedBitSet::new(max_doc as usize);
    let mut doc = iterator.next_doc()?;
    while doc != NO_MORE_DOCS {
        set.set(doc as usize);
        doc = iterator.next_doc()?;
    }
    Ok(set)
}

/// `TwoPhaseIterator.asDocIdSetIterator(...)` of a conjunction, or the
/// conjunction itself.
fn into_disi(conjunction: ScorerIterator) -> Box<dyn DocIdSetIterator> {
    conjunction.into_doc_id_set_iterator()
}

fn css(score_mode: ScoreMode, it: Box<dyn DocIdSetIterator>) -> Box<dyn Scorer> {
    Box::new(ConstantScoreScorer::from_iterator(0.0, score_mode, it))
}

/// The three-way choice of `testConjunction` and
/// `testRecursiveConjunctionApproximation`; returns the scorer, its set and
/// whether it has an approximation.
fn random_scorer_3(max_doc: i32) -> Result<(Box<dyn Scorer>, FixedBitSet, bool)> {
    let set = random_set(max_doc);
    Ok(match random().next_int_bound(3) {
        0 => {
            let it = anonymize_iterator(bit_doc_id_set_iterator(&set)?);
            (css(ScoreMode::TOP_SCORES, it), set, false)
        }
        1 => {
            let it = bit_doc_id_set_iterator(&set)?;
            (css(ScoreMode::TOP_SCORES, it), set, false)
        }
        _ => {
            let confirmed = clear_random_bits(&set);
            let approximation = approximation(bit_doc_id_set_iterator(&set)?, confirmed.clone());
            (scorer_two_phase(approximation), confirmed, true)
        }
    })
}

fn test_conjunction() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        let max_doc = TestUtil::next_int(&*random(), 100, 10000);
        let num_iterators = TestUtil::next_int(&*random(), 2, 5);
        let mut sets = Vec::new();
        let mut iterators = Vec::new();
        for _ in 0..num_iterators {
            let (scorer, set, _) = random_scorer_3(max_doc)?;
            sets.push(set);
            iterators.push(scorer);
        }

        let mut conjunction = into_disi(ConjunctionUtils::intersect_scorers(iterators)?);
        assert_equals(&intersect(&sets), &to_bit_set(max_doc, &mut *conjunction)?);
    }
    Ok(())
}

fn test_conjunction_approximation() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        let max_doc = TestUtil::next_int(&*random(), 100, 10000);
        let num_iterators = TestUtil::next_int(&*random(), 2, 5);
        let mut sets = Vec::new();
        let mut iterators = Vec::new();
        let mut has_approximation = false;
        for _ in 0..num_iterators {
            let set = random_set(max_doc);
            if random().next_boolean() {
                let it = bit_doc_id_set_iterator(&set)?;
                sets.push(set);
                iterators.push(css(ScoreMode::COMPLETE_NO_SCORES, it));
            } else {
                let confirmed = clear_random_bits(&set);
                let approximation =
                    approximation(bit_doc_id_set_iterator(&set)?, confirmed.clone());
                sets.push(confirmed);
                iterators.push(scorer_two_phase(approximation));
                has_approximation = true;
            }
        }

        let conjunction = ConjunctionUtils::intersect_scorers(iterators)?;
        let two_phase_iterator = match conjunction {
            ScorerIterator::TwoPhase(tpi) => Some(tpi),
            ScorerIterator::Simple(_) => None,
        };
        assert_equals(&has_approximation, &two_phase_iterator.is_some());
        if has_approximation {
            let mut disi = TwoPhaseIteratorAsDocIdSetIterator::new(
                two_phase_iterator.expect("has approximation"),
            );
            assert_equals(&intersect(&sets), &to_bit_set(max_doc, &mut disi)?);
        }
    }
    Ok(())
}

fn test_recursive_conjunction_approximation() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        let max_doc = TestUtil::next_int(&*random(), 100, 10000);
        let num_iterators = TestUtil::next_int(&*random(), 2, 5);
        let mut sets = Vec::new();
        // The first iterator, then the `scorer(conj, unwrap(conj))` of each
        // conjunction built over it.
        let mut first: Option<Box<dyn Scorer>> = None;
        let mut conjunction: Option<TestScorer> = None;
        let mut has_approximation = false;
        for _ in 0..num_iterators {
            let (new_iterator, set, approximated) = random_scorer_3(max_doc)?;
            sets.push(set);
            has_approximation |= approximated;

            if first.is_none() && conjunction.is_none() {
                first = Some(new_iterator);
            } else {
                let current: Box<dyn Scorer> = match conjunction.take() {
                    Some(c) => Box::new(c),
                    None => first.take().expect("the first iterator"),
                };
                let conj = ConjunctionUtils::intersect_scorers(vec![current, new_iterator])?;
                conjunction = Some(test_scorer_of(conj));
            }
        }

        // `numIterators >= 2`, so the conjunction is a scorer of this test.
        let conjunction = conjunction.expect("at least two iterators");
        let two_phase_iterator = match conjunction.view {
            ScorerView::TwoPhase(tpi) => Ok(tpi),
            ScorerView::Plain(it) => Err(it),
        };
        assert_equals(&has_approximation, &two_phase_iterator.is_ok());
        match two_phase_iterator {
            Ok(tpi) => {
                let mut disi = TwoPhaseIteratorAsDocIdSetIterator::new(tpi);
                assert_equals(&intersect(&sets), &to_bit_set(max_doc, &mut disi)?);
            }
            Err(mut it) => {
                // `conjunction.iterator()`.
                assert_equals(&intersect(&sets), &to_bit_set(max_doc, &mut *it)?);
            }
        }
    }
    Ok(())
}

/// `testCollapseSubConjunctions(boolean)`.
fn test_collapse_sub_conjunctions(wrap_with_scorer: bool) -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        let max_doc = TestUtil::next_int(&*random(), 100, 10000);
        let num_iterators = TestUtil::next_int(&*random(), 5, 10);
        let mut sets = Vec::new();
        let mut scorers: Vec<Box<dyn Scorer>> = Vec::new();
        for _ in 0..num_iterators {
            let set = random_set(max_doc);
            if random().next_boolean() {
                let it = bit_doc_id_set_iterator(&set)?;
                sets.push(set);
                scorers.push(css(ScoreMode::TOP_SCORES, it));
            } else {
                let confirmed = clear_random_bits(&set);
                let approximation =
                    approximation(bit_doc_id_set_iterator(&set)?, confirmed.clone());
                sets.push(confirmed);
                scorers.push(scorer_two_phase(approximation));
            }
        }

        // make some sub sequences into sub conjunctions
        let sub_iters = LuceneTestCase::at_least(3);
        let mut sub_iter = 0;
        while sub_iter < sub_iters && scorers.len() > 3 {
            let sub_seq_start =
                TestUtil::next_int(&*random(), 0, scorers.len() as i32 - 2) as usize;
            let sub_seq_end =
                TestUtil::next_int(&*random(), sub_seq_start as i32 + 2, scorers.len() as i32)
                    as usize;
            let sub_iterators: Vec<Box<dyn Scorer>> =
                scorers.drain(sub_seq_start..sub_seq_end).collect();
            let sub_conjunction: Box<dyn Scorer> = if wrap_with_scorer {
                Box::new(ConjunctionScorer::new(sub_iterators, Vec::new())?)
            } else {
                match ConjunctionUtils::intersect_scorers(sub_iterators)? {
                    ScorerIterator::Simple(it) => css(ScoreMode::TOP_SCORES, it),
                    ScorerIterator::TwoPhase(tpi) => Box::new(ConstantScoreScorer::from_iterator(
                        0.0,
                        ScoreMode::TOP_SCORES,
                        Box::new(TwoPhaseIteratorAsDocIdSetIterator::new(tpi)),
                    )),
                }
            };
            scorers.insert(sub_seq_start, sub_conjunction);
            sub_iter += 1;
        }

        if scorers.len() == 1 {
            // ConjunctionDISI needs two iterators
            scorers.push(css(ScoreMode::TOP_SCORES, Box::new(all(max_doc)?)));
        }

        let mut conjunction = into_disi(ConjunctionUtils::intersect_scorers(scorers)?);
        assert_equals(&intersect(&sets), &to_bit_set(max_doc, &mut *conjunction)?);
    }
    Ok(())
}

/// A sub-iterator the test also keeps a handle on, as Java keeps the
/// `iterators` array while the conjunction holds the same objects.
///
/// **Divergence from Lucene 10.5.0.** A boxed iterator given to the
/// conjunction is moved into it; the test advances it from outside through
/// this shared view. Every sub-iterator has the same cost here, so
/// `ConjunctionUtils` puts none of them on its bit-set path either way.
struct Shared(Rc<RefCell<Box<dyn DocIdSetIterator>>>);

impl DocIdSetIterator for Shared {
    fn doc_id(&self) -> i32 {
        self.0.borrow().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.0.borrow_mut().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.borrow_mut().advance(target)
    }

    fn cost(&self) -> i64 {
        self.0.borrow().cost()
    }
}

fn test_illegal_advancement_of_sub_iterators_trips_assertion() -> Result<()> {
    LuceneTestCase::assume_true(
        "Assertions must be enabled for this test!",
        LuceneTestCase::ASSERTS_ARE_ENABLED,
    );
    let max_doc = 100;
    let num_iterators = TestUtil::next_int(&*random(), 2, 5);
    let set = random_set(max_doc);

    let mut iterators = Vec::new();
    for _ in 0..num_iterators {
        iterators.push(Rc::new(RefCell::new(bit_doc_id_set_iterator(&set)?)));
    }
    let mut conjunction = into_disi(ConjunctionUtils::intersect_iterators(
        iterators
            .iter()
            .map(|it| Box::new(Shared(Rc::clone(it))) as Box<dyn DocIdSetIterator>)
            .collect(),
    )?);
    let idx = TestUtil::next_int(&*random(), 0, iterators.len() as i32 - 1) as usize;
    iterators[idx].borrow_mut().next_doc()?; // illegally advancing one of the sub-iterators outside of the conjunction

    let ex = expect_throws_panic("java.lang.AssertionError", || {
        let _ = conjunction.next_doc();
    });
    assert_equals(
        &Some("Sub-iterators of ConjunctionDISI are not on the same document!".to_string()),
        &ex.message().map(str::to_string),
    );
    Ok(())
}

fn test_bit_set_conjunction_disi_doc_id_on_exhaust() -> Result<()> {
    let num_bit_set_iterators = TestUtil::next_int(&*random(), 2, 5);
    let mut iterators: Vec<Box<dyn DocIdSetIterator>> = Vec::new();
    let max_bit_set_length = 1000;
    let min_bit_set_length = 2;
    let lead_max_doc = max_bit_set_length + 1;
    let mut bit_sets = Vec::new();
    for _ in 0..num_bit_set_iterators {
        let bit_set_length = TestUtil::next_int(&*random(), min_bit_set_length, max_bit_set_length);
        let mut bit_set = FixedBitSet::new(bit_set_length as usize);
        bit_set.set_range(0, bit_set_length as usize - 1);
        bit_sets.push(bit_doc_id_set_iterator(&bit_set)?);
    }
    iterators.extend(bit_sets);
    // `iterators[iterators.length - 1]`: the lead is the last element.
    iterators.push(Box::new(range(lead_max_doc, lead_max_doc + 1)?));

    let mut conjunction = into_disi(ConjunctionUtils::intersect_iterators(iterators)?);
    assert_equals(&NO_MORE_DOCS, &conjunction.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &conjunction.doc_id());
    Ok(())
}
