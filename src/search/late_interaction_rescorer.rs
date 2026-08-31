//! Late-interaction rescoring, ported from
//! `org.apache.lucene.search.LateInteractionRescorer`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::VectorSimilarityFunction;
use crate::search::double_values_source::DoubleValuesSource;
use crate::search::double_values_source_rescorer::{
    DoubleValuesSourceRescorer, DoubleValuesSourceRescorerImpl,
};
use crate::search::index_searcher::IndexSearcher;
use crate::search::late_interaction_float_values_source::LateInteractionFloatValuesSource;
use crate::search::rescorer::Rescorer;
use crate::search::similarities::Explanation;
use crate::search::top_docs::TopDocs;

/// What a [`LateInteractionRescorer`] does with a document that has no value in
/// the late-interaction field.
///
/// Equivalent to the difference between
/// `LateInteractionRescorer.combine(..)` — which scores such a document `0f` —
/// and the anonymous subclass that
/// `withFallbackToFirstPassScore` builds, which keeps the first-pass score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LateInteractionFallback {
    /// Assign a score of `0` when the document has no value.
    ///
    /// Equivalent to `LateInteractionRescorer.combine`.
    Zero,
    /// Keep the first-pass score when the document has no value.
    ///
    /// Equivalent to the anonymous subclass of
    /// `LateInteractionRescorer.withFallbackToFirstPassScore`.
    FirstPassScore,
}

impl DoubleValuesSourceRescorerImpl for LateInteractionFallback {
    fn combine(&self, first_pass_score: f32, value_present: bool, source_value: f64) -> f32 {
        if value_present {
            source_value as f32
        } else {
            match self {
                LateInteractionFallback::Zero => 0.0,
                LateInteractionFallback::FirstPassScore => first_pass_score,
            }
        }
    }

    fn combiner_name(&self) -> String {
        match self {
            LateInteractionFallback::Zero => "LateInteractionRescorer".to_string(),
            LateInteractionFallback::FirstPassScore => {
                "LateInteractionRescorer$FirstPassFallback".to_string()
            }
        }
    }
}

/// Rescores the top-N results of a first-pass query using a
/// [`LateInteractionFloatValuesSource`].
///
/// Equivalent to `org.apache.lucene.search.LateInteractionRescorer`.
///
/// Typically a low-cost first-pass query collects results from across the
/// index, and this rescorer reranks the top-N hits using multi-vectors, usually
/// from a late-interaction model. The multi-vectors must be indexed in the
/// `LateInteractionField` the rescorer is given.
#[derive(Debug)]
pub struct LateInteractionRescorer {
    inner: DoubleValuesSourceRescorer<LateInteractionFallback>,
}

impl LateInteractionRescorer {
    /// Wraps a late-interaction value source, scoring a document without a
    /// value `0`.
    ///
    /// Equivalent to
    /// `new LateInteractionRescorer(LateInteractionFloatValuesSource)`.
    pub fn new(values_source: Arc<LateInteractionFloatValuesSource>) -> Self {
        Self::with_fallback(values_source, LateInteractionFallback::Zero)
    }

    /// Wraps a late-interaction value source with an explicit fallback.
    ///
    /// Equivalent to the constructor plus the anonymous subclass of
    /// `withFallbackToFirstPassScore`.
    pub fn with_fallback(
        values_source: Arc<LateInteractionFloatValuesSource>,
        fallback: LateInteractionFallback,
    ) -> Self {
        let values_source: Arc<dyn DoubleValuesSource> = values_source;
        Self {
            inner: DoubleValuesSourceRescorer::new(values_source, fallback),
        }
    }

    /// Creates a rescorer for the given query multi-vector, comparing with
    /// cosine similarity.
    ///
    /// Equivalent to `LateInteractionRescorer.create(String, float[][])`.
    ///
    /// # Errors
    ///
    /// As [`LateInteractionFloatValuesSource::with_similarity`].
    pub fn create(field_name: impl Into<String>, query_vector: Vec<Vec<f32>>) -> Result<Self> {
        Self::create_with_similarity(field_name, query_vector, VectorSimilarityFunction::COSINE)
    }

    /// Creates a rescorer for the given query multi-vector and similarity
    /// function.
    ///
    /// Equivalent to
    /// `LateInteractionRescorer.create(String, float[][], VectorSimilarityFunction)`.
    /// A document with no value in `field_name` is assigned a score of `0`.
    ///
    /// # Errors
    ///
    /// As [`LateInteractionFloatValuesSource::with_similarity`].
    pub fn create_with_similarity(
        field_name: impl Into<String>,
        query_vector: Vec<Vec<f32>>,
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Result<Self> {
        let values_source = LateInteractionFloatValuesSource::with_similarity(
            field_name,
            query_vector,
            vector_similarity_function,
        )?;
        Ok(Self::new(Arc::new(values_source)))
    }

    /// Creates a rescorer that keeps the first-pass score for a document with
    /// no value in `field_name`.
    ///
    /// Equivalent to
    /// `LateInteractionRescorer.withFallbackToFirstPassScore(String, float[][], VectorSimilarityFunction)`.
    ///
    /// # Errors
    ///
    /// As [`LateInteractionFloatValuesSource::with_similarity`].
    pub fn with_fallback_to_first_pass_score(
        field_name: impl Into<String>,
        query_vector: Vec<Vec<f32>>,
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Result<Self> {
        let values_source = LateInteractionFloatValuesSource::with_similarity(
            field_name,
            query_vector,
            vector_similarity_function,
        )?;
        Ok(Self::with_fallback(
            Arc::new(values_source),
            LateInteractionFallback::FirstPassScore,
        ))
    }
}

impl Rescorer for LateInteractionRescorer {
    fn rescore(
        &self,
        searcher: &IndexSearcher,
        first_pass_top_docs: &TopDocs,
        top_n: i32,
    ) -> Result<TopDocs> {
        self.inner.rescore(searcher, first_pass_top_docs, top_n)
    }

    fn explain(
        &self,
        searcher: &IndexSearcher,
        first_pass_explanation: &Explanation,
        doc_id: i32,
    ) -> Result<Explanation> {
        self.inner.explain(searcher, first_pass_explanation, doc_id)
    }
}
