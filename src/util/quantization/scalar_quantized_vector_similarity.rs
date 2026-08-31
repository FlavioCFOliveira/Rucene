//! `ScalarQuantizedVectorSimilarity` ported from
//! `org.apache.lucene.util.quantization`.

use crate::error::Result;
use crate::index::VectorSimilarityFunction;
use crate::util::vector_util;

/// Scores two quantized vectors, applying the corrections quantization needs.
///
/// Equivalent to
/// `org.apache.lucene.util.quantization.ScalarQuantizedVectorSimilarity`.
///
/// **Divergence from Lucene 10.5.0.** Java models this as an interface with
/// three implementing classes plus a `ByteVectorComparator` functional
/// interface, chosen by a switch over the similarity. The three differ only in
/// the final arithmetic, and the comparator only in whether it treats a byte as
/// four bits or eight, so this port is one enum carrying both choices. The
/// scores are identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarQuantizedVectorSimilarity {
    /// Squared distance, turned into a similarity.
    Euclidean {
        /// Whether components are four bits wide rather than eight.
        four_bit: bool,
    },
    /// Dot product, mapped into `[0, 1]`.
    DotProduct {
        /// Whether components are four bits wide rather than eight.
        four_bit: bool,
    },
    /// Maximum inner product, scaled by Lucene's unbounded-score mapping.
    MaximumInnerProduct {
        /// Whether components are four bits wide rather than eight.
        four_bit: bool,
    },
}

impl ScalarQuantizedVectorSimilarity {
    /// Selects the scorer for a similarity function and bit width.
    ///
    /// Equivalent to
    /// `ScalarQuantizedVectorSimilarity.fromVectorSimilarity(VectorSimilarityFunction, float, byte)`,
    /// whose `constMultiplier` this port takes at scoring time instead of
    /// binding it into the object.
    pub fn from_vector_similarity(sim: VectorSimilarityFunction, bits: u8) -> Self {
        let four_bit = bits <= 4;
        match sim {
            VectorSimilarityFunction::EUCLIDEAN => Self::Euclidean { four_bit },
            VectorSimilarityFunction::COSINE | VectorSimilarityFunction::DOT_PRODUCT => {
                Self::DotProduct { four_bit }
            }
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
                Self::MaximumInnerProduct { four_bit }
            }
        }
    }

    /// Returns the dot product of two quantized vectors at this bit width.
    fn dot_product(self, a: &[u8], b: &[u8]) -> Result<i32> {
        let four_bit = match self {
            Self::Euclidean { four_bit }
            | Self::DotProduct { four_bit }
            | Self::MaximumInnerProduct { four_bit } => four_bit,
        };
        if four_bit {
            // At four bits the two halves of each byte are separate components.
            let mut total = 0i32;
            for (&x, &y) in a.iter().zip(b.iter()) {
                total += i32::from(x & 0x0F) * i32::from(y & 0x0F);
                total += i32::from(x >> 4) * i32::from(y >> 4);
            }
            Ok(total)
        } else {
            vector_util::dot_product_bytes(a, b)
        }
    }

    /// Scores `query_vector` against `stored_vector`.
    ///
    /// Equivalent to `ScalarQuantizedVectorSimilarity.score`.
    pub fn score(
        self,
        query_vector: &[u8],
        query_offset: f32,
        stored_vector: &[u8],
        vector_offset: f32,
        const_multiplier: f32,
    ) -> Result<f32> {
        match self {
            Self::Euclidean { .. } => {
                let square_distance =
                    vector_util::square_distance_bytes(stored_vector, query_vector)?;
                let adjusted = square_distance as f32 * const_multiplier;
                Ok(1.0 / (1.0 + adjusted))
            }
            Self::DotProduct { .. } => {
                let dot = self.dot_product(stored_vector, query_vector)?;
                let adjusted = dot as f32 * const_multiplier + query_offset + vector_offset;
                Ok(((1.0 + adjusted) / 2.0).max(0.0))
            }
            Self::MaximumInnerProduct { .. } => {
                let dot = self.dot_product(stored_vector, query_vector)?;
                let adjusted = dot as f32 * const_multiplier + query_offset + vector_offset;
                Ok(vector_util::scale_max_inner_product_score(adjusted))
            }
        }
    }
}
