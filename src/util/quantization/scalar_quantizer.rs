//! `ScalarQuantizer` ported from `org.apache.lucene.util.quantization`.

use crate::error::{LuceneError, Result};
use crate::index::VectorSimilarityFunction;
use crate::util::vector_util;

/// How many vectors are sampled when quantiles are computed from a segment.
///
/// Equivalent to `ScalarQuantizer.SCALAR_QUANTIZATION_SAMPLE_SIZE`.
pub const SCALAR_QUANTIZATION_SAMPLE_SIZE: usize = 25_000;

/// Guards against extreme confidence intervals and huge allocations.
///
/// Equivalent to `ScalarQuantizer.SCRATCH_SIZE`.
pub const SCRATCH_SIZE: usize = 20;

/// Maps a float vector onto `bits`-wide integers over a fixed quantile range.
///
/// Equivalent to `org.apache.lucene.util.quantization.ScalarQuantizer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScalarQuantizer {
    /// Width of one quantization step.
    alpha: f32,
    /// Reciprocal of `alpha`, applied when quantizing.
    scale: f32,
    bits: u8,
    min_quantile: f32,
    max_quantile: f32,
}

impl ScalarQuantizer {
    /// Creates a quantizer over `[min_quantile, max_quantile]` at `bits` bits.
    ///
    /// Fails on a non-finite quantile, as Java's `IllegalStateException` does.
    pub fn new(min_quantile: f32, max_quantile: f32, bits: u8) -> Result<Self> {
        if !min_quantile.is_finite() || !max_quantile.is_finite() {
            return Err(LuceneError::IllegalState(
                "Scalar quantizer does not support infinite or NaN values".to_string(),
            ));
        }
        if max_quantile < min_quantile {
            return Err(LuceneError::IllegalArgument(format!(
                "maxQuantile {max_quantile} must be >= minQuantile {min_quantile}"
            )));
        }
        if bits == 0 || bits > 8 {
            return Err(LuceneError::IllegalArgument(format!(
                "bits must be in [1, 8], got {bits}"
            )));
        }
        let divisor = ((1u32 << bits) - 1) as f32;
        Ok(Self {
            alpha: (max_quantile - min_quantile) / divisor,
            scale: divisor / (max_quantile - min_quantile),
            bits,
            min_quantile,
            max_quantile,
        })
    }

