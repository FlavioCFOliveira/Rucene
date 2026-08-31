//! Treating several terms as synonyms, ported from
//! `org.apache.lucene.search.SynonymQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    FreqAndNormBuffer, Impacts, ImpactsEnum, ImpactsSource, LeafReaderContext, NumericDocValues,
    SlowImpactsEnum, Term, POSTINGS_ENUM_FREQS,
};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_weight::java_float_to_string;
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::disjunction_disi_approximation::DisjunctionDISIApproximation;
use crate::search::disjunction_matches_iterator::from_terms;
use crate::search::doc_id_set_iterator::{self, DocIdSetIterator};
use crate::search::impacts_disi::ImpactsDISI;
use crate::search::index_searcher::{IndexSearcher, TooManyClauses};
use crate::search::matches::{owned_leaf_context, Matches, MatchesUtils};
use crate::search::max_score_cache::MaxScoreCache;
use crate::search::phrase_matcher::{IteratorWithImpacts, SharedPostings};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::sim_scorer_source::{
    similarity_simple_name, SharedSimScorer, SharedSimScorerRef, SimScorerSource,
};
use crate::search::similarities::{Explanation, Similarity, TermStatistics};
use crate::search::term_query::TermQuery;
use crate::search::term_scorer::TermScorer;
use crate::search::term_states::TermStates;
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::search::weight::Weight;
use crate::util::BytesRef;

/// A term of a [`SynonymQuery`] and the boost applied to its document
/// frequencies.
///
/// Equivalent to the private record `SynonymQuery.TermAndBoost`.
#[derive(Debug, Clone, PartialEq)]
pub struct TermAndBoost {
    /// The term bytes.
    pub term: BytesRef,
    /// The boost applied to the term's document frequencies.
    pub boost: f32,
}

/// A builder for [`SynonymQuery`].
///
/// Equivalent to `SynonymQuery.Builder`.
#[derive(Debug, Clone)]
pub struct Builder {
    field: String,
    terms: Vec<TermAndBoost>,
}

impl Builder {
    /// Creates a builder for the given target field.
    ///
    /// Equivalent to the sole `Builder(String)` constructor.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            terms: Vec::new(),
        }
    }

    /// Adds the provided term as a synonym, with a boost of `1`.
    ///
    /// Equivalent to `Builder.addTerm(Term)`.
    ///
    /// # Errors
    ///
    /// As [`add_term_with_boost`](Self::add_term_with_boost).
    pub fn add_term(&mut self, term: &Term) -> Result<&mut Self> {
        self.add_term_with_boost(term, 1.0)
    }

    /// Adds the provided term as a synonym, boosting its document frequencies.
    ///
    /// Equivalent to `Builder.addTerm(Term, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the term is on another
    /// field, and the errors of [`add_bytes`](Self::add_bytes).
    pub fn add_term_with_boost(&mut self, term: &Term, boost: f32) -> Result<&mut Self> {
        if self.field != term.field() {
            return Err(LuceneError::IllegalArgument(
                "Synonyms must be across the same field".to_string(),
            ));
        }
        self.add_bytes(term.bytes().clone(), boost)
    }

    /// Adds the provided term bytes as a synonym, boosting its document
    /// frequencies.
    ///
    /// Equivalent to `Builder.addTerm(BytesRef, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `boost` is not in
    /// `(0, 1]`, and [`TooManyClauses`] once
    /// [`IndexSearcher::get_max_clause_count`] terms have been added.
    pub fn add_bytes(&mut self, term: BytesRef, boost: f32) -> Result<&mut Self> {
        if boost.is_nan() || boost <= 0.0 || boost > 1.0 {
            return Err(LuceneError::IllegalArgument(
                "boost must be a positive float between 0 (exclusive) and 1 (inclusive)"
                    .to_string(),
            ));
        }
        self.terms.push(TermAndBoost { term, boost });
        if self.terms.len() > IndexSearcher::get_max_clause_count() as usize {
            return Err(TooManyClauses::new().into());
        }
        Ok(self)
    }

    /// Builds the [`SynonymQuery`].
    ///
    /// Equivalent to `Builder.build()`, which sorts the terms by their bytes.
    pub fn build(&self) -> SynonymQuery {
        let mut terms = self.terms.clone();
        terms.sort_by(|a, b| a.term.cmp(&b.term));
        SynonymQuery::new(terms, self.field.clone())
    }
}

/// A query that treats several terms as synonyms.
///
/// Equivalent to the `final org.apache.lucene.search.SynonymQuery`. For scoring
/// purposes it tries to score the terms as if they had been indexed as one
/// term: it matches any of them, but invokes the similarity a single time,
/// scoring the sum of all the term frequencies of the document.
#[derive(Debug, Clone)]
pub struct SynonymQuery {
    terms: Vec<TermAndBoost>,
    field: String,
}

