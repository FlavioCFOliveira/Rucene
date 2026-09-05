//! `QuantizedByteVectorValues` and friends, ported from
//! `org.apache.lucene.util.quantization`.

use crate::codecs::state::SegmentWriteState;
use crate::error::{LuceneError, Result};
use crate::index::field_infos::FieldInfo;
use crate::index::vector_values::DocIndexIterator;
use crate::search::VectorScorer;
use crate::store::IndexInput;
use crate::util::hnsw::CloseableRandomVectorScorerSupplier;
use crate::util::quantization::optimized_scalar_quantizer::{
    OptimizedScalarQuantizer, QuantizationResult,
};
use crate::util::quantization::scalar_quantizer::ScalarQuantizer;

/// How many bits each quantized component occupies, and how they are packed.
///
/// Equivalent to `QuantizedByteVectorValues.ScalarEncoding`.
///
/// **Divergence from Lucene 10.5.0.** Java declares five encodings; this port
/// models the three symmetric ones. The two missing ones,
/// `SINGLE_BIT_QUERY_NIBBLE` (wire number 3) and `DIBIT_QUERY_NIBBLE` (wire
/// number 4), quantize a query to four bits while quantizing a document to one
/// or two, so they need the asymmetric query path
/// (`OptimizedScalarQuantizer.packAsBinary` / `transposeDibit` and the
/// query-packed off-heap values) that this port does not have. A segment
/// written by Lucene with either of them is therefore rejected by
/// [`from_wire_number`](Self::from_wire_number) rather than misread. See
/// `QuantizedByteVectorValues.java:60-112` in Lucene 10.5.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarEncoding {
    /// Each dimension is quantized to 8 bits and treated as an unsigned value.
    UnsignedByte,
    /// Each dimension is quantized to 4 bits and two values are packed into
    /// each output byte.
    PackedNibble,
    /// Each dimension is quantized to 7 bits and treated as a signed value.
    ///
    /// This exists for backwards compatibility with older iterations of scalar
    /// quantization: it produces an index the same size as
    /// [`UnsignedByte`](Self::UnsignedByte) but less accurate comparisons.
    SevenBit,
}

