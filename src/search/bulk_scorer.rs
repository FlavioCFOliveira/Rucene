//! Bulk scoring, ported from `org.apache.lucene.search.BulkScorer` and
//! `org.apache.lucene.search.Weight.DefaultBulkScorer`.
//!
//! [`DefaultBulkScorer`] is a `protected static` nested class of `Weight` in
//! Java. It lives here rather than in [`crate::search::weight`] so that
//! [`ScorerSupplier::bulk_scorer`](crate::search::ScorerSupplier::bulk_scorer),
//! which is where Lucene builds it, does not have to depend on the weight
//! module. This is a placement divergence only; the class and its behaviour are
//! unchanged.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorer::Scorer;
use crate::util::Bits;

/// Scores a range of documents at once.
///
/// Equivalent to `org.apache.lucene.search.BulkScorer`, returned by
/// [`Weight::bulk_scorer`](crate::search::Weight::bulk_scorer). Only queries
/// that have a more optimised means of scoring across a range of documents need
/// to implement it directly; otherwise [`DefaultBulkScorer`] wraps the
/// [`Scorer`] the weight produces.
pub trait BulkScorer {
    /// Collects matching documents in a range and returns an estimation of the
    /// next matching document on or after `max`.
    ///
    /// Equivalent to `BulkScorer.score(LeafCollector, Bits, int, int)`. The
    /// return value must be:
    ///
    /// * `>= max`;
    /// * [`NO_MORE_DOCS`](crate::search::doc_id_set_iterator::NO_MORE_DOCS) if
    ///   there are no more matches;
    /// * `<=` the first matching document that is `>= max` otherwise.
    ///
    /// `min` is the minimum document to be considered for matching; all
    /// documents strictly before it must be ignored. Although `max` would be a
    /// legal return value, higher values may help callers skip more efficiently
    /// over non-matching portions of the doc ID space.
    ///
    /// * `collector` — the collector all matching documents are passed to;
    /// * `accept_docs` — the documents allowed to match, or `None` if they are
    ///   all allowed;
    /// * `min` — score starting at, and including, this document;
    /// * `max` — score up to, but not including, this document.
    ///
    /// # Errors
    ///
    /// Returns
    /// [`CollectionTerminated`](crate::search::CollectionError::CollectionTerminated)
    /// when the collector ends collection of this leaf early, and propagates
    /// any I/O error otherwise.
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32>;

    /// Returns an estimate of the number of documents this bulk scorer will
    /// visit.
    ///
    /// Equivalent to `BulkScorer.cost()`, the bulk-scorer analogue of
    /// [`DocIdSetIterator::cost`].
    fn cost(&self) -> i64;
}

/// Wraps a [`Scorer`] and performs top scoring with it.
///
/// Equivalent to `org.apache.lucene.search.Weight.DefaultBulkScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java caches the driving iterator — the
/// scorer's own iterator, or the approximation of its two-phase iterator — in a
/// field alongside the scorer. Rust cannot hold that borrow while the scorer is
/// also passed to the collector, so this port records *which* of the two to
/// drive in a flag and re-borrows on each step. The sequence of calls to the
/// iterator, the two-phase confirmation and the collector is unchanged,
/// including Lucene's four specialised loops.
pub struct DefaultBulkScorer {
    scorer: Box<dyn Scorer>,
    two_phase: bool,
    cost: i64,
}

impl std::fmt::Debug for DefaultBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultBulkScorer")
            .field("two_phase", &self.two_phase)
            .field("cost", &self.cost)
            .finish()
    }
}

/// Message used where the two-phase iterator is known to be present because it
/// was observed once at construction time and a scorer must return the same
/// view on every call.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: two_phase was observed at construction and a Scorer returns a stable view";

impl DefaultBulkScorer {
    /// Wraps the given scorer.
    ///
    /// Equivalent to `new DefaultBulkScorer(Scorer)`.
    pub fn new(mut scorer: Box<dyn Scorer>) -> Self {
        let two_phase = scorer.two_phase_iterator().is_some();
        let cost = if two_phase {
            scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation_ref()
                .cost()
        } else {
            scorer.iterator().cost()
        };
        Self {
            scorer,
            two_phase,
            cost,
        }
    }

    /// The doc ID of the iterator that drives collection: the approximation
    /// when this scorer is two-phase, the scorer's own iterator otherwise.
    fn lead_doc_id(&mut self) -> i32 {
        if self.two_phase {
            self.scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation_ref()
                .doc_id()
        } else {
            self.scorer.iterator().doc_id()
        }
    }

    fn lead_next_doc(&mut self) -> Result<i32> {
        if self.two_phase {
            self.scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation()
                .next_doc()
        } else {
            self.scorer.iterator().next_doc()
        }
    }