impl SynonymQuery {
    /// Creates a query matching any of the supplied terms, which must all be on
    /// `field`.
    ///
    /// Equivalent to the private `SynonymQuery(TermAndBoost[], String)`, which
    /// [`Builder::build`] calls; it is public here because Rust has no
    /// package-private visibility.
    pub fn new(terms: Vec<TermAndBoost>, field: impl Into<String>) -> Self {
        Self {
            terms,
            field: field.into(),
        }
    }

    /// Returns the terms of this query.
    ///
    /// Equivalent to `SynonymQuery.getTerms()`.
    pub fn get_terms(&self) -> Vec<Term> {
        self.terms
            .iter()
            .map(|t| Term::new(self.field.clone(), t.term.clone()))
            .collect()
    }

    /// Returns the field name of this query.
    ///
    /// Equivalent to `SynonymQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the terms and their boosts.
    ///
    /// Equivalent to reading the `private final TermAndBoost[] terms` field.
    pub fn terms_and_boosts(&self) -> &[TermAndBoost] {
        &self.terms
    }
}

impl Query for SynonymQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut builder = String::from("Synonym(");
        for (i, t) in self.terms.iter().enumerate() {
            if i != 0 {
                builder.push(' ');
            }
            let term_query = TermQuery::new(Term::new(self.field.clone(), t.term.clone()));
            builder.push_str(&term_query.to_query_string(field));
            if t.boost != 1.0 {
                builder.push('^');
                builder.push_str(&java_float_to_string(t.boost));
            }
        }
        builder.push(')');
        builder
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if !visitor.accept_field(&self.field) {
            return;
        }
        let mut v = visitor.get_sub_visitor(Occur::SHOULD, self);
        let ts = self.get_terms();
        v.consume_terms(self, &ts);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        // Optimise the zero-term and the non-boosted single-term cases.
        if self.terms.is_empty() {
            return Ok(Some(Arc::new(BooleanQueryBuilder::new().build())));
        }
        if self.terms.len() == 1 && self.terms[0].boost == 1.0 {
            return Ok(Some(Arc::new(TermQuery::new(Term::new(
                self.field.clone(),
                self.terms[0].term.clone(),
            )))));
        }
        Ok(None)
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        if score_mode.needs_scores() {
            Ok(Arc::new(SynonymWeight::new(
                self.clone(),
                searcher,
                score_mode,
                boost,
            )?))
        } else {
            // If scores are not needed, let the boolean weight optimise that
            // case.
            let mut bq = BooleanQueryBuilder::new();
            for term in &self.terms {
                bq.add(
                    Arc::new(TermQuery::new(Term::new(
                        self.field.clone(),
                        term.term.clone(),
                    ))),
                    Occur::SHOULD,
                )?;
            }
            let rewritten = searcher.rewrite(Arc::new(bq.build()))?;
            rewritten.create_weight(searcher, ScoreMode::COMPLETE_NO_SCORES, boost)
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<SynonymQuery>() else {
            return false;
        };
        self.field == other.field
            && self.terms.len() == other.terms.len()
            && self
                .terms
                .iter()
                .zip(&other.terms)
                .all(|(a, b)| a.term == b.term && a.boost.to_bits() == b.boost.to_bits())
    }

    fn query_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for t in &self.terms {
            t.term.slice().hash(&mut hasher);
            t.boost.to_bits().hash(&mut hasher);
        }
        let terms_hash = hasher.finish();
        let mut hasher = DefaultHasher::new();
        self.field.hash(&mut hasher);
        31u64
            .wrapping_mul(self.class_hash())
            .wrapping_add(terms_hash)
            .wrapping_add(hasher.finish())
    }
}

/// The state a [`SynonymWeight`] and the scorer suppliers it produces share.
///
/// **Divergence from Lucene 10.5.0.** Java's anonymous `ScorerSupplier` reads
/// the enclosing `SynonymWeight` through the outer instance. A `ScorerSupplier`
/// is `'static` in this port and cannot borrow the weight, so the state lives
/// behind an [`Arc`] that both hold.
struct SynonymWeightState {
    query: SynonymQuery,
    term_states: Vec<Arc<TermStates>>,
    /// `None` when no term exists at all, in which case the similarity is not
    /// used.
    sim_weight: Option<SharedSimScorer>,
    score_mode: ScoreMode,
    similarity: Arc<dyn Similarity>,
}

impl std::fmt::Debug for SynonymWeightState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynonymWeightState")
            .field("query", &self.query)
            .field("score_mode", &self.score_mode)
            .finish_non_exhaustive()
    }
}

impl SynonymWeightState {
    /// Returns the similarity scorer, or one that always answers `0` when no
    /// term exists at all — the path Java reaches with a `null` scorer and
    /// never takes.
    fn sim_weight(&self) -> SharedSimScorer {
        match self.sim_weight.as_ref() {
            Some(sim_weight) => Arc::clone(sim_weight),
            None => Arc::new(crate::search::sim_scorer_source::ZeroSimScorer),
        }
    }

