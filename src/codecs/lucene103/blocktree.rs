//! BlockTree terms dictionary ported from Lucene 10.5.0.
//!
//! This module provides the terms dictionary used by `Lucene104PostingsFormat`,
//! reading and writing the `.tim` (terms), `.tmd` (metadata) and `.tip` (term
//! index) files.
//!
//! Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree`.

#![deny(unsafe_code)]

use std::fmt;

use crate::codecs::codec_util::{write_be_long, write_footer, write_index_header};
use crate::codecs::postings::{
    Fields, FieldsConsumer, FieldsProducer, MergeState, NormsProducer, PostingsReaderBase,
    PostingsWriterBase, Terms, TermsEnum,
};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::{FieldInfo, FieldInfos};
use crate::codecs::term_state::BlockTermState;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::segment_file_name;
use crate::index::SegmentInfo;
use crate::store::{Directory, IOContext};
use crate::util::compress::{LowercaseAsciiCompression, Lz4};
use crate::util::{BytesRef, BytesRefBuilder, FixedBitSet};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// Extension of the terms dictionary file (`.tim`).
pub const TERMS_EXTENSION: &str = "tim";
/// Codec name written into the `.tim` header.
pub const TERMS_CODEC_NAME: &str = "BlockTreeTermsDict";
/// Extension of the term index file (`.tip`).
pub const TERMS_INDEX_EXTENSION: &str = "tip";
/// Codec name written into the `.tip` header.
pub const TERMS_INDEX_CODEC_NAME: &str = "BlockTreeTermsIndex";
/// Extension of the term metadata file (`.tmd`).
pub const TERMS_META_EXTENSION: &str = "tmd";
/// Codec name written into the `.tmd` header.
pub const TERMS_META_CODEC_NAME: &str = "BlockTreeTermsMeta";

/// Initial terms format version.
pub const VERSION_START: i32 = 0;
/// Current terms format version.
pub const VERSION_CURRENT: i32 = VERSION_START;

/// Default minimum number of entries per block.
pub const DEFAULT_MIN_BLOCK_SIZE: i32 = 25;
/// Default maximum number of entries per block.
pub const DEFAULT_MAX_BLOCK_SIZE: i32 = 48;

// -----------------------------------------------------------------------------
// CompressionAlgorithm
// -----------------------------------------------------------------------------

/// Compression algorithm used for suffix bytes of a block.
///
/// Lucene Core equivalent: `CompressionAlgorithm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlgorithm {
    /// No compression; suffix bytes are stored verbatim.
    NoCompression = 0,
    /// Specialized packer for lowercase ASCII text.
    LowercaseAscii = 1,
    /// LZ4 block compression.
    Lz4 = 2,
}

impl CompressionAlgorithm {
    /// Returns the algorithm for the given code byte.
    pub fn by_code(code: i32) -> Result<Self> {
        match code {
            0 => Ok(Self::NoCompression),
            1 => Ok(Self::LowercaseAscii),
            2 => Ok(Self::Lz4),
            _ => Err(LuceneError::IllegalArgument(format!(
                "illegal compression algorithm code: {code}"
            ))),
        }
    }

    /// Returns the code byte for this algorithm.
    pub fn code(&self) -> i32 {
        *self as i32
    }

