//! The machinery shared by the two constant-score multi-term wrappers, ported
//! from `org.apache.lucene.search.AbstractMultiTermQueryConstantScoreWrapper`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{IndexReaderContext, LeafReaderContext, Term, TermState, Terms, TermsEnum};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::bulk_scorer::{BulkScorer, DefaultBulkScorer};
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use crate::search::doc_id_set_iterator::{self, DocIdSetIterator};
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::{Matches, MatchesUtils};
use crate::search::multi_term_query::{get_terms_enum, MultiTermQuery};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::term_query::TermQuery;
use crate::search::term_states::TermStates;
use crate::search::two_phase_iterator::ScorerIterator;
use crate::util::{BytesRef, RamUsageEstimator};

/// A multi-term query that matches this many terms or fewer is executed as a
/// regular disjunction.
///
/// Equivalent to the package-private
/// `AbstractMultiTermQueryConstantScoreWrapper.BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD`.
pub const BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD: usize = 16;

/// A term collected from a segment, with everything needed to build a
/// [`TermQuery`] out of it.
///
/// Equivalent to the `protected static final
/// AbstractMultiTermQueryConstantScoreWrapper.TermAndState`.
#[derive(Debug)]
pub struct TermAndState {
    /// The term bytes.
    pub term: BytesRef,
    /// The state that repositions a terms enum on the term.
    pub state: Box<dyn TermState>,
    /// The term's document frequency in the segment.
    pub doc_freq: i32,
    /// The term's total term frequency in the segment.
    pub total_term_freq: i64,
}

/// Either a [`Weight`](crate::search::Weight) or a [`DocIdSetIterator`].
///
/// Equivalent to the `protected static final
/// AbstractMultiTermQueryConstantScoreWrapper.WeightOrDocIdSetIterator`, whose
/// two constructors set exactly one of its two fields; an enum states the same
/// thing without the null checks.
pub enum WeightOrDocIdSetIterator {
    /// The query rewrote to a weight.
    Weight(Arc<dyn crate::search::weight::Weight>),
    /// The query rewrote to an iterator over its matching documents.
    Iterator(Box<dyn DocIdSetIterator>),
}

impl Debug for WeightOrDocIdSetIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Weight(_) => f.write_str("WeightOrDocIdSetIterator::Weight"),
            Self::Iterator(_) => f.write_str("WeightOrDocIdSetIterator::Iterator"),
        }
    }
}

/// The state a [`RewritingWeight`] and the scorer suppliers it produces share.
///
/// **Divergence from Lucene 10.5.0.** Java's `RewritingWeight` keeps this state
/// in fields and its anonymous `ScorerSupplier` reads it through the enclosing
/// instance. A `ScorerSupplier` is `'static` in this port and cannot borrow the
/// weight, so the state lives behind an [`Arc`] that both hold. It carries a
/// clone of the [`IndexSearcher`] for the same reason — see
/// [`IndexSearcher`]'s `Clone` implementation.
#[derive(Debug)]
pub struct RewritingState {
    /// The wrapped multi-term query.
    pub query: Arc<dyn MultiTermQuery>,
    /// The score mode the weight was built for.
    pub score_mode: ScoreMode,
    /// The searcher the weight was built for.
    pub searcher: IndexSearcher,
    /// The constant score, which is the boost.
    pub score: f32,
    /// The per-wrapper rewriting strategy.
    pub inner: Arc<dyn RewriteInner>,
}

/// Rewrites the terms a [`RewritingWeight`] could not fold into a boolean
/// query.
///
/// Equivalent to the `protected abstract
/// AbstractMultiTermQueryConstantScoreWrapper.RewritingWeight.rewriteInner`.
/// Before it is called, the weight attempts to collect the found terms up to a
/// threshold; if fewer terms than the threshold are found, the query is simply
/// rewritten into a boolean query and this is not called. At the point it is
/// invoked, `terms_enum` is positioned on the next uncollected term, and the
/// terms already collected are in `collected_terms`.
pub trait RewriteInner: Send + Sync + Debug {
    /// Rewrites the query as either a weight or a document iterator.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading postings.
    #[allow(clippy::too_many_arguments)]
    fn rewrite_inner(
        &self,
        state: &RewritingState,
        context: &Arc<LeafReaderContext>,
        field_doc_count: i32,
        terms: &dyn Terms,
        terms_enum: &mut dyn TermsEnum,
        collected_terms: &[TermAndState],
        lead_cost: i64,
    ) -> Result<WeightOrDocIdSetIterator>;
}