    /// Opens the postings of every query term that exists in the leaf.
    ///
    /// Equivalent to the private `init()` of the anonymous `ScorerSupplier`
    /// that `SynonymWeight.scorerSupplier` returns.
    fn init(&self, context: &LeafReaderContext) -> Result<SynonymInit> {
        let field = self.query.get_field();
        let mut iterators = Vec::new();
        let mut impacts = Vec::new();
        let mut term_boosts = Vec::new();
        let mut cost = 0i64;

        for (i, t) in self.query.terms_and_boosts().iter().enumerate() {
            let Some(state) = self.term_states[i].get(context)? else {
                continue;
            };
            let Some(terms) = context.leaf_reader().terms(field)? else {
                continue;
            };
            let mut terms_enum = terms.iterator()?;
            terms_enum.seek_term_state(&t.term, &*state)?;
            let enumeration: Box<dyn ImpactsEnum> = if self.score_mode == ScoreMode::TOP_SCORES {
                terms_enum.impacts(POSTINGS_ENUM_FREQS)?
            } else {
                Box::new(SlowImpactsEnum::new(
                    terms_enum.postings(None, POSTINGS_ENUM_FREQS)?,
                ))
            };
            // Java keeps the enum twice, in `iterators` and in `impacts`; a
            // shared handle is how this port expresses that aliasing.
            let shared = SharedPostings::new(enumeration);
            impacts.push(shared.clone());
            iterators.push(shared);
            term_boosts.push(t.boost);
        }

        for iterator in &iterators {
            cost += DocIdSetIterator::cost(iterator);
        }

        Ok(SynonymInit {
            iterators,
            impacts,
            term_boosts,
            cost,
        })
    }

    /// Builds the scorer for a leaf, keeping its concrete shape so that
    /// [`SynonymWeight::explain`] can read the frequency off it, which Java
    /// does with three `instanceof` checks.
    ///
    /// Equivalent to the `get(long)` of the anonymous `ScorerSupplier`.
    fn build_scorer(
        &self,
        context: &LeafReaderContext,
        mut init: SynonymInit,
        lead_cost: i64,
    ) -> Result<SynonymScorerKind> {
        let field = self.query.get_field();
        if init.iterators.is_empty() {
            return Ok(SynonymScorerKind::Empty(
                ConstantScoreScorer::from_iterator(
                    0.0,
                    self.score_mode,
                    Box::new(doc_id_set_iterator::empty()),
                ),
            ));
        }

        let sim_weight = self.sim_weight();

        // The "term not in segment" case must be optimised: a disjunction
        // requires two or more subs.
        if init.iterators.len() == 1 {
            let shared = init.iterators.remove(0);
            let boost = init.term_boosts[0];
            let scorer = TermScorer::new(
                Box::new(shared),
                Arc::clone(&sim_weight),
                context.leaf_reader().get_norm_values(field)?,
            );
            return if self.score_mode == ScoreMode::COMPLETE_NO_SCORES || boost == 1.0 {
                Ok(SynonymScorerKind::Term(scorer))
            } else {
                Ok(SynonymScorerKind::FreqBoost(FreqBoostTermScorer::new(
                    boost,
                    scorer,
                    sim_weight,
                    context.leaf_reader().get_norm_values(field)?,
                )?))
            };
        }

        // Term scorers plus a disjunction are used as an implementation detail.
        //
        // **Divergence from Lucene 10.5.0.** Java hands the very same
        // `NumericDocValues` to every term scorer and to the synonym scorer;
        // this port opens one view per scorer, because a `Box<dyn
        // NumericDocValues>` cannot be shared. Only the synonym scorer ever
        // reads norms — the term scorers are used as iterators and for their
        // frequencies — so the values read are the same.
        let mut wrappers = Vec::with_capacity(init.iterators.len());
        for (i, shared) in init.iterators.iter().enumerate() {
            let term_scorer = TermScorer::new(
                Box::new(shared.clone()),
                Arc::clone(&sim_weight),
                context.leaf_reader().get_norm_values(field)?,
            );
            wrappers.push(DisiWrapper::with_weight(
                Box::new(term_scorer),
                false,
                init.term_boosts[i],
            ));
        }
        // Even though it is called an approximation, it is accurate, since none
        // of the sub-iterators is a two-phase iterator.
        let disjunction = DisjunctionDISIApproximation::new(wrappers, lead_cost);

        let impacts_source: Box<dyn ImpactsSource> = Box::new(merge_impacts(
            std::mem::take(&mut init.impacts),
            init.term_boosts.clone(),
        ));
        let mut position_to_original = vec![0usize; init.iterators.len()];
        for (i, &position) in disjunction.original_order().iter().enumerate() {
            position_to_original[position] = i;
        }
        let impacts_disi = ImpactsDISI::new(
            IteratorWithImpacts::new(disjunction, impacts_source),
            MaxScoreCache::new(Box::new(SharedSimScorerRef::new(Arc::clone(&sim_weight)))),
        );

        // TODO(lucene): only do this when this is the top-level scoring clause
        // (`ScorerSupplier#setTopLevelScoringClause`) to save the overhead of
        // wrapping with `ImpactsDISI` when it would not help. This port keeps
        // Lucene's current, unconditional behaviour.
        let use_impacts = self.score_mode == ScoreMode::TOP_SCORES;

        Ok(SynonymScorerKind::Synonym(SynonymScorer {
            impacts_disi,
            use_impacts,
            subs: init.iterators,
            boosts: init.term_boosts,
            position_to_original,
            scorer: sim_weight,
            norms: context.leaf_reader().get_norm_values(field)?,
        }))
    }
}

