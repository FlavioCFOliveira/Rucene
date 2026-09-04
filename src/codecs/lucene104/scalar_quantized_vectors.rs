//! Scalar-quantized vectors ported from `org.apache.lucene.codecs.lucene104`.
//!
//! Stores each float vector as one byte (or half a byte) per dimension, against
//! a per-field centroid, with the corrections that let the compressed form be
//! scored directly.

use std::sync::Arc;

use crate::codecs::codec_util::{check_index_header, retrieve_checksum};
use crate::codecs::hnsw::flat_vectors::{DocsWithFieldSet, FlatVectorsScorer};
use crate::codecs::lucene95::{HasIndexSlice, OrdToDocDISIReaderConfiguration};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::error::{LuceneError, Result};
use crate::index::index_sorter::SorterDocMap;
use crate::index::{
    segment_file_name, DenseDocIndexIterator, DocIndexIterator, FieldInfo, FloatVectorValues,
    MergeState, VectorEncoding, VectorSimilarityFunction,
};
use crate::search::NO_MORE_DOCS;
use crate::store::{IOContext, IndexInput, IndexOutput};
use crate::util::hnsw::scorer::{
    CloseableRandomVectorScorerSupplier, RandomVectorScorer, RandomVectorScorerSupplier,
    UpdateableRandomVectorScorer,
};
use crate::util::quantization::{
    OptimizedScalarQuantizer, QuantizationResult, QuantizedByteVectorValues,
    QuantizedVectorsReader, ScalarEncoding, ScalarQuantizedVectorSimilarity, ScalarQuantizer,
};
use crate::util::vector_util;

/// Info-stream component name this format reports under.
pub const QUANTIZED_VECTOR_COMPONENT: &str = "QVEC";
/// SPI name of the format.
pub const NAME: &str = "Lucene104ScalarQuantizedVectorsFormat";
/// First format version.
pub const VERSION_START: i32 = 0;
/// Current format version.
pub const VERSION_CURRENT: i32 = VERSION_START;
/// Codec name of the metadata file.
pub const META_CODEC_NAME: &str = "Lucene104ScalarQuantizedVectorsFormatMeta";
/// Codec name of the vector data file.
pub const VECTOR_DATA_CODEC_NAME: &str = "Lucene104ScalarQuantizedVectorsFormatData";
/// Extension of the metadata file.
pub const META_EXTENSION: &str = "vemq";
/// Extension of the vector data file.
pub const VECTOR_DATA_EXTENSION: &str = "veq";
/// Block shift of the monotonic ord-to-doc mapping.
pub const DIRECT_MONOTONIC_BLOCK_SHIFT: i32 = 16;
/// Largest vector dimension this format accepts.
pub const MAX_DIMS: i32 = 1024;

/// One field's entry in the metadata file.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorsReader.FieldEntry`.
#[derive(Clone, Debug)]
pub struct FieldEntry {
    /// How the field's vectors are scored.
    pub similarity_function: VectorSimilarityFunction,
    /// How the raw vectors were encoded.
    pub vector_encoding: VectorEncoding,
    /// How many components each vector has.
    pub dimension: i32,
    /// Where the field's vectors start in the data file.
    pub vector_data_offset: i64,
    /// How many bytes the field's vectors occupy.
    pub vector_data_length: i64,
    /// How many vectors the field holds.
    pub size: i32,
    /// How the quantized components are packed.
    pub scalar_encoding: ScalarEncoding,
    /// The centroid every vector was quantized against.
    pub centroid: Option<Vec<f32>>,
    /// Dot product of the centroid with itself.
    pub centroid_dp: f32,
    /// How the vector ordinals map back to document ids.
    pub ord_to_doc: OrdToDocDISIReaderConfiguration,
}

impl FieldEntry {
    /// Reads one field's entry.
    ///
    /// Equivalent to `FieldEntry.create(IndexInput, VectorEncoding, VectorSimilarityFunction)`.
    pub fn create(
        input: &mut dyn IndexInput,
        vector_encoding: VectorEncoding,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Self> {
        let dimension = input.read_v_int()?;
        let vector_data_offset = input.read_v_long()?;
        let vector_data_length = input.read_v_long()?;
        let size = input.read_v_int()?;

        let mut scalar_encoding = ScalarEncoding::UnsignedByte;
        let mut centroid = None;
        let mut centroid_dp = 0.0f32;

        if size > 0 {
            let wire_number = input.read_v_int()?;
            scalar_encoding = ScalarEncoding::from_code(wire_number).ok_or_else(|| {
                LuceneError::IllegalState(format!(
                    "Could not get ScalarEncoding from wire number: {wire_number}"
                ))
            })?;
            let mut values = vec![0f32; dimension.max(0) as usize];
            for slot in values.iter_mut() {
                *slot = f32::from_bits(input.read_int()? as u32);
            }
            centroid = Some(values);
            centroid_dp = f32::from_bits(input.read_int()? as u32);
        }

        let ord_to_doc = OrdToDocDISIReaderConfiguration::read_stored_meta(input, size)?;

        Ok(Self {
            similarity_function,
            vector_encoding,
            dimension,
            vector_data_offset,
            vector_data_length,
            size,
            scalar_encoding,
            centroid,
            centroid_dp,
            ord_to_doc,
        })
    }

    /// Returns how many bytes one quantized vector occupies, including its
    /// three corrective floats and the component sum.
    ///
    /// Equivalent to the stride `OffHeapScalarQuantizedVectorValues` computes:
    /// `encoding.getDocPackedLength(dimension) + Float.BYTES * 3 +
    /// Integer.BYTES`, which is also the term
    /// `Lucene104ScalarQuantizedVectorsReader.validateFieldEntry` multiplies by
    /// the vector count.
    pub fn byte_size(&self) -> usize {
        self.scalar_encoding
            .get_doc_packed_length(self.dimension.max(0)) as usize
            + 3 * std::mem::size_of::<f32>()
            + std::mem::size_of::<i32>()
    }
}

/// Reads one field's quantized vectors straight out of the data file.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.OffHeapScalarQuantizedVectorValues`.
pub struct OffHeapScalarQuantizedVectorValues {
    entry: FieldEntry,
    slice: Box<dyn IndexInput>,
    byte_size: usize,
    /// Scratch holding the vector last read.
    vector: Vec<u8>,
    /// Corrections of the vector last read.
    corrections: QuantizationResult,
    /// Which ordinal `vector` holds, or `-1`.
    loaded_ord: i32,
}

impl std::fmt::Debug for OffHeapScalarQuantizedVectorValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OffHeapScalarQuantizedVectorValues")
            .field("size", &self.entry.size)
            .field("dimension", &self.entry.dimension)
            .finish_non_exhaustive()
    }
}

impl OffHeapScalarQuantizedVectorValues {
    /// Opens the values of one field over `slice`, which must already be
    /// positioned at the field's data.
    ///
    /// Equivalent to `OffHeapScalarQuantizedVectorValues.load`.
    pub fn load(entry: FieldEntry, slice: Box<dyn IndexInput>) -> Self {
        let byte_size = entry.byte_size();
        let dim_bytes = entry
            .scalar_encoding
            .get_doc_length(entry.dimension.max(0) as usize);
        Self {
            entry,
            slice,
            byte_size,
            vector: vec![0u8; dim_bytes],
            corrections: QuantizationResult {
                lower_interval: 0.0,
                upper_interval: 0.0,
                additional_correction: 0.0,
                quantized_component_sum: 0,
            },
            loaded_ord: -1,
        }
    }

    /// Returns the centroid the field was quantized against.
    pub fn get_centroid(&self) -> Option<&[f32]> {
        self.entry.centroid.as_deref()
    }

    /// Returns the dot product of the centroid with itself.
    pub fn get_centroid_dp(&self) -> f32 {
        self.entry.centroid_dp
    }

    /// Returns the corrections of the vector last read.
    pub fn get_corrective_terms(&mut self, ord: i32) -> Result<QuantizationResult> {
        self.ensure_loaded(ord)?;
        Ok(self.corrections)
    }

