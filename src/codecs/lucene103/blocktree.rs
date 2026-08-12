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
use crate::codecs::term_state::{BlockTermState, TermStats};
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::segment_file_name;
use crate::index::IndexOptions;
use crate::index::SegmentInfo;
use crate::store::{ByteBuffersDataOutput, DataOutput, Directory, IOContext, IndexOutput};
use crate::util::compress::{LowercaseAsciiCompression, Lz4};
use crate::util::{ArrayUtil, BytesRef, BytesRefBuilder, FixedBitSet, StringHelper};

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
/// Builder for the blocktree term index trie.
///
/// This is a minimal implementation for the current checkpoint. It stores the
/// file pointer and has-terms flag of the root block for each field and writes
/// a trivial one-node trie to the `.tip` file.
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

    /// Compiles the index for a list of pending blocks.
    ///
    /// In this simplified version only the first block's output is retained.
    pub(crate) fn compile(
        blocks: &[PendingBlock],
        scratch: &mut ByteBuffersDataOutput,
    ) -> Result<Self> {
        scratch.reset();
        let first = &blocks[0];
        if first.is_floor {
            scratch.write_v_int((blocks.len() - 1) as i32)?;
            for block in &blocks[1..] {
                scratch.write_byte(block.floor_lead_byte as u8)?;
                scratch.write_v_long(
                    ((block.fp - first.fp) << 1) | if block.has_terms { 1 } else { 0 },
                )?;
            }
        }
        Ok(Self::new(first.fp, first.has_terms))
    }

    /// Saves this trie to the index output, recording offsets in `meta`.
    pub fn save(&self, meta: &mut dyn DataOutput, index_out: &mut dyn IndexOutput) -> Result<()> {
        let index_start_fp = index_out.file_pointer();
        meta.write_v_long(index_start_fp)?;
        let root_fp = index_out.file_pointer();
        // Trivial trie: single leaf node with output.
        let output_fp_bytes = Self::bytes_required_v_long(self.index_start_fp);
        let header = 0x00 | ((output_fp_bytes - 1) << 2) | if self.has_terms { 1 << 5 } else { 0 };
        index_out.write_byte(header as u8)?;
        Self::write_long_n_bytes(index_out, self.index_start_fp, output_fp_bytes)?;
        index_out.write_long(0)?;
        meta.write_v_long(root_fp)?;
        meta.write_v_long(index_out.file_pointer())?;
        Ok(())
    }

    fn bytes_required_v_long(v: i64) -> i32 {
        (8 - ((v | 1).leading_zeros() >> 3)) as i32
    }

    fn write_long_n_bytes(out: &mut dyn IndexOutput, v: i64, n: i32) -> Result<()> {
        let mut v = v;
        for _ in 0..n {
            out.write_byte(v as u8)?;
            v >>= 8;
        }
        Ok(())
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
struct TermsWriter<'a> {
    field_info: FieldInfo,
    min_items_in_block: i32,
    max_items_in_block: i32,
    num_terms: i64,
    docs_seen: FixedBitSet,
    sum_total_term_freq: i64,
    sum_doc_freq: i64,
    pending: Vec<PendingEntry>,
    prefix_starts: Vec<i32>,
    last_term: BytesRefBuilder,
    first_pending_term: Option<PendingTerm>,
    last_pending_term: Option<PendingTerm>,
    suffix_lengths_writer: ByteBuffersDataOutput,
    suffix_writer: BytesRefBuilder,
    stats_writer: ByteBuffersDataOutput,
    meta_writer: ByteBuffersDataOutput,
    spare_writer: ByteBuffersDataOutput,
    spare_bytes: Vec<u8>,
    compression_hash_table: Option<Box<dyn Lz4HashTable>>,
    terms_out: &'a mut dyn IndexOutput,
    index_out: &'a mut dyn IndexOutput,
    scratch_bytes: &'a mut ByteBuffersDataOutput,
    has_freqs: bool,
    new_blocks: Vec<PendingBlock>,
}

