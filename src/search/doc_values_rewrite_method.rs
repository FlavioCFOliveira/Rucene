//! Rewriting a multi-term query against doc values, ported from
//! `org.apache.lucene.search.DocValuesRewriteMethod`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    DocValues, LeafReader, LeafReaderContext, SortedSetDocValues, SortedSetDocValuesTermsEnum,
    Terms, TermsEnum,
};
use crate::search::boolean_clause::Occur;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_values_range_iterator::DocValuesRangeIterator;
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::{owned_leaf_context, Matches, MatchesUtils};
use crate::search::multi_term_query::{
    get_terms_enum, rewrite_method_type_hash, MultiTermQuery, RewriteMethod,
};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::search::weight::Weight;

/// Rewrites a [`MultiTermQuery`] into a filter, using doc values for term
/// enumeration.
///
/// Equivalent to the `final org.apache.lucene.search.DocValuesRewriteMethod`.
/// This makes it possible to run such a query against a field that has doc
/// values but is not indexed.
#[derive(Debug, Default, Clone, Copy)]
pub struct DocValuesRewriteMethod;

impl RewriteMethod for DocValuesRewriteMethod {
    fn rewrite(
        &self,
        _index_searcher: &IndexSearcher,
        query: Arc<dyn MultiTermQuery>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(ConstantScoreQuery::new(Arc::new(
            MultiTermQueryDocValuesWrapper::new(query),
        ))))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn method_hash(&self) -> u64 {
        // Java answers the constant 641; the type hash keeps the value
        // consistent with the rest of this port's rewrite methods, and the two
        // are equally arbitrary.
        rewrite_method_type_hash(self).wrapping_add(641)
    }
}

/// Wraps a [`MultiTermQuery`] as a doc-values filter.
///
/// Equivalent to the package-private
/// `DocValuesRewriteMethod.MultiTermQueryDocValuesWrapper`; it is public here
/// because Rust has no package visibility.
#[derive(Debug, Clone)]
pub struct MultiTermQueryDocValuesWrapper {
    query: Arc<dyn MultiTermQuery>,
}

impl MultiTermQueryDocValuesWrapper {
    /// Wraps a multi-term query as a filter.
    ///
    /// Equivalent to `MultiTermQueryDocValuesWrapper(MultiTermQuery)`.
    pub fn new(query: Arc<dyn MultiTermQuery>) -> Self {
        Self { query }
    }

    /// Returns the field name for this query.
    ///
    /// Equivalent to the `final
    /// MultiTermQueryDocValuesWrapper.getField()`.
    pub fn get_field(&self) -> &str {
        self.query.get_field()
    }
}

impl Query for MultiTermQueryDocValuesWrapper {
    fn to_query_string(&self, field: &str) -> String {
        // The wrapped query's rendering is fine for the filter too, since the
        // query boost is 1.
        self.query.as_query().to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(self.query.get_field()) {
            // Java pulls the sub-visitor and discards it, so the wrapped query
            // is never actually visited; reproduced here so that a visitor
            // observing `getSubVisitor` sees the same calls.
            let _ = visitor.get_sub_visitor(Occur::FILTER, self.query.as_query());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            DocValuesWeightImpl {
                query: Arc::clone(&self.query),
                score_mode,
                score: boost,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other
            .as_any()
            .downcast_ref::<MultiTermQueryDocValuesWrapper>()
        else {
            return false;
        };
        self.query.as_query().query_eq(other.query.as_query())
    }

    fn query_hash(&self) -> u64 {
        31u64
            .wrapping_mul(self.class_hash())
            .wrapping_add(self.query.as_query().query_hash())
    }
}

/// Opens the sorted-set view of a doc-values field.
///
/// Equivalent to `DocValues.getSortedSet(LeafReader, String)`, whose
/// single-valued branch wraps a `SortedDocValues` in a singleton and whose
/// missing-field branch answers the empty instance.
///
/// **Divergence from Lucene 10.5.0.** Java's `checkField` reports the
/// mismatched doc-values type before answering the empty instance; this port
/// has no `DocValuesType` check to make here, so a field with the wrong type
/// simply has no sorted or sorted-set view and yields the empty instance.
///
/// # Errors
///
/// Propagates any I/O error raised while opening the doc values.
fn get_sorted_set(
    reader: &Arc<dyn LeafReader>,
    field: &str,
) -> Result<Box<dyn SortedSetDocValues>> {
    if let Some(dv) = reader.get_sorted_set_doc_values(field)? {
        return Ok(dv);
    }
    if let Some(sorted) = reader.get_sorted_doc_values(field)? {
        return Ok(Box::new(DocValues::singleton_sorted(sorted)));
    }
    Ok(Box::new(DocValues::empty_sorted_set()))
}

/// Returns whether the doc-values field has not been updated.
///
/// Equivalent to `DocValues.isCacheable(LeafReaderContext, String...)`.
fn doc_values_is_cacheable(ctx: &LeafReaderContext, field: &str) -> bool {
    match ctx.leaf_reader().get_field_infos().field_info(field) {
        Some(fi) => fi.doc_values_gen <= -1,
        None => true,
    }
}

/// The doc-values-backed [`Terms`] a [`MultiTermQuery`] enumerates.
///
/// Equivalent to the anonymous `Terms` built by
/// `MultiTermQueryDocValuesWrapper`'s weight, whose `iterator()` is
/// `values.termsEnum()` and whose statistics are unsupported.
///
/// **Divergence from Lucene 10.5.0.** Java closes over a single
/// `SortedSetDocValues` instance and hands out terms enums over it; this port's
/// [`SortedSetDocValuesTermsEnum`] owns the values it reads, so each
/// [`Terms::iterator`] call opens a fresh view of the same field. The terms
/// enumerated are the same, and the caller can no longer disturb the doc
/// iteration of the values instance the scorer uses.
struct DocValuesTerms {
    reader: Arc<dyn LeafReader>,
    field: String,
}

impl Terms for DocValuesTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(SortedSetDocValuesTermsEnum::new(get_sorted_set(
            &self.reader,
            &self.field,
        )?))
    }

