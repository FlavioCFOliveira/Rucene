//! Field-presence queries, ported from
//! `org.apache.lucene.search.FieldExistsQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexReaderContext, LeafReader,
    LeafReaderContext, VectorEncoding,
};
use crate::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::two_phase_iterator::ScorerIterator;
use crate::search::weight::Weight;

/// A [`Query`] that matches documents that contain either a value for a given
/// field, or points, norms or vectors on it.
///
/// Equivalent to `org.apache.lucene.search.FieldExistsQuery`.
#[derive(Debug, Clone)]
pub struct FieldExistsQuery {
    field: String,
}

impl FieldExistsQuery {
    /// Creates a query that matches every document with a value for `field`.
    ///
    /// Equivalent to `new FieldExistsQuery(String)`; Java's
    /// `Objects.requireNonNull` is unnecessary because a `&str` cannot be null.
    pub fn new(field: &str) -> Self {
        Self {
            field: field.to_string(),
        }
    }

    /// Returns the field this query matches on.
    ///
    /// Equivalent to `FieldExistsQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns a [`DocIdSetIterator`] over the documents that have a doc-values
    /// value for `field`, or `None` when the field has no doc values in the
    /// leaf.
    ///
    /// Equivalent to the static
    /// `FieldExistsQuery.getDocValuesDocIdSetIterator(String, LeafReader)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the doc values.
    pub fn get_doc_values_doc_id_set_iterator(
        field: &str,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(field) else {
            return Ok(None);
        };
        Ok(match field_info.get_doc_values_type() {
            DocValuesType::NONE => None,
            DocValuesType::NUMERIC => reader
                .get_numeric_doc_values(field)?
                .map(|v| v as Box<dyn DocIdSetIterator>),
            DocValuesType::BINARY => reader
                .get_binary_doc_values(field)?
                .map(|v| v as Box<dyn DocIdSetIterator>),
            DocValuesType::SORTED => reader
                .get_sorted_doc_values(field)?
                .map(|v| v as Box<dyn DocIdSetIterator>),
            DocValuesType::SORTED_NUMERIC => reader
                .get_sorted_numeric_doc_values(field)?
                .map(|v| v as Box<dyn DocIdSetIterator>),
            DocValuesType::SORTED_SET => reader
                .get_sorted_set_doc_values(field)?
                .map(|v| v as Box<dyn DocIdSetIterator>),
        })
    }

    /// Equivalent to the private `FieldExistsQuery.buildErrorMsg(FieldInfo)`.
    fn build_error_msg(name: &str) -> String {
        format!(
            "FieldExistsQuery requires that the field indexes doc values, norms or vectors, \
             but field '{name}' exists and indexes neither of these data structures"
        )
    }

    /// Equivalent to the private
    /// `FieldExistsQuery.getVectorValuesSize(FieldInfo, LeafReader)`.
    fn vector_values_size(
        encoding: VectorEncoding,
        field: &str,
        reader: &dyn LeafReader,
    ) -> Result<i32> {
        Ok(match encoding {
            VectorEncoding::FLOAT32 => {
                let float_vector_values = reader.get_float_vector_values(field)?;
                debug_assert!(
                    float_vector_values.is_some(),
                    "unexpected null float vector values"
                );
                float_vector_values.map_or(0, |values| values.size())
            }
            VectorEncoding::BYTE => {
                let byte_vector_values = reader.get_byte_vector_values(field)?;
                debug_assert!(
                    byte_vector_values.is_some(),
                    "unexpected null byte vector values"
                );
                byte_vector_values.map_or(0, |values| values.size())
            }
        })
    }
}

/// The weight of a [`FieldExistsQuery`].
///
/// Equivalent to the anonymous `ConstantScoreWeight` that
/// `FieldExistsQuery.createWeight` returns.
#[derive(Debug)]
struct FieldExistsWeight {
    field: String,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for FieldExistsWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(&self.field) else {
            return Ok(None);
        };

        let iterator: Option<Box<dyn DocIdSetIterator>> = if field_info.has_norms() {
            // The field indexes norms.
            reader
                .get_norm_values(&self.field)?
                .map(|v| v as Box<dyn DocIdSetIterator>)
        } else if field_info.get_vector_dimension() != 0 {
            // The field indexes vectors.
            match field_info.get_vector_encoding() {
                VectorEncoding::FLOAT32 => match reader.get_float_vector_values(&self.field)? {
                    None => None,
                    Some(values) => Some(values.iterator()? as Box<dyn DocIdSetIterator>),
                },
                VectorEncoding::BYTE => match reader.get_byte_vector_values(&self.field)? {
                    None => None,
                    Some(values) => Some(values.iterator()? as Box<dyn DocIdSetIterator>),
                },
            }
        } else if field_info.get_doc_values_type() != DocValuesType::NONE {
            // The field indexes doc values.
            FieldExistsQuery::get_doc_values_doc_id_set_iterator(&self.field, reader.as_ref())?
        } else {
            return Err(LuceneError::illegal_state(
                FieldExistsQuery::build_error_msg(&field_info.name),
            ));
        };

        let Some(iterator) = iterator else {
            return Ok(None);
        };
        Ok(Some(Box::new(ConstantScoreScorerSupplier::from_iterator(
            ScorerIterator::Simple(iterator),
            score,
            self.score_mode,
            reader.max_doc(),
        ))))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let reader = context.leaf_reader();
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(&self.field) else {
            return Ok(0);
        };
        if field_info.has_norms() {
            // The field indexes norms; if every doc has a value we can take a
            // shortcut.
            if context.reader().get_doc_count(&self.field)? == reader.max_doc() {
                return Ok(reader.num_docs());
            }
            return Ok(-1);
        }

