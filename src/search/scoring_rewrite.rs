//! Rewriting a multi-term query into one query per term, ported from
//! `org.apache.lucene.search.ScoringRewrite`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use crate::error::Result;
use crate::index::TermStates;
use crate::index::{IndexReaderContext, LeafReaderContext, Term, TermsEnum};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::boost_attribute::{add_boost_attribute, boost_of};
use crate::search::boost_query::BoostQuery;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::index_searcher::{IndexSearcher, TooManyClauses};
use crate::search::multi_term_query::{rewrite_method_type_hash, MultiTermQuery, RewriteMethod};
use crate::search::query::Query;
use crate::search::term_collecting_rewrite::{
    collect_terms, TermCollectingRewrite, TermCollector, TopLevelBuilder,
};
use crate::search::term_query::TermQuery;
use crate::util::attribute::AttributeSource;
use crate::util::BytesRef;

/// The base rewrite method that translates each term into a query and keeps the
/// scores as computed by that query.
///
/// Equivalent to the abstract class `org.apache.lucene.search.ScoringRewrite`,
/// which is only public in Java so that the spans package can reach it. The two
/// abstract members Java declares beyond
/// [`TermCollectingRewrite`](crate::search::term_collecting_rewrite) are here;
/// the `final rewrite` is the free function [`scoring_rewrite`].
pub trait ScoringRewrite: TermCollectingRewrite {
    /// Checks, after every new term, that the maximum number of clauses is not
    /// exceeded.
    ///
    /// Equivalent to the `protected abstract
    /// ScoringRewrite.checkMaxClauseCount(int)`.
    ///
    /// # Errors
    ///
    /// Returns the error that corresponds to the `RuntimeException` Java
    /// throws — [`TooManyClauses`] for the boolean rewrite.
    fn check_max_clause_count(&self, count: usize) -> Result<()>;
}

/// The terms collected for one expanded term of a multi-term query.
///
/// Equivalent to one entry of the parallel arrays
/// `ScoringRewrite.TermFreqBoostByteStart` keeps beside its `BytesRefHash`.
struct CollectedTerm {
    bytes: BytesRef,
    boost: f32,
    term_state: TermStates,
}

/// The collector `ScoringRewrite.rewrite` feeds.
///
/// Equivalent to the inner `ScoringRewrite.ParallelArraysTermCollector`.
///
/// **Divergence from Lucene 10.5.0.** Java deduplicates the terms with a
/// `BytesRefHash` over a `ByteBlockPool` and keeps the boost and the
/// [`TermStates`] in parallel arrays indexed by the hash ordinal, then sorts
/// the ordinals. This port keeps the same parallel structure with a
/// [`HashMap`] from the term bytes to the entry index; the set of collected
/// terms, their boosts, their states and the order they are added to the
/// builder — ascending by term bytes — are identical.
struct ParallelArraysTermCollector<'a, R: ScoringRewrite + ?Sized> {
    attributes: AttributeSource,
    rewrite: &'a R,
    terms: Vec<CollectedTerm>,
    index: HashMap<Vec<u8>, usize>,
    top_reader_context: Option<Arc<dyn IndexReaderContext>>,
    reader_ord: usize,
}

impl<R: ScoringRewrite + ?Sized> TermCollector for ParallelArraysTermCollector<'_, R> {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn set_reader_context(
        &mut self,
        top_reader_context: &Arc<dyn IndexReaderContext>,
        reader_context: &Arc<LeafReaderContext>,
    ) -> Result<()> {
        self.top_reader_context = Some(Arc::clone(top_reader_context));
        self.reader_ord = reader_context.ord() as usize;
        Ok(())
    }

    fn set_next_enum(&mut self, terms_enum: &mut dyn TermsEnum) -> Result<()> {
        // Java stores the enum and adds a `BoostAttribute` to it; the enum is
        // passed to `collect` here, so only the attribute is installed.
        add_boost_attribute(terms_enum.attributes());
        Ok(())
    }

    fn collect(&mut self, bytes: &BytesRef, terms_enum: &mut dyn TermsEnum) -> Result<bool> {
        let boost = boost_of(terms_enum.attributes());
        let state = terms_enum.term_state()?;
        let doc_freq = terms_enum.doc_freq()?;
        let total_term_freq = terms_enum.total_term_freq()?;
        let key = bytes.slice().to_vec();
        match self.index.get(&key) {
            Some(&pos) => {
                // Duplicate term: update the doc freq.
                self.terms[pos].term_state.register(
                    state,
                    self.reader_ord,
                    doc_freq,
                    total_term_freq,
                );
                debug_assert_eq!(
                    self.terms[pos].boost, boost,
                    "boost should be equal in all segment TermsEnums"
                );
            }
            None => {
                // New entry: populate it initially.
                let top = self
                    .top_reader_context
                    .as_ref()
                    .expect("INVARIANT: set_reader_context runs before collect");
                let term_state =
                    TermStates::with_state(top, state, self.reader_ord, doc_freq, total_term_freq)?;
                self.terms.push(CollectedTerm {
                    bytes: BytesRef::deep_copy_of(bytes),
                    boost,
                    term_state,
                });
                self.index.insert(key, self.terms.len() - 1);
                self.rewrite.check_max_clause_count(self.terms.len())?;
            }
        }
        Ok(true)
    }
}

