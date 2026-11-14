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

//! Port of `SpanMultiTermQueryWrapper.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{Term, TermStates};
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{MultiTermQuery, RewriteMethod};
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scoring_rewrite::{scoring_rewrite, ScoringRewrite};
use rucene::search::term_collecting_rewrite::{TermCollectingRewrite, TopLevelBuilder};
use rucene::search::top_terms_rewrite::{
    top_terms_rewrite, top_terms_rewrite_eq, top_terms_rewrite_hash, TopTermsRewrite,
};
use rucene::search::weight::Weight;
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::spans::span_or_query::SpanOrQuery;
use crate::spans::span_query::{accept_field, SpanQuery};
use crate::spans::span_term_query::SpanTermQuery;
use crate::spans::span_weight::SpanWeight;

/// Wraps any [`MultiTermQuery`] as a [`SpanQuery`], so it can be nested within
/// other span query classes.
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanMultiTermQueryWrapper`.
/// Its type parameter `Q extends MultiTermQuery`, which only types the
/// protected `query` field, is erased to `dyn MultiTermQuery`.
///
/// The query is rewritten by default to a
/// [`SpanOrQuery`] containing the expanded terms, but this can be customized —
/// see [`set_rewrite_method`](Self::set_rewrite_method).
#[derive(Debug, Clone)]
pub struct SpanMultiTermQueryWrapper {
    /// The protected final `query` field.
    query: Arc<dyn MultiTermQuery>,
    rewrite_method: Arc<dyn SpanRewriteMethod>,
}

impl SpanMultiTermQueryWrapper {
    /// Creates a new wrapper.
    ///
    /// Equivalent to `SpanMultiTermQueryWrapper(Q)`.
    pub fn new(query: Arc<dyn MultiTermQuery>) -> Self {
        let rewrite_method = Self::select_rewrite_method(&*query);
        Self {
            query,
            rewrite_method,
        }
    }

    /// Equivalent to the private static
    /// `SpanMultiTermQueryWrapper.selectRewriteMethod(MultiTermQuery)`, whose
    /// `method instanceof TopTermsRewrite` test is
    /// [`RewriteMethod::as_top_terms_rewrite`].
    fn select_rewrite_method(query: &dyn MultiTermQuery) -> Arc<dyn SpanRewriteMethod> {
        let method = query.get_rewrite_method();
        match method.as_top_terms_rewrite() {
            Some(top_terms) => {
                let pqsize = top_terms.get_size();
                Arc::new(TopTermsSpanBooleanQueryRewrite::new(pqsize))
            }
            None => scoring_span_query_rewrite(),
        }
    }

    /// Expert: returns the rewrite method.
    ///
    /// Equivalent to the final `SpanMultiTermQueryWrapper.getRewriteMethod()`.
    pub fn get_rewrite_method(&self) -> &Arc<dyn SpanRewriteMethod> {
        &self.rewrite_method
    }

    /// Expert: sets the rewrite method. This only makes sense to be a span
    /// rewrite method.
    ///
    /// Equivalent to the final
    /// `SpanMultiTermQueryWrapper.setRewriteMethod(SpanRewriteMethod)`.
    pub fn set_rewrite_method(&mut self, rewrite_method: Arc<dyn SpanRewriteMethod>) {
        self.rewrite_method = rewrite_method;
    }

    /// Returns the wrapped query.
    ///
    /// Equivalent to `SpanMultiTermQueryWrapper.getWrappedQuery()`.
    pub fn get_wrapped_query(&self) -> Arc<dyn Query> {
        self.query.to_query_arc()
    }
}

impl Query for SpanMultiTermQueryWrapper {
    fn to_query_string(&self, field: &str) -> String {
        let mut builder = String::new();
        builder.push_str("SpanMultiTermQueryWrapper(");
        let query_str = self.query.to_query_string(field);
        builder.push_str(&query_str);
        builder.push(')');
        builder
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if accept_field(visitor, Some(self.query.get_field())) {
            self.query
                .visit(&mut *visitor.get_sub_visitor(Occur::MUST, self));
        }
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
        Ok(self.create_span_weight(searcher, score_mode, boost)?)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.rewrite_span(searcher)?.map(|q| q as Arc<dyn Query>))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<SpanMultiTermQueryWrapper>()
                .is_some_and(|other| self.query.query_eq(&*other.query as &dyn Query))
    }

    /// `classHash() * 31 + query.hashCode()`.
    fn query_hash(&self) -> i32 {
        self.class_hash()
            .wrapping_mul(31)
            .wrapping_add(self.query.query_hash())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanMultiTermQueryWrapper"
    }
}

impl SpanQuery for SpanMultiTermQueryWrapper {
    fn get_field(&self) -> Option<&str> {
        Some(self.query.get_field())
    }

    fn create_span_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        Err(LuceneError::illegal_argument("Rewrite first!".to_string()))
    }

    /// Equivalent to `SpanMultiTermQueryWrapper.rewrite(IndexSearcher)`, which
    /// always returns what the rewrite method builds.
    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        Ok(Some(
            self.rewrite_method
                .rewrite_span(searcher, Arc::clone(&self.query))?,
        ))
    }
}

/// Defines how the query is rewritten.
///
/// Equivalent to the abstract static nested class
/// `org.apache.lucene.queries.spans.SpanMultiTermQueryWrapper.SpanRewriteMethod`,
/// which extends `MultiTermQuery.RewriteMethod` and narrows `rewrite` to return
/// a `SpanQuery`. The narrowed method is
/// [`rewrite_span`](Self::rewrite_span); every implementation's
/// [`RewriteMethod::rewrite`] forwards to it.
pub trait SpanRewriteMethod: RewriteMethod {
    /// Rewrites the query into a span query.
    ///
    /// Equivalent to the covariant
    /// `SpanRewriteMethod.rewrite(IndexSearcher, MultiTermQuery)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting.
    fn rewrite_span(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn SpanQuery>>;
}

