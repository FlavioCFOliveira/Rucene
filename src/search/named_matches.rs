//! Naming sub-queries so their matches can be identified, ported from
//! `org.apache.lucene.search.NamedMatches`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::boolean_clause::Occur;
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::{Matches, MatchesIterator};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::{FilterWeight, Weight};

/// Helps extract the set of sub-queries that matched, out of a larger query.
///
/// Equivalent to `org.apache.lucene.search.NamedMatches`. Individual
/// sub-queries may be wrapped with [`wrap_query`], and the matching queries for
/// a particular document can then be pulled from the parent query's [`Matches`]
/// object by calling [`find_named_matches`](NamedMatches::find_named_matches).
pub struct NamedMatches {
    inner: Arc<dyn Matches>,
    name: String,
}

impl std::fmt::Debug for NamedMatches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamedMatches")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl NamedMatches {
    /// Wraps a [`Matches`] object and associates a name with it.
    ///
    /// Equivalent to `NamedMatches(String, Matches)`; Java's
    /// `Objects.requireNonNull` is unnecessary because `Arc<dyn Matches>`
    /// cannot be null.
    pub fn new(name: impl Into<String>, inner: Arc<dyn Matches>) -> Self {
        Self {
            inner,
            name: name.into(),
        }
    }

    /// Returns the name of this [`Matches`].
    ///
    /// Equivalent to `NamedMatches.getName()`.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Finds all [`NamedMatches`] in a [`Matches`] tree.
    ///
    /// Equivalent to `NamedMatches.findNamedMatches(Matches)`, which walks the
    /// tree breadth-first through `getSubMatches()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a
    /// `List<NamedMatches>`. Rust cannot narrow an `Arc<dyn Matches>` to an
    /// `Arc<NamedMatches>` without `Arc<dyn Any + Send + Sync>`, which the
    /// [`Matches`] trait is not, so the elements keep the erased type. Every
    /// element is a [`NamedMatches`]; recover the name with
    /// `m.as_any().downcast_ref::<NamedMatches>()`.
    pub fn find_named_matches(matches: Arc<dyn Matches>) -> Vec<Arc<dyn Matches>> {
        let mut nm: Vec<Arc<dyn Matches>> = Vec::new();
        let mut to_process: VecDeque<Arc<dyn Matches>> = VecDeque::new();
        to_process.push_back(matches);
        while let Some(matches) = to_process.pop_front() {
            if matches.as_any().is::<NamedMatches>() {
                nm.push(Arc::clone(&matches));
            }
            to_process.extend(matches.get_sub_matches());
        }
        nm
    }
}

impl Matches for NamedMatches {
    fn get_matches(&self, field: &str) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.inner.get_matches(field)
    }

    fn get_sub_matches(&self) -> Vec<Arc<dyn Matches>> {
        vec![Arc::clone(&self.inner)]
    }

    fn fields(&self) -> Vec<String> {
        self.inner.fields()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Wraps a query so that it associates `name` with its [`Matches`].
///
/// Equivalent to `NamedMatches.wrapQuery(String, Query)`.
pub fn wrap_query(name: impl Into<String>, inner: Arc<dyn Query>) -> Arc<dyn Query> {
    Arc::new(NamedQuery::new(name, inner))
}

/// A [`Query`] that tags the [`Matches`] of the query it wraps with a name.
///
/// Equivalent to the private `NamedMatches.NamedQuery`; it is public here
/// because Rust has no package visibility and [`wrap_query`] hands it out as a
/// `dyn Query`.
#[derive(Debug, Clone)]
pub struct NamedQuery {
    name: String,
    inner: Arc<dyn Query>,
}

impl NamedQuery {
    /// Wraps `inner` under `name`.
    ///
    /// Equivalent to the private `NamedQuery(String, Query)`.
    pub fn new(name: impl Into<String>, inner: Arc<dyn Query>) -> Self {
        Self {
            name: name.into(),
            inner,
        }
    }
}

impl Query for NamedQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "NamedQuery({},{})",
            self.name,
            self.inner.to_query_string(field)
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::MUST, self);
        self.inner.visit(&mut *sub);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let w = self.inner.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(NamedWeight {
            inner: FilterWeight::new(w),
            name: self.name.clone(),
        }))
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.inner.rewrite(searcher)? {
            Some(rewritten) => Ok(Some(Arc::new(NamedQuery::new(
                self.name.clone(),
                rewritten,
            )))),
            None => Ok(None),
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<NamedQuery>() else {
            return false;
        };
        self.name == other.name && self.inner.query_eq(&*other.inner)
    }

    fn query_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.name.hash(&mut hasher);
        let mut h = self.class_hash();
        h = h.wrapping_mul(31).wrapping_add(hasher.finish());
        h = h.wrapping_mul(31).wrapping_add(self.inner.query_hash());
        h
    }
}

/// The [`Weight`] of a [`NamedQuery`], which tags the matches it produces.
///
/// Equivalent to the anonymous `FilterWeight` subclass built by
/// `NamedQuery.createWeight`.
#[derive(Debug)]
struct NamedWeight {
    inner: FilterWeight,
    name: String,
}

impl SegmentCacheable for NamedWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl Weight for NamedWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.inner.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.inner.explain(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.inner.scorer_supplier(context)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.inner.count(context)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        // Java reads the `FilterWeight.in` field directly, so that the wrapped
        // weight's own `matches` runs rather than this override.
        let m = self.inner.inner.matches(context, doc)?;
        match m {
            None => Ok(None),
            Some(m) => Ok(Some(Arc::new(NamedMatches::new(self.name.clone(), m)))),
        }
    }
}
