//! Similarity implementations, ported from `org.apache.lucene.search.similarities`.
//!
//! # Scope of this module
//!
//! Java's `Similarity` has two halves that are used at two different times:
//!
//! * the **index-time** half — `computeNorm(FieldInvertState)`, which turns the
//!   statistics gathered while a field is inverted into the single `long` the
//!   norms format stores for that document; and
//! * the **query-time** half — `scorer(float, CollectionStatistics,
//!   TermStatistics...)` and the `SimScorer`/`BulkSimScorer` it returns, which
//!   turn a term frequency and a norm into a score.
//!
//! Only the index-time half is ported here, because it is what
//! [`NormValuesWriter`](crate::index::norms_writer::NormValuesWriter) needs to
//! write a segment. The query-time half belongs with `IndexSearcher` and the
//! query stack and is deliberately left out; [`Similarity`] is shaped so that
//! adding it later is a pure addition:
//!
//! * `compute_norm` is a **default** method, exactly as in Java where
//!   `Similarity.computeNorm` is concrete and `Similarity.scorer` is abstract
//!   (`Similarity.java:153` and `:176`). Every scoring similarity Lucene ships
//!   — BM25 included — inherits `computeNorm` unchanged, so the default carries
//!   the whole behaviour. The two that do override it in Lucene Core 10.5.0 are
//!   the composition wrappers, `MultiSimilarity` (delegating to `sims[0]`,
//!   `MultiSimilarity.java:41-44`) and `PerFieldSimilarityWrapper` (delegating
//!   to the similarity of `state.getName()`,
//!   `PerFieldSimilarityWrapper.java:36-39`); both need `scorer()` to be
//!   constructible at all, so they arrive with the scoring half, and both fit
//!   by overriding this default.
//! * `discount_overlaps` is the only piece of state the index-time half reads.
//!   In Java it is a `private final` field on the abstract base set by the
//!   constructor (`Similarity.java:98`, `:124`); here it is an accessor with a
//!   default of `true`, so an implementation that does not care never mentions
//!   it.
//!
//! # The encoded norm is a *signed* byte
//!
//! `Similarity.computeNorm` returns `long`, but its body ends in
//! `return SmallFloat.intToByte4(numTerms);` (`Similarity.java:161`) — a
//! `byte`. Java widens `byte` to `long` with **sign extension**, so the value
//! that reaches the norms format is in `[-128, 127]`, not `[0, 255]`.
//!
//! This is not cosmetic. `Lucene90NormsConsumer.addNormsField` sizes the packed
//! values from `min`/`max` over exactly these longs
//! (`Lucene90NormsConsumer.java:98-127`): a segment whose norms are signed
//! bytes needs one byte per value, while the same norms treated as unsigned
//! would span `[0, 255]` and need two. Getting the sign wrong therefore changes
//! the bytes on disk, not just an in-memory number, and the resulting `.nvd`
//! could not be read by Lucene. [`Similarity::compute_norm`] reproduces the
//! sign extension explicitly.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

use crate::error::{LuceneError, Result};
use crate::index::indexing_chain::FieldInvertState;
use crate::index::IndexOptions;
use crate::util::SmallFloat;

// ---------------------------------------------------------------------------
// Similarity
// ---------------------------------------------------------------------------

/// Defines the components of scoring, of which only the index-time
/// normalization is ported so far.
///
/// Equivalent to `org.apache.lucene.search.similarities.Similarity`, restricted
/// to `computeNorm(FieldInvertState)` and the `discountOverlaps` flag that
/// governs it. See the module documentation for why the scoring half is absent
/// and how it will be added.
pub trait Similarity: Send + Sync + Debug {
    /// Returns `true` when overlap tokens — tokens whose position increment is
    /// zero, such as synonyms — are discounted from the document's length.
    ///
    /// Equivalent to `Similarity.getDiscountOverlaps()`
    /// (`Similarity.java:105`). Lucene's default is `true`
    /// (`Similarity.java:110-112`).
    ///
    /// Changing this requires re-indexing: it is consumed only by
    /// [`Self::compute_norm`], whose result is frozen into the segment.
    fn discount_overlaps(&self) -> bool {
        true
    }

