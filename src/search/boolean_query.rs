//! Boolean combinations of queries, ported from
//! `org.apache.lucene.search.BooleanQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::TermStates;
use crate::search::boolean_clause::{BooleanClause, Occur};
use crate::search::boolean_weight::BooleanWeight;
use crate::search::boost_query::BoostQuery;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::index_searcher::{IndexSearcher, TooManyClauses};
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::multiset::Multiset;
use crate::search::query::{Query, QueryKey};
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::term_query::TermQuery;
use crate::search::weight::Weight;

/// The four operators, in the order `java.util.EnumMap` iterates them: the
/// declaration order of `BooleanClause.Occur`.
const OCCUR_ORDER: [Occur; 4] = [Occur::MUST, Occur::FILTER, Occur::SHOULD, Occur::MUST_NOT];

/// The clauses of a [`BooleanQuery`], grouped by operator.
///
/// Equivalent to the private `Map<Occur, Collection<Query>> clauseSets` field,
/// an `EnumMap` whose `SHOULD` and `MUST` entries are `Multiset`s — duplicates
/// matter — and whose `FILTER` and `MUST_NOT` entries are `HashSet`s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ClauseSets {
    must: Multiset<QueryKey>,
    filter: HashSet<QueryKey>,
    should: Multiset<QueryKey>,
    must_not: HashSet<QueryKey>,
}

impl ClauseSets {
    fn add(&mut self, clause: &BooleanClause) {
        let key = QueryKey::new(Arc::clone(clause.query()));
        match clause.occur() {
            Occur::MUST => self.must.add(key),
            Occur::FILTER => {
                self.filter.insert(key);
            }
            Occur::SHOULD => self.should.add(key),
            Occur::MUST_NOT => {
                self.must_not.insert(key);
            }
        }
    }

    fn len(&self, occur: Occur) -> usize {
        match occur {
            Occur::MUST => self.must.len(),
            Occur::FILTER => self.filter.len(),
            Occur::SHOULD => self.should.len(),
            Occur::MUST_NOT => self.must_not.len(),
        }
    }

    fn queries(&self, occur: Occur) -> Vec<Arc<dyn Query>> {
        match occur {
            Occur::MUST => self
                .must
                .iter()
                .map(|key| Arc::clone(key.query()))
                .collect(),
            Occur::FILTER => self
                .filter
                .iter()
                .map(|key| Arc::clone(key.query()))
                .collect(),
            Occur::SHOULD => self
                .should
                .iter()
                .map(|key| Arc::clone(key.query()))
                .collect(),
            Occur::MUST_NOT => self
                .must_not
                .iter()
                .map(|key| Arc::clone(key.query()))
                .collect(),
        }
    }

    fn contains(&self, occur: Occur, key: &QueryKey) -> bool {
        match occur {
            Occur::MUST => self.must.contains(key),
            Occur::FILTER => self.filter.contains(key),
            Occur::SHOULD => self.should.contains(key),
            Occur::MUST_NOT => self.must_not.contains(key),
        }
    }
}

/// Hashes a set of queries independently of its iteration order.
///
/// Equivalent to `java.util.AbstractSet.hashCode()`, which sums the element
/// hashes; [`Multiset`] already hashes that way.
fn hash_query_set(set: &HashSet<QueryKey>) -> u64 {
    let mut aggregate: u64 = 0;
    for key in set {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        aggregate = aggregate.wrapping_add(hasher.finish());
    }
    aggregate
}

/// A builder for boolean queries.
///
/// Equivalent to the static `BooleanQuery.Builder`.
///
/// **Divergence from Lucene 10.5.0.** Java's `add` returns `this` so that calls
/// chain; here it can fail, so it returns
/// `Result<&mut Self>` and a chain is written with `?`.
#[derive(Debug, Default, Clone)]
pub struct Builder {
    minimum_number_should_match: i32,
    clauses: Vec<BooleanClause>,
}