    /// Decompresses `len` bytes from `input` into `out`.
    pub fn read(
        &self,
        input: &mut dyn crate::store::DataInput,
        out: &mut [u8],
        len: usize,
    ) -> Result<()> {
        match self {
            Self::NoCompression => input.read_bytes(out, 0, len),
            Self::LowercaseAscii => {
                LowercaseAsciiCompression::decompress(input, out, len)?;
                Ok(())
            }
            Self::Lz4 => {
                Lz4::decompress(input, len, out, 0)?;
                Ok(())
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Stats
// -----------------------------------------------------------------------------

/// BlockTree statistics for a single field.
///
/// Lucene Core equivalent: `Stats`.
#[derive(Debug, Default, Clone)]
pub struct Stats {
    /// Byte size of the index.
    pub index_num_bytes: i64,
    /// Total number of terms in the field.
    pub total_term_count: i64,
    /// Total number of bytes across all terms in the field.
    pub total_term_bytes: i64,
    /// Number of normal (non-floor) blocks.
    pub non_floor_block_count: i32,
    /// Number of floor blocks.
    pub floor_block_count: i32,
    /// Number of floor sub-blocks.
    pub floor_sub_block_count: i32,
    /// Number of mixed blocks (terms + sub-blocks).
    pub mixed_block_count: i32,
    /// Number of leaf blocks (terms only).
    pub terms_only_block_count: i32,
    /// Number of sub-block-only blocks.
    pub sub_blocks_only_block_count: i32,
    /// Total number of blocks.
    pub total_block_count: i32,
    /// Number of blocks at each prefix depth.
    pub block_count_by_prefix_len: Vec<i32>,
    /// Total bytes used to store term suffixes after compression.
    pub total_block_suffix_bytes: i64,
    /// Number of times each compression method has been used.
    pub compression_algorithms: [i64; 3],
    /// Total suffix bytes before compression.
    pub total_uncompressed_block_suffix_bytes: i64,
    /// Total bytes used to store term stats.
    pub total_block_stats_bytes: i64,
    /// Total bytes stored by the postings reader plus a few vInts in the frame.
    pub total_block_other_bytes: i64,
    /// Segment name.
    pub segment: String,
    /// Field name.
    pub field: String,
    start_block_count: i32,
    end_block_count: i32,
}

#[allow(dead_code)]
impl Stats {
    fn new(segment: String, field: String) -> Self {
        Self {
            block_count_by_prefix_len: vec![0; 10],
            segment,
            field,
            ..Default::default()
        }
    }

    fn term(&mut self, term: &BytesRef) {
        self.total_term_bytes += term.length as i64;
    }

    fn finish(&self) {
        assert_eq!(
            self.start_block_count, self.end_block_count,
            "startBlockCount={} endBlockCount={}",
            self.start_block_count, self.end_block_count
        );
        assert_eq!(
            self.total_block_count,
            self.floor_sub_block_count + self.non_floor_block_count,
            "floorSubBlockCount={} nonFloorBlockCount={} totalBlockCount={}",
            self.floor_sub_block_count,
            self.non_floor_block_count,
            self.total_block_count
        );
        assert_eq!(
            self.total_block_count,
            self.mixed_block_count + self.terms_only_block_count + self.sub_blocks_only_block_count,
            "totalBlockCount={} mixedBlockCount={} subBlocksOnlyBlockCount={} termsOnlyBlockCount={}",
            self.total_block_count,
            self.mixed_block_count,
            self.sub_blocks_only_block_count,
            self.terms_only_block_count
        );
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  index trie:")?;
        writeln!(f, "    {} bytes", self.index_num_bytes)?;
        writeln!(f, "  terms:")?;
        writeln!(f, "    {} terms", self.total_term_count)?;
        write!(f, "    {} bytes", self.total_term_bytes)?;
        if self.total_term_count != 0 {
            writeln!(
                f,
                " ({:.1} bytes/term)",
                self.total_term_bytes as f64 / self.total_term_count as f64
            )?;
        } else {
            writeln!(f)?;
        }
        writeln!(f, "  blocks:")?;
        writeln!(f, "    {} blocks", self.total_block_count)?;
        writeln!(f, "    {} terms-only blocks", self.terms_only_block_count)?;
        writeln!(
            f,
            "    {} sub-block-only blocks",
            self.sub_blocks_only_block_count
        )?;
        writeln!(f, "    {} mixed blocks", self.mixed_block_count)?;
        writeln!(f, "    {} floor blocks", self.floor_block_count)?;
        writeln!(
            f,
            "    {} non-floor blocks",
            self.total_block_count - self.floor_sub_block_count
        )?;
        writeln!(f, "    {} floor sub-blocks", self.floor_sub_block_count)?;
        write!(
            f,
            "    {} term suffix bytes before compression",
            self.total_uncompressed_block_suffix_bytes
        )?;
        if self.total_block_count != 0 {
            write!(
                f,
                " ({:.1} suffix-bytes/block)",
                self.total_block_suffix_bytes as f64 / self.total_block_count as f64
            )?;
        }
        writeln!(f)?;
        let mut compression_counts = String::new();
        for code in 0..3 {
            if self.compression_algorithms[code] == 0 {
                continue;
            }
            if !compression_counts.is_empty() {
                compression_counts.push_str(", ");
            }
            compression_counts.push_str(&format!(
                "{:?}: {}",
                CompressionAlgorithm::by_code(code as i32).unwrap(),
                self.compression_algorithms[code]
            ));
        }
        write!(
            f,
            "    {} compressed term suffix bytes",
            self.total_block_suffix_bytes
        )?;
        if self.total_block_count != 0 && self.total_uncompressed_block_suffix_bytes > 0 {
            write!(
                f,
                " ({:.2} compression ratio - compression count by algorithm: {})",
                self.total_block_suffix_bytes as f64
                    / self.total_uncompressed_block_suffix_bytes as f64,
                compression_counts
            )?;
        }
        writeln!(f)?;
        write!(f, "    {} term stats bytes", self.total_block_stats_bytes)?;
        if self.total_block_count != 0 {
            write!(
                f,
                " ({:.1} stats-bytes/block)",
                self.total_block_stats_bytes as f64 / self.total_block_count as f64
            )?;
        }
        writeln!(f)?;
        write!(f, "    {} other bytes", self.total_block_other_bytes)?;
        if self.total_block_count != 0 {
            write!(
                f,
                " ({:.1} other-bytes/block)",
                self.total_block_other_bytes as f64 / self.total_block_count as f64
            )?;
        }
        writeln!(f)
    }
}

// -----------------------------------------------------------------------------
// TrieBuilder / TrieReader skeletons
// -----------------------------------------------------------------------------

/// Builder for the blocktree term index trie.
///
/// This is a minimal skeleton for the current checkpoint. It stores the file
/// pointer and has-terms flag of the root block for each field. The full
/// prefix-tree construction (depth-first serialization, floor blocks, child
/// strategies, etc.) will be added in a later run.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder`.
#[derive(Debug, Default, Clone)]
pub struct TrieBuilder {
    /// File pointer in the `.tip` file where this field's trie starts.
    pub index_start_fp: i64,
    /// Whether the root block pointed to contains any terms.
    pub has_terms: bool,
}

impl TrieBuilder {
    /// Creates a trie builder rooted at `index_start_fp` with the given term flag.
    pub fn new(index_start_fp: i64, has_terms: bool) -> Self {
        Self {
            index_start_fp,
            has_terms,
        }
    }
}

/// Reader for the blocktree term index trie.
///
/// This is a minimal skeleton for the current checkpoint. It records the file
/// pointer of the trie root so that later work can load the on-disk nodes.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.TrieReader`.
#[derive(Debug, Default, Clone)]
pub struct TrieReader {
    /// File pointer in the `.tip` file where this field's trie root is stored.
    pub root_fp: i64,
}

impl TrieReader {
    /// Creates a trie reader rooted at `root_fp`.
    pub fn new(root_fp: i64) -> Self {
        Self { root_fp }
    }
}

// -----------------------------------------------------------------------------
// Pending term / block helpers
// -----------------------------------------------------------------------------

/// Base type for entries queued while building a block.
///
/// Lucene Core equivalent: `Lucene103BlockTreeTermsWriter.PendingEntry`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum PendingEntry {
    Term(PendingTerm),
    Block(PendingBlock),
}

/// A term waiting to be written to a block.
///
/// Lucene Core equivalent: `Lucene103BlockTreeTermsWriter.PendingTerm`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingTerm {
    term_bytes: Vec<u8>,
    state: BlockTermState,
}

impl PendingTerm {
    fn new(term: &BytesRef, state: BlockTermState) -> Self {
        Self {
            term_bytes: term.bytes[term.offset..term.offset + term.length].to_vec(),
            state,
        }
    }
}

/// A block that has been written to the terms file and is waiting to be
/// linked into the trie index.
///
/// Lucene Core equivalent: `Lucene103BlockTreeTermsWriter.PendingBlock`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingBlock {
    prefix: BytesRef,
    fp: i64,
    has_terms: bool,
    is_floor: bool,
    floor_lead_byte: i32,
    index: Option<TrieBuilder>,
    sub_indices: Option<Vec<TrieBuilder>>,
}

impl PendingBlock {
    fn new(
        prefix: BytesRef,
        fp: i64,
        has_terms: bool,
        is_floor: bool,
        floor_lead_byte: i32,
        sub_indices: Option<Vec<TrieBuilder>>,
    ) -> Self {
        Self {
            prefix,
            fp,
            has_terms,
            is_floor,
            floor_lead_byte,
            index: None,
            sub_indices,
        }
    }
}

// -----------------------------------------------------------------------------
// Per-field terms writer
// -----------------------------------------------------------------------------

/// Writer for the terms of a single field.
///
/// In this checkpoint the writer only collects per-field statistics in memory.
/// Recursive block building, suffix compression and trie serialization are
/// intentionally not implemented yet.
///
/// Lucene Core equivalent: `Lucene103BlockTreeTermsWriter.TermsWriter`.
#[allow(dead_code)]
struct TermsWriter {
    field_info: FieldInfo,
    num_terms: i64,
    docs_seen: FixedBitSet,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    pending: Vec<PendingEntry>,
    prefix_starts: Vec<i32>,
    last_term: BytesRefBuilder,
    first_pending_term: Option<PendingTerm>,
    last_pending_term: Option<PendingTerm>,
}

impl TermsWriter {
    fn new(field_info: FieldInfo, max_doc: i32) -> Self {
        Self {
            field_info,
            num_terms: 0,
            docs_seen: FixedBitSet::new(max_doc as usize),
            sum_total_term_freq: 0,
            sum_doc_freq: 0,
            pending: Vec::new(),
            prefix_starts: vec![0; 8],
            last_term: BytesRefBuilder::new(),
            first_pending_term: None,
            last_pending_term: None,
        }
    }

