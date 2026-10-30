//! `ScalarQuantizer` ported from `org.apache.lucene.util.quantization`.

use crate::error::{LuceneError, Result};
use crate::index::vector_values::FloatVectorValues;
use crate::index::VectorSimilarityFunction;
use crate::search::bayesian_score_estimator::JavaRandom;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::hit_queue::HitQueue;
use crate::search::score_doc::ScoreDoc;
use crate::util::quantization::ScalarQuantizedVectorSimilarity;
use crate::util::selector::{intro_select, PivotOps};
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
            return Err(LuceneError::illegal_state(
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
    /// Equivalent to `ScalarQuantizer.quantize(float[], byte[],
    /// VectorSimilarityFunction)` (`ScalarQuantizer.java:115-125`), which
    /// delegates to `VectorUtil.minMaxScalarQuantize`.
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
        let correction = vector_util::min_max_scalar_quantize(
            src,
            dest,
            self.scale,
            self.alpha,
            self.min_quantile,
            self.max_quantile,
        )?;
        if similarity_function == VectorSimilarityFunction::EUCLIDEAN {
            return Ok(0.0);
        }
        Ok(correction)
    }

    /// Recomputes the corrective offset of an already quantized vector against
    /// a different quantizer.
    ///
    /// Equivalent to `ScalarQuantizer.recalculateCorrectiveOffset`
    /// (`ScalarQuantizer.java:135-151`), which delegates to
    /// `VectorUtil.recalculateOffset`.
    pub fn recalculate_corrective_offset(
        &self,
        quantized_vector: &[u8],
        old_quantizer: &ScalarQuantizer,
        similarity_function: VectorSimilarityFunction,
    ) -> f32 {
        if similarity_function == VectorSimilarityFunction::EUCLIDEAN {
            return 0.0;
        }
        vector_util::recalculate_offset(
            quantized_vector,
            old_quantizer.alpha,
            old_quantizer.min_quantile,
            self.scale,
            self.alpha,
            self.min_quantile,
            self.max_quantile,
        )
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
    /// computation itself, for a caller that has already gathered the sample.
    /// [`from_vectors`](Self::from_vectors) is the full form.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `confidence_interval` is
    /// outside `[0.9, 1.0]`, and whatever [`new`](Self::new) raises.
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
        if confidence_interval == 1.0 {
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            for &v in sample {
                min = min.min(v);
                max = max.max(v);
            }
            return Self::new(min, max, bits);
        }
        let mut scratch = sample.to_vec();
        let [lower, upper] = Self::get_upper_and_lower_quantile(&mut scratch, confidence_interval);
        Self::new(lower, upper, bits)
    }

    /// Returns the values at the `(1 - confidence_interval)/2` and
    /// `confidence_interval/2` percentiles of `arr`, which need not be sorted.
    ///
    /// Equivalent to `ScalarQuantizer.getUpperAndLowerQuantile(float[], float)`.
    /// Given `[0..100]` and a confidence interval of `0.9`, it returns `5` and
    /// `95`. The array is permuted in place, as Java's is.
    ///
    /// # Panics
    ///
    /// Panics when `arr` is empty, which is Java's `assert arr.length > 0`.
    pub fn get_upper_and_lower_quantile(arr: &mut [f32], confidence_interval: f32) -> [f32; 2] {
        assert!(!arr.is_empty(), "arr.length > 0");
        // With one or two values the quantiles are undefined: return the
        // minimum and the maximum.
        if arr.len() <= 2 {
            arr.sort_by(f32::total_cmp);
            return [arr[0], arr[arr.len() - 1]];
        }
        let selector_index = (arr.len() as f32 * (1.0 - confidence_interval) / 2.0 + 0.5) as usize;
        if selector_index > 0 {
            let len = arr.len();
            let mut selector = FloatSelector::new(arr);
            intro_select(&mut selector, 0, len, len - selector_index);
            let mut selector = FloatSelector::new(arr);
            intro_select(&mut selector, 0, len - selector_index, selector_index);
        }
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for &v in &arr[selector_index..arr.len() - selector_index] {
            min = min.min(v);
            max = max.max(v);
        }
        [min, max]
    }

    /// Chooses `sample_size` ordinals out of `num_float_vecs`, in ascending
    /// order.
    ///
    /// Equivalent to the private
    /// `ScalarQuantizer.reservoirSampleIndices(int, int)`, which draws from a
    /// generator seeded with `42`, so the same input always yields the same
    /// sample — and the same quantiles in the index.
    pub fn reservoir_sample_indices(num_float_vecs: i32, sample_size: i32) -> Vec<i32> {
        let mut random = JavaRandom::new(42);
        let mut vectors_to_take: Vec<i32> = (0..sample_size).collect();
        for i in sample_size..num_float_vecs {
            let j = random.next_int(i + 1);
            if j < sample_size {
                vectors_to_take[j as usize] = i;
            }
        }
        vectors_to_take.sort_unstable();
        vectors_to_take
    }

    /// Accumulates one scratch buffer's quantiles into the running sums.
    ///
    /// Equivalent to the private `ScalarQuantizer.extractQuantiles(float[],
    /// float[], double[], double[])`.
    fn extract_quantiles(
        confidence_intervals: &[f32],
        quantile_gathering_scratch: &mut [f32],
        upper_sum: &mut [f64],
        lower_sum: &mut [f64],
    ) {
        debug_assert!(
            confidence_intervals.len() == upper_sum.len()
                && confidence_intervals.len() == lower_sum.len()
        );
        for (i, &interval) in confidence_intervals.iter().enumerate() {
            let upper_and_lower =
                Self::get_upper_and_lower_quantile(quantile_gathering_scratch, interval);
            upper_sum[i] += f64::from(upper_and_lower[1]);
            lower_sum[i] += f64::from(upper_and_lower[0]);
        }
    }

    /// Copies one vector into the scratch buffer and keeps a copy of it.
    ///
    /// Equivalent to the private `ScalarQuantizer.gatherSample(float[],
    /// float[], List, int)`.
    fn gather_sample(
        vector_value: &[f32],
        quantile_gathering_scratch: &mut [f32],
        sampled_docs: &mut Vec<Vec<f32>>,
        i: usize,
    ) {
        sampled_docs.push(vector_value.to_vec());
        let start = i * vector_value.len();
        quantile_gathering_scratch[start..start + vector_value.len()].copy_from_slice(vector_value);
    }

    /// Reads the float vectors and computes the quantiles.
    ///
    /// Equivalent to `ScalarQuantizer.fromVectors(FloatVectorValues, float,
    /// int, byte)`, which samples
    /// [`SCALAR_QUANTIZATION_SAMPLE_SIZE`] vectors when there are more than
    /// that many.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the vectors.
    pub fn from_vectors(
        float_vector_values: &dyn FloatVectorValues,
        confidence_interval: f32,
        total_vector_count: i32,
        bits: u8,
    ) -> Result<Self> {
        Self::from_vectors_with_sample_size(
            float_vector_values,
            confidence_interval,
            total_vector_count,
            bits,
            SCALAR_QUANTIZATION_SAMPLE_SIZE as i32,
        )
    }

    /// Reads the float vectors and computes the quantiles over a sample of the
    /// given size.
    ///
    /// Equivalent to the package-private
    /// `ScalarQuantizer.fromVectors(FloatVectorValues, float, int, byte, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the vectors.
    pub fn from_vectors_with_sample_size(
        float_vector_values: &dyn FloatVectorValues,
        confidence_interval: f32,
        total_vector_count: i32,
        bits: u8,
        quantization_sample_size: i32,
    ) -> Result<Self> {
        debug_assert!((0.9..=1.0).contains(&confidence_interval));
        debug_assert!(quantization_sample_size > SCRATCH_SIZE as i32);
        if total_vector_count == 0 {
            return Self::new(0.0, 0.0, bits);
        }
        let mut iterator = float_vector_values.iterator()?;
        if confidence_interval == 1.0 {
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            while iterator.next_doc()? != NO_MORE_DOCS {
                for v in float_vector_values.vector_value(iterator.index())? {
                    min = min.min(v);
                    max = max.max(v);
                }
            }
            return Self::new(min, max, bits);
        }
        let dimension = float_vector_values.dimension() as usize;
        let scratch_len = dimension * SCRATCH_SIZE.min(total_vector_count as usize);
        let mut quantile_gathering_scratch = vec![0.0f32; scratch_len];
        let mut count = 0usize;
        let mut upper_sum = [0.0f64; 1];
        let mut lower_sum = [0.0f64; 1];
        let confidence_intervals = [confidence_interval];

        if total_vector_count <= quantization_sample_size {
            let scratch_size = SCRATCH_SIZE.min(total_vector_count as usize);
            let mut i = 0usize;
            while iterator.next_doc()? != NO_MORE_DOCS {
                let vector_value = float_vector_values.vector_value(iterator.index())?;
                quantile_gathering_scratch[i * vector_value.len()..(i + 1) * vector_value.len()]
                    .copy_from_slice(&vector_value);
                i += 1;
                if i == scratch_size {
                    Self::extract_quantiles(
                        &confidence_intervals,
                        &mut quantile_gathering_scratch,
                        &mut upper_sum,
                        &mut lower_sum,
                    );
                    i = 0;
                    count += 1;
                }
            }
            // The rest of the scratch state is deliberately unused when there
            // are fewer than SCRATCH_SIZE vectors: with so small a sample the
            // extreme confidence intervals would distort the result.
            return Self::new(
                lower_sum[0] as f32 / count as f32,
                upper_sum[0] as f32 / count as f32,
                bits,
            );
        }

        let vectors_to_take =
            Self::reservoir_sample_indices(total_vector_count, quantization_sample_size);
        let mut index = 0i32;
        let mut idx = 0usize;
        for i in vectors_to_take {
            while index <= i {
                // `advance(docId)` cannot be used: MergedVectorValues does not
                // support it.
                iterator.next_doc()?;
                index += 1;
            }
            debug_assert!(iterator.doc_id() != NO_MORE_DOCS);
            let vector_value = float_vector_values.vector_value(iterator.index())?;
            quantile_gathering_scratch[idx * vector_value.len()..(idx + 1) * vector_value.len()]
                .copy_from_slice(&vector_value);
            idx += 1;
            if idx == SCRATCH_SIZE {
                Self::extract_quantiles(
                    &confidence_intervals,
                    &mut quantile_gathering_scratch,
                    &mut upper_sum,
                    &mut lower_sum,
                );
                count += 1;
                idx = 0;
            }
        }
        Self::new(
            lower_sum[0] as f32 / count as f32,
            upper_sum[0] as f32 / count as f32,
            bits,
        )
    }

    /// Reads the float vectors and picks the quantile pair that best preserves
    /// the ranking of the nearest neighbours.
    ///
    /// Equivalent to `ScalarQuantizer.fromVectorsAutoInterval(FloatVectorValues,
    /// VectorSimilarityFunction, int, byte)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the quantile computation
    /// produces a non-finite value, and propagates any error raised while
    /// reading the vectors.
    pub fn from_vectors_auto_interval(
        float_vector_values: &dyn FloatVectorValues,
        function: VectorSimilarityFunction,
        total_vector_count: i32,
        bits: u8,
    ) -> Result<Self> {
        debug_assert!(function != VectorSimilarityFunction::COSINE);
        if total_vector_count == 0 {
            return Self::new(0.0, 0.0, bits);
        }

        let sample_size = total_vector_count.min(1000);
        let dimension = float_vector_values.dimension();
        let scratch_len = dimension as usize * SCRATCH_SIZE.min(total_vector_count as usize);
        let mut quantile_gathering_scratch = vec![0.0f32; scratch_len];
        let mut count = 0usize;
        let mut upper_sum = [0.0f64; 2];
        let mut lower_sum = [0.0f64; 2];
        let mut sampled_docs: Vec<Vec<f32>> = Vec::with_capacity(sample_size as usize);
        let confidence_intervals = [
            1.0 - (32.0f32).min(dimension as f32 / 10.0) / (dimension as f32 + 1.0),
            1.0 - 1.0 / (dimension as f32 + 1.0),
        ];
        let mut iterator = float_vector_values.iterator()?;
        if total_vector_count <= sample_size {
            let scratch_size = SCRATCH_SIZE.min(total_vector_count as usize);
            let mut i = 0usize;
            while iterator.next_doc()? != NO_MORE_DOCS {
                let vector_value = float_vector_values.vector_value(iterator.index())?;
                Self::gather_sample(
                    &vector_value,
                    &mut quantile_gathering_scratch,
                    &mut sampled_docs,
                    i,
                );
                i += 1;
                if i == scratch_size {
                    Self::extract_quantiles(
                        &confidence_intervals,
                        &mut quantile_gathering_scratch,
                        &mut upper_sum,
                        &mut lower_sum,
                    );
                    i = 0;
                    count += 1;
                }
            }
        } else {
            // Reservoir-sample the vector ordinals to read.
            let vectors_to_take = Self::reservoir_sample_indices(total_vector_count, 1000);
            let mut index = 0i32;
            let mut idx = 0usize;
            for i in vectors_to_take {
                while index <= i {
                    // `advance(docId)` cannot be used: MergedVectorValues does
                    // not support it.
                    iterator.next_doc()?;
                    index += 1;
                }
                debug_assert!(iterator.doc_id() != NO_MORE_DOCS);
                let vector_value = float_vector_values.vector_value(iterator.index())?;
                Self::gather_sample(
                    &vector_value,
                    &mut quantile_gathering_scratch,
                    &mut sampled_docs,
                    idx,
                );
                idx += 1;
                if idx == SCRATCH_SIZE {
                    Self::extract_quantiles(
                        &confidence_intervals,
                        &mut quantile_gathering_scratch,
                        &mut upper_sum,
                        &mut lower_sum,
                    );
                    count += 1;
                    idx = 0;
                }
            }
        }

        // Gather the upper and lower bounds for the quantile grid search.
        let al = lower_sum[1] as f32 / count as f32;
        let bu = upper_sum[1] as f32 / count as f32;
        let au = lower_sum[0] as f32 / count as f32;
        let bl = upper_sum[0] as f32 / count as f32;
        if !al.is_finite() || !au.is_finite() || !bl.is_finite() || !bu.is_finite() {
            return Err(LuceneError::illegal_state(
                "Quantile calculation resulted in NaN or infinite values".to_string(),
            ));
        }
        let mut lower_candidates = [0.0f32; 16];
        let mut upper_candidates = [0.0f32; 16];
        let mut idx = 0usize;
        let mut i = 0.0f32;
        while i < 32.0 {
            lower_candidates[idx] = al + i * (au - al) / 32.0;
            upper_candidates[idx] = bl + i * (bu - bl) / 32.0;
            idx += 1;
            i += 2.0;
        }
        // Find the best candidate pair by correlating the true quantized
        // nearest-neighbour scores with the float vector scores.
        let nearest_neighbors = Self::find_nearest_neighbors(&sampled_docs, function)?;
        let best_pair = Self::candidate_grid_search(
            &nearest_neighbors,
            &sampled_docs,
            &lower_candidates,
            &upper_candidates,
            function,
            bits,
        )?;
        Self::new(best_pair[0], best_pair[1], bits)
    }

    /// Searches the candidate grid for the quantile pair whose quantized scores
    /// correlate best with the float ones.
    ///
    /// Equivalent to the private `ScalarQuantizer.candidateGridSearch`, which
    /// first scans the grid coarsely and then refines inside the best quadrant.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while scoring.
    fn candidate_grid_search(
        nearest_neighbors: &[ScoreDocsAndScoreVariance],
        vectors: &[Vec<f32>],
        lower_candidates: &[f32; 16],
        upper_candidates: &[f32; 16],
        function: VectorSimilarityFunction,
        bits: u8,
    ) -> Result<[f32; 2]> {
        let mut max_corr = f64::NEG_INFINITY;
        let mut best_lower = 0.0f32;
        let mut best_upper = 0.0f32;
        let mut correlator = ScoreErrorCorrelator::new(function, nearest_neighbors, vectors, bits);
        // First a coarse search, for the initial best candidate pair.
        let mut best_quadrant_lower = 0usize;
        let mut best_quadrant_upper = 0usize;
        let mut i = 0usize;
        while i < lower_candidates.len() {
            let lower = lower_candidates[i];
            if !lower.is_finite() {
                debug_assert!(false, "Lower candidate is NaN or infinite");
                i += 4;
                continue;
            }
            let mut j = 0usize;
            while j < upper_candidates.len() {
                let upper = upper_candidates[j];
                if !upper.is_finite() {
                    debug_assert!(false, "Upper candidate is NaN or infinite");
                    j += 4;
                    continue;
                }
                if upper <= lower {
                    j += 4;
                    continue;
                }
                let mean = correlator.score_error_correlation(lower, upper)?;
                if mean > max_corr {
                    max_corr = mean;
                    best_lower = lower;
                    best_upper = upper;
                    best_quadrant_lower = i;
                    best_quadrant_upper = j;
                }
                j += 4;
            }
            i += 4;
        }
        // Now search inside the best quadrant.
        for i in best_quadrant_lower + 1..best_quadrant_lower + 4 {
            for j in best_quadrant_upper + 1..best_quadrant_upper + 4 {
                let lower = lower_candidates[i];
                let upper = upper_candidates[j];
                if !lower.is_finite() || !upper.is_finite() {
                    debug_assert!(false, "Lower or upper candidate is NaN or infinite");
                    continue;
                }
                if upper <= lower {
                    continue;
                }
                let mean = correlator.score_error_correlation(lower, upper)?;
                if mean > max_corr {
                    max_corr = mean;
                    best_lower = lower;
                    best_upper = upper;
                }
            }
        }
        Ok([best_lower, best_upper])
    }

    /// Returns the ten nearest neighbours of each vector among the others, with
    /// the variance of their scores.
    ///
    /// Equivalent to the private `ScalarQuantizer.findNearestNeighbors(List,
    /// VectorSimilarityFunction)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while comparing vectors.
    fn find_nearest_neighbors(
        vectors: &[Vec<f32>],
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Vec<ScoreDocsAndScoreVariance>> {
        let mut queues: Vec<HitQueue> = Vec::with_capacity(vectors.len());
        queues.push(HitQueue::new(10, false)?);
        for i in 0..vectors.len() {
            for j in i + 1..vectors.len() {
                let score = similarity_function.compare_f32(&vectors[i], &vectors[j])?;
                // Initialise the rest of the queues.
                if queues.len() <= j {
                    queues.push(HitQueue::new(10, false)?);
                }
                queues[i].insert_with_overflow(ScoreDoc::new(j as i32, score));
                queues[j].insert_with_overflow(ScoreDoc::new(i as i32, score));
            }
        }
        // Extract the top ten from each queue.
        let mut result: Vec<ScoreDocsAndScoreVariance> = Vec::with_capacity(vectors.len());
        let mut mean_and_var = OnlineMeanAndVar::default();
        for queue in queues.iter_mut().take(vectors.len()) {
            let mut score_docs: Vec<ScoreDoc> = vec![ScoreDoc::new(0, 0.0); queue.size()];
            for j in (0..queue.size()).rev() {
                let doc = queue
                    .pop()
                    .expect("INVARIANT: the queue holds size() elements");
                mean_and_var.add(f64::from(doc.score));
                score_docs[j] = doc;
            }
            result.push(ScoreDocsAndScoreVariance {
                score_docs,
                score_variance: mean_and_var.var(),
            });
            mean_and_var.reset();
        }
        Ok(result)
    }
}

/// The selector `getUpperAndLowerQuantile` partitions the sample with.
///
/// Equivalent to the private `ScalarQuantizer.FloatSelector`, an `IntroSelector`
/// over a `float[]`.
///
/// **Divergence from Lucene 10.5.0.** Java holds the pivot as a `float` field
/// beside the array; the selector here borrows the array mutably, so the pivot
/// is a copy of the element rather than a reference to its slot. The values
/// compared are identical.
struct FloatSelector<'a> {
    arr: &'a mut [f32],
    pivot: f32,
}

