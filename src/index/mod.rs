//! Indexing engine ported from `org.apache.lucene.index`.
//!
//! This module exposes the core value-accessor abstractions used by codec
//! producers and index consumers: field metadata, segment information,
//! doc-values, vector values, and point values.

#![deny(unsafe_code)]

pub mod doc_values;
pub mod field_infos;
pub mod merge;
pub mod point_values;
pub mod postings_enum;
pub mod segment_info;
pub mod terms;
pub mod vector_values;

pub use doc_values::{
    BinaryDocValues, DocValues, DocValuesIterator, DocValuesSkipper, EmptyBinaryDocValues,
    EmptyDocValuesSkipper, EmptyNumericDocValues, EmptySortedDocValues,
    EmptySortedNumericDocValues, EmptySortedSetDocValues, NumericDocValues,
    SingletonSortedNumericDocValues, SingletonSortedSetDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};
pub use field_infos::{FieldInfo, FieldInfos};
pub use merge::{
    deletion_doc_map, identity_doc_map, DocIDMerger, DocIDMergerSub, DocMap, MergeState,
};
pub use point_values::{
    EmptyPointValues, IntersectVisitor, PointValues, Relation, MAX_DIMENSIONS,
    MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
pub use postings_enum::{
    feature_requested, DocAndFloatFeatureBuffer, EmptyPostingsEnum, FreqAndNormBuffer, Impacts,
    ImpactsEnum, ImpactsSource, PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
pub use segment_info::{SegmentCommitInfo, SegmentInfo};
pub use terms::{
    EmptyFields, EmptyTerms, EmptyTermsEnum, Fields, SeekStatus, TermState, Terms, TermsEnum,
};
pub use vector_values::{
    ByteVectorValues, DenseDocIndexIterator, DocIndexIterator, EmptyByteVectorValues,
    EmptyFloatVectorValues, EmptyKnnVectorValues, FloatVectorValues, FromDisiDocIndexIterator,
    KnnVectorValues,
};

use std::collections::HashMap;

use crate::{
    analysis::{Analyzer, TokenStream},
    document::{InvertableType, NumericValue, StoredValue},
    store::{ByteArrayDataInput, DataInput},
    util::BytesRef,
};

/// Controls how much information is stored in the postings lists.
///
/// Equivalent to `org.apache.lucene.index.IndexOptions`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum IndexOptions {
    /// Not indexed.
    #[default]
    NONE,
    /// Only documents are indexed.
    DOCS,
    /// Documents and term frequencies are indexed.
    DOCS_AND_FREQS,
    /// Documents, frequencies and positions are indexed.
    DOCS_AND_FREQS_AND_POSITIONS,
    /// Documents, frequencies, positions and offsets are indexed.
    DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
    /// Like `DOCS_AND_FREQS` but the custom frequencies are treated as scores.
    DOCS_AND_CUSTOM_FREQS,
}

impl IndexOptions {
    /// Returns `true` if this option records at least as much information as
    /// `other`.
    pub fn subsumes(&self, other: IndexOptions) -> bool {
        let ord = *self as i32;
        let other_ord = other as i32;
        ord >= other_ord
    }
}

/// DocValues types.
///
/// Equivalent to `org.apache.lucene.index.DocValuesType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum DocValuesType {
    /// No doc values for this field.
    NONE,
    /// A per-document Number.
    NUMERIC,
    /// A per-document byte array.
    BINARY,
    /// A pre-sorted byte array.
    SORTED,
    /// A pre-sorted Number array.
    SORTED_NUMERIC,
    /// A pre-sorted Set of byte arrays.
    SORTED_SET,
}

/// Options for skip indexes on doc values.
///
/// Equivalent to `org.apache.lucene.index.DocValuesSkipIndexType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum DocValuesSkipIndexType {
    /// No skip index should be created.
    NONE,
    /// Record range of values per range of doc IDs.
    RANGE,
}