    /// Writes a single term and its postings.
    ///
    /// This skeleton only records the term bytes and updates field-level
    /// counters; the recursive block construction is left for a later run.
    fn write(
        &mut self,
        term: &BytesRef,
        _terms_enum: &mut dyn TermsEnum,
        _norms: &dyn NormsProducer,
    ) -> Result<()> {
        self.num_terms += 1;
        self.last_term
            .copy_bytes(&term.bytes, term.offset, term.length);
        let pending = PendingTerm::new(term, BlockTermState::default());
        if self.first_pending_term.is_none() {
            self.first_pending_term = Some(pending.clone());
        }
        self.last_pending_term = Some(pending);
        Ok(())
    }

    /// Finishes this field. For the skeleton this is a no-op when no terms were
    /// seen; real block flushing will happen here later.
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Terms dictionary writer
// -----------------------------------------------------------------------------

/// Block-based terms index and dictionary writer.
///
/// Writes the `.tim`, `.tmd` and `.tip` files for a single segment. In this
/// checkpoint only the file envelopes (headers, footers and the field-count
/// metadata wrapper) are implemented. The recursive block tree construction,
/// suffix compression and FST-style trie index are left for later work.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.Lucene103BlockTreeTermsWriter`.
pub struct Lucene103BlockTreeTermsWriter<'a> {
    directory: &'a dyn Directory,
    context: &'a dyn IOContext,
    segment_info: &'a SegmentInfo,
    segment_suffix: String,
    field_infos: FieldInfos,
    max_doc: i32,
    min_items_in_block: i32,
    max_items_in_block: i32,
    version: i32,
    postings_writer: Box<dyn PostingsWriterBase>,
    fields: Vec<Vec<u8>>,
    closed: bool,
}

impl<'a> Lucene103BlockTreeTermsWriter<'a> {
    /// Creates a new writer with the current format version.
    pub fn new(
        state: &SegmentWriteState<'a>,
        postings_writer: Box<dyn PostingsWriterBase>,
        min_items_in_block: i32,
        max_items_in_block: i32,
    ) -> Result<Self> {
        Self::new_with_version(
            state,
            postings_writer,
            min_items_in_block,
            max_items_in_block,
            VERSION_CURRENT,
        )
    }

