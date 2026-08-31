//! Float vector similarity as a value source, ported from
//! `org.apache.lucene.search.FloatVectorSimilarityValuesSource`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{check_float_field, LeafReaderContext};
use crate::search::double_values::DoubleValues;
use crate::search::double_values_source::{hash_of, DoubleValuesSource};
use crate::search::index_searcher::IndexSearcher;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::vector_scorer::VectorScorer;
use crate::search::vector_similarity_values_source::{
    vector_similarity_values, VectorSimilarityValuesSource,
};

/// A [`DoubleValuesSource`] computing the vector similarity between a query
/// vector and a `KnnFloatVectorField`.
///
/// Equivalent to the package-private
/// `org.apache.lucene.search.FloatVectorSimilarityValuesSource`.
#[derive(Debug, Clone)]
pub struct FloatVectorSimilarityValuesSource {
    query_vector: Vec<f32>,
    field_name: String,
}

impl FloatVectorSimilarityValuesSource {
    /// Creates a source over `field_name` for the given query vector.
    ///
    /// Equivalent to
    /// `new FloatVectorSimilarityValuesSource(float[], String)`.
    pub fn new(vector: Vec<f32>, field_name: impl Into<String>) -> Self {
        Self {
            query_vector: vector,
            field_name: field_name.into(),
        }
    }
}

impl SegmentCacheable for FloatVectorSimilarityValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl VectorSimilarityValuesSource for FloatVectorSimilarityValuesSource {
    fn field_name(&self) -> &str {
        &self.field_name
    }

    fn get_scorer(&self, ctx: &LeafReaderContext) -> Result<Option<Box<dyn VectorScorer>>> {
        let reader = ctx.leaf_reader();
        let vector_values = reader.get_float_vector_values(&self.field_name)?;
        match vector_values {
            None => {
                check_float_field(reader.as_ref(), &self.field_name)?;
                Ok(None)
            }
            Some(values) => values.scorer(&self.query_vector),
        }
    }
}

impl DoubleValuesSource for FloatVectorSimilarityValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Ok(vector_similarity_values(self.get_scorer(ctx)?))
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
            .downcast_ref::<FloatVectorSimilarityValuesSource>()
        {
            Some(other) => {
                self.field_name == other.field_name && self.query_vector == other.query_vector
            }
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        let bits: Vec<u32> = self.query_vector.iter().map(|v| v.to_bits()).collect();
        hash_of(&(&self.field_name, bits))
    }

    fn to_source_string(&self) -> String {
        format!(
            "FloatVectorSimilarityValuesSource(fieldName={} queryVector={:?})",
            self.field_name, self.query_vector
        )
    }
}
