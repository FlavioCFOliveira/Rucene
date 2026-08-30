//! The axiomatic approaches to information retrieval, ported from
//! `org.apache.lucene.search.similarities.Axiomatic` and its six
//! F1/F2/F3 × EXP/LOG variants.
//!
//! The six variants share most of their components, and Java shares them by
//! repeating the bodies across the six subclasses. Here each formula is
//! transcribed once into a private free function and the six variants select
//! among them, so that a component used by three variants cannot drift between
//! them.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};

use super::similarity_base::similarity_base_scorer;
use super::{java_fmt, java_math};
use super::{
    BasicStats, CollectionStatistics, Explanation, SimScorer, Similarity, SimilarityBase,
    TermStatistics,
};

/// Default hyper-parameter of the primitive weighting function, as in every
/// Java constructor that leaves `k` out (`Axiomatic.java:73`).
const DEFAULT_K: f32 = 0.35;
/// Default hyper-parameter of the growth function, as in `new Axiomatic()`
/// (`Axiomatic.java:85`).
const DEFAULT_S: f32 = 0.25;
/// Default query length, as in every Java constructor that leaves it out.
const DEFAULT_QUERY_LEN: i32 = 1;

/// The three hyper-parameters `Axiomatic` holds as `protected final` fields,
/// plus the `discountOverlaps` flag its constructor forwards to `Similarity`.
///
/// Java repeats these fields on the abstract base; the six variants here hold
/// this struct instead, so that the validation Java performs in
/// `Axiomatic(boolean, float, int, float)` is written once.
#[derive(Debug, Clone, Copy, PartialEq)]
struct AxiomaticParams {
    s: f32,
    k: f32,
    query_len: i32,
    discount_overlaps: bool,
}

impl AxiomaticParams {
    /// Validates and stores the hyper-parameters.
    ///
    /// This is the body of `Axiomatic(boolean, float, int, float)`
    /// (`Axiomatic.java:57-75`). Java tests `Float.isFinite(x) == false ||
    /// Float.isNaN(x)`, which is redundant — a NaN is not finite — and is
    /// collapsed here.
    fn new(discount_overlaps: bool, s: f32, query_len: i32, k: f32) -> Result<Self> {
        if !s.is_finite() || !(0.0..=1.0).contains(&s) {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal s value: {}, must be between 0 and 1",
                java_fmt::float_to_string(s)
            )));
        }
        if !k.is_finite() || !(0.0..=1.0).contains(&k) {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal k value: {}, must be between 0 and 1",
                java_fmt::float_to_string(k)
            )));
        }
        if query_len < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal query length value: {query_len}, must be larger 0"
            )));
        }
        Ok(Self {
            s,
            k,
            query_len,
            discount_overlaps,
        })
    }
}

impl Default for AxiomaticParams {
    /// Equivalent to `new Axiomatic()` (`Axiomatic.java:84-86`).
    fn default() -> Self {
        Self {
            s: DEFAULT_S,
            k: DEFAULT_K,
            query_len: DEFAULT_QUERY_LEN,
            discount_overlaps: true,
        }
    }
}

/// Axiomatic approaches for information retrieval.
///
/// Equivalent to `org.apache.lucene.search.similarities.Axiomatic`. From Hui
/// Fang and Chengxiang Zhai, *An Exploration of Axiomatic Approaches to
/// Information Retrieval*, SIGIR '05, pp. 480-487.
///
/// The family is based on BM25, pivoted document length normalization and the
/// language model with Dirichlet prior; some components — term frequency,
/// inverse document frequency — are modified so that they satisfy axiomatic
/// constraints.
///
/// Implementors forward [`SimilarityBase::score`] to [`axiomatic_score`],
/// [`SimilarityBase::explain_details`] to [`axiomatic_explain_details`], and
/// [`SimilarityBase::explain`] to [`axiomatic_explain`]; those three are what
/// Java inherits from the abstract class.
pub trait Axiomatic: SimilarityBase<Stats = BasicStats> {
    /// Returns the hyper-parameter of the growth function.
    ///
    /// Java holds it in the `protected final float s` field
    /// (`Axiomatic.java:33`).
    fn s(&self) -> f32;

