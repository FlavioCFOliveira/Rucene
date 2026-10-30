//! Scalar-quantized vectors ported from `org.apache.lucene.codecs.lucene104`.
//!
//! Stores each float vector as one byte (or half a byte) per dimension, against
//! a per-field centroid, with the corrections that let the compressed form be
//! scored directly.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::codecs::codec_util::{check_index_header, retrieve_checksum};
use crate::codecs::hnsw::flat_vectors::{
    DefaultFlatFieldVectorsWriter, DocsWithFieldSet, FlatFieldVectorsWriter, FlatVectorsFormat,
    FlatVectorsReader, FlatVectorsScorer, FlatVectorsWriter,
};
use crate::codecs::knn_vectors::{
    BufferingKnnVectorsWriter, FieldVectorWriter, KnnFieldVectorsWriter, KnnVectorsFormat,
    KnnVectorsReader, KnnVectorsWriter, MergedVectorValues,
};
use crate::codecs::lucene95::{HasIndexSlice, OrdToDocDISIReaderConfiguration};
use crate::codecs::lucene99::flat_vectors_format::{
    read_ord_to_doc_reader, sparse_accept_ords, EmptyOffHeapFloatVectorValues,
};
use crate::codecs::state::{OwnedSegmentWriteState, SegmentReadState, SegmentWriteState};
use crate::error::{LuceneError, Result};
use crate::index::index_sorter::SorterDocMap;
use crate::index::{
    accept_ords_with, segment_file_name, ByteVectorValues, DenseDocIndexIterator, DocIndexIterator,
    FieldInfo, FloatVectorValues, KnnVectorValues, MergeState, VectorEncoding,
    VectorSimilarityFunction,
};
use crate::search::vector_scorer::OffHeapVectorScorer;
use crate::search::NO_MORE_DOCS;
use crate::store::{IOContext, IndexInput, IndexOutput};
use crate::util::extra::LongValues;
use crate::util::hnsw::scorer::{
    CloseableRandomVectorScorerSupplier, RandomVectorScorer, RandomVectorScorerSupplier,
    UpdateableRandomVectorScorer,
};
use crate::util::packed::SharedDirectMonotonicReader;
use crate::util::quantization::{
    OptimizedScalarQuantizer, QuantizationResult, QuantizedByteVectorValues,
    QuantizedVectorsReader, ScalarEncoding, ScalarQuantizer,
};
use crate::util::vector_util;
use crate::util::Bits;

/// Info-stream component name this format reports under.
pub const QUANTIZED_VECTOR_COMPONENT: &str = "QVEC";
/// SPI name of the format.
pub const NAME: &str = "Lucene104ScalarQuantizedVectorsFormat";

/// The SPI name of the HNSW-over-scalar-quantized format.
///
/// Equivalent to `Lucene104HnswScalarQuantizedVectorsFormat.NAME`, which is
/// **not** the class name: Lucene registers it as
/// `Lucene104HnswBinaryQuantizedVectorsFormat`, and a segment written by Lucene
/// carries that string, so the port must answer with it.
pub const HNSW_NAME: &str = "Lucene104HnswBinaryQuantizedVectorsFormat";
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
            scalar_encoding = ScalarEncoding::from_wire_number(wire_number).ok_or_else(|| {
                LuceneError::illegal_state(format!(
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
    /// The quantizer the vectors were compressed with, which is fixed by the
    /// field's similarity.
    ///
    /// Equivalent to Lucene's `final OptimizedScalarQuantizer quantizer`, which
    /// the reader builds as `new OptimizedScalarQuantizer(fi.similarityFunction)`
    /// and passes to `load`.
    quantizer: OptimizedScalarQuantizer,
    /// The ordinal-to-document reader of a sparse field, built once at load
    /// and shared between copies; `None` when the field is dense or empty.
    ///
    /// Equivalent to `SparseOffHeapVectorValues.ordToDoc`
    /// (`OffHeapScalarQuantizedVectorValues.java:377`).
    ord_to_doc: Option<Arc<SharedDirectMonotonicReader>>,
    byte_size: usize,
    /// Scratch holding the vector last read.
    vector: Vec<u8>,
    /// Corrections of the vector last read.
    corrections: QuantizationResult,
    /// Which ordinal `vector` holds, or `-1`.
    loaded_ord: i32,
    /// Whether these are the query-width copies an asymmetric encoding writes
    /// for merge-time scoring, rather than the stored documents.
    ///
    /// Equivalent to `OffHeapScalarQuantizedVectorValues.isQuerySide`
    /// (`OffHeapScalarQuantizedVectorValues.java:56,77-104`).
    is_query_side: bool,
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
    ///
    /// Java splits the result into a dense, a sparse and an empty subclass;
    /// they differ only in `ordToDoc` and in the iterator, both of which this
    /// single type derives from the entry's configuration. A sparse field's
    /// ordinal-to-document reader is built here, once, as the sparse
    /// subclass's constructor builds it.
    ///
    /// # Errors
    ///
    /// Returns whatever reading the ordinal-to-document addresses raises.
    pub fn load(entry: FieldEntry, slice: Box<dyn IndexInput>) -> Result<Self> {
        let ord_to_doc = if entry.ord_to_doc.is_empty() || entry.ord_to_doc.is_dense() {
            None
        } else {
            Some(read_ord_to_doc_reader(slice.as_ref(), &entry.ord_to_doc)?)
        };
        Ok(Self::load_with_ord_to_doc(entry, slice, ord_to_doc, false))
    }

    /// Opens the dense, query-side values an asymmetric encoding writes into a
    /// temporary file at merge time: `size` vectors packed at the *query*
    /// width, each followed by its corrections, starting at offset 0.
    ///
    /// Equivalent to `new OffHeapScalarQuantizedVectorValues.DenseOffHeapVectorValues(true, …)`
    /// (`Lucene104ScalarQuantizedVectorsReader.java:498-509`).
    fn load_query_side(entry: &FieldEntry, size: i32, slice: Box<dyn IndexInput>) -> Self {
        let mut query_entry = entry.clone();
        query_entry.size = size;
        query_entry.vector_data_offset = 0;
        query_entry.ord_to_doc.size = size;
        // A dense configuration: the ordinal is the document id.
        query_entry.ord_to_doc.docs_with_field_offset = -1;
        Self::load_with_ord_to_doc(query_entry, slice, None, true)
    }

    /// As [`load`](Self::load), with the sparse ordinal-to-document reader
    /// already built; `copy()` shares it this way.
    fn load_with_ord_to_doc(
        entry: FieldEntry,
        slice: Box<dyn IndexInput>,
        ord_to_doc: Option<Arc<SharedDirectMonotonicReader>>,
        is_query_side: bool,
    ) -> Self {
        let dimension = entry.dimension.max(0);
        let dim_bytes = if is_query_side {
            entry.scalar_encoding.get_query_packed_length(dimension)
        } else {
            entry.scalar_encoding.get_doc_packed_length(dimension)
        }
        .max(0) as usize;
        let byte_size = dim_bytes + 3 * std::mem::size_of::<f32>() + std::mem::size_of::<i32>();
        let quantizer = OptimizedScalarQuantizer::new(entry.similarity_function);
        Self {
            entry,
            slice,
            quantizer,
            ord_to_doc,
            byte_size,
            vector: vec![0u8; dim_bytes],
            corrections: QuantizationResult {
                lower_interval: 0.0,
                upper_interval: 0.0,
                additional_correction: 0.0,
                quantized_component_sum: 0,
            },
            loaded_ord: -1,
            is_query_side,
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

    /// Returns the quantizer these vectors were compressed with.
    ///
    /// Equivalent to `OffHeapScalarQuantizedVectorValues.getQuantizer()`.
    pub fn get_quantizer(&self) -> &OptimizedScalarQuantizer {
        &self.quantizer
    }

    /// Maps a vector ordinal back to the document that carried it.
    ///
    /// Equivalent to `KnnVectorValues.ordToDoc(int)` as the dense and sparse
    /// subclasses implement it: the ordinal itself when every document has a
    /// vector, and the stored monotonic mapping otherwise.
    pub fn ord_to_doc(&self, ord: i32) -> i32 {
        match &self.ord_to_doc {
            Some(reader) => reader.get(i64::from(ord)) as i32,
            None => ord,
        }
    }

    /// Returns the accepted ordinals, given the accepted documents.
    ///
    /// Equivalent to the `getAcceptOrds(Bits)` overrides of
    /// `OffHeapScalarQuantizedVectorValues`: `acceptDocs` for a dense field, a
    /// lazy `acceptDocs.get(ordToDoc(ord))` view for a sparse one, and `null`
    /// for an empty one (`OffHeapScalarQuantizedVectorValues.java:308, 404,
    /// 481`).
    pub fn get_accept_ords<'a>(
        &self,
        accept_docs: Option<&'a dyn Bits>,
    ) -> Option<Box<dyn Bits + 'a>> {
        if self.entry.ord_to_doc.is_empty() {
            return None;
        }
        sparse_accept_ords(self.entry.size, self.ord_to_doc.as_ref(), accept_docs)
    }

    /// Returns how many bytes one *packed* vector occupies, without its three
    /// corrective floats and its component sum.
    ///
    /// Equivalent to `OffHeapScalarQuantizedVectorValues.getVectorByteLength()`,
    /// which returns `vectorValue.length` — the buffer sized to
    /// `encoding.getDocPackedLength(dimension)` in the constructor, or
    /// `getQueryPackedLength(dimension)` for query-side values. It is not
    /// the same as [`FieldEntry::byte_size`], which is the whole on-disk stride.
    pub fn get_vector_byte_length(&self) -> i32 {
        self.vector.len() as i32
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
        Ok(Self::load_with_ord_to_doc(
            self.entry.clone(),
            self.slice.clone_input()?,
            self.ord_to_doc.clone(),
            self.is_query_side,
        ))
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
        // Java slices the data file down to the field's region and then seeks
        // `targetOrd * byteSize` inside it; this holds a view of the whole file
        // instead, so the field's offset is added back here. Same byte.
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

    fn ord_to_doc(&self, ord: i32) -> i32 {
        OffHeapScalarQuantizedVectorValues::ord_to_doc(self, ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        OffHeapScalarQuantizedVectorValues::get_accept_ords(self, accept_docs)
    }

    /// Scores `target` against a copy of these quantized vectors, driven by
    /// the copy's own iterator; a field with no vector has no scorer.
    ///
    /// Equivalent to the `scorer(float[])` overrides of
    /// `OffHeapScalarQuantizedVectorValues.DenseOffHeapVectorValues`,
    /// `SparseOffHeapVectorValues` and `EmptyOffHeapVectorValues`
    /// (`OffHeapScalarQuantizedVectorValues.java:313-335, 427-449, 486-488`):
    /// a `copy()`, its `iterator()`, and
    /// `vectorsScorer.getRandomVectorScorer(similarityFunction, copy, target)`,
    /// whose quantized branch is
    /// [`Lucene104ScalarQuantizedVectorScorer::get_random_vector_scorer_quantized`].
    fn scorer(&self, target: &[f32]) -> Result<Option<Box<dyn crate::search::VectorScorer>>> {
        if self.entry.ord_to_doc.is_empty() {
            return Ok(None);
        }
        let copy = OffHeapScalarQuantizedVectorValues::copy(self)?;
        let iterator = QuantizedByteVectorValues::iterator(&copy)?;
        let dense = self.entry.ord_to_doc.is_dense();
        let random_vector_scorer =
            Lucene104ScalarQuantizedVectorScorer::get_random_vector_scorer_quantized(
                self.entry.similarity_function,
                copy,
                target,
            )?;
        Ok(Some(Box::new(OffHeapVectorScorer::new(
            Box::new(random_vector_scorer),
            iterator,
            dense,
        ))))
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

    fn get_quantizer(&self) -> Option<&OptimizedScalarQuantizer> {
        Some(OffHeapScalarQuantizedVectorValues::get_quantizer(self))
    }

    fn copy(&self) -> Result<Box<dyn QuantizedByteVectorValues>> {
        Ok(Box::new(OffHeapScalarQuantizedVectorValues::copy(self)?))
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

/// Presents one field's quantized vectors as floats, dequantizing on each read.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.OffHeapScalarQuantizedFloatVectorValues`.
///
/// Java splits this into `DenseOffHeapVectorValues`,
/// `SparseOffHeapVectorValues` and `EmptyOffHeapVectorValues`; the three differ
/// only in how an ordinal maps back to a document and how the iterator is
/// built, so this port carries the sparse configuration in an `Option` — the
/// same shape `OffHeapFloatVectorValues` already uses for the unquantized
/// format.
pub struct OffHeapScalarQuantizedFloatVectorValues {
    dimension: i32,
    size: i32,
    /// The whole on-disk stride of one entry: the packed components, the three
    /// corrective floats and the component sum.
    byte_size: usize,
    /// How many bytes the packed components occupy inside that stride.
    packed_length: usize,
    encoding: ScalarEncoding,
    centroid: Vec<f32>,
    slice: Mutex<Box<dyn IndexInput>>,
    config: Option<OrdToDocDISIReaderConfiguration>,
    data_input: Option<Box<dyn IndexInput>>,
    /// The ordinal-to-document reader of a sparse field, built once at load
    /// and shared between copies; `None` when the field is dense.
    ///
    /// Equivalent to `SparseOffHeapVectorValues.ordToDoc`
    /// (`OffHeapScalarQuantizedFloatVectorValues.java:294`).
    ord_to_doc: Option<Arc<SharedDirectMonotonicReader>>,
    /// Equivalent to the `final VectorSimilarityFunction similarityFunction`
    /// field.
    similarity_function: VectorSimilarityFunction,
    /// Equivalent to the `final FlatVectorsScorer vectorsScorer` field, which
    /// `scorer` scores through.
    vectors_scorer: Arc<dyn FlatVectorsScorer>,
}

impl std::fmt::Debug for OffHeapScalarQuantizedFloatVectorValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OffHeapScalarQuantizedFloatVectorValues")
            .field("size", &self.size)
            .field("dimension", &self.dimension)
            .finish_non_exhaustive()
    }
}

impl OffHeapScalarQuantizedFloatVectorValues {
    /// Opens the dequantizing view of one field's vectors.
    ///
    /// Equivalent to `OffHeapScalarQuantizedFloatVectorValues.load`: an empty
    /// configuration answers the empty values, a dense one indexes ordinals
    /// directly, and a sparse one reads the ordinal-to-document mapping.
    ///
    /// # Errors
    ///
    /// Returns whatever slicing the data file or reading the mapping raises.
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        config: &OrdToDocDISIReaderConfiguration,
        dimension: i32,
        size: i32,
        encoding: ScalarEncoding,
        similarity_function: VectorSimilarityFunction,
        vectors_scorer: Arc<dyn FlatVectorsScorer>,
        centroid: Option<&[f32]>,
        vector_data_offset: i64,
        vector_data_length: i64,
        vector_data: &dyn IndexInput,
    ) -> Result<Box<dyn FloatVectorValues>> {
        if config.is_empty() {
            // Java's `EmptyOffHeapVectorValues`
            // (`OffHeapScalarQuantizedFloatVectorValues.java:367-401`) behaves
            // exactly as the `lucene95` empty values: the field's dimension,
            // no vector, no copy, no scorer.
            return Ok(Box::new(EmptyOffHeapFloatVectorValues { dimension }));
        }
        // Java asserts `centroid != null` here: a non-empty configuration means
        // the metadata carried a centroid.
        let centroid = centroid.ok_or_else(|| {
            LuceneError::corrupt_index(
                "quantized vectors have no centroid to dequantize against",
                "quantized vectors metadata",
            )
        })?;
        let dims = dimension.max(0) as usize;
        let packed_length = encoding.get_doc_packed_length(dimension.max(0)).max(0) as usize;
        let byte_size = packed_length + 3 * std::mem::size_of::<f32>() + std::mem::size_of::<i32>();
        let slice = Mutex::new(vector_data.slice(
            "scalar-quantized-float-vector-data",
            vector_data_offset,
            vector_data_length,
        )?);
        let (config, data_input, ord_to_doc) = if config.is_dense() {
            (None, None, None)
        } else {
            (
                Some(config.clone()),
                Some(vector_data.clone_input()?),
                Some(read_ord_to_doc_reader(vector_data, config)?),
            )
        };
        let _ = dims;
        Ok(Box::new(Self {
            dimension,
            size,
            byte_size,
            packed_length,
            encoding,
            centroid: centroid.to_vec(),
            slice,
            config,
            data_input,
            ord_to_doc,
            similarity_function,
            vectors_scorer,
        }))
    }

    /// Returns how many bytes one *dequantized* vector occupies, which is the
    /// dimension because every component unpacks to one byte.
    ///
    /// Equivalent to
    /// `OffHeapScalarQuantizedFloatVectorValues.getVectorByteLength()`, which
    /// returns `dimension` rather than the packed length.
    pub fn get_vector_byte_length(&self) -> i32 {
        self.dimension
    }

    /// Clones the values together with a fresh view of the data.
    fn clone_values(&self) -> Result<Self> {
        let slice = Mutex::new(self.lock_slice()?.clone_input()?);
        Ok(Self {
            dimension: self.dimension,
            size: self.size,
            byte_size: self.byte_size,
            packed_length: self.packed_length,
            encoding: self.encoding,
            centroid: self.centroid.clone(),
            slice,
            config: self.config.clone(),
            data_input: match &self.data_input {
                Some(input) => Some(input.clone_input()?),
                None => None,
            },
            ord_to_doc: self.ord_to_doc.clone(),
            similarity_function: self.similarity_function,
            vectors_scorer: Arc::clone(&self.vectors_scorer),
        })
    }

    fn lock_slice(&self) -> Result<MutexGuard<'_, Box<dyn IndexInput>>> {
        self.slice.lock().map_err(|_| {
            LuceneError::illegal_state(
                "quantized float vector slice mutex was poisoned".to_string(),
            )
        })
    }
}

impl KnnVectorValues for OffHeapScalarQuantizedFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        match &self.ord_to_doc {
            Some(reader) => reader.get(i64::from(ord)) as i32,
            None => ord,
        }
    }

    /// Equivalent to the dense (`acceptDocs`) and sparse (lazy
    /// `acceptDocs.get(ordToDoc(ord))`) `getAcceptOrds` overrides
    /// (`OffHeapScalarQuantizedFloatVectorValues.java:238, 319`).
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        sparse_accept_ords(self.size, self.ord_to_doc.as_ref(), accept_docs)
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn vector_byte_length(&self) -> i32 {
        self.get_vector_byte_length()
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        match &self.config {
            Some(config) => {
                let data_input = self
                    .data_input
                    .as_ref()
                    .expect("INVARIANT: sparse values have a data input");
                Ok(Box::new(config.get_indexed_disi(data_input.as_ref())?))
            }
            None => Ok(Box::new(DenseDocIndexIterator::new(self.size))),
        }
    }
}

impl FloatVectorValues for OffHeapScalarQuantizedFloatVectorValues {
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    /// Reads the packed components at `ord`, unpacks them when the encoding
    /// packs more than one component per byte, and dequantizes with the two
    /// interval ends stored alongside them.
    ///
    /// Equivalent to
    /// `OffHeapScalarQuantizedFloatVectorValues.vectorValue(int)`. The unpack
    /// step is what makes `PACKED_NIBBLE` readable: its stored bytes hold two
    /// components each, and `deQuantize` needs one component per byte.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        if ord < 0 || ord >= self.size {
            return Err(LuceneError::IllegalArgument(format!(
                "vector ordinal {ord} is out of range for {} vectors",
                self.size
            )));
        }
        let dims = self.dimension.max(0) as usize;
        let mut packed = vec![0u8; self.packed_length];
        let mut corrective = [0f32; 3];
        {
            let mut slice = self.lock_slice()?;
            slice.seek(i64::from(ord) * self.byte_size as i64)?;
            let len = packed.len();
            slice.read_bytes(&mut packed, 0, len)?;
            slice.read_floats(&mut corrective, 0, 3)?;
        }
        let quantized = match self.encoding {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => packed,
            ScalarEncoding::PackedNibble => {
                let mut unpacked = vec![0u8; self.packed_length * 2];
                OffHeapScalarQuantizedVectorValues::unpack_nibbles(&packed, &mut unpacked)?;
                unpacked
            }
            // Java unpacks into `new byte[dimension]`
            // (`OffHeapScalarQuantizedFloatVectorValues.java:96,125-128`).
            ScalarEncoding::SingleBitQueryNibble => {
                let mut unpacked = vec![0u8; dims];
                OptimizedScalarQuantizer::unpack_binary(&packed, &mut unpacked);
                unpacked
            }
            ScalarEncoding::DibitQueryNibble => {
                let mut unpacked = vec![0u8; dims];
                OptimizedScalarQuantizer::untranspose_dibit(&packed, &mut unpacked);
                unpacked
            }
        };
        let result = QuantizationResult {
            lower_interval: corrective[0],
            upper_interval: corrective[1],
            additional_correction: corrective[2],
            quantized_component_sum: 0,
        };
        let mut values = OptimizedScalarQuantizer::de_quantize(
            &quantized,
            self.encoding.get_bits(),
            &result,
            &self.centroid,
        );
        // `getDiscreteDimensions` rounds the packed vector up to a whole number
        // of bytes, so an unpacked nibble vector can carry one padding
        // component past the field's dimension; Java's `deQuantize` writes into
        // a `float[dimension]` and stops there.
        values.truncate(dims);
        Ok(values)
    }

    /// Scores `target` against a copy of these dequantized vectors, driven by
    /// the copy's own iterator.
    ///
    /// Equivalent to `DenseOffHeapVectorValues.scorer(float[])` and
    /// `SparseOffHeapVectorValues.scorer(float[])`
    /// (`OffHeapScalarQuantizedFloatVectorValues.java:243-265, 342-364`): a
    /// `copy()`, its `iterator()`, and
    /// `vectorsScorer.getRandomVectorScorer(similarityFunction, copy, target)`.
    /// The copy is float vector values, not quantized ones, so the reader's
    /// `Lucene104ScalarQuantizedVectorScorer` hands it to its non-quantized
    /// delegate.
    fn scorer(&self, target: &[f32]) -> Result<Option<Box<dyn crate::search::VectorScorer>>> {
        let copy = self.clone_values()?;
        let iterator = copy.iterator()?;
        let dense = copy.config.is_none();
        let random_vector_scorer = self.vectors_scorer.get_random_vector_scorer_float(
            self.similarity_function,
            Box::new(copy),
            target,
        )?;
        Ok(Some(Box::new(OffHeapVectorScorer::new(
            random_vector_scorer,
            iterator,
            dense,
        ))))
    }
}

