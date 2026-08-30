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
//! Both halves are ported. The index-time half is what
//! [`NormValuesWriter`](crate::index::norms_writer::NormValuesWriter) needs to
//! write a segment; the query-time half is what the ranking models in this
//! module exist for.
//!
//! # Abstract classes, `final` methods, and how they map to traits
//!
//! The package is a set of small abstract-class hierarchies. Each becomes a
//! Rust trait plus its concrete implementations, keeping Lucene's names. Two
//! Java constructs have no Rust equivalent and are handled the same way
//! everywhere:
//!
//! * **A `final` method on an abstract base** — `SimilarityBase.scorer`,
//!   `TFIDFSimilarity.scorer`, `PerFieldSimilarityWrapper.computeNorm` — cannot
//!   be inherited, because a Rust subtrait cannot supply a supertrait's
//!   required method. Each becomes a free function
//!   ([`similarity_base_scorer`], [`tfidf_scorer`], [`per_field_compute_norm`],
//!   [`per_field_scorer`]) that every concrete type forwards to in one line.
//! * **`getClass().getSimpleName()`**, which several explanations embed in
//!   their descriptions, has no reflective equivalent. Every trait whose
//!   explanations need it declares a `simple_name` method returning the Java
//!   class name.
//!
//! Java's `Explanation`, `CollectionStatistics` and `TermStatistics` belong to
//! `org.apache.lucene.search`, which has not been ported yet; they live under
//! this module for now and are re-exported from [`crate::search`], which is
//! where they will stay. See [`explanation`] for the full note.
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
//!
//! Query-time scoring reads the same byte back unsigned, with
//! `norm & 0xFF`, so a norm that encodes as `-128` decodes through table entry
//! `128`. Every scorer here reproduces that masking.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

use crate::error::{LuceneError, Result};
use crate::index::indexing_chain::FieldInvertState;
use crate::index::IndexOptions;
use crate::util::{ArrayUtil, SmallFloat};

pub mod after_effect;
pub mod axiomatic;
pub mod basic_model;
pub mod basic_stats;
pub mod boolean_similarity;
pub mod dfi_similarity;
pub mod dfr_similarity;
pub mod distribution;
pub mod explanation;
pub mod ib_similarity;
pub mod independence;
mod java_fmt;
mod java_math;
pub mod lambda;
pub mod lm_similarity;
pub mod multi_similarity;
pub mod normalization;
pub mod per_field_similarity_wrapper;
pub mod raw_tf_similarity;
pub mod similarity_base;
pub mod statistics;
pub mod tfidf_similarity;

