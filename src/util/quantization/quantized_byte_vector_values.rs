//! `QuantizedByteVectorValues` and friends, ported from
//! `org.apache.lucene.util.quantization`.

use crate::codecs::state::SegmentWriteState;
use crate::error::{LuceneError, Result};
use crate::index::field_infos::FieldInfo;
use crate::index::vector_values::DocIndexIterator;
use crate::store::IndexInput;
use crate::util::hnsw::CloseableRandomVectorScorerSupplier;
use crate::util::quantization::optimized_scalar_quantizer::QuantizationResult;
use crate::util::quantization::scalar_quantizer::ScalarQuantizer;

/// How many bits each quantized component occupies, and how they are packed.
///
/// Equivalent to `QuantizedByteVectorValues.ScalarEncoding`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarEncoding {
    /// One signed byte per component.
    UnsignedByte,
    /// Four bits per component, one component per byte.
    SevenBit,
    /// Four bits per component, two components packed per byte.
    PackedNibble,
}

impl ScalarEncoding {
    /// Returns how many bits one component occupies.
    ///
    /// Equivalent to `ScalarEncoding.getBits()`.
    pub fn get_bits(self) -> u8 {
        match self {
            Self::UnsignedByte => 8,
            Self::SevenBit => 7,
            Self::PackedNibble => 4,
        }
    }

    /// Returns how many bytes a vector of `dimension` components occupies.
    ///
    /// Equivalent to `ScalarEncoding.getDocLength(int)`.
    pub fn get_doc_length(self, dimension: usize) -> usize {
        match self {
            Self::PackedNibble => dimension.div_ceil(2),
            _ => dimension,
        }
    }

    /// Returns the encoding a code names, as written into the segment metadata.
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::UnsignedByte),
            1 => Some(Self::SevenBit),
            2 => Some(Self::PackedNibble),
            _ => None,
        }
    }

    /// Returns the code this encoding is written as.
    pub fn code(self) -> i32 {
        match self {
            Self::UnsignedByte => 0,
            Self::SevenBit => 1,
            Self::PackedNibble => 2,
        }
    }

    /// Returns how many bits one component of a *query* vector occupies.
    ///
    /// Every encoding this port models quantizes queries exactly as it
    /// quantizes documents, so this is [`get_bits`](Self::get_bits).
    ///
    /// Equivalent to `ScalarEncoding.getQueryBits()`.
    pub fn get_query_bits(self) -> u8 {
        match self {
            Self::UnsignedByte => 8,
            Self::SevenBit => 7,
            Self::PackedNibble => 4,
        }
    }

    /// Returns whether queries are quantized to a different width than
    /// documents.
    ///
    /// Equivalent to `ScalarEncoding.isAsymmetric()`.
    pub fn is_asymmetric(self) -> bool {
        self.get_bits() != self.get_query_bits()
    }

    /// Returns how many components of a document vector share one byte.
    ///
    /// This is not the same as [`get_bits`](Self::get_bits): `SevenBit` stores
    /// seven significant bits in a whole byte, so it packs eight bits per
    /// dimension.
    ///
    /// Equivalent to `ScalarEncoding.getDocBitsPerDim()`.
    pub fn get_doc_bits_per_dim(self) -> i32 {
        match self {
            Self::UnsignedByte => 8,
            Self::SevenBit => 8,
            Self::PackedNibble => 4,
        }
    }

    /// Returns how many components of a query vector share one byte.
    ///
    /// Equivalent to `ScalarEncoding.getQueryBitsPerDim()`.
    pub fn get_query_bits_per_dim(self) -> i32 {
        self.get_doc_bits_per_dim()
    }

    /// Returns `dimensions` rounded up so that the packed vector fills whole
    /// bytes.
    ///
    /// Equivalent to `ScalarEncoding.getDiscreteDimensions(int)`.
    pub fn get_discrete_dimensions(self, dimensions: i32) -> i32 {
        let bits_per_dim = self.get_doc_bits_per_dim();
        let total_bits = dimensions * bits_per_dim;
        (total_bits + 7) / 8 * 8 / bits_per_dim
    }

    /// Returns how many bytes a packed document vector of `dimensions`
    /// components occupies.
    ///
    /// Equivalent to `ScalarEncoding.getDocPackedLength(int)`.
    pub fn get_doc_packed_length(self, dimensions: i32) -> i32 {
        let discretized = self.get_discrete_dimensions(dimensions);
        let total_bits = discretized * self.get_doc_bits_per_dim();
        (total_bits + 7) / 8
    }

    /// Returns how many bytes a packed query vector of `dimensions` components
    /// occupies.
    ///
    /// Equivalent to `ScalarEncoding.getQueryPackedLength(int)`.
    pub fn get_query_packed_length(self, dimensions: i32) -> i32 {
        let discretized = self.get_discrete_dimensions(dimensions);
        let total_bits = discretized * self.get_query_bits_per_dim();
        (total_bits + 7) / 8
    }
}

