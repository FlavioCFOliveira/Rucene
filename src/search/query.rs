//! Queries, ported from `org.apache.lucene.search.Query`.
//!
//! # Relationship with [`crate::index::Query`]
//!
//! [`crate::index::documents_writer`] declares a placeholder `Query` trait,
//! carrying only a textual representation, so that delete-by-query entries
//! could be recorded before the search package existed. This module is the real
//! `org.apache.lucene.search.Query`: it is what
//! [`IndexSearcher`](crate::search::IndexSearcher) and
//! [`Weight`](crate::search::Weight) are defined against. The placeholder
//! should eventually be replaced by this trait; doing so touches the indexing
//! write path and is left for the task that owns it.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::index_searcher::IndexSearcher;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// The base trait for queries.
///
/// Equivalent to the abstract class `org.apache.lucene.search.Query`.
pub trait Query: Debug + Send + Sync + 'static {
    /// Prints this query to a string, with `field` assumed to be the default
    /// field and omitted.
    ///
    /// Equivalent to `Query.toString(String)`.
    fn to_query_string(&self, field: &str) -> String;

    /// Recurses through the query tree, visiting any child queries.
    ///
    /// Equivalent to `Query.visit(QueryVisitor)`.
    fn visit(&self, visitor: &mut dyn QueryVisitor);

    /// Returns this query as [`Any`], so that
    /// [`same_class_as`](Self::same_class_as) and an implementation of
    /// [`query_eq`](Self::query_eq) can recover the concrete type.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reaches the concrete type
    /// through `getClass()` and a cast. Rust needs the escape hatch to be
    /// declared; every implementation writes `self`.
    fn as_any(&self) -> &dyn Any;

    /// Constructs an appropriate [`Weight`] implementation for this query.
    ///
    /// Equivalent to `Query.createWeight(IndexSearcher, ScoreMode, float)`,
    /// which throws `UnsupportedOperationException` by default. Only primitive
    /// queries — those that rewrite to themselves — implement it.
    ///
    /// * `score_mode` — how the produced scorers will be consumed;
    /// * `boost` — the boost propagated by the parent queries.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] by default, and propagates
    /// any I/O error an implementation raises.
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Err(LuceneError::UnsupportedOperation(format!(
            "Query {} does not implement createWeight",
            self.to_query_string("")
        )))
    }

    /// Rewrites this query into primitive queries — for example a prefix query
    /// into a boolean query of term queries.
    ///
    /// Equivalent to `Query.rewrite(IndexSearcher)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns `this` when the query
    /// does not rewrite, and callers detect that by *reference identity*:
    /// `IndexSearcher.rewrite` loops `while (rewritten != query)`. A trait
    /// object cannot return itself by value, so this port returns `None` for
    /// "rewrites to itself"; the searcher's loop is otherwise identical.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting.
    fn rewrite(&self, _searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(None)
    }

    /// Query instance equivalence.
    ///
    /// Equivalent to `Query.equals(Object)`, which Java leaves abstract. It is
    /// required so that [`QueryCache`](crate::search::QueryCache) works
    /// properly: typically a query is equal to another only if it is of the
    /// same concrete type and its document-filtering properties are identical.
    fn query_eq(&self, other: &dyn Query) -> bool;

    /// Query hash code.
    ///
    /// Equivalent to `Query.hashCode()`, which Java leaves abstract. It is
    /// required so that [`QueryCache`](crate::search::QueryCache) works
    /// properly, and must be consistent with [`query_eq`](Self::query_eq).
    fn query_hash(&self) -> u64;

    /// Checks whether `other` is exactly of the same concrete type as this
    /// query.
    ///
    /// Equivalent to `Query.sameClassAs(Object)`. When used in an
    /// implementation of [`query_eq`](Self::query_eq), consider using
    /// [`class_hash`](Self::class_hash) in [`query_hash`](Self::query_hash) so
    /// that different types hash differently.
    fn same_class_as(&self, other: &dyn Query) -> bool {
        self.as_any().type_id() == other.as_any().type_id()
    }

    /// Provides a constant integer for a given concrete type.
    ///
    /// Equivalent to `Query.classHash()`. Java derives it from the class name
    /// rather than from `Class.hashCode()`, so that hashes stay consistent
    /// across executions and debugging is easier; this port hashes the
    /// [`std::any::TypeId`], which is likewise stable within a build.
    fn class_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.as_any().type_id().hash(&mut hasher);
        hasher.finish()
    }
}

/// Prints a query to a string.
///
/// Equivalent to the `final Query.toString()`, which is `toString("")`. It is a
/// free function because a trait cannot provide an inherent `Display`
/// implementation for every implementor.
pub fn query_to_string(query: &dyn Query) -> String {
    query.to_query_string("")
}
