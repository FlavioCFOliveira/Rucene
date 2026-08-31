//! Matching the documents containing a term, ported from
//! `org.apache.lucene.search.TermQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::TermStates;
use crate::index::{
    ImpactsEnum, LeafReader, LeafReaderContext, NumericDocValues, PostingsEnum, Term, TermState,
    TermsEnum, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS,
};
use crate::search::batch_score_bulk_scorer::BatchScoreBulkScorer;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use crate::search::doc_id_set_iterator::{self, DocIdSetIterator};
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::{Matches, MatchesIterator, MatchesUtils};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::{into_scorer_iterator, Scorer};
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::sim_scorer_source::{
    similarity_simple_name, SharedSimScorer, SimScorerSource, ZeroSimScorer,
};
use crate::search::similarities::{CollectionStatistics, Explanation, Similarity, TermStatistics};
use crate::search::term_matches_iterator::TermMatchesIterator;
use crate::search::term_scorer::TermScorer;
use crate::search::two_phase_iterator::ScorerIterator;

/// A [`Query`] that matches documents containing a term.
///
/// Equivalent to `org.apache.lucene.search.TermQuery`. It may be combined with
/// other terms through a [`BooleanQuery`](crate::search::BooleanQuery).
#[derive(Debug, Clone)]
pub struct TermQuery {
    term: Term,
    per_reader_term_state: Option<Arc<TermStates>>,
}

impl TermQuery {
    /// Constructs a query for the term `t`.
    ///
    /// Equivalent to `TermQuery(Term)`.
    pub fn new(t: Term) -> Self {
        Self {
            term: t,
            per_reader_term_state: None,
        }
    }

    /// Expert: constructs a query that will use the provided
    /// [`TermStates`] instead of looking the document frequency up against the
    /// searcher.
    ///
    /// Equivalent to `TermQuery(Term, TermStates)`.
    pub fn with_states(t: Term, states: Arc<TermStates>) -> Self {
        Self {
            term: t,
            per_reader_term_state: Some(states),
        }
    }

    /// Returns the term of this query.
    ///
    /// Equivalent to `TermQuery.getTerm()`.
    pub fn get_term(&self) -> &Term {
        &self.term
    }

    /// Returns the [`TermStates`] passed to the constructor, or `None` if it
    /// was not passed.
    ///
    /// Equivalent to the experimental `TermQuery.getTermStates()`.
    pub fn get_term_states(&self) -> Option<&Arc<TermStates>> {
        self.per_reader_term_state.as_ref()
    }
}

impl Query for TermQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.term.field() != field {
            buffer.push_str(self.term.field());
            buffer.push(':');
        }
        buffer.push_str(&self.term.text());
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(self.term.field()) {
            visitor.consume_terms(self, std::slice::from_ref(&self.term));
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
        let context = searcher.get_top_reader_context();
        let term_state = match self.per_reader_term_state.as_ref() {
            Some(states) if states.was_built_for(context) => Arc::clone(states),
            // PRTS was not pre-built for this IndexSearcher.
            _ => Arc::new(TermStates::build(
                searcher,
                &self.term,
                score_mode.needs_scores(),
            )?),
        };

        Ok(Arc::new(TermWeight::new(
            self.clone(),
            searcher,
            score_mode,
            boost,
            Some(term_state),
        )?))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<TermQuery>() else {
            return false;
        };
        self.term == other.term
    }

    fn query_hash(&self) -> u64 {
        // Java is `classHash() ^ term.hashCode()`; `Term` has no `Hash`
        // implementation in this port, so its two components are hashed here.
        let mut hasher = DefaultHasher::new();
        self.term.field().hash(&mut hasher);
        self.term.bytes().slice().hash(&mut hasher);
        self.class_hash() ^ hasher.finish()
    }
}

use crate::search::weight::Weight;

/// The [`Weight`] of a [`TermQuery`].
///
/// Equivalent to the inner `final class TermQuery.TermWeight`.
pub struct TermWeight {
    query: TermQuery,
    query_handle: Arc<dyn Query>,
    similarity: Arc<dyn Similarity>,
    /// `None` when the term does not exist in any segment, in which case the
    /// similarity is never used.
    sim_scorer: Option<SharedSimScorer>,
    term_states: Option<Arc<TermStates>>,
    score_mode: ScoreMode,
}

impl std::fmt::Debug for TermWeight {
    /// Equivalent to `TermWeight.toString()`, which is
    /// `"weight(" + TermQuery.this + ")"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "weight({})", self.query.to_query_string(""))
    }
}