    /// Returns how the components are packed.
    pub fn encoding(&self) -> ScalarEncoding {
        self.entry.scalar_encoding
    }

    /// Returns a second view over the same vectors, so that two ordinals can be
    /// read at once without clobbering one scratch buffer.
    ///
    /// # Errors
    ///
    /// Returns whatever cloning the underlying input raises.
    ///
    /// Equivalent to `OffHeapScalarQuantizedVectorValues.copy()`.
    pub fn copy(&self) -> Result<Self> {
        Ok(Self::load(self.entry.clone(), self.slice.clone_input()?))
    }

    /// Packs a vector of four-bit components, one per byte, into half as many
    /// bytes.
    ///
    /// The two halves of `unpacked` are striped, not adjacent: component `i`
    /// becomes the high nibble of `packed[i]` and component
    /// `packed.len() + i` its low nibble. This is the layout
    /// `VectorUtil.int4DotProductSinglePacked` expects.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `unpacked` is not exactly
    /// twice as long as `packed`, which Java asserts.
    ///
    /// Equivalent to `OffHeapScalarQuantizedVectorValues.packNibbles`.
    pub fn pack_nibbles(unpacked: &[u8], packed: &mut [u8]) -> Result<()> {
        if unpacked.len() != packed.len() * 2 {
            return Err(LuceneError::IllegalArgument(format!(
                "unpacked length {} must be twice the packed length {}",
                unpacked.len(),
                packed.len()
            )));
        }
        for i in 0..packed.len() {
            let x = (unpacked[i] << 4) | unpacked[packed.len() + i];
            packed[i] = x;
        }
        Ok(())
    }

    /// Expands the striped nibbles [`pack_nibbles`](Self::pack_nibbles) wrote
    /// back to one component per byte.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `unpacked` is not exactly
    /// twice as long as `packed`, which Java asserts.
    ///
    /// Equivalent to `OffHeapScalarQuantizedVectorValues.unpackNibbles`.
    pub fn unpack_nibbles(packed: &[u8], unpacked: &mut [u8]) -> Result<()> {
        if unpacked.len() != packed.len() * 2 {
            return Err(LuceneError::IllegalArgument(format!(
                "unpacked length {} must be twice the packed length {}",
                unpacked.len(),
                packed.len()
            )));
        }
        for i in 0..packed.len() {
            unpacked[i] = (packed[i] >> 4) & 0x0F;
            unpacked[packed.len() + i] = packed[i] & 0x0F;
        }
        Ok(())
    }

    /// Reads the vector at `ord` and its corrections, unless already loaded.
    ///
    /// The on-disk layout of one entry is the packed components, then the lower
    /// and upper interval, then the additional correction, then the component
    /// sum as a short.
    fn ensure_loaded(&mut self, ord: i32) -> Result<()> {
        if self.loaded_ord == ord {
            return Ok(());
        }
        if ord < 0 || ord >= self.entry.size {
            return Err(LuceneError::IllegalArgument(format!(
                "vector ordinal {ord} is out of range for {} vectors",
                self.entry.size
            )));
        }
        // The slice starts at the field's data, so the ordinal indexes it
        // directly, as Java's `slice.seek(targetOrd * byteSize)` does.
        let offset = self.entry.vector_data_offset + i64::from(ord) * self.byte_size as i64;
        self.slice.seek(offset)?;
        let len = self.vector.len();
        self.slice.read_bytes(&mut self.vector, 0, len)?;
        self.corrections = QuantizationResult {
            lower_interval: f32::from_bits(self.slice.read_int()? as u32),
            upper_interval: f32::from_bits(self.slice.read_int()? as u32),
            additional_correction: f32::from_bits(self.slice.read_int()? as u32),
            quantized_component_sum: self.slice.read_int()?,
        };
        self.loaded_ord = ord;
        Ok(())
    }
}

impl QuantizedByteVectorValues for OffHeapScalarQuantizedVectorValues {
    fn vector_value(&mut self, ord: i32) -> Result<&[u8]> {
        self.ensure_loaded(ord)?;
        Ok(&self.vector)
    }

    fn get_score_correction_constant(&mut self, ord: i32) -> Result<f32> {
        self.ensure_loaded(ord)?;
        Ok(self.corrections.additional_correction)
    }

    fn size(&self) -> i32 {
        self.entry.size
    }

    fn dimension(&self) -> i32 {
        self.entry.dimension
    }

    fn get_scalar_encoding(&self) -> ScalarEncoding {
        self.entry.scalar_encoding
    }

    fn get_corrective_terms(&mut self, ord: i32) -> Result<QuantizationResult> {
        OffHeapScalarQuantizedVectorValues::get_corrective_terms(self, ord)
    }

    fn get_centroid(&self) -> Option<&[f32]> {
        OffHeapScalarQuantizedVectorValues::get_centroid(self)
    }

    fn get_centroid_dp(&self) -> f32 {
        OffHeapScalarQuantizedVectorValues::get_centroid_dp(self)
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        if self.entry.ord_to_doc.is_empty() {
            return Ok(Box::new(DenseDocIndexIterator::new(0)));
        }
        if self.entry.ord_to_doc.is_dense() {
            return Ok(Box::new(DenseDocIndexIterator::new(self.entry.size)));
        }
        Ok(Box::new(
            self.entry
                .ord_to_doc
                .get_indexed_disi(self.slice.as_ref())?,
        ))
    }
}

/// Equivalent to `OffHeapScalarQuantizedVectorValues.getSlice()`, which the
/// class inherits from `HasIndexSlice`.
impl HasIndexSlice for OffHeapScalarQuantizedVectorValues {
    fn get_slice(&self) -> Option<&dyn IndexInput> {
        Some(self.slice.as_ref())
    }
}

/// Presents quantized vectors as floats, dequantizing on each read.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.OffHeapScalarQuantizedFloatVectorValues`.
pub struct OffHeapScalarQuantizedFloatVectorValues {
    inner: OffHeapScalarQuantizedVectorValues,
    /// Scratch holding the dequantized vector.
    scratch: Vec<f32>,
}

impl std::fmt::Debug for OffHeapScalarQuantizedFloatVectorValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OffHeapScalarQuantizedFloatVectorValues")
            .finish_non_exhaustive()
    }
}

impl OffHeapScalarQuantizedFloatVectorValues {
    /// Wraps quantized values so they read as floats.
    pub fn new(inner: OffHeapScalarQuantizedVectorValues) -> Self {
        let dimension = inner.dimension().max(0) as usize;
        Self {
            inner,
            scratch: vec![0f32; dimension],
        }
    }

    /// Returns the dequantized vector at `ord`.
    ///
    /// Equivalent to `OffHeapScalarQuantizedFloatVectorValues.vectorValue(int)`.
    pub fn vector_value(&mut self, ord: i32) -> Result<&[f32]> {
        let bits = self.inner.encoding().get_bits();
        let corrections = self.inner.get_corrective_terms(ord)?;
        let centroid = self
            .inner
            .get_centroid()
            .map(|c| c.to_vec())
            .unwrap_or_else(|| vec![0f32; self.scratch.len()]);
        let quantized = self.inner.vector_value(ord)?.to_vec();
        self.scratch =
            OptimizedScalarQuantizer::de_quantize(&quantized, bits, &corrections, &centroid);
        Ok(&self.scratch)
    }

    /// Returns how many vectors the field holds.
    pub fn size(&self) -> i32 {
        self.inner.size()
    }

    /// Returns how many components each vector has.
    pub fn dimension(&self) -> i32 {
        self.inner.dimension()
    }
}

/// Equivalent to `OffHeapScalarQuantizedFloatVectorValues.getSlice()`, which
/// the class inherits from `HasIndexSlice`.
impl HasIndexSlice for OffHeapScalarQuantizedFloatVectorValues {
    fn get_slice(&self) -> Option<&dyn IndexInput> {
        self.inner.get_slice()
    }
}