/// The postings of the query terms that exist in one leaf.
///
/// Equivalent to the `iterators`, `impacts`, `termBoosts` and `cost` fields of
/// the anonymous `ScorerSupplier`.
struct SynonymInit {
    iterators: Vec<SharedPostings>,
    impacts: Vec<SharedPostings>,
    term_boosts: Vec<f32>,
    cost: i64,
}

/// The scorer a [`SynonymWeight`] builds, in the shape Java recovers with
/// `instanceof`.
enum SynonymScorerKind {
    /// No query term exists in the leaf.
    Empty(ConstantScoreScorer),
    /// A single term, with a boost of `1` or without scores.
    Term(TermScorer),
    /// A single term with a boost.
    FreqBoost(FreqBoostTermScorer),
    /// Several terms.
    Synonym(SynonymScorer),
}

impl SynonymScorerKind {
    /// Returns the frequency of the current document, which
    /// [`SynonymWeight::explain`] needs, or `None` for the empty scorer, which
    /// never matches.
    fn freq(&mut self) -> Result<Option<f32>> {
        match self {
            Self::Empty(_) => Ok(None),
            Self::Term(scorer) => Ok(Some(scorer.freq()? as f32)),
            Self::FreqBoost(scorer) => Ok(Some(scorer.freq()?)),
            Self::Synonym(scorer) => Ok(Some(scorer.freq()?)),
        }
    }

    fn into_scorer(self) -> Box<dyn Scorer> {
        match self {
            Self::Empty(scorer) => Box::new(scorer),
            Self::Term(scorer) => Box::new(scorer),
            Self::FreqBoost(scorer) => Box::new(scorer),
            Self::Synonym(scorer) => Box::new(scorer),
        }
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        match self {
            Self::Empty(scorer) => scorer.iterator(),
            Self::Term(scorer) => scorer.iterator(),
            Self::FreqBoost(scorer) => scorer.iterator(),
            Self::Synonym(scorer) => scorer.iterator(),
        }
    }
}

/// The [`Weight`] of a [`SynonymQuery`].
///
/// Equivalent to the inner `SynonymQuery.SynonymWeight`.
#[derive(Debug)]
pub struct SynonymWeight {
    state: Arc<SynonymWeightState>,
    query_handle: Arc<dyn Query>,
}

impl SynonymWeight {
    /// Builds the weight of a synonym query.
    ///
    /// Equivalent to
    /// `SynonymWeight(Query, IndexSearcher, ScoreMode, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: SynonymQuery,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Self> {
        debug_assert!(score_mode.needs_scores());
        let field = query.get_field().to_string();
        let collection_stats = searcher.collection_statistics(&field)?;
        let mut doc_freq = 0i64;
        let mut total_term_freq = 0i64;
        let mut term_states = Vec::with_capacity(query.terms_and_boosts().len());
        for t in query.terms_and_boosts() {
            let term = Term::new(field.clone(), t.term.clone());
            let ts = Arc::new(TermStates::build(searcher, &term, true)?);
            if ts.doc_freq()? > 0 {
                let term_stats =
                    searcher.term_statistics(&term, ts.doc_freq()?, ts.total_term_freq()?)?;
                doc_freq = doc_freq.max(term_stats.doc_freq());
                total_term_freq += term_stats.total_term_freq();
            }
            term_states.push(ts);
        }
        let similarity = Arc::clone(searcher.get_similarity());
        let sim_weight: Option<SharedSimScorer> = if doc_freq > 0 {
            let pseudo_stats = TermStatistics::new(
                BytesRef::new(b"synonym pseudo-term".to_vec()),
                doc_freq,
                total_term_freq,
            )?;
            let collection_stats = collection_stats.ok_or_else(|| {
                LuceneError::IllegalState(format!(
                    "field {field} has term statistics but no collection statistics"
                ))
            })?;
            Some(Arc::new(SimScorerSource::new(
                Arc::clone(&similarity),
                boost,
                collection_stats,
                vec![pseudo_stats],
            )))
        } else {
            // No term exists at all, so the similarity is not used.
            None
        };
        let query_handle: Arc<dyn Query> = Arc::new(query.clone());
        Ok(Self {
            state: Arc::new(SynonymWeightState {
                query,
                term_states,
                sim_weight,
                score_mode,
                similarity,
            }),
            query_handle,
        })
    }
}