impl Builder {
    /// Creates an empty builder.
    ///
    /// Equivalent to the sole `BooleanQuery.Builder()` constructor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Specifies a minimum number of the optional clauses which must be
    /// satisfied.
    ///
    /// Equivalent to `Builder.setMinimumNumberShouldMatch(int)`. By default no
    /// optional clause is necessary for a match, unless there are no required
    /// clauses. This is entirely independent of specifying that any particular
    /// clause is required or prohibited: the number is only compared against the
    /// number of matching optional clauses.
    pub fn set_minimum_number_should_match(&mut self, min: i32) -> &mut Self {
        self.minimum_number_should_match = min;
        self
    }

    /// Adds a new clause.
    ///
    /// Equivalent to `Builder.add(BooleanClause)`. The order in which clauses
    /// are added has no impact on the matching documents or on query
    /// performance.
    ///
    /// # Errors
    ///
    /// Returns the [`TooManyClauses`] error when the new number of clauses
    /// exceeds [`IndexSearcher::get_max_clause_count`]. The final deep check
    /// happens in [`IndexSearcher::rewrite`]; this one short-circuits a single
    /// query holding more than that many clauses, which is not merely an
    /// optimisation: it prevents a runaway rewrite of a bad query from building
    /// boolean queries that eat up all the heap.
    pub fn add_clause(&mut self, clause: BooleanClause) -> Result<&mut Self> {
        if self.clauses.len() >= IndexSearcher::get_max_clause_count() as usize {
            return Err(TooManyClauses::new().into());
        }
        self.clauses.push(clause);
        Ok(self)
    }

    /// Adds a new clause built from a query and an operator.
    ///
    /// Equivalent to `Builder.add(Query, Occur)`.
    ///
    /// # Errors
    ///
    /// As [`add_clause`](Self::add_clause).
    pub fn add(&mut self, query: Arc<dyn Query>, occur: Occur) -> Result<&mut Self> {
        self.add_clause(BooleanClause::new(query, occur))
    }

    /// Adds a collection of clauses.
    ///
    /// Equivalent to `Builder.add(Collection<BooleanClause>)`.
    ///
    /// # Errors
    ///
    /// As [`add_clause`](Self::add_clause).
    pub fn add_all(&mut self, clauses: Vec<BooleanClause>) -> Result<&mut Self> {
        if self.clauses.len() + clauses.len() > IndexSearcher::get_max_clause_count() as usize {
            return Err(TooManyClauses::new().into());
        }
        self.clauses.extend(clauses);
        Ok(self)
    }

    /// Creates a new [`BooleanQuery`] from the parameters set on this builder.
    ///
    /// Equivalent to `Builder.build()`.
    pub fn build(&self) -> BooleanQuery {
        BooleanQuery::new(self.minimum_number_should_match, self.clauses.clone())
    }
}

/// A query that matches documents matching boolean combinations of other
/// queries.
///
/// Equivalent to `org.apache.lucene.search.BooleanQuery`, which also implements
/// `Iterable<BooleanClause>`; [`clauses`](Self::clauses) is the Rust way to
/// iterate them.
///
#[derive(Debug, Clone)]
pub struct BooleanQuery {
    minimum_number_should_match: i32,
    /// The clauses, in the order they were added; used by
    /// [`to_query_string`](Query::to_query_string) and
    /// [`clauses`](Self::clauses).
    clauses: Vec<BooleanClause>,
    /// The clauses grouped by operator; used by equality and hashing.
    ///
    /// WARNING: do not let this escape, as it would break immutability.
    clause_sets: ClauseSets,
}

impl BooleanQuery {
    /// Builds a boolean query from its clauses.
    ///
    /// Equivalent to the private
    /// `BooleanQuery(int, BooleanClause[])` constructor, which
    /// [`Builder::build`] calls. It is public here because Rust has no
    /// package-private visibility and because a builder is the only way to
    /// reach it in Java anyway.
    pub fn new(minimum_number_should_match: i32, clauses: Vec<BooleanClause>) -> Self {
        let mut clause_sets = ClauseSets::default();
        for clause in &clauses {
            clause_sets.add(clause);
        }
        Self {
            minimum_number_should_match,
            clauses,
            clause_sets,
        }
    }

    /// Returns the minimum number of the optional clauses which must be
    /// satisfied.
    ///
    /// Equivalent to `BooleanQuery.getMinimumNumberShouldMatch()`.
    pub fn get_minimum_number_should_match(&self) -> i32 {
        self.minimum_number_should_match
    }

