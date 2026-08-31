//! The wrapper behind `CONSTANT_SCORE_BLENDED_REWRITE`, ported from
//! `org.apache.lucene.search.MultiTermQueryConstantScoreBlendedWrapper`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{LeafReaderContext, PostingsEnum, Term, Terms, TermsEnum, POSTINGS_ENUM_NONE};
use crate::search::abstract_multi_term_query_constant_score_wrapper::{
    rewriting_matches, rewriting_scorer_supplier, AbstractMultiTermQueryConstantScoreWrapper,
    RewriteInner, RewritingState, RewritingWeight, TermAndState, WeightOrDocIdSetIterator,
};
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::disjunction_disi_approximation::DisjunctionDISIApproximation;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::Matches;
use crate::search::multi_term_query::MultiTermQuery;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::term_query::TermQuery;
use crate::search::term_states::TermStates;
use crate::search::weight::Weight;
use crate::util::{Accountable, DocIdSetBuilder, PriorityQueue, PriorityQueueComparator};

/// Postings lists under this threshold are always pre-processed into a bit set.
///
/// Equivalent to the private
/// `MultiTermQueryConstantScoreBlendedWrapper.POSTINGS_PRE_PROCESS_THRESHOLD`.
const POSTINGS_PRE_PROCESS_THRESHOLD: i32 = 512;

/// Provides the functionality behind
/// [`constant_score_blended_rewrite`](crate::search::constant_score_blended_rewrite).
///
/// Equivalent to the `final class
/// org.apache.lucene.search.MultiTermQueryConstantScoreBlendedWrapper`, which
/// is package-private in Java; it is public here because Rust has no package
/// visibility. It maintains a boolean-query-like approach over a limited number
/// of the most costly terms while rewriting the remaining terms into a filter
/// bit set.
///
/// **Divergence from Lucene 10.5.0.** As with
/// [`MultiTermQueryConstantScoreWrapper`](crate::search::MultiTermQueryConstantScoreWrapper),
/// Java's type parameter over the wrapped query's class is erased at run time
/// and is replaced here by the erased `Arc<dyn MultiTermQuery>`.
#[derive(Debug, Clone)]
pub struct MultiTermQueryConstantScoreBlendedWrapper {
    query: Arc<dyn MultiTermQuery>,
}

impl MultiTermQueryConstantScoreBlendedWrapper {
    /// Wraps a multi-term query.
    ///
    /// Equivalent to `MultiTermQueryConstantScoreBlendedWrapper(Q)`.
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

    /// Returns the shared half of this wrapper, which Java inherits from
    /// [`AbstractMultiTermQueryConstantScoreWrapper`].
    pub fn base(&self) -> AbstractMultiTermQueryConstantScoreWrapper {
        AbstractMultiTermQueryConstantScoreWrapper::new(Arc::clone(&self.query))
    }
}

impl Accountable for MultiTermQueryConstantScoreBlendedWrapper {
    fn ram_bytes_used(&self) -> i64 {
        self.base().ram_bytes_used()
    }
}

