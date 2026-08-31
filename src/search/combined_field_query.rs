//! Treating several fields as one stream, ported from
//! `org.apache.lucene.search.CombinedFieldQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{DocAndFloatFeatureBuffer, LeafReaderContext, Term, POSTINGS_ENUM_FREQS};
use crate::search::batch_score_bulk_scorer::BatchScoreBulkScorer;
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::{BooleanQuery, Builder as BooleanQueryBuilder};
use crate::search::bulk_scorer::BulkScorer;
use crate::search::constant_score_weight::java_float_to_string;
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::disjunction_disi_approximation::DisjunctionDISIApproximation;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::index_searcher::{IndexSearcher, TooManyClauses};
use crate::search::matches::Matches;
use crate::search::multi_norms_leaf_sim_scorer::MultiNormsLeafSimScorer;
use crate::search::phrase_matcher::SharedPostings;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::sim_scorer_source::{SharedSimScorer, SimScorerSource};
use crate::search::similarities::{CollectionStatistics, Explanation, TermStatistics};
use crate::search::term_query::TermQuery;
use crate::search::term_range_query::term_bytes_to_string;
use crate::search::term_scorer::TermScorer;
use crate::search::term_states::TermStates;
use crate::search::weight::Weight;
use crate::util::{Accountable, Bits, BytesRef, RamUsageEstimator};

/// The shallow size of a [`CombinedFieldQuery`], standing in for Java's
/// `RamUsageEstimator.shallowSizeOfInstance(CombinedFieldQuery.class)`.
const BASE_RAM_BYTES: i64 =
    3 * RamUsageEstimator::NUM_BYTES_OBJECT_REF + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 8;

/// A field of a [`CombinedFieldQuery`] and the weight it carries.
///
/// Equivalent to the package-private record
/// `CombinedFieldQuery.FieldAndWeight`.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldAndWeight {
    field: String,
    weight: f32,
}

impl FieldAndWeight {
    /// Pairs a field with its weight.
    pub fn new(field: impl Into<String>, weight: f32) -> Self {
        Self {
            field: field.into(),
            weight,
        }
    }

    /// Returns the field name.
    ///
    /// Equivalent to `FieldAndWeight.field()`.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the weight associated with the field.
    ///
    /// Equivalent to `FieldAndWeight.weight()`.
    pub fn weight(&self) -> f32 {
        self.weight
    }
}

/// A builder for [`CombinedFieldQuery`].
///
/// Equivalent to `CombinedFieldQuery.Builder`.
#[derive(Debug, Clone)]
pub struct Builder {
    field_and_weights: BTreeMap<String, FieldAndWeight>,
    term: BytesRef,
}

impl Builder {
    /// Creates a builder for the given term text.
    ///
    /// Equivalent to `Builder(String)`.
    pub fn new(term: &str) -> Self {
        Self::from_bytes(&BytesRef::new(term.as_bytes().to_vec()))
    }

    /// Creates a builder for the given term bytes.
    ///
    /// Equivalent to `Builder(BytesRef)`, which deep-copies the bytes.
    pub fn from_bytes(term: &BytesRef) -> Self {
        Self {
            field_and_weights: BTreeMap::new(),
            term: BytesRef::deep_copy_of(term),
        }
    }

    /// Adds a field with a weight of `1`.
    ///
    /// Equivalent to `Builder.addField(String)`.
    ///
    /// # Errors
    ///
    /// As [`add_field_with_weight`](Self::add_field_with_weight).
    pub fn add_field(&mut self, field: impl Into<String>) -> Result<&mut Self> {
        self.add_field_with_weight(field, 1.0)
    }

    /// Adds a field with the given weight.
    ///
    /// Equivalent to `Builder.addField(String, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the weight is below `1`,
    /// which is the `IllegalArgumentException` Java throws.
    pub fn add_field_with_weight(
        &mut self,
        field: impl Into<String>,
        weight: f32,
    ) -> Result<&mut Self> {
        if weight < 1.0 {
            return Err(LuceneError::IllegalArgument(
                "weight must be greater or equal to 1".to_string(),
            ));
        }
        let field = field.into();
        self.field_and_weights
            .insert(field.clone(), FieldAndWeight::new(field, weight));
        Ok(self)
    }