    /// Returns the clauses of this query, in the order they were added.
    ///
    /// Equivalent to `BooleanQuery.clauses()`, and to the `Iterable` the class
    /// implements.
    pub fn clauses(&self) -> &[BooleanClause] {
        &self.clauses
    }

    /// Returns the queries of the clauses with the given operator.
    ///
    /// Equivalent to `BooleanQuery.getClauses(Occur)`. As in Java, duplicates
    /// are preserved for [`Occur::MUST`] and [`Occur::SHOULD`] and removed for
    /// [`Occur::FILTER`] and [`Occur::MUST_NOT`], and the order is unspecified.
    pub fn get_clauses(&self, occur: Occur) -> Vec<Arc<dyn Query>> {
        self.clause_sets.queries(occur)
    }

    /// Returns the number of clauses with the given operator, counting
    /// duplicates for [`Occur::MUST`] and [`Occur::SHOULD`] only.
    ///
    /// Equivalent to `getClauses(occur).size()`, which Lucene writes throughout
    /// `rewrite`.
    pub fn clause_count(&self, occur: Occur) -> usize {
        self.clause_sets.len(occur)
    }

    /// Returns whether this query is a pure disjunction: it only has
    /// [`Occur::SHOULD`] clauses, and a single one matching is enough.
    ///
    /// Equivalent to the package-private `BooleanQuery.isPureDisjunction()`.
    pub fn is_pure_disjunction(&self) -> bool {
        self.clauses.len() == self.clause_count(Occur::SHOULD)
            && self.minimum_number_should_match <= 1
    }

    /// Returns whether this query is a two-clause disjunction whose clauses are
    /// both [`TermQuery`]s.
    ///
    /// Equivalent to the package-private
    /// `BooleanQuery.isTwoClausePureDisjunctionWithTerms()`.
    pub fn is_two_clause_pure_disjunction_with_terms(&self) -> bool {
        self.clauses.len() == 2
            && self.is_pure_disjunction()
            && self.clauses[0].query().as_any().is::<TermQuery>()
            && self.clauses[1].query().as_any().is::<TermQuery>()
    }

    /// Rewrites a two-clause disjunction of term queries into the two term
    /// queries and their conjunction, so that
    /// [`IndexSearcher::count`](crate::search::IndexSearcher::count) can apply
    /// the inclusion–exclusion principle.
    ///
    /// Equivalent to the package-private
    /// `BooleanQuery.rewriteTwoClauseDisjunctionWithTermsForCount(IndexSearcher)`,
    /// which returns a `Query[3]`: the two term queries followed by their
    /// conjunction.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when a clause is not a
    /// [`TermQuery`] — the caller must check
    /// [`is_two_clause_pure_disjunction_with_terms`](Self::is_two_clause_pure_disjunction_with_terms)
    /// first, which is what Java's cast asserts — and propagates any I/O error
    /// raised while building the term states.
    pub fn rewrite_two_clause_disjunction_with_terms_for_count(
        &self,
        index_searcher: &IndexSearcher,
    ) -> Result<[Arc<dyn Query>; 3]> {
        let mut new_query = Builder::new();
        let mut queries: Vec<Arc<dyn Query>> = Vec::with_capacity(2);
        for clause in &self.clauses {
            let term_query = clause
                .query()
                .as_any()
                .downcast_ref::<TermQuery>()
                .ok_or_else(|| {
                    LuceneError::IllegalState(
                        "rewriteTwoClauseDisjunctionWithTermsForCount requires TermQuery clauses"
                            .to_string(),
                    )
                })?;
            // The optimisation counts each term query several times, so a
            // cached `TermStates` avoids repeating the terms-dictionary lookup.
            let term_query: Arc<dyn Query> = if term_query.get_term_states().is_none() {
                Arc::new(TermQuery::with_states(
                    term_query.get_term().clone(),
                    Arc::new(TermStates::build(
                        index_searcher,
                        term_query.get_term(),
                        false,
                    )?),
                ))
            } else {
                Arc::clone(clause.query())
            };
            new_query.add(Arc::clone(&term_query), Occur::MUST)?;
            queries.push(term_query);
        }
        let conjunction: Arc<dyn Query> = Arc::new(new_query.build());
        let mut queries = queries.into_iter();
        Ok([
            queries
                .next()
                .expect("INVARIANT: a two-clause query has two clauses"),
            queries
                .next()
                .expect("INVARIANT: a two-clause query has two clauses"),
            conjunction,
        ])
    }