/// A sequence of quantized vectors, each with its corrective offset.
///
/// Equivalent to `org.apache.lucene.util.quantization.QuantizedByteVectorValues`.
pub trait QuantizedByteVectorValues {
    /// Returns the quantized vector at `ord`.
    ///
    /// Equivalent to `QuantizedByteVectorValues.vectorValue(int)`.
    fn vector_value(&mut self, ord: i32) -> Result<&[u8]>;

    /// Returns the corrective offset stored for `ord`.
    ///
    /// Equivalent to `QuantizedByteVectorValues.getScoreCorrectionConstant(int)`.
    fn get_score_correction_constant(&mut self, ord: i32) -> Result<f32>;

    /// Returns how many vectors the sequence holds.
    fn size(&self) -> i32;

    /// Returns how many components each vector has.
    fn dimension(&self) -> i32;

    /// Returns the quantizer these vectors were compressed with.
    ///
    /// Equivalent to `QuantizedByteVectorValues.getScalarQuantizer()`.
    fn get_scalar_quantizer(&self) -> Option<&ScalarQuantizer> {
        None
    }

    /// Returns how the components are packed.
    fn get_scalar_encoding(&self) -> ScalarEncoding {
        ScalarEncoding::UnsignedByte
    }

    /// Returns the four corrective terms stored for `ord`: the lower and upper
    /// end of the optimized interval, the additional correction, and the sum of
    /// the quantized components.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] unless the implementation
    /// overrides this method — Java declares it abstract, which Rust cannot do
    /// without breaking implementors that predate it — or an I/O error raised
    /// while reading the terms.
    ///
    /// Equivalent to `QuantizedByteVectorValues.getCorrectiveTerms(int)`.
    fn get_corrective_terms(&mut self, _ord: i32) -> Result<QuantizationResult> {
        Err(LuceneError::UnsupportedOperation(
            "QuantizedByteVectorValues::get_corrective_terms".to_string(),
        ))
    }

    /// Returns the centroid every vector of this field was quantized against,
    /// or `None` when the field stores none.
    ///
    /// Equivalent to `QuantizedByteVectorValues.getCentroid()`.
    fn get_centroid(&self) -> Option<&[f32]> {
        None
    }

    /// Returns the dot product of the centroid with itself.
    ///
    /// Equivalent to `QuantizedByteVectorValues.getCentroidDP()`.
    fn get_centroid_dp(&self) -> f32 {
        0.0
    }

    /// Returns an iterator over the documents that have a vector, pairing each
    /// document id with the ordinal of its vector.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] unless the implementation
    /// overrides this method, which is what `KnnVectorValues.iterator()` does
    /// in Java, or an I/O error raised while opening the doc-id encoding.
    ///
    /// Equivalent to `KnnVectorValues.iterator()`, which
    /// `QuantizedByteVectorValues` inherits.
    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Err(LuceneError::UnsupportedOperation(
            "QuantizedByteVectorValues::iterator".to_string(),
        ))
    }
}