impl DocValuesSkipIndexType {
    /// Returns `true` if this skip-index type is compatible with the given
    /// DocValues type.
    ///
    /// Equivalent to `DocValuesSkipIndexType.isCompatibleWith(DocValuesType)`.
    pub fn is_compatible_with(self, doc_values_type: DocValuesType) -> bool {
        match self {
            Self::NONE => true,
            Self::RANGE => matches!(
                doc_values_type,
                DocValuesType::NUMERIC
                    | DocValuesType::SORTED_NUMERIC
                    | DocValuesType::SORTED
                    | DocValuesType::SORTED_SET
            ),
        }
    }
}

/// The numeric datatype of the vector values.
///
/// Equivalent to `org.apache.lucene.index.VectorEncoding`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum VectorEncoding {
    /// Encodes a vector using 8 bits of precision per sample.
    BYTE,
    /// Encodes a vector using 32 bits of precision per sample in IEEE format.
    FLOAT32,
}

impl VectorEncoding {
    /// The number of bytes required to encode a scalar in this format.
    pub const fn byte_size(&self) -> i32 {
        match self {
            Self::BYTE => 1,
            Self::FLOAT32 => 4,
        }
    }
}

/// Vector similarity function used to return the top-K most similar vectors.
///
/// Equivalent to `org.apache.lucene.index.VectorSimilarityFunction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum VectorSimilarityFunction {
    /// Euclidean distance.
    EUCLIDEAN,
    /// Dot product.
    DOT_PRODUCT,
    /// Cosine similarity.
    COSINE,
    /// Maximum inner product.
    MAXIMUM_INNER_PRODUCT,
}

/// Describes the properties of a field.
///
/// Equivalent to `org.apache.lucene.index.IndexableFieldType`.
pub trait IndexableFieldType {
    /// True if the field's value should be stored.
    fn stored(&self) -> bool;

    /// True if this field's value should be analyzed by the `Analyzer`.
    fn tokenized(&self) -> bool;

    /// True if this field's indexed form should also be stored into term vectors.
    fn store_term_vectors(&self) -> bool;

    /// True if this field's token character offsets should also be stored into term vectors.
    fn store_term_vector_offsets(&self) -> bool;

    /// True if this field's token positions should also be stored into term vectors.
    fn store_term_vector_positions(&self) -> bool;

    /// True if this field's token payloads should also be stored into term vectors.
    fn store_term_vector_payloads(&self) -> bool;

    /// True if normalization values should be omitted for the field.
    fn omit_norms(&self) -> bool;

    /// Describes what should be recorded into the inverted index.
    fn index_options(&self) -> IndexOptions;

    /// How the field's value will be indexed into docValues.
    fn doc_values_type(&self) -> DocValuesType;

    /// Whether a skip index for doc values should be created on this field.
    fn doc_values_skip_index_type(&self) -> DocValuesSkipIndexType;

    /// Number of point dimensions if positive, meaning the field is indexed as a point.
    fn point_dimension_count(&self) -> i32;

    /// Number of dimensions used for the index key.
    fn point_index_dimension_count(&self) -> i32;

    /// Number of bytes in each dimension's values.
    fn point_num_bytes(&self) -> i32;

    /// Number of dimensions of the field's vector value.
    fn vector_dimension(&self) -> i32;

    /// The encoding of the field's vector value.
    fn vector_encoding(&self) -> VectorEncoding;

    /// The similarity function of the field's vector value.
    fn vector_similarity_function(&self) -> VectorSimilarityFunction;

    /// Attributes for the field type.
    fn attributes(&self) -> &HashMap<String, String>;
}

/// Represents a single field for indexing.
///
/// Equivalent to `org.apache.lucene.index.IndexableField`.
pub trait IndexableField {
    /// Field name.
    fn name(&self) -> &str;

    /// Type describing the properties of this field.
    fn field_type(&self) -> &dyn IndexableFieldType;