    /// Rewrites this query for the case where scores are not needed, or returns
    /// `None` when it rewrites to itself.
    ///
    /// Equivalent to the package-private `BooleanQuery.rewriteNoScoring()`,
    /// called from [`ConstantScoreQuery`]'s rewrite. Java returns `this` when
    /// nothing changed; this port returns `None`, exactly as
    /// [`Query::rewrite`] does and for the same reason.
    ///
    /// NOTE: this must not call [`Query::rewrite`], or the method could run in
    /// time exponential in the depth of the query, as every new level would
    /// rewrite twice as much as its parent level.
    pub fn rewrite_no_scoring(&self) -> Option<BooleanQuery> {
        let mut actually_rewritten = false;
        let mut new_query = Builder::new();
        new_query.set_minimum_number_should_match(self.get_minimum_number_should_match());

        let keep_should = self.get_minimum_number_should_match() > 0
            || (self.clause_count(Occur::MUST) + self.clause_count(Occur::FILTER) == 0);

        for clause in &self.clauses {
            let query = clause.query();
            let mut rewritten = Arc::clone(query);
            if let Some(boost) = rewritten.as_any().downcast_ref::<BoostQuery>() {
                rewritten = boost.get_query();
            }
            if let Some(constant) = rewritten.as_any().downcast_ref::<ConstantScoreQuery>() {
                rewritten = constant.get_query();
            }
            let inner_rewritten = rewritten
                .as_any()
                .downcast_ref::<BooleanQuery>()
                .and_then(BooleanQuery::rewrite_no_scoring);
            if let Some(inner_rewritten) = inner_rewritten {
                rewritten = Arc::new(inner_rewritten);
            }

            let occur = clause.occur();
            let result = if occur == Occur::SHOULD && !keep_should {
                // ignore clause
                actually_rewritten = true;
                Ok(&mut new_query)
            } else if occur == Occur::MUST {
                // replace MUST clauses with FILTER clauses
                actually_rewritten = true;
                new_query.add(rewritten, Occur::FILTER)
            } else if !Arc::ptr_eq(query, &rewritten) {
                actually_rewritten = true;
                new_query.add(rewritten, occur)
            } else {
                new_query.add_clause(clause.clone())
            };
            // `Builder.add` can only fail by exceeding the clause-count limit,
            // which this query already satisfies because it holds at least as
            // many clauses as the rewritten one.
            debug_assert!(result.is_ok());
            if result.is_err() {
                return None;
            }
        }

        if !actually_rewritten {
            return None;
        }

        Some(new_query.build())
    }

    /// Equivalent to the private `BooleanQuery.computeHashCode()`.
    fn compute_hash_code(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.minimum_number_should_match.hash(&mut hasher);
        self.clause_sets.must.hash(&mut hasher);
        hasher.write_u64(hash_query_set(&self.clause_sets.filter));
        self.clause_sets.should.hash(&mut hasher);
        hasher.write_u64(hash_query_set(&self.clause_sets.must_not));
        let hash = hasher.finish();
        if hash == 0 {
            1
        } else {
            hash
        }
    }
}

/// Strips the boost wrappers off a query, returning the base query and the
/// product of the boosts.
///
/// Equivalent to the `while (query instanceof BoostQuery)` loops of the two
/// boost-deduplication steps of `BooleanQuery.rewrite`.
fn unwrap_boosts(query: &Arc<dyn Query>) -> (Arc<dyn Query>, f64) {
    let mut boost = 1.0f64;
    let mut query = Arc::clone(query);
    while let Some(boost_query) = query.as_any().downcast_ref::<BoostQuery>() {
        boost *= f64::from(boost_query.get_boost());
        let inner = boost_query.get_query();
        query = inner;
    }
    (query, boost)
}