/// Scores a query against the quantized vectors of one field.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java's scorer chooses between a scalar
/// and a vectorised inner loop through `VectorizationProvider`, and quantizes
/// the query with `OptimizedScalarQuantizer` bound to the field's centroid.
/// This port keeps the scalar loop and takes the already quantized query, since
/// the crate has no vectorisation provider; the scores are the same.
pub struct Lucene104ScalarQuantizedVectorScorer {
    non_quantized_delegate: Arc<dyn FlatVectorsScorer>,
}

impl std::fmt::Debug for Lucene104ScalarQuantizedVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene104ScalarQuantizedVectorScorer")
            .finish_non_exhaustive()
    }
}

impl Lucene104ScalarQuantizedVectorScorer {
    /// Creates the scorer, delegating unquantized fields to `delegate`.
    pub fn new(delegate: Arc<dyn FlatVectorsScorer>) -> Self {
        Self {
            non_quantized_delegate: delegate,
        }
    }

    /// Returns the scorer used for unquantized fields.
    pub fn get_non_quantized_delegate(&self) -> &Arc<dyn FlatVectorsScorer> {
        &self.non_quantized_delegate
    }

    /// Scores an already quantized query against the vectors of a field.
    ///
    /// Equivalent to the body of
    /// `Lucene104ScalarQuantizedVectorScorer`'s random scorer.
    pub fn score(
        &self,
        similarity_function: VectorSimilarityFunction,
        encoding: ScalarEncoding,
        query: &[u8],
        query_corrections: &QuantizationResult,
        values: &mut OffHeapScalarQuantizedVectorValues,
        ord: i32,
    ) -> Result<f32> {
        let similarity = ScalarQuantizedVectorSimilarity::from_vector_similarity(
            similarity_function,
            encoding.get_bits(),
        );
        let corrections = values.get_corrective_terms(ord)?;
        let stored = values.vector_value(ord)?.to_vec();
        // The interval width is the multiplier a quantized dot product carries.
        let points = ((1u32 << encoding.get_bits()) - 1) as f32;
        let query_step =
            (query_corrections.upper_interval - query_corrections.lower_interval) / points;
        let stored_step = (corrections.upper_interval - corrections.lower_interval) / points;
        similarity.score(
            query,
            query_corrections.additional_correction,
            &stored,
            corrections.additional_correction,
            query_step * stored_step,
        )
    }

    /// Scores an already quantized query against the vector stored at
    /// `target_ord`, applying the four corrective terms both sides carry.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the query and the stored
    /// vector have different lengths, and whatever reading the stored vector
    /// and its corrections raises.
    ///
    /// **Divergence from Lucene 10.5.0.** Java dispatches over five
    /// `ScalarEncoding` constants; this crate's `ScalarEncoding` models the
    /// three symmetric ones, so the `SINGLE_BIT_QUERY_NIBBLE` and
    /// `DIBIT_QUERY_NIBBLE` arms — `VectorUtil.int4BitDotProduct` and
    /// `VectorUtil.int4DibitDotProduct`, at
    /// `Lucene104ScalarQuantizedVectorScorer.java:251-253` — have no variant to
    /// match on. Everything else, including the query and document scales taken
    /// from `SCALE_LUT`, is Lucene's arithmetic unchanged.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorScorer#quantizedScore`.
    pub fn quantized_score(
        quantized_query: &[u8],
        query_corrections: &QuantizationResult,
        target_vectors: &mut dyn QuantizedByteVectorValues,
        target_ord: i32,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<f32> {
        let scalar_encoding = target_vectors.get_scalar_encoding();
        let dimension = target_vectors.dimension();
        let centroid_dp = target_vectors.get_centroid_dp();
        let qc_dist = {
            let quantized_doc = target_vectors.vector_value(target_ord)?;
            match scalar_encoding {
                ScalarEncoding::UnsignedByte => {
                    vector_util::uint8_dot_product(quantized_query, quantized_doc)? as f32
                }
                ScalarEncoding::SevenBit => {
                    vector_util::dot_product_bytes(quantized_query, quantized_doc)? as f32
                }
                ScalarEncoding::PackedNibble => {
                    vector_util::int4_dot_product_single_packed(quantized_query, quantized_doc)?
                        as f32
                }
            }
        };
        let index_corrections = target_vectors.get_corrective_terms(target_ord)?;
        let query_scale = SCALE_LUT[usize::from(scalar_encoding.get_query_bits()) - 1];
        let scale = SCALE_LUT[usize::from(scalar_encoding.get_bits()) - 1];
        let x1 = index_corrections.quantized_component_sum as f32;
        let ax = index_corrections.lower_interval;
        // Here we must scale according to the bits.
        let lx = (index_corrections.upper_interval - ax) * scale;
        let ay = query_corrections.lower_interval;
        let ly = (query_corrections.upper_interval - ay) * query_scale;
        let y1 = query_corrections.quantized_component_sum as f32;
        let mut score =
            ax * ay * dimension as f32 + ay * lx * x1 + ax * ly * y1 + lx * ly * qc_dist;
        // For euclidean, we need to invert the score and apply the additional
        // correction, which is assumed to be the squared l2norm of the centroid
        // centered vectors.
        if similarity_function == VectorSimilarityFunction::EUCLIDEAN {
            score = query_corrections.additional_correction
                + index_corrections.additional_correction
                - 2.0 * score;
            // Ensure that 'score' (the squared euclidean distance) is
            // non-negative. The computed value may be negative as a result of
            // quantization loss.
            Ok(1.0 / (1.0 + java_math_max(score, 0.0)))
        } else {
            // For cosine and max inner product, we need to apply the additional
            // correction, which is assumed to be the non-centered dot-product
            // between the vector and the centroid.
            score += query_corrections.additional_correction
                + index_corrections.additional_correction
                - centroid_dp;
            if similarity_function == VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT {
                return Ok(vector_util::scale_max_inner_product_score(score));
            }
            // Ensure that 'score' (a normalized dot product) is in [-1,1]. The
            // computed value may be out of bounds as a result of quantization
            // loss.
            score = score.clamp(-1.0, 1.0);
            Ok((1.0 + score) / 2.0)
        }
    }

    /// Returns a supplier of scorers over one field's quantized vectors.
    ///
    /// # Errors
    ///
    /// Returns whatever copying the values raises.
    ///
    /// **Divergence from Lucene 10.5.0.** Java declares
    /// `getRandomVectorScorerSupplier(VectorSimilarityFunction, KnnVectorValues)`
    /// and narrows to `QuantizedByteVectorValues` with `instanceof`, falling
    /// back to the unquantized delegate otherwise. Rust has no `instanceof`, so
    /// the quantized overload takes the quantized values directly; the
    /// unquantized case is the delegate's own
    /// [`FlatVectorsScorer`] methods, reached through
    /// [`get_non_quantized_delegate`](Self::get_non_quantized_delegate).
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorScorer.getRandomVectorScorerSupplier(VectorSimilarityFunction, KnnVectorValues)`.
    pub fn get_random_vector_scorer_supplier_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: OffHeapScalarQuantizedVectorValues,
    ) -> Result<ScalarQuantizedVectorScorerSupplier> {
        ScalarQuantizedVectorScorerSupplier::new(vector_values, similarity_function)
    }
}

/// `1 / ((1 << bits) - 1)` for every width a component can be quantized to,
/// indexed by `bits - 1`.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorScorer.SCALE_LUT`.
const SCALE_LUT: [f32; 8] = [
    1.0,
    1.0 / ((1 << 2) - 1) as f32,
    1.0 / ((1 << 3) - 1) as f32,
    1.0 / ((1 << 4) - 1) as f32,
    1.0 / ((1 << 5) - 1) as f32,
    1.0 / ((1 << 6) - 1) as f32,
    1.0 / ((1 << 7) - 1) as f32,
    1.0 / ((1 << 8) - 1) as f32,
];

/// `Math.max(float, float)` as specified by the JLS.
///
/// `f32::max` returns the non-NaN operand when exactly one operand is NaN,
/// whereas Java propagates NaN; a score that quantization turned into NaN must
/// stay NaN here, as it does in Lucene.
#[inline]
fn java_math_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// Supplies scorers over one field's quantized vectors, for the symmetric
/// encodings.
///
/// Equivalent to the private
/// `Lucene104ScalarQuantizedVectorScorer.ScalarQuantizedVectorScorerSupplier`.
///
/// **Divergence from Lucene 10.5.0.** Java's supplier is not closeable and is
/// wrapped by `CloseableRandomVectorScorerSupplier.create(supplier, size,
/// () -> {})` when a merge asks for one; that wrapper adds nothing but the
/// vector count and an empty clean-up, so this port implements
/// [`CloseableRandomVectorScorerSupplier`] on the supplier itself.
pub struct ScalarQuantizedVectorScorerSupplier {
    target_values: OffHeapScalarQuantizedVectorValues,
    values: OffHeapScalarQuantizedVectorValues,
    similarity: VectorSimilarityFunction,
}

impl std::fmt::Debug for ScalarQuantizedVectorScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarQuantizedVectorScorerSupplier")
            .field("similarity", &self.similarity)
            .finish_non_exhaustive()
    }
}