/// The constant-score weight both wrappers build.
///
/// Equivalent to the `protected abstract static
/// AbstractMultiTermQueryConstantScoreWrapper.RewritingWeight`, which extends
/// `ConstantScoreWeight`. Compose it with
/// [`ConstantScoreWeight`](crate::search::ConstantScoreWeight).
#[derive(Debug)]
pub struct RewritingWeight {
    state: Arc<RewritingState>,
}

impl RewritingWeight {
    /// Creates the weight.
    ///
    /// Equivalent to
    /// `RewritingWeight(MultiTermQuery, float, ScoreMode, IndexSearcher)`; the
    /// `super(q, boost)` call is made by the caller, which wraps this in a
    /// [`ConstantScoreWeight`](crate::search::ConstantScoreWeight) over
    /// `q`.
    pub fn new(
        query: Arc<dyn MultiTermQuery>,
        boost: f32,
        score_mode: ScoreMode,
        searcher: &IndexSearcher,
        inner: Arc<dyn RewriteInner>,
    ) -> Self {
        Self {
            state: Arc::new(RewritingState {
                query,
                score_mode,
                searcher: searcher.clone(),
                score: boost,
                inner,
            }),
        }
    }

    /// Returns the shared state.
    pub fn state(&self) -> &Arc<RewritingState> {
        &self.state
    }
}

/// Rewrites the collected terms into a constant-score boolean query and builds
/// its weight.
///
/// Equivalent to the private
/// `RewritingWeight.rewriteAsBooleanQuery(LeafReaderContext, List<TermAndState>)`.
///
/// # Errors
///
/// Propagates any error raised while rewriting or building the weight.
pub fn rewrite_as_boolean_query(
    state: &RewritingState,
    context: &Arc<LeafReaderContext>,
    collected_terms: &[TermAndState],
) -> Result<WeightOrDocIdSetIterator> {
    let mut bq = BooleanQueryBuilder::new();
    for t in collected_terms {
        let mut term_states = TermStates::new(state.searcher.get_top_reader_context())?;
        term_states.register(
            t.state.clone_box(),
            context.ord() as usize,
            t.doc_freq,
            t.total_term_freq,
        );
        bq.add(
            Arc::new(TermQuery::with_states(
                Term::new(state.query.get_field(), t.term.clone()),
                Arc::new(term_states),
            )),
            Occur::SHOULD,
        )?;
    }
    let q: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(Arc::new(bq.build())));
    let rewritten = state.searcher.rewrite(q)?;
    // Java calls `Query.createWeight` directly, not `IndexSearcher.createWeight`,
    // so the query cache is deliberately bypassed here.
    let weight = rewritten.create_weight(&state.searcher, state.score_mode, state.score)?;
    Ok(WeightOrDocIdSetIterator::Weight(weight))
}

/// Collects up to the rewrite threshold of terms, returning `true` when the
/// enumeration was exhausted.
///
/// Equivalent to the private
/// `RewritingWeight.collectTerms(int, TermsEnum, List<TermAndState>)`.
///
/// # Errors
///
/// Propagates any I/O error raised while enumerating terms.
pub fn collect_terms(
    field_doc_count: i32,
    terms_enum: &mut dyn TermsEnum,
    terms: &mut Vec<TermAndState>,
) -> Result<bool> {
    let threshold = BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD
        .min(IndexSearcher::get_max_clause_count().max(0) as usize);
    for _ in 0..threshold {
        let Some(term) = terms_enum.next()? else {
            return Ok(true);
        };
        let state = terms_enum.term_state()?;
        let doc_freq = terms_enum.doc_freq()?;
        let term_and_state = TermAndState {
            term: BytesRef::deep_copy_of(&term),
            state,
            doc_freq,
            total_term_freq: terms_enum.total_term_freq()?,
        };
        if field_doc_count == doc_freq {
            // If the term contains every document with a value for the field,
            // all other terms can be ignored.
            terms.clear();
            terms.push(term_and_state);
            return Ok(true);
        }
        terms.push(term_and_state);
    }
    Ok(terms_enum.next()?.is_none())
}

