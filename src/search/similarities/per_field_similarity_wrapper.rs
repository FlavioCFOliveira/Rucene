//! Per-field similarity selection, ported from
//! `org.apache.lucene.search.similarities.PerFieldSimilarityWrapper`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::indexing_chain::FieldInvertState;

use super::{CollectionStatistics, SimScorer, Similarity, TermStatistics};

/// Provides the ability to use a different [`Similarity`] for different fields.
///
/// Equivalent to
/// `org.apache.lucene.search.similarities.PerFieldSimilarityWrapper`.
/// Implementors provide [`Self::get`], and forward
/// [`Similarity::compute_norm`] and [`Similarity::scorer`] to
/// [`per_field_compute_norm`] and [`per_field_scorer`] — the two methods Java
/// declares `final` on the abstract class, and which a Rust subtrait cannot
/// supply on its supertrait's behalf.
///
/// # Divergence: `get` returns a borrow
///
/// Java's `Similarity get(String name)` may return a freshly constructed
/// similarity. Here it returns `&dyn Similarity` borrowed from `self`, because
/// [`Similarity::scorer`] hands out a scorer that borrows the similarity it
/// came from: a similarity built inside `get` would be dropped before the
/// scorer that reads it. Implementations therefore have to own their per-field
/// similarities — typically in a map keyed by field name, which is what the
/// real implementations do anyway — rather than build one per call.
pub trait PerFieldSimilarityWrapper: Similarity {
    /// Returns the [`Similarity`] to score `name` with.
    ///
    /// Equivalent to the abstract
    /// `PerFieldSimilarityWrapper.get(String)`
    /// (`PerFieldSimilarityWrapper.java:47`).
    fn get(&self, name: &str) -> &dyn Similarity;
}

/// Computes the norm with the similarity registered for the field being
/// inverted.
///
/// This is the body of `PerFieldSimilarityWrapper.computeNorm`
/// (`PerFieldSimilarityWrapper.java:36-39`), which Java declares `final`.
///
/// # Errors
///
/// Propagates whatever the delegate's [`Similarity::compute_norm`] returns.
pub fn per_field_compute_norm<W>(wrapper: &W, state: &FieldInvertState) -> Result<i64>
where
    W: PerFieldSimilarityWrapper + ?Sized,
{
    wrapper.get(state.name()).compute_norm(state)
}

/// Builds the scorer with the similarity registered for the field being
/// queried.
///
/// This is the body of `PerFieldSimilarityWrapper.scorer`
/// (`PerFieldSimilarityWrapper.java:41-45`), which Java declares `final`.
pub fn per_field_scorer<'a, W>(
    wrapper: &'a W,
    boost: f32,
    collection_stats: &CollectionStatistics,
    term_stats: &[TermStatistics],
) -> Box<dyn SimScorer + 'a>
where
    W: PerFieldSimilarityWrapper + ?Sized,
{
    wrapper
        .get(collection_stats.field())
        .scorer(boost, collection_stats, term_stats)
}
