//! Full-precision vector similarity as a value source, ported from
//! `org.apache.lucene.search.FullPrecisionFloatVectorSimilarityValuesSource`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    check_float_field, DocIndexIterator, FloatVectorValues, LeafReaderContext,
    VectorSimilarityFunction,
};
use crate::search::double_values::{DoubleValues, EmptyDoubleValues};
use crate::search::double_values_source::{hash_of, DoubleValuesSource};
use crate::search::index_searcher::IndexSearcher;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::vector_scorer::{SharedVectorScorer, SharedVectorScorerIterator};
use crate::search::DocIdSetIterator;

/// A [`DoubleValuesSource`] computing the vector similarity between a query
/// vector and the raw, full-precision vectors of a `KnnFloatVectorField`.
///
/// Equivalent to
/// `org.apache.lucene.search.FullPrecisionFloatVectorSimilarityValuesSource`.
#[derive(Debug, Clone)]
pub struct FullPrecisionFloatVectorSimilarityValuesSource {
    query_vector: Vec<f32>,
    field_name: String,
    vector_similarity_function: Option<VectorSimilarityFunction>,
}

impl FullPrecisionFloatVectorSimilarityValuesSource {
    /// Creates a source scoring with an explicit similarity function.
    ///
    /// Equivalent to
    /// `new FullPrecisionFloatVectorSimilarityValuesSource(float[], String, VectorSimilarityFunction)`.
    pub fn new(
        vector: Vec<f32>,
        field_name: impl Into<String>,
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Self {
        Self {
            query_vector: vector,
            field_name: field_name.into(),
            vector_similarity_function: Some(vector_similarity_function),
        }
    }

    /// Creates a source scoring with the similarity function configured for the
    /// field.
    ///
    /// Equivalent to
    /// `new FullPrecisionFloatVectorSimilarityValuesSource(float[], String)`,
    /// which passes a `null` function.
    pub fn with_field_similarity(vector: Vec<f32>, field_name: impl Into<String>) -> Self {
        Self {
            query_vector: vector,
            field_name: field_name.into(),
            vector_similarity_function: None,
        }
    }

    /// Fetches the full-precision similarity scores of a leaf.
    ///
    /// Equivalent to
    /// `FullPrecisionFloatVectorSimilarityValuesSource.getSimilarityScores(LeafReaderContext)`,
    /// which is `getValues(ctx, null)`.
    ///
    /// # Errors
    ///
    /// As [`DoubleValuesSource::get_values`].
    pub fn get_similarity_scores(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
    ) -> Result<Box<dyn DoubleValues>> {
        self.get_values(ctx, None)
    }
}

impl SegmentCacheable for FullPrecisionFloatVectorSimilarityValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl DoubleValuesSource for FullPrecisionFloatVectorSimilarityValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let reader = ctx.leaf_reader();
        let Some(vector_values) = reader.get_float_vector_values(&self.field_name)? else {
            check_float_field(reader.as_ref(), &self.field_name)?;
            return Ok(Box::new(EmptyDoubleValues));
        };
        let infos = reader.get_field_infos();
        let dimension = infos
            .field_info(&self.field_name)
            .map(|fi| fi.vector_dimension)
            .unwrap_or(0);
        if dimension as usize != self.query_vector.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "Query vector dimension does not match field dimension: {} != {}",
                self.query_vector.len(),
                dimension
            )));
        }

        let Some(function) = self.vector_similarity_function else {
            let Some(scorer) = vector_values.rescorer(&self.query_vector)? else {
                return Ok(Box::new(EmptyDoubleValues));
            };
            let scorer = SharedVectorScorer::new(scorer);
            let iterator = scorer.iterator();
            return Ok(Box::new(RescorerDoubleValues { scorer, iterator }));
        };

        let iterator = vector_values.iterator()?;
        Ok(Box::new(FullPrecisionDoubleValues {
            query_vector: self.query_vector.clone(),
            vector_similarity_function: function,
            vector_values,
            iterator,
        }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other
            .as_any()
            .downcast_ref::<FullPrecisionFloatVectorSimilarityValuesSource>()
        {
            Some(other) => {
                self.field_name == other.field_name
                    && self.vector_similarity_function == other.vector_similarity_function
                    && self.query_vector == other.query_vector
            }
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        let bits: Vec<u32> = self.query_vector.iter().map(|v| v.to_bits()).collect();
        hash_of(&(
            &self.field_name,
            bits,
            self.vector_similarity_function.map(|f| f as u8),
        ))
    }

    fn to_source_string(&self) -> String {
        // **Divergence from Lucene 10.5.0.** Java writes
        // `vectorSimilarityFunction.name()` unconditionally, which throws a
        // `NullPointerException` for the field-similarity form; this port
        // renders `null` there instead of failing, which is what Java's own
        // `String.valueOf` contract would have produced.
        let function = match self.vector_similarity_function {
            Some(function) => format!("{function:?}"),
            None => "null".to_string(),
        };
        format!(
            "FullPrecisionFloatVectorSimilarityValuesSource(fieldName={} vectorSimilarityFunction={} queryVector={:?})",
            self.field_name, function, self.query_vector
        )
    }
}

/// The values read through the field's own rescorer.
///
/// Equivalent to the first anonymous `DoubleValues` of
/// `FullPrecisionFloatVectorSimilarityValuesSource.getValues`.
struct RescorerDoubleValues {
    scorer: SharedVectorScorer,
    iterator: SharedVectorScorerIterator,
}

impl DoubleValues for RescorerDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.scorer.score()? as f64)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        if doc < self.iterator.doc_id() {
            return Ok(false);
        }
        if self.iterator.doc_id() == doc {
            return Ok(true);
        }
        Ok(self.iterator.advance(doc)? == doc)
    }
}

/// The values computed against the raw vectors.
///
/// Equivalent to the second anonymous `DoubleValues` of
/// `FullPrecisionFloatVectorSimilarityValuesSource.getValues`.
struct FullPrecisionDoubleValues {
    query_vector: Vec<f32>,
    vector_similarity_function: VectorSimilarityFunction,
    vector_values: Box<dyn FloatVectorValues>,
    iterator: Box<dyn DocIndexIterator>,
}

impl DoubleValues for FullPrecisionDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        let value = self.vector_values.vector_value(self.iterator.index())?;
        Ok(self
            .vector_similarity_function
            .compare_f32(&self.query_vector, &value)? as f64)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        if doc < self.iterator.doc_id() {
            return Ok(false);
        }
        if self.iterator.doc_id() == doc {
            return Ok(true);
        }
        Ok(self.iterator.advance(doc)? == doc)
    }
}
