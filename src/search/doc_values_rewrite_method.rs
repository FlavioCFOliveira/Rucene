//! Rewriting a multi-term query against doc values, ported from
//! `org.apache.lucene.search.DocValuesRewriteMethod`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::{Arc, Mutex};

use crate::error::{LuceneError, Result};
use crate::index::{
    DocValues, LeafReader, LeafReaderContext, SortedSetDocValues, SortedSetDocValuesTermsEnum,
    Terms, TermsEnum,
};
use crate::search::boolean_clause::Occur;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_values_range_iterator::{build_ordinal_set, DocValuesRangeIterator};
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::{owned_leaf_context, Matches, MatchesUtils};
use crate::search::multi_term_query::{get_terms_enum, MultiTermQuery, RewriteMethod};
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

    /// Two doc-values rewrite methods are always equal.
    ///
    /// Equivalent to `DocValuesRewriteMethod#equals`
    /// (`DocValuesRewriteMethod.java:195-197`), which is
    /// `other != null && getClass() == other.getClass()`. Unlike the
    /// singletons of [`MultiTermQuery`], this class is instantiated freely, so
    /// Java overrides `equals` to compare the class rather than the identity;
    /// comparing the concrete type is the same test, and a `&dyn` reference
    /// cannot be `null`.
    fn method_eq(&self, other: &dyn RewriteMethod) -> bool {
        other.as_any().is::<DocValuesRewriteMethod>()
    }

    /// Returns `641`, the constant every instance hashes to.
    ///
    /// Equivalent to `DocValuesRewriteMethod#hashCode`
    /// (`DocValuesRewriteMethod.java:200-202`).
    fn method_hash(&self) -> i32 {
        641
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
    /// Equivalent to `MultiTermQueryDocValuesWrapper.toString(String)`.
    fn to_query_string(&self, field: &str) -> String {
        // The wrapped query's rendering is fine for the filter too, since the
        // query boost is 1.
        (&*self.query as &dyn Query).to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(self.query.get_field()) {
            // Java pulls the sub-visitor and discards it, so the wrapped query
            // is never actually visited; reproduced here so that a visitor
            // observing `getSubVisitor` sees the same calls.
            let _ = visitor.get_sub_visitor(Occur::FILTER, &*self.query as &dyn Query);
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
        (&*self.query as &dyn Query).query_eq(&*other.query as &dyn Query)
    }

    /// `getClass().getName()` of `org.apache.lucene.search.DocValuesRewriteMethod$MultiTermQueryDocValuesWrapper`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.DocValuesRewriteMethod$MultiTermQueryDocValuesWrapper"
    }

    fn query_hash(&self) -> i32 {
        // `31 * classHash() + query.hashCode()` (`DocValuesRewriteMethod.java:80-82`).
        31i32
            .wrapping_mul(self.class_hash())
            .wrapping_add((&*self.query as &dyn Query).query_hash())
    }
}

/// The doc-values-backed [`Terms`] a [`MultiTermQuery`] enumerates.
///
/// Equivalent to the anonymous `Terms` built by
/// `MultiTermQueryDocValuesWrapper`'s weight in `getTermsEnum(values)`
/// (`DocValuesRewriteMethod.java:96-142`), whose `iterator()` is
/// `values.termsEnum()` over the weight's own values object and whose
/// statistics are unsupported. The object is shared through a lock because a
/// [`Terms`] may be handed to another thread.
struct DocValuesTerms {
    values: Arc<Mutex<Box<dyn SortedSetDocValues>>>,
}