    fn lead_advance(&mut self, target: i32) -> Result<i32> {
        if self.two_phase {
            self.scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation()
                .advance(target)
        } else {
            self.scorer.iterator().advance(target)
        }
    }

    fn matches(&mut self) -> Result<bool> {
        self.scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .matches()
    }

    fn accepted(accept_docs: Option<&dyn Bits>, doc: i32) -> bool {
        accept_docs.map_or(true, |bits| bits.get(doc as usize))
    }

    /// Optimises simple iterators with collectors that cannot skip.
    ///
    /// Equivalent to `DefaultBulkScorer.scoreIterator`.
    fn score_iterator(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        max: i32,
    ) -> CollectionResult<()> {
        let mut doc = self.lead_doc_id();
        while doc < max {
            if Self::accepted(accept_docs, doc) {
                collector.collect(doc, self.scorer.as_scorable())?;
            }
            doc = self.lead_next_doc()?;
        }
        Ok(())
    }

    /// Equivalent to `DefaultBulkScorer.scoreTwoPhaseIterator`.
    fn score_two_phase_iterator(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        max: i32,
    ) -> CollectionResult<()> {
        let mut doc = self.lead_doc_id();
        while doc < max {
            if Self::accepted(accept_docs, doc) && self.matches()? {
                collector.collect(doc, self.scorer.as_scorable())?;
            }
            doc = self.lead_next_doc()?;
        }
        Ok(())
    }

    /// Equivalent to `DefaultBulkScorer.scoreCompetitiveIterator`.
    fn score_competitive_iterator(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        competitive_iterator: &mut dyn DocIdSetIterator,
        max: i32,
    ) -> CollectionResult<()> {
        let mut doc = self.lead_doc_id();
        while doc < max {
            debug_assert!(competitive_iterator.doc_id() <= doc); // invariant
            if competitive_iterator.doc_id() < doc {
                let competitive_next = competitive_iterator.advance(doc)?;
                if competitive_next != doc {
                    doc = self.lead_advance(competitive_next)?;
                    continue;
                }
            }

            if Self::accepted(accept_docs, doc) {
                collector.collect(doc, self.scorer.as_scorable())?;
            }

            doc = self.lead_next_doc()?;
        }
        Ok(())
    }

    /// Equivalent to `DefaultBulkScorer.scoreTwoPhaseOrCompetitiveIterator`.
    fn score_two_phase_or_competitive_iterator(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        competitive_iterator: &mut dyn DocIdSetIterator,
        max: i32,
    ) -> CollectionResult<()> {
        let mut doc = self.lead_doc_id();
        while doc < max {
            debug_assert!(competitive_iterator.doc_id() <= doc); // invariant
            if competitive_iterator.doc_id() < doc {
                let competitive_next = competitive_iterator.advance(doc)?;
                if competitive_next != doc {
                    doc = self.lead_advance(competitive_next)?;
                    continue;
                }
            }

            if Self::accepted(accept_docs, doc) && self.matches()? {
                collector.collect(doc, self.scorer.as_scorable())?;
            }

            doc = self.lead_next_doc()?;
        }
        Ok(())
    }
}

impl BulkScorer for DefaultBulkScorer {
    fn cost(&self) -> i64 {
        self.cost
    }

    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        collector.set_scorer(self.scorer.as_scorable())?;
        let mut competitive_iterator = collector.competitive_iterator()?;

        let mut min = min;
        if let Some(competitive) = competitive_iterator.as_ref() {
            if competitive.doc_id() > min {
                min = competitive.doc_id();
                // The competitive iterator may not match any docs in the range.
                min = min.min(max);
            }
        }

        if self.lead_doc_id() < min {
            if self.lead_doc_id() == min - 1 {
                self.lead_next_doc()?;
            } else {
                self.lead_advance(min)?;
            }
        }

        // These various specializations help save some null checks in a hot
        // loop, but as importantly if not more importantly, they help reduce
        // the polymorphism of call sites to next_doc() and collect(), because
        // only a subset of collectors produce a competitive iterator, and the
        // set of implementing types for two-phase approximations is smaller
        // than the set of doc id set iterator implementations.
        match (self.two_phase, competitive_iterator.as_mut()) {
            (false, None) => self.score_iterator(collector, accept_docs, max)?,
            (true, None) => self.score_two_phase_iterator(collector, accept_docs, max)?,
            (false, Some(competitive)) => {
                self.score_competitive_iterator(collector, accept_docs, &mut **competitive, max)?
            }
            (true, Some(competitive)) => self.score_two_phase_or_competitive_iterator(
                collector,
                accept_docs,
                &mut **competitive,
                max,
            )?,
        }

        Ok(self.lead_doc_id())
    }
}
