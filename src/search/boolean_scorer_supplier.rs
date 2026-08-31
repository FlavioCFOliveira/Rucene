//! Boolean scorer selection, ported from
//! `org.apache.lucene.search.BooleanScorerSupplier`.

#![deny(unsafe_code)]

use std::cell::Cell;

use crate::error::{LuceneError, Result};
use crate::search::block_max_conjunction_bulk_scorer::BlockMaxConjunctionBulkScorer;
use crate::search::block_max_conjunction_scorer::BlockMaxConjunctionScorer;
use crate::search::boolean_clause::Occur;
use crate::search::boolean_scorer::BooleanScorer;
use crate::search::bulk_scorer::{BulkScorer, DefaultBulkScorer};
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::conjunction_bulk_scorer::ConjunctionBulkScorer;
use crate::search::conjunction_scorer::ConjunctionScorer;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::dense_conjunction_bulk_scorer::{
    DenseConjunctionBulkScorer, DENSITY_THRESHOLD_INVERSE, WINDOW_SIZE,
};
use crate::search::disjunction_sum_scorer::DisjunctionSumScorer;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::doc_id_stream::DocIdStream;
use crate::search::max_score_bulk_scorer::MaxScoreBulkScorer;
use crate::search::req_excl_bulk_scorer::ReqExclBulkScorer;
use crate::search::req_excl_scorer::ReqExclScorer;
use crate::search::req_opt_sum_scorer::ReqOptSumScorer;
use crate::search::scorable::{Scorable, SimpleScorable};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::{into_scorer_iterator, Scorer, ScorerAsIterator};
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::scorer_util::ScorerUtil;
use crate::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};
use crate::search::wand_scorer::WANDScorer;
use crate::util::Bits;

/// The scorer suppliers of a boolean query, grouped by the way their clause
/// must occur.
///
/// Equivalent to the `Map<Occur, Collection<ScorerSupplier>>` that
/// `BooleanWeight.scorerSupplier` builds as an `EnumMap`; Rust has no enum map,
/// and the four groups are always all present, so they are four fields.
#[derive(Default)]
pub struct ClauseSuppliers {
    /// The suppliers of the [`Occur::SHOULD`] clauses.
    pub should: Vec<Box<dyn ScorerSupplier>>,
    /// The suppliers of the [`Occur::MUST`] clauses.
    pub must: Vec<Box<dyn ScorerSupplier>>,
    /// The suppliers of the [`Occur::FILTER`] clauses.
    pub filter: Vec<Box<dyn ScorerSupplier>>,
    /// The suppliers of the [`Occur::MUST_NOT`] clauses.
    pub must_not: Vec<Box<dyn ScorerSupplier>>,
}

impl std::fmt::Debug for ClauseSuppliers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClauseSuppliers")
            .field("should", &self.should.len())
            .field("must", &self.must.len())
            .field("filter", &self.filter.len())
            .field("must_not", &self.must_not.len())
            .finish()
    }
}

impl ClauseSuppliers {
    /// Returns the suppliers of the clauses that occur the given way.
    ///
    /// Equivalent to `subs.get(Occur)`.
    pub fn get(&self, occur: Occur) -> &Vec<Box<dyn ScorerSupplier>> {
        match occur {
            Occur::SHOULD => &self.should,
            Occur::MUST => &self.must,
            Occur::FILTER => &self.filter,
            Occur::MUST_NOT => &self.must_not,
        }
    }

    /// Returns the suppliers of the clauses that occur the given way, for
    /// mutation.
    pub fn get_mut(&mut self, occur: Occur) -> &mut Vec<Box<dyn ScorerSupplier>> {
        match occur {
            Occur::SHOULD => &mut self.should,
            Occur::MUST => &mut self.must,
            Occur::FILTER => &mut self.filter,
            Occur::MUST_NOT => &mut self.must_not,
        }
    }
}