    /// Returns the hyper-parameter of the primitive weighting function.
    ///
    /// Java holds it in the `protected final float k` field
    /// (`Axiomatic.java:36`).
    fn k(&self) -> f32;

    /// Returns the query length.
    ///
    /// Java holds it in the `protected final int queryLen` field
    /// (`Axiomatic.java:39`).
    fn query_len(&self) -> i32;

    /// Computes the term frequency component.
    ///
    /// Equivalent to the abstract `Axiomatic.tf` (`Axiomatic.java:148`).
    fn tf(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64;

    /// Computes the document length component.
    ///
    /// Equivalent to the abstract `Axiomatic.ln` (`Axiomatic.java:151`).
    fn ln(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64;

    /// Computes the mixed term frequency and document length component.
    ///
    /// Equivalent to the abstract `Axiomatic.tfln` (`Axiomatic.java:154`).
    fn tfln(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64;

    /// Computes the inverted document frequency component.
    ///
    /// Equivalent to the abstract `Axiomatic.idf` (`Axiomatic.java:157`).
    fn idf(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64;

    /// Computes the gamma component, which only the F3 variants use.
    ///
    /// Equivalent to the abstract `Axiomatic.gamma` (`Axiomatic.java:160`).
    fn gamma(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64;

    /// Explains the term frequency component.
    ///
    /// Equivalent to the abstract `Axiomatic.tfExplain`
    /// (`Axiomatic.java:170`).
    fn tf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation;

    /// Explains the document length component.
    ///
    /// Equivalent to the abstract `Axiomatic.lnExplain`
    /// (`Axiomatic.java:180`).
    fn ln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation;

    /// Explains the mixed term frequency and document length component.
    ///
    /// Equivalent to the abstract `Axiomatic.tflnExplain`
    /// (`Axiomatic.java:191`).
    fn tfln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation;

    /// Explains the inverted document frequency component.
    ///
    /// Equivalent to the abstract `Axiomatic.idfExplain`
    /// (`Axiomatic.java:201`).
    fn idf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation;
}

/// Scores a document with the axiomatic formula.
///
/// This is the body of `Axiomatic.score` (`Axiomatic.java:88-98`): the product
/// of the four components minus gamma, boosted, and floored at zero — the F3
/// variants can produce negative scores through their gamma component.
pub fn axiomatic_score<A: Axiomatic + ?Sized>(
    similarity: &A,
    stats: &BasicStats,
    freq: f64,
    doc_len: f64,
) -> f64 {
    let mut score = similarity.tf(stats, freq, doc_len)
        * similarity.ln(stats, freq, doc_len)
        * similarity.tfln(stats, freq, doc_len)
        * similarity.idf(stats, freq, doc_len)
        - similarity.gamma(stats, freq, doc_len);
    score *= stats.boost();
    java_math::max_f64(0.0, score)
}

/// Adds the axiomatic components to an explanation.
///
/// This is the body of `Axiomatic.explain(List, BasicStats, double, double)`
/// (`Axiomatic.java:128-142`). Its trailing `super.explain` call reaches
/// `SimilarityBase`'s empty implementation and adds nothing.
pub fn axiomatic_explain_details<A: Axiomatic + ?Sized>(
    similarity: &A,
    subs: &mut Vec<Explanation>,
    stats: &BasicStats,
    freq: f64,
    doc_len: f64,
) {
    if stats.boost() != 1.0 {
        subs.push(Explanation::matched(
            stats.boost() as f32,
            "boost, query boost",
            vec![],
        ));
    }

    subs.push(Explanation::matched(
        similarity.k(),
        "k, hyperparam for the primitive weighting function",
        vec![],
    ));
    subs.push(Explanation::matched(
        similarity.s(),
        "s, hyperparam for the growth function",
        vec![],
    ));
    // Java passes the `int` query length, which renders without a fraction.
    subs.push(Explanation::matched(
        similarity.query_len(),
        "queryLen, query length",
        vec![],
    ));
    subs.push(similarity.tf_explain(stats, freq, doc_len));
    subs.push(similarity.ln_explain(stats, freq, doc_len));
    subs.push(similarity.tfln_explain(stats, freq, doc_len));
    subs.push(similarity.idf_explain(stats, freq, doc_len));
    subs.push(Explanation::matched(
        similarity.gamma(stats, freq, doc_len) as f32,
        "gamma",
        vec![],
    ));
}

/// Explains the axiomatic score.
///
/// This is the body of `Axiomatic.explain(BasicStats, Explanation, double)`
/// (`Axiomatic.java:100-126`). Unlike the other similarities in this package it
/// wraps rather than replaces: the unboosted score is explained first, then
/// wrapped in a boost node when the boost is not `1`, then wrapped again in a
/// `max of:` node when the raw score is negative.
pub fn axiomatic_explain<A: Axiomatic + ?Sized>(
    similarity: &A,
    stats: &BasicStats,
    freq: &Explanation,
    doc_len: f64,
) -> Explanation {
    let mut subs = Vec::new();
    let f = freq.value().double_value();
    axiomatic_explain_details(similarity, &mut subs, stats, f, doc_len);

    let score = similarity.tf(stats, f, doc_len)
        * similarity.ln(stats, f, doc_len)
        * similarity.tfln(stats, f, doc_len)
        * similarity.idf(stats, f, doc_len)
        - similarity.gamma(stats, f, doc_len);

    let mut explanation = Explanation::matched(
        score as f32,
        format!(
            "score({}, freq={}), computed from:",
            similarity.simple_name(),
            freq.value()
        ),
        subs,
    );
    if stats.boost() != 1.0 {
        explanation = Explanation::matched(
            (score * stats.boost()) as f32,
            "Boosted score, computed as (score * boost) from:",
            vec![
                explanation,
                Explanation::matched(stats.boost() as f32, "Query boost", vec![]),
            ],
        );
    }
    if score < 0.0 {
        explanation = Explanation::matched(
            0,
            "max of:",
            vec![
                Explanation::matched(0, "Minimum legal score", vec![]),
                explanation,
            ],
        );
    }
    explanation
}

// ---------------------------------------------------------------------------
// Shared components
// ---------------------------------------------------------------------------

/// `1 + log(1 + log(freq + 1))`, the term frequency component of the F1 and F3
/// variants.
///
/// The `+ 1` guards against the negative scores the raw formula produces for
/// frequencies below one.
fn tf_log_log(freq: f64) -> f64 {
    let freq = freq + 1.0;
    1.0 + (1.0 + freq.ln()).ln()
}

/// `(avgdl + s) / (avgdl + dl * s)`, the document length component of the F1
/// variants.
fn ln_pivoted(stats: &BasicStats, s: f32, doc_len: f64) -> f64 {
    let s = f64::from(s);
    (stats.avg_field_length() + s) / (stats.avg_field_length() + doc_len * s)
}

/// `freq / (freq + s + s * dl / avgdl)`, the mixed component of the F2
/// variants.
fn tfln_pivoted(stats: &BasicStats, s: f32, freq: f64, doc_len: f64) -> f64 {
    let s = f64::from(s);
    freq / (freq + s + s * doc_len / stats.avg_field_length())
}

/// `pow((N + 1) / n, k)`, the inverted document frequency of the EXP variants.
fn idf_exp(stats: &BasicStats, k: f32) -> f64 {
    ((stats.number_of_documents() as f64 + 1.0) / stats.doc_freq() as f64).powf(f64::from(k))
}

/// `log((N + 1) / n)`, the inverted document frequency of the LOG variants.
fn idf_log(stats: &BasicStats) -> f64 {
    ((stats.number_of_documents() as f64 + 1.0) / stats.doc_freq() as f64).ln()
}

/// `(dl - queryLen) * s * queryLen / avgdl`, the gamma component of the F3
/// variants. It is what makes their raw scores able to go negative.
fn gamma_f3(stats: &BasicStats, s: f32, query_len: i32, doc_len: f64) -> f64 {
    (doc_len - f64::from(query_len)) * f64::from(s) * f64::from(query_len)
        / stats.avg_field_length()
}

/// The `tfExplain` the F1 and F3 variants share.
fn tf_log_log_explain(value: f64, freq: f64) -> Explanation {
    Explanation::matched(
        value as f32,
        "tf, term frequency computed as 1 + log(1 + log(freq)) from:",
        vec![Explanation::matched(
            freq as f32,
            "freq, number of occurrences of term in the document",
            vec![],
        )],
    )
}

/// The `lnExplain` the F1 variants share.
fn ln_pivoted_explain(value: f64, stats: &BasicStats, doc_len: f64) -> Explanation {
    Explanation::matched(
        value as f32,
        "ln, document length computed as (avgdl + s) / (avgdl + dl * s) from:",
        vec![
            Explanation::matched(
                stats.avg_field_length() as f32,
                "avgdl, average length of field across all documents",
                vec![],
            ),
            Explanation::matched(doc_len as f32, "dl, length of field", vec![]),
        ],
    )
}

/// The `tflnExplain` the F2 variants share.
fn tfln_pivoted_explain(value: f64, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
    Explanation::matched(
        value as f32,
        "tfln, mixed term frequency and document length, computed as freq / (freq + s + s * dl / avgdl) from:",
        vec![
            Explanation::matched(
                freq as f32,
                "freq, number of occurrences of term in the document",
                vec![],
            ),
            Explanation::matched(doc_len as f32, "dl, length of field", vec![]),
            Explanation::matched(
                stats.avg_field_length() as f32,
                "avgdl, average length of field across all documents",
                vec![],
            ),
        ],
    )
}

/// The `idfExplain` the EXP variants share.
fn idf_exp_explain(value: f64, stats: &BasicStats) -> Explanation {
    Explanation::matched(
        value as f32,
        "idf, inverted document frequency computed as Math.pow((N + 1) / n, k) from:",
        vec![
            Explanation::matched(
                stats.number_of_documents() as f32,
                "N, total number of documents with field",
                vec![],
            ),
            Explanation::matched(
                stats.doc_freq() as f32,
                "n, number of documents containing term",
                vec![],
            ),
        ],
    )
}

/// The `idfExplain` the LOG variants share.
fn idf_log_explain(value: f64, stats: &BasicStats) -> Explanation {
    Explanation::matched(
        value as f32,
        "idf, inverted document frequency computed as log((N + 1) / n) from:",
        vec![
            Explanation::matched(
                stats.number_of_documents() as f32,
                "N, total number of documents with field",
                vec![],
            ),
            Explanation::matched(
                stats.doc_freq() as f32,
                "n, number of documents containing term",
                vec![],
            ),
        ],
    )
}

/// The constant `tfln` explanation of the F1 and F3 variants.
fn tfln_one_explain(value: f64) -> Explanation {
    Explanation::matched(
        value as f32,
        "tfln, mixed term frequency and document length, equals to 1",
        vec![],
    )
}

/// The constant `tf` explanation of the F2 variants.
fn tf_one_explain(value: f64) -> Explanation {
    Explanation::matched(value as f32, "tf, term frequency, equals to 1", vec![])
}

/// The constant `ln` explanation of the F2 and F3 variants.
fn ln_one_explain(value: f64) -> Explanation {
    Explanation::matched(value as f32, "ln, document length, equals to 1", vec![])
}

/// Generates the boilerplate every variant repeats: the [`Similarity`] and
/// [`SimilarityBase`] impls that forward to the three `axiomatic_*` bodies,
/// the three hyper-parameter accessors, and the `Display` rendering.
macro_rules! axiomatic_boilerplate {
    ($ty:ident, $name:literal) => {
        impl Similarity for $ty {
            fn discount_overlaps(&self) -> bool {
                self.params.discount_overlaps
            }

            fn scorer<'a>(
                &'a self,
                boost: f32,
                collection_stats: &CollectionStatistics,
                term_stats: &[TermStatistics],
            ) -> Box<dyn SimScorer + 'a> {
                similarity_base_scorer(self, boost, collection_stats, term_stats)
            }
        }

        impl SimilarityBase for $ty {
            type Stats = BasicStats;

            fn simple_name(&self) -> &'static str {
                stringify!($ty)
            }

            fn score(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64 {
                axiomatic_score(self, stats, freq, doc_len)
            }

            fn explain_details(
                &self,
                subs: &mut Vec<Explanation>,
                stats: &BasicStats,
                freq: f64,
                doc_len: f64,
            ) {
                axiomatic_explain_details(self, subs, stats, freq, doc_len);
            }

            fn explain(&self, stats: &BasicStats, freq: &Explanation, doc_len: f64) -> Explanation {
                axiomatic_explain(self, stats, freq, doc_len)
            }
        }

        impl fmt::Display for $ty {
            /// Renders the name of the axiomatic method, as the Java
            /// `toString()` override does.
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str($name)
            }
        }
    };
}

// ---------------------------------------------------------------------------
// F1EXP
// ---------------------------------------------------------------------------

/// `Sum(tf(term_doc_freq) * ln(docLen) * IDF(term))` with
/// `IDF(t) = pow((N + 1) / df(t), k)`.
///
/// Equivalent to `org.apache.lucene.search.similarities.AxiomaticF1EXP`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxiomaticF1EXP {
    params: AxiomaticParams,
}

impl AxiomaticF1EXP {
    /// Creates the similarity with the default `s = 0.25`, `queryLen = 1` and
    /// `k = 0.35`.
    ///
    /// Equivalent to `new AxiomaticF1EXP()` (`AxiomaticF1EXP.java:44-46`).
    pub fn new() -> Self {
        Self {
            params: AxiomaticParams::default(),
        }
    }