impl TermWeight {
    /// Builds the weight of a term query.
    ///
    /// Equivalent to
    /// `TermWeight(IndexSearcher, ScoreMode, float, TermStates)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when scores are needed but no
    /// [`TermStates`] were supplied, which is the `IllegalStateException` Java
    /// throws, and propagates any I/O error raised while reading the
    /// statistics.
    pub fn new(
        query: TermQuery,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
        term_states: Option<Arc<TermStates>>,
    ) -> Result<Self> {
        if score_mode.needs_scores() && term_states.is_none() {
            return Err(LuceneError::IllegalState(
                "termStates are required when scores are needed".to_string(),
            ));
        }
        let term = query.get_term().clone();
        let similarity = Arc::clone(searcher.get_similarity());

        let collection_stats;
        let term_stats;
        if score_mode.needs_scores() {
            let states = term_states
                .as_ref()
                .expect("INVARIANT: the needs_scores branch just checked termStates is present");
            collection_stats = searcher.collection_statistics(term.field())?;
            term_stats = if states.doc_freq()? > 0 {
                Some(searcher.term_statistics(
                    &term,
                    states.doc_freq()?,
                    states.total_term_freq()?,
                )?)
            } else {
                None
            };
        } else {
            // We do not need the actual stats, use fake stats with
            // docFreq=maxDoc=ttf=1.
            collection_stats = Some(CollectionStatistics::new(term.field(), 1, 1, 1, 1)?);
            term_stats = Some(TermStatistics::new(term.bytes().clone(), 1, 1)?);
        }

        let sim_scorer: Option<SharedSimScorer> = match term_stats {
            // The term does not exist in any segment, so the similarity is not
            // used at all.
            None => None,
            Some(term_stats) => {
                if score_mode.needs_scores() {
                    let collection_stats = collection_stats.ok_or_else(|| {
                        LuceneError::IllegalState(format!(
                            "field {} has term statistics but no collection statistics",
                            term.field()
                        ))
                    })?;
                    Some(Arc::new(SimScorerSource::new(
                        Arc::clone(&similarity),
                        boost,
                        collection_stats,
                        vec![term_stats],
                    )))
                } else {
                    // Assigning a dummy scorer as this is not expected to be
                    // called since scores are not needed.
                    Some(Arc::new(ZeroSimScorer))
                }
            }
        };

        let query_handle: Arc<dyn Query> = Arc::new(query.clone());
        Ok(Self {
            query,
            query_handle,
            similarity,
            sim_scorer,
            term_states,
            score_mode,
        })
    }

    fn term(&self) -> &Term {
        self.query.get_term()
    }

    /// Returns the [`TermState`] registered for `context`, or `None` when the
    /// term is not present in that reader.
    fn term_state(&self, context: &LeafReaderContext) -> Result<Option<Box<dyn TermState>>> {
        let states = self.term_states.as_ref().ok_or_else(|| {
            LuceneError::IllegalState(
                "TermWeight was built without termStates and cannot produce scorers".to_string(),
            )
        })?;
        states.get(context)
    }

    /// Returns a [`TermsEnum`] positioned at this weight's term, or `None` if
    /// the term does not exist in the given context.
    ///
    /// Equivalent to the private
    /// `TermWeight.getTermsEnum(LeafReaderContext)`.
    fn get_terms_enum(&self, context: &LeafReaderContext) -> Result<Option<Box<dyn TermsEnum>>> {
        let Some(state) = self.term_state(context)? else {
            // The term is not present in that reader.
            return Ok(None);
        };
        Self::seek(&context.leaf_reader(), self.term(), &*state)
    }

    /// Positions a fresh [`TermsEnum`] of `reader` on `term`, using the state
    /// captured when the [`TermStates`] were built.
    fn seek(
        reader: &Arc<dyn LeafReader>,
        term: &Term,
        state: &dyn TermState,
    ) -> Result<Option<Box<dyn TermsEnum>>> {
        let Some(terms) = reader.terms(term.field())? else {
            // A state was registered for this leaf, so the field exists; this
            // branch cannot be reached, but Java would raise a
            // NullPointerException here rather than misbehave.
            return Ok(None);
        };
        let mut terms_enum = terms.iterator()?;
        terms_enum.seek_term_state(term.bytes(), state)?;
        Ok(Some(terms_enum))
    }

