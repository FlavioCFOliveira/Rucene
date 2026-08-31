//! Bayesian calibration parameter estimation, ported from
//! `org.apache.lucene.search.BayesianScoreEstimator`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{MultiTerms, Term};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::index_searcher::IndexSearcher;
use crate::search::query::Query;
use crate::search::term_query::TermQuery;
use crate::util::BytesRef;

/// The default number of pseudo-queries sampled.
///
/// Equivalent to `BayesianScoreEstimator.DEFAULT_N_SAMPLES`.
const DEFAULT_N_SAMPLES: i32 = 50;

/// The default number of indexed terms per pseudo-query.
///
/// Equivalent to `BayesianScoreEstimator.DEFAULT_TOKENS_PER_QUERY`.
const DEFAULT_TOKENS_PER_QUERY: i32 = 5;

/// The percentile above which a document counts towards the base rate.
///
/// Equivalent to `BayesianScoreEstimator.PERCENTILE_THRESHOLD`.
const PERCENTILE_THRESHOLD: f64 = 0.95;

/// The lowest base rate the estimator reports.
///
/// Equivalent to `BayesianScoreEstimator.BASE_RATE_MIN`.
const BASE_RATE_MIN: f32 = 1e-6;

/// The highest base rate the estimator reports.
///
/// Equivalent to `BayesianScoreEstimator.BASE_RATE_MAX`.
const BASE_RATE_MAX: f32 = 0.5;

/// The estimated parameters of a
/// [`BayesianScoreQuery`](crate::search::BayesianScoreQuery).
///
/// Equivalent to the record
/// `BayesianScoreEstimator.Parameters(float alpha, float beta, float baseRate)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Parameters {
    /// The sigmoid steepness.
    pub alpha: f32,
    /// The sigmoid midpoint.
    pub beta: f32,
    /// The corpus-level relevance prior.
    pub base_rate: f32,
}

impl Parameters {
    /// Creates a parameter triple.
    ///
    /// Equivalent to the record's canonical constructor.
    pub fn new(alpha: f32, beta: f32, base_rate: f32) -> Self {
        Self {
            alpha,
            beta,
            base_rate,
        }
    }
}

/// Estimates [`BayesianScoreQuery`](crate::search::BayesianScoreQuery)
/// parameters from corpus statistics through pseudo-query sampling.
///
/// Equivalent to `org.apache.lucene.search.BayesianScoreEstimator`, a class
/// with a private constructor and only static members; the port therefore has
/// no value of it and exposes the members as free functions of this module,
/// re-exported under the class's name.
///
/// The estimation algorithm:
///
/// 1. reservoir-sample terms from the target field's indexed vocabulary;
/// 2. partition the sampled terms into pseudo-queries;
/// 3. run each pseudo-query and collect the score distribution;
/// 4. estimate `beta = median(scores)` and `alpha = 1 / std(scores)`;
/// 5. estimate the base rate as the mean fraction of documents scoring above
///    the 95th percentile.
#[derive(Debug, Clone, Copy)]
pub struct BayesianScoreEstimator;