/// Scores a query against the quantized vectors of one field.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorScorer`.
///
/// **Divergence from Lucene 10.5.0.** Two, both forced. First, Java's
/// `VectorUtil` picks a vectorised inner loop through `VectorizationProvider`;
/// this crate's `VectorUtil` is scalar-only (rmp #165), which yields the same
/// scores. Second, the [`FlatVectorsScorer`] impl forwards all four methods to
/// the non-quantized delegate, without the `instanceof
/// QuantizedByteVectorValues` branch of
/// `Lucene104ScalarQuantizedVectorScorer.java:47-101`, because a
/// `Box<dyn FloatVectorValues>` cannot be narrowed to the quantized type; the
/// reader, which knows it holds quantized values, calls the quantized branch
/// directly ([`Self::get_random_vector_scorer_quantized`],
/// [`Self::get_random_vector_scorer_supplier_quantized`]).
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

    /// Scores an already quantized query against the vector stored at
    /// `target_ord`, applying the four corrective terms both sides carry.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the query and the stored
    /// vector have different lengths, and whatever reading the stored vector
    /// and its corrections raises.
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
                ScalarEncoding::SingleBitQueryNibble => {
                    vector_util::int4_bit_dot_product(quantized_query, quantized_doc)? as f32
                }
                ScalarEncoding::DibitQueryNibble => {
                    vector_util::int4_dibit_dot_product(quantized_query, quantized_doc)? as f32
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
    /// Quantizes `target` the way the field's vectors were quantized and binds
    /// it to a scorer over `vector_values`.
    ///
    /// Equivalent to the `instanceof QuantizedByteVectorValues` branch of
    /// `Lucene104ScalarQuantizedVectorScorer.getRandomVectorScorer(VectorSimilarityFunction,
    /// KnnVectorValues, float[])`: the target is copied before quantizing
    /// because quantization mutates it, normalised first when the similarity is
    /// cosine, and quantized at the *query* width against the field's centroid.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the target's length is not
    /// the field's dimension, [`LuceneError::IllegalState`] when the field
    /// stored no centroid, and whatever quantizing raises.
    pub fn get_random_vector_scorer_quantized(
        similarity_function: VectorSimilarityFunction,
        vector_values: OffHeapScalarQuantizedVectorValues,
        target: &[f32],
    ) -> Result<Lucene104ScalarQuantizedRandomVectorScorer> {
        let dimension = vector_values.dimension();
        if target.len() as i32 != dimension {
            return Err(LuceneError::IllegalArgument(format!(
                "vector query dimension: {} differs from field dimension: {dimension}",
                target.len()
            )));
        }
        let encoding = vector_values.get_scalar_encoding();
        let mut scratch = vec![0u8; encoding.get_discrete_dimensions(dimension).max(0) as usize];
        // An asymmetric encoding packs the quantized query into its own buffer
        // (`Lucene104ScalarQuantizedVectorScorer.java:69-74`).
        let mut target_quantized = if encoding.is_asymmetric() {
            Some(vec![
                0u8;
                encoding
                    .get_query_packed_length(scratch.len() as i32)
                    .max(0) as usize
            ])
        } else {
            None
        };
        // Java copies the target because `scalarQuantize` mutates its input.
        let mut copy = target.to_vec();
        if similarity_function == VectorSimilarityFunction::COSINE {
            vector_util::l2normalize(&mut copy, true)?;
        }
        let centroid = vector_values
            .get_centroid()
            .ok_or_else(|| {
                LuceneError::illegal_state(
                    "quantized vectors have no centroid to quantize the query against".to_string(),
                )
            })?
            .to_vec();
        let corrections = vector_values.get_quantizer().scalar_quantize(
            &mut copy,
            &mut scratch,
            encoding.get_query_bits(),
            &centroid,
        )?;
        // For asymmetric encodings with a 4-bit query, the nibbles are
        // transposed for fast scoring (`Lucene104ScalarQuantizedVectorScorer.java:84-89`).
        if matches!(
            encoding,
            ScalarEncoding::SingleBitQueryNibble | ScalarEncoding::DibitQueryNibble
        ) {
            if let Some(packed) = target_quantized.as_mut() {
                OptimizedScalarQuantizer::transpose_half_byte(&scratch, packed);
            }
        }
        Ok(Lucene104ScalarQuantizedRandomVectorScorer::new(
            similarity_function,
            encoding,
            target_quantized.unwrap_or(scratch),
            corrections,
            vector_values,
        ))
    }

    /// Returns a supplier of scorers over `vector_values`, for the merge path,
    /// which scores stored vectors against one another rather than against a
    /// query.
    ///
    /// Equivalent to the `instanceof QuantizedByteVectorValues` branch of
    /// `Lucene104ScalarQuantizedVectorScorer.getRandomVectorScorerSupplier(VectorSimilarityFunction,
    /// KnnVectorValues)`; the unquantized case is the delegate's own
    /// [`FlatVectorsScorer`] methods, reached through
    /// [`get_non_quantized_delegate`](Self::get_non_quantized_delegate).
    ///
    /// # Errors
    ///
    /// Returns whatever duplicating the vector values raises.
    pub fn get_random_vector_scorer_supplier_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: OffHeapScalarQuantizedVectorValues,
    ) -> Result<ScalarQuantizedVectorScorerSupplier> {
        ScalarQuantizedVectorScorerSupplier::new(vector_values, similarity_function)
    }
}

/// Scores queries against a field's vectors, quantizing the query first.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorScorer implements
/// FlatVectorsScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java dispatches on `vectorValues
/// instanceof QuantizedByteVectorValues` and otherwise forwards to the
/// non-quantized delegate. The trait here is typed on `FloatVectorValues` and
/// `ByteVectorValues`, which cannot be narrowed to the quantized form, so these
/// four methods are exactly the delegate branch; the quantized branch is
/// [`Self::get_random_vector_scorer_quantized`] and
/// [`Self::get_random_vector_scorer_supplier_quantized`], which the reader
/// calls directly because it knows it holds quantized values.
impl FlatVectorsScorer for Lucene104ScalarQuantizedVectorScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.non_quantized_delegate
            .get_random_vector_scorer_supplier_float(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.non_quantized_delegate
            .get_random_vector_scorer_supplier_byte(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.non_quantized_delegate.get_random_vector_scorer_float(
            similarity_function,
            vector_values,
            target,
        )
    }

    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.non_quantized_delegate.get_random_vector_scorer_byte(
            similarity_function,
            vector_values,
            target,
        )
    }
}

/// Presents the raw float vectors of a field alongside its quantized form, so
/// that reads return the full-precision vector while scoring runs off the
/// compressed one.
///
/// Equivalent to the nested
/// `Lucene104ScalarQuantizedVectorsReader.ScalarQuantizedVectorValues`.
pub struct ScalarQuantizedVectorValues {
    raw: Box<dyn FloatVectorValues>,
    quantized: Box<dyn QuantizedByteVectorValues>,
}

impl std::fmt::Debug for ScalarQuantizedVectorValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarQuantizedVectorValues")
            .finish_non_exhaustive()
    }
}

impl ScalarQuantizedVectorValues {
    /// Pairs the raw vectors with their quantized form.
    pub fn new(
        raw: Box<dyn FloatVectorValues>,
        quantized: Box<dyn QuantizedByteVectorValues>,
    ) -> Self {
        Self { raw, quantized }
    }

    /// Returns the quantized form.
    ///
    /// Equivalent to
    /// `ScalarQuantizedVectorValues.getQuantizedVectorValues()`.
    pub fn get_quantized_vector_values(&self) -> &dyn QuantizedByteVectorValues {
        self.quantized.as_ref()
    }
}

impl KnnVectorValues for ScalarQuantizedVectorValues {
    fn dimension(&self) -> i32 {
        self.raw.dimension()
    }

    fn size(&self) -> i32 {
        self.raw.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.raw.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.raw.get_accept_ords(accept_docs)
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(Self::new(
            self.raw.copy_float()?,
            self.quantized.copy()?,
        )))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        self.raw.iterator()
    }
}

