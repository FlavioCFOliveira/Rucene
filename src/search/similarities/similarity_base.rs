//! The simplified similarity base class, ported from
//! `org.apache.lucene.search.similarities.SimilarityBase`.
//!
//! # How the abstract class becomes a trait
//!
//! Java's `SimilarityBase` is an abstract class that extends `Similarity`,
//! implements `scorer` as `final`, and leaves `score` and `toString` to its
//! descendants. Rust has no inheritance, so the shape here is:
//!
//! * [`SimilarityBase`] is a trait with [`Similarity`] as a supertrait. It
//!   carries the methods Java's descendants override — `score`, `newStats`,
//!   `fillBasicStats` and the two `explain` overloads — with the same defaults.
//! * The `final scorer` body becomes the free function
//!   [`similarity_base_scorer`]. Every concrete similarity forwards to it in
//!   one line from its `impl Similarity`, which is the closest Rust gets to
//!   inheriting a `final` method.
//! * `newStats` returns an associated [`SimilarityBase::Stats`] type instead of
//!   a `BasicStats` that Java then downcasts (`LMSimilarity.java:56`). The
//!   downcast is what the associated type replaces: the language-modelling
//!   family sets `Stats = LMStats` and reads its extra field without a cast.
//!
//! # Floating-point fidelity
//!
//! Every formula in this package is transcribed with Java's widening rules
//! preserved, because `float` and `double` arithmetic are not interchangeable
//! and a single misplaced widening silently reorders results. Where Java calls
//! `Math.log` or `Math.pow`, this port calls `f64::ln` or `f64::powf`. Neither
//! `Math.log` nor `Math.pow` is correctly rounded — the JDK allows one ulp of
//! error and delegates to the platform, exactly as Rust delegates to the
//! platform's libm — so results can differ in the last ulp between a JVM and
//! this crate, just as they can between two JVMs on different hosts. Lucene
//! itself would need `StrictMath` to avoid that, and does not use it.

#![deny(unsafe_code)]

use std::borrow::{Borrow, BorrowMut};
use std::sync::LazyLock;

use crate::util::SmallFloat;

use super::multi_similarity::MultiSimScorer;
use super::{BasicStats, CollectionStatistics, Explanation, SimScorer, Similarity, TermStatistics};

/// Cache of decoded norm bytes, as `SimilarityBase.LENGTH_TABLE`
/// (`SimilarityBase.java:158-164`).
///
/// Entry `i` is `SmallFloat.byte4ToInt((byte) i)` as a `float`; the `float` is
/// deliberate, because lengths above `2^24` lose precision there and the loss
/// is part of the scores Lucene produces.
static LENGTH_TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut table = [0.0f32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        *entry = SmallFloat::byte4_to_int(i as u8) as f32;
    }
    table
});

/// Looks up the decoded length of an encoded norm as a `float`.
///
/// This is the `LENGTH_TABLE[Byte.toUnsignedInt((byte) norm)]` lookup that
/// `SimilarityBase.BasicSimScorer.getLengthValue`
/// (`SimilarityBase.java:180-182`), `BM25Similarity.BM25Scorer.explainTF`
/// (`BM25Similarity.java:281`) and `TFIDFSimilarity` all perform. Java keeps
/// one private copy of the table per class, all filled identically; one shared
/// table produces the same values.
pub(crate) fn norm_length(norm: i64) -> f32 {
    LENGTH_TABLE[usize::from(norm as u8)]
}

/// Decodes an encoded norm into the field length the scoring formulas take.
///
/// Equivalent to `SimilarityBase.BasicSimScorer.getLengthValue(long)` followed
/// by the widening to `double` that `SimilarityBase.score` performs.
pub(crate) fn decode_norm(norm: i64) -> f64 {
    f64::from(norm_length(norm))
}

