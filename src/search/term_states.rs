//! Per-segment term states, ported from
//! `org.apache.lucene.index.TermStates`.
//!
//! **Placement divergence from Lucene 10.5.0.** This type belongs to
//! `org.apache.lucene.index`, and [`crate::index::TermStates`] is a partial
//! port of it: it carries the state array and the statistics, but not the
//! top-level-context identity, the deferred term, `build`, `wasBuiltFor` or the
//! per-leaf `get`. Those are exactly what the term, phrase and multi-term
//! queries need, so the complete port lives here, in the only package this
//! batch owns. The two should be merged — this file moved to `crate::index` and
//! the partial type deleted — as a follow-up.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{EmptyTerms, IndexReaderContext, LeafReaderContext, Term, TermState, Terms};
use crate::search::index_searcher::IndexSearcher;

/// Maintains a [`TermState`] view over the leaves of an index reader, for a
/// single term.
///
/// Equivalent to the `final org.apache.lucene.index.TermStates`. It does not
/// track whether the given [`TermState`] objects are valid, nor whether they
/// refer to the same term in the associated readers.
#[derive(Debug)]
pub struct TermStates {
    /// Important: do **not** keep hard references to index readers. Java stores
    /// `context.identity`, an `Object` used only for reference comparison; this
    /// port stores [`IndexReaderContext::id`], which is the same identity value
    /// and likewise does not reference the reader.
    top_reader_context_identity: usize,
    states: Vec<Option<Box<dyn TermState>>>,
    /// `None` if the statistics are to be used.
    term: Option<Term>,
    doc_freq: i32,
    total_term_freq: i64,
}

impl TermStates {
    fn with_term(term: Option<Term>, context: &Arc<dyn IndexReaderContext>) -> Result<Self> {
        if !context.is_top_level() {
            return Err(LuceneError::IllegalArgument(
                "TermStates must be built from a top-level IndexReaderContext".to_string(),
            ));
        }
        let num_leaves = Arc::clone(context).leaves().len();
        Ok(Self {
            top_reader_context_identity: context.id(),
            states: (0..num_leaves).map(|_| None).collect(),
            term,
            doc_freq: 0,
            total_term_freq: 0,
        })
    }

    /// Creates an empty `TermStates` from an [`IndexReaderContext`].
    ///
    /// Equivalent to `TermStates(IndexReaderContext)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `context` is not a
    /// top-level context, which is the condition Java asserts.
    pub fn new(context: &Arc<dyn IndexReaderContext>) -> Result<Self> {
        Self::with_term(None, context)
    }

    /// Creates a `TermStates` holding an initial [`TermState`].
    ///
    /// Equivalent to
    /// `TermStates(IndexReaderContext, TermState, int, int, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `context` is not a
    /// top-level context.
    pub fn with_state(
        context: &Arc<dyn IndexReaderContext>,
        state: Box<dyn TermState>,
        ord: usize,
        doc_freq: i32,
        total_term_freq: i64,
    ) -> Result<Self> {
        let mut states = Self::with_term(None, context)?;
        states.register(state, ord, doc_freq, total_term_freq);
        Ok(states)
    }

    /// Returns whether this `TermStates` was built for the given
    /// [`IndexReaderContext`].
    ///
    /// Equivalent to `TermStates.wasBuiltFor(IndexReaderContext)`, which
    /// compares the stored identity by reference.
    pub fn was_built_for(&self, context: &Arc<dyn IndexReaderContext>) -> bool {
        self.top_reader_context_identity == context.id()
    }

    /// Builds a `TermStates` for `term` over every leaf of the searcher's
    /// top-level context, registering the leaves that contain it.
    ///
    /// Equivalent to `TermStates.build(IndexSearcher, Term, boolean)`.
    ///
    /// `needs_stats` selects whether the term statistics are collected; when it
    /// is `false`, [`doc_freq`](Self::doc_freq) and
    /// [`total_term_freq`](Self::total_term_freq) refuse to answer, exactly as
    /// in Java.
    ///
    /// **Divergence from Lucene 10.5.0.** Java visits every leaf up front only
    /// when `needsStats` is `true`; otherwise it defers the term-dictionary
    /// seek to the first [`get`](Self::get) for that leaf, and it schedules the
    /// seeks in the background through `TermsEnum.prepareSeekExact`, which
    /// returns an `IOBooleanSupplier`. This port visits every leaf up front in
    /// both cases, because `prepareSeekExact` and `IOBooleanSupplier` are not
    /// ported, and because the deferred path mutates the state array from
    /// `&self` — which Rust would need interior mutability for, since a
    /// [`Weight`](crate::search::Weight) is `Sync`. The states registered, the
    /// statistics accumulated and every value [`get`](Self::get) answers are
    /// identical; only the moment the term-dictionary seek happens differs, and
    /// leaves that end up not being scored are now seeked eagerly.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while seeking the term dictionaries.
    pub fn build(searcher: &IndexSearcher, term: &Term, needs_stats: bool) -> Result<Self> {
        let context = searcher.get_top_reader_context();
        let mut per_reader_term_state = Self::with_term(
            if needs_stats {
                None
            } else {
                Some(term.clone())
            },
            context,
        )?;
        for ctx in searcher.get_leaf_contexts() {
            // `Terms.getTerms(ctx.reader(), term.field())` answers `Terms.EMPTY`
            // when the field is absent.
            let terms: Box<dyn Terms> = match ctx.leaf_reader().terms(term.field())? {
                Some(terms) => terms,
                None => Box::new(EmptyTerms),
            };
            let mut terms_enum = terms.iterator()?;
            if terms_enum.seek_exact(term.bytes())? {
                let ord = ctx.ord() as usize;
                let state = terms_enum.term_state()?;
                if needs_stats {
                    per_reader_term_state.register(
                        state,
                        ord,
                        terms_enum.doc_freq()?,
                        terms_enum.total_term_freq()?,
                    );
                } else {
                    per_reader_term_state.register_state(state, ord);
                }
            }
        }
        Ok(per_reader_term_state)
    }