pub use after_effect::{AfterEffect, AfterEffectB, AfterEffectL};
pub use axiomatic::{
    axiomatic_explain, axiomatic_explain_details, axiomatic_score, Axiomatic, AxiomaticF1EXP,
    AxiomaticF1LOG, AxiomaticF2EXP, AxiomaticF2LOG, AxiomaticF3EXP, AxiomaticF3LOG,
};
pub use basic_model::{BasicModel, BasicModelG, BasicModelIF, BasicModelIn, BasicModelIne};
pub use basic_stats::BasicStats;
pub use boolean_similarity::BooleanSimilarity;
pub use dfi_similarity::DFISimilarity;
pub use dfr_similarity::DFRSimilarity;
pub use distribution::{Distribution, DistributionLL, DistributionSPL};
pub use explanation::{Explanation, ExplanationValue};
pub use ib_similarity::IBSimilarity;
pub use independence::{
    Independence, IndependenceChiSquared, IndependenceSaturated, IndependenceStandardized,
};
pub use lambda::{Lambda, LambdaDF, LambdaTTF};
pub use lm_similarity::{
    lm_explain_details, lm_fill_basic_stats, lm_new_stats, lm_to_string, CollectionModel,
    DefaultCollectionModel, IndriCollectionModel, IndriDirichletSimilarity, LMDirichletSimilarity,
    LMJelinekMercerSimilarity, LMSimilarity, LMStats,
};
pub use multi_similarity::MultiSimilarity;
pub use normalization::{
    NoNormalization, Normalization, NormalizationH1, NormalizationH2, NormalizationH3,
    NormalizationZ,
};
pub use per_field_similarity_wrapper::{
    per_field_compute_norm, per_field_scorer, PerFieldSimilarityWrapper,
};
pub use raw_tf_similarity::RawTFSimilarity;
pub use similarity_base::{fill_basic_stats, log2, similarity_base_scorer, SimilarityBase};
pub use statistics::{CollectionStatistics, TermStatistics};
pub use tfidf_similarity::{tfidf_scorer, ClassicSimilarity, TFIDFSimilarity};

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

    /// Computes any collection-level weight — IDF, average document length and
    /// the like — needed for scoring a query, and returns the scorer that will
    /// use it.
    ///
    /// Equivalent to the abstract
    /// `Similarity.scorer(float, CollectionStatistics, TermStatistics...)`
    /// (`Similarity.java:175-176`). It is called once per query term set, so
    /// implementations are free to precompute whatever they need; the
    /// statistics passed in already carry every raw value, so no I/O is
    /// triggered.
    ///
    /// * `boost` — a multiplicative factor to apply to the produced scores;
    /// * `collection_stats` — collection-level statistics for the field;
    /// * `term_stats` — term-level statistics, one entry per query term
    ///   (several entries describe a phrase).
    ///
    /// The returned scorer borrows `self`, because Java's scorers are inner
    /// classes holding a reference to the similarity that built them, and
    /// because they read parameters that live on it. The similarity must
    /// therefore outlive every scorer it produces — which is exactly the Java
    /// contract, only checked at compile time.
    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a>;
}

// ---------------------------------------------------------------------------
// SimScorer / BulkSimScorer
// ---------------------------------------------------------------------------

/// Scores a single document from its term frequency and its encoded norm.
///
/// Equivalent to the abstract nested class `Similarity.SimScorer`
/// (`Similarity.java:181-231`). Implementations must honour the monotonicity
/// contract Lucene relies on for its dynamic pruning:
///
/// * the score must not decrease when `freq` increases;
/// * the score must not increase when the *unsigned* `norm` increases;
/// * consequently the maximum score is bounded by `score(f32::MAX, 1)`.
pub trait SimScorer {
    /// Scores a single document.
    ///
    /// Equivalent to `Similarity.SimScorer.score(float, long)`
    /// (`Similarity.java:204`). `freq` is the sloppy term frequency and must be
    /// finite and positive; `norm` is the encoded normalization factor
    /// [`Similarity::compute_norm`] produced at index time, or `1` when norms
    /// are disabled. `norm` is never `0`.
    fn score(&self, freq: f32, norm: i64) -> f32;

    /// Explains the score for a single document.
    ///
    /// Equivalent to `Similarity.SimScorer.explain(Explanation, long)`
    /// (`Similarity.java:224-230`), whose default wraps the score around the
    /// supplied frequency explanation.
    fn explain(&self, freq: &Explanation, norm: i64) -> Explanation {
        Explanation::matched(
            self.score(freq.value().float_value(), norm),
            format!("score(freq={}), with freq of:", freq.value()),
            vec![freq.clone()],
        )
    }

    /// Returns a [`BulkSimScorer`] producing exactly the same scores as this
    /// scorer, but more efficient at computing them in bulk.
    ///
    /// Equivalent to `Similarity.SimScorer.asBulkSimScorer()`
    /// (`Similarity.java:213-215`). The returned instance is not thread-safe,
    /// which is why [`BulkSimScorer::score`] takes `&mut self`.
    fn as_bulk_sim_scorer(&self) -> Box<dyn BulkSimScorer + '_> {
        Box::new(DefaultBulkSimScorer { scorer: self })
    }
}