impl ScalarQuantizedVectorScorerSupplier {
    /// Creates a supplier over `values`, which must use a symmetric encoding.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the encoding is asymmetric,
    /// which Java asserts, and whatever copying the values raises.
    ///
    /// Equivalent to the
    /// `ScalarQuantizedVectorScorerSupplier(QuantizedByteVectorValues, VectorSimilarityFunction)`
    /// constructor.
    pub fn new(
        values: OffHeapScalarQuantizedVectorValues,
        similarity: VectorSimilarityFunction,
    ) -> Result<Self> {
        if values.get_scalar_encoding().is_asymmetric() {
            return Err(LuceneError::IllegalState(format!(
                "{:?} encoding is not supported for symmetric quantization",
                values.get_scalar_encoding()
            )));
        }
        let target_values = values.copy()?;
        Ok(Self {
            target_values,
            values,
            similarity,
        })
    }
}

impl RandomVectorScorerSupplier for ScalarQuantizedVectorScorerSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        Ok(Box::new(ScalarQuantizedUpdateableScorer {
            target_values: self.target_values.copy()?,
            values: self.values.copy()?,
            similarity: self.similarity,
            target_vector: None,
            target_corrective_terms: None,
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::new(self.values.copy()?, self.similarity)?))
    }
}

impl CloseableRandomVectorScorerSupplier for ScalarQuantizedVectorScorerSupplier {
    fn total_vector_count(&self) -> i32 {
        self.values.size()
    }
}

/// The scorer [`ScalarQuantizedVectorScorerSupplier`] hands out: it scores the
/// vector last selected with
/// [`set_scoring_ordinal`](UpdateableRandomVectorScorer::set_scoring_ordinal)
/// against every other vector of the field.
///
/// Equivalent to the anonymous
/// `UpdateableRandomVectorScorer.AbstractUpdateableRandomVectorScorer` that
/// `ScalarQuantizedVectorScorerSupplier.scorer()` returns.
struct ScalarQuantizedUpdateableScorer {
    target_values: OffHeapScalarQuantizedVectorValues,
    values: OffHeapScalarQuantizedVectorValues,
    similarity: VectorSimilarityFunction,
    target_vector: Option<Vec<u8>>,
    target_corrective_terms: Option<QuantizationResult>,
}

impl RandomVectorScorer for ScalarQuantizedUpdateableScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let (Some(target_vector), Some(target_corrective_terms)) =
            (self.target_vector.as_ref(), self.target_corrective_terms)
        else {
            return Err(LuceneError::IllegalState(
                "setScoringOrdinal was not called".to_string(),
            ));
        };
        Lucene104ScalarQuantizedVectorScorer::quantized_score(
            target_vector,
            &target_corrective_terms,
            &mut self.values,
            node,
            self.similarity,
        )
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }
}

impl UpdateableRandomVectorScorer for ScalarQuantizedUpdateableScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let raw_target_vector = self.target_values.vector_value(node)?.to_vec();
        match self.values.get_scalar_encoding() {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => {
                self.target_vector = Some(raw_target_vector);
            }
            ScalarEncoding::PackedNibble => {
                let width = OptimizedScalarQuantizer::discretize(self.values.dimension(), 2).max(0)
                    as usize;
                let mut unpacked = vec![0u8; width];
                OffHeapScalarQuantizedVectorValues::unpack_nibbles(
                    &raw_target_vector,
                    &mut unpacked,
                )?;
                self.target_vector = Some(unpacked);
            }
        }
        self.target_corrective_terms = Some(self.target_values.get_corrective_terms(node)?);
        Ok(())
    }
}

/// A scorer bound to one query and one field's quantized vectors.
pub struct Lucene104ScalarQuantizedRandomVectorScorer {
    scorer: Lucene104ScalarQuantizedVectorScorer,
    similarity_function: VectorSimilarityFunction,
    encoding: ScalarEncoding,
    query: Vec<u8>,
    query_corrections: QuantizationResult,
    values: OffHeapScalarQuantizedVectorValues,
}

impl std::fmt::Debug for Lucene104ScalarQuantizedRandomVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene104ScalarQuantizedRandomVectorScorer")
            .finish_non_exhaustive()
    }
}

impl Lucene104ScalarQuantizedRandomVectorScorer {
    /// Binds `scorer` to a quantized query and a field's vectors.
    pub fn new(
        scorer: Lucene104ScalarQuantizedVectorScorer,
        similarity_function: VectorSimilarityFunction,
        encoding: ScalarEncoding,
        query: Vec<u8>,
        query_corrections: QuantizationResult,
        values: OffHeapScalarQuantizedVectorValues,
    ) -> Self {
        Self {
            scorer,
            similarity_function,
            encoding,
            query,
            query_corrections,
            values,
        }
    }
}

impl RandomVectorScorer for Lucene104ScalarQuantizedRandomVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        self.scorer.score(
            self.similarity_function,
            self.encoding,
            &self.query,
            &self.query_corrections,
            &mut self.values,
            node,
        )
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }
}

/// Reads the quantized vectors of a segment.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader`.
///
/// **Divergence from Lucene 10.5.0.** Java's reader also implements the full
/// `FlatVectorsReader` surface, forwarding the raw float vectors to a wrapped
/// `Lucene99FlatVectorsReader` and answering searches from the quantized form.
/// This port reads the metadata and hands out the quantized values and their
/// scorer; the `FlatVectorsReader` surface stays with the raw delegate, because
/// the search path this format plugs into is not ported yet.
pub struct Lucene104ScalarQuantizedVectorsReader {
    /// One entry per field that has quantized vectors, by field name.
    fields: std::collections::BTreeMap<String, FieldEntry>,
    vector_data: Arc<dyn IndexInput>,
    scorer: Arc<Lucene104ScalarQuantizedVectorScorer>,
}

impl std::fmt::Debug for Lucene104ScalarQuantizedVectorsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene104ScalarQuantizedVectorsReader")
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}

impl Lucene104ScalarQuantizedVectorsReader {
    /// Builds a reader over the metadata already read from the `.vemq` file.
    pub fn new(
        fields: std::collections::BTreeMap<String, FieldEntry>,
        vector_data: Arc<dyn IndexInput>,
        scorer: Arc<Lucene104ScalarQuantizedVectorScorer>,
    ) -> Self {
        Self {
            fields,
            vector_data,
            scorer,
        }
    }

    /// Reads the per-field entries of a `.vemq` metadata file.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.readFields`. The
    /// list ends with the sentinel field number `-1`.
    pub fn read_fields(
        meta: &mut dyn IndexInput,
        field_infos: &crate::index::FieldInfos,
    ) -> Result<std::collections::BTreeMap<String, FieldEntry>> {
        let mut fields = std::collections::BTreeMap::new();
        loop {
            let field_number = meta.read_int()?;
            if field_number == -1 {
                break;
            }
            let info = field_infos
                .field_info_by_number(field_number)
                .ok_or_else(|| {
                    LuceneError::corrupt_index(
                        format!("invalid field number: {field_number}"),
                        "quantized vectors metadata",
                    )
                })?
                .clone();

            let field_entry = Self::read_field(meta, &info)?;
            Self::validate_field_entry(&info, &field_entry)?;
            fields.insert(info.name.clone(), field_entry);
        }
        Ok(fields)
    }

