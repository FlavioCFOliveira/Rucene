//! Blending index statistics across terms, ported from
//! `org.apache.lucene.search.BlendedTermQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{IndexReaderContext, Term};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::boost_query::BoostQuery;
use crate::search::disjunction_max_query::DisjunctionMaxQuery;
use crate::search::index_searcher::{IndexSearcher, TooManyClauses};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::term_query::TermQuery;
use crate::search::term_states::TermStates;

/// Defines how the queries for the individual terms of a
/// [`BlendedTermQuery`] are merged.
///
/// Equivalent to the abstract nested class `BlendedTermQuery.RewriteMethod`.
pub trait BlendedRewriteMethod: Send + Sync + Debug {
    /// Merges the provided sub-queries into a single [`Query`].
    ///
    /// Equivalent to `BlendedTermQuery.RewriteMethod.rewrite(Query[])`.
    ///
    /// # Errors
    ///
    /// Propagates the errors the merged query's constructor raises.
    fn rewrite(&self, sub_queries: Vec<Arc<dyn Query>>) -> Result<Arc<dyn Query>>;

    /// Returns this method as [`Any`], so that
    /// [`method_eq`](Self::method_eq) can recover the concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Rewrite-method equivalence.
    ///
    /// Equivalent to `RewriteMethod.equals(Object)`, which the anonymous
    /// `BOOLEAN_REWRITE` inherits from `Object` — identity, which for a
    /// singleton of a unique class is the same as comparing the class.
    fn method_eq(&self, other: &dyn BlendedRewriteMethod) -> bool {
        self.as_any().type_id() == other.as_any().type_id()
    }

    /// Rewrite-method hash code, consistent with [`method_eq`](Self::method_eq).
    ///
    /// Equivalent to `RewriteMethod.hashCode()`.
    fn method_hash(&self) -> u64;
}

/// Hashes a [`std::any::TypeId`], which stands in for Java's
/// `getClass().hashCode()`.
fn type_hash<T: ?Sized + Any>(value: &T) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    value.type_id().hash(&mut hasher);
    hasher.finish()
}

/// A [`BlendedRewriteMethod`] that adds all sub-queries to a
/// [`BooleanQuery`](crate::search::BooleanQuery).
///
/// Equivalent to the anonymous class behind
/// `BlendedTermQuery.BOOLEAN_REWRITE`. It is useful when matching on several
/// fields is considered better than having a good match on a single field.
#[derive(Debug, Default, Clone, Copy)]
pub struct BooleanRewrite;