impl SegmentCacheable for SynonymWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for SynonymWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query_handle)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let field = self.state.query.get_field().to_string();
        if context.leaf_reader().terms(&field)?.is_none() {
            // Java falls back to `Weight.matches`, whose default confirms the
            // match without positions.
            return default_weight_matches(self, context, doc);
        }
        let term_list = self.state.query.get_terms();
        let query = self.get_query();
        let context = owned_leaf_context(context);
        let supplier_field = field.clone();
        MatchesUtils::for_field(
            field,
            Arc::new(move || {
                from_terms(
                    &context,
                    doc,
                    Arc::clone(&query),
                    &supplier_field,
                    &term_list,
                )
            }),
        )
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let init = self.state.init(context)?;
        let mut scorer = self.state.build_scorer(context, init, i64::MAX)?;
        let new_doc = scorer.iterator().advance(doc)?;
        if new_doc == doc {
            if let Some(freq) = scorer.freq()? {
                let freq_explanation =
                    Explanation::matched(freq, format!("termFreq={freq}"), Vec::new());
                let mut norms = context
                    .leaf_reader()
                    .get_norm_values(self.state.query.get_field())?;
                let mut norm = 1i64;
                if let Some(norms) = norms.as_mut() {
                    if norms.advance_exact(doc)? {
                        norm = norms.long_value()?;
                    }
                }
                let sim_weight = self.state.sim_weight.as_ref().ok_or_else(|| {
                    LuceneError::IllegalState(
                        "a matching document requires a term to exist, and therefore a scorer"
                            .to_string(),
                    )
                })?;
                let score_explanation = sim_weight.explain(&freq_explanation, norm);
                return Ok(Explanation::matched(
                    score_explanation.value().float_value(),
                    format!(
                        "weight({} in {doc}) [{}], result of:",
                        self.state.query.to_query_string(""),
                        similarity_simple_name(&*self.state.similarity)
                    ),
                    vec![score_explanation],
                ));
            }
        }
        Ok(Explanation::no_match("no matching term", Vec::new()))
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        // **Divergence from Lucene 10.5.0.** Java schedules the
        // terms-dictionary lookups in the background and defers `init()` to the
        // first `get()` or `cost()` call. This port's `TermStates` resolves the
        // lookups eagerly and `ScorerSupplier::cost` is declared on `&self`, so
        // the postings are opened while the supplier is built. The scorer
        // produced and the cost reported are the same.
        let init = self.state.init(context)?;
        let cost = init.cost;
        Ok(Some(Box::new(SynonymScorerSupplier {
            state: Arc::clone(&self.state),
            context: owned_leaf_context(context),
            init: Some(init),
            cost,
        })))
    }
}

/// Runs the default [`Weight::matches`], which Java reaches with
/// `super.matches(context, doc)`.
fn default_weight_matches(
    weight: &SynonymWeight,
    context: &LeafReaderContext,
    doc: i32,
) -> Result<Option<Arc<dyn Matches>>> {
    let Some(mut supplier) = weight.scorer_supplier(context)? else {
        return Ok(None);
    };
    let mut scorer = supplier.get(1)?;
    if scorer.iterator().advance(doc)? != doc {
        return Ok(None);
    }
    Ok(Some(MatchesUtils::match_with_no_terms()))
}

/// The [`ScorerSupplier`] of a [`SynonymWeight`].
///
/// Equivalent to the anonymous `ScorerSupplier` returned by
/// `SynonymWeight.scorerSupplier(LeafReaderContext)`.
struct SynonymScorerSupplier {
    state: Arc<SynonymWeightState>,
    context: Arc<LeafReaderContext>,
    init: Option<SynonymInit>,
    cost: i64,
}

impl std::fmt::Debug for SynonymScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynonymScorerSupplier")
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl ScorerSupplier for SynonymScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let init = self.init.take().ok_or_else(|| {
            LuceneError::IllegalState(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })?;
        Ok(self
            .state
            .build_scorer(&self.context, init, lead_cost)?
            .into_scorer())
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// The scorer over several synonyms.
///
/// Equivalent to the private static `SynonymQuery.SynonymScorer`.
struct SynonymScorer {
    impacts_disi: ImpactsDISI<IteratorWithImpacts<DisjunctionDISIApproximation>>,
    /// Whether the iteration goes through the impacts-aware wrapper, which is
    /// what Java's `iterator` field points at under
    /// [`ScoreMode::TOP_SCORES`].
    use_impacts: bool,
    /// The shared postings, in the order the query lists them.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads the frequency off the
    /// `DisiWrapperFreq`s that `disjunctionDisi.topList()` chains, which hold
    /// the postings enum they were built from. The wrappers live inside the
    /// disjunction here and cannot lend out a `PostingsEnum`, so the frequency
    /// comes from the shared postings instead; `top_list` still decides *which*
    /// clauses contribute and in *which order* they are summed, so the
    /// floating-point result is bit-for-bit the one Java computes.
    subs: Vec<SharedPostings>,
    boosts: Vec<f32>,
    /// Maps a `top_list` position back to the index of the clause in
    /// [`subs`](Self::subs).
    position_to_original: Vec<usize>,
    scorer: SharedSimScorer,
    norms: Option<Box<dyn NumericDocValues>>,
}

impl std::fmt::Debug for SynonymScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynonymScorer")
            .field("subs", &self.subs.len())
            .finish_non_exhaustive()
    }
}