impl<'a> TermsWriter<'a> {
    fn new(
        field_info: FieldInfo,
        max_doc: i32,
        min_items_in_block: i32,
        max_items_in_block: i32,
        terms_out: &'a mut dyn IndexOutput,
        index_out: &'a mut dyn IndexOutput,
        scratch_bytes: &'a mut ByteBuffersDataOutput,
        compression_hash_table: Option<Box<dyn Lz4HashTable>>,
    ) -> Self {
        let has_freqs = field_info.index_options != IndexOptions::DOCS;
        Self {
            field_info,
            min_items_in_block,
            max_items_in_block,
            num_terms: 0,
            docs_seen: FixedBitSet::new(max_doc as usize),
            sum_total_term_freq: 0,
            sum_doc_freq: 0,
            pending: Vec::new(),
            prefix_starts: vec![0; 8],
            last_term: BytesRefBuilder::new(),
            first_pending_term: None,
            last_pending_term: None,
            suffix_lengths_writer: ByteBuffersDataOutput::new_resettable_instance(),
            suffix_writer: BytesRefBuilder::new(),
            stats_writer: ByteBuffersDataOutput::new_resettable_instance(),
            meta_writer: ByteBuffersDataOutput::new_resettable_instance(),
            spare_writer: ByteBuffersDataOutput::new_resettable_instance(),
            spare_bytes: Vec::new(),
            compression_hash_table,
            terms_out,
            index_out,
            scratch_bytes,
            has_freqs,
            new_blocks: Vec::new(),
        }
    }

    fn write(
        &mut self,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        postings_writer: &mut dyn PostingsWriterBase,
        norms: &dyn NormsProducer,
    ) -> Result<()> {
        if let Some(state) =
            postings_writer.write_term(term, terms_enum, &mut self.docs_seen, norms)?
        {
            self.push_term(term, postings_writer)?;
            let pending = PendingTerm::new(term, state);
            self.pending.push(PendingEntry::Term(pending.clone()));
            self.sum_doc_freq += state.doc_freq as i64;
            self.sum_total_term_freq += state.total_term_freq;
            self.num_terms += 1;
            if self.first_pending_term.is_none() {
                self.first_pending_term = Some(pending.clone());
            }
            self.last_pending_term = Some(pending);
        }
        Ok(())
    }

    fn push_term(
        &mut self,
        text: &BytesRef,
        postings_writer: &mut dyn PostingsWriterBase,
    ) -> Result<()> {
        let prefix_length = if self.last_term.length() == 0 {
            0
        } else {
            let last = self.last_term.get();
            let last_ref = BytesRef {
                bytes: last.bytes.clone(),
                offset: last.offset,
                length: last.length,
            };
            let text_ref = BytesRef {
                bytes: text.bytes.clone(),
                offset: text.offset,
                length: text.length,
            };
            StringHelper::bytes_difference(&last_ref, &text_ref).unwrap_or(0)
        };

        for i in ((prefix_length as usize)..self.last_term.length()).rev() {
            let prefix_top_size = self.pending.len() as i32 - self.prefix_starts[i];
            if prefix_top_size >= self.min_items_in_block {
                self.write_blocks(i as i32 + 1, prefix_top_size, postings_writer)?;
                self.prefix_starts[i] -= prefix_top_size - 1;
            }
        }

        if self.prefix_starts.len() < text.length {
            self.prefix_starts.resize(text.length + 1, 0);
        }
        for i in prefix_length as usize..text.length {
            self.prefix_starts[i] = self.pending.len() as i32;
        }

        self.last_term
            .copy_bytes(&text.bytes, text.offset, text.length);
        Ok(())
    }