    /// Expert constructor that allows configuring the version, used for backward
    /// compatibility tests.
    pub fn new_with_version(
        state: &SegmentWriteState<'a>,
        postings_writer: Box<dyn PostingsWriterBase>,
        min_items_in_block: i32,
        max_items_in_block: i32,
        version: i32,
    ) -> Result<Self> {
        Self::validate_settings(min_items_in_block, max_items_in_block)?;

        if version < VERSION_START || version > VERSION_CURRENT {
            return Err(LuceneError::IllegalArgument(format!(
                "expected version in range [{}, {}], but got {}",
                VERSION_START, VERSION_CURRENT, version
            )));
        }

        Ok(Self {
            directory: state.directory,
            context: state.context,
            segment_info: state.segment_info,
            segment_suffix: state.segment_suffix.clone(),
            field_infos: state.field_infos.clone(),
            max_doc: state.segment_info.max_doc()?,
            min_items_in_block,
            max_items_in_block,
            version,
            postings_writer,
            fields: Vec::new(),
            closed: false,
        })
    }

    /// Validates the min/max block size settings, matching the Java checks.
    pub fn validate_settings(min_items_in_block: i32, max_items_in_block: i32) -> Result<()> {
        if min_items_in_block <= 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "minItemsInBlock must be >= 2; got {min_items_in_block}"
            )));
        }
        if min_items_in_block > max_items_in_block {
            return Err(LuceneError::IllegalArgument(format!(
                "maxItemsInBlock must be >= minItemsInBlock; got maxItemsInBlock={max_items_in_block} minItemsInBlock={min_items_in_block}"
            )));
        }
        if 2 * (min_items_in_block - 1) > max_items_in_block {
            return Err(LuceneError::IllegalArgument(format!(
                "maxItemsInBlock must be at least 2*(minItemsInBlock-1); got maxItemsInBlock={max_items_in_block} minItemsInBlock={min_items_in_block}"
            )));
        }
        Ok(())
    }
}