impl FloatVectorValues for ScalarQuantizedVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        self.raw.vector_value(ord)
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(Self::new(
            self.raw.copy_float()?,
            self.quantized.copy()?,
        )))
    }

    /// Scores against the *quantized* vectors.
    ///
    /// Equivalent to `ScalarQuantizedVectorValues.scorer(float[])`, which is
    /// the whole point of the pairing: reads are full precision, scoring is
    /// compressed.
    fn scorer(&self, target: &[f32]) -> Result<Option<Box<dyn crate::search::VectorScorer>>> {
        self.quantized.scorer(target)
    }

    /// Rescoring uses the raw vectors, which is where the extra precision is.
    ///
    /// Equivalent to `ScalarQuantizedVectorValues.rescorer(float[])`.
    fn rescorer(&self, target: &[f32]) -> Result<Option<Box<dyn crate::search::VectorScorer>>> {
        self.raw.rescorer(target)
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
            return Err(LuceneError::illegal_state(format!(
                "{} encoding is not supported for symmetric quantization",
                values.get_scalar_encoding().name()
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

/// Supplies merge-time scorers for an asymmetric encoding: each scoring
/// ordinal is read from the query-width copies, and scored against the stored
/// document vectors.
///
/// Equivalent to
/// `Lucene104ScalarQuantizedVectorScorer.AsymmetricQuantizedRandomVectorScorerSupplier`
/// (`Lucene104ScalarQuantizedVectorScorer.java:124-171`), built by
/// `getRandomVectorScorerSupplier(VectorSimilarityFunction, QuantizedByteVectorValues, QuantizedByteVectorValues)`.
pub struct AsymmetricQuantizedRandomVectorScorerSupplier {
    query_vectors: OffHeapScalarQuantizedVectorValues,
    target_vectors: OffHeapScalarQuantizedVectorValues,
    similarity_function: VectorSimilarityFunction,
}

impl std::fmt::Debug for AsymmetricQuantizedRandomVectorScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsymmetricQuantizedRandomVectorScorerSupplier")
            .field("similarity_function", &self.similarity_function)
            .finish_non_exhaustive()
    }
}

impl AsymmetricQuantizedRandomVectorScorerSupplier {
    /// Creates the supplier.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the target encoding is
    /// symmetric, which Java asserts.
    pub fn new(
        query_vectors: OffHeapScalarQuantizedVectorValues,
        target_vectors: OffHeapScalarQuantizedVectorValues,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Self> {
        if !target_vectors.get_scalar_encoding().is_asymmetric() {
            return Err(LuceneError::illegal_state(format!(
                "{} encoding is not asymmetric",
                target_vectors.get_scalar_encoding().name()
            )));
        }
        Ok(Self {
            query_vectors,
            target_vectors,
            similarity_function,
        })
    }
}

impl RandomVectorScorerSupplier for AsymmetricQuantizedRandomVectorScorerSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        Ok(Box::new(AsymmetricQuantizedUpdateableScorer {
            target_vectors: self.target_vectors.copy()?,
            query_vectors: self.query_vectors.copy()?,
            similarity_function: self.similarity_function,
            vector: None,
            query_corrections: None,
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::new(
            self.query_vectors.copy()?,
            self.target_vectors.copy()?,
            self.similarity_function,
        )?))
    }
}

/// The scorer [`AsymmetricQuantizedRandomVectorScorerSupplier`] hands out.
///
/// Equivalent to the anonymous `AbstractUpdateableRandomVectorScorer` of
/// `AsymmetricQuantizedRandomVectorScorerSupplier.scorer()`.
struct AsymmetricQuantizedUpdateableScorer {
    target_vectors: OffHeapScalarQuantizedVectorValues,
    query_vectors: OffHeapScalarQuantizedVectorValues,
    similarity_function: VectorSimilarityFunction,
    vector: Option<Vec<u8>>,
    query_corrections: Option<QuantizationResult>,
}

impl RandomVectorScorer for AsymmetricQuantizedUpdateableScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let (Some(vector), Some(query_corrections)) =
            (self.vector.as_ref(), self.query_corrections)
        else {
            return Err(LuceneError::illegal_state(
                "setScoringOrdinal was not called".to_string(),
            ));
        };
        Lucene104ScalarQuantizedVectorScorer::quantized_score(
            vector,
            &query_corrections,
            &mut self.target_vectors,
            node,
            self.similarity_function,
        )
    }

    fn max_ord(&self) -> i32 {
        self.target_vectors.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.target_vectors.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.target_vectors.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for AsymmetricQuantizedUpdateableScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        self.vector = Some(self.query_vectors.vector_value(node)?.to_vec());
        self.query_corrections = Some(self.query_vectors.get_corrective_terms(node)?);
        Ok(())
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
            return Err(LuceneError::illegal_state(
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

    /// Equivalent to `AbstractUpdateableRandomVectorScorer.ordToDoc(int)`,
    /// which forwards to the values the scorer was built over.
    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    /// Equivalent to the `getAcceptOrds` Java's `AbstractUpdateableRandomVectorScorer` provides,
    /// `values.getAcceptOrds(acceptDocs)`: the view shares the values'
    /// ordinal-to-document reader and copies nothing.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
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
            encoding
            @ (ScalarEncoding::SingleBitQueryNibble | ScalarEncoding::DibitQueryNibble) => {
                return Err(LuceneError::illegal_state(format!(
                    "{} encoding is not supported for symmetric quantization",
                    encoding.name()
                )));
            }
        }
        self.target_corrective_terms = Some(self.target_values.get_corrective_terms(node)?);
        Ok(())
    }
}

/// A scorer bound to one query and one field's quantized vectors.
///
/// Equivalent to the anonymous `RandomVectorScorer.AbstractRandomVectorScorer`
/// that `Lucene104ScalarQuantizedVectorScorer.getRandomVectorScorer` returns.
/// It holds no reference to the scorer that built it, because the scoring it
/// performs — `quantizedScore` — is a `private static` method.
pub struct Lucene104ScalarQuantizedRandomVectorScorer {
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
        similarity_function: VectorSimilarityFunction,
        encoding: ScalarEncoding,
        query: Vec<u8>,
        query_corrections: QuantizationResult,
        values: OffHeapScalarQuantizedVectorValues,
    ) -> Self {
        Self {
            similarity_function,
            encoding,
            query,
            query_corrections,
            values,
        }
    }
}

impl RandomVectorScorer for Lucene104ScalarQuantizedRandomVectorScorer {
    /// Scores the quantized query against the vector at `node`.
    ///
    /// Equivalent to the anonymous `AbstractRandomVectorScorer` that
    /// `Lucene104ScalarQuantizedVectorScorer.getRandomVectorScorer` returns,
    /// whose `score(int)` is `quantizedScore(targetQuantized,
    /// targetCorrectiveTerms, qv, node, similarityFunction)` — the same
    /// arithmetic the merge-time supplier uses.
    fn score(&mut self, node: i32) -> Result<f32> {
        let _ = self.encoding;
        Lucene104ScalarQuantizedVectorScorer::quantized_score(
            &self.query,
            &self.query_corrections,
            &mut self.values,
            node,
            self.similarity_function,
        )
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    /// Maps an ordinal back to its document, through the field's stored
    /// ordinal-to-document mapping.
    ///
    /// Equivalent to `AbstractRandomVectorScorer.ordToDoc(int)`, which forwards
    /// to the `KnnVectorValues` it was constructed with.
    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    /// Equivalent to the `getAcceptOrds` Java's `AbstractRandomVectorScorer` provides,
    /// `values.getAcceptOrds(acceptDocs)`: the view shares the values'
    /// ordinal-to-document reader and copies nothing.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }
}

/// The shallow size of `Lucene104ScalarQuantizedVectorsReader` on the JVM.
///
/// Equivalent to Lucene's `SHALLOW_SIZE =
/// RamUsageEstimator.shallowSizeOfInstance(Lucene104ScalarQuantizedVectorsReader.class)`.
/// Measured against `lucene-core-10.5.0.jar`, which answers 32; the Rust struct
/// has a different layout, so the JVM's number is stated rather than derived —
/// `ramBytesUsed` answers what the Java object would cost.
pub const READER_SHALLOW_SIZE: i64 = 32;

/// The shallow size of the reader's `FieldEntry` record on the JVM, which is
/// the per-entry cost `ramBytesUsed` charges for the field map.
///
/// Measured against `lucene-core-10.5.0.jar`: 64.
pub const FIELD_ENTRY_SHALLOW_SIZE: i64 = 64;

/// How many ordinals an exhaustive search scores in one batch.
///
/// Equivalent to
/// `Lucene104ScalarQuantizedVectorsReader.EXHAUSTIVE_BULK_SCORE_ORDS`.
pub const EXHAUSTIVE_BULK_SCORE_ORDS: usize = 64;

/// Reads the quantized vectors of a segment.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader`,
/// which is a `FlatVectorsReader` wrapping the raw `Lucene99FlatVectorsReader`:
/// the byte vectors and the raw float values come from the delegate, while the
/// float view, the scorers and the search run off the quantized form.
pub struct Lucene104ScalarQuantizedVectorsReader {
    /// One entry per field that has quantized vectors, by field name.
    fields: std::collections::BTreeMap<String, FieldEntry>,
    vector_data: Arc<dyn IndexInput>,
    /// The reader of the unquantized vectors this one wraps.
    ///
    /// Equivalent to Lucene's `final FlatVectorsReader rawVectorsReader`.
    raw_vectors_reader: Box<dyn FlatVectorsReader>,
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
    /// Opens the reader of a segment: reads the `.vemq` metadata and opens the
    /// `.veq` data file, keeping `raw_vectors_reader` for everything the
    /// quantized form does not answer.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when either file's header, footer
    /// or version disagrees with what the format writes, and whatever reading
    /// the metadata raises.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsReader(SegmentReadState, FlatVectorsReader, Lucene104ScalarQuantizedVectorScorer)`.
    pub fn new(
        state: &SegmentReadState<'_>,
        raw_vectors_reader: Box<dyn FlatVectorsReader>,
        scorer: Arc<Lucene104ScalarQuantizedVectorScorer>,
    ) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta = state.directory.open_checksum_input(&meta_file_name)?;
        let version_meta = check_index_header(
            meta.as_mut(),
            META_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        let fields = Self::read_fields(meta.as_mut(), state.field_infos)?;
        crate::codecs::codec_util::check_footer(meta.as_mut())?;
        meta.close()?;

        let vector_data = Self::open_data_input(
            state,
            version_meta,
            VECTOR_DATA_EXTENSION,
            VECTOR_DATA_CODEC_NAME,
            state.context,
        )?;

        Ok(Self {
            fields,
            vector_data: Arc::from(vector_data),
            raw_vectors_reader,
            scorer,
        })
    }

    /// Builds a reader over metadata that has already been read, for the paths
    /// that open the two files themselves.
    ///
    /// This is not a Lucene constructor: Java's reader always reads its own
    /// metadata. It exists so that a caller holding the parts does not have to
    /// re-open the segment.
    pub fn from_parts(
        fields: std::collections::BTreeMap<String, FieldEntry>,
        vector_data: Arc<dyn IndexInput>,
        raw_vectors_reader: Box<dyn FlatVectorsReader>,
        scorer: Arc<Lucene104ScalarQuantizedVectorScorer>,
    ) -> Self {
        Self {
            fields,
            vector_data,
            raw_vectors_reader,
            scorer,
        }
    }

    /// Returns the centroid the vectors of `field` were quantized against.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.getCentroid(String)`,
    /// which answers `null` for a field this reader holds no entry for.
    pub fn get_centroid(&self, field: &str) -> Option<&[f32]> {
        self.fields
            .get(field)
            .and_then(|entry| entry.centroid.as_deref())
    }

    /// Returns a scorer of `target` against `field`'s quantized vectors, or
    /// `None` when this reader holds no entry for the field.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsReader.getRandomVectorScorer(String, float[])`,
    /// keeping Java's `null` return as an `Option`.
    ///
    /// # Errors
    ///
    /// Returns whatever opening the values or quantizing the query raises.
    pub fn random_vector_scorer_float(
        &self,
        field: &str,
        target: &[f32],
    ) -> Result<Option<Box<dyn RandomVectorScorer>>> {
        let Some(entry) = self.fields.get(field) else {
            return Ok(None);
        };
        let values = self.quantized_byte_vector_values(entry)?;
        Ok(Some(Box::new(
            Lucene104ScalarQuantizedVectorScorer::get_random_vector_scorer_quantized(
                entry.similarity_function,
                values,
                target,
            )?,
        )))
    }

    /// Opens `entry`'s quantized vectors over a fresh view of the `.veq` file,
    /// carrying the sparse ordinal-to-document addresses when there are any.
    fn quantized_byte_vector_values(
        &self,
        entry: &FieldEntry,
    ) -> Result<OffHeapScalarQuantizedVectorValues> {
        OffHeapScalarQuantizedVectorValues::load(entry.clone(), self.vector_data.clone_input()?)
    }

    /// Returns an independent view of this reader over the same files.
    ///
    /// Java's `FlatVectorsReader.getMergeInstance()` answers `this`, because a
    /// Java reader can be shared; here the merge instance is a clone whose
    /// inputs are fresh, so two threads never seek the same handle.
    fn clone_reader(&self) -> Result<Self> {
        Ok(Self {
            fields: self.fields.clone(),
            vector_data: Arc::from(self.vector_data.clone_input()?),
            raw_vectors_reader: self.raw_vectors_reader.get_merge_instance_flat()?,
            scorer: self.scorer.clone(),
        })
    }