    fn write_blocks(
        &mut self,
        prefix_length: i32,
        count: i32,
        postings_writer: &mut dyn PostingsWriterBase,
    ) -> Result<()> {
        debug_assert!(count > 0);
        debug_assert!(prefix_length > 0 || count as usize == self.pending.len());

        let start = self.pending.len() - count as usize;
        let end = self.pending.len();
        let mut last_suffix_lead_label = -1;
        let mut has_terms = false;
        let mut has_sub_blocks = false;
        let mut next_block_start = start;
        let mut next_floor_lead_label = -1;

        for i in start..end {
            let suffix_lead_label = match &self.pending[i] {
                PendingEntry::Term(term) => {
                    if term.term_bytes.len() == prefix_length as usize {
                        -1
                    } else {
                        term.term_bytes[prefix_length as usize] as i32 & 0xff
                    }
                }
                PendingEntry::Block(block) => {
                    debug_assert!(block.prefix.length > prefix_length as usize);
                    block.prefix.bytes[block.prefix.offset + prefix_length as usize] as i32 & 0xff
                }
            };

            if suffix_lead_label != last_suffix_lead_label {
                let items_in_block = i as i32 - next_block_start as i32;
                if items_in_block >= self.min_items_in_block
                    && end as i32 - next_block_start as i32 > self.max_items_in_block
                {
                    let is_floor = items_in_block < count;
                    let block = self.write_block(
                        prefix_length,
                        is_floor,
                        next_floor_lead_label,
                        next_block_start,
                        i,
                        has_terms,
                        has_sub_blocks,
                        postings_writer,
                    )?;
                    self.new_blocks.push(block);
                    has_terms = false;
                    has_sub_blocks = false;
                    next_floor_lead_label = suffix_lead_label;
                    next_block_start = i;
                }
                last_suffix_lead_label = suffix_lead_label;
            }

            match &self.pending[i] {
                PendingEntry::Term(_) => has_terms = true,
                PendingEntry::Block(_) => has_sub_blocks = true,
            }
        }

        if next_block_start < end {
            let items_in_block = end as i32 - next_block_start as i32;
            let is_floor = items_in_block < count;
            let block = self.write_block(
                prefix_length,
                is_floor,
                next_floor_lead_label,
                next_block_start,
                end,
                has_terms,
                has_sub_blocks,
                postings_writer,
            )?;
            self.new_blocks.push(block);
        }

        debug_assert!(!self.new_blocks.is_empty());
        let first_block = self.new_blocks.remove(0);
        debug_assert!(first_block.is_floor || self.new_blocks.is_empty());

        // Compile index for the first block using the new_blocks list.
        let mut all_blocks = vec![first_block.clone()];
        all_blocks.extend(self.new_blocks.drain(..));
        let compiled = TrieBuilder::compile(&all_blocks, &mut *self.scratch_bytes)?;
        let first_block = PendingBlock {
            index: Some(compiled),
            ..all_blocks.into_iter().next().unwrap()
        };

        self.pending.truncate(self.pending.len() - count as usize);
        self.pending.push(PendingEntry::Block(first_block));
        Ok(())
    }

    fn all_equal(bytes: &[u8], start: usize, end: usize, value: u8) -> bool {
        bytes[start..end].iter().all(|&b| b == value)
    }