impl<'a> fmt::Debug for Lucene103BlockTreeTermsWriter<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene103BlockTreeTermsWriter")
            .field("segment", &self.segment_info.name)
            .field("version", &self.version)
            .field("min_items_in_block", &self.min_items_in_block)
            .field("max_items_in_block", &self.max_items_in_block)
            .field("fields", &self.fields.len())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl<'a> FieldsConsumer for Lucene103BlockTreeTermsWriter<'a> {
    fn write(&mut self, fields: &dyn Fields, norms: &dyn NormsProducer) -> Result<()> {
        for field in fields.iterator() {
            if let Some(terms) = fields.terms(&field)? {
                let mut terms_enum = terms.iterator()?;
                let field_info = self
                    .field_infos
                    .field_info(&field)
                    .cloned()
                    .unwrap_or_default();
                let mut terms_writer = TermsWriter::new(field_info, self.max_doc);
                while let Some(term) = terms_enum.next()? {
                    terms_writer.write(&term, &mut *terms_enum, norms)?;
                }
                terms_writer.finish()?;
            }
        }
        Ok(())
    }

    fn merge(&mut self, _merge_state: &MergeState, _norms: &dyn NormsProducer) -> Result<()> {
        // Merges will be implemented once the write path is complete.
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;

        let terms_name = segment_file_name(
            &self.segment_info.name,
            &self.segment_suffix,
            TERMS_EXTENSION,
        );
        let mut terms_out = self.directory.create_output(&terms_name, self.context)?;
        write_index_header(
            terms_out.as_mut(),
            TERMS_CODEC_NAME,
            self.version,
            &self.segment_info.id(),
            &self.segment_suffix,
        )?;

        let index_name = segment_file_name(
            &self.segment_info.name,
            &self.segment_suffix,
            TERMS_INDEX_EXTENSION,
        );
        let mut index_out = self.directory.create_output(&index_name, self.context)?;
        write_index_header(
            index_out.as_mut(),
            TERMS_INDEX_CODEC_NAME,
            self.version,
            &self.segment_info.id(),
            &self.segment_suffix,
        )?;

        let meta_name = segment_file_name(
            &self.segment_info.name,
            &self.segment_suffix,
            TERMS_META_EXTENSION,
        );
        let mut meta_out = self.directory.create_output(&meta_name, self.context)?;
        write_index_header(
            meta_out.as_mut(),
            TERMS_META_CODEC_NAME,
            self.version,
            &self.segment_info.id(),
            &self.segment_suffix,
        )?;

        self.postings_writer.init(
            meta_out.as_mut(),
            &SegmentWriteState::new(
                crate::util::default_info_stream(),
                self.directory,
                self.segment_info,
                &self.field_infos,
                &crate::codecs::stub::BufferedUpdates,
                self.context,
            ),
        )?;

        meta_out.write_v_int(self.fields.len() as i32)?;
        for field_meta in &self.fields {
            meta_out.write_bytes(field_meta, 0, field_meta.len())?;
        }

        write_footer(index_out.as_mut())?;
        write_be_long(meta_out.as_mut(), index_out.file_pointer())?;
        write_footer(terms_out.as_mut())?;
        write_be_long(meta_out.as_mut(), terms_out.file_pointer())?;
        write_footer(meta_out.as_mut())?;

        meta_out.close()?;
        terms_out.close()?;
        index_out.close()?;
        self.postings_writer.close()?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Terms dictionary reader skeleton
// -----------------------------------------------------------------------------

/// Block-based terms index and dictionary reader.
///
/// This is a minimal skeleton that satisfies the [`FieldsProducer`] contract.
/// It opens no files and reports an empty term space. The full recursive block
/// tree loading, suffix decompression and FST-style trie index will be added in
/// later tasks.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.Lucene103BlockTreeTermsReader`.
pub struct Lucene103BlockTreeTermsReader {
    postings_reader: Box<dyn PostingsReaderBase>,
    segment: String,
    segment_suffix: String,
    version: i32,
}

impl std::fmt::Debug for Lucene103BlockTreeTermsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene103BlockTreeTermsReader")
            .field("segment", &self.segment)
            .field("segment_suffix", &self.segment_suffix)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl Lucene103BlockTreeTermsReader {
    /// Creates a new skeleton reader.
    pub fn new(
        postings_reader: Box<dyn PostingsReaderBase>,
        state: &SegmentReadState,
    ) -> Result<Self> {
        Ok(Self {
            postings_reader,
            segment: state.segment_info.name.clone(),
            segment_suffix: state.segment_suffix.clone(),
            version: VERSION_CURRENT,
        })
    }

    /// Returns the segment name this reader was opened for.
    pub fn segment(&self) -> &str {
        &self.segment
    }

    /// Returns the segment suffix used when opening this reader.
    pub fn segment_suffix(&self) -> &str {
        &self.segment_suffix
    }

    /// Returns the format version of the underlying files.
    pub fn version(&self) -> i32 {
        self.version
    }
}

impl Fields for Lucene103BlockTreeTermsReader {
    fn size(&self) -> i32 {
        0
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(None)
    }

    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(std::iter::empty::<String>())
    }
}

impl FieldsProducer for Lucene103BlockTreeTermsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Err(LuceneError::UnsupportedOperation(
            "Lucene103BlockTreeTermsReader skeleton does not support merge instances".to_string(),
        ))
    }

    fn close(&mut self) -> Result<()> {
        self.postings_reader.close()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::codec_util::{check_footer, check_index_header, footer_length};
    use crate::codecs::postings::FieldsConsumer;
    use crate::codecs::term_state::BlockTermState;
    use crate::index::{EmptyFields, SegmentInfo};
    use crate::store::{Directory, IndexInput, IndexOutput, RamDirectory};
    use crate::util::default_info_stream;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// No-op postings writer used to satisfy the terms writer constructor.
    #[derive(Debug, Default, Clone)]
    struct StubPostingsWriter;

    impl PostingsWriterBase for StubPostingsWriter {
        fn init(
            &mut self,
            _terms_out: &mut dyn IndexOutput,
            _state: &SegmentWriteState,
        ) -> Result<()> {
            Ok(())
        }

        fn write_term(
            &mut self,
            _term: &BytesRef,
            _terms_enum: &mut dyn TermsEnum,
            _docs_seen: &mut FixedBitSet,
            _norms: &dyn NormsProducer,
        ) -> Result<Option<BlockTermState>> {
            Ok(None)
        }

        fn encode_term(
            &mut self,
            _out: &mut dyn crate::store::DataOutput,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn set_field(&mut self, _field_info: &FieldInfo) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// No-op norms producer.
    struct StubNormsProducer;

    impl NormsProducer for StubNormsProducer {
        fn get_norms(
            &self,
            _field_info: &FieldInfo,
        ) -> Result<Box<dyn crate::codecs::postings::NumericDocValues>> {
            Err(LuceneError::UnsupportedOperation(
                "stub norms producer".to_string(),
            ))
        }
    }

    fn test_segment_info(name: &str, max_doc: i32) -> SegmentInfo {
        SegmentInfo::new(
            Arc::new(RamDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            HashMap::new(),
            [0u8; crate::util::string_helper::ID_LENGTH],
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .expect("test segment info should be valid")
    }

    fn test_write_state<'a>(
        dir: &'a dyn Directory,
        info: &'a SegmentInfo,
        infos: &'a FieldInfos,
    ) -> SegmentWriteState<'a> {
        SegmentWriteState::new(
            default_info_stream(),
            dir,
            info,
            infos,
            &crate::codecs::stub::BufferedUpdates,
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    #[test]
    fn writer_creates_empty_terms_files_with_valid_headers_and_footers() {
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let state = test_write_state(dir_ref, &segment_info, &field_infos);

        let postings_writer: Box<dyn PostingsWriterBase> = Box::new(StubPostingsWriter);
        let mut writer = Lucene103BlockTreeTermsWriter::new(&state, postings_writer, 25, 48)
            .expect("writer should be created");

        writer
            .write(&EmptyFields::new(), &StubNormsProducer)
            .expect("write should succeed");
        writer.close().expect("close should succeed");

        let expected_files = &[
            ("_0.tim", TERMS_CODEC_NAME),
            ("_0.tmd", TERMS_META_CODEC_NAME),
            ("_0.tip", TERMS_INDEX_CODEC_NAME),
        ];

        let names = dir.list_all().expect("directory should list files");
        for (name, _codec) in expected_files {
            assert!(names.contains(&name.to_string()), "missing file: {name}");
        }

        let segment_id = segment_info.id();
        for (name, codec) in expected_files {
            let mut input = dir
                .open_checksum_input(name)
                .expect("should open checksum input");
            let version = check_index_header(
                &mut *input,
                codec,
                VERSION_START,
                VERSION_CURRENT,
                &segment_id,
                "",
            )
            .expect("header should be valid");
            assert_eq!(version, VERSION_CURRENT);
            let footer_start = input.length() - footer_length() as i64;
            input.seek(footer_start).expect("should seek to footer");
            check_footer(&mut *input).expect("footer should be valid");
        }
    }

    #[test]
    fn validate_settings_rejects_bad_block_sizes() {
        assert!(Lucene103BlockTreeTermsWriter::validate_settings(1, 48).is_err());
        assert!(Lucene103BlockTreeTermsWriter::validate_settings(25, 24).is_err());
        assert!(Lucene103BlockTreeTermsWriter::validate_settings(25, 25).is_err());
        assert!(Lucene103BlockTreeTermsWriter::validate_settings(25, 48).is_ok());
    }
}
