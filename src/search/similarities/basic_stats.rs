//! Corpus-level statistics shared by the [`SimilarityBase`] family, ported from
//! `org.apache.lucene.search.similarities.BasicStats`.
//!
//! [`SimilarityBase`]: super::SimilarityBase

#![deny(unsafe_code)]

/// Stores all statistics commonly used by ranking methods.
///
/// Equivalent to `org.apache.lucene.search.similarities.BasicStats`. Java fills
/// the mutable fields through setters after construction, in
/// `SimilarityBase.fillBasicStats` (`SimilarityBase.java:88-101`); this port
/// keeps that shape so that a similarity overriding `fill_basic_stats` — as the
/// language-modelling family does — reads exactly like its Java counterpart.
#[derive(Debug, Clone, PartialEq)]
pub struct BasicStats {
    field: String,
    number_of_documents: i64,
    number_of_field_tokens: i64,
    avg_field_length: f64,
    doc_freq: i64,
    total_term_freq: i64,
    boost: f64,
}

impl BasicStats {
    /// Creates statistics for the given field and query-time boost.
    ///
    /// Equivalent to `new BasicStats(String, double)`
    /// (`BasicStats.java:49-52`). The remaining fields default to zero, as
    /// Java's do, and are filled in by
    /// [`SimilarityBase::fill_basic_stats`](super::SimilarityBase::fill_basic_stats).
    pub fn new(field: impl Into<String>, boost: f64) -> Self {
        Self {
            field: field.into(),
            number_of_documents: 0,
            number_of_field_tokens: 0,
            avg_field_length: 0.0,
            doc_freq: 0,
            total_term_freq: 0,
            boost,
        }
    }

    /// Returns the field these statistics describe.
    ///
    /// Java declares `BasicStats.field` package-private with no getter
    /// (`BasicStats.java:26`), so this accessor is crate-visible rather than
    /// public. Nothing in Lucene Core reads it either — it is state the class
    /// carries for similarities written inside the package — which is why the
    /// dead-code lint is silenced rather than the field dropped.
    #[allow(dead_code)]
    pub(crate) fn field(&self) -> &str {
        &self.field
    }

    /// Returns the number of documents.
    ///
    /// Equivalent to `BasicStats.getNumberOfDocuments()`.
    pub fn number_of_documents(&self) -> i64 {
        self.number_of_documents
    }

    /// Sets the number of documents.
    ///
    /// Equivalent to `BasicStats.setNumberOfDocuments(long)`.
    pub fn set_number_of_documents(&mut self, number_of_documents: i64) {
        self.number_of_documents = number_of_documents;
    }

    /// Returns the total number of tokens in the field.
    ///
    /// Equivalent to `BasicStats.getNumberOfFieldTokens()`, which mirrors
    /// `Terms.getSumTotalTermFreq()`.
    pub fn number_of_field_tokens(&self) -> i64 {
        self.number_of_field_tokens
    }

    /// Sets the total number of tokens in the field.
    ///
    /// Equivalent to `BasicStats.setNumberOfFieldTokens(long)`.
    pub fn set_number_of_field_tokens(&mut self, number_of_field_tokens: i64) {
        self.number_of_field_tokens = number_of_field_tokens;
    }

    /// Returns the average field length.
    ///
    /// Equivalent to `BasicStats.getAvgFieldLength()`.
    pub fn avg_field_length(&self) -> f64 {
        self.avg_field_length
    }

    /// Sets the average field length.
    ///
    /// Equivalent to `BasicStats.setAvgFieldLength(double)`.
    pub fn set_avg_field_length(&mut self, avg_field_length: f64) {
        self.avg_field_length = avg_field_length;
    }

    /// Returns the document frequency.
    ///
    /// Equivalent to `BasicStats.getDocFreq()`.
    pub fn doc_freq(&self) -> i64 {
        self.doc_freq
    }

    /// Sets the document frequency.
    ///
    /// Equivalent to `BasicStats.setDocFreq(long)`.
    pub fn set_doc_freq(&mut self, doc_freq: i64) {
        self.doc_freq = doc_freq;
    }

    /// Returns the total number of occurrences of this term across all
    /// documents.
    ///
    /// Equivalent to `BasicStats.getTotalTermFreq()`.
    pub fn total_term_freq(&self) -> i64 {
        self.total_term_freq
    }

    /// Sets the total number of occurrences of this term across all documents.
    ///
    /// Equivalent to `BasicStats.setTotalTermFreq(long)`.
    pub fn set_total_term_freq(&mut self, total_term_freq: i64) {
        self.total_term_freq = total_term_freq;
    }

    /// Returns the query boost, applied as a multiplicative factor to the
    /// score.
    ///
    /// Equivalent to `BasicStats.getBoost()`. The field is `final` in Java, so
    /// there is no setter.
    pub fn boost(&self) -> f64 {
        self.boost
    }
}