        let mut count: i32 = -1;
        if field_info.has_vector_values() {
            // The field indexes vectors.
            count = FieldExistsQuery::vector_values_size(
                field_info.get_vector_encoding(),
                &self.field,
                reader.as_ref(),
            )?;
        } else if field_info.get_doc_values_type() != DocValuesType::NONE {
            // The field indexes doc values.
            if field_info.doc_values_skip_index_type() != DocValuesSkipIndexType::NONE {
                count = reader
                    .get_doc_values_skipper(&self.field)?
                    .map_or(0, |skipper| skipper.global_doc_count());
            } else if !context.reader().has_deletions() {
                // No deletions: points or terms doc counts are a proxy for the
                // doc-values count.
                if field_info.get_point_dimension_count() > 0 {
                    count = match reader.get_point_values(&self.field)? {
                        Some(values) => values.doc_count()?,
                        None => 0,
                    };
                } else if field_info.get_index_options() != IndexOptions::NONE {
                    count = match reader.terms(&self.field)? {
                        Some(terms) => terms.doc_count()?,
                        None => 0,
                    };
                }
            }
        } else {
            return Err(LuceneError::illegal_state(
                FieldExistsQuery::build_error_msg(&field_info.name),
            ));
        }

        if count == 0 {
            // One of the cases above shows the field is not present on this leaf.
            Ok(0)
        } else if count == reader.max_doc() {
            // Every doc in the leaf, live or deleted, has the field; return the
            // count of live docs.
            Ok(reader.num_docs())
        } else if count >= 0 && !context.reader().has_deletions() {
            // No deleted docs, so the computed count can be trusted.
            Ok(count)
        } else {
            // Some docs do not have the field and some docs are deleted, so the
            // intersection has to be scanned for.
            Ok(-1)
        }
    }

    /// Equivalent to the weight's `isCacheable(LeafReaderContext)`, which
    /// delegates to `DocValues.isCacheable(ctx, field)` for a doc-values field.
    ///
    /// **Divergence from Lucene 10.5.0.** `crate::index::DocValues` does not
    /// expose that static, so its body — "not cacheable once the field's doc
    /// values have been updated", that is once its doc-values generation is no
    /// longer `-1` — is inlined here.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        let field_infos = ctx.leaf_reader().get_field_infos();
        if let Some(field_info) = field_infos.field_info(&self.field) {
            if field_info.get_doc_values_type() != DocValuesType::NONE {
                return field_info.doc_values_gen <= -1;
            }
        }
        true
    }
}

impl Query for FieldExistsQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!("FieldExistsQuery [field={}]", self.field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let reader = Arc::clone(searcher.get_index_reader());
        let mut all_readers_rewritable = true;
        for context in Arc::clone(&reader).leaves()? {
            let leaf = context.leaf_reader();
            let field_infos = leaf.get_field_infos();
            let Some(field_info) = field_infos.field_info(&self.field) else {
                all_readers_rewritable = false;
                break;
            };
            if field_info.has_norms() {
                // The field indexes norms.
                if reader.get_doc_count(&self.field)? != reader.max_doc() {
                    all_readers_rewritable = false;
                    break;
                }
            } else if field_info.get_vector_dimension() != 0 {
                // The field indexes vectors.
                let size = FieldExistsQuery::vector_values_size(
                    field_info.get_vector_encoding(),
                    &self.field,
                    leaf.as_ref(),
                )?;
                if size != leaf.max_doc() {
                    all_readers_rewritable = false;
                    break;
                }
            } else if field_info.get_doc_values_type() != DocValuesType::NONE {
                // The field indexes doc values or points. This optimization is
                // possible because a field always uses the same data structures
                // in every document — all or nothing.
                let terms_doc_count = match leaf.terms(&self.field)? {
                    Some(terms) => Some(terms.doc_count()?),
                    None => None,
                };
                let points_doc_count = match leaf.get_point_values(&self.field)? {
                    Some(values) => Some(values.doc_count()?),
                    None => None,
                };
                let skipper_doc_count = leaf
                    .get_doc_values_skipper(&self.field)?
                    .map(|skipper| skipper.global_doc_count());
                let max_doc = leaf.max_doc();
                if terms_doc_count != Some(max_doc)
                    && points_doc_count != Some(max_doc)
                    && skipper_doc_count != Some(max_doc)
                {
                    all_readers_rewritable = false;
                    break;
                }
            } else {
                return Err(LuceneError::illegal_state(
                    FieldExistsQuery::build_error_msg(&field_info.name),
                ));
            }
        }
        if all_readers_rewritable {
            return Ok(Some(Arc::new(MatchAllDocsQuery::instance())));
        }
        Ok(None)
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let query: Arc<dyn Query> = Arc::new(self.clone());
        let inner = FieldExistsWeight {
            field: self.field.clone(),
            score_mode,
        };
        Ok(Arc::new(ConstantScoreWeight::new(query, boost, inner)))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<FieldExistsQuery>() {
            Some(other) => self.field == other.field,
            None => false,
        }
    }

    /// `getClass().getName()` of `org.apache.lucene.search.FieldExistsQuery`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.FieldExistsQuery"
    }

    fn query_hash(&self) -> i32 {
        // `31 * classHash() + field.hashCode()` (`FieldExistsQuery.java`).
        31i32.wrapping_mul(self.class_hash()).wrapping_add(
            crate::util::java_collections::java_string_hash_code(&self.field),
        )
    }
}
