//! The Indri conjunction weight, ported from
//! `org.apache.lucene.search.IndriAndWeight`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::index_searcher::IndexSearcher;
use crate::search::indri_and_scorer::new_indri_and_scorer;
use crate::search::indri_query::IndriQuery;
use crate::search::query::{query_to_string, Query};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::{DefaultScorerSupplier, Weight};

/// The weight of an [`IndriAndQuery`](crate::search::IndriAndQuery), used to
/// normalise, score and explain it.
///
/// Equivalent to `org.apache.lucene.search.IndriAndWeight`.
#[derive(Debug)]
pub struct IndriAndWeight {
    parent_query: Arc<dyn Query>,
    query: IndriQuery,
    weights: Vec<Arc<dyn Weight>>,
    score_mode: ScoreMode,
    boost: f32,
}

impl IndriAndWeight {
    /// Weights every clause of `query` with the given searcher.
    ///
    /// Equivalent to
    /// `new IndriAndWeight(IndriAndQuery, IndexSearcher, ScoreMode, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while weighting a clause.
    pub fn new(
        parent_query: Arc<dyn Query>,
        query: IndriQuery,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Self> {
        let mut weights = Vec::with_capacity(query.get_clauses().len());
        for clause in query.iter() {
            weights.push(searcher.create_weight(Arc::clone(clause.query()), score_mode, 1.0)?);
        }
        Ok(Self {
            parent_query,
            query,
            weights,
            score_mode,
            boost,
        })
    }

    /// Builds the scorer of one leaf.
    ///
    /// Equivalent to the private
    /// `IndriAndWeight.getScorer(LeafReaderContext)`.
    fn get_scorer(&self, context: &LeafReaderContext) -> Result<Option<Box<dyn Scorer>>> {
        let mut sub_scorers = Vec::with_capacity(self.weights.len());
        for weight in &self.weights {
            if let Some(scorer) = weight.scorer(context)? {
                sub_scorers.push(scorer);
            }
        }

        if sub_scorers.is_empty() {
            return Ok(None);
        }
        if sub_scorers.len() == 1 {
            return Ok(sub_scorers.pop());
        }
        Ok(Some(Box::new(new_indri_and_scorer(
            sub_scorers,
            self.score_mode,
            self.boost,
        ))))
    }
}

impl SegmentCacheable for IndriAndWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.weights.iter().all(|weight| weight.is_cacheable(ctx))
    }
}

impl Weight for IndriAndWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.parent_query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let mut subs = Vec::new();
        let mut fail = false;
        for (weight, clause) in self.weights.iter().zip(self.query.iter()) {
            let explanation = weight.explain(context, doc)?;
            if explanation.is_match() {
                subs.push(explanation);
            } else if clause.is_required() {
                subs.push(Explanation::no_match(
                    format!(
                        "no match on required clause ({})",
                        query_to_string(clause.query().as_ref())
                    ),
                    vec![explanation],
                ));
                fail = true;
            }
        }
        if fail {
            return Ok(Explanation::no_match(
                "Failure to meet condition(s) of required/prohibited clause(s)",
                subs,
            ));
        }
        match self.scorer(context)? {
            Some(mut scorer) => {
                let advanced = scorer.iterator().advance(doc)?;
                debug_assert_eq!(advanced, doc);
                Ok(Explanation::matched(scorer.score()?, "sum of:", subs))
            }
            None => Ok(Explanation::no_match(
                "Failure to meet condition(s) of required/prohibited clause(s)",
                subs,
            )),
        }
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        match self.get_scorer(context)? {
            None => Ok(None),
            Some(scorer) => Ok(Some(Box::new(DefaultScorerSupplier::new(scorer)))),
        }
    }
}