    /// Computes the normalization value for a field at index time.
    ///
    /// Equivalent to `Similarity.computeNorm(FieldInvertState)`
    /// (`Similarity.java:153-162`). The default implementation encodes the
    /// number of terms with [`SmallFloat::int_to_byte4`], as every scoring
    /// similarity Lucene ships does; overriding it requires re-indexing for the
    /// change to take effect.
    ///
    /// The number of terms is:
    ///
    /// * [`FieldInvertState::unique_term_count`] when the field is indexed with
    ///   [`IndexOptions::DOCS`] — without frequencies there is no point in
    ///   counting repetitions;
    /// * [`FieldInvertState::length`] minus [`FieldInvertState::num_overlap`]
    ///   when [`Self::discount_overlaps`] is set;
    /// * [`FieldInvertState::length`] otherwise.
    ///
    /// The result is a **sign-extended byte**, in `[-128, 127]`; see the module
    /// documentation for why that matters on disk. `0` is not a legal norm.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the term count is
    /// negative, matching the `IllegalArgumentException` Java's
    /// `SmallFloat.intToByte4` throws (`SmallFloat.java:148-150`). Only the
    /// public setters on [`FieldInvertState`] can produce such a state; the
    /// inverter never does.
    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        compute_default_norm(state, self.discount_overlaps())
    }
}

/// The body of `Similarity.computeNorm`, shared by every implementation that
/// does not override it.
///
/// Equivalent to the body of `Similarity.computeNorm(FieldInvertState)`
/// (`Similarity.java:153-162`). It is a free function so that an implementation
/// that overrides [`Similarity::compute_norm`] to adjust the term count can
/// still delegate the encoding, which Java achieves with `super.computeNorm`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the resulting term count is
/// negative.
pub fn compute_default_norm(state: &FieldInvertState, discount_overlaps: bool) -> Result<i64> {
    let num_terms = if state.index_options() == IndexOptions::DOCS {
        state.unique_term_count()
    } else if discount_overlaps {
        // Java subtracts two `int`s and lets the result wrap; the values are
        // bounded by the token count of a single field, so the subtraction
        // cannot overflow, but a saturating form keeps a hostile
        // `set_num_overlap` from panicking in a debug build. A negative result
        // is refused by `int_to_byte4` either way.
        state.length().saturating_sub(state.num_overlap())
    } else {
        state.length()
    };
    // Java: `return SmallFloat.intToByte4(numTerms);` — a `byte` widened to
    // `long` with sign extension. `as i8 as i64` is that widening.
    Ok(SmallFloat::int_to_byte4(num_terms)? as i8 as i64)
}

// ---------------------------------------------------------------------------
// BM25Similarity
// ---------------------------------------------------------------------------

/// Default `k1` of [`BM25Similarity`], as in `BM25Similarity.java:98`.
const DEFAULT_K1: f32 = 1.2;
/// Default `b` of [`BM25Similarity`], as in `BM25Similarity.java:98`.
const DEFAULT_B: f32 = 0.75;

/// BM25 similarity, Lucene's default.
///
/// Equivalent to `org.apache.lucene.search.similarities.BM25Similarity`,
/// restricted to what the index-time half needs: the `k1` and `b` parameters
/// are validated and carried, and the `discountOverlaps` flag is honoured.
///
/// BM25 does **not** override `computeNorm` (verified against
/// `BM25Similarity.java` at tag `releases/lucene/10.5.0`, which contains no
/// `computeNorm` at all), so the norms it produces are exactly
/// [`compute_default_norm`] and an index can be re-scored with a different
/// similarity without being rebuilt. `k1` and `b` take part only in scoring and are
/// therefore inert until the query-time half is ported; they are validated here
/// so that a misconfigured similarity is refused when it is built rather than
/// when it is first used to score.
///
/// Introduced in Stephen E. Robertson, Steve Walker, Susan Jones, Micheline
/// Hancock-Beaulieu, and Mike Gatford, *Okapi at TREC-3*, TREC 1994.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BM25Similarity {
    k1: f32,
    b: f32,
    discount_overlaps: bool,
}