    /// Creates the similarity with the supplied `s`, leaving `k` and `queryLen`
    /// at their defaults.
    ///
    /// Equivalent to `new AxiomaticF1EXP(float)`
    /// (`AxiomaticF1EXP.java:38-40`).
    ///
    /// # Errors
    ///
    /// See [`Self::with_s_and_k`].
    pub fn with_s(s: f32) -> Result<Self> {
        Self::with_s_and_k(s, DEFAULT_K)
    }

    /// Creates the similarity with the supplied `s` and `k`, leaving `queryLen`
    /// at its default.
    ///
    /// Equivalent to `new AxiomaticF1EXP(float, float)`
    /// (`AxiomaticF1EXP.java:29-31`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `s` or `k` is not finite
    /// or lies outside `[0, 1]`.
    pub fn with_s_and_k(s: f32, k: f32) -> Result<Self> {
        Ok(Self {
            params: AxiomaticParams::new(true, s, DEFAULT_QUERY_LEN, k)?,
        })
    }
}

impl Default for AxiomaticF1EXP {
    fn default() -> Self {
        Self::new()
    }
}

impl Axiomatic for AxiomaticF1EXP {
    fn s(&self) -> f32 {
        self.params.s
    }

    fn k(&self) -> f32 {
        self.params.k
    }