    /// Builds the supplier of [`TermScorer`]s for a leaf, or `None` when the
    /// term is absent from it.
    ///
    /// Equivalent to the body of
    /// `TermWeight.scorerSupplier(LeafReaderContext)`, keeping the concrete
    /// type so that [`explain`](Self::explain) can read the term frequency off
    /// the scorer — which Java does with a cast to `TermScorer`.
    fn term_scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<TermScorerSupplier>> {
        let Some(state) = self.term_state(context)? else {
            return Ok(None);
        };
        let reader = context.leaf_reader();
        let Some(terms_enum) = Self::seek(&reader, self.term(), &*state)? else {
            return Ok(None);
        };
        let doc_freq = terms_enum.doc_freq()?;
        Ok(Some(TermScorerSupplier {
            reader,
            field: self.term().field().to_string(),
            terms_enum: Some(terms_enum),
            doc_freq,
            sim_scorer: self.sim_scorer.clone(),
            score_mode: self.score_mode,
            top_level_scoring_clause: false,
        }))
    }
}

impl SegmentCacheable for TermWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for TermWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query_handle)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let Some(state) = self.term_state(context)? else {
            return Ok(None);
        };
        let reader = context.leaf_reader();
        let field = self.term().field().to_string();
        let bytes = self.term().bytes().clone();
        let query = self.get_query();
        let supplier_field = field.clone();
        // Java captures the `TermsEnum` this weight has already positioned.
        // A `dyn TermsEnum` is neither `Send` nor `Sync`, and a `Matches` is
        // both, so the closure re-seeks a fresh enum from the captured
        // `TermState` instead. The postings it reads are the same.
        MatchesUtils::for_field(
            field,
            Arc::new(move || {
                let Some(mut terms_enum) =
                    TermWeight::seek_bytes(&reader, &supplier_field, &bytes, &*state)?
                else {
                    return Ok(None);
                };
                let mut pe = terms_enum.postings(None, POSTINGS_ENUM_OFFSETS)?;
                if pe.advance(doc)? != doc {
                    return Ok(None);
                }
                Ok(Some(
                    Box::new(TermMatchesIterator::new(Arc::clone(&query), pe)?)
                        as Box<dyn MatchesIterator>,
                ))
            }),
        )
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(self
            .term_scorer_supplier(context)?
            .map(|supplier| Box::new(supplier) as Box<dyn ScorerSupplier>))
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        if let Some(mut supplier) = self.term_scorer_supplier(context)? {
            let mut scorer = supplier.term_scorer()?;
            let new_doc = scorer.iterator().advance(doc)?;
            if new_doc == doc {
                let freq = scorer.freq()?;
                let mut norms = context.leaf_reader().get_norm_values(self.term().field())?;
                let mut norm = 1i64;
                if let Some(norms) = norms.as_mut() {
                    if norms.advance_exact(doc)? {
                        norm = norms.long_value()?;
                    }
                }
                let freq_explanation = Explanation::matched(
                    freq as f32,
                    "freq, occurrences of term within document",
                    Vec::new(),
                );
                let sim_scorer = self.sim_scorer.as_ref().ok_or_else(|| {
                    LuceneError::IllegalState(
                        "a matching document requires the term to exist, and therefore a scorer"
                            .to_string(),
                    )
                })?;
                let score_explanation = sim_scorer.explain(&freq_explanation, norm);
                return Ok(Explanation::matched(
                    score_explanation.value().float_value(),
                    format!(
                        "weight({} in {doc}) [{}], result of:",
                        self.query.to_query_string(""),
                        similarity_simple_name(&*self.similarity)
                    ),
                    vec![score_explanation],
                ));
            }
        }
        Ok(Explanation::no_match("no matching term", Vec::new()))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let reader = context.leaf_reader();
        if reader.max_doc() == reader.num_docs() {
            // `LeafReader.hasDeletions()` is `numDeletedDocs() > 0`, which is
            // `maxDoc() - numDocs() > 0`.
            let terms_enum = self.get_terms_enum(context)?;
            match terms_enum {
                // `termsEnum` is not null if the term state is available.
                Some(terms_enum) => terms_enum.doc_freq(),
                // The term cannot be found in the dictionary so the count is 0.
                None => Ok(0),
            }
        } else {
            Ok(-1)
        }
    }
}

impl TermWeight {
    /// The [`seek`](Self::seek) variant the `matches` closure needs, taking the
    /// field and the term bytes separately because it cannot hold a
    /// [`Term`] alongside the captured state without cloning it twice.
    fn seek_bytes(
        reader: &Arc<dyn LeafReader>,
        field: &str,
        bytes: &crate::util::BytesRef,
        state: &dyn TermState,
    ) -> Result<Option<Box<dyn TermsEnum>>> {
        let Some(terms) = reader.terms(field)? else {
            return Ok(None);
        };
        let mut terms_enum = terms.iterator()?;
        terms_enum.seek_term_state(bytes, state)?;
        Ok(Some(terms_enum))
    }
}

