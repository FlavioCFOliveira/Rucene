//! `OptimizedScalarQuantizer` ported from `org.apache.lucene.util.quantization`.
//!
//! Picks the quantization interval per vector by coordinate descent, rather than
//! using one interval for the whole field, which keeps far more of the ranking
//! at the same bit width.

use crate::error::{LuceneError, Result};
use crate::index::VectorSimilarityFunction;

/// Intervals that minimise mean squared error for a unit-variance Gaussian, one
/// row per bit width from one to eight.
///
/// Equivalent to `OptimizedScalarQuantizer.MINIMUM_MSE_GRID`.
pub const MINIMUM_MSE_GRID: [[f32; 2]; 8] = [
    [-0.798, 0.798],
    [-1.493, 1.493],
    [-2.051, 2.051],
    [-2.514, 2.514],
    [-2.916, 2.916],
    [-3.278, 3.278],
    [-3.611, 3.611],
    [-3.922, 3.922],
];

/// Default weight given to the squared-error term against the projection term.
pub const DEFAULT_LAMBDA: f32 = 0.1;
/// Default number of coordinate-descent iterations.
pub const DEFAULT_ITERS: i32 = 5;

/// What quantizing one vector produced.
///
/// Equivalent to `OptimizedScalarQuantizer.QuantizationResult`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuantizationResult {
    /// Lower end of the chosen interval.
    pub lower_interval: f32,
    /// Upper end of the chosen interval.
    pub upper_interval: f32,
    /// The squared norm for Euclidean, or the centroid dot product otherwise.
    pub additional_correction: f32,
    /// Sum of the quantized components, which the scorer needs.
    pub quantized_component_sum: i32,
}

/// Quantizes a vector against a centroid, choosing the interval per vector.
///
/// Equivalent to
/// `org.apache.lucene.util.quantization.OptimizedScalarQuantizer`.
#[derive(Clone, Copy, Debug)]
pub struct OptimizedScalarQuantizer {
    similarity_function: VectorSimilarityFunction,
    lambda: f32,
    iters: i32,
}

