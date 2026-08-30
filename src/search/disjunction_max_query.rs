//! Disjunction-max queries, ported from
//! `org.apache.lucene.search.DisjunctionMaxQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cell::Cell;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::bulk_scorer::{BulkScorer, DefaultBulkScorer};
use crate::search::constant_score_weight::java_float_to_string;
use crate::search::disjunction_max_bulk_scorer::DisjunctionMaxBulkScorer;
use crate::search::disjunction_max_scorer::DisjunctionMaxScorer;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::matches::{Matches, MatchesUtils};
use crate::search::query::{Query, QueryKey};
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::Weight;
use crate::search::Multiset;

/// The number of clauses above which a disjunction-max query is not cached.
///
/// Equivalent to
/// `AbstractMultiTermQueryConstantScoreWrapper.BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD`,
/// which is 16; that class is not ported yet, so the constant is spelled out
/// here, cited from `AbstractMultiTermQueryConstantScoreWrapper.java:43`.
const BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD: usize = 16;

/// A query that generates the union of the documents produced by its
/// subqueries, and that scores each document with the maximum score any
/// subquery produced for it, plus a tie-breaking increment for every additional
/// matching subquery.
///
/// Equivalent to the `final org.apache.lucene.search.DisjunctionMaxQuery`,
/// which also implements `Iterable<Query>`; [`get_disjuncts`](Self::get_disjuncts)
/// is the Rust way to iterate the subqueries.
///
/// This is useful when searching for a word in several fields with different
/// boost factors, so that the fields cannot equivalently be combined into a
/// single search field: the primary score should be the one associated with the
/// highest boost, not the sum of the field scores that a boolean query would
/// give. The tie breaker lets results that include the same term in several
/// fields be judged better than results that include the term only in the best
/// of those fields, without confusing that with the better case of two
/// different terms in the several fields.
#[derive(Debug, Clone)]
pub struct DisjunctionMaxQuery {
    /// The subqueries, deduplicated the way `equals` and `hashCode` need.
    disjuncts: Multiset<QueryKey>,
    /// The subqueries in the order the caller supplied them; used by
    /// [`to_query_string`](Query::to_query_string).
    ordered_queries: Vec<Arc<dyn Query>>,
    /// The multiple of the non-maximum disjunct scores added into the final
    /// score; a non-zero value supports tie-breaking.
    tie_breaker_multiplier: f32,
}

impl DisjunctionMaxQuery {
    /// Creates a new disjunction-max query.
    ///
    /// Equivalent to
    /// `DisjunctionMaxQuery(Collection<? extends Query>, float)`.
    ///
    /// * `disjuncts` — every disjunct query to add;
    /// * `tie_breaker_multiplier` — the score of each non-maximum disjunct for
    ///   a document is multiplied by this weight and added into the final score.
    ///   If non-zero, the value should be small, on the order of `0.1`, which
    ///   says that ten occurrences of a word in a lower-scored field that is
    ///   also in a higher-scored field is just as good as a unique word in the
    ///   lower-scored field.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `tie_breaker_multiplier`
    /// falls outside `[0, 1]`, which is the `IllegalArgumentException` Java
    /// throws.
    pub fn new(disjuncts: Vec<Arc<dyn Query>>, tie_breaker_multiplier: f32) -> Result<Self> {
        // Not `!(0.0..=1.0).contains(..)`: Java's two comparisons both answer
        // false for NaN, so a NaN tie breaker is accepted, and `contains` would
        // reject it.
        #[allow(clippy::manual_range_contains)]
        if tie_breaker_multiplier < 0.0 || tie_breaker_multiplier > 1.0 {
            return Err(LuceneError::IllegalArgument(
                "tieBreakerMultiplier must be in [0, 1]".to_string(),
            ));
        }
        let mut set = Multiset::new();
        for query in &disjuncts {
            set.add(QueryKey::new(Arc::clone(query)));
        }
        Ok(Self {
            disjuncts: set,
            // order from the caller
            ordered_queries: disjuncts,
            tie_breaker_multiplier,
        })
    }

    /// Returns the disjuncts, in the order the caller supplied them.
    ///
    /// Equivalent to `DisjunctionMaxQuery.getDisjuncts()`, and to the
    /// `Iterable<Query>` the class implements.
    pub fn get_disjuncts(&self) -> &[Arc<dyn Query>] {
        &self.ordered_queries
    }

    /// Returns the tie-breaker value used for multiple matches.
    ///
    /// Equivalent to `DisjunctionMaxQuery.getTieBreakerMultiplier()`.
    pub fn get_tie_breaker_multiplier(&self) -> f32 {
        self.tie_breaker_multiplier
    }

    /// Returns the disjuncts in the order Java's `Multiset` iteration yields
    /// them, which is what `rewrite`, `visit` and the weight walk.
    fn disjunct_set(&self) -> Vec<Arc<dyn Query>> {
        self.disjuncts
            .iter()
            .map(|key| Arc::clone(key.query()))
            .collect()
    }
}