    /// Returns the float view of `field`'s quantized vectors, dequantizing on
    /// each read.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field is not encoded
    /// as `FLOAT32`, and whatever opening the values raises.
    fn quantized_float_vector_values(
        &self,
        field: &str,
        entry: &FieldEntry,
    ) -> Result<Box<dyn FloatVectorValues>> {
        if entry.vector_encoding != VectorEncoding::FLOAT32 {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{field}\" is encoded as: {:?} expected: FLOAT32",
                entry.vector_encoding
            )));
        }
        OffHeapScalarQuantizedFloatVectorValues::load(
            &entry.ord_to_doc,
            entry.dimension,
            entry.size,
            entry.scalar_encoding,
            entry.similarity_function,
            Arc::clone(&self.scorer) as Arc<dyn FlatVectorsScorer>,
            entry.centroid.as_deref(),
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )
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
            return Err(LuceneError::illegal_state(format!(
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
            return Err(LuceneError::illegal_state(format!(
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
                LuceneError::illegal_state(format!(
                    "vector data length overflows for field=\"{}\"",
                    info.name
                ))
            })?;
        if num_quantized_vector_bytes != field_entry.vector_data_length {
            return Err(LuceneError::illegal_state(format!(
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
        Ok(Some(self.quantized_byte_vector_values(entry)?))
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
    /// For an asymmetric encoding, every vector is quantized a second time at
    /// the query width into a temporary `queries` file, and the supplier scores
    /// those query-side copies against the stored documents; closing the
    /// supplier deletes the file.
    ///
    /// # Errors
    ///
    /// Returns whatever opening the values, writing or opening the temporary
    /// file raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#getRandomVectorScorerSupplierForMerge`
    /// (`Lucene104ScalarQuantizedVectorsReader.java:453-524`).
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
        let float_vector_values =
            self.get_float_vector_values(&field_info.name)?
                .ok_or_else(|| {
                    LuceneError::illegal_state(format!(
                        "field=\"{}\" has quantized vectors but no float vectors",
                        field_info.name
                    ))
                })?;
        let quantizer = OptimizedScalarQuantizer::new(field_info.vector_similarity_function);
        let directory = Arc::clone(&segment_write_state.directory);
        let mut temp_output = directory.create_temp_output(
            &segment_write_state.segment_info.name,
            "queries",
            segment_write_state.context,
        )?;
        let temp_name = temp_output.name().to_string();
        let written = (|| -> Result<DocsWithFieldSet> {
            let docs_with_field = Self::write_binarized_query_data(
                &vector_values,
                scalar_encoding,
                temp_output.as_mut(),
                float_vector_values.as_ref(),
                &quantizer,
            )?;
            crate::codecs::codec_util::write_footer(temp_output.as_mut())?;
            temp_output.close()?;
            Ok(docs_with_field)
        })();
        let docs_with_field = match written {
            Ok(docs) => docs,
            Err(error) => {
                let _ = temp_output.close();
                let _ = directory.delete_file(&temp_name);
                return Err(error);
            }
        };
        drop(temp_output);
        let quantized_score_data_input =
            directory.open_input(&temp_name, segment_write_state.context)?;
        // Java hands `quantizedScoreDataInput` itself to the values and closes
        // it from the supplier's close action. The values here own their
        // input, so they read through a clone and the close action keeps the
        // original to close (`Lucene104ScalarQuantizedVectorsReader.java:
        // 490-521`). A failure on the way drops — closes — the input without
        // masking the error, as Java's `closeWhileSuppressingExceptions`.
        let values_input = quantized_score_data_input.clone_input()?;
        let score_vector_values = OffHeapScalarQuantizedVectorValues::load_query_side(
            fi,
            docs_with_field.cardinality(),
            values_input,
        );
        let total_vector_count = vector_values.size();
        let mut quantized_score_data_input = Some(quantized_score_data_input);
        let scorer_supplier = AsymmetricQuantizedRandomVectorScorerSupplier::new(
            score_vector_values,
            vector_values,
            field_info.vector_similarity_function,
        )?;
        Ok(Some(Box::new(crate::util::hnsw::scorer::create(
            Box::new(scorer_supplier),
            total_vector_count,
            Box::new(move || {
                // `Lucene104ScalarQuantizedVectorsReader.java:512-515`:
                // `IOUtils.close(quantizedScoreDataInput)` first, whose
                // exception propagates and skips the deletion; then
                // `IOUtils.deleteFilesIgnoringExceptions` on the temp file.
                if let Some(mut input) = quantized_score_data_input.take() {
                    input.close()?;
                }
                let _ = directory.delete_file(&temp_name);
                Ok(())
            }),
        ))))
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
            LuceneError::illegal_state(
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

/// Accounts for the field map and the delegate.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorsReader.ramBytesUsed()`:
/// `SHALLOW_SIZE + sizeOfMap(fields, shallowSizeOfInstance(FieldEntry.class)) +
/// rawVectorsReader.ramBytesUsed()`.
impl crate::util::Accountable for Lucene104ScalarQuantizedVectorsReader {
    fn ram_bytes_used(&self) -> i64 {
        let mut size = READER_SHALLOW_SIZE;
        size += crate::util::RamUsageEstimator::size_of_map(
            self.fields.len(),
            FIELD_ENTRY_SHALLOW_SIZE,
        );
        size += self.raw_vectors_reader.ram_bytes_used();
        size
    }
}

impl KnnVectorsReader for Lucene104ScalarQuantizedVectorsReader {
    /// Verifies both the delegate's files and the `.veq` checksum.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.checkIntegrity()`.
    fn check_integrity(&self) -> Result<()> {
        self.raw_vectors_reader.check_integrity()?;
        let mut data = self.vector_data.clone_input()?;
        crate::codecs::codec_util::checksum_entire_file(data.as_mut())?;
        Ok(())
    }

    /// Returns the float vectors of `field`.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.getFloatVectorValues(String)`:
    /// when the delegate stored the raw vectors they are the ones handed back,
    /// paired with the quantized values so that `scorer` runs off the compressed
    /// form; when the delegate stored none, the dequantized view stands in. A
    /// field this reader holds no entry for answers `None`, Java's `null`
    /// (`Lucene104ScalarQuantizedVectorsReader.java:230-234`).
    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        let Some(entry) = self.fields.get(field) else {
            return Ok(None);
        };
        if entry.vector_encoding != VectorEncoding::FLOAT32 {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{field}\" is encoded as: {:?} expected: FLOAT32",
                entry.vector_encoding
            )));
        }
        // Java dereferences the delegate's answer without a `null` check.
        let raw = self
            .raw_vectors_reader
            .get_float_vector_values(field)?
            .ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "the raw vectors of field \"{field}\" are missing"
                ))
            })?;
        if raw.size() == 0 {
            return self.quantized_float_vector_values(field, entry).map(Some);
        }
        Ok(Some(Box::new(ScalarQuantizedVectorValues::new(
            raw,
            Box::new(self.quantized_byte_vector_values(entry)?),
        ))))
    }

    /// Byte-encoded fields are never quantized by this format, so the delegate
    /// answers.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.getByteVectorValues(String)`.
    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        self.raw_vectors_reader.get_byte_vector_values(field)
    }

    /// Scores every accepted ordinal against the quantized vectors, in batches.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.search(String,
    /// float[], KnnCollector, AcceptDocs)`. There is no graph in this format, so
    /// the search is exhaustive; the batching exists so that a whole batch can
    /// be skipped when its best score is not competitive.
    fn search(
        &self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn crate::codecs::knn_vectors::KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        if knn_collector.k() == 0 {
            return Ok(());
        }
        let Some(mut scorer) = self.random_vector_scorer_float(field, target)? else {
            return Ok(());
        };
        let accepted = accept_docs.bits()?;
        let accepted_ords = scorer.get_accept_ords(accepted.as_deref());
        let mut ords = [0i32; EXHAUSTIVE_BULK_SCORE_ORDS];
        let mut scores = [0f32; EXHAUSTIVE_BULK_SCORE_ORDS];
        let mut num_ords = 0usize;
        let num_vectors = scorer.max_ord();
        for i in 0..num_vectors {
            if accepted_ords.as_deref().map_or(Ok(false), |accepted| {
                accepted.try_get(i as usize).map(|a| !a)
            })? {
                continue;
            }
            if knn_collector.early_terminated() {
                break;
            }
            ords[num_ords] = i;
            num_ords += 1;
            if num_ords == ords.len() {
                collect_batch(&mut scorer, &ords, &mut scores, num_ords, knn_collector)?;
                num_ords = 0;
            }
        }
        if num_ords > 0 {
            collect_batch(&mut scorer, &ords, &mut scores, num_ords, knn_collector)?;
        }
        Ok(())
    }

    /// Byte queries are not quantized by this format, so the delegate answers.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.search(String,
    /// byte[], KnnCollector, AcceptDocs)`.
    fn search_byte(
        &self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn crate::codecs::knn_vectors::KnnCollector,
        accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        self.raw_vectors_reader
            .search_byte(field, target, knn_collector, accept_docs)
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone_reader()?))
    }

    /// Closes the `.veq` input and the delegate.
    ///
    /// Equivalent to `IOUtils.close(quantizedVectorData, rawVectorsReader)`.
    fn close(&mut self) -> Result<()> {
        let data = match Arc::get_mut(&mut self.vector_data) {
            Some(input) => input.close(),
            // Another view of the same file is still alive, exactly as when
            // Java's `IndexInput` has been cloned; Lucene closes the original
            // and lets the clones fall away.
            None => Ok(()),
        };
        let delegate = self.raw_vectors_reader.close();
        data.and(delegate)
    }

    /// Adds this format's `.veq` bytes to what the delegate reports.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.getOffHeapByteSize(FieldInfo)`.
    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        let raw = self.raw_vectors_reader.get_off_heap_byte_size(field_info);
        let Some(entry) = self.fields.get(&field_info.name) else {
            // Java asserts the field is byte-encoded here: only float fields
            // are quantized, so only they have an entry.
            debug_assert_eq!(field_info.vector_encoding, VectorEncoding::BYTE);
            return raw;
        };
        let mut quant = HashMap::new();
        quant.insert(VECTOR_DATA_EXTENSION.to_string(), entry.vector_data_length);
        crate::codecs::knn_vectors::merge_off_heap_byte_size_maps(raw, quant)
    }

    /// This reader is a [`FlatVectorsReader`], so it answers with itself.
    fn as_flat_vectors_reader(&self) -> Option<&dyn FlatVectorsReader> {
        Some(self)
    }
}

impl FlatVectorsReader for Lucene104ScalarQuantizedVectorsReader {
    /// Every field of this format is scored by the quantized scorer.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.getFlatVectorScorer(String)`,
    /// which ignores the field and returns the one scorer it was built with.
    fn get_flat_vector_scorer(&self, _field: &str) -> Result<Box<dyn FlatVectorsScorer>> {
        Ok(Box::new(Lucene104ScalarQuantizedVectorScorer::new(
            self.scorer.get_non_quantized_delegate().clone(),
        )))
    }

    /// Returns a scorer of `target` against `field`'s quantized vectors.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.getRandomVectorScorer(String, float[])`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns `null` for a field this
    /// reader holds no entry for; the trait's return type is not optional, so
    /// the absent field is an `IllegalArgumentException`-shaped error instead.
    /// [`Self::random_vector_scorer_float`] keeps Java's optional form for the
    /// callers inside this file that need to tell the two apart.
    fn get_random_vector_scorer_float(
        &self,
        field: &str,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.random_vector_scorer_float(field, target)?
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!("field=\"{field}\" has no vector values"))
            })
    }

    /// Byte queries are not quantized, so the delegate answers.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsReader.getRandomVectorScorer(String, byte[])`.
    fn get_random_vector_scorer_byte(
        &self,
        field: &str,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.raw_vectors_reader
            .get_random_vector_scorer_byte(field, target)
    }

    fn get_merge_instance_flat(&self) -> Result<Box<dyn FlatVectorsReader>> {
        Ok(Box::new(self.clone_reader()?))
    }

    /// This reader stores quantized vectors, so it answers with itself.
    ///
    /// This is the `true` arm of Java's `flatVectorsReader instanceof
    /// QuantizedVectorsReader`.
    fn as_quantized_vectors_reader(&self) -> Option<&dyn QuantizedVectorsReader> {
        Some(self)
    }
}

/// Scores one batch of ordinals and collects them when the batch is
/// competitive.
///
/// Equivalent to the repeated block inside
/// `Lucene104ScalarQuantizedVectorsReader.search`: the visited count is charged
/// for the whole batch, and the batch is only collected when its best score
/// beats what the collector already holds.
fn collect_batch(
    scorer: &mut Box<dyn RandomVectorScorer>,
    ords: &[i32],
    scores: &mut [f32],
    num_ords: usize,
    knn_collector: &mut dyn crate::codecs::knn_vectors::KnnCollector,
) -> Result<()> {
    knn_collector.inc_visited_count(num_ords as i32);
    let best = scorer.bulk_score(ords, scores, num_ords as i32)?;
    if best > knn_collector.min_competitive_similarity() {
        for j in 0..num_ords {
            knn_collector.collect(scorer.ord_to_doc(ords[j]), scores[j]);
        }
    }
    Ok(())
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

    /// Returns the merge-time scorer supplier for `field_info`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsReader#getRandomVectorScorerSupplierForMerge`,
    /// forwarding to the inherent method of the same name.
    fn get_random_vector_scorer_supplier_for_merge(
        &self,
        field_info: &FieldInfo,
        segment_write_state: &SegmentWriteState<'_>,
    ) -> Result<Option<Box<dyn CloseableRandomVectorScorerSupplier>>> {
        Lucene104ScalarQuantizedVectorsReader::get_random_vector_scorer_supplier_for_merge(
            self,
            field_info,
            segment_write_state,
        )
    }
}

/// The scalar-quantized vectors format.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsFormat`:
/// a [`FlatVectorsFormat`] whose reader and writer wrap a
/// `Lucene99FlatVectorsFormat` delegate for the raw vectors.
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
    /// Port of `Lucene104ScalarQuantizedVectorsFormat.toString()`.
    ///
    /// `scorer` and `rawVectorFormat` are `static final` fields in Java
    /// (`Lucene104ScalarQuantizedVectorsFormat.java:48-51`), so they are the
    /// same two values for every instance; this port builds them where they are
    /// needed rather than holding them in the struct, and renders the same two
    /// here.
    ///
    /// `encoding` goes through the enum's own `toString()`, which for a Java
    /// enum is the constant name — `UNSIGNED_BYTE`, not the Rust variant
    /// spelling.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scorer = Lucene104ScalarQuantizedVectorScorer::new(std::sync::Arc::new(
            crate::codecs::hnsw::flat_vectors::DefaultFlatVectorScorer::INSTANCE,
        ));
        let raw = crate::codecs::lucene99::flat_vectors_format::Lucene99FlatVectorsFormat::new(
            crate::codecs::hnsw::flat_vectors::DefaultFlatVectorScorer::INSTANCE,
        );
        write!(
            f,
            "Lucene104ScalarQuantizedVectorsFormat(name={NAME}, encoding={}, flatVectorScorer={scorer}, rawVectorFormat={raw})",
            self.encoding
        )
    }
}

/// Builds the readers and writers of the scalar-quantized flat format.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorsFormat extends
/// FlatVectorsFormat`: both halves wrap the raw `Lucene99FlatVectorsFormat`,
/// which keeps the full-precision vectors while this format writes the
/// quantized ones.
impl KnnVectorsFormat for Lucene104ScalarQuantizedVectorsFormat {
    fn name(&self) -> &str {
        NAME
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        Ok(self.fields_writer_flat(&state.borrow())?)
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(self.fields_reader_flat(state)?)
    }

    fn get_max_dimensions(&self, field_name: &str) -> i32 {
        Lucene104ScalarQuantizedVectorsFormat::get_max_dimensions(self, field_name)
    }
}

impl FlatVectorsFormat for Lucene104ScalarQuantizedVectorsFormat {
    fn fields_writer_flat(
        &self,
        state: &SegmentWriteState<'_>,
    ) -> Result<Box<dyn FlatVectorsWriter>> {
        let raw = raw_vector_format().fields_writer_flat(state)?;
        Ok(Box::new(Lucene104ScalarQuantizedVectorsWriter::new(
            state,
            self.encoding,
            raw,
        )?))
    }

    fn fields_reader_flat<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn FlatVectorsReader>> {
        let raw = raw_vector_format().fields_reader_flat(state)?;
        Ok(Box::new(Lucene104ScalarQuantizedVectorsReader::new(
            state,
            raw,
            Arc::new(Lucene104ScalarQuantizedVectorScorer::new(Arc::new(
                crate::codecs::hnsw::flat_vectors::FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer(),
            ))),
        )?))
    }
}

/// The format that stores the unquantized vectors underneath this one.
///
/// Equivalent to Lucene's `private static final FlatVectorsFormat
/// rawVectorFormat = new Lucene99FlatVectorsFormat(
/// FlatVectorScorerUtil.getLucene99FlatVectorsScorer())`.
fn raw_vector_format() -> crate::codecs::lucene99::flat_vectors_format::Lucene99FlatVectorsFormat {
    crate::codecs::lucene99::flat_vectors_format::Lucene99FlatVectorsFormat::new(
        crate::codecs::hnsw::flat_vectors::FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer(),
    )
}