/// A scorer that reports a score of zero for every document it matches.
///
/// Equivalent to the anonymous `FilterScorer` subclasses of
/// `BooleanScorerSupplier` that override `score()` and `getMaxScore(int)` to
/// return `0f`.
///
/// **Divergence from Lucene 10.5.0.** This port does not build on
/// [`FilterScorer`](crate::search::FilterScorer), because that type delegates
/// `setMinCompetitiveScore`, `advanceShallow`, `getChildren` and
/// `smoothingScore` to the wrapped scorer, where Java's `FilterScorer` leaves
/// them at `Scorer`'s defaults. Those defaults are load-bearing here: a
/// filter-only required clause must not let a minimum competitive score reach
/// the clause it hides the score of.
struct ZeroScoreScorer {
    inner: Box<dyn Scorer>,
}

impl std::fmt::Debug for ZeroScoreScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZeroScoreScorer")
    }
}

impl Scorable for ZeroScoreScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }
}

impl Scorer for ZeroScoreScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.inner.iterator()
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.inner.two_phase_iterator()
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(0.0)
    }
}

/// The collector a [`disable_scoring`] wrapper installs, which hands the
/// wrapped collector a scorable that always scores zero.
///
/// Equivalent to the anonymous `LeafCollector` in
/// `BooleanScorerSupplier.disableScoring`. As in Java it overrides `setScorer`,
/// `collect(int)` and `collect(DocIdStream)` only: `collectRange` and
/// `competitiveIterator()` keep [`LeafCollector`]'s defaults, so the wrapped
/// bulk scorer never sees a competitive iterator.
struct NoScoreLeafCollector<'a> {
    inner: &'a mut dyn LeafCollector,
    fake: SimpleScorable,
}

impl LeafCollector for NoScoreLeafCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        self.inner.set_scorer(&mut self.fake)
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.inner.collect(doc, &mut self.fake)
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        self.inner.collect_stream(stream, &mut self.fake)
    }
}

/// A [`BulkScorer`] that hides the scores of the one it wraps.
///
/// Equivalent to the anonymous `BulkScorer` that
/// `BooleanScorerSupplier.disableScoring(BulkScorer)` returns.
struct DisableScoringBulkScorer {
    inner: Box<dyn BulkScorer>,
}

impl BulkScorer for DisableScoringBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let mut no_score_collector = NoScoreLeafCollector {
            inner: collector,
            fake: SimpleScorable::new(),
        };
        self.inner
            .score(&mut no_score_collector, accept_docs, min, max)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

/// Wraps a bulk scorer so that the collector never sees its scores.
///
/// Equivalent to the static `BooleanScorerSupplier.disableScoring(BulkScorer)`.
fn disable_scoring(scorer: Box<dyn BulkScorer>) -> Box<dyn BulkScorer> {
    Box::new(DisableScoringBulkScorer { inner: scorer })
}

/// Builds a constant-score scorer of zero over another scorer's iteration.
///
/// Equivalent to
/// `scorer.twoPhaseIterator() != null
///     ? new ConstantScoreScorer(0f, scoreMode, scorer.twoPhaseIterator())
///     : new ConstantScoreScorer(0f, scoreMode, scorer.iterator())`.
fn constant_score_of_zero(scorer: Box<dyn Scorer>, score_mode: ScoreMode) -> Box<dyn Scorer> {
    match into_scorer_iterator(scorer) {
        ScorerIterator::Simple(iterator) => Box::new(ConstantScoreScorer::from_iterator(
            0.0, score_mode, iterator,
        )),
        ScorerIterator::TwoPhase(two_phase) => Box::new(ConstantScoreScorer::from_two_phase(
            0.0, score_mode, two_phase,
        )),
    }
}

