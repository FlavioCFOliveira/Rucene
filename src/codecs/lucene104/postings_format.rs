//! Lucene 10.4 postings format.
//!
//! This module provides the format wrapper, constants, and skeleton reader/writer
//! for the Lucene 10.4 postings format. It delegates the terms dictionary side
//! to the `Lucene103BlockTreeTermsWriter` / `Lucene103BlockTreeTermsReader` from
//! `crate::codecs::lucene103::blocktree`.
//!
//! Lucene Core equivalent: `org.apache.lucene.codecs.lucene104.Lucene104PostingsFormat`.

#![deny(unsafe_code)]

use crate::codecs::lucene103::blocktree::{
    Lucene103BlockTreeTermsReader, Lucene103BlockTreeTermsWriter, DEFAULT_MAX_BLOCK_SIZE,
    DEFAULT_MIN_BLOCK_SIZE,
};
use crate::codecs::lucene104::postings_reader::Lucene104PostingsReader;
use crate::codecs::lucene104::postings_writer::Lucene104PostingsWriter;
use crate::codecs::postings::{FieldsConsumer, FieldsProducer, PostingsFormat};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::error::{LuceneError, Result};

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

/// Number of documents in a packed postings block.
///
/// Mirrors `Lucene104PostingsFormat.BLOCK_SIZE` and `ForUtil.BLOCK_SIZE`.
pub const BLOCK_SIZE: i32 = 256;

/// Maximum number of documents in a packed postings block.
///
/// This is an alias for [`BLOCK_SIZE`] kept for parity with the naming used in
/// older postings-format descriptions.
pub const MAX_BLOCK_SIZE: i32 = BLOCK_SIZE;

/// `log2(BLOCK_SIZE)`.
pub const BLOCK_SIZE_LOG2: i32 = 8;

/// Mask for extracting the in-block offset of a document.
pub const BLOCK_MASK: i32 = BLOCK_SIZE - 1;

/// Number of level-1 blocks between skip entries.
///
/// Mirrors `Lucene104PostingsFormat.LEVEL1_FACTOR`.
pub const LEVEL1_FACTOR: i32 = 32;

/// Total number of docs covered by one level-1 skip entry.
///
/// Mirrors `Lucene104PostingsFormat.LEVEL1_NUM_DOCS`.
pub const LEVEL1_NUM_DOCS: i32 = LEVEL1_FACTOR * BLOCK_SIZE;

/// Mask for extracting the position inside a level-1 range.
pub const LEVEL1_MASK: i32 = LEVEL1_NUM_DOCS - 1;

/// Skip interval between level-0 skip entries, equal to [`BLOCK_SIZE`].
pub const SKIP_INTERVAL: i32 = BLOCK_SIZE;

// -----------------------------------------------------------------------------
// File extensions
// -----------------------------------------------------------------------------

/// Filename extension for small metadata about how postings are encoded (`.psm`).
pub const META_EXTENSION: &str = "psm";

/// Filename extension for document numbers, frequencies, and skip data (`.doc`).
pub const DOC_EXTENSION: &str = "doc";

/// Filename extension for positions (`.pos`).
pub const POS_EXTENSION: &str = "pos";

/// Filename extension for payloads and offsets (`.pay`).
pub const PAY_EXTENSION: &str = "pay";

// -----------------------------------------------------------------------------
// Codec names written into index headers
// -----------------------------------------------------------------------------

/// Codec name stored in the `.tim` file header.
pub const TERMS_CODEC: &str = "Lucene104PostingsWriterTerms";

/// Codec name stored in the `.psm` file header.
pub const META_CODEC: &str = "Lucene104PostingsWriterMeta";

/// Codec name stored in the `.doc` file header.
pub const DOC_CODEC: &str = "Lucene104PostingsWriterDoc";

/// Codec name stored in the `.pos` file header.
pub const POS_CODEC: &str = "Lucene104PostingsWriterPos";

/// Codec name stored in the `.pay` file header.
pub const PAY_CODEC: &str = "Lucene104PostingsWriterPay";

/// Initial format version.
pub const VERSION_START: i32 = 0;

/// Current format version.
pub const VERSION_CURRENT: i32 = VERSION_START;

// -----------------------------------------------------------------------------
// Lucene104 postings format
// -----------------------------------------------------------------------------