/// An HNSW format whose vectors are scalar-quantized.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104HnswScalarQuantizedVectorsFormat`,
/// which pairs the graph parameters of the HNSW format with the quantized flat
/// format above.
#[derive(Clone, Debug)]
pub struct Lucene104HnswScalarQuantizedVectorsFormat {
    max_conn: i32,
    beam_width: i32,
    /// Equivalent to `Lucene104HnswScalarQuantizedVectorsFormat.numMergeWorkers`.
    num_merge_workers: i32,
    /// Equivalent to `Lucene104HnswScalarQuantizedVectorsFormat.mergeExec`,
    /// the `TaskExecutor` wrapped around the `ExecutorService` the constructor
    /// is given (`Lucene104HnswScalarQuantizedVectorsFormat.java:183-187`).
    merge_exec: Option<Arc<crate::search::TaskExecutor>>,
    /// Equivalent to
    /// `Lucene104HnswScalarQuantizedVectorsFormat.tinySegmentsThreshold`.
    tiny_segments_threshold: i32,
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
            num_merge_workers:
                crate::codecs::lucene99::hnsw_vectors_format::DEFAULT_NUM_MERGE_WORKER,
            merge_exec: None,
            tiny_segments_threshold:
                crate::codecs::lucene99::hnsw_vectors_format::HNSW_GRAPH_THRESHOLD,
            flat: Lucene104ScalarQuantizedVectorsFormat::new(),
        }
    }

    /// Creates the format with explicit graph parameters, encoding, merge
    /// workers and merge executor.
    ///
    /// Equivalent to `Lucene104HnswScalarQuantizedVectorsFormat(ScalarEncoding,
    /// int, int, int, ExecutorService)` (`:133-140`) when
    /// `tiny_segments_threshold` is `HNSW_GRAPH_THRESHOLD`, and to the
    /// six-argument constructor (`:152-188`) otherwise; an `ExecutorService`
    /// is an [`Executor`](crate::search::Executor) here.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `max_conn` or
    /// `beam_width` is out of range, or when `num_merge_workers` is `1` and a
    /// `merge_exec` is given, as Java throws `IllegalArgumentException`.
    pub fn with_merge_exec(
        encoding: ScalarEncoding,
        max_conn: i32,
        beam_width: i32,
        num_merge_workers: i32,
        merge_exec: Option<Arc<dyn crate::search::Executor>>,
        tiny_segments_threshold: i32,
    ) -> Result<Self> {
        let mut format = Self::with_params(max_conn, beam_width, encoding)?;
        format.tiny_segments_threshold = tiny_segments_threshold;
        if num_merge_workers == 1 && merge_exec.is_some() {
            return Err(LuceneError::IllegalArgument(
                "No executor service is needed as we'll use single thread to merge".to_string(),
            ));
        }
        format.num_merge_workers = num_merge_workers;
        format.merge_exec =
            merge_exec.map(|executor| Arc::new(crate::search::TaskExecutor::new(executor)));
        Ok(format)
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
            num_merge_workers:
                crate::codecs::lucene99::hnsw_vectors_format::DEFAULT_NUM_MERGE_WORKER,
            merge_exec: None,
            tiny_segments_threshold:
                crate::codecs::lucene99::hnsw_vectors_format::HNSW_GRAPH_THRESHOLD,
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

/// The shallow size of `Lucene104ScalarQuantizedVectorsWriter` on the JVM.
///
/// Equivalent to Lucene's `SHALLOW_RAM_BYTES_USED`. Measured against
/// `lucene-core-10.5.0.jar`: 48.
pub const WRITER_SHALLOW_RAM_BYTES_USED: i64 = 48;

/// The shallow size of the writer's `FieldWriter` on the JVM, the first term of
/// `quantizationOverheadBytesUsed()`.
///
/// Measured against `lucene-core-10.5.0.jar`: 32.
pub const FIELD_WRITER_SHALLOW_SIZE: i64 = 32;

/// One float field this writer is buffering, together with the running
/// componentwise sums and magnitudes the flush needs.
///
/// Equivalent to the nested
/// `Lucene104ScalarQuantizedVectorsWriter.FieldWriter`.
struct SqFieldWriter {
    field_info: FieldInfo,
    state: Arc<Mutex<SqFieldState>>,
}

/// The buffer and the running sums one field accumulates while indexing.
///
/// Java keeps the buffer in the flat delegate's `FlatFieldVectorsWriter` and the
/// two running arrays in its own `FieldWriter`; both are reached from the same
/// `addValue`, so they live together here.
struct SqFieldState {
    vectors: DefaultFlatFieldVectorsWriter<Vec<f32>>,
    /// Componentwise sum of every vector added, which divided by the count is
    /// the cluster centre. For a cosine field the *normalised* vector is summed.
    ///
    /// Equivalent to Lucene's `float[] dimensionSums`.
    dimension_sums: Vec<f32>,
    /// The magnitude of each vector of a cosine field, kept so that flush can
    /// normalise in place without recomputing them.
    ///
    /// Equivalent to Lucene's `FloatArrayList magnitudes`.
    magnitudes: Vec<f32>,
}

impl SqFieldWriter {
    /// The heap this field writer costs beyond the buffered vectors.
    ///
    /// Equivalent to `FieldWriter.quantizationOverheadBytesUsed()`, which
    /// deliberately leaves out the flat field writer: the raw delegate already
    /// accounts for every buffered vector, and adding it here would count it
    /// twice.
    fn quantization_overhead_bytes_used(&self) -> i64 {
        let Ok(state) = self.state.lock() else {
            return FIELD_WRITER_SHALLOW_SIZE;
        };
        FIELD_WRITER_SHALLOW_SIZE
            + crate::util::RamUsageEstimator::shallow_size_of_array::<f32>(state.magnitudes.len())
            + crate::util::RamUsageEstimator::shallow_size_of_array::<f32>(
                state.dimension_sums.len(),
            )
    }
}

/// Builds the readers and writers that put an HNSW graph over the
/// scalar-quantized vectors.
///
/// Equivalent to `Lucene104HnswScalarQuantizedVectorsFormat extends
/// KnnVectorsFormat`: the graph is Lucene 9.9's, and only the flat format
/// underneath it changes.
impl KnnVectorsFormat for Lucene104HnswScalarQuantizedVectorsFormat {
    fn name(&self) -> &str {
        HNSW_NAME
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        let borrowed = state.borrow();
        let flat_writer = self.flat.fields_writer_flat(&borrowed)?;
        Ok(Box::new(
            crate::codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsWriter::with_flat_format(
                state,
                self.max_conn,
                self.beam_width,
                // `Lucene104HnswScalarQuantizedVectorsFormat.java:190-200`.
                self.num_merge_workers,
                self.merge_exec.clone(),
                self.tiny_segments_threshold,
                crate::codecs::lucene99::hnsw_vectors_format::VERSION_CURRENT,
                Box::new(self.flat),
                flat_writer,
            )?,
        ))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        let flat_reader = self.flat.fields_reader_flat(state)?;
        Ok(Box::new(
            crate::codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsReader::new(
                state,
                flat_reader,
            )?,
        ))
    }

    fn get_max_dimensions(&self, field_name: &str) -> i32 {
        Lucene104HnswScalarQuantizedVectorsFormat::get_max_dimensions(self, field_name)
    }
}

/// Quantizes and writes every vector of a field, in the order given.
///
/// Equivalent to the private
/// `Lucene104ScalarQuantizedVectorsWriter.writeVectors(FieldWriter, float[], OptimizedScalarQuantizer)`:
/// the packed components, then the lower and upper interval, the additional
/// correction, and the component sum.
fn write_quantized_vectors(
    encoding: ScalarEncoding,
    vectors: &mut [Vec<f32>],
    cluster_center: &[f32],
    quantizer: &OptimizedScalarQuantizer,
    vector_data: &mut dyn IndexOutput,
) -> Result<()> {
    let bits = encoding.get_bits();
    let dimension = cluster_center.len() as i32;
    let mut scratch = vec![0u8; encoding.get_discrete_dimensions(dimension).max(0) as usize];
    let mut packed = match encoding {
        ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => Vec::new(),
        ScalarEncoding::PackedNibble
        | ScalarEncoding::SingleBitQueryNibble
        | ScalarEncoding::DibitQueryNibble => {
            vec![0u8; encoding.get_doc_packed_length(scratch.len() as i32).max(0) as usize]
        }
    };
    for vector in vectors.iter_mut() {
        let corrections = quantizer.scalar_quantize(vector, &mut scratch, bits, cluster_center)?;
        let written = match encoding {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => &scratch[..],
            ScalarEncoding::PackedNibble => {
                OffHeapScalarQuantizedVectorValues::pack_nibbles(&scratch, &mut packed)?;
                &packed[..]
            }
            ScalarEncoding::SingleBitQueryNibble => {
                OptimizedScalarQuantizer::pack_as_binary(&scratch, &mut packed);
                &packed[..]
            }
            ScalarEncoding::DibitQueryNibble => {
                OptimizedScalarQuantizer::transpose_dibit(&scratch, &mut packed);
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

/// Writes one field's entry into the metadata file.
///
/// Equivalent to the private
/// `Lucene104ScalarQuantizedVectorsWriter.writeMeta(FieldInfo, int, long, long, float[], float, DocsWithFieldSet)`.
#[allow(clippy::too_many_arguments)]
fn write_meta_entry(
    encoding: ScalarEncoding,
    field: &FieldInfo,
    max_doc: i32,
    vector_data_offset: i64,
    vector_data_length: i64,
    cluster_center: &[f32],
    centroid_dp: f32,
    count: i32,
    docs_with_field: &DocsWithFieldSet,
    meta: &mut dyn IndexOutput,
    vector_data: &mut dyn IndexOutput,
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
        meta.write_v_int(encoding.get_wire_number())?;
        // The centroid is written as little-endian floats.
        for &c in cluster_center {
            meta.write_int(c.to_bits() as i32)?;
        }
        meta.write_int(centroid_dp.to_bits() as i32)?;
    }
    OrdToDocDISIReaderConfiguration::write_stored_meta(
        DIRECT_MONOTONIC_BLOCK_SHIFT,
        meta,
        vector_data,
        count,
        max_doc,
        docs_with_field,
    )
}

/// Writes the quantized vectors of a segment.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsWriter`.
///
/// **Divergence from Lucene 10.5.0.** Java's `addField` chains to
/// `rawVectorDelegate.addField(fieldInfo)` and keeps a reference to the
/// `FlatFieldVectorsWriter` the delegate created, so one buffer serves both.
/// A `Box<dyn KnnFieldVectorsWriter<T>>` cannot be read back through the trait
/// here, so this writer owns the buffer and hands the vectors to the delegate
/// at flush through [`BufferingKnnVectorsWriter::write_field_float`] — the same
/// arrangement `Lucene99HnswVectorsWriter` already uses in this crate, and the
/// same bytes in both files.
pub struct Lucene104ScalarQuantizedVectorsWriter {
    encoding: ScalarEncoding,
    meta: Option<Box<dyn IndexOutput>>,
    vector_data: Option<Box<dyn IndexOutput>>,
    /// The writer of the unquantized vectors.
    ///
    /// Equivalent to Lucene's `final FlatVectorsWriter rawVectorDelegate`.
    raw_vector_delegate: Option<Box<dyn FlatVectorsWriter>>,
    fields: Vec<SqFieldWriter>,
    /// Byte-encoded fields, which this format does not quantize; they are
    /// buffered here only so that they can be handed to the delegate at flush.
    byte_fields: Vec<BufferedByteField>,
    /// `maxDoc` of the segment, when it was already set when this writer was
    /// created; see the identical field on `Lucene99FlatVectorsWriter` for why
    /// it is optional.
    segment_max_doc: Option<i32>,
    finished: bool,
}

impl std::fmt::Debug for Lucene104ScalarQuantizedVectorsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene104ScalarQuantizedVectorsWriter")
            .field("encoding", &self.encoding)
            .field("fields", &self.fields.len())
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl Lucene104ScalarQuantizedVectorsWriter {
    /// Creates the writer of a segment: creates the `.vemq` and `.veq` files
    /// and writes their headers.
    ///
    /// # Errors
    ///
    /// Returns whatever creating the two outputs or writing their headers
    /// raises; both are closed before the error escapes.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsWriter(SegmentWriteState, ScalarEncoding, FlatVectorsWriter, Lucene104ScalarQuantizedVectorScorer)`.
    pub fn new(
        state: &SegmentWriteState<'_>,
        encoding: ScalarEncoding,
        raw_vector_delegate: Box<dyn FlatVectorsWriter>,
    ) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let vector_data_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            VECTOR_DATA_EXTENSION,
        );
        let mut meta_opt = Some(
            state
                .directory
                .create_output(&meta_file_name, state.context)?,
        );
        let mut vector_data_opt = Some(
            state
                .directory
                .create_output(&vector_data_file_name, state.context)?,
        );
        let result = (|| -> Result<Self> {
            let meta = meta_opt.as_mut().expect("INVARIANT: just created");
            let vector_data = vector_data_opt.as_mut().expect("INVARIANT: just created");
            crate::codecs::codec_util::write_index_header(
                meta.as_mut(),
                META_CODEC_NAME,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            crate::codecs::codec_util::write_index_header(
                vector_data.as_mut(),
                VECTOR_DATA_CODEC_NAME,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            Ok(Self {
                encoding,
                meta: meta_opt.take(),
                vector_data: vector_data_opt.take(),
                raw_vector_delegate: Some(raw_vector_delegate),
                fields: Vec::new(),
                byte_fields: Vec::new(),
                segment_max_doc: state.segment_info.max_doc().ok(),
                finished: false,
            })
        })();
        if result.is_err() {
            if let Some(mut meta) = meta_opt.take() {
                let _ = meta.close();
            }
            if let Some(mut data) = vector_data_opt.take() {
                let _ = data.close();
            }
        }
        result
    }

    /// Creates a writer that only carries the encoding, for the callers that
    /// drive the format-defining steps themselves.
    ///
    /// This is not a Lucene constructor; it exists so that
    /// [`Self::write_vectors`], [`Self::write_meta`] and the centroid helpers
    /// can be exercised without a segment.
    pub fn with_encoding(encoding: ScalarEncoding) -> Self {
        Self {
            encoding,
            meta: None,
            vector_data: None,
            raw_vector_delegate: None,
            fields: Vec::new(),
            byte_fields: Vec::new(),
            segment_max_doc: None,
            finished: false,
        }
    }

    /// Returns the encoding this writer quantizes with.
    pub fn encoding(&self) -> ScalarEncoding {
        self.encoding
    }

    /// Quantizes one field's buffered vectors and writes both its data and its
    /// metadata entry.
    ///
    /// Equivalent to the private
    /// `Lucene104ScalarQuantizedVectorsWriter.writeField(FieldWriter, float[], int, OptimizedScalarQuantizer)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the writer is already closed,
    /// and whatever quantizing or writing raises.
    fn write_field(
        &mut self,
        field_info: &FieldInfo,
        vectors: &mut [Vec<f32>],
        docs_with_field: &DocsWithFieldSet,
        cluster_center: &[f32],
        max_doc: i32,
        quantizer: &OptimizedScalarQuantizer,
    ) -> Result<()> {
        let encoding = self.encoding;
        let count = docs_with_field.cardinality();
        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::illegal_state(
                "Lucene104ScalarQuantizedVectorsWriter is already closed".to_string(),
            )
        })?;
        let vector_data_offset =
            vector_data.align_file_pointer(std::mem::size_of::<f32>() as i32)?;
        write_quantized_vectors(
            encoding,
            vectors,
            cluster_center,
            quantizer,
            vector_data.as_mut(),
        )?;
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        // Java: `!fieldData.getVectors().isEmpty() ? dotProduct(c, c) : 0`.
        let centroid_dp = if vectors.is_empty() {
            0.0
        } else {
            vector_util::dot_product_f32(cluster_center, cluster_center)?
        };
        let (meta, vector_data) = self.outputs()?;
        write_meta_entry(
            encoding,
            field_info,
            max_doc,
            vector_data_offset,
            vector_data_length,
            cluster_center,
            centroid_dp,
            count,
            docs_with_field,
            meta,
            vector_data,
        )
    }

    /// Returns the raw-vector delegate, or fails when there is none.
    fn delegate(&mut self) -> Result<&mut (dyn FlatVectorsWriter + 'static)> {
        match self.raw_vector_delegate.as_deref_mut() {
            Some(delegate) => Ok(delegate),
            None => Err(LuceneError::illegal_state(
                "Lucene104ScalarQuantizedVectorsWriter has no raw vector delegate".to_string(),
            )),
        }
    }

    /// Returns both outputs, or fails when the writer has been closed.
    fn outputs(&mut self) -> Result<(&mut dyn IndexOutput, &mut dyn IndexOutput)> {
        match (self.meta.as_mut(), self.vector_data.as_mut()) {
            (Some(meta), Some(data)) => Ok((meta.as_mut(), data.as_mut())),
            _ => Err(LuceneError::illegal_state(
                "Lucene104ScalarQuantizedVectorsWriter is already closed".to_string(),
            )),
        }
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
        write_quantized_vectors(
            self.encoding,
            vectors,
            cluster_center,
            quantizer,
            vector_data,
        )
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
        write_sorted_quantized_vectors(
            self.encoding,
            vectors,
            cluster_center,
            ord_map,
            scalar_quantizer,
            vector_data,
        )
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
        write_sorting_field_entry(
            self.encoding,
            vectors,
            field,
            max_doc,
            sort_map,
            docs_with_field,
            cluster_center,
            scalar_quantizer,
            meta,
            vector_data,
        )
    }

    /// Writes one field's entry into the metadata file.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.writeMeta`.
    #[allow(clippy::too_many_arguments)]
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
        write_meta_entry(
            self.encoding,
            field,
            max_doc,
            vector_data_offset,
            vector_data_length,
            cluster_center,
            centroid_dp,
            count,
            docs_with_field,
            meta,
            vector_data,
        )
    }

    /// Writes the quantized vectors `quantized_byte_vector_values` yields into
    /// `output`, one entry per document that has a vector, and returns those
    /// documents.
    ///
    /// This is the merge path, fed by [`QuantizedFloatVectorValues`], which
    /// quantizes each vector as it is asked for.
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
    /// `None` when the reader is not a quantized one.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsWriter.getCentroid(KnnVectorsReader, String)`
    /// (`Lucene104ScalarQuantizedVectorsWriter.java:388-396`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java narrows the reader with two
    /// `instanceof` tests — through `PerFieldKnnVectorsFormat.FieldsReader` to
    /// the field's own reader, then to `Lucene104ScalarQuantizedVectorsReader` —
    /// and reads that reader's stored centroid. Rust cannot narrow a trait
    /// object, so each test is asked of the reader instead:
    /// [`KnnVectorsReader::get_field_reader`] answers for the per-field reader,
    /// and [`KnnVectorsReader::as_flat_vectors_reader`] followed by
    /// [`FlatVectorsReader::as_quantized_vectors_reader`] for the quantized
    /// one. Each answers `None` exactly where Java's `instanceof` fails, so a
    /// reader of another format still yields `None` and
    /// [`merge_and_recalculate_centroids`](Self::merge_and_recalculate_centroids)
    /// recomputes the centroid, as in Java. Where the centroid itself is read
    /// from is the inline note below.
    pub fn get_centroid(
        vectors_reader: &dyn KnnVectorsReader,
        field_name: &str,
    ) -> Result<Option<Vec<f32>>> {
        // `vectorsReader instanceof PerFieldKnnVectorsFormat.FieldsReader`
        let reader = vectors_reader
            .get_field_reader(field_name)
            .unwrap_or(vectors_reader);
        // `vectorsReader instanceof Lucene104ScalarQuantizedVectorsReader`
        let Some(quantized) = reader
            .as_flat_vectors_reader()
            .and_then(FlatVectorsReader::as_quantized_vectors_reader)
        else {
            return Ok(None);
        };
        // **Divergence from Lucene 10.5.0.** Java reads
        // `fieldEntry.centroid` straight off the reader's field map. There is
        // no way to narrow a `&dyn KnnVectorsReader` to this crate's concrete
        // reader, so the centroid is taken from the field's quantized values,
        // which hold the very same array. The value is identical; only the
        // cost of opening the values is not.
        let Some(values) = quantized.get_quantized_vector_values(field_name)? else {
            return Ok(None);
        };
        Ok(values.get_centroid().map(<[f32]>::to_vec))
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
            // `Lucene104ScalarQuantizedVectorsWriter.java:404-406`: a segment
            // with no reader, or whose reader has no vectors for the field —
            // the per-field reader of a segment without it — is skipped.
            let Some(knn_vectors_reader) = reader_opt else {
                continue;
            };
            let Some(vector_values) =
                knn_vectors_reader.get_float_vector_values(&field_info.name)?
            else {
                continue;
            };
            let centroid = Self::get_centroid(knn_vectors_reader.as_ref(), &field_info.name)?;
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
            // `Lucene104ScalarQuantizedVectorsWriter.java:449-452`.
            let Some(vector_values) =
                knn_vectors_reader.get_float_vector_values(&field_info.name)?
            else {
                continue;
            };
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
impl KnnVectorsWriter for Lucene104ScalarQuantizedVectorsWriter {
    /// Starts buffering `field_info`'s vectors.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.addField(FieldInfo)`:
    /// a float field gets the quantizing writer, and a byte field is left to
    /// the raw delegate because this format never quantizes bytes.
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        match field_info.vector_encoding {
            VectorEncoding::FLOAT32 => {
                let state = Arc::new(Mutex::new(SqFieldState {
                    vectors: DefaultFlatFieldVectorsWriter::<Vec<f32>>::new(field_info.clone()),
                    dimension_sums: vec![0f32; field_info.vector_dimension.max(0) as usize],
                    magnitudes: Vec::new(),
                }));
                self.fields.push(SqFieldWriter {
                    field_info: field_info.clone(),
                    state: Arc::clone(&state),
                });
                Ok(FieldVectorWriter::Float(Box::new(SharedFloatFieldWriter {
                    inner: state,
                    similarity_function: field_info.vector_similarity_function,
                })))
            }
            VectorEncoding::BYTE => {
                let vectors = Arc::new(Mutex::new(DefaultFlatFieldVectorsWriter::<Vec<u8>>::new(
                    field_info.clone(),
                )));
                self.byte_fields
                    .push((field_info.clone(), Arc::clone(&vectors)));
                Ok(FieldVectorWriter::Byte(Box::new(SharedByteFieldWriter {
                    inner: vectors,
                })))
            }
        }
    }

    /// Writes every buffered field: the raw vectors through the delegate, then
    /// the quantized form and its metadata.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.flush(int, Sorter.DocMap)`.
    /// The cluster centre is the componentwise mean of the field's vectors,
    /// l2-normalised when the similarity is cosine — and for a cosine field the
    /// vectors themselves are normalised first, exactly as Java does before
    /// clustering.
    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        // Hand the raw vectors to the delegate first, as Java's
        // `rawVectorDelegate.flush(maxDoc, sortMap)` does with the buffer they
        // share.
        let byte_fields = std::mem::take(&mut self.byte_fields);
        for (info, writer) in byte_fields {
            let values = {
                let guard = lock_field(&writer)?;
                BufferedVectorValues::new(
                    info.vector_dimension,
                    guard.vectors().to_vec(),
                    guard.docs_with_field_set(),
                )?
            };
            self.delegate()?.write_field_byte(&info, &values, max_doc)?;
            lock_field(&writer)?.finish()?;
        }

        let fields = std::mem::take(&mut self.fields);
        for field in fields {
            let (mut vectors, docs_with_field, dimension_sums, magnitudes) = {
                let guard = lock_state(&field.state)?;
                (
                    guard.vectors.vectors().to_vec(),
                    guard.vectors.docs_with_field_set().clone(),
                    guard.dimension_sums.clone(),
                    guard.magnitudes.clone(),
                )
            };
            {
                let values = BufferedVectorValues::new(
                    field.field_info.vector_dimension,
                    vectors.clone(),
                    &docs_with_field,
                )?;
                self.delegate()?
                    .write_field_float(&field.field_info, &values, max_doc)?;
            }

            // After the raw vectors are written, normalise for clustering and
            // quantization.
            if field.field_info.vector_similarity_function == VectorSimilarityFunction::COSINE {
                normalize_vectors(&mut vectors, &magnitudes);
            }
            let vector_count = vectors.len();
            let mut cluster_center = vec![0f32; dimension_sums.len()];
            if vector_count > 0 {
                for (i, sum) in dimension_sums.iter().enumerate() {
                    cluster_center[i] = sum / vector_count as f32;
                }
                if field.field_info.vector_similarity_function == VectorSimilarityFunction::COSINE {
                    vector_util::l2normalize(&mut cluster_center, true)?;
                }
            }
            let quantizer =
                OptimizedScalarQuantizer::new(field.field_info.vector_similarity_function);
            match sort_map {
                None => {
                    let info = field.field_info.clone();
                    self.write_field(
                        &info,
                        &mut vectors,
                        &docs_with_field,
                        &cluster_center,
                        max_doc,
                        &quantizer,
                    )?;
                }
                Some(sort_map) => {
                    let encoding = self.encoding;
                    let info = field.field_info.clone();
                    let (meta, vector_data) = self.outputs()?;
                    write_sorting_field_entry(
                        encoding,
                        &mut vectors,
                        &info,
                        max_doc,
                        sort_map,
                        &docs_with_field,
                        &cluster_center,
                        &quantizer,
                        meta,
                        vector_data,
                    )?;
                }
            }
            lock_state(&field.state)?.vectors.finish()?;
        }
        self.delegate()?.flush(max_doc, sort_map)
    }

    /// Writes the end-of-fields marker and both footers, and finishes the
    /// delegate.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.finish()`.
    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::illegal_state("already finished".to_string()));
        }
        self.finished = true;
        if self.raw_vector_delegate.is_some() {
            self.delegate()?.finish()?;
        }
        if let Some(meta) = self.meta.as_mut() {
            meta.write_int(-1)?;
            crate::codecs::codec_util::write_footer(meta.as_mut())?;
        }
        if let Some(vector_data) = self.vector_data.as_mut() {
            crate::codecs::codec_util::write_footer(vector_data.as_mut())?;
        }
        Ok(())
    }

    fn merge_one_field<'m>(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &'m MergeState,
    ) -> Result<Option<Box<dyn crate::codecs::knn_vectors::IORunnable + 'm>>> {
        self.merge_one_flat_vector_field(field_info, merge_state)?;
        Ok(None)
    }

    /// Closes both outputs and the delegate.
    ///
    /// Equivalent to `IOUtils.close(meta, vectorData, rawVectorDelegate)`:
    /// everything is closed even when one of them fails, and the first failure
    /// is the one reported.
    fn close(&mut self) -> Result<()> {
        let meta = match self.meta.take() {
            Some(mut out) => out.close(),
            None => Ok(()),
        };
        let data = match self.vector_data.take() {
            Some(mut out) => out.close(),
            None => Ok(()),
        };
        let delegate = match self.raw_vector_delegate.as_mut() {
            Some(writer) => writer.close(),
            None => Ok(()),
        };
        meta.and(data).and(delegate)
    }

    /// Accounts for the delegate and this writer's quantization overhead.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.ramBytesUsed()`.
    /// The buffered vectors are charged once, by the delegate; each field adds
    /// only its magnitudes and dimension sums on top.
    fn ram_bytes_used(&self) -> i64 {
        let mut total = WRITER_SHALLOW_RAM_BYTES_USED;
        if let Some(delegate) = self.raw_vector_delegate.as_ref() {
            total += delegate.ram_bytes_used();
        }
        for field in &self.fields {
            total += field.quantization_overhead_bytes_used();
        }
        total
    }
}