/// The [`ScorerSupplier`] of a boolean query: the component that picks, out of
/// every scorer this package offers, the one that matches the shape of the
/// query.
///
/// Equivalent to the `final org.apache.lucene.search.BooleanScorerSupplier`,
/// which is package-private in Java; it is public here because Rust has no
/// package visibility and
/// [`BooleanWeight`](crate::search::BooleanWeight), which builds it, lives in a
/// sibling module.
///
/// **Divergence from Lucene 10.5.0.** Java's constructor also takes the
/// `Weight` that built it and never uses it; this port does not carry the dead
/// parameter.
pub struct BooleanScorerSupplier {
    subs: ClauseSuppliers,
    score_mode: ScoreMode,
    min_should_match: usize,
    max_doc: i32,
    /// Memoises [`ScorerSupplier::cost`]; `-1` means "not computed yet",
    /// exactly as Java's `long cost = -1` field does.
    cost: Cell<i64>,
    top_level_scoring_clause: bool,
}

impl std::fmt::Debug for BooleanScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BooleanScorerSupplier")
            .field("subs", &self.subs)
            .field("score_mode", &self.score_mode)
            .field("min_should_match", &self.min_should_match)
            .field("max_doc", &self.max_doc)
            .finish()
    }
}

/// Message used where the should-cost computation is known to be well-formed
/// because the constructor validated the minimum-should-match.
const SHOULD_COST_INVARIANT: &str =
    "INVARIANT: the constructor rejects a minimum-should-match above the number of SHOULD clauses";

