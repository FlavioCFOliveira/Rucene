//! Required-except-prohibited bulk scoring, ported from
//! `org.apache.lucene.search.ReqExclBulkScorer`.

#![deny(unsafe_code)]

use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::conjunction_disi::ConjunctionMember;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::util::Bits;

/// A [`BulkScorer`] that removes the matches of a prohibited clause from those
/// of a required one.
///
/// Equivalent to the `final class
/// org.apache.lucene.search.ReqExclBulkScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java's three constructors store the
/// prohibited clause as an approximation plus an optional two-phase view — the
/// same pair a [`ConjunctionMember`] holds — so this port keeps it in one, which
/// is also what lets the scorer, iterator and two-phase forms share the code.
pub struct ReqExclBulkScorer {
    req: Box<dyn BulkScorer>,
    excl: ConjunctionMember,
}

impl std::fmt::Debug for ReqExclBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqExclBulkScorer").finish_non_exhaustive()
    }
}

impl ReqExclBulkScorer {
    /// Removes the matches of a prohibited [`Scorer`].
    ///
    /// Equivalent to `ReqExclBulkScorer(BulkScorer, Scorer)`.
    pub fn from_scorer(req: Box<dyn BulkScorer>, excl: Box<dyn Scorer>) -> Self {
        Self {
            req,
            excl: ConjunctionMember::from_scorer(excl),
        }
    }

    /// Removes the matches of a prohibited [`DocIdSetIterator`].
    ///
    /// Equivalent to `ReqExclBulkScorer(BulkScorer, DocIdSetIterator)`.
    pub fn from_iterator(req: Box<dyn BulkScorer>, excl: Box<dyn DocIdSetIterator>) -> Self {
        Self {
            req,
            excl: ConjunctionMember::from_iterator(excl),
        }
    }

    /// Removes the matches of a prohibited [`TwoPhaseIterator`].
    ///
    /// Equivalent to `ReqExclBulkScorer(BulkScorer, TwoPhaseIterator)`.
    pub fn from_two_phase(req: Box<dyn BulkScorer>, excl: Box<dyn TwoPhaseIterator>) -> Self {
        Self {
            req,
            excl: ConjunctionMember::from_two_phase(excl),
        }
    }
}

impl BulkScorer for ReqExclBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let mut up_to = min;
        let mut excl_doc = self.excl.doc_id();

        while up_to < max {
            if excl_doc < up_to {
                excl_doc = self.excl.advance(up_to)?;
            }
            if excl_doc == up_to {
                if !self.excl.has_two_phase() {
                    // from up_to to doc_id_run_end() are excluded, so we scored
                    // up to doc_id_run_end()
                    up_to = self.excl.doc_id_run_end()?.min(max);
                } else if self.excl.matches()? {
                    // up_to is excluded so we can consider that we scored up to
                    // up_to + 1
                    up_to += 1;
                }
                excl_doc = self.excl.next_doc()?;
            } else {
                up_to = self
                    .req
                    .score(collector, accept_docs, up_to, excl_doc.min(max))?;
            }
        }

        if up_to == max {
            up_to = self.req.score(collector, accept_docs, up_to, up_to)?;
        }

        Ok(up_to)
    }

    fn cost(&self) -> i64 {
        self.req.cost()
    }
}