impl BufferingKnnVectorsWriter for Lucene104ScalarQuantizedVectorsWriter {
    fn write_field_float(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn FloatVectorValues,
        max_doc: i32,
    ) -> Result<()> {
        let mut vectors = Vec::with_capacity(values.size().max(0) as usize);
        let mut docs_with_field = DocsWithFieldSet::new();
        let mut iter = values.iterator()?;
        while iter.next_doc()? != NO_MORE_DOCS {
            vectors.push(values.vector_value(iter.index())?);
            docs_with_field.add(iter.doc_id())?;
        }
        let cluster_center = Self::cluster_center(
            &vectors,
            field_info.vector_dimension.max(0) as usize,
            field_info.vector_similarity_function,
        )?;
        let quantizer = OptimizedScalarQuantizer::new(field_info.vector_similarity_function);
        self.write_field(
            field_info,
            &mut vectors,
            &docs_with_field,
            &cluster_center,
            max_doc,
            &quantizer,
        )
    }

    fn write_field_byte(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn ByteVectorValues,
        max_doc: i32,
    ) -> Result<()> {
        self.delegate()?
            .write_field_byte(field_info, values, max_doc)
    }
}

impl FlatVectorsWriter for Lucene104ScalarQuantizedVectorsWriter {
    fn get_flat_vector_scorer(&self) -> &dyn FlatVectorsScorer {
        &*QUANTIZED_WRITER_SCORER
    }