impl<'a> FloatSelector<'a> {
    fn new(arr: &'a mut [f32]) -> Self {
        Self {
            arr,
            pivot: f32::NAN,
        }
    }
}

impl PivotOps for FloatSelector<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.arr.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot = self.arr[i];
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        // Float.compare orders NaN above every number and -0.0 below 0.0.
        match self.pivot.total_cmp(&self.arr[j]) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

/// A vector's nearest neighbours with the variance of their scores.
///
/// Equivalent to the private record
/// `ScalarQuantizer.ScoreDocsAndScoreVariance`.
#[derive(Clone, Debug)]
struct ScoreDocsAndScoreVariance {
    score_docs: Vec<ScoreDoc>,
    score_variance: f32,
}

/// Welford's running mean and variance.
///
/// Equivalent to the private `ScalarQuantizer.OnlineMeanAndVar`.
#[derive(Clone, Copy, Debug, Default)]
struct OnlineMeanAndVar {
    mean: f64,
    var: f64,
    n: i32,
}

impl OnlineMeanAndVar {
    fn reset(&mut self) {
        self.mean = 0.0;
        self.var = 0.0;
        self.n = 0;
    }

    fn add(&mut self, x: f64) {
        self.n += 1;
        let delta = x - self.mean;
        self.mean += delta / f64::from(self.n);
        self.var += delta * (x - self.mean);
    }

