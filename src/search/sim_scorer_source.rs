//! Owned similarity scorers.
//!
//! **This module has no counterpart in Lucene 10.5.0.** It exists because the
//! two halves of the port meet with incompatible lifetimes:
//!
//! * [`Similarity::scorer`] is declared
//!   `fn scorer<'a>(&'a self, ..) -> Box<dyn SimScorer + 'a>`, so the scorer it
//!   returns borrows the similarity that produced it;
//! * [`Weight::scorer_supplier`](crate::search::Weight::scorer_supplier) and
//!   [`ScorerSupplier::get`](crate::search::ScorerSupplier::get) return
//!   `Box<dyn ScorerSupplier>` and `Box<dyn Scorer>`, which are `'static`.
//!
//! Java has no such tension: `TermWeight` holds both the `Similarity` and the
//! `Similarity.SimScorer` it produced, and the garbage collector keeps the
//! former alive for as long as the latter needs it. In Rust the same shape is a
//! self-referential struct, which cannot be written without `unsafe`, and this
//! crate forbids `unsafe`.
//!
//! [`Similarity::scorer_owned`] is the entry point that resolves it: it takes
//! `self: Arc<Self>` and returns a `Box<dyn SimScorer + 'static>`, so the
//! similarity is shared *into* the scorer rather than borrowed by it — the same
//! object graph Java builds. [`SimScorerSource`] builds that scorer once, in
//! [`SimScorerSource::new`], and delegates every call to it; it remains as the
//! single place where a weight stores what Lucene stores as a
//! `Similarity.SimScorer` field, and as the home of the helper scorers below.
//!
//! The scores produced are identical to a rebuild-on-every-use design; only the
//! amount of work spent producing them differs, so nothing about which
//! documents match or how they rank depends on this module.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::search::similarities::{
    BulkSimScorer, CollectionStatistics, Explanation, SimScorer, Similarity, TermStatistics,
};

/// A [`SimScorer`] that owns the [`Similarity`] and statistics it is built
/// from.
///
/// See the module documentation for why this type exists. It stands in for a
/// `Similarity.SimScorer` field wherever Lucene stores one on a `Weight`.
pub struct SimScorerSource {
    similarity: Arc<dyn Similarity>,
    boost: f32,
    collection_stats: CollectionStatistics,
    term_stats: Vec<TermStatistics>,
    scorer: Box<dyn SimScorer + Send + Sync + 'static>,
}

impl std::fmt::Debug for SimScorerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimScorerSource")
            .field("similarity", &self.similarity)
            .field("boost", &self.boost)
            .field("collection_stats", &self.collection_stats)
            .field("term_stats", &self.term_stats)
            .finish_non_exhaustive()
    }
}

impl SimScorerSource {
    /// Captures everything [`Similarity::scorer`] needs.
    ///
    /// Equivalent to the `similarity.scorer(boost, collectionStats, termStats)`
    /// call a Lucene weight makes in its constructor.
    pub fn new(
        similarity: Arc<dyn Similarity>,
        boost: f32,
        collection_stats: CollectionStatistics,
        term_stats: Vec<TermStatistics>,
    ) -> Self {
        let scorer = Arc::clone(&similarity).scorer_owned(boost, &collection_stats, &term_stats);
        Self {
            similarity,
            boost,
            collection_stats,
            term_stats,
            scorer,
        }
    }

    /// Returns the similarity this source scores with.
    pub fn similarity(&self) -> &Arc<dyn Similarity> {
        &self.similarity
    }

    /// Returns the boost this source scores with.
    pub fn boost(&self) -> f32 {
        self.boost
    }

    /// Returns the collection statistics this source scores with.
    pub fn collection_stats(&self) -> &CollectionStatistics {
        &self.collection_stats
    }

    /// Returns the term statistics this source scores with.
    pub fn term_stats(&self) -> &[TermStatistics] {
        &self.term_stats
    }