/// Clamps `v` into `[lo, hi]`.
/// `java.lang.Math.round(double)`, as a `f64`.
///
/// Java returns "the closest `long` to the argument, with ties rounding to
/// positive infinity", and **`0` when the argument is NaN**. Rust's
/// [`f64::round`] rounds halves *away from zero* and propagates NaN, so both
/// halves of that contract differ:
///
/// * a tie at a negative value — Java's `round(-2.5)` is `-2`, Rust's is `-3`;
/// * NaN — which this quantizer really produces, because a vector whose
///   residual components are all equal gives `a == b`, hence an infinite
///   `stepInv` and a `0 * Infinity` product. Java turns that into `0` and the
///   optimiser leaves the interval alone; propagating NaN instead poisons every
///   accumulator and writes a NaN interval into the `.vemq`.
///
/// The value is `floor(x + 0.5)` computed without the intermediate addition, so
/// it does not inherit that addition's double rounding.
fn java_math_round_f64(x: f64) -> f64 {
    if x.is_nan() {
        return 0.0;
    }
    let floor = x.floor();
    if x - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `java.lang.Math.round(float)`, as a `f32`. See [`java_math_round_f64`] for
/// why Rust's [`f32::round`] cannot be used.
fn java_math_round_f32(x: f32) -> f32 {
    if x.is_nan() {
        return 0.0;
    }
    let floor = x.floor();
    if x - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

impl OptimizedScalarQuantizer {
    /// Creates a quantizer with Lucene's default lambda and iteration count.
    pub fn new(similarity_function: VectorSimilarityFunction) -> Self {
        Self {
            similarity_function,
            lambda: DEFAULT_LAMBDA,
            iters: DEFAULT_ITERS,
        }
    }

    /// Creates a quantizer with an explicit lambda and iteration count.
    pub fn with_params(
        similarity_function: VectorSimilarityFunction,
        lambda: f32,
        iters: i32,
    ) -> Self {
        Self {
            similarity_function,
            lambda,
            iters,
        }
    }

    /// Returns the similarity the quantization is tuned for.
    pub fn similarity_function(&self) -> VectorSimilarityFunction {
        self.similarity_function
    }

    /// The objective the interval search minimises: a weighted sum of the
    /// projection error and the squared error.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.loss`.
    fn loss(&self, vector: &[f32], interval: [f32; 2], points: i32, norm2: f32) -> f64 {
        let a = f64::from(interval[0]);
        let b = f64::from(interval[1]);
        let step = (b - a) / f64::from(points - 1);
        let step_inv = 1.0 / step;
        let mut xe = 0.0f64;
        let mut e = 0.0f64;
        for &xi in vector {
            let xi = f64::from(xi);
            // Quantize then dequantize, and measure what was lost.
            let xiq = a + step * java_math_round_f64((clamp(xi, a, b) - a) * step_inv);
            xe += xi * (xi - xiq);
            e += (xi - xiq) * (xi - xiq);
        }
        // Java is `(1.0 - lambda) * xe * xe / norm2 + lambda * e`, where `1.0`
        // is a **double** literal, so `1.0 - lambda` is a double subtraction of
        // the widened float. Computing it in `f32` first and widening — as this
        // port did — gives 0.9f (0.899999976…) instead of 0.899999998…, which
        // shifts the loss and can pick a different optimised interval.
        (1.0 - f64::from(self.lambda)) * xe * xe / f64::from(norm2) + f64::from(self.lambda) * e
    }

    /// Refines `interval` by coordinate descent, stopping as soon as a step
    /// stops improving — the objective is not convex, so it can get worse.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.optimizeIntervals`.
    fn optimize_intervals(&self, interval: &mut [f32; 2], vector: &[f32], norm2: f32, points: i32) {
        let mut initial_loss = self.loss(vector, *interval, points, norm2);
        let scale = (1.0 - self.lambda) / norm2;
        if !scale.is_finite() {
            return;
        }

        for _ in 0..self.iters {
            let a = interval[0];
            let b = interval[1];
            let step_inv = (points as f32 - 1.0) / (b - a);

            let (mut daa, mut dab, mut dbb, mut dax, mut dbx) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
            for &xi in vector {
                // Java is `float k = Math.round((clamp(xi, a, b) - a) * stepInv)`,
                // where `clamp` returns a **double**, so the subtraction and the
                // multiplication both widen to double and only the rounded
                // result narrows to float. Narrowing the clamped value to `f32`
                // first — as this port did — changes the last bits of `k` for
                // some vectors, which moves the optimised interval and then a
                // quantized component, and the `.veq` stops matching Lucene's.
                let k = java_math_round_f64(
                    (clamp(f64::from(xi), f64::from(a), f64::from(b)) - f64::from(a))
                        * f64::from(step_inv),
                ) as f32;
                // `s` stays a `float`, because Java's five accumulations do
                // **not** all widen it. `1.0` is a double literal, so
                // `(1.0 - s)` is a double and `daa`, `dab` and `dax` are double
                // products; but `s * s` and `xi * s` are float*float, rounded to
                // float *before* being added to the double accumulator. Doing
                // those two in `f64` — as this port did — makes `dbb` and `dbx`
                // differ in the 8th digit, which moves the optimised interval by
                // one ULP and changes a quantized component in the `.veq`.
                let s = k / (points as f32 - 1.0);
                let s_wide = f64::from(s);
                daa += (1.0 - s_wide) * (1.0 - s_wide);
                dab += (1.0 - s_wide) * s_wide;
                dbb += f64::from(s * s);
                dax += f64::from(xi) * (1.0 - s_wide);
                dbx += f64::from(xi * s);
            }

            let m0 = f64::from(scale) * dax * dax + f64::from(self.lambda) * daa;
            let m1 = f64::from(scale) * dax * dbx + f64::from(self.lambda) * dab;
            let m2 = f64::from(scale) * dbx * dbx + f64::from(self.lambda) * dbb;

            let det = m0 * m2 - m1 * m1;
            if det == 0.0 {
                return;
            }
            let a_opt = ((m2 * dax - m1 * dbx) / det) as f32;
            let b_opt = ((m0 * dbx - m1 * dax) / det) as f32;

            if (interval[0] - a_opt).abs() < 1e-8 && (interval[1] - b_opt).abs() < 1e-8 {
                return;
            }
            let new_loss = self.loss(vector, [a_opt, b_opt], points, norm2);
            if new_loss > initial_loss {
                return;
            }
            interval[0] = a_opt;
            interval[1] = b_opt;
            initial_loss = new_loss;
        }
    }

    /// Quantizes `vector` against `centroid` into `destination`.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.scalarQuantize`. `vector` is
    /// modified in place, as Java's is: it becomes the residual against the
    /// centroid.
    pub fn scalar_quantize(
        &self,
        vector: &mut [f32],
        destination: &mut [u8],
        bits: u8,
        centroid: &[f32],
    ) -> Result<QuantizationResult> {
        if bits == 0 || bits > 8 {
            return Err(LuceneError::IllegalArgument(format!(
                "bits must be in [1, 8], got {bits}"
            )));
        }
        if vector.len() > destination.len() || vector.len() != centroid.len() {
            return Err(LuceneError::IllegalArgument(
                "vector, destination and centroid lengths are inconsistent".to_string(),
            ));
        }

        let points = 1i32 << bits;
        let mut vec_mean = 0.0f64;
        let mut vec_var = 0.0f64;
        let mut norm2 = 0.0f32;
        let mut centroid_dot = 0.0f32;
        let mut min = f32::MAX;
        let mut max = -f32::MAX;

        for i in 0..vector.len() {
            if self.similarity_function != VectorSimilarityFunction::EUCLIDEAN {
                centroid_dot += vector[i] * centroid[i];
            }
            // Everything downstream works on the residual against the centroid.
            vector[i] -= centroid[i];
            min = min.min(vector[i]);
            max = max.max(vector[i]);
            norm2 += vector[i] * vector[i];
            let delta = f64::from(vector[i]) - vec_mean;
            vec_mean += delta / (i as f64 + 1.0);
            vec_var += delta * (f64::from(vector[i]) - vec_mean);
        }
        vec_var /= vector.len() as f64;
        let vec_std = vec_var.sqrt();

        // Start from the MSE-optimal interval for a Gaussian of this spread,
        // then refine it.
        let grid = MINIMUM_MSE_GRID[(bits - 1) as usize];
        let mut interval = [
            clamp(
                f64::from(grid[0]) * vec_std + vec_mean,
                f64::from(min),
                f64::from(max),
            ) as f32,
            clamp(
                f64::from(grid[1]) * vec_std + vec_mean,
                f64::from(min),
                f64::from(max),
            ) as f32,
        ];
        self.optimize_intervals(&mut interval, vector, norm2, points);

        let n_steps = (points - 1) as f32;
        let a = interval[0];
        let b = interval[1];
        let step = (b - a) / n_steps;
        let mut sum_query = 0i32;
        for h in 0..vector.len() {
            let xi = clamp(f64::from(vector[h]), f64::from(a), f64::from(b)) as f32;
            let assignment = java_math_round_f32((xi - a) / step) as i32;
            sum_query += assignment;
            destination[h] = assignment as u8;
        }

        Ok(QuantizationResult {
            lower_interval: interval[0],
            upper_interval: interval[1],
            additional_correction: if self.similarity_function
                == VectorSimilarityFunction::EUCLIDEAN
            {
                norm2
            } else {
                centroid_dot
            },
            quantized_component_sum: sum_query,
        })
    }

    /// Quantizes one vector at several bit widths at once.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.multiScalarQuantize`.
    pub fn multi_scalar_quantize(
        &self,
        vector: &mut [f32],
        destinations: &mut [&mut [u8]],
        bits: &[u8],
        centroid: &[f32],
    ) -> Result<Vec<QuantizationResult>> {
        if bits.len() != destinations.len() {
            return Err(LuceneError::IllegalArgument(
                "bits and destinations must have the same length".to_string(),
            ));
        }
        // The residual is shared across the widths, so it is computed once and
        // the vector is restored for each pass.
        let original = vector.to_vec();
        let mut results = Vec::with_capacity(bits.len());
        for (i, &b) in bits.iter().enumerate() {
            vector.copy_from_slice(&original);
            results.push(self.scalar_quantize(vector, destinations[i], b, centroid)?);
        }
        Ok(results)
    }

    /// Turns a quantized vector back into floats.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.deQuantize`.
    pub fn de_quantize(
        quantized: &[u8],
        bits: u8,
        result: &QuantizationResult,
        centroid: &[f32],
    ) -> Vec<f32> {
        let points = (1i32 << bits) - 1;
        let step = (result.upper_interval - result.lower_interval) / points as f32;
        quantized
            .iter()
            .zip(centroid.iter())
            .map(|(&q, &c)| result.lower_interval + step * f32::from(q) + c)
            .collect()
    }

    /// Rounds `value` up to the next multiple of `bucket`.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.discretize`.
    pub fn discretize(value: i32, bucket: i32) -> i32 {
        ((value + bucket - 1) / bucket) * bucket
    }

    /// Packs one bit per component into `packed`, most significant bit first.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.packAsBinary`.
    /// Transposes a 4-bit quantized vector into four bit-planes.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.transposeHalfByte(byte[],
    /// byte[])`: each of the four bits of a component goes to its own quarter
    /// of `quant_query_byte`, most significant bit first inside each byte.
    ///
    /// # Panics
    ///
    /// Panics in a debug build when a component is outside `[0, 15]`, which is
    /// Java's `assert`.
    pub fn transpose_half_byte(q: &[u8], quant_query_byte: &mut [u8]) {
        let quarter = quant_query_byte.len() / 4;
        let half = quant_query_byte.len() / 2;
        let three_quarters = 3 * quant_query_byte.len() / 4;
        let mut i = 0usize;
        while i < q.len() {
            debug_assert!(q[i] <= 15);
            let mut lower_byte = 0i32;
            let mut lower_middle_byte = 0i32;
            let mut upper_middle_byte = 0i32;
            let mut upper_byte = 0i32;
            let mut j = 7i32;
            while j >= 0 && i < q.len() {
                let v = i32::from(q[i]);
                lower_byte |= (v & 1) << j;
                lower_middle_byte |= ((v >> 1) & 1) << j;
                upper_middle_byte |= ((v >> 2) & 1) << j;
                upper_byte |= ((v >> 3) & 1) << j;
                i += 1;
                j -= 1;
            }
            let index = ((i + 7) / 8) - 1;
            quant_query_byte[index] = lower_byte as u8;
            quant_query_byte[index + quarter] = lower_middle_byte as u8;
            quant_query_byte[index + half] = upper_middle_byte as u8;
            quant_query_byte[index + three_quarters] = upper_byte as u8;
        }
    }

    /// Transposes a 2-bit quantized vector into two bit-planes.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.transposeDibit(byte[], byte[])`,
    /// which is [`transpose_half_byte`](Self::transpose_half_byte) for two bits
    /// instead of four.
    ///
    /// # Panics
    ///
    /// Panics in a debug build when a component is outside `[0, 3]`, which is
    /// Java's `assert`.
    pub fn transpose_dibit(vector: &[u8], packed: &mut [u8]) {
        let stripe_size = packed.len() / 2;
        let limit = vector.len().saturating_sub(7);
        let mut i = 0usize;
        let mut index = 0usize;
        while i < limit {
            let lower_byte = (i32::from(vector[i] & 1) << 7)
                | (i32::from(vector[i + 1] & 1) << 6)
                | (i32::from(vector[i + 2] & 1) << 5)
                | (i32::from(vector[i + 3] & 1) << 4)
                | (i32::from(vector[i + 4] & 1) << 3)
                | (i32::from(vector[i + 5] & 1) << 2)
                | (i32::from(vector[i + 6] & 1) << 1)
                | i32::from(vector[i + 7] & 1);
            let upper_byte = (i32::from((vector[i] >> 1) & 1) << 7)
                | (i32::from((vector[i + 1] >> 1) & 1) << 6)
                | (i32::from((vector[i + 2] >> 1) & 1) << 5)
                | (i32::from((vector[i + 3] >> 1) & 1) << 4)
                | (i32::from((vector[i + 4] >> 1) & 1) << 3)
                | (i32::from((vector[i + 5] >> 1) & 1) << 2)
                | (i32::from((vector[i + 6] >> 1) & 1) << 1)
                | i32::from((vector[i + 7] >> 1) & 1);
            packed[index] = lower_byte as u8;
            packed[index + stripe_size] = upper_byte as u8;
            i += 8;
            index += 1;
        }
        if i == vector.len() {
            return;
        }
        let mut lower_byte = 0i32;
        let mut upper_byte = 0i32;
        let mut j = 7i32;
        while i < vector.len() {
            debug_assert!(vector[i] <= 3);
            lower_byte |= i32::from(vector[i] & 1) << j;
            upper_byte |= i32::from((vector[i] >> 1) & 1) << j;
            j -= 1;
            i += 1;
        }
        packed[index] = lower_byte as u8;
        packed[index + stripe_size] = upper_byte as u8;
    }

    /// Rebuilds a 2-bit quantized vector from its two bit-planes.
    ///
    /// Equivalent to `OptimizedScalarQuantizer.untransposeDibit(byte[],
    /// byte[])`, the inverse of
    /// [`transpose_dibit`](Self::transpose_dibit).
    pub fn untranspose_dibit(packed: &[u8], vector: &mut [u8]) {
        let stripe_size = packed.len() / 2;
        let limit = vector.len().saturating_sub(7);
        let mut i = 0usize;
        let mut index = 0usize;
        while i < limit {
            let lower_byte = packed[index];
            let upper_byte = packed[index + stripe_size];
            for k in 0..8usize {
                let shift = 7 - k as u32;
                vector[i + k] = ((lower_byte >> shift) & 1) | (((upper_byte >> shift) & 1) << 1);
            }
            i += 8;
            index += 1;
        }
        if i < vector.len() {
            let lower_byte = packed[index];
            let upper_byte = packed[index + stripe_size];
            let mut j = 7u32;
            while i < vector.len() {
                vector[i] = ((lower_byte >> j) & 1) | (((upper_byte >> j) & 1) << 1);
                j = j.wrapping_sub(1);
                i += 1;
            }
        }
    }

    pub fn pack_as_binary(vector: &[u8], packed: &mut [u8]) {
        packed.fill(0);
        for (i, &v) in vector.iter().enumerate() {
            if v != 0 {
                packed[i / 8] |= 1 << (7 - (i % 8));
            }
        }
    }

    /// Reverses [`pack_as_binary`].
    ///
    /// Equivalent to `OptimizedScalarQuantizer.unpackBinary`.
    pub fn unpack_binary(packed: &[u8], vector: &mut [u8]) {
        for (i, slot) in vector.iter_mut().enumerate() {
            *slot = (packed[i / 8] >> (7 - (i % 8))) & 1;
        }
    }
}