    /// Reads one field's entry, checking that the similarity it was written
    /// with is the one the field carries.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the encoding or similarity
    /// ordinal is not one Lucene writes, [`LuceneError::IllegalState`] when the
    /// similarity disagrees with the field info, and whatever reading the entry
    /// raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#readField`.
    pub fn read_field(input: &mut dyn IndexInput, info: &FieldInfo) -> Result<FieldEntry> {
        let encoding_ordinal = input.read_int()?;
        let vector_encoding = match encoding_ordinal {
            0 => VectorEncoding::BYTE,
            1 => VectorEncoding::FLOAT32,
            other => {
                return Err(LuceneError::corrupt_index(
                    format!("invalid vector encoding ordinal: {other}"),
                    "quantized vectors metadata",
                ))
            }
        };
        let similarity_ordinal = input.read_int()?;
        let similarity_function = match similarity_ordinal {
            0 => VectorSimilarityFunction::EUCLIDEAN,
            1 => VectorSimilarityFunction::DOT_PRODUCT,
            2 => VectorSimilarityFunction::COSINE,
            3 => VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
            other => {
                return Err(LuceneError::corrupt_index(
                    format!("invalid vector similarity ordinal: {other}"),
                    "quantized vectors metadata",
                ))
            }
        };
        if similarity_function != info.vector_similarity_function {
            return Err(LuceneError::IllegalState(format!(
                "Inconsistent vector similarity function for field=\"{}\"; {:?} != {:?}",
                info.name, similarity_function, info.vector_similarity_function
            )));
        }
        FieldEntry::create(input, vector_encoding, info.vector_similarity_function)
    }

    /// Checks that a field's entry agrees with the field info: the same
    /// dimension, and a data length that is exactly the vector count times the
    /// stride one quantized vector occupies.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when either check fails, which is
    /// Java's `IllegalStateException`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#validateFieldEntry`.
    pub fn validate_field_entry(info: &FieldInfo, field_entry: &FieldEntry) -> Result<()> {
        let dimension = info.vector_dimension;
        if dimension != field_entry.dimension {
            return Err(LuceneError::IllegalState(format!(
                "Inconsistent vector dimension for field=\"{}\"; {} != {}",
                info.name, dimension, field_entry.dimension
            )));
        }

        let stride = i64::from(field_entry.scalar_encoding.get_doc_packed_length(dimension))
            + (std::mem::size_of::<f32>() as i64) * 3
            + std::mem::size_of::<i32>() as i64;
        let num_quantized_vector_bytes = stride
            .checked_mul(i64::from(field_entry.size))
            .ok_or_else(|| {
                LuceneError::IllegalState(format!(
                    "vector data length overflows for field=\"{}\"",
                    info.name
                ))
            })?;
        if num_quantized_vector_bytes != field_entry.vector_data_length {
            return Err(LuceneError::IllegalState(format!(
                "vector data length {} not matching size = {} * (dims={} + 16) = {}",
                field_entry.vector_data_length,
                field_entry.size,
                dimension,
                num_quantized_vector_bytes
            )));
        }
        Ok(())
    }

    /// Opens one of the format's data files and checks that its header matches
    /// the version the metadata file declared.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the header or the version
    /// disagrees with the metadata, and an I/O error when the file cannot be
    /// opened.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#openDataInput`.
    pub fn open_data_input(
        state: &SegmentReadState<'_>,
        version_meta: i32,
        file_extension: &str,
        codec_name: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexInput>> {
        let file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            file_extension,
        );
        let mut input = state.directory.open_input(&file_name, context)?;
        let version_vector_data = check_index_header(
            input.as_mut(),
            codec_name,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        if version_meta != version_vector_data {
            return Err(LuceneError::corrupt_index(
                format!(
                    "Format versions mismatch: meta={version_meta}, {codec_name}={version_vector_data}"
                ),
                &file_name,
            ));
        }
        retrieve_checksum(input.as_mut())?;
        Ok(input)
    }

    /// Returns the quantized vectors of `field`.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsReader.getQuantizedVectorValues`.
    pub fn get_quantized_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<OffHeapScalarQuantizedVectorValues>> {
        let Some(entry) = self.fields.get(field) else {
            return Ok(None);
        };
        if entry.vector_encoding != VectorEncoding::FLOAT32 {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{field}\" is encoded as {:?}, expected FLOAT32",
                entry.vector_encoding
            )));
        }
        let slice = self.vector_data.clone_input()?;
        Ok(Some(OffHeapScalarQuantizedVectorValues::load(
            entry.clone(),
            slice,
        )))
    }

    /// Returns the metadata entry of `field`.
    pub fn field_entry(&self, field: &str) -> Option<&FieldEntry> {
        self.fields.get(field)
    }

    /// Returns the scorer this reader uses.
    pub fn scorer(&self) -> &Arc<Lucene104ScalarQuantizedVectorScorer> {
        &self.scorer
    }

    /// Returns a supplier of scorers over `field_info`'s quantized vectors, to
    /// score candidates while a merge builds the new graph, or `None` when the
    /// segment has no quantized vectors for that field.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] when the field uses an
    /// asymmetric encoding — see the divergence below — and whatever opening
    /// the values raises.
    ///
    /// **Divergence from Lucene 10.5.0.** Java has a second branch, taken when
    /// `fi.scalarEncoding.isAsymmetric()`, that quantizes every vector a second
    /// time at the query width into a temporary file and scores against that
    /// (`Lucene104ScalarQuantizedVectorsReader.java:467-522`). It needs
    /// `OffHeapScalarQuantizedVectorValues.DenseOffHeapVectorValues` over
    /// query-packed data, which this port does not have, and it is unreachable
    /// here in any case: every constant of this crate's `ScalarEncoding`
    /// quantizes queries and documents to the same width, so
    /// [`is_asymmetric`](ScalarEncoding::is_asymmetric) is always false. The
    /// branch therefore reports the missing piece rather than guessing.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#getRandomVectorScorerSupplierForMerge`.
    pub fn get_random_vector_scorer_supplier_for_merge(
        &self,
        field_info: &FieldInfo,
        segment_write_state: &SegmentWriteState<'_>,
    ) -> Result<Option<Box<dyn CloseableRandomVectorScorerSupplier>>> {
        let Some(fi) = self.fields.get(&field_info.name) else {
            return Ok(None);
        };
        let scalar_encoding = fi.scalar_encoding;
        let Some(vector_values) = self.get_quantized_vector_values(&field_info.name)? else {
            return Ok(None);
        };
        if !scalar_encoding.is_asymmetric() {
            let supplier = self.scorer.get_random_vector_scorer_supplier_quantized(
                field_info.vector_similarity_function,
                vector_values,
            )?;
            return Ok(Some(Box::new(supplier)));
        }
        let _ = segment_write_state;
        Err(LuceneError::UnsupportedOperation(format!(
            "asymmetric merge scoring for field=\"{}\" needs a query-packed \
             OffHeapScalarQuantizedVectorValues, which is not ported",
            field_info.name
        )))
    }

    /// Quantizes every vector of a field a second time, at the *query* width,
    /// into `binarized_query_data`, returning the documents it wrote.
    ///
    /// Only an asymmetric encoding — one that quantizes queries more finely
    /// than documents — has a second width to write, so this refuses a
    /// symmetric one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `encoding` is symmetric,
    /// [`LuceneError::IllegalState`] when the field stores no centroid, and
    /// whatever reading the vectors or writing the output raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#writeBinarizedQueryData`.
    pub fn write_binarized_query_data(
        quantized_byte_vector_values: &dyn QuantizedByteVectorValues,
        encoding: ScalarEncoding,
        binarized_query_data: &mut dyn IndexOutput,
        float_vector_values: &dyn FloatVectorValues,
        binary_quantizer: &OptimizedScalarQuantizer,
    ) -> Result<DocsWithFieldSet> {
        if !encoding.is_asymmetric() {
            return Err(LuceneError::IllegalArgument(
                "encoding and queryEncoding must be different".to_string(),
            ));
        }
        let centroid = quantized_byte_vector_values.get_centroid().ok_or_else(|| {
            LuceneError::IllegalState(
                "quantized vectors have no centroid to quantize the queries against".to_string(),
            )
        })?;
        let mut docs_with_field = DocsWithFieldSet::new();
        let discretized_dims = encoding.get_discrete_dimensions(float_vector_values.dimension());
        let mut quantization_scratch = vec![0u8; discretized_dims.max(0) as usize];
        let mut to_query =
            vec![0u8; encoding.get_query_packed_length(discretized_dims).max(0) as usize];
        let mut iterator = float_vector_values.iterator()?;
        loop {
            let doc_v = iterator.next_doc()?;
            if doc_v == NO_MORE_DOCS {
                break;
            }
            // write index vector
            let mut vector = float_vector_values.vector_value(iterator.index())?;
            let r = binary_quantizer.scalar_quantize(
                &mut vector,
                &mut quantization_scratch,
                encoding.get_query_bits(),
                centroid,
            )?;
            docs_with_field.add(doc_v)?;
            // pack and store the 4bit query vector
            OptimizedScalarQuantizer::transpose_half_byte(&quantization_scratch, &mut to_query);
            binarized_query_data.write_bytes(&to_query, 0, to_query.len())?;
            binarized_query_data.write_int(r.lower_interval.to_bits() as i32)?;
            binarized_query_data.write_int(r.upper_interval.to_bits() as i32)?;
            binarized_query_data.write_int(r.additional_correction.to_bits() as i32)?;
            binarized_query_data.write_int(r.quantized_component_sum)?;
        }
        Ok(docs_with_field)
    }
}

