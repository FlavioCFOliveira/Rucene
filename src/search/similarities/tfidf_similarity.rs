//! The vector-space, tf-idf family, ported from
//! `org.apache.lucene.search.similarities.TFIDFSimilarity` and
//! `ClassicSimilarity`.

#![deny(unsafe_code)]

use std::fmt;

use super::similarity_base::decoded_length;
use super::{CollectionStatistics, Explanation, SimScorer, Similarity, TermStatistics};

/// Expert: a similarity with a simple lexical model of tf-idf scoring.
///
/// Equivalent to `org.apache.lucene.search.similarities.TFIDFSimilarity`, the
/// abstract base of Lucene's classic vector-space scoring. Implementors supply
/// [`Self::tf`], [`Self::idf`] and [`Self::length_norm`], and forward
/// [`Similarity::scorer`] to [`tfidf_scorer`] — which is the body Java inherits
/// from the `final TFIDFSimilarity.scorer`.
///
/// Consider [`BM25Similarity`](super::BM25Similarity) instead, which is
/// generally held to be superior to tf-idf.
pub trait TFIDFSimilarity: Similarity {
    /// Computes a score factor based on a term or phrase's frequency in a
    /// document.
    ///
    /// Equivalent to the abstract `TFIDFSimilarity.tf(float)`
    /// (`TFIDFSimilarity.java:333`). `freq` is the sloppy frequency, which is
    /// why it is a `float` rather than an `int`.
    fn tf(&self, freq: f32) -> f32;

    /// Computes a score factor based on a term's document frequency.
    ///
    /// Equivalent to the abstract `TFIDFSimilarity.idf(long, long)`
    /// (`TFIDFSimilarity.java:399`).
    fn idf(&self, doc_freq: i64, doc_count: i64) -> f32;

    /// Computes the norm value encoded at index time.
    ///
    /// Equivalent to the abstract `TFIDFSimilarity.lengthNorm(int)`
    /// (`TFIDFSimilarity.java:408`). The argument is the decoded number of
    /// terms, not the encoded byte.
    fn length_norm(&self, length: i32) -> f32;

    /// Computes a score factor for a simple term, and explains it.
    ///
    /// Equivalent to
    /// `TFIDFSimilarity.idfExplain(CollectionStatistics, TermStatistics)`
    /// (`TFIDFSimilarity.java:355-364`).
    ///
    /// `CollectionStatistics::doc_count` is used rather than the reader's
    /// `numDocs()`, for the reason given on
    /// [`BM25Similarity::idf_explain`](super::BM25Similarity::idf_explain).
    fn idf_explain(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
    ) -> Explanation {
        let df = term_stats.doc_freq();
        let doc_count = collection_stats.doc_count();
        let idf = self.idf(df, doc_count);
        Explanation::matched(
            idf,
            "idf(docFreq, docCount)",
            vec![
                Explanation::matched(df, "docFreq, number of documents containing term", vec![]),
                Explanation::matched(
                    doc_count,
                    "docCount, total number of documents with field",
                    vec![],
                ),
            ],
        )
    }

    /// Computes a score factor for a phrase, and explains it.
    ///
    /// Equivalent to
    /// `TFIDFSimilarity.idfExplain(CollectionStatistics, TermStatistics[])`
    /// (`TFIDFSimilarity.java:376-385`), which sums the per-term factors into a
    /// `double` before narrowing once.
    fn idf_explain_phrase(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Explanation {
        let mut idf = 0.0f64;
        let mut subs = Vec::with_capacity(term_stats.len());
        for stat in term_stats {
            let idf_explain = self.idf_explain(collection_stats, stat);
            idf += f64::from(idf_explain.value().float_value());
            subs.push(idf_explain);
        }
        Explanation::matched(idf as f32, "idf(), sum of:", subs)
    }
}

/// Builds the scorer for a [`TFIDFSimilarity`] descendant.
///
/// This is the body of `TFIDFSimilarity.scorer`
/// (`TFIDFSimilarity.java:419-433`), which Java declares `final`. The 256 field
/// norms are precomputed here; entry `0` is never a legal norm, so Lucene fills
/// it with `1 / normTable[255]`, the largest value the table can take.
pub fn tfidf_scorer<'a, S>(
    similarity: &'a S,
    boost: f32,
    collection_stats: &CollectionStatistics,
    term_stats: &[TermStatistics],
) -> Box<dyn SimScorer + 'a>
where
    S: TFIDFSimilarity + ?Sized,
{
    let idf = match term_stats {
        [single] => similarity.idf_explain(collection_stats, single),
        many => similarity.idf_explain_phrase(collection_stats, many),
    };
    let mut norm_table = [0.0f32; 256];
    for (i, entry) in norm_table.iter_mut().enumerate().skip(1) {
        *entry = similarity.length_norm(decoded_length(i as u8));
    }
    norm_table[0] = 1.0 / norm_table[255];
    Box::new(TFIDFScorer::new(similarity, boost, idf, norm_table))
}