impl Query for MultiTermQueryConstantScoreBlendedWrapper {
    fn to_query_string(&self, field: &str) -> String {
        self.base().to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.base().visit(self, visitor);
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
        let weight = RewritingWeight::new(
            Arc::clone(&self.query),
            boost,
            score_mode,
            searcher,
            Arc::new(BlendedRewrite),
        );
        Ok(Arc::new(ConstantScoreWeight::new(
            self.query.to_query_arc(),
            boost,
            BlendedRewritingWeightImpl { weight },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other
            .as_any()
            .downcast_ref::<MultiTermQueryConstantScoreBlendedWrapper>()
        else {
            return false;
        };
        self.base().query_eq(&other.base())
    }

    fn query_hash(&self) -> u64 {
        self.base().query_hash(self)
    }
}

/// The [`ConstantScoreWeightImpl`] half of the weight this wrapper builds.
///
/// Equivalent to the anonymous `RewritingWeight` subclass
/// `MultiTermQueryConstantScoreBlendedWrapper.createWeight` returns.
#[derive(Debug)]
struct BlendedRewritingWeightImpl {
    weight: RewritingWeight,
}

impl ConstantScoreWeightImpl for BlendedRewritingWeightImpl {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        rewriting_scorer_supplier(self.weight.state(), context)
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        rewriting_matches(self.weight.state(), context, doc)
    }
}

/// Orders postings enums by cost, least costly first.
///
/// Equivalent to the anonymous `PriorityQueue.lessThan` of
/// `MultiTermQueryConstantScoreBlendedWrapper.rewriteInner`.
struct ByCost;

impl PriorityQueueComparator<Box<dyn PostingsEnum>> for ByCost {
    fn less_than(&self, a: &Box<dyn PostingsEnum>, b: &Box<dyn PostingsEnum>) -> bool {
        a.cost() < b.cost()
    }
}

/// Keeps the most costly postings lists as separate iterators and folds the
/// rest into a bit set.
///
/// Equivalent to the `rewriteInner` override of
/// `MultiTermQueryConstantScoreBlendedWrapper`.
#[derive(Debug, Default, Clone, Copy)]
struct BlendedRewrite;

impl RewriteInner for BlendedRewrite {
    fn rewrite_inner(
        &self,
        state: &RewritingState,
        context: &Arc<LeafReaderContext>,
        field_doc_count: i32,
        terms: &dyn Terms,
        terms_enum: &mut dyn TermsEnum,
        collected_terms: &[TermAndState],
        lead_cost: i64,
    ) -> Result<WeightOrDocIdSetIterator> {
        let max_doc = context.leaf_reader().max_doc();
        let mut other_terms = DocIdSetBuilder::from_terms(max_doc, terms);
        let mut high_frequency_terms: PriorityQueue<Box<dyn PostingsEnum>, ByCost> =
            PriorityQueue::new(collected_terms.len(), ByCost)?;

        // Handle the already-collected terms.
        let mut reuse: Option<Box<dyn PostingsEnum>> = None;
        if !collected_terms.is_empty() {
            let mut terms_enum2 = terms.iterator()?;
            for t in collected_terms {
                terms_enum2.seek_term_state(&t.term, &*t.state)?;
                let mut postings = terms_enum2.postings(reuse.take(), POSTINGS_ENUM_NONE)?;
                if t.doc_freq <= POSTINGS_PRE_PROCESS_THRESHOLD {
                    other_terms.add(&mut *postings)?;
                    reuse = Some(postings);
                } else {
                    high_frequency_terms.add(postings);
                    // The postings cannot be reused, because they have not been
                    // processed yet.
                    reuse = None;
                }
            }
        }

        // Then collect the remaining terms.
        loop {
            let mut postings = terms_enum.postings(reuse.take(), POSTINGS_ENUM_NONE)?;
            // If a term contains all docs with a value for the specified field,
            // the other terms can be discarded and the dense term's postings
            // used on their own.
            let doc_freq = terms_enum.doc_freq()?;
            if field_doc_count == doc_freq {
                let mut term_states = TermStates::new(state.searcher.get_top_reader_context())?;
                term_states.register(
                    terms_enum.term_state()?,
                    context.ord() as usize,
                    doc_freq,
                    terms_enum.total_term_freq()?,
                );
                let q: Arc<dyn Query> =
                    Arc::new(ConstantScoreQuery::new(Arc::new(TermQuery::with_states(
                        Term::new(state.query.get_field(), terms_enum.term()?),
                        Arc::new(term_states),
                    ))));
                let rewritten = state.searcher.rewrite(q)?;
                let weight =
                    rewritten.create_weight(&state.searcher, state.score_mode, state.score)?;
                return Ok(WeightOrDocIdSetIterator::Weight(weight));
            }

            if doc_freq <= POSTINGS_PRE_PROCESS_THRESHOLD {
                other_terms.add(&mut *postings)?;
                reuse = Some(postings);
            } else {
                let dropped = high_frequency_terms.insert_with_overflow(postings);
                match dropped {
                    Some(mut dropped) => {
                        other_terms.add(&mut *dropped)?;
                        // Reuse the postings that dropped out of the queue.
                        reuse = Some(dropped);
                    }
                    // Nothing was evicted, so no postings can be reused: the
                    // ones in the queue are still live.
                    None => reuse = None,
                }
            }

            if terms_enum.next()?.is_none() {
                break;
            }
        }

        let queued: Vec<Box<dyn PostingsEnum>> = high_frequency_terms.into_iter().collect();
        let mut subs: Vec<DisiWrapper> = Vec::with_capacity(queued.len() + 1);
        for disi in queued {
            let scorer = wrap_with_dummy_scorer(disi);
            subs.push(DisiWrapper::new(scorer, false));
        }
        let scorer = wrap_with_dummy_scorer(other_terms.build()?.iterator()?);
        subs.push(DisiWrapper::new(scorer, false));

        Ok(WeightOrDocIdSetIterator::Iterator(Box::new(
            DisjunctionDISIApproximation::new(subs, lead_cost),
        )))
    }
}

/// Wraps an iterator with a dummy scorer so that
/// [`DisiWrapper`] and [`DisjunctionDISIApproximation`] can be used as-is.
///
/// Equivalent to the private static
/// `MultiTermQueryConstantScoreBlendedWrapper.wrapWithDummyScorer(Weight, DocIdSetIterator)`.
/// It is just a convenient vehicle to get the iterator into the priority queue
/// the approximation uses; the scorer the weight ultimately provides carries the
/// constant boost and reflects the actual score mode. The score and score mode
/// do not matter here, except that
/// [`ScoreMode::TOP_SCORES`](crate::search::ScoreMode::TOP_SCORES) would create
/// another wrapper object around the iterator, which is why it is avoided.
fn wrap_with_dummy_scorer(disi: Box<dyn DocIdSetIterator>) -> Box<dyn Scorer> {
    Box::new(ConstantScoreScorer::from_iterator(
        1.0,
        ScoreMode::COMPLETE_NO_SCORES,
        disi,
    ))
}