    fn size(&self) -> i64 {
        -1
    }

    fn sum_total_term_freq(&self) -> i64 {
        // Java throws `UnsupportedOperationException`; the trait cannot report
        // an error from these accessors, and nothing in this rewrite reads
        // them, so the "unknown" sentinel stands in.
        -1
    }

    fn sum_doc_freq(&self) -> i64 {
        -1
    }

    fn doc_count(&self) -> i32 {
        -1
    }

    fn has_freqs(&self) -> bool {
        false
    }

    fn has_offsets(&self) -> bool {
        false
    }

    fn has_positions(&self) -> bool {
        false
    }

    fn has_payloads(&self) -> bool {
        false
    }
}

/// The [`ConstantScoreWeightImpl`] of a [`MultiTermQueryDocValuesWrapper`].
///
/// Equivalent to the anonymous `ConstantScoreWeight`
/// `MultiTermQueryDocValuesWrapper.createWeight` returns.
#[derive(Debug)]
struct DocValuesWeightImpl {
    query: Arc<dyn MultiTermQuery>,
    score_mode: ScoreMode,
    score: f32,
}

impl DocValuesWeightImpl {
    /// Creates a terms enum providing the intersection of the query terms with
    /// the terms present in the doc values.
    ///
    /// Equivalent to the private
    /// `getTermsEnum(SortedSetDocValues)` of the anonymous weight.
    fn query_terms_enum(&self, reader: &Arc<dyn LeafReader>) -> Result<Box<dyn TermsEnum>> {
        let terms: Arc<dyn Terms> = Arc::new(DocValuesTerms {
            reader: Arc::clone(reader),
            field: self.query.get_field().to_string(),
        });
        get_terms_enum(&*self.query, &terms)
    }
}

impl ConstantScoreWeightImpl for DocValuesWeightImpl {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let values = get_sorted_set(&reader, self.query.get_field())?;
        if values.get_value_count()? == 0 {
            // No values or docs, so nothing can match.
            return Ok(None);
        }
        let cost = values.cost();
        let terms_enum = self.query_terms_enum(&reader)?;
        Ok(Some(Box::new(DocValuesScorerSupplier {
            reader,
            field: self.query.get_field().to_string(),
            terms_enum: Some(terms_enum),
            score: self.score,
            score_mode: self.score_mode,
            cost,
        })))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        doc_values_is_cacheable(ctx, self.query.get_field())
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let reader = context.leaf_reader();
        let field = self.query.get_field().to_string();
        let query = Arc::clone(&self.query);
        let supplier_field = field.clone();
        // Java captures the `LeafReaderContext`; this port looks the owning
        // handle up the way the constant-score wrappers do, but the matches
        // iterator only needs the reader and the doc, so the reader handle is
        // captured directly.
        let context_handle = owned_leaf_context(context);
        MatchesUtils::for_field(
            field,
            Arc::new(move || {
                let terms: Arc<dyn Terms> = Arc::new(DocValuesTerms {
                    reader: Arc::clone(&reader),
                    field: supplier_field.clone(),
                });
                let terms_enum = get_terms_enum(&*query, &terms)?;
                crate::search::disjunction_matches_iterator::from_terms_enum(
                    &context_handle,
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
}

/// The [`ScorerSupplier`] of a [`MultiTermQueryDocValuesWrapper`].
///
/// Equivalent to the anonymous `ScorerSupplier` the weight returns.
struct DocValuesScorerSupplier {
    reader: Arc<dyn LeafReader>,
    field: String,
    terms_enum: Option<Box<dyn TermsEnum>>,
    score: f32,
    score_mode: ScoreMode,
    cost: i64,
}

impl std::fmt::Debug for DocValuesScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocValuesScorerSupplier")
            .field("field", &self.field)
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl ScorerSupplier for DocValuesScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        // Create a terms enum that provides the intersection of the terms the
        // query specifies with the values present in the doc values.
        let mut terms_enum = self.terms_enum.take().ok_or_else(|| {
            LuceneError::IllegalState(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })?;
        let values = get_sorted_set(&self.reader, &self.field)?;
        // Leverage a doc-values skipper when one was indexed for the field.
        let skipper = self.reader.get_doc_values_skipper(&self.field)?;
        let iterator: Box<dyn TwoPhaseIterator> = Box::new(
            DocValuesRangeIterator::for_sorted_set_ordinal_set(values, skipper, &mut *terms_enum)?,
        );
        Ok(Box::new(ConstantScoreScorer::from_two_phase(
            self.score,
            self.score_mode,
            iterator,
        )))
    }

    fn cost(&self) -> i64 {
        // There is no prior knowledge of how many docs might match any given
        // query term, so every doc with a value is assumed to be a match.
        self.cost
    }
}