/// Specialization of [`SimScorer`] for bulk computation of scores.
///
/// Equivalent to the nested interface `Similarity.BulkSimScorer`
/// (`Similarity.java:234-249`).
pub trait BulkSimScorer {
    /// Computes `size` scores at once: for each `i` in `[0, size)`,
    /// `scores[i]` becomes `score(freqs[i], norms[i])`.
    ///
    /// Equivalent to `Similarity.BulkSimScorer.score(int, float[], long[],
    /// float[])`.
    ///
    /// Java allows `freqs` and `scores` to be the same array; Rust's aliasing
    /// rules forbid it, so the two are distinct slices here. That is the only
    /// difference: the loop reads every `freqs[i]` before writing
    /// `scores[i]` in Java too, so no caller relying on the aliasing gets a
    /// different result from passing two arrays.
    ///
    /// # Panics
    ///
    /// Panics when any of the three slices holds fewer than `size` elements,
    /// which is the same contract violation Java reports with an
    /// `ArrayIndexOutOfBoundsException`.
    fn score(&mut self, size: usize, freqs: &[f32], norms: &[i64], scores: &mut [f32]);
}

/// The loop `Similarity.DefaultBulkSimScorer` runs when a scorer does not
/// specialize bulk scoring (`Similarity.java:251-270`).
struct DefaultBulkSimScorer<'a, S: ?Sized> {
    scorer: &'a S,
}

impl<S: SimScorer + ?Sized> BulkSimScorer for DefaultBulkSimScorer<'_, S> {
    fn score(&mut self, size: usize, freqs: &[f32], norms: &[i64], scores: &mut [f32]) {
        let freqs = &freqs[..size];
        let norms = &norms[..size];
        let scores = &mut scores[..size];
        for ((score, freq), norm) in scores.iter_mut().zip(freqs).zip(norms) {
            *score = self.scorer.score(*freq, *norm);
        }
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

    /// The BM25 inverse document frequency.
    ///
    /// Equivalent to `BM25Similarity.idf(long, long)`
    /// (`BM25Similarity.java:100-103`), implemented as
    /// `log(1 + (docCount - docFreq + 0.5) / (docFreq + 0.5))` in `double` and
    /// narrowed once.
    pub fn idf(&self, doc_freq: i64, doc_count: i64) -> f32 {
        (1.0 + ((doc_count - doc_freq) as f64 + 0.5) / (doc_freq as f64 + 0.5)).ln() as f32
    }

    /// The average field length.
    ///
    /// Equivalent to `BM25Similarity.avgFieldLength(CollectionStatistics)`
    /// (`BM25Similarity.java:106-108`): `sumTotalTermFreq / docCount`, computed
    /// in `double` and narrowed once.
    pub fn avg_field_length(&self, collection_stats: &CollectionStatistics) -> f32 {
        (collection_stats.sum_total_term_freq() as f64 / collection_stats.doc_count() as f64) as f32
    }

    /// Computes a score factor for a simple term, and explains it.
    ///
    /// Equivalent to
    /// `BM25Similarity.idfExplain(CollectionStatistics, TermStatistics)`
    /// (`BM25Similarity.java:139-148`).
    ///
    /// `CollectionStatistics::doc_count` is used rather than the reader's
    /// `numDocs()`, because `TermStatistics::doc_freq` is used as well: when
    /// the latter is inaccurate so is the former, in the same direction, and
    /// `doc_count` does not skew when fields are sparse.
    pub fn idf_explain(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
    ) -> Explanation {
        let df = term_stats.doc_freq();
        let doc_count = collection_stats.doc_count();
        let idf = self.idf(df, doc_count);
        Explanation::matched(
            idf,
            "idf, computed as log(1 + (N - n + 0.5) / (n + 0.5)) from:",
            vec![
                Explanation::matched(df, "n, number of documents containing term", vec![]),
                Explanation::matched(doc_count, "N, total number of documents with field", vec![]),
            ],
        )
    }

    /// Computes a score factor for a phrase, and explains it.
    ///
    /// Equivalent to
    /// `BM25Similarity.idfExplain(CollectionStatistics, TermStatistics[])`
    /// (`BM25Similarity.java:159-169`), which sums the per-term factors into a
    /// `double` before narrowing once.
    pub fn idf_explain_phrase(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Explanation {
        let mut idf = 0.0f64;
        let mut details = Vec::with_capacity(term_stats.len());
        for stat in term_stats {
            let idf_explain = self.idf_explain(collection_stats, stat);
            idf += f64::from(idf_explain.value().float_value());
            details.push(idf_explain);
        }
        Explanation::matched(idf as f32, "idf, sum of:", details)
    }
}

impl Similarity for BM25Similarity {
    fn discount_overlaps(&self) -> bool {
        self.discount_overlaps
    }

    /// Precomputes the IDF, the average field length and the 256 inverse norm
    /// factors, as `BM25Similarity.scorer` does
    /// (`BM25Similarity.java:170-184`). Java declares the method `final`.
    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        let idf = match term_stats {
            [single] => self.idf_explain(collection_stats, single),
            many => self.idf_explain_phrase(collection_stats, many),
        };
        let avgdl = self.avg_field_length(collection_stats);

        let mut cache = [0.0f32; 256];
        for (i, entry) in cache.iter_mut().enumerate() {
            // Java: 1f / (k1 * ((1 - b) + b * LENGTH_TABLE[i] / avgdl)), all in
            // `float`; `1 - b` is an `int` minus a `float`, hence a `float`.
            *entry = 1.0
                / (self.k1
                    * ((1.0 - self.b) + self.b * similarity_base::norm_length(i as i64) / avgdl));
        }
        Box::new(BM25Scorer::new(boost, self.k1, self.b, idf, avgdl, cache))
    }
}

