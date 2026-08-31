//! Matching no document, ported from
//! `org.apache.lucene.search.MatchNoDocsQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::index_searcher::IndexSearcher;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::Weight;

/// A query that matches no documents.
///
/// Equivalent to `org.apache.lucene.search.MatchNoDocsQuery`.
///
/// **NOTE:** all instances of this type are equal, even when they were built
/// with distinct reasons.
#[derive(Debug, Default, Clone)]
pub struct MatchNoDocsQuery {
    reason: String,
}

impl MatchNoDocsQuery {
    /// Creates a query with a blank reason.
    ///
    /// Equivalent to the `MatchNoDocsQuery.INSTANCE` constant, which is
    /// `new MatchNoDocsQuery()` and therefore `new MatchNoDocsQuery("")`.
    pub fn instance() -> Self {
        Self::new("")
    }

    /// Creates a query, recording the reason it was used.
    ///
    /// Equivalent to `new MatchNoDocsQuery(String)`.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// Returns the reason this query was used.
    ///
    /// Equivalent to reading the private `reason` field, which Java only uses
    /// from `toString` and from the weight's explanation.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// The weight [`MatchNoDocsQuery`] creates.
///
/// Equivalent to the anonymous `Weight` subclass in
/// `MatchNoDocsQuery.createWeight`.
#[derive(Debug)]
struct MatchNoDocsWeight {
    query: Arc<dyn Query>,
    reason: String,
}

impl SegmentCacheable for MatchNoDocsWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for MatchNoDocsWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, _context: &LeafReaderContext, _doc: i32) -> Result<Explanation> {
        Ok(Explanation::no_match(self.reason.clone(), Vec::new()))
    }

    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(None)
    }

    fn count(&self, _context: &LeafReaderContext) -> Result<i32> {
        Ok(0)
    }
}

impl Query for MatchNoDocsQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!("MatchNoDocsQuery(\"{}\")", self.reason)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(MatchNoDocsWeight {
            query: Arc::new(self.clone()),
            reason: self.reason.clone(),
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
    }

    fn query_hash(&self) -> u64 {
        self.class_hash()
    }
}