impl SynonymScorer {
    /// Returns the boosted sum of the frequencies of the clauses positioned on
    /// the current document.
    ///
    /// Equivalent to the package-private `SynonymScorer.freq()`.
    fn freq(&mut self) -> Result<f32> {
        let mut freq = 0f32;
        for position in self.impacts_disi.inner_ref().iterator_ref().top_list() {
            let i = self.position_to_original[position];
            freq += self.boosts[i] * self.subs[i].freq()? as f32;
        }
        Ok(freq)
    }
}

impl DocIdSetIterator for SynonymScorer {
    fn doc_id(&self) -> i32 {
        if self.use_impacts {
            DocIdSetIterator::doc_id(&self.impacts_disi)
        } else {
            DocIdSetIterator::doc_id(self.impacts_disi.inner_ref())
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.use_impacts {
            self.impacts_disi.next_doc()
        } else {
            self.impacts_disi.inner().next_doc()
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if self.use_impacts {
            self.impacts_disi.advance(target)
        } else {
            self.impacts_disi.inner().advance(target)
        }
    }

    fn cost(&self) -> i64 {
        if self.use_impacts {
            DocIdSetIterator::cost(&self.impacts_disi)
        } else {
            DocIdSetIterator::cost(self.impacts_disi.inner_ref())
        }
    }
}

impl Scorable for SynonymScorer {
    fn score(&mut self) -> Result<f32> {
        let doc = DocIdSetIterator::doc_id(&self.impacts_disi);
        let mut norm = 1i64;
        if let Some(norms) = self.norms.as_mut() {
            if norms.advance_exact(doc)? {
                norm = norms.long_value()?;
            }
        }
        let freq = self.freq()?;
        Ok(self.scorer.score(freq, norm))
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.impacts_disi.set_min_competitive_score(min_score);
        Ok(())
    }
}

impl Scorer for SynonymScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        DocIdSetIterator::doc_id(self)
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        let (source, cache) = self.impacts_disi.split_mut();
        cache.advance_shallow(source, target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let (source, cache) = self.impacts_disi.split_mut();
        cache.get_max_score(source, up_to)
    }
}

/// A [`TermScorer`] whose frequency is boosted.
///
/// Equivalent to the private static `SynonymQuery.FreqBoostTermScorer`, which
/// extends `FilterScorer`.
pub struct FreqBoostTermScorer {
    boost: f32,
    inner: TermScorer,
    scorer: SharedSimScorer,
    norms: Option<Box<dyn NumericDocValues>>,
}

impl std::fmt::Debug for FreqBoostTermScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreqBoostTermScorer")
            .field("boost", &self.boost)
            .finish_non_exhaustive()
    }
}

impl FreqBoostTermScorer {
    /// Boosts the frequency of a term scorer.
    ///
    /// Equivalent to
    /// `FreqBoostTermScorer(float, TermScorer, SimScorer, NumericDocValues)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `boost` is not in
    /// `[0, 1]`, which is the `IllegalArgumentException` Java throws.
    pub fn new(
        boost: f32,
        inner: TermScorer,
        scorer: SharedSimScorer,
        norms: Option<Box<dyn NumericDocValues>>,
    ) -> Result<Self> {
        // Not `!(0.0..=1.0).contains(..)`: Java's two comparisons both answer
        // false for NaN, which the explicit `isNaN` check catches first, and
        // `contains` would fold the three into one.
        #[allow(clippy::manual_range_contains)]
        let out_of_range = boost < 0.0 || boost > 1.0;
        if boost.is_nan() || out_of_range {
            return Err(LuceneError::IllegalArgument(
                "boost must be a positive float between 0 (exclusive) and 1 (inclusive)"
                    .to_string(),
            ));
        }
        Ok(Self {
            boost,
            inner,
            scorer,
            norms,
        })
    }

    /// Returns the boosted frequency of the current document.
    ///
    /// Equivalent to the package-private `FreqBoostTermScorer.freq()`.
    pub fn freq(&mut self) -> Result<f32> {
        Ok(self.boost * self.inner.freq()? as f32)
    }
}

impl Scorable for FreqBoostTermScorer {
    fn score(&mut self) -> Result<f32> {
        let doc = Scorer::doc_id(&self.inner);
        let mut norm = 1i64;
        if let Some(norms) = self.norms.as_mut() {
            if norms.advance_exact(doc)? {
                norm = norms.long_value()?;
            }
        }
        let freq = self.freq()?;
        Ok(self.scorer.score(freq, norm))
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        self.inner.smoothing_score(doc_id)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.inner.set_min_competitive_score(min_score)
    }
}

impl Scorer for FreqBoostTermScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        Scorer::doc_id(&self.inner)
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.inner.iterator()
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.inner.two_phase_iterator()
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.inner.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.inner.get_max_score(up_to)
    }
}