    fn write_block(
        &mut self,
        prefix_length: i32,
        is_floor: bool,
        floor_lead_label: i32,
        start: usize,
        end: usize,
        has_terms: bool,
        has_sub_blocks: bool,
        postings_writer: &mut dyn PostingsWriterBase,
    ) -> Result<PendingBlock> {
        let start_fp = self.terms_out.file_pointer();
        let has_floor_lead_label = is_floor && floor_lead_label != -1;
        let prefix_len = prefix_length as usize + if has_floor_lead_label { 1 } else { 0 };
        let mut prefix = BytesRef::with_capacity(prefix_len);
        let last = self.last_term.get();
        prefix.bytes[..prefix_length as usize]
            .copy_from_slice(&last.bytes[last.offset..last.offset + prefix_length as usize]);
        prefix.length = prefix_length as usize;

        let num_entries = end - start;
        let mut code = (num_entries as i32) << 1;
        if end == self.pending.len() {
            code |= 1;
        }
        self.terms_out.write_v_int(code)?;

        let is_leaf_block = !has_sub_blocks;
        let mut sub_indices: Option<Vec<TrieBuilder>> = if is_leaf_block {
            None
        } else {
            Some(Vec::new())
        };
        let mut absolute = true;

        // Snapshot pending entries so we can mutate self while iterating.
        let entries: Vec<PendingEntry> = self.pending[start..end].to_vec();

        if is_leaf_block {
            for ent in entries {
                if let PendingEntry::Term(term) = ent {
                    let suffix = term.term_bytes.len() - prefix_length as usize;
                    self.suffix_lengths_writer.write_v_int(suffix as i32)?;
                    self.suffix_writer.append_bytes(
                        &term.term_bytes,
                        prefix_length as usize,
                        suffix,
                    );
                    self.write_term_stats(term.state.doc_freq, term.state.total_term_freq)?;
                    postings_writer.encode_term(
                        &mut self.meta_writer,
                        &self.field_info,
                        &term.state,
                        absolute,
                    )?;
                    absolute = false;
                } else {
                    panic!("leaf block contains a sub-block");
                }
            }
        } else {
            for ent in entries {
                match ent {
                    PendingEntry::Term(term) => {
                        let suffix = term.term_bytes.len() - prefix_length as usize;
                        self.suffix_lengths_writer
                            .write_v_int((suffix as i32) << 1)?;
                        self.suffix_writer.append_bytes(
                            &term.term_bytes,
                            prefix_length as usize,
                            suffix,
                        );
                        self.write_term_stats(term.state.doc_freq, term.state.total_term_freq)?;
                        postings_writer.encode_term(
                            &mut self.meta_writer,
                            &self.field_info,
                            &term.state,
                            absolute,
                        )?;
                        absolute = false;
                    }
                    PendingEntry::Block(block) => {
                        let suffix = block.prefix.length - prefix_length as usize;
                        self.suffix_lengths_writer
                            .write_v_int(((suffix as i32) << 1) | 1)?;
                        self.suffix_writer.append_bytes(
                            &block.prefix.bytes,
                            block.prefix.offset + prefix_length as usize,
                            suffix,
                        );
                        self.terms_out.write_v_long(start_fp - block.fp)?;
                        if let Some(idx) = &block.index {
                            sub_indices.as_mut().unwrap().push(idx.clone());
                        }
                    }
                }
            }
        }
        // Stats are written as we go; the stats_writer is a simple ByteBuffersDataOutput.

        // Suffix compression (disabled in this checkpoint: always no compression).
        let compression_alg = CompressionAlgorithm::NoCompression;
        let suffix_len = self.suffix_writer.length();

        let mut token = (suffix_len as i64) << 3;
        if is_leaf_block {
            token |= 0x04;
        }
        token |= compression_alg.code() as i64;
        self.terms_out.write_v_long(token)?;
        if compression_alg == CompressionAlgorithm::NoCompression {
            self.terms_out
                .write_bytes(self.suffix_writer.bytes(), 0, suffix_len)?;
        } else {
            self.spare_writer.copy_to(self.terms_out)?;
        }
        self.suffix_writer.clear();
        self.spare_writer.reset();

        // Suffix lengths
        let num_suffix_bytes = self.suffix_lengths_writer.size();
        self.spare_bytes.resize(num_suffix_bytes, 0);
        let mut suffix_lengths_out = crate::store::ByteArrayDataOutput::new();
        self.suffix_lengths_writer
            .copy_to(&mut suffix_lengths_out)?;
        self.suffix_lengths_writer.reset();
        self.spare_bytes.resize(suffix_lengths_out.len(), 0);
        self.spare_bytes
            .copy_from_slice(suffix_lengths_out.as_inner());
        if num_suffix_bytes > 1
            && Self::all_equal(&self.spare_bytes, 1, num_suffix_bytes, self.spare_bytes[0])
        {
            self.terms_out
                .write_v_int(((num_suffix_bytes as i32) << 1) | 1)?;
            self.terms_out.write_byte(self.spare_bytes[0])?;
        } else {
            self.terms_out.write_v_int((num_suffix_bytes as i32) << 1)?;
            self.terms_out
                .write_bytes(&self.spare_bytes, 0, num_suffix_bytes)?;
        }

        // Stats
        let num_stats_bytes = self.stats_writer.size();
        self.terms_out.write_v_int(num_stats_bytes as i32)?;
        self.stats_writer.copy_to(self.terms_out)?;
        self.stats_writer.reset();

        // Term metadata
        self.terms_out.write_v_int(self.meta_writer.size() as i32)?;
        self.meta_writer.copy_to(self.terms_out)?;
        self.meta_writer.reset();

        if has_floor_lead_label {
            prefix.bytes[prefix.length] = floor_lead_label as u8;
            prefix.length += 1;
        }

        Ok(PendingBlock::new(
            prefix,
            start_fp,
            has_terms,
            is_floor,
            floor_lead_label,
            sub_indices,
        ))
    }