/// Estimates the cost of a multi-term query on a segment.
///
/// Equivalent to the private static
/// `RewritingWeight.estimateCost(Terms, long)`. The reasoning, from
/// LUCENE-10207, is:
///
/// 1. when the number of query terms is unknown, assume that every term could
///    be in the query and estimate the work as the total number of docs across
///    all terms;
/// 2. when it is known, assume every query term matches at least one document,
///    and add the total number of docs beyond the first one for each term,
///    which bounds the extra docs that could match.
///
/// # Errors
///
/// Propagates any I/O error raised while reading the terms statistics.
pub fn estimate_cost(terms: &dyn Terms, query_terms_count: i64) -> Result<i64> {
    if query_terms_count == -1 {
        return Ok(terms.sum_doc_freq());
    }
    let mut potential_extra_cost = terms.sum_doc_freq();
    let indexed_term_count = terms.size();
    if indexed_term_count != -1 {
        potential_extra_cost -= indexed_term_count;
    }
    Ok(query_terms_count + potential_extra_cost)
}

/// The scorer supplier a [`RewritingWeight`] produces.
///
/// Equivalent to the anonymous `ScorerSupplier` returned by
/// `RewritingWeight.scorerSupplier(LeafReaderContext)`, together with the
/// `IOLongFunction<WeightOrDocIdSetIterator>` it captures.
pub struct RewritingScorerSupplier {
    state: Arc<RewritingState>,
    context: Arc<LeafReaderContext>,
    field_doc_count: i32,
    terms: Arc<dyn Terms>,
    terms_enum: Box<dyn TermsEnum>,
    collected_terms: Vec<TermAndState>,
    /// `Some` when the terms were collected while building this supplier, and
    /// `None` when collection is deferred to [`ScorerSupplier::get`].
    collect_result: Option<bool>,
    cost: i64,
}

impl Debug for RewritingScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RewritingScorerSupplier")
            .field("field_doc_count", &self.field_doc_count)
            .field("collect_result", &self.collect_result)
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl RewritingScorerSupplier {
    /// Equivalent to applying the captured
    /// `IOLongFunction<WeightOrDocIdSetIterator>`.
    fn weight_or_iterator(&mut self, lead_cost: i64) -> Result<WeightOrDocIdSetIterator> {
        let collect_result = match self.collect_result {
            Some(collect_result) => collect_result,
            None => collect_terms(
                self.field_doc_count,
                &mut *self.terms_enum,
                &mut self.collected_terms,
            )?,
        };
        if collect_result {
            rewrite_as_boolean_query(&self.state, &self.context, &self.collected_terms)
        } else {
            // Too many terms to rewrite as a simple boolean query; invoke the
            // per-wrapper rewriting logic.
            let inner = Arc::clone(&self.state.inner);
            inner.rewrite_inner(
                &self.state,
                &self.context,
                self.field_doc_count,
                &*self.terms,
                &mut *self.terms_enum,
                &self.collected_terms,
                lead_cost,
            )
        }
    }

    fn empty_scorer(&self) -> Result<ConstantScoreScorer> {
        Ok(ConstantScoreScorer::from_iterator(
            self.state.score,
            self.state.score_mode,
            Box::new(doc_id_set_iterator::empty()),
        ))
    }
}

