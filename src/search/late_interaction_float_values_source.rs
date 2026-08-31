//! Late-interaction scoring, ported from
//! `org.apache.lucene.search.LateInteractionFloatValuesSource`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::document::shape_field::LateInteractionField;
use crate::error::{LuceneError, Result};
use crate::index::{
    BinaryDocValues, DocValuesIterator, LeafReaderContext, VectorSimilarityFunction,
};
use crate::search::double_values::{DoubleValues, EmptyDoubleValues};
use crate::search::double_values_source::{hash_of, DoubleValuesSource};
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_vector_similarity::MultiVectorSimilarity;
use crate::search::segment_cacheable::SegmentCacheable;

/// `java.lang.Float.MIN_VALUE`: the smallest positive subnormal `float`, not
/// the most negative one.
///
/// `LateInteractionFloatValuesSource.ScoreFunction.SUM_MAX_SIM` seeds its
/// running maximum with it and returns it for an empty document vector, so the
/// exact constant matters.
const JAVA_FLOAT_MIN_VALUE: f32 = 1.4e-45;

/// A [`DoubleValuesSource`] scoring documents by the similarity between a
/// multi-vector query and indexed document multi-vectors.
///
/// Equivalent to
/// `org.apache.lucene.search.LateInteractionFloatValuesSource`. It is useful
/// for re-ranking query results with late-interaction models, where documents
/// and queries are represented as multi-vectors of composing token vectors.
/// Document vectors are indexed with
/// [`LateInteractionField`](crate::document::shape_field::LateInteractionField).
#[derive(Debug, Clone)]
pub struct LateInteractionFloatValuesSource {
    field_name: String,
    query_vector: Vec<Vec<f32>>,
    vector_similarity_function: VectorSimilarityFunction,
    score_function: Arc<dyn MultiVectorSimilarity>,
}

impl LateInteractionFloatValuesSource {
    /// Creates a source scoring with cosine similarity and
    /// [`LateInteractionScoreFunction::SumMaxSim`].
    ///
    /// Equivalent to
    /// `new LateInteractionFloatValuesSource(String, float[][])`.
    ///
    /// # Errors
    ///
    /// As [`with_score_function`](Self::with_score_function).
    pub fn new(field_name: impl Into<String>, query_vector: Vec<Vec<f32>>) -> Result<Self> {
        Self::with_score_function(
            field_name,
            query_vector,
            VectorSimilarityFunction::COSINE,
            Arc::new(LateInteractionScoreFunction::SumMaxSim),
        )
    }

    /// Creates a source scoring with an explicit similarity function and
    /// [`LateInteractionScoreFunction::SumMaxSim`].
    ///
    /// Equivalent to
    /// `new LateInteractionFloatValuesSource(String, float[][], VectorSimilarityFunction)`.
    ///
    /// # Errors
    ///
    /// As [`with_score_function`](Self::with_score_function).
    pub fn with_similarity(
        field_name: impl Into<String>,
        query_vector: Vec<Vec<f32>>,
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Result<Self> {
        Self::with_score_function(
            field_name,
            query_vector,
            vector_similarity_function,
            Arc::new(LateInteractionScoreFunction::SumMaxSim),
        )
    }

    /// Creates a source scoring with an explicit similarity function and
    /// multi-vector score function.
    ///
    /// Equivalent to
    /// `new LateInteractionFloatValuesSource(String, float[][], VectorSimilarityFunction, MultiVectorSimilarity)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's messages — when
    /// the query multi-vector is empty, when a composing token vector is empty,
    /// or when the token vectors do not all have the same length.
    pub fn with_score_function(
        field_name: impl Into<String>,
        query_vector: Vec<Vec<f32>>,
        vector_similarity_function: VectorSimilarityFunction,
        score_function: Arc<dyn MultiVectorSimilarity>,
    ) -> Result<Self> {
        Ok(Self {
            field_name: field_name.into(),
            query_vector: validate_query_vector(query_vector)?,
            vector_similarity_function,
            score_function,
        })
    }

    /// Returns the query multi-vector.
    pub fn query_vector(&self) -> &[Vec<f32>] {
        &self.query_vector
    }
}

/// Equivalent to the private
/// `LateInteractionFloatValuesSource.validateQueryVector(float[][])`.
fn validate_query_vector(query_vector: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>> {
    if query_vector.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "queryVector must not be null or empty".to_string(),
        ));
    }
    if query_vector[0].is_empty() {
        return Err(LuceneError::IllegalArgument(
            "composing token vectors in provided query vector should not be null or empty"
                .to_string(),
        ));
    }
    let dimension = query_vector[0].len();
    for vector in query_vector.iter().skip(1) {
        if vector.len() != dimension {
            return Err(LuceneError::IllegalArgument(
                "all composing token vectors in provided query vector should have the same length"
                    .to_string(),
            ));
        }
    }
    Ok(query_vector)
}