/// Rewrites a multi-term query by collecting every expanded term and adding one
/// clause per term to the rewrite's builder.
///
/// Equivalent to the `final ScoringRewrite.rewrite(IndexSearcher,
/// MultiTermQuery)`.
///
/// # Errors
///
/// Propagates any I/O error raised while enumerating terms, and the error
/// [`ScoringRewrite::check_max_clause_count`] reports.
pub fn scoring_rewrite<R>(
    rewrite: &R,
    index_searcher: &IndexSearcher,
    query: &dyn MultiTermQuery,
) -> Result<Arc<dyn Query>>
where
    R: ScoringRewrite + RewriteMethod,
{
    let mut builder = rewrite.get_top_level_builder()?;
    let mut col = ParallelArraysTermCollector {
        attributes: AttributeSource::new(),
        rewrite,
        terms: Vec::new(),
        index: HashMap::new(),
        top_reader_context: None,
        reader_ord: 0,
    };
    collect_terms(index_searcher, rewrite, query, &mut col)?;

    // Java sorts the hash ordinals and walks `termStates[pos]`; sorting the
    // entries themselves by their bytes produces exactly the same order.
    let mut terms = col.terms;
    terms.sort_by(|a, b| a.bytes.cmp(&b.bytes));
    for collected in terms {
        let term = Term::new(query.get_field(), collected.bytes);
        let doc_freq = collected.term_state.doc_freq()?;
        builder.add_clause(
            term,
            doc_freq,
            collected.boost,
            Some(Arc::new(collected.term_state)),
        )?;
    }
    builder.build()
}

/// The top-level builder of [`ScoringBooleanRewrite`].
///
/// Equivalent to the `BooleanQuery.Builder` the anonymous
/// `ScoringRewrite.SCORING_BOOLEAN_REWRITE` uses, together with its
/// `addClause` and `build` overrides.
#[derive(Debug, Default)]
pub struct ScoringBooleanQueryBuilder {
    builder: BooleanQueryBuilder,
}

impl ScoringBooleanQueryBuilder {
    /// Creates an empty builder.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TopLevelBuilder for ScoringBooleanQueryBuilder {
    fn add_clause(
        &mut self,
        term: Term,
        _doc_count: i32,
        boost: f32,
        states: Option<Arc<TermStates>>,
    ) -> Result<()> {
        let tq: Arc<dyn Query> = match states {
            Some(states) => Arc::new(TermQuery::with_states(term, states)),
            // Java's four-argument `addClause` passes `null`, and
            // `TermQuery(Term, TermStates)` requires a non-null value, so the
            // one-argument constructor is the faithful counterpart.
            None => Arc::new(TermQuery::new(term)),
        };
        self.builder
            .add(Arc::new(BoostQuery::new(tq, boost)?), Occur::SHOULD)?;
        Ok(())
    }

    fn build(self: Box<Self>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(self.builder.build()))
    }
}

/// The rewrite method that translates each term into an
/// [`Occur::SHOULD`] clause of a boolean query, keeping the scores as computed
/// by that query.
///
/// Equivalent to the anonymous class behind
/// `ScoringRewrite.SCORING_BOOLEAN_REWRITE`, which is also
/// `MultiTermQuery.SCORING_BOOLEAN_REWRITE`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScoringBooleanRewrite;

impl TermCollectingRewrite for ScoringBooleanRewrite {
    fn get_top_level_builder(&self) -> Result<Box<dyn TopLevelBuilder>> {
        Ok(Box::new(ScoringBooleanQueryBuilder::new()))
    }
}

impl ScoringRewrite for ScoringBooleanRewrite {
    fn check_max_clause_count(&self, count: usize) -> Result<()> {
        if count > IndexSearcher::get_max_clause_count() as usize {
            return Err(TooManyClauses::new().into());
        }
        Ok(())
    }
}

impl RewriteMethod for ScoringBooleanRewrite {
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

    fn method_hash(&self) -> u64 {
        rewrite_method_type_hash(self)
    }
}

/// Like [`ScoringBooleanRewrite`] except that scores are not computed: each
/// matching document receives a constant score equal to the query's boost.
///
/// Equivalent to the anonymous class behind
/// `ScoringRewrite.CONSTANT_SCORE_BOOLEAN_REWRITE`, which is also
/// `MultiTermQuery.CONSTANT_SCORE_BOOLEAN_REWRITE`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConstantScoreBooleanRewrite;

impl RewriteMethod for ConstantScoreBooleanRewrite {
    fn rewrite(
        &self,
        index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        let bq = ScoringBooleanRewrite.rewrite(index_searcher, query)?;
        // Strip the scores off.
        Ok(Arc::new(ConstantScoreQuery::new(bq)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_hash(&self) -> u64 {
        rewrite_method_type_hash(self)
    }
}