/// Lucene 10.4 postings format.
///
/// Encodes inverted-index postings using packed integer blocks of size
/// [`BLOCK_SIZE`]. The terms dictionary is delegated to
/// [`Lucene103BlockTreeTermsWriter`] / [`Lucene103BlockTreeTermsReader`].
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene104.Lucene104PostingsFormat`.
#[derive(Debug, Clone)]
pub struct Lucene104PostingsFormat {
    version: i32,
    min_term_block_size: i32,
    max_term_block_size: i32,
}

impl Default for Lucene104PostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl Lucene104PostingsFormat {
    /// Creates the format with default block-tree settings.
    pub fn new() -> Self {
        Self::with_block_sizes(DEFAULT_MIN_BLOCK_SIZE, DEFAULT_MAX_BLOCK_SIZE)
    }

    /// Creates the format with custom block-tree settings.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `min_block_size` or
    /// `max_block_size` are invalid.
    pub fn with_block_sizes(min_term_block_size: i32, max_term_block_size: i32) -> Self {
        Lucene103BlockTreeTermsWriter::validate_settings(min_term_block_size, max_term_block_size)
            .expect("invalid block-tree term block sizes");
        Self {
            version: VERSION_CURRENT,
            min_term_block_size,
            max_term_block_size,
        }
    }

    /// Expert constructor that allows setting the format version.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `version` is out of range.
    pub fn with_version(
        min_term_block_size: i32,
        max_term_block_size: i32,
        version: i32,
    ) -> Result<Self> {
        Lucene103BlockTreeTermsWriter::validate_settings(min_term_block_size, max_term_block_size)?;
        if version < VERSION_START || version > VERSION_CURRENT {
            return Err(LuceneError::IllegalArgument(format!(
                "Lucene104PostingsFormat version out of range: {version}"
            )));
        }
        Ok(Self {
            version,
            min_term_block_size,
            max_term_block_size,
        })
    }

    /// Returns the configured minimum term block size.
    pub fn min_term_block_size(&self) -> i32 {
        self.min_term_block_size
    }

    /// Returns the configured maximum term block size.
    pub fn max_term_block_size(&self) -> i32 {
        self.max_term_block_size
    }

    /// Returns the format version.
    pub fn version(&self) -> i32 {
        self.version
    }
}

impl PostingsFormat for Lucene104PostingsFormat {
    fn name(&self) -> &str {
        "Lucene104"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let postings_writer = Box::new(Lucene104PostingsWriter::with_version(state, self.version)?);
        let terms_writer = Lucene103BlockTreeTermsWriter::new(
            state,
            postings_writer,
            self.min_term_block_size,
            self.max_term_block_size,
        )?;
        Ok(Box::new(terms_writer))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn FieldsProducer + 'a>> {
        let postings_reader = Box::new(Lucene104PostingsReader::new(state)?);
        let reader = Lucene103BlockTreeTermsReader::new(postings_reader, state)?;
        Ok(Box::new(reader))
    }
}

// -----------------------------------------------------------------------------
// Lucene104 postings writer
// -----------------------------------------------------------------------------

// The full writer lives in `crate::codecs::lucene104::postings_writer` and is
// re-exported as `crate::codecs::lucene104::Lucene104PostingsWriter`. It is
// imported above and instantiated in `fields_consumer`.

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::postings::{NormsProducer, NumericDocValues};
    use crate::codecs::stub::{FieldInfo, FieldInfos};
    use crate::index::SegmentInfo;
    use crate::store::{Directory, RamDirectory};
    use crate::util::{StringHelper, Version};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// No-op norms producer for testing.
    #[derive(Debug, Default, Clone)]
    struct TestNormsProducer;

    impl NumericDocValues for TestNormsProducer {
        fn get(&self, _doc_id: i32) -> Result<i64> {
            Ok(0)
        }
    }

    impl NormsProducer for TestNormsProducer {
        fn get_norms(&self, _field_info: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
            Ok(Box::new(TestNormsProducer))
        }
    }

    fn test_segment_info(name: &str, max_doc: i32) -> SegmentInfo {
        SegmentInfo::new(
            Arc::new(RamDirectory::default()),
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .expect("test segment info should be valid")
    }

    fn test_write_state<'a>(
        dir: &'a dyn crate::store::Directory,
        info: &'a SegmentInfo,
        infos: &'a FieldInfos,
    ) -> SegmentWriteState<'a> {
        SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir,
            info,
            infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    fn test_read_state<'a>(
        dir: &'a dyn crate::store::Directory,
        info: &'a SegmentInfo,
        infos: &'a FieldInfos,
    ) -> SegmentReadState<'a> {
        SegmentReadState::new(dir, info, infos, &*crate::store::DEFAULT_IO_CONTEXT)
    }

    #[test]
    fn constants_match_lucene_104() {
        assert_eq!(BLOCK_SIZE, 256);
        assert_eq!(MAX_BLOCK_SIZE, 256);
        assert_eq!(BLOCK_SIZE_LOG2, 8);
        assert_eq!(BLOCK_MASK, 255);
        assert_eq!(SKIP_INTERVAL, 256);
        assert_eq!(LEVEL1_FACTOR, 32);
        assert_eq!(LEVEL1_NUM_DOCS, 8192);
        assert_eq!(LEVEL1_MASK, 8191);

        assert_eq!(META_EXTENSION, "psm");
        assert_eq!(DOC_EXTENSION, "doc");
        assert_eq!(POS_EXTENSION, "pos");
        assert_eq!(PAY_EXTENSION, "pay");

        assert_eq!(TERMS_CODEC, "Lucene104PostingsWriterTerms");
        assert_eq!(META_CODEC, "Lucene104PostingsWriterMeta");
        assert_eq!(DOC_CODEC, "Lucene104PostingsWriterDoc");
        assert_eq!(POS_CODEC, "Lucene104PostingsWriterPos");
        assert_eq!(PAY_CODEC, "Lucene104PostingsWriterPay");

        assert_eq!(VERSION_START, 0);
        assert_eq!(VERSION_CURRENT, 0);
    }

    #[test]
    fn format_name_is_lucene104() {
        let format = Lucene104PostingsFormat::new();
        assert_eq!(format.name(), "Lucene104");
    }

    #[test]
    fn default_block_sizes_match_defaults() {
        let format = Lucene104PostingsFormat::new();
        assert_eq!(format.min_term_block_size(), DEFAULT_MIN_BLOCK_SIZE);
        assert_eq!(format.max_term_block_size(), DEFAULT_MAX_BLOCK_SIZE);
        assert_eq!(format.version(), VERSION_CURRENT);
    }

    #[test]
    fn custom_block_sizes_are_stored() {
        let format = Lucene104PostingsFormat::with_block_sizes(30, 60);
        assert_eq!(format.min_term_block_size(), 30);
        assert_eq!(format.max_term_block_size(), 60);
    }

    #[test]
    fn invalid_block_sizes_panic() {
        let result = std::panic::catch_unwind(|| {
            Lucene104PostingsFormat::with_block_sizes(1, 48);
        });
        assert!(result.is_err());
    }

    #[test]
    fn with_version_rejects_out_of_range() {
        let err = Lucene104PostingsFormat::with_version(25, 48, -1).expect_err("bad version");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn fields_consumer_returns_writer() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let segment_info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let write_state = test_write_state(dir_ref, &segment_info, &field_infos);

        let format = Lucene104PostingsFormat::new();
        let mut consumer = format
            .fields_consumer(&write_state)
            .expect("consumer should build");

        consumer
            .write(&crate::index::EmptyFields::new(), &TestNormsProducer)
            .expect("write should succeed on skeleton");
        consumer.close().expect("close should succeed");

        // The writer should have produced the blocktree terms files.
        let names = dir.list_all().expect("directory should list files");
        assert!(names.contains(&"_0.tim".to_string()));
        assert!(names.contains(&"_0.tip".to_string()));
        assert!(names.contains(&"_0.tmd".to_string()));
    }

    #[test]
    fn fields_producer_returns_reader_after_writing_empty_fields() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let segment_info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let write_state = test_write_state(dir_ref, &segment_info, &field_infos);

        let format = Lucene104PostingsFormat::new();
        let mut consumer = format
            .fields_consumer(&write_state)
            .expect("consumer should build");
        consumer
            .write(&crate::index::EmptyFields::new(), &TestNormsProducer)
            .expect("write should succeed on empty fields");
        consumer.close().expect("close should succeed");

        let read_state = test_read_state(dir_ref, &segment_info, &field_infos);
        let mut producer = format
            .fields_producer(&read_state)
            .expect("producer should build");

        assert_eq!(producer.size(), 0);
        producer
            .check_integrity()
            .expect("check_integrity should succeed");
        producer.close().expect("close should succeed");
    }
}
