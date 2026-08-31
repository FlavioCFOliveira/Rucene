//! Second-pass rescoring, ported from `org.apache.lucene.search.Rescorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::index_searcher::IndexSearcher;
use crate::search::similarities::Explanation;
use crate::search::top_docs::TopDocs;

/// Re-scores the top-N results of an original query.
///
/// Equivalent to the abstract class `org.apache.lucene.search.Rescorer`. See
/// [`QueryRescorer`](crate::search::QueryRescorer) for an implementation.
/// Typically a low-cost first-pass query runs across the whole index,
/// collecting the top few hundred hits, and this trait then mixes in a more
/// costly second-pass scoring.
pub trait Rescorer {
    /// Rescores a first-pass [`TopDocs`].
    ///
    /// Equivalent to `Rescorer.rescore(IndexSearcher, TopDocs, int)`.
    ///
    /// * `searcher` — the searcher that produced the first-pass hits;
    /// * `first_pass_top_docs` — the first-pass hits. It is very important that
    ///   they were produced by the provided searcher, otherwise the doc IDs
    ///   will not match;
    /// * `top_n` — how many rescored hits to return.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rescoring.
    fn rescore(
        &self,
        searcher: &IndexSearcher,
        first_pass_top_docs: &TopDocs,
        top_n: i32,
    ) -> Result<TopDocs>;

    /// Explains how the score of the given document was computed.
    ///
    /// Equivalent to
    /// `Rescorer.explain(IndexSearcher, Explanation, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while explaining.
    fn explain(
        &self,
        searcher: &IndexSearcher,
        first_pass_explanation: &Explanation,
        doc_id: i32,
    ) -> Result<Explanation>;
}