    /// Builds the [`CombinedFieldQuery`].
    ///
    /// Equivalent to `Builder.build()`.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyClauses`] when more fields than
    /// [`IndexSearcher::get_max_clause_count`] were added.
    pub fn build(&self) -> Result<CombinedFieldQuery> {
        if self.field_and_weights.len() > IndexSearcher::get_max_clause_count() as usize {
            return Err(TooManyClauses::new().into());
        }
        CombinedFieldQuery::new(self.field_and_weights.clone(), self.term.clone())
    }
}

/// A [`Query`] that treats several fields as a single stream and scores terms
/// as if they had been indexed in a single field whose values were the union of
/// the values of the provided fields.
///
/// Equivalent to the `final org.apache.lucene.search.CombinedFieldQuery`, which
/// implements [`Accountable`]. The query works as follows:
///
/// 1. given a list of fields and weights, it pretends there is a synthetic
///    combined field in which all terms have been indexed, and computes new
///    term and collection statistics for that combined field;
/// 2. it uses a disjunction iterator and
///    [`IndexSearcher::get_similarity`] to score documents.
///
/// For a similarity to be compatible,
/// [`Similarity::compute_norm`](crate::search::similarities::Similarity::compute_norm)
/// must be additive — the norm of the combined field is the sum of the norms of
/// each individual field — and the norms must be encoded with
/// [`SmallFloat::int_to_byte4`](crate::util::SmallFloat::int_to_byte4). Those
/// requirements hold for every similarity that does not customise
/// `computeNorm`, which includes
/// [`BM25Similarity`](crate::search::BM25Similarity) and
/// [`DFRSimilarity`](crate::search::DFRSimilarity). Per-field similarities are
/// not supported.
///
/// The query also requires that either all fields or no fields have norms
/// enabled; having only some fields with norms enabled can result in errors. It
/// assumes that all fields share the same analyzer — scores may not make much
/// sense otherwise.
///
/// The scoring is based on BM25F's simple formula, described in
/// <http://www.staff.city.ac.uk/~sb317/papers/foundations_bm25_review.pdf>;
/// this query implements the same approach but allows similarities other than
/// BM25.
#[derive(Debug, Clone)]
pub struct CombinedFieldQuery {
    /// The fields and their weights, sorted by field.
    field_and_weights: BTreeMap<String, FieldAndWeight>,
    /// The term bytes.
    term: BytesRef,
    /// One term per field, sorted by field.
    field_terms: Vec<Term>,
    ram_bytes_used: i64,
}

impl CombinedFieldQuery {
    /// Builds the query from its fields and term.
    ///
    /// Equivalent to the private
    /// `CombinedFieldQuery(TreeMap<String, FieldAndWeight>, BytesRef)`, which
    /// [`Builder::build`] calls; it is public here because Rust has no
    /// package-private visibility.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyClauses`] when there are more fields than
    /// [`IndexSearcher::get_max_clause_count`].
    pub fn new(
        field_and_weights: BTreeMap<String, FieldAndWeight>,
        term: BytesRef,
    ) -> Result<Self> {
        if field_and_weights.len() > IndexSearcher::get_max_clause_count() as usize {
            return Err(TooManyClauses::new().into());
        }
        let field_terms: Vec<Term> = field_and_weights
            .keys()
            .map(|field| Term::new(field.clone(), term.clone()))
            .collect();
        // Java sums the deep sizes of the map, the term array and the term
        // bytes; those helpers are not part of this port, so the figure is
        // estimated from the same three parts. It is only ever an estimate,
        // used for cache accounting.
        let ram_bytes_used = BASE_RAM_BYTES
            + field_and_weights
                .keys()
                .map(|f| f.len() as i64 + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 4)
                .sum::<i64>()
            + field_terms
                .iter()
                .map(|t| {
                    t.field().len() as i64
                        + t.bytes().length as i64
                        + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                })
                .sum::<i64>()
            + term.length as i64
            + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER;
        Ok(Self {
            field_and_weights,
            term,
            field_terms,
            ram_bytes_used,
        })
    }

