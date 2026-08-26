//! Indexing engine ported from `org.apache.lucene.index`.
//!
//! This module exposes the core value-accessor abstractions used by codec
//! producers and index consumers: field metadata, segment information,
//! doc-values, vector values, point values, and the reader hierarchy
//! (`IndexReader`, `LeafReader`, and their contexts).
//!
//! It also holds the write path: [`documents_writer`] buffers documents and
//! orchestrates flushes, [`indexing_chain`] inverts each document, and
//! [`freq_prox_terms_writer`] buffers the resulting postings in RAM and streams
//! them through the codec's postings format at flush time.

#![deny(unsafe_code)]

pub mod automaton_terms_enum;
pub mod directory_reader;
pub mod doc_values;
pub mod doc_values_field_updates;
pub mod documents_writer;
pub mod field_infos;
pub mod freq_prox_terms_writer;
pub mod index_file_names;
pub mod index_reader;
pub mod index_writer_config;
pub mod indexing_chain;
pub mod leaf_reader;
pub mod mapped_multi_fields;
pub mod mapping_multi_postings_enum;
pub mod merge;
pub mod multi_bits;
pub mod multi_doc_values;
pub mod multi_fields;
pub mod multi_leaf_reader;
pub mod multi_reader;
pub mod parallel_reader;
pub mod point_values;
pub mod postings_enum;
pub mod reader_context;
pub mod reader_manager;
pub mod readers_and_updates;
pub mod segment_info;
pub mod segment_infos;
pub mod segment_reader;
pub mod terms;
pub mod terms_enum_index;
pub mod vector_values;