impl ScorerSupplier for RewritingScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let weight_or_iterator = self.weight_or_iterator(lead_cost)?;
        let scorer: Option<Box<dyn Scorer>> = match weight_or_iterator {
            WeightOrDocIdSetIterator::Weight(weight) => weight.scorer(&self.context)?,
            WeightOrDocIdSetIterator::Iterator(iterator) => {
                Some(Box::new(ConstantScoreScorer::from_iterator(
                    self.state.score,
                    self.state.score_mode,
                    iterator,
                )))
            }
        };
        // It is against the API contract to return a null scorer from a
        // non-null ScorerSupplier, so an empty scorer stands in when the
        // supplier thought there might be hits and there turn out to be none.
        match scorer {
            Some(scorer) => Ok(scorer),
            None => Ok(Box::new(self.empty_scorer()?)),
        }
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        let weight_or_iterator = self.weight_or_iterator(i64::MAX)?;
        let bulk_scorer: Option<Box<dyn BulkScorer>> = match weight_or_iterator {
            WeightOrDocIdSetIterator::Weight(weight) => weight.bulk_scorer(&self.context)?,
            WeightOrDocIdSetIterator::Iterator(iterator) => {
                let max_doc = self.context.leaf_reader().max_doc();
                Some(
                    ConstantScoreScorerSupplier::from_iterator(
                        ScorerIterator::Simple(iterator),
                        self.state.score,
                        self.state.score_mode,
                        max_doc,
                    )
                    .bulk_scorer()?,
                )
            }
        };
        match bulk_scorer {
            Some(bulk_scorer) => Ok(bulk_scorer),
            None => Ok(Box::new(DefaultBulkScorer::new(Box::new(
                self.empty_scorer()?,
            )))),
        }
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// Builds the scorer supplier of a [`RewritingWeight`].
///
/// Equivalent to `RewritingWeight.scorerSupplier(LeafReaderContext)`.
///
/// # Errors
///
/// Propagates any I/O error raised while reading the terms dictionary.
pub fn rewriting_scorer_supplier(
    state: &Arc<RewritingState>,
    context: &LeafReaderContext,
) -> Result<Option<Box<dyn ScorerSupplier>>> {
    let Some(terms) = context.leaf_reader().terms(state.query.get_field())? else {
        return Ok(None);
    };
    let terms: Arc<dyn Terms> = Arc::from(terms);

    let field_doc_count = terms.doc_count();
    let mut terms_enum = get_terms_enum(&*state.query, &terms)?;
    let context = owned_context(state, context)?;

    let mut collected_terms = Vec::new();
    let collect_result;
    let cost;
    // Only collect terms while building the scorer supplier when the query
    // exposes a known, bounded term count — `TermInSetQuery`, whose
    // `getTermsCount()` is `>= 0`. There, collecting is cheap and lets a null
    // supplier be returned up front so that a parent boolean query can
    // short-circuit.
    //
    // For queries with an unknown term count — automaton queries: wildcard,
    // regexp, prefix, range — collecting eagerly can scan the whole term
    // dictionary while the supplier is built: a leading wildcard such as
    // `*foo*` cannot seek and must visit every term. That is supposed to be the
    // cheap planning phase, and doing it there defeats a parent conjunction's
    // ability to short-circuit. So for an unknown term count the cost is
    // estimated and term collection is deferred to `get`.
    if state.query.get_terms_count() >= 0 {
        let collected = collect_terms(field_doc_count, &mut *terms_enum, &mut collected_terms)?;
        if collected {
            // Return no supplier if no query term was in the segment.
            if collected_terms.is_empty() {
                return Ok(None);
            }
            let mut sum_term_cost = 0i64;
            for collected_term in &collected_terms {
                sum_term_cost += i64::from(collected_term.doc_freq);
            }
            cost = sum_term_cost;
        } else {
            cost = estimate_cost(&*terms, state.query.get_terms_count())?;
        }
        collect_result = Some(collected);
    } else {
        cost = estimate_cost(&*terms, state.query.get_terms_count())?;
        collect_result = None;
    }

    Ok(Some(Box::new(RewritingScorerSupplier {
        state: Arc::clone(state),
        context,
        field_doc_count,
        terms,
        terms_enum,
        collected_terms,
        collect_result,
        cost,
    })))
}

/// Recovers the shared [`Arc`] of a leaf context from the searcher.
///
/// **Divergence from Lucene 10.5.0.** Java's weight captures the
/// `LeafReaderContext` it was handed. This port's
/// [`Weight::scorer_supplier`](crate::search::Weight::scorer_supplier) receives
/// a borrow and the supplier it returns is `'static`, so the owning handle is
/// looked up by leaf ordinal in the searcher this weight was built for. It is
/// the same object.
fn owned_context(
    state: &RewritingState,
    context: &LeafReaderContext,
) -> Result<Arc<LeafReaderContext>> {
    let leaves = state.searcher.get_leaf_contexts();
    let ord = context.ord();
    let candidate = usize::try_from(ord)
        .ok()
        .and_then(|ord| leaves.get(ord))
        .filter(|leaf| leaf.id() == context.id());
    match candidate {
        Some(leaf) => Ok(Arc::clone(leaf)),
        None => Err(LuceneError::IllegalState(format!(
            "leaf context with ord {ord} does not belong to the searcher this weight was built for"
        ))),
    }
}

/// Returns the [`Matches`] of the wrapped multi-term query for a document.
///
/// Equivalent to `RewritingWeight.matches(LeafReaderContext, int)`.
///
/// # Errors
///
/// Propagates any I/O error raised while reading postings.
pub fn rewriting_matches(
    state: &Arc<RewritingState>,
    context: &LeafReaderContext,
    doc: i32,
) -> Result<Option<Arc<dyn Matches>>> {
    if context
        .leaf_reader()
        .terms(state.query.get_field())?
        .is_none()
    {
        return Ok(None);
    }
    let context = owned_context(state, context)?;
    let field = state.query.get_field().to_string();
    let query = Arc::clone(&state.query);
    let supplier_field = field.clone();
    MatchesUtils::for_field(
        field,
        Arc::new(move || {
            let Some(terms) = context.leaf_reader().terms(&supplier_field)? else {
                return Ok(None);
            };
            let terms: Arc<dyn Terms> = Arc::from(terms);
            let terms_enum = get_terms_enum(&*query, &terms)?;
            crate::search::disjunction_matches_iterator::from_terms_enum(
                &context,
                doc,
                query.to_query_arc(),
                &supplier_field,
                Box::new(
                    crate::search::disjunction_matches_iterator::TermsEnumBytesRefIterator::new(
                        terms_enum,
                    ),
                ),
            )
        }),
    )
}

