//! Total hit counts, ported from `org.apache.lucene.search.TotalHits`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};

/// How a [`TotalHits`] value should be interpreted.
///
/// Equivalent to `org.apache.lucene.search.TotalHits.Relation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum TotalHitsRelation {
    /// The total hit count is equal to the value.
    EQUAL_TO,
    /// The total hit count is greater than or equal to the value.
    GREATER_THAN_OR_EQUAL_TO,
}

/// Description of the total number of hits of a query.
///
/// Equivalent to the `org.apache.lucene.search.TotalHits` record.
///
/// The total hit count generally cannot be computed accurately without visiting
/// every match, which is costly for queries that match many documents. Since a
/// lower bound — "there are more than 1000 hits" — is often enough, Lucene stops
/// counting as soon as a threshold has been reached, which is what the
/// [`relation`](Self::relation) records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TotalHits {
    value: i64,
    relation: TotalHitsRelation,
}

impl TotalHits {
    /// Creates a total hit count.
    ///
    /// Equivalent to the compact constructor of the Java record.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when `value` is negative.
    pub fn new(value: i64, relation: TotalHitsRelation) -> Result<Self> {
        if value < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "value must be >= 0, got {value}"
            )));
        }
        Ok(Self { value, relation })
    }

    /// The value of the total hit count, to be interpreted in the context of
    /// [`relation`](Self::relation).
    ///
    /// Equivalent to `TotalHits.value()`.
    pub fn value(&self) -> i64 {
        self.value
    }

    /// Whether [`value`](Self::value) is the exact hit count or a lower bound.
    ///
    /// Equivalent to `TotalHits.relation()`.
    pub fn relation(&self) -> TotalHitsRelation {
        self.relation
    }
}

impl fmt::Display for TotalHits {
    /// Renders the count exactly as `TotalHits.toString()`: the value, a `+`
    /// when it is only a lower bound, and the word `hits`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let suffix = if self.relation == TotalHitsRelation::EQUAL_TO {
            ""
        } else {
            "+"
        };
        write!(f, "{}{} hits", self.value, suffix)
    }
}
