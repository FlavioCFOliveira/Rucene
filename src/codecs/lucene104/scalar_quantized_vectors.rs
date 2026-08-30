//! Scalar-quantized vectors ported from `org.apache.lucene.codecs.lucene104`.
//!
//! Stores each float vector as one byte (or half a byte) per dimension, against
//! a per-field centroid, with the corrections that let the compressed form be
//! scored directly.

use std::sync::Arc;

use crate::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use crate::codecs::lucene95::OrdToDocDISIReaderConfiguration;
use crate::error::{LuceneError, Result};
use crate::index::{VectorEncoding, VectorSimilarityFunction};
use crate::store::IndexInput;
use crate::util::hnsw::scorer::RandomVectorScorer;
use crate::util::quantization::{
    OptimizedScalarQuantizer, QuantizationResult, QuantizedByteVectorValues, ScalarEncoding,
    ScalarQuantizedVectorSimilarity,
};

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

    /// Returns how many bytes one quantized vector occupies, including its four
    /// corrective floats and the component sum.
    ///
    /// Equivalent to the stride `OffHeapScalarQuantizedVectorValues` computes.
    pub fn byte_size(&self) -> usize {
        self.scalar_encoding
            .get_doc_length(self.dimension.max(0) as usize)
            + 3 * std::mem::size_of::<f32>()
            + std::mem::size_of::<u16>()
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
        let offset = self.entry.vector_data_offset + i64::from(ord) * self.byte_size as i64;
        self.slice.seek(offset)?;
        let len = self.vector.len();
        self.slice.read_bytes(&mut self.vector, 0, len)?;
        self.corrections = QuantizationResult {
            lower_interval: f32::from_bits(self.slice.read_int()? as u32),
            upper_interval: f32::from_bits(self.slice.read_int()? as u32),
            additional_correction: f32::from_bits(self.slice.read_int()? as u32),
            quantized_component_sum: i32::from(self.slice.read_short()?) & 0xFFFF,
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

            let encoding_ordinal = meta.read_int()?;
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
            let similarity_ordinal = meta.read_int()?;
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
                    "Inconsistent vector similarity function for field=\"{}\"",
                    info.name
                )));
            }

            let entry = FieldEntry::create(meta, vector_encoding, similarity_function)?;
            fields.insert(info.name.clone(), entry);
        }
        Ok(fields)
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