/// The functionality common to
/// [`MultiTermQueryConstantScoreWrapper`](crate::search::MultiTermQueryConstantScoreWrapper)
/// and
/// [`MultiTermQueryConstantScoreBlendedWrapper`](crate::search::MultiTermQueryConstantScoreBlendedWrapper).
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.search.AbstractMultiTermQueryConstantScoreWrapper<Q>`,
/// which implements [`Accountable`](crate::util::Accountable). It is an
/// internal implementation detail, not an extension point for users; it is
/// public here because Rust has no package visibility.
///
/// **Divergence from Lucene 10.5.0.** Java's class *is* a `Query`, and the two
/// wrappers extend it and add only `createWeight`. Rust has no implementation
/// inheritance, so each wrapper holds one of these and forwards the six members
/// it defines. Java's type parameter over the wrapped query's class is erased
/// at run time and never observed, so the erased `Arc<dyn MultiTermQuery>` is
/// held directly.
#[derive(Debug, Clone)]
pub struct AbstractMultiTermQueryConstantScoreWrapper {
    query: Arc<dyn MultiTermQuery>,
}

impl AbstractMultiTermQueryConstantScoreWrapper {
    /// Wraps a multi-term query.
    ///
    /// Equivalent to the `protected
    /// AbstractMultiTermQueryConstantScoreWrapper(Q)` constructor.
    pub fn new(query: Arc<dyn MultiTermQuery>) -> Self {
        Self { query }
    }

    /// Returns the encapsulated query.
    ///
    /// Equivalent to
    /// `AbstractMultiTermQueryConstantScoreWrapper.getQuery()`.
    pub fn get_query(&self) -> &Arc<dyn MultiTermQuery> {
        &self.query
    }

    /// Returns the field name for this query.
    ///
    /// Equivalent to
    /// `AbstractMultiTermQueryConstantScoreWrapper.getField()`.
    pub fn get_field(&self) -> &str {
        self.query.get_field()
    }

    /// Renders the query.
    ///
    /// Equivalent to
    /// `AbstractMultiTermQueryConstantScoreWrapper.toString(String)`, which is
    /// the wrapped query's own rendering — correct for the filter too, since
    /// the boost is `1`.
    pub fn to_query_string(&self, field: &str) -> String {
        self.query.as_query().to_query_string(field)
    }

    /// Recurses into the wrapped query.
    ///
    /// Equivalent to
    /// `AbstractMultiTermQueryConstantScoreWrapper.visit(QueryVisitor)`;
    /// `wrapper` is the outer query, which Java passes as `this`.
    pub fn visit(&self, wrapper: &dyn Query, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(self.query.get_field()) {
            let mut sub = visitor.get_sub_visitor(Occur::FILTER, wrapper);
            self.query.as_query().visit(&mut *sub);
        }
    }

    /// Compares the wrapped queries.
    ///
    /// Equivalent to
    /// `AbstractMultiTermQueryConstantScoreWrapper.equals(Object)` beyond its
    /// `sameClassAs` check, which the caller has already made.
    pub fn query_eq(&self, other: &Self) -> bool {
        self.query.as_query().query_eq(other.query.as_query())
    }

    /// The hash code of the query.
    ///
    /// Equivalent to
    /// `AbstractMultiTermQueryConstantScoreWrapper.hashCode()`; `wrapper` is
    /// the outer query, whose class hash Java's `classHash()` answers.
    pub fn query_hash(&self, wrapper: &dyn Query) -> u64 {
        31u64
            .wrapping_mul(wrapper.class_hash())
            .wrapping_add(self.query.as_query().query_hash())
    }

    /// The estimated heap usage of the query.
    ///
    /// Equivalent to
    /// `AbstractMultiTermQueryConstantScoreWrapper.ramBytesUsed()`.
    pub fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + self
                .query
                .accountable_ram_bytes_used()
                .unwrap_or(RamUsageEstimator::QUERY_DEFAULT_RAM_BYTES_USED)
    }
}
