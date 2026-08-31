//! Sorted search results, ported from
//! `org.apache.lucene.search.TopFieldDocs`.

#![deny(unsafe_code)]

use crate::search::field_doc::FieldDoc;
use crate::search::sort::SortField;
use crate::search::top_docs::TopDocs;
use crate::search::total_hits::TotalHits;

/// Represents the hits returned by a search with a
/// [`Sort`](crate::search::Sort).
///
/// Equivalent to `org.apache.lucene.search.TopFieldDocs`, which extends
/// `TopDocs` and narrows its `ScoreDoc[]` to hold `FieldDoc` instances. Rust
/// has no implementation inheritance and cannot narrow a `Vec<ScoreDoc>`, so
/// the hits are typed as [`FieldDoc`] here and
/// [`to_top_docs`](Self::to_top_docs) produces the base view Java gets for
/// free.
#[derive(Debug, Clone, PartialEq)]
pub struct TopFieldDocs {
    /// The total number of hits for the query.
    ///
    /// Equivalent to the `TotalHits totalHits` field inherited from `TopDocs`.
    pub total_hits: TotalHits,

    /// The top hits for the query.
    ///
    /// Equivalent to the `ScoreDoc[] scoreDocs` field inherited from
    /// `TopDocs`, whose elements are always `FieldDoc` instances here.
    pub score_docs: Vec<FieldDoc>,

    /// The fields which were used to sort the results by.
    ///
    /// Equivalent to the public `SortField[] fields`.
    pub fields: Vec<SortField>,
}

impl TopFieldDocs {
    /// Creates one of these objects with the given sort information.
    ///
    /// Equivalent to `new TopFieldDocs(TotalHits, ScoreDoc[], SortField[])`.
    pub fn new(total_hits: TotalHits, score_docs: Vec<FieldDoc>, fields: Vec<SortField>) -> Self {
        Self {
            total_hits,
            score_docs,
            fields,
        }
    }

    /// Returns these results as a plain [`TopDocs`], dropping the per-hit sort
    /// values.
    ///
    /// Equivalent to viewing a `TopFieldDocs` through its `TopDocs` supertype,
    /// which Java gets from inheritance.
    pub fn to_top_docs(&self) -> TopDocs {
        TopDocs::new(
            self.total_hits,
            self.score_docs.iter().map(|hit| hit.score_doc).collect(),
        )
    }
}
