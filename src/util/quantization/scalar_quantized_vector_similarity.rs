//! `ScalarQuantizedVectorSimilarity` ported from
//! `org.apache.lucene.util.quantization`.

use crate::error::Result;
use crate::index::VectorSimilarityFunction;
use crate::util::hnsw::scorer::java_math_max;
use crate::util::vector_util;

/// Scores two quantized vectors, applying the corrections quantization needs.
///
/// Equivalent to the interface
/// `org.apache.lucene.util.quantization.ScalarQuantizedVectorSimilarity`
/// (`ScalarQuantizedVectorSimilarity.java:28`). Lucene ships three
/// implementations, [`Euclidean`], [`DotProduct`] and [`MaximumInnerProduct`],
/// which [`from_vector_similarity`](<dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity)
/// selects; a user may implement the trait for a similarity of their own.
///
/// `Send + Sync` lets the scorers that hold one be handed across threads, as
/// every scorer in this port can be.
pub trait ScalarQuantizedVectorSimilarity: Send + Sync {
    /// Scores `query_vector` against `stored_vector`.
    ///
    /// Equivalent to `ScalarQuantizedVectorSimilarity.score(byte[], float, byte[], float)`
    /// (`ScalarQuantizedVectorSimilarity.java:55`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the two vectors have different lengths, which is the
    /// `IllegalArgumentException` the `VectorUtil` comparison throws.
    fn score(
        &self,
        query_vector: &[u8],
        query_vector_offset: f32,
        stored_vector: &[u8],
        vector_offset: f32,
    ) -> Result<f32>;
}

impl dyn ScalarQuantizedVectorSimilarity {
    /// Creates the similarity that applies the corrections for `sim`.
    ///
    /// Equivalent to the static
    /// `ScalarQuantizedVectorSimilarity.fromVectorSimilarity(VectorSimilarityFunction, float, byte)`
    /// (`ScalarQuantizedVectorSimilarity.java:39-53`): a `bits` of four or
    /// fewer compares with `VectorUtil::int4DotProduct`, anything wider with
    /// `VectorUtil::uint8DotProduct`.
    pub fn from_vector_similarity(
        sim: VectorSimilarityFunction,
        const_multiplier: f32,
        bits: u8,
    ) -> Box<dyn ScalarQuantizedVectorSimilarity> {
        let comparator = || -> Box<dyn ByteVectorComparator> {
            if bits <= 4 {
                Box::new(vector_util::int4_dot_product)
            } else {
                Box::new(vector_util::uint8_dot_product)
            }
        };
        match sim {
            VectorSimilarityFunction::EUCLIDEAN => Box::new(Euclidean::new(const_multiplier)),
            VectorSimilarityFunction::COSINE | VectorSimilarityFunction::DOT_PRODUCT => {
                Box::new(DotProduct::new(const_multiplier, comparator()))
            }
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
                Box::new(MaximumInnerProduct::new(const_multiplier, comparator()))
            }
        }
    }
}

/// Compares two byte vectors.
///
/// Equivalent to the functional interface
/// `ScalarQuantizedVectorSimilarity.ByteVectorComparator`
/// (`ScalarQuantizedVectorSimilarity.java:116-118`). Every
/// `Fn(&[u8], &[u8]) -> Result<i32>` is one, as every matching method reference
/// is in Java, so `vector_util::uint8_dot_product` serves directly.
pub trait ByteVectorComparator: Send + Sync {
    /// Compares `v1` with `v2`.
    ///
    /// Equivalent to `ByteVectorComparator.compare(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the comparison raises.
    fn compare(&self, v1: &[u8], v2: &[u8]) -> Result<i32>;
}

impl<F> ByteVectorComparator for F
where
    F: Fn(&[u8], &[u8]) -> Result<i32> + Send + Sync,
{
    fn compare(&self, v1: &[u8], v2: &[u8]) -> Result<i32> {
        self(v1, v2)
    }
}

/// Calculates euclidean distance on quantized vectors, applying the
/// appropriate corrections.
///
/// Equivalent to `ScalarQuantizedVectorSimilarity.Euclidean`
/// (`ScalarQuantizedVectorSimilarity.java:58-72`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Euclidean {
    const_multiplier: f32,
}

impl Euclidean {
    /// Equivalent to `Euclidean(float constMultiplier)`.
    pub fn new(const_multiplier: f32) -> Self {
        Self { const_multiplier }
    }
}

impl ScalarQuantizedVectorSimilarity for Euclidean {
    fn score(
        &self,
        query_vector: &[u8],
        _query_vector_offset: f32,
        stored_vector: &[u8],
        _vector_offset: f32,
    ) -> Result<f32> {
        let square_distance = vector_util::uint8_square_distance(stored_vector, query_vector)?;
        let adjusted_distance = square_distance as f32 * self.const_multiplier;
        Ok(1.0 / (1.0 + adjusted_distance))
    }
}

/// Calculates dot product on quantized vectors, applying the appropriate
/// corrections.
///
/// Equivalent to `ScalarQuantizedVectorSimilarity.DotProduct`
/// (`ScalarQuantizedVectorSimilarity.java:75-93`).
pub struct DotProduct {
    const_multiplier: f32,
    comparator: Box<dyn ByteVectorComparator>,
}

impl DotProduct {
    /// Equivalent to `DotProduct(float constMultiplier, ByteVectorComparator comparator)`.
    pub fn new(const_multiplier: f32, comparator: Box<dyn ByteVectorComparator>) -> Self {
        Self {
            const_multiplier,
            comparator,
        }
    }
}

