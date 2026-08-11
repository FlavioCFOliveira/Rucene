//! Codec framework and Lucene 10.5.0 bundled codecs ported from
//! `org.apache.lucene.codecs`.
//!
//! This module contains the abstract `Codec` API plus the concrete
//! implementations required to read and write index files that are
//! byte-compatible with Apache Lucene Core 10.5.0.

pub mod codec_util;
pub mod compound;
pub mod compressing;
pub mod doc_values;
pub mod field_infos;
pub mod knn_vectors;
pub mod live_docs;

/// Bundled Lucene 9.0 sub-formats reused by the default `Lucene104` codec.
pub mod lucene90;

/// Bundled Lucene 9.4 sub-formats reused by the default `Lucene104` codec.
pub mod lucene94;

/// Bundled Lucene 9.9 sub-formats reused by the default `Lucene104` codec.
pub mod lucene99;

/// Bundled Lucene 10.3 sub-formats reused by the default `Lucene104` codec.
pub mod lucene103;

/// Low-level helpers for the `Lucene104` postings format.
pub mod lucene104;

pub mod norms;
pub mod per_field;
pub mod points;
pub mod postings;
pub mod segment_info;
pub mod skip_list;
pub mod state;
pub mod stored_fields;
pub mod stub;
pub mod term_state;
pub mod term_vectors;

pub use codec_util::{
    check_footer, check_header, check_header_no_magic, check_index_header, check_index_header_id,
    check_index_header_suffix, checksum_entire_file, footer_length, header_length,
    index_header_length, read_be_int, read_be_long, retrieve_checksum,
    retrieve_checksum_expected_length, write_be_int, write_be_long, write_footer, write_header,
    write_index_header, CODEC_MAGIC, FOOTER_MAGIC,
};
pub use compressing::{CompressionMode, Compressor, Decompressor, MatchingReaders};

pub use compound::{
    CompoundDirectory, CompoundFileDirectory, CompoundFileEntry, CompoundFormat,
    EmptyCompoundDirectory, EmptyCompoundFormat,
};
pub use doc_values::{
    available_doc_values_formats, doc_values_for_name, BinaryDocValues, DocValuesConsumer,
    DocValuesFormat, DocValuesFormatRegistry, DocValuesProducer, DocValuesSkipper,
    EmptyBinaryDocValues, EmptyDocValuesConsumer, EmptyDocValuesFormat, EmptyDocValuesProducer,
    EmptyDocValuesSkipper, EmptyNumericDocValues, EmptySortedDocValues,
    EmptySortedNumericDocValues, EmptySortedSetDocValues, NumericDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};