/// `(SpanQuery) delegate.rewrite(indexSearcher, query)`: the delegates of the
/// span rewrite methods build a [`SpanOrQuery`].
fn to_span_query(query: Arc<dyn Query>) -> Result<Arc<dyn SpanQuery>> {
    match query.as_any().downcast_ref::<SpanOrQuery>() {
        Some(span_or_query) => Ok(Arc::new(span_or_query.clone())),
        None => Err(LuceneError::from(
            rucene::error::LuceneErrorKind::Unchecked {
                class: "java.lang.ClassCastException",
                message: Some(format!(
                    "class {} cannot be cast to class org.apache.lucene.queries.spans.SpanQuery",
                    query.java_class_name()
                )),
                cause: None,
            },
        )),
    }
}

/// The top-level builder both span rewrite methods use: a list of span
/// queries, built into a [`SpanOrQuery`].
///
/// Equivalent to the `getTopLevelBuilder()`, `build(List<SpanQuery>)` and
/// `addClause(List<SpanQuery>, Term, int, float, TermStates)` overrides the
/// two anonymous delegates share.
#[derive(Debug, Default)]
struct SpanQueryListBuilder {
    clauses: Vec<Arc<dyn SpanQuery>>,
}

impl TopLevelBuilder for SpanQueryListBuilder {
    fn add_clause(
        &mut self,
        term: Term,
        _doc_count: i32,
        _boost: f32,
        states: Option<Arc<TermStates>>,
    ) -> Result<()> {
        let q = SpanTermQuery::with_term_states(term, states);
        self.clauses.push(Arc::new(q));
        Ok(())
    }

    fn build(self: Box<Self>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(SpanOrQuery::new(self.clauses)?))
    }
}

/// The delegate of [`ScoringSpanQueryRewrite`].
///
/// Equivalent to the anonymous `ScoringRewrite<List<SpanQuery>>` held by the
/// private `delegate` field of `SCORING_SPAN_QUERY_REWRITE`.
#[derive(Debug, Default, Clone, Copy)]
struct ScoringSpanQueryRewriteDelegate;

impl TermCollectingRewrite for ScoringSpanQueryRewriteDelegate {
    fn get_top_level_builder(&self) -> Result<Box<dyn TopLevelBuilder>> {
        Ok(Box::new(SpanQueryListBuilder::default()))
    }
}

impl ScoringRewrite for ScoringSpanQueryRewriteDelegate {
    fn check_max_clause_count(&self, _count: usize) -> Result<()> {
        // we accept all terms as SpanOrQuery has no limits
        Ok(())
    }
}

impl RewriteMethod for ScoringSpanQueryRewriteDelegate {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        scoring_rewrite(self, index_searcher, &*query)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_hash(&self) -> i32 {
        java_identity_hash_code_stand_in(
            "org.apache.lucene.queries.spans.SpanMultiTermQueryWrapper$1$1",
        )
    }
}

/// A rewrite method that first translates each term into a
/// [`SpanTermQuery`] in an [`Occur::SHOULD`] clause of a boolean query, and
/// keeps the scores as computed by the query.
///
/// Equivalent to the anonymous class behind
/// `SpanMultiTermQueryWrapper.SCORING_SPAN_QUERY_REWRITE`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScoringSpanQueryRewrite;

/// Returns the rewrite method that first translates each term into a
/// [`SpanTermQuery`] in an [`Occur::SHOULD`] clause of a boolean query, and
/// keeps the scores as computed by the query.
///
/// Equivalent to the constant `SpanMultiTermQueryWrapper.SCORING_SPAN_QUERY_REWRITE`.
pub fn scoring_span_query_rewrite() -> Arc<dyn SpanRewriteMethod> {
    Arc::new(ScoringSpanQueryRewrite)
}

impl SpanRewriteMethod for ScoringSpanQueryRewrite {
    fn rewrite_span(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn SpanQuery>> {
        to_span_query(ScoringSpanQueryRewriteDelegate.rewrite(index_searcher, query)?)
    }
}

impl RewriteMethod for ScoringSpanQueryRewrite {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        Ok(self.rewrite_span(index_searcher, query)?)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_hash(&self) -> i32 {
        java_identity_hash_code_stand_in(
            "org.apache.lucene.queries.spans.SpanMultiTermQueryWrapper.SCORING_SPAN_QUERY_REWRITE",
        )
    }
}

/// The delegate of [`TopTermsSpanBooleanQueryRewrite`].
///
/// Equivalent to the anonymous `TopTermsRewrite<List<SpanQuery>>` held by
/// `TopTermsSpanBooleanQueryRewrite`'s private `delegate` field.
#[derive(Debug, Clone, Copy)]
struct TopTermsSpanDelegate {
    size: i32,
}

impl TermCollectingRewrite for TopTermsSpanDelegate {
    fn get_top_level_builder(&self) -> Result<Box<dyn TopLevelBuilder>> {
        Ok(Box::new(SpanQueryListBuilder::default()))
    }
}

impl TopTermsRewrite for TopTermsSpanDelegate {
    fn get_size(&self) -> i32 {
        self.size
    }

    fn get_max_size(&self) -> i32 {
        i32::MAX
    }
}

impl RewriteMethod for TopTermsSpanDelegate {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        top_terms_rewrite(self, index_searcher, &*query)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_eq(&self, other: &dyn RewriteMethod) -> bool {
        match other.as_any().downcast_ref::<TopTermsSpanDelegate>() {
            Some(other) => top_terms_rewrite_eq(self, other),
            None => false,
        }
    }

    fn method_hash(&self) -> i32 {
        top_terms_rewrite_hash(self)
    }

    fn as_top_terms_rewrite(&self) -> Option<&dyn TopTermsRewrite> {
        Some(self)
    }
}

/// A rewrite method that first translates each term into a
/// [`SpanTermQuery`] in an [`Occur::SHOULD`] clause of a boolean query, and
/// keeps the scores as computed by the query.
///
/// Equivalent to the final static nested class
/// `org.apache.lucene.queries.spans.SpanMultiTermQueryWrapper.TopTermsSpanBooleanQueryRewrite`.
/// This rewrite method only uses the top scoring terms so it will not
/// overflow the boolean max clause count.
#[derive(Debug, Clone, Copy)]
pub struct TopTermsSpanBooleanQueryRewrite {
    delegate: TopTermsSpanDelegate,
}

impl TopTermsSpanBooleanQueryRewrite {
    /// Creates a rewrite for at most `size` terms.
    ///
    /// Equivalent to `TopTermsSpanBooleanQueryRewrite(int)`.
    pub fn new(size: i32) -> Self {
        Self {
            delegate: TopTermsSpanDelegate { size },
        }
    }

    /// Returns the maximum priority queue size.
    ///
    /// Equivalent to `TopTermsSpanBooleanQueryRewrite.getSize()`.
    pub fn get_size(&self) -> i32 {
        self.delegate.get_size()
    }
}

impl SpanRewriteMethod for TopTermsSpanBooleanQueryRewrite {
    fn rewrite_span(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn SpanQuery>> {
        to_span_query(self.delegate.rewrite(index_searcher, query)?)
    }
}

impl RewriteMethod for TopTermsSpanBooleanQueryRewrite {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        Ok(self.rewrite_span(index_searcher, query)?)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `TopTermsSpanBooleanQueryRewrite.equals(Object)`: the same
    /// class and equal delegates.
    fn method_eq(&self, other: &dyn RewriteMethod) -> bool {
        match other
            .as_any()
            .downcast_ref::<TopTermsSpanBooleanQueryRewrite>()
        {
            Some(other) => self.delegate.method_eq(&other.delegate),
            None => false,
        }
    }

    /// Equivalent to `TopTermsSpanBooleanQueryRewrite.hashCode()`,
    /// `31 * delegate.hashCode()`.
    fn method_hash(&self) -> i32 {
        31i32.wrapping_mul(self.delegate.method_hash())
    }
}