impl Terms for DocValuesTerms {
    /// `values.termsEnum()` (`DocValuesRewriteMethod.java:103-105`): the
    /// implementation's own enum (Lucene90's `TermsDict`), or the default
    /// `SortedSetDocValuesTermsEnum` over the shared object.
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        let independent = self
            .values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .independent_terms_enum()?;
        Ok(match independent {
            Some(terms_enum) => terms_enum,
            None => SortedSetDocValuesTermsEnum::new(Arc::clone(&self.values)),
        })
    }

    /// Equivalent to the anonymous `Terms.size()` (`DocValuesRewriteMethod.java:122-125`).
    fn size(&self) -> Result<i64> {
        Ok(-1)
    }

    /// Throws `UnsupportedOperationException`, as Java's does
    /// (`DocValuesRewriteMethod.java:107-110`).
    fn sum_total_term_freq(&self) -> Result<i64> {
        // No message, as Java's `new UnsupportedOperationException()`.
        Err(LuceneError::unsupported_operation(String::new()))
    }

    /// Throws `UnsupportedOperationException`, as Java's does
    /// (`DocValuesRewriteMethod.java:112-115`).
    fn sum_doc_freq(&self) -> Result<i64> {
        // No message, as Java's `new UnsupportedOperationException()`.
        Err(LuceneError::unsupported_operation(String::new()))
    }

    /// Throws `UnsupportedOperationException`, as Java's does
    /// (`DocValuesRewriteMethod.java:117-120`).
    fn doc_count(&self) -> Result<i32> {
        // No message, as Java's `new UnsupportedOperationException()`.
        Err(LuceneError::unsupported_operation(String::new()))
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
}

impl DocValuesWeightImpl {
    /// Creates a terms enum providing the intersection of the query terms with
    /// the terms present in `values`.
    ///
    /// Equivalent to the private `getTermsEnum(SortedSetDocValues)` of the
    /// anonymous weight (`DocValuesRewriteMethod.java:96-142`).
    fn get_terms_enum(
        query: &dyn MultiTermQuery,
        values: &Arc<Mutex<Box<dyn SortedSetDocValues>>>,
    ) -> Result<Box<dyn TermsEnum>> {
        let terms: Arc<dyn Terms> = Arc::new(DocValuesTerms {
            values: Arc::clone(values),
        });
        get_terms_enum(query, &terms)
    }
}

impl ConstantScoreWeightImpl for DocValuesWeightImpl {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        // `DocValues.getSortedSet(context.reader(), query.field)`
        // (`DocValuesRewriteMethod.java:151`), with its `checkField`.
        let values = DocValues::get_sorted_set(reader.as_ref(), self.query.get_field())?;
        if values.get_value_count()? == 0 {
            // No values or docs, so nothing can match.
            return Ok(None);
        }
        let cost = values.cost();
        Ok(Some(Box::new(DocValuesScorerSupplier {
            reader,
            query: Arc::clone(&self.query),
            values: Some(values),
            score,
            score_mode: self.score_mode,
            cost,
        })))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        // `DocValues.isCacheable(ctx, query.field)` (`DocValuesRewriteMethod.java:188`).
        DocValues::is_cacheable(ctx, &[self.query.get_field()])
    }

    fn matches(
        &self,
        context: &LeafReaderContext,
        doc: i32,
        _score: f32,
    ) -> Result<Option<Arc<dyn Matches>>> {
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
                // `final SortedSetDocValues values = DocValues.getSortedSet(context.reader(),
                // query.field)` and `getTermsEnum(values)` (`DocValuesRewriteMethod.java:85-90`).
                let values = Arc::new(Mutex::new(DocValues::get_sorted_set(
                    reader.as_ref(),
                    &supplier_field,
                )?));
                let terms_enum = DocValuesWeightImpl::get_terms_enum(&*query, &values)?;
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
    query: Arc<dyn MultiTermQuery>,
    /// The `values` the anonymous supplier captures; `get` takes them.
    values: Option<Box<dyn SortedSetDocValues>>,
    score: f32,
    score_mode: ScoreMode,
    cost: i64,
}