    fn query_len(&self) -> i32 {
        self.params.query_len
    }

    fn tf(&self, _stats: &BasicStats, freq: f64, _doc_len: f64) -> f64 {
        tf_log_log(freq)
    }

    fn ln(&self, stats: &BasicStats, _freq: f64, doc_len: f64) -> f64 {
        ln_pivoted(stats, self.params.s, doc_len)
    }

    fn tfln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn idf(&self, stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        idf_exp(stats, self.params.k)
    }

    fn gamma(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        0.0
    }

    fn tf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tf_log_log_explain(self.tf(stats, freq, doc_len), freq)
    }

    fn ln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        ln_pivoted_explain(self.ln(stats, freq, doc_len), stats, doc_len)
    }

    fn tfln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tfln_one_explain(self.tfln(stats, freq, doc_len))
    }

    fn idf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        idf_exp_explain(self.idf(stats, freq, doc_len), stats)
    }
}

axiomatic_boilerplate!(AxiomaticF1EXP, "F1EXP");

// ---------------------------------------------------------------------------
// F1LOG
// ---------------------------------------------------------------------------

/// `Sum(tf(term_doc_freq) * ln(docLen) * IDF(term))` with
/// `IDF(t) = ln((N + 1) / df(t))`.
///
/// Equivalent to `org.apache.lucene.search.similarities.AxiomaticF1LOG`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxiomaticF1LOG {
    params: AxiomaticParams,
}

