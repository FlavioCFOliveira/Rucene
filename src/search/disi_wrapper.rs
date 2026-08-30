//! Iterator wrappers, ported from `org.apache.lucene.search.DisiWrapper`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorable::Scorable;
use crate::search::scorer::Scorer;
use crate::search::scorer_util::ScorerUtil;
use crate::search::two_phase_iterator::TwoPhaseIterator;

/// Message used where a two-phase view is known to be present because it was
/// observed once at construction time and a scorer must return the same view on
/// every call.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: two_phase was observed at construction and a Scorer returns a stable view";

/// Wrapper used in [`DisiPriorityQueue`](crate::search::DisiPriorityQueue).
///
/// Equivalent to `org.apache.lucene.search.DisiWrapper`.
///
/// **Divergence from Lucene 10.5.0.** Java stores five aliases of the same
/// object side by side — `scorer`, `scorable`, `iterator`, `approximation` and
/// `twoPhaseView` — because a Java field can hold a second reference to a live
/// object. Rust forbids that aliasing, so this port owns the [`Scorer`] and
/// derives the four views on demand through [`scorable`](Self::scorable),
/// [`iterator`](Self::iterator), [`approximation`](Self::approximation) and
/// [`two_phase_view`](Self::two_phase_view). Every call reaches exactly the
/// object Java's corresponding field pointed at.
///
/// Java's `postingsEnum` field, which holds the iterator when it happens to be
/// a `PostingsEnum` and `null` otherwise, has no counterpart: it only exists so
/// that HotSpot sees a monomorphic receiver, and nothing in Lucene 10.5.0 reads
/// it.
pub struct DisiWrapper {
    scorer: Box<dyn Scorer>,
    two_phase: bool,

    /// The cost of the wrapped iterator, read once at construction.
    ///
    /// Equivalent to the `public final long cost` field.
    pub cost: i64,

    /// The match cost for two-phase iterators, `0` otherwise.
    ///
    /// Equivalent to the `public final float matchCost` field.
    pub match_cost: f32,

    /// The current doc, used for comparison.
    ///
    /// Equivalent to the `public int doc` field.
    pub doc: i32,

    /// Scaled maximum score, used by
    /// [`WANDScorer`](crate::search::WANDScorer).
    ///
    /// Equivalent to the package-private `long scaledMaxScore` field.
    pub scaled_max_score: i64,

    /// Maximum score over the current window, used by
    /// [`MaxScoreBulkScorer`](crate::search::MaxScoreBulkScorer).
    ///
    /// Equivalent to the package-private `float maxWindowScore` field.
    pub max_window_score: f32,

    /// The per-clause weight, used by `CombinedFieldQuery` (BM25F).
    ///
    /// Equivalent to the package-private `final float weight` field.
    pub weight: f32,
}

impl std::fmt::Debug for DisiWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisiWrapper")
            .field("cost", &self.cost)
            .field("match_cost", &self.match_cost)
            .field("doc", &self.doc)
            .field("two_phase", &self.two_phase)
            .finish_non_exhaustive()
    }
}

impl DisiWrapper {
    /// Wraps a scorer, with a weight of `1`.
    ///
    /// Equivalent to `new DisiWrapper(Scorer, boolean)`. Pass `impacts = true`
    /// when the wrapped iterator is likely an
    /// [`ImpactsEnum`](crate::index::ImpactsEnum).
    pub fn new(scorer: Box<dyn Scorer>, impacts: bool) -> Self {
        Self::with_weight(scorer, impacts, 1.0)
    }

    /// Wraps a scorer with the given per-clause weight.
    ///
    /// Equivalent to the package-private
    /// `DisiWrapper(Scorer, boolean, float)`.
    pub fn with_weight(mut scorer: Box<dyn Scorer>, impacts: bool, weight: f32) -> Self {
        let two_phase = scorer.two_phase_iterator().is_some();
        let (cost, match_cost) = if two_phase {
            let view = scorer.two_phase_iterator().expect(TWO_PHASE_INVARIANT);
            let match_cost = view.match_cost();
            // Java reads the cost from `iterator`, which is the scorer's own
            // iterator regardless of two-phase support.
            (scorer.iterator().cost(), match_cost)
        } else {
            (scorer.iterator().cost(), 0.0)
        };
        // Kept for parity with Java, which routes the iterator through
        // `ScorerUtil.likelyImpactsEnum` when `impacts` is set; see that
        // method for why this port returns its argument unchanged.
        let _ = impacts;
        Self {
            scorer,
            two_phase,
            cost,
            match_cost,
            doc: -1,
            scaled_max_score: 0,
            max_window_score: 0.0,
            weight,
        }
    }

    /// Returns the wrapped scorer.
    ///
    /// Equivalent to reading the `public final Scorer scorer` field.
    pub fn scorer(&mut self) -> &mut dyn Scorer {
        &mut *self.scorer
    }

    /// Returns the wrapped scorer viewed as a [`Scorable`].
    ///
    /// Equivalent to reading the `public final Scorable scorable` field, which
    /// Java initialises with `ScorerUtil.likelyTermScorer(scorer)`.
    pub fn scorable(&mut self) -> &mut dyn Scorable {
        ScorerUtil::likely_term_scorer(self.scorer.as_scorable())
    }

    /// Returns the iterator over matching documents.
    ///
    /// Equivalent to reading the `public final DocIdSetIterator iterator`
    /// field.
    pub fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.scorer.iterator()
    }

    /// Returns the approximation: the two-phase approximation when the scorer
    /// supports two-phase iteration, the iterator itself otherwise.
    ///
    /// Equivalent to reading the `public final DocIdSetIterator approximation`
    /// field.
    pub fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.two_phase {
            self.scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation()
        } else {
            self.scorer.iterator()
        }
    }

    /// Returns the current doc ID of the approximation.
    ///
    /// The shared-borrow companion of [`approximation`](Self::approximation);
    /// Java reads `approximation.docID()` directly.
    pub fn approximation_doc_id(&self) -> i32 {
        self.scorer.doc_id()
    }

    /// Returns the two-phase view of the scorer, or `None` when the scorer does
    /// not support two-phase iteration.
    ///
    /// Equivalent to reading the `public final TwoPhaseIterator twoPhaseView`
    /// field.
    pub fn two_phase_view(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        if self.two_phase {
            self.scorer.two_phase_iterator()
        } else {
            None
        }
    }

    /// Returns whether the wrapped scorer supports two-phase iteration, that
    /// is, whether Java's `twoPhaseView` field is non-`null`.
    pub fn has_two_phase(&self) -> bool {
        self.two_phase
    }

    /// Confirms the current document through the two-phase view, or returns
    /// `true` when the scorer has none.
    ///
    /// The shape `w.twoPhaseView == null || w.twoPhaseView.matches()` takes in
    /// Java, spelled once here because the borrow has to be re-taken.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while confirming the match.
    pub fn matches(&mut self) -> Result<bool> {
        match self.two_phase_view() {
            None => Ok(true),
            Some(view) => view.matches(),
        }
    }

    /// Unwraps this wrapper, returning the scorer it was built from.
    pub fn into_scorer(self) -> Box<dyn Scorer> {
        self.scorer
    }
}
