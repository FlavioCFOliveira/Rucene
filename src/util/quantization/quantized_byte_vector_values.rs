//! `QuantizedByteVectorValues` and friends, ported from
//! `org.apache.lucene.util.quantization`.

use crate::error::Result;
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
}