impl AxiomaticF1LOG {
    /// Creates the similarity with the default `s = 0.25`, `queryLen = 1` and
    /// `k = 0.35`.
    ///
    /// Equivalent to `new AxiomaticF1LOG()` (`AxiomaticF1LOG.java:37-39`).
    pub fn new() -> Self {
        Self {
            params: AxiomaticParams::default(),
        }
    }

    /// Creates the similarity with the supplied `s`, leaving `k` and `queryLen`
    /// at their defaults.
    ///
    /// Equivalent to `new AxiomaticF1LOG(float)`
    /// (`AxiomaticF1LOG.java:31-33`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `s` is not finite or lies
    /// outside `[0, 1]`.
    pub fn with_s(s: f32) -> Result<Self> {
        Ok(Self {
            params: AxiomaticParams::new(true, s, DEFAULT_QUERY_LEN, DEFAULT_K)?,
        })
    }
}

impl Default for AxiomaticF1LOG {
    fn default() -> Self {
        Self::new()
    }
}

impl Axiomatic for AxiomaticF1LOG {
    fn s(&self) -> f32 {
        self.params.s
    }

    fn k(&self) -> f32 {
        self.params.k
    }

    fn query_len(&self) -> i32 {
        self.params.query_len
    }

    fn tf(&self, _stats: &BasicStats, freq: f64, _doc_len: f64) -> f64 {
        tf_log_log(freq)
    }