    /// Quantizes `src` into `dest`, returning the corrective offset the scorer
    /// needs, or `0` for Euclidean similarity which needs none.
    ///
    /// Equivalent to `ScalarQuantizer.quantize`.
    pub fn quantize(
        &self,
        src: &[f32],
        dest: &mut [u8],
        similarity_function: VectorSimilarityFunction,
    ) -> Result<f32> {
        if src.len() != dest.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "vector lengths differ: {} vs {}",
                src.len(),
                dest.len()
            )));
        }
        let correction = self.min_max_scalar_quantize(src, dest);
        if similarity_function == VectorSimilarityFunction::EUCLIDEAN {
            return Ok(0.0);
        }
        Ok(correction)
    }

    /// Clamps each component into the quantile range, rounds it onto the
    /// integer grid, and accumulates the correction the dot-product scorers
    /// apply.
    ///
    /// Equivalent to `VectorUtil.minMaxScalarQuantize`, which Lucene keeps in
    /// `VectorUtil` so it can be vectorised; this port keeps it beside its only
    /// caller until the crate has a vectorised counterpart.
    fn min_max_scalar_quantize(&self, src: &[f32], dest: &mut [u8]) -> f32 {
        let mut correction = 0.0f32;
        for (i, &v) in src.iter().enumerate() {
            let clamped = v.clamp(self.min_quantile, self.max_quantile);
            let quantized = ((clamped - self.min_quantile) * self.scale + 0.5) as u8;
            dest[i] = quantized;
            // The residual is what rounding threw away; the scorer adds it back.
            let dequantized = self.alpha * f32::from(quantized) + self.min_quantile;
            correction += (v - dequantized) * dequantized;
        }
        correction
    }

    /// Recomputes the corrective offset of an already quantized vector against
    /// a different quantizer.
    ///
    /// Equivalent to `ScalarQuantizer.recalculateCorrectiveOffset`.
    pub fn recalculate_corrective_offset(
        &self,
        quantized_vector: &[u8],
        old_quantizer: &ScalarQuantizer,
        similarity_function: VectorSimilarityFunction,
    ) -> f32 {
        if similarity_function == VectorSimilarityFunction::EUCLIDEAN {
            return 0.0;
        }
        let mut correction = 0.0f32;
        for &q in quantized_vector {
            // Undo the old quantizer, then measure what this one would lose.
            let original = old_quantizer.alpha * f32::from(q) + old_quantizer.min_quantile;
            let clamped = original.clamp(self.min_quantile, self.max_quantile);
            let requantized = ((clamped - self.min_quantile) * self.scale + 0.5) as u8;
            let dequantized = self.alpha * f32::from(requantized) + self.min_quantile;
            correction += (original - dequantized) * dequantized;
        }
        correction
    }

    /// Turns a quantized vector back into floats.
    ///
    /// Equivalent to `ScalarQuantizer.deQuantize`.
    pub fn de_quantize(&self, src: &[u8], dest: &mut [f32]) -> Result<()> {
        if src.len() != dest.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "vector lengths differ: {} vs {}",
                src.len(),
                dest.len()
            )));
        }
        for (i, &q) in src.iter().enumerate() {
            dest[i] = self.alpha * f32::from(q) + self.min_quantile;
        }
        Ok(())
    }

    /// Returns the lower quantile.
    pub fn get_lower_quantile(&self) -> f32 {
        self.min_quantile
    }

    /// Returns the upper quantile.
    pub fn get_upper_quantile(&self) -> f32 {
        self.max_quantile
    }

    /// Returns the multiplier a quantized dot product is scaled by.
    ///
    /// Equivalent to `ScalarQuantizer.getConstantMultiplier()`.
    pub fn get_constant_multiplier(&self) -> f32 {
        self.alpha * self.alpha
    }

    /// Returns how many bits each component occupies.
    pub fn get_bits(&self) -> u8 {
        self.bits
    }

    /// Computes quantiles from a sample of vectors at the given confidence
    /// interval.
    ///
    /// Equivalent to `ScalarQuantizer.fromVectors`, reduced to the quantile
    /// computation itself: Java's version also samples the vector values it is
    /// handed, which needs the `QuantizedByteVectorValues` iteration this port
    /// leaves to the caller.
    pub fn from_sample(
        sample: &[f32],
        confidence_interval: f32,
        bits: u8,
        dimension: usize,
    ) -> Result<Self> {
        if sample.is_empty() || dimension == 0 {
            return Self::new(0.0, 0.0, bits);
        }
        if !(0.9..=1.0).contains(&confidence_interval) {
            return Err(LuceneError::IllegalArgument(format!(
                "confidenceInterval must be in [0.9, 1.0], got {confidence_interval}"
            )));
        }
        let mut sorted: Vec<f32> = sample.to_vec();
        sorted.sort_by(f32::total_cmp);

        if confidence_interval == 1.0 {
            return Self::new(sorted[0], sorted[sorted.len() - 1], bits);
        }
        // Trim the same fraction off each tail, as Java's quantile selection does.
        let selector = ((1.0 - confidence_interval) * sorted.len() as f32) as usize;
        let lower = sorted[selector.min(sorted.len() - 1)];
        let upper = sorted[sorted.len() - 1 - selector.min(sorted.len() - 1)];
        Self::new(lower, upper, bits)
    }
}

impl std::fmt::Display for ScalarQuantizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ScalarQuantizer{{minQuantile={}, maxQuantile={}, bits={}}}",
            self.min_quantile, self.max_quantile, self.bits
        )
    }
}

/// Returns whether `v` is a unit vector, which cosine quantization requires.
pub fn is_unit_vector(v: &[f32]) -> bool {
    vector_util::is_unit_vector(v)
}