pub use field_infos::{EmptyFieldInfosFormat, FieldInfosFormat};
pub use knn_vectors::{
    available_knn_vectors_formats, knn_vectors_for_name, BufferingKnnVectorsWriter,
    ByteVectorValues, EmptyBufferingKnnVectorsWriter, EmptyByteVectorValues,
    EmptyFloatVectorValues, EmptyKnnCollector, EmptyKnnFieldVectorsWriter, EmptyKnnVectorsFormat,
    EmptyKnnVectorsReader, EmptyKnnVectorsWriter, FloatVectorValues, KnnCollector,
    KnnFieldVectorsWriter, KnnSearchStrategy, KnnVectorsFormat, KnnVectorsFormatRegistry,
    KnnVectorsReader, KnnVectorsWriter, SorterDocMap, TopDocs,
};
pub use live_docs::{EmptyLiveDocsFormat, LiveDocsFormat};
pub use lucene103::{
    Lucene103BlockTreeTermsReader, Lucene103BlockTreeTermsWriter, TrieBuilder, TrieReader,
};
pub use lucene104::{
    Lucene104PostingsFormat, Lucene104PostingsReader, Lucene104PostingsWriter, BLOCK_MASK,
    BLOCK_SIZE, BLOCK_SIZE_LOG2, DOC_CODEC, DOC_EXTENSION, LEVEL1_FACTOR, LEVEL1_MASK,
    LEVEL1_NUM_DOCS, MAX_BLOCK_SIZE, META_CODEC, META_EXTENSION, PAY_CODEC, PAY_EXTENSION,
    POS_CODEC, POS_EXTENSION, SKIP_INTERVAL, TERMS_CODEC, VERSION_CURRENT, VERSION_START,
};
pub use lucene90::live_docs::Lucene90LiveDocsFormat;
pub use lucene90::{
    Lucene90CompressingStoredFieldsFormat, Lucene90DocValuesFormat, Lucene90StoredFieldsFormat,
    Mode,
};
pub use lucene94::Lucene94FieldInfosFormat;
pub use lucene99::Lucene99SegmentInfoFormat;
pub use norms::{
    EmptyNormsConsumer, EmptyNormsDocValues, EmptyNormsFormat, EmptyNormsProducer, NormsConsumer,
    NormsFormat, NormsProducer,
};
pub use per_field::{
    PerFieldDocValuesFormat, PerFieldKnnVectorsFormat, PerFieldMergeState, PerFieldPostingsFormat,
};
pub use points::{
    EmptyPointValues, EmptyPointsFormat, EmptyPointsReader, EmptyPointsWriter, PointValues,
    PointsFormat, PointsReader, PointsWriter,
};
pub use postings::{
    available_postings_formats, postings_for_name, FieldsConsumer, FieldsProducer, MergeState,
    PostingsFormat, PostingsFormatRegistry, PostingsReaderBase, PostingsWriterBase,
    PushPostingsWriterBase, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
    POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
pub use segment_info::{EmptySegmentInfoFormat, SegmentInfoFormat};
pub use skip_list::{
    MultiLevelSkipListReader, MultiLevelSkipListWriter, SkipDataReader, SkipDataWriter,
};
pub use state::{SegmentReadState, SegmentWriteState};
pub use stored_fields::{
    EmptyStoredFieldsFormat, EmptyStoredFieldsReader, EmptyStoredFieldsWriter, StoredFieldsFormat,
    StoredFieldsReader, StoredFieldsWriter,
};
pub use term_state::{BlockTermState, CompetitiveImpactAccumulator, Impact, TermStats};
pub use term_vectors::{
    EmptyTermVectorsFormat, EmptyTermVectorsReader, EmptyTermVectorsWriter, TermVectorsFormat,
    TermVectorsReader, TermVectorsWriter,
};

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};

use crate::error::{LuceneError, Result};

// ---------------------------------------------------------------------------
// Codec trait
// ---------------------------------------------------------------------------

/// Name of the default codec used by newly created index writers.
///
/// Matches the default for Apache Lucene Core 10.5.0.
pub const DEFAULT_CODEC_NAME: &str = "Lucene104";

/// Encodes and decodes an inverted index segment.
///
/// A codec is a named collection of format factories. The codec name is
/// written into index segments, so any later reader must be able to look it up
/// via [`CodecRegistry::for_name`].
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.Codec`.
pub trait Codec: Send + Sync + fmt::Debug {
    /// Returns this codec's name.
    fn name(&self) -> &str;

    /// Returns the postings format used by this codec.
    fn postings_format(&self) -> &dyn PostingsFormat;

    /// Returns the doc-values format used by this codec.
    fn doc_values_format(&self) -> &dyn DocValuesFormat;

    /// Returns the stored-fields format used by this codec.
    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat;

    /// Returns the term-vectors format used by this codec.
    fn term_vectors_format(&self) -> &dyn TermVectorsFormat;

    /// Returns the field-infos format used by this codec.
    fn field_infos_format(&self) -> &dyn FieldInfosFormat;

    /// Returns the segment-info format used by this codec.
    fn segment_info_format(&self) -> &dyn SegmentInfoFormat;

    /// Returns the norms format used by this codec.
    fn norms_format(&self) -> &dyn NormsFormat;

    /// Returns the live-docs format used by this codec.
    fn live_docs_format(&self) -> &dyn LiveDocsFormat;

    /// Returns the compound-file format used by this codec.
    fn compound_format(&self) -> &dyn CompoundFormat;

    /// Returns the points format used by this codec.
    fn points_format(&self) -> &dyn PointsFormat;

    /// Returns the KNN-vectors format used by this codec.
    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat;
}

impl fmt::Display for dyn Codec + '_ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ---------------------------------------------------------------------------
// Codec registry
// ---------------------------------------------------------------------------

/// A registry mapping codec short names to [`Codec`] implementations.
///
/// The registry intentionally does not use reflection or SPI loading. Codecs are
/// registered explicitly with [`CodecRegistry::register`] and looked up by
/// name with [`CodecRegistry::for_name`].
///
/// Multiple independent registries can coexist; there is also a global
/// registry accessible via the module-level [`for_name`] and
/// [`available_codecs`] helpers.
///
/// Lucene Core equivalent: the static registry backing
/// `org.apache.lucene.codecs.Codec#forName`.
#[derive(Debug, Default, Clone)]
pub struct CodecRegistry {
    codecs: Arc<RwLock<HashMap<String, Arc<dyn Codec>>>>,
}