/// Groups the given queries by their base query, summing the boosts, keeping
/// the order in which they were first seen.
///
/// Equivalent to the `Map<Query, Double>` the two boost-deduplication steps of
/// `BooleanQuery.rewrite` build. Java uses a `HashMap`, whose iteration order is
/// unspecified; keeping the first-seen order here removes a source of
/// non-determinism without changing the resulting query, whose equality ignores
/// clause order.
fn group_by_boost(queries: &[Arc<dyn Query>]) -> Vec<(Arc<dyn Query>, f64)> {
    let mut grouped: Vec<(Arc<dyn Query>, f64)> = Vec::new();
    for query in queries {
        let (base, boost) = unwrap_boosts(query);
        match grouped
            .iter_mut()
            .find(|(existing, _)| existing.query_eq(&*base))
        {
            Some((_, total)) => *total += boost,
            None => grouped.push((base, boost)),
        }
    }
    grouped
}

impl BooleanQuery {
    /// Equivalent to the "recursively rewrite" block of
    /// `BooleanQuery.rewrite`.
    fn rewrite_clauses(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let mut builder = Builder::new();
        builder.set_minimum_number_should_match(self.get_minimum_number_should_match());
        let mut actually_rewritten = false;
        for clause in &self.clauses {
            let query = clause.query();
            let occur = clause.occur();
            let rewritten: Arc<dyn Query> = if occur == Occur::FILTER || occur == Occur::MUST_NOT {
                // Clauses that are not involved in scoring can get some extra
                // simplifications.
                let constant = ConstantScoreQuery::new(Arc::clone(query));
                match constant.rewrite(searcher)? {
                    // The constant-score wrapper rewrote to itself, so
                    // unwrapping it gives the clause back unchanged.
                    None => Arc::clone(query),
                    Some(rewritten) => {
                        match rewritten.as_any().downcast_ref::<ConstantScoreQuery>() {
                            Some(constant) => constant.get_query(),
                            None => rewritten,
                        }
                    }
                }
            } else {
                query
                    .rewrite(searcher)?
                    .unwrap_or_else(|| Arc::clone(query))
            };

            if !Arc::ptr_eq(&rewritten, query) || query.as_any().is::<MatchNoDocsQuery>() {
                // rewrite clause
                actually_rewritten = true;
                if rewritten.as_any().is::<MatchNoDocsQuery>() {
                    match occur {
                        // the clause can be safely ignored
                        Occur::SHOULD | Occur::MUST_NOT => {}
                        Occur::MUST | Occur::FILTER => return Ok(Some(rewritten)),
                    }
                } else {
                    builder.add(rewritten, occur)?;
                }
            } else {
                // leave as-is
                builder.add_clause(clause.clone())?;
            }
        }
        if actually_rewritten {
            return Ok(Some(Arc::new(builder.build())));
        }
        Ok(None)
    }
}

