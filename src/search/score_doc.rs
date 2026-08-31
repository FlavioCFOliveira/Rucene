//! Hits, ported from `org.apache.lucene.search.ScoreDoc`.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::fmt;

/// Holds one hit in a [`TopDocs`](crate::search::TopDocs).
///
/// Equivalent to `org.apache.lucene.search.ScoreDoc`, whose three fields are
/// public and mutable in Java and are public here for the same reason: the
/// collectors and the merge routine write them in place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreDoc {
    /// The score of this document for the query.
    pub score: f32,

    /// A hit document's number, as accepted by
    /// [`StoredFields::document`](crate::index::StoredFields::document).
    pub doc: i32,

    /// Only set by [`TopDocs::merge`](crate::search::TopDocs::merge).
    pub shard_index: i32,
}

impl ScoreDoc {
    /// Constructs a hit with no shard index.
    ///
    /// Equivalent to `new ScoreDoc(int, float)`, which passes a shard index of
    /// `-1`.
    pub fn new(doc: i32, score: f32) -> Self {
        Self::with_shard_index(doc, score, -1)
    }

    /// Constructs a hit belonging to a specific shard.
    ///
    /// Equivalent to `new ScoreDoc(int, float, int)`.
    pub fn with_shard_index(doc: i32, score: f32, shard_index: i32) -> Self {
        Self {
            doc,
            score,
            shard_index,
        }
    }

    /// Sorts by score descending, then by doc ID ascending.
    ///
    /// Equivalent to the `ScoreDoc.COMPARATOR` constant. Note that Java
    /// compares the scores with `>` and `<` rather than `Float.compare`, so
    /// `NaN` scores fall through to the doc ID comparison; this port reproduces
    /// that exactly.
    pub fn compare(a: &ScoreDoc, b: &ScoreDoc) -> Ordering {
        if a.score > b.score {
            Ordering::Less
        } else if a.score < b.score {
            Ordering::Greater
        } else {
            a.doc.cmp(&b.doc)
        }
    }
}

impl fmt::Display for ScoreDoc {
    /// Renders the hit exactly as `ScoreDoc.toString()`, a convenience for
    /// debugging.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "doc={} score={} shardIndex={}",
            self.doc, self.score, self.shard_index
        )
    }
}
