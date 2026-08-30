//! Collection- and term-level statistics, ported from
//! `org.apache.lucene.search.CollectionStatistics` and
//! `org.apache.lucene.search.TermStatistics`.
//!
//! See [`super::explanation`] for why these two live under `similarities`
//! rather than under `search`: it is a placement divergence only, and
//! [`crate::search`] re-exports them where they will eventually live.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::util::BytesRef;

/// Statistics for a collection (field).
///
/// Equivalent to `org.apache.lucene.search.CollectionStatistics`, a Java
/// record. Every field is validated on construction, so an instance always
/// satisfies the invariants Lucene documents:
///
/// * every statistic is a positive integer, never zero or negative;
/// * `doc_count <= max_doc`;
/// * `doc_count <= sum_doc_freq <= sum_total_term_freq`.
///
/// Values may include statistics on deleted documents that have not yet been
/// merged away.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollectionStatistics {
    field: String,
    max_doc: i64,
    doc_count: i64,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
}

impl CollectionStatistics {
    /// Creates statistics for a collection (field).
    ///
    /// Equivalent to the compact constructor of the Java record
    /// (`CollectionStatistics.java:76-109`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the same message text
    /// Java produces — when `max_doc`, `doc_count`, `sum_doc_freq` or
    /// `sum_total_term_freq` is not positive, when `doc_count` exceeds
    /// `max_doc`, when `sum_doc_freq` is below `doc_count`, or when
    /// `sum_total_term_freq` is below `sum_doc_freq`.
    pub fn new(
        field: impl Into<String>,
        max_doc: i64,
        doc_count: i64,
        sum_total_term_freq: i64,
        sum_doc_freq: i64,
    ) -> Result<Self> {
        if max_doc <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxDoc must be positive, maxDoc: {max_doc}"
            )));
        }
        if doc_count <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "docCount must be positive, docCount: {doc_count}"
            )));
        }
        if doc_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "docCount must not exceed maxDoc, docCount: {doc_count}, maxDoc: {max_doc}"
            )));
        }
        if sum_doc_freq <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "sumDocFreq must be positive, sumDocFreq: {sum_doc_freq}"
            )));
        }
        if sum_doc_freq < doc_count {
            return Err(LuceneError::IllegalArgument(format!(
                "sumDocFreq must be at least docCount, sumDocFreq: {sum_doc_freq}, docCount: {doc_count}"
            )));
        }
        if sum_total_term_freq <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "sumTotalTermFreq must be positive, sumTotalTermFreq: {sum_total_term_freq}"
            )));
        }
        if sum_total_term_freq < sum_doc_freq {
            return Err(LuceneError::IllegalArgument(format!(
                "sumTotalTermFreq must be at least sumDocFreq, sumTotalTermFreq: {sum_total_term_freq}, sumDocFreq: {sum_doc_freq}"
            )));
        }
        Ok(Self {
            field: field.into(),
            max_doc,
            doc_count,
            sum_total_term_freq,
            sum_doc_freq,
        })
    }

    /// Returns the field's name.
    ///
    /// Equivalent to `CollectionStatistics.field()`.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the total number of documents, regardless of whether they all
    /// contain a value for this field.
    ///
    /// Equivalent to `CollectionStatistics.maxDoc()`.
    pub fn max_doc(&self) -> i64 {
        self.max_doc
    }

    /// Returns the number of documents that have at least one term for this
    /// field.
    ///
    /// Equivalent to `CollectionStatistics.docCount()`.
    pub fn doc_count(&self) -> i64 {
        self.doc_count
    }

    /// Returns the total number of tokens for this field — the "word count"
    /// across all documents.
    ///
    /// Equivalent to `CollectionStatistics.sumTotalTermFreq()`.
    pub fn sum_total_term_freq(&self) -> i64 {
        self.sum_total_term_freq
    }

    /// Returns the total number of posting-list entries for this field.
    ///
    /// Equivalent to `CollectionStatistics.sumDocFreq()`.
    pub fn sum_doc_freq(&self) -> i64 {
        self.sum_doc_freq
    }
}

/// Statistics for a specific term.
///
/// Equivalent to `org.apache.lucene.search.TermStatistics`, a Java record. As
/// in Java, both statistics are positive and `doc_freq <= total_term_freq`;
/// the cross-checks against the enclosing [`CollectionStatistics`] are, also as
/// in Java, not performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermStatistics {
    term: BytesRef,
    doc_freq: i64,
    total_term_freq: i64,
}

impl TermStatistics {
    /// Creates statistics for a term.
    ///
    /// Equivalent to the compact constructor of the Java record
    /// (`TermStatistics.java:71-84`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the same message text
    /// Java produces — when `doc_freq` or `total_term_freq` is not positive, or
    /// when `total_term_freq` is below `doc_freq`.
    pub fn new(term: BytesRef, doc_freq: i64, total_term_freq: i64) -> Result<Self> {
        if doc_freq <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "docFreq must be positive, docFreq: {doc_freq}"
            )));
        }
        if total_term_freq <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "totalTermFreq must be positive, totalTermFreq: {total_term_freq}"
            )));
        }
        if total_term_freq < doc_freq {
            return Err(LuceneError::IllegalArgument(format!(
                "totalTermFreq must be at least docFreq, totalTermFreq: {total_term_freq}, docFreq: {doc_freq}"
            )));
        }
        Ok(Self {
            term,
            doc_freq,
            total_term_freq,
        })
    }

    /// Returns the term's bytes.
    ///
    /// Equivalent to `TermStatistics.term()`.
    pub fn term(&self) -> &BytesRef {
        &self.term
    }

    /// Returns the number of documents containing the term in the collection.
    ///
    /// Equivalent to `TermStatistics.docFreq()`.
    pub fn doc_freq(&self) -> i64 {
        self.doc_freq
    }

    /// Returns the number of occurrences of the term in the collection.
    ///
    /// Equivalent to `TermStatistics.totalTermFreq()`.
    pub fn total_term_freq(&self) -> i64 {
        self.total_term_freq
    }
}