impl CodecRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a codec under the given short name.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `name` is empty, contains
    /// characters other than ASCII alphanumerics, or is longer than 127 bytes.
    /// Returns [`LuceneError::IllegalState`] if the name is already registered.
    pub fn register<C>(&self, name: impl Into<String>, codec: C) -> Result<()>
    where
        C: Codec + 'static,
    {
        let name = name.into();
        validate_service_name(&name)?;

        let mut codecs = self.codecs.write().map_err(|_| {
            LuceneError::IllegalState("codec registry lock was poisoned".to_string())
        })?;

        if codecs.contains_key(&name) {
            return Err(LuceneError::IllegalState(format!(
                "codec already registered: {name}"
            )));
        }

        codecs.insert(name, Arc::new(codec));
        Ok(())
    }

    /// Looks up a codec by name.
    ///
    /// Returns `None` if no codec has been registered under the given name.
    pub fn for_name(&self, name: &str) -> Option<Arc<dyn Codec>> {
        self.codecs
            .read()
            .map_err(|_| LuceneError::IllegalState("codec registry lock was poisoned".to_string()))
            .ok()?
            .get(name)
            .cloned()
    }

    /// Returns the names of all registered codecs, sorted alphabetically.
    pub fn available_codecs(&self) -> Vec<String> {
        let Ok(codecs) = self.codecs.read() else {
            return Vec::new();
        };
        let mut names: Vec<String> = codecs.keys().cloned().collect();
        names.sort();
        names
    }

    /// Returns the codec registered under [`DEFAULT_CODEC_NAME`], if any.
    pub fn default_codec(&self) -> Option<Arc<dyn Codec>> {
        self.for_name(DEFAULT_CODEC_NAME)
    }
}

/// Validates that a codec/format SPI name contains only ASCII alphanumerics
/// and is between 1 and 127 bytes long.
pub(crate) fn validate_service_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "codec name must not be empty".to_string(),
        ));
    }
    if name.len() > 127 {
        return Err(LuceneError::IllegalArgument(format!(
            "codec name too long ({} > 127 bytes): {name}",
            name.len()
        )));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(LuceneError::IllegalArgument(format!(
            "codec name must be ASCII alphanumeric: {name}"
        )));
    }
    Ok(())
}

static GLOBAL_REGISTRY: LazyLock<CodecRegistry> = LazyLock::new(CodecRegistry::new);

/// Looks up a codec by name from the global registry.
///
/// Returns `None` if no codec has been registered under the given name.
pub fn for_name(name: &str) -> Option<Arc<dyn Codec>> {
    GLOBAL_REGISTRY.for_name(name)
}

/// Returns the names of all codecs registered in the global registry.
pub fn available_codecs() -> Vec<String> {
    GLOBAL_REGISTRY.available_codecs()
}

/// Returns the codec registered as [`DEFAULT_CODEC_NAME`] in the global
/// registry, if any.
pub fn default_codec() -> Option<Arc<dyn Codec>> {
    GLOBAL_REGISTRY.default_codec()
}

// ---------------------------------------------------------------------------
// FilterCodec
// ---------------------------------------------------------------------------

/// A codec that forwards all its method calls to another codec.
///
/// Use this to reuse the functionality of an existing codec while replacing one
/// or more sub-formats via the `with_*` builder methods.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.FilterCodec`.
#[derive(Debug)]
pub struct FilterCodec {
    name: String,
    delegate: Arc<dyn Codec>,
    postings_format: Option<Box<dyn PostingsFormat>>,
    doc_values_format: Option<Box<dyn DocValuesFormat>>,
    stored_fields_format: Option<Box<dyn StoredFieldsFormat>>,
    term_vectors_format: Option<Box<dyn TermVectorsFormat>>,
    field_infos_format: Option<Box<dyn FieldInfosFormat>>,
    segment_info_format: Option<Box<dyn SegmentInfoFormat>>,
    norms_format: Option<Box<dyn NormsFormat>>,
    live_docs_format: Option<Box<dyn LiveDocsFormat>>,
    compound_format: Option<Box<dyn CompoundFormat>>,
    points_format: Option<Box<dyn PointsFormat>>,
    knn_vectors_format: Option<Box<dyn KnnVectorsFormat>>,
}