impl BlendedRewriteMethod for BooleanRewrite {
    fn rewrite(&self, sub_queries: Vec<Arc<dyn Query>>) -> Result<Arc<dyn Query>> {
        let mut merged = BooleanQueryBuilder::new();
        for query in sub_queries {
            merged.add(query, Occur::SHOULD)?;
        }
        Ok(Arc::new(merged.build()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_hash(&self) -> u64 {
        type_hash(self)
    }
}

/// A [`BlendedRewriteMethod`] that creates a
/// [`DisjunctionMaxQuery`] out of the sub-queries.
///
/// Equivalent to `BlendedTermQuery.DisjunctionMaxRewrite`. It is useful when
/// having a good match on a single field is considered better than having
/// average matches on several fields.
#[derive(Debug, Clone, Copy)]
pub struct DisjunctionMaxRewrite {
    tie_breaker_multiplier: f32,
}

impl DisjunctionMaxRewrite {
    /// Creates instances of [`DisjunctionMaxQuery`] with the provided tie
    /// breaker.
    ///
    /// Equivalent to `DisjunctionMaxRewrite(float)`.
    pub fn new(tie_breaker_multiplier: f32) -> Self {
        Self {
            tie_breaker_multiplier,
        }
    }
}

impl BlendedRewriteMethod for DisjunctionMaxRewrite {
    fn rewrite(&self, sub_queries: Vec<Arc<dyn Query>>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(DisjunctionMaxQuery::new(
            sub_queries,
            self.tie_breaker_multiplier,
        )?))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_eq(&self, other: &dyn BlendedRewriteMethod) -> bool {
        match other.as_any().downcast_ref::<DisjunctionMaxRewrite>() {
            Some(other) => self.tie_breaker_multiplier == other.tie_breaker_multiplier,
            None => false,
        }
    }

    fn method_hash(&self) -> u64 {
        type_hash(self)
            .wrapping_mul(31)
            .wrapping_add(u64::from(self.tie_breaker_multiplier.to_bits()))
    }
}

/// Returns the shared [`BooleanRewrite`].
///
/// Equivalent to the `BlendedTermQuery.BOOLEAN_REWRITE` constant.
pub fn boolean_rewrite() -> Arc<dyn BlendedRewriteMethod> {
    Arc::new(BooleanRewrite)
}

/// Returns a [`DisjunctionMaxRewrite`] with a tie breaker of `0.01`.
///
/// Equivalent to the `BlendedTermQuery.DISJUNCTION_MAX_REWRITE` constant.
pub fn disjunction_max_rewrite() -> Arc<dyn BlendedRewriteMethod> {
    Arc::new(DisjunctionMaxRewrite::new(0.01))
}

/// A builder for [`BlendedTermQuery`].
///
/// Equivalent to `BlendedTermQuery.Builder`.
#[derive(Debug, Clone)]
pub struct Builder {
    terms: Vec<Term>,
    boosts: Vec<f32>,
    contexts: Vec<Option<Arc<TermStates>>>,
    rewrite_method: Arc<dyn BlendedRewriteMethod>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// Creates an empty builder.
    ///
    /// Equivalent to the sole `Builder()` constructor.
    pub fn new() -> Self {
        Self {
            terms: Vec::new(),
            boosts: Vec::new(),
            contexts: Vec::new(),
            rewrite_method: disjunction_max_rewrite(),
        }
    }

    /// Sets the [`BlendedRewriteMethod`]; the default is
    /// [`disjunction_max_rewrite`].
    ///
    /// Equivalent to `Builder.setRewriteMethod(RewriteMethod)`.
    pub fn set_rewrite_method(
        &mut self,
        rewrite_method: Arc<dyn BlendedRewriteMethod>,
    ) -> &mut Self {
        self.rewrite_method = rewrite_method;
        self
    }

    /// Adds a term with a boost of `1`.
    ///
    /// Equivalent to `Builder.add(Term)`.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyClauses`] once
    /// [`IndexSearcher::get_max_clause_count`] terms have been added.
    pub fn add_term(&mut self, term: Term) -> Result<&mut Self> {
        self.add(term, 1.0, None)
    }

    /// Adds a term with the provided boost. The higher the boost, the more the
    /// term contributes to the overall score of the [`BlendedTermQuery`].
    ///
    /// Equivalent to `Builder.add(Term, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyClauses`] once
    /// [`IndexSearcher::get_max_clause_count`] terms have been added.
    pub fn add_boosted(&mut self, term: Term, boost: f32) -> Result<&mut Self> {
        self.add(term, boost, None)
    }

    /// Expert: adds a term with the provided boost and context, which is useful
    /// when a [`TermStates`] has already been built for the term.
    ///
    /// Equivalent to `Builder.add(Term, float, TermStates)`.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyClauses`] once
    /// [`IndexSearcher::get_max_clause_count`] terms have been added.
    pub fn add(
        &mut self,
        term: Term,
        boost: f32,
        context: Option<Arc<TermStates>>,
    ) -> Result<&mut Self> {
        if self.terms.len() >= IndexSearcher::get_max_clause_count() as usize {
            return Err(TooManyClauses::new().into());
        }
        self.terms.push(term);
        self.boosts.push(boost);
        self.contexts.push(context);
        Ok(self)
    }

    /// Builds the [`BlendedTermQuery`].
    ///
    /// Equivalent to `Builder.build()`.
    pub fn build(&self) -> BlendedTermQuery {
        BlendedTermQuery::new(
            self.terms.clone(),
            self.boosts.clone(),
            self.contexts.clone(),
            Arc::clone(&self.rewrite_method),
        )
    }
}

/// A [`Query`] that blends index statistics across several terms.
///
/// Equivalent to the `final org.apache.lucene.search.BlendedTermQuery`. It is
/// particularly useful when several terms should produce identical scores,
/// regardless of their index statistics — resolving synonyms at search time,
/// for instance, where the default behaviour tends to give higher scores to
/// rare terms. Cross-field search is another use case: searching `john` on
/// `first_name` and `last_name` without giving a higher weight to matches on
/// the field where `john` is rarer.
#[derive(Debug, Clone)]
pub struct BlendedTermQuery {
    terms: Vec<Term>,
    boosts: Vec<f32>,
    contexts: Vec<Option<Arc<TermStates>>>,
    rewrite_method: Arc<dyn BlendedRewriteMethod>,
}

impl BlendedTermQuery {
    /// Builds a blended query from its parallel arrays.
    ///
    /// Equivalent to the private
    /// `BlendedTermQuery(Term[], float[], TermStates[], RewriteMethod)`, which
    /// [`Builder::build`] calls. It is public here because Rust has no
    /// package-private visibility and because a builder is the only way to
    /// reach it in Java anyway.
    ///
    /// # Panics
    ///
    /// Panics when the three arrays do not have the same length, which Java
    /// asserts.
    pub fn new(
        terms: Vec<Term>,
        boosts: Vec<f32>,
        contexts: Vec<Option<Arc<TermStates>>>,
        rewrite_method: Arc<dyn BlendedRewriteMethod>,
    ) -> Self {
        assert_eq!(terms.len(), boosts.len());
        assert_eq!(terms.len(), contexts.len());

        // The terms are sorted so that equality and hashing do not depend on
        // the order. Java uses an `InPlaceMergeSorter`, which is stable, and so
        // is `sort_by_key`.
        let mut order: Vec<usize> = (0..terms.len()).collect();
        order.sort_by(|&a, &b| terms[a].cmp(&terms[b]));
        let mut sorted_terms = Vec::with_capacity(terms.len());
        let mut sorted_boosts = Vec::with_capacity(terms.len());
        let mut sorted_contexts = Vec::with_capacity(terms.len());
        for i in order {
            sorted_terms.push(terms[i].clone());
            sorted_boosts.push(boosts[i]);
            sorted_contexts.push(contexts[i].clone());
        }

        Self {
            terms: sorted_terms,
            boosts: sorted_boosts,
            contexts: sorted_contexts,
            rewrite_method,
        }
    }

    /// Adjusts a [`TermStates`] so that it reports the blended statistics.
    ///
    /// Equivalent to the private
    /// `BlendedTermQuery.adjustFrequencies(IndexReaderContext, TermStates, int, long)`.
    fn adjust_frequencies(
        reader_context: &Arc<dyn IndexReaderContext>,
        ctx: &TermStates,
        artificial_df: i32,
        artificial_ttf: i64,
    ) -> Result<TermStates> {
        let leaves = Arc::clone(reader_context).leaves();
        let mut new_ctx = TermStates::new(reader_context)?;
        for (i, leaf) in leaves.iter().enumerate() {
            let Some(term_state) = ctx.get(leaf)? else {
                continue;
            };
            new_ctx.register_state(term_state, i);
        }
        new_ctx.accumulate_statistics(artificial_df, artificial_ttf);
        Ok(new_ctx)
    }
}

impl Query for BlendedTermQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut builder = String::from("Blended(");
        for i in 0..self.terms.len() {
            if i != 0 {
                builder.push(' ');
            }
            let term_query: Arc<dyn Query> = Arc::new(TermQuery::new(self.terms[i].clone()));
            let term_query: Arc<dyn Query> = if self.boosts[i] != 1.0 {
                match BoostQuery::new(term_query, self.boosts[i]) {
                    Ok(boosted) => Arc::new(boosted),
                    // A boost that `BoostQuery` rejects cannot have been stored
                    // by the builder in the first place; print the bare term.
                    Err(_) => Arc::new(TermQuery::new(self.terms[i].clone())),
                }
            } else {
                term_query
            };
            builder.push_str(&term_query.to_query_string(field));
        }
        builder.push(')');
        builder
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let terms_to_visit: Vec<Term> = self
            .terms
            .iter()
            .filter(|t| visitor.accept_field(t.field()))
            .cloned()
            .collect();
        if !terms_to_visit.is_empty() {
            let mut v = visitor.get_sub_visitor(Occur::SHOULD, self);
            v.consume_terms(self, &terms_to_visit);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let top = index_searcher.get_top_reader_context();
        let mut contexts: Vec<Arc<TermStates>> = Vec::with_capacity(self.contexts.len());
        for (i, ctx) in self.contexts.iter().enumerate() {
            match ctx {
                Some(ctx) if ctx.was_built_for(top) => contexts.push(Arc::clone(ctx)),
                _ => contexts.push(Arc::new(TermStates::build(
                    index_searcher,
                    &self.terms[i],
                    true,
                )?)),
            }
        }

        // Compute aggregated doc freq and total term freq: `df` is the maximum
        // of all doc freqs and `ttf` the sum of all total term freqs.
        let mut df = 0i32;
        let mut ttf = 0i64;
        for ctx in &contexts {
            df = df.max(ctx.doc_freq()?);
            ttf += ctx.total_term_freq()?;
        }

        let mut adjusted: Vec<Arc<TermStates>> = Vec::with_capacity(contexts.len());
        for ctx in &contexts {
            adjusted.push(Arc::new(Self::adjust_frequencies(top, ctx, df, ttf)?));
        }

        let mut term_queries: Vec<Arc<dyn Query>> = Vec::with_capacity(self.terms.len());
        for (i, ctx) in adjusted.iter().enumerate() {
            let mut term_query: Arc<dyn Query> = Arc::new(TermQuery::with_states(
                self.terms[i].clone(),
                Arc::clone(ctx),
            ));
            if self.boosts[i] != 1.0 {
                term_query = Arc::new(BoostQuery::new(term_query, self.boosts[i])?);
            }
            term_queries.push(term_query);
        }
        Ok(Some(self.rewrite_method.rewrite(term_queries)?))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<BlendedTermQuery>() else {
            return false;
        };
        self.terms == other.terms
            && self.contexts.len() == other.contexts.len()
            && self
                .contexts
                .iter()
                .zip(&other.contexts)
                // Java compares `TermStates` with `Object.equals`, which is
                // reference identity; `Arc::ptr_eq` is exactly that.
                .all(|(a, b)| match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                })
            && self.boosts.len() == other.boosts.len()
            && self
                .boosts
                .iter()
                .zip(&other.boosts)
                .all(|(a, b)| a.to_bits() == b.to_bits())
            && self.rewrite_method.method_eq(&*other.rewrite_method)
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = self.class_hash();
        let mut hasher = DefaultHasher::new();
        for term in &self.terms {
            term.field().hash(&mut hasher);
            term.bytes().slice().hash(&mut hasher);
        }
        h = h.wrapping_mul(31).wrapping_add(hasher.finish());
        // Java's `Arrays.hashCode(contexts)` mixes the identity hash codes of
        // the `TermStates`, which are arbitrary; the pointer addresses are the
        // same kind of value and are consistent with `query_eq`.
        let mut hasher = DefaultHasher::new();
        for ctx in &self.contexts {
            match ctx {
                None => 0usize.hash(&mut hasher),
                Some(ctx) => (Arc::as_ptr(ctx) as usize).hash(&mut hasher),
            }
        }
        h = h.wrapping_mul(31).wrapping_add(hasher.finish());
        let mut hasher = DefaultHasher::new();
        for boost in &self.boosts {
            boost.to_bits().hash(&mut hasher);
        }
        h = h.wrapping_mul(31).wrapping_add(hasher.finish());
        h.wrapping_mul(31)
            .wrapping_add(self.rewrite_method.method_hash())
    }
}