impl BooleanScorerSupplier {
    /// Builds the supplier of a boolean query.
    ///
    /// Equivalent to `BooleanScorerSupplier(Weight, Map<Occur,
    /// Collection<ScorerSupplier>>, ScoreMode, int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for each of the four
    /// `IllegalArgumentException`s Java throws: a negative minimum-should-match,
    /// one that is not strictly below the number of `SHOULD` clauses, purely
    /// optional clauses when scores are not needed, and the absence of any
    /// positive clause.
    pub fn new(
        subs: ClauseSuppliers,
        score_mode: ScoreMode,
        min_should_match: i32,
        max_doc: i32,
    ) -> Result<Self> {
        if min_should_match < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "minShouldMatch must be positive, but got: {min_should_match}"
            )));
        }
        let min_should_match = min_should_match as usize;
        if min_should_match != 0 && min_should_match >= subs.should.len() {
            return Err(LuceneError::IllegalArgument(
                "minShouldMatch must be strictly less than the number of SHOULD clauses"
                    .to_string(),
            ));
        }
        if !score_mode.needs_scores()
            && min_should_match == 0
            && !subs.should.is_empty()
            && subs.must.len() + subs.filter.len() > 0
        {
            return Err(LuceneError::IllegalArgument(
                "Cannot pass purely optional clauses if scores are not needed".to_string(),
            ));
        }
        if subs.should.len() + subs.must.len() + subs.filter.len() == 0 {
            return Err(LuceneError::IllegalArgument(
                "There should be at least one positive clause".to_string(),
            ));
        }
        Ok(Self {
            subs,
            score_mode,
            min_should_match,
            max_doc,
            cost: Cell::new(-1),
            top_level_scoring_clause: false,
        })
    }

    /// Equivalent to the private `BooleanScorerSupplier.computeShouldCost()`.
    fn compute_should_cost(&self) -> i64 {
        let costs = self.subs.should.iter().map(|supplier| supplier.cost());
        ScorerUtil::cost_with_min_should_match(
            costs.collect::<Vec<_>>(),
            self.subs.should.len(),
            self.min_should_match,
        )
        .expect(SHOULD_COST_INVARIANT)
    }

    /// Equivalent to the private `BooleanScorerSupplier.computeCost()`.
    fn compute_cost(&self) -> i64 {
        let min_required_cost = self
            .subs
            .must
            .iter()
            .chain(self.subs.filter.iter())
            .map(|supplier| supplier.cost())
            .min();
        match min_required_cost {
            Some(min_required_cost) if self.min_should_match == 0 => min_required_cost,
            other => {
                let should_cost = self.compute_should_cost();
                other.unwrap_or(i64::MAX).min(should_cost)
            }
        }
    }

    /// Equivalent to the private `BooleanScorerSupplier.getInternal(long)`.
    fn get_internal(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        // three cases: conjunction, disjunction, or mix
        let lead_cost = lead_cost.min(self.cost());
        let score_mode = self.score_mode;
        let min_should_match = self.min_should_match;
        let top_level_scoring_clause = self.top_level_scoring_clause;
        let ClauseSuppliers {
            should,
            must,
            filter,
            must_not,
        } = &mut self.subs;

        // pure conjunction
        if should.is_empty() {
            let main = req(
                filter,
                must,
                lead_cost,
                top_level_scoring_clause,
                score_mode,
            )?;
            return excl(main, must_not, lead_cost);
        }

        // pure disjunction
        if filter.is_empty() && must.is_empty() {
            let main = opt(
                should,
                min_should_match,
                score_mode,
                lead_cost,
                top_level_scoring_clause,
            )?;
            return excl(main, must_not, lead_cost);
        }

        // conjunction-disjunction mix:
        // we create the required and optional pieces, and then combine the two:
        // if minNrShouldMatch > 0, then it's a conjunction, because the optional
        // side must match; otherwise it's required + optional.
        if min_should_match > 0 {
            let required = req(filter, must, lead_cost, false, score_mode)?;
            let required = excl(required, must_not, lead_cost)?;
            let optional = opt(should, min_should_match, score_mode, lead_cost, false)?;
            Ok(Box::new(ConjunctionScorer::new(
                vec![required, optional],
                vec![0, 1],
            )?))
        } else {
            debug_assert!(score_mode.needs_scores());
            let required = req(filter, must, lead_cost, false, score_mode)?;
            let required = excl(required, must_not, lead_cost)?;
            let optional = opt(should, min_should_match, score_mode, lead_cost, false)?;
            Ok(Box::new(ReqOptSumScorer::new(
                required, optional, score_mode,
            )?))
        }
    }

    /// Returns a [`BulkScorer`] specialised for this boolean query, or `None`
    /// when none applies and the generic scorer-driven path must be used.
    ///
    /// Equivalent to the package-private
    /// `BooleanScorerSupplier.booleanScorer()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the sub scorers.
    pub fn boolean_scorer(&mut self) -> Result<Option<Box<dyn BulkScorer>>> {
        let num_optional_clauses = self.subs.should.len();
        let num_must_clauses = self.subs.must.len();
        let num_required_clauses = num_must_clauses + self.subs.filter.len();

        let positive_scorer = if num_required_clauses == 0 {
            // TODO(lucene): what is the right heuristic here?
            let cost_threshold: i64 = if self.min_should_match <= 1 {
                // when all clauses are optional, use BooleanScorer aggressively
                -1
            } else {
                // when a minimum number of clauses should match, BooleanScorer
                // is going to score all windows that have at least
                // minNrShouldMatch matches in the window. But there is no way to
                // know if there is an intersection (all clauses might match a
                // different doc ID and there will be no matches in the end) so
                // we should only use BooleanScorer if matches are very dense.
                i64::from(self.max_doc / 3)
            };

            if self.cost() < cost_threshold {
                return Ok(None);
            }

            self.optional_bulk_scorer()?
        } else if num_must_clauses == 0 && num_optional_clauses > 1 && self.min_should_match >= 1 {
            self.filtered_optional_bulk_scorer()?
        } else if num_required_clauses > 0
            && num_optional_clauses == 0
            && self.min_should_match == 0
        {
            self.required_bulk_scorer()?
        } else {
            // TODO(lucene): there are some cases where BooleanScorer would
            // handle conjunctions faster than BooleanScorer2...
            return Ok(None);
        };

        let Some(positive_scorer) = positive_scorer else {
            return Ok(None);
        };
        let positive_scorer_cost = positive_scorer.cost();

        let mut prohibited = Vec::with_capacity(self.subs.must_not.len());
        for supplier in &mut self.subs.must_not {
            prohibited.push(supplier.get(positive_scorer_cost)?);
        }

        if prohibited.is_empty() {
            Ok(Some(positive_scorer))
        } else {
            let prohibited_scorer: Box<dyn Scorer> = if prohibited.len() == 1 {
                prohibited
                    .pop()
                    .expect("INVARIANT: the vector was just observed to hold one element")
            } else {
                Box::new(DisjunctionSumScorer::new(
                    prohibited,
                    ScoreMode::COMPLETE_NO_SCORES,
                    positive_scorer_cost,
                )?)
            };
            Ok(Some(Box::new(ReqExclBulkScorer::from_scorer(
                positive_scorer,
                prohibited_scorer,
            ))))
        }
    }

    /// Returns a bulk scorer for the optional clauses only, or `None` when it
    /// is not applicable.
    ///
    /// Equivalent to the package-private
    /// `BooleanScorerSupplier.optionalBulkScorer()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the sub scorers.
    pub fn optional_bulk_scorer(&mut self) -> Result<Option<Box<dyn BulkScorer>>> {
        if self.subs.should.is_empty() {
            return Ok(None);
        } else if self.subs.should.len() == 1 && self.min_should_match <= 1 {
            return Ok(Some(self.subs.should[0].bulk_scorer()?));
        }

        if self.score_mode == ScoreMode::TOP_SCORES {
            if self.min_should_match > 1 {
                // Fall back to BS2/WANDScorer: it supports both block-max impact
                // pruning and minShouldMatch > 1. BooleanScorer (the fall-through
                // below) does not consult score upper bounds and would score
                // every doc in the 4096-doc window, defeating top-K pruning.
                return Ok(None);
            }
            let mut optional_scorers = Vec::with_capacity(self.subs.should.len());
            for supplier in &mut self.subs.should {
                optional_scorers.push(supplier.get(i64::MAX)?);
            }

            return Ok(Some(Box::new(MaxScoreBulkScorer::new(
                self.max_doc,
                optional_scorers,
                None,
            )?)));
        }

        let should_cost = self.compute_should_cost();
        let mut optional = Vec::with_capacity(self.subs.should.len());
        for supplier in &mut self.subs.should {
            optional.push(supplier.get(should_cost)?);
        }

        Ok(Some(Box::new(BooleanScorer::new(
            optional,
            self.min_should_match.max(1),
            self.score_mode.needs_scores(),
        )?)))
    }

    /// Returns a bulk scorer for optional clauses gated by filters, or `None`
    /// when it is not applicable.
    ///
    /// Equivalent to the package-private
    /// `BooleanScorerSupplier.filteredOptionalBulkScorer()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the sub scorers.
    pub fn filtered_optional_bulk_scorer(&mut self) -> Result<Option<Box<dyn BulkScorer>>> {
        if !self.subs.must.is_empty()
            || self.subs.filter.is_empty()
            || (self.score_mode.needs_scores() && self.score_mode != ScoreMode::TOP_SCORES)
            || self.subs.should.len() <= 1
            || self.min_should_match != 1
        {
            return Ok(None);
        }
        let cost = self.cost();
        let mut optional_scorers = Vec::with_capacity(self.subs.should.len());
        for supplier in &mut self.subs.should {
            optional_scorers.push(supplier.get(cost)?);
        }
        let mut filters = Vec::with_capacity(self.subs.filter.len());
        for supplier in &mut self.subs.filter {
            filters.push(supplier.get(cost)?);
        }
        if self.score_mode == ScoreMode::TOP_SCORES {
            let filter_scorer: Box<dyn Scorer> = if filters.len() == 1 {
                filters
                    .pop()
                    .expect("INVARIANT: the vector was just observed to hold one element")
            } else {
                Box::new(ConjunctionScorer::new(filters, Vec::new())?)
            };
            Ok(Some(Box::new(MaxScoreBulkScorer::new(
                self.max_doc,
                optional_scorers,
                Some(filter_scorer),
            )?)))
        } else {
            // In the beginning of this method, we exited early if the score mode
            // is not either TOP_SCORES or a score mode that doesn't need scores.
            debug_assert!(!self.score_mode.needs_scores());
            filters.push(Box::new(DisjunctionSumScorer::new(
                optional_scorers,
                self.score_mode,
                cost,
            )?));

            if self.max_doc >= WINDOW_SIZE
                && cost >= i64::from(self.max_doc / DENSITY_THRESHOLD_INVERSE)
            {
                return Ok(Some(Box::new(DenseConjunctionBulkScorer::of(
                    filters,
                    self.max_doc,
                    0.0,
                )?)));
            }

            Ok(Some(Box::new(DefaultBulkScorer::new(Box::new(
                ConjunctionScorer::new(filters, Vec::new())?,
            )))))
        }
    }

    /// Returns a bulk scorer for the required clauses only, or `None` when it
    /// is not applicable.
    ///
    /// Equivalent to the private
    /// `BooleanScorerSupplier.requiredBulkScorer()`.
    fn required_bulk_scorer(&mut self) -> Result<Option<Box<dyn BulkScorer>>> {
        if self.subs.must.len() + self.subs.filter.len() == 0 {
            // No required clauses at all.
            return Ok(None);
        } else if self.subs.must.len() + self.subs.filter.len() == 1 {
            let scorer = if !self.subs.must.is_empty() {
                self.subs.must[0].bulk_scorer()?
            } else {
                let scorer = self.subs.filter[0].bulk_scorer()?;
                if self.score_mode.needs_scores() {
                    disable_scoring(scorer)
                } else {
                    scorer
                }
            };
            return Ok(Some(scorer));
        }

        let must_lead_cost = self
            .subs
            .must
            .iter()
            .map(|supplier| supplier.cost())
            .min()
            .unwrap_or(i64::MAX);
        let filter_lead_cost = self
            .subs
            .filter
            .iter()
            .map(|supplier| supplier.cost())
            .min()
            .unwrap_or(i64::MAX);
        let lead_cost = must_lead_cost.min(filter_lead_cost);

        let mut required_no_scoring = Vec::with_capacity(self.subs.filter.len());
        for supplier in &mut self.subs.filter {
            required_no_scoring.push(supplier.get(lead_cost)?);
        }
        let mut required_scoring = Vec::with_capacity(self.subs.must.len());
        let num_must = self.subs.must.len();
        for supplier in &mut self.subs.must {
            if num_must == 1 {
                supplier.set_top_level_scoring_clause()?;
            }
            required_scoring.push(supplier.get(lead_cost)?);
        }

        if self.score_mode == ScoreMode::TOP_SCORES
            && required_scoring.len() > 1
            // Only specialize top-level conjunctions for clauses that don't
            // have a two-phase iterator.
            && required_no_scoring
                .iter_mut()
                .all(|scorer| scorer.two_phase_iterator().is_none())
            && required_scoring
                .iter_mut()
                .all(|scorer| scorer.two_phase_iterator().is_none())
        {
            // Turn all filters into scoring clauses with a score of zero, so
            // that BlockMaxConjunctionBulkScorer is applicable.
            for filter in required_no_scoring {
                required_scoring.push(Box::new(ConstantScoreScorer::from_iterator(
                    0.0,
                    ScoreMode::COMPLETE,
                    Box::new(ScorerAsIterator::new(filter)),
                )));
            }
            return Ok(Some(Box::new(BlockMaxConjunctionBulkScorer::new(
                self.max_doc,
                required_scoring,
            )?)));
        }
        if self.score_mode != ScoreMode::TOP_SCORES
            && required_scoring.len() + required_no_scoring.len() >= 2
            && required_scoring
                .iter_mut()
                .all(|scorer| scorer.two_phase_iterator().is_none())
        {
            if required_scoring.is_empty()
                && self.max_doc >= WINDOW_SIZE
                && lead_cost >= i64::from(self.max_doc / DENSITY_THRESHOLD_INVERSE)
            {
                return Ok(Some(Box::new(DenseConjunctionBulkScorer::of(
                    required_no_scoring,
                    self.max_doc,
                    0.0,
                )?)));
            } else if required_no_scoring
                .iter_mut()
                .all(|scorer| scorer.two_phase_iterator().is_none())
            {
                return Ok(Some(Box::new(ConjunctionBulkScorer::new(
                    required_scoring,
                    required_no_scoring,
                )?)));
            }
        }
        if self.score_mode == ScoreMode::TOP_SCORES && required_scoring.len() > 1 {
            required_scoring = vec![Box::new(BlockMaxConjunctionScorer::new(required_scoring)?)];
        }
        let conjunction_scorer: Box<dyn Scorer>;
        if required_no_scoring.len() + required_scoring.len() == 1 {
            if required_scoring.len() == 1 {
                conjunction_scorer = required_scoring
                    .pop()
                    .expect("INVARIANT: the vector was just observed to hold one element");
            } else {
                let inner = required_no_scoring
                    .pop()
                    .expect("INVARIANT: the vector was just observed to hold one element");
                conjunction_scorer = if self.score_mode.needs_scores() {
                    Box::new(ZeroScoreScorer { inner })
                } else {
                    inner
                };
            }
        } else {
            let scoring_positions: Vec<usize> = (0..required_scoring.len()).collect();
            let mut required = required_scoring;
            required.extend(required_no_scoring);
            let scorer = ConjunctionScorer::new(required, scoring_positions.clone())?;
            conjunction_scorer =
                if self.score_mode == ScoreMode::TOP_SCORES && scoring_positions.is_empty() {
                    constant_score_of_zero(Box::new(scorer), self.score_mode)
                } else {
                    Box::new(scorer)
                };
        }
        Ok(Some(Box::new(DefaultBulkScorer::new(conjunction_scorer))))
    }
}