/// The scorer `TFIDFSimilarity.scorer` returns.
///
/// Equivalent to the inner class `TFIDFSimilarity.TFIDFScorer`
/// (`TFIDFSimilarity.java:439-490`), which is non-static so that it can call
/// `tf(float)` on its enclosing similarity; the borrow here plays that role.
struct TFIDFScorer<'a, S: TFIDFSimilarity + ?Sized> {
    similarity: &'a S,
    /// The idf and its explanation.
    idf: Explanation,
    boost: f32,
    query_weight: f32,
    norm_table: [f32; 256],
}

impl<'a, S: TFIDFSimilarity + ?Sized> TFIDFScorer<'a, S> {
    fn new(similarity: &'a S, boost: f32, idf: Explanation, norm_table: [f32; 256]) -> Self {
        let query_weight = boost * idf.value().float_value();
        Self {
            similarity,
            idf,
            boost,
            query_weight,
            norm_table,
        }
    }
}

impl<S: TFIDFSimilarity + ?Sized> SimScorer for TFIDFScorer<'_, S> {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        // tf(f) * weight, then normalized for the field.
        let raw = self.similarity.tf(freq) * self.query_weight;
        let norm_value = self.norm_table[(norm & 0xFF) as usize];
        raw * norm_value
    }

    fn explain(&self, freq: &Explanation, encoded_norm: i64) -> Explanation {
        let mut subs = Vec::new();
        if self.boost != 1.0 {
            subs.push(Explanation::matched(self.boost, "boost", vec![]));
        }
        subs.push(self.idf.clone());
        let tf = Explanation::matched(
            self.similarity.tf(freq.value().float_value()),
            format!("tf(freq={}), with freq of:", freq.value()),
            vec![freq.clone()],
        );
        let tf_value = tf.value().float_value();
        subs.push(tf);

        let norm = self.norm_table[(encoded_norm & 0xFF) as usize];
        subs.push(Explanation::matched(norm, "fieldNorm", vec![]));

        Explanation::matched(
            self.query_weight * tf_value * norm,
            format!("score(freq={}), product of:", freq.value()),
            subs,
        )
    }
}

/// Expert: historical scoring implementation.
///
/// Equivalent to `org.apache.lucene.search.similarities.ClassicSimilarity`.
/// Consider [`BM25Similarity`](super::BM25Similarity) instead, which is
/// generally held to be superior to tf-idf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicSimilarity {
    discount_overlaps: bool,
}

impl ClassicSimilarity {
    /// Creates the similarity, discounting overlaps.
    ///
    /// Equivalent to `new ClassicSimilarity()`
    /// (`ClassicSimilarity.java:29-31`).
    pub fn new() -> Self {
        Self {
            discount_overlaps: true,
        }
    }

    /// Creates the similarity with an explicit `discount_overlaps`.
    ///
    /// Equivalent to `new ClassicSimilarity(boolean)`
    /// (`ClassicSimilarity.java:33-35`).
    pub fn with_discount_overlaps(discount_overlaps: bool) -> Self {
        Self { discount_overlaps }
    }
}

impl Default for ClassicSimilarity {
    fn default() -> Self {
        Self::new()
    }
}

impl Similarity for ClassicSimilarity {
    fn discount_overlaps(&self) -> bool {
        self.discount_overlaps
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        tfidf_scorer(self, boost, collection_stats, term_stats)
    }
}

impl TFIDFSimilarity for ClassicSimilarity {
    /// Implemented as `sqrt(freq)`, as `ClassicSimilarity.tf(float)` does.
    fn tf(&self, freq: f32) -> f32 {
        f64::from(freq).sqrt() as f32
    }

    /// Implemented as `log((docCount + 1) / (docFreq + 1)) + 1`, as
    /// `ClassicSimilarity.idf(long, long)` does.
    fn idf(&self, doc_freq: i64, doc_count: i64) -> f32 {
        // Java adds the `1.0` inside the `double` and narrows once.
        (((doc_count + 1) as f64 / (doc_freq + 1) as f64).ln() + 1.0) as f32
    }

    /// Implemented as `1 / sqrt(length)`, as
    /// `ClassicSimilarity.lengthNorm(int)` does.
    fn length_norm(&self, num_terms: i32) -> f32 {
        (1.0 / f64::from(num_terms).sqrt()) as f32
    }

    /// Overrides the base explanation with the concrete formula, as
    /// `ClassicSimilarity.idfExplain` does
    /// (`ClassicSimilarity.java:53-65`).
    fn idf_explain(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
    ) -> Explanation {
        let df = term_stats.doc_freq();
        let doc_count = collection_stats.doc_count();
        let idf = self.idf(df, doc_count);
        Explanation::matched(
            idf,
            "idf, computed as log((docCount+1)/(docFreq+1)) + 1 from:",
            vec![
                Explanation::matched(df, "docFreq, number of documents containing term", vec![]),
                Explanation::matched(
                    doc_count,
                    "docCount, total number of documents with field",
                    vec![],
                ),
            ],
        )
    }
}

impl fmt::Display for ClassicSimilarity {
    /// Renders the similarity as `ClassicSimilarity.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClassicSimilarity")
    }
}