/// Returns `SmallFloat.byte4ToInt((byte) index)`.
///
/// `TFIDFSimilarity` caches this as an `int[]` rather than a `float[]`
/// (`TFIDFSimilarity.java:410-417`) because `TFIDFSimilarity.lengthNorm` takes
/// an `int`; the integer form is therefore kept separate from
/// [`norm_length`].
pub(crate) fn decoded_length(index: u8) -> i32 {
    SmallFloat::byte4_to_int(index)
}

/// Returns the base-two logarithm of `x`.
///
/// Equivalent to `SimilarityBase.log2(double)` (`SimilarityBase.java:169-172`),
/// which divides `Math.log(x)` by a precomputed `Math.log(2)`.
/// [`std::f64::consts::LN_2`] is the same `double`.
pub fn log2(x: f64) -> f64 {
    x.ln() / std::f64::consts::LN_2
}

/// Fills the fields every ranking method needs.
///
/// This is the body of `SimilarityBase.fillBasicStats`
/// (`SimilarityBase.java:88-101`), factored out so that
/// [`SimilarityBase::fill_basic_stats`] can keep its default and a similarity
/// that overrides it — as [`LMSimilarity`](super::LMSimilarity) does — can
/// still run it first, the way Java's `super.fillBasicStats` does.
///
/// Java asserts that the term statistics are within the collection statistics
/// before filling; the assertion is disabled in production and
/// [`TermStatistics`] and [`CollectionStatistics`] already validate their own
/// invariants on construction, so it is not reproduced.
pub fn fill_basic_stats(
    stats: &mut BasicStats,
    collection_stats: &CollectionStatistics,
    term_stats: &TermStatistics,
) {
    stats.set_number_of_documents(collection_stats.doc_count());
    stats.set_number_of_field_tokens(collection_stats.sum_total_term_freq());
    stats.set_avg_field_length(
        collection_stats.sum_total_term_freq() as f64 / collection_stats.doc_count() as f64,
    );
    stats.set_doc_freq(term_stats.doc_freq());
    stats.set_total_term_freq(term_stats.total_term_freq());
}

/// A similarity that computes its score from index statistics through a
/// simplified API.
///
/// Equivalent to `org.apache.lucene.search.similarities.SimilarityBase`.
/// Implementors must provide [`Self::score`], [`Self::simple_name`] and a
/// [`Display`](std::fmt::Display) rendering — Java declares `toString`
/// abstract on this class (`SimilarityBase.java:153`), which the supertrait
/// bound reproduces — and must forward [`Similarity::scorer`] to
/// [`similarity_base_scorer`].
///
/// Multi-word queries such as phrase queries are scored differently from
/// Lucene's default ranking: rather than faking an IDF for the phrase as a
/// whole, this family sums the individual term scores.
pub trait SimilarityBase: Similarity + std::fmt::Display {
    /// The statistics object this similarity fills and scores from.
    ///
    /// `BasicStats` for every similarity except the language-modelling family,
    /// which uses `LMStats`. Java expresses the same thing with a
    /// `newStats` factory plus a downcast in `score`.
    type Stats: Borrow<BasicStats> + BorrowMut<BasicStats> + From<BasicStats>;

    /// Returns the name this similarity puts into its explanations.
    ///
    /// Equivalent to `getClass().getSimpleName()`, which Java evaluates on the
    /// runtime class; Rust has no such reflection, so the name is declared.
    fn simple_name(&self) -> &'static str;

    /// Factory method returning a custom statistics object.
    ///
    /// Equivalent to `SimilarityBase.newStats(String, double)`
    /// (`SimilarityBase.java:82-84`).
    fn new_stats(&self, field: &str, boost: f64) -> Self::Stats {
        BasicStats::new(field, boost).into()
    }

    /// Fills all fields defined in [`BasicStats`].
    ///
    /// Equivalent to `SimilarityBase.fillBasicStats`
    /// (`SimilarityBase.java:88-101`). Implementors that add statistics of
    /// their own should call [`fill_basic_stats`] first, as Java's overrides
    /// call `super.fillBasicStats`.
    fn fill_basic_stats(
        &self,
        stats: &mut Self::Stats,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
    ) {
        fill_basic_stats(stats.borrow_mut(), collection_stats, term_stats);
    }