/// Creates a new scorer for the given required clauses.
///
/// Equivalent to the private `BooleanScorerSupplier.req`. `required_scoring` is
/// the subset of the required clauses that should participate in scoring.
///
/// # Errors
///
/// Propagates any I/O error raised while building the sub scorers.
fn req(
    required_no_scoring: &mut [Box<dyn ScorerSupplier>],
    required_scoring: &mut [Box<dyn ScorerSupplier>],
    lead_cost: i64,
    top_level_scoring_clause: bool,
    score_mode: ScoreMode,
) -> Result<Box<dyn Scorer>> {
    if required_no_scoring.len() + required_scoring.len() == 1 {
        let requirement = if required_no_scoring.is_empty() {
            required_scoring[0].get(lead_cost)?
        } else {
            required_no_scoring[0].get(lead_cost)?
        };

        if !score_mode.needs_scores() {
            return Ok(requirement);
        }

        if required_scoring.is_empty() {
            // Scores are needed but we only have a filter clause. BooleanWeight
            // expects that calling score() is ok so we need to wrap to prevent
            // score() from being propagated.
            return Ok(Box::new(ZeroScoreScorer { inner: requirement }));
        }

        return Ok(requirement);
    }

    let mut required_scorers = Vec::with_capacity(required_no_scoring.len());
    for supplier in required_no_scoring.iter_mut() {
        required_scorers.push(supplier.get(lead_cost)?);
    }
    let mut scoring_scorers = Vec::with_capacity(required_scoring.len());
    for supplier in required_scoring.iter_mut() {
        scoring_scorers.push(supplier.get(lead_cost)?);
    }
    if score_mode == ScoreMode::TOP_SCORES && scoring_scorers.len() > 1 && top_level_scoring_clause
    {
        let block_max_scorer = BlockMaxConjunctionScorer::new(scoring_scorers)?;
        if required_scorers.is_empty() {
            return Ok(Box::new(block_max_scorer));
        }
        scoring_scorers = vec![Box::new(block_max_scorer)];
    }
    // The scoring clauses come after the non-scoring ones, which is the order
    // Java's `requiredScorers.addAll(scoringScorers)` produces.
    let scoring_positions: Vec<usize> =
        (required_scorers.len()..required_scorers.len() + scoring_scorers.len()).collect();
    required_scorers.extend(scoring_scorers);
    Ok(Box::new(ConjunctionScorer::new(
        required_scorers,
        scoring_positions,
    )?))
}