impl SegmentCacheable for LateInteractionFloatValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl DoubleValuesSource for LateInteractionFloatValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let values = ctx.leaf_reader().get_binary_doc_values(&self.field_name)?;
        match values {
            None => Ok(Box::new(EmptyDoubleValues)),
            Some(values) => Ok(Box::new(LateInteractionDoubleValues {
                source: Arc::clone(&self),
                values,
            })),
        }
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
            .downcast_ref::<LateInteractionFloatValuesSource>()
        {
            Some(other) => {
                self.field_name == other.field_name
                    && self.vector_similarity_function == other.vector_similarity_function
                    && self
                        .score_function
                        .similarity_eq(other.score_function.as_ref())
                    && self.query_vector == other.query_vector
            }
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        let bits: Vec<Vec<u32>> = self
            .query_vector
            .iter()
            .map(|vector| vector.iter().map(|v| v.to_bits()).collect())
            .collect();
        hash_of(&(
            &self.field_name,
            bits,
            self.vector_similarity_function as u8,
            self.score_function.similarity_hash(),
        ))
    }

    fn to_source_string(&self) -> String {
        format!(
            "LateInteractionFloatValuesSource(fieldName={} similarityFunction={:?} scoreFunction={:?} queryVector={:?})",
            self.field_name,
            self.vector_similarity_function,
            self.score_function,
            self.query_vector
        )
    }
}

/// The values a [`LateInteractionFloatValuesSource`] hands out.
///
/// Equivalent to the anonymous `DoubleValues` of
/// `LateInteractionFloatValuesSource.getValues`.
struct LateInteractionDoubleValues {
    source: Arc<LateInteractionFloatValuesSource>,
    values: Box<dyn BinaryDocValues>,
}

impl DoubleValues for LateInteractionDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        let payload = self.values.binary_value()?;
        let doc_vector = LateInteractionField::decode(&payload)?;
        Ok(self.source.score_function.compare(
            &self.source.query_vector,
            &doc_vector,
            self.source.vector_similarity_function,
        )? as f64)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.values.advance_exact(doc)
    }
}

/// The functions computing a similarity score between a query multi-vector and
/// a document multi-vector.
///
/// Equivalent to the nested enum
/// `org.apache.lucene.search.LateInteractionFloatValuesSource.ScoreFunction`,
/// which implements [`MultiVectorSimilarity`]. It is named
/// `LateInteractionScoreFunction` here because Rust has no nested types and
/// `ScoreFunction` alone would not say what it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LateInteractionScoreFunction {
    /// Computes the sum of the maximum similarity between each query token
    /// vector and the document token vectors.
    ///
    /// Equivalent to `ScoreFunction.SUM_MAX_SIM`.
    SumMaxSim,
}

impl MultiVectorSimilarity for LateInteractionScoreFunction {
    fn compare(
        &self,
        query_vector: &[Vec<f32>],
        doc_vector: &[Vec<f32>],
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Result<f32> {
        match self {
            LateInteractionScoreFunction::SumMaxSim => {
                if doc_vector.is_empty() {
                    return Ok(JAVA_FLOAT_MIN_VALUE);
                }
                let mut result = 0f32;
                for q in query_vector {
                    let mut max_sim = JAVA_FLOAT_MIN_VALUE;
                    for d in doc_vector {
                        if q.len() != d.len() {
                            return Err(LuceneError::IllegalArgument(format!(
                                "Provided multi-vectors are incompatible. Their composing token \
                                 vectors should have the same dimension, got {} != {}",
                                q.len(),
                                d.len()
                            )));
                        }
                        max_sim =
                            java_max_f32(max_sim, vector_similarity_function.compare_f32(q, d)?);
                    }
                    result += max_sim;
                }
                Ok(result)
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn similarity_eq(&self, other: &dyn MultiVectorSimilarity) -> bool {
        match other
            .as_any()
            .downcast_ref::<LateInteractionScoreFunction>()
        {
            Some(other) => self == other,
            None => false,
        }
    }

    fn similarity_hash(&self) -> u64 {
        hash_of(self)
    }
}

/// `java.lang.Float.max(float, float)`, which propagates `NaN` where
/// [`f32::max`] would discard it.
fn java_max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a > b {
        a
    } else if a < b {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() {
            a
        } else {
            b
        }
    } else {
        a
    }
}