    fn ln(&self, stats: &BasicStats, _freq: f64, doc_len: f64) -> f64 {
        ln_pivoted(stats, self.params.s, doc_len)
    }

    fn tfln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn idf(&self, stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        idf_log(stats)
    }

    fn gamma(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        0.0
    }

    fn tf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tf_log_log_explain(self.tf(stats, freq, doc_len), freq)
    }

    fn ln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        ln_pivoted_explain(self.ln(stats, freq, doc_len), stats, doc_len)
    }

    fn tfln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tfln_one_explain(self.tfln(stats, freq, doc_len))
    }

    fn idf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        idf_log_explain(self.idf(stats, freq, doc_len), stats)
    }
}

axiomatic_boilerplate!(AxiomaticF1LOG, "F1LOG");

// ---------------------------------------------------------------------------
// F2EXP
// ---------------------------------------------------------------------------

/// `Sum(tfln(term_doc_freq, docLen) * IDF(term))` with
/// `IDF(t) = pow((N + 1) / df(t), k)`.
///
/// Equivalent to `org.apache.lucene.search.similarities.AxiomaticF2EXP`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxiomaticF2EXP {
    params: AxiomaticParams,
}

impl AxiomaticF2EXP {
    /// Creates the similarity with the default `s = 0.25`, `queryLen = 1` and
    /// `k = 0.35`.
    ///
    /// Equivalent to `new AxiomaticF2EXP()` (`AxiomaticF2EXP.java:44-46`).
    pub fn new() -> Self {
        Self {
            params: AxiomaticParams::default(),
        }
    }

    /// Creates the similarity with the supplied `s`, leaving `k` and `queryLen`
    /// at their defaults.
    ///
    /// Equivalent to `new AxiomaticF2EXP(float)`
    /// (`AxiomaticF2EXP.java:38-40`).
    ///
    /// # Errors
    ///
    /// See [`Self::with_s_and_k`].
    pub fn with_s(s: f32) -> Result<Self> {
        Self::with_s_and_k(s, DEFAULT_K)
    }

    /// Creates the similarity with the supplied `s` and `k`, leaving `queryLen`
    /// at its default.
    ///
    /// Equivalent to `new AxiomaticF2EXP(float, float)`
    /// (`AxiomaticF2EXP.java:29-31`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `s` or `k` is not finite
    /// or lies outside `[0, 1]`.
    pub fn with_s_and_k(s: f32, k: f32) -> Result<Self> {
        Ok(Self {
            params: AxiomaticParams::new(true, s, DEFAULT_QUERY_LEN, k)?,
        })
    }
}

impl Default for AxiomaticF2EXP {
    fn default() -> Self {
        Self::new()
    }
}

impl Axiomatic for AxiomaticF2EXP {
    fn s(&self) -> f32 {
        self.params.s
    }

    fn k(&self) -> f32 {
        self.params.k
    }

    fn query_len(&self) -> i32 {
        self.params.query_len
    }

    fn tf(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn ln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn tfln(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64 {
        tfln_pivoted(stats, self.params.s, freq, doc_len)
    }

    fn idf(&self, stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        idf_exp(stats, self.params.k)
    }

    fn gamma(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        0.0
    }

    fn tf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tf_one_explain(self.tf(stats, freq, doc_len))
    }

    fn ln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        ln_one_explain(self.ln(stats, freq, doc_len))
    }