impl BayesianScoreEstimator {
    /// Estimates the parameters from the given index.
    ///
    /// Equivalent to
    /// `BayesianScoreEstimator.estimate(IndexSearcher, String, int, int, long)`.
    ///
    /// * `searcher` — the searcher to sample from;
    /// * `field` — the indexed text field to build pseudo-queries for;
    /// * `n_samples` — the number of pseudo-queries to sample;
    /// * `tokens_per_query` — the number of indexed terms per pseudo-query;
    /// * `seed` — the random seed, for reproducible sampling.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's messages — when
    /// `n_samples` or `tokens_per_query` is not positive, or when
    /// `n_samples * tokens_per_query` overflows, which is Java's
    /// `Math.multiplyExact`; propagates any I/O error raised while reading the
    /// index.
    pub fn estimate_with(
        searcher: &IndexSearcher,
        field: &str,
        n_samples: i32,
        tokens_per_query: i32,
        seed: i64,
    ) -> Result<Parameters> {
        if n_samples <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "nSamples must be positive, got {n_samples}"
            )));
        }
        if tokens_per_query <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "tokensPerQuery must be positive, got {tokens_per_query}"
            )));
        }

        let reader = searcher.get_index_reader();
        let max_doc = reader.max_doc();
        if max_doc == 0 {
            return Ok(Parameters::new(1.0, 0.0, 0.01));
        }

        let mut rng = JavaRandom::new(seed);
        let sample_size = n_samples.checked_mul(tokens_per_query).ok_or_else(|| {
            LuceneError::IllegalArgument("nSamples * tokensPerQuery overflows an int".to_string())
        })?;
        let sampled_terms = sample_vocabulary_terms(reader, field, sample_size, &mut rng)?;
        if sampled_terms.is_empty() {
            return Ok(Parameters::new(1.0, 0.0, 0.01));
        }

        // Create pseudo-queries from the indexed vocabulary terms and collect
        // their scores.
        let mut all_score_arrays: Vec<Vec<f32>> = Vec::new();
        let mut base_rate_fractions: Vec<f32> = Vec::new();

        let tokens_per_query = tokens_per_query as usize;
        let mut offset = 0usize;
        while offset < sampled_terms.len() {
            let mut builder = BooleanQueryBuilder::new();
            let end = (offset + tokens_per_query).min(sampled_terms.len());
            for term in &sampled_terms[offset..end] {
                builder.add(
                    Arc::new(TermQuery::new(Term::new(field, term.clone()))),
                    Occur::SHOULD,
                )?;
            }
            let pseudo_query: Arc<dyn Query> = Arc::new(builder.build());

            // Collect all scores.
            let scores = collect_scores(searcher, pseudo_query, max_doc)?;
            offset += tokens_per_query;
            if scores.is_empty() {
                continue;
            }

            // Base rate: the fraction of docs above the 95th percentile.
            let mut sorted = scores.clone();
            sorted.sort_by(java_float_compare);
            let mut p_idx = (sorted.len() as f64 * PERCENTILE_THRESHOLD) as usize;
            p_idx = p_idx.min(sorted.len() - 1);
            let threshold = sorted[p_idx];
            let high_count = scores.iter().filter(|s| **s >= threshold).count();
            base_rate_fractions.push(high_count as f32 / max_doc as f32);

            all_score_arrays.push(scores);
        }

        if all_score_arrays.is_empty() {
            return Ok(Parameters::new(1.0, 0.0, 0.01));
        }

        // Flatten all the scores for the global statistics.
        let mut all_scores: Vec<f32> = Vec::new();
        for array in &all_score_arrays {
            all_scores.extend_from_slice(array);
        }

        // beta = median
        all_scores.sort_by(java_float_compare);
        let beta = all_scores[all_scores.len() / 2];

        // alpha = 1 / std
        let mut mean = 0f64;
        for score in &all_scores {
            mean += *score as f64;
        }
        mean /= all_scores.len() as f64;
        let mut variance = 0f64;
        for score in &all_scores {
            let diff = *score as f64 - mean;
            variance += diff * diff;
        }
        variance /= all_scores.len() as f64;
        let std = variance.sqrt();
        let alpha = if std > 0.0 { (1.0 / std) as f32 } else { 1.0 };

        // base rate = the mean of the per-query fractions, clamped
        let mut base_rate = 0f32;
        for fraction in &base_rate_fractions {
            base_rate += fraction;
        }
        base_rate /= base_rate_fractions.len() as f32;
        base_rate = if base_rate.is_nan() {
            base_rate
        } else if base_rate < BASE_RATE_MIN {
            BASE_RATE_MIN
        } else if base_rate > BASE_RATE_MAX {
            BASE_RATE_MAX
        } else {
            base_rate
        };

        Ok(Parameters::new(alpha, beta, base_rate))
    }

    /// Estimates the parameters with the default settings: 50 samples, 5 tokens
    /// per query, seed 42.
    ///
    /// Equivalent to
    /// `BayesianScoreEstimator.estimate(IndexSearcher, String)`.
    ///
    /// # Errors
    ///
    /// As [`estimate_with`](Self::estimate_with).
    pub fn estimate(searcher: &IndexSearcher, field: &str) -> Result<Parameters> {
        Self::estimate_with(
            searcher,
            field,
            DEFAULT_N_SAMPLES,
            DEFAULT_TOKENS_PER_QUERY,
            42,
        )
    }
}

