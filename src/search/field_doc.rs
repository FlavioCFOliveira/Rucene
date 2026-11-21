//! Sorted hits, ported from `org.apache.lucene.search.FieldDoc`.

#![deny(unsafe_code)]

use std::fmt;

use crate::search::field_comparator::SortValue;
use crate::search::score_doc::ScoreDoc;

/// Expert: a [`ScoreDoc`] that also carries the values used to sort it.
///
/// Equivalent to `org.apache.lucene.search.FieldDoc`, which extends `ScoreDoc`
/// and which
/// [`IndexSearcher::search`](crate::search::IndexSearcher::search) with a
/// [`Sort`](crate::search::Sort) returns. Rust has no implementation
/// inheritance, so the base hit is a field rather than a superclass.
///
/// The values are the sort criteria of the document: the values of the fields
/// the search sorted by, in the internal representation the comparators use, so
/// that the hit can be collated with hits from other searchers.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDoc {
    /// The doc ID, the score and the shard index of this hit.
    ///
    /// Equivalent to the state `FieldDoc` inherits from `ScoreDoc`.
    pub score_doc: ScoreDoc,

    /// Expert: the values which are used to sort the referenced document.
    ///
    /// Equivalent to the public `Object[] fields`, which is `null` until a
    /// collector fills it — hence the [`Option`].
    pub fields: Option<Vec<SortValue>>,
}

impl FieldDoc {
    /// Expert: creates a hit with empty sort information.
    ///
    /// Equivalent to `new FieldDoc(int, float)`.
    pub fn new(doc: i32, score: f32) -> Self {
        Self {
            score_doc: ScoreDoc::new(doc, score),
            fields: None,
        }
    }

    /// Expert: creates a hit with the given sort information.
    ///
    /// Equivalent to `new FieldDoc(int, float, Object[])`.
    pub fn with_fields(doc: i32, score: f32, fields: Vec<SortValue>) -> Self {
        Self {
            score_doc: ScoreDoc::new(doc, score),
            fields: Some(fields),
        }
    }

    /// Expert: creates a hit belonging to a specific shard, with the given sort
    /// information.
    ///
    /// Equivalent to `new FieldDoc(int, float, Object[], int)`.
    pub fn with_shard_index(
        doc: i32,
        score: f32,
        fields: Vec<SortValue>,
        shard_index: i32,
    ) -> Self {
        Self {
            score_doc: ScoreDoc::with_shard_index(doc, score, shard_index),
            fields: Some(fields),
        }
    }

    /// The doc ID of this hit.
    pub fn doc(&self) -> i32 {
        self.score_doc.doc
    }

    /// The score of this hit.
    pub fn score(&self) -> f32 {
        self.score_doc.score
    }

    /// The shard index of this hit.
    pub fn shard_index(&self) -> i32 {
        self.score_doc.shard_index
    }
}

impl fmt::Display for FieldDoc {
    /// Renders the hit exactly as `FieldDoc.toString()`, a convenience for
    /// debugging: the doc and score information, then the sort values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} fields=", self.score_doc)?;
        match &self.fields {
            None => f.write_str("null"),
            Some(fields) => {
                f.write_str("[")?;
                for (i, value) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{value}")?;
                }
                f.write_str("]")
            }
        }
    }
}