/// The shared state every on-disk `QuantizedByteVectorValues` carries.
///
/// Equivalent to
/// `org.apache.lucene.util.quantization.BaseQuantizedByteVectorValues`, which
/// Java makes an abstract class supplying `dimension` and `size`. Rust has no
/// implementation inheritance, so the port is a struct a concrete values type
/// holds.
#[derive(Clone, Copy, Debug)]
pub struct BaseQuantizedByteVectorValues {
    /// How many components each vector has.
    pub dimension: i32,
    /// How many vectors the sequence holds.
    pub size: i32,
}

impl BaseQuantizedByteVectorValues {
    /// Creates the shared state.
    pub fn new(dimension: i32, size: i32) -> Self {
        Self { dimension, size }
    }
}

/// A vectors reader that can hand out the quantized form and its quantizer.
///
/// Equivalent to `org.apache.lucene.util.quantization.QuantizedVectorsReader`.
pub trait QuantizedVectorsReader {
    /// Returns the quantized vectors stored for `field`.
    ///
    /// Equivalent to `QuantizedVectorsReader.getQuantizedVectorValues(String)`.
    fn get_quantized_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn QuantizedByteVectorValues>>>;

    /// Returns the quantizer `field` was compressed with.
    ///
    /// Equivalent to `QuantizedVectorsReader.getQuantizationState(String)`.
    fn get_quantization_state(&self, field: &str) -> Result<Option<ScalarQuantizer>>;

    /// Returns a scorer supplier over the merged quantized vectors of
    /// `field_info`, so a merge can score them however it needs to.
    ///
    /// The supplier may hold temporary file handles over auxiliary structures,
    /// so it should be closed once the merge is done.
    ///
    /// # Errors
    ///
    /// Returns whatever building the supplier raises.
    ///
    /// Equivalent to
    /// `QuantizedVectorsReader.getRandomVectorScorerSupplierForMerge(FieldInfo, SegmentWriteState)`.
    fn get_random_vector_scorer_supplier_for_merge(
        &self,
        field_info: &FieldInfo,
        segment_write_state: &SegmentWriteState<'_>,
    ) -> Result<Option<Box<dyn CloseableRandomVectorScorerSupplier>>>;
}

/// Quantized vectors written by the Lucene 9.9 format, which stores one
/// corrective constant per vector rather than the four terms the 10.4 format
/// stores.
///
/// Equivalent to
/// `org.apache.lucene.util.quantization.LegacyQuantizedByteVectorValues`, which
/// Java makes an abstract class over `BaseQuantizedByteVectorValues`. Rust has
/// no implementation inheritance, so the port is a trait carrying the two
/// methods that class adds: the quantizer, which the legacy format does store,
/// and the per-vector correction.
pub trait LegacyQuantizedByteVectorValues: QuantizedByteVectorValues {
    /// Returns the quantizer these vectors were compressed with.
    ///
    /// Equivalent to `LegacyQuantizedByteVectorValues.getScalarQuantizer()`,
    /// which the base class refuses; a legacy reader always has one.
    fn scalar_quantizer(&self) -> &ScalarQuantizer;

    /// Returns how many bytes one stored vector occupies, which is smaller
    /// than the dimension when four-bit components are packed two to a byte.
    ///
    /// Equivalent to `KnnVectorValues.getVectorByteLength()`, whose default for
    /// a byte-encoded field is one byte per dimension.
    fn vector_byte_length(&self) -> i32 {
        self.dimension()
    }

    /// Returns the input the vectors are read from, when there is one.
    ///
    /// Equivalent to `HasIndexSlice.getSlice()`, which returns `null` unless
    /// the values are backed by a file.
    fn get_slice(&self) -> Option<&dyn IndexInput> {
        None
    }

    /// Returns the input the vectors are read from, so a caller can seek in it.
    ///
    /// **Divergence from Lucene 10.5.0.** Java has only `getSlice()`: seeking
    /// mutates the `IndexInput` through the reference it returns. Rust needs an
    /// explicit `&mut` to do the same, so the mutable accessor is separate.
    fn get_slice_mut(&mut self) -> Option<&mut dyn IndexInput> {
        None
    }
}