impl QuantizedVectorsReader for Lucene104ScalarQuantizedVectorsReader {
    /// Returns the quantized vectors of `field`.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsReader.getQuantizedVectorValues(String)`.
    fn get_quantized_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn QuantizedByteVectorValues>>> {
        Ok(
            Lucene104ScalarQuantizedVectorsReader::get_quantized_vector_values(self, field)?
                .map(|values| Box::new(values) as Box<dyn QuantizedByteVectorValues>),
        )
    }

    /// Returns no quantizer: the 10.4 format stores the interval per vector
    /// rather than one `ScalarQuantizer` per field, so there is no state of
    /// that shape to hand back.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#getQuantizationState`.
    fn get_quantization_state(&self, _field_name: &str) -> Result<Option<ScalarQuantizer>> {
        Ok(None)
    }
}

/// The scalar-quantized vectors format.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsFormat`.
///
/// **Divergence from Lucene 10.5.0.** Java's format extends `FlatVectorsFormat`
/// and builds a reader and a writer that wrap a `Lucene99FlatVectorsFormat`
/// delegate. This port carries the format's identity, its file names and its
/// encoding choice, and reads the metadata through
/// [`Lucene104ScalarQuantizedVectorsReader`]; plugging it in as a
/// `FlatVectorsFormat` needs the writer half, which is the remaining piece.
#[derive(Clone, Copy, Debug)]
pub struct Lucene104ScalarQuantizedVectorsFormat {
    encoding: ScalarEncoding,
}

impl Default for Lucene104ScalarQuantizedVectorsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl Lucene104ScalarQuantizedVectorsFormat {
    /// Creates the format with Lucene's default one-byte encoding.
    pub fn new() -> Self {
        Self {
            encoding: ScalarEncoding::UnsignedByte,
        }
    }

    /// Creates the format with an explicit encoding.
    pub fn with_encoding(encoding: ScalarEncoding) -> Self {
        Self { encoding }
    }

    /// Returns the SPI name of the format.
    pub fn name(&self) -> &str {
        NAME
    }

    /// Returns how the components are packed.
    pub fn encoding(&self) -> ScalarEncoding {
        self.encoding
    }

    /// Returns the largest dimension the format accepts.
    ///
    /// Equivalent to `getMaxDimensions(String)`.
    pub fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        MAX_DIMS
    }

    /// Returns the quantizer this format uses for `similarity_function`.
    pub fn quantizer(
        &self,
        similarity_function: VectorSimilarityFunction,
    ) -> OptimizedScalarQuantizer {
        OptimizedScalarQuantizer::new(similarity_function)
    }
}

impl std::fmt::Display for Lucene104ScalarQuantizedVectorsFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lucene104ScalarQuantizedVectorsFormat(name={NAME}, encoding={:?})",
            self.encoding
        )
    }
}

/// An HNSW format whose vectors are scalar-quantized.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104HnswScalarQuantizedVectorsFormat`,
/// which pairs the graph parameters of the HNSW format with the quantized flat
/// format above.
#[derive(Clone, Copy, Debug)]
pub struct Lucene104HnswScalarQuantizedVectorsFormat {
    max_conn: i32,
    beam_width: i32,
    flat: Lucene104ScalarQuantizedVectorsFormat,
}

/// Default number of connections per graph node.
pub const DEFAULT_MAX_CONN: i32 = 16;
/// Default beam width used while building the graph.
pub const DEFAULT_BEAM_WIDTH: i32 = 100;

impl Default for Lucene104HnswScalarQuantizedVectorsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl Lucene104HnswScalarQuantizedVectorsFormat {
    /// Creates the format with Lucene's default graph parameters.
    pub fn new() -> Self {
        Self {
            max_conn: DEFAULT_MAX_CONN,
            beam_width: DEFAULT_BEAM_WIDTH,
            flat: Lucene104ScalarQuantizedVectorsFormat::new(),
        }
    }

    /// Creates the format with explicit graph parameters and encoding.
    pub fn with_params(max_conn: i32, beam_width: i32, encoding: ScalarEncoding) -> Result<Self> {
        if max_conn <= 0 || max_conn > 512 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxConn must be in [1, 512], got {max_conn}"
            )));
        }
        if beam_width <= 0 || beam_width > 3200 {
            return Err(LuceneError::IllegalArgument(format!(
                "beamWidth must be in [1, 3200], got {beam_width}"
            )));
        }
        Ok(Self {
            max_conn,
            beam_width,
            flat: Lucene104ScalarQuantizedVectorsFormat::with_encoding(encoding),
        })
    }

    /// Returns how many connections each graph node keeps.
    pub fn max_conn(&self) -> i32 {
        self.max_conn
    }

    /// Returns the beam width used while building the graph.
    pub fn beam_width(&self) -> i32 {
        self.beam_width
    }

    /// Returns the flat format storing the quantized vectors.
    pub fn flat_format(&self) -> &Lucene104ScalarQuantizedVectorsFormat {
        &self.flat
    }

    /// Returns the largest dimension the format accepts.
    pub fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        MAX_DIMS
    }
}

/// Writes the quantized vectors of a segment.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsWriter`.
///
/// **Divergence from Lucene 10.5.0.** Java's writer is a `FlatVectorsWriter`
/// that buffers each field's vectors through a delegate, and on flush computes
/// the centroid, quantizes, and writes both files. This port carries the two
/// steps that define the file format — computing the centroid and writing the
/// vectors and the metadata — and takes the buffered vectors from the caller,
/// because the `FlatVectorsWriter` buffering surface is a separate piece.
pub struct Lucene104ScalarQuantizedVectorsWriter {
    encoding: ScalarEncoding,
}

impl std::fmt::Debug for Lucene104ScalarQuantizedVectorsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene104ScalarQuantizedVectorsWriter")
            .field("encoding", &self.encoding)
            .finish()
    }
}

impl Lucene104ScalarQuantizedVectorsWriter {
    /// Creates a writer using `encoding`.
    pub fn new(encoding: ScalarEncoding) -> Self {
        Self { encoding }
    }