/// Collection statistics for the BM25 model.
///
/// Equivalent to `BM25Similarity.BM25Scorer` (`BM25Similarity.java:187-303`).
#[derive(Debug)]
struct BM25Scorer {
    /// Query boost.
    boost: f32,
    /// `k1` value for the scale factor.
    k1: f32,
    /// `b` value for the length-normalization impact.
    b: f32,
    /// BM25's IDF and its explanation.
    idf: Explanation,
    /// The average document length.
    avgdl: f32,
    /// Precomputed `1 / (k1 * ((1 - b) + b * dl / avgdl))` per encoded norm.
    cache: [f32; 256],
    /// `idf * boost`.
    weight: f32,
}

impl BM25Scorer {
    fn new(boost: f32, k1: f32, b: f32, idf: Explanation, avgdl: f32, cache: [f32; 256]) -> Self {
        let weight = boost * idf.value().float_value();
        Self {
            boost,
            k1,
            b,
            idf,
            avgdl,
            cache,
            weight,
        }
    }

    /// The rewritten BM25 kernel of `BM25Scorer.doScore`
    /// (`BM25Similarity.java:213-228`).
    ///
    /// Lucene rewrites `weight * freq / (freq + norm)` as
    /// `weight - weight / (1 + freq * (1 / norm))` so that the result stays
    /// monotonic in both `freq` and `norm` without promoting to `double`:
    /// multiplication and division each round to the nearest `float`, and
    /// monotonicity survives `x -> 1 + x` and `x -> 1 - 1 / x`. The expansion
    /// also runs slightly faster than the equivalent product.
    fn do_score(&self, freq: f32, norm_inverse: f32) -> f32 {
        self.weight - self.weight / (1.0 + freq * norm_inverse)
    }

    /// The `tf` sub-explanation of `BM25Scorer.explainTF`
    /// (`BM25Similarity.java:275-292`).
    fn explain_tf(&self, freq: &Explanation, norm: i64) -> Explanation {
        let doclen = similarity_base::norm_length(norm);
        let mut subs = vec![
            freq.clone(),
            Explanation::matched(self.k1, "k1, term saturation parameter", vec![]),
            Explanation::matched(self.b, "b, length normalization parameter", vec![]),
        ];
        // Norms above byte 39 are stored lossily, so Lucene says so.
        if (norm & 0xFF) > 39 {
            subs.push(Explanation::matched(
                doclen,
                "dl, length of field (approximate)",
                vec![],
            ));
        } else {
            subs.push(Explanation::matched(doclen, "dl, length of field", vec![]));
        }
        subs.push(Explanation::matched(
            self.avgdl,
            "avgdl, average length of field",
            vec![],
        ));
        let norm_inverse = 1.0 / (self.k1 * ((1.0 - self.b) + self.b * doclen / self.avgdl));
        Explanation::matched(
            1.0 - 1.0 / (1.0 + freq.value().float_value() * norm_inverse),
            "tf, computed as freq / (freq + k1 * (1 - b + b * dl / avgdl)) from:",
            subs,
        )
    }