/// Merges the impacts of several synonyms.
///
/// Equivalent to the package-private static
/// `SynonymQuery.mergeImpacts(ImpactsEnum[], float[])`.
///
/// # Panics
///
/// Panics when there are not as many boosts as impacts enums, which Java
/// asserts.
pub fn merge_impacts(impacts_enums: Vec<SharedPostings>, boosts: Vec<f32>) -> SynonymImpactsSource {
    assert_eq!(impacts_enums.len(), boosts.len());
    SynonymImpactsSource {
        impacts_enums,
        boosts,
    }
}

/// The [`ImpactsSource`] [`merge_impacts`] builds.
///
/// Equivalent to the anonymous `ImpactsSource` of
/// `SynonymQuery.mergeImpacts`.
pub struct SynonymImpactsSource {
    impacts_enums: Vec<SharedPostings>,
    boosts: Vec<f32>,
}

impl std::fmt::Debug for SynonymImpactsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynonymImpactsSource")
            .field("terms", &self.impacts_enums.len())
            .finish()
    }
}

impl ImpactsSource for SynonymImpactsSource {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        for impacts_enum in &mut self.impacts_enums {
            if DocIdSetIterator::doc_id(impacts_enum) < target {
                impacts_enum.advance_shallow(target)?;
            }
        }
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        let mut impacts: Vec<Box<dyn Impacts>> = Vec::with_capacity(self.impacts_enums.len());
        // Use the impacts that have the lower next boundary as a lead; it
        // decides on the number of levels and the block boundaries.
        let mut lead_index = 0usize;
        let mut lead_set = false;
        for impacts_enum in &mut self.impacts_enums {
            impacts.push(impacts_enum.get_impacts()?);
            let i = impacts.len() - 1;
            if !lead_set || impacts[i].doc_id_up_to(0) < impacts[lead_index].doc_id_up_to(0) {
                lead_index = i;
                lead_set = true;
            }
        }
        let doc_ids: Vec<i32> = self
            .impacts_enums
            .iter()
            .map(DocIdSetIterator::doc_id)
            .collect();
        Ok(Box::new(SynonymImpacts {
            impacts,
            lead_index,
            boosts: self.boosts.clone(),
            doc_ids,
        }))
    }
}

/// The merged view of the impacts of several synonyms.
///
/// Equivalent to the anonymous `Impacts` of `SynonymQuery.mergeImpacts`.
struct SynonymImpacts {
    impacts: Vec<Box<dyn Impacts>>,
    lead_index: usize,
    boosts: Vec<f32>,
    /// The doc ID of each impacts enum, read when the impacts were taken.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads `impactsEnums[i].docID()`
    /// inside `getImpacts(int)`, which this port cannot do because the enums
    /// are not reachable from the returned `Impacts`; the doc IDs are captured
    /// when `getImpacts()` builds it, and nothing advances the enums in
    /// between.
    doc_ids: Vec<i32>,
}

/// One synonym's impact list, walked in parallel with the others.
///
/// Equivalent to the static nested `SubIterator` of
/// `SynonymQuery.mergeImpacts`.
struct SubIterator {
    buffer: FreqAndNormBuffer,
    index: usize,
    previous_freq: i32,
    freq: i32,
    norm: i64,
    exhausted: bool,
}

impl SubIterator {
    fn new(buffer: FreqAndNormBuffer) -> Self {
        let mut it = Self {
            buffer,
            index: 0,
            previous_freq: 0,
            freq: 0,
            norm: 0,
            exhausted: false,
        };
        it.next();
        it
    }

    fn next(&mut self) {
        self.previous_freq = self.freq;
        if self.index >= self.buffer.size {
            self.exhausted = true;
        } else {
            self.freq = self.buffer.freqs[self.index];
            self.norm = self.buffer.norms[self.index];
            self.index += 1;
        }
    }
}

/// A binary heap of [`SubIterator`]s ordered by norm, with the exhausted ones
/// last.
///
/// Equivalent to the anonymous `PriorityQueue<SubIterator>` of
/// `SynonymQuery.mergeImpacts`; see
/// [`ExactPhraseMatcher`](crate::search::ExactPhraseMatcher) for why the heap
/// is local rather than [`crate::util::PriorityQueue`].
struct SubIteratorQueue {
    heap: Vec<Option<SubIterator>>,
    size: usize,
}

impl SubIteratorQueue {
    fn new(max_size: usize) -> Self {
        let heap_size = if max_size == 0 { 2 } else { max_size + 1 };
        Self {
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
        }
    }

    fn less_than(a: &SubIterator, b: &SubIterator) -> bool {
        if a.exhausted {
            return false;
        }
        if b.exhausted {
            return true;
        }
        (a.norm as u64) < (b.norm as u64)
    }

    fn add(&mut self, element: SubIterator) {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(index);
    }

    fn top(&self) -> &SubIterator {
        self.heap[1]
            .as_ref()
            .expect("INVARIANT: the queue is not empty while merging impacts")
    }

    fn top_mut(&mut self) -> &mut SubIterator {
        self.heap[1]
            .as_mut()
            .expect("INVARIANT: the queue is not empty while merging impacts")
    }