impl FilterCodec {
    /// Creates a new `FilterCodec` that delegates every method to `delegate`
    /// until a sub-format override is installed with a `with_*` method.
    pub fn new(name: impl Into<String>, delegate: Arc<dyn Codec>) -> Self {
        Self {
            name: name.into(),
            delegate,
            postings_format: None,
            doc_values_format: None,
            stored_fields_format: None,
            term_vectors_format: None,
            field_infos_format: None,
            segment_info_format: None,
            norms_format: None,
            live_docs_format: None,
            compound_format: None,
            points_format: None,
            knn_vectors_format: None,
        }
    }

    /// Returns the delegate codec.
    pub fn delegate(&self) -> &dyn Codec {
        &*self.delegate
    }

    /// Installs a custom postings format override.
    pub fn with_postings_format(mut self, fmt: impl PostingsFormat + 'static) -> Self {
        self.postings_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom doc-values format override.
    pub fn with_doc_values_format(mut self, fmt: impl DocValuesFormat + 'static) -> Self {
        self.doc_values_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom stored-fields format override.
    pub fn with_stored_fields_format(mut self, fmt: impl StoredFieldsFormat + 'static) -> Self {
        self.stored_fields_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom term-vectors format override.
    pub fn with_term_vectors_format(mut self, fmt: impl TermVectorsFormat + 'static) -> Self {
        self.term_vectors_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom field-infos format override.
    pub fn with_field_infos_format(mut self, fmt: impl FieldInfosFormat + 'static) -> Self {
        self.field_infos_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom segment-info format override.
    pub fn with_segment_info_format(mut self, fmt: impl SegmentInfoFormat + 'static) -> Self {
        self.segment_info_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom norms format override.
    pub fn with_norms_format(mut self, fmt: impl NormsFormat + 'static) -> Self {
        self.norms_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom live-docs format override.
    pub fn with_live_docs_format(mut self, fmt: impl LiveDocsFormat + 'static) -> Self {
        self.live_docs_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom compound-file format override.
    pub fn with_compound_format(mut self, fmt: impl CompoundFormat + 'static) -> Self {
        self.compound_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom points format override.
    pub fn with_points_format(mut self, fmt: impl PointsFormat + 'static) -> Self {
        self.points_format = Some(Box::new(fmt));
        self
    }

    /// Installs a custom KNN-vectors format override.
    pub fn with_knn_vectors_format(mut self, fmt: impl KnnVectorsFormat + 'static) -> Self {
        self.knn_vectors_format = Some(Box::new(fmt));
        self
    }
}

impl Codec for FilterCodec {
    fn name(&self) -> &str {
        &self.name
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        self.postings_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.postings_format())
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.doc_values_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.doc_values_format())
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        self.stored_fields_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.stored_fields_format())
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        self.term_vectors_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.term_vectors_format())
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.field_infos_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.field_infos_format())
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.segment_info_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.segment_info_format())
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        self.norms_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.norms_format())
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        self.live_docs_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.live_docs_format())
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        self.compound_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.compound_format())
    }

    fn points_format(&self) -> &dyn PointsFormat {
        self.points_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.points_format())
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        self.knn_vectors_format
            .as_deref()
            .unwrap_or_else(|| self.delegate.knn_vectors_format())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::stub::*;
    use super::*;

    /// A no-op fields consumer for the dummy postings format.
    #[derive(Debug, Default, Clone)]
    struct DummyFieldsConsumer;

    impl FieldsConsumer for DummyFieldsConsumer {
        fn write(
            &mut self,
            _fields: &dyn crate::codecs::postings::Fields,
            _norms: &dyn crate::codecs::postings::NormsProducer,
        ) -> Result<()> {
            Ok(())
        }

        fn merge(
            &mut self,
            _merge_state: &MergeState,
            _norms: &dyn crate::codecs::postings::NormsProducer,
        ) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// A no-op fields producer for the dummy postings format.
    #[derive(Debug, Default, Clone)]
    struct DummyFieldsProducer;

    impl crate::codecs::postings::Fields for DummyFieldsProducer {
        fn size(&self) -> i32 {
            0
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn crate::codecs::postings::Terms>>> {
            Ok(None)
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty::<String>())
        }
    }