    /// The boost and IDF nodes of `BM25Scorer.explainConstantFactors`
    /// (`BM25Similarity.java:294-303`).
    fn explain_constant_factors(&self) -> Vec<Explanation> {
        let mut subs = Vec::new();
        if self.boost != 1.0 {
            subs.push(Explanation::matched(self.boost, "boost", vec![]));
        }
        subs.push(self.idf.clone());
        subs
    }
}

impl SimScorer for BM25Scorer {
    fn score(&self, freq: f32, encoded_norm: i64) -> f32 {
        let norm_inverse = self.cache[usize::from(encoded_norm as u8)];
        self.do_score(freq, norm_inverse)
    }

    fn explain(&self, freq: &Explanation, encoded_norm: i64) -> Explanation {
        let mut subs = self.explain_constant_factors();
        subs.push(self.explain_tf(freq, encoded_norm));
        let norm_inverse = self.cache[usize::from(encoded_norm as u8)];
        // Java does not say "product of" here: the rewrite in `do_score`
        // introduces a small rounding difference that `CheckHits` complains
        // about, so the value is recomputed the same way instead.
        Explanation::matched(
            self.weight - self.weight / (1.0 + freq.value().float_value() * norm_inverse),
            format!(
                "score(freq={}), computed as boost * idf * tf from:",
                freq.value()
            ),
            subs,
        )
    }

    fn as_bulk_sim_scorer(&self) -> Box<dyn BulkSimScorer + '_> {
        Box::new(BM25BulkSimScorer {
            scorer: self,
            norm_inverses: Vec::new(),
        })
    }
}

/// The specialized bulk scorer of `BM25Scorer.asBulkSimScorer`
/// (`BM25Similarity.java:238-263`): the norms are decoded into a scratch buffer
/// first so that the scoring loop auto-vectorizes.
struct BM25BulkSimScorer<'a> {
    scorer: &'a BM25Scorer,
    norm_inverses: Vec<f32>,
}

impl BulkSimScorer for BM25BulkSimScorer<'_> {
    fn score(&mut self, size: usize, freqs: &[f32], norms: &[i64], scores: &mut [f32]) {
        if self.norm_inverses.len() < size {
            self.norm_inverses
                .resize(ArrayUtil::oversize(size, std::mem::size_of::<f32>()), 0.0);
        }
        let norms = &norms[..size];
        let freqs = &freqs[..size];
        let scores = &mut scores[..size];
        let norm_inverses = &mut self.norm_inverses[..size];
        for (norm_inverse, norm) in norm_inverses.iter_mut().zip(norms) {
            *norm_inverse = self.scorer.cache[usize::from(*norm as u8)];
        }
        for ((score, freq), norm_inverse) in scores.iter_mut().zip(freqs).zip(norm_inverses.iter())
        {
            *score = self.scorer.do_score(*freq, *norm_inverse);
        }
    }
}

impl fmt::Display for BM25Similarity {
    /// Renders the similarity the way `BM25Similarity.toString()` does
    /// (`BM25Similarity.java:309-311`).
    ///
    /// Java concatenates the two `float`s, so the rendering is
    /// `Float.toString`'s and not Rust's: `k1 = 2` prints as `2.0`, not `2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BM25(k1={},b={})",
            java_fmt::float_to_string(self.k1),
            java_fmt::float_to_string(self.b)
        )
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