    /// Scores a document.
    ///
    /// Equivalent to the abstract `SimilarityBase.score(BasicStats, double,
    /// double)` (`SimilarityBase.java:112`). `freq` is the term frequency and
    /// `doc_len` the decoded document length.
    fn score(&self, stats: &Self::Stats, freq: f64, doc_len: f64) -> f64;

    /// Adds this similarity's own details to an explanation.
    ///
    /// Equivalent to `SimilarityBase.explain(List, BasicStats, double, double)`
    /// (`SimilarityBase.java:125-126`), whose default does nothing.
    fn explain_details(
        &self,
        sub_expls: &mut Vec<Explanation>,
        stats: &Self::Stats,
        freq: f64,
        doc_len: f64,
    ) {
        let _ = (sub_expls, stats, freq, doc_len);
    }

    /// Explains the score.
    ///
    /// Equivalent to `SimilarityBase.explain(BasicStats, Explanation, double)`
    /// (`SimilarityBase.java:140-148`), which renders
    /// `score(<name>, freq=<freq>), computed from:` over the details collected
    /// by [`Self::explain_details`].
    fn explain(&self, stats: &Self::Stats, freq: &Explanation, doc_len: f64) -> Explanation {
        let mut subs = Vec::new();
        // Java narrows the frequency to a `float` before widening it back to a
        // `double` for both calls, so the narrowing is reproduced here.
        let freq_value = f64::from(freq.value().float_value());
        self.explain_details(&mut subs, stats, freq_value, doc_len);

        Explanation::matched(
            self.score(stats, freq_value, doc_len) as f32,
            format!(
                "score({}, freq={}), computed from:",
                self.simple_name(),
                freq.value()
            ),
            subs,
        )
    }
}

/// Builds the scorer for a [`SimilarityBase`] descendant.
///
/// This is the body of `SimilarityBase.scorer`
/// (`SimilarityBase.java:64-77`), which Java declares `final`. One
/// `BasicSimScorer` is built per term; a single term returns its scorer
/// directly, and several terms are combined with the scorer of
/// [`MultiSimilarity`](super::MultiSimilarity), which sums them.
///
/// The returned scorer borrows `sim`, mirroring Java's inner class holding a
/// reference to its enclosing instance.
pub fn similarity_base_scorer<'a, S>(
    sim: &'a S,
    boost: f32,
    collection_stats: &CollectionStatistics,
    term_stats: &[TermStatistics],
) -> Box<dyn SimScorer + 'a>
where
    S: SimilarityBase + ?Sized,
{
    let mut weights: Vec<Box<dyn SimScorer + 'a>> = Vec::with_capacity(term_stats.len());
    for term in term_stats {
        let mut stats = sim.new_stats(collection_stats.field(), f64::from(boost));
        sim.fill_basic_stats(&mut stats, collection_stats, term);
        weights.push(Box::new(BasicSimScorer { base: sim, stats }));
    }
    if weights.len() == 1 {
        if let Some(single) = weights.pop() {
            return single;
        }
    }
    Box::new(MultiSimScorer::new(weights))
}

/// Delegates scoring and explaining to the enclosing [`SimilarityBase`].
///
/// Equivalent to `SimilarityBase.BasicSimScorer`
/// (`SimilarityBase.java:174-190`), which Java declares as a non-static inner
/// class so that it can reach `SimilarityBase.this`; the borrow here plays the
/// same role.
struct BasicSimScorer<'a, S: SimilarityBase + ?Sized> {
    base: &'a S,
    stats: S::Stats,
}

impl<S: SimilarityBase + ?Sized> SimScorer for BasicSimScorer<'_, S> {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        self.base
            .score(&self.stats, f64::from(freq), decode_norm(norm)) as f32
    }

    fn explain(&self, freq: &Explanation, norm: i64) -> Explanation {
        self.base.explain(&self.stats, freq, decode_norm(norm))
    }
}
