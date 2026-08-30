//! Scorer helpers, ported from `org.apache.lucene.search.ScorerUtil` and
//! `org.apache.lucene.search.DocAndScoreAccBuffer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::DocAndFloatFeatureBuffer;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorable::Scorable;
use crate::util::{Bits, MathUtil, PriorityQueue, PriorityQueueComparator};

/// Parallel arrays storing doc IDs and their corresponding score accumulators.
///
/// Equivalent to the `final org.apache.lucene.search.DocAndScoreAccBuffer`.
/// The scores are `f64` because they accumulate the contributions of several
/// clauses, which is where the extra precision matters.
#[derive(Debug, Default, Clone)]
pub struct DocAndScoreAccBuffer {
    /// Doc IDs.
    pub docs: Vec<i32>,
    /// Scores.
    pub scores: Vec<f64>,
    /// Number of valid entries in the doc ID and score arrays.
    pub size: usize,
}

impl DocAndScoreAccBuffer {
    /// Creates an empty buffer.
    ///
    /// Equivalent to `new DocAndScoreAccBuffer()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Grows both arrays so that they can store at least `min_size` entries.
    /// Existing content may be discarded.
    ///
    /// Equivalent to `DocAndScoreAccBuffer.growNoCopy(int)`.
    pub fn grow_no_copy(&mut self, min_size: usize) {
        if self.docs.len() < min_size {
            self.docs = vec![0; min_size];
            self.scores = vec![0.0; min_size];
        }
    }

    /// Grows both arrays so that they can store at least `min_size` entries,
    /// preserving the existing content.
    ///
    /// Equivalent to `DocAndScoreAccBuffer.grow(int)`.
    pub fn grow(&mut self, min_size: usize) {
        if self.docs.len() < min_size {
            self.docs.resize(min_size, 0);
            self.scores.resize(self.docs.len(), 0.0);
        }
    }

    /// Copies the content of a [`DocAndFloatFeatureBuffer`], widening the
    /// `f32` scores to `f64`.
    ///
    /// Equivalent to
    /// `DocAndScoreAccBuffer.copyFrom(DocAndFloatFeatureBuffer)`.
    pub fn copy_from(&mut self, buffer: &DocAndFloatFeatureBuffer) {
        self.grow_no_copy(buffer.size);
        self.docs[..buffer.size].copy_from_slice(&buffer.docs[..buffer.size]);
        for i in 0..buffer.size {
            self.scores[i] = f64::from(buffer.features[i]);
        }
        self.size = buffer.size;
    }
}

/// Orders costs so that the greatest is the top of the queue, which is what
/// [`ScorerUtil::cost_with_min_should_match`] needs to keep the least costly
/// clauses.
#[derive(Debug, Default, Clone, Copy)]
struct GreatestCostFirst;

impl PriorityQueueComparator<i64> for GreatestCostFirst {
    fn less_than(&self, a: &i64, b: &i64) -> bool {
        a > b
    }
}

/// Helpers shared by the scorer implementations.
///
/// Equivalent to `org.apache.lucene.search.ScorerUtil`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility.
#[derive(Debug, Clone, Copy)]
pub struct ScorerUtil;

