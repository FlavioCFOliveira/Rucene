//! Conjunction builders, ported from
//! `org.apache.lucene.search.ConjunctionUtils`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::conjunction_disi::{self, ConjunctionMember};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};

/// Helper methods for building conjunction iterators.
///
/// Equivalent to the `final class org.apache.lucene.search.ConjunctionUtils`.
///
/// **Divergence from Lucene 10.5.0.** Java threads two parallel lists — the
/// iterators and the two-phase iterators — through the builders, because the
/// approximation of a two-phase iterator can be referenced from both. Rust
/// cannot own an approximation apart from its iterator, so the two lists become
/// one list of [`ConjunctionMember`]s, each of which carries its own
/// approximation. The clauses collected are the same ones.
#[derive(Debug, Clone, Copy)]
pub struct ConjunctionUtils;

impl ConjunctionUtils {
    /// Creates a conjunction over the provided [`Scorer`]s.
    ///
    /// Equivalent to `ConjunctionUtils.intersectScorers(Collection<Scorer>)`.
    /// The returned iterator may leverage two-phase iteration, in which case it
    /// is a [`ScorerIterator::TwoPhase`] — the shape Java's callers recover
    /// with `TwoPhaseIterator.unwrap`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when fewer than two scorers are
    /// supplied, or when they are not all on the same document.
    pub fn intersect_scorers(scorers: Vec<Box<dyn Scorer>>) -> Result<ScorerIterator> {
        if scorers.len() < 2 {
            return Err(LuceneError::IllegalArgument(
                "Cannot make a ConjunctionDISI of less than 2 iterators".to_string(),
            ));
        }
        let mut members = Vec::with_capacity(scorers.len());
        for scorer in scorers {
            Self::add_scorer(scorer, &mut members);
        }
        conjunction_disi::create_conjunction(members)
    }

    /// Creates a conjunction over the provided [`DocIdSetIterator`]s.
    ///
    /// Equivalent to
    /// `ConjunctionUtils.intersectIterators(List<? extends DocIdSetIterator>)`.
    ///
    /// # Errors
    ///
    /// As [`intersect_scorers`](Self::intersect_scorers).
    pub fn intersect_iterators(
        iterators: Vec<Box<dyn DocIdSetIterator>>,
    ) -> Result<ScorerIterator> {
        if iterators.len() < 2 {
            return Err(LuceneError::IllegalArgument(
                "Cannot make a ConjunctionDISI of less than 2 iterators".to_string(),
            ));
        }
        let mut members = Vec::with_capacity(iterators.len());
        for iterator in iterators {
            Self::add_iterator(iterator, &mut members);
        }
        conjunction_disi::create_conjunction(members)
    }

    /// Creates a conjunction over the provided clauses.
    ///
    /// Equivalent to
    /// `ConjunctionUtils.createConjunction(List<DocIdSetIterator>,
    /// List<TwoPhaseIterator>)`.
    ///
    /// # Errors
    ///
    /// As [`intersect_scorers`](Self::intersect_scorers).
    pub fn create_conjunction(members: Vec<ConjunctionMember>) -> Result<ScorerIterator> {
        conjunction_disi::create_conjunction(members)
    }

    /// Adds a scorer, splitting it into an approximation and a two-phase view
    /// when it supports two-phase iteration.
    ///
    /// Equivalent to the package-private `ConjunctionDISI.addScorer(Scorer,
    /// List<DocIdSetIterator>, List<TwoPhaseIterator>)`.
    pub fn add_scorer(scorer: Box<dyn Scorer>, members: &mut Vec<ConjunctionMember>) {
        members.push(ConjunctionMember::from_scorer(scorer));
    }

    /// Adds an iterator.
    ///
    /// Equivalent to `ConjunctionUtils.addIterator(DocIdSetIterator,
    /// List<DocIdSetIterator>, List<TwoPhaseIterator>)`. Java additionally
    /// flattens nested conjunctions and unwraps two-phase iterators hidden
    /// behind an erased iterator; see the
    /// [`conjunction_disi`](crate::search::conjunction_disi) module for why
    /// this port cannot.
    pub fn add_iterator(iterator: Box<dyn DocIdSetIterator>, members: &mut Vec<ConjunctionMember>) {
        members.push(ConjunctionMember::from_iterator(iterator));
    }

    /// Adds a two-phase iterator.
    ///
    /// Equivalent to
    /// `ConjunctionUtils.addTwoPhaseIterator(TwoPhaseIterator,
    /// List<DocIdSetIterator>, List<TwoPhaseIterator>)`.
    pub fn add_two_phase_iterator(
        two_phase: Box<dyn TwoPhaseIterator>,
        members: &mut Vec<ConjunctionMember>,
    ) {
        members.push(ConjunctionMember::from_two_phase(two_phase));
    }
}
