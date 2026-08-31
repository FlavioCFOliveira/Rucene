//! The Indri scorer base, ported from
//! `org.apache.lucene.search.IndriScorer`.

#![deny(unsafe_code)]

use crate::search::scorer::Scorer;

/// The Indri parent scorer, which stores the boost so that Indri scorers can
/// use it outside the term.
///
/// Equivalent to the abstract class `org.apache.lucene.search.IndriScorer`,
/// which extends `Scorer` and adds a `boost` field. Rust has no implementation
/// inheritance, so the field becomes the one accessor of this trait and each
/// implementation carries the value.
pub trait IndriScorer: Scorer {
    /// Returns the boost this scorer contributes.
    ///
    /// Equivalent to `IndriScorer.getBoost()`.
    fn get_boost(&self) -> f32;
}