    /// Merges one field: the delegate writes the raw vectors, then the merged
    /// centroid is computed and every merged vector is quantized against it.
    ///
    /// Equivalent to
    /// `Lucene104ScalarQuantizedVectorsWriter.mergeOneFlatVectorField(FieldInfo, MergeState)`
    /// (`Lucene104ScalarQuantizedVectorsWriter.java:322-363`): the merged view
    /// [`MergedVectorValues::merge_float_vector_values`] builds — every
    /// surviving document, in merged order, under its merged document id — is
    /// wrapped in [`NormalizedFloatVectorValues`] for cosine and in
    /// [`QuantizedFloatVectorValues`], and streamed through
    /// [`write_vector_data`](Lucene104ScalarQuantizedVectorsWriter::write_vector_data),
    /// one vector quantized at a time.
    fn merge_one_flat_vector_field(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<()> {
        // Don't need access to the random vectors, we can just use the merged
        self.delegate()?
            .merge_one_flat_vector_field(field_info, merge_state)?;
        if field_info.vector_encoding != VectorEncoding::FLOAT32 {
            return Ok(());
        }
        let mut merged_centroid = vec![0f32; field_info.vector_dimension.max(0) as usize];
        let vector_count =
            Self::merge_and_recalculate_centroids(merge_state, field_info, &mut merged_centroid)?;
        let centroid = merged_centroid;
        // `Lucene104ScalarQuantizedVectorsWriter.java:333-336` reports on the
        // write state's info stream, which `SegmentMerger` builds from the
        // merge state's.
        if merge_state
            .info_stream
            .is_enabled(QUANTIZED_VECTOR_COMPONENT)
        {
            merge_state.info_stream.message(
                QUANTIZED_VECTOR_COMPONENT,
                &format!("Vectors' count:{vector_count}"),
            );
        }
        let mut float_vector_values =
            MergedVectorValues::merge_float_vector_values(field_info, merge_state)?;
        if field_info.vector_similarity_function == VectorSimilarityFunction::COSINE {
            float_vector_values = Box::new(NormalizedFloatVectorValues::new(float_vector_values)?);
        }
        let mut quantized_vector_values = QuantizedFloatVectorValues::new(
            float_vector_values,
            OptimizedScalarQuantizer::new(field_info.vector_similarity_function),
            self.encoding,
            centroid.clone(),
        )?;
        let max_doc = self
            .segment_max_doc
            .ok_or_else(|| LuceneError::illegal_state("maxDoc isn't set yet".to_string()))?;
        let encoding = self.encoding;
        let (meta, vector_data) = self.outputs()?;
        let vector_data_offset =
            vector_data.align_file_pointer(std::mem::size_of::<f32>() as i32)?;
        let docs_with_field = Self::write_vector_data(vector_data, &mut quantized_vector_values)?;
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        let centroid_dp = if docs_with_field.cardinality() > 0 {
            vector_util::dot_product_f32(&centroid, &centroid)?
        } else {
            0.0
        };
        write_meta_entry(
            encoding,
            field_info,
            max_doc,
            vector_data_offset,
            vector_data_length,
            &centroid,
            centroid_dp,
            docs_with_field.cardinality(),
            &docs_with_field,
            meta,
            vector_data,
        )
    }
}

/// Quantizes the vectors of the wrapped values against a centroid, one
/// ordinal at a time, as they are read.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.QuantizedFloatVectorValues`
/// (`Lucene104ScalarQuantizedVectorsWriter.java:583-696`). The quantized and
/// packed forms and the corrective terms are those of the last ordinal asked
/// for; [`get_corrective_terms`](QuantizedByteVectorValues::get_corrective_terms)
/// answers only for that ordinal. Java's `getQuantizer()` throws
/// `UnsupportedOperationException`; the port's getter keeps its default, which
/// answers that there is none.
pub struct QuantizedFloatVectorValues {
    corrections: Option<QuantizationResult>,
    quantized: Vec<u8>,
    /// The packed form, for the encodings that pack; for the others Java makes
    /// `packed` the same array as `quantized`, which `vector_value` reproduces
    /// by answering `quantized`.
    packed: Vec<u8>,
    centroid: Vec<f32>,
    centroid_dp: f32,
    values: Box<dyn FloatVectorValues>,
    quantizer: OptimizedScalarQuantizer,
    encoding: ScalarEncoding,
    last_ord: i32,
}

impl QuantizedFloatVectorValues {
    /// Wraps `delegate`, whose vectors are quantized with `quantizer` and
    /// `encoding` against `centroid`.
    ///
    /// Equivalent to `QuantizedFloatVectorValues(FloatVectorValues,
    /// OptimizedScalarQuantizer, ScalarEncoding, float[])`.
    ///
    /// # Errors
    ///
    /// Returns the error of [`KnnVectorValues::try_dimension`] when `delegate`
    /// is a merged view without subs, where Java's `delegate.dimension()`
    /// (`Lucene104ScalarQuantizedVectorsWriter.java:603`) throws
    /// `IndexOutOfBoundsException`. The centroid's dot product with itself
    /// cannot fail: `VectorUtil.dotProduct` rejects only vectors of unequal
    /// length.
    pub fn new(
        delegate: Box<dyn FloatVectorValues>,
        quantizer: OptimizedScalarQuantizer,
        encoding: ScalarEncoding,
        centroid: Vec<f32>,
    ) -> Result<Self> {
        let quantized = vec![
            0u8;
            encoding
                .get_discrete_dimensions(delegate.try_dimension()?)
                .max(0) as usize
        ];
        let packed = match encoding {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => Vec::new(),
            ScalarEncoding::PackedNibble
            | ScalarEncoding::SingleBitQueryNibble
            | ScalarEncoding::DibitQueryNibble => {
                vec![
                    0u8;
                    encoding
                        .get_doc_packed_length(quantized.len() as i32)
                        .max(0) as usize
                ]
            }
        };
        let centroid_dp = vector_util::dot_product_f32(&centroid, &centroid)?;
        Ok(Self {
            corrections: None,
            quantized,
            packed,
            centroid,
            centroid_dp,
            values: delegate,
            quantizer,
            encoding,
            last_ord: -1,
        })
    }

    /// Quantizes the vector at `ord` into `quantized`, and packs it.
    ///
    /// Equivalent to the private `QuantizedFloatVectorValues.quantize(int)`.
    fn quantize(&mut self, ord: i32) -> Result<()> {
        let mut vector = self.values.vector_value(ord)?;
        self.corrections = Some(self.quantizer.scalar_quantize(
            &mut vector,
            &mut self.quantized,
            self.encoding.get_bits(),
            &self.centroid,
        )?);
        match self.encoding {
            ScalarEncoding::PackedNibble => {
                OffHeapScalarQuantizedVectorValues::pack_nibbles(&self.quantized, &mut self.packed)?
            }
            ScalarEncoding::SingleBitQueryNibble => {
                OptimizedScalarQuantizer::pack_as_binary(&self.quantized, &mut self.packed)
            }
            ScalarEncoding::DibitQueryNibble => {
                OptimizedScalarQuantizer::transpose_dibit(&self.quantized, &mut self.packed)
            }
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => {}
        }
        Ok(())
    }
}

impl QuantizedByteVectorValues for QuantizedFloatVectorValues {
    /// Equivalent to `QuantizedFloatVectorValues.vectorValue(int)`.
    fn vector_value(&mut self, ord: i32) -> Result<&[u8]> {
        if ord != self.last_ord {
            self.quantize(ord)?;
            self.last_ord = ord;
        }
        Ok(match self.encoding {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => &self.quantized,
            ScalarEncoding::PackedNibble
            | ScalarEncoding::SingleBitQueryNibble
            | ScalarEncoding::DibitQueryNibble => &self.packed,
        })
    }

    /// Equivalent to `QuantizedFloatVectorValues.getCorrectiveTerms(int)`.
    fn get_corrective_terms(&mut self, ord: i32) -> Result<QuantizationResult> {
        match self.corrections {
            Some(corrections) if ord == self.last_ord => Ok(corrections),
            _ => Err(LuceneError::illegal_state(format!(
                "attempt to retrieve corrective terms for different ord {ord} than the \
                 quantization was done for: {}",
                self.last_ord
            ))),
        }
    }

    /// The port's trait requires this accessor, which
    /// `QuantizedByteVectorValues` in Lucene 10.5.0 does not declare
    /// (`QuantizedByteVectorValues.java:205-234`); this view has no such value.
    fn get_score_correction_constant(&mut self, _ord: i32) -> Result<f32> {
        Err(LuceneError::UnsupportedOperation(
            "QuantizedFloatVectorValues has no score correction constant".to_string(),
        ))
    }

    fn size(&self) -> i32 {
        self.values.size()
    }

    fn dimension(&self) -> i32 {
        self.values.dimension()
    }

    /// Equivalent to `QuantizedFloatVectorValues.ordToDoc(int)`, which forwards
    /// to the values being quantized (`Lucene104ScalarQuantizedVectorsWriter.java:692-695`).
    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    /// Equivalent to the inherited `KnnVectorValues.getAcceptOrds(Bits)`,
    /// which maps through the forwarded `ordToDoc`.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }

    /// Equivalent to `QuantizedFloatVectorValues.copy()`.
    fn copy(&self) -> Result<Box<dyn QuantizedByteVectorValues>> {
        Ok(Box::new(Self::new(
            self.values.copy_float()?,
            self.quantizer,
            self.encoding,
            self.centroid.clone(),
        )?))
    }

    fn get_scalar_encoding(&self) -> ScalarEncoding {
        self.encoding
    }

    fn get_centroid(&self) -> Option<&[f32]> {
        Some(&self.centroid)
    }

    fn get_centroid_dp(&self) -> f32 {
        self.centroid_dp
    }

    /// Equivalent to `QuantizedFloatVectorValues.iterator()`: the wrapped
    /// values' iterator.
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        self.values.iterator()
    }
}

/// Normalises each vector of the wrapped values to unit length as it is read.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.NormalizedFloatVectorValues`
/// (`Lucene104ScalarQuantizedVectorsWriter.java:698-738`).
pub struct NormalizedFloatVectorValues {
    values: Box<dyn FloatVectorValues>,
}

impl NormalizedFloatVectorValues {
    /// Wraps `values`.
    ///
    /// Equivalent to `NormalizedFloatVectorValues(FloatVectorValues)`, which
    /// sizes its scratch vector with `values.dimension()`
    /// (`Lucene104ScalarQuantizedVectorsWriter.java:702-705`). The port needs
    /// no scratch vector but keeps the lookup, so that the merge fails at the
    /// same point.
    ///
    /// # Errors
    ///
    /// Returns the error of [`KnnVectorValues::try_dimension`] when `values` is
    /// a merged view without subs, where Java's `dimension()` throws
    /// `IndexOutOfBoundsException`.
    pub fn new(values: Box<dyn FloatVectorValues>) -> Result<Self> {
        values.try_dimension()?;
        Ok(Self { values })
    }
}

impl KnnVectorValues for NormalizedFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.values.dimension()
    }

    fn try_dimension(&self) -> Result<i32> {
        self.values.try_dimension()
    }

    fn size(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(Self::new(self.values.copy_float()?)?))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        self.values.iterator()
    }
}

impl FloatVectorValues for NormalizedFloatVectorValues {
    /// Equivalent to `NormalizedFloatVectorValues.vectorValue(int)`: a copy of
    /// the wrapped vector, normalised with `VectorUtil.l2normalize(float[])`.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        let mut normalized_vector = self.values.vector_value(ord)?;
        vector_util::l2normalize(&mut normalized_vector, true)?;
        Ok(normalized_vector)
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(Self::new(self.values.copy_float()?)?))
    }
}

/// The scorer a quantized writer reports, which is the same one the format
/// builds its readers with.
static QUANTIZED_WRITER_SCORER: std::sync::LazyLock<Lucene104ScalarQuantizedVectorScorer> =
    std::sync::LazyLock::new(|| {
        Lucene104ScalarQuantizedVectorScorer::new(Arc::new(
            crate::codecs::hnsw::flat_vectors::FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer(),
        ))
    });

/// Writes one field's vectors in the order an index sort put their documents
/// in, and its metadata entry.
///
/// Equivalent to the private
/// `Lucene104ScalarQuantizedVectorsWriter.writeSortingField`.
#[allow(clippy::too_many_arguments)]
fn write_sorting_field_entry(
    encoding: ScalarEncoding,
    vectors: &mut [Vec<f32>],
    field: &FieldInfo,
    max_doc: i32,
    sort_map: &SorterDocMap,
    docs_with_field: &DocsWithFieldSet,
    cluster_center: &[f32],
    quantizer: &OptimizedScalarQuantizer,
    meta: &mut dyn IndexOutput,
    vector_data: &mut dyn IndexOutput,
) -> Result<()> {
    let old_docs = docs_with_field_to_docs(docs_with_field)?;
    let (_old_to_new, new_to_old) =
        crate::codecs::knn_vectors::map_old_ord_to_new_ord(&old_docs, sort_map)?;
    let new_docs_with_field = new_docs_with_field(&old_docs, sort_map)?;

    let vector_data_offset = vector_data.align_file_pointer(std::mem::size_of::<f32>() as i32)?;
    write_sorted_quantized_vectors(
        encoding,
        vectors,
        cluster_center,
        &new_to_old,
        quantizer,
        vector_data,
    )?;
    let vector_data_length = vector_data.file_pointer() - vector_data_offset;
    let centroid_dp = vector_util::dot_product_f32(cluster_center, cluster_center)?;
    write_meta_entry(
        encoding,
        field,
        max_doc,
        vector_data_offset,
        vector_data_length,
        cluster_center,
        centroid_dp,
        new_docs_with_field.cardinality(),
        &new_docs_with_field,
        meta,
        vector_data,
    )
}

/// Quantizes and writes the vectors in the order `ord_map` gives.
///
/// Equivalent to the private
/// `Lucene104ScalarQuantizedVectorsWriter.writeSortedVectors`.
fn write_sorted_quantized_vectors(
    encoding: ScalarEncoding,
    vectors: &mut [Vec<f32>],
    cluster_center: &[f32],
    ord_map: &[i32],
    quantizer: &OptimizedScalarQuantizer,
    vector_data: &mut dyn IndexOutput,
) -> Result<()> {
    let bits = encoding.get_bits();
    let dimension = cluster_center.len() as i32;
    let mut scratch = vec![0u8; encoding.get_discrete_dimensions(dimension).max(0) as usize];
    let mut packed = match encoding {
        ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => Vec::new(),
        ScalarEncoding::PackedNibble
        | ScalarEncoding::SingleBitQueryNibble
        | ScalarEncoding::DibitQueryNibble => {
            vec![0u8; encoding.get_doc_packed_length(scratch.len() as i32).max(0) as usize]
        }
    };
    let count = vectors.len();
    for &ordinal in ord_map {
        let index = usize::try_from(ordinal).unwrap_or(usize::MAX);
        if index >= count {
            return Err(LuceneError::IllegalArgument(format!(
                "ordinal {ordinal} is out of range for {count} vectors"
            )));
        }
        let corrections =
            quantizer.scalar_quantize(&mut vectors[index], &mut scratch, bits, cluster_center)?;
        let written = match encoding {
            ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => &scratch[..],
            ScalarEncoding::PackedNibble => {
                OffHeapScalarQuantizedVectorValues::pack_nibbles(&scratch, &mut packed)?;
                &packed[..]
            }
            ScalarEncoding::SingleBitQueryNibble => {
                OptimizedScalarQuantizer::pack_as_binary(&scratch, &mut packed);
                &packed[..]
            }
            ScalarEncoding::DibitQueryNibble => {
                OptimizedScalarQuantizer::transpose_dibit(&scratch, &mut packed);
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

/// Normalises every vector in place, dividing by the magnitude recorded when it
/// was added.
///
/// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.FieldWriter.normalizeVectors()`,
/// which reuses the magnitudes instead of recomputing them.
fn normalize_vectors(vectors: &mut [Vec<f32>], magnitudes: &[f32]) {
    for (i, vector) in vectors.iter_mut().enumerate() {
        let Some(&magnitude) = magnitudes.get(i) else {
            continue;
        };
        for value in vector.iter_mut() {
            // Java divides — `vector[j] /= magnitude` — and a multiply by the
            // reciprocal is a different float, which would show up in the
            // quantized bytes.
            *value /= magnitude;
        }
    }
}

/// Locks a shared field buffer.
fn lock_field<T>(
    writer: &Arc<Mutex<DefaultFlatFieldVectorsWriter<T>>>,
) -> Result<std::sync::MutexGuard<'_, DefaultFlatFieldVectorsWriter<T>>> {
    writer.lock().map_err(|_| {
        LuceneError::illegal_state("FlatFieldVectorsWriter mutex was poisoned".to_string())
    })
}

/// Locks a shared float field's buffer and running sums.
fn lock_state(state: &Arc<Mutex<SqFieldState>>) -> Result<std::sync::MutexGuard<'_, SqFieldState>> {
    state.lock().map_err(|_| {
        LuceneError::illegal_state("FlatFieldVectorsWriter mutex was poisoned".to_string())
    })
}

/// One byte-encoded field this writer is holding for the delegate: the field it
/// belongs to, and the buffer its values land in.
type BufferedByteField = (
    FieldInfo,
    Arc<Mutex<DefaultFlatFieldVectorsWriter<Vec<u8>>>>,
);

/// The buffered vectors of one field, presented as vector values whose iterator
/// yields the documents that actually carried them.
///
/// The delegate is handed the vectors through
/// [`BufferingKnnVectorsWriter::write_field_float`], which reads the documents
/// off the iterator; a dense view built from the vector list alone would tell
/// it every document has a vector and write the wrong `DocsWithFieldSet` into
/// the `.vec`. Java never faces this, because the delegate owns the very
/// `FlatFieldVectorsWriter` the values were added to.
struct BufferedVectorValues<T> {
    dimension: i32,
    vectors: Vec<T>,
    /// Shared, so that the `getAcceptOrds` view can map through it.
    ord_to_doc: Arc<Vec<i32>>,
}

impl<T> BufferedVectorValues<T> {
    fn new(dimension: i32, vectors: Vec<T>, docs_with_field: &DocsWithFieldSet) -> Result<Self> {
        Ok(Self {
            dimension,
            vectors,
            ord_to_doc: Arc::new(docs_with_field_to_docs(docs_with_field)?),
        })
    }
}

impl<T: Send + Sync + Clone + 'static> KnnVectorValues for BufferedVectorValues<T> {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.vectors.len() as i32
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.ord_to_doc[ord as usize]
    }

    /// Equivalent to the inherited `KnnVectorValues.getAcceptOrds(Bits)`,
    /// which maps every ordinal through `ordToDoc`.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        let ord_to_doc = Arc::clone(&self.ord_to_doc);
        accept_ords_with(
            self.size(),
            move |ord| ord_to_doc[ord as usize],
            accept_docs,
        )
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(Self {
            dimension: self.dimension,
            vectors: self.vectors.clone(),
            ord_to_doc: self.ord_to_doc.clone(),
        }))
    }

    fn encoding(&self) -> VectorEncoding {
        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<Vec<f32>>() {
            VectorEncoding::FLOAT32
        } else {
            VectorEncoding::BYTE
        }
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(
            crate::codecs::lucene99::hnsw_vectors_format::VecDocIndexIterator::new(
                self.ord_to_doc.as_ref().clone(),
            ),
        ))
    }
}