impl std::fmt::Debug for DocValuesScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocValuesScorerSupplier")
            .field("field", &self.query.get_field())
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl ScorerSupplier for DocValuesScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let values = self.values.take().ok_or_else(|| {
            LuceneError::illegal_state(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })?;
        let field = self.query.get_field();
        // Create a TermsEnum that will provide the intersection of the terms
        // specified in the query with the values present in the doc values:
        // `getTermsEnum(values)`, over this very values object.
        let values = Arc::new(Mutex::new(values));
        let mut terms_enum = DocValuesWeightImpl::get_terms_enum(&*self.query, &values)?;
        // Leverage a DV skipper if one was indexed for the field.
        let skipper = self.reader.get_doc_values_skipper(field)?;
        // `DocValuesRangeIterator.forOrdinalSet(values-or-singleton, skipper,
        // termsEnum)` (`DocValuesRewriteMethod.java:164-174`) walks the terms
        // enum into the ordinal set, then keeps the values for the documents;
        // the walk is done here, before the values leave the shared handle.
        let ord_count = values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_value_count()?;
        let ordinal_set = build_ordinal_set(&mut *terms_enum, ord_count)?;
        drop(terms_enum);
        let values = Arc::try_unwrap(values)
            .map_err(|_| {
                LuceneError::illegal_state(
                    "the doc values are still referenced by a terms enum".to_string(),
                )
            })?
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // `SortedDocValues singleton = DocValues.unwrapSingleton(values);`
        let iterator: Box<dyn TwoPhaseIterator> = if values.is_singleton() {
            let singleton = DocValues::unwrap_singleton_sorted(values)?
                .expect("INVARIANT: is_singleton answered true");
            Box::new(DocValuesRangeIterator::for_ordinal_set_inner(
                singleton,
                skipper,
                ordinal_set,
            ))
        } else {
            Box::new(DocValuesRangeIterator::for_sorted_set_ordinal_set_inner(
                values,
                skipper,
                ordinal_set,
            ))
        };
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

#[cfg(test)]
mod shared_values_tests {
    use super::*;
    use crate::codecs::{register_codec, Lucene104Codec};
    use crate::document::{Document, SortedSetDocValuesField};
    use crate::index::directory_reader;
    use crate::index::index_writer::IndexWriter;
    use crate::index::index_writer_config::IndexWriterConfig;
    use crate::index::Term;
    use crate::search::prefix_query::PrefixQuery;
    use crate::store::{ByteBuffersDirectory, Directory};
    use crate::util::BytesRef;

    /// Regression for rmp #668: the anonymous `Terms` of
    /// `getTermsEnum(values)` answers `values.termsEnum()` over the weight's
    /// own values object (`DocValuesRewriteMethod.java:96-105`), the codec's
    /// `TermsDict` on Lucene90 values, and the query's terms enum yields the
    /// values' terms the query accepts.
    #[test]
    fn the_query_terms_enum_reads_the_weights_values_object() {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for value in ["ab", "b", "ac", "c"] {
            let mut doc = Document::new();
            doc.add(SortedSetDocValuesField::new(
                "f",
                BytesRef::new(value.as_bytes().to_vec()),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();
        let reader = directory_reader::open(dir).unwrap();
        let leaves = Arc::clone(&reader).leaves().unwrap();
        let values = Arc::new(Mutex::new(
            DocValues::get_sorted_set(leaves[0].leaf_reader().as_ref(), "f").unwrap(),
        ));

        // `values.termsEnum()` of Lucene90 values is the codec's own
        // `TermsDict` (`Lucene90DocValuesProducer.java:1567-1570`).
        let terms = DocValuesTerms {
            values: Arc::clone(&values),
        };
        let dictionary = terms.iterator().unwrap();
        assert!(
            dictionary
                .to_string()
                .contains("Lucene90DocValuesProducer$TermsDict@"),
            "{dictionary}"
        );
        drop((dictionary, terms));

        let query = PrefixQuery::new(Term::from_text("f", "a")).unwrap();
        let mut terms_enum = DocValuesWeightImpl::get_terms_enum(&query, &values).unwrap();
        let mut terms = Vec::new();
        while let Some(term) = terms_enum.next().unwrap() {
            terms.push(term.slice().to_vec());
        }
        assert_eq!(terms, vec![b"ab".to_vec(), b"ac".to_vec()]);
        drop(terms_enum);
        assert_eq!(Arc::strong_count(&values), 1);
        drop(values);
        reader.close().unwrap();
    }
}
