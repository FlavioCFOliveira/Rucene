/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `SpanQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::collections::BTreeMap;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{Term, TermStates};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;

use crate::spans::span_weight::SpanWeight;

/// A map of terms to their [`TermStates`], ordered by term.
///
/// Equivalent to the `Map<Term, TermStates>` the span weights exchange, which
/// [`get_term_states`] creates as a `java.util.TreeMap`: its iteration order —
/// the order of the term statistics a
/// [`SpanWeight`] hands to the similarity
/// — is the natural order of [`Term`].
pub type TermStatesMap = BTreeMap<Term, Arc<TermStates>>;

/// Base trait for span-based queries.
///
/// Equivalent to the abstract class `org.apache.lucene.queries.spans.SpanQuery`.
///
/// **Divergence from Lucene 10.5.0.** Java narrows two inherited members by
/// covariance: `createWeight` returns a `SpanWeight` (`SpanQuery.java:36`), and
/// the composite span queries cast the `Query` that `rewrite` returns back to
/// `SpanQuery` (`SpanNearQuery.java:261`, `SpanOrQuery.java:74`). Rust has
/// neither covariant overrides nor a cast from `dyn Query` to `dyn SpanQuery`,
/// so the narrowed members are declared here —
/// [`create_span_weight`](Self::create_span_weight) and
/// [`rewrite_span`](Self::rewrite_span) — and every implementation's
/// [`Query::create_weight`] and [`Query::rewrite`] forward to them.
pub trait SpanQuery: Query {
    /// Returns the name of the field matched by this query, or `None` where
    /// Java returns `null`.
    ///
    /// Equivalent to `SpanQuery.getField()`.
    fn get_field(&self) -> Option<&str>;

    /// Creates the [`SpanWeight`] of this query.
    ///
    /// Equivalent to the covariant `SpanQuery.createWeight(IndexSearcher,
    /// ScoreMode, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the weight.
    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>>;

    /// Rewrites this query, returning `None` when it rewrites to itself.
    ///
    /// Equivalent to `Query.rewrite(IndexSearcher)` on a span query, whose
    /// result every span query of Lucene is: the composite span queries cast it
    /// to `SpanQuery` (`SpanNearQuery.java:261`, `SpanOrQuery.java:74`, …).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting.
    fn rewrite_span(&self, _searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        Ok(None)
    }
}

/// Builds a map of terms to [`TermStates`], for use in constructing span
/// weights.
///
/// Equivalent to `SpanQuery.getTermStates(SpanWeight...)`.
pub fn get_term_states(weights: &[&dyn SpanWeight]) -> TermStatesMap {
    let mut terms = TermStatesMap::new();
    for w in weights {
        w.extract_term_states(&mut terms);
    }
    terms
}

/// Builds a map of terms to [`TermStates`], for use in constructing span
/// weights.
///
/// Equivalent to `SpanQuery.getTermStates(Collection<SpanWeight>)`.
pub fn get_term_states_of(weights: &[Arc<dyn SpanWeight>]) -> TermStatesMap {
    let mut terms = TermStatesMap::new();
    for w in weights {
        w.extract_term_states(&mut terms);
    }
    terms
}

/// `visitor.acceptField(field)` for a span query's possibly-`null` field.
///
/// Java passes the `null` field of a
/// [`SpanOrQuery`](crate::spans::span_or_query::SpanOrQuery) or a
/// [`SpanNearQuery`](crate::spans::span_near_query::SpanNearQuery) without
/// clauses to `QueryVisitor.acceptField(String)` (`SpanOrQuery.java:86`,
/// `SpanNearQuery.java:279`); [`QueryVisitor::accept_field`] takes a `&str`,
/// so the `null` argument goes to [`QueryVisitor::accept_null_field`], which
/// answers as Java's `acceptField(null)` does.
pub(crate) fn accept_field(visitor: &dyn QueryVisitor, field: Option<&str>) -> bool {
    match field {
        Some(field) => visitor.accept_field(field),
        None => visitor.accept_null_field(),
    }
}

/// `List.equals` over two lists of span queries: same length and pairwise
/// [`Query::query_eq`].
pub(crate) fn span_query_list_eq(a: &[Arc<dyn SpanQuery>], b: &[Arc<dyn SpanQuery>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.query_eq(&**y as &dyn Query))
}

/// `List.hashCode()` over a list of span queries: `31 * h + e.hashCode()` from
/// `1`.
pub(crate) fn span_query_list_hash(queries: &[Arc<dyn SpanQuery>]) -> i32 {
    queries
        .iter()
        .fold(1i32, |h, q| h.wrapping_mul(31).wrapping_add(q.query_hash()))
}