impl FloatVectorValues for BufferedVectorValues<Vec<f32>> {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        Ok(self.vectors[ord as usize].clone())
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(Self {
            dimension: self.dimension,
            vectors: self.vectors.clone(),
            ord_to_doc: self.ord_to_doc.clone(),
        }))
    }
}

impl ByteVectorValues for BufferedVectorValues<Vec<u8>> {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        Ok(self.vectors[ord as usize].clone())
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(Self {
            dimension: self.dimension,
            vectors: self.vectors.clone(),
            ord_to_doc: self.ord_to_doc.clone(),
        }))
    }
}

/// The handle `add_field` hands back for a float field: every value added
/// through it lands in the buffer this writer flushes, and updates the running
/// componentwise sums the centroid is computed from.
struct SharedFloatFieldWriter {
    inner: Arc<Mutex<SqFieldState>>,
    similarity_function: VectorSimilarityFunction,
}

impl KnnFieldVectorsWriter<Vec<f32>> for SharedFloatFieldWriter {
    /// Buffers `vector_value` and folds it into the running sums.
    ///
    /// Equivalent to `Lucene104ScalarQuantizedVectorsWriter.FieldWriter.addValue(int, float[])`:
    /// a cosine field sums the vector divided by its own magnitude and records
    /// that magnitude, so the flush can normalise the buffer in place; every
    /// other similarity sums the vector as it is.
    fn add_value(&mut self, doc_id: i32, vector_value: Vec<f32>) -> Result<()> {
        let mut state = lock_state(&self.inner)?;
        if self.similarity_function == VectorSimilarityFunction::COSINE {
            let dp = vector_util::dot_product_f32(&vector_value, &vector_value)?;
            let divisor = dp.sqrt();
            state.magnitudes.push(divisor);
            for (i, &v) in vector_value.iter().enumerate() {
                if let Some(slot) = state.dimension_sums.get_mut(i) {
                    *slot += v / divisor;
                }
            }
        } else {
            for (i, &v) in vector_value.iter().enumerate() {
                if let Some(slot) = state.dimension_sums.get_mut(i) {
                    *slot += v;
                }
            }
        }
        state.vectors.add_value(doc_id, vector_value)
    }

    /// Java's `FieldWriter.copyValue` throws `UnsupportedOperationException`;
    /// the vector is already owned here, so there is nothing to copy and the
    /// value is returned unchanged.
    fn copy_value(&self, vector_value: Vec<f32>) -> Vec<f32> {
        vector_value
    }

    fn ram_bytes_used(&self) -> i64 {
        // The buffer is accounted once, by the writer that owns it.
        0
    }
}

/// The handle `add_field` hands back for a byte field.
struct SharedByteFieldWriter {
    inner: Arc<Mutex<DefaultFlatFieldVectorsWriter<Vec<u8>>>>,
}

impl KnnFieldVectorsWriter<Vec<u8>> for SharedByteFieldWriter {
    fn add_value(&mut self, doc_id: i32, vector_value: Vec<u8>) -> Result<()> {
        lock_field(&self.inner)?.add_value(doc_id, vector_value)
    }

    fn copy_value(&self, vector_value: Vec<u8>) -> Vec<u8> {
        vector_value
    }

    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

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

impl std::fmt::Display for Lucene104ScalarQuantizedVectorScorer {
    /// Port of `Lucene104ScalarQuantizedVectorScorer.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lucene104ScalarQuantizedVectorScorer(nonQuantizedDelegate={})",
            self.non_quantized_delegate
        )
    }
}

impl std::fmt::Display for Lucene104HnswScalarQuantizedVectorsFormat {
    /// Port of `Lucene104HnswScalarQuantizedVectorsFormat.toString()`.
    ///
    /// The `name=` is a literal in Java, not `getName()`, and the flat format
    /// is rendered by its own `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lucene104HnswScalarQuantizedVectorsFormat(name=Lucene104HnswScalarQuantizedVectorsFormat, maxConn={}, beamWidth={}, tinySegmentsThreshold={}, flatVectorFormat={})",
            self.max_conn,
            self.beam_width,
            self.tiny_segments_threshold,
            self.flat
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::knn_vectors::MergedFloat32VectorValues;
    use crate::index::vector_values::from_floats;

    /// Writes `max_doc` documents of one three-dimensional field through this
    /// format, every document when `dense`, every third otherwise, and opens
    /// the segment's reader.
    fn dequantized_values(
        dense: bool,
        max_doc: i32,
    ) -> (
        Box<dyn FloatVectorValues>,
        Vec<i32>,
        VectorSimilarityFunction,
    ) {
        use crate::codecs::knn_vectors::FieldVectorWriter;
        use crate::codecs::stub::BufferedUpdates;
        use crate::index::{FieldInfos, SegmentInfo};
        use crate::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
        use crate::util::{default_info_stream, Version};

        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let similarity = VectorSimilarityFunction::EUCLIDEAN;
        let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let info = SegmentInfo::new(
            Arc::clone(&directory),
            Version::LATEST,
            Some(Version::LATEST),
            "_dq".to_string(),
            max_doc,
            false,
            false,
            crate::codecs::default_codec().unwrap(),
            HashMap::new(),
            [7u8; 16],
            HashMap::new(),
            Default::default(),
        )
        .unwrap();
        let mut field_info = FieldInfo::new("v", 0);
        field_info.vector_dimension = 3;
        field_info.vector_encoding = VectorEncoding::FLOAT32;
        field_info.vector_similarity_function = similarity;
        let infos = FieldInfos::new(vec![field_info.clone()]).unwrap();
        let write_state = SegmentWriteState::new(
            default_info_stream(),
            Arc::clone(&directory),
            &info,
            &infos,
            Box::leak(Box::new(BufferedUpdates::default())),
            &*DEFAULT_IO_CONTEXT,
        );
        let format = Lucene104ScalarQuantizedVectorsFormat::new();
        let mut writer = format.fields_writer_flat(&write_state).unwrap();
        let FieldVectorWriter::Float(mut field_writer) = writer.add_field(&field_info).unwrap()
        else {
            panic!("a float field writer");
        };
        let mut docs = Vec::new();
        for doc in 0..max_doc {
            if dense || doc % 3 == 0 {
                let f = doc as f32;
                field_writer
                    .add_value(doc, vec![f * 0.5, 2.0 - f * 0.25, (doc % 4) as f32])
                    .unwrap();
                docs.push(doc);
            }
        }
        writer.flush(max_doc, None).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();

        let read_state =
            SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
        let raw = raw_vector_format().fields_reader_flat(&read_state).unwrap();
        let reader = Lucene104ScalarQuantizedVectorsReader::new(
            &read_state,
            raw,
            Arc::new(Lucene104ScalarQuantizedVectorScorer::new(Arc::new(
                crate::codecs::hnsw::flat_vectors::FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer(),
            ))),
        )
        .unwrap();
        let entry = reader.fields.get("v").unwrap();
        let values = reader.quantized_float_vector_values("v", entry).unwrap();
        (values, docs, similarity)
    }

    /// The dequantized values of the 10.4 format — what Lucene hands back when
    /// the raw vectors are absent (`Lucene104ScalarQuantizedVectorsReader.java:246-258`)
    /// — score through `OffHeapScalarQuantizedFloatVectorValues.scorer`
    /// (`OffHeapScalarQuantizedFloatVectorValues.java:243-265, 342-364`): the
    /// reader's scorer hands the float copy to its unquantized delegate, so
    /// every document scores `similarity(target, vectorValue(ord))`, one by one
    /// and in bulk, dense and sparse; and `getAcceptOrds` maps through the
    /// ordinal-to-document reader.
    #[test]
    fn the_dequantized_values_score_their_own_vectors() {
        use crate::search::NO_MORE_DOCS;
        let target = [3.0f32, 1.0, 2.0];
        for dense in [true, false] {
            let (values, docs, similarity) = dequantized_values(dense, 20);
            assert_eq!(values.size() as usize, docs.len());
            let expected: Vec<(i32, f32)> = (0..values.size())
                .map(|ord| {
                    let vector = values.vector_value(ord).unwrap();
                    (
                        docs[ord as usize],
                        similarity.compare_f32(&target, &vector).unwrap(),
                    )
                })
                .collect();

            let mut scorer = values.scorer(&target).unwrap().expect("a scorer");
            let mut one = Vec::new();
            loop {
                let doc = scorer.iterator().next_doc().unwrap();
                if doc == NO_MORE_DOCS {
                    break;
                }
                one.push((doc, scorer.score().unwrap()));
            }
            assert_eq!(one, expected, "dense={dense}: one by one");

            let mut bulk = values.scorer(&target).unwrap().unwrap().bulk(None).unwrap();
            let mut buffer = crate::index::DocAndFloatFeatureBuffer::new();
            let mut all = Vec::new();
            loop {
                let max = bulk
                    .next_docs_and_scores(NO_MORE_DOCS, None, &mut buffer)
                    .unwrap();
                if buffer.size == 0 {
                    break;
                }
                let batch: Vec<(i32, f32)> = (0..buffer.size)
                    .map(|i| (buffer.docs[i], buffer.features[i]))
                    .collect();
                let batch_max = batch
                    .iter()
                    .map(|(_, s)| *s)
                    .fold(f32::NEG_INFINITY, f32::max);
                assert_eq!(max, batch_max, "dense={dense}: bulk max");
                all.extend(batch);
            }
            assert_eq!(all, expected, "dense={dense}: bulk");

            let accept = crate::util::MatchAllBits::new(20);
            let view = values.get_accept_ords(Some(&accept)).unwrap();
            assert_eq!(view.length(), docs.len());
            #[derive(Debug)]
            struct Only(i32);
            impl crate::util::Bits for Only {
                fn get(&self, index: usize) -> bool {
                    index as i32 == self.0
                }
                fn length(&self) -> usize {
                    20
                }
            }
            let last = *docs.last().unwrap();
            let only_last = Only(last);
            let view = values.get_accept_ords(Some(&only_last)).unwrap();
            let accepted: Vec<usize> = (0..docs.len()).filter(|&ord| view.get(ord)).collect();
            assert_eq!(accepted, vec![docs.len() - 1], "dense={dense}");
        }
    }

    /// Wrapping a merged view without subs fails where Java's constructors
    /// call `dimension()` on it and throw: `NormalizedFloatVectorValues`
    /// (`Lucene104ScalarQuantizedVectorsWriter.java:702-705`) and
    /// `QuantizedFloatVectorValues` (`:603`); over values that have vectors,
    /// both wrappers build.
    #[test]
    fn the_merge_wrappers_refuse_an_empty_merged_view() {
        let state = MergeState::from_fields_producers(Vec::new(), Vec::new());
        let empty = || {
            Box::new(MergedFloat32VectorValues::new(Vec::new(), &state).unwrap())
                as Box<dyn FloatVectorValues>
        };
        let quantize = |values: Box<dyn FloatVectorValues>| {
            QuantizedFloatVectorValues::new(
                values,
                OptimizedScalarQuantizer::new(VectorSimilarityFunction::EUCLIDEAN),
                ScalarEncoding::UnsignedByte,
                vec![0.0, 0.0],
            )
        };
        assert!(matches!(
            NormalizedFloatVectorValues::new(empty()),
            Err(LuceneError::IllegalArgument(_))
        ));
        assert!(matches!(
            quantize(empty()),
            Err(LuceneError::IllegalArgument(_))
        ));

        // Positive control: one vector of two dimensions.
        let values =
            || Box::new(from_floats(vec![vec![3.0, 4.0]], 2)) as Box<dyn FloatVectorValues>;
        let normalized = NormalizedFloatVectorValues::new(values()).unwrap();
        let mut expected = vec![3.0, 4.0];
        vector_util::l2normalize(&mut expected, true).unwrap();
        assert_eq!(normalized.vector_value(0).unwrap(), expected);
        assert_eq!(quantize(values()).unwrap().dimension(), 2);
    }

    /// `Lucene104HnswScalarQuantizedVectorsFormat(ScalarEncoding, int, int,
    /// int, ExecutorService, int)` (`:152-188`) rejects an executor for a
    /// single worker, keeps one for several, and prints its threshold.
    #[test]
    fn hnsw_sq_format_takes_merge_workers_and_executor_as_lucene_does() {
        let executor: Arc<dyn crate::search::Executor> =
            Arc::new(crate::util::SameThreadExecutorService::new());
        let error = Lucene104HnswScalarQuantizedVectorsFormat::with_merge_exec(
            ScalarEncoding::UnsignedByte,
            16,
            100,
            1,
            Some(Arc::clone(&executor)),
            100,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            LuceneError::IllegalArgument(
                "No executor service is needed as we'll use single thread to merge".to_string()
            )
            .to_string()
        );
        let format = Lucene104HnswScalarQuantizedVectorsFormat::with_merge_exec(
            ScalarEncoding::UnsignedByte,
            16,
            100,
            2,
            Some(executor),
            7,
        )
        .unwrap();
        assert_eq!(format.num_merge_workers, 2);
        assert!(format.merge_exec.is_some());
        assert!(format.to_string().contains("tinySegmentsThreshold=7,"));
        assert!(Lucene104HnswScalarQuantizedVectorsFormat::new()
            .to_string()
            .contains("tinySegmentsThreshold=100,"));
    }
}