impl ScorerUtil {
    /// Computes the cost of a disjunction that requires `min_should_match` of
    /// its `num_scorers` clauses to match.
    ///
    /// Equivalent to
    /// `ScorerUtil.costWithMinShouldMatch(LongStream, int, int)`.
    ///
    /// The reasoning is the following: a boolean query `c1, c2, ..., cn` with
    /// `minShouldMatch = m` could be rewritten to
    /// `(c1 AND (c2..cn|msm=m-1)) OR (!c1 AND (c2..cn|msm=m))`. Assuming the
    /// clauses come in ascending cost, the cost of the first part is the cost
    /// of `c1`, because the cost of a conjunction is the cost of its least
    /// costly clause; the cost of the second part is the cost of finding `m`
    /// matches among the remaining clauses. Since the whole is a disjunction,
    /// the total cost is the sum of the two. Recursing to the end shows that
    /// the cost is the sum of the costs of the
    /// `num_scorers - min_should_match + 1` least costly scorers.
    ///
    /// # Errors
    ///
    /// Propagates the [`PriorityQueue`] construction error when
    /// `num_scorers - min_should_match + 1` is not a size the queue can hold.
    pub fn cost_with_min_should_match(
        costs: impl IntoIterator<Item = i64>,
        num_scorers: usize,
        min_should_match: usize,
    ) -> Result<i64> {
        let capacity = num_scorers
            .checked_sub(min_should_match)
            .and_then(|slack| slack.checked_add(1))
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "minShouldMatch must not exceed numScorers, got minShouldMatch={min_should_match}, numScorers={num_scorers}"
                ))
            })?;
        let mut pq: PriorityQueue<i64, GreatestCostFirst> =
            PriorityQueue::new(capacity, GreatestCostFirst)?;
        for cost in costs {
            pq.insert_with_overflow(cost);
        }
        Ok(pq.into_iter().sum())
    }

    /// Optimises a [`DocIdSetIterator`] for the case when it is likely
    /// implemented over an [`ImpactsEnum`](crate::index::ImpactsEnum).
    ///
    /// Equivalent to `ScorerUtil.likelyImpactsEnum(DocIdSetIterator)`.
    ///
    /// **Divergence from Lucene 10.5.0.** The Java method wraps the iterator in
    /// a `FilterDocIdSetIterator` unless it is already one of two known classes,
    /// so that the JIT sees at most two receiver types at the call sites of
    /// `nextDoc`/`advance` and can inline them. That is a HotSpot
    /// profile-shaping trick with no counterpart in Rust, which resolves
    /// `dyn` calls through a vtable and gains nothing from a uniform receiver
    /// type — the extra wrapper would be pure overhead. This port therefore
    /// returns the iterator unchanged, which is semantically identical because
    /// `FilterDocIdSetIterator` is a transparent delegation.
    pub fn likely_impacts_enum(it: Box<dyn DocIdSetIterator>) -> Box<dyn DocIdSetIterator> {
        it
    }

    /// Optimises a [`Scorable`] for the case when it is likely implemented by a
    /// term scorer.
    ///
    /// Equivalent to `ScorerUtil.likelyTermScorer(Scorable)`; see
    /// [`likely_impacts_enum`](Self::likely_impacts_enum) for why this port
    /// returns its argument unchanged.
    pub fn likely_term_scorer(scorable: &mut dyn Scorable) -> &mut dyn Scorable {
        scorable
    }

    /// Optimises the [`Bits`] representing the set of accepted documents for
    /// the case when they are likely the segment's live docs.
    ///
    /// Equivalent to `ScorerUtil.likelyLiveDocs(Bits)`; see
    /// [`likely_impacts_enum`](Self::likely_impacts_enum) for why this port
    /// returns its argument unchanged.
    pub fn likely_live_docs(accept_docs: Option<&dyn Bits>) -> Option<&dyn Bits> {
        accept_docs
    }

    /// Computes a minimum required score such that
    /// `sum_upper_bound(min_required_score + max_remaining_score, num_scorers)`
    /// stays below `min_competitive_score`.
    ///
    /// Equivalent to `ScorerUtil.minRequiredScore(double, float, int)`. The
    /// computed value may not be the greatest that meets the condition, which
    /// means some documents may fail to be filtered out. That does not hurt
    /// correctness: those documents are simply filtered out later, and
    /// computing an optimal value would be unlikely to pay for itself.
    pub fn min_required_score(
        max_remaining_score: f64,
        min_competitive_score: f32,
        num_scorers: i32,
    ) -> f64 {
        let mut min_required_score = f64::from(min_competitive_score) - max_remaining_score;
        // Note: we want the float ulp in order to converge faster, not the
        // double ulp.
        let subtraction = f64::from(float_ulp(min_competitive_score));
        while min_required_score > 0.0
            && MathUtil::sum_upper_bound(min_required_score + max_remaining_score, num_scorers)
                as f32
                >= min_competitive_score
        {
            min_required_score -= subtraction;
        }
        min_required_score
    }

    /// Filters competitive hits from the provided buffer.
    ///
    /// Equivalent to
    /// `ScorerUtil.filterCompetitiveHits(DocAndScoreAccBuffer, double, float,
    /// int)`. It removes documents that cannot possibly have a score
    /// competitive enough to exceed the minimum competitive score, given the
    /// maximum remaining score and the number of scorers.
    ///
    /// **Divergence from Lucene 10.5.0.** Java delegates the compaction to
    /// `VectorUtil.filterByScore`, which dispatches to a SIMD implementation
    /// when one is available. This crate's `vector_util` does not expose that
    /// primitive, so the scalar loop that backs Java's default provider is
    /// spelled out here; the result is identical.
    pub fn filter_competitive_hits(
        buffer: &mut DocAndScoreAccBuffer,
        max_remaining_score: f64,
        min_competitive_score: f32,
        num_scorers: i32,
    ) {
        let min_required_score =
            Self::min_required_score(max_remaining_score, min_competitive_score, num_scorers);

        if min_required_score <= 0.0 {
            return;
        }

        let mut new_size = 0;
        for i in 0..buffer.size {
            let doc = buffer.docs[i];
            let score = buffer.scores[i];
            buffer.docs[new_size] = doc;
            buffer.scores[new_size] = score;
            if score >= min_required_score {
                new_size += 1;
            }
        }
        buffer.size = new_size;
    }

    /// Applies the given scorable as a required clause on the buffer, removing
    /// the documents it does not match and adding its scores to the ones held.
    ///
    /// Equivalent to
    /// `ScorerUtil.applyRequiredClause(DocAndScoreAccBuffer, DocIdSetIterator,
    /// Scorable)`. The buffer must contain doc IDs in sorted order, with no
    /// duplicates.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing or scoring.
    pub fn apply_required_clause(
        buffer: &mut DocAndScoreAccBuffer,
        iterator: &mut dyn DocIdSetIterator,
        scorable: &mut dyn Scorable,
    ) -> Result<()> {
        let mut intersection_size = 0;
        let mut cur_doc = iterator.doc_id();
        for i in 0..buffer.size {
            let target_doc = buffer.docs[i];
            if cur_doc < target_doc {
                cur_doc = iterator.advance(target_doc)?;
            }
            if cur_doc == target_doc {
                buffer.docs[intersection_size] = target_doc;
                buffer.scores[intersection_size] = buffer.scores[i] + f64::from(scorable.score()?);
                intersection_size += 1;
            }
        }
        buffer.size = intersection_size;
        Ok(())
    }

    /// Applies the given scorable as an optional clause on the buffer, adding
    /// its scores to the ones held.
    ///
    /// Equivalent to
    /// `ScorerUtil.applyOptionalClause(DocAndScoreAccBuffer, DocIdSetIterator,
    /// Scorable)`. The buffer must contain doc IDs in sorted order, with no
    /// duplicates.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing or scoring.
    pub fn apply_optional_clause(
        buffer: &mut DocAndScoreAccBuffer,
        iterator: &mut dyn DocIdSetIterator,
        scorable: &mut dyn Scorable,
    ) -> Result<()> {
        let mut cur_doc = iterator.doc_id();
        for i in 0..buffer.size {
            let target_doc = buffer.docs[i];
            if cur_doc < target_doc {
                cur_doc = iterator.advance(target_doc)?;
            }
            if cur_doc == target_doc {
                buffer.scores[i] += f64::from(scorable.score()?);
            }
        }
        Ok(())
    }
}

/// Reproduces `java.lang.Math.ulp(float)`: the size of an ulp of the argument.
///
/// Rust has no `f32::ulp`, and stepping the bit pattern overflows to infinity
/// at `f32::MAX`, so the JDK's exponent arithmetic is reproduced directly.
fn float_ulp(value: f32) -> f32 {
    if value.is_nan() || value.is_infinite() {
        return value.abs();
    }
    // Math.getExponent: the unbiased exponent, or MIN_EXPONENT - 1 for zero and
    // subnormal values.
    let exp = (((value.to_bits() >> 23) & 0xFF) as i32) - 127;
    if exp == -127 {
        // Zero or subnormal: Float.MIN_VALUE.
        return f32::from_bits(1);
    }
    // SIGNIFICAND_WIDTH - 1 == 23, MIN_EXPONENT == -126.
    let exp = exp - 23;
    if exp >= -126 {
        f32::from_bits(((exp + 127) as u32) << 23)
    } else {
        f32::from_bits(1u32 << (exp + 149))
    }
}
