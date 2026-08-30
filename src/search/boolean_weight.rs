//! The weight of a boolean query, ported from
//! `org.apache.lucene.search.BooleanWeight`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::boolean_clause::{BooleanClause, Occur};
use crate::search::boolean_query::BooleanQuery;
use crate::search::boolean_scorer_supplier::{BooleanScorerSupplier, ClauseSuppliers};
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::{Matches, MatchesUtils};
use crate::search::query::Query;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::{Explanation, Similarity};
use crate::search::weight::Weight;

/// The number of clauses above which a boolean query is not cached.
///
/// Equivalent to
/// `AbstractMultiTermQueryConstantScoreWrapper.BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD`,
/// which is 16. That class is not ported yet, so the constant is spelled out
/// here; it is the same number, cited from
/// `AbstractMultiTermQueryConstantScoreWrapper.java:43`.
const BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD: usize = 16;

/// A clause of a boolean query paired with the weight built for it.
///
/// Equivalent to the `protected static
/// BooleanWeight.WeightedBooleanClause`.
pub struct WeightedBooleanClause {
    /// The clause.
    pub clause: BooleanClause,
    /// The weight built from the clause's query.
    pub weight: Arc<dyn Weight>,
}

impl std::fmt::Debug for WeightedBooleanClause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeightedBooleanClause")
            .field("clause", &self.clause)
            .finish_non_exhaustive()
    }
}

/// Expert: the [`Weight`] of a [`BooleanQuery`], used to normalise, score and
/// explain these queries.
///
/// Equivalent to the `final org.apache.lucene.search.BooleanWeight`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility.
pub struct BooleanWeight {
    query: BooleanQuery,
    query_handle: Arc<dyn Query>,
    similarity: Arc<dyn Similarity>,
    weighted_clauses: Vec<WeightedBooleanClause>,
    score_mode: ScoreMode,
}

impl std::fmt::Debug for BooleanWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BooleanWeight")
            .field("query", &self.query)
            .field("score_mode", &self.score_mode)
            .finish_non_exhaustive()
    }
}

impl BooleanWeight {
    /// Builds the weight of a boolean query, recursively building the weight of
    /// every clause.
    ///
    /// Equivalent to
    /// `BooleanWeight(BooleanQuery, IndexSearcher, ScoreMode, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while building a clause's weight.
    pub fn new(
        query: BooleanQuery,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Self> {
        let similarity = Arc::clone(searcher.get_similarity());
        let mut weighted_clauses = Vec::with_capacity(query.clauses().len());
        for clause in query.clauses() {
            let clause_score_mode = if clause.is_scoring() {
                score_mode
            } else {
                ScoreMode::COMPLETE_NO_SCORES
            };
            let weight =
                searcher.create_weight(Arc::clone(clause.query()), clause_score_mode, boost)?;
            weighted_clauses.push(WeightedBooleanClause {
                clause: clause.clone(),
                weight,
            });
        }
        let query_handle: Arc<dyn Query> = Arc::new(query.clone());
        Ok(Self {
            query,
            query_handle,
            similarity,
            weighted_clauses,
            score_mode,
        })
    }

    /// Returns the similarity this weight was built against.
    ///
    /// Equivalent to reading the `final Similarity similarity` field, which
    /// Java stores for its subclasses' benefit.
    pub fn similarity(&self) -> &Arc<dyn Similarity> {
        &self.similarity
    }

    /// Returns the clauses of the query paired with their weights.
    ///
    /// Equivalent to reading the `final ArrayList<WeightedBooleanClause>
    /// weightedClauses` field.
    pub fn weighted_clauses(&self) -> &[WeightedBooleanClause] {
        &self.weighted_clauses
    }

    /// Returns the number of matches of the required clauses, `-1` when it is
    /// unknown, or the number of live documents when there is no required
    /// clause.
    ///
    /// Equivalent to the private `BooleanWeight.reqCount(LeafReaderContext)`.
    fn req_count(&self, context: &LeafReaderContext) -> Result<i32> {
        let num_docs = context.leaf_reader().num_docs();
        let mut req_count = num_docs;
        for weighted_clause in &self.weighted_clauses {
            if !weighted_clause.clause.is_required() {
                continue;
            }
            let count = weighted_clause.weight.count(context)?;
            if count == -1 || count == 0 {
                // If the count of one clause is unknown, then the count of the
                // conjunction is unknown too. If one clause doesn't match any
                // docs then the conjunction doesn't match any docs either.
                return Ok(count);
            } else if count == num_docs {
                // the query matches all docs, it can be safely ignored
            } else if req_count == num_docs {
                // all clauses seen so far match all docs, so the count of the
                // new clause is also the count of the conjunction
                req_count = count;
            } else {
                // We have two clauses whose count is in [1, numDocs), we can't
                // figure out the number of docs that match the conjunction
                // without running the query.
                return Ok(-1);
            }
        }
        Ok(req_count)
    }

    /// Returns the number of matches of the optional clauses, `-1` when it is
    /// unknown, or `0` when there is no optional clause.
    ///
    /// Equivalent to the private
    /// `BooleanWeight.optCount(LeafReaderContext, Occur)`.
    fn opt_count(&self, context: &LeafReaderContext, occur: Occur) -> Result<i32> {
        let num_docs = context.leaf_reader().num_docs();
        let mut opt_count = 0;
        let mut unknown_count = false;
        for weighted_clause in &self.weighted_clauses {
            if weighted_clause.clause.occur() != occur {
                continue;
            }
            let count = weighted_clause.weight.count(context)?;
            if count == -1 {
                // If one clause has a number of matches that is unknown, let's
                // be more aggressive to check whether remaining clauses could
                // match all docs.
                unknown_count = true;
                continue;
            } else if count == num_docs {
                // If either clause matches all docs, then the disjunction
                // matches all docs.
                return Ok(count);
            } else if count == 0 {
                // We can safely ignore this clause, it doesn't affect the count.
            } else if opt_count == 0 {
                // This is the first clause we see that has a non-zero count, it
                // becomes the count of the disjunction.
                opt_count = count;
            } else {
                // We have two clauses whose count is in [1, numDocs), we can't
                // figure out the number of docs that match the disjunction
                // without running the query.
                unknown_count = true;
            }
        }
        // If at least one of the clauses has a number of matches that is unknown
        // and no clause matches all docs, then the number of matches of the
        // disjunction is unknown.
        Ok(if unknown_count { -1 } else { opt_count })
    }
}

impl SegmentCacheable for BooleanWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        if self.query.clauses().len() > BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD {
            // Disallow caching large boolean queries to not encourage users to
            // build large boolean queries as a workaround to the fact that we
            // disallow caching large TermInSetQueries.
            return false;
        }
        for weighted_clause in &self.weighted_clauses {
            if !weighted_clause.weight.is_cacheable(ctx) {
                return false;
            }
        }
        true
    }
}