pub use automaton_terms_enum::AutomatonTermsEnum;
pub use doc_values::{
    BinaryDocValues, DocValues, DocValuesIterator, DocValuesSkipper, EmptyBinaryDocValues,
    EmptyDocValuesProducer, EmptyDocValuesSkipper, EmptyNumericDocValues, EmptySortedDocValues,
    EmptySortedNumericDocValues, EmptySortedSetDocValues, FilterBinaryDocValues,
    FilterNumericDocValues, FilterSortedDocValues, FilterSortedNumericDocValues,
    FilterSortedSetDocValues, NumericDocValues, OrdinalMap, SingletonSortedNumericDocValues,
    SingletonSortedSetDocValues, SortedDocValues, SortedDocValuesTermsEnum, SortedNumericDocValues,
    SortedSetDocValues, SortedSetDocValuesTermsEnum,
};
pub use field_infos::{FieldInfo, FieldInfos, FieldInfosBuilder, FieldNumbers};
pub use index_file_names::{
    file_name_from_generation, get_extension, is_codec_file, matches_extension, parse_generation,
    parse_segment_name, segment_file_name, standard_extensions, strip_extension,
    strip_segment_name, COMPOUND_FILE_ENTRIES_EXTENSION, COMPOUND_FILE_EXTENSION,
    DOC_VALUES_EXTENSION, DOC_VALUES_META_EXTENSION, FIELD_INFO_EXTENSION, KNN_VECTORS_EXTENSION,
    KNN_VECTORS_FORMAT_META_EXTENSION, KNN_VECTORS_INDEX_EXTENSION, KNN_VECTORS_META_EXTENSION,
    LIVE_DOCS_EXTENSION, NORMS_EXTENSION, NORMS_META_EXTENSION, OLD_LIVE_DOCS_EXTENSION,
    PAYLOADS_EXTENSION, PENDING_SEGMENTS, POINTS_EXTENSION, POINTS_INDEX_EXTENSION,
    POINTS_META_EXTENSION, POSITIONS_EXTENSION, POSTINGS_EXTENSION, SEGMENTS,
    SEGMENT_INFO_EXTENSION, STORED_FIELDS_EXTENSION, STORED_FIELDS_INDEX_EXTENSION,
    STORED_FIELDS_META_EXTENSION, TERMS_EXTENSION, TERMS_INDEX_EXTENSION, TERMS_META_EXTENSION,
    TERMS_POSTINGS_EXTENSION, VECTORS_FIELDS_EXTENSION, VECTORS_INDEX_EXTENSION,
    VECTORS_META_EXTENSION,
};
pub use index_reader::{
    CacheHelper, CacheKey, ClosedListener, CompositeReader, IndexReader, IndexReaderCore,
    StoredFields,
};
pub use leaf_reader::{EmptyTermVectors, LeafMetaData, LeafReader, TermVectors};
pub use merge::{
    deletion_doc_map, identity_doc_map, DocIDMerger, DocIDMergerSub, DocMap, MergeState,
};
// The per-reader aggregation helpers (`size`, `doc_count`, `min_packed_value`,
// `max_packed_value`) and the traversal free functions (`intersect`,
// `estimate_point_count`, ...) are deliberately not re-exported here: their
// names are far too generic at the `index` root. They are reached through
// `crate::index::point_values::`, which is the exact analogue of Java's
// `PointValues.size(reader, field)` static call.
pub use point_values::{
    EmptyPointValues, InMemoryPointTree, InMemoryPointValues, IntersectVisitor, PointTree,
    PointValues, Relation, MAX_DIMENSIONS, MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
pub use postings_enum::{
    feature_requested, DocAndFloatFeatureBuffer, EmptyPostingsEnum, FreqAndNormBuffer, Impacts,
    ImpactsEnum, ImpactsSource, PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
pub use reader_context::{CompositeReaderContext, IndexReaderContext, LeafReaderContext};
pub use segment_info::{
    SegmentCommitInfo, SegmentInfo, SegmentOrder, SegmentReadState, SegmentWriteState,
    UNSET_MAX_DOC,
};
pub use segment_infos::SegmentInfos;

pub use freq_prox_terms_writer::{
    ByteSlicePool, ByteSliceReader, FreqProxFields, FreqProxPosting, FreqProxTermsWriter,
    FreqProxTermsWriterPerField, InvertedToken, TermSlot, TermsHash, TermsHashPerField,
    FIRST_LEVEL_SIZE, LEVEL_SIZE_ARRAY, NEXT_LEVEL_ARRAY,
};
pub use indexing_chain::{
    DefaultIndexingChain, EmptyNormsProducer, FieldInvertState, MAX_POSITION,
};

pub use index_writer_config::{
    ConcurrentMergeScheduler, DefaultSimilarity, IndexDeletionPolicy, IndexWriterConfig,
    IndexWriterEventListener, KeepOnlyLastCommitDeletionPolicy, LeafComparator,
    LiveIndexWriterConfig, MergePolicy, MergeScheduler, MergeSpecification, MergedSegmentWarmer,
    NoOpIndexWriterEventListener, OpenMode, Similarity, TieredMergePolicy,
};

// In-memory indexing pipeline exports.
pub use documents_writer::{
    BufferedUpdates, DeleteNode, DeleteSlice, DocumentsWriter, DocumentsWriterDeleteQueue,
    DocumentsWriterFlushControl, DocumentsWriterFlushQueue, DocumentsWriterPerThread,
    DocumentsWriterPerThreadPool, DocumentsWriterShared, DocumentsWriterStallControl, DwptGuard,
    FlushByRamOrCountsPolicy, FlushControlHandle, FlushNotifications, FlushPolicy, FlushTicket,
    FlushedSegment, FrozenBufferedUpdates, IndexingChain, IndexingChainFactory,
    IndexingChainFlushState, LockAllGuard, NoOpFlushNotifications, Query, SegmentNameSupplier,
    SharedIndexingScratch, TermDelete, BYTES_PER_DEL_QUERY, BYTES_SCRATCH_SIZE, INTS_SCRATCH_SIZE,
    MAX_DOCS, SOURCE_FLUSH,
};

// Directory/segment reader exports.
pub use directory_reader::{
    index_exists, list_commits, open as open_directory_reader, open_if_changed,
    open_if_changed_with_commit, open_if_changed_with_writer, open_with_commit, DirectoryReader,
    IndexCommit, IndexWriter as DirectoryReaderIndexWriter, StandardDirectoryReader,
};
pub use mapped_multi_fields::MappedMultiFields;
pub use mapping_multi_postings_enum::MappingMultiPostingsEnum;
pub use multi_bits::{BitsSlice, MultiBits};
pub use multi_doc_values::{MultiDocValues, MultiSortedDocValues, MultiSortedSetDocValues};
pub use multi_fields::{
    EnumWithSlice, MultiFields, MultiPostingsEnum, MultiTerms, MultiTermsEnum, SlowImpactsEnum,
};
pub use multi_reader::{
    get_top_level_context, index_of, sub_index, sub_index_from_leaves, MultiReader, ReaderSlice,
};
pub use parallel_reader::{ParallelCompositeReader, ParallelLeafReader};
pub use reader_manager::ReaderManager;
pub use segment_reader::SegmentReader;

pub use terms::{
    AcceptStatus, EmptyFields, EmptyTerms, EmptyTermsEnum, Fields, FilteredTermsEnum,
    FilteredTermsEnumFilter, OrdTermState, PrefixCodedTerms, PrefixCodedTermsBuilder,
    PrefixCodedTermsIterator, SeekStatus, SingleTermsEnum, Term, TermState, Terms, TermsEnum,
};
pub use terms_enum_index::{
    prefix8_to_comparable_unsigned_long, TermsEnumIndex, TermsEnumIndexState,
};
pub use vector_values::{
    accept_ords, check_byte_field, check_float_field, from_bytes, from_floats, AcceptOrds,
    ByteVectorValues, DenseDocIndexIterator, DocIndexIterator, EmptyByteVectorValues,
    EmptyFloatVectorValues, EmptyKnnVectorValues, FloatVectorValues, FromDisiDocIndexIterator,
    KnnVectorValues, ListByteVectorValues, ListFloatVectorValues, SparseDocIndexIterator,
};

pub use doc_values_field_updates::DocValuesFieldUpdates;
pub use readers_and_updates::{PendingDeletes, PendingSoftDeletes, ReaderPool, ReadersAndUpdates};

use std::collections::HashMap;

use crate::{
    analysis::{Analyzer, TokenStream},
    document::{InvertableType, NumericValue, StoredValue},
    error::Result,
    store::{ByteArrayDataInput, DataInput},
    util::{vector_util, BytesRef},
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
    ///
    /// Equivalent to `org.apache.lucene.index.IndexOptions.subsumes(IndexOptions)`.
    /// `DOCS_AND_CUSTOM_FREQS` is encoded with the same bits as
    /// `DOCS_AND_FREQS`, so for ordering purposes it is treated as the latter.
    pub fn subsumes(&self, other: IndexOptions) -> bool {
        if *self == IndexOptions::DOCS_AND_CUSTOM_FREQS {
            return IndexOptions::DOCS_AND_FREQS.subsumes(other);
        }
        if other == IndexOptions::DOCS_AND_CUSTOM_FREQS {
            return self.subsumes(IndexOptions::DOCS_AND_FREQS);
        }
        (*self as i32) >= (other as i32)
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

impl VectorSimilarityFunction {
    /// Returns the similarity score between two float vectors.
    ///
    /// Higher scores correspond to closer vectors. Equivalent to
    /// `VectorSimilarityFunction.compare(float[], float[])`, which delegates
    /// every arithmetic step to `VectorUtil`; this port does the same through
    /// [`crate::util::vector_util`], so the scores are bit-identical to
    /// Lucene's scalar, non-FMA path.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::LuceneError::IllegalArgument`] when the two
    /// vectors have different dimensions, matching the
    /// `IllegalArgumentException` that `VectorUtil` throws.
    pub fn compare_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        match self {
            Self::EUCLIDEAN => Ok(vector_util::normalize_distance_to_unit_interval(
                vector_util::square_distance_f32(a, b)?,
            )),
            Self::DOT_PRODUCT => Ok(vector_util::normalize_to_unit_interval(
                vector_util::dot_product_f32(a, b)?,
            )),
            Self::COSINE => Ok(vector_util::normalize_to_unit_interval(
                vector_util::cosine_f32(a, b)?,
            )),
            Self::MAXIMUM_INNER_PRODUCT => Ok(vector_util::scale_max_inner_product_score(
                vector_util::dot_product_f32(a, b)?,
            )),
        }
    }

    /// Returns the similarity score between two byte vectors.
    ///
    /// The bytes are interpreted as **signed** values in `[-128, 127]`, as
    /// Lucene does. Equivalent to
    /// `VectorSimilarityFunction.compare(byte[], byte[])`.
    ///
    /// Note the deliberate asymmetry with [`compare_f32`](Self::compare_f32):
    /// the float `DOT_PRODUCT` and `COSINE` paths clamp negative scores to
    /// zero, the byte paths do not. That is Lucene's behaviour, not an
    /// oversight.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::LuceneError::IllegalArgument`] when the two
    /// vectors have different dimensions.
    pub fn compare_bytes(&self, a: &[u8], b: &[u8]) -> Result<f32> {
        match self {
            Self::EUCLIDEAN => {
                let dist = vector_util::square_distance_bytes(a, b)?;
                Ok(1.0 / (1.0 + dist as f32))
            }
            Self::DOT_PRODUCT => vector_util::dot_product_score(a, b),
            Self::COSINE => {
                let score = vector_util::cosine_bytes(a, b)?;
                Ok((1.0 + score) / 2.0)
            }
            Self::MAXIMUM_INNER_PRODUCT => Ok(vector_util::scale_max_inner_product_score(
                vector_util::dot_product_bytes(a, b)? as f32,
            )),
        }
    }
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
    fn index_options_subsumes_matches_java() {
        // DOCS_AND_CUSTOM_FREQS is encoded like DOCS_AND_FREQS, so it is treated
        // as that option for subsumes comparisons.
        assert!(!IndexOptions::DOCS_AND_FREQS.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS));
        assert!(!IndexOptions::DOCS_AND_CUSTOM_FREQS
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS));
        assert!(IndexOptions::DOCS_AND_CUSTOM_FREQS.subsumes(IndexOptions::DOCS));
        assert!(IndexOptions::DOCS_AND_CUSTOM_FREQS.subsumes(IndexOptions::DOCS_AND_FREQS));
        assert!(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
            .subsumes(IndexOptions::DOCS_AND_CUSTOM_FREQS));
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
    fn vector_encoding_ordinals_match_java() {
        assert_eq!(VectorEncoding::BYTE as usize, 0);
        assert_eq!(VectorEncoding::FLOAT32 as usize, 1);
    }

    #[test]
    fn vector_similarity_function_ordinals_match_java() {
        assert_eq!(VectorSimilarityFunction::EUCLIDEAN as usize, 0);
        assert_eq!(VectorSimilarityFunction::DOT_PRODUCT as usize, 1);
        assert_eq!(VectorSimilarityFunction::COSINE as usize, 2);
        assert_eq!(VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT as usize, 3);
    }

    #[test]
    fn vector_similarity_byte_comparisons_use_signed_arithmetic() {
        // Java VectorUtil treats byte vectors as signed for every similarity function.
        let a = [0xFFu8, 0x00]; // -1, 0 as signed bytes
        let b = [0x01u8, 0x00]; //  1, 0 as signed bytes

        // Dot product of [-1, 0] and [1, 0] is -1, scaled by 2^15 * 2.
        let dot = VectorSimilarityFunction::DOT_PRODUCT
            .compare_bytes(&a, &b)
            .unwrap();
        let expected_dot = 0.5f32 - 1.0 / ((a.len() * (1 << 15)) as f32);
        assert!(
            (dot - expected_dot).abs() < 1e-6,
            "dot product score should be ~{expected_dot}"
        );

        // Maximum inner product of [-1, 0] and [1, 0] is -1 -> scaled to 1/2.
        let mip = VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT
            .compare_bytes(&a, &b)
            .unwrap();
        assert!(
            (mip - 0.5f32).abs() < f32::EPSILON,
            "MIP score should be 0.5"
        );

        // Euclidean distance squared of [-1, 0] and [1, 0] is 4.
        let euclid = VectorSimilarityFunction::EUCLIDEAN
            .compare_bytes(&a, &b)
            .unwrap();
        assert!(
            (euclid - 1.0 / 5.0).abs() < 1e-6,
            "euclidean score should be 1/5"
        );
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