    fn write_term_stats(&mut self, doc_freq: i32, total_term_freq: i64) -> Result<()> {
        if doc_freq == 1 && (!self.has_freqs || total_term_freq == 1) {
            self.stats_writer.write_v_int(1)?;
        } else {
            self.stats_writer.write_v_int(doc_freq << 1)?;
            if self.has_freqs {
                self.stats_writer
                    .write_v_long(total_term_freq - doc_freq as i64)?;
            }
        }
        Ok(())
    }

    fn finish(&mut self, postings_writer: &mut dyn PostingsWriterBase) -> Result<Vec<u8>> {
        if self.num_terms > 0 {
            self.push_term(&BytesRef::new(Vec::new()), postings_writer)?;
            self.push_term(&BytesRef::new(Vec::new()), postings_writer)?;
            self.write_blocks(0, self.pending.len() as i32, postings_writer)?;

            debug_assert!(self.pending.len() == 1);
            let root = match self.pending.pop() {
                Some(PendingEntry::Block(b)) => b,
                _ => panic!("expected single root block"),
            };
            debug_assert!(root.prefix.length == 0);

            let mut meta = ByteBuffersDataOutput::new();
            meta.write_v_int(self.field_info.number)?;
            meta.write_v_long(self.num_terms)?;
            if self.has_freqs {
                meta.write_v_long(self.sum_total_term_freq)?;
            }
            meta.write_v_long(self.sum_doc_freq)?;
            meta.write_v_int(self.docs_seen.cardinality() as i32)?;
            Self::write_bytes_ref(
                &mut meta,
                &BytesRef::new(self.first_pending_term.as_ref().unwrap().term_bytes.clone()),
            )?;
            Self::write_bytes_ref(
                &mut meta,
                &BytesRef::new(self.last_pending_term.as_ref().unwrap().term_bytes.clone()),
            )?;

            let first_term = self.first_pending_term.take().unwrap();
            let root_index = root
                .index
                .unwrap_or_else(|| TrieBuilder::new(0, root.has_terms));
            root_index.save(&mut meta, self.index_out)?;

            // Keep postings writer in sync.
            let _ = postings_writer;
            let _ = first_term;

            Ok(meta.to_array_copy())
        } else {
            Ok(Vec::new())
        }
    }