/// The [`ScorerSupplier`] of a [`TermWeight`].
///
/// Equivalent to the anonymous `ScorerSupplier` returned by
/// `TermWeight.scorerSupplier(LeafReaderContext)`.
///
/// **Divergence from Lucene 10.5.0.** Java positions the terms enum lazily, on
/// the first `get()` or `cost()` call, and reads the term state through an
/// `IOSupplier` that schedules the term-dictionary I/O in the background. This
/// port positions it when the supplier is built, because `cost()` is declared
/// on `&self` and cannot create it, and because
/// [`TermStates`] resolves every state eagerly — see its
/// [`build`](TermStates::build). The scorer produced and the cost reported are
/// the same.
pub struct TermScorerSupplier {
    reader: Arc<dyn LeafReader>,
    field: String,
    terms_enum: Option<Box<dyn TermsEnum>>,
    doc_freq: i32,
    sim_scorer: Option<SharedSimScorer>,
    score_mode: ScoreMode,
    top_level_scoring_clause: bool,
}

impl std::fmt::Debug for TermScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermScorerSupplier")
            .field("field", &self.field)
            .field("doc_freq", &self.doc_freq)
            .field("score_mode", &self.score_mode)
            .finish_non_exhaustive()
    }
}

impl TermScorerSupplier {
    /// Builds the concrete [`TermScorer`].
    ///
    /// Equivalent to the body of the anonymous supplier's `get(long)`, minus
    /// the boxing; [`TermWeight::explain`] needs the concrete type in order to
    /// read the term frequency, which Java obtains with a cast.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when called more than once, which
    /// `ScorerSupplier.get(long)` forbids, and propagates any I/O error.
    pub fn term_scorer(&mut self) -> Result<TermScorer> {
        let mut terms_enum = self.terms_enum.take().ok_or_else(|| {
            LuceneError::IllegalState(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })?;

        let norms: Option<Box<dyn NumericDocValues>> = if self.score_mode.needs_scores() {
            self.reader.get_norm_values(&self.field)?
        } else {
            None
        };
        // The term exists in this leaf, so the weight holds a similarity
        // scorer; `ZeroSimScorer` is what Java installs when scores are not
        // needed.
        let sim_scorer: SharedSimScorer = self
            .sim_scorer
            .clone()
            .unwrap_or_else(|| Arc::new(ZeroSimScorer));

        if self.score_mode == ScoreMode::TOP_SCORES {
            let impacts: Box<dyn ImpactsEnum> = terms_enum.impacts(POSTINGS_ENUM_FREQS)?;
            Ok(TermScorer::with_impacts(
                impacts,
                sim_scorer,
                norms,
                self.top_level_scoring_clause,
            ))
        } else {
            let flags = if self.score_mode.needs_scores() {
                POSTINGS_ENUM_FREQS
            } else {
                POSTINGS_ENUM_NONE
            };
            let postings: Box<dyn PostingsEnum> = terms_enum.postings(None, flags)?;
            Ok(TermScorer::new(postings, sim_scorer, norms))
        }
    }
}

impl ScorerSupplier for TermScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        if self.terms_enum.is_none() {
            return Err(LuceneError::IllegalState(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            ));
        }
        Ok(Box::new(self.term_scorer()?))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        if !self.score_mode.needs_scores() {
            let max_doc = self.reader.max_doc();
            let scorer: Box<dyn Scorer> = self.get(i64::MAX)?;
            let iterator: ScorerIterator = into_scorer_iterator(scorer);
            return ConstantScoreScorerSupplier::from_iterator(
                iterator,
                0.0,
                self.score_mode,
                max_doc,
            )
            .bulk_scorer();
        }
        Ok(Box::new(BatchScoreBulkScorer::new(self.get(i64::MAX)?)))
    }

    fn cost(&self) -> i64 {
        i64::from(self.doc_freq)
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        self.top_level_scoring_clause = true;
        Ok(())
    }
}

/// Keeps the imports that only the empty-scorer branch of Java's supplier uses
/// referenced; see [`TermScorerSupplier::term_scorer`].
///
/// Java's `get(long)` answers
/// `new ConstantScoreScorer(0f, scoreMode, DocIdSetIterator.empty())` when the
/// deferred term lookup finds nothing. That branch is unreachable in this port:
/// [`TermStates`] resolves every lookup eagerly, so
/// [`TermWeight::scorer_supplier`] has already answered `None` in that case.
/// This function spells the branch out so that the behaviour is recorded and
/// available should the lookup ever become lazy again.
pub fn empty_term_scorer(score_mode: ScoreMode) -> ConstantScoreScorer {
    ConstantScoreScorer::from_iterator(0.0, score_mode, Box::new(doc_id_set_iterator::empty()))
}