    fn tfln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tfln_pivoted_explain(self.tfln(stats, freq, doc_len), stats, freq, doc_len)
    }

    fn idf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        idf_exp_explain(self.idf(stats, freq, doc_len), stats)
    }
}

axiomatic_boilerplate!(AxiomaticF2EXP, "F2EXP");

// ---------------------------------------------------------------------------
// F2LOG
// ---------------------------------------------------------------------------

/// `Sum(tfln(term_doc_freq, docLen) * IDF(term))` with
/// `IDF(t) = ln((N + 1) / df(t))`.
///
/// Equivalent to `org.apache.lucene.search.similarities.AxiomaticF2LOG`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxiomaticF2LOG {
    params: AxiomaticParams,
}

impl AxiomaticF2LOG {
    /// Creates the similarity with the default `s = 0.25`, `queryLen = 1` and
    /// `k = 0.35`.
    ///
    /// Equivalent to `new AxiomaticF2LOG()` (`AxiomaticF2LOG.java:37-39`).
    pub fn new() -> Self {
        Self {
            params: AxiomaticParams::default(),
        }
    }

    /// Creates the similarity with the supplied `s`, leaving `k` and `queryLen`
    /// at their defaults.
    ///
    /// Equivalent to `new AxiomaticF2LOG(float)`
    /// (`AxiomaticF2LOG.java:31-33`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `s` is not finite or lies
    /// outside `[0, 1]`.
    pub fn with_s(s: f32) -> Result<Self> {
        Ok(Self {
            params: AxiomaticParams::new(true, s, DEFAULT_QUERY_LEN, DEFAULT_K)?,
        })
    }
}

impl Default for AxiomaticF2LOG {
    fn default() -> Self {
        Self::new()
    }
}

impl Axiomatic for AxiomaticF2LOG {
    fn s(&self) -> f32 {
        self.params.s
    }

    fn k(&self) -> f32 {
        self.params.k
    }

    fn query_len(&self) -> i32 {
        self.params.query_len
    }

    fn tf(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn ln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn tfln(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64 {
        tfln_pivoted(stats, self.params.s, freq, doc_len)
    }

    fn idf(&self, stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        idf_log(stats)
    }

    fn gamma(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        0.0
    }

    fn tf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tf_one_explain(self.tf(stats, freq, doc_len))
    }

    fn ln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        ln_one_explain(self.ln(stats, freq, doc_len))
    }

    fn tfln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tfln_pivoted_explain(self.tfln(stats, freq, doc_len), stats, freq, doc_len)
    }

    fn idf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        idf_log_explain(self.idf(stats, freq, doc_len), stats)
    }
}

axiomatic_boilerplate!(AxiomaticF2LOG, "F2LOG");

// ---------------------------------------------------------------------------
// F3EXP
// ---------------------------------------------------------------------------

/// `Sum(tf(term_doc_freq) * IDF(term) - gamma(docLen, queryLen))` with
/// `IDF(t) = pow((N + 1) / df(t), k)` and
/// `gamma(docLen, queryLen) = (docLen - queryLen) * queryLen * s / avgdl`.
///
/// Equivalent to `org.apache.lucene.search.similarities.AxiomaticF3EXP`. Note
/// that the gamma component of this similarity creates negative scores, which
/// [`axiomatic_score`] floors at zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxiomaticF3EXP {
    params: AxiomaticParams,
}

impl AxiomaticF3EXP {
    /// Creates the similarity with the supplied `s` and `queryLen`, leaving `k`
    /// at its default.
    ///
    /// Equivalent to `new AxiomaticF3EXP(float, int)`
    /// (`AxiomaticF3EXP.java:35-37`).
    ///
    /// # Errors
    ///
    /// See [`Self::with_parameters`].
    pub fn with_s_and_query_len(s: f32, query_len: i32) -> Result<Self> {
        Self::with_parameters(s, query_len, DEFAULT_K)
    }

