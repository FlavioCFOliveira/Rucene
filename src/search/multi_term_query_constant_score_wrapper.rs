//! The wrapper behind `CONSTANT_SCORE_REWRITE`, ported from
//! `org.apache.lucene.search.MultiTermQueryConstantScoreWrapper`.

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
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::Matches;
use crate::search::multi_term_query::MultiTermQuery;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::term_query::TermQuery;
use crate::search::term_states::TermStates;
use crate::search::weight::Weight;
use crate::util::{Accountable, DocIdSetBuilder};

/// Provides the functionality behind
/// [`constant_score_rewrite`](crate::search::constant_score_rewrite).
///
/// Equivalent to the `final class
/// org.apache.lucene.search.MultiTermQueryConstantScoreWrapper`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility. It tries to rewrite per segment as a boolean query that returns
/// a constant score, and otherwise fills a bit set with the matches and builds
/// a scorer on top of that bit set.
///
/// **Divergence from Lucene 10.5.0.** Java's class is generic in the wrapped
/// query's type, `MultiTermQueryConstantScoreWrapper<Q extends
/// MultiTermQuery>`, which is erased at run time and never observed; this port
/// holds the erased `Arc<dyn MultiTermQuery>` directly.
#[derive(Debug, Clone)]
pub struct MultiTermQueryConstantScoreWrapper {
    query: Arc<dyn MultiTermQuery>,
}

impl MultiTermQueryConstantScoreWrapper {
    /// Wraps a multi-term query.
    ///
    /// Equivalent to `MultiTermQueryConstantScoreWrapper(Q)`.
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

impl Accountable for MultiTermQueryConstantScoreWrapper {
    fn ram_bytes_used(&self) -> i64 {
        self.base().ram_bytes_used()
    }
}

impl Query for MultiTermQueryConstantScoreWrapper {
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
            Arc::new(BitSetRewrite),
        );
        Ok(Arc::new(ConstantScoreWeight::new(
            self.query.to_query_arc(),
            boost,
            RewritingWeightImpl { weight },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other
            .as_any()
            .downcast_ref::<MultiTermQueryConstantScoreWrapper>()
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
/// `MultiTermQueryConstantScoreWrapper.createWeight` returns.
#[derive(Debug)]
struct RewritingWeightImpl {
    weight: RewritingWeight,
}

impl ConstantScoreWeightImpl for RewritingWeightImpl {
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

/// Fills a [`DocIdSetBuilder`] with the postings of every remaining term.
///
/// Equivalent to the `rewriteInner` override of
/// `MultiTermQueryConstantScoreWrapper`.
#[derive(Debug, Default, Clone, Copy)]
struct BitSetRewrite;

impl RewriteInner for BitSetRewrite {
    fn rewrite_inner(
        &self,
        state: &RewritingState,
        context: &Arc<LeafReaderContext>,
        field_doc_count: i32,
        terms: &dyn Terms,
        terms_enum: &mut dyn TermsEnum,
        collected_terms: &[TermAndState],
        _lead_cost: i64,
    ) -> Result<WeightOrDocIdSetIterator> {
        let max_doc = context.leaf_reader().max_doc();
        let mut builder = DocIdSetBuilder::from_terms(max_doc, terms);
        let mut docs: Option<Box<dyn PostingsEnum>> = None;

        // Handle the already-collected terms.
        if !collected_terms.is_empty() {
            let mut terms_enum2 = terms.iterator()?;
            for t in collected_terms {
                terms_enum2.seek_term_state(&t.term, &*t.state)?;
                let mut postings = terms_enum2.postings(docs.take(), POSTINGS_ENUM_NONE)?;
                builder.add(&mut *postings)?;
                docs = Some(postings);
            }
        }

        // Then keep filling the bit set with the remaining terms.
        loop {
            let mut postings = terms_enum.postings(docs.take(), POSTINGS_ENUM_NONE)?;
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
            builder.add(&mut *postings)?;
            docs = Some(postings);
            if terms_enum.next()?.is_none() {
                break;
            }
        }

        Ok(WeightOrDocIdSetIterator::Iterator(
            builder.build()?.iterator()?,
        ))
    }
}