impl std::fmt::Debug for DotProduct {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DotProduct")
            .field("const_multiplier", &self.const_multiplier)
            .finish_non_exhaustive()
    }
}

impl ScalarQuantizedVectorSimilarity for DotProduct {
    fn score(
        &self,
        query_vector: &[u8],
        query_offset: f32,
        stored_vector: &[u8],
        vector_offset: f32,
    ) -> Result<f32> {
        let dot_product = self.comparator.compare(stored_vector, query_vector)?;
        // For the current implementation of scalar quantization, all dot
        // products should be >= 0 (`ScalarQuantizedVectorSimilarity.java:88`).
        debug_assert!(dot_product >= 0);
        let adjusted_distance =
            dot_product as f32 * self.const_multiplier + query_offset + vector_offset;
        // `Math.max(float, int)` widens the `0` to `0f`.
        Ok(java_math_max((1.0 + adjusted_distance) / 2.0, 0.0))
    }
}

/// Calculates max inner product on quantized vectors, applying the
/// appropriate corrections.
///
/// Equivalent to `ScalarQuantizedVectorSimilarity.MaximumInnerProduct`
/// (`ScalarQuantizedVectorSimilarity.java:96-113`).
pub struct MaximumInnerProduct {
    const_multiplier: f32,
    comparator: Box<dyn ByteVectorComparator>,
}

impl MaximumInnerProduct {
    /// Equivalent to
    /// `MaximumInnerProduct(float constMultiplier, ByteVectorComparator comparator)`.
    pub fn new(const_multiplier: f32, comparator: Box<dyn ByteVectorComparator>) -> Self {
        Self {
            const_multiplier,
            comparator,
        }
    }
}

impl std::fmt::Debug for MaximumInnerProduct {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaximumInnerProduct")
            .field("const_multiplier", &self.const_multiplier)
            .finish_non_exhaustive()
    }
}

impl ScalarQuantizedVectorSimilarity for MaximumInnerProduct {
    fn score(
        &self,
        query_vector: &[u8],
        query_offset: f32,
        stored_vector: &[u8],
        vector_offset: f32,
    ) -> Result<f32> {
        let dot_product = self.comparator.compare(stored_vector, query_vector)?;
        // See `DotProduct::score` (`ScalarQuantizedVectorSimilarity.java:109`).
        debug_assert!(dot_product >= 0);
        let adjusted_distance =
            dot_product as f32 * self.const_multiplier + query_offset + vector_offset;
        Ok(vector_util::scale_max_inner_product_score(
            adjusted_distance,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Java compares at eight bits with `VectorUtil.uint8DotProduct` and
    /// `uint8SquareDistance` (`ScalarQuantizedVectorSimilarity.java:44-51,67`),
    /// which read each byte as unsigned. A component above 127 tells the two
    /// readings apart: signed, `0xC8` is -56.
    #[test]
    fn eight_bit_scores_read_bytes_as_unsigned() {
        let query = [200u8, 1];
        let stored = [100u8, 3];

        let dot = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            VectorSimilarityFunction::DOT_PRODUCT,
            0.5,
            8,
        );
        // 200*100 + 1*3 = 20003; (1 + 20003*0.5 + 0.25 + 0.25) / 2.
        let expected = (1.0f32 + (20003.0f32 * 0.5 + 0.25 + 0.25)) / 2.0;
        assert_eq!(dot.score(&query, 0.25, &stored, 0.25).unwrap(), expected);

        let euclidean = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            VectorSimilarityFunction::EUCLIDEAN,
            0.5,
            8,
        );
        // (100-200)^2 + (3-1)^2 = 10004.
        let expected = 1.0f32 / (1.0 + 10004.0f32 * 0.5);
        assert_eq!(
            euclidean.score(&query, 0.0, &stored, 0.0).unwrap(),
            expected
        );
    }

    /// At four bits or fewer Java compares with `VectorUtil.int4DotProduct`
    /// over one value in `[0, 15]` per byte.
    #[test]
    fn four_bit_scores_use_the_int4_dot_product() {
        let query = [15u8, 2, 7];
        let stored = [3u8, 15, 1];
        let mip = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
            0.25,
            4,
        );
        let dot = vector_util::int4_dot_product(&stored, &query).unwrap();
        assert_eq!(dot, 45 + 30 + 7);
        let expected = vector_util::scale_max_inner_product_score(dot as f32 * 0.25 + 1.0);
        assert_eq!(mip.score(&query, 0.5, &stored, 0.5).unwrap(), expected);
    }

    /// The trait is open, as Java's interface is: a user similarity, and a
    /// user comparator handed to a Lucene implementation, both score.
    #[test]
    fn a_user_similarity_and_comparator_score() {
        struct Constant;
        impl ScalarQuantizedVectorSimilarity for Constant {
            fn score(&self, _: &[u8], _: f32, _: &[u8], _: f32) -> Result<f32> {
                Ok(0.75)
            }
        }
        let user: Box<dyn ScalarQuantizedVectorSimilarity> = Box::new(Constant);
        assert_eq!(user.score(&[1], 0.0, &[2], 0.0).unwrap(), 0.75);

        let always_four = |_: &[u8], _: &[u8]| -> Result<i32> { Ok(4) };
        let dot = DotProduct::new(1.0, Box::new(always_four));
        assert_eq!(dot.score(&[0], 0.0, &[0], 0.0).unwrap(), 2.5);
    }
}
