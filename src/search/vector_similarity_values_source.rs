//! Vector similarity as a value source, ported from
//! `org.apache.lucene.search.VectorSimilarityValuesSource`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::double_values::{DoubleValues, EmptyDoubleValues};
use crate::search::double_values_source::DoubleValuesSource;
use crate::search::vector_scorer::{SharedVectorScorer, SharedVectorScorerIterator, VectorScorer};
use crate::search::DocIdSetIterator;

/// A [`DoubleValuesSource`] providing the vector similarity scores between a
/// query vector and the vector field of each document.
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.search.VectorSimilarityValuesSource`, which extends
/// `DoubleValuesSource`.
///
/// **Divergence from Lucene 10.5.0.** Rust has no implementation inheritance,
/// so the abstract class becomes this trait — carrying the one abstract method,
/// `getScorer` — plus the free function [`vector_similarity_values`], which is
/// the shared `getValues` body. The two concrete subclasses implement both this
/// trait and [`DoubleValuesSource`], forwarding `get_values` to that function
/// and answering `needs_scores`, `rewrite` and `is_cacheable` exactly as the
/// base class does.
pub trait VectorSimilarityValuesSource: DoubleValuesSource {
    /// Returns the field this source scores.
    ///
    /// Equivalent to reading the `protected final String fieldName` field.
    fn field_name(&self) -> &str;

    /// Returns the vector scorer for a leaf, or `None` when the field holds no
    /// vectors of the expected kind.
    ///
    /// Equivalent to the abstract
    /// `VectorSimilarityValuesSource.getScorer(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the vector values.
    fn get_scorer(&self, ctx: &LeafReaderContext) -> Result<Option<Box<dyn VectorScorer>>>;
}

/// The shared body of `VectorSimilarityValuesSource.getValues`.
///
/// Equivalent to the base class's `getValues(LeafReaderContext, DoubleValues)`,
/// which returns `DoubleValues.EMPTY` when there is no scorer and otherwise
/// exposes the scorer's score at the current position of its iterator.
pub fn vector_similarity_values(scorer: Option<Box<dyn VectorScorer>>) -> Box<dyn DoubleValues> {
    match scorer {
        None => Box::new(EmptyDoubleValues),
        Some(scorer) => {
            let scorer = SharedVectorScorer::new(scorer);
            let iterator = scorer.iterator();
            Box::new(VectorSimilarityDoubleValues { scorer, iterator })
        }
    }
}

/// The values a [`VectorSimilarityValuesSource`] hands out.
///
/// Equivalent to the anonymous `DoubleValues` of
/// `VectorSimilarityValuesSource.getValues`.
struct VectorSimilarityDoubleValues {
    scorer: SharedVectorScorer,
    iterator: SharedVectorScorerIterator,
}

impl DoubleValues for VectorSimilarityDoubleValues {
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