impl std::fmt::Display for ScalarEncoding {
    /// A Java enum's `toString()` returns its constant name, and Lucene relies
    /// on that in `Lucene104ScalarQuantizedVectorsFormat.toString()`, so the
    /// constant names are reproduced here rather than the Rust variant names
    /// (`UNSIGNED_BYTE`, not `UnsignedByte`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsignedByte => "UNSIGNED_BYTE",
            Self::PackedNibble => "PACKED_NIBBLE",
            Self::SevenBit => "SEVEN_BIT",
        })
    }
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

    /// Returns the encoding a wire number names, or `None` when no encoding
    /// carries it.
    ///
    /// The wire number is the value the codec writes into the segment metadata;
    /// it is deliberately not the variant's ordinal, so that the order of the
    /// variants can change without changing the file format.
    ///
    /// Equivalent to `QuantizedByteVectorValues.ScalarEncoding.fromWireNumber(int)`.
    pub fn from_wire_number(wire_number: i32) -> Option<Self> {
        match wire_number {
            0 => Some(Self::UnsignedByte),
            1 => Some(Self::PackedNibble),
            2 => Some(Self::SevenBit),
            _ => None,
        }
    }

    /// Returns the number used to identify this encoding on the wire.
    ///
    /// The values are Lucene's own and were measured from
    /// `lucene-core-10.5.0.jar`: `UNSIGNED_BYTE` is 0, `PACKED_NIBBLE` is 1 and
    /// `SEVEN_BIT` is 2.
    ///
    /// Equivalent to `QuantizedByteVectorValues.ScalarEncoding.getWireNumber()`.
    pub fn get_wire_number(self) -> i32 {
        match self {
            Self::UnsignedByte => 0,
            Self::PackedNibble => 1,
            Self::SevenBit => 2,
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
///
/// This trait carried a `get_scalar_quantizer` returning a [`ScalarQuantizer`]
/// until rmp task #173, documented as "equivalent to
/// `QuantizedByteVectorValues.getScalarQuantizer()`". Lucene 10.5.0 declares no
/// such method on this class: `getScalarQuantizer()` belongs to
/// `LegacyQuantizedByteVectorValues`
/// (`LegacyQuantizedByteVectorValues.java:29-31`), where the port already has
/// it as [`LegacyQuantizedByteVectorValues::scalar_quantizer`]. The duplicate
/// had no implementor and no caller, so it was removed rather than
/// re-documented. The quantizer this class really exposes is the
/// [`OptimizedScalarQuantizer`] of [`get_quantizer`](Self::get_quantizer).
pub trait QuantizedByteVectorValues: Send + Sync {
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

    /// Returns a scorer for `query`, or `None` when there is none.
    ///
    /// Java reaches this through inheritance: `QuantizedByteVectorValues`
    /// extends `BaseQuantizedByteVectorValues`, so every implementation
    /// responds to `scorer(float[])` and the ones that can score a float query
    /// against their quantized bytes override it. Rust has no implementation
    /// inheritance, so the overridable form is this provided trait method,
    /// whose default is the base behaviour —
    /// [`BaseQuantizedByteVectorValues::scorer`], which throws.
    ///
    /// # Errors
    ///
    /// The default returns [`LuceneError::UnsupportedOperation`]; an
    /// implementation that overrides it propagates its own I/O errors.
    fn scorer(&self, query: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        BaseQuantizedByteVectorValues::new(self.dimension(), self.size()).scorer(query)
    }

    /// Returns a second view of the same vectors, so that two ordinals can be
    /// held at once without one read clobbering the other's scratch buffer.
    ///
    /// Equivalent to the abstract `QuantizedByteVectorValues.copy()`.
    ///
    /// # Errors
    ///
    /// The default reports [`LuceneError::UnsupportedOperation`], which is what
    /// an implementation that cannot be duplicated should keep; every
    /// file-backed implementation overrides it.
    fn copy(&self) -> Result<Box<dyn QuantizedByteVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "these quantized vector values cannot be copied".to_string(),
        ))
    }

    /// Returns the quantizer these vectors were compressed with, or `None`
    /// when the implementation does not hold one.
    ///
    /// Equivalent to the abstract
    /// `QuantizedByteVectorValues.getQuantizer()`, which every real
    /// implementation answers. The `Option` is the provided-method form Rust
    /// needs for a trait with no implementation inheritance; a value-holding
    /// implementation must override it.
    fn get_quantizer(&self) -> Option<&OptimizedScalarQuantizer> {
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

    /// Returns a scorer for `query`, which the shared state never has.
    ///
    /// Equivalent to
    /// `org.apache.lucene.util.quantization.BaseQuantizedByteVectorValues#scorer(float[])`
    /// (`BaseQuantizedByteVectorValues.java:38-40`), whose body is
    /// `throw new UnsupportedOperationException()`: the method exists so that a
    /// concrete values type that *can* score a float query against its
    /// quantized bytes may override it — `Lucene104ScalarQuantizedVectorsReader`
    /// is the caller that relies on such an override
    /// (`Lucene104ScalarQuantizedVectorsReader.java:662`).
    ///
    /// [`QuantizedByteVectorValues::scorer`] is the overridable form; this is
    /// the base behaviour it falls back to.
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`], which is the
    /// exception Java throws.
    pub fn scorer(&self, query: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        let _ = query;
        Err(LuceneError::UnsupportedOperation(
            "BaseQuantizedByteVectorValues does not score float queries".to_string(),
        ))
    }

    /// Returns the input the vectors are read from, which the shared state
    /// never has.
    ///
    /// Equivalent to
    /// `org.apache.lucene.util.quantization.BaseQuantizedByteVectorValues#getSlice`,
    /// which overrides `HasIndexSlice.getSlice()` to answer `null`. A concrete
    /// values type that *is* backed by a file answers with its own slice —
    /// [`LegacyQuantizedByteVectorValues::get_slice`] is one such override —
    /// exactly as a Java subclass overrides this method.
    pub fn get_slice(&self) -> Option<&dyn IndexInput> {
        None
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

#[cfg(test)]
mod scorer_tests {
    use super::*;
    use crate::index::VectorSimilarityFunction;
    use crate::search::DocIdSetIterator;

    /// The smallest `QuantizedByteVectorValues` there is: it supplies only what
    /// the trait requires and overrides nothing.
    struct Plain {
        base: BaseQuantizedByteVectorValues,
        bytes: Vec<u8>,
    }

    impl QuantizedByteVectorValues for Plain {
        fn vector_value(&mut self, ord: i32) -> Result<&[u8]> {
            let dim = self.base.dimension as usize;
            let start = ord as usize * dim;
            Ok(&self.bytes[start..start + dim])
        }

        fn get_score_correction_constant(&mut self, _ord: i32) -> Result<f32> {
            Ok(0.0)
        }

        fn size(&self) -> i32 {
            self.base.size
        }

        fn dimension(&self) -> i32 {
            self.base.dimension
        }
    }

    /// A values type that holds a quantizer, standing for
    /// `OffHeapScalarQuantizedVectorValues`, whose `getQuantizer()` returns the
    /// `new OptimizedScalarQuantizer(fi.similarityFunction)` its reader built.
    struct Quantized(OptimizedScalarQuantizer);

    impl QuantizedByteVectorValues for Quantized {
        fn vector_value(&mut self, _ord: i32) -> Result<&[u8]> {
            Ok(&[])
        }

        fn get_score_correction_constant(&mut self, _ord: i32) -> Result<f32> {
            Ok(0.0)
        }

        fn size(&self) -> i32 {
            1
        }

        fn dimension(&self) -> i32 {
            2
        }

        fn get_quantizer(&self) -> Option<&OptimizedScalarQuantizer> {
            Some(&self.0)
        }
    }

    /// `QuantizedByteVectorValues.getQuantizer()` is abstract in Lucene, so
    /// every real implementation answers with the quantizer the field's
    /// similarity determines. Rust cannot make a trait method abstract without
    /// breaking implementors, so it is provided and answers `None`; what
    /// matters is that an implementor's override is what a consumer sees
    /// through the trait object — which is the shape Java's
    /// `Lucene104ScalarQuantizedVectorScorer` uses (`qv.getQuantizer()` on a
    /// `QuantizedByteVectorValues`).
    #[test]
    fn get_quantizer_is_visible_through_the_trait_object() {
        let plain = Plain {
            base: BaseQuantizedByteVectorValues::new(2, 1),
            bytes: vec![0, 0],
        };
        let plain: &dyn QuantizedByteVectorValues = &plain;
        assert!(
            plain.get_quantizer().is_none(),
            "an implementation that holds no quantizer answers None"
        );

        for similarity in [
            VectorSimilarityFunction::EUCLIDEAN,
            VectorSimilarityFunction::DOT_PRODUCT,
            VectorSimilarityFunction::COSINE,
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        ] {
            let values = Quantized(OptimizedScalarQuantizer::new(similarity));
            let values: &dyn QuantizedByteVectorValues = &values;
            let quantizer = values
                .get_quantizer()
                .expect("a value-holding implementation answers its quantizer");

            // Identity is asserted behaviourally: quantizing the same vector
            // with the returned quantizer and with a freshly built one for the
            // same similarity must give the same corrections. `OptimizedScalar\
            // Quantizer` has no equality, and comparing addresses would prove
            // nothing about which similarity it was built for.
            let mut a = vec![1.0f32, -2.0];
            let mut b = vec![1.0f32, -2.0];
            let mut dst_a = vec![0u8; 2];
            let mut dst_b = vec![0u8; 2];
            let centroid = [0.5f32, 0.5];
            let from_trait = quantizer
                .scalar_quantize(&mut a, &mut dst_a, 8, &centroid)
                .expect("quantize");
            let from_fresh = OptimizedScalarQuantizer::new(similarity)
                .scalar_quantize(&mut b, &mut dst_b, 8, &centroid)
                .expect("quantize");
            assert_eq!(
                from_trait, from_fresh,
                "getQuantizer() must answer the quantizer of the field's similarity ({similarity:?})"
            );
            assert_eq!(dst_a, dst_b);
        }
    }

    /// A values type that *can* score a float query, standing for the
    /// `Lucene104ScalarQuantizedVectorsReader` override.
    struct Scoring;

    struct ConstantScorer(f32);

    impl VectorScorer for ConstantScorer {
        fn score(&mut self) -> Result<f32> {
            Ok(self.0)
        }

        fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
            unimplemented!("not exercised by this test")
        }

        fn bulk(
            self: Box<Self>,
            _matching_docs: Option<Box<dyn DocIdSetIterator>>,
        ) -> Result<Box<dyn crate::search::VectorScorerBulk>> {
            unimplemented!("not exercised by this test")
        }
    }

    impl QuantizedByteVectorValues for Scoring {
        fn vector_value(&mut self, _ord: i32) -> Result<&[u8]> {
            Ok(&[])
        }

        fn get_score_correction_constant(&mut self, _ord: i32) -> Result<f32> {
            Ok(0.0)
        }

        fn size(&self) -> i32 {
            1
        }

        fn dimension(&self) -> i32 {
            2
        }

        fn scorer(&self, query: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
            Ok(Some(Box::new(ConstantScorer(query.iter().sum()))))
        }
    }

    /// `BaseQuantizedByteVectorValues.scorer(float[])` throws
    /// `UnsupportedOperationException`.
    #[test]
    fn base_scorer_is_unsupported() {
        let base = BaseQuantizedByteVectorValues::new(4, 2);
        let err = match base.scorer(&[1.0, 2.0, 3.0, 4.0]) {
            Err(e) => e,
            Ok(_) => panic!("the base must not produce a scorer"),
        };
        assert!(
            matches!(err, LuceneError::UnsupportedOperation(_)),
            "expected UnsupportedOperation, got {err:?}"
        );
        // The base also has no slice, the other thing Java's base answers.
        assert!(base.get_slice().is_none());
    }

    /// An implementation that does not override `scorer` inherits the base
    /// behaviour, exactly as a Java subclass of
    /// `BaseQuantizedByteVectorValues` does.
    #[test]
    fn an_implementation_that_does_not_override_scorer_inherits_the_base_error() {
        let values = Plain {
            base: BaseQuantizedByteVectorValues::new(2, 2),
            bytes: vec![1, 2, 3, 4],
        };
        let err = match values.scorer(&[0.5, 0.5]) {
            Err(e) => e,
            Ok(_) => panic!("an implementation that does not override must not score"),
        };
        assert!(
            matches!(err, LuceneError::UnsupportedOperation(_)),
            "expected UnsupportedOperation, got {err:?}"
        );
    }

    /// An implementation that overrides `scorer` answers with its own scorer,
    /// which is what the caller in `Lucene104ScalarQuantizedVectorsReader`
    /// relies on.
    #[test]
    fn an_implementation_that_overrides_scorer_answers_with_it() {
        let values = Scoring;
        let mut scorer = values
            .scorer(&[1.5, 2.5])
            .expect("the override must not error")
            .expect("the override returns a scorer");
        assert_eq!(scorer.score().expect("score"), 4.0);
    }
}