    /// Returns the fields and their weights, sorted by field.
    ///
    /// Equivalent to reading the
    /// `private final TreeMap<String, FieldAndWeight> fieldAndWeights` field.
    pub fn field_and_weights(&self) -> &BTreeMap<String, FieldAndWeight> {
        &self.field_and_weights
    }

    /// Returns the term bytes.
    ///
    /// Equivalent to reading the `private final BytesRef term` field.
    pub fn term(&self) -> &BytesRef {
        &self.term
    }

    /// Returns one term per field, sorted by field.
    ///
    /// Equivalent to reading the `private final Term[] fieldTerms` field.
    pub fn field_terms(&self) -> &[Term] {
        &self.field_terms
    }

    /// Rewrites to a simple disjunction, for when the score is not needed.
    ///
    /// Equivalent to the private `CombinedFieldQuery.rewriteToBoolean()`.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyClauses`] when the boolean query would have too many
    /// clauses.
    pub fn rewrite_to_boolean(&self) -> Result<BooleanQuery> {
        let mut bq = BooleanQueryBuilder::new();
        for term in &self.field_terms {
            bq.add(Arc::new(TermQuery::new(term.clone())), Occur::SHOULD)?;
        }
        Ok(bq.build())
    }

    /// Checks that either all fields or no fields have norms.
    ///
    /// Equivalent to the private
    /// `CombinedFieldQuery.validateConsistentNorms(IndexReader)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the norms are
    /// inconsistent, which is the `IllegalArgumentException` Java throws.
    fn validate_consistent_norms(&self, searcher: &IndexSearcher) -> Result<()> {
        let mut all_fields_have_norms = true;
        let mut no_fields_have_norms = true;

        for context in searcher.get_leaf_contexts() {
            let field_infos = context.leaf_reader().get_field_infos();
            for field in self.field_and_weights.keys() {
                if let Some(field_info) = field_infos.field_info(field) {
                    all_fields_have_norms &= field_info.has_norms();
                    no_fields_have_norms &= field_info.omits_norms();
                }
            }
        }

        if !all_fields_have_norms && !no_fields_have_norms {
            return Err(LuceneError::IllegalArgument(
                "CombinedFieldQuery requires norms to be consistent across fields: some fields \
                 cannot  have norms enabled, while others have norms disabled"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

impl Accountable for CombinedFieldQuery {
    fn ram_bytes_used(&self) -> i64 {
        self.ram_bytes_used
    }
}

impl Query for CombinedFieldQuery {
    fn to_query_string(&self, _field: &str) -> String {
        let mut builder = String::from("CombinedFieldQuery((");
        for (pos, field_weight) in self.field_and_weights.values().enumerate() {
            if pos != 0 {
                builder.push(' ');
            }
            builder.push_str(field_weight.field());
            if field_weight.weight() != 1.0 {
                builder.push('^');
                builder.push_str(&java_float_to_string(field_weight.weight()));
            }
        }
        builder.push_str(")(");
        builder.push_str(&term_bytes_to_string(&self.term));
        builder.push_str("))");
        builder
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let selected_terms: Vec<Term> = self
            .field_terms
            .iter()
            .filter(|t| visitor.accept_field(t.field()))
            .cloned()
            .collect();
        if !selected_terms.is_empty() {
            let mut v = visitor.get_sub_visitor(Occur::SHOULD, self);
            v.consume_terms(self, &selected_terms);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.field_and_weights.is_empty() {
            return Ok(Some(Arc::new(BooleanQueryBuilder::new().build())));
        }
        Ok(None)
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        self.validate_consistent_norms(searcher)?;
        if score_mode.needs_scores() {
            Ok(Arc::new(CombinedFieldWeight::new(
                self.clone(),
                searcher,
                score_mode,
                boost,
            )?))
        } else {
            // Rewrite to a simple disjunction if the score is not needed.
            let bq: Arc<dyn Query> = Arc::new(self.rewrite_to_boolean()?);
            let rewritten = searcher.rewrite(bq)?;
            rewritten.create_weight(searcher, ScoreMode::COMPLETE_NO_SCORES, boost)
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<CombinedFieldQuery>() else {
            return false;
        };
        self.field_and_weights.len() == other.field_and_weights.len()
            && self
                .field_and_weights
                .iter()
                .zip(&other.field_and_weights)
                .all(|((ka, va), (kb, vb))| {
                    ka == kb && va.field == vb.field && va.weight.to_bits() == vb.weight.to_bits()
                })
            && self.term == other.term
    }

    fn query_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for (field, weight) in &self.field_and_weights {
            field.hash(&mut hasher);
            weight.weight.to_bits().hash(&mut hasher);
        }
        let fields_hash = hasher.finish();
        let mut hasher = DefaultHasher::new();
        self.term.slice().hash(&mut hasher);
        let mut result = self.class_hash();
        result = 31u64.wrapping_mul(result).wrapping_add(fields_hash);
        31u64.wrapping_mul(result).wrapping_add(hasher.finish())
    }
}

/// The state a [`CombinedFieldWeight`] and the scorer suppliers it produces
/// share.
///
/// **Divergence from Lucene 10.5.0.** Java's anonymous `ScorerSupplier` reads
/// the enclosing weight through the outer instance, and the weight keeps the
/// `IndexSearcher` it was built from. A `ScorerSupplier` is `'static` in this
/// port and cannot borrow the weight, so the state lives behind an [`Arc`] that
/// both hold; it carries a clone of the searcher for the same reason — see
/// [`IndexSearcher`]'s `Clone` implementation.
struct CombinedFieldWeightState {
    query: CombinedFieldQuery,
    searcher: IndexSearcher,
    term_states: Vec<Arc<TermStates>>,
    /// `None` when no term exists at all, in which case the similarity is not
    /// used.
    sim_weight: Option<SharedSimScorer>,
}

impl std::fmt::Debug for CombinedFieldWeightState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombinedFieldWeightState")
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl CombinedFieldWeightState {
    fn sim_weight(&self) -> SharedSimScorer {
        match self.sim_weight.as_ref() {
            Some(sim_weight) => Arc::clone(sim_weight),
            // Java would pass `null`, and the scorer is never used because no
            // term exists; a scorer that always answers `0` keeps the
            // unreachable path total.
            None => Arc::new(crate::search::sim_scorer_source::ZeroSimScorer),
        }
    }

    /// Opens the postings of every field term that exists in the leaf.
    ///
    /// Equivalent to the loop at the top of
    /// `CombinedFieldWeight.scorerSupplier(LeafReaderContext)`.
    fn open(&self, context: &LeafReaderContext) -> Result<CombinedFieldInit> {
        let mut iterators = Vec::new();
        let mut fields = Vec::new();
        let mut cost = 0i64;
        for (i, term) in self.query.field_terms().iter().enumerate() {
            let Some(state) = self.term_states[i].get(context)? else {
                continue;
            };
            let Some(terms) = context.leaf_reader().terms(term.field())? else {
                continue;
            };
            let mut terms_enum = terms.iterator()?;
            terms_enum.seek_term_state(term.bytes(), &*state)?;
            let postings = SharedPostings::new(Box::new(crate::index::SlowImpactsEnum::new(
                terms_enum.postings(None, POSTINGS_ENUM_FREQS)?,
            )));
            cost += DocIdSetIterator::cost(&postings);
            iterators.push(postings);
            fields.push(
                self.query
                    .field_and_weights()
                    .get(term.field())
                    .cloned()
                    .ok_or_else(|| {
                        LuceneError::IllegalState(format!(
                            "field {} has a term but no weight",
                            term.field()
                        ))
                    })?,
            );
        }
        Ok(CombinedFieldInit {
            iterators,
            fields,
            cost,
        })
    }

    /// Builds the scorer for a leaf.
    ///
    /// Equivalent to the `get(long)` of the anonymous `ScorerSupplier`.
    fn build_scorer(
        &self,
        context: &LeafReaderContext,
        init: &CombinedFieldInit,
        lead_cost: i64,
    ) -> Result<CombinedFieldScorer> {
        let sim_weight = self.sim_weight();
        let scoring_sim_scorer = MultiNormsLeafSimScorer::new(
            Arc::clone(&sim_weight),
            &context.leaf_reader(),
            &self
                .query
                .field_and_weights()
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            true,
        )?;

        // Term scorers plus a disjunction are used as an implementation detail.
        let mut wrappers = Vec::with_capacity(init.iterators.len());
        for (i, postings) in init.iterators.iter().enumerate() {
            let weight = init.fields[i].weight();
            let scorer = TermScorer::new(Box::new(postings.clone()), Arc::clone(&sim_weight), None);
            wrappers.push(DisiWrapper::with_weight(Box::new(scorer), false, weight));
        }
        // Even though it is called an approximation, it is accurate, since none
        // of the sub-iterators is a two-phase iterator.
        let iterator = DisjunctionDISIApproximation::new(wrappers, lead_cost);
        let mut position_to_original = vec![0usize; init.iterators.len()];
        for (i, &position) in iterator.original_order().iter().enumerate() {
            position_to_original[position] = i;
        }
        Ok(CombinedFieldScorer::new(
            iterator,
            scoring_sim_scorer,
            init.iterators.clone(),
            init.fields.iter().map(FieldAndWeight::weight).collect(),
            position_to_original,
        ))
    }
}

/// The postings of the field terms that exist in one leaf.
#[derive(Debug)]
struct CombinedFieldInit {
    iterators: Vec<SharedPostings>,
    fields: Vec<FieldAndWeight>,
    cost: i64,
}

/// The [`Weight`] of a [`CombinedFieldQuery`].
///
/// Equivalent to the inner `CombinedFieldQuery.CombinedFieldWeight`.
#[derive(Debug)]
pub struct CombinedFieldWeight {
    state: Arc<CombinedFieldWeightState>,
    query_handle: Arc<dyn Query>,
}

impl CombinedFieldWeight {
    /// Builds the weight of a combined-field query.
    ///
    /// Equivalent to
    /// `CombinedFieldWeight(Query, IndexSearcher, ScoreMode, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: CombinedFieldQuery,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Self> {
        debug_assert!(score_mode.needs_scores());
        let mut doc_freq = 0i64;
        let mut total_term_freq = 0i64;
        let mut term_states = Vec::with_capacity(query.field_terms().len());
        for term in query.field_terms() {
            let field = query
                .field_and_weights()
                .get(term.field())
                .cloned()
                .ok_or_else(|| {
                    LuceneError::IllegalState(format!(
                        "field {} has a term but no weight",
                        term.field()
                    ))
                })?;
            let ts = Arc::new(TermStates::build(searcher, term, true)?);
            if ts.doc_freq()? > 0 {
                let term_stats =
                    searcher.term_statistics(term, ts.doc_freq()?, ts.total_term_freq()?)?;
                doc_freq = doc_freq.max(term_stats.doc_freq());
                total_term_freq +=
                    (f64::from(field.weight()) * term_stats.total_term_freq() as f64) as i64;
            }
            term_states.push(ts);
        }
        let sim_weight: Option<SharedSimScorer> = if doc_freq > 0 {
            let pseudo_collection_stats = merge_collection_statistics(&query, searcher)?;
            let pseudo_term_statistics = TermStatistics::new(
                BytesRef::new(b"pseudo_term".to_vec()),
                doc_freq,
                total_term_freq.max(1),
            )?;
            Some(Arc::new(SimScorerSource::new(
                Arc::clone(searcher.get_similarity()),
                boost,
                pseudo_collection_stats,
                vec![pseudo_term_statistics],
            )))
        } else {
            None
        };
        let query_handle: Arc<dyn Query> = Arc::new(query.clone());
        Ok(Self {
            state: Arc::new(CombinedFieldWeightState {
                query,
                searcher: searcher.clone(),
                term_states,
                sim_weight,
            }),
            query_handle,
        })
    }
}

/// Merges the collection statistics of every field into the pseudo field's.
///
/// Equivalent to the private
/// `CombinedFieldWeight.mergeCollectionStatistics(IndexSearcher)`.
///
/// # Errors
///
/// Propagates any I/O error raised while reading the statistics, and the
/// [`CollectionStatistics`] validation error.
fn merge_collection_statistics(
    query: &CombinedFieldQuery,
    searcher: &IndexSearcher,
) -> Result<CollectionStatistics> {
    let mut max_doc = 0i64;
    let mut doc_count = 0i64;
    let mut sum_total_term_freq = 0i64;
    let mut sum_doc_freq = 0i64;
    for field_weight in query.field_and_weights().values() {
        if let Some(collection_stats) = searcher.collection_statistics(field_weight.field())? {
            max_doc = max_doc.max(collection_stats.max_doc());
            doc_count = doc_count.max(collection_stats.doc_count());
            sum_doc_freq = sum_doc_freq.max(collection_stats.sum_doc_freq());
            sum_total_term_freq += (f64::from(field_weight.weight())
                * collection_stats.sum_total_term_freq() as f64)
                as i64;
        }
    }
    CollectionStatistics::new(
        "pseudo_field",
        max_doc,
        doc_count,
        sum_total_term_freq,
        sum_doc_freq,
    )
}

impl SegmentCacheable for CombinedFieldWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for CombinedFieldWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query_handle)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let bq: Arc<dyn Query> = Arc::new(self.state.query.rewrite_to_boolean()?);
        let rewritten = self.state.searcher.rewrite(bq)?;
        let weight = rewritten.create_weight(&self.state.searcher, ScoreMode::COMPLETE, 1.0)?;
        weight.matches(context, doc)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let init = self.state.open(context)?;
        if !init.iterators.is_empty() {
            let mut scorer = self.state.build_scorer(context, &init, i64::MAX)?;
            let new_doc = scorer.iterator().advance(doc)?;
            if new_doc == doc {
                let freq = scorer.freq()?;
                let mut doc_scorer = MultiNormsLeafSimScorer::new(
                    self.state.sim_weight(),
                    &context.leaf_reader(),
                    &self
                        .state
                        .query
                        .field_and_weights()
                        .values()
                        .cloned()
                        .collect::<Vec<_>>(),
                    true,
                )?;
                let freq_explanation =
                    Explanation::matched(freq, format!("termFreq={freq}"), Vec::new());
                let score_explanation = doc_scorer.explain(doc, &freq_explanation)?;
                return Ok(Explanation::matched(
                    score_explanation.value().float_value(),
                    format!(
                        "weight({} in {doc}), result of:",
                        self.state.query.to_query_string("")
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
        let init = self.state.open(context)?;
        if init.iterators.is_empty() {
            return Ok(None);
        }
        let cost = init.cost;
        Ok(Some(Box::new(CombinedFieldScorerSupplier {
            state: Arc::clone(&self.state),
            context: crate::search::matches::owned_leaf_context(context),
            init,
            cost,
        })))
    }
}

/// The [`ScorerSupplier`] of a [`CombinedFieldWeight`].
///
/// Equivalent to the anonymous `ScorerSupplier` returned by
/// `CombinedFieldWeight.scorerSupplier(LeafReaderContext)`.
struct CombinedFieldScorerSupplier {
    state: Arc<CombinedFieldWeightState>,
    context: Arc<LeafReaderContext>,
    init: CombinedFieldInit,
    cost: i64,
}

impl std::fmt::Debug for CombinedFieldScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombinedFieldScorerSupplier")
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl ScorerSupplier for CombinedFieldScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        Ok(Box::new(self.state.build_scorer(
            &self.context,
            &self.init,
            lead_cost,
        )?))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        Ok(Box::new(BatchScoreBulkScorer::new(self.get(i64::MAX)?)))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// The scorer of a [`CombinedFieldQuery`].
///
/// Equivalent to the private static
/// `CombinedFieldQuery.CombinedFieldScorer`.
pub struct CombinedFieldScorer {
    iterator: DisjunctionDISIApproximation,
    sim_scorer: MultiNormsLeafSimScorer,
    max_score: f32,
    /// The shared postings, in the order the query lists them.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads the frequency off the
    /// `DisiWrapper`s that `iterator.topList()` chains, which hold the postings
    /// enum they were built from. The wrappers live inside the disjunction here
    /// and cannot lend out a `PostingsEnum`, so the frequency comes from the
    /// shared postings instead; `top_list` still decides *which* clauses
    /// contribute and in *which order* they are summed, so the floating-point
    /// result is bit-for-bit the one Java computes.
    subs: Vec<SharedPostings>,
    weights: Vec<f32>,
    /// Maps a `top_list` position back to the index of the clause in
    /// [`subs`](Self::subs).
    position_to_original: Vec<usize>,
}

impl std::fmt::Debug for CombinedFieldScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombinedFieldScorer")
            .field("subs", &self.subs.len())
            .field("max_score", &self.max_score)
            .finish_non_exhaustive()
    }
}

impl CombinedFieldScorer {
    /// Creates the scorer.
    ///
    /// Equivalent to
    /// `CombinedFieldScorer(DisjunctionDISIApproximation, MultiNormsLeafSimScorer)`.
    pub fn new(
        iterator: DisjunctionDISIApproximation,
        sim_scorer: MultiNormsLeafSimScorer,
        subs: Vec<SharedPostings>,
        weights: Vec<f32>,
        position_to_original: Vec<usize>,
    ) -> Self {
        let max_score = sim_scorer.get_sim_scorer().score(f32::INFINITY, 1);
        Self {
            iterator,
            sim_scorer,
            max_score,
            subs,
            weights,
            position_to_original,
        }
    }

    /// Returns the weighted sum of the frequencies of the clauses positioned on
    /// the current document.
    ///
    /// Equivalent to the package-private `CombinedFieldScorer.freq()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the frequencies.
    pub fn freq(&mut self) -> Result<f32> {
        let positions = self.iterator.top_list();
        let mut freq = 0f32;
        let mut first = true;
        for position in positions {
            let i = self.position_to_original[position];
            freq += self.subs[i].freq()? as f32 * self.weights[i];
            if !first && freq < 0.0 {
                // Overflow.
                return Ok(i32::MAX as f32);
            }
            first = false;
        }
        Ok(freq)
    }
}

impl DocIdSetIterator for CombinedFieldScorer {
    fn doc_id(&self) -> i32 {
        self.iterator.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.iterator.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.iterator.advance(target)
    }

    fn cost(&self) -> i64 {
        DocIdSetIterator::cost(&self.iterator)
    }
}

impl Scorable for CombinedFieldScorer {
    fn score(&mut self) -> Result<f32> {
        let doc = self.iterator.doc_id();
        let freq = self.freq()?;
        self.sim_scorer.score(doc, freq)
    }
}

impl Scorer for CombinedFieldScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.iterator.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iterator
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(self.max_score)
    }

    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        let batch_size = 64; // arbitrary
        buffer.grow_no_copy(batch_size);
        let mut size = 0;
        let mut doc = self.iterator.doc_id();
        while doc < up_to && size < batch_size {
            if live_docs.map_or(true, |bits| bits.get(doc as usize)) {
                buffer.docs[size] = doc;
                buffer.features[size] = self.freq()?;
                size += 1;
            }
            doc = self.iterator.next_doc()?;
        }
        buffer.size = size;
        self.sim_scorer.score_range(buffer)
    }
}
