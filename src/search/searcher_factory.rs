//! Searcher creation, ported from
//! `org.apache.lucene.search.SearcherFactory`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::IndexReader;
use crate::search::index_searcher::IndexSearcher;

/// Creates the [`IndexSearcher`] instances a
/// [`SearcherManager`](crate::search::SearcherManager) hands out.
///
/// Equivalent to `org.apache.lucene.search.SearcherFactory`, whose default
/// implementation just creates an `IndexSearcher` with no custom behaviour.
/// Java makes it a class one subclasses; Rust has no implementation
/// inheritance, so it is a trait with the same default body and
/// [`DefaultSearcherFactory`] plays the role of `new SearcherFactory()`.
///
/// Supply your own implementation for custom behaviour, such as:
///
/// * setting a custom scoring model with
///   [`IndexSearcher::set_similarity`];
/// * parallel per-segment search with
///   [`IndexSearcher::with_executor`];
/// * running queries to warm the searcher before it goes live.
pub trait SearcherFactory: Send + Sync + std::fmt::Debug {
    /// Returns a new searcher over the given reader.
    ///
    /// Equivalent to `SearcherFactory.newSearcher(IndexReader, IndexReader)`.
    ///
    /// * `reader` — the reader to create a new searcher for;
    /// * `previous_reader` — the reader previously used to create a searcher.
    ///   It is `None` if unknown, or if `reader` is the initially opened
    ///   reader. When present it can be used to find the segments that are new
    ///   relative to `reader`, so that the searcher can be warmed before it is
    ///   returned.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building or warming the searcher.
    fn new_searcher(
        &self,
        reader: Arc<dyn IndexReader>,
        _previous_reader: Option<Arc<dyn IndexReader>>,
    ) -> Result<IndexSearcher> {
        IndexSearcher::new(reader)
    }
}

/// The factory `new SearcherFactory()` produces: it creates a plain
/// [`IndexSearcher`] and does nothing else.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSearcherFactory;

impl SearcherFactory for DefaultSearcherFactory {}
