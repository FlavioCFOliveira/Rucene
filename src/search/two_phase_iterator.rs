//! Two-phase iteration, ported from
//! `org.apache.lucene.search.TwoPhaseIterator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::FixedBitSet;

/// Exposes an approximation of a [`DocIdSetIterator`], returned by
/// [`Scorer::two_phase_iterator`](crate::search::Scorer::two_phase_iterator).
///
/// Equivalent to `org.apache.lucene.search.TwoPhaseIterator`. When the
/// [`approximation`](Self::approximation)'s
/// [`next_doc`](DocIdSetIterator::next_doc) or
/// [`advance`](DocIdSetIterator::advance) return, [`matches`](Self::matches)
/// must be checked in order to know whether the returned doc ID actually
/// matches.
///
/// **Divergence from Lucene 10.5.0.** Java stores the approximation in a
/// `protected final` field and exposes it through a single `approximation()`
/// accessor that callers use both to read and to advance. Rust needs the two
/// uses kept apart, so this trait declares the shared-borrow
/// [`approximation_ref`](Self::approximation_ref) beside the exclusive-borrow
/// [`approximation`](Self::approximation). Both must return the same object.
pub trait TwoPhaseIterator {
    /// Returns the approximation for iteration.
    ///
    /// Equivalent to `TwoPhaseIterator.approximation()`. The returned iterator
    /// is a superset of the matching documents, and each match needs to be
    /// confirmed with [`matches`](Self::matches) in order to know whether it
    /// matches or not.
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator;

    /// Returns the approximation for inspection.
    ///
    /// The shared-borrow sibling of [`approximation`](Self::approximation); see
    /// the trait documentation for why it exists.
    fn approximation_ref(&self) -> &dyn DocIdSetIterator;

    /// Returns whether the current doc ID that the approximation is on matches.
    ///
    /// Equivalent to `TwoPhaseIterator.matches()`. This method should only be
    /// called when the iterator is positioned — that is, not when
    /// [`DocIdSetIterator::doc_id`] is `-1` or [`NO_MORE_DOCS`] — and at most
    /// once per position.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while confirming the match.
    fn matches(&mut self) -> Result<bool>;

    /// An estimate of the expected cost to determine that a single document
    /// [`matches`](Self::matches).
    ///
    /// Equivalent to `TwoPhaseIterator.matchCost()`. This can be called before
    /// iterating the documents of the approximation. The returned value is an
    /// expected cost in number of simple operations — addition, multiplication,
    /// comparing two numbers, indexing an array — and must be positive.
    fn match_cost(&self) -> f32;

    /// Returns the end of the run of consecutive doc IDs that match this
    /// iterator and that contains the current doc ID of the approximation, that
    /// is: one plus the last doc ID of the run.
    ///
    /// Equivalent to `TwoPhaseIterator.docIDRunEnd()`, whose default returns
    /// the current doc ID of the approximation.
    ///
    /// It is illegal to call this method when the approximation is exhausted or
    /// not positioned.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while computing the run end.
    fn doc_id_run_end(&mut self) -> Result<i32> {
        Ok(self.approximation_ref().doc_id())
    }

    /// Loads the doc IDs that both belong to the approximation and
    /// [`match`](Self::matches), and are in
    /// `[approximation().doc_id(), up_to)`, into `bit_set`, the document whose
    /// ID is `i` being stored at bit `i - offset`.
    ///
    /// Equivalent to `TwoPhaseIterator.intoBitSet(int, FixedBitSet, int)`. Upon
    /// return the approximation is positioned on the first doc that is
    /// `>= up_to`, mirroring [`DocIdSetIterator::into_bit_set`].
    ///
    /// The default implementation walks the approximation and confirms each doc
    /// with [`matches`](Self::matches) — functionally identical to leap-frog
    /// evaluation, just writing into a bit set. Implementations that can
    /// confirm matches in bulk should override it.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while iterating or confirming.
    // Lucene's name; the method fills a caller-supplied bit set rather than
    // consuming the iterator, so it takes `&mut self`, exactly as
    // `DocIdSetIterator::into_bit_set` does.
    #[allow(clippy::wrong_self_convention)]
    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        let mut doc = self.approximation_ref().doc_id();
        while doc < up_to {
            if self.matches()? {
                bit_set.set((doc - offset) as usize);
            }
            doc = self.approximation().next_doc()?;
        }
        Ok(())
    }
}

impl<T: TwoPhaseIterator + ?Sized> TwoPhaseIterator for Box<T> {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        (**self).approximation()
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        (**self).approximation_ref()
    }

    fn matches(&mut self) -> Result<bool> {
        (**self).matches()
    }

    fn match_cost(&self) -> f32 {
        (**self).match_cost()
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        (**self).doc_id_run_end()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        (**self).into_bit_set(up_to, bit_set, offset)
    }
}