impl Weight for BooleanWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query_handle)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let min_should_match = self.query.get_minimum_number_should_match();
        let mut subs: Vec<Explanation> = Vec::new();
        let mut failing_optionals: Vec<Explanation> = Vec::new();
        let mut fail = false;
        let mut match_count = 0;
        let mut should_match_count = 0;
        for weighted_clause in &self.weighted_clauses {
            let weight = &weighted_clause.weight;
            let clause = &weighted_clause.clause;
            let explanation = weight.explain(context, doc)?;
            if explanation.is_match() {
                if clause.is_scoring() {
                    subs.push(explanation);
                } else if clause.is_required() {
                    subs.push(Explanation::matched(
                        0.0f32,
                        "match on required clause, product of:",
                        vec![
                            Explanation::matched(
                                0.0f32,
                                format!("{} clause", Occur::FILTER),
                                Vec::new(),
                            ),
                            explanation,
                        ],
                    ));
                } else if clause.is_prohibited() {
                    subs.push(Explanation::no_match(
                        format!(
                            "match on prohibited clause ({})",
                            clause.query().to_query_string("")
                        ),
                        vec![explanation],
                    ));
                    fail = true;
                }
                if !clause.is_prohibited() {
                    match_count += 1;
                }
                if clause.occur() == Occur::SHOULD {
                    should_match_count += 1;
                }
            } else if clause.is_required() {
                subs.push(Explanation::no_match(
                    format!(
                        "no match on required clause ({})",
                        clause.query().to_query_string("")
                    ),
                    vec![explanation],
                ));
                fail = true;
            } else if clause.occur() == Occur::SHOULD {
                failing_optionals.push(Explanation::no_match(
                    format!(
                        "no match on optional clause ({})",
                        clause.query().to_query_string("")
                    ),
                    vec![explanation],
                ));
            }
        }
        if fail {
            Ok(Explanation::no_match(
                "Failure to meet condition(s) of required/prohibited clause(s)",
                subs,
            ))
        } else if match_count == 0 {
            subs.extend(failing_optionals);
            Ok(Explanation::no_match("No matching clauses", subs))
        } else if should_match_count < min_should_match {
            subs.extend(failing_optionals);
            Ok(Explanation::no_match(
                format!(
                    "Failure to match minimum number of optional clauses: {min_should_match}, matched: {should_match_count}"
                ),
                subs,
            ))
        } else {
            // Replicating the same floating-point errors as the scorer does is
            // quite complex (essentially because of how ReqOptSumScorer casts
            // intermediate contributions to the score to floats), so in order to
            // make sure that explanations have the same value as the score, we
            // pull a scorer and use it to compute the score.
            let mut scorer = self
                .scorer(context)?
                .expect("INVARIANT: a matching document implies a scorer for this leaf");
            let advanced = scorer.iterator().advance(doc)?;
            debug_assert_eq!(advanced, doc);
            let score = scorer.score()?;
            Ok(Explanation::matched(score, "sum of:", subs))
        }
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let min_should_match = self.query.get_minimum_number_should_match();
        let mut matches: Vec<Arc<dyn Matches>> = Vec::new();
        let mut should_match_count = 0;
        for weighted_clause in &self.weighted_clauses {
            let weight = &weighted_clause.weight;
            let clause = &weighted_clause.clause;
            let clause_matches = weight.matches(context, doc)?;
            if clause.is_prohibited() && clause_matches.is_some() {
                return Ok(None);
            }
            if clause.is_required() {
                let Some(clause_matches) = clause_matches else {
                    return Ok(None);
                };
                matches.push(clause_matches);
            } else if clause.occur() == Occur::SHOULD {
                if let Some(clause_matches) = clause_matches {
                    matches.push(clause_matches);
                    should_match_count += 1;
                }
            }
        }
        if should_match_count < min_should_match {
            return Ok(None);
        }
        Ok(MatchesUtils::from_sub_matches(matches))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let num_docs = context.leaf_reader().num_docs();
        if self.query.is_pure_disjunction() {
            return self.opt_count(context, Occur::SHOULD);
        }
        let positive_count = if (self.query.clause_count(Occur::FILTER) != 0
            || self.query.clause_count(Occur::MUST) != 0)
            && self.query.get_minimum_number_should_match() == 0
        {
            self.req_count(context)?
        } else {
            // The query has a non-zero min-should match. We could handle some
            // cases, e.g. minShouldMatch=N and we can find N SHOULD clauses that
            // match all docs, but are there real-world queries that would
            // benefit from Lucene handling this case?
            -1
        };

        if positive_count == 0 {
            return Ok(0);
        }

        let prohibited_count = self.opt_count(context, Occur::MUST_NOT)?;
        if prohibited_count == -1 {
            Ok(-1)
        } else if prohibited_count == 0 {
            Ok(positive_count)
        } else if prohibited_count == num_docs {
            Ok(0)
        } else if positive_count == num_docs {
            Ok(num_docs - prohibited_count)
        } else {
            Ok(-1)
        }
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let mut min_should_match = self.query.get_minimum_number_should_match();

        let mut scorers = ClauseSuppliers::default();

        for weighted_clause in &self.weighted_clauses {
            let weight = &weighted_clause.weight;
            let clause = &weighted_clause.clause;
            let sub_scorer = weight.scorer_supplier(context)?;
            match sub_scorer {
                None => {
                    if clause.is_required() {
                        return Ok(None);
                    }
                }
                Some(sub_scorer) => scorers.get_mut(clause.occur()).push(sub_scorer),
            }
        }

        // scorer simplifications:

        if scorers.should.len() as i32 == min_should_match {
            // any optional clauses are in fact required
            let should = std::mem::take(&mut scorers.should);
            scorers.must.extend(should);
            min_should_match = 0;
        }

        if scorers.filter.is_empty() && scorers.must.is_empty() && scorers.should.is_empty() {
            // no required and optional clauses.
            return Ok(None);
        } else if (scorers.should.len() as i32) < min_should_match {
            // either >1 req scorer, or there are 0 req scorers and at least 1
            // optional scorer. Therefore if there are not enough optional
            // scorers no documents will be matched by the query
            return Ok(None);
        }

        if !self.score_mode.needs_scores()
            && min_should_match == 0
            && scorers.must.len() + scorers.filter.len() > 0
        {
            // Purely optional clauses are useless without scoring.
            scorers.should.clear();
        }

        Ok(Some(Box::new(BooleanScorerSupplier::new(
            scorers,
            self.score_mode,
            min_should_match,
            context.leaf_reader().max_doc(),
        )?)))
    }
}