/// Reservoir-samples `sample_size` terms from a field's indexed vocabulary.
///
/// Equivalent to the package-private
/// `BayesianScoreEstimator.sampleVocabularyTerms(IndexReader, String, int, Random)`.
///
/// # Errors
///
/// Propagates any I/O error raised while iterating the terms.
pub fn sample_vocabulary_terms(
    reader: &Arc<dyn crate::index::IndexReader>,
    field: &str,
    sample_size: i32,
    rng: &mut JavaRandom,
) -> Result<Vec<BytesRef>> {
    let Some(terms) = MultiTerms::get_terms(reader, field)? else {
        return Ok(Vec::new());
    };

    let sample_size = sample_size.max(0) as usize;
    let mut reservoir: Vec<BytesRef> = Vec::with_capacity(sample_size);
    let mut terms_enum = terms.iterator()?;
    let mut seen: i64 = 0;
    while let Some(term) = terms_enum.next()? {
        seen += 1;
        if reservoir.len() < sample_size {
            reservoir.push(BytesRef::deep_copy_of(&term));
        } else {
            let replacement = next_long(rng, seen);
            if replacement < sample_size as i64 {
                reservoir[replacement as usize] = BytesRef::deep_copy_of(&term);
            }
        }
    }
    Ok(reservoir)
}

/// Equivalent to the private
/// `BayesianScoreEstimator.nextLong(Random, long)`, which is
/// `java.util.Random.nextLong(long)`'s rejection loop.
fn next_long(rng: &mut JavaRandom, bound: i64) -> i64 {
    loop {
        let bits = (rng.next_long() as u64 >> 1) as i64;
        let value = bits % bound;
        if bits.wrapping_sub(value).wrapping_add(bound - 1) >= 0 {
            return value;
        }
    }
}

/// Equivalent to the private
/// `BayesianScoreEstimator.collectScores(IndexSearcher, Query, int)`.
fn collect_scores(
    searcher: &IndexSearcher,
    query: Arc<dyn Query>,
    max_doc: i32,
) -> Result<Vec<f32>> {
    let top_n = max_doc.min(10_000);
    let top_docs = searcher.search(query, top_n)?;
    Ok(top_docs.score_docs.iter().map(|hit| hit.score).collect())
}

/// Orders floats exactly as `java.util.Arrays.sort(float[])` does, which is
/// `Float.compare`: every `NaN` is greater than every number, and `-0.0` sorts
/// before `0.0`.
fn java_float_compare(a: &f32, b: &f32) -> std::cmp::Ordering {
    if a < b {
        return std::cmp::Ordering::Less;
    }
    if a > b {
        return std::cmp::Ordering::Greater;
    }
    let a_bits = if a.is_nan() {
        0x7fc0_0000u32 as i32
    } else {
        a.to_bits() as i32
    };
    let b_bits = if b.is_nan() {
        0x7fc0_0000u32 as i32
    } else {
        b.to_bits() as i32
    };
    a_bits.cmp(&b_bits)
}

/// The `java.util.Random` linear congruential generator.
///
/// **Divergence from Lucene 10.5.0.** Java uses `java.util.Random` directly.
/// This crate has no port of it, and the estimator's contract is that a given
/// seed yields reproducible sampling, so the generator is transcribed here from
/// its specification (`java.util.Random`, "linear congruential pseudorandom
/// number generator"): the same seed therefore samples the same terms as Java
/// does.
#[derive(Debug, Clone)]
pub struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5DEEC_E66D;
    const ADDEND: i64 = 0xB;
    const MASK: i64 = (1 << 48) - 1;

    /// Creates a generator with the given seed.
    ///
    /// Equivalent to `new java.util.Random(long)`.
    pub fn new(seed: i64) -> Self {
        Self {
            seed: (seed ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    /// Equivalent to the protected `java.util.Random.next(int)`.
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND)
            & Self::MASK;
        ((self.seed as u64) >> (48 - bits)) as i32
    }

    /// Returns the next pseudorandom `long`.
    ///
    /// Equivalent to `java.util.Random.nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        ((self.next(32) as i64) << 32).wrapping_add(self.next(32) as i64)
    }
}