/// The scorer supplier of a [`DisjunctionMaxQuery`] with more than one clause.
///
/// Equivalent to the anonymous `ScorerSupplier` in
/// `DisjunctionMaxQuery.DisjunctionMaxWeight.scorerSupplier`.
struct DisjunctionMaxScorerSupplier {
    scorer_suppliers: Vec<Box<dyn ScorerSupplier>>,
    tie_breaker_multiplier: f32,
    score_mode: ScoreMode,
    /// Memoises [`ScorerSupplier::cost`]; `-1` means "not computed yet", as
    /// Java's `long cost = -1` field does.
    cost: Cell<i64>,
}

impl ScorerSupplier for DisjunctionMaxScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let mut scorers = Vec::with_capacity(self.scorer_suppliers.len());
        for supplier in &mut self.scorer_suppliers {
            scorers.push(supplier.get(lead_cost)?);
        }
        Ok(Box::new(DisjunctionMaxScorer::new(
            self.tie_breaker_multiplier,
            scorers,
            self.score_mode,
            lead_cost,
        )?))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        if self.tie_breaker_multiplier == 0.0 && self.score_mode == ScoreMode::TOP_SCORES {
            let mut scorers = Vec::with_capacity(self.scorer_suppliers.len());
            for supplier in &mut self.scorer_suppliers {
                scorers.push(supplier.bulk_scorer()?);
            }
            return Ok(Box::new(DisjunctionMaxBulkScorer::new(scorers)?));
        }
        // Reproduces `ScorerSupplier`'s default, which Java reaches with
        // `super.bulkScorer()`.
        Ok(Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?)))
    }

    fn cost(&self) -> i64 {
        if self.cost.get() == -1 {
            let mut cost: i64 = 0;
            for supplier in &self.scorer_suppliers {
                cost = cost.wrapping_add(supplier.cost());
            }
            self.cost.set(cost);
        }
        self.cost.get()
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        if self.tie_breaker_multiplier == 0.0 {
            for supplier in &mut self.scorer_suppliers {
                // sub scorers need to be able to skip too as calls to
                // setMinCompetitiveScore get propagated
                supplier.set_top_level_scoring_clause()?;
            }
        }
        Ok(())
    }
}

/// Expert: the [`Weight`] of a [`DisjunctionMaxQuery`], used to normalise,
/// score and explain these queries.
///
/// Equivalent to the `protected class
/// DisjunctionMaxQuery.DisjunctionMaxWeight`.
pub struct DisjunctionMaxWeight {
    query: Arc<dyn Query>,
    /// The weights of the subqueries, in one-to-one correspondence with the
    /// disjuncts.
    weights: Vec<Arc<dyn Weight>>,
    tie_breaker_multiplier: f32,
    score_mode: ScoreMode,
}

impl std::fmt::Debug for DisjunctionMaxWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisjunctionMaxWeight")
            .field("weights", &self.weights.len())
            .field("tie_breaker_multiplier", &self.tie_breaker_multiplier)
            .field("score_mode", &self.score_mode)
            .finish()
    }
}

impl DisjunctionMaxWeight {
    /// Builds the weight of a disjunction-max query, recursively building the
    /// weight of every subquery.
    ///
    /// Equivalent to
    /// `DisjunctionMaxWeight(IndexSearcher, ScoreMode, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while building a subquery's weight.
    pub fn new(
        query: &DisjunctionMaxQuery,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Self> {
        let mut weights = Vec::new();
        for disjunct_query in query.disjunct_set() {
            weights.push(searcher.create_weight(disjunct_query, score_mode, boost)?);
        }
        Ok(Self {
            query: Arc::new(query.clone()),
            weights,
            tie_breaker_multiplier: query.tie_breaker_multiplier,
            score_mode,
        })
    }

    /// Returns the weights of the subqueries.
    ///
    /// Equivalent to reading the `protected final ArrayList<Weight> weights`
    /// field.
    pub fn weights(&self) -> &[Arc<dyn Weight>] {
        &self.weights
    }
}

impl SegmentCacheable for DisjunctionMaxWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        if self.weights.len() > BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD {
            // Disallow caching large dismax queries to not encourage users to
            // build large dismax queries as a workaround to the fact that we
            // disallow caching large TermInSetQueries.
            return false;
        }
        for weight in &self.weights {
            if !weight.is_cacheable(ctx) {
                return false;
            }
        }
        true
    }
}