    fn var(&self) -> f32 {
        (self.var / f64::from(self.n - 1)) as f32
    }
}

/// Correlates the quantized nearest-neighbour scores with the float ones, which
/// is how the best quantile pair is chosen.
///
/// Equivalent to the private `ScalarQuantizer.ScoreErrorCorrelator`.
struct ScoreErrorCorrelator<'a> {
    corr: OnlineMeanAndVar,
    errors: OnlineMeanAndVar,
    function: VectorSimilarityFunction,
    nearest_neighbors: &'a [ScoreDocsAndScoreVariance],
    vectors: &'a [Vec<f32>],
    query: Vec<u8>,
    vector: Vec<u8>,
    bits: u8,
}

impl<'a> ScoreErrorCorrelator<'a> {
    fn new(
        function: VectorSimilarityFunction,
        nearest_neighbors: &'a [ScoreDocsAndScoreVariance],
        vectors: &'a [Vec<f32>],
        bits: u8,
    ) -> Self {
        let dimension = vectors.first().map_or(0, Vec::len);
        Self {
            corr: OnlineMeanAndVar::default(),
            errors: OnlineMeanAndVar::default(),
            function,
            nearest_neighbors,
            vectors,
            query: vec![0u8; dimension],
            vector: vec![0u8; dimension],
            bits,
        }
    }