    fn update_top(&mut self) {
        self.down_heap(1);
    }

    fn up_heap(&mut self, orig_pos: usize) {
        let mut i = orig_pos;
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: up_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i >> 1;
        while j > 0 && Self::less_than(&node, self.heap[j].as_ref().expect(occupied)) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
    }

    fn down_heap(&mut self, mut i: usize) {
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: down_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && Self::less_than(
                self.heap[k].as_ref().expect(occupied),
                self.heap[j].as_ref().expect(occupied),
            )
        {
            j = k;
        }
        while j <= self.size && Self::less_than(self.heap[j].as_ref().expect(occupied), &node) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && Self::less_than(
                    self.heap[k].as_ref().expect(occupied),
                    self.heap[j].as_ref().expect(occupied),
                )
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
    }
}

impl SynonymImpacts {
    /// Returns the minimum level whose impacts are valid up to `doc_id_up_to`,
    /// or `-1` if there is no such level.
    ///
    /// Equivalent to the private `getLevel(Impacts, int)`.
    fn get_level(impacts: &dyn Impacts, doc_id_up_to: i32) -> i32 {
        for level in 0..impacts.num_levels() {
            if impacts.doc_id_up_to(level) >= doc_id_up_to {
                return level;
            }
        }
        -1
    }
}

impl Impacts for SynonymImpacts {
    fn num_levels(&self) -> i32 {
        // Delegate to the lead.
        self.impacts[self.lead_index].num_levels()
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        // Delegate to the lead.
        self.impacts[self.lead_index].doc_id_up_to(level)
    }

    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer {
        let doc_id_up_to = self.doc_id_up_to(level);

        let mut to_merge: Vec<FreqAndNormBuffer> = Vec::new();
        let mut merged_impacts = FreqAndNormBuffer::new();

        for i in 0..self.impacts.len() {
            if self.doc_ids[i] <= doc_id_up_to {
                let impacts_level = Self::get_level(&*self.impacts[i], doc_id_up_to);
                if impacts_level == -1 {
                    // One instance does not have impacts that cover up to
                    // `doc_id_up_to`; return impacts that trigger the maximum
                    // score.
                    merged_impacts.grow_no_copy(1);
                    merged_impacts.freqs[0] = i32::MAX;
                    merged_impacts.norms[0] = 1;
                    merged_impacts.size = 1;
                    return merged_impacts;
                }
                let impact_list = if self.boosts[i] != 1.0 {
                    let unboosted = self.impacts[i].get_impacts(impacts_level);
                    let mut boosted = FreqAndNormBuffer::new();
                    boosted.grow_no_copy(unboosted.size);
                    boosted.size = unboosted.size;
                    boosted.norms[..unboosted.size]
                        .copy_from_slice(&unboosted.norms[..unboosted.size]);
                    let boost = self.boosts[i];
                    for j in 0..unboosted.size {
                        boosted.freqs[j] = (unboosted.freqs[j] as f32 * boost).ceil() as i32;
                    }
                    boosted
                } else {
                    self.impacts[i].get_impacts(impacts_level)
                };
                to_merge.push(impact_list);
            }
        }
        // Otherwise it would mean the doc ID is `> doc_id_up_to`, which is
        // wrong.
        debug_assert!(!to_merge.is_empty());

        if to_merge.len() == 1 {
            // Common when one synonym is common and the other one is rare.
            return to_merge.remove(0);
        }

        let mut pq = SubIteratorQueue::new(self.impacts.len().max(to_merge.len()));
        for impacts in to_merge {
            pq.add(SubIterator::new(impacts));
        }

        // Idea: merge impacts by norm. The tricky thing is that norm values not
        // in the impacts must be considered too. For instance if the list of
        // impacts is `[{freq=2,norm=10}, {freq=4,norm=12}]`, there might well be
        // a document that has a freq of 2 and a length of 11, which was just
        // not added to the list of impacts because `{freq=2,norm=10}` is more
        // competitive. So the sum of the term freqs seen so far is tracked, to
        // account for these implicit impacts.
        let mut sum_tf = 0i64;
        loop {
            let norm = pq.top().norm;
            loop {
                {
                    let top = pq.top_mut();
                    sum_tf += i64::from(top.freq - top.previous_freq);
                    top.next();
                }
                pq.update_top();
                if pq.top().exhausted || pq.top().norm != norm {
                    break;
                }
            }

            let freq_upper_bound = sum_tf.min(i64::from(i32::MAX)) as i32;
            if merged_impacts.size == 0 {
                merged_impacts.add(freq_upper_bound, norm);
            } else {
                let prev_freq = merged_impacts.freqs[merged_impacts.size - 1];
                let prev_norm = merged_impacts.norms[merged_impacts.size - 1];
                debug_assert!((prev_norm as u64) < (norm as u64));
                if freq_upper_bound > prev_freq {
                    merged_impacts.add(freq_upper_bound, norm);
                }
                // Otherwise the previous impact is already more competitive.
            }

            if pq.top().exhausted {
                break;
            }
        }

        merged_impacts
    }
}