/// Excludes the prohibited clauses from `main`.
///
/// Equivalent to the private `BooleanScorerSupplier.excl`.
///
/// # Errors
///
/// Propagates any I/O error raised while building the sub scorers.
fn excl(
    main: Box<dyn Scorer>,
    prohibited: &mut [Box<dyn ScorerSupplier>],
    lead_cost: i64,
) -> Result<Box<dyn Scorer>> {
    if prohibited.is_empty() {
        Ok(main)
    } else {
        let excluded = opt(
            prohibited,
            1,
            ScoreMode::COMPLETE_NO_SCORES,
            lead_cost,
            false,
        )?;
        Ok(Box::new(ReqExclScorer::new(main, excluded)))
    }
}

/// Creates a new scorer for the given optional clauses.
///
/// Equivalent to the private `BooleanScorerSupplier.opt`.
///
/// # Errors
///
/// Propagates any I/O error raised while building the sub scorers.
fn opt(
    optional: &mut [Box<dyn ScorerSupplier>],
    min_should_match: usize,
    score_mode: ScoreMode,
    lead_cost: i64,
    top_level_scoring_clause: bool,
) -> Result<Box<dyn Scorer>> {
    if optional.len() == 1 {
        return optional[0].get(lead_cost);
    }
    let mut optional_scorers = Vec::with_capacity(optional.len());
    for supplier in optional.iter_mut() {
        optional_scorers.push(supplier.get(lead_cost)?);
    }

    // Technically speaking, WANDScorer should be able to handle the following 3
    // conditions now:
    // 1. Any ScoreMode (with scoring or not)
    // 2. Any minCompetitiveScore ( >= 0 )
    // 3. Any minShouldMatch ( >= 0 )
    //
    // However, as WANDScorer uses a more complex algorithm and data structure,
    // we would like to still use DisjunctionSumScorer to handle exhaustive pure
    // disjunctions, which may be faster.
    if (score_mode == ScoreMode::TOP_SCORES && top_level_scoring_clause) || min_should_match > 1 {
        Ok(Box::new(WANDScorer::new(
            optional_scorers,
            min_should_match,
            score_mode,
            lead_cost,
        )?))
    } else {
        Ok(Box::new(DisjunctionSumScorer::new(
            optional_scorers,
            score_mode,
            lead_cost,
        )?))
    }
}

impl ScorerSupplier for BooleanScorerSupplier {
    fn cost(&self) -> i64 {
        if self.cost.get() == -1 {
            self.cost.set(self.compute_cost());
        }
        self.cost.get()
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        self.top_level_scoring_clause = true;
        if self.subs.should.len() + self.subs.must.len() == 1 {
            // If there is a single scoring clause, propagate the call.
            for supplier in &mut self.subs.should {
                supplier.set_top_level_scoring_clause()?;
            }
            for supplier in &mut self.subs.must {
                supplier.set_top_level_scoring_clause()?;
            }
        }
        Ok(())
    }

    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let scorer = self.get_internal(lead_cost)?;
        if self.score_mode == ScoreMode::TOP_SCORES
            && self.subs.should.is_empty()
            && self.subs.must.is_empty()
        {
            // no scoring clauses but scores are needed so we wrap the scorer in
            // a constant score in order to allow early termination
            return Ok(constant_score_of_zero(scorer, self.score_mode));
        }
        Ok(scorer)
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        match self.boolean_scorer()? {
            // bulk scoring is applicable, use it
            Some(bulk_scorer) => Ok(bulk_scorer),
            // use a Scorer-based impl (BS2)
            None => Ok(Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?))),
        }
    }
}