    /// Clears the internal state and removes all registered [`TermState`]s.
    ///
    /// Equivalent to `TermStates.clear()`.
    pub fn clear(&mut self) {
        self.doc_freq = 0;
        self.total_term_freq = 0;
        for state in &mut self.states {
            *state = None;
        }
    }

    /// Registers a [`TermState`] for a leaf ordinal and accumulates its
    /// statistics.
    ///
    /// Equivalent to `TermStates.register(TermState, int, int, long)`.
    ///
    /// # Panics
    ///
    /// In debug builds, panics when `ord` is out of range or already carries a
    /// state; Java asserts the same two conditions.
    pub fn register(
        &mut self,
        state: Box<dyn TermState>,
        ord: usize,
        doc_freq: i32,
        total_term_freq: i64,
    ) {
        self.register_state(state, ord);
        self.accumulate_statistics(doc_freq, total_term_freq);
    }

    /// Registers a [`TermState`] for a leaf ordinal without updating the term
    /// statistics.
    ///
    /// Equivalent to the expert `TermStates.register(TermState, int)`.
    ///
    /// # Panics
    ///
    /// In debug builds, panics when `ord` is out of range or already carries a
    /// state; Java asserts the same two conditions.
    pub fn register_state(&mut self, state: Box<dyn TermState>, ord: usize) {
        debug_assert!(ord < self.states.len(), "ord {ord} is out of range");
        debug_assert!(
            self.states[ord].is_none(),
            "state for ord: {ord} already registered"
        );
        self.states[ord] = Some(state);
    }

    /// Accumulates term statistics.
    ///
    /// Equivalent to the expert
    /// `TermStates.accumulateStatistics(int, long)`.
    ///
    /// # Panics
    ///
    /// In debug builds, panics when either statistic is negative or when
    /// `doc_freq` exceeds `total_term_freq`; Java asserts the same.
    pub fn accumulate_statistics(&mut self, doc_freq: i32, total_term_freq: i64) {
        debug_assert!(doc_freq >= 0);
        debug_assert!(total_term_freq >= 0);
        debug_assert!(i64::from(doc_freq) <= total_term_freq);
        self.doc_freq += doc_freq;
        self.total_term_freq += total_term_freq;
    }

    /// Returns the [`TermState`] registered for the given leaf, or `None` when
    /// the term does not exist in it.
    ///
    /// Equivalent to `TermStates.get(LeafReaderContext)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns an
    /// `IOSupplier<TermState>` so that the term-dictionary I/O of several terms
    /// can be scheduled together and resolved later. Every state is resolved by
    /// the time [`build`](Self::build) returns here, so there is nothing left to
    /// defer and the state is returned directly. It is cloned because Java
    /// hands out the stored reference while Rust cannot let one escape the
    /// borrow; [`TermState::clone_box`] is the deep copy `TermState.copyFrom`
    /// performs.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the leaf ordinal is out of
    /// range, which is the condition Java asserts.
    pub fn get(&self, ctx: &LeafReaderContext) -> Result<Option<Box<dyn TermState>>> {
        let ord = ctx.ord();
        if ord < 0 || ord as usize >= self.states.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "leaf ordinal {ord} is out of range for a TermStates over {} leaves",
                self.states.len()
            )));
        }
        Ok(self.states[ord as usize]
            .as_ref()
            .map(|state| state.clone_box()))
    }

    /// Returns the accumulated document frequency of every registered
    /// [`TermState`].
    ///
    /// Equivalent to `TermStates.docFreq()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when this instance was built with
    /// `needs_stats = false`, which is the `IllegalStateException` Java throws.
    pub fn doc_freq(&self) -> Result<i32> {
        if self.term.is_some() {
            return Err(LuceneError::IllegalState(
                "Cannot call docFreq() when needsStats=false".to_string(),
            ));
        }
        Ok(self.doc_freq)
    }

    /// Returns the accumulated total term frequency of every registered
    /// [`TermState`].
    ///
    /// Equivalent to `TermStates.totalTermFreq()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when this instance was built with
    /// `needs_stats = false`, which is the `IllegalStateException` Java throws.
    pub fn total_term_freq(&self) -> Result<i64> {
        if self.term.is_some() {
            return Err(LuceneError::IllegalState(
                "Cannot call totalTermFreq() when needsStats=false".to_string(),
            ));
        }
        Ok(self.total_term_freq)
    }
}

impl std::fmt::Display for TermStates {
    /// Equivalent to `TermStates.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "TermStates")?;
        for state in &self.states {
            writeln!(f, "  state={state:?}")?;
        }
        Ok(())
    }
}