impl Query for BooleanQuery {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(BooleanWeight::new(
            self.clone(),
            searcher,
            score_mode,
            boost,
        )?))
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.clauses.is_empty() {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new("empty BooleanQuery"))));
        }

        // Queries with no positive clauses have no matches
        if self.clauses.len() == self.clause_count(Occur::MUST_NOT) {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                "pure negative BooleanQuery",
            ))));
        }

        // optimize 1-clause queries
        if self.clauses.len() == 1 {
            let clause = &self.clauses[0];
            let query = clause.query();
            if self.minimum_number_should_match == 1 && clause.occur() == Occur::SHOULD {
                return Ok(Some(Arc::clone(query)));
            } else if self.minimum_number_should_match == 0 {
                match clause.occur() {
                    Occur::SHOULD | Occur::MUST => return Ok(Some(Arc::clone(query))),
                    Occur::FILTER => {
                        // no scoring clauses, so return a score of 0
                        return Ok(Some(Arc::new(BoostQuery::new(
                            Arc::new(ConstantScoreQuery::new(Arc::clone(query))),
                            0.0,
                        )?)));
                    }
                    // Java raises an AssertionError here: a single MUST_NOT
                    // clause was already turned into a MatchNoDocsQuery above.
                    Occur::MUST_NOT => {
                        debug_assert!(false, "a pure negative BooleanQuery was already rewritten");
                    }
                }
            }
        }

        // recursively rewrite
        if let Some(rewritten) = self.rewrite_clauses(searcher)? {
            return Ok(Some(rewritten));
        }

        // remove duplicate FILTER and MUST_NOT clauses
        {
            let clause_count: usize = OCCUR_ORDER
                .iter()
                .map(|occur| self.clause_count(*occur))
                .sum();
            if clause_count != self.clauses.len() {
                // since clause_sets implicitly deduplicates FILTER and MUST_NOT
                // clauses, this means there were duplicates
                let mut rewritten = Builder::new();
                rewritten.set_minimum_number_should_match(self.minimum_number_should_match);
                for occur in OCCUR_ORDER {
                    for query in self.clause_sets.queries(occur) {
                        rewritten.add(query, occur)?;
                    }
                }
                return Ok(Some(Arc::new(rewritten.build())));
            }
        }

        // Check whether some clauses are both required and excluded
        if self.clause_count(Occur::MUST_NOT) > 0 {
            let match_all = QueryKey::new(Arc::new(MatchAllDocsQuery));
            for key in &self.clause_sets.must_not {
                if self.clause_sets.contains(Occur::MUST, key)
                    || self.clause_sets.contains(Occur::FILTER, key)
                {
                    return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                        "FILTER or MUST clause also in MUST_NOT",
                    ))));
                }
            }
            if self.clause_sets.must_not.contains(&match_all) {
                return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                    "MUST_NOT clause is MatchAllDocsQuery",
                ))));
            }
        }

        // remove FILTER clauses that are also MUST clauses or that match all
        // documents
        if self.clause_count(Occur::FILTER) > 0 {
            let mut filters: HashSet<QueryKey> = self.clause_sets.filter.clone();
            let mut modified = false;
            if filters.len() > 1 || self.clause_count(Occur::MUST) != 0 {
                modified = filters.remove(&QueryKey::new(Arc::new(MatchAllDocsQuery)));
            }
            for key in self.clause_sets.must.iter() {
                modified |= filters.remove(key);
            }
            if modified {
                let mut builder = Builder::new();
                builder.set_minimum_number_should_match(self.get_minimum_number_should_match());
                for clause in &self.clauses {
                    if clause.occur() != Occur::FILTER {
                        builder.add_clause(clause.clone())?;
                    }
                }
                for filter in filters {
                    builder.add(filter.into_query(), Occur::FILTER)?;
                }
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // convert FILTER clauses that are also SHOULD clauses to MUST clauses
        if self.clause_count(Occur::SHOULD) > 0 && self.clause_count(Occur::FILTER) > 0 {
            let intersection: HashSet<QueryKey> = self
                .clause_sets
                .filter
                .iter()
                .filter(|key| self.clause_sets.should.contains(key))
                .cloned()
                .collect();

            if !intersection.is_empty() {
                let mut builder = Builder::new();
                let mut min_should_match = self.get_minimum_number_should_match();

                for clause in &self.clauses {
                    let key = QueryKey::new(Arc::clone(clause.query()));
                    if intersection.contains(&key) {
                        if clause.occur() == Occur::SHOULD {
                            builder.add_clause(BooleanClause::new(
                                Arc::clone(clause.query()),
                                Occur::MUST,
                            ))?;
                            min_should_match -= 1;
                        }
                    } else {
                        builder.add_clause(clause.clone())?;
                    }
                }

                builder.set_minimum_number_should_match(min_should_match.max(0));
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // Deduplicate SHOULD clauses by summing up their boosts
        if self.clause_count(Occur::SHOULD) > 0 && self.minimum_number_should_match <= 1 {
            let should_clauses = group_by_boost(&self.clause_sets.queries(Occur::SHOULD));
            if should_clauses.len() != self.clause_count(Occur::SHOULD) {
                let mut builder = Builder::new();
                builder.set_minimum_number_should_match(self.minimum_number_should_match);
                for (query, boost) in should_clauses {
                    let boost = boost as f32;
                    let query: Arc<dyn Query> = if boost != 1.0 {
                        Arc::new(BoostQuery::new(query, boost)?)
                    } else {
                        query
                    };
                    builder.add(query, Occur::SHOULD)?;
                }
                for clause in &self.clauses {
                    if clause.occur() != Occur::SHOULD {
                        builder.add_clause(clause.clone())?;
                    }
                }
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // Deduplicate MUST clauses by summing up their boosts
        if self.clause_count(Occur::MUST) > 0 {
            let must_clauses = group_by_boost(&self.clause_sets.queries(Occur::MUST));
            if must_clauses.len() != self.clause_count(Occur::MUST) {
                let mut builder = Builder::new();
                builder.set_minimum_number_should_match(self.minimum_number_should_match);
                for (query, boost) in must_clauses {
                    let boost = boost as f32;
                    let query: Arc<dyn Query> = if boost != 1.0 {
                        Arc::new(BoostQuery::new(query, boost)?)
                    } else {
                        query
                    };
                    builder.add(query, Occur::MUST)?;
                }
                for clause in &self.clauses {
                    if clause.occur() != Occur::MUST {
                        builder.add_clause(clause.clone())?;
                    }
                }
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // Rewrite queries whose single scoring clause is a MUST clause on a
        // MatchAllDocsQuery to a ConstantScoreQuery
        if self.clause_count(Occur::MUST) == 1 && self.clause_count(Occur::FILTER) > 0 {
            let musts = self.clause_sets.queries(Occur::MUST);
            let mut must = Arc::clone(&musts[0]);
            let mut boost = 1.0f32;
            if let Some(boost_query) = must.as_any().downcast_ref::<BoostQuery>() {
                boost = boost_query.get_boost();
                let inner = boost_query.get_query();
                must = inner;
            }
            if must.as_any().is::<MatchAllDocsQuery>() {
                // our single scoring clause matches everything: rewrite to a CSQ
                // on the filter, ignoring the SHOULD clauses for now
                let mut builder = Builder::new();
                for clause in &self.clauses {
                    match clause.occur() {
                        Occur::FILTER | Occur::MUST_NOT => {
                            builder.add_clause(clause.clone())?;
                        }
                        // ignore
                        Occur::MUST | Occur::SHOULD => {}
                    }
                }
                let mut rewritten: Arc<dyn Query> = Arc::new(builder.build());
                rewritten = Arc::new(ConstantScoreQuery::new(rewritten));
                if boost != 1.0 {
                    rewritten = Arc::new(BoostQuery::new(rewritten, boost)?);
                }

                // now add back the SHOULD clauses
                let mut builder = Builder::new();
                builder.set_minimum_number_should_match(self.get_minimum_number_should_match());
                builder.add(rewritten, Occur::MUST)?;
                for query in self.clause_sets.queries(Occur::SHOULD) {
                    builder.add(query, Occur::SHOULD)?;
                }
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // Flatten nested disjunctions, this is important for block-max WAND to
        // perform well
        if self.minimum_number_should_match <= 1 {
            let mut builder = Builder::new();
            builder.set_minimum_number_should_match(self.minimum_number_should_match);
            let mut actually_rewritten = false;
            for clause in &self.clauses {
                let inner = if clause.occur() == Occur::SHOULD {
                    clause.query().as_any().downcast_ref::<BooleanQuery>()
                } else {
                    None
                };
                match inner {
                    Some(inner_query) if inner_query.is_pure_disjunction() => {
                        actually_rewritten = true;
                        for inner_clause in inner_query.clauses() {
                            builder.add_clause(inner_clause.clone())?;
                        }
                    }
                    _ => {
                        builder.add_clause(clause.clone())?;
                    }
                }
            }
            if actually_rewritten {
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // Inline required / prohibited clauses. This helps run filtered
        // conjunctive queries more efficiently by providing all clauses to the
        // block-max AND scorer.
        {
            let mut builder = Builder::new();
            builder.set_minimum_number_should_match(self.minimum_number_should_match);
            let mut actually_rewritten = false;
            for outer_clause in &self.clauses {
                let inner = if outer_clause.is_required() {
                    outer_clause.query().as_any().downcast_ref::<BooleanQuery>()
                } else {
                    None
                };
                match inner {
                    Some(inner_query)
                        if inner_query.get_minimum_number_should_match() == 0
                            && inner_query.clause_count(Occur::SHOULD) == 0 =>
                    {
                        // Inlining prohibited clauses is not legal if the query
                        // is a pure negation, since pure negations have no
                        // matches. It works because the inner BooleanQuery would
                        // have first rewritten to a MatchNoDocsQuery if it only
                        // had prohibited clauses.
                        debug_assert_ne!(
                            inner_query.clause_count(Occur::MUST_NOT),
                            inner_query.clauses().len()
                        );
                        actually_rewritten = true;
                        for inner_clause in inner_query.clauses() {
                            let inner_occur = inner_clause.occur();
                            if inner_occur == Occur::FILTER
                                || inner_occur == Occur::MUST_NOT
                                || outer_clause.occur() == Occur::MUST
                            {
                                builder.add_clause(inner_clause.clone())?;
                            } else {
                                debug_assert!(
                                    outer_clause.occur() == Occur::FILTER
                                        && inner_occur == Occur::MUST
                                );
                                // In this case we need to change the occur of
                                // the inner query from MUST to FILTER.
                                builder.add(Arc::clone(inner_clause.query()), Occur::FILTER)?;
                            }
                        }
                    }
                    _ => {
                        builder.add_clause(outer_clause.clone())?;
                    }
                }
            }
            if actually_rewritten {
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // SHOULD clause count less than or equal to minimumNumberShouldMatch.
        // Important: this can only be processed after nested clauses have been
        // flattened.
        {
            let shoulds = self.clause_count(Occur::SHOULD) as i32;
            if shoulds < self.minimum_number_should_match {
                return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                    "SHOULD clause count less than minimumNumberShouldMatch",
                ))));
            }
            if shoulds > 0 && shoulds == self.minimum_number_should_match {
                let mut builder = Builder::new();
                for clause in &self.clauses {
                    if clause.occur() == Occur::SHOULD {
                        builder.add(Arc::clone(clause.query()), Occur::MUST)?;
                    } else {
                        builder.add_clause(clause.clone())?;
                    }
                }
                return Ok(Some(Arc::new(builder.build())));
            }
        }

        // Inline SHOULD clauses from the only MUST clause
        if self.clause_count(Occur::SHOULD) == 0 && self.clause_count(Occur::MUST) == 1 {
            let musts = self.clause_sets.queries(Occur::MUST);
            let inner = musts[0].as_any().downcast_ref::<BooleanQuery>();
            if let Some(inner) = inner {
                if inner.clauses.len() == inner.clause_count(Occur::SHOULD) {
                    let mut rewritten = Builder::new();
                    for clause in &self.clauses {
                        if clause.occur() != Occur::MUST {
                            rewritten.add_clause(clause.clone())?;
                        }
                    }
                    for inner_clause in inner.clauses() {
                        rewritten.add_clause(inner_clause.clone())?;
                    }
                    rewritten.set_minimum_number_should_match(
                        inner.get_minimum_number_should_match().max(1),
                    );
                    return Ok(Some(Arc::new(rewritten.build())));
                }
            }
        }

        Ok(None)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::MUST, self);
        for occur in OCCUR_ORDER {
            if self.clause_count(occur) > 0 {
                if occur == Occur::MUST {
                    for query in self.clause_sets.queries(occur) {
                        query.visit(&mut *sub);
                    }
                } else {
                    let mut nested = sub.get_sub_visitor(occur, self);
                    for query in self.clause_sets.queries(occur) {
                        query.visit(&mut *nested);
                    }
                }
            }
        }
    }

    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        let need_parens = self.get_minimum_number_should_match() > 0;
        if need_parens {
            buffer.push('(');
        }

        for (i, clause) in self.clauses.iter().enumerate() {
            buffer.push_str(&clause.occur().to_string());

            let sub_query = clause.query();
            if sub_query.as_any().is::<BooleanQuery>() {
                // wrap sub-bools in parens
                buffer.push('(');
                buffer.push_str(&sub_query.to_query_string(field));
                buffer.push(')');
            } else {
                buffer.push_str(&sub_query.to_query_string(field));
            }

            if i != self.clauses.len() - 1 {
                buffer.push(' ');
            }
        }

        if need_parens {
            buffer.push(')');
        }

        if self.get_minimum_number_should_match() > 0 {
            buffer.push('~');
            buffer.push_str(&self.get_minimum_number_should_match().to_string());
        }

        buffer
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<BooleanQuery>() else {
            return false;
        };
        self.get_minimum_number_should_match() == other.get_minimum_number_should_match()
            && self.clause_sets == other.clause_sets
    }

    fn query_hash(&self) -> u64 {
        self.compute_hash_code()
    }
}