    fn write_bytes_ref(out: &mut dyn DataOutput, bytes: &BytesRef) -> Result<()> {
        out.write_v_int(bytes.length as i32)?;
        out.write_bytes(&bytes.bytes, bytes.offset, bytes.length)?;
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
    scratch_bytes: ByteBuffersDataOutput,
    compression_hash_table: Option<Box<dyn Lz4HashTable>>,
}

/// Hash-table interface used by the LZ4 high-compression encoder.
///
/// Lucene Core equivalent: `org.apache.lucene.util.compress.LZ4.HASH_TABLE`.
pub trait Lz4HashTable: Send + Sync {
    /// Resets the table for a new input region.
    fn reset(&mut self, bytes: &[u8], off: usize, len: usize) -> Result<()>;
    /// Initializes the dictionary portion of the input.
    fn init_dictionary(&mut self, bytes: &[u8], dict_len: usize);
    /// Looks up a 4-byte hash at `off`; returns a previous offset or `None`.
    fn get(&mut self, bytes: &[u8], off: usize) -> Option<usize>;
    /// Returns the previous occurrence for a matched offset.
    fn previous(&mut self, bytes: &[u8], off: usize) -> Option<usize>;
}

impl Lz4HashTable for crate::util::compress::FastCompressionHashTable {
    fn reset(&mut self, bytes: &[u8], off: usize, len: usize) -> Result<()> {
        crate::util::compress::FastCompressionHashTable::reset(self, bytes, off, len)
    }
    fn init_dictionary(&mut self, bytes: &[u8], dict_len: usize) {
        crate::util::compress::FastCompressionHashTable::init_dictionary(self, bytes, dict_len);
    }
    fn get(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        crate::util::compress::FastCompressionHashTable::get(self, bytes, off)
    }
    fn previous(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        crate::util::compress::FastCompressionHashTable::previous(self, bytes, off)
    }
}

impl Lz4HashTable for crate::util::compress::HighCompressionHashTable {
    fn reset(&mut self, bytes: &[u8], off: usize, len: usize) -> Result<()> {
        crate::util::compress::HighCompressionHashTable::reset(self, bytes, off, len)
    }
    fn init_dictionary(&mut self, bytes: &[u8], dict_len: usize) {
        crate::util::compress::HighCompressionHashTable::init_dictionary(self, bytes, dict_len);
    }
    fn get(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        crate::util::compress::HighCompressionHashTable::get(self, bytes, off)
    }
    fn previous(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        crate::util::compress::HighCompressionHashTable::previous(self, bytes, off)
    }
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
            scratch_bytes: ByteBuffersDataOutput::new_resettable_instance(),
            compression_hash_table: None,
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
        // Open outputs early so per-field TermsWriter can write blocks directly.
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

        for field in fields.iterator() {
            if let Some(terms) = fields.terms(&field)? {
                let mut terms_enum = terms.iterator()?;
                let field_info = self
                    .field_infos
                    .field_info(&field)
                    .cloned()
                    .unwrap_or_default();
                {
                    let postings_writer_ref: &mut dyn PostingsWriterBase =
                        self.postings_writer.as_mut();
                    postings_writer_ref.set_field(&field_info)?;
                    let mut terms_writer = TermsWriter::new(
                        field_info,
                        self.max_doc,
                        self.min_items_in_block,
                        self.max_items_in_block,
                        terms_out.as_mut(),
                        index_out.as_mut(),
                        &mut self.scratch_bytes,
                        self.compression_hash_table.take(),
                    );
                    while let Some(term) = terms_enum.next()? {
                        terms_writer.write(&term, &mut *terms_enum, postings_writer_ref, norms)?;
                    }
                    let field_meta = terms_writer.finish(postings_writer_ref)?;
                    self.fields.push(field_meta);
                    self.compression_hash_table = terms_writer.compression_hash_table;
                }
            }
        }

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
        self.closed = true;
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

        for field_meta in &self.fields {
            meta_out.write_bytes(field_meta, 0, field_meta.len())?;
        }

        meta_out.write_v_int(self.fields.len() as i32)?;
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