    /// Runs `f` with the [`SimScorer`] this source describes.
    ///
    /// The scorer is the one [`Similarity::scorer_owned`] produced when this
    /// source was created, so this is a plain borrow.
    pub fn with<R>(&self, f: impl FnOnce(&dyn SimScorer) -> R) -> R {
        f(&*self.scorer)
    }

    /// Runs `f` with a [`BulkSimScorer`] over the [`SimScorer`] this source
    /// describes.
    ///
    /// Equivalent to `simScorer.asBulkSimScorer()` on a stored scorer.
    pub fn with_bulk<R>(&self, f: impl FnOnce(&mut dyn BulkSimScorer) -> R) -> R {
        self.with(|scorer| {
            let mut bulk = scorer.as_bulk_sim_scorer();
            f(&mut *bulk)
        })
    }
}

impl SimScorer for SimScorerSource {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        self.with(|scorer| scorer.score(freq, norm))
    }

    fn explain(&self, freq: &Explanation, norm: i64) -> Explanation {
        self.with(|scorer| scorer.explain(freq, norm))
    }
}

/// A [`SimScorer`] whose score is always `1`.
///
/// Equivalent to the anonymous `Similarity.SimScorer` that
/// [`PhraseWeight`](crate::search::PhraseWeight) installs when the phrase has
/// no terms at all, or when scores are not needed.
#[derive(Debug, Default, Clone, Copy)]
pub struct OneSimScorer;

impl SimScorer for OneSimScorer {
    fn score(&self, _freq: f32, _norm: i64) -> f32 {
        1.0
    }
}

/// Returns the short type name of a [`Similarity`], for an explanation.
///
/// **Divergence from Lucene 10.5.0.** Java writes
/// `similarity.getClass().getSimpleName()`. Rust has no runtime class name for
/// a trait object, so this reads the leading identifier of the similarity's
/// [`Debug`](std::fmt::Debug) representation, which is exactly the type name
/// for the `#[derive(Debug)]` implementations every shipped similarity uses.
/// Only the wording of an [`Explanation`] depends on it.
pub fn similarity_simple_name(similarity: &dyn Similarity) -> String {
    let debug = format!("{similarity:?}");
    let name: String = debug
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        debug
    } else {
        name
    }
}

/// A [`SimScorer`] whose score is always `0`.
///
/// Equivalent to the anonymous `Similarity.SimScorer` that
/// `TermQuery.TermWeight` installs when scores are not needed, so that the
/// default BM25 scorer's `float[]` allocations are avoided — see
/// [LUCENE issue 12297](https://github.com/apache/lucene/issues/12297). It is
/// never expected to be called.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZeroSimScorer;

impl SimScorer for ZeroSimScorer {
    fn score(&self, _freq: f32, _norm: i64) -> f32 {
        0.0
    }
}

/// The shared handle every scoring weight in this package stores in place of a
/// `Similarity.SimScorer` field.
///
/// Java stores a bare `Similarity.SimScorer` reference on the weight and hands
/// the same object to every scorer it creates. `Arc` is that sharing.
pub type SharedSimScorer = Arc<dyn SimScorer + Send + Sync>;

/// A [`SimScorer`] that delegates to a [`SharedSimScorer`].
///
/// It exists so that a shared scorer can also be handed to an API taking an
/// owned `Box<dyn SimScorer>` — [`MaxScoreCache::new`] above all — which is
/// what Java achieves by passing the same reference twice.
///
/// [`MaxScoreCache::new`]: crate::search::MaxScoreCache::new
#[derive(Clone)]
pub struct SharedSimScorerRef(SharedSimScorer);

impl std::fmt::Debug for SharedSimScorerRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedSimScorerRef")
    }
}

impl SharedSimScorerRef {
    /// Wraps a shared scorer.
    pub fn new(scorer: SharedSimScorer) -> Self {
        Self(scorer)
    }
}

impl SimScorer for SharedSimScorerRef {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        self.0.score(freq, norm)
    }

    fn explain(&self, freq: &Explanation, norm: i64) -> Explanation {
        self.0.explain(freq, norm)
    }
}