impl Default for BM25Similarity {
    fn default() -> Self {
        Self {
            k1: DEFAULT_K1,
            b: DEFAULT_B,
            discount_overlaps: true,
        }
    }
}

impl BM25Similarity {
    /// Creates a BM25 similarity with `k1 = 1.2`, `b = 0.75` and
    /// `discountOverlaps = true`.
    ///
    /// Equivalent to `new BM25Similarity()` (`BM25Similarity.java:97-99`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a BM25 similarity with the default `k1` and `b` and the supplied
    /// `discount_overlaps`.
    ///
    /// Equivalent to `new BM25Similarity(boolean)`
    /// (`BM25Similarity.java:84-86`).
    pub fn with_discount_overlaps(discount_overlaps: bool) -> Self {
        Self {
            discount_overlaps,
            ..Self::default()
        }
    }

    /// Creates a BM25 similarity with the supplied `k1` and `b` and
    /// `discountOverlaps = true`.
    ///
    /// Equivalent to `new BM25Similarity(float, float)`
    /// (`BM25Similarity.java:67-69`).
    ///
    /// # Errors
    ///
    /// See [`Self::with_parameters_and_overlaps`].
    pub fn with_parameters(k1: f32, b: f32) -> Result<Self> {
        Self::with_parameters_and_overlaps(k1, b, true)
    }

    /// Creates a BM25 similarity with every parameter supplied.
    ///
    /// Equivalent to `new BM25Similarity(float, float, boolean)`
    /// (`BM25Similarity.java:46-57`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `k1` is not finite or is
    /// negative, or if `b` is NaN or outside `[0, 1]` — the two
    /// `IllegalArgumentException`s Java's constructor throws.
    pub fn with_parameters_and_overlaps(k1: f32, b: f32, discount_overlaps: bool) -> Result<Self> {
        if !k1.is_finite() || k1 < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal k1 value: {k1}, must be a non-negative finite value"
            )));
        }
        if b.is_nan() || !(0.0..=1.0).contains(&b) {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal b value: {b}, must be between 0 and 1"
            )));
        }
        Ok(Self {
            k1,
            b,
            discount_overlaps,
        })
    }

    /// Returns the `k1` parameter, which controls term-frequency saturation.
    ///
    /// Equivalent to `BM25Similarity.getK1()`.
    pub fn k1(&self) -> f32 {
        self.k1
    }

    /// Returns the `b` parameter, which controls length normalization.
    ///
    /// Equivalent to `BM25Similarity.getB()`.
    pub fn b(&self) -> f32 {
        self.b
    }
}

impl Similarity for BM25Similarity {
    fn discount_overlaps(&self) -> bool {
        self.discount_overlaps
    }
}