    /// Computes the centroid every vector of a field is quantized against: the
    /// componentwise mean, normalised when the similarity is cosine.
    ///
    /// Equivalent to the cluster-centre computation in
    /// `Lucene104ScalarQuantizedVectorsWriter.flush`.
    pub fn cluster_center(
        vectors: &[Vec<f32>],
        dimension: usize,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Vec<f32>> {
        let mut center = vec![0f32; dimension];
        if vectors.is_empty() {
            return Ok(center);
        }
        for vector in vectors {
            for (i, &v) in vector.iter().enumerate() {
                center[i] += v;
            }
        }
        let count = vectors.len() as f32;
        for slot in center.iter_mut() {
            *slot /= count;
        }
        if similarity_function == VectorSimilarityFunction::COSINE {
            crate::util::vector_util::l2normalize(&mut center, true)?;
        }
        Ok(center)
    }

    /// Writes one field's quantized vectors into the data file.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.writeVectors`. Each
    /// entry is the packed components, then the lower and upper interval, the
    /// additional correction, and the component sum.
    pub fn write_vectors(
        &self,
        vectors: &mut [Vec<f32>],
        cluster_center: &[f32],
        quantizer: &OptimizedScalarQuantizer,
        vector_data: &mut dyn crate::store::IndexOutput,
    ) -> Result<()> {
        let bits = self.encoding.get_bits();
        let dimension = cluster_center.len() as i32;
        let mut scratch =
            vec![0u8; self.encoding.get_discrete_dimensions(dimension).max(0) as usize];
        let mut packed = match self.encoding {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => Vec::new(),
            ScalarEncoding::PackedNibble => vec![
                0u8;
                self.encoding
                    .get_doc_packed_length(scratch.len() as i32)
                    .max(0) as usize
            ],
        };
        for vector in vectors.iter_mut() {
            let corrections =
                quantizer.scalar_quantize(vector, &mut scratch, bits, cluster_center)?;
            let written = match self.encoding {
                ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => &scratch[..],
                ScalarEncoding::PackedNibble => {
                    OffHeapScalarQuantizedVectorValues::pack_nibbles(&scratch, &mut packed)?;
                    &packed[..]
                }
            };

            vector_data.write_bytes(written, 0, written.len())?;
            vector_data.write_int(corrections.lower_interval.to_bits() as i32)?;
            vector_data.write_int(corrections.upper_interval.to_bits() as i32)?;
            vector_data.write_int(corrections.additional_correction.to_bits() as i32)?;
            vector_data.write_int(corrections.quantized_component_sum)?;
        }
        Ok(())
    }

    /// Writes one field's quantized vectors into the data file in the order
    /// `ord_map` gives, which is the order an index sort put their documents
    /// in: `ord_map[n]` is the old ordinal of the vector that becomes ordinal
    /// `n`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when an entry of `ord_map` is
    /// out of range, and whatever quantizing or writing raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsWriter#writeSortedVectors`.
    pub fn write_sorted_vectors(
        &self,
        vectors: &mut [Vec<f32>],
        cluster_center: &[f32],
        ord_map: &[i32],
        scalar_quantizer: &OptimizedScalarQuantizer,
        vector_data: &mut dyn IndexOutput,
    ) -> Result<()> {
        let bits = self.encoding.get_bits();
        let dimension = cluster_center.len() as i32;
        let mut scratch =
            vec![0u8; self.encoding.get_discrete_dimensions(dimension).max(0) as usize];
        let mut packed = match self.encoding {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => Vec::new(),
            ScalarEncoding::PackedNibble => vec![
                0u8;
                self.encoding
                    .get_doc_packed_length(scratch.len() as i32)
                    .max(0) as usize
            ],
        };
        let count = vectors.len();
        for &ordinal in ord_map {
            let index = usize::try_from(ordinal).unwrap_or(usize::MAX);
            if index >= count {
                return Err(LuceneError::IllegalArgument(format!(
                    "ordinal {ordinal} is out of range for {count} vectors"
                )));
            }
            let vector = &mut vectors[index];
            let corrections =
                scalar_quantizer.scalar_quantize(vector, &mut scratch, bits, cluster_center)?;
            let written = match self.encoding {
                ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => &scratch[..],
                ScalarEncoding::PackedNibble => {
                    OffHeapScalarQuantizedVectorValues::pack_nibbles(&scratch, &mut packed)?;
                    &packed[..]
                }
            };

            vector_data.write_bytes(written, 0, written.len())?;
            vector_data.write_int(corrections.lower_interval.to_bits() as i32)?;
            vector_data.write_int(corrections.upper_interval.to_bits() as i32)?;
            vector_data.write_int(corrections.additional_correction.to_bits() as i32)?;
            vector_data.write_int(corrections.quantized_component_sum)?;
        }
        Ok(())
    }

    /// Writes one field of a sorted segment: the vectors in the new ordinal
    /// order, then the field's metadata entry under the new document numbering.
    ///
    /// # Errors
    ///
    /// Returns whatever mapping the ordinals, quantizing, or writing raises.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads the buffered vectors and
    /// the document set from a `FieldWriter`, which this port does not have —
    /// its writer takes both from the caller, exactly as
    /// [`write_vectors`](Self::write_vectors) already does.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsWriter#writeSortingField`.
    #[allow(clippy::too_many_arguments)]
    pub fn write_sorting_field(
        &self,
        vectors: &mut [Vec<f32>],
        field: &FieldInfo,
        max_doc: i32,
        sort_map: &SorterDocMap,
        docs_with_field: &DocsWithFieldSet,
        cluster_center: &[f32],
        scalar_quantizer: &OptimizedScalarQuantizer,
        meta: &mut dyn IndexOutput,
        vector_data: &mut dyn IndexOutput,
    ) -> Result<()> {
        let old_doc_ids = docs_with_field_to_docs(docs_with_field)?;
        // new ord to old ord
        let (_old_to_new, ord_map) =
            crate::codecs::knn_vectors::map_old_ord_to_new_ord(&old_doc_ids, sort_map)?;
        let new_docs_with_field = new_docs_with_field(&old_doc_ids, sort_map)?;

        // write vector values
        let vector_data_offset =
            vector_data.align_file_pointer(std::mem::size_of::<f32>() as i32)?;
        self.write_sorted_vectors(
            vectors,
            cluster_center,
            &ord_map,
            scalar_quantizer,
            vector_data,
        )?;
        let quantized_vector_length = vector_data.file_pointer() - vector_data_offset;

        let centroid_dp = vector_util::dot_product_f32(cluster_center, cluster_center)?;
        self.write_meta(
            field,
            max_doc,
            vector_data_offset,
            quantized_vector_length,
            cluster_center,
            centroid_dp,
            new_docs_with_field.cardinality(),
            &new_docs_with_field,
            meta,
            vector_data,
        )
    }

    /// Writes one field's entry into the metadata file.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.writeMeta`.
    #[allow(clippy::too_many_arguments)]
    pub fn write_meta(
        &self,
        field: &crate::index::FieldInfo,
        max_doc: i32,
        vector_data_offset: i64,
        vector_data_length: i64,
        cluster_center: &[f32],
        centroid_dp: f32,
        count: i32,
        docs_with_field: &crate::codecs::hnsw::flat_vectors::DocsWithFieldSet,
        meta: &mut dyn crate::store::IndexOutput,
        vector_data: &mut dyn crate::store::IndexOutput,
    ) -> Result<()> {
        meta.write_int(field.number)?;
        meta.write_int(match field.vector_encoding {
            VectorEncoding::BYTE => 0,
            VectorEncoding::FLOAT32 => 1,
        })?;
        meta.write_int(match field.vector_similarity_function {
            VectorSimilarityFunction::EUCLIDEAN => 0,
            VectorSimilarityFunction::DOT_PRODUCT => 1,
            VectorSimilarityFunction::COSINE => 2,
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => 3,
        })?;
        meta.write_v_int(field.vector_dimension)?;
        meta.write_v_long(vector_data_offset)?;
        meta.write_v_long(vector_data_length)?;
        meta.write_v_int(count)?;

        if count > 0 {
            meta.write_v_int(self.encoding.code())?;
            // The centroid is written as little-endian floats.
            for &c in cluster_center {
                meta.write_int(c.to_bits() as i32)?;
            }
            meta.write_int(centroid_dp.to_bits() as i32)?;
        }

        OrdToDocDISIReaderConfiguration::write_stored_meta(
            meta,
            vector_data,
            count,
            max_doc,
            docs_with_field,
        )
    }

    /// Writes the end-of-fields marker.
    ///
    /// Equivalent to the `meta.writeInt(-1)` in
    /// `Lucene104ScalarQuantizedVectorsWriter.finish`.
    pub fn finish(&self, meta: &mut dyn crate::store::IndexOutput) -> Result<()> {
        meta.write_int(-1)
    }

    /// Copies already quantized vectors straight into `output`, one entry per
    /// document that has a vector, and returns those documents.
    ///
    /// This is the merge path: the vectors arrive quantized, so nothing is
    /// re-quantized here.
    ///
    /// # Errors
    ///
    /// Returns whatever reading the values or writing the output raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsWriter#writeVectorData`.
    pub fn write_vector_data(
        output: &mut dyn IndexOutput,
        quantized_byte_vector_values: &mut dyn QuantizedByteVectorValues,
    ) -> Result<DocsWithFieldSet> {
        let mut docs_with_field = DocsWithFieldSet::new();
        let mut iterator = quantized_byte_vector_values.iterator()?;
        loop {
            let doc_v = iterator.next_doc()?;
            if doc_v == NO_MORE_DOCS {
                break;
            }
            let index = iterator.index();
            // write vector
            let binary_value = quantized_byte_vector_values.vector_value(index)?.to_vec();
            output.write_bytes(&binary_value, 0, binary_value.len())?;
            let corrections = quantized_byte_vector_values.get_corrective_terms(index)?;
            output.write_int(corrections.lower_interval.to_bits() as i32)?;
            output.write_int(corrections.upper_interval.to_bits() as i32)?;
            output.write_int(corrections.additional_correction.to_bits() as i32)?;
            output.write_int(corrections.quantized_component_sum)?;
            docs_with_field.add(doc_v)?;
        }
        Ok(docs_with_field)
    }

    /// Returns the centroid a reader already computed for `field_name`, or
    /// `None` when it has none.
    ///
    /// **Divergence from Lucene 10.5.0.** Java narrows the reader with two
    /// `instanceof` tests — through `PerFieldKnnVectorsFormat.FieldsReader` and
    /// then to `Lucene104ScalarQuantizedVectorsReader` — and asks it for its
    /// stored centroid (`Lucene104ScalarQuantizedVectorsWriter.java:389-397`).
    /// `dyn KnnVectorsReader` in this crate carries no `Any`, so neither test
    /// can be written, and this always answers `None`. The only caller,
    /// [`merge_and_recalculate_centroids`](Self::merge_and_recalculate_centroids),
    /// then takes the branch Java takes for a reader that stores no centroid:
    /// it recomputes the centroid from the vectors themselves, which is the
    /// same value the weighted average of per-segment centroids approximates.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsWriter.getCentroid(KnnVectorsReader, String)`.
    pub fn get_centroid(
        _vectors_reader: &dyn crate::codecs::knn_vectors::KnnVectorsReader,
        _field_name: &str,
    ) -> Option<Vec<f32>> {
        None
    }

    /// Combines the centroids of the segments being merged into
    /// `merged_centroid`, recomputing it from the vectors when a segment has no
    /// centroid of its own or has deletions, and returns how many vectors it
    /// covers.
    ///
    /// # Errors
    ///
    /// Returns whatever reading a segment's vectors raises, and
    /// [`LuceneError::IllegalArgument`] when the field's vectors are all zero
    /// and the similarity is cosine.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsWriter#mergeAndRecalculateCentroids`.
    pub fn merge_and_recalculate_centroids(
        merge_state: &MergeState,
        field_info: &FieldInfo,
        merged_centroid: &mut [f32],
    ) -> Result<i32> {
        let mut recalculate = false;
        let mut total_vector_count = 0i32;
        for (i, reader_opt) in merge_state.knn_vectors_readers.iter().enumerate() {
            let Some(knn_vectors_reader) = reader_opt else {
                continue;
            };
            let vector_values = knn_vectors_reader.get_float_vector_values(&field_info.name)?;
            let centroid = Self::get_centroid(knn_vectors_reader.as_ref(), &field_info.name);
            let vector_count = vector_values.size();
            if vector_count == 0 {
                continue;
            }
            total_vector_count += vector_count;
            // If there aren't centroids, or previously clustered with more than
            // one cluster or if there are deleted docs, we must recalculate the
            // centroid.
            if centroid.is_none() || merge_state.live_docs.get(i).is_some_and(Option::is_some) {
                recalculate = true;
                break;
            }
            let centroid = centroid.expect("INVARIANT: the None case broke out of the loop above");
            for (j, &c) in centroid.iter().enumerate() {
                merged_centroid[j] += c * vector_count as f32;
            }
        }
        if total_vector_count == 0 {
            Ok(0)
        } else if recalculate {
            Self::calculate_centroid(merge_state, field_info, merged_centroid)
        } else {
            for slot in merged_centroid.iter_mut() {
                *slot /= total_vector_count as f32;
            }
            if field_info.vector_similarity_function == VectorSimilarityFunction::COSINE {
                vector_util::l2normalize(merged_centroid, true)?;
            }
            Ok(total_vector_count)
        }
    }

    /// Computes the centroid of every vector of `field_info` across the
    /// segments being merged, and returns how many vectors went into it.
    ///
    /// # Errors
    ///
    /// Returns whatever reading a segment's vectors raises, and
    /// [`LuceneError::IllegalArgument`] when the vectors sum to zero and the
    /// similarity is cosine.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsWriter#calculateCentroid`.
    pub fn calculate_centroid(
        merge_state: &MergeState,
        field_info: &FieldInfo,
        centroid: &mut [f32],
    ) -> Result<i32> {
        debug_assert_eq!(field_info.vector_encoding, VectorEncoding::FLOAT32);
        // clear out the centroid
        centroid.fill(0.0);
        let mut count = 0i32;
        for reader_opt in merge_state.knn_vectors_readers.iter() {
            let Some(knn_vectors_reader) = reader_opt else {
                continue;
            };
            let vector_values = knn_vectors_reader.get_float_vector_values(&field_info.name)?;
            let mut iterator = vector_values.iterator()?;
            loop {
                let doc = iterator.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                count += 1;
                let vector = vector_values.vector_value(iterator.index())?;
                for (j, &v) in vector.iter().enumerate() {
                    centroid[j] += v;
                }
            }
        }
        if count == 0 {
            return Ok(count);
        }
        for slot in centroid.iter_mut() {
            *slot /= count as f32;
        }
        if field_info.vector_similarity_function == VectorSimilarityFunction::COSINE {
            vector_util::l2normalize(centroid, true)?;
        }
        Ok(count)
    }
}

/// Returns the document ids a field carries, in ordinal order.
///
/// Equivalent to iterating the `DocsWithFieldSet` Java hands to
/// `KnnVectorsWriter.mapOldOrdToNewOrd`.
fn docs_with_field_to_docs(docs_with_field: &DocsWithFieldSet) -> Result<Vec<i32>> {
    let mut docs = Vec::with_capacity(docs_with_field.cardinality() as usize);
    let mut iterator = docs_with_field.iterator()?;
    loop {
        let doc = iterator.next_doc()?;
        if doc == NO_MORE_DOCS {
            break;
        }
        docs.push(doc);
    }
    Ok(docs)
}

/// Returns the document set of a field under the new numbering.
///
/// Equivalent to the `newDocsWithField` that
/// `KnnVectorsWriter.mapOldOrdToNewOrd` fills as its fifth argument: the same
/// documents, at the positions the sort moved them to.
fn new_docs_with_field(old_doc_ids: &[i32], sort_map: &SorterDocMap) -> Result<DocsWithFieldSet> {
    let mut new_ids: Vec<i32> = old_doc_ids
        .iter()
        .map(|&doc| sort_map.old_to_new(doc))
        .collect();
    new_ids.sort_unstable();
    let mut set = DocsWithFieldSet::new();
    for doc in new_ids {
        set.add(doc)?;
    }
    Ok(set)
}