impl Weight for DisjunctionMaxWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let mut mis = Vec::new();
        for weight in &self.weights {
            if let Some(mi) = weight.matches(context, doc)? {
                mis.push(mi);
            }
        }
        Ok(MatchesUtils::from_sub_matches(mis))
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let mut scorer_suppliers = Vec::new();
        for weight in &self.weights {
            if let Some(supplier) = weight.scorer_supplier(context)? {
                scorer_suppliers.push(supplier);
            }
        }

        if scorer_suppliers.is_empty() {
            Ok(None)
        } else if scorer_suppliers.len() == 1 {
            Ok(Some(scorer_suppliers.pop().expect(
                "INVARIANT: the vector was just observed to hold one element",
            )))
        } else {
            Ok(Some(Box::new(DisjunctionMaxScorerSupplier {
                scorer_suppliers,
                tie_breaker_multiplier: self.tie_breaker_multiplier,
                score_mode: self.score_mode,
                cost: Cell::new(-1),
            })))
        }
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let mut matched = false;
        let mut max = 0.0f64;
        let mut other_sum = 0.0f64;
        let mut subs_on_match: Vec<Explanation> = Vec::new();
        let mut subs_on_no_match: Vec<Explanation> = Vec::new();
        for weight in &self.weights {
            let explanation = weight.explain(context, doc)?;
            if explanation.is_match() {
                matched = true;
                let score = f64::from(explanation.value().float_value());
                subs_on_match.push(explanation);
                if score >= max {
                    other_sum += max;
                    max = score;
                } else {
                    other_sum += score;
                }
            } else if !matched {
                subs_on_no_match.push(explanation);
            }
        }
        if matched {
            let score = (max + other_sum * f64::from(self.tie_breaker_multiplier)) as f32;
            let desc = if self.tie_breaker_multiplier == 0.0 {
                "max of:".to_string()
            } else {
                format!(
                    "max plus {} times others of:",
                    java_float_to_string(self.tie_breaker_multiplier)
                )
            };
            Ok(Explanation::matched(score, desc, subs_on_match))
        } else {
            Ok(Explanation::no_match(
                "No matching clause",
                subs_on_no_match,
            ))
        }
    }
}

impl Query for DisjunctionMaxQuery {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(DisjunctionMaxWeight::new(
            self, searcher, score_mode, boost,
        )?))
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.disjuncts.is_empty() {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                "empty DisjunctionMaxQuery",
            ))));
        }

        let disjuncts = self.disjunct_set();

        if disjuncts.len() == 1 {
            return Ok(Some(Arc::clone(&disjuncts[0])));
        }

        if self.tie_breaker_multiplier == 1.0 {
            let mut builder = BooleanQueryBuilder::new();
            for sub in &disjuncts {
                builder.add(Arc::clone(sub), Occur::SHOULD)?;
            }
            return Ok(Some(Arc::new(builder.build())));
        }

        let mut actually_rewritten = false;
        let mut rewritten_disjuncts: Vec<Arc<dyn Query>> = Vec::new();
        for sub in &disjuncts {
            let rewritten_sub = sub.rewrite(searcher)?;
            let changed = rewritten_sub.is_some();
            let rewritten_sub = rewritten_sub.unwrap_or_else(|| Arc::clone(sub));
            let is_match_no_docs = rewritten_sub.as_any().is::<MatchNoDocsQuery>();
            if changed || is_match_no_docs {
                actually_rewritten = true;
            }
            if !is_match_no_docs {
                rewritten_disjuncts.push(rewritten_sub);
            }
        }

        if !actually_rewritten {
            return Ok(None);
        }

        if rewritten_disjuncts.is_empty() {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                "empty DisjunctionMaxQuery",
            ))));
        }

        if rewritten_disjuncts.len() == 1 {
            return Ok(Some(Arc::clone(&rewritten_disjuncts[0])));
        }

        Ok(Some(Arc::new(DisjunctionMaxQuery::new(
            rewritten_disjuncts,
            self.tie_breaker_multiplier,
        )?)))
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::SHOULD, self);
        for query in self.disjunct_set() {
            query.visit(&mut *sub);
        }
    }

    fn to_query_string(&self, field: &str) -> String {
        let rendered: Vec<String> = self
            .ordered_queries
            .iter()
            .map(|subquery| {
                if subquery
                    .as_any()
                    .is::<crate::search::boolean_query::BooleanQuery>()
                {
                    // wrap sub-bools in parens
                    format!("({})", subquery.to_query_string(field))
                } else {
                    subquery.to_query_string(field)
                }
            })
            .collect();
        let suffix = if self.tie_breaker_multiplier != 0.0 {
            format!("~{}", java_float_to_string(self.tie_breaker_multiplier))
        } else {
            String::new()
        };
        format!("({}){}", rendered.join(" | "), suffix)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<DisjunctionMaxQuery>() else {
            return false;
        };
        self.tie_breaker_multiplier == other.tie_breaker_multiplier
            && self.disjuncts == other.disjuncts
    }

    fn query_hash(&self) -> u64 {
        let mut h = self.class_hash();
        h = h
            .wrapping_mul(31)
            .wrapping_add(u64::from(self.tie_breaker_multiplier.to_bits()));
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&self.disjuncts, &mut hasher);
        h = h
            .wrapping_mul(31)
            .wrapping_add(std::hash::Hasher::finish(&hasher));
        h
    }
}