    impl FieldsProducer for DummyFieldsProducer {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// A trivial named placeholder format used by the dummy codec in tests.
    #[derive(Debug)]
    pub(crate) struct DummyFormat(&'static str);

    impl PostingsFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn fields_consumer<'a>(
            &self,
            _state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn FieldsConsumer + 'a>> {
            Ok(Box::new(DummyFieldsConsumer))
        }

        fn fields_producer<'a>(
            &self,
            _state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn FieldsProducer + 'a>> {
            Ok(Box::new(DummyFieldsProducer))
        }
    }

    impl DocValuesFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn fields_consumer<'a>(
            &self,
            _state: &SegmentWriteState<'a>,
        ) -> crate::error::Result<Box<dyn DocValuesConsumer + 'a>> {
            Ok(Box::new(EmptyDocValuesConsumer))
        }

        fn fields_producer<'a>(
            &self,
            _state: &SegmentReadState<'a>,
        ) -> crate::error::Result<Box<dyn DocValuesProducer + 'a>> {
            Ok(Box::new(EmptyDocValuesProducer))
        }
    }

    impl StoredFieldsFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn fields_reader(
            &self,
            _directory: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
            _field_infos: &FieldInfos,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<Box<dyn StoredFieldsReader>> {
            Ok(Box::new(EmptyStoredFieldsReader))
        }

        fn fields_writer(
            &self,
            _directory: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<Box<dyn StoredFieldsWriter>> {
            Ok(Box::new(EmptyStoredFieldsWriter))
        }
    }

    impl TermVectorsFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn vectors_reader(
            &self,
            _directory: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
            _field_infos: &FieldInfos,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<Box<dyn TermVectorsReader>> {
            Ok(Box::new(EmptyTermVectorsReader))
        }

        fn vectors_writer(
            &self,
            _directory: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<Box<dyn TermVectorsWriter>> {
            Ok(Box::new(EmptyTermVectorsWriter))
        }
    }

    impl FieldInfosFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn read(
            &self,
            _directory: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
            _segment_suffix: &str,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<FieldInfos> {
            Ok(FieldInfos::default())
        }

        fn write(
            &self,
            _directory: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
            _segment_suffix: &str,
            _infos: &FieldInfos,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<()> {
            Ok(())
        }
    }

    impl SegmentInfoFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn read(
            &self,
            _directory: &dyn crate::store::Directory,
            segment_name: &str,
            segment_id: &[u8],
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<SegmentInfo> {
            let mut id = [0u8; crate::util::string_helper::ID_LENGTH];
            let len = segment_id.len().min(id.len());
            id[..len].copy_from_slice(&segment_id[..len]);
            SegmentInfo::new(
                Arc::new(crate::store::FilterDirectory::new(Box::new(
                    crate::store::RamDirectory::default(),
                ))),
                crate::util::Version::LUCENE_10_5_0,
                Some(crate::util::Version::LUCENE_10_5_0),
                segment_name.to_string(),
                0,
                false,
                false,
                Arc::new(DummyCodec::new("Dummy")),
                HashMap::new(),
                id,
                HashMap::new(),
                crate::search::Sort::default(),
            )
        }

        fn write(
            &self,
            _directory: &dyn crate::store::Directory,
            _info: &SegmentInfo,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<()> {
            Ok(())
        }
    }

    impl NormsFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn norms_consumer(
            &self,
            _state: &SegmentWriteState,
        ) -> crate::error::Result<Box<dyn NormsConsumer>> {
            Ok(Box::new(EmptyNormsConsumer))
        }

        fn norms_producer(
            &self,
            _state: &SegmentReadState,
        ) -> crate::error::Result<Box<dyn NormsProducer>> {
            Ok(Box::new(EmptyNormsProducer))
        }
    }

    impl LiveDocsFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn read_live_docs(
            &self,
            _dir: &dyn crate::store::Directory,
            _info: &SegmentCommitInfo,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<Box<dyn crate::util::Bits>> {
            Ok(Box::new(crate::util::MatchAllBits::new(0)))
        }

        fn write_live_docs(
            &self,
            _bits: &dyn crate::util::Bits,
            _dir: &dyn crate::store::Directory,
            _info: &SegmentCommitInfo,
            _new_del_count: i32,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<()> {
            Ok(())
        }

        fn files(
            &self,
            _info: &SegmentCommitInfo,
            _files: &mut Vec<String>,
        ) -> crate::error::Result<()> {
            Ok(())
        }
    }

    impl CompoundFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn get_compound_reader(
            &self,
            _dir: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
        ) -> crate::error::Result<Box<dyn CompoundDirectory>> {
            Ok(Box::new(EmptyCompoundDirectory))
        }

        fn write(
            &self,
            _dir: &dyn crate::store::Directory,
            _segment_info: &SegmentInfo,
            _context: &dyn crate::store::IOContext,
        ) -> crate::error::Result<()> {
            Ok(())
        }
    }

    impl PointsFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn fields_writer(
            &self,
            _state: &SegmentWriteState,
        ) -> crate::error::Result<Box<dyn PointsWriter>> {
            Ok(Box::new(EmptyPointsWriter))
        }

        fn fields_reader(
            &self,
            _state: &SegmentReadState,
        ) -> crate::error::Result<Box<dyn PointsReader>> {
            Ok(Box::new(EmptyPointsReader))
        }
    }

    impl KnnVectorsFormat for DummyFormat {
        fn name(&self) -> &str {
            self.0
        }

        fn fields_writer<'a>(
            &self,
            _state: &SegmentWriteState<'a>,
        ) -> crate::error::Result<Box<dyn KnnVectorsWriter + 'a>> {
            Ok(Box::new(EmptyKnnVectorsWriter))
        }

        fn fields_reader<'a>(
            &self,
            _state: &SegmentReadState<'a>,
        ) -> crate::error::Result<Box<dyn KnnVectorsReader + 'a>> {
            Ok(Box::new(EmptyKnnVectorsReader))
        }

        fn get_max_dimensions(&self, _field_name: &str) -> i32 {
            1024
        }
    }

    /// A minimal codec implementation used only for unit testing the SPI and
    /// delegation machinery.
    #[derive(Debug)]
    pub(crate) struct DummyCodec {
        name: String,
        postings: DummyFormat,
        doc_values: DummyFormat,
        stored_fields: DummyFormat,
        term_vectors: DummyFormat,
        field_infos: DummyFormat,
        segment_info: DummyFormat,
        norms: DummyFormat,
        live_docs: DummyFormat,
        compound: DummyFormat,
        points: DummyFormat,
        knn_vectors: DummyFormat,
    }

    impl DummyCodec {
        pub(crate) fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                postings: DummyFormat("dummy-postings"),
                doc_values: DummyFormat("dummy-doc-values"),
                stored_fields: DummyFormat("dummy-stored-fields"),
                term_vectors: DummyFormat("dummy-term-vectors"),
                field_infos: DummyFormat("dummy-field-infos"),
                segment_info: DummyFormat("dummy-segment-info"),
                norms: DummyFormat("dummy-norms"),
                live_docs: DummyFormat("dummy-live-docs"),
                compound: DummyFormat("dummy-compound"),
                points: DummyFormat("dummy-points"),
                knn_vectors: DummyFormat("dummy-knn-vectors"),
            }
        }
    }

    impl Codec for DummyCodec {
        fn name(&self) -> &str {
            &self.name
        }

        fn postings_format(&self) -> &dyn PostingsFormat {
            &self.postings
        }

        fn doc_values_format(&self) -> &dyn DocValuesFormat {
            &self.doc_values
        }

        fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
            &self.stored_fields
        }

        fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
            &self.term_vectors
        }

        fn field_infos_format(&self) -> &dyn FieldInfosFormat {
            &self.field_infos
        }

        fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
            &self.segment_info
        }

        fn norms_format(&self) -> &dyn NormsFormat {
            &self.norms
        }

        fn live_docs_format(&self) -> &dyn LiveDocsFormat {
            &self.live_docs
        }

        fn compound_format(&self) -> &dyn CompoundFormat {
            &self.compound
        }

        fn points_format(&self) -> &dyn PointsFormat {
            &self.points
        }

        fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
            &self.knn_vectors
        }
    }

    /// Returns a minimal `SegmentInfo` for use in cross-module codec tests.
    pub(crate) fn test_segment_info(name: &str, max_doc: i32) -> SegmentInfo {
        SegmentInfo::new(
            std::sync::Arc::new(crate::store::RamDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            std::sync::Arc::new(DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            [0u8; crate::util::string_helper::ID_LENGTH],
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .expect("test segment info should be valid")
    }

    /// Returns a minimal `SegmentCommitInfo` for use in cross-module codec tests.
    pub(crate) fn test_segment_commit_info(
        name: &str,
        max_doc: i32,
    ) -> crate::index::SegmentCommitInfo {
        crate::index::SegmentCommitInfo::new(
            test_segment_info(name, max_doc),
            0,
            0,
            -1,
            -1,
            -1,
            [0u8; crate::util::string_helper::ID_LENGTH],
        )
        .expect("test segment commit info should be valid")
    }

    #[test]
    fn registry_register_and_lookup() {
        let registry = CodecRegistry::new();
        registry
            .register("Dummy", DummyCodec::new("Dummy"))
            .unwrap();

        let looked_up = registry.for_name("Dummy").expect("codec should be present");
        assert_eq!(looked_up.name(), "Dummy");
        assert_eq!(
            format!("{looked_up}"),
            "Dummy",
            "Display should mirror the codec name"
        );

        let names = registry.available_codecs();
        assert_eq!(names, vec!["Dummy".to_string()]);
    }

    #[test]
    fn registry_default_codec_name_is_lucene104() {
        let registry = CodecRegistry::new();
        assert!(registry.default_codec().is_none());
        registry
            .register(DEFAULT_CODEC_NAME, DummyCodec::new("Lucene104"))
            .unwrap();
        assert_eq!(registry.default_codec().unwrap().name(), DEFAULT_CODEC_NAME);
    }

    #[test]
    fn registry_rejects_duplicate_registration() {
        let registry = CodecRegistry::new();
        registry
            .register("Dummy", DummyCodec::new("Dummy"))
            .unwrap();
        let err = registry
            .register("Dummy", DummyCodec::new("Dummy2"))
            .expect_err("duplicate registration should fail");
        assert!(matches!(err, LuceneError::IllegalState(_)));
    }

    #[test]
    fn registry_rejects_invalid_name() {
        let registry = CodecRegistry::new();
        let err = registry
            .register("", DummyCodec::new("Empty"))
            .expect_err("empty name should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));

        let err = registry
            .register("Bad Name", DummyCodec::new("Bad"))
            .expect_err("name with space should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn filter_codec_delegates_everything() {
        let base = Arc::new(DummyCodec::new("Base"));
        let filtered = FilterCodec::new("Filtered", base);

        assert_eq!(filtered.name(), "Filtered");
        assert_eq!(filtered.postings_format().name(), "dummy-postings");
        assert_eq!(filtered.doc_values_format().name(), "dummy-doc-values");
        assert_eq!(
            filtered.stored_fields_format().name(),
            "dummy-stored-fields"
        );
        assert_eq!(filtered.term_vectors_format().name(), "dummy-term-vectors");
        assert_eq!(filtered.field_infos_format().name(), "dummy-field-infos");
        assert_eq!(filtered.segment_info_format().name(), "dummy-segment-info");
        assert_eq!(filtered.norms_format().name(), "dummy-norms");
        assert_eq!(filtered.live_docs_format().name(), "dummy-live-docs");
        assert_eq!(filtered.compound_format().name(), "dummy-compound");
        assert_eq!(filtered.points_format().name(), "dummy-points");
        assert_eq!(filtered.knn_vectors_format().name(), "dummy-knn-vectors");
        assert_eq!(filtered.delegate().name(), "Base");
    }

    #[test]
    fn filter_codec_overrides_one_format() {
        let base = Arc::new(DummyCodec::new("Base"));
        let filtered =
            FilterCodec::new("Filtered", base).with_knn_vectors_format(DummyFormat("custom-knn"));

        assert_eq!(filtered.knn_vectors_format().name(), "custom-knn");
        // All other formats still come from the delegate.
        assert_eq!(filtered.postings_format().name(), "dummy-postings");
        assert_eq!(filtered.doc_values_format().name(), "dummy-doc-values");
        assert_eq!(
            filtered.stored_fields_format().name(),
            "dummy-stored-fields"
        );
        assert_eq!(filtered.term_vectors_format().name(), "dummy-term-vectors");
        assert_eq!(filtered.field_infos_format().name(), "dummy-field-infos");
        assert_eq!(filtered.segment_info_format().name(), "dummy-segment-info");
        assert_eq!(filtered.norms_format().name(), "dummy-norms");
        assert_eq!(filtered.live_docs_format().name(), "dummy-live-docs");
        assert_eq!(filtered.compound_format().name(), "dummy-compound");
        assert_eq!(filtered.points_format().name(), "dummy-points");
    }
}