    /// Creates the similarity with every hyper-parameter supplied.
    ///
    /// Equivalent to `new AxiomaticF3EXP(float, int, float)`
    /// (`AxiomaticF3EXP.java:26-28`). Java gives this variant no default
    /// constructor, because the query length has no sensible default.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `s` or `k` is not finite
    /// or lies outside `[0, 1]`, or when `query_len` is negative.
    pub fn with_parameters(s: f32, query_len: i32, k: f32) -> Result<Self> {
        Ok(Self {
            params: AxiomaticParams::new(true, s, query_len, k)?,
        })
    }
}

impl Axiomatic for AxiomaticF3EXP {
    fn s(&self) -> f32 {
        self.params.s
    }

    fn k(&self) -> f32 {
        self.params.k
    }

    fn query_len(&self) -> i32 {
        self.params.query_len
    }

    fn tf(&self, _stats: &BasicStats, freq: f64, _doc_len: f64) -> f64 {
        tf_log_log(freq)
    }

    fn ln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn tfln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn idf(&self, stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        idf_exp(stats, self.params.k)
    }

    fn gamma(&self, stats: &BasicStats, _freq: f64, doc_len: f64) -> f64 {
        gamma_f3(stats, self.params.s, self.params.query_len, doc_len)
    }

    fn tf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tf_log_log_explain(self.tf(stats, freq, doc_len), freq)
    }

    fn ln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        ln_one_explain(self.ln(stats, freq, doc_len))
    }

    fn tfln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tfln_one_explain(self.tfln(stats, freq, doc_len))
    }

    fn idf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        idf_exp_explain(self.idf(stats, freq, doc_len), stats)
    }
}

axiomatic_boilerplate!(AxiomaticF3EXP, "F3EXP");

// ---------------------------------------------------------------------------
// F3LOG
// ---------------------------------------------------------------------------

/// `Sum(tf(term_doc_freq) * IDF(term) - gamma(docLen, queryLen))` with
/// `IDF(t) = ln((N + 1) / df(t))` and
/// `gamma(docLen, queryLen) = (docLen - queryLen) * queryLen * s / avgdl`.
///
/// Equivalent to `org.apache.lucene.search.similarities.AxiomaticF3LOG`. Note
/// that the gamma component of this similarity creates negative scores, which
/// [`axiomatic_score`] floors at zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxiomaticF3LOG {
    params: AxiomaticParams,
}

impl AxiomaticF3LOG {
    /// Creates the similarity with the supplied `s` and `queryLen`, leaving `k`
    /// at its default.
    ///
    /// Equivalent to `new AxiomaticF3LOG(float, int)`
    /// (`AxiomaticF3LOG.java:27-29`), the only constructor Java offers for this
    /// variant.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `s` is not finite or lies
    /// outside `[0, 1]`, or when `query_len` is negative.
    pub fn with_s_and_query_len(s: f32, query_len: i32) -> Result<Self> {
        Ok(Self {
            params: AxiomaticParams::new(true, s, query_len, DEFAULT_K)?,
        })
    }
}

impl Axiomatic for AxiomaticF3LOG {
    fn s(&self) -> f32 {
        self.params.s
    }

    fn k(&self) -> f32 {
        self.params.k
    }

    fn query_len(&self) -> i32 {
        self.params.query_len
    }

    fn tf(&self, _stats: &BasicStats, freq: f64, _doc_len: f64) -> f64 {
        tf_log_log(freq)
    }

    fn ln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn tfln(&self, _stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        1.0
    }

    fn idf(&self, stats: &BasicStats, _freq: f64, _doc_len: f64) -> f64 {
        idf_log(stats)
    }

    fn gamma(&self, stats: &BasicStats, _freq: f64, doc_len: f64) -> f64 {
        gamma_f3(stats, self.params.s, self.params.query_len, doc_len)
    }

    fn tf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tf_log_log_explain(self.tf(stats, freq, doc_len), freq)
    }

    fn ln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        ln_one_explain(self.ln(stats, freq, doc_len))
    }

    fn tfln_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        tfln_one_explain(self.tfln(stats, freq, doc_len))
    }

    fn idf_explain(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> Explanation {
        idf_log_explain(self.idf(stats, freq, doc_len), stats)
    }
}

axiomatic_boilerplate!(AxiomaticF3LOG, "F3LOG");