/// A [`DocIdSetIterator`] view of a [`TwoPhaseIterator`].
///
/// Equivalent to the package-private
/// `TwoPhaseIterator.TwoPhaseIteratorAsDocIdSetIterator`, created by
/// `TwoPhaseIterator.asDocIdSetIterator(TwoPhaseIterator)`. Iterating it
/// advances the approximation and confirms every candidate, so only true
/// matches are returned.
///
/// **Divergence from Lucene 10.5.0.** Java's static
/// `TwoPhaseIterator.unwrap(DocIdSetIterator)` recovers the wrapped
/// two-phase iterator by an `instanceof` test on an erased iterator. Rust has
/// no such downcast on `dyn DocIdSetIterator`, so this port keeps the wrapped
/// iterator reachable through [`two_phase`](Self::two_phase) /
/// [`two_phase_mut`](Self::two_phase_mut) on the concrete wrapper, and callers
/// that need to keep the two shapes apart use [`ScorerIterator`].
pub struct TwoPhaseIteratorAsDocIdSetIterator<T: TwoPhaseIterator + ?Sized> {
    two_phase: Box<T>,
}

impl<T: TwoPhaseIterator + ?Sized> TwoPhaseIteratorAsDocIdSetIterator<T> {
    /// Wraps the given two-phase iterator into a confirming
    /// [`DocIdSetIterator`].
    ///
    /// Equivalent to `TwoPhaseIterator.asDocIdSetIterator(TwoPhaseIterator)`.
    pub fn new(two_phase: Box<T>) -> Self {
        Self { two_phase }
    }

    /// Returns the wrapped two-phase iterator.
    ///
    /// The owning equivalent of `TwoPhaseIterator.unwrap(DocIdSetIterator)`.
    pub fn two_phase(&self) -> &T {
        &self.two_phase
    }

    /// Returns the wrapped two-phase iterator for iteration.
    pub fn two_phase_mut(&mut self) -> &mut T {
        &mut self.two_phase
    }

    /// Unwraps this view, returning the two-phase iterator it was built from.
    pub fn into_two_phase(self) -> Box<T> {
        self.two_phase
    }

    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.two_phase.matches()? {
                return Ok(doc);
            }
            doc = self.two_phase.approximation().next_doc()?;
        }
    }
}

impl<T: TwoPhaseIterator + ?Sized> DocIdSetIterator for TwoPhaseIteratorAsDocIdSetIterator<T> {
    fn doc_id(&self) -> i32 {
        self.two_phase.approximation_ref().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.two_phase.approximation().next_doc()?;
        self.do_next(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.two_phase.approximation().advance(target)?;
        self.do_next(doc)
    }

    fn cost(&self) -> i64 {
        self.two_phase.approximation_ref().cost()
    }
}

/// The two shapes a [`ScorerSupplier`](crate::search::ScorerSupplier) may
/// produce for iteration: a plain iterator, or a two-phase iterator whose
/// candidates still need confirming.
///
/// **Divergence from Lucene 10.5.0.** Java models this as a single
/// `DocIdSetIterator` that callers probe with
/// `TwoPhaseIterator.unwrap(DocIdSetIterator)`, an `instanceof` test on the
/// erased iterator. Rust cannot downcast a `dyn DocIdSetIterator`, so the two
/// shapes are kept apart in this enum instead of being erased and recovered.
/// The set of behaviours is exactly the same; only the way a caller learns
/// which shape it holds differs.
pub enum ScorerIterator {
    /// A plain iterator: every returned doc ID is a match.
    Simple(Box<dyn DocIdSetIterator>),
    /// An approximation whose candidates must be confirmed with
    /// [`TwoPhaseIterator::matches`].
    TwoPhase(Box<dyn TwoPhaseIterator>),
}

impl ScorerIterator {
    /// Returns an estimate of the number of documents this iterator might
    /// match, taken from the approximation in the two-phase case.
    ///
    /// Equivalent to `DocIdSetIterator.cost()` on the value Java would have
    /// returned from `ScorerSupplier`'s iterator method.
    pub fn cost(&self) -> i64 {
        match self {
            Self::Simple(it) => it.cost(),
            Self::TwoPhase(tp) => tp.approximation_ref().cost(),
        }
    }

    /// Collapses this value into a plain [`DocIdSetIterator`], wrapping the
    /// two-phase case with [`TwoPhaseIteratorAsDocIdSetIterator`] so that only
    /// confirmed matches are returned.
    ///
    /// Equivalent to what Java gets for free, because both shapes are already
    /// a `DocIdSetIterator` there.
    pub fn into_doc_id_set_iterator(self) -> Box<dyn DocIdSetIterator> {
        match self {
            Self::Simple(it) => it,
            Self::TwoPhase(tp) => Box::new(TwoPhaseIteratorAsDocIdSetIterator::new(tp)),
        }
    }
}

impl std::fmt::Debug for ScorerIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simple(_) => f.write_str("ScorerIterator::Simple"),
            Self::TwoPhase(_) => f.write_str("ScorerIterator::TwoPhase"),
        }
    }
}