    /// Equivalent to `ScoreErrorCorrelator.scoreErrorCorrelation(float, float)`.
    fn score_error_correlation(&mut self, lower_quantile: f32, upper_quantile: f32) -> Result<f64> {
        self.corr.reset();
        let quantizer = ScalarQuantizer::new(lower_quantile, upper_quantile, self.bits)?;
        let similarity = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            self.function,
            quantizer.get_constant_multiplier(),
            quantizer.get_bits(),
        );
        for i in 0..self.nearest_neighbors.len() {
            let query_correction =
                quantizer.quantize(&self.vectors[i], &mut self.query, self.function)?;
            let score_variance = self.nearest_neighbors[i].score_variance;
            // Score the vector against its nearest neighbours, but with the
            // quantized scores this time.
            self.errors.reset();
            for k in 0..self.nearest_neighbors[i].score_docs.len() {
                let score_doc = self.nearest_neighbors[i].score_docs[k].clone();
                let vector_correction = quantizer.quantize(
                    &self.vectors[score_doc.doc as usize],
                    &mut self.vector,
                    self.function,
                )?;
                let q_score = similarity.score(
                    &self.query,
                    query_correction,
                    &self.vector,
                    vector_correction,
                )?;
                self.errors.add(f64::from(q_score - score_doc.score));
            }
            self.corr
                .add(f64::from(1.0 - self.errors.var() / score_variance));
        }
        Ok(if self.corr.mean.is_nan() {
            0.0
        } else {
            self.corr.mean
        })
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