    /// Creates the `TokenStream` used for indexing this field.
    fn token_stream(
        &self,
        analyzer: &dyn Analyzer,
        reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream>;

    /// Non-null if this field has a binary value.
    fn binary_value(&self) -> Option<BytesRef>;

    /// Non-null if this field has a string value.
    fn string_value(&self) -> Option<String>;

    /// Non-null if this field has a string value.
    fn char_sequence_value(&self) -> Option<String> {
        self.string_value()
    }

    /// Non-null if this field has a Reader value.
    fn reader_value(&mut self) -> Option<&mut dyn std::io::Read>;

    /// Non-null if this field has a numeric value.
    fn numeric_value(&self) -> Option<NumericValue>;

    /// Stored value.
    fn stored_value(&self) -> Option<StoredValue>;

    /// Describes how this field should be inverted.
    fn invertable_type(&self) -> Option<InvertableType>;
}

/// A fixed size `DataInput` which includes the length of the input.
///
/// Equivalent to `org.apache.lucene.index.StoredFieldDataInput`.
pub struct StoredFieldDataInput<'a> {
    /// The underlying data input.
    input: &'a mut dyn DataInput,
    /// The length of the data input.
    length: i32,
}

impl<'a> StoredFieldDataInput<'a> {
    /// Creates a `StoredFieldDataInput` from a `DataInput` and a length.
    pub fn new(input: &'a mut dyn DataInput, length: i32) -> Self {
        Self { input, length }
    }

    /// Creates a `StoredFieldDataInput` from a `ByteArrayDataInput`.
    pub fn from_byte_array_input(input: &'a mut ByteArrayDataInput) -> Self {
        let length = input.length() as i32;
        Self::new(input, length)
    }

    /// Returns the data input.
    pub fn data_input(&mut self) -> &mut dyn DataInput {
        self.input
    }

    /// Returns the length of the data input.
    pub fn length(&self) -> i32 {
        self.length
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ByteArrayDataInput;

    #[test]
    fn index_options_ordinals_match_java() {
        assert_eq!(IndexOptions::NONE as usize, 0);
        assert_eq!(IndexOptions::DOCS as usize, 1);
        assert_eq!(IndexOptions::DOCS_AND_FREQS as usize, 2);
        assert_eq!(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS as usize, 3);
        assert_eq!(
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS as usize,
            4
        );
        assert_eq!(IndexOptions::DOCS_AND_CUSTOM_FREQS as usize, 5);
    }

    #[test]
    fn doc_values_type_ordinals_match_java() {
        assert_eq!(DocValuesType::NONE as usize, 0);
        assert_eq!(DocValuesType::NUMERIC as usize, 1);
        assert_eq!(DocValuesType::BINARY as usize, 2);
        assert_eq!(DocValuesType::SORTED as usize, 3);
        assert_eq!(DocValuesType::SORTED_NUMERIC as usize, 4);
        assert_eq!(DocValuesType::SORTED_SET as usize, 5);
    }

    #[test]
    fn doc_values_skip_index_type_ordinals_match_java() {
        assert_eq!(DocValuesSkipIndexType::NONE as usize, 0);
        assert_eq!(DocValuesSkipIndexType::RANGE as usize, 1);
    }

    #[test]
    fn vector_encoding_byte_size_match_java() {
        assert_eq!(VectorEncoding::BYTE.byte_size(), 1);
        assert_eq!(VectorEncoding::FLOAT32.byte_size(), 4);
    }

    #[test]
    fn vector_similarity_function_ordinals_match_java() {
        assert_eq!(VectorSimilarityFunction::EUCLIDEAN as usize, 0);
        assert_eq!(VectorSimilarityFunction::DOT_PRODUCT as usize, 1);
        assert_eq!(VectorSimilarityFunction::COSINE as usize, 2);
        assert_eq!(VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT as usize, 3);
    }

    #[test]
    fn point_values_constants_match_java() {
        assert_eq!(MAX_NUM_BYTES, 16);
        assert_eq!(MAX_DIMENSIONS, 16);
        assert_eq!(MAX_INDEX_DIMENSIONS, 8);
    }

    #[test]
    fn stored_field_data_input_wraps_byte_array_input() {
        let mut input = ByteArrayDataInput::new(vec![0x01, 0x02, 0x03]);
        let mut stored = StoredFieldDataInput::from_byte_array_input(&mut input);
        assert_eq!(stored.length(), 3);
        assert_eq!(stored.data_input().read_byte().unwrap(), 0x01);
    }
}