impl fmt::Display for BM25Similarity {
    /// Renders the similarity the way `BM25Similarity.toString()` does
    /// (`BM25Similarity.java:309-311`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BM25(k1={},b={})", self.k1, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(options: IndexOptions, length: i32, overlap: i32, unique: i32) -> FieldInvertState {
        let mut state = FieldInvertState::new(10, "body".to_string(), options);
        state.set_length(length);
        state.set_num_overlap(overlap);
        for _ in 0..unique {
            state.increment_unique_term_count();
        }
        state
    }

    #[test]
    fn the_default_norm_encodes_the_length_minus_the_overlaps() {
        let sim = BM25Similarity::new();
        let state = state(IndexOptions::DOCS_AND_FREQS, 10, 3, 4);
        // 10 - 3 = 7, and 7 < NUM_FREE_VALUES so it is stored verbatim.
        assert_eq!(sim.compute_norm(&state).unwrap(), 7);
    }

    #[test]
    fn keeping_overlaps_counts_the_whole_length() {
        let sim = BM25Similarity::with_discount_overlaps(false);
        let state = state(IndexOptions::DOCS_AND_FREQS, 10, 3, 4);
        assert_eq!(sim.compute_norm(&state).unwrap(), 10);
    }

    #[test]
    fn a_docs_only_field_is_normalized_by_its_unique_term_count() {
        // Without frequencies, repetitions carry no information, so Lucene uses
        // the number of distinct terms and ignores both length and overlaps.
        let sim = BM25Similarity::new();
        let state = state(IndexOptions::DOCS, 100, 40, 5);
        assert_eq!(sim.compute_norm(&state).unwrap(), 5);
        let no_discount = BM25Similarity::with_discount_overlaps(false);
        assert_eq!(no_discount.compute_norm(&state).unwrap(), 5);
    }

    #[test]
    fn a_long_field_encodes_to_a_negative_long() {
        // The encoding is a *signed* byte widened to a long. The first term
        // count whose byte has the high bit set must therefore come back
        // negative, which is what the norms format stores.
        let sim = BM25Similarity::new();
        let mut first_negative = None;
        for length in 0..100_000 {
            let norm = sim
                .compute_norm(&state(IndexOptions::DOCS_AND_FREQS, length, 0, 0))
                .unwrap();
            assert!(
                (-128..=127).contains(&norm),
                "norm {norm} for length {length} is not a signed byte"
            );
            if norm < 0 && first_negative.is_none() {
                first_negative = Some((length, norm));
            }
        }
        let (length, norm) = first_negative.expect("some length must encode to a negative byte");
        // `intToByte4` maps this length to the unsigned byte 128.
        assert_eq!(SmallFloat::int_to_byte4(length).unwrap(), 128);
        assert_eq!(norm, -128);
    }

    #[test]
    fn every_norm_round_trips_through_the_small_float_decoder() {
        let sim = BM25Similarity::new();
        for length in [0, 1, 23, 24, 25, 100, 1_000, 65_536, i32::MAX] {
            let norm = sim
                .compute_norm(&state(IndexOptions::DOCS_AND_FREQS, length, 0, 0))
                .unwrap();
            let decoded = SmallFloat::byte4_to_int(norm as u8);
            assert!(
                decoded <= length,
                "decoding {norm} for length {length} gave {decoded}, which is not a lower bound"
            );
        }
    }

    #[test]
    fn a_negative_term_count_is_refused() {
        let sim = BM25Similarity::new();
        let state = state(IndexOptions::DOCS_AND_FREQS, 2, 5, 0);
        let error = sim.compute_norm(&state).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m) if m.contains("positive")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn an_illegal_k1_or_b_is_refused() {
        assert!(BM25Similarity::with_parameters(-1.0, 0.5).is_err());
        assert!(BM25Similarity::with_parameters(f32::INFINITY, 0.5).is_err());
        assert!(BM25Similarity::with_parameters(f32::NAN, 0.5).is_err());
        assert!(BM25Similarity::with_parameters(1.2, -0.1).is_err());
        assert!(BM25Similarity::with_parameters(1.2, 1.1).is_err());
        assert!(BM25Similarity::with_parameters(1.2, f32::NAN).is_err());
        // The boundaries are legal.
        assert!(BM25Similarity::with_parameters(0.0, 0.0).is_ok());
        assert!(BM25Similarity::with_parameters(0.0, 1.0).is_ok());
    }

    #[test]
    fn the_defaults_match_lucene() {
        let sim = BM25Similarity::new();
        assert_eq!(sim.k1(), 1.2);
        assert_eq!(sim.b(), 0.75);
        assert!(sim.discount_overlaps());
        assert_eq!(sim.to_string(), "BM25(k1=1.2,b=0.75)");
    }

    #[test]
    fn a_field_with_no_terms_encodes_to_zero() {
        // `PerField.finish` never reaches `computeNorm` for an empty field: it
        // short-circuits to a norm of zero. The encoder agrees, which is why
        // Lucene can treat a returned zero as a bug in a custom similarity.
        let sim = BM25Similarity::new();
        assert_eq!(
            sim.compute_norm(&state(IndexOptions::DOCS_AND_FREQS, 0, 0, 0))
                .unwrap(),
            0
        );
    }
}
